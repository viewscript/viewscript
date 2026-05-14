/**
 * ESM Module Analyzer for Vite Plugin
 *
 * Extracts exported function names from ESM modules.
 * Uses the AST provided by Vite's `this.parse()` (Rollup-compatible).
 *
 * This module does NOT perform parsing itself - it analyzes the AST
 * provided by the Vite transform context.
 */

import type { Node } from 'estree';

// =============================================================================
// Types
// =============================================================================

/**
 * Information about an exported function.
 */
export interface ExportedFunction {
  /** Function name as exported */
  name: string;
  /** True if this is a re-export from another module */
  isReExport: boolean;
  /** Source module for re-exports (e.g., `export { foo } from './bar'`) */
  sourceModule?: string;
}

/**
 * Result of analyzing an ESM module.
 */
export interface EsmAnalysisResult {
  /** List of exported functions */
  exports: ExportedFunction[];
  /** True if module has a default export */
  hasDefaultExport: boolean;
}

// =============================================================================
// AST Analysis
// =============================================================================

/**
 * Extract exported function names from an ESTree AST.
 *
 * Handles:
 * - `export function foo() {}`
 * - `export const foo = () => {}`
 * - `export const foo = function() {}`
 * - `export { foo, bar }`
 * - `export { foo } from './module'`
 *
 * @param ast - ESTree Program node from Vite's `this.parse()`
 * @returns Analysis result with exported functions
 */
export function analyzeEsmExports(ast: Node): EsmAnalysisResult {
  const result: EsmAnalysisResult = {
    exports: [],
    hasDefaultExport: false,
  };

  if (ast.type !== 'Program') {
    return result;
  }

  const program = ast as unknown as { body: Node[] };

  for (const node of program.body) {
    switch (node.type) {
      case 'ExportNamedDeclaration':
        handleExportNamedDeclaration(node as ExportNamedDeclarationNode, result);
        break;

      case 'ExportDefaultDeclaration':
        result.hasDefaultExport = true;
        break;
    }
  }

  return result;
}

// =============================================================================
// Node Type Definitions (subset of ESTree)
// =============================================================================

interface ExportNamedDeclarationNode {
  type: 'ExportNamedDeclaration';
  declaration?: DeclarationNode | null;
  specifiers: ExportSpecifierNode[];
  source?: { value: string } | null;
}

interface ExportSpecifierNode {
  type: 'ExportSpecifier';
  exported: IdentifierNode;
  local: IdentifierNode;
}

interface IdentifierNode {
  type: 'Identifier';
  name: string;
}

interface DeclarationNode {
  type: string;
  id?: IdentifierNode;
  declarations?: VariableDeclaratorNode[];
}

interface VariableDeclaratorNode {
  type: 'VariableDeclarator';
  id: IdentifierNode;
  init?: InitNode | null;
}

interface InitNode {
  type: string;
}

// =============================================================================
// Declaration Handlers
// =============================================================================

function handleExportNamedDeclaration(
  node: ExportNamedDeclarationNode,
  result: EsmAnalysisResult
): void {
  // Handle `export { foo, bar }` or `export { foo } from './module'`
  if (node.specifiers.length > 0) {
    const sourceModule = node.source?.value;
    for (const spec of node.specifiers) {
      result.exports.push({
        name: spec.exported.name,
        isReExport: sourceModule !== undefined,
        sourceModule,
      });
    }
    return;
  }

  // Handle `export function foo() {}` or `export const foo = ...`
  if (node.declaration) {
    handleDeclaration(node.declaration, result);
  }
}

function handleDeclaration(
  declaration: DeclarationNode,
  result: EsmAnalysisResult
): void {
  switch (declaration.type) {
    case 'FunctionDeclaration':
      // `export function foo() {}`
      if (declaration.id) {
        result.exports.push({
          name: declaration.id.name,
          isReExport: false,
        });
      }
      break;

    case 'VariableDeclaration':
      // `export const foo = () => {}`
      if (declaration.declarations) {
        for (const decl of declaration.declarations) {
          if (isFunctionInit(decl.init)) {
            result.exports.push({
              name: decl.id.name,
              isReExport: false,
            });
          }
        }
      }
      break;
  }
}

/**
 * Check if an initializer is a function (arrow or regular).
 */
function isFunctionInit(init: InitNode | null | undefined): boolean {
  if (!init) return false;
  return init.type === 'ArrowFunctionExpression' || init.type === 'FunctionExpression';
}

// =============================================================================
// Utility Functions
// =============================================================================

/**
 * Check if a name is exported from the analysis result.
 *
 * @param result - Analysis result
 * @param name - Function name to check
 * @returns True if the name is exported
 */
export function hasExport(result: EsmAnalysisResult, name: string): boolean {
  return result.exports.some((exp) => exp.name === name);
}

/**
 * Get all exported function names as a Set for fast lookup.
 *
 * @param result - Analysis result
 * @returns Set of exported function names
 */
export function getExportedNames(result: EsmAnalysisResult): Set<string> {
  return new Set(result.exports.map((exp) => exp.name));
}
