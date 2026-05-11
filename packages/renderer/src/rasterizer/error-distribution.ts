/**
 * Subpixel Error Distribution Algorithm
 *
 * This module implements the Largest Remainder Method (LRM) for distributing
 * subpixel rounding errors across child elements within a parent container.
 *
 * ## The Problem (Architect's Decision #2: Spatial Closure)
 *
 * Given a 100px container with 3 children of equal width (33.333...px each):
 *
 * ```
 * Naive rounding:
 *   floor(33.333) = 33
 *   33 + 33 + 33 = 99px  ← 1px HOLE!
 * ```
 *
 * This violates VS axiom: "Constraint holes cannot be hidden in theoretical blind spots"
 *
 * ## Solution: Largest Remainder Method
 *
 * 1. Compute integer quotients: floor(33.333) = 33 for each
 * 2. Compute remainders: 0.333... for each
 * 3. Total shortfall: 100 - 99 = 1px
 * 4. Distribute 1px to elements with largest remainders
 *
 * Result: [34, 33, 33] or [33, 34, 33] or [33, 33, 34]
 * Sum: 100px exactly ✓
 *
 * ## Mathematical Guarantee
 *
 * For any set of positive rationals r₁, r₂, ..., rₙ where Σrᵢ = T (integer):
 *
 *   Σ⌊rᵢ⌋ ≤ T ≤ Σ⌈rᵢ⌉
 *
 * LRM distributes exactly (T - Σ⌊rᵢ⌋) extra pixels to achieve Σ = T.
 */

import type { EntityId, Rational } from '../ast/types';

// =============================================================================
// Types
// =============================================================================

/**
 * A sibling group: children that must sum to parent's dimension.
 */
export interface SiblingGroup {
  /** Parent container entity */
  parentId: EntityId;

  /** Child entities in layout order */
  childIds: EntityId[];

  /** Axis being distributed */
  axis: 'horizontal' | 'vertical';

  /** Parent's exact pixel dimension (must be integer) */
  parentDimension: number;

  /** Each child's rational dimension (pre-rounding) */
  childDimensions: Map<EntityId, number>;
}

/**
 * Distribution result for a single child.
 */
export interface DistributedDimension {
  entityId: EntityId;

  /** Integer pixel dimension after distribution */
  pixels: number;

  /** Original fractional value */
  original: number;

  /** Error introduced by rounding (-0.5 to +0.5) */
  error: number;
}

/**
 * Complete distribution result.
 */
export interface DistributionResult {
  /** Distributed dimensions for each child */
  dimensions: DistributedDimension[];

  /** Verification: sum of all pixels */
  totalPixels: number;

  /** Should equal parentDimension exactly */
  isExact: boolean;

  /** Distribution method used */
  method: 'largest-remainder' | 'first-fit';
}

// =============================================================================
// Largest Remainder Method
// =============================================================================

/**
 * Distribute subpixel errors using the Largest Remainder Method.
 *
 * ## Algorithm
 *
 * ```
 * INPUT: childDimensions = [33.333, 33.333, 33.333], parentDimension = 100
 *
 * Step 1: Compute floors
 *   floors = [33, 33, 33]
 *   sum(floors) = 99
 *
 * Step 2: Compute remainders
 *   remainders = [0.333, 0.333, 0.333]
 *
 * Step 3: Compute shortfall
 *   shortfall = 100 - 99 = 1
 *
 * Step 4: Sort by remainder (descending), distribute shortfall
 *   Give 1px to first element (arbitrary tie-break: leftmost)
 *
 * OUTPUT: [34, 33, 33]
 * ```
 *
 * ## Tie-Breaking Strategy
 *
 * When remainders are equal (common for equal-width elements):
 * - Distribute extra pixels left-to-right (reading order)
 * - This is visually predictable and matches user expectation
 */
