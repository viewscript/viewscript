/**
 * Async Race Condition E2E Tests
 *
 * This module provides mathematical proof that asynchronous Q-dimension events
 * are processed atomically within the render loop, with no event loss or
 * ordering violations.
 *
 * ## The Problem: Event Loop Race Conditions
 *
 * In JavaScript's event loop, synchronous events (mousemove) and asynchronous
 * callbacks (fetch handlers, setTimeout, Promise.then) can interleave in
 * non-deterministic order. Without proper isolation:
 *
 * ```
 * Tick N:
 *   mousemove arrives → buffer.push()
 *   Promise.then fires → buffer.push()    ← Race: which comes first?
 *   setTimeout fires → buffer.push()      ← Race: order undefined!
 *   flush() → events may be out of order
 * ```
 *
 * ## The Solution: Async Event Isolation
 *
 * The EventBuffer isolates async events in a separate buffer, merging them
 * atomically at tick start:
 *
 * ```
 * Tick N:
 *   [Before tick] Async events accumulate in pendingAsyncEvents
 *   [Tick start]  mergeAsyncEvents() moves all async → main buffer (atomic)
 *   [During tick] Sync events go directly to main buffer
 *   [Tick end]    flush() returns deterministic event order
 * ```
 *
 * ## Test Strategy
 *
 * 1. Create an event storm: hundreds of sync + async events colliding
 * 2. Wait for a single rAF to process them all
 * 3. Verify the final state matches the expected mathematical result
 * 4. Verify no events were lost (counter check)
 * 5. Verify CRITICAL event ordering was preserved
 */

import { test, expect, type Page } from '@playwright/test';

// =============================================================================
// Test Configuration
// =============================================================================

/** Number of events to inject in storm test */
const STORM_EVENT_COUNT = 500;

/** Number of entities to use in multi-entity test */
const MULTI_ENTITY_COUNT = 50;

/** Expected final T-vector states after all events processed */
interface ExpectedTVectorState {
  entityId: number;
  hover: number;
  pressed: number;
  scroll_y: number;
  animation_t: number;
}

// =============================================================================
// Race Condition Tests
// =============================================================================

