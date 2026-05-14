//! Scene Builder: Solver Output → Renderable Scene Graph (Phase G)
//!
//! This module converts the constraint solver's output and VsBuildInfo
//! into a renderable scene graph. The output is an intermediate representation
//! that `vsc-gpu` converts to GPU-specific `CanvasNode` trees.
//!
//! ## Architecture
//!
//! ```text
//! VsBuildInfo + HashMap<VarId, Rational>
//!              │
//!              ▼
//!        SceneBuilder::build_scene()
//!              │
//!              ▼
//!        Vec<SceneNode>  ─────────▶  vsc-gpu::CanvasNode (conversion)
//! ```
//!
//! ## Why Intermediate Representation?
//!
//! `vsc-gpu::CanvasNode` contains GPU-specific fields like `ChunkId` for
//! spatial partitioning. The scene representation stays in P-dimension
//! (exact Rational coordinates) without GPU concerns.

use crate::{
    buildinfo::VsBuildInfo,
    solver::VarId,
    types::{
        ConditionId, ConditionKind, CoordRef, CrossingDirection, Edge, EntityId, FillRule,
        FillSpec, LineCap, LineJoin, PathCommand, PathEntityEntry, PostSolveCondition, Rational,
        StrokeSpec,
        TopoConstraint, UvTransform, VectorComponent,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// =============================================================================
// Scene Error Types
// =============================================================================

/// Errors that can occur during scene construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    /// A control point coordinate was not found in solver output.
    MissingCoordinate {
        entity_id: EntityId,
        component: VectorComponent,
    },
    /// A referenced gradient entity was not found.
    MissingGradient { gradient_id: EntityId },
    /// A referenced color stop was not found.
    MissingColorStop { stop_id: EntityId },
    /// Path resolution failed.
    PathResolutionFailed { entity_id: EntityId, reason: String },
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SceneError::MissingCoordinate {
                entity_id,
                component,
            } => {
                write!(
                    f,
                    "Missing coordinate: EntityId({}).{:?} not in solver output",
                    entity_id.0, component
                )
            }
            SceneError::MissingGradient { gradient_id } => {
                write!(f, "Gradient EntityId({}) not found", gradient_id.0)
            }
            SceneError::MissingColorStop { stop_id } => {
                write!(f, "Color stop EntityId({}) not found", stop_id.0)
            }
            SceneError::PathResolutionFailed { entity_id, reason } => {
                write!(
                    f,
                    "Path resolution failed for EntityId({}): {}",
                    entity_id.0, reason
                )
            }
        }
    }
}

impl std::error::Error for SceneError {}

// =============================================================================
// Scene Node Types (Intermediate Representation)
// =============================================================================

/// A node in the scene graph.
///
/// This is the intermediate representation between solver output and
/// GPU-specific `CanvasNode`. It contains resolved P-dimension coordinates
/// but no GPU-specific fields like `ChunkId`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SceneNode {
    /// A path-based shape.
    Path(ScenePathNode),
    /// A group of nodes with transform.
    Group(SceneGroupNode),
}

impl SceneNode {
    /// Get the entity ID for this node.
    pub fn entity_id(&self) -> EntityId {
        match self {
            SceneNode::Path(n) => n.entity_id,
            SceneNode::Group(n) => n.entity_id,
        }
    }

    /// Get the z-order for this node.
    pub fn z_order(&self) -> i32 {
        match self {
            SceneNode::Path(n) => n.z_order,
            SceneNode::Group(n) => n.z_order,
        }
    }
}

/// A path node in the scene graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenePathNode {
    /// Entity ID for back-reference.
    pub entity_id: EntityId,

    /// Z-order for painter's algorithm.
    pub z_order: i32,

    /// Bounding box in P-dimension (exact Rational).
    pub bounds: SceneBounds,

    /// Resolved path commands.
    pub path_data: Vec<PathCommand>,

    /// Whether the path is closed.
    pub closed: bool,

    /// Fill rule.
    pub fill_rule: FillRule,

    /// Fill style (resolved from FillSpec).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<SceneFillStyle>,

    /// Stroke style (resolved from StrokeSpec).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<SceneStrokeStyle>,
}

/// A group node in the scene graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneGroupNode {
    /// Entity ID for back-reference.
    pub entity_id: EntityId,

    /// Z-order for painter's algorithm.
    pub z_order: i32,

    /// Bounding box (union of children).
    pub bounds: SceneBounds,

    /// Child nodes.
    pub children: Vec<SceneNode>,

    /// 2D translation offset.
    pub translate: (Rational, Rational),

    /// 2D scale factor.
    pub scale: (Rational, Rational),
}

/// Bounding box in P-dimension space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneBounds {
    /// Minimum X coordinate.
    pub x_min: Rational,
    /// Minimum Y coordinate.
    pub y_min: Rational,
    /// Maximum X coordinate.
    pub x_max: Rational,
    /// Maximum Y coordinate.
    pub y_max: Rational,
}

impl SceneBounds {
    /// Create bounds from two corners.
    pub fn new(x_min: Rational, y_min: Rational, x_max: Rational, y_max: Rational) -> Self {
        Self {
            x_min,
            y_min,
            x_max,
            y_max,
        }
    }

    /// Create an empty (zero-size) bounds at origin.
    pub fn empty() -> Self {
        Self {
            x_min: Rational::zero(),
            y_min: Rational::zero(),
            x_max: Rational::zero(),
            y_max: Rational::zero(),
        }
    }

    /// Expand bounds to include a point.
    pub fn include_point(&mut self, x: &Rational, y: &Rational) {
        if *x < self.x_min {
            self.x_min = x.clone();
        }
        if *x > self.x_max {
            self.x_max = x.clone();
        }
        if *y < self.y_min {
            self.y_min = y.clone();
        }
        if *y > self.y_max {
            self.y_max = y.clone();
        }
    }

    /// Create bounds from path commands.
    pub fn from_path_commands(commands: &[PathCommand]) -> Self {
        let mut bounds = Self {
            x_min: Rational::from_int(i64::MAX),
            y_min: Rational::from_int(i64::MAX),
            x_max: Rational::from_int(i64::MIN),
            y_max: Rational::from_int(i64::MIN),
        };

        for cmd in commands {
            match cmd {
                PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => {
                    bounds.include_point(x, y);
                }
                PathCommand::QuadTo { x1, y1, x, y } => {
                    bounds.include_point(x1, y1);
                    bounds.include_point(x, y);
                }
                PathCommand::CubicTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => {
                    bounds.include_point(x1, y1);
                    bounds.include_point(x2, y2);
                    bounds.include_point(x, y);
                }
                PathCommand::ArcTo { x, y, .. } => {
                    bounds.include_point(x, y);
                }
                PathCommand::Close => {}
            }
        }

        // Handle empty path
        if bounds.x_min > bounds.x_max {
            return Self::empty();
        }

        bounds
    }
}

