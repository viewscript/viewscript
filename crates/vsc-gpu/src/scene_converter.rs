//! Scene to Canvas Node Conversion (Phase G-1)
//!
//! Converts `vsc-core::SceneNode` to `vsc-gpu::CanvasNode` for GPU rendering.
//!
//! ## Architecture
//!
//! ```text
//! vsc-core::SceneNode  ──────►  vsc-gpu::CanvasNode
//!     │                              │
//!     ├─ ScenePathNode       ──────► CanvasPathNode
//!     ├─ SceneFillStyle      ──────► FillStyle
//!     ├─ SceneStrokeStyle    ──────► StrokeStyle
//!     └─ SceneBounds         ──────► PVectorBounds
//! ```
//!
//! ## ChunkId Assignment
//!
//! Initial implementation uses simple incremental IDs. Spatial partitioning
//! optimization is deferred to a later phase.

use std::collections::HashMap;
use vsc_core::scene::{
    SceneBounds, SceneFillStyle, SceneGradientStop, SceneGroupNode, SceneNode, ScenePathNode,
    SceneStrokeStyle,
};
use vsc_core::{EntityId, Rational, UvTransform};

use crate::rasterizer::{round_with_topology_preservation, RoundingResult, TopoConstraint};
use crate::shaders::hex_to_rgba;
use crate::{
    CanvasGroupNode, CanvasNode, CanvasNodeBase, CanvasPathNode, ChunkId, FillStyle, GradientPoint,
    GradientStop, PVector, PVectorBounds, RasterBounds, StrokeStyle,
};

// =============================================================================
// Scene Converter
// =============================================================================

/// Converts scene nodes to canvas nodes for GPU rendering.
///
/// ## Usage
///
/// ```ignore
/// use vsc_gpu::SceneConverter;
/// use vsc_core::scene::SceneNode;
///
/// let scene_nodes: Vec<SceneNode> = /* from SceneBuilder */;
/// let mut converter = SceneConverter::new();
/// let canvas_nodes = converter.convert(&scene_nodes);
/// // canvas_nodes ready for GpuRenderer
/// ```
pub struct SceneConverter {
    chunk_id_counter: u32,
}

impl SceneConverter {
    /// Create a new scene converter.
    pub fn new() -> Self {
        Self {
            chunk_id_counter: 0,
        }
    }

    /// Convert a list of scene nodes to canvas nodes (without topology rounding).
    ///
    /// Use this for tests, offscreen rendering, or when topology preservation
    /// is not required.
    ///
    /// Takes a slice reference to avoid deep copying the entire scene graph.
    pub fn convert(&mut self, scene_nodes: &[SceneNode]) -> Vec<CanvasNode> {
        scene_nodes
            .iter()
            .map(|node| self.convert_node(node))
            .collect()
    }

    /// Convert scene nodes to canvas nodes with topology-preserving rounding.
    ///
    /// This method ensures that adjacent surfaces maintain their topological
    /// relationships (no 1px gaps) after rasterization.
    ///
    /// ## Arguments
    ///
    /// * `scene_nodes` - Scene nodes from SceneBuilder
    /// * `topo_constraints` - Topological constraints (adjacency, containment)
    /// * `device_pixel_ratio` - DPR for coordinate scaling (e.g., 2.0 for Retina)
    ///
    /// ## Returns
    ///
    /// Tuple of (canvas_nodes, rounding_result):
    /// - `canvas_nodes`: Converted nodes with rounded bounds
    /// - `rounding_result`: Contains rasterized bounds and any violations
    ///
    /// ## Flow
    ///
    /// 1. Convert scene_nodes to canvas_nodes (Rational coordinates)
    /// 2. Collect all PVectorBounds into HashMap<EntityId, PVectorBounds>
    /// 3. Call round_with_topology_preservation()
    /// 4. Update canvas_nodes' bounds with rounded integer coordinates
    pub fn convert_with_rounding(
        &mut self,
        scene_nodes: &[SceneNode],
        topo_constraints: &[TopoConstraint],
        device_pixel_ratio: f64,
    ) -> (Vec<CanvasNode>, RoundingResult) {
        // Step 1: Convert to canvas nodes with Rational coordinates
        let mut canvas_nodes = self.convert(scene_nodes);

        // Step 2: Collect all PVectorBounds
        let entities = Self::collect_bounds(&canvas_nodes);

        // Step 3: Run topology-preserving rounding
        let rounding_result =
            round_with_topology_preservation(&entities, topo_constraints, device_pixel_ratio);

        // Step 4: Update canvas nodes with rounded bounds
        Self::apply_rounded_bounds(
            &mut canvas_nodes,
            &rounding_result.bounds,
            device_pixel_ratio,
        );

        // Step 5: Warn if topology violations were detected
        if !rounding_result.violations.is_empty() {
            log::warn!(
                "[vsc-gpu::SceneConverter] {} topology violation(s) after rounding: {:?}",
                rounding_result.violations.len(),
                rounding_result.violations
            );
        }

        (canvas_nodes, rounding_result)
    }

