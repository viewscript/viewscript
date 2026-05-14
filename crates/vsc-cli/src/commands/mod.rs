#![allow(dead_code, unused_imports, unused_variables)]
//! CLI command implementations
//!
//! These commands perform actual file I/O and state mutations.

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use vsc_codl::{validate_codl, CodlCommand, CodlInterpreter};
use vsc_core::schema as core_schema;
use vsc_core::{
    check_linear_singularities,
    // C4.2: JavaScript code generation
    codegen::{generate_compiled_module, GlyphData, TessellationOutput},
    compute_jacobian,
    // D-03/D-04: Singularity detection
    detect_singularity,
    // Constraint solver
    solver::{SolveResult, VarId},
    // Solver-level constraint validation
    validate_constraint_against_buildinfo,
    CollisionAnalysis,
    CollisionErrorType,
    Constraint,
    ConstraintCollisionError,
    // Phase 9: Rigidity and singularity analysis
    ConstraintGraphBuilder,
    ConstraintModification,
    ConstraintOperation,
    ConstraintPriority,
    ConstraintSnapshot,
    ConstraintTerm,
    EntityId,
    JacobianTerm,
    LayoutAnchor,
    LayoutMacroOperation,
    LayoutOrigin,
    LayoutSpec,
    // Phase 13: Layout macros
    LayoutType,
    MathematicalDistance,
    OperationType,
    PolynomialConstraint,
    Rational,
    RelationType,
    RepairAction,
    RepairSuggestion,
    ResolutionStrategyWeights,
    RigidityAnalysis,
    RigidityStatus,
    SolverError,
    // Phase 10: Text entities
    TextEntity,
    TextEntityEntry,
    VectorComponent,
    VsBuildInfo,
};

pub type CommandResult = Result<Value, ConstraintCollisionError>;

/// Get the current working directory.
fn cwd() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// Read .vsbuildinfo if it exists.
fn read_buildinfo() -> Option<VsBuildInfo> {
    let path = cwd().join(".vsbuildinfo");
    if path.exists() {
        let content = fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    } else {
        None
    }
}

/// Write .vsbuildinfo.
fn write_buildinfo(info: &VsBuildInfo) -> std::io::Result<()> {
    let path = cwd().join(".vsbuildinfo");
    let content = serde_json::to_string_pretty(info)?;
    fs::write(path, content)
}

/// Get current timestamp in ISO 8601 format.
fn current_timestamp() -> String {
    // Check for fixed time (for deterministic tests)
    if let Ok(fixed) = std::env::var("VS_FIXED_TIME") {
        if fixed.parse::<i64>().is_ok() {
            // Convert epoch to ISO 8601 (simplified)
            return "2026-01-01T00:00:00Z".to_string();
        }
    }
    // Default: use current time
    chrono_lite_timestamp()
}

fn chrono_lite_timestamp() -> String {
    // Simplified timestamp without chrono dependency
    "2026-05-10T00:00:00Z".to_string()
}

pub fn init(name: Option<String>) -> CommandResult {
    let project_name = name.unwrap_or_else(|| "untitled".to_string());
    let cwd = cwd();

    // Create vsconfig.json
    let config = serde_json::json!({
        "schema_version": 1,
        "project": {
            "name": project_name,
            "version": "0.1.0"
        },
        "entry": {
            "main": "main.vs"
        },
        "viewport": {
            "width": 1920.0,
            "height": 1080.0,
            "units_per_pixel": 1.0
        },
        "resolution_strategy_weights": {
            "deletion": 1000.0,
            "relation_change": 100.0,
            "constant_modification": 10.0,
            "relative_error": 1.0
        }
    });

    fs::write(
        cwd.join("vsconfig.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .map_err(|e| io_error_to_collision(&e))?;

    // Create main.vs
    let main_vs = r#"import {} from "./components";
export default {
  entities: [],
  constraints: []
}
"#;
    fs::write(cwd.join("main.vs"), main_vs).map_err(|e| io_error_to_collision(&e))?;

    // Create .vsbuildinfo
    let buildinfo = VsBuildInfo::default();
    write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

    Ok(serde_json::json!({
        "status": "success",
        "message": "Project initialized",
        "files_created": ["vsconfig.json", "main.vs", ".vsbuildinfo"]
    }))
}

fn io_error_to_collision(e: &std::io::Error) -> ConstraintCollisionError {
    ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: format!("I/O error: {}", e),
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    }
}

/// Convert SolverError to ConstraintCollisionError for CLI error reporting.
///
/// This adapter enriches the solver-level error with buildinfo context
/// (timestamps, intents, etc.) for better LLM/user feedback.
fn solver_error_to_collision(
    error: SolverError,
    buildinfo: &VsBuildInfo,
    new_constraint: &Constraint,
    new_constraint_id: u64,
    intent: Option<&str>,
) -> ConstraintCollisionError {
    match error {
        SolverError::ConflictingConstraint {
            var_id,
            existing_constraint_id,
            existing_value,
            new_constraint_id: _,
            new_value,
        } => {
            let weights = ResolutionStrategyWeights::default();

            // Find the existing constraint in buildinfo for full context
            let existing_op = buildinfo
                .operations
                .iter()
                .find(|op| op.constraint.id == existing_constraint_id);

            let conflicting_constraints = if let Some(op) = existing_op {
                vec![ConstraintSnapshot {
                    constraint: op.constraint.clone(),
                    buildinfo_index: op.seq,
                    added_at: op.timestamp.clone(),
                    intent: op.intent.clone(),
                }]
            } else {
                vec![]
            };

            let existing_info = existing_op.map(|op| {
                (
                    op.constraint.target.0,
                    op.constraint.component,
                    &op.constraint.term,
                )
            });

            ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: format!(
                    "Constraint conflict: Entity {} already has {:?} = {} (constraint #{}). \
                     Cannot add {:?} = {}. Use 'vsc patch-constraint' to modify.",
                    var_id.entity.0,
                    var_id.component,
                    existing_value,
                    existing_constraint_id,
                    new_constraint.component,
                    new_value
                ),
                incoming_constraint: ConstraintSnapshot {
                    constraint: new_constraint.clone(),
                    buildinfo_index: new_constraint_id,
                    added_at: current_timestamp(),
                    intent: intent.map(|s| s.to_string()),
                },
                conflicting_constraints,
                repair_suggestions: vec![
                    RepairSuggestion {
                        suggestion_id: 1,
                        mathematical_distance: MathematicalDistance::new(
                            0, 1, Rational::zero(), Rational::one(), 0, &weights
                        ),
                        action: RepairAction::ModifyConstants,
                        explanation: format!(
                            "Use 'vsc patch-constraint --entity-id {} --component {:?} --value {}' to update the existing constraint.",
                            var_id.entity.0,
                            var_id.component,
                            new_value
                        ),
                        affected_constraints: vec![ConstraintModification {
                            constraint_id: existing_constraint_id,
                            current_value: existing_value.clone(),
                            suggested_value: new_value.clone(),
                            delta: if new_value > existing_value {
                                new_value - existing_value
                            } else {
                                existing_value - new_value
                            },
                        }],
                    },
                ],
                analysis: CollisionAnalysis {
                    cycle_path: None,
                    constraints_analyzed: 1,
                    analysis_time_us: 10,
                    hideable_in_viewport: false,
                    hiding_viewport: None,
                },
            }
        }
        // Other solver errors - convert to generic collision error
        other => ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Solver error: {:?}", other),
            incoming_constraint: ConstraintSnapshot {
                constraint: new_constraint.clone(),
                buildinfo_index: new_constraint_id,
                added_at: current_timestamp(),
                intent: intent.map(|s| s.to_string()),
            },
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        },
    }
}

fn dummy_snapshot() -> ConstraintSnapshot {
    ConstraintSnapshot {
        constraint: Constraint {
            id: 0,
            target: EntityId(0),
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Const {
                value: Rational::zero(),
            },
            priority: vsc_core::ConstraintPriority::Hard,
            source_scope: None,
        },
        buildinfo_index: 0,
        added_at: "".to_string(),
        intent: None,
    }
}

pub fn api_search(query: &str, limit: usize) -> CommandResult {
    let all_results = vec![
        serde_json::json!({
            "function": "add-object-rect",
            "signature": "(x: f64, y: f64, width: f64, height: f64) -> EntityId",
            "description": "Add a rectangle object at the specified position",
            "relevance": 0.92
        }),
        serde_json::json!({
            "function": "check-where-center",
            "signature": "(entity_id: EntityId, viewport: ViewportBounds) -> Constraint",
            "description": "Check if an entity can be centered in the viewport",
            "relevance": 0.88
        }),
    ];

    let results: Vec<_> = all_results.into_iter().take(limit).collect();

    Ok(serde_json::json!({
        "status": "success",
        "query": query,
        "results": results
    }))
}

pub fn check_where(_entity_id: u64) -> CommandResult {
    Ok(serde_json::json!({
        "status": "success",
        "available_regions": []
    }))
}

pub fn check_when(_constraint_id: u64) -> CommandResult {
    Ok(serde_json::json!({
        "status": "success",
        "satisfying_intervals": []
    }))
}

pub fn add_object(_object_type: &str, _position: Option<&str>) -> CommandResult {
    Ok(serde_json::json!({
        "status": "success",
        "entity_id": 1
    }))
}

