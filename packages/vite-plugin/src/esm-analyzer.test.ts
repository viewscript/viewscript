/**
 * esm-analyzer.ts Unit Tests
 */

import { describe, it, expect } from 'vitest';
import {
  analyzeEsmExports,
  hasExport,
  getExportedNames,
  type EsmAnalysisResult,
  type ExportedFunction,
} from './esm-analyzer.js';

// =============================================================================
// AST Factory Helpers
// =============================================================================

function createProgram(body: unknown[]): unknown {
  return { type: 'Program', body };
}

function createExportNamedDeclaration(opts: {
  declaration?: unknown;
  specifiers?: unknown[];
  source?: string;
}): unknown {
  return {
    type: 'ExportNamedDeclaration',
    declaration: opts.declaration ?? null,
    specifiers: opts.specifiers ?? [],
    source: opts.source ? { value: opts.source } : null,
  };
}

function createFunctionDeclaration(name: string): unknown {
  return {
    type: 'FunctionDeclaration',
    id: { type: 'Identifier', name },
  };
}

function createVariableDeclaration(declarations: unknown[]): unknown {
  return {
    type: 'VariableDeclaration',
    declarations,
  };
}

function createVariableDeclarator(name: string, init: unknown): unknown {
  return {
    type: 'VariableDeclarator',
    id: { type: 'Identifier', name },
    init,
  };
}

function createArrowFunction(): unknown {
  return { type: 'ArrowFunctionExpression' };
}

function createFunctionExpression(): unknown {
  return { type: 'FunctionExpression' };
}

function createExportSpecifier(exported: string, local?: string): unknown {
  return {
    type: 'ExportSpecifier',
    exported: { type: 'Identifier', name: exported },
    local: { type: 'Identifier', name: local ?? exported },
  };
}

function createExportDefaultDeclaration(): unknown {
  return { type: 'ExportDefaultDeclaration' };
}

// =============================================================================
// Tests
// =============================================================================