    /// Collect PVectorBounds from all canvas nodes into a HashMap.
    fn collect_bounds(nodes: &[CanvasNode]) -> HashMap<EntityId, PVectorBounds> {
        let mut bounds = HashMap::new();
        Self::collect_bounds_recursive(nodes, &mut bounds);
        bounds
    }

    fn collect_bounds_recursive(
        nodes: &[CanvasNode],
        bounds: &mut HashMap<EntityId, PVectorBounds>,
    ) {
        for node in nodes {
            match node {
                CanvasNode::Path(path) => {
                    bounds.insert(path.base.entity_id, path.base.bounds.clone());
                }
                CanvasNode::Group(group) => {
                    bounds.insert(group.base.entity_id, group.base.bounds.clone());
                    Self::collect_bounds_recursive(&group.children, bounds);
                }
                CanvasNode::Text(text) => {
                    bounds.insert(text.base.entity_id, text.base.bounds.clone());
                }
                CanvasNode::Image(image) => {
                    bounds.insert(image.base.entity_id, image.base.bounds.clone());
                }
            }
        }
    }

    /// Apply rounded bounds back to canvas nodes.
    ///
    /// Converts RasterBounds (f64 pixels) back to PVectorBounds (Rational)
    /// using integer Rationals to preserve exact pixel values during tessellation.
    fn apply_rounded_bounds(
        nodes: &mut [CanvasNode],
        rounded: &HashMap<EntityId, RasterBounds>,
        device_pixel_ratio: f64,
    ) {
        for node in nodes {
            match node {
                CanvasNode::Path(path) => {
                    if let Some(raster) = rounded.get(&path.base.entity_id) {
                        path.base.bounds =
                            Self::raster_to_pvector_bounds(raster, device_pixel_ratio);
                    }
                }
                CanvasNode::Group(group) => {
                    if let Some(raster) = rounded.get(&group.base.entity_id) {
                        group.base.bounds =
                            Self::raster_to_pvector_bounds(raster, device_pixel_ratio);
                    }
                    Self::apply_rounded_bounds(&mut group.children, rounded, device_pixel_ratio);
                }
                CanvasNode::Text(text) => {
                    if let Some(raster) = rounded.get(&text.base.entity_id) {
                        text.base.bounds =
                            Self::raster_to_pvector_bounds(raster, device_pixel_ratio);
                    }
                }
                CanvasNode::Image(image) => {
                    if let Some(raster) = rounded.get(&image.base.entity_id) {
                        image.base.bounds =
                            Self::raster_to_pvector_bounds(raster, device_pixel_ratio);
                    }
                }
            }
        }
    }

    /// Convert RasterBounds (f64 CSS pixels) to PVectorBounds (Rational).
    ///
    /// Uses integer Rationals scaled by DPR to preserve exact pixel alignment.
    fn raster_to_pvector_bounds(raster: &RasterBounds, device_pixel_ratio: f64) -> PVectorBounds {
        // Convert CSS pixels to device pixels, then to integer Rationals
        let x = (raster.x * device_pixel_ratio).round() as i64;
        let y = (raster.y * device_pixel_ratio).round() as i64;
        let right = ((raster.x + raster.width) * device_pixel_ratio).round() as i64;
        let bottom = ((raster.y + raster.height) * device_pixel_ratio).round() as i64;

        // Store as Rational with DPR denominator for exact representation
        let dpr_denom = (device_pixel_ratio * 1000.0).round() as i64;
        let scale = 1000; // Keep precision

        PVectorBounds {
            top_left: PVector {
                x: Rational::new(x * scale, dpr_denom),
                y: Rational::new(y * scale, dpr_denom),
                z: Rational::zero(),
                t: Rational::zero(),
            },
            bottom_right: PVector {
                x: Rational::new(right * scale, dpr_denom),
                y: Rational::new(bottom * scale, dpr_denom),
                z: Rational::zero(),
                t: Rational::zero(),
            },
        }
    }

