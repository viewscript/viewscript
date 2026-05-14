//! JavaScript Module Code Generation (C2)
//!
//! Generates standalone JavaScript modules from VsBuildInfo + SolveResult.
//!
//! ## Architecture
//!
//! ```text
//! === WebGPU Layer (Visual Rendering) ===
//!
//! Stage 1: Variable Declarations
//!   └─ let e1000_x = 100, e1000_y = 50, ...;
//!
//! Stage 2: update() Chain
//!   └─ function update() { e1001_x = e1000_x + 10; ... }
//!
//! Stage 3: compute_*_control_points()
//!   └─ function compute_path_42_control_points() { return [...]; }
//!
//! Stage 4: Mesh Constants
//!   └─ const MESH_42_VERTICES = new Float32Array([...]);
//!
//! Stage 5: render()
//!   └─ function render(runtime) { ... }
//!
//! Stage 6: Event Handlers (Q-variable based)
//!   └─ function onPointerMove(x, y) { ... }
//!
//! Stage 7: init() (GPU initialization)
//!   └─ export async function init(canvas) { ... }
//!
//! === DOM Layer (Interaction + Accessibility) ===
//!
//! Stage 8: mountDOM()
//!   └─ function mountDOM(container) { ... }
//!
//! Stage 9: updateDOM()
//!   └─ function updateDOM() { ... }
//!
//! Stage 10: bindEvents()
//!   └─ function bindEvents() { ... }
//!
//! Stage 11: mount() (unified entry point)
//!   └─ export async function mount(container) { ... }
//!
//! Stage 12: Module Exports
//!   └─ export { update, render, mount, ... };
//! ```

use crate::buildinfo::VsBuildInfo;
use crate::codegen::interactive::{DomElementKind, DomEventType, EventAction, InteractiveInfo};
use crate::codegen::js_expr::{rational_to_js, term_to_js_expr, VarNameMap};
use crate::solver::SolveResult;
use crate::types::*;
use std::collections::{HashMap, HashSet};

// =============================================================================
// Error Types
// =============================================================================

/// Error indicating a cycle was detected in constraint dependencies.
///
/// When constraints form a cycle (e.g., A depends on B, B depends on A),
/// there is no valid execution order. This error is reported to the user
/// via the Vite plugin as a compile-time error.
#[derive(Debug, Clone)]
pub struct CycleError {
    /// Constraint IDs involved in the cycle.
    pub involved_constraints: Vec<u64>,
}

impl std::fmt::Display for CycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Circular dependency detected in constraints: {:?}",
            self.involved_constraints
        )
    }
}

impl std::error::Error for CycleError {}

// =============================================================================
// Tessellation Output (placeholder for C3)
// =============================================================================

/// Tessellation output for a path entity.
/// This is populated by build-time tessellation before codegen.
#[derive(Debug, Clone)]
pub struct TessellationOutput {
    /// Pre-tessellated vertex data (Loop-Blinn or SDF).
    pub vertices: Vec<f32>,
    /// Index buffer.
    pub indices: Vec<u32>,
    /// Whether this is a fill (Loop-Blinn) or stroke (SDF).
    pub is_fill: bool,
}

/// Glyph data for text rendering.
#[derive(Debug, Clone)]
pub struct GlyphData {
    /// Tessellated glyph geometry.
    pub tessellation: TessellationOutput,
    /// Advance width for layout.
    pub advance_width: f32,
}

// =============================================================================
// Main Entry Point
// =============================================================================

/// Generate a complete JavaScript module from build artifacts.
///
/// ## Arguments
///
/// * `build_info` - The VsBuildInfo containing entities and constraints
/// * `solve_result` - Solved variable values from the constraint solver
/// * `tessellation_outputs` - Pre-tessellated geometry for each path entity
/// * `glyph_table` - Pre-compiled glyph data for text rendering
/// * `interactive_entities` - Interactive DOM layer entities for bilayer architecture
///
/// ## Returns
///
/// - `Ok(String)` - A complete JavaScript module
/// - `Err(CycleError)` - Circular dependency detected in constraints
pub fn generate_compiled_module(
    build_info: &VsBuildInfo,
    solve_result: &SolveResult,
    _tessellation_outputs: &HashMap<EntityId, TessellationOutput>,
    _glyph_table: &HashMap<char, GlyphData>,
    interactive_entities: &[InteractiveInfo],
) -> Result<String, CycleError> {
    let mut output = String::new();

    // Build variable name mapping
    let var_name_map = build_var_name_map(build_info, solve_result);

    // === WebGPU Layer (Visual Rendering) ===

    // Stage 1: Variable declarations
    output.push_str(&generate_variable_declarations(
        build_info,
        solve_result,
        &var_name_map,
    ));
    output.push('\n');

    // Stage 2: update() chain (may fail on cycle)
    output.push_str(&generate_update_function(build_info, &var_name_map)?);
    output.push('\n');

    // Stage 3: compute_*_control_points() functions
    output.push_str(&generate_control_point_functions(build_info, &var_name_map));
    output.push('\n');

    // Stage 4: Mesh constants (pre-tessellated vertex/index data)
    output.push_str(&generate_mesh_constants(build_info, _tessellation_outputs));
    output.push('\n');

    // Stage 5: render() function
    output.push_str(&generate_render_function(build_info, &var_name_map));
    output.push('\n');

    // Stage 6: Event handlers (Q-variable based)
    output.push_str(&generate_event_handlers(build_info, &var_name_map));
    output.push('\n');

    // Stage 7: init() function (GPU initialization)
    output.push_str(&generate_init_function(build_info));
    output.push('\n');

    // === DOM Layer (Interaction + Accessibility) ===

    // Stage 8: mountDOM() function
    output.push_str(&generate_mount_dom_function(
        interactive_entities,
        &var_name_map,
    ));
    output.push('\n');

    // Stage 9: updateDOM() function
    output.push_str(&generate_update_dom_function(
        interactive_entities,
        &var_name_map,
    ));
    output.push('\n');

    // Stage 10: bindEvents() function
    output.push_str(&generate_bind_events_function(
        interactive_entities,
        &var_name_map,
    ));
    output.push('\n');

    // Stage 11: mount() unified entry point
    output.push_str(&generate_mount_function(build_info));
    output.push('\n');

    // Stage 12: Module exports
    output.push_str(&generate_module_exports(build_info, interactive_entities));

    Ok(output)
}

// =============================================================================
// Variable Name Mapping
// =============================================================================

/// Build a mapping from VarId to JavaScript variable names.
///
/// ## Naming Strategy
///
/// 1. Q-variables: Use their declared name (e.g., "input.pointer.x" → "pointer_x")
/// 2. Other entities: Synthetic names "e{id}_{component}" (e.g., "e1000_x")
fn build_var_name_map(build_info: &VsBuildInfo, solve_result: &SolveResult) -> VarNameMap {
    let mut map = HashMap::new();

    // First, add Q-variable names
    for q_var in &build_info.q_variables {
        let var_id = q_var.target_var;
        let name = q_variable_to_js_name(&q_var.name);
        map.insert((var_id.entity, var_id.component), name);
    }

    // Then, add synthetic names for all solved variables not already named
    for (var_id, _) in &solve_result.values {
        let key = (var_id.entity, var_id.component);
        if !map.contains_key(&key) {
            let name = format!(
                "e{}_{}",
                var_id.entity.0,
                component_suffix(var_id.component)
            );
            map.insert(key, name);
        }
    }

    map
}

