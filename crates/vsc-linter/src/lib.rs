//! ViewScript Static Analysis Linter
//!
//! This crate provides static analysis tools to verify mathematical integrity
//! of the ViewScript constraint system. It performs AST-level analysis to ensure:
//!
//! 1. **Float Contamination Prevention**: No f32/f64 in P-dimension core logic
//! 2. **Global State Immutability**: No mutable statics that could leak between WASM instances
//! 3. **Cycle Detection Correctness**: Proper graph traversal patterns in constraint solver
//!
//! ## Usage
//!
//! ```bash
//! vsc-lint --check float-contamination ./crates/vsc-core/src
//! vsc-lint --check global-state ./crates/vsc-core/src
//! vsc-lint --check cycle-detection ./crates/vsc-core/src
//! vsc-lint --all ./crates/vsc-core/src
//! ```

pub mod checks;

use std::path::Path;
use colored::Colorize;

/// A lint violation found during static analysis.
#[derive(Debug, Clone)]
pub struct LintViolation {
    /// File path where the violation was found.
    pub file: String,
    /// Line number (1-indexed).
    pub line: usize,
    /// Column number (1-indexed).
    pub column: usize,
    /// The check that found this violation.
    pub check: LintCheck,
    /// Human-readable message.
    pub message: String,
    /// The offending code snippet.
    pub snippet: String,
    /// Severity level.
    pub severity: Severity,
}

impl LintViolation {
    /// Format the violation for terminal output.
    pub fn format(&self) -> String {
        let severity_str = match self.severity {
            Severity::Error => "error".red().bold(),
            Severity::Warning => "warning".yellow().bold(),
        };

        format!(
            "{}: {} [{}]\n  --> {}:{}:{}\n   |\n   | {}\n   |",
            severity_str,
            self.message,
            self.check.as_str().cyan(),
            self.file,
            self.line,
            self.column,
            self.snippet.trim(),
        )
    }
}

/// Severity of a lint violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Available lint checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintCheck {
    FloatContamination,
    GlobalState,
    CycleDetection,
    /// Phase 6: Detects non-linear constraint patterns that violate FM-decidability.
    NonLinearConstraint,
    /// Phase 7: Detects circular/arc locus evaluations (x² + y² = R²).
    LocusProhibition,
}

impl LintCheck {
    pub fn as_str(&self) -> &'static str {
        match self {
            LintCheck::FloatContamination => "float-contamination",
            LintCheck::GlobalState => "global-state",
            LintCheck::CycleDetection => "cycle-detection",
            LintCheck::NonLinearConstraint => "nonlinear-constraint",
            LintCheck::LocusProhibition => "locus-prohibition",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "float-contamination" => Some(LintCheck::FloatContamination),
            "global-state" => Some(LintCheck::GlobalState),
            "cycle-detection" => Some(LintCheck::CycleDetection),
            "nonlinear-constraint" => Some(LintCheck::NonLinearConstraint),
            "locus-prohibition" => Some(LintCheck::LocusProhibition),
            _ => None,
        }
    }
}

/// Result of running lint checks.
#[derive(Debug, Default)]
pub struct LintResult {
    pub violations: Vec<LintViolation>,
    pub files_checked: usize,
}

impl LintResult {
    pub fn has_errors(&self) -> bool {
        self.violations.iter().any(|v| v.severity == Severity::Error)
    }

    pub fn error_count(&self) -> usize {
        self.violations.iter().filter(|v| v.severity == Severity::Error).count()
    }

    pub fn warning_count(&self) -> usize {
        self.violations.iter().filter(|v| v.severity == Severity::Warning).count()
    }
}

/// Run lint checks on a directory.
pub fn run_checks(
    path: &Path,
    checks: &[LintCheck],
) -> Result<LintResult, std::io::Error> {
    use walkdir::WalkDir;

    let mut result = LintResult::default();

    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "rs"))
    {
        let file_path = entry.path();
        let content = std::fs::read_to_string(file_path)?;

        // Parse the file
        let syntax = match syn::parse_file(&content) {
            Ok(syntax) => syntax,
            Err(err) => {
                eprintln!("Warning: Failed to parse {}: {}", file_path.display(), err);
                continue;
            }
        };

        let file_str = file_path.to_string_lossy().to_string();
        result.files_checked += 1;

        // Run requested checks
        for check in checks {
            match check {
                LintCheck::FloatContamination => {
                    let violations = checks::float_contamination::check(&syntax, &file_str, &content);
                    result.violations.extend(violations);
                }
                LintCheck::GlobalState => {
                    let violations = checks::global_state::check(&syntax, &file_str, &content);
                    result.violations.extend(violations);
                }
                LintCheck::CycleDetection => {
                    let violations = checks::cycle_detection::check(&syntax, &file_str, &content);
                    result.violations.extend(violations);
                }
                LintCheck::NonLinearConstraint => {
                    let violations = checks::nonlinear_constraint::check(&syntax, &file_str, &content);
                    result.violations.extend(violations);
                }
                LintCheck::LocusProhibition => {
                    let violations = checks::locus_prohibition::check(&syntax, &file_str, &content);
                    result.violations.extend(violations);
                }
            }
        }
    }

    Ok(result)
}
