//! WebGPU Renderer for Browser Environment
//!
//! This module provides `WasmGpuRenderer`, the WASM-bindgen interface for
//! rendering ViewScript scenes to a browser `<canvas>` element via WebGPU.
//!
//! ## Stage 1: Minimal wgpu Surface Display
//!
//! This implementation focuses on establishing the WebGPU pipeline:
//! 1. Acquire GPU adapter and device from browser
//! 2. Configure canvas surface for WebGPU rendering
//! 3. Render solid-color shapes via `vsc-gpu::GpuRenderer`
//!
//! ## Architecture
//!
//! ```text
//! TypeScript                 WASM Boundary              Rust
//! ──────────────────────────────────────────────────────────────
//! const renderer =           wasm-bindgen
//!   await WasmGpuRenderer    ─────────────►  WasmGpuRenderer::create()
//!     .create(canvas);                              │
//!                                                   ▼
//! renderer.render(json);     ─────────────►  GpuRenderer::render_frame()
//!        ▲                                          │
//!        │                                          ▼
//!   WebGPU Surface  ◄───────────────────────  wgpu::Surface
//! ```
//!
//! ## Usage (TypeScript)
//!
//! ```typescript
//! import init, { WasmGpuRenderer } from 'vsc-wasm';
//!
//! await init();
//!
//! const canvas = document.getElementById('viewport') as HTMLCanvasElement;
//! const renderer = await WasmGpuRenderer.create(canvas);
//!
//! // Render a red triangle
//! renderer.render(JSON.stringify([{
//!   kind: "path",
//!   entity_id: 1,
//!   bounds: { ... },
//!   z_order: 0,
//!   chunk_id: "main",
//!   path_data: [...],
//!   fill: { type: "solid", color: "#ff0000" },
//! }]));
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, Window};

use vsc_core::target::RenderTarget;
use vsc_core::{
    buildinfo::{TextEntityEntry, VsBuildInfo},
    f32_to_rational_exact,
    ffi::{evaluate_derived, DerivedQVariable, DerivedRule, QSnapshot, QValue, QVariable},
    scene::{evaluate_conditions, SceneBuilder, SceneNode},
    solver::{ConstraintSolver, VarId, VariableState},
    text::{ExpandedText, TextShaper},
    types::{
        ConditionId, EntityId, FillRule, FillSpec, PathCommand, PathEntityEntry, PathSegment,
        PostSolveCondition, Rational, VectorComponent,
    },
};

use crate::ffi_bridge::{FfiArg, FfiManifest, FfiTrigger, PendingFfiCall, TickResult};
use vsc_gpu::WebTarget;

// =============================================================================
// Error Handling
// =============================================================================

/// Convert Rust errors to JavaScript errors.
fn to_js_error<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// Compare two QValues for equality (for derived variable change detection).
fn q_values_equal(a: &QValue, b: &QValue) -> bool {
    match (a, b) {
        (QValue::Float(x), QValue::Float(y)) => (x - y).abs() < 1e-10,
        (QValue::Int(x), QValue::Int(y)) => x == y,
        (QValue::Bool(x), QValue::Bool(y)) => x == y,
        (QValue::Rational(x), QValue::Rational(y)) => x == y,
        (QValue::None, QValue::None) => true,
        _ => false,
    }
}

/// Convert f64 to Rational using IEEE 754 exact conversion.
///
/// This function uses f32_to_rational_exact internally after casting,
/// which preserves the exact binary representation for typical UI coordinates.
fn f64_to_rational(v: f64) -> Rational {
    // For UI coordinates, f32 precision is sufficient and exact conversion works.
    // Values exceeding f32 range will saturate to infinity (caught by f32_to_rational_exact).
    f32_to_rational_exact(v as f32)
}

// =============================================================================
// TextureRegistry (Phase J-3)
// =============================================================================

/// Media type for texture update strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Static image (PNG/JPEG/WebP) - single upload, no updates.
    StaticImage,
    /// Video stream (MP4/WebM) - requires update every frame.
    Video,
    /// Animated image (GIF/APNG) - update on frame change.
    AnimatedImage,
}

/// Entry in the texture registry.
struct TextureEntry {
    /// GPU texture resource.
    texture: wgpu::Texture,
    /// View for shader binding.
    view: wgpu::TextureView,
    /// Texture width in pixels.
    width: u32,
    /// Texture height in pixels.
    height: u32,
    /// Media type for update strategy.
    media_type: MediaType,
}

/// Registry for external textures (images, videos, canvases).
///
/// Maps opaque `u64` IDs to GPU texture resources. The host registers
/// textures via `register_image_texture()` and updates them via
/// `update_texture_pixels()`.
///
/// ## Lifecycle
///
/// ```text
/// JS: register_image_texture(width, height, pixels) → id
/// JS: (optional) update_texture_pixels(id, pixels)
/// JS: remove_texture(id)
/// ```
pub struct TextureRegistry {
    /// Registered textures by ID.
    textures: HashMap<u64, TextureEntry>,
    /// Next available texture ID.
    next_id: u64,
}

impl TextureRegistry {
    /// Create a new empty texture registry.
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            next_id: 1,
        }
    }

    /// Register a new texture with the given dimensions.
    ///
    /// Returns the assigned texture ID.
    pub fn register(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        media_type: MediaType,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("ExternalTexture_{}", id)),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.textures.insert(
            id,
            TextureEntry {
                texture,
                view,
                width,
                height,
                media_type,
            },
        );

        id
    }

    /// Get the texture view for the given ID.
    pub fn get_view(&self, id: u64) -> Option<&wgpu::TextureView> {
        self.textures.get(&id).map(|e| &e.view)
    }

    /// Create a new texture view for the given ID.
    ///
    /// This is used to pass an owned view to `GpuRenderer.set_external_texture()`.
    /// Each call creates a new view of the same underlying texture.
    pub fn create_view_for_renderer(&self, id: u64) -> Option<wgpu::TextureView> {
        self.textures.get(&id).map(|e| {
            e.texture
                .create_view(&wgpu::TextureViewDescriptor::default())
        })
    }

    /// Get texture dimensions for the given ID.
    pub fn get_dimensions(&self, id: u64) -> Option<(u32, u32)> {
        self.textures.get(&id).map(|e| (e.width, e.height))
    }

    /// Update texture pixel data.
    ///
    /// The `data` must be RGBA8 format with length `width * height * 4`.
    /// Returns `true` if the texture was found and updated.
    pub fn update_pixels(&self, queue: &wgpu::Queue, id: u64, data: &[u8]) -> bool {
        if let Some(entry) = self.textures.get(&id) {
            let expected_len = (entry.width * entry.height * 4) as usize;
            if data.len() != expected_len {
                web_sys::console::warn_1(
                    &format!(
                        "TextureRegistry: pixel data length mismatch for id {}. Expected {}, got {}",
                        id, expected_len, data.len()
                    )
                    .into(),
                );
                return false;
            }

            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &entry.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * entry.width),
                    rows_per_image: Some(entry.height),
                },
                wgpu::Extent3d {
                    width: entry.width,
                    height: entry.height,
                    depth_or_array_layers: 1,
                },
            );
            true
        } else {
            false
        }
    }

    /// Remove a texture from the registry.
    ///
    /// Returns `true` if the texture was found and removed.
    pub fn remove(&mut self, id: u64) -> bool {
        self.textures.remove(&id).is_some()
    }

    /// Get the number of registered textures.
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}

impl Default for TextureRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// WasmGpuRenderer
// =============================================================================

/// WebGPU surface manager for browser environment.
///
/// This struct manages the WebGPU surface lifecycle for browser rendering.
/// It does NOT contain a `GpuRenderer` - rendering is delegated to `WebTarget`.
///
/// ## Responsibilities
///
/// - Surface creation and configuration
/// - Surface texture acquisition (`surface_texture_view()`)
/// - Resize handling
/// - Device/queue sharing with `WebTarget`
#[wasm_bindgen]
pub struct WasmGpuRenderer {
    /// wgpu surface bound to the canvas element.
    surface: wgpu::Surface<'static>,
    /// Surface configuration for resize handling.
    surface_config: wgpu::SurfaceConfiguration,
    /// Current canvas width in device pixels.
    width: u32,
    /// Current canvas height in device pixels.
    height: u32,
    /// Shared wgpu device (also used by WebTarget).
    device: Arc<wgpu::Device>,
    /// Shared wgpu queue (also used by WebTarget).
    queue: Arc<wgpu::Queue>,
    /// Surface texture format (needed for WebTarget creation).
    format: wgpu::TextureFormat,
}

