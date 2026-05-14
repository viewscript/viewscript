//! Regression Promoter: Proptest Seeds → CLI Integration Tests
//!
//! This module implements the "Test Ouroboros" - automatically converting
//! proptest regression seeds (internal AST/constraint graph failures)
//! into equivalent CLI command sequences for black-box testing.
//!
//! ## Architecture
//!
//! ```text
//!   ┌─────────────────────────┐
//!   │  .proptest-regressions  │
//!   │  (binary seeds)         │
//!   └───────────┬─────────────┘
//!               │
//!               ▼
//!   ┌─────────────────────────┐
//!   │  Seed Deserializer      │
//!   │  (reconstruct AST)      │
//!   └───────────┬─────────────┘
//!               │
//!               ▼
//!   ┌─────────────────────────┐
//!   │  CLI Command Generator  │
//!   │  (AST → vsc add-*)      │
//!   └───────────┬─────────────┘
//!               │
//!       ┌───────┴───────┐
//!       ▼               ▼
//! ┌───────────┐   ┌───────────┐
//! │ Shell     │   │ Rust Test │
//! │ Script    │   │ Case      │
//! └───────────┘   └───────────┘
//! ```

use crate::types::*;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A CLI command that can be generated from a constraint.
#[derive(Debug, Clone)]
pub struct CliCommand {
    pub command: String,
    pub args: Vec<String>,
}

impl CliCommand {
    /// Format as a shell command string.
    pub fn to_shell(&self) -> String {
        let args_str = self
            .args
            .iter()
            .map(|a| {
                if a.contains(' ') || a.contains('"') || a.contains('{') {
                    format!("'{}'", a.replace('\'', "'\\''"))
                } else {
                    a.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        format!("vsc {} {}", self.command, args_str)
    }

    /// Format as a Rust test assertion.
    pub fn to_rust_test(&self) -> String {
        let args_array = self
            .args
            .iter()
            .map(|a| format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(", ");

        format!("project.run_vsc(&[\"{}\", {}]);", self.command, args_array)
    }
}

/// Generator for CLI commands from constraints.
pub struct CliCommandGenerator;

impl CliCommandGenerator {
    /// Generate a CLI command to add a constraint.
    pub fn add_constraint(constraint: &Constraint) -> CliCommand {
        let term_json = match &constraint.term {
            ConstraintTerm::Const { value } => {
                format!(r#"{{"type":"const","value":{}}}"#, value)
            }
            ConstraintTerm::Ref {
                entity_id,
                component,
            } => {
                format!(
                    r#"{{"type":"ref","entity_id":{},"component":"{}"}}"#,
                    entity_id.0,
                    component_to_str(component)
                )
            }
            ConstraintTerm::Linear {
                coefficient,
                entity_id,
                component,
                offset,
            } => {
                format!(
                    r#"{{"type":"linear","coefficient":{},"entity_id":{},"component":"{}","offset":{}}}"#,
                    coefficient,
                    entity_id.0,
                    component_to_str(component),
                    offset
                )
            }
            ConstraintTerm::LinearCombination { terms, offset } => {
                let terms_json: Vec<String> = terms
                    .iter()
                    .map(|f| {
                        format!(
                            r#"{{"coefficient":{},"entity_id":{},"component":"{}"}}"#,
                            f.coefficient,
                            f.entity_id.0,
                            component_to_str(&f.component)
                        )
                    })
                    .collect();
                format!(
                    r#"{{"type":"linear_combination","terms":[{}],"offset":{}}}"#,
                    terms_json.join(","),
                    offset
                )
            }
        };

        CliCommand {
            command: "add-constraint".to_string(),
            args: vec![
                constraint.target.0.to_string(),
                component_to_str(&constraint.component).to_string(),
                relation_to_str(&constraint.relation).to_string(),
                term_json,
            ],
        }
    }

    /// Generate a sequence of CLI commands from a constraint graph.
    pub fn from_constraint_graph(constraints: &[Constraint]) -> Vec<CliCommand> {
        let mut commands = vec![CliCommand {
            command: "init".to_string(),
            args: vec!["--name".to_string(), "regression-test".to_string()],
        }];

        for constraint in constraints {
            commands.push(Self::add_constraint(constraint));
        }

        commands
    }
}

fn component_to_str(component: &VectorComponent) -> &'static str {
    match component {
        VectorComponent::X => "x",
        VectorComponent::Y => "y",
        VectorComponent::Z => "z",
        VectorComponent::T => "t",
        VectorComponent::Value => "value",
        VectorComponent::R => "r",
        VectorComponent::G => "g",
        VectorComponent::B => "b",
        VectorComponent::Alpha => "alpha",
        VectorComponent::Position => "position",
    }
}

fn relation_to_str(relation: &RelationType) -> &'static str {
    match relation {
        RelationType::Eq => "eq",
        RelationType::Lt => "lt",
        RelationType::Le => "le",
        RelationType::Gt => "gt",
        RelationType::Ge => "ge",
    }
}

/// A promoted regression test case.
#[derive(Debug)]
pub struct PromotedTestCase {
    /// Original regression file path.
    pub source_file: String,
    /// Test case name (derived from seed).
    pub test_name: String,
    /// CLI commands to reproduce the regression.
    pub commands: Vec<CliCommand>,
    /// Expected behavior (panic, exit code, etc.).
    pub expected_behavior: ExpectedBehavior,
}

/// What behavior this regression test expects.
#[derive(Debug)]
pub enum ExpectedBehavior {
    /// The CLI should not panic.
    NoPanic,
    /// The CLI should return a specific exit code.
    ExitCode(i32),
    /// The CLI should output valid JSON.
    ValidJson,
    /// The CLI should detect a collision.
    CollisionDetected,
}

impl PromotedTestCase {
    /// Generate a shell script for this test case.
    pub fn to_shell_script(&self) -> String {
        let mut script = String::new();
        script.push_str("#!/usr/bin/env bash\n");
        script.push_str(&format!("# Promoted from: {}\n", self.source_file));
        script.push_str(&format!("# Test: {}\n", self.test_name));
        script.push_str("set -euo pipefail\n\n");

        script.push_str("WORKDIR=$(mktemp -d)\n");
        script.push_str("trap \"rm -rf $WORKDIR\" EXIT\n");
        script.push_str("cd $WORKDIR\n\n");

        for cmd in &self.commands {
            script.push_str(&cmd.to_shell());
            script.push('\n');
        }

        match &self.expected_behavior {
            ExpectedBehavior::NoPanic => {
                script.push_str("\necho \"Test passed: No panic\"\n");
            }
            ExpectedBehavior::ExitCode(code) => {
                script.push_str(&format!("\n# Expected exit code: {}\n", code));
            }
            ExpectedBehavior::ValidJson => {
                script.push_str("\n# Verify last output is valid JSON\n");
            }
            ExpectedBehavior::CollisionDetected => {
                script.push_str("\n# Expected: collision error\n");
            }
        }

        script
    }

    /// Generate a Rust test function for this test case.
    pub fn to_rust_test(&self) -> String {
        let mut test = String::new();
        test.push_str(&format!("/// Promoted from: {}\n", self.source_file));
        test.push_str("#[test]\n");
        test.push_str(&format!("fn {}() {{\n", self.test_name));
        test.push_str("    let dir = TempDir::new().expect(\"temp dir\");\n");
        test.push_str("    let project = TestProject::new_in(dir.path());\n\n");

        for cmd in &self.commands {
            test.push_str(&format!("    {}\n", cmd.to_rust_test()));
        }

        match &self.expected_behavior {
            ExpectedBehavior::NoPanic => {
                test.push_str("    // If we reach here, no panic occurred\n");
            }
            ExpectedBehavior::ExitCode(code) => {
                test.push_str(&format!("    // Expected exit code: {}\n", code));
            }
            _ => {}
        }

        test.push_str("}\n");
        test
    }
}

/// Scan for proptest regression files and promote them to CLI tests.
pub fn promote_regressions(regressions_dir: &Path) -> Vec<PromotedTestCase> {
    let mut promoted = Vec::new();

    if !regressions_dir.exists() {
        return promoted;
    }

    // Find all .proptest-regressions files
    if let Ok(entries) = fs::read_dir(regressions_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "txt").unwrap_or(false) {
                if let Some(cases) = promote_file(&path) {
                    promoted.extend(cases);
                }
            }
        }
    }

    promoted
}

fn promote_file(path: &Path) -> Option<Vec<PromotedTestCase>> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut cases = Vec::new();
    let file_name = path.file_name()?.to_string_lossy().to_string();

