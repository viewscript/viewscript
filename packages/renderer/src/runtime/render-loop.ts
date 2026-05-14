/**
 * Atomic Render Loop for ViewScript
 *
 * This module implements a frame-synchronous render pipeline that guarantees
 * Canvas and DOM layers are updated atomically within a single animation frame.
 *
 * ## Architectural Invariant: No Frame Tearing
 *
 * If a button's visual representation moves at frame N, its DOM hit region
 * MUST also move at frame N. Any desynchronization causes Q-dimension inputs
 * to "hit the void" - a catastrophic UX failure.
 *
 * ## Strategy: Double-Buffered Dirty Tracking
 *
 * ```
 * Frame N:
 *   ┌─────────────────────────────────────────────────────────────────┐
 *   │ 1. Flush pending T-vector mutations (from Q-dimension events)  │
 *   │ 2. Evaluate constraint graph (P-dimension solver)              │
 *   │ 3. Topology-preserving rounding (with error distribution)      │
 *   │ 4. Diff against previous frame's RasterBounds                  │
 *   │ 5. Batch Canvas draw commands (wgpu)                          │
 *   │ 6. Batch DOM style mutations (transform only, no reflow)       │
 *   │ 7. Commit: GpuRenderer.flush() + requestAnimationFrame boundary│
 *   └─────────────────────────────────────────────────────────────────┘
 * ```
 *
 * ## Reflow Prevention Strategy
 *
 * DOM mutations are restricted to compositor-only properties:
 * - transform: translate3d(x, y, 0) - GPU-accelerated, no reflow
 * - opacity - compositor-only
 * - will-change: transform - hint to browser
 *
 * We NEVER touch: width, height, top, left, margin, padding (reflow triggers)
 */

import type {
  EntityId,
  RenderableEntity,
  RasterBounds,
  PVectorBounds,
  ChunkId,
} from '../ast/types';
import type { FfiDispatcher, PendingFfiCall } from './ffi-dispatcher.js';
import type { WasmSolverBridge } from './wasm-solver-bridge.js';

// =============================================================================
// Types
// =============================================================================

/**
 * Semantic T-vector state keys.
 * Mirrors the type from event-backpressure.ts.
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
 * Pending T-vector state mutation from Q-dimension event.
 *
 * CRITICAL: These are SEMANTIC state mutations, not spatial coordinate
 * assignments. P-dimension coordinates (X, Y, Z) are derived by the
 * constraint solver as functions of T-vector state.
 */
interface TStateMutation {
  entityId: EntityId;
  /** Semantic state key (hover, scroll_y, etc.) - NOT spatial coordinates */
  state: TStateKey;
  /** State value (boolean as 0/1, normalized 0-1, or discrete phase) */
  value: number;
  timestamp: number;
}

/**
 * Frame state for double buffering.
 */
interface FrameState {
  /** Raster bounds from previous frame (for diffing) */
  previousBounds: Map<EntityId, RasterBounds>;

  /** Current frame's computed bounds */
  currentBounds: Map<EntityId, RasterBounds>;

  /** Entities that changed this frame */
  dirtyEntities: Set<EntityId>;

  /** Frame number for debugging */
  frameNumber: number;
}

/**
 * Canvas draw command (batched for GPU renderer).
 */
interface DrawCommand {
  entityId: EntityId;
  type: 'path' | 'text' | 'image' | 'group';
  bounds: RasterBounds;
  payload: unknown; // Type-specific draw data
}

/**
 * DOM mutation command (batched for atomic commit).
 */
interface DOMMutation {
  entityId: EntityId;
  element: HTMLElement;
  transform: string;
  opacity?: number;
}

/**
 * Render loop configuration.
 */
export interface RenderLoopConfig {
  /** Target frames per second (default: 60) */
  targetFPS: number;

  /** Enable debug timing logs */
  debugTiming: boolean;

  /** Maximum mutations per frame (backpressure) */
  maxMutationsPerFrame: number;
}

// =============================================================================
// Core Render Loop
// =============================================================================