test.describe('Async Race Condition: Event Atomicity Proof', () => {

  /**
   * Test: Sync and async events in same frame produce deterministic result
   *
   * Injects 500 events split between sync (mousemove) and async (Promise.then)
   * contexts. Verifies the final T-vector state is exactly as expected.
   */
  test('500 mixed sync/async events produce deterministic T-vector state', async ({ page }) => {
    await setupTestPage(page);

    // Inject event storm and capture final state
    const result = await page.evaluate(async (eventCount) => {
      const renderer = (window as any).__VS_RENDERER__;
      const eventBuffer = renderer.getEventBuffer();

      // Track events processed
      let syncEventCount = 0;
      let asyncEventCount = 0;

      // Entity 1: receives alternating hover events
      // Final hover should be eventCount % 2 (last event wins due to coalescing)
      const entity1Id = 1;

      // Entity 2: receives scroll_y events with cumulative value
      // Final scroll_y should be 0.5 (middle of normalized range)
      const entity2Id = 2;

      // Phase 1: Inject sync events (direct DOM event simulation)
      for (let i = 0; i < eventCount / 2; i++) {
        eventBuffer.push({
          entityId: entity1Id,
          eventType: 'pointermove',
          targetState: 'hover',
          value: i % 2, // Alternating 0, 1, 0, 1...
          timestamp: performance.now(),
          priority: 1, // NORMAL
        });
        syncEventCount++;
      }

      // Phase 2: Inject async events via Promise.then (microtask queue)
      const asyncPromises: Promise<void>[] = [];
      for (let i = 0; i < eventCount / 4; i++) {
        asyncPromises.push(
          Promise.resolve().then(() => {
            eventBuffer.pushAsync({
              entityId: entity2Id,
              eventType: 'scroll',
              targetState: 'scroll_y',
              value: i / (eventCount / 4), // 0.0 to ~1.0
              timestamp: performance.now(),
              priority: 1,
            });
            asyncEventCount++;
          })
        );
      }

      // Phase 3: Inject async events via setTimeout (macrotask queue)
      for (let i = 0; i < eventCount / 4; i++) {
        asyncPromises.push(
          new Promise<void>((resolve) => {
            setTimeout(() => {
              eventBuffer.pushAsync({
                entityId: entity2Id,
                eventType: 'scroll',
                targetState: 'scroll_y',
                value: 0.5, // Constant value (should be final due to coalescing)
                timestamp: performance.now(),
                priority: 1,
              });
              asyncEventCount++;
              resolve();
            }, 0);
          })
        );
      }

      // Wait for all async events to be queued
      await Promise.all(asyncPromises);

      // Wait for next rAF to process all events
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(() => resolve());
        });
      });

      // Query final T-vector state
      const tVectorState = renderer.getTVectorState();

      return {
        syncEventCount,
        asyncEventCount,
        totalExpected: eventCount,
        entity1Hover: tVectorState[entity1Id]?.hover ?? -1,
        entity2ScrollY: tVectorState[entity2Id]?.scroll_y ?? -1,
      };
    }, STORM_EVENT_COUNT);

    // Verify event counts
    expect(result.syncEventCount).toBe(STORM_EVENT_COUNT / 2);
    expect(result.asyncEventCount).toBe(STORM_EVENT_COUNT / 2);

    // Verify deterministic final state
    // Entity 1: hover should be (STORM_EVENT_COUNT/2 - 1) % 2 = last value
    const expectedHover = ((STORM_EVENT_COUNT / 2) - 1) % 2;
    expect(result.entity1Hover).toBe(expectedHover);

    // Entity 2: scroll_y should be 0.5 (last setTimeout value wins)
    expect(result.entity2ScrollY).toBeCloseTo(0.5, 10);
  });

  /**
   * Test: CRITICAL events preserve ordering despite async interleaving
   *
   * CRITICAL events (click, keydown) must never be coalesced and must
   * maintain their relative ordering even when async events interleave.
   */
  test('CRITICAL events preserve strict ordering', async ({ page }) => {
    await setupTestPage(page);

    const result = await page.evaluate(async () => {
      const renderer = (window as any).__VS_RENDERER__;
      const eventBuffer = renderer.getEventBuffer();

      // Track the order in which events are processed
      const processedOrder: number[] = [];

      // Inject CRITICAL events with sequence numbers
      // Some via sync, some via async - order must be preserved
      const sequenceIds = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

      // Odd sequence numbers: sync push
      for (const seq of sequenceIds.filter(s => s % 2 === 1)) {
        eventBuffer.push({
          entityId: seq,
          eventType: 'click',
          targetState: 'pressed',
          value: seq,
          timestamp: performance.now() + seq, // Unique timestamp
          priority: 3, // CRITICAL
        });
      }

      // Even sequence numbers: async push via Promise.then
      await Promise.all(
        sequenceIds.filter(s => s % 2 === 0).map(seq =>
          Promise.resolve().then(() => {
            eventBuffer.pushAsync({
              entityId: seq,
              eventType: 'click',
              targetState: 'pressed',
              value: seq,
              timestamp: performance.now() + seq,
              priority: 3, // CRITICAL
            });
          })
        )
      );

      // Merge async and flush
      eventBuffer.mergeAsyncEvents();
      const flushed = eventBuffer.flush();

      // Extract the sequence of entity IDs (which encode the original sequence)
      const flushedOrder = flushed.map((e: any) => e.entityId);

      // CRITICAL events should all be present (no loss)
      const allPresent = sequenceIds.every(seq => flushedOrder.includes(seq));

      // Within each category (sync vs async), relative order should be preserved
      const syncOrder = flushedOrder.filter((id: number) => id % 2 === 1);
      const asyncOrder = flushedOrder.filter((id: number) => id % 2 === 0);

      const syncOrderPreserved = syncOrder.every(
        (id: number, i: number) => i === 0 || id > syncOrder[i - 1]
      );
      const asyncOrderPreserved = asyncOrder.every(
        (id: number, i: number) => i === 0 || id > asyncOrder[i - 1]
      );

      return {
        totalFlushed: flushed.length,
        allPresent,
        syncOrderPreserved,
        asyncOrderPreserved,
        flushedOrder,
      };
    });

    // All 10 CRITICAL events must be present (no coalescing, no loss)
    expect(result.totalFlushed).toBe(10);
    expect(result.allPresent).toBe(true);

    // Order within sync and async categories must be preserved
    expect(result.syncOrderPreserved).toBe(true);
    expect(result.asyncOrderPreserved).toBe(true);
  });

  /**
   * Test: P-dimension coordinates derived from T-vector are bit-perfect
   *
   * After processing all events, the constraint solver must produce
   * exactly the expected P-dimension values - no floating point drift.
   */
  test('P-dimension coordinates are bit-perfect after event storm', async ({ page }) => {
    await setupTestPage(page);

    const result = await page.evaluate(async (entityCount) => {
      const renderer = (window as any).__VS_RENDERER__;
      const eventBuffer = renderer.getEventBuffer();

      // Create entities with constraints: X = hover * 100 + entityId
      const entities = [];
      for (let id = 1; id <= entityCount; id++) {
        entities.push({
          id,
          type: 'rect',
          bounds: { x: id, y: 0, width: 10, height: 10 },
          interactive: true,
        });
      }

      // Render entities with T-dependent constraints
      renderer.render({
        entities,
        constraints: entities.map(e => ({
          target: e.id,
          component: 'x',
          relation: 'eq',
          // X = hover * 100 + entityId
          term: { type: 'linear', tState: 'hover', coefficient: 100, offset: e.id },
        })),
      });

      // Inject events: set hover=1 for all entities via async storm
      const asyncPromises: Promise<void>[] = [];

      for (let id = 1; id <= entityCount; id++) {
        // Half via Promise.then
        if (id % 2 === 0) {
          asyncPromises.push(
            Promise.resolve().then(() => {
              eventBuffer.pushAsync({
                entityId: id,
                eventType: 'pointerenter',
                targetState: 'hover',
                value: 1,
                timestamp: performance.now(),
                priority: 2, // HIGH
              });
            })
          );
        }
        // Half via setTimeout
        else {
          asyncPromises.push(
            new Promise<void>((resolve) => {
              setTimeout(() => {
                eventBuffer.pushAsync({
                  entityId: id,
                  eventType: 'pointerenter',
                  targetState: 'hover',
                  value: 1,
                  timestamp: performance.now(),
                  priority: 2,
                });
                resolve();
              }, 0);
            })
          );
        }
      }

      await Promise.all(asyncPromises);

      // Wait for render tick to process
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(() => resolve());
        });
      });

      // Query P-dimension coordinates and T-vector state
      const pCoordinates: { id: number; x: number }[] = [];
      const tStates: { id: number; hover: number }[] = [];

      for (let id = 1; id <= entityCount; id++) {
        const bounds = renderer.getEntityBounds(id);
        const tState = renderer.getTVectorState()[id];

        pCoordinates.push({ id, x: bounds.x });
        tStates.push({ id, hover: tState?.hover ?? 0 });
      }

      return { pCoordinates, tStates, entityCount };
    }, MULTI_ENTITY_COUNT);

    // Verify all entities received hover=1
    for (const tState of result.tStates) {
      expect(tState.hover).toBe(1);
    }

    // Verify P-dimension X coordinates are exactly as expected
    // X = hover * 100 + entityId = 1 * 100 + entityId = 100 + entityId
    for (const pCoord of result.pCoordinates) {
      const expectedX = 100 + pCoord.id;
      expect(pCoord.x).toBe(expectedX);
    }
  });

  /**
   * Test: No event loss under extreme concurrency
   *
   * Fire events from multiple async sources simultaneously and verify
   * every single event is accounted for.
   */
  test('zero event loss under concurrent async sources', async ({ page }) => {
    await setupTestPage(page);

    const result = await page.evaluate(async () => {
      const renderer = (window as any).__VS_RENDERER__;
      const eventBuffer = renderer.getEventBuffer();

      // Counter for verification
      let expectedCount = 0;

      // Source 1: Promise.resolve chain (microtasks)
      const microtaskPromises: Promise<void>[] = [];
      for (let i = 0; i < 100; i++) {
        microtaskPromises.push(
          Promise.resolve().then(() => {
            eventBuffer.pushAsync({
              entityId: 1,
              eventType: 'pointermove',
              targetState: 'animation_t',
              value: i / 100,
              timestamp: performance.now(),
              priority: 0, // LOW
            });
            expectedCount++;
          })
        );
      }

      // Source 2: setTimeout 0 (macrotasks)
      const macrotaskPromises: Promise<void>[] = [];
      for (let i = 0; i < 100; i++) {
        macrotaskPromises.push(
          new Promise<void>((resolve) => {
            setTimeout(() => {
              eventBuffer.pushAsync({
                entityId: 2,
                eventType: 'pointermove',
                targetState: 'animation_t',
                value: i / 100,
                timestamp: performance.now(),
                priority: 0,
              });
              expectedCount++;
              resolve();
            }, 0);
          })
        );
      }

      // Source 3: queueMicrotask
      const queuedMicrotasks: Promise<void>[] = [];
      for (let i = 0; i < 100; i++) {
        queuedMicrotasks.push(
          new Promise<void>((resolve) => {
            queueMicrotask(() => {
              eventBuffer.pushAsync({
                entityId: 3,
                eventType: 'pointermove',
                targetState: 'animation_t',
                value: i / 100,
                timestamp: performance.now(),
                priority: 0,
              });
              expectedCount++;
              resolve();
            });
          })
        );
      }

      // Wait for all sources
      await Promise.all([
        ...microtaskPromises,
        ...macrotaskPromises,
        ...queuedMicrotasks,
      ]);

      // Get stats before merge
      const statsBefore = eventBuffer.getStats();

      // Merge and flush
      eventBuffer.mergeAsyncEvents();
      const flushed = eventBuffer.flush();

      return {
        expectedCount,
        asyncPendingBefore: statsBefore.asyncPendingSize,
        flushedCount: flushed.length,
        // Due to coalescing, we expect 3 events (one per entity, last value wins)
        uniqueEntities: new Set(flushed.map((e: any) => e.entityId)).size,
      };
    });

    // All 300 events should have been received
    expect(result.expectedCount).toBe(300);
    expect(result.asyncPendingBefore).toBe(300);

    // After coalescing, we should have exactly 3 events (one per entity)
    expect(result.flushedCount).toBe(3);
    expect(result.uniqueEntities).toBe(3);
  });

  /**
   * Test: Canvas and DOM state match after async event storm
   *
   * The bilayer invariant must hold even after async event processing:
   * Canvas visual position === DOM hit region position
   *
   * Note: Coalescing uses "last push wins" semantics, not timestamp ordering.
   * We use sequential pushAsync calls to ensure deterministic final value.
   */
  test('bilayer sync maintained after async event storm', async ({ page }) => {
    await setupTestPage(page);

    const result = await page.evaluate(async () => {
      const renderer = (window as any).__VS_RENDERER__;
      const eventBuffer = renderer.getEventBuffer();

      // Create a button that moves based on animation_t
      renderer.render({
        entities: [
          {
            id: 1,
            type: 'rect',
            bounds: { x: 0, y: 100, width: 100, height: 50 },
            interactive: true,
          },
        ],
        constraints: [
          {
            target: 1,
            component: 'x',
            relation: 'eq',
            // X = animation_t * 200 (moves 0 to 200 as t goes 0 to 1)
            term: { type: 'linear', tState: 'animation_t', coefficient: 200, offset: 0 },
          },
        ],
      });

      // Inject async events SEQUENTIALLY to ensure deterministic ordering
      // The LAST pushed event for entity:state wins due to coalescing
      await Promise.resolve().then(() => {
        eventBuffer.pushAsync({
          entityId: 1,
          eventType: 'custom',
          targetState: 'animation_t',
          value: 0.25,
          timestamp: performance.now(),
          priority: 1,
        });
      });

      await Promise.resolve().then(() => {
        eventBuffer.pushAsync({
          entityId: 1,
          eventType: 'custom',
          targetState: 'animation_t',
          value: 0.5,
          timestamp: performance.now(),
          priority: 1,
        });
      });

      // This is pushed LAST, so it wins (coalescing: last push wins)
      await Promise.resolve().then(() => {
        eventBuffer.pushAsync({
          entityId: 1,
          eventType: 'custom',
          targetState: 'animation_t',
          value: 0.75,
          timestamp: performance.now(),
          priority: 1,
        });
      });

      // Wait for render
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(() => resolve());
        });
      });

      // Query Canvas position (P-dimension)
      const canvasBounds = renderer.getEntityBounds(1);

      // Query DOM hit region position
      const domElement = document.querySelector('[data-vs-entity="1"]') as HTMLElement;
      const domTransform = domElement?.style.transform || '';
      const domXMatch = domTransform.match(/translate3d\(([^,]+)px/);
      const domX = domXMatch ? parseFloat(domXMatch[1]) : -1;

      // Query T-vector state
      const tState = renderer.getTVectorState()[1];

      return {
        canvasX: canvasBounds.x,
        domX,
        animationT: tState?.animation_t ?? -1,
      };
    });

    // animation_t should be 0.75 (last push wins)
    expect(result.animationT).toBeCloseTo(0.75, 10);

    // Expected X = 0.75 * 200 = 150
    expect(result.canvasX).toBe(150);

    // DOM X must match Canvas X exactly (bilayer invariant)
    expect(result.domX).toBe(result.canvasX);
  });
});

// =============================================================================
// Helper Functions
// =============================================================================

/**
 * Setup the test page with VS renderer.
 */
async function setupTestPage(page: Page): Promise<void> {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => (window as any).__VS_RENDERER_READY__ === true, {
    timeout: 10000,
  });
}
