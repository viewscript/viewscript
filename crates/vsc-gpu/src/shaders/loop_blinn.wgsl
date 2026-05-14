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
// This avoids `discard` and preserves Early-Z optimization.

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
