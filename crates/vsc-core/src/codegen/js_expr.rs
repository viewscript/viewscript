//! JavaScript Expression Generation (C1)
//!
//! Converts `ConstraintTerm` to JavaScript arithmetic expressions.
//!
//! ## Precision Handling
//!
//! Rational values that cannot be exactly represented in f64 are output
//! as fraction expressions `(numer / denom)` to preserve precision.
//! This ensures the Rasterization Boundary precision guarantees extend
//! to compiled output.
//!
//! ## Examples
//!
//! ```text
//! Const { 100 }                    → "100"
//! Const { 1/3 }                    → "(1 / 3)"
//! Ref { bg, X }                    → "bg_x"
//! Linear { 1, bg, X, 0 }           → "bg_x"
//! Linear { 2, bg, X, 10 }          → "2 * bg_x + 10"
//! LinearCombination { [a, b], 0 }  → "a + b"
//! LinearCombination { [2*a, -b], 5 } → "2 * a + -b + 5"
//! ```

use crate::{ConstraintTerm, EntityId, LinearFactor, Rational, VectorComponent};
use std::collections::HashMap;

// =============================================================================
// Rational to JavaScript Conversion
// =============================================================================

/// Convert a Rational number to a JavaScript expression string.
///
/// ## Precision Preservation
///
/// - If denominator is 1: output integer literal
/// - If f64 round-trip is exact: output f64 literal
/// - Otherwise: output fraction expression `(numer / denom)`
///
/// ## Examples
///
/// ```
/// use vsc_core::Rational;
/// use vsc_core::codegen::rational_to_js;
///
/// assert_eq!(rational_to_js(&Rational::from_int(100)), "100");
/// assert_eq!(rational_to_js(&Rational::new(1, 2)), "0.5");
/// assert_eq!(rational_to_js(&Rational::new(1, 3)), "(1 / 3)");
/// ```
pub fn rational_to_js(r: &Rational) -> String {
    // Case 1: Integer (denominator is 1)
    if r.is_integer() {
        return format!("{}", r.numer());
    }

    // Case 2: Check if f64 round-trip preserves value
    // A fraction n/d is exactly representable in f64 if d is a power of 2
    // and the resulting float doesn't lose precision
    let f = r.to_f64_for_rasterization();

    // Check if f64 representation is exact by verifying the denominator is a power of 2
    // and the numerator fits in f64 mantissa
    if is_exact_f64_representable(r) {
        return format_f64_literal(f);
    }

    // Case 3: Fraction expression required for precision
    format!("({} / {})", r.numer(), r.denom())
}

/// Check if a rational can be exactly represented as f64.
/// A fraction is exactly representable if:
/// 1. Denominator is a power of 2
/// 2. Both numerator and denominator fit within f64 mantissa precision
fn is_exact_f64_representable(r: &Rational) -> bool {
    use num_traits::ToPrimitive;

    let denom = r.denom();

    // Check if denominator is a power of 2
    // For BigInt, we check if it equals 2^k for some k
    if denom.to_i64().is_none() {
        return false; // Too large
    }
    let d = denom.to_i64().unwrap();
    if d <= 0 || (d & (d - 1)) != 0 {
        return false; // Not a power of 2
    }

    // Check numerator fits in f64 mantissa (53 bits)
    let numer = r.numer();
    if let Some(n) = numer.to_i64() {
        // Both fit in i64, and denom is power of 2 → exactly representable
        let max_mantissa = 1i64 << 53;
        n.abs() <= max_mantissa
    } else {
        false
    }
}

/// Format f64 as JavaScript literal, avoiding unnecessary decimals.
fn format_f64_literal(f: f64) -> String {
    if f.fract() == 0.0 {
        // Integer-valued f64
        format!("{}", f as i64)
    } else {
        // Use full precision
        format!("{}", f)
    }
}

// =============================================================================
// ConstraintTerm to JavaScript Expression
// =============================================================================

/// Variable name mapping: (EntityId, Component) → JS variable name.
pub type VarNameMap = HashMap<(EntityId, VectorComponent), String>;

