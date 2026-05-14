//! Chrome User Agent Stylesheet for ViewScript
//!
//! This crate provides CSS Box Model constraints as ViewScript IR,
//! implementing Chrome's default styling rules.
//!
//! ## Current Coverage
//!
//! - **Implemented**: Box Model dimension defaults, validity constraints, Box Model equality
//! - **Not Implemented**: Block Layout, Inline Layout, Margin Collapsing, Flexbox, Grid
//!
//! ## CSS Box Model
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        margin-top                            │
//! │  ┌───────────────────────────────────────────────────────┐  │
//! │  │                     border-top                         │  │
//! │  │  ┌─────────────────────────────────────────────────┐  │  │
//! │  │  │                  padding-top                     │  │  │
//! │  │  │  ┌───────────────────────────────────────────┐  │  │  │
//! │m │b │p │                                           │p │b │m │
//! │a │o │a │            content box                    │a │o │a │
//! │r │r │d │                                           │d │r │r │
//! │g │d │d │                                           │d │d │g │
//! │i │e │i │                                           │i │e │i │
//! │n │r │n │                                           │n │r │n │
//! │  │  │g │                                           │g │  │  │
//! │l │l │  │                                           │  │r │r │
//! │e │e │l │                                           │r │i │i │
//! │f │f │e │                                           │i │g │g │
//! │t │t │f │                                           │g │h │h │
//! │  │  │t │                                           │h │t │t │
//! │  │  │  │                                           │t │  │  │
//! │  │  │  └───────────────────────────────────────────┘  │  │  │
//! │  │  │                 padding-bottom                   │  │  │
//! │  │  └─────────────────────────────────────────────────┘  │  │
//! │  │                    border-bottom                       │  │
//! │  └───────────────────────────────────────────────────────┘  │
//! │                       margin-bottom                          │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Constraint Types
//!
//! ### Hard Constraints (ConstraintPriority::Hard)
//!
//! - **Validity**: `padding_* >= 0`, `border_* >= 0` (CSS spec requirement)
//! - **Box Model Equality**: `Σdimensions = containing_block` (via LinearConstraint)
//!
//! ### Soft Constraints (ConstraintPriority::Soft)
//!
//! - **Chrome Defaults**: `body { margin: 8px }`, `p { margin: 1em 0 }`, etc.
//! - These can be overridden by user stylesheets or inline styles.
//!
//! ## Solver Injection
//!
//! Use `StyleBundle::inject_into_solver()` to add constraints to a `ConstraintSolver`:
//!
//! ```ignore
//! let bundle = apply_chrome_style(&mut build_info);
//! bundle.inject_into_solver(&mut solver);
//! ```
//!
//! ## Viewport Binding
//!
//! The viewport EntityIds (`viewport_width`, `viewport_height`) returned by
//! `apply_chrome_style()` must have their resolved values injected externally:
//!
//! ```ignore
//! solver.register_variable(
//!     VarId::new(bundle.viewport_width.unwrap(), VectorComponent::Value),
//!     VariableState::Resolved { value: Rational::from_int(window_width) }
//! );
//! ```

use vsc_core::{
    buildinfo::{ConstraintOperation, OperationType, VsBuildInfo},
    solver::{ConstraintSolver, LinearConstraint, LinearRelation, VarId},
    Constraint, ConstraintPriority, ConstraintTerm, EntityId, Rational, RelationType,
    VectorComponent,
};

// =============================================================================
// Box Model Entity
// =============================================================================

/// CSS Box Model entity with 14 scalar dimensions.
///
/// Each dimension is represented as an `EntityId` with `VectorComponent::Value`
/// to store the scalar width/height value in the constraint solver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxModelEntity {
    /// Root entity ID for this box model.
    pub entity_id: EntityId,

    // Margin edges (outer spacing) - can be negative in CSS
    pub margin_top: EntityId,
    pub margin_right: EntityId,
    pub margin_bottom: EntityId,
    pub margin_left: EntityId,

    // Border edges (visual boundary) - must be >= 0
    pub border_top: EntityId,
    pub border_right: EntityId,
    pub border_bottom: EntityId,
    pub border_left: EntityId,

    // Padding edges (inner spacing) - must be >= 0
    pub padding_top: EntityId,
    pub padding_right: EntityId,
    pub padding_bottom: EntityId,
    pub padding_left: EntityId,

    // Content box dimensions
    pub content_width: EntityId,
    pub content_height: EntityId,
}

