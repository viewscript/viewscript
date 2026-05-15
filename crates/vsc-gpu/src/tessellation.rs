//! Path Tessellation Module
//!
//! Converts P-dimension `PathCommand` sequences (with exact Rational coordinates)
//! into GPU-ready vertex buffers using lyon tessellation.
//!
//! ## Rasterization Boundary
//!
//! This module is the RASTERIZATION BOUNDARY for path geometry. The conversion
//! from `Rational` to `f32` happens here, immediately before lyon processing.
//!
//! ```text
//! PathCommand (Rational)
//!        │
//!        ▼
//! ┌──────────────────────────┐
//! │  to_f32_for_rasterization │  ← Rasterization Boundary
//! └──────────────────────────┘
//!        │
//!        ▼
//! lyon::path::Path (f32)
//!        │
//!        ▼
//! ┌──────────────────────────┐
//! │  FillTessellator         │
//! │  or StrokeTessellator    │
//! └──────────────────────────┘
//!        │
//!        ▼
//! TessellationOutput (GpuVertex[])
//! ```
//!
//! ## Tessellator Selection
//!
//! - **FillTessellator**: Used when `fill` is `Some(_)`. Produces triangles
//!   that cover the interior of the path according to fill rule (nonzero/evenodd).
//!
//! - **StrokeTessellator**: Used when `stroke` is `Some(_)`. Produces triangles
//!   that form the outline of the path with specified width, cap, and join styles.
//!
//! A path with both fill and stroke requires two tessellation passes.

use lyon::geom::{point, Angle};
use lyon::path::{builder::SvgPathBuilder, ArcFlags, Path as LyonPath};
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, StrokeOptions,
    StrokeTessellator, StrokeVertex, StrokeVertexConstructor, VertexBuffers,
};
use thiserror::Error;
use vsc_core::Rational;

// Re-export wgpu types used in GpuVertex::desc()
#[cfg(feature = "gpu")]
pub use wgpu;

use crate::{FillStyle, LineCap, LineJoin, PathCommand, StrokeStyle};

// =============================================================================
// Output Types
// =============================================================================

/// GPU vertex for wgpu rendering.
///
/// Uses f32 for GPU compatibility.
/// - `position`: Device pixel coordinates
/// - `uv`: Normalized coordinates within bounding box [0, 1]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVertex {
    /// Position in device pixels.
    pub position: [f32; 2],
    /// UV coordinates normalized to bounding box [0, 1].
    /// Used by fragment shader for gradient evaluation.
    pub uv: [f32; 2],
}

impl GpuVertex {
    /// Create a new vertex at the given position with UV coordinates.
    pub fn new(x: f32, y: f32, u: f32, v: f32) -> Self {
        Self {
            position: [x, y],
            uv: [u, v],
        }
    }

    /// Vertex buffer layout descriptor for wgpu pipeline.
    #[cfg(feature = "gpu")]
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // @location(0) position: vec2<f32>
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(1) uv: vec2<f32>
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

/// Axis-aligned bounding box for UV coordinate calculation.
///
/// Uses `Rational` coordinates to preserve P-dimension precision for bounds.
/// This keeps the bounding box computation as a single rasterization boundary
/// point, rather than converting to f32 prematurely.
///
/// ## UV Computation Precision
///
/// When computing UV coordinates, the bounding box bounds are converted to f64
/// (not f32) to minimize precision loss in the division. The vertex position
/// from lyon is already f32, so we work in f64 precision and convert the
/// final UV result to f32.
#[derive(Debug, Clone)]
pub struct BoundingBox {
    pub min_x: Rational,
    pub min_y: Rational,
    pub max_x: Rational,
    pub max_y: Rational,
}

impl BoundingBox {
    /// Create a bounding box with inverted bounds (ready for expansion).
    pub fn empty() -> Self {
        // Use very large/small rational values as sentinels
        let max_val = Rational::from_int(i64::MAX / 2); // Avoid overflow in arithmetic
        let min_val = Rational::from_int(i64::MIN / 2);
        Self {
            min_x: max_val.clone(),
            min_y: max_val,
            max_x: min_val.clone(),
            max_y: min_val,
        }
    }

    /// Expand the bounding box to include a point.
    pub fn expand(&mut self, x: &Rational, y: &Rational) {
        if *x < self.min_x {
            self.min_x = x.clone();
        }
        if *x > self.max_x {
            self.max_x = x.clone();
        }
        if *y < self.min_y {
            self.min_y = y.clone();
        }
        if *y > self.max_y {
            self.max_y = y.clone();
        }
    }

