//! Jacobian rank-based singularity detection for constraint systems.
//!
//! This module constructs the Jacobian matrix of a constraint graph and uses
//! Gaussian elimination with partial pivoting (over `Rational`) to compute
//! the exact rank.  Because all arithmetic is performed in the `Rational`
//! field there are **no numerical errors**: the rank is algebraically exact.
//!
//! # Supported constraint kinds
//!
//! | Constraint kind | Partial derivative |
//! |---|---|
//! | `Linear { coefficient, .. }` | `coefficient` |
//! | `Ref { .. }` | `+1` (target variable) or `-1` (source variable) |
//! | `Const { .. }` | `0` (no variable dependency → empty row) |
//!
//! # Singularity condition
//!
//! Let the system have *n* free variables (columns).
//! If `rank(J) < n`, at least one variable is under-determined and a
//! [`SingularityWarning`] is emitted.

use crate::types::{Constraint, ConstraintTerm, EntityId, VectorComponent};
use crate::Rational;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

/// A variable in the Jacobian: one (entity, component) pair.
///
/// `PartialOrd` / `Ord` are implemented manually because `EntityId` and
/// `VectorComponent` do not derive those traits in the upstream types crate.
/// The ordering is lexicographic on `(entity.0, component_discriminant())`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JacVar {
    pub entity: EntityId,
    pub component: VectorComponent,
}

impl JacVar {
    pub fn new(entity: EntityId, component: VectorComponent) -> Self {
        Self { entity, component }
    }

    /// Stable numeric discriminant for `VectorComponent`, used for ordering.
    fn component_ord(c: VectorComponent) -> u8 {
        match c {
            VectorComponent::X => 0,
            VectorComponent::Y => 1,
            VectorComponent::Z => 2,
            VectorComponent::T => 3,
            VectorComponent::Value => 4,
            VectorComponent::R => 5,
            VectorComponent::G => 6,
            VectorComponent::B => 7,
            VectorComponent::Alpha => 8,
            VectorComponent::Position => 9,
        }
    }
}

impl PartialOrd for JacVar {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JacVar {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let entity_cmp = self.entity.0.cmp(&other.entity.0);
        if entity_cmp != std::cmp::Ordering::Equal {
            return entity_cmp;
        }
        Self::component_ord(self.component).cmp(&Self::component_ord(other.component))
    }
}

/// A dense Jacobian matrix stored in row-major order.
///
/// Row *i* corresponds to `constraints[i]`.
/// Column *j* corresponds to `variables[j]`.
#[derive(Clone, Debug)]
pub struct JacobianMatrix {
    /// Ordered list of variables (columns).
    pub variables: Vec<JacVar>,
    /// Constraint IDs matching each row.
    pub constraint_ids: Vec<u64>,
    /// The matrix data: `data[i][j]` = ∂(constraint i)/∂(variable j).
    pub data: Vec<Vec<Rational>>,
    /// Number of rows (constraints).
    pub rows: usize,
    /// Number of columns (variables).
    pub cols: usize,
}

/// Warning produced when the Jacobian rank is less than the number of variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SingularityWarning {
    /// Computed rank of the Jacobian.
    pub rank: usize,
    /// Number of free variables (= number of columns).
    pub num_variables: usize,
    /// Rank deficiency = `num_variables - rank`.
    pub deficiency: usize,
    /// Constraint IDs that were identified as linearly dependent (non-pivot rows
    /// after elimination).  These are the *redundant* constraints.
    pub redundant_constraint_ids: Vec<u64>,
    /// Human-readable description.
    pub message: String,
}

// ---------------------------------------------------------------------------
// Step 1 – Build Jacobian
// ---------------------------------------------------------------------------