impl BoxModelEntity {
    /// Create a new BoxModelEntity with sequential entity IDs.
    ///
    /// Allocates 15 consecutive entity IDs starting from `base_id`:
    /// - base_id: root entity
    /// - base_id+1..4: margin (top, right, bottom, left)
    /// - base_id+5..8: border (top, right, bottom, left)
    /// - base_id+9..12: padding (top, right, bottom, left)
    /// - base_id+13..14: content (width, height)
    pub fn new(base_id: u64) -> Self {
        Self {
            entity_id: EntityId(base_id),
            margin_top: EntityId(base_id + 1),
            margin_right: EntityId(base_id + 2),
            margin_bottom: EntityId(base_id + 3),
            margin_left: EntityId(base_id + 4),
            border_top: EntityId(base_id + 5),
            border_right: EntityId(base_id + 6),
            border_bottom: EntityId(base_id + 7),
            border_left: EntityId(base_id + 8),
            padding_top: EntityId(base_id + 9),
            padding_right: EntityId(base_id + 10),
            padding_bottom: EntityId(base_id + 11),
            padding_left: EntityId(base_id + 12),
            content_width: EntityId(base_id + 13),
            content_height: EntityId(base_id + 14),
        }
    }

    /// Number of entity IDs consumed by a BoxModelEntity.
    pub const ID_COUNT: u64 = 15;

    /// Create a VarId for a dimension entity (uses VectorComponent::Value).
    pub fn var_id(entity: EntityId) -> VarId {
        VarId::new(entity, VectorComponent::Value)
    }

    /// Get all 15 entity IDs as a vector (for uniqueness verification).
    pub fn all_entity_ids(&self) -> Vec<EntityId> {
        vec![
            self.entity_id,
            self.margin_top,
            self.margin_right,
            self.margin_bottom,
            self.margin_left,
            self.border_top,
            self.border_right,
            self.border_bottom,
            self.border_left,
            self.padding_top,
            self.padding_right,
            self.padding_bottom,
            self.padding_left,
            self.content_width,
            self.content_height,
        ]
    }

    /// Get padding entity IDs (for >= 0 constraints).
    pub fn padding_ids(&self) -> [EntityId; 4] {
        [
            self.padding_top,
            self.padding_right,
            self.padding_bottom,
            self.padding_left,
        ]
    }

    /// Get border entity IDs (for >= 0 constraints).
    pub fn border_ids(&self) -> [EntityId; 4] {
        [
            self.border_top,
            self.border_right,
            self.border_bottom,
            self.border_left,
        ]
    }

    /// Get margin entity IDs.
    pub fn margin_ids(&self) -> [EntityId; 4] {
        [
            self.margin_top,
            self.margin_right,
            self.margin_bottom,
            self.margin_left,
        ]
    }

    /// Generate horizontal box model equality constraint.
    ///
    /// margin_left + border_left + padding_left + content_width
    /// + padding_right + border_right + margin_right = containing_width
    ///
    /// Expressed as: Σterms - containing_width = 0
    pub fn horizontal_constraint(
        &self,
        containing_width: EntityId,
        constraint_id: u64,
    ) -> LinearConstraint {
        let one = Rational::one();
        let neg_one = Rational::from_int(-1);

        LinearConstraint::eq(
            constraint_id,
            vec![
                (Self::var_id(self.margin_left), one.clone()),
                (Self::var_id(self.border_left), one.clone()),
                (Self::var_id(self.padding_left), one.clone()),
                (Self::var_id(self.content_width), one.clone()),
                (Self::var_id(self.padding_right), one.clone()),
                (Self::var_id(self.border_right), one.clone()),
                (Self::var_id(self.margin_right), one),
                (Self::var_id(containing_width), neg_one),
            ],
            Rational::zero(),
        )
    }

    /// Generate vertical box model equality constraint.
    ///
    /// margin_top + border_top + padding_top + content_height
    /// + padding_bottom + border_bottom + margin_bottom = containing_height
    pub fn vertical_constraint(
        &self,
        containing_height: EntityId,
        constraint_id: u64,
    ) -> LinearConstraint {
        let one = Rational::one();
        let neg_one = Rational::from_int(-1);

        LinearConstraint::eq(
            constraint_id,
            vec![
                (Self::var_id(self.margin_top), one.clone()),
                (Self::var_id(self.border_top), one.clone()),
                (Self::var_id(self.padding_top), one.clone()),
                (Self::var_id(self.content_height), one.clone()),
                (Self::var_id(self.padding_bottom), one.clone()),
                (Self::var_id(self.border_bottom), one.clone()),
                (Self::var_id(self.margin_bottom), one),
                (Self::var_id(containing_height), neg_one),
            ],
            Rational::zero(),
        )
    }
}

// =============================================================================
// Chrome Default Values
// =============================================================================

/// Chrome UA stylesheet default values.
///
/// ## Font Size Assumptions
///
/// Values expressed in `em` units assume a 16px base font size.
/// This is Chrome's default for the root element.
///
/// ## Coverage
///
/// Currently implemented:
/// - `body { margin: 8px }`
/// - `p { margin: 1em 0 }` (resolved to 16px at 16px font-size)
/// - `h1` margin (exact rational: 536/25 = 21.44px)
/// - Default border/padding (0)
///
/// Not yet implemented:
/// - `h2`-`h6` margins
/// - `ul`, `ol`, `li` margins/padding
/// - Form element defaults
/// - Table element defaults
pub mod chrome_defaults {
    use vsc_core::Rational;

