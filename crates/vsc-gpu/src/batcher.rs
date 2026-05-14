//! Draw Call Batcher (Phase H)
//!
//! This module implements draw call batching to reduce WASM→JS boundary crossings
//! in the wgpu Web backend. Instead of issuing one draw call per `CanvasNode`,
//! nodes with identical rendering state are merged into batches.
//!
//! ## Batching Strategy
//!
//! ```text
//! Before (per-node draw calls):
//!   set_pipeline(solid) → draw(rect1)
//!   set_pipeline(solid) → draw(rect2)  // Same pipeline, same color!
//!   set_pipeline(solid) → draw(rect3)
//!
//! After (batched):
//!   set_pipeline(solid) → draw(rect1 + rect2 + rect3)  // Single draw call
//! ```
//!
//! ## Batch Key
//!
//! Nodes are grouped into batches by:
//! - `PipelineKey`: solid / linear_gradient / radial_gradient
//! - `UniformData`: color or gradient parameters (byte-wise equality)
//! - `stencil_ref`: clip path scope
//! - `opacity`: accumulated opacity from scene hierarchy
//!
//! ## Transform Handling
//!
//! Transforms are applied on CPU to vertices before batching. This allows
//! nodes with different transforms but identical other state to be merged.
//! The shader receives identity transform (viewport conversion only).

use crate::loop_blinn::{
    tessellate_cubic_beziers, tessellate_quadratic_beziers, CubicLoopBlinnOutput,
    CubicLoopBlinnVertex, LoopBlinnOutput, LoopBlinnVertex,
};
use crate::opacity::OpacityStack;
use crate::pipeline::{PipelineManager, PipelineSet};
use crate::sdf_stroke::{
    tessellate_cubic_stroke_segments, tessellate_stroke_segments, CubicSdfStrokeOutput,
    CubicSdfStrokeVertex, SdfStrokeOutput, SdfStrokeVertex,
};
use crate::shaders::{
    GradientStopUniform, GradientUniform, RadialGradientUniform, SolidColorUniform,
    TransformUniform,
};
use crate::stencil::StencilStack;
use crate::tessellation::{tessellate_path, tessellate_path_stroke, GpuVertex, TessellationOutput};
use crate::transform::TransformStack;
use crate::{
    AffineTransform, CanvasGroupNode, CanvasNode, CanvasPathNode, FillStyle, GradientStop,
};
use vsc_core::PathCommand;
use wgpu::util::DeviceExt;

// =============================================================================
// Pipeline and Uniform Types
// =============================================================================

/// Pipeline type for batch grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineKey {
    Solid,
    LinearGradient,
    RadialGradient,
    /// Loop-Blinn curve rendering pipeline for quadratic Bezier.
    /// Uses LoopBlinnVertex format with implicit curve evaluation: u² - v.
    LoopBlinn,
    /// Loop-Blinn curve rendering pipeline for cubic Bezier.
    /// Uses CubicLoopBlinnVertex format with implicit curve evaluation: k³ - l·m.
    LoopBlinnCubic,
    /// SDF stroke rendering pipeline (quadratic).
    /// Uses SdfStrokeVertex format with Cardano's formula for distance evaluation.
    SdfStroke,
    /// SDF stroke rendering pipeline (cubic).
    /// Uses CubicSdfStrokeVertex format with Newton's method for distance evaluation.
    SdfStrokeCubic,
    /// External texture sampling pipeline.
    /// Uses GpuVertex format with texture + sampler bind group.
    Texture,
}

// =============================================================================
// Batch Vertices (abstracts over GpuVertex and LoopBlinnVertex)
// =============================================================================

/// Vertex data for a draw batch, abstracting over different vertex formats.
///
/// This allows a single `DrawBatch` type to handle both standard fill tessellation
/// (using `GpuVertex`) and Loop-Blinn curve rendering (using `LoopBlinnVertex`).
#[derive(Debug, Clone)]
pub enum BatchVertices {
    /// Standard vertices from lyon tessellation.
    Standard(Vec<GpuVertex>),
    /// Loop-Blinn quadratic curve vertices with implicit function parameters (u² - v).
    LoopBlinn(Vec<LoopBlinnVertex>),
    /// Loop-Blinn cubic curve vertices with implicit function parameters (k³ - l·m).
    LoopBlinnCubic(Vec<CubicLoopBlinnVertex>),
    /// SDF stroke vertices with curve control points for distance evaluation (quadratic).
    SdfStroke(Vec<SdfStrokeVertex>),
    /// SDF stroke cubic vertices with 4 control points for Newton's method (cubic).
    SdfStrokeCubic(Vec<CubicSdfStrokeVertex>),
}

impl BatchVertices {
    /// Check if the vertex buffer is empty.
    pub fn is_empty(&self) -> bool {
        match self {
            BatchVertices::Standard(v) => v.is_empty(),
            BatchVertices::LoopBlinn(v) => v.is_empty(),
            BatchVertices::LoopBlinnCubic(v) => v.is_empty(),
            BatchVertices::SdfStroke(v) => v.is_empty(),
            BatchVertices::SdfStrokeCubic(v) => v.is_empty(),
        }
    }

    /// Get the number of vertices.
    pub fn len(&self) -> usize {
        match self {
            BatchVertices::Standard(v) => v.len(),
            BatchVertices::LoopBlinn(v) => v.len(),
            BatchVertices::LoopBlinnCubic(v) => v.len(),
            BatchVertices::SdfStroke(v) => v.len(),
            BatchVertices::SdfStrokeCubic(v) => v.len(),
        }
    }

    /// Get the pipeline key based on vertex type and uniform data.
    pub fn pipeline_key(&self, uniform_data: &UniformData) -> PipelineKey {
        match self {
            BatchVertices::LoopBlinn(_) => PipelineKey::LoopBlinn,
            BatchVertices::LoopBlinnCubic(_) => PipelineKey::LoopBlinnCubic,
            BatchVertices::SdfStroke(_) => PipelineKey::SdfStroke,
            BatchVertices::SdfStrokeCubic(_) => PipelineKey::SdfStrokeCubic,
            BatchVertices::Standard(_) => uniform_data.pipeline_key(),
        }
    }

    /// Get a reference to standard vertices, if this is a standard batch.
    pub fn as_standard(&self) -> Option<&Vec<GpuVertex>> {
        match self {
            BatchVertices::Standard(v) => Some(v),
            BatchVertices::LoopBlinn(_) => None,
            BatchVertices::LoopBlinnCubic(_) => None,
            BatchVertices::SdfStroke(_) => None,
            BatchVertices::SdfStrokeCubic(_) => None,
        }
    }

    /// Get a reference to Loop-Blinn vertices, if this is a Loop-Blinn batch.
    pub fn as_loop_blinn(&self) -> Option<&Vec<LoopBlinnVertex>> {
        match self {
            BatchVertices::Standard(_) => None,
            BatchVertices::LoopBlinn(v) => Some(v),
            BatchVertices::LoopBlinnCubic(_) => None,
            BatchVertices::SdfStroke(_) => None,
            BatchVertices::SdfStrokeCubic(_) => None,
        }
    }

    /// Get a reference to Loop-Blinn cubic vertices, if this is a Loop-Blinn cubic batch.
    pub fn as_loop_blinn_cubic(&self) -> Option<&Vec<CubicLoopBlinnVertex>> {
        match self {
            BatchVertices::Standard(_) => None,
            BatchVertices::LoopBlinn(_) => None,
            BatchVertices::LoopBlinnCubic(v) => Some(v),
            BatchVertices::SdfStroke(_) => None,
            BatchVertices::SdfStrokeCubic(_) => None,
        }
    }

    /// Get a reference to SDF stroke vertices, if this is an SDF stroke batch.
    pub fn as_sdf_stroke(&self) -> Option<&Vec<SdfStrokeVertex>> {
        match self {
            BatchVertices::Standard(_) => None,
            BatchVertices::LoopBlinn(_) => None,
            BatchVertices::LoopBlinnCubic(_) => None,
            BatchVertices::SdfStroke(v) => Some(v),
            BatchVertices::SdfStrokeCubic(_) => None,
        }
    }

    /// Get a reference to SDF stroke cubic vertices, if this is an SDF stroke cubic batch.
    pub fn as_sdf_stroke_cubic(&self) -> Option<&Vec<CubicSdfStrokeVertex>> {
        match self {
            BatchVertices::Standard(_) => None,
            BatchVertices::LoopBlinn(_) => None,
            BatchVertices::LoopBlinnCubic(_) => None,
            BatchVertices::SdfStroke(_) => None,
            BatchVertices::SdfStrokeCubic(v) => Some(v),
        }
    }
}

// =============================================================================
// Path Analysis Helpers
// =============================================================================

/// Check if a path contains any QuadTo commands.
///
/// Used to determine whether to use Loop-Blinn rendering path.
fn has_quadratic_bezier(commands: &[PathCommand]) -> bool {
    commands
        .iter()
        .any(|cmd| matches!(cmd, PathCommand::QuadTo { .. }))
}

/// Check if a path contains any CubicTo commands.
///
/// Used to determine whether to use Loop-Blinn cubic curve rendering.
fn has_cubic_bezier(commands: &[PathCommand]) -> bool {
    commands
        .iter()
        .any(|cmd| matches!(cmd, PathCommand::CubicTo { .. }))
}

/// Uniform data for a draw batch.
///
/// This enum holds the shader uniform data. For batching, two nodes can only
/// be merged if their `UniformData` is byte-wise identical.
#[derive(Debug, Clone)]
pub enum UniformData {
    Solid(SolidColorUniform),
    LinearGradient(GradientUniform),
    RadialGradient(RadialGradientUniform),
    /// External texture reference (Phase J-3).
    /// Stores texture_id for lookup in GpuRenderer::external_textures.
    /// Actual bind group creation happens at render time.
    Texture {
        /// Texture ID from FillStyle::ExternalTexture.
        texture_id: u64,
    },
}

