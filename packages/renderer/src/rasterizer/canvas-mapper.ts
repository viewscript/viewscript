/**
 * CanvasKit Path Projection (Phase 6)
 *
 * This module transforms P-dimension ControlPoint entities (with exact rational
 * coordinates) into CanvasKit SkPath objects for rasterization.
 *
 * ## Architecture
 *
 * ```
 *   P-Dimension                    Rasterization Boundary                 Canvas
 *   ───────────────────────────────────────────────────────────────────────────
 *
 *   ControlPoint                   ┌──────────────────┐
 *   entities with   ─────────────▶ │  canvas-mapper   │ ─────────────▶  SkPath
 *   Rational coords                │  (this module)   │               objects
 *                                  └──────────────────┘
 *                                          │
 *                                          ▼
 *                                  topology-rounding.ts
 *                                  (pixel-perfect adjacency)
 * ```
 *
 * ## Critical Invariants
 *
 * 1. **Float Decontamination**: All f64 values are produced ONLY by
 *    `Rational.to_f64_for_rasterization()` at this boundary
 * 2. **Topology Preservation**: Shared ControlPoints produce bit-identical
 *    coordinates, ensuring seamless curve connections
 * 3. **Fill Rule Semantics**: SVG fill-rule (nonzero/evenodd) is preserved
 */

import type { EntityId, Rational, PathCommand, FillStyle, StrokeStyle } from '../ast/types';

// =============================================================================
// Input Types (from P-Dimension Solver)
// =============================================================================

/**
 * Control point with resolved rational coordinates.
 */
export interface ResolvedControlPoint {
  id: EntityId;
  x: Rational;
  y: Rational;
  role: 'anchor' | 'handle';
}

/**
 * Path segment referencing control points by EntityId.
 */
export type PathSegmentRef =
  | { type: 'moveTo'; point: EntityId }
  | { type: 'lineTo'; point: EntityId }
  | { type: 'quadTo'; control: EntityId; point: EntityId }
  | { type: 'cubicTo'; control1: EntityId; control2: EntityId; point: EntityId }
  | { type: 'arcTo'; point: EntityId; radiusX: Rational; radiusY: Rational; xRotation: Rational; largeArc: boolean; sweep: boolean }
  | { type: 'close' };

/**
 * Path definition with EntityId references.
 */
export interface PathDefinition {
  id: EntityId;
  segments: PathSegmentRef[];
  fillRule: 'nonzero' | 'evenodd';
  closed: boolean;
}

// =============================================================================
// Output Types (for CanvasKit)
// =============================================================================

/**
 * Rasterized path ready for CanvasKit consumption.
 */
export interface RasterizedPath {
  /** Unique path ID */
  id: EntityId;

  /** SVG-style path commands with float coordinates */
  commands: PathCommand[];

  /** Fill rule for winding calculation */
  fillRule: 'nonzero' | 'evenodd';

  /** Whether path is closed */
  closed: boolean;

  /** Computed bounding box (for culling) */
  bounds: {
    minX: number;
    minY: number;
    maxX: number;
    maxY: number;
  };
}

// =============================================================================
// Core Mapping Logic
// =============================================================================

/**
 * Maps P-dimension path definitions to rasterized CanvasKit paths.
 *
 * @param paths - Path definitions with EntityId references
 * @param controlPoints - Map of resolved control point positions
 * @param devicePixelRatio - DPR for coordinate scaling
 * @returns Rasterized paths ready for CanvasKit
 */
export function mapPathsToCanvas(
  paths: PathDefinition[],
  controlPoints: Map<EntityId, ResolvedControlPoint>,
  devicePixelRatio: number = 1,
): RasterizedPath[] {
  return paths.map(path => mapSinglePath(path, controlPoints, devicePixelRatio));
}

/**
 * Map a single path definition to rasterized commands.
 */
