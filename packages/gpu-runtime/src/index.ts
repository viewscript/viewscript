// =============================================================================
// @viewscript/gpu-runtime
// =============================================================================
//
// Standalone WebGPU runtime for ViewScript compiled output.
// No WASM dependency - pure JavaScript/TypeScript with WebGPU API.
//
// ## Usage
//
// ```typescript
// import { initGpu, createRuntime } from '@viewscript/gpu-runtime';
//
// const gpu = await initGpu(canvas);
// const runtime = createRuntime(gpu);
//
// // Register meshes from compiled output
// runtime.registerMesh('rect_1', meshData);
//
// // Render loop
// function animate() {
//   runtime.render();
//   requestAnimationFrame(animate);
// }
// animate();
// ```

// Re-export types and utilities
export { SOLID_WGSL, LOOP_BLINN_WGSL, LOOP_BLINN_CUBIC_WGSL } from './shaders';
export { createPipelines, selectPipeline } from './pipelines';
export type { PipelineKey, PipelineSet, Pipelines } from './pipelines';
export { MeshRegistry } from './mesh';
export type { MeshId, MeshData, GpuMesh } from './mesh';
export {
  createTransformBuffer,
  createTransformBindGroups,
  updateTransform,
  renderFrame,
} from './frame';
export type { TransformData, FrameContext } from './frame';

import { createPipelines, type Pipelines } from './pipelines';
import { MeshRegistry, type MeshId, type MeshData, type GpuMesh } from './mesh';
import {
  createTransformBuffer,
  createTransformBindGroups,
  updateTransform,
  renderFrame,
  type TransformData,
  type FrameContext,
} from './frame';

// =============================================================================
// GPU Initialization
// =============================================================================

/** GPU context after successful initialization */
export interface GpuContext {
  adapter: GPUAdapter;
  device: GPUDevice;
  context: GPUCanvasContext;
  format: GPUTextureFormat;
  canvas: HTMLCanvasElement;
}

/** GPU initialization error */
export class GpuInitError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'GpuInitError';
  }
}

/**
 * Initialize WebGPU context for a canvas
 *
 * @param canvas - Target canvas element
 * @returns GPU context
 * @throws GpuInitError if WebGPU is not available
 */
export async function initGpu(canvas: HTMLCanvasElement): Promise<GpuContext> {
  // Check WebGPU availability
  if (!navigator.gpu) {
    throw new GpuInitError(
      'WebGPU is not supported in this browser. ' +
      'Please use a WebGPU-enabled browser (Chrome 113+, Edge 113+, or Firefox Nightly with flags).'
    );
  }

  // Request adapter
  const adapter = await navigator.gpu.requestAdapter({
    powerPreference: 'high-performance',
  });
  if (!adapter) {
    throw new GpuInitError(
      'Failed to get WebGPU adapter. ' +
      'Your GPU may not support WebGPU, or it may be disabled.'
    );
  }

  // Request device
  const device = await adapter.requestDevice();
  if (!device) {
    throw new GpuInitError('Failed to get WebGPU device.');
  }

  // Configure canvas context
  const context = canvas.getContext('webgpu');
  if (!context) {
    throw new GpuInitError('Failed to get WebGPU canvas context.');
  }

  const format = navigator.gpu.getPreferredCanvasFormat();
  context.configure({
    device,
    format,
    alphaMode: 'premultiplied',
  });

  return { adapter, device, context, format, canvas };
}

// =============================================================================
// Runtime
// =============================================================================

/** ViewScript GPU Runtime */
export interface VsRuntime {
  /** GPU context */
  gpu: GpuContext;
  /** Render pipelines */
  pipelines: Pipelines;
  /** Mesh registry */
  meshes: MeshRegistry;
  /** Register a mesh from compiled output */
  registerMesh(id: MeshId, data: MeshData): GpuMesh;
  /** Update mesh positions (Q-dimension reactive) */
  updatePositions(id: MeshId, positions: Float32Array): void;
  /** Update mesh color */
  updateColor(id: MeshId, color: [number, number, number, number]): void;
  /** Update transform (viewport, opacity) */
  setTransform(transform: Partial<TransformData>): void;
  /** Render a frame */
  render(clearColor?: { r: number; g: number; b: number; a: number }): void;
  /** Release all resources */
  destroy(): void;
}

/**
 * Create a ViewScript runtime instance
 */
export function createRuntime(gpu: GpuContext): VsRuntime {
  const { device, context, format, canvas } = gpu;

  // Create pipelines
  const pipelines = createPipelines(device, format);

  // Create mesh registry (use solid pipeline's style layout as default)
  const meshes = new MeshRegistry(device, pipelines.solid.styleBindGroupLayout);

  // Create transform resources
  const transformBuffer = createTransformBuffer(device);
  const transformBindGroups = createTransformBindGroups(device, pipelines, transformBuffer);

  // Current transform state (viewport in CSS pixels, not device pixels)
  let currentTransform: TransformData = {
    a: 1, b: 0, c: 0, d: 1, tx: 0, ty: 0,
    viewportWidth: canvas.clientWidth,
    viewportHeight: canvas.clientHeight,
    opacity: 1,
  };

  // Update initial transform
  updateTransform(device, transformBuffer, currentTransform);

  // Frame context
  const frameCtx: FrameContext = {
    device,
    pipelines,
    meshRegistry: meshes,
    transformBuffer,
    transformBindGroups,
  };

  return {
    gpu,
    pipelines,
    meshes,

    registerMesh(id: MeshId, data: MeshData): GpuMesh {
      return meshes.registerMesh(id, data);
    },

    updatePositions(id: MeshId, positions: Float32Array): void {
      meshes.updatePositions(id, positions);
    },

    updateColor(id: MeshId, color: [number, number, number, number]): void {
      meshes.updateColor(id, color);
    },

    setTransform(transform: Partial<TransformData>): void {
      currentTransform = { ...currentTransform, ...transform };
      updateTransform(device, transformBuffer, currentTransform);
    },

    render(clearColor = { r: 0, g: 0, b: 0, a: 1 }): void {
      // Update viewport if canvas size changed (CSS pixels, not device pixels)
      if (canvas.clientWidth !== currentTransform.viewportWidth ||
          canvas.clientHeight !== currentTransform.viewportHeight) {
        currentTransform.viewportWidth = canvas.clientWidth;
        currentTransform.viewportHeight = canvas.clientHeight;
        updateTransform(device, transformBuffer, currentTransform);
      }

      const textureView = context.getCurrentTexture().createView();
      renderFrame(frameCtx, textureView, clearColor);
    },

    destroy(): void {
      meshes.destroy();
      transformBuffer.destroy();
    },
  };
}