    /// Compute normalized UV coordinates for an f32 point within this bounding box.
    ///
    /// The bounding box bounds are converted to f64 at the RASTERIZATION BOUNDARY,
    /// UV computation is performed in f64 precision, and the result is converted
    /// to f32 for the GPU.
    ///
    /// ## Why f64 intermediate precision?
    ///
    /// - Vertex positions from lyon are f32 (unavoidable)
    /// - Bounding box bounds are Rational (exact)
    /// - Division in f64 preserves more precision than f32
    /// - Final UV is f32 (GPU requirement)
    ///
    /// Returns [0, 1] for points within the box.
    pub fn normalize_f32_pos(&self, x: f32, y: f32) -> [f32; 2] {
        // RASTERIZATION BOUNDARY: Convert Rational bounds to f64
        let min_x = self.min_x.to_f64_for_rasterization();
        let min_y = self.min_y.to_f64_for_rasterization();
        let max_x = self.max_x.to_f64_for_rasterization();
        let max_y = self.max_y.to_f64_for_rasterization();

        let width = max_x - min_x;
        let height = max_y - min_y;

        // Compute UV in f64 precision
        let u = if width.abs() > f64::EPSILON {
            ((x as f64) - min_x) / width
        } else {
            0.5 // Degenerate width
        };

        let v = if height.abs() > f64::EPSILON {
            ((y as f64) - min_y) / height
        } else {
            0.5 // Degenerate height
        };

        [u as f32, v as f32]
    }
}

/// Tessellation output containing vertex and index buffers.
#[derive(Debug, Clone)]
pub struct TessellationOutput {
    /// Vertex buffer (positions).
    pub vertices: Vec<GpuVertex>,

    /// Index buffer (triangle indices, u32 for wgpu compatibility).
    pub indices: Vec<u32>,

    /// Number of triangles generated.
    pub triangle_count: usize,
}

impl TessellationOutput {
    /// Create an empty output.
    pub fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            triangle_count: 0,
        }
    }

    /// Check if the output is empty.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Merge another output into this one.
    pub fn merge(&mut self, other: TessellationOutput) {
        let vertex_offset = self.vertices.len() as u32;

        self.vertices.extend(other.vertices);
        self.indices
            .extend(other.indices.iter().map(|i| i + vertex_offset));
        self.triangle_count += other.triangle_count;
    }
}

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur during tessellation.
#[derive(Debug, Error)]
pub enum TessellationError {
    /// Path is empty (no commands).
    #[error("Path is empty")]
    EmptyPath,

    /// Path does not start with MoveTo.
    #[error("Path must start with MoveTo command")]
    MissingInitialMoveTo,

    /// Lyon tessellation failed.
    #[error("Tessellation failed: {0}")]
    TessellationFailed(String),

    /// Invalid path command sequence.
    #[error("Invalid path command at index {index}: {reason}")]
    InvalidCommand { index: usize, reason: String },
}

// =============================================================================
// Vertex Constructors for Lyon
// =============================================================================

/// Vertex constructor for fill tessellation with UV computation.
///
/// Bounding box bounds are stored as Rational and converted to f64 at the
/// rasterization boundary during UV computation. This keeps the bounding box
/// as the single source of truth in Rational precision.
struct FillVertexBuilder {
    bbox: BoundingBox,
}

impl FillVertexBuilder {
    fn new(bbox: BoundingBox) -> Self {
        Self { bbox }
    }
}

impl FillVertexConstructor<GpuVertex> for FillVertexBuilder {
    fn new_vertex(&mut self, vertex: FillVertex) -> GpuVertex {
        let pos = vertex.position();
        let uv = self.bbox.normalize_f32_pos(pos.x, pos.y);
        GpuVertex::new(pos.x, pos.y, uv[0], uv[1])
    }
}

/// Vertex constructor for stroke tessellation with UV computation.
struct StrokeVertexBuilder {
    bbox: BoundingBox,
}

impl StrokeVertexBuilder {
    fn new(bbox: BoundingBox) -> Self {
        Self { bbox }
    }
}

impl StrokeVertexConstructor<GpuVertex> for StrokeVertexBuilder {
    fn new_vertex(&mut self, vertex: StrokeVertex) -> GpuVertex {
        let pos = vertex.position();
        let uv = self.bbox.normalize_f32_pos(pos.x, pos.y);
        GpuVertex::new(pos.x, pos.y, uv[0], uv[1])
    }
}

// =============================================================================
// Rational to f32 Conversion
// =============================================================================

