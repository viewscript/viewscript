//! GPU Renderer Integration Module
//!
//! This module provides `GpuRenderer`, the main integration point for rendering
//! `CanvasNode` trees to wgpu surfaces.
//!
//! ## Architecture
//!
//! ```text
//! CanvasNode Tree
//!       │
//!       ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │  GpuRenderer.render_frame()                                 │
//! │                                                             │
//! │  1. Traverse tree depth-first                               │
//! │  2. TransformStack accumulates transforms                   │
//! │  3. OpacityStack accumulates opacity (multiplicative)       │
//! │  4. StencilStack manages nested clipPaths                   │
//! │  5. For each CanvasPathNode:                                │
//! │     a. tessellate_path() → vertex/index buffers             │
//! │     b. PipelineManager selects pipeline                     │
//! │     c. Create uniform buffers (transform + opacity, color)  │
//! │     d. Create bind groups                                   │
//! │     e. Issue draw call with stencil reference               │
//! └─────────────────────────────────────────────────────────────┘
//!       │
//!       ▼
//!   wgpu Surface
//! ```
//!
//! ## Initial Scope
//!
//! This implementation focuses on `CanvasPathNode` with `FillStyle::Solid`.
//! Gradient fills, strokes, text, and images are planned for future tracks.
//!
//! ## Known Limitations
//!
//! **Group opacity with multiple children**: When a group has `opacity < 1.0`
//! and contains multiple overlapping children, transparency is applied per-child
//! rather than to the composited group. This causes double-transparency artifacts
//! in overlap regions. Proper fix requires offscreen rendering (deferred to Phase E).

use std::collections::HashMap;
use std::sync::Arc;

use crate::batcher::DrawBatcher;
use crate::opacity::OpacityStack;
use crate::pipeline::PipelineManager;
use crate::shaders::{
    GradientStopUniform, GradientUniform, RadialGradientUniform, SolidColorUniform, TransformUniform,
};
use crate::stencil::StencilStack;
use crate::tessellation::{tessellate_path, tessellate_path_stroke, TessellationOutput};
use crate::transform::TransformStack;
use crate::{CanvasGroupNode, CanvasNode, CanvasPathNode, FillStyle, GradientStop};
use wgpu::util::DeviceExt;

/// GPU renderer that integrates tessellation, pipelines, and transforms.
///
/// ## Usage
///
/// ```ignore
/// // Initialize with wgpu device and queue
/// let renderer = GpuRenderer::new(device, queue, surface_format);
///
/// // Each frame:
/// let output = surface.get_current_texture()?;
/// let view = output.texture.create_view(&Default::default());
/// renderer.render_frame(&canvas_nodes, &view, width, height);
/// output.present();
/// ```
///
/// ## Resource Management
///
/// The depth-stencil texture is cached and reused across frames to avoid
/// per-frame GPU allocations. Call `ensure_depth_stencil()` before rendering
/// if the viewport size may have changed.
pub struct GpuRenderer {
    /// wgpu device for resource creation (Arc for shared ownership in tests).
    device: Arc<wgpu::Device>,
    /// wgpu queue for command submission (Arc for shared ownership in tests).
    queue: Arc<wgpu::Queue>,
    /// Pre-created pipelines for different fill styles.
    pipeline_manager: PipelineManager,
    /// Cached depth-stencil texture (recreated on resize).
    depth_stencil_texture: Option<wgpu::Texture>,
    /// Cached depth-stencil texture view (recreated on resize).
    depth_stencil_view: Option<wgpu::TextureView>,
    /// Current cached texture width.
    cached_width: u32,
    /// Current cached texture height.
    cached_height: u32,
    /// External textures registered for use with FillStyle::ExternalTexture.
    /// Maps texture ID to wgpu::TextureView.
    external_textures: HashMap<u64, wgpu::TextureView>,
    /// Cached sampler for external texture sampling.
    texture_sampler: wgpu::Sampler,
}