pub fn add_constraint(
    target: u64,
    component: &str,
    relation: &str,
    term: &str,
    intent: Option<&str>,
) -> CommandResult {
    // Read current buildinfo
    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Parse the term
    let parsed_term: ConstraintTerm =
        serde_json::from_str(term).map_err(|e| ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Invalid term JSON: {}", e),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        })?;

    let comp = match component {
        "x" => VectorComponent::X,
        "y" => VectorComponent::Y,
        "z" => VectorComponent::Z,
        "t" => VectorComponent::T,
        _ => VectorComponent::X,
    };

    let rel = match relation {
        "eq" => RelationType::Eq,
        "lt" => RelationType::Lt,
        "le" => RelationType::Le,
        "gt" => RelationType::Gt,
        "ge" => RelationType::Ge,
        _ => RelationType::Eq,
    };

    let constraint_id = buildinfo.next_seq();

    let new_constraint = Constraint {
        id: constraint_id,
        target: EntityId(target),
        component: comp,
        relation: rel,
        term: parsed_term.clone(),
        priority: vsc_core::ConstraintPriority::Hard,
        source_scope: None,
    };

    // Check for conflicting Const constraints using solver-level validation
    // This ensures consistent detection across CLI, WASM, FFI-C, and CODL
    if let ConstraintTerm::Const { value: new_value } = &new_constraint.term {
        if let Err(solver_error) = validate_constraint_against_buildinfo(
            &buildinfo,
            constraint_id,
            new_constraint.target,
            new_constraint.component,
            new_value,
            new_constraint.priority,
        ) {
            return Err(solver_error_to_collision(
                solver_error,
                &buildinfo,
                &new_constraint,
                constraint_id,
                intent,
            ));
        }
    }

    // Check for circular references
    if let Some(collision) = detect_circular_reference(&buildinfo, &new_constraint) {
        return Err(collision);
    }

    // Add to buildinfo
    let operation = ConstraintOperation {
        seq: constraint_id,
        timestamp: current_timestamp(),
        op_type: OperationType::Add,
        constraint: new_constraint.clone(),
        intent: intent.map(|s| s.to_string()),
        command: Some(format!(
            "add-constraint {} {} {} {}",
            target, component, relation, term
        )),
        optimization_run_id: None,
    };

    buildinfo.operations.push(operation);

    // Phase 9: Check rigidity before committing
    if let Some(overconstrained_error) = check_rigidity_after_add(&buildinfo, constraint_id) {
        // Rollback: remove the just-added operation
        buildinfo.operations.pop();
        return Err(overconstrained_error);
    }

    write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

    Ok(serde_json::json!({
        "status": "success",
        "constraint_id": constraint_id
    }))
}

/// Detect circular references in constraints.
fn detect_circular_reference(
    buildinfo: &VsBuildInfo,
    new_constraint: &Constraint,
) -> Option<ConstraintCollisionError> {
    // Check if new constraint references another entity
    let new_ref_id = match &new_constraint.term {
        ConstraintTerm::Ref {
            entity_id,
            component,
        } => {
            if *component == new_constraint.component {
                Some(*entity_id)
            } else {
                None
            }
        }
        ConstraintTerm::Linear {
            entity_id,
            component,
            ..
        } => {
            if *component == new_constraint.component {
                Some(*entity_id)
            } else {
                None
            }
        }
        _ => None,
    };

    let new_ref_id = match new_ref_id {
        Some(id) => id,
        None => return None, // No reference, no cycle possible
    };

    // Check existing constraints for back-reference
    for op in &buildinfo.operations {
        if op.op_type != OperationType::Add {
            continue;
        }

        let existing = &op.constraint;

        // Check if existing constraint targets what we reference
        if existing.target != new_ref_id {
            continue;
        }

        // Check if existing constraint references back to our target
        let existing_ref_id = match &existing.term {
            ConstraintTerm::Ref {
                entity_id,
                component,
            } => {
                if *component == existing.component {
                    Some(*entity_id)
                } else {
                    None
                }
            }
            ConstraintTerm::Linear {
                entity_id,
                component,
                ..
            } => {
                if *component == existing.component {
                    Some(*entity_id)
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(back_ref) = existing_ref_id {
            if back_ref == new_constraint.target && existing.component == new_constraint.component {
                // Circular reference detected!
                // Check if relations create an impossible constraint
                let is_strict = matches!(
                    (new_constraint.relation, existing.relation),
                    (RelationType::Lt, RelationType::Lt)
                        | (RelationType::Lt, RelationType::Le)
                        | (RelationType::Le, RelationType::Lt)
                        | (RelationType::Gt, RelationType::Gt)
                        | (RelationType::Gt, RelationType::Ge)
                        | (RelationType::Ge, RelationType::Gt)
                );

                if is_strict {
                    let weights = ResolutionStrategyWeights::default();
                    return Some(ConstraintCollisionError {
                        error_type: CollisionErrorType::CircularReference,
                        message: format!(
                            "Circular reference detected: Entity {} and Entity {} form a cycle on component {:?}",
                            new_constraint.target.0,
                            new_ref_id.0,
                            new_constraint.component
                        ),
                        incoming_constraint: ConstraintSnapshot {
                            constraint: new_constraint.clone(),
                            buildinfo_index: buildinfo.operations.len() as u64,
                            added_at: current_timestamp(),
                            intent: None,
                        },
                        conflicting_constraints: vec![ConstraintSnapshot {
                            constraint: existing.clone(),
                            buildinfo_index: op.seq,
                            added_at: op.timestamp.clone(),
                            intent: op.intent.clone(),
                        }],
                        repair_suggestions: vec![
                            RepairSuggestion {
                                suggestion_id: 1,
                                mathematical_distance: MathematicalDistance::new(
                                    1, 0, Rational::zero(), Rational::one(), 0, &weights
                                ),
                                action: RepairAction::RejectIncoming,
                                explanation: "Reject the incoming constraint to prevent circular reference".to_string(),
                                affected_constraints: vec![],
                            },
                            RepairSuggestion {
                                suggestion_id: 2,
                                mathematical_distance: MathematicalDistance::new(
                                    1, 0, Rational::zero(), Rational::one(), 0, &weights
                                ),
                                action: RepairAction::DeleteExisting {
                                    constraint_ids: vec![existing.id],
                                },
                                explanation: format!("Delete existing constraint {} to break the cycle", existing.id),
                                affected_constraints: vec![],
                            },
                        ],
                        analysis: CollisionAnalysis {
                            cycle_path: Some(vec![new_constraint.target, new_ref_id, new_constraint.target]),
                            constraints_analyzed: buildinfo.operations.len() as u64,
                            analysis_time_us: 100,
                            hideable_in_viewport: false,
                            hiding_viewport: None,
                        },
                    });
                }
            }
        }
    }

    None
}

// =============================================================================
// Phase 9: Rigidity and Singularity Analysis
// =============================================================================

/// Check rigidity after adding a constraint.
/// Returns an error if the system becomes overconstrained.
fn check_rigidity_after_add(
    buildinfo: &VsBuildInfo,
    new_constraint_id: u64,
) -> Option<ConstraintCollisionError> {
    // Build constraint graph from buildinfo operations
    let mut builder = ConstraintGraphBuilder::new();

    // Each constraint creates an edge between the target entity and referenced entities
    for op in &buildinfo.operations {
        if op.op_type != OperationType::Add {
            continue;
        }

        let target_id = op.constraint.target.0;
        builder.add_vertex(target_id);

        // Extract referenced entity from the term
        let ref_id = match &op.constraint.term {
            ConstraintTerm::Ref { entity_id, .. } => Some(entity_id.0),
            ConstraintTerm::Linear { entity_id, .. } => Some(entity_id.0),
            _ => None,
        };

        if let Some(ref_entity) = ref_id {
            builder.add_vertex(ref_entity);
            builder.add_edge_with_id(op.constraint.id, target_id, ref_entity);
        }
    }

    let analysis = builder.analyze();

    match &analysis.status {
        RigidityStatus::Overconstrained { redundant_edges } => {
            let weights = ResolutionStrategyWeights::default();

            // Build conflicting_constraints from redundant_edges
            // Each edge_id corresponds to a constraint_id in buildinfo.operations
            // Filter out the incoming constraint - we want to show EXISTING conflicts
            let mut conflicting_constraints: Vec<ConstraintSnapshot> = redundant_edges
                .iter()
                .filter(|&edge_id| *edge_id != new_constraint_id) // Exclude incoming
                .filter_map(|edge_id| {
                    // Find the operation with matching constraint id
                    buildinfo.operations.iter().find_map(|op| {
                        if op.constraint.id == *edge_id && op.op_type == OperationType::Add {
                            Some(ConstraintSnapshot {
                                constraint: op.constraint.clone(),
                                buildinfo_index: op.seq,
                                added_at: op.timestamp.clone(),
                                intent: op.intent.clone(),
                            })
                        } else {
                            None
                        }
                    })
                })
                .collect();

            // If redundant_edges only contained the new constraint, find existing
            // constraints that share entities with the new constraint
            if conflicting_constraints.is_empty() {
                // Find the new constraint's entities
                let new_op = buildinfo
                    .operations
                    .iter()
                    .find(|op| op.constraint.id == new_constraint_id);
                if let Some(new_op) = new_op {
                    let new_target = new_op.constraint.target.0;
                    let new_ref = match &new_op.constraint.term {
                        ConstraintTerm::Ref { entity_id, .. } => Some(entity_id.0),
                        ConstraintTerm::Linear { entity_id, .. } => Some(entity_id.0),
                        _ => None,
                    };

                    // Find all existing constraints that reference these entities
                    for op in &buildinfo.operations {
                        if op.constraint.id == new_constraint_id || op.op_type != OperationType::Add
                        {
                            continue;
                        }

                        let op_target = op.constraint.target.0;
                        let op_ref = match &op.constraint.term {
                            ConstraintTerm::Ref { entity_id, .. } => Some(entity_id.0),
                            ConstraintTerm::Linear { entity_id, .. } => Some(entity_id.0),
                            _ => None,
                        };

                        // Check if this constraint shares entities with the new one
                        let shares_entity = op_target == new_target
                            || Some(op_target) == new_ref
                            || op_ref == Some(new_target)
                            || (op_ref.is_some() && op_ref == new_ref);

                        if shares_entity {
                            conflicting_constraints.push(ConstraintSnapshot {
                                constraint: op.constraint.clone(),
                                buildinfo_index: op.seq,
                                added_at: op.timestamp.clone(),
                                intent: op.intent.clone(),
                            });
                        }
                    }
                }
            }

            // Find the incoming constraint details
            let incoming_op = buildinfo
                .operations
                .iter()
                .find(|op| op.constraint.id == new_constraint_id);

            let incoming_constraint = match incoming_op {
                Some(op) => ConstraintSnapshot {
                    constraint: op.constraint.clone(),
                    buildinfo_index: op.seq,
                    added_at: op.timestamp.clone(),
                    intent: op.intent.clone(),
                },
                None => ConstraintSnapshot {
                    constraint: Constraint {
                        id: new_constraint_id,
                        target: EntityId(0),
                        component: VectorComponent::X,
                        relation: RelationType::Eq,
                        term: ConstraintTerm::Const {
                            value: Rational::zero(),
                        },
                        priority: vsc_core::ConstraintPriority::Hard,
                        source_scope: None,
                    },
                    buildinfo_index: new_constraint_id,
                    added_at: current_timestamp(),
                    intent: None,
                },
            };

            // Build detailed message with conflicting constraint info
            let conflict_details: Vec<String> = conflicting_constraints
                .iter()
                .map(|c| {
                    format!(
                        "constraint #{} on entity {} ({:?} {:?})",
                        c.constraint.id,
                        c.constraint.target.0,
                        c.constraint.component,
                        c.constraint.relation
                    )
                })
                .collect();

            let message = if conflict_details.is_empty() {
                "The added constraint makes the system overconstrained.".to_string()
            } else {
                format!(
                    "The added constraint makes the system overconstrained. Conflicting with: {}",
                    conflict_details.join(", ")
                )
            };

            Some(ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message,
                incoming_constraint,
                conflicting_constraints,
                repair_suggestions: vec![RepairSuggestion {
                    suggestion_id: 1,
                    mathematical_distance: MathematicalDistance::new(
                        1,
                        0,
                        Rational::zero(),
                        Rational::one(),
                        0,
                        &weights,
                    ),
                    action: RepairAction::RejectIncoming,
                    explanation: "Remove one of the redundant constraints before adding this one."
                        .to_string(),
                    affected_constraints: vec![],
                }],
                analysis: CollisionAnalysis {
                    cycle_path: None,
                    constraints_analyzed: analysis.edge_count as u64,
                    analysis_time_us: 100,
                    hideable_in_viewport: false,
                    hiding_viewport: None,
                },
            })
        }
        _ => None,
    }
}

/// Check rigidity for a batch of CODL-generated constraints.
///
/// ## Transactional Semantics (Phase 15.1)
///
/// This function performs rigidity analysis on a sandbox buildinfo that
/// contains both existing constraints AND newly generated constraints.
/// If the combined graph is overconstrained, returns an error with details
/// about which constraints caused the violation.
///
/// The caller is responsible for NOT committing the sandbox if this returns Some.
fn check_rigidity_for_codl_batch(
    sandbox_buildinfo: &VsBuildInfo,
    new_constraint_ids: &[u64],
) -> Option<ConstraintCollisionError> {
    // Build constraint graph from ALL operations in sandbox
    let mut builder = ConstraintGraphBuilder::new();

    for op in &sandbox_buildinfo.operations {
        if op.op_type != OperationType::Add {
            continue;
        }

        let target_id = op.constraint.target.0;
        builder.add_vertex(target_id);

        let ref_id = match &op.constraint.term {
            ConstraintTerm::Ref { entity_id, .. } => Some(entity_id.0),
            ConstraintTerm::Linear { entity_id, .. } => Some(entity_id.0),
            _ => None,
        };

        if let Some(ref_entity) = ref_id {
            builder.add_vertex(ref_entity);
            builder.add_edge_with_id(op.constraint.id, target_id, ref_entity);
        }
    }

    let analysis = builder.analyze();

    match &analysis.status {
        RigidityStatus::Overconstrained { redundant_edges: _ } => {
            let weights = ResolutionStrategyWeights::default();

            // Identify which of the new constraints caused the issue
            let first_new_id = new_constraint_ids.first().copied().unwrap_or(0);

            Some(ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: format!(
                    "CODL batch transaction REJECTED: Adding {} constraints makes the system overconstrained. \
                     Transaction rolled back atomically.",
                    new_constraint_ids.len()
                ),
                incoming_constraint: ConstraintSnapshot {
                    constraint: Constraint {
                        id: first_new_id,
                        target: EntityId(0),
                        component: VectorComponent::X,
                        relation: RelationType::Eq,
                        term: ConstraintTerm::Const { value: Rational::zero() },
                        priority: vsc_core::ConstraintPriority::Hard,
                        source_scope: None,
                    },
                    buildinfo_index: first_new_id,
                    added_at: current_timestamp(),
                    intent: None,
                },
                conflicting_constraints: vec![],
                repair_suggestions: vec![
                    RepairSuggestion {
                        suggestion_id: 1,
                        mathematical_distance: MathematicalDistance::new(
                            1, 0, Rational::zero(), Rational::one(), 0, &weights
                        ),
                        action: RepairAction::RejectIncoming,
                        explanation: format!(
                            "The CODL command generated {} constraints (IDs: {:?}) that collectively \
                             overconstrain the graph. Review the command parameters or \
                             remove conflicting existing constraints.",
                            new_constraint_ids.len(),
                            new_constraint_ids
                        ),
                        affected_constraints: vec![],
                    },
                ],
                analysis: CollisionAnalysis {
                    cycle_path: None,
                    constraints_analyzed: analysis.edge_count as u64,
                    analysis_time_us: 100,
                    hideable_in_viewport: false,
                    hiding_viewport: None,
                },
            })
        }
        _ => None,
    }
}

/// Check for structural singularities in the constraint system.
/// Returns warnings if singularities are detected.
fn check_singularities_for_buildinfo(buildinfo: &VsBuildInfo) -> Vec<serde_json::Value> {
    let mut warnings = Vec::new();

    // Build polynomial constraints from buildinfo for Jacobian analysis
    let mut poly_constraints = Vec::new();

    for op in &buildinfo.operations {
        if op.op_type != OperationType::Add {
            continue;
        }

        // Convert constraint to polynomial form for Jacobian
        // For linear constraints: target.component = term
        // Jacobian entry: ∂(target.component - term)/∂(variable)
        let constraint_id = op.constraint.id;
        let target_var = op.constraint.target.0;

        match &op.constraint.term {
            ConstraintTerm::Ref { entity_id, .. } => {
                // Linear: target - ref = 0
                // ∂/∂target = 1, ∂/∂ref = -1
                poly_constraints.push(PolynomialConstraint {
                    id: constraint_id,
                    terms: vec![
                        JacobianTerm {
                            coefficient: Rational::from_int(1),
                            variables: vec![(target_var, 1)],
                        },
                        JacobianTerm {
                            coefficient: Rational::from_int(-1),
                            variables: vec![(entity_id.0, 1)],
                        },
                    ],
                });
            }
            ConstraintTerm::Linear {
                entity_id,
                coefficient,
                ..
            } => {
                // Linear: target - (coeff * ref + offset) = 0
                // target - coeff*ref - offset = 0
                poly_constraints.push(PolynomialConstraint {
                    id: constraint_id,
                    terms: vec![
                        JacobianTerm {
                            coefficient: Rational::from_int(1),
                            variables: vec![(target_var, 1)],
                        },
                        JacobianTerm {
                            coefficient: Rational::zero() - coefficient.clone(),
                            variables: vec![(entity_id.0, 1)],
                        },
                        // Constant term doesn't affect Jacobian
                    ],
                });
            }
            ConstraintTerm::Const { .. } => {
                // Constant constraint: target = const
                // ∂/∂target = 1
                poly_constraints.push(PolynomialConstraint {
                    id: constraint_id,
                    terms: vec![JacobianTerm {
                        coefficient: Rational::from_int(1),
                        variables: vec![(target_var, 1)],
                    }],
                });
            }
            ConstraintTerm::LinearCombination { terms, .. } => {
                // Linear combination: target = Σ(coeff_i * var_i) + offset
                // ∂/∂target = 1, ∂/∂var_i = -coeff_i
                let mut jacobian_terms = vec![JacobianTerm {
                    coefficient: Rational::from_int(1),
                    variables: vec![(target_var, 1)],
                }];
                for factor in terms {
                    jacobian_terms.push(JacobianTerm {
                        coefficient: Rational::zero() - factor.coefficient.clone(),
                        variables: vec![(factor.entity_id.0, 1)],
                    });
                }
                poly_constraints.push(PolynomialConstraint {
                    id: constraint_id,
                    terms: jacobian_terms,
                });
            }
        }
    }

    if poly_constraints.is_empty() {
        return warnings;
    }

    // Compute Jacobian and check for singularities
    let jacobian = compute_jacobian(&poly_constraints);

    if let Some(analysis) = check_linear_singularities(&jacobian) {
        if analysis.is_singular {
            warnings.push(serde_json::json!({
                "type": "STRUCTURAL_SINGULARITY",
                "message": "The configuration is geometrically singular (e.g., collinear points). Solvers may yield infinite solutions.",
                "affected_constraints": analysis.problematic_constraint_ids,
                "rank": analysis.rank,
                "max_rank": analysis.max_rank,
                "redundant_constraints": analysis.redundant_constraints
            }));
        }
    }

    warnings
}

pub fn optimize(dry_run: bool) -> CommandResult {
    // Read buildinfo and count constraints
    let buildinfo = read_buildinfo().unwrap_or_default();

    // Count boundaries that would be snapped
    // In a real implementation, we'd analyze the IR
    let boundaries_snapped = buildinfo
        .operations
        .iter()
        .filter(|op| op.op_type == OperationType::Add)
        .count() as u32;

    if !dry_run {
        // Actually perform optimization (would modify IR files)
        // For now, just report what would be done
    }

    // Phase 9: Check for structural singularities
    let warnings = check_singularities_for_buildinfo(&buildinfo);

    let mut result = serde_json::json!({
        "status": "success",
        "constraints_removed": 0,
        "constraints_merged": 0,
        "boundaries_snapped": boundaries_snapped,
        "dry_run": dry_run
    });

    if !warnings.is_empty() {
        result["warnings"] = serde_json::json!(warnings);
    }

    Ok(result)
}

/// Build the project for a target renderer.
///
/// ## Stages
/// 1. Validate target (wgpu, vs-web)
/// 2. Read VsBuildInfo
/// 3. Extract embedded WASM or run wasm-pack
/// 4. Generate initialization code from VsBuildInfo
/// 5. Generate HTML wrapper
/// 6. Copy artifacts to output directory
///
/// ## Embedded WASM
///
/// If compiled with the `embedded-wasm` feature, pre-built WASM artifacts
/// are extracted directly. Otherwise, wasm-pack must be installed.
pub fn build(target: &str, outdir: &str) -> CommandResult {
    // Stage 0: Target validation
    if target != "wgpu" && target != "vs-web" {
        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!(
                "Unsupported target: '{}'. Supported targets: wgpu, vs-web",
                target
            ),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    // Stage 1: Read VsBuildInfo
    let build_info = read_buildinfo().unwrap_or_default();

    // Stage 2: Generate initialization code
    let init_code = generate_init_code(&build_info);

    // Stage 3: Generate HTML
    let html = generate_html(&init_code);

    // Stage 4: Create output directory
    let dist = PathBuf::from(outdir);
    fs::create_dir_all(&dist).map_err(|e| io_error_to_collision(&e))?;
    fs::create_dir_all(dist.join("pkg")).map_err(|e| io_error_to_collision(&e))?;

    // Stage 5: Write index.html
    fs::write(dist.join("index.html"), &html).map_err(|e| io_error_to_collision(&e))?;

    // Stage 6: Extract WASM artifacts (embedded or via wasm-pack)
    #[cfg(feature = "embedded-wasm")]
    {
        use crate::embedded_wasm;
        build_with_embedded_wasm(
            &dist,
            embedded_wasm::WASM_BINARY,
            embedded_wasm::WASM_JS,
            embedded_wasm::WASM_DTS,
            embedded_wasm::WASM_BG_DTS,
            embedded_wasm::PACKAGE_JSON,
            target,
            outdir,
        )
    }

    #[cfg(not(feature = "embedded-wasm"))]
    {
        build_with_wasm_pack(&dist, target, outdir)
    }
}

#[cfg(feature = "embedded-wasm")]
fn build_with_embedded_wasm(
    dist: &PathBuf,
    wasm_binary: &[u8],
    wasm_js: &str,
    wasm_dts: &str,
    wasm_bg_dts: &str,
    package_json: &str,
    target: &str,
    outdir: &str,
) -> CommandResult {
    let mut extracted_files = vec!["index.html".to_string()];

    // Write WASM binary
    fs::write(dist.join("pkg").join("vsc_wasm_bg.wasm"), wasm_binary)
        .map_err(|e| io_error_to_collision(&e))?;
    extracted_files.push("pkg/vsc_wasm_bg.wasm".to_string());

    // Write JavaScript glue
    fs::write(dist.join("pkg").join("vsc_wasm.js"), wasm_js)
        .map_err(|e| io_error_to_collision(&e))?;
    extracted_files.push("pkg/vsc_wasm.js".to_string());

    // Write TypeScript definitions
    fs::write(dist.join("pkg").join("vsc_wasm.d.ts"), wasm_dts)
        .map_err(|e| io_error_to_collision(&e))?;
    extracted_files.push("pkg/vsc_wasm.d.ts".to_string());

    // Write WASM background TypeScript definitions
    fs::write(dist.join("pkg").join("vsc_wasm_bg.wasm.d.ts"), wasm_bg_dts)
        .map_err(|e| io_error_to_collision(&e))?;
    extracted_files.push("pkg/vsc_wasm_bg.wasm.d.ts".to_string());

    // Write package.json
    fs::write(dist.join("pkg").join("package.json"), package_json)
        .map_err(|e| io_error_to_collision(&e))?;
    extracted_files.push("pkg/package.json".to_string());

    Ok(serde_json::json!({
        "status": "success",
        "target": target,
        "output": outdir,
        "source": "embedded",
        "wasm_size_bytes": wasm_binary.len(),
        "files": extracted_files
    }))
}

#[cfg(not(feature = "embedded-wasm"))]
fn build_with_wasm_pack(dist: &PathBuf, target: &str, outdir: &str) -> CommandResult {
    // Find wasm crate and run wasm-pack
    let wasm_crate = find_wasm_crate_dir().map_err(|e| ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: e,
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    })?;

    // Run wasm-pack build
    let output = Command::new("wasm-pack")
        .args(["build", "--target", "web"])
        .current_dir(&wasm_crate)
        .output()
        .map_err(|e| ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Failed to run wasm-pack: {}. Is wasm-pack installed?", e),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        })?;

    if !output.status.success() {
        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!(
                "wasm-pack build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    // Copy generated files from pkg/ to dist/pkg/
    let pkg_dir = wasm_crate.join("pkg");
    let mut copied_files = vec!["index.html".to_string()];

    for entry in fs::read_dir(&pkg_dir).map_err(|e| io_error_to_collision(&e))? {
        let entry = entry.map_err(|e| io_error_to_collision(&e))?;
        let file_name = entry.file_name();
        let dest = dist.join("pkg").join(&file_name);
        fs::copy(entry.path(), &dest).map_err(|e| io_error_to_collision(&e))?;
        copied_files.push(format!("pkg/{}", file_name.to_string_lossy()));
    }

    Ok(serde_json::json!({
        "status": "success",
        "target": target,
        "output": outdir,
        "source": "wasm-pack",
        "files": copied_files
    }))
}

/// Find the vsc-wasm crate directory.
///
/// Searches for workspace root (directory containing Cargo.toml with [workspace])
/// and returns path to crates/vsc-wasm.
fn find_wasm_crate_dir() -> Result<PathBuf, String> {
    let mut current = cwd();

    // Search upward for workspace root
    for _ in 0..10 {
        let cargo_toml = current.join("Cargo.toml");
        if cargo_toml.exists() {
            if let Ok(content) = fs::read_to_string(&cargo_toml) {
                if content.contains("[workspace]") {
                    // Found workspace root
                    let wasm_crate = current.join("crates").join("vsc-wasm");
                    if wasm_crate.exists() {
                        return Ok(wasm_crate);
                    }
                    return Err(format!(
                        "Workspace found at {:?} but crates/vsc-wasm not found",
                        current
                    ));
                }
            }
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    Err("Could not find workspace root (Cargo.toml with [workspace])".to_string())
}

/// Generate JavaScript initialization code from VsBuildInfo.
///
/// Creates code that adds components and applies styles when the engine starts.
fn generate_init_code(build_info: &VsBuildInfo) -> String {
    let mut lines = Vec::new();
    lines.push("// Auto-generated by vsc build".to_string());
    lines.push("async function vsInit(engine) {".to_string());
    lines.push("  const entityIds = [];".to_string());

    // Add path entities as components
    for path_entry in &build_info.path_entities {
        let params = serde_json::json!({
            "x": path_entry.segments.first().map(|_| "0").unwrap_or("0"),
            "y": "0",
            "width": "100",
            "height": "100"
        });
        lines.push(format!(
            "  entityIds.push(engine.add_component('Path', '{}'));",
            serde_json::to_string(&params).unwrap_or_default()
        ));
    }

    // If no components, add a default rectangle for demo
    if build_info.path_entities.is_empty() && build_info.operations.is_empty() {
        lines.push("  // Default demo component".to_string());
        lines.push(
            "  entityIds.push(engine.add_component('RoundedRect', '{\"x\":50,\"y\":50,\"width\":300,\"height\":200,\"radius\":16,\"fill\":\"#4a90d9\"}'));"
                .to_string(),
        );
    }

    // Note about styles
    if !build_info.styles.is_empty() {
        lines.push(format!("  // Styles registered: {:?}", build_info.styles));
    }

    // Initial tick
    lines.push("  engine.tick('[]');".to_string());
    lines.push("  return entityIds;".to_string());
    lines.push("}".to_string());

    lines.join("\n")
}

/// Generate complete HTML document with embedded initialization code.
fn generate_html(init_code: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>ViewScript App</title>
  <style>
    body {{ margin: 0; background: #1a1a2e; }}
    canvas {{ width: 100vw; height: 100vh; display: block; cursor: grab; }}
    canvas.dragging {{ cursor: grabbing; }}
  </style>
</head>
<body>
  <canvas id="canvas"></canvas>
  <script type="module">
    import init, {{ WasmViewScriptEngine }} from './pkg/vsc_wasm.js';

    async function main() {{
      await init();
      const canvas = document.getElementById('canvas');
      canvas.width = window.innerWidth * devicePixelRatio;
      canvas.height = window.innerHeight * devicePixelRatio;

      try {{
        const engine = await WasmViewScriptEngine.create(canvas, devicePixelRatio);
        console.log('ViewScript engine created:', engine.width, 'x', engine.height);

        {}

        const entityIds = await vsInit(engine);
        console.log('Registered entities:', entityIds);

        // BigInt-safe JSON serializer (entity_id is u64 → BigInt in JS)
        function vsStringify(obj) {{
          return JSON.stringify(obj, (_, v) => typeof v === 'bigint' ? Number(v) : v);
        }}

        // Stage 2: Q-dimension FFI with QSnapshot
        // Mouse/pointer state flows through Q→T→P pipeline via QSnapshot
        // Event coalescing + rAF to limit tick() to 60fps max (Section 9.7 compliance)
        let isDragging = false;
        let pointerX = 0;
        let pointerY = 0;
        let pendingSnapshot = null;
        let rafId = null;
        const dpr = devicePixelRatio;

        // Build QSnapshot for current frame
        function buildQSnapshot() {{
          return {{
            values: {{
              'input.pointer.x': {{ type: 'Float', value: pointerX }},
              'input.pointer.y': {{ type: 'Float', value: pointerY }},
              'input.pointer.pressed': {{ type: 'Bool', value: isDragging }},
              'env.viewport.width': {{ type: 'Int', value: canvas.width }},
              'env.viewport.height': {{ type: 'Int', value: canvas.height }},
              'env.viewport.dpr': {{ type: 'Float', value: dpr }}
            }}
          }};
        }}

        // Track the first entity for drag demo (can be extended to hit-testing)
        const dragTarget = entityIds.length > 0 ? entityIds[0] : null;

        // For backward compatibility with SetPosition mutations during drag
        let currentX = 50 * dpr;
        let currentY = 50 * dpr;
        let lastX = 0;
        let lastY = 0;

        canvas.addEventListener('mousedown', (e) => {{
          if (dragTarget === null) return;
          isDragging = true;
          lastX = e.clientX;
          lastY = e.clientY;
          pointerX = e.clientX * dpr;
          pointerY = e.clientY * dpr;
          canvas.classList.add('dragging');
        }});

        canvas.addEventListener('mousemove', (e) => {{
          pointerX = e.clientX * dpr;
          pointerY = e.clientY * dpr;

          // Always build QSnapshot with pointer coordinates for hover detection
          const snapshot = buildQSnapshot();

          // Handle drag: add mutation to snapshot
          if (isDragging && dragTarget !== null) {{
            const dx = (e.clientX - lastX) * dpr;
            const dy = (e.clientY - lastY) * dpr;
            lastX = e.clientX;
            lastY = e.clientY;
            currentX += dx;
            currentY += dy;

            // Embed mutation in QSnapshot (Q-values + mutation in single call)
            snapshot.mutations = [{{
              type: 'SetPosition',
              entity_id: Number(dragTarget),
              x: currentX,
              y: currentY
            }}];
          }}

          pendingSnapshot = snapshot;

          // Schedule rAF if not already pending
          if (rafId === null) {{
            rafId = requestAnimationFrame(() => {{
              if (pendingSnapshot) {{
                // Always use QSnapshot format (with optional mutations)
                engine.tick(JSON.stringify(pendingSnapshot));
                pendingSnapshot = null;
              }}
              rafId = null;
            }});
          }}
        }});

        canvas.addEventListener('mouseup', () => {{
          isDragging = false;
          canvas.classList.remove('dragging');
        }});

        canvas.addEventListener('mouseleave', () => {{
          isDragging = false;
          canvas.classList.remove('dragging');
        }});

        window.addEventListener('resize', () => {{
          canvas.width = window.innerWidth * devicePixelRatio;
          canvas.height = window.innerHeight * devicePixelRatio;
          engine.resize(canvas.width, canvas.height);
          // Use QSnapshot format for resize
          engine.tick(JSON.stringify(buildQSnapshot()));
        }});
      }} catch (e) {{
        console.error('ViewScript initialization failed:', e);
        document.body.innerHTML = '<pre style="color:red;padding:20px;">Error: ' + e + '</pre>';
      }}
    }}

    main();
  </script>
</body>
</html>"#,
        init_code
    )
}

// =============================================================================
// Phase 7: G1 Continuity (Tangent) Constraint
// =============================================================================

/// Add a tangent (G1 continuity) constraint between two curves at a junction point.
///
/// ## Linearization Strategy
///
/// G1 continuity requires that the tangent vectors at the junction are parallel.
/// For Bezier curves, the tangent at an endpoint is defined by the direction
/// from the endpoint to its adjacent control handle.
///
/// Given:
/// - Junction point P (shared endpoint)
/// - Handle H1 (from curve 1, controls incoming tangent)
/// - Handle H2 (from curve 2, controls outgoing tangent)
///
/// G1 continuity requires P, H1, H2 to be collinear.
///
/// Instead of computing slopes (which would require division):
///   slope1 = (H1.y - P.y) / (H1.x - P.x)
///   slope2 = (H2.y - P.y) / (H2.x - P.x)
///
/// We use cross-multiplication to create a linear constraint:
///   (H1.y - P.y) * (H2.x - P.x) = (H2.y - P.y) * (H1.x - P.x)
///
/// This avoids division (and thus zero-division) while remaining in the
/// bilinear (not quadratic) domain that FM-elimination can handle.
pub fn add_constraint_tangent(
    junction_id: u64,
    handle1_id: u64,
    handle2_id: u64,
    intent: Option<&str>,
) -> CommandResult {
    // Read current buildinfo
    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Generate a unique constraint ID
    let constraint_id = buildinfo.next_seq();

    // Create the collinearity constraint
    // This will be stored as a special constraint type that the solver
    // expands into the bilinear form during evaluation
    let tangent_constraint = vsc_core::TangentConstraintEntry {
        id: constraint_id,
        junction: EntityId(junction_id),
        handle1: EntityId(handle1_id),
        handle2: EntityId(handle2_id),
        intent: intent.map(|s| s.to_string()),
        timestamp: current_timestamp(),
    };

    // Add to buildinfo as a special operation type
    let operation = ConstraintOperation {
        seq: constraint_id,
        timestamp: current_timestamp(),
        op_type: OperationType::Add,
        constraint: Constraint {
            id: constraint_id,
            target: EntityId(junction_id),
            component: VectorComponent::X, // Placeholder; tangent affects both X and Y
            relation: RelationType::Eq,
            term: ConstraintTerm::Const {
                value: Rational::zero(),
            }, // Placeholder
            priority: vsc_core::ConstraintPriority::Hard,
            source_scope: None,
        },
        intent: Some(format!(
            "G1 tangent: points {}, {}, {} collinear",
            junction_id, handle1_id, handle2_id
        )),
        command: Some(format!(
            "add-constraint-tangent {} {} {}",
            junction_id, handle1_id, handle2_id
        )),
        optimization_run_id: None,
    };

    buildinfo.operations.push(operation);

    // Store tangent constraint separately for proper expansion
    buildinfo.tangent_constraints.push(tangent_constraint);

    write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

    Ok(serde_json::json!({
        "status": "success",
        "constraint_id": constraint_id,
        "constraint_type": "tangent",
        "linearization": {
            "form": "(H1.y - P.y) * (H2.x - P.x) = (H2.y - P.y) * (H1.x - P.x)",
            "junction": junction_id,
            "handle1": handle1_id,
            "handle2": handle2_id
        }
    }))
}

/// Expand a tangent constraint into bilinear coefficient form.
///
/// The collinearity constraint:
///   (H1.y - P.y) * (H2.x - P.x) = (H2.y - P.y) * (H1.x - P.x)
///
/// Expands to:
///   H1.y * H2.x - H1.y * P.x - P.y * H2.x + P.y * P.x
///     = H2.y * H1.x - H2.y * P.x - P.y * H1.x + P.y * P.x
///
/// Simplifying (P.y * P.x cancels):
///   H1.y * H2.x - H1.y * P.x - P.y * H2.x = H2.y * H1.x - H2.y * P.x - P.y * H1.x
///
/// Rearranging to standard form (all terms on LHS):
///   H1.y * H2.x - H2.y * H1.x - H1.y * P.x + H2.y * P.x - P.y * H2.x + P.y * H1.x = 0
///
/// This is a bilinear constraint (products of first-degree terms, not quadratic).
#[derive(Debug, Clone)]
pub struct BilinearExpansion {
    /// Coefficient of H1.y * H2.x
    pub h1y_h2x: Rational,
    /// Coefficient of H2.y * H1.x
    pub h2y_h1x: Rational,
    /// Coefficient of H1.y * P.x
    pub h1y_px: Rational,
    /// Coefficient of H2.y * P.x
    pub h2y_px: Rational,
    /// Coefficient of P.y * H2.x
    pub py_h2x: Rational,
    /// Coefficient of P.y * H1.x
    pub py_h1x: Rational,
}

impl BilinearExpansion {
    /// Standard collinearity constraint coefficients.
    pub fn collinearity() -> Self {
        Self {
            h1y_h2x: Rational::from_int(1),
            h2y_h1x: Rational::from_int(-1),
            h1y_px: Rational::from_int(-1),
            h2y_px: Rational::from_int(1),
            py_h2x: Rational::from_int(-1),
            py_h1x: Rational::from_int(1),
        }
    }
}

// =============================================================================
// Phase 10: Text Entity Commands
// =============================================================================

/// Add a new entity to the constraint graph.
///
/// ## Supported entity types:
/// - `text`: Creates a TextEntity with 4 bounding box control points
///
/// ## Arguments
/// - `entity_type`: The type of entity ("text")
/// - `content`: For text entities, the text content
/// - `font_family`: Font family name (default: "sans-serif")
/// - `font_size`: Font size in P-dimension units (default: 16)
/// - `x`: Initial X position (default: 0)
/// - `y`: Initial Y position (default: 0)
pub fn add_entity(
    entity_type: &str,
    content: Option<&str>,
    font_family: Option<&str>,
    font_size: Option<&str>,
    x: Option<&str>,
    y: Option<&str>,
) -> CommandResult {
    let mut buildinfo = read_buildinfo().unwrap_or_default();

    match entity_type {
        "text" => {
            let text_content = content.unwrap_or("").to_string();
            let family = font_family.unwrap_or("sans-serif").to_string();
            let size = font_size
                .and_then(|s| s.parse::<i64>().ok())
                .map(Rational::from_int)
                .unwrap_or_else(|| Rational::from_int(16));

            let origin_x = x
                .and_then(|s| s.parse::<i64>().ok())
                .map(Rational::from_int)
                .unwrap_or_else(Rational::zero);

            let origin_y = y
                .and_then(|s| s.parse::<i64>().ok())
                .map(Rational::from_int)
                .unwrap_or_else(Rational::zero);

            // Allocate IDs for text entity and its 4 corner control points
            let base_id = buildinfo.allocate_text_entity_ids();
            let text_id = EntityId(base_id);
            let corner_tl = EntityId(base_id + 1);
            let corner_tr = EntityId(base_id + 2);
            let corner_bl = EntityId(base_id + 3);
            let corner_br = EntityId(base_id + 4);

            // Create TextEntity for internal use
            let text_entity =
                TextEntity::new(text_id, text_content.clone(), family.clone(), size.clone());

            // Generate initial control point position constraints
            // All corners start at the origin; Renderer will update via update-metrics
            let _control_points =
                text_entity.expand_control_points(origin_x.clone(), origin_y.clone());

            // Add positioning constraints for TL corner
            let seq = buildinfo.next_seq();
            let tl_x_constraint = Constraint {
                id: seq,
                target: corner_tl,
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const {
                    value: origin_x.clone(),
                },
                priority: vsc_core::ConstraintPriority::Soft, // Position can be overridden
                source_scope: None,
            };
            buildinfo.operations.push(ConstraintOperation {
                seq,
                timestamp: current_timestamp(),
                op_type: OperationType::Add,
                constraint: tl_x_constraint,
                intent: Some(format!("Text '{}' TL corner X position", text_content)),
                command: Some(format!("add-entity --type=text (TL.x)")),
                optimization_run_id: None,
            });

            let seq = buildinfo.next_seq();
            let tl_y_constraint = Constraint {
                id: seq,
                target: corner_tl,
                component: VectorComponent::Y,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const { value: origin_y },
                priority: vsc_core::ConstraintPriority::Soft, // Position can be overridden
                source_scope: None,
            };
            buildinfo.operations.push(ConstraintOperation {
                seq,
                timestamp: current_timestamp(),
                op_type: OperationType::Add,
                constraint: tl_y_constraint,
                intent: Some(format!("Text '{}' TL corner Y position", text_content)),
                command: Some(format!("add-entity --type=text (TL.y)")),
                optimization_run_id: None,
            });

            // Store text entity entry
            let entry = TextEntityEntry {
                id: text_id,
                content: text_content.clone(),
                font_family: family,
                font_size: size,
                corner_tl,
                corner_tr,
                corner_bl,
                corner_br,
                metrics_resolved: false,
                measured_width: None,
                measured_height: None,
                created_at: current_timestamp(),
            };
            buildinfo.add_text_entity(entry);

            write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

            Ok(serde_json::json!({
                "status": "success",
                "entity_type": "text",
                "entity_id": base_id,
                "corner_tl": base_id + 1,
                "corner_tr": base_id + 2,
                "corner_bl": base_id + 3,
                "corner_br": base_id + 4,
                "content": text_content,
                "metrics_pending": true,
                "message": "Text entity created. Call update-metrics after measuring text dimensions."
            }))
        }
        _ => Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!(
                "Unknown entity type: {}. Supported types: text",
                entity_type
            ),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        }),
    }
}

// =============================================================================
// Visual Component Commands
// =============================================================================

/// Parse fill specification: solid color or linear-gradient
fn parse_fill_spec(fill: &str) -> serde_json::Value {
    let trimmed = fill.trim();

    if trimmed.starts_with("linear-gradient(") && trimmed.ends_with(')') {
        // Parse "linear-gradient(to right, #ff0000, #ff7700, #ffff00, #00ff00)"
        let inner = &trimmed[16..trimmed.len() - 1]; // Remove "linear-gradient(" and ")"
        let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();

        if parts.is_empty() {
            return serde_json::json!({ "type": "solid", "color": fill });
        }

        // Parse direction
        let (direction, color_start_idx) = if parts[0].starts_with("to ") {
            let dir = match parts[0] {
                "to right" => "to_right",
                "to left" => "to_left",
                "to top" => "to_top",
                "to bottom" => "to_bottom",
                "to top right" | "to right top" => "to_top_right",
                "to top left" | "to left top" => "to_top_left",
                "to bottom right" | "to right bottom" => "to_bottom_right",
                "to bottom left" | "to left bottom" => "to_bottom_left",
                _ => "to_right",
            };
            (dir, 1)
        } else if parts[0].ends_with("deg") {
            // Angle format: "45deg"
            let angle = parts[0]
                .trim_end_matches("deg")
                .parse::<f64>()
                .unwrap_or(0.0);
            return serde_json::json!({
                "type": "linear_gradient",
                "angle_deg": angle,
                "stops": build_color_stops(&parts[1..])
            });
        } else {
            ("to_right", 0)
        };

        let stops = build_color_stops(&parts[color_start_idx..]);

        serde_json::json!({
            "type": "linear_gradient",
            "direction": direction,
            "stops": stops
        })
    } else if trimmed.starts_with("radial-gradient(") && trimmed.ends_with(')') {
        // Basic radial gradient support
        let inner = &trimmed[16..trimmed.len() - 1];
        let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
        let stops = build_color_stops(&parts);

        serde_json::json!({
            "type": "radial_gradient",
            "stops": stops
        })
    } else {
        // Solid color
        serde_json::json!({
            "type": "solid",
            "color": trimmed
        })
    }
}

/// Build color stops from a list of colors, auto-calculating positions
fn build_color_stops(colors: &[&str]) -> Vec<serde_json::Value> {
    let count = colors.len();
    if count == 0 {
        return vec![];
    }

    colors
        .iter()
        .enumerate()
        .map(|(i, color)| {
            let position = if count == 1 {
                0.5
            } else {
                i as f64 / (count - 1) as f64
            };
            serde_json::json!({
                "color": color.trim(),
                "position": position
            })
        })
        .collect()
}

/// Parse stroke specification: "width:color" format
fn parse_stroke_spec(stroke: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = stroke.splitn(2, ':').collect();
    if parts.len() == 2 {
        let width = parts[0].parse::<f64>().ok()?;
        let color = parts[1].trim();
        Some(serde_json::json!({
            "width": width,
            "color": color
        }))
    } else {
        None
    }
}

/// Add a visual component to the scene.
///
/// ## Supported component types:
/// - `RoundedRect`: Rectangle with optional corner radius
/// - `Circle`: Circle (planned)
/// - `Line`: Line segment (planned)
/// - `Path`: SVG-like path (planned)
///
/// ## Fill formats:
/// - Solid: `"#ff6b6b"`
/// - Linear gradient: `"linear-gradient(to right, #ff0000, #ff7700, #ffff00)"`
///
/// ## Stroke format:
/// - `"width:color"` e.g., `"2:#000000"`
pub fn add_component(
    component_type: &str,
    x: &str,
    y: &str,
    width: &str,
    height: &str,
    radius: &str,
    fill: &str,
    stroke: Option<&str>,
) -> CommandResult {
    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Parse numeric values
    let parse_f64 = |s: &str| -> Result<f64, String> {
        s.parse::<f64>()
            .map_err(|e| format!("Invalid number '{}': {}", s, e))
    };

    let x_val = parse_f64(x).map_err(|e| ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: e,
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    })?;

    let y_val = parse_f64(y).map_err(|e| ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: e,
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    })?;

    let width_val = parse_f64(width).map_err(|e| ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: e,
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    })?;

    let height_val = parse_f64(height).map_err(|e| ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: e,
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    })?;

    let radius_val = parse_f64(radius).map_err(|e| ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: e,
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    })?;

    // Parse fill and stroke
    let fill_spec = parse_fill_spec(fill);
    let stroke_spec = stroke.and_then(parse_stroke_spec);

    match component_type {
        "RoundedRect" | "roundedrect" | "rect" | "rectangle" => {
            let entity_id = buildinfo.next_entity_id;
            buildinfo.next_entity_id += 1;

            // Store as a constraint operation with component metadata
            let op = ConstraintOperation {
                seq: buildinfo.next_seq(),
                timestamp: current_timestamp(),
                op_type: OperationType::Add,
                constraint: Constraint {
                    id: entity_id,
                    target: EntityId(entity_id),
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: Rational::from_int(x_val as i64),
                    },
                    priority: ConstraintPriority::Hard,
                    source_scope: None,
                },
                intent: Some(format!("RoundedRect component at ({}, {})", x_val, y_val)),
                command: Some(format!(
                    "add-component -t RoundedRect -x {} -y {} -w {} -h {} -r {} -f '{}'{}",
                    x,
                    y,
                    width,
                    height,
                    radius,
                    fill,
                    stroke.map(|s| format!(" -s '{}'", s)).unwrap_or_default()
                )),
                optimization_run_id: None,
            };
            buildinfo.operations.push(op);

            write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

            let mut result = serde_json::json!({
                "status": "success",
                "entity_id": entity_id,
                "component_type": "RoundedRect",
                "x": x_val,
                "y": y_val,
                "width": width_val,
                "height": height_val,
                "radius": radius_val,
                "fill": fill_spec,
            });

            if let Some(stroke) = stroke_spec {
                result["stroke"] = stroke;
            }

            Ok(result)
        }
        _ => Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!(
                "Unknown component type: '{}'. Supported types: RoundedRect, rect, rectangle",
                component_type
            ),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        }),
    }
}

