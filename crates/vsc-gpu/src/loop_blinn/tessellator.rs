//! Loop-Blinn Curve Tessellator
//!
//! Converts PathCommand sequences into Loop-Blinn vertices for quadratic
//! Bezier curves, while collecting non-curve segments for interior fill.

use super::convexity::compute_curve_sign;
use super::cubic::classify_cubic;
use super::vertex::{CubicLoopBlinnVertex, LoopBlinnVertex};
use vsc_core::PathCommand;

/// Output from Loop-Blinn tessellation.
///
/// Contains both the curve triangles (for Loop-Blinn rendering) and the
/// remaining path commands (for interior fill via lyon).
#[derive(Debug, Clone)]
pub struct LoopBlinnOutput {
    /// Vertices for Loop-Blinn curve triangles.
    /// Each curve produces 3 vertices (one triangle).
    pub vertices: Vec<LoopBlinnVertex>,

    /// Indices for Loop-Blinn triangles.
    /// Each curve produces 3 indices.
    pub indices: Vec<u32>,

    /// Path commands for interior fill tessellation.
    /// Contains all non-curve segments plus collinear curves converted to lines.
    pub interior_commands: Vec<PathCommand>,
}

impl LoopBlinnOutput {
    /// Create an empty output.
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            interior_commands: Vec::new(),
        }
    }

    /// Create with pre-allocated capacity.
    ///
    /// Estimates based on typical glyph composition:
    /// - ~60% of segments are quadratic curves
    /// - ~40% are lines or degenerate curves
    pub fn with_capacity(segment_count: usize) -> Self {
        let curve_estimate = segment_count * 6 / 10; // 60% curves
        Self {
            vertices: Vec::with_capacity(curve_estimate * 3),
            indices: Vec::with_capacity(curve_estimate * 3),
            interior_commands: Vec::with_capacity(segment_count),
        }
    }

    /// Number of Loop-Blinn triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

impl Default for LoopBlinnOutput {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Cubic Loop-Blinn Output (I-2)
// =============================================================================

/// Output from cubic Loop-Blinn tessellation.
///
/// Contains both the curve triangles (for Loop-Blinn rendering) and the
/// remaining path commands (for interior fill via lyon).
#[derive(Debug, Clone)]
pub struct CubicLoopBlinnOutput {
    /// Vertices for cubic Loop-Blinn curve triangles.
    /// Each curve produces 4 vertices (two triangles covering the control polygon).
    pub vertices: Vec<CubicLoopBlinnVertex>,

    /// Indices for cubic Loop-Blinn triangles.
    /// Each curve produces 6 indices (two triangles).
    pub indices: Vec<u32>,

    /// Path commands for interior fill tessellation.
    /// Contains all non-curve segments plus CubicTo commands converted to lines.
    pub interior_commands: Vec<PathCommand>,
}

impl CubicLoopBlinnOutput {
    /// Create an empty output.
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            interior_commands: Vec::new(),
        }
    }

    /// Create with pre-allocated capacity.
    pub fn with_capacity(segment_count: usize) -> Self {
        let curve_estimate = segment_count * 6 / 10; // 60% curves
        Self {
            vertices: Vec::with_capacity(curve_estimate * 4),
            indices: Vec::with_capacity(curve_estimate * 6),
            interior_commands: Vec::with_capacity(segment_count),
        }
    }

    /// Number of Loop-Blinn triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

impl Default for CubicLoopBlinnOutput {
    fn default() -> Self {
        Self::new()
    }
}

