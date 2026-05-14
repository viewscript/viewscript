//! Transform Stack Module
//!
//! Manages hierarchical transform composition during CanvasNode tree traversal.
//!
//! ## Usage
//!
//! When rendering a `CanvasGroupNode` tree:
//!
//! ```ignore
//! fn render_node(node: &CanvasNode, stack: &mut TransformStack) {
//!     match node {
//!         CanvasNode::Group { transform, children, .. } => {
//!             stack.push(transform);
//!             for child in children {
//!                 render_node(child, stack);
//!             }
//!             stack.pop();
//!         }
//!         CanvasNode::Path { .. } => {
//!             let world_transform = stack.current();
//!             // Use world_transform to render the path
//!         }
//!     }
//! }
//! ```

use crate::AffineTransform;

/// Stack for accumulating hierarchical transforms during tree traversal.
///
/// Each level in the CanvasNode tree may have a transform that affects all
/// descendants. `TransformStack` maintains the composed transform at each
/// level, allowing efficient queries of the current world transform.
#[derive(Debug, Clone)]
pub struct TransformStack {
    /// Stack of composed transforms.
    /// Each entry is the cumulative transform from root to that level.
    /// stack[0] is always identity (root level).
    stack: Vec<AffineTransform>,
}

impl Default for TransformStack {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformStack {
    /// Create a new transform stack initialized with identity.
    pub fn new() -> Self {
        Self {
            stack: vec![AffineTransform::identity()],
        }
    }

    /// Push a transform onto the stack.
    ///
    /// The new transform is composed with the current accumulated transform.
    /// The pushed transform is applied "on top of" the existing transforms,
    /// meaning it will be applied first in the final transform chain.
    ///
    /// ## Example
    ///
    /// ```
    /// # use vsc_gpu::transform::TransformStack;
    /// # use vsc_gpu::AffineTransform;
    /// let mut stack = TransformStack::new();
    ///
    /// // Push parent's translation
    /// stack.push(&AffineTransform::translation(100.0, 50.0));
    ///
    /// // Push child's scale (applied in child's local space)
    /// stack.push(&AffineTransform::scale(2.0, 2.0));
    ///
    /// // Current transform: translate(100, 50) then scale(2, 2)
    /// let (x, y) = stack.current().transform_point(10.0, 10.0);
    /// // Point (10, 10) → scale → (20, 20) → translate → (120, 70)
    /// assert!((x - 120.0).abs() < 0.001);
    /// assert!((y - 70.0).abs() < 0.001);
    /// ```
    pub fn push(&mut self, transform: &AffineTransform) {
        let current = self.current();
        let composed = current.compose(transform);
        self.stack.push(composed);
    }

    /// Pop the top transform from the stack.
    ///
    /// ## Panics
    ///
    /// Panics if attempting to pop the root identity transform (stack underflow).
    pub fn pop(&mut self) {
        if self.stack.len() <= 1 {
            panic!("TransformStack underflow: cannot pop root transform");
        }
        self.stack.pop();
    }

    /// Get the current accumulated transform.
    ///
    /// This is the composition of all transforms from root to the current level,
    /// ready to be converted to `TransformUniform` and uploaded to the GPU.
    pub fn current(&self) -> AffineTransform {
        self.stack
            .last()
            .cloned()
            .unwrap_or_else(AffineTransform::identity)
    }

    /// Get the current stack depth (number of pushed transforms + 1 for root).
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Check if the stack is at root level (no transforms pushed).
    pub fn is_root(&self) -> bool {
        self.stack.len() == 1
    }