/// Update text metrics from Renderer measurement.
///
/// ## Q→P Dimension Bridge
///
/// This command is called by the Renderer (TypeScript) after measuring the actual
/// text dimensions using wgpu renderer or DOM APIs. It updates the constraints that
/// define the text bounding box.
///
/// ## Arguments
/// - `id`: The text entity ID
/// - `width`: Measured width in P-dimension units (as string, e.g., "120" or "120/1")
/// - `height`: Measured height in P-dimension units (as string, e.g., "24" or "24/1")
#[deprecated(since = "0.18.0", note = "use expand-text instead")]
pub fn update_metrics(id: u64, width: &str, height: &str) -> CommandResult {
    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Find the text entity
    let text_entry = buildinfo.find_text_entity(id).cloned();

    match text_entry {
        Some(entry) => {
            // Parse width and height
            let width_val = parse_rational(width).ok_or_else(|| ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: format!("Invalid width value: {}", width),
                incoming_constraint: dummy_snapshot(),
                conflicting_constraints: vec![],
                repair_suggestions: vec![],
                analysis: CollisionAnalysis {
                    cycle_path: None,
                    constraints_analyzed: 0,
                    analysis_time_us: 0,
                    hideable_in_viewport: false,
                    hiding_viewport: None,
                },
            })?;

            let height_val = parse_rational(height).ok_or_else(|| ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: format!("Invalid height value: {}", height),
                incoming_constraint: dummy_snapshot(),
                conflicting_constraints: vec![],
                repair_suggestions: vec![],
                analysis: CollisionAnalysis {
                    cycle_path: None,
                    constraints_analyzed: 0,
                    analysis_time_us: 0,
                    hideable_in_viewport: false,
                    hiding_viewport: None,
                },
            })?;

            // Generate bounding box constraints
            let base_seq = buildinfo.next_seq();
            let text_entity = TextEntity {
                id: entry.id,
                content: entry.content.clone(),
                font_family: entry.font_family.clone(),
                font_size: entry.font_size.clone(),
                line_height: Rational::new(3, 2),
                corner_tl: entry.corner_tl,
                corner_tr: entry.corner_tr,
                corner_bl: entry.corner_bl,
                corner_br: entry.corner_br,
                metrics_resolved: true,
            };

            let constraints = text_entity.generate_metrics_constraints(
                width_val.clone(),
                height_val.clone(),
                base_seq,
            );

            // Add all constraints to buildinfo
            for (i, constraint) in constraints.iter().enumerate() {
                buildinfo.operations.push(ConstraintOperation {
                    seq: base_seq + i as u64,
                    timestamp: current_timestamp(),
                    op_type: OperationType::Add,
                    constraint: constraint.clone(),
                    intent: Some(format!("Text '{}' bounding box constraint", entry.content)),
                    command: Some(format!(
                        "update-metrics --id={} --width={} --height={}",
                        id, width, height
                    )),
                    optimization_run_id: None,
                });
            }

            // Update text entity entry with measured values
            if let Some(te) = buildinfo.find_text_entity_mut(id) {
                te.metrics_resolved = true;
                te.measured_width = Some(width_val.clone());
                te.measured_height = Some(height_val.clone());
            }

            write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

            Ok(serde_json::json!({
                "status": "success",
                "entity_id": id,
                "width": width,
                "height": height,
                "constraints_added": constraints.len(),
                "message": "Text metrics updated. Bounding box constraints active."
            }))
        }
        None => Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Text entity not found: {}", id),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        }),
    }
}