// =============================================================================
// Scene Fill and Stroke Styles (Resolved)
// =============================================================================

/// Resolved fill style for scene nodes.
///
/// Unlike `FillSpec` which references entities, this contains
/// the actual resolved gradient data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SceneFillStyle {
    /// Solid color fill.
    Solid {
        /// CSS color string.
        color: String,
    },
    /// Linear gradient fill.
    LinearGradient {
        /// Color stops (position 0-1, CSS color).
        stops: Vec<SceneGradientStop>,
        /// Start point (resolved coordinates).
        start: (Rational, Rational),
        /// End point (resolved coordinates).
        end: (Rational, Rational),
    },
    /// Radial gradient fill.
    RadialGradient {
        /// Color stops (position 0-1, CSS color).
        stops: Vec<SceneGradientStop>,
        /// Center point (resolved coordinates).
        center: (Rational, Rational),
        /// X-radius.
        radius_x: Rational,
        /// Y-radius.
        radius_y: Rational,
    },
    /// External texture fill.
    ///
    /// The texture is bound by the target-specific renderer at render time.
    /// P-dimension only stores the handle reference and UV transformation.
    ExternalTexture {
        /// Name of the Q-variable holding the TextureHandle.
        handle_name: String,
        /// Texture dimensions (from TextureHandle, for UV calculation).
        width: u32,
        height: u32,
        /// UV transformation for texture mapping.
        uv_transform: UvTransform,
    },
}

/// A color stop in a gradient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneGradientStop {
    /// Position along gradient (0.0 to 1.0 as Rational).
    pub position: Rational,
    /// CSS color string.
    pub color: String,
}

/// Resolved stroke style for scene nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneStrokeStyle {
    /// CSS color string.
    pub color: String,
    /// Stroke width.
    pub width: Rational,
    /// Line cap style.
    pub line_cap: LineCap,
    /// Line join style.
    pub line_join: LineJoin,
    /// Dash pattern (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dash_array: Option<Vec<Rational>>,
}

// =============================================================================
// Scene Builder
// =============================================================================

/// Builds a scene graph from solver output and VsBuildInfo.
///
/// ## Usage
///
/// ```ignore
/// let builder = SceneBuilder::new(&solutions, &build_info);
/// let scene = builder.build_scene()?;
/// // scene: Vec<SceneNode> ready for vsc-gpu conversion
/// ```
pub struct SceneBuilder<'a> {
    solutions: &'a HashMap<VarId, Rational>,
    build_info: &'a VsBuildInfo,
}

impl<'a> SceneBuilder<'a> {
    /// Create a new scene builder.
    pub fn new(solutions: &'a HashMap<VarId, Rational>, build_info: &'a VsBuildInfo) -> Self {
        Self {
            solutions,
            build_info,
        }
    }

    /// Build the scene graph from all path entities.
    ///
    /// Returns a sorted list of scene nodes (by z_order).
    pub fn build_scene(&self) -> Result<Vec<SceneNode>, SceneError> {
        let mut nodes = Vec::new();

        for path_entry in &self.build_info.path_entities {
            let node = self.build_path_node(path_entry)?;
            nodes.push(SceneNode::Path(node));
        }

        // Sort by z_order (painter's algorithm)
        nodes.sort_by_key(|n| n.z_order());

        Ok(nodes)
    }

    /// Build a single path node from a PathEntityEntry.
    fn build_path_node(&self, entry: &PathEntityEntry) -> Result<ScenePathNode, SceneError> {
        // Resolve path segments to path commands
        let path_data =
            crate::types::resolve_path_commands(&entry.segments, entry.closed, |id, component| {
                let var_id = VarId::new(id, component);
                self.solutions.get(&var_id).cloned()
            })
            .map_err(|e| SceneError::PathResolutionFailed {
                entity_id: entry.id,
                reason: e.to_string(),
            })?;

        // Calculate bounding box
        let bounds = SceneBounds::from_path_commands(&path_data);

        // Resolve fill style
        let fill = match &entry.fill {
            Some(spec) => Some(self.resolve_fill_spec(spec)?),
            None => None,
        };

        // Resolve stroke style
        let stroke = entry.stroke.as_ref().map(|s| self.resolve_stroke_spec(s));

        Ok(ScenePathNode {
            entity_id: entry.id,
            z_order: 0, // Default z-order; can be enhanced with entity metadata
            bounds,
            path_data,
            closed: entry.closed,
            fill_rule: entry.fill_rule.clone(),
            fill,
            stroke,
        })
    }

    /// Resolve FillSpec to SceneFillStyle.
    fn resolve_fill_spec(&self, spec: &FillSpec) -> Result<SceneFillStyle, SceneError> {
        match spec {
            FillSpec::Solid { color } => Ok(SceneFillStyle::Solid {
                color: color.clone(),
            }),
            FillSpec::Gradient { gradient_id } => self.resolve_gradient(*gradient_id),
            FillSpec::ExternalTexture {
                handle_name,
                uv_transform,
            } => {
                // External texture: copy handle reference and UV transform.
                // Actual texture binding happens at render time via Q-dimension lookup.
                // Width/height are placeholders - the renderer fetches from TextureHandle.
                Ok(SceneFillStyle::ExternalTexture {
                    handle_name: handle_name.clone(),
                    width: 0,  // Resolved at render time from TextureHandle
                    height: 0, // Resolved at render time from TextureHandle
                    uv_transform: uv_transform.clone().unwrap_or_default(),
                })
            }
        }
    }

    /// Resolve a gradient entity to SceneFillStyle.
    fn resolve_gradient(&self, gradient_id: EntityId) -> Result<SceneFillStyle, SceneError> {
        // Try linear gradient first
        if let Some(linear) = self
            .build_info
            .linear_gradients
            .iter()
            .find(|g| g.id == gradient_id)
        {
            // Resolve start point
            let start = self.resolve_control_point(linear.start)?;
            let end = self.resolve_control_point(linear.end)?;

            // Resolve color stops
            let stops = self.resolve_color_stops(&linear.stops)?;

            return Ok(SceneFillStyle::LinearGradient { stops, start, end });
        }

        // Try radial gradient
        if let Some(radial) = self
            .build_info
            .radial_gradients
            .iter()
            .find(|g| g.id == gradient_id)
        {
            // Resolve center point
            let center = self.resolve_control_point(radial.center)?;

            // Resolve radii
            let radius_x = self
                .build_info
                .radii
                .iter()
                .find(|r| r.id == radial.radius_x)
                .map(|r| r.value.clone())
                .unwrap_or_else(Rational::zero);

            let radius_y = self
                .build_info
                .radii
                .iter()
                .find(|r| r.id == radial.radius_y)
                .map(|r| r.value.clone())
                .unwrap_or_else(Rational::zero);

            // Resolve color stops
            let stops = self.resolve_color_stops(&radial.stops)?;

            return Ok(SceneFillStyle::RadialGradient {
                stops,
                center,
                radius_x,
                radius_y,
            });
        }

        Err(SceneError::MissingGradient { gradient_id })
    }

