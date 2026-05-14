/**
 * FFI Manifest Generation for Vite Plugin
 *
 * Generates FfiManifest JSON matching the schema in vsc-wasm/src/ffi_bridge.rs.
 * The manifest maps .vs file declarations to ESM module exports.
 */

import type { VsParseResult, VsBind, VsTrigger } from './vs-parser.js';
import type { EsmAnalysisResult, ExportedFunction } from './esm-analyzer.js';

// =============================================================================
// Manifest Types (mirrors vsc-wasm/src/ffi_bridge.rs)
// =============================================================================

/**
 * FFI Manifest schema version.
 */
export const MANIFEST_VERSION = 1;

/**
 * FFI argument types.
 */
export type FfiArg =
  | { type: 'static'; value: unknown }
  | { type: 'q_ref'; name: string }
  | { type: 'entity_coord'; entity_id: number; component: string };

/**
 * FFI binding declaration.
 *
 * Maps a Q-dimension variable to a JS function call.
 */
export interface FfiBinding {
  /** Unique binding ID */
  ffi_id: number;
  /** Q-variable name (e.g., "clamped_opacity") */
  bind_name: string;
  /** Resolved module path */
  module_path: string;
  /** Exported function name */
  export_name: string;
  /** Function arguments */
  args: FfiArg[];
}

/**
 * FFI trigger declaration.
 *
 * Maps a condition to an FFI function call.
 */
export interface FfiTrigger {
  /** Unique trigger ID */
  trigger_id: number;
  /** FFI function ID to call when triggered */
  ffi_id: number;
  /** Resolved module path */
  module_path: string;
  /** Exported function name */
  export_name: string;
  /** Condition that triggers this FFI call */
  condition: ConditionKind;
  /** Function arguments */
  args: FfiArg[];
}

/**
 * Vector component for property references.
 */
export type VectorComponent = 'x' | 'y' | 'z' | 't';

/**
 * Direction of threshold crossing.
 */
export type CrossingDirection = 'rising' | 'falling';

/**
 * Condition kinds (mirrors vsc-core/src/types.rs).
 */
export type ConditionKind =
  | {
      kind: 'bounds_overlap';
      entity_a: number;
      entity_b: number;
    }
  | {
      kind: 'properties_equal';
      entity_a: number;
      component_a: VectorComponent;
      entity_b: number;
      component_b: VectorComponent;
    }
  | {
      kind: 'property_less_than';
      entity_a: number;
      component_a: VectorComponent;
      entity_b: number;
      component_b: VectorComponent;
    }
  | {
      kind: 'threshold_crossing';
      entity: number;
      component: VectorComponent;
      threshold: string; // Rational as "num/den" string
      direction: CrossingDirection;
    };

/**
 * Complete FFI manifest.
 */
export interface FfiManifest {
  /** Schema version */
  version: number;
  /** Map of entity names to EntityIds */
  entity_map: Record<string, number>;
  /** FFI bindings (Q-dimension) */
  bindings: FfiBinding[];
  /** FFI triggers (condition-based) */
  triggers: FfiTrigger[];
}

// =============================================================================
// Manifest Generation Context
// =============================================================================

/**
 * Module resolution result.
 */
export interface ResolvedModule {
  /** Original import path */
  originalPath: string;
  /** Resolved absolute path */
  resolvedPath: string;
  /** Analysis result of the module */
  analysis: EsmAnalysisResult;
}

/**
 * Context for manifest generation.
 */
export interface ManifestContext {
  /** Parsed .vs file result */
  vsParseResult: VsParseResult;
  /** Map of original import path to resolved module info */
  resolvedModules: Map<string, ResolvedModule>;
  /** Map of entity names to IDs (from VsBuildInfo) */
  entityMap: Map<string, number>;
}

/**
 * Error during manifest generation.
 */
export interface ManifestError {
  message: string;
  line?: number;
}

/**
 * Result of manifest generation.
 */
export interface ManifestResult {
  manifest: FfiManifest | null;
  errors: ManifestError[];
}

// =============================================================================
// Manifest Generation
// =============================================================================

/**
 * Generate an FFI manifest from parsed .vs declarations.
 *
 * @param context - Generation context with parsed data
 * @returns Generated manifest and any errors
 */
export function generateManifest(context: ManifestContext): ManifestResult {
  const errors: ManifestError[] = [];
  const bindings: FfiBinding[] = [];
  const triggers: FfiTrigger[] = [];

  // Build import name to module path mapping
  const importToModule = buildImportMapping(context.vsParseResult);

  // Process bindings
  let ffiId = 1;
  for (const bind of context.vsParseResult.binds) {
    const result = processBinding(bind, ffiId, importToModule, context);
    if (result.error) {
      errors.push(result.error);
    } else if (result.binding) {
      bindings.push(result.binding);
      ffiId++;
    }
  }

  // Process triggers
  let triggerId = 1;
  for (const trigger of context.vsParseResult.triggers) {
    const result = processTrigger(trigger, triggerId, ffiId, importToModule, context);
    if (result.error) {
      errors.push(result.error);
    } else if (result.trigger) {
      triggers.push(result.trigger);
      triggerId++;
      ffiId++;
    }
  }

  // Return null manifest if there are errors
  if (errors.length > 0) {
    return { manifest: null, errors };
  }

  const manifest: FfiManifest = {
    version: MANIFEST_VERSION,
    entity_map: Object.fromEntries(context.entityMap),
    bindings,
    triggers,
  };

  return { manifest, errors: [] };
}

