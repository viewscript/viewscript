/**
 * FFI Dispatcher for ViewScript Runtime
 *
 * Evaluates pending FFI calls after frame commit (Phase 8) and buffers
 * results for consumption in the next frame's QSnapshot merge (Phase 0-1).
 *
 * ## Design Rationale
 *
 * FFI function execution time is non-deterministic (user-defined functions).
 * By executing after commitFrame(), we:
 * 1. Keep the rendering critical path (Phase 2-7) free of unpredictable latency
 * 2. Utilize the idle time between commitFrame() and the next rAF callback
 * 3. Maintain 1-frame latency for FFI results (Axiom 2: Ouroboros Binding)
 *
 * ## Data Flow
 *
 * ```
 * Frame N:
 *   Phase 2: tick() → TickResult { pending_ffi_calls }
 *   Phase 8: dispatch(pending_ffi_calls) → buffer results
 *
 * Frame N+1:
 *   Phase 0-1: drainResults() → QSnapshot.values merge
 *   Phase 2: solver sees FFI results as Q-dimension input
 * ```
 */

// =============================================================================
// Types (mirrors vsc-wasm/src/ffi_bridge.rs)
// =============================================================================

/**
 * FFI argument types from manifest.
 */
export type FfiArg =
  | { type: 'static'; value: unknown }
  | { type: 'q_ref'; name: string }
  | { type: 'entity_coord'; entity_id: number; component: string };

/**
 * FFI binding from manifest.
 */
export interface FfiBinding {
  ffi_id: number;
  bind_name: string;
  module_path: string;
  export_name: string;
  args: FfiArg[];
}

/**
 * FFI trigger from manifest.
 */
export interface FfiTrigger {
  trigger_id: number;
  ffi_id: number;
  module_path: string;
  export_name: string;
  condition: {
    kind: string;
    entity_a?: number;
    entity_b?: number;
  };
  args: FfiArg[];
}

/**
 * Complete FFI manifest.
 */
export interface FfiManifest {
  version: number;
  entity_map: Record<string, number>;
  bindings: FfiBinding[];
  triggers: FfiTrigger[];
}

/**
 * Pending FFI call from WASM tick() result.
 */
export interface PendingFfiCall {
  ffi_id: number;
  trigger_id?: number;
  args: unknown[];
}

/**
 * Result of an FFI call, ready for QSnapshot merge.
 */
export interface FfiResult {
  /** Q-variable name to update */
  name: string;
  /** Computed value */
  value: number;
  /** Timestamp for event ordering */
  timestamp: number;
}

/**
 * Registered FFI function entry.
 */
interface FfiFunctionEntry {
  ffi_id: number;
  bind_name: string;
  module_path: string;
  export_name: string;
  args: FfiArg[];
  /** Resolved function reference (set by registerModule) */
  fn: ((...args: unknown[]) => unknown) | null;
}

// =============================================================================
// FFI Dispatcher
// =============================================================================

export class FfiDispatcher {
  /** Function registry: ffi_id -> entry */
  private registry = new Map<number, FfiFunctionEntry>();

  /** Pending modules awaiting registration */
  private pendingModules = new Set<string>();

  /** Buffered results for next frame */
  private resultBuffer: FfiResult[] = [];

  /** In-flight async calls (prevents duplicate dispatch while awaiting) */
  private inflight = new Set<number>();

  /** Entity coordinate resolver (injected) */
  private coordResolver: CoordinateResolver | null = null;

  /** Q-variable resolver (injected) */
  private qResolver: QVariableResolver | null = null;

  // ===========================================================================
  // Public API
  // ===========================================================================