#[wasm_bindgen]
impl WasmGpuRenderer {
    /// Create a new WebGPU renderer bound to a canvas element.
    ///
    /// This is an async operation that:
    /// 1. Requests a GPU adapter from the browser
    /// 2. Requests a device and queue from the adapter
    /// 3. Creates a surface from the canvas element
    /// 4. Configures the surface for rendering
    ///
    /// # Arguments
    ///
    /// * `canvas` - The HTML canvas element to render to
    ///
    /// # Returns
    ///
    /// A `Promise` that resolves to `WasmGpuRenderer` or rejects with an error.
    #[wasm_bindgen]
    pub async fn create(canvas: HtmlCanvasElement) -> Result<WasmGpuRenderer, JsValue> {
        // Set up panic hook for better error messages in browser console
        #[cfg(feature = "gpu")]
        console_error_panic_hook::set_once();

        // Get canvas dimensions
        let width = canvas.width();
        let height = canvas.height();

        if width == 0 || height == 0 {
            return Err(JsValue::from_str(
                "Canvas must have non-zero dimensions. Set width/height attributes.",
            ));
        }

        // Create wgpu instance with WebGPU backend
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });

        // Create surface from canvas
        // SAFETY: The canvas element outlives the surface in normal browser usage.
        // The 'static lifetime is required by wgpu's Surface API but the actual
        // lifetime is tied to the WasmGpuRenderer instance.
        #[cfg(target_arch = "wasm32")]
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(to_js_error)?;

        #[cfg(not(target_arch = "wasm32"))]
        let surface: wgpu::Surface<'static> = {
            // This branch is never executed in practice, but allows cargo check
            // on non-wasm targets to succeed. At runtime on wasm32, only the
            // branch above runs.
            return Err(JsValue::from_str(
                "WasmGpuRenderer is only supported on wasm32",
            ));
        };

        // Request GPU adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .ok_or_else(|| JsValue::from_str("Failed to find a suitable GPU adapter"))?;

        // Log adapter info for debugging
        let info = adapter.get_info();
        web_sys::console::log_1(
            &format!(
                "[ViewScript] GPU Adapter: {} ({:?})",
                info.name, info.backend
            )
            .into(),
        );

        // Request device and queue
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("ViewScript Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                        .using_resolution(adapter.limits()),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(to_js_error)?;

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        // Get surface capabilities and configure
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &surface_config);

        web_sys::console::log_1(
            &format!(
                "[ViewScript] Surface initialized: {}x{}, format={:?}",
                width, height, format
            )
            .into(),
        );

        Ok(WasmGpuRenderer {
            surface,
            surface_config,
            width,
            height,
            device,
            queue,
            format,
        })
    }

    /// Resize the rendering surface.
    ///
    /// Call this when the canvas element is resized.
    ///
    /// # Arguments
    ///
    /// * `width` - New width in device pixels
    /// * `height` - New height in device pixels
    #[wasm_bindgen]
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
        if width == 0 || height == 0 {
            return Err(JsValue::from_str("Width and height must be non-zero"));
        }

        self.width = width;
        self.height = height;
        self.surface_config.width = width;
        self.surface_config.height = height;

        self.surface.configure(&self.device, &self.surface_config);

        web_sys::console::log_1(&format!("[ViewScript] Resized to {}x{}", width, height).into());

        Ok(())
    }

    /// Get the current width in device pixels.
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get the current height in device pixels.
    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.height
    }
}

// =============================================================================
// Internal Rust API (not exposed to JS)
// =============================================================================

impl WasmGpuRenderer {
    /// Get the surface texture view for the current frame.
    ///
    /// Returns a tuple of (TextureView, SurfaceTexture) for advanced rendering scenarios.
    /// The caller is responsible for calling `present()` on the SurfaceTexture.
    pub fn surface_texture_view(
        &self,
    ) -> Result<(wgpu::TextureView, wgpu::SurfaceTexture), JsValue> {
        let output = self
            .surface
            .get_current_texture()
            .map_err(|e| JsValue::from_str(&format!("Failed to get surface texture: {}", e)))?;

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        Ok((view, output))
    }

    /// Get a reference to the wgpu device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Get a reference to the wgpu queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Get the device Arc for shared ownership.
    pub fn device_arc(&self) -> Arc<wgpu::Device> {
        Arc::clone(&self.device)
    }

    /// Get the queue Arc for shared ownership.
    pub fn queue_arc(&self) -> Arc<wgpu::Queue> {
        Arc::clone(&self.queue)
    }

    /// Get the surface texture format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }
}

// =============================================================================
// Convenience Functions
// =============================================================================

/// Check if WebGPU is available in the current browser.
///
/// Returns `true` if `navigator.gpu` exists and is not undefined.
#[wasm_bindgen]
pub fn is_webgpu_available() -> bool {
    let window: Window = match web_sys::window() {
        Some(w) => w,
        None => return false,
    };

    let navigator = window.navigator();

    // Check if navigator.gpu exists
    js_sys::Reflect::get(&navigator, &JsValue::from_str("gpu"))
        .map(|gpu| !gpu.is_undefined() && !gpu.is_null())
        .unwrap_or(false)
}

/// Get WebGPU adapter info as a JSON string.
///
/// Returns information about the GPU adapter, or an error if WebGPU is not available.
#[wasm_bindgen]
pub async fn get_adapter_info() -> Result<String, JsValue> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..Default::default()
    });

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await
        .ok_or_else(|| JsValue::from_str("No GPU adapter available"))?;

    let info = adapter.get_info();

    Ok(format!(
        r#"{{"name":"{}","vendor":{},"device":{},"device_type":"{:?}","driver":"{}","driver_info":"{}","backend":"{:?}"}}"#,
        info.name,
        info.vendor,
        info.device,
        info.device_type,
        info.driver,
        info.driver_info,
        info.backend
    ))
}

// =============================================================================
// WasmViewScriptEngine - Full Pipeline Integration (Stage 2)
// =============================================================================

// =============================================================================
// Mutation Types (T-vector state changes from Q-dimension input)
// =============================================================================

/// A mutation representing user input (Q-dimension) affecting component state (T-dimension).
///
/// Mutations are applied each tick to update constraint solver variables,
/// which then propagate through the P-dimension constraint system.
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "type")]
pub enum Mutation {
    /// Translate a component by a delta offset.
    /// All control points belonging to the component are moved by (dx, dy).
    Translate { entity_id: u64, dx: f64, dy: f64 },
    /// Set the absolute position of a component.
    /// The component's origin (typically top-left) is moved to (x, y).
    SetPosition { entity_id: u64, x: f64, y: f64 },
}

/// ViewScript rendering engine for browser environment.
///
/// This struct integrates the full rendering pipeline:
/// - `ConstraintSolver`: Evaluates P-dimension constraints
/// - `VsBuildInfo`: Event-sourcing ledger for constraint operations
/// - `SceneBuilder`: Converts solver output to scene graph
/// - `SceneConverter`: Converts scene graph to canvas nodes with topology rounding
/// - `WasmGpuRenderer`: WebGPU-based rendering to canvas
///
/// ## Architecture
///
/// ```text
/// TypeScript                    WASM Boundary                     Rust
/// ─────────────────────────────────────────────────────────────────────────────
/// const engine =                wasm-bindgen
///   await WasmViewScriptEngine   ─────────────►  WasmViewScriptEngine::create()
///     .create(canvas, dpr);                              │
///                                                        ▼
/// engine.add_component(...)     ─────────────►  add_component() → VsBuildInfo
///                                                        │
///                                                        ▼
/// engine.tick(mutations_json)   ─────────────►  tick()
///                                                │
///                                                ├─► Apply mutations (T-vector)
///                                                ├─► ConstraintSolver.solve()
///                                                ├─► SceneBuilder.build_scene()
///                                                ├─► SceneConverter.convert_with_rounding()
///                                                └─► WasmGpuRenderer.render_nodes()
/// ```
///
/// ## Mutation Format (JSON)
///
/// ```typescript
/// // Translate: move component by delta
/// engine.tick(JSON.stringify([
///   { type: "Translate", entity_id: 1000, dx: 10, dy: 5 }
/// ]));
///
/// // SetPosition: move component to absolute position
/// engine.tick(JSON.stringify([
///   { type: "SetPosition", entity_id: 1000, x: 200, y: 150 }
/// ]));
/// ```
///
/// ## Usage (TypeScript)
///
/// ```typescript
/// import init, { WasmViewScriptEngine } from 'vsc-wasm';
///
/// await init();
///
/// const canvas = document.getElementById('viewport') as HTMLCanvasElement;
/// const engine = await WasmViewScriptEngine.create(canvas, window.devicePixelRatio);
///
/// // Add a rounded rectangle component
/// engine.add_component('RoundedRect', JSON.stringify({
///   x: 100, y: 100, width: 200, height: 150, radius: 20, fill: '#ff0000'
/// }));
///
/// // Animation loop
/// function animate() {
///   engine.tick('[]'); // Empty mutations for static content
///   requestAnimationFrame(animate);
/// }
/// animate();
/// ```
#[wasm_bindgen]
pub struct WasmViewScriptEngine {
    /// P-dimension constraint solver.
    solver: ConstraintSolver,
    /// Event-sourcing ledger for constraint operations.
    build_info: VsBuildInfo,
    /// WebGPU surface manager (low-level API).
    wasm_renderer: WasmGpuRenderer,
    /// Render target abstraction (vs-web).
    target: WebTarget,
    /// Device pixel ratio for coordinate scaling.
    device_pixel_ratio: f64,
    /// Next available entity ID for allocation.
    next_entity_id: u64,
    /// Mapping from component entity ID to its control point entity IDs.
    /// Used for applying mutations to all control points of a component.
    component_control_points: HashMap<EntityId, Vec<EntityId>>,
    /// Cached origin position for each component (for SetPosition mutations).
    /// Stores the (x, y) of the first control point.
    component_origins: HashMap<EntityId, (Rational, Rational)>,
    /// Registry for external textures (images, videos, canvases).
    texture_registry: TextureRegistry,
    /// Post-solve condition state for rising-edge detection (persists across frames).
    prev_satisfied: HashSet<ConditionId>,
    /// Mapping from ConditionId to FFI function ID (for trigger dispatch).
    condition_to_ffi: HashMap<ConditionId, u64>,
    /// Stored trigger definitions for building FFI requests with args.
    ffi_triggers: HashMap<ConditionId, FfiTrigger>,
    /// Font cache: family name → font binary data.
    font_cache: HashMap<String, Vec<u8>>,
    /// Expanded text data: text entity ID → (expanded paths, fill color, position).
    text_expanded: HashMap<EntityId, (ExpandedText, String, Rational, Rational)>,
}