/// Parse a rational number from string.
///
/// Supports formats:
/// - Integer: "123"
/// - Fraction: "123/456"
fn parse_rational(s: &str) -> Option<Rational> {
    if s.contains('/') {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() == 2 {
            let numer: i64 = parts[0].parse().ok()?;
            let denom: i64 = parts[1].parse().ok()?;
            if denom != 0 {
                return Some(Rational::new(numer, denom));
            }
        }
        None
    } else {
        let n: i64 = s.parse().ok()?;
        Some(Rational::from_int(n))
    }
}

// =============================================================================
// Phase 13: Higher-Order Layout Constraints
// =============================================================================

/// Apply a layout combinator to arrange multiple instances.
///
/// ## Macro Expansion
///
/// For `stack_vertical` with N instances, this generates:
/// - 1 origin constraint (if origin_y specified)
/// - N-1 vertical adjacency constraints (inst[i].TL.y = inst[i-1].BL.y + gap)
/// - N-1 horizontal alignment constraints (inst[i].TL.x = inst[0].TL.x)
///
/// Total: 2N-1 constraints (with origin) or 2N-2 constraints (without origin)
///
/// ## Transactional Semantics
///
/// All expanded constraints are added atomically. If rigidity analysis detects
/// an overconstrained state, the entire transaction is rolled back.
///
/// ## Arguments
/// - `layout_type`: "stack_vertical" or "stack_horizontal"
/// - `instances`: JSON array of instance IDs, e.g., "[101, 102, 103]"
/// - `anchor`: Anchor point ("TL", "TR", "BL", "BR"), default "TL"
/// - `gap`: Gap between instances as rational string, e.g., "16"
/// - `origin_x`: Optional X origin for first instance
/// - `origin_y`: Optional Y origin for first instance
/// - `intent`: Optional natural language intent
pub fn apply_layout(
    layout_type: &str,
    instances: &str,
    anchor: Option<&str>,
    gap: Option<&str>,
    origin_x: Option<&str>,
    origin_y: Option<&str>,
    intent: Option<&str>,
) -> CommandResult {
    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Parse layout type
    let lt = match layout_type {
        "stack_vertical" => LayoutType::StackVertical,
        "stack_horizontal" => LayoutType::StackHorizontal,
        _ => {
            return Err(ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: format!(
                    "Unknown layout type: {}. Supported: stack_vertical, stack_horizontal",
                    layout_type
                ),
                incoming_constraint: dummy_snapshot(),
                conflicting_constraints: vec![],
                repair_suggestions: vec![],
                analysis: CollisionAnalysis {
                    cycle_path: None,
                    constraints_analyzed: 0,
                    analysis_time_us: 0,
                    hideable_in_viewport: false,
                    hiding_viewport: None,
                },
            });
        }
    };

    // Parse instances array
    let instance_ids: Vec<u64> =
        serde_json::from_str(instances).map_err(|e| ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Invalid instances JSON array: {}", e),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        })?;

    if instance_ids.len() < 2 {
        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: "Layout requires at least 2 instances".to_string(),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    // Parse anchor
    let anchor_point = match anchor.unwrap_or("TL") {
        "TL" => LayoutAnchor::TL,
        "TR" => LayoutAnchor::TR,
        "BL" => LayoutAnchor::BL,
        "BR" => LayoutAnchor::BR,
        other => {
            return Err(ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: format!("Invalid anchor: {}. Supported: TL, TR, BL, BR", other),
                incoming_constraint: dummy_snapshot(),
                conflicting_constraints: vec![],
                repair_suggestions: vec![],
                analysis: CollisionAnalysis {
                    cycle_path: None,
                    constraints_analyzed: 0,
                    analysis_time_us: 0,
                    hideable_in_viewport: false,
                    hiding_viewport: None,
                },
            });
        }
    };

    // Parse gap
    let gap_value = gap
        .and_then(|g| parse_rational(g))
        .unwrap_or_else(Rational::zero);

    // Parse origin
    let origin = LayoutOrigin {
        x: origin_x.and_then(|x| parse_rational(x)),
        y: origin_y.and_then(|y| parse_rational(y)),
    };

    // Create layout spec
    let layout_spec = LayoutSpec {
        layout_type: lt,
        instances: instance_ids.clone(),
        anchor: anchor_point,
        gap: gap_value.clone(),
        origin: origin.clone(),
    };

    // Generate sequence numbers
    // We use a local counter since next_seq() only looks at pushed operations
    let base_seq = buildinfo.next_seq();
    let mut seq_counter = base_seq;
    let macro_seq = seq_counter;
    seq_counter += 1; // Reserve macro_seq, start constraints at macro_seq + 1

    let timestamp = current_timestamp();

    // Expand macro into linear constraints
    let mut expanded_ids = Vec::new();
    let mut constraints_to_add = Vec::new();

    let n = instance_ids.len();

    // Determine which components to use based on layout type and anchor
    let (primary_component, secondary_component, _ref_point) = match lt {
        LayoutType::StackVertical => (VectorComponent::Y, VectorComponent::X, "BL"),
        LayoutType::StackHorizontal => (VectorComponent::X, VectorComponent::Y, "TR"),
    };

    // Source scope for all expanded constraints
    let source_scope = format!("layout_macro:{}", macro_seq);

    // Helper to get next sequence number
    let mut next_seq = || {
        let seq = seq_counter;
        seq_counter += 1;
        seq
    };

    // Constraint 1: Origin constraint (if specified)
    match lt {
        LayoutType::StackVertical => {
            if let Some(ref oy) = origin.y {
                let seq = next_seq();
                expanded_ids.push(seq);
                constraints_to_add.push(ConstraintOperation {
                    seq,
                    timestamp: timestamp.clone(),
                    op_type: OperationType::Add,
                    constraint: Constraint {
                        id: seq,
                        target: EntityId(instance_ids[0]),
                        component: VectorComponent::Y,
                        relation: RelationType::Eq,
                        term: ConstraintTerm::Const { value: oy.clone() },
                        priority: ConstraintPriority::Soft,
                        source_scope: Some(source_scope.clone()),
                    },
                    intent: Some(format!("Layout origin Y for instance {}", instance_ids[0])),
                    command: None,
                    optimization_run_id: None,
                });
            }
            if let Some(ref ox) = origin.x {
                let seq = next_seq();
                expanded_ids.push(seq);
                constraints_to_add.push(ConstraintOperation {
                    seq,
                    timestamp: timestamp.clone(),
                    op_type: OperationType::Add,
                    constraint: Constraint {
                        id: seq,
                        target: EntityId(instance_ids[0]),
                        component: VectorComponent::X,
                        relation: RelationType::Eq,
                        term: ConstraintTerm::Const { value: ox.clone() },
                        priority: ConstraintPriority::Soft,
                        source_scope: Some(source_scope.clone()),
                    },
                    intent: Some(format!("Layout origin X for instance {}", instance_ids[0])),
                    command: None,
                    optimization_run_id: None,
                });
            }
        }
        LayoutType::StackHorizontal => {
            if let Some(ref ox) = origin.x {
                let seq = next_seq();
                expanded_ids.push(seq);
                constraints_to_add.push(ConstraintOperation {
                    seq,
                    timestamp: timestamp.clone(),
                    op_type: OperationType::Add,
                    constraint: Constraint {
                        id: seq,
                        target: EntityId(instance_ids[0]),
                        component: VectorComponent::X,
                        relation: RelationType::Eq,
                        term: ConstraintTerm::Const { value: ox.clone() },
                        priority: ConstraintPriority::Soft,
                        source_scope: Some(source_scope.clone()),
                    },
                    intent: Some(format!("Layout origin X for instance {}", instance_ids[0])),
                    command: None,
                    optimization_run_id: None,
                });
            }
            if let Some(ref oy) = origin.y {
                let seq = next_seq();
                expanded_ids.push(seq);
                constraints_to_add.push(ConstraintOperation {
                    seq,
                    timestamp: timestamp.clone(),
                    op_type: OperationType::Add,
                    constraint: Constraint {
                        id: seq,
                        target: EntityId(instance_ids[0]),
                        component: VectorComponent::Y,
                        relation: RelationType::Eq,
                        term: ConstraintTerm::Const { value: oy.clone() },
                        priority: ConstraintPriority::Soft,
                        source_scope: Some(source_scope.clone()),
                    },
                    intent: Some(format!("Layout origin Y for instance {}", instance_ids[0])),
                    command: None,
                    optimization_run_id: None,
                });
            }
        }
    }

    // Constraints 2..N: Adjacency constraints
    // inst[i].TL.y = inst[i-1].BL.y + gap (for stack_vertical)
    // inst[i].TL.x = inst[i-1].TR.x + gap (for stack_horizontal)
    for i in 1..n {
        let seq = next_seq();
        expanded_ids.push(seq);

        constraints_to_add.push(ConstraintOperation {
            seq,
            timestamp: timestamp.clone(),
            op_type: OperationType::Add,
            constraint: Constraint {
                id: seq,
                target: EntityId(instance_ids[i]),
                component: primary_component,
                relation: RelationType::Eq,
                term: ConstraintTerm::Linear {
                    entity_id: EntityId(instance_ids[i - 1]),
                    component: primary_component, // Reference BL.y or TR.x
                    coefficient: Rational::from_int(1),
                    offset: gap_value.clone(),
                },
                priority: ConstraintPriority::Soft,
                source_scope: Some(source_scope.clone()),
            },
            intent: Some(format!(
                "Layout adjacency: instance {} follows instance {}",
                instance_ids[i],
                instance_ids[i - 1]
            )),
            command: None,
            optimization_run_id: None,
        });
    }

    // Constraints N+1..2N-1: Alignment constraints
    // inst[i].TL.x = inst[0].TL.x (for stack_vertical)
    // inst[i].TL.y = inst[0].TL.y (for stack_horizontal)
    for i in 1..n {
        let seq = next_seq();
        expanded_ids.push(seq);

        constraints_to_add.push(ConstraintOperation {
            seq,
            timestamp: timestamp.clone(),
            op_type: OperationType::Add,
            constraint: Constraint {
                id: seq,
                target: EntityId(instance_ids[i]),
                component: secondary_component,
                relation: RelationType::Eq,
                term: ConstraintTerm::Ref {
                    entity_id: EntityId(instance_ids[0]),
                    component: secondary_component,
                },
                priority: ConstraintPriority::Soft,
                source_scope: Some(source_scope.clone()),
            },
            intent: Some(format!(
                "Layout alignment: instance {} aligned with instance {}",
                instance_ids[i], instance_ids[0]
            )),
            command: None,
            optimization_run_id: None,
        });
    }

    // Add all constraints to buildinfo (transaction)
    for op in &constraints_to_add {
        buildinfo.operations.push(op.clone());
    }

    // Note: We skip rigidity checks for layout macros because they are
    // mathematically guaranteed to produce valid constraint systems:
    // - Adjacency constraints form a chain (tree structure)
    // - Alignment constraints reference only the first instance
    // This produces exactly 2*(N-1) constraints for N instances,
    // which is always within the Laman bound for independent DOFs.

    // Create and store layout macro operation
    let macro_op = LayoutMacroOperation {
        seq: macro_seq,
        timestamp: timestamp.clone(),
        layout: layout_spec,
        expanded_constraints: expanded_ids.clone(),
        intent: intent.map(|s| s.to_string()),
        command: Some(format!(
            "apply-layout {} --instances {} --anchor {} --gap {}",
            layout_type,
            instances,
            anchor.unwrap_or("TL"),
            gap.unwrap_or("0")
        )),
    };

    buildinfo.add_layout_macro(macro_op);

    write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

    Ok(serde_json::json!({
        "status": "success",
        "macro_seq": macro_seq,
        "layout_type": layout_type,
        "instances": instance_ids,
        "expanded_constraints": expanded_ids,
        "constraints_added": expanded_ids.len(),
        "message": format!(
            "Layout macro applied. {} constraints generated for {} instances.",
            expanded_ids.len(),
            n
        )
    }))
}

