//! Loop-Blinn Vertex Definition
//!
//! Defines the vertex format for Loop-Blinn curve triangles, separate from
//! the standard `GpuVertex` used for interior fill triangles.

/// GPU vertex for Loop-Blinn curve rendering.
///
/// Each quadratic Bezier curve is rendered as a single triangle with three
/// vertices at the control points P₀, P₁, P₂. The fragment shader evaluates
/// the implicit curve equation u² - v to determine inside/outside.
///
/// ## Memory Layout
///
/// Total size: 20 bytes (5 × f32), 4-byte aligned.
///
/// ```text
/// Offset  Size  Field
/// 0       8     position [f32; 2]
/// 8       8     curve_uv [f32; 2]
/// 16      4     curve_sign f32
/// ```
///
/// ## Texture Coordinate Assignment
///
/// For a quadratic Bezier with control points P₀, P₁, P₂:
/// - P₀: curve_uv = (0, 0)
/// - P₁: curve_uv = (0.5, 0)
/// - P₂: curve_uv = (1, 1)
///
/// The implicit function f(u,v) = u² - v equals zero exactly on the curve.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LoopBlinnVertex {
    /// Position in device pixels (post-rasterization).
    pub position: [f32; 2],

    /// Loop-Blinn texture coordinates for implicit function evaluation.
    /// Fragment shader computes: f = u² - v
    pub curve_uv: [f32; 2],

    /// Curve orientation sign: +1.0 (convex) or -1.0 (concave).
    ///
    /// - Convex (P₁ inside curve): f < 0 means inside
    /// - Concave (P₁ outside curve): f > 0 means inside
    ///
    /// Fragment shader tests: f * curve_sign < 0
    pub curve_sign: f32,
}

impl LoopBlinnVertex {
    /// Create a new Loop-Blinn vertex.
    pub fn new(x: f32, y: f32, u: f32, v: f32, sign: f32) -> Self {
        Self {
            position: [x, y],
            curve_uv: [u, v],
            curve_sign: sign,
        }
    }

    /// Create the three vertices for a quadratic Bezier curve.
    ///
    /// Returns vertices for P₀, P₁, P₂ with fixed texture coordinate assignment.
    pub fn from_quadratic(
        p0: [f32; 2],
        p1: [f32; 2],
        p2: [f32; 2],
        curve_sign: f32,
    ) -> [Self; 3] {
        [
            Self::new(p0[0], p0[1], 0.0, 0.0, curve_sign),
            Self::new(p1[0], p1[1], 0.5, 0.0, curve_sign),
            Self::new(p2[0], p2[1], 1.0, 1.0, curve_sign),
        ]
    }

    /// Vertex buffer layout descriptor for wgpu pipeline.
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LoopBlinnVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // @location(0) position: vec2<f32>
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(1) curve_uv: vec2<f32>
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(2) curve_sign: f32
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32,
                },
            ],
        }
    }
}

// =============================================================================
// Cubic Loop-Blinn Vertex (I-2)
// =============================================================================

/// GPU vertex for cubic Bezier Loop-Blinn curve rendering.
///
/// Each cubic Bezier curve is rendered as two triangles covering the control
/// polygon (4 vertices). The fragment shader evaluates the implicit curve
/// equation k³ - l·m to determine inside/outside.
///
/// ## Memory Layout
///
/// Total size: 24 bytes (6 × f32), 4-byte aligned.
///
/// ```text
/// Offset  Size  Field
/// 0       8     position [f32; 2]
/// 8       12    curve_klm [f32; 3]
/// 20      4     curve_sign f32
/// ```
///
/// ## Texture Coordinate Assignment
///
/// For a cubic Bezier classified as Serpentine, Cusp, or Loop, the (k, l, m)
/// coordinates are computed by `classify_cubic()` based on the curve type.
/// The implicit function f(k,l,m) = k³ - l·m equals zero exactly on the curve.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CubicLoopBlinnVertex {
    /// Position in device pixels (post-rasterization).
    pub position: [f32; 2],

    /// Loop-Blinn texture coordinates for implicit function evaluation.
    /// Fragment shader computes: f = k³ - l·m
    pub curve_klm: [f32; 3],

    /// Curve orientation sign: +1.0 (fill inside) or -1.0 (fill outside).
    ///
    /// Fragment shader tests: f * curve_sign < 0
    pub curve_sign: f32,
}

impl CubicLoopBlinnVertex {
    /// Create a new cubic Loop-Blinn vertex.
    pub fn new(position: [f32; 2], klm: [f32; 3], sign: f32) -> Self {
        Self {
            position,
            curve_klm: klm,
            curve_sign: sign,
        }
    }

    /// Create the four vertices for a cubic Bezier curve.
    ///
    /// Returns vertices for P₀, P₁, P₂, P₃ with texture coordinates from
    /// the classification result.
    ///
    /// # Arguments
    ///
    /// * `p0`, `p1`, `p2`, `p3` - Control point positions
    /// * `klm` - Texture coordinates from `classify_cubic().klm`
    /// * `curve_sign` - Curve orientation from `classify_cubic().curve_sign`
    pub fn from_cubic(
        p0: [f32; 2],
        p1: [f32; 2],
        p2: [f32; 2],
        p3: [f32; 2],
        klm: &[[f32; 3]; 4],
        curve_sign: f32,
    ) -> [Self; 4] {
        [
            Self::new(p0, klm[0], curve_sign),
            Self::new(p1, klm[1], curve_sign),
            Self::new(p2, klm[2], curve_sign),
            Self::new(p3, klm[3], curve_sign),
        ]
    }

