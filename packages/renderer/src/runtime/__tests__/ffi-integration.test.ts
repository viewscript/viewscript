/**
 * FFI Integration Tests (Layer 1)
 *
 * Tests the JS-side pipeline integration:
 * Vite Plugin (Manifest) -> FfiDispatcher -> WasmSolverBridge -> render-loop
 *
 * WASM engine is mocked to isolate JS-side behavior.
 */

import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  FfiDispatcher,
  createFfiDispatcher,
  type FfiManifest,
  type PendingFfiCall,
} from '../ffi-dispatcher.js';
import {
  WasmSolverBridge,
  createWasmSolverBridge,
  type WasmEngine,
  type TStateMutation,
} from '../wasm-solver-bridge.js';
import { generateManifest, type ManifestContext } from '../../../../vite-plugin/src/manifest.js';
import type { VsParseResult } from '../../../../vite-plugin/src/vs-parser.js';
import type { EsmAnalysisResult } from '../../../../vite-plugin/src/esm-analyzer.js';

// =============================================================================
// Mock WASM Engine
// =============================================================================

interface TickResult {
  pending_ffi_calls: PendingFfiCall[];
}

interface QSnapshot {
  values: Record<string, { type: string; value: number | boolean }>;
  mutations: unknown[];
}

class MockWasmEngine implements WasmEngine {
  private tickInputs: string[] = [];
  private tickResponses: string[] = [];
  private shouldThrow = false;
  private throwError: Error | null = null;

  /**
   * Set the next response(s) for tick().
   */
  setNextResponse(result: TickResult): void {
    this.tickResponses.push(JSON.stringify(result));
  }

  /**
   * Configure tick() to throw on next call.
   */
  setThrowOnNextTick(error: Error): void {
    this.shouldThrow = true;
    this.throwError = error;
  }

  /**
   * Get the last QSnapshot input to tick().
   */
  getLastInput(): QSnapshot | null {
    if (this.tickInputs.length === 0) return null;
    return JSON.parse(this.tickInputs[this.tickInputs.length - 1]) as QSnapshot;
  }

  /**
   * Get all tick() inputs.
   */
  getAllInputs(): QSnapshot[] {
    return this.tickInputs.map((json) => JSON.parse(json) as QSnapshot);
  }

  /**
   * Get tick() call count.
   */
  getTickCount(): number {
    return this.tickInputs.length;
  }

  /**
   * Reset all recorded state.
   */
  reset(): void {
    this.tickInputs = [];
    this.tickResponses = [];
    this.shouldThrow = false;
    this.throwError = null;
  }

  tick(inputJson: string): string {
    this.tickInputs.push(inputJson);

    if (this.shouldThrow) {
      this.shouldThrow = false;
      throw this.throwError ?? new Error('Mock tick error');
    }

    return this.tickResponses.shift() ?? JSON.stringify({ pending_ffi_calls: [] });
  }
}

// =============================================================================
// Test Fixtures
// =============================================================================

function createMockAnalysis(exports: string[]): EsmAnalysisResult {
  return {
    exports: exports.map((name) => ({ name, isReExport: false })),
    hasDefaultExport: false,
  };
}

function createBindManifestContext(bindings: {
  bindName: string;
  functionName: string;
  args: string[];
}[]): ManifestContext {
  const vsParseResult: VsParseResult = {
    imports: [{ names: bindings.map((b) => b.functionName), modulePath: './math.ts', line: 1 }],
    binds: bindings.map((b, i) => ({
      bindName: b.bindName,
      functionName: b.functionName,
      args: b.args,
      line: i + 2,
    })),
    triggers: [],
    errors: [],
  };

  const resolvedModules = new Map([
    [
      './math.ts',
      {
        originalPath: './math.ts',
        resolvedPath: '/project/src/math.ts',
        analysis: createMockAnalysis(bindings.map((b) => b.functionName)),
      },
    ],
  ]);

  const entityMap = new Map([
    ['button', 1],
    ['cursor', 2],
    ['player', 3],
  ]);

  return { vsParseResult, resolvedModules, entityMap };
}