/// Convert Rational to f32 at the RASTERIZATION BOUNDARY.
///
/// This is the only place where Rational coordinates become floating-point.
///
/// ## Future Replacement (Topology-Preserving Rounding)
///
/// This direct conversion is a PLACEHOLDER. When `rasterizer.rs` implements
/// topology-preserving rounding, this function will be replaced:
///
/// 1. Union-Find equivalence class construction (Rational or f64 precision)
/// 2. Largest Remainder Method for pixel grid snapping
/// 3. THEN convert to f32 for GPU vertex buffers
///
/// The current `Rational → f64 → f32` chain loses the opportunity for
/// coordinated rounding of shared vertices. The future chain will be:
///
/// ```text
/// Rational → rasterizer::round() → i32 (pixel coords) → f32 (GPU)
/// ```
#[inline]
fn to_f32(r: &Rational) -> f32 {
    r.to_f64_for_rasterization() as f32
}

// =============================================================================
// Path Building
// =============================================================================

/// Build a lyon Path from PathCommand slice.
///
/// Supports all PathCommand variants:
/// - MoveTo, LineTo, Close: Basic path construction
/// - CubicTo: Cubic Bezier curves
/// - QuadTo: Quadratic Bezier curves
/// - ArcTo: SVG-style elliptical arcs
fn build_lyon_path(commands: &[PathCommand]) -> Result<LyonPath, TessellationError> {
    if commands.is_empty() {
        return Err(TessellationError::EmptyPath);
    }

    // Verify first command is MoveTo
    match &commands[0] {
        PathCommand::MoveTo { .. } => {}
        _ => return Err(TessellationError::MissingInitialMoveTo),
    }

    // Use SvgPathBuilder for arc_to support
    let mut builder = LyonPath::builder().with_svg();
    let mut path_started = false;

    for (_index, cmd) in commands.iter().enumerate() {
        match cmd {
            PathCommand::MoveTo { x, y } => {
                // SvgPathBuilder automatically ends the previous subpath when move_to is called
                builder.move_to(point(to_f32(x), to_f32(y)));
                path_started = true;
            }

            PathCommand::LineTo { x, y } => {
                builder.line_to(point(to_f32(x), to_f32(y)));
            }

            PathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                // Cubic Bezier: two control points (x1,y1), (x2,y2) and endpoint (x,y)
                builder.cubic_bezier_to(
                    point(to_f32(x1), to_f32(y1)),
                    point(to_f32(x2), to_f32(y2)),
                    point(to_f32(x), to_f32(y)),
                );
            }

            PathCommand::QuadTo { x1, y1, x, y } => {
                // Quadratic Bezier: one control point (x1,y1) and endpoint (x,y)
                builder.quadratic_bezier_to(
                    point(to_f32(x1), to_f32(y1)),
                    point(to_f32(x), to_f32(y)),
                );
            }

            PathCommand::ArcTo {
                rx,
                ry,
                rotation,
                large_arc,
                sweep,
                x,
                y,
            } => {
                // SVG-style elliptical arc
                // rotation is already f64 (per Section 9.6 exception)
                let radii = lyon::geom::Vector::new(to_f32(rx), to_f32(ry));
                let x_rotation = Angle::degrees(*rotation as f32);
                let flags = ArcFlags {
                    large_arc: *large_arc,
                    sweep: *sweep,
                };
                builder.arc_to(radii, x_rotation, flags, point(to_f32(x), to_f32(y)));
            }

            PathCommand::Close => {
                builder.close();
                path_started = false;
            }
        }
    }

    // SvgPathBuilder automatically handles open subpaths when build() is called
    let _ = path_started; // silence unused warning

    Ok(builder.build())
}

// =============================================================================
// Bounding Box Computation
// =============================================================================

/// Compute bounding box from path commands.
///
/// Note: This is a conservative estimate that includes control points.
/// For curves, the actual bounding box may be smaller, but this is safe
/// for UV normalization purposes.
fn compute_bounding_box(commands: &[PathCommand]) -> BoundingBox {
    let mut bbox = BoundingBox::empty();

    for cmd in commands {
        match cmd {
            PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => {
                bbox.expand(x, y);
            }
            PathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                // Include control points and endpoint (conservative bound)
                bbox.expand(x1, y1);
                bbox.expand(x2, y2);
                bbox.expand(x, y);
            }
            PathCommand::QuadTo { x1, y1, x, y } => {
                // Include control point and endpoint
                bbox.expand(x1, y1);
                bbox.expand(x, y);
            }
            PathCommand::ArcTo { rx, ry, x, y, .. } => {
                // Conservative: expand by radii in all directions from endpoint
                // Arc can extend up to rx/ry from its endpoints
                bbox.expand(x, y);
                // Clone for arithmetic (Rational ops consume ownership)
                let x_minus_rx = x.clone() - rx.clone();
                let y_minus_ry = y.clone() - ry.clone();
                let x_plus_rx = x.clone() + rx.clone();
                let y_plus_ry = y.clone() + ry.clone();
                bbox.expand(&x_minus_rx, &y_minus_ry);
                bbox.expand(&x_plus_rx, &y_plus_ry);
            }
            PathCommand::Close => {}
        }
    }

    bbox
}