#[wasm_bindgen]
impl WasmViewScriptEngine {
    /// Create a new ViewScript engine bound to a canvas element.
    ///
    /// This is an async operation that initializes the WebGPU pipeline
    /// and sets up the constraint solver.
    ///
    /// # Arguments
    ///
    /// * `canvas` - The HTML canvas element to render to
    /// * `device_pixel_ratio` - DPR for coordinate scaling (e.g., 2.0 for Retina)
    ///
    /// # Returns
    ///
    /// A `Promise` that resolves to `WasmViewScriptEngine` or rejects with an error.
    #[wasm_bindgen]
    pub async fn create(
        canvas: HtmlCanvasElement,
        device_pixel_ratio: f64,
    ) -> Result<WasmViewScriptEngine, JsValue> {
        // Initialize the WebGPU surface manager
        let wasm_renderer = WasmGpuRenderer::create(canvas).await?;

        // Get shared device, queue, and format from wasm_renderer
        // WebTarget will create the single GpuRenderer instance
        let device = wasm_renderer.device_arc();
        let queue = wasm_renderer.queue_arc();
        let format = wasm_renderer.format();
        let width = wasm_renderer.width();
        let height = wasm_renderer.height();

        // Create WebTarget (contains the only GpuRenderer instance)
        let target = WebTarget::new(device, queue, format, width, height, device_pixel_ratio);

        // Initialize build_info with vs-web target
        let mut build_info = VsBuildInfo::default();
        build_info.targets.push("vs-web".to_string());

        // Register standard Q-dimension variables
        // Entity IDs 0-99 reserved for Q-dimension system variables
        let pointer_x_entity = EntityId(0);
        let pointer_y_entity = EntityId(1);
        let pointer_pressed_entity = EntityId(2);
        let viewport_width_entity = EntityId(3);
        let viewport_height_entity = EntityId(4);
        let viewport_dpr_entity = EntityId(5);

        build_info.q_variables.push(QVariable::new(
            "input.pointer.x",
            QValue::Float(0.0),
            VarId::new(pointer_x_entity, VectorComponent::Value),
        ));
        build_info.q_variables.push(QVariable::new(
            "input.pointer.y",
            QValue::Float(0.0),
            VarId::new(pointer_y_entity, VectorComponent::Value),
        ));
        build_info.q_variables.push(QVariable::new(
            "input.pointer.pressed",
            QValue::Bool(false),
            VarId::new(pointer_pressed_entity, VectorComponent::Value),
        ));
        build_info.q_variables.push(QVariable::new(
            "env.viewport.width",
            QValue::Int(width as i64),
            VarId::new(viewport_width_entity, VectorComponent::Value),
        ));
        build_info.q_variables.push(QVariable::new(
            "env.viewport.height",
            QValue::Int(height as i64),
            VarId::new(viewport_height_entity, VectorComponent::Value),
        ));
        build_info.q_variables.push(QVariable::new(
            "env.viewport.dpr",
            QValue::Float(device_pixel_ratio),
            VarId::new(viewport_dpr_entity, VectorComponent::Value),
        ));

        web_sys::console::log_1(
            &format!(
                "[ViewScript] Engine initialized: DPR={}, target=vs-web, q_vars={}",
                device_pixel_ratio,
                build_info.q_variables.len()
            )
            .into(),
        );

        Ok(WasmViewScriptEngine {
            solver: ConstraintSolver::new(),
            build_info,
            wasm_renderer,
            target,
            device_pixel_ratio,
            next_entity_id: 1000, // Reserve 0-999 for system entities
            component_control_points: HashMap::new(),
            component_origins: HashMap::new(),
            texture_registry: TextureRegistry::new(),
            prev_satisfied: HashSet::new(),
            condition_to_ffi: HashMap::new(),
            ffi_triggers: HashMap::new(),
            font_cache: HashMap::new(),
            text_expanded: HashMap::new(),
        })
    }

    /// Load an FFI manifest generated by the Vite plugin.
    ///
    /// This method registers:
    /// - `bindings`: Q-dimension variables bound to JS functions
    /// - `triggers`: Post-solve conditions that fire FFI calls
    ///
    /// # Arguments
    ///
    /// * `manifest_json` - JSON string conforming to FfiManifest schema
    ///
    /// # Example
    ///
    /// ```json
    /// {
    ///   "version": 1,
    ///   "entity_map": { "button": 42, "cursor": 57 },
    ///   "bindings": [
    ///     { "ffi_id": 1, "bind_name": "mouse.x", "module_path": "./input.ts", "export_name": "getMouseX" }
    ///   ],
    ///   "triggers": [
    ///     {
    ///       "trigger_id": 100,
    ///       "ffi_id": 2,
    ///       "condition": { "kind": "bounds_overlap", "entity_a": 42, "entity_b": 57 },
    ///       "args": []
    ///     }
    ///   ]
    /// }
    /// ```
    #[wasm_bindgen(js_name = loadFfiManifest)]
    pub fn load_ffi_manifest(&mut self, manifest_json: &str) -> Result<(), JsValue> {
        let manifest: FfiManifest = serde_json::from_str(manifest_json)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse FFI manifest: {}", e)))?;

        // Note: bindings don't need explicit Q-variable registration.
        // JS side evaluates FFI functions and includes results in QSnapshot.
        // The binding metadata (module_path, export_name) is used by JS runtime,
        // not by the WASM constraint engine.
        //
        // If bindings need to drive T-dimension variables, they should be
        // registered as DerivedQVariables with appropriate rules.
        let _ = &manifest.bindings; // Acknowledge bindings are parsed but not stored here

        // Register triggers as post-solve conditions
        for trigger in &manifest.triggers {
            let condition = PostSolveCondition {
                id: trigger.trigger_id,
                kind: trigger.condition.clone(),
            };
            self.solver.register_condition(condition);

            // Store mapping for FFI dispatch
            self.condition_to_ffi
                .insert(trigger.trigger_id, trigger.ffi_id);
            self.ffi_triggers
                .insert(trigger.trigger_id, trigger.clone());
        }

        Ok(())
    }

    /// Register a font for text rendering.
    ///
    /// The font binary is stored in the engine's font cache. When adding
    /// a Text component, the font_family parameter is used to look up the font.
    ///
    /// # Arguments
    ///
    /// * `family` - Font family name (e.g., "Inter", "Roboto")
    /// * `font_bytes` - Raw font file bytes (TTF, OTF, or TTC)
    ///
    /// # Returns
    ///
    /// `Ok(())` if the font was registered successfully, `Err` if parsing failed.
    #[wasm_bindgen]
    pub fn register_font(&mut self, family: &str, font_bytes: &[u8]) -> Result<(), JsValue> {
        // V6: Prevent replacement if font is in use by active text entities
        if self.font_cache.contains_key(family) {
            let in_use = self
                .build_info
                .text_entities
                .iter()
                .any(|e| e.font_family == family);
            if in_use {
                return Err(JsValue::from_str(&format!(
                    "Cannot replace font '{}': in use by active text entities. \
                     Remove text entities first or use a different family name.",
                    family
                )));
            }
        }

        // Validate font by attempting to parse it
        let _shaper = TextShaper::new(font_bytes).map_err(|e| {
            JsValue::from_str(&format!("Failed to parse font '{}': {:?}", family, e))
        })?;

        // Store the font data (owned copy)
        self.font_cache
            .insert(family.to_string(), font_bytes.to_vec());

        web_sys::console::log_1(
            &format!(
                "[ViewScript] Font '{}' registered ({} bytes)",
                family,
                font_bytes.len()
            )
            .into(),
        );

        Ok(())
    }

    /// Build FFI call requests from triggered condition IDs.
    fn build_ffi_requests(
        &self,
        triggered: &[ConditionId],
        values: &HashMap<VarId, Rational>,
        q_snapshot: &Option<QSnapshot>,
    ) -> Vec<PendingFfiCall> {
        triggered
            .iter()
            .filter_map(|cond_id| {
                let trigger = self.ffi_triggers.get(cond_id)?;
                let ffi_id = self.condition_to_ffi.get(cond_id)?;

                // Resolve arguments
                let args: Vec<serde_json::Value> = trigger
                    .args
                    .iter()
                    .map(|arg| self.resolve_ffi_arg(arg, values, q_snapshot))
                    .collect();

                Some(PendingFfiCall {
                    ffi_id: *ffi_id,
                    args,
                })
            })
            .collect()
    }

    /// Resolve a single FFI argument to a JSON value.
    fn resolve_ffi_arg(
        &self,
        arg: &FfiArg,
        values: &HashMap<VarId, Rational>,
        q_snapshot: &Option<QSnapshot>,
    ) -> serde_json::Value {
        match arg {
            FfiArg::Static { value } => value.clone(),
            FfiArg::QRef { name } => {
                // Look up Q variable value from snapshot
                q_snapshot
                    .as_ref()
                    .and_then(|snap| snap.values.get(name))
                    .map(|qv| match qv {
                        QValue::Float(f) => serde_json::json!(f),
                        QValue::Int(i) => serde_json::json!(i),
                        QValue::Bool(b) => serde_json::json!(b),
                        QValue::Rational(r) => serde_json::json!(r.to_f64_for_rasterization()),
                        QValue::None => serde_json::Value::Null,
                        _ => serde_json::Value::Null,
                    })
                    .unwrap_or(serde_json::Value::Null)
            }
            FfiArg::EntityCoord {
                entity_id,
                component,
            } => {
                let comp = match component.as_str() {
                    "x" => VectorComponent::X,
                    "y" => VectorComponent::Y,
                    "value" => VectorComponent::Value,
                    _ => return serde_json::Value::Null,
                };
                let var_id = VarId::new(EntityId(*entity_id), comp);
                values
                    .get(&var_id)
                    .map(|r| serde_json::json!(r.to_f64_for_rasterization()))
                    .unwrap_or(serde_json::Value::Null)
            }
        }
    }

