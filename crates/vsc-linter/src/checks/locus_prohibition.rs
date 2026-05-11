//! Locus Evaluation Prohibition Check
//!
//! This check detects attempts to bind entity coordinates (X, Y) to circular
//! or arc boundary conditions, which would introduce quadratic equations into
//! the P-dimension linear solver.
//!
//! ## The Problem
//!
//! A circle is defined by x² + y² = R². Evaluating points ON this locus requires:
//! 1. Solving quadratic equations
//! 2. Introducing irrational numbers (√2, etc.)
//! 3. Using epsilon-based approximations
//!
//! This breaks FM-decidability and causes topology failures (gaps, overlaps).
//!
//! ## Detected Violations
//!
//! 1. Quadratic forms: `x * x`, `x.pow(2)`, `x.powi(2)`
//! 2. Circle equations: `x² + y² = r²` patterns
//! 3. Trig functions: `cos`, `sin`, `tan` (parametric circle evaluation)
//! 4. Sqrt calls: `sqrt(x² + y²)` (distance calculations)
//! 5. Constraint terms referencing Arc/Radius locus points
//!
//! ## Allowed Patterns
//!
//! - Linear constraints on Radius scalar: `R1 = R2 + 10`
//! - Arc center position constraints: `arc.center.x = 100`
//! - Angle constraints: `arc.start_angle = 90` (scalar, not locus)
//!
//! ## Error Code
//!
//! `LOCUS_EVALUATION_REJECTED`: The constraint attempts to evaluate a point
//! on a circular/arc boundary. Use ControlPoints for constrainable positions.

use syn::{
    visit::Visit,
    spanned::Spanned,
    File, ItemFn, Expr, ExprBinary, ExprMethodCall, ExprCall, BinOp,
};
use crate::{LintCheck, LintViolation, Severity};

/// Patterns indicating locus (quadratic) operations.
const QUADRATIC_PATTERNS: &[&str] = &[
    "pow",
    "powi",
    "powf",
    "sqrt",
    "hypot",
];

/// Trig functions that indicate parametric circle evaluation.
const TRIG_FUNCTIONS: &[&str] = &[
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sin_cos",
];

/// Patterns indicating distance or radius calculations.
const DISTANCE_PATTERNS: &[&str] = &[
    "distance",
    "distance_squared",
    "magnitude",
    "length",
    "norm",
    "radius_at",
    "point_on_circle",
    "point_on_arc",
    "circumference_point",
    "arc_point",
];

/// Check a parsed Rust file for locus evaluation violations.
pub fn check(file: &File, file_path: &str, source: &str) -> Vec<LintViolation> {
    let mut visitor = LocusProhibitionVisitor {
        file_path: file_path.to_string(),
        source: source.to_string(),
        violations: Vec::new(),
        in_constraint_context: false,
    };

    visitor.visit_file(file);
    visitor.violations
}

struct LocusProhibitionVisitor {
    file_path: String,
    source: String,
    violations: Vec<LintViolation>,
    in_constraint_context: bool,
}

impl LocusProhibitionVisitor {
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
            check: LintCheck::LocusProhibition,
            message,
            snippet,
            severity: Severity::Error,
        });
    }

    fn is_quadratic_method(&self, name: &str) -> bool {
        QUADRATIC_PATTERNS.iter().any(|p| name == *p)
    }

    fn is_trig_function(&self, name: &str) -> bool {
        TRIG_FUNCTIONS.iter().any(|p| name == *p)
    }

    fn is_distance_function(&self, name: &str) -> bool {
        DISTANCE_PATTERNS.iter().any(|p| name.contains(p))
    }

    fn check_constraint_context(&self, fn_name: &str) -> bool {
        let lower = fn_name.to_lowercase();
        lower.contains("constraint")
            || lower.contains("solve")
            || lower.contains("evaluate")
    }
}

impl<'ast> Visit<'ast> for LocusProhibitionVisitor {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        let fn_name = node.sig.ident.to_string();
        let was_in_context = self.in_constraint_context;

        if self.check_constraint_context(&fn_name) {
            self.in_constraint_context = true;
        }

        syn::visit::visit_item_fn(self, node);