impl UniformData {
    /// Get the pipeline key for this uniform data.
    pub fn pipeline_key(&self) -> PipelineKey {
        match self {
            UniformData::Solid(_) => PipelineKey::Solid,
            UniformData::LinearGradient(_) => PipelineKey::LinearGradient,
            UniformData::RadialGradient(_) => PipelineKey::RadialGradient,
            UniformData::Texture { .. } => PipelineKey::Texture,
        }
    }
}

// Implement PartialEq by comparing bytes (Pod types)
impl PartialEq for UniformData {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (UniformData::Solid(a), UniformData::Solid(b)) => {
                bytemuck::bytes_of(a) == bytemuck::bytes_of(b)
            }
            (UniformData::LinearGradient(a), UniformData::LinearGradient(b)) => {
                bytemuck::bytes_of(a) == bytemuck::bytes_of(b)
            }
            (UniformData::RadialGradient(a), UniformData::RadialGradient(b)) => {
                bytemuck::bytes_of(a) == bytemuck::bytes_of(b)
            }
            (UniformData::Texture { texture_id: a }, UniformData::Texture { texture_id: b }) => {
                a == b
            }
            _ => false,
        }
    }
}

impl Eq for UniformData {}

impl std::hash::Hash for UniformData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash the discriminant
        std::mem::discriminant(self).hash(state);
        // Hash the bytes
        match self {
            UniformData::Solid(u) => bytemuck::bytes_of(u).hash(state),
            UniformData::LinearGradient(u) => bytemuck::bytes_of(u).hash(state),
            UniformData::RadialGradient(u) => bytemuck::bytes_of(u).hash(state),
            UniformData::Texture { texture_id } => texture_id.hash(state),
        }
    }
}

// =============================================================================
// Batch Key (for HashMap grouping)
// =============================================================================

/// Key for grouping draw calls into batches.
///
/// Two draw calls can be merged if they have identical `BatchKey`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BatchKey {
    pipeline: PipelineKey,
    /// Hash of uniform data (for fast lookup)
    uniform_hash: u64,
    stencil_ref: u32,
    /// Opacity as integer bits (for exact comparison)
    opacity_bits: u32,
}

impl BatchKey {
    fn new(uniform_data: &UniformData, stencil_ref: u32, opacity: f32) -> Self {
        Self::with_pipeline(
            uniform_data.pipeline_key(),
            uniform_data,
            stencil_ref,
            opacity,
        )
    }

    fn new_loop_blinn(uniform_data: &UniformData, stencil_ref: u32, opacity: f32) -> Self {
        Self::with_pipeline(PipelineKey::LoopBlinn, uniform_data, stencil_ref, opacity)
    }

    fn new_loop_blinn_cubic(uniform_data: &UniformData, stencil_ref: u32, opacity: f32) -> Self {
        Self::with_pipeline(
            PipelineKey::LoopBlinnCubic,
            uniform_data,
            stencil_ref,
            opacity,
        )
    }

    fn new_sdf_stroke(uniform_data: &UniformData, stencil_ref: u32, opacity: f32) -> Self {
        Self::with_pipeline(PipelineKey::SdfStroke, uniform_data, stencil_ref, opacity)
    }

    fn new_sdf_stroke_cubic(uniform_data: &UniformData, stencil_ref: u32, opacity: f32) -> Self {
        Self::with_pipeline(
            PipelineKey::SdfStrokeCubic,
            uniform_data,
            stencil_ref,
            opacity,
        )
    }

    fn with_pipeline(
        pipeline: PipelineKey,
        uniform_data: &UniformData,
        stencil_ref: u32,
        opacity: f32,
    ) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        uniform_data.hash(&mut hasher);
        let uniform_hash = hasher.finish();

        // Clamp opacity to [0.0, 1.0] and map NaN → 1.0 (fully opaque fallback)
        // so that NaN inputs produce a deterministic, valid key instead of an
        // unpredictable bit pattern. Using 1.0 is safer than 0.0 as it preserves
        // visibility rather than making content invisible.
        let safe_opacity = if opacity.is_nan() {
            1.0f32
        } else {
            opacity.clamp(0.0, 1.0)
        };

        Self {
            pipeline,
            uniform_hash,
            stencil_ref,
            opacity_bits: safe_opacity.to_bits(),
        }
    }
}

// =============================================================================
// Draw Batch
// =============================================================================

/// A batch of draw calls with identical rendering state.
///
/// Contains merged vertex and index buffers from multiple `CanvasNode`s
/// that share the same pipeline, uniform data, stencil reference, and opacity.
#[derive(Debug, Clone)]
pub struct DrawBatch {
    /// Pipeline to use for this batch.
    pub pipeline_key: PipelineKey,

    /// Uniform data (color or gradient parameters).
    pub uniform_data: UniformData,

    /// Merged vertex buffer (world-space, pre-transformed).
    /// Can be either standard GpuVertex (for lyon tessellation) or
    /// LoopBlinnVertex (for quadratic Bezier curve rendering).
    pub vertices: BatchVertices,

    /// Merged index buffer (offset-corrected).
    pub indices: Vec<u32>,

    /// Transform uniform (identity + viewport for batched draws).
    pub transform: TransformUniform,

    /// Stencil reference value for clip path testing.
    pub stencil_ref: u32,

    /// Accumulated opacity.
    pub opacity: f32,
}

impl DrawBatch {
    /// Create a new empty batch with standard vertices.
    fn new(uniform_data: UniformData, stencil_ref: u32, opacity: f32) -> Self {
        Self {
            pipeline_key: uniform_data.pipeline_key(),
            uniform_data,
            vertices: BatchVertices::Standard(Vec::new()),
            indices: Vec::new(),
            transform: TransformUniform::identity(1.0, 1.0), // Updated in finalize
            stencil_ref,
            opacity,
        }
    }

    /// Create a new empty batch with Loop-Blinn vertices.
    fn new_loop_blinn(uniform_data: UniformData, stencil_ref: u32, opacity: f32) -> Self {
        Self {
            pipeline_key: PipelineKey::LoopBlinn,
            uniform_data,
            vertices: BatchVertices::LoopBlinn(Vec::new()),
            indices: Vec::new(),
            transform: TransformUniform::identity(1.0, 1.0), // Updated in finalize
            stencil_ref,
            opacity,
        }
    }

    /// Create a new empty batch with Loop-Blinn cubic vertices.
    fn new_loop_blinn_cubic(uniform_data: UniformData, stencil_ref: u32, opacity: f32) -> Self {
        Self {
            pipeline_key: PipelineKey::LoopBlinnCubic,
            uniform_data,
            vertices: BatchVertices::LoopBlinnCubic(Vec::new()),
            indices: Vec::new(),
            transform: TransformUniform::identity(1.0, 1.0), // Updated in finalize
            stencil_ref,
            opacity,
        }
    }

    /// Create a new empty batch with SDF stroke vertices (quadratic).
    fn new_sdf_stroke(uniform_data: UniformData, stencil_ref: u32, opacity: f32) -> Self {
        Self {
            pipeline_key: PipelineKey::SdfStroke,
            uniform_data,
            vertices: BatchVertices::SdfStroke(Vec::new()),
            indices: Vec::new(),
            transform: TransformUniform::identity(1.0, 1.0), // Updated in finalize
            stencil_ref,
            opacity,
        }
    }

    /// Create a new empty batch with SDF stroke cubic vertices.
    fn new_sdf_stroke_cubic(uniform_data: UniformData, stencil_ref: u32, opacity: f32) -> Self {
        Self {
            pipeline_key: PipelineKey::SdfStrokeCubic,
            uniform_data,
            vertices: BatchVertices::SdfStrokeCubic(Vec::new()),
            indices: Vec::new(),
            transform: TransformUniform::identity(1.0, 1.0), // Updated in finalize
            stencil_ref,
            opacity,
        }
    }

    /// Merge tessellation output into this batch.
    ///
    /// Applies the given transform to vertices on CPU and adjusts indices.
    fn merge(&mut self, tessellation: &TessellationOutput, transform: &AffineTransform) {
        let BatchVertices::Standard(ref mut vertices) = self.vertices else {
            // Should not happen: merging standard tessellation into LoopBlinn batch
            log::warn!("Attempting to merge standard tessellation into LoopBlinn batch");
            return;
        };

        let vertex_offset = vertices.len() as u32;

        // Transform vertices to world space and add to batch
        for vertex in &tessellation.vertices {
            let (wx, wy) =
                transform.transform_point(vertex.position[0] as f64, vertex.position[1] as f64);
            vertices.push(GpuVertex {
                position: [wx as f32, wy as f32],
                uv: vertex.uv,
            });
        }

        // Add indices with offset correction
        self.indices
            .extend(tessellation.indices.iter().map(|i| i + vertex_offset));
    }

    /// Merge Loop-Blinn tessellation output into this batch.
    ///
    /// Applies the given transform to vertices on CPU and adjusts indices.
    fn merge_loop_blinn(&mut self, output: &LoopBlinnOutput, transform: &AffineTransform) {
        let BatchVertices::LoopBlinn(ref mut vertices) = self.vertices else {
            // Should not happen: merging LoopBlinn tessellation into standard batch
            log::warn!("Attempting to merge LoopBlinn tessellation into standard batch");
            return;
        };

        let vertex_offset = vertices.len() as u32;

        // Transform vertices to world space and add to batch
        for vertex in &output.vertices {
            let (wx, wy) =
                transform.transform_point(vertex.position[0] as f64, vertex.position[1] as f64);
            vertices.push(LoopBlinnVertex {
                position: [wx as f32, wy as f32],
                curve_uv: vertex.curve_uv,
                curve_sign: vertex.curve_sign,
            });
        }

        // Add indices with offset correction
        self.indices
            .extend(output.indices.iter().map(|i| i + vertex_offset));
    }

