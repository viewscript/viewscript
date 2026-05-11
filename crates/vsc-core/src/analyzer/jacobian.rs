//! Jacobian rank analysis for detecting structural singularities.
//!
//! This module computes the Jacobian matrix of a constraint system and
//! analyzes its rank to detect singularities and degenerate configurations.
//!
//! # Jacobian Matrix
//!
//! For a system of m constraints in n variables, the Jacobian is an m×n matrix
//! where entry (i,j) = ∂fᵢ/∂xⱼ (partial derivative of constraint i w.r.t. variable j).
//!
//! # Singularity Detection
//!
//! - Full rank (= min(m, n)): System is well-conditioned
//! - Rank deficient: System has structural singularities
//!
//! Rank deficiency indicates configurations where:
//! - Constraints become linearly dependent
//! - Small changes in inputs cause large changes in outputs
//! - The system may have infinitely many solutions

use crate::Rational;
use std::collections::HashMap;

/// Variable identifier in the Jacobian.
pub type JacobianVarId = u64;

/// Constraint identifier.
pub type ConstraintId = u64;

/// A term in a polynomial constraint (for computing derivatives).
#[derive(Clone, Debug)]
pub struct JacobianTerm {
    /// Coefficient of this term.
    pub coefficient: Rational,
    /// Variables and their exponents in this term.
    /// For linear constraints, all exponents are 1.
    pub variables: Vec<(JacobianVarId, u32)>,
}

/// A polynomial constraint for Jacobian computation.
#[derive(Clone, Debug)]
pub struct PolynomialConstraint {
    /// Unique identifier for this constraint.
    pub id: ConstraintId,
    /// Terms in the polynomial (sum of terms = 0).
    pub terms: Vec<JacobianTerm>,
}

impl PolynomialConstraint {
    /// Compute the partial derivative of this constraint with respect to a variable.
    ///
    /// Returns None if the variable doesn't appear in the constraint.
    pub fn partial_derivative(&self, var: JacobianVarId) -> Option<PartialDerivative> {
        let mut derivative_terms = Vec::new();

        for term in &self.terms {
            // Find if this variable appears in the term
            let var_info = term.variables.iter().find(|(v, _)| *v == var);

            if let Some(&(_, exp)) = var_info {
                if exp == 0 {
                    continue;
                }

                // d/dx (c * x^n * other_vars) = c * n * x^(n-1) * other_vars
                let new_coeff = term.coefficient.clone() * Rational::from_int(exp as i64);

                let new_vars: Vec<_> = term
                    .variables
                    .iter()
                    .map(|&(v, e)| {
                        if v == var {
                            (v, e - 1)
                        } else {
                            (v, e)
                        }
                    })
                    .filter(|(_, e)| *e > 0)
                    .collect();

                derivative_terms.push(DerivativeTerm {
                    coefficient: new_coeff,
                    variables: new_vars,
                });
            }
        }

        if derivative_terms.is_empty() {
            None
        } else {
            Some(PartialDerivative {
                terms: derivative_terms,
            })
        }
    }
}

/// A term in a partial derivative.
#[derive(Clone, Debug)]
pub struct DerivativeTerm {
    /// Coefficient.
    pub coefficient: Rational,
    /// Remaining variables after differentiation.
    pub variables: Vec<(JacobianVarId, u32)>,
}

/// The partial derivative of a constraint.
#[derive(Clone, Debug)]
pub struct PartialDerivative {
    /// Terms in the derivative.
    pub terms: Vec<DerivativeTerm>,
}

impl PartialDerivative {
    /// Evaluate the derivative at a given point.
    pub fn evaluate(&self, values: &HashMap<JacobianVarId, Rational>) -> Rational {
        let mut result = Rational::zero();

        for term in &self.terms {
            let mut term_value = term.coefficient.clone();

            for &(var, exp) in &term.variables {
                let var_value = values.get(&var).cloned().unwrap_or(Rational::zero());
                for _ in 0..exp {
                    term_value = term_value * var_value.clone();
                }
            }

            result = result + term_value;
        }

        result
    }

    /// Check if this derivative is constant (no variables).
    pub fn is_constant(&self) -> bool {
        self.terms.iter().all(|t| t.variables.is_empty())
    }

