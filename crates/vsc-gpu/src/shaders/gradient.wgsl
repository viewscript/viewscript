// =============================================================================
// gradient.wgsl - Linear Gradient Shader
// =============================================================================
//
// Implements CSS-compatible multi-stop linear gradient rendering.
// Gradient stops are evaluated per-fragment for accurate color interpolation.
//
// ## Memory Layout (std140)
//
// GradientStop: 32 bytes
//   - color: vec4<f32> (16 bytes, 16-byte aligned)
//   - offset: f32 (4 bytes)
//   - _pad: 12 bytes (to reach 32-byte stride for array element alignment)
//
// GradientUniforms: 16 + 4 + 12 + (32 * 8) = 288 bytes
//   - start: vec2<f32> (8 bytes)
//   - end: vec2<f32> (8 bytes)
//   - stop_count: u32 (4 bytes)
//   - _pad: 12 bytes (padding before array for 16-byte alignment)
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
// Gradient Uniforms (Group 1)
// =============================================================================

struct GradientStop {
    color: vec4<f32>,  // Linear RGBA [0, 1]
    offset: f32,       // Position along gradient [0, 1]
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
}

struct GradientUniforms {
    start: vec2<f32>,              // Gradient start point in UV space
    end: vec2<f32>,                // Gradient end point in UV space
    stop_count: u32,               // Number of active stops (1..8)
    _pad1: u32,                    // Padding for 16-byte alignment
    _pad2: u32,
    _pad3: u32,
    stops: array<GradientStop, 8>, // Fixed-size stop array
}

@group(1) @binding(0)
var<uniform> gradient: GradientUniforms;

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

/// Compute gradient position t by projecting UV onto the gradient axis.
/// Returns value in [0, 1] (clamped).
fn compute_gradient_t(uv: vec2<f32>) -> f32 {
    let axis = gradient.end - gradient.start;
    let axis_length_sq = dot(axis, axis);

    // Handle degenerate case (start == end)
    if axis_length_sq < 0.0001 {
        return 0.0;
    }

    // Vector from start to current UV
    let to_point = uv - gradient.start;

    // Project onto axis and normalize
    let t = dot(to_point, axis) / axis_length_sq;

    // Clamp to [0, 1]
    return clamp(t, 0.0, 1.0);
}

/// Evaluate gradient color at position t.
/// Performs linear interpolation between stops.
fn evaluate_gradient(t: f32) -> vec4<f32> {
    let count = gradient.stop_count;

    // Edge case: no stops (shouldn't happen, but handle gracefully)
    if count == 0u {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // Edge case: single stop - return that color
    if count == 1u {
        return gradient.stops[0].color;
    }

    // Before first stop: clamp to first color
    if t <= gradient.stops[0].offset {
        return gradient.stops[0].color;
    }

    // After last stop: clamp to last color
    let last_idx = count - 1u;
    if t >= gradient.stops[last_idx].offset {
        return gradient.stops[last_idx].color;
    }

    // Find the two stops surrounding t
    for (var i = 1u; i < count; i = i + 1u) {
        let prev_stop = gradient.stops[i - 1u];
        let curr_stop = gradient.stops[i];

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
    return gradient.stops[last_idx].color;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Compute position along gradient axis
    let t = compute_gradient_t(in.uv);

    // Evaluate gradient color
    var color = evaluate_gradient(t);

    // Apply accumulated opacity from scene graph hierarchy
    color.a = color.a * transform.opacity;

    return color;
}