/// Convert a Q-variable name to a valid JavaScript identifier.
///
/// "input.pointer.x" → "pointer_x"
/// "window.width" → "window_width"
fn q_variable_to_js_name(name: &str) -> String {
    // Take the last two segments and join with underscore
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() >= 2 {
        format!("{}_{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        name.replace('.', "_")
    }
}

/// Get the suffix for a vector component.
fn component_suffix(component: VectorComponent) -> &'static str {
    match component {
        VectorComponent::X => "x",
        VectorComponent::Y => "y",
        VectorComponent::Z => "z",
        VectorComponent::T => "t",
        VectorComponent::Value => "value",
        VectorComponent::R => "r",
        VectorComponent::G => "g",
        VectorComponent::B => "b",
        VectorComponent::Alpha => "alpha",
        VectorComponent::Position => "position",
    }
}

// =============================================================================
// Stage 1: Variable Declarations
// =============================================================================

/// Generate JavaScript variable declarations with initial values.
///
/// Output format:
/// ```javascript
/// // P-dimension variables (constraint-resolved)
/// let e1000_x = 100;
/// let e1000_y = 50;
/// let pointer_x = 0;
/// ```
fn generate_variable_declarations(
    _build_info: &VsBuildInfo,
    solve_result: &SolveResult,
    var_name_map: &VarNameMap,
) -> String {
    let mut output = String::new();
    output.push_str("// P-dimension variables (constraint-resolved)\n");

    // Collect and sort variables for deterministic output
    let mut vars: Vec<_> = solve_result.values.iter().collect();
    vars.sort_by_key(|(var_id, _)| (var_id.entity.0, var_id.component as u8));

    for (var_id, value) in vars {
        let key = (var_id.entity, var_id.component);
        if let Some(name) = var_name_map.get(&key) {
            output.push_str(&format!("let {} = {};\n", name, rational_to_js(value)));
        }
    }

    output
}

// =============================================================================
// Stage 2: update() Function
// =============================================================================

/// Generate the update() function with topologically sorted constraint assignments.
///
/// Output format:
/// ```javascript
/// function update() {
///   e1001_x = e1000_x + 10;
///   e1001_y = e1000_y + 20;
/// }
/// ```
///
/// ## Returns
///
/// - `Ok(String)` - Generated update function
/// - `Err(CycleError)` - Circular dependency detected
fn generate_update_function(
    build_info: &VsBuildInfo,
    var_name_map: &VarNameMap,
) -> Result<String, CycleError> {
    let mut output = String::new();
    output.push_str("// Update chain (topologically sorted)\n");
    output.push_str("function update() {\n");

    // Collect active constraints (not deleted)
    let active_constraints = collect_active_constraints(build_info);

    // Build dependency graph and topologically sort (may fail on cycle)
    let sorted = topological_sort_constraints(&active_constraints)?;

    // Generate assignment for each constraint
    for constraint in sorted {
        if constraint.relation == RelationType::Eq {
            let target_key = (constraint.target, constraint.component);
            if let Some(target_name) = var_name_map.get(&target_key) {
                let expr = term_to_js_expr(&constraint.term, var_name_map);
                output.push_str(&format!("  {} = {};\n", target_name, expr));
            }
        }
    }

    output.push_str("}\n");
    Ok(output)
}

/// Collect active (non-deleted) constraints from buildinfo operations.
fn collect_active_constraints(build_info: &VsBuildInfo) -> Vec<Constraint> {
    let mut active: HashMap<u64, Constraint> = HashMap::new();
    let mut deleted: HashSet<u64> = HashSet::new();

    for op in &build_info.operations {
        match op.op_type {
            crate::buildinfo::OperationType::Add => {
                if !deleted.contains(&op.constraint.id) {
                    active.insert(op.constraint.id, op.constraint.clone());
                }
            }
            crate::buildinfo::OperationType::Modify => {
                if !deleted.contains(&op.constraint.id) {
                    active.insert(op.constraint.id, op.constraint.clone());
                }
            }
            crate::buildinfo::OperationType::Delete => {
                deleted.insert(op.constraint.id);
                active.remove(&op.constraint.id);
            }
            crate::buildinfo::OperationType::Merge => {
                // Merged constraints are effectively deleted
                deleted.insert(op.constraint.id);
                active.remove(&op.constraint.id);
            }
            crate::buildinfo::OperationType::LayoutMacro => {
                // Layout macro marker, no direct constraint change
            }
        }
    }

    active.into_values().collect()
}

/// Topologically sort constraints based on dependencies.
///
/// A constraint C depends on another if C's term references a variable
/// that is the target of another constraint.
///
/// ## Returns
///
/// - `Ok(Vec<Constraint>)` - Sorted constraints in dependency order
/// - `Err(CycleError)` - Cycle detected, containing IDs of involved constraints
fn topological_sort_constraints(constraints: &[Constraint]) -> Result<Vec<Constraint>, CycleError> {
    // Build target variable set
    let targets: HashSet<(EntityId, VectorComponent)> = constraints
        .iter()
        .map(|c| (c.target, c.component))
        .collect();

    // Build adjacency list (constraint index -> indices it depends on)
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); constraints.len()];
    let mut in_degree: Vec<usize> = vec![0; constraints.len()];

    // Map target -> constraint index
    let mut target_to_idx: HashMap<(EntityId, VectorComponent), usize> = HashMap::new();
    for (i, c) in constraints.iter().enumerate() {
        target_to_idx.insert((c.target, c.component), i);
    }

    // Build edges: if constraint i's term references constraint j's target,
    // then j must come before i (edge j -> i)
    for (i, constraint) in constraints.iter().enumerate() {
        let deps = get_term_dependencies(&constraint.term);
        for dep in deps {
            if targets.contains(&dep) {
                if let Some(&j) = target_to_idx.get(&dep) {
                    if i != j {
                        adj[j].push(i);
                        in_degree[i] += 1;
                    }
                }
            }
        }
    }

    // Kahn's algorithm
    let mut queue: Vec<usize> = in_degree
        .iter()
        .enumerate()
        .filter_map(|(i, &d)| if d == 0 { Some(i) } else { None })
        .collect();

    let mut result: Vec<Constraint> = Vec::with_capacity(constraints.len());

    while let Some(i) = queue.pop() {
        result.push(constraints[i].clone());
        for &j in &adj[i] {
            in_degree[j] -= 1;
            if in_degree[j] == 0 {
                queue.push(j);
            }
        }
    }

    // If there's a cycle, some constraints won't be in result.
    // Report as compile-time error with involved constraint IDs.
    if result.len() < constraints.len() {
        let involved: Vec<u64> = constraints
            .iter()
            .enumerate()
            .filter_map(|(i, c)| if in_degree[i] > 0 { Some(c.id) } else { None })
            .collect();

        return Err(CycleError {
            involved_constraints: involved,
        });
    }

    Ok(result)
}

/// Extract variable dependencies from a constraint term.
fn get_term_dependencies(term: &ConstraintTerm) -> Vec<(EntityId, VectorComponent)> {
    match term {
        ConstraintTerm::Const { .. } => vec![],
        ConstraintTerm::Ref {
            entity_id,
            component,
        } => vec![(*entity_id, *component)],
        ConstraintTerm::Linear {
            entity_id,
            component,
            ..
        } => vec![(*entity_id, *component)],
        ConstraintTerm::LinearCombination { terms, .. } => {
            terms.iter().map(|f| (f.entity_id, f.component)).collect()
        }
    }
}

// =============================================================================
// Stage 3: Control Point Functions
// =============================================================================

