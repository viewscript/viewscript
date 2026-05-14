/**
 * ViewScript Renderer AST: Bilayer Orthogonal Architecture
 *
 * This module defines the type system for the dual-layer rendering model:
 * - Canvas Layer: Visual representation (wgpu draw commands)
 * - DOM Layer: Interaction regions (accessibility, events, focus)
 *
 * ## Architectural Invariants
 *
 * 1. Every logical entity has exactly one EntityId
 * 2. An entity MAY have a CanvasNode (visual), a DOMNode (interactive), or both
 * 3. Canvas and DOM nodes are synchronized via shared EntityId
 * 4. DOM nodes have NO visual properties; Canvas nodes have NO interaction logic
 *
 * ## Data Flow
 *
 * ```
 *   IR (.vs)                    Renderer AST
 *   ────────────────────────────────────────────────────────
 *
 *   ┌─────────────┐         ┌─────────────────────────────┐
 *   │  Constraint │         │       RenderableEntity      │
 *   │    Graph    │   ──▶   │  ┌─────────┐ ┌───────────┐  │
 *   │  (P-dim)    │         │  │ Canvas  │ │   DOM     │  │
 *   └─────────────┘         │  │  Node   │ │   Node    │  │
 *                           │  └─────────┘ └───────────┘  │
 *                           │       ▲           ▲         │
 *                           │       └─────┬─────┘         │
 *                           │         EntityId            │
 *                           └─────────────────────────────┘
 * ```
 */

// =============================================================================
// Core Identity Types
// =============================================================================

/** Unique identifier for all P-dimension entities. */
export type EntityId = number;

/** Unique identifier for constraints. */
export type ConstraintId = number;

/** Unique identifier for render chunks (for progressive loading). */
export type ChunkId = string;

// =============================================================================
// Coordinate Types (Pre-Rasterization)
// =============================================================================

/**
 * Rational number representation for exact arithmetic.
 * Used before rasterization to pixel coordinates.
 */
export interface Rational {
  numerator: bigint;
  denominator: bigint;
}

/**
 * P-dimension vector with exact rational coordinates.
 * T is included for animation/state-dependent positioning.
 */
export interface PVector {
  x: Rational;
  y: Rational;
  z: Rational; // Layering order (not depth in 3D sense)
  t: Rational; // Time/state parameter
}

/**
 * Rasterized coordinates for actual rendering.
 * Produced by the topology-preserving rounding algorithm.
 */
export interface RasterCoord {
  /** X in device pixels */
  x: number;
  /** Y in device pixels */
  y: number;
  /** Z-index for layering */
  zIndex: number;
}

// =============================================================================
// Canvas Layer Types (Visual Representation)
// =============================================================================

/**
 * Base interface for all Canvas layer nodes.
 * These produce wgpu draw commands.
 */
export interface CanvasNodeBase {
  /** Discriminant for node type */
  readonly kind: string;

  /** Back-reference to logical entity */
  entityId: EntityId;

  /** Pre-rasterization coordinates */
  bounds: PVectorBounds;

  /** Rasterized coordinates (computed by rounding algorithm) */
  rasterBounds: RasterBounds;

  /** Z-order for painter's algorithm */
  zOrder: number;

  /** Chunk this node belongs to (for progressive loading) */
  chunkId: ChunkId;
}

export interface PVectorBounds {
  topLeft: PVector;
  bottomRight: PVector;
}

export interface RasterBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Path-based canvas node (curves, shapes).
 */
export interface CanvasPathNode extends CanvasNodeBase {
  kind: 'path';

  /** SVG-like path commands, pre-compiled */
  pathData: PathCommand[];

  /** Fill style (solid, gradient, pattern) */
  fill: FillStyle | null;

  /** Stroke style */
  stroke: StrokeStyle | null;
}

export type PathCommand =
  | { type: 'M'; x: Rational; y: Rational }
  | { type: 'L'; x: Rational; y: Rational }
  | { type: 'C'; x1: Rational; y1: Rational; x2: Rational; y2: Rational; x: Rational; y: Rational }
  | { type: 'Q'; x1: Rational; y1: Rational; x: Rational; y: Rational }
  | { type: 'A'; rx: Rational; ry: Rational; rotation: number; largeArc: boolean; sweep: boolean; x: Rational; y: Rational }
  | { type: 'Z' };