// =============================================================================
// Internal Helpers
// =============================================================================

/**
 * Build a mapping from import names to their module paths.
 */
function buildImportMapping(parseResult: VsParseResult): Map<string, string> {
  const mapping = new Map<string, string>();
  for (const imp of parseResult.imports) {
    for (const name of imp.names) {
      mapping.set(name, imp.modulePath);
    }
  }
  return mapping;
}

interface BindingResult {
  binding?: FfiBinding;
  error?: ManifestError;
}

/**
 * Process a single bind declaration.
 */
function processBinding(
  bind: VsBind,
  ffiId: number,
  importToModule: Map<string, string>,
  context: ManifestContext
): BindingResult {
  const modulePath = importToModule.get(bind.functionName);
  if (!modulePath) {
    return {
      error: {
        message: `Function '${bind.functionName}' is not imported`,
        line: bind.line,
      },
    };
  }

  const resolved = context.resolvedModules.get(modulePath);
  if (!resolved) {
    return {
      error: {
        message: `Module '${modulePath}' could not be resolved`,
        line: bind.line,
      },
    };
  }

  // Verify the function is exported
  const isExported = resolved.analysis.exports.some(
    (exp: ExportedFunction) => exp.name === bind.functionName
  );
  if (!isExported) {
    return {
      error: {
        message: `Function '${bind.functionName}' is not exported from '${modulePath}'`,
        line: bind.line,
      },
    };
  }

  const args = parseBindArgs(bind.args, context.entityMap);

  return {
    binding: {
      ffi_id: ffiId,
      bind_name: bind.bindName,
      module_path: resolved.resolvedPath,
      export_name: bind.functionName,
      args,
    },
  };
}

interface TriggerResult {
  trigger?: FfiTrigger;
  error?: ManifestError;
}

/**
 * Process a single trigger declaration.
 */
function processTrigger(
  trigger: VsTrigger,
  triggerId: number,
  ffiId: number,
  importToModule: Map<string, string>,
  context: ManifestContext
): TriggerResult {
  // Build condition based on kind
  const conditionResult = buildCondition(
    trigger.conditionKind,
    trigger.conditionArgs,
    context.entityMap,
    trigger.line
  );

  if (conditionResult.error) {
    return { error: conditionResult.error };
  }

  // Validate function import
  const modulePath = importToModule.get(trigger.functionName);
  if (!modulePath) {
    return {
      error: {
        message: `Function '${trigger.functionName}' is not imported`,
        line: trigger.line,
      },
    };
  }

  const resolved = context.resolvedModules.get(modulePath);
  if (!resolved) {
    return {
      error: {
        message: `Module '${modulePath}' could not be resolved`,
        line: trigger.line,
      },
    };
  }

  const isExported = resolved.analysis.exports.some(
    (exp: ExportedFunction) => exp.name === trigger.functionName
  );
  if (!isExported) {
    return {
      error: {
        message: `Function '${trigger.functionName}' is not exported from '${modulePath}'`,
        line: trigger.line,
      },
    };
  }

  const args = parseBindArgs(trigger.functionArgs, context.entityMap);

  return {
    trigger: {
      trigger_id: triggerId,
      ffi_id: ffiId,
      module_path: resolved.resolvedPath,
      export_name: trigger.functionName,
      condition: conditionResult.condition!,
      args,
    },
  };
}

interface ConditionResult {
  condition?: ConditionKind;
  error?: ManifestError;
}

/**
 * Build a ConditionKind from parsed trigger arguments.
 */