/// Convert a ConstraintTerm to a JavaScript arithmetic expression.
///
/// ## Arguments
///
/// * `term` - The constraint term to convert
/// * `var_name_map` - Mapping from (EntityId, Component) to JS variable names
///
/// ## Returns
///
/// A JavaScript expression string that computes the term's value.
///
/// ## Simplification Rules
///
/// - `coefficient == 1` with no offset → just variable name
/// - `coefficient == -1` → `-varName`
/// - `offset == 0` → omit offset
/// - Empty LinearCombination → offset as constant
pub fn term_to_js_expr(term: &ConstraintTerm, var_name_map: &VarNameMap) -> String {
    match term {
        ConstraintTerm::Const { value } => rational_to_js(value),

        ConstraintTerm::Ref { entity_id, component } => {
            var_name_map
                .get(&(*entity_id, *component))
                .cloned()
                .unwrap_or_else(|| format!("/* unknown {:?}.{:?} */", entity_id, component))
        }

        ConstraintTerm::Linear {
            coefficient,
            entity_id,
            component,
            offset,
        } => {
            let var_name = var_name_map
                .get(&(*entity_id, *component))
                .cloned()
                .unwrap_or_else(|| format!("/* unknown {:?}.{:?} */", entity_id, component));

            format_linear_term(&var_name, coefficient, offset)
        }

        ConstraintTerm::LinearCombination { terms, offset } => {
            format_linear_combination(terms, offset, var_name_map)
        }
    }
}

/// Format a single linear term: coefficient * var + offset
fn format_linear_term(var_name: &str, coefficient: &Rational, offset: &Rational) -> String {
    let coef_is_one = *coefficient == Rational::from_int(1);
    let coef_is_neg_one = *coefficient == Rational::from_int(-1);
    let offset_is_zero = *offset == Rational::zero();

    match (coef_is_one, coef_is_neg_one, offset_is_zero) {
        // coefficient=1, offset=0 → varName
        (true, _, true) => var_name.to_string(),

        // coefficient=1, offset≠0 → varName + offset
        (true, _, false) => {
            if offset.is_negative() {
                format!("{} - {}", var_name, rational_to_js(&offset.abs()))
            } else {
                format!("{} + {}", var_name, rational_to_js(offset))
            }
        }

        // coefficient=-1, offset=0 → -varName
        (_, true, true) => format!("-{}", var_name),

        // coefficient=-1, offset≠0 → -varName + offset
        (_, true, false) => {
            if offset.is_negative() {
                format!("-{} - {}", var_name, rational_to_js(&offset.abs()))
            } else {
                format!("-{} + {}", var_name, rational_to_js(offset))
            }
        }

        // General case: coef * varName + offset
        (false, false, true) => {
            format!("{} * {}", rational_to_js(coefficient), var_name)
        }

        (false, false, false) => {
            let coef_str = rational_to_js(coefficient);
            if offset.is_negative() {
                format!(
                    "{} * {} - {}",
                    coef_str,
                    var_name,
                    rational_to_js(&offset.abs())
                )
            } else {
                format!("{} * {} + {}", coef_str, var_name, rational_to_js(offset))
            }
        }
    }
}

