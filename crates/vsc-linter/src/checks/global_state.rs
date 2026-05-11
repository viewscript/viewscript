//! Global State Immutability Check
//!
//! This check prevents the use of mutable global state, which could cause
//! state leakage between WASM instances. In a WASI-P1 environment, each
//! invocation should be stateless.
//!
//! ## Detected Violations
//!
//! 1. `static mut` declarations
//! 2. `lazy_static!` macro usage
//! 3. `thread_local!` macro usage
//! 4. `std::sync::OnceLock` usage
//! 5. `std::sync::Mutex` wrapping static data
//!
//! ## Rationale
//!
//! ViewScript WASM modules are designed to be invoked multiple times without
//! shared state. Mutable statics would persist across invocations, causing
//! non-deterministic behavior and potential security issues.

use syn::{
    visit::Visit,
    File, Item, ItemStatic, ItemMacro, Type, TypePath, StaticMutability,
};
use crate::{LintCheck, LintViolation, Severity};

/// Dangerous type patterns that indicate global mutable state.
const DANGEROUS_TYPES: &[&str] = &[
    "OnceLock",
    "OnceCell",
    "Mutex",
    "RwLock",
    "AtomicBool",
    "AtomicI8", "AtomicI16", "AtomicI32", "AtomicI64", "AtomicIsize",
    "AtomicU8", "AtomicU16", "AtomicU32", "AtomicU64", "AtomicUsize",
    "AtomicPtr",
    "Cell",
    "RefCell",
    "UnsafeCell",
];

/// Dangerous macro patterns.
const DANGEROUS_MACROS: &[&str] = &[
    "lazy_static",
    "thread_local",
    "once_cell",
];

/// Check a parsed Rust file for global state violations.
pub fn check(file: &File, file_path: &str, source: &str) -> Vec<LintViolation> {
    let mut visitor = GlobalStateVisitor {
        file_path: file_path.to_string(),
        source: source.to_string(),
        violations: Vec::new(),
    };

    visitor.visit_file(file);
    visitor.violations
}

struct GlobalStateVisitor {
    file_path: String,
    source: String,
    violations: Vec<LintViolation>,
}

impl GlobalStateVisitor {
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

    fn add_violation(&mut self, span: proc_macro2::Span, message: String, severity: Severity) {
        let (line, column) = self.get_line_col(span);
        let snippet = self.get_snippet(line);

        self.violations.push(LintViolation {
            file: self.file_path.clone(),
            line,
            column,
            check: LintCheck::GlobalState,
            message,
            snippet,
            severity,
        });
    }

    fn check_type_for_dangerous_patterns(&mut self, ty: &Type, static_span: proc_macro2::Span) {
        if let Type::Path(TypePath { path, .. }) = ty {
            for segment in &path.segments {
                let ident = segment.ident.to_string();

                if DANGEROUS_TYPES.contains(&ident.as_str()) {
                    self.add_violation(
                        static_span,
                        format!(
                            "Static variable uses `{}`, which enables mutable global state. \
                             This can cause state leakage between WASM invocations.",
                            ident
                        ),
                        Severity::Error,
                    );
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for GlobalStateVisitor {
    fn visit_item_static(&mut self, node: &'ast ItemStatic) {
        let span = node.ident.span();

        // Check for `static mut`
        if matches!(node.mutability, StaticMutability::Mut(_)) {
            self.add_violation(
                span,
                format!(
                    "Mutable static `{}` detected. `static mut` is forbidden in WASM modules \
                     to prevent state leakage between invocations.",
                    node.ident
                ),
                Severity::Error,
            );
        }

        // Check for dangerous wrapper types even on immutable statics
        self.check_type_for_dangerous_patterns(&node.ty, span);

        syn::visit::visit_item_static(self, node);
    }

    fn visit_item_macro(&mut self, node: &'ast ItemMacro) {
        // Check for dangerous macros
        if let Some(ref ident) = node.ident {
            // Named macros
            let name = ident.to_string();
            if DANGEROUS_MACROS.iter().any(|m| name.contains(m)) {
                self.add_violation(
                    ident.span(),
                    format!(
                        "Macro `{}!` creates global mutable state. \
                         This is forbidden in WASM modules.",
                        name
                    ),
                    Severity::Error,
                );
            }
        }

        // Check the macro path
        if let Some(segment) = node.mac.path.segments.last() {
            let macro_name = segment.ident.to_string();
            if DANGEROUS_MACROS.contains(&macro_name.as_str()) {
                self.add_violation(
                    segment.ident.span(),
                    format!(
                        "Macro `{}!` creates global mutable state. \
                         This is forbidden in WASM modules.",
                        macro_name
                    ),
                    Severity::Error,
                );
            }
        }

        syn::visit::visit_item_macro(self, node);
    }

    fn visit_item(&mut self, node: &'ast Item) {
        // Also check for use statements that import dangerous types
        if let Item::Use(use_item) = node {
            let use_str = quote::quote!(#use_item).to_string();

            for dangerous in DANGEROUS_TYPES {
                if use_str.contains(dangerous) {
                    // This is just a warning - importing is not violation, using is
                    // We don't add violation here, the type usage check will catch it
                }
            }
        }

        syn::visit::visit_item(self, node);
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
    fn test_detects_static_mut() {
        let code = r#"
            static mut COUNTER: i32 = 0;
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("static mut"));
    }

    #[test]
    fn test_detects_lazy_static() {
        let code = r#"
            lazy_static! {
                static ref FOO: String = String::new();
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("lazy_static"));
    }

    #[test]
    fn test_detects_thread_local() {
        let code = r#"
            thread_local! {
                static FOO: RefCell<i32> = RefCell::new(0);
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("thread_local"));
    }

    #[test]
    fn test_detects_once_lock() {
        let code = r#"
            static CONFIG: OnceLock<Config> = OnceLock::new();
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("OnceLock"));
    }

    #[test]
    fn test_detects_mutex_in_static() {
        let code = r#"
            static STATE: Mutex<Vec<i32>> = Mutex::new(Vec::new());
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty());
        assert!(violations[0].message.contains("Mutex"));
    }

    #[test]
    fn test_allows_immutable_static_const() {
        let code = r#"
            static VERSION: &str = "1.0.0";
            const MAX_SIZE: usize = 1024;
        "#;
        let violations = check_code(code);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_allows_local_mutex() {
        let code = r#"
            fn process() {
                let mutex = Mutex::new(vec![]);
                // Local mutex is fine
            }
        "#;
        let violations = check_code(code);
        assert!(violations.is_empty(), "Local variables should be allowed");
    }
}
