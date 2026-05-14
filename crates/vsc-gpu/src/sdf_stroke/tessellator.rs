//! SDF Stroke Tessellator
//!
//! Generates bounding rectangles for quadratic Bezier curve strokes.
//! Each curve segment produces 4 vertices (1 rectangle = 2 triangles).

use super::vertex::SdfStrokeVertex;
use vsc_core::PathCommand;

/// Output from SDF stroke tessellation.
///
/// ## Current Limitations (Phase I-3)
///
/// The current implementation renders each curve segment independently:
///
/// - **line_cap**: Effectively `butt` only. No `round` or `square` cap support.
/// - **line_join**: Not implemented. Segments are rendered without explicit join geometry.
///
/// When adjacent segments share endpoints (P2 of segment N = P0 of segment N+1),
/// the SDF distance fields naturally overlap at the junction, producing acceptable
/// visual results for most cases. However, sharp angles may show minor artifacts.
///
/// For full `line_join` (miter/round/bevel) and `line_cap` (round/square) support,
/// use the lyon tessellation fallback path (`tessellate_path_stroke()`), which
/// handles all SVG stroke styles correctly.
///
/// ## Future Work (Phase I-3b)
///
/// Round join and round cap will be added by emitting circular SDF regions at
/// segment junctions and endpoints:
/// - Join: Circle with radius `half_width` at shared endpoint
/// - Cap: Semicircle with radius `half_width` at path endpoints
#[derive(Debug, Clone)]
pub struct SdfStrokeOutput {
    /// Vertices for bounding rectangles.
    /// Each curve produces 4 vertices.
    pub vertices: Vec<SdfStrokeVertex>,

    /// Indices for triangles.
    /// Each curve produces 6 indices (2 triangles).
    pub indices: Vec<u32>,
}

impl SdfStrokeOutput {
    /// Create an empty output.
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Create with pre-allocated capacity.
    pub fn with_capacity(segment_count: usize) -> Self {
        Self {
            vertices: Vec::with_capacity(segment_count * 4),
            indices: Vec::with_capacity(segment_count * 6),
        }
    }

    /// Number of curve segments.
    pub fn segment_count(&self) -> usize {
        self.indices.len() / 6
    }