    /// Get the constant value if this is a constant derivative.
    pub fn constant_value(&self) -> Option<Rational> {
        if self.is_constant() {
            Some(self.terms.iter().fold(Rational::zero(), |acc, t| {
                acc + t.coefficient.clone()
            }))
        } else {
            None
        }
    }
}

/// The Jacobian matrix of a constraint system.
#[derive(Clone, Debug)]
pub struct JacobianMatrix {
    /// Number of constraints (rows).
    pub rows: usize,
    /// Number of variables (columns).
    pub cols: usize,
    /// Variable IDs in column order.
    pub variables: Vec<JacobianVarId>,
    /// Constraint IDs in row order.
    pub constraints: Vec<ConstraintId>,
    /// Partial derivatives: [row][col] -> derivative.
    pub entries: Vec<Vec<Option<PartialDerivative>>>,
}

impl JacobianMatrix {
    /// Evaluate the Jacobian at a specific point.
    ///
    /// Returns a matrix of Rational values.
    pub fn evaluate(&self, values: &HashMap<JacobianVarId, Rational>) -> Vec<Vec<Rational>> {
        self.entries
            .iter()
            .map(|row| {
                row.iter()
                    .map(|entry| {
                        entry
                            .as_ref()
                            .map(|d| d.evaluate(values))
                            .unwrap_or(Rational::zero())
                    })
                    .collect()
            })
            .collect()
    }
}

/// Result of singularity analysis.
#[derive(Clone, Debug)]
pub struct SingularityAnalysis {
    /// The rank of the Jacobian matrix.
    pub rank: usize,

    /// Maximum possible rank (min of rows and cols).
    pub max_rank: usize,

    /// Whether the system is singular (rank < max_rank).
    pub is_singular: bool,

    /// Number of constraints that are redundant.
    pub redundant_constraints: usize,

    /// Indices of potentially problematic constraints.
    pub problematic_constraint_indices: Vec<usize>,

    /// The constraint IDs corresponding to problematic constraints.
    pub problematic_constraint_ids: Vec<ConstraintId>,
}

/// Compute the Jacobian matrix for a set of polynomial constraints.
pub fn compute_jacobian(constraints: &[PolynomialConstraint]) -> JacobianMatrix {
    // Collect all variables
    let mut all_vars: Vec<JacobianVarId> = constraints
        .iter()
        .flat_map(|c| c.terms.iter().flat_map(|t| t.variables.iter().map(|(v, _)| *v)))
        .collect();
    all_vars.sort();
    all_vars.dedup();

    let rows = constraints.len();
    let cols = all_vars.len();

    let entries: Vec<Vec<Option<PartialDerivative>>> = constraints
        .iter()
        .map(|constraint| {
            all_vars
                .iter()
                .map(|&var| constraint.partial_derivative(var))
                .collect()
        })
        .collect();

    let constraint_ids: Vec<_> = constraints.iter().map(|c| c.id).collect();

    JacobianMatrix {
        rows,
        cols,
        variables: all_vars,
        constraints: constraint_ids,
        entries,
    }
}

/// Check for singularities in the constraint system at a given configuration.
///
/// Uses Gaussian elimination with partial pivoting to compute the rank.
pub fn check_singularities(
    jacobian: &JacobianMatrix,
    values: &HashMap<JacobianVarId, Rational>,
) -> SingularityAnalysis {
    let evaluated = jacobian.evaluate(values);

    let (rank, pivot_rows) = compute_rank_with_pivots(&evaluated);

    let max_rank = jacobian.rows.min(jacobian.cols);
    let is_singular = rank < max_rank;
    let redundant_constraints = jacobian.rows.saturating_sub(rank);

    // Find non-pivot rows (these constraints are potentially redundant)
    let problematic_indices: Vec<usize> = (0..jacobian.rows)
        .filter(|i| !pivot_rows.contains(i))
        .collect();

    let problematic_ids: Vec<ConstraintId> = problematic_indices
        .iter()
        .filter_map(|&i| jacobian.constraints.get(i).copied())
        .collect();

    SingularityAnalysis {
        rank,
        max_rank,
        is_singular,
        redundant_constraints,
        problematic_constraint_indices: problematic_indices,
        problematic_constraint_ids: problematic_ids,
    }
}