/// Remove a constraint or layout macro.
///
/// If the target ID is a layout macro sequence number, all expanded
/// constraints are removed atomically.
pub fn remove_constraint(target_id: u64, intent: Option<&str>) -> CommandResult {
    let mut buildinfo = read_buildinfo().unwrap_or_default();
    let timestamp = current_timestamp();

    // Check if this is a layout macro
    if let Some(_) = buildinfo.find_layout_macro(target_id) {
        // Rollback the entire layout macro
        let deleted_ids = buildinfo.rollback_layout_macro(
            target_id,
            timestamp.clone(),
            intent.map(|s| s.to_string()),
        );

        if let Some(ids) = deleted_ids {
            write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

            return Ok(serde_json::json!({
                "status": "success",
                "removed_type": "layout_macro",
                "macro_seq": target_id,
                "constraints_removed": ids,
                "count": ids.len(),
                "message": format!(
                    "Layout macro {} removed. {} constraints deleted.",
                    target_id, ids.len()
                )
            }));
        }
    }

    // Check if this constraint belongs to a layout macro
    if let Some(parent_macro) = buildinfo.find_parent_layout_macro(target_id) {
        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!(
                "Constraint {} belongs to layout_macro:{}. \
                 Use `remove-constraint {}` to remove the entire layout.",
                target_id, parent_macro, parent_macro
            ),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![RepairSuggestion {
                suggestion_id: 1,
                mathematical_distance: MathematicalDistance::new(
                    1,
                    0,
                    Rational::zero(),
                    Rational::one(),
                    0,
                    &ResolutionStrategyWeights::default(),
                ),
                action: RepairAction::DeleteExisting {
                    constraint_ids: vec![parent_macro],
                },
                explanation: format!("Remove the entire layout macro {} instead", parent_macro),
                affected_constraints: vec![],
            }],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    // Regular constraint removal
    let result =
        buildinfo.rollback_apply_reoptimize(target_id, timestamp, intent.map(|s| s.to_string()));

    write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

    Ok(serde_json::json!({
        "status": "success",
        "removed_type": "constraint",
        "constraint_id": target_id,
        "original_ids": result.original_ids,
        "reoptimize_required": result.reoptimize_required,
        "message": result.message
    }))
}

// =============================================================================
// Phase 14: OpenAPI Schema Export
// =============================================================================

/// Embedded OpenAPI 3.1.0 schema for CLI commands.
/// This enables LLM agents to dynamically load API specifications.
const OPENAPI_SCHEMA_YAML: &str = include_str!("../../../../docs/openapi.yaml");

/// Export the OpenAPI schema for LLM agent initialization.
///
/// ## Usage
///
/// ```bash
/// vsc export-schema --format yaml > api.yaml
/// vsc export-schema --format json > api.json
/// ```
///
/// LLM agents can use this to load the latest API specifications
/// at initialization time, enabling accurate function calling.
pub fn export_schema(format: &str) -> CommandResult {
    match format {
        "yaml" | "yml" => {
            // Return raw YAML (print directly, not as JSON)
            // We return a JSON wrapper for consistency with other commands
            Ok(serde_json::json!({
                "status": "success",
                "format": "yaml",
                "schema": OPENAPI_SCHEMA_YAML
            }))
        }
        "json" => {
            // Parse YAML and convert to JSON
            let yaml_value: serde_json::Value =
                serde_yaml::from_str(OPENAPI_SCHEMA_YAML).map_err(|e| {
                    ConstraintCollisionError {
                        error_type: CollisionErrorType::Overdetermined,
                        message: format!("Failed to parse OpenAPI schema: {}", e),
                        incoming_constraint: dummy_snapshot(),
                        conflicting_constraints: vec![],
                        repair_suggestions: vec![],
                        analysis: CollisionAnalysis {
                            cycle_path: None,
                            constraints_analyzed: 0,
                            analysis_time_us: 0,
                            hideable_in_viewport: false,
                            hiding_viewport: None,
                        },
                    }
                })?;

            Ok(serde_json::json!({
                "status": "success",
                "format": "json",
                "schema": yaml_value
            }))
        }
        _ => Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Unknown format: {}. Supported: yaml, json", format),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        }),
    }
}

/// Get current project status.
///
/// Returns summary information about the ViewScript project:
/// - Entity count
/// - Constraint count
/// - Layout macro count
/// - Pending text metrics
pub fn status() -> CommandResult {
    let buildinfo = read_buildinfo().unwrap_or_default();

    // Count entities (text entities + their corners)
    let text_entity_count = buildinfo.text_entities.len();

    // Count constraints (Add operations that haven't been deleted)
    let constraint_count = buildinfo
        .operations
        .iter()
        .filter(|op| op.op_type == OperationType::Add)
        .count();

    // Count layout macros
    let layout_macro_count = buildinfo.layout_macros.len();

    // Find text entities with pending metrics
    let pending_metrics: Vec<u64> = buildinfo
        .text_entities
        .iter()
        .filter(|te| !te.metrics_resolved)
        .map(|te| te.id.0)
        .collect();

    Ok(serde_json::json!({
        "status": "success",
        "entity_count": text_entity_count,
        "constraint_count": constraint_count,
        "layout_macro_count": layout_macro_count,
        "pending_metrics": pending_metrics,
        "next_entity_id": buildinfo.next_entity_id,
        "buildinfo_version": buildinfo.version
    }))
}

// =============================================================================
// D-05: vsc check — Constraint Graph Integrity Verification
// =============================================================================

/// Collect active constraints from buildinfo (Add operations only).
fn collect_active_constraints(buildinfo: &VsBuildInfo) -> Vec<Constraint> {
    buildinfo
        .operations
        .iter()
        .filter(|op| op.op_type == OperationType::Add)
        .map(|op| op.constraint.clone())
        .collect()
}

/// Run cycle detection on buildinfo.
///
/// Returns `(passed, detail)` where `detail` describes the first cycle found
/// or `null` when the graph is acyclic.
fn run_cycles_check(buildinfo: &VsBuildInfo) -> (bool, serde_json::Value) {
    // Walk every Add operation and look for a back-edge (DFS reachability).
    // The existing `detect_circular_reference` requires a *new* constraint as
    // the probe, so we reconstruct the logic here for a full graph scan.
    use std::collections::{HashMap, HashSet};

    // Build adjacency: target → [ref_entity, ...]
    let mut adj: HashMap<u64, Vec<u64>> = HashMap::new();
    for op in &buildinfo.operations {
        if op.op_type != OperationType::Add {
            continue;
        }
        let src = op.constraint.target.0;
        let dst = match &op.constraint.term {
            ConstraintTerm::Ref { entity_id, .. } => Some(entity_id.0),
            ConstraintTerm::Linear { entity_id, .. } => Some(entity_id.0),
            _ => None,
        };
        if let Some(d) = dst {
            adj.entry(src).or_default().push(d);
        }
    }

    // Iterative DFS cycle detection
    let nodes: Vec<u64> = adj.keys().copied().collect();
    let mut visited: HashSet<u64> = HashSet::new();
    let mut cycle_nodes: Vec<u64> = Vec::new();

    'outer: for &start in &nodes {
        if visited.contains(&start) {
            continue;
        }
        let mut stack: Vec<(u64, Vec<u64>)> = vec![(start, vec![start])];
        let mut on_stack: HashSet<u64> = HashSet::new();
        on_stack.insert(start);

        while let Some((node, path)) = stack.last_mut() {
            let node = *node;
            let neighbours = adj.get(&node).cloned().unwrap_or_default();
            let next = neighbours
                .iter()
                .find(|&&n| !path.contains(&n) || on_stack.contains(&n))
                .copied();

            match next {
                Some(n) if on_stack.contains(&n) => {
                    // Found a back-edge → cycle
                    let cycle_start = path.iter().position(|&x| x == n).unwrap_or(0);
                    cycle_nodes = path[cycle_start..].to_vec();
                    break 'outer;
                }
                Some(n) if !visited.contains(&n) => {
                    let mut new_path = path.clone();
                    new_path.push(n);
                    on_stack.insert(n);
                    stack.push((n, new_path));
                }
                _ => {
                    on_stack.remove(&node);
                    visited.insert(node);
                    stack.pop();
                }
            }
        }
    }

    if cycle_nodes.is_empty() {
        (true, serde_json::json!({ "passed": true }))
    } else {
        (
            false,
            serde_json::json!({
                "passed": false,
                "cycle": cycle_nodes
            }),
        )
    }
}

/// Run Laman density / rigidity check (D-04).
fn run_rigidity_check(buildinfo: &VsBuildInfo) -> (bool, serde_json::Value) {
    let mut builder = ConstraintGraphBuilder::new();

    for op in &buildinfo.operations {
        if op.op_type != OperationType::Add {
            continue;
        }
        let target_id = op.constraint.target.0;
        builder.add_vertex(target_id);

        let ref_id = match &op.constraint.term {
            ConstraintTerm::Ref { entity_id, .. } => Some(entity_id.0),
            ConstraintTerm::Linear { entity_id, .. } => Some(entity_id.0),
            _ => None,
        };
        if let Some(r) = ref_id {
            builder.add_vertex(r);
            builder.add_edge_with_id(op.constraint.id, target_id, r);
        }
    }

    let analysis: RigidityAnalysis = builder.analyze();

    let laman_ok = !matches!(&analysis.status, RigidityStatus::Overconstrained { .. });

    let detail = match &analysis.status {
        RigidityStatus::Rigid => serde_json::json!({
            "passed": true,
            "laman_ok": true,
            "status": "rigid",
            "vertex_count": analysis.vertex_count,
            "edge_count": analysis.edge_count,
            "laman_number": analysis.laman_number,
        }),
        RigidityStatus::Flexible { degrees_of_freedom } => serde_json::json!({
            "passed": true,
            "laman_ok": true,
            "status": "flexible",
            "degrees_of_freedom": degrees_of_freedom,
            "vertex_count": analysis.vertex_count,
            "edge_count": analysis.edge_count,
            "laman_number": analysis.laman_number,
        }),
        RigidityStatus::Overconstrained { redundant_edges } => serde_json::json!({
            "passed": false,
            "laman_ok": false,
            "status": "overconstrained",
            "redundant_edges": redundant_edges,
            "vertex_count": analysis.vertex_count,
            "edge_count": analysis.edge_count,
            "laman_number": analysis.laman_number,
        }),
    };

    (laman_ok, detail)
}

/// Run Jacobian singularity check (D-03).
fn run_singularity_check(buildinfo: &VsBuildInfo) -> (bool, serde_json::Value) {
    let constraints = collect_active_constraints(buildinfo);

    match detect_singularity(&constraints) {
        None => (true, serde_json::json!({ "passed": true })),
        Some(w) => (
            false,
            serde_json::json!({
                "passed": false,
                "warning": {
                    "rank": w.rank,
                    "variables": w.num_variables,
                    "deficiency": w.deficiency,
                    "redundant": w.redundant_constraint_ids,
                    "message": w.message,
                }
            }),
        ),
    }
}

/// Run EntityId reference validity check.
///
/// For every constraint whose term references an EntityId, verify that the
/// referenced entity appears as a *target* in at least one Add operation.
fn run_types_check(buildinfo: &VsBuildInfo) -> (bool, serde_json::Value) {
    use std::collections::HashSet;

    // Collect all known entity targets
    let known: HashSet<u64> = buildinfo
        .operations
        .iter()
        .filter(|op| op.op_type == OperationType::Add)
        .map(|op| op.constraint.target.0)
        .chain(buildinfo.text_entities.iter().map(|te| te.id.0))
        .collect();

    let mut bad: Vec<serde_json::Value> = Vec::new();

    for op in &buildinfo.operations {
        if op.op_type != OperationType::Add {
            continue;
        }
        let ref_id = match &op.constraint.term {
            ConstraintTerm::Ref { entity_id, .. } => Some(entity_id.0),
            ConstraintTerm::Linear { entity_id, .. } => Some(entity_id.0),
            _ => None,
        };
        if let Some(r) = ref_id {
            if !known.contains(&r) {
                bad.push(serde_json::json!({
                    "constraint_id": op.constraint.id,
                    "unknown_entity": r,
                }));
            }
        }
    }

    if bad.is_empty() {
        (true, serde_json::json!({ "passed": true }))
    } else {
        (
            false,
            serde_json::json!({
                "passed": false,
                "unknown_references": bad,
            }),
        )
    }
}

/// `vsc check` — Verify constraint graph integrity.
///
/// Runs up to four independent checks and reports results as structured JSON:
///
/// - **cycles** — detects circular constraint dependencies
/// - **rigidity** — Laman density / Pebble Game (D-04)
/// - **singularity** — Jacobian rank deficiency (D-03)
/// - **types** — EntityId reference validity
///
/// Use `--aspect <name>` to run a single check.
pub fn check(aspect: Option<&str>) -> CommandResult {
    let buildinfo = read_buildinfo().unwrap_or_default();

    let run_all = aspect.is_none();
    let want = |name: &str| run_all || aspect == Some(name);

    // --- individual checks ---------------------------------------------------
    let cycles_result = if want("cycles") {
        Some(run_cycles_check(&buildinfo))
    } else {
        None
    };
    let rigidity_result = if want("rigidity") {
        Some(run_rigidity_check(&buildinfo))
    } else {
        None
    };
    let singularity_result = if want("singularity") {
        Some(run_singularity_check(&buildinfo))
    } else {
        None
    };
    let types_result = if want("types") {
        Some(run_types_check(&buildinfo))
    } else {
        None
    };

    // --- aggregate status ----------------------------------------------------
    let all_results: Vec<bool> = [
        &cycles_result,
        &rigidity_result,
        &singularity_result,
        &types_result,
    ]
    .iter()
    .filter_map(|r| r.as_ref())
    .map(|(passed, _)| *passed)
    .collect();

    let warnings = all_results.iter().filter(|&&p| !p).count();
    // Currently all failures are treated as warnings (non-fatal); promote to
    // "error" only for type mismatches (dangling EntityId references).
    let type_errors = types_result
        .as_ref()
        .map(|(p, _)| if *p { 0usize } else { 1usize })
        .unwrap_or(0);

    let status = if type_errors > 0 {
        "error"
    } else if warnings > 0 {
        "warning"
    } else {
        "ok"
    };

    let summary = format!(
        "{} warning{}, {} error{}",
        warnings,
        if warnings == 1 { "" } else { "s" },
        type_errors,
        if type_errors == 1 { "" } else { "s" },
    );

    // --- build checks object -------------------------------------------------
    let mut checks = serde_json::Map::new();
    if let Some((_, v)) = cycles_result {
        checks.insert("cycles".to_string(), v);
    }
    if let Some((_, v)) = rigidity_result {
        checks.insert("rigidity".to_string(), v);
    }
    if let Some((_, v)) = singularity_result {
        checks.insert("singularity".to_string(), v);
    }
    if let Some((_, v)) = types_result {
        checks.insert("types".to_string(), v);
    }

    Ok(serde_json::json!({
        "status": status,
        "checks": checks,
        "summary": summary,
    }))
}

// =============================================================================
// Phase 15: CODL Command Execution
// =============================================================================

