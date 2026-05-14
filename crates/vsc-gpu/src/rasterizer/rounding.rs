//! Topology-Preserving Rounding Algorithm
//!
//! This module implements the rasterization layer that projects P-dimension
//! rational coordinates to discrete pixel coordinates while preserving
//! topological relationships (adjacency, containment, ordering).
//!
//! ## The Problem
//!
//! Given two adjacent surfaces A and B where:
//!   A.right = 100.333... (rational)
//!   B.left = 100.333... (same rational)
//!
//! Naive rounding may produce:
//!   A.right = 100px (floor)
//!   B.left = 101px (ceil)
//!
//! This creates a 1px gap that violates the topological constraint
//! that A and B are adjacent (no gap, no overlap).
//!
//! ## Solution: Constraint-Aware Rounding
//!
//! Instead of rounding each coordinate independently, we:
//! 1. Build a graph of topological relationships (adjacency, containment)
//! 2. Partition coordinates into equivalence classes (same rational = same pixel)
//! 3. Round equivalence classes together
//! 4. Propagate rounding decisions through the constraint graph
//!
//! ## Algorithm
//!
//! ```text
//! INPUT:
//!   - Set of surfaces S with rational bounds
//!   - Topological constraints T (adjacency, containment)
//!   - Device pixel ratio DPR
//!
//! OUTPUT:
//!   - Integer pixel coordinates for all surfaces
//!   - Guarantee: topology is preserved
//!
//! ALGORITHM:
//!
//! Phase 1: Build Coordinate Equivalence Classes
//!   For each unique rational value r:
//!     equiv[r] = { all coordinates that equal r }
//!
//! Phase 2: Compute Rounding Constraints
//!   For each adjacency constraint (A.right = B.left):
//!     round(A.right) MUST equal round(B.left)
//!   For each ordering constraint (A.right < B.left):
//!     round(A.right) MUST be < round(B.left)
//!
//! Phase 3: Propagate Rounding Decisions
//!   Using constraint propagation:
//!   - Start with coordinates that have no constraints (free variables)
//!   - Round them to nearest integer
//!   - Propagate to constrained coordinates
//!   - Resolve conflicts by adjusting adjacent surfaces symmetrically
//!
//! Phase 4: Verify Topology Preservation
//!   Assert all topological constraints are satisfied
//! ```

use std::collections::HashMap;
use vsc_core::{CoordRef, Edge, EntityId, Rational, TopoConstraint};

use super::union_find::UnionFind;

// Use types from crate root (lib.rs) to avoid duplication
use crate::{PVectorBounds, RasterBounds};

/// A coordinate in the pre-rasterization space.
#[derive(Debug, Clone)]
struct RationalCoord {
    entity_id: EntityId,
    edge: Edge,
    value: Rational,
}

/// Result of the rounding algorithm.
#[derive(Debug, Clone)]
pub struct RoundingResult {
    /// Rasterized bounds for each entity.
    pub bounds: HashMap<EntityId, RasterBounds>,

    /// Any topology violations detected (should be empty if algorithm is correct).
    pub violations: Vec<TopologyViolation>,

    /// Statistics about the rounding process.
    pub stats: RoundingStats,
}

/// A topology violation detected during verification.
#[derive(Debug, Clone)]
pub struct TopologyViolation {
    /// The constraint that was violated.
    pub constraint: TopoConstraint,
    /// Human-readable description of the violation.
    pub message: String,
}

/// Statistics about the rounding process.
#[derive(Debug, Clone, Default)]
pub struct RoundingStats {
    /// Total number of coordinates processed.
    pub total_coordinates: usize,
    /// Number of equivalence classes formed.
    pub equivalence_classes: usize,
    /// Number of constraints propagated.
    pub constraints_propagated: usize,
    /// Number of conflicts resolved.
    pub conflicts_resolved: usize,
}

// =============================================================================
// Core Algorithm
// =============================================================================

