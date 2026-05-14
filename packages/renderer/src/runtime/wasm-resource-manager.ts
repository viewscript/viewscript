/**
 * WASM Resource Manager
 *
 * This module provides explicit lifecycle management for WASM GPU resources.
 * JavaScript's garbage collector cannot free WASM heap memory - we must call
 * delete() explicitly on GPU objects.
 *
 * ## Problem
 *
 * ```
 * const paint = canvasKit.Paint();  // Allocates on WASM heap
 * paint = null;                      // JS reference gone, but WASM memory leaked!
 * ```
 *
 * ## Solution: FinalizationRegistry
 *
 * We track all WASM resources and ensure delete() is called when the JS wrapper
 * is garbage collected, OR when explicitly released via the resource pool.
 *
 * ## Resource Categories
 *
 * | Resource | Lifetime | Release Strategy |
 * |----------|----------|------------------|
 * | PaintSpec  | Entity   | On entity remove / HMR update |
 * | PathEntity | Entity   | On path change / entity remove |
 * | GpuImage   | Async    | On image unload / src change |
 * | GpuFont    | Global   | On font unload (rare) |
 */

// =============================================================================
// Types
// =============================================================================

/**
 * Any GPU WASM object with a delete() method.
 */
interface Deletable {
  delete(): void;
  isDeleted?(): boolean;
}

/**
 * Resource metadata for tracking.
 */
interface ResourceEntry {
  type: ResourceType;
  entityId: number | null;
  createdAt: number;
  accessed: number;
}

type ResourceType = 'paint' | 'path' | 'image' | 'font' | 'shader' | 'surface';

/**
 * Resource pool statistics.
 */
export interface ResourceStats {
  totalAllocated: number;
  totalReleased: number;
  currentActive: number;
  byType: Record<ResourceType, number>;
  leakSuspects: number;
}

// =============================================================================
// Resource Manager
// =============================================================================

export class WASMResourceManager {
  /** FinalizationRegistry for automatic cleanup on GC */
  private registry: FinalizationRegistry<CleanupToken>;

  /** Active resources (strong references for explicit management) */
  private resources: Map<symbol, { ref: WeakRef<Deletable>; entry: ResourceEntry }>;

  /** Statistics */
  private stats: {
    allocated: number;
    released: number;
  };

  /** Cleanup queue (for batch processing) */
  private cleanupQueue: CleanupToken[] = [];
  private cleanupScheduled = false;

  constructor() {
    this.resources = new Map();
    this.stats = { allocated: 0, released: 0 };

    // Create registry with cleanup callback
    this.registry = new FinalizationRegistry((token: CleanupToken) => {
      this.queueCleanup(token);
    });
  }

  // ===========================================================================
  // Public API
  // ===========================================================================

  /**
   * Register a WASM resource for tracking.
   *
   * @param resource - GPU WASM object with delete() method
   * @param type - Resource category
   * @param entityId - Associated entity (optional)
   * @returns Symbol token for explicit release
   */
  register<T extends Deletable>(
    resource: T,
    type: ResourceType,
    entityId: number | null = null,
  ): symbol {
    const token = Symbol(`wasm-${type}-${this.stats.allocated}`);

    const entry: ResourceEntry = {
      type,
      entityId,
      createdAt: performance.now(),
      accessed: performance.now(),
    };

    // Store weak reference for tracking
    this.resources.set(token, {
      ref: new WeakRef(resource),
      entry,
    });

    // Register for finalization (GC-triggered cleanup)
    const cleanupToken: CleanupToken = { token, type };
    this.registry.register(resource, cleanupToken, resource);

    this.stats.allocated++;

    return token;
  }

  /**
   * Explicitly release a resource by token.
   * Preferred over waiting for GC.
   */
  release(token: symbol): boolean {
    const entry = this.resources.get(token);
    if (!entry) return false;

    const resource = entry.ref.deref();
    if (resource && !resource.isDeleted?.()) {
      try {
        resource.delete();
        this.registry.unregister(resource);
      } catch (e) {
        console.warn('[WASM] Failed to delete resource:', e);
      }
    }

    this.resources.delete(token);
    this.stats.released++;

    return true;
  }