/// Format a linear combination: Σ(coefficient_i * var_i) + offset
fn format_linear_combination(
    terms: &[LinearFactor],
    offset: &Rational,
    var_name_map: &VarNameMap,
) -> String {
    // Empty terms → just offset (constant)
    if terms.is_empty() {
        return rational_to_js(offset);
    }

    let mut parts: Vec<String> = Vec::with_capacity(terms.len() + 1);

    for (i, factor) in terms.iter().enumerate() {
        let var_name = var_name_map
            .get(&(factor.entity_id, factor.component))
            .cloned()
            .unwrap_or_else(|| {
                format!(
                    "/* unknown {:?}.{:?} */",
                    factor.entity_id, factor.component
                )
            });

        let coef = &factor.coefficient;
        let coef_is_one = *coef == Rational::from_int(1);
        let coef_is_neg_one = *coef == Rational::from_int(-1);

        // First term: output directly (include sign if negative)
        if i == 0 {
            if coef_is_one {
                parts.push(var_name);
            } else if coef_is_neg_one {
                parts.push(format!("-{}", var_name));
            } else {
                parts.push(format!("{} * {}", rational_to_js(coef), var_name));
            }
        } else if coef.is_negative() {
            // Subsequent negative coefficient: output as " - abs(coef) * var"
            if coef_is_neg_one {
                parts.push(format!(" - {}", var_name));
            } else {
                let abs_coef = coef.abs();
                if abs_coef == Rational::from_int(1) {
                    parts.push(format!(" - {}", var_name));
                } else {
                    parts.push(format!(" - {} * {}", rational_to_js(&abs_coef), var_name));
                }
            }
        } else {
            // Subsequent positive coefficient
            if coef_is_one {
                parts.push(format!(" + {}", var_name));
            } else {
                parts.push(format!(" + {} * {}", rational_to_js(coef), var_name));
            }
        }
    }

    // Add offset if non-zero
    if *offset != Rational::zero() {
        if offset.is_negative() {
            parts.push(format!(" - {}", rational_to_js(&offset.abs())));
        } else {
            parts.push(format!(" + {}", rational_to_js(offset)));
        }
    }

    parts.concat()
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // rational_to_js Tests
    // =========================================================================

    #[test]
    fn test_rational_to_js_integer() {
        assert_eq!(rational_to_js(&Rational::from_int(100)), "100");
        assert_eq!(rational_to_js(&Rational::from_int(0)), "0");
        assert_eq!(rational_to_js(&Rational::from_int(-42)), "-42");
    }

    #[test]
    fn test_rational_to_js_exact_fraction() {
        // 1/2 = 0.5 (exact in f64)
        assert_eq!(rational_to_js(&Rational::new(1, 2)), "0.5");
        // 1/4 = 0.25 (exact in f64)
        assert_eq!(rational_to_js(&Rational::new(1, 4)), "0.25");
        // 3/4 = 0.75 (exact in f64)
        assert_eq!(rational_to_js(&Rational::new(3, 4)), "0.75");
    }

    #[test]
    fn test_rational_to_js_inexact_fraction() {
        // 1/3 cannot be exactly represented in f64
        assert_eq!(rational_to_js(&Rational::new(1, 3)), "(1 / 3)");
        // 1/7 cannot be exactly represented in f64
        assert_eq!(rational_to_js(&Rational::new(1, 7)), "(1 / 7)");
        // 2/3 cannot be exactly represented
        assert_eq!(rational_to_js(&Rational::new(2, 3)), "(2 / 3)");
    }

    #[test]
    fn test_rational_to_js_negative() {
        assert_eq!(rational_to_js(&Rational::new(-1, 2)), "-0.5");
        assert_eq!(rational_to_js(&Rational::new(-1, 3)), "(-1 / 3)");
    }

    // =========================================================================
    // term_to_js_expr: Const Tests
    // =========================================================================

    #[test]
    fn test_const_integer() {
        let term = ConstraintTerm::Const {
            value: Rational::from_int(100),
        };
        let result = term_to_js_expr(&term, &HashMap::new());
        assert_eq!(result, "100");
    }

    #[test]
    fn test_const_fraction_inexact() {
        let term = ConstraintTerm::Const {
            value: Rational::new(1, 3),
        };
        let result = term_to_js_expr(&term, &HashMap::new());
        assert_eq!(result, "(1 / 3)");
    }

    // =========================================================================
    // term_to_js_expr: Ref Tests
    // =========================================================================

    #[test]
    fn test_ref_simple() {
        let term = ConstraintTerm::Ref {
            entity_id: EntityId(1),
            component: VectorComponent::X,
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "bg_x".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "bg_x");
    }

    #[test]
    fn test_ref_unknown() {
        let term = ConstraintTerm::Ref {
            entity_id: EntityId(999),
            component: VectorComponent::Y,
        };

        let result = term_to_js_expr(&term, &HashMap::new());
        assert!(result.contains("unknown"));
    }

    // =========================================================================
    // term_to_js_expr: Linear Tests
    // =========================================================================

    #[test]
    fn test_linear_identity() {
        // coefficient=1, offset=0 → just variable name
        let term = ConstraintTerm::Linear {
            coefficient: Rational::from_int(1),
            entity_id: EntityId(1),
            component: VectorComponent::X,
            offset: Rational::zero(),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "bg_x".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "bg_x");
    }

    #[test]
    fn test_linear_with_coefficient_and_offset() {
        // 2 * bg_x + 10
        let term = ConstraintTerm::Linear {
            coefficient: Rational::from_int(2),
            entity_id: EntityId(1),
            component: VectorComponent::X,
            offset: Rational::from_int(10),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "bg_x".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "2 * bg_x + 10");
    }

    #[test]
    fn test_linear_negative_coefficient() {
        // -bg_x
        let term = ConstraintTerm::Linear {
            coefficient: Rational::from_int(-1),
            entity_id: EntityId(1),
            component: VectorComponent::X,
            offset: Rational::zero(),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "bg_x".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "-bg_x");
    }

    #[test]
    fn test_linear_coefficient_only() {
        // 3 * bg_x (offset=0)
        let term = ConstraintTerm::Linear {
            coefficient: Rational::from_int(3),
            entity_id: EntityId(1),
            component: VectorComponent::X,
            offset: Rational::zero(),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "bg_x".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "3 * bg_x");
    }

    #[test]
    fn test_linear_offset_only() {
        // bg_x + 5 (coefficient=1)
        let term = ConstraintTerm::Linear {
            coefficient: Rational::from_int(1),
            entity_id: EntityId(1),
            component: VectorComponent::X,
            offset: Rational::from_int(5),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "bg_x".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "bg_x + 5");
    }

    #[test]
    fn test_linear_negative_offset() {
        // bg_x - 10
        let term = ConstraintTerm::Linear {
            coefficient: Rational::from_int(1),
            entity_id: EntityId(1),
            component: VectorComponent::X,
            offset: Rational::from_int(-10),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "bg_x".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "bg_x - 10");
    }

    // =========================================================================
    // term_to_js_expr: LinearCombination Tests
    // =========================================================================

    #[test]
    fn test_linear_combination_two_terms() {
        // a + b
        let term = ConstraintTerm::LinearCombination {
            terms: vec![
                LinearFactor {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(1),
                    component: VectorComponent::X,
                },
                LinearFactor {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(2),
                    component: VectorComponent::X,
                },
            ],
            offset: Rational::zero(),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "a".to_string());
        var_map.insert((EntityId(2), VectorComponent::X), "b".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "a + b");
    }

    #[test]
    fn test_linear_combination_mixed_coefficients() {
        // 2 * a - b + 5
        let term = ConstraintTerm::LinearCombination {
            terms: vec![
                LinearFactor {
                    coefficient: Rational::from_int(2),
                    entity_id: EntityId(1),
                    component: VectorComponent::X,
                },
                LinearFactor {
                    coefficient: Rational::from_int(-1),
                    entity_id: EntityId(2),
                    component: VectorComponent::X,
                },
            ],
            offset: Rational::from_int(5),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "a".to_string());
        var_map.insert((EntityId(2), VectorComponent::X), "b".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "2 * a - b + 5");
    }

    #[test]
    fn test_linear_combination_empty_offset_only() {
        // Empty terms → just offset (constant)
        let term = ConstraintTerm::LinearCombination {
            terms: vec![],
            offset: Rational::from_int(42),
        };

        let result = term_to_js_expr(&term, &HashMap::new());
        assert_eq!(result, "42");
    }

    #[test]
    fn test_linear_combination_three_terms() {
        // x + y + z
        let term = ConstraintTerm::LinearCombination {
            terms: vec![
                LinearFactor {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(1),
                    component: VectorComponent::X,
                },
                LinearFactor {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(2),
                    component: VectorComponent::Y,
                },
                LinearFactor {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(3),
                    component: VectorComponent::Z,
                },
            ],
            offset: Rational::zero(),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "x".to_string());
        var_map.insert((EntityId(2), VectorComponent::Y), "y".to_string());
        var_map.insert((EntityId(3), VectorComponent::Z), "z".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "x + y + z");
    }

    #[test]
    fn test_linear_combination_negative_offset() {
        // a + b - 10
        let term = ConstraintTerm::LinearCombination {
            terms: vec![
                LinearFactor {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(1),
                    component: VectorComponent::X,
                },
                LinearFactor {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(2),
                    component: VectorComponent::X,
                },
            ],
            offset: Rational::from_int(-10),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "a".to_string());
        var_map.insert((EntityId(2), VectorComponent::X), "b".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "a + b - 10");
    }

    #[test]
    fn test_linear_combination_fractional_coefficient() {
        // 0.5 * a + b (where 0.5 is exact)
        let term = ConstraintTerm::LinearCombination {
            terms: vec![
                LinearFactor {
                    coefficient: Rational::new(1, 2),
                    entity_id: EntityId(1),
                    component: VectorComponent::X,
                },
                LinearFactor {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(2),
                    component: VectorComponent::X,
                },
            ],
            offset: Rational::zero(),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "a".to_string());
        var_map.insert((EntityId(2), VectorComponent::X), "b".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "0.5 * a + b");
    }

    #[test]
    fn test_linear_combination_inexact_fraction() {
        // (1/3) * a + b
        let term = ConstraintTerm::LinearCombination {
            terms: vec![
                LinearFactor {
                    coefficient: Rational::new(1, 3),
                    entity_id: EntityId(1),
                    component: VectorComponent::X,
                },
                LinearFactor {
                    coefficient: Rational::from_int(1),
                    entity_id: EntityId(2),
                    component: VectorComponent::X,
                },
            ],
            offset: Rational::zero(),
        };

        let mut var_map = HashMap::new();
        var_map.insert((EntityId(1), VectorComponent::X), "a".to_string());
        var_map.insert((EntityId(2), VectorComponent::X), "b".to_string());

        let result = term_to_js_expr(&term, &var_map);
        assert_eq!(result, "(1 / 3) * a + b");
    }
}