    /// Check if output is empty.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}

impl Default for SdfStrokeOutput {
    fn default() -> Self {
        Self::new()
    }
}

/// Tessellate quadratic Bezier strokes from path commands.
///
/// For each `QuadTo` segment, generates a bounding rectangle that encompasses
/// the curve plus stroke width. The rectangle vertices contain all curve
/// parameters needed for SDF evaluation in the fragment shader.
///
/// ## Parameters
///
/// - `commands`: Path commands (only `QuadTo` segments are processed)
/// - `stroke_width`: Total stroke width (half is added to each side)
///
/// ## Output
///
/// `SdfStrokeOutput` containing:
/// - `vertices`: 4 vertices per curve (rectangle corners)
/// - `indices`: 6 indices per curve (2 triangles)
///
/// ## Bounding Box Calculation
///
/// For each QuadTo(P0 -> P1 -> P2):
/// 1. Compute AABB of {P0, P1, P2}
/// 2. Expand by stroke_width/2 on all sides
/// 3. Generate 4 corner vertices with embedded curve data
///
/// ## Example
///
/// ```
/// use vsc_gpu::sdf_stroke::tessellate_stroke_segments;
/// use vsc_core::{PathCommand, Rational};
///
/// let commands = vec![
///     PathCommand::MoveTo {
///         x: Rational::from_int(0),
///         y: Rational::from_int(0),
///     },
///     PathCommand::QuadTo {
///         x1: Rational::from_int(50),
///         y1: Rational::from_int(100),
///         x: Rational::from_int(100),
///         y: Rational::from_int(0),
///     },
/// ];
///
/// let output = tessellate_stroke_segments(&commands, 4.0);
///
/// // One curve = 4 vertices, 6 indices
/// assert_eq!(output.vertices.len(), 4);
/// assert_eq!(output.indices.len(), 6);
/// assert_eq!(output.segment_count(), 1);
/// ```
pub fn tessellate_stroke_segments(commands: &[PathCommand], stroke_width: f32) -> SdfStrokeOutput {
    let half_width = stroke_width / 2.0;

    // Count QuadTo segments for capacity estimation
    let quad_count = commands
        .iter()
        .filter(|c| matches!(c, PathCommand::QuadTo { .. }))
        .count();

    let mut output = SdfStrokeOutput::with_capacity(quad_count);

    // Track current pen position
    let mut pen_x = 0.0f32;
    let mut pen_y = 0.0f32;
    let mut subpath_start_x = 0.0f32;
    let mut subpath_start_y = 0.0f32;

    for cmd in commands {
        match cmd {
            PathCommand::MoveTo { x, y } => {
                pen_x = x.to_f64_for_rasterization() as f32;
                pen_y = y.to_f64_for_rasterization() as f32;
                subpath_start_x = pen_x;
                subpath_start_y = pen_y;
            }

            PathCommand::LineTo { x, y } => {
                // LineTo doesn't generate SDF stroke geometry
                // (could be added later for line segment strokes)
                pen_x = x.to_f64_for_rasterization() as f32;
                pen_y = y.to_f64_for_rasterization() as f32;
            }

            PathCommand::QuadTo { x1, y1, x, y } => {
                let p0 = [pen_x, pen_y];
                let p1 = [
                    x1.to_f64_for_rasterization() as f32,
                    y1.to_f64_for_rasterization() as f32,
                ];
                let p2 = [
                    x.to_f64_for_rasterization() as f32,
                    y.to_f64_for_rasterization() as f32,
                ];

                // Generate bounding rectangle for this curve
                generate_curve_rectangle(&mut output, p0, p1, p2, half_width);

                // Update pen position
                pen_x = p2[0];
                pen_y = p2[1];
            }

            PathCommand::CubicTo { x, y, .. } => {
                // Cubic curves not handled in I-3 (deferred to I-2 integration)
                pen_x = x.to_f64_for_rasterization() as f32;
                pen_y = y.to_f64_for_rasterization() as f32;
            }

            PathCommand::ArcTo { x, y, .. } => {
                // Arc segments not handled (could be approximated)
                pen_x = x.to_f64_for_rasterization() as f32;
                pen_y = y.to_f64_for_rasterization() as f32;
            }

            PathCommand::Close => {
                pen_x = subpath_start_x;
                pen_y = subpath_start_y;
            }
        }
    }

    output
}

/// Generate a bounding rectangle for a single quadratic Bezier curve.
///
/// The rectangle is axis-aligned and contains all points within `half_width`
/// of the curve.
fn generate_curve_rectangle(
    output: &mut SdfStrokeOutput,
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    half_width: f32,
) {
    // Compute AABB of control points
    let min_x = p0[0].min(p1[0]).min(p2[0]);
    let max_x = p0[0].max(p1[0]).max(p2[0]);
    let min_y = p0[1].min(p1[1]).min(p2[1]);
    let max_y = p0[1].max(p1[1]).max(p2[1]);

    // Expand by half stroke width
    let rect_min_x = min_x - half_width;
    let rect_max_x = max_x + half_width;
    let rect_min_y = min_y - half_width;
    let rect_max_y = max_y + half_width;

    // Current vertex count (for index calculation)
    let base_index = output.vertices.len() as u32;

    // Generate 4 corner vertices
    // Vertex order: bottom-left, bottom-right, top-right, top-left (CCW)
    //
    //  3 --- 2
    //  |     |
    //  0 --- 1

    // Vertex 0: bottom-left
    output.vertices.push(SdfStrokeVertex::new(
        [rect_min_x, rect_min_y], // world position
        [rect_min_x, rect_min_y], // local position (same as world for now)
        p0,
        p1,
        p2,
        half_width,
    ));

    // Vertex 1: bottom-right
    output.vertices.push(SdfStrokeVertex::new(
        [rect_max_x, rect_min_y],
        [rect_max_x, rect_min_y],
        p0,
        p1,
        p2,
        half_width,
    ));

    // Vertex 2: top-right
    output.vertices.push(SdfStrokeVertex::new(
        [rect_max_x, rect_max_y],
        [rect_max_x, rect_max_y],
        p0,
        p1,
        p2,
        half_width,
    ));

    // Vertex 3: top-left
    output.vertices.push(SdfStrokeVertex::new(
        [rect_min_x, rect_max_y],
        [rect_min_x, rect_max_y],
        p0,
        p1,
        p2,
        half_width,
    ));

    // Generate 2 triangles (6 indices)
    // Triangle 1: 0, 1, 2
    // Triangle 2: 0, 2, 3
    output.indices.push(base_index);
    output.indices.push(base_index + 1);
    output.indices.push(base_index + 2);

    output.indices.push(base_index);
    output.indices.push(base_index + 2);
    output.indices.push(base_index + 3);
}

#[cfg(test)]
mod tests {
    use super::*;
    use vsc_core::Rational;

