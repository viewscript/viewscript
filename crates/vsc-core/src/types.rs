//! Core type definitions for ViewScript P-dimension space.
//!
//! These types correspond to the LEAN 4 axiomatization in rfc/lean/ViewScriptRFC/PDimension.lean
//!
//! ## Float Decontamination (Architect Directive)
//!
//! P-dimension is a strict rational number space. Native floating-point operations
//! (f32, f64) are FORBIDDEN in constraint evaluation to preserve:
//!
//! 1. **Decidability**: LEAN 4 `Decidable` proofs require exact arithmetic
//! 2. **Determinism**: IEEE 754 rounding modes vary across platforms
//! 3. **Closure**: Rational operations are closed (unlike sin, sqrt, etc.)
//!
//! The only place f64 is permitted is at the RASTERIZATION BOUNDARY, where
//! rational coordinates are projected to device pixels.

use num_bigint::BigInt;
use num_rational::Ratio;
use num_traits::{Signed, ToPrimitive, Zero};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

// =============================================================================
// Rasterization Boundary Constants
// =============================================================================

/// Maximum safe integer for IEEE 754 double precision mantissa.
///
/// Values exceeding this threshold (2^53) cannot be represented exactly
/// in f64 and will incur precision loss during rasterization.
///
/// ## Phase 1 Remediation (Architect Directive)
///
/// Uses BigInt directly to avoid i128 intermediate casts, which would
/// fail silently for values exceeding 3.4×10^38.
fn f64_mantissa_safe_limit() -> BigInt {
    BigInt::from(9_007_199_254_740_992i64) // 2^53
}

// =============================================================================
// Exact Rational Type (P-Dimension Core)
// =============================================================================

/// Exact rational number for P-dimension arithmetic.
///
/// Uses arbitrary-precision integers to avoid overflow and maintain exactness.
/// All P-dimension coordinates, constraints, and computations MUST use this type.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Rational(pub Ratio<BigInt>);

impl Rational {
    /// Create a new rational from numerator and denominator.
    pub fn new(numerator: i64, denominator: i64) -> Self {
        Self(Ratio::new(
            BigInt::from(numerator),
            BigInt::from(denominator),
        ))
    }

    /// Create a rational from an integer.
    pub fn from_int(n: i64) -> Self {
        Self(Ratio::from_integer(BigInt::from(n)))
    }

    /// Zero.
    pub fn zero() -> Self {
        Self(Ratio::zero())
    }

    /// One.
    pub fn one() -> Self {
        Self::from_int(1)
    }

    /// Convert to f64 for RASTERIZATION ONLY.
    ///
    /// WARNING: This is the ONLY place where f64 conversion is permitted.
    /// Use ONLY at the final rasterization step when projecting to device pixels.
    ///
    /// ## Phase 1 Remediation: Safe Conversion with Overflow Detection
    ///
    /// This method uses `num_traits::ToPrimitive` for direct conversion,
    /// avoiding the inefficient and non-deterministic `to_string().parse()` path.
    ///
    /// If the numerator or denominator exceeds IEEE 754's safe integer limit (2^53),
    /// a precision warning is emitted to stderr (non-destructive monitoring).
    #[inline]
    pub fn to_f64_for_rasterization(&self) -> f64 {
        let numer = self.0.numer();
        let denom = self.0.denom();

        // Phase 1 Remediation: Overflow detection for 2^53 boundary
        // Direct BigInt comparison (no i128 intermediate cast)
        let limit = f64_mantissa_safe_limit();
        let n_abs = numer.abs();
        let d_abs = denom.abs();

        if n_abs > limit || d_abs > limit {
            // Non-destructive warning: do not panic, but log for telemetry
            eprintln!(
                "[vsc-core::PrecisionWarning::MantissaOverflow] \
                 Rational {}/{} exceeds IEEE 754 safe limit (2^53). \
                 Rasterization may incur precision loss.",
                numer, denom
            );
        }

        // Direct conversion using ToPrimitive (efficient, no string parsing)
        let n_f64 = numer.to_f64().unwrap_or_else(|| {
            // Fallback for extreme values: use sign * infinity
            if numer.is_negative() {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        });
        let d_f64 = denom.to_f64().unwrap_or(1.0);

        n_f64 / d_f64
    }

    /// Absolute value.
    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }
}

impl fmt::Debug for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.0.numer(), self.0.denom())
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // For JSON serialization, output the fraction representation
        write!(f, "{}/{}", self.0.numer(), self.0.denom())
    }
}

impl std::ops::Add for Rational {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Rational {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::Mul for Rational {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self(self.0 * rhs.0)
    }
}

impl std::ops::Div for Rational {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        Self(self.0 / rhs.0)
    }
}

impl std::ops::Neg for Rational {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl Rational {
    /// Check if this rational is zero.
    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.0.cmp(&other.0))
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

// Serialization: Use string representation to preserve exactness
impl Serialize for Rational {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = format!("{}/{}", self.0.numer(), self.0.denom());
        serializer.serialize_str(&s)
    }
}

impl<'de> Deserialize<'de> for Rational {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let parts: Vec<&str> = s.split('/').collect();
        match parts.len() {
            1 => {
                // Integer format: "50" → 50/1
                let numer: BigInt = parts[0].parse().map_err(serde::de::Error::custom)?;
                Ok(Self(Ratio::new(numer, BigInt::from(1))))
            }
            2 => {
                // Fraction format: "50/1" → 50/1
                let numer: BigInt = parts[0].parse().map_err(serde::de::Error::custom)?;
                let denom: BigInt = parts[1].parse().map_err(serde::de::Error::custom)?;
                Ok(Self(Ratio::new(numer, denom)))
            }
            _ => Err(serde::de::Error::custom(
                "Expected integer (\"50\") or fraction (\"50/1\") format",
            )),
        }
    }
}

impl JsonSchema for Rational {
    fn schema_name() -> String {
        "Rational".to_string()
    }

    fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::InstanceType::String.into()),
            format: Some("rational".to_string()),
            metadata: Some(Box::new(schemars::schema::Metadata {
                description: Some(
                    "Exact rational number in 'numerator/denominator' format (e.g. '3/4', '100/1')"
                        .to_string(),
                ),
                examples: vec![
                    serde_json::Value::String("3/4".to_string()),
                    serde_json::Value::String("100/1".to_string()),
                    serde_json::Value::String("-1/2".to_string()),
                ],
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

// =============================================================================
// Global Epsilon (Rational)
// =============================================================================

/// Global epsilon for rational tolerance.
///
/// RFC 2119: MUST be invariant across all component boundaries.
/// Represented as the rational 1/10^10.
pub fn epsilon() -> Rational {
    Rational::new(1, 10_000_000_000)
}

// =============================================================================
// IEEE 754 → Rational Exact Conversion (Phase E)
// =============================================================================

/// Convert an IEEE 754 f32 to an exact Rational representation.
///
/// ## Phase E: Font Outline Integration
///
/// Font files (TrueType and CFF) store glyph coordinates that `ttf-parser`
/// exposes as `f32`. This function recovers the exact rational representation
/// without any precision loss.
///
/// ## IEEE 754 f32 Layout
///
/// ```text
/// [S|EEEEEEEE|MMMMMMMMMMMMMMMMMMMMMMM]
///  1    8              23 bits
/// ```
///
/// - S: Sign bit (0 = positive, 1 = negative)
/// - E: Biased exponent (bias = 127)
/// - M: Mantissa (fractional part, implicit leading 1 for normalized)
///
/// ## Conversion Formula
///
/// For normalized numbers (E ∈ [1, 254]):
/// ```text
/// value = (-1)^S × (1 + M/2^23) × 2^(E-127)
///       = (-1)^S × (2^23 + M) / 2^(150-E)   when E < 150
///       = (-1)^S × (2^23 + M) × 2^(E-150)   when E >= 150
/// ```
///
/// For denormalized numbers (E = 0):
/// ```text
/// value = (-1)^S × (M/2^23) × 2^(-126)
///       = (-1)^S × M / 2^149
/// ```
///
/// ## Exactness Guarantee
///
/// Every finite f32 is exactly representable as m × 2^e where m and e are
/// integers. Since Rational uses arbitrary-precision BigInt, the conversion
/// is lossless for all finite f32 values.
///
/// ## Special Cases
///
/// - `0.0` and `-0.0` → `Rational(0/1)`
/// - `f32::INFINITY` → panics (not representable)
/// - `f32::NEG_INFINITY` → panics (not representable)
/// - `f32::NAN` → panics (not representable)
///
/// ## Examples
///
/// ```
/// use vsc_core::{Rational, f32_to_rational_exact};
///
/// // Integer (TrueType typical)
/// assert_eq!(f32_to_rational_exact(100.0), Rational::new(100, 1));
///
/// // Power-of-2 fraction (CFF typical)
/// assert_eq!(f32_to_rational_exact(0.75), Rational::new(3, 4));
///
/// // Non-trivial fraction
/// assert_eq!(f32_to_rational_exact(33.5), Rational::new(67, 2));
/// ```
pub fn f32_to_rational_exact(v: f32) -> Rational {
    // Handle special cases
    if v.is_nan() {
        panic!("f32_to_rational_exact: NaN is not representable as Rational");
    }
    if v.is_infinite() {
        panic!("f32_to_rational_exact: Infinity is not representable as Rational");
    }

    // Zero (both +0.0 and -0.0)
    if v == 0.0 {
        return Rational::zero();
    }

    // Extract IEEE 754 components from bit representation
    let bits = v.to_bits();
    let sign = (bits >> 31) & 1;
    let exponent_biased = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x7F_FFFF; // 23 bits

    // Build the exact rational
    let (numerator, denominator) = if exponent_biased == 0 {
        // Denormalized number: value = M × 2^(-149)
        // No implicit leading 1
        let m = BigInt::from(mantissa);
        let d = BigInt::from(1u64) << 149; // 2^149
        (m, d)
    } else {
        // Normalized number: value = (2^23 + M) × 2^(E-150)
        let significand = BigInt::from((1u64 << 23) + u64::from(mantissa)); // 2^23 + M
        let exp = exponent_biased - 150; // E - 127 - 23

        if exp >= 0 {
            // Positive exponent: multiply numerator by 2^exp
            let n = significand << (exp as usize);
            (n, BigInt::from(1))
        } else {
            // Negative exponent: denominator is 2^(-exp)
            let d = BigInt::from(1u64) << ((-exp) as usize);
            (significand, d)
        }
    };

    // Apply sign
    let final_numerator = if sign == 1 { -numerator } else { numerator };

    Rational(Ratio::new(final_numerator, denominator))
}

/// Convert f64 to exact Rational representation.
///
/// This function extracts the exact IEEE 754 representation of an f64
/// and converts it to a Rational without any rounding or approximation.
///
/// ## Panics
///
/// - If the input is NaN
/// - If the input is infinite
///
/// ## Example
///
/// ```
/// use vsc_core::types::{Rational, f64_to_rational};
///
/// // Integer values are exact
/// assert_eq!(f64_to_rational(100.0), Rational::new(100, 1));
///
/// // Power-of-2 fractions are exact
/// assert_eq!(f64_to_rational(0.75), Rational::new(3, 4));
/// assert_eq!(f64_to_rational(1.5), Rational::new(3, 2));
/// ```
pub fn f64_to_rational(v: f64) -> Rational {
    // Handle special cases
    if v.is_nan() {
        panic!("f64_to_rational: NaN is not representable as Rational");
    }
    if v.is_infinite() {
        panic!("f64_to_rational: Infinity is not representable as Rational");
    }

    // Zero (both +0.0 and -0.0)
    if v == 0.0 {
        return Rational::zero();
    }

    // Extract IEEE 754 components from bit representation
    let bits = v.to_bits();
    let sign = (bits >> 63) & 1;
    let exponent_biased = ((bits >> 52) & 0x7FF) as i32;
    let mantissa = bits & 0xF_FFFF_FFFF_FFFF; // 52 bits

    // Build the exact rational
    let (numerator, denominator) = if exponent_biased == 0 {
        // Denormalized number: value = M × 2^(-1074)
        // No implicit leading 1
        let m = BigInt::from(mantissa);
        let d = BigInt::from(1u64) << 1074; // 2^1074 (will be reduced)
        (m, d)
    } else {
        // Normalized number: value = (2^52 + M) × 2^(E-1075)
        let significand = BigInt::from((1u64 << 52) + mantissa); // 2^52 + M
        let exp = exponent_biased - 1075; // E - 1023 - 52

        if exp >= 0 {
            // Positive exponent: multiply numerator by 2^exp
            let n = significand << (exp as usize);
            (n, BigInt::from(1))
        } else {
            // Negative exponent: denominator is 2^(-exp)
            let d = BigInt::from(1u64) << ((-exp) as usize);
            (significand, d)
        }
    };

    // Apply sign
    let final_numerator = if sign == 1 { -numerator } else { numerator };

    Rational(Ratio::new(final_numerator, denominator))
}

/// Check if two rationals are ε-equivalent.
pub fn epsilon_eq(a: &Rational, b: &Rational) -> bool {
    (a.clone() - b.clone()).abs() < epsilon()
}

// =============================================================================
// Legacy f64 constant (deprecated)
// =============================================================================

/// DEPRECATED: Use `epsilon()` function instead.
/// Kept only for backward compatibility with rasterization layer.
#[deprecated(note = "Use epsilon() for P-dimension; f64 only at rasterization boundary")]
pub const EPSILON_F64: f64 = 1e-10;

/// Unique identifier for all P-dimension entities (points, curves, surfaces, constraints).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct EntityId(pub u64);

// =============================================================================
// Post-Solve Condition Types (FFI Trigger Support)
// =============================================================================

/// Unique identifier for post-solve conditions.
pub type ConditionId = u64;

/// A condition evaluated after constraint solving, using resolved coordinates.
///
/// Post-solve conditions enable declarative FFI triggers:
/// - Registered before solving (condition definition)
/// - Evaluated after solving (using resolved EntityId coordinates)
/// - Rising-edge detection determines which conditions fire FFI calls
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PostSolveCondition {
    pub id: ConditionId,
    pub kind: ConditionKind,
}

/// The kind of condition to evaluate post-solve.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConditionKind {
    /// True when the axis-aligned bounding boxes of two entities overlap.
    BoundsOverlap {
        entity_a: EntityId,
        entity_b: EntityId,
    },

    /// True when two entity properties have equal values (exact rational equality).
    PropertiesEqual {
        entity_a: EntityId,
        component_a: VectorComponent,
        entity_b: EntityId,
        component_b: VectorComponent,
    },

    /// True when entity_a.component_a < entity_b.component_b.
    ///
    /// For GreaterThan, swap the arguments.
    PropertyLessThan {
        entity_a: EntityId,
        component_a: VectorComponent,
        entity_b: EntityId,
        component_b: VectorComponent,
    },

    /// True when a property crosses a threshold in the specified direction.
    ///
    /// Combined with rising-edge detection:
    /// - `Rising`: triggers when value goes from <= threshold to > threshold
    /// - `Falling`: triggers when value goes from >= threshold to < threshold
    ThresholdCrossing {
        entity: EntityId,
        component: VectorComponent,
        threshold: Rational,
        direction: CrossingDirection,
    },
}

/// Direction of threshold crossing for ThresholdCrossing conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CrossingDirection {
    /// Condition is true when value > threshold (crossed from below)
    Rising,
    /// Condition is true when value < threshold (crossed from above)
    Falling,
}

/// A vector in P-dimension space: X, Y, Z spatial + T temporal.
///
/// All components are exact rationals (no floating-point contamination).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PVector {
    pub x: Rational,
    pub y: Rational,
    pub z: Rational,
    pub t: Rational,
}

