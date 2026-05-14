//! WGSL Shader Sources
//!
//! This module provides compile-time embedded shader source code for the wgpu renderer.
//!
//! ## Shader Architecture
//!
//! All shaders share the same vertex shader structure:
//! - Bind Group 0: Global transform (AffineTransform + viewport)
//! - Bind Group 1: Per-draw style parameters
//!
//! ```text
//! Bind Group Layout:
//!
//! Group 0 (global):
//!   @binding(0) transform: Transform (a, b, c, d, tx, ty, viewport_w, viewport_h)
//!
//! Group 1 (per-draw):
//!   @binding(0) solid_color: SolidColor (r, g, b, a)           [solid.wgsl]
//!   @binding(0) gradient: GradientParams                       [gradient.wgsl]
//!   @binding(1) stops: array<GradientStop, MAX_STOPS>          [gradient.wgsl]
//! ```

/// Solid color shader source (vertex + fragment).
/// Used for `FillStyle::Solid` and solid stroke colors.
pub const SOLID_WGSL: &str = include_str!("solid.wgsl");

/// Gradient shader source (vertex + fragment).
/// Used for `FillStyle::LinearGradient`.
pub const GRADIENT_WGSL: &str = include_str!("gradient.wgsl");

/// Radial gradient shader source (vertex + fragment).
/// Used for `FillStyle::RadialGradient`.
/// Supports both circular and elliptical gradients via separate x/y radii.
pub const RADIAL_WGSL: &str = include_str!("radial.wgsl");

/// Loop-Blinn quadratic Bezier curve shader (vertex + fragment).
/// Renders each quadratic curve as a single triangle with implicit function evaluation.
/// Uses smoothstep anti-aliasing instead of discard for better quality.
pub const LOOP_BLINN_WGSL: &str = include_str!("loop_blinn.wgsl");

/// Loop-Blinn cubic Bezier curve shader (vertex + fragment).
/// Renders each cubic curve as two triangles with implicit function evaluation.
/// Implicit function: f = k³ - l·m (vs u² - v for quadratic).
pub const LOOP_BLINN_CUBIC_WGSL: &str = include_str!("loop_blinn_cubic.wgsl");

/// SDF-based stroke shader for quadratic Bezier curves (vertex + fragment).
/// Renders strokes using Signed Distance Field evaluation with Cardano's formula.
/// Each curve segment is a bounding rectangle; fragment shader computes distance analytically.
pub const SDF_STROKE_WGSL: &str = include_str!("sdf_stroke.wgsl");

/// SDF-based stroke shader for cubic Bezier curves (vertex + fragment).
/// Renders strokes using Signed Distance Field evaluation with Newton's method.
/// Each curve segment is a bounding rectangle; fragment shader iteratively
/// refines the closest point using 5-point initial sampling + 4 Newton iterations.
pub const SDF_STROKE_CUBIC_WGSL: &str = include_str!("sdf_stroke_cubic.wgsl");

/// External texture sampling shader (vertex + fragment).
/// Used for `FillStyle::ExternalTexture` (images, videos, canvas).
/// Phase J-3 implementation.
pub const TEXTURE_WGSL: &str = include_str!("texture.wgsl");

/// Maximum number of gradient stops (Section 9.5: static fixed count).
pub const MAX_GRADIENT_STOPS: usize = 8;

// =============================================================================
// Color Parsing Utilities
// =============================================================================

/// Parse CSS hex color string to packed RGBA bytes.
///
/// Supports: #RGB, #RGBA, #RRGGBB, #RRGGBBAA formats.
/// Returns [r, g, b, a] with a=255 for formats without alpha.
///
/// ## Example
/// ```ignore
/// assert_eq!(hex_to_rgba("#ff0000"), Some([255, 0, 0, 255]));
/// assert_eq!(hex_to_rgba("#0f0"), Some([0, 255, 0, 255]));
/// ```
pub fn hex_to_rgba(hex: &str) -> Option<[u8; 4]> {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            Some([r, g, b, 255])
        }
        4 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
            Some([r, g, b, a])
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some([r, g, b, 255])
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some([r, g, b, a])
        }
        _ => None,
    }
}

// =============================================================================
// Uniform Buffer Layouts (Rust-side definitions matching WGSL structs)
// =============================================================================

/// Transform uniform buffer layout (matches WGSL `Transform` struct).
///
/// ## Memory Layout
/// ```text
/// Offset  Size  Field
/// 0       4     a
/// 4       4     b
/// 8       4     c
/// 12      4     d
/// 16      4     tx
/// 20      4     ty
/// 24      4     viewport_width
/// 28      4     viewport_height
/// 32      4     opacity
/// 36      12    _pad (padding for 16-byte alignment)
/// Total: 48 bytes
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TransformUniform {
    /// Affine matrix element a (scale x / rotation cos)
    pub a: f32,
    /// Affine matrix element b (shear x / rotation -sin)
    pub b: f32,
    /// Affine matrix element c (shear y / rotation sin)
    pub c: f32,
    /// Affine matrix element d (scale y / rotation cos)
    pub d: f32,
    /// Translation x
    pub tx: f32,
    /// Translation y
    pub ty: f32,
    /// Viewport width in device pixels
    pub viewport_width: f32,
    /// Viewport height in device pixels
    pub viewport_height: f32,
    /// Accumulated opacity from scene graph hierarchy [0, 1]
    pub opacity: f32,
    /// Padding for 16-byte alignment (total 48 bytes)
    pub _pad: [f32; 3],
}

impl TransformUniform {
    /// Create an identity transform for a given viewport size.
    pub fn identity(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
            viewport_width,
            viewport_height,
            opacity: 1.0,
            _pad: [0.0; 3],
        }
    }

    /// Create from an AffineTransform and viewport size with default opacity (1.0).
    ///
    /// Note: AffineTransform uses f64 fields (Section 9.6 exception for transforms).
    pub fn from_affine(
        transform: &crate::AffineTransform,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Self {
        Self::from_affine_with_opacity(transform, viewport_width, viewport_height, 1.0)
    }

    /// Create from an AffineTransform, viewport size, and accumulated opacity.
    ///
    /// This is the primary constructor for rendering with opacity support.
    /// The opacity value should come from OpacityStack::current().
    pub fn from_affine_with_opacity(
        transform: &crate::AffineTransform,
        viewport_width: f32,
        viewport_height: f32,
        opacity: f32,
    ) -> Self {
        Self {
            a: transform.a as f32,
            b: transform.b as f32,
            c: transform.c as f32,
            d: transform.d as f32,
            tx: transform.tx as f32,
            ty: transform.ty as f32,
            viewport_width,
            viewport_height,
            opacity: opacity.clamp(0.0, 1.0),
            _pad: [0.0; 3],
        }
    }
}

/// Solid color uniform buffer layout (matches WGSL `SolidColor` struct).
///
/// ## Memory Layout
/// ```text
/// Offset  Size  Field
/// 0       4     r
/// 4       4     g
/// 8       4     b
/// 12      4     a
/// Total: 16 bytes
/// ```
///
/// ## Phase 17 Integration Note
///
/// Currently `FillStyle::Solid { color: String }` uses hex string representation.
/// However, Phase 17 specification integrates color channels as individual Rationals
/// in the constraint system, enabling dynamic expressions like:
///
/// ```ignore
/// stop.r = 255 * T.hover
/// ```
///
/// When the solver resolves such constraints, colors arrive as `[Rational; 4]` (RGBA),
/// not hex strings. The `from_hex()` method handles static colors, but dynamic
/// constraint-driven colors will require `from_rational_rgba()`.
///
/// This will be addressed alongside `gradient.wgsl` implementation, as gradient stops
/// face the same architectural requirement.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SolidColorUniform {
    /// Red component [0, 1]
    pub r: f32,
    /// Green component [0, 1]
    pub g: f32,
    /// Blue component [0, 1]
    pub b: f32,
    /// Alpha component [0, 1]
    pub a: f32,
}

