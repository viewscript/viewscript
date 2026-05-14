//! Cubic SDF Stroke Vertex Definition
//!
//! Defines the vertex format for SDF-based cubic Bezier stroke rendering.
//! Each cubic curve segment is rendered as a bounding rectangle with
//! Newton's method distance evaluation in the fragment shader.

/// GPU vertex for cubic SDF stroke rendering.
///
/// Each cubic Bezier stroke segment is rendered as a bounding rectangle
/// (4 vertices, 2 triangles). The fragment shader uses Newton's method
/// to find the closest point on the curve and compute the signed distance.
///
/// ## Memory Layout
///
/// Total size: 52 bytes (13 × f32), 4-byte aligned.
///
/// ```text
/// Offset  Size  Field
/// 0       8     position [f32; 2]
/// 8       8     local_pos [f32; 2]
/// 16      8     p0 [f32; 2]
/// 24      8     p1 [f32; 2]
/// 32      8     p2 [f32; 2]
/// 40      8     p3 [f32; 2]
/// 48      4     half_width f32
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CubicSdfStrokeVertex {
    /// World-space position of this rectangle vertex.
    pub position: [f32; 2],

    /// Local position within the bounding rectangle.
    /// Used for distance calculation in fragment shader.
    pub local_pos: [f32; 2],

    /// Curve start point P0.
    pub p0: [f32; 2],

    /// Curve control point P1.
    pub p1: [f32; 2],

    /// Curve control point P2.
    pub p2: [f32; 2],

    /// Curve end point P3.
    pub p3: [f32; 2],

    /// Half stroke width (radius).
    pub half_width: f32,
}

impl CubicSdfStrokeVertex {
    /// Create a new cubic SDF stroke vertex.
    pub fn new(
        position: [f32; 2],
        local_pos: [f32; 2],
        p0: [f32; 2],
        p1: [f32; 2],
        p2: [f32; 2],
        p3: [f32; 2],
        half_width: f32,
    ) -> Self {
        Self {
            position,
            local_pos,
            p0,
            p1,
            p2,
            p3,
            half_width,
        }
    }

    /// Vertex buffer layout descriptor for wgpu pipeline.
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CubicSdfStrokeVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // @location(0) position: vec2<f32>
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(1) local_pos: vec2<f32>
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(2) p0: vec2<f32>
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 2]>() * 2) as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(3) p1: vec2<f32>
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 2]>() * 3) as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(4) p2: vec2<f32>
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 2]>() * 4) as wgpu::BufferAddress,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(5) p3: vec2<f32>
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 2]>() * 5) as wgpu::BufferAddress,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(6) half_width: f32
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 2]>() * 6) as wgpu::BufferAddress,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cubic_sdf_stroke_vertex_size() {
        // Verify 52-byte size (13 × f32)
        assert_eq!(std::mem::size_of::<CubicSdfStrokeVertex>(), 52);
    }

    #[test]
    fn test_cubic_sdf_stroke_vertex_alignment() {
        // Verify 4-byte alignment
        assert_eq!(std::mem::align_of::<CubicSdfStrokeVertex>(), 4);
    }

    #[test]
    fn test_cubic_sdf_stroke_vertex_buffer_layout() {
        let layout = CubicSdfStrokeVertex::desc();

        // Verify stride matches struct size (52 bytes)
        assert_eq!(layout.array_stride, 52);

        // Verify attribute count (7 attributes)
        assert_eq!(layout.attributes.len(), 7);

        // Verify offsets
        assert_eq!(layout.attributes[0].offset, 0);   // position
        assert_eq!(layout.attributes[1].offset, 8);   // local_pos
        assert_eq!(layout.attributes[2].offset, 16);  // p0
        assert_eq!(layout.attributes[3].offset, 24);  // p1
        assert_eq!(layout.attributes[4].offset, 32);  // p2
        assert_eq!(layout.attributes[5].offset, 40);  // p3
        assert_eq!(layout.attributes[6].offset, 48);  // half_width

        // Verify formats
        assert_eq!(layout.attributes[0].format, wgpu::VertexFormat::Float32x2);
        assert_eq!(layout.attributes[6].format, wgpu::VertexFormat::Float32);
    }
}