/// Generate compute_path_{id}_control_points() functions for each path entity.
///
/// Output format:
/// ```javascript
/// function compute_path_42_control_points() {
///   return [
///     e1000_x, e1000_y,  // anchor 0
///     e1001_x, e1001_y,  // handle 0
///     ...
///   ];
/// }
/// ```
fn generate_control_point_functions(build_info: &VsBuildInfo, var_name_map: &VarNameMap) -> String {
    let mut output = String::new();
    output.push_str("// Control point coordinate functions\n");

    for path_entry in &build_info.path_entities {
        output.push_str(&generate_path_control_point_function(
            path_entry,
            var_name_map,
        ));
        output.push('\n');
    }

    output
}

/// Generate control point function for a single path entity.
fn generate_path_control_point_function(
    path_entry: &PathEntityEntry,
    var_name_map: &VarNameMap,
) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "function compute_path_{}_control_points() {{\n",
        path_entry.id.0
    ));
    output.push_str("  return [\n");

    // Collect control point EntityIds from segments
    let control_points = collect_path_control_points(&path_entry.segments);

    for (i, entity_id) in control_points.iter().enumerate() {
        let x_key = (*entity_id, VectorComponent::X);
        let y_key = (*entity_id, VectorComponent::Y);

        let x_name = var_name_map
            .get(&x_key)
            .cloned()
            .unwrap_or_else(|| format!("e{}_x", entity_id.0));
        let y_name = var_name_map
            .get(&y_key)
            .cloned()
            .unwrap_or_else(|| format!("e{}_y", entity_id.0));

        let comment = format!("// point {}", i);
        output.push_str(&format!("    {}, {}, {}\n", x_name, y_name, comment));
    }

    output.push_str("  ];\n");
    output.push_str("}\n");
    output
}

/// Collect all control point EntityIds from path segments in order.
///
/// Each segment has a `from` endpoint that typically equals the previous segment's `to`.
/// We deduplicate these shared anchor points while preserving handles.
fn collect_path_control_points(segments: &[PathSegment]) -> Vec<EntityId> {
    let mut points = Vec::new();
    let mut seen: std::collections::HashSet<EntityId> = std::collections::HashSet::new();

    /// Helper to add a point if not already seen
    fn add_if_new(
        id: EntityId,
        points: &mut Vec<EntityId>,
        seen: &mut std::collections::HashSet<EntityId>,
    ) {
        if seen.insert(id) {
            points.push(id);
        }
    }

    for segment in segments {
        match segment {
            PathSegment::Line { from, to } => {
                add_if_new(*from, &mut points, &mut seen);
                add_if_new(*to, &mut points, &mut seen);
            }
            PathSegment::Quad { from, handle, to } => {
                add_if_new(*from, &mut points, &mut seen);
                add_if_new(*handle, &mut points, &mut seen);
                add_if_new(*to, &mut points, &mut seen);
            }
            PathSegment::Cubic {
                from,
                handle1,
                handle2,
                to,
            } => {
                add_if_new(*from, &mut points, &mut seen);
                add_if_new(*handle1, &mut points, &mut seen);
                add_if_new(*handle2, &mut points, &mut seen);
                add_if_new(*to, &mut points, &mut seen);
            }
            PathSegment::Arc { from, to, .. } => {
                add_if_new(*from, &mut points, &mut seen);
                add_if_new(*to, &mut points, &mut seen);
            }
        }
    }

    points
}

// =============================================================================
// Stage 4: Mesh Constants
// =============================================================================

/// Generate mesh constants with pre-tessellated vertex and index data.
///
/// Output format:
/// ```javascript
/// // Pre-tessellated mesh data
/// const MESH_42_VERTICES = new Float32Array([...]);
/// const MESH_42_INDICES = new Uint32Array([...]);
/// const MESH_42_PIPELINE = 'loop_blinn'; // or 'solid', 'loop_blinn_cubic'
/// ```
fn generate_mesh_constants(
    build_info: &VsBuildInfo,
    tessellation_outputs: &HashMap<EntityId, TessellationOutput>,
) -> String {
    let mut output = String::new();
    output.push_str("// Pre-tessellated mesh data (Stage 4)\n");

    for path_entry in &build_info.path_entities {
        let mesh_id = path_entry.id.0;

        if let Some(tess) = tessellation_outputs.get(&path_entry.id) {
            // Vertices as Float32Array
            output.push_str(&format!(
                "const MESH_{}_VERTICES = new Float32Array([{}]);\n",
                mesh_id,
                tess.vertices
                    .iter()
                    .map(|v| format!("{:.9}", v))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));

            // Indices as Uint32Array
            output.push_str(&format!(
                "const MESH_{}_INDICES = new Uint32Array([{}]);\n",
                mesh_id,
                tess.indices
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));

            // Pipeline type based on tessellation type
            let pipeline = if tess.is_fill {
                "loop_blinn"
            } else {
                "sdf_stroke"
            };
            output.push_str(&format!(
                "const MESH_{}_PIPELINE = '{}';\n",
                mesh_id, pipeline
            ));

            // Color from fill/stroke spec
            let color = extract_color_from_path(path_entry);
            output.push_str(&format!(
                "const MESH_{}_COLOR = [{}];\n",
                mesh_id,
                color
                    .iter()
                    .map(|c| format!("{:.6}", c))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));

            output.push('\n');
        }
    }

    output
}

/// Extract RGBA color from a path entity's fill or stroke spec.
fn extract_color_from_path(path_entry: &PathEntityEntry) -> [f32; 4] {
    // Try fill first
    if let Some(FillSpec::Solid { color }) = &path_entry.fill {
        return parse_css_color(color);
    }

    // Then stroke
    if let Some(stroke) = &path_entry.stroke {
        return parse_css_color(&stroke.color);
    }

    // Default: opaque black
    [0.0, 0.0, 0.0, 1.0]
}

/// Parse a CSS color string to RGBA (simplified parser).
///
/// Supports: #RGB, #RRGGBB, rgb(r, g, b), rgba(r, g, b, a)
fn parse_css_color(color: &str) -> [f32; 4] {
    let color = color.trim();

    // Hex format
    if color.starts_with('#') {
        let hex = &color[1..];
        if hex.len() == 3 {
            // #RGB
            let r = u8::from_str_radix(&hex[0..1], 16).unwrap_or(0);
            let g = u8::from_str_radix(&hex[1..2], 16).unwrap_or(0);
            let b = u8::from_str_radix(&hex[2..3], 16).unwrap_or(0);
            return [
                (r * 17) as f32 / 255.0,
                (g * 17) as f32 / 255.0,
                (b * 17) as f32 / 255.0,
                1.0,
            ];
        } else if hex.len() == 6 {
            // #RRGGBB
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
            return [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];
        }
    }

    // rgb/rgba format
    if color.starts_with("rgb") {
        let inner = color
            .trim_start_matches("rgba")
            .trim_start_matches("rgb")
            .trim_start_matches('(')
            .trim_end_matches(')');
        let parts: Vec<f32> = inner
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        if parts.len() >= 3 {
            return [
                parts[0] / 255.0,
                parts[1] / 255.0,
                parts[2] / 255.0,
                parts.get(3).copied().unwrap_or(1.0),
            ];
        }
    }

    // Default: opaque black
    [0.0, 0.0, 0.0, 1.0]
}

// =============================================================================
// Stage 5: render() Function
// =============================================================================

