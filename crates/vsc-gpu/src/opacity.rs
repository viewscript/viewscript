//! Opacity Stack Management
//!
//! This module provides `OpacityStack`, which tracks accumulated opacity
//! through the scene graph hierarchy. Opacity is multiplicative: a child
//! with 0.5 opacity inside a group with 0.5 opacity renders at 0.25 opacity.
//!
//! ## Known Limitation
//!
//! Group opacity with multiple overlapping children causes double-blending
//! at overlap regions. Full correctness requires offscreen rendering (Phase E).
//!
//! For example, two 50% opacity rectangles overlapping:
//! - Correct: render both to offscreen buffer, then composite at 50%
//! - Current: each rectangle blends at 50%, overlap region is 75% visible
//!
//! ## Integration
//!
//! The current opacity value is passed to shaders via `TransformUniform.opacity`.
//! The fragment shader multiplies the final alpha: `color.a * uniform.opacity`.

/// Tracks accumulated opacity through scene graph hierarchy.
///
/// Opacity values are multiplicative: each nested group multiplies its
/// opacity with the accumulated parent opacity.
#[derive(Debug, Clone)]
pub struct OpacityStack {
    /// Stack of accumulated opacity values.
    /// stack[0] is always 1.0 (root level, fully opaque).
    stack: Vec<f32>,
}

impl Default for OpacityStack {
    fn default() -> Self {
        Self::new()
    }
}

impl OpacityStack {
    /// Create a new opacity stack with root opacity of 1.0.
    pub fn new() -> Self {
        Self { stack: vec![1.0] }
    }

    /// Push a new opacity level.
    ///
    /// The new accumulated opacity is `current * opacity`.
    ///
    /// ## Parameters
    ///
    /// - `opacity`: The local opacity value [0.0, 1.0]. Values outside this
    ///   range are clamped.
    pub fn push(&mut self, opacity: f32) {
        let safe_opacity = if opacity.is_nan() {
            log::warn!("NaN opacity detected, falling back to 1.0 (fully opaque)");
            1.0
        } else {
            opacity.clamp(0.0, 1.0)
        };
        let accumulated = self.current() * safe_opacity;
        self.stack.push(accumulated);
    }

    /// Pop the current opacity level, returning to the parent's opacity.
    ///
    /// Returns the popped opacity value, or 1.0 if attempting to pop the root.
    pub fn pop(&mut self) -> f32 {
        // Never pop the root (index 0)
        if self.stack.len() > 1 {
            self.stack.pop().unwrap_or(1.0)
        } else {
            1.0
        }
    }

    /// Get the current accumulated opacity value.
    ///
    /// This value should be passed to shaders for alpha multiplication.
    pub fn current(&self) -> f32 {
        *self.stack.last().unwrap_or(&1.0)
    }

    /// Check if opacity is fully opaque (1.0).
    pub fn is_opaque(&self) -> bool {
        (self.current() - 1.0).abs() < 0.001
    }

    /// Check if opacity is effectively invisible (near 0.0).
    pub fn is_invisible(&self) -> bool {
        self.current() < 0.001
    }

    /// Get the current nesting depth.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Reset the stack to initial state (fully opaque).
    pub fn reset(&mut self) {
        self.stack.clear();
        self.stack.push(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opacity_stack_new() {
        let stack = OpacityStack::new();
        assert!((stack.current() - 1.0).abs() < 0.001);
        assert!(stack.is_opaque());
        assert!(!stack.is_invisible());
    }

    #[test]
    fn test_opacity_stack_push() {
        let mut stack = OpacityStack::new();

        stack.push(0.5);
        assert!((stack.current() - 0.5).abs() < 0.001);

        // Multiplicative: 0.5 * 0.5 = 0.25
        stack.push(0.5);
        assert!((stack.current() - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_opacity_stack_pop() {
        let mut stack = OpacityStack::new();
        stack.push(0.5);
        stack.push(0.25);

        assert!((stack.current() - 0.125).abs() < 0.001);

        stack.pop();
        assert!((stack.current() - 0.5).abs() < 0.001);

        stack.pop();
        assert!((stack.current() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_opacity_stack_pop_underflow() {
        let mut stack = OpacityStack::new();

        // Should not pop below root
        stack.pop();
        assert!((stack.current() - 1.0).abs() < 0.001);

        stack.push(0.5);
        stack.pop();
        stack.pop();
        stack.pop();
        assert!((stack.current() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_opacity_stack_clamp() {
        let mut stack = OpacityStack::new();

        // Values > 1.0 should clamp to 1.0
        stack.push(1.5);
        assert!((stack.current() - 1.0).abs() < 0.001);

        stack.pop();

        // Values < 0.0 should clamp to 0.0
        stack.push(-0.5);
        assert!(stack.current() < 0.001);
    }

    #[test]
    fn test_opacity_stack_is_invisible() {
        let mut stack = OpacityStack::new();
        assert!(!stack.is_invisible());

        stack.push(0.0);
        assert!(stack.is_invisible());
    }

    #[test]
    fn test_opacity_stack_reset() {
        let mut stack = OpacityStack::new();
        stack.push(0.5);
        stack.push(0.5);
        stack.push(0.5);

        stack.reset();
        assert!((stack.current() - 1.0).abs() < 0.001);
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn test_opacity_stack_depth() {
        let mut stack = OpacityStack::new();
        assert_eq!(stack.depth(), 1);

        stack.push(0.5);
        assert_eq!(stack.depth(), 2);

        stack.push(0.5);
        assert_eq!(stack.depth(), 3);

        stack.pop();
        assert_eq!(stack.depth(), 2);
    }

    #[test]
    fn test_opacity_stack_nan_fallback() {
        let mut stack = OpacityStack::new();

        // NaN should fall back to 1.0 (fully opaque)
        stack.push(f32::NAN);
        assert!(
            (stack.current() - 1.0).abs() < 0.001,
            "NaN opacity should fall back to 1.0, got {}",
            stack.current()
        );

        // Push another NaN - should still be 1.0 * 1.0 = 1.0
        stack.push(f32::NAN);
        assert!(
            (stack.current() - 1.0).abs() < 0.001,
            "Nested NaN opacity should remain 1.0, got {}",
            stack.current()
        );

        // Push valid opacity after NaN
        stack.push(0.5);
        assert!(
            (stack.current() - 0.5).abs() < 0.001,
            "Valid opacity after NaN should work, got {}",
            stack.current()
        );
    }
}
