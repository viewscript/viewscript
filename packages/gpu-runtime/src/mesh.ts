// =============================================================================
// Mesh Management for @viewscript/gpu-runtime
// =============================================================================
//
// Manages GPU buffers for tessellated meshes generated at build time.
// Supports position updates for Q-dimension (reactive) variables.

import type { PipelineKey } from './pipelines';

// =============================================================================
// Types
// =============================================================================

/** Mesh identifier (matches EntityId from build output) */
export type MeshId = string;

/** Pre-tessellated mesh data from build output */
export interface MeshData {
  /** Pipeline type for this mesh */
  pipelineKey: PipelineKey;
  /** Vertex data (Float32Array) - interleaved [x, y, u, v, ...] per vertex */
  vertices: Float32Array;
  /** Index data (Uint16Array or Uint32Array) */
  indices: Uint16Array | Uint32Array;
  /** Fill color RGBA [0-1] */
  color: [number, number, number, number];
  /** Number of vertices (for updatePositions stride calculation) */
  vertexCount: number;
  /** Floats per vertex (stride), default 4 for [x, y, u, v] */
  vertexStride?: number;
}

/** GPU-uploaded mesh with buffers */
export interface GpuMesh {
  id: MeshId;
  pipelineKey: PipelineKey;
  vertexBuffer: GPUBuffer;
  indexBuffer: GPUBuffer;
  indexCount: number;
  indexFormat: GPUIndexFormat;
  colorUniform: GPUBuffer;
  colorBindGroup: GPUBindGroup;
  /** Number of vertices */
  vertexCount: number;
  /** Floats per vertex (stride) */
  vertexStride: number;
}

/** Mesh registry for the runtime */
export class MeshRegistry {
  private meshes: Map<MeshId, GpuMesh> = new Map();
  private device: GPUDevice;
  private styleBindGroupLayout: GPUBindGroupLayout;

  constructor(device: GPUDevice, styleBindGroupLayout: GPUBindGroupLayout) {
    this.device = device;
    this.styleBindGroupLayout = styleBindGroupLayout;
  }

  /**
   * Register a pre-tessellated mesh
   */
  registerMesh(id: MeshId, data: MeshData): GpuMesh {
    // Create vertex buffer
    const vertexBuffer = this.device.createBuffer({
      label: `Vertex Buffer: ${id}`,
      size: data.vertices.byteLength,
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
      mappedAtCreation: true,
    });
    new Float32Array(vertexBuffer.getMappedRange()).set(data.vertices);
    vertexBuffer.unmap();

    // Create index buffer
    const indexFormat: GPUIndexFormat = data.indices instanceof Uint32Array ? 'uint32' : 'uint16';
    const indexBuffer = this.device.createBuffer({
      label: `Index Buffer: ${id}`,
      size: data.indices.byteLength,
      usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST,
      mappedAtCreation: true,
    });
    if (data.indices instanceof Uint32Array) {
      new Uint32Array(indexBuffer.getMappedRange()).set(data.indices);
    } else {
      new Uint16Array(indexBuffer.getMappedRange()).set(data.indices);
    }
    indexBuffer.unmap();

    // Create color uniform buffer (16 bytes for RGBA f32)
    const colorUniform = this.device.createBuffer({
      label: `Color Uniform: ${id}`,
      size: 16,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.device.queue.writeBuffer(colorUniform, 0, new Float32Array(data.color));

    // Create bind group for color
    const colorBindGroup = this.device.createBindGroup({
      label: `Color Bind Group: ${id}`,
      layout: this.styleBindGroupLayout,
      entries: [{
        binding: 0,
        resource: { buffer: colorUniform },
      }],
    });

    const mesh: GpuMesh = {
      id,
      pipelineKey: data.pipelineKey,
      vertexBuffer,
      indexBuffer,
      indexCount: data.indices.length,
      indexFormat,
      colorUniform,
      colorBindGroup,
      vertexCount: data.vertexCount,
      vertexStride: data.vertexStride ?? 4, // Default: [x, y, u, v]
    };

    this.meshes.set(id, mesh);
    return mesh;
  }

  /**
   * Update vertex positions for a mesh (Q-dimension reactive update)
   *
   * Writes position data with proper stride to preserve interleaved attributes.
   * For vertex layout [x, y, u, v], only x and y are updated per vertex.
   *
   * @param id - Mesh identifier
   * @param positions - New position data (flat array of [x0, y0, x1, y1, ...])
   */
  updatePositions(id: MeshId, positions: Float32Array): void {
    const mesh = this.meshes.get(id);
    if (!mesh) {
      console.warn(`MeshRegistry: mesh ${id} not found`);
      return;
    }

    const expectedPositionFloats = mesh.vertexCount * 2; // 2 floats (x, y) per vertex
    if (positions.length !== expectedPositionFloats) {
      console.warn(
        `MeshRegistry: position count mismatch for ${id}. ` +
        `Expected ${expectedPositionFloats}, got ${positions.length}`
      );
      return;
    }

    // Write positions with stride to preserve UV data
    // Each vertex has `vertexStride` floats, we update the first 2 (position)
    const strideBytes = mesh.vertexStride * 4; // 4 bytes per float
    const positionBytes = 2 * 4; // 2 floats * 4 bytes

    for (let i = 0; i < mesh.vertexCount; i++) {
      this.device.queue.writeBuffer(
        mesh.vertexBuffer,
        i * strideBytes, // Offset to this vertex
        positions.buffer,
        positions.byteOffset + i * 2 * 4, // Source offset: i * 2 floats * 4 bytes
        positionBytes
      );
    }
  }

  /**
   * Update fill color for a mesh
   */
  updateColor(id: MeshId, color: [number, number, number, number]): void {
    const mesh = this.meshes.get(id);
    if (!mesh) {
      console.warn(`MeshRegistry: mesh ${id} not found`);
      return;
    }

    this.device.queue.writeBuffer(mesh.colorUniform, 0, new Float32Array(color));
  }

  /**
   * Get a mesh by ID
   */
  getMesh(id: MeshId): GpuMesh | undefined {
    return this.meshes.get(id);
  }

  /**
   * Get all meshes in registration order (Z-order for rendering)
   *
   * Map preserves insertion order per ES2015 spec.
   * Compiler emits registerMesh calls in Z-order, so iteration order = draw order.
   */
  getAllInOrder(): GpuMesh[] {
    return Array.from(this.meshes.values());
  }

  /**
   * Remove a mesh and release its GPU resources
   */
  removeMesh(id: MeshId): boolean {
    const mesh = this.meshes.get(id);
    if (!mesh) return false;

    mesh.vertexBuffer.destroy();
    mesh.indexBuffer.destroy();
    mesh.colorUniform.destroy();
    this.meshes.delete(id);
    return true;
  }

  /**
   * Release all GPU resources
   */
  destroy(): void {
    for (const mesh of this.meshes.values()) {
      mesh.vertexBuffer.destroy();
      mesh.indexBuffer.destroy();
      mesh.colorUniform.destroy();
    }
    this.meshes.clear();
  }
}