/// Topology-preserving rounding entry point.
///
/// ## Arguments
///
/// * `entities` - Map from EntityId to P-dimension bounds (exact rational)
/// * `constraints` - Topological constraints to preserve
/// * `device_pixel_ratio` - DPR for coordinate scaling (e.g., 2.0 for Retina)
///
/// ## Returns
///
/// `RoundingResult` containing rasterized bounds and any violations detected.
pub fn round_with_topology_preservation(
    entities: &HashMap<EntityId, PVectorBounds>,
    constraints: &[TopoConstraint],
    device_pixel_ratio: f64,
) -> RoundingResult {
    let mut stats = RoundingStats::default();

    // Phase 1: Extract all coordinates and build equivalence classes
    let coords = extract_coordinates(entities);
    stats.total_coordinates = coords.len();

    let (equiv_classes, coord_to_class) = build_equivalence_classes(&coords, constraints);
    stats.equivalence_classes = equiv_classes.len();

    // Phase 2: Compute rounding for each equivalence class
    let mut rounded_classes: HashMap<CoordRef, i32> = HashMap::new();

    for (class_root, members) in &equiv_classes {
        // All members have the same rational value (or are constrained to be equal)
        // Use the first member's value for rounding
        let rational_value = &members[0].value;
        let float_value = rational_value.to_f64_for_rasterization() * device_pixel_ratio;

        // Default: round to nearest
        rounded_classes.insert(*class_root, float_value.round() as i32);
    }

    // Phase 3: Propagate constraints and resolve conflicts
    let (adjusted, conflicts_resolved) = propagate_constraints(
        rounded_classes,
        &equiv_classes,
        &coord_to_class,
        constraints,
    );
    stats.constraints_propagated = constraints.len();
    stats.conflicts_resolved = conflicts_resolved;

    // Phase 4: Build final bounds
    let bounds = build_final_bounds(entities, &adjusted, &coord_to_class, device_pixel_ratio);

    // Phase 5: Verify topology
    let violations = verify_topology(&bounds, constraints);

    RoundingResult {
        bounds,
        violations,
        stats,
    }
}

// =============================================================================
// Phase 1: Coordinate Extraction and Equivalence Classes
// =============================================================================

fn extract_coordinates(entities: &HashMap<EntityId, PVectorBounds>) -> Vec<RationalCoord> {
    let mut coords = Vec::with_capacity(entities.len() * 4);

    for (&entity_id, bounds) in entities {
        coords.push(RationalCoord {
            entity_id,
            edge: Edge::Left,
            value: bounds.top_left.x.clone(),
        });
        coords.push(RationalCoord {
            entity_id,
            edge: Edge::Right,
            value: bounds.bottom_right.x.clone(),
        });
        coords.push(RationalCoord {
            entity_id,
            edge: Edge::Top,
            value: bounds.top_left.y.clone(),
        });
        coords.push(RationalCoord {
            entity_id,
            edge: Edge::Bottom,
            value: bounds.bottom_right.y.clone(),
        });
    }

    coords
}