    /// Resolve a control point to (x, y) coordinates.
    fn resolve_control_point(
        &self,
        point_id: EntityId,
    ) -> Result<(Rational, Rational), SceneError> {
        // First try solver output
        let x_var = VarId::new(point_id, VectorComponent::X);
        let y_var = VarId::new(point_id, VectorComponent::Y);

        if let (Some(x), Some(y)) = (self.solutions.get(&x_var), self.solutions.get(&y_var)) {
            return Ok((x.clone(), y.clone()));
        }

        // Fall back to control_points in build_info
        if let Some(cp) = self
            .build_info
            .control_points
            .iter()
            .find(|cp| cp.id == point_id)
        {
            return Ok((cp.x.clone(), cp.y.clone()));
        }

        Err(SceneError::MissingCoordinate {
            entity_id: point_id,
            component: VectorComponent::X,
        })
    }

    /// Resolve color stop IDs to SceneGradientStop list.
    fn resolve_color_stops(
        &self,
        stop_ids: &[EntityId],
    ) -> Result<Vec<SceneGradientStop>, SceneError> {
        let mut stops = Vec::with_capacity(stop_ids.len());

        for stop_id in stop_ids {
            let stop_entry = self
                .build_info
                .color_stops
                .iter()
                .find(|s| s.id == *stop_id)
                .ok_or(SceneError::MissingColorStop { stop_id: *stop_id })?;

            // Convert RGBA to CSS color string
            let color = format!(
                "rgba({}, {}, {}, {})",
                stop_entry.r.to_f64_for_rasterization().round() as u8,
                stop_entry.g.to_f64_for_rasterization().round() as u8,
                stop_entry.b.to_f64_for_rasterization().round() as u8,
                stop_entry.a.to_f64_for_rasterization()
            );

            stops.push(SceneGradientStop {
                position: stop_entry.position.clone(),
                color,
            });
        }

        // Sort by position
        stops.sort_by(|a, b| a.position.cmp(&b.position));

        Ok(stops)
    }

    /// Resolve StrokeSpec to SceneStrokeStyle (direct field mapping).
    fn resolve_stroke_spec(&self, spec: &StrokeSpec) -> SceneStrokeStyle {
        SceneStrokeStyle {
            color: spec.color.clone(),
            width: spec.width.clone(),
            line_cap: spec.line_cap,
            line_join: spec.line_join,
            dash_array: spec.dash_array.clone(),
        }
    }

    /// Derive topological constraints from path entities.
    ///
    /// Analyzes `VsBuildInfo.path_entities` to identify entities that share
    /// control points, and generates `TopoConstraint::Equal` for their
    /// corresponding edges to ensure topology preservation during rasterization.
    ///
    /// Also calls `derive_value_equality_constraints` to detect independent
    /// variables that happen to resolve to the same `Rational` value and
    /// adds `TopoConstraint::Equal` for them (topology-rounding D-19).
    ///
    /// ## Algorithm
    ///
    /// 1. Build reverse map: control point ID → set of entity IDs that reference it
    /// 2. For each control point shared by multiple entities:
    ///    - Generate Equal constraints for corresponding edges
    ///    - Currently constrains all four edges (Left, Right, Top, Bottom)
    ///      for shared bounding box corners
    /// 3. Collect value-equality constraints from solver solutions
    ///
    /// ## Example
    ///
    /// If entity A and entity B both reference control point P:
    /// ```text
    /// A.path uses P for bottom-right corner
    /// B.path uses P for top-left corner
    /// ```
    ///
    /// This generates constraints ensuring A and B's bounds at P round together.
    pub fn derive_topo_constraints(&self) -> Vec<TopoConstraint> {
        // Step 1: Build reverse map from control point ID to entity IDs
        let mut cp_to_entities: HashMap<EntityId, HashSet<EntityId>> = HashMap::new();

        for path_entry in &self.build_info.path_entities {
            let control_points = path_entry.referenced_control_points();
            for cp_id in control_points {
                cp_to_entities
                    .entry(cp_id)
                    .or_default()
                    .insert(path_entry.id);
            }
        }

        // Step 2: Generate constraints for entities sharing control points
        let mut constraints = Vec::new();

        for (_cp_id, entity_ids) in &cp_to_entities {
            // Only generate constraints when multiple entities share a control point
            if entity_ids.len() < 2 {
                continue;
            }

            // Convert to sorted Vec for deterministic iteration
            let mut entities: Vec<EntityId> = entity_ids.iter().copied().collect();
            entities.sort_by_key(|e| e.0);

            // Generate pairwise Equal constraints for all edges
            // This ensures entities sharing a control point have their
            // bounding boxes constrained together at that point
            for i in 0..entities.len() {
                for j in (i + 1)..entities.len() {
                    let entity_a = entities[i];
                    let entity_b = entities[j];

                    // Constrain all four edges for shared corners
                    for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
                        let a: CoordRef = (entity_a, edge);
                        let b: CoordRef = (entity_b, edge);
                        constraints.push(TopoConstraint::Equal { a, b });
                    }
                }
            }
        }

        // Step 3: Add value-equality constraints (D-19 topology-rounding)
        constraints.extend(derive_value_equality_constraints(self.solutions));

        constraints
    }
}

// =============================================================================
// D-19: Value-Equality Constraint Derivation (topology-rounding)
// =============================================================================

/// Map a `VectorComponent` to the representative `Edge` used in `CoordRef`.
///
/// Only `X` and `Y` components have a natural spatial edge interpretation.
/// `X` maps to `Edge::Left` (horizontal axis representative) and `Y` maps
/// to `Edge::Top` (vertical axis representative).  All other components
/// (Z, T, Value, R, G, B, Alpha, Position) are spatial-agnostic and are
/// excluded from topology-rounding constraints.
fn component_to_edge(component: VectorComponent) -> Option<Edge> {
    match component {
        VectorComponent::X => Some(Edge::Left),
        VectorComponent::Y => Some(Edge::Top),
        _ => None,
    }
}