impl SolidColorUniform {
    /// Create from RGBA values in [0, 1] range.
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Create from 8-bit RGBA values [0, 255].
    pub fn from_u8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    /// Create from Rational RGBA values (Phase 17 constraint system integration).
    ///
    /// This is the primary entry point for colors resolved by the solver from
    /// dynamic constraints like `stop.r = 255 * T.hover`.
    ///
    /// Conversion happens at the RASTERIZATION BOUNDARY.
    ///
    /// ## Out-of-Range Clamping
    ///
    /// Constraint-solver values may exceed `[0, 255]` (e.g., 300, -10) due to
    /// dynamic expressions. Values are clamped to `[0.0, 1.0]` after normalization
    /// to guarantee well-formed GPU uniform data.
    pub fn from_rational_rgba(
        r: &vsc_core::Rational,
        g: &vsc_core::Rational,
        b: &vsc_core::Rational,
        a: &vsc_core::Rational,
    ) -> Self {
        // Rational values are in [0, 255] range from constraint system.
        // Clamp to [0.0, 1.0] to handle out-of-range constraint outputs.
        Self {
            r: (r.to_f64_for_rasterization() / 255.0).clamp(0.0, 1.0) as f32,
            g: (g.to_f64_for_rasterization() / 255.0).clamp(0.0, 1.0) as f32,
            b: (b.to_f64_for_rasterization() / 255.0).clamp(0.0, 1.0) as f32,
            a: (a.to_f64_for_rasterization() / 255.0).clamp(0.0, 1.0) as f32,
        }
    }

    /// Create from packed RGBA bytes [r, g, b, a].
    ///
    /// This is the zero-copy path for pre-parsed colors.
    #[inline]
    pub fn from_rgba(rgba: [u8; 4]) -> Self {
        Self::from_u8(rgba[0], rgba[1], rgba[2], rgba[3])
    }

    /// Parse from CSS hex color string (e.g., "#ff0000", "#f00", "#ff0000ff").
    ///
    /// Note: This handles static colors only. For dynamic constraint-driven colors,
    /// use `from_rational_rgba()`.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        match hex.len() {
            // #RGB
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some(Self::from_u8(r, g, b, 255))
            }
            // #RGBA
            4 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
                Some(Self::from_u8(r, g, b, a))
            }
            // #RRGGBB
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Self::from_u8(r, g, b, 255))
            }
            // #RRGGBBAA
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(Self::from_u8(r, g, b, a))
            }
            _ => None,
        }
    }
}

// =============================================================================
// Gradient Uniform Structures
// =============================================================================

/// Single gradient stop uniform (matches WGSL `GradientStop` struct).
///
/// ## Memory Layout (std140)
/// ```text
/// Offset  Size  Field
/// 0       16    color (vec4<f32>)
/// 16      4     offset
/// 20      4     _pad1
/// 24      4     _pad2
/// 28      4     _pad3
/// Total: 32 bytes (required for array element alignment)
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GradientStopUniform {
    /// RGBA color in linear space [0, 1]
    pub color: [f32; 4],
    /// Position along gradient [0, 1]
    pub offset: f32,
    /// Padding for 32-byte stride (std140 array alignment)
    pub _pad1: f32,
    pub _pad2: f32,
    pub _pad3: f32,
}

impl GradientStopUniform {
    /// Create a new gradient stop.
    pub fn new(r: f32, g: f32, b: f32, a: f32, offset: f32) -> Self {
        Self {
            color: [r, g, b, a],
            offset,
            _pad1: 0.0,
            _pad2: 0.0,
            _pad3: 0.0,
        }
    }

    /// Create from Rational RGBA values (Phase 17 constraint system).
    ///
    /// ## Out-of-Range Clamping
    ///
    /// Constraint-solver values may exceed valid ranges due to dynamic expressions.
    /// - Color channels: Values are divided by 255 then clamped to `[0.0, 1.0]`.
    /// - Position (offset): Clamped to `[0.0, 1.0]`.
    ///
    /// When clamping occurs, a warning is logged to aid debugging.
    pub fn from_rational(
        r: &vsc_core::Rational,
        g: &vsc_core::Rational,
        b: &vsc_core::Rational,
        a: &vsc_core::Rational,
        offset: &vsc_core::Rational,
    ) -> Self {
        // Helper to clamp and warn
        fn clamp_and_warn(value: f64, min: f64, max: f64, field: &str) -> f32 {
            let clamped = value.clamp(min, max) as f32;
            if (value - clamped as f64).abs() > 1e-9 {
                log::warn!("GradientStopUniform {} value {} clamped to {}", field, value, clamped);
            }
            clamped
        }

        let r_val = r.to_f64_for_rasterization() / 255.0;
        let g_val = g.to_f64_for_rasterization() / 255.0;
        let b_val = b.to_f64_for_rasterization() / 255.0;
        let a_val = a.to_f64_for_rasterization() / 255.0;
        let offset_val = offset.to_f64_for_rasterization();

        Self {
            color: [
                clamp_and_warn(r_val, 0.0, 1.0, "r"),
                clamp_and_warn(g_val, 0.0, 1.0, "g"),
                clamp_and_warn(b_val, 0.0, 1.0, "b"),
                clamp_and_warn(a_val, 0.0, 1.0, "a"),
            ],
            offset: clamp_and_warn(offset_val, 0.0, 1.0, "offset"),
            _pad1: 0.0,
            _pad2: 0.0,
            _pad3: 0.0,
        }
    }
}

/// Gradient uniform buffer layout (matches WGSL `GradientUniforms` struct).
///
/// ## Memory Layout (std140)
/// ```text
/// Offset  Size  Field
/// 0       8     start (vec2<f32>)
/// 8       8     end (vec2<f32>)
/// 16      4     stop_count
/// 20      4     _pad1
/// 24      4     _pad2
/// 28      4     _pad3
/// 32      256   stops (array<GradientStop, 8>, each 32 bytes)
/// Total: 288 bytes
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GradientUniform {
    /// Gradient start point in UV space [0, 1]
    pub start: [f32; 2],
    /// Gradient end point in UV space [0, 1]
    pub end: [f32; 2],
    /// Number of active stops (1..MAX_GRADIENT_STOPS)
    pub stop_count: u32,
    /// Padding for 16-byte alignment before array
    pub _pad1: u32,
    pub _pad2: u32,
    pub _pad3: u32,
    /// Fixed-size stop array
    pub stops: [GradientStopUniform; MAX_GRADIENT_STOPS],
}

impl GradientUniform {
    /// Create a horizontal gradient (left to right).
    pub fn horizontal(stops: &[GradientStopUniform]) -> Self {
        Self::new([0.0, 0.5], [1.0, 0.5], stops)
    }

    /// Create a vertical gradient (top to bottom).
    pub fn vertical(stops: &[GradientStopUniform]) -> Self {
        Self::new([0.5, 0.0], [0.5, 1.0], stops)
    }

    /// Create a gradient with custom start/end points.
    pub fn new(start: [f32; 2], end: [f32; 2], stops: &[GradientStopUniform]) -> Self {
        let stop_count = stops.len().min(MAX_GRADIENT_STOPS) as u32;

        let mut stops_array = [GradientStopUniform::new(0.0, 0.0, 0.0, 1.0, 0.0); MAX_GRADIENT_STOPS];
        for (i, stop) in stops.iter().take(MAX_GRADIENT_STOPS).enumerate() {
            stops_array[i] = *stop;
        }

        Self {
            start,
            end,
            stop_count,
            _pad1: 0,
            _pad2: 0,
            _pad3: 0,
            stops: stops_array,
        }
    }

    /// Create from angle (degrees) specification.
    ///
    /// Converts angle to start/end UV coordinates.
    /// CSS convention: 0° = to top, 90° = to right, 180° = to bottom, etc.
    ///
    /// This is retained for future CSS `angle` parameter support.
    pub fn from_angle(angle_degrees: f64, stops: &[GradientStopUniform]) -> Self {
        // CSS: 0° = to top (upward), angles increase clockwise
        // Convert to math convention: 0° = right, counter-clockwise
        let angle_rad = (90.0 - angle_degrees).to_radians();
        let dx = angle_rad.cos() as f32;
        let dy = -angle_rad.sin() as f32; // Negate for screen coordinates (Y down)

        // Center at (0.5, 0.5), extend to edges
        let start = [0.5 - dx * 0.5, 0.5 - dy * 0.5];
        let end = [0.5 + dx * 0.5, 0.5 + dy * 0.5];

        Self::new(start, end, stops)
    }

    /// Create from FillStyle::LinearGradient with optional start/end points.
    ///
    /// This is the primary entry point for converting `FillStyle::LinearGradient`
    /// to a GPU-ready uniform buffer.
    ///
    /// ## Default Behavior
    ///
    /// If `start` or `end` is `None`, defaults to CSS 180° (top to bottom):
    /// - start = (0.5, 0.0) — top center
    /// - end = (0.5, 1.0) — bottom center
    ///
    /// ## Coordinate System
    ///
    /// Points are in UV space [0, 1] where:
    /// - (0, 0) = top-left
    /// - (1, 1) = bottom-right
    pub fn from_linear_gradient_points(
        start: Option<&crate::GradientPoint>,
        end: Option<&crate::GradientPoint>,
        stops: &[GradientStopUniform],
    ) -> Self {
        let (start_uv, end_uv) = Self::resolve_start_end(start, end);
        Self::new(start_uv, end_uv, stops)
    }

