/**
 * ViewScript (.vs) Parser for Vite Plugin
 *
 * Line-by-line parser extracting FFI-related declarations:
 * - import statements
 * - q bind declarations
 * - q trigger declarations
 *
 * This is NOT a full AST parser. It uses regex matching to extract
 * only the information needed for FFI manifest generation.
 */

// =============================================================================
// Types
// =============================================================================

/**
 * Parsed import statement.
 *
 * Example: `import { clamp, lerp } from "./utils/math.ts"`
 */
export interface VsImport {
  /** Imported names (e.g., ["clamp", "lerp"]) */
  names: string[];
  /** Module path (e.g., "./utils/math.ts") */
  modulePath: string;
  /** Line number (1-indexed) */
  line: number;
}

/**
 * Parsed q bind declaration.
 *
 * Example: `q bind clamped_opacity = clamp(hover_progress, 0, 1)`
 */
export interface VsBind {
  /** Binding name (e.g., "clamped_opacity") */
  bindName: string;
  /** Function name (e.g., "clamp") */
  functionName: string;
  /** Raw argument strings (e.g., ["hover_progress", "0", "1"]) */
  args: string[];
  /** Line number (1-indexed) */
  line: number;
}

/**
 * Parsed q trigger declaration.
 *
 * Example: `q trigger on_collision = bounds_overlap(rect_1, circle_1) -> notify(data)`
 */
export interface VsTrigger {
  /** Trigger name (e.g., "on_collision") */
  triggerName: string;
  /** Condition kind (e.g., "bounds_overlap") */
  conditionKind: string;
  /** Condition arguments (e.g., ["rect_1", "circle_1"]) */
  conditionArgs: string[];
  /** FFI function name (e.g., "notify") */
  functionName: string;
  /** FFI function arguments (e.g., ["data"]) */
  functionArgs: string[];
  /** Line number (1-indexed) */
  line: number;
}

/**
 * Result of parsing a .vs file.
 */
export interface VsParseResult {
  imports: VsImport[];
  binds: VsBind[];
  triggers: VsTrigger[];
  errors: VsParseError[];
}

/**
 * Parse error with location.
 */
export interface VsParseError {
  message: string;
  line: number;
}

// =============================================================================
// Regex Patterns
// =============================================================================

/**
 * Import statement pattern.
 *
 * Matches: `import { name1, name2 } from "path"`
 * Groups: 1=names, 2=path
 */
const IMPORT_PATTERN = /^\s*import\s*\{\s*([^}]+)\s*\}\s*from\s*["']([^"']+)["']\s*$/;

/**
 * Q bind declaration pattern.
 *
 * Matches: `q bind name = func(arg1, arg2)`
 * Groups: 1=bindName, 2=functionName, 3=args
 */
const BIND_PATTERN = /^\s*q\s+bind\s+(\w+)\s*=\s*(\w+)\s*\(\s*([^)]*)\s*\)\s*$/;

/**
 * Q trigger declaration pattern.
 *
 * Matches: `q trigger name = condition(args) -> func(args)`
 * Groups: 1=triggerName, 2=conditionKind, 3=conditionArgs, 4=functionName, 5=functionArgs
 */
const TRIGGER_PATTERN =
  /^\s*q\s+trigger\s+(\w+)\s*=\s*(\w+)\s*\(\s*([^)]*)\s*\)\s*->\s*(\w+)\s*\(\s*([^)]*)\s*\)\s*$/;

// =============================================================================
// Parser Implementation
// =============================================================================

/**
 * Parse a comma-separated argument list.
 *
 * Handles whitespace and empty args.
 */
function parseArgs(argsStr: string): string[] {
  if (!argsStr.trim()) {
    return [];
  }
  return argsStr.split(',').map((arg) => arg.trim()).filter(Boolean);
}

/**
 * Parse import names from the captured group.
 *
 * Handles: `{ name1, name2 as alias }` -> ["name1", "name2"]
 * Note: Aliases are ignored in Phase 1.
 */
function parseImportNames(namesStr: string): string[] {
  return namesStr
    .split(',')
    .map((name) => {
      // Handle `name as alias` -> take `name`
      const parts = name.trim().split(/\s+as\s+/);
      return parts[0].trim();
    })
    .filter(Boolean);
}

/**
 * Parse a single line and extract declaration if present.
 */
function parseLine(
  line: string,
  lineNumber: number,
  result: VsParseResult
): void {
  // Skip empty lines and comments
  const trimmed = line.trim();
  if (!trimmed || trimmed.startsWith('//') || trimmed.startsWith('/*')) {
    return;
  }

  // Try import pattern
  const importMatch = line.match(IMPORT_PATTERN);
  if (importMatch) {
    result.imports.push({
      names: parseImportNames(importMatch[1]),
      modulePath: importMatch[2],
      line: lineNumber,
    });
    return;
  }

  // Try q bind pattern
  const bindMatch = line.match(BIND_PATTERN);
  if (bindMatch) {
    result.binds.push({
      bindName: bindMatch[1],
      functionName: bindMatch[2],
      args: parseArgs(bindMatch[3]),
      line: lineNumber,
    });
    return;
  }

  // Try q trigger pattern
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
    return;
  }

  // Lines starting with `q ` but not matching patterns are errors
  if (trimmed.startsWith('q ')) {
    result.errors.push({
      message: `Invalid q declaration: ${trimmed}`,
      line: lineNumber,
    });
  }
}

/**
 * Parse a .vs file content and extract FFI-related declarations.
 *
 * @param content - The .vs file content
 * @returns Parsed imports, binds, triggers, and errors
 */
export function parseVsFile(content: string): VsParseResult {
  const result: VsParseResult = {
    imports: [],
    binds: [],
    triggers: [],
    errors: [],
  };

  const lines = content.split('\n');

  for (let i = 0; i < lines.length; i++) {
    parseLine(lines[i], i + 1, result);
  }

  return result;
}

/**
 * Validate that all function references in binds/triggers are imported.
 *
 * @param result - Parse result to validate
 * @returns Array of validation errors
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
