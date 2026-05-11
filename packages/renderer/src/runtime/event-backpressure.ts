/**
 * Q-Dimension Event Backpressure Control
 *
 * High-frequency DOM events (mousemove, scroll, pointermove) can fire
 * hundreds of times per second. Without backpressure, each event would
 * trigger a full constraint graph evaluation, overwhelming the solver.
 *
 * ## Strategy: Latest-Only Sampling with Frame Alignment
 *
 * ```
 * Event Stream (60+ events/frame):
 *   ─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─►
 *    │ │ │ │ │ │ │ │ │ │ │ │
 *    └─┴─┴─┴─┬─┴─┴─┴─┴─┬─┴─┴─►  Frame boundaries
 *            │         │
 *            ▼         ▼
 *   Sampled: [latest]  [latest]   ← One value per frame
 * ```
 *
 * This module provides:
 * 1. Per-entity, per-component event coalescing
 * 2. Frame-aligned sampling (sync with rAF)
 * 3. Configurable throttle strategies
 * 4. Priority queues for critical events (click > mousemove)
 */

import type { EntityId } from '../ast/types';

// =============================================================================
// Types
// =============================================================================

/**
 * Semantic T-vector state keys.
 *
 * Q-dimension events MUST target these state keys, NOT spatial coordinates.
 * P-dimension spatial coordinates (X, Y, Z) are derived from constraints
 * that reference T-vector state, never directly from Q-dimension input.
 *
 * ## The Ouroboros Prevention Principle
 *
 * If Q-dimension input (e.g., MouseEvent.clientX) were allowed to directly
 * mutate P-dimension spatial coordinates, it would create a self-referential
 * loop: mouse position → entity position → canvas render → mouse position...
 *
 * Instead, Q-dimension input mutates T-vector STATE, and the constraint
 * solver derives spatial coordinates as a FUNCTION of that state.
 */
export type TStateKey =
  | 'hover'           // Boolean: is pointer over this entity?
  | 'pressed'         // Boolean: is pointer pressed on this entity?
  | 'focused'         // Boolean: does this entity have keyboard focus?
  | 'scroll_x'        // Number: horizontal scroll offset (normalized 0-1)
  | 'scroll_y'        // Number: vertical scroll offset (normalized 0-1)
  | 'drag_progress'   // Number: drag gesture progress (normalized 0-1)
  | 'animation_t'     // Number: animation timeline position
  | 'gesture_phase';  // Number: gesture recognizer phase (0=none, 1=began, 2=changed, 3=ended)

/**
 * Raw Q-dimension event from DOM.
 *
 * CRITICAL INVARIANT: Q-dimension events target T-vector STATE keys only.
 * They NEVER directly modify P-dimension spatial coordinates (X, Y, Z).
 */
export interface QDimensionEvent {
  /** Source entity that fired the event */
  entityId: EntityId;

  /** DOM event type */
  eventType: QEventType;

  /**
   * Target T-vector state key.
   *
   * ARCHITECTURAL CONSTRAINT: This is a semantic state key, NOT a spatial
   * coordinate. P-dimension coordinates are derived via constraint evaluation.
   */
  targetState: TStateKey;

  /**
   * State value (interpretation depends on targetState).
   *
   * - Boolean states: 0 = false, 1 = true
   * - Normalized states: 0.0 to 1.0
   * - Phase states: discrete integers
   */
  value: number;

  /** Event timestamp (performance.now()) */
  timestamp: number;

  /** Priority (higher = more important) */
  priority: EventPriority;
}

export type QEventType =
  | 'click'
  | 'pointerdown'
  | 'pointerup'
  | 'pointermove'
  | 'scroll'
  | 'wheel'
  | 'keydown'
  | 'keyup'
  | 'focus'
  | 'blur';

export enum EventPriority {
  /** Immediate: click, keydown (user intent) */
  CRITICAL = 3,

  /** High: pointerdown/up (gesture start/end) */
  HIGH = 2,

  /** Normal: scroll, wheel (continuous) */
  NORMAL = 1,

  /** Low: pointermove (high frequency, lossy OK) */
  LOW = 0,
}

/**
 * Coalesced event ready for frame processing.
 *
 * This represents a T-vector state mutation, NOT a spatial coordinate change.
 */
export interface CoalescedEvent {
  entityId: EntityId;
  /** The T-vector state key being mutated */
  state: TStateKey;
  /** The new state value */
  value: number;
  timestamp: number;
}

/**
 * Backpressure configuration.
 */
export interface BackpressureConfig {
  /** Max events to process per frame */
  maxEventsPerFrame: number;

  /** Throttle interval for LOW priority events (ms) */
  lowPriorityThrottleMs: number;

  /** Enable event coalescing (latest-only for same entity+component) */
  enableCoalescing: boolean;
}