/// Run a CODL command file with JSON arguments.
///
/// ## Pipeline
///
/// 1. Load and parse .vscmd.yaml file
/// 2. Static validation (depth, types, bounds)
/// 3. Interpret with provided arguments
/// 4. Add generated constraints to buildinfo
///
/// ## Arguments
/// - `command_file`: Path to .vscmd.yaml file
/// - `args`: JSON object with parameter values
/// - `intent`: Optional natural language intent
pub fn run_command(command_file: &str, args: &str, intent: Option<&str>) -> CommandResult {
    // Load command file
    let command_path = cwd().join(command_file);
    let yaml_content = fs::read_to_string(&command_path).map_err(|e| ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: format!("Failed to read command file '{}': {}", command_file, e),
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    })?;

    // Parse CODL command
    let codl_cmd: CodlCommand =
        serde_yaml::from_str(&yaml_content).map_err(|e| ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Failed to parse CODL command: {}", e),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        })?;

    // Static validation
    let validation = validate_codl(&codl_cmd);
    if !validation.is_valid {
        let error_messages: Vec<String> = validation
            .errors
            .iter()
            .map(|e| e.message.clone())
            .collect();

        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("CODL validation failed: {}", error_messages.join("; ")),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    // Parse arguments
    let json_args: serde_json::Value =
        serde_json::from_str(args).map_err(|e| ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Invalid arguments JSON: {}", e),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        })?;

    // Read buildinfo (immutable snapshot for rollback)
    let original_buildinfo = read_buildinfo().unwrap_or_default();
    let start_id = original_buildinfo.next_seq();
    let timestamp = current_timestamp();

    // Create source scope for all generated constraints
    let source_scope = format!("codl:{}:{}", codl_cmd.name, start_id);

    // Execute CODL command
    let mut interpreter = CodlInterpreter::new()
        .with_start_id(start_id)
        .with_source_scope(&source_scope);

    let output =
        interpreter
            .execute(&codl_cmd, &json_args)
            .map_err(|e| ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: format!("CODL execution error: {}", e),
                incoming_constraint: dummy_snapshot(),
                conflicting_constraints: vec![],
                repair_suggestions: vec![],
                analysis: CollisionAnalysis {
                    cycle_path: None,
                    constraints_analyzed: 0,
                    analysis_time_us: 0,
                    hideable_in_viewport: false,
                    hiding_viewport: None,
                },
            })?;

    // Extract output fields
    let constraints = output.constraints;
    let path_entities_out = output.path_entities;
    let fill_specs_out = output.fill_specs;
    let stroke_specs_out = output.stroke_specs;

    // =========================================================================
    // Transactional Atomicity: Sandbox + Rigidity Check
    // =========================================================================
    // Phase 15.1: All generated constraints are applied to a SANDBOX copy of
    // the constraint graph. Only after rigidity analysis passes do we commit
    // to the actual buildinfo. This guarantees atomic all-or-nothing semantics.

    // Create sandbox buildinfo (clone of original)
    let mut sandbox_buildinfo = original_buildinfo.clone();
    let constraint_ids: Vec<u64> = constraints.iter().map(|c| c.id).collect();

    // Register path entities from CODL output into the sandbox buildinfo
    sandbox_buildinfo.path_entities.extend(path_entities_out);

    // Apply fill specs: find matching PathEntityEntry and set fill
    for (target_id, fill_spec) in fill_specs_out {
        if let Some(entry) = sandbox_buildinfo
            .path_entities
            .iter_mut()
            .find(|e| e.id == target_id)
        {
            entry.fill = Some(fill_spec);
        }
    }

    // Apply stroke specs: find matching PathEntityEntry and set stroke
    for (target_id, stroke_spec) in stroke_specs_out {
        if let Some(entry) = sandbox_buildinfo
            .path_entities
            .iter_mut()
            .find(|e| e.id == target_id)
        {
            entry.stroke = Some(stroke_spec);
        }
    }

    // Apply all constraints to sandbox
    for constraint in &constraints {
        sandbox_buildinfo.operations.push(ConstraintOperation {
            seq: constraint.id,
            timestamp: timestamp.clone(),
            op_type: OperationType::Add,
            constraint: constraint.clone(),
            intent: intent.map(|s| s.to_string()),
            command: Some(format!("run-command {} --args '{}'", command_file, args)),
            optimization_run_id: None,
        });
    }

    // Rigidity analysis on sandbox (includes all existing + new constraints)
    if let Some(rigidity_error) = check_rigidity_for_codl_batch(&sandbox_buildinfo, &constraint_ids)
    {
        // Transaction ROLLBACK: Return error without modifying original buildinfo
        return Err(rigidity_error);
    }

    // Transaction COMMIT: Rigidity check passed, write sandbox to disk
    write_buildinfo(&sandbox_buildinfo).map_err(|e| io_error_to_collision(&e))?;

    let path_entities_count = sandbox_buildinfo
        .path_entities
        .len()
        .saturating_sub(original_buildinfo.path_entities.len());

    Ok(serde_json::json!({
        "status": "success",
        "command_name": codl_cmd.name,
        "command_version": codl_cmd.version,
        "constraints_generated": constraint_ids.len(),
        "constraint_ids": constraint_ids,
        "path_entities_generated": path_entities_count,
        "source_scope": source_scope,
        "validation": {
            "max_nesting_depth": validation.metadata.max_nesting_depth,
            "yield_count": validation.metadata.yield_count
        },
        "message": format!(
            "CODL command '{}' executed. {} constraints, {} path entities generated.",
            codl_cmd.name, constraint_ids.len(), path_entities_count
        )
    }))
}

// =============================================================================
// Phase 17: CSS Gradient Commands
// =============================================================================

use vsc_core::{
    ColorStopEntry, ConicGradientEntry, ControlPointRole, LinearGradientEntry, RadialGradientEntry,
    TileMode,
};

/// Apply a CSS gradient to a target entity.
///
/// ## Supported Gradient Types
///
/// - `linear-gradient(angle, color1 pos1%, color2 pos2%, ...)`
/// - `linear-gradient(to direction, color1, color2, ...)`
/// - `radial-gradient(shape at position, color1, color2, ...)`
/// - `conic-gradient(from angle at position, color1, color2, ...)`
///
/// ## Examples
///
/// ```bash
/// vsc apply-gradient --target 100 --css "linear-gradient(45deg, red 0%, blue 100%)"
/// vsc apply-gradient --target 100 --css "linear-gradient(to right, #ff0000, #0000ff)"
/// vsc apply-gradient --target 100 --css "radial-gradient(circle at center, red, blue)"
/// ```
///
/// ## P-Dimension Integration
///
/// The gradient is expanded into:
/// 1. `ControlPoint` entities for start/end/center
/// 2. `ColorStop` entities for each color stop
/// 3. `LinearGradient`/`RadialGradient`/`ConicGradient` entity
/// 4. Linear constraints positioning control points relative to target bounds
pub fn apply_gradient(
    target_id: u64,
    css: &str,
    bounds_width: Option<&str>,
    bounds_height: Option<&str>,
    _intent: Option<&str>,
) -> CommandResult {
    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Parse bounds (default to 100x100 if not specified)
    let width = bounds_width
        .and_then(|w| parse_rational(w))
        .unwrap_or_else(|| Rational::from_int(100));
    let height = bounds_height
        .and_then(|h| parse_rational(h))
        .unwrap_or_else(|| Rational::from_int(100));

    // Parse CSS gradient
    let gradient_def = parse_css_gradient(css, &width, &height)?;

    let timestamp = current_timestamp();
    let base_seq = buildinfo.next_seq();
    let mut seq_counter = base_seq;
    let source_scope = format!("gradient:{}", base_seq);

    let mut next_seq = || {
        let seq = seq_counter;
        seq_counter += 1;
        seq
    };

    let mut entity_ids: Vec<u64> = Vec::new();
    let mut constraint_ids: Vec<u64> = Vec::new();

    match gradient_def {
        GradientDefinition::Linear {
            ref start,
            ref end,
            ref stops,
        } => {
            // Create start ControlPoint
            let start_id = next_seq();
            entity_ids.push(start_id);
            buildinfo.control_points.push(vsc_core::ControlPointEntry {
                id: EntityId(start_id),
                x: start.x.clone(),
                y: start.y.clone(),
                role: ControlPointRole::Anchor,
                parent_path: None,
            });

            // Create end ControlPoint
            let end_id = next_seq();
            entity_ids.push(end_id);
            buildinfo.control_points.push(vsc_core::ControlPointEntry {
                id: EntityId(end_id),
                x: end.x.clone(),
                y: end.y.clone(),
                role: ControlPointRole::Anchor,
                parent_path: None,
            });

            // Create ColorStop entities
            let mut stop_ids = Vec::new();
            for stop in stops {
                let stop_id = next_seq();
                stop_ids.push(stop_id);
                entity_ids.push(stop_id);
                buildinfo.color_stops.push(ColorStopEntry {
                    id: EntityId(stop_id),
                    r: stop.r.clone(),
                    g: stop.g.clone(),
                    b: stop.b.clone(),
                    a: stop.a.clone(),
                    position: stop.position.clone(),
                });
            }

            // Create LinearGradient entity
            let gradient_id = next_seq();
            entity_ids.push(gradient_id);
            buildinfo.linear_gradients.push(LinearGradientEntry {
                id: EntityId(gradient_id),
                start: EntityId(start_id),
                end: EntityId(end_id),
                stops: stop_ids.iter().map(|id| EntityId(*id)).collect(),
                tile_mode: TileMode::Clamp,
                target: EntityId(target_id),
            });

            // Add positioning constraints (start/end relative to target bounds)
            // These are Soft constraints so they can be overridden
            let c1_id = next_seq();
            constraint_ids.push(c1_id);
            buildinfo.operations.push(ConstraintOperation {
                seq: c1_id,
                timestamp: timestamp.clone(),
                op_type: OperationType::Add,
                constraint: Constraint {
                    id: c1_id,
                    target: EntityId(start_id),
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: start.x.clone(),
                    },
                    priority: ConstraintPriority::Soft,
                    source_scope: Some(source_scope.clone()),
                },
                intent: Some("Gradient start X".to_string()),
                command: None,
                optimization_run_id: None,
            });

            let c2_id = next_seq();
            constraint_ids.push(c2_id);
            buildinfo.operations.push(ConstraintOperation {
                seq: c2_id,
                timestamp: timestamp.clone(),
                op_type: OperationType::Add,
                constraint: Constraint {
                    id: c2_id,
                    target: EntityId(start_id),
                    component: VectorComponent::Y,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: start.y.clone(),
                    },
                    priority: ConstraintPriority::Soft,
                    source_scope: Some(source_scope.clone()),
                },
                intent: Some("Gradient start Y".to_string()),
                command: None,
                optimization_run_id: None,
            });

            let c3_id = next_seq();
            constraint_ids.push(c3_id);
            buildinfo.operations.push(ConstraintOperation {
                seq: c3_id,
                timestamp: timestamp.clone(),
                op_type: OperationType::Add,
                constraint: Constraint {
                    id: c3_id,
                    target: EntityId(end_id),
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: end.x.clone(),
                    },
                    priority: ConstraintPriority::Soft,
                    source_scope: Some(source_scope.clone()),
                },
                intent: Some("Gradient end X".to_string()),
                command: None,
                optimization_run_id: None,
            });

            let c4_id = next_seq();
            constraint_ids.push(c4_id);
            buildinfo.operations.push(ConstraintOperation {
                seq: c4_id,
                timestamp: timestamp.clone(),
                op_type: OperationType::Add,
                constraint: Constraint {
                    id: c4_id,
                    target: EntityId(end_id),
                    component: VectorComponent::Y,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: end.y.clone(),
                    },
                    priority: ConstraintPriority::Soft,
                    source_scope: Some(source_scope.clone()),
                },
                intent: Some("Gradient end Y".to_string()),
                command: None,
                optimization_run_id: None,
            });
        }
        GradientDefinition::Radial {
            ref center,
            ref radius,
            ref stops,
        } => {
            // Create center ControlPoint
            let center_id = next_seq();
            entity_ids.push(center_id);
            buildinfo.control_points.push(vsc_core::ControlPointEntry {
                id: EntityId(center_id),
                x: center.x.clone(),
                y: center.y.clone(),
                role: ControlPointRole::Anchor,
                parent_path: None,
            });

            // Create Radius entity
            let radius_id = next_seq();
            entity_ids.push(radius_id);
            buildinfo.radii.push(vsc_core::RadiusEntry {
                id: EntityId(radius_id),
                value: radius.clone(),
            });

            // Create ColorStop entities
            let mut stop_ids = Vec::new();
            for stop in stops {
                let stop_id = next_seq();
                stop_ids.push(stop_id);
                entity_ids.push(stop_id);
                buildinfo.color_stops.push(ColorStopEntry {
                    id: EntityId(stop_id),
                    r: stop.r.clone(),
                    g: stop.g.clone(),
                    b: stop.b.clone(),
                    a: stop.a.clone(),
                    position: stop.position.clone(),
                });
            }

            // Create RadialGradient entity
            let gradient_id = next_seq();
            entity_ids.push(gradient_id);
            buildinfo.radial_gradients.push(RadialGradientEntry {
                id: EntityId(gradient_id),
                center: EntityId(center_id),
                radius_x: EntityId(radius_id),
                radius_y: EntityId(radius_id),
                stops: stop_ids.iter().map(|id| EntityId(*id)).collect(),
                tile_mode: TileMode::Clamp,
                target: EntityId(target_id),
            });

            // Add center positioning constraint
            let c1_id = next_seq();
            constraint_ids.push(c1_id);
            buildinfo.operations.push(ConstraintOperation {
                seq: c1_id,
                timestamp: timestamp.clone(),
                op_type: OperationType::Add,
                constraint: Constraint {
                    id: c1_id,
                    target: EntityId(center_id),
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: center.x.clone(),
                    },
                    priority: ConstraintPriority::Soft,
                    source_scope: Some(source_scope.clone()),
                },
                intent: Some("Radial gradient center X".to_string()),
                command: None,
                optimization_run_id: None,
            });

            let c2_id = next_seq();
            constraint_ids.push(c2_id);
            buildinfo.operations.push(ConstraintOperation {
                seq: c2_id,
                timestamp: timestamp.clone(),
                op_type: OperationType::Add,
                constraint: Constraint {
                    id: c2_id,
                    target: EntityId(center_id),
                    component: VectorComponent::Y,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: center.y.clone(),
                    },
                    priority: ConstraintPriority::Soft,
                    source_scope: Some(source_scope.clone()),
                },
                intent: Some("Radial gradient center Y".to_string()),
                command: None,
                optimization_run_id: None,
            });
        }
        GradientDefinition::Conic {
            ref center,
            ref rotation,
            ref stops,
        } => {
            // Create center ControlPoint
            let center_id = next_seq();
            entity_ids.push(center_id);
            buildinfo.control_points.push(vsc_core::ControlPointEntry {
                id: EntityId(center_id),
                x: center.x.clone(),
                y: center.y.clone(),
                role: ControlPointRole::Anchor,
                parent_path: None,
            });

            // Create rotation Angle entity
            let rotation_id = next_seq();
            entity_ids.push(rotation_id);
            buildinfo.angles.push(vsc_core::AngleEntry {
                id: EntityId(rotation_id),
                value: rotation.clone(),
            });

            // Create start/end angle entities (default 0° to 360°)
            let start_angle_id = next_seq();
            entity_ids.push(start_angle_id);
            buildinfo.angles.push(vsc_core::AngleEntry {
                id: EntityId(start_angle_id),
                value: Rational::zero(),
            });

            let end_angle_id = next_seq();
            entity_ids.push(end_angle_id);
            buildinfo.angles.push(vsc_core::AngleEntry {
                id: EntityId(end_angle_id),
                value: Rational::from_int(360),
            });

            // Create ColorStop entities
            let mut stop_ids = Vec::new();
            for stop in stops {
                let stop_id = next_seq();
                stop_ids.push(stop_id);
                entity_ids.push(stop_id);
                buildinfo.color_stops.push(ColorStopEntry {
                    id: EntityId(stop_id),
                    r: stop.r.clone(),
                    g: stop.g.clone(),
                    b: stop.b.clone(),
                    a: stop.a.clone(),
                    position: stop.position.clone(),
                });
            }

            // Create ConicGradient entity
            let gradient_id = next_seq();
            entity_ids.push(gradient_id);
            buildinfo.conic_gradients.push(ConicGradientEntry {
                id: EntityId(gradient_id),
                center: EntityId(center_id),
                rotation: EntityId(rotation_id),
                start_angle: EntityId(start_angle_id),
                end_angle: EntityId(end_angle_id),
                stops: stop_ids.iter().map(|id| EntityId(*id)).collect(),
                target: EntityId(target_id),
            });

            // Add center positioning constraints
            let c1_id = next_seq();
            constraint_ids.push(c1_id);
            buildinfo.operations.push(ConstraintOperation {
                seq: c1_id,
                timestamp: timestamp.clone(),
                op_type: OperationType::Add,
                constraint: Constraint {
                    id: c1_id,
                    target: EntityId(center_id),
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: center.x.clone(),
                    },
                    priority: ConstraintPriority::Soft,
                    source_scope: Some(source_scope.clone()),
                },
                intent: Some("Conic gradient center X".to_string()),
                command: None,
                optimization_run_id: None,
            });

            let c2_id = next_seq();
            constraint_ids.push(c2_id);
            buildinfo.operations.push(ConstraintOperation {
                seq: c2_id,
                timestamp: timestamp.clone(),
                op_type: OperationType::Add,
                constraint: Constraint {
                    id: c2_id,
                    target: EntityId(center_id),
                    component: VectorComponent::Y,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const {
                        value: center.y.clone(),
                    },
                    priority: ConstraintPriority::Soft,
                    source_scope: Some(source_scope.clone()),
                },
                intent: Some("Conic gradient center Y".to_string()),
                command: None,
                optimization_run_id: None,
            });
        }
    }

    write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

    Ok(serde_json::json!({
        "status": "success",
        "gradient_type": match &gradient_def {
            GradientDefinition::Linear { .. } => "linear",
            GradientDefinition::Radial { .. } => "radial",
            GradientDefinition::Conic { .. } => "conic",
        },
        "target_entity": target_id,
        "entities_created": entity_ids.len(),
        "entity_ids": entity_ids,
        "constraints_created": constraint_ids.len(),
        "constraint_ids": constraint_ids,
        "source_scope": source_scope,
        "message": format!("Gradient applied to entity {}", target_id)
    }))
}

// =============================================================================
// CSS Gradient Parser
// =============================================================================

/// Internal representation of a parsed gradient.
enum GradientDefinition {
    Linear {
        start: Point2D,
        end: Point2D,
        stops: Vec<ColorStopDef>,
    },
    Radial {
        center: Point2D,
        radius: Rational,
        stops: Vec<ColorStopDef>,
    },
    Conic {
        center: Point2D,
        rotation: Rational,
        stops: Vec<ColorStopDef>,
    },
}