/// Generate the render() function that updates mesh positions from Q-variables.
///
/// Output format:
/// ```javascript
/// function render(runtime) {
///   // Update mesh positions from control points
///   runtime.updatePositions('mesh_42', new Float32Array(compute_path_42_control_points()));
/// }
/// ```
fn generate_render_function(build_info: &VsBuildInfo, _var_name_map: &VarNameMap) -> String {
    let mut output = String::new();
    output.push_str("// Render function (Stage 5)\n");
    output.push_str("function render(runtime) {\n");
    output.push_str("  // Update signal chain\n");
    output.push_str("  update();\n\n");

    output.push_str("  // Update mesh positions from Q-reactive control points\n");
    for path_entry in &build_info.path_entities {
        let mesh_id = path_entry.id.0;
        output.push_str(&format!(
            "  runtime.updatePositions('mesh_{}', new Float32Array(compute_path_{}_control_points()));\n",
            mesh_id, mesh_id
        ));
    }

    output.push_str("}\n");
    output
}

// =============================================================================
// Stage 6: Event Handlers
// =============================================================================

/// Generate event handler functions for Q-variable updates.
///
/// Output format:
/// ```javascript
/// function onPointerMove(x, y) {
///   pointer_x = x;
///   pointer_y = y;
/// }
/// ```
fn generate_event_handlers(build_info: &VsBuildInfo, var_name_map: &VarNameMap) -> String {
    let mut output = String::new();
    output.push_str("// Event handlers (Stage 6)\n");

    // Check for pointer Q-variables
    let has_pointer_x = var_name_map.values().any(|v| v == "pointer_x");
    let has_pointer_y = var_name_map.values().any(|v| v == "pointer_y");

    if has_pointer_x || has_pointer_y {
        output.push_str("function onPointerMove(x, y) {\n");
        if has_pointer_x {
            output.push_str("  pointer_x = x;\n");
        }
        if has_pointer_y {
            output.push_str("  pointer_y = y;\n");
        }
        output.push_str("}\n\n");
    }

    // Check for window dimensions Q-variables
    let has_window_width = var_name_map.values().any(|v| v == "window_width");
    let has_window_height = var_name_map.values().any(|v| v == "window_height");

    if has_window_width || has_window_height {
        output.push_str("function onResize(width, height) {\n");
        if has_window_width {
            output.push_str("  window_width = width;\n");
        }
        if has_window_height {
            output.push_str("  window_height = height;\n");
        }
        output.push_str("}\n\n");
    }

    // Check for time Q-variable
    let has_time = build_info
        .q_variables
        .iter()
        .any(|q| q.name.contains("time"));

    if has_time {
        output.push_str("function onFrame(timestamp) {\n");
        output.push_str("  time_t = timestamp;\n");
        output.push_str("}\n\n");
    }

    output
}

// =============================================================================
// Stage 7: init() Function
// =============================================================================

/// Generate the init() function for GPU initialization and render loop.
///
/// Output format:
/// ```javascript
/// export async function init(canvas) {
///   const { initGpu, createRuntime } = await import('@viewscript/gpu-runtime');
///   const gpu = await initGpu(canvas);
///   const runtime = createRuntime(gpu);
///
///   // Register meshes
///   runtime.registerMesh('mesh_42', { ... });
///
///   // Event listeners
///   canvas.addEventListener('pointermove', (e) => { ... });
///
///   // Render loop
///   function animate() {
///     render(runtime);
///     runtime.render();
///     requestAnimationFrame(animate);
///   }
///   animate();
///
///   return runtime;
/// }
/// ```
fn generate_init_function(build_info: &VsBuildInfo) -> String {
    let mut output = String::new();
    output.push_str("// Initialization (Stage 7)\n");
    output.push_str("export async function init(canvas) {\n");
    output.push_str(
        "  const { initGpu, createRuntime } = await import('@viewscript/gpu-runtime');\n",
    );
    output.push_str("  const gpu = await initGpu(canvas);\n");
    output.push_str("  const runtime = createRuntime(gpu);\n\n");

    // Register meshes in Z-order (path_entities order)
    output.push_str("  // Register meshes in Z-order\n");
    for path_entry in &build_info.path_entities {
        let mesh_id = path_entry.id.0;
        output.push_str(&format!("  runtime.registerMesh('mesh_{}', {{\n", mesh_id));
        output.push_str(&format!("    vertices: MESH_{}_VERTICES,\n", mesh_id));
        output.push_str(&format!("    indices: MESH_{}_INDICES,\n", mesh_id));
        output.push_str(&format!("    pipelineKey: MESH_{}_PIPELINE,\n", mesh_id));
        output.push_str(&format!("    color: MESH_{}_COLOR,\n", mesh_id));
        output.push_str("  });\n");
    }
    output.push('\n');

    // Event listeners
    output.push_str("  // Event listeners\n");
    output.push_str("  canvas.addEventListener('pointermove', (e) => {\n");
    output.push_str("    const rect = canvas.getBoundingClientRect();\n");
    output.push_str("    if (typeof onPointerMove === 'function') {\n");
    output.push_str("      onPointerMove(e.clientX - rect.left, e.clientY - rect.top);\n");
    output.push_str("    }\n");
    output.push_str("  });\n\n");

    output.push_str("  window.addEventListener('resize', () => {\n");
    output.push_str("    if (typeof onResize === 'function') {\n");
    output.push_str("      onResize(canvas.width, canvas.height);\n");
    output.push_str("    }\n");
    output.push_str("  });\n\n");

    // Render loop
    output.push_str("  // Render loop\n");
    output.push_str("  function animate(timestamp) {\n");
    output.push_str("    if (typeof onFrame === 'function') {\n");
    output.push_str("      onFrame(timestamp);\n");
    output.push_str("    }\n");
    output.push_str("    render(runtime);\n");
    output.push_str("    runtime.render({ r: 1, g: 1, b: 1, a: 1 });\n");
    output.push_str("    requestAnimationFrame(animate);\n");
    output.push_str("  }\n");
    output.push_str("  requestAnimationFrame(animate);\n\n");

    output.push_str("  return runtime;\n");
    output.push_str("}\n");
    output
}

// =============================================================================
// Stage 8: mountDOM() Function
// =============================================================================

