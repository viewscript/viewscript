//! ViewScript GPU Renderer (wgpu backend)
//!
//! This crate implements the GPU rasterization layer for ViewScript (wgpu backend).
//! It receives `CanvasNode` trees from the TypeScript renderer and produces
//! GPU draw calls via wgpu.
//!
//! ## Architecture
//!
//! ```text
//! TypeScript (render-loop.ts)          WASM Boundary          Rust (vsc-gpu)
//! ─────────────────────────────────────────────────────────────────────────────
//!                                           │
//! CanvasNode (JSON) ────────────────────────┼───────────▶ CanvasNode (Rust)
//!                                           │                    │
//!                                           │                    ▼
//!                                           │            lyon tessellation
//!                                           │                    │
//!                                           │                    ▼
//!                                           │            wgpu render pipeline
//!                                           │                    │
//!                                           │                    ▼
//! ◀──────────────────────────────────────────────────── GPU surface
//! ```
//!
//! ## Module Structure
//!
//! - `types` (this file): CanvasNode hierarchy mirroring TypeScript AST
//! - `tessellation`: lyon-based path tessellation (Rational → vertex buffers)
//! - `rasterizer`: Topology-preserving rounding (future: Union-Find + LRM)
//!
//! ## Type Correspondence
//!
//! | TypeScript (ast/types.ts) | Rust (this module)       |
//! |---------------------------|--------------------------|
//! | `CanvasNode`              | `CanvasNode`             |
//! | `CanvasPathNode`          | `CanvasNode::Path { }`   |
//! | `PathCommand`             | `PathCommand`            |
//! | `FillStyle`               | `FillStyle`              |
//! | `StrokeStyle`             | `StrokeStyle`            |
//! | `AffineTransform`         | `AffineTransform`        |

pub mod batcher;
pub mod loop_blinn;
pub mod opacity;
pub mod pipeline;
pub mod rasterizer;
pub mod renderer;
pub mod scene_converter;
pub mod sdf_stroke;
pub mod shaders;
pub mod stencil;
pub mod tessellation;
pub mod transform;
pub mod web_target;

// Re-export core types
pub use batcher::{DrawBatch, DrawBatcher, GpuBatchResources, PipelineKey, UniformData};
pub use loop_blinn::{compute_curve_sign, tessellate_quadratic_beziers, LoopBlinnOutput, LoopBlinnVertex};
pub use sdf_stroke::{tessellate_stroke_segments, SdfStrokeOutput, SdfStrokeVertex};
pub use opacity::OpacityStack;
pub use pipeline::{PipelineManager, PipelineSet};
pub use renderer::GpuRenderer;
pub use scene_converter::SceneConverter;
pub use stencil::StencilStack;
pub use web_target::WebTarget;
pub use tessellation::{BoundingBox, GpuVertex, TessellationError, TessellationOutput};

// Re-export rasterizer types for topology-preserving rounding
pub use rasterizer::{
    round_with_topology_preservation, CoordRef, Edge, RoundingResult, RoundingStats,
    TopoConstraint, TopologyViolation,
};

use serde::{Deserialize, Serialize};
use vsc_core::{EntityId, Rational, UvTransform};

// Re-export path types from vsc-core (canonical definitions)
pub use vsc_core::{LineCap, LineJoin, PathCommand};

// =============================================================================
// Coordinate Types
// =============================================================================

/// P-dimension bounds (pre-rasterization, exact rational).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PVectorBounds {
    pub top_left: PVector,
    pub bottom_right: PVector,
}

/// P-dimension vector with exact rational coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PVector {
    pub x: Rational,
    pub y: Rational,
    pub z: Rational,
    pub t: Rational,
}

impl PVector {
    /// Create a zero vector.
    pub fn zero() -> Self {
        Self {
            x: Rational::zero(),
            y: Rational::zero(),
            z: Rational::zero(),
            t: Rational::zero(),
        }
    }
}

/// Rasterized bounds (post-rasterization, device pixels).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RasterBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Chunk identifier for progressive loading.
pub type ChunkId = String;

// =============================================================================
// Canvas Node Hierarchy
// =============================================================================

/// A node in the visual (Canvas) layer of the bilayer architecture.
///
/// This enum mirrors the TypeScript `CanvasNode` union type.
/// Each variant contains the common fields plus type-specific data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CanvasNode {
    /// Path-based shape (curves, polygons, etc.)
    Path(CanvasPathNode),

    /// Text rendering node.
    Text(CanvasTextNode),

    /// Image rendering node.
    Image(CanvasImageNode),

    /// Group node with transform and children.
    Group(CanvasGroupNode),
}