impl PVector {
    /// Create a zero vector.
    pub fn zero() -> Self {
        Self {
            x: Rational::zero(),
            y: Rational::zero(),
            z: Rational::zero(),
            t: Rational::zero(),
        }
    }

    /// Check if two vectors are ε-equivalent using exact rational arithmetic.
    pub fn epsilon_eq(&self, other: &PVector) -> bool {
        epsilon_eq(&self.x, &other.x)
            && epsilon_eq(&self.y, &other.y)
            && epsilon_eq(&self.z, &other.z)
            && epsilon_eq(&self.t, &other.t)
    }

    /// Convert to f64 tuple for RASTERIZATION ONLY.
    pub fn to_f64_for_rasterization(&self) -> (f64, f64, f64, f64) {
        (
            self.x.to_f64_for_rasterization(),
            self.y.to_f64_for_rasterization(),
            self.z.to_f64_for_rasterization(),
            self.t.to_f64_for_rasterization(),
        )
    }
}

/// Which component of an entity a constraint references.
///
/// For spatial entities (ControlPoint, Rect), use X/Y/Z/T.
/// For scalar entities (Radius, Angle), use Value.
/// For ColorStop entities, use R/G/B/A/Position (Phase 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum VectorComponent {
    X,
    Y,
    Z,
    T,
    /// Scalar value component for entities like Radius, Angle.
    /// Enables constraints such as: R1.value = R2.value + offset
    Value,
    /// Red channel [0, 255] for ColorStop (Phase 17).
    R,
    /// Green channel [0, 255] for ColorStop (Phase 17).
    G,
    /// Blue channel [0, 255] for ColorStop (Phase 17).
    B,
    /// Alpha channel [0, 1] for ColorStop (Phase 17).
    #[serde(rename = "alpha")]
    Alpha,
    /// Position [0, 1] along gradient axis for ColorStop (Phase 17).
    Position,
}

/// Binary relation types for constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A term in a constraint expression.
///
/// All numeric values are exact rationals (no floating-point contamination).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConstraintTerm {
    /// A constant rational value.
    Const { value: Rational },
    /// A reference to another entity's component.
    Ref {
        entity_id: EntityId,
        component: VectorComponent,
    },
    /// Linear combination: coefficient * ref + offset
    /// Result = coefficient * reference_value + offset
    Linear {
        coefficient: Rational,
        entity_id: EntityId,
        component: VectorComponent,
        offset: Rational,
    },
}

// =============================================================================
// Phase 11: Constraint Priority (Soft/Hard) for Hierarchical Shadowing
// =============================================================================

/// Priority level for constraints, enabling hierarchical shadowing.
///
/// ## Architectural Decision (Phase 11)
///
/// When a component is imported and a parent scope adds conflicting constraints,
/// the constraint resolution follows this priority order:
///
/// 1. **Hard**: Structural constraints that cannot be shadowed. Violation = error.
///    Example: Topological relationships, entity existence.
///
/// 2. **Soft**: Default values that can be overridden by parent scopes.
///    Example: Default corner radius, padding, colors.
///
/// ## Shadowing Semantics
///
/// When FM elimination detects infeasibility:
/// 1. Identify all constraints involved in the contradiction
/// 2. If any `Soft` constraints exist, temporarily remove the lowest-priority one
/// 3. Re-run elimination; repeat if still infeasible
/// 4. If only `Hard` constraints remain, report error
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintPriority {
    /// Hard constraint: cannot be shadowed. Violation is an error.
    #[default]
    Hard,
    /// Soft constraint: can be shadowed by parent scope.
    /// Lower number = lower priority (more easily shadowed).
    Soft,
}

/// A single constraint in the P-dimension space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Constraint {
    /// Unique identifier for this constraint.
    pub id: u64,
    /// The entity whose component is being constrained.
    pub target: EntityId,
    /// Which component of the target entity.
    pub component: VectorComponent,
    /// The relation type (=, <, ≤, >, ≥).
    pub relation: RelationType,
    /// The term to compare against.
    pub term: ConstraintTerm,
    /// Priority level for shadowing resolution (Phase 11).
    /// Default: Hard (backward compatible).
    #[serde(default)]
    pub priority: ConstraintPriority,
    /// Source scope for debugging (e.g., "RoundedRect::inst_42").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_scope: Option<String>,
}

// =============================================================================
// Phase 6: Vector Curve and Control Point Integration
// =============================================================================

/// The role of a control point within a curve segment.
///
/// Distinguishes between anchor points (on-curve) and handle points (off-curve).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControlPointRole {
    /// An anchor point that lies on the curve.
    Anchor,
    /// A handle (off-curve) control point for Bezier curves.
    Handle,
}

/// A control point entity in P-dimension space.
///
/// Control points are first-class entities with their own EntityId, enabling
/// standard linear constraints (FM-eliminable) to be applied to curve geometry.
///
/// ## Architectural Decision
/// By treating control points as independent P-vectors, we can express
/// constraints like "handle H1 is 50px to the right of anchor A1" using
/// the existing constraint solver without extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ControlPoint {
    /// Unique identifier for this control point.
    pub id: EntityId,
    /// The position in P-dimension space (XYZT).
    pub position: PVector,
    /// Role: anchor (on-curve) or handle (off-curve).
    pub role: ControlPointRole,
    /// Optional: the path this control point belongs to (for validation).
    pub parent_path: Option<EntityId>,
}

/// A segment of a path, referencing control points by EntityId.
///
/// ## Phase G Design: Explicit From/To References
///
/// Each segment explicitly references all control points involved, including
/// the start point (`from`). This enables:
///
/// 1. **Self-contained segments**: No need to track "current point" state
/// 2. **Direct solver access**: `solver.get_resolved(entity_id)` → Rational coordinates
/// 3. **Trivial PathCommand conversion**: Each segment maps directly to a PathCommand
///
/// ## Constraint Integration
///
/// G1 continuity constraints (`add_g1_continuity()`) reference the same handle
/// EntityIds used here, ensuring type-level correspondence between constraints
/// and path geometry.
/// Note: PartialEq only due to f64 rotation in Arc variant (f64 doesn't impl Eq).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PathSegment {
    /// Draw a line between two anchor points.
    Line {
        /// Start anchor point.
        from: EntityId,
        /// End anchor point.
        to: EntityId,
    },
    /// Draw a quadratic Bezier curve.
    Quad {
        /// Start anchor point.
        from: EntityId,
        /// Control handle (off-curve).
        handle: EntityId,
        /// End anchor point.
        to: EntityId,
    },
    /// Draw a cubic Bezier curve.
    Cubic {
        /// Start anchor point.
        from: EntityId,
        /// First control handle (off-curve).
        handle1: EntityId,
        /// Second control handle (off-curve).
        handle2: EntityId,
        /// End anchor point.
        to: EntityId,
    },
    /// Draw an elliptical arc.
    ///
    /// Note: Arc parameters (radii, rotation, flags) are intrinsic to the arc
    /// definition and not constrainable as entities.
    Arc {
        /// Start anchor point.
        from: EntityId,
        /// End anchor point.
        to: EntityId,
        /// X-axis radius (rational).
        rx: Rational,
        /// Y-axis radius (rational).
        ry: Rational,
        /// X-axis rotation in degrees (f64 at rasterization boundary).
        rotation: f64,
        /// Large arc flag (SVG arc semantics).
        large_arc: bool,
        /// Sweep direction flag (SVG arc semantics).
        sweep: bool,
    },
}

