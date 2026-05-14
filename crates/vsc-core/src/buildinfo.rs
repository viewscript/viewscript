//! .vsbuildinfo: Event-Sourcing Ledger for Constraint Operations
//!
//! This module defines the structure of the append-only ledger that tracks
//! the order and intent of constraint additions by the LLM.

use crate::ffi::{DerivedQVariable, QVariable};
use crate::types::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The .vsbuildinfo file structure: an append-only event log.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VsBuildInfo {
    /// Schema version for forward compatibility.
    pub version: u32,

    /// The ordered list of constraint operations (append-only).
    pub operations: Vec<ConstraintOperation>,

    /// Optimization history (for undo after `vsc optimize`).
    pub optimization_runs: Vec<OptimizationRun>,

    /// Phase 7: Tangent (G1 continuity) constraints.
    /// These are stored separately as they expand to bilinear forms.
    #[serde(default)]
    pub tangent_constraints: Vec<TangentConstraintEntry>,

    /// Phase 10: Text entities with their bounding box control points.
    /// When a TextEntity is added, this maps the text ID to its metadata.
    #[serde(default)]
    pub text_entities: Vec<TextEntityEntry>,

    /// Phase 10: Next available entity ID for allocation.
    /// Ensures unique IDs across all entity types.
    #[serde(default)]
    pub next_entity_id: u64,

    /// Phase 13: Layout macro operations.
    /// Higher-order constraints that expand into multiple linear constraints.
    #[serde(default)]
    pub layout_macros: Vec<LayoutMacroOperation>,

    /// Phase G: Path entities (from CODL or scene builder).
    #[serde(default)]
    pub path_entities: Vec<PathEntityEntry>,

    // =========================================================================
    // Phase 17: Gradient Entities
    // =========================================================================
    /// Control points (gradient start/end, center, etc.).
    #[serde(default)]
    pub control_points: Vec<ControlPointEntry>,

    /// Color stops for gradients.
    #[serde(default)]
    pub color_stops: Vec<ColorStopEntry>,

    /// Linear gradients.
    #[serde(default)]
    pub linear_gradients: Vec<LinearGradientEntry>,

    /// Radial gradients.
    #[serde(default)]
    pub radial_gradients: Vec<RadialGradientEntry>,

    /// Conic gradients.
    #[serde(default)]
    pub conic_gradients: Vec<ConicGradientEntry>,

    /// Radius entities (for radial gradients).
    #[serde(default)]
    pub radii: Vec<RadiusEntry>,

    /// Angle entities (for conic gradients).
    #[serde(default)]
    pub angles: Vec<AngleEntry>,

    // =========================================================================
    // Target System
    // =========================================================================
    /// Registered render targets (e.g., "vs-web", "vs-native").
    #[serde(default)]
    pub targets: Vec<String>,

    // =========================================================================
    // Style System
    // =========================================================================
    /// Registered style packages (e.g., "vs-style-chrome", "vs-style-firefox").
    #[serde(default)]
    pub styles: Vec<String>,

    // =========================================================================
    // Q-Dimension FFI
    // =========================================================================
    /// Q-dimension variable declarations.
    /// These define external inputs that can bind to T-dimension variables.
    #[serde(default)]
    pub q_variables: Vec<QVariable>,

    /// Derived Q-dimension variables.
    /// These are computed from rules using Q-values and P-dimension state.
    #[serde(default)]
    pub derived_q_variables: Vec<DerivedQVariable>,
}

impl Default for VsBuildInfo {
    fn default() -> Self {
        Self {
            version: 1,
            operations: Vec::new(),
            optimization_runs: Vec::new(),
            tangent_constraints: Vec::new(),
            text_entities: Vec::new(),
            next_entity_id: 1000, // Reserve 0-999 for legacy/manual IDs
            layout_macros: Vec::new(),
            // Phase G: Path entities
            path_entities: Vec::new(),
            // Phase 17: Gradient entities
            control_points: Vec::new(),
            color_stops: Vec::new(),
            linear_gradients: Vec::new(),
            radial_gradients: Vec::new(),
            conic_gradients: Vec::new(),
            radii: Vec::new(),
            angles: Vec::new(),
            // Target system
            targets: Vec::new(),
            // Style system
            styles: Vec::new(),
            // Q-dimension FFI
            q_variables: Vec::new(),
            derived_q_variables: Vec::new(),
        }
    }
}