    /// Execute one frame of the rendering pipeline.
    ///
    /// This method implements the Q→T→P pipeline with derived variable evaluation:
    /// 1. Phase 1: Inject Q-dimension values or apply legacy mutations
    /// 2. Phase 2: solve → derive → re-solve loop (max 2 iterations)
    ///    - Run constraint solver
    ///    - Build scene graph
    ///    - Evaluate derived Q-variables (e.g., hover detection)
    ///    - If derived values changed, re-inject and loop
    /// 3. Render the final scene
    ///
    /// # Arguments
    ///
    /// * `input_json` - Either:
    ///   - New format: `{"values": {"input.pointer.x": {"type": "Float", "value": 100}}}`
    ///   - Legacy format: `[{"type": "Translate", "entity_id": 1, "dx": 10, "dy": 5}]`
    ///
    /// # Returns
    ///
    /// JSON string with `TickResult`:
    /// - `{"pending_ffi_calls": []}` when no triggers fired
    /// - `{"pending_ffi_calls": [{"ffi_id": 2, "args": [...]}]}` when triggers fired
    #[wasm_bindgen]
    pub fn tick(&mut self, input_json: &str) -> Result<String, JsValue> {
        // =================================================================
        // Phase 1: Q-dimension injection / legacy mutation processing
        // =================================================================
        let trimmed = input_json.trim();
        let q_snapshot: Option<QSnapshot>;

        if trimmed.starts_with('{') {
            // New format: QSnapshot (with optional mutations)
            let snapshot: QSnapshot = serde_json::from_str(input_json).map_err(|e| {
                JsValue::from_str(&format!("Failed to parse QSnapshot JSON: {}", e))
            })?;

            // Apply Q→T bindings: inject Q values into solver variables
            for q_var in &self.build_info.q_variables {
                if let Some(q_value) = snapshot.get(&q_var.name) {
                    if let Some(rational) = q_value.to_rational() {
                        self.solver.register_variable(
                            q_var.target_var,
                            VariableState::Resolved { value: rational },
                        );
                    }
                }
            }

            // Process embedded mutations (for combined Q+mutation operations like drag)
            for mutation_value in &snapshot.mutations {
                if let Ok(mutation) = serde_json::from_value::<Mutation>(mutation_value.clone()) {
                    self.apply_mutation(mutation)?;
                }
            }

            q_snapshot = Some(snapshot);
        } else if trimmed.starts_with('[') {
            // Legacy format: Mutation array
            let mutations: Vec<Mutation> = serde_json::from_str(input_json).map_err(|e| {
                JsValue::from_str(&format!("Failed to parse mutations JSON: {}", e))
            })?;

            for mutation in mutations {
                self.apply_mutation(mutation)?;
            }
            q_snapshot = None;
        } else {
            return Err(JsValue::from_str(
                "Invalid input: expected JSON object (QSnapshot) or array (Mutations)",
            ));
        }

        // =================================================================
        // Phase 2: solve → derive → re-solve loop (max 2 iterations)
        // =================================================================
        const MAX_DERIVED_ITERATIONS: usize = 2;
        let mut prev_derived: HashMap<String, QValue> = HashMap::new();
        let mut final_triggered: Vec<ConditionId> = Vec::new();
        let mut final_values: HashMap<VarId, Rational> = HashMap::new();

        for iteration in 0..=MAX_DERIVED_ITERATIONS {
            // 2a: Run constraint solver
            let solve_result = self
                .solver
                .solve()
                .map_err(|e| JsValue::from_str(&format!("Solver error: {:?}", e)))?;

            // 2b: Build scene graph
            // Text glyphs are now registered as PathEntityEntry in add_text(),
            // so SceneBuilder processes them with Topology-Preserving Rounding
            let scene_builder = SceneBuilder::new(&solve_result.values, &self.build_info);
            let scene_nodes = scene_builder
                .build_scene()
                .map_err(|e| JsValue::from_str(&format!("Scene build error: {}", e)))?;

            // 2c: Final iteration → render and exit
            if iteration == MAX_DERIVED_ITERATIONS {
                // Evaluate post-solve conditions (rising-edge detection)
                let (triggered, new_satisfied) = evaluate_conditions(
                    self.solver.conditions(),
                    &solve_result.values,
                    &self.build_info,
                    &self.prev_satisfied,
                );
                self.prev_satisfied = new_satisfied;
                final_triggered = triggered;
                final_values = solve_result.values;

                self.render_scene(&scene_nodes, &prev_derived)?;
                break;
            }

            // 2d: Evaluate derived Q-variables
            // Skip if no derived variables registered
            if self.build_info.derived_q_variables.is_empty() {
                // Evaluate post-solve conditions (rising-edge detection)
                let (triggered, new_satisfied) = evaluate_conditions(
                    self.solver.conditions(),
                    &solve_result.values,
                    &self.build_info,
                    &self.prev_satisfied,
                );
                self.prev_satisfied = new_satisfied;
                final_triggered = triggered;
                final_values = solve_result.values;

                self.render_scene(&scene_nodes, &prev_derived)?;
                break;
            }

            // Collect current Q-values for derived evaluation
            let q_values = self.collect_q_values(&q_snapshot);
            let mut changed = false;

            for derived in &self.build_info.derived_q_variables {
                let new_value = evaluate_derived(&derived.rule, &q_values, &scene_nodes);

                let value_changed = prev_derived
                    .get(&derived.name)
                    .map(|prev| !q_values_equal(prev, &new_value))
                    .unwrap_or(true);

                if value_changed {
                    if let Some(rational) = new_value.to_rational() {
                        self.solver.register_variable(
                            derived.target_var,
                            VariableState::Resolved { value: rational },
                        );
                        changed = true;
                    }
                    prev_derived.insert(derived.name.clone(), new_value);
                }
            }

            // 2e: No change → render and exit (early termination)
            if !changed {
                // Evaluate post-solve conditions (rising-edge detection)
                let (triggered, new_satisfied) = evaluate_conditions(
                    self.solver.conditions(),
                    &solve_result.values,
                    &self.build_info,
                    &self.prev_satisfied,
                );
                self.prev_satisfied = new_satisfied;
                final_triggered = triggered;
                final_values = solve_result.values;

                self.render_scene(&scene_nodes, &prev_derived)?;
                break;
            }
            // Changed → loop continues (re-solve)
        }

        // =================================================================
        // Phase 3: Build FFI call requests from triggered conditions
        // =================================================================
        let pending_calls = self.build_ffi_requests(&final_triggered, &final_values, &q_snapshot);
        let tick_result = TickResult::with_calls(pending_calls);

        tick_result
            .to_json()
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize TickResult: {}", e)))
    }

    /// Render the scene to the WebGPU surface.
    ///
    /// Applies hover visual effects before rendering.
    fn render_scene(
        &mut self,
        scene_nodes: &[SceneNode],
        derived_values: &HashMap<String, QValue>,
    ) -> Result<(), JsValue> {
        // Get texture view from surface
        let (texture_view, surface_texture) = self.wasm_renderer.surface_texture_view()?;

        // Apply hover visual effects
        // Note: This is a temporary approach. Full constraint-based color will be
        // implemented in Phase 17 (color channel constraints).
        let scene_nodes = self.apply_hover_effects(scene_nodes, derived_values);

        // Render frame via WebTarget (RenderTarget abstraction)
        self.target
            .render_to_view(&scene_nodes, &texture_view)
            .map_err(|e| JsValue::from_str(&format!("Render error: {}", e)))?;

        // Present the frame
        surface_texture.present();

        Ok(())
    }

    /// Collect Q-values for derived variable evaluation.
    fn collect_q_values(&self, snapshot: &Option<QSnapshot>) -> HashMap<String, QValue> {
        let mut values = HashMap::new();

        // From QSnapshot if available
        if let Some(snap) = snapshot {
            for (name, value) in &snap.values {
                values.insert(name.clone(), value.clone());
            }
        }

        // Add defaults from registered Q-variables if not in snapshot
        for q_var in &self.build_info.q_variables {
            if !values.contains_key(&q_var.name) {
                values.insert(q_var.name.clone(), q_var.default.clone());
            }
        }

        values
    }

    /// Apply hover visual effects to scene nodes.
    ///
    /// Modifies fill color for hovered entities. This is a temporary visual
    /// effect until constraint-based color channels are implemented.
    fn apply_hover_effects(
        &self,
        scene_nodes: &[SceneNode],
        derived_values: &HashMap<String, QValue>,
    ) -> Vec<SceneNode> {
        let mut nodes = scene_nodes.to_vec();

        for derived in &self.build_info.derived_q_variables {
            // Currently only HitTest rule exists, but match for future extensibility
            match &derived.rule {
                DerivedRule::HitTest { entity_id, .. } => {
                    let is_hovered = derived_values
                        .get(&derived.name)
                        .and_then(|v| v.to_rational())
                        .map(|r| r > Rational::zero())
                        .unwrap_or(false);

                    if is_hovered {
                        Self::modify_node_fill(&mut nodes, *entity_id);
                    }
                }
            }
        }

        nodes
    }

    /// Modify the fill color of a node to indicate hover state.
    fn modify_node_fill(nodes: &mut [SceneNode], target_entity: EntityId) {
        use vsc_core::scene::SceneFillStyle;

        for node in nodes.iter_mut() {
            match node {
                SceneNode::Path(path) => {
                    if path.entity_id == target_entity {
                        // Change to hover color (light blue)
                        if let Some(ref mut fill) = path.fill {
                            *fill = SceneFillStyle::Solid {
                                color: "#64c8ff".to_string(), // Light blue hover color
                            };
                        }
                    }
                }
                SceneNode::Group(group) => {
                    // Groups don't have fill, but recurse into children
                    Self::modify_node_fill(&mut group.children, target_entity);
                }
            }
        }
    }

    /// Add a component to the scene.
    ///
    /// This method creates the necessary path entities and control points
    /// in the VsBuildInfo ledger based on the component type.
    ///
    /// # Arguments
    ///
    /// * `component_type` - Type of component: "RoundedRect", "Circle", "Path", etc.
    /// * `params_json` - JSON object with component parameters
    ///
    /// # Example JSON for RoundedRect
    ///
    /// ```json
    /// {
    ///   "x": 100,
    ///   "y": 100,
    ///   "width": 200,
    ///   "height": 150,
    ///   "radius": 20,
    ///   "fill": "#ff0000"
    /// }
    /// ```
    ///
    /// # Returns
    ///
    /// The entity ID of the created component.
    #[wasm_bindgen]
    pub fn add_component(
        &mut self,
        component_type: &str,
        params_json: &str,
    ) -> Result<u64, JsValue> {
        // Auto-register vs-web target if not present
        if !self.build_info.targets.contains(&"vs-web".to_string()) {
            self.build_info.targets.push("vs-web".to_string());
        }

        match component_type {
            "RoundedRect" => self.add_rounded_rect(params_json),
            "Circle" => self.add_circle(params_json),
            "Rect" => self.add_rect(params_json),
            "Path" => self.add_path(params_json),
            "Text" => self.add_text(params_json),
            _ => Err(JsValue::from_str(&format!(
                "Unknown component type: {}",
                component_type
            ))),
        }
    }

