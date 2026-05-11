/**
 * Performance Profiling Tests (Jank & Backpressure Verification)
 *
 * This module verifies that the render loop maintains 60fps under
 * extreme stress conditions using Chrome DevTools Protocol (CDP).
 *
 * ## Performance Targets
 *
 * | Metric                    | Target          | Rationale                    |
 * |---------------------------|-----------------|------------------------------|
 * | Frame Time p99            | < 16.6ms        | 60fps guarantee              |
 * | Forced Sync Layout        | = 0             | No layout thrashing          |
 * | Event Coalesce Rate       | > 90%           | Backpressure working         |
 * | Constraint Solve Time p95 | < 10ms          | Leave headroom for render    |
 *
 * ## Stress Test Scenarios
 *
 * 1. Event Storm: 1000 mousemove events per frame
 * 2. Complex Graph: 10,000 constraint nodes
 * 3. Combined: Both simultaneously
 */

import { test, expect, type Page, type CDPSession } from '@playwright/test';

// =============================================================================
// Performance Thresholds
// =============================================================================

const PERF_THRESHOLDS = {
  /** 60fps = 16.666ms per frame */
  FRAME_TIME_P99_MS: 16.6,

  /** Zero tolerance for layout thrashing */
  MAX_FORCED_SYNC_LAYOUT: 0,

  /** Minimum event coalesce rate */
  MIN_COALESCE_RATE: 0.9,

  /** Constraint solver budget (leave 6ms for rendering) */
  CONSTRAINT_SOLVE_P95_MS: 10,

  /** Test duration */
  STRESS_TEST_DURATION_MS: 5000,
};

// =============================================================================
// Types
// =============================================================================

interface PerformanceMetrics {
  frameTimes: number[];
  forcedSyncLayoutCount?: number;
  totalEventsReceived: number;
  totalEventsProcessed: number;
  constraintSolveTimes: number[];
}

interface PerformanceReport {
  frameTimeP50: number;
  frameTimeP95: number;
  frameTimeP99: number;
  frameTimeMax: number;
  forcedSyncLayoutCount: number;
  coalesceRate: number;
  constraintSolveP95: number;
  passed: boolean;
  failures: string[];
}

// =============================================================================
// Performance Tests
// =============================================================================