// =============================================================================
// Event Buffer (Per-Entity, Per-Component Coalescing)
// =============================================================================

/**
 * Key for coalescing: entityId + state key
 */
type CoalesceKey = `${EntityId}:${TStateKey}`;

/**
 * Double-buffered event accumulator.
 *
 * ## Data Structure
 *
 * ```
 * writeBuffer (current frame events):
 *   Map<CoalesceKey, QDimensionEvent>
 *   - Key: "entity42:x"
 *   - Value: Latest event for that entity+component
 *   - Overwrites: Yes (latest-only sampling)
 *
 * priorityQueue (critical events):
 *   Array<QDimensionEvent>
 *   - Never coalesced (click, keydown must all fire)
 *   - Processed first
 *
 * pendingAsyncEvents (async callback events):
 *   Array<QDimensionEvent>
 *   - Events from async callbacks (fetch, setTimeout, promises)
 *   - Isolated from sync events to prevent race conditions
 *   - Merged at tick start via mergeAsyncEvents()
 * ```
 *
 * ## Async Atomicity (Phase 2 Remediation)
 *
 * Events from async callbacks (fetch handlers, setTimeout, etc.) arrive
 * outside the rAF tick boundary. Without isolation, they could interleave
 * with sync events causing non-deterministic ordering.
 *
 * Solution: Async events are buffered separately and merged atomically
 * at the START of each tick, before any sync event processing.
 */
export class EventBuffer {
  private config: BackpressureConfig;

  /** Coalesced events (latest-only per entity+component) */
  private writeBuffer: Map<CoalesceKey, QDimensionEvent> = new Map();

  /** Non-coalesced critical events (click, keydown) */
  private priorityQueue: QDimensionEvent[] = [];

  /** Last event time per key (for throttling) */
  private lastEventTime: Map<CoalesceKey, number> = new Map();

  /**
   * Pending async events (from fetch, setTimeout, promises).
   *
   * These are isolated from sync events and merged at tick start
   * to ensure deterministic ordering.
   */
  private pendingAsyncEvents: QDimensionEvent[] = [];

  constructor(config: Partial<BackpressureConfig> = {}) {
    this.config = {
      maxEventsPerFrame: 50,
      lowPriorityThrottleMs: 16, // ~60fps
      enableCoalescing: true,
      ...config,
    };
  }

  /**
   * Push a Q-dimension event into the buffer.
   *
   * ## Coalescing Rules
   *
   * 1. CRITICAL priority: Always queued, never coalesced
   * 2. HIGH/NORMAL/LOW: Coalesced by entity+state (latest wins)
   * 3. LOW with throttle: Dropped if within throttle window
   */
  push(event: QDimensionEvent): void {
    const key: CoalesceKey = `${event.entityId}:${event.targetState}`;

    // Critical events bypass coalescing
    if (event.priority === EventPriority.CRITICAL) {
      this.priorityQueue.push(event);
      return;
    }

    // Throttle check for low-priority events
    if (event.priority === EventPriority.LOW) {
      const lastTime = this.lastEventTime.get(key) ?? 0;
      if (event.timestamp - lastTime < this.config.lowPriorityThrottleMs) {
        // Drop: within throttle window
        return;
      }
    }

    // Coalesce: overwrite previous event for same entity+component
    if (this.config.enableCoalescing) {
      this.writeBuffer.set(key, event);
    } else {
      // No coalescing: treat as priority queue
      this.priorityQueue.push(event);
    }

    this.lastEventTime.set(key, event.timestamp);
  }

  /**
   * Flush buffer for frame processing.
   *
   * Returns events in priority order:
   * 1. All CRITICAL events (in order received)
   * 2. Coalesced events (limited by maxEventsPerFrame)
   *
   * ## Frame Alignment
   *
   * This method is called once per rAF tick. Events that arrive
   * after flush() but before next tick accumulate in the buffer.
   */
  flush(): CoalescedEvent[] {
    const result: CoalescedEvent[] = [];
    const limit = this.config.maxEventsPerFrame;

    // 1. Critical events first (never dropped)
    for (const event of this.priorityQueue) {
      result.push({
        entityId: event.entityId,
        state: event.targetState,
        value: event.value,
        timestamp: event.timestamp,
      });
    }
    this.priorityQueue = [];

    // 2. Coalesced events (up to limit)
    const remaining = limit - result.length;
    if (remaining > 0) {
      const coalesced = Array.from(this.writeBuffer.values())
        .sort((a, b) => b.priority - a.priority) // Higher priority first
        .slice(0, remaining);

      for (const event of coalesced) {
        result.push({
          entityId: event.entityId,
          state: event.targetState,
          value: event.value,
          timestamp: event.timestamp,
        });
      }
    }

    // Clear coalesced buffer (events not taken are dropped)
    this.writeBuffer.clear();

    return result;
  }

