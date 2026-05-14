/**
 * ViewScript Renderer
 *
 * Compiles .vs IR files into target-specific output (wgpu, WebGL, SVG).
 */

export interface RenderTarget {
  name: string;
  compile(ir: VsIR): Promise<CompiledOutput>;
}

export interface VsIR {
  entities: Entity[];
  constraints: Constraint[];
}

export interface Entity {
  id: number;
  type: 'point' | 'curve' | 'surface';
  vector: PVector;
}

export interface PVector {
  x: number;
  y: number;
  z: number;
  t: number;
}

export interface Constraint {
  id: number;
  target: number;
  component: 'x' | 'y' | 'z' | 't';
  relation: 'eq' | 'lt' | 'le' | 'gt' | 'ge';
  term: ConstraintTerm;
}

export type ConstraintTerm =
  | { type: 'const'; value: number }
  | { type: 'ref'; entityId: number; component: 'x' | 'y' | 'z' | 't' }
  | { type: 'linear'; coefficient: number; entityId: number; component: 'x' | 'y' | 'z' | 't'; offset: number };

export interface CompiledOutput {
  html: string;
  js: string;
  css: string;
  assets: Map<string, Uint8Array>;
}

export class WgpuTarget implements RenderTarget {
  name = 'wgpu';

  async compile(ir: VsIR): Promise<CompiledOutput> {
    // TODO: Implement wgpu compilation
    return {
      html: '',
      js: '',
      css: '',
      assets: new Map(),
    };
  }
}
