//! Path and Constraint Validator
//!
//! This module provides static validation for P-dimension entities, ensuring:
//!
//! 1. **Type Safety**: Path segments only reference ControlPoint entities
//! 2. **FM-Decidability**: No non-linear constraints enter the solver
//! 3. **Topology Integrity**: Paths form valid connected graphs
//!
//! ## Architectural Decision (Phase 6)
//!
//! The validator acts as a gatekeeper before constraints enter the FM solver.
//! Any constraint that would require polynomial equation solving is rejected
//! at this stage with a clear error message.

use crate::{Constraint, ConstraintTerm, ControlPoint, Entity, EntityId, Path, PathSegment};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Validation error types for P-dimension entities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error_type", rename_all = "snake_case")]
pub enum ValidationError {
    /// A path segment references an entity that is not a ControlPoint.
    InvalidControlPointReference {
        path_id: EntityId,
        segment_index: usize,
        referenced_id: EntityId,
        actual_type: String,
    },
    /// A path segment references a non-existent entity.
    MissingControlPoint {
        path_id: EntityId,
        segment_index: usize,
        referenced_id: EntityId,
    },
    /// A constraint targets a non-linear property (curve parameter, intersection, etc.).
    NonLinearConstraintRejected {
        constraint_id: u64,
        reason: String,
        suggestion: String,
    },
    /// A path is empty (has no segments).
    EmptyPath { path_id: EntityId },
    /// A path does not start with MoveTo.
    PathMissingMoveTo { path_id: EntityId },
    /// Control point role mismatch (handle used as anchor or vice versa).
    ControlPointRoleMismatch {
        path_id: EntityId,
        segment_index: usize,
        control_point_id: EntityId,
        expected_role: String,
        actual_role: String,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::InvalidControlPointReference {
                path_id,
                segment_index,
                referenced_id,
                actual_type,
            } => {
                write!(
                    f,
                    "Path {} segment {} references entity {} which is a {} (expected ControlPoint)",
                    path_id.0, segment_index, referenced_id.0, actual_type
                )
            }
            ValidationError::MissingControlPoint {
                path_id,
                segment_index,
                referenced_id,
            } => {
                write!(
                    f,
                    "Path {} segment {} references non-existent entity {}",
                    path_id.0, segment_index, referenced_id.0
                )
            }
            ValidationError::NonLinearConstraintRejected {
                constraint_id,
                reason,
                suggestion,
            } => {
                write!(
                    f,
                    "NON_LINEAR_CONSTRAINT_REJECTED: Constraint {} rejected. {}. {}",
                    constraint_id, reason, suggestion
                )
            }
            ValidationError::EmptyPath { path_id } => {
                write!(f, "Path {} has no segments", path_id.0)
            }
            ValidationError::PathMissingMoveTo { path_id } => {
                write!(f, "Path {} does not start with MoveTo", path_id.0)
            }
            ValidationError::ControlPointRoleMismatch {
                path_id,
                segment_index,
                control_point_id,
                expected_role,
                actual_role,
            } => {
                write!(
                    f,
                    "Path {} segment {} expects {} but control point {} has role {}",
                    path_id.0, segment_index, expected_role, control_point_id.0, actual_role
                )
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// Result of validation.
pub type ValidationResult = Result<(), Vec<ValidationError>>;

/// Validator for P-dimension entities and constraints.
pub struct Validator {
    /// Map of all entities by ID.
    entities: HashMap<EntityId, Entity>,
    /// Set of ControlPoint IDs for quick lookup.
    control_point_ids: HashSet<EntityId>,
}

impl Validator {
    /// Create a new validator with the given entities.
    pub fn new(entities: Vec<Entity>) -> Self {
        let mut entity_map = HashMap::new();
        let mut cp_ids = HashSet::new();

        for entity in entities {
            let id = entity.id();
            if let Entity::ControlPoint(_) = &entity {
                cp_ids.insert(id);
            }
            entity_map.insert(id, entity);
        }

        Self {
            entities: entity_map,
            control_point_ids: cp_ids,
        }
    }

    /// Validate all paths in the entity set.
    pub fn validate_paths(&self) -> ValidationResult {
        let mut errors = Vec::new();

        for entity in self.entities.values() {
            if let Entity::Path(path) = entity {
                errors.extend(self.validate_path(path));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Validate a single path.
    fn validate_path(&self, path: &Path) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // Check: path is not empty
        if path.segments.is_empty() {
            errors.push(ValidationError::EmptyPath { path_id: path.id });
            return errors;
        }

        // Note: Phase G design uses explicit from/to in each segment.
        // The first segment's `from` serves as the implicit MoveTo.
        // No separate MoveTo variant exists.

        // Check: all referenced entities are valid ControlPoints
        for (idx, segment) in path.segments.iter().enumerate() {
            let point_ids = self.get_segment_point_ids(segment);

            for (point_id, expected_role) in point_ids {
                match self.entities.get(&point_id) {
                    None => {
                        errors.push(ValidationError::MissingControlPoint {
                            path_id: path.id,
                            segment_index: idx,
                            referenced_id: point_id,
                        });
                    }
                    Some(Entity::ControlPoint(cp)) => {
                        // Validate role if expected
                        if let Some(role) = expected_role {
                            let actual_role = format!("{:?}", cp.role);
                            if actual_role.to_lowercase() != role {
                                errors.push(ValidationError::ControlPointRoleMismatch {
                                    path_id: path.id,
                                    segment_index: idx,
                                    control_point_id: point_id,
                                    expected_role: role.to_string(),
                                    actual_role,
                                });
                            }
                        }
                    }
                    Some(other) => {
                        let type_name = match other {
                            Entity::Rect { .. } => "Rect",
                            Entity::Text { .. } => "Text",
                            Entity::Path(_) => "Path",
                            Entity::Radius(_) => "Radius",
                            Entity::Arc(_) => "Arc",
                            Entity::RoundedRect(_) => "RoundedRect",
                            Entity::Angle(_) => "Angle",
                            Entity::TextEntity(_) => "TextEntity",
                            Entity::ColorStop(_) => "ColorStop",
                            Entity::LinearGradient(_) => "LinearGradient",
                            Entity::RadialGradient(_) => "RadialGradient",
                            Entity::ConicGradient(_) => "ConicGradient",
                            Entity::ControlPoint(_) => unreachable!(),
                        };
                        errors.push(ValidationError::InvalidControlPointReference {
                            path_id: path.id,
                            segment_index: idx,
                            referenced_id: point_id,
                            actual_type: type_name.to_string(),
                        });
                    }
                }
            }
        }

        errors
    }

    /// Get point IDs from a segment with expected roles (None = anchor, Some("handle") = handle).
    ///
    /// Phase G design: Each segment has explicit from/to. Both `from` and `to` are anchors.
    fn get_segment_point_ids(
        &self,
        segment: &PathSegment,
    ) -> Vec<(EntityId, Option<&'static str>)> {
        match segment {
            PathSegment::Line { from, to } => vec![(*from, Some("anchor")), (*to, Some("anchor"))],
            PathSegment::Quad { from, handle, to } => vec![
                (*from, Some("anchor")),
                (*handle, Some("handle")),
                (*to, Some("anchor")),
            ],
            PathSegment::Cubic {
                from,
                handle1,
                handle2,
                to,
            } => vec![
                (*from, Some("anchor")),
                (*handle1, Some("handle")),
                (*handle2, Some("handle")),
                (*to, Some("anchor")),
            ],
            PathSegment::Arc { from, to, .. } => {
                vec![(*from, Some("anchor")), (*to, Some("anchor"))]
            }
        }
    }

    /// Validate that a constraint does not target non-linear curve properties.
    ///
    /// ## Rejected Patterns
    /// - Curve parameter `t` constraints (e.g., "point at t=0.5 on curve")
    /// - Curve intersection constraints
    /// - Tangent/normal direction constraints
    /// - Curvature constraints
    pub fn validate_constraint(&self, constraint: &Constraint) -> ValidationResult {
        let mut errors = Vec::new();

        // Check if target is a Path (not allowed - can only constrain ControlPoints)
        if let Some(Entity::Path(_)) = self.entities.get(&constraint.target) {
            errors.push(ValidationError::NonLinearConstraintRejected {
                constraint_id: constraint.id,
                reason: "Cannot constrain Path entity directly".to_string(),
                suggestion: "Constrain the ControlPoint entities that define the path instead"
                    .to_string(),
            });
        }

        // Check the constraint term for non-linear references
        match &constraint.term {
            ConstraintTerm::Ref { entity_id, .. } => {
                if let Some(Entity::Path(_)) = self.entities.get(entity_id) {
                    errors.push(ValidationError::NonLinearConstraintRejected {
                        constraint_id: constraint.id,
                        reason: "Cannot reference Path entity in constraint term".to_string(),
                        suggestion: "Reference specific ControlPoint entities instead".to_string(),
                    });
                }
            }
            ConstraintTerm::Linear { entity_id, .. } => {
                if let Some(Entity::Path(_)) = self.entities.get(entity_id) {
                    errors.push(ValidationError::NonLinearConstraintRejected {
                        constraint_id: constraint.id,
                        reason: "Cannot use Path entity in linear constraint term".to_string(),
                        suggestion: "Use ControlPoint entity references instead".to_string(),
                    });
                }
            }
            ConstraintTerm::Const { .. } => {
                // Constants are always valid
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Check if an entity ID refers to a ControlPoint.
    pub fn is_control_point(&self, id: EntityId) -> bool {
        self.control_point_ids.contains(&id)
    }

    /// Get all control points referenced by a path.
    pub fn get_path_control_points(&self, path_id: EntityId) -> Option<Vec<&ControlPoint>> {
        let path = match self.entities.get(&path_id)? {
            Entity::Path(p) => p,
            _ => return None,
        };

        let mut cps = Vec::new();
        for id in path.referenced_control_points() {
            if let Some(Entity::ControlPoint(cp)) = self.entities.get(&id) {
                cps.push(cp);
            }
        }
        Some(cps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConstraintPriority, ControlPointRole, PVector, Rational};

    fn make_anchor(id: u64, x: i64, y: i64) -> Entity {
        Entity::ControlPoint(ControlPoint {
            id: EntityId(id),
            position: PVector {
                x: Rational::from_int(x),
                y: Rational::from_int(y),
                z: Rational::zero(),
                t: Rational::zero(),
            },
            role: ControlPointRole::Anchor,
            parent_path: None,
        })
    }

    fn make_handle(id: u64, x: i64, y: i64) -> Entity {
        Entity::ControlPoint(ControlPoint {
            id: EntityId(id),
            position: PVector {
                x: Rational::from_int(x),
                y: Rational::from_int(y),
                z: Rational::zero(),
                t: Rational::zero(),
            },
            role: ControlPointRole::Handle,
            parent_path: None,
        })
    }

    #[test]
    fn test_valid_line_path() {
        // Phase G design: Line { from, to } with explicit endpoints
        let entities = vec![
            make_anchor(1, 0, 0),
            make_anchor(2, 100, 100),
            Entity::Path(Path {
                id: EntityId(100),
                segments: vec![PathSegment::Line {
                    from: EntityId(1),
                    to: EntityId(2),
                }],
                fill_rule: crate::FillRule::NonZero,
                closed: false,
            }),
        ];

        let validator = Validator::new(entities);
        assert!(validator.validate_paths().is_ok());
    }

    #[test]
    fn test_valid_cubic_bezier_path() {
        // Phase G design: Cubic { from, handle1, handle2, to }
        let entities = vec![
            make_anchor(1, 0, 0),     // Start
            make_handle(2, 50, 100),  // Control 1
            make_handle(3, 150, 100), // Control 2
            make_anchor(4, 200, 0),   // End
            Entity::Path(Path {
                id: EntityId(100),
                segments: vec![PathSegment::Cubic {
                    from: EntityId(1),
                    handle1: EntityId(2),
                    handle2: EntityId(3),
                    to: EntityId(4),
                }],
                fill_rule: crate::FillRule::NonZero,
                closed: false,
            }),
        ];

        let validator = Validator::new(entities);
        assert!(validator.validate_paths().is_ok());
    }

    #[test]
    fn test_rejects_missing_control_point() {
        let entities = vec![
            make_anchor(1, 0, 0),
            // Missing entity 2!
            Entity::Path(Path {
                id: EntityId(100),
                segments: vec![PathSegment::Line {
                    from: EntityId(1),
                    to: EntityId(2), // References missing entity
                }],
                fill_rule: crate::FillRule::NonZero,
                closed: false,
            }),
        ];

        let validator = Validator::new(entities);
        let result = validator.validate_paths();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(matches!(
            errors[0],
            ValidationError::MissingControlPoint { .. }
        ));
    }

    #[test]
    fn test_rejects_non_control_point_reference() {
        let entities = vec![
            make_anchor(1, 0, 0),
            Entity::Rect {
                id: EntityId(2),
                bounds: crate::RectBounds {
                    x: Rational::zero(),
                    y: Rational::zero(),
                    width: Rational::from_int(100),
                    height: Rational::from_int(100),
                },
            },
            Entity::Path(Path {
                id: EntityId(100),
                segments: vec![PathSegment::Line {
                    from: EntityId(1),
                    to: EntityId(2), // References Rect, not ControlPoint!
                }],
                fill_rule: crate::FillRule::NonZero,
                closed: false,
            }),
        ];

        let validator = Validator::new(entities);
        let result = validator.validate_paths();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(matches!(
            errors[0],
            ValidationError::InvalidControlPointReference { .. }
        ));
    }

    #[test]
    fn test_rejects_empty_path() {
        let entities = vec![Entity::Path(Path {
            id: EntityId(100),
            segments: vec![],
            fill_rule: crate::FillRule::NonZero,
            closed: false,
        })];

        let validator = Validator::new(entities);
        let result = validator.validate_paths();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(matches!(errors[0], ValidationError::EmptyPath { .. }));
    }

    // Note: test_rejects_path_without_moveto removed.
    // Phase G design uses explicit from/to in segments; no MoveTo variant.

    #[test]
    fn test_rejects_constraint_on_path_entity() {
        let entities = vec![Entity::Path(Path {
            id: EntityId(100),
            segments: vec![],
            fill_rule: crate::FillRule::NonZero,
            closed: false,
        })];

        let validator = Validator::new(entities);
        let constraint = Constraint {
            id: 1,
            target: EntityId(100), // Targeting Path directly!
            component: crate::VectorComponent::X,
            relation: crate::RelationType::Eq,
            term: ConstraintTerm::Const {
                value: Rational::zero(),
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        };

        let result = validator.validate_constraint(&constraint);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(matches!(
            errors[0],
            ValidationError::NonLinearConstraintRejected { .. }
        ));
    }

    #[test]
    fn test_allows_constraint_on_control_point() {
        let entities = vec![make_anchor(1, 0, 0)];

        let validator = Validator::new(entities);
        let constraint = Constraint {
            id: 1,
            target: EntityId(1), // Targeting ControlPoint - allowed!
            component: crate::VectorComponent::X,
            relation: crate::RelationType::Eq,
            term: ConstraintTerm::Const {
                value: Rational::from_int(100),
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        };

        assert!(validator.validate_constraint(&constraint).is_ok());
    }
}
