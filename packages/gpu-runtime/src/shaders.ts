// =============================================================================
// WGSL Shader Sources for @viewscript/gpu-runtime
// =============================================================================
//
// These shaders are mirrored from vsc-gpu/src/shaders/*.wgsl
// to enable standalone JavaScript execution without WASM runtime.

/**
 * Solid Color Shader
 *
 * Simplest rendering pipeline:
 * - Vertex: applies AffineTransform, passes through UV
 * - Fragment: outputs uniform solid color
 */
export const SOLID_WGSL = /* wgsl */ `
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
`;

/**
 * Loop-Blinn Quadratic Bezier Curve Shader
 *
 * Renders quadratic Bezier curves using implicit function f(u,v) = u^2 - v
 * - Texture coords: P0=(0,0), P1=(0.5,0), P2=(1,1)
 * - Anti-aliasing via smoothstep with fwidth()
 */
export const LOOP_BLINN_WGSL = /* wgsl */ `
// =============================================================================
// loop_blinn.wgsl - Loop-Blinn Quadratic Bezier Curve Shader
// =============================================================================
//
// Renders quadratic Bezier curves using the Loop-Blinn algorithm.
// Each curve is a single triangle (P0, P1, P2) with the fragment shader
// evaluating the implicit curve equation to determine inside/outside.
//
// ## Algorithm
//
// For quadratic Bezier B(t) = (1-t)^2 P0 + 2t(1-t) P1 + t^2 P2:
// - Texture coords: P0=(0,0), P1=(0.5,0), P2=(1,1)
// - Implicit function: f(u,v) = u^2 - v
// - f = 0 on the curve, f < 0 one side, f > 0 other side
//
// ## Anti-aliasing
//
// Uses smoothstep with fwidth() for screen-space anti-aliasing.
// This avoids \`discard\` and preserves Early-Z optimization.

// =============================================================================
// Uniform Buffers (shared with solid.wgsl)
// =============================================================================

struct Transform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
    viewport_width: f32,
    viewport_height: f32,
    opacity: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
}

@group(0) @binding(0)
var<uniform> transform: Transform;

struct SolidColor {
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

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) curve_uv: vec2<f32>,
    @location(2) curve_sign: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) curve_uv: vec2<f32>,
    @location(1) curve_sign: f32,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Apply 2D affine transform
    let transformed = vec2<f32>(
        transform.a * in.position.x + transform.b * in.position.y + transform.tx,
        transform.c * in.position.x + transform.d * in.position.y + transform.ty
    );

    // Convert to NDC (Normalized Device Coordinates)
    let ndc = vec2<f32>(
        (transformed.x / transform.viewport_width) * 2.0 - 1.0,
        1.0 - (transformed.y / transform.viewport_height) * 2.0
    );

    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.curve_uv = in.curve_uv;
    out.curve_sign = in.curve_sign;

    return out;
}

// =============================================================================
// Fragment Shader
// =============================================================================

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let u = in.curve_uv.x;
    let v = in.curve_uv.y;

    // Loop-Blinn implicit function: f = u^2 - v
    // f = 0 on the curve
    // f < 0 on one side (typically "inside" for convex curves)
    // f > 0 on the other side
    let f = u * u - v;

    // Screen-space derivative for anti-aliasing width
    // fwidth(f) gives the sum of absolute partial derivatives in screen space
    let fw = fwidth(f);

    // Signed distance considering curve orientation
    // curve_sign: +1 for convex (f < 0 is inside), -1 for concave (f > 0 is inside)
    let signed_f = f * in.curve_sign;

    // Anti-aliased alpha using smoothstep
    // When signed_f is:
    //   < -fw: fully inside (alpha = 1)
    //   > +fw: fully outside (alpha = 0)
    //   in between: smooth transition
    let alpha = smoothstep(fw, -fw, signed_f);

    // Early out for fully transparent fragments (optional optimization)
    if alpha < 0.001 {
        discard;
    }

    // Output color with combined alpha
    let base_alpha = solid_color.a * transform.opacity;
    return vec4<f32>(
        solid_color.r,
        solid_color.g,
        solid_color.b,
        base_alpha * alpha
    );
}
`;

/**
 * Loop-Blinn Cubic Bezier Curve Shader
 *
 * Renders cubic Bezier curves using implicit function f(k,l,m) = k^3 - l*m
 * - Curve classification: serpentine (D>0), cusp (D=0), loop (D<0)
 * - Anti-aliasing via smoothstep with fwidth()
 */
/**
 * Texture Sampling Shader
 *
 * Renders textured quads for external images (WebP, PNG, etc.):
 * - Vertex: same as solid (position + uv)
 * - Fragment: samples texture at UV, applies opacity
 *
 * Bind Group Layout:
 *   Group 0: Transform uniform (shared)
 *   Group 1: Texture + Sampler
 */
