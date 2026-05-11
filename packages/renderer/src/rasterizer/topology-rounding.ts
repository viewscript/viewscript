/**
 * Topology-Preserving Rounding Algorithm
 *
 * This module implements the rasterization layer that projects P-dimension
 * rational coordinates to discrete pixel coordinates while preserving
 * topological relationships (adjacency, containment, ordering).
 *
 * ## The Problem
 *
 * Given two adjacent surfaces A and B where:
 *   A.right = 100.333... (rational)
 *   B.left = 100.333... (same rational)
 *
 * Naive rounding may produce:
 *   A.right = 100px (floor)
 *   B.left = 101px (ceil)
 *
 * This creates a 1px gap that violates the topological constraint
 * that A and B are adjacent (no gap, no overlap).
 *
 * ## Solution: Constraint-Aware Rounding
 *
 * Instead of rounding each coordinate independently, we:
 * 1. Build a graph of topological relationships (adjacency, containment)
 * 2. Partition coordinates into equivalence classes (same rational = same pixel)
 * 3. Round equivalence classes together
 * 4. Propagate rounding decisions through the constraint graph
 *
 * ## Algorithm
 *
 * ```
 * INPUT:
 *   - Set of surfaces S with rational bounds
 *   - Topological constraints T (adjacency, containment)
 *   - Device pixel ratio DPR
 *
 * OUTPUT:
 *   - Integer pixel coordinates for all surfaces
 *   - Guarantee: topology is preserved
 *
 * ALGORITHM:
 *
 * Phase 1: Build Coordinate Equivalence Classes
 *   For each unique rational value r:
 *     equiv[r] = { all coordinates that equal r }
 *
 * Phase 2: Compute Rounding Constraints
 *   For each adjacency constraint (A.right = B.left):
 *     round(A.right) MUST equal round(B.left)
 *   For each ordering constraint (A.right < B.left):
 *     round(A.right) MUST be < round(B.left)
 *
 * Phase 3: Propagate Rounding Decisions
 *   Using constraint propagation:
 *   - Start with coordinates that have no constraints (free variables)
 *   - Round them to nearest integer
 *   - Propagate to constrained coordinates
 *   - Resolve conflicts by adjusting adjacent surfaces symmetrically
 *
 * Phase 4: Verify Topology Preservation
 *   Assert all topological constraints are satisfied
 * ```
 */

import type { EntityId, Rational, RasterBounds, PVectorBounds } from '../ast/types';

// =============================================================================
// Types
// =============================================================================

/**
 * A coordinate in the pre-rasterization space.
 */
interface RationalCoord {
  entityId: EntityId;
  edge: 'left' | 'right' | 'top' | 'bottom';
  value: Rational;
}

/**
 * Topological constraint between coordinates.
 */
type TopoConstraint =
  | { type: 'equal'; a: CoordRef; b: CoordRef }      // A and B must round to same pixel
  | { type: 'less-than'; a: CoordRef; b: CoordRef }  // A must round to less than B
  | { type: 'adjacent'; a: CoordRef; b: CoordRef };  // A.right touches B.left (no gap, no overlap)

interface CoordRef {
  entityId: EntityId;
  edge: 'left' | 'right' | 'top' | 'bottom';
}

/**
 * Result of the rounding algorithm.
 */
export interface RoundingResult {
  /** Rasterized bounds for each entity */
  bounds: Map<EntityId, RasterBounds>;

  /** Any topology violations detected (should be empty if algorithm is correct) */
  violations: TopologyViolation[];

  /** Statistics about the rounding process */
  stats: RoundingStats;
}

interface TopologyViolation {
  constraint: TopoConstraint;
  message: string;
}

interface RoundingStats {
  totalCoordinates: number;
  equivalenceClasses: number;
  constraintsPropagated: number;
  conflictsResolved: number;
}

// =============================================================================
// Core Algorithm
// =============================================================================

/**
 * Topology-preserving rounding entry point.
 */
export function roundWithTopologyPreservation(
  entities: Map<EntityId, PVectorBounds>,
  constraints: TopoConstraint[],
  devicePixelRatio: number,
): RoundingResult {
  const stats: RoundingStats = {
    totalCoordinates: 0,
    equivalenceClasses: 0,
    constraintsPropagated: 0,
    conflictsResolved: 0,
  };

  // Phase 1: Extract all coordinates and build equivalence classes
  const coords = extractCoordinates(entities);
  stats.totalCoordinates = coords.length;

  const equivClasses = buildEquivalenceClasses(coords, constraints);
  stats.equivalenceClasses = equivClasses.size;

  // Phase 2: Compute rounding for each equivalence class
  const roundedClasses = new Map<string, number>();

  for (const [classId, members] of equivClasses) {
    // All members have the same rational value
    const rationalValue = members[0].value;
    const floatValue = rationalToFloat(rationalValue) * devicePixelRatio;

    // Default: round to nearest
    roundedClasses.set(classId, Math.round(floatValue));
  }

  // Phase 3: Propagate constraints and resolve conflicts
  const { adjusted, conflictsResolved } = propagateConstraints(
    roundedClasses,
    equivClasses,
    constraints,
  );
  stats.constraintsPropagated = constraints.length;
  stats.conflictsResolved = conflictsResolved;

  // Phase 4: Build final bounds
  const bounds = buildFinalBounds(entities, adjusted, equivClasses, devicePixelRatio);

  // Phase 5: Verify topology
  const violations = verifyTopology(bounds, constraints);

  return { bounds, violations, stats };
}