/// Entry for a text entity in the buildinfo (Phase 10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextEntityEntry {
    /// Unique identifier for the text entity.
    pub id: EntityId,

    /// Text content.
    pub content: String,

    /// Font family name.
    pub font_family: String,

    /// Font size (rational).
    pub font_size: Rational,

    /// Control point IDs for bounding box corners.
    pub corner_tl: EntityId,
    pub corner_tr: EntityId,
    pub corner_bl: EntityId,
    pub corner_br: EntityId,

    /// Whether metrics have been resolved by the Renderer.
    pub metrics_resolved: bool,

    /// Measured width (set by update-metrics command).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured_width: Option<Rational>,

    /// Measured height (set by update-metrics command).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured_height: Option<Rational>,

    /// Timestamp when the entity was created.
    pub created_at: String,
}

/// Entry for a tangent (collinearity) constraint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TangentConstraintEntry {
    /// Unique identifier.
    pub id: u64,
    /// The junction point (shared endpoint).
    pub junction: EntityId,
    /// Handle from curve 1.
    pub handle1: EntityId,
    /// Handle from curve 2.
    pub handle2: EntityId,
    /// Intent description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// Timestamp.
    pub timestamp: String,
}

// =============================================================================
// Phase 17: Gradient Entity Entries
// =============================================================================

/// Entry for a control point in the buildinfo (Phase 17).
/// Used for gradient start/end points, centers, etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ControlPointEntry {
    /// Unique entity ID.
    pub id: EntityId,
    /// X coordinate (rational).
    pub x: Rational,
    /// Y coordinate (rational).
    pub y: Rational,
    /// Role of the control point.
    pub role: ControlPointRole,
    /// Parent path ID (if part of a path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_path: Option<EntityId>,
}

/// Entry for a color stop in a gradient (Phase 17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ColorStopEntry {
    /// Unique entity ID.
    pub id: EntityId,
    /// Red channel [0, 255] (rational).
    pub r: Rational,
    /// Green channel [0, 255] (rational).
    pub g: Rational,
    /// Blue channel [0, 255] (rational).
    pub b: Rational,
    /// Alpha channel [0, 1] (rational).
    pub a: Rational,
    /// Position along gradient [0, 1] (rational).
    pub position: Rational,
}

/// Entry for a linear gradient (Phase 17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LinearGradientEntry {
    /// Unique entity ID.
    pub id: EntityId,
    /// Start control point ID.
    pub start: EntityId,
    /// End control point ID.
    pub end: EntityId,
    /// Color stop IDs (ordered by position).
    pub stops: Vec<EntityId>,
    /// Tile mode for out-of-bounds sampling.
    #[serde(default)]
    pub tile_mode: TileMode,
    /// Target entity to apply gradient to.
    pub target: EntityId,
}

/// Entry for a radial gradient (Phase 17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RadialGradientEntry {
    /// Unique entity ID.
    pub id: EntityId,
    /// Center control point ID.
    pub center: EntityId,
    /// X-radius entity ID.
    pub radius_x: EntityId,
    /// Y-radius entity ID.
    pub radius_y: EntityId,
    /// Color stop IDs (ordered by position).
    pub stops: Vec<EntityId>,
    /// Tile mode for out-of-bounds sampling.
    #[serde(default)]
    pub tile_mode: TileMode,
    /// Target entity to apply gradient to.
    pub target: EntityId,
}