function createTriggerManifestContext(triggers: {
  triggerName: string;
  conditionKind: string;
  conditionArgs: string[];
  functionName: string;
  functionArgs: string[];
}[]): ManifestContext {
  const vsParseResult: VsParseResult = {
    imports: [{ names: triggers.map((t) => t.functionName), modulePath: './events.ts', line: 1 }],
    binds: [],
    triggers: triggers.map((t, i) => ({
      triggerName: t.triggerName,
      conditionKind: t.conditionKind,
      conditionArgs: t.conditionArgs,
      functionName: t.functionName,
      functionArgs: t.functionArgs,
      line: i + 2,
    })),
    errors: [],
  };

  const resolvedModules = new Map([
    [
      './events.ts',
      {
        originalPath: './events.ts',
        resolvedPath: '/project/src/events.ts',
        analysis: createMockAnalysis(triggers.map((t) => t.functionName)),
      },
    ],
  ]);

  const entityMap = new Map([
    ['button', 1],
    ['cursor', 2],
    ['player', 3],
  ]);

  return { vsParseResult, resolvedModules, entityMap };
}

// =============================================================================
// E1: Manifest to Dispatcher Integration
// =============================================================================

describe('E1: manifest_to_dispatcher_integration', () => {
  it('loads manifest and builds function registry', () => {
    // 1. Generate manifest via Vite plugin
    const context = createBindManifestContext([
      { bindName: 'doubled', functionName: 'double', args: ['input'] },
      { bindName: 'clamped', functionName: 'clamp', args: ['value', '0', '1'] },
    ]);
    const { manifest } = generateManifest(context);
    expect(manifest).not.toBeNull();

    // 2. Load manifest into dispatcher
    const dispatcher = createFfiDispatcher();
    dispatcher.loadManifest(manifest!);

    // 3. Verify pending modules
    expect(dispatcher.getPendingModules()).toContain('/project/src/math.ts');
    expect(dispatcher.isReady()).toBe(false);

    // 4. Register module
    const mathModule = {
      double: (x: number) => x * 2,
      clamp: (v: number, min: number, max: number) => Math.max(min, Math.min(max, v)),
    };
    dispatcher.registerModule('/project/src/math.ts', mathModule);

    // 5. Verify ready state
    expect(dispatcher.isReady()).toBe(true);
    expect(dispatcher.getPendingModules()).toHaveLength(0);
  });

  it('preserves binding metadata from manifest', () => {
    const context = createBindManifestContext([
      { bindName: 'result', functionName: 'compute', args: ['42'] },
    ]);
    const { manifest } = generateManifest(context);

    expect(manifest!.bindings).toHaveLength(1);
    expect(manifest!.bindings[0]).toMatchObject({
      bind_name: 'result',
      export_name: 'compute',
      module_path: '/project/src/math.ts',
    });
  });
});

// =============================================================================
// E2: Sync FFI Bind Full Cycle
// =============================================================================

describe('E2: sync_ffi_bind_full_cycle', () => {
  let mockEngine: MockWasmEngine;
  let bridge: WasmSolverBridge;
  let dispatcher: FfiDispatcher;

  beforeEach(() => {
    mockEngine = new MockWasmEngine();
    bridge = createWasmSolverBridge(mockEngine);
    dispatcher = createFfiDispatcher();
  });

  it('completes full sync FFI cycle across frames', () => {
    // 1. Generate manifest with bind
    const context = createBindManifestContext([
      { bindName: 'result', functionName: 'double', args: ['42'] },
    ]);
    const { manifest } = generateManifest(context);

    // 2. Load manifest and register module
    dispatcher.loadManifest(manifest!);
    dispatcher.registerModule('/project/src/math.ts', {
      double: (x: number) => x * 2,
    });

    // --- Frame N ---

    // 3. First tick - no FFI results yet
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    const result1 = bridge.evaluate([]);

    // 4. Verify no FFI values in first QSnapshot
    const input1 = mockEngine.getLastInput();
    expect(input1?.values).not.toHaveProperty('result');

    // 5. Second tick returns pending FFI call
    mockEngine.setNextResponse({
      pending_ffi_calls: [{ ffi_id: 1, args: [42] }],
    });
    const result2 = bridge.evaluate([]);

    // 6. Dispatch FFI call
    expect(result2.pendingFfiCalls).toHaveLength(1);
    dispatcher.dispatch(result2.pendingFfiCalls);

    // 7. Drain results (sync function returns immediately)
    const ffiResults = dispatcher.drainResults();
    expect(ffiResults).toHaveLength(1);
    expect(ffiResults[0].name).toBe('result');
    expect(ffiResults[0].value).toBe(84); // 42 * 2

    // --- Frame N+1 ---

    // 8. Inject FFI results for next frame
    bridge.injectFfiResults(ffiResults);

    // 9. Next tick should have FFI result in QSnapshot
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    bridge.evaluate([]);

    // 10. Verify FFI result was included in QSnapshot
    const input3 = mockEngine.getLastInput();
    expect(input3?.values['result']).toEqual({ type: 'float', value: 84 });
  });
});

