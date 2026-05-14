/**
 * Tests for FFI Dispatcher
 *
 * Validates:
 * 1. Manifest loading and function registry
 * 2. Module registration
 * 3. FFI call dispatch and result buffering
 * 4. Argument resolution (static, q_ref, entity_coord)
 * 5. Result draining for QSnapshot merge
 */

import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  FfiDispatcher,
  createFfiDispatcher,
  type FfiManifest,
  type PendingFfiCall,
  type CoordinateResolver,
  type QVariableResolver,
  type FfiResult,
} from '../ffi-dispatcher.js';

// =============================================================================
// Test Fixtures
// =============================================================================

function createTestManifest(): FfiManifest {
  return {
    version: 1,
    entity_map: { button: 1, cursor: 2 },
    bindings: [
      {
        ffi_id: 1,
        bind_name: 'clamped_value',
        module_path: '/src/math.ts',
        export_name: 'clamp',
        args: [
          { type: 'q_ref', name: 'input_value' },
          { type: 'static', value: 0 },
          { type: 'static', value: 1 },
        ],
      },
      {
        ffi_id: 2,
        bind_name: 'distance',
        module_path: '/src/math.ts',
        export_name: 'euclidean',
        args: [
          { type: 'entity_coord', entity_id: 1, component: 'x' },
          { type: 'entity_coord', entity_id: 1, component: 'y' },
          { type: 'entity_coord', entity_id: 2, component: 'x' },
          { type: 'entity_coord', entity_id: 2, component: 'y' },
        ],
      },
    ],
    triggers: [
      {
        trigger_id: 1,
        ffi_id: 3,
        module_path: '/src/math.ts',
        export_name: 'notify',
        condition: { kind: 'bounds_overlap', entity_a: 1, entity_b: 2 },
        args: [{ type: 'static', value: 'clicked' }],
      },
    ],
  };
}

function createMockMathModule(): Record<string, unknown> {
  return {
    clamp: (value: number, min: number, max: number) =>
      Math.max(min, Math.min(max, value)),
    euclidean: (x1: number, y1: number, x2: number, y2: number) =>
      Math.sqrt((x2 - x1) ** 2 + (y2 - y1) ** 2),
    notify: () => 1, // Trigger function
  };
}

function createMockCoordResolver(): CoordinateResolver {
  const coords: Record<string, Record<string, number>> = {
    '1': { x: 100, y: 200, width: 50, height: 30 },
    '2': { x: 150, y: 250, width: 10, height: 10 },
  };
  return {
    getEntityCoord(entityId: number, component: string): number {
      return coords[String(entityId)]?.[component] ?? 0;
    },
  };
}

function createMockQResolver(): QVariableResolver {
  const values: Record<string, number> = {
    input_value: 0.5,
    hover_progress: 0.75,
  };
  return {
    getQValue(name: string): number {
      return values[name] ?? 0;
    },
  };
}

// =============================================================================
// Tests
// =============================================================================