/// Build equivalence classes from coordinates and equality constraints.
///
/// Two coordinates are in the same class if:
/// 1. They have the same rational value, OR
/// 2. They are connected by an 'Equal' or 'Adjacent' constraint
fn build_equivalence_classes(
    coords: &[RationalCoord],
    constraints: &[TopoConstraint],
) -> (
    HashMap<CoordRef, Vec<RationalCoord>>,
    HashMap<CoordRef, CoordRef>,
) {
    let mut union_find = UnionFind::with_capacity(coords.len());

    // Initialize each coord
    for coord in coords {
        let key: CoordRef = (coord.entity_id, coord.edge);
        union_find.find(key); // Ensure it exists
    }

    // Union coordinates with same rational value
    // Use normalized Rational as key (Rational implements Hash after normalization)
    let mut by_value: HashMap<String, Vec<&RationalCoord>> = HashMap::new();
    for coord in coords {
        // Use string representation for grouping (Rational doesn't implement Hash directly)
        let val_key = format!("{}", coord.value);
        by_value.entry(val_key).or_default().push(coord);
    }

    for group in by_value.values() {
        if group.len() > 1 {
            let first: CoordRef = (group[0].entity_id, group[0].edge);
            for coord in group.iter().skip(1) {
                let key: CoordRef = (coord.entity_id, coord.edge);
                union_find.union(first, key);
            }
        }
    }

    // Union by equality/adjacency constraints
    for constraint in constraints {
        match constraint {
            TopoConstraint::Equal { a, b } | TopoConstraint::Adjacent { a, b } => {
                union_find.union(*a, *b);
            }
            TopoConstraint::LessThan { .. } => {
                // Less-than constraints don't merge classes
            }
        }
    }

    // Build final classes and coord-to-class mapping
    let mut classes: HashMap<CoordRef, Vec<RationalCoord>> = HashMap::new();
    let mut coord_to_class: HashMap<CoordRef, CoordRef> = HashMap::new();

    for coord in coords {
        let key: CoordRef = (coord.entity_id, coord.edge);
        let root = union_find.find(key);
        coord_to_class.insert(key, root);
        classes.entry(root).or_default().push(coord.clone());
    }

    (classes, coord_to_class)
}

// =============================================================================
// Phase 3: Constraint Propagation
// =============================================================================

/// Propagate rounding decisions through less-than constraints.
///
/// If A < B in rational space, we must ensure round(A) < round(B) in pixel space.
/// If rounding would violate this, we adjust by:
/// 1. Decreasing A by 1, OR
/// 2. Increasing B by 1
///
/// We choose the option that minimizes total visual shift.
fn propagate_constraints(
    mut rounded: HashMap<CoordRef, i32>,
    equiv_classes: &HashMap<CoordRef, Vec<RationalCoord>>,
    coord_to_class: &HashMap<CoordRef, CoordRef>,
    constraints: &[TopoConstraint],
) -> (HashMap<CoordRef, i32>, usize) {
    let mut conflicts_resolved = 0;

    // Process less-than constraints
    for constraint in constraints {
        if let TopoConstraint::LessThan { a, b } = constraint {
            let class_a = coord_to_class.get(a);
            let class_b = coord_to_class.get(b);

            if let (Some(&class_a), Some(&class_b)) = (class_a, class_b) {
                let val_a = *rounded.get(&class_a).unwrap_or(&0);
                let val_b = *rounded.get(&class_b).unwrap_or(&0);

                // Must satisfy: val_a < val_b
                if val_a >= val_b {
                    // Conflict! Need to adjust.
                    // Strategy: Create a gap of 1px

                    // Option 1: Decrease A
                    let cost_decrease_a =
                        compute_adjustment_cost(&class_a, val_a, val_a - 1, equiv_classes);

                    // Option 2: Increase B
                    let cost_increase_b =
                        compute_adjustment_cost(&class_b, val_b, val_b + 1, equiv_classes);

                    if cost_decrease_a <= cost_increase_b {
                        rounded.insert(class_a, val_b - 1);
                    } else {
                        rounded.insert(class_b, val_a + 1);
                    }

                    conflicts_resolved += 1;
                }
            }
        }
    }

    (rounded, conflicts_resolved)
}

/// Compute the visual cost of adjusting a coordinate.
///
/// Cost is proportional to:
/// - Number of entities affected
/// - Distance of adjustment
fn compute_adjustment_cost(
    class_root: &CoordRef,
    from: i32,
    to: i32,
    equiv_classes: &HashMap<CoordRef, Vec<RationalCoord>>,
) -> usize {
    let members = equiv_classes.get(class_root).map(|v| v.len()).unwrap_or(0);
    let distance = (to - from).unsigned_abs() as usize;
    members * distance
}

// =============================================================================
// Phase 4: Build Final Bounds
// =============================================================================

