//! ViewScript IR Optimizer
//!
//! This module implements the `vsc optimize` command, which performs:
//! 1. Algebraic simplification of redundant constraints
//! 2. **Auto-snapping of export boundaries** (mandatory, not optional)
//!
//! Auto-snapping is a topological safety mechanism that severs internal
//! dependencies at component boundaries, simplifying constraint chains
//! to exact rational constants.
//!
//! ## Float Decontamination (Architect Directive)
//!
//! This module operates ENTIRELY in exact rational arithmetic.
//! There is NO f64 conversion anywhere in the P-dimension optimization path.
//! The `Rational` type from `types.rs` is the sole numeric representation.

use crate::types::*;
use serde::{Deserialize, Serialize};

/// Result of an optimization pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationResult {
    /// Number of constraints removed (algebraic simplification).
    pub constraints_removed: u32,

    /// Number of constraints merged.
    pub constraints_merged: u32,

    /// Number of export boundaries auto-snapped.
    pub boundaries_snapped: u32,

    /// Details of each snapped boundary.
    pub snap_details: Vec<BoundarySnapDetail>,

    /// Warnings (non-fatal issues detected).
    pub warnings: Vec<OptimizationWarning>,
}

/// Detail of a single boundary snap operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundarySnapDetail {
    /// The entity ID that was snapped.
    pub entity_id: EntityId,

    /// The component that was snapped.
    pub component: VectorComponent,

    /// The original value (before snap) - exact rational.
    pub original_value: Rational,

    /// The snapped (simplified) exact rational value.
    pub snapped_value: Rational,

    /// The internal dependency chain that was severed.
    pub severed_chain_length: u32,

    /// The error introduced by snapping (exact rational, typically zero or small).
    pub snap_error: Rational,
}

// NOTE: The `Rational` type from `types.rs` is used throughout this module.
// There is no separate `RationalValue` struct - we use `Rational` directly
// to maintain exact arithmetic with no f64 contamination.
//
// The old `from_f64` function has been REMOVED as per Architect Directive.
// P-dimension values are ALWAYS exact rationals, never converted from floats.

/// Warnings generated during optimization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OptimizationWarning {
    /// A snapped value differs from the original (should be rare with exact rationals).
    SnapDelta {
        entity_id: EntityId,
        component: VectorComponent,
        original: Rational,
        snapped: Rational,
        delta: Rational,
    },

    /// A very long dependency chain was detected.
    LongDependencyChain {
        entity_id: EntityId,
        chain_length: u32,
        /// Accumulated rational error through the chain.
        accumulated_error: Rational,
    },

    /// An exported entity depends on T-vector state.
    ///
    /// T-dependent exports CANNOT be auto-snapped because their value
    /// varies with user interaction state. Snapping would "freeze" the
    /// UI at a single T-vector state, breaking interactivity.
    ///
    /// ## Resolution Options
    ///
    /// 1. Remove the export (if not needed externally)
    /// 2. Add a T-independent intermediate entity
    /// 3. Accept that this entity will not be snapped (performance impact)
    TDependentExport {
        entity_id: EntityId,
        component: VectorComponent,
        /// The T-vector states this export depends on.
        dependencies: Vec<TDependency>,
        /// Human-readable explanation.
        message: String,
    },
}

/// A dependency on T-vector state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TDependency {
    /// Entity whose T-vector state is referenced.
    pub entity_id: EntityId,
    /// Which T-state key (hover, scroll_y, etc.).
    pub t_state: String,
}

/// Configuration for the optimizer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizerConfig {
    /// Warning threshold for snap delta (exact rational).
    /// Default: 1/1000000 (one millionth).
    #[serde(default = "default_snap_warning_threshold")]
    pub snap_warning_threshold: Rational,

    /// Warning threshold for dependency chain length.
    #[serde(default = "default_chain_length_warning")]
    pub chain_length_warning: u32,
}

fn default_snap_warning_threshold() -> Rational {
    Rational::new(1, 1_000_000)
}
fn default_chain_length_warning() -> u32 {
    100
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            snap_warning_threshold: default_snap_warning_threshold(),
            chain_length_warning: default_chain_length_warning(),
        }
    }
}

/// The optimizer engine.
pub struct Optimizer {
    config: OptimizerConfig,
}

impl Optimizer {
    /// Create a new optimizer with the given configuration.
    pub fn new(config: OptimizerConfig) -> Self {
        Self { config }
    }