impl CanvasNode {
    /// Get the entity ID for this node.
    pub fn entity_id(&self) -> EntityId {
        match self {
            CanvasNode::Path(n) => n.base.entity_id,
            CanvasNode::Text(n) => n.base.entity_id,
            CanvasNode::Image(n) => n.base.entity_id,
            CanvasNode::Group(n) => n.base.entity_id,
        }
    }

    /// Get the z-order for painter's algorithm sorting.
    pub fn z_order(&self) -> i32 {
        match self {
            CanvasNode::Path(n) => n.base.z_order,
            CanvasNode::Text(n) => n.base.z_order,
            CanvasNode::Image(n) => n.base.z_order,
            CanvasNode::Group(n) => n.base.z_order,
        }
    }

    /// Get the P-dimension bounds for this node.
    pub fn bounds(&self) -> &PVectorBounds {
        match self {
            CanvasNode::Path(n) => &n.base.bounds,
            CanvasNode::Text(n) => &n.base.bounds,
            CanvasNode::Image(n) => &n.base.bounds,
            CanvasNode::Group(n) => &n.base.bounds,
        }
    }
}

/// Common fields for all canvas nodes.
///
/// ## Axiom 3 Compliance
///
/// This struct contains ONLY P-dimension data (Rational coordinates).
/// `raster_bounds` has been intentionally removed from the public API.
/// Rasterization (Rational → f64 → vertex buffer) occurs internally
/// within the `vsc-gpu` tessellation pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasNodeBase {
    /// Back-reference to logical entity.
    pub entity_id: EntityId,

    /// Pre-rasterization coordinates (P-dimension, exact Rational).
    pub bounds: PVectorBounds,

    /// Z-order for painter's algorithm.
    pub z_order: i32,

    /// Chunk this node belongs to.
    pub chunk_id: ChunkId,
}

// =============================================================================
// Path Node
// =============================================================================

/// Path-based canvas node (curves, shapes).
///
/// Corresponds to TypeScript `CanvasPathNode`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasPathNode {
    /// Common node fields.
    #[serde(flatten)]
    pub base: CanvasNodeBase,

    /// SVG-like path commands.
    pub path_data: Vec<PathCommand>,

    /// Fill style (solid, gradient, pattern).
    pub fill: Option<FillStyle>,

    /// Stroke style.
    pub stroke: Option<StrokeStyle>,
}

// PathCommand is re-exported from vsc_core (see pub use above).
// Type correspondence to TypeScript:
// | TypeScript       | Rust                   |
// |------------------|------------------------|
// | `{ type: 'M' }`  | `PathCommand::MoveTo`  |
// | `{ type: 'L' }`  | `PathCommand::LineTo`  |
// | `{ type: 'C' }`  | `PathCommand::CubicTo` |
// | `{ type: 'Q' }`  | `PathCommand::QuadTo`  |
// | `{ type: 'A' }`  | `PathCommand::ArcTo`   |
// | `{ type: 'Z' }`  | `PathCommand::Close`   |

// =============================================================================
// Fill and Stroke Styles
// =============================================================================

/// Fill style for paths.
///
/// Corresponds to TypeScript `FillStyle`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum FillStyle {
    /// Solid color fill.
    Solid {
        /// Packed RGBA bytes [r, g, b, a]. Use `shaders::hex_to_rgba()` to parse from hex.
        rgba: [u8; 4],
    },

    /// Linear gradient fill.
    LinearGradient {
        /// Gradient color stops.
        stops: Vec<GradientStop>,
        /// Start point (normalized 0-1 within bounds).
        #[serde(default)]
        start: Option<GradientPoint>,
        /// End point (normalized 0-1 within bounds).
        #[serde(default)]
        end: Option<GradientPoint>,
    },

    /// Radial gradient fill.
    RadialGradient {
        /// Gradient color stops.
        stops: Vec<GradientStop>,
        /// Center point (normalized 0-1 within bounds).
        #[serde(default)]
        center: Option<GradientPoint>,
        /// Radius (normalized to bounds).
        #[serde(default)]
        radius: Option<Rational>,
    },

    /// Pattern fill (references another entity).
    Pattern {
        /// EntityId of the pattern source.
        pattern_ref: EntityId,
    },

    /// External texture fill (Phase J-3).
    ///
    /// References a texture managed by the host via TextureRegistry.
    /// The texture_id corresponds to `TextureHandle.id` from Q-dimension.
    ///
    /// **Current status:** Type definition only. Rendering falls back to magenta
    /// until TextureRegistry and texture sampling shader are implemented.
    ExternalTexture {
        /// Opaque texture ID from host (maps to TextureHandle.id).
        texture_id: u64,
        /// UV transformation for texture mapping.
        uv_transform: UvTransform,
    },
}

/// A color stop in a gradient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradientStop {
    /// Position along gradient axis (0-1).
    pub offset: Rational,
    /// Packed RGBA bytes [r, g, b, a].
    pub rgba: [u8; 4],
}