test.describe('Performance Profiling: Jank & Backpressure', () => {

  /**
   * Test: Backpressure under event storm
   *
   * Inject 1000 mousemove events per frame, verify coalescing.
   */
  test('backpressure coalesces 1000 events/frame to < 50', async ({ page }) => {
    const cdp = await page.context().newCDPSession(page);
    await setupTestPage(page);

    // Enable performance tracing
    await enablePerformanceTracing(cdp);

    // Start the event storm
    const metrics = await runEventStormTest(page, {
      eventsPerFrame: 1000,
      durationMs: PERF_THRESHOLDS.STRESS_TEST_DURATION_MS,
    });

    // Stop tracing and collect
    await disablePerformanceTracing(cdp);

    // Analyze results
    const report = analyzeMetrics(metrics);

    console.log('=== Backpressure Test Results ===');
    console.log(`Events received:  ${metrics.totalEventsReceived}`);
    console.log(`Events processed: ${metrics.totalEventsProcessed}`);
    console.log(`Coalesce rate:    ${(report.coalesceRate * 100).toFixed(1)}%`);

    // Assertions
    expect(report.coalesceRate).toBeGreaterThanOrEqual(PERF_THRESHOLDS.MIN_COALESCE_RATE);
  });

  /**
   * Test: 10,000 node constraint graph at 60fps
   *
   * Render and animate a massive constraint graph.
   */
  test('maintains 60fps with 10,000 constraint nodes', async ({ page }) => {
    const cdp = await page.context().newCDPSession(page);
    await setupTestPage(page);

    await enablePerformanceTracing(cdp);

    // Generate and render massive constraint graph
    const metrics = await runComplexGraphTest(page, {
      nodeCount: 10000,
      durationMs: PERF_THRESHOLDS.STRESS_TEST_DURATION_MS,
    });

    await disablePerformanceTracing(cdp);

    const report = analyzeMetrics(metrics);

    console.log('=== Complex Graph Test Results ===');
    console.log(`Frame Time p50:  ${report.frameTimeP50.toFixed(2)}ms`);
    console.log(`Frame Time p95:  ${report.frameTimeP95.toFixed(2)}ms`);
    console.log(`Frame Time p99:  ${report.frameTimeP99.toFixed(2)}ms`);
    console.log(`Frame Time max:  ${report.frameTimeMax.toFixed(2)}ms`);
    console.log(`Constraint Solve p95: ${report.constraintSolveP95.toFixed(2)}ms`);

    // Assertions
    expect(report.frameTimeP99).toBeLessThan(PERF_THRESHOLDS.FRAME_TIME_P99_MS);
    expect(report.constraintSolveP95).toBeLessThan(PERF_THRESHOLDS.CONSTRAINT_SOLVE_P95_MS);
  });

  /**
   * Test: Zero forced synchronous layouts
   *
   * Verify that our atomic render loop never causes layout thrashing.
   */
  test('zero forced synchronous layouts during rendering', async ({ page }) => {
    const cdp = await page.context().newCDPSession(page);
    await setupTestPage(page);

    // Enable Layout Instability detection
    await cdp.send('Performance.enable');

    const metrics = await runCombinedStressTest(page, {
      eventsPerFrame: 100,
      nodeCount: 1000,
      durationMs: PERF_THRESHOLDS.STRESS_TEST_DURATION_MS,
    });

    // Get layout metrics from CDP
    const layoutMetrics = await getLayoutMetrics(cdp);

    console.log('=== Layout Thrashing Test Results ===');
    console.log(`Forced Sync Layouts: ${layoutMetrics.forcedSyncLayoutCount}`);

    // CRITICAL: Zero tolerance
    expect(layoutMetrics.forcedSyncLayoutCount).toBe(PERF_THRESHOLDS.MAX_FORCED_SYNC_LAYOUT);
  });

  /**
   * Test: Combined stress (event storm + complex graph)
   *
   * The ultimate stress test: everything at once.
   */
  test('combined stress maintains performance invariants', async ({ page }) => {
    const cdp = await page.context().newCDPSession(page);
    await setupTestPage(page);

    await enablePerformanceTracing(cdp);

    const metrics = await runCombinedStressTest(page, {
      eventsPerFrame: 1000,
      nodeCount: 10000,
      durationMs: PERF_THRESHOLDS.STRESS_TEST_DURATION_MS,
    });

    await disablePerformanceTracing(cdp);

    const report = analyzeMetrics(metrics);

    console.log('=== Combined Stress Test Results ===');
    console.log(`Frame Time p99:       ${report.frameTimeP99.toFixed(2)}ms`);
    console.log(`Coalesce Rate:        ${(report.coalesceRate * 100).toFixed(1)}%`);
    console.log(`Forced Sync Layouts:  ${report.forcedSyncLayoutCount}`);
    console.log(`Test Passed:          ${report.passed ? 'YES' : 'NO'}`);
    if (!report.passed) {
      console.log(`Failures: ${report.failures.join(', ')}`);
    }

    // All invariants must hold
    expect(report.passed).toBe(true);
  });
});

// =============================================================================
// CDP Helpers
// =============================================================================

async function enablePerformanceTracing(cdp: CDPSession): Promise<void> {
  await cdp.send('Performance.enable');
  await cdp.send('Tracing.start', {
    categories: 'devtools.timeline,v8.execute,blink.user_timing',
    options: 'sampling-frequency=10000',
  });
}

async function disablePerformanceTracing(cdp: CDPSession): Promise<void> {
  await cdp.send('Tracing.end');
}

async function getLayoutMetrics(cdp: CDPSession): Promise<{ forcedSyncLayoutCount: number }> {
  const result = await cdp.send('Performance.getMetrics');
  const metrics = result.metrics;

  // CRITICAL: Use correct metric for forced synchronous layouts
  // LayoutCount = total layouts (includes async)
  // ForcedStyleAndLayoutDuration > 0 indicates forced sync layouts occurred
  // RecalcStyleCount can also indicate style thrashing
  const forcedStyleDuration = metrics.find(m => m.name === 'ForcedStyleAndLayoutDuration');
  const layoutDuration = metrics.find(m => m.name === 'LayoutDuration');

  // Heuristic: If forced style duration is significant (>1ms), we have forced layouts
  // More accurate than LayoutCount which doesn't distinguish sync vs async
  const hasForcedLayouts = (forcedStyleDuration?.value ?? 0) > 0.001;

  return {
    forcedSyncLayoutCount: hasForcedLayouts ? 1 : 0,
  };
}

// =============================================================================
// Test Scenarios
// =============================================================================

