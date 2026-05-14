//! Non-Linear Constraint Detection
//!
//! This check detects attempts to create constraints that would require
//! non-linear equation solving, which violates FM-decidability.
//!
//! ## Detected Violations
//!
//! 1. Function calls to parametric curve evaluation (e.g., `bezier_at(t)`)
//! 2. Intersection-related function patterns (e.g., `curve_intersection`)
//! 3. Tangent/normal/curvature computation patterns
//! 4. Constraint terms that reference Path entities directly
//!
//! ## Architectural Rationale (Phase 6)
//!
//! P-dimension operates strictly within the linear-rational domain to maintain
//! decidability via Fourier-Motzkin elimination. Non-linear operations would
//! require solving polynomial equations (Richardson's theorem territory).
//!
//! The only valid constraint targets are:
//! - ControlPoint entities (first-class P-vectors)
//! - Other entities with direct coordinate semantics
//!
//! Invalid constraint targets:
//! - Path entities (curves are defined by ControlPoints, not constrainable themselves)
//! - Parametric curve evaluations (t-parameter)
//! - Curve intersections
//! - Tangent/normal directions

use crate::{LintCheck, LintViolation, Severity};
use syn::{visit::Visit, Expr, ExprCall, ExprMethodCall, File, ItemFn};

/// Patterns indicating non-linear curve operations.
const NONLINEAR_FUNCTION_PATTERNS: &[&str] = &[
    // Parametric evaluation
    "bezier_at",
    "curve_at",
    "evaluate_at",
    "point_at_t",
    "sample_curve",
    "parametric_point",
    // Intersection computations
    "curve_intersection",
    "path_intersection",
    "bezier_intersection",
    "find_intersection",
    "intersect_curves",
    "line_curve_intersection",
    // Tangent/normal operations
    "tangent_at",
    "normal_at",
    "derivative_at",
    "curvature_at",
    "curve_tangent",
    "curve_normal",
    // Arc length (requires numerical integration)
    "arc_length",
    "length_at_t",
    "total_length",
    // Closest point (requires polynomial root finding)
    "closest_point_on_curve",
    "project_to_curve",
    "nearest_point",
];

/// Patterns in type names that indicate non-linear curve types.
const NONLINEAR_TYPE_PATTERNS: &[&str] = &[
    "CurvePoint",
    "ParametricPoint",
    "IntersectionPoint",
    "CurveSample",
];

/// Check a parsed Rust file for non-linear constraint patterns.
pub fn check(file: &File, file_path: &str, source: &str) -> Vec<LintViolation> {
    let mut visitor = NonLinearVisitor {
        file_path: file_path.to_string(),
        source: source.to_string(),
        violations: Vec::new(),
        in_constraint_context: false,
    };

    visitor.visit_file(file);
    visitor.violations
}

struct NonLinearVisitor {
    file_path: String,
    source: String,
    violations: Vec<LintViolation>,
    in_constraint_context: bool,
}

impl NonLinearVisitor {
    fn get_line_col(&self, span: proc_macro2::Span) -> (usize, usize) {
        let start = span.start();
        (start.line, start.column + 1)
    }

    fn get_snippet(&self, line: usize) -> String {
        self.source
            .lines()
            .nth(line.saturating_sub(1))
            .unwrap_or("")
            .to_string()
    }

    fn add_violation(&mut self, span: proc_macro2::Span, message: String) {
        let (line, column) = self.get_line_col(span);
        let snippet = self.get_snippet(line);

        self.violations.push(LintViolation {
            file: self.file_path.clone(),
            line,
            column,
            check: LintCheck::NonLinearConstraint,
            message,
            snippet,
            severity: Severity::Error,
        });
    }

    fn is_nonlinear_function(&self, name: &str) -> bool {
        let lower = name.to_lowercase();
        NONLINEAR_FUNCTION_PATTERNS
            .iter()
            .any(|p| lower.contains(p))
    }

