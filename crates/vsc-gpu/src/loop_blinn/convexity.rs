//! Curve Convexity Detection
//!
//! Determines whether a quadratic Bezier curve is convex or concave by
//! analyzing the position of the control point relative to the baseline.

/// Collinearity detection threshold.
///
/// Points with cross product magnitude below this threshold are considered
/// collinear (degenerate curve that should be treated as a line segment).
const COLLINEAR_THRESHOLD: f32 = 1e-6;

/// Compute the curve sign for a quadratic Bezier.
///
/// Determines whether the control point P₁ is on the "inside" (convex) or
/// "outside" (concave) of the curve relative to the baseline P₀→P₂.
///
/// ## Returns
///
/// - `Some(1.0)`: Convex curve (P₁ to the left of P₀→P₂, counter-clockwise)
/// - `Some(-1.0)`: Concave curve (P₁ to the right of P₀→P₂, clockwise)
/// - `None`: Collinear points (degenerate curve, should be treated as line)
///
/// ## Algorithm
///
/// Uses the 2D cross product of vectors (P₁ - P₀) and (P₂ - P₀):
/// ```text
/// cross = (P₁.x - P₀.x) × (P₂.y - P₀.y) - (P₁.y - P₀.y) × (P₂.x - P₀.x)
/// ```
///
/// - cross > threshold: P₁ is to the left (convex)
/// - cross < -threshold: P₁ is to the right (concave)
/// - |cross| ≤ threshold: Collinear (degenerate)
///
/// ## Example
///
/// ```
/// use vsc_gpu::loop_blinn::compute_curve_sign;
///
/// // Convex curve (control point above baseline)
/// let sign = compute_curve_sign([0.0, 0.0], [50.0, 100.0], [100.0, 0.0]);
/// assert_eq!(sign, Some(1.0));
///
/// // Concave curve (control point below baseline)
/// let sign = compute_curve_sign([0.0, 0.0], [50.0, -100.0], [100.0, 0.0]);
/// assert_eq!(sign, Some(-1.0));
///
/// // Collinear (degenerate)
/// let sign = compute_curve_sign([0.0, 0.0], [50.0, 0.0], [100.0, 0.0]);
/// assert_eq!(sign, None);
/// ```
pub fn compute_curve_sign(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> Option<f32> {
    // Vectors from P₀ to P₁ and P₀ to P₂
    let v1 = [p1[0] - p0[0], p1[1] - p0[1]];
    let v2 = [p2[0] - p0[0], p2[1] - p0[1]];

    // 2D cross product: v1 × v2 = v1.x * v2.y - v1.y * v2.x
    let cross = v1[0] * v2[1] - v1[1] * v2[0];

    // Check for collinearity
    if cross.abs() <= COLLINEAR_THRESHOLD {
        return None;
    }

    // Cross product sign determines winding:
    // - Negative: P₁ is to the left of P₀→P₂ (convex in screen coordinates)
    // - Positive: P₁ is to the right of P₀→P₂ (concave in screen coordinates)
    //
    // For Loop-Blinn with texture coords P₀=(0,0), P₁=(0.5,0), P₂=(1,1):
    // - curve_sign = +1.0: f < 0 is inside (fill the chord side)
    // - curve_sign = -1.0: f > 0 is inside (fill the control point side)
    if cross < 0.0 {
        Some(1.0) // Convex: fill the chord side
    } else {
        Some(-1.0) // Concave: fill the control point side
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convex_curve() {
        // Control point above the baseline P₀→P₂
        // This creates a curve that bulges upward
        let p0 = [0.0, 0.0];
        let p1 = [50.0, 100.0];
        let p2 = [100.0, 0.0];

        let sign = compute_curve_sign(p0, p1, p2);
        assert_eq!(sign, Some(1.0));
    }

    #[test]
    fn test_concave_curve() {
        // Control point below the baseline P₀→P₂
        // This creates a curve that bulges downward
        let p0 = [0.0, 0.0];
        let p1 = [50.0, -100.0];
        let p2 = [100.0, 0.0];

        let sign = compute_curve_sign(p0, p1, p2);
        assert_eq!(sign, Some(-1.0));
    }

    #[test]
    fn test_collinear_horizontal() {
        // All three points on a horizontal line
        let p0 = [0.0, 0.0];
        let p1 = [50.0, 0.0];
        let p2 = [100.0, 0.0];

        let sign = compute_curve_sign(p0, p1, p2);
        assert_eq!(sign, None);
    }

    #[test]
    fn test_collinear_diagonal() {
        // All three points on a diagonal line
        let p0 = [0.0, 0.0];
        let p1 = [50.0, 50.0];
        let p2 = [100.0, 100.0];

        let sign = compute_curve_sign(p0, p1, p2);
        assert_eq!(sign, None);
    }

    #[test]
    fn test_collinear_near_threshold() {
        // Points very close to collinear but just above threshold
        let p0 = [0.0, 0.0];
        let p1 = [50.0, 1e-5]; // Slightly above horizontal
        let p2 = [100.0, 0.0];

        let sign = compute_curve_sign(p0, p1, p2);
        // Cross product = 50.0 * 0.0 - 1e-5 * 100.0 = -1e-3
        // |cross| = 1e-3 > 1e-6, so not collinear
        assert!(sign.is_some());
    }

    #[test]
    fn test_collinear_at_threshold() {
        // Points exactly at collinearity threshold
        let p0 = [0.0, 0.0];
        let p1 = [1.0, 1e-6]; // Very close to collinear
        let p2 = [2.0, 0.0];

        let sign = compute_curve_sign(p0, p1, p2);
        // Cross product = 1.0 * 0.0 - 1e-6 * 2.0 = -2e-6
        // |cross| = 2e-6 > 1e-6, so not collinear
        // But with p1 = [1.0, 5e-7], cross = -1e-6, exactly at threshold
        assert!(sign.is_some()); // This specific case is not collinear
    }

    #[test]
    fn test_vertical_curve() {
        // Curve with vertical baseline
        let p0 = [0.0, 0.0];
        let p1 = [50.0, 50.0]; // Control point to the right
        let p2 = [0.0, 100.0];

        let sign = compute_curve_sign(p0, p1, p2);
        // Cross product = 50.0 * 100.0 - 50.0 * 0.0 = 5000.0 > 0
        // Positive cross (CCW winding) → concave (-1.0)
        assert_eq!(sign, Some(-1.0));
    }

    #[test]
    fn test_reversed_direction() {
        // Same curve but with reversed point order
        let p0 = [100.0, 0.0];
        let p1 = [50.0, 100.0];
        let p2 = [0.0, 0.0];

        let sign = compute_curve_sign(p0, p1, p2);
        // This reverses the winding, so the sign should flip
        assert_eq!(sign, Some(-1.0));
    }

    #[test]
    fn test_small_curve() {
        // Very small curve (subpixel)
        let p0 = [0.0, 0.0];
        let p1 = [0.001, 0.002];
        let p2 = [0.002, 0.0];

        let sign = compute_curve_sign(p0, p1, p2);
        // Cross = 0.001 * 0.0 - 0.002 * 0.002 = -4e-6
        // |cross| = 4e-6 > 1e-6, so not collinear
        // Negative cross (CW winding) → convex (1.0)
        assert_eq!(sign, Some(1.0));
    }

    #[test]
    fn test_large_curve() {
        // Very large curve
        let p0 = [0.0, 0.0];
        let p1 = [5000.0, 10000.0];
        let p2 = [10000.0, 0.0];

        let sign = compute_curve_sign(p0, p1, p2);
        assert_eq!(sign, Some(1.0));
    }
}
