//! SDF Stroke Vertex Definition
//!
//! Vertex format for SDF-based stroke rendering. Each vertex contains all
//! information needed for distance calculation, eliminating per-segment
//! uniform switches.

/// Vertex for SDF stroke rendering.
///
/// Contains curve control points directly in vertex attributes to enable
/// batching without uniform buffer switching.
///
/// ## Size
///
/// 44 bytes (11 × f32), well under WebGPU's 2048-byte stride limit.
///
/// ## WGSL Mapping
///
/// ```wgsl
/// struct VertexInput {
///     @location(0) position: vec2<f32>,
///     @location(1) local_pos: vec2<f32>,
///     @location(2) p0: vec2<f32>,
///     @location(3) p1: vec2<f32>,
///     @location(4) p2: vec2<f32>,
///     @location(5) half_width: f32,
/// }
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SdfStrokeVertex {
    /// World-space position of this rectangle vertex.
    pub position: [f32; 2],

    /// Local position within the bounding rectangle.
    /// Used for distance calculation in fragment shader.
    /// Ranges from AABB min to AABB max in curve-local space.
    pub local_pos: [f32; 2],

    /// Curve start point P0.
    pub p0: [f32; 2],

    /// Curve control point P1.
    pub p1: [f32; 2],

    /// Curve end point P2.
    pub p2: [f32; 2],

    /// Half stroke width (stroke_width / 2).
    /// Fragment is inside stroke if distance <= half_width.
    pub half_width: f32,
}

impl SdfStrokeVertex {
    /// Create a new SDF stroke vertex.
    pub fn new(
        position: [f32; 2],
        local_pos: [f32; 2],
        p0: [f32; 2],
        p1: [f32; 2],
        p2: [f32; 2],
        half_width: f32,
    ) -> Self {
        Self {
            position,
            local_pos,
            p0,
            p1,
            p2,
            half_width,
        }
    }

    /// Returns the wgpu vertex buffer layout descriptor for this vertex type.
    ///
    /// Uses 6 locations:
    /// - @location(0): position (vec2)
    /// - @location(1): local_pos (vec2)
    /// - @location(2): p0 (vec2)
    /// - @location(3): p1 (vec2)
    /// - @location(4): p2 (vec2)
    /// - @location(5): half_width (f32)
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
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
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(2) p0: vec2<f32>
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(3) p1: vec2<f32>
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(4) p2: vec2<f32>
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(5) half_width: f32
                wgpu::VertexAttribute {
                    offset: 40,
                    shader_location: 5,
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
    fn test_vertex_size() {
        // Must be exactly 44 bytes (11 × f32)
        assert_eq!(
            std::mem::size_of::<SdfStrokeVertex>(),
            44,
            "SdfStrokeVertex must be 44 bytes"
        );
    }

    #[test]
    fn test_vertex_alignment() {
        // Must be 4-byte aligned (f32 alignment)
        assert_eq!(
            std::mem::align_of::<SdfStrokeVertex>(),
            4,
            "SdfStrokeVertex must be 4-byte aligned"
        );
    }

    #[test]
    fn test_desc_stride() {
        let layout = SdfStrokeVertex::desc();
        assert_eq!(layout.array_stride, 44);
    }

    #[test]
    fn test_desc_attributes_count() {
        let layout = SdfStrokeVertex::desc();
        assert_eq!(layout.attributes.len(), 6, "Expected 6 vertex attributes");
    }

    #[test]
    fn test_desc_attribute_offsets() {
        let layout = SdfStrokeVertex::desc();

        // Verify offsets are correct
        assert_eq!(layout.attributes[0].offset, 0, "position offset");
        assert_eq!(layout.attributes[1].offset, 8, "local_pos offset");
        assert_eq!(layout.attributes[2].offset, 16, "p0 offset");
        assert_eq!(layout.attributes[3].offset, 24, "p1 offset");
        assert_eq!(layout.attributes[4].offset, 32, "p2 offset");
        assert_eq!(layout.attributes[5].offset, 40, "half_width offset");
    }

    #[test]
    fn test_desc_shader_locations() {
        let layout = SdfStrokeVertex::desc();

        // Verify shader locations are sequential 0-5
        for (i, attr) in layout.attributes.iter().enumerate() {
            assert_eq!(
                attr.shader_location, i as u32,
                "Attribute {} should have shader_location {}",
                i, i
            );
        }
    }

    #[test]
    fn test_bytemuck_pod() {
        // Verify we can safely transmute to/from bytes
        let vertex = SdfStrokeVertex::new(
            [1.0, 2.0],
            [3.0, 4.0],
            [5.0, 6.0],
            [7.0, 8.0],
            [9.0, 10.0],
            0.5,
        );

        let bytes = bytemuck::bytes_of(&vertex);
        assert_eq!(bytes.len(), 44);

        // Round-trip through bytes
        let restored: &SdfStrokeVertex = bytemuck::from_bytes(bytes);
        assert_eq!(restored.position, vertex.position);
        assert_eq!(restored.half_width, vertex.half_width);
    }
}
