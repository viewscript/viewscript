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