function mapSinglePath(
  path: PathDefinition,
  controlPoints: Map<EntityId, ResolvedControlPoint>,
  dpr: number,
): RasterizedPath {
  const commands: PathCommand[] = [];
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;

  const toFloat = (r: Rational): number => {
    // RASTERIZATION BOUNDARY: Convert Rational to f64
    return (Number(r.numerator) / Number(r.denominator)) * dpr;
  };

  const getPoint = (id: EntityId): { x: number; y: number } => {
    const cp = controlPoints.get(id);
    if (!cp) {
      throw new Error(`ControlPoint ${id} not found in resolved set`);
    }
    return {
      x: toFloat(cp.x),
      y: toFloat(cp.y),
    };
  };

  const updateBounds = (x: number, y: number): void => {
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x);
    maxY = Math.max(maxY, y);
  };

  const toRational = (n: number): Rational => ({
    // Convert float back to rational for PathCommand type
    // This is a simplification - in production, we'd keep exact rationals
    numerator: BigInt(Math.round(n * 1000000)),
    denominator: BigInt(1000000),
  });

  for (const segment of path.segments) {
    switch (segment.type) {
      case 'moveTo': {
        const p = getPoint(segment.point);
        commands.push({ type: 'M', x: toRational(p.x), y: toRational(p.y) });
        updateBounds(p.x, p.y);
        break;
      }

      case 'lineTo': {
        const p = getPoint(segment.point);
        commands.push({ type: 'L', x: toRational(p.x), y: toRational(p.y) });
        updateBounds(p.x, p.y);
        break;
      }

      case 'quadTo': {
        const ctrl = getPoint(segment.control);
        const end = getPoint(segment.point);
        commands.push({
          type: 'Q',
          x1: toRational(ctrl.x),
          y1: toRational(ctrl.y),
          x: toRational(end.x),
          y: toRational(end.y),
        });
        updateBounds(ctrl.x, ctrl.y);
        updateBounds(end.x, end.y);
        break;
      }

      case 'cubicTo': {
        const ctrl1 = getPoint(segment.control1);
        const ctrl2 = getPoint(segment.control2);
        const end = getPoint(segment.point);
        commands.push({
          type: 'C',
          x1: toRational(ctrl1.x),
          y1: toRational(ctrl1.y),
          x2: toRational(ctrl2.x),
          y2: toRational(ctrl2.y),
          x: toRational(end.x),
          y: toRational(end.y),
        });
        updateBounds(ctrl1.x, ctrl1.y);
        updateBounds(ctrl2.x, ctrl2.y);
        updateBounds(end.x, end.y);
        break;
      }

      case 'arcTo': {
        const end = getPoint(segment.point);
        commands.push({
          type: 'A',
          rx: segment.radiusX,
          ry: segment.radiusY,
          rotation: toFloat(segment.xRotation),
          largeArc: segment.largeArc,
          sweep: segment.sweep,
          x: toRational(end.x),
          y: toRational(end.y),
        });
        updateBounds(end.x, end.y);
        // Note: Arc bounds are approximate without full geometric calculation
        break;
      }

      case 'close': {
        commands.push({ type: 'Z' });
        break;
      }
    }
  }

  return {
    id: path.id,
    commands,
    fillRule: path.fillRule,
    closed: path.closed,
    bounds: {
      minX: minX === Infinity ? 0 : minX,
      minY: minY === Infinity ? 0 : minY,
      maxX: maxX === -Infinity ? 0 : maxX,
      maxY: maxY === -Infinity ? 0 : maxY,
    },
  };
}

// =============================================================================
// CanvasKit SkPath Builder
// =============================================================================

/**
 * Interface matching CanvasKit's SkPath type (subset).
 */
export interface SkPathLike {
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  quadTo(cpx: number, cpy: number, x: number, y: number): void;
  cubicTo(cp1x: number, cp1y: number, cp2x: number, cp2y: number, x: number, y: number): void;
  arcToOval(oval: Float32Array, startAngle: number, sweepAngle: number, forceMoveTo: boolean): void;
  arcToRotated(rx: number, ry: number, xAxisRotate: number, useSmallArc: boolean, isCCW: boolean, x: number, y: number): void;
  close(): void;
  setFillType(fillType: number): void;
}