/// Fill rule for closed paths (SVG semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FillRule {
    /// Non-zero winding rule.
    NonZero,
    /// Even-odd (parity) rule.
    EvenOdd,
}

impl Default for FillRule {
    fn default() -> Self {
        FillRule::NonZero
    }
}

// =============================================================================
// Phase G: Style Types (Moved from vsc-gpu for Core Layer Access)
// =============================================================================

/// Line cap style for strokes (SVG/CSS standard).
///
/// Defines how the endpoints of open paths are rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LineCap {
    /// Flat edge at endpoint (default).
    #[default]
    Butt,
    /// Semicircle at endpoint.
    Round,
    /// Square projection beyond endpoint.
    Square,
}

/// Line join style for strokes (SVG/CSS standard).
///
/// Defines how corners are rendered when two path segments meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LineJoin {
    /// Sharp corner (default).
    #[default]
    Miter,
    /// Rounded corner.
    Round,
    /// Beveled corner.
    Bevel,
}

/// Fill specification for path entities.
///
/// ## Phase G: Core Layer Style Storage
///
/// This type is stored in VsBuildInfo and converted to vsc-gpu's FillStyle
/// at render time. Gradient fills reference gradient entities by ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FillSpec {
    /// Solid color fill.
    ///
    /// Color is a CSS color string (e.g., "#ff0000", "rgb(255, 0, 0)").
    Solid { color: String },
    /// Gradient fill (references a gradient entity).
    ///
    /// The gradient_id must reference a LinearGradient, RadialGradient,
    /// or ConicGradient entity.
    Gradient { gradient_id: EntityId },
    /// External texture fill (references a Q-dimension texture handle).
    ///
    /// The texture is managed by the host (OffscreenCanvas, video, native texture).
    /// Target-specific binding occurs at render time.
    ExternalTexture {
        /// Name of the Q-variable holding the TextureHandle.
        /// e.g., "resource.texture.video0"
        handle_name: String,
        /// Optional UV transformation for texture mapping.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uv_transform: Option<UvTransform>,
    },
}

/// UV transformation for texture mapping.
///
/// Defines how texture coordinates are mapped to the shape's bounding box.
/// Default is identity (texture fills entire bounds, no rotation).
///
/// ## JSON Format
///
/// ```json
/// {
///   "scale_u": { "numer": 1, "denom": 1 },
///   "scale_v": { "numer": 1, "denom": 1 },
///   "offset_u": { "numer": 0, "denom": 1 },
///   "offset_v": { "numer": 0, "denom": 1 }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UvTransform {
    /// Scale factor for U (horizontal) axis.
    pub scale_u: Rational,
    /// Scale factor for V (vertical) axis.
    pub scale_v: Rational,
    /// Offset for U axis (0-1 range).
    pub offset_u: Rational,
    /// Offset for V axis (0-1 range).
    pub offset_v: Rational,
}

impl Default for UvTransform {
    fn default() -> Self {
        Self {
            scale_u: Rational::from_int(1),
            scale_v: Rational::from_int(1),
            offset_u: Rational::zero(),
            offset_v: Rational::zero(),
        }
    }
}

/// Stroke specification for path entities.
///
/// ## Phase G: Core Layer Style Storage
///
/// All stroke parameters are exact rationals until rasterization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StrokeSpec {
    /// CSS color string.
    pub color: String,
    /// Stroke width (exact rational).
    pub width: Rational,
    /// Line cap style.
    #[serde(default)]
    pub line_cap: LineCap,
    /// Line join style.
    #[serde(default)]
    pub line_join: LineJoin,
    /// Optional dash pattern (array of dash/gap lengths).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash_array: Option<Vec<Rational>>,
}

impl StrokeSpec {
    /// Create a new solid stroke with default cap/join.
    pub fn new(color: impl Into<String>, width: Rational) -> Self {
        Self {
            color: color.into(),
            width,
            line_cap: LineCap::default(),
            line_join: LineJoin::default(),
            dash_array: None,
        }
    }

    /// Set line cap style (builder pattern).
    pub fn with_line_cap(mut self, cap: LineCap) -> Self {
        self.line_cap = cap;
        self
    }

    /// Set line join style (builder pattern).
    pub fn with_line_join(mut self, join: LineJoin) -> Self {
        self.line_join = join;
        self
    }

    /// Set dash pattern (builder pattern).
    pub fn with_dash(mut self, pattern: Vec<Rational>) -> Self {
        self.dash_array = Some(pattern);
        self
    }
}

/// A path entity entry combining geometry and style.
///
/// ## Phase G: Complete Entity Definition
///
/// This structure pairs path geometry (segments) with visual style (fill, stroke).
/// It is stored in VsBuildInfo and used to construct CanvasNode at render time.
///
/// ## Segment Connectivity
///
/// Segments are ordered. Adjacent segments share endpoints:
/// - `segments[0].to == segments[1].from` (for continuous paths)
/// - Disjoint segments create separate subpaths
///
/// ## CanvasNode Construction
///
/// ```text
/// PathEntityEntry + SolverOutput
///         │
///         ▼
///   resolve_path_commands()
///         │
///         ▼
///   Vec<PathCommand> + FillSpec + StrokeSpec
///         │
///         ▼
///   CanvasPathNode
/// ```
/// Note: PartialEq only due to PathSegment containing f64 (Arc.rotation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PathEntityEntry {
    /// Unique entity ID.
    pub id: EntityId,
    /// Ordered path segments.
    pub segments: Vec<PathSegment>,
    /// Whether the path is closed (line back to start).
    pub closed: bool,
    /// Fill rule for closed paths.
    #[serde(default)]
    pub fill_rule: FillRule,
    /// Fill style (solid or gradient reference).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<FillSpec>,
    /// Stroke style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<StrokeSpec>,
}

impl PathEntityEntry {
    /// Create a new path entity with no fill or stroke.
    pub fn new(id: EntityId, segments: Vec<PathSegment>, closed: bool) -> Self {
        Self {
            id,
            segments,
            closed,
            fill_rule: FillRule::default(),
            fill: None,
            stroke: None,
        }
    }

    /// Set fill (builder pattern).
    pub fn with_fill(mut self, fill: FillSpec) -> Self {
        self.fill = Some(fill);
        self
    }

    /// Set stroke (builder pattern).
    pub fn with_stroke(mut self, stroke: StrokeSpec) -> Self {
        self.stroke = Some(stroke);
        self
    }

    /// Set fill rule (builder pattern).
    pub fn with_fill_rule(mut self, rule: FillRule) -> Self {
        self.fill_rule = rule;
        self
    }

    /// Collect all control point EntityIds referenced by this path entry.
    pub fn referenced_control_points(&self) -> Vec<EntityId> {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        let mut points = Vec::new();

        for segment in &self.segments {
            let ids: Vec<EntityId> = match segment {
                PathSegment::Line { from, to } => vec![*from, *to],
                PathSegment::Quad { from, handle, to } => vec![*from, *handle, *to],
                PathSegment::Cubic {
                    from,
                    handle1,
                    handle2,
                    to,
                } => {
                    vec![*from, *handle1, *handle2, *to]
                }
                PathSegment::Arc { from, to, .. } => vec![*from, *to],
            };

            for id in ids {
                if seen.insert(id) {
                    points.push(id);
                }
            }
        }
        points
    }
}

// =============================================================================
// Phase G: Path Command Output (Rasterization-Ready)
// =============================================================================

/// A path command with resolved coordinates, ready for rasterization.
///
/// This is the output format from `resolve_path_commands()`. Unlike `PathSegment`
/// which references EntityIds, `PathCommand` contains actual Rational coordinates.
///
/// ## Serde Format
///
/// Matches `vsc-gpu::PathCommand` for JSON interoperability:
/// - `MoveTo` → `{"type": "M", "x": ..., "y": ...}`
/// - `LineTo` → `{"type": "L", "x": ..., "y": ...}`
/// - etc.
/// Note: PartialEq only due to f64 rotation in ArcTo variant (f64 doesn't impl Eq).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PathCommand {
    /// Move pen to (x, y) without drawing.
    #[serde(rename = "M")]
    MoveTo { x: Rational, y: Rational },

    /// Draw line from current position to (x, y).
    #[serde(rename = "L")]
    LineTo { x: Rational, y: Rational },

    /// Draw quadratic Bezier: control point (x1, y1), end point (x, y).
    #[serde(rename = "Q")]
    QuadTo {
        x1: Rational,
        y1: Rational,
        x: Rational,
        y: Rational,
    },

    /// Draw cubic Bezier: control points (x1, y1), (x2, y2), end (x, y).
    #[serde(rename = "C")]
    CubicTo {
        x1: Rational,
        y1: Rational,
        x2: Rational,
        y2: Rational,
        x: Rational,
        y: Rational,
    },

    /// Draw elliptical arc (SVG arc semantics).
    #[serde(rename = "A")]
    ArcTo {
        /// X-axis radius.
        rx: Rational,
        /// Y-axis radius.
        ry: Rational,
        /// X-axis rotation in degrees.
        rotation: f64,
        /// Large arc flag.
        large_arc: bool,
        /// Sweep direction flag.
        sweep: bool,
        /// End point X.
        x: Rational,
        /// End point Y.
        y: Rational,
    },

    /// Close current subpath (line back to last MoveTo).
    #[serde(rename = "Z")]
    Close,
}

// =============================================================================
// Phase G-0c: PathSegment → PathCommand Resolution
// =============================================================================

/// Error type for path command resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathResolveError {
    /// A control point was not found in the solution.
    MissingControlPoint {
        entity_id: EntityId,
        component: VectorComponent,
    },
}

impl std::fmt::Display for PathResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathResolveError::MissingControlPoint {
                entity_id,
                component,
            } => {
                write!(
                    f,
                    "Control point {:?}.{:?} not found in solution",
                    entity_id, component
                )
            }
        }
    }
}

impl std::error::Error for PathResolveError {}