export interface FillStyle {
  type: 'solid' | 'linear-gradient' | 'radial-gradient' | 'pattern';
  color?: string; // For solid
  stops?: GradientStop[]; // For gradients
  patternRef?: EntityId; // For patterns
}

export interface GradientStop {
  offset: Rational; // 0-1
  color: string;
}

export interface StrokeStyle {
  color: string;
  width: Rational;
  lineCap: 'butt' | 'round' | 'square';
  lineJoin: 'miter' | 'round' | 'bevel';
  dashArray?: Rational[];
}

/**
 * Text canvas node.
 */
export interface CanvasTextNode extends CanvasNodeBase {
  kind: 'text';

  /** Text content (may be Q-dimension bound) */
  content: string | QDimensionRef;

  /** Font specification */
  font: FontSpec;

  /** Text layout results (computed) */
  glyphs: GlyphRun[];
}

export interface FontSpec {
  family: string;
  size: Rational;
  weight: number;
  style: 'normal' | 'italic' | 'oblique';
}

export interface GlyphRun {
  glyphIds: number[];
  positions: RasterCoord[];
}

/**
 * Image canvas node (Q-dimension source).
 */
export interface CanvasImageNode extends CanvasNodeBase {
  kind: 'image';

  /** Reference to Q-dimension image source */
  source: QDimensionRef;

  /** How to fit the image in bounds */
  fit: 'fill' | 'contain' | 'cover' | 'none';
}

/**
 * Group node for hierarchical transforms.
 */
export interface CanvasGroupNode extends CanvasNodeBase {
  kind: 'group';

  /** Child nodes */
  children: CanvasNode[];

  /** Transform matrix (2D affine) */
  transform: AffineTransform;

  /** Clip path (optional) */
  clipPath?: PathCommand[];

  /** Opacity (0-1) */
  opacity: number;
}

export interface AffineTransform {
  a: number; b: number;
  c: number; d: number;
  tx: number; ty: number;
}

export type CanvasNode =
  | CanvasPathNode
  | CanvasTextNode
  | CanvasImageNode
  | CanvasGroupNode;

// =============================================================================
// DOM Layer Types (Interaction Regions)
// =============================================================================

/**
 * Base interface for all DOM layer nodes.
 * These produce invisible DOM elements for interaction.
 */
export interface DOMNodeBase {
  /** Discriminant for node type */
  readonly kind: string;

  /** Back-reference to logical entity (MUST match CanvasNode if paired) */
  entityId: EntityId;

  /** Position synchronized with Canvas layer */
  rasterBounds: RasterBounds;

  /** ARIA attributes for accessibility */
  aria: ARIAAttributes;

  /** Tab index for keyboard navigation (-1 = not focusable) */
  tabIndex: number;
}

export interface ARIAAttributes {
  role?: string;
  label?: string;
  describedBy?: EntityId;
  labelledBy?: EntityId;
  hidden?: boolean;
  expanded?: boolean;
  selected?: boolean;
  checked?: boolean | 'mixed';
  disabled?: boolean;
  live?: 'off' | 'polite' | 'assertive';
}

/**
 * Interactive region (clickable, focusable).
 */
export interface DOMInteractiveNode extends DOMNodeBase {
  kind: 'interactive';

  /** Event bindings (Q-dimension triggers) */
  events: EventBinding[];

  /** Cursor style when hovering */
  cursor: string;
}

export interface EventBinding {
  /** DOM event type */
  type: 'click' | 'pointerdown' | 'pointerup' | 'pointermove' | 'focus' | 'blur' | 'keydown' | 'keyup';

  /** Constraint to update on event (T-vector mutation) */
  targetConstraint: ConstraintId;

  /** How to compute the new value */
  valueMapping: EventValueMapping;
}

export type EventValueMapping =
  | { type: 'constant'; value: Rational }
  | { type: 'toggle'; values: [Rational, Rational] }
  | { type: 'increment'; delta: Rational }
  | { type: 'pointer-x' }
  | { type: 'pointer-y' }
  | { type: 'pointer-delta-x' }
  | { type: 'pointer-delta-y' };

