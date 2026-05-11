//! Monomial representation for polynomial algebra.
//!
//! A monomial is a product of variables raised to non-negative integer powers.
//! For example: x²y³z represents the monomial with exponents [2, 3, 1].

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

/// A variable identifier in the polynomial ring.
///
/// Variables are identified by a unique index. For P-dimension constraints,
/// this maps to (EntityId, VectorComponent) pairs.
pub type VarIndex = u32;

/// A monomial represented as a product of variables with exponents.
///
/// The monomial x₁^a₁ * x₂^a₂ * ... * xₙ^aₙ is stored as a map from
/// variable indices to their exponents.
///
/// ## Example
/// ```text
/// x²y³ = { 0: 2, 1: 3 }
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Monomial {
    /// Map from variable index to its exponent.
    /// Variables with exponent 0 are not stored.
    exponents: BTreeMap<VarIndex, u32>,
}

impl Monomial {
    /// Create a constant monomial (degree 0).
    pub fn one() -> Self {
        Self {
            exponents: BTreeMap::new(),
        }
    }

    /// Create a monomial for a single variable with exponent 1.
    pub fn var(index: VarIndex) -> Self {
        let mut exponents = BTreeMap::new();
        exponents.insert(index, 1);
        Self { exponents }
    }

    /// Create a monomial for a single variable with the given exponent.
    pub fn var_pow(index: VarIndex, exp: u32) -> Self {
        if exp == 0 {
            return Self::one();
        }
        let mut exponents = BTreeMap::new();
        exponents.insert(index, exp);
        Self { exponents }
    }

    /// Create a monomial from a list of (variable, exponent) pairs.
    pub fn from_exponents(pairs: impl IntoIterator<Item = (VarIndex, u32)>) -> Self {
        let exponents: BTreeMap<_, _> = pairs
            .into_iter()
            .filter(|(_, exp)| *exp > 0)
            .collect();
        Self { exponents }
    }

    /// Get the exponent of a variable.
    pub fn exponent(&self, var: VarIndex) -> u32 {
        *self.exponents.get(&var).unwrap_or(&0)
    }

    /// Get all variable indices in this monomial.
    pub fn variables(&self) -> impl Iterator<Item = VarIndex> + '_ {
        self.exponents.keys().copied()
    }

    /// Get the total degree (sum of all exponents).
    pub fn total_degree(&self) -> u32 {
        self.exponents.values().sum()
    }

    /// Check if this is the constant monomial (degree 0).
    pub fn is_constant(&self) -> bool {
        self.exponents.is_empty()
    }

    /// Multiply two monomials.
    pub fn multiply(&self, other: &Monomial) -> Monomial {
        let mut result = self.exponents.clone();
        for (&var, &exp) in &other.exponents {
            *result.entry(var).or_insert(0) += exp;
        }
        Monomial { exponents: result }
    }

    /// Divide this monomial by another, if divisible.
    ///
    /// Returns `Some(quotient)` if `self` is divisible by `other`,
    /// `None` otherwise.
    pub fn divide(&self, other: &Monomial) -> Option<Monomial> {
        let mut result = self.exponents.clone();
        for (&var, &exp) in &other.exponents {
            let self_exp = result.get(&var).copied().unwrap_or(0);
            if self_exp < exp {
                return None; // Not divisible
            }
            if self_exp == exp {
                result.remove(&var);
            } else {
                result.insert(var, self_exp - exp);
            }
        }
        Some(Monomial { exponents: result })
    }

    /// Check if this monomial is divisible by another.
    pub fn is_divisible_by(&self, other: &Monomial) -> bool {
        for (&var, &exp) in &other.exponents {
            if self.exponent(var) < exp {
                return false;
            }
        }
        true
    }

    /// Compute the least common multiple of two monomials.
    pub fn lcm(&self, other: &Monomial) -> Monomial {
        let mut result = self.exponents.clone();
        for (&var, &exp) in &other.exponents {
            let entry = result.entry(var).or_insert(0);
            *entry = (*entry).max(exp);
        }
        Monomial { exponents: result }
    }

    /// Compute the greatest common divisor of two monomials.
    pub fn gcd(&self, other: &Monomial) -> Monomial {
        let mut result = BTreeMap::new();
        for (&var, &exp) in &self.exponents {
            let other_exp = other.exponent(var);
            if other_exp > 0 {
                result.insert(var, exp.min(other_exp));
            }
        }
        Monomial { exponents: result }
    }

    /// Get the exponents as a vector for comparison.
    /// Uses a canonical ordering of variables.
    fn exponent_vec(&self, max_var: VarIndex) -> Vec<u32> {
        (0..=max_var).map(|i| self.exponent(i)).collect()
    }
}

impl fmt::Debug for Monomial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_constant() {
            write!(f, "1")
        } else {
            let mut first = true;
            for (&var, &exp) in &self.exponents {
                if !first {
                    write!(f, "*")?;
                }
                first = false;
                if exp == 1 {
                    write!(f, "x{}", var)?;
                } else {
                    write!(f, "x{}^{}", var, exp)?;
                }
            }
            Ok(())
        }
    }
}

