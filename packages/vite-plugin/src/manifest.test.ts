/**
 * manifest.ts Unit Tests
 */

import { describe, it, expect } from 'vitest';
import {
  generateManifest,
  serializeManifest,
  MANIFEST_VERSION,
  type ManifestContext,
  type FfiManifest,
} from './manifest.js';
import type { VsParseResult } from './vs-parser.js';
import type { EsmAnalysisResult } from './esm-analyzer.js';

// =============================================================================
// Test Fixtures
// =============================================================================

function createMockAnalysis(exports: string[]): EsmAnalysisResult {
  return {
    exports: exports.map((name) => ({ name, isReExport: false })),
    hasDefaultExport: false,
  };
}

function createBasicContext(): ManifestContext {
  const vsParseResult: VsParseResult = {
    imports: [
      { names: ['clamp', 'lerp'], modulePath: './math.ts', line: 1 },
      { names: ['notify'], modulePath: './events.ts', line: 2 },
    ],
    binds: [
      { bindName: 'opacity', functionName: 'clamp', args: ['hover_progress', '0', '1'], line: 5 },
    ],
    triggers: [
      {
        triggerName: 'on_click',
        conditionKind: 'bounds_overlap',
        conditionArgs: ['button', 'cursor'],
        functionName: 'notify',
        functionArgs: ['click_data'],
        line: 8,
      },
    ],
    errors: [],
  };

  const resolvedModules = new Map([
    [
      './math.ts',
      {
        originalPath: './math.ts',
        resolvedPath: '/project/src/math.ts',
        analysis: createMockAnalysis(['clamp', 'lerp', 'smoothstep']),
      },
    ],
    [
      './events.ts',
      {
        originalPath: './events.ts',
        resolvedPath: '/project/src/events.ts',
        analysis: createMockAnalysis(['notify', 'log']),
      },
    ],
  ]);

  const entityMap = new Map([
    ['button', 1],
    ['cursor', 2],
    ['panel', 3],
  ]);

  return { vsParseResult, resolvedModules, entityMap };
}

// =============================================================================
// Tests
// =============================================================================