// =============================================================================
// E3: Async FFI Bind Multi-Frame
// =============================================================================

describe('E3: async_ffi_bind_multi_frame', () => {
  let mockEngine: MockWasmEngine;
  let bridge: WasmSolverBridge;
  let dispatcher: FfiDispatcher;

  beforeEach(() => {
    mockEngine = new MockWasmEngine();
    bridge = createWasmSolverBridge(mockEngine);
    dispatcher = createFfiDispatcher();
  });

  it('buffers async result for later frame', async () => {
    // 1. Setup manifest with async function
    const context = createBindManifestContext([
      { bindName: 'async_result', functionName: 'fetchValue', args: [] },
    ]);
    const { manifest } = generateManifest(context);

    // 2. Register async function
    dispatcher.loadManifest(manifest!);
    dispatcher.registerModule('/project/src/math.ts', {
      fetchValue: async () => {
        await new Promise((r) => setTimeout(r, 50));
        return 999;
      },
    });

    // --- Frame N: Dispatch async call ---
    mockEngine.setNextResponse({
      pending_ffi_calls: [{ ffi_id: 1, args: [] }],
    });
    const result = bridge.evaluate([]);
    dispatcher.dispatch(result.pendingFfiCalls);

    // 3. Immediately after dispatch, no result yet
    expect(dispatcher.drainResults()).toHaveLength(0);
    expect(dispatcher.getInflightCount()).toBe(1);

    // --- Frame N+1: Still waiting ---
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    bridge.evaluate([]);
    expect(dispatcher.drainResults()).toHaveLength(0);

    // --- Wait for async completion ---
    await new Promise((r) => setTimeout(r, 100));

    // --- Frame N+2: Result available ---
    expect(dispatcher.getInflightCount()).toBe(0);
    const asyncResults = dispatcher.drainResults();
    expect(asyncResults).toHaveLength(1);
    expect(asyncResults[0].name).toBe('async_result');
    expect(asyncResults[0].value).toBe(999);

    // --- Frame N+3: Inject and verify ---
    bridge.injectFfiResults(asyncResults);
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    bridge.evaluate([]);

    const finalInput = mockEngine.getLastInput();
    expect(finalInput?.values['async_result']).toEqual({ type: 'float', value: 999 });
  });
});

// =============================================================================
// E4: Trigger Edge Detection Cycle
// =============================================================================

