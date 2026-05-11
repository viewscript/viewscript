/**
 * HMR Controller: Safe T-Vector State Injection
 *
 * This module manages Hot Module Replacement for ViewScript, ensuring that
 * T-vector state is preserved across AST updates when mathematically safe,
 * and gracefully falls back to initial state when conflicts are detected.
 *
 * ## The Problem: T-Vector State Conflicts
 *
 * When the constraint graph is updated via HMR, the saved T-vector state
 * may be inconsistent with the new constraints. For example:
 *
 * ```
 * [Before HMR]
 *   Entity A: T.hover = true
 *   Constraint: A.x = 100 when T.hover = true
 *
 * [After HMR - new constraints]
 *   Constraint: A.x = 50 when T.hover = true  (changed)
 *   Constraint: A.x < B.x                      (new)
 *   B.x = 30                                   (existing)
 *
 *   Problem: A.x = 50 > B.x = 30 violates the new constraint!
 * ```
 *
 * ## Solution: Satisfiability Check Before Injection
 *
 * 1. Snapshot T-vector state before AST update
 * 2. Build new constraint graph
 * 3. Check if T-vector state satisfies new constraints
 * 4. If satisfiable: inject T-vector state
 * 5. If unsatisfiable: fallback to T=0 (initial state)
 *
 * ## Architect Directive
 *
 * This is a MANDATORY safety mechanism. T-vector injection without
 * satisfiability checking can cause non-deterministic rendering bugs
 * (Jank) that are extremely difficult to diagnose.
 */

// =============================================================================
// Types
// =============================================================================

/**
 * Entity identifier (mirrors vsc-core).
 */
export type EntityId = number;

/**
 * T-vector state snapshot.
 *
 * Maps entity IDs to their T-vector state values.
 * State keys are semantic (hover, scroll_y, etc.), not spatial coordinates.
 */
export interface TvectorSnapshot {
  /** Timestamp when snapshot was taken */
  timestamp: number;

  /** Per-entity state */
  entities: Map<EntityId, EntityTState>;
}

/**
 * T-vector state for a single entity.
 */
export interface EntityTState {
  hover: number;        // 0 or 1
  pressed: number;      // 0 or 1
  focused: number;      // 0 or 1
  scroll_x: number;     // 0.0 to 1.0
  scroll_y: number;     // 0.0 to 1.0
  drag_progress: number; // 0.0 to 1.0
  animation_t: number;  // timeline position
  gesture_phase: number; // 0-3
}

/**
 * AST diff from file watcher.
 */
export interface AstDiff {
  /** Entities added */
  added: EntityId[];

  /** Entities removed */
  removed: EntityId[];

  /** Entities with modified constraints */
  modified: EntityId[];

  /** The new AST (full) */
  newAst: ConstraintGraph;
}

/**
 * Constraint graph (simplified representation).
 */
export interface ConstraintGraph {
  entities: EntityId[];
  constraints: Constraint[];
}

/**
 * A single constraint.
 */
export interface Constraint {
  id: number;
  target: EntityId;
  component: 'x' | 'y' | 'z';
  relation: 'eq' | 'lt' | 'le' | 'gt' | 'ge';
  term: ConstraintTerm;
  /** Optional: condition on T-vector state */
  condition?: TCondition;
}

/**
 * Constraint term.
 */
export type ConstraintTerm =
  | { type: 'const'; value: number }
  | { type: 'ref'; entityId: EntityId; component: 'x' | 'y' | 'z' }
  | { type: 'linear'; coefficient: number; entityId: EntityId; component: 'x' | 'y' | 'z'; offset: number };

/**
 * Condition on T-vector state.
 */
export interface TCondition {
  entityId: EntityId;
  state: keyof EntityTState;
  operator: 'eq' | 'ne' | 'lt' | 'le' | 'gt' | 'ge';
  value: number;
}

/**
 * Result of satisfiability check.
 */
export interface SatisfiabilityResult {
  satisfiable: boolean;
  conflicts: ConflictDetail[];
}

/**
 * Detail of a constraint conflict.
 */
export interface ConflictDetail {
  constraint: Constraint;
  expectedValue: number;
  actualValue: number;
  message: string;
}

/**
 * Result of HMR application.
 */
export interface HMRResult {
  status: 'success' | 'fallback' | 'error';
  tVectorPreserved: boolean;
  conflictDetails?: ConflictDetail[];
  error?: string;
}

// =============================================================================
// Renderer Interface (to be implemented by the actual renderer)
// =============================================================================