/// Resolve path segments to path commands using solver output.
///
/// ## Phase G-0c: Segment → Command Conversion
///
/// Converts `PathSegment` (which references EntityIds) to `PathCommand`
/// (which contains actual Rational coordinates) using the solver's output.
///
/// ## Algorithm
///
/// 1. For each segment, lookup coordinates for all referenced EntityIds
/// 2. Generate MoveTo for the first segment's `from` point
/// 3. Generate the appropriate drawing command (LineTo, QuadTo, CubicTo, ArcTo)
/// 4. If `closed`, append Close command
///
/// ## Example
///
/// ```ignore
/// let segments = vec![
///     PathSegment::Line { from: EntityId(1), to: EntityId(2) },
///     PathSegment::Quad { from: EntityId(2), handle: EntityId(3), to: EntityId(4) },
/// ];
///
/// let get_coord = |id: EntityId, comp: VectorComponent| -> Option<Rational> {
///     solutions.get(&(id, comp)).cloned()
/// };
///
/// let commands = resolve_path_commands(&segments, true, get_coord)?;
/// // → [MoveTo(p1), LineTo(p2), QuadTo(h3, p4), Close]
/// ```
///
/// ## Continuity Assumption
///
/// Adjacent segments are assumed to share endpoints:
/// - `segments[i].to == segments[i+1].from`
///
/// The function does NOT emit redundant MoveTo commands between continuous segments.
/// If segments are disjoint, use separate `resolve_path_commands` calls for each subpath.
pub fn resolve_path_commands<F>(
    segments: &[PathSegment],
    closed: bool,
    get_coord: F,
) -> Result<Vec<PathCommand>, PathResolveError>
where
    F: Fn(EntityId, VectorComponent) -> Option<Rational>,
{
    if segments.is_empty() {
        return Ok(Vec::new());
    }

    let mut commands = Vec::with_capacity(segments.len() + 2); // +2 for MoveTo and Close

    // Helper to lookup X/Y coordinates
    let lookup = |id: EntityId| -> Result<(Rational, Rational), PathResolveError> {
        let x = get_coord(id, VectorComponent::X).ok_or(PathResolveError::MissingControlPoint {
            entity_id: id,
            component: VectorComponent::X,
        })?;
        let y = get_coord(id, VectorComponent::Y).ok_or(PathResolveError::MissingControlPoint {
            entity_id: id,
            component: VectorComponent::Y,
        })?;
        Ok((x, y))
    };

    // Emit MoveTo for the first segment's starting point
    let first_from = match &segments[0] {
        PathSegment::Line { from, .. } => *from,
        PathSegment::Quad { from, .. } => *from,
        PathSegment::Cubic { from, .. } => *from,
        PathSegment::Arc { from, .. } => *from,
    };
    let (start_x, start_y) = lookup(first_from)?;
    commands.push(PathCommand::MoveTo {
        x: start_x,
        y: start_y,
    });

    // Process each segment
    for segment in segments {
        match segment {
            PathSegment::Line { to, .. } => {
                let (x, y) = lookup(*to)?;
                commands.push(PathCommand::LineTo { x, y });
            }
            PathSegment::Quad { handle, to, .. } => {
                let (x1, y1) = lookup(*handle)?;
                let (x, y) = lookup(*to)?;
                commands.push(PathCommand::QuadTo { x1, y1, x, y });
            }
            PathSegment::Cubic {
                handle1,
                handle2,
                to,
                ..
            } => {
                let (x1, y1) = lookup(*handle1)?;
                let (x2, y2) = lookup(*handle2)?;
                let (x, y) = lookup(*to)?;
                commands.push(PathCommand::CubicTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                });
            }
            PathSegment::Arc {
                to,
                rx,
                ry,
                rotation,
                large_arc,
                sweep,
                ..
            } => {
                let (x, y) = lookup(*to)?;
                commands.push(PathCommand::ArcTo {
                    rx: rx.clone(),
                    ry: ry.clone(),
                    rotation: *rotation,
                    large_arc: *large_arc,
                    sweep: *sweep,
                    x,
                    y,
                });
            }
        }
    }

    // Close path if requested
    if closed {
        commands.push(PathCommand::Close);
    }

    Ok(commands)
}

/// A path entity composed of segments referencing control points.
///
/// ## Architectural Decision (Phase 6)
/// - Paths do NOT contain coordinate data directly
/// - All geometry is defined by ControlPoint entities referenced by segments
/// - The P-dimension solver resolves ControlPoint positions
/// - GpuRenderer receives resolved coordinates at rasterization boundary
///
/// ## Non-Linear Constraint Prohibition
/// The following are statically FORBIDDEN and will be rejected by the linter:
/// - Constraints targeting "a point on the curve at parameter t"
/// - Constraints involving curve-curve intersection points
/// - Constraints on curve normals, tangents, or curvature
///
/// These require solving polynomial equations, violating FM-decidability.
///
/// Note: PartialEq only due to PathSegment containing f64 (Arc.rotation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Path {
    /// Unique identifier for this path entity.
    pub id: EntityId,
    /// Ordered sequence of path segments.
    pub segments: Vec<PathSegment>,
    /// Fill rule for rendering (if closed).
    pub fill_rule: FillRule,
    /// Whether this path is closed.
    pub closed: bool,
}

impl Path {
    /// Create a new empty path.
    pub fn new(id: EntityId) -> Self {
        Self {
            id,
            segments: Vec::new(),
            fill_rule: FillRule::default(),
            closed: false,
        }
    }

    /// Collect all control point EntityIds referenced by this path.
    ///
    /// Note: Returns deduplicated IDs. Shared endpoints between segments
    /// are only included once.
    pub fn referenced_control_points(&self) -> Vec<EntityId> {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        let mut points = Vec::new();

        for segment in &self.segments {
            let ids: Vec<EntityId> = match segment {
                PathSegment::Line { from, to } => vec![*from, *to],
                PathSegment::Quad { from, handle, to } => vec![*from, *handle, *to],
                PathSegment::Cubic {
                    from,
                    handle1,
                    handle2,
                    to,
                } => {
                    vec![*from, *handle1, *handle2, *to]
                }
                PathSegment::Arc { from, to, .. } => vec![*from, *to],
            };

            for id in ids {
                if seen.insert(id) {
                    points.push(id);
                }
            }
        }
        points
    }
}

/// Entity types in the P-dimension space.
///
/// All entities have an EntityId and can be targets of constraints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "entity_type", rename_all = "snake_case")]
pub enum Entity {
    /// A primitive rectangle (legacy, will be migrated to Path).
    Rect { id: EntityId, bounds: RectBounds },
    /// A text element (legacy, use TextEntity for constraint-based layout).
    Text {
        id: EntityId,
        content: String,
        bounds: RectBounds,
    },
    /// A control point (first-class, Phase 6).
    ControlPoint(ControlPoint),
    /// A path composed of curve segments (Phase 6).
    Path(Path),
    /// A scalar radius value (Phase 7).
    Radius(Radius),
    /// An arc defined by center, radius, and endpoint ControlPoints (Phase 3 Remediation).
    Arc(Arc),
    /// A rounded rectangle macro (expands to ControlPoints + Arcs + Path).
    /// Note: This is a template, not stored directly in the constraint graph.
    RoundedRect(RoundedRect),
    /// A scalar angle value in degrees (Phase 3 Remediation).
    Angle(Angle),
    /// A text entity with bounding box control points (Phase 10).
    /// Macro-expands to 4 ControlPoints (TL, TR, BL, BR).
    TextEntity(TextEntity),
    /// A color stop for gradients (Phase 17).
    /// Channels (r, g, b, a, position) are constrainable scalars.
    ColorStop(ColorStop),
    /// A linear gradient (Phase 17).
    LinearGradient(LinearGradient),
    /// A radial gradient (Phase 17).
    RadialGradient(RadialGradient),
    /// A conic (sweep) gradient (Phase 17).
    ConicGradient(ConicGradient),
}

impl Entity {
    /// Get the EntityId of this entity.
    pub fn id(&self) -> EntityId {
        match self {
            Entity::Rect { id, .. } => *id,
            Entity::Text { id, .. } => *id,
            Entity::ControlPoint(cp) => cp.id,
            Entity::Path(p) => p.id,
            Entity::Radius(r) => r.id,
            Entity::Arc(a) => a.id,
            Entity::RoundedRect(rr) => rr.id,
            Entity::Angle(a) => a.id,
            Entity::TextEntity(te) => te.id,
            Entity::ColorStop(cs) => cs.id,
            Entity::LinearGradient(lg) => lg.id,
            Entity::RadialGradient(rg) => rg.id,
            Entity::ConicGradient(cg) => cg.id,
        }
    }

    /// Check if this entity is a constrainable coordinate entity.
    ///
    /// Returns true for entities whose X/Y coordinates can be constrained.
    /// Returns false for scalar entities (Radius, Angle) or composite entities (Path).
    pub fn is_coordinate_entity(&self) -> bool {
        matches!(
            self,
            Entity::Rect { .. } | Entity::Text { .. } | Entity::ControlPoint(_)
        )
    }

    /// Check if this entity is a scalar entity (Radius, Angle, ColorStop).
    ///
    /// Scalar entities use VectorComponent::Value for constraints.
    /// ColorStop has multiple scalar fields (r, g, b, a, position).
    pub fn is_scalar_entity(&self) -> bool {
        matches!(
            self,
            Entity::Radius(_) | Entity::Angle(_) | Entity::ColorStop(_)
        )
    }

    /// Check if this entity is a composite/path entity.
    ///
    /// Composite entities contain references to other entities but are not
    /// directly constrainable at the component level.
    pub fn is_composite_entity(&self) -> bool {
        matches!(self, Entity::Path(_) | Entity::Arc(_))
    }

    /// Check if this entity is a gradient entity.
    ///
    /// Gradient entities reference control points and color stops.
    pub fn is_gradient_entity(&self) -> bool {
        matches!(
            self,
            Entity::LinearGradient(_) | Entity::RadialGradient(_) | Entity::ConicGradient(_)
        )
    }

    /// Check if this entity is a macro template (expands to multiple entities).
    pub fn is_macro_entity(&self) -> bool {
        matches!(self, Entity::RoundedRect(_) | Entity::TextEntity(_))
    }
}

/// Bounds for a rectangle (legacy structure).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RectBounds {
    pub x: Rational,
    pub y: Rational,
    pub width: Rational,
    pub height: Rational,
}

// =============================================================================
// Phase 10: Text Entity with Bounding Box Control Points
// =============================================================================

