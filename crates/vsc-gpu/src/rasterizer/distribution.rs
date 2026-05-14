//! Subpixel Error Distribution via Largest Remainder Method (LRM)
//!
//! This module implements the Largest Remainder Method for distributing
//! subpixel rounding errors across child elements within a parent container.
//!
//! ## The Problem (Architect's Decision #2: Spatial Closure)
//!
//! Given a 100px container with 3 children of equal width (33.333...px each):
//!
//! ```text
//! Naive rounding:
//!   floor(33.333) = 33
//!   33 + 33 + 33 = 99px  <- 1px HOLE!
//! ```
//!
//! This violates VS axiom: "Constraint holes cannot be hidden in theoretical blind spots"
//!
//! ## Solution: Largest Remainder Method
//!
//! 1. Compute integer quotients: floor(33.333) = 33 for each
//! 2. Compute remainders: 0.333... for each
//! 3. Total shortfall: 100 - 99 = 1px
//! 4. Distribute 1px to elements with largest remainders
//!
//! Result: [34, 33, 33] or [33, 34, 33] or [33, 33, 34]
//! Sum: 100px exactly
//!
//! ## Mathematical Guarantee
//!
//! For any set of positive rationals r_1, r_2, ..., r_n where Sum(r_i) = T (integer):
//!
//!   Sum(floor(r_i)) <= T <= Sum(ceil(r_i))
//!
//! LRM distributes exactly (T - Sum(floor(r_i))) extra pixels to achieve Sum = T.
//!
//! ## Error Bounds
//!
//! The error for each element is bounded by (-1.0, 1.0), not [-0.5, +0.5] as
//! incorrectly stated in the original TypeScript implementation comments.
//!
//! Proof:
//! - LRM assigns each child either `floor(x)` or `floor(x) + 1` pixels
//! - For a child NOT receiving the extra pixel: `error = floor(x) - x ∈ (-1, 0]`
//! - For a child receiving the extra pixel: `error = floor(x) + 1 - x ∈ (0, 1]`
//! - Combined range: `(-1, 1)`
//!
//! The TS comment `[-0.5, +0.5]` was mathematically incorrect.

use std::collections::HashMap;
use vsc_core::EntityId;

use super::union_find::Axis;

/// A sibling group: children that must sum to parent's dimension.
#[derive(Debug, Clone)]
pub struct SiblingGroup {
    /// Parent container entity.
    pub parent_id: EntityId,

    /// Child entities in layout order.
    pub child_ids: Vec<EntityId>,

    /// Axis being distributed.
    pub axis: Axis,

    /// Parent's exact pixel dimension (must be integer).
    pub parent_dimension: i32,

    /// Each child's fractional dimension (pre-rounding).
    pub child_dimensions: HashMap<EntityId, f64>,
}

/// Distribution result for a single child.
#[derive(Debug, Clone, PartialEq)]
pub struct DistributedDimension {
    /// Entity identifier.
    pub entity_id: EntityId,

    /// Integer pixel dimension after distribution.
    pub pixels: i32,

    /// Original fractional value.
    pub original: f64,

    /// Error introduced by rounding (pixels - original).
    ///
    /// For LRM, this is bounded by:
    /// - If element gets extra pixel: error = 1 - remainder (positive, up to ~1.0)
    /// - If element doesn't get extra pixel: error = -remainder (negative, up to ~0)
    ///
    /// In practice, error is in range (-1.0, 1.0).
    pub error: f64,
}

impl DistributedDimension {
    /// Create a new distributed dimension with error range assertion.
    ///
    /// # Debug Assertion
    ///
    /// In debug builds, asserts that error is within (-1.0, 1.0).
    /// This is a fundamental invariant of the Largest Remainder Method:
    /// each element receives either floor(original) or floor(original) + 1 pixels.
    pub fn new(entity_id: EntityId, pixels: i32, original: f64) -> Self {
        let error = pixels as f64 - original;
        debug_assert!(
            error > -1.0 - 1e-9 && error < 1.0 + 1e-9,
            "LRM error out of range: {:.6} (expected (-1.0, 1.0)) for entity {:?}",
            error,
            entity_id
        );
        Self {
            entity_id,
            pixels,
            original,
            error,
        }
    }
}

/// Distribution method used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributionMethod {
    /// Largest Remainder Method (optimal for integer apportionment).
    LargestRemainder,
    /// First-fit proportional scaling (fallback for constraint violations).
    FirstFit,
}

