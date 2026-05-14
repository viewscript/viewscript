//! Cubic SDF Stroke Tessellator
//!
//! Generates bounding rectangles for cubic Bezier curve strokes.
//! Each curve segment produces 4 vertices (1 rectangle = 2 triangles).

use super::cubic_vertex::CubicSdfStrokeVertex;
use vsc_core::PathCommand;

/// Output from cubic SDF stroke tessellation.
///
/// ## Current Limitations (Phase I-4)
///
/// The current implementation renders each curve segment independently:
///
/// - **line_cap**: Effectively `butt` only. No `round` or `square` cap support.
/// - **line_join**: Not implemented. Segments are rendered without explicit join geometry.
///
/// When adjacent segments share endpoints (P3 of segment N = P0 of segment N+1),
/// the SDF distance fields naturally overlap at the junction, producing acceptable
/// visual results for most cases. However, sharp angles may show minor artifacts.
#[derive(Debug, Clone)]
pub struct CubicSdfStrokeOutput {
    /// Vertices for bounding rectangles.
    /// Each curve produces 4 vertices.
    pub vertices: Vec<CubicSdfStrokeVertex>,

    /// Indices for triangles.
    /// Each curve produces 6 indices (2 triangles).
    pub indices: Vec<u32>,
}

impl CubicSdfStrokeOutput {
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

impl Default for CubicSdfStrokeOutput {
    fn default() -> Self {
        Self::new()
    }
}

/// Tessellate cubic Bezier strokes from path commands.
///
/// For each `CubicTo` segment, generates a bounding rectangle that encompasses
/// the curve plus stroke width. The rectangle vertices contain all curve
/// parameters needed for SDF evaluation in the fragment shader.
///
/// ## Parameters
///
/// - `commands`: Path commands (only `CubicTo` segments are processed)
/// - `stroke_width`: Total stroke width (half is added to each side)
///
/// ## Output
///
/// `CubicSdfStrokeOutput` containing:
/// - `vertices`: 4 vertices per curve (rectangle corners)
/// - `indices`: 6 indices per curve (2 triangles)
///
/// ## Bounding Box Calculation
///
/// For each CubicTo(P0 -> P1 -> P2 -> P3):
/// 1. Compute AABB of {P0, P1, P2, P3}
/// 2. Expand by stroke_width/2 on all sides
/// 3. Generate 4 corner vertices with embedded curve data
///
/// ## Example
///
/// ```
/// use vsc_gpu::sdf_stroke::tessellate_cubic_stroke_segments;
/// use vsc_core::{PathCommand, Rational};
///
/// let commands = vec![
///     PathCommand::MoveTo {
///         x: Rational::from_int(0),
///         y: Rational::from_int(0),
///     },
///     PathCommand::CubicTo {
///         x1: Rational::from_int(33),
///         y1: Rational::from_int(100),
///         x2: Rational::from_int(66),
///         y2: Rational::from_int(100),
///         x: Rational::from_int(100),
///         y: Rational::from_int(0),
///     },
/// ];
///
/// let output = tessellate_cubic_stroke_segments(&commands, 4.0);
///
/// // One curve = 4 vertices, 6 indices
/// assert_eq!(output.vertices.len(), 4);
/// assert_eq!(output.indices.len(), 6);
/// assert_eq!(output.segment_count(), 1);
/// ```
pub fn tessellate_cubic_stroke_segments(
    commands: &[PathCommand],
    stroke_width: f32,
) -> CubicSdfStrokeOutput {
    let half_width = stroke_width / 2.0;

    // Count CubicTo segments for capacity estimation
    let cubic_count = commands
        .iter()
        .filter(|c| matches!(c, PathCommand::CubicTo { .. }))
        .count();

    let mut output = CubicSdfStrokeOutput::with_capacity(cubic_count);

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
                pen_x = x.to_f64_for_rasterization() as f32;
                pen_y = y.to_f64_for_rasterization() as f32;
            }