    /// Resolve optional start/end points to concrete UV coordinates.
    ///
    /// Default (CSS 180°): top-to-bottom gradient
    /// - start = (0.5, 0.0)
    /// - end = (0.5, 1.0)
    fn resolve_start_end(
        start: Option<&crate::GradientPoint>,
        end: Option<&crate::GradientPoint>,
    ) -> ([f32; 2], [f32; 2]) {
        // CSS default: 180° = top to bottom
        const DEFAULT_START: [f32; 2] = [0.5, 0.0]; // Top center
        const DEFAULT_END: [f32; 2] = [0.5, 1.0];   // Bottom center

        let start_uv = match start {
            Some(pt) => [
                pt.x.to_f64_for_rasterization() as f32,
                pt.y.to_f64_for_rasterization() as f32,
            ],
            None => DEFAULT_START,
        };

        let end_uv = match end {
            Some(pt) => [
                pt.x.to_f64_for_rasterization() as f32,
                pt.y.to_f64_for_rasterization() as f32,
            ],
            None => DEFAULT_END,
        };

        (start_uv, end_uv)
    }
}

// =============================================================================
// Radial Gradient Uniform Structure
// =============================================================================

/// Radial gradient uniform buffer layout (matches WGSL `RadialGradientUniforms` struct).
///
/// ## Memory Layout (std140)
/// ```text
/// Offset  Size  Field
/// 0       8     center (vec2<f32>)
/// 8       8     radius (vec2<f32>) - x/y for ellipse support
/// 16      8     focal_point (vec2<f32>)
/// 24      4     stop_count
/// 28      4     _pad1
/// 32      256   stops (array<GradientStop, 8>, each 32 bytes)
/// Total: 288 bytes
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RadialGradientUniform {
    /// Center point in UV space [0, 1]
    pub center: [f32; 2],
    /// Radius x/y in UV space (allows elliptical gradients)
    pub radius: [f32; 2],
    /// Focal point offset from center (reserved for future use)
    pub focal_point: [f32; 2],
    /// Number of active stops (1..MAX_GRADIENT_STOPS)
    pub stop_count: u32,
    /// Padding for 16-byte alignment before array
    pub _pad1: u32,
    /// Fixed-size stop array
    pub stops: [GradientStopUniform; MAX_GRADIENT_STOPS],
}

impl RadialGradientUniform {
    /// Create a circular radial gradient centered at (0.5, 0.5) with radius 0.5.
    pub fn centered(stops: &[GradientStopUniform]) -> Self {
        Self::new([0.5, 0.5], [0.5, 0.5], stops)
    }

    /// Create a radial gradient with custom center and uniform radius (circular).
    pub fn circular(center: [f32; 2], radius: f32, stops: &[GradientStopUniform]) -> Self {
        Self::new(center, [radius, radius], stops)
    }

    /// Create an elliptical radial gradient with separate x/y radii.
    pub fn elliptical(center: [f32; 2], radius: [f32; 2], stops: &[GradientStopUniform]) -> Self {
        Self::new(center, radius, stops)
    }

    /// Create a radial gradient with custom center and radius.
    pub fn new(center: [f32; 2], radius: [f32; 2], stops: &[GradientStopUniform]) -> Self {
        let stop_count = stops.len().min(MAX_GRADIENT_STOPS) as u32;

        let mut stops_array = [GradientStopUniform::new(0.0, 0.0, 0.0, 1.0, 0.0); MAX_GRADIENT_STOPS];
        for (i, stop) in stops.iter().take(MAX_GRADIENT_STOPS).enumerate() {
            stops_array[i] = *stop;
        }

        Self {
            center,
            radius,
            focal_point: [0.0, 0.0], // Reserved for future focal point support
            stop_count,
            _pad1: 0,
            stops: stops_array,
        }
    }