/// Statistical summary of error distribution.
///
/// ## Phase 6: Observability Pipeline
///
/// These statistics enable monitoring of subpixel rounding quality:
/// - `max_error`: Worst-case single-element error (visual artifact risk)
/// - `stddev`: Distribution spread (uniformity indicator)
/// - `histogram`: Error frequency distribution for detailed analysis
#[derive(Debug, Clone, PartialEq)]
pub struct DistributionStats {
    /// Maximum absolute error across all children.
    pub max_error: f64,

    /// Standard deviation of errors.
    pub stddev: f64,

    /// Mean error (should be near 0 for well-balanced distribution).
    pub mean: f64,

    /// Histogram of errors in 10 bins from -0.5 to +0.5.
    pub histogram: Vec<HistogramBin>,
}

/// Single histogram bin.
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramBin {
    /// Lower bound (inclusive).
    pub lower: f64,
    /// Upper bound (exclusive, except for last bin).
    pub upper: f64,
    /// Number of elements in this bin.
    pub count: usize,
}

/// Complete distribution result.
#[derive(Debug, Clone)]
pub struct DistributionResult {
    /// Distributed dimensions for each child.
    pub dimensions: Vec<DistributedDimension>,

    /// Verification: sum of all pixels.
    pub total_pixels: i32,

    /// Should equal parent_dimension exactly.
    pub is_exact: bool,

    /// Distribution method used.
    pub method: DistributionMethod,

    /// Statistical summary of error distribution.
    pub stats: DistributionStats,
}

// =============================================================================
// Statistics Computation (Phase 6: Observability)
// =============================================================================

/// Compute distribution statistics in O(N) time.
///
/// ## Algorithm Complexity
///
/// Single pass computes: sum, sum_of_squares, max_abs_error, histogram bins.
/// - Mean: O(1) from sum
/// - Variance: O(1) using Sum(x^2) - (Sum(x))^2/n formula
/// - Stddev: O(1) sqrt of variance
/// - Histogram: O(1) per element (direct bin indexing)
///
/// Total: O(N) where N = number of children
fn compute_distribution_stats(dimensions: &[DistributedDimension]) -> DistributionStats {
    let n = dimensions.len();

    // Edge case: no elements
    if n == 0 {
        return DistributionStats {
            max_error: 0.0,
            stddev: 0.0,
            mean: 0.0,
            histogram: create_empty_histogram(),
        };
    }

    // Initialize histogram bins: [-0.5, -0.4), [-0.4, -0.3), ..., [0.4, 0.5]
    let mut bin_counts = [0usize; 10];

    // Single-pass accumulation
    let mut sum = 0.0;
    let mut sum_of_squares = 0.0;
    let mut max_abs_error = 0.0;

    for dim in dimensions {
        let err = dim.error;

        // Accumulate for mean/stddev
        sum += err;
        sum_of_squares += err * err;

        // Track max absolute error
        let abs_err = err.abs();
        if abs_err > max_abs_error {
            max_abs_error = abs_err;
        }

        // Histogram bin assignment
        // Bins: [-0.5, -0.4), [-0.4, -0.3), ..., [0.3, 0.4), [0.4, 0.5]
        // Clamp to [-0.5, 0.5] range
        let clamped_err = err.clamp(-0.5, 0.5);
        // Map [-0.5, 0.5] to [0, 10), with 0.5 mapping to bin 9
        let mut bin_index = ((clamped_err + 0.5) * 10.0).floor() as usize;
        if bin_index >= 10 {
            bin_index = 9;
        }
        bin_counts[bin_index] += 1;
    }

    // Compute mean
    let mean = sum / n as f64;

    // Compute stddev using: sigma = sqrt(E[X^2] - E[X]^2)
    // Variance = (Sum(x^2))/n - (Sum(x)/n)^2
    let variance = sum_of_squares / n as f64 - mean * mean;
    // Protect against floating-point errors causing negative variance
    let stddev = variance.max(0.0).sqrt();

    // Build histogram bins
    let histogram: Vec<HistogramBin> = bin_counts
        .iter()
        .enumerate()
        .map(|(i, &count)| HistogramBin {
            lower: -0.5 + i as f64 * 0.1,
            upper: -0.5 + (i + 1) as f64 * 0.1,
            count,
        })
        .collect();

    DistributionStats {
        max_error: max_abs_error,
        stddev,
        mean,
        histogram,
    }
}