/// Compute the rank of a matrix using Gaussian elimination with partial pivoting.
///
/// Returns (rank, set of pivot row indices).
fn compute_rank_with_pivots(matrix: &[Vec<Rational>]) -> (usize, std::collections::HashSet<usize>) {
    use std::collections::HashSet;

    if matrix.is_empty() {
        return (0, HashSet::new());
    }

    let rows = matrix.len();
    let cols = matrix[0].len();

    if cols == 0 {
        return (0, HashSet::new());
    }

    // Create a mutable copy
    let mut m: Vec<Vec<Rational>> = matrix.to_vec();

    let mut pivot_rows = HashSet::new();
    let mut pivot_row = 0;

    for col in 0..cols {
        if pivot_row >= rows {
            break;
        }

        // Find pivot (first non-zero entry in this column at or below pivot_row)
        let mut pivot_found = None;
        for r in pivot_row..rows {
            if m[r][col] != Rational::zero() {
                pivot_found = Some(r);
                break;
            }
        }

        let pivot_r = match pivot_found {
            Some(r) => r,
            None => continue, // No pivot in this column
        };

        // Swap rows
        m.swap(pivot_row, pivot_r);
        pivot_rows.insert(pivot_row);

        // Eliminate below
        let pivot_val = m[pivot_row][col].clone();
        for r in (pivot_row + 1)..rows {
            if m[r][col] != Rational::zero() {
                let factor = m[r][col].clone() / pivot_val.clone();
                for c in col..cols {
                    let sub = m[pivot_row][c].clone() * factor.clone();
                    m[r][c] = m[r][c].clone() - sub;
                }
            }
        }

        pivot_row += 1;
    }

    (pivot_rows.len(), pivot_rows)
}

