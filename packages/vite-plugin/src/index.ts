/**
 * ViewScript Vite Plugin
 *
 * Transforms .vs files and generates FFI manifests at build time.
 *
 * Hooks:
 * - configResolved: Capture Vite config
 * - transform: Parse .vs files, resolve imports, analyze ESM modules
 * - generateBundle: Emit ffi-manifest.json
 */

import type { Plugin, ResolvedConfig } from 'vite';
import { parseVsFile, validateImports, type VsParseResult } from './vs-parser.js';
import { analyzeEsmExports, type EsmAnalysisResult } from './esm-analyzer.js';
import {
  generateManifest,
  serializeManifest,
  type ManifestContext,
  type ResolvedModule,
  type FfiManifest,
} from './manifest.js';

// =============================================================================
// Plugin Options
// =============================================================================

/**
 * Plugin configuration options.
 */
export interface ViewScriptPluginOptions {
  /**
   * Entity map providing name -> EntityId mapping.
   * In Phase 1, this is provided statically.
   * In later phases, this will be derived from VsBuildInfo.
   */
  entityMap?: Record<string, number>;

  /**
   * Output filename for the FFI manifest.
   * @default 'ffi-manifest.json'
   */
  manifestFilename?: string;
}

// =============================================================================
// Plugin State
// =============================================================================

interface VsFileState {
  parseResult: VsParseResult;
  resolvedModules: Map<string, ResolvedModule>;
}

// =============================================================================
// Plugin Implementation
// =============================================================================

/**
 * ViewScript Vite Plugin.
 *
 * @param options - Plugin options
 * @returns Vite plugin
 */
export function viewScriptPlugin(options: ViewScriptPluginOptions = {}): Plugin {
  const {
    entityMap = {},
    manifestFilename = 'ffi-manifest.json',
  } = options;

  let config: ResolvedConfig;
  const vsFiles = new Map<string, VsFileState>();
  const moduleCache = new Map<string, EsmAnalysisResult>();

  return {
    name: 'viewscript',

    // =========================================================================
    // configResolved
    // =========================================================================

    configResolved(resolvedConfig) {
      config = resolvedConfig;
    },

    // =========================================================================
    // transform
    // =========================================================================

    async transform(code, id) {
      // Only process .vs files
      if (!id.endsWith('.vs')) {
        return null;
      }

      // Parse the .vs file
      const parseResult = parseVsFile(code);

      // Report parse errors
      for (const error of parseResult.errors) {
        this.error({
          message: error.message,
          id,
          loc: { line: error.line, column: 0 },
        });
      }

      // Validate imports
      const importErrors = validateImports(parseResult);
      for (const error of importErrors) {
        this.error({
          message: error.message,
          id,
          loc: { line: error.line, column: 0 },
        });
      }

      // Resolve and analyze imported modules
      const resolvedModules = new Map<string, ResolvedModule>();

      for (const imp of parseResult.imports) {
        const resolved = await this.resolve(imp.modulePath, id);
        if (!resolved) {
          this.error({
            message: `Cannot resolve module '${imp.modulePath}'`,
            id,
            loc: { line: imp.line, column: 0 },
          });
          continue;
        }

        // Check module cache
        let analysis = moduleCache.get(resolved.id);
        if (!analysis) {
          // Load and parse the module
          const moduleCode = await this.load({ id: resolved.id });
          if (typeof moduleCode === 'string') {
            // moduleCode might be an object with code property in some cases
            try {
              const ast = this.parse(moduleCode);
              analysis = analyzeEsmExports(ast);
              moduleCache.set(resolved.id, analysis);
            } catch (parseError) {
              this.warn({
                message: `Failed to parse module '${resolved.id}': ${parseError}`,
                id,
              });
              analysis = { exports: [], hasDefaultExport: false };
            }
          } else if (moduleCode && typeof moduleCode === 'object' && 'code' in moduleCode) {
            try {
              const ast = this.parse((moduleCode as { code: string }).code);
              analysis = analyzeEsmExports(ast);
              moduleCache.set(resolved.id, analysis);
            } catch (parseError) {
              this.warn({
                message: `Failed to parse module '${resolved.id}': ${parseError}`,
                id,
              });
              analysis = { exports: [], hasDefaultExport: false };
            }
          } else {
            analysis = { exports: [], hasDefaultExport: false };
          }
        }

        resolvedModules.set(imp.modulePath, {
          originalPath: imp.modulePath,
          resolvedPath: resolved.id,
          analysis,
        });
      }

      // Store state for generateBundle
      vsFiles.set(id, { parseResult, resolvedModules });

      // Return transformed code (placeholder for Phase 1)
      // In later phases, this would emit runtime bindings
      return {
        code: `// ViewScript compiled from ${id}\nexport default {};`,
        map: null,
      };
    },

    // =========================================================================
    // generateBundle
    // =========================================================================

    generateBundle() {
      // Aggregate all .vs file declarations
      const aggregatedBinds: VsParseResult['binds'] = [];
      const aggregatedTriggers: VsParseResult['triggers'] = [];
      const aggregatedImports: VsParseResult['imports'] = [];
      const aggregatedModules = new Map<string, ResolvedModule>();

      for (const [, state] of vsFiles) {
        aggregatedBinds.push(...state.parseResult.binds);
        aggregatedTriggers.push(...state.parseResult.triggers);
        aggregatedImports.push(...state.parseResult.imports);

        for (const [path, mod] of state.resolvedModules) {
          aggregatedModules.set(path, mod);
        }
      }

      // Skip manifest generation if no FFI declarations
      if (aggregatedBinds.length === 0 && aggregatedTriggers.length === 0) {
        return;
      }

      // Build manifest context
      const context: ManifestContext = {
        vsParseResult: {
          imports: aggregatedImports,
          binds: aggregatedBinds,
          triggers: aggregatedTriggers,
          errors: [],
        },
        resolvedModules: aggregatedModules,
        entityMap: new Map(Object.entries(entityMap)),
      };

      // Generate manifest
      const result = generateManifest(context);

      if (result.errors.length > 0) {
        for (const error of result.errors) {
          this.error({
            message: `Manifest generation error: ${error.message}`,
          });
        }
        return;
      }

      if (result.manifest) {
        // Emit the manifest file
        this.emitFile({
          type: 'asset',
          fileName: manifestFilename,
          source: serializeManifest(result.manifest),
        });

        // Log in dev mode
        if (config.command === 'serve') {
          config.logger.info(
            `[viewscript] Generated ${manifestFilename} with ${result.manifest.bindings.length} bindings and ${result.manifest.triggers.length} triggers`
          );
        }
      }
    },
  };
}

// =============================================================================
// Re-exports
// =============================================================================

export { parseVsFile, validateImports } from './vs-parser.js';
export type { VsParseResult, VsImport, VsBind, VsTrigger, VsParseError } from './vs-parser.js';

export { analyzeEsmExports, hasExport, getExportedNames } from './esm-analyzer.js';
export type { EsmAnalysisResult, ExportedFunction } from './esm-analyzer.js';

export { generateManifest, serializeManifest, MANIFEST_VERSION } from './manifest.js';
export type {
  FfiManifest,
  FfiBinding,
  FfiTrigger,
  FfiArg,
  ConditionKind,
  ManifestContext,
  ManifestResult,
  ManifestError,
  ResolvedModule,
} from './manifest.js';

// Default export for convenient usage
export default viewScriptPlugin;