/// Derive `TopoConstraint::Equal` for pairs of `VarId`s that share the same
/// resolved `Rational` value, even when they have no shared control point.
///
/// ## Algorithm
///
/// 1. Iterate `solutions` to collect all `(VarId, Rational)` pairs whose
///    component maps to a spatial `Edge` (X → Left, Y → Top).
/// 2. Group `VarId`s by value into a `HashMap<Rational, Vec<VarId>>`.
/// 3. For each group with ≥ 2 members, emit `TopoConstraint::Equal` constraints
///    using the first member as the base, pairing it with every other member.
///
/// ## Example
///
/// Two independent points both resolved to `x = 50/1`:
/// ```text
/// EntityId(1).X = 50  →  VarId { entity: 1, component: X }
/// EntityId(3).X = 50  →  VarId { entity: 3, component: X }
/// ```
/// Result: `TopoConstraint::Equal { a: (EntityId(1), Edge::Left), b: (EntityId(3), Edge::Left) }`
pub fn derive_value_equality_constraints(
    solutions: &HashMap<VarId, Rational>,
) -> Vec<TopoConstraint> {
    // Group VarId by resolved Rational value (only for spatially-mapped components)
    let mut value_groups: HashMap<Rational, Vec<VarId>> = HashMap::new();

    for (&var_id, value) in solutions {
        // Only consider components that map to a spatial Edge
        if component_to_edge(var_id.component).is_some() {
            value_groups.entry(value.clone()).or_default().push(var_id);
        }
    }

    let mut constraints = Vec::new();

    for (_, mut var_ids) in value_groups {
        if var_ids.len() < 2 {
            continue;
        }

        // Sort for deterministic output (entity id first, then component via debug string)
        var_ids.sort_by(|a, b| {
            a.entity
                .0
                .cmp(&b.entity.0)
                .then_with(|| format!("{:?}", a.component).cmp(&format!("{:?}", b.component)))
        });

        let base = var_ids[0];
        let base_edge = component_to_edge(base.component)
            .expect("component_to_edge: already filtered to Some above");
        let a: CoordRef = (base.entity, base_edge);

        for &other in &var_ids[1..] {
            let other_edge = component_to_edge(other.component)
                .expect("component_to_edge: already filtered to Some above");
            let b: CoordRef = (other.entity, other_edge);
            constraints.push(TopoConstraint::Equal { a, b });
        }
    }

    constraints
}

// =============================================================================
// Post-Solve Condition Evaluation (FFI Trigger Support)
// =============================================================================

/// Axis-aligned bounding box for geometric condition evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundingBox {
    pub min_x: Rational,
    pub min_y: Rational,
    pub max_x: Rational,
    pub max_y: Rational,
}

