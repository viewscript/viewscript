//! Lint check implementations.
//!
//! Each check module provides a `check` function that analyzes a parsed Rust file
//! and returns any violations found.

pub mod float_contamination;
pub mod global_state;
pub mod cycle_detection;
pub mod nonlinear_constraint;
pub mod locus_prohibition;
