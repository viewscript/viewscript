/**
 * Tests for SemanticTranslator (Phase 7: Task 20)
 */

import { describe, it, expect } from 'vitest';
import {
  SemanticTranslator,
  createEntityRegistry,
  createEmptyRegistry,
  type EntityMetadata,
  type RawSolution,
} from '../semantic-translator.js';

describe('SemanticTranslator', () => {
  describe('parseVarId', () => {
    it('should parse valid VarId keys', () => {
      const translator = new SemanticTranslator(createEmptyRegistry());

      expect(translator.parseVarId('1:x')).toEqual({ entityId: 1, component: 'x' });
      expect(translator.parseVarId('42:y')).toEqual({ entityId: 42, component: 'y' });
      expect(translator.parseVarId('100:value')).toEqual({ entityId: 100, component: 'value' });
      expect(translator.parseVarId('5:r')).toEqual({ entityId: 5, component: 'r' });
      expect(translator.parseVarId('5:alpha')).toEqual({ entityId: 5, component: 'alpha' });
    });

    it('should return null for invalid keys', () => {
      const translator = new SemanticTranslator(createEmptyRegistry());

      expect(translator.parseVarId('')).toBeNull();
      expect(translator.parseVarId('invalid')).toBeNull();
      expect(translator.parseVarId(':x')).toBeNull();
      expect(translator.parseVarId('1:')).toBeNull();
      expect(translator.parseVarId('1:unknown_component')).toBeNull();
      expect(translator.parseVarId('abc:x')).toBeNull();
    });
  });

  describe('parseRationalToFloat', () => {
    it('should parse rational strings', () => {
      const translator = new SemanticTranslator(createEmptyRegistry());

      expect(translator.parseRationalToFloat('100/1')).toBe(100);
      expect(translator.parseRationalToFloat('1/2')).toBe(0.5);
      expect(translator.parseRationalToFloat('100')).toBe(100);
      expect(translator.parseRationalToFloat('-50/1')).toBe(-50);
      expect(translator.parseRationalToFloat('1/3')).toBeCloseTo(0.333, 2);
    });

    it('should handle zero denominator', () => {
      const translator = new SemanticTranslator(createEmptyRegistry());
      expect(translator.parseRationalToFloat('1/0')).toBeNaN();
    });
  });

  describe('translateSolution', () => {
    it('should translate a simple point solution', () => {
      const registry = createEntityRegistry([
        { entityId: 1, type: 'point', name: 'P1' },
      ]);
      const translator = new SemanticTranslator(registry);

      const rawSolution: RawSolution = new Map([
        ['1:x', '100/1'],
        ['1:y', '200/1'],
      ]);

      const result = translator.translateSolution(rawSolution, 0);

      expect(result.solutionIndex).toBe(0);
      expect(result.entities).toHaveLength(1);
      expect(result.entities[0].entityId).toBe(1);
      expect(result.entities[0].type).toBe('point');
      expect(result.entities[0].coordinates?.x).toBe(100);
      expect(result.entities[0].coordinates?.y).toBe(200);
      expect(result.entities[0].description).toContain('P1');
      expect(result.entities[0].description).toContain('100');
      expect(result.entities[0].description).toContain('200');
    });

    it('should translate multiple entities', () => {
      const registry = createEntityRegistry([
        { entityId: 1, type: 'point', name: 'Start' },
        { entityId: 2, type: 'point', name: 'End' },
        { entityId: 3, type: 'control_point', name: 'CP1' },
      ]);
      const translator = new SemanticTranslator(registry);

      const rawSolution: RawSolution = new Map([
        ['1:x', '0/1'],
        ['1:y', '0/1'],
        ['2:x', '100/1'],
        ['2:y', '0/1'],
        ['3:x', '50/1'],
        ['3:y', '50/1'],
      ]);

      const result = translator.translateSolution(rawSolution, 0);

      expect(result.entities).toHaveLength(3);
      expect(result.summary).toContain('3 entities');
    });

    it('should translate color stop entities', () => {
      const registry = createEntityRegistry([
        { entityId: 10, type: 'color_stop', name: 'Stop1' },
      ]);
      const translator = new SemanticTranslator(registry);

      const rawSolution: RawSolution = new Map([
        ['10:r', '255/1'],
        ['10:g', '128/1'],
        ['10:b', '0/1'],
        ['10:alpha', '1/1'],
        ['10:position', '1/2'],
      ]);

      const result = translator.translateSolution(rawSolution, 0);

      expect(result.entities).toHaveLength(1);
      expect(result.entities[0].type).toBe('color_stop');
      expect(result.entities[0].color?.r).toBe(255);
      expect(result.entities[0].color?.g).toBe(128);
      expect(result.entities[0].color?.b).toBe(0);
      expect(result.entities[0].color?.position).toBe(0.5);
      expect(result.entities[0].description).toContain('50%');
      expect(result.entities[0].description).toContain('rgb(255, 128, 0)');
    });

    it('should handle unknown entity types', () => {
      const translator = new SemanticTranslator(createEmptyRegistry());

      const rawSolution: RawSolution = new Map([
        ['99:x', '50/1'],
        ['99:y', '75/1'],
      ]);

      const result = translator.translateSolution(rawSolution, 0);

      expect(result.entities).toHaveLength(1);
      expect(result.entities[0].type).toBe('unknown');
      expect(result.entities[0].coordinates?.x).toBe(50);
    });

    it('should detect horizontal alignment relationships', () => {
      const registry = createEntityRegistry([
        { entityId: 1, type: 'point' },
        { entityId: 2, type: 'point' },
        { entityId: 3, type: 'point' },
      ]);
      const translator = new SemanticTranslator(registry);

      // Three points on the same horizontal line (y=100)
      const rawSolution: RawSolution = new Map([
        ['1:x', '0/1'],
        ['1:y', '100/1'],
        ['2:x', '50/1'],
        ['2:y', '100/1'],
        ['3:x', '100/1'],
        ['3:y', '100/1'],
      ]);

      const result = translator.translateSolution(rawSolution, 0);

      expect(result.relationships.length).toBeGreaterThan(0);
      const alignment = result.relationships.find((r: { type: string }) => r.type === 'alignment');
      expect(alignment).toBeDefined();
      expect(alignment?.description).toContain('horizontally aligned');
    });
  });

  describe('compareSolutions', () => {
    it('should detect identical solutions', () => {
      const registry = createEntityRegistry([
        { entityId: 1, type: 'point', name: 'P1' },
      ]);
      const translator = new SemanticTranslator(registry);

      const rawSolution: RawSolution = new Map([
        ['1:x', '100/1'],
        ['1:y', '200/1'],
      ]);

      const solution1 = translator.translateSolution(rawSolution, 0);
      const solution2 = translator.translateSolution(rawSolution, 1);

      const diff = translator.compareSolutions(solution1, solution2);

      expect(diff.differingEntities).toHaveLength(0);
      expect(diff.summary).toContain('identical');
    });

    it('should detect differing solutions', () => {
      const registry = createEntityRegistry([
        { entityId: 1, type: 'point', name: 'P1' },
      ]);
      const translator = new SemanticTranslator(registry);

      const solution1 = translator.translateSolution(
        new Map([['1:x', '100/1'], ['1:y', '200/1']]),
        0,
      );
      const solution2 = translator.translateSolution(
        new Map([['1:x', '150/1'], ['1:y', '250/1']]),
        1,
      );

      const diff = translator.compareSolutions(solution1, solution2);

      expect(diff.differingEntities).toHaveLength(1);
      expect(diff.differingEntities[0].entityId).toBe(1);
      expect(diff.summary).toContain('1 entities differ');
    });
  });

  describe('translateMultipleSolutions', () => {
    it('should translate and compare multiple solutions', () => {
      const registry = createEntityRegistry([
        { entityId: 1, type: 'point', name: 'P1' },
      ]);
      const translator = new SemanticTranslator(registry);

      const rawSolutions: RawSolution[] = [
        new Map([['1:x', '100/1'], ['1:y', '0/1']]),
        new Map([['1:x', '-100/1'], ['1:y', '0/1']]),
      ];

      const result = translator.translateMultipleSolutions(rawSolutions);

      expect(result.solutions).toHaveLength(2);
      expect(result.comparison).toContain('2 solutions found');
      expect(result.comparison).toContain('differ');
    });

    it('should handle single solution', () => {
      const translator = new SemanticTranslator(createEmptyRegistry());

      const rawSolutions: RawSolution[] = [
        new Map([['1:x', '100/1']]),
      ];

      const result = translator.translateMultipleSolutions(rawSolutions);

      expect(result.solutions).toHaveLength(1);
      expect(result.comparison).toContain('Only one solution');
    });
  });
});

describe('createEntityRegistry', () => {
  it('should create a working registry', () => {
    const entities: EntityMetadata[] = [
      { entityId: 1, type: 'point', name: 'P1' },
      { entityId: 2, type: 'rect', name: 'R1' },
    ];

    const registry = createEntityRegistry(entities);

    expect(registry.get(1)?.type).toBe('point');
    expect(registry.get(2)?.name).toBe('R1');
    expect(registry.get(999)).toBeUndefined();
    expect(registry.getAll()).toHaveLength(2);
  });
});