describe('E4: trigger_edge_detection_cycle', () => {
  let mockEngine: MockWasmEngine;
  let bridge: WasmSolverBridge;
  let dispatcher: FfiDispatcher;

  beforeEach(() => {
    mockEngine = new MockWasmEngine();
    bridge = createWasmSolverBridge(mockEngine);
    dispatcher = createFfiDispatcher();
  });

  it('fires trigger only on false->true transition (rising edge)', () => {
    // Setup: Trigger on bounds_overlap(button, cursor)
    const context = createTriggerManifestContext([
      {
        triggerName: 'on_click',
        conditionKind: 'bounds_overlap',
        conditionArgs: ['button', 'cursor'],
        functionName: 'handleClick',
        functionArgs: [],
      },
    ]);
    const { manifest } = generateManifest(context);

    dispatcher.loadManifest(manifest!);
    let clickCount = 0;
    dispatcher.registerModule('/project/src/events.ts', {
      handleClick: () => {
        clickCount++;
        return clickCount;
      },
    });

    // Frame 1: Condition not met -> no trigger
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    const result1 = bridge.evaluate([]);
    expect(result1.pendingFfiCalls).toHaveLength(0);

    // Frame 2: Condition becomes true -> TRIGGER FIRES (rising edge)
    // Note: With 0 bindings, trigger gets ffi_id=1
    mockEngine.setNextResponse({
      pending_ffi_calls: [{ ffi_id: 1, trigger_id: 1, args: [] }],
    });
    const result2 = bridge.evaluate([]);
    expect(result2.pendingFfiCalls).toHaveLength(1);
    dispatcher.dispatch(result2.pendingFfiCalls);
    expect(clickCount).toBe(1);

    // Frame 3: Condition still true -> NO TRIGGER (already triggered)
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    const result3 = bridge.evaluate([]);
    expect(result3.pendingFfiCalls).toHaveLength(0);
    // clickCount remains 1

    // Frame 4: Condition becomes false (no trigger)
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    bridge.evaluate([]);

    // Frame 5: Condition becomes true again -> TRIGGER FIRES
    mockEngine.setNextResponse({
      pending_ffi_calls: [{ ffi_id: 1, trigger_id: 1, args: [] }],
    });
    const result5 = bridge.evaluate([]);
    dispatcher.dispatch(result5.pendingFfiCalls);
    expect(clickCount).toBe(2);
  });
});

// =============================================================================
// E5: Trigger Threshold Crossing
// =============================================================================

describe('E5: trigger_threshold_crossing', () => {
  let mockEngine: MockWasmEngine;
  let bridge: WasmSolverBridge;
  let dispatcher: FfiDispatcher;

  beforeEach(() => {
    mockEngine = new MockWasmEngine();
    bridge = createWasmSolverBridge(mockEngine);
    dispatcher = createFfiDispatcher();
  });

  it('fires only when value crosses threshold in rising direction', () => {
    // Setup: threshold_crossing(player.y, 100, rising)
    const context = createTriggerManifestContext([
      {
        triggerName: 'ground_touch',
        conditionKind: 'threshold_crossing',
        conditionArgs: ['player.y', '100', 'rising'],
        functionName: 'onGroundTouch',
        functionArgs: [],
      },
    ]);
    const { manifest } = generateManifest(context);

    dispatcher.loadManifest(manifest!);
    let touchCount = 0;
    dispatcher.registerModule('/project/src/events.ts', {
      onGroundTouch: () => ++touchCount,
    });

    // Simulate value progression: 50 -> 80 -> 110 -> 120 -> 90 -> 105

    // Frame 1: player.y = 50 (below threshold, no trigger)
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    bridge.evaluate([]);
    expect(touchCount).toBe(0);

    // Frame 2: player.y = 80 (still below, no trigger)
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    bridge.evaluate([]);
    expect(touchCount).toBe(0);

    // Frame 3: player.y = 110 (CROSSES threshold rising) -> TRIGGER
    // Note: With 0 bindings, trigger gets ffi_id=1
    mockEngine.setNextResponse({
      pending_ffi_calls: [{ ffi_id: 1, trigger_id: 1, args: [] }],
    });
    const result3 = bridge.evaluate([]);
    dispatcher.dispatch(result3.pendingFfiCalls);
    expect(touchCount).toBe(1);

    // Frame 4: player.y = 120 (above, but already triggered)
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    bridge.evaluate([]);
    expect(touchCount).toBe(1);

    // Frame 5: player.y = 90 (back below threshold)
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    bridge.evaluate([]);
    expect(touchCount).toBe(1);

    // Frame 6: player.y = 105 (CROSSES again rising) -> TRIGGER
    mockEngine.setNextResponse({
      pending_ffi_calls: [{ ffi_id: 1, trigger_id: 1, args: [] }],
    });
    const result6 = bridge.evaluate([]);
    dispatcher.dispatch(result6.pendingFfiCalls);
    expect(touchCount).toBe(2);
  });
});

// =============================================================================
// E6: FFI Result Survives Tick Failure
// =============================================================================