// =============================================================================
// Phase 1: Coordinate Extraction and Equivalence Classes
// =============================================================================

function extractCoordinates(entities: Map<EntityId, PVectorBounds>): RationalCoord[] {
  const coords: RationalCoord[] = [];

  for (const [entityId, bounds] of entities) {
    coords.push(
      { entityId, edge: 'left', value: bounds.topLeft.x },
      { entityId, edge: 'right', value: bounds.bottomRight.x },
      { entityId, edge: 'top', value: bounds.topLeft.y },
      { entityId, edge: 'bottom', value: bounds.bottomRight.y },
    );
  }

  return coords;
}

/**
 * Build equivalence classes from coordinates and equality constraints.
 *
 * Two coordinates are in the same class if:
 * 1. They have the same rational value, OR
 * 2. They are connected by an 'equal' or 'adjacent' constraint
 */
function buildEquivalenceClasses(
  coords: RationalCoord[],
  constraints: TopoConstraint[],
): Map<string, RationalCoord[]> {
  // Union-Find data structure
  const parent = new Map<string, string>();

  const coordKey = (c: CoordRef): string => `${c.entityId}:${c.edge}`;

  const find = (key: string): string => {
    if (!parent.has(key)) {
      parent.set(key, key);
      return key;
    }
    if (parent.get(key) !== key) {
      parent.set(key, find(parent.get(key)!));
    }
    return parent.get(key)!;
  };

  const union = (a: string, b: string): void => {
    const rootA = find(a);
    const rootB = find(b);
    if (rootA !== rootB) {
      parent.set(rootA, rootB);
    }
  };

  // Initialize each coord as its own class
  for (const coord of coords) {
    const key = `${coord.entityId}:${coord.edge}`;
    parent.set(key, key);
  }

  // Union coordinates with same rational value
  const byValue = new Map<string, RationalCoord[]>();
  for (const coord of coords) {
    const valKey = rationalKey(coord.value);
    if (!byValue.has(valKey)) {
      byValue.set(valKey, []);
    }
    byValue.get(valKey)!.push(coord);
  }

  for (const [, group] of byValue) {
    if (group.length > 1) {
      const first = coordKey({ entityId: group[0].entityId, edge: group[0].edge });
      for (let i = 1; i < group.length; i++) {
        union(first, coordKey({ entityId: group[i].entityId, edge: group[i].edge }));
      }
    }
  }

  // Union by equality/adjacency constraints
  for (const c of constraints) {
    if (c.type === 'equal' || c.type === 'adjacent') {
      union(coordKey(c.a), coordKey(c.b));
    }
  }

  // Build final classes
  const classes = new Map<string, RationalCoord[]>();
  for (const coord of coords) {
    const key = `${coord.entityId}:${coord.edge}`;
    const root = find(key);
    if (!classes.has(root)) {
      classes.set(root, []);
    }
    classes.get(root)!.push(coord);
  }

  return classes;
}

function rationalKey(r: Rational): string {
  // Normalize to lowest terms for consistent keying
  const gcd = bigIntGcd(r.numerator < 0n ? -r.numerator : r.numerator, r.denominator);
  const num = r.numerator / gcd;
  const den = r.denominator / gcd;
  return `${num}/${den}`;
}

function bigIntGcd(a: bigint, b: bigint): bigint {
  while (b !== 0n) {
    const t = b;
    b = a % b;
    a = t;
  }
  return a;
}

function rationalToFloat(r: Rational): number {
  return Number(r.numerator) / Number(r.denominator);
}

// =============================================================================
// Phase 3: Constraint Propagation
// =============================================================================

interface PropagationResult {
  adjusted: Map<string, number>;
  conflictsResolved: number;
}

/**
 * Propagate rounding decisions through less-than constraints.
 *
 * If A < B in rational space, we must ensure round(A) < round(B) in pixel space.
 * If rounding would violate this, we adjust by:
 * 1. Decreasing A by 1, OR
 * 2. Increasing B by 1
 *
 * We choose the option that minimizes total visual shift.
 */