/// Entry for a conic gradient (Phase 17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConicGradientEntry {
    /// Unique entity ID.
    pub id: EntityId,
    /// Center control point ID.
    pub center: EntityId,
    /// Rotation angle entity ID.
    pub rotation: EntityId,
    /// Start angle entity ID.
    pub start_angle: EntityId,
    /// End angle entity ID.
    pub end_angle: EntityId,
    /// Color stop IDs (ordered by position).
    pub stops: Vec<EntityId>,
    /// Target entity to apply gradient to.
    pub target: EntityId,
}

/// Entry for a radius value (Phase 17).
/// Used for radial gradient radii.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RadiusEntry {
    /// Unique entity ID.
    pub id: EntityId,
    /// Radius value (rational).
    pub value: Rational,
}

/// Entry for an angle value (Phase 17).
/// Used for conic gradient angles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AngleEntry {
    /// Unique entity ID.
    pub id: EntityId,
    /// Angle value in degrees (rational).
    pub value: Rational,
}

/// A single constraint operation in the event log.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConstraintOperation {
    /// Monotonically increasing sequence number (never reused).
    pub seq: u64,

    /// ISO 8601 timestamp.
    pub timestamp: String,

    /// The type of operation.
    pub op_type: OperationType,

    /// The constraint affected by this operation.
    pub constraint: Constraint,

    /// Optional: the natural language intent from the LLM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,

    /// Optional: the CLI command that triggered this operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// If this operation was part of an optimization, reference the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimization_run_id: Option<u64>,
}

/// Types of constraint operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationType {
    /// A new constraint was added.
    Add,

    /// A constraint was modified (constant change, relation change).
    Modify,

    /// A constraint was deleted.
    Delete,

    /// A constraint was merged with another during optimization.
    Merge,

    /// A layout macro was applied (Phase 13).
    /// The actual constraints are stored in expanded_constraints.
    LayoutMacro,
}

// =============================================================================
// Phase 13: Higher-Order Layout Constraints
// =============================================================================

/// Layout type for higher-order layout combinators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayoutType {
    /// Vertical stacking: each item's TL.y = previous item's BL.y + gap
    StackVertical,
    /// Horizontal stacking: each item's TL.x = previous item's TR.x + gap
    StackHorizontal,
}

/// Anchor point for layout alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum LayoutAnchor {
    TL,
    TR,
    BL,
    BR,
}

impl Default for LayoutAnchor {
    fn default() -> Self {
        Self::TL
    }
}

/// Origin coordinates for layout (nullable for optional positioning).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutOrigin {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<Rational>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<Rational>,
}

impl Default for LayoutOrigin {
    fn default() -> Self {
        Self { x: None, y: None }
    }
}

/// Layout specification for a macro operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutSpec {
    /// Type of layout combinator.
    #[serde(rename = "type")]
    pub layout_type: LayoutType,

    /// Ordered array of instance IDs to arrange.
    pub instances: Vec<u64>,

    /// Anchor point for alignment.
    #[serde(default)]
    pub anchor: LayoutAnchor,

    /// Gap between adjacent instances.
    #[serde(default = "default_gap")]
    pub gap: Rational,

    /// Optional origin position for the first instance.
    #[serde(default)]
    pub origin: LayoutOrigin,
}

/// Default gap value (zero).
fn default_gap() -> Rational {
    Rational::zero()
}

/// A layout macro operation in the event log.
///
/// This represents a higher-order layout constraint that expands into
/// multiple linear constraints. The expanded constraints are tracked
/// so they can be rolled back atomically.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LayoutMacroOperation {
    /// Monotonically increasing sequence number.
    pub seq: u64,

    /// ISO 8601 timestamp.
    pub timestamp: String,

    /// Layout specification.
    pub layout: LayoutSpec,

    /// IDs of constraints generated by macro expansion.
    /// These are added to the main operations list with source_scope
    /// pointing back to this macro.
    pub expanded_constraints: Vec<u64>,

    /// Optional: the natural language intent from the LLM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,

    /// The CLI command that triggered this operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// Record of an optimization run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OptimizationRun {
    /// Unique identifier for this optimization run.
    pub run_id: u64,

    /// ISO 8601 timestamp.
    pub timestamp: String,

    /// Number of constraints before optimization.
    pub constraints_before: u64,

    /// Number of constraints after optimization.
    pub constraints_after: u64,

    /// Mapping from original constraint IDs to optimized IDs (or None if deleted).
    pub id_mapping: Vec<(u64, Option<u64>)>,
}