function buildCondition(
  kind: string,
  args: string[],
  entityMap: Map<string, number>,
  line: number
): ConditionResult {
  switch (kind) {
    case 'bounds_overlap': {
      // bounds_overlap(entity_a, entity_b)
      if (args.length !== 2) {
        return {
          error: { message: `bounds_overlap requires 2 arguments, got ${args.length}`, line },
        };
      }
      const entityA = resolveEntityId(args[0], entityMap);
      const entityB = resolveEntityId(args[1], entityMap);
      if (entityA === null) {
        return { error: { message: `Unknown entity '${args[0]}' in condition`, line } };
      }
      if (entityB === null) {
        return { error: { message: `Unknown entity '${args[1]}' in condition`, line } };
      }
      return {
        condition: { kind: 'bounds_overlap', entity_a: entityA, entity_b: entityB },
      };
    }

    case 'properties_equal': {
      // properties_equal(entity_a.component_a, entity_b.component_b)
      if (args.length !== 2) {
        return {
          error: { message: `properties_equal requires 2 arguments, got ${args.length}`, line },
        };
      }
      const propA = parsePropertyRef(args[0], entityMap);
      const propB = parsePropertyRef(args[1], entityMap);
      if (propA.error) return { error: { message: propA.error, line } };
      if (propB.error) return { error: { message: propB.error, line } };
      return {
        condition: {
          kind: 'properties_equal',
          entity_a: propA.entityId!,
          component_a: propA.component!,
          entity_b: propB.entityId!,
          component_b: propB.component!,
        },
      };
    }

    case 'property_less_than': {
      // property_less_than(entity_a.component_a, entity_b.component_b)
      if (args.length !== 2) {
        return {
          error: { message: `property_less_than requires 2 arguments, got ${args.length}`, line },
        };
      }
      const propA = parsePropertyRef(args[0], entityMap);
      const propB = parsePropertyRef(args[1], entityMap);
      if (propA.error) return { error: { message: propA.error, line } };
      if (propB.error) return { error: { message: propB.error, line } };
      return {
        condition: {
          kind: 'property_less_than',
          entity_a: propA.entityId!,
          component_a: propA.component!,
          entity_b: propB.entityId!,
          component_b: propB.component!,
        },
      };
    }

    case 'threshold_crossing': {
      // threshold_crossing(entity.component, threshold, direction)
      if (args.length !== 3) {
        return {
          error: { message: `threshold_crossing requires 3 arguments, got ${args.length}`, line },
        };
      }
      const prop = parsePropertyRef(args[0], entityMap);
      if (prop.error) return { error: { message: prop.error, line } };

      const threshold = parseThreshold(args[1]);
      if (threshold === null) {
        return { error: { message: `Invalid threshold '${args[1]}'`, line } };
      }

      const direction = parseDirection(args[2]);
      if (direction === null) {
        return {
          error: { message: `Invalid direction '${args[2]}', expected 'rising' or 'falling'`, line },
        };
      }

      return {
        condition: {
          kind: 'threshold_crossing',
          entity: prop.entityId!,
          component: prop.component!,
          threshold,
          direction,
        },
      };
    }

    default:
      return { error: { message: `Unknown condition kind '${kind}'`, line } };
  }
}

interface PropertyRefResult {
  entityId?: number;
  component?: VectorComponent;
  error?: string;
}

/**
 * Parse a property reference like "entity.x" or "entity.y".
 */
function parsePropertyRef(ref: string, entityMap: Map<string, number>): PropertyRefResult {
  const match = ref.match(/^(\w+)\.(x|y|z|t)$/);
  if (!match) {
    return { error: `Invalid property reference '${ref}', expected 'entity.component'` };
  }
  const entityName = match[1];
  const component = match[2] as VectorComponent;
  const entityId = entityMap.get(entityName);
  if (entityId === undefined) {
    return { error: `Unknown entity '${entityName}'` };
  }
  return { entityId, component };
}

/**
 * Parse a threshold value as a Rational string "num/den" or integer.
 */
function parseThreshold(value: string): string | null {
  // Already a rational string?
  if (/^-?\d+\/-?\d+$/.test(value)) {
    return value;
  }
  // Integer or decimal?
  const num = parseFloat(value);
  if (isNaN(num)) {
    return null;
  }
  // Convert to rational string (simple integer case)
  if (Number.isInteger(num)) {
    return `${num}/1`;
  }
  // For decimals, use a simple conversion (limited precision)
  const precision = 1000000;
  const numerator = Math.round(num * precision);
  return `${numerator}/${precision}`;
}

/**
 * Parse a crossing direction.
 */
function parseDirection(value: string): CrossingDirection | null {
  if (value === 'rising' || value === 'falling') {
    return value;
  }
  return null;
}

/**
 * Parse bind/trigger arguments into FfiArg array.
 *
 * Simple heuristics:
 * - Numeric literals -> Static
 * - Entity names (in entityMap) with .x/.y suffix -> EntityCoord
 * - Everything else -> QRef
 */
function parseBindArgs(args: string[], entityMap: Map<string, number>): FfiArg[] {
  return args.map((arg) => {
    // Check for numeric literal
    const num = parseFloat(arg);
    if (!isNaN(num)) {
      return { type: 'static', value: num };
    }

    // Check for entity coordinate (entity_name.component)
    const coordMatch = arg.match(/^(\w+)\.(x|y|width|height)$/);
    if (coordMatch) {
      const entityName = coordMatch[1];
      const entityId = entityMap.get(entityName);
      if (entityId !== undefined) {
        return {
          type: 'entity_coord',
          entity_id: entityId,
          component: coordMatch[2],
        };
      }
    }

    // Default to Q-ref
    return { type: 'q_ref', name: arg };
  });
}

/**
 * Resolve an entity name to its ID.
 */
function resolveEntityId(
  name: string,
  entityMap: Map<string, number>
): number | null {
  return entityMap.get(name) ?? null;
}

// =============================================================================
// Manifest Serialization
// =============================================================================

/**
 * Serialize manifest to JSON string.
 *
 * @param manifest - Manifest to serialize
 * @returns JSON string
 */
export function serializeManifest(manifest: FfiManifest): string {
  return JSON.stringify(manifest, null, 2);
}