function propagateConstraints(
  initial: Map<string, number>,
  equivClasses: Map<string, RationalCoord[]>,
  constraints: TopoConstraint[],
): PropagationResult {
  const adjusted = new Map(initial);
  let conflictsResolved = 0;

  // Build class lookup
  const coordToClass = new Map<string, string>();
  for (const [classId, members] of equivClasses) {
    for (const m of members) {
      coordToClass.set(`${m.entityId}:${m.edge}`, classId);
    }
  }

  // Process less-than constraints
  for (const c of constraints) {
    if (c.type !== 'less-than') continue;

    const classA = coordToClass.get(`${c.a.entityId}:${c.a.edge}`);
    const classB = coordToClass.get(`${c.b.entityId}:${c.b.edge}`);

    if (!classA || !classB) continue;

    const valA = adjusted.get(classA) ?? 0;
    const valB = adjusted.get(classB) ?? 0;

    // Must satisfy: valA < valB
    if (valA >= valB) {
      // Conflict! Need to adjust.
      // Strategy: Create a gap of 1px

      // Option 1: Decrease A
      const costDecreaseA = computeAdjustmentCost(classA, valA, valA - 1, equivClasses);

      // Option 2: Increase B
      const costIncreaseB = computeAdjustmentCost(classB, valB, valB + 1, equivClasses);

      if (costDecreaseA <= costIncreaseB) {
        adjusted.set(classA, valB - 1);
      } else {
        adjusted.set(classB, valA + 1);
      }

      conflictsResolved++;
    }
  }

  return { adjusted, conflictsResolved };
}

/**
 * Compute the visual cost of adjusting a coordinate.
 *
 * Cost is proportional to:
 * - Number of entities affected
 * - Distance of adjustment
 */
function computeAdjustmentCost(
  classId: string,
  from: number,
  to: number,
  equivClasses: Map<string, RationalCoord[]>,
): number {
  const members = equivClasses.get(classId) ?? [];
  const distance = Math.abs(to - from);
  return members.length * distance;
}

// =============================================================================
// Phase 4: Build Final Bounds
// =============================================================================

function buildFinalBounds(
  entities: Map<EntityId, PVectorBounds>,
  roundedClasses: Map<string, number>,
  equivClasses: Map<string, RationalCoord[]>,
  devicePixelRatio: number,
): Map<EntityId, RasterBounds> {
  // Build coord to class lookup
  const coordToClass = new Map<string, string>();
  for (const [classId, members] of equivClasses) {
    for (const m of members) {
      coordToClass.set(`${m.entityId}:${m.edge}`, classId);
    }
  }

  const bounds = new Map<EntityId, RasterBounds>();

  for (const [entityId] of entities) {
    const leftClass = coordToClass.get(`${entityId}:left`);
    const rightClass = coordToClass.get(`${entityId}:right`);
    const topClass = coordToClass.get(`${entityId}:top`);
    const bottomClass = coordToClass.get(`${entityId}:bottom`);

    const left = roundedClasses.get(leftClass!) ?? 0;
    const right = roundedClasses.get(rightClass!) ?? 0;
    const top = roundedClasses.get(topClass!) ?? 0;
    const bottom = roundedClasses.get(bottomClass!) ?? 0;

    // Convert from device pixels to CSS pixels
    bounds.set(entityId, {
      x: left / devicePixelRatio,
      y: top / devicePixelRatio,
      width: (right - left) / devicePixelRatio,
      height: (bottom - top) / devicePixelRatio,
    });
  }

  return bounds;
}

// =============================================================================
// Phase 5: Topology Verification
// =============================================================================

function verifyTopology(
  bounds: Map<EntityId, RasterBounds>,
  constraints: TopoConstraint[],
): TopologyViolation[] {
  const violations: TopologyViolation[] = [];

  for (const c of constraints) {
    const boundsA = bounds.get(c.a.entityId);
    const boundsB = bounds.get(c.b.entityId);

    if (!boundsA || !boundsB) continue;

    const valA = getEdgeValue(boundsA, c.a.edge);
    const valB = getEdgeValue(boundsB, c.b.edge);

    switch (c.type) {
      case 'equal':
        if (valA !== valB) {
          violations.push({
            constraint: c,
            message: `Equal constraint violated: ${valA} !== ${valB}`,
          });
        }
        break;

      case 'adjacent':
        if (valA !== valB) {
          violations.push({
            constraint: c,
            message: `Adjacent constraint violated: ${valA} !== ${valB} (gap or overlap)`,
          });
        }
        break;

      case 'less-than':
        if (valA >= valB) {
          violations.push({
            constraint: c,
            message: `Less-than constraint violated: ${valA} >= ${valB}`,
          });
        }
        break;
    }
  }

  return violations;
}

function getEdgeValue(bounds: RasterBounds, edge: 'left' | 'right' | 'top' | 'bottom'): number {
  switch (edge) {
    case 'left': return bounds.x;
    case 'right': return bounds.x + bounds.width;
    case 'top': return bounds.y;
    case 'bottom': return bounds.y + bounds.height;
  }
}

// =============================================================================
// Exports for Testing
// =============================================================================

export const _internals = {
  extractCoordinates,
  buildEquivalenceClasses,
  propagateConstraints,
  verifyTopology,
  rationalToFloat,
};
