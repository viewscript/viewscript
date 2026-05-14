// =============================================================================
// solid.wgsl - Solid Color Shader
// =============================================================================
//
// This is the simplest rendering pipeline for ViewScript:
// - Vertex shader: applies AffineTransform and passes through UV coordinates
// - Fragment shader: outputs a uniform solid color
//
// Used for FillStyle::Solid and StrokeStyle solid colors.

// =============================================================================
// Uniform Buffers
// =============================================================================

/// Global transform uniform (binding group 0, binding 0)
/// Contains the 2D affine transformation matrix, viewport size, and accumulated opacity.
struct Transform {
    // Affine transform matrix (row-major 2x3)
    // | a  b  tx |
    // | c  d  ty |
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
    // Padding for 16-byte alignment (3 floats = 12 bytes)
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
}

@group(0) @binding(0)
var<uniform> transform: Transform;

/// Per-draw solid color uniform (binding group 1, binding 0)
struct SolidColor {
    // RGBA color in linear space [0, 1]
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

@group(1) @binding(0)
var<uniform> solid_color: SolidColor;

// =============================================================================
// Vertex Shader
// =============================================================================

/// Vertex input from GpuVertex
struct VertexInput {
    @location(0) position: vec2<f32>,  // Device pixel coordinates
    @location(1) uv: vec2<f32>,        // Normalized UV [0, 1]
}

/// Vertex output to fragment shader
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,  // Pass through for gradient shaders
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
    // Device pixels: x [0, viewport_width], y [0, viewport_height] (origin = top-left)
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
    // Solid color: output the uniform color with accumulated opacity
    // Note: in.uv is available but unused for solid fills
    var color = vec4<f32>(solid_color.r, solid_color.g, solid_color.b, solid_color.a);
    // Apply accumulated opacity from scene graph hierarchy
    color.a = color.a * transform.opacity;
    return color;
}