/// Symbolic singularity check for linear constraints.
///
/// Analyzes the constant Jacobian without evaluating at a specific point.
pub fn check_linear_singularities(jacobian: &JacobianMatrix) -> Option<SingularityAnalysis> {
    // Check if all entries are constant
    let all_constant = jacobian.entries.iter().all(|row| {
        row.iter()
            .all(|entry| entry.as_ref().map(|d| d.is_constant()).unwrap_or(true))
    });

    if !all_constant {
        return None; // Cannot do symbolic analysis for non-linear constraints
    }

    // Extract constant matrix
    let constant_matrix: Vec<Vec<Rational>> = jacobian
        .entries
        .iter()
        .map(|row| {
            row.iter()
                .map(|entry| {
                    entry
                        .as_ref()
                        .and_then(|d| d.constant_value())
                        .unwrap_or(Rational::zero())
                })
                .collect()
        })
        .collect();

    let (rank, pivot_rows) = compute_rank_with_pivots(&constant_matrix);

    let max_rank = jacobian.rows.min(jacobian.cols);
    let is_singular = rank < max_rank;
    let redundant_constraints = jacobian.rows.saturating_sub(rank);

    let problematic_indices: Vec<usize> = (0..jacobian.rows)
        .filter(|i| !pivot_rows.contains(i))
        .collect();

    let problematic_ids: Vec<ConstraintId> = problematic_indices
        .iter()
        .filter_map(|&i| jacobian.constraints.get(i).copied())
        .collect();

    Some(SingularityAnalysis {
        rank,
        max_rank,
        is_singular,
        redundant_constraints,
        problematic_constraint_indices: problematic_indices,
        problematic_constraint_ids: problematic_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_linear_constraint(
        id: ConstraintId,
        coeffs: Vec<(JacobianVarId, i64)>,
    ) -> PolynomialConstraint {
        let terms: Vec<JacobianTerm> = coeffs
            .into_iter()
            .map(|(var, coeff)| JacobianTerm {
                coefficient: Rational::from_int(coeff),
                variables: vec![(var, 1)],
            })
            .collect();
        PolynomialConstraint { id, terms }
    }

    #[test]
    fn test_partial_derivative_linear() {
        // Constraint: 2x + 3y = 0
        let constraint = PolynomialConstraint {
            id: 1,
            terms: vec![
                JacobianTerm {
                    coefficient: Rational::from_int(2),
                    variables: vec![(0, 1)],
                },
                JacobianTerm {
                    coefficient: Rational::from_int(3),
                    variables: vec![(1, 1)],
                },
            ],
        };

        // ∂/∂x = 2
        let dx = constraint.partial_derivative(0).unwrap();
        assert!(dx.is_constant());
        assert_eq!(dx.constant_value().unwrap(), Rational::from_int(2));

        // ∂/∂y = 3
        let dy = constraint.partial_derivative(1).unwrap();
        assert!(dy.is_constant());
        assert_eq!(dy.constant_value().unwrap(), Rational::from_int(3));
    }

    #[test]
    fn test_partial_derivative_quadratic() {
        // Constraint: x² + y² = 0
        let constraint = PolynomialConstraint {
            id: 1,
            terms: vec![
                JacobianTerm {
                    coefficient: Rational::from_int(1),
                    variables: vec![(0, 2)], // x²
                },
                JacobianTerm {
                    coefficient: Rational::from_int(1),
                    variables: vec![(1, 2)], // y²
                },
            ],
        };

        // ∂/∂x = 2x
        let dx = constraint.partial_derivative(0).unwrap();
        assert!(!dx.is_constant());

        let mut values = HashMap::new();
        values.insert(0, Rational::from_int(3));
        values.insert(1, Rational::from_int(4));

        // At x=3, ∂/∂x = 6
        assert_eq!(dx.evaluate(&values), Rational::from_int(6));
    }

    #[test]
    fn test_jacobian_linear_system() {
        // System:
        //   x + y = 0
        //   x - y = 0
        let constraints = vec![
            make_linear_constraint(1, vec![(0, 1), (1, 1)]),
            make_linear_constraint(2, vec![(0, 1), (1, -1)]),
        ];

        let jacobian = compute_jacobian(&constraints);

        assert_eq!(jacobian.rows, 2);
        assert_eq!(jacobian.cols, 2);

        // Check rank symbolically
        let analysis = check_linear_singularities(&jacobian).unwrap();
        assert_eq!(analysis.rank, 2);
        assert!(!analysis.is_singular);
    }

    #[test]
    fn test_jacobian_redundant_constraint() {
        // System:
        //   x + y = 0
        //   2x + 2y = 0  (redundant)
        let constraints = vec![
            make_linear_constraint(1, vec![(0, 1), (1, 1)]),
            make_linear_constraint(2, vec![(0, 2), (1, 2)]),
        ];

        let jacobian = compute_jacobian(&constraints);

        let analysis = check_linear_singularities(&jacobian).unwrap();
        assert_eq!(analysis.rank, 1);
        assert!(analysis.is_singular);
        assert_eq!(analysis.redundant_constraints, 1);
        assert_eq!(analysis.problematic_constraint_ids, vec![2]);
    }

    #[test]
    fn test_jacobian_square_system() {
        // Well-conditioned 3x3 system
        //   x + 0 + 0 = 0
        //   0 + y + 0 = 0
        //   0 + 0 + z = 0
        let constraints = vec![
            make_linear_constraint(1, vec![(0, 1)]),
            make_linear_constraint(2, vec![(1, 1)]),
            make_linear_constraint(3, vec![(2, 1)]),
        ];

        let jacobian = compute_jacobian(&constraints);

        let analysis = check_linear_singularities(&jacobian).unwrap();
        assert_eq!(analysis.rank, 3);
        assert!(!analysis.is_singular);
    }

    #[test]
    fn test_jacobian_underdetermined() {
        // 2 constraints, 3 variables
        //   x + y = 0
        //   y + z = 0
        let constraints = vec![
            make_linear_constraint(1, vec![(0, 1), (1, 1)]),
            make_linear_constraint(2, vec![(1, 1), (2, 1)]),
        ];

        let jacobian = compute_jacobian(&constraints);

        assert_eq!(jacobian.rows, 2);
        assert_eq!(jacobian.cols, 3);

        let analysis = check_linear_singularities(&jacobian).unwrap();
        assert_eq!(analysis.rank, 2);
        assert!(!analysis.is_singular); // Full row rank
    }

    #[test]
    fn test_jacobian_overdetermined() {
        // 3 constraints, 2 variables
        //   x + y = 0
        //   x - y = 0
        //   2x + 0 = 0  (row 3 = row 1 + row 2)
        let constraints = vec![
            make_linear_constraint(1, vec![(0, 1), (1, 1)]),
            make_linear_constraint(2, vec![(0, 1), (1, -1)]),
            make_linear_constraint(3, vec![(0, 2)]),
        ];

        let jacobian = compute_jacobian(&constraints);

        let analysis = check_linear_singularities(&jacobian).unwrap();
        // Third constraint is linear combination of first two
        // Jacobian is 3x2, rank is 2, max_rank = min(3,2) = 2
        // is_singular = (rank < max_rank) = false (Jacobian has full column rank)
        // But there IS one redundant constraint (row rank < row count)
        assert_eq!(analysis.rank, 2);
        assert_eq!(analysis.max_rank, 2);
        assert!(!analysis.is_singular); // Full column rank means not singular
        assert_eq!(analysis.redundant_constraints, 1);
        assert!(analysis.problematic_constraint_ids.contains(&3));
    }

    #[test]
    fn test_quadratic_jacobian_at_point() {
        // Circle constraint: x² + y² - 25 = 0
        let constraint = PolynomialConstraint {
            id: 1,
            terms: vec![
                JacobianTerm {
                    coefficient: Rational::from_int(1),
                    variables: vec![(0, 2)], // x²
                },
                JacobianTerm {
                    coefficient: Rational::from_int(1),
                    variables: vec![(1, 2)], // y²
                },
                JacobianTerm {
                    coefficient: Rational::from_int(-25),
                    variables: vec![], // constant
                },
            ],
        };

        let jacobian = compute_jacobian(&[constraint]);

        // Evaluate at point (3, 4)
        let mut values = HashMap::new();
        values.insert(0, Rational::from_int(3));
        values.insert(1, Rational::from_int(4));

        let evaluated = jacobian.evaluate(&values);

        // Jacobian at (3, 4) should be [6, 8] (2*3, 2*4)
        assert_eq!(evaluated[0][0], Rational::from_int(6));
        assert_eq!(evaluated[0][1], Rational::from_int(8));

        let analysis = check_singularities(&jacobian, &values);
        assert_eq!(analysis.rank, 1); // One constraint, rank is 1
        assert!(!analysis.is_singular); // Full rank for 1x2 matrix
    }

    #[test]
    fn test_singularity_at_origin() {
        // Two circle constraints that are tangent at origin
        // x² + y² = 0 (degenerate)
        // (x-1)² + y² - 1 = 0
        let constraints = vec![
            PolynomialConstraint {
                id: 1,
                terms: vec![
                    JacobianTerm {
                        coefficient: Rational::from_int(1),
                        variables: vec![(0, 2)],
                    },
                    JacobianTerm {
                        coefficient: Rational::from_int(1),
                        variables: vec![(1, 2)],
                    },
                ],
            },
            // Expanded: x² - 2x + 1 + y² - 1 = x² + y² - 2x = 0
            PolynomialConstraint {
                id: 2,
                terms: vec![
                    JacobianTerm {
                        coefficient: Rational::from_int(1),
                        variables: vec![(0, 2)],
                    },
                    JacobianTerm {
                        coefficient: Rational::from_int(1),
                        variables: vec![(1, 2)],
                    },
                    JacobianTerm {
                        coefficient: Rational::from_int(-2),
                        variables: vec![(0, 1)],
                    },
                ],
            },
        ];

        let jacobian = compute_jacobian(&constraints);

        // At origin (0, 0), both Jacobian rows become [0, 0] and [-2, 0]
        let mut values = HashMap::new();
        values.insert(0, Rational::from_int(0));
        values.insert(1, Rational::from_int(0));

        let evaluated = jacobian.evaluate(&values);

        // First constraint Jacobian at origin: [2*0, 2*0] = [0, 0]
        assert_eq!(evaluated[0][0], Rational::from_int(0));
        assert_eq!(evaluated[0][1], Rational::from_int(0));

        // Second constraint Jacobian at origin: [2*0 - 2, 2*0] = [-2, 0]
        assert_eq!(evaluated[1][0], Rational::from_int(-2));
        assert_eq!(evaluated[1][1], Rational::from_int(0));

        let analysis = check_singularities(&jacobian, &values);
        assert!(analysis.is_singular);
        assert_eq!(analysis.rank, 1);
    }
}