impl VsBuildInfo {
    /// Get the next sequence number.
    pub fn next_seq(&self) -> u64 {
        self.operations.last().map(|op| op.seq + 1).unwrap_or(0)
    }

    /// Find the operation that added a specific constraint.
    pub fn find_add_operation(&self, constraint_id: u64) -> Option<&ConstraintOperation> {
        self.operations
            .iter()
            .find(|op| op.constraint.id == constraint_id && op.op_type == OperationType::Add)
    }

    /// Get all operations in reverse chronological order (newest first).
    pub fn reverse_scan(&self) -> impl Iterator<Item = &ConstraintOperation> {
        self.operations.iter().rev()
    }

    /// Find the original constraint ID before optimization.
    ///
    /// When `vsc optimize` merges or rewrites constraints, the post-optimization
    /// constraint IDs differ from the original IDs. This function traces backwards
    /// through the optimization history to find the original ID.
    ///
    /// ## Returns
    ///
    /// - `Some(original_id)` if the constraint was created by optimization
    /// - `None` if the constraint ID is already an original (no optimization history)
    pub fn find_original_id(&self, optimized_id: u64) -> Option<u64> {
        // Search optimization runs in reverse order (most recent first)
        for run in self.optimization_runs.iter().rev() {
            for (original, mapped) in &run.id_mapping {
                if let Some(mapped_id) = mapped {
                    if *mapped_id == optimized_id {
                        return Some(*original);
                    }
                }
            }
        }
        None
    }

    /// Restore pre-optimization state for a specific constraint.
    ///
    /// This reconstructs what the constraint looked like before the specified
    /// optimization run by replaying operations up to that point.
    pub fn restore_pre_optimization_state(
        &self,
        original_id: u64,
        before_run_id: u64,
    ) -> Option<Constraint> {
        // Find the operation that added or last modified this constraint
        // before the specified optimization run
        let mut result: Option<&Constraint> = None;

        for op in &self.operations {
            // Stop if we've reached operations from the target optimization run
            if let Some(run_id) = op.optimization_run_id {
                if run_id >= before_run_id {
                    break;
                }
            }

            // Track the latest state of this constraint
            if op.constraint.id == original_id {
                match op.op_type {
                    OperationType::Add | OperationType::Modify => {
                        result = Some(&op.constraint);
                    }
                    OperationType::Delete => {
                        result = None;
                    }
                    OperationType::Merge => {
                        // Merged into another; treat as deleted
                        result = None;
                    }
                    OperationType::LayoutMacro => {
                        // Layout macro operations are tracked separately
                        // Individual constraint state is unchanged by the macro marker
                    }
                }
            }
        }

        result.cloned()
    }