async function setupTestPage(page: Page): Promise<void> {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => (window as any).__VS_RENDERER_READY__ === true, {
    timeout: 10000,
  });

  // Initialize metrics collection
  await page.evaluate(() => {
    (window as any).__VS_PERF_METRICS__ = {
      frameTimes: [],
      constraintSolveTimes: [],
      totalEventsReceived: 0,
      totalEventsProcessed: 0,
      forcedSyncLayoutCount: 0,
    };

    // Track frame-to-frame timing (actual frame duration)
    // This measures the real interval between animation frames,
    // NOT just JS execution time within a frame
    let lastFrameTime: number | null = null;

    const measureFrameTime = (timestamp: number) => {
      if (lastFrameTime !== null) {
        const frameInterval = timestamp - lastFrameTime;
        (window as any).__VS_PERF_METRICS__.frameTimes.push(frameInterval);
      }
      lastFrameTime = timestamp;
      requestAnimationFrame(measureFrameTime);
    };
    requestAnimationFrame(measureFrameTime);

    // Hook into render loop for constraint solve timing only
    const originalTick = (window as any).__VS_RENDERER__.tick;
    (window as any).__VS_RENDERER__.tick = function(ts: number) {
      const start = performance.now();
      originalTick.call(this, ts);
      const elapsed = performance.now() - start;
      (window as any).__VS_PERF_METRICS__.constraintSolveTimes.push(elapsed);
    };
  });
}

async function runEventStormTest(
  page: Page,
  config: { eventsPerFrame: number; durationMs: number },
): Promise<PerformanceMetrics> {
  await page.evaluate(async (cfg) => {
    const startTime = performance.now();
    const metrics = (window as any).__VS_PERF_METRICS__;

    const runFrame = () => {
      // Inject N synthetic mousemove events
      for (let i = 0; i < cfg.eventsPerFrame; i++) {
        const event = new MouseEvent('mousemove', {
          clientX: Math.random() * 800,
          clientY: Math.random() * 600,
        });
        document.dispatchEvent(event);
        metrics.totalEventsReceived++;
      }

      if (performance.now() - startTime < cfg.durationMs) {
        requestAnimationFrame(runFrame);
      }
    };

    requestAnimationFrame(runFrame);

    // Wait for test completion
    await new Promise(resolve => setTimeout(resolve, cfg.durationMs + 100));

    // Get processed count from EventBuffer
    metrics.totalEventsProcessed = (window as any).__VS_RENDERER__.getProcessedEventCount();
  }, config);

  return await page.evaluate(() => (window as any).__VS_PERF_METRICS__);
}

async function runComplexGraphTest(
  page: Page,
  config: { nodeCount: number; durationMs: number },
): Promise<PerformanceMetrics> {
  await page.evaluate(async (cfg) => {
    const renderer = (window as any).__VS_RENDERER__;
    const metrics = (window as any).__VS_PERF_METRICS__;

    // Generate massive constraint graph
    const entities = [];
    const constraints = [];

    for (let i = 0; i < cfg.nodeCount; i++) {
      entities.push({
        id: i,
        type: 'rect',
        bounds: {
          x: (i % 100) * 10,
          y: Math.floor(i / 100) * 10,
          width: 8,
          height: 8,
        },
        fill: `hsl(${(i * 137) % 360}, 70%, 50%)`,
      });

      // Create constraint chains
      if (i > 0) {
        constraints.push({
          type: 'adjacent',
          a: { entityId: i - 1, edge: 'right' },
          b: { entityId: i, edge: 'left' },
        });
      }
    }

    // Render initial state
    renderer.render({ entities, constraints });

    // Animate for duration
    const startTime = performance.now();

    const animate = () => {
      const t = (performance.now() - startTime) / 1000;

      // Update T-vector to animate positions
      const solveStart = performance.now();
      renderer.updateTVector(t);
      metrics.constraintSolveTimes.push(performance.now() - solveStart);

      if (performance.now() - startTime < cfg.durationMs) {
        requestAnimationFrame(animate);
      }
    };

    requestAnimationFrame(animate);

    await new Promise(resolve => setTimeout(resolve, cfg.durationMs + 100));
  }, config);

  return await page.evaluate(() => (window as any).__VS_PERF_METRICS__);
}

