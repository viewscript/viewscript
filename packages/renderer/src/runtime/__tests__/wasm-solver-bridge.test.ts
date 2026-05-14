/**
 * Tests for WASM Solver Bridge
 *
 * Validates:
 * 1. QSnapshot construction from mutations + FFI results
 * 2. WASM tick() delegation and result parsing
 * 3. FFI result injection lifecycle
 */

import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  WasmSolverBridge,
  createWasmSolverBridge,
  type WasmEngine,
  type TStateMutation,
} from '../wasm-solver-bridge.js';
import type { FfiResult } from '../ffi-dispatcher.js';

// =============================================================================
// Mock WASM Engine
// =============================================================================

function createMockEngine(): WasmEngine & { lastInput: string | null } {
  return {
    lastInput: null,
    tick(inputJson: string): string {
      this.lastInput = inputJson;
      return JSON.stringify({ pending_ffi_calls: [] });
    },
  };
}

function createMockEngineWithCalls(calls: unknown[]): WasmEngine {
  return {
    tick(): string {
      return JSON.stringify({ pending_ffi_calls: calls });
    },
  };
}

// =============================================================================
// Tests
// =============================================================================

describe('WasmSolverBridge', () => {
  let bridge: WasmSolverBridge;
  let mockEngine: WasmEngine & { lastInput: string | null };

  beforeEach(() => {
    mockEngine = createMockEngine();
    bridge = createWasmSolverBridge(mockEngine);
  });

  // ===========================================================================
  // Basic Evaluation
  // ===========================================================================

  describe('evaluate', () => {
    it('calls engine.tick() with QSnapshot JSON', () => {
      const mutations: TStateMutation[] = [];

      bridge.evaluate(mutations);

      expect(mockEngine.lastInput).not.toBeNull();
      const snapshot = JSON.parse(mockEngine.lastInput!);
      expect(snapshot).toHaveProperty('values');
      expect(snapshot).toHaveProperty('mutations');
    });

    it('returns empty bounds and pendingFfiCalls by default', () => {
      const result = bridge.evaluate([]);

      expect(result.bounds.size).toBe(0);
      expect(result.pendingFfiCalls).toEqual([]);
    });

    it('extracts pendingFfiCalls from tick result', () => {
      const calls = [{ ffi_id: 1, args: [1, 2, 3] }];
      bridge = createWasmSolverBridge(createMockEngineWithCalls(calls));

      const result = bridge.evaluate([]);

      expect(result.pendingFfiCalls).toEqual(calls);
    });
  });

  // ===========================================================================
  // Mutation Conversion
  // ===========================================================================

  describe('mutation conversion', () => {
    it('converts TStateMutation to QSnapshot values', () => {
      const mutations: TStateMutation[] = [
        { entityId: 1, state: 'hover', value: 1, timestamp: 100 },
        { entityId: 2, state: 'scroll_y', value: 0.5, timestamp: 100 },
      ];

      bridge.evaluate(mutations);

      const snapshot = JSON.parse(mockEngine.lastInput!);
      expect(snapshot.values['1_hover']).toEqual({ type: 'float', value: 1 });
      expect(snapshot.values['2_scroll_y']).toEqual({ type: 'float', value: 0.5 });
    });

    it('includes mutations array for legacy format', () => {
      const mutations: TStateMutation[] = [
        { entityId: 1, state: 'pressed', value: 1, timestamp: 100 },
      ];

      bridge.evaluate(mutations);

      const snapshot = JSON.parse(mockEngine.lastInput!);
      expect(snapshot.mutations).toHaveLength(1);
      expect(snapshot.mutations[0]).toMatchObject({
        entity_id: 1,
        state: 'pressed',
        value: 1,
      });
    });
  });

  // ===========================================================================
  // FFI Result Injection
  // ===========================================================================

  describe('injectFfiResults', () => {
    it('includes injected FFI results in QSnapshot values', () => {
      const ffiResults: FfiResult[] = [
        { name: 'clamped_opacity', value: 0.75, timestamp: 100 },
        { name: 'distance', value: 42, timestamp: 100 },
      ];

      bridge.injectFfiResults(ffiResults);
      bridge.evaluate([]);

      const snapshot = JSON.parse(mockEngine.lastInput!);
      expect(snapshot.values['clamped_opacity']).toEqual({ type: 'float', value: 0.75 });
      expect(snapshot.values['distance']).toEqual({ type: 'float', value: 42 });
    });

    it('clears FFI results after evaluate()', () => {
      bridge.injectFfiResults([{ name: 'test', value: 1, timestamp: 100 }]);

      // First evaluate includes FFI result
      bridge.evaluate([]);
      let snapshot = JSON.parse(mockEngine.lastInput!);
      expect(snapshot.values['test']).toBeDefined();

      // Second evaluate should NOT include FFI result
      bridge.evaluate([]);
      snapshot = JSON.parse(mockEngine.lastInput!);
      expect(snapshot.values['test']).toBeUndefined();
    });

    it('FFI results override mutation values with same name', () => {
      // This tests the merge order: FFI results come first, then mutations
      // If there's a naming collision, the mutation should win
      const ffiResults: FfiResult[] = [
        { name: '1_hover', value: 0.5, timestamp: 100 },
      ];
      const mutations: TStateMutation[] = [
        { entityId: 1, state: 'hover', value: 1, timestamp: 100 },
      ];

      bridge.injectFfiResults(ffiResults);
      bridge.evaluate(mutations);

      const snapshot = JSON.parse(mockEngine.lastInput!);
      // Mutation overwrites FFI result (processed later)
      expect(snapshot.values['1_hover'].value).toBe(1);
    });
  });

  // ===========================================================================
  // Error Handling
  // ===========================================================================

  describe('error handling', () => {
    it('handles engine.tick() throwing', () => {
      const errorEngine: WasmEngine = {
        tick(): string {
          throw new Error('WASM panic');
        },
      };
      bridge = createWasmSolverBridge(errorEngine);

      const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

      const result = bridge.evaluate([]);

      expect(result.bounds.size).toBe(0);
      expect(result.pendingFfiCalls).toEqual([]);
      expect(errorSpy).toHaveBeenCalled();

      errorSpy.mockRestore();
    });

    it('preserves FFI results when tick() throws for next frame retry', () => {
      let callCount = 0;
      const flakyEngine: WasmEngine & { lastInput: string | null } = {
        lastInput: null,
        tick(inputJson: string): string {
          callCount++;
          if (callCount === 1) {
            throw new Error('Transient WASM error');
          }
          this.lastInput = inputJson;
          return JSON.stringify({ pending_ffi_calls: [] });
        },
      };
      bridge = createWasmSolverBridge(flakyEngine);

      const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

      // Inject FFI results
      bridge.injectFfiResults([{ name: 'important_value', value: 42, timestamp: 100 }]);

      // First evaluate fails - FFI results should be preserved
      bridge.evaluate([]);
      expect(errorSpy).toHaveBeenCalled();

      // Second evaluate succeeds - FFI results should still be included
      bridge.evaluate([]);
      const snapshot = JSON.parse(flakyEngine.lastInput!);
      expect(snapshot.values['important_value']).toEqual({ type: 'float', value: 42 });

      errorSpy.mockRestore();
    });

    it('handles invalid JSON from tick()', () => {
      const badEngine: WasmEngine = {
        tick(): string {
          return 'not valid json';
        },
      };
      bridge = createWasmSolverBridge(badEngine);

      const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

      const result = bridge.evaluate([]);

      expect(result.pendingFfiCalls).toEqual([]);
      expect(errorSpy).toHaveBeenCalled();

      errorSpy.mockRestore();
    });

    it('handles missing pending_ffi_calls in result', () => {
      const partialEngine: WasmEngine = {
        tick(): string {
          return JSON.stringify({}); // No pending_ffi_calls field
        },
      };
      bridge = createWasmSolverBridge(partialEngine);

      const result = bridge.evaluate([]);

      expect(result.pendingFfiCalls).toEqual([]);
    });
  });

  // ===========================================================================
  // Factory Function
  // ===========================================================================

  describe('createWasmSolverBridge', () => {
    it('creates a new instance', () => {
      const engine = createMockEngine();
      const b1 = createWasmSolverBridge(engine);
      const b2 = createWasmSolverBridge(engine);

      expect(b1).not.toBe(b2);
      expect(b1).toBeInstanceOf(WasmSolverBridge);
    });
  });
});
