// =============================================================================
// Pipeline Management for @viewscript/gpu-runtime
// =============================================================================
//
// Creates and manages WebGPU render pipelines for ViewScript rendering.
// Mirrors vsc-gpu/src/pipeline.rs bind group layout structure.

import { SOLID_WGSL, LOOP_BLINN_WGSL, LOOP_BLINN_CUBIC_WGSL, TEXTURE_WGSL } from './shaders';

// =============================================================================
// Types
// =============================================================================

/** Pipeline type identifier */
export type PipelineKey = 'solid' | 'loopBlinn' | 'loopBlinnCubic' | 'texture';

/** Complete pipeline set for a specific rendering type */
export interface PipelineSet {
  pipeline: GPURenderPipeline;
  transformBindGroupLayout: GPUBindGroupLayout;
  styleBindGroupLayout: GPUBindGroupLayout;
}

/** All pipelines managed by the runtime */
export interface Pipelines {
  solid: PipelineSet;
  loopBlinn: PipelineSet;
  loopBlinnCubic: PipelineSet;
  texture: PipelineSet;
}

// =============================================================================
// Vertex Formats
// =============================================================================

/**
 * GpuVertex layout for solid pipeline (16 bytes)
 * - position: vec2<f32> (8 bytes)
 * - uv: vec2<f32> (8 bytes)
 */
const GPU_VERTEX_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 16,
  stepMode: 'vertex',
  attributes: [
    { shaderLocation: 0, offset: 0, format: 'float32x2' },  // position
    { shaderLocation: 1, offset: 8, format: 'float32x2' },  // uv
  ],
};

/**
 * LoopBlinnVertex layout for quadratic Bezier (20 bytes)
 * - position: vec2<f32> (8 bytes)
 * - curve_uv: vec2<f32> (8 bytes)
 * - curve_sign: f32 (4 bytes)
 */
const LOOP_BLINN_VERTEX_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 20,
  stepMode: 'vertex',
  attributes: [
    { shaderLocation: 0, offset: 0, format: 'float32x2' },   // position
    { shaderLocation: 1, offset: 8, format: 'float32x2' },   // curve_uv
    { shaderLocation: 2, offset: 16, format: 'float32' },    // curve_sign
  ],
};

/**
 * CubicLoopBlinnVertex layout for cubic Bezier (24 bytes)
 * - position: vec2<f32> (8 bytes)
 * - curve_klm: vec3<f32> (12 bytes)
 * - curve_sign: f32 (4 bytes)
 */
const CUBIC_LOOP_BLINN_VERTEX_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 24,
  stepMode: 'vertex',
  attributes: [
    { shaderLocation: 0, offset: 0, format: 'float32x2' },   // position
    { shaderLocation: 1, offset: 8, format: 'float32x3' },   // curve_klm
    { shaderLocation: 2, offset: 20, format: 'float32' },    // curve_sign
  ],
};

// =============================================================================
// Pipeline Creation
// =============================================================================

/**
 * Create shared transform bind group layout (Group 0)
 * Visible to both vertex (for transform) and fragment (for opacity)
 */
function createTransformBindGroupLayout(device: GPUDevice, label: string): GPUBindGroupLayout {
  return device.createBindGroupLayout({
    label,
    entries: [{
      binding: 0,
      visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
      buffer: { type: 'uniform' },
    }],
  });
}

/**
 * Create solid color bind group layout (Group 1)
 * Used for solid fills and Loop-Blinn curve rendering
 */
function createSolidColorBindGroupLayout(device: GPUDevice, label: string): GPUBindGroupLayout {
  return device.createBindGroupLayout({
    label,
    entries: [{
      binding: 0,
      visibility: GPUShaderStage.FRAGMENT,
      buffer: { type: 'uniform' },
    }],
  });
}

/**
 * Create solid color pipeline
 */