/// Create an empty histogram (for edge cases).
fn create_empty_histogram() -> Vec<HistogramBin> {
    (0..10)
        .map(|i| HistogramBin {
            lower: -0.5 + i as f64 * 0.1,
            upper: -0.5 + (i + 1) as f64 * 0.1,
            count: 0,
        })
        .collect()
}

// =============================================================================
// Largest Remainder Method
// =============================================================================

/// Distribute subpixel errors using the Largest Remainder Method.
///
/// ## Algorithm
///
/// ```text
/// INPUT: child_dimensions = [33.333, 33.333, 33.333], parent_dimension = 100
///
/// Step 1: Compute floors
///   floors = [33, 33, 33]
///   sum(floors) = 99
///
/// Step 2: Compute remainders
///   remainders = [0.333, 0.333, 0.333]
///
/// Step 3: Compute shortfall
///   shortfall = 100 - 99 = 1
///
/// Step 4: Sort by remainder (descending), distribute shortfall
///   Give 1px to first element (arbitrary tie-break: leftmost)
///
/// OUTPUT: [34, 33, 33]
/// ```
///
/// ## Tie-Breaking Strategy
///
/// When remainders are equal (common for equal-width elements):
/// - Distribute extra pixels left-to-right (reading order)
/// - This is visually predictable and matches user expectation
pub fn distribute_with_largest_remainder(group: &SiblingGroup) -> DistributionResult {
    let SiblingGroup {
        child_ids,
        parent_dimension,
        child_dimensions,
        ..
    } = group;

    // Edge case: no children
    if child_ids.is_empty() {
        return DistributionResult {
            dimensions: vec![],
            total_pixels: 0,
            is_exact: *parent_dimension == 0,
            method: DistributionMethod::LargestRemainder,
            stats: compute_distribution_stats(&[]),
        };
    }

    // Step 1: Compute floors and remainders
    struct WorkItem {
        entity_id: EntityId,
        original: f64,
        floor: i32,
        remainder: f64,
        index: usize, // Original order for tie-breaking
    }

    let items: Vec<WorkItem> = child_ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let original = *child_dimensions.get(id).unwrap_or(&0.0);
            let floor = original.floor() as i32;
            WorkItem {
                entity_id: *id,
                original,
                floor,
                remainder: original - floor as f64,
                index,
            }
        })
        .collect();

    // Step 2: Compute shortfall
    let sum_of_floors: i32 = items.iter().map(|item| item.floor).sum();
    let shortfall = parent_dimension - sum_of_floors;

    // Sanity check: shortfall should be non-negative and <= child_count
    if shortfall < 0 || shortfall as usize > items.len() {
        // This indicates a constraint violation (children sum > parent)
        // Fall back to proportional scaling
        return distribute_proportionally(group);
    }

    // Step 3: Sort by remainder (descending), then by index (ascending) for tie-break
    let mut sorted: Vec<&WorkItem> = items.iter().collect();
    sorted.sort_by(|a, b| {
        let remainder_cmp = b
            .remainder
            .partial_cmp(&a.remainder)
            .unwrap_or(std::cmp::Ordering::Equal);
        if remainder_cmp != std::cmp::Ordering::Equal {
            return remainder_cmp;
        }
        // Tie-break: leftmost first
        a.index.cmp(&b.index)
    });

    // Step 4: Distribute shortfall to top N elements
    let mut extra_pixels: std::collections::HashSet<EntityId> = std::collections::HashSet::new();
    for i in 0..(shortfall as usize) {
        extra_pixels.insert(sorted[i].entity_id);
    }

    // Step 5: Build result
    let dimensions: Vec<DistributedDimension> = items
        .iter()
        .map(|item| {
            let pixels = item.floor
                + if extra_pixels.contains(&item.entity_id) {
                    1
                } else {
                    0
                };
            DistributedDimension::new(item.entity_id, pixels, item.original)
        })
        .collect();

    let total_pixels: i32 = dimensions.iter().map(|d| d.pixels).sum();
    let stats = compute_distribution_stats(&dimensions);

    DistributionResult {
        dimensions,
        total_pixels,
        is_exact: total_pixels == *parent_dimension,
        method: DistributionMethod::LargestRemainder,
        stats,
    }
}