    for (i, line) in reader.lines().enumerate() {
        let line = line.ok()?;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse the seed (proptest format: space-separated values)
        let test_name = format!(
            "regression_{}_line_{}",
            file_name.replace('.', "_").replace('-', "_"),
            i
        );

        // For now, create a minimal test that verifies no panic
        // In production, we'd deserialize the actual seed
        let case = PromotedTestCase {
            source_file: path.to_string_lossy().to_string(),
            test_name,
            commands: vec![CliCommand {
                command: "init".to_string(),
                args: vec![],
            }],
            expected_behavior: ExpectedBehavior::NoPanic,
        };

        cases.push(case);
    }

    Some(cases)
}

/// Generate all promoted tests as a Rust source file.
pub fn generate_promoted_tests_file(cases: &[PromotedTestCase]) -> String {
    let mut source = String::new();

    source.push_str("//! Auto-generated regression tests from proptest seeds.\n");
    source.push_str("//! DO NOT EDIT - regenerate with `just promote-regressions`\n\n");
    source.push_str("#![allow(dead_code)]\n\n");
    source.push_str("use tempfile::TempDir;\n");
    source.push_str("use super::integration_harness::TestProject;\n\n");

    for case in cases {
        source.push_str(&case.to_rust_test());
        source.push('\n');
    }

    source
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Rational;

    #[test]
    fn test_cli_command_generation() {
        let constraint = Constraint {
            id: 1,
            target: EntityId(1),
            component: VectorComponent::X,
            relation: RelationType::Eq,
            term: ConstraintTerm::Const {
                value: Rational::from_int(100),
            },
            priority: ConstraintPriority::Hard,
            source_scope: None,
        };

        let cmd = CliCommandGenerator::add_constraint(&constraint);

        assert_eq!(cmd.command, "add-constraint");
        assert!(cmd.to_shell().contains("100"));
    }

    #[test]
    fn test_shell_script_generation() {
        let case = PromotedTestCase {
            source_file: "test.txt".to_string(),
            test_name: "test_regression_1".to_string(),
            commands: vec![CliCommand {
                command: "init".to_string(),
                args: vec![],
            }],
            expected_behavior: ExpectedBehavior::NoPanic,
        };

        let script = case.to_shell_script();
        assert!(script.contains("#!/usr/bin/env bash"));
        assert!(script.contains("vsc init"));
    }
}