export class AtomicRenderLoop {
  private config: RenderLoopConfig;
  private frameState: FrameState;
  private pendingMutations: TStateMutation[] = [];
  private isRunning = false;
  private rafHandle: number | null = null;

  // External dependencies (injected)
  private constraintSolver: ConstraintSolver;
  private topologyRounder: TopologyRounder;
  private canvasRenderer: CanvasRenderer;
  private domLayer: DOMLayer;

  /**
   * Event buffer for async event atomicity.
   *
   * Injected via setEventBuffer() to support mergeAsyncEvents() at tick start.
   */
  private eventBuffer: EventBufferInterface | null = null;

  /**
   * FFI dispatcher for Phase 8 function evaluation.
   *
   * Injected via setFfiDispatcher() to support JS function calls after frame commit.
   */
  private ffiDispatcher: FfiDispatcher | null = null;

  /**
   * Latest tick result from WASM solver (for Phase 8 FFI dispatch).
   */
  private latestTickResult: { pending_ffi_calls: PendingFfiCall[] } | null = null;

  /**
   * Concrete solver bridge for FFI result injection (Phase 0.5).
   *
   * This is the concrete type of constraintSolver, used only for
   * injectFfiResults(). The ConstraintSolver interface remains FFI-agnostic.
   */
  private solverBridge: WasmSolverBridge | null = null;

  constructor(
    config: Partial<RenderLoopConfig>,
    constraintSolver: ConstraintSolver,
    topologyRounder: TopologyRounder,
    canvasRenderer: CanvasRenderer,
    domLayer: DOMLayer,
  ) {
    this.config = {
      targetFPS: 60,
      debugTiming: false,
      maxMutationsPerFrame: 100,
      ...config,
    };

    this.frameState = {
      previousBounds: new Map(),
      currentBounds: new Map(),
      dirtyEntities: new Set(),
      frameNumber: 0,
    };

    this.constraintSolver = constraintSolver;
    this.topologyRounder = topologyRounder;
    this.canvasRenderer = canvasRenderer;
    this.domLayer = domLayer;
  }

  // ===========================================================================
  // Public API
  // ===========================================================================

  /**
   * Start the render loop.
   */
  start(): void {
    if (this.isRunning) return;
    this.isRunning = true;
    this.scheduleFrame();
  }

  /**
   * Stop the render loop.
   */
  stop(): void {
    this.isRunning = false;
    if (this.rafHandle !== null) {
      cancelAnimationFrame(this.rafHandle);
      this.rafHandle = null;
    }
  }

  /**
   * Queue a T-vector mutation from Q-dimension event.
   * Called from event handlers (backpressure-controlled).
   */
  queueMutation(mutation: TStateMutation): void {
    this.pendingMutations.push(mutation);
  }

  /**
   * Set the event buffer for async event atomicity.
   *
   * The event buffer is used to merge async events (from fetch, setTimeout, etc.)
   * at the start of each tick, ensuring deterministic event ordering.
   */
  setEventBuffer(buffer: EventBufferInterface): void {
    this.eventBuffer = buffer;
  }

  /**
   * Set the FFI dispatcher for Phase 8 function evaluation.
   *
   * The FFI dispatcher evaluates JS functions after frame commit and buffers
   * results for the next frame's QSnapshot merge.
   */
  setFfiDispatcher(dispatcher: FfiDispatcher): void {
    this.ffiDispatcher = dispatcher;
  }

  /**
   * Set the concrete solver bridge for FFI result injection.
   *
   * The solver bridge is the concrete implementation of ConstraintSolver
   * that supports injectFfiResults(). This must be the same instance
   * as the constraintSolver passed to the constructor.
   */
  setSolverBridge(bridge: WasmSolverBridge): void {
    this.solverBridge = bridge;
  }

  // ===========================================================================
  // Frame Execution (Atomic Commit)
  // ===========================================================================