    /// Convert a single scene node to a canvas node.
    fn convert_node(&mut self, node: &SceneNode) -> CanvasNode {
        match node {
            SceneNode::Path(path) => CanvasNode::Path(self.convert_path_node(path)),
            SceneNode::Group(group) => CanvasNode::Group(self.convert_group_node(group)),
        }
    }

    /// Convert a scene path node to a canvas path node.
    fn convert_path_node(&mut self, path: &ScenePathNode) -> CanvasPathNode {
        let chunk_id = self.allocate_chunk_id();

        CanvasPathNode {
            base: CanvasNodeBase {
                entity_id: path.entity_id,
                bounds: Self::convert_bounds(&path.bounds),
                z_order: path.z_order,
                chunk_id,
            },
            path_data: path.path_data.clone(),
            fill: path.fill.clone().map(Self::convert_fill_style),
            stroke: path.stroke.clone().map(Self::convert_stroke_style),
        }
    }

    /// Convert a scene group node to a canvas group node.
    fn convert_group_node(&mut self, group: &SceneGroupNode) -> CanvasGroupNode {
        let chunk_id = self.allocate_chunk_id();

        // Recursively convert children
        let children: Vec<CanvasNode> = group
            .children
            .iter()
            .map(|child| self.convert_node(child))
            .collect();

        CanvasGroupNode {
            base: CanvasNodeBase {
                entity_id: group.entity_id,
                bounds: Self::convert_bounds(&group.bounds),
                z_order: group.z_order,
                chunk_id,
            },
            children,
            // Rasterization boundary: convert Rational translate to f64
            transform: crate::AffineTransform::translation(
                group.translate.0.to_f64_for_rasterization(),
                group.translate.1.to_f64_for_rasterization(),
            ),
            clip_path: None,
            opacity: 1.0,
        }
    }

    /// Allocate a new chunk ID (simple incremental for now).
    ///
    /// Uses `wrapping_add` so that overflow at u32::MAX wraps to 0
    /// instead of panicking in debug or silently overflowing in release.
    fn allocate_chunk_id(&mut self) -> ChunkId {
        let id = format!("chunk_{}", self.chunk_id_counter);
        self.chunk_id_counter = self.chunk_id_counter.wrapping_add(1);
        id
    }

    /// Convert SceneBounds to PVectorBounds.
    fn convert_bounds(bounds: &SceneBounds) -> PVectorBounds {
        PVectorBounds {
            top_left: PVector {
                x: bounds.x_min.clone(),
                y: bounds.y_min.clone(),
                z: Rational::zero(),
                t: Rational::zero(),
            },
            bottom_right: PVector {
                x: bounds.x_max.clone(),
                y: bounds.y_max.clone(),
                z: Rational::zero(),
                t: Rational::zero(),
            },
        }
    }

    /// Convert SceneFillStyle to FillStyle.
    ///
    /// Parses hex color strings to [u8; 4] RGBA at this boundary,
    /// eliminating String clones in the hot path.
    fn convert_fill_style(fill: SceneFillStyle) -> FillStyle {
        match fill {
            SceneFillStyle::Solid { color } => FillStyle::Solid {
                rgba: hex_to_rgba(&color).unwrap_or([0, 0, 0, 255]),
            },

            SceneFillStyle::LinearGradient { stops, start, end } => FillStyle::LinearGradient {
                stops: stops.into_iter().map(Self::convert_gradient_stop).collect(),
                start: Some(GradientPoint {
                    x: start.0,
                    y: start.1,
                }),
                end: Some(GradientPoint { x: end.0, y: end.1 }),
            },

            SceneFillStyle::RadialGradient {
                stops,
                center,
                radius_x,
                radius_y: _, // FillStyle only has single radius
            } => FillStyle::RadialGradient {
                stops: stops.into_iter().map(Self::convert_gradient_stop).collect(),
                center: Some(GradientPoint {
                    x: center.0,
                    y: center.1,
                }),
                radius: Some(radius_x),
            },

            SceneFillStyle::ExternalTexture {
                handle_name,
                width: _,
                height: _,
                uv_transform,
            } => {
                // Parse texture_id from handle_name (format: "resource.texture.<id>")
                let texture_id = handle_name
                    .strip_prefix("resource.texture.")
                    .and_then(|id_str| id_str.parse::<u64>().ok())
                    .unwrap_or_else(|| {
                        log::warn!(
                            "Invalid external texture handle_name: '{}', using fallback id 0",
                            handle_name
                        );
                        0
                    });

                FillStyle::ExternalTexture {
                    texture_id,
                    uv_transform: uv_transform.clone(),
                }
            }
        }
    }