describe('esm-analyzer', () => {
  // ===========================================================================
  // Function Declaration Exports
  // ===========================================================================

  describe('export function declarations', () => {
    it('extracts exported function declaration', () => {
      const ast = createProgram([
        createExportNamedDeclaration({
          declaration: createFunctionDeclaration('clamp'),
        }),
      ]);

      const result = analyzeEsmExports(ast as never);

      expect(result.exports).toHaveLength(1);
      expect(result.exports[0]).toEqual({
        name: 'clamp',
        isReExport: false,
      });
    });

    it('extracts multiple exported functions', () => {
      const ast = createProgram([
        createExportNamedDeclaration({
          declaration: createFunctionDeclaration('clamp'),
        }),
        createExportNamedDeclaration({
          declaration: createFunctionDeclaration('lerp'),
        }),
      ]);

      const result = analyzeEsmExports(ast as never);

      expect(result.exports).toHaveLength(2);
      expect(result.exports.map((e: ExportedFunction) => e.name)).toEqual(['clamp', 'lerp']);
    });
  });

  // ===========================================================================
  // Arrow Function Exports
  // ===========================================================================

  describe('export const arrow functions', () => {
    it('extracts exported arrow function', () => {
      const ast = createProgram([
        createExportNamedDeclaration({
          declaration: createVariableDeclaration([
            createVariableDeclarator('smoothstep', createArrowFunction()),
          ]),
        }),
      ]);

      const result = analyzeEsmExports(ast as never);

      expect(result.exports).toHaveLength(1);
      expect(result.exports[0].name).toBe('smoothstep');
    });

    it('extracts multiple arrow functions in single export', () => {
      const ast = createProgram([
        createExportNamedDeclaration({
          declaration: createVariableDeclaration([
            createVariableDeclarator('add', createArrowFunction()),
            createVariableDeclarator('sub', createArrowFunction()),
          ]),
        }),
      ]);

      const result = analyzeEsmExports(ast as never);

      expect(result.exports).toHaveLength(2);
    });

    it('extracts exported function expression', () => {
      const ast = createProgram([
        createExportNamedDeclaration({
          declaration: createVariableDeclaration([
            createVariableDeclarator('notify', createFunctionExpression()),
          ]),
        }),
      ]);

      const result = analyzeEsmExports(ast as never);

      expect(result.exports).toHaveLength(1);
      expect(result.exports[0].name).toBe('notify');
    });

    it('ignores non-function exports', () => {
      const ast = createProgram([
        createExportNamedDeclaration({
          declaration: createVariableDeclaration([
            createVariableDeclarator('PI', { type: 'Literal', value: 3.14 }),
          ]),
        }),
      ]);

      const result = analyzeEsmExports(ast as never);

      expect(result.exports).toHaveLength(0);
    });
  });

  // ===========================================================================
  // Named Export Specifiers
  // ===========================================================================

  describe('export specifiers', () => {
    it('extracts named exports', () => {
      const ast = createProgram([
        createExportNamedDeclaration({
          specifiers: [
            createExportSpecifier('foo'),
            createExportSpecifier('bar'),
          ],
        }),
      ]);

      const result = analyzeEsmExports(ast as never);

      expect(result.exports).toHaveLength(2);
      expect(result.exports[0]).toEqual({
        name: 'foo',
        isReExport: false,
        sourceModule: undefined,
      });
    });

    it('extracts re-exports with source', () => {
      const ast = createProgram([
        createExportNamedDeclaration({
          specifiers: [createExportSpecifier('clamp')],
          source: './utils/math',
        }),
      ]);

      const result = analyzeEsmExports(ast as never);

      expect(result.exports).toHaveLength(1);
      expect(result.exports[0]).toEqual({
        name: 'clamp',
        isReExport: true,
        sourceModule: './utils/math',
      });
    });
  });

  // ===========================================================================
  // Default Export
  // ===========================================================================

  describe('default export', () => {
    it('detects default export', () => {
      const ast = createProgram([createExportDefaultDeclaration()]);

      const result = analyzeEsmExports(ast as never);

      expect(result.hasDefaultExport).toBe(true);
    });

    it('hasDefaultExport is false when no default', () => {
      const ast = createProgram([
        createExportNamedDeclaration({
          declaration: createFunctionDeclaration('foo'),
        }),
      ]);

      const result = analyzeEsmExports(ast as never);

      expect(result.hasDefaultExport).toBe(false);
    });
  });

  // ===========================================================================
  // Edge Cases
  // ===========================================================================

  describe('edge cases', () => {
    it('handles empty program', () => {
      const ast = createProgram([]);

      const result = analyzeEsmExports(ast as never);

      expect(result.exports).toHaveLength(0);
      expect(result.hasDefaultExport).toBe(false);
    });

    it('handles non-Program node', () => {
      const result = analyzeEsmExports({ type: 'Identifier', name: 'x' } as never);

      expect(result.exports).toHaveLength(0);
    });
  });

  // ===========================================================================
  // Utility Functions
  // ===========================================================================

  describe('hasExport', () => {
    it('returns true for existing export', () => {
      const result: EsmAnalysisResult = {
        exports: [{ name: 'clamp', isReExport: false }],
        hasDefaultExport: false,
      };

      expect(hasExport(result, 'clamp')).toBe(true);
    });

    it('returns false for missing export', () => {
      const result: EsmAnalysisResult = {
        exports: [{ name: 'clamp', isReExport: false }],
        hasDefaultExport: false,
      };

      expect(hasExport(result, 'lerp')).toBe(false);
    });
  });

  describe('getExportedNames', () => {
    it('returns Set of all exported names', () => {
      const result: EsmAnalysisResult = {
        exports: [
          { name: 'clamp', isReExport: false },
          { name: 'lerp', isReExport: false },
          { name: 'notify', isReExport: true, sourceModule: './events' },
        ],
        hasDefaultExport: false,
      };

      const names = getExportedNames(result);

      expect(names).toEqual(new Set(['clamp', 'lerp', 'notify']));
    });
  });
});