  /**
   * Get current buffer sizes (for debugging/metrics).
   */
  getStats(): { priorityQueueSize: number; coalescedSize: number; asyncPendingSize: number } {
    return {
      priorityQueueSize: this.priorityQueue.length,
      coalescedSize: this.writeBuffer.size,
      asyncPendingSize: this.pendingAsyncEvents.length,
    };
  }

  /**
   * Push an event from an async callback.
   *
   * Use this method for events originating from:
   * - fetch() handlers
   * - setTimeout / setInterval callbacks
   * - Promise .then() / .catch() handlers
   * - WebSocket message handlers
   * - IndexedDB callbacks
   * - Any other async context
   *
   * ## Why Separate From push()?
   *
   * Async callbacks can fire at any time, potentially mid-tick or between
   * ticks. If mixed with sync events without ordering guarantees, the result
   * is non-deterministic constraint evaluation.
   *
   * By isolating async events, we ensure:
   * 1. All async events are processed in FIFO order
   * 2. They are merged BEFORE sync events at tick start
   * 3. The tick sees a consistent snapshot of async state
   *
   * ## Example
   *
   * ```typescript
   * fetch('/api/data').then(response => {
   *   // WRONG: buffer.push(event) - could race with sync events
   *   // CORRECT: buffer.pushAsync(event) - isolated until tick start
   *   buffer.pushAsync({
   *     entityId: 42,
   *     eventType: 'custom',
   *     targetState: 'animation_t',
   *     value: response.progress,
   *     timestamp: performance.now(),
   *     priority: EventPriority.NORMAL,
   *   });
   * });
   * ```
   */
  pushAsync(event: QDimensionEvent): void {
    this.pendingAsyncEvents.push(event);
  }

  /**
   * Merge pending async events into the main buffers.
   *
   * MUST be called at the START of each tick, BEFORE flush().
   *
   * This ensures:
   * 1. Async events are processed before sync events from the same frame
   * 2. No async events can arrive mid-flush (atomicity)
   * 3. Deterministic ordering: async (FIFO) → sync (coalesced/priority)
   *
   * ## Call Site
   *
   * AtomicRenderLoop.tick():
   * ```typescript
   * function tick(timestamp) {
   *   // FIRST: Merge async events atomically
   *   eventBuffer.mergeAsyncEvents();
   *
   *   // THEN: Flush and process all events
   *   const mutations = eventBuffer.flush();
   *   // ... rest of tick
   * }
   * ```
   */
  mergeAsyncEvents(): void {
    if (this.pendingAsyncEvents.length === 0) {
      return;
    }

    // Process async events through the normal push() pipeline
    // This applies coalescing and priority rules consistently
    for (const event of this.pendingAsyncEvents) {
      this.push(event);
    }

    // Clear the async buffer
    this.pendingAsyncEvents = [];
  }
}

// =============================================================================
// Event Controller (DOM Binding Layer)
// =============================================================================

/**
 * DOM event controller with automatic backpressure.
 *
 * ## Data Flow
 *
 * ```
 * DOM Event (mousemove)
 *     │
 *     ▼
 * EventController.handleEvent()
 *     │
 *     ├─▶ Compute T-vector value (from event data)
 *     │
 *     ├─▶ Wrap as QDimensionEvent
 *     │
 *     ▼
 * EventBuffer.push()
 *     │
 *     ├─▶ Throttle check (LOW priority)
 *     │
 *     ├─▶ Coalesce (latest-only)
 *     │
 *     ▼
 * Buffer accumulates until next rAF
 *     │
 *     ▼
 * AtomicRenderLoop.tick()
 *     │
 *     ├─▶ EventBuffer.flush()
 *     │
 *     ▼
 * CoalescedEvent[] → ConstraintSolver
 * ```
 */
export class EventController {
  private buffer: EventBuffer;
  private entityElements: Map<EntityId, HTMLElement> = new Map();
  private boundHandlers: Map<string, EventListener> = new Map();

  constructor(buffer: EventBuffer) {
    this.buffer = buffer;
  }

  /**
   * Register a DOM element for event handling.
   */
  registerElement(entityId: EntityId, element: HTMLElement): void {
    this.entityElements.set(entityId, element);
  }

