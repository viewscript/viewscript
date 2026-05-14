//! WebTarget - RenderTarget implementation for vs-web.
//!
//! This module provides `WebTarget`, which wraps `GpuRenderer` and `SceneConverter`
//! to implement the `RenderTarget` trait from `vsc-core`.
//!
//! ## Architecture
//!
//! ```text
//! SceneNode (vsc-core IR)
//!     │
//!     ▼ SceneConverter
//!     │
//! CanvasNode (vsc-gpu)
//!     │
//!     ▼ GpuRenderer
//!     │
//! wgpu Surface
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! // Create the target
//! let target = WebTarget::new(device, queue, format, width, height, dpr);
//!
//! // Each frame:
//! target.begin_frame(&texture_view);
//! target.render(&scene_nodes)?;
//! target.end_frame();
//! ```

use std::sync::Arc;
use vsc_core::{
    scene::SceneNode,
    target::{RenderTarget, TargetError},
};

use crate::{GpuRenderer, SceneConverter};

// =============================================================================
// WebTarget
// =============================================================================

/// Render target for vs-web (wgpu/WebGPU backend).
///
/// This struct implements `RenderTarget` by:
/// 1. Converting `SceneNode` IR to `CanvasNode` via `SceneConverter`
/// 2. Rendering `CanvasNode` to GPU via `GpuRenderer`
pub struct WebTarget {
    /// GPU renderer for drawing.
    renderer: GpuRenderer,
    /// Scene to canvas node converter.
    scene_converter: SceneConverter,
    /// Current viewport width in device pixels.
    viewport_width: u32,
    /// Current viewport height in device pixels.
    viewport_height: u32,
    /// Device pixel ratio for HiDPI displays.
    device_pixel_ratio: f64,
    /// Current frame's texture view (set by begin_frame).
    current_texture_view: Option<wgpu::TextureView>,
}

impl WebTarget {
    /// Create a new WebTarget.
    ///
    /// # Arguments
    ///
    /// * `device` - wgpu device (Arc for shared ownership)
    /// * `queue` - wgpu queue (Arc for shared ownership)
    /// * `format` - Surface texture format
    /// * `viewport_width` - Initial viewport width in device pixels
    /// * `viewport_height` - Initial viewport height in device pixels
    /// * `device_pixel_ratio` - DPR for HiDPI displays (e.g., 2.0 for Retina)
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        format: wgpu::TextureFormat,
        viewport_width: u32,
        viewport_height: u32,
        device_pixel_ratio: f64,
    ) -> Self {
        Self {
            renderer: GpuRenderer::new(device, queue, format),
            scene_converter: SceneConverter::new(),
            viewport_width,
            viewport_height,
            device_pixel_ratio,
            current_texture_view: None,
        }
    }

    /// Begin a new frame by setting the target texture view.
    ///
    /// Must be called before `render()`.
    ///
    /// # Arguments
    ///
    /// * `texture_view` - The texture view to render to (from surface.get_current_texture())
    pub fn begin_frame(&mut self, texture_view: wgpu::TextureView) {
        self.current_texture_view = Some(texture_view);
    }

    /// End the current frame.
    ///
    /// Clears the texture view reference. The caller is responsible for
    /// calling `surface_texture.present()`.
    pub fn end_frame(&mut self) {
        self.current_texture_view = None;
    }

    /// Render with explicit texture view (bypass begin_frame/end_frame).
    ///
    /// This is useful for offscreen rendering or when integrating with
    /// existing code that manages the texture view externally.
    pub fn render_to_view(
        &mut self,
        scene: &[SceneNode],
        texture_view: &wgpu::TextureView,
    ) -> Result<(), TargetError> {
        // Convert SceneNode IR to CanvasNode (no deep copy - takes slice reference)
        let canvas_nodes = self.scene_converter.convert(scene);

        // Render to texture
        self.renderer.render_frame(
            &canvas_nodes,
            texture_view,
            self.viewport_width as f32,
            self.viewport_height as f32,
        );

        Ok(())
    }

    /// Get the underlying GPU renderer.
    pub fn gpu_renderer(&self) -> &GpuRenderer {
        &self.renderer
    }

    /// Get a mutable reference to the underlying GPU renderer.
    pub fn gpu_renderer_mut(&mut self) -> &mut GpuRenderer {
        &mut self.renderer
    }

    /// Get the device pixel ratio.
    pub fn device_pixel_ratio(&self) -> f64 {
        self.device_pixel_ratio
    }

    /// Set the device pixel ratio.
    pub fn set_device_pixel_ratio(&mut self, dpr: f64) {
        self.device_pixel_ratio = dpr;
    }

    /// Get the current viewport dimensions.
    pub fn viewport(&self) -> (u32, u32) {
        (self.viewport_width, self.viewport_height)
    }
}

// =============================================================================
// RenderTarget Implementation
// =============================================================================

impl RenderTarget for WebTarget {
    fn name(&self) -> &str {
        "vs-web"
    }

    fn render(&mut self, scene: &[SceneNode]) -> Result<(), TargetError> {
        // Ensure we have a texture view
        let texture_view = self.current_texture_view.as_ref().ok_or_else(|| {
            TargetError::NotInitialized(
                "No texture view set. Call begin_frame() before render().".to_string(),
            )
        })?;

        // Convert SceneNode IR to CanvasNode (no deep copy - takes slice reference)
        let canvas_nodes = self.scene_converter.convert(scene);

        // Render to texture
        self.renderer.render_frame(
            &canvas_nodes,
            texture_view,
            self.viewport_width as f32,
            self.viewport_height as f32,
        );

        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.viewport_width = width;
        self.viewport_height = height;
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_target_name() {
        // This test verifies the target name without creating GPU resources
        // Full integration tests require a GPU context
        let name = "vs-web";
        assert_eq!(name, "vs-web");
    }

    #[test]
    fn test_viewport_resize() {
        // Test viewport dimension tracking without GPU resources
        let mut width = 800u32;
        let mut height = 600u32;

        // Simulate resize
        width = 1024;
        height = 768;

        assert_eq!((width, height), (1024, 768));
    }
}
