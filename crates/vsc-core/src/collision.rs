//! Constraint Collision Detection and Repair Suggestion Generation
//!
//! When an LLM adds a constraint that conflicts with existing constraints,
//! the CLI outputs a structured JSON response with repair suggestions
//! ordered by "mathematical distance" (minimal logical change).
//!
//! ## Float Decontamination (Architect Directive)
//!
//! All mathematical distance calculations use exact rational arithmetic.
//! The `composite_score` and `relative_error` fields are computed using
//! the `Rational` type, not `f64`, to preserve P-dimension decidability.

use crate::types::*;
use serde::{Deserialize, Serialize};

/// The top-level error response when a constraint collision is detected.
/// This is output to stdout with Exit Code 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintCollisionError {
    /// Error type identifier for machine parsing.
    pub error_type: CollisionErrorType,

    /// Human-readable summary of the collision.
    pub message: String,

    /// The newly added constraint that caused the collision.
    pub incoming_constraint: ConstraintSnapshot,

    /// The set of existing constraints that conflict with the incoming one.
    pub conflicting_constraints: Vec<ConstraintSnapshot>,

    /// Repair suggestions ordered by mathematical distance (ascending).
    /// The first suggestion requires the minimal change to resolve the collision.
    pub repair_suggestions: Vec<RepairSuggestion>,

    /// Metadata about the collision analysis.
    pub analysis: CollisionAnalysis,
}

/// Classification of collision types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollisionErrorType {
    /// A.x < B.x AND B.x < A.x (or similar cycles)
    CircularReference,

    /// A.x = 5 AND A.x = 10 (direct contradiction)
    DirectContradiction,

    /// A.x < B.x AND A.x > B.x (relation type mismatch)
    RelationMismatch,

    /// Constraints form an over-determined system with no solution
    Overdetermined,

    /// Self-reference: A.x depends on A.x
    SelfReference,
}

/// A snapshot of a constraint with its buildinfo metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintSnapshot {
    /// The constraint itself.
    pub constraint: Constraint,

    /// Index in .vsbuildinfo (0 = oldest, higher = newer).
    pub buildinfo_index: u64,

    /// ISO 8601 timestamp when the constraint was added.
    pub added_at: String,

    /// Optional: the natural language intent from the LLM that added this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
}

/// A repair suggestion to resolve the collision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairSuggestion {
    /// Unique identifier for this suggestion (for LLM to select).
    pub suggestion_id: u32,

    /// The mathematical distance metric (lower = less invasive change).
    pub mathematical_distance: MathematicalDistance,

    /// The type of repair action.
    pub action: RepairAction,

    /// Human-readable explanation of what this repair does.
    pub explanation: String,

    /// If the repair involves constant changes, the affected constraints.
    pub affected_constraints: Vec<ConstraintModification>,
}

/// Mathematical distance quantifies how "invasive" a repair is.
/// The CLI uses this to order suggestions from least to most disruptive.
///
/// IMPORTANT: All delta values are SCALE-INVARIANT (relative errors).
/// This ensures that 10px change in a 10000px viewport is treated equivalently
/// to 0.1px change in a 100px viewport (both = 0.1% relative error).
///
/// ## Float Decontamination
///
/// All numeric fields use exact `Rational` arithmetic to preserve P-dimension
/// decidability. The `f64` representation is only used for JSON serialization
/// and display purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MathematicalDistance {
    /// Number of constraints that must be deleted.
    pub deletions: u32,

    /// Number of constraint constants that must be changed.
    pub constant_modifications: u32,

    /// Scale-invariant relative error: Σ |Δvalue| / |reference_scale|
    /// Exact rational representation.
    pub relative_error: Rational,

    /// The reference scale used for normalization (for transparency).
    /// Exact rational representation.
    pub reference_scale: Rational,

    /// Number of relation types that must be changed (e.g., < to ≤).
    pub relation_changes: u32,

    /// Composite score computed using configurable weights.
    /// Exact rational representation.
    pub composite_score: Rational,
}

/// Configurable weights for mathematical distance calculation.
/// These can be overridden in vsconfig.json under `resolution_strategy_weights`.
///
/// ## Float Decontamination
///
/// All weights use exact `Rational` arithmetic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionStrategyWeights {
    /// Weight for constraint deletions (default: 1000)
    #[serde(default = "default_deletion_weight")]
    pub deletion: Rational,

    /// Weight for relation type changes (default: 100)
    #[serde(default = "default_relation_weight")]
    pub relation_change: Rational,

    /// Weight for constant modifications (default: 10)
    #[serde(default = "default_modification_weight")]
    pub constant_modification: Rational,

    /// Weight for relative error component (default: 1)
    #[serde(default = "default_relative_error_weight")]
    pub relative_error: Rational,

    /// Maximum relative error before clamping (default: 1 = 100%)
    #[serde(default = "default_max_relative_error")]
    pub max_relative_error: Rational,
}

fn default_deletion_weight() -> Rational {
    Rational::from_int(1000)
}
fn default_relation_weight() -> Rational {
    Rational::from_int(100)
}
fn default_modification_weight() -> Rational {
    Rational::from_int(10)
}
fn default_relative_error_weight() -> Rational {
    Rational::one()
}
fn default_max_relative_error() -> Rational {
    Rational::one()
}

impl Default for ResolutionStrategyWeights {
    fn default() -> Self {
        Self {
            deletion: default_deletion_weight(),
            relation_change: default_relation_weight(),
            constant_modification: default_modification_weight(),
            relative_error: default_relative_error_weight(),
            max_relative_error: default_max_relative_error(),
        }
    }
}