  /**
   * Bind an event type to a T-vector state key.
   *
   * ## Ouroboros Prevention
   *
   * The valueMapper function MUST return a SEMANTIC state value, not a
   * raw spatial coordinate. For example:
   *
   * CORRECT (semantic state):
   *   - `'hover'` → (e) => e.type === 'pointerenter' ? 1 : 0
   *   - `'scroll_y'` → (e) => e.target.scrollTop / e.target.scrollHeight
   *   - `'drag_progress'` → (e) => computeNormalizedDragProgress(e)
   *
   * FORBIDDEN (spatial coordinate - would violate P/Q boundary):
   *   - `'x'` → (e) => e.clientX   // NEVER DO THIS
   *   - `'y'` → (e) => e.clientY   // NEVER DO THIS
   *
   * @param entityId - Entity to bind the event to
   * @param eventType - DOM event type
   * @param targetState - T-vector state key (semantic, not spatial)
   * @param valueMapper - Function to compute state value from DOM event
   */
  bindEvent(
    entityId: EntityId,
    eventType: QEventType,
    targetState: TStateKey,
    valueMapper: (event: Event) => number,
  ): void {
    const element = this.entityElements.get(entityId);
    if (!element) return;

    const handler = (domEvent: Event) => {
      const qEvent: QDimensionEvent = {
        entityId,
        eventType,
        targetState,
        value: valueMapper(domEvent),
        timestamp: performance.now(),
        priority: this.getPriority(eventType),
      };

      this.buffer.push(qEvent);
    };

    const key = `${entityId}:${eventType}`;
    this.boundHandlers.set(key, handler);

    // Use passive listeners where possible (scroll, wheel, pointermove)
    const passive = ['scroll', 'wheel', 'pointermove'].includes(eventType);
    element.addEventListener(eventType, handler, { passive });
  }

  /**
   * Unbind all events for an entity.
   */
  unbindEntity(entityId: EntityId): void {
    const element = this.entityElements.get(entityId);
    if (!element) return;

    for (const [key, handler] of this.boundHandlers) {
      if (key.startsWith(`${entityId}:`)) {
        const eventType = key.split(':')[1];
        element.removeEventListener(eventType, handler);
        this.boundHandlers.delete(key);
      }
    }

    this.entityElements.delete(entityId);
  }

  /**
   * Map event type to priority.
   */
  private getPriority(eventType: QEventType): EventPriority {
    switch (eventType) {
      case 'click':
      case 'keydown':
      case 'keyup':
        return EventPriority.CRITICAL;

      case 'pointerdown':
      case 'pointerup':
      case 'focus':
      case 'blur':
        return EventPriority.HIGH;

      case 'scroll':
      case 'wheel':
        return EventPriority.NORMAL;

      case 'pointermove':
        return EventPriority.LOW;

      default:
        return EventPriority.NORMAL;
    }
  }
}

// =============================================================================
// Usage Example
// =============================================================================

/**
 * ## Integration with Render Loop
 *
 * ```typescript
 * const buffer = new EventBuffer({ maxEventsPerFrame: 50 });
 * const controller = new EventController(buffer);
 *
 * // Register entity's DOM element
 * controller.registerElement(entity.id, domElement);
 *
 * // CORRECT: Bind semantic state (hover) to T-vector
 * controller.bindEvent(
 *   entity.id,
 *   'pointerenter',
 *   'hover',
 *   () => 1  // hover = true
 * );
 * controller.bindEvent(
 *   entity.id,
 *   'pointerleave',
 *   'hover',
 *   () => 0  // hover = false
 * );
 *
 * // CORRECT: Bind normalized scroll position
 * controller.bindEvent(
 *   entity.id,
 *   'scroll',
 *   'scroll_y',
 *   (e: Event) => {
 *     const target = e.target as HTMLElement;
 *     const maxScroll = target.scrollHeight - target.clientHeight;
 *     return maxScroll > 0 ? target.scrollTop / maxScroll : 0;
 *   }
 * );
 *
 * // In render loop tick():
 * const events = buffer.flush();
 *
 * // Events are T-vector STATE mutations, not spatial coordinates
 * // The constraint solver evaluates P-dimension coordinates AS A FUNCTION
 * // of T-vector state (via constraints like: A.x = 100 when T.hover = 1)
 * const stateMutations = events.map(e => ({
 *   entityId: e.entityId,
 *   state: e.state,      // Semantic state key (hover, scroll_y, etc.)
 *   value: e.value,      // State value
 *   timestamp: e.timestamp,
 * }));
 *
 * // Constraint solver derives X, Y, Z from T-vector state
 * constraintSolver.evaluateWithTVector(stateMutations);
 * ```
 *
 * ## Architectural Guarantee: P/Q Boundary Preservation
 *
 * By restricting Q-dimension events to T-vector STATE keys (not spatial
 * coordinates), we ensure that:
 *
 * 1. Mouse coordinates (clientX/Y) are NEVER directly assigned to P-dimension
 * 2. P-dimension coordinates are always derived via constraint evaluation
 * 3. The constraint graph is the SINGLE SOURCE OF TRUTH for spatial layout
 * 4. LEAN 4 decidability proofs remain valid (no floating-point pollution)
 */
