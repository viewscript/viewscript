// =============================================================================
// sdf_stroke.wgsl - SDF-based Quadratic Bezier Stroke Shader
// =============================================================================
//
// Renders quadratic Bezier curve strokes using Signed Distance Field evaluation.
// Each curve segment is a bounding rectangle; the fragment shader analytically
// computes the distance from each pixel to the curve using Cardano's formula.
//
// ## Algorithm
//
// 1. Compute cubic equation coefficients for closest-point problem
// 2. Solve using Cardano's formula (handles both 1 and 3 real root cases)
// 3. Clamp solutions to [0, 1] and check endpoints
// 4. Find minimum distance among all candidates
// 5. Apply smoothstep anti-aliasing
//
// ## Coordinate Space
//
// Distance calculation is performed entirely in path-local space.
// - `local_pos`: Interpolated local position (from vertex attribute)
// - `p0/p1/p2`: Curve control points (path-local, passed through from vertex)
// - `half_width`: Half stroke width (path-local)
//
// The `position` attribute is transformed to world space for rasterization,
// but all other attributes remain in local space for correct distance math.

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
    @location(5) half_width: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) p0: vec2<f32>,
    @location(2) p1: vec2<f32>,
    @location(3) p2: vec2<f32>,
    @location(4) half_width: f32,
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
    out.half_width = in.half_width;

    return out;
}

// =============================================================================
// Fragment Shader - Cardano's Formula Implementation
// =============================================================================

const PI: f32 = 3.14159265359;
const EPSILON: f32 = 1e-6;

/// Evaluate quadratic Bezier at parameter t.
/// B(t) = (1-t)²P₀ + 2t(1-t)P₁ + t²P₂
fn eval_bezier(p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, t: f32) -> vec2<f32> {
    let mt = 1.0 - t;
    return mt * mt * p0 + 2.0 * mt * t * p1 + t * t * p2;
}

/// Compute distance squared from point to bezier at parameter t.
fn dist_sq_to_bezier(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, t: f32) -> f32 {
    let b = eval_bezier(p0, p1, p2, t);
    let d = p - b;
    return dot(d, d);
}

/// Cube root that handles negative values.
fn cbrt(x: f32) -> f32 {
    return sign(x) * pow(abs(x), 1.0 / 3.0);
}

/// Solve cubic equation: t³ + pt + q = 0 (depressed cubic)
/// Returns up to 3 real roots in the result vector.
/// result.w contains the number of real roots (1 or 3).
fn solve_depressed_cubic(p: f32, q: f32) -> vec4<f32> {
    // Discriminant: D = (q/2)² + (p/3)³
    let q_half = q * 0.5;
    let p_third = p / 3.0;
    let D = q_half * q_half + p_third * p_third * p_third;

    if D > EPSILON {
        // One real root: Cardano's formula
        let sqrt_D = sqrt(D);
        let u = cbrt(-q_half + sqrt_D);
        let v = cbrt(-q_half - sqrt_D);
        return vec4<f32>(u + v, 0.0, 0.0, 1.0);
    } else if D < -EPSILON {
        // Three real roots: trigonometric method
        let r = sqrt(-p_third * p_third * p_third);
        let phi = acos(clamp(-q_half / r, -1.0, 1.0));
        let two_sqrt_r = 2.0 * cbrt(r);

        let t1 = two_sqrt_r * cos(phi / 3.0);
        let t2 = two_sqrt_r * cos((phi + 2.0 * PI) / 3.0);
        let t3 = two_sqrt_r * cos((phi + 4.0 * PI) / 3.0);

        return vec4<f32>(t1, t2, t3, 3.0);
    } else {
        // D ≈ 0: repeated roots
        let u = cbrt(-q_half);
        return vec4<f32>(2.0 * u, -u, -u, 3.0);
    }
}

