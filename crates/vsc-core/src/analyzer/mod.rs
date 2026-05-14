//! Structural analysis module for constraint systems.
//!
//! This module provides algorithms for analyzing the structural properties
//! of constraint graphs, including rigidity analysis and singularity detection.

pub mod jacobian;
pub mod rigidity;
pub mod singularity;

pub use jacobian::*;
pub use rigidity::*;
pub use singularity::{
    build_jacobian_matrix, compute_rank_rational, detect_singularity, JacVar, JacobianMatrix,
    SingularityWarning,
};
