// =============================================================================
// Frame Rendering for @viewscript/gpu-runtime
// =============================================================================
//
// Executes the render loop, encoding draw commands for all registered meshes.

import type { Pipelines, PipelineKey } from './pipelines';
import type { MeshRegistry, TextureRegistry } from './mesh';

// =============================================================================
// Types
// =============================================================================

/** Transform uniform data (48 bytes, 16-byte aligned) */
export interface TransformData {
  /** Affine transform: a, b, c, d, tx, ty */
  a: number;
  b: number;
  c: number;
  d: number;
  tx: number;
  ty: number;
  /** Viewport dimensions */
  viewportWidth: number;
  viewportHeight: number;
  /** Accumulated opacity [0, 1] */
  opacity: number;
}

/** Frame context for rendering */
export interface FrameContext {
  device: GPUDevice;
  pipelines: Pipelines;
  meshRegistry: MeshRegistry;
  textureRegistry: TextureRegistry;
  transformBuffer: GPUBuffer;
  transformBindGroups: Map<PipelineKey, GPUBindGroup>;
}

// =============================================================================
// Transform Uniform
// =============================================================================

/** Size of transform uniform in bytes (12 floats, 48 bytes) */
const TRANSFORM_UNIFORM_SIZE = 48;

/**
 * Create transform uniform buffer
 */
export function createTransformBuffer(device: GPUDevice): GPUBuffer {
  return device.createBuffer({
    label: 'Transform Uniform Buffer',
    size: TRANSFORM_UNIFORM_SIZE,
    usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
  });
}

/**
 * Create transform bind groups for all pipelines
 */
export function createTransformBindGroups(
  device: GPUDevice,
  pipelines: Pipelines,
  transformBuffer: GPUBuffer
): Map<PipelineKey, GPUBindGroup> {
  const groups = new Map<PipelineKey, GPUBindGroup>();

  const keys: PipelineKey[] = ['solid', 'loopBlinn', 'loopBlinnCubic', 'texture'];
  for (const key of keys) {
    const pipelineSet = pipelines[key];
    groups.set(key, device.createBindGroup({
      label: `Transform Bind Group (${key})`,
      layout: pipelineSet.transformBindGroupLayout,
      entries: [{
        binding: 0,
        resource: { buffer: transformBuffer },
      }],
    }));
  }

  return groups;
}

/**
 * Write transform data to uniform buffer
 */
export function updateTransform(
  device: GPUDevice,
  buffer: GPUBuffer,
  transform: TransformData
): void {
  const data = new Float32Array([
    transform.a,
    transform.b,
    transform.c,
    transform.d,
    transform.tx,
    transform.ty,
    transform.viewportWidth,
    transform.viewportHeight,
    transform.opacity,
    0, 0, 0, // padding for 16-byte alignment
  ]);
  device.queue.writeBuffer(buffer, 0, data);
}

// =============================================================================
// Render Pass
// =============================================================================

/**
 * Render a single frame
 *
 * Draws meshes in registration order (Z-order).
 * Compiler emits registerMesh() calls in Z-order, so registration order = draw order.
 *
 * Note: Currently renders solid meshes first, then textured meshes.
 * For interleaved Z-order, a unified draw list would be needed.
 */
export function renderFrame(
  ctx: FrameContext,
  textureView: GPUTextureView,
  clearColor: GPUColor = { r: 0, g: 0, b: 0, a: 1 }
): void {
  const { device, pipelines, meshRegistry, textureRegistry, transformBindGroups } = ctx;

  const commandEncoder = device.createCommandEncoder({
    label: 'Frame Command Encoder',
  });

  const renderPass = commandEncoder.beginRenderPass({
    label: 'Main Render Pass',
    colorAttachments: [{
      view: textureView,
      clearValue: clearColor,
      loadOp: 'clear',
      storeOp: 'store',
    }],
  });

  // Render solid meshes in registration order (Z-order from compiler)
  let currentPipeline: PipelineKey | null = null;

  for (const mesh of meshRegistry.getAllInOrder()) {
    // Switch pipeline only when needed
    if (mesh.pipelineKey !== currentPipeline) {
      currentPipeline = mesh.pipelineKey;
      const pipelineSet = pipelines[currentPipeline];
      const transformBindGroup = transformBindGroups.get(currentPipeline)!;

      renderPass.setPipeline(pipelineSet.pipeline);
      renderPass.setBindGroup(0, transformBindGroup);
    }

    renderPass.setBindGroup(1, mesh.colorBindGroup);
    renderPass.setVertexBuffer(0, mesh.vertexBuffer);
    renderPass.setIndexBuffer(mesh.indexBuffer, mesh.indexFormat);
    renderPass.drawIndexed(mesh.indexCount);
  }

  // Render textured meshes
  const texturedMeshes = textureRegistry.getAllTexturedMeshes();
  if (texturedMeshes.length > 0) {
    // Switch to texture pipeline
    const texturePipelineSet = pipelines.texture;
    const textureTransformBindGroup = transformBindGroups.get('texture')!;

    renderPass.setPipeline(texturePipelineSet.pipeline);
    renderPass.setBindGroup(0, textureTransformBindGroup);

    for (const mesh of texturedMeshes) {
      renderPass.setBindGroup(1, mesh.textureBindGroup);
      renderPass.setVertexBuffer(0, mesh.vertexBuffer);
      renderPass.setIndexBuffer(mesh.indexBuffer, mesh.indexFormat);
      renderPass.drawIndexed(mesh.indexCount);
    }
  }

  renderPass.end();

  device.queue.submit([commandEncoder.finish()]);
}

