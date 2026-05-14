//! Topology-Preserving Rasterizer Module
//!
//! This module implements the rasterization layer that projects P-dimension
//! rational coordinates to discrete pixel coordinates while preserving
//! topological relationships (adjacency, containment, ordering).
//!
//! ## Module Structure
//!
//! - [`union_find`]: Union-Find data structure for coordinate equivalence classes
//! - [`distribution`]: Largest Remainder Method (LRM) for subpixel error distribution
//! - [`rounding`]: Main topology-preserving rounding algorithm
//!
//! ## The Problem
//!
//! Given two adjacent surfaces A and B where:
//!   A.right = 100.333... (rational)
//!   B.left = 100.333... (same rational)
//!
//! Naive rounding may produce:
//!   A.right = 100px (floor)
//!   B.left = 101px (ceil)
//!
//! This creates a 1px gap that violates the topological constraint
//! that A and B are adjacent (no gap, no overlap).
//!
//! ## Solution: Constraint-Aware Rounding
//!
//! Instead of rounding each coordinate independently, we:
//! 1. Build a graph of topological relationships (adjacency, containment)
//! 2. Partition coordinates into equivalence classes (same rational = same pixel)
//! 3. Round equivalence classes together
//! 4. Propagate rounding decisions through the constraint graph

pub mod distribution;
pub mod rounding;
pub mod union_find;

// Re-exports for convenient access
pub use distribution::{
    distribute_with_largest_remainder, DistributedDimension, DistributionMethod,
    DistributionResult, DistributionStats, SiblingGroup,
};
pub use rounding::{
    round_with_topology_preservation, RoundingResult, RoundingStats, TopologyViolation,
};
// Note: PVector, PVectorBounds, RasterBounds are defined in crate root (lib.rs)
// Note: Edge, CoordRef, TopoConstraint are defined in vsc-core::types
pub use union_find::{Axis, UnionFind};
pub use vsc_core::{CoordRef, Edge, TopoConstraint};