    fn r(val: i64) -> Rational {
        Rational::from_int(val)
    }

    #[test]
    fn test_empty_path() {
        let output = tessellate_stroke_segments(&[], 4.0);
        assert!(output.is_empty());
        assert_eq!(output.segment_count(), 0);
    }

    #[test]
    fn test_no_curves() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::LineTo { x: r(100), y: r(0) },
            PathCommand::Close,
        ];

        let output = tessellate_stroke_segments(&commands, 4.0);
        assert!(output.is_empty());
    }

    #[test]
    fn test_single_curve() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::QuadTo {
                x1: r(50),
                y1: r(100),
                x: r(100),
                y: r(0),
            },
        ];

        let output = tessellate_stroke_segments(&commands, 4.0);

        // 1 curve = 4 vertices, 6 indices
        assert_eq!(output.vertices.len(), 4);
        assert_eq!(output.indices.len(), 6);
        assert_eq!(output.segment_count(), 1);
    }

    #[test]
    fn test_multiple_curves() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::QuadTo {
                x1: r(25),
                y1: r(50),
                x: r(50),
                y: r(0),
            },
            PathCommand::QuadTo {
                x1: r(75),
                y1: r(50),
                x: r(100),
                y: r(0),
            },
        ];

        let output = tessellate_stroke_segments(&commands, 4.0);

        // 2 curves = 8 vertices, 12 indices
        assert_eq!(output.vertices.len(), 8);
        assert_eq!(output.indices.len(), 12);
        assert_eq!(output.segment_count(), 2);
    }

    #[test]
    fn test_bounding_box_expansion() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::QuadTo {
                x1: r(50),
                y1: r(100),
                x: r(100),
                y: r(0),
            },
        ];

        let stroke_width = 10.0;
        let half_width = stroke_width / 2.0;
        let output = tessellate_stroke_segments(&commands, stroke_width);

        // Control points: (0,0), (50,100), (100,0)
        // AABB: min=(0,0), max=(100,100)
        // Expanded: min=(-5,-5), max=(105,105)

        let v = &output.vertices;

        // Check bounding box expansion
        let min_x = v
            .iter()
            .map(|v| v.position[0])
            .fold(f32::INFINITY, f32::min);
        let max_x = v
            .iter()
            .map(|v| v.position[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = v
            .iter()
            .map(|v| v.position[1])
            .fold(f32::INFINITY, f32::min);
        let max_y = v
            .iter()
            .map(|v| v.position[1])
            .fold(f32::NEG_INFINITY, f32::max);

        assert!((min_x - (-half_width)).abs() < 0.001, "min_x should be -5");
        assert!(
            (max_x - (100.0 + half_width)).abs() < 0.001,
            "max_x should be 105"
        );
        assert!((min_y - (-half_width)).abs() < 0.001, "min_y should be -5");
        assert!(
            (max_y - (100.0 + half_width)).abs() < 0.001,
            "max_y should be 105"
        );
    }

    #[test]
    fn test_control_points_embedded() {
        let commands = vec![
            PathCommand::MoveTo { x: r(10), y: r(20) },
            PathCommand::QuadTo {
                x1: r(30),
                y1: r(40),
                x: r(50),
                y: r(60),
            },
        ];

        let output = tessellate_stroke_segments(&commands, 4.0);

        // All 4 vertices should have same control points
        for v in &output.vertices {
            assert_eq!(v.p0, [10.0, 20.0], "p0 should be start point");
            assert_eq!(v.p1, [30.0, 40.0], "p1 should be control point");
            assert_eq!(v.p2, [50.0, 60.0], "p2 should be end point");
            assert_eq!(v.half_width, 2.0, "half_width should be stroke_width/2");
        }
    }

    #[test]
    fn test_indices_valid() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::QuadTo {
                x1: r(50),
                y1: r(100),
                x: r(100),
                y: r(0),
            },
            PathCommand::QuadTo {
                x1: r(150),
                y1: r(100),
                x: r(200),
                y: r(0),
            },
        ];

        let output = tessellate_stroke_segments(&commands, 4.0);

        // Verify all indices are within vertex range
        let vertex_count = output.vertices.len() as u32;
        for &idx in &output.indices {
            assert!(
                idx < vertex_count,
                "Index {} out of range (max {})",
                idx,
                vertex_count - 1
            );
        }

        // Verify indices form complete triangles
        assert_eq!(
            output.indices.len() % 3,
            0,
            "Indices should form complete triangles"
        );

        // Verify first curve indices: 0, 1, 2, 0, 2, 3
        assert_eq!(output.indices[0..6], [0, 1, 2, 0, 2, 3]);

        // Verify second curve indices: 4, 5, 6, 4, 6, 7
        assert_eq!(output.indices[6..12], [4, 5, 6, 4, 6, 7]);
    }

    #[test]
    fn test_pen_tracking() {
        // Verify pen position is tracked correctly across segments
        let commands = vec![
            PathCommand::MoveTo {
                x: r(100),
                y: r(200),
            },
            PathCommand::LineTo {
                x: r(150),
                y: r(200),
            }, // Pen moves to (150, 200)
            PathCommand::QuadTo {
                x1: r(175),
                y1: r(250),
                x: r(200),
                y: r(200),
            },
        ];

        let output = tessellate_stroke_segments(&commands, 4.0);

        // The QuadTo should start from (150, 200), not (0, 0)
        assert_eq!(
            output.vertices[0].p0,
            [150.0, 200.0],
            "p0 should be pen position after LineTo"
        );
    }

    #[test]
    fn test_close_resets_pen() {
        let commands = vec![
            PathCommand::MoveTo { x: r(10), y: r(20) },
            PathCommand::LineTo {
                x: r(100),
                y: r(20),
            },
            PathCommand::Close, // Pen returns to (10, 20)
            PathCommand::QuadTo {
                x1: r(50),
                y1: r(70),
                x: r(100),
                y: r(20),
            },
        ];

        let output = tessellate_stroke_segments(&commands, 4.0);

        // QuadTo after Close should start from subpath start
        assert_eq!(
            output.vertices[0].p0,
            [10.0, 20.0],
            "p0 should be subpath start after Close"
        );
    }

    #[test]
    fn test_local_pos_equals_world_pos() {
        // In current implementation, local_pos equals world position
        // (transform is applied later by the renderer)
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::QuadTo {
                x1: r(50),
                y1: r(100),
                x: r(100),
                y: r(0),
            },
        ];

        let output = tessellate_stroke_segments(&commands, 4.0);

        for v in &output.vertices {
            assert_eq!(v.position, v.local_pos, "local_pos should equal position");
        }
    }
}
