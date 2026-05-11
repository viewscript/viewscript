//! Structural analysis module for constraint systems.
//!
//! This module provides algorithms for analyzing the structural properties
//! of constraint graphs, including rigidity analysis and singularity detection.

pub mod rigidity;
pub mod jacobian;

pub use rigidity::*;
pub use jacobian::*;