    /// Reset the stack to initial state (identity only).
    pub fn clear(&mut self) {
        self.stack.clear();
        self.stack.push(AffineTransform::identity());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_stack_new() {
        let stack = TransformStack::new();
        assert_eq!(stack.depth(), 1);
        assert!(stack.is_root());

        // Current should be identity
        let t = stack.current();
        assert_eq!(t.a, 1.0);
        assert_eq!(t.d, 1.0);
        assert_eq!(t.tx, 0.0);
        assert_eq!(t.ty, 0.0);
    }

    #[test]
    fn test_transform_stack_push_translation() {
        let mut stack = TransformStack::new();

        stack.push(&AffineTransform::translation(100.0, 50.0));
        assert_eq!(stack.depth(), 2);
        assert!(!stack.is_root());

        let t = stack.current();
        assert_eq!(t.tx, 100.0);
        assert_eq!(t.ty, 50.0);

        // Point (0, 0) should transform to (100, 50)
        let (x, y) = t.transform_point(0.0, 0.0);
        assert!((x - 100.0).abs() < 0.001);
        assert!((y - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_transform_stack_nested_transforms() {
        let mut stack = TransformStack::new();

        // Parent: translate (100, 0)
        stack.push(&AffineTransform::translation(100.0, 0.0));

        // Child: scale 2x
        stack.push(&AffineTransform::scale(2.0, 2.0));

        assert_eq!(stack.depth(), 3);

        // Point (10, 10) in child's local space:
        // 1. Scale by 2 → (20, 20)
        // 2. Translate by 100 → (120, 20)
        let (x, y) = stack.current().transform_point(10.0, 10.0);
        assert!((x - 120.0).abs() < 0.001);
        assert!((y - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_transform_stack_pop() {
        let mut stack = TransformStack::new();

        stack.push(&AffineTransform::translation(100.0, 0.0));
        stack.push(&AffineTransform::scale(2.0, 2.0));
        assert_eq!(stack.depth(), 3);

        // Pop child transform
        stack.pop();
        assert_eq!(stack.depth(), 2);

        // Should be back to translation only
        let (x, y) = stack.current().transform_point(10.0, 10.0);
        assert!((x - 110.0).abs() < 0.001);
        assert!((y - 10.0).abs() < 0.001);

        // Pop parent transform
        stack.pop();
        assert!(stack.is_root());

        // Should be identity
        let (x, y) = stack.current().transform_point(10.0, 10.0);
        assert!((x - 10.0).abs() < 0.001);
        assert!((y - 10.0).abs() < 0.001);
    }

    #[test]
    #[should_panic(expected = "underflow")]
    fn test_transform_stack_underflow() {
        let mut stack = TransformStack::new();
        stack.pop(); // Should panic
    }

    #[test]
    fn test_transform_stack_clear() {
        let mut stack = TransformStack::new();

        stack.push(&AffineTransform::translation(100.0, 0.0));
        stack.push(&AffineTransform::scale(2.0, 2.0));
        assert_eq!(stack.depth(), 3);

        stack.clear();
        assert!(stack.is_root());
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn test_transform_stack_rotation_composition() {
        use std::f64::consts::PI;

        let mut stack = TransformStack::new();

        // Parent: translate to (100, 100)
        stack.push(&AffineTransform::translation(100.0, 100.0));

        // Child: rotate 90° (π/2 radians)
        stack.push(&AffineTransform::rotation(PI / 2.0));

        // Point (10, 0) in child's local space:
        // 1. Rotate 90° → (0, 10)
        // 2. Translate → (100, 110)
        let (x, y) = stack.current().transform_point(10.0, 0.0);
        assert!((x - 100.0).abs() < 0.001, "x = {}", x);
        assert!((y - 110.0).abs() < 0.001, "y = {}", y);
    }

    /// Task 1: compose() is associative within floating-point tolerance 1e-10.
    ///
    /// (A.compose(&B)).compose(&C)  ==  A.compose(&B.compose(&C))
    #[test]
    fn test_compose_associativity() {
        use std::f64::consts::PI;

        let a = AffineTransform::translation(13.0, -7.0);
        let b = AffineTransform::scale(3.0, 0.5);
        let c = AffineTransform::rotation(PI / 6.0);

        let lhs = a.compose(&b).compose(&c);
        let rhs = a.compose(&b.compose(&c));

        let tol = 1e-10;
        assert!((lhs.a - rhs.a).abs() < tol, "a: {} vs {}", lhs.a, rhs.a);
        assert!((lhs.b - rhs.b).abs() < tol, "b: {} vs {}", lhs.b, rhs.b);
        assert!((lhs.c - rhs.c).abs() < tol, "c: {} vs {}", lhs.c, rhs.c);
        assert!((lhs.d - rhs.d).abs() < tol, "d: {} vs {}", lhs.d, rhs.d);
        assert!((lhs.tx - rhs.tx).abs() < tol, "tx: {} vs {}", lhs.tx, rhs.tx);
        assert!((lhs.ty - rhs.ty).abs() < tol, "ty: {} vs {}", lhs.ty, rhs.ty);
    }

    /// Task 2: push(identity) followed by pop() leaves the stack state unchanged.
    #[test]
    fn test_identity_push_pop_invariant() {
        let mut stack = TransformStack::new();

        // Establish a non-trivial base state
        stack.push(&AffineTransform::translation(42.0, 17.0));
        let before = stack.current();

        // Push identity and immediately pop
        stack.push(&AffineTransform::identity());
        stack.pop();

        let after = stack.current();

        // All six components must be identical
        assert_eq!(before.a, after.a);
        assert_eq!(before.b, after.b);
        assert_eq!(before.c, after.c);
        assert_eq!(before.d, after.d);
        assert_eq!(before.tx, after.tx);
        assert_eq!(before.ty, after.ty);
    }

    #[test]
    fn test_affine_compose_method() {
        // Test the compose method directly
        let parent = AffineTransform::translation(100.0, 0.0);
        let child = AffineTransform::scale(2.0, 2.0);

        let combined = parent.compose(&child);

        // Point (10, 10):
        // 1. Scale by 2 → (20, 20)
        // 2. Translate → (120, 20)
        let (x, y) = combined.transform_point(10.0, 10.0);
        assert!((x - 120.0).abs() < 0.001);
        assert!((y - 20.0).abs() < 0.001);
    }
}