    fn check_constraint_context(&self, fn_name: &str) -> bool {
        let lower = fn_name.to_lowercase();
        lower.contains("constraint")
            || lower.contains("add_constraint")
            || lower.contains("create_constraint")
            || lower.contains("build_constraint")
    }
}

impl<'ast> Visit<'ast> for NonLinearVisitor {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        let fn_name = node.sig.ident.to_string();
        let was_in_context = self.in_constraint_context;

        if self.check_constraint_context(&fn_name) {
            self.in_constraint_context = true;
        }

        syn::visit::visit_item_fn(self, node);

        self.in_constraint_context = was_in_context;
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        let method_name = node.method.to_string();

        if self.is_nonlinear_function(&method_name) {
            self.add_violation(
                node.method.span(),
                format!(
                    "NON_LINEAR_CONSTRAINT_REJECTED: Method `.{}()` performs non-linear curve \
                     computation. This cannot be used as a constraint target. \
                     Constrain ControlPoint entities instead.",
                    method_name
                ),
            );
        }

        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        // Check function calls
        if let Expr::Path(path) = node.func.as_ref() {
            if let Some(segment) = path.path.segments.last() {
                let fn_name = segment.ident.to_string();

                if self.is_nonlinear_function(&fn_name) {
                    self.add_violation(
                        segment.ident.span(),
                        format!(
                            "NON_LINEAR_CONSTRAINT_REJECTED: Function `{}()` performs non-linear \
                             curve computation. P-dimension constraints must target ControlPoint \
                             entities with linear-rational terms only.",
                            fn_name
                        ),
                    );
                }
            }
        }

        syn::visit::visit_expr_call(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_code(code: &str) -> Vec<LintViolation> {
        let file = syn::parse_file(code).expect("Failed to parse test code");
        check(&file, "test.rs", code)
    }

    #[test]
    fn test_detects_bezier_at_call() {
        let code = r#"
            fn create_constraint() {
                let point = curve.bezier_at(0.5);
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0]
            .message
            .contains("NON_LINEAR_CONSTRAINT_REJECTED"));
    }

    #[test]
    fn test_detects_curve_intersection() {
        let code = r#"
            fn compute_reference() {
                let pt = curve_intersection(curve_a, curve_b);
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0]
            .message
            .contains("NON_LINEAR_CONSTRAINT_REJECTED"));
    }

    #[test]
    fn test_detects_tangent_at() {
        let code = r#"
            fn get_direction() {
                let tangent = path.tangent_at(t);
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
    }

    #[test]
    fn test_detects_closest_point_on_curve() {
        let code = r#"
            fn snap_to_curve() {
                let snapped = closest_point_on_curve(curve, external_point);
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
    }

    #[test]
    fn test_allows_control_point_operations() {
        let code = r#"
            fn add_constraint(control_point: &ControlPoint) {
                // This is fine - we're constraining a ControlPoint's coordinates
                constraint.target = control_point.id;
                constraint.component = VectorComponent::X;
                constraint.term = ConstraintTerm::Const { value: Rational::from_int(100) };
            }
        "#;
        let violations = check_code(code);
        assert!(
            violations.is_empty(),
            "ControlPoint operations should be allowed"
        );
    }

    #[test]
    fn test_allows_linear_constraint_construction() {
        let code = r#"
            fn build_relative_constraint(anchor1: EntityId, anchor2: EntityId) {
                // Linear constraint: anchor1.x = anchor2.x + offset
                // This is FM-decidable
                let constraint = Constraint {
                    target: anchor1,
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Linear {
                        coefficient: Rational::one(),
                        entity_id: anchor2,
                        component: VectorComponent::X,
                        offset: Rational::from_int(50),
                    },
                };
            }
        "#;
        let violations = check_code(code);
        assert!(
            violations.is_empty(),
            "Linear constraints should be allowed"
        );
    }

    #[test]
    fn test_detects_arc_length_computation() {
        let code = r#"
            fn compute_length() {
                let len = path.arc_length();
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
    }
}