/// Find the minimum distance from point P to quadratic Bezier curve.
/// Returns the squared distance.
fn min_dist_sq_to_bezier(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>) -> f32 {
    // Rewrite Bezier as: B(t) = P₀ + 2tA + t²C
    // where A = P₁ - P₀, C = P₂ - 2P₁ + P₀
    let A = p1 - p0;
    let C = p2 - 2.0 * p1 + p0;
    let V = p - p0;

    // The derivative of distance² set to zero gives:
    // |C|²t³ + 3(A·C)t² + (2|A|² - V·C)t - V·A = 0
    let a = dot(C, C);       // Coefficient of t³
    let b = 3.0 * dot(A, C); // Coefficient of t²
    let c = 2.0 * dot(A, A) - dot(V, C); // Coefficient of t
    let d = -dot(V, A);      // Constant term

    // Handle degenerate case: C ≈ 0 (curve is nearly linear)
    if abs(a) < EPSILON {
        // Quadratic or linear case
        if abs(b) < EPSILON {
            // Linear case: ct + d = 0
            var t_candidate = 0.0;
            if abs(c) > EPSILON {
                t_candidate = clamp(-d / c, 0.0, 1.0);
            }
            let d0 = dist_sq_to_bezier(p, p0, p1, p2, 0.0);
            let d1 = dist_sq_to_bezier(p, p0, p1, p2, 1.0);
            let dt = dist_sq_to_bezier(p, p0, p1, p2, t_candidate);
            return min(min(d0, d1), dt);
        }
        // Quadratic case: bt² + ct + d = 0
        let disc = c * c - 4.0 * b * d;
        var min_d = dist_sq_to_bezier(p, p0, p1, p2, 0.0);
        min_d = min(min_d, dist_sq_to_bezier(p, p0, p1, p2, 1.0));

        if disc >= 0.0 {
            let sqrt_disc = sqrt(disc);
            let t1 = clamp((-c + sqrt_disc) / (2.0 * b), 0.0, 1.0);
            let t2 = clamp((-c - sqrt_disc) / (2.0 * b), 0.0, 1.0);
            min_d = min(min_d, dist_sq_to_bezier(p, p0, p1, p2, t1));
            min_d = min(min_d, dist_sq_to_bezier(p, p0, p1, p2, t2));
        }
        return min_d;
    }

    // Normalize to monic cubic: t³ + (b/a)t² + (c/a)t + (d/a) = 0
    let inv_a = 1.0 / a;
    let p_coef = b * inv_a;
    let q_coef = c * inv_a;
    let r_coef = d * inv_a;

    // Substitute t = u - p/3 to get depressed cubic: u³ + Pu + Q = 0
    let p_third = p_coef / 3.0;
    let P = q_coef - p_coef * p_third;
    let Q = r_coef - q_coef * p_third + 2.0 * p_third * p_third * p_third;

    // Solve depressed cubic
    let roots = solve_depressed_cubic(P, Q);
    let num_roots = i32(roots.w);

    // Convert back from u to t and evaluate distances
    var min_d = dist_sq_to_bezier(p, p0, p1, p2, 0.0);  // Endpoint t=0
    min_d = min(min_d, dist_sq_to_bezier(p, p0, p1, p2, 1.0)); // Endpoint t=1

    // Check each root
    let t1 = clamp(roots.x - p_third, 0.0, 1.0);
    min_d = min(min_d, dist_sq_to_bezier(p, p0, p1, p2, t1));

    if num_roots >= 2 {
        let t2 = clamp(roots.y - p_third, 0.0, 1.0);
        min_d = min(min_d, dist_sq_to_bezier(p, p0, p1, p2, t2));
    }

    if num_roots >= 3 {
        let t3 = clamp(roots.z - p_third, 0.0, 1.0);
        min_d = min(min_d, dist_sq_to_bezier(p, p0, p1, p2, t3));
    }

    return min_d;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Compute minimum distance to curve (in local space)
    let dist_sq = min_dist_sq_to_bezier(in.local_pos, in.p0, in.p1, in.p2);
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