    /// Get the current canvas width in device pixels.
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 {
        self.wasm_renderer.width()
    }

    /// Get the current canvas height in device pixels.
    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.wasm_renderer.height()
    }

    /// Resize the rendering surface.
    ///
    /// # Arguments
    ///
    /// * `width` - New width in device pixels
    /// * `height` - New height in device pixels
    #[wasm_bindgen]
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
        self.wasm_renderer.resize(width, height)?;
        self.target.resize(width, height);
        Ok(())
    }

    // =========================================================================
    // Texture Registry API (Phase J-3)
    // =========================================================================

    /// Register a static image texture.
    ///
    /// The pixel data must be in RGBA8 format with length `width * height * 4`.
    /// Returns the assigned texture ID for use with `FillSpec::ExternalTexture`.
    ///
    /// # Arguments
    ///
    /// * `width` - Texture width in pixels
    /// * `height` - Texture height in pixels
    /// * `pixels` - RGBA8 pixel data
    ///
    /// # Returns
    ///
    /// A unique texture ID that can be used in `FillSpec::ExternalTexture.handle_name`
    /// as `"resource.texture.<id>"`.
    #[wasm_bindgen]
    pub fn register_image_texture(&mut self, width: u32, height: u32, pixels: &[u8]) -> u64 {
        let id = self.texture_registry.register(
            self.wasm_renderer.device(),
            width,
            height,
            MediaType::StaticImage,
        );
        self.texture_registry
            .update_pixels(self.wasm_renderer.queue(), id, pixels);

        // Sync texture view to WebTarget's GpuRenderer for rendering
        // IMPORTANT: WebTarget has its own GpuRenderer, which is used for actual rendering.
        // wasm_renderer.gpu_renderer is NOT used for rendering in WasmViewScriptEngine.
        if let Some(view) = self.texture_registry.create_view_for_renderer(id) {
            self.target
                .gpu_renderer_mut()
                .set_external_texture(id, view);
        }

        web_sys::console::log_1(
            &format!(
                "[ViewScript] Registered texture id={} ({}x{}, {} bytes)",
                id,
                width,
                height,
                pixels.len()
            )
            .into(),
        );

        id
    }

    /// Register a video texture (requires per-frame updates).
    ///
    /// Unlike `register_image_texture`, this does not upload pixel data immediately.
    /// Call `update_texture_pixels()` each frame to update the video frame.
    ///
    /// # Arguments
    ///
    /// * `width` - Video frame width in pixels
    /// * `height` - Video frame height in pixels
    ///
    /// # Returns
    ///
    /// A unique texture ID for use with `update_texture_pixels()`.
    #[wasm_bindgen]
    pub fn register_video_texture(&mut self, width: u32, height: u32) -> u64 {
        let id = self.texture_registry.register(
            self.wasm_renderer.device(),
            width,
            height,
            MediaType::Video,
        );

        // Sync texture view to WebTarget's GpuRenderer for rendering
        if let Some(view) = self.texture_registry.create_view_for_renderer(id) {
            self.target
                .gpu_renderer_mut()
                .set_external_texture(id, view);
        }

        web_sys::console::log_1(
            &format!(
                "[ViewScript] Registered video texture id={} ({}x{})",
                id, width, height
            )
            .into(),
        );

        id
    }

    /// Update texture pixel data.
    ///
    /// Use this for video textures (every frame) or animated images (on frame change).
    /// The pixel data must be RGBA8 format with length `width * height * 4`.
    ///
    /// # Arguments
    ///
    /// * `texture_id` - ID returned by `register_*_texture()`
    /// * `pixels` - RGBA8 pixel data
    ///
    /// # Returns
    ///
    /// `true` if the texture was found and updated, `false` otherwise.
    #[wasm_bindgen]
    pub fn update_texture_pixels(&mut self, texture_id: u64, pixels: &[u8]) -> bool {
        self.texture_registry
            .update_pixels(self.wasm_renderer.queue(), texture_id, pixels)
    }

    /// Remove a texture from the registry.
    ///
    /// Frees the GPU resources associated with the texture.
    ///
    /// # Arguments
    ///
    /// * `texture_id` - ID returned by `register_*_texture()`
    ///
    /// # Returns
    ///
    /// `true` if the texture was found and removed, `false` otherwise.
    #[wasm_bindgen]
    pub fn remove_texture(&mut self, texture_id: u64) -> bool {
        let result = self.texture_registry.remove(texture_id);
        if result {
            // Also remove from WebTarget's GpuRenderer
            self.target
                .gpu_renderer_mut()
                .remove_external_texture(texture_id);
            web_sys::console::log_1(
                &format!("[ViewScript] Removed texture id={}", texture_id).into(),
            );
        }
        result
    }

    /// Get the number of registered textures.
    #[wasm_bindgen]
    pub fn texture_count(&self) -> usize {
        self.texture_registry.len()
    }

    /// Set a component's fill to an external texture.
    ///
    /// This modifies the `PathEntityEntry` in `build_info.path_entities` to use
    /// `FillSpec::ExternalTexture` with the specified texture ID.
    ///
    /// # Arguments
    ///
    /// * `component_entity_id` - Entity ID of the component (path) to modify
    /// * `texture_id` - Texture ID returned by `register_image_texture()`
    ///
    /// # Returns
    ///
    /// `Ok(())` if the component was found and updated, `Err` otherwise.
    #[wasm_bindgen]
    pub fn set_component_fill_texture(
        &mut self,
        component_entity_id: u64,
        texture_id: u64,
    ) -> Result<(), JsValue> {
        let target_id = EntityId(component_entity_id);

        // Find the path entity with matching ID
        let path_entry = self
            .build_info
            .path_entities
            .iter_mut()
            .find(|entry| entry.id == target_id);

        if let Some(entry) = path_entry {
            // Set fill to ExternalTexture
            entry.fill = Some(FillSpec::ExternalTexture {
                handle_name: format!("resource.texture.{}", texture_id),
                uv_transform: None,
            });
            web_sys::console::log_1(
                &format!(
                    "[ViewScript] Component {} fill set to texture {}",
                    component_entity_id, texture_id
                )
                .into(),
            );
            Ok(())
        } else {
            Err(JsValue::from_str(&format!(
                "Component with entity_id {} not found",
                component_entity_id
            )))
        }
    }

    /// Update text content for an existing text entity.
    ///
    /// Re-expands the text to paths and updates the stored data.
    /// Removes old glyph PathEntityEntries and registers new ones.
    ///
    /// # Arguments
    ///
    /// * `entity_id` - The text entity ID returned by `add_component('Text', ...)`
    /// * `content` - The new text content to display
    ///
    /// # Example
    ///
    /// ```javascript
    /// const labelId = engine.add_component('Text', JSON.stringify({
    ///   x: 100, y: 100, content: '0', font_family: 'Inter', font_size: 48
    /// }));
    /// // Later, update the content:
    /// engine.update_text_content(labelId, '42');
    /// ```
    #[wasm_bindgen]
    pub fn update_text_content(&mut self, entity_id: u64, content: &str) -> Result<(), JsValue> {
        self.update_text_content_internal(entity_id, content)
    }
}

// =============================================================================
// Mutation Application (internal)
// =============================================================================

impl WasmViewScriptEngine {
    /// Apply a single mutation to the solver state.
    fn apply_mutation(&mut self, mutation: Mutation) -> Result<(), JsValue> {
        match mutation {
            Mutation::Translate { entity_id, dx, dy } => {
                let entity_id = EntityId(entity_id);
                let control_points = self.component_control_points.get(&entity_id).cloned();

                if let Some(points) = control_points {
                    let dx_rat = f64_to_rational(dx);
                    let dy_rat = f64_to_rational(dy);

                    for point_id in points {
                        // Get current X value and add delta
                        let var_x = VarId::new(point_id, VectorComponent::X);
                        if let Some(current_x) = self.solver.get_value(&var_x) {
                            let new_x = current_x + dx_rat.clone();
                            self.solver
                                .register_variable(var_x, VariableState::Resolved { value: new_x });
                        }

                        // Get current Y value and add delta
                        let var_y = VarId::new(point_id, VectorComponent::Y);
                        if let Some(current_y) = self.solver.get_value(&var_y) {
                            let new_y = current_y + dy_rat.clone();
                            self.solver
                                .register_variable(var_y, VariableState::Resolved { value: new_y });
                        }
                    }

                    // Update cached origin
                    if let Some((ox, oy)) = self.component_origins.get(&entity_id).cloned() {
                        self.component_origins
                            .insert(entity_id, (ox + dx_rat, oy + dy_rat));
                    }
                }
                Ok(())
            }

            Mutation::SetPosition { entity_id, x, y } => {
                let entity_id = EntityId(entity_id);

                // Get current origin to compute delta
                if let Some((ox, oy)) = self.component_origins.get(&entity_id).cloned() {
                    let target_x = f64_to_rational(x);
                    let target_y = f64_to_rational(y);

                    // Compute delta from current origin to target
                    let dx_rat = target_x.clone() - ox;
                    let dy_rat = target_y.clone() - oy;

                    // Apply delta to all control points
                    if let Some(points) = self.component_control_points.get(&entity_id).cloned() {
                        for point_id in points {
                            let var_x = VarId::new(point_id, VectorComponent::X);
                            if let Some(current_x) = self.solver.get_value(&var_x) {
                                let new_x = current_x + dx_rat.clone();
                                self.solver.register_variable(
                                    var_x,
                                    VariableState::Resolved { value: new_x },
                                );
                            }

                            let var_y = VarId::new(point_id, VectorComponent::Y);
                            if let Some(current_y) = self.solver.get_value(&var_y) {
                                let new_y = current_y + dy_rat.clone();
                                self.solver.register_variable(
                                    var_y,
                                    VariableState::Resolved { value: new_y },
                                );
                            }
                        }
                    }

                    // Update cached origin
                    self.component_origins
                        .insert(entity_id, (target_x, target_y));
                }
                Ok(())
            }
        }
    }
}