function createSolidPipeline(device: GPUDevice, format: GPUTextureFormat): PipelineSet {
  const transformBindGroupLayout = createTransformBindGroupLayout(device, 'Transform Layout (Solid)');
  const styleBindGroupLayout = createSolidColorBindGroupLayout(device, 'Solid Color Layout');

  const pipelineLayout = device.createPipelineLayout({
    label: 'Solid Pipeline Layout',
    bindGroupLayouts: [transformBindGroupLayout, styleBindGroupLayout],
  });

  const shaderModule = device.createShaderModule({
    label: 'Solid Shader',
    code: SOLID_WGSL,
  });

  const pipeline = device.createRenderPipeline({
    label: 'Solid Render Pipeline',
    layout: pipelineLayout,
    vertex: {
      module: shaderModule,
      entryPoint: 'vs_main',
      buffers: [GPU_VERTEX_LAYOUT],
    },
    fragment: {
      module: shaderModule,
      entryPoint: 'fs_main',
      targets: [{
        format,
        blend: {
          color: {
            srcFactor: 'src-alpha',
            dstFactor: 'one-minus-src-alpha',
            operation: 'add',
          },
          alpha: {
            srcFactor: 'one',
            dstFactor: 'one-minus-src-alpha',
            operation: 'add',
          },
        },
        writeMask: GPUColorWrite.ALL,
      }],
    },
    primitive: {
      topology: 'triangle-list',
      frontFace: 'ccw',
      cullMode: 'none',
    },
  });

  return { pipeline, transformBindGroupLayout, styleBindGroupLayout };
}

/**
 * Create Loop-Blinn quadratic Bezier pipeline
 */
function createLoopBlinnPipeline(device: GPUDevice, format: GPUTextureFormat): PipelineSet {
  const transformBindGroupLayout = createTransformBindGroupLayout(device, 'Transform Layout (LoopBlinn)');
  const styleBindGroupLayout = createSolidColorBindGroupLayout(device, 'LoopBlinn Color Layout');

  const pipelineLayout = device.createPipelineLayout({
    label: 'LoopBlinn Pipeline Layout',
    bindGroupLayouts: [transformBindGroupLayout, styleBindGroupLayout],
  });

  const shaderModule = device.createShaderModule({
    label: 'LoopBlinn Shader',
    code: LOOP_BLINN_WGSL,
  });

  const pipeline = device.createRenderPipeline({
    label: 'LoopBlinn Render Pipeline',
    layout: pipelineLayout,
    vertex: {
      module: shaderModule,
      entryPoint: 'vs_main',
      buffers: [LOOP_BLINN_VERTEX_LAYOUT],
    },
    fragment: {
      module: shaderModule,
      entryPoint: 'fs_main',
      targets: [{
        format,
        blend: {
          color: {
            srcFactor: 'src-alpha',
            dstFactor: 'one-minus-src-alpha',
            operation: 'add',
          },
          alpha: {
            srcFactor: 'one',
            dstFactor: 'one-minus-src-alpha',
            operation: 'add',
          },
        },
        writeMask: GPUColorWrite.ALL,
      }],
    },
    primitive: {
      topology: 'triangle-list',
      frontFace: 'ccw',
      cullMode: 'none',
    },
  });

  return { pipeline, transformBindGroupLayout, styleBindGroupLayout };
}

/**
 * Create Loop-Blinn cubic Bezier pipeline
 */
