/**
 * ViewScript (.vs) Parser - TypeScript AST Based
 *
 * Parses .vs files which use TypeScript syntax:
 *
 * ```vs
 * import { getCount, increment } from "./logic"
 *
 * export default {
 *   bg: rect({ x: 100, y: 100, width: 320, height: 200, fill: "#1e1e2e" }),
 *   label: text({ x: bg.x + bg.width / 2, content: getCount, fill: "#cdd6f4" }),
 *   btn: rect({
 *     x: 200, y: 300, width: 80, height: 48,
 *     interactive: true,
 *     onClick: { type: 'increment', target: 'count', delta: 1 }
 *   }),
 * }
 * ```
 *
 * Uses TypeScript compiler API for robust parsing of:
 * - Import statements
 * - Export default object literal (scene definition)
 * - Component declarations via function calls (rect, text, etc.)
 * - Expression references (bg.x + bg.width / 2)
 * - Interactive flags and event bindings
 *
 * Also supports legacy `q bind` / `q trigger` syntax for backward compatibility.
 */

import ts from 'typescript';

// =============================================================================
// Types
// =============================================================================

/**
 * Parsed import statement.
 */
export interface VsImport {
  names: string[];
  modulePath: string;
  line: number;
}

/**
 * Parsed component declaration from function call syntax.
 * e.g., `bg: rect({ x: 100, ... })`
 */
export interface VsComponentDecl {
  /** Component instance name (e.g., "bg") */
  name: string;
  /** Component type (e.g., "rect", "text") */
  type: string;
  /** Properties as key-value pairs */
  properties: Record<string, VsPropertyValue>;
  /** Whether this component is interactive */
  interactive: boolean;
  /** Event bindings (onClick, etc.) */
  eventBindings: VsEventBinding[];
  /** Line number */
  line: number;
}

/**
 * Property value - can be literal, reference, or expression.
 */
export type VsPropertyValue =
  | { kind: 'literal'; value: string | number | boolean }
  | { kind: 'reference'; entity: string; component?: string }
  | { kind: 'ffiCall'; functionName: string; args: VsPropertyValue[] }
  | { kind: 'expression'; ast: VsExpressionNode; source: string };

/**
 * Expression AST node for constraint generation.
 */
export type VsExpressionNode =
  | { type: 'const'; value: number }
  | { type: 'ref'; entity: string; component: string }
  | { type: 'binary'; op: '+' | '-' | '*' | '/'; left: VsExpressionNode; right: VsExpressionNode }
  | { type: 'call'; name: string; args: VsExpressionNode[] };

/**
 * Event binding from onClick, onHover, etc.
 */
export interface VsEventBinding {
  event: 'click' | 'hover' | 'pointerdown' | 'pointerup';
  action: VsEventAction;
  line: number;
}

/**
 * Event action types.
 */
export type VsEventAction =
  | { type: 'increment'; target: string; delta: number }
  | { type: 'decrement'; target: string; delta: number }
  | { type: 'toggle'; target: string; values: [number, number] }
  | { type: 'set'; target: string; value: number }
  | { type: 'call'; handler: string; args?: string[] };

// Legacy types for backward compatibility
export interface VsBind {
  bindName: string;
  functionName: string;
  args: string[];
  line: number;
}

export interface VsTrigger {
  triggerName: string;
  conditionKind: string;
  conditionArgs: string[];
  functionName: string;
  functionArgs: string[];
  line: number;
}

// Legacy component types (block syntax)
export interface VsComponent {
  name: string;
  type: string;
  properties: Record<string, string>;
  eventHandlers: VsEventHandler[];
  line: number;
}

export interface VsEventHandler {
  event: string;
  body: string;
  line: number;
}

export interface VsScene {
  components: string[];
  line: number;
}

export interface VsConst {
  name: string;
  value: string;
  line: number;
}

export interface VsParseError {
  message: string;
  line: number;
}

/**
 * Result of parsing a .vs file.
 */
