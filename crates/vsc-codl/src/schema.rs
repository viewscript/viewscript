//! JSON Schema generation for CODL (Constraint Operation Description Language) types.
//!
//! This module provides functions to automatically generate JSON Schema
//! from the `CodlCommand` Rust type definitions using the `schemars` crate.

use schemars::schema_for;
use serde_json;

use crate::ast::CodlCommand;

/// Generate JSON Schema for `CodlCommand` (the `.vscmd.yaml` file format).
///
/// Returns a pretty-printed JSON Schema string representing the full
/// structure of a CODL command file.
pub fn generate_schema() -> String {
    let schema = schema_for!(CodlCommand);
    serde_json::to_string_pretty(&schema).expect("Schema serialization failed")
}

/// Generate JSON Schema for `CodlCommand` as a parsed JSON value.
pub fn generate_schema_value() -> serde_json::Value {
    let schema = schema_for!(CodlCommand);
    serde_json::to_value(&schema).expect("Schema serialization failed")
}
