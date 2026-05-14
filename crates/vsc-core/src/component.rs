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

    /// Path entities defined by this component (Phase D-01).
    ///
    /// Path segments reference control points by local ID (stored as EntityId(local_id)).
    /// These are remapped to global EntityIds during instantiation.
    ///
    /// For components like `Text` where paths are dynamically generated,
    /// this vector is empty.
    #[serde(default)]
    pub path_entities: Vec<PathEntityEntry>,
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
    Ref {
        local_id: u64,
        component: VectorComponent,
    },
}

/// A single factor in a component's multi-variable linear combination (uses local IDs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentLinearFactor {
    /// The multiplicative coefficient.
    pub coefficient: Rational,
    /// Local ID of the entity.
    pub local_id: u64,
    /// Which component of the entity.
    pub component: VectorComponent,
}

/// A term in a component constraint (may reference local IDs).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ComponentTerm {
    /// Constant value.
    Const { value: Rational },
    /// Reference to local entity.
    Ref {
        local_id: u64,
        component: VectorComponent,
    },
    /// Single-variable linear combination with local entity reference.
    Linear {
        coefficient: Rational,
        local_id: u64,
        component: VectorComponent,
        offset: Rational,
    },
    /// Multi-variable linear combination: Σ(coefficient_i * local_entity_i.component_i) + offset
    LinearCombination {
        terms: Vec<ComponentLinearFactor>,
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
    /// Path entities after namespace resolution (Phase D-01).
    /// Ready for registration in VsBuildInfo.path_entities.
    pub resolved_path_entities: Vec<PathEntityEntry>,
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

        // Use next_entity_id as namespace suffix for cross-session uniqueness
        // (instance_id resets if resolver is recreated between CLI calls)
        let namespace = format!("{}_{}", definition.name.to_lowercase(), self.next_entity_id);

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

        // Resolve path entities (Phase D-01)
        let resolved_path_entities: Vec<PathEntityEntry> = definition
            .path_entities
            .iter()
            .map(|pe| {
                // Allocate new EntityId for the path entity itself
                let path_id = EntityId(self.next_entity_id);
                self.next_entity_id += 1;

                // Remap segment EntityIds from local to global
                let resolved_segments: Vec<PathSegment> = pe
                    .segments
                    .iter()
                    .map(|seg| self.resolve_path_segment(seg, &id_mapping))
                    .collect();

                PathEntityEntry {
                    id: path_id,
                    segments: resolved_segments,
                    closed: pe.closed,
                    fill_rule: pe.fill_rule,
                    fill: pe.fill.clone(),
                    stroke: pe.stroke.clone(),
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
            resolved_path_entities,
        }
    }

    /// Resolve a PathSegment by remapping local EntityIds to global ones.
    fn resolve_path_segment(
        &self,
        segment: &PathSegment,
        id_mapping: &HashMap<u64, EntityId>,
    ) -> PathSegment {
        match segment {
            PathSegment::Line { from, to } => PathSegment::Line {
                from: id_mapping.get(&from.0).copied().unwrap_or(*from),
                to: id_mapping.get(&to.0).copied().unwrap_or(*to),
            },
            PathSegment::Quad { from, handle, to } => PathSegment::Quad {
                from: id_mapping.get(&from.0).copied().unwrap_or(*from),
                handle: id_mapping.get(&handle.0).copied().unwrap_or(*handle),
                to: id_mapping.get(&to.0).copied().unwrap_or(*to),
            },
            PathSegment::Cubic {
                from,
                handle1,
                handle2,
                to,
            } => PathSegment::Cubic {
                from: id_mapping.get(&from.0).copied().unwrap_or(*from),
                handle1: id_mapping.get(&handle1.0).copied().unwrap_or(*handle1),
                handle2: id_mapping.get(&handle2.0).copied().unwrap_or(*handle2),
                to: id_mapping.get(&to.0).copied().unwrap_or(*to),
            },
            PathSegment::Arc {
                from,
                to,
                rx,
                ry,
                rotation,
                large_arc,
                sweep,
            } => PathSegment::Arc {
                from: id_mapping.get(&from.0).copied().unwrap_or(*from),
                to: id_mapping.get(&to.0).copied().unwrap_or(*to),
                rx: rx.clone(),
                ry: ry.clone(),
                rotation: *rotation,
                large_arc: *large_arc,
                sweep: *sweep,
            },
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
            ComponentValue::Param { name } => {
                params.get(name).cloned().unwrap_or_else(Rational::zero)
            }
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
            ComponentTerm::Ref {
                local_id,
                component,
            } => ConstraintTerm::Ref {
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
            ComponentTerm::LinearCombination { terms, offset } => {
                use crate::LinearFactor;
                ConstraintTerm::LinearCombination {
                    terms: terms
                        .iter()
                        .map(|f| LinearFactor {
                            coefficient: f.coefficient.clone(),
                            entity_id: id_mapping[&f.local_id],
                            component: f.component,
                        })
                        .collect(),
                    offset: offset.clone(),
                }
            }
            ComponentTerm::Param { name } => ConstraintTerm::Const {
                value: params.get(name).cloned().unwrap_or_else(Rational::zero),
            },
        }
    }

    /// Get the current state for serialization.
    pub fn state(&self) -> (u64, u64, u64) {
        (
            self.next_instance_id,
            self.next_entity_id,
            self.next_constraint_id,
        )
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
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            ComponentControlPoint {
                local_id: 2,
                name: "tl_left".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            ComponentControlPoint {
                local_id: 3,
                name: "tr_top".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            ComponentControlPoint {
                local_id: 4,
                name: "tr_right".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            ComponentControlPoint {
                local_id: 5,
                name: "br_right".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            ComponentControlPoint {
                local_id: 6,
                name: "br_bottom".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            ComponentControlPoint {
                local_id: 7,
                name: "bl_bottom".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            ComponentControlPoint {
                local_id: 8,
                name: "bl_left".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            // Scalar radius entities (local_ids 9-12).
            // The X component stores the radius value; Y is unused (zero).
            // Ge constraints on these enforce radius >= 0 at the solver level.
            ComponentControlPoint {
                local_id: 9,
                name: "radius_tl".to_string(),
                role: ControlPointRole::Handle,
                initial_x: ComponentValue::Param {
                    name: "radius_tl".to_string(),
                },
                initial_y: ComponentValue::Const {
                    value: Rational::zero(),
                },
            },
            ComponentControlPoint {
                local_id: 10,
                name: "radius_tr".to_string(),
                role: ControlPointRole::Handle,
                initial_x: ComponentValue::Param {
                    name: "radius_tr".to_string(),
                },
                initial_y: ComponentValue::Const {
                    value: Rational::zero(),
                },
            },
            ComponentControlPoint {
                local_id: 11,
                name: "radius_bl".to_string(),
                role: ControlPointRole::Handle,
                initial_x: ComponentValue::Param {
                    name: "radius_bl".to_string(),
                },
                initial_y: ComponentValue::Const {
                    value: Rational::zero(),
                },
            },
            ComponentControlPoint {
                local_id: 12,
                name: "radius_br".to_string(),
                role: ControlPointRole::Handle,
                initial_x: ComponentValue::Param {
                    name: "radius_br".to_string(),
                },
                initial_y: ComponentValue::Const {
                    value: Rational::zero(),
                },
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
            // radius_tl >= 0 (Hard: radius_tl.x must be non-negative)
            ComponentConstraint {
                local_id: 4,
                target_local_id: 9, // radius_tl
                component: VectorComponent::X,
                relation: RelationType::Ge,
                term: ComponentTerm::Const {
                    value: Rational::zero(),
                },
                priority: ConstraintPriority::Hard,
                description: Some("std_rounded_rect:radius_validation: radius_tl >= 0".to_string()),
            },
            // radius_tr >= 0 (Hard: radius_tr.x must be non-negative)
            ComponentConstraint {
                local_id: 5,
                target_local_id: 10, // radius_tr
                component: VectorComponent::X,
                relation: RelationType::Ge,
                term: ComponentTerm::Const {
                    value: Rational::zero(),
                },
                priority: ConstraintPriority::Hard,
                description: Some("std_rounded_rect:radius_validation: radius_tr >= 0".to_string()),
            },
            // radius_bl >= 0 (Hard: radius_bl.x must be non-negative)
            ComponentConstraint {
                local_id: 6,
                target_local_id: 11, // radius_bl
                component: VectorComponent::X,
                relation: RelationType::Ge,
                term: ComponentTerm::Const {
                    value: Rational::zero(),
                },
                priority: ConstraintPriority::Hard,
                description: Some("std_rounded_rect:radius_validation: radius_bl >= 0".to_string()),
            },
            // radius_br >= 0 (Hard: radius_br.x must be non-negative)
            ComponentConstraint {
                local_id: 7,
                target_local_id: 12, // radius_br
                component: VectorComponent::X,
                relation: RelationType::Ge,
                term: ComponentTerm::Const {
                    value: Rational::zero(),
                },
                priority: ConstraintPriority::Hard,
                description: Some("std_rounded_rect:radius_validation: radius_br >= 0".to_string()),
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
            m.insert("radius_tl".to_string(), 9);
            m.insert("radius_tr".to_string(), 10);
            m.insert("radius_bl".to_string(), 11);
            m.insert("radius_br".to_string(), 12);
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
                description: Some(
                    "Default corner radius (can be overridden per-corner)".to_string(),
                ),
            },
            ComponentParameter {
                name: "radius_tl".to_string(),
                default: Rational::from_int(10),
                allow_negative: false,
                description: Some("Top-left corner radius (overrides corner_radius)".to_string()),
            },
            ComponentParameter {
                name: "radius_tr".to_string(),
                default: Rational::from_int(10),
                allow_negative: false,
                description: Some("Top-right corner radius (overrides corner_radius)".to_string()),
            },
            ComponentParameter {
                name: "radius_bl".to_string(),
                default: Rational::from_int(10),
                allow_negative: false,
                description: Some(
                    "Bottom-left corner radius (overrides corner_radius)".to_string(),
                ),
            },
            ComponentParameter {
                name: "radius_br".to_string(),
                default: Rational::from_int(10),
                allow_negative: false,
                description: Some(
                    "Bottom-right corner radius (overrides corner_radius)".to_string(),
                ),
            },
        ],
        // Path topology (Phase D-01)
        // Local IDs:
        //   1: tl_top,    2: tl_left
        //   3: tr_top,    4: tr_right
        //   5: br_right,  6: br_bottom
        //   7: bl_bottom, 8: bl_left
        //
        // Path order (clockwise from tl_top):
        //   tl_top -> tr_top (line, top edge)
        //   tr_top -> tr_right (arc, top-right corner)
        //   tr_right -> br_right (line, right edge)
        //   br_right -> br_bottom (arc, bottom-right corner)
        //   br_bottom -> bl_bottom (line, bottom edge)
        //   bl_bottom -> bl_left (arc, bottom-left corner)
        //   bl_left -> tl_left (line, left edge)
        //   tl_left -> tl_top (arc, top-left corner)
        path_entities: vec![PathEntityEntry {
            id: EntityId(100), // Placeholder; remapped during instantiation
            segments: vec![
                // Top edge: tl_top -> tr_top
                PathSegment::Line {
                    from: EntityId(1), // tl_top
                    to: EntityId(3),   // tr_top
                },
                // Top-right corner arc: tr_top -> tr_right
                PathSegment::Arc {
                    from: EntityId(3),          // tr_top
                    to: EntityId(4),            // tr_right
                    rx: Rational::from_int(10), // Default corner_radius
                    ry: Rational::from_int(10),
                    rotation: 0.0,
                    large_arc: false,
                    sweep: true, // Clockwise
                },
                // Right edge: tr_right -> br_right
                PathSegment::Line {
                    from: EntityId(4), // tr_right
                    to: EntityId(5),   // br_right
                },
                // Bottom-right corner arc: br_right -> br_bottom
                PathSegment::Arc {
                    from: EntityId(5), // br_right
                    to: EntityId(6),   // br_bottom
                    rx: Rational::from_int(10),
                    ry: Rational::from_int(10),
                    rotation: 0.0,
                    large_arc: false,
                    sweep: true,
                },
                // Bottom edge: br_bottom -> bl_bottom
                PathSegment::Line {
                    from: EntityId(6), // br_bottom
                    to: EntityId(7),   // bl_bottom
                },
                // Bottom-left corner arc: bl_bottom -> bl_left
                PathSegment::Arc {
                    from: EntityId(7), // bl_bottom
                    to: EntityId(8),   // bl_left
                    rx: Rational::from_int(10),
                    ry: Rational::from_int(10),
                    rotation: 0.0,
                    large_arc: false,
                    sweep: true,
                },
                // Left edge: bl_left -> tl_left
                PathSegment::Line {
                    from: EntityId(8), // bl_left
                    to: EntityId(2),   // tl_left
                },
                // Top-left corner arc: tl_left -> tl_top
                PathSegment::Arc {
                    from: EntityId(2), // tl_left
                    to: EntityId(1),   // tl_top
                    rx: Rational::from_int(10),
                    ry: Rational::from_int(10),
                    rotation: 0.0,
                    large_arc: false,
                    sweep: true,
                },
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: None,   // Style injected at instantiation
            stroke: None, // Style injected at instantiation
        }],
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
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            ComponentControlPoint {
                local_id: 2,
                name: "TR".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            ComponentControlPoint {
                local_id: 3,
                name: "BL".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
            },
            ComponentControlPoint {
                local_id: 4,
                name: "BR".to_string(),
                role: ControlPointRole::Anchor,
                initial_x: ComponentValue::Param {
                    name: "x".to_string(),
                },
                initial_y: ComponentValue::Param {
                    name: "y".to_string(),
                },
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
        // Text paths are dynamically generated by expand-text-to-paths
        // via TextShaper. No static path topology in the definition.
        path_entities: vec![],
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
        // Namespace uses next_entity_id at instantiation time (10000 for new resolver)
        assert_eq!(instance.namespace, "text_10000");
        assert_eq!(instance.id_mapping.len(), 4); // 4 control points
        assert_eq!(instance.resolved_constraints.len(), 4); // 4 structural constraints
        assert_eq!(instance.resolved_control_points.len(), 4);

        // All constraint source scopes should be set
        for c in &instance.resolved_constraints {
            assert_eq!(c.source_scope, Some("text_10000".to_string()));
        }
    }

    #[test]
    fn test_namespace_resolver_multiple_instances() {
        let mut resolver = NamespaceResolver::new();
        let text_def = std_text();

        let inst1 = resolver.instantiate(&text_def, HashMap::new());
        let inst2 = resolver.instantiate(&text_def, HashMap::new());

        // Namespaces use next_entity_id at instantiation time
        // inst1: starts at 10000, allocates 4 IDs (10000-10003)
        // inst2: starts at 10004, allocates 4 IDs (10004-10007)
        assert_eq!(inst1.namespace, "text_10000");
        assert_eq!(inst2.namespace, "text_10004");

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

        assert!(
            soft_count > 0,
            "RoundedRect should have Soft constraints for override"
        );
    }

    // =========================================================================
    // Phase D-01: Path Entity Tests
    // =========================================================================

    #[test]
    fn test_rounded_rect_has_path_entity() {
        let rr_def = std_rounded_rect();

        // Should have exactly one path entity (the outline)
        assert_eq!(rr_def.path_entities.len(), 1);

        let path = &rr_def.path_entities[0];

        // Should have 8 segments: 4 Line + 4 Arc
        assert_eq!(path.segments.len(), 8);

        // Count segment types
        let line_count = path
            .segments
            .iter()
            .filter(|s| matches!(s, PathSegment::Line { .. }))
            .count();
        let arc_count = path
            .segments
            .iter()
            .filter(|s| matches!(s, PathSegment::Arc { .. }))
            .count();

        assert_eq!(line_count, 4, "Should have 4 Line segments");
        assert_eq!(arc_count, 4, "Should have 4 Arc segments");

        // Path should be closed
        assert!(path.closed);

        // Fill and stroke should be None (injected at instantiation)
        assert!(path.fill.is_none());
        assert!(path.stroke.is_none());
    }

    #[test]
    fn test_rounded_rect_path_segment_order() {
        let rr_def = std_rounded_rect();
        let path = &rr_def.path_entities[0];

        // Verify segment order: Line, Arc, Line, Arc, Line, Arc, Line, Arc
        assert!(matches!(path.segments[0], PathSegment::Line { .. })); // Top edge
        assert!(matches!(path.segments[1], PathSegment::Arc { .. })); // Top-right corner
        assert!(matches!(path.segments[2], PathSegment::Line { .. })); // Right edge
        assert!(matches!(path.segments[3], PathSegment::Arc { .. })); // Bottom-right corner
        assert!(matches!(path.segments[4], PathSegment::Line { .. })); // Bottom edge
        assert!(matches!(path.segments[5], PathSegment::Arc { .. })); // Bottom-left corner
        assert!(matches!(path.segments[6], PathSegment::Line { .. })); // Left edge
        assert!(matches!(path.segments[7], PathSegment::Arc { .. })); // Top-left corner
    }

    #[test]
    fn test_rounded_rect_path_connectivity() {
        let rr_def = std_rounded_rect();
        let path = &rr_def.path_entities[0];

        // Extract "to" from each segment to verify connectivity
        let endpoints: Vec<EntityId> = path
            .segments
            .iter()
            .map(|seg| match seg {
                PathSegment::Line { to, .. } => *to,
                PathSegment::Arc { to, .. } => *to,
                PathSegment::Quad { to, .. } => *to,
                PathSegment::Cubic { to, .. } => *to,
            })
            .collect();

        // Each segment's "to" should be the next segment's "from"
        for i in 0..path.segments.len() - 1 {
            let next_from = match &path.segments[i + 1] {
                PathSegment::Line { from, .. } => *from,
                PathSegment::Arc { from, .. } => *from,
                PathSegment::Quad { from, .. } => *from,
                PathSegment::Cubic { from, .. } => *from,
            };
            assert_eq!(
                endpoints[i],
                next_from,
                "Segment {} 'to' ({:?}) should equal segment {} 'from' ({:?})",
                i,
                endpoints[i],
                i + 1,
                next_from
            );
        }

        // For closed path, last segment's "to" should be first segment's "from"
        let first_from = match &path.segments[0] {
            PathSegment::Line { from, .. } => *from,
            PathSegment::Arc { from, .. } => *from,
            PathSegment::Quad { from, .. } => *from,
            PathSegment::Cubic { from, .. } => *from,
        };
        let last_to = endpoints.last().unwrap();
        assert_eq!(
            *last_to, first_from,
            "Last segment 'to' ({:?}) should equal first segment 'from' ({:?}) for closed path",
            last_to, first_from
        );
    }

    #[test]
    fn test_text_has_no_path_entity() {
        let text_def = std_text();

        // Text paths are dynamically generated, so no static paths
        assert!(text_def.path_entities.is_empty());
    }

    #[test]
    fn test_path_entity_instantiation() {
        let mut resolver = NamespaceResolver::new();
        let rr_def = std_rounded_rect();

        let instance = resolver.instantiate(&rr_def, HashMap::new());

        // Should have one resolved path entity
        assert_eq!(instance.resolved_path_entities.len(), 1);

        let path = &instance.resolved_path_entities[0];

        // Path ID should be globally unique (not the placeholder 100)
        assert_ne!(path.id.0, 100);

        // All segment EntityIds should be remapped to global IDs
        for seg in &path.segments {
            let ids: Vec<EntityId> = match seg {
                PathSegment::Line { from, to } => vec![*from, *to],
                PathSegment::Arc { from, to, .. } => vec![*from, *to],
                PathSegment::Quad { from, handle, to } => vec![*from, *handle, *to],
                PathSegment::Cubic {
                    from,
                    handle1,
                    handle2,
                    to,
                } => {
                    vec![*from, *handle1, *handle2, *to]
                }
            };

            for id in ids {
                // Global IDs should be >= 10000 (NamespaceResolver's starting offset)
                assert!(
                    id.0 >= 10000,
                    "EntityId {:?} should be remapped to global ID (>= 10000)",
                    id
                );
            }
        }
    }

    // =========================================================================
    // Phase D-14: radius >= 0 Constraint Tests
    // =========================================================================

    #[test]
    fn test_rounded_rect_has_radius_ge_constraints() {
        let rr_def = std_rounded_rect();

        // Collect all Ge constraints targeting radius entities (local_ids 9-12)
        let radius_local_ids: std::collections::HashSet<u64> =
            [9, 10, 11, 12].iter().cloned().collect();
        let ge_constraints: Vec<&ComponentConstraint> = rr_def
            .constraints
            .iter()
            .filter(|c| {
                c.relation == RelationType::Ge
                    && radius_local_ids.contains(&c.target_local_id)
                    && c.priority == ConstraintPriority::Hard
            })
            .collect();

        assert_eq!(
            ge_constraints.len(),
            4,
            "Expected 4 Hard Ge constraints for radius_tl, radius_tr, radius_bl, radius_br; got {}",
            ge_constraints.len()
        );

        // Each Ge constraint must compare against Rational::zero()
        for c in &ge_constraints {
            match &c.term {
                ComponentTerm::Const { value } => {
                    assert_eq!(
                        *value,
                        Rational::zero(),
                        "Ge constraint for local_id {} must compare against zero",
                        c.target_local_id
                    );
                }
                other => panic!(
                    "Expected Const term for radius Ge constraint, got {:?}",
                    other
                ),
            }
        }
    }

    #[test]
    fn test_rounded_rect_radius_ge_constraint_covers_all_corners() {
        let rr_def = std_rounded_rect();

        // Verify each corner has a dedicated Ge constraint
        for (local_id, name) in &[
            (9u64, "radius_tl"),
            (10u64, "radius_tr"),
            (11u64, "radius_bl"),
            (12u64, "radius_br"),
        ] {
            let found = rr_def.constraints.iter().any(|c| {
                c.target_local_id == *local_id
                    && c.relation == RelationType::Ge
                    && c.priority == ConstraintPriority::Hard
            });
            assert!(
                found,
                "Missing Hard Ge(>= 0) constraint for corner '{}' (local_id {})",
                name, local_id
            );
        }
    }

    #[test]
    fn test_rounded_rect_negative_radius_violates_ge_constraint() {
        // This test verifies that when a negative radius parameter is provided,
        // the instantiated constraint reflects a violation scenario:
        // the resolved control point's initial X value is negative, which the
        // Hard Ge constraint (radius.x >= 0) would reject during solving.
        let mut resolver = NamespaceResolver::new();
        let rr_def = std_rounded_rect();

        // Instantiate with a negative radius_tl
        let mut params = HashMap::new();
        params.insert("radius_tl".to_string(), Rational::from_int(-5));

        let instance = resolver.instantiate(&rr_def, params);

        // Find the resolved control point for radius_tl (local_id = 9)
        let radius_tl_global_id = instance.id_mapping[&9];
        let radius_tl_cp = instance
            .resolved_control_points
            .iter()
            .find(|cp| cp.id == radius_tl_global_id)
            .expect("radius_tl control point must exist after instantiation");

        // The initial X value should be -5 (negative — violates the Ge >= 0 constraint)
        assert_eq!(
            radius_tl_cp.position.x,
            Rational::from_int(-5),
            "radius_tl initial X should carry the negative value from params"
        );

        // The Ge constraint for radius_tl must exist in the resolved constraints
        let ge_constraint = instance
            .resolved_constraints
            .iter()
            .find(|c| {
                c.target == radius_tl_global_id
                    && c.relation == RelationType::Ge
                    && c.priority == ConstraintPriority::Hard
            })
            .expect("Hard Ge constraint for radius_tl must be present in resolved constraints");

        // The constraint term must be Const { value: 0 }
        match &ge_constraint.term {
            ConstraintTerm::Const { value } => {
                assert_eq!(*value, Rational::zero(), "Ge constraint bound must be zero");
            }
            other => panic!("Expected Const term, got {:?}", other),
        }

        // Confirm violation: initial value (-5) < 0, so the constraint is violated
        // The solver would return SolverError::Infeasible when it encounters this
        // Hard Ge constraint with a negative resolved value.
        assert!(
            radius_tl_cp.position.x < Rational::zero(),
            "Negative radius (-5) must be less than zero, confirming Ge constraint violation"
        );
    }
}