    /// Vertex buffer layout descriptor for wgpu pipeline.
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CubicLoopBlinnVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // @location(0) position: vec2<f32>
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(1) curve_klm: vec3<f32>
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                // @location(2) curve_sign: f32
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 2]>() + std::mem::size_of::<[f32; 3]>())
                        as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Quadratic LoopBlinnVertex Tests
    // =========================================================================

    #[test]
    fn test_vertex_size() {
        // Verify 20-byte size (5 × f32)
        assert_eq!(std::mem::size_of::<LoopBlinnVertex>(), 20);
    }

    #[test]
    fn test_vertex_alignment() {
        // Verify 4-byte alignment
        assert_eq!(std::mem::align_of::<LoopBlinnVertex>(), 4);
    }

    #[test]
    fn test_from_quadratic_convex() {
        let p0 = [0.0, 0.0];
        let p1 = [50.0, 100.0]; // Control point
        let p2 = [100.0, 0.0];

        let vertices = LoopBlinnVertex::from_quadratic(p0, p1, p2, 1.0);

        // Verify positions
        assert_eq!(vertices[0].position, p0);
        assert_eq!(vertices[1].position, p1);
        assert_eq!(vertices[2].position, p2);

        // Verify fixed texture coordinates
        assert_eq!(vertices[0].curve_uv, [0.0, 0.0]);
        assert_eq!(vertices[1].curve_uv, [0.5, 0.0]);
        assert_eq!(vertices[2].curve_uv, [1.0, 1.0]);

        // Verify curve sign
        assert_eq!(vertices[0].curve_sign, 1.0);
        assert_eq!(vertices[1].curve_sign, 1.0);
        assert_eq!(vertices[2].curve_sign, 1.0);
    }

    #[test]
    fn test_from_quadratic_concave() {
        let p0 = [0.0, 0.0];
        let p1 = [50.0, -50.0]; // Control point below baseline
        let p2 = [100.0, 0.0];

        let vertices = LoopBlinnVertex::from_quadratic(p0, p1, p2, -1.0);

        // Verify curve sign is negative for concave
        assert_eq!(vertices[0].curve_sign, -1.0);
        assert_eq!(vertices[1].curve_sign, -1.0);
        assert_eq!(vertices[2].curve_sign, -1.0);
    }

    #[test]
    fn test_vertex_buffer_layout() {
        let layout = LoopBlinnVertex::desc();

        // Verify stride matches struct size
        assert_eq!(layout.array_stride, 20);

        // Verify attribute count
        assert_eq!(layout.attributes.len(), 3);

        // Verify offsets
        assert_eq!(layout.attributes[0].offset, 0); // position
        assert_eq!(layout.attributes[1].offset, 8); // curve_uv
        assert_eq!(layout.attributes[2].offset, 16); // curve_sign
    }

    // =========================================================================
    // Cubic CubicLoopBlinnVertex Tests (I-2)
    // =========================================================================

    #[test]
    fn test_cubic_vertex_size() {
        // Verify 24-byte size (6 × f32)
        assert_eq!(std::mem::size_of::<CubicLoopBlinnVertex>(), 24);
    }

    #[test]
    fn test_cubic_vertex_alignment() {
        // Verify 4-byte alignment
        assert_eq!(std::mem::align_of::<CubicLoopBlinnVertex>(), 4);
    }

    #[test]
    fn test_cubic_from_cubic() {
        let p0 = [0.0, 0.0];
        let p1 = [33.0, 100.0];
        let p2 = [66.0, 100.0];
        let p3 = [100.0, 0.0];

        let klm = [
            [0.1, 0.2, 0.3],
            [0.4, 0.5, 0.6],
            [0.7, 0.8, 0.9],
            [1.0, 1.1, 1.2],
        ];

        let vertices = CubicLoopBlinnVertex::from_cubic(p0, p1, p2, p3, &klm, 1.0);

        // Verify positions
        assert_eq!(vertices[0].position, p0);
        assert_eq!(vertices[1].position, p1);
        assert_eq!(vertices[2].position, p2);
        assert_eq!(vertices[3].position, p3);

        // Verify texture coordinates
        assert_eq!(vertices[0].curve_klm, klm[0]);
        assert_eq!(vertices[1].curve_klm, klm[1]);
        assert_eq!(vertices[2].curve_klm, klm[2]);
        assert_eq!(vertices[3].curve_klm, klm[3]);

        // Verify curve sign
        for v in &vertices {
            assert_eq!(v.curve_sign, 1.0);
        }
    }

    #[test]
    fn test_cubic_vertex_buffer_layout() {
        let layout = CubicLoopBlinnVertex::desc();

        // Verify stride matches struct size (24 bytes)
        assert_eq!(layout.array_stride, 24);

        // Verify attribute count
        assert_eq!(layout.attributes.len(), 3);

        // Verify offsets
        assert_eq!(layout.attributes[0].offset, 0); // position: vec2<f32>
        assert_eq!(layout.attributes[1].offset, 8); // curve_klm: vec3<f32>
        assert_eq!(layout.attributes[2].offset, 20); // curve_sign: f32

        // Verify formats
        assert_eq!(layout.attributes[0].format, wgpu::VertexFormat::Float32x2);
        assert_eq!(layout.attributes[1].format, wgpu::VertexFormat::Float32x3);
        assert_eq!(layout.attributes[2].format, wgpu::VertexFormat::Float32);
    }
}