  /**
   * Release all resources associated with an entity.
   * Called on entity removal or HMR update.
   */
  releaseByEntity(entityId: number): number {
    let released = 0;

    for (const [token, { entry }] of this.resources) {
      if (entry.entityId === entityId) {
        if (this.release(token)) {
          released++;
        }
      }
    }

    return released;
  }

  /**
   * Release all resources of a specific type.
   */
  releaseByType(type: ResourceType): number {
    let released = 0;

    for (const [token, { entry }] of this.resources) {
      if (entry.type === type) {
        if (this.release(token)) {
          released++;
        }
      }
    }

    return released;
  }

  /**
   * Force cleanup of all queued finalizations.
   * Call after GC to ensure WASM memory is freed.
   */
  flushCleanupQueue(): number {
    const count = this.cleanupQueue.length;

    for (const token of this.cleanupQueue) {
      this.release(token.token);
    }

    this.cleanupQueue = [];
    return count;
  }

  /**
   * Release ALL resources. Use with caution.
   */
  releaseAll(): number {
    let released = 0;

    for (const token of this.resources.keys()) {
      if (this.release(token)) {
        released++;
      }
    }

    return released;
  }

  /**
   * Get current resource statistics.
   */
  getStats(): ResourceStats {
    const byType: Record<ResourceType, number> = {
      paint: 0,
      path: 0,
      image: 0,
      font: 0,
      shader: 0,
      surface: 0,
    };

    let leakSuspects = 0;
    const now = performance.now();
    const LEAK_THRESHOLD_MS = 60000; // 1 minute without access

    for (const { ref, entry } of this.resources.values()) {
      const resource = ref.deref();
      if (resource) {
        byType[entry.type]++;

        // Check for potential leaks (old, unaccessed resources)
        if (now - entry.accessed > LEAK_THRESHOLD_MS) {
          leakSuspects++;
        }
      }
    }

    return {
      totalAllocated: this.stats.allocated,
      totalReleased: this.stats.released,
      currentActive: this.resources.size,
      byType,
      leakSuspects,
    };
  }

  /**
   * Mark a resource as recently accessed (resets leak timer).
   */
  touch(token: symbol): void {
    const entry = this.resources.get(token);
    if (entry) {
      entry.entry.accessed = performance.now();
    }
  }

  // ===========================================================================
  // Private Methods
  // ===========================================================================

  /**
   * Queue a cleanup token for batch processing.
   */
  private queueCleanup(token: CleanupToken): void {
    this.cleanupQueue.push(token);

    // Schedule batch cleanup on next microtask
    if (!this.cleanupScheduled) {
      this.cleanupScheduled = true;
      queueMicrotask(() => {
        this.flushCleanupQueue();
        this.cleanupScheduled = false;
      });
    }
  }
}

/**
 * Token passed to FinalizationRegistry callback.
 */
interface CleanupToken {
  token: symbol;
  type: ResourceType;
}

// =============================================================================
// Global Instance
// =============================================================================

/**
 * Singleton resource manager for the renderer.
 */
export const wasmResources = new WASMResourceManager();

// =============================================================================
// Helper: Scoped Resource Guard
// =============================================================================

/**
 * RAII-style guard for temporary WASM resources.
 *
 * Usage:
 * ```
 * using(canvasKit.Paint(), paint => {
 *   canvas.drawRect(rect, paint);
 * }); // paint.delete() called automatically
 * ```
 */
export function using<T extends Deletable, R>(
  resource: T,
  fn: (resource: T) => R,
): R {
  try {
    return fn(resource);
  } finally {
    resource.delete();
  }
}

/**
 * Async version of using().
 */
export async function usingAsync<T extends Deletable, R>(
  resource: T,
  fn: (resource: T) => Promise<R>,
): Promise<R> {
  try {
    return await fn(resource);
  } finally {
    resource.delete();
  }
}