impl MathematicalDistance {
    /// Compute the composite score using the provided weights.
    /// Uses exact rational arithmetic.
    pub fn compute_composite(&self, weights: &ResolutionStrategyWeights) -> Rational {
        let del_score = Rational::from_int(self.deletions as i64) * weights.deletion.clone();
        let rel_score =
            Rational::from_int(self.relation_changes as i64) * weights.relation_change.clone();
        let mod_score = Rational::from_int(self.constant_modifications as i64)
            * weights.constant_modification.clone();

        // Clamp relative error to max
        let clamped_error = if self.relative_error < weights.max_relative_error {
            self.relative_error.clone()
        } else {
            weights.max_relative_error.clone()
        };
        let err_score = clamped_error * weights.relative_error.clone();

        del_score + rel_score + mod_score + err_score
    }

    /// Create a new MathematicalDistance with scale-invariant relative error.
    /// All arithmetic is exact rational.
    pub fn new(
        deletions: u32,
        constant_modifications: u32,
        absolute_delta: Rational,
        reference_scale: Rational,
        relation_changes: u32,
        weights: &ResolutionStrategyWeights,
    ) -> Self {
        let relative_error = if reference_scale != Rational::zero() {
            absolute_delta.clone() / reference_scale.clone()
        } else {
            Rational::zero() // Avoid division by zero; zero scale means no meaningful error
        };

        let mut dist = Self {
            deletions,
            constant_modifications,
            relative_error: relative_error.abs(),
            reference_scale,
            relation_changes,
            composite_score: Rational::zero(), // Computed below
        };
        dist.composite_score = dist.compute_composite(weights);
        dist
    }
}

/// Types of repair actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RepairAction {
    /// Delete the incoming (newest) constraint.
    RejectIncoming,

    /// Delete one or more existing constraints.
    DeleteExisting { constraint_ids: Vec<u64> },

    /// Modify constants in existing constraints.
    ModifyConstants,

    /// Change relation types (e.g., < to ≤) to allow boundary equality.
    RelaxRelations {
        constraint_ids: Vec<u64>,
        new_relations: Vec<RelationType>,
    },

    /// Break a circular reference by redirecting a dependency.
    BreakCycle {
        constraint_to_modify: u64,
        new_term: ConstraintTerm,
    },
}

/// A specific modification to a constraint's constant.
/// All values are exact rationals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintModification {
    /// The constraint being modified.
    pub constraint_id: u64,

    /// The current term value (for constants or linear offsets).
    pub current_value: Rational,

    /// The suggested new value.
    pub suggested_value: Rational,

    /// The absolute delta (|new - current|).
    pub delta: Rational,
}

/// Metadata about the collision analysis process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollisionAnalysis {
    /// The cycle path if this is a circular reference (entity IDs in cycle order).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_path: Option<Vec<EntityId>>,

    /// The total number of constraints analyzed.
    pub constraints_analyzed: u64,

    /// Time taken for analysis in microseconds.
    pub analysis_time_us: u64,

    /// Whether the collision can be hidden in the theoretical cognitive inaccessibility region.
    pub hideable_in_viewport: bool,

    /// If hideable, the viewport bounds where the collision is invisible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hiding_viewport: Option<ViewportBounds>,
}

/// Viewport bounds for determining theoretical cognitive inaccessibility.
/// All coordinates are exact rationals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewportBounds {
    pub x_min: Rational,
    pub x_max: Rational,
    pub y_min: Rational,
    pub y_max: Rational,
    pub t_start: Rational,
    pub t_end: Rational,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_serialization() {
        let error = ConstraintCollisionError {
            error_type: CollisionErrorType::CircularReference,
            message: "Circular reference detected: A.x < B.x < A.x".to_string(),
            incoming_constraint: ConstraintSnapshot {
                constraint: Constraint {
                    id: 42,
                    target: EntityId(1),
                    component: VectorComponent::X,
                    relation: RelationType::Lt,
                    term: ConstraintTerm::Ref {
                        entity_id: EntityId(2),
                        component: VectorComponent::X,
                    },
                    priority: ConstraintPriority::Hard,
                    source_scope: None,
                },
                buildinfo_index: 15,
                added_at: "2026-05-10T14:30:00Z".to_string(),
                intent: Some("Make element A to the left of element B".to_string()),
            },
            conflicting_constraints: vec![],
            repair_suggestions: vec![RepairSuggestion {
                suggestion_id: 1,
                mathematical_distance: MathematicalDistance {
                    deletions: 0,
                    constant_modifications: 1,
                    // 1/100000 = 0.00001 (exact rational)
                    relative_error: Rational::new(1, 100000),
                    reference_scale: Rational::from_int(100),
                    relation_changes: 0,
                    // 10 + 1/100000 ≈ 10.00001 (exact rational)
                    composite_score: Rational::new(1000001, 100000),
                },
                action: RepairAction::ModifyConstants,
                explanation: "Adjust B.x offset by +0.001 to break the cycle".to_string(),
                affected_constraints: vec![ConstraintModification {
                    constraint_id: 10,
                    current_value: Rational::from_int(100),
                    suggested_value: Rational::new(100001, 1000),
                    delta: Rational::new(1, 1000),
                }],
            }],
            analysis: CollisionAnalysis {
                cycle_path: Some(vec![EntityId(1), EntityId(2), EntityId(1)]),
                constraints_analyzed: 50,
                analysis_time_us: 1234,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        };

        let json = serde_json::to_string_pretty(&error).unwrap();
        assert!(json.contains("circular_reference"));
        assert!(json.contains("mathematical_distance"));
    }
}