/// A text entity with 4 bounding box control points for constraint-based layout.
///
/// ## Architectural Decision (Phase 10)
///
/// Text entities are **macro-expanded** into 4 `ControlPoint` entities representing
/// the bounding box corners (TL, TR, BL, BR). This enables:
///
/// 1. **Linear Constraints on Text Bounds**: Width/height can be expressed as:
///    - `TR.x - TL.x = W` (width from Renderer)
///    - `BL.y - TL.y = H` (height from Renderer)
///
/// 2. **Q→P Dimension Bridge**: The Renderer measures actual text dimensions
///    using wgpu renderer/DOM and feeds them back as constant constraints.
///
/// 3. **FM-Decidable Layout**: A button containing text can constrain:
///    - `button.left = text.TL.x - padding`
///    - `button.width = TR.x - TL.x + 2*padding`
///
/// ## Control Point Layout
///
/// ```text
///    TL ●━━━━━━━━━━━━━● TR
///       │   "Hello"   │
///    BL ●━━━━━━━━━━━━━● BR
/// ```
///
/// ## Text Rendering
///
/// The actual glyph rendering is delegated to GpuRenderer's `drawText` for performance.
/// Only when `expand-text-to-paths` is invoked are glyphs converted to Path entities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEntity {
    /// Unique identifier for this text entity.
    pub id: EntityId,

    /// The text content (UTF-8 string).
    pub content: String,

    /// Font family name (e.g., "Inter", "Roboto Mono").
    pub font_family: String,

    /// Font size in P-dimension units (references a scalar or constant).
    pub font_size: Rational,

    /// Line height multiplier (e.g., 1.5 for 150% line height).
    #[serde(default = "default_line_height")]
    pub line_height: Rational,

    /// Top-left corner control point.
    pub corner_tl: EntityId,

    /// Top-right corner control point.
    pub corner_tr: EntityId,

    /// Bottom-left corner control point.
    pub corner_bl: EntityId,

    /// Bottom-right corner control point.
    pub corner_br: EntityId,

    /// Whether metrics have been measured by the Renderer.
    /// When false, width/height constraints are pending.
    #[serde(default)]
    pub metrics_resolved: bool,
}

fn default_line_height() -> Rational {
    Rational::new(3, 2) // 1.5
}

impl TextEntity {
    /// Create a new text entity with auto-generated control point IDs.
    ///
    /// ## ID Allocation
    ///
    /// Given base ID `n`, the control points are assigned:
    /// - TL: `n + 1`
    /// - TR: `n + 2`
    /// - BL: `n + 3`
    /// - BR: `n + 4`
    ///
    /// The caller must ensure these IDs are reserved.
    pub fn new(id: EntityId, content: String, font_family: String, font_size: Rational) -> Self {
        let base = id.0;
        Self {
            id,
            content,
            font_family,
            font_size,
            line_height: default_line_height(),
            corner_tl: EntityId(base + 1),
            corner_tr: EntityId(base + 2),
            corner_bl: EntityId(base + 3),
            corner_br: EntityId(base + 4),
            metrics_resolved: false,
        }
    }

    /// Get all corner control point IDs.
    pub fn corner_ids(&self) -> [EntityId; 4] {
        [
            self.corner_tl,
            self.corner_tr,
            self.corner_bl,
            self.corner_br,
        ]
    }

    /// Generate the 4 control points for this text entity at a given origin.
    ///
    /// Initially all corners are placed at the origin (0, 0).
    /// The Renderer will update positions via `update-metrics`.
    pub fn expand_control_points(
        &self,
        origin_x: Rational,
        origin_y: Rational,
    ) -> Vec<ControlPoint> {
        vec![
            ControlPoint {
                id: self.corner_tl,
                position: PVector {
                    x: origin_x.clone(),
                    y: origin_y.clone(),
                    z: Rational::zero(),
                    t: Rational::zero(),
                },
                role: ControlPointRole::Anchor,
                parent_path: None,
            },
            ControlPoint {
                id: self.corner_tr,
                position: PVector {
                    x: origin_x.clone(),
                    y: origin_y.clone(),
                    z: Rational::zero(),
                    t: Rational::zero(),
                },
                role: ControlPointRole::Anchor,
                parent_path: None,
            },
            ControlPoint {
                id: self.corner_bl,
                position: PVector {
                    x: origin_x.clone(),
                    y: origin_y.clone(),
                    z: Rational::zero(),
                    t: Rational::zero(),
                },
                role: ControlPointRole::Anchor,
                parent_path: None,
            },
            ControlPoint {
                id: self.corner_br,
                position: PVector {
                    x: origin_x,
                    y: origin_y,
                    z: Rational::zero(),
                    t: Rational::zero(),
                },
                role: ControlPointRole::Anchor,
                parent_path: None,
            },
        ]
    }

    /// Generate width and height constraints from measured metrics.
    ///
    /// Returns constraints that enforce:
    /// - `TR.x - TL.x = width`
    /// - `BR.x - BL.x = width` (parallel)
    /// - `BL.y - TL.y = height`
    /// - `BR.y - TR.y = height` (parallel)
    ///
    /// Plus alignment constraints:
    /// - `TL.y = TR.y` (top edge horizontal)
    /// - `BL.y = BR.y` (bottom edge horizontal)
    /// - `TL.x = BL.x` (left edge vertical)
    /// - `TR.x = BR.x` (right edge vertical)
    pub fn generate_metrics_constraints(
        &self,
        width: Rational,
        height: Rational,
        base_constraint_id: u64,
    ) -> Vec<Constraint> {
        let mut constraints = Vec::new();
        let mut id = base_constraint_id;

        // Width constraint: TR.x = TL.x + width (Soft: can be overridden by measured metrics)
        constraints.push(Constraint {
            id,
            target: self.corner_tr,
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Linear {
                coefficient: Rational::one(),
                entity_id: self.corner_tl,
                component: VectorComponent::X,
                offset: width.clone(),
            },
            priority: ConstraintPriority::Soft,
            source_scope: None,
        });
        id += 1;

        // BR.x = BL.x + width (parallel width)
        constraints.push(Constraint {
            id,
            target: self.corner_br,
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Linear {
                coefficient: Rational::one(),
                entity_id: self.corner_bl,
                component: VectorComponent::X,
                offset: width,
            },
            priority: ConstraintPriority::Soft,
            source_scope: None,
        });
        id += 1;

        // Height constraint: BL.y = TL.y + height
        constraints.push(Constraint {
            id,
            target: self.corner_bl,
            component: VectorComponent::Y,
            relation: RelationType::Eq,
            term: ConstraintTerm::Linear {
                coefficient: Rational::one(),
                entity_id: self.corner_tl,
                component: VectorComponent::Y,
                offset: height.clone(),
            },
            priority: ConstraintPriority::Soft,
            source_scope: None,
        });
        id += 1;

        // BR.y = TR.y + height (parallel height)
        constraints.push(Constraint {
            id,
            target: self.corner_br,
            component: VectorComponent::Y,
            relation: RelationType::Eq,
            term: ConstraintTerm::Linear {
                coefficient: Rational::one(),
                entity_id: self.corner_tr,
                component: VectorComponent::Y,
                offset: height,
            },
            priority: ConstraintPriority::Soft,
            source_scope: None,
        });
        id += 1;

        // Top edge horizontal: TR.y = TL.y (Hard: structural)
        constraints.push(Constraint {
            id,
            target: self.corner_tr,
            component: VectorComponent::Y,
            relation: RelationType::Eq,
            term: ConstraintTerm::Ref {
                entity_id: self.corner_tl,
                component: VectorComponent::Y,
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        });
        id += 1;

        // Bottom edge horizontal: BR.y = BL.y (Hard: structural)
        constraints.push(Constraint {
            id,
            target: self.corner_br,
            component: VectorComponent::Y,
            relation: RelationType::Eq,
            term: ConstraintTerm::Ref {
                entity_id: self.corner_bl,
                component: VectorComponent::Y,
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        });
        id += 1;

        // Left edge vertical: BL.x = TL.x (Hard: structural)
        constraints.push(Constraint {
            id,
            target: self.corner_bl,
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Ref {
                entity_id: self.corner_tl,
                component: VectorComponent::X,
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        });
        id += 1;

        // Right edge vertical: BR.x = TR.x (Hard: structural)
        constraints.push(Constraint {
            id,
            target: self.corner_br,
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Ref {
                entity_id: self.corner_tr,
                component: VectorComponent::X,
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        });

        constraints
    }
}

// =============================================================================
// Phase 7: Algebraic Topology and Non-Linear Boundary Resolution
// =============================================================================

/// A scalar radius value in P-dimension space.
///
/// ## Architectural Decision (Phase 7)
///
/// Radius is a **scalar** quantity, not a locus. The equation x² + y² = R² defines
/// a circular locus, but evaluating points ON that locus requires irrational numbers
/// (√2, etc.), breaking FM-decidability.
///
/// Instead, we:
/// 1. Store R as a first-class rational scalar entity
/// 2. Allow linear constraints on R itself (e.g., R1 = R2 + offset)
/// 3. **PROHIBIT** constraints that reference "a point on the circle"
/// 4. Defer circle rendering to the rasterization boundary (GpuRenderer)
///
/// ## What IS Allowed
/// - `Radius.value = 50/1` (constant assignment)
/// - `Radius1.value = Radius2.value * 2` (linear relationship)
/// - `Arc.radius = Radius.value` (reference to radius entity)
///
/// ## What is FORBIDDEN (Linter rejects)
/// - `ControlPoint.x = center.x + radius * cos(θ)` (locus evaluation)
/// - `constraint: x² + y² <= R²` (quadratic constraint)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Radius {
    /// Unique identifier for this radius entity.
    pub id: EntityId,
    /// The scalar radius value (exact rational).
    pub value: Rational,
}

impl Radius {
    /// Create a new radius with the given value.
    pub fn new(id: EntityId, value: Rational) -> Self {
        Self { id, value }
    }

    /// Convert to f64 for RASTERIZATION ONLY.
    #[inline]
    pub fn to_f64_for_rasterization(&self) -> f64 {
        self.value.to_f64_for_rasterization()
    }
}

/// An arc entity defined by center, radius, and endpoint ControlPoints.
///
/// ## Architectural Decision (Phase 3 Remediation)
///
/// An arc is parameterized by:
/// - **Center**: A `ControlPoint` entity (linearly constrainable)
/// - **Radius**: A `Radius` entity (scalar, linearly constrainable)
/// - **Start/End Points**: `ControlPoint` entities (first-class, linearly constrainable)
///
/// The start and end points are constrained to lie ON the circle via
/// `CircumferenceConstraint`, which is a quadratic constraint:
///   (P.x - C.x)² + (P.y - C.y)² = R²
///
/// This quadratic constraint is placed in the Suspended Queue and evaluated
/// via lazy promotion when the center/radius are resolved.
///
/// ## Why Endpoints Instead of Angles
///
/// Angles (θ) require trigonometric evaluation to compute positions:
///   P.x = C.x + R * cos(θ)  ← NON-LINEAR (trig function)
///
/// By using explicit ControlPoints, we can:
/// 1. Apply standard linear constraints to endpoint coordinates
/// 2. Connect lines/curves to arc endpoints via EntityId reference
/// 3. Enforce G1 continuity at junctions using collinearity constraints
///
/// ## G1 Continuity
///
/// To smoothly connect a line to an arc at the endpoint:
/// 1. The line endpoint shares the arc's start_point or end_point
/// 2. A TangentConstraint ensures collinearity with the radial direction
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arc {
    /// Unique identifier for this arc entity.
    pub id: EntityId,
    /// Center point (must reference a ControlPoint entity).
    pub center: EntityId,
    /// Radius (must reference a Radius entity).
    pub radius: EntityId,
    /// Start point on the circumference (must reference a ControlPoint entity).
    /// Constrained to satisfy: (start.x - center.x)² + (start.y - center.y)² = radius²
    pub start_point: EntityId,
    /// End point on the circumference (must reference a ControlPoint entity).
    /// Constrained to satisfy: (end.x - center.x)² + (end.y - center.y)² = radius²
    pub end_point: EntityId,
    /// Whether to draw the arc clockwise from start to end.
    pub clockwise: bool,
}

impl Arc {
    /// Create a new arc with explicit endpoint ControlPoints.
    pub fn new(
        id: EntityId,
        center: EntityId,
        radius: EntityId,
        start_point: EntityId,
        end_point: EntityId,
    ) -> Self {
        Self {
            id,
            center,
            radius,
            start_point,
            end_point,
            clockwise: false,
        }
    }

    /// Set the arc direction to clockwise.
    pub fn clockwise(mut self) -> Self {
        self.clockwise = true;
        self
    }
}

/// A rounded rectangle as a macro-expansion template.
///
/// ## Architectural Decision (Phase 3 Remediation)
///
/// `RoundedRect` is NOT a primitive entity stored in the constraint graph.
/// Instead, it is a **macro** that expands into:
///
/// - 4 `Radius` entities (corner radii)
/// - 8 `ControlPoint` entities (tangent points at each corner)
/// - 4 `Arc` entities (corner arcs)
/// - 4 line segments (edges connecting tangent points)
/// - 1 `Path` entity (closed path combining all segments)
///
/// This ensures all internal geometry is exposed as first-class entities
/// with `EntityId`s that can be freely referenced and constrained.
///
/// ## Expansion Geometry
///
/// ```text
///            tangent_tl_top         tangent_tr_top
///                  ●━━━━━━━━━━━━━━━━━━━━━●
///                 ╱                       ╲
///   tangent_tl_left●       (arc_tl)       (arc_tr)●tangent_tr_right
///                  │                       │
///                  │                       │
///                  │                       │
///   tangent_bl_left●       (arc_bl)       (arc_br)●tangent_br_right
///                 ╲                       ╱
///                  ●━━━━━━━━━━━━━━━━━━━━━●
///           tangent_bl_bottom     tangent_br_bottom
/// ```
///
/// ## Usage
///
/// When `vsc add-entity --type=rounded-rect` is invoked, the CLI
/// internally calls `RoundedRect::expand()` to generate all constituent
/// entities and their constraints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundedRect {
    /// Unique identifier for the macro (not stored in constraint graph).
    pub id: EntityId,
    /// Bounds (x, y, width, height).
    pub bounds: RectBounds,
    /// Top-left corner radius (references Radius entity).
    pub radius_tl: EntityId,
    /// Top-right corner radius.
    pub radius_tr: EntityId,
    /// Bottom-right corner radius.
    pub radius_br: EntityId,
    /// Bottom-left corner radius.
    pub radius_bl: EntityId,
}

/// Result of expanding a RoundedRect macro into constituent entities.
#[derive(Debug, Clone)]
pub struct RoundedRectExpansion {
    /// The 8 tangent points (corners where arcs meet edges).
    pub tangent_points: RoundedRectTangentPoints,
    /// The 4 corner arc entities.
    pub arcs: RoundedRectArcs,
    /// The closed path combining all segments.
    pub path: Path,
    /// Linear constraints positioning tangent points relative to bounds.
    pub positioning_constraints: Vec<Constraint>,
    /// Circumference constraints ensuring arc endpoints lie on circles.
    pub circumference_constraints: Vec<CircumferenceConstraint>,
}

/// The 8 tangent points of a rounded rectangle.
#[derive(Debug, Clone)]
pub struct RoundedRectTangentPoints {
    pub tl_top: ControlPoint,
    pub tl_left: ControlPoint,
    pub tr_top: ControlPoint,
    pub tr_right: ControlPoint,
    pub br_right: ControlPoint,
    pub br_bottom: ControlPoint,
    pub bl_bottom: ControlPoint,
    pub bl_left: ControlPoint,
}

/// The 4 corner arcs of a rounded rectangle.
#[derive(Debug, Clone)]
pub struct RoundedRectArcs {
    pub tl: Arc,
    pub tr: Arc,
    pub br: Arc,
    pub bl: Arc,
}

/// A tangent constraint ensuring G1 continuity between curves.
///
/// ## Linearization Strategy (Phase 7)
///
/// G1 continuity (tangent matching) at a junction requires the tangent vectors
/// of both curves to be parallel. For a cubic Bezier, the tangent at an endpoint
/// is the direction from the endpoint to its adjacent control handle.
///
/// Given:
/// - Point P (junction)
/// - Handle H1 (from curve 1)
/// - Handle H2 (from curve 2)
///
/// G1 continuity requires: P, H1, H2 are collinear.
///
/// This is linearized as:
/// ```text
/// (H1.y - P.y) * (H2.x - P.x) = (H2.y - P.y) * (H1.x - P.x)
/// ```
///
/// Which expands to a linear constraint on the control point coordinates.
///
/// ## Why This Works
///
/// The collinearity constraint avoids division (slope comparison) and uses only
/// multiplication of rational coordinates, producing a rational result that
/// FM-elimination can process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TangentConstraint {
    /// Unique identifier for this tangent constraint.
    pub id: u64,
    /// The junction point where curves meet.
    pub junction: EntityId,
    /// Handle from the first curve (incoming).
    pub handle1: EntityId,
    /// Handle from the second curve (outgoing).
    pub handle2: EntityId,
    /// Constraint intent for debugging.
    pub intent: Option<String>,
}

impl TangentConstraint {
    /// Create a new tangent constraint.
    pub fn new(id: u64, junction: EntityId, handle1: EntityId, handle2: EntityId) -> Self {
        Self {
            id,
            junction,
            handle1,
            handle2,
            intent: None,
        }
    }