/**
 * Text input region.
 */
export interface DOMInputNode extends DOMNodeBase {
  kind: 'input';

  /** Input type */
  inputType: 'text' | 'number' | 'password' | 'email' | 'search';

  /** Constraint bound to input value */
  valueConstraint: ConstraintId;

  /** Placeholder text */
  placeholder?: string;

  /** Max length */
  maxLength?: number;
}

/**
 * Scroll container region.
 */
export interface DOMScrollNode extends DOMNodeBase {
  kind: 'scroll';

  /** Scroll direction */
  direction: 'horizontal' | 'vertical' | 'both';

  /** Constraints bound to scroll position */
  scrollXConstraint?: ConstraintId;
  scrollYConstraint?: ConstraintId;

  /** Content size (for scrollbar calculation) */
  contentSize: { width: Rational; height: Rational };
}

/**
 * Focus trap region (for modals, dialogs).
 */
export interface DOMFocusTrapNode extends DOMNodeBase {
  kind: 'focus-trap';

  /** First and last focusable children */
  focusableChildren: EntityId[];
}

export type DOMNode =
  | DOMInteractiveNode
  | DOMInputNode
  | DOMScrollNode
  | DOMFocusTrapNode;

// =============================================================================
// Q-Dimension References
// =============================================================================

/**
 * Reference to Q-dimension (unpredictable) data.
 */
export interface QDimensionRef {
  /** Type of Q-dimension source */
  type: 'user-input' | 'fetch' | 'image' | 'video' | 'audio' | 'shader' | 'time';

  /** Source identifier */
  sourceId: string;

  /** Current value (runtime state, not in IR) */
  currentValue?: unknown;
}

// =============================================================================
// Unified Renderable Entity
// =============================================================================

/**
 * A complete renderable entity combining Canvas and DOM representations.
 *
 * ## Pairing Rules
 *
 * | Entity Type     | Canvas Node | DOM Node    |
 * |-----------------|-------------|-------------|
 * | Static shape    | Required    | None        |
 * | Button          | Required    | Required    |
 * | Text input      | Required    | Required    |
 * | Decorative img  | Required    | None        |
 * | Interactive img | Required    | Required    |
 * | Scroll area     | Optional    | Required    |
 * | Focus trap      | None        | Required    |
 */
export interface RenderableEntity {
  /** Unique identifier */
  id: EntityId;

  /** Human-readable name (for debugging) */
  name?: string;

  /** Canvas layer representation (visual) */
  canvas: CanvasNode | null;

  /** DOM layer representation (interactive) */
  dom: DOMNode | null;

  /** Constraints that affect this entity */
  dependentConstraints: ConstraintId[];

  /** Entities that this entity references */
  referencedEntities: EntityId[];

  /** Chunk membership */
  chunkId: ChunkId;
}

// =============================================================================
// Render Tree (Complete AST)
// =============================================================================

/**
 * The complete render tree produced by the compiler.
 */
export interface RenderTree {
  /** All renderable entities, indexed by EntityId */
  entities: Map<EntityId, RenderableEntity>;

  /** Root entity IDs (top-level elements) */
  roots: EntityId[];

  /** Chunk definitions for progressive loading */
  chunks: Map<ChunkId, Chunk>;

  /** Viewport configuration */
  viewport: ViewportConfig;

  /** Device pixel ratio for rasterization */
  devicePixelRatio: number;
}

export interface Chunk {
  id: ChunkId;

  /** Entities in this chunk */
  entityIds: EntityId[];

  /** Dependencies on other chunks */
  dependsOn: ChunkId[];

  /** Is this the initial chunk? */
  isInitial: boolean;

  /** Trigger conditions for lazy loading */
  loadTriggers: LoadTrigger[];
}

export type LoadTrigger =
  | { type: 'immediate' }
  | { type: 'viewport-intersect'; entityId: EntityId }
  | { type: 'event'; eventType: string; targetEntity: EntityId }
  | { type: 'constraint-change'; constraintId: ConstraintId };

export interface ViewportConfig {
  width: Rational;
  height: Rational;
  unitsPerPixel: Rational;
}