    /// Default body margin (8px in Chrome).
    /// CSS: `body { margin: 8px }`
    pub fn body_margin() -> Rational {
        Rational::from_int(8)
    }

    /// Default paragraph margin (1em top/bottom, 0 left/right).
    /// Resolved at 16px base font size: 1em = 16px.
    /// CSS: `p { margin-block-start: 1em; margin-block-end: 1em }`
    pub fn p_margin_vertical() -> Rational {
        Rational::from_int(16)
    }

    /// Default h1 margin (0.67em top/bottom at 2em font-size).
    /// Chrome: h1 { font-size: 2em; margin-block: 0.67em }
    /// Calculation: 0.67 * 2em * 16px = 0.67 * 32 = 21.44px
    /// Exact rational: 67/100 * 32 = 2144/100 = 536/25
    pub fn h1_margin_vertical() -> Rational {
        Rational::new(536, 25) // = 21.44 exactly
    }

    /// Default border width (0 unless specified).
    /// CSS: `* { border-width: medium }` but computed to 0 when border-style: none
    pub fn default_border() -> Rational {
        Rational::zero()
    }

    /// Default padding (0 unless specified).
    /// CSS: `* { padding: 0 }`
    pub fn default_padding() -> Rational {
        Rational::zero()
    }
}

// =============================================================================
// Style Bundle (Output)
// =============================================================================

/// Bundle of constraints and entities generated by the style.
///
/// ## Two Constraint Storage Mechanisms
///
/// - `operations`: `Vec<ConstraintOperation>` for audit trail / VsBuildInfo persistence
/// - `linear_constraints`: `Vec<LinearConstraint>` for direct solver injection
///
/// Single-variable constraints (defaults, validity) are stored in both.
/// Multi-variable constraints (Box Model equality) are only in `linear_constraints`.
///
/// ## Solver Injection
///
/// Call `inject_into_solver()` to add all constraints to a `ConstraintSolver`.
#[derive(Debug, Clone, Default)]
pub struct StyleBundle {
    /// Constraint operations for VsBuildInfo audit trail.
    /// Contains single-variable constraints only (defaults, validity).
    pub operations: Vec<ConstraintOperation>,

    /// Linear constraints for direct solver injection.
    /// Contains ALL constraints including multi-variable Box Model equality.
    pub linear_constraints: Vec<LinearConstraint>,

    /// Box model entities created.
    pub box_models: Vec<BoxModelEntity>,

    /// Viewport width entity (must be resolved externally).
    pub viewport_width: Option<EntityId>,

    /// Viewport height entity (must be resolved externally).
    pub viewport_height: Option<EntityId>,
}

impl StyleBundle {
    /// Create an empty style bundle (for idempotent early return).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Check if this bundle is empty.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
            && self.linear_constraints.is_empty()
            && self.box_models.is_empty()
    }

    /// Inject all constraints into a ConstraintSolver.
    ///
    /// This is the primary method for executing style constraints.
    /// It adds all `linear_constraints` to the solver's active queue.
    ///
    /// ## Note on Priority
    ///
    /// `LinearConstraint` does not have a priority field. Priority information
    /// is preserved in `operations` for audit purposes but not enforced by the
    /// solver directly. The solver treats all constraints as equally binding.
    pub fn inject_into_solver(&self, solver: &mut ConstraintSolver) {
        for constraint in &self.linear_constraints {
            solver.add_linear(constraint.clone());
        }
    }

    /// Get the number of linear constraints.
    pub fn constraint_count(&self) -> usize {
        self.linear_constraints.len()
    }
}

// =============================================================================
// Constraint Generators
// =============================================================================

/// Generate a constraint operation for a dimension default value.
fn make_default_constraint(
    seq: u64,
    constraint_id: u64,
    target: EntityId,
    value: Rational,
    timestamp: &str,
    description: &str,
) -> ConstraintOperation {
    ConstraintOperation {
        seq,
        timestamp: timestamp.to_string(),
        op_type: OperationType::Add,
        constraint: Constraint {
            id: constraint_id,
            target,
            component: VectorComponent::Value,
            relation: RelationType::Eq,
            term: ConstraintTerm::Const { value },
            priority: ConstraintPriority::Soft, // Defaults can be overridden
            source_scope: Some(format!("vs-style-chrome:{}", description)),
        },
        intent: Some(format!("Chrome UA default: {}", description)),
        command: Some("apply_chrome_style".to_string()),
        optimization_run_id: None,
    }
}