    /// Set the intent description.
    pub fn with_intent(mut self, intent: impl Into<String>) -> Self {
        self.intent = Some(intent.into());
        self
    }

    /// Expand this tangent constraint into linear coordinate constraints.
    ///
    /// Returns constraints that enforce:
    /// `(H1.y - P.y) * (H2.x - P.x) = (H2.y - P.y) * (H1.x - P.x)`
    ///
    /// This is represented as a `BilinearTerm` that the solver can handle.
    pub fn to_bilinear_form(&self) -> CollinearityConstraint {
        CollinearityConstraint {
            point_a: self.junction,
            point_b: self.handle1,
            point_c: self.handle2,
        }
    }
}

/// A collinearity constraint ensuring three points lie on the same line.
///
/// ## Mathematical Form
///
/// For points A, B, C to be collinear:
/// ```text
/// (B.y - A.y) * (C.x - A.x) = (C.y - A.y) * (B.x - A.x)
/// ```
///
/// Expanding:
/// ```text
/// B.y * C.x - B.y * A.x - A.y * C.x + A.y * A.x
///   = C.y * B.x - C.y * A.x - A.y * B.x + A.y * A.x
/// ```
///
/// Simplifying:
/// ```text
/// B.y * C.x - B.y * A.x - A.y * C.x = C.y * B.x - C.y * A.x - A.y * B.x
/// ```
///
/// This involves products of coordinates (bilinear), not squares (quadratic).
/// The FM solver can handle this through variable substitution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollinearityConstraint {
    /// First point (typically the junction).
    pub point_a: EntityId,
    /// Second point (typically handle 1).
    pub point_b: EntityId,
    /// Third point (typically handle 2).
    pub point_c: EntityId,
}

/// A circumference constraint ensuring a point lies on a circle.
///
/// ## Mathematical Form
///
/// For point P to lie on a circle with center C and radius R:
/// ```text
/// (P.x - C.x)² + (P.y - C.y)² = R²
/// ```
///
/// This is a **quadratic** constraint that cannot be directly processed
/// by FM elimination. It is placed in the Suspended Queue and handled
/// via lazy evaluation.
///
/// ## Lazy Evaluation Strategy
///
/// When center C and radius R are both resolved (DoF = 0):
/// 1. The circumference constraint can compute valid positions for P
/// 2. If P has additional linear constraints, they are combined to find
///    the intersection of the line with the circle
/// 3. The resulting position(s) are substituted back
///
/// ## Use Case: Arc Endpoints
///
/// Every `Arc` entity generates two `CircumferenceConstraint`s:
/// - `arc.start_point` must lie on circle(arc.center, arc.radius)
/// - `arc.end_point` must lie on circle(arc.center, arc.radius)
///
/// This allows other entities to connect to arc endpoints via `EntityId`
/// while maintaining the geometric invariant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CircumferenceConstraint {
    /// Unique identifier for this constraint.
    pub id: u64,
    /// The point that must lie on the circumference.
    pub point: EntityId,
    /// The center of the circle.
    pub center: EntityId,
    /// The radius of the circle (references a Radius entity).
    pub radius: EntityId,
    /// Constraint intent for debugging.
    pub intent: Option<String>,
}

impl CircumferenceConstraint {
    /// Create a new circumference constraint.
    pub fn new(id: u64, point: EntityId, center: EntityId, radius: EntityId) -> Self {
        Self {
            id,
            point,
            center,
            radius,
            intent: None,
        }
    }

    /// Set the intent description.
    pub fn with_intent(mut self, intent: impl Into<String>) -> Self {
        self.intent = Some(intent.into());
        self
    }
}

/// An angle entity representing a rotational parameter.
///
/// ## Architectural Decision (Phase 3 Remediation)
///
/// Angles are first-class scalar entities with `EntityId`, enabling:
/// - Linear constraints on angle values (e.g., θ1 = θ2 + 90°)
/// - Dynamic angle relationships
/// - Constraint-based rotation control
///
/// The angle value is stored in degrees as a rational number.
/// Conversion to radians happens only at the rasterization boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Angle {
    /// Unique identifier for this angle entity.
    pub id: EntityId,
    /// The angle value in degrees (exact rational).
    pub value: Rational,
}

impl Angle {
    /// Create a new angle with the given value in degrees.
    pub fn new(id: EntityId, degrees: Rational) -> Self {
        Self { id, value: degrees }
    }

    /// Create a new angle from an integer degree value.
    pub fn from_degrees(id: EntityId, degrees: i64) -> Self {
        Self {
            id,
            value: Rational::from_int(degrees),
        }
    }

    /// Convert to radians for RASTERIZATION ONLY.
    #[inline]
    pub fn to_radians_for_rasterization(&self) -> f64 {
        self.value.to_f64_for_rasterization() * std::f64::consts::PI / 180.0
    }
}

// =============================================================================
// Phase 17: CSS-Compatible Gradient Entities
// =============================================================================

/// Tile mode for gradient overflow behavior.
///
/// Maps directly to GPU TileMode enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TileMode {
    /// Clamp to edge colors (default CSS behavior)
    #[default]
    Clamp,
    /// Repeat the gradient pattern
    Repeat,
    /// Mirror/reflect the gradient pattern
    Mirror,
    /// Transparent outside gradient bounds
    Decal,
}

