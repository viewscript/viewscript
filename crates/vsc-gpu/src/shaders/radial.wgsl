// =============================================================================
// radial.wgsl - Radial Gradient Shader
// =============================================================================
//
// Implements CSS-compatible multi-stop radial gradient rendering.
// Supports both circular and elliptical gradients via separate x/y radii.
// Gradient stops are evaluated per-fragment for accurate color interpolation.
//
// ## Memory Layout (std140)
//
// GradientStop: 32 bytes
//   - color: vec4<f32> (16 bytes, 16-byte aligned)
//   - offset: f32 (4 bytes)
//   - _pad: 12 bytes (to reach 32-byte stride for array element alignment)
//
// RadialGradientUniforms: 32 + (32 * 8) = 288 bytes
//   - center: vec2<f32> (8 bytes)
//   - radius: vec2<f32> (8 bytes) - x/y radius for ellipse support
//   - focal_point: vec2<f32> (8 bytes) - optional focal point offset
//   - stop_count: u32 (4 bytes)
//   - _pad: 4 bytes (padding for 16-byte alignment before array)
//   - stops: array<GradientStop, 8> (256 bytes)

// =============================================================================
// Shared Transform Uniform (Group 0)
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
    // Accumulated opacity from scene graph hierarchy [0, 1]
    opacity: f32,
    // Padding for 16-byte alignment
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
}

@group(0) @binding(0)
var<uniform> transform: Transform;

// =============================================================================
// Radial Gradient Uniforms (Group 1)
// =============================================================================

struct GradientStop {
    color: vec4<f32>,  // Linear RGBA [0, 1]
    offset: f32,       // Position along gradient [0, 1]
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
}

struct RadialGradientUniforms {
    center: vec2<f32>,         // Center point in UV space
    radius: vec2<f32>,         // Radius x/y (ellipse support)
    focal_point: vec2<f32>,    // Optional focal point offset from center
    stop_count: u32,           // Number of active stops (1..8)
    _pad1: u32,                // Padding for 16-byte alignment
    stops: array<GradientStop, 8>, // Fixed-size stop array
}

@group(1) @binding(0)
var<uniform> radial_gradient: RadialGradientUniforms;

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
    out.uv = in.uv;

    return out;
}

// =============================================================================
// Fragment Shader
// =============================================================================

/// Compute radial gradient position t based on distance from center.
/// Uses ellipse-aware distance for non-circular gradients.
/// Returns value in [0, 1] (clamped).
fn compute_radial_t(uv: vec2<f32>) -> f32 {
    let center = radial_gradient.center;
    let radius = radial_gradient.radius;

    // Handle degenerate case (zero radius)
    if radius.x < 0.0001 || radius.y < 0.0001 {
        return 0.0;
    }

    // Vector from center to current UV, scaled by radius for ellipse support
    // This transforms the ellipse to a unit circle for distance calculation
    let to_point = (uv - center) / radius;

    // Euclidean distance in normalized space gives us t
    let t = length(to_point);

    // Clamp to [0, 1]
    return clamp(t, 0.0, 1.0);
}

/// Evaluate gradient color at position t.
/// Performs linear interpolation between stops.
fn evaluate_gradient(t: f32) -> vec4<f32> {
    let count = radial_gradient.stop_count;

    // Edge case: no stops (shouldn't happen, but handle gracefully)
    if count == 0u {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // Edge case: single stop - return that color
    if count == 1u {
        return radial_gradient.stops[0].color;
    }

    // Before first stop: clamp to first color
    if t <= radial_gradient.stops[0].offset {
        return radial_gradient.stops[0].color;
    }

    // After last stop: clamp to last color
    let last_idx = count - 1u;
    if t >= radial_gradient.stops[last_idx].offset {
        return radial_gradient.stops[last_idx].color;
    }

    // Find the two stops surrounding t
    for (var i = 1u; i < count; i = i + 1u) {
        let prev_stop = radial_gradient.stops[i - 1u];
        let curr_stop = radial_gradient.stops[i];

        if t <= curr_stop.offset {
            // t is between prev_stop and curr_stop
            let segment_length = curr_stop.offset - prev_stop.offset;

            // Handle degenerate segment (same offset)
            if segment_length < 0.0001 {
                return curr_stop.color;
            }

            // Compute local interpolation factor
            let local_t = (t - prev_stop.offset) / segment_length;

            // Linear interpolation in linear color space
            return mix(prev_stop.color, curr_stop.color, local_t);
        }
    }

    // Fallback (should not reach here)
    return radial_gradient.stops[last_idx].color;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Compute position along radial gradient (distance from center)
    let t = compute_radial_t(in.uv);

    // Evaluate gradient color
    var color = evaluate_gradient(t);

    // Apply accumulated opacity from scene graph hierarchy
    color.a = color.a * transform.opacity;

    return color;
}