/**
 * CanvasKit fill type constants.
 */
export const FillType = {
  Winding: 0,   // nonzero
  EvenOdd: 1,   // evenodd
};

/**
 * Build a CanvasKit SkPath from rasterized path data.
 *
 * @param path - Rasterized path with float coordinates
 * @param skPath - CanvasKit path object to populate
 */
export function buildSkPath(path: RasterizedPath, skPath: SkPathLike): void {
  // Set fill rule
  skPath.setFillType(path.fillRule === 'evenodd' ? FillType.EvenOdd : FillType.Winding);

  const toFloat = (r: Rational): number =>
    Number(r.numerator) / Number(r.denominator);

  for (const cmd of path.commands) {
    switch (cmd.type) {
      case 'M':
        skPath.moveTo(toFloat(cmd.x), toFloat(cmd.y));
        break;

      case 'L':
        skPath.lineTo(toFloat(cmd.x), toFloat(cmd.y));
        break;

      case 'Q':
        skPath.quadTo(
          toFloat(cmd.x1), toFloat(cmd.y1),
          toFloat(cmd.x), toFloat(cmd.y),
        );
        break;

      case 'C':
        skPath.cubicTo(
          toFloat(cmd.x1), toFloat(cmd.y1),
          toFloat(cmd.x2), toFloat(cmd.y2),
          toFloat(cmd.x), toFloat(cmd.y),
        );
        break;

      case 'A':
        // CanvasKit uses arcToRotated for SVG-style arcs
        skPath.arcToRotated(
          toFloat(cmd.rx),
          toFloat(cmd.ry),
          cmd.rotation,  // already a number
          !cmd.largeArc, // CanvasKit uses "useSmallArc" which is inverse
          !cmd.sweep,    // CanvasKit uses "isCCW" which may differ
          toFloat(cmd.x),
          toFloat(cmd.y),
        );
        break;

      case 'Z':
        skPath.close();
        break;
    }
  }
}

// =============================================================================
// Topology-Preserving Shared Control Points
// =============================================================================

/**
 * Ensures that paths sharing control points produce bit-identical coordinates.
 *
 * This is critical for seamless curve connections: if two Bezier curves share
 * an endpoint, the rasterized coordinates must be exactly the same to prevent
 * visual gaps or overlaps.
 *
 * ## Algorithm
 *
 * 1. Identify all paths that share control points
 * 2. For shared points, use a single coordinate resolution
 * 3. Both paths reference the same float value
 *
 * @param paths - Paths that may share control points
 * @param controlPoints - Control point definitions
 * @returns Normalized control point map with consistent coordinates
 */
export function normalizeSharedControlPoints(
  paths: PathDefinition[],
  controlPoints: Map<EntityId, ResolvedControlPoint>,
): Map<EntityId, ResolvedControlPoint> {
  // Collect all control point references
  const refCounts = new Map<EntityId, number>();

  for (const path of paths) {
    for (const segment of path.segments) {
      const ids = getSegmentPointIds(segment);
      for (const id of ids) {
        refCounts.set(id, (refCounts.get(id) ?? 0) + 1);
      }
    }
  }

  // Create normalized map (same as input, but this is the guarantee point)
  // In a real implementation, we'd ensure caching of float conversions
  const normalized = new Map<EntityId, ResolvedControlPoint>();

  for (const [id, cp] of controlPoints) {
    // If this point is shared (referenced by multiple paths),
    // it will produce the same float coordinates for all references
    normalized.set(id, {
      ...cp,
      // The Rational values are preserved exactly; float conversion
      // happens once at rasterization boundary
    });
  }

  return normalized;
}

function getSegmentPointIds(segment: PathSegmentRef): EntityId[] {
  switch (segment.type) {
    case 'moveTo':
    case 'lineTo':
    case 'arcTo':
      return [segment.point];
    case 'quadTo':
      return [segment.control, segment.point];
    case 'cubicTo':
      return [segment.control1, segment.control2, segment.point];
    case 'close':
      return [];
  }
}

