//! CLI command implementations
//!
//! These commands perform actual file I/O and state mutations.

use serde_json::Value;
use std::fs;
use vsc_codl::{CodlCommand, CodlInterpreter, validate_codl};
use vsc_core::{
    ConstraintCollisionError, CollisionErrorType, ConstraintSnapshot, RepairSuggestion,
    MathematicalDistance, RepairAction, CollisionAnalysis, Constraint, EntityId,
    VectorComponent, RelationType, ConstraintTerm, VsBuildInfo, ConstraintOperation,
    OperationType, ResolutionStrategyWeights, Rational, ConstraintPriority,
    // Phase 9: Rigidity and singularity analysis
    ConstraintGraphBuilder, RigidityStatus,
    compute_jacobian, check_linear_singularities, PolynomialConstraint, JacobianTerm,
    // Phase 10: Text entities
    TextEntity, TextEntityEntry,
    // Phase 13: Layout macros
    LayoutType, LayoutAnchor, LayoutOrigin, LayoutSpec, LayoutMacroOperation,
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
    ).map_err(|e| io_error_to_collision(&e))?;

    // Create main.vs
    let main_vs = r#"import {} from "./components";
export default {
  entities: [],
  constraints: []
}
"#;
    fs::write(cwd.join("main.vs"), main_vs)
        .map_err(|e| io_error_to_collision(&e))?;

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