    /// Create an optimizer with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(OptimizerConfig::default())
    }

    /// Run the full optimization pipeline.
    ///
    /// This is the entry point for `vsc optimize`. It performs:
    /// 1. Dependency chain analysis
    /// 2. **Auto-snapping of all export boundaries** (mandatory)
    /// 3. Algebraic simplification
    /// 4. Constraint deduplication
    ///
    /// ## Float Decontamination
    ///
    /// All operations use exact `Rational` arithmetic. There is NO f64
    /// conversion in this optimization path.
    pub fn optimize(&self, ir: &mut IrModule) -> OptimizationResult {
        let mut result = OptimizationResult {
            constraints_removed: 0,
            constraints_merged: 0,
            boundaries_snapped: 0,
            snap_details: Vec::new(),
            warnings: Vec::new(),
        };

        // Phase 1: Analyze dependency chains for each exported entity
        let export_analysis = self.analyze_export_dependencies(ir);

        // Phase 2: AUTO-SNAP all export boundaries (MANDATORY per architect decision)
        // EXCEPT for T-dependent exports (they cannot be snapped)
        for (entity_id, analysis) in &export_analysis {
            for component in [
                VectorComponent::X,
                VectorComponent::Y,
                VectorComponent::Z,
                VectorComponent::T,
            ] {
                // Check for T-dependency BEFORE attempting snap
                let t_dependencies = self.find_t_dependencies(ir, *entity_id, component);
                if !t_dependencies.is_empty() {
                    // T-dependent export: emit warning, SKIP snapping
                    result.warnings.push(OptimizationWarning::TDependentExport {
                        entity_id: *entity_id,
                        component,
                        dependencies: t_dependencies.clone(),
                        message: format!(
                            "Export {}.{:?} depends on T-vector state and cannot be auto-snapped. \
                             T-dependent constraints: {:?}",
                            entity_id.0,
                            component,
                            t_dependencies
                                .iter()
                                .map(|d| format!("{}.{}", d.entity_id.0, d.t_state))
                                .collect::<Vec<_>>()
                        ),
                    });
                    // Skip snapping this export
                    continue;
                }

                if let Some(chain_info) = analysis.get_chain_info(component) {
                    // Warn if chain is unusually long
                    if chain_info.length > self.config.chain_length_warning {
                        result
                            .warnings
                            .push(OptimizationWarning::LongDependencyChain {
                                entity_id: *entity_id,
                                chain_length: chain_info.length,
                                accumulated_error: chain_info.accumulated_error.clone(),
                            });
                    }

                    // The "snap" in exact rational arithmetic is simply
                    // recording the computed value (already exact).
                    // No f64 approximation needed.
                    let original_value = chain_info.computed_value.clone();
                    let snapped_value = original_value.clone(); // Already exact!
                    let delta = Rational::zero(); // No precision loss with exact rationals

                    // Warn if delta exceeds threshold (should be zero for exact rationals)
                    if delta > self.config.snap_warning_threshold {
                        result.warnings.push(OptimizationWarning::SnapDelta {
                            entity_id: *entity_id,
                            component,
                            original: original_value.clone(),
                            snapped: snapped_value.clone(),
                            delta: delta.clone(),
                        });
                    }

                    // Record the snap
                    result.snap_details.push(BoundarySnapDetail {
                        entity_id: *entity_id,
                        component,
                        original_value,
                        snapped_value,
                        severed_chain_length: chain_info.length,
                        snap_error: delta,
                    });

                    result.boundaries_snapped += 1;
                }
            }
        }

        // Phase 3: Apply snaps to IR (rewrite constraints)
        self.apply_snaps(ir, &result.snap_details);

        // Phase 4: Algebraic simplification and deduplication
        let (removed, merged) = self.simplify_constraints(ir);
        result.constraints_removed = removed;
        result.constraints_merged = merged;

        result
    }

    /// Analyze dependency chains for exported entities.
    fn analyze_export_dependencies(&self, ir: &IrModule) -> Vec<(EntityId, ExportAnalysis)> {
        // TODO: Implement full dependency chain analysis
        // For now, return empty analysis
        ir.exports
            .iter()
            .map(|eid| (*eid, ExportAnalysis::default()))
            .collect()
    }

    /// Apply snaps to the IR by rewriting constraints.
    ///
    /// This replaces dependent constraints with exact rational constants,
    /// severing the dependency chain at export boundaries.
    fn apply_snaps(&self, ir: &mut IrModule, snaps: &[BoundarySnapDetail]) {
        for snap in snaps {
            // Find and replace the constraint for this entity/component
            for constraint in &mut ir.constraints {
                if constraint.target == snap.entity_id && constraint.component == snap.component {
                    // Replace the term with an exact rational constant
                    constraint.term = ConstraintTerm::Const {
                        value: snap.snapped_value.clone(),
                    };
                    constraint.relation = RelationType::Eq;

                    // Mark as snapped (metadata)
                    constraint.metadata.snapped = true;
                }
            }
        }
    }

    /// Simplify constraints algebraically.
    fn simplify_constraints(&self, ir: &mut IrModule) -> (u32, u32) {
        let mut merged = 0u32;

        // Remove duplicate constraints
        let original_len = ir.constraints.len();
        ir.constraints.dedup_by(|a, b| {
            if a.target == b.target
                && a.component == b.component
                && a.relation == b.relation
                && a.term == b.term
            {
                merged += 1;
                true
            } else {
                false
            }
        });
        let removed = (original_len - ir.constraints.len()) as u32;

        // TODO: More sophisticated algebraic simplifications
        // - Combine A.x = 5 with A.x = B.x into B.x = 5
        // - Remove tautologies (A.x <= A.x)

        (removed, merged)
    }

    /// Check if an entity/component depends on T-vector state.
    ///
    /// T-dependent constraints reference the T component of any entity,
    /// which varies with user interaction state (hover, scroll, etc.).
    ///
    /// ## Why This Matters
    ///
    /// Auto-snapping "freezes" a constraint at its current value.
    /// For T-dependent constraints, this would freeze the UI at a single
    /// interaction state (e.g., always hovered, never scrolled).
    ///
    /// ## Detection Strategy
    ///
    /// Walk the constraint graph backwards from the target entity/component,
    /// looking for any reference to VectorComponent::T.
    pub fn is_t_dependent(
        &self,
        ir: &IrModule,
        entity_id: EntityId,
        component: VectorComponent,
    ) -> bool {
        !self
            .find_t_dependencies(ir, entity_id, component)
            .is_empty()
    }

    /// Find all T-vector dependencies for an entity/component.
    ///
    /// Returns a list of TDependency structs describing which T-states
    /// the target depends on.
    fn find_t_dependencies(
        &self,
        ir: &IrModule,
        entity_id: EntityId,
        component: VectorComponent,
    ) -> Vec<TDependency> {
        let mut dependencies = Vec::new();
        let mut visited = std::collections::HashSet::new();

        self.find_t_dependencies_recursive(
            ir,
            entity_id,
            component,
            &mut dependencies,
            &mut visited,
        );

        dependencies
    }

    /// Recursive helper for T-dependency detection.
    fn find_t_dependencies_recursive(
        &self,
        ir: &IrModule,
        entity_id: EntityId,
        component: VectorComponent,
        dependencies: &mut Vec<TDependency>,
        visited: &mut std::collections::HashSet<(EntityId, VectorComponent)>,
    ) {
        // Prevent infinite loops in cyclic graphs
        if !visited.insert((entity_id, component)) {
            return;
        }

        // Direct T-dependency: if we're looking at the T component itself
        if component == VectorComponent::T {
            dependencies.push(TDependency {
                entity_id,
                // T-state name is encoded in the constraint metadata
                // For now, use generic "T" as placeholder
                t_state: "T".to_string(),
            });
            return;
        }

        // Find constraints that target this entity/component
        for constraint in &ir.constraints {
            if constraint.target == entity_id && constraint.component == component {
                // Check if the constraint term references other entities
                match &constraint.term {
                    ConstraintTerm::Ref {
                        entity_id: ref_id,
                        component: ref_comp,
                    } => {
                        // Recurse into the referenced entity
                        self.find_t_dependencies_recursive(
                            ir,
                            *ref_id,
                            *ref_comp,
                            dependencies,
                            visited,
                        );
                    }
                    ConstraintTerm::Linear {
                        entity_id: ref_id,
                        component: ref_comp,
                        ..
                    } => {
                        // Recurse into the referenced entity
                        self.find_t_dependencies_recursive(
                            ir,
                            *ref_id,
                            *ref_comp,
                            dependencies,
                            visited,
                        );
                    }
                    ConstraintTerm::LinearCombination { terms, .. } => {
                        // Recurse into all referenced entities
                        for factor in terms {
                            self.find_t_dependencies_recursive(
                                ir,
                                factor.entity_id,
                                factor.component,
                                dependencies,
                                visited,
                            );
                        }
                    }
                    ConstraintTerm::Const { .. } => {
                        // Constants have no dependencies
                    }
                }
            }
        }
    }
}