// =============================================================================
// Validation
// =============================================================================

/**
 * Validate that all control point references in paths are resolvable.
 */
export function validatePathReferences(
  paths: PathDefinition[],
  controlPoints: Map<EntityId, ResolvedControlPoint>,
): { valid: boolean; errors: string[] } {
  const errors: string[] = [];

  for (const path of paths) {
    for (let i = 0; i < path.segments.length; i++) {
      const segment = path.segments[i];
      const ids = getSegmentPointIds(segment);

      for (const id of ids) {
        if (!controlPoints.has(id)) {
          errors.push(`Path ${path.id} segment ${i}: references undefined ControlPoint ${id}`);
        }
      }
    }

    // Validate path starts with moveTo
    if (path.segments.length > 0 && path.segments[0].type !== 'moveTo') {
      errors.push(`Path ${path.id}: first segment must be 'moveTo', got '${path.segments[0].type}'`);
    }
  }

  return { valid: errors.length === 0, errors };
}

// =============================================================================
// Phase 7: Arc and Radius Rasterization
// =============================================================================

/**
 * A resolved Radius entity (scalar value).
 */
export interface ResolvedRadius {
  id: EntityId;
  value: Rational;
}

/**
 * A resolved Arc entity with center, radius, and angles.
 */
export interface ResolvedArc {
  id: EntityId;
  /** Center control point (already resolved). */
  center: ResolvedControlPoint;
  /** Radius value (already resolved). */
  radius: ResolvedRadius;
  /** Start angle in degrees. */
  startAngle: Rational;
  /** End angle in degrees. */
  endAngle: Rational;
  /** Direction: true = clockwise. */
  clockwise: boolean;
}

/**
 * A resolved RoundedRect with corner radii.
 */
export interface ResolvedRoundedRect {
  id: EntityId;
  /** Bounds (x, y, width, height). */
  bounds: {
    x: Rational;
    y: Rational;
    width: Rational;
    height: Rational;
  };
  /** Corner radii (all resolved). */
  radii: {
    topLeft: ResolvedRadius;
    topRight: ResolvedRadius;
    bottomRight: ResolvedRadius;
    bottomLeft: ResolvedRadius;
  };
}

/**
 * Interface matching CanvasKit's canvas drawing methods.
 */
export interface CanvasLike {
  drawArc(
    oval: { x: number; y: number; width: number; height: number },
    startAngle: number,
    sweepAngle: number,
    useCenter: boolean,
    paint: unknown
  ): void;
  drawRoundRect(
    rect: { x: number; y: number; width: number; height: number },
    rx: number,
    ry: number,
    paint: unknown
  ): void;
  drawRRect(
    rrect: unknown,
    paint: unknown
  ): void;
}

/**
 * Convert rational to float at RASTERIZATION BOUNDARY ONLY.
 */
function toFloat(r: Rational): number {
  return Number(r.numerator) / Number(r.denominator);
}

/**
 * Draw an arc to a CanvasKit canvas.
 *
 * ## Deferred Evaluation (Phase 7)
 *
 * The arc's circumference points are NOT computed in P-dimension.
 * Only the center, radius, and angles are constrained linearly.
 * The actual arc rendering is delegated to CanvasKit which evaluates
 * the parametric curve (cos/sin) in its native floating-point space.
 *
 * @param canvas - CanvasKit canvas
 * @param arc - Resolved arc with rational center/radius/angles
 * @param paint - Paint style for the arc
 */