impl BoundingBox {
    /// Create a new bounding box from min/max coordinates.
    pub fn new(min_x: Rational, min_y: Rational, max_x: Rational, max_y: Rational) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// Create a bounding box from a collection of points, computing min/max.
    pub fn from_points(points: impl IntoIterator<Item = (Rational, Rational)>) -> Option<Self> {
        let mut iter = points.into_iter();
        let (first_x, first_y) = iter.next()?;

        let mut min_x = first_x.clone();
        let mut max_x = first_x;
        let mut min_y = first_y.clone();
        let mut max_y = first_y;

        for (x, y) in iter {
            if x < min_x {
                min_x = x.clone();
            }
            if x > max_x {
                max_x = x;
            }
            if y < min_y {
                min_y = y.clone();
            }
            if y > max_y {
                max_y = y;
            }
        }

        Some(Self {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    }

    /// Check if this bounding box overlaps with another.
    pub fn overlaps(&self, other: &BoundingBox) -> bool {
        self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_y <= other.max_y
            && self.max_y >= other.min_y
    }
}

/// Collect control point EntityIds for an entity based on its type.
///
/// Returns a list of EntityIds whose X/Y coordinates should be used
/// to compute the bounding box.
fn collect_entity_control_points(
    entity_id: EntityId,
    build_info: &VsBuildInfo,
) -> Vec<EntityId> {
    // Check if it's a path entity
    if let Some(path) = build_info.path_entities.iter().find(|p| p.id == entity_id) {
        return path.referenced_control_points();
    }

    // Check if it's a text entity
    if let Some(text) = build_info.text_entities.iter().find(|t| t.id == entity_id) {
        return vec![text.corner_tl, text.corner_tr, text.corner_bl, text.corner_br];
    }

    // Check if it's a control point itself (single point collision)
    if build_info.control_points.iter().any(|cp| cp.id == entity_id) {
        return vec![entity_id];
    }

    // Fallback: treat entity_id as a single point
    vec![entity_id]
}

/// Compute the bounding box for an entity from solved variable values.
///
/// Uses `VsBuildInfo` to determine entity structure (path, text, etc.)
/// and collects all control points to compute the AABB.
///
/// Returns `None` if any required coordinate is missing from the solution.
pub fn compute_entity_bounds(
    entity_id: EntityId,
    values: &HashMap<VarId, Rational>,
    build_info: &VsBuildInfo,
) -> Option<BoundingBox> {
    let control_point_ids = collect_entity_control_points(entity_id, build_info);

    let points: Vec<(Rational, Rational)> = control_point_ids
        .iter()
        .filter_map(|&cp_id| {
            let x = values.get(&VarId::new(cp_id, VectorComponent::X))?;
            let y = values.get(&VarId::new(cp_id, VectorComponent::Y))?;
            Some((x.clone(), y.clone()))
        })
        .collect();

    BoundingBox::from_points(points)
}

/// Evaluate a single condition against the current solver state.
fn evaluate_condition(
    condition: &PostSolveCondition,
    values: &HashMap<VarId, Rational>,
    build_info: &VsBuildInfo,
) -> bool {
    match &condition.kind {
        ConditionKind::BoundsOverlap { entity_a, entity_b } => {
            let bounds_a = match compute_entity_bounds(*entity_a, values, build_info) {
                Some(b) => b,
                None => return false,
            };
            let bounds_b = match compute_entity_bounds(*entity_b, values, build_info) {
                Some(b) => b,
                None => return false,
            };
            bounds_a.overlaps(&bounds_b)
        }

        ConditionKind::PropertiesEqual {
            entity_a,
            component_a,
            entity_b,
            component_b,
        } => {
            let var_a = VarId::new(*entity_a, *component_a);
            let var_b = VarId::new(*entity_b, *component_b);

            match (values.get(&var_a), values.get(&var_b)) {
                (Some(val_a), Some(val_b)) => val_a == val_b,
                _ => false,
            }
        }

        ConditionKind::PropertyLessThan {
            entity_a,
            component_a,
            entity_b,
            component_b,
        } => {
            let var_a = VarId::new(*entity_a, *component_a);
            let var_b = VarId::new(*entity_b, *component_b);

            match (values.get(&var_a), values.get(&var_b)) {
                (Some(val_a), Some(val_b)) => val_a < val_b,
                _ => false,
            }
        }

        ConditionKind::ThresholdCrossing {
            entity,
            component,
            threshold,
            direction,
        } => {
            let var = VarId::new(*entity, *component);

            match values.get(&var) {
                Some(value) => match direction {
                    CrossingDirection::Rising => value > threshold,
                    CrossingDirection::Falling => value < threshold,
                },
                None => false,
            }
        }
    }
}

/// Evaluate post-solve conditions and return newly triggered conditions.
///
/// This function implements rising-edge detection:
/// - Only returns conditions that transitioned from false→true this frame
/// - Conditions that were already true in `prev_satisfied` are NOT returned
///
/// # Arguments
///
/// * `conditions` - All registered post-solve conditions
/// * `values` - Current frame's solver output (resolved VarId→Rational)
/// * `build_info` - Build info containing entity structure information
/// * `prev_satisfied` - Set of ConditionIds that were satisfied in the previous frame
///
/// # Returns
///
/// A tuple of:
/// * `Vec<ConditionId>` - Conditions that triggered this frame (false→true transitions)
/// * `HashSet<ConditionId>` - All conditions satisfied this frame (for next frame's prev_satisfied)
pub fn evaluate_conditions(
    conditions: &[PostSolveCondition],
    values: &HashMap<VarId, Rational>,
    build_info: &VsBuildInfo,
    prev_satisfied: &HashSet<ConditionId>,
) -> (Vec<ConditionId>, HashSet<ConditionId>) {
    let mut triggered = Vec::new();
    let mut currently_satisfied = HashSet::new();

    for condition in conditions {
        let is_satisfied = evaluate_condition(condition, values, build_info);

        if is_satisfied {
            currently_satisfied.insert(condition.id);

            // Rising edge: was false, now true
            if !prev_satisfied.contains(&condition.id) {
                triggered.push(condition.id);
            }
        }
    }

    (triggered, currently_satisfied)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PathSegment;

    /// Helper to create a solver solution map.
    fn make_solutions(points: &[(u64, i64, i64)]) -> HashMap<VarId, Rational> {
        let mut solutions = HashMap::new();
        for (id, x, y) in points {
            solutions.insert(
                VarId::new(EntityId(*id), VectorComponent::X),
                Rational::from_int(*x),
            );
            solutions.insert(
                VarId::new(EntityId(*id), VectorComponent::Y),
                Rational::from_int(*y),
            );
        }
        solutions
    }

    #[test]
    fn test_build_triangle_with_solid_fill() {
        // Create a triangle with 3 control points
        let solutions = make_solutions(&[
            (1, 0, 0),    // Point 1: (0, 0)
            (2, 100, 0),  // Point 2: (100, 0)
            (3, 50, 100), // Point 3: (50, 100)
        ]);

        let mut build_info = VsBuildInfo::default();

        // Add triangle path entity
        build_info.path_entities.push(PathEntityEntry {
            id: EntityId(100),
            segments: vec![
                PathSegment::Line {
                    from: EntityId(1),
                    to: EntityId(2),
                },
                PathSegment::Line {
                    from: EntityId(2),
                    to: EntityId(3),
                },
                PathSegment::Line {
                    from: EntityId(3),
                    to: EntityId(1),
                },
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: Some(FillSpec::Solid {
                color: "#ff0000".to_string(),
            }),
            stroke: None,
        });

        // Build scene
        let builder = SceneBuilder::new(&solutions, &build_info);
        let scene = builder.build_scene().expect("build_scene failed");

        // Verify: 1 path node
        assert_eq!(scene.len(), 1);

        let node = match &scene[0] {
            SceneNode::Path(p) => p,
            _ => panic!("Expected Path node"),
        };

        // Verify path_data: MoveTo, LineTo, LineTo, LineTo, Close
        // (3 Line segments: first generates MoveTo+LineTo, remaining generate LineTo each)
        assert_eq!(node.path_data.len(), 5);
        assert!(matches!(node.path_data[0], PathCommand::MoveTo { .. }));
        assert!(matches!(node.path_data[1], PathCommand::LineTo { .. }));
        assert!(matches!(node.path_data[2], PathCommand::LineTo { .. }));
        assert!(matches!(node.path_data[3], PathCommand::LineTo { .. }));
        assert!(matches!(node.path_data[4], PathCommand::Close));

        // Verify coordinates
        if let PathCommand::MoveTo { x, y } = &node.path_data[0] {
            assert_eq!(*x, Rational::from_int(0));
            assert_eq!(*y, Rational::from_int(0));
        }

        // Verify fill
        assert!(matches!(
            &node.fill,
            Some(SceneFillStyle::Solid { color }) if color == "#ff0000"
        ));

        // Verify bounds
        assert_eq!(node.bounds.x_min, Rational::from_int(0));
        assert_eq!(node.bounds.x_max, Rational::from_int(100));
        assert_eq!(node.bounds.y_min, Rational::from_int(0));
        assert_eq!(node.bounds.y_max, Rational::from_int(100));
    }

    #[test]
    fn test_build_path_with_linear_gradient() {
        use crate::buildinfo::{ColorStopEntry, ControlPointEntry, LinearGradientEntry};
        use crate::types::{ControlPointRole, TileMode};

        // Create a simple rectangle path
        let solutions = make_solutions(&[
            (1, 0, 0),
            (2, 100, 0),
            (3, 100, 100),
            (4, 0, 100),
            // Gradient control points
            (10, 0, 50),   // gradient start
            (11, 100, 50), // gradient end
        ]);

        let mut build_info = VsBuildInfo::default();

        // Add gradient control points (using Anchor role for gradient endpoints)
        build_info.control_points.push(ControlPointEntry {
            id: EntityId(10),
            x: Rational::from_int(0),
            y: Rational::from_int(50),
            role: ControlPointRole::Anchor,
            parent_path: None,
        });
        build_info.control_points.push(ControlPointEntry {
            id: EntityId(11),
            x: Rational::from_int(100),
            y: Rational::from_int(50),
            role: ControlPointRole::Anchor,
            parent_path: None,
        });

        // Add color stops
        build_info.color_stops.push(ColorStopEntry {
            id: EntityId(20),
            r: Rational::from_int(255),
            g: Rational::from_int(0),
            b: Rational::from_int(0),
            a: Rational::from_int(1),
            position: Rational::from_int(0),
        });
        build_info.color_stops.push(ColorStopEntry {
            id: EntityId(21),
            r: Rational::from_int(0),
            g: Rational::from_int(0),
            b: Rational::from_int(255),
            a: Rational::from_int(1),
            position: Rational::from_int(1),
        });

        // Add linear gradient
        build_info.linear_gradients.push(LinearGradientEntry {
            id: EntityId(30),
            start: EntityId(10),
            end: EntityId(11),
            stops: vec![EntityId(20), EntityId(21)],
            tile_mode: TileMode::default(),
            target: EntityId(100),
        });

        // Add path entity with gradient fill
        build_info.path_entities.push(PathEntityEntry {
            id: EntityId(100),
            segments: vec![
                PathSegment::Line {
                    from: EntityId(1),
                    to: EntityId(2),
                },
                PathSegment::Line {
                    from: EntityId(2),
                    to: EntityId(3),
                },
                PathSegment::Line {
                    from: EntityId(3),
                    to: EntityId(4),
                },
                PathSegment::Line {
                    from: EntityId(4),
                    to: EntityId(1),
                },
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: Some(FillSpec::Gradient {
                gradient_id: EntityId(30),
            }),
            stroke: None,
        });

        // Build scene
        let builder = SceneBuilder::new(&solutions, &build_info);
        let scene = builder.build_scene().expect("build_scene failed");

        assert_eq!(scene.len(), 1);

        let node = match &scene[0] {
            SceneNode::Path(p) => p,
            _ => panic!("Expected Path node"),
        };

        // Verify gradient fill
        match &node.fill {
            Some(SceneFillStyle::LinearGradient { stops, start, end }) => {
                // Verify start/end points
                assert_eq!(start.0, Rational::from_int(0));
                assert_eq!(start.1, Rational::from_int(50));
                assert_eq!(end.0, Rational::from_int(100));
                assert_eq!(end.1, Rational::from_int(50));

                // Verify color stops
                assert_eq!(stops.len(), 2);
                assert_eq!(stops[0].position, Rational::from_int(0));
                assert!(stops[0].color.contains("255")); // red
                assert_eq!(stops[1].position, Rational::from_int(1));
                assert!(stops[1].color.contains("255")); // blue
            }
            other => panic!("Expected LinearGradient fill, got {:?}", other),
        }
    }

    #[test]
    fn test_derive_topo_constraints_shared_control_point() {
        // Create two paths that share a control point (common edge)
        let solutions = make_solutions(&[
            (1, 0, 0),     // Path A: top-left
            (2, 50, 0),    // Path A: top-right, Path B: top-left (SHARED)
            (3, 50, 100),  // Path A: bottom-right, Path B: bottom-left (SHARED)
            (4, 0, 100),   // Path A: bottom-left
            (5, 100, 0),   // Path B: top-right
            (6, 100, 100), // Path B: bottom-right
        ]);

        let mut build_info = VsBuildInfo::default();

        // Path A: rectangle using points 1, 2, 3, 4
        build_info.path_entities.push(PathEntityEntry {
            id: EntityId(100),
            segments: vec![
                PathSegment::Line {
                    from: EntityId(1),
                    to: EntityId(2),
                },
                PathSegment::Line {
                    from: EntityId(2),
                    to: EntityId(3),
                },
                PathSegment::Line {
                    from: EntityId(3),
                    to: EntityId(4),
                },
                PathSegment::Line {
                    from: EntityId(4),
                    to: EntityId(1),
                },
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: Some(FillSpec::Solid {
                color: "#ff0000".to_string(),
            }),
            stroke: None,
        });

        // Path B: rectangle using points 2, 5, 6, 3 (shares points 2 and 3 with Path A)
        build_info.path_entities.push(PathEntityEntry {
            id: EntityId(200),
            segments: vec![
                PathSegment::Line {
                    from: EntityId(2),
                    to: EntityId(5),
                },
                PathSegment::Line {
                    from: EntityId(5),
                    to: EntityId(6),
                },
                PathSegment::Line {
                    from: EntityId(6),
                    to: EntityId(3),
                },
                PathSegment::Line {
                    from: EntityId(3),
                    to: EntityId(2),
                },
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: Some(FillSpec::Solid {
                color: "#00ff00".to_string(),
            }),
            stroke: None,
        });

        let builder = SceneBuilder::new(&solutions, &build_info);
        let constraints = builder.derive_topo_constraints();

        // Should have constraints for shared control points 2 and 3
        // Each shared point generates 4 edge constraints (Left, Right, Top, Bottom)
        // between the two entities
        // 2 shared points × 4 edges = 8 topological constraints
        //
        // Additionally, derive_value_equality_constraints() produces constraints
        // for control point VarIds that share the same resolved Rational value
        // (D-19 topology-rounding).  Because X and Y coordinates are grouped by
        // raw Rational value (ignoring component), mixed-component pairs also
        // generate constraints.
        //
        // For these 6 control points the value groups are:
        //   value=0:   [E1.X, E1.Y, E2.Y, E4.X, E5.Y]  → 4 pairs  (base E1.X)
        //   value=50:  [E2.X, E3.X]                     → 1 pair
        //   value=100: [E3.Y, E4.Y, E5.X, E6.X, E6.Y]  → 4 pairs  (base E3.Y)
        // Value-equality subtotal = 9.
        //
        // Grand total = 8 (topo) + 9 (value-eq) = 17.
        assert_eq!(constraints.len(), 17);

        // Verify all constraints are Equal type
        for constraint in &constraints {
            assert!(
                matches!(constraint, TopoConstraint::Equal { .. }),
                "Expected Equal constraint, got {:?}",
                constraint
            );
        }
    }

    // =========================================================================
    // New boundary-condition tests
    // =========================================================================

    /// Task 1: MissingControlPoint error propagation test.
    ///
    /// A PathEntityEntry references an EntityId that is absent from the
    /// solutions map.  build_scene() must propagate the error as
    /// Err(SceneError::PathResolutionFailed { entity_id, .. }).
    #[test]
    fn test_build_scene_missing_control_point_propagates_error() {
        // Only EntityId(1) is in solutions; the path references EntityId(99) which is missing.
        let solutions = make_solutions(&[(1, 0, 0)]);

        let mut build_info = VsBuildInfo::default();
        build_info.path_entities.push(PathEntityEntry {
            id: EntityId(200),
            segments: vec![PathSegment::Line {
                from: EntityId(1),
                to: EntityId(99), // NOT in solutions
            }],
            closed: false,
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: None,
        });

        let builder = SceneBuilder::new(&solutions, &build_info);
        let result = builder.build_scene();

        assert!(
            result.is_err(),
            "Expected Err for missing control point, got Ok"
        );
        match result.unwrap_err() {
            SceneError::PathResolutionFailed { entity_id, .. } => {
                assert_eq!(entity_id, EntityId(200));
            }
            other => panic!("Expected PathResolutionFailed, got {:?}", other),
        }
    }

    /// Task 2: MissingGradient error test.
    ///
    /// A PathEntityEntry uses FillSpec::Gradient with gradient_id = EntityId(999),
    /// which is absent from both linear_gradients and radial_gradients.
    /// build_scene() must return Err(SceneError::MissingGradient { gradient_id: EntityId(999) }).
    #[test]
    fn test_build_scene_missing_gradient_returns_error() {
        use crate::types::FillSpec;

        let solutions = make_solutions(&[(1, 0, 0), (2, 100, 0), (3, 50, 100)]);

        let mut build_info = VsBuildInfo::default();
        // No gradient entries at all → gradient_id 999 is missing.
        build_info.path_entities.push(PathEntityEntry {
            id: EntityId(300),
            segments: vec![
                PathSegment::Line {
                    from: EntityId(1),
                    to: EntityId(2),
                },
                PathSegment::Line {
                    from: EntityId(2),
                    to: EntityId(3),
                },
                PathSegment::Line {
                    from: EntityId(3),
                    to: EntityId(1),
                },
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: Some(FillSpec::Gradient {
                gradient_id: EntityId(999),
            }),
            stroke: None,
        });

        let builder = SceneBuilder::new(&solutions, &build_info);
        let result = builder.build_scene();

        assert!(result.is_err(), "Expected Err for missing gradient, got Ok");
        match result.unwrap_err() {
            SceneError::MissingGradient { gradient_id } => {
                assert_eq!(gradient_id, EntityId(999));
            }
            other => panic!("Expected MissingGradient, got {:?}", other),
        }
    }

    /// Task 3: z_order documentation test.
    ///
    /// build_path_node() currently hard-codes z_order = 0.
    /// This test documents the current behaviour and will fail if
    /// z_order is changed without updating this assertion.
    #[test]
    fn test_build_scene_z_order_is_zero_hardcoded() {
        let solutions = make_solutions(&[(1, 0, 0), (2, 100, 0), (3, 50, 100)]);

        let mut build_info = VsBuildInfo::default();
        build_info.path_entities.push(PathEntityEntry {
            id: EntityId(400),
            segments: vec![
                PathSegment::Line {
                    from: EntityId(1),
                    to: EntityId(2),
                },
                PathSegment::Line {
                    from: EntityId(2),
                    to: EntityId(3),
                },
                PathSegment::Line {
                    from: EntityId(3),
                    to: EntityId(1),
                },
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: None,
        });

        let builder = SceneBuilder::new(&solutions, &build_info);
        let scene = builder.build_scene().expect("build_scene failed");

        assert_eq!(scene.len(), 1);
        // KNOWN ISSUE: PathEntityEntry has no z_order field; build_path_node()
        // hard-codes z_order: 0.  The test documents this current behaviour.
        assert_eq!(
            scene[0].z_order(),
            0,
            "z_order is currently hard-coded to 0 in build_path_node()"
        );
    }

    #[test]
    fn test_derive_topo_constraints_no_shared_points() {
        // Create two paths with no shared control points
        let solutions = make_solutions(&[
            (1, 0, 0),
            (2, 50, 0),
            (3, 50, 50),
            (4, 0, 50),
            // Second path (completely separate)
            (5, 100, 0),
            (6, 150, 0),
            (7, 150, 50),
            (8, 100, 50),
        ]);

        let mut build_info = VsBuildInfo::default();

        // Path A
        build_info.path_entities.push(PathEntityEntry {
            id: EntityId(100),
            segments: vec![
                PathSegment::Line {
                    from: EntityId(1),
                    to: EntityId(2),
                },
                PathSegment::Line {
                    from: EntityId(2),
                    to: EntityId(3),
                },
                PathSegment::Line {
                    from: EntityId(3),
                    to: EntityId(4),
                },
                PathSegment::Line {
                    from: EntityId(4),
                    to: EntityId(1),
                },
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: None,
        });

        // Path B (no shared points with A)
        build_info.path_entities.push(PathEntityEntry {
            id: EntityId(200),
            segments: vec![
                PathSegment::Line {
                    from: EntityId(5),
                    to: EntityId(6),
                },
                PathSegment::Line {
                    from: EntityId(6),
                    to: EntityId(7),
                },
                PathSegment::Line {
                    from: EntityId(7),
                    to: EntityId(8),
                },
                PathSegment::Line {
                    from: EntityId(8),
                    to: EntityId(5),
                },
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: None,
        });

        let builder = SceneBuilder::new(&solutions, &build_info);
        let constraints = builder.derive_topo_constraints();

        // No shared control points → no topological constraints from shared entities.
        // However, derive_value_equality_constraints() still generates constraints for
        // control point VarIds that share the same resolved Rational value (D-19).
        //
        // For these 8 control points the value groups are (X=Left, Y=Top, grouped by
        // raw value regardless of component):
        //   value=0:   [E1.X, E1.Y, E2.Y, E4.X, E5.Y, E6.Y]  → 5 pairs  (base E1.X)
        //   value=50:  [E2.X, E3.X, E3.Y, E4.Y, E7.Y, E8.Y]  → 5 pairs  (base E2.X)
        //   value=100: [E5.X, E8.X]                            → 1 pair
        //   value=150: [E6.X, E7.X]                            → 1 pair
        // Value-equality total = 12.
        assert_eq!(constraints.len(), 12);

        // Verify all constraints are Equal type
        for constraint in &constraints {
            assert!(
                matches!(constraint, TopoConstraint::Equal { .. }),
                "Expected Equal constraint, got {:?}",
                constraint
            );
        }
    }

    // =========================================================================
    // D-19: derive_value_equality_constraints tests
    // =========================================================================

    /// D-19 Test 1: Two independent points with the same X coordinate generate
    /// a value-equality constraint.
    ///
    /// EntityId(1).X = 50  and  EntityId(3).X = 50
    /// Expected: one Equal constraint between (EntityId(1), Edge::Left) and
    ///           (EntityId(3), Edge::Left).
    #[test]
    fn test_derive_value_equality_same_x_generates_constraint() {
        let mut solutions = HashMap::new();
        // Point 1: x=50, y=0
        solutions.insert(
            VarId::new(EntityId(1), VectorComponent::X),
            Rational::from_int(50),
        );
        solutions.insert(
            VarId::new(EntityId(1), VectorComponent::Y),
            Rational::from_int(0),
        );
        // Point 3: x=50 (same!), y=99
        solutions.insert(
            VarId::new(EntityId(3), VectorComponent::X),
            Rational::from_int(50),
        );
        solutions.insert(
            VarId::new(EntityId(3), VectorComponent::Y),
            Rational::from_int(99),
        );

        let constraints = derive_value_equality_constraints(&solutions);

        // X=50 group → 1 Equal constraint (Left edge)
        // Y=0 and Y=99 are distinct → no additional constraints
        assert_eq!(
            constraints.len(),
            1,
            "Expected exactly 1 Equal constraint for shared x=50, got {}",
            constraints.len()
        );

        match &constraints[0] {
            TopoConstraint::Equal { a, b } => {
                let (ea, edge_a) = a;
                let (eb, edge_b) = b;
                assert_eq!(*edge_a, Edge::Left, "Expected Edge::Left for X component");
                assert_eq!(*edge_b, Edge::Left, "Expected Edge::Left for X component");
                // Both entities must be 1 and 3 (order may vary)
                let ids = {
                    let mut v = vec![ea.0, eb.0];
                    v.sort_unstable();
                    v
                };
                assert_eq!(ids, vec![1, 3], "Expected EntityId(1) and EntityId(3)");
            }
            other => panic!("Expected Equal constraint, got {:?}", other),
        }
    }

    /// D-19 Test 2: All coordinates are distinct → no value-equality constraints.
    #[test]
    fn test_derive_value_equality_distinct_values_no_constraints() {
        let mut solutions = HashMap::new();
        solutions.insert(
            VarId::new(EntityId(1), VectorComponent::X),
            Rational::from_int(10),
        );
        solutions.insert(
            VarId::new(EntityId(1), VectorComponent::Y),
            Rational::from_int(20),
        );
        solutions.insert(
            VarId::new(EntityId(2), VectorComponent::X),
            Rational::from_int(30),
        );
        solutions.insert(
            VarId::new(EntityId(2), VectorComponent::Y),
            Rational::from_int(40),
        );

        let constraints = derive_value_equality_constraints(&solutions);

        assert!(
            constraints.is_empty(),
            "Expected no constraints for all-distinct values, got {:?}",
            constraints
        );
    }

    // =========================================================================
    // T1: evaluate_conditions Edge Detection Tests
    // =========================================================================

    /// Helper to create a condition for bounds overlap between two entities.
    fn make_overlap_condition(id: u64, entity_a: u64, entity_b: u64) -> PostSolveCondition {
        PostSolveCondition {
            id,
            kind: ConditionKind::BoundsOverlap {
                entity_a: EntityId(entity_a),
                entity_b: EntityId(entity_b),
            },
        }
    }

    /// T1.1: Condition transitions false→true: triggered ✓
    #[test]
    fn test_evaluate_conditions_false_to_true_triggers() {
        let build_info = VsBuildInfo::default();

        // Two overlapping points at (50, 50)
        let values = make_solutions(&[(1, 50, 50), (2, 50, 50)]);

        let conditions = vec![make_overlap_condition(100, 1, 2)];
        let prev_satisfied = HashSet::new(); // Previously not satisfied

        let (triggered, currently_satisfied) =
            evaluate_conditions(&conditions, &values, &build_info, &prev_satisfied);

        assert_eq!(triggered, vec![100], "Condition should trigger on false→true");
        assert!(
            currently_satisfied.contains(&100),
            "Condition should be in currently_satisfied"
        );
    }

    /// T1.2: Condition remains true→true: NOT triggered
    #[test]
    fn test_evaluate_conditions_true_to_true_no_trigger() {
        let build_info = VsBuildInfo::default();

        // Two overlapping points at (50, 50)
        let values = make_solutions(&[(1, 50, 50), (2, 50, 50)]);

        let conditions = vec![make_overlap_condition(100, 1, 2)];
        let mut prev_satisfied = HashSet::new();
        prev_satisfied.insert(100); // Was already satisfied

        let (triggered, currently_satisfied) =
            evaluate_conditions(&conditions, &values, &build_info, &prev_satisfied);

        assert!(
            triggered.is_empty(),
            "Condition should NOT trigger on true→true (no edge)"
        );
        assert!(
            currently_satisfied.contains(&100),
            "Condition should still be in currently_satisfied"
        );
    }

    /// T1.3: Condition transitions true→false→true: triggered on second rising edge
    #[test]
    fn test_evaluate_conditions_true_false_true_triggers() {
        let build_info = VsBuildInfo::default();
        let conditions = vec![make_overlap_condition(100, 1, 2)];

        // Frame 1: false→true (initial trigger)
        let values_overlap = make_solutions(&[(1, 50, 50), (2, 50, 50)]);
        let prev_satisfied_f1 = HashSet::new();
        let (triggered_f1, satisfied_f1) =
            evaluate_conditions(&conditions, &values_overlap, &build_info, &prev_satisfied_f1);
        assert_eq!(triggered_f1, vec![100], "Frame 1: should trigger");

        // Frame 2: true→false (no trigger, condition no longer met)
        let values_no_overlap = make_solutions(&[(1, 0, 0), (2, 100, 100)]);
        let (triggered_f2, satisfied_f2) =
            evaluate_conditions(&conditions, &values_no_overlap, &build_info, &satisfied_f1);
        assert!(triggered_f2.is_empty(), "Frame 2: should not trigger");
        assert!(
            !satisfied_f2.contains(&100),
            "Frame 2: condition should not be satisfied"
        );

        // Frame 3: false→true (re-trigger)
        let (triggered_f3, satisfied_f3) =
            evaluate_conditions(&conditions, &values_overlap, &build_info, &satisfied_f2);
        assert_eq!(triggered_f3, vec![100], "Frame 3: should trigger again");
        assert!(
            satisfied_f3.contains(&100),
            "Frame 3: condition should be satisfied"
        );
    }

    /// T1.4: Unregistered conditions are ignored
    #[test]
    fn test_evaluate_conditions_empty_conditions() {
        let build_info = VsBuildInfo::default();
        let values = make_solutions(&[(1, 50, 50), (2, 50, 50)]);
        let conditions: Vec<PostSolveCondition> = vec![]; // No conditions registered

        let mut prev_satisfied = HashSet::new();
        prev_satisfied.insert(999); // Stale condition ID

        let (triggered, currently_satisfied) =
            evaluate_conditions(&conditions, &values, &build_info, &prev_satisfied);

        assert!(triggered.is_empty(), "No conditions to trigger");
        assert!(
            currently_satisfied.is_empty(),
            "No conditions should be satisfied"
        );
    }

    /// T1.5: Missing entity coordinates result in false (no panic)
    #[test]
    fn test_evaluate_conditions_missing_coordinates_no_panic() {
        let build_info = VsBuildInfo::default();

        // Only entity 1 has coordinates, entity 2 is missing
        let values = make_solutions(&[(1, 50, 50)]);

        let conditions = vec![make_overlap_condition(100, 1, 2)];
        let prev_satisfied = HashSet::new();

        // Should NOT panic, should return false for the condition
        let (triggered, currently_satisfied) =
            evaluate_conditions(&conditions, &values, &build_info, &prev_satisfied);

        assert!(
            triggered.is_empty(),
            "Condition with missing coords should not trigger"
        );
        assert!(
            !currently_satisfied.contains(&100),
            "Condition with missing coords should not be satisfied"
        );
    }

    /// T1.6: Multiple conditions with mixed states
    #[test]
    fn test_evaluate_conditions_multiple_mixed() {
        let build_info = VsBuildInfo::default();

        // Entity layout:
        // - Entities 1,2 overlap at (50,50)
        // - Entities 3,4 do NOT overlap (3 at origin, 4 at 100,100)
        let values = make_solutions(&[(1, 50, 50), (2, 50, 50), (3, 0, 0), (4, 100, 100)]);

        let conditions = vec![
            make_overlap_condition(100, 1, 2), // Will be satisfied
            make_overlap_condition(101, 3, 4), // Will NOT be satisfied
        ];

        // Condition 101 was previously satisfied (simulating it went true→false)
        let mut prev_satisfied = HashSet::new();
        prev_satisfied.insert(101);

        let (triggered, currently_satisfied) =
            evaluate_conditions(&conditions, &values, &build_info, &prev_satisfied);

        // Only condition 100 should trigger (false→true)
        assert_eq!(triggered, vec![100], "Only new overlap should trigger");
        assert!(currently_satisfied.contains(&100));
        assert!(!currently_satisfied.contains(&101));
    }
}