  /**
   * Load FFI manifest and build function registry.
   *
   * @param manifest - Parsed FfiManifest JSON
   */
  loadManifest(manifest: FfiManifest): void {
    this.registry.clear();
    this.pendingModules.clear();

    // Register bindings
    for (const binding of manifest.bindings) {
      this.registry.set(binding.ffi_id, {
        ffi_id: binding.ffi_id,
        bind_name: binding.bind_name,
        module_path: binding.module_path,
        export_name: binding.export_name,
        args: binding.args,
        fn: null,
      });
      this.pendingModules.add(binding.module_path);
    }

    // Register triggers (each trigger has its own function reference)
    for (const trigger of manifest.triggers) {
      if (!this.registry.has(trigger.ffi_id)) {
        this.registry.set(trigger.ffi_id, {
          ffi_id: trigger.ffi_id,
          bind_name: `trigger_${trigger.trigger_id}`, // Triggers don't have bind names
          module_path: trigger.module_path,
          export_name: trigger.export_name,
          args: trigger.args,
          fn: null,
        });
        this.pendingModules.add(trigger.module_path);
      }
    }
  }

  /**
   * Register a dynamically imported ESM module.
   *
   * Call this after `import(modulePath)` resolves.
   *
   * @param modulePath - Resolved module path (must match manifest's module_path)
   * @param module - The imported module object
   */
  registerModule(modulePath: string, module: Record<string, unknown>): void {
    for (const entry of this.registry.values()) {
      if (entry.module_path === modulePath) {
        const fn = module[entry.export_name];
        if (typeof fn === 'function') {
          entry.fn = fn as (...args: unknown[]) => unknown;
        } else {
          console.warn(
            `[FfiDispatcher] Export '${entry.export_name}' from '${modulePath}' is not a function`
          );
        }
      }
    }
    this.pendingModules.delete(modulePath);
  }

  /**
   * Set the coordinate resolver for EntityCoord arguments.
   */
  setCoordinateResolver(resolver: CoordinateResolver): void {
    this.coordResolver = resolver;
  }

  /**
   * Set the Q-variable resolver for QRef arguments.
   */
  setQResolver(resolver: QVariableResolver): void {
    this.qResolver = resolver;
  }

  /**
   * Get list of module paths that need to be imported.
   */
  getPendingModules(): string[] {
    return [...this.pendingModules];
  }

  /**
   * Check if all modules have been registered.
   */
  isReady(): boolean {
    return this.pendingModules.size === 0;
  }

  /**
   * Dispatch pending FFI calls (Phase 8).
   *
   * Evaluates JS functions and buffers results. Supports both sync and async
   * functions. For async functions, results are buffered when the Promise
   * resolves (may be multiple frames later).
   *
   * In-flight guard: If an async call for a given ffi_id is still pending,
   * subsequent dispatch requests for the same ffi_id are skipped until the
   * previous call completes.
   *
   * @param pendingCalls - Array of PendingFfiCall from TickResult
   */
  dispatch(pendingCalls: PendingFfiCall[]): void {
    for (const call of pendingCalls) {
      const entry = this.registry.get(call.ffi_id);
      if (!entry) {
        console.warn(`[FfiDispatcher] Unknown ffi_id: ${call.ffi_id}`);
        continue;
      }

      if (!entry.fn) {
        console.warn(
          `[FfiDispatcher] Function not registered for ffi_id ${call.ffi_id} ` +
            `(${entry.module_path}::${entry.export_name})`
        );
        continue;
      }

      // In-flight guard: skip if previous async call hasn't completed
      if (this.inflight.has(call.ffi_id)) {
        continue;
      }

      try {
        // Resolve arguments
        const resolvedArgs = this.resolveArgs(entry.args, call.args);

        // Call the function
        const result = entry.fn(...resolvedArgs);

        // Handle async (Promise) results
        if (result instanceof Promise) {
          this.inflight.add(call.ffi_id);

          result
            .then((value) => {
              this.bufferResult(entry.bind_name, value);
            })
            .catch((error) => {
              console.error(
                `[FfiDispatcher] Async error in ${entry.export_name}:`,
                error
              );
            })
            .finally(() => {
              this.inflight.delete(call.ffi_id);
            });
        } else {
          // Handle sync results
          this.bufferResult(entry.bind_name, result);
        }
      } catch (error) {
        console.error(
          `[FfiDispatcher] Error calling ${entry.export_name}:`,
          error
        );
      }
    }
  }

