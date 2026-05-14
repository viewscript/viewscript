//! Float Contamination Static Check
//!
//! This check ensures that P-dimension core logic does not use floating-point
//! types or operations, which would violate the exact rational arithmetic
//! requirement for constraint decidability.
//!
//! ## Detected Violations
//!
//! 1. Type declarations using `f32` or `f64`
//! 2. Cast expressions `as f32` or `as f64`
//! 3. Floating-point literals (e.g., `1.0`, `1e-15`)
//! 4. Float-specific method calls (`floor`, `round`, `sin`, `cos`, etc.)
//!
//! ## Allowed Exceptions
//!
//! Functions annotated with `#[allow(float_contamination)]` or containing
//! `_for_rasterization` in their name are exempt, as rasterization is the
//! only permitted f64 conversion point.

use crate::{LintCheck, LintViolation, Severity};
use syn::{
    visit::Visit, Attribute, ExprCast, ExprLit, ExprMethodCall, File, ItemFn, Lit, Type, TypePath,
};

/// Float-specific methods that indicate float contamination.
const FLOAT_METHODS: &[&str] = &[
    "floor",
    "ceil",
    "round",
    "trunc",
    "fract",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sinh",
    "cosh",
    "tanh",
    "asinh",
    "acosh",
    "atanh",
    "sqrt",
    "cbrt",
    "hypot",
    "exp",
    "exp2",
    "exp_m1",
    "ln",
    "ln_1p",
    "log",
    "log2",
    "log10",
    "powi",
    "powf",
    "abs", // Note: abs exists for integers too, but context matters
    "signum",
    "copysign",
    "mul_add",
    "div_euclid",
    "rem_euclid",
    "to_degrees",
    "to_radians",
    "is_nan",
    "is_infinite",
    "is_finite",
    "is_subnormal",
    "is_normal",
    "classify",
    "is_sign_positive",
    "is_sign_negative",
    "recip",
    "max",
    "min", // These exist for floats specifically as methods
];

/// Check a parsed Rust file for float contamination.
pub fn check(file: &File, file_path: &str, source: &str) -> Vec<LintViolation> {
    let mut visitor = FloatContaminationVisitor {
        file_path: file_path.to_string(),
        source: source.to_string(),
        violations: Vec::new(),
        in_allowed_context: false,
        current_fn_name: None,
    };

    visitor.visit_file(file);
    visitor.violations
}

struct FloatContaminationVisitor {
    file_path: String,
    source: String,
    violations: Vec<LintViolation>,
    in_allowed_context: bool,
    current_fn_name: Option<String>,
}

impl FloatContaminationVisitor {
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