/// Fallback: Proportional scaling when LRM fails.
///
/// Used when child sum exceeds parent (constraint violation).
fn distribute_proportionally(group: &SiblingGroup) -> DistributionResult {
    let SiblingGroup {
        child_ids,
        parent_dimension,
        child_dimensions,
        ..
    } = group;

    let total_original: f64 = child_ids
        .iter()
        .map(|id| *child_dimensions.get(id).unwrap_or(&0.0))
        .sum();

    if total_original == 0.0 {
        let dimensions: Vec<DistributedDimension> = child_ids
            .iter()
            .map(|&id| DistributedDimension::new(id, 0, 0.0))
            .collect();
        return DistributionResult {
            dimensions: dimensions.clone(),
            total_pixels: 0,
            is_exact: *parent_dimension == 0,
            method: DistributionMethod::FirstFit,
            stats: compute_distribution_stats(&dimensions),
        };
    }

    let scale = *parent_dimension as f64 / total_original;

    // Create a scaled group and apply LRM
    let scaled_dimensions: HashMap<EntityId, f64> = child_ids
        .iter()
        .map(|id| {
            let original = *child_dimensions.get(id).unwrap_or(&0.0);
            (*id, original * scale)
        })
        .collect();

    let scaled_group = SiblingGroup {
        parent_id: group.parent_id,
        child_ids: child_ids.clone(),
        axis: group.axis,
        parent_dimension: *parent_dimension,
        child_dimensions: scaled_dimensions,
    };

    let mut result = distribute_with_largest_remainder(&scaled_group);
    result.method = DistributionMethod::FirstFit;
    result
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_group(dimensions: &[f64], parent_dimension: i32) -> SiblingGroup {
        let child_ids: Vec<EntityId> = (0..dimensions.len()).map(|i| EntityId(i as u64)).collect();
        let child_dimensions: HashMap<EntityId, f64> = child_ids
            .iter()
            .zip(dimensions.iter())
            .map(|(&id, &dim)| (id, dim))
            .collect();

        SiblingGroup {
            parent_id: EntityId(999),
            child_ids,
            axis: Axis::Horizontal,
            parent_dimension,
            child_dimensions,
        }
    }

    #[test]
    fn test_lrm_three_equal_children() {
        // Classic case: 100px / 3 = 33.333...
        let group = make_group(&[100.0 / 3.0, 100.0 / 3.0, 100.0 / 3.0], 100);
        let result = distribute_with_largest_remainder(&group);

        assert!(result.is_exact);
        assert_eq!(result.total_pixels, 100);
        assert_eq!(result.method, DistributionMethod::LargestRemainder);

        // All children should get [34, 33, 33] or similar
        let pixels: Vec<i32> = result.dimensions.iter().map(|d| d.pixels).collect();
        assert_eq!(pixels.iter().sum::<i32>(), 100);

        // One should be 34, two should be 33
        let count_34 = pixels.iter().filter(|&&p| p == 34).count();
        let count_33 = pixels.iter().filter(|&&p| p == 33).count();
        assert_eq!(count_34, 1);
        assert_eq!(count_33, 2);
    }

    #[test]
    fn test_lrm_error_range() {
        // Verify all errors are within (-1.0, 1.0)
        // LRM guarantees each element gets floor(original) or floor(original) + 1
        let group = make_group(&[33.333, 33.333, 33.334], 100);
        let result = distribute_with_largest_remainder(&group);

        for dim in &result.dimensions {
            assert!(
                dim.error > -1.0 - 1e-9 && dim.error < 1.0 + 1e-9,
                "Error {} out of range for entity {:?}",
                dim.error,
                dim.entity_id
            );
        }
    }

    #[test]
    fn test_lrm_exact_fit() {
        // Children sum to exactly parent
        let group = make_group(&[25.0, 25.0, 25.0, 25.0], 100);
        let result = distribute_with_largest_remainder(&group);

        assert!(result.is_exact);
        assert_eq!(result.total_pixels, 100);

        let pixels: Vec<i32> = result.dimensions.iter().map(|d| d.pixels).collect();
        assert_eq!(pixels, vec![25, 25, 25, 25]);
    }

    #[test]
    fn test_lrm_unequal_remainders() {
        // 10.7, 10.2, 10.1 -> floors are 10, 10, 10 = 30
        // parent = 31, shortfall = 1
        // Extra pixel goes to 10.7 (largest remainder)
        let group = make_group(&[10.7, 10.2, 10.1], 31);
        let result = distribute_with_largest_remainder(&group);

        assert!(result.is_exact);
        let pixels: Vec<i32> = result.dimensions.iter().map(|d| d.pixels).collect();
        assert_eq!(pixels[0], 11); // 10.7 gets the extra pixel
        assert_eq!(pixels[1], 10);
        assert_eq!(pixels[2], 10);
    }

    #[test]
    fn test_lrm_tie_breaking() {
        // All equal remainders: tie-break by index (leftmost first)
        let group = make_group(&[10.5, 10.5], 22);
        let result = distribute_with_largest_remainder(&group);

        assert!(result.is_exact);
        let pixels: Vec<i32> = result.dimensions.iter().map(|d| d.pixels).collect();
        // shortfall = 22 - 20 = 2, so both get 1 extra
        assert_eq!(pixels, vec![11, 11]);
    }

    #[test]
    fn test_lrm_empty_group() {
        let group = SiblingGroup {
            parent_id: EntityId(999),
            child_ids: vec![],
            axis: Axis::Horizontal,
            parent_dimension: 0,
            child_dimensions: HashMap::new(),
        };
        let result = distribute_with_largest_remainder(&group);

        assert!(result.dimensions.is_empty());
        assert_eq!(result.total_pixels, 0);
        assert!(result.is_exact);
    }

    #[test]
    fn test_proportional_fallback() {
        // Children sum > parent (constraint violation)
        let group = make_group(&[60.0, 60.0], 100);
        let result = distribute_with_largest_remainder(&group);

        // Should fall back to proportional scaling
        assert_eq!(result.method, DistributionMethod::FirstFit);
        assert_eq!(result.total_pixels, 100);
        // 60/(120) * 100 = 50 each
        let pixels: Vec<i32> = result.dimensions.iter().map(|d| d.pixels).collect();
        assert_eq!(pixels, vec![50, 50]);
    }

    #[test]
    fn test_stats_computation() {
        let group = make_group(&[33.333, 33.333, 33.334], 100);
        let result = distribute_with_largest_remainder(&group);

        // Stats should be computed
        assert!(result.stats.max_error >= 0.0);
        assert!(result.stats.stddev >= 0.0);
        assert_eq!(result.stats.histogram.len(), 10);
    }

    /// Task 1: parent_dimension = 0 must not produce a zero-division panic.
    ///
    /// When parent_dimension is 0 and children have 0-valued dimensions the
    /// LRM path returns cleanly.  When children have positive dimensions the
    /// proportional-fallback path is exercised; that path divides by
    /// total_original, not by parent_dimension, so it must also be safe.
    #[test]
    fn test_lrm_parent_dimension_zero_no_division_by_zero() {
        // Case A: all children are also zero → pure LRM path
        let group_zero = make_group(&[0.0, 0.0, 0.0], 0);
        let result_zero = distribute_with_largest_remainder(&group_zero);
        assert_eq!(result_zero.total_pixels, 0);
        assert!(result_zero.is_exact);

        // Case B: children sum > 0 but parent = 0 → proportional fallback is triggered.
        // distribute_proportionally divides by total_original (positive), not parent_dimension,
        // so there must be no division-by-zero panic.
        let group_positive = make_group(&[10.0, 20.0], 0);
        let result_positive = distribute_with_largest_remainder(&group_positive);
        // Proportional fallback scales everything to fit 0px → all dimensions become 0.
        assert_eq!(result_positive.total_pixels, 0);
    }

    #[test]
    fn test_histogram_bins() {
        // All zeros should go to the middle-ish bin
        let dimensions = vec![
            DistributedDimension::new(EntityId(0), 10, 10.0), // error = 0
            DistributedDimension::new(EntityId(1), 10, 10.0), // error = 0
        ];
        let stats = compute_distribution_stats(&dimensions);

        // Error of 0 should be in bin 5 (centered around 0)
        // Bin 5 is [-0.0, 0.1), which contains 0
        let zero_bin_count: usize = stats
            .histogram
            .iter()
            .filter(|b| b.lower <= 0.0 && 0.0 < b.upper)
            .map(|b| b.count)
            .sum();
        assert_eq!(zero_bin_count, 2);
    }
}