    /// Rollback a constraint removal and re-optimize.
    ///
    /// When an LLM issues `remove-constraint` on a constraint that was transformed
    /// by `vsc optimize`, this function:
    ///
    /// 1. Resolves the optimized ID to its original ID(s)
    /// 2. Records the delete operation in the ledger
    /// 3. Returns the set of original constraints that should be removed
    /// 4. The caller must then re-run optimization
    ///
    /// ## Ledger Integrity
    ///
    /// The ledger remains append-only. We record a DELETE operation with metadata
    /// indicating this is a rollback-triggered deletion.
    ///
    /// ## Returns
    ///
    /// `RollbackResult` containing:
    /// - `original_ids`: IDs of original constraints being removed
    /// - `affected_run_id`: The optimization run that needs to be invalidated
    /// - `reoptimize_required`: Whether re-optimization is needed
    pub fn rollback_apply_reoptimize(
        &mut self,
        target_id: u64,
        timestamp: String,
        intent: Option<String>,
    ) -> RollbackResult {
        let seq = self.next_seq();

        // Check if this ID came from optimization
        if let Some(original_id) = self.find_original_id(target_id) {
            // Find which optimization run created this mapping
            let affected_run_id = self
                .optimization_runs
                .iter()
                .rev()
                .find(|run| {
                    run.id_mapping
                        .iter()
                        .any(|(orig, mapped)| *orig == original_id && *mapped == Some(target_id))
                })
                .map(|run| run.run_id);

            // Record the delete operation with rollback metadata
            self.operations.push(ConstraintOperation {
                seq,
                timestamp,
                op_type: OperationType::Delete,
                constraint: Constraint {
                    id: target_id,
                    target: EntityId(0), // Placeholder; actual value not needed for delete
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: Rational::zero(),
                    },
                    priority: ConstraintPriority::Hard,
                    source_scope: None,
                },
                intent,
                command: Some(format!(
                    "remove-constraint {} (rollback from optimized ID {})",
                    original_id, target_id
                )),
                optimization_run_id: None,
            });

            RollbackResult {
                original_ids: vec![original_id],
                affected_run_id,
                reoptimize_required: true,
                message: format!(
                    "Constraint {} was created by optimization from original {}. \
                     Re-optimization required.",
                    target_id, original_id
                ),
            }
        } else {
            // Target ID is an original constraint (not from optimization)
            // Simple delete, no rollback needed
            let constraint = self
                .find_add_operation(target_id)
                .map(|op| op.constraint.clone())
                .unwrap_or_else(|| Constraint {
                    id: target_id,
                    target: EntityId(0),
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: Rational::zero(),
                    },
                    priority: ConstraintPriority::Hard,
                    source_scope: None,
                });

            self.operations.push(ConstraintOperation {
                seq,
                timestamp,
                op_type: OperationType::Delete,
                constraint,
                intent,
                command: Some(format!("remove-constraint {}", target_id)),
                optimization_run_id: None,
            });

            RollbackResult {
                original_ids: vec![target_id],
                affected_run_id: None,
                reoptimize_required: false,
                message: format!(
                    "Constraint {} removed directly (no optimization history).",
                    target_id
                ),
            }
        }
    }

    /// Check if a constraint ID exists in optimization mappings.
    pub fn is_optimized_id(&self, id: u64) -> bool {
        self.find_original_id(id).is_some()
    }

    // =========================================================================
    // Phase 10: Text Entity Management
    // =========================================================================

    /// Allocate a new entity ID and advance the counter.
    ///
    /// Returns the allocated ID. For TextEntity, this allocates 5 IDs
    /// (1 for text + 4 for corner control points).
    pub fn allocate_entity_id(&mut self) -> u64 {
        let id = self.next_entity_id;
        self.next_entity_id += 1;
        id
    }

    /// Allocate a block of entity IDs for a text entity (5 total).
    ///
    /// Returns the base ID. Corner points will be base+1, base+2, base+3, base+4.
    pub fn allocate_text_entity_ids(&mut self) -> u64 {
        let base = self.next_entity_id;
        self.next_entity_id += 5; // 1 text + 4 corners
        base
    }

    /// Find a text entity by ID.
    pub fn find_text_entity(&self, id: u64) -> Option<&TextEntityEntry> {
        self.text_entities.iter().find(|te| te.id.0 == id)
    }

    /// Find a text entity by ID (mutable).
    pub fn find_text_entity_mut(&mut self, id: u64) -> Option<&mut TextEntityEntry> {
        self.text_entities.iter_mut().find(|te| te.id.0 == id)
    }

    /// Add a new text entity.
    pub fn add_text_entity(&mut self, entry: TextEntityEntry) {
        self.text_entities.push(entry);
    }

    // =========================================================================
    // Phase 13: Layout Macro Management
    // =========================================================================

    /// Find a layout macro by sequence number.
    pub fn find_layout_macro(&self, seq: u64) -> Option<&LayoutMacroOperation> {
        self.layout_macros.iter().find(|lm| lm.seq == seq)
    }

    /// Add a layout macro operation.
    pub fn add_layout_macro(&mut self, macro_op: LayoutMacroOperation) {
        self.layout_macros.push(macro_op);
    }

    /// Rollback a layout macro and all its expanded constraints.
    ///
    /// When a layout macro is removed, all constraints it generated must be
    /// deleted atomically. This maintains the invariant that layout constraints
    /// are either fully present or fully absent.
    ///
    /// ## Returns
    /// - `Some(ids)`: Vector of constraint IDs that were deleted
    /// - `None`: Layout macro not found
    pub fn rollback_layout_macro(
        &mut self,
        macro_seq: u64,
        timestamp: String,
        intent: Option<String>,
    ) -> Option<Vec<u64>> {
        // Find the layout macro
        let macro_idx = self
            .layout_macros
            .iter()
            .position(|lm| lm.seq == macro_seq)?;
        let macro_op = self.layout_macros.remove(macro_idx);

        let deleted_ids = macro_op.expanded_constraints.clone();

        // Record delete operations for each expanded constraint
        for constraint_id in &deleted_ids {
            let seq = self.next_seq();

            // Find the original constraint to include in the delete record
            let original = self
                .find_add_operation(*constraint_id)
                .map(|op| op.constraint.clone());

            if let Some(constraint) = original {
                self.operations.push(ConstraintOperation {
                    seq,
                    timestamp: timestamp.clone(),
                    op_type: OperationType::Delete,
                    constraint,
                    intent: intent.clone(),
                    command: Some(format!(
                        "remove-constraint {} (rollback from layout_macro:{})",
                        constraint_id, macro_seq
                    )),
                    optimization_run_id: None,
                });
            }
        }

        Some(deleted_ids)
    }

    /// Check if a constraint ID was generated by a layout macro.
    ///
    /// Returns the macro sequence number if the constraint belongs to a macro.
    pub fn find_parent_layout_macro(&self, constraint_id: u64) -> Option<u64> {
        self.layout_macros
            .iter()
            .find(|lm| lm.expanded_constraints.contains(&constraint_id))
            .map(|lm| lm.seq)
    }
}