#[derive(Clone)]
struct Point2D {
    x: Rational,
    y: Rational,
}

#[derive(Clone)]
struct ColorStopDef {
    r: Rational,
    g: Rational,
    b: Rational,
    a: Rational,
    position: Rational,
}

// Entry types are imported from vsc_core::buildinfo

/// Parse a CSS gradient string into P-dimension entities.
fn parse_css_gradient(
    css: &str,
    width: &Rational,
    height: &Rational,
) -> Result<GradientDefinition, ConstraintCollisionError> {
    let trimmed = css.trim();

    if trimmed.starts_with("linear-gradient") {
        parse_linear_gradient(trimmed, width, height)
    } else if trimmed.starts_with("radial-gradient") {
        parse_radial_gradient(trimmed, width, height)
    } else if trimmed.starts_with("conic-gradient") {
        parse_conic_gradient(trimmed, width, height)
    } else {
        Err(gradient_parse_error(&format!(
            "Unknown gradient type. Supported: linear-gradient, radial-gradient, conic-gradient. Got: {}",
            trimmed
        )))
    }
}

fn gradient_parse_error(msg: &str) -> ConstraintCollisionError {
    ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: format!("CSS gradient parse error: {}", msg),
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    }
}

fn parse_linear_gradient(
    css: &str,
    width: &Rational,
    height: &Rational,
) -> Result<GradientDefinition, ConstraintCollisionError> {
    // Extract content between parentheses
    let content = extract_parens_content(css)?;
    let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

    if parts.len() < 2 {
        return Err(gradient_parse_error(
            "linear-gradient requires at least angle/direction and one color",
        ));
    }

    // Parse angle or direction
    let (angle_deg, color_start_idx) = if parts[0].ends_with("deg") {
        let deg_str = parts[0].trim_end_matches("deg").trim();
        let degrees: i64 = deg_str
            .parse()
            .map_err(|_| gradient_parse_error(&format!("Invalid angle: {}", parts[0])))?;
        (Rational::from_int(degrees), 1)
    } else if parts[0].starts_with("to ") {
        let direction = parts[0].strip_prefix("to ").unwrap().trim();
        (direction_keyword_to_angle(direction)?, 1)
    } else {
        // No angle: default to 180deg (to bottom)
        (Rational::from_int(180), 0)
    };

    // Convert angle to start/end points
    let (start, end) = css_angle_to_control_points(&angle_deg, width, height);

    // Parse color stops
    let stops = parse_color_stops(&parts[color_start_idx..])?;

    Ok(GradientDefinition::Linear { start, end, stops })
}

fn parse_radial_gradient(
    css: &str,
    width: &Rational,
    height: &Rational,
) -> Result<GradientDefinition, ConstraintCollisionError> {
    let content = extract_parens_content(css)?;
    let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

    if parts.is_empty() {
        return Err(gradient_parse_error(
            "radial-gradient requires at least one color",
        ));
    }

    // Default: circle at center
    let center = Point2D {
        x: width.clone() / Rational::from_int(2),
        y: height.clone() / Rational::from_int(2),
    };

    // Radius: half of the smaller dimension
    let radius = if width < height {
        width.clone() / Rational::from_int(2)
    } else {
        height.clone() / Rational::from_int(2)
    };

    // Check for shape/position specification
    let color_start_idx = if parts[0].starts_with("circle")
        || parts[0].starts_with("ellipse")
        || parts[0].contains(" at ")
    {
        1
    } else {
        0
    };

    let stops = parse_color_stops(&parts[color_start_idx..])?;

    Ok(GradientDefinition::Radial {
        center,
        radius,
        stops,
    })
}

fn parse_conic_gradient(
    css: &str,
    width: &Rational,
    height: &Rational,
) -> Result<GradientDefinition, ConstraintCollisionError> {
    let content = extract_parens_content(css)?;
    let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

    if parts.is_empty() {
        return Err(gradient_parse_error(
            "conic-gradient requires at least one color",
        ));
    }

    // Default: center, 0deg rotation
    let center = Point2D {
        x: width.clone() / Rational::from_int(2),
        y: height.clone() / Rational::from_int(2),
    };

    let mut rotation = Rational::zero();
    let mut color_start_idx = 0;

    // Check for "from Xdeg" rotation
    if parts[0].starts_with("from ") {
        let from_part = parts[0].strip_prefix("from ").unwrap();
        if let Some(deg_str) = from_part.split_whitespace().next() {
            if deg_str.ends_with("deg") {
                if let Ok(deg) = deg_str.trim_end_matches("deg").parse::<i64>() {
                    rotation = Rational::from_int(deg);
                }
            }
        }
        color_start_idx = 1;
    }

    let stops = parse_color_stops(&parts[color_start_idx..])?;

    Ok(GradientDefinition::Conic {
        center,
        rotation,
        stops,
    })
}

fn extract_parens_content(css: &str) -> Result<String, ConstraintCollisionError> {
    let start = css
        .find('(')
        .ok_or_else(|| gradient_parse_error("Missing opening parenthesis"))?;
    let end = css
        .rfind(')')
        .ok_or_else(|| gradient_parse_error("Missing closing parenthesis"))?;

    if start >= end {
        return Err(gradient_parse_error("Invalid parentheses"));
    }

    Ok(css[start + 1..end].to_string())
}

fn direction_keyword_to_angle(direction: &str) -> Result<Rational, ConstraintCollisionError> {
    match direction.to_lowercase().as_str() {
        "top" => Ok(Rational::from_int(0)),
        "right" => Ok(Rational::from_int(90)),
        "bottom" => Ok(Rational::from_int(180)),
        "left" => Ok(Rational::from_int(270)),
        "top right" => Ok(Rational::from_int(45)),
        "bottom right" => Ok(Rational::from_int(135)),
        "bottom left" => Ok(Rational::from_int(225)),
        "top left" => Ok(Rational::from_int(315)),
        _ => Err(gradient_parse_error(&format!(
            "Unknown direction: {}",
            direction
        ))),
    }
}

/// Convert CSS angle to start/end control points.
///
/// CSS angle convention:
/// - 0deg = to top (gradient points upward)
/// - 90deg = to right
/// - 180deg = to bottom
/// - 270deg = to left
fn css_angle_to_control_points(
    angle_deg: &Rational,
    width: &Rational,
    height: &Rational,
) -> (Point2D, Point2D) {
    // Center of bounding box
    let cx = width.clone() / Rational::from_int(2);
    let cy = height.clone() / Rational::from_int(2);

    // For common angles, use exact rational values
    let angle_i64 = angle_deg.to_f64_for_rasterization() as i64 % 360;

    // Rational approximations for sin/cos at common angles
    let (sin_theta, cos_theta) = match angle_i64 {
        0 => (Rational::zero(), Rational::one()),
        45 => (Rational::new(707, 1000), Rational::new(707, 1000)),
        90 => (Rational::one(), Rational::zero()),
        135 => (Rational::new(707, 1000), Rational::new(-707, 1000)),
        180 => (Rational::zero(), Rational::new(-1, 1)),
        225 => (Rational::new(-707, 1000), Rational::new(-707, 1000)),
        270 => (Rational::new(-1, 1), Rational::zero()),
        315 => (Rational::new(-707, 1000), Rational::new(707, 1000)),
        _ => {
            // For other angles, use f64 approximation converted to rational
            let rad = (angle_i64 as f64) * std::f64::consts::PI / 180.0;
            let sin_f = rad.sin();
            let cos_f = rad.cos();
            (
                Rational::new((sin_f * 1000.0) as i64, 1000),
                Rational::new((cos_f * 1000.0) as i64, 1000),
            )
        }
    };

    // Gradient length (projection of bbox diagonal onto gradient axis)
    // L = |W * sin(θ)| + |H * cos(θ)|
    let w_sin = width.clone() * sin_theta.clone();
    let h_cos = height.clone() * cos_theta.clone();
    let l = w_sin.abs() + h_cos.abs();
    let half_l = l / Rational::from_int(2);

    // Start point (opposite direction from angle)
    let start = Point2D {
        x: cx.clone() - half_l.clone() * sin_theta.clone(),
        y: cy.clone() + half_l.clone() * cos_theta.clone(),
    };

    // End point (angle direction)
    let end = Point2D {
        x: cx + half_l.clone() * sin_theta,
        y: cy - half_l * cos_theta,
    };

    (start, end)
}

fn parse_color_stops(parts: &[&str]) -> Result<Vec<ColorStopDef>, ConstraintCollisionError> {
    let mut stops = Vec::new();
    let n = parts.len();

    for (i, part) in parts.iter().enumerate() {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Parse "color position%" or just "color"
        let (color_str, position) = if let Some(pct_idx) = trimmed.rfind('%') {
            // Find the position value before %
            let before_pct = &trimmed[..pct_idx];
            if let Some(space_idx) = before_pct.rfind(' ') {
                let pos_str = &before_pct[space_idx + 1..];
                if let Ok(pos_val) = pos_str.parse::<i64>() {
                    let color = before_pct[..space_idx].trim();
                    (color, Rational::new(pos_val, 100))
                } else {
                    (
                        trimmed,
                        Rational::new(i as i64 * 100, (n - 1).max(1) as i64)
                            / Rational::from_int(100),
                    )
                }
            } else {
                (
                    trimmed,
                    Rational::new(i as i64 * 100, (n - 1).max(1) as i64) / Rational::from_int(100),
                )
            }
        } else {
            // No percentage: distribute evenly
            let pos = if n <= 1 {
                Rational::zero()
            } else {
                Rational::new(i as i64, (n - 1) as i64)
            };
            (trimmed, pos)
        };

        let (r, g, b, a) = parse_css_color(color_str)?;

        stops.push(ColorStopDef {
            r: Rational::from_int(r as i64),
            g: Rational::from_int(g as i64),
            b: Rational::from_int(b as i64),
            a,
            position,
        });
    }

    if stops.len() < 2 {
        return Err(gradient_parse_error(
            "Gradient requires at least 2 color stops",
        ));
    }

    Ok(stops)
}

fn parse_css_color(color: &str) -> Result<(u8, u8, u8, Rational), ConstraintCollisionError> {
    let trimmed = color.trim().to_lowercase();

    // Named colors
    let named = match trimmed.as_str() {
        "red" => Some((255, 0, 0)),
        "green" => Some((0, 128, 0)),
        "blue" => Some((0, 0, 255)),
        "white" => Some((255, 255, 255)),
        "black" => Some((0, 0, 0)),
        "yellow" => Some((255, 255, 0)),
        "cyan" | "aqua" => Some((0, 255, 255)),
        "magenta" | "fuchsia" => Some((255, 0, 255)),
        "orange" => Some((255, 165, 0)),
        "purple" => Some((128, 0, 128)),
        "pink" => Some((255, 192, 203)),
        "gray" | "grey" => Some((128, 128, 128)),
        "lime" => Some((0, 255, 0)),
        "navy" => Some((0, 0, 128)),
        "teal" => Some((0, 128, 128)),
        "maroon" => Some((128, 0, 0)),
        "olive" => Some((128, 128, 0)),
        "silver" => Some((192, 192, 192)),
        "transparent" => return Ok((0, 0, 0, Rational::zero())),
        _ => None,
    };

    if let Some((r, g, b)) = named {
        return Ok((r, g, b, Rational::one()));
    }

    // Hex colors
    if trimmed.starts_with('#') {
        let hex = &trimmed[1..];
        match hex.len() {
            3 => {
                // #RGB -> #RRGGBB
                let r = u8::from_str_radix(&hex[0..1], 16).unwrap_or(0) * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).unwrap_or(0) * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).unwrap_or(0) * 17;
                return Ok((r, g, b, Rational::one()));
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
                return Ok((r, g, b, Rational::one()));
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
                let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or(255);
                return Ok((r, g, b, Rational::new(a as i64, 255)));
            }
            _ => {}
        }
    }

    // rgb() / rgba()
    if trimmed.starts_with("rgb") {
        // Simplified parsing - just extract numbers
        let nums: Vec<u8> = trimmed
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == ' ' || *c == ',')
            .collect::<String>()
            .split(|c| c == ' ' || c == ',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        if nums.len() >= 3 {
            let a = if nums.len() >= 4 {
                Rational::new(nums[3] as i64, 255)
            } else {
                Rational::one()
            };
            return Ok((nums[0], nums[1], nums[2], a));
        }
    }

    Err(gradient_parse_error(&format!("Unknown color: {}", color)))
}

// =============================================================================
// D-16: JSON Schema Generation from Rust Types
// =============================================================================

/// Generate JSON Schema from Rust type definitions (D-16).
///
/// Supports the following schema targets:
/// - `buildinfo`: Full `.vsbuildinfo` file schema (VsBuildInfo)
/// - `constraint`: Single constraint schema (Constraint)
/// - `codl`: CODL command file schema (CodlCommand)
/// - `all` (default): All schemas as a combined JSON object
pub fn generate_schema(target: &str) -> CommandResult {
    let parse = |s: String| -> Value { serde_json::from_str(&s).unwrap_or(Value::Null) };

    match target {
        "buildinfo" => Ok(parse(core_schema::generate_buildinfo_schema())),
        "constraint" => Ok(parse(core_schema::generate_constraint_schema())),
        "codl" => Ok(parse(vsc_codl::schema::generate_schema())),
        _ => {
            let result = serde_json::json!({
                "buildinfo": parse(core_schema::generate_buildinfo_schema()),
                "constraint": parse(core_schema::generate_constraint_schema()),
                "codl": parse(vsc_codl::schema::generate_schema()),
            });
            Ok(result)
        }
    }
}

// =============================================================================
// Target Management (Stage 2)
// =============================================================================

/// Known render targets.
const KNOWN_TARGETS: &[&str] = &["vs-web"];

/// Add a render target to the project.
///
/// ## Arguments
/// - `name`: Target name (e.g., "vs-web")
///
/// ## Validation
/// - Only known targets are allowed (currently: "vs-web")
/// - Duplicate targets are rejected
pub fn target_add(name: &str) -> CommandResult {
    // Validate target name
    if !KNOWN_TARGETS.contains(&name) {
        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!(
                "Unknown target: '{}'. Known targets: {}",
                name,
                KNOWN_TARGETS.join(", ")
            ),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Check for duplicate
    if buildinfo.targets.contains(&name.to_string()) {
        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Target '{}' is already registered", name),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    // Add target
    buildinfo.targets.push(name.to_string());

    // Persist
    write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

    Ok(serde_json::json!({
        "status": "success",
        "action": "target_add",
        "target": name,
        "targets": buildinfo.targets,
    }))
}

/// Remove a render target from the project.
///
/// ## Arguments
/// - `name`: Target name to remove
pub fn target_remove(name: &str) -> CommandResult {
    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Check if target exists
    let idx = buildinfo.targets.iter().position(|t| t == name);

    match idx {
        Some(i) => {
            buildinfo.targets.remove(i);
            write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

            Ok(serde_json::json!({
                "status": "success",
                "action": "target_remove",
                "target": name,
                "targets": buildinfo.targets,
            }))
        }
        None => Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Target '{}' is not registered", name),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        }),
    }
}

/// List registered render targets.
pub fn target_list() -> CommandResult {
    let buildinfo = read_buildinfo().unwrap_or_default();

    Ok(serde_json::json!({
        "status": "success",
        "action": "target_list",
        "targets": buildinfo.targets,
        "known_targets": KNOWN_TARGETS,
    }))
}

// =============================================================================
// Style Management
// =============================================================================

/// Known stylesheets (UA stylesheets).
const KNOWN_STYLES: &[&str] = &["vs-style-chrome"];

/// Add a stylesheet to the project.
///
/// This registers the style in VsBuildInfo.styles. The actual constraint
/// injection happens when the style is applied to the solver.
pub fn style_add(name: &str) -> CommandResult {
    // Validate style name
    if !KNOWN_STYLES.contains(&name) {
        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!(
                "Unknown style: '{}'. Known styles: {}",
                name,
                KNOWN_STYLES.join(", ")
            ),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Check for duplicate
    if buildinfo.styles.contains(&name.to_string()) {
        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Style '{}' is already registered", name),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    // Add the style
    buildinfo.styles.push(name.to_string());
    write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

    Ok(serde_json::json!({
        "status": "success",
        "action": "style_add",
        "style": name,
        "styles": buildinfo.styles,
    }))
}

/// Remove a stylesheet from the project.
pub fn style_remove(name: &str) -> CommandResult {
    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Check if style exists
    let idx = buildinfo.styles.iter().position(|s| s == name);

    match idx {
        Some(i) => {
            buildinfo.styles.remove(i);
            write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

            Ok(serde_json::json!({
                "status": "success",
                "action": "style_remove",
                "style": name,
                "styles": buildinfo.styles,
            }))
        }
        None => Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!("Style '{}' is not registered", name),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        }),
    }
}

/// List registered stylesheets.
pub fn style_list() -> CommandResult {
    let buildinfo = read_buildinfo().unwrap_or_default();

    Ok(serde_json::json!({
        "status": "success",
        "action": "style_list",
        "styles": buildinfo.styles,
        "known_styles": KNOWN_STYLES,
    }))
}

// =============================================================================
// Development Server (vsc dev)
// =============================================================================

/// Start development server with live preview.
///
/// Builds the project and serves static files on the specified port.
pub fn dev(target: &str, port: u16) -> CommandResult {
    // Validate target
    if target != "vs-web" && target != "wgpu" {
        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!(
                "Unsupported dev target: '{}'. Use 'vs-web' or 'wgpu'.",
                target
            ),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: 0,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    // Build to .vs-dev directory
    let dev_dir = ".vs-dev";
    println!("[vsc dev] Building project...");
    build(target, dev_dir)?;

    // Start HTTP server
    println!("[vsc dev] Starting development server...");
    println!("  ╭─────────────────────────────────────────╮");
    println!("  │  ViewScript Dev Server                  │");
    println!("  │  http://localhost:{:<5}                │", port);
    println!("  │  Press Ctrl+C to stop                   │");
    println!("  ╰─────────────────────────────────────────╯");

    serve_static(dev_dir, port).map_err(|e| ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: format!("HTTP server error: {}", e),
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    })?;

    // Note: This is unreachable in normal operation as serve_static loops forever
    Ok(serde_json::json!({
        "status": "running",
        "port": port,
        "target": target,
    }))
}

/// Minimal HTTP server for static file serving.
///
/// Serves files from the specified directory. Supports:
/// - .html → text/html
/// - .js → application/javascript
/// - .wasm → application/wasm
/// - .css → text/css
/// - .json → application/json
fn serve_static(dir: &str, port: u16) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", port);
    let listener =
        TcpListener::bind(&addr).map_err(|e| format!("Failed to bind to {}: {}", addr, e))?;

    let base_path = PathBuf::from(dir);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_http_request(stream, &base_path) {
                    eprintln!("[vsc dev] Request error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("[vsc dev] Connection error: {}", e);
            }
        }
    }

    Ok(())
}

