//! Pipeline Management Module
//!
//! Manages wgpu render pipelines and bind group layouts for ViewScript rendering.
//!
//! ## Architecture
//!
//! Pipelines are created once at initialization and selected at draw time based on
//! `FillStyle`. This avoids expensive pipeline creation during frame rendering.
//!
//! ```text
//! PipelineManager
//!   ├── solid_pipeline      (FillStyle::Solid)
//!   ├── gradient_pipeline   (FillStyle::LinearGradient)
//!   └── [future: radial, pattern]
//! ```
//!
//! ## Bind Group Layout
//!
//! All pipelines share the same Group 0 layout (transform uniform).
//! Group 1 varies by pipeline type:
//!
//! - Solid: `SolidColorUniform`
//! - Gradient: `GradientUniform`

use crate::loop_blinn::{CubicLoopBlinnVertex, LoopBlinnVertex};
use crate::sdf_stroke::{CubicSdfStrokeVertex, SdfStrokeVertex};
use crate::shaders::{
    GRADIENT_WGSL, LOOP_BLINN_CUBIC_WGSL, LOOP_BLINN_WGSL, RADIAL_WGSL, SDF_STROKE_CUBIC_WGSL,
    SDF_STROKE_WGSL, SOLID_WGSL, TEXTURE_WGSL,
};
use crate::tessellation::GpuVertex;
use crate::FillStyle;

/// A complete pipeline set for a specific fill type.
pub struct PipelineSet {
    /// The compiled render pipeline.
    pub render_pipeline: wgpu::RenderPipeline,
    /// Bind group layout for Group 0 (transform uniform).
    pub transform_bind_group_layout: wgpu::BindGroupLayout,
    /// Bind group layout for Group 1 (style-specific uniforms).
    pub style_bind_group_layout: wgpu::BindGroupLayout,
}

/// Manages render pipelines for different fill styles.
///
/// Pipelines are created once at initialization. During rendering,
/// `select_pipeline()` returns the appropriate pipeline without allocation.
pub struct PipelineManager {
    /// Target texture format.
    format: wgpu::TextureFormat,
    /// Pipeline for solid color fills.
    solid: PipelineSet,
    /// Pipeline for linear gradient fills.
    gradient: PipelineSet,
    /// Pipeline for radial gradient fills.
    radial: PipelineSet,
    /// Pipeline for writing to stencil buffer (clipPath mask).
    /// Uses empty color write mask and IncrementClamp stencil operation.
    stencil_write: PipelineSet,
    /// Pipeline for Loop-Blinn quadratic Bezier curve rendering.
    /// Uses LoopBlinnVertex format with implicit curve evaluation.
    loop_blinn: PipelineSet,
    /// Pipeline for Loop-Blinn cubic Bezier curve rendering.
    /// Uses CubicLoopBlinnVertex format with implicit curve evaluation: k³ - l·m.
    loop_blinn_cubic: PipelineSet,
    /// Pipeline for SDF-based stroke rendering (quadratic).
    /// Uses SdfStrokeVertex format with Cardano's formula distance evaluation.
    sdf_stroke: PipelineSet,
    /// Pipeline for SDF-based stroke rendering (cubic).
    /// Uses CubicSdfStrokeVertex format with Newton's method distance evaluation.
    sdf_stroke_cubic: PipelineSet,
    /// Pipeline for external texture sampling (images, videos, canvas).
    /// Uses GpuVertex format with texture + sampler bind group.
    texture: PipelineSet,
}

impl PipelineManager {
    /// Create a new pipeline manager with all pipelines initialized.
    ///
    /// ## Parameters
    ///
    /// - `device`: wgpu device for pipeline creation
    /// - `format`: Target texture format (typically surface format)
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        // Create shared transform bind group layout (Group 0)
        // Visible to both VERTEX (for transform) and FRAGMENT (for opacity)
        let transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Transform Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Create solid color pipeline
        let solid = Self::create_solid_pipeline(device, format, &transform_bind_group_layout);

        // Create gradient pipeline
        let gradient = Self::create_gradient_pipeline(device, format, &transform_bind_group_layout);

        // Create radial gradient pipeline
        let radial = Self::create_radial_pipeline(device, format, &transform_bind_group_layout);