    /// Merge cubic Loop-Blinn tessellation output into this batch.
    ///
    /// Applies the given transform to vertices on CPU and adjusts indices.
    fn merge_cubic_loop_blinn(
        &mut self,
        output: &CubicLoopBlinnOutput,
        transform: &AffineTransform,
    ) {
        let BatchVertices::LoopBlinnCubic(ref mut vertices) = self.vertices else {
            // Should not happen: merging cubic LoopBlinn tessellation into wrong batch type
            log::warn!("Attempting to merge cubic LoopBlinn tessellation into non-cubic batch");
            return;
        };

        let vertex_offset = vertices.len() as u32;

        // Transform vertices to world space and add to batch
        for vertex in &output.vertices {
            let (wx, wy) =
                transform.transform_point(vertex.position[0] as f64, vertex.position[1] as f64);
            vertices.push(CubicLoopBlinnVertex {
                position: [wx as f32, wy as f32],
                curve_klm: vertex.curve_klm,
                curve_sign: vertex.curve_sign,
            });
        }

        // Add indices with offset correction
        self.indices
            .extend(output.indices.iter().map(|i| i + vertex_offset));
    }

    /// Merge SDF stroke tessellation output into this batch (quadratic).
    ///
    /// Coordinate transform strategy:
    /// - `position`: Transform to world space (for rasterization)
    /// - `local_pos`, `p0`, `p1`, `p2`, `half_width`: Keep in local space (for distance calculation)
    ///
    /// This ensures the SDF distance calculation is correct under non-uniform scaling.
    fn merge_sdf_stroke(&mut self, output: &SdfStrokeOutput, transform: &AffineTransform) {
        let BatchVertices::SdfStroke(ref mut vertices) = self.vertices else {
            log::warn!("Attempting to merge SdfStroke tessellation into non-SdfStroke batch");
            return;
        };

        let vertex_offset = vertices.len() as u32;

        // Transform only position to world space; keep other attributes in local space
        for vertex in &output.vertices {
            let (wx, wy) =
                transform.transform_point(vertex.position[0] as f64, vertex.position[1] as f64);
            vertices.push(SdfStrokeVertex {
                position: [wx as f32, wy as f32], // World space
                local_pos: vertex.local_pos,      // Local space (for SDF)
                p0: vertex.p0,                    // Local space
                p1: vertex.p1,                    // Local space
                p2: vertex.p2,                    // Local space
                half_width: vertex.half_width,    // Local space
            });
        }

        // Add indices with offset correction
        self.indices
            .extend(output.indices.iter().map(|i| i + vertex_offset));
    }

    /// Merge SDF stroke cubic tessellation output into this batch.
    ///
    /// Coordinate transform strategy:
    /// - `position`: Transform to world space (for rasterization)
    /// - `local_pos`, `p0`, `p1`, `p2`, `p3`, `half_width`: Keep in local space (for distance calculation)
    ///
    /// This ensures the SDF distance calculation is correct under non-uniform scaling.
    fn merge_sdf_stroke_cubic(
        &mut self,
        output: &CubicSdfStrokeOutput,
        transform: &AffineTransform,
    ) {
        let BatchVertices::SdfStrokeCubic(ref mut vertices) = self.vertices else {
            log::warn!(
                "Attempting to merge SdfStrokeCubic tessellation into non-SdfStrokeCubic batch"
            );
            return;
        };

        let vertex_offset = vertices.len() as u32;

        // Transform only position to world space; keep other attributes in local space
        for vertex in &output.vertices {
            let (wx, wy) =
                transform.transform_point(vertex.position[0] as f64, vertex.position[1] as f64);
            vertices.push(CubicSdfStrokeVertex::new(
                [wx as f32, wy as f32], // World space
                vertex.local_pos,       // Local space (for SDF)
                vertex.p0,              // Local space
                vertex.p1,              // Local space
                vertex.p2,              // Local space
                vertex.p3,              // Local space
                vertex.half_width,      // Local space
            ));
        }

        // Add indices with offset correction
        self.indices
            .extend(output.indices.iter().map(|i| i + vertex_offset));
    }

    /// Check if the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Get the number of triangles in this batch.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Create GPU resources for this batch.
    ///
    /// Allocates vertex and index buffers, creates uniform buffers,
    /// and builds bind groups for the batch's uniform data.
    ///
    /// ## Parameters
    ///
    /// - `device`: wgpu device for resource creation
    /// - `pipeline_set`: Pipeline set matching this batch's `pipeline_key`
    pub fn create_gpu_resources(
        &self,
        device: &wgpu::Device,
        pipeline_set: &PipelineSet,
    ) -> GpuBatchResources {
        // Create vertex buffer based on vertex type
        let vertex_buffer = match &self.vertices {
            BatchVertices::Standard(vertices) => {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Batch Vertex Buffer (Standard)"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            }
            BatchVertices::LoopBlinn(vertices) => {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Batch Vertex Buffer (LoopBlinn)"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            }
            BatchVertices::LoopBlinnCubic(vertices) => {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Batch Vertex Buffer (LoopBlinnCubic)"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            }
            BatchVertices::SdfStroke(vertices) => {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Batch Vertex Buffer (SdfStroke)"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            }
            BatchVertices::SdfStrokeCubic(vertices) => {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Batch Vertex Buffer (SdfStrokeCubic)"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            }
        };

        // Create index buffer
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Batch Index Buffer"),
            contents: bytemuck::cast_slice(&self.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Create transform uniform buffer
        let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Batch Transform Uniform Buffer"),
            contents: bytemuck::bytes_of(&self.transform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create style uniform buffer based on uniform_data type
        let style_buffer = match &self.uniform_data {
            UniformData::Solid(uniform) => {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Batch Solid Color Uniform Buffer"),
                    contents: bytemuck::bytes_of(uniform),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            }
            UniformData::LinearGradient(uniform) => {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Batch Linear Gradient Uniform Buffer"),
                    contents: bytemuck::bytes_of(uniform),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            }
            UniformData::RadialGradient(uniform) => {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Batch Radial Gradient Uniform Buffer"),
                    contents: bytemuck::bytes_of(uniform),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            }
            UniformData::Texture { texture_id } => {
                // Texture pipeline requires texture + sampler bind group, not a uniform buffer.
                // This code path should not be reached - texture batches use create_gpu_resources_with_texture().
                log::error!(
                    "DrawBatch::create_gpu_resources called for Texture (id={}). \
                     Use create_gpu_resources_with_texture() instead.",
                    texture_id
                );
                // Create dummy buffer to satisfy type requirements (will render incorrectly)
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Dummy Texture Buffer (ERROR)"),
                    contents: &[0u8; 16],
                    usage: wgpu::BufferUsages::UNIFORM,
                })
            }
        };

        // Create bind groups
        let transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Batch Transform Bind Group"),
            layout: &pipeline_set.transform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buffer.as_entire_binding(),
            }],
        });

        let style_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Batch Style Bind Group"),
            layout: &pipeline_set.style_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: style_buffer.as_entire_binding(),
            }],
        });

        GpuBatchResources {
            vertex_buffer,
            index_buffer,
            index_count: self.indices.len() as u32,
            transform_bind_group,
            style_bind_group,
            stencil_ref: self.stencil_ref,
        }
    }

    /// Create GPU resources for a texture batch.
    ///
    /// This method handles `UniformData::Texture` batches which require a
    /// texture view and sampler for the style bind group instead of a uniform buffer.
    ///
    /// ## Parameters
    ///
    /// - `device`: wgpu device for resource creation
    /// - `pipeline_set`: Pipeline set containing bind group layouts
    /// - `texture_view`: External texture view to sample from
    /// - `sampler`: Sampler for texture filtering
    pub fn create_gpu_resources_with_texture(
        &self,
        device: &wgpu::Device,
        pipeline_set: &PipelineSet,
        texture_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> GpuBatchResources {
        // Create vertex buffer based on vertex type (same as create_gpu_resources)
        let vertex_buffer = match &self.vertices {
            BatchVertices::Standard(vertices) => {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Texture Batch Vertex Buffer"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            }
            _ => {
                log::warn!("Texture batch with non-standard vertices, using empty buffer");
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Texture Batch Vertex Buffer (Empty)"),
                    contents: &[],
                    usage: wgpu::BufferUsages::VERTEX,
                })
            }
        };

        // Create index buffer
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Texture Batch Index Buffer"),
            contents: bytemuck::cast_slice(&self.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Create transform uniform buffer
        let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Texture Batch Transform Uniform Buffer"),
            contents: bytemuck::bytes_of(&self.transform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create bind groups
        let transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Texture Batch Transform Bind Group"),
            layout: &pipeline_set.transform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buffer.as_entire_binding(),
            }],
        });

        // Create texture style bind group with texture view and sampler
        let style_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Texture Batch Style Bind Group"),
            layout: &pipeline_set.style_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        GpuBatchResources {
            vertex_buffer,
            index_buffer,
            index_count: self.indices.len() as u32,
            transform_bind_group,
            style_bind_group,
            stencil_ref: self.stencil_ref,
        }
    }

    /// Get the texture ID if this is a texture batch.
    pub fn texture_id(&self) -> Option<u64> {
        match &self.uniform_data {
            UniformData::Texture { texture_id } => Some(*texture_id),
            _ => None,
        }
    }
}

// =============================================================================
// GPU Batch Resources
// =============================================================================

/// GPU-side resources for a draw batch.
///
/// Contains all resources needed to issue a single GPU draw call
/// for a batch of merged geometry.
pub struct GpuBatchResources {
    /// Vertex buffer containing merged, world-space transformed vertices.
    pub vertex_buffer: wgpu::Buffer,

    /// Index buffer with offset-corrected indices.
    pub index_buffer: wgpu::Buffer,

    /// Number of indices to draw.
    pub index_count: u32,

    /// Bind group for transform uniform (identity + viewport conversion).
    pub transform_bind_group: wgpu::BindGroup,

    /// Bind group for style uniform (color or gradient).
    pub style_bind_group: wgpu::BindGroup,

    /// Stencil reference value for clip path testing.
    pub stencil_ref: u32,
}

// =============================================================================
// Draw Batcher
// =============================================================================