function createLoopBlinnCubicPipeline(device: GPUDevice, format: GPUTextureFormat): PipelineSet {
  const transformBindGroupLayout = createTransformBindGroupLayout(device, 'Transform Layout (LoopBlinnCubic)');
  const styleBindGroupLayout = createSolidColorBindGroupLayout(device, 'LoopBlinnCubic Color Layout');

  const pipelineLayout = device.createPipelineLayout({
    label: 'LoopBlinnCubic Pipeline Layout',
    bindGroupLayouts: [transformBindGroupLayout, styleBindGroupLayout],
  });

  const shaderModule = device.createShaderModule({
    label: 'LoopBlinnCubic Shader',
    code: LOOP_BLINN_CUBIC_WGSL,
  });

  const pipeline = device.createRenderPipeline({
    label: 'LoopBlinnCubic Render Pipeline',
    layout: pipelineLayout,
    vertex: {
      module: shaderModule,
      entryPoint: 'vs_main',
      buffers: [CUBIC_LOOP_BLINN_VERTEX_LAYOUT],
    },
    fragment: {
      module: shaderModule,
      entryPoint: 'fs_main',
      targets: [{
        format,
        blend: {
          color: {
            srcFactor: 'src-alpha',
            dstFactor: 'one-minus-src-alpha',
            operation: 'add',
          },
          alpha: {
            srcFactor: 'one',
            dstFactor: 'one-minus-src-alpha',
            operation: 'add',
          },
        },
        writeMask: GPUColorWrite.ALL,
      }],
    },
    primitive: {
      topology: 'triangle-list',
      frontFace: 'ccw',
      cullMode: 'none',
    },
  });

  return { pipeline, transformBindGroupLayout, styleBindGroupLayout };
}

/**
 * Create texture bind group layout (Group 1)
 * Used for texture sampling (images, videos, canvas)
 */
function createTextureBindGroupLayout(device: GPUDevice, label: string): GPUBindGroupLayout {
  return device.createBindGroupLayout({
    label,
    entries: [
      {
        binding: 0,
        visibility: GPUShaderStage.FRAGMENT,
        texture: { sampleType: 'float' },
      },
      {
        binding: 1,
        visibility: GPUShaderStage.FRAGMENT,
        sampler: { type: 'filtering' },
      },
    ],
  });
}

/**
 * Create texture sampling pipeline
 */
function createTexturePipeline(device: GPUDevice, format: GPUTextureFormat): PipelineSet {
  const transformBindGroupLayout = createTransformBindGroupLayout(device, 'Transform Layout (Texture)');
  const styleBindGroupLayout = createTextureBindGroupLayout(device, 'Texture Layout');

  const pipelineLayout = device.createPipelineLayout({
    label: 'Texture Pipeline Layout',
    bindGroupLayouts: [transformBindGroupLayout, styleBindGroupLayout],
  });

  const shaderModule = device.createShaderModule({
    label: 'Texture Shader',
    code: TEXTURE_WGSL,
  });

  const pipeline = device.createRenderPipeline({
    label: 'Texture Render Pipeline',
    layout: pipelineLayout,
    vertex: {
      module: shaderModule,
      entryPoint: 'vs_main',
      buffers: [GPU_VERTEX_LAYOUT],
    },
    fragment: {
      module: shaderModule,
      entryPoint: 'fs_main',
      targets: [{
        format,
        blend: {
          color: {
            srcFactor: 'src-alpha',
            dstFactor: 'one-minus-src-alpha',
            operation: 'add',
          },
          alpha: {
            srcFactor: 'one',
            dstFactor: 'one-minus-src-alpha',
            operation: 'add',
          },
        },
        writeMask: GPUColorWrite.ALL,
      }],
    },
    primitive: {
      topology: 'triangle-list',
      frontFace: 'ccw',
      cullMode: 'none',
    },
  });

  return { pipeline, transformBindGroupLayout, styleBindGroupLayout };
}

/**
 * Create all pipelines for the runtime
 */
export function createPipelines(device: GPUDevice, format: GPUTextureFormat): Pipelines {
  return {
    solid: createSolidPipeline(device, format),
    loopBlinn: createLoopBlinnPipeline(device, format),
    loopBlinnCubic: createLoopBlinnCubicPipeline(device, format),
    texture: createTexturePipeline(device, format),
  };
}

/**
 * Select pipeline by key
 */
export function selectPipeline(pipelines: Pipelines, key: PipelineKey): PipelineSet {
  return pipelines[key];
}
