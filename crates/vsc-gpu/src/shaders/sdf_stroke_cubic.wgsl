// =============================================================================
// sdf_stroke_cubic.wgsl - SDF-based Cubic Bezier Stroke Shader
// =============================================================================
//
// Renders cubic Bezier curve strokes using Signed Distance Field evaluation.
// Each curve segment is a bounding rectangle; the fragment shader uses Newton's
// method to find the closest point on the curve.
//
// ## Algorithm
//
// 1. Sample 5 initial points along the curve (t = 0, 0.25, 0.5, 0.75, 1)
// 2. Use best sample as starting point for Newton's method
// 3. Refine with 4 Newton iterations
// 4. Compare distance to half stroke width
// 5. Apply smoothstep anti-aliasing
//
// ## Why Newton's Method?
//
// For cubic Bezier curves, the closest-point equation is a quintic polynomial
// (degree 5), which has no closed-form solution. Newton's method provides
// fast convergence from a good initial guess.
//
// ## Coordinate Space
//
// Distance calculation is performed entirely in path-local space.
// - `local_pos`: Interpolated local position (from vertex attribute)
// - `p0/p1/p2/p3`: Curve control points (path-local, passed through from vertex)
// - `half_width`: Half stroke width (path-local)

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

struct StrokeColor {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

@group(1) @binding(0)
var<uniform> stroke_color: StrokeColor;

// =============================================================================
// Vertex Shader
// =============================================================================

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) local_pos: vec2<f32>,
    @location(2) p0: vec2<f32>,
    @location(3) p1: vec2<f32>,
    @location(4) p2: vec2<f32>,
    @location(5) p3: vec2<f32>,
    @location(6) half_width: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) p0: vec2<f32>,
    @location(2) p1: vec2<f32>,
    @location(3) p2: vec2<f32>,
    @location(4) p3: vec2<f32>,
    @location(5) half_width: f32,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Position is already in world space (transformed by batcher on CPU)
    // Convert to NDC
    let ndc = vec2<f32>(
        (in.position.x / transform.viewport_width) * 2.0 - 1.0,
        1.0 - (in.position.y / transform.viewport_height) * 2.0
    );

    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);

    // Pass through local-space attributes unchanged
    out.local_pos = in.local_pos;
    out.p0 = in.p0;
    out.p1 = in.p1;
    out.p2 = in.p2;
    out.p3 = in.p3;
    out.half_width = in.half_width;

    return out;
}

// =============================================================================
// Fragment Shader - Newton's Method Implementation
// =============================================================================

const EPSILON: f32 = 1e-6;
const NEWTON_ITERATIONS: i32 = 4;

/// Evaluate cubic Bezier at parameter t.
/// B(t) = (1-t)^3 P0 + 3t(1-t)^2 P1 + 3t^2(1-t) P2 + t^3 P3
fn eval_cubic_bezier(p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>, t: f32) -> vec2<f32> {
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let mt3 = mt2 * mt;
    let t2 = t * t;
    let t3 = t2 * t;
    return mt3 * p0 + 3.0 * mt2 * t * p1 + 3.0 * mt * t2 * p2 + t3 * p3;
}

/// Evaluate cubic Bezier first derivative at parameter t.
/// B'(t) = 3(1-t)^2 (P1-P0) + 6(1-t)t (P2-P1) + 3t^2 (P3-P2)
fn eval_cubic_bezier_derivative(p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>, t: f32) -> vec2<f32> {
    let mt = 1.0 - t;
    let d01 = p1 - p0;
    let d12 = p2 - p1;
    let d23 = p3 - p2;
    return 3.0 * mt * mt * d01 + 6.0 * mt * t * d12 + 3.0 * t * t * d23;
}

/// Evaluate cubic Bezier second derivative at parameter t.
/// B''(t) = 6(1-t)(P2 - 2P1 + P0) + 6t(P3 - 2P2 + P1)
fn eval_cubic_bezier_second_derivative(p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>, t: f32) -> vec2<f32> {
    let mt = 1.0 - t;
    let d2_01 = p2 - 2.0 * p1 + p0;
    let d2_12 = p3 - 2.0 * p2 + p1;
    return 6.0 * mt * d2_01 + 6.0 * t * d2_12;
}