describe('FfiDispatcher', () => {
  let dispatcher: FfiDispatcher;

  beforeEach(() => {
    dispatcher = createFfiDispatcher();
  });

  // ===========================================================================
  // Manifest Loading
  // ===========================================================================

  describe('loadManifest', () => {
    it('registers bindings from manifest', () => {
      const manifest = createTestManifest();
      dispatcher.loadManifest(manifest);

      expect(dispatcher.getPendingModules()).toContain('/src/math.ts');
    });

    it('tracks pending modules for import', () => {
      const manifest = createTestManifest();
      dispatcher.loadManifest(manifest);

      expect(dispatcher.isReady()).toBe(false);
      expect(dispatcher.getPendingModules()).toHaveLength(1);
    });

    it('clears previous registry on reload', () => {
      const manifest1 = createTestManifest();
      dispatcher.loadManifest(manifest1);

      const manifest2: FfiManifest = {
        version: 1,
        entity_map: {},
        bindings: [],
        triggers: [],
      };
      dispatcher.loadManifest(manifest2);

      expect(dispatcher.getPendingModules()).toHaveLength(0);
      expect(dispatcher.isReady()).toBe(true);
    });
  });

  // ===========================================================================
  // Module Registration
  // ===========================================================================

  describe('registerModule', () => {
    it('resolves functions from imported module', () => {
      const manifest = createTestManifest();
      dispatcher.loadManifest(manifest);

      const mathModule = createMockMathModule();
      dispatcher.registerModule('/src/math.ts', mathModule);

      expect(dispatcher.isReady()).toBe(true);
      expect(dispatcher.getPendingModules()).toHaveLength(0);
    });

    it('warns for missing exports', () => {
      const manifest = createTestManifest();
      dispatcher.loadManifest(manifest);

      const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

      dispatcher.registerModule('/src/math.ts', { clamp: () => 0 }); // missing euclidean

      expect(warnSpy).toHaveBeenCalled();
      warnSpy.mockRestore();
    });

    it('warns for non-function exports', () => {
      const manifest = createTestManifest();
      dispatcher.loadManifest(manifest);

      const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

      dispatcher.registerModule('/src/math.ts', {
        clamp: 'not a function',
        euclidean: () => 0,
      });

      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining('is not a function')
      );
      warnSpy.mockRestore();
    });
  });

  // ===========================================================================
  // Dispatch
  // ===========================================================================

  describe('dispatch', () => {
    beforeEach(() => {
      const manifest = createTestManifest();
      dispatcher.loadManifest(manifest);
      dispatcher.registerModule('/src/math.ts', createMockMathModule());
      dispatcher.setQResolver(createMockQResolver());
      dispatcher.setCoordinateResolver(createMockCoordResolver());
    });

    it('calls registered function with resolved args', () => {
      const calls: PendingFfiCall[] = [{ ffi_id: 1, args: [] }];

      dispatcher.dispatch(calls);

      const results = dispatcher.drainResults();
      expect(results).toHaveLength(1);
      expect(results[0].name).toBe('clamped_value');
      expect(results[0].value).toBe(0.5); // clamp(0.5, 0, 1) = 0.5
    });

    it('uses runtime args when provided', () => {
      const calls: PendingFfiCall[] = [{ ffi_id: 1, args: [1.5, 0, 1] }];

      dispatcher.dispatch(calls);

      const results = dispatcher.drainResults();
      expect(results[0].value).toBe(1); // clamp(1.5, 0, 1) = 1
    });

    it('resolves entity coordinates', () => {
      const calls: PendingFfiCall[] = [{ ffi_id: 2, args: [] }];

      dispatcher.dispatch(calls);

      const results = dispatcher.drainResults();
      expect(results).toHaveLength(1);
      expect(results[0].name).toBe('distance');
      // euclidean(100, 200, 150, 250) = sqrt(50^2 + 50^2) ≈ 70.71
      expect(results[0].value).toBeCloseTo(70.71, 1);
    });

    it('buffers multiple results', () => {
      const calls: PendingFfiCall[] = [
        { ffi_id: 1, args: [0.3, 0, 1] },
        { ffi_id: 1, args: [0.7, 0, 1] },
      ];

      dispatcher.dispatch(calls);

      expect(dispatcher.getBufferedCount()).toBe(2);
    });

    it('handles unknown ffi_id gracefully', () => {
      const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

      const calls: PendingFfiCall[] = [{ ffi_id: 999, args: [] }];
      dispatcher.dispatch(calls);

      expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('Unknown ffi_id'));
      expect(dispatcher.getBufferedCount()).toBe(0);
      warnSpy.mockRestore();
    });

    it('handles function errors gracefully', () => {
      // Register a throwing function
      dispatcher.registerModule('/src/math.ts', {
        clamp: () => {
          throw new Error('Test error');
        },
        euclidean: () => 0,
      });

      const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

      const calls: PendingFfiCall[] = [{ ffi_id: 1, args: [0.5, 0, 1] }];
      dispatcher.dispatch(calls);

      expect(errorSpy).toHaveBeenCalled();
      expect(dispatcher.getBufferedCount()).toBe(0);
      errorSpy.mockRestore();
    });

    it('ignores undefined/null results (side-effect functions)', () => {
      dispatcher.registerModule('/src/math.ts', {
        clamp: () => undefined,
        euclidean: () => null,
      });

      const calls: PendingFfiCall[] = [
        { ffi_id: 1, args: [] },
        { ffi_id: 2, args: [] },
      ];
      dispatcher.dispatch(calls);

      expect(dispatcher.getBufferedCount()).toBe(0);
    });

    it('coerces non-number results to numbers', () => {
      dispatcher.registerModule('/src/math.ts', {
        clamp: () => '42',
        euclidean: () => 0,
      });

      const calls: PendingFfiCall[] = [{ ffi_id: 1, args: [] }];
      dispatcher.dispatch(calls);

      const results = dispatcher.drainResults();
      expect(results[0].value).toBe(42);
    });
  });

  // ===========================================================================
  // Result Draining
  // ===========================================================================

  describe('drainResults', () => {
    beforeEach(() => {
      const manifest = createTestManifest();
      dispatcher.loadManifest(manifest);
      dispatcher.registerModule('/src/math.ts', createMockMathModule());
      dispatcher.setQResolver(createMockQResolver());
    });

    it('returns buffered results', () => {
      dispatcher.dispatch([{ ffi_id: 1, args: [0.5, 0, 1] }]);

      const results = dispatcher.drainResults();

      expect(results).toHaveLength(1);
      expect(results[0]).toMatchObject({
        name: 'clamped_value',
        value: 0.5,
      });
      expect(results[0].timestamp).toBeGreaterThan(0);
    });

    it('clears buffer after drain', () => {
      dispatcher.dispatch([{ ffi_id: 1, args: [0.5, 0, 1] }]);

      dispatcher.drainResults();
      const secondDrain = dispatcher.drainResults();

      expect(secondDrain).toHaveLength(0);
    });

    it('returns empty array when no results', () => {
      const results = dispatcher.drainResults();

      expect(results).toEqual([]);
    });

    it('preserves order of dispatch', () => {
      dispatcher.dispatch([
        { ffi_id: 1, args: [0.1, 0, 1] },
        { ffi_id: 1, args: [0.2, 0, 1] },
        { ffi_id: 1, args: [0.3, 0, 1] },
      ]);

      const results = dispatcher.drainResults();

      expect(results.map((r: FfiResult) => r.value)).toEqual([0.1, 0.2, 0.3]);
    });
  });

  // ===========================================================================
  // Async Dispatch
  // ===========================================================================

  describe('async dispatch', () => {
    beforeEach(() => {
      const manifest = createTestManifest();
      dispatcher.loadManifest(manifest);
      dispatcher.setQResolver(createMockQResolver());
    });

    it('handles async functions returning Promise<number>', async () => {
      dispatcher.registerModule('/src/math.ts', {
        clamp: async (value: number, min: number, max: number) => {
          await new Promise((resolve) => setTimeout(resolve, 10));
          return Math.max(min, Math.min(max, value));
        },
        euclidean: () => 0,
      });

      dispatcher.dispatch([{ ffi_id: 1, args: [0.5, 0, 1] }]);

      // Result not immediately available
      expect(dispatcher.getBufferedCount()).toBe(0);
      expect(dispatcher.getInflightCount()).toBe(1);

      // Wait for async completion
      await new Promise((resolve) => setTimeout(resolve, 20));

      expect(dispatcher.getBufferedCount()).toBe(1);
      expect(dispatcher.getInflightCount()).toBe(0);

      const results = dispatcher.drainResults();
      expect(results[0].value).toBe(0.5);
    });

    it('prevents duplicate dispatch while async call is in-flight', async () => {
      let callCount = 0;
      dispatcher.registerModule('/src/math.ts', {
        clamp: async () => {
          callCount++;
          await new Promise((resolve) => setTimeout(resolve, 50));
          return callCount;
        },
        euclidean: () => 0,
      });

      // First dispatch starts async call
      dispatcher.dispatch([{ ffi_id: 1, args: [] }]);
      expect(dispatcher.isInflight(1)).toBe(true);

      // Second dispatch while in-flight should be skipped
      dispatcher.dispatch([{ ffi_id: 1, args: [] }]);
      dispatcher.dispatch([{ ffi_id: 1, args: [] }]);

      // Wait for completion
      await new Promise((resolve) => setTimeout(resolve, 60));

      // Only one call was made
      expect(callCount).toBe(1);
      expect(dispatcher.getBufferedCount()).toBe(1);
    });

    it('allows new dispatch after async call completes', async () => {
      let callCount = 0;
      dispatcher.registerModule('/src/math.ts', {
        clamp: async () => {
          callCount++;
          await new Promise((resolve) => setTimeout(resolve, 10));
          return callCount * 10;
        },
        euclidean: () => 0,
      });

      // First call
      dispatcher.dispatch([{ ffi_id: 1, args: [] }]);
      await new Promise((resolve) => setTimeout(resolve, 20));

      // Second call after first completes
      dispatcher.dispatch([{ ffi_id: 1, args: [] }]);
      await new Promise((resolve) => setTimeout(resolve, 20));

      expect(callCount).toBe(2);
      const results = dispatcher.drainResults();
      expect(results).toHaveLength(2);
      expect(results[0].value).toBe(10);
      expect(results[1].value).toBe(20);
    });

    it('handles async errors gracefully', async () => {
      dispatcher.registerModule('/src/math.ts', {
        clamp: async () => {
          await new Promise((resolve) => setTimeout(resolve, 10));
          throw new Error('Async failure');
        },
        euclidean: () => 0,
      });

      const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

      dispatcher.dispatch([{ ffi_id: 1, args: [] }]);
      await new Promise((resolve) => setTimeout(resolve, 20));

      expect(errorSpy).toHaveBeenCalled();
      expect(dispatcher.getInflightCount()).toBe(0); // Cleared on error
      expect(dispatcher.getBufferedCount()).toBe(0); // No result buffered

      errorSpy.mockRestore();
    });

    it('mixes sync and async functions correctly', async () => {
      dispatcher.registerModule('/src/math.ts', {
        clamp: (value: number, min: number, max: number) => {
          // Sync function
          return Math.max(min, Math.min(max, value));
        },
        euclidean: async () => {
          // Async function
          await new Promise((resolve) => setTimeout(resolve, 10));
          return 42;
        },
      });

      dispatcher.dispatch([
        { ffi_id: 1, args: [0.5, 0, 1] }, // sync
        { ffi_id: 2, args: [] }, // async
      ]);

      // Sync result immediately available
      expect(dispatcher.getBufferedCount()).toBe(1);

      // Wait for async
      await new Promise((resolve) => setTimeout(resolve, 20));

      expect(dispatcher.getBufferedCount()).toBe(2);
      const results = dispatcher.drainResults();
      expect(results.map((r: FfiResult) => r.value)).toContain(0.5);
      expect(results.map((r: FfiResult) => r.value)).toContain(42);
    });
  });

  // ===========================================================================
  // Factory Function
  // ===========================================================================

  describe('createFfiDispatcher', () => {
    it('creates a new instance', () => {
      const d1 = createFfiDispatcher();
      const d2 = createFfiDispatcher();

      expect(d1).not.toBe(d2);
      expect(d1).toBeInstanceOf(FfiDispatcher);
    });
  });
});