impl fmt::Display for Monomial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Monomial ordering for Gröbner basis computation.
///
/// Different orderings lead to different Gröbner bases with different
/// properties. Lexicographic order is used for variable elimination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MonomialOrder {
    #[default]
    /// Lexicographic order (lex).
    /// Compares exponents from the first variable to the last.
    /// Good for variable elimination.
    Lexicographic,
    /// Graded lexicographic order (grlex).
    /// First compares total degree, then lexicographic.
    GradedLex,
    /// Graded reverse lexicographic order (grevlex).
    /// First compares total degree, then reverse lexicographic.
    /// Often fastest for computation.
    GradedRevLex,
}

impl MonomialOrder {
    /// Compare two monomials according to this ordering.
    pub fn compare(&self, a: &Monomial, b: &Monomial) -> Ordering {
        match self {
            MonomialOrder::Lexicographic => self.compare_lex(a, b),
            MonomialOrder::GradedLex => self.compare_grlex(a, b),
            MonomialOrder::GradedRevLex => self.compare_grevlex(a, b),
        }
    }

    fn compare_lex(&self, a: &Monomial, b: &Monomial) -> Ordering {
        // Find max variable index
        let max_var = a
            .variables()
            .chain(b.variables())
            .max()
            .unwrap_or(0);

        // Compare exponents from first to last variable
        for var in 0..=max_var {
            match a.exponent(var).cmp(&b.exponent(var)) {
                Ordering::Equal => continue,
                other => return other,
            }
        }
        Ordering::Equal
    }

    fn compare_grlex(&self, a: &Monomial, b: &Monomial) -> Ordering {
        // First compare total degree
        match a.total_degree().cmp(&b.total_degree()) {
            Ordering::Equal => self.compare_lex(a, b),
            other => other,
        }
    }

    fn compare_grevlex(&self, a: &Monomial, b: &Monomial) -> Ordering {
        // First compare total degree
        match a.total_degree().cmp(&b.total_degree()) {
            Ordering::Equal => {
                // Then reverse lexicographic (compare from last to first, reversed)
                let max_var = a
                    .variables()
                    .chain(b.variables())
                    .max()
                    .unwrap_or(0);

                for var in (0..=max_var).rev() {
                    match a.exponent(var).cmp(&b.exponent(var)) {
                        Ordering::Equal => continue,
                        // Note: reversed comparison for grevlex
                        Ordering::Less => return Ordering::Greater,
                        Ordering::Greater => return Ordering::Less,
                    }
                }
                Ordering::Equal
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monomial_creation() {
        let m = Monomial::var(0);
        assert_eq!(m.exponent(0), 1);
        assert_eq!(m.total_degree(), 1);

        let m2 = Monomial::var_pow(1, 3);
        assert_eq!(m2.exponent(1), 3);
        assert_eq!(m2.total_degree(), 3);
    }

    #[test]
    fn test_monomial_multiply() {
        let x = Monomial::var(0);
        let y = Monomial::var(1);
        let xy = x.multiply(&y);

        assert_eq!(xy.exponent(0), 1);
        assert_eq!(xy.exponent(1), 1);
        assert_eq!(xy.total_degree(), 2);

        let x2y = xy.multiply(&x);
        assert_eq!(x2y.exponent(0), 2);
        assert_eq!(x2y.exponent(1), 1);
    }

    #[test]
    fn test_monomial_divide() {
        let x2y = Monomial::from_exponents([(0, 2), (1, 1)]);
        let x = Monomial::var(0);

        let result = x2y.divide(&x);
        assert!(result.is_some());
        let xy = result.unwrap();
        assert_eq!(xy.exponent(0), 1);
        assert_eq!(xy.exponent(1), 1);

        // Cannot divide xy by y²
        let y2 = Monomial::var_pow(1, 2);
        assert!(xy.divide(&y2).is_none());
    }

    #[test]
    fn test_monomial_lcm_gcd() {
        let x2y = Monomial::from_exponents([(0, 2), (1, 1)]);
        let xy2 = Monomial::from_exponents([(0, 1), (1, 2)]);

        let lcm = x2y.lcm(&xy2);
        assert_eq!(lcm.exponent(0), 2);
        assert_eq!(lcm.exponent(1), 2);

        let gcd = x2y.gcd(&xy2);
        assert_eq!(gcd.exponent(0), 1);
        assert_eq!(gcd.exponent(1), 1);
    }

    #[test]
    fn test_lex_order() {
        let order = MonomialOrder::Lexicographic;

        let x2 = Monomial::var_pow(0, 2);
        let xy = Monomial::from_exponents([(0, 1), (1, 1)]);
        let y3 = Monomial::var_pow(1, 3);

        // x² > xy > y³ in lex order
        assert_eq!(order.compare(&x2, &xy), Ordering::Greater);
        assert_eq!(order.compare(&xy, &y3), Ordering::Greater);
        assert_eq!(order.compare(&x2, &y3), Ordering::Greater);
    }

    #[test]
    fn test_grlex_order() {
        let order = MonomialOrder::GradedLex;

        let x2 = Monomial::var_pow(0, 2);
        let xy = Monomial::from_exponents([(0, 1), (1, 1)]);
        let y3 = Monomial::var_pow(1, 3);

        // y³ > x² = xy in total degree
        // x² > xy in lex
        assert_eq!(order.compare(&y3, &x2), Ordering::Greater);
        assert_eq!(order.compare(&x2, &xy), Ordering::Greater);
    }
}