    /// Create from FillStyle::RadialGradient fields.
    ///
    /// ## Default Behavior
    ///
    /// If `center` is `None`, defaults to (0.5, 0.5) - shape center.
    /// If `radius` is `None`, defaults to 0.5 - edge of normalized UV space.
    pub fn from_radial_gradient(
        center: Option<&crate::GradientPoint>,
        radius: Option<&vsc_core::Rational>,
        stops: &[GradientStopUniform],
    ) -> Self {
        // Default center: shape center
        let center_uv = match center {
            Some(pt) => [
                pt.x.to_f64_for_rasterization() as f32,
                pt.y.to_f64_for_rasterization() as f32,
            ],
            None => [0.5, 0.5],
        };

        // Default radius: 0.5 (reaches edge of normalized space)
        // Currently only supports uniform radius (circular), but structure
        // supports elliptical for future CSS "closest-side" / "farthest-corner" etc.
        let r = match radius {
            Some(r) => r.to_f64_for_rasterization() as f32,
            None => 0.5,
        };

        Self::new(center_uv, [r, r], stops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solid_color_from_hex() {
        // 6-digit hex
        let color = SolidColorUniform::from_hex("#ff0000").unwrap();
        assert!((color.r - 1.0).abs() < 0.01);
        assert!(color.g.abs() < 0.01);
        assert!(color.b.abs() < 0.01);
        assert!((color.a - 1.0).abs() < 0.01);

        // 3-digit hex
        let color = SolidColorUniform::from_hex("#0f0").unwrap();
        assert!(color.r.abs() < 0.01);
        assert!((color.g - 1.0).abs() < 0.01);
        assert!(color.b.abs() < 0.01);

        // 8-digit hex with alpha
        let color = SolidColorUniform::from_hex("#0000ff80").unwrap();
        assert!(color.r.abs() < 0.01);
        assert!(color.g.abs() < 0.01);
        assert!((color.b - 1.0).abs() < 0.01);
        assert!((color.a - 0.5).abs() < 0.02);
    }

    #[test]
    fn test_solid_color_from_rational_rgba() {
        use vsc_core::Rational;

        // Pure red (255, 0, 0, 255) in Rational
        let color = SolidColorUniform::from_rational_rgba(
            &Rational::from_int(255),
            &Rational::zero(),
            &Rational::zero(),
            &Rational::from_int(255),
        );
        assert!((color.r - 1.0).abs() < 0.01);
        assert!(color.g.abs() < 0.01);
        assert!(color.b.abs() < 0.01);
        assert!((color.a - 1.0).abs() < 0.01);

        // Half-transparent green using fractional Rational (127.5 = 255/2)
        let half = Rational::new(255, 2);
        let color = SolidColorUniform::from_rational_rgba(
            &Rational::zero(),
            &Rational::from_int(255),
            &Rational::zero(),
            &half,
        );
        assert!(color.r.abs() < 0.01);
        assert!((color.g - 1.0).abs() < 0.01);
        assert!(color.b.abs() < 0.01);
        assert!((color.a - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_transform_identity() {
        let t = TransformUniform::identity(800.0, 600.0);
        assert_eq!(t.a, 1.0);
        assert_eq!(t.b, 0.0);
        assert_eq!(t.c, 0.0);
        assert_eq!(t.d, 1.0);
        assert_eq!(t.tx, 0.0);
        assert_eq!(t.ty, 0.0);
        assert_eq!(t.viewport_width, 800.0);
        assert_eq!(t.viewport_height, 600.0);
    }

    #[test]
    fn test_uniform_sizes() {
        // Verify struct sizes match WGSL expectations (std140 alignment)
        assert_eq!(std::mem::size_of::<TransformUniform>(), 48);
        assert_eq!(std::mem::size_of::<SolidColorUniform>(), 16);

        // Gradient uniform sizes (critical for GPU buffer layout)
        assert_eq!(
            std::mem::size_of::<GradientStopUniform>(),
            32,
            "GradientStopUniform must be 32 bytes for std140 array alignment"
        );
        assert_eq!(
            std::mem::size_of::<GradientUniform>(),
            288,
            "GradientUniform must be 288 bytes (32 header + 256 stops)"
        );

        // Radial gradient uniform size (must match WGSL RadialGradientUniforms layout)
        assert_eq!(
            std::mem::size_of::<RadialGradientUniform>(),
            288,
            "RadialGradientUniform must be 288 bytes (32 header + 256 stops)"
        );
    }

    #[test]
    fn test_shader_source_not_empty() {
        assert!(!SOLID_WGSL.is_empty());
        assert!(SOLID_WGSL.contains("vs_main"));
        assert!(SOLID_WGSL.contains("fs_main"));

        // Gradient shader
        assert!(!GRADIENT_WGSL.is_empty());
        assert!(GRADIENT_WGSL.contains("vs_main"));
        assert!(GRADIENT_WGSL.contains("fs_main"));
        assert!(GRADIENT_WGSL.contains("GradientStop"));
        assert!(GRADIENT_WGSL.contains("evaluate_gradient"));

        // Radial gradient shader
        assert!(!RADIAL_WGSL.is_empty());
        assert!(RADIAL_WGSL.contains("vs_main"));
        assert!(RADIAL_WGSL.contains("fs_main"));
        assert!(RADIAL_WGSL.contains("RadialGradientUniforms"));
        assert!(RADIAL_WGSL.contains("compute_radial_t"));
    }

    // =========================================================================
    // Gradient Tests
    // =========================================================================

    #[test]
    fn test_gradient_stop_creation() {
        let stop = GradientStopUniform::new(1.0, 0.0, 0.0, 1.0, 0.5);
        assert_eq!(stop.color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(stop.offset, 0.5);
    }

    #[test]
    fn test_gradient_horizontal() {
        let stops = [
            GradientStopUniform::new(0.0, 0.0, 0.0, 1.0, 0.0), // Black at 0
            GradientStopUniform::new(1.0, 1.0, 1.0, 1.0, 1.0), // White at 1
        ];

        let gradient = GradientUniform::horizontal(&stops);

        assert_eq!(gradient.start, [0.0, 0.5]);
        assert_eq!(gradient.end, [1.0, 0.5]);
        assert_eq!(gradient.stop_count, 2);
    }

    #[test]
    fn test_gradient_vertical() {
        let stops = [
            GradientStopUniform::new(1.0, 0.0, 0.0, 1.0, 0.0), // Red at top
            GradientStopUniform::new(0.0, 0.0, 1.0, 1.0, 1.0), // Blue at bottom
        ];

        let gradient = GradientUniform::vertical(&stops);

        assert_eq!(gradient.start, [0.5, 0.0]);
        assert_eq!(gradient.end, [0.5, 1.0]);
        assert_eq!(gradient.stop_count, 2);
    }

    #[test]
    fn test_gradient_from_angle() {
        // 0° = to top (CSS convention)
        let stops = [GradientStopUniform::new(0.0, 0.0, 0.0, 1.0, 0.0)];
        let gradient = GradientUniform::from_angle(0.0, &stops);

        // Should be vertical, bottom to top
        // start.y > end.y (start at bottom, end at top)
        assert!(
            gradient.start[1] > gradient.end[1],
            "0° gradient should go bottom to top"
        );

        // 90° = to right
        let gradient = GradientUniform::from_angle(90.0, &stops);
        assert!(
            gradient.end[0] > gradient.start[0],
            "90° gradient should go left to right"
        );
    }

    #[test]
    fn test_gradient_default_none_none() {
        // When start and end are both None, should default to CSS 180° (top to bottom)
        let stops = [
            GradientStopUniform::new(1.0, 0.0, 0.0, 1.0, 0.0), // Red at top
            GradientStopUniform::new(0.0, 0.0, 1.0, 1.0, 1.0), // Blue at bottom
        ];

        let gradient = GradientUniform::from_linear_gradient_points(None, None, &stops);

        // Default: start = (0.5, 0.0), end = (0.5, 1.0) — top to bottom
        assert!(
            (gradient.start[0] - 0.5).abs() < 0.01,
            "Default start.x should be 0.5"
        );
        assert!(
            gradient.start[1].abs() < 0.01,
            "Default start.y should be 0.0 (top)"
        );
        assert!(
            (gradient.end[0] - 0.5).abs() < 0.01,
            "Default end.x should be 0.5"
        );
        assert!(
            (gradient.end[1] - 1.0).abs() < 0.01,
            "Default end.y should be 1.0 (bottom)"
        );

        // Verify it's a vertical gradient (top to bottom)
        assert!(
            gradient.end[1] > gradient.start[1],
            "Default should be top-to-bottom (end.y > start.y)"
        );
    }

    #[test]
    fn test_gradient_diagonal_points() {
        use vsc_core::Rational;

        // Diagonal gradient from top-left (0,0) to bottom-right (1,1)
        let start_pt = crate::GradientPoint {
            x: Rational::zero(),
            y: Rational::zero(),
        };
        let end_pt = crate::GradientPoint {
            x: Rational::one(),
            y: Rational::one(),
        };

        let stops = [
            GradientStopUniform::new(1.0, 1.0, 1.0, 1.0, 0.0), // White at start
            GradientStopUniform::new(0.0, 0.0, 0.0, 1.0, 1.0), // Black at end
        ];

        let gradient =
            GradientUniform::from_linear_gradient_points(Some(&start_pt), Some(&end_pt), &stops);

        // Start should be (0, 0)
        assert!(gradient.start[0].abs() < 0.01, "start.x should be 0");
        assert!(gradient.start[1].abs() < 0.01, "start.y should be 0");

        // End should be (1, 1)
        assert!((gradient.end[0] - 1.0).abs() < 0.01, "end.x should be 1");
        assert!((gradient.end[1] - 1.0).abs() < 0.01, "end.y should be 1");

        // Test color evaluation: center (0.5, 0.5) should project to t=0.5
        // For diagonal, the projection of (0.5, 0.5) onto (0,0)→(1,1) is exactly midpoint
        // So color should be mid-gray
        let t = compute_gradient_t_cpu(&gradient, [0.5, 0.5]);
        assert!(
            (t - 0.5).abs() < 0.01,
            "Center point should project to t=0.5 on diagonal, got {}",
            t
        );

        let color = evaluate_gradient_cpu(&gradient, t);
        assert!(
            (color[0] - 0.5).abs() < 0.01,
            "Diagonal midpoint should be mid-gray"
        );
    }

    /// Compute gradient t value (mirrors WGSL compute_gradient_t)
    fn compute_gradient_t_cpu(gradient: &GradientUniform, uv: [f32; 2]) -> f32 {
        let axis = [
            gradient.end[0] - gradient.start[0],
            gradient.end[1] - gradient.start[1],
        ];
        let axis_length_sq = axis[0] * axis[0] + axis[1] * axis[1];

        if axis_length_sq < 0.0001 {
            return 0.0;
        }

        let to_point = [uv[0] - gradient.start[0], uv[1] - gradient.start[1]];
        let dot = to_point[0] * axis[0] + to_point[1] * axis[1];
        let t = dot / axis_length_sq;

        t.clamp(0.0, 1.0)
    }

    /// Simulate gradient evaluation for testing.
    /// This mirrors the WGSL evaluate_gradient function.
    fn evaluate_gradient_cpu(gradient: &GradientUniform, t: f32) -> [f32; 4] {
        let count = gradient.stop_count as usize;

        if count == 0 {
            return [0.0, 0.0, 0.0, 1.0];
        }

        if count == 1 {
            return gradient.stops[0].color;
        }

        // Before first stop
        if t <= gradient.stops[0].offset {
            return gradient.stops[0].color;
        }

        // After last stop
        let last_idx = count - 1;
        if t >= gradient.stops[last_idx].offset {
            return gradient.stops[last_idx].color;
        }

        // Find surrounding stops
        for i in 1..count {
            let prev = &gradient.stops[i - 1];
            let curr = &gradient.stops[i];

            if t <= curr.offset {
                let segment = curr.offset - prev.offset;
                if segment < 0.0001 {
                    return curr.color;
                }

                let local_t = (t - prev.offset) / segment;

                // Linear interpolation
                return [
                    prev.color[0] + (curr.color[0] - prev.color[0]) * local_t,
                    prev.color[1] + (curr.color[1] - prev.color[1]) * local_t,
                    prev.color[2] + (curr.color[2] - prev.color[2]) * local_t,
                    prev.color[3] + (curr.color[3] - prev.color[3]) * local_t,
                ];
            }
        }

        gradient.stops[last_idx].color
    }

    #[test]
    fn test_gradient_2stop_interpolation() {
        // Black to White gradient
        let stops = [
            GradientStopUniform::new(0.0, 0.0, 0.0, 1.0, 0.0), // Black at 0
            GradientStopUniform::new(1.0, 1.0, 1.0, 1.0, 1.0), // White at 1
        ];
        let gradient = GradientUniform::horizontal(&stops);

        // t=0.5 should give mid-gray (0.5, 0.5, 0.5, 1.0)
        let color = evaluate_gradient_cpu(&gradient, 0.5);
        assert!(
            (color[0] - 0.5).abs() < 0.01,
            "R at t=0.5 should be ~0.5, got {}",
            color[0]
        );
        assert!(
            (color[1] - 0.5).abs() < 0.01,
            "G at t=0.5 should be ~0.5, got {}",
            color[1]
        );
        assert!(
            (color[2] - 0.5).abs() < 0.01,
            "B at t=0.5 should be ~0.5, got {}",
            color[2]
        );
        assert!(
            (color[3] - 1.0).abs() < 0.01,
            "A at t=0.5 should be ~1.0, got {}",
            color[3]
        );

        // t=0.0 should be black
        let color = evaluate_gradient_cpu(&gradient, 0.0);
        assert!(color[0].abs() < 0.01, "R at t=0 should be ~0");

        // t=1.0 should be white
        let color = evaluate_gradient_cpu(&gradient, 1.0);
        assert!((color[0] - 1.0).abs() < 0.01, "R at t=1 should be ~1");
    }

    #[test]
    fn test_gradient_3stop_interpolation() {
        // Red → Green → Blue gradient
        let stops = [
            GradientStopUniform::new(1.0, 0.0, 0.0, 1.0, 0.0),  // Red at 0
            GradientStopUniform::new(0.0, 1.0, 0.0, 1.0, 0.5),  // Green at 0.5
            GradientStopUniform::new(0.0, 0.0, 1.0, 1.0, 1.0),  // Blue at 1
        ];
        let gradient = GradientUniform::horizontal(&stops);

        // t=0.0 should be red
        let color = evaluate_gradient_cpu(&gradient, 0.0);
        assert!((color[0] - 1.0).abs() < 0.01, "Red at t=0");
        assert!(color[1].abs() < 0.01, "No green at t=0");

        // t=0.5 should be green
        let color = evaluate_gradient_cpu(&gradient, 0.5);
        assert!(color[0].abs() < 0.01, "No red at t=0.5");
        assert!((color[1] - 1.0).abs() < 0.01, "Green at t=0.5");
        assert!(color[2].abs() < 0.01, "No blue at t=0.5");

        // t=1.0 should be blue
        let color = evaluate_gradient_cpu(&gradient, 1.0);
        assert!(color[0].abs() < 0.01, "No red at t=1");
        assert!((color[2] - 1.0).abs() < 0.01, "Blue at t=1");

        // t=0.25 should be between red and green (orange-ish)
        let color = evaluate_gradient_cpu(&gradient, 0.25);
        assert!(
            (color[0] - 0.5).abs() < 0.01,
            "R at t=0.25 should be ~0.5, got {}",
            color[0]
        );
        assert!(
            (color[1] - 0.5).abs() < 0.01,
            "G at t=0.25 should be ~0.5, got {}",
            color[1]
        );

        // t=0.75 should be between green and blue (cyan-ish)
        let color = evaluate_gradient_cpu(&gradient, 0.75);
        assert!(
            (color[1] - 0.5).abs() < 0.01,
            "G at t=0.75 should be ~0.5, got {}",
            color[1]
        );
        assert!(
            (color[2] - 0.5).abs() < 0.01,
            "B at t=0.75 should be ~0.5, got {}",
            color[2]
        );
    }

    #[test]
    fn test_gradient_single_stop_degenerate() {
        // Single stop: all pixels should be this color
        let stops = [GradientStopUniform::new(0.5, 0.3, 0.8, 1.0, 0.5)];
        let gradient = GradientUniform::horizontal(&stops);

        // Any t value should return the same color
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let color = evaluate_gradient_cpu(&gradient, t);
            assert!(
                (color[0] - 0.5).abs() < 0.01,
                "Single stop: R should be 0.5 at t={}",
                t
            );
            assert!(
                (color[1] - 0.3).abs() < 0.01,
                "Single stop: G should be 0.3 at t={}",
                t
            );
            assert!(
                (color[2] - 0.8).abs() < 0.01,
                "Single stop: B should be 0.8 at t={}",
                t
            );
        }
    }

    #[test]
    fn test_gradient_zero_stops_degenerate() {
        // 0 stops: stop_count should be 0, no panic
        let gradient = GradientUniform::from_linear_gradient_points(None, None, &[]);
        assert_eq!(gradient.stop_count, 0, "0 stops should produce stop_count=0");

        // Verify evaluate_gradient_cpu handles 0 stops gracefully
        let color = evaluate_gradient_cpu(&gradient, 0.5);
        // Expected: black with alpha=1 (the fallback for count==0)
        assert_eq!(color, [0.0, 0.0, 0.0, 1.0], "0 stops should return black");
    }

    #[test]
    fn test_gradient_more_than_max_stops_clamped() {
        // 9 stops: should be silently clamped to MAX_GRADIENT_STOPS (8)
        let nine_stops: Vec<GradientStopUniform> = (0..9)
            .map(|i| GradientStopUniform::new(i as f32 / 8.0, 0.0, 0.0, 1.0, i as f32 / 8.0))
            .collect();

        let gradient = GradientUniform::from_linear_gradient_points(None, None, &nine_stops);

        assert_eq!(
            gradient.stop_count, MAX_GRADIENT_STOPS as u32,
            "9 stops should be clamped to MAX_GRADIENT_STOPS ({}), got {}",
            MAX_GRADIENT_STOPS, gradient.stop_count
        );

        // Verify all 8 stored stops are from the original array (first 8 only)
        for i in 0..MAX_GRADIENT_STOPS {
            let expected_offset = i as f32 / 8.0;
            assert!(
                (gradient.stops[i].offset - expected_offset).abs() < 0.001,
                "Stop[{}] offset should be {:.3}, got {:.3}",
                i, expected_offset, gradient.stops[i].offset
            );
        }
    }

    #[test]
    fn test_solid_color_from_rational_rgba_out_of_range_clamp() {
        use vsc_core::Rational;

        // r=300 (above 255), g=-10 (below 0), b=128, a=255
        let color = SolidColorUniform::from_rational_rgba(
            &Rational::from_int(300),   // above range → should clamp to 1.0
            &Rational::from_int(-10),   // below range → should clamp to 0.0
            &Rational::from_int(128),   // valid: 128/255 ≈ 0.502
            &Rational::from_int(255),   // valid: 1.0
        );

        assert!(
            (color.r - 1.0).abs() < 0.001,
            "r=300/255 should clamp to 1.0, got {}",
            color.r
        );
        assert!(
            color.g.abs() < 0.001,
            "g=-10/255 should clamp to 0.0, got {}",
            color.g
        );
        assert!(
            (color.b - 128.0 / 255.0).abs() < 0.005,
            "b=128/255 should be ~0.502, got {}",
            color.b
        );
        assert!(
            (color.a - 1.0).abs() < 0.001,
            "a=255/255 should be 1.0, got {}",
            color.a
        );

        // Verify GPU values are within valid range
        assert!(color.r >= 0.0 && color.r <= 1.0, "r out of [0,1]: {}", color.r);
        assert!(color.g >= 0.0 && color.g <= 1.0, "g out of [0,1]: {}", color.g);
        assert!(color.b >= 0.0 && color.b <= 1.0, "b out of [0,1]: {}", color.b);
        assert!(color.a >= 0.0 && color.a <= 1.0, "a out of [0,1]: {}", color.a);
    }

    #[test]
    fn test_gradient_clamp_boundaries() {
        let stops = [
            GradientStopUniform::new(1.0, 0.0, 0.0, 1.0, 0.2), // Red at 0.2
            GradientStopUniform::new(0.0, 0.0, 1.0, 1.0, 0.8), // Blue at 0.8
        ];
        let gradient = GradientUniform::horizontal(&stops);

        // t < first stop offset: should clamp to first color
        let color = evaluate_gradient_cpu(&gradient, 0.0);
        assert!((color[0] - 1.0).abs() < 0.01, "t=0 should clamp to red");
        assert!(color[2].abs() < 0.01, "t=0 should have no blue");

        let color = evaluate_gradient_cpu(&gradient, 0.1);
        assert!((color[0] - 1.0).abs() < 0.01, "t=0.1 should clamp to red");

        // t > last stop offset: should clamp to last color
        let color = evaluate_gradient_cpu(&gradient, 0.9);
        assert!((color[2] - 1.0).abs() < 0.01, "t=0.9 should clamp to blue");
        assert!(color[0].abs() < 0.01, "t=0.9 should have no red");

        let color = evaluate_gradient_cpu(&gradient, 1.0);
        assert!((color[2] - 1.0).abs() < 0.01, "t=1 should clamp to blue");
    }

    // =========================================================================
    // Loop-Blinn Shader Tests
    // =========================================================================

    #[test]
    fn test_loop_blinn_shader_source_not_empty() {
        assert!(!LOOP_BLINN_WGSL.is_empty());
        assert!(LOOP_BLINN_WGSL.contains("vs_main"));
        assert!(LOOP_BLINN_WGSL.contains("fs_main"));
        assert!(LOOP_BLINN_WGSL.contains("curve_uv"));
        assert!(LOOP_BLINN_WGSL.contains("curve_sign"));
        assert!(LOOP_BLINN_WGSL.contains("smoothstep"));
        assert!(LOOP_BLINN_WGSL.contains("fwidth"));
    }

    /// CPU emulation of the Loop-Blinn smoothstep anti-aliasing formula.
    ///
    /// This mirrors the WGSL fragment shader logic:
    /// ```wgsl
    /// let f = u * u - v;
    /// let fw = fwidth(f);
    /// let signed_f = f * curve_sign;
    /// let alpha = smoothstep(fw, -fw, signed_f);
    /// ```
    fn loop_blinn_alpha_cpu(u: f32, v: f32, curve_sign: f32, fwidth_f: f32) -> f32 {
        let f = u * u - v;
        let signed_f = f * curve_sign;

        // smoothstep(edge0, edge1, x) where edge0 > edge1 inverts the transition
        // smoothstep(fw, -fw, signed_f):
        //   signed_f <= -fw → 1.0 (fully inside)
        //   signed_f >= +fw → 0.0 (fully outside)
        //   in between → smooth transition
        fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
            let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }

        smoothstep(fwidth_f, -fwidth_f, signed_f)
    }

    #[test]
    fn test_loop_blinn_alpha_on_curve_boundary() {
        // On the curve (f = 0), alpha should be approximately 0.5
        // For u² - v = 0, we can use any point on the parabola v = u²
        // Let's use u = 0.5, v = 0.25 (exactly on curve)
        let u = 0.5;
        let v = 0.25; // u² = 0.25
        let curve_sign = 1.0; // convex
        let fwidth_f = 0.01; // typical screen-space derivative

        let alpha = loop_blinn_alpha_cpu(u, v, curve_sign, fwidth_f);

        // f = 0.5² - 0.25 = 0.0, so signed_f = 0.0
        // smoothstep(0.01, -0.01, 0.0) should be 0.5
        assert!(
            (alpha - 0.5).abs() < 0.01,
            "Alpha at curve boundary should be ~0.5, got {}",
            alpha
        );
    }

    #[test]
    fn test_loop_blinn_alpha_inside_curve() {
        // Inside the curve: f < 0 for convex (curve_sign = 1.0)
        // Use u = 0.5, v = 0.5 (above the parabola, so f < 0)
        let u = 0.5;
        let v = 0.5; // u² = 0.25 < 0.5, so f = 0.25 - 0.5 = -0.25
        let curve_sign = 1.0;
        let fwidth_f = 0.01;

        let alpha = loop_blinn_alpha_cpu(u, v, curve_sign, fwidth_f);

        // signed_f = -0.25 * 1.0 = -0.25, which is << -fwidth_f
        // So alpha should be ~1.0 (fully inside)
        assert!(
            alpha > 0.99,
            "Alpha inside curve should be ~1.0, got {}",
            alpha
        );
    }

    #[test]
    fn test_loop_blinn_alpha_outside_curve() {
        // Outside the curve: f > 0 for convex (curve_sign = 1.0)
        // Use u = 0.5, v = 0.0 (below the parabola, so f > 0)
        let u = 0.5;
        let v = 0.0; // u² = 0.25 > 0.0, so f = 0.25 - 0.0 = 0.25
        let curve_sign = 1.0;
        let fwidth_f = 0.01;

        let alpha = loop_blinn_alpha_cpu(u, v, curve_sign, fwidth_f);

        // signed_f = 0.25 * 1.0 = 0.25, which is >> +fwidth_f
        // So alpha should be ~0.0 (fully outside)
        assert!(
            alpha < 0.01,
            "Alpha outside curve should be ~0.0, got {}",
            alpha
        );
    }

    #[test]
    fn test_loop_blinn_alpha_concave_curve() {
        // For concave curve (curve_sign = -1.0), the inside/outside is flipped
        // Inside: f > 0, outside: f < 0
        let u = 0.5;
        let v = 0.0; // f = 0.25 > 0
        let curve_sign = -1.0; // concave
        let fwidth_f = 0.01;

        let alpha = loop_blinn_alpha_cpu(u, v, curve_sign, fwidth_f);

        // signed_f = 0.25 * (-1.0) = -0.25 << -fwidth_f
        // So alpha should be ~1.0 (fully inside for concave)
        assert!(
            alpha > 0.99,
            "Alpha inside concave curve should be ~1.0, got {}",
            alpha
        );

        // Now test outside for concave
        let v = 0.5; // f = -0.25 < 0
        let alpha = loop_blinn_alpha_cpu(u, v, curve_sign, fwidth_f);

        // signed_f = -0.25 * (-1.0) = 0.25 >> +fwidth_f
        // So alpha should be ~0.0 (fully outside for concave)
        assert!(
            alpha < 0.01,
            "Alpha outside concave curve should be ~0.0, got {}",
            alpha
        );
    }

    #[test]
    fn test_loop_blinn_alpha_anti_aliasing_transition() {
        // Test that the transition zone works correctly
        // Use points near the curve boundary within the fwidth range
        let curve_sign = 1.0;
        let fwidth_f = 0.1; // larger fwidth for easier testing

        // At u = 0.5, curve is at v = 0.25
        // Test at v = 0.25 + small offset
        let u = 0.5;

        // Just inside (f slightly negative)
        let v_inside = 0.25 + 0.05; // f = 0.25 - 0.30 = -0.05
        let alpha_inside = loop_blinn_alpha_cpu(u, v_inside, curve_sign, fwidth_f);

        // Just outside (f slightly positive)
        let v_outside = 0.25 - 0.05; // f = 0.25 - 0.20 = +0.05
        let alpha_outside = loop_blinn_alpha_cpu(u, v_outside, curve_sign, fwidth_f);

        // Alpha should be higher for inside than outside
        assert!(
            alpha_inside > alpha_outside,
            "Alpha inside ({}) should be > alpha outside ({})",
            alpha_inside,
            alpha_outside
        );

        // Both should be in the transition range (not 0 or 1)
        assert!(
            alpha_inside > 0.3 && alpha_inside < 1.0,
            "Alpha inside should be in transition zone, got {}",
            alpha_inside
        );
        assert!(
            alpha_outside > 0.0 && alpha_outside < 0.7,
            "Alpha outside should be in transition zone, got {}",
            alpha_outside
        );
    }

    // =========================================================================
    // SDF Stroke Shader Tests (Cardano's Formula CPU Emulation)
    // =========================================================================

    #[test]
    fn test_sdf_stroke_shader_source_not_empty() {
        assert!(!SDF_STROKE_WGSL.is_empty());
        assert!(SDF_STROKE_WGSL.contains("vs_main"));
        assert!(SDF_STROKE_WGSL.contains("fs_main"));
        assert!(SDF_STROKE_WGSL.contains("solve_depressed_cubic"));
        assert!(SDF_STROKE_WGSL.contains("min_dist_sq_to_bezier"));
        assert!(SDF_STROKE_WGSL.contains("smoothstep"));
        assert!(SDF_STROKE_WGSL.contains("fwidth"));
    }

    /// Evaluate quadratic Bezier at parameter t.
    /// B(t) = (1-t)²P₀ + 2t(1-t)P₁ + t²P₂
    fn eval_bezier_cpu(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], t: f32) -> [f32; 2] {
        let mt = 1.0 - t;
        [
            mt * mt * p0[0] + 2.0 * mt * t * p1[0] + t * t * p2[0],
            mt * mt * p0[1] + 2.0 * mt * t * p1[1] + t * t * p2[1],
        ]
    }

    /// Distance squared from point to bezier at parameter t.
    fn dist_sq_to_bezier_cpu(p: [f32; 2], p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], t: f32) -> f32 {
        let b = eval_bezier_cpu(p0, p1, p2, t);
        let dx = p[0] - b[0];
        let dy = p[1] - b[1];
        dx * dx + dy * dy
    }

    /// Cube root that handles negative values.
    fn cbrt_cpu(x: f32) -> f32 {
        if x >= 0.0 {
            x.powf(1.0 / 3.0)
        } else {
            -(-x).powf(1.0 / 3.0)
        }
    }

    const EPSILON: f32 = 1e-6;
    const PI: f32 = std::f32::consts::PI;

    /// Solve depressed cubic: t³ + pt + q = 0
    /// Returns (roots, num_roots)
    fn solve_depressed_cubic_cpu(p: f32, q: f32) -> ([f32; 3], usize) {
        let q_half = q * 0.5;
        let p_third = p / 3.0;
        let d = q_half * q_half + p_third * p_third * p_third;

        if d > EPSILON {
            // One real root
            let sqrt_d = d.sqrt();
            let u = cbrt_cpu(-q_half + sqrt_d);
            let v = cbrt_cpu(-q_half - sqrt_d);
            ([u + v, 0.0, 0.0], 1)
        } else if d < -EPSILON {
            // Three real roots (trigonometric method)
            let r = (-p_third * p_third * p_third).sqrt();
            let phi = (-q_half / r).clamp(-1.0, 1.0).acos();
            let two_sqrt_r = 2.0 * cbrt_cpu(r);

            let t1 = two_sqrt_r * (phi / 3.0).cos();
            let t2 = two_sqrt_r * ((phi + 2.0 * PI) / 3.0).cos();
            let t3 = two_sqrt_r * ((phi + 4.0 * PI) / 3.0).cos();

            ([t1, t2, t3], 3)
        } else {
            // D ≈ 0: repeated roots
            let u = cbrt_cpu(-q_half);
            ([2.0 * u, -u, -u], 3)
        }
    }

    /// Find minimum squared distance from point P to quadratic Bezier curve.
    /// CPU emulation of WGSL min_dist_sq_to_bezier.
    fn min_dist_sq_to_bezier_cpu(p: [f32; 2], p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> f32 {
        // Rewrite Bezier as: B(t) = P₀ + 2tA + t²C
        let a = [p1[0] - p0[0], p1[1] - p0[1]];
        let c = [p2[0] - 2.0 * p1[0] + p0[0], p2[1] - 2.0 * p1[1] + p0[1]];
        let v = [p[0] - p0[0], p[1] - p0[1]];

        // Cubic coefficients
        let a_coef = c[0] * c[0] + c[1] * c[1]; // |C|²
        let b_coef = 3.0 * (a[0] * c[0] + a[1] * c[1]); // 3(A·C)
        let c_coef = 2.0 * (a[0] * a[0] + a[1] * a[1]) - (v[0] * c[0] + v[1] * c[1]); // 2|A|² - V·C
        let d_coef = -(v[0] * a[0] + v[1] * a[1]); // -V·A

        // Handle degenerate case
        if a_coef.abs() < EPSILON {
            if b_coef.abs() < EPSILON {
                // Linear case
                let mut t_candidate = 0.0;
                if c_coef.abs() > EPSILON {
                    t_candidate = (-d_coef / c_coef).clamp(0.0, 1.0);
                }
                let d0 = dist_sq_to_bezier_cpu(p, p0, p1, p2, 0.0);
                let d1 = dist_sq_to_bezier_cpu(p, p0, p1, p2, 1.0);
                let dt = dist_sq_to_bezier_cpu(p, p0, p1, p2, t_candidate);
                return d0.min(d1).min(dt);
            }
            // Quadratic case
            let disc = c_coef * c_coef - 4.0 * b_coef * d_coef;
            let mut min_d = dist_sq_to_bezier_cpu(p, p0, p1, p2, 0.0);
            min_d = min_d.min(dist_sq_to_bezier_cpu(p, p0, p1, p2, 1.0));

            if disc >= 0.0 {
                let sqrt_disc = disc.sqrt();
                let t1 = ((-c_coef + sqrt_disc) / (2.0 * b_coef)).clamp(0.0, 1.0);
                let t2 = ((-c_coef - sqrt_disc) / (2.0 * b_coef)).clamp(0.0, 1.0);
                min_d = min_d.min(dist_sq_to_bezier_cpu(p, p0, p1, p2, t1));
                min_d = min_d.min(dist_sq_to_bezier_cpu(p, p0, p1, p2, t2));
            }
            return min_d;
        }

        // Normalize to monic cubic
        let inv_a = 1.0 / a_coef;
        let p_coef = b_coef * inv_a;
        let q_coef = c_coef * inv_a;
        let r_coef = d_coef * inv_a;

        // Substitute t = u - p/3 to get depressed cubic
        let p_third = p_coef / 3.0;
        let p_depressed = q_coef - p_coef * p_third;
        let q_depressed = r_coef - q_coef * p_third + 2.0 * p_third * p_third * p_third;

        // Solve depressed cubic
        let (roots, num_roots) = solve_depressed_cubic_cpu(p_depressed, q_depressed);

        // Evaluate distances at endpoints and roots
        let mut min_d = dist_sq_to_bezier_cpu(p, p0, p1, p2, 0.0);
        min_d = min_d.min(dist_sq_to_bezier_cpu(p, p0, p1, p2, 1.0));

        let t1 = (roots[0] - p_third).clamp(0.0, 1.0);
        min_d = min_d.min(dist_sq_to_bezier_cpu(p, p0, p1, p2, t1));

        if num_roots >= 2 {
            let t2 = (roots[1] - p_third).clamp(0.0, 1.0);
            min_d = min_d.min(dist_sq_to_bezier_cpu(p, p0, p1, p2, t2));
        }

        if num_roots >= 3 {
            let t3 = (roots[2] - p_third).clamp(0.0, 1.0);
            min_d = min_d.min(dist_sq_to_bezier_cpu(p, p0, p1, p2, t3));
        }

        min_d
    }

    #[test]
    fn test_sdf_stroke_distance_at_control_points() {
        // Test curve: (0,0) -> (50,100) -> (100,0)
        let p0 = [0.0f32, 0.0];
        let p1 = [50.0, 100.0];
        let p2 = [100.0, 0.0];

        // Distance at P0 should be 0
        let dist_at_p0 = min_dist_sq_to_bezier_cpu(p0, p0, p1, p2).sqrt();
        assert!(
            dist_at_p0 < 0.01,
            "Distance at P0 should be ~0, got {}",
            dist_at_p0
        );

        // Distance at P2 should be 0
        let dist_at_p2 = min_dist_sq_to_bezier_cpu(p2, p0, p1, p2).sqrt();
        assert!(
            dist_at_p2 < 0.01,
            "Distance at P2 should be ~0, got {}",
            dist_at_p2
        );
    }

    #[test]
    fn test_sdf_stroke_distance_at_midpoint() {
        // Test curve: (0,0) -> (50,100) -> (100,0)
        let p0 = [0.0f32, 0.0];
        let p1 = [50.0, 100.0];
        let p2 = [100.0, 0.0];

        // Evaluate curve at t=0.5: B(0.5) = 0.25*P0 + 0.5*P1 + 0.25*P2
        let mid = eval_bezier_cpu(p0, p1, p2, 0.5);
        // mid = [0.25*0 + 0.5*50 + 0.25*100, 0.25*0 + 0.5*100 + 0.25*0] = [50, 50]

        assert!(
            (mid[0] - 50.0).abs() < 0.01 && (mid[1] - 50.0).abs() < 0.01,
            "Midpoint should be (50, 50), got ({}, {})",
            mid[0],
            mid[1]
        );

        // Distance from midpoint to curve should be 0
        let dist_at_mid = min_dist_sq_to_bezier_cpu(mid, p0, p1, p2).sqrt();
        assert!(
            dist_at_mid < 0.01,
            "Distance at curve midpoint should be ~0, got {}",
            dist_at_mid
        );
    }

    #[test]
    fn test_sdf_stroke_distance_off_curve() {
        // Test curve: (0,0) -> (50,100) -> (100,0)
        let p0 = [0.0f32, 0.0];
        let p1 = [50.0, 100.0];
        let p2 = [100.0, 0.0];

        // Point far from curve
        let far_point = [200.0f32, 200.0];
        let dist_far = min_dist_sq_to_bezier_cpu(far_point, p0, p1, p2).sqrt();

        // Should be significantly larger than 0
        assert!(
            dist_far > 100.0,
            "Distance from far point should be >100, got {}",
            dist_far
        );
    }

    #[test]
    fn test_sdf_stroke_linear_degenerate() {
        // Degenerate case: P1 is on the line P0-P2 (collinear control points)
        // This makes the curve a straight line
        let p0 = [0.0f32, 0.0];
        let p1 = [50.0, 50.0]; // Midpoint of line
        let p2 = [100.0, 100.0];

        // Point perpendicular to line at (50, 50)
        let test_point = [60.0f32, 40.0]; // 10*sqrt(2)/2 away from line

        let dist = min_dist_sq_to_bezier_cpu(test_point, p0, p1, p2).sqrt();

        // Expected distance: perpendicular distance from (60,40) to line y=x
        // Line: x - y = 0, point (60, 40)
        // Distance = |60 - 40| / sqrt(2) = 20 / sqrt(2) ≈ 14.14
        let expected = 20.0 / (2.0f32).sqrt();
        assert!(
            (dist - expected).abs() < 1.0,
            "Distance to line should be ~{:.2}, got {:.2}",
            expected,
            dist
        );
    }

    #[test]
    fn test_sdf_stroke_symmetric_curve() {
        // Symmetric parabola-like curve
        let p0 = [0.0f32, 0.0];
        let p1 = [50.0, 100.0];
        let p2 = [100.0, 0.0];

        // Points symmetric about x=50 should have equal distances
        let point_left = [25.0f32, 30.0];
        let point_right = [75.0f32, 30.0];

        let dist_left = min_dist_sq_to_bezier_cpu(point_left, p0, p1, p2).sqrt();
        let dist_right = min_dist_sq_to_bezier_cpu(point_right, p0, p1, p2).sqrt();

        assert!(
            (dist_left - dist_right).abs() < 1.0,
            "Symmetric points should have equal distances: left={:.2}, right={:.2}",
            dist_left,
            dist_right
        );
    }

    // =========================================================================
    // Cardano Discriminant Edge Cases (D < 0 and D ≈ 0)
    // =========================================================================

    /// Helper to compute discriminant D for debugging.
    /// D > 0: 1 real root (Cardano cube root)
    /// D < 0: 3 real roots (trigonometric method)
    /// D = 0: boundary (repeated roots)
    fn compute_discriminant_cpu(p: [f32; 2], p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> f32 {
        let a = [p1[0] - p0[0], p1[1] - p0[1]];
        let c = [p2[0] - 2.0 * p1[0] + p0[0], p2[1] - 2.0 * p1[1] + p0[1]];
        let v = [p[0] - p0[0], p[1] - p0[1]];

        let a_coef = c[0] * c[0] + c[1] * c[1];
        if a_coef.abs() < EPSILON { return f32::NAN; }

        let b_coef = 3.0 * (a[0] * c[0] + a[1] * c[1]);
        let c_coef = 2.0 * (a[0] * a[0] + a[1] * a[1]) - (v[0] * c[0] + v[1] * c[1]);
        let d_coef = -(v[0] * a[0] + v[1] * a[1]);

        let inv_a = 1.0 / a_coef;
        let p_coef = b_coef * inv_a;
        let q_coef = c_coef * inv_a;
        let r_coef = d_coef * inv_a;

        let p_third = p_coef / 3.0;
        let p_dep = q_coef - p_coef * p_third;
        let q_dep = r_coef - q_coef * p_third + 2.0 * p_third * p_third * p_third;

        let q_half = q_dep * 0.5;
        let p_third_dep = p_dep / 3.0;
        q_half * q_half + p_third_dep * p_third_dep * p_third_dep
    }

    #[test]
    fn test_sdf_stroke_discriminant_negative_three_roots() {
        // Test D < 0 case: point below the curve where 3 real roots exist
        // This exercises the trigonometric branch of Cardano's formula
        let p0 = [0.0f32, 0.0];
        let p1 = [50.0, 100.0];
        let p2 = [100.0, 0.0];

        // Point (50, -20) is below the baseline, giving D < 0
        let test_point = [50.0f32, -20.0];
        let d = compute_discriminant_cpu(test_point, p0, p1, p2);

        assert!(
            d < -EPSILON,
            "Point (50, -20) should give D < 0 (3 real roots), got D = {:e}",
            d
        );

        // Distance should be computed without NaN (trigonometric branch works)
        let dist = min_dist_sq_to_bezier_cpu(test_point, p0, p1, p2).sqrt();

        assert!(
            !dist.is_nan() && dist.is_finite() && dist > 0.0,
            "Distance with D < 0 should be valid, got {}",
            dist
        );

        // Verify distance is reasonable (closest point at t=0 or t=1)
        // Distance to P0 (0,0) = sqrt(50² + 20²) ≈ 53.9
        // Distance to P2 (100,0) = sqrt(50² + 20²) ≈ 53.9
        let dist_to_p0 = ((50.0f32).powi(2) + (20.0f32).powi(2)).sqrt();
        assert!(
            (dist - dist_to_p0).abs() < 1.0,
            "Distance should be ~{:.2} (to endpoints), got {:.2}",
            dist_to_p0, dist
        );
    }

    #[test]
    fn test_sdf_stroke_discriminant_near_zero_boundary() {
        // Test D ≈ 0 case: boundary between 1 and 3 real roots
        // This is where numerical instability could occur
        let p0 = [0.0f32, 0.0];
        let p1 = [50.0, 100.0];
        let p2 = [100.0, 0.0];

        // Point (50, 25) is exactly on the D = 0 boundary
        let boundary_point = [50.0f32, 25.0];
        let d = compute_discriminant_cpu(boundary_point, p0, p1, p2);

        // D should be very close to 0
        assert!(
            d.abs() < 1e-3,
            "Point (50, 25) should give D ≈ 0, got D = {:e}",
            d
        );

        // Distance should still be computed correctly without NaN
        let dist = min_dist_sq_to_bezier_cpu(boundary_point, p0, p1, p2).sqrt();

        assert!(
            !dist.is_nan() && dist.is_finite(),
            "Distance at D ≈ 0 boundary should not be NaN/Inf, got {}",
            dist
        );

        // The point (50, 25) should have a small but non-zero distance
        // (it's inside the convex hull but not on the curve)
        assert!(
            dist > 0.0 && dist < 50.0,
            "Distance at (50, 25) should be reasonable, got {:.2}",
            dist
        );
    }

    #[test]
    fn test_sdf_stroke_discriminant_slightly_negative() {
        // Test D slightly below 0: edge case that might trigger acos argument issues
        let p0 = [0.0f32, 0.0];
        let p1 = [50.0, 100.0];
        let p2 = [100.0, 0.0];

        // Point (50, 20) gives D ≈ -5.79e-7 (very small negative)
        let test_point = [50.0f32, 20.0];
        let d = compute_discriminant_cpu(test_point, p0, p1, p2);

        // Should be slightly negative
        assert!(
            d < 0.0 && d > -1e-4,
            "Point (50, 20) should give small negative D, got D = {:e}",
            d
        );

        // Distance must be computed without NaN (acos clamp must work)
        let dist = min_dist_sq_to_bezier_cpu(test_point, p0, p1, p2).sqrt();

        assert!(
            !dist.is_nan() && dist.is_finite(),
            "Distance with small negative D should not produce NaN, got {}",
            dist
        );
    }

    #[test]
    fn test_sdf_stroke_acos_argument_clamping() {
        // Verify CPU emulation matches WGSL clamp behavior
        // When D < 0, acos argument is: -q_half / r
        // This can exceed [-1, 1] due to floating-point errors near D = 0
        let p0 = [0.0f32, 0.0];
        let p1 = [50.0, 100.0];
        let p2 = [100.0, 0.0];

        // Test multiple points near D = 0 boundary
        let test_points = [
            [50.0f32, 25.0],   // D = 0 exact
            [50.0, 24.9],      // D slightly positive
            [50.0, 25.1],      // D slightly negative
            [50.0, 20.0],      // D small negative
            [45.0, 5.0],       // D near zero
        ];

        for pt in &test_points {
            let dist = min_dist_sq_to_bezier_cpu(*pt, p0, p1, p2).sqrt();
            assert!(
                !dist.is_nan() && dist.is_finite() && dist >= 0.0,
                "Point ({}, {}) produced invalid distance: {}",
                pt[0], pt[1], dist
            );
        }
    }
}