// =============================================================================
// Component Factory Methods (internal)
// =============================================================================

impl WasmViewScriptEngine {
    /// Allocate a new entity ID.
    fn allocate_entity_id(&mut self) -> EntityId {
        let id = self.next_entity_id;
        self.next_entity_id += 1;
        EntityId(id)
    }

    /// Register a hover-detecting derived Q-variable for a component.
    ///
    /// This creates a T.hover variable that is 1.0 when the pointer is inside
    /// the component's bounding box, and 0.0 otherwise.
    fn register_hover_variable(&mut self, component_id: EntityId) {
        // Allocate entity ID for the hover variable
        let hover_entity = self.allocate_entity_id();

        // Initialize hover state to 0 (not hovered)
        self.solver.register_variable(
            VarId::new(hover_entity, VectorComponent::Value),
            VariableState::Resolved {
                value: Rational::from_int(0),
            },
        );

        // Register derived Q-variable for hover detection
        self.build_info
            .derived_q_variables
            .push(DerivedQVariable::hover(
                component_id.0,
                VarId::new(hover_entity, VectorComponent::Value),
                component_id,
            ));
    }

    /// Add a rounded rectangle component.
    fn add_rounded_rect(&mut self, params_json: &str) -> Result<u64, JsValue> {
        #[derive(serde::Deserialize)]
        struct RoundedRectParams {
            x: f64,
            y: f64,
            width: f64,
            height: f64,
            #[serde(default)]
            radius: f64,
            #[serde(default = "default_fill")]
            fill: String,
        }

        fn default_fill() -> String {
            "#808080".to_string() // Default gray fill
        }

        let params: RoundedRectParams = serde_json::from_str(params_json).map_err(|e| {
            JsValue::from_str(&format!("Failed to parse RoundedRect params: {}", e))
        })?;

        // Allocate entity IDs for control points
        let path_id = self.allocate_entity_id();
        let tl = self.allocate_entity_id(); // top-left
        let tr = self.allocate_entity_id(); // top-right
        let br = self.allocate_entity_id(); // bottom-right
        let bl = self.allocate_entity_id(); // bottom-left

        // Register control points for this component
        self.component_control_points
            .insert(path_id, vec![tl, tr, br, bl]);

        // Set control point coordinates in solver
        let x = f64_to_rational(params.x);
        let y = f64_to_rational(params.y);
        let w = f64_to_rational(params.width);
        let h = f64_to_rational(params.height);
        let _r = f64_to_rational(params.radius);

        // Cache origin position for SetPosition mutations
        self.component_origins
            .insert(path_id, (x.clone(), y.clone()));

        // Top-left corner
        self.solver.register_variable(
            VarId::new(tl, VectorComponent::X),
            VariableState::Resolved { value: x.clone() },
        );
        self.solver.register_variable(
            VarId::new(tl, VectorComponent::Y),
            VariableState::Resolved { value: y.clone() },
        );

        // Top-right corner
        self.solver.register_variable(
            VarId::new(tr, VectorComponent::X),
            VariableState::Resolved {
                value: x.clone() + w.clone(),
            },
        );
        self.solver.register_variable(
            VarId::new(tr, VectorComponent::Y),
            VariableState::Resolved { value: y.clone() },
        );

        // Bottom-right corner
        self.solver.register_variable(
            VarId::new(br, VectorComponent::X),
            VariableState::Resolved {
                value: x.clone() + w.clone(),
            },
        );
        self.solver.register_variable(
            VarId::new(br, VectorComponent::Y),
            VariableState::Resolved {
                value: y.clone() + h.clone(),
            },
        );

        // Bottom-left corner
        self.solver.register_variable(
            VarId::new(bl, VectorComponent::X),
            VariableState::Resolved { value: x.clone() },
        );
        self.solver.register_variable(
            VarId::new(bl, VectorComponent::Y),
            VariableState::Resolved {
                value: y.clone() + h.clone(),
            },
        );

        // Create path segments for rounded rectangle
        // If radius is 0, create a simple rectangle
        let segments = if params.radius <= 0.0 {
            vec![
                PathSegment::Line { from: tl, to: tr },
                PathSegment::Line { from: tr, to: br },
                PathSegment::Line { from: br, to: bl },
                PathSegment::Line { from: bl, to: tl },
            ]
        } else {
            // For rounded corners, we need additional control points
            // Simplified: use lines for now, arc support coming in future phase
            // TODO: Add QuadBezier/Arc segments for actual rounded corners
            vec![
                PathSegment::Line { from: tl, to: tr },
                PathSegment::Line { from: tr, to: br },
                PathSegment::Line { from: br, to: bl },
                PathSegment::Line { from: bl, to: tl },
            ]
        };

        // Add path entity to build_info
        self.build_info.path_entities.push(PathEntityEntry {
            id: path_id,
            segments,
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: Some(FillSpec::Solid { color: params.fill }),
            stroke: None,
        });

        // Register hover detection for this component
        self.register_hover_variable(path_id);

        Ok(path_id.0)
    }

    /// Add a circle component.
    fn add_circle(&mut self, params_json: &str) -> Result<u64, JsValue> {
        #[derive(serde::Deserialize)]
        struct CircleParams {
            cx: f64,
            cy: f64,
            r: f64,
            #[serde(default = "default_fill")]
            fill: String,
        }

        fn default_fill() -> String {
            "#808080".to_string()
        }

        let params: CircleParams = serde_json::from_str(params_json)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse Circle params: {}", e)))?;

        // For a circle, we approximate with a polygon (16 sides)
        // Full arc support is deferred to a future phase
        let path_id = self.allocate_entity_id();
        let num_sides = 16;
        let mut control_points = Vec::with_capacity(num_sides);

        for i in 0..num_sides {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (num_sides as f64);
            let px = params.cx + params.r * angle.cos();
            let py = params.cy + params.r * angle.sin();

            let point_id = self.allocate_entity_id();
            self.solver.register_variable(
                VarId::new(point_id, VectorComponent::X),
                VariableState::Resolved {
                    value: f64_to_rational(px),
                },
            );
            self.solver.register_variable(
                VarId::new(point_id, VectorComponent::Y),
                VariableState::Resolved {
                    value: f64_to_rational(py),
                },
            );
            control_points.push(point_id);
        }

        // Register control points for this component
        self.component_control_points
            .insert(path_id, control_points.clone());
        // Cache origin as center of circle
        self.component_origins.insert(
            path_id,
            (f64_to_rational(params.cx), f64_to_rational(params.cy)),
        );

        // Create line segments forming polygon
        let mut segments = Vec::with_capacity(num_sides);
        for i in 0..num_sides {
            let from = control_points[i];
            let to = control_points[(i + 1) % num_sides];
            segments.push(PathSegment::Line { from, to });
        }

        self.build_info.path_entities.push(PathEntityEntry {
            id: path_id,
            segments,
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: Some(FillSpec::Solid { color: params.fill }),
            stroke: None,
        });

        // Register hover detection for this component
        self.register_hover_variable(path_id);

        Ok(path_id.0)
    }

    /// Add a simple rectangle component.
    fn add_rect(&mut self, params_json: &str) -> Result<u64, JsValue> {
        // Reuse RoundedRect with radius=0
        #[derive(serde::Deserialize)]
        struct RectParams {
            x: f64,
            y: f64,
            width: f64,
            height: f64,
            #[serde(default = "default_fill")]
            fill: String,
        }

        fn default_fill() -> String {
            "#808080".to_string()
        }

        let params: RectParams = serde_json::from_str(params_json)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse Rect params: {}", e)))?;

        let rounded_params = format!(
            r#"{{"x":{},"y":{},"width":{},"height":{},"radius":0,"fill":"{}"}}"#,
            params.x, params.y, params.width, params.height, params.fill
        );

        self.add_rounded_rect(&rounded_params)
    }

    /// Add a custom path component.
    fn add_path(&mut self, params_json: &str) -> Result<u64, JsValue> {
        #[derive(serde::Deserialize)]
        struct PathParams {
            points: Vec<(f64, f64)>,
            closed: Option<bool>,
            #[serde(default = "default_fill")]
            fill: Option<String>,
        }

        fn default_fill() -> Option<String> {
            Some("#808080".to_string())
        }

        let params: PathParams = serde_json::from_str(params_json)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse Path params: {}", e)))?;

        if params.points.len() < 2 {
            return Err(JsValue::from_str("Path must have at least 2 points"));
        }

        let path_id = self.allocate_entity_id();
        let mut control_points = Vec::with_capacity(params.points.len());

        for (px, py) in &params.points {
            let point_id = self.allocate_entity_id();
            self.solver.register_variable(
                VarId::new(point_id, VectorComponent::X),
                VariableState::Resolved {
                    value: f64_to_rational(*px),
                },
            );
            self.solver.register_variable(
                VarId::new(point_id, VectorComponent::Y),
                VariableState::Resolved {
                    value: f64_to_rational(*py),
                },
            );
            control_points.push(point_id);
        }

        // Register control points for this component
        self.component_control_points
            .insert(path_id, control_points.clone());
        // Cache origin as first point
        if let Some(&(px, py)) = params.points.first() {
            self.component_origins
                .insert(path_id, (f64_to_rational(px), f64_to_rational(py)));
        }

        // Create line segments
        let mut segments = Vec::with_capacity(params.points.len());
        for i in 0..params.points.len() - 1 {
            segments.push(PathSegment::Line {
                from: control_points[i],
                to: control_points[i + 1],
            });
        }

        // Close path if requested
        let closed = params.closed.unwrap_or(false);
        if closed && params.points.len() > 2 {
            segments.push(PathSegment::Line {
                from: *control_points.last().unwrap(),
                to: control_points[0],
            });
        }

        let fill = params.fill.map(|c| FillSpec::Solid { color: c });

        self.build_info.path_entities.push(PathEntityEntry {
            id: path_id,
            segments,
            closed,
            fill_rule: FillRule::NonZero,
            fill,
            stroke: None,
        });

        // Register hover detection for this component
        self.register_hover_variable(path_id);

        Ok(path_id.0)
    }