/// Point for gradient positioning (normalized 0-1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradientPoint {
    pub x: Rational,
    pub y: Rational,
}

/// Stroke style for paths.
///
/// Corresponds to TypeScript `StrokeStyle`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrokeStyle {
    /// Packed RGBA bytes [r, g, b, a].
    pub rgba: [u8; 4],

    /// Stroke width (exact rational).
    pub width: Rational,

    /// Line cap style.
    pub line_cap: LineCap,

    /// Line join style.
    pub line_join: LineJoin,

    /// Dash pattern (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash_array: Option<Vec<Rational>>,
}

// LineCap and LineJoin are re-exported from vsc_core (see pub use above).

// =============================================================================
// Text Node
// =============================================================================

/// Text canvas node.
///
/// Corresponds to TypeScript `CanvasTextNode`.
/// Text rendering is deferred to Phase E due to complexity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasTextNode {
    /// Common node fields.
    #[serde(flatten)]
    pub base: CanvasNodeBase,

    /// Text content (string or Q-dimension reference).
    pub content: TextContent,

    /// Font specification.
    pub font: FontSpec,

    /// Pre-computed glyph positions (from text shaping).
    #[serde(default)]
    pub glyphs: Vec<GlyphRun>,
}

/// Text content source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextContent {
    /// Static string content.
    Static(String),
    /// Dynamic Q-dimension reference.
    Dynamic(QDimensionRef),
}

/// Font specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontSpec {
    /// Font family name.
    pub family: String,
    /// Font size (exact rational).
    pub size: Rational,
    /// Font weight (100-900).
    pub weight: u16,
    /// Font style.
    pub style: FontStyle,
}

/// Font style variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

/// A run of glyphs with positions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlyphRun {
    /// Glyph IDs from the font.
    pub glyph_ids: Vec<u16>,
    /// Positions in device pixels.
    pub positions: Vec<RasterCoord>,
}

/// Rasterized coordinate for glyph positioning.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RasterCoord {
    pub x: f64,
    pub y: f64,
    pub z_index: i32,
}

// =============================================================================
// Image Node
// =============================================================================

/// Image canvas node.
///
/// Corresponds to TypeScript `CanvasImageNode`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasImageNode {
    /// Common node fields.
    #[serde(flatten)]
    pub base: CanvasNodeBase,

    /// Q-dimension image source reference.
    pub source: QDimensionRef,

    /// Object-fit mode.
    pub fit: ImageFit,
}

/// Image fitting mode (CSS object-fit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ImageFit {
    Fill,
    #[default]
    Contain,
    Cover,
    None,
}

// =============================================================================
// Group Node
// =============================================================================

/// Group canvas node with transform and children.
///
/// Corresponds to TypeScript `CanvasGroupNode`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasGroupNode {
    /// Common node fields.
    #[serde(flatten)]
    pub base: CanvasNodeBase,

    /// Child nodes.
    pub children: Vec<CanvasNode>,

    /// 2D affine transform matrix.
    pub transform: AffineTransform,

    /// Optional clip path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_path: Option<Vec<PathCommand>>,

    /// Opacity (0.0 - 1.0).
    pub opacity: f64,
}

/// 2D affine transformation matrix.
///
/// ```text
/// | a  b  tx |
/// | c  d  ty |
/// | 0  0  1  |
/// ```
///
/// Point transformation: [x', y'] = [a*x + c*y + tx, b*x + d*y + ty]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AffineTransform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Default for AffineTransform {
    /// Identity transform.
    fn default() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }
}

impl AffineTransform {
    /// Create an identity transform.
    pub fn identity() -> Self {
        Self::default()
    }

    /// Create a translation transform.
    pub fn translation(tx: f64, ty: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx,
            ty,
        }
    }

    /// Create a scale transform.
    pub fn scale(sx: f64, sy: f64) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Create a rotation transform (angle in radians).
    pub fn rotation(angle: f64) -> Self {
        let cos = angle.cos();
        let sin = angle.sin();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Multiply two transforms (self * other).
    ///
    /// In matrix terms: applies `other` first, then `self`.
    pub fn then(&self, other: &Self) -> Self {
        Self {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            tx: self.a * other.tx + self.b * other.ty + self.tx,
            ty: self.c * other.tx + self.d * other.ty + self.ty,
        }
    }

    /// Compose transforms: apply `other` in local space, then `self`.
    ///
    /// This is the natural order for hierarchical transforms:
    /// - `self` is the accumulated parent transform
    /// - `other` is the child's local transform
    /// - Result applies child transform first, then parent
    ///
    /// Mathematically: `self * other` (matrix multiplication order).
    ///
    /// ## Scene Graph Semantics
    ///
    /// For a point P in a child node's local coordinates:
    /// 1. Apply child's local transform: `other * P`
    /// 2. Apply parent's transform: `self * (other * P)`
    /// 3. Combined: `(self * other) * P`
    ///
    /// ## Example
    ///
    /// ```
    /// # use vsc_gpu::AffineTransform;
    /// let parent = AffineTransform::translation(100.0, 0.0);
    /// let child = AffineTransform::scale(2.0, 2.0);
    ///
    /// // Point at (10, 10) in child's local space:
    /// // 1. Scale by 2 → (20, 20)
    /// // 2. Translate by 100 → (120, 20)
    /// let combined = parent.compose(&child);
    /// let (x, y) = combined.transform_point(10.0, 10.0);
    /// assert!((x - 120.0).abs() < 0.001);
    /// assert!((y - 20.0).abs() < 0.001);
    /// ```
    pub fn compose(&self, other: &Self) -> Self {
        // Combined = self * other (parent * child)
        // Point transforms as: (parent * child) * point
        self.then(other)
    }

    /// Transform a point.
    pub fn transform_point(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }
}