/// Result of a rollback operation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollbackResult {
    /// Original constraint IDs that are being removed.
    pub original_ids: Vec<u64>,

    /// The optimization run that needs to be invalidated (if any).
    pub affected_run_id: Option<u64>,

    /// Whether the caller must re-run `vsc optimize`.
    pub reoptimize_required: bool,

    /// Human-readable message explaining what happened.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_constraint(id: u64) -> Constraint {
        Constraint {
            id,
            target: EntityId(1),
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Const {
                value: Rational::from_int(100),
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        }
    }

    #[test]
    fn test_find_original_id_returns_none_for_non_optimized() {
        let buildinfo = VsBuildInfo::default();

        // No optimization runs, so no ID should be found
        assert_eq!(buildinfo.find_original_id(42), None);
    }

    #[test]
    fn test_find_original_id_traces_through_optimization() {
        let mut buildinfo = VsBuildInfo::default();

        // Add an optimization run that maps original ID 10 -> optimized ID 100
        buildinfo.optimization_runs.push(OptimizationRun {
            run_id: 1,
            timestamp: "2026-05-10T00:00:00Z".to_string(),
            constraints_before: 5,
            constraints_after: 3,
            id_mapping: vec![
                (10, Some(100)), // Original 10 became 100
                (11, Some(101)), // Original 11 became 101
                (12, None),      // Original 12 was deleted
            ],
        });

        // Should find original ID 10 for optimized ID 100
        assert_eq!(buildinfo.find_original_id(100), Some(10));
        assert_eq!(buildinfo.find_original_id(101), Some(11));

        // Non-existent optimized ID should return None
        assert_eq!(buildinfo.find_original_id(999), None);

        // Original IDs should not be found (they're not in the "mapped to" column)
        assert_eq!(buildinfo.find_original_id(10), None);
    }

    #[test]
    fn test_find_original_id_uses_most_recent_optimization() {
        let mut buildinfo = VsBuildInfo::default();

        // First optimization: 10 -> 100
        buildinfo.optimization_runs.push(OptimizationRun {
            run_id: 1,
            timestamp: "2026-05-10T00:00:00Z".to_string(),
            constraints_before: 5,
            constraints_after: 3,
            id_mapping: vec![(10, Some(100))],
        });

        // Second optimization: 100 -> 200 (chained)
        buildinfo.optimization_runs.push(OptimizationRun {
            run_id: 2,
            timestamp: "2026-05-10T01:00:00Z".to_string(),
            constraints_before: 3,
            constraints_after: 2,
            id_mapping: vec![(100, Some(200))],
        });

        // Looking up 200 should find 100 (from most recent run)
        assert_eq!(buildinfo.find_original_id(200), Some(100));
        // Looking up 100 should find 10 (from earlier run)
        assert_eq!(buildinfo.find_original_id(100), Some(10));
    }

    #[test]
    fn test_is_optimized_id() {
        let mut buildinfo = VsBuildInfo::default();

        buildinfo.optimization_runs.push(OptimizationRun {
            run_id: 1,
            timestamp: "2026-05-10T00:00:00Z".to_string(),
            constraints_before: 2,
            constraints_after: 1,
            id_mapping: vec![(10, Some(100))],
        });

        assert!(buildinfo.is_optimized_id(100));
        assert!(!buildinfo.is_optimized_id(10));
        assert!(!buildinfo.is_optimized_id(999));
    }

    #[test]
    fn test_rollback_on_original_constraint() {
        let mut buildinfo = VsBuildInfo::default();

        // Add an original constraint
        buildinfo.operations.push(ConstraintOperation {
            seq: 0,
            timestamp: "2026-05-10T00:00:00Z".to_string(),
            op_type: OperationType::Add,
            constraint: create_test_constraint(42),
            intent: Some("Add button position".to_string()),
            command: None,
            optimization_run_id: None,
        });

        // Rollback the original constraint (not optimized)
        let result =
            buildinfo.rollback_apply_reoptimize(42, "2026-05-10T01:00:00Z".to_string(), None);

        // Should NOT require reoptimization (was never optimized)
        assert!(!result.reoptimize_required);
        assert_eq!(result.original_ids, vec![42]);
        assert_eq!(result.affected_run_id, None);

        // Should have added a DELETE operation
        assert_eq!(buildinfo.operations.len(), 2);
        assert_eq!(buildinfo.operations[1].op_type, OperationType::Delete);
    }

    #[test]
    fn test_rollback_on_optimized_constraint() {
        let mut buildinfo = VsBuildInfo::default();

        // Add original constraint
        buildinfo.operations.push(ConstraintOperation {
            seq: 0,
            timestamp: "2026-05-10T00:00:00Z".to_string(),
            op_type: OperationType::Add,
            constraint: create_test_constraint(10),
            intent: None,
            command: None,
            optimization_run_id: None,
        });

        // Add optimization run
        buildinfo.optimization_runs.push(OptimizationRun {
            run_id: 1,
            timestamp: "2026-05-10T00:30:00Z".to_string(),
            constraints_before: 1,
            constraints_after: 1,
            id_mapping: vec![(10, Some(100))],
        });

        // Rollback the OPTIMIZED constraint ID (100)
        let result = buildinfo.rollback_apply_reoptimize(
            100,
            "2026-05-10T01:00:00Z".to_string(),
            Some("Remove button".to_string()),
        );

        // SHOULD require reoptimization
        assert!(result.reoptimize_required);
        assert_eq!(result.original_ids, vec![10]); // Returns original ID
        assert_eq!(result.affected_run_id, Some(1));

        // Should have added a DELETE operation with rollback metadata
        assert_eq!(buildinfo.operations.len(), 2);
        let delete_op = &buildinfo.operations[1];
        assert_eq!(delete_op.op_type, OperationType::Delete);
        assert!(delete_op.command.as_ref().unwrap().contains("rollback"));
    }

    #[test]
    fn test_restore_pre_optimization_state() {
        let mut buildinfo = VsBuildInfo::default();

        // Add constraint
        let original_constraint = create_test_constraint(10);
        buildinfo.operations.push(ConstraintOperation {
            seq: 0,
            timestamp: "2026-05-10T00:00:00Z".to_string(),
            op_type: OperationType::Add,
            constraint: original_constraint.clone(),
            intent: None,
            command: None,
            optimization_run_id: None,
        });

        // Modify constraint
        let modified_constraint = Constraint {
            id: 10,
            target: EntityId(1),
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Const {
                value: Rational::from_int(200),
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        };
        buildinfo.operations.push(ConstraintOperation {
            seq: 1,
            timestamp: "2026-05-10T00:10:00Z".to_string(),
            op_type: OperationType::Modify,
            constraint: modified_constraint.clone(),
            intent: None,
            command: None,
            optimization_run_id: None,
        });

        // Add optimization run at seq 2
        buildinfo.operations.push(ConstraintOperation {
            seq: 2,
            timestamp: "2026-05-10T00:20:00Z".to_string(),
            op_type: OperationType::Modify,
            constraint: Constraint {
                id: 10,
                target: EntityId(1),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const {
                    value: Rational::from_int(300),
                },
                priority: ConstraintPriority::Hard,
                source_scope: None,
            },
            intent: None,
            command: None,
            optimization_run_id: Some(1), // Part of optimization run 1
        });

        // Restore state before optimization run 1
        let restored = buildinfo.restore_pre_optimization_state(10, 1);

        assert!(restored.is_some());
        let restored = restored.unwrap();
        // Should return the modified constraint (value=200), not the optimized one (value=300)
        assert_eq!(
            restored.term,
            ConstraintTerm::Const {
                value: Rational::from_int(200)
            }
        );
    }

    #[test]
    fn test_restore_pre_optimization_state_deleted() {
        let mut buildinfo = VsBuildInfo::default();

        // Add then delete a constraint
        buildinfo.operations.push(ConstraintOperation {
            seq: 0,
            timestamp: "2026-05-10T00:00:00Z".to_string(),
            op_type: OperationType::Add,
            constraint: create_test_constraint(10),
            intent: None,
            command: None,
            optimization_run_id: None,
        });

        buildinfo.operations.push(ConstraintOperation {
            seq: 1,
            timestamp: "2026-05-10T00:10:00Z".to_string(),
            op_type: OperationType::Delete,
            constraint: create_test_constraint(10),
            intent: None,
            command: None,
            optimization_run_id: None,
        });

        // Restore should return None (was deleted)
        let restored = buildinfo.restore_pre_optimization_state(10, 99);
        assert!(restored.is_none());
    }

    #[test]
    fn test_vsbuildinfo_backward_compatibility_q_variables() {
        // Test that existing JSON without q_variables still deserializes correctly.
        // This ensures #[serde(default)] works as expected.
        let legacy_json = r#"{
            "version": 1,
            "operations": [],
            "optimization_runs": []
        }"#;

        let buildinfo: VsBuildInfo =
            serde_json::from_str(legacy_json).expect("legacy JSON should deserialize");

        // q_variables should default to empty vec
        assert!(buildinfo.q_variables.is_empty());
        assert_eq!(buildinfo.version, 1);
    }

    #[test]
    fn test_vsbuildinfo_with_q_variables() {
        use crate::ffi::{QValue, QVariable};
        use crate::solver::VarId;

        let mut buildinfo = VsBuildInfo::default();

        // Add a Q-variable declaration
        let q_var = QVariable {
            name: "input.pointer.x".to_string(),
            default: QValue::Int(0),
            target_var: VarId::new(EntityId(100), VectorComponent::X),
        };
        buildinfo.q_variables.push(q_var);

        // Serialize and deserialize
        let json = serde_json::to_string(&buildinfo).expect("serialize");
        let parsed: VsBuildInfo = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.q_variables.len(), 1);
        assert_eq!(parsed.q_variables[0].name, "input.pointer.x");
    }
}
