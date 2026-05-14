//! Polynomial representation for algebraic geometry solver.
//!
//! A polynomial is a finite sum of terms, where each term is a coefficient
//! multiplied by a monomial.

use super::monomial::{Monomial, MonomialOrder, VarIndex};
use crate::Rational;
use std::collections::BTreeMap;
use std::fmt;

/// A polynomial with rational coefficients.
///
/// Polynomials are represented as a map from monomials to their coefficients.
/// Zero coefficients are not stored.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Polynomial {
    /// Map from monomial to coefficient.
    terms: BTreeMap<MonomialKey, Rational>,
    /// The monomial ordering used for this polynomial.
    order: MonomialOrder,
}

/// A wrapper around Monomial that implements Ord based on a fixed ordering.
/// This allows us to use BTreeMap for sorted storage.
#[derive(Clone, PartialEq, Eq, Hash)]
struct MonomialKey {
    monomial: Monomial,
}

impl PartialOrd for MonomialKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MonomialKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Use grevlex as default for internal storage
        // The actual ordering for Gröbner computation is specified separately
        MonomialOrder::GradedRevLex.compare(&self.monomial, &other.monomial)
    }
}

impl Polynomial {
    /// Create the zero polynomial.
    pub fn zero() -> Self {
        Self {
            terms: BTreeMap::new(),
            order: MonomialOrder::Lexicographic,
        }
    }

    /// Create a constant polynomial.
    pub fn constant(c: Rational) -> Self {
        if c == Rational::zero() {
            return Self::zero();
        }
        let mut terms = BTreeMap::new();
        terms.insert(
            MonomialKey {
                monomial: Monomial::one(),
            },
            c,
        );
        Self {
            terms,
            order: MonomialOrder::Lexicographic,
        }
    }

    /// Create a polynomial for a single variable.
    pub fn var(index: VarIndex) -> Self {
        let mut terms = BTreeMap::new();
        terms.insert(
            MonomialKey {
                monomial: Monomial::var(index),
            },
            Rational::from_int(1),
        );
        Self {
            terms,
            order: MonomialOrder::Lexicographic,
        }
    }

    /// Create a polynomial from a single term (coefficient * monomial).
    pub fn term(coeff: Rational, monomial: Monomial) -> Self {
        if coeff == Rational::zero() {
            return Self::zero();
        }
        let mut terms = BTreeMap::new();
        terms.insert(MonomialKey { monomial }, coeff);
        Self {
            terms,
            order: MonomialOrder::Lexicographic,
        }
    }

    /// Create a polynomial from a list of (coefficient, monomial) pairs.
    pub fn from_terms(pairs: impl IntoIterator<Item = (Rational, Monomial)>) -> Self {
        let mut poly = Self::zero();
        for (coeff, mon) in pairs {
            poly.add_term(coeff, mon);
        }
        poly
    }

    /// Set the monomial ordering for this polynomial.
    pub fn with_order(mut self, order: MonomialOrder) -> Self {
        self.order = order;
        self
    }

    /// Add a term to this polynomial.
    fn add_term(&mut self, coeff: Rational, monomial: Monomial) {
        if coeff == Rational::zero() {
            return;
        }
        let key = MonomialKey { monomial };
        let entry = self.terms.entry(key).or_insert(Rational::zero());
        *entry = entry.clone() + coeff;
        // Remove if coefficient became zero
        if *entry == Rational::zero() {
            // Need to get the key again since we modified the entry
            // This is a bit awkward but necessary
            self.terms.retain(|_, v| *v != Rational::zero());
        }
    }

    /// Check if this is the zero polynomial.
    pub fn is_zero(&self) -> bool {
        self.terms.is_empty()
    }

    /// Get the leading term according to the specified ordering.
    ///
    /// Returns (coefficient, monomial) of the largest monomial.
    pub fn leading_term(&self, order: MonomialOrder) -> Option<(Rational, Monomial)> {
        self.terms
            .iter()
            .max_by(|(k1, _), (k2, _)| order.compare(&k1.monomial, &k2.monomial))
            .map(|(k, c)| (c.clone(), k.monomial.clone()))
    }

    /// Get the leading monomial according to the specified ordering.
    pub fn leading_monomial(&self, order: MonomialOrder) -> Option<Monomial> {
        self.leading_term(order).map(|(_, m)| m)
    }

    /// Get the leading coefficient according to the specified ordering.
    pub fn leading_coefficient(&self, order: MonomialOrder) -> Option<Rational> {
        self.leading_term(order).map(|(c, _)| c)
    }

    /// Get the total degree of the polynomial.
    pub fn total_degree(&self) -> u32 {
        self.terms
            .keys()
            .map(|k| k.monomial.total_degree())
            .max()
            .unwrap_or(0)
    }