// =============================================================================
// Q-Dimension Reference
// =============================================================================

/// Reference to Q-dimension (unpredictable) data source.
///
/// Q-dimension sources include user input, network fetches, and other
/// non-deterministic data. The P-dimension solver cannot directly access
/// these; they are injected as T-vector mutations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QDimensionRef {
    /// Type of Q-dimension source.
    #[serde(rename = "type")]
    pub source_type: QDimensionType,

    /// Source identifier.
    pub source_id: String,
}

/// Types of Q-dimension data sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QDimensionType {
    UserInput,
    Fetch,
    Image,
    Video,
    Audio,
    Shader,
    Time,
}

// =============================================================================
// Rational to f64 Conversion (Rasterization Boundary)
// =============================================================================

/// Extension trait for rasterization boundary conversion.
pub trait ToF64ForRasterization {
    /// Convert to f64 at the RASTERIZATION BOUNDARY ONLY.
    fn to_f64(&self) -> f64;
}

impl ToF64ForRasterization for Rational {
    #[inline]
    fn to_f64(&self) -> f64 {
        self.to_f64_for_rasterization()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_command_serde() {
        let cmd = PathCommand::MoveTo {
            x: Rational::from_int(100),
            y: Rational::from_int(200),
        };

        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""type":"M""#));

        let parsed: PathCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cmd);
    }

    #[test]
    fn test_fill_style_solid_serde() {
        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        let json = serde_json::to_string(&fill).unwrap();
        assert!(json.contains(r#""type":"solid""#));

        let parsed: FillStyle = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, fill);
    }

    #[test]
    fn test_fill_style_linear_gradient_serde() {
        let fill = FillStyle::LinearGradient {
            stops: vec![
                GradientStop {
                    offset: Rational::zero(),
                    rgba: [255, 0, 0, 255],
                },
                GradientStop {
                    offset: Rational::one(),
                    rgba: [0, 0, 255, 255],
                },
            ],
            start: None,
            end: None,
        };

        let json = serde_json::to_string(&fill).unwrap();
        assert!(json.contains(r#""type":"linear-gradient""#));

        let parsed: FillStyle = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, fill);
    }

    #[test]
    fn test_affine_transform_identity() {
        let t = AffineTransform::identity();
        let (x, y) = t.transform_point(10.0, 20.0);
        assert_eq!(x, 10.0);
        assert_eq!(y, 20.0);
    }

    #[test]
    fn test_affine_transform_translation() {
        let t = AffineTransform::translation(5.0, 10.0);
        let (x, y) = t.transform_point(10.0, 20.0);
        assert_eq!(x, 15.0);
        assert_eq!(y, 30.0);
    }

    #[test]
    fn test_affine_transform_scale() {
        let t = AffineTransform::scale(2.0, 3.0);
        let (x, y) = t.transform_point(10.0, 20.0);
        assert_eq!(x, 20.0);
        assert_eq!(y, 60.0);
    }

    #[test]
    fn test_canvas_node_entity_id() {
        let node = CanvasNode::Path(CanvasPathNode {
            base: CanvasNodeBase {
                entity_id: EntityId(42),
                bounds: PVectorBounds {
                    top_left: PVector {
                        x: Rational::zero(),
                        y: Rational::zero(),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                    bottom_right: PVector {
                        x: Rational::from_int(100),
                        y: Rational::from_int(100),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: "main".to_string(),
            },
            path_data: vec![
                PathCommand::MoveTo {
                    x: Rational::zero(),
                    y: Rational::zero(),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(100),
                    y: Rational::from_int(100),
                },
                PathCommand::Close,
            ],
            fill: Some(FillStyle::Solid {
                rgba: [0, 0, 0, 255],
            }),
            stroke: None,
        });

        assert_eq!(node.entity_id(), EntityId(42));
    }
}