/// Tessellate quadratic Bezier curves from a path command sequence.
///
/// This function processes a path and separates it into:
/// 1. **Loop-Blinn curve triangles**: Each `QuadTo` command that passes
///    convexity check becomes a single triangle.
/// 2. **Interior path commands**: All other commands (MoveTo, LineTo, Close)
///    plus collinear QuadTo commands (converted to LineTo).
///
/// ## Input
///
/// A slice of `PathCommand` representing a closed or open path.
///
/// ## Output
///
/// `LoopBlinnOutput` containing:
/// - `vertices` / `indices`: GPU-ready buffers for Loop-Blinn curve rendering
/// - `interior_commands`: Path commands for lyon interior fill tessellation
///
/// ## Algorithm
///
/// 1. Track current pen position across commands
/// 2. For each `QuadTo`:
///    - Compute curve sign via cross product
///    - If collinear (None): convert to LineTo for interior path
///    - If valid curve: generate 3 vertices with fixed UV assignment
/// 3. All non-QuadTo commands pass through to interior path
///
/// ## Example
///
/// ```
/// use vsc_gpu::loop_blinn::tessellate_quadratic_beziers;
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
///     PathCommand::Close,
/// ];
///
/// let output = tessellate_quadratic_beziers(&commands);
///
/// // One curve triangle
/// assert_eq!(output.triangle_count(), 1);
/// assert_eq!(output.vertices.len(), 3);
/// assert_eq!(output.indices.len(), 3);
///
/// // Interior path has MoveTo, LineTo (replacement), Close
/// assert_eq!(output.interior_commands.len(), 3);
/// ```
pub fn tessellate_quadratic_beziers(commands: &[PathCommand]) -> LoopBlinnOutput {
    let mut output = LoopBlinnOutput::with_capacity(commands.len());

    // Current pen position (for QuadTo start point)
    let mut pen_x = 0.0f32;
    let mut pen_y = 0.0f32;

    // Subpath start (for Close command)
    let mut subpath_start_x = 0.0f32;
    let mut subpath_start_y = 0.0f32;

    for cmd in commands {
        match cmd {
            PathCommand::MoveTo { x, y } => {
                let fx = x.to_f64_for_rasterization() as f32;
                let fy = y.to_f64_for_rasterization() as f32;
                pen_x = fx;
                pen_y = fy;
                subpath_start_x = fx;
                subpath_start_y = fy;

                output.interior_commands.push(cmd.clone());
            }

            PathCommand::LineTo { x, y } => {
                let fx = x.to_f64_for_rasterization() as f32;
                let fy = y.to_f64_for_rasterization() as f32;
                pen_x = fx;
                pen_y = fy;

                output.interior_commands.push(cmd.clone());
            }

            PathCommand::QuadTo { x1, y1, x, y } => {
                // Convert control points to f32
                let p0 = [pen_x, pen_y];
                let p1 = [
                    x1.to_f64_for_rasterization() as f32,
                    y1.to_f64_for_rasterization() as f32,
                ];
                let p2 = [
                    x.to_f64_for_rasterization() as f32,
                    y.to_f64_for_rasterization() as f32,
                ];

                // Check convexity
                match compute_curve_sign(p0, p1, p2) {
                    Some(curve_sign) => {
                        // Valid curve: generate Loop-Blinn triangle
                        let base_index = output.vertices.len() as u32;
                        let vertices = LoopBlinnVertex::from_quadratic(p0, p1, p2, curve_sign);

                        output.vertices.extend_from_slice(&vertices);
                        output.indices.push(base_index);
                        output.indices.push(base_index + 1);
                        output.indices.push(base_index + 2);

                        // Add LineTo for interior path (connects P0 to P2)
                        output.interior_commands.push(PathCommand::LineTo {
                            x: x.clone(),
                            y: y.clone(),
                        });
                    }
                    None => {
                        // Collinear: treat as line segment
                        output.interior_commands.push(PathCommand::LineTo {
                            x: x.clone(),
                            y: y.clone(),
                        });
                    }
                }

                // Update pen position
                pen_x = p2[0];
                pen_y = p2[1];
            }

            PathCommand::CubicTo { x, y, .. } => {
                // Cubic curves are passed through for Phase I-2
                // For now, add as-is to interior (lyon will handle)
                let fx = x.to_f64_for_rasterization() as f32;
                let fy = y.to_f64_for_rasterization() as f32;
                pen_x = fx;
                pen_y = fy;

                output.interior_commands.push(cmd.clone());
            }

            PathCommand::ArcTo { x, y, .. } => {
                // Arc commands pass through to interior
                let fx = x.to_f64_for_rasterization() as f32;
                let fy = y.to_f64_for_rasterization() as f32;
                pen_x = fx;
                pen_y = fy;

                output.interior_commands.push(cmd.clone());
            }

            PathCommand::Close => {
                // Close returns pen to subpath start
                pen_x = subpath_start_x;
                pen_y = subpath_start_y;

                output.interior_commands.push(cmd.clone());
            }
        }
    }

    output
}

