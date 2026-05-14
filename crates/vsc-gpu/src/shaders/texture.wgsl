// =============================================================================
// texture.wgsl - External Texture Sampling Shader (Phase J-3)
// =============================================================================
//
// Renders shapes with external texture fills (images, videos, canvas).
// Uses the same vertex format as solid.wgsl (GpuVertex with position + uv).
//
// Bind Group Layout:
//   Group 0: Transform uniform (shared with all pipelines)
//   Group 1: Texture + Sampler

// =============================================================================
// Uniform Buffers
// =============================================================================

/// Global transform uniform (binding group 0, binding 0)
/// Matches solid.wgsl Transform struct for vertex shader compatibility.
struct Transform {
    // Affine transform matrix (row-major 2x3)
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
    // Viewport dimensions for device-to-NDC conversion
    viewport_width: f32,
    viewport_height: f32,
    // Accumulated opacity from scene graph hierarchy [0, 1]
    opacity: f32,
    // Padding for 16-byte alignment
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
}

@group(0) @binding(0)
var<uniform> transform: Transform;

// Texture and sampler (binding group 1)
@group(1) @binding(0)
var t_texture: texture_2d<f32>;

@group(1) @binding(1)
var t_sampler: sampler;

// =============================================================================
// Vertex Shader
// =============================================================================

/// Vertex input from GpuVertex (same as solid.wgsl)
struct VertexInput {
    @location(0) position: vec2<f32>,  // Device pixel coordinates
    @location(1) uv: vec2<f32>,        // Normalized UV [0, 1]
}

/// Vertex output to fragment shader
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,  // Texture coordinates
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Apply 2D affine transform: p' = M * p + t
    let transformed = vec2<f32>(
        transform.a * in.position.x + transform.b * in.position.y + transform.tx,
        transform.c * in.position.x + transform.d * in.position.y + transform.ty
    );

    // Convert device pixels to NDC (Normalized Device Coordinates)
    // NDC range: x [-1, 1], y [-1, 1] (top-left = (-1, 1), bottom-right = (1, -1))
    let ndc = vec2<f32>(
        (transformed.x / transform.viewport_width) * 2.0 - 1.0,
        1.0 - (transformed.y / transform.viewport_height) * 2.0  // Y flipped for screen coords
    );

    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = in.uv;

    return out;
}

// =============================================================================
// Fragment Shader
// =============================================================================

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample texture at UV coordinates
    let color = textureSample(t_texture, t_sampler, in.uv);

    // Apply accumulated opacity from scene graph hierarchy
    return vec4<f32>(color.rgb, color.a * transform.opacity);
}
