/**
 * wgpu Renderer Adapter
 *
 * This module provides a `CanvasRenderer` implementation that delegates to
 * `WasmGpuRenderer` from the vsc-wasm crate.
 *
 * ## Phase F: wgpu Migration
 *
 * This adapter implements the wgpu-based renderer:
 *
 * ```text
 * Before (legacy):
 *   RenderLoop → GpuRenderer → PathEntity → WebGL
 *
 * After (wgpu):
 *   RenderLoop → WgpuRendererAdapter → WasmGpuRenderer → wgpu → WebGPU/WebGL
 * ```
 *
 * ## Key Differences from legacy renderer
 *
 * 1. **No Resource Management**: wgpu resources are managed on the Rust side.
 *    TypeScript does NOT need to call delete() on GPU objects.
 *
 * 2. **JSON Serialization**: DrawCommands are converted to CanvasNode JSON
 *    and passed to WASM. This adds serialization overhead but simplifies
 *    the boundary significantly.
 *
 * 3. **Batched Rendering**: draw() accumulates commands; flush() sends them
 *    all to the GPU in a single render pass.
 */

import type { EntityId, RasterBounds, CanvasNode, RenderableEntity, Rational } from '../ast/types.js';

// =============================================================================
// JSON Serialization Helpers
// =============================================================================

/**
 * Custom JSON replacer for WASM boundary serialization.
 *
 * ## Problem
 *
 * JavaScript's `JSON.stringify()` does not support `bigint`:
 * ```
 * JSON.stringify({ n: 100n }) // TypeError: BigInt value can't be serialized
 * ```
 *
 * ## Solution
 *
 * This replacer converts:
 * - `bigint` → string (e.g., `100n` → `"100"`)
 * - `Rational` object → `"numerator/denominator"` string
 *
 * The Rust side expects Rational as `"num/den"` string format
 * (see `vsc-core/src/types.rs` Deserialize impl).
 */
function wasmBoundaryReplacer(_key: string, value: unknown): unknown {
  // Handle raw bigint (shouldn't occur in well-typed code, but safety first)
  if (typeof value === 'bigint') {
    return value.toString();
  }

  // Handle Rational objects: { numerator: bigint, denominator: bigint }
  if (isRational(value)) {
    return `${value.numerator}/${value.denominator}`;
  }

  return value;
}

/**
 * Type guard for Rational objects.
 */
function isRational(value: unknown): value is Rational {
  return (
    value !== null &&
    typeof value === 'object' &&
    'numerator' in value &&
    'denominator' in value &&
    typeof (value as Rational).numerator === 'bigint' &&
    typeof (value as Rational).denominator === 'bigint'
  );
}

// =============================================================================
// Types
// =============================================================================

/**
 * Canvas draw command (batched for wgpu).
 */
interface DrawCommand {
  entityId: EntityId;
  type: 'path' | 'text' | 'image' | 'group';
  bounds: RasterBounds;
  payload: unknown;
}

/**
 * CanvasRenderer interface from render-loop.ts
 */
interface CanvasRenderer {
  getEntity(id: EntityId): RenderableEntity | undefined;
  draw(command: DrawCommand): void;
  flush(): void;
}

/**
 * WasmGpuRenderer interface (from vsc-wasm with "gpu" feature).
 *
 * This is the TypeScript-side view of the Rust struct.
 */
interface WasmGpuRenderer {
  render(nodes_json: string): void;
  resize(width: number, height: number): void;
  readonly width: number;
  readonly height: number;
}

/**
 * Factory function for WasmGpuRenderer.
 */
interface WasmGpuRendererStatic {
  create(canvas: HTMLCanvasElement): Promise<WasmGpuRenderer>;
}

// =============================================================================
// Adapter Implementation
// =============================================================================

/**
 * wgpu-based CanvasRenderer implementation.
 *
 * This adapter accumulates DrawCommands during a frame, converts them to
 * CanvasNode JSON, and sends them to WasmGpuRenderer on flush().
 *
 * ## Usage
 *
 * ```typescript
 * import { createWgpuRendererAdapter } from './wgpu-renderer-adapter';
 *
 * const canvas = document.getElementById('viewport') as HTMLCanvasElement;
 * const adapter = await createWgpuRendererAdapter(canvas, entityStore);
 *
 * // In render loop:
 * adapter.draw(command1);
 * adapter.draw(command2);
 * adapter.flush(); // Sends all commands to GPU
 * ```
 */
export class WgpuRendererAdapter implements CanvasRenderer {
  /** WASM GPU renderer instance */
  private renderer: WasmGpuRenderer;