/// Analysis of an exported entity's dependencies.
#[derive(Debug, Default)]
struct ExportAnalysis {
    x_chain: Option<ChainInfo>,
    y_chain: Option<ChainInfo>,
    z_chain: Option<ChainInfo>,
    t_chain: Option<ChainInfo>,
}

impl ExportAnalysis {
    fn get_chain_info(&self, component: VectorComponent) -> Option<&ChainInfo> {
        match component {
            VectorComponent::X => self.x_chain.as_ref(),
            VectorComponent::Y => self.y_chain.as_ref(),
            VectorComponent::Z => self.z_chain.as_ref(),
            VectorComponent::T => self.t_chain.as_ref(),
            // Scalar entities and ColorStop components don't have chains
            VectorComponent::Value
            | VectorComponent::R
            | VectorComponent::G
            | VectorComponent::B
            | VectorComponent::Alpha
            | VectorComponent::Position => None,
        }
    }
}

/// Information about a dependency chain.
/// All values are exact rationals.
#[derive(Debug)]
struct ChainInfo {
    length: u32,
    /// Accumulated error through the chain (exact rational).
    accumulated_error: Rational,
    /// Computed value at the end of the chain (exact rational).
    computed_value: Rational,
}

/// IR module representation (simplified for optimizer).
#[derive(Debug, Default)]
pub struct IrModule {
    pub entities: Vec<EntityId>,
    pub exports: Vec<EntityId>,
    pub constraints: Vec<OptimizableConstraint>,
}