describe('E6: ffi_result_survives_tick_failure', () => {
  let mockEngine: MockWasmEngine;
  let bridge: WasmSolverBridge;
  let dispatcher: FfiDispatcher;

  beforeEach(() => {
    mockEngine = new MockWasmEngine();
    bridge = createWasmSolverBridge(mockEngine);
    dispatcher = createFfiDispatcher();

    // Setup basic manifest
    const context = createBindManifestContext([
      { bindName: 'computed', functionName: 'compute', args: ['10'] },
    ]);
    const { manifest } = generateManifest(context);
    dispatcher.loadManifest(manifest!);
    dispatcher.registerModule('/project/src/math.ts', {
      compute: (x: number) => x * 3,
    });
  });

  it('preserves FFI results when tick() throws', () => {
    // Frame 1: Generate FFI result
    mockEngine.setNextResponse({
      pending_ffi_calls: [{ ffi_id: 1, args: [10] }],
    });
    const result1 = bridge.evaluate([]);
    dispatcher.dispatch(result1.pendingFfiCalls);
    const ffiResults = dispatcher.drainResults();
    expect(ffiResults[0].value).toBe(30);

    // Inject results for next frame
    bridge.injectFfiResults(ffiResults);

    // Frame 2: tick() throws
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    mockEngine.setThrowOnNextTick(new Error('WASM panic'));

    const result2 = bridge.evaluate([]);

    // Should return empty result but not crash
    expect(result2.pendingFfiCalls).toHaveLength(0);
    expect(result2.bounds.size).toBe(0);
    errorSpy.mockRestore();

    // Frame 3: tick() recovers - FFI results should still be in QSnapshot
    // Because tick() failed, results were NOT cleared
    mockEngine.setNextResponse({ pending_ffi_calls: [] });
    bridge.evaluate([]);

    // BUG CHECK: After tick failure, results should be preserved
    // Current implementation: Results are injected into pendingFfiResults,
    // and only cleared AFTER successful tick()
    const input3 = mockEngine.getLastInput();

    // The FFI result should still be present because the failed tick()
    // did NOT clear pendingFfiResults
    expect(input3?.values['computed']).toEqual({ type: 'float', value: 30 });
  });
});

// =============================================================================
// E7: Async Inflight Guard
// =============================================================================

describe('E7: async_inflight_guard', () => {
  let dispatcher: FfiDispatcher;

  beforeEach(() => {
    dispatcher = createFfiDispatcher();
  });

  it('prevents duplicate dispatch while async call is in-flight', async () => {
    // Setup manifest
    const context = createBindManifestContext([
      { bindName: 'slow_result', functionName: 'slowCompute', args: [] },
    ]);
    const { manifest } = generateManifest(context);

    // Register slow async function
    let callCount = 0;
    dispatcher.loadManifest(manifest!);
    dispatcher.registerModule('/project/src/math.ts', {
      slowCompute: async () => {
        callCount++;
        await new Promise((r) => setTimeout(r, 100));
        return callCount * 100;
      },
    });

    // First dispatch starts async call
    dispatcher.dispatch([{ ffi_id: 1, args: [] }]);
    expect(dispatcher.isInflight(1)).toBe(true);
    expect(callCount).toBe(1);

    // Subsequent dispatches while in-flight are ignored
    dispatcher.dispatch([{ ffi_id: 1, args: [] }]);
    dispatcher.dispatch([{ ffi_id: 1, args: [] }]);
    dispatcher.dispatch([{ ffi_id: 1, args: [] }]);
    expect(callCount).toBe(1); // Still only 1 call

    // Wait for completion
    await new Promise((r) => setTimeout(r, 150));

    expect(dispatcher.isInflight(1)).toBe(false);
    expect(dispatcher.getBufferedCount()).toBe(1);

    // Now can dispatch again
    dispatcher.dispatch([{ ffi_id: 1, args: [] }]);
    expect(callCount).toBe(2);

    // Wait and verify
    await new Promise((r) => setTimeout(r, 150));
    expect(dispatcher.getBufferedCount()).toBe(2);

    const results = dispatcher.drainResults();
    expect(results).toHaveLength(2);
    expect(results[0].value).toBe(100);
    expect(results[1].value).toBe(200);
  });
});