describe('manifest', () => {
  // ===========================================================================
  // Basic Generation
  // ===========================================================================

  describe('generateManifest', () => {
    it('generates manifest with correct version', () => {
      const context = createBasicContext();
      const result = generateManifest(context);

      expect(result.errors).toHaveLength(0);
      expect(result.manifest?.version).toBe(MANIFEST_VERSION);
    });

    it('includes entity map', () => {
      const context = createBasicContext();
      const result = generateManifest(context);

      expect(result.manifest?.entity_map).toEqual({
        button: 1,
        cursor: 2,
        panel: 3,
      });
    });

    it('generates binding from q bind', () => {
      const context = createBasicContext();
      const result = generateManifest(context);

      expect(result.manifest?.bindings).toHaveLength(1);
      expect(result.manifest?.bindings[0]).toEqual({
        ffi_id: 1,
        bind_name: 'opacity',
        module_path: '/project/src/math.ts',
        export_name: 'clamp',
        args: [
          { type: 'q_ref', name: 'hover_progress' },
          { type: 'static', value: 0 },
          { type: 'static', value: 1 },
        ],
      });
    });

    it('generates trigger from q trigger', () => {
      const context = createBasicContext();
      const result = generateManifest(context);

      expect(result.manifest?.triggers).toHaveLength(1);
      expect(result.manifest?.triggers[0]).toEqual({
        trigger_id: 1,
        ffi_id: 2,
        module_path: '/project/src/events.ts',
        export_name: 'notify',
        condition: {
          kind: 'bounds_overlap',
          entity_a: 1,
          entity_b: 2,
        },
        args: [{ type: 'q_ref', name: 'click_data' }],
      });
    });
  });

  // ===========================================================================
  // Argument Parsing
  // ===========================================================================

  describe('argument parsing', () => {
    it('parses numeric literals as static', () => {
      const context = createBasicContext();
      context.vsParseResult.binds[0].args = ['3.14', '-10', '0.5'];
      const result = generateManifest(context);

      expect(result.manifest?.bindings[0].args).toEqual([
        { type: 'static', value: 3.14 },
        { type: 'static', value: -10 },
        { type: 'static', value: 0.5 },
      ]);
    });

    it('parses entity coordinates', () => {
      const context = createBasicContext();
      context.vsParseResult.binds[0].args = ['button.x', 'cursor.y', 'panel.width'];
      const result = generateManifest(context);

      expect(result.manifest?.bindings[0].args).toEqual([
        { type: 'entity_coord', entity_id: 1, component: 'x' },
        { type: 'entity_coord', entity_id: 2, component: 'y' },
        { type: 'entity_coord', entity_id: 3, component: 'width' },
      ]);
    });

    it('parses unknown identifiers as q_ref', () => {
      const context = createBasicContext();
      context.vsParseResult.binds[0].args = ['some_var', 'other_var'];
      const result = generateManifest(context);

      expect(result.manifest?.bindings[0].args).toEqual([
        { type: 'q_ref', name: 'some_var' },
        { type: 'q_ref', name: 'other_var' },
      ]);
    });

    it('handles entity.component for unknown entity as q_ref', () => {
      const context = createBasicContext();
      context.vsParseResult.binds[0].args = ['unknown_entity.x'];
      const result = generateManifest(context);

      // Unknown entity falls through to q_ref
      expect(result.manifest?.bindings[0].args).toEqual([
        { type: 'q_ref', name: 'unknown_entity.x' },
      ]);
    });
  });

  // ===========================================================================
  // Error Handling
  // ===========================================================================

  describe('error handling', () => {
    it('reports error for unimported function in bind', () => {
      const context = createBasicContext();
      context.vsParseResult.binds[0].functionName = 'unknown_func';
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors).toHaveLength(1);
      expect(result.errors[0].message).toContain("'unknown_func' is not imported");
      expect(result.errors[0].line).toBe(5);
    });

    it('reports error for unimported function in trigger', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0].functionName = 'unknown_func';
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors[0].message).toContain("'unknown_func' is not imported");
    });

    it('reports error for unresolved module', () => {
      const context = createBasicContext();
      context.resolvedModules.delete('./math.ts');
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors[0].message).toContain("'./math.ts' could not be resolved");
    });

    it('reports error for function not exported from module', () => {
      const context = createBasicContext();
      const mathModule = context.resolvedModules.get('./math.ts')!;
      mathModule.analysis = createMockAnalysis(['lerp']); // clamp is missing
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors[0].message).toContain("'clamp' is not exported");
    });

    it('reports error for unknown entity in trigger condition', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0].conditionArgs = ['unknown_entity', 'cursor'];
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors[0].message).toContain("Unknown entity 'unknown_entity'");
    });

    it('reports error for unknown condition kind', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0].conditionKind = 'unknown_condition';
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors[0].message).toContain("Unknown condition kind 'unknown_condition'");
    });
  });

  // ===========================================================================
  // Expanded Condition Kinds (B1-B4)
  // ===========================================================================

  describe('expanded condition kinds', () => {
    it('generates properties_equal condition', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0] = {
        triggerName: 'sync_pos',
        conditionKind: 'properties_equal',
        conditionArgs: ['button.x', 'cursor.x'],
        functionName: 'notify',
        functionArgs: [],
        line: 8,
      };
      const result = generateManifest(context);

      expect(result.errors).toHaveLength(0);
      expect(result.manifest?.triggers[0].condition).toEqual({
        kind: 'properties_equal',
        entity_a: 1,
        component_a: 'x',
        entity_b: 2,
        component_b: 'x',
      });
    });

    it('generates property_less_than condition', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0] = {
        triggerName: 'below',
        conditionKind: 'property_less_than',
        conditionArgs: ['button.y', 'panel.y'],
        functionName: 'notify',
        functionArgs: [],
        line: 8,
      };
      const result = generateManifest(context);

      expect(result.errors).toHaveLength(0);
      expect(result.manifest?.triggers[0].condition).toEqual({
        kind: 'property_less_than',
        entity_a: 1,
        component_a: 'y',
        entity_b: 3,
        component_b: 'y',
      });
    });

    it('generates threshold_crossing condition with integer threshold', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0] = {
        triggerName: 'ground_hit',
        conditionKind: 'threshold_crossing',
        conditionArgs: ['button.y', '100', 'falling'],
        functionName: 'notify',
        functionArgs: [],
        line: 8,
      };
      const result = generateManifest(context);

      expect(result.errors).toHaveLength(0);
      expect(result.manifest?.triggers[0].condition).toEqual({
        kind: 'threshold_crossing',
        entity: 1,
        component: 'y',
        threshold: '100/1',
        direction: 'falling',
      });
    });

    it('generates threshold_crossing condition with decimal threshold', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0] = {
        triggerName: 'half_opacity',
        conditionKind: 'threshold_crossing',
        conditionArgs: ['button.t', '0.5', 'rising'],
        functionName: 'notify',
        functionArgs: [],
        line: 8,
      };
      const result = generateManifest(context);

      expect(result.errors).toHaveLength(0);
      expect(result.manifest?.triggers[0].condition).toMatchObject({
        kind: 'threshold_crossing',
        entity: 1,
        component: 't',
        direction: 'rising',
      });
      // Threshold should be rational string (precision depends on implementation)
      expect(result.manifest?.triggers[0].condition).toHaveProperty('threshold');
    });

    it('generates threshold_crossing condition with rational string', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0] = {
        triggerName: 'halfway',
        conditionKind: 'threshold_crossing',
        conditionArgs: ['cursor.x', '1/2', 'rising'],
        functionName: 'notify',
        functionArgs: [],
        line: 8,
      };
      const result = generateManifest(context);

      expect(result.errors).toHaveLength(0);
      expect(result.manifest?.triggers[0].condition).toEqual({
        kind: 'threshold_crossing',
        entity: 2,
        component: 'x',
        threshold: '1/2',
        direction: 'rising',
      });
    });

    it('reports error for invalid property reference in properties_equal', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0] = {
        triggerName: 'bad',
        conditionKind: 'properties_equal',
        conditionArgs: ['button', 'cursor.x'], // missing component
        functionName: 'notify',
        functionArgs: [],
        line: 8,
      };
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors[0].message).toContain("Invalid property reference 'button'");
    });

    it('reports error for invalid direction in threshold_crossing', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0] = {
        triggerName: 'bad',
        conditionKind: 'threshold_crossing',
        conditionArgs: ['button.x', '100', 'sideways'],
        functionName: 'notify',
        functionArgs: [],
        line: 8,
      };
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors[0].message).toContain("Invalid direction 'sideways'");
    });

    it('reports error for invalid threshold in threshold_crossing', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0] = {
        triggerName: 'bad',
        conditionKind: 'threshold_crossing',
        conditionArgs: ['button.x', 'not_a_number', 'rising'],
        functionName: 'notify',
        functionArgs: [],
        line: 8,
      };
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors[0].message).toContain("Invalid threshold 'not_a_number'");
    });

    it('reports error for wrong argument count in properties_equal', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0] = {
        triggerName: 'bad',
        conditionKind: 'properties_equal',
        conditionArgs: ['button.x'], // needs 2
        functionName: 'notify',
        functionArgs: [],
        line: 8,
      };
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors[0].message).toContain('requires 2 arguments');
    });

    it('reports error for wrong argument count in threshold_crossing', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers[0] = {
        triggerName: 'bad',
        conditionKind: 'threshold_crossing',
        conditionArgs: ['button.x', '100'], // needs 3
        functionName: 'notify',
        functionArgs: [],
        line: 8,
      };
      const result = generateManifest(context);

      expect(result.manifest).toBeNull();
      expect(result.errors[0].message).toContain('requires 3 arguments');
    });
  });

  // ===========================================================================
  // Multiple Declarations
  // ===========================================================================

  describe('multiple declarations', () => {
    it('handles multiple binds', () => {
      const context = createBasicContext();
      context.vsParseResult.binds = [
        { bindName: 'a', functionName: 'clamp', args: ['x'], line: 1 },
        { bindName: 'b', functionName: 'lerp', args: ['y'], line: 2 },
      ];
      const result = generateManifest(context);

      expect(result.manifest?.bindings).toHaveLength(2);
      expect(result.manifest?.bindings[0].ffi_id).toBe(1);
      expect(result.manifest?.bindings[1].ffi_id).toBe(2);
    });

    it('handles multiple triggers', () => {
      const context = createBasicContext();
      context.vsParseResult.triggers = [
        {
          triggerName: 't1',
          conditionKind: 'bounds_overlap',
          conditionArgs: ['button', 'cursor'],
          functionName: 'notify',
          functionArgs: [],
          line: 1,
        },
        {
          triggerName: 't2',
          conditionKind: 'bounds_overlap',
          conditionArgs: ['panel', 'cursor'],
          functionName: 'notify',
          functionArgs: [],
          line: 2,
        },
      ];
      const result = generateManifest(context);

      expect(result.manifest?.triggers).toHaveLength(2);
      expect(result.manifest?.triggers[0].trigger_id).toBe(1);
      expect(result.manifest?.triggers[1].trigger_id).toBe(2);
    });

    it('assigns sequential ffi_ids across binds and triggers', () => {
      const context = createBasicContext();
      // 1 bind + 1 trigger in basic context
      const result = generateManifest(context);

      expect(result.manifest?.bindings[0].ffi_id).toBe(1);
      expect(result.manifest?.triggers[0].ffi_id).toBe(2);
    });
  });

  // ===========================================================================
  // Serialization
  // ===========================================================================

  describe('serializeManifest', () => {
    it('produces valid JSON', () => {
      const manifest: FfiManifest = {
        version: 1,
        entity_map: { button: 1 },
        bindings: [],
        triggers: [],
      };

      const json = serializeManifest(manifest);
      const parsed = JSON.parse(json);

      expect(parsed.version).toBe(1);
      expect(parsed.entity_map).toEqual({ button: 1 });
    });

    it('produces formatted output', () => {
      const manifest: FfiManifest = {
        version: 1,
        entity_map: {},
        bindings: [],
        triggers: [],
      };

      const json = serializeManifest(manifest);

      expect(json).toContain('\n'); // Pretty printed
    });
  });
});