  /**
   * Execute a single frame tick.
   *
   * CRITICAL: This function executes entirely within a single rAF callback,
   * ensuring Canvas and DOM updates are committed atomically before the
   * browser's compositor thread runs.
   */
  private tick(timestamp: DOMHighResTimeStamp): void {
    if (!this.isRunning) return;

    const frameStart = performance.now();
    this.frameState.frameNumber++;

    // -------------------------------------------------------------------------
    // Phase 0: Merge Async Events (MUST be first - async atomicity)
    // -------------------------------------------------------------------------
    // Events from async callbacks (fetch, setTimeout, promises) are isolated
    // in a separate buffer to prevent race conditions. They are merged here
    // at the START of the tick, before any sync event processing.
    //
    // This ensures:
    // 1. Deterministic ordering: async events processed before sync events
    // 2. Atomicity: no async events can arrive mid-tick
    // 3. Consistency: the tick sees a complete snapshot of async state
    if (this.eventBuffer) {
      this.eventBuffer.mergeAsyncEvents();
    }

    // -------------------------------------------------------------------------
    // Phase 0.5: Merge FFI Results from Previous Frame
    // -------------------------------------------------------------------------
    // FFI function results from Phase 8 of the previous frame are drained
    // here and injected into the solver bridge. The bridge will include these
    // values in the QSnapshot when evaluate() is called in Phase 2.
    //
    // This maintains the 1-frame latency required by Axiom 2 (Ouroboros Binding):
    // FFI results flow Q→T→P, never directly into P-dimension.
    if (this.ffiDispatcher && this.solverBridge) {
      const ffiResults = this.ffiDispatcher.drainResults();
      if (ffiResults.length > 0) {
        this.solverBridge.injectFfiResults(ffiResults);
      }
    }

    // -------------------------------------------------------------------------
    // Phase 1: Flush Pending Mutations (Backpressure-Limited)
    // -------------------------------------------------------------------------
    const mutations = this.flushMutations();

    // -------------------------------------------------------------------------
    // Phase 2: Evaluate Constraint Graph
    // -------------------------------------------------------------------------
    const solverResult = this.constraintSolver.evaluate(mutations);
    const pVectorBounds = solverResult.bounds;

    // Store pending FFI calls for Phase 8 (after commit)
    this.latestTickResult = { pending_ffi_calls: solverResult.pendingFfiCalls };

    // -------------------------------------------------------------------------
    // Phase 3: Topology-Preserving Rounding (with Error Distribution)
    // -------------------------------------------------------------------------
    const rasterResult = this.topologyRounder.round(pVectorBounds);

    // -------------------------------------------------------------------------
    // Phase 4: Compute Dirty Set (Diff Against Previous Frame)
    // -------------------------------------------------------------------------
    this.computeDirtySet(rasterResult.bounds);

    // -------------------------------------------------------------------------
    // Phase 5: Batch Canvas Draw Commands
    // -------------------------------------------------------------------------
    const drawCommands = this.buildDrawCommands();

    // -------------------------------------------------------------------------
    // Phase 6: Batch DOM Mutations (Transform Only)
    // -------------------------------------------------------------------------
    const domMutations = this.buildDOMMutations();

    // -------------------------------------------------------------------------
    // Phase 7: Atomic Commit
    // -------------------------------------------------------------------------
    this.commitFrame(drawCommands, domMutations);

    // -------------------------------------------------------------------------
    // Phase 8: FFI Dispatch (Post-Commit)
    // -------------------------------------------------------------------------
    // FFI functions are evaluated AFTER frame commit to keep the rendering
    // critical path (Phase 2-7) free of unpredictable latency. Results are
    // buffered for consumption in the next frame's Phase 0.5.
    //
    // This utilizes the idle time between commitFrame() and the next rAF.
    if (this.ffiDispatcher && this.latestTickResult) {
      this.ffiDispatcher.dispatch(this.latestTickResult.pending_ffi_calls);
      this.latestTickResult = null;
    }

    // -------------------------------------------------------------------------
    // Swap Buffers
    // -------------------------------------------------------------------------
    this.swapBuffers();

    // -------------------------------------------------------------------------
    // Schedule Next Frame
    // -------------------------------------------------------------------------
    const frameEnd = performance.now();
    if (this.config.debugTiming) {
      console.log(`Frame ${this.frameState.frameNumber}: ${(frameEnd - frameStart).toFixed(2)}ms`);
    }

    this.scheduleFrame();
  }