async function runCombinedStressTest(
  page: Page,
  config: { eventsPerFrame: number; nodeCount: number; durationMs: number },
): Promise<PerformanceMetrics> {
  await page.evaluate(async (cfg) => {
    const renderer = (window as any).__VS_RENDERER__;
    const metrics = (window as any).__VS_PERF_METRICS__;

    // Generate constraint graph
    const entities = [];
    for (let i = 0; i < cfg.nodeCount; i++) {
      entities.push({
        id: i,
        type: 'rect',
        bounds: {
          x: (i % 100) * 8,
          y: Math.floor(i / 100) * 8,
          width: 6,
          height: 6,
        },
        fill: `hsl(${(i * 137) % 360}, 70%, 50%)`,
      });
    }
    renderer.render({ entities, constraints: [] });

    const startTime = performance.now();

    const runFrame = () => {
      // Event storm
      for (let i = 0; i < cfg.eventsPerFrame; i++) {
        const event = new MouseEvent('mousemove', {
          clientX: Math.random() * 800,
          clientY: Math.random() * 600,
        });
        document.dispatchEvent(event);
        metrics.totalEventsReceived++;
      }

      // Constraint solving
      const t = (performance.now() - startTime) / 1000;
      const solveStart = performance.now();
      renderer.updateTVector(t);
      metrics.constraintSolveTimes.push(performance.now() - solveStart);

      if (performance.now() - startTime < cfg.durationMs) {
        requestAnimationFrame(runFrame);
      }
    };

    requestAnimationFrame(runFrame);

    await new Promise(resolve => setTimeout(resolve, cfg.durationMs + 100));

    // Safely get processed event count (may not be implemented)
    const processedCount = typeof renderer.getProcessedEventCount === 'function'
      ? renderer.getProcessedEventCount()
      : metrics.totalEventsReceived; // Fallback: assume all processed (no coalescing)
    metrics.totalEventsProcessed = processedCount;
  }, config);

  return await page.evaluate(() => (window as any).__VS_PERF_METRICS__);
}

// =============================================================================
// Analysis
// =============================================================================

function analyzeMetrics(metrics: PerformanceMetrics): PerformanceReport {
  const failures: string[] = [];

  // Sort for percentile calculation
  const sortedFrameTimes = [...metrics.frameTimes].sort((a, b) => a - b);
  const sortedConstraintTimes = [...metrics.constraintSolveTimes].sort((a, b) => a - b);

  const percentile = (arr: number[], p: number): number => {
    if (arr.length === 0) return 0;
    const idx = Math.ceil(arr.length * p) - 1;
    return arr[Math.max(0, idx)];
  };

  const frameTimeP50 = percentile(sortedFrameTimes, 0.50);
  const frameTimeP95 = percentile(sortedFrameTimes, 0.95);
  const frameTimeP99 = percentile(sortedFrameTimes, 0.99);
  const frameTimeMax = sortedFrameTimes[sortedFrameTimes.length - 1] ?? 0;
  const constraintSolveP95 = percentile(sortedConstraintTimes, 0.95);

  const coalesceRate = metrics.totalEventsReceived > 0
    ? 1 - (metrics.totalEventsProcessed / metrics.totalEventsReceived)
    : 1;

  // Check thresholds
  if (frameTimeP99 >= PERF_THRESHOLDS.FRAME_TIME_P99_MS) {
    failures.push(`Frame time p99 (${frameTimeP99.toFixed(2)}ms) >= ${PERF_THRESHOLDS.FRAME_TIME_P99_MS}ms`);
  }
  if (metrics.forcedSyncLayoutCount > PERF_THRESHOLDS.MAX_FORCED_SYNC_LAYOUT) {
    failures.push(`Forced sync layouts (${metrics.forcedSyncLayoutCount}) > 0`);
  }
  if (coalesceRate < PERF_THRESHOLDS.MIN_COALESCE_RATE) {
    failures.push(`Coalesce rate (${(coalesceRate * 100).toFixed(1)}%) < 90%`);
  }
  if (constraintSolveP95 >= PERF_THRESHOLDS.CONSTRAINT_SOLVE_P95_MS) {
    failures.push(`Constraint solve p95 (${constraintSolveP95.toFixed(2)}ms) >= ${PERF_THRESHOLDS.CONSTRAINT_SOLVE_P95_MS}ms`);
  }

  return {
    frameTimeP50,
    frameTimeP95,
    frameTimeP99,
    frameTimeMax,
    forcedSyncLayoutCount: metrics.forcedSyncLayoutCount,
    coalesceRate,
    constraintSolveP95,
    passed: failures.length === 0,
    failures,
  };
}