  /** Entity store for getEntity() lookups */
  private entityStore: Map<EntityId, RenderableEntity>;

  /** Accumulated draw commands for current frame */
  private pendingCommands: DrawCommand[] = [];

  /** Canvas element for resize handling */
  private canvas: HTMLCanvasElement;

  private constructor(
    renderer: WasmGpuRenderer,
    canvas: HTMLCanvasElement,
    entityStore: Map<EntityId, RenderableEntity>,
  ) {
    this.renderer = renderer;
    this.canvas = canvas;
    this.entityStore = entityStore;
  }

  /**
   * Create a new wgpu renderer adapter.
   *
   * @param canvas - HTML canvas element to render to
   * @param entityStore - Map of entities for getEntity() lookups
   * @param wasmModule - WASM module containing WasmGpuRenderer
   */
  static async create(
    canvas: HTMLCanvasElement,
    entityStore: Map<EntityId, RenderableEntity>,
    wasmModule: { WasmGpuRenderer: WasmGpuRendererStatic },
  ): Promise<WgpuRendererAdapter> {
    const renderer = await wasmModule.WasmGpuRenderer.create(canvas);
    return new WgpuRendererAdapter(renderer, canvas, entityStore);
  }

  // ===========================================================================
  // CanvasRenderer Interface
  // ===========================================================================

  /**
   * Get a renderable entity by ID.
   */
  getEntity(id: EntityId): RenderableEntity | undefined {
    return this.entityStore.get(id);
  }

  /**
   * Queue a draw command for the current frame.
   *
   * Commands are accumulated until flush() is called.
   */
  draw(command: DrawCommand): void {
    this.pendingCommands.push(command);
  }

  /**
   * Flush all pending commands to the GPU.
   *
   * This converts accumulated DrawCommands to CanvasNode JSON and
   * sends them to the WASM renderer in a single call.
   *
   * ## Serialization
   *
   * Uses `wasmBoundaryReplacer` to handle:
   * - `bigint` values (not supported by standard JSON.stringify)
   * - `Rational` objects → `"num/den"` string format (Rust expectation)
   */
  flush(): void {
    if (this.pendingCommands.length === 0) {
      return;
    }

    // Convert DrawCommands to CanvasNodes
    const nodes = this.commandsToCanvasNodes(this.pendingCommands);

    // Serialize with custom replacer for Rational/bigint handling
    const json = JSON.stringify(nodes, wasmBoundaryReplacer);
    this.renderer.render(json);

    // Clear pending commands
    this.pendingCommands = [];
  }

  // ===========================================================================
  // Public Utilities
  // ===========================================================================

  /**
   * Handle canvas resize.
   *
   * Call this when the canvas element is resized.
   */
  resize(width: number, height: number): void {
    this.renderer.resize(width, height);
  }

  /**
   * Get the current render surface dimensions.
   */
  getDimensions(): { width: number; height: number } {
    return {
      width: this.renderer.width,
      height: this.renderer.height,
    };
  }

  /**
   * Update the entity store reference.
   *
   * Call this when the entity store is replaced (e.g., after HMR).
   */
  setEntityStore(store: Map<EntityId, RenderableEntity>): void {
    this.entityStore = store;
  }

  // ===========================================================================
  // Private Methods
  // ===========================================================================

  /**
   * Convert DrawCommands to CanvasNodes.
   *
   * This is the key translation layer between the render loop's command
   * abstraction and the GPU renderer's scene graph.
   */
  private commandsToCanvasNodes(commands: DrawCommand[]): CanvasNode[] {
    const nodes: CanvasNode[] = [];

    for (const cmd of commands) {
      const entity = this.entityStore.get(cmd.entityId);
      if (!entity?.canvas) {
        continue;
      }

      // Use the entity's CanvasNode directly
      // The payload in DrawCommand may contain overrides, but for now
      // we use the entity's stored canvas representation
      nodes.push(entity.canvas);
    }

    return nodes;
  }
}

// =============================================================================
// Factory Function
// =============================================================================

/**
 * Create a wgpu renderer adapter.
 *
 * This is the main entry point for creating a wgpu-based renderer.
 *
 * @param canvas - HTML canvas element to render to
 * @param entityStore - Map of entities for getEntity() lookups
 * @param wasmModule - WASM module containing WasmGpuRenderer
 */
export async function createWgpuRendererAdapter(
  canvas: HTMLCanvasElement,
  entityStore: Map<EntityId, RenderableEntity>,
  wasmModule: { WasmGpuRenderer: WasmGpuRendererStatic },
): Promise<WgpuRendererAdapter> {
  return WgpuRendererAdapter.create(canvas, entityStore, wasmModule);
}