/// A color stop in a gradient.
///
/// ## Phase 17: P-Dimension Color Representation
///
/// Each RGBA channel is stored as exact Rational:
/// - R, G, B: [0, 255] integer range as Rational
/// - A: [0, 1] normalized range as Rational
/// - Position: [0, 1] gradient progress as Rational
///
/// This enables:
/// 1. Exact color interpolation without floating-point error
/// 2. Linear constraints on color channels (e.g., `stop.r = 255 * T.hover`)
/// 3. Deterministic gradient rendering across platforms
///
/// ## Constraint Targeting
///
/// ColorStop fields are targeted via `VectorComponent`:
/// - `VectorComponent::Value` with field specifier for r/g/b/a/position
///
/// For simplicity, we use a compound entity approach where each channel
/// can be individually constrained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorStop {
    /// Unique identifier for this color stop.
    pub id: EntityId,
    /// Red channel [0, 255] as exact Rational.
    pub r: Rational,
    /// Green channel [0, 255] as exact Rational.
    pub g: Rational,
    /// Blue channel [0, 255] as exact Rational.
    pub b: Rational,
    /// Alpha channel [0, 1] as exact Rational.
    pub a: Rational,
    /// Position along gradient axis [0, 1] as exact Rational.
    pub position: Rational,
}

impl ColorStop {
    /// Create a new color stop with RGBA values and position.
    pub fn new(
        id: EntityId,
        r: Rational,
        g: Rational,
        b: Rational,
        a: Rational,
        position: Rational,
    ) -> Self {
        Self {
            id,
            r,
            g,
            b,
            a,
            position,
        }
    }

    /// Create from integer RGB values (alpha = 1, position must be specified).
    pub fn from_rgb(id: EntityId, r: u8, g: u8, b: u8, position: Rational) -> Self {
        Self {
            id,
            r: Rational::from_int(r as i64),
            g: Rational::from_int(g as i64),
            b: Rational::from_int(b as i64),
            a: Rational::one(),
            position,
        }
    }

    /// Create from CSS color name (limited palette).
    pub fn from_css_color(id: EntityId, color: &str, position: Rational) -> Option<Self> {
        let (r, g, b) = match color.to_lowercase().as_str() {
            "red" => (255, 0, 0),
            "green" => (0, 128, 0),
            "blue" => (0, 0, 255),
            "white" => (255, 255, 255),
            "black" => (0, 0, 0),
            "yellow" => (255, 255, 0),
            "cyan" | "aqua" => (0, 255, 255),
            "magenta" | "fuchsia" => (255, 0, 255),
            "orange" => (255, 165, 0),
            "purple" => (128, 0, 128),
            "pink" => (255, 192, 203),
            "gray" | "grey" => (128, 128, 128),
            "transparent" => {
                return Some(Self {
                    id,
                    r: Rational::zero(),
                    g: Rational::zero(),
                    b: Rational::zero(),
                    a: Rational::zero(),
                    position,
                })
            }
            _ => return None,
        };
        Some(Self::from_rgb(id, r, g, b, position))
    }

    /// Convert to f32 array [r, g, b, a] normalized to [0, 1] for RASTERIZATION ONLY.
    #[inline]
    pub fn to_f32_normalized_for_rasterization(&self) -> [f32; 4] {
        [
            (self.r.to_f64_for_rasterization() / 255.0) as f32,
            (self.g.to_f64_for_rasterization() / 255.0) as f32,
            (self.b.to_f64_for_rasterization() / 255.0) as f32,
            self.a.to_f64_for_rasterization() as f32,
        ]
    }

    /// Convert position to f32 for RASTERIZATION ONLY.
    #[inline]
    pub fn position_f32_for_rasterization(&self) -> f32 {
        self.position.to_f64_for_rasterization() as f32
    }
}

/// Linear gradient defined by two control points.
///
/// ## Phase 17: P-Dimension Integration
///
/// A linear gradient is fully determined by:
/// - Start point: `ControlPoint` entity (linearly constrainable)
/// - End point: `ControlPoint` entity (linearly constrainable)
/// - Color stops: Array of `ColorStop` entities (channels constrainable)
///
/// ## CSS Mapping
///
/// ```css
/// linear-gradient(45deg, red 0%, blue 100%)
/// ```
///
/// The angle (45deg) is converted to start/end control points via:
/// - Center = bounding box center
/// - Direction vector = (sin(θ), -cos(θ)) in CSS convention
/// - Length = diagonal projection onto gradient axis
///
/// ## Constraint Examples
///
/// ```text
/// start.x = bounds.left
/// end.x = bounds.right
/// stop[1].position = 0.5 + 0.3 * T.hover  // Animated stop position
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearGradient {
    /// Unique identifier for this gradient entity.
    pub id: EntityId,
    /// Start point (must reference a ControlPoint entity).
    pub start: EntityId,
    /// End point (must reference a ControlPoint entity).
    pub end: EntityId,
    /// Color stops (must reference ColorStop entities).
    /// Minimum 2 stops required.
    pub stops: Vec<EntityId>,
    /// Tile mode for out-of-bounds behavior.
    #[serde(default)]
    pub tile_mode: TileMode,
}

impl LinearGradient {
    /// Create a new linear gradient with start/end points and color stops.
    pub fn new(id: EntityId, start: EntityId, end: EntityId, stops: Vec<EntityId>) -> Self {
        Self {
            id,
            start,
            end,
            stops,
            tile_mode: TileMode::default(),
        }
    }

    /// Set tile mode (builder pattern).
    pub fn with_tile_mode(mut self, mode: TileMode) -> Self {
        self.tile_mode = mode;
        self
    }
}

/// Radial gradient defined by center, radii, and optional focal point.
///
/// ## Phase 17: P-Dimension Integration
///
/// A radial gradient is determined by:
/// - Center: `ControlPoint` entity
/// - Radius X: `Radius` entity (for elliptical gradients)
/// - Radius Y: `Radius` entity (may equal radius_x for circles)
/// - Focal point: Optional `ControlPoint` for two-point conical gradients
/// - Focal radius: Optional `Radius` for focal circle size
///
/// ## CSS Mapping
///
/// ```css
/// radial-gradient(circle at 50% 50%, red, blue)
/// radial-gradient(ellipse 100px 50px at center, red 0%, blue 100%)
/// ```
///
/// ## GPU Shader Mapping
///
/// - Circle: `MakeRadialGradient(center, radius, colors, pos, mode)`
/// - Ellipse: Uses local matrix transform
/// - Two-point conical: `MakeTwoPointConicalGradientShader(start, startR, end, endR, ...)`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RadialGradient {
    /// Unique identifier for this gradient entity.
    pub id: EntityId,
    /// Center point (must reference a ControlPoint entity).
    pub center: EntityId,
    /// X-axis radius (must reference a Radius entity).
    pub radius_x: EntityId,
    /// Y-axis radius (must reference a Radius entity).
    /// For circles, this should reference the same entity as radius_x.
    pub radius_y: EntityId,
    /// Optional focal point for two-point conical gradients.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focal_point: Option<EntityId>,
    /// Optional focal radius for two-point conical gradients.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focal_radius: Option<EntityId>,
    /// Color stops (must reference ColorStop entities).
    pub stops: Vec<EntityId>,
    /// Tile mode for out-of-bounds behavior.
    #[serde(default)]
    pub tile_mode: TileMode,
}

impl RadialGradient {
    /// Create a circular radial gradient (radius_x = radius_y).
    pub fn circle(id: EntityId, center: EntityId, radius: EntityId, stops: Vec<EntityId>) -> Self {
        Self {
            id,
            center,
            radius_x: radius,
            radius_y: radius,
            focal_point: None,
            focal_radius: None,
            stops,
            tile_mode: TileMode::default(),
        }
    }

    /// Create an elliptical radial gradient.
    pub fn ellipse(
        id: EntityId,
        center: EntityId,
        radius_x: EntityId,
        radius_y: EntityId,
        stops: Vec<EntityId>,
    ) -> Self {
        Self {
            id,
            center,
            radius_x,
            radius_y,
            focal_point: None,
            focal_radius: None,
            stops,
            tile_mode: TileMode::default(),
        }
    }

    /// Set focal point for two-point conical gradient (builder pattern).
    pub fn with_focal(mut self, focal_point: EntityId, focal_radius: EntityId) -> Self {
        self.focal_point = Some(focal_point);
        self.focal_radius = Some(focal_radius);
        self
    }

    /// Set tile mode (builder pattern).
    pub fn with_tile_mode(mut self, mode: TileMode) -> Self {
        self.tile_mode = mode;
        self
    }
}

/// Conic (sweep) gradient defined by center and rotation.
///
/// ## Phase 17: P-Dimension Integration
///
/// A conic gradient is determined by:
/// - Center: `ControlPoint` entity
/// - Rotation: `Angle` entity (rotation from top, clockwise)
/// - Start angle: `Angle` entity (where gradient begins, default 0°)
/// - End angle: `Angle` entity (where gradient ends, default 360°)
///
/// ## CSS Mapping
///
/// ```css
/// conic-gradient(from 45deg at center, red, blue)
/// conic-gradient(from 0deg at 50% 50%, red 0deg, blue 360deg)
/// ```
///
/// ## GPU Shader Mapping
///
/// `MakeSweepGradient(cx, cy, colors, pos, mode, matrix, flags, startAngle, endAngle)`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConicGradient {
    /// Unique identifier for this gradient entity.
    pub id: EntityId,
    /// Center point (must reference a ControlPoint entity).
    pub center: EntityId,
    /// Rotation angle from top (must reference an Angle entity).
    /// Default: 0° (gradient starts from top).
    pub rotation: EntityId,
    /// Start angle where gradient begins (must reference an Angle entity).
    /// Default: 0°.
    pub start_angle: EntityId,
    /// End angle where gradient ends (must reference an Angle entity).
    /// Default: 360°.
    pub end_angle: EntityId,
    /// Color stops (must reference ColorStop entities).
    pub stops: Vec<EntityId>,
}

impl ConicGradient {
    /// Create a new conic gradient.
    pub fn new(
        id: EntityId,
        center: EntityId,
        rotation: EntityId,
        start_angle: EntityId,
        end_angle: EntityId,
        stops: Vec<EntityId>,
    ) -> Self {
        Self {
            id,
            center,
            rotation,
            start_angle,
            end_angle,
            stops,
        }
    }
}

// =============================================================================
// Topology Constraint Types (Phase G Rasterization)
// =============================================================================

/// Edge direction for coordinate reference in topology constraints.
///
/// Represents which edge of an entity's bounding box a coordinate refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Coordinate reference: (EntityId, Edge) tuple.
///
/// Identifies a specific edge coordinate of an entity for topology constraints.
pub type CoordRef = (EntityId, Edge);

