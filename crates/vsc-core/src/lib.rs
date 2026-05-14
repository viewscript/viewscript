//! ViewScript Core Library
//!
//! This crate provides the fundamental types and algorithms for the ViewScript
//! constraint system, including P-dimension space representation, constraint
//! collision detection, and repair suggestion generation.

pub mod types;
pub mod collision;
pub mod buildinfo;
pub mod config;
pub mod telemetry;
pub mod optimizer;
pub mod regression_promoter;
pub mod validator;
pub mod solver;
pub mod algebra;
pub mod analyzer;
pub mod component;
pub mod schema;
pub mod scene;
pub mod target;
pub mod ffi;

#[cfg(feature = "text-shaping")]
pub mod text;

#[cfg(feature = "proptest-tests")]
pub mod proptest_checks;

pub use types::*;
pub use collision::*;
pub use buildinfo::*;
pub use config::*;
pub use telemetry::*;
pub use optimizer::*;
pub use regression_promoter::*;
pub use validator::*;
pub use solver::*;
pub use algebra::*;
pub use analyzer::*;
pub use component::*;
pub use scene::*;
pub use target::*;
pub use ffi::*;

#[cfg(feature = "text-shaping")]
pub use text::*;
