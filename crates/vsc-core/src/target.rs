//! Render target abstraction for ViewScript.
//!
//! This module defines the `RenderTarget` trait that all rendering backends
//! must implement. The trait consumes `SceneNode` IR (intermediate representation)
//! and produces rendered output specific to each target.
//!
//! ## Architecture
//!
//! ```text
//! vsc-core (SceneBuilder)
//!     │
//!     ▼ Vec<SceneNode>  ← Target-independent IR
//!     │
//!     ├── vs-web (wgpu/wasm)  ← Current implementation
//!     ├── vs-native (future)
//!     └── vs-svg (future)
//! ```
//!
//! ## Design Principles
//!
//! 1. **Minimal trait surface**: Only essential methods are included.
//! 2. **Initialization is target-specific**: Async surface creation (wgpu)
//!    and platform-specific setup are handled by target constructors.
//! 3. **Mutable render**: Targets may cache GPU state between frames.

use crate::scene::SceneNode;
use thiserror::Error;

// =============================================================================
// RenderTarget Trait
// =============================================================================

/// A rendering backend that consumes `SceneNode` IR.
///
/// Implementors convert the target-independent scene graph into their
/// specific rendering primitives (GPU vertices, SVG elements, etc.).
///
/// ## Example Implementation
///
/// ```ignore
/// impl RenderTarget for WebTarget {
///     fn name(&self) -> &str { "vs-web" }
///
///     fn render(&mut self, scene: &[SceneNode]) -> Result<(), TargetError> {
///         let canvas_nodes = self.converter.convert(scene.to_vec());
///         self.gpu_renderer.render_frame(&canvas_nodes, ...);
///         Ok(())
///     }
///
///     fn resize(&mut self, width: u32, height: u32) {
///         self.gpu_renderer.resize(width, height);
///     }
/// }
/// ```
pub trait RenderTarget {
    /// Returns the target name (e.g., "vs-web", "vs-native", "vs-svg").
    fn name(&self) -> &str;

    /// Render the scene graph to the target's output.
    ///
    /// This method may update internal GPU state (buffer caches, etc.),
    /// hence the `&mut self` receiver.
    ///
    /// # Arguments
    ///
    /// * `scene` - Slice of `SceneNode` representing the scene graph.
    ///
    /// # Errors
    ///
    /// Returns `TargetError` if rendering fails.
    fn render(&mut self, scene: &[SceneNode]) -> Result<(), TargetError>;

    /// Resize the rendering viewport.
    ///
    /// # Arguments
    ///
    /// * `width` - New viewport width in device pixels.
    /// * `height` - New viewport height in device pixels.
    fn resize(&mut self, width: u32, height: u32);
}

// =============================================================================
// Target Configuration
// =============================================================================

/// Configuration for initializing a render target.
#[derive(Debug, Clone)]
pub struct TargetConfig {
    /// Target name (e.g., "vs-web").
    pub name: String,
    /// Viewport width in device pixels.
    pub viewport_width: u32,
    /// Viewport height in device pixels.
    pub viewport_height: u32,
    /// Device pixel ratio for HiDPI displays.
    pub device_pixel_ratio: f64,
}

impl Default for TargetConfig {
    fn default() -> Self {
        Self {
            name: "vs-web".to_string(),
            viewport_width: 800,
            viewport_height: 600,
            device_pixel_ratio: 1.0,
        }
    }
}

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur during target operations.
#[derive(Debug, Error)]
pub enum TargetError {
    /// Rendering failed.
    #[error("render failed: {0}")]
    RenderFailed(String),

    /// Target not properly initialized.
    #[error("target not initialized: {0}")]
    NotInitialized(String),

    /// Resize operation failed.
    #[error("resize failed: {0}")]
    ResizeFailed(String),

    /// Scene conversion failed.
    #[error("scene conversion failed: {0}")]
    ConversionFailed(String),
}