export interface IRenderer {
  captureTVector(): TvectorSnapshot;
  injectTVector(snapshot: TvectorSnapshot): void;
  resetTVector(): void;
  updateConstraints(graph: ConstraintGraph): void;
  getEntityValue(entityId: EntityId, component: 'x' | 'y' | 'z'): number;
}

// =============================================================================
// HMR Controller
// =============================================================================

/**
 * HMR Controller with safe T-vector state injection.
 *
 * ## Usage
 *
 * ```typescript
 * const controller = new HMRController(renderer, solver);
 *
 * // When file changes detected:
 * const result = controller.applyAstDiff(diff);
 *
 * if (result.status === 'fallback') {
 *   console.warn('T-vector reset due to conflicts:', result.conflictDetails);
 * }
 * ```
 */
export class HMRController {
  private renderer: IRenderer;

  constructor(renderer: IRenderer) {
    this.renderer = renderer;
  }

  /**
   * Apply an AST diff with safe T-vector state handling.
   *
   * This is the main entry point for HMR. It ensures T-vector state
   * is only preserved when mathematically consistent with the new
   * constraint graph.
   */
  applyAstDiff(diff: AstDiff): HMRResult {
    try {
      // Phase 1: Snapshot current T-vector state
      const tVectorSnapshot = this.renderer.captureTVector();
      console.log('[HMR] Captured T-vector snapshot:', tVectorSnapshot.entities.size, 'entities');

      // Phase 2: Build new constraint graph (without T-vector)
      const newGraph = diff.newAst;

      // Phase 3: Update constraints first (T-vector still at old state)
      this.renderer.updateConstraints(newGraph);

      // Phase 4: Satisfiability check
      const satResult = this.checkSatisfiability(newGraph, tVectorSnapshot);

      // Phase 5: Apply result
      if (satResult.satisfiable) {
        // No conflicts: preserve T-vector state
        this.renderer.injectTVector(tVectorSnapshot);
        console.log('[HMR] T-vector preserved successfully');
        return { status: 'success', tVectorPreserved: true };
      } else {
        // Conflicts detected: fallback to T=0
        console.warn('[HMR] T-vector conflicts detected, falling back to T=0');
        console.warn('[HMR] Conflicts:', satResult.conflicts);
        this.renderer.resetTVector();
        return {
          status: 'fallback',
          tVectorPreserved: false,
          conflictDetails: satResult.conflicts,
        };
      }
    } catch (error) {
      console.error('[HMR] Error during AST diff application:', error);
      // On error, reset to safe state
      this.renderer.resetTVector();
      return {
        status: 'error',
        tVectorPreserved: false,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }

  /**
   * Check if T-vector state satisfies all constraints.
   *
   * This performs a lightweight satisfiability check by:
   * 1. Evaluating all constraints with the given T-vector state
   * 2. Checking if the resulting P-dimension values satisfy relations
   *
   * ## Complexity
   *
   * O(n * m) where n = number of entities, m = number of constraints.
   * For typical UI graphs (< 1000 entities), this is < 1ms.
   */
  private checkSatisfiability(
    graph: ConstraintGraph,
    tVector: TvectorSnapshot,
  ): SatisfiabilityResult {
    const conflicts: ConflictDetail[] = [];

    // Evaluate each constraint
    for (const constraint of graph.constraints) {
      // Check if constraint is active given T-vector state
      if (!this.isConstraintActive(constraint, tVector)) {
        continue; // Constraint not active, skip
      }

      // Get the target entity's current value
      const targetValue = this.renderer.getEntityValue(
        constraint.target,
        constraint.component,
      );

      // Evaluate the term
      const termValue = this.evaluateTerm(constraint.term);

      // Check relation
      const satisfied = this.checkRelation(
        targetValue,
        constraint.relation,
        termValue,
      );

      if (!satisfied) {
        conflicts.push({
          constraint,
          expectedValue: termValue,
          actualValue: targetValue,
          message: `Constraint ${constraint.id}: ${constraint.target}.${constraint.component} ` +
            `${constraint.relation} ${termValue} failed (actual: ${targetValue})`,
        });
      }
    }

    return {
      satisfiable: conflicts.length === 0,
      conflicts,
    };
  }

  /**
   * Check if a constraint is active given T-vector state.
   */
  private isConstraintActive(
    constraint: Constraint,
    tVector: TvectorSnapshot,
  ): boolean {
    if (!constraint.condition) {
      return true; // No condition = always active
    }

    const entityState = tVector.entities.get(constraint.condition.entityId);
    if (!entityState) {
      return false; // Entity not in snapshot = constraint inactive
    }

    const stateValue = entityState[constraint.condition.state];
    const conditionValue = constraint.condition.value;

    switch (constraint.condition.operator) {
      case 'eq': return stateValue === conditionValue;
      case 'ne': return stateValue !== conditionValue;
      case 'lt': return stateValue < conditionValue;
      case 'le': return stateValue <= conditionValue;
      case 'gt': return stateValue > conditionValue;
      case 'ge': return stateValue >= conditionValue;
      default: return true;
    }
  }

  /**
   * Evaluate a constraint term.
   */
  private evaluateTerm(term: ConstraintTerm): number {
    switch (term.type) {
      case 'const':
        return term.value;

      case 'ref':
        return this.renderer.getEntityValue(term.entityId, term.component);

      case 'linear': {
        const refValue = this.renderer.getEntityValue(term.entityId, term.component);
        return term.coefficient * refValue + term.offset;
      }

      default:
        return 0;
    }
  }

  /**
   * Check if a relation is satisfied.
   */
  private checkRelation(
    left: number,
    relation: 'eq' | 'lt' | 'le' | 'gt' | 'ge',
    right: number,
  ): boolean {
    const EPSILON = 1e-10; // For float comparison at rasterization boundary

    switch (relation) {
      case 'eq': return Math.abs(left - right) < EPSILON;
      case 'lt': return left < right - EPSILON;
      case 'le': return left <= right + EPSILON;
      case 'gt': return left > right + EPSILON;
      case 'ge': return left >= right - EPSILON;
      default: return true;
    }
  }

  /**
   * Force a full T-vector reset.
   *
   * Use this when you want to explicitly clear all state
   * (e.g., on navigation or explicit user action).
   */
  forceReset(): void {
    console.log('[HMR] Forcing T-vector reset');
    this.renderer.resetTVector();
  }
}

// =============================================================================
// Factory Function
// =============================================================================

/**
 * Create an HMR controller for the given renderer.
 */
export function createHMRController(renderer: IRenderer): HMRController {
  return new HMRController(renderer);
}

// =============================================================================
// Default Entity T-State
// =============================================================================

/**
 * Create a default (zero) entity T-state.
 */
export function createDefaultEntityTState(): EntityTState {
  return {
    hover: 0,
    pressed: 0,
    focused: 0,
    scroll_x: 0,
    scroll_y: 0,
    drag_progress: 0,
    animation_t: 0,
    gesture_phase: 0,
  };
}

// =============================================================================
// Phase 10: Text Metrics Q→P Bridge
// =============================================================================

/**
 * Text entity metadata from the constraint graph.
 */
export interface TextEntityInfo {
  id: number;
  content: string;
  fontFamily: string;
  fontSize: number;
  cornerTl: number;
  cornerTr: number;
  cornerBl: number;
  cornerBr: number;
}

/**
 * Measured text metrics.
 */
export interface TextMetrics {
  width: number;
  height: number;
}

/**
 * Result of updating metrics for a text entity.
 */
export interface UpdateMetricsResult {
  success: boolean;
  entityId: number;
  width: number;
  height: number;
  constraintsAdded?: number;
  error?: string;
}

/**
 * Interface for CLI command execution.
 */
export interface ICLIExecutor {
  /**
   * Execute a CLI command and return the result.
   */
  execute(args: string[]): Promise<{ exitCode: number; stdout: string; stderr: string }>;
}

/**
 * Text Metrics Measurer: Q→P Dimension Bridge
 *
 * This class measures text dimensions using browser APIs (CanvasKit or DOM)
 * and feeds the results back to the P-dimension constraint solver via CLI.
 *
 * ## Architecture
 *
 * ```
 *   Renderer (Q-dimension)          CLI (P-dimension)
 *   ┌─────────────────────┐        ┌─────────────────────┐
 *   │ CanvasKit/DOM       │        │ vsc update-metrics  │
 *   │ measureText()       │───────▶│ --id=N              │
 *   │                     │        │ --width=W           │
 *   │                     │        │ --height=H          │
 *   └─────────────────────┘        └─────────────────────┘
 * ```
 *
 * ## Usage
 *
 * ```typescript
 * const measurer = new TextMetricsMeasurer(cliExecutor);
 *
 * // After adding a text entity via CLI:
 * const result = await measurer.measureAndUpdate({
 *   id: 1000,
 *   content: "Hello, World!",
 *   fontFamily: "Inter",
 *   fontSize: 16,
 *   cornerTl: 1001,
 *   cornerTr: 1002,
 *   cornerBl: 1003,
 *   cornerBr: 1004,
 * });
 * ```
 */
export class TextMetricsMeasurer {
  private cliExecutor: ICLIExecutor;
  private measurementCanvas: HTMLCanvasElement | null = null;
  private measurementContext: CanvasRenderingContext2D | null = null;

  constructor(cliExecutor: ICLIExecutor) {
    this.cliExecutor = cliExecutor;
  }

  /**
   * Initialize the measurement canvas (lazy initialization).
   */
  private initMeasurementCanvas(): void {
    if (this.measurementCanvas) return;

    // Create an off-screen canvas for text measurement
    this.measurementCanvas = document.createElement('canvas');
    this.measurementCanvas.width = 1;
    this.measurementCanvas.height = 1;
    this.measurementContext = this.measurementCanvas.getContext('2d');
  }

  /**
   * Measure text dimensions using Canvas 2D API.
   *
   * This is a fallback for environments without CanvasKit.
   * For production, prefer CanvasKit's measureText for accuracy.
   */
  measureText(content: string, fontFamily: string, fontSize: number): TextMetrics {
    this.initMeasurementCanvas();

    if (!this.measurementContext) {
      // Fallback: estimate based on character count (very rough)
      const avgCharWidth = fontSize * 0.6;
      return {
        width: Math.ceil(content.length * avgCharWidth),
        height: Math.ceil(fontSize * 1.2),
      };
    }

    // Set font and measure
    this.measurementContext.font = `${fontSize}px "${fontFamily}"`;
    const metrics = this.measurementContext.measureText(content);

    // Calculate height from font metrics
    // Note: actualBoundingBoxAscent/Descent may not be available in all browsers
    const height = metrics.actualBoundingBoxAscent !== undefined
      ? metrics.actualBoundingBoxAscent + metrics.actualBoundingBoxDescent
      : fontSize * 1.2; // Fallback

    return {
      width: Math.ceil(metrics.width),
      height: Math.ceil(height),
    };
  }

  /**
   * Measure text and update the P-dimension constraint graph via CLI.
   *
   * ## Process
   *
   * 1. Measure text dimensions using Canvas 2D API
   * 2. Call `vsc update-metrics` with measured dimensions
   * 3. Return the result
   *
   * ## Rational Number Conversion
   *
   * Browser measurements are floating-point. We convert to integers
   * by rounding up (ceiling) to ensure the bounding box fully contains
   * the text.
   */
  async measureAndUpdate(textEntity: TextEntityInfo): Promise<UpdateMetricsResult> {
    // Step 1: Measure
    const metrics = this.measureText(
      textEntity.content,
      textEntity.fontFamily,
      textEntity.fontSize,
    );

    // Step 2: Update via CLI
    const result = await this.cliExecutor.execute([
      'update-metrics',
      `--id=${textEntity.id}`,
      `--width=${metrics.width}`,
      `--height=${metrics.height}`,
    ]);

    if (result.exitCode !== 0) {
      return {
        success: false,
        entityId: textEntity.id,
        width: metrics.width,
        height: metrics.height,
        error: result.stderr || 'CLI command failed',
      };
    }

    // Parse result to get constraints added count
    let constraintsAdded = 0;
    try {
      const output = JSON.parse(result.stdout);
      constraintsAdded = output.constraints_added || 0;
    } catch {
      // Ignore parse errors
    }

    return {
      success: true,
      entityId: textEntity.id,
      width: metrics.width,
      height: metrics.height,
      constraintsAdded,
    };
  }

  /**
   * Batch measure and update multiple text entities.
   *
   * Useful when loading a constraint graph with multiple pending text entities.
   */
  async measureAndUpdateBatch(
    textEntities: TextEntityInfo[],
  ): Promise<UpdateMetricsResult[]> {
    const results: UpdateMetricsResult[] = [];

    for (const entity of textEntities) {
      const result = await this.measureAndUpdate(entity);
      results.push(result);
    }

    return results;
  }
}

/**
 * Create a text metrics measurer for the given CLI executor.
 */
export function createTextMetricsMeasurer(cliExecutor: ICLIExecutor): TextMetricsMeasurer {
  return new TextMetricsMeasurer(cliExecutor);
}