    /// Get all variables that appear in this polynomial.
    pub fn variables(&self) -> Vec<VarIndex> {
        let mut vars: Vec<_> = self
            .terms
            .keys()
            .flat_map(|k| k.monomial.variables())
            .collect();
        vars.sort();
        vars.dedup();
        vars
    }

    /// Add two polynomials.
    pub fn add(&self, other: &Polynomial) -> Polynomial {
        let mut result = self.clone();
        for (key, coeff) in &other.terms {
            result.add_term(coeff.clone(), key.monomial.clone());
        }
        result
    }

    /// Subtract two polynomials.
    pub fn sub(&self, other: &Polynomial) -> Polynomial {
        let mut result = self.clone();
        for (key, coeff) in &other.terms {
            result.add_term(Rational::zero() - coeff.clone(), key.monomial.clone());
        }
        result
    }

    /// Multiply two polynomials.
    pub fn mul(&self, other: &Polynomial) -> Polynomial {
        let mut result = Polynomial::zero();
        for (k1, c1) in &self.terms {
            for (k2, c2) in &other.terms {
                let new_coeff = c1.clone() * c2.clone();
                let new_mon = k1.monomial.multiply(&k2.monomial);
                result.add_term(new_coeff, new_mon);
            }
        }
        result
    }

    /// Multiply by a scalar.
    pub fn scale(&self, scalar: &Rational) -> Polynomial {
        if *scalar == Rational::zero() {
            return Polynomial::zero();
        }
        let mut result = self.clone();
        for coeff in result.terms.values_mut() {
            *coeff = coeff.clone() * scalar.clone();
        }
        result
    }

    /// Multiply by a monomial.
    pub fn mul_monomial(&self, monomial: &Monomial) -> Polynomial {
        let mut result = Polynomial::zero();
        for (key, coeff) in &self.terms {
            let new_mon = key.monomial.multiply(monomial);
            result.add_term(coeff.clone(), new_mon);
        }
        result
    }

    /// Negate the polynomial.
    pub fn neg(&self) -> Polynomial {
        self.scale(&Rational::from_int(-1))
    }

    /// Make the polynomial monic (leading coefficient = 1).
    pub fn make_monic(&self, order: MonomialOrder) -> Polynomial {
        if let Some(lc) = self.leading_coefficient(order) {
            if lc != Rational::zero() {
                return self.scale(&(Rational::from_int(1) / lc));
            }
        }
        self.clone()
    }

    /// Polynomial division with remainder.
    ///
    /// Given `self` and a list of divisors, computes quotients q₁, ..., qₙ
    /// and remainder r such that:
    ///   self = q₁*divisors[0] + ... + qₙ*divisors[n-1] + r
    ///
    /// where no monomial of r is divisible by any leading monomial of divisors.
    pub fn divide(
        &self,
        divisors: &[Polynomial],
        order: MonomialOrder,
    ) -> (Vec<Polynomial>, Polynomial) {
        let n = divisors.len();
        let mut quotients: Vec<Polynomial> = vec![Polynomial::zero(); n];
        let mut remainder = Polynomial::zero();
        let mut p = self.clone();

        while !p.is_zero() {
            let (lt_coeff, lt_mon) = match p.leading_term(order) {
                Some(lt) => lt,
                None => break,
            };

            let mut divided = false;
            for i in 0..n {
                if let Some((div_coeff, div_mon)) = divisors[i].leading_term(order) {
                    if lt_mon.is_divisible_by(&div_mon) {
                        // Compute quotient term
                        let quot_mon = lt_mon.divide(&div_mon).unwrap();
                        let quot_coeff = lt_coeff.clone() / div_coeff;
                        let quot_term = Polynomial::term(quot_coeff.clone(), quot_mon.clone());

                        // Update quotient
                        quotients[i] = quotients[i].add(&quot_term);

                        // Subtract from p
                        let subtrahend = divisors[i].mul(&quot_term);
                        p = p.sub(&subtrahend);

                        divided = true;
                        break;
                    }
                }
            }

            if !divided {
                // Leading term is not divisible by any divisor's leading term
                // Move it to remainder
                let lt_poly = Polynomial::term(lt_coeff, lt_mon.clone());
                remainder = remainder.add(&lt_poly);
                p = p.sub(&lt_poly);
            }
        }

        (quotients, remainder)
    }

    /// Compute the remainder when dividing by a list of polynomials.
    pub fn reduce(&self, divisors: &[Polynomial], order: MonomialOrder) -> Polynomial {
        self.divide(divisors, order).1
    }