/// Compute distance squared from point to cubic bezier at parameter t.
fn dist_sq_to_cubic_bezier(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>, t: f32) -> f32 {
    let b = eval_cubic_bezier(p0, p1, p2, p3, t);
    let d = p - b;
    return dot(d, d);
}

/// Newton's method step for finding closest point on cubic Bezier.
///
/// Minimizes f(t) = |P - B(t)|^2
/// f'(t) = -2 * (P - B(t)) . B'(t) = 2 * (B(t) - P) . B'(t)
/// f''(t) = 2 * (B'(t) . B'(t) + (B(t) - P) . B''(t))
///
/// Newton update: t_new = t - f'(t) / f''(t)
fn newton_step(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>, t: f32) -> f32 {
    let b = eval_cubic_bezier(p0, p1, p2, p3, t);
    let b_prime = eval_cubic_bezier_derivative(p0, p1, p2, p3, t);
    let b_double_prime = eval_cubic_bezier_second_derivative(p0, p1, p2, p3, t);

    let diff = b - p;

    // f'(t) = 2 * diff . B'(t)
    let f_prime = 2.0 * dot(diff, b_prime);

    // f''(t) = 2 * (B'(t) . B'(t) + diff . B''(t))
    let f_double_prime = 2.0 * (dot(b_prime, b_prime) + dot(diff, b_double_prime));

    // Avoid division by zero
    if abs(f_double_prime) < EPSILON {
        return t;
    }

    // Newton update, clamped to valid range
    return clamp(t - f_prime / f_double_prime, 0.0, 1.0);
}

/// Find the minimum distance from point P to cubic Bezier curve.
/// Uses 5-point initial sampling followed by Newton refinement.
/// Returns the squared distance.
fn min_dist_sq_to_cubic_bezier(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>) -> f32 {
    // Sample 5 initial points along the curve
    var best_t = 0.0;
    var best_dist_sq = dist_sq_to_cubic_bezier(p, p0, p1, p2, p3, 0.0);

    let d1 = dist_sq_to_cubic_bezier(p, p0, p1, p2, p3, 0.25);
    if d1 < best_dist_sq {
        best_dist_sq = d1;
        best_t = 0.25;
    }

    let d2 = dist_sq_to_cubic_bezier(p, p0, p1, p2, p3, 0.5);
    if d2 < best_dist_sq {
        best_dist_sq = d2;
        best_t = 0.5;
    }

    let d3 = dist_sq_to_cubic_bezier(p, p0, p1, p2, p3, 0.75);
    if d3 < best_dist_sq {
        best_dist_sq = d3;
        best_t = 0.75;
    }

    let d4 = dist_sq_to_cubic_bezier(p, p0, p1, p2, p3, 1.0);
    if d4 < best_dist_sq {
        best_dist_sq = d4;
        best_t = 1.0;
    }

    // Refine with Newton's method (4 iterations)
    var t = best_t;
    for (var i = 0; i < NEWTON_ITERATIONS; i = i + 1) {
        t = newton_step(p, p0, p1, p2, p3, t);
    }

    // Final distance at refined t
    let refined_dist_sq = dist_sq_to_cubic_bezier(p, p0, p1, p2, p3, t);

    // Also check endpoints explicitly (Newton might not converge to them)
    let d_start = dist_sq_to_cubic_bezier(p, p0, p1, p2, p3, 0.0);
    let d_end = dist_sq_to_cubic_bezier(p, p0, p1, p2, p3, 1.0);

    return min(min(refined_dist_sq, d_start), d_end);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Compute minimum distance to curve (in local space)
    let dist_sq = min_dist_sq_to_cubic_bezier(in.local_pos, in.p0, in.p1, in.p2, in.p3);
    let dist = sqrt(dist_sq);

    // Anti-aliased stroke using smoothstep
    // fwidth gives screen-space derivative for AA width
    let fw = fwidth(dist);
    let alpha = smoothstep(in.half_width + fw, in.half_width - fw, dist);

    // Early discard for fully transparent fragments
    if alpha < 0.001 {
        discard;
    }

    // Output color with combined alpha
    let base_alpha = stroke_color.a * transform.opacity;
    return vec4<f32>(
        stroke_color.r,
        stroke_color.g,
        stroke_color.b,
        base_alpha * alpha
    );
}