        // Create stencil write pipeline
        let stencil_write = Self::create_stencil_write_pipeline(device, format, &transform_bind_group_layout);

        // Create Loop-Blinn curve pipeline
        let loop_blinn = Self::create_loop_blinn_pipeline(device, format, &transform_bind_group_layout);

        // Create Loop-Blinn cubic curve pipeline
        let loop_blinn_cubic = Self::create_loop_blinn_cubic_pipeline(device, format, &transform_bind_group_layout);

        // Create SDF stroke pipeline (quadratic)
        let sdf_stroke = Self::create_sdf_stroke_pipeline(device, format, &transform_bind_group_layout);

        // Create SDF stroke cubic pipeline
        let sdf_stroke_cubic = Self::create_sdf_stroke_cubic_pipeline(device, format, &transform_bind_group_layout);

        // Create texture sampling pipeline
        let texture = Self::create_texture_pipeline(device, format, &transform_bind_group_layout);

        Self { format, solid, gradient, radial, stencil_write, loop_blinn, loop_blinn_cubic, sdf_stroke, sdf_stroke_cubic, texture }
    }

    /// Get the target texture format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Select the appropriate pipeline for a fill style.
    ///
    /// ## Panics
    ///
    /// Panics for unsupported fill styles (RadialGradient, Pattern).
    pub fn select_pipeline_for_fill(&self, fill: &FillStyle) -> &PipelineSet {
        match fill {
            FillStyle::Solid { .. } => &self.solid,
            FillStyle::LinearGradient { .. } => &self.gradient,
            FillStyle::RadialGradient { .. } => &self.radial,
            FillStyle::Pattern { .. } => {
                unimplemented!("Pattern pipeline not yet implemented")
            }
            FillStyle::ExternalTexture { .. } => {
                &self.texture
            }
        }
    }

    /// Select the pipeline for stroke rendering.
    ///
    /// Strokes are always rendered with solid color (from `StrokeStyle.rgba`),
    /// so this always returns the solid pipeline.
    ///
    /// ## Usage
    ///
    /// For a `CanvasPathNode` with `fill: None, stroke: Some(stroke_style)`:
    /// ```ignore
    /// let pipeline = manager.select_pipeline_for_stroke(&stroke_style);
    /// // Use stroke_style.rgba to create SolidColorUniform
    /// ```
    pub fn select_pipeline_for_stroke(&self, _stroke: &crate::StrokeStyle) -> &PipelineSet {
        // Strokes use solid color only (no gradient strokes in current spec)
        &self.solid
    }

    /// Get the solid color pipeline directly.
    ///
    /// Prefer `select_pipeline_for_fill` or `select_pipeline_for_stroke` for
    /// clarity in rendering code.
    pub fn solid_pipeline(&self) -> &PipelineSet {
        &self.solid
    }

    /// Get the gradient pipeline directly.
    pub fn gradient_pipeline(&self) -> &PipelineSet {
        &self.gradient
    }

    /// Get the radial gradient pipeline directly.
    pub fn radial_pipeline(&self) -> &PipelineSet {
        &self.radial
    }

    /// Select pipeline by `PipelineKey`.
    ///
    /// Used by the batching system to select the appropriate pipeline
    /// for a draw batch based on its `pipeline_key`.
    pub fn select_pipeline_by_key(&self, key: crate::batcher::PipelineKey) -> &PipelineSet {
        match key {
            crate::batcher::PipelineKey::Solid => &self.solid,
            crate::batcher::PipelineKey::LinearGradient => &self.gradient,
            crate::batcher::PipelineKey::RadialGradient => &self.radial,
            crate::batcher::PipelineKey::LoopBlinn => &self.loop_blinn,
            crate::batcher::PipelineKey::LoopBlinnCubic => &self.loop_blinn_cubic,
            crate::batcher::PipelineKey::SdfStroke => &self.sdf_stroke,
            crate::batcher::PipelineKey::SdfStrokeCubic => &self.sdf_stroke_cubic,
            crate::batcher::PipelineKey::Texture => &self.texture,
        }
    }

    /// Get the Loop-Blinn curve pipeline directly.
    ///
    /// Used for rendering quadratic Bezier curves with implicit function evaluation.
    pub fn loop_blinn_pipeline(&self) -> &PipelineSet {
        &self.loop_blinn
    }

    /// Get the Loop-Blinn cubic curve pipeline directly.
    ///
    /// Used for rendering cubic Bezier curves with implicit function evaluation (k³ - l·m).
    pub fn loop_blinn_cubic_pipeline(&self) -> &PipelineSet {
        &self.loop_blinn_cubic
    }

    /// Get the SDF stroke pipeline directly (quadratic).
    ///
    /// Used for rendering quadratic Bezier curve strokes with SDF evaluation.
    /// The fragment shader uses Cardano's formula to find the closest point on the curve.
    pub fn sdf_stroke_pipeline(&self) -> &PipelineSet {
        &self.sdf_stroke
    }

    /// Get the SDF stroke cubic pipeline directly.
    ///
    /// Used for rendering cubic Bezier curve strokes with SDF evaluation.
    /// The fragment shader uses Newton's method to find the closest point on the curve.
    pub fn sdf_stroke_cubic_pipeline(&self) -> &PipelineSet {
        &self.sdf_stroke_cubic
    }

    /// Get the stencil write pipeline for clipPath mask rendering.
    ///
    /// This pipeline writes to the stencil buffer without outputting color.
    /// Use it to render clip path geometry before rendering clipped content.
    pub fn stencil_write_pipeline(&self) -> &PipelineSet {
        &self.stencil_write
    }

    // =========================================================================
    // Pipeline Creation
    // =========================================================================

    fn create_solid_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        transform_layout: &wgpu::BindGroupLayout,
    ) -> PipelineSet {
        // Solid color bind group layout (Group 1)
        let style_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Solid Color Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Solid Pipeline Layout"),
            bind_group_layouts: &[transform_layout, &style_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solid Shader"),
            source: wgpu::ShaderSource::Wgsl(SOLID_WGSL.into()),
        });

        // Render pipeline
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Solid Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // No culling for 2D
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Clone the transform layout for storage
        // (We need to create a new one since BindGroupLayout doesn't impl Clone)
        let transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Transform Bind Group Layout (Solid)"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        PipelineSet {
            render_pipeline,
            transform_bind_group_layout,
            style_bind_group_layout,
        }
    }

    fn create_gradient_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        transform_layout: &wgpu::BindGroupLayout,
    ) -> PipelineSet {
        // Gradient bind group layout (Group 1)
        let style_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Gradient Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Gradient Pipeline Layout"),
            bind_group_layouts: &[transform_layout, &style_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Gradient Shader"),
            source: wgpu::ShaderSource::Wgsl(GRADIENT_WGSL.into()),
        });

        // Render pipeline
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Gradient Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Clone transform layout
        let transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Transform Bind Group Layout (Gradient)"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        PipelineSet {
            render_pipeline,
            transform_bind_group_layout,
            style_bind_group_layout,
        }
    }

    fn create_radial_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        transform_layout: &wgpu::BindGroupLayout,
    ) -> PipelineSet {
        // Radial gradient bind group layout (Group 1)
        // Same structure as linear gradient - single uniform buffer
        let style_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Radial Gradient Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Radial Gradient Pipeline Layout"),
            bind_group_layouts: &[transform_layout, &style_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Radial Gradient Shader"),
            source: wgpu::ShaderSource::Wgsl(RADIAL_WGSL.into()),
        });

        // Render pipeline
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Radial Gradient Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Clone transform layout
        let transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Transform Bind Group Layout (Radial)"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        PipelineSet {
            render_pipeline,
            transform_bind_group_layout,
            style_bind_group_layout,
        }
    }

    /// Create the stencil write pipeline for clipPath mask rendering.
    ///
    /// This pipeline:
    /// - Uses empty color write mask (no color output)
    /// - Increments stencil buffer on pass (IncrementClamp)
    /// - Uses the solid shader but discards color output
    fn create_stencil_write_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        transform_layout: &wgpu::BindGroupLayout,
    ) -> PipelineSet {
        // Use same bind group layout as solid (we don't actually need color, but
        // the shader expects a uniform buffer)
        let style_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Stencil Write Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Stencil Write Pipeline Layout"),
            bind_group_layouts: &[transform_layout, &style_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Use solid shader - we'll just discard the color output via write mask
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Stencil Write Shader"),
            source: wgpu::ShaderSource::Wgsl(SOLID_WGSL.into()),
        });

        // Stencil state for clip path rendering:
        // - Always pass depth test (we don't use depth)
        // - Increment stencil value on pass (marks pixels inside clip path)
        let stencil_face_state = wgpu::StencilFaceState {
            compare: wgpu::CompareFunction::Always,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op: wgpu::StencilOperation::IncrementClamp,
        };

        // Render pipeline with stencil write configuration
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Stencil Write Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None, // No blending needed
                    write_mask: wgpu::ColorWrites::empty(), // No color output
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState {
                    front: stencil_face_state,
                    back: stencil_face_state,
                    read_mask: 0xff,
                    write_mask: 0xff,
                },
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Clone transform layout
        let transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Transform Bind Group Layout (Stencil Write)"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        PipelineSet {
            render_pipeline,
            transform_bind_group_layout,
            style_bind_group_layout,
        }
    }

    /// Create the Loop-Blinn curve rendering pipeline.
    ///
    /// This pipeline renders quadratic Bezier curves using the Loop-Blinn algorithm:
    /// - Each curve is a single triangle (P0, P1, P2)
    /// - Fragment shader evaluates implicit function f = u² - v
    /// - Uses smoothstep anti-aliasing for smooth edges
    ///
    /// Bind groups are shared with solid pipeline (same SolidColorUniform).
    fn create_loop_blinn_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        transform_layout: &wgpu::BindGroupLayout,
    ) -> PipelineSet {
        // Same bind group layout as solid (uses SolidColorUniform)
        let style_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Loop-Blinn Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Loop-Blinn Pipeline Layout"),
            bind_group_layouts: &[transform_layout, &style_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Loop-Blinn shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Loop-Blinn Shader"),
            source: wgpu::ShaderSource::Wgsl(LOOP_BLINN_WGSL.into()),
        });

        // Render pipeline with LoopBlinnVertex format
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Loop-Blinn Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[LoopBlinnVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // No culling for 2D
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Clone transform layout
        let transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Transform Bind Group Layout (Loop-Blinn)"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        PipelineSet {
            render_pipeline,
            transform_bind_group_layout,
            style_bind_group_layout,
        }
    }

    /// Create the Loop-Blinn cubic curve rendering pipeline.
    ///
    /// This pipeline renders cubic Bezier curves using the Loop-Blinn algorithm:
    /// - Each curve is two triangles covering the control polygon (P0, P1, P2, P3)
    /// - Fragment shader evaluates implicit function f = k³ - l·m
    /// - Uses smoothstep anti-aliasing for smooth edges
    ///
    /// Bind groups are shared with solid pipeline (same SolidColorUniform).
    fn create_loop_blinn_cubic_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        transform_layout: &wgpu::BindGroupLayout,
    ) -> PipelineSet {
        // Same bind group layout as solid (uses SolidColorUniform)
        let style_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Loop-Blinn Cubic Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Loop-Blinn Cubic Pipeline Layout"),
            bind_group_layouts: &[transform_layout, &style_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Loop-Blinn cubic shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Loop-Blinn Cubic Shader"),
            source: wgpu::ShaderSource::Wgsl(LOOP_BLINN_CUBIC_WGSL.into()),
        });

        // Render pipeline with CubicLoopBlinnVertex format
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Loop-Blinn Cubic Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[CubicLoopBlinnVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // No culling for 2D
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Clone transform layout
        let transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Transform Bind Group Layout (Loop-Blinn Cubic)"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        PipelineSet {
            render_pipeline,
            transform_bind_group_layout,
            style_bind_group_layout,
        }
    }

    /// Create the SDF stroke rendering pipeline.
    ///
    /// This pipeline renders quadratic Bezier strokes using Signed Distance Field:
    /// - Each curve emits a bounding rectangle (2 triangles)
    /// - Fragment shader analytically computes distance to curve using Cardano's formula
    /// - Uses smoothstep anti-aliasing for smooth stroke edges
    ///
    /// Bind groups are shared with solid pipeline (same SolidColorUniform).
    fn create_sdf_stroke_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        transform_layout: &wgpu::BindGroupLayout,
    ) -> PipelineSet {
        // Same bind group layout as solid (uses SolidColorUniform for stroke color)
        let style_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("SDF Stroke Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SDF Stroke Pipeline Layout"),
            bind_group_layouts: &[transform_layout, &style_bind_group_layout],
            push_constant_ranges: &[],
        });

        // SDF stroke shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SDF Stroke Shader"),
            source: wgpu::ShaderSource::Wgsl(SDF_STROKE_WGSL.into()),
        });

        // Render pipeline with SdfStrokeVertex format
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF Stroke Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[SdfStrokeVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // No culling for 2D
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Clone transform layout
        let transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Transform Bind Group Layout (SDF Stroke)"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        PipelineSet {
            render_pipeline,
            transform_bind_group_layout,
            style_bind_group_layout,
        }
    }

    /// Create the SDF stroke cubic rendering pipeline.
    ///
    /// This pipeline renders cubic Bezier strokes using Signed Distance Field:
    /// - Each curve emits a bounding rectangle (2 triangles)
    /// - Fragment shader uses Newton's method to find the closest point on the curve
    /// - Uses smoothstep anti-aliasing for smooth stroke edges
    ///
    /// Bind groups are shared with solid pipeline (same SolidColorUniform).
    fn create_sdf_stroke_cubic_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        transform_layout: &wgpu::BindGroupLayout,
    ) -> PipelineSet {
        // Same bind group layout as solid (uses SolidColorUniform for stroke color)
        let style_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("SDF Stroke Cubic Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SDF Stroke Cubic Pipeline Layout"),
            bind_group_layouts: &[transform_layout, &style_bind_group_layout],
            push_constant_ranges: &[],
        });

        // SDF stroke cubic shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SDF Stroke Cubic Shader"),
            source: wgpu::ShaderSource::Wgsl(SDF_STROKE_CUBIC_WGSL.into()),
        });

        // Render pipeline with CubicSdfStrokeVertex format
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF Stroke Cubic Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[CubicSdfStrokeVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // No culling for 2D
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Clone transform layout
        let transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Transform Bind Group Layout (SDF Stroke Cubic)"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        PipelineSet {
            render_pipeline,
            transform_bind_group_layout,
            style_bind_group_layout,
        }
    }

    fn create_texture_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        transform_layout: &wgpu::BindGroupLayout,
    ) -> PipelineSet {
        // Texture + sampler bind group layout (Group 1)
        let style_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Texture Bind Group Layout"),
                entries: &[
                    // @binding(0) texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // @binding(1) sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Texture Pipeline Layout"),
            bind_group_layouts: &[transform_layout, &style_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Texture Shader"),
            source: wgpu::ShaderSource::Wgsl(TEXTURE_WGSL.into()),
        });

        // Render pipeline
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Texture Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // No culling for 2D
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Clone the transform layout for storage
        let transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Transform Bind Group Layout (Texture)"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        PipelineSet {
            render_pipeline,
            transform_bind_group_layout,
            style_bind_group_layout,
        }
    }

    /// Get the texture sampling pipeline directly.
    ///
    /// Used for rendering shapes with external texture fills (images, videos, canvas).
    pub fn texture_pipeline(&self) -> &PipelineSet {
        &self.texture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: GPU tests require actual device initialization.
    // These tests verify the module structure compiles correctly.
    // Integration tests with real GPU will be in a separate test crate
    // or behind a feature flag.

    #[test]
    fn test_pipeline_module_compiles() {
        // Verify the module structure is valid
        // Actual GPU testing requires pollster + wgpu instance
        assert!(!SOLID_WGSL.is_empty());
        assert!(!GRADIENT_WGSL.is_empty());
        assert!(!RADIAL_WGSL.is_empty());
    }

    #[test]
    fn test_fill_style_matching() {
        // Verify FillStyle variants can be matched
        let solid = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };
        let gradient = FillStyle::LinearGradient {
            stops: vec![],
            start: None,
            end: None,
        };

        // These should not panic (just pattern match verification)
        match &solid {
            FillStyle::Solid { .. } => {}
            _ => panic!("Should match Solid"),
        }
        match &gradient {
            FillStyle::LinearGradient { .. } => {}
            _ => panic!("Should match LinearGradient"),
        }
    }

    /// Verify that `select_pipeline_for_fill(FillStyle::Pattern { .. })` panics
    /// with `unimplemented!()`. Pattern pipeline is not yet implemented.
    ///
    /// This test requires a real wgpu device; skip if no GPU is available.
    #[test]
    #[should_panic(expected = "Pattern pipeline not yet implemented")]
    fn test_select_pipeline_pattern_panics() {
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
                    // No GPU available: skip by panicking with the expected message
                    // so the #[should_panic] is satisfied.
                    panic!("Pattern pipeline not yet implemented");
                }
            };

            let (device, _queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            let format = wgpu::TextureFormat::Rgba8Unorm;
            let manager = PipelineManager::new(&device, format);

            let pattern_fill = FillStyle::Pattern {
                pattern_ref: vsc_core::EntityId(999),
            };

            // This must panic with "Pattern pipeline not yet implemented"
            let _ = manager.select_pipeline_for_fill(&pattern_fill);
        });
    }

    /// Verify that `select_pipeline_by_key` returns a valid pipeline for
    /// each of the three defined keys: Solid, LinearGradient, RadialGradient.
    ///
    /// This test requires a real wgpu device.
    #[test]
    fn test_select_pipeline_by_key_all_variants() {
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

            let (device, _queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            let format = wgpu::TextureFormat::Rgba8Unorm;
            let manager = PipelineManager::new(&device, format);

            // Verify each PipelineKey returns without panic and the pipeline is
            // consistent with the fill-style-based accessor.
            let solid_by_key = manager.select_pipeline_by_key(crate::batcher::PipelineKey::Solid);
            let solid_by_fill = manager.select_pipeline_for_fill(&FillStyle::Solid {
                rgba: [255, 255, 255, 255],
            });
            // Both must point to the solid pipeline (same object address)
            assert!(
                std::ptr::eq(solid_by_key, solid_by_fill),
                "select_pipeline_by_key(Solid) and select_pipeline_for_fill(Solid) must return the same PipelineSet"
            );

            let grad_by_key = manager.select_pipeline_by_key(crate::batcher::PipelineKey::LinearGradient);
            let grad_by_fill = manager.select_pipeline_for_fill(&FillStyle::LinearGradient {
                stops: vec![],
                start: None,
                end: None,
            });
            assert!(
                std::ptr::eq(grad_by_key, grad_by_fill),
                "select_pipeline_by_key(LinearGradient) and select_pipeline_for_fill(LinearGradient) must return the same PipelineSet"
            );

            let radial_by_key = manager.select_pipeline_by_key(crate::batcher::PipelineKey::RadialGradient);
            let radial_by_fill = manager.select_pipeline_for_fill(&FillStyle::RadialGradient {
                stops: vec![],
                center: None,
                radius: None,
            });
            assert!(
                std::ptr::eq(radial_by_key, radial_by_fill),
                "select_pipeline_by_key(RadialGradient) and select_pipeline_for_fill(RadialGradient) must return the same PipelineSet"
            );

            // LoopBlinn pipeline (no corresponding FillStyle - used for curve triangles)
            let loop_blinn_by_key = manager.select_pipeline_by_key(crate::batcher::PipelineKey::LoopBlinn);
            let loop_blinn_direct = manager.loop_blinn_pipeline();
            assert!(
                std::ptr::eq(loop_blinn_by_key, loop_blinn_direct),
                "select_pipeline_by_key(LoopBlinn) and loop_blinn_pipeline() must return the same PipelineSet"
            );

            // LoopBlinnCubic pipeline (no corresponding FillStyle - used for cubic curve triangles)
            let loop_blinn_cubic_by_key = manager.select_pipeline_by_key(crate::batcher::PipelineKey::LoopBlinnCubic);
            let loop_blinn_cubic_direct = manager.loop_blinn_cubic_pipeline();
            assert!(
                std::ptr::eq(loop_blinn_cubic_by_key, loop_blinn_cubic_direct),
                "select_pipeline_by_key(LoopBlinnCubic) and loop_blinn_cubic_pipeline() must return the same PipelineSet"
            );

            // SdfStroke pipeline (no corresponding FillStyle - used for quadratic curve strokes)
            let sdf_stroke_by_key = manager.select_pipeline_by_key(crate::batcher::PipelineKey::SdfStroke);
            let sdf_stroke_direct = manager.sdf_stroke_pipeline();
            assert!(
                std::ptr::eq(sdf_stroke_by_key, sdf_stroke_direct),
                "select_pipeline_by_key(SdfStroke) and sdf_stroke_pipeline() must return the same PipelineSet"
            );

            // SdfStrokeCubic pipeline (no corresponding FillStyle - used for cubic curve strokes)
            let sdf_stroke_cubic_by_key = manager.select_pipeline_by_key(crate::batcher::PipelineKey::SdfStrokeCubic);
            let sdf_stroke_cubic_direct = manager.sdf_stroke_cubic_pipeline();
            assert!(
                std::ptr::eq(sdf_stroke_cubic_by_key, sdf_stroke_cubic_direct),
                "select_pipeline_by_key(SdfStrokeCubic) and sdf_stroke_cubic_pipeline() must return the same PipelineSet"
            );

            println!("select_pipeline_by_key: all 7 keys verified successfully");
        });
    }

    #[test]
    fn test_loop_blinn_vertex_stride() {
        // LoopBlinnVertex must be 20 bytes (5 x f32)
        let layout = LoopBlinnVertex::desc();
        assert_eq!(
            layout.array_stride, 20,
            "LoopBlinnVertex stride must be 20 bytes, got {}",
            layout.array_stride
        );
    }

    #[test]
    fn test_loop_blinn_wgsl_not_empty() {
        assert!(!LOOP_BLINN_WGSL.is_empty(), "LOOP_BLINN_WGSL should not be empty");
        assert!(
            LOOP_BLINN_WGSL.contains("vs_main"),
            "LOOP_BLINN_WGSL should contain vs_main"
        );
        assert!(
            LOOP_BLINN_WGSL.contains("fs_main"),
            "LOOP_BLINN_WGSL should contain fs_main"
        );
    }

    #[test]
    fn test_loop_blinn_cubic_wgsl_not_empty() {
        assert!(!LOOP_BLINN_CUBIC_WGSL.is_empty(), "LOOP_BLINN_CUBIC_WGSL should not be empty");
        assert!(
            LOOP_BLINN_CUBIC_WGSL.contains("vs_main"),
            "LOOP_BLINN_CUBIC_WGSL should contain vs_main"
        );
        assert!(
            LOOP_BLINN_CUBIC_WGSL.contains("fs_main"),
            "LOOP_BLINN_CUBIC_WGSL should contain fs_main"
        );
        // Verify cubic-specific elements
        assert!(
            LOOP_BLINN_CUBIC_WGSL.contains("curve_klm"),
            "LOOP_BLINN_CUBIC_WGSL should contain curve_klm (k, l, m texture coordinates)"
        );
        assert!(
            LOOP_BLINN_CUBIC_WGSL.contains("curve_sign"),
            "LOOP_BLINN_CUBIC_WGSL should contain curve_sign"
        );
    }

    #[test]
    fn test_loop_blinn_cubic_vertex_stride() {
        // CubicLoopBlinnVertex must be 24 bytes (6 x f32)
        let layout = CubicLoopBlinnVertex::desc();
        assert_eq!(
            layout.array_stride, 24,
            "CubicLoopBlinnVertex stride must be 24 bytes, got {}",
            layout.array_stride
        );
    }

    #[test]
    fn test_sdf_stroke_wgsl_not_empty() {
        assert!(!SDF_STROKE_WGSL.is_empty(), "SDF_STROKE_WGSL should not be empty");
        assert!(
            SDF_STROKE_WGSL.contains("vs_main"),
            "SDF_STROKE_WGSL should contain vs_main"
        );
        assert!(
            SDF_STROKE_WGSL.contains("fs_main"),
            "SDF_STROKE_WGSL should contain fs_main"
        );
        // Verify Cardano's formula implementation
        assert!(
            SDF_STROKE_WGSL.contains("solve_depressed_cubic"),
            "SDF_STROKE_WGSL should contain solve_depressed_cubic (Cardano's formula)"
        );
        assert!(
            SDF_STROKE_WGSL.contains("min_dist_sq_to_bezier"),
            "SDF_STROKE_WGSL should contain min_dist_sq_to_bezier (distance function)"
        );
    }

    #[test]
    fn test_sdf_stroke_vertex_stride() {
        // SdfStrokeVertex must be 44 bytes (11 x f32)
        let layout = SdfStrokeVertex::desc();
        assert_eq!(
            layout.array_stride, 44,
            "SdfStrokeVertex stride must be 44 bytes, got {}",
            layout.array_stride
        );
    }

    #[test]
    fn test_sdf_stroke_cubic_wgsl_not_empty() {
        assert!(!SDF_STROKE_CUBIC_WGSL.is_empty(), "SDF_STROKE_CUBIC_WGSL should not be empty");
        assert!(
            SDF_STROKE_CUBIC_WGSL.contains("vs_main"),
            "SDF_STROKE_CUBIC_WGSL should contain vs_main"
        );
        assert!(
            SDF_STROKE_CUBIC_WGSL.contains("fs_main"),
            "SDF_STROKE_CUBIC_WGSL should contain fs_main"
        );
        // Verify Newton's method implementation
        assert!(
            SDF_STROKE_CUBIC_WGSL.contains("newton_step"),
            "SDF_STROKE_CUBIC_WGSL should contain newton_step (Newton's method)"
        );
        assert!(
            SDF_STROKE_CUBIC_WGSL.contains("min_dist_sq_to_cubic_bezier"),
            "SDF_STROKE_CUBIC_WGSL should contain min_dist_sq_to_cubic_bezier (distance function)"
        );
        // Verify it handles 4 control points
        assert!(
            SDF_STROKE_CUBIC_WGSL.contains("p3"),
            "SDF_STROKE_CUBIC_WGSL should contain p3 (4th control point)"
        );
    }

    #[test]
    fn test_sdf_stroke_cubic_vertex_stride() {
        // CubicSdfStrokeVertex must be 52 bytes (13 x f32)
        let layout = CubicSdfStrokeVertex::desc();
        assert_eq!(
            layout.array_stride, 52,
            "CubicSdfStrokeVertex stride must be 52 bytes, got {}",
            layout.array_stride
        );
    }

    // =========================================================================
    // Texture Pipeline Tests (Phase J-3)
    // =========================================================================

    #[test]
    fn test_texture_wgsl_not_empty() {
        assert!(!TEXTURE_WGSL.is_empty(), "TEXTURE_WGSL should not be empty");
        assert!(
            TEXTURE_WGSL.contains("vs_main"),
            "TEXTURE_WGSL should contain vs_main"
        );
        assert!(
            TEXTURE_WGSL.contains("fs_main"),
            "TEXTURE_WGSL should contain fs_main"
        );
        assert!(
            TEXTURE_WGSL.contains("textureSample"),
            "TEXTURE_WGSL should contain textureSample"
        );
        assert!(
            TEXTURE_WGSL.contains("t_texture"),
            "TEXTURE_WGSL should contain t_texture binding"
        );
        assert!(
            TEXTURE_WGSL.contains("t_sampler"),
            "TEXTURE_WGSL should contain t_sampler binding"
        );
    }

    /// Verify that `select_pipeline_by_key(Texture)` returns the texture pipeline.
    ///
    /// This test requires a real wgpu device.
    #[test]
    fn test_select_pipeline_by_key_texture() {
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

            let (device, _queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");

            let format = wgpu::TextureFormat::Rgba8Unorm;
            let manager = PipelineManager::new(&device, format);

            // Verify PipelineKey::Texture returns the texture pipeline
            let texture_by_key = manager.select_pipeline_by_key(crate::batcher::PipelineKey::Texture);
            let texture_direct = manager.texture_pipeline();
            assert!(
                std::ptr::eq(texture_by_key, texture_direct),
                "select_pipeline_by_key(Texture) and texture_pipeline() must return the same PipelineSet"
            );

            // Verify FillStyle::ExternalTexture selects the texture pipeline
            let external_fill = FillStyle::ExternalTexture {
                texture_id: 42,
                uv_transform: vsc_core::UvTransform::default(),
            };
            let texture_by_fill = manager.select_pipeline_for_fill(&external_fill);
            assert!(
                std::ptr::eq(texture_by_fill, texture_direct),
                "select_pipeline_for_fill(ExternalTexture) must return the texture pipeline"
            );

            println!("select_pipeline_by_key(Texture): verified successfully");
        });
    }
}
