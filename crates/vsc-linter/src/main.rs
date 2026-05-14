//! ViewScript Static Analysis Linter CLI
//!
//! ## Usage
//!
//! ```bash
//! # Run specific check
//! vsc-lint --check float-contamination ./crates/vsc-core/src
//!
//! # Run all checks
//! vsc-lint --all ./crates/vsc-core/src
//!
//! # Output JSON
//! vsc-lint --all --format json ./crates/vsc-core/src
//! ```

use clap::{Parser, ValueEnum};
use colored::Colorize;
use std::path::PathBuf;
use std::process::ExitCode;

use vsc_linter::{run_checks, LintCheck, LintResult, Severity};

#[derive(Parser)]
#[command(name = "vsc-lint")]
#[command(about = "ViewScript static analysis linter for mathematical integrity")]
#[command(version)]
struct Cli {
    /// Path to the source directory to lint
    #[arg(required = true)]
    path: PathBuf,

    /// Run all checks
    #[arg(long, conflicts_with = "check")]
    all: bool,

    /// Specific check to run
    #[arg(long, value_parser = parse_check)]
    check: Option<LintCheck>,

    /// Output format
    #[arg(long, value_enum, default_value = "terminal")]
    format: OutputFormat,

    /// Treat warnings as errors
    #[arg(long)]
    warnings_as_errors: bool,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Terminal,
    Json,
}

fn parse_check(s: &str) -> Result<LintCheck, String> {
    LintCheck::from_str(s).ok_or_else(|| {
        format!(
            "Unknown check: {}. Available: float-contamination, global-state, cycle-detection",
            s
        )
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Determine which checks to run
    let checks: Vec<LintCheck> = if cli.all {
        vec![
            LintCheck::FloatContamination,
            LintCheck::GlobalState,
            LintCheck::CycleDetection,
        ]
    } else if let Some(check) = cli.check {
        vec![check]
    } else {
        eprintln!(
            "{}: Must specify --all or --check <CHECK>",
            "error".red().bold()
        );
        return ExitCode::FAILURE;
    };

    // Verify path exists
    if !cli.path.exists() {
        eprintln!(
            "{}: Path does not exist: {}",
            "error".red().bold(),
            cli.path.display()
        );
        return ExitCode::FAILURE;
    }

    // Run checks
    println!(
        "{} Running {} check(s) on {}...\n",
        "vsc-lint".cyan().bold(),
        checks.len(),
        cli.path.display()
    );

    let result = match run_checks(&cli.path, &checks) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("{}: {}", "error".red().bold(), err);
            return ExitCode::FAILURE;
        }
    };

    // Output results
    match cli.format {
        OutputFormat::Terminal => print_terminal(&result),
        OutputFormat::Json => print_json(&result),
    }

    // Determine exit code
    let has_failures = if cli.warnings_as_errors {
        !result.violations.is_empty()
    } else {
        result.has_errors()
    };

    if has_failures {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn print_terminal(result: &LintResult) {
    // Print violations
    for violation in &result.violations {
        println!("{}\n", violation.format());
    }

    // Print summary
    let error_count = result.error_count();
    let warning_count = result.warning_count();

    println!();
    println!(
        "{}: checked {} file(s)",
        "summary".bold(),
        result.files_checked
    );

    if error_count > 0 || warning_count > 0 {
        let error_str = if error_count > 0 {
            format!("{} error(s)", error_count).red().to_string()
        } else {
            format!("{} error(s)", error_count)
        };

        let warning_str = if warning_count > 0 {
            format!("{} warning(s)", warning_count).yellow().to_string()
        } else {
            format!("{} warning(s)", warning_count)
        };

        println!("  {} {}, {}", "found".bold(), error_str, warning_str);
    } else {
        println!("  {} {}", "result".bold(), "no issues found".green());
    }
}

fn print_json(result: &LintResult) {
    #[derive(serde::Serialize)]
    struct JsonOutput {
        files_checked: usize,
        error_count: usize,
        warning_count: usize,
        violations: Vec<JsonViolation>,
    }

    #[derive(serde::Serialize)]
    struct JsonViolation {
        file: String,
        line: usize,
        column: usize,
        check: String,
        severity: String,
        message: String,
        snippet: String,
    }

    let output = JsonOutput {
        files_checked: result.files_checked,
        error_count: result.error_count(),
        warning_count: result.warning_count(),
        violations: result
            .violations
            .iter()
            .map(|v| JsonViolation {
                file: v.file.clone(),
                line: v.line,
                column: v.column,
                check: v.check.as_str().to_string(),
                severity: match v.severity {
                    Severity::Error => "error".to_string(),
                    Severity::Warning => "warning".to_string(),
                },
                message: v.message.clone(),
                snippet: v.snippet.clone(),
            })
            .collect(),
    };

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