export function distributeWithLargestRemainder(group: SiblingGroup): DistributionResult {
  const { childIds, parentDimension, childDimensions } = group;

  // Edge case: no children
  if (childIds.length === 0) {
    return {
      dimensions: [],
      totalPixels: 0,
      isExact: parentDimension === 0,
      method: 'largest-remainder',
    };
  }

  // Step 1: Compute floors and remainders
  interface WorkItem {
    entityId: EntityId;
    original: number;
    floor: number;
    remainder: number;
    index: number; // Original order for tie-breaking
  }

  const items: WorkItem[] = childIds.map((id, index) => {
    const original = childDimensions.get(id) ?? 0;
    const floor = Math.floor(original);
    return {
      entityId: id,
      original,
      floor,
      remainder: original - floor,
      index,
    };
  });

  // Step 2: Compute shortfall
  const sumOfFloors = items.reduce((sum, item) => sum + item.floor, 0);
  const shortfall = parentDimension - sumOfFloors;

  // Sanity check: shortfall should be non-negative and <= childCount
  if (shortfall < 0 || shortfall > items.length) {
    // This indicates a constraint violation (children sum > parent)
    // Fall back to proportional scaling
    return distributeProportionally(group);
  }

  // Step 3: Sort by remainder (descending), then by index (ascending) for tie-break
  const sorted = [...items].sort((a, b) => {
    const remainderDiff = b.remainder - a.remainder;
    if (Math.abs(remainderDiff) > 1e-10) {
      return remainderDiff;
    }
    // Tie-break: leftmost first
    return a.index - b.index;
  });

  // Step 4: Distribute shortfall to top N elements
  const extraPixels = new Set<EntityId>();
  for (let i = 0; i < shortfall; i++) {
    extraPixels.add(sorted[i].entityId);
  }

  // Step 5: Build result
  const dimensions: DistributedDimension[] = items.map(item => {
    const pixels = item.floor + (extraPixels.has(item.entityId) ? 1 : 0);
    return {
      entityId: item.entityId,
      pixels,
      original: item.original,
      error: pixels - item.original,
    };
  });

  const totalPixels = dimensions.reduce((sum, d) => sum + d.pixels, 0);

  return {
    dimensions,
    totalPixels,
    isExact: totalPixels === parentDimension,
    method: 'largest-remainder',
  };
}

/**
 * Fallback: Proportional scaling when LRM fails.
 *
 * Used when child sum exceeds parent (constraint violation).
 */
function distributeProportionally(group: SiblingGroup): DistributionResult {
  const { childIds, parentDimension, childDimensions } = group;

  const totalOriginal = childIds.reduce(
    (sum, id) => sum + (childDimensions.get(id) ?? 0),
    0
  );

  if (totalOriginal === 0) {
    return {
      dimensions: childIds.map(id => ({
        entityId: id,
        pixels: 0,
        original: 0,
        error: 0,
      })),
      totalPixels: 0,
      isExact: parentDimension === 0,
      method: 'first-fit',
    };
  }

  const scale = parentDimension / totalOriginal;
  const scaled = childIds.map(id => {
    const original = childDimensions.get(id) ?? 0;
    return {
      entityId: id,
      original,
      scaled: original * scale,
    };
  });

  // Apply LRM to scaled values
  const scaledGroup: SiblingGroup = {
    ...group,
    childDimensions: new Map(scaled.map(s => [s.entityId, s.scaled])),
  };

  const result = distributeWithLargestRemainder(scaledGroup);
  result.method = 'first-fit';
  return result;
}

// =============================================================================
// Integration with Topology Rounding
// =============================================================================

/**
 * Parent-child containment constraint for error distribution.
 */
export interface ContainmentConstraint {
  parentId: EntityId;
  childIds: EntityId[];
  axis: 'horizontal' | 'vertical';
}

/**
 * Apply error distribution to a set of sibling groups.
 *
 * This is called AFTER basic topology-preserving rounding,
 * to ensure parent boundaries are exactly satisfied.
 */
export function applyErrorDistribution(
  roundedBounds: Map<EntityId, { x: number; y: number; width: number; height: number }>,
  containments: ContainmentConstraint[],
): Map<EntityId, { x: number; y: number; width: number; height: number }> {
  const result = new Map(roundedBounds);

  for (const constraint of containments) {
    const parentBounds = result.get(constraint.parentId);
    if (!parentBounds) continue;

    const parentDimension = constraint.axis === 'horizontal'
      ? parentBounds.width
      : parentBounds.height;

    // Build sibling group
    const childDimensions = new Map<EntityId, number>();
    for (const childId of constraint.childIds) {
      const childBounds = result.get(childId);
      if (childBounds) {
        const dim = constraint.axis === 'horizontal'
          ? childBounds.width
          : childBounds.height;
        childDimensions.set(childId, dim);
      }
    }

    const group: SiblingGroup = {
      parentId: constraint.parentId,
      childIds: constraint.childIds,
      axis: constraint.axis,
      parentDimension,
      childDimensions,
    };

    // Distribute
    const distribution = distributeWithLargestRemainder(group);

    // Apply distributed dimensions
    let offset = constraint.axis === 'horizontal' ? parentBounds.x : parentBounds.y;

    for (const dist of distribution.dimensions) {
      const childBounds = result.get(dist.entityId);
      if (childBounds) {
        if (constraint.axis === 'horizontal') {
          childBounds.x = offset;
          childBounds.width = dist.pixels;
        } else {
          childBounds.y = offset;
          childBounds.height = dist.pixels;
        }
        offset += dist.pixels;
      }
    }
  }

  return result;
}

// =============================================================================
// Exports for Testing
// =============================================================================

export const _internals = {
  distributeProportionally,
};