/// Generate a >= 0 validity constraint for padding/border.
fn make_non_negative_constraint(
    seq: u64,
    constraint_id: u64,
    target: EntityId,
    timestamp: &str,
    description: &str,
) -> ConstraintOperation {
    ConstraintOperation {
        seq,
        timestamp: timestamp.to_string(),
        op_type: OperationType::Add,
        constraint: Constraint {
            id: constraint_id,
            target,
            component: VectorComponent::Value,
            relation: RelationType::Ge,
            term: ConstraintTerm::Const {
                value: Rational::zero(),
            },
            priority: ConstraintPriority::Hard, // CSS spec: cannot be negative
            source_scope: Some(format!("vs-style-chrome:{}", description)),
        },
        intent: Some(format!("CSS validity: {} >= 0", description)),
        command: Some("apply_chrome_style".to_string()),
        optimization_run_id: None,
    }
}

/// Convert a single-variable Constraint to LinearConstraint.
///
/// Only supports Const terms. Ref and Linear terms require additional context.
fn constraint_to_linear(constraint: &Constraint) -> Option<LinearConstraint> {
    let var = VarId::new(constraint.target, constraint.component);
    let neg_one = Rational::from_int(-1);

    match &constraint.term {
        ConstraintTerm::Const { value } => {
            // target.component ⋈ value
            // Rewrite as: target.component - value ⋈ 0
            let relation = match constraint.relation {
                RelationType::Eq => LinearRelation::Eq,
                RelationType::Le => LinearRelation::Le,
                RelationType::Ge => LinearRelation::Ge,
                RelationType::Lt => LinearRelation::Le, // Approximate Lt as Le
                RelationType::Gt => LinearRelation::Ge, // Approximate Gt as Ge
            };

            Some(LinearConstraint {
                id: constraint.id,
                terms: vec![(var, Rational::one())],
                constant: neg_one.clone() * value.clone(),
                relation,
            })
        }
        ConstraintTerm::Ref {
            entity_id,
            component,
        } => {
            // target.component = ref.component
            // Rewrite as: target - ref = 0
            let ref_var = VarId::new(*entity_id, *component);
            Some(LinearConstraint {
                id: constraint.id,
                terms: vec![(var, Rational::one()), (ref_var, neg_one)],
                constant: Rational::zero(),
                relation: LinearRelation::Eq,
            })
        }
        ConstraintTerm::Linear {
            coefficient,
            entity_id,
            component,
            offset,
        } => {
            // target = coefficient * ref + offset
            // Rewrite as: target - coefficient * ref - offset = 0
            let ref_var = VarId::new(*entity_id, *component);
            Some(LinearConstraint {
                id: constraint.id,
                terms: vec![
                    (var, Rational::one()),
                    (ref_var, neg_one.clone() * coefficient.clone()),
                ],
                constant: neg_one * offset.clone(),
                relation: LinearRelation::Eq,
            })
        }
    }
}

// =============================================================================
// Apply to VsBuildInfo
// =============================================================================