// =============================================================================
// Cubic Bezier Tessellation (I-2)
// =============================================================================

/// Tessellate cubic Bezier curves from a path command sequence.
///
/// This function processes a path and separates it into:
/// 1. **Loop-Blinn curve triangles**: Each `CubicTo` command that passes
///    classification becomes two triangles (4 vertices, 6 indices).
/// 2. **Interior path commands**: All other commands (MoveTo, LineTo, Close)
///    plus CubicTo commands converted to LineTo.
///
/// ## Input
///
/// A slice of `PathCommand` representing a closed or open path.
///
/// ## Output
///
/// `CubicLoopBlinnOutput` containing:
/// - `vertices` / `indices`: GPU-ready buffers for Loop-Blinn cubic rendering
/// - `interior_commands`: Path commands for lyon interior fill tessellation
///
/// ## Algorithm
///
/// 1. Track current pen position across commands
/// 2. For each `CubicTo`:
///    - Classify curve via `classify_cubic()` (Serpentine, Cusp, Loop, Degenerate)
///    - If Degenerate: convert to LineTo for interior path
///    - Otherwise: generate 4 vertices with (k, l, m) texture coordinates
/// 3. All non-CubicTo commands pass through to interior path
pub fn tessellate_cubic_beziers(commands: &[PathCommand]) -> CubicLoopBlinnOutput {
    let mut output = CubicLoopBlinnOutput::with_capacity(commands.len());

    // Current pen position (for CubicTo start point)
    let mut pen_x = 0.0f32;
    let mut pen_y = 0.0f32;

    // Subpath start (for Close command)
    let mut subpath_start_x = 0.0f32;
    let mut subpath_start_y = 0.0f32;

    for cmd in commands {
        match cmd {
            PathCommand::MoveTo { x, y } => {
                let fx = x.to_f64_for_rasterization() as f32;
                let fy = y.to_f64_for_rasterization() as f32;
                pen_x = fx;
                pen_y = fy;
                subpath_start_x = fx;
                subpath_start_y = fy;

                output.interior_commands.push(cmd.clone());
            }

            PathCommand::LineTo { x, y } => {
                let fx = x.to_f64_for_rasterization() as f32;
                let fy = y.to_f64_for_rasterization() as f32;
                pen_x = fx;
                pen_y = fy;

                output.interior_commands.push(cmd.clone());
            }

            PathCommand::QuadTo { x, y, .. } => {
                // QuadTo commands pass through to interior (handled by I-1)
                let fx = x.to_f64_for_rasterization() as f32;
                let fy = y.to_f64_for_rasterization() as f32;
                pen_x = fx;
                pen_y = fy;

                output.interior_commands.push(cmd.clone());
            }

            PathCommand::CubicTo { x1, y1, x2, y2, x, y } => {
                // Convert control points to f32
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

                // Classify the cubic curve
                let classification = classify_cubic(p0, p1, p2, p3);

                match classification.curve_type {
                    super::cubic::CubicCurveType::Degenerate => {
                        // Degenerate: treat as line segment
                        output.interior_commands.push(PathCommand::LineTo {
                            x: x.clone(),
                            y: y.clone(),
                        });
                    }
                    _ => {
                        // Valid curve: generate Loop-Blinn triangles (2 triangles, 4 vertices)
                        let base_index = output.vertices.len() as u32;
                        let vertices = CubicLoopBlinnVertex::from_cubic(
                            p0, p1, p2, p3,
                            &classification.klm,
                            classification.curve_sign,
                        );

                        output.vertices.extend_from_slice(&vertices);

                        // Two triangles covering the control polygon:
                        // Triangle 1: P0, P1, P2
                        // Triangle 2: P0, P2, P3
                        output.indices.push(base_index);
                        output.indices.push(base_index + 1);
                        output.indices.push(base_index + 2);

                        output.indices.push(base_index);
                        output.indices.push(base_index + 2);
                        output.indices.push(base_index + 3);

                        // Add LineTo for interior path (connects P0 to P3)
                        output.interior_commands.push(PathCommand::LineTo {
                            x: x.clone(),
                            y: y.clone(),
                        });
                    }
                }

                // Update pen position
                pen_x = p3[0];
                pen_y = p3[1];
            }

            PathCommand::ArcTo { x, y, .. } => {
                // Arc commands pass through to interior
                let fx = x.to_f64_for_rasterization() as f32;
                let fy = y.to_f64_for_rasterization() as f32;
                pen_x = fx;
                pen_y = fy;

                output.interior_commands.push(cmd.clone());
            }

            PathCommand::Close => {
                // Close returns pen to subpath start
                pen_x = subpath_start_x;
                pen_y = subpath_start_y;

                output.interior_commands.push(cmd.clone());
            }
        }
    }

    output
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
        let output = tessellate_quadratic_beziers(&[]);

        assert_eq!(output.vertices.len(), 0);
        assert_eq!(output.indices.len(), 0);
        assert_eq!(output.interior_commands.len(), 0);
    }

    #[test]
    fn test_line_only_path() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::LineTo {
                x: r(100),
                y: r(0),
            },
            PathCommand::LineTo {
                x: r(100),
                y: r(100),
            },
            PathCommand::Close,
        ];

        let output = tessellate_quadratic_beziers(&commands);

        // No curves
        assert_eq!(output.triangle_count(), 0);
        assert_eq!(output.vertices.len(), 0);

        // All commands pass through
        assert_eq!(output.interior_commands.len(), 4);
    }

    #[test]
    fn test_single_convex_curve() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::QuadTo {
                x1: r(50),
                y1: r(100),
                x: r(100),
                y: r(0),
            },
            PathCommand::Close,
        ];

        let output = tessellate_quadratic_beziers(&commands);

        // One curve triangle
        assert_eq!(output.triangle_count(), 1);
        assert_eq!(output.vertices.len(), 3);
        assert_eq!(output.indices.len(), 3);

        // Verify curve sign is positive (convex)
        assert_eq!(output.vertices[0].curve_sign, 1.0);

        // Interior has MoveTo, LineTo (from QuadTo), Close
        assert_eq!(output.interior_commands.len(), 3);

        // Verify the QuadTo was replaced with LineTo in interior
        match &output.interior_commands[1] {
            PathCommand::LineTo { x, y } => {
                assert_eq!(*x, r(100));
                assert_eq!(*y, r(0));
            }
            _ => panic!("Expected LineTo"),
        }
    }

    #[test]
    fn test_single_concave_curve() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::QuadTo {
                x1: r(50),
                y1: r(-100), // Control point below baseline
                x: r(100),
                y: r(0),
            },
            PathCommand::Close,
        ];

        let output = tessellate_quadratic_beziers(&commands);

        // One curve triangle
        assert_eq!(output.triangle_count(), 1);

        // Verify curve sign is negative (concave)
        assert_eq!(output.vertices[0].curve_sign, -1.0);
    }

    #[test]
    fn test_collinear_curve_becomes_line() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::QuadTo {
                x1: r(50),
                y1: r(0), // Control point on the baseline (collinear)
                x: r(100),
                y: r(0),
            },
            PathCommand::Close,
        ];

        let output = tessellate_quadratic_beziers(&commands);

        // No curve triangles (collinear)
        assert_eq!(output.triangle_count(), 0);
        assert_eq!(output.vertices.len(), 0);

        // Interior has MoveTo, LineTo, Close
        assert_eq!(output.interior_commands.len(), 3);
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
                y1: r(-50), // Concave
                x: r(100),
                y: r(0),
            },
            PathCommand::Close,
        ];

        let output = tessellate_quadratic_beziers(&commands);

        // Two curve triangles
        assert_eq!(output.triangle_count(), 2);
        assert_eq!(output.vertices.len(), 6);
        assert_eq!(output.indices.len(), 6);

        // First curve is convex, second is concave
        assert_eq!(output.vertices[0].curve_sign, 1.0);
        assert_eq!(output.vertices[3].curve_sign, -1.0);
    }

    #[test]
    fn test_texture_coordinates() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::QuadTo {
                x1: r(50),
                y1: r(100),
                x: r(100),
                y: r(0),
            },
        ];

        let output = tessellate_quadratic_beziers(&commands);

        // Verify fixed UV assignment
        assert_eq!(output.vertices[0].curve_uv, [0.0, 0.0]); // P0
        assert_eq!(output.vertices[1].curve_uv, [0.5, 0.0]); // P1
        assert_eq!(output.vertices[2].curve_uv, [1.0, 1.0]); // P2
    }

    #[test]
    fn test_cubic_passes_through() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(25),
                y1: r(50),
                x2: r(75),
                y2: r(50),
                x: r(100),
                y: r(0),
            },
            PathCommand::Close,
        ];

        let output = tessellate_quadratic_beziers(&commands);

        // No Loop-Blinn triangles (cubic not supported in I-1)
        assert_eq!(output.triangle_count(), 0);

        // All commands pass through to interior
        assert_eq!(output.interior_commands.len(), 3);
        matches!(output.interior_commands[1], PathCommand::CubicTo { .. });
    }

    #[test]
    fn test_indices_are_sequential() {
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

        let output = tessellate_quadratic_beziers(&commands);

        // Verify sequential indices
        assert_eq!(output.indices, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_pen_tracking_across_commands() {
        // Test that pen position is correctly tracked for curves
        // that don't start at origin
        let commands = vec![
            PathCommand::MoveTo {
                x: r(100),
                y: r(100),
            },
            PathCommand::LineTo {
                x: r(200),
                y: r(100),
            },
            PathCommand::QuadTo {
                x1: r(250),
                y1: r(150),
                x: r(300),
                y: r(100),
            },
        ];

        let output = tessellate_quadratic_beziers(&commands);

        // Verify the curve starts at (200, 100)
        assert_eq!(output.vertices[0].position, [200.0, 100.0]);
        assert_eq!(output.vertices[1].position, [250.0, 150.0]);
        assert_eq!(output.vertices[2].position, [300.0, 100.0]);
    }

    #[test]
    fn test_subpath_tracking_with_close() {
        let commands = vec![
            PathCommand::MoveTo {
                x: r(10),
                y: r(20),
            },
            PathCommand::LineTo {
                x: r(100),
                y: r(20),
            },
            PathCommand::Close,
            // After Close, pen should be back at (10, 20)
            PathCommand::QuadTo {
                x1: r(60),
                y1: r(70), // Control above pen
                x: r(110),
                y: r(20),
            },
        ];

        let output = tessellate_quadratic_beziers(&commands);

        // The QuadTo should start from (10, 20) after Close
        assert_eq!(output.vertices[0].position, [10.0, 20.0]);
    }

    // =========================================================================
    // Cubic Loop-Blinn Tessellation Tests (I-2)
    // =========================================================================

    #[test]
    fn test_cubic_empty_path() {
        let output = tessellate_cubic_beziers(&[]);

        assert_eq!(output.vertices.len(), 0);
        assert_eq!(output.indices.len(), 0);
        assert_eq!(output.interior_commands.len(), 0);
    }

    #[test]
    fn test_cubic_line_only_path() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::LineTo { x: r(100), y: r(0) },
            PathCommand::LineTo { x: r(100), y: r(100) },
            PathCommand::Close,
        ];

        let output = tessellate_cubic_beziers(&commands);

        // No curves
        assert_eq!(output.triangle_count(), 0);
        assert_eq!(output.vertices.len(), 0);

        // All commands pass through
        assert_eq!(output.interior_commands.len(), 4);
    }

    #[test]
    fn test_cubic_single_curve_serpentine() {
        // S-curve: classic serpentine classification
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(100),
                y1: r(100),
                x2: r(0),
                y2: r(200),
                x: r(100),
                y: r(300),
            },
            PathCommand::Close,
        ];

        let output = tessellate_cubic_beziers(&commands);

        // One cubic curve = 2 triangles, 4 vertices, 6 indices
        assert_eq!(output.triangle_count(), 2);
        assert_eq!(output.vertices.len(), 4);
        assert_eq!(output.indices.len(), 6);

        // Verify curve sign is non-zero
        assert!(output.vertices[0].curve_sign.abs() == 1.0);

        // Interior has MoveTo, LineTo (from CubicTo), Close
        assert_eq!(output.interior_commands.len(), 3);

        // Verify the CubicTo was replaced with LineTo in interior
        match &output.interior_commands[1] {
            PathCommand::LineTo { x, y } => {
                assert_eq!(*x, r(100));
                assert_eq!(*y, r(300));
            }
            _ => panic!("Expected LineTo"),
        }
    }

    #[test]
    fn test_cubic_degenerate_collinear() {
        // All points collinear: degenerate case, should be treated as line
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(10),
                y1: r(10),
                x2: r(20),
                y2: r(20),
                x: r(30),
                y: r(30),
            },
            PathCommand::Close,
        ];

        let output = tessellate_cubic_beziers(&commands);

        // Degenerate: no curve triangles
        assert_eq!(output.triangle_count(), 0);
        assert_eq!(output.vertices.len(), 0);

        // Interior has MoveTo, LineTo, Close
        assert_eq!(output.interior_commands.len(), 3);
    }

    #[test]
    fn test_cubic_multiple_curves() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(50),
                y1: r(100),
                x2: r(150),
                y2: r(100),
                x: r(200),
                y: r(0),
            },
            PathCommand::CubicTo {
                x1: r(250),
                y1: r(-100), // Control points below
                x2: r(350),
                y2: r(-100),
                x: r(400),
                y: r(0),
            },
            PathCommand::Close,
        ];

        let output = tessellate_cubic_beziers(&commands);

        // Two cubic curves = 4 triangles, 8 vertices, 12 indices
        assert_eq!(output.triangle_count(), 4);
        assert_eq!(output.vertices.len(), 8);
        assert_eq!(output.indices.len(), 12);
    }

    #[test]
    fn test_cubic_vertices_have_klm_coords() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(50),
                y1: r(100),
                x2: r(150),
                y2: r(100),
                x: r(200),
                y: r(0),
            },
        ];

        let output = tessellate_cubic_beziers(&commands);

        // Verify vertices have finite (k, l, m) texture coordinates
        for vertex in &output.vertices {
            assert!(
                vertex.curve_klm[0].is_finite(),
                "k should be finite"
            );
            assert!(
                vertex.curve_klm[1].is_finite(),
                "l should be finite"
            );
            assert!(
                vertex.curve_klm[2].is_finite(),
                "m should be finite"
            );
        }
    }

    #[test]
    fn test_cubic_indices_are_sequential() {
        let commands = vec![
            PathCommand::MoveTo { x: r(0), y: r(0) },
            PathCommand::CubicTo {
                x1: r(50),
                y1: r(100),
                x2: r(150),
                y2: r(100),
                x: r(200),
                y: r(0),
            },
            PathCommand::CubicTo {
                x1: r(250),
                y1: r(100),
                x2: r(350),
                y2: r(100),
                x: r(400),
                y: r(0),
            },
        ];

        let output = tessellate_cubic_beziers(&commands);

        // Verify indices form valid triangles
        // First curve: (0,1,2), (0,2,3)
        // Second curve: (4,5,6), (4,6,7)
        assert_eq!(
            output.indices,
            vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]
        );
    }

    #[test]
    fn test_cubic_pen_tracking() {
        // Test that pen position is correctly tracked for cubics
        let commands = vec![
            PathCommand::MoveTo { x: r(100), y: r(100) },
            PathCommand::LineTo { x: r(200), y: r(100) },
            PathCommand::CubicTo {
                x1: r(250),
                y1: r(150),
                x2: r(350),
                y2: r(150),
                x: r(400),
                y: r(100),
            },
        ];

        let output = tessellate_cubic_beziers(&commands);

        // Verify the cubic starts at (200, 100)
        assert_eq!(output.vertices[0].position, [200.0, 100.0]);
        assert_eq!(output.vertices[1].position, [250.0, 150.0]);
        assert_eq!(output.vertices[2].position, [350.0, 150.0]);
        assert_eq!(output.vertices[3].position, [400.0, 100.0]);
    }

    #[test]
    fn test_cubic_vertex_stride() {
        // CubicLoopBlinnVertex must be 24 bytes (6 x f32)
        assert_eq!(
            std::mem::size_of::<CubicLoopBlinnVertex>(),
            24,
            "CubicLoopBlinnVertex must be 24 bytes"
        );
    }
}