// =============================================================================
// Main Tessellation Function
// =============================================================================

/// Tessellate a path into GPU-ready vertex buffers.
///
/// ## Parameters
///
/// - `commands`: Slice of PathCommand with Rational coordinates (P-dimension).
/// - `fill`: Optional fill style. If present, fill tessellation is performed.
///
/// ## Returns
///
/// `TessellationOutput` containing vertex and index buffers suitable for wgpu.
///
/// ## Tessellator Selection
///
/// This function uses `FillTessellator` when `fill` is `Some(_)`.
/// The fill tessellator produces triangles covering the path interior.
///
/// For stroke tessellation, use `tessellate_path_stroke()` (to be implemented).
///
/// ## Example
///
/// ```ignore
/// use vsc_gpu::{PathCommand, FillStyle, tessellation::tessellate_path};
/// use vsc_core::Rational;
///
/// let commands = vec![
///     PathCommand::MoveTo { x: Rational::zero(), y: Rational::zero() },
///     PathCommand::LineTo { x: Rational::from_int(100), y: Rational::zero() },
///     PathCommand::LineTo { x: Rational::from_int(100), y: Rational::from_int(100) },
///     PathCommand::Close,
/// ];
///
/// let fill = Some(FillStyle::Solid { color: "#ff0000".to_string() });
/// let output = tessellate_path(&commands, fill.as_ref())?;
///
/// // output.vertices and output.indices are ready for wgpu
/// ```
pub fn tessellate_path(
    commands: &[PathCommand],
    fill: Option<&FillStyle>,
) -> Result<TessellationOutput, TessellationError> {
    // Build lyon path from commands (Rational → f32 conversion happens here)
    let path = build_lyon_path(commands)?;

    // If no fill style, return empty output
    // (Stroke-only paths will use tessellate_path_stroke)
    let _fill = match fill {
        Some(f) => f,
        None => return Ok(TessellationOutput::empty()),
    };

    // Compute bounding box for UV normalization
    let bbox = compute_bounding_box(commands);

    // Prepare vertex buffers
    let mut buffers: VertexBuffers<GpuVertex, u32> = VertexBuffers::new();

    // Create fill tessellator
    let mut tessellator = FillTessellator::new();

    // Fill options: NonZero winding for overlapping contours (like + sign)
    let options = FillOptions::non_zero();

    // Tessellate the path with UV-aware vertex builder
    tessellator
        .tessellate_path(
            &path,
            &options,
            &mut BuffersBuilder::new(&mut buffers, FillVertexBuilder::new(bbox)),
        )
        .map_err(|e| TessellationError::TessellationFailed(format!("{:?}", e)))?;

    let triangle_count = buffers.indices.len() / 3;

    Ok(TessellationOutput {
        vertices: buffers.vertices,
        indices: buffers.indices,
        triangle_count,
    })
}