fn build_final_bounds(
    entities: &HashMap<EntityId, PVectorBounds>,
    rounded_classes: &HashMap<CoordRef, i32>,
    coord_to_class: &HashMap<CoordRef, CoordRef>,
    device_pixel_ratio: f64,
) -> HashMap<EntityId, RasterBounds> {
    let mut bounds = HashMap::with_capacity(entities.len());

    for &entity_id in entities.keys() {
        let left_key: CoordRef = (entity_id, Edge::Left);
        let right_key: CoordRef = (entity_id, Edge::Right);
        let top_key: CoordRef = (entity_id, Edge::Top);
        let bottom_key: CoordRef = (entity_id, Edge::Bottom);

        let get_rounded = |key: CoordRef| -> i32 {
            let class_root = coord_to_class.get(&key).copied().unwrap_or(key);
            *rounded_classes.get(&class_root).unwrap_or(&0)
        };

        let left = get_rounded(left_key);
        let right = get_rounded(right_key);
        let top = get_rounded(top_key);
        let bottom = get_rounded(bottom_key);

        // Convert from device pixels to CSS pixels
        bounds.insert(
            entity_id,
            RasterBounds {
                x: left as f64 / device_pixel_ratio,
                y: top as f64 / device_pixel_ratio,
                width: (right - left) as f64 / device_pixel_ratio,
                height: (bottom - top) as f64 / device_pixel_ratio,
            },
        );
    }

    bounds
}

// =============================================================================
// Phase 5: Topology Verification
// =============================================================================

fn verify_topology(
    bounds: &HashMap<EntityId, RasterBounds>,
    constraints: &[TopoConstraint],
) -> Vec<TopologyViolation> {
    let mut violations = Vec::new();

    for constraint in constraints {
        let (a, b, constraint_type) = match constraint {
            TopoConstraint::Equal { a, b } => (a, b, "equal"),
            TopoConstraint::Adjacent { a, b } => (a, b, "adjacent"),
            TopoConstraint::LessThan { a, b } => (a, b, "less-than"),
        };

        let bounds_a = bounds.get(&a.0);
        let bounds_b = bounds.get(&b.0);

        if let (Some(bounds_a), Some(bounds_b)) = (bounds_a, bounds_b) {
            let val_a = get_edge_value(bounds_a, a.1);
            let val_b = get_edge_value(bounds_b, b.1);

            let violated = match constraint_type {
                "equal" | "adjacent" => (val_a - val_b).abs() > 1e-9,
                "less-than" => val_a >= val_b,
                _ => false,
            };

            if violated {
                violations.push(TopologyViolation {
                    constraint: constraint.clone(),
                    message: format!(
                        "{} constraint violated: {:.3} vs {:.3}",
                        constraint_type, val_a, val_b
                    ),
                });
            }
        }
    }

    violations
}