/// Collects and batches draw calls from a `CanvasNode` tree.
///
/// ## Z-Order Preservation
///
/// Batching only merges CONSECUTIVE nodes with identical `BatchKey`.
/// Non-consecutive nodes with the same key are kept in separate batches
/// to preserve painter's algorithm z-ordering.
///
/// Example:
/// ```text
/// Input:  red(z=1), blue(z=2), red(z=3)
/// Output: 3 batches: [red], [blue], [red]  // NOT 2 batches!
/// ```
///
/// ## Usage
///
/// ```ignore
/// let mut batcher = DrawBatcher::new();
/// batcher.collect(&canvas_nodes, &pipeline_manager);
/// let batches = batcher.drain();
///
/// // Issue one draw call per batch instead of per node
/// for batch in batches {
///     render_batch(&batch);
/// }
/// ```
pub struct DrawBatcher {
    /// Batches in draw order.
    batches: Vec<DrawBatch>,

    /// Current batch key (for consecutive merging).
    current_key: Option<BatchKey>,

    /// Viewport dimensions for transform uniform.
    viewport_width: f32,
    viewport_height: f32,
}

impl DrawBatcher {
    /// Create a new empty batcher.
    pub fn new() -> Self {
        Self {
            batches: Vec::new(),
            current_key: None,
            viewport_width: 1.0,
            viewport_height: 1.0,
        }
    }

    /// Collect draw batches from a `CanvasNode` tree.
    ///
    /// Walks the tree depth-first, tessellating each path node and grouping
    /// them into batches by rendering state.
    ///
    /// ## Contract
    ///
    /// **Input ordering**: Nodes are assumed to be in z-order (painter's algorithm)
    /// ascending order. The caller is responsible for sorting by `z_order` before
    /// calling this method. Batching only merges **consecutive** nodes with identical
    /// `BatchKey`; z-order is preserved by never merging non-consecutive nodes even
    /// when their keys are equal.
    ///
    /// ## Parameters
    ///
    /// - `nodes`: Root-level canvas nodes (must be pre-sorted by z-order)
    /// - `_pipeline_manager`: Pipeline manager (for future use)
    /// - `viewport_width`: Viewport width in device pixels
    /// - `viewport_height`: Viewport height in device pixels
    pub fn collect(
        &mut self,
        nodes: &[CanvasNode],
        _pipeline_manager: &PipelineManager,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        self.collect_nodes(nodes, viewport_width, viewport_height);
    }

    /// Collect draw batches from nodes (internal implementation).
    ///
    /// This is the core collection logic, separated for testability.
    ///
    /// ## Contract
    ///
    /// **Input is assumed to be z-order sorted** (ascending). Batching merges only
    /// consecutive nodes with identical `BatchKey` to preserve painter's algorithm
    /// ordering. Non-consecutive identical keys produce separate batches.
    pub fn collect_nodes(
        &mut self,
        nodes: &[CanvasNode],
        viewport_width: f32,
        viewport_height: f32,
    ) {
        self.viewport_width = viewport_width;
        self.viewport_height = viewport_height;

        // Initialize stacks
        let mut transform_stack = TransformStack::new();
        let mut opacity_stack = OpacityStack::new();
        let mut stencil_stack = StencilStack::new();

        // Walk tree depth-first
        for node in nodes {
            self.collect_node(
                node,
                &mut transform_stack,
                &mut opacity_stack,
                &mut stencil_stack,
            );
        }
    }

    /// Collect batches from a single node and its children.
    fn collect_node(
        &mut self,
        node: &CanvasNode,
        transform_stack: &mut TransformStack,
        opacity_stack: &mut OpacityStack,
        stencil_stack: &mut StencilStack,
    ) {
        match node {
            CanvasNode::Group(group) => {
                self.collect_group(group, transform_stack, opacity_stack, stencil_stack);
            }
            CanvasNode::Path(path) => {
                self.collect_path(path, transform_stack, opacity_stack, stencil_stack);
            }
            CanvasNode::Text(_) => {
                // Text batching deferred to future track
            }
            CanvasNode::Image(_) => {
                // Image batching deferred to future track
            }
        }
    }

    /// Collect batches from a group node.
    fn collect_group(
        &mut self,
        group: &CanvasGroupNode,
        transform_stack: &mut TransformStack,
        opacity_stack: &mut OpacityStack,
        stencil_stack: &mut StencilStack,
    ) {
        // Push group transform
        transform_stack.push(&group.transform);
        opacity_stack.push(group.opacity as f32);

        // Handle clip path (stencil increments, but we don't tessellate clip paths here)
        if group.clip_path.is_some() {
            stencil_stack.push();
        }

        // Collect children
        for child in &group.children {
            self.collect_node(child, transform_stack, opacity_stack, stencil_stack);
        }

        // Pop state
        if group.clip_path.is_some() {
            stencil_stack.pop();
        }
        opacity_stack.pop();
        transform_stack.pop();
    }

    /// Collect batches from a path node.
    fn collect_path(
        &mut self,
        path: &CanvasPathNode,
        transform_stack: &TransformStack,
        opacity_stack: &OpacityStack,
        stencil_stack: &StencilStack,
    ) {
        // Skip if invisible
        if opacity_stack.is_invisible() {
            return;
        }

        let world_transform = transform_stack.current();
        let opacity = opacity_stack.current();
        let stencil_ref = stencil_stack.current();

        // Collect fill
        if let Some(ref fill) = path.fill {
            let uniform_data = self.fill_to_uniform(fill);

            if has_cubic_bezier(&path.path_data) {
                // Path contains CubicTo: use Loop-Blinn cubic for curves
                let cubic_output = tessellate_cubic_beziers(&path.path_data);

                // Add Loop-Blinn cubic curve triangles (if any non-degenerate curves)
                if !cubic_output.vertices.is_empty() {
                    self.add_cubic_loop_blinn_batch(
                        uniform_data.clone(),
                        &cubic_output,
                        &world_transform,
                        stencil_ref,
                        opacity,
                    );
                }

                // Tessellate interior (with CubicTo→LineTo) using lyon
                if let Ok(tessellation) =
                    tessellate_path(&cubic_output.interior_commands, Some(fill))
                {
                    if !tessellation.is_empty() {
                        self.add_to_batch(
                            uniform_data,
                            &tessellation,
                            &world_transform,
                            stencil_ref,
                            opacity,
                        );
                    }
                }
            } else if has_quadratic_bezier(&path.path_data) {
                // Path contains QuadTo: use Loop-Blinn for curves
                let loop_blinn_output = tessellate_quadratic_beziers(&path.path_data);

                // Add Loop-Blinn curve triangles (if any non-degenerate curves)
                if !loop_blinn_output.vertices.is_empty() {
                    self.add_loop_blinn_batch(
                        uniform_data.clone(),
                        &loop_blinn_output,
                        &world_transform,
                        stencil_ref,
                        opacity,
                    );
                }

                // Tessellate interior (with QuadTo→LineTo) using lyon
                if let Ok(tessellation) =
                    tessellate_path(&loop_blinn_output.interior_commands, Some(fill))
                {
                    if !tessellation.is_empty() {
                        self.add_to_batch(
                            uniform_data,
                            &tessellation,
                            &world_transform,
                            stencil_ref,
                            opacity,
                        );
                    }
                }
            } else {
                // No curves: use standard tessellation
                if let Ok(tessellation) = tessellate_path(&path.path_data, Some(fill)) {
                    if !tessellation.is_empty() {
                        self.add_to_batch(
                            uniform_data,
                            &tessellation,
                            &world_transform,
                            stencil_ref,
                            opacity,
                        );
                    }
                }
            }
        }

        // Collect stroke
        if let Some(ref stroke) = path.stroke {
            // Strokes use solid color
            let uniform_data = self.rgba_to_uniform(stroke.rgba);
            let stroke_width = stroke.width.to_f64_for_rasterization() as f32;

            let has_quad = has_quadratic_bezier(&path.path_data);
            let has_cubic = has_cubic_bezier(&path.path_data);

            if has_quad {
                // Path contains QuadTo: use SDF stroke rendering (quadratic)
                let sdf_output = tessellate_stroke_segments(&path.path_data, stroke_width);

                if !sdf_output.is_empty() {
                    self.add_sdf_stroke_batch(
                        uniform_data.clone(),
                        &sdf_output,
                        &world_transform,
                        stencil_ref,
                        opacity,
                    );
                }
            }

            if has_cubic {
                // Path contains CubicTo: use SDF stroke rendering (cubic)
                let sdf_cubic_output =
                    tessellate_cubic_stroke_segments(&path.path_data, stroke_width);

                if !sdf_cubic_output.is_empty() {
                    self.add_sdf_stroke_cubic_batch(
                        uniform_data.clone(),
                        &sdf_cubic_output,
                        &world_transform,
                        stencil_ref,
                        opacity,
                    );
                }
            }

            if !has_quad && !has_cubic {
                // No curves: use standard lyon tessellation
                if let Ok(tessellation) = tessellate_path_stroke(&path.path_data, stroke) {
                    if !tessellation.is_empty() {
                        self.add_to_batch(
                            uniform_data,
                            &tessellation,
                            &world_transform,
                            stencil_ref,
                            opacity,
                        );
                    }
                }
            }
        }
    }

    /// Add tessellation to the appropriate batch.
    ///
    /// Only merges with the current batch if BatchKey matches.
    /// Otherwise, creates a new batch to preserve z-order.
    fn add_to_batch(
        &mut self,
        uniform_data: UniformData,
        tessellation: &TessellationOutput,
        transform: &AffineTransform,
        stencil_ref: u32,
        opacity: f32,
    ) {
        let key = BatchKey::new(&uniform_data, stencil_ref, opacity);

        // Check if we can merge with the current (last) batch
        let should_create_new = match &self.current_key {
            Some(current) if current == &key => false, // Same key, merge
            _ => true,                                 // Different key or no current batch
        };

        if should_create_new {
            // Create new batch
            self.batches
                .push(DrawBatch::new(uniform_data.clone(), stencil_ref, opacity));
            self.current_key = Some(key);
        }

        // Merge into current (last) batch
        if let Some(batch) = self.batches.last_mut() {
            batch.merge(tessellation, transform);
        }
    }