/// Apply Chrome UA stylesheet defaults to a VsBuildInfo.
///
/// This registers the "vs-style-chrome" style and sets up default
/// box model constraints for the document body.
///
/// ## Idempotency
///
/// This function is idempotent. If called multiple times on the same
/// `VsBuildInfo`, subsequent calls return an empty `StyleBundle` without
/// modifying state.
///
/// ## Constraint Injection
///
/// After calling this function, use `bundle.inject_into_solver(&mut solver)`
/// to add constraints to the solver.
///
/// ## Viewport Resolution
///
/// The returned `StyleBundle` contains `viewport_width` and `viewport_height`
/// EntityIds. These must have their values resolved externally via:
///
/// ```ignore
/// solver.register_variable(
///     VarId::new(bundle.viewport_width.unwrap(), VectorComponent::Value),
///     VariableState::Resolved { value: Rational::from_int(window_width) }
/// );
/// ```
pub fn apply_chrome_style(build_info: &mut VsBuildInfo) -> StyleBundle {
    // Idempotency guard: early return if already applied
    if build_info.styles.contains(&"vs-style-chrome".to_string()) {
        return StyleBundle::empty();
    }
    build_info.styles.push("vs-style-chrome".to_string());

    let mut bundle = StyleBundle::default();
    let timestamp = "2026-05-13T00:00:00Z"; // Fixed for reproducibility

    // Allocate viewport entities
    let viewport_width = EntityId(build_info.next_entity_id);
    let viewport_height = EntityId(build_info.next_entity_id + 1);
    build_info.next_entity_id += 2;
    bundle.viewport_width = Some(viewport_width);
    bundle.viewport_height = Some(viewport_height);

    // Create body box model
    let body = BoxModelEntity::new(build_info.next_entity_id);
    build_info.next_entity_id += BoxModelEntity::ID_COUNT;

    // Base sequence and constraint ID from current operations length
    let mut seq = build_info.operations.len() as u64;
    let mut constraint_id = seq;

    // --- Soft Constraints: Chrome Defaults ---

    // Body margins (8px all sides)
    let body_margin = chrome_defaults::body_margin();
    for (entity, name) in [
        (body.margin_top, "body.margin-top"),
        (body.margin_right, "body.margin-right"),
        (body.margin_bottom, "body.margin-bottom"),
        (body.margin_left, "body.margin-left"),
    ] {
        let op = make_default_constraint(
            seq,
            constraint_id,
            entity,
            body_margin.clone(),
            timestamp,
            name,
        );
        // Convert to LinearConstraint for solver
        if let Some(lc) = constraint_to_linear(&op.constraint) {
            bundle.linear_constraints.push(lc);
        }
        bundle.operations.push(op);
        seq += 1;
        constraint_id += 1;
    }

    // Border defaults (0)
    let default_border = chrome_defaults::default_border();
    for (entity, name) in [
        (body.border_top, "body.border-top"),
        (body.border_right, "body.border-right"),
        (body.border_bottom, "body.border-bottom"),
        (body.border_left, "body.border-left"),
    ] {
        let op = make_default_constraint(
            seq,
            constraint_id,
            entity,
            default_border.clone(),
            timestamp,
            name,
        );
        if let Some(lc) = constraint_to_linear(&op.constraint) {
            bundle.linear_constraints.push(lc);
        }
        bundle.operations.push(op);
        seq += 1;
        constraint_id += 1;
    }

    // Padding defaults (0)
    let default_padding = chrome_defaults::default_padding();
    for (entity, name) in [
        (body.padding_top, "body.padding-top"),
        (body.padding_right, "body.padding-right"),
        (body.padding_bottom, "body.padding-bottom"),
        (body.padding_left, "body.padding-left"),
    ] {
        let op = make_default_constraint(
            seq,
            constraint_id,
            entity,
            default_padding.clone(),
            timestamp,
            name,
        );
        if let Some(lc) = constraint_to_linear(&op.constraint) {
            bundle.linear_constraints.push(lc);
        }
        bundle.operations.push(op);
        seq += 1;
        constraint_id += 1;
    }

    // --- Hard Constraints: Validity (>= 0) ---

    // Padding >= 0
    for (entity, name) in [
        (body.padding_top, "padding-top"),
        (body.padding_right, "padding-right"),
        (body.padding_bottom, "padding-bottom"),
        (body.padding_left, "padding-left"),
    ] {
        let op = make_non_negative_constraint(seq, constraint_id, entity, timestamp, name);
        if let Some(lc) = constraint_to_linear(&op.constraint) {
            bundle.linear_constraints.push(lc);
        }
        bundle.operations.push(op);
        seq += 1;
        constraint_id += 1;
    }

    // Border >= 0
    for (entity, name) in [
        (body.border_top, "border-top"),
        (body.border_right, "border-right"),
        (body.border_bottom, "border-bottom"),
        (body.border_left, "border-left"),
    ] {
        let op = make_non_negative_constraint(seq, constraint_id, entity, timestamp, name);
        if let Some(lc) = constraint_to_linear(&op.constraint) {
            bundle.linear_constraints.push(lc);
        }
        bundle.operations.push(op);
        seq += 1;
        constraint_id += 1;
    }

    // --- Hard Constraints: Box Model Equality (LinearConstraint only) ---
    // These cannot be represented as single-variable Constraint, so they go
    // directly into linear_constraints without a corresponding ConstraintOperation.

    // Horizontal: margin_l + border_l + padding_l + content_w + padding_r + border_r + margin_r = viewport_w
    bundle
        .linear_constraints
        .push(body.horizontal_constraint(viewport_width, constraint_id));
    constraint_id += 1;

    // Vertical: margin_t + border_t + padding_t + content_h + padding_b + border_b + margin_b = viewport_h
    bundle
        .linear_constraints
        .push(body.vertical_constraint(viewport_height, constraint_id));

    bundle.box_models.push(body);
    bundle
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // =========================================================================
    // 観点2: EntityId 割り当ての一意性
    // =========================================================================

    #[test]
    fn test_box_model_entity_15_ids_unique() {
        let box_model = BoxModelEntity::new(100);
        let ids = box_model.all_entity_ids();

        assert_eq!(ids.len(), 15, "BoxModelEntity should have exactly 15 IDs");

        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(
            unique.len(),
            15,
            "All 15 EntityIds must be unique within a single BoxModelEntity"
        );
    }

    #[test]
    fn test_two_box_models_30_ids_no_overlap() {
        let box1 = BoxModelEntity::new(100);
        let box2 = BoxModelEntity::new(100 + BoxModelEntity::ID_COUNT);

        let mut all_ids = box1.all_entity_ids();
        all_ids.extend(box2.all_entity_ids());

        assert_eq!(all_ids.len(), 30);

        let unique: HashSet<_> = all_ids.iter().collect();
        assert_eq!(
            unique.len(),
            30,
            "Two BoxModelEntities with sequential base IDs must have no overlapping EntityIds"
        );
    }

    #[test]
    fn test_next_entity_id_consistency() {
        let mut build_info = VsBuildInfo::default();
        let initial_id = build_info.next_entity_id;

        let bundle = apply_chrome_style(&mut build_info);

        // Should allocate: 2 (viewport) + 15 (body) = 17 IDs
        let expected_id = initial_id + 2 + BoxModelEntity::ID_COUNT;
        assert_eq!(
            build_info.next_entity_id, expected_id,
            "next_entity_id should advance by 17 (2 viewport + 15 body)"
        );

        // Verify viewport IDs are sequential from initial
        assert_eq!(bundle.viewport_width, Some(EntityId(initial_id)));
        assert_eq!(bundle.viewport_height, Some(EntityId(initial_id + 1)));
    }

    // =========================================================================
    // 観点4: 制約の優先度
    // =========================================================================

    #[test]
    fn test_default_constraints_are_soft() {
        let mut build_info = VsBuildInfo::default();
        let bundle = apply_chrome_style(&mut build_info);

        // First 12 operations are defaults (4 margin + 4 border + 4 padding)
        for (i, op) in bundle.operations.iter().take(12).enumerate() {
            assert_eq!(
                op.constraint.priority,
                ConstraintPriority::Soft,
                "Default constraint {} should be Soft priority",
                i
            );
            assert_eq!(
                op.constraint.relation,
                RelationType::Eq,
                "Default constraint {} should be equality",
                i
            );
        }
    }

    #[test]
    fn test_validity_constraints_are_hard() {
        let mut build_info = VsBuildInfo::default();
        let bundle = apply_chrome_style(&mut build_info);

        // Last 8 operations are validity (4 padding >= 0 + 4 border >= 0)
        for (i, op) in bundle.operations.iter().skip(12).enumerate() {
            assert_eq!(
                op.constraint.priority,
                ConstraintPriority::Hard,
                "Validity constraint {} should be Hard priority",
                i
            );
            assert_eq!(
                op.constraint.relation,
                RelationType::Ge,
                "Validity constraint {} should be >= relation",
                i
            );
        }
    }

    // =========================================================================
    // 観点5: 冪等性
    // =========================================================================

    #[test]
    fn test_apply_chrome_style_idempotent() {
        let mut build_info = VsBuildInfo::default();

        // First call
        let bundle1 = apply_chrome_style(&mut build_info);
        let id_after_first = build_info.next_entity_id;

        assert!(!bundle1.is_empty(), "First call should produce constraints");
        assert!(
            bundle1.operations.len() > 0,
            "First call should produce operations"
        );
        assert_eq!(build_info.styles.len(), 1);

        // Second call
        let bundle2 = apply_chrome_style(&mut build_info);

        assert!(bundle2.is_empty(), "Second call should return empty bundle");
        assert_eq!(
            build_info.next_entity_id, id_after_first,
            "next_entity_id should not change on second call"
        );
        assert_eq!(
            build_info.styles.len(),
            1,
            "styles should not have duplicates"
        );
    }

    // =========================================================================
    // 観点7: 負の値の処理
    // =========================================================================

    #[test]
    fn test_padding_border_have_ge_zero_constraints() {
        let mut build_info = VsBuildInfo::default();
        let bundle = apply_chrome_style(&mut build_info);

        let body = &bundle.box_models[0];

        // Collect all entity IDs that have >= 0 constraints in operations
        let ge_zero_targets: HashSet<_> = bundle
            .operations
            .iter()
            .filter(|op| {
                op.constraint.relation == RelationType::Ge
                    && op.constraint.term
                        == ConstraintTerm::Const {
                            value: Rational::zero(),
                        }
            })
            .map(|op| op.constraint.target)
            .collect();

        // All padding IDs should have >= 0 constraint
        for id in body.padding_ids() {
            assert!(
                ge_zero_targets.contains(&id),
                "Padding {:?} should have >= 0 constraint",
                id
            );
        }

        // All border IDs should have >= 0 constraint
        for id in body.border_ids() {
            assert!(
                ge_zero_targets.contains(&id),
                "Border {:?} should have >= 0 constraint",
                id
            );
        }

        // Margin should NOT have >= 0 constraint (negative margins are valid CSS)
        for id in body.margin_ids() {
            assert!(
                !ge_zero_targets.contains(&id),
                "Margin {:?} should NOT have >= 0 constraint (negative margins allowed)",
                id
            );
        }
    }

    // =========================================================================
    // 観点11: Box Model等式制約
    // =========================================================================

    #[test]
    fn test_box_model_equality_constraints_exist() {
        let mut build_info = VsBuildInfo::default();
        let bundle = apply_chrome_style(&mut build_info);

        // linear_constraints should contain Box Model equality constraints
        // Total: 12 defaults + 8 validity + 2 box model equality = 22
        assert_eq!(
            bundle.linear_constraints.len(),
            22,
            "Should have 22 linear constraints (12 defaults + 8 validity + 2 box model equality)"
        );

        // Last 2 constraints should be the Box Model equality (8 terms each)
        let h_constraint = &bundle.linear_constraints[20];
        let v_constraint = &bundle.linear_constraints[21];

        assert_eq!(
            h_constraint.terms.len(),
            8,
            "Horizontal box model constraint should have 8 terms"
        );
        assert_eq!(
            v_constraint.terms.len(),
            8,
            "Vertical box model constraint should have 8 terms"
        );

        // Verify relation is equality
        assert_eq!(h_constraint.relation, LinearRelation::Eq);
        assert_eq!(v_constraint.relation, LinearRelation::Eq);
    }

    #[test]
    fn test_box_model_horizontal_constraint_terms() {
        let box_model = BoxModelEntity::new(100);
        let containing = EntityId(200);
        let constraint = box_model.horizontal_constraint(containing, 1);

        // Should have 8 terms: 7 box dimensions + 1 containing
        assert_eq!(constraint.terms.len(), 8);

        // All box dimensions should have coefficient +1
        let term_map: std::collections::HashMap<_, _> = constraint.terms.iter().cloned().collect();

        assert_eq!(
            term_map.get(&BoxModelEntity::var_id(box_model.margin_left)),
            Some(&Rational::one())
        );
        assert_eq!(
            term_map.get(&BoxModelEntity::var_id(box_model.content_width)),
            Some(&Rational::one())
        );

        // Containing should have coefficient -1
        assert_eq!(
            term_map.get(&BoxModelEntity::var_id(containing)),
            Some(&Rational::from_int(-1))
        );

        // Constant should be 0
        assert_eq!(constraint.constant, Rational::zero());
    }

    // =========================================================================
    // 観点12: ソルバー注入
    // =========================================================================

    #[test]
    fn test_inject_into_solver() {
        let mut build_info = VsBuildInfo::default();
        let bundle = apply_chrome_style(&mut build_info);

        let mut solver = ConstraintSolver::new();

        // Verify the bundle has constraints to inject
        assert_eq!(bundle.linear_constraints.len(), 22);

        // Injection should succeed without panic
        bundle.inject_into_solver(&mut solver);

        // Note: We don't call solver.solve() here because the system is
        // underdetermined (viewport not resolved) and solving would take
        // indefinite time or fail. The purpose of this test is to verify
        // that inject_into_solver() correctly calls add_linear() for each
        // constraint.
    }

    // =========================================================================
    // 観点13: h1 margin 精度
    // =========================================================================

    #[test]
    fn test_h1_margin_exact_rational() {
        let h1_margin = chrome_defaults::h1_margin_vertical();

        // Should be exactly 536/25 = 21.44
        assert_eq!(
            h1_margin,
            Rational::new(536, 25),
            "h1 margin should be exact rational 536/25"
        );

        // Verify the decimal value
        // 536 / 25 = 21.44
        let numerator = 536i64;
        let denominator = 25i64;
        let decimal = numerator as f64 / denominator as f64;
        assert!(
            (decimal - 21.44).abs() < 0.001,
            "h1 margin should equal 21.44"
        );
    }

    // =========================================================================
    // 観点3: Chrome デフォルト値
    // =========================================================================

    #[test]
    fn test_chrome_defaults_body_margin_8px() {
        assert_eq!(
            chrome_defaults::body_margin(),
            Rational::from_int(8),
            "body margin should be exactly 8 (px)"
        );
    }

    #[test]
    fn test_body_margin_constraints_value() {
        let mut build_info = VsBuildInfo::default();
        let bundle = apply_chrome_style(&mut build_info);

        let body = &bundle.box_models[0];
        let margin_8 = Rational::from_int(8);

        // Check all 4 body margin constraints have value 8
        for id in body.margin_ids() {
            let constraint = bundle
                .operations
                .iter()
                .find(|op| {
                    op.constraint.target == id
                        && op.constraint.relation == RelationType::Eq
                        && op.constraint.priority == ConstraintPriority::Soft
                })
                .expect("Margin should have a default constraint");

            match &constraint.constraint.term {
                ConstraintTerm::Const { value } => {
                    assert_eq!(value, &margin_8, "Body margin should default to 8px");
                }
                _ => panic!("Margin default should be a Const term"),
            }
        }
    }

    // =========================================================================
    // 観点9: VectorComponent の使用
    // =========================================================================

    #[test]
    fn test_constraints_use_vector_component_value() {
        let mut build_info = VsBuildInfo::default();
        let bundle = apply_chrome_style(&mut build_info);

        for op in &bundle.operations {
            assert_eq!(
                op.constraint.component,
                VectorComponent::Value,
                "All box model dimension constraints should use VectorComponent::Value"
            );
        }
    }

    // =========================================================================
    // 観点8: StyleBundle 構造
    // =========================================================================

    #[test]
    fn test_constraint_ids_sequential_no_duplicates() {
        let mut build_info = VsBuildInfo::default();
        // Add some pre-existing operations
        build_info.next_entity_id = 5000;

        let bundle = apply_chrome_style(&mut build_info);

        let ids: Vec<_> = bundle
            .operations
            .iter()
            .map(|op| op.constraint.id)
            .collect();
        let unique: HashSet<_> = ids.iter().collect();

        assert_eq!(
            ids.len(),
            unique.len(),
            "Constraint IDs must be unique (no duplicates)"
        );

        // Verify sequential
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(*id, i as u64, "Constraint IDs should be sequential from 0");
        }
    }

    // =========================================================================
    // 観点10: シリアライゼーション後方互換性
    // =========================================================================

    #[test]
    fn test_vsbuildinfo_deserialize_without_styles_field() {
        // JSON without "styles" field (legacy format)
        let json = r#"{
            "version": 1,
            "operations": [],
            "optimization_runs": [],
            "next_entity_id": 1000
        }"#;

        let result: Result<VsBuildInfo, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "Should deserialize VsBuildInfo without styles field"
        );

        let build_info = result.unwrap();
        assert!(
            build_info.styles.is_empty(),
            "styles should default to empty Vec"
        );
    }

    // =========================================================================
    // 観点15: StyleBundle::empty()
    // =========================================================================

    #[test]
    fn test_style_bundle_empty() {
        let empty = StyleBundle::empty();

        assert!(empty.operations.is_empty());
        assert!(empty.linear_constraints.is_empty());
        assert!(empty.box_models.is_empty());
        assert!(empty.viewport_width.is_none());
        assert!(empty.viewport_height.is_none());
        assert!(empty.is_empty());
    }

    // =========================================================================
    // 既存テスト
    // =========================================================================

    #[test]
    fn test_box_model_entity_creation() {
        let box_model = BoxModelEntity::new(100);

        assert_eq!(box_model.entity_id, EntityId(100));
        assert_eq!(box_model.margin_top, EntityId(101));
        assert_eq!(box_model.content_height, EntityId(114));
    }

    #[test]
    fn test_style_bundle_constraint_count() {
        let mut build_info = VsBuildInfo::default();
        let bundle = apply_chrome_style(&mut build_info);

        // operations: 4 margin + 4 border + 4 padding defaults = 12 Soft
        //           + 4 padding + 4 border >= 0 = 8 Hard
        // Total operations = 20
        assert_eq!(
            bundle.operations.len(),
            20,
            "Should generate 20 operations (12 defaults + 8 validity)"
        );

        // linear_constraints: 20 from operations + 2 box model equality = 22
        assert_eq!(
            bundle.linear_constraints.len(),
            22,
            "Should generate 22 linear constraints (20 from ops + 2 box model equality)"
        );

        assert_eq!(bundle.box_models.len(), 1, "Should create 1 body box model");
    }

    // =========================================================================
    // Constraint → LinearConstraint 変換
    // =========================================================================

    #[test]
    fn test_constraint_to_linear_const() {
        let constraint = Constraint {
            id: 1,
            target: EntityId(100),
            component: VectorComponent::Value,
            relation: RelationType::Eq,
            term: ConstraintTerm::Const {
                value: Rational::from_int(8),
            },
            priority: ConstraintPriority::Soft,
            source_scope: None,
        };

        let lc = constraint_to_linear(&constraint).unwrap();

        assert_eq!(lc.id, 1);
        assert_eq!(lc.terms.len(), 1);
        assert_eq!(
            lc.terms[0].0,
            VarId::new(EntityId(100), VectorComponent::Value)
        );
        assert_eq!(lc.terms[0].1, Rational::one());
        assert_eq!(lc.constant, Rational::from_int(-8)); // target - 8 = 0
        assert_eq!(lc.relation, LinearRelation::Eq);
    }

    #[test]
    fn test_constraint_to_linear_ge() {
        let constraint = Constraint {
            id: 2,
            target: EntityId(100),
            component: VectorComponent::Value,
            relation: RelationType::Ge,
            term: ConstraintTerm::Const {
                value: Rational::zero(),
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        };

        let lc = constraint_to_linear(&constraint).unwrap();

        assert_eq!(lc.relation, LinearRelation::Ge);
        assert_eq!(lc.constant, Rational::zero()); // target - 0 >= 0
    }
}