fn get_edge_value(bounds: &RasterBounds, edge: Edge) -> f64 {
    match edge {
        Edge::Left => bounds.x,
        Edge::Right => bounds.x + bounds.width,
        Edge::Top => bounds.y,
        Edge::Bottom => bounds.y + bounds.height,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PVector;

    fn make_bounds(left: i64, top: i64, right: i64, bottom: i64) -> PVectorBounds {
        PVectorBounds {
            top_left: PVector {
                x: Rational::from_int(left),
                y: Rational::from_int(top),
                z: Rational::zero(),
                t: Rational::zero(),
            },
            bottom_right: PVector {
                x: Rational::from_int(right),
                y: Rational::from_int(bottom),
                z: Rational::zero(),
                t: Rational::zero(),
            },
        }
    }

    fn make_rational_bounds(
        left_num: i64,
        left_den: i64,
        top: i64,
        right_num: i64,
        right_den: i64,
        bottom: i64,
    ) -> PVectorBounds {
        PVectorBounds {
            top_left: PVector {
                x: Rational::new(left_num, left_den),
                y: Rational::from_int(top),
                z: Rational::zero(),
                t: Rational::zero(),
            },
            bottom_right: PVector {
                x: Rational::new(right_num, right_den),
                y: Rational::from_int(bottom),
                z: Rational::zero(),
                t: Rational::zero(),
            },
        }
    }

    #[test]
    fn test_basic_rounding() {
        let mut entities = HashMap::new();
        entities.insert(EntityId(1), make_bounds(0, 0, 100, 50));

        let result = round_with_topology_preservation(&entities, &[], 1.0);

        assert!(result.violations.is_empty());
        let bounds = result.bounds.get(&EntityId(1)).unwrap();
        assert_eq!(bounds.x, 0.0);
        assert_eq!(bounds.y, 0.0);
        assert_eq!(bounds.width, 100.0);
        assert_eq!(bounds.height, 50.0);
    }

    #[test]
    fn test_adjacent_surfaces_preserve_adjacency() {
        // Two adjacent surfaces: A.right = B.left = 100.333...
        let mut entities = HashMap::new();
        entities.insert(EntityId(1), make_rational_bounds(0, 1, 0, 301, 3, 50)); // 0 to 100.333...
        entities.insert(EntityId(2), make_rational_bounds(301, 3, 0, 200, 1, 50)); // 100.333... to 200

        let constraints = vec![TopoConstraint::Adjacent {
            a: (EntityId(1), Edge::Right),
            b: (EntityId(2), Edge::Left),
        }];

        let result = round_with_topology_preservation(&entities, &constraints, 1.0);

        // Should have no violations
        assert!(
            result.violations.is_empty(),
            "Violations: {:?}",
            result.violations
        );

        // A.right should equal B.left
        let bounds_a = result.bounds.get(&EntityId(1)).unwrap();
        let bounds_b = result.bounds.get(&EntityId(2)).unwrap();
        let a_right = bounds_a.x + bounds_a.width;
        let b_left = bounds_b.x;
        assert!(
            (a_right - b_left).abs() < 1e-9,
            "Adjacency violated: A.right={}, B.left={}",
            a_right,
            b_left
        );
    }

    #[test]
    fn test_containment_preserves_order() {
        // Parent contains child: parent.left < child.left < child.right < parent.right
        let mut entities = HashMap::new();
        entities.insert(EntityId(1), make_bounds(0, 0, 100, 50)); // Parent
        entities.insert(EntityId(2), make_bounds(10, 10, 90, 40)); // Child

        let constraints = vec![
            TopoConstraint::LessThan {
                a: (EntityId(1), Edge::Left),
                b: (EntityId(2), Edge::Left),
            },
            TopoConstraint::LessThan {
                a: (EntityId(2), Edge::Right),
                b: (EntityId(1), Edge::Right),
            },
        ];

        let result = round_with_topology_preservation(&entities, &constraints, 1.0);

        assert!(
            result.violations.is_empty(),
            "Violations: {:?}",
            result.violations
        );

        let parent = result.bounds.get(&EntityId(1)).unwrap();
        let child = result.bounds.get(&EntityId(2)).unwrap();

        assert!(parent.x < child.x);
        assert!(child.x + child.width < parent.x + parent.width);
    }

    #[test]
    fn test_dpr_scaling() {
        let mut entities = HashMap::new();
        entities.insert(EntityId(1), make_bounds(0, 0, 100, 50));

        // DPR = 2.0 (Retina)
        let result = round_with_topology_preservation(&entities, &[], 2.0);

        // Device pixels are scaled, but CSS pixels should be halved
        let bounds = result.bounds.get(&EntityId(1)).unwrap();
        // 100 * 2 = 200 device pixels, /2 = 100 CSS pixels
        assert_eq!(bounds.width, 100.0);
    }

    #[test]
    fn test_equivalence_classes() {
        // Three surfaces with same right/left boundary
        let mut entities = HashMap::new();
        entities.insert(EntityId(1), make_bounds(0, 0, 50, 30)); // A: 0-50
        entities.insert(EntityId(2), make_bounds(50, 0, 100, 30)); // B: 50-100
        entities.insert(EntityId(3), make_bounds(50, 30, 100, 60)); // C: 50-100 (below B)

        // A.right = B.left = C.left (same rational value)
        let result = round_with_topology_preservation(&entities, &[], 1.0);

        // All three boundaries should be at same pixel
        let a = result.bounds.get(&EntityId(1)).unwrap();
        let b = result.bounds.get(&EntityId(2)).unwrap();
        let c = result.bounds.get(&EntityId(3)).unwrap();

        let a_right = a.x + a.width;
        assert!(
            (a_right - b.x).abs() < 1e-9,
            "A.right != B.left: {} vs {}",
            a_right,
            b.x
        );
        assert!(
            (b.x - c.x).abs() < 1e-9,
            "B.left != C.left: {} vs {}",
            b.x,
            c.x
        );
    }

    /// Task 1: Non-integer DPR 1.5 – rounding must not produce NaN/inf and
    /// the output CSS-pixel dimensions must be within 1 CSS pixel of the input.
    #[test]
    fn test_non_integer_dpr_1_5() {
        let mut entities = HashMap::new();
        // A 100×50 logical-pixel rectangle
        entities.insert(EntityId(1), make_bounds(0, 0, 100, 50));

        let result = round_with_topology_preservation(&entities, &[], 1.5);

        assert!(result.violations.is_empty());
        let bounds = result.bounds.get(&EntityId(1)).unwrap();

        // x and y should be 0
        assert!(bounds.x.is_finite(), "x is not finite");
        assert!(bounds.y.is_finite(), "y is not finite");
        assert!(bounds.width.is_finite(), "width is not finite");
        assert!(bounds.height.is_finite(), "height is not finite");

        // Original width = 100 CSS px; after rounding at DPR 1.5 → 150 device px → /1.5 = 100
        assert!(
            (bounds.width - 100.0).abs() < 1.0,
            "width after DPR 1.5 rounding deviates by more than 1px: {}",
            bounds.width
        );
        assert!(
            (bounds.height - 50.0).abs() < 1.0,
            "height after DPR 1.5 rounding deviates by more than 1px: {}",
            bounds.height
        );
    }

    /// Task 1 (continued): Non-integer DPR 2.25 – adjacency must still be preserved.
    #[test]
    fn test_non_integer_dpr_2_25_adjacency_preserved() {
        let mut entities = HashMap::new();
        // Two adjacent surfaces, boundary at the non-representable fraction 1/3 * 100
        entities.insert(EntityId(1), make_rational_bounds(0, 1, 0, 100, 3, 50));
        entities.insert(EntityId(2), make_rational_bounds(100, 3, 0, 200, 1, 50));

        let constraints = vec![TopoConstraint::Adjacent {
            a: (EntityId(1), Edge::Right),
            b: (EntityId(2), Edge::Left),
        }];

        let result = round_with_topology_preservation(&entities, &constraints, 2.25);

        assert!(
            result.violations.is_empty(),
            "Violations at DPR 2.25: {:?}",
            result.violations
        );

        let bounds_a = result.bounds.get(&EntityId(1)).unwrap();
        let bounds_b = result.bounds.get(&EntityId(2)).unwrap();
        let a_right = bounds_a.x + bounds_a.width;
        let b_left = bounds_b.x;
        assert!(
            (a_right - b_left).abs() < 1e-9,
            "Adjacency violated at DPR 2.25: A.right={}, B.left={}",
            a_right,
            b_left
        );
    }

    #[test]
    fn test_stats() {
        let mut entities = HashMap::new();
        entities.insert(EntityId(1), make_bounds(0, 0, 100, 50));
        entities.insert(EntityId(2), make_bounds(100, 0, 200, 50));

        let constraints = vec![TopoConstraint::Adjacent {
            a: (EntityId(1), Edge::Right),
            b: (EntityId(2), Edge::Left),
        }];

        let result = round_with_topology_preservation(&entities, &constraints, 1.0);

        // 2 entities * 4 edges = 8 coordinates
        assert_eq!(result.stats.total_coordinates, 8);
        assert!(result.stats.equivalence_classes > 0);
        assert_eq!(result.stats.constraints_propagated, 1);
    }
}
