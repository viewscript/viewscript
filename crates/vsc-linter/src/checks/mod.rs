//! Lint check implementations.
//!
//! Each check module provides a `check` function that analyzes a parsed Rust file
//! and returns any violations found.

pub mod cycle_detection;
pub mod float_contamination;
pub mod global_state;
pub mod locus_prohibition;
pub mod nonlinear_constraint;
