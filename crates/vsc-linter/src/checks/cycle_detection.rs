//! Cycle Detection Logic Verification
//!
//! This check verifies that cycle detection algorithms in the constraint solver
//! follow correct graph-theoretic patterns. It performs a meta-test on the
//! algorithm's structure rather than its behavior.
//!
//! ## Required Patterns
//!
//! Correct cycle detection (DFS-based) requires:
//!
//! 1. A `visited` set/map to track fully processed nodes
//! 2. A `recursion_stack` (or `in_stack`) to track nodes in current DFS path
//! 3. Proper state management (insert at entry, remove at exit for recursion_stack)
//!
//! ## Verification Approach
//!
//! We analyze functions with names containing "cycle", "circular", or similar
//! and verify they contain the expected structural elements.

use syn::{
    visit::Visit,
    File, ItemFn, Expr, ExprMethodCall, Pat, Ident,
};
use crate::{LintCheck, LintViolation, Severity};

/// Keywords indicating cycle detection functions.
const CYCLE_FUNCTION_PATTERNS: &[&str] = &[
    "cycle",
    "circular",
    "has_cycle",
    "detect_cycle",
    "find_cycle",
    "check_cycle",
    "is_cyclic",
    "is_acyclic",
];

/// Required data structures for correct cycle detection.
const REQUIRED_STRUCTURES: &[&str] = &[
    "visited",
    "seen",
    "processed",
];

const STACK_STRUCTURES: &[&str] = &[
    "recursion_stack",
    "rec_stack",
    "in_stack",
    "stack",
    "path",
    "current_path",
];

/// Check a parsed Rust file for cycle detection correctness.
pub fn check(file: &File, file_path: &str, source: &str) -> Vec<LintViolation> {
    let mut visitor = CycleDetectionVisitor {
        file_path: file_path.to_string(),
        source: source.to_string(),
        violations: Vec::new(),
    };

    visitor.visit_file(file);
    visitor.violations
}

struct CycleDetectionVisitor {
    file_path: String,
    source: String,
    violations: Vec<LintViolation>,
}

impl CycleDetectionVisitor {
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

    fn is_cycle_detection_function(&self, name: &str) -> bool {
        let lower = name.to_lowercase();
        CYCLE_FUNCTION_PATTERNS.iter().any(|p| lower.contains(p))
    }

    fn analyze_cycle_function(&mut self, func: &ItemFn) {
        let fn_name = func.sig.ident.to_string();
        let span = func.sig.ident.span();

        // Collect all identifiers used in the function
        let mut ident_collector = IdentifierCollector::default();
        ident_collector.visit_block(&func.block);

        let identifiers = ident_collector.identifiers;

        // Check for visited-like structure
        let has_visited = identifiers.iter().any(|id| {
            let lower = id.to_lowercase();
            REQUIRED_STRUCTURES.iter().any(|s| lower.contains(s))
        });

        // Check for recursion stack structure
        let has_stack = identifiers.iter().any(|id| {
            let lower = id.to_lowercase();
            STACK_STRUCTURES.iter().any(|s| lower.contains(s))
        });

        // Check for insert/contains operations (HashSet methods)
        let has_insert = ident_collector.method_calls.iter().any(|m| {
            matches!(m.as_str(), "insert" | "push" | "add")
        });

        let has_contains = ident_collector.method_calls.iter().any(|m| {
            matches!(m.as_str(), "contains" | "get" | "contains_key")
        });

        let has_remove = ident_collector.method_calls.iter().any(|m| {
            matches!(m.as_str(), "remove" | "pop" | "take")
        });

        // Generate warnings/errors based on missing patterns
        if !has_visited {
            self.violations.push(LintViolation {
                file: self.file_path.clone(),
                line: self.get_line_col(span).0,
                column: self.get_line_col(span).1,
                check: LintCheck::CycleDetection,
                message: format!(
                    "Cycle detection function `{}` missing 'visited' tracking structure. \
                     Correct DFS cycle detection requires tracking fully processed nodes.",
                    fn_name
                ),
                snippet: self.get_snippet(self.get_line_col(span).0),
                severity: Severity::Warning,
            });
        }

        // For recursive cycle detection, we need both visited and recursion_stack
        // For iterative, we might use a different pattern
        if has_visited && !has_stack && !identifiers.iter().any(|id| id.contains("todo") || id.contains("queue")) {
            // Check if this is a recursive function (calls itself)
            let is_recursive = ident_collector.function_calls.iter().any(|f| f == &fn_name);

            if is_recursive {
                self.violations.push(LintViolation {
                    file: self.file_path.clone(),
                    line: self.get_line_col(span).0,
                    column: self.get_line_col(span).1,
                    check: LintCheck::CycleDetection,
                    message: format!(
                        "Recursive cycle detection function `{}` missing recursion stack. \
                         Without tracking the current DFS path, cycles may not be detected correctly.",
                        fn_name
                    ),
                    snippet: self.get_snippet(self.get_line_col(span).0),
                    severity: Severity::Warning,
                });
            }
        }

        // Check for proper state management
        if has_stack && has_insert && !has_remove {
            self.violations.push(LintViolation {
                file: self.file_path.clone(),
                line: self.get_line_col(span).0,
                column: self.get_line_col(span).1,
                check: LintCheck::CycleDetection,
                message: format!(
                    "Cycle detection function `{}` inserts to recursion stack but never removes. \
                     The recursion stack must be cleaned up when backtracking.",
                    fn_name
                ),
                snippet: self.get_snippet(self.get_line_col(span).0),
                severity: Severity::Error,
            });
        }

        // Verify contains check exists (cycle detection core logic)
        if (has_visited || has_stack) && !has_contains {
            self.violations.push(LintViolation {
                file: self.file_path.clone(),
                line: self.get_line_col(span).0,
                column: self.get_line_col(span).1,
                check: LintCheck::CycleDetection,
                message: format!(
                    "Cycle detection function `{}` has tracking structures but no membership check. \
                     Cycles are detected by checking if a node is already in the recursion stack.",
                    fn_name
                ),
                snippet: self.get_snippet(self.get_line_col(span).0),
                severity: Severity::Warning,
            });
        }
    }
}