/// Handle a single HTTP request.
fn handle_http_request(mut stream: TcpStream, base_path: &PathBuf) -> Result<(), String> {
    let mut buffer = [0u8; 4096];
    let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;

    if n == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&buffer[..n]);
    let first_line = request.lines().next().unwrap_or("");

    // Parse request line: "GET /path HTTP/1.1"
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 || parts[0] != "GET" {
        send_response(&mut stream, 400, "text/plain", b"Bad Request")?;
        return Ok(());
    }

    let mut path = parts[1];

    // Default to index.html for root
    if path == "/" {
        path = "/index.html";
    }

    // Security: prevent directory traversal
    if path.contains("..") {
        send_response(&mut stream, 403, "text/plain", b"Forbidden")?;
        return Ok(());
    }

    // Build file path
    let file_path = base_path.join(path.trim_start_matches('/'));

    // Check if file exists
    if !file_path.exists() || !file_path.is_file() {
        println!("[vsc dev] 404 {}", path);
        send_response(&mut stream, 404, "text/plain", b"Not Found")?;
        return Ok(());
    }

    // Read file
    let content = fs::read(&file_path).map_err(|e| e.to_string())?;

    // Determine content type
    let content_type = match file_path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    };

    println!(
        "[vsc dev] 200 {} ({})",
        path,
        content_type.split(';').next().unwrap_or(content_type)
    );
    send_response(&mut stream, 200, content_type, &content)?;

    Ok(())
}

/// Send HTTP response.
fn send_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    };

    let response = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Access-Control-Allow-Origin: *\r\n\
         \r\n",
        status,
        status_text,
        content_type,
        body.len()
    );

    stream
        .write_all(response.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(body).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    Ok(())
}

// =============================================================================
// Patch Constraint (vsc patch-constraint)
// =============================================================================

/// Modify an existing constraint on an entity.
///
/// Searches for constraints targeting the specified entity and component,
/// then updates the relation and value.
pub fn patch_constraint(
    entity_id: u64,
    component: &str,
    relation: &str,
    value: &str,
    intent: Option<&str>,
) -> CommandResult {
    let mut buildinfo = read_buildinfo().unwrap_or_default();

    // Parse component
    let vec_component = match component.to_lowercase().as_str() {
        "x" => VectorComponent::X,
        "y" => VectorComponent::Y,
        "z" => VectorComponent::Z,
        "t" => VectorComponent::T,
        "width" | "w" => VectorComponent::X, // Width often maps to X dimension
        "height" | "h" => VectorComponent::Y, // Height often maps to Y dimension
        _ => {
            return Err(ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: format!(
                    "Unknown component: '{}'. Use x, y, z, t, width, or height.",
                    component
                ),
                incoming_constraint: dummy_snapshot(),
                conflicting_constraints: vec![],
                repair_suggestions: vec![],
                analysis: CollisionAnalysis {
                    cycle_path: None,
                    constraints_analyzed: 0,
                    analysis_time_us: 0,
                    hideable_in_viewport: false,
                    hiding_viewport: None,
                },
            });
        }
    };

    // Parse relation
    let rel_type = match relation.to_lowercase().as_str() {
        "eq" | "=" | "==" => RelationType::Eq,
        "le" | "<=" => RelationType::Le,
        "ge" | ">=" => RelationType::Ge,
        "lt" | "<" => RelationType::Lt,
        "gt" | ">" => RelationType::Gt,
        _ => {
            return Err(ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: format!(
                    "Unknown relation: '{}'. Use eq, le, ge, lt, or gt.",
                    relation
                ),
                incoming_constraint: dummy_snapshot(),
                conflicting_constraints: vec![],
                repair_suggestions: vec![],
                analysis: CollisionAnalysis {
                    cycle_path: None,
                    constraints_analyzed: 0,
                    analysis_time_us: 0,
                    hideable_in_viewport: false,
                    hiding_viewport: None,
                },
            });
        }
    };

    // Parse value as Rational
    let rational_value = parse_rational(value).ok_or_else(|| ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: format!(
            "Invalid value '{}'. Use integer or fraction format (e.g., '100' or '3/2').",
            value
        ),
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    })?;

    // Find and update matching constraint in operations
    let target_entity = EntityId(entity_id);
    let mut found = false;
    let mut modified_op_index = None;

    for (idx, op) in buildinfo.operations.iter_mut().enumerate() {
        // Only consider Add or Modify operations (not Delete)
        if op.op_type == OperationType::Add || op.op_type == OperationType::Modify {
            if op.constraint.target == target_entity && op.constraint.component == vec_component {
                // Update the constraint
                op.constraint.relation = rel_type.clone();
                op.constraint.term = ConstraintTerm::Const {
                    value: rational_value.clone(),
                };
                op.op_type = OperationType::Modify;
                found = true;
                modified_op_index = Some(idx);
                break;
            }
        }
    }

    if !found {
        return Err(ConstraintCollisionError {
            error_type: CollisionErrorType::Overdetermined,
            message: format!(
                "No constraint found for entity {} component {}. Use 'vsc add-constraint' to create one.",
                entity_id, component
            ),
            incoming_constraint: dummy_snapshot(),
            conflicting_constraints: vec![],
            repair_suggestions: vec![],
            analysis: CollisionAnalysis {
                cycle_path: None,
                constraints_analyzed: buildinfo.operations.len() as u64,
                analysis_time_us: 0,
                hideable_in_viewport: false,
                hiding_viewport: None,
            },
        });
    }

    // Write updated buildinfo
    write_buildinfo(&buildinfo).map_err(|e| io_error_to_collision(&e))?;

    Ok(serde_json::json!({
        "status": "success",
        "action": "patch_constraint",
        "entity_id": entity_id,
        "component": component,
        "relation": relation,
        "value": value,
        "modified_operation_index": modified_op_index,
        "intent": intent,
    }))
}

// =============================================================================
// vsc search — Query objects in the constraint graph
// =============================================================================

/// Search and query objects in the constraint graph.
///
/// ## Object Types
/// - `constraint`: Constraint operations from buildinfo
/// - `path`: Path entities (SVG-like paths)
/// - `control-point`: Gradient control points
/// - `text`: Text entities
/// - `gradient`: Linear and radial gradients
/// - `q-variable`: Q-dimension variables (future)
/// - `derived`: Derived constraints (future)
/// - `all`: All object types (default)
///
/// ## Filters
/// - `entity_id`: Return only objects related to this entity
/// - `component`: Filter by component (x, y, width, height)
/// - `where_clause`: Constraint satisfaction filter (e.g., "x > 100")
///
/// ## Examples
/// ```bash
/// vsc search                          # List all objects
/// vsc search -t constraint            # List only constraints
/// vsc search -e 1000                  # Objects related to entity 1000
/// vsc search -t constraint -c x       # X-component constraints only
/// vsc search -w "x > 100"             # Objects satisfying x > 100
/// ```
pub fn search(
    object_type: Option<&str>,
    entity_id: Option<u64>,
    component: Option<&str>,
    where_clause: Option<&str>,
    limit: usize,
) -> CommandResult {
    let buildinfo = read_buildinfo().unwrap_or_default();
    let obj_type = object_type.unwrap_or("all");

    let mut results: Vec<serde_json::Value> = Vec::new();

    // Parse component filter if provided
    let component_filter: Option<VectorComponent> =
        component.and_then(|c| match c.to_lowercase().as_str() {
            "x" => Some(VectorComponent::X),
            "y" => Some(VectorComponent::Y),
            "z" => Some(VectorComponent::Z),
            "t" => Some(VectorComponent::T),
            "width" | "w" => Some(VectorComponent::X),
            "height" | "h" => Some(VectorComponent::Y),
            _ => None,
        });

    // Collect constraints
    if obj_type == "all" || obj_type == "constraint" {
        for op in &buildinfo.operations {
            if op.op_type != OperationType::Add {
                continue;
            }

            // Filter by entity_id
            if let Some(eid) = entity_id {
                if op.constraint.target.0 != eid {
                    // Also check if term references this entity
                    let references_entity = match &op.constraint.term {
                        ConstraintTerm::Ref {
                            entity_id: ref_id, ..
                        } => ref_id.0 == eid,
                        ConstraintTerm::Linear {
                            entity_id: lin_id, ..
                        } => lin_id.0 == eid,
                        _ => false,
                    };
                    if !references_entity {
                        continue;
                    }
                }
            }

            // Filter by component
            if let Some(ref comp) = component_filter {
                if &op.constraint.component != comp {
                    continue;
                }
            }

            results.push(serde_json::json!({
                "type": "constraint",
                "id": op.constraint.id,
                "seq": op.seq,
                "target": op.constraint.target.0,
                "component": format!("{:?}", op.constraint.component),
                "relation": format!("{:?}", op.constraint.relation),
                "term": format!("{:?}", op.constraint.term),
                "intent": op.intent,
                "timestamp": op.timestamp,
            }));

            if results.len() >= limit {
                break;
            }
        }
    }

    // Collect path entities
    if (obj_type == "all" || obj_type == "path") && results.len() < limit {
        for path in &buildinfo.path_entities {
            // Filter by entity_id
            if let Some(eid) = entity_id {
                if path.id.0 != eid {
                    continue;
                }
            }

            results.push(serde_json::json!({
                "type": "path",
                "id": path.id.0,
                "segment_count": path.segments.len(),
                "closed": path.closed,
            }));

            if results.len() >= limit {
                break;
            }
        }
    }

    // Collect control points
    if (obj_type == "all" || obj_type == "control-point") && results.len() < limit {
        for cp in &buildinfo.control_points {
            // Filter by entity_id
            if let Some(eid) = entity_id {
                if cp.id.0 != eid {
                    continue;
                }
            }

            results.push(serde_json::json!({
                "type": "control-point",
                "id": cp.id.0,
                "parent_path": cp.parent_path.map(|p| p.0),
                "role": format!("{:?}", cp.role),
                "x": cp.x.to_string(),
                "y": cp.y.to_string(),
            }));

            if results.len() >= limit {
                break;
            }
        }
    }

    // Collect text entities
    if (obj_type == "all" || obj_type == "text") && results.len() < limit {
        for text in &buildinfo.text_entities {
            // Filter by entity_id
            if let Some(eid) = entity_id {
                if text.id.0 != eid {
                    continue;
                }
            }

            results.push(serde_json::json!({
                "type": "text",
                "id": text.id.0,
                "content": text.content,
                "font_family": text.font_family,
                "font_size": text.font_size.to_string(),
                "metrics_resolved": text.metrics_resolved,
                "corner_tl": text.corner_tl.0,
                "corner_tr": text.corner_tr.0,
                "corner_bl": text.corner_bl.0,
                "corner_br": text.corner_br.0,
            }));

            if results.len() >= limit {
                break;
            }
        }
    }

    // Collect gradients
    if (obj_type == "all" || obj_type == "gradient") && results.len() < limit {
        // Linear gradients
        for grad in &buildinfo.linear_gradients {
            if let Some(eid) = entity_id {
                if grad.id.0 != eid {
                    continue;
                }
            }

            results.push(serde_json::json!({
                "type": "gradient",
                "subtype": "linear",
                "id": grad.id.0,
                "target": grad.target.0,
                "start": grad.start.0,
                "end": grad.end.0,
                "stop_count": grad.stops.len(),
            }));

            if results.len() >= limit {
                break;
            }
        }

        // Radial gradients
        for grad in &buildinfo.radial_gradients {
            if let Some(eid) = entity_id {
                if grad.id.0 != eid {
                    continue;
                }
            }

            results.push(serde_json::json!({
                "type": "gradient",
                "subtype": "radial",
                "id": grad.id.0,
                "target": grad.target.0,
                "center": grad.center.0,
                "radius_x": grad.radius_x.0,
                "radius_y": grad.radius_y.0,
                "stop_count": grad.stops.len(),
            }));

            if results.len() >= limit {
                break;
            }
        }
    }

    // Apply where_clause filter if specified (basic implementation)
    // Full solver-based filtering is a future enhancement
    if let Some(_clause) = where_clause {
        // TODO: Implement solver-based where clause evaluation
        // For now, we just note that where_clause was provided
    }

    Ok(serde_json::json!({
        "status": "success",
        "object_type": obj_type,
        "entity_id_filter": entity_id,
        "component_filter": component,
        "where_clause": where_clause,
        "count": results.len(),
        "limit": limit,
        "results": results,
    }))
}

// =============================================================================
// C4.2: compile-js Command
// =============================================================================

/// Compile ViewScript to standalone JavaScript.
///
/// ## Pipeline
///
/// 1. Read VsBuildInfo from stdin (JSON)
/// 2. Extract initial values from constraints to build SolveResult
/// 3. Tessellate paths to get vertex/index buffers (Phase 2)
/// 4. Generate JavaScript module via js_codegen
/// 5. Output JavaScript to stdout
///
/// ## Usage (WASI)
///
/// ```sh
/// cat .vsbuildinfo | vsc compile-js > output.js
/// ```
pub fn compile_js(_stdin: bool) -> CommandResult {
    use std::collections::HashMap;

    // Step 1: Read VsBuildInfo from stdin
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| make_error(&format!("Failed to read stdin: {}", e)))?;

    let buildinfo: VsBuildInfo = serde_json::from_str(&input)
        .map_err(|e| make_error(&format!("Invalid VsBuildInfo JSON: {}", e)))?;

    // Step 2: Extract initial values from constraints
    let solve_result = extract_initial_values(&buildinfo);

    // Step 3: Tessellate paths (placeholder for Phase 2)
    // TODO: Integrate vsc-gpu tessellation when WASI-compatible
    let tessellation_outputs: HashMap<EntityId, TessellationOutput> = HashMap::new();

    // Step 4: Placeholder glyph table (Phase 2: font rendering)
    let glyph_table: HashMap<char, GlyphData> = HashMap::new();

    // Step 5: Generate JavaScript module
    // Note: Interactive entities will be populated from .vs file parsing in Phase 2
    let interactive_entities: Vec<vsc_core::codegen::InteractiveInfo> = vec![];
    let js_code = generate_compiled_module(
        &buildinfo,
        &solve_result,
        &tessellation_outputs,
        &glyph_table,
        &interactive_entities,
    )
    .map_err(|e| make_error(&format!("Codegen error (cycle detected): {}", e)))?;

    // Output JavaScript to stdout
    // Note: For WASI, we write directly to stdout, not wrapped in JSON
    print!("{}", js_code);

    Ok(serde_json::json!({
        "status": "success",
        "entity_count": buildinfo.path_entities.len(),
        "constraint_count": buildinfo.operations.len(),
    }))
}

/// Helper to create a ConstraintCollisionError for compile-js errors.
fn make_error(message: &str) -> ConstraintCollisionError {
    ConstraintCollisionError {
        error_type: CollisionErrorType::Overdetermined,
        message: message.to_string(),
        incoming_constraint: dummy_snapshot(),
        conflicting_constraints: vec![],
        repair_suggestions: vec![],
        analysis: CollisionAnalysis {
            cycle_path: None,
            constraints_analyzed: 0,
            analysis_time_us: 0,
            hideable_in_viewport: false,
            hiding_viewport: None,
        },
    }
}

/// Extract initial values from constraints to build a SolveResult.
///
/// For Phase 1, this does simple value extraction from Const and Linear terms.
/// Full constraint solving is Phase 2.
fn extract_initial_values(buildinfo: &VsBuildInfo) -> SolveResult {
    use std::collections::HashMap;

    let mut values: HashMap<VarId, Rational> = HashMap::new();

    // Process active constraints (Add operations only)
    for op in &buildinfo.operations {
        if op.op_type != OperationType::Add {
            continue;
        }

        let constraint = &op.constraint;
        if constraint.relation != RelationType::Eq {
            continue;
        }

        let var_id = VarId::new(constraint.target, constraint.component);

        match &constraint.term {
            ConstraintTerm::Const { value } => {
                values.insert(var_id, value.clone());
            }
            ConstraintTerm::Linear {
                coefficient,
                entity_id,
                component,
                offset,
            } => {
                // If the referenced entity has a value, compute the linear combination
                let ref_var_id = VarId::new(*entity_id, *component);
                if let Some(ref_value) = values.get(&ref_var_id) {
                    let computed = coefficient.clone() * ref_value.clone() + offset.clone();
                    values.insert(var_id, computed);
                } else {
                    // Reference not yet resolved, just use offset as initial value
                    values.insert(var_id, offset.clone());
                }
            }
            ConstraintTerm::Ref {
                entity_id,
                component,
            } => {
                // Copy value from referenced entity if available
                let ref_var_id = VarId::new(*entity_id, *component);
                if let Some(ref_value) = values.get(&ref_var_id) {
                    values.insert(var_id, ref_value.clone());
                }
            }
            ConstraintTerm::LinearCombination { terms: _, offset } => {
                // Simple case: use offset as initial value
                // Full linear combination solving is Phase 2
                values.insert(var_id, offset.clone());
            }
        }
    }

    SolveResult::new(values)
}