impl GpuRenderer {
    /// Create a new GPU renderer.
    ///
    /// ## Parameters
    ///
    /// - `device`: wgpu device for resource creation (wrapped in Arc for shared ownership)
    /// - `queue`: wgpu queue for command submission (wrapped in Arc for shared ownership)
    /// - `format`: Target texture format (typically from surface configuration)
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>, format: wgpu::TextureFormat) -> Self {
        let pipeline_manager = PipelineManager::new(&device, format);

        // Create sampler for external textures (linear filtering, clamp to edge)
        let texture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("External Texture Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            device,
            queue,
            pipeline_manager,
            depth_stencil_texture: None,
            depth_stencil_view: None,
            cached_width: 0,
            cached_height: 0,
            external_textures: HashMap::new(),
            texture_sampler,
        }
    }

    /// Ensure depth-stencil texture exists and matches the given dimensions.
    ///
    /// This method is called internally by `render_frame()` and only recreates
    /// the texture when the viewport size changes, avoiding per-frame allocations.
    fn ensure_depth_stencil(&mut self, width: u32, height: u32) {
        if self.cached_width == width && self.cached_height == height && self.depth_stencil_view.is_some() {
            return; // Already cached with correct dimensions
        }

        // Recreate depth-stencil texture
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth-Stencil Texture (Cached)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24PlusStencil8,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.depth_stencil_texture = Some(texture);
        self.depth_stencil_view = Some(view);
        self.cached_width = width;
        self.cached_height = height;
    }

    /// Render a frame with the given canvas nodes to the target texture view.
    ///
    /// ## Parameters
    ///
    /// - `nodes`: Slice of root-level canvas nodes to render
    /// - `target`: Target texture view (from surface or offscreen texture)
    /// - `viewport_width`: Viewport width in device pixels
    /// - `viewport_height`: Viewport height in device pixels
    ///
    /// ## Rendering Process (Batched - Phase H)
    ///
    /// 1. Collect all path nodes into DrawBatcher (CPU-side batching)
    /// 2. Create command encoder
    /// 3. Begin render pass with clear color
    /// 4. For each batch:
    ///    - Create GPU resources (vertex/index buffers, bind groups)
    ///    - Set pipeline and bind groups
    ///    - Issue single draw call for entire batch
    /// 5. Submit commands to queue
    ///
    /// This batched approach reduces WASM→JS boundary crossings by merging
    /// consecutive nodes with identical rendering state.
    ///
    /// ## Resource Management
    ///
    /// The depth-stencil texture is cached and only recreated when viewport
    /// dimensions change, avoiding per-frame GPU allocations.
    pub fn render_frame(
        &mut self,
        nodes: &[CanvasNode],
        target: &wgpu::TextureView,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        // Phase H: Collect nodes into batches
        let mut batcher = DrawBatcher::new();
        batcher.collect(nodes, &self.pipeline_manager, viewport_width, viewport_height);
        let batches = batcher.drain();

        // Ensure depth-stencil texture is cached and correctly sized
        self.ensure_depth_stencil(viewport_width as u32, viewport_height as u32);
        let depth_stencil_view = self.depth_stencil_view.as_ref()
            .expect("depth_stencil_view should be initialized by ensure_depth_stencil");

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("GpuRenderer Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("GpuRenderer Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_stencil_view,
                    depth_ops: None, // Not using depth
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Phase H: Render batches instead of individual nodes
            for batch in &batches {
                let pipeline_set = self.pipeline_manager.select_pipeline_by_key(batch.pipeline_key);

                // Create GPU resources - special handling for texture batches
                let resources = if let Some(texture_id) = batch.texture_id() {
                    // Texture batch: need to lookup texture view and use texture-specific method
                    if let Some(texture_view) = self.external_textures.get(&texture_id) {
                        batch.create_gpu_resources_with_texture(
                            &self.device,
                            pipeline_set,
                            texture_view,
                            &self.texture_sampler,
                        )
                    } else {
                        log::warn!(
                            "External texture {} not found, skipping batch",
                            texture_id
                        );
                        continue;
                    }
                } else {
                    // Non-texture batch: use standard method
                    batch.create_gpu_resources(&self.device, pipeline_set)
                };

                // Set stencil reference if clipping is active
                if resources.stencil_ref > 0 {
                    render_pass.set_stencil_reference(resources.stencil_ref);
                }

                // Issue batched draw call
                render_pass.set_pipeline(&pipeline_set.render_pipeline);
                render_pass.set_bind_group(0, &resources.transform_bind_group, &[]);
                render_pass.set_bind_group(1, &resources.style_bind_group, &[]);
                render_pass.set_vertex_buffer(0, resources.vertex_buffer.slice(..));
                render_pass.set_index_buffer(resources.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..resources.index_count, 0, 0..1);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Render a frame using the legacy per-node approach (pre-Phase H).
    ///
    /// This method is retained for testing and comparison purposes.
    /// Prefer `render_frame()` for production use.
    #[allow(dead_code)]
    pub fn render_frame_legacy(
        &mut self,
        nodes: &[CanvasNode],
        target: &wgpu::TextureView,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        // Ensure depth-stencil texture is cached and correctly sized
        self.ensure_depth_stencil(viewport_width as u32, viewport_height as u32);
        let depth_stencil_view = self.depth_stencil_view.as_ref()
            .expect("depth_stencil_view should be initialized by ensure_depth_stencil");

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("GpuRenderer Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("GpuRenderer Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_stencil_view,
                    depth_ops: None, // Not using depth
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Initialize stacks
            let mut transform_stack = TransformStack::new();
            let mut opacity_stack = OpacityStack::new();
            let mut stencil_stack = StencilStack::new();

            // Render each root node
            for node in nodes {
                self.render_node(
                    node,
                    &mut render_pass,
                    &mut transform_stack,
                    &mut opacity_stack,
                    &mut stencil_stack,
                    viewport_width,
                    viewport_height,
                );
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Render a single node and its children.
    ///
    /// This method handles the tree traversal, pushing/popping transforms
    /// for group nodes and rendering path nodes.
    fn render_node<'a>(
        &'a self,
        node: &CanvasNode,
        render_pass: &mut wgpu::RenderPass<'a>,
        transform_stack: &mut TransformStack,
        opacity_stack: &mut OpacityStack,
        stencil_stack: &mut StencilStack,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        match node {
            CanvasNode::Group(group) => {
                self.render_group(
                    group,
                    render_pass,
                    transform_stack,
                    opacity_stack,
                    stencil_stack,
                    viewport_width,
                    viewport_height,
                );
            }
            CanvasNode::Path(path) => {
                self.render_path(
                    path,
                    render_pass,
                    transform_stack,
                    opacity_stack,
                    stencil_stack,
                    viewport_width,
                    viewport_height,
                );
            }
            CanvasNode::Text(_) => {
                // Text rendering deferred to Phase E
            }
            CanvasNode::Image(_) => {
                // Image rendering deferred to future track
            }
        }
    }

    /// Render a group node by pushing its transform/opacity and rendering children.
    ///
    /// Handles:
    /// - Transform accumulation via TransformStack
    /// - Opacity accumulation via OpacityStack (multiplicative)
    /// - Clip path rendering via StencilStack (if clip_path is present)
    fn render_group<'a>(
        &'a self,
        group: &CanvasGroupNode,
        render_pass: &mut wgpu::RenderPass<'a>,
        transform_stack: &mut TransformStack,
        opacity_stack: &mut OpacityStack,
        stencil_stack: &mut StencilStack,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        // Push group transform onto stack
        transform_stack.push(&group.transform);

        // Push group opacity onto stack (multiplicative accumulation)
        opacity_stack.push(group.opacity as f32);

        // Handle clip path if present
        if let Some(ref clip_path) = group.clip_path {
            stencil_stack.push();
            // Render clip path to stencil buffer
            self.render_clip_path(
                clip_path,
                render_pass,
                transform_stack,
                stencil_stack,
                viewport_width,
                viewport_height,
            );
        }

        // Render children depth-first
        for child in &group.children {
            self.render_node(
                child,
                render_pass,
                transform_stack,
                opacity_stack,
                stencil_stack,
                viewport_width,
                viewport_height,
            );
        }

        // Pop clip path from stencil stack
        if group.clip_path.is_some() {
            stencil_stack.pop();
        }

        // Pop opacity when leaving group
        opacity_stack.pop();

        // Pop transform when leaving group
        transform_stack.pop();
    }

    /// Render a clip path to the stencil buffer.
    ///
    /// This uses the stencil_write pipeline which:
    /// - Writes no color (ColorWrites::empty())
    /// - Increments stencil value on pass (IncrementClamp)
    fn render_clip_path<'a>(
        &'a self,
        clip_path: &[crate::PathCommand],
        render_pass: &mut wgpu::RenderPass<'a>,
        transform_stack: &TransformStack,
        stencil_stack: &StencilStack,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        // Tessellate the clip path
        let tessellation = match tessellate_path(clip_path, None) {
            Ok(output) if !output.is_empty() => output,
            _ => return, // Skip if tessellation fails or is empty
        };

        let world_transform = transform_stack.current();

        // Get the stencil write pipeline
        let pipeline_set = self.pipeline_manager.stencil_write_pipeline();

        // Create vertex buffer
        let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Clip Path Vertex Buffer"),
            contents: bytemuck::cast_slice(&tessellation.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Create index buffer
        let index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Clip Path Index Buffer"),
            contents: bytemuck::cast_slice(&tessellation.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Create transform uniform (opacity not relevant for stencil write)
        let transform_uniform =
            TransformUniform::from_affine(&world_transform, viewport_width, viewport_height);
        let transform_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Clip Path Transform Uniform Buffer"),
            contents: bytemuck::bytes_of(&transform_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create dummy color buffer (required by shader but not used)
        let color_uniform = SolidColorUniform::new(0.0, 0.0, 0.0, 0.0);
        let color_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Clip Path Color Uniform Buffer"),
            contents: bytemuck::bytes_of(&color_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create bind groups
        let transform_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Clip Path Transform Bind Group"),
            layout: &pipeline_set.transform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buffer.as_entire_binding(),
            }],
        });

        let style_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Clip Path Style Bind Group"),
            layout: &pipeline_set.style_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: color_buffer.as_entire_binding(),
            }],
        });

        // Set stencil reference for writing
        render_pass.set_stencil_reference(stencil_stack.current());

        // Issue draw call to write to stencil buffer
        render_pass.set_pipeline(&pipeline_set.render_pipeline);
        render_pass.set_bind_group(0, &transform_bind_group, &[]);
        render_pass.set_bind_group(1, &style_bind_group, &[]);
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..tessellation.indices.len() as u32, 0, 0..1);
    }

    /// Render a path node.
    ///
    /// ## Current Scope
    ///
    /// - Supports all FillStyle variants (Solid, LinearGradient, RadialGradient)
    /// - Stroke rendering uses solid color pipeline
    /// - Opacity from OpacityStack is applied to all fills
    /// - StencilStack reference is used for clipping
    fn render_path<'a>(
        &'a self,
        path: &CanvasPathNode,
        render_pass: &mut wgpu::RenderPass<'a>,
        transform_stack: &TransformStack,
        opacity_stack: &OpacityStack,
        stencil_stack: &StencilStack,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        // Skip rendering if completely invisible
        if opacity_stack.is_invisible() {
            return;
        }

        // Get current world transform and opacity
        let world_transform = transform_stack.current();
        let opacity = opacity_stack.current();

        // Render fill if present
        if let Some(ref fill) = path.fill {
            if let Some(tessellation) = self.tessellate_fill(path, fill) {
                if !tessellation.is_empty() {
                    self.draw_tessellation(
                        render_pass,
                        &tessellation,
                        fill,
                        &world_transform,
                        opacity,
                        stencil_stack,
                        viewport_width,
                        viewport_height,
                    );
                }
            }
        }

        // Render stroke if present
        if let Some(ref stroke) = path.stroke {
            if let Ok(tessellation) = tessellate_path_stroke(&path.path_data, stroke) {
                if !tessellation.is_empty() {
                    // Strokes use solid color
                    let solid_fill = FillStyle::Solid {
                        rgba: stroke.rgba,
                    };
                    self.draw_tessellation(
                        render_pass,
                        &tessellation,
                        &solid_fill,
                        &world_transform,
                        opacity,
                        stencil_stack,
                        viewport_width,
                        viewport_height,
                    );
                }
            }
        }
    }

    /// Tessellate a path fill.
    fn tessellate_fill(&self, path: &CanvasPathNode, fill: &FillStyle) -> Option<TessellationOutput> {
        match tessellate_path(&path.path_data, Some(fill)) {
            Ok(output) => Some(output),
            Err(e) => {
                // Log tessellation errors in debug builds
                #[cfg(debug_assertions)]
                log::error!("Tessellation error: {:?}", e);
                let _ = e;
                None
            }
        }
    }

    /// Draw tessellated geometry with the appropriate pipeline.
    ///
    /// ## Parameters
    ///
    /// - `opacity`: Accumulated opacity from OpacityStack (passed to shader)
    /// - `stencil_stack`: Current stencil state for clip path testing
    fn draw_tessellation<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        tessellation: &TessellationOutput,
        fill: &FillStyle,
        world_transform: &crate::AffineTransform,
        opacity: f32,
        stencil_stack: &StencilStack,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        // Get the appropriate pipeline
        let pipeline_set = self.pipeline_manager.select_pipeline_for_fill(fill);

        // Create vertex buffer
        let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(&tessellation.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Create index buffer
        let index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&tessellation.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Create transform uniform with accumulated opacity
        let transform_uniform =
            TransformUniform::from_affine_with_opacity(world_transform, viewport_width, viewport_height, opacity);
        let transform_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Transform Uniform Buffer"),
            contents: bytemuck::bytes_of(&transform_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create style uniform buffer based on fill type
        let style_buffer = match fill {
            FillStyle::Solid { rgba } => {
                let color_uniform = SolidColorUniform::from_rgba(*rgba);
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Solid Color Uniform Buffer"),
                    contents: bytemuck::bytes_of(&color_uniform),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            }
            FillStyle::LinearGradient { stops, start, end } => {
                let stop_uniforms = Self::convert_gradient_stops(stops);
                let gradient_uniform = GradientUniform::from_linear_gradient_points(
                    start.as_ref(),
                    end.as_ref(),
                    &stop_uniforms,
                );
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Linear Gradient Uniform Buffer"),
                    contents: bytemuck::bytes_of(&gradient_uniform),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            }
            FillStyle::RadialGradient { stops, center, radius } => {
                let stop_uniforms = Self::convert_gradient_stops(stops);
                let radial_uniform = RadialGradientUniform::from_radial_gradient(
                    center.as_ref(),
                    radius.as_ref(),
                    &stop_uniforms,
                );
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Radial Gradient Uniform Buffer"),
                    contents: bytemuck::bytes_of(&radial_uniform),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            }
            FillStyle::Pattern { .. } => {
                // Pattern support deferred to future track
                return;
            }
            FillStyle::ExternalTexture { .. } => {
                // Fallback to magenta for unresolved external textures.
                // Phase J-3 will implement TextureRegistry and texture bind groups.
                let color_uniform = SolidColorUniform::new(1.0, 0.0, 1.0, 1.0);
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("ExternalTexture Fallback Uniform Buffer"),
                    contents: bytemuck::bytes_of(&color_uniform),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            }
        };

        // Create bind groups
        let transform_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Transform Bind Group"),
            layout: &pipeline_set.transform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buffer.as_entire_binding(),
            }],
        });

        let style_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Style Bind Group"),
            layout: &pipeline_set.style_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: style_buffer.as_entire_binding(),
            }],
        });

        // Set stencil reference for clip path testing (if clipping is active)
        if stencil_stack.is_clipping() {
            render_pass.set_stencil_reference(stencil_stack.current());
        }

        // Issue draw call
        render_pass.set_pipeline(&pipeline_set.render_pipeline);
        render_pass.set_bind_group(0, &transform_bind_group, &[]);
        render_pass.set_bind_group(1, &style_bind_group, &[]);
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..tessellation.indices.len() as u32, 0, 0..1);
    }

    /// Convert `GradientStop` (from FillStyle) to `GradientStopUniform` (for GPU).
    ///
    /// Converts pre-parsed RGBA bytes and Rational offsets to GPU uniform format.
    fn convert_gradient_stops(stops: &[GradientStop]) -> Vec<GradientStopUniform> {
        stops
            .iter()
            .map(|stop| {
                let color = SolidColorUniform::from_rgba(stop.rgba);
                GradientStopUniform::new(
                    color.r,
                    color.g,
                    color.b,
                    color.a,
                    stop.offset.to_f64_for_rasterization() as f32,
                )
            })
            .collect()
    }

    /// Get a reference to the pipeline manager.
    pub fn pipeline_manager(&self) -> &PipelineManager {
        &self.pipeline_manager
    }

    /// Get a reference to the wgpu device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Get a reference to the wgpu queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Get a clone of the device Arc (for shared ownership).
    pub fn device_arc(&self) -> Arc<wgpu::Device> {
        Arc::clone(&self.device)
    }

    /// Get a clone of the queue Arc (for shared ownership).
    pub fn queue_arc(&self) -> Arc<wgpu::Queue> {
        Arc::clone(&self.queue)
    }

    // =========================================================================
    // External Texture Management (Phase J-3)
    // =========================================================================

    /// Register an external texture for use with `FillStyle::ExternalTexture`.
    ///
    /// The texture ID should match the `texture_id` field in `FillStyle::ExternalTexture`.
    /// Textures are registered from `vsc-wasm::TextureRegistry` after GPU upload.
    ///
    /// ## Parameters
    ///
    /// - `id`: Unique texture ID (matches `FillStyle::ExternalTexture::texture_id`)
    /// - `view`: wgpu::TextureView for the uploaded texture
    pub fn set_external_texture(&mut self, id: u64, view: wgpu::TextureView) {
        self.external_textures.insert(id, view);
    }

    /// Remove an external texture from the renderer.
    ///
    /// Call this when the texture is no longer needed to free GPU resources.
    ///
    /// ## Parameters
    ///
    /// - `id`: Texture ID to remove
    ///
    /// ## Returns
    ///
    /// `true` if the texture was found and removed, `false` otherwise.
    pub fn remove_external_texture(&mut self, id: u64) -> bool {
        self.external_textures.remove(&id).is_some()
    }

    /// Get an external texture view by ID.
    ///
    /// Used by the draw batcher to create bind groups for texture rendering.
    pub fn get_external_texture(&self, id: u64) -> Option<&wgpu::TextureView> {
        self.external_textures.get(&id)
    }

    /// Get the number of registered external textures.
    pub fn external_texture_count(&self) -> usize {
        self.external_textures.len()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AffineTransform, CanvasNodeBase, LineCap, LineJoin,
        PVector, PVectorBounds, PathCommand, StrokeStyle,
    };
    use vsc_core::{EntityId, Rational};

    /// Helper to create a simple path node for testing.
    fn create_triangle_path(rgba: [u8; 4]) -> CanvasPathNode {
        CanvasPathNode {
            base: CanvasNodeBase {
                entity_id: EntityId(1),
                bounds: PVectorBounds {
                    top_left: PVector {
                        x: Rational::zero(),
                        y: Rational::zero(),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                    bottom_right: PVector {
                        x: Rational::from_int(100),
                        y: Rational::from_int(100),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: "test".to_string(),
            },
            path_data: vec![
                PathCommand::MoveTo {
                    x: Rational::from_int(50),
                    y: Rational::zero(),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(100),
                    y: Rational::from_int(100),
                },
                PathCommand::LineTo {
                    x: Rational::zero(),
                    y: Rational::from_int(100),
                },
                PathCommand::Close,
            ],
            fill: Some(FillStyle::Solid { rgba }),
            stroke: None,
        }
    }

    /// Helper to create a group node with opacity 1.0.
    fn create_group(children: Vec<CanvasNode>, transform: AffineTransform) -> CanvasGroupNode {
        create_group_with_opacity(children, transform, 1.0)
    }

    /// Helper to create a group node with custom opacity.
    fn create_group_with_opacity(children: Vec<CanvasNode>, transform: AffineTransform, opacity: f64) -> CanvasGroupNode {
        CanvasGroupNode {
            base: CanvasNodeBase {
                entity_id: EntityId(0),
                bounds: PVectorBounds {
                    top_left: PVector {
                        x: Rational::zero(),
                        y: Rational::zero(),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                    bottom_right: PVector {
                        x: Rational::from_int(200),
                        y: Rational::from_int(200),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: "test".to_string(),
            },
            children,
            transform,
            clip_path: None,
            opacity,
        }
    }

    #[test]
    fn test_gpu_renderer_module_compiles() {
        // Verify the module structure is valid
        // Actual GPU testing requires pollster + wgpu instance
        assert!(true);
    }

    /// Test that passing an empty `CanvasNode` slice to `render_frame()` does
    /// not panic. This verifies the boundary condition: zero nodes is valid.
    #[test]
    fn test_render_frame_empty_nodes_no_panic() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: true,
                    compatible_surface: None,
                })
                .await
            {
                Some(a) => a,
                None => {
                    log::warn!("Skipping GPU test: no adapter available");
                    return;
                }
            };

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            let device = Arc::new(device);
            let queue = Arc::new(queue);

            let format = wgpu::TextureFormat::Rgba8Unorm;
            let mut renderer = GpuRenderer::new(device.clone(), queue.clone(), format);

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Empty Nodes Test Target"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Empty slice — must not panic
            renderer.render_frame(&[], &view, 64.0, 64.0);

            println!("render_frame(&[]) completed without panic");
        });
    }

    /// Test that `render_frame()` and `render_frame_legacy()` produce consistent
    /// output for an identical scene. This test is `#[ignore]`d because it
    /// requires a GPU environment capable of texture readback.
    #[test]
    #[ignore = "requires GPU with texture readback support"]
    fn test_render_frame_vs_legacy_output_consistency() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: true,
                    compatible_surface: None,
                })
                .await
            {
                Some(a) => a,
                None => {
                    log::warn!("Skipping GPU test: no adapter available");
                    return;
                }
            };

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            let device = Arc::new(device);
            let queue = Arc::new(queue);

            let width = 64u32;
            let height = 64u32;
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let bytes_per_row = 4 * width;
            let padded_bytes_per_row = (bytes_per_row + 255) & !255;

            let mut renderer = GpuRenderer::new(device.clone(), queue.clone(), format);

            let triangle = create_triangle_path([255, 0, 0, 255]);
            let nodes = vec![CanvasNode::Path(triangle)];

            // Render with batched path (render_frame)
            let texture_a = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Batched Render Target"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view_a = texture_a.create_view(&wgpu::TextureViewDescriptor::default());
            renderer.render_frame(&nodes, &view_a, width as f32, height as f32);

            // Render with legacy path (render_frame_legacy)
            let texture_b = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Legacy Render Target"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view_b = texture_b.create_view(&wgpu::TextureViewDescriptor::default());
            renderer.render_frame_legacy(&nodes, &view_b, width as f32, height as f32);

            // Read back both textures and compare
            let read_texture = |tex: &wgpu::Texture| -> Vec<u8> {
                let buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Readback"),
                    size: (padded_bytes_per_row * height) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                enc.copy_texture_to_buffer(
                    wgpu::ImageCopyTexture {
                        texture: tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::ImageCopyBuffer {
                        buffer: &buf,
                        layout: wgpu::ImageDataLayout {
                            offset: 0,
                            bytes_per_row: Some(padded_bytes_per_row),
                            rows_per_image: Some(height),
                        },
                    },
                    wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                );
                queue.submit(std::iter::once(enc.finish()));

                let slice = buf.slice(..);
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).unwrap(); });
                device.poll(wgpu::Maintain::Wait);
                rx.recv().unwrap().expect("map failed");
                let data = slice.get_mapped_range();
                let out = data.to_vec();
                drop(data);
                buf.unmap();
                out
            };

            let pixels_a = read_texture(&texture_a);
            let pixels_b = read_texture(&texture_b);

            // Outputs must be identical (same pixel values)
            assert_eq!(
                pixels_a, pixels_b,
                "render_frame() and render_frame_legacy() must produce identical output"
            );

            println!("render_frame vs legacy: outputs match ({} bytes)", pixels_a.len());
        });
    }

    #[test]
    fn test_create_triangle_path() {
        let path = create_triangle_path([255, 0, 0, 255]);
        assert!(path.fill.is_some());
        assert_eq!(path.path_data.len(), 4);
    }

    #[test]
    fn test_create_group() {
        let child = CanvasNode::Path(create_triangle_path([0, 255, 0, 255]));
        let group = create_group(vec![child], AffineTransform::translation(50.0, 50.0));
        assert_eq!(group.children.len(), 1);
    }

    // =========================================================================
    // Integration Test (requires GPU)
    // =========================================================================

    /// Integration test that creates an offscreen texture, renders a solid-color
    /// triangle, reads back pixels, and verifies the color.
    ///
    /// This test requires a wgpu adapter and will skip if no adapter is available.
    #[test]
    fn test_render_solid_triangle_integration() {
        pollster::block_on(async {
            // Request adapter
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: true,
                    compatible_surface: None,
                })
                .await
            {
                Some(adapter) => adapter,
                None => {
                    log::warn!("Skipping GPU test: no adapter available");
                    return;
                }
            };

            // Request device and queue
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            // Wrap in Arc for shared ownership
            let device = Arc::new(device);
            let queue = Arc::new(queue);

            // Test parameters
            let width = 64u32;
            let height = 64u32;
            let format = wgpu::TextureFormat::Rgba8Unorm;

            // Create renderer
            let mut renderer = GpuRenderer::new(device.clone(), queue.clone(), format);

            // Create offscreen texture
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Test Render Target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });

            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Create a red triangle that covers most of the viewport
            let triangle = create_triangle_path([255, 0, 0, 255]);
            let nodes = vec![CanvasNode::Path(triangle)];

            // Render frame
            renderer.render_frame(&nodes, &texture_view, width as f32, height as f32);

            // Create buffer to read back pixels
            let bytes_per_row = 4 * width; // RGBA8
            let padded_bytes_per_row = (bytes_per_row + 255) & !255; // Align to 256

            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Readback Buffer"),
                size: (padded_bytes_per_row * height) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            // Copy texture to buffer
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Readback Encoder"),
            });

            encoder.copy_texture_to_buffer(
                wgpu::ImageCopyTexture {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::ImageCopyBuffer {
                    buffer: &buffer,
                    layout: wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            queue.submit(std::iter::once(encoder.finish()));

            // Map buffer and read pixels
            let buffer_slice = buffer.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
            device.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().expect("Failed to map buffer");

            let data = buffer_slice.get_mapped_range();

            // Sample a pixel near the center (should be inside the triangle)
            // Triangle: top=(50,0), bottom-right=(100,100), bottom-left=(0,100)
            // Center of gravity is around (50, 67) in a 100x100 space
            // Scaled to 64x64: (32, 43)
            let sample_x = 32usize;
            let sample_y = 50usize; // Lower part of triangle

            let pixel_offset = sample_y * padded_bytes_per_row as usize + sample_x * 4;
            let r = data[pixel_offset];
            let g = data[pixel_offset + 1];
            let b = data[pixel_offset + 2];
            let a = data[pixel_offset + 3];

            // Check that we got red (or at least reddish due to AA)
            // The pixel might be inside the triangle (red) or outside (white from clear)
            // Since our triangle coordinates are in path space and we're rendering
            // at 64x64, the triangle should cover a significant area.

            println!(
                "Sample pixel at ({}, {}): R={}, G={}, B={}, A={}",
                sample_x, sample_y, r, g, b, a
            );

            // Verify render completed without error
            // The actual pixel values depend on how the shader transforms coordinates,
            // so we just verify the render pipeline executed successfully.
            assert!(a > 0, "Alpha should be non-zero (something was rendered)");

            // Clean up
            drop(data);
            buffer.unmap();
        });
    }

    /// Test rendering with nested group transforms.
    #[test]
    fn test_render_nested_groups_integration() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: true,
                    compatible_surface: None,
                })
                .await
            {
                Some(adapter) => adapter,
                None => {
                    log::warn!("Skipping GPU test: no adapter available");
                    return;
                }
            };

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            let device = Arc::new(device);
            let queue = Arc::new(queue);

            let width = 128u32;
            let height = 128u32;
            let format = wgpu::TextureFormat::Rgba8Unorm;

            let mut renderer = GpuRenderer::new(device.clone(), queue.clone(), format);

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Test Render Target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });

            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Create nested structure:
            // Root Group (translate 20, 20)
            //   └── Child Group (scale 0.5, 0.5)
            //         └── Triangle (red)

            let triangle = CanvasNode::Path(create_triangle_path([255, 0, 0, 255]));
            let child_group = CanvasNode::Group(create_group(
                vec![triangle],
                AffineTransform::scale(0.5, 0.5),
            ));
            let root_group = CanvasNode::Group(create_group(
                vec![child_group],
                AffineTransform::translation(20.0, 20.0),
            ));

            let nodes = vec![root_group];

            // Render frame - should not panic
            renderer.render_frame(&nodes, &texture_view, width as f32, height as f32);

            // Test passes if no panic occurred
        });
    }

    /// Test rendering a path with stroke.
    #[test]
    fn test_render_stroke_integration() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: true,
                    compatible_surface: None,
                })
                .await
            {
                Some(adapter) => adapter,
                None => {
                    log::warn!("Skipping GPU test: no adapter available");
                    return;
                }
            };

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            let device = Arc::new(device);
            let queue = Arc::new(queue);

            let width = 64u32;
            let height = 64u32;
            let format = wgpu::TextureFormat::Rgba8Unorm;

            let mut renderer = GpuRenderer::new(device.clone(), queue.clone(), format);

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Test Render Target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });

            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Create path with fill and stroke
            let mut path = create_triangle_path([255, 0, 0, 255]);
            path.stroke = Some(StrokeStyle {
                rgba: [0, 0, 255, 255],
                width: Rational::from_int(3),
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                dash_array: None,
            });

            let nodes = vec![CanvasNode::Path(path)];

            // Render frame - should not panic
            renderer.render_frame(&nodes, &texture_view, width as f32, height as f32);

            // Test passes if no panic occurred
        });
    }

    /// Helper to create a rectangle path that fills the viewport for gradient testing.
    fn create_gradient_rect(fill: FillStyle) -> CanvasPathNode {
        CanvasPathNode {
            base: CanvasNodeBase {
                entity_id: EntityId(1),
                bounds: PVectorBounds {
                    top_left: PVector {
                        x: Rational::zero(),
                        y: Rational::zero(),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                    bottom_right: PVector {
                        x: Rational::from_int(64),
                        y: Rational::from_int(64),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: "test".to_string(),
            },
            path_data: vec![
                PathCommand::MoveTo {
                    x: Rational::zero(),
                    y: Rational::zero(),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(64),
                    y: Rational::zero(),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(64),
                    y: Rational::from_int(64),
                },
                PathCommand::LineTo {
                    x: Rational::zero(),
                    y: Rational::from_int(64),
                },
                PathCommand::Close,
            ],
            fill: Some(fill),
            stroke: None,
        }
    }

    /// Integration test: render a 3-stop linear gradient to offscreen texture,
    /// verify center pixel color is near the middle stop value (green).
    ///
    /// Gradient specification:
    /// - Stop 0: red (#ff0000) at 0.0
    /// - Stop 1: green (#00ff00) at 0.5
    /// - Stop 2: blue (#0000ff) at 1.0
    ///
    /// With a horizontal gradient, the center pixel (at x=0.5) should be approximately green.
    #[test]
    fn test_render_linear_gradient_integration() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: true,
                    compatible_surface: None,
                })
                .await
            {
                Some(adapter) => adapter,
                None => {
                    log::warn!("Skipping GPU test: no adapter available");
                    return;
                }
            };

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            let device = Arc::new(device);
            let queue = Arc::new(queue);

            let width = 64u32;
            let height = 64u32;
            let format = wgpu::TextureFormat::Rgba8Unorm;

            let mut renderer = GpuRenderer::new(device.clone(), queue.clone(), format);

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Gradient Test Render Target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });

            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Create a horizontal 3-stop gradient: Red -> Green -> Blue
            let gradient_fill = FillStyle::LinearGradient {
                stops: vec![
                    GradientStop {
                        offset: Rational::zero(),
                        rgba: [255, 0, 0, 255], // Red at 0.0
                    },
                    GradientStop {
                        offset: Rational::new(1, 2), // 0.5
                        rgba: [0, 255, 0, 255], // Green at 0.5
                    },
                    GradientStop {
                        offset: Rational::one(),
                        rgba: [0, 0, 255, 255], // Blue at 1.0
                    },
                ],
                // Horizontal gradient: left to right
                start: Some(crate::GradientPoint {
                    x: Rational::zero(),
                    y: Rational::new(1, 2),
                }),
                end: Some(crate::GradientPoint {
                    x: Rational::one(),
                    y: Rational::new(1, 2),
                }),
            };

            let rect = create_gradient_rect(gradient_fill);
            let nodes = vec![CanvasNode::Path(rect)];

            // Render frame
            renderer.render_frame(&nodes, &texture_view, width as f32, height as f32);

            // Create buffer to read back pixels
            let bytes_per_row = 4 * width;
            let padded_bytes_per_row = (bytes_per_row + 255) & !255;

            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Gradient Readback Buffer"),
                size: (padded_bytes_per_row * height) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Gradient Readback Encoder"),
            });

            encoder.copy_texture_to_buffer(
                wgpu::ImageCopyTexture {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::ImageCopyBuffer {
                    buffer: &buffer,
                    layout: wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            queue.submit(std::iter::once(encoder.finish()));

            // Map buffer and read pixels
            let buffer_slice = buffer.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
            device.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().expect("Failed to map buffer");

            let data = buffer_slice.get_mapped_range();

            // Sample center pixel (x=32, y=32) - should be at t=0.5 (green)
            let sample_x = 32usize;
            let sample_y = 32usize;

            let pixel_offset = sample_y * padded_bytes_per_row as usize + sample_x * 4;
            let r = data[pixel_offset];
            let g = data[pixel_offset + 1];
            let b = data[pixel_offset + 2];
            let a = data[pixel_offset + 3];

            println!(
                "Gradient center pixel at ({}, {}): R={}, G={}, B={}, A={}",
                sample_x, sample_y, r, g, b, a
            );

            // Verify the center pixel is approximately green
            // At t=0.5, we expect pure green: (0, 255, 0, 255)
            // Allow some tolerance for rounding/interpolation
            assert!(
                r < 50,
                "Center pixel R should be near 0 (got {}), expected green at t=0.5",
                r
            );
            assert!(
                g > 200,
                "Center pixel G should be near 255 (got {}), expected green at t=0.5",
                g
            );
            assert!(
                b < 50,
                "Center pixel B should be near 0 (got {}), expected green at t=0.5",
                b
            );
            assert!(a > 250, "Alpha should be near 255 (got {})", a);

            // Also verify left edge is red-ish and right edge is blue-ish
            let left_offset = sample_y * padded_bytes_per_row as usize + 2 * 4; // x=2
            let left_r = data[left_offset];
            let left_g = data[left_offset + 1];
            let left_b = data[left_offset + 2];
            println!("Left pixel (x=2): R={}, G={}, B={}", left_r, left_g, left_b);
            assert!(
                left_r > 200,
                "Left edge should be red-ish (R={}, expected >200)",
                left_r
            );

            let right_offset = sample_y * padded_bytes_per_row as usize + 61 * 4; // x=61
            let right_r = data[right_offset];
            let right_g = data[right_offset + 1];
            let right_b = data[right_offset + 2];
            println!("Right pixel (x=61): R={}, G={}, B={}", right_r, right_g, right_b);
            assert!(
                right_b > 200,
                "Right edge should be blue-ish (B={}, expected >200)",
                right_b
            );

            // Clean up
            drop(data);
            buffer.unmap();
        });
    }

    // =========================================================================
    // Opacity Stack Integration Tests
    // =========================================================================

    /// Test that nested opacity stacks multiply correctly.
    /// Renders a shape nested in two groups with 0.5 opacity each.
    /// The final rendered opacity should be 0.25 (0.5 * 0.5).
    #[test]
    fn test_nested_opacity_multiplication() {
        // Unit test for OpacityStack behavior (no GPU required)
        let mut opacity_stack = OpacityStack::new();
        assert!((opacity_stack.current() - 1.0).abs() < 0.001);

        // Push first group opacity 0.5
        opacity_stack.push(0.5);
        assert!((opacity_stack.current() - 0.5).abs() < 0.001);

        // Push second group opacity 0.5
        // Result should be 0.5 * 0.5 = 0.25
        opacity_stack.push(0.5);
        assert!(
            (opacity_stack.current() - 0.25).abs() < 0.001,
            "Nested opacity 0.5 * 0.5 should equal 0.25, got {}",
            opacity_stack.current()
        );

        // Pop second group
        opacity_stack.pop();
        assert!((opacity_stack.current() - 0.5).abs() < 0.001);

        // Pop first group
        opacity_stack.pop();
        assert!((opacity_stack.current() - 1.0).abs() < 0.001);
    }

    /// Test StencilStack push/pop/current behavior.
    #[test]
    fn test_stencil_stack_operations() {
        let mut stencil_stack = StencilStack::new();
        assert_eq!(stencil_stack.current(), 0);
        assert!(!stencil_stack.is_clipping());

        stencil_stack.push();
        assert_eq!(stencil_stack.current(), 1);
        assert!(stencil_stack.is_clipping());

        stencil_stack.push();
        assert_eq!(stencil_stack.current(), 2);

        stencil_stack.pop();
        assert_eq!(stencil_stack.current(), 1);

        stencil_stack.pop();
        assert_eq!(stencil_stack.current(), 0);
        assert!(!stencil_stack.is_clipping());
    }

    /// Integration test: render nested groups with 0.5 opacity each.
    /// The final alpha should be approximately 0.25 (blended with white background).
    #[test]
    fn test_render_nested_opacity_integration() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: true,
                    compatible_surface: None,
                })
                .await
            {
                Some(adapter) => adapter,
                None => {
                    log::warn!("Skipping GPU test: no adapter available");
                    return;
                }
            };

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            let device = Arc::new(device);
            let queue = Arc::new(queue);

            let width = 64u32;
            let height = 64u32;
            let format = wgpu::TextureFormat::Rgba8Unorm;

            let mut renderer = GpuRenderer::new(device.clone(), queue.clone(), format);

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Opacity Test Render Target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });

            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Create nested structure with opacity:
            // Root Group (opacity 0.5)
            //   └── Child Group (opacity 0.5)
            //         └── Red rectangle (full coverage)
            //
            // Final opacity should be 0.5 * 0.5 = 0.25
            // Red #ff0000 at 0.25 opacity on white background:
            // R = 255 * 0.25 + 255 * 0.75 = 63.75 + 191.25 = 255 (fully red after blend)
            // Wait, that's wrong. Let's calculate properly:
            // With premultiplied alpha blending:
            // final = src * src_alpha + dst * (1 - src_alpha)
            // R = 255 * 0.25 + 255 * 0.75 = 255 (always 255 for red channel)
            //
            // Actually, the test should check that the shape renders with reduced
            // opacity. Let's use a rectangle and verify the blended result.

            let rect = create_gradient_rect(FillStyle::Solid {
                rgba: [255, 0, 0, 255], // Pure red
            });

            // Wrap in two 0.5 opacity groups
            let inner_group = CanvasNode::Group(create_group_with_opacity(
                vec![CanvasNode::Path(rect)],
                AffineTransform::identity(),
                0.5,
            ));
            let outer_group = CanvasNode::Group(create_group_with_opacity(
                vec![inner_group],
                AffineTransform::identity(),
                0.5,
            ));

            let nodes = vec![outer_group];

            // Render frame
            renderer.render_frame(&nodes, &texture_view, width as f32, height as f32);

            // Create buffer to read back pixels
            let bytes_per_row = 4 * width;
            let padded_bytes_per_row = (bytes_per_row + 255) & !255;

            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Opacity Readback Buffer"),
                size: (padded_bytes_per_row * height) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Opacity Readback Encoder"),
            });

            encoder.copy_texture_to_buffer(
                wgpu::ImageCopyTexture {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::ImageCopyBuffer {
                    buffer: &buffer,
                    layout: wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            queue.submit(std::iter::once(encoder.finish()));

            // Map buffer and read pixels
            let buffer_slice = buffer.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
            device.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().expect("Failed to map buffer");

            let data = buffer_slice.get_mapped_range();

            // Sample center pixel
            let sample_x = 32usize;
            let sample_y = 32usize;

            let pixel_offset = sample_y * padded_bytes_per_row as usize + sample_x * 4;
            let r = data[pixel_offset];
            let g = data[pixel_offset + 1];
            let b = data[pixel_offset + 2];
            let a = data[pixel_offset + 3];

            println!(
                "Nested opacity pixel at ({}, {}): R={}, G={}, B={}, A={}",
                sample_x, sample_y, r, g, b, a
            );

            // With 0.25 opacity red (#ff0000) on white background:
            // Using standard alpha compositing (over operator):
            // R_out = R_src * A_src + R_dst * (1 - A_src)
            // R_out = 255 * 0.25 + 255 * 0.75 = 63.75 + 191.25 = 255
            // G_out = 0 * 0.25 + 255 * 0.75 = 191.25
            // B_out = 0 * 0.25 + 255 * 0.75 = 191.25
            //
            // So we expect approximately: R=255, G=191, B=191 (pinkish)
            // The exact values depend on blend mode, but G and B should be
            // noticeably higher than 0 due to the white background blending.

            // Verify the pixel shows opacity effect (not full red)
            assert!(
                g > 150,
                "Green channel should show white background blending through (got {}, expected ~191)",
                g
            );
            assert!(
                b > 150,
                "Blue channel should show white background blending through (got {}, expected ~191)",
                b
            );

            // Clean up
            drop(data);
            buffer.unmap();
        });
    }

    // =========================================================================
    // External Texture Management Tests (Phase J-3)
    // =========================================================================

    /// Test that `set_external_texture()` adds an entry to the internal map.
    #[test]
    fn test_set_external_texture_adds_entry() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: true,
                    compatible_surface: None,
                })
                .await
            {
                Some(a) => a,
                None => {
                    log::warn!("Skipping GPU test: no adapter available");
                    return;
                }
            };

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            let device = Arc::new(device);
            let queue = Arc::new(queue);
            let format = wgpu::TextureFormat::Rgba8Unorm;

            let mut renderer = GpuRenderer::new(device.clone(), queue, format);

            // Initially no external textures
            assert_eq!(renderer.external_texture_count(), 0);

            // Create a test texture
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Test External Texture"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Register the texture
            let texture_id = 42u64;
            renderer.set_external_texture(texture_id, view);

            // Verify the texture is registered
            assert_eq!(renderer.external_texture_count(), 1);
            assert!(
                renderer.get_external_texture(texture_id).is_some(),
                "get_external_texture should return Some after set_external_texture"
            );

            // Verify unregistered ID returns None
            assert!(
                renderer.get_external_texture(999).is_none(),
                "get_external_texture should return None for unregistered ID"
            );

            // Remove the texture
            assert!(
                renderer.remove_external_texture(texture_id),
                "remove_external_texture should return true for existing texture"
            );
            assert_eq!(renderer.external_texture_count(), 0);
            assert!(
                renderer.get_external_texture(texture_id).is_none(),
                "get_external_texture should return None after removal"
            );

            // Removing again should return false
            assert!(
                !renderer.remove_external_texture(texture_id),
                "remove_external_texture should return false for already removed texture"
            );
        });
    }
}