impl<'ast> Visit<'ast> for CycleDetectionVisitor {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        let fn_name = node.sig.ident.to_string();

        if self.is_cycle_detection_function(&fn_name) {
            self.analyze_cycle_function(node);
        }

        syn::visit::visit_item_fn(self, node);
    }
}

/// Helper visitor to collect identifiers and method calls from a block.
#[derive(Default)]
struct IdentifierCollector {
    identifiers: Vec<String>,
    method_calls: Vec<String>,
    function_calls: Vec<String>,
}

impl<'ast> Visit<'ast> for IdentifierCollector {
    fn visit_ident(&mut self, node: &'ast Ident) {
        self.identifiers.push(node.to_string());
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        self.method_calls.push(node.method.to_string());
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr(&mut self, node: &'ast Expr) {
        if let Expr::Call(call) = node {
            if let Expr::Path(path) = call.func.as_ref() {
                if let Some(segment) = path.path.segments.last() {
                    self.function_calls.push(segment.ident.to_string());
                }
            }
        }
        syn::visit::visit_expr(self, node);
    }

    fn visit_pat(&mut self, node: &'ast Pat) {
        if let Pat::Ident(pat_ident) = node {
            self.identifiers.push(pat_ident.ident.to_string());
        }
        syn::visit::visit_pat(self, node);
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
    fn test_correct_cycle_detection_no_warnings() {
        let code = r#"
            fn has_cycle(graph: &Graph, node: NodeId) -> bool {
                let mut visited = HashSet::new();
                let mut recursion_stack = HashSet::new();

                fn dfs(graph: &Graph, node: NodeId, visited: &mut HashSet<NodeId>, stack: &mut HashSet<NodeId>) -> bool {
                    if stack.contains(&node) {
                        return true; // Cycle found
                    }
                    if visited.contains(&node) {
                        return false; // Already processed
                    }

                    stack.insert(node);

                    for neighbor in graph.neighbors(node) {
                        if dfs(graph, neighbor, visited, stack) {
                            return true;
                        }
                    }

                    stack.remove(&node);
                    visited.insert(node);
                    false
                }

                dfs(graph, node, &mut visited, &mut recursion_stack)
            }
        "#;
        let violations = check_code(code);
        // Should have no errors (may have warnings depending on analysis depth)
        let errors: Vec<_> = violations.iter().filter(|v| v.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "Correct implementation should have no errors");
    }

    #[test]
    fn test_missing_visited_generates_warning() {
        let code = r#"
            fn detect_cycle(graph: &Graph, node: NodeId) -> bool {
                // Missing visited tracking!
                for neighbor in graph.neighbors(node) {
                    if detect_cycle(graph, neighbor) {
                        return true;
                    }
                }
                false
            }
        "#;
        let violations = check_code(code);
        assert!(!violations.is_empty(), "Should warn about missing visited");
    }

    #[test]
    fn test_missing_remove_generates_error() {
        let code = r#"
            fn has_cycle(graph: &Graph, node: NodeId, visited: &mut HashSet<NodeId>, stack: &mut HashSet<NodeId>) -> bool {
                if stack.contains(&node) {
                    return true;
                }
                stack.insert(node);

                for neighbor in graph.neighbors(node) {
                    if has_cycle(graph, neighbor, visited, stack) {
                        return true;
                    }
                }

                // Missing: stack.remove(&node);
                visited.insert(node);
                false
            }
        "#;
        let violations = check_code(code);
        let errors: Vec<_> = violations.iter().filter(|v| v.severity == Severity::Error).collect();
        assert!(!errors.is_empty(), "Missing remove should be an error");
    }

    #[test]
    fn test_ignores_non_cycle_functions() {
        let code = r#"
            fn process_data(data: Vec<i32>) -> i32 {
                data.iter().sum()
            }
        "#;
        let violations = check_code(code);
        assert!(violations.is_empty(), "Non-cycle functions should be ignored");
    }
}