    /// Evaluate the polynomial at a given point.
    ///
    /// The point is given as a map from variable indices to rational values.
    pub fn evaluate(&self, values: &BTreeMap<VarIndex, Rational>) -> Rational {
        let mut result = Rational::zero();
        for (key, coeff) in &self.terms {
            let mut term_value = coeff.clone();
            for var in key.monomial.variables() {
                let var_value = values.get(&var).cloned().unwrap_or(Rational::zero());
                let exp = key.monomial.exponent(var);
                for _ in 0..exp {
                    term_value = term_value * var_value.clone();
                }
            }
            result = result + term_value;
        }
        result
    }

    /// Get an iterator over all terms (coefficient, monomial).
    pub fn iter_terms(&self) -> impl Iterator<Item = (&Rational, &Monomial)> {
        self.terms.iter().map(|(k, c)| (c, &k.monomial))
    }
}

impl fmt::Debug for Polynomial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return write!(f, "0");
        }

        let order = MonomialOrder::Lexicographic;
        let mut terms: Vec<_> = self.terms.iter().collect();
        terms.sort_by(|(k1, _), (k2, _)| order.compare(&k2.monomial, &k1.monomial));

        let mut first = true;
        for (key, coeff) in terms {
            let coeff_str = format!("{:?}", coeff);
            let is_one = coeff_str == "1/1";
            let is_neg_one = coeff_str == "-1/1";
            let is_constant = key.monomial.is_constant();

            if first {
                if is_neg_one && !is_constant {
                    write!(f, "-{:?}", key.monomial)?;
                } else if is_one && !is_constant {
                    write!(f, "{:?}", key.monomial)?;
                } else if is_constant {
                    write!(f, "{}", coeff_str)?;
                } else {
                    write!(f, "{}*{:?}", coeff_str, key.monomial)?;
                }
            } else {
                // Check if coefficient is negative by parsing
                let neg = coeff_str.starts_with('-');
                if neg {
                    let abs_coeff = &coeff_str[1..];
                    if abs_coeff == "1/1" && !is_constant {
                        write!(f, " - {:?}", key.monomial)?;
                    } else if is_constant {
                        write!(f, " - {}", abs_coeff)?;
                    } else {
                        write!(f, " - {}*{:?}", abs_coeff, key.monomial)?;
                    }
                } else {
                    if is_one && !is_constant {
                        write!(f, " + {:?}", key.monomial)?;
                    } else if is_constant {
                        write!(f, " + {}", coeff_str)?;
                    } else {
                        write!(f, " + {}*{:?}", coeff_str, key.monomial)?;
                    }
                }
            }
            first = false;
        }
        Ok(())
    }
}

impl fmt::Display for Polynomial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polynomial_creation() {
        let x = Polynomial::var(0);
        assert!(!x.is_zero());
        assert_eq!(x.total_degree(), 1);

        let c = Polynomial::constant(Rational::from_int(5));
        assert_eq!(c.total_degree(), 0);
    }

    #[test]
    fn test_polynomial_add() {
        let x = Polynomial::var(0);
        let y = Polynomial::var(1);
        let sum = x.add(&y);

        assert_eq!(sum.total_degree(), 1);
        let vars = sum.variables();
        assert!(vars.contains(&0));
        assert!(vars.contains(&1));
    }

    #[test]
    fn test_polynomial_mul() {
        let x = Polynomial::var(0);
        let y = Polynomial::var(1);

        // (x + y)²  = x² + 2xy + y²
        let sum = x.add(&y);
        let squared = sum.mul(&sum);

        assert_eq!(squared.total_degree(), 2);
    }

    #[test]
    fn test_polynomial_division() {
        // Divide x² - 1 by x - 1
        // Should give quotient x + 1, remainder 0
        let x = Polynomial::var(0);
        let one = Polynomial::constant(Rational::from_int(1));
        let neg_one = Polynomial::constant(Rational::from_int(-1));

        let x2_minus_1 = x.mul(&x).add(&neg_one);
        let x_minus_1 = x.add(&neg_one);

        let (quotients, remainder) = x2_minus_1.divide(&[x_minus_1], MonomialOrder::Lexicographic);

        assert!(
            remainder.is_zero(),
            "Remainder should be zero: {:?}",
            remainder
        );
        // quotient should be x + 1
        let expected_quotient = x.add(&one);
        assert_eq!(quotients[0], expected_quotient);
    }

    #[test]
    fn test_polynomial_evaluate() {
        // f(x, y) = x² + xy + y
        let x = Polynomial::var(0);
        let y = Polynomial::var(1);
        let f = x.mul(&x).add(&x.mul(&y)).add(&y);

        // Evaluate at x=2, y=3
        let mut values = BTreeMap::new();
        values.insert(0, Rational::from_int(2));
        values.insert(1, Rational::from_int(3));

        let result = f.evaluate(&values);
        // 2² + 2*3 + 3 = 4 + 6 + 3 = 13
        assert_eq!(result, Rational::from_int(13));
    }
}