  private scheduleFrame(): void {
    this.rafHandle = requestAnimationFrame((ts) => this.tick(ts));
  }

  // ===========================================================================
  // Phase Implementations
  // ===========================================================================

  /**
   * Phase 1: Flush pending mutations with backpressure limit.
   */
  private flushMutations(): TStateMutation[] {
    const limit = this.config.maxMutationsPerFrame;
    const flushed = this.pendingMutations.splice(0, limit);
    return flushed;
  }

  /**
   * Phase 4: Compute which entities changed this frame.
   */
  private computeDirtySet(currentBounds: Map<EntityId, RasterBounds>): void {
    this.frameState.dirtyEntities.clear();
    this.frameState.currentBounds = currentBounds;

    for (const [entityId, bounds] of currentBounds) {
      const prev = this.frameState.previousBounds.get(entityId);

      if (!prev || !boundsEqual(prev, bounds)) {
        this.frameState.dirtyEntities.add(entityId);
      }
    }

    // Detect removed entities
    for (const entityId of this.frameState.previousBounds.keys()) {
      if (!currentBounds.has(entityId)) {
        this.frameState.dirtyEntities.add(entityId);
      }
    }
  }

  /**
   * Phase 5: Build Canvas draw commands for dirty entities only.
   */
  private buildDrawCommands(): DrawCommand[] {
    const commands: DrawCommand[] = [];

    for (const entityId of this.frameState.dirtyEntities) {
      const bounds = this.frameState.currentBounds.get(entityId);
      if (!bounds) continue;

      // Get entity's canvas node and build draw command
      const entity = this.canvasRenderer.getEntity(entityId);
      if (entity?.canvas) {
        commands.push({
          entityId,
          type: entity.canvas.kind,
          bounds,
          payload: entity.canvas,
        });
      }
    }

    return commands;
  }

  /**
   * Phase 6: Build DOM mutations (transform-only, no reflow).
   *
   * CRITICAL: We use transform: translate3d() which is compositor-only.
   * This means the GPU handles the positioning without triggering
   * the browser's layout engine.
   */
  private buildDOMMutations(): DOMMutation[] {
    const mutations: DOMMutation[] = [];

    for (const entityId of this.frameState.dirtyEntities) {
      const bounds = this.frameState.currentBounds.get(entityId);
      if (!bounds) continue;

      const element = this.domLayer.getElement(entityId);
      if (!element) continue;

      // Use translate3d for GPU acceleration
      // Note: Element must have position: absolute and will-change: transform
      const transform = `translate3d(${bounds.x}px, ${bounds.y}px, 0)`;

      mutations.push({
        entityId,
        element,
        transform,
      });
    }

    return mutations;
  }

  /**
   * Phase 7: Atomic commit of Canvas and DOM updates.
   *
   * This function guarantees that by the time rAF callback returns:
   * 1. All wgpu draw commands have been flushed to GPU
   * 2. All DOM transforms have been written
   * 3. Browser's compositor will see consistent state
   *
   * ## Strict DOM Proxy Enforcement
   *
   * DOM elements are initialized with `initializeDOMProxyElement()` which
   * installs a Proxy guard. Only `transform` can be modified here.
   */
  private commitFrame(drawCommands: DrawCommand[], domMutations: DOMMutation[]): void {
    // --- Canvas Commit ---
    // GPU renderer batches internally; we just need to issue commands and flush
    for (const cmd of drawCommands) {
      this.canvasRenderer.draw(cmd);
    }
    this.canvasRenderer.flush();

    // --- DOM Commit (Single Write Batch) ---
    // Using updateDOMProxyTransform which is the only allowed mutation
    for (const mutation of domMutations) {
      updateDOMProxyTransform(
        mutation.element,
        this.frameState.currentBounds.get(mutation.entityId)?.x ?? 0,
        this.frameState.currentBounds.get(mutation.entityId)?.y ?? 0,
      );
    }

    // No reads after writes in this function = no reflow triggered
  }