/// Generate the mountDOM() function that creates transparent DOM elements.
///
/// Output format:
/// ```javascript
/// const _domElements = {};
///
/// function mountDOM(container) {
///   // Create overlay container
///   const overlay = document.createElement('div');
///   overlay.style.cssText = 'position:absolute;inset:0;pointer-events:none;';
///   overlay.setAttribute('aria-live', 'polite');
///
///   // Create interactive elements
///   const increment_btn = document.createElement('button');
///   increment_btn.style.cssText = 'position:absolute;...;pointer-events:auto;';
///   increment_btn.setAttribute('aria-label', 'Increment counter');
///   overlay.appendChild(increment_btn);
///   _domElements['increment_btn'] = increment_btn;
///
///   container.appendChild(overlay);
///   return overlay;
/// }
/// ```
fn generate_mount_dom_function(
    interactive_entities: &[InteractiveInfo],
    _var_name_map: &VarNameMap,
) -> String {
    let mut output = String::new();
    output.push_str("// DOM Layer: mountDOM (Stage 8)\n");
    output.push_str("const _domElements = {};\n\n");

    output.push_str("function mountDOM(container) {\n");
    output.push_str("  // Create overlay container for interaction layer\n");
    output.push_str("  const overlay = document.createElement('div');\n");
    output.push_str(
        "  overlay.style.cssText = 'position:absolute;inset:0;pointer-events:none;overflow:hidden;';\n",
    );
    output.push_str("  overlay.setAttribute('aria-live', 'polite');\n\n");

    // Generate DOM elements for each interactive entity
    for entity in interactive_entities {
        let var_name = &entity.entity_name;
        let tag = entity.dom_element.tag_name();

        output.push_str(&format!(
            "  // Entity {} (ID: {})\n",
            var_name, entity.entity_id.0
        ));
        output.push_str(&format!(
            "  const {} = document.createElement('{}');\n",
            var_name, tag
        ));

        // Style for transparent overlay element
        let base_style = match entity.dom_element {
            DomElementKind::Button => {
                "position:absolute;background:transparent;border:none;cursor:pointer;pointer-events:auto;"
            }
            DomElementKind::TextSpan => {
                "position:absolute;background:transparent;pointer-events:auto;user-select:text;"
            }
            DomElementKind::Region => {
                "position:absolute;background:transparent;pointer-events:auto;"
            }
        };
        output.push_str(&format!(
            "  {}.style.cssText = '{}';\n",
            var_name, base_style
        ));

        // ARIA attributes
        if let Some(label) = &entity.aria_label {
            output.push_str(&format!(
                "  {}.setAttribute('aria-label', '{}');\n",
                var_name,
                escape_js_string(label)
            ));
        }

        if let Some(role) = entity.dom_element.aria_role() {
            output.push_str(&format!(
                "  {}.setAttribute('role', '{}');\n",
                var_name, role
            ));
        }

        // Append to overlay and store reference
        output.push_str(&format!("  overlay.appendChild({});\n", var_name));
        output.push_str(&format!(
            "  _domElements['{}'] = {};\n\n",
            var_name, var_name
        ));
    }

    output.push_str("  container.appendChild(overlay);\n");
    output.push_str("  return overlay;\n");
    output.push_str("}\n");

    output
}

/// Escape special characters in a JavaScript string literal.
fn escape_js_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

// =============================================================================
// Stage 9: updateDOM() Function
// =============================================================================

/// Generate the updateDOM() function that synchronizes DOM positions.
///
/// Output format:
/// ```javascript
/// function updateDOM() {
///   _domElements['increment_btn'].style.transform = `translate3d(${e1000_x}px, ${e1000_y}px, 0)`;
///   _domElements['increment_btn'].style.width = `${e1000_width}px`;
///   _domElements['increment_btn'].style.height = `${e1000_height}px`;
/// }
/// ```
fn generate_update_dom_function(
    interactive_entities: &[InteractiveInfo],
    var_name_map: &VarNameMap,
) -> String {
    let mut output = String::new();
    output.push_str("// DOM Layer: updateDOM (Stage 9)\n");
    output.push_str("function updateDOM() {\n");

    for entity in interactive_entities {
        let var_name = &entity.entity_name;
        let entity_id = entity.entity_id;

        // Look up position variables
        let x_var = var_name_map
            .get(&(entity_id, VectorComponent::X))
            .cloned()
            .unwrap_or_else(|| format!("e{}_x", entity_id.0));
        let y_var = var_name_map
            .get(&(entity_id, VectorComponent::Y))
            .cloned()
            .unwrap_or_else(|| format!("e{}_y", entity_id.0));

        // Generate transform update using translate3d for GPU-accelerated positioning
        output.push_str(&format!(
            "  _domElements['{}'].style.transform = `translate3d(${{{}}}px, ${{{}}}px, 0)`;\n",
            var_name, x_var, y_var
        ));

        // If width/height variables exist, update dimensions
        // Check using Value component for width (common pattern)
        let has_width = var_name_map.contains_key(&(entity_id, VectorComponent::Value));

        if has_width {
            let width_var = var_name_map
                .get(&(entity_id, VectorComponent::Value))
                .cloned()
                .unwrap_or_else(|| format!("e{}_value", entity_id.0));

            output.push_str(&format!(
                "  _domElements['{}'].style.width = `${{{}}}px`;\n",
                var_name, width_var
            ));
        }
    }

    output.push_str("}\n");
    output
}

// =============================================================================
// Stage 10: bindEvents() Function
// =============================================================================

/// Generate the bindEvents() function that wires DOM events to state mutations.
///
/// Output format:
/// ```javascript
/// function bindEvents(runtime) {
///   _domElements['increment_btn'].addEventListener('click', () => {
///     counter += 1;
///     update();
///     updateDOM();
///     render(runtime);
///   });
/// }
/// ```
fn generate_bind_events_function(
    interactive_entities: &[InteractiveInfo],
    _var_name_map: &VarNameMap,
) -> String {
    let mut output = String::new();
    output.push_str("// DOM Layer: bindEvents (Stage 10)\n");
    output.push_str("function bindEvents(runtime) {\n");

    for entity in interactive_entities {
        let var_name = &entity.entity_name;

        for binding in &entity.event_bindings {
            let event_name = binding.event_type.event_name();

            output.push_str(&format!(
                "  _domElements['{}'].addEventListener('{}', () => {{\n",
                var_name, event_name
            ));

            // Generate action code
            match &binding.action {
                EventAction::Increment { target_var, delta } => {
                    output.push_str(&format!(
                        "    {} += {};\n",
                        target_var,
                        rational_to_js(delta)
                    ));
                }
                EventAction::Toggle { target_var, values } => {
                    let (a, b) = values;
                    output.push_str(&format!(
                        "    {} = ({} === {}) ? {} : {};\n",
                        target_var,
                        target_var,
                        rational_to_js(a),
                        rational_to_js(b),
                        rational_to_js(a)
                    ));
                }
                EventAction::SetConstant { target_var, value } => {
                    output.push_str(&format!(
                        "    {} = {};\n",
                        target_var,
                        rational_to_js(value)
                    ));
                }
                EventAction::CallHandler { handler_name } => {
                    output.push_str(&format!("    {}();\n", handler_name));
                }
            }

            // Trigger update chain, DOM sync, and WebGPU re-render
            output.push_str("    update();\n");
            output.push_str("    updateDOM();\n");
            output.push_str("    render(runtime);\n");
            output.push_str("  });\n\n");
        }
    }

    output.push_str("}\n");
    output
}

// =============================================================================
// Stage 11: mount() Function
// =============================================================================

/// Generate the unified mount() entry point.
///
/// Output format:
/// ```javascript
/// export async function mount(container) {
///   // Create canvas for WebGPU layer
///   const canvas = document.createElement('canvas');
///   canvas.style.cssText = 'position:absolute;inset:0;';
///   container.style.position = 'relative';
///   container.appendChild(canvas);
///
///   // Initialize WebGPU layer
///   const runtime = await init(canvas);
///
///   // Initialize DOM layer
///   mountDOM(container);
///   bindEvents();
///   updateDOM();
///
///   return { runtime, canvas };
/// }
/// ```
fn generate_mount_function(_build_info: &VsBuildInfo) -> String {
    let mut output = String::new();
    output.push_str("// Unified entry point (Stage 11)\n");
    output.push_str("export async function mount(container) {\n");

    // Create canvas for WebGPU
    output.push_str("  // Create canvas for WebGPU layer\n");
    output.push_str("  const canvas = document.createElement('canvas');\n");
    output.push_str(
        "  canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;';\n",
    );
    output.push_str("  container.style.position = 'relative';\n");
    output.push_str("  container.appendChild(canvas);\n\n");

    // Resize canvas to container
    output.push_str("  // Match canvas size to container\n");
    output.push_str("  const resizeCanvas = () => {\n");
    output.push_str("    const rect = container.getBoundingClientRect();\n");
    output.push_str("    canvas.width = rect.width * devicePixelRatio;\n");
    output.push_str("    canvas.height = rect.height * devicePixelRatio;\n");
    output.push_str("  };\n");
    output.push_str("  resizeCanvas();\n");
    output.push_str("  window.addEventListener('resize', resizeCanvas);\n\n");

    // Initialize WebGPU
    output.push_str("  // Initialize WebGPU layer\n");
    output.push_str("  const runtime = await init(canvas);\n\n");

    // Initialize DOM layer
    output.push_str("  // Initialize DOM interaction layer\n");
    output.push_str("  const overlay = mountDOM(container);\n");
    output.push_str("  bindEvents(runtime);\n");
    output.push_str("  updateDOM();\n\n");

    output.push_str("  return { runtime, canvas, overlay };\n");
    output.push_str("}\n");

    output
}