export interface VsParseResult {
  imports: VsImport[];
  /** New-style component declarations (function call syntax) */
  componentDecls: VsComponentDecl[];
  /** Legacy q bind declarations */
  binds: VsBind[];
  /** Legacy q trigger declarations */
  triggers: VsTrigger[];
  /** Legacy block-style components */
  components: VsComponent[];
  /** Legacy scene declaration */
  scene: VsScene | null;
  /** Legacy const declarations */
  consts: VsConst[];
  errors: VsParseError[];
}

// =============================================================================
// Main Parser
// =============================================================================

/**
 * Parse a .vs file using TypeScript compiler API.
 */
export function parseVsFile(content: string, filename = 'input.vs'): VsParseResult {
  const result: VsParseResult = {
    imports: [],
    componentDecls: [],
    binds: [],
    triggers: [],
    components: [],
    scene: null,
    consts: [],
    errors: [],
  };

  // Create TypeScript source file
  const sourceFile = ts.createSourceFile(
    filename,
    content,
    ts.ScriptTarget.ES2022,
    true,
    ts.ScriptKind.TS
  );

  // Walk top-level statements
  for (const statement of sourceFile.statements) {
    try {
      if (ts.isImportDeclaration(statement)) {
        parseImportDeclaration(statement, sourceFile, result);
      } else if (ts.isExportAssignment(statement)) {
        parseExportDefault(statement, sourceFile, result);
      } else if (ts.isVariableStatement(statement)) {
        // Check for legacy `q bind` or const declarations
        parseLegacyStatements(statement, sourceFile, result);
      } else if (ts.isExpressionStatement(statement)) {
        // Check for legacy q declarations
        parseLegacyExpression(statement, sourceFile, result);
      }
    } catch (err) {
      const line = sourceFile.getLineAndCharacterOfPosition(statement.getStart()).line + 1;
      result.errors.push({
        message: `Parse error: ${err instanceof Error ? err.message : String(err)}`,
        line,
      });
    }
  }

  // Also parse legacy line-based syntax for backward compatibility
  parseLegacyLineBased(content, result);

  return result;
}

// =============================================================================
// Import Parsing
// =============================================================================

function parseImportDeclaration(
  node: ts.ImportDeclaration,
  sourceFile: ts.SourceFile,
  result: VsParseResult
): void {
  const line = sourceFile.getLineAndCharacterOfPosition(node.getStart()).line + 1;

  // Get module path
  if (!ts.isStringLiteral(node.moduleSpecifier)) return;
  const modulePath = node.moduleSpecifier.text;

  // Get imported names
  const names: string[] = [];
  const importClause = node.importClause;

  if (importClause) {
    // Named imports: import { a, b } from "..." or import { a as b } from "..."
    if (importClause.namedBindings && ts.isNamedImports(importClause.namedBindings)) {
      for (const element of importClause.namedBindings.elements) {
        // If aliased (import { x as y }), use the original name (x), not the alias (y)
        // This matches the legacy behavior
        const originalName = element.propertyName?.text ?? element.name.text;
        names.push(originalName);
      }
    }
    // Default import: import x from "..."
    if (importClause.name) {
      names.push(importClause.name.text);
    }
  }

  if (names.length > 0) {
    result.imports.push({ names, modulePath, line });
  }
}

// =============================================================================
// Export Default Parsing
// =============================================================================

function parseExportDefault(
  node: ts.ExportAssignment,
  sourceFile: ts.SourceFile,
  result: VsParseResult
): void {
  // export default { ... }
  if (!ts.isObjectLiteralExpression(node.expression)) {
    return;
  }

  const objectLiteral = node.expression;

  for (const prop of objectLiteral.properties) {
    if (!ts.isPropertyAssignment(prop)) continue;
    if (!ts.isIdentifier(prop.name)) continue;

    const componentName = prop.name.text;
    const line = sourceFile.getLineAndCharacterOfPosition(prop.getStart()).line + 1;

    // Check if value is a function call: rect({ ... }), text({ ... })
    if (ts.isCallExpression(prop.initializer)) {
      const call = prop.initializer;
      if (ts.isIdentifier(call.expression)) {
        const componentType = call.expression.text;
        const componentDecl = parseComponentCall(componentName, componentType, call, sourceFile, line);
        if (componentDecl) {
          result.componentDecls.push(componentDecl);
        }
      }
    }
  }
}