  /**
   * Swap frame buffers for next frame's diff.
   */
  private swapBuffers(): void {
    this.frameState.previousBounds = new Map(this.frameState.currentBounds);
  }
}

// =============================================================================
// Helper Functions
// =============================================================================

function boundsEqual(a: RasterBounds, b: RasterBounds): boolean {
  return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
}

// =============================================================================
// Dependency Interfaces (to be implemented by other modules)
// =============================================================================

/**
 * Constraint solver interface.
 *
 * ## P/Q Boundary Enforcement
 *
 * The solver receives T-vector STATE mutations (hover, scroll_y, etc.)
 * and returns P-dimension SPATIAL coordinates (X, Y, Z).
 *
 * The solver is the ONLY component that derives spatial coordinates.
 * It does so by evaluating constraints of the form:
 *
 *   A.x = 100 when A.T.hover = 1
 *   A.x = 0 when A.T.hover = 0
 *
 * This ensures the constraint graph is the single source of truth.
 */
/**
 * Result of constraint solver evaluation.
 *
 * Contains both P-dimension bounds and pending FFI calls from WASM tick().
 */
interface SolverEvaluationResult {
  /** P-dimension bounds for all entities */
  bounds: Map<EntityId, PVectorBounds>;
  /** Pending FFI calls from trigger evaluation (may be empty) */
  pendingFfiCalls: PendingFfiCall[];
}

interface ConstraintSolver {
  evaluate(mutations: TStateMutation[]): SolverEvaluationResult;
}

interface TopologyRounder {
  round(bounds: Map<EntityId, PVectorBounds>): {
    bounds: Map<EntityId, RasterBounds>;
    violations: unknown[];
  };
}

interface CanvasRenderer {
  getEntity(id: EntityId): RenderableEntity | undefined;
  draw(command: DrawCommand): void;
  flush(): void;
}

interface DOMLayer {
  getElement(id: EntityId): HTMLElement | undefined;
  initializeProxyElement(id: EntityId, element: HTMLElement, bounds: RasterBounds): void;
}

/**
 * Event buffer interface for async event atomicity.
 *
 * The render loop calls mergeAsyncEvents() at the start of each tick
 * to integrate events from async callbacks (fetch, setTimeout, etc.)
 * before processing sync events.
 */
interface EventBufferInterface {
  /**
   * Merge pending async events into the main buffers.
   *
   * Called at tick start to ensure deterministic event ordering.
   */
  mergeAsyncEvents(): void;
}

// =============================================================================
// DOM Proxy Enforcement (Architect Directive: Strict DOM Proxy)
// =============================================================================

/**
 * CSS properties allowed on DOM proxy elements.
 *
 * These are compositor-only properties that do NOT trigger browser reflow.
 * Any attempt to set other properties (margin, padding, display, etc.)
 * will throw a runtime error.
 */
const ALLOWED_CSS_PROPERTIES = new Set([
  'transform',
  'opacity',
  'pointerEvents',
  'pointer-events', // Allow both camelCase and kebab-case
  'willChange',
  'will-change',
]);

/**
 * Initialize a DOM element as a strict ViewScript proxy.
 *
 * This function:
 * 1. Sets all required CSS properties for compositor-only updates
 * 2. Wraps the style object in a Proxy to prevent layout-triggering mutations
 * 3. Ensures the element is a "transparent tactile proxy" with no visual rendering
 *
 * ## Architect Directive: Zero Layout Engine Intervention
 *
 * The browser's layout engine (Reflow) must NEVER be invoked by DOM proxy
 * elements. All positioning is done via GPU-accelerated `transform: translate3d()`.
 */