/// Build the Jacobian matrix from a slice of [`Constraint`]s.
///
/// # Variable discovery
///
/// The set of variables is derived automatically from the constraints:
/// - Every `(target, component)` pair in a constraint contributes a column.
/// - For `Ref` and `Linear` constraints the *referenced* entity also contributes
///   a column (with coefficient `-1` and `-coefficient` respectively).
///
/// Variables are sorted deterministically (`EntityId` then `VectorComponent`)
/// so that the column order is stable across invocations.
///
/// # Partial-derivative rules
///
/// Each constraint encodes an equation of the form `lhs ⋈ rhs`.
/// We rewrite this as `f = lhs − rhs = 0` and compute ∂f/∂xⱼ:
///
/// | Term kind | f | ∂f/∂(target,comp) | ∂f/∂(ref,comp) |
/// |---|---|---|---|
/// | `Const { value }` | target − value | +1 | — |
/// | `Ref { entity, component }` | target − ref | +1 | −1 |
/// | `Linear { coeff, entity, component, offset }` | target − (coeff·ref + offset) | +1 | −coeff |
pub fn build_jacobian_matrix(constraints: &[Constraint]) -> JacobianMatrix {
    // --- collect all variable (entity, component) pairs ---
    let mut var_set: std::collections::BTreeSet<JacVar> = std::collections::BTreeSet::new();

    for c in constraints {
        // The target is always a variable.
        var_set.insert(JacVar::new(c.target, c.component));

        match &c.term {
            ConstraintTerm::Ref { entity_id, component } => {
                var_set.insert(JacVar::new(*entity_id, *component));
            }
            ConstraintTerm::Linear {
                entity_id,
                component,
                ..
            } => {
                var_set.insert(JacVar::new(*entity_id, *component));
            }
            ConstraintTerm::Const { .. } => {
                // No additional variable.
            }
        }
    }

    let variables: Vec<JacVar> = var_set.into_iter().collect();
    let var_index: HashMap<JacVar, usize> = variables
        .iter()
        .enumerate()
        .map(|(i, v)| (*v, i))
        .collect();

    let rows = constraints.len();
    let cols = variables.len();

    // --- build matrix rows ---
    let mut data: Vec<Vec<Rational>> = vec![vec![Rational::zero(); cols]; rows];
    let mut constraint_ids: Vec<u64> = Vec::with_capacity(rows);

    for (row, c) in constraints.iter().enumerate() {
        constraint_ids.push(c.id);

        let target_col = var_index[&JacVar::new(c.target, c.component)];

        // ∂f/∂(target) = +1 in all cases.
        data[row][target_col] = Rational::one();

        match &c.term {
            ConstraintTerm::Const { .. } => {
                // f = target − const  →  ∂f/∂(ref) = 0 (no ref variable)
            }
            ConstraintTerm::Ref { entity_id, component } => {
                // f = target − ref  →  ∂f/∂(ref) = −1
                let ref_col = var_index[&JacVar::new(*entity_id, *component)];
                accumulate(&mut data[row][ref_col], Rational::from_int(-1));
            }
            ConstraintTerm::Linear {
                coefficient,
                entity_id,
                component,
                ..
            } => {
                // f = target − (coeff·ref + offset)  →  ∂f/∂(ref) = −coeff
                let ref_col = var_index[&JacVar::new(*entity_id, *component)];
                accumulate(
                    &mut data[row][ref_col],
                    Rational::zero() - coefficient.clone(),
                );
            }
        }
    }

    JacobianMatrix {
        variables,
        constraint_ids,
        data,
        rows,
        cols,
    }
}

/// Add `delta` into `cell` in-place.
#[inline]
fn accumulate(cell: &mut Rational, delta: Rational) {
    *cell = cell.clone() + delta;
}

// ---------------------------------------------------------------------------
// Step 2 – Rank via Gaussian elimination with partial pivoting
// ---------------------------------------------------------------------------