    /// Convert SceneGradientStop to GradientStop.
    fn convert_gradient_stop(stop: SceneGradientStop) -> GradientStop {
        GradientStop {
            offset: stop.position,
            rgba: hex_to_rgba(&stop.color).unwrap_or([0, 0, 0, 255]),
        }
    }

    /// Convert SceneStrokeStyle to StrokeStyle.
    fn convert_stroke_style(stroke: SceneStrokeStyle) -> StrokeStyle {
        StrokeStyle {
            rgba: hex_to_rgba(&stroke.color).unwrap_or([0, 0, 0, 255]),
            width: stroke.width,
            line_cap: stroke.line_cap,
            line_join: stroke.line_join,
            dash_array: stroke.dash_array,
        }
    }
}

impl Default for SceneConverter {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use vsc_core::scene::{SceneBounds, SceneFillStyle, ScenePathNode};
    use vsc_core::types::{EntityId, FillRule, PathCommand};

    #[test]
    fn test_convert_triangle_with_solid_fill() {
        // Create a triangle path node
        let scene_path = ScenePathNode {
            entity_id: EntityId(100),
            z_order: 0,
            bounds: SceneBounds::new(
                Rational::from_int(0),
                Rational::from_int(0),
                Rational::from_int(100),
                Rational::from_int(100),
            ),
            path_data: vec![
                PathCommand::MoveTo {
                    x: Rational::from_int(0),
                    y: Rational::from_int(0),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(100),
                    y: Rational::from_int(0),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(50),
                    y: Rational::from_int(100),
                },
                PathCommand::Close,
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: Some(SceneFillStyle::Solid {
                color: "#ff0000".to_string(),
            }),
            stroke: None,
        };

        let scene_nodes = vec![SceneNode::Path(scene_path)];

        // Convert
        let mut converter = SceneConverter::new();
        let canvas_nodes = converter.convert(&scene_nodes);

        // Verify
        assert_eq!(canvas_nodes.len(), 1);

        let canvas_path = match &canvas_nodes[0] {
            CanvasNode::Path(p) => p,
            _ => panic!("Expected Path node"),
        };

        // Verify entity_id
        assert_eq!(canvas_path.base.entity_id, EntityId(100));

        // Verify z_order
        assert_eq!(canvas_path.base.z_order, 0);

        // Verify bounds
        assert_eq!(canvas_path.base.bounds.top_left.x, Rational::from_int(0));
        assert_eq!(canvas_path.base.bounds.top_left.y, Rational::from_int(0));
        assert_eq!(
            canvas_path.base.bounds.bottom_right.x,
            Rational::from_int(100)
        );
        assert_eq!(
            canvas_path.base.bounds.bottom_right.y,
            Rational::from_int(100)
        );

        // Verify chunk_id is assigned
        assert!(canvas_path.base.chunk_id.starts_with("chunk_"));

        // Verify path_data (should be identical)
        assert_eq!(canvas_path.path_data.len(), 4);
        assert!(matches!(
            canvas_path.path_data[0],
            PathCommand::MoveTo { .. }
        ));
        assert!(matches!(
            canvas_path.path_data[1],
            PathCommand::LineTo { .. }
        ));
        assert!(matches!(
            canvas_path.path_data[2],
            PathCommand::LineTo { .. }
        ));
        assert!(matches!(canvas_path.path_data[3], PathCommand::Close));

        // Verify fill
        match &canvas_path.fill {
            Some(FillStyle::Solid { rgba }) => {
                assert_eq!(*rgba, [255, 0, 0, 255]);
            }
            other => panic!("Expected Solid fill, got {:?}", other),
        }

        // Verify stroke is None
        assert!(canvas_path.stroke.is_none());
    }

    #[test]
    fn test_convert_with_linear_gradient() {
        use vsc_core::scene::SceneGradientStop;

        let scene_path = ScenePathNode {
            entity_id: EntityId(200),
            z_order: 1,
            bounds: SceneBounds::new(
                Rational::from_int(0),
                Rational::from_int(0),
                Rational::from_int(200),
                Rational::from_int(100),
            ),
            path_data: vec![
                PathCommand::MoveTo {
                    x: Rational::from_int(0),
                    y: Rational::from_int(0),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(200),
                    y: Rational::from_int(100),
                },
                PathCommand::Close,
            ],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: Some(SceneFillStyle::LinearGradient {
                stops: vec![
                    SceneGradientStop {
                        position: Rational::from_int(0),
                        color: "rgba(255, 0, 0, 1)".to_string(),
                    },
                    SceneGradientStop {
                        position: Rational::from_int(1),
                        color: "rgba(0, 0, 255, 1)".to_string(),
                    },
                ],
                start: (Rational::from_int(0), Rational::from_int(50)),
                end: (Rational::from_int(200), Rational::from_int(50)),
            }),
            stroke: None,
        };

        let mut converter = SceneConverter::new();
        let scene_nodes = vec![SceneNode::Path(scene_path)];
        let canvas_nodes = converter.convert(&scene_nodes);

        let canvas_path = match &canvas_nodes[0] {
            CanvasNode::Path(p) => p,
            _ => panic!("Expected Path node"),
        };

        // Verify linear gradient conversion
        match &canvas_path.fill {
            Some(FillStyle::LinearGradient { stops, start, end }) => {
                // Verify stops
                assert_eq!(stops.len(), 2);
                assert_eq!(stops[0].offset, Rational::from_int(0));
                assert_eq!(stops[1].offset, Rational::from_int(1));

                // Verify start/end points
                let start = start.as_ref().expect("start should be Some");
                let end = end.as_ref().expect("end should be Some");

                assert_eq!(start.x, Rational::from_int(0));
                assert_eq!(start.y, Rational::from_int(50));
                assert_eq!(end.x, Rational::from_int(200));
                assert_eq!(end.y, Rational::from_int(50));
            }
            other => panic!("Expected LinearGradient fill, got {:?}", other),
        }
    }

    #[test]
    fn test_chunk_id_increments() {
        let scene_nodes = vec![
            SceneNode::Path(ScenePathNode {
                entity_id: EntityId(1),
                z_order: 0,
                bounds: SceneBounds::empty(),
                path_data: vec![],
                closed: false,
                fill_rule: FillRule::NonZero,
                fill: None,
                stroke: None,
            }),
            SceneNode::Path(ScenePathNode {
                entity_id: EntityId(2),
                z_order: 0,
                bounds: SceneBounds::empty(),
                path_data: vec![],
                closed: false,
                fill_rule: FillRule::NonZero,
                fill: None,
                stroke: None,
            }),
        ];

        let mut converter = SceneConverter::new();
        let canvas_nodes = converter.convert(&scene_nodes);

        // Verify chunk IDs are different
        let chunk_id_1 = match &canvas_nodes[0] {
            CanvasNode::Path(p) => &p.base.chunk_id,
            _ => panic!("Expected Path"),
        };
        let chunk_id_2 = match &canvas_nodes[1] {
            CanvasNode::Path(p) => &p.base.chunk_id,
            _ => panic!("Expected Path"),
        };

        assert_ne!(chunk_id_1, chunk_id_2);
        assert_eq!(chunk_id_1, "chunk_0");
        assert_eq!(chunk_id_2, "chunk_1");
    }

    /// Task 1: ChunkId wraps around at u32::MAX without panic.
    #[test]
    fn test_chunk_id_wraps_at_u32_max() {
        let mut converter = SceneConverter {
            chunk_id_counter: u32::MAX,
        };

        // Allocate at u32::MAX
        let id_max = converter.allocate_chunk_id();
        assert_eq!(id_max, format!("chunk_{}", u32::MAX));

        // Next allocation must wrap to 0
        let id_wrapped = converter.allocate_chunk_id();
        assert_eq!(id_wrapped, "chunk_0");
    }

    /// Task 2: Violations non-empty causes warning output and does not panic.
    /// Test that topology violations are detected when a LessThan constraint
    /// cannot be satisfied because both coordinates are in the same equivalence class.
    ///
    /// Setup: A.right = B.left = 100 (same value → same equivalence class)
    /// Constraint: A.right < B.left (impossible: X < X)
    ///
    /// Since they're in the same class, the algorithm can't make one less than the other.
    #[test]
    fn test_convert_with_rounding_violations_trigger_warning() {
        use crate::rasterizer::{Edge, TopoConstraint};

        let scene_a = ScenePathNode {
            entity_id: EntityId(10),
            z_order: 0,
            bounds: SceneBounds::new(
                Rational::from_int(0),
                Rational::from_int(0),
                Rational::from_int(100), // A.right = 100
                Rational::from_int(50),
            ),
            path_data: vec![],
            closed: false,
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: None,
        };

        let scene_b = ScenePathNode {
            entity_id: EntityId(20),
            z_order: 0,
            bounds: SceneBounds::new(
                Rational::from_int(100), // B.left = 100 (same as A.right → same class)
                Rational::from_int(0),
                Rational::from_int(200),
                Rational::from_int(50),
            ),
            path_data: vec![],
            closed: false,
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: None,
        };

        let scene_nodes = vec![SceneNode::Path(scene_a), SceneNode::Path(scene_b)];

        // LessThan: A.right < B.left where both have value 100
        // They're in the same equivalence class, so this can't be satisfied
        let constraints = vec![TopoConstraint::LessThan {
            a: (EntityId(10), Edge::Right),
            b: (EntityId(20), Edge::Left),
        }];

        let mut converter = SceneConverter::new();
        let (_canvas_nodes, rounding_result) =
            converter.convert_with_rounding(&scene_nodes, &constraints, 1.0);

        // Since A.right and B.left are in the same equivalence class (both = 100),
        // the LessThan constraint X < X is impossible to satisfy
        assert!(
            !rounding_result.violations.is_empty(),
            "Expected LessThan violation: A.right and B.left are in same class (both = 100)"
        );
    }

    #[test]
    fn test_convert_with_rounding_adjacency_preserved() {
        use crate::rasterizer::{Edge, TopoConstraint};

        // Two adjacent surfaces: A.right = B.left = 100.333...
        let scene_a = ScenePathNode {
            entity_id: EntityId(1),
            z_order: 0,
            bounds: SceneBounds::new(
                Rational::from_int(0),
                Rational::from_int(0),
                Rational::new(301, 3), // 100.333...
                Rational::from_int(50),
            ),
            path_data: vec![],
            closed: false,
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: None,
        };

        let scene_b = ScenePathNode {
            entity_id: EntityId(2),
            z_order: 0,
            bounds: SceneBounds::new(
                Rational::new(301, 3), // 100.333...
                Rational::from_int(0),
                Rational::from_int(200),
                Rational::from_int(50),
            ),
            path_data: vec![],
            closed: false,
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: None,
        };

        let scene_nodes = vec![SceneNode::Path(scene_a), SceneNode::Path(scene_b)];

        // Adjacency constraint: A.right touches B.left
        let constraints = vec![TopoConstraint::Adjacent {
            a: (EntityId(1), Edge::Right),
            b: (EntityId(2), Edge::Left),
        }];

        let mut converter = SceneConverter::new();
        let (canvas_nodes, rounding_result) =
            converter.convert_with_rounding(&scene_nodes, &constraints, 1.0);

        // Should have no topology violations
        assert!(
            rounding_result.violations.is_empty(),
            "Violations: {:?}",
            rounding_result.violations
        );

        // Verify adjacency is preserved in raster bounds
        let raster_a = rounding_result.bounds.get(&EntityId(1)).unwrap();
        let raster_b = rounding_result.bounds.get(&EntityId(2)).unwrap();
        let a_right = raster_a.x + raster_a.width;
        let b_left = raster_b.x;

        assert!(
            (a_right - b_left).abs() < 1e-9,
            "Adjacency violated: A.right={}, B.left={}",
            a_right,
            b_left
        );

        // Verify canvas_nodes were updated
        assert_eq!(canvas_nodes.len(), 2);
    }
}