function parseComponentCall(
  name: string,
  type: string,
  call: ts.CallExpression,
  sourceFile: ts.SourceFile,
  line: number
): VsComponentDecl | null {
  // Expect single object argument: rect({ ... })
  if (call.arguments.length !== 1) return null;
  const arg = call.arguments[0];
  if (!ts.isObjectLiteralExpression(arg)) return null;

  const properties: Record<string, VsPropertyValue> = {};
  let interactive = false;
  const eventBindings: VsEventBinding[] = [];

  for (const prop of arg.properties) {
    if (!ts.isPropertyAssignment(prop)) continue;
    if (!ts.isIdentifier(prop.name)) continue;

    const propName = prop.name.text;
    const propLine = sourceFile.getLineAndCharacterOfPosition(prop.getStart()).line + 1;

    // Check for special properties
    if (propName === 'interactive') {
      if (prop.initializer.kind === ts.SyntaxKind.TrueKeyword) {
        interactive = true;
      }
      continue;
    }

    if (propName.startsWith('on') && propName.length > 2) {
      // Event binding: onClick, onHover, etc.
      const eventType = propName.slice(2).toLowerCase() as VsEventBinding['event'];
      const action = parseEventAction(prop.initializer, sourceFile);
      if (action) {
        eventBindings.push({ event: eventType, action, line: propLine });
      }
      continue;
    }

    // Regular property
    const value = parsePropertyValue(prop.initializer, sourceFile);
    if (value) {
      properties[propName] = value;
    }
  }

  return { name, type, properties, interactive, eventBindings, line };
}

// =============================================================================
// Property Value Parsing
// =============================================================================

function parsePropertyValue(node: ts.Expression, sourceFile: ts.SourceFile): VsPropertyValue | null {
  // String literal
  if (ts.isStringLiteral(node)) {
    return { kind: 'literal', value: node.text };
  }

  // Numeric literal
  if (ts.isNumericLiteral(node)) {
    return { kind: 'literal', value: parseFloat(node.text) };
  }

  // Boolean literal
  if (node.kind === ts.SyntaxKind.TrueKeyword) {
    return { kind: 'literal', value: true };
  }
  if (node.kind === ts.SyntaxKind.FalseKeyword) {
    return { kind: 'literal', value: false };
  }

  // Simple identifier (reference to another component or FFI function)
  if (ts.isIdentifier(node)) {
    // Could be a component reference or FFI function reference
    return { kind: 'reference', entity: node.text };
  }

  // Property access: bg.x, bg.width
  if (ts.isPropertyAccessExpression(node)) {
    if (ts.isIdentifier(node.expression) && ts.isIdentifier(node.name)) {
      return {
        kind: 'reference',
        entity: node.expression.text,
        component: node.name.text,
      };
    }
  }

  // Function call: getCount(), clamp(value, 0, 1)
  if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
    const args = node.arguments.map(arg => parsePropertyValue(arg, sourceFile)).filter(Boolean) as VsPropertyValue[];
    return {
      kind: 'ffiCall',
      functionName: node.expression.text,
      args,
    };
  }

  // Binary expression: bg.x + bg.width / 2
  if (ts.isBinaryExpression(node)) {
    const ast = parseExpressionToAst(node, sourceFile);
    const source = node.getText(sourceFile);
    if (ast) {
      return { kind: 'expression', ast, source };
    }
  }

  // Prefix unary: -100
  if (ts.isPrefixUnaryExpression(node) && node.operator === ts.SyntaxKind.MinusToken) {
    if (ts.isNumericLiteral(node.operand)) {
      return { kind: 'literal', value: -parseFloat(node.operand.text) };
    }
  }

  return null;
}

// =============================================================================
// Expression AST Parsing
// =============================================================================

