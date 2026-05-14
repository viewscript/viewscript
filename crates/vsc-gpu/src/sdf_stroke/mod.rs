//! SDF Stroke Rendering (Phase I-3 / I-4)
//!
//! Renders quadratic and cubic Bezier curve strokes using Signed Distance Field
//! evaluation in the fragment shader. Each curve segment emits a single bounding
//! rectangle (2 triangles), and the shader analytically computes the distance
//! from each pixel to the curve.
//!
//! ## Algorithm (Quadratic)
//!
//! For a quadratic Bezier B(t) = (1-t)^2 P0 + 2t(1-t) P1 + t^2 P2:
//!
//! 1. Generate AABB bounding box around control points + stroke width
//! 2. For each fragment, solve cubic equation to find closest point on curve
//! 3. Use Cardano's formula for analytical cubic solution
//! 4. Compare distance to half stroke width
//! 5. Apply smoothstep anti-aliasing
//!
//! ## Algorithm (Cubic - Phase I-4)
//!
//! For a cubic Bezier B(t) = (1-t)^3 P0 + 3t(1-t)^2 P1 + 3t^2(1-t) P2 + t^3 P3:
//!
//! 1. Generate AABB bounding box around 4 control points + stroke width
//! 2. For each fragment, use Newton's method to find closest point
//! 3. Sample 5 initial points, refine with 4 iterations
//! 4. Compare distance to half stroke width
//! 5. Apply smoothstep anti-aliasing
//!
//! ## Vertex Formats
//!
//! Quadratic vertex contains curve control points (no uniform switching):
//!
//! ```text
//! SdfStrokeVertex (44 bytes):
//!   position:   [f32; 2]  // World-space rectangle vertex
//!   local_pos:  [f32; 2]  // Local coordinate for distance calculation
//!   p0:         [f32; 2]  // Curve start point
//!   p1:         [f32; 2]  // Control point
//!   p2:         [f32; 2]  // Curve end point
//!   half_width: f32       // Half stroke width
//! ```
//!
//! Cubic vertex stores all 4 control points:
//!
//! ```text
//! CubicSdfStrokeVertex (52 bytes):
//!   position:   [f32; 2]  // World-space rectangle vertex
//!   local_pos:  [f32; 2]  // Local coordinate for distance calculation
//!   p0:         [f32; 2]  // Curve start point
//!   p1:         [f32; 2]  // Control point 1
//!   p2:         [f32; 2]  // Control point 2
//!   p3:         [f32; 2]  // Curve end point
//!   half_width: f32       // Half stroke width
//! ```
//!
//! ## Batching
//!
//! Since control points are embedded in vertices, all strokes with the same
//! color can be batched into a single draw call without uniform switching.

mod cubic_tessellator;
mod cubic_vertex;
mod tessellator;
mod vertex;

pub use cubic_tessellator::{tessellate_cubic_stroke_segments, CubicSdfStrokeOutput};
pub use cubic_vertex::CubicSdfStrokeVertex;
pub use tessellator::{tessellate_stroke_segments, SdfStrokeOutput};
pub use vertex::SdfStrokeVertex;