// =============================================================================
// Stage 12: Module Exports
// =============================================================================

/// Generate module exports.
///
/// Output format:
/// ```javascript
/// export { update, render, updateDOM };
/// export const ENTITY_IDS = [1000, 1001, ...];
/// export default { init, mount, update, render, ENTITY_IDS };
/// ```
fn generate_module_exports(
    build_info: &VsBuildInfo,
    interactive_entities: &[InteractiveInfo],
) -> String {
    let mut output = String::new();
    output.push_str("// Module exports (Stage 12)\n");

    // Collect entity IDs from path entities
    let entity_ids: Vec<u64> = build_info.path_entities.iter().map(|p| p.id.0).collect();

    // Collect interactive entity IDs
    let interactive_ids: Vec<u64> = interactive_entities.iter().map(|e| e.entity_id.0).collect();

    output.push_str(&format!(
        "export const ENTITY_IDS = [{}];\n",
        entity_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));

    if !interactive_ids.is_empty() {
        output.push_str(&format!(
            "export const INTERACTIVE_IDS = [{}];\n",
            interactive_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    output.push_str("export { update, render, updateDOM, bindEvents, mountDOM };\n");
    output.push_str("export default { init, mount, update, render, updateDOM, ENTITY_IDS };\n");

    output
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildinfo::{ConstraintOperation, OperationType};
    use crate::codegen::interactive::{EventAction, InteractiveInfo};
    use crate::solver::VarId;

    fn create_test_build_info() -> VsBuildInfo {
        let mut build_info = VsBuildInfo::default();

        // Add some constraints
        build_info.operations.push(ConstraintOperation {
            seq: 0,
            timestamp: "2026-05-14T00:00:00Z".to_string(),
            op_type: OperationType::Add,
            constraint: Constraint {
                id: 1,
                target: EntityId(1000),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const {
                    value: Rational::from_int(100),
                },
                priority: ConstraintPriority::Hard,
                source_scope: None,
            },
            intent: None,
            command: None,
            optimization_run_id: None,
        });

        build_info.operations.push(ConstraintOperation {
            seq: 1,
            timestamp: "2026-05-14T00:00:01Z".to_string(),
            op_type: OperationType::Add,
            constraint: Constraint {
                id: 2,
                target: EntityId(1001),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Linear {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(1000),
                    component: VectorComponent::X,
                    offset: Rational::from_int(10),
                },
                priority: ConstraintPriority::Hard,
                source_scope: None,
            },
            intent: None,
            command: None,
            optimization_run_id: None,
        });

        build_info
    }

    fn create_test_solve_result() -> SolveResult {
        let mut values = HashMap::new();
        values.insert(
            VarId::new(EntityId(1000), VectorComponent::X),
            Rational::from_int(100),
        );
        values.insert(
            VarId::new(EntityId(1001), VectorComponent::X),
            Rational::from_int(110),
        );
        SolveResult::new(values)
    }

    #[test]
    fn test_build_var_name_map() {
        let build_info = create_test_build_info();
        let solve_result = create_test_solve_result();

        let map = build_var_name_map(&build_info, &solve_result);

        assert_eq!(
            map.get(&(EntityId(1000), VectorComponent::X)),
            Some(&"e1000_x".to_string())
        );
        assert_eq!(
            map.get(&(EntityId(1001), VectorComponent::X)),
            Some(&"e1001_x".to_string())
        );
    }

    #[test]
    fn test_q_variable_to_js_name() {
        assert_eq!(q_variable_to_js_name("input.pointer.x"), "pointer_x");
        assert_eq!(q_variable_to_js_name("window.width"), "window_width");
        assert_eq!(q_variable_to_js_name("x"), "x");
    }

    #[test]
    fn test_generate_variable_declarations() {
        let build_info = create_test_build_info();
        let solve_result = create_test_solve_result();
        let var_name_map = build_var_name_map(&build_info, &solve_result);

        let output = generate_variable_declarations(&build_info, &solve_result, &var_name_map);

        assert!(output.contains("let e1000_x = 100;"));
        assert!(output.contains("let e1001_x = 110;"));
    }

    #[test]
    fn test_generate_update_function() {
        let build_info = create_test_build_info();
        let solve_result = create_test_solve_result();
        let var_name_map = build_var_name_map(&build_info, &solve_result);

        let output = generate_update_function(&build_info, &var_name_map).unwrap();

        assert!(output.contains("function update()"));
        // The constraint e1001_x = e1000_x + 10 should be output
        assert!(output.contains("e1001_x = e1000_x + 10;"));
    }

    #[test]
    fn test_topological_sort_simple() {
        // C1: e1000_x = 100 (no deps)
        // C2: e1001_x = e1000_x + 10 (depends on C1)
        let constraints = vec![
            Constraint {
                id: 2,
                target: EntityId(1001),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Linear {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(1000),
                    component: VectorComponent::X,
                    offset: Rational::from_int(10),
                },
                priority: ConstraintPriority::Hard,
                source_scope: None,
            },
            Constraint {
                id: 1,
                target: EntityId(1000),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const {
                    value: Rational::from_int(100),
                },
                priority: ConstraintPriority::Hard,
                source_scope: None,
            },
        ];

        let sorted = topological_sort_constraints(&constraints).unwrap();

        // C1 (e1000_x = 100) should come before C2 (e1001_x = e1000_x + 10)
        let idx_c1 = sorted.iter().position(|c| c.id == 1).unwrap();
        let idx_c2 = sorted.iter().position(|c| c.id == 2).unwrap();
        assert!(idx_c1 < idx_c2, "C1 should come before C2");
    }

    #[test]
    fn test_topological_sort_cycle_detection() {
        // Create a cycle: A depends on B, B depends on A
        let constraints = vec![
            Constraint {
                id: 1,
                target: EntityId(1000),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Ref {
                    entity_id: EntityId(1001),
                    component: VectorComponent::X,
                },
                priority: ConstraintPriority::Hard,
                source_scope: None,
            },
            Constraint {
                id: 2,
                target: EntityId(1001),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Ref {
                    entity_id: EntityId(1000),
                    component: VectorComponent::X,
                },
                priority: ConstraintPriority::Hard,
                source_scope: None,
            },
        ];

        let result = topological_sort_constraints(&constraints);
        assert!(result.is_err(), "Should detect cycle");

        let err = result.unwrap_err();
        assert_eq!(err.involved_constraints.len(), 2);
        assert!(err.involved_constraints.contains(&1));
        assert!(err.involved_constraints.contains(&2));
    }

    #[test]
    fn test_collect_active_constraints_with_delete() {
        let mut build_info = VsBuildInfo::default();

        // Add constraint
        build_info.operations.push(ConstraintOperation {
            seq: 0,
            timestamp: "2026-05-14T00:00:00Z".to_string(),
            op_type: OperationType::Add,
            constraint: Constraint {
                id: 1,
                target: EntityId(1000),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const {
                    value: Rational::from_int(100),
                },
                priority: ConstraintPriority::Hard,
                source_scope: None,
            },
            intent: None,
            command: None,
            optimization_run_id: None,
        });

        // Delete constraint
        build_info.operations.push(ConstraintOperation {
            seq: 1,
            timestamp: "2026-05-14T00:00:01Z".to_string(),
            op_type: OperationType::Delete,
            constraint: Constraint {
                id: 1,
                target: EntityId(1000),
                component: VectorComponent::X,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const {
                    value: Rational::from_int(100),
                },
                priority: ConstraintPriority::Hard,
                source_scope: None,
            },
            intent: None,
            command: None,
            optimization_run_id: None,
        });

        let active = collect_active_constraints(&build_info);
        assert!(active.is_empty(), "Deleted constraint should not be active");
    }

    #[test]
    fn test_collect_path_control_points() {
        let segments = vec![
            PathSegment::Cubic {
                from: EntityId(1),
                handle1: EntityId(2),
                handle2: EntityId(3),
                to: EntityId(4),
            },
            PathSegment::Line {
                from: EntityId(4), // Same as previous 'to'
                to: EntityId(5),
            },
        ];

        let points = collect_path_control_points(&segments);
        assert_eq!(
            points,
            vec![
                EntityId(1), // from (first segment)
                EntityId(2), // handle1
                EntityId(3), // handle2
                EntityId(4), // to
                EntityId(5), // to (second segment, from=4 already seen)
            ]
        );
    }

    // =========================================================================
    // Stage 4-8 Tests
    // =========================================================================

    #[test]
    fn test_parse_css_color_hex6() {
        let color = parse_css_color("#ff0000");
        assert!((color[0] - 1.0).abs() < 0.001); // R = 1.0
        assert!((color[1] - 0.0).abs() < 0.001); // G = 0.0
        assert!((color[2] - 0.0).abs() < 0.001); // B = 0.0
        assert!((color[3] - 1.0).abs() < 0.001); // A = 1.0
    }

    #[test]
    fn test_parse_css_color_hex3() {
        let color = parse_css_color("#f00");
        assert!((color[0] - 1.0).abs() < 0.001); // R = 1.0
        assert!((color[1] - 0.0).abs() < 0.001); // G = 0.0
        assert!((color[2] - 0.0).abs() < 0.001); // B = 0.0
    }

    #[test]
    fn test_parse_css_color_rgb() {
        let color = parse_css_color("rgb(255, 128, 0)");
        assert!((color[0] - 1.0).abs() < 0.001); // R = 1.0
        assert!((color[1] - 0.5).abs() < 0.01); // G ≈ 0.5
        assert!((color[2] - 0.0).abs() < 0.001); // B = 0.0
        assert!((color[3] - 1.0).abs() < 0.001); // A = 1.0 (default)
    }

    #[test]
    fn test_parse_css_color_rgba() {
        let color = parse_css_color("rgba(0, 0, 255, 0.5)");
        assert!((color[0] - 0.0).abs() < 0.001); // R = 0.0
        assert!((color[1] - 0.0).abs() < 0.001); // G = 0.0
        assert!((color[2] - 1.0).abs() < 0.001); // B = 1.0
        assert!((color[3] - 0.5).abs() < 0.001); // A = 0.5
    }

    #[test]
    fn test_generate_mesh_constants() {
        let build_info = VsBuildInfo {
            path_entities: vec![PathEntityEntry {
                id: EntityId(42),
                segments: vec![],
                closed: true,
                fill_rule: FillRule::NonZero,
                fill: Some(FillSpec::Solid {
                    color: "#ff0000".to_string(),
                }),
                stroke: None,
            }],
            ..Default::default()
        };

        let mut tessellation_outputs = HashMap::new();
        tessellation_outputs.insert(
            EntityId(42),
            TessellationOutput {
                vertices: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0],
                indices: vec![0, 1, 2],
                is_fill: true,
            },
        );

        let output = generate_mesh_constants(&build_info, &tessellation_outputs);

        assert!(output.contains("MESH_42_VERTICES"));
        assert!(output.contains("MESH_42_INDICES"));
        assert!(output.contains("MESH_42_PIPELINE = 'loop_blinn'"));
        assert!(output.contains("MESH_42_COLOR"));
    }

    #[test]
    fn test_generate_init_function() {
        let build_info = VsBuildInfo {
            path_entities: vec![PathEntityEntry {
                id: EntityId(100),
                segments: vec![],
                closed: true,
                fill_rule: FillRule::NonZero,
                fill: None,
                stroke: None,
            }],
            ..Default::default()
        };

        let output = generate_init_function(&build_info);

        assert!(output.contains("export async function init(canvas)"));
        assert!(output.contains("initGpu"));
        assert!(output.contains("createRuntime"));
        assert!(output.contains("runtime.registerMesh('mesh_100'"));
        assert!(output.contains("requestAnimationFrame"));
    }

    #[test]
    fn test_generate_module_exports() {
        let build_info = VsBuildInfo {
            path_entities: vec![
                PathEntityEntry {
                    id: EntityId(1),
                    segments: vec![],
                    closed: true,
                    fill_rule: FillRule::NonZero,
                    fill: None,
                    stroke: None,
                },
                PathEntityEntry {
                    id: EntityId(2),
                    segments: vec![],
                    closed: false,
                    fill_rule: FillRule::NonZero,
                    fill: None,
                    stroke: None,
                },
            ],
            ..Default::default()
        };

        let output = generate_module_exports(&build_info, &[]);

        assert!(output.contains("ENTITY_IDS = [1, 2]"));
        assert!(output.contains("export { update, render"));
        assert!(output.contains("export default"));
    }

    // =========================================================================
    // Stage 8-11 Tests (DOM Layer)
    // =========================================================================

    /// Create a counter app scenario: 2 buttons + 1 text label
    fn create_counter_interactive_entities() -> Vec<InteractiveInfo> {
        vec![
            InteractiveInfo::button(
                "increment_btn",
                EntityId(1000),
                "Increment counter",
                EventAction::Increment {
                    target_var: "counter".to_string(),
                    delta: Rational::from_int(1),
                },
            ),
            InteractiveInfo::button(
                "decrement_btn",
                EntityId(1001),
                "Decrement counter",
                EventAction::Increment {
                    target_var: "counter".to_string(),
                    delta: Rational::from_int(-1),
                },
            ),
            InteractiveInfo::text_span(
                "counter_label",
                EntityId(1002),
                Some("Counter value".to_string()),
            ),
        ]
    }

    fn create_counter_var_name_map() -> VarNameMap {
        let mut map = HashMap::new();
        map.insert((EntityId(1000), VectorComponent::X), "e1000_x".to_string());
        map.insert((EntityId(1000), VectorComponent::Y), "e1000_y".to_string());
        map.insert((EntityId(1001), VectorComponent::X), "e1001_x".to_string());
        map.insert((EntityId(1001), VectorComponent::Y), "e1001_y".to_string());
        map.insert((EntityId(1002), VectorComponent::X), "e1002_x".to_string());
        map.insert((EntityId(1002), VectorComponent::Y), "e1002_y".to_string());
        map
    }

    #[test]
    fn test_stage8_mount_dom_generates_elements() {
        let interactive = create_counter_interactive_entities();
        let var_map = create_counter_var_name_map();

        let output = generate_mount_dom_function(&interactive, &var_map);

        // Must contain mountDOM function
        assert!(
            output.contains("function mountDOM(container)"),
            "Missing mountDOM function"
        );

        // Must create overlay with aria-live
        assert!(
            output.contains("aria-live"),
            "Missing aria-live attribute on overlay"
        );

        // Must create button elements
        assert!(
            output.contains("createElement('button')"),
            "Missing button element creation"
        );

        // Must set aria-label
        assert!(
            output.contains("aria-label"),
            "Missing aria-label attribute"
        );
        assert!(
            output.contains("Increment counter"),
            "Missing increment button aria-label"
        );
        assert!(
            output.contains("Decrement counter"),
            "Missing decrement button aria-label"
        );

        // Must create text span
        assert!(
            output.contains("createElement('span')"),
            "Missing span element creation"
        );

        // Must store references in _domElements
        assert!(
            output.contains("_domElements['increment_btn']"),
            "Missing increment_btn reference"
        );
        assert!(
            output.contains("_domElements['decrement_btn']"),
            "Missing decrement_btn reference"
        );
        assert!(
            output.contains("_domElements['counter_label']"),
            "Missing counter_label reference"
        );
    }

    #[test]
    fn test_stage9_update_dom_uses_translate3d() {
        let interactive = create_counter_interactive_entities();
        let var_map = create_counter_var_name_map();

        let output = generate_update_dom_function(&interactive, &var_map);

        // Must contain updateDOM function
        assert!(
            output.contains("function updateDOM()"),
            "Missing updateDOM function"
        );

        // Must use translate3d for GPU-accelerated positioning
        assert!(
            output.contains("translate3d"),
            "Missing translate3d for GPU acceleration"
        );

        // Must reference correct variables
        assert!(output.contains("e1000_x"), "Missing e1000_x reference");
        assert!(output.contains("e1000_y"), "Missing e1000_y reference");
    }

    #[test]
    fn test_stage10_bind_events_creates_click_listeners() {
        let interactive = create_counter_interactive_entities();
        let var_map = create_counter_var_name_map();

        let output = generate_bind_events_function(&interactive, &var_map);

        // Must contain bindEvents function with runtime parameter
        assert!(
            output.contains("function bindEvents(runtime)"),
            "Missing bindEvents(runtime) function"
        );

        // Must add click event listeners
        assert!(
            output.contains("addEventListener('click'"),
            "Missing click event listener"
        );

        // Must contain increment action
        assert!(output.contains("counter += 1"), "Missing counter increment");
        assert!(
            output.contains("counter += -1"),
            "Missing counter decrement"
        );

        // Must trigger update chain after event
        assert!(
            output.contains("update();"),
            "Missing update() call after event"
        );
        assert!(
            output.contains("updateDOM();"),
            "Missing updateDOM() call after event"
        );
        assert!(
            output.contains("render(runtime);"),
            "Missing render(runtime) call after event"
        );
    }

    #[test]
    fn test_stage11_mount_function_unifies_layers() {
        let build_info = VsBuildInfo::default();

        let output = generate_mount_function(&build_info);

        // Must export mount function
        assert!(
            output.contains("export async function mount(container)"),
            "Missing mount export"
        );

        // Must create canvas for WebGPU
        assert!(
            output.contains("createElement('canvas')"),
            "Missing canvas creation"
        );

        // Must call init for WebGPU layer
        assert!(output.contains("await init(canvas)"), "Missing WebGPU init");

        // Must initialize DOM layer
        assert!(
            output.contains("mountDOM(container)"),
            "Missing mountDOM call"
        );
        assert!(
            output.contains("bindEvents(runtime)"),
            "Missing bindEvents(runtime) call"
        );
        assert!(output.contains("updateDOM()"), "Missing updateDOM call");

        // Must return combined result
        assert!(
            output.contains("runtime, canvas, overlay"),
            "Missing return values"
        );
    }

    #[test]
    fn test_counter_app_full_integration() {
        // Full counter app: 2 buttons + 1 text
        let mut build_info = VsBuildInfo::default();

        // Add counter variable constraint
        build_info.operations.push(ConstraintOperation {
            seq: 0,
            timestamp: "2026-05-14T00:00:00Z".to_string(),
            op_type: OperationType::Add,
            constraint: Constraint {
                id: 1,
                target: EntityId(9999),
                component: VectorComponent::Value,
                relation: RelationType::Eq,
                term: ConstraintTerm::Const {
                    value: Rational::from_int(0),
                },
                priority: ConstraintPriority::Hard,
                source_scope: None,
            },
            intent: None,
            command: None,
            optimization_run_id: None,
        });

        let mut solve_result = SolveResult::new(HashMap::new());
        solve_result.values.insert(
            VarId::new(EntityId(9999), VectorComponent::Value),
            Rational::from_int(0),
        );
        // Add positions for interactive elements
        solve_result.values.insert(
            VarId::new(EntityId(1000), VectorComponent::X),
            Rational::from_int(50),
        );
        solve_result.values.insert(
            VarId::new(EntityId(1000), VectorComponent::Y),
            Rational::from_int(100),
        );
        solve_result.values.insert(
            VarId::new(EntityId(1001), VectorComponent::X),
            Rational::from_int(150),
        );
        solve_result.values.insert(
            VarId::new(EntityId(1001), VectorComponent::Y),
            Rational::from_int(100),
        );
        solve_result.values.insert(
            VarId::new(EntityId(1002), VectorComponent::X),
            Rational::from_int(100),
        );
        solve_result.values.insert(
            VarId::new(EntityId(1002), VectorComponent::Y),
            Rational::from_int(50),
        );

        let interactive = create_counter_interactive_entities();
        let tessellation = HashMap::new();
        let glyphs = HashMap::new();

        let output = generate_compiled_module(
            &build_info,
            &solve_result,
            &tessellation,
            &glyphs,
            &interactive,
        )
        .expect("Should generate module");

        // Verify all required features are present
        assert!(
            output.contains("mountDOM"),
            "Missing mountDOM in full output"
        );
        assert!(
            output.contains("addEventListener('click'"),
            "Missing click listener in full output"
        );
        assert!(
            output.contains("translate3d"),
            "Missing translate3d in full output"
        );
        assert!(
            output.contains("aria-label"),
            "Missing aria-label in full output"
        );
        assert!(
            output.contains("aria-live"),
            "Missing aria-live in full output"
        );
        assert!(
            output.contains("export async function mount"),
            "Missing mount export in full output"
        );
        assert!(
            output.contains("render(runtime)"),
            "Missing render(runtime) in event handler"
        );

        // Verify INTERACTIVE_IDS export
        assert!(
            output.contains("INTERACTIVE_IDS"),
            "Missing INTERACTIVE_IDS export"
        );
        assert!(
            output.contains("1000"),
            "Missing entity 1000 in INTERACTIVE_IDS"
        );
        assert!(
            output.contains("1001"),
            "Missing entity 1001 in INTERACTIVE_IDS"
        );
        assert!(
            output.contains("1002"),
            "Missing entity 1002 in INTERACTIVE_IDS"
        );
    }

    #[test]
    fn test_escape_js_string() {
        assert_eq!(escape_js_string("hello"), "hello");
        assert_eq!(escape_js_string("it's"), "it\\'s");
        assert_eq!(escape_js_string("line\nbreak"), "line\\nbreak");
        assert_eq!(escape_js_string("back\\slash"), "back\\\\slash");
    }
}