function parseExpressionToAst(node: ts.Expression, sourceFile: ts.SourceFile): VsExpressionNode | null {
  // Numeric literal
  if (ts.isNumericLiteral(node)) {
    return { type: 'const', value: parseFloat(node.text) };
  }

  // Parenthesized expression
  if (ts.isParenthesizedExpression(node)) {
    return parseExpressionToAst(node.expression, sourceFile);
  }

  // Property access: entity.component
  if (ts.isPropertyAccessExpression(node)) {
    if (ts.isIdentifier(node.expression) && ts.isIdentifier(node.name)) {
      return {
        type: 'ref',
        entity: node.expression.text,
        component: node.name.text,
      };
    }
    // Nested: handle deeper property access by unwrapping
    return null;
  }

  // Binary expression
  if (ts.isBinaryExpression(node)) {
    const left = parseExpressionToAst(node.left, sourceFile);
    const right = parseExpressionToAst(node.right, sourceFile);
    if (!left || !right) return null;

    let op: '+' | '-' | '*' | '/';
    switch (node.operatorToken.kind) {
      case ts.SyntaxKind.PlusToken: op = '+'; break;
      case ts.SyntaxKind.MinusToken: op = '-'; break;
      case ts.SyntaxKind.AsteriskToken: op = '*'; break;
      case ts.SyntaxKind.SlashToken: op = '/'; break;
      default: return null;
    }

    return { type: 'binary', op, left, right };
  }

  // Function call
  if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
    const args = node.arguments
      .map(arg => parseExpressionToAst(arg, sourceFile))
      .filter(Boolean) as VsExpressionNode[];
    return {
      type: 'call',
      name: node.expression.text,
      args,
    };
  }

  // Simple identifier (treat as entity.x by default)
  if (ts.isIdentifier(node)) {
    return { type: 'ref', entity: node.text, component: 'value' };
  }

  // Prefix unary: -value
  if (ts.isPrefixUnaryExpression(node) && node.operator === ts.SyntaxKind.MinusToken) {
    const operand = parseExpressionToAst(node.operand, sourceFile);
    if (operand) {
      return {
        type: 'binary',
        op: '*',
        left: { type: 'const', value: -1 },
        right: operand,
      };
    }
  }

  return null;
}

// =============================================================================
// Event Action Parsing
// =============================================================================

function parseEventAction(node: ts.Expression, sourceFile: ts.SourceFile): VsEventAction | null {
  // Object literal: { type: 'increment', target: 'count', delta: 1 }
  if (ts.isObjectLiteralExpression(node)) {
    let actionType: string | undefined;
    let target: string | undefined;
    let delta: number | undefined;
    let value: number | undefined;
    let values: [number, number] | undefined;
    let handler: string | undefined;

    for (const prop of node.properties) {
      if (!ts.isPropertyAssignment(prop)) continue;
      if (!ts.isIdentifier(prop.name)) continue;

      const propName = prop.name.text;

      if (propName === 'type' && ts.isStringLiteral(prop.initializer)) {
        actionType = prop.initializer.text;
      }
      if (propName === 'target' && ts.isStringLiteral(prop.initializer)) {
        target = prop.initializer.text;
      }
      if (propName === 'delta' && ts.isNumericLiteral(prop.initializer)) {
        delta = parseFloat(prop.initializer.text);
      }
      if (propName === 'value' && ts.isNumericLiteral(prop.initializer)) {
        value = parseFloat(prop.initializer.text);
      }
      if (propName === 'handler' && ts.isStringLiteral(prop.initializer)) {
        handler = prop.initializer.text;
      }
      if (propName === 'values' && ts.isArrayLiteralExpression(prop.initializer)) {
        const [v1, v2] = prop.initializer.elements;
        if (ts.isNumericLiteral(v1) && ts.isNumericLiteral(v2)) {
          values = [parseFloat(v1.text), parseFloat(v2.text)];
        }
      }
    }

    if (actionType === 'increment' && target && delta !== undefined) {
      return { type: 'increment', target, delta };
    }
    if (actionType === 'decrement' && target && delta !== undefined) {
      return { type: 'decrement', target, delta };
    }
    if (actionType === 'toggle' && target && values) {
      return { type: 'toggle', target, values };
    }
    if (actionType === 'set' && target && value !== undefined) {
      return { type: 'set', target, value };
    }
    if (actionType === 'call' && handler) {
      return { type: 'call', handler };
    }
  }

  // Simple function call: increment
  if (ts.isIdentifier(node)) {
    return { type: 'call', handler: node.text };
  }

  return null;
}