    /// Add a text component.
    ///
    /// Renders text as vector paths using the registered font.
    /// The text is expanded to glyph outlines at creation time.
    ///
    /// # JSON Parameters
    ///
    /// ```json
    /// {
    ///   "x": 100,
    ///   "y": 100,
    ///   "content": "Hello",
    ///   "font_family": "Inter",
    ///   "font_size": 48,
    ///   "fill": "#ffffff"
    /// }
    /// ```
    fn add_text(&mut self, params_json: &str) -> Result<u64, JsValue> {
        #[derive(serde::Deserialize)]
        struct TextParams {
            x: f64,
            y: f64,
            content: String,
            #[serde(default = "default_font_family")]
            font_family: String,
            #[serde(default = "default_font_size")]
            font_size: f64,
            #[serde(default = "default_text_fill")]
            fill: String,
        }

        fn default_font_family() -> String {
            "sans-serif".to_string()
        }
        fn default_font_size() -> f64 {
            16.0
        }
        fn default_text_fill() -> String {
            "#ffffff".to_string()
        }

        let params: TextParams = serde_json::from_str(params_json)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse Text params: {}", e)))?;

        // Get font data from cache (clone to release borrow before mutable operations)
        let font_data = self
            .font_cache
            .get(&params.font_family)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Font '{}' not registered. Call register_font() first.",
                    params.font_family
                ))
            })?
            .clone();

        // Allocate text entity ID (requires &mut self)
        let text_id = self.allocate_entity_id();

        // Create text shaper (now safe since font_data is owned)
        let shaper = TextShaper::new(&font_data)
            .map_err(|e| JsValue::from_str(&format!("Failed to create text shaper: {:?}", e)))?;

        // Expand text to paths
        let font_size = f64_to_rational(params.font_size);
        let expanded = shaper
            .expand_to_paths(&params.content, &font_size, self.next_entity_id)
            .map_err(|e| JsValue::from_str(&format!("Text shaping failed: {:?}", e)))?;

        // Update next_entity_id based on allocated prototypes and instances
        let allocated_count = expanded.prototypes.len() + expanded.instances.len();
        self.next_entity_id += allocated_count as u64;

        // Calculate bounding box from expanded text
        let (width, height) = self.calculate_text_bounds(&expanded);

        // Store position
        let x = f64_to_rational(params.x);
        let y = f64_to_rational(params.y);

        // Allocate corner control points
        let tl = self.allocate_entity_id();
        let tr = self.allocate_entity_id();
        let bl = self.allocate_entity_id();
        let br = self.allocate_entity_id();

        // Register corner control points in solver
        // TL (top-left)
        self.solver.register_variable(
            VarId::new(tl, VectorComponent::X),
            VariableState::Resolved { value: x.clone() },
        );
        self.solver.register_variable(
            VarId::new(tl, VectorComponent::Y),
            VariableState::Resolved { value: y.clone() },
        );

        // TR (top-right)
        self.solver.register_variable(
            VarId::new(tr, VectorComponent::X),
            VariableState::Resolved {
                value: x.clone() + width.clone(),
            },
        );
        self.solver.register_variable(
            VarId::new(tr, VectorComponent::Y),
            VariableState::Resolved { value: y.clone() },
        );

        // BL (bottom-left)
        self.solver.register_variable(
            VarId::new(bl, VectorComponent::X),
            VariableState::Resolved { value: x.clone() },
        );
        self.solver.register_variable(
            VarId::new(bl, VectorComponent::Y),
            VariableState::Resolved {
                value: y.clone() + height.clone(),
            },
        );

        // BR (bottom-right)
        self.solver.register_variable(
            VarId::new(br, VectorComponent::X),
            VariableState::Resolved {
                value: x.clone() + width.clone(),
            },
        );
        self.solver.register_variable(
            VarId::new(br, VectorComponent::Y),
            VariableState::Resolved {
                value: y.clone() + height.clone(),
            },
        );

        // Store in text_entities
        self.build_info.text_entities.push(TextEntityEntry {
            id: text_id,
            content: params.content.clone(),
            font_family: params.font_family.clone(),
            font_size: font_size.clone(),
            corner_tl: tl,
            corner_tr: tr,
            corner_bl: bl,
            corner_br: br,
            metrics_resolved: true,
            measured_width: Some(width),
            measured_height: Some(height),
            created_at: String::new(),
        });

        // Register control points for component
        self.component_control_points
            .insert(text_id, vec![tl, tr, bl, br]);
        self.component_origins
            .insert(text_id, (x.clone(), y.clone()));

        // Convert glyph paths to PathEntityEntry and register in build_info.path_entities
        // This ensures text goes through SceneBuilder for Topology-Preserving Rounding
        let fill_spec = FillSpec::Solid {
            color: params.fill.clone(),
        };
        self.register_glyph_paths(&expanded, &x, &y, &fill_spec)?;

        // Store expanded text for update_text_content (no longer used for rendering)
        self.text_expanded
            .insert(text_id, (expanded, params.fill, x, y));

        web_sys::console::log_1(
            &format!(
                "[ViewScript] Text '{}' added (id={}, font={})",
                params.content, text_id.0, params.font_family
            )
            .into(),
        );

        Ok(text_id.0)
    }

    /// Register glyph paths as PathEntityEntry in build_info.path_entities.
    ///
    /// This converts each glyph instance's PathCommands to PathSegments by:
    /// 1. Transforming coordinates (scale + translate)
    /// 2. Registering each vertex as a control point in the solver
    /// 3. Creating PathSegments that reference these control points
    /// 4. Adding PathEntityEntry to build_info.path_entities
    ///
    /// By going through path_entities, SceneBuilder applies Topology-Preserving
    /// Rounding (D-19) to text glyphs.
    fn register_glyph_paths(
        &mut self,
        expanded: &ExpandedText,
        origin_x: &Rational,
        origin_y: &Rational,
        fill_spec: &FillSpec,
    ) -> Result<(), JsValue> {
        for instance in &expanded.instances {
            // Find prototype for this instance
            let prototype = expanded
                .prototypes
                .values()
                .find(|p| p.entity_id == instance.prototype_id);

            let Some(prototype) = prototype else {
                continue;
            };

            // Skip glyphs without outlines (space, etc.)
            if prototype.path_commands.is_empty() {
                continue;
            }

            // Calculate transform for this instance
            let scale = &expanded.scale_factor;
            let tx = origin_x.clone() + instance.origin.0.clone();
            let ty = origin_y.clone() + instance.origin.1.clone();

            // Convert PathCommands to PathSegments
            let (segments, closed) =
                self.path_commands_to_segments(&prototype.path_commands, scale, &tx, &ty)?;

            if segments.is_empty() {
                continue;
            }

            // Create PathEntityEntry
            let path_entry = PathEntityEntry {
                id: instance.entity_id,
                segments,
                closed,
                fill_rule: FillRule::NonZero,
                fill: Some(fill_spec.clone()),
                stroke: None,
            };

            self.build_info.path_entities.push(path_entry);
        }

        Ok(())
    }

    /// Convert a sequence of PathCommands to PathSegments.
    ///
    /// Registers each vertex as a control point in the solver and returns
    /// PathSegments that reference these EntityIds.
    fn path_commands_to_segments(
        &mut self,
        commands: &[PathCommand],
        scale: &Rational,
        tx: &Rational,
        ty: &Rational,
    ) -> Result<(Vec<PathSegment>, bool), JsValue> {
        let mut segments = Vec::new();
        let mut current_point: Option<EntityId> = None;
        let mut first_point: Option<EntityId> = None;
        let mut closed = false;

        for cmd in commands {
            match cmd {
                PathCommand::MoveTo { x, y } => {
                    // Transform and register the point
                    let px = x.clone() * scale.clone() + tx.clone();
                    let py = y.clone() * scale.clone() + ty.clone();
                    let point_id = self.register_control_point(&px, &py);
                    current_point = Some(point_id);
                    if first_point.is_none() {
                        first_point = Some(point_id);
                    }
                }
                PathCommand::LineTo { x, y } => {
                    let Some(from_id) = current_point else {
                        continue;
                    };
                    let px = x.clone() * scale.clone() + tx.clone();
                    let py = y.clone() * scale.clone() + ty.clone();
                    let to_id = self.register_control_point(&px, &py);
                    segments.push(PathSegment::Line {
                        from: from_id,
                        to: to_id,
                    });
                    current_point = Some(to_id);
                }
                PathCommand::QuadTo { x1, y1, x, y } => {
                    let Some(from_id) = current_point else {
                        continue;
                    };
                    let hx = x1.clone() * scale.clone() + tx.clone();
                    let hy = y1.clone() * scale.clone() + ty.clone();
                    let handle_id = self.register_control_point(&hx, &hy);
                    let px = x.clone() * scale.clone() + tx.clone();
                    let py = y.clone() * scale.clone() + ty.clone();
                    let to_id = self.register_control_point(&px, &py);
                    segments.push(PathSegment::Quad {
                        from: from_id,
                        handle: handle_id,
                        to: to_id,
                    });
                    current_point = Some(to_id);
                }
                PathCommand::CubicTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => {
                    let Some(from_id) = current_point else {
                        continue;
                    };
                    let h1x = x1.clone() * scale.clone() + tx.clone();
                    let h1y = y1.clone() * scale.clone() + ty.clone();
                    let handle1_id = self.register_control_point(&h1x, &h1y);
                    let h2x = x2.clone() * scale.clone() + tx.clone();
                    let h2y = y2.clone() * scale.clone() + ty.clone();
                    let handle2_id = self.register_control_point(&h2x, &h2y);
                    let px = x.clone() * scale.clone() + tx.clone();
                    let py = y.clone() * scale.clone() + ty.clone();
                    let to_id = self.register_control_point(&px, &py);
                    segments.push(PathSegment::Cubic {
                        from: from_id,
                        handle1: handle1_id,
                        handle2: handle2_id,
                        to: to_id,
                    });
                    current_point = Some(to_id);
                }
                PathCommand::ArcTo {
                    rx,
                    ry,
                    rotation,
                    large_arc,
                    sweep,
                    x,
                    y,
                } => {
                    let Some(from_id) = current_point else {
                        continue;
                    };
                    let px = x.clone() * scale.clone() + tx.clone();
                    let py = y.clone() * scale.clone() + ty.clone();
                    let to_id = self.register_control_point(&px, &py);
                    segments.push(PathSegment::Arc {
                        from: from_id,
                        to: to_id,
                        rx: rx.clone() * scale.clone(),
                        ry: ry.clone() * scale.clone(),
                        rotation: *rotation,
                        large_arc: *large_arc,
                        sweep: *sweep,
                    });
                    current_point = Some(to_id);
                }
                PathCommand::Close => {
                    closed = true;
                    // Close implicitly draws back to first point (no explicit segment needed)
                }
            }
        }

        Ok((segments, closed))
    }

    /// Register a control point with the given coordinates.
    ///
    /// Allocates an EntityId and registers X/Y as Resolved in the solver.
    fn register_control_point(&mut self, x: &Rational, y: &Rational) -> EntityId {
        let id = self.allocate_entity_id();
        self.solver.register_variable(
            VarId::new(id, VectorComponent::X),
            VariableState::Resolved { value: x.clone() },
        );
        self.solver.register_variable(
            VarId::new(id, VectorComponent::Y),
            VariableState::Resolved { value: y.clone() },
        );
        id
    }

    /// Calculate text bounding box from expanded text data.
    fn calculate_text_bounds(&self, expanded: &ExpandedText) -> (Rational, Rational) {
        let mut max_x = Rational::zero();
        let mut max_y = Rational::zero();

        for instance in &expanded.instances {
            // Get prototype for this instance
            if let Some(prototype) = expanded
                .prototypes
                .values()
                .find(|p| p.entity_id == instance.prototype_id)
            {
                // Instance position + prototype advance width (scaled)
                let instance_right = instance.origin.0.clone()
                    + prototype.advance_width.clone() * expanded.scale_factor.clone();

                if instance_right > max_x {
                    max_x = instance_right;
                }

                // Use prototype bbox for height if available
                if let Some((_, _, _, y_max)) = &prototype.bbox {
                    let instance_bottom =
                        instance.origin.1.clone() + y_max.clone() * expanded.scale_factor.clone();
                    if instance_bottom > max_y {
                        max_y = instance_bottom;
                    }
                }
            }
        }

        // Fallback to font_size for height if no bbox
        if max_y == Rational::zero() {
            max_y = expanded.font_size.clone();
        }

        (max_x, max_y)
    }

    /// Internal implementation for updating text content.
    fn update_text_content_internal(
        &mut self,
        entity_id: u64,
        content: &str,
    ) -> Result<(), JsValue> {
        let text_id = EntityId(entity_id);

        // First pass: extract data from entry (releases borrow after scope)
        let (font_family, font_size, corner_tr, corner_br, corner_bl) = {
            let entry = self
                .build_info
                .text_entities
                .iter()
                .find(|e| e.id == text_id)
                .ok_or_else(|| {
                    JsValue::from_str(&format!("Text entity {} not found", entity_id))
                })?;
            (
                entry.font_family.clone(),
                entry.font_size.clone(),
                entry.corner_tr,
                entry.corner_br,
                entry.corner_bl,
            )
        };

        // Collect old glyph instance EntityIds for removal
        let old_glyph_ids: Vec<EntityId> =
            if let Some((old_expanded, _, _, _)) = self.text_expanded.get(&text_id) {
                old_expanded
                    .instances
                    .iter()
                    .map(|inst| inst.entity_id)
                    .collect()
            } else {
                Vec::new()
            };

        // Remove old PathEntityEntries from build_info.path_entities
        self.build_info
            .path_entities
            .retain(|pe| !old_glyph_ids.contains(&pe.id));

        // Get font data and shape text (requires &self)
        let font_data = self
            .font_cache
            .get(&font_family)
            .ok_or_else(|| JsValue::from_str(&format!("Font '{}' not in cache", font_family)))?
            .clone();

        let shaper = TextShaper::new(&font_data)
            .map_err(|e| JsValue::from_str(&format!("Text shaper error: {:?}", e)))?;

        let expanded = shaper
            .expand_to_paths(content, &font_size, self.next_entity_id)
            .map_err(|e| JsValue::from_str(&format!("Text shaping failed: {:?}", e)))?;

        // Update next_entity_id
        let allocated_count = expanded.prototypes.len() + expanded.instances.len();
        self.next_entity_id += allocated_count as u64;

        // Recalculate bounds
        let (width, height) = self.calculate_text_bounds(&expanded);

        // Get current position from stored data
        let (fill, x, y) = if let Some((_, fill, x, y)) = self.text_expanded.get(&text_id) {
            (fill.clone(), x.clone(), y.clone())
        } else {
            ("#ffffff".to_string(), Rational::zero(), Rational::zero())
        };

        // Register new glyph paths in build_info.path_entities
        let fill_spec = FillSpec::Solid {
            color: fill.clone(),
        };
        self.register_glyph_paths(&expanded, &x, &y, &fill_spec)?;

        // Second pass: update entry (requires &mut self)
        if let Some(entry) = self
            .build_info
            .text_entities
            .iter_mut()
            .find(|e| e.id == text_id)
        {
            entry.content = content.to_string();
            entry.measured_width = Some(width.clone());
            entry.measured_height = Some(height.clone());
        }

        // Update corner positions
        self.solver.register_variable(
            VarId::new(corner_tr, VectorComponent::X),
            VariableState::Resolved {
                value: x.clone() + width.clone(),
            },
        );
        self.solver.register_variable(
            VarId::new(corner_br, VectorComponent::X),
            VariableState::Resolved {
                value: x.clone() + width.clone(),
            },
        );
        self.solver.register_variable(
            VarId::new(corner_bl, VectorComponent::Y),
            VariableState::Resolved {
                value: y.clone() + height.clone(),
            },
        );
        self.solver.register_variable(
            VarId::new(corner_br, VectorComponent::Y),
            VariableState::Resolved {
                value: y.clone() + height.clone(),
            },
        );

        // Update stored expanded text
        self.text_expanded.insert(text_id, (expanded, fill, x, y));

        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that TextureRegistry::new() initializes correctly.
    #[test]
    fn texture_registry_new() {
        let registry = TextureRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    /// Test MediaType enum equality.
    #[test]
    fn media_type_equality() {
        assert_eq!(MediaType::StaticImage, MediaType::StaticImage);
        assert_eq!(MediaType::Video, MediaType::Video);
        assert_eq!(MediaType::AnimatedImage, MediaType::AnimatedImage);
        assert_ne!(MediaType::StaticImage, MediaType::Video);
    }

    /// Test TextureRegistry default implementation.
    #[test]
    fn texture_registry_default() {
        let registry = TextureRegistry::default();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }
}

/// Integration tests requiring WebGPU context.
/// Run with: wasm-pack test --headless --chrome -- --features gpu
#[cfg(all(test, target_arch = "wasm32", feature = "gpu"))]
mod wasm_tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Test TextureRegistry::register() returns unique IDs.
    #[wasm_bindgen_test]
    async fn texture_registry_unique_ids() {
        // Create test instance and device
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .expect("Failed to find adapter");

        let (device, _queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("Failed to request device");

        let mut registry = TextureRegistry::new();

        // Register multiple textures and verify unique IDs
        let id1 = registry.register(&device, 64, 64, MediaType::StaticImage);
        let id2 = registry.register(&device, 128, 128, MediaType::StaticImage);
        let id3 = registry.register(&device, 256, 256, MediaType::Video);

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
        assert_eq!(registry.len(), 3);
    }

    /// Test TextureRegistry::get_view() returns Some for registered textures.
    #[wasm_bindgen_test]
    async fn texture_registry_get_view() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .expect("Failed to find adapter");

        let (device, _queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("Failed to request device");

        let mut registry = TextureRegistry::new();
        let id = registry.register(&device, 64, 64, MediaType::StaticImage);

        // get_view should return Some for registered ID
        assert!(registry.get_view(id).is_some());

        // get_view should return None for unregistered ID
        assert!(registry.get_view(9999).is_none());
    }

    /// Test TextureRegistry::remove() makes get_view() return None.
    #[wasm_bindgen_test]
    async fn texture_registry_remove() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .expect("Failed to find adapter");

        let (device, _queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("Failed to request device");

        let mut registry = TextureRegistry::new();
        let id = registry.register(&device, 64, 64, MediaType::StaticImage);

        // Verify texture exists
        assert!(registry.get_view(id).is_some());
        assert_eq!(registry.len(), 1);

        // Remove texture
        let removed = registry.remove(id);
        assert!(removed);

        // Verify get_view returns None after remove
        assert!(registry.get_view(id).is_none());
        assert_eq!(registry.len(), 0);

        // Removing again should return false
        let removed_again = registry.remove(id);
        assert!(!removed_again);
    }
}
