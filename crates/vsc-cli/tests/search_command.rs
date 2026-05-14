//! Tests for `vsc search` command.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::tempdir;

/// Helper to run vsc command in a directory.
fn vsc_in(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("vsc").unwrap();
    cmd.current_dir(dir);
    cmd.env("VS_FIXED_TIME", "1704067200"); // Deterministic timestamps
    cmd
}

/// Test: `vsc search` returns empty array on fresh init.
#[test]
fn search_empty_buildinfo_returns_empty_array() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();

    // Initialize project
    vsc_in(dir)
        .args(["init", "--name", "search-test"])
        .assert()
        .success();

    // Search with no filters
    let output = vsc_in(dir)
        .args(["search"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["status"], "success");
    assert_eq!(json["count"], 0);
    assert!(json["results"].as_array().unwrap().is_empty());
}

/// Test: `vsc search -e <id>` returns related objects after add-component.
#[test]
fn search_by_entity_id_after_add_component() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();

    // Initialize project
    vsc_in(dir)
        .args(["init", "--name", "search-entity-test"])
        .assert()
        .success();

    // Add a component (this creates entity with constraints)
    let add_output = vsc_in(dir)
        .args([
            "add-component",
            "-t",
            "RoundedRect",
            "-x",
            "100",
            "-y",
            "200",
            "-w",
            "300",
            "--height",
            "150",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let add_json: Value = serde_json::from_slice(&add_output).unwrap();
    let entity_id = add_json["entity_id"].as_u64().unwrap();

    // Search by entity ID
    let search_output = vsc_in(dir)
        .args(["search", "-e", &entity_id.to_string()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let search_json: Value = serde_json::from_slice(&search_output).unwrap();

    assert_eq!(search_json["status"], "success");
    assert!(search_json["count"].as_u64().unwrap() >= 1);

    // Verify results contain the target entity
    let results = search_json["results"].as_array().unwrap();
    let has_target = results
        .iter()
        .any(|r| r["target"].as_u64() == Some(entity_id) || r["id"].as_u64() == Some(entity_id));
    assert!(has_target, "Results should contain the target entity");
}

/// Test: `vsc search -t constraint` returns only constraints.
#[test]
fn search_by_type_constraint_only() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();

    // Initialize project
    vsc_in(dir)
        .args(["init", "--name", "search-type-test"])
        .assert()
        .success();

    // Add multiple components to create constraints
    vsc_in(dir)
        .args([
            "add-component",
            "-t",
            "rect",
            "-x",
            "0",
            "-y",
            "0",
            "-w",
            "100",
            "--height",
            "100",
        ])
        .assert()
        .success();

    vsc_in(dir)
        .args([
            "add-component",
            "-t",
            "rect",
            "-x",
            "100",
            "-y",
            "100",
            "-w",
            "50",
            "--height",
            "50",
        ])
        .assert()
        .success();

    // Search for constraints only
    let output = vsc_in(dir)
        .args(["search", "-t", "constraint"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["status"], "success");
    assert_eq!(json["object_type"], "constraint");

    // All results should be constraints
    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "Should have constraint results");

    for result in results {
        assert_eq!(
            result["type"], "constraint",
            "All results should be constraints"
        );
    }
}

/// Test: `vsc search -t path` returns only paths (empty initially).
#[test]
fn search_by_type_path_empty() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();

    // Initialize project
    vsc_in(dir)
        .args(["init", "--name", "search-path-test"])
        .assert()
        .success();

    // Add a rect component (not a path)
    vsc_in(dir)
        .args(["add-component", "-t", "rect", "-x", "0", "-y", "0"])
        .assert()
        .success();

    // Search for paths only - should be empty
    let output = vsc_in(dir)
        .args(["search", "-t", "path"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["status"], "success");
    assert_eq!(json["object_type"], "path");
    assert_eq!(json["count"], 0);
}

/// Test: `vsc search -c x` filters by X component.
#[test]
fn search_by_component_x() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();

    // Initialize project
    vsc_in(dir)
        .args(["init", "--name", "search-component-test"])
        .assert()
        .success();

    // Add a component
    vsc_in(dir)
        .args(["add-component", "-t", "rect", "-x", "50", "-y", "100"])
        .assert()
        .success();

    // Search for X-component constraints
    let output = vsc_in(dir)
        .args(["search", "-t", "constraint", "-c", "x"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["status"], "success");
    assert_eq!(json["component_filter"], "x");

    // All results should have X component
    let results = json["results"].as_array().unwrap();
    for result in results {
        assert_eq!(
            result["component"], "X",
            "All results should have X component"
        );
    }
}

/// Test: `vsc search --limit 2` respects limit.
#[test]
fn search_respects_limit() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();

    // Initialize project
    vsc_in(dir)
        .args(["init", "--name", "search-limit-test"])
        .assert()
        .success();

    // Add multiple components to create many constraints
    for i in 0..5 {
        vsc_in(dir)
            .args([
                "add-component",
                "-t",
                "rect",
                "-x",
                &(i * 10).to_string(),
                "-y",
                &(i * 10).to_string(),
            ])
            .assert()
            .success();
    }

    // Search with limit 2
    let output = vsc_in(dir)
        .args(["search", "-l", "2"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["status"], "success");
    assert_eq!(json["limit"], 2);
    assert!(json["count"].as_u64().unwrap() <= 2);
}
