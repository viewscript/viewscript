//! JSON Schema generation for ViewScript core types.
//!
//! This module provides functions to automatically generate JSON Schema
//! from the Rust type definitions using the `schemars` crate.
//!
//! ## Usage
//!
//! ```rust
//! use vsc_core::schema::{generate_schema, generate_constraint_schema, generate_buildinfo_schema};
//!
//! // Generate full VsBuildInfo schema
//! let schema = generate_buildinfo_schema();
//! println!("{}", schema);
//! ```

use schemars::schema_for;
use serde_json;

use crate::buildinfo::VsBuildInfo;
use crate::types::Constraint;

/// Generate JSON Schema for `VsBuildInfo` (the `.vsbuildinfo` file format).
///
/// Returns a pretty-printed JSON Schema string.
pub fn generate_buildinfo_schema() -> String {
    let schema = schema_for!(VsBuildInfo);
    serde_json::to_string_pretty(&schema).expect("Schema serialization failed")
}

/// Generate JSON Schema for `Constraint` (a single P-dimension constraint).
///
/// Returns a pretty-printed JSON Schema string.
pub fn generate_constraint_schema() -> String {
    let schema = schema_for!(Constraint);
    serde_json::to_string_pretty(&schema).expect("Schema serialization failed")
}

/// Generate JSON Schema for a given type and return as a parsed `serde_json::Value`.
///
/// This is the primary entry point for schema generation.
/// The returned value can be serialized, compared, or embedded in other documents.
pub fn generate_schema() -> String {
    generate_buildinfo_schema()
}