export const TEXTURE_WGSL = /* wgsl */ `
// =============================================================================
// texture.wgsl - External Texture Sampling Shader
// =============================================================================
//
// Renders shapes with external texture fills (images, videos, canvas).
// Uses the same vertex format as solid.wgsl (GpuVertex with position + uv).

// =============================================================================
// Uniform Buffers
// =============================================================================

struct Transform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
    viewport_width: f32,
    viewport_height: f32,
    opacity: f32,
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

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    let transformed = vec2<f32>(
        transform.a * in.position.x + transform.b * in.position.y + transform.tx,
        transform.c * in.position.x + transform.d * in.position.y + transform.ty
    );

    let ndc = vec2<f32>(
        (transformed.x / transform.viewport_width) * 2.0 - 1.0,
        1.0 - (transformed.y / transform.viewport_height) * 2.0
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
    let color = textureSample(t_texture, t_sampler, in.uv);
    return vec4<f32>(color.rgb, color.a * transform.opacity);
}
`;

export const LOOP_BLINN_CUBIC_WGSL = /* wgsl */ `
// =============================================================================
// loop_blinn_cubic.wgsl - Loop-Blinn Cubic Bezier Curve Shader
// =============================================================================
//
// Renders cubic Bezier curves using the Loop-Blinn algorithm.
// Each curve is rendered as two triangles covering the control polygon (4 vertices).
// The fragment shader evaluates the implicit curve equation to determine inside/outside.
//
// ## Algorithm
//
// For cubic Bezier B(t) = (1-t)^3 P0 + 3t(1-t)^2 P1 + 3t^2(1-t) P2 + t^3 P3:
// - Texture coords (k, l, m) computed by classify_cubic() based on curve type
// - Implicit function: f(k,l,m) = k^3 - l*m
// - f = 0 on the curve, f < 0 one side, f > 0 other side
//
// ## Curve Classification
//
// Cubic curves are classified into three types:
// - Serpentine (D > 0): Two inflection points
// - Cusp (D = 0): Degenerate inflection point
// - Loop (D < 0): Self-intersecting curve
//
// Each type has different (k, l, m) coordinate formulas.
//
// ## Anti-aliasing
//
// Uses smoothstep with fwidth() for screen-space anti-aliasing.
// This avoids \`discard\` and preserves Early-Z optimization.

// =============================================================================
// Uniform Buffers (shared with solid.wgsl)
// =============================================================================

struct Transform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
    viewport_width: f32,
    viewport_height: f32,
    opacity: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
}

@group(0) @binding(0)
var<uniform> transform: Transform;

struct SolidColor {
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

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) curve_klm: vec3<f32>,
    @location(2) curve_sign: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) curve_klm: vec3<f32>,
    @location(1) curve_sign: f32,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Apply 2D affine transform
    let transformed = vec2<f32>(
        transform.a * in.position.x + transform.b * in.position.y + transform.tx,
        transform.c * in.position.x + transform.d * in.position.y + transform.ty
    );

    // Convert to NDC (Normalized Device Coordinates)
    let ndc = vec2<f32>(
        (transformed.x / transform.viewport_width) * 2.0 - 1.0,
        1.0 - (transformed.y / transform.viewport_height) * 2.0
    );

    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.curve_klm = in.curve_klm;
    out.curve_sign = in.curve_sign;

    return out;
}

// =============================================================================
// Fragment Shader
// =============================================================================

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let k = in.curve_klm.x;
    let l = in.curve_klm.y;
    let m = in.curve_klm.z;

    // Loop-Blinn implicit function for cubic curves: f = k^3 - l*m
    // f = 0 on the curve
    // f < 0 on one side (typically "inside" for convex curves)
    // f > 0 on the other side
    let f = k * k * k - l * m;

    // Screen-space derivative for anti-aliasing width
    // fwidth(f) gives the sum of absolute partial derivatives in screen space
    let fw = fwidth(f);

    // Signed distance considering curve orientation
    // curve_sign: +1 for fill inside, -1 for fill outside
    let signed_f = f * in.curve_sign;

    // Anti-aliased alpha using smoothstep
    // When signed_f is:
    //   < -fw: fully inside (alpha = 1)
    //   > +fw: fully outside (alpha = 0)
    //   in between: smooth transition
    let alpha = smoothstep(fw, -fw, signed_f);

    // Early out for fully transparent fragments (optional optimization)
    if alpha < 0.001 {
        discard;
    }

    // Output color with combined alpha
    let base_alpha = solid_color.a * transform.opacity;
    return vec4<f32>(
        solid_color.r,
        solid_color.g,
        solid_color.b,
        base_alpha * alpha
    );
}
`;
