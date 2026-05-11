//! Component System for Modular Constraint Graphs (Phase 11)
//!
//! This module provides the infrastructure for importing, instantiating, and
//! resolving namespaced constraints from `.vs` component files.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                     Component Import Flow                               │
//! └─────────────────────────────────────────────────────────────────────────┘
//!
//!   ┌──────────────┐     ┌──────────────────┐     ┌──────────────────────┐
//!   │ button.vs    │     │  Namespace       │     │  Expanded to         │
//!   │ TL: (0, 0)   │────▶│  Resolution      │────▶│  btn_1::TL: (0, 0)   │
//!   │ TR: (W, 0)   │     │  (inst_prefix)   │     │  btn_1::TR: (W, 0)   │
//!   └──────────────┘     └──────────────────┘     └──────────────────────┘
//!          │
//!          ▼
//!   ┌──────────────┐     ┌──────────────────┐     ┌──────────────────────┐
//!   │ Parent Scope │     │  Constraint      │     │  Hard: btn_1::TL.x=0 │
//!   │ btn.TL.x = 0 │────▶│  Merge +         │────▶│  Soft: (shadowed)    │
//!   │              │     │  Shadow Check    │     │                      │
//!   └──────────────┘     └──────────────────┘     └──────────────────────┘
//! ```
//!
//! ## Key Concepts
//!
//! - **ComponentDefinition**: A template loaded from a `.vs` file
//! - **ComponentInstance**: A concrete instantiation with unique namespace
//! - **NamespaceResolver**: Maps local IDs to globally unique IDs

use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A component definition loaded from a `.vs` file.
///
/// This is the template from which instances are created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentDefinition {
    /// Unique name of this component (e.g., "RoundedRect", "Button").
    pub name: String,

    /// Version for compatibility checking.
    pub version: String,

    /// Control points defined by this component (local IDs).
    pub control_points: Vec<ComponentControlPoint>,

    /// Constraints defined by this component.
    pub constraints: Vec<ComponentConstraint>,

    /// Exported ports: named control points that can be referenced externally.
    /// Key: port name (e.g., "TL", "TR"), Value: local EntityId.
    pub exports: HashMap<String, u64>,

    /// Parameters that can be customized during instantiation.
    pub parameters: Vec<ComponentParameter>,
}

/// A control point definition within a component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentControlPoint {
    /// Local ID within the component (before namespace resolution).
    pub local_id: u64,
    /// Semantic name (e.g., "TL", "TR", "center").
    pub name: String,
    /// Role: anchor or handle.
    pub role: ControlPointRole,
    /// Initial position (may reference parameters).
    pub initial_x: ComponentValue,
    pub initial_y: ComponentValue,
}

/// A constraint definition within a component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentConstraint {
    /// Local constraint ID.
    pub local_id: u64,
    /// Target local entity ID.
    pub target_local_id: u64,
    /// Component being constrained.
    pub component: VectorComponent,
    /// Relation type.
    pub relation: RelationType,
    /// Term (may reference local IDs or parameters).
    pub term: ComponentTerm,
    /// Priority for shadowing.
    pub priority: ConstraintPriority,
    /// Optional description.
    pub description: Option<String>,
}

/// A value that may be a constant or a parameter reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ComponentValue {
    /// Constant rational value.
    Const { value: Rational },
    /// Reference to a parameter.
    Param { name: String },
    /// Reference to another control point's component.
    Ref { local_id: u64, component: VectorComponent },
}

/// A term in a component constraint (may reference local IDs).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ComponentTerm {
    /// Constant value.
    Const { value: Rational },
    /// Reference to local entity.
    Ref { local_id: u64, component: VectorComponent },
    /// Linear combination with local entity reference.
    Linear {
        coefficient: Rational,
        local_id: u64,
        component: VectorComponent,
        offset: Rational,
    },
    /// Reference to a parameter.
    Param { name: String },
}

/// A customizable parameter of a component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentParameter {
    /// Parameter name (e.g., "width", "corner_radius").
    pub name: String,
    /// Default value.
    pub default: Rational,
    /// Whether this parameter can be negative.
    pub allow_negative: bool,
    /// Description for documentation.
    pub description: Option<String>,
}

