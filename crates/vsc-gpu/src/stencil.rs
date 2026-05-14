//! Stencil Buffer Management for Clip Paths
//!
//! This module provides `StencilStack`, which tracks the current stencil reference
//! value for nested clip paths. Each clip path increases the stencil reference,
//! allowing proper nesting of clipping regions.
//!
//! ## How Stencil Clipping Works
//!
//! 1. Clip path geometry is rendered with `stencil_write` pipeline:
//!    - Color write mask: EMPTY (no color output)
//!    - Stencil operation: IncrementClamp on pass
//!
//! 2. Subsequent geometry uses stencil test:
//!    - Compare function: Equal
//!    - Reference value: current stencil depth
//!    - Only pixels matching the reference value pass
//!
//! 3. Nested clips increment the reference, popping decrements it.
//!
//! ## Example
//!
//! ```ignore
//! // Initial state: stencil = 0, all pixels pass
//! stencil_stack.push();  // Render clip path, stencil = 1 inside
//!
//! // Now only pixels where stencil == 1 pass (inside clip)
//! render_children();
//!
//! stencil_stack.pop();   // Back to stencil = 0
//! ```

/// Tracks current stencil reference value for nested clip paths.
///
/// The stencil buffer approach allows efficient nested clipping without
/// requiring multiple render passes or complex geometry operations.
#[derive(Debug, Clone)]
pub struct StencilStack {
    /// Current stencil reference value (0 = no clipping).
    depth: u32,
}

impl Default for StencilStack {
    fn default() -> Self {
        Self::new()
    }
}

impl StencilStack {
    /// Create a new stencil stack with depth 0 (no clipping).
    pub fn new() -> Self {
        Self { depth: 0 }
    }

    /// Push a new clip level, incrementing the stencil reference.
    ///
    /// Call this AFTER rendering the clip path geometry with the stencil write pipeline.
    pub fn push(&mut self) {
        self.depth += 1;
    }

    /// Pop a clip level, decrementing the stencil reference.
    ///
    /// Returns the previous depth before popping.
    pub fn pop(&mut self) -> u32 {
        let prev = self.depth;
        self.depth = self.depth.saturating_sub(1);
        prev
    }

    /// Get the current stencil reference value.
    ///
    /// This value should be passed to `render_pass.set_stencil_reference()`.
    pub fn current(&self) -> u32 {
        self.depth
    }

    /// Check if any clipping is active.
    pub fn is_clipping(&self) -> bool {
        self.depth > 0
    }

    /// Reset the stack to initial state (no clipping).
    pub fn reset(&mut self) {
        self.depth = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stencil_stack_new() {
        let stack = StencilStack::new();
        assert_eq!(stack.current(), 0);
        assert!(!stack.is_clipping());
    }

    #[test]
    fn test_stencil_stack_push_pop() {
        let mut stack = StencilStack::new();

        stack.push();
        assert_eq!(stack.current(), 1);
        assert!(stack.is_clipping());

        stack.push();
        assert_eq!(stack.current(), 2);

        let prev = stack.pop();
        assert_eq!(prev, 2);
        assert_eq!(stack.current(), 1);

        stack.pop();
        assert_eq!(stack.current(), 0);
        assert!(!stack.is_clipping());
    }

    #[test]
    fn test_stencil_stack_pop_underflow() {
        let mut stack = StencilStack::new();

        // Should not underflow past 0
        stack.pop();
        assert_eq!(stack.current(), 0);

        stack.push();
        stack.pop();
        stack.pop();
        stack.pop();
        assert_eq!(stack.current(), 0);
    }

    #[test]
    fn test_stencil_stack_reset() {
        let mut stack = StencilStack::new();
        stack.push();
        stack.push();
        stack.push();
        assert_eq!(stack.current(), 3);

        stack.reset();
        assert_eq!(stack.current(), 0);
        assert!(!stack.is_clipping());
    }
}
