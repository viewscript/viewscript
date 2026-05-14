//! Tests for target management commands (Stage 2).
//!
//! These tests verify the `vsc target add/remove/list` commands.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Helper to run vsc command in a temp directory.
fn run_vsc_in_dir(dir: &TempDir, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_vsc"))
        .args(args)
        .current_dir(dir.path())
        .output()
        .expect("Failed to execute vsc");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    (output.status.success(), stdout, stderr)
}

/// Helper to read .vsbuildinfo from temp directory.
fn read_buildinfo(dir: &TempDir) -> serde_json::Value {
    let path = dir.path().join(".vsbuildinfo");
    let content = fs::read_to_string(path).expect("Failed to read .vsbuildinfo");
    serde_json::from_str(&content).expect("Failed to parse .vsbuildinfo")
}

/// Test: `vsc target add vs-web` adds the target to buildinfo.
#[test]
fn test_target_add_vs_web() {
    let dir = TempDir::new().unwrap();

    // Initialize project
    let (success, _, _) = run_vsc_in_dir(&dir, &["init", "--name", "test"]);
    assert!(success, "init should succeed");

    // Add vs-web target
    let (success, stdout, stderr) = run_vsc_in_dir(&dir, &["target", "add", "vs-web"]);
    assert!(success, "target add should succeed: stderr={}", stderr);

    // Verify output
    let output: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON output");
    assert_eq!(output["status"], "success");
    assert_eq!(output["target"], "vs-web");

    // Verify buildinfo
    let buildinfo = read_buildinfo(&dir);
    let targets = buildinfo["targets"].as_array().expect("targets should be array");
    assert!(targets.iter().any(|t| t == "vs-web"), "targets should contain vs-web");
}

/// Test: Duplicate target addition returns error.
#[test]
fn test_target_add_duplicate_error() {
    let dir = TempDir::new().unwrap();

    // Initialize and add target
    run_vsc_in_dir(&dir, &["init", "--name", "test"]);
    run_vsc_in_dir(&dir, &["target", "add", "vs-web"]);

    // Try to add again
    let (success, _, stderr) = run_vsc_in_dir(&dir, &["target", "add", "vs-web"]);
    assert!(!success, "duplicate add should fail");

    // Verify error message
    let error: serde_json::Value = serde_json::from_str(&stderr).expect("Invalid JSON error");
    assert!(
        error["message"].as_str().unwrap().contains("already registered"),
        "error should mention already registered"
    );
}

/// Test: `vsc target list` returns added targets.
#[test]
fn test_target_list() {
    let dir = TempDir::new().unwrap();

    // Initialize and add target
    run_vsc_in_dir(&dir, &["init", "--name", "test"]);
    run_vsc_in_dir(&dir, &["target", "add", "vs-web"]);

    // List targets
    let (success, stdout, _) = run_vsc_in_dir(&dir, &["target", "list"]);
    assert!(success, "target list should succeed");

    // Verify output
    let output: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON output");
    assert_eq!(output["status"], "success");
    let targets = output["targets"].as_array().expect("targets should be array");
    assert!(targets.iter().any(|t| t == "vs-web"));

    // Verify known_targets is included
    let known = output["known_targets"].as_array().expect("known_targets should be array");
    assert!(known.iter().any(|t| t == "vs-web"));
}

/// Test: `vsc target remove vs-web` removes the target.
#[test]
fn test_target_remove() {
    let dir = TempDir::new().unwrap();

    // Initialize and add target
    run_vsc_in_dir(&dir, &["init", "--name", "test"]);
    run_vsc_in_dir(&dir, &["target", "add", "vs-web"]);

    // Remove target
    let (success, stdout, _) = run_vsc_in_dir(&dir, &["target", "remove", "vs-web"]);
    assert!(success, "target remove should succeed");

    // Verify output
    let output: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON output");
    assert_eq!(output["status"], "success");
    assert_eq!(output["action"], "target_remove");

    // Verify buildinfo
    let buildinfo = read_buildinfo(&dir);
    let targets = buildinfo["targets"].as_array().expect("targets should be array");
    assert!(!targets.iter().any(|t| t == "vs-web"), "targets should not contain vs-web");
}

/// Test: Unknown target name is rejected.
#[test]
fn test_target_add_unknown_error() {
    let dir = TempDir::new().unwrap();

    // Initialize
    run_vsc_in_dir(&dir, &["init", "--name", "test"]);

    // Try to add unknown target
    let (success, _, stderr) = run_vsc_in_dir(&dir, &["target", "add", "vs-unknown"]);
    assert!(!success, "unknown target should fail");

    // Verify error message
    let error: serde_json::Value = serde_json::from_str(&stderr).expect("Invalid JSON error");
    assert!(
        error["message"].as_str().unwrap().contains("Unknown target"),
        "error should mention unknown target"
    );
    assert!(
        error["message"].as_str().unwrap().contains("vs-unknown"),
        "error should include the target name"
    );
}

/// Test: Remove non-existent target returns error.
#[test]
fn test_target_remove_nonexistent_error() {
    let dir = TempDir::new().unwrap();

    // Initialize (no targets added)
    run_vsc_in_dir(&dir, &["init", "--name", "test"]);

    // Try to remove non-existent target
    let (success, _, stderr) = run_vsc_in_dir(&dir, &["target", "remove", "vs-web"]);
    assert!(!success, "remove non-existent should fail");

    // Verify error message
    let error: serde_json::Value = serde_json::from_str(&stderr).expect("Invalid JSON error");
    assert!(
        error["message"].as_str().unwrap().contains("not registered"),
        "error should mention not registered"
    );
}