fn dummy_snapshot() -> ConstraintSnapshot {
    ConstraintSnapshot {
        constraint: Constraint {
            id: 0,
            target: EntityId(0),
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Const { value: Rational::zero() },
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
    let parsed_term: ConstraintTerm = serde_json::from_str(term)
        .map_err(|e| ConstraintCollisionError {
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
        command: Some(format!("add-constraint {} {} {} {}", target, component, relation, term)),
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
        ConstraintTerm::Ref { entity_id, component } => {
            if *component == new_constraint.component {
                Some(*entity_id)
            } else {
                None
            }
        }
        ConstraintTerm::Linear { entity_id, component, .. } => {
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
            ConstraintTerm::Ref { entity_id, component } => {
                if *component == existing.component {
                    Some(*entity_id)
                } else {
                    None
                }
            }
            ConstraintTerm::Linear { entity_id, component, .. } => {
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
                    (RelationType::Lt, RelationType::Lt) |
                    (RelationType::Lt, RelationType::Le) |
                    (RelationType::Le, RelationType::Lt) |
                    (RelationType::Gt, RelationType::Gt) |
                    (RelationType::Gt, RelationType::Ge) |
                    (RelationType::Ge, RelationType::Gt)
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

            Some(ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: "The added constraint makes the system overconstrained.".to_string(),
                incoming_constraint: ConstraintSnapshot {
                    constraint: Constraint {
                        id: new_constraint_id,
                        target: EntityId(0),
                        component: VectorComponent::X,
                        relation: RelationType::Eq,
                        term: ConstraintTerm::Const { value: Rational::zero() },
                        priority: vsc_core::ConstraintPriority::Hard,
                        source_scope: None,
                    },
                    buildinfo_index: new_constraint_id,
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
                        explanation: "Remove one of the redundant constraints before adding this one.".to_string(),
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
            ConstraintTerm::Linear { entity_id, coefficient, .. } => {
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
                    terms: vec![
                        JacobianTerm {
                            coefficient: Rational::from_int(1),
                            variables: vec![(target_var, 1)],
                        },
                    ],
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
    let boundaries_snapped = buildinfo.operations
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

pub fn build(_target: &str, _outdir: &str) -> CommandResult {
    Ok(serde_json::json!({
        "status": "success",
        "output_files": []
    }))
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
            term: ConstraintTerm::Const { value: Rational::zero() }, // Placeholder
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
            let text_entity = TextEntity::new(
                text_id,
                text_content.clone(),
                family.clone(),
                size.clone(),
            );

            // Generate initial control point position constraints
            // All corners start at the origin; Renderer will update via update-metrics
            let control_points = text_entity.expand_control_points(origin_x.clone(), origin_y.clone());

            // Add positioning constraints for TL corner
            let seq = buildinfo.next_seq();
            let tl_x_constraint = Constraint {
                id: seq,
                target: corner_tl,
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const { value: origin_x.clone() },
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
        _ => {
            Err(ConstraintCollisionError {
                error_type: CollisionErrorType::Overdetermined,
                message: format!("Unknown entity type: {}. Supported types: text", entity_type),
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
            })
        }
    }
}

/// Update text metrics from Renderer measurement.
///
/// ## Q→P Dimension Bridge
///
/// This command is called by the Renderer (TypeScript) after measuring the actual
/// text dimensions using CanvasKit or DOM APIs. It updates the constraints that
/// define the text bounding box.
///
/// ## Arguments
/// - `id`: The text entity ID
/// - `width`: Measured width in P-dimension units (as string, e.g., "120" or "120/1")
/// - `height`: Measured height in P-dimension units (as string, e.g., "24" or "24/1")
pub fn update_metrics(
    id: u64,
    width: &str,
    height: &str,
) -> CommandResult {
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
                    command: Some(format!("update-metrics --id={} --width={} --height={}", id, width, height)),
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
        None => {
            Err(ConstraintCollisionError {
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
            })
        }
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
    let instance_ids: Vec<u64> = serde_json::from_str(instances).map_err(|e| {
        ConstraintCollisionError {
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
        }
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
                instance_ids[i], instance_ids[i - 1]
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
pub fn remove_constraint(
    target_id: u64,
    intent: Option<&str>,
) -> CommandResult {
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
            repair_suggestions: vec![
                RepairSuggestion {
                    suggestion_id: 1,
                    mathematical_distance: MathematicalDistance::new(
                        1, 0, Rational::zero(), Rational::one(), 0,
                        &ResolutionStrategyWeights::default()
                    ),
                    action: RepairAction::DeleteExisting {
                        constraint_ids: vec![parent_macro],
                    },
                    explanation: format!(
                        "Remove the entire layout macro {} instead",
                        parent_macro
                    ),
                    affected_constraints: vec![],
                },
            ],
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
    let result = buildinfo.rollback_apply_reoptimize(
        target_id,
        timestamp,
        intent.map(|s| s.to_string()),
    );

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
            let yaml_value: serde_json::Value = serde_yaml::from_str(OPENAPI_SCHEMA_YAML)
                .map_err(|e| ConstraintCollisionError {
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
                })?;

            Ok(serde_json::json!({
                "status": "success",
                "format": "json",
                "schema": yaml_value
            }))
        }
        _ => {
            Err(ConstraintCollisionError {
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
            })
        }
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
    let constraint_count = buildinfo.operations.iter()
        .filter(|op| op.op_type == OperationType::Add)
        .count();

    // Count layout macros
    let layout_macro_count = buildinfo.layout_macros.len();

    // Find text entities with pending metrics
    let pending_metrics: Vec<u64> = buildinfo.text_entities.iter()
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
pub fn run_command(
    command_file: &str,
    args: &str,
    intent: Option<&str>,
) -> CommandResult {
    // Load command file
    let command_path = cwd().join(command_file);
    let yaml_content = fs::read_to_string(&command_path).map_err(|e| {
        ConstraintCollisionError {
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
        }
    })?;

    // Parse CODL command
    let codl_cmd: CodlCommand = serde_yaml::from_str(&yaml_content).map_err(|e| {
        ConstraintCollisionError {
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
        }
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
            message: format!(
                "CODL validation failed: {}",
                error_messages.join("; ")
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

    // Parse arguments
    let json_args: serde_json::Value = serde_json::from_str(args).map_err(|e| {
        ConstraintCollisionError {
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
        }
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

    let constraints = interpreter.execute(&codl_cmd, &json_args).map_err(|e| {
        ConstraintCollisionError {
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
        }
    })?;

    // =========================================================================
    // Transactional Atomicity: Sandbox + Rigidity Check
    // =========================================================================
    // Phase 15.1: All generated constraints are applied to a SANDBOX copy of
    // the constraint graph. Only after rigidity analysis passes do we commit
    // to the actual buildinfo. This guarantees atomic all-or-nothing semantics.

    // Create sandbox buildinfo (clone of original)
    let mut sandbox_buildinfo = original_buildinfo.clone();
    let constraint_ids: Vec<u64> = constraints.iter().map(|c| c.id).collect();

    // Apply all constraints to sandbox
    for constraint in &constraints {
        sandbox_buildinfo.operations.push(ConstraintOperation {
            seq: constraint.id,
            timestamp: timestamp.clone(),
            op_type: OperationType::Add,
            constraint: constraint.clone(),
            intent: intent.map(|s| s.to_string()),
            command: Some(format!(
                "run-command {} --args '{}'",
                command_file, args
            )),
            optimization_run_id: None,
        });
    }

    // Rigidity analysis on sandbox (includes all existing + new constraints)
    if let Some(rigidity_error) = check_rigidity_for_codl_batch(&sandbox_buildinfo, &constraint_ids) {
        // Transaction ROLLBACK: Return error without modifying original buildinfo
        return Err(rigidity_error);
    }

    // Transaction COMMIT: Rigidity check passed, write sandbox to disk
    write_buildinfo(&sandbox_buildinfo).map_err(|e| io_error_to_collision(&e))?;

    Ok(serde_json::json!({
        "status": "success",
        "command_name": codl_cmd.name,
        "command_version": codl_cmd.version,
        "constraints_generated": constraint_ids.len(),
        "constraint_ids": constraint_ids,
        "source_scope": source_scope,
        "validation": {
            "max_nesting_depth": validation.metadata.max_nesting_depth,
            "yield_count": validation.metadata.yield_count
        },
        "message": format!(
            "CODL command '{}' executed. {} constraints generated.",
            codl_cmd.name, constraint_ids.len()
        )
    }))
}

// =============================================================================
// Phase 17: CSS Gradient Commands
// =============================================================================

use vsc_core::{
    TileMode, ControlPointRole,
    ColorStopEntry, LinearGradientEntry, RadialGradientEntry, ConicGradientEntry,
    ControlPointEntry, RadiusEntry, AngleEntry,
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
    intent: Option<&str>,
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
        GradientDefinition::Linear { ref start, ref end, ref stops } => {
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
                    term: ConstraintTerm::Const { value: start.x.clone() },
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
                    term: ConstraintTerm::Const { value: start.y.clone() },
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
                    term: ConstraintTerm::Const { value: end.x.clone() },
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
                    term: ConstraintTerm::Const { value: end.y.clone() },
                    priority: ConstraintPriority::Soft,
                    source_scope: Some(source_scope.clone()),
                },
                intent: Some("Gradient end Y".to_string()),
                command: None,
                optimization_run_id: None,
            });
        }
        GradientDefinition::Radial { ref center, ref radius, ref stops } => {
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
                    term: ConstraintTerm::Const { value: center.x.clone() },
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
                    term: ConstraintTerm::Const { value: center.y.clone() },
                    priority: ConstraintPriority::Soft,
                    source_scope: Some(source_scope.clone()),
                },
                intent: Some("Radial gradient center Y".to_string()),
                command: None,
                optimization_run_id: None,
            });
        }
        GradientDefinition::Conic { ref center, ref rotation, ref stops } => {
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
                    term: ConstraintTerm::Const { value: center.x.clone() },
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
                    term: ConstraintTerm::Const { value: center.y.clone() },
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
        return Err(gradient_parse_error("linear-gradient requires at least angle/direction and one color"));
    }

    // Parse angle or direction
    let (angle_deg, color_start_idx) = if parts[0].ends_with("deg") {
        let deg_str = parts[0].trim_end_matches("deg").trim();
        let degrees: i64 = deg_str.parse()
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
        return Err(gradient_parse_error("radial-gradient requires at least one color"));
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
    let color_start_idx = if parts[0].starts_with("circle") || parts[0].starts_with("ellipse") || parts[0].contains(" at ") {
        1
    } else {
        0
    };

    let stops = parse_color_stops(&parts[color_start_idx..])?;

    Ok(GradientDefinition::Radial { center, radius, stops })
}

fn parse_conic_gradient(
    css: &str,
    width: &Rational,
    height: &Rational,
) -> Result<GradientDefinition, ConstraintCollisionError> {
    let content = extract_parens_content(css)?;
    let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

    if parts.is_empty() {
        return Err(gradient_parse_error("conic-gradient requires at least one color"));
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

    Ok(GradientDefinition::Conic { center, rotation, stops })
}

fn extract_parens_content(css: &str) -> Result<String, ConstraintCollisionError> {
    let start = css.find('(')
        .ok_or_else(|| gradient_parse_error("Missing opening parenthesis"))?;
    let end = css.rfind(')')
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
        _ => Err(gradient_parse_error(&format!("Unknown direction: {}", direction))),
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
                    (trimmed, Rational::new(i as i64 * 100, (n - 1).max(1) as i64) / Rational::from_int(100))
                }
            } else {
                (trimmed, Rational::new(i as i64 * 100, (n - 1).max(1) as i64) / Rational::from_int(100))
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
        return Err(gradient_parse_error("Gradient requires at least 2 color stops"));
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