/// An instantiated component with resolved namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInstance {
    /// Unique instance ID.
    pub instance_id: u64,
    /// Name of the component definition.
    pub component_name: String,
    /// Namespace prefix (e.g., "inst_42").
    pub namespace: String,
    /// Mapping from local ID to global EntityId.
    pub id_mapping: HashMap<u64, EntityId>,
    /// Resolved parameter values.
    pub parameters: HashMap<String, Rational>,
    /// Constraints after namespace resolution (ready for solver).
    pub resolved_constraints: Vec<Constraint>,
    /// Control points after namespace resolution.
    pub resolved_control_points: Vec<ControlPoint>,
}

/// Namespace resolver for component instantiation.
pub struct NamespaceResolver {
    /// Current instance counter for generating unique namespaces.
    next_instance_id: u64,
    /// Global entity ID counter.
    next_entity_id: u64,
    /// Global constraint ID counter.
    next_constraint_id: u64,
}

impl Default for NamespaceResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl NamespaceResolver {
    /// Create a new namespace resolver.
    pub fn new() -> Self {
        Self {
            next_instance_id: 1,
            next_entity_id: 10000, // Reserve lower IDs for manual/root scope
            next_constraint_id: 10000,
        }
    }

    /// Create with specific starting IDs (for integration with existing buildinfo).
    pub fn with_offsets(entity_offset: u64, constraint_offset: u64) -> Self {
        Self {
            next_instance_id: 1,
            next_entity_id: entity_offset,
            next_constraint_id: constraint_offset,
        }
    }

    /// Instantiate a component definition.
    ///
    /// Returns a ComponentInstance with all IDs resolved to globally unique values.
    pub fn instantiate(
        &mut self,
        definition: &ComponentDefinition,
        parameters: HashMap<String, Rational>,
    ) -> ComponentInstance {
        let instance_id = self.next_instance_id;
        self.next_instance_id += 1;

        let namespace = format!("{}_{}", definition.name.to_lowercase(), instance_id);

        // Build ID mapping: local ID -> global EntityId
        let mut id_mapping = HashMap::new();
        for cp in &definition.control_points {
            let global_id = EntityId(self.next_entity_id);
            self.next_entity_id += 1;
            id_mapping.insert(cp.local_id, global_id);
        }

        // Merge default parameters with provided ones
        let mut resolved_params = HashMap::new();
        for param in &definition.parameters {
            let value = parameters
                .get(&param.name)
                .cloned()
                .unwrap_or_else(|| param.default.clone());
            resolved_params.insert(param.name.clone(), value);
        }

        // Resolve control points
        let resolved_control_points: Vec<ControlPoint> = definition
            .control_points
            .iter()
            .map(|cp| {
                let global_id = id_mapping[&cp.local_id];
                ControlPoint {
                    id: global_id,
                    position: PVector {
                        x: self.resolve_value(&cp.initial_x, &resolved_params, &id_mapping),
                        y: self.resolve_value(&cp.initial_y, &resolved_params, &id_mapping),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                    role: cp.role,
                    parent_path: None,
                }
            })
            .collect();

        // Resolve constraints
        let resolved_constraints: Vec<Constraint> = definition
            .constraints
            .iter()
            .map(|c| {
                let global_target = id_mapping[&c.target_local_id];
                let constraint_id = self.next_constraint_id;
                self.next_constraint_id += 1;

                Constraint {
                    id: constraint_id,
                    target: global_target,
                    component: c.component,
                    relation: c.relation,
                    term: self.resolve_term(&c.term, &resolved_params, &id_mapping),
                    priority: c.priority,
                    source_scope: Some(namespace.clone()),
                }
            })
            .collect();

        ComponentInstance {
            instance_id,
            component_name: definition.name.clone(),
            namespace,
            id_mapping,
            parameters: resolved_params,
            resolved_constraints,
            resolved_control_points,
        }
    }

    /// Resolve a ComponentValue to a Rational.
    fn resolve_value(
        &self,
        value: &ComponentValue,
        params: &HashMap<String, Rational>,
        _id_mapping: &HashMap<u64, EntityId>,
    ) -> Rational {
        match value {
            ComponentValue::Const { value } => value.clone(),
            ComponentValue::Param { name } => params
                .get(name)
                .cloned()
                .unwrap_or_else(Rational::zero),
            ComponentValue::Ref { .. } => {
                // For initial values, references are resolved later via constraints
                Rational::zero()
            }
        }
    }

    /// Resolve a ComponentTerm to a ConstraintTerm.
    fn resolve_term(
        &self,
        term: &ComponentTerm,
        params: &HashMap<String, Rational>,
        id_mapping: &HashMap<u64, EntityId>,
    ) -> ConstraintTerm {
        match term {
            ComponentTerm::Const { value } => ConstraintTerm::Const {
                value: value.clone(),
            },
            ComponentTerm::Ref { local_id, component } => ConstraintTerm::Ref {
                entity_id: id_mapping[local_id],
                component: *component,
            },
            ComponentTerm::Linear {
                coefficient,
                local_id,
                component,
                offset,
            } => ConstraintTerm::Linear {
                coefficient: coefficient.clone(),
                entity_id: id_mapping[local_id],
                component: *component,
                offset: offset.clone(),
            },
            ComponentTerm::Param { name } => ConstraintTerm::Const {
                value: params
                    .get(name)
                    .cloned()
                    .unwrap_or_else(Rational::zero),
            },
        }
    }

    /// Get the current state for serialization.
    pub fn state(&self) -> (u64, u64, u64) {
        (self.next_instance_id, self.next_entity_id, self.next_constraint_id)
    }

    /// Restore state from serialized values.
    pub fn restore_state(&mut self, instance_id: u64, entity_id: u64, constraint_id: u64) {
        self.next_instance_id = instance_id;
        self.next_entity_id = entity_id;
        self.next_constraint_id = constraint_id;
    }
}

// =============================================================================
// Import Resolution
// =============================================================================

/// An import declaration in a `.vs` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportDeclaration {
    /// The component name being imported.
    pub component_name: String,
    /// The file path (relative to importing file).
    pub source_path: String,
    /// Optional alias for the import.
    pub alias: Option<String>,
}