export function initializeDOMProxyElement(
  element: HTMLElement,
  bounds: RasterBounds,
): void {
  // Store original style object for Proxy wrapping
  const originalStyle = element.style;

  // Apply mandatory CSS for compositor-only updates
  originalStyle.position = 'absolute';
  originalStyle.top = '0';
  originalStyle.left = '0';
  originalStyle.width = `${bounds.width}px`;
  originalStyle.height = `${bounds.height}px`;
  originalStyle.opacity = '0';                    // Visually transparent
  originalStyle.pointerEvents = 'auto';           // Tactile proxy active
  originalStyle.willChange = 'transform';         // GPU layer hint
  originalStyle.margin = '0';                     // Explicit zero
  originalStyle.padding = '0';                    // Explicit zero
  originalStyle.border = 'none';                  // Explicit none
  originalStyle.boxSizing = 'border-box';         // Predictable sizing
  originalStyle.overflow = 'hidden';              // No scrollbars
  originalStyle.transform = `translate3d(${bounds.x}px, ${bounds.y}px, 0)`;

  // Create a Proxy to guard against layout-triggering mutations
  const guardedStyle = new Proxy(originalStyle, {
    set(target, property: string, value: string): boolean {
      // Allow setting allowed properties
      if (ALLOWED_CSS_PROPERTIES.has(property)) {
        return Reflect.set(target, property, value);
      }

      // Block all other properties with a clear error
      throw new Error(
        `[ViewScript DOM Proxy Violation] Cannot set CSS property '${property}' on DOM proxy element. ` +
        `Only compositor-only properties are allowed: ${[...ALLOWED_CSS_PROPERTIES].join(', ')}. ` +
        `This restriction prevents browser layout engine intervention.`
      );
    },

    get(target, property: string): unknown {
      return Reflect.get(target, property);
    },
  });

  // Replace the element's style with the guarded proxy
  // Note: We can't directly assign to element.style, but we can use defineProperty
  Object.defineProperty(element, 'style', {
    value: guardedStyle,
    writable: false,
    configurable: false,
  });
}

/**
 * Update a DOM proxy element's transform (position).
 *
 * This is the ONLY way to change a proxy element's position after initialization.
 * It uses compositor-only transform, avoiding layout recalculation.
 */
export function updateDOMProxyTransform(
  element: HTMLElement,
  x: number,
  y: number,
): void {
  // Direct property access bypasses our Proxy guard (intentionally)
  // because this is an internal, trusted call
  (element as any).style.transform = `translate3d(${x}px, ${y}px, 0)`;
}

// =============================================================================
// Control Flow Summary (for Architect)
// =============================================================================

/**
 * ## tick() Control Flow (Pseudocode)
 *
 * ```
 * function tick(timestamp):
 *   // 1. Backpressure: Take at most N mutations from queue
 *   mutations = pendingMutations.splice(0, MAX_PER_FRAME)
 *
 *   // 2. P-dimension: Solve constraints with new T-vector values
 *   pBounds = constraintSolver.evaluate(mutations)
 *
 *   // 3. Rasterize: Rational → Integer with topology preservation
 *   rBounds = topologyRounder.round(pBounds)
 *
 *   // 4. Diff: Find entities that changed since last frame
 *   dirtySet = diff(previousBounds, rBounds)
 *
 *   // 5. Canvas: Build draw commands for dirty entities only
 *   drawCmds = buildDrawCommands(dirtySet)
 *
 *   // 6. DOM: Build transform mutations (no width/height!)
 *   domMuts = buildDOMMutations(dirtySet)
 *
 *   // 7. ATOMIC COMMIT (no interleaved reads/writes)
 *   for cmd in drawCmds: canvasRenderer.draw(cmd)
 *   canvasRenderer.flush()  // GPU sync point
 *   for mut in domMuts: mut.element.style.transform = mut.transform
 *
 *   // 8. Swap buffers
 *   previousBounds = rBounds
 *
 *   // 9. Schedule next frame
 *   requestAnimationFrame(tick)
 * ```
 *
 * ## Reflow Prevention Strategy
 *
 * 1. DOM elements are created once with fixed dimensions (via CSS)
 * 2. Position changes use ONLY transform: translate3d()
 * 3. translate3d() is compositor-only (GPU, no layout recalc)
 * 4. will-change: transform hints browser to promote layer
 * 5. All style writes happen before any reads (no thrashing)
 * 6. Size changes require re-creating the element (rare)
 */