/// Tessellate a path stroke into GPU-ready vertex buffers.
///
/// ## Parameters
///
/// - `commands`: Slice of PathCommand with Rational coordinates (P-dimension).
/// - `stroke`: Stroke style specifying width, cap, and join.
///
/// ## Tessellator Selection
///
/// Uses `StrokeTessellator` which produces triangles forming the path outline.
pub fn tessellate_path_stroke(
    commands: &[PathCommand],
    stroke: &StrokeStyle,
) -> Result<TessellationOutput, TessellationError> {
    // Build lyon path from commands
    let path = build_lyon_path(commands)?;

    // Compute bounding box for UV normalization
    // Note: stroke extends beyond path bounds, but we use path bounds for consistency
    let bbox = compute_bounding_box(commands);

    // Prepare vertex buffers
    let mut buffers: VertexBuffers<GpuVertex, u32> = VertexBuffers::new();

    // Create stroke tessellator
    let mut tessellator = StrokeTessellator::new();

    // Convert stroke width from Rational
    let width = to_f32(&stroke.width);

    // Map line cap
    let line_cap = match stroke.line_cap {
        LineCap::Butt => lyon::tessellation::LineCap::Butt,
        LineCap::Round => lyon::tessellation::LineCap::Round,
        LineCap::Square => lyon::tessellation::LineCap::Square,
    };

    // Map line join
    let line_join = match stroke.line_join {
        LineJoin::Miter => lyon::tessellation::LineJoin::Miter,
        LineJoin::Round => lyon::tessellation::LineJoin::Round,
        LineJoin::Bevel => lyon::tessellation::LineJoin::Bevel,
    };

    // Stroke options
    let options = StrokeOptions::default()
        .with_line_width(width)
        .with_line_cap(line_cap)
        .with_line_join(line_join);

    // Tessellate the path with UV-aware vertex builder
    tessellator
        .tessellate_path(
            &path,
            &options,
            &mut BuffersBuilder::new(&mut buffers, StrokeVertexBuilder::new(bbox)),
        )
        .map_err(|e| TessellationError::TessellationFailed(format!("{:?}", e)))?;

    let triangle_count = buffers.indices.len() / 3;

    Ok(TessellationOutput {
        vertices: buffers.vertices,
        indices: buffers.indices,
        triangle_count,
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use vsc_core::Rational;

    #[test]
    fn test_tessellate_triangle() {
        // Simple triangle: (0,0) → (100,0) → (50,100) → close
        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: Rational::from_int(50),
                y: Rational::from_int(100),
            },
            PathCommand::Close,
        ];

        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        let output = tessellate_path(&commands, Some(&fill)).unwrap();

        // Should produce at least 1 triangle (3 indices)
        assert!(!output.is_empty());
        assert!(output.indices.len() >= 3);
        assert_eq!(output.indices.len() % 3, 0); // Must be multiple of 3
        assert!(output.triangle_count >= 1);

        println!(
            "Triangle tessellation: {} vertices, {} indices, {} triangles",
            output.vertices.len(),
            output.indices.len(),
            output.triangle_count
        );
    }

    #[test]
    fn test_tessellate_rectangle() {
        // Rectangle: (0,0) → (100,0) → (100,50) → (0,50) → close
        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::from_int(50),
            },
            PathCommand::LineTo {
                x: Rational::zero(),
                y: Rational::from_int(50),
            },
            PathCommand::Close,
        ];

        let fill = FillStyle::Solid {
            rgba: [0, 255, 0, 255],
        };

        let output = tessellate_path(&commands, Some(&fill)).unwrap();

        // Rectangle should produce at least 2 triangles
        assert!(output.triangle_count >= 2);

        println!(
            "Rectangle tessellation: {} vertices, {} indices, {} triangles",
            output.vertices.len(),
            output.indices.len(),
            output.triangle_count
        );
    }

    #[test]
    fn test_tessellate_stroke() {
        // Simple line with stroke
        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::from_int(100),
            },
        ];

        let stroke = StrokeStyle {
            rgba: [0, 0, 255, 255],
            width: Rational::from_int(5),
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            dash_array: None,
        };

        let output = tessellate_path_stroke(&commands, &stroke).unwrap();

        // Stroke should produce triangles for the line width
        assert!(!output.is_empty());
        assert!(output.triangle_count >= 1);

        println!(
            "Stroke tessellation: {} vertices, {} indices, {} triangles",
            output.vertices.len(),
            output.indices.len(),
            output.triangle_count
        );
    }

    #[test]
    fn test_empty_path_error() {
        let commands: Vec<PathCommand> = vec![];
        let fill = FillStyle::Solid {
            rgba: [0, 0, 0, 255],
        };

        let result = tessellate_path(&commands, Some(&fill));
        assert!(matches!(result, Err(TessellationError::EmptyPath)));
    }

    #[test]
    fn test_missing_moveto_error() {
        // Path starting with LineTo (invalid)
        let commands = vec![PathCommand::LineTo {
            x: Rational::from_int(100),
            y: Rational::from_int(100),
        }];

        let fill = FillStyle::Solid {
            rgba: [0, 0, 0, 255],
        };

        let result = tessellate_path(&commands, Some(&fill));
        assert!(matches!(
            result,
            Err(TessellationError::MissingInitialMoveTo)
        ));
    }

    #[test]
    fn test_no_fill_returns_empty() {
        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::zero(),
            },
            PathCommand::Close,
        ];

        // No fill style
        let output = tessellate_path(&commands, None).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_rational_precision_preserved() {
        // Use non-integer rational: 1/3
        let one_third = Rational::new(1, 3);

        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: one_third.clone(),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: one_third,
                y: Rational::from_int(1),
            },
            PathCommand::Close,
        ];

        let fill = FillStyle::Solid {
            rgba: [0, 0, 0, 255],
        };

        let output = tessellate_path(&commands, Some(&fill)).unwrap();

        // Should successfully tessellate with fractional coordinates
        assert!(!output.is_empty());

        // Check that vertices contain approximately 0.333...
        let has_fractional = output
            .vertices
            .iter()
            .any(|v| (v.position[0] - 0.333).abs() < 0.01);
        assert!(has_fractional, "Expected fractional coordinate 1/3 ≈ 0.333");
    }

    #[test]
    fn test_gpu_vertex_bytemuck() {
        // Verify GpuVertex can be cast to bytes (required for wgpu)
        let vertex = GpuVertex::new(1.0, 2.0, 0.5, 0.5);
        let bytes: &[u8] = bytemuck::bytes_of(&vertex);
        assert_eq!(bytes.len(), 16); // 4 * f32 = 16 bytes (position + uv)
    }

    #[test]
    fn test_uv_coordinates_normalized() {
        // Rectangle from (10, 20) to (110, 70)
        // Bounding box: min=(10, 20), max=(110, 70)
        // UV should be normalized to [0, 1]
        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::from_int(10),
                y: Rational::from_int(20),
            },
            PathCommand::LineTo {
                x: Rational::from_int(110),
                y: Rational::from_int(20),
            },
            PathCommand::LineTo {
                x: Rational::from_int(110),
                y: Rational::from_int(70),
            },
            PathCommand::LineTo {
                x: Rational::from_int(10),
                y: Rational::from_int(70),
            },
            PathCommand::Close,
        ];

        let fill = FillStyle::Solid {
            rgba: [0, 0, 0, 255],
        };

        let output = tessellate_path(&commands, Some(&fill)).unwrap();

        // All UV coordinates should be in [0, 1] range
        for vertex in &output.vertices {
            assert!(
                vertex.uv[0] >= 0.0 && vertex.uv[0] <= 1.0,
                "UV.x out of range: {} at position ({}, {})",
                vertex.uv[0],
                vertex.position[0],
                vertex.position[1]
            );
            assert!(
                vertex.uv[1] >= 0.0 && vertex.uv[1] <= 1.0,
                "UV.y out of range: {} at position ({}, {})",
                vertex.uv[1],
                vertex.position[0],
                vertex.position[1]
            );
        }

        // Check corner UVs: find vertices near corners and verify their UVs
        let has_origin_uv = output.vertices.iter().any(|v| {
            (v.position[0] - 10.0).abs() < 1.0
                && (v.position[1] - 20.0).abs() < 1.0
                && v.uv[0].abs() < 0.1
                && v.uv[1].abs() < 0.1
        });
        assert!(has_origin_uv, "Expected UV (0, 0) at position (10, 20)");

        let has_opposite_uv = output.vertices.iter().any(|v| {
            (v.position[0] - 110.0).abs() < 1.0
                && (v.position[1] - 70.0).abs() < 1.0
                && (v.uv[0] - 1.0).abs() < 0.1
                && (v.uv[1] - 1.0).abs() < 0.1
        });
        assert!(has_opposite_uv, "Expected UV (1, 1) at position (110, 70)");

        println!(
            "UV test: {} vertices, all UVs in [0, 1] range",
            output.vertices.len()
        );
    }

    // =========================================================================
    // ArcTo Degenerate Radii Tests
    // =========================================================================

    #[test]
    fn test_arc_to_zero_rx_fallback() {
        // ArcTo with rx=0: lyon should treat this as a line (degenerate arc)
        // or return an error. Either way, there must be no panic.
        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            },
            PathCommand::ArcTo {
                rx: Rational::zero(), // rx = 0: degenerate radius
                ry: Rational::from_int(50),
                rotation: 0.0,
                large_arc: false,
                sweep: true,
                x: Rational::from_int(100),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::from_int(10),
            },
            PathCommand::LineTo {
                x: Rational::zero(),
                y: Rational::from_int(10),
            },
            PathCommand::Close,
        ];

        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        // Should not panic regardless of how lyon handles rx=0
        let result = tessellate_path(&commands, Some(&fill));
        // The result is either Ok (lyon fell back to a line) or Err
        // (lyon reported a tessellation error). Both are acceptable.
        match result {
            Ok(output) => {
                println!(
                    "ArcTo rx=0: lyon fallback to line, {} vertices",
                    output.vertices.len()
                );
            }
            Err(e) => {
                println!("ArcTo rx=0: tessellation error (expected): {:?}", e);
            }
        }
        // Unconditional: must not have panicked
    }

    #[test]
    fn test_arc_to_zero_ry_fallback() {
        // ArcTo with ry=0: lyon should treat this as a line or return an error.
        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            },
            PathCommand::ArcTo {
                rx: Rational::from_int(50),
                ry: Rational::zero(), // ry = 0: degenerate radius
                rotation: 0.0,
                large_arc: false,
                sweep: true,
                x: Rational::from_int(100),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::from_int(10),
            },
            PathCommand::LineTo {
                x: Rational::zero(),
                y: Rational::from_int(10),
            },
            PathCommand::Close,
        ];

        let fill = FillStyle::Solid {
            rgba: [0, 255, 0, 255],
        };

        let result = tessellate_path(&commands, Some(&fill));
        match result {
            Ok(output) => {
                println!(
                    "ArcTo ry=0: lyon fallback to line, {} vertices",
                    output.vertices.len()
                );
            }
            Err(e) => {
                println!("ArcTo ry=0: tessellation error (expected): {:?}", e);
            }
        }
    }

    // =========================================================================
    // Degenerate Width UV Tests (zero-division guard)
    // =========================================================================

    #[test]
    fn test_uv_no_zero_division_when_width_is_zero() {
        // All points share the same x-coordinate → width = 0 → degenerate path.
        // BoundingBox::normalize_f32_pos() must return 0.5 for u (not NaN/Inf).
        let x_fixed = Rational::from_int(50);
        let commands = vec![
            PathCommand::MoveTo {
                x: x_fixed.clone(),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: x_fixed.clone(),
                y: Rational::from_int(100),
            },
            PathCommand::LineTo {
                x: x_fixed.clone(),
                y: Rational::from_int(50),
            },
            PathCommand::Close,
        ];

        let fill = FillStyle::Solid {
            rgba: [0, 0, 255, 255],
        };

        // tessellate_path may produce an empty output (degenerate polygon has zero area)
        // but it must NOT panic or produce NaN/Inf UVs.
        let result = tessellate_path(&commands, Some(&fill));
        match result {
            Ok(output) => {
                // Verify no NaN or Inf in UV coordinates
                for vertex in &output.vertices {
                    assert!(
                        vertex.uv[0].is_finite(),
                        "UV.u should be finite (no NaN/Inf), got {}",
                        vertex.uv[0]
                    );
                    assert!(
                        vertex.uv[1].is_finite(),
                        "UV.v should be finite (no NaN/Inf), got {}",
                        vertex.uv[1]
                    );
                }
                println!(
                    "Degenerate width (all x={}): {} vertices, no NaN UV",
                    50,
                    output.vertices.len()
                );
            }
            Err(e) => {
                // Also acceptable — degenerate path may fail tessellation
                println!("Degenerate width: tessellation error (acceptable): {:?}", e);
            }
        }
    }

    // =========================================================================
    // CubicTo / QuadTo / ArcTo Tests
    // =========================================================================

    #[test]
    fn test_cubic_bezier_s_curve() {
        // S-curve: control points on opposite sides of the baseline
        // Baseline: (0,50) → (100,50)
        // Control point 1: (30, 0)   - above baseline
        // Control point 2: (70, 100) - below baseline
        // This creates an S-shaped curve that should produce non-degenerate tessellation
        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::from_int(50),
            },
            PathCommand::CubicTo {
                x1: Rational::from_int(30),
                y1: Rational::zero(), // control point 1 above
                x2: Rational::from_int(70),
                y2: Rational::from_int(100), // control point 2 below
                x: Rational::from_int(100),
                y: Rational::from_int(50),
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::from_int(60),
            },
            PathCommand::CubicTo {
                x1: Rational::from_int(70),
                y1: Rational::from_int(110), // mirrored control points
                x2: Rational::from_int(30),
                y2: Rational::from_int(10),
                x: Rational::zero(),
                y: Rational::from_int(60),
            },
            PathCommand::Close,
        ];

        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        let output = tessellate_path(&commands, Some(&fill)).unwrap();

        // Non-degenerate tessellation should produce multiple triangles
        assert!(
            !output.is_empty(),
            "CubicTo S-curve should not produce empty output"
        );
        assert!(
            output.triangle_count >= 2,
            "CubicTo S-curve should produce at least 2 triangles, got {}",
            output.triangle_count
        );

        // Verify we have a reasonable number of vertices (curves need more vertices than lines)
        assert!(
            output.vertices.len() >= 4,
            "CubicTo should produce at least 4 vertices for proper curve representation"
        );

        println!(
            "CubicTo S-curve: {} vertices, {} indices, {} triangles",
            output.vertices.len(),
            output.indices.len(),
            output.triangle_count
        );
    }

    #[test]
    fn test_quadratic_bezier_endpoints() {
        // Quadratic Bezier: start (0,0), control (50,100), end (100,0)
        // Creates a parabolic arc. The curve doesn't pass through the control point
        // but is "pulled toward" it. For a quadratic bezier, the curve reaches
        // max height at t=0.5, which is at y = 50 (halfway to control point).
        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            },
            PathCommand::QuadTo {
                x1: Rational::from_int(50),
                y1: Rational::from_int(100), // control point above
                x: Rational::from_int(100),
                y: Rational::zero(), // endpoint
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::from_int(10),
            },
            PathCommand::QuadTo {
                x1: Rational::from_int(50),
                y1: Rational::from_int(90),
                x: Rational::zero(),
                y: Rational::from_int(10),
            },
            PathCommand::Close,
        ];

        let fill = FillStyle::Solid {
            rgba: [0, 255, 0, 255],
        };

        let output = tessellate_path(&commands, Some(&fill)).unwrap();

        assert!(!output.is_empty(), "QuadTo should produce non-empty output");

        // Verify endpoints are correctly connected by checking vertex bounds
        let xs: Vec<f32> = output.vertices.iter().map(|v| v.position[0]).collect();
        let ys: Vec<f32> = output.vertices.iter().map(|v| v.position[1]).collect();

        let min_x = xs.iter().cloned().fold(f32::INFINITY, f32::min);
        let max_x = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let min_y = ys.iter().cloned().fold(f32::INFINITY, f32::min);
        let max_y = ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max);

        // Start point (0,0) and end point (100,0) should be near x bounds
        assert!(min_x < 1.0, "Minimum x should be near 0 (start point)");
        assert!(max_x > 99.0, "Maximum x should be near 100 (end point)");

        // Y should span from 0 to at least 40 (quadratic bezier reaches ~50% of control point height)
        assert!(min_y < 1.0, "Minimum y should be near 0 (endpoints)");
        assert!(
            max_y > 40.0,
            "Maximum y should show curve influence (got {:.1})",
            max_y
        );

        // Verify the path was properly tessellated with multiple triangles
        // (curved shapes need more than the minimum 2 triangles of a quad)
        assert!(
            output.triangle_count >= 2,
            "QuadTo should produce multiple triangles"
        );

        println!(
            "QuadTo: {} vertices, {} triangles, bounds x=[{:.1}, {:.1}] y=[{:.1}, {:.1}]",
            output.vertices.len(),
            output.triangle_count,
            min_x,
            max_x,
            min_y,
            max_y
        );
    }

    #[test]
    fn test_arc_to_large_arc_flag() {
        // Test that large_arc=true produces more vertices than large_arc=false
        //
        // Geometry: chord from (0,0) to (100,0) with radius=100
        // - chord = 100, 2r = 200, so chord/(2r) = 0.5
        // - small arc angle = 2 * arcsin(0.5) = 60°
        // - large arc angle = 360° - 60° = 300°
        //
        // This gives a 5:1 ratio in angular coverage, which should produce
        // noticeably different vertex counts.

        // Small arc (large_arc=false): 60° arc
        let small_arc_commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            },
            PathCommand::ArcTo {
                rx: Rational::from_int(100),
                ry: Rational::from_int(100),
                rotation: 0.0,
                large_arc: false, // small arc ~60°
                sweep: true,
                x: Rational::from_int(100),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::from_int(10),
            },
            PathCommand::ArcTo {
                rx: Rational::from_int(100),
                ry: Rational::from_int(100),
                rotation: 0.0,
                large_arc: false,
                sweep: false,
                x: Rational::zero(),
                y: Rational::from_int(10),
            },
            PathCommand::Close,
        ];

        // Large arc (large_arc=true): 300° arc
        let large_arc_commands = vec![
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            },
            PathCommand::ArcTo {
                rx: Rational::from_int(100),
                ry: Rational::from_int(100),
                rotation: 0.0,
                large_arc: true, // large arc ~300°
                sweep: true,
                x: Rational::from_int(100),
                y: Rational::zero(),
            },
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::from_int(10),
            },
            PathCommand::ArcTo {
                rx: Rational::from_int(100),
                ry: Rational::from_int(100),
                rotation: 0.0,
                large_arc: true,
                sweep: false,
                x: Rational::zero(),
                y: Rational::from_int(10),
            },
            PathCommand::Close,
        ];

        let fill = FillStyle::Solid {
            rgba: [0, 0, 255, 255],
        };

        let small_output = tessellate_path(&small_arc_commands, Some(&fill)).unwrap();
        let large_output = tessellate_path(&large_arc_commands, Some(&fill)).unwrap();

        // Both should produce valid tessellation
        assert!(!small_output.is_empty(), "Small arc should produce output");
        assert!(!large_output.is_empty(), "Large arc should produce output");

        // Large arc should have more vertices (covers more angular distance)
        // 300° vs 60° = 5x coverage, so vertices should be noticeably different
        assert!(
            large_output.vertices.len() > small_output.vertices.len(),
            "Large arc ({} vertices) should have more vertices than small arc ({} vertices)",
            large_output.vertices.len(),
            small_output.vertices.len()
        );

        // Vertex counts should be reasonable
        assert!(
            small_output.vertices.len() >= 4,
            "Small arc should have at least 4 vertices"
        );
        assert!(
            large_output.vertices.len() >= 8,
            "Large arc should have at least 8 vertices"
        );
        assert!(
            large_output.vertices.len() < 1000,
            "Large arc vertex count should be reasonable (< 1000)"
        );

        println!(
            "ArcTo: small_arc={} vertices, large_arc={} vertices (ratio: {:.2}x)",
            small_output.vertices.len(),
            large_output.vertices.len(),
            large_output.vertices.len() as f64 / small_output.vertices.len() as f64
        );
    }
}