            PathCommand::QuadTo { x, y, .. } => {
                // Quadratic curves handled by quadratic tessellator
                pen_x = x.to_f64_for_rasterization() as f32;
                pen_y = y.to_f64_for_rasterization() as f32;
            }

            PathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let p0 = [pen_x, pen_y];
                let p1 = [
                    x1.to_f64_for_rasterization() as f32,
                    y1.to_f64_for_rasterization() as f32,
                ];
                let p2 = [
                    x2.to_f64_for_rasterization() as f32,
                    y2.to_f64_for_rasterization() as f32,
                ];
                let p3 = [
                    x.to_f64_for_rasterization() as f32,
                    y.to_f64_for_rasterization() as f32,
                ];

                // Generate bounding rectangle for this curve
                generate_cubic_rectangle(&mut output, p0, p1, p2, p3, half_width);

                // Update pen position
                pen_x = p3[0];
                pen_y = p3[1];
            }

            PathCommand::ArcTo { x, y, .. } => {
                // Arc segments not handled
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

/// Generate a bounding rectangle for a single cubic Bezier curve.
///
/// The rectangle is axis-aligned and contains all points within `half_width`
/// of the curve.
fn generate_cubic_rectangle(
    output: &mut CubicSdfStrokeOutput,
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    p3: [f32; 2],
    half_width: f32,
) {
    // Compute AABB of all 4 control points
    let min_x = p0[0].min(p1[0]).min(p2[0]).min(p3[0]);
    let max_x = p0[0].max(p1[0]).max(p2[0]).max(p3[0]);
    let min_y = p0[1].min(p1[1]).min(p2[1]).min(p3[1]);
    let max_y = p0[1].max(p1[1]).max(p2[1]).max(p3[1]);

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
    output.vertices.push(CubicSdfStrokeVertex::new(
        [rect_min_x, rect_min_y], // world position
        [rect_min_x, rect_min_y], // local position
        p0,
        p1,
        p2,
        p3,
        half_width,
    ));

    // Vertex 1: bottom-right
    output.vertices.push(CubicSdfStrokeVertex::new(
        [rect_max_x, rect_min_y],
        [rect_max_x, rect_min_y],
        p0,
        p1,
        p2,
        p3,
        half_width,
    ));

    // Vertex 2: top-right
    output.vertices.push(CubicSdfStrokeVertex::new(
        [rect_max_x, rect_max_y],
        [rect_max_x, rect_max_y],
        p0,
        p1,
        p2,
        p3,
        half_width,
    ));

    // Vertex 3: top-left
    output.vertices.push(CubicSdfStrokeVertex::new(
        [rect_min_x, rect_max_y],
        [rect_min_x, rect_max_y],
        p0,
        p1,
        p2,
        p3,
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
        let output = tessellate_cubic_stroke_segments(&[], 4.0);
        assert!(output.is_empty());
        assert_eq!(output.segment_count(), 0);
    }

    #[test]
    fn test_no_cubics() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::LineTo { x: r(100), y: r(0) },
            PathCommand::QuadTo {
                x1: r(50),
                y1: r(50),
                x: r(100),
                y: r(100),
            },
            PathCommand::Close,
        ];

        let output = tessellate_cubic_stroke_segments(&commands, 4.0);
        assert!(output.is_empty());
    }

    #[test]
    fn test_single_cubic() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(33),
                y1: r(100),
                x2: r(66),
                y2: r(100),
                x: r(100),
                y: r(0),
            },
        ];

        let output = tessellate_cubic_stroke_segments(&commands, 4.0);

        // 1 curve = 4 vertices, 6 indices
        assert_eq!(output.vertices.len(), 4);
        assert_eq!(output.indices.len(), 6);
        assert_eq!(output.segment_count(), 1);
    }

    #[test]
    fn test_multiple_cubics() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(10),
                y1: r(30),
                x2: r(40),
                y2: r(30),
                x: r(50),
                y: r(0),
            },
            PathCommand::CubicTo {
                x1: r(60),
                y1: r(30),
                x2: r(90),
                y2: r(30),
                x: r(100),
                y: r(0),
            },
        ];

        let output = tessellate_cubic_stroke_segments(&commands, 4.0);

        // 2 curves = 8 vertices, 12 indices
        assert_eq!(output.vertices.len(), 8);
        assert_eq!(output.indices.len(), 12);
        assert_eq!(output.segment_count(), 2);
    }

    #[test]
    fn test_bounding_box_expansion() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(25),
                y1: r(100),
                x2: r(75),
                y2: r(100),
                x: r(100),
                y: r(0),
            },
        ];

        let stroke_width = 10.0;
        let half_width = stroke_width / 2.0;
        let output = tessellate_cubic_stroke_segments(&commands, stroke_width);

        // Control points: (0,0), (25,100), (75,100), (100,0)
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

        assert!(
            (min_x - (-half_width)).abs() < 0.001,
            "min_x should be -5, got {}",
            min_x
        );
        assert!(
            (max_x - (100.0 + half_width)).abs() < 0.001,
            "max_x should be 105, got {}",
            max_x
        );
        assert!(
            (min_y - (-half_width)).abs() < 0.001,
            "min_y should be -5, got {}",
            min_y
        );
        assert!(
            (max_y - (100.0 + half_width)).abs() < 0.001,
            "max_y should be 105, got {}",
            max_y
        );
    }

    #[test]
    fn test_control_points_embedded() {
        let commands = vec![
            PathCommand::MoveTo { x: r(10), y: r(20) },
            PathCommand::CubicTo {
                x1: r(30),
                y1: r(40),
                x2: r(50),
                y2: r(60),
                x: r(70),
                y: r(80),
            },
        ];

        let output = tessellate_cubic_stroke_segments(&commands, 4.0);

        // All 4 vertices should have same control points
        for v in &output.vertices {
            assert_eq!(v.p0, [10.0, 20.0], "p0 should be start point");
            assert_eq!(v.p1, [30.0, 40.0], "p1 should be control point 1");
            assert_eq!(v.p2, [50.0, 60.0], "p2 should be control point 2");
            assert_eq!(v.p3, [70.0, 80.0], "p3 should be end point");
            assert_eq!(v.half_width, 2.0, "half_width should be stroke_width/2");
        }
    }

    #[test]
    fn test_indices_valid() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(25),
                y1: r(100),
                x2: r(75),
                y2: r(100),
                x: r(100),
                y: r(0),
            },
            PathCommand::CubicTo {
                x1: r(125),
                y1: r(100),
                x2: r(175),
                y2: r(100),
                x: r(200),
                y: r(0),
            },
        ];

        let output = tessellate_cubic_stroke_segments(&commands, 4.0);

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
            PathCommand::CubicTo {
                x1: r(160),
                y1: r(230),
                x2: r(190),
                y2: r(230),
                x: r(200),
                y: r(200),
            },
        ];

        let output = tessellate_cubic_stroke_segments(&commands, 4.0);

        // The CubicTo should start from (150, 200), not (0, 0)
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
            PathCommand::CubicTo {
                x1: r(30),
                y1: r(50),
                x2: r(70),
                y2: r(50),
                x: r(100),
                y: r(20),
            },
        ];

        let output = tessellate_cubic_stroke_segments(&commands, 4.0);

        // CubicTo after Close should start from subpath start
        assert_eq!(
            output.vertices[0].p0,
            [10.0, 20.0],
            "p0 should be subpath start after Close"
        );
    }

    #[test]
    fn test_local_pos_equals_world_pos() {
        // In current implementation, local_pos equals world position
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(33),
                y1: r(100),
                x2: r(66),
                y2: r(100),
                x: r(100),
                y: r(0),
            },
        ];

        let output = tessellate_cubic_stroke_segments(&commands, 4.0);

        for v in &output.vertices {
            assert_eq!(v.position, v.local_pos, "local_pos should equal position");
        }
    }

    #[test]
    fn test_vertex_size() {
        // Verify cubic vertex is 52 bytes (13 * f32)
        assert_eq!(std::mem::size_of::<CubicSdfStrokeVertex>(), 52);
    }
}