    /// Add Loop-Blinn tessellation to the appropriate batch.
    ///
    /// Similar to `add_to_batch` but for Loop-Blinn curve vertices.
    /// Uses `PipelineKey::LoopBlinn` for the batch key.
    fn add_loop_blinn_batch(
        &mut self,
        uniform_data: UniformData,
        output: &LoopBlinnOutput,
        transform: &AffineTransform,
        stencil_ref: u32,
        opacity: f32,
    ) {
        let key = BatchKey::new_loop_blinn(&uniform_data, stencil_ref, opacity);

        // Check if we can merge with the current (last) batch
        let should_create_new = match &self.current_key {
            Some(current) if current == &key => false, // Same key, merge
            _ => true,                                 // Different key or no current batch
        };

        if should_create_new {
            // Create new Loop-Blinn batch
            self.batches.push(DrawBatch::new_loop_blinn(
                uniform_data.clone(),
                stencil_ref,
                opacity,
            ));
            self.current_key = Some(key);
        }

        // Merge into current (last) batch
        if let Some(batch) = self.batches.last_mut() {
            batch.merge_loop_blinn(output, transform);
        }
    }

    /// Add cubic Loop-Blinn tessellation to the appropriate batch.
    ///
    /// Similar to `add_to_batch` but for Loop-Blinn cubic curve vertices.
    /// Uses `PipelineKey::LoopBlinnCubic` for the batch key.
    fn add_cubic_loop_blinn_batch(
        &mut self,
        uniform_data: UniformData,
        output: &CubicLoopBlinnOutput,
        transform: &AffineTransform,
        stencil_ref: u32,
        opacity: f32,
    ) {
        let key = BatchKey::new_loop_blinn_cubic(&uniform_data, stencil_ref, opacity);

        // Check if we can merge with the current (last) batch
        let should_create_new = match &self.current_key {
            Some(current) if current == &key => false, // Same key, merge
            _ => true,                                 // Different key or no current batch
        };

        if should_create_new {
            // Create new cubic Loop-Blinn batch
            self.batches.push(DrawBatch::new_loop_blinn_cubic(
                uniform_data.clone(),
                stencil_ref,
                opacity,
            ));
            self.current_key = Some(key);
        }

        // Merge into current (last) batch
        if let Some(batch) = self.batches.last_mut() {
            batch.merge_cubic_loop_blinn(output, transform);
        }
    }

    /// Add SDF stroke tessellation to the appropriate batch.
    ///
    /// Similar to `add_to_batch` but for SDF stroke vertices.
    /// Uses `PipelineKey::SdfStroke` for the batch key.
    fn add_sdf_stroke_batch(
        &mut self,
        uniform_data: UniformData,
        output: &SdfStrokeOutput,
        transform: &AffineTransform,
        stencil_ref: u32,
        opacity: f32,
    ) {
        let key = BatchKey::new_sdf_stroke(&uniform_data, stencil_ref, opacity);

        // Check if we can merge with the current (last) batch
        let should_create_new = match &self.current_key {
            Some(current) if current == &key => false, // Same key, merge
            _ => true,                                 // Different key or no current batch
        };

        if should_create_new {
            // Create new SDF stroke batch
            self.batches.push(DrawBatch::new_sdf_stroke(
                uniform_data.clone(),
                stencil_ref,
                opacity,
            ));
            self.current_key = Some(key);
        }

        // Merge into current (last) batch
        if let Some(batch) = self.batches.last_mut() {
            batch.merge_sdf_stroke(output, transform);
        }
    }

    /// Add SDF stroke cubic tessellation to the appropriate batch.
    ///
    /// Similar to `add_sdf_stroke_batch` but for cubic Bezier curves.
    /// Uses `PipelineKey::SdfStrokeCubic` for the batch key.
    fn add_sdf_stroke_cubic_batch(
        &mut self,
        uniform_data: UniformData,
        output: &CubicSdfStrokeOutput,
        transform: &AffineTransform,
        stencil_ref: u32,
        opacity: f32,
    ) {
        let key = BatchKey::new_sdf_stroke_cubic(&uniform_data, stencil_ref, opacity);

        // Check if we can merge with the current (last) batch
        let should_create_new = match &self.current_key {
            Some(current) if current == &key => false, // Same key, merge
            _ => true,                                 // Different key or no current batch
        };

        if should_create_new {
            // Create new SDF stroke cubic batch
            self.batches.push(DrawBatch::new_sdf_stroke_cubic(
                uniform_data.clone(),
                stencil_ref,
                opacity,
            ));
            self.current_key = Some(key);
        }

        // Merge into current (last) batch
        if let Some(batch) = self.batches.last_mut() {
            batch.merge_sdf_stroke_cubic(output, transform);
        }
    }

    /// Convert FillStyle to UniformData.
    fn fill_to_uniform(&self, fill: &FillStyle) -> UniformData {
        match fill {
            FillStyle::Solid { rgba } => self.rgba_to_uniform(*rgba),
            FillStyle::LinearGradient { stops, start, end } => {
                let stop_uniforms = Self::convert_gradient_stops(stops);
                let gradient = GradientUniform::from_linear_gradient_points(
                    start.as_ref(),
                    end.as_ref(),
                    &stop_uniforms,
                );
                UniformData::LinearGradient(gradient)
            }
            FillStyle::RadialGradient {
                stops,
                center,
                radius,
            } => {
                let stop_uniforms = Self::convert_gradient_stops(stops);
                let radial = RadialGradientUniform::from_radial_gradient(
                    center.as_ref(),
                    radius.as_ref(),
                    &stop_uniforms,
                );
                UniformData::RadialGradient(radial)
            }
            FillStyle::Pattern { .. } => {
                // Fallback to black for unsupported patterns
                UniformData::Solid(SolidColorUniform::new(0.0, 0.0, 0.0, 1.0))
            }
            FillStyle::ExternalTexture { texture_id, .. } => UniformData::Texture {
                texture_id: *texture_id,
            },
        }
    }

    /// Convert RGBA bytes to UniformData.
    fn rgba_to_uniform(&self, rgba: [u8; 4]) -> UniformData {
        let uniform = SolidColorUniform::from_rgba(rgba);
        UniformData::Solid(uniform)
    }

    /// Convert GradientStop to GradientStopUniform.
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

    /// Drain collected batches.
    ///
    /// Returns batches in draw order with finalized transform uniforms.
    /// Clears the batcher for reuse.
    pub fn drain(&mut self) -> Vec<DrawBatch> {
        // Finalize transform uniforms for each batch
        for batch in &mut self.batches {
            batch.transform = TransformUniform::identity(self.viewport_width, self.viewport_height);
            batch.transform.opacity = batch.opacity;
        }

        // Take batches, leaving empty vec
        let result: Vec<DrawBatch> = self.batches.drain(..).filter(|b| !b.is_empty()).collect();

        self.current_key = None;
        result
    }

    /// Clear the batcher without returning batches.
    pub fn clear(&mut self) {
        self.batches.clear();
        self.current_key = None;
    }

    /// Get the number of pending batches.
    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }
}