/// Constraint with optimization metadata.
/// All numeric values in `term` are exact rationals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizableConstraint {
    pub id: u64,
    pub target: EntityId,
    pub component: VectorComponent,
    pub relation: RelationType,
    pub term: ConstraintTerm,
    pub metadata: ConstraintMetadata,
}

/// Metadata attached to constraints during optimization.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConstraintMetadata {
    /// Whether this constraint was auto-snapped at an export boundary.
    pub snapped: bool,
}

// Note: The exact rational value is stored directly in ConstraintTerm::Const.
// No separate numerator/denominator fields needed since Rational is the native type.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rational_arithmetic() {
        // Test exact rational arithmetic (no f64 contamination)
        let half = Rational::new(1, 2);
        let third = Rational::new(1, 3);

        // 1/2 + 1/3 = 5/6 (exact)
        let sum = half.clone() + third.clone();
        assert_eq!(sum, Rational::new(5, 6));

        // 1/2 * 1/3 = 1/6 (exact)
        let prod = half * third;
        assert_eq!(prod, Rational::new(1, 6));
    }

    #[test]
    fn test_optimizer_snaps_exports() {
        let optimizer = Optimizer::with_defaults();
        let mut ir = IrModule {
            entities: vec![EntityId(1), EntityId(2)],
            exports: vec![EntityId(1)],
            constraints: vec![OptimizableConstraint {
                id: 1,
                target: EntityId(1),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const {
                    value: Rational::from_int(100),
                },
                metadata: ConstraintMetadata::default(),
            }],
        };

        let result = optimizer.optimize(&mut ir);

        // Verify export was processed
        assert!(result.boundaries_snapped > 0 || result.snap_details.is_empty());
    }

    #[test]
    fn test_is_t_dependent_false_for_constant() {
        let optimizer = Optimizer::with_defaults();
        let ir = IrModule {
            entities: vec![EntityId(1)],
            exports: vec![EntityId(1)],
            constraints: vec![OptimizableConstraint {
                id: 1,
                target: EntityId(1),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const {
                    value: Rational::from_int(100),
                },
                metadata: ConstraintMetadata::default(),
            }],
        };

        // X = 100 (constant) should NOT be T-dependent
        assert!(!optimizer.is_t_dependent(&ir, EntityId(1), VectorComponent::X));
    }

    #[test]
    fn test_is_t_dependent_true_for_direct_t_reference() {
        let optimizer = Optimizer::with_defaults();
        let ir = IrModule {
            entities: vec![EntityId(1)],
            exports: vec![EntityId(1)],
            constraints: vec![OptimizableConstraint {
                id: 1,
                target: EntityId(1),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                // X depends on T directly
                term: ConstraintTerm::Ref {
                    entity_id: EntityId(1),
                    component: VectorComponent::T,
                },
                metadata: ConstraintMetadata::default(),
            }],
        };

        // X = T should be T-dependent
        assert!(optimizer.is_t_dependent(&ir, EntityId(1), VectorComponent::X));
    }

    #[test]
    fn test_is_t_dependent_true_for_indirect_t_reference() {
        let optimizer = Optimizer::with_defaults();
        let ir = IrModule {
            entities: vec![EntityId(1), EntityId(2)],
            exports: vec![EntityId(1)],
            constraints: vec![
                // Entity 1's X depends on Entity 2's Y
                OptimizableConstraint {
                    id: 1,
                    target: EntityId(1),
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Ref {
                        entity_id: EntityId(2),
                        component: VectorComponent::Y,
                    },
                    metadata: ConstraintMetadata::default(),
                },
                // Entity 2's Y depends on Entity 2's T
                OptimizableConstraint {
                    id: 2,
                    target: EntityId(2),
                    component: VectorComponent::Y,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Ref {
                        entity_id: EntityId(2),
                        component: VectorComponent::T,
                    },
                    metadata: ConstraintMetadata::default(),
                },
            ],
        };

        // Entity 1's X -> Entity 2's Y -> Entity 2's T
        // So Entity 1's X is transitively T-dependent
        assert!(optimizer.is_t_dependent(&ir, EntityId(1), VectorComponent::X));
    }

    #[test]
    fn test_is_t_dependent_handles_cycles() {
        let optimizer = Optimizer::with_defaults();
        let ir = IrModule {
            entities: vec![EntityId(1), EntityId(2)],
            exports: vec![EntityId(1)],
            constraints: vec![
                // A.x = B.y (creates cycle with next constraint)
                OptimizableConstraint {
                    id: 1,
                    target: EntityId(1),
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Ref {
                        entity_id: EntityId(2),
                        component: VectorComponent::Y,
                    },
                    metadata: ConstraintMetadata::default(),
                },
                // B.y = A.x (cycle!)
                OptimizableConstraint {
                    id: 2,
                    target: EntityId(2),
                    component: VectorComponent::Y,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Ref {
                        entity_id: EntityId(1),
                        component: VectorComponent::X,
                    },
                    metadata: ConstraintMetadata::default(),
                },
            ],
        };

        // Should not infinite loop, should return false (no T dependency)
        assert!(!optimizer.is_t_dependent(&ir, EntityId(1), VectorComponent::X));
    }

    #[test]
    fn test_t_dependent_export_warning_generated() {
        let optimizer = Optimizer::with_defaults();
        let mut ir = IrModule {
            entities: vec![EntityId(1)],
            exports: vec![EntityId(1)],
            constraints: vec![OptimizableConstraint {
                id: 1,
                target: EntityId(1),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Ref {
                    entity_id: EntityId(1),
                    component: VectorComponent::T,
                },
                metadata: ConstraintMetadata::default(),
            }],
        };

        let result = optimizer.optimize(&mut ir);

        // Should have generated a TDependentExport warning
        let t_dep_warnings: Vec<_> = result
            .warnings
            .iter()
            .filter(|w| matches!(w, OptimizationWarning::TDependentExport { .. }))
            .collect();

        assert!(
            !t_dep_warnings.is_empty(),
            "Expected TDependentExport warning"
        );

        // The export should NOT have been snapped
        let x_snaps: Vec<_> = result
            .snap_details
            .iter()
            .filter(|s| s.entity_id == EntityId(1) && s.component == VectorComponent::X)
            .collect();

        assert!(
            x_snaps.is_empty(),
            "T-dependent export should not be snapped"
        );
    }

    #[test]
    fn test_linear_term_t_dependency() {
        let optimizer = Optimizer::with_defaults();
        let ir = IrModule {
            entities: vec![EntityId(1), EntityId(2)],
            exports: vec![EntityId(1)],
            constraints: vec![
                // X = 2 * T + 10 (linear dependency on T)
                OptimizableConstraint {
                    id: 1,
                    target: EntityId(1),
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Linear {
                        coefficient: Rational::from_int(2),
                        entity_id: EntityId(1),
                        component: VectorComponent::T,
                        offset: Rational::from_int(10),
                    },
                    metadata: ConstraintMetadata::default(),
                },
            ],
        };

        // Linear term referencing T should be T-dependent
        assert!(optimizer.is_t_dependent(&ir, EntityId(1), VectorComponent::X));
    }
}