/// Result of parsing a `.vs` component file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedVsFile {
    /// Import declarations.
    pub imports: Vec<ImportDeclaration>,
    /// Component definition (if this file defines one).
    pub definition: Option<ComponentDefinition>,
    /// Inline constraints (root scope).
    pub root_constraints: Vec<Constraint>,
    /// Inline control points (root scope).
    pub root_control_points: Vec<ControlPoint>,
}

// =============================================================================
// Standard Library Components
// =============================================================================

/// Create the standard RoundedRect component definition.
///
/// This was previously hardcoded in the CLI but is now a pure component.
pub fn std_rounded_rect() -> ComponentDefinition {
    ComponentDefinition {
        name: "RoundedRect".to_string(),
        version: "1.0.0".to_string(),
        control_points: vec![
            // 8 tangent points for corner arcs
            ComponentControlPoint {
                local_id: 1,
                name: "tl_top".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
            ComponentControlPoint {
                local_id: 2,
                name: "tl_left".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
            ComponentControlPoint {
                local_id: 3,
                name: "tr_top".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
            ComponentControlPoint {
                local_id: 4,
                name: "tr_right".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
            ComponentControlPoint {
                local_id: 5,
                name: "br_right".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
            ComponentControlPoint {
                local_id: 6,
                name: "br_bottom".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
            ComponentControlPoint {
                local_id: 7,
                name: "bl_bottom".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
            ComponentControlPoint {
                local_id: 8,
                name: "bl_left".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
        ],
        constraints: vec![
            // Top edge: tr_top.x = tl_top.x + width - radius_tr - radius_tl
            // (Soft: can be overridden)
            ComponentConstraint {
                local_id: 1,
                target_local_id: 3, // tr_top
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ComponentTerm::Linear {
                    coefficient: Rational::one(),
                    local_id: 1, // tl_top
                    component: VectorComponent::X,
                    offset: Rational::from_int(100), // Default width - radii (placeholder)
                },
                priority: ConstraintPriority::Soft,
                description: Some("Top edge width".to_string()),
            },
            // Corner radius constraints (Soft: can be overridden)
            ComponentConstraint {
                local_id: 2,
                target_local_id: 1, // tl_top
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ComponentTerm::Linear {
                    coefficient: Rational::one(),
                    local_id: 2, // tl_left
                    component: VectorComponent::X,
                    offset: Rational::from_int(10), // Default corner radius
                },
                priority: ConstraintPriority::Soft,
                description: Some("Top-left corner radius (horizontal)".to_string()),
            },
            // Horizontal alignment (Hard: structural)
            ComponentConstraint {
                local_id: 3,
                target_local_id: 1, // tl_top
                component: VectorComponent::Y,
                relation: RelationType::Eq,
                term: ComponentTerm::Ref {
                    local_id: 3, // tr_top
                    component: VectorComponent::Y,
                },
                priority: ConstraintPriority::Hard,
                description: Some("Top edge horizontal alignment".to_string()),
            },
        ],
        exports: {
            let mut m = HashMap::new();
            m.insert("tl_top".to_string(), 1);
            m.insert("tl_left".to_string(), 2);
            m.insert("tr_top".to_string(), 3);
            m.insert("tr_right".to_string(), 4);
            m.insert("br_right".to_string(), 5);
            m.insert("br_bottom".to_string(), 6);
            m.insert("bl_bottom".to_string(), 7);
            m.insert("bl_left".to_string(), 8);
            m
        },
        parameters: vec![
            ComponentParameter {
                name: "x".to_string(),
                default: Rational::zero(),
                allow_negative: true,
                description: Some("X position of top-left corner".to_string()),
            },
            ComponentParameter {
                name: "y".to_string(),
                default: Rational::zero(),
                allow_negative: true,
                description: Some("Y position of top-left corner".to_string()),
            },
            ComponentParameter {
                name: "width".to_string(),
                default: Rational::from_int(100),
                allow_negative: false,
                description: Some("Width of rectangle".to_string()),
            },
            ComponentParameter {
                name: "height".to_string(),
                default: Rational::from_int(50),
                allow_negative: false,
                description: Some("Height of rectangle".to_string()),
            },
            ComponentParameter {
                name: "corner_radius".to_string(),
                default: Rational::from_int(10),
                allow_negative: false,
                description: Some("Default corner radius (can be overridden per-corner)".to_string()),
            },
        ],
    }
}

/// Create the standard Text component definition.
///
/// This was previously hardcoded in the CLI but is now a pure component.
pub fn std_text() -> ComponentDefinition {
    ComponentDefinition {
        name: "Text".to_string(),
        version: "1.0.0".to_string(),
        control_points: vec![
            ComponentControlPoint {
                local_id: 1,
                name: "TL".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
            ComponentControlPoint {
                local_id: 2,
                name: "TR".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
            ComponentControlPoint {
                local_id: 3,
                name: "BL".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
            ComponentControlPoint {
                local_id: 4,
                name: "BR".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param { name: "x".to_string() },
                initial_y: ComponentValue::Param { name: "y".to_string() },
            },
        ],
        // Note: Width/height constraints are added dynamically via update-metrics
        // These are structural constraints that ensure the box stays rectangular
        constraints: vec![
            // TL.y = TR.y (top edge horizontal)
            ComponentConstraint {
                local_id: 1,
                target_local_id: 2, // TR
                component: VectorComponent::Y,
                relation: RelationType::Eq,
                term: ComponentTerm::Ref {
                    local_id: 1, // TL
                    component: VectorComponent::Y,
                },
                priority: ConstraintPriority::Hard,
                description: Some("Top edge horizontal".to_string()),
            },
            // BL.y = BR.y (bottom edge horizontal)
            ComponentConstraint {
                local_id: 2,
                target_local_id: 4, // BR
                component: VectorComponent::Y,
                relation: RelationType::Eq,
                term: ComponentTerm::Ref {
                    local_id: 3, // BL
                    component: VectorComponent::Y,
                },
                priority: ConstraintPriority::Hard,
                description: Some("Bottom edge horizontal".to_string()),
            },
            // TL.x = BL.x (left edge vertical)
            ComponentConstraint {
                local_id: 3,
                target_local_id: 3, // BL
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ComponentTerm::Ref {
                    local_id: 1, // TL
                    component: VectorComponent::X,
                },
                priority: ConstraintPriority::Hard,
                description: Some("Left edge vertical".to_string()),
            },
            // TR.x = BR.x (right edge vertical)
            ComponentConstraint {
                local_id: 4,
                target_local_id: 4, // BR
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ComponentTerm::Ref {
                    local_id: 2, // TR
                    component: VectorComponent::X,
                },
                priority: ConstraintPriority::Hard,
                description: Some("Right edge vertical".to_string()),
            },
        ],
        exports: {
            let mut m = HashMap::new();
            m.insert("TL".to_string(), 1);
            m.insert("TR".to_string(), 2);
            m.insert("BL".to_string(), 3);
            m.insert("BR".to_string(), 4);
            m
        },
        parameters: vec![
            ComponentParameter {
                name: "x".to_string(),
                default: Rational::zero(),
                allow_negative: true,
                description: Some("X position".to_string()),
            },
            ComponentParameter {
                name: "y".to_string(),
                default: Rational::zero(),
                allow_negative: true,
                description: Some("Y position".to_string()),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_resolver_instantiation() {
        let mut resolver = NamespaceResolver::new();
        let text_def = std_text();

        let instance = resolver.instantiate(&text_def, HashMap::new());

        assert_eq!(instance.component_name, "Text");
        assert_eq!(instance.namespace, "text_1");
        assert_eq!(instance.id_mapping.len(), 4); // 4 control points
        assert_eq!(instance.resolved_constraints.len(), 4); // 4 structural constraints
        assert_eq!(instance.resolved_control_points.len(), 4);

        // All constraint source scopes should be set
        for c in &instance.resolved_constraints {
            assert_eq!(c.source_scope, Some("text_1".to_string()));
        }
    }

    #[test]
    fn test_namespace_resolver_multiple_instances() {
        let mut resolver = NamespaceResolver::new();
        let text_def = std_text();

        let inst1 = resolver.instantiate(&text_def, HashMap::new());
        let inst2 = resolver.instantiate(&text_def, HashMap::new());

        assert_eq!(inst1.namespace, "text_1");
        assert_eq!(inst2.namespace, "text_2");

        // IDs should not overlap
        let ids1: std::collections::HashSet<_> = inst1.id_mapping.values().collect();
        let ids2: std::collections::HashSet<_> = inst2.id_mapping.values().collect();
        assert!(ids1.is_disjoint(&ids2));
    }

    #[test]
    fn test_parameter_override() {
        let mut resolver = NamespaceResolver::new();
        let text_def = std_text();

        let mut params = HashMap::new();
        params.insert("x".to_string(), Rational::from_int(100));
        params.insert("y".to_string(), Rational::from_int(50));

        let instance = resolver.instantiate(&text_def, params);

        // Check that TL is at (100, 50)
        let tl = instance
            .resolved_control_points
            .iter()
            .find(|cp| cp.id == instance.id_mapping[&1])
            .unwrap();

        assert_eq!(tl.position.x, Rational::from_int(100));
        assert_eq!(tl.position.y, Rational::from_int(50));
    }

    #[test]
    fn test_rounded_rect_has_soft_constraints() {
        let rr_def = std_rounded_rect();

        // Check that corner radius constraints are Soft
        let soft_count = rr_def
            .constraints
            .iter()
            .filter(|c| c.priority == ConstraintPriority::Soft)
            .count();

        assert!(soft_count > 0, "RoundedRect should have Soft constraints for override");
    }
}