  /**
   * Buffer a result value if it's numeric.
   */
  private bufferResult(bindName: string, result: unknown): void {
    const timestamp = performance.now();

    if (typeof result === 'number') {
      this.resultBuffer.push({
        name: bindName,
        value: result,
        timestamp,
      });
    } else if (result !== undefined && result !== null) {
      // Attempt numeric coercion for non-number results
      const numValue = Number(result);
      if (!isNaN(numValue)) {
        this.resultBuffer.push({
          name: bindName,
          value: numValue,
          timestamp,
        });
      }
      // Non-numeric results are silently ignored (side-effect only functions)
    }
  }

  /**
   * Drain buffered results for QSnapshot merge (Phase 0-1).
   *
   * Returns all buffered results and clears the buffer.
   * Call this at the start of the next frame.
   *
   * @returns Array of FfiResult for QSnapshot.values merge
   */
  drainResults(): FfiResult[] {
    const results = this.resultBuffer;
    this.resultBuffer = [];
    return results;
  }

  /**
   * Get the number of buffered results (for diagnostics).
   */
  getBufferedCount(): number {
    return this.resultBuffer.length;
  }

  /**
   * Get the number of in-flight async calls (for diagnostics/testing).
   */
  getInflightCount(): number {
    return this.inflight.size;
  }

  /**
   * Check if a specific ffi_id has an in-flight async call.
   */
  isInflight(ffiId: number): boolean {
    return this.inflight.has(ffiId);
  }

  // ===========================================================================
  // Internal Helpers
  // ===========================================================================

  /**
   * Resolve FfiArg array to concrete values.
   *
   * @param argSpecs - Argument specifications from manifest
   * @param runtimeArgs - Runtime argument overrides from PendingFfiCall
   */
  private resolveArgs(argSpecs: FfiArg[], runtimeArgs: unknown[]): unknown[] {
    // If runtime args are provided, use them directly (pre-resolved by WASM)
    if (runtimeArgs.length > 0) {
      return runtimeArgs;
    }

    // Otherwise, resolve from specs
    return argSpecs.map((spec) => this.resolveArg(spec));
  }

  /**
   * Resolve a single FfiArg to its concrete value.
   */
  private resolveArg(spec: FfiArg): unknown {
    switch (spec.type) {
      case 'static':
        return spec.value;

      case 'q_ref':
        if (this.qResolver) {
          return this.qResolver.getQValue(spec.name);
        }
        console.warn(`[FfiDispatcher] No Q resolver, cannot resolve ${spec.name}`);
        return 0;

      case 'entity_coord':
        if (this.coordResolver) {
          return this.coordResolver.getEntityCoord(spec.entity_id, spec.component);
        }
        console.warn(
          `[FfiDispatcher] No coord resolver, cannot resolve entity ${spec.entity_id}.${spec.component}`
        );
        return 0;

      default:
        console.warn(`[FfiDispatcher] Unknown arg type: ${(spec as FfiArg).type}`);
        return 0;
    }
  }
}

// =============================================================================
// Resolver Interfaces
// =============================================================================

/**
 * Interface for resolving entity coordinates.
 */
export interface CoordinateResolver {
  /**
   * Get a coordinate component for an entity.
   *
   * @param entityId - Entity ID
   * @param component - Component name ('x', 'y', 'width', 'height')
   * @returns Coordinate value (integer pixels)
   */
  getEntityCoord(entityId: number, component: string): number;
}

/**
 * Interface for resolving Q-variable values.
 */
export interface QVariableResolver {
  /**
   * Get current value of a Q-variable.
   *
   * @param name - Q-variable name
   * @returns Current value
   */
  getQValue(name: string): number;
}

// =============================================================================
// Factory Function
// =============================================================================

/**
 * Create a new FFI dispatcher instance.
 */
export function createFfiDispatcher(): FfiDispatcher {
  return new FfiDispatcher();
}
