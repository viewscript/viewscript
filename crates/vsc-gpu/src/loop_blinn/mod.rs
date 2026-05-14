//! Loop-Blinn Curve Rendering Module
//!
//! Implements GPU-accelerated quadratic Bezier rendering using the Loop-Blinn
//! algorithm. Instead of tessellating curves into many small triangles, each
//! quadratic Bezier segment is rendered as a single triangle with the fragment
//! shader evaluating the implicit curve equation.
//!
//! ## Performance Characteristics
//!
//! For 100 glyphs with 20 curve segments each:
//! - Lyon tessellation: 20,000-30,000 triangles (10-15 per segment)
//! - Loop-Blinn: 2,000 triangles (1 per segment)
//!
//! The trade-off is increased fragment shader complexity, but the net effect
//! is typically positive for text rendering workloads.
//!
//! ## Algorithm Overview
//!
//! For a quadratic Bezier B(t) = (1-t)²P₀ + 2t(1-t)P₁ + t²P₂:
//!
//! 1. Render the triangle (P₀, P₁, P₂)
//! 2. Assign texture coordinates: P₀=(0,0), P₁=(0.5,0), P₂=(1,1)
//! 3. Fragment shader evaluates f(u,v) = u² - v
//! 4. f < 0 → inside curve (draw), f > 0 → outside (discard)
//!
//! For concave curves (P₁ outside the curve), the sign is flipped.
//!
//! ## Cubic Bezier Support (I-2)
//!
//! Cubic Bezier curves are also supported via the `cubic` module, which
//! implements the full Loop-Blinn classification:
//!
//! - **Serpentine** (D > 0): Two inflection points
//! - **Cusp** (D = 0): Degenerate case
//! - **Loop** (D < 0): Self-intersecting curve
//!
//! The fragment shader evaluates f(k,l,m) = k³ - l·m for cubic curves.

mod convexity;
pub mod cubic;
mod tessellator;
mod vertex;

pub use convexity::compute_curve_sign;
pub use cubic::{classify_cubic, CubicClassification, CubicCurveType};
pub use tessellator::{
    tessellate_cubic_beziers, tessellate_quadratic_beziers,
    CubicLoopBlinnOutput, LoopBlinnOutput,
};
pub use vertex::{CubicLoopBlinnVertex, LoopBlinnVertex};