// =============================================================================
// Legacy Parsing (for backward compatibility)
// =============================================================================

function parseLegacyStatements(
  statement: ts.VariableStatement,
  sourceFile: ts.SourceFile,
  result: VsParseResult
): void {
  // Handle const declarations
  for (const decl of statement.declarationList.declarations) {
    if (!ts.isIdentifier(decl.name)) continue;
    if (!decl.initializer) continue;

    const line = sourceFile.getLineAndCharacterOfPosition(decl.getStart()).line + 1;
    const name = decl.name.text;
    const value = decl.initializer.getText(sourceFile);

    result.consts.push({ name, value, line });
  }
}

function parseLegacyExpression(
  statement: ts.ExpressionStatement,
  sourceFile: ts.SourceFile,
  result: VsParseResult
): void {
  // Legacy q declarations would be parsed here if needed
  // Currently handled by line-based parsing below
}

// =============================================================================
// Legacy Line-Based Parsing
// =============================================================================

const IMPORT_PATTERN = /^\s*import\s*\{\s*([^}]+)\s*\}\s*from\s*["']([^"']+)["']\s*$/;
const BIND_PATTERN = /^\s*q\s+bind\s+(\w+)\s*=\s*(\w+)\s*\(\s*([^)]*)\s*\)\s*$/;
const TRIGGER_PATTERN = /^\s*q\s+trigger\s+(\w+)\s*=\s*(\w+)\s*\(\s*([^)]*)\s*\)\s*->\s*(\w+)\s*\(\s*([^)]*)\s*\)\s*$/;
const COMPONENT_PATTERN = /^\s*component\s+(\w+)\s*:\s*(\w+)\s*\{\s*$/;
const SCENE_PATTERN = /^\s*scene\s*\{\s*$/;
const PROPERTY_PATTERN = /^\s*(\w+)\s*:\s*(.+?)\s*$/;
const EVENT_HANDLER_PATTERN = /^\s*on\s+(\w+)\s*\{\s*$/;
const CONST_PATTERN = /^\s*const\s+(\w+)\s*=\s*(.+?)\s*$/;
const CLOSE_BRACE_PATTERN = /^\s*\}\s*$/;

type LegacyParserState =
  | { type: 'root' }
  | { type: 'component'; component: VsComponent }
  | { type: 'event'; component: VsComponent; handler: VsEventHandler; braceDepth: number }
  | { type: 'scene'; scene: VsScene };

function parseLegacyLineBased(content: string, result: VsParseResult): void {
  const lines = content.split('\n');
  let state: LegacyParserState = { type: 'root' };

  for (let i = 0; i < lines.length; i++) {
    const lineNumber = i + 1;
    const line = lines[i];
    const trimmed = line.trim();

    if (!trimmed || trimmed.startsWith('//') || trimmed.startsWith('/*')) {
      continue;
    }

    switch (state.type) {
      case 'root':
        state = parseLegacyRootLine(line, lineNumber, result, state);
        break;
      case 'component':
        state = parseLegacyComponentLine(line, lineNumber, result, state);
        break;
      case 'event':
        state = parseLegacyEventLine(line, lineNumber, result, state);
        break;
      case 'scene':
        state = parseLegacySceneLine(line, lineNumber, result, state);
        break;
    }
  }

  if (state.type !== 'root') {
    result.errors.push({
      message: `Unclosed ${state.type} block at end of file`,
      line: lines.length,
    });
  }
}

function parseArgs(argsStr: string): string[] {
  if (!argsStr.trim()) return [];
  return argsStr.split(',').map(arg => arg.trim()).filter(Boolean);
}

function parseLegacyRootLine(
  line: string,
  lineNumber: number,
  result: VsParseResult,
  state: LegacyParserState
): LegacyParserState {
  const trimmed = line.trim();

  // q bind
  const bindMatch = line.match(BIND_PATTERN);
  if (bindMatch) {
    result.binds.push({
      bindName: bindMatch[1],
      functionName: bindMatch[2],
      args: parseArgs(bindMatch[3]),
      line: lineNumber,
    });
    return state;
  }

  // q trigger
  const triggerMatch = line.match(TRIGGER_PATTERN);
  if (triggerMatch) {
    result.triggers.push({
      triggerName: triggerMatch[1],
      conditionKind: triggerMatch[2],
      conditionArgs: parseArgs(triggerMatch[3]),
      functionName: triggerMatch[4],
      functionArgs: parseArgs(triggerMatch[5]),
      line: lineNumber,
    });
    return state;
  }

  // component block
  const componentMatch = line.match(COMPONENT_PATTERN);
  if (componentMatch) {
    return {
      type: 'component',
      component: {
        name: componentMatch[1],
        type: componentMatch[2],
        properties: {},
        eventHandlers: [],
        line: lineNumber,
      },
    };
  }

  // scene block
  const sceneMatch = line.match(SCENE_PATTERN);
  if (sceneMatch) {
    return {
      type: 'scene',
      scene: { components: [], line: lineNumber },
    };
  }

  // Invalid q declaration
  if (trimmed.startsWith('q ') && !trimmed.startsWith('q env') && !trimmed.startsWith('q input')) {
    result.errors.push({
      message: `Invalid q declaration: ${trimmed}`,
      line: lineNumber,
    });
  }

  return state;
}

function parseLegacyComponentLine(
  line: string,
  lineNumber: number,
  result: VsParseResult,
  state: LegacyParserState & { type: 'component' }
): LegacyParserState {
  const trimmed = line.trim();

  if (CLOSE_BRACE_PATTERN.test(line)) {
    result.components.push(state.component);
    return { type: 'root' };
  }

  const eventMatch = line.match(EVENT_HANDLER_PATTERN);
  if (eventMatch) {
    return {
      type: 'event',
      component: state.component,
      handler: { event: eventMatch[1], body: '', line: lineNumber },
      braceDepth: 1,
    };
  }

  const propMatch = trimmed.match(PROPERTY_PATTERN);
  if (propMatch) {
    state.component.properties[propMatch[1]] = propMatch[2];
  }

  return state;
}

function parseLegacyEventLine(
  line: string,
  lineNumber: number,
  result: VsParseResult,
  state: LegacyParserState & { type: 'event' }
): LegacyParserState {
  const trimmed = line.trim();
  const openBraces = (line.match(/\{/g) || []).length;
  const closeBraces = (line.match(/\}/g) || []).length;
  const newDepth = state.braceDepth + openBraces - closeBraces;

  if (newDepth === 0) {
    state.component.eventHandlers.push(state.handler);
    return { type: 'component', component: state.component };
  }

  if (trimmed && !CLOSE_BRACE_PATTERN.test(line)) {
    state.handler.body += (state.handler.body ? '\n' : '') + trimmed;
  }

  return { ...state, braceDepth: newDepth };
}

function parseLegacySceneLine(
  line: string,
  lineNumber: number,
  result: VsParseResult,
  state: LegacyParserState & { type: 'scene' }
): LegacyParserState {
  const trimmed = line.trim();

  if (CLOSE_BRACE_PATTERN.test(line)) {
    result.scene = state.scene;
    return { type: 'root' };
  }

  if (/^\w+$/.test(trimmed)) {
    state.scene.components.push(trimmed);
  }

  return state;
}

// =============================================================================
// Import Validation (for legacy support)
// =============================================================================

/**
 * Validate that all function references in binds/triggers are imported.
 */
export function validateImports(result: VsParseResult): VsParseError[] {
  const errors: VsParseError[] = [];
  const importedNames = new Set<string>();

  for (const imp of result.imports) {
    for (const name of imp.names) {
      importedNames.add(name);
    }
  }

  // Check binds
  for (const bind of result.binds) {
    if (!importedNames.has(bind.functionName)) {
      errors.push({
        message: `Function '${bind.functionName}' used in bind '${bind.bindName}' is not imported`,
        line: bind.line,
      });
    }
  }

  // Check triggers
  for (const trigger of result.triggers) {
    if (!importedNames.has(trigger.functionName)) {
      errors.push({
        message: `Function '${trigger.functionName}' used in trigger '${trigger.triggerName}' is not imported`,
        line: trigger.line,
      });
    }
  }

  return errors;
}
