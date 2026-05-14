/**
 * WASM Solver Bridge
 *
 * Implements ConstraintSolver by delegating to WasmViewScriptEngine.tick().
 * Handles QSnapshot construction and TickResult parsing.
 *
 * ## Responsibilities
 *
 * 1. Convert TStateMutation[] to QSnapshot JSON format
 * 2. Inject FFI results into QSnapshot.values (via injectFfiResults)
 * 3. Call WasmViewScriptEngine.tick() with JSON payload
 * 4. Parse TickResult and extract bounds + pendingFfiCalls
 *
 * ## Data Flow
 *
 * ```
 * TStateMutation[] + FfiResult[]
 *        │
 *        ▼
 *   buildQSnapshot()
 *        │
 *        ▼
 *   QSnapshot JSON ─────► WasmViewScriptEngine.tick()
 *                                   │
 *                                   ▼
 *                            TickResult JSON
 *                                   │
 *                                   ▼
 *                         SolverEvaluationResult
 * ```
 */

import type { EntityId, PVectorBounds } from '../ast/types.js';
import type { FfiResult, PendingFfiCall } from './ffi-dispatcher.js';

// =============================================================================
// Types
// =============================================================================

/**
 * Semantic T-vector state keys.
 */
type TStateKey =
  | 'hover'
  | 'pressed'
  | 'focused'
  | 'scroll_x'
  | 'scroll_y'
  | 'drag_progress'
  | 'animation_t'
  | 'gesture_phase';

/**
 * T-vector state mutation from Q-dimension event.
 */
export interface TStateMutation {
  entityId: EntityId;
  state: TStateKey;
  value: number;
  timestamp: number;
}

/**
 * Result of constraint solver evaluation.
 */
export interface SolverEvaluationResult {
  bounds: Map<EntityId, PVectorBounds>;
  pendingFfiCalls: PendingFfiCall[];
}

/**
 * ConstraintSolver interface.
 *
 * Note: This interface is FFI-agnostic. FFI result injection
 * is handled by the concrete WasmSolverBridge implementation.
 */
export interface ConstraintSolver {
  evaluate(mutations: TStateMutation[]): SolverEvaluationResult;
}

/**
 * QSnapshot format for WASM tick() input.
 */
interface QSnapshot {
  values: Record<string, QValue>;
  mutations: unknown[];
}

/**
 * Q-dimension value types.
 */
type QValue =
  | { type: 'float'; value: number }
  | { type: 'int'; value: number }
  | { type: 'bool'; value: boolean };

/**
 * TickResult from WASM tick() output.
 */
interface TickResult {
  pending_ffi_calls: PendingFfiCall[];
}

/**
 * WASM engine interface (subset used by bridge).
 */
export interface WasmEngine {
  tick(inputJson: string): string;
}

// =============================================================================
// WASM Solver Bridge
// =============================================================================

export class WasmSolverBridge implements ConstraintSolver {
  private engine: WasmEngine;

  /** Pending FFI results to inject into next QSnapshot */
  private pendingFfiResults: FfiResult[] = [];

  /** Entity name to ID mapping (for mutation conversion) */
  private entityNameToId: Map<string, EntityId> = new Map();

  /** Q-variable name to entity+state mapping */
  private qVarMapping: Map<string, { entityId: EntityId; state: TStateKey }> = new Map();

  constructor(engine: WasmEngine) {
    this.engine = engine;
  }

  // ===========================================================================
  // Public API
  // ===========================================================================

  /**
   * Inject FFI results for the next evaluate() call.
   *
   * Called from render-loop Phase 0.5. Results are consumed
   * and cleared when evaluate() runs.
   *
   * @param results - FFI function results from previous frame
   */
  injectFfiResults(results: FfiResult[]): void {
    this.pendingFfiResults = results;
  }

  /**
   * Register entity name to ID mapping.
   *
   * Called during initialization to enable mutation conversion.
   */
  registerEntity(name: string, id: EntityId): void {
    this.entityNameToId.set(name, id);
  }

  /**
   * Register Q-variable mapping.
   *
   * Maps Q-variable names to entity+state for mutation routing.
   */
  registerQVariable(name: string, entityId: EntityId, state: TStateKey): void {
    this.qVarMapping.set(name, { entityId, state });
  }

  /**
   * Evaluate constraints and return solver result.
   *
   * Implements ConstraintSolver interface.
   */
  evaluate(mutations: TStateMutation[]): SolverEvaluationResult {
    // 1. Build QSnapshot from mutations + FFI results
    const qSnapshot = this.buildQSnapshot(mutations);

    // 2. Call WASM tick()
    let resultJson: string;
    try {
      resultJson = this.engine.tick(JSON.stringify(qSnapshot));
    } catch (error) {
      // tick() failed - FFI results are preserved for next frame retry
      console.error('[WasmSolverBridge] tick() error:', error);
      return {
        bounds: new Map(),
        pendingFfiCalls: [],
      };
    }

    // 3. tick() succeeded - now safe to clear FFI results
    this.pendingFfiResults = [];

    // 4. Parse TickResult
    let tickResult: TickResult;
    try {
      tickResult = JSON.parse(resultJson) as TickResult;
    } catch (error) {
      console.error('[WasmSolverBridge] Failed to parse TickResult:', error);
      return {
        bounds: new Map(),
        pendingFfiCalls: [],
      };
    }

    // 5. Build SolverEvaluationResult
    // Note: bounds extraction is deferred - WASM tick() currently
    // handles rendering internally. This will be refactored when
    // bounds are returned explicitly from tick().
    return {
      bounds: new Map(), // TODO: Extract from tick() when available
      pendingFfiCalls: tickResult.pending_ffi_calls ?? [],
    };
  }

  // ===========================================================================
  // Internal Helpers
  // ===========================================================================

  /**
   * Build QSnapshot from mutations and FFI results.
   */
  private buildQSnapshot(mutations: TStateMutation[]): QSnapshot {
    const values: Record<string, QValue> = {};

    // 1. Inject FFI results into values
    for (const result of this.pendingFfiResults) {
      values[result.name] = { type: 'float', value: result.value };
    }

    // 2. Convert TStateMutations to Q-values
    // Each mutation targets an entity's T-state, which maps to a Q-variable
    for (const mutation of mutations) {
      const qVarName = this.getQVarName(mutation.entityId, mutation.state);
      values[qVarName] = { type: 'float', value: mutation.value };
    }

    // 3. Build mutation array for legacy format compatibility
    const mutationArray = mutations.map((m) => ({
      entity_id: m.entityId,
      state: m.state,
      value: m.value,
    }));

    return {
      values,
      mutations: mutationArray,
    };
  }

  /**
   * Get Q-variable name for an entity's T-state.
   *
   * Convention: `{entityId}_{state}` (e.g., "1_hover", "2_scroll_y")
   */
  private getQVarName(entityId: EntityId, state: TStateKey): string {
    return `${entityId}_${state}`;
  }
}

// =============================================================================
// Factory Function
// =============================================================================

/**
 * Create a WASM solver bridge instance.
 *
 * @param engine - WasmViewScriptEngine instance
 */
export function createWasmSolverBridge(engine: WasmEngine): WasmSolverBridge {
  return new WasmSolverBridge(engine);
}