    fn is_allowed_by_attributes(&self, attrs: &[Attribute]) -> bool {
        for attr in attrs {
            if let Some(ident) = attr.path().get_ident() {
                let ident_str = ident.to_string();
                // Check for #[allow(float_contamination)] or similar
                if ident_str == "allow" {
                    if let syn::Meta::List(meta_list) = &attr.meta {
                        let tokens = meta_list.tokens.to_string();
                        if tokens.contains("float_contamination") {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn is_rasterization_context(&self) -> bool {
        if let Some(ref fn_name) = self.current_fn_name {
            fn_name.contains("rasterization")
                || fn_name.contains("to_f64_for_rasterization")
                || fn_name.contains("to_f32_for_rasterization")
        } else {
            false
        }
    }

    fn add_violation(&mut self, span: proc_macro2::Span, message: String) {
        if self.in_allowed_context || self.is_rasterization_context() {
            return;
        }

        let (line, column) = self.get_line_col(span);
        let snippet = self.get_snippet(line);

        self.violations.push(LintViolation {
            file: self.file_path.clone(),
            line,
            column,
            check: LintCheck::FloatContamination,
            message,
            snippet,
            severity: Severity::Error,
        });
    }
}

impl<'ast> Visit<'ast> for FloatContaminationVisitor {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        let prev_allowed = self.in_allowed_context;
        let prev_fn_name = self.current_fn_name.clone();

        // Check for allow attribute
        if self.is_allowed_by_attributes(&node.attrs) {
            self.in_allowed_context = true;
        }

        // Track function name for rasterization context
        self.current_fn_name = Some(node.sig.ident.to_string());

        // Visit the function body
        syn::visit::visit_item_fn(self, node);

        self.in_allowed_context = prev_allowed;
        self.current_fn_name = prev_fn_name;
    }

    fn visit_type(&mut self, node: &'ast Type) {
        if let Type::Path(TypePath { path, .. }) = node {
            if let Some(segment) = path.segments.last() {
                let ident = segment.ident.to_string();
                if ident == "f32" || ident == "f64" {
                    self.add_violation(
                        segment.ident.span(),
                        format!(
                            "Floating-point type `{}` detected in P-dimension code. \
                             Use `Rational` for exact arithmetic.",
                            ident
                        ),
                    );
                }
            }
        }
        syn::visit::visit_type(self, node);
    }

    fn visit_expr_cast(&mut self, node: &'ast ExprCast) {
        // Check for `as f32` or `as f64`
        if let Type::Path(TypePath { path, .. }) = node.ty.as_ref() {
            if let Some(segment) = path.segments.last() {
                let ident = segment.ident.to_string();
                if ident == "f32" || ident == "f64" {
                    self.add_violation(
                        segment.ident.span(),
                        format!(
                            "Cast to floating-point type `as {}` detected. \
                             This introduces float contamination into P-dimension.",
                            ident
                        ),
                    );
                }
            }
        }
        syn::visit::visit_expr_cast(self, node);
    }

    fn visit_expr_lit(&mut self, node: &'ast ExprLit) {
        if let Lit::Float(lit_float) = &node.lit {
            self.add_violation(
                lit_float.span(),
                format!(
                    "Floating-point literal `{}` detected. \
                     Use `Rational::new(num, denom)` for exact representation.",
                    lit_float.to_string()
                ),
            );
        }
        syn::visit::visit_expr_lit(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        let method_name = node.method.to_string();

        // Check for float-specific methods
        if FLOAT_METHODS.contains(&method_name.as_str()) {
            // Special case: abs() and similar might be used on integers
            // We report as warning, not error, for ambiguous methods
            let is_ambiguous = matches!(method_name.as_str(), "abs" | "signum" | "max" | "min");

            if !is_ambiguous {
                self.add_violation(
                    node.method.span(),
                    format!(
                        "Float-specific method `.{}()` detected. \
                         This method is only available on floating-point types.",
                        method_name
                    ),
                );
            }
        }

        syn::visit::visit_expr_method_call(self, node);
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
    fn test_detects_f64_type() {
        let code = r#"
            fn foo() {
                let x: f64 = 1.0;
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("f64"));
    }

    #[test]
    fn test_detects_f32_type() {
        let code = r#"
            fn foo() {
                let x: f32 = 1.0;
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
    }

    #[test]
    fn test_detects_float_literal() {
        let code = r#"
            fn foo() {
                let x = 1.5;
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("1.5"));
    }

    #[test]
    fn test_detects_cast_to_f64() {
        let code = r#"
            fn foo() {
                let x = 5 as f64;
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("as f64"));
    }

    #[test]
    fn test_allows_rasterization_function() {
        let code = r#"
            fn to_f64_for_rasterization(&self) -> f64 {
                let x: f64 = 1.0;
                x
            }
        "#;
        let violations = check_code(code);
        assert!(
            violations.is_empty(),
            "Rasterization functions should be allowed"
        );
    }

    #[test]
    fn test_allows_attributed_function() {
        let code = r#"
            #[allow(float_contamination)]
            fn render() {
                let x: f64 = 1.0;
            }
        "#;
        let violations = check_code(code);
        assert!(
            violations.is_empty(),
            "Functions with allow attribute should be skipped"
        );
    }

    #[test]
    fn test_no_violations_for_clean_code() {
        let code = r#"
            use some_crate::Rational;

            fn calculate(a: Rational, b: Rational) -> Rational {
                a + b
            }
        "#;
        let violations = check_code(code);
        assert!(violations.is_empty());
    }
}
