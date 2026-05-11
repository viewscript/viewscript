//! Algebraic Geometry Module for P-Dimension Constraint Solving
//!
//! This module provides polynomial algebra and Gröbner basis computation
//! for solving systems of polynomial equations that arise from geometric
//! constraints (e.g., circle tangency, Apollonius problem).
//!
//! ## Architecture
//!
//! The solver pipeline is hierarchical:
//! - **L0**: Fourier-Motzkin elimination (linear constraints)
//! - **L1**: Lazy substitution (bilinear/quadratic with constant propagation)
//! - **L2**: Gröbner basis (general polynomial systems)
//!
//! ## Warning: Computational Complexity
//!
//! Gröbner basis computation has worst-case doubly exponential time complexity
//! (EXPSPACE-complete). The hierarchical pipeline ensures that L2 is only
//! invoked when L0/L1 cannot reduce the problem further.

pub mod polynomial;
pub mod groebner;
pub mod monomial;

pub use polynomial::*;
pub use groebner::*;
pub use monomial::*;