        self.in_constraint_context = was_in_context;
    }

    fn visit_expr_binary(&mut self, node: &'ast ExprBinary) {
        // Detect x * x patterns (squaring)
        if matches!(node.op, BinOp::Mul(_)) {
            // Check if left and right are the same identifier (x * x)
            if let (Expr::Path(left), Expr::Path(right)) = (node.left.as_ref(), node.right.as_ref()) {
                if left.path.segments.len() == 1 && right.path.segments.len() == 1 {
                    let left_name = left.path.segments[0].ident.to_string();
                    let right_name = right.path.segments[0].ident.to_string();

                    if left_name == right_name {
                        // This is x * x, which is a quadratic term
                        self.add_violation(
                            node.op.span(),
                            format!(
                                "LOCUS_EVALUATION_REJECTED: Quadratic expression `{} * {}` detected. \
                                 Squaring coordinates creates quadratic constraints that violate \
                                 FM-decidability. Use linear constraints on ControlPoints instead.",
                                left_name, right_name
                            ),
                        );
                    }
                }
            }
        }

        syn::visit::visit_expr_binary(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        let method_name = node.method.to_string();

        // Check for quadratic methods (pow, sqrt, etc.)
        if self.is_quadratic_method(&method_name) {
            self.add_violation(
                node.method.span(),
                format!(
                    "LOCUS_EVALUATION_REJECTED: Method `.{}()` introduces non-linear \
                     computation. Circular loci (x² + y² = R²) cannot be evaluated \
                     in P-dimension without breaking FM-decidability.",
                    method_name
                ),
            );
        }

        // Check for trig methods (sin, cos, etc.)
        if self.is_trig_function(&method_name) {
            self.add_violation(
                node.method.span(),
                format!(
                    "LOCUS_EVALUATION_REJECTED: Trigonometric method `.{}()` detected. \
                     Parametric circle evaluation (x = r*cos(θ)) introduces irrationals. \
                     Constrain Arc center/radius/angles linearly instead.",
                    method_name
                ),
            );
        }

        // Check for distance-like methods
        if self.is_distance_function(&method_name) {
            self.add_violation(
                node.method.span(),
                format!(
                    "LOCUS_EVALUATION_REJECTED: Method `.{}()` evaluates a circular locus. \
                     Distance calculations (√(x² + y²)) require irrational numbers. \
                     Use linear constraints on Radius scalar entities instead.",
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

                // Check for quadratic functions
                if self.is_quadratic_method(&fn_name) {
                    self.add_violation(
                        segment.ident.span(),
                        format!(
                            "LOCUS_EVALUATION_REJECTED: Function `{}()` introduces quadratic \
                             computation. Use linear constraints on scalar Radius entities.",
                            fn_name
                        ),
                    );
                }

                // Check for trig functions
                if self.is_trig_function(&fn_name) {
                    self.add_violation(
                        segment.ident.span(),
                        format!(
                            "LOCUS_EVALUATION_REJECTED: Trigonometric function `{}()` detected. \
                             Defer parametric evaluation to rasterization boundary.",
                            fn_name
                        ),
                    );
                }

                // Check for distance functions
                if self.is_distance_function(&fn_name) {
                    self.add_violation(
                        segment.ident.span(),
                        format!(
                            "LOCUS_EVALUATION_REJECTED: Function `{}()` evaluates a locus. \
                             P-dimension operates on linear-rational constraints only.",
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
    fn test_detects_x_squared() {
        let code = r#"
            fn compute_distance(x: Rational, y: Rational) -> Rational {
                x * x + y * y
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("LOCUS_EVALUATION_REJECTED"));
    }

    #[test]
    fn test_detects_sqrt_call() {
        let code = r#"
            fn compute_magnitude(x: f64, y: f64) -> f64 {
                (x * x + y * y).sqrt()
            }
        "#;
        let violations = check_code(code);
        // Should detect both the multiplication and the sqrt
        assert!(violations.len() >= 1);
    }

    #[test]
    fn test_detects_sin_cos() {
        let code = r#"
            fn point_on_circle(r: f64, theta: f64) -> (f64, f64) {
                (r * theta.cos(), r * theta.sin())
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations.iter().any(|v| v.message.contains("cos")));
        assert!(violations.iter().any(|v| v.message.contains("sin")));
    }

    #[test]
    fn test_detects_pow_method() {
        let code = r#"
            fn square_value(x: Rational) -> Rational {
                x.pow(2)
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("pow"));
    }

    #[test]
    fn test_detects_distance_function() {
        let code = r#"
            fn get_radius(center: Point, point: Point) -> Rational {
                center.distance(point)
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("distance"));
    }

    #[test]
    fn test_allows_linear_radius_constraint() {
        let code = r#"
            fn constrain_radii(r1: &Radius, r2: &Radius) -> Constraint {
                // This is fine - linear constraint on scalar radius values
                Constraint {
                    target: r1.id,
                    component: VectorComponent::Value, // Scalar component
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Linear {
                        coefficient: Rational::from_int(2),
                        entity_id: r2.id,
                        component: VectorComponent::Value,
                        offset: Rational::from_int(10),
                    },
                }
            }
        "#;
        let violations = check_code(code);
        assert!(violations.is_empty(), "Linear radius constraints should be allowed");
    }

    #[test]
    fn test_allows_arc_center_constraint() {
        let code = r#"
            fn constrain_arc_center(arc: &Arc, x_pos: Rational) {
                // This is fine - constraining the center ControlPoint
                let constraint = Constraint {
                    target: arc.center,
                    component: VectorComponent::X,
                    relation: RelationType::Eq,
                    term: ConstraintTerm::Const { value: x_pos },
                };
            }
        "#;
        let violations = check_code(code);
        assert!(violations.is_empty(), "Arc center constraints should be allowed");
    }

    #[test]
    fn test_detects_point_on_arc() {
        let code = r#"
            fn get_arc_point(arc: &Arc, t: f64) -> Point {
                arc.point_on_arc(t)
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("LOCUS_EVALUATION_REJECTED"));
    }

    #[test]
    fn test_detects_hypot() {
        let code = r#"
            fn compute_radius(x: f64, y: f64) -> f64 {
                x.hypot(y)
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("hypot"));
    }
}
