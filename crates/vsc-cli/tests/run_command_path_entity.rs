//! Integration tests for `vsc run-command` with PathEntity + FillSpec support.
//!
//! ## What is tested
//!
//! Verifies that after executing a CODL command that yields `path_entity` and
//! `fill_spec` operations:
//!
//! 1. The command exits successfully.
//! 2. The `.vsbuildinfo` file contains the expected `path_entities` entry.
//! 3. The `fill` field of the registered entity matches the RGBA values declared
//!    in the CODL fixture.

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Minimal project helper: init + run arbitrary vsc commands in a temp dir.
struct TestProject {
    _dir: TempDir,
    root: PathBuf,
}

impl TestProject {
    fn new() -> Self {
        let dir = TempDir::new().expect("Failed to create temp directory");
        let root = dir.path().to_path_buf();
        Self { _dir: dir, root }
    }

    fn run_vsc(&self, args: &[&str]) -> (i32, String, String) {
        let mut cmd = Command::cargo_bin("vsc").expect("Failed to find vsc binary");
        cmd.current_dir(&self.root);
        cmd.args(args);
        let output = cmd.output().expect("Failed to execute vsc");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    }

    fn read_buildinfo(&self) -> Value {
        let content =
            fs::read_to_string(self.root.join(".vsbuildinfo")).expect(".vsbuildinfo should exist");
        serde_json::from_str(&content).expect(".vsbuildinfo should be valid JSON")
    }

    fn write_file(&self, relative: &str, content: &str) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&path, content).expect("Failed to write file");
    }
}

/// CODL fixture: yields one path_entity (no segments) + one fill_spec (solid red).
const PATH_WITH_FILL_YAML: &str = r#"
name: path_with_fill
version: "1.0.0"
description: "Creates a path entity and applies a solid fill"

parameters:
  - name: path_id
    type: EntityId
    description: "Entity ID to assign to the new path"

operations:
  - type: path_entity
    id: "${path_id}"
    segments: []
    closed: false

  - type: fill_spec
    target: "${path_id}"
    fill:
      type: solid
      r: "255"
      g: "0"
      b: "0"
      a: "1"

metadata:
  author: "test"
  category: "test"
"#;

#[test]
fn test_run_command_registers_path_entity() {
    let proj = TestProject::new();

    // Initialize project
    let (code, _, stderr) = proj.run_vsc(&["init", "--name", "path-entity-test"]);
    assert_eq!(code, 0, "vsc init failed: {}", stderr);

    // Write the CODL fixture into the project directory
    proj.write_file("path_with_fill.vscmd.yaml", PATH_WITH_FILL_YAML);

    // Execute run-command with path_id = 42
    let args_json = r#"{"path_id": 42}"#;
    let (code, stdout, stderr) = proj.run_vsc(&[
        "run-command",
        "path_with_fill.vscmd.yaml",
        "--args",
        args_json,
    ]);
    assert_eq!(
        code, 0,
        "vsc run-command failed.\nstdout: {}\nstderr: {}",
        stdout, stderr
    );

    // The response JSON should indicate success
    let response: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {}\nstdout: {}", e, stdout));
    assert_eq!(
        response["status"], "success",
        "Expected status=success, got: {}",
        response
    );

    // path_entities_generated should be 1
    assert_eq!(
        response["path_entities_generated"], 1,
        "Expected path_entities_generated=1, got: {}",
        response
    );

    // Read .vsbuildinfo and verify path_entities
    let buildinfo = proj.read_buildinfo();
    let path_entities = buildinfo["path_entities"]
        .as_array()
        .expect("path_entities should be an array");

    assert_eq!(
        path_entities.len(),
        1,
        "Expected 1 path entity, got {}",
        path_entities.len()
    );

    let entry = &path_entities[0];
    assert_eq!(
        entry["id"], 42,
        "Expected path entity id=42, got: {}",
        entry["id"]
    );

    // Verify fill field is present and is a solid fill
    let fill = &entry["fill"];
    assert!(
        !fill.is_null(),
        "Expected fill to be Some(...) but it was null"
    );
    assert_eq!(
        fill["type"], "solid",
        "Expected fill.type=solid, got: {}",
        fill
    );

    // The color string should encode rgba(255,0,0,1)
    let color = fill["color"]
        .as_str()
        .expect("fill.color should be a string");
    assert_eq!(
        color, "rgba(255,0,0,1)",
        "Expected fill.color=rgba(255,0,0,1), got: {}",
        color
    );
}

#[test]
fn test_run_command_path_entity_fill_isolated_from_other_entities() {
    // Verify that the fill spec is applied only to the matching path_id and
    // does not affect other entities that may already be in the buildinfo.
    let proj = TestProject::new();
    let (code, _, stderr) = proj.run_vsc(&["init", "--name", "isolation-test"]);
    assert_eq!(code, 0, "vsc init failed: {}", stderr);

    proj.write_file("path_with_fill.vscmd.yaml", PATH_WITH_FILL_YAML);

    // Run twice with different IDs – each should produce its own path_entity
    let (code, stdout, stderr) = proj.run_vsc(&[
        "run-command",
        "path_with_fill.vscmd.yaml",
        "--args",
        r#"{"path_id": 10}"#,
    ]);
    assert_eq!(
        code, 0,
        "first run-command failed.\nstdout: {}\nstderr: {}",
        stdout, stderr
    );

    let (code, stdout, stderr) = proj.run_vsc(&[
        "run-command",
        "path_with_fill.vscmd.yaml",
        "--args",
        r#"{"path_id": 20}"#,
    ]);
    assert_eq!(
        code, 0,
        "second run-command failed.\nstdout: {}\nstderr: {}",
        stdout, stderr
    );

    let buildinfo = proj.read_buildinfo();
    let path_entities = buildinfo["path_entities"]
        .as_array()
        .expect("path_entities should be an array");

    assert_eq!(
        path_entities.len(),
        2,
        "Expected 2 path entities after two runs, got {}",
        path_entities.len()
    );

    // Both should have solid red fills
    for entry in path_entities {
        let fill = &entry["fill"];
        assert!(!fill.is_null(), "All entries should have a fill");
        assert_eq!(fill["type"], "solid");
        assert_eq!(fill["color"], "rgba(255,0,0,1)");
    }

    // IDs should be distinct
    let ids: Vec<u64> = path_entities
        .iter()
        .map(|e| e["id"].as_u64().expect("id should be u64"))
        .collect();
    assert!(ids.contains(&10), "Should have entity id=10");
    assert!(ids.contains(&20), "Should have entity id=20");
}