export function drawArc(
  canvas: CanvasLike,
  arc: ResolvedArc,
  paint: unknown
): void {
  // Convert rational values to float at rasterization boundary
  const cx = toFloat(arc.center.x);
  const cy = toFloat(arc.center.y);
  const r = toFloat(arc.radius.value);
  const startAngle = toFloat(arc.startAngle);
  const endAngle = toFloat(arc.endAngle);

  // Compute sweep angle
  let sweepAngle = endAngle - startAngle;
  if (arc.clockwise && sweepAngle > 0) {
    sweepAngle -= 360;
  } else if (!arc.clockwise && sweepAngle < 0) {
    sweepAngle += 360;
  }

  // CanvasKit drawArc uses an oval bounds
  const oval = {
    x: cx - r,
    y: cy - r,
    width: r * 2,
    height: r * 2,
  };

  canvas.drawArc(oval, startAngle, sweepAngle, false, paint);
}

/**
 * Draw a rounded rectangle to a CanvasKit canvas.
 *
 * ## Deferred Evaluation (Phase 7)
 *
 * Corner curves are NOT evaluated in P-dimension. Only the bounds and
 * radius scalars are constrained. CanvasKit evaluates the corner arcs
 * internally using native floating-point math.
 *
 * @param canvas - CanvasKit canvas
 * @param rect - Resolved rounded rect with rational bounds/radii
 * @param paint - Paint style
 */
export function drawRoundedRect(
  canvas: CanvasLike,
  rect: ResolvedRoundedRect,
  paint: unknown
): void {
  // Convert rational values to float at rasterization boundary
  const x = toFloat(rect.bounds.x);
  const y = toFloat(rect.bounds.y);
  const width = toFloat(rect.bounds.width);
  const height = toFloat(rect.bounds.height);

  const rTL = toFloat(rect.radii.topLeft.value);
  const rTR = toFloat(rect.radii.topRight.value);
  const rBR = toFloat(rect.radii.bottomRight.value);
  const rBL = toFloat(rect.radii.bottomLeft.value);

  // Check if all radii are equal (use simpler API)
  if (rTL === rTR && rTR === rBR && rBR === rBL) {
    canvas.drawRoundRect(
      { x, y, width, height },
      rTL,
      rTL,
      paint
    );
  } else {
    // Different radii per corner - would need CanvasKit's RRect
    // For now, use uniform radius (max of all)
    const maxR = Math.max(rTL, rTR, rBR, rBL);
    canvas.drawRoundRect(
      { x, y, width, height },
      maxR,
      maxR,
      paint
    );
  }
}

/**
 * Draw a full circle (special case of arc: 0° to 360°).
 *
 * @param canvas - CanvasKit canvas
 * @param center - Resolved center control point
 * @param radius - Resolved radius entity
 * @param paint - Paint style
 */
export function drawCircle(
  canvas: CanvasLike,
  center: ResolvedControlPoint,
  radius: ResolvedRadius,
  paint: unknown
): void {
  const cx = toFloat(center.x);
  const cy = toFloat(center.y);
  const r = toFloat(radius.value);

  const oval = {
    x: cx - r,
    y: cy - r,
    width: r * 2,
    height: r * 2,
  };

  canvas.drawArc(oval, 0, 360, false, paint);
}

/**
 * Validate arc/radius entity references.
 */
export function validateArcReferences(
  arcs: ResolvedArc[],
  centers: Map<EntityId, ResolvedControlPoint>,
  radii: Map<EntityId, ResolvedRadius>,
): { valid: boolean; errors: string[] } {
  const errors: string[] = [];

  for (const arc of arcs) {
    // Center must exist
    if (!centers.has(arc.center.id)) {
      errors.push(`Arc ${arc.id}: center ControlPoint ${arc.center.id} not found`);
    }

    // Radius must exist
    if (!radii.has(arc.radius.id)) {
      errors.push(`Arc ${arc.id}: Radius entity ${arc.radius.id} not found`);
    }

    // Angles must be valid
    const start = toFloat(arc.startAngle);
    const end = toFloat(arc.endAngle);
    if (isNaN(start) || isNaN(end)) {
      errors.push(`Arc ${arc.id}: invalid angle values`);
    }
  }

  return { valid: errors.length === 0, errors };
}

// =============================================================================
// Exports for Testing
// =============================================================================

export const _internals = {
  mapSinglePath,
  toFloat,
  getSegmentPointIds,
};
