//! ViewScript Core Library
//!
//! This crate provides the fundamental types and algorithms for the ViewScript
//! constraint system, including P-dimension space representation, constraint
//! collision detection, and repair suggestion generation.

pub mod algebra;
pub mod analyzer;
pub mod buildinfo;
pub mod collision;
pub mod component;
pub mod config;
pub mod ffi;
pub mod optimizer;
pub mod regression_promoter;
pub mod scene;
pub mod schema;
pub mod solver;
pub mod target;
pub mod telemetry;
pub mod types;
pub mod validator;

#[cfg(feature = "text-shaping")]
pub mod text;

#[cfg(feature = "proptest-tests")]
pub mod proptest_checks;

pub use algebra::*;
pub use analyzer::*;
pub use buildinfo::*;
pub use collision::*;
pub use component::*;
pub use config::*;
pub use ffi::*;
pub use optimizer::*;
pub use regression_promoter::*;
pub use scene::*;
pub use solver::*;
pub use target::*;
pub use telemetry::*;
pub use types::*;
pub use validator::*;

#[cfg(feature = "text-shaping")]
pub use text::*;