/// Compute the rank of `matrix` using Gaussian elimination with partial
/// pivoting over `Rational`.
///
/// Returns `(rank, pivot_rows)` where `pivot_rows` is the set of row indices
/// that were selected as pivots.  Rows **not** in `pivot_rows` are linearly
/// dependent on the pivot rows above them.
///
/// Partial pivoting selects the row with the largest absolute value in the
/// current column (by `Rational` ordering).  Because we are working over an
/// exact field this only matters for numerical stability in a conceptual sense;
/// in exact arithmetic any non-zero entry is a valid pivot.  The largest-value
/// heuristic nonetheless tends to keep intermediate coefficients smaller.
pub fn compute_rank_rational(
    matrix: &[Vec<Rational>],
) -> (usize, std::collections::HashSet<usize>) {
    use std::collections::HashSet;

    if matrix.is_empty() {
        return (0, HashSet::new());
    }

    let rows = matrix.len();
    let cols = matrix[0].len();

    if cols == 0 {
        return (0, HashSet::new());
    }

    // Working copy.
    let mut m: Vec<Vec<Rational>> = matrix.to_vec();

    let mut pivot_rows: HashSet<usize> = HashSet::new();
    // `row_to_original[i]` = original row index of the row currently at
    // position i after swaps.
    let mut row_to_original: Vec<usize> = (0..rows).collect();

    let mut pivot_row: usize = 0; // next row to place a pivot into

    for col in 0..cols {
        if pivot_row >= rows {
            break;
        }

        // --- partial pivot: find row with largest |entry| in this column ---
        let mut best_row: Option<usize> = None;
        let mut best_abs = Rational::zero();

        for r in pivot_row..rows {
            let abs_val = m[r][col].abs();
            if abs_val > best_abs {
                best_abs = abs_val.clone();
                best_row = Some(r);
            }
        }

        let pivot_r = match best_row {
            Some(r) if best_abs != Rational::zero() => r,
            _ => continue, // entire column below pivot_row is zero → skip
        };

        // Swap rows in the working matrix and track original indices.
        m.swap(pivot_row, pivot_r);
        row_to_original.swap(pivot_row, pivot_r);

        pivot_rows.insert(row_to_original[pivot_row]);

        // --- eliminate entries below the pivot ---
        let pivot_val = m[pivot_row][col].clone();

        for r in (pivot_row + 1)..rows {
            if m[r][col] != Rational::zero() {
                let factor = m[r][col].clone() / pivot_val.clone();
                // Subtract factor × pivot_row from row r.
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

// ---------------------------------------------------------------------------
// Step 3 – Detect singularity
// ---------------------------------------------------------------------------

/// Analyse a set of constraints and return a [`SingularityWarning`] if the
/// Jacobian rank is less than the number of distinct variables.
///
/// Returns `None` when the system is full-rank (no singularity detected).
///
/// # Algorithm
///
/// 1. [`build_jacobian_matrix`] — discover variables and fill coefficient rows.
/// 2. [`compute_rank_rational`] — exact rank via Gaussian elimination.
/// 3. Compare rank to the column count (variable count).
///    - `rank == cols` → full rank, no warning.
///    - `rank < cols`  → rank deficient, emit [`SingularityWarning`].
///
/// The warning includes the IDs of the **redundant** constraints (non-pivot
/// rows), which the caller can report to the user for repair.
pub fn detect_singularity(constraints: &[Constraint]) -> Option<SingularityWarning> {
    if constraints.is_empty() {
        return None;
    }

    let jacobian = build_jacobian_matrix(constraints);

    if jacobian.cols == 0 {
        return None;
    }

    let (rank, pivot_rows) = compute_rank_rational(&jacobian.data);

    let num_variables = jacobian.cols;

    if rank >= num_variables {
        return None; // full rank — well-determined system
    }

    // Collect constraint IDs for non-pivot rows (linearly dependent rows).
    let redundant_constraint_ids: Vec<u64> = (0..jacobian.rows)
        .filter(|i| !pivot_rows.contains(i))
        .filter_map(|i| jacobian.constraint_ids.get(i).copied())
        .collect();

    let deficiency = num_variables - rank;

    let message = format!(
        "Jacobian rank deficiency detected: rank={rank}, variables={num_variables}, \
         deficiency={deficiency}. Redundant constraint IDs: {redundant_constraint_ids:?}"
    );

    Some(SingularityWarning {
        rank,
        num_variables,
        deficiency,
        redundant_constraint_ids,
        message,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        Constraint, ConstraintPriority, ConstraintTerm, EntityId, RelationType, VectorComponent,
    };

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn x_eq_const(id: u64, entity: EntityId, value: i64) -> Constraint {
        Constraint {
            id,
            target: entity,
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Const {
                value: Rational::from_int(value),
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        }
    }

    fn x_eq_ref(id: u64, from: EntityId, to: EntityId) -> Constraint {
        Constraint {
            id,
            target: from,
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Ref {
                entity_id: to,
                component: VectorComponent::X,
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        }
    }

    fn x_eq_linear(id: u64, from: EntityId, coeff: i64, to: EntityId, offset: i64) -> Constraint {
        Constraint {
            id,
            target: from,
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Linear {
                coefficient: Rational::from_int(coeff),
                entity_id: to,
                component: VectorComponent::X,
                offset: Rational::from_int(offset),
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        }
    }

    // ------------------------------------------------------------------
    // Test 1: Independent constraints — rank == variables
    // ------------------------------------------------------------------

    /// Three independent constraints, each fixing one distinct variable.
    /// Expected: no singularity.
    #[test]
    fn test_independent_constraints_no_singularity() {
        // Entity 1.X = 0,  Entity 2.X = 5,  Entity 3.X = 10
        let constraints = vec![
            x_eq_const(1, EntityId(1), 0),
            x_eq_const(2, EntityId(2), 5),
            x_eq_const(3, EntityId(3), 10),
        ];

        assert!(detect_singularity(&constraints).is_none());
    }

    // ------------------------------------------------------------------
    // Test 2: Linearly dependent constraints — rank < variables
    // ------------------------------------------------------------------

    /// Two constraints with identical coefficient rows (same variable, same
    /// equation).  The second row is a duplicate of the first, so rank = 1
    /// while there is 1 variable → rank == num_variables, so actually NOT
    /// singular from a variable-count perspective.
    ///
    /// To get an actual rank < variables scenario we need more variables than
    /// independent constraints.  This test uses a Ref chain where the third
    /// constraint is redundant:
    ///   Entity 1.X = Entity 2.X   (row: +1 for E1, -1 for E2)
    ///   Entity 2.X = Entity 3.X   (row: +1 for E2, -1 for E3)
    ///   Entity 1.X = Entity 3.X   (row: +1 for E1, -1 for E3) ← dependent (= row1 + row2)
    ///
    /// Variables: E1.X, E2.X, E3.X (cols = 3).
    /// Rank after elimination = 2 < 3 → singularity.
    #[test]
    fn test_dependent_constraints_singularity() {
        let c1 = x_eq_ref(1, EntityId(1), EntityId(2)); // E1.X = E2.X
        let c2 = x_eq_ref(2, EntityId(2), EntityId(3)); // E2.X = E3.X
        let c3 = x_eq_ref(3, EntityId(1), EntityId(3)); // E1.X = E3.X  (redundant)

        let warning = detect_singularity(&[c1, c2, c3]).expect("expected singularity warning");

        assert_eq!(warning.num_variables, 3, "three distinct variables");
        assert_eq!(warning.rank, 2, "rank should be 2");
        assert_eq!(warning.deficiency, 1);
        // The redundant constraint (ID 3) should appear in the warning.
        assert!(
            warning.redundant_constraint_ids.contains(&3),
            "constraint 3 is redundant, got: {:?}",
            warning.redundant_constraint_ids
        );
    }

    /// Explicit duplicate row test: same equation appears twice.
    /// Jacobian row for "E1.X = const" is [1] (single column).
    /// Duplicate gives rank 1 with 2 rows, 1 variable → rank == cols → no singularity.
    ///
    /// However the *duplicate* row represents an over-determined inconsistency;
    /// for the Jacobian rank test this particular case is NOT a rank deficiency
    /// (rank == num_variables = 1).  That is correct behaviour — the solver
    /// separately handles value conflicts; the Jacobian test checks linear
    /// independence.
    #[test]
    fn test_duplicate_row_not_rank_deficient_in_single_variable() {
        let constraints = vec![
            x_eq_const(1, EntityId(1), 0), // E1.X = 0
            x_eq_const(2, EntityId(1), 0), // E1.X = 0  (exact duplicate)
        ];

        // 1 variable, rank = 1 → full rank from variable perspective → no warning
        assert!(detect_singularity(&constraints).is_none());
    }

    // ------------------------------------------------------------------
    // Test 3: Overconstrained single point — conflicting constants
    // ------------------------------------------------------------------

    /// x = 0  and  x = 10 on the same variable.
    ///
    /// Jacobian rows:
    ///   [+1]  (from x = 0)
    ///   [+1]  (from x = 10)
    ///
    /// Both rows are identical (coefficient +1 for the single variable).
    /// After elimination:
    ///   pivot row 0: [1]   (selects E1.X as pivot)
    ///   row 1 becomes zero after elimination
    ///
    /// rank = 1 == num_variables = 1 → NOT rank-deficient by the Jacobian criterion.
    /// (The inconsistency x=0 ∧ x=10 is a *value* conflict, not a linear-independence
    /// issue; the Jacobian sees two identical coefficient rows.)
    ///
    /// This test documents that behaviour explicitly.
    #[test]
    fn test_single_point_overconstrained_jacobian_not_rank_deficient() {
        let constraints = vec![
            x_eq_const(1, EntityId(1), 0),  // x = 0
            x_eq_const(2, EntityId(1), 10), // x = 10
        ];

        // Both rows are [+1] for E1.X; rank = 1 = num_variables = 1.
        // The Jacobian does NOT signal singularity here.
        assert!(detect_singularity(&constraints).is_none());
    }

    /// To expose over-determination as a rank deficiency we need the constant
    /// terms to produce different coefficient patterns — or we introduce a
    /// second variable.  This test uses a two-variable system where both
    /// constraints pin the same equation:
    ///   E1.X = E2.X  (row: +1 for E1, -1 for E2)
    ///   E1.X = E2.X  (row: +1 for E1, -1 for E2)  ← exact duplicate
    ///
    /// rank = 1, num_variables = 2 → singularity detected.
    #[test]
    fn test_duplicate_ref_rows_singularity() {
        let constraints = vec![
            x_eq_ref(10, EntityId(1), EntityId(2)), // E1.X = E2.X
            x_eq_ref(11, EntityId(1), EntityId(2)), // E1.X = E2.X  (exact duplicate)
        ];

        let warning = detect_singularity(&constraints).expect("expected singularity");
        assert_eq!(warning.num_variables, 2);
        assert_eq!(warning.rank, 1);
        assert_eq!(warning.deficiency, 1);
        // Constraint 11 (row 1) is the redundant one.
        assert!(warning.redundant_constraint_ids.contains(&11));
    }

    // ------------------------------------------------------------------
    // Test 4: Linear coefficient rows
    // ------------------------------------------------------------------

    /// Two-variable system with linearly independent linear constraints.
    ///   E1.X = 2·E2.X + 0   (row: +1 for E1, -2 for E2)
    ///   E1.X = 3·E2.X + 0   (row: +1 for E1, -3 for E2)
    ///
    /// These are independent (different slopes) → rank = 2 = num_variables.
    #[test]
    fn test_linear_constraints_full_rank() {
        let constraints = vec![
            x_eq_linear(1, EntityId(1), 2, EntityId(2), 0), // E1.X = 2·E2.X
            x_eq_linear(2, EntityId(1), 3, EntityId(2), 0), // E1.X = 3·E2.X
        ];

        // rank = 2 = num_variables = 2
        assert!(detect_singularity(&constraints).is_none());
    }

    /// Same slope, so rows are proportional — rank deficient.
    ///   E1.X = 2·E2.X   (row: +1 for E1, -2 for E2)
    ///   E1.X = 2·E2.X   (row: +1 for E1, -2 for E2)  ← proportional (same)
    #[test]
    fn test_linear_constraints_rank_deficient() {
        let constraints = vec![
            x_eq_linear(3, EntityId(1), 2, EntityId(2), 0), // E1.X = 2·E2.X
            x_eq_linear(4, EntityId(1), 2, EntityId(2), 0), // E1.X = 2·E2.X  (duplicate)
        ];

        let warning = detect_singularity(&constraints).expect("expected singularity");
        assert_eq!(warning.deficiency, 1);
        assert!(warning.redundant_constraint_ids.contains(&4));
    }

    // ------------------------------------------------------------------
    // Test 5: Empty and degenerate inputs
    // ------------------------------------------------------------------

    #[test]
    fn test_empty_constraints_no_warning() {
        assert!(detect_singularity(&[]).is_none());
    }

    #[test]
    fn test_single_const_constraint_no_singularity() {
        let constraints = vec![x_eq_const(99, EntityId(42), 7)];
        assert!(detect_singularity(&constraints).is_none());
    }

    // ------------------------------------------------------------------
    // Test 6: build_jacobian_matrix column ordering
    // ------------------------------------------------------------------

    #[test]
    fn test_jacobian_column_ordering_stable() {
        // Build twice; column order must be the same.
        let constraints = vec![
            x_eq_ref(1, EntityId(3), EntityId(1)), // deliberately non-sequential entity IDs
            x_eq_ref(2, EntityId(1), EntityId(2)),
        ];

        let j1 = build_jacobian_matrix(&constraints);
        let j2 = build_jacobian_matrix(&constraints);

        assert_eq!(j1.variables, j2.variables);
    }

    // ------------------------------------------------------------------
    // Test 7: compute_rank_rational edge cases
    // ------------------------------------------------------------------

    #[test]
    fn test_rank_empty_matrix() {
        let (rank, pivots) = compute_rank_rational(&[]);
        assert_eq!(rank, 0);
        assert!(pivots.is_empty());
    }

    #[test]
    fn test_rank_identity_3x3() {
        let m = vec![
            vec![Rational::one(), Rational::zero(), Rational::zero()],
            vec![Rational::zero(), Rational::one(), Rational::zero()],
            vec![Rational::zero(), Rational::zero(), Rational::one()],
        ];
        let (rank, pivots) = compute_rank_rational(&m);
        assert_eq!(rank, 3);
        assert_eq!(pivots.len(), 3);
    }

    #[test]
    fn test_rank_zero_matrix() {
        let m = vec![
            vec![Rational::zero(), Rational::zero()],
            vec![Rational::zero(), Rational::zero()],
        ];
        let (rank, _) = compute_rank_rational(&m);
        assert_eq!(rank, 0);
    }

    #[test]
    fn test_rank_rational_fraction_entries() {
        // Matrix: [[1/2, 1/3], [1, 2/3]]
        // Row 2 = 2 * Row 1  → rank 1
        let m = vec![
            vec![Rational::new(1, 2), Rational::new(1, 3)],
            vec![Rational::from_int(1), Rational::new(2, 3)],
        ];
        let (rank, _) = compute_rank_rational(&m);
        assert_eq!(rank, 1);
    }
}