impl Default for DrawBatcher {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CanvasNodeBase, PVector, PVectorBounds, PathCommand};
    use vsc_core::{EntityId, Rational};

    /// Helper to create a simple rectangle path node.
    fn create_rect_path(
        entity_id: u64,
        rgba: [u8; 4],
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> CanvasPathNode {
        CanvasPathNode {
            base: CanvasNodeBase {
                entity_id: EntityId(entity_id),
                bounds: PVectorBounds {
                    top_left: PVector {
                        x: Rational::from_int(x as i64),
                        y: Rational::from_int(y as i64),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                    bottom_right: PVector {
                        x: Rational::from_int((x + w) as i64),
                        y: Rational::from_int((y + h) as i64),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: String::new(),
            },
            path_data: vec![
                PathCommand::MoveTo {
                    x: Rational::from_int(x as i64),
                    y: Rational::from_int(y as i64),
                },
                PathCommand::LineTo {
                    x: Rational::from_int((x + w) as i64),
                    y: Rational::from_int(y as i64),
                },
                PathCommand::LineTo {
                    x: Rational::from_int((x + w) as i64),
                    y: Rational::from_int((y + h) as i64),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(x as i64),
                    y: Rational::from_int((y + h) as i64),
                },
                PathCommand::Close,
            ],
            fill: Some(FillStyle::Solid { rgba }),
            stroke: None,
        }
    }

    #[test]
    fn test_same_color_batches_together() {
        let mut batcher = DrawBatcher::new();

        // Create 3 CONSECUTIVE rectangles with the same color
        let nodes = vec![
            CanvasNode::Path(create_rect_path(
                1,
                [255, 0, 0, 255],
                0.0,
                0.0,
                100.0,
                100.0,
            )),
            CanvasNode::Path(create_rect_path(
                2,
                [255, 0, 0, 255],
                100.0,
                0.0,
                100.0,
                100.0,
            )),
            CanvasNode::Path(create_rect_path(
                3,
                [255, 0, 0, 255],
                200.0,
                0.0,
                100.0,
                100.0,
            )),
        ];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Should produce exactly 1 batch (consecutive same-color)
        assert_eq!(
            batches.len(),
            1,
            "Expected 1 batch for consecutive same-color rects"
        );

        // Verify batch contains vertices from all 3 rectangles
        let batch = &batches[0];
        assert_eq!(batch.pipeline_key, PipelineKey::Solid);

        // Each rectangle produces 4 vertices
        assert!(
            batch.vertices.len() >= 12,
            "Expected at least 12 vertices (4 per rect), got {}",
            batch.vertices.len()
        );
    }

    #[test]
    fn test_different_colors_separate_batches() {
        let mut batcher = DrawBatcher::new();

        // Create 2 rectangles with different colors
        let nodes = vec![
            CanvasNode::Path(create_rect_path(
                1,
                [255, 0, 0, 255],
                0.0,
                0.0,
                100.0,
                100.0,
            )),
            CanvasNode::Path(create_rect_path(
                2,
                [0, 255, 0, 255],
                100.0,
                0.0,
                100.0,
                100.0,
            )),
        ];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Should produce exactly 2 batches
        assert_eq!(batches.len(), 2, "Expected 2 batches for different colors");

        // Both should be solid pipelines
        assert!(batches.iter().all(|b| b.pipeline_key == PipelineKey::Solid));
    }

    #[test]
    fn test_index_offset_correction() {
        let mut batcher = DrawBatcher::new();

        // Create 2 rectangles with same color (consecutive = merged)
        let nodes = vec![
            CanvasNode::Path(create_rect_path(
                1,
                [255, 0, 0, 255],
                0.0,
                0.0,
                100.0,
                100.0,
            )),
            CanvasNode::Path(create_rect_path(
                2,
                [255, 0, 0, 255],
                100.0,
                0.0,
                100.0,
                100.0,
            )),
        ];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        assert_eq!(batches.len(), 1);
        let batch = &batches[0];

        // Verify indices are valid (all indices should be < vertex count)
        let vertex_count = batch.vertices.len() as u32;
        for &index in &batch.indices {
            assert!(
                index < vertex_count,
                "Index {} exceeds vertex count {}",
                index,
                vertex_count
            );
        }

        // All indices must form complete triangles
        assert!(
            batch.indices.len() % 3 == 0,
            "Indices should form complete triangles"
        );
    }

    #[test]
    fn test_transform_applied_to_vertices() {
        let mut batcher = DrawBatcher::new();

        // Create a group with transform containing a rectangle
        let rect = create_rect_path(1, [255, 0, 0, 255], 0.0, 0.0, 100.0, 100.0);
        let group = CanvasGroupNode {
            base: CanvasNodeBase {
                entity_id: EntityId(100),
                bounds: PVectorBounds {
                    top_left: PVector::zero(),
                    bottom_right: PVector {
                        x: Rational::from_int(100),
                        y: Rational::from_int(100),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: String::new(),
            },
            transform: AffineTransform::translation(50.0, 50.0),
            opacity: 1.0,
            children: vec![CanvasNode::Path(rect)],
            clip_path: None,
        };

        let nodes = vec![CanvasNode::Group(group)];
        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        assert_eq!(batches.len(), 1);
        let batch = &batches[0];

        // Verify vertices are transformed (should be offset by 50, 50)
        // Original rect is at (0,0)-(100,100), transformed should be (50,50)-(150,150)
        let vertices = batch
            .vertices
            .as_standard()
            .expect("Expected standard vertices");
        let min_x = vertices
            .iter()
            .map(|v| v.position[0])
            .fold(f32::INFINITY, f32::min);
        let min_y = vertices
            .iter()
            .map(|v| v.position[1])
            .fold(f32::INFINITY, f32::min);

        // Allow some tolerance for tessellation artifacts
        assert!(
            (min_x - 50.0).abs() < 1.0,
            "Expected min_x ~50, got {}",
            min_x
        );
        assert!(
            (min_y - 50.0).abs() < 1.0,
            "Expected min_y ~50, got {}",
            min_y
        );
    }

    #[test]
    fn test_different_opacity_separate_batches() {
        let mut batcher = DrawBatcher::new();

        // Create 2 groups with same color but different opacity
        let rect1 = create_rect_path(1, [255, 0, 0, 255], 0.0, 0.0, 100.0, 100.0);
        let rect2 = create_rect_path(2, [255, 0, 0, 255], 100.0, 0.0, 100.0, 100.0);

        let group1 = CanvasGroupNode {
            base: CanvasNodeBase {
                entity_id: EntityId(100),
                bounds: PVectorBounds {
                    top_left: PVector::zero(),
                    bottom_right: PVector {
                        x: Rational::from_int(100),
                        y: Rational::from_int(100),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: String::new(),
            },
            transform: AffineTransform::identity(),
            opacity: 1.0,
            children: vec![CanvasNode::Path(rect1)],
            clip_path: None,
        };

        let group2 = CanvasGroupNode {
            base: CanvasNodeBase {
                entity_id: EntityId(101),
                bounds: PVectorBounds {
                    top_left: PVector {
                        x: Rational::from_int(100),
                        y: Rational::zero(),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                    bottom_right: PVector {
                        x: Rational::from_int(200),
                        y: Rational::from_int(100),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: String::new(),
            },
            transform: AffineTransform::identity(),
            opacity: 0.5, // Different opacity
            children: vec![CanvasNode::Path(rect2)],
            clip_path: None,
        };

        let nodes = vec![CanvasNode::Group(group1), CanvasNode::Group(group2)];
        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Should produce 2 batches (different opacity)
        assert_eq!(
            batches.len(),
            2,
            "Expected 2 batches for different opacities"
        );
    }

    #[test]
    fn test_empty_batcher() {
        let mut batcher = DrawBatcher::new();
        let batches = batcher.drain();
        assert!(batches.is_empty());
    }

    #[test]
    fn test_batch_key_equality() {
        let uniform1 = UniformData::Solid(SolidColorUniform::new(1.0, 0.0, 0.0, 1.0));
        let uniform2 = UniformData::Solid(SolidColorUniform::new(1.0, 0.0, 0.0, 1.0));
        let uniform3 = UniformData::Solid(SolidColorUniform::new(0.0, 1.0, 0.0, 1.0));

        let key1 = BatchKey::new(&uniform1, 0, 1.0);
        let key2 = BatchKey::new(&uniform2, 0, 1.0);
        let key3 = BatchKey::new(&uniform3, 0, 1.0);

        assert_eq!(key1, key2, "Same uniform data should produce same key");
        assert_ne!(
            key1, key3,
            "Different uniform data should produce different key"
        );
    }

    /// Test z-order preservation: interleaved colors must NOT be merged.
    ///
    /// Scenario:
    ///   z=1: red A
    ///   z=2: blue B
    ///   z=3: red C
    ///
    /// Expected: 3 batches (red, blue, red) to preserve painter's algorithm.
    /// Bug case: 2 batches (red[A+C], blue[B]) - blue would overdraw red C.
    #[test]
    fn test_zorder_preserves_batch_separation() {
        let mut batcher = DrawBatcher::new();

        // Create interleaved color nodes (processed in tree order = z-order)
        // z=1: red, z=2: blue, z=3: red
        let nodes = vec![
            CanvasNode::Path(create_rect_path(
                1,
                [255, 0, 0, 255],
                0.0,
                0.0,
                100.0,
                100.0,
            )), // red A
            CanvasNode::Path(create_rect_path(
                2,
                [0, 0, 255, 255],
                50.0,
                50.0,
                100.0,
                100.0,
            )), // blue B
            CanvasNode::Path(create_rect_path(
                3,
                [255, 0, 0, 255],
                100.0,
                100.0,
                100.0,
                100.0,
            )), // red C
        ];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Print actual batch count for diagnosis
        log::debug!("Actual batch count: {}", batches.len());
        for (i, batch) in batches.iter().enumerate() {
            log::debug!(
                "Batch {}: pipeline={:?}, vertices={}, indices={}",
                i,
                batch.pipeline_key,
                batch.vertices.len(),
                batch.indices.len()
            );
        }

        // MUST be 3 batches to preserve z-order
        assert_eq!(
            batches.len(),
            3,
            "Expected 3 batches (red, blue, red) for z-order preservation, got {}",
            batches.len()
        );

        // Each batch should have roughly the same vertex count (one rect each)
        // If batch 0 has 2x the vertices, red A and C were incorrectly merged
        let v0 = batches[0].vertices.len();
        let v1 = batches[1].vertices.len();
        let v2 = batches[2].vertices.len();
        let _ = v1; // suppress warning

        // All batches should have similar vertex counts (single rectangle)
        assert!(
            (v0 as i32 - v2 as i32).abs() < 10,
            "First and third batch should have similar vertex counts (not merged). v0={}, v2={}",
            v0,
            v2
        );
    }

    /// Test batch count reduction: 10 consecutive same-color rectangles → 1 batch.
    ///
    /// This validates the Phase H optimization: instead of 10 draw calls,
    /// we should issue only 1 draw call with merged geometry.
    /// Test that UV coordinates after transform application remain in [0.0, 1.0].
    ///
    /// UV coordinates are set during tessellation relative to the path's own bounding
    /// box. When the batcher applies a translation transform to vertex positions, it
    /// must NOT touch UV coordinates (UVs are computed in path-local space).
    /// Therefore, UVs remain in [0.0, 1.0] regardless of the world-space position.
    #[test]
    fn test_uv_coordinates_in_range_after_transform() {
        let mut batcher = DrawBatcher::new();

        // Rectangle at (0,0)–(100,100), then translated by (500, 500) via group transform.
        // UV should still be in [0, 1] (path-local normalization is unaffected by world transform).
        let rect = create_rect_path(1, [255, 0, 0, 255], 0.0, 0.0, 100.0, 100.0);
        let group = CanvasGroupNode {
            base: CanvasNodeBase {
                entity_id: EntityId(100),
                bounds: PVectorBounds {
                    top_left: PVector::zero(),
                    bottom_right: PVector {
                        x: Rational::from_int(100),
                        y: Rational::from_int(100),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: String::new(),
            },
            transform: AffineTransform::translation(500.0, 500.0),
            opacity: 1.0,
            children: vec![CanvasNode::Path(rect)],
            clip_path: None,
        };

        let nodes = vec![CanvasNode::Group(group)];
        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        assert_eq!(batches.len(), 1);
        let batch = &batches[0];

        // Verify world-space positions are shifted
        let vertices = batch
            .vertices
            .as_standard()
            .expect("Expected standard vertices");
        let min_x = vertices
            .iter()
            .map(|v| v.position[0])
            .fold(f32::INFINITY, f32::min);
        assert!(
            min_x > 400.0,
            "World position should be shifted by 500, min_x={}",
            min_x
        );

        // Verify UV coordinates remain in [0.0, 1.0] (path-local, not world-local)
        for (i, vertex) in vertices.iter().enumerate() {
            assert!(
                vertex.uv[0] >= 0.0 && vertex.uv[0] <= 1.0,
                "Vertex[{}] UV.u out of range: {} (position.x={})",
                i,
                vertex.uv[0],
                vertex.position[0]
            );
            assert!(
                vertex.uv[1] >= 0.0 && vertex.uv[1] <= 1.0,
                "Vertex[{}] UV.v out of range: {} (position.y={})",
                i,
                vertex.uv[1],
                vertex.position[1]
            );
        }
    }

    /// Test that `BatchKey::new()` with NaN opacity produces opacity_bits=1.0
    /// (i.e., NaN is mapped to 1.0 via the safe-opacity guard for visibility preservation).
    #[test]
    fn test_batch_key_nan_opacity_becomes_one() {
        let uniform = UniformData::Solid(SolidColorUniform::new(1.0, 0.0, 0.0, 1.0));

        // NaN opacity
        let key_nan = BatchKey::new(&uniform, 0, f32::NAN);
        // 1.0 opacity
        let key_one = BatchKey::new(&uniform, 0, 1.0);

        // NaN must map to 1.0 bits, so keys must be equal
        assert_eq!(
            key_nan.opacity_bits, key_one.opacity_bits,
            "NaN opacity should be treated as 1.0 (got bits {} vs {})",
            key_nan.opacity_bits, key_one.opacity_bits
        );

        // Sanity: a different valid opacity (0.0) must produce a different key
        let key_zero = BatchKey::new(&uniform, 0, 0.0);
        assert_ne!(
            key_nan.opacity_bits, key_zero.opacity_bits,
            "NaN-mapped-to-1.0 should differ from opacity=0.0"
        );

        // Verify opacity > 1.0 is clamped
        let key_over = BatchKey::new(&uniform, 0, 2.0);
        let key_clamped = BatchKey::new(&uniform, 0, 1.0);
        assert_eq!(
            key_over.opacity_bits, key_clamped.opacity_bits,
            "opacity=2.0 should clamp to 1.0"
        );
    }

    #[test]
    fn test_batch_count_reduction_10_same_color() {
        let mut batcher = DrawBatcher::new();

        // Create 10 consecutive rectangles with the same color
        let nodes: Vec<CanvasNode> = (0..10)
            .map(|i| {
                CanvasNode::Path(create_rect_path(
                    i as u64,
                    [255, 0, 0, 255],
                    (i as f32) * 100.0,
                    0.0,
                    100.0,
                    100.0,
                ))
            })
            .collect();

        batcher.collect_nodes(&nodes, 1200.0, 600.0);
        let batches = batcher.drain();

        // Should produce exactly 1 batch (all consecutive same-color)
        assert_eq!(
            batches.len(),
            1,
            "Expected 1 batch for 10 consecutive same-color rects, got {}",
            batches.len()
        );

        // Verify merged geometry contains all 10 rectangles
        let batch = &batches[0];
        assert_eq!(batch.pipeline_key, PipelineKey::Solid);

        // Each rectangle produces 4 vertices, so 10 rects = 40 vertices
        assert!(
            batch.vertices.len() >= 40,
            "Expected at least 40 vertices (4 per rect * 10 rects), got {}",
            batch.vertices.len()
        );

        // Verify indices form complete triangles
        assert!(
            batch.indices.len() % 3 == 0,
            "Indices should form complete triangles"
        );

        // Each rectangle produces at least 2 triangles (6 indices)
        // 10 rects * 6 indices = 60 indices minimum
        assert!(
            batch.indices.len() >= 60,
            "Expected at least 60 indices (6 per rect * 10 rects), got {}",
            batch.indices.len()
        );

        log::debug!(
            "Batch count reduction: 10 rects merged into 1 batch with {} vertices, {} indices",
            batch.vertices.len(),
            batch.indices.len()
        );
    }

    // =========================================================================
    // Loop-Blinn Integration Tests
    // =========================================================================

    /// Helper to create a path with a quadratic Bezier curve.
    fn create_quad_curve_path(entity_id: u64, rgba: [u8; 4]) -> CanvasPathNode {
        CanvasPathNode {
            base: CanvasNodeBase {
                entity_id: EntityId(entity_id),
                bounds: PVectorBounds {
                    top_left: PVector::zero(),
                    bottom_right: PVector {
                        x: Rational::from_int(100),
                        y: Rational::from_int(100),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: String::new(),
            },
            path_data: vec![
                PathCommand::MoveTo {
                    x: Rational::from_int(0),
                    y: Rational::from_int(0),
                },
                PathCommand::QuadTo {
                    x1: Rational::from_int(50),
                    y1: Rational::from_int(100), // Control point above baseline
                    x: Rational::from_int(100),
                    y: Rational::from_int(0),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(100),
                    y: Rational::from_int(50),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(0),
                    y: Rational::from_int(50),
                },
                PathCommand::Close,
            ],
            fill: Some(FillStyle::Solid { rgba }),
            stroke: None,
        }
    }

    /// Test: Path with QuadTo produces 2 batches (Loop-Blinn curves + interior fill).
    ///
    /// A path containing a quadratic Bezier should be split into:
    /// 1. Loop-Blinn batch for curve triangles
    /// 2. Standard batch for interior fill (tessellated with QuadTo→LineTo)
    #[test]
    fn test_quadto_path_produces_two_batches() {
        let mut batcher = DrawBatcher::new();

        let path = create_quad_curve_path(1, [255, 0, 0, 255]);
        let nodes = vec![CanvasNode::Path(path)];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Should produce 2 batches: Loop-Blinn + Standard
        assert_eq!(
            batches.len(),
            2,
            "Expected 2 batches for QuadTo path (Loop-Blinn + interior), got {}",
            batches.len()
        );

        // Find the Loop-Blinn batch and Standard batch
        let loop_blinn_batch = batches
            .iter()
            .find(|b| b.pipeline_key == PipelineKey::LoopBlinn);
        let standard_batch = batches
            .iter()
            .find(|b| b.pipeline_key == PipelineKey::Solid);

        assert!(loop_blinn_batch.is_some(), "Expected a Loop-Blinn batch");
        assert!(
            standard_batch.is_some(),
            "Expected a Standard (solid) batch"
        );

        let loop_blinn = loop_blinn_batch.unwrap();
        let standard = standard_batch.unwrap();

        // Verify Loop-Blinn batch contains curve vertices
        assert!(
            loop_blinn.vertices.as_loop_blinn().is_some(),
            "Loop-Blinn batch should have LoopBlinn vertices"
        );
        let lb_vertices = loop_blinn.vertices.as_loop_blinn().unwrap();
        // One QuadTo = 3 vertices (one triangle)
        assert_eq!(
            lb_vertices.len(),
            3,
            "Expected 3 Loop-Blinn vertices for one curve"
        );

        // Verify standard batch contains interior fill vertices
        assert!(
            standard.vertices.as_standard().is_some(),
            "Standard batch should have Standard vertices"
        );
    }

    /// Test: Path without QuadTo produces 1 batch (standard only).
    ///
    /// A path with only MoveTo, LineTo, Close should use standard tessellation
    /// and produce a single Solid pipeline batch.
    #[test]
    fn test_no_quadto_path_produces_one_batch() {
        let mut batcher = DrawBatcher::new();

        // Standard rectangle (no curves)
        let rect = create_rect_path(1, [255, 0, 0, 255], 0.0, 0.0, 100.0, 100.0);
        let nodes = vec![CanvasNode::Path(rect)];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Should produce exactly 1 batch (standard tessellation only)
        assert_eq!(
            batches.len(),
            1,
            "Expected 1 batch for LineTo-only path, got {}",
            batches.len()
        );

        let batch = &batches[0];
        assert_eq!(batch.pipeline_key, PipelineKey::Solid);
        assert!(
            batch.vertices.as_standard().is_some(),
            "Expected standard vertices for LineTo-only path"
        );
    }

    /// Test: Multiple consecutive same-color QuadTo paths preserve z-order batches.
    ///
    /// When multiple paths with QuadTo have the same fill color, each path produces
    /// a Loop-Blinn batch followed by an interior batch. Due to z-order preservation,
    /// alternating batch types (LB, Interior, LB, Interior, ...) cannot be merged.
    ///
    /// For 3 paths: [LB1, Interior1, LB2, Interior2, LB3, Interior3] = 6 batches
    /// (3 Loop-Blinn + 3 Standard)
    #[test]
    fn test_same_color_quadto_paths_preserve_zorder() {
        let mut batcher = DrawBatcher::new();

        // Create 3 consecutive paths with QuadTo, all same color
        let nodes = vec![
            CanvasNode::Path(create_quad_curve_path(1, [255, 0, 0, 255])),
            CanvasNode::Path(create_quad_curve_path(2, [255, 0, 0, 255])),
            CanvasNode::Path(create_quad_curve_path(3, [255, 0, 0, 255])),
        ];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Count batches by pipeline type
        let loop_blinn_count = batches
            .iter()
            .filter(|b| b.pipeline_key == PipelineKey::LoopBlinn)
            .count();
        let solid_count = batches
            .iter()
            .filter(|b| b.pipeline_key == PipelineKey::Solid)
            .count();

        // Z-order preservation means alternating batch types don't merge:
        // Each path produces: [LoopBlinn, Interior]
        // 3 paths produce: [LB, S, LB, S, LB, S] = 6 batches
        assert_eq!(
            loop_blinn_count, 3,
            "Expected 3 Loop-Blinn batches (one per path), got {}",
            loop_blinn_count
        );
        assert_eq!(
            solid_count, 3,
            "Expected 3 Solid batches (one per path), got {}",
            solid_count
        );
        assert_eq!(
            batches.len(),
            6,
            "Expected 6 total batches (2 per path), got {}",
            batches.len()
        );

        // Verify each Loop-Blinn batch has 3 vertices (1 curve)
        for batch in batches
            .iter()
            .filter(|b| b.pipeline_key == PipelineKey::LoopBlinn)
        {
            let lb_vertices = batch.vertices.as_loop_blinn().unwrap();
            assert_eq!(
                lb_vertices.len(),
                3,
                "Expected 3 Loop-Blinn vertices per curve, got {}",
                lb_vertices.len()
            );
        }
    }

    /// Test: Different color QuadTo paths produce separate batches.
    ///
    /// When paths have different fill colors, they cannot be merged,
    /// even if they both contain QuadTo commands.
    #[test]
    fn test_different_color_quadto_paths_separate_batches() {
        let mut batcher = DrawBatcher::new();

        // Two paths with different colors
        let nodes = vec![
            CanvasNode::Path(create_quad_curve_path(1, [255, 0, 0, 255])), // Red
            CanvasNode::Path(create_quad_curve_path(2, [0, 255, 0, 255])), // Green
        ];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Each path produces 2 batches (Loop-Blinn + Standard)
        // Different colors = no merging
        // Total: 4 batches
        assert_eq!(
            batches.len(),
            4,
            "Expected 4 batches for 2 different-color QuadTo paths, got {}",
            batches.len()
        );

        let loop_blinn_count = batches
            .iter()
            .filter(|b| b.pipeline_key == PipelineKey::LoopBlinn)
            .count();
        assert_eq!(
            loop_blinn_count, 2,
            "Expected 2 Loop-Blinn batches (one per color)"
        );
    }

    // =========================================================================
    // SDF Stroke Tests (Phase I-3)
    // =========================================================================

    /// Helper to create a path with QuadTo and stroke (no fill).
    fn create_quad_curve_stroke_only(
        entity_id: u64,
        stroke_rgba: [u8; 4],
        stroke_width: f32,
    ) -> CanvasPathNode {
        CanvasPathNode {
            base: CanvasNodeBase {
                entity_id: EntityId(entity_id),
                bounds: PVectorBounds {
                    top_left: PVector::zero(),
                    bottom_right: PVector {
                        x: Rational::from_int(100),
                        y: Rational::from_int(100),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: String::new(),
            },
            path_data: vec![
                PathCommand::MoveTo {
                    x: Rational::from_int(0),
                    y: Rational::from_int(0),
                },
                PathCommand::QuadTo {
                    x1: Rational::from_int(50),
                    y1: Rational::from_int(100),
                    x: Rational::from_int(100),
                    y: Rational::from_int(0),
                },
            ],
            fill: None,
            stroke: Some(crate::StrokeStyle {
                rgba: stroke_rgba,
                width: Rational::from_int(stroke_width as i64),
                line_cap: crate::LineCap::Butt,
                line_join: crate::LineJoin::Miter,
                dash_array: None,
            }),
        }
    }

    /// Helper to create a path with QuadTo, fill, and stroke.
    fn create_quad_curve_fill_and_stroke(
        entity_id: u64,
        fill_rgba: [u8; 4],
        stroke_rgba: [u8; 4],
        stroke_width: f32,
    ) -> CanvasPathNode {
        CanvasPathNode {
            base: CanvasNodeBase {
                entity_id: EntityId(entity_id),
                bounds: PVectorBounds {
                    top_left: PVector::zero(),
                    bottom_right: PVector {
                        x: Rational::from_int(100),
                        y: Rational::from_int(100),
                        z: Rational::zero(),
                        t: Rational::zero(),
                    },
                },
                z_order: 0,
                chunk_id: String::new(),
            },
            path_data: vec![
                PathCommand::MoveTo {
                    x: Rational::from_int(0),
                    y: Rational::from_int(0),
                },
                PathCommand::QuadTo {
                    x1: Rational::from_int(50),
                    y1: Rational::from_int(100),
                    x: Rational::from_int(100),
                    y: Rational::from_int(0),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(100),
                    y: Rational::from_int(50),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(0),
                    y: Rational::from_int(50),
                },
                PathCommand::Close,
            ],
            fill: Some(FillStyle::Solid { rgba: fill_rgba }),
            stroke: Some(crate::StrokeStyle {
                rgba: stroke_rgba,
                width: Rational::from_int(stroke_width as i64),
                line_cap: crate::LineCap::Butt,
                line_join: crate::LineJoin::Miter,
                dash_array: None,
            }),
        }
    }

    /// Test: Path with QuadTo + stroke (no fill) produces SdfStroke batch.
    #[test]
    fn test_quadto_stroke_produces_sdf_stroke_batch() {
        let mut batcher = DrawBatcher::new();

        let path = create_quad_curve_stroke_only(1, [0, 255, 0, 255], 4.0);
        let nodes = vec![CanvasNode::Path(path)];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Should produce 1 batch: SdfStroke
        assert_eq!(
            batches.len(),
            1,
            "Expected 1 batch for QuadTo stroke-only path, got {}",
            batches.len()
        );

        let batch = &batches[0];
        assert_eq!(
            batch.pipeline_key,
            PipelineKey::SdfStroke,
            "Expected SdfStroke pipeline for QuadTo stroke"
        );

        // Verify vertices are SdfStroke type
        assert!(
            batch.vertices.as_sdf_stroke().is_some(),
            "Expected SdfStroke vertices"
        );

        let sdf_vertices = batch.vertices.as_sdf_stroke().unwrap();
        // One QuadTo = 4 vertices (bounding rectangle)
        assert_eq!(
            sdf_vertices.len(),
            4,
            "Expected 4 SdfStroke vertices for one curve, got {}",
            sdf_vertices.len()
        );

        // Verify indices (2 triangles = 6 indices)
        assert_eq!(
            batch.indices.len(),
            6,
            "Expected 6 indices for one curve rectangle"
        );
    }

    /// Test: Path with QuadTo + fill + stroke produces 3 batches.
    ///
    /// Expected batches:
    /// 1. LoopBlinn (curve fill triangles)
    /// 2. Solid (interior fill)
    /// 3. SdfStroke (curve stroke)
    #[test]
    fn test_quadto_fill_and_stroke_produces_three_batches() {
        let mut batcher = DrawBatcher::new();

        let path = create_quad_curve_fill_and_stroke(1, [255, 0, 0, 255], [0, 0, 255, 255], 4.0);
        let nodes = vec![CanvasNode::Path(path)];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Should produce 3 batches: LoopBlinn + Solid + SdfStroke
        assert_eq!(
            batches.len(),
            3,
            "Expected 3 batches for QuadTo fill+stroke path, got {}",
            batches.len()
        );

        // Find each batch type
        let loop_blinn_batch = batches
            .iter()
            .find(|b| b.pipeline_key == PipelineKey::LoopBlinn);
        let solid_batch = batches
            .iter()
            .find(|b| b.pipeline_key == PipelineKey::Solid);
        let sdf_stroke_batch = batches
            .iter()
            .find(|b| b.pipeline_key == PipelineKey::SdfStroke);

        assert!(
            loop_blinn_batch.is_some(),
            "Expected a LoopBlinn batch for curve fill"
        );
        assert!(
            solid_batch.is_some(),
            "Expected a Solid batch for interior fill"
        );
        assert!(
            sdf_stroke_batch.is_some(),
            "Expected a SdfStroke batch for curve stroke"
        );

        // Verify vertex types
        assert!(
            loop_blinn_batch.unwrap().vertices.as_loop_blinn().is_some(),
            "LoopBlinn batch should have LoopBlinn vertices"
        );
        assert!(
            solid_batch.unwrap().vertices.as_standard().is_some(),
            "Solid batch should have Standard vertices"
        );
        assert!(
            sdf_stroke_batch.unwrap().vertices.as_sdf_stroke().is_some(),
            "SdfStroke batch should have SdfStroke vertices"
        );
    }

    /// Test: Stroke color is correctly set in SolidColorUniform.
    #[test]
    fn test_sdf_stroke_color_uniform() {
        let mut batcher = DrawBatcher::new();

        // Create path with specific stroke color
        let path = create_quad_curve_stroke_only(1, [0, 255, 0, 255], 4.0); // Green stroke
        let nodes = vec![CanvasNode::Path(path)];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        assert_eq!(batches.len(), 1, "Expected 1 batch");

        let batch = &batches[0];
        assert_eq!(batch.pipeline_key, PipelineKey::SdfStroke);

        // Verify uniform data is Solid color
        match &batch.uniform_data {
            UniformData::Solid(color) => {
                // #00ff00 = green (r=0, g=1, b=0)
                assert!(color.r.abs() < 0.01, "Red should be ~0, got {}", color.r);
                assert!(
                    (color.g - 1.0).abs() < 0.01,
                    "Green should be ~1, got {}",
                    color.g
                );
                assert!(color.b.abs() < 0.01, "Blue should be ~0, got {}", color.b);
                assert!(
                    (color.a - 1.0).abs() < 0.01,
                    "Alpha should be ~1, got {}",
                    color.a
                );
            }
            _ => panic!("Expected Solid uniform data for SdfStroke"),
        }
    }

    /// Test: Path without QuadTo but with stroke uses lyon tessellation (not SdfStroke).
    #[test]
    fn test_no_quadto_stroke_uses_lyon() {
        let mut batcher = DrawBatcher::new();

        // Create rectangle with stroke (no QuadTo)
        let mut rect = create_rect_path(1, [255, 0, 0, 255], 0.0, 0.0, 100.0, 100.0);
        rect.stroke = Some(crate::StrokeStyle {
            rgba: [0, 0, 255, 255],
            width: Rational::from_int(4),
            line_cap: crate::LineCap::Butt,
            line_join: crate::LineJoin::Miter,
            dash_array: None,
        });

        let nodes = vec![CanvasNode::Path(rect)];

        batcher.collect_nodes(&nodes, 800.0, 600.0);
        let batches = batcher.drain();

        // Should produce 2 batches: fill (Solid) + stroke (Solid via lyon)
        assert_eq!(batches.len(), 2, "Expected 2 batches for rect fill+stroke");

        // Neither should be SdfStroke (no QuadTo)
        let sdf_count = batches
            .iter()
            .filter(|b| b.pipeline_key == PipelineKey::SdfStroke)
            .count();
        assert_eq!(sdf_count, 0, "No SdfStroke batches for path without QuadTo");

        // Both should be Solid (fill and stroke use lyon tessellation)
        let solid_count = batches
            .iter()
            .filter(|b| b.pipeline_key == PipelineKey::Solid)
            .count();
        assert_eq!(
            solid_count, 2,
            "Expected 2 Solid batches (fill + stroke via lyon)"
        );
    }
}