/// Topological constraint between coordinates.
///
/// Used by the topology-preserving rounding algorithm to ensure adjacent
/// surfaces maintain their spatial relationships after rasterization.
///
/// ## Example
///
/// If entity A's right edge touches entity B's left edge, we create:
/// ```ignore
/// TopoConstraint::Adjacent {
///     a: (EntityId(1), Edge::Right),
///     b: (EntityId(2), Edge::Left),
/// }
/// ```
///
/// This ensures both coordinates round to the same pixel value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopoConstraint {
    /// A and B must round to same pixel (shared boundary).
    Equal { a: CoordRef, b: CoordRef },
    /// A must round to less than B (strict ordering).
    LessThan { a: CoordRef, b: CoordRef },
    /// A.right touches B.left (no gap, no overlap).
    /// Equivalent to Equal for rounding purposes.
    Adjacent { a: CoordRef, b: CoordRef },
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test Rational serialization produces "num/den" string format.
    #[test]
    fn test_rational_serialize() {
        let r = Rational::new(100, 3);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#""100/3""#);
    }

    /// Test Rational deserialization from "num/den" string format.
    ///
    /// This is the format TypeScript sends via `wasmBoundaryReplacer`.
    #[test]
    fn test_rational_deserialize_string_format() {
        let json = r#""100/3""#;
        let r: Rational = serde_json::from_str(json).unwrap();
        assert_eq!(r, Rational::new(100, 3));
    }

    /// Test negative Rational deserialization.
    #[test]
    fn test_rational_deserialize_negative() {
        let json = r#""-42/7""#;
        let r: Rational = serde_json::from_str(json).unwrap();
        assert_eq!(r, Rational::new(-42, 7));
        assert_eq!(r, Rational::from_int(-6)); // -42/7 = -6
    }

    /// Test large Rational (bigint range) deserialization.
    #[test]
    fn test_rational_deserialize_large() {
        // JavaScript bigint can exceed i64 range
        let json = r#""9007199254740993/1""#; // 2^53 + 1
        let r: Rational = serde_json::from_str(json).unwrap();
        // Verify it parsed correctly
        assert!(r > Rational::from_int(9007199254740992));
    }

    /// Test Rational roundtrip (serialize → deserialize).
    #[test]
    fn test_rational_roundtrip() {
        let original = Rational::new(355, 113); // π approximation
        let json = serde_json::to_string(&original).unwrap();
        let restored: Rational = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    /// Test invalid format is rejected.
    #[test]
    fn test_rational_deserialize_invalid_format() {
        let json = r#""not-a-rational""#;
        let result: Result<Rational, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    /// Test division by zero in denominator.
    #[test]
    #[should_panic]
    fn test_rational_zero_denominator() {
        // This should panic or error in Rational::new
        let _ = Rational::new(1, 0);
    }

    // =========================================================================
    // f32_to_rational_exact Tests (Phase E)
    // =========================================================================

    /// Test integer coordinates (TrueType typical case).
    ///
    /// TrueType fonts store coordinates as i16 in font units.
    /// ttf-parser converts these to f32 without loss.
    #[test]
    fn test_f32_to_rational_exact_integer() {
        assert_eq!(f32_to_rational_exact(100.0), Rational::new(100, 1));
        assert_eq!(f32_to_rational_exact(-100.0), Rational::new(-100, 1));
        assert_eq!(f32_to_rational_exact(0.0), Rational::zero());
        assert_eq!(f32_to_rational_exact(1.0), Rational::one());
        assert_eq!(f32_to_rational_exact(-1.0), Rational::new(-1, 1));

        // i16 boundary values
        assert_eq!(f32_to_rational_exact(32767.0), Rational::new(32767, 1));
        assert_eq!(f32_to_rational_exact(-32768.0), Rational::new(-32768, 1));
    }

    /// Test power-of-2 fractions (CFF typical case).
    ///
    /// CFF fonts use fixed-point coordinates that result in binary fractions.
    #[test]
    fn test_f32_to_rational_exact_binary_fraction() {
        // 0.75 = 3/4
        assert_eq!(f32_to_rational_exact(0.75), Rational::new(3, 4));

        // 0.5 = 1/2
        assert_eq!(f32_to_rational_exact(0.5), Rational::new(1, 2));

        // 0.25 = 1/4
        assert_eq!(f32_to_rational_exact(0.25), Rational::new(1, 4));

        // 0.125 = 1/8
        assert_eq!(f32_to_rational_exact(0.125), Rational::new(1, 8));

        // Negative binary fraction
        assert_eq!(f32_to_rational_exact(-0.75), Rational::new(-3, 4));
    }

    /// Test non-trivial fractions.
    #[test]
    fn test_f32_to_rational_exact_nontrivial() {
        // 33.5 = 67/2
        assert_eq!(f32_to_rational_exact(33.5), Rational::new(67, 2));

        // 1.5 = 3/2
        assert_eq!(f32_to_rational_exact(1.5), Rational::new(3, 2));

        // 10.25 = 41/4
        assert_eq!(f32_to_rational_exact(10.25), Rational::new(41, 4));

        // -100.5 = -201/2
        assert_eq!(f32_to_rational_exact(-100.5), Rational::new(-201, 2));
    }

    /// Test roundtrip: f32 → Rational → f64 preserves value.
    #[test]
    fn test_f32_to_rational_exact_roundtrip() {
        let test_values = [
            0.0f32,
            1.0,
            -1.0,
            0.5,
            -0.5,
            0.75,
            0.125,
            100.0,
            -100.0,
            33.5,
            12345.6875,
            f32::MIN_POSITIVE, // Smallest positive normalized
        ];

        for &v in &test_values {
            let r = f32_to_rational_exact(v);
            let back = r.to_f64_for_rasterization();
            assert!(
                (back - v as f64).abs() < 1e-10,
                "Roundtrip failed for {}: got {}",
                v,
                back
            );
        }
    }

    /// Test denormalized (subnormal) numbers.
    #[test]
    fn test_f32_to_rational_exact_denormal() {
        // Smallest positive denormal
        let denormal = f32::from_bits(1); // 2^(-149)
        let r = f32_to_rational_exact(denormal);
        // Should be 1 / 2^149
        assert!(r > Rational::zero());
        assert!(r < Rational::new(1, 1_000_000_000)); // Very small
    }

    /// Test special cases panic as expected.
    #[test]
    #[should_panic(expected = "NaN")]
    fn test_f32_to_rational_exact_nan() {
        f32_to_rational_exact(f32::NAN);
    }

    #[test]
    #[should_panic(expected = "Infinity")]
    fn test_f32_to_rational_exact_infinity() {
        f32_to_rational_exact(f32::INFINITY);
    }

    #[test]
    #[should_panic(expected = "Infinity")]
    fn test_f32_to_rational_exact_neg_infinity() {
        f32_to_rational_exact(f32::NEG_INFINITY);
    }

    // =========================================================================
    // Phase G-0c: resolve_path_commands Tests
    // =========================================================================

    /// Test PathSegment::Quad → PathCommand::QuadTo resolution.
    ///
    /// Verifies that a quadratic Bezier segment with 3 control points
    /// (from, handle, to) correctly resolves to MoveTo + QuadTo commands.
    #[test]
    fn test_resolve_path_commands_quad() {
        // Define EntityIds for the 3 control points
        let from_id = EntityId(101);
        let handle_id = EntityId(102);
        let to_id = EntityId(103);

        // Create a single Quad segment
        let segments = vec![PathSegment::Quad {
            from: from_id,
            handle: handle_id,
            to: to_id,
        }];

        // Solver output: provide coordinates for each control point
        // from: (0, 0), handle: (50, 100), to: (100, 0)
        let get_coord = |id: EntityId, component: VectorComponent| -> Option<Rational> {
            match (id, component) {
                (EntityId(101), VectorComponent::X) => Some(Rational::from_int(0)),
                (EntityId(101), VectorComponent::Y) => Some(Rational::from_int(0)),
                (EntityId(102), VectorComponent::X) => Some(Rational::from_int(50)),
                (EntityId(102), VectorComponent::Y) => Some(Rational::from_int(100)),
                (EntityId(103), VectorComponent::X) => Some(Rational::from_int(100)),
                (EntityId(103), VectorComponent::Y) => Some(Rational::from_int(0)),
                _ => None,
            }
        };

        // Resolve with closed=false (open path)
        let commands = resolve_path_commands(&segments, false, get_coord).unwrap();

        // Expected: MoveTo(0, 0), QuadTo(50, 100, 100, 0)
        assert_eq!(commands.len(), 2);

        // First command: MoveTo to 'from' point
        assert!(
            matches!(
                &commands[0],
                PathCommand::MoveTo { x, y }
                if *x == Rational::from_int(0) && *y == Rational::from_int(0)
            ),
            "Expected MoveTo(0, 0), got {:?}",
            commands[0]
        );

        // Second command: QuadTo with handle and to coordinates
        assert!(
            matches!(
                &commands[1],
                PathCommand::QuadTo { x1, y1, x, y }
                if *x1 == Rational::from_int(50)
                    && *y1 == Rational::from_int(100)
                    && *x == Rational::from_int(100)
                    && *y == Rational::from_int(0)
            ),
            "Expected QuadTo(50, 100, 100, 0), got {:?}",
            commands[1]
        );
    }

    /// Test resolve_path_commands with closed=true appends Close command.
    #[test]
    fn test_resolve_path_commands_quad_closed() {
        let from_id = EntityId(101);
        let handle_id = EntityId(102);
        let to_id = EntityId(103);

        let segments = vec![PathSegment::Quad {
            from: from_id,
            handle: handle_id,
            to: to_id,
        }];

        let get_coord = |id: EntityId, component: VectorComponent| -> Option<Rational> {
            match (id, component) {
                (EntityId(101), VectorComponent::X) => Some(Rational::from_int(0)),
                (EntityId(101), VectorComponent::Y) => Some(Rational::from_int(0)),
                (EntityId(102), VectorComponent::X) => Some(Rational::from_int(50)),
                (EntityId(102), VectorComponent::Y) => Some(Rational::from_int(100)),
                (EntityId(103), VectorComponent::X) => Some(Rational::from_int(100)),
                (EntityId(103), VectorComponent::Y) => Some(Rational::from_int(0)),
                _ => None,
            }
        };

        // Resolve with closed=true
        let commands = resolve_path_commands(&segments, true, get_coord).unwrap();

        // Expected: MoveTo, QuadTo, Close
        assert_eq!(commands.len(), 3);
        assert!(matches!(commands[0], PathCommand::MoveTo { .. }));
        assert!(matches!(commands[1], PathCommand::QuadTo { .. }));
        assert!(matches!(commands[2], PathCommand::Close));
    }

    /// Test resolve_path_commands returns error for missing coordinate.
    #[test]
    fn test_resolve_path_commands_missing_coord() {
        let from_id = EntityId(101);
        let handle_id = EntityId(102);
        let to_id = EntityId(103);

        let segments = vec![PathSegment::Quad {
            from: from_id,
            handle: handle_id,
            to: to_id,
        }];

        // Return None for handle Y coordinate to simulate missing solver output
        let get_coord = |id: EntityId, component: VectorComponent| -> Option<Rational> {
            match (id, component) {
                (EntityId(101), VectorComponent::X) => Some(Rational::from_int(0)),
                (EntityId(101), VectorComponent::Y) => Some(Rational::from_int(0)),
                (EntityId(102), VectorComponent::X) => Some(Rational::from_int(50)),
                // Missing: (EntityId(102), VectorComponent::Y)
                (EntityId(103), VectorComponent::X) => Some(Rational::from_int(100)),
                (EntityId(103), VectorComponent::Y) => Some(Rational::from_int(0)),
                _ => None,
            }
        };

        let result = resolve_path_commands(&segments, false, get_coord);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, PathResolveError::MissingControlPoint { .. }));
    }
}
