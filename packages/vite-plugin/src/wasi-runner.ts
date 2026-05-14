/**
 * WASI Runner for ViewScript Compiler
 *
 * Invokes the Rust compiler (vsc) via WASI to:
 * 1. Parse .vs source files
 * 2. Run the constraint solver
 * 3. Generate standalone JavaScript code
 *
 * ## Phase 1 Architecture
 *
 * ```
 * .vs source
 *     │
 *     ▼
 * ┌─────────────────────────────────────┐
 * │ WASI Runtime (Node.js)              │
 * │                                     │
 * │  vsc.wasm (vsc-cli WASI build)      │
 * │  ├── parse .vs                      │
 * │  ├── build VsBuildInfo              │
 * │  ├── solve constraints              │
 * │  └── js_codegen → JS output         │
 * └─────────────────────────────────────┘
 *     │
 *     ▼
 * Standalone JavaScript (uses @viewscript/gpu-runtime)
 * ```
 *
 * ## Build Requirement
 *
 * The WASI binary must be built with:
 * ```sh
 * cargo build --release --target wasm32-wasip1 -p vsc-cli
 * ```
 */

import { WASI } from 'node:wasi';
import { readFile, writeFile, unlink, mkdtemp, rmdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { openSync, closeSync, readFileSync, writeFileSync } from 'node:fs';

// =============================================================================
// Types
// =============================================================================

/** Compilation result from WASI runner */
export interface CompileResult {
  success: boolean;
  /** Generated JavaScript code (if successful) */
  code?: string;
  /** Error message (if failed) */
  error?: string;
  /** Build metadata */
  metadata?: {
    entityCount: number;
    constraintCount: number;
    meshCount: number;
  };
}

/** VsBuildInfo JSON structure (matches Rust vsc_core::buildinfo::VsBuildInfo) */
export interface VsBuildInfoInput {
  version: number;
  operations: Array<{
    op: string;
    [key: string]: unknown;
  }>;
  optimization_runs?: Array<unknown>;
  [key: string]: unknown;
}

// =============================================================================
// WASI Binary Discovery
// =============================================================================

/**
 * Locate the WASI binary (vsc.wasm)
 *
 * Search order:
 * 1. VS_WASI_BINARY env var
 * 2. Workspace target: ../../target/wasm32-wasip1/release/vsc.wasm
 * 3. Package-local: ../wasm/vsc.wasm
 */
async function findWasiBinary(): Promise<string> {
  // 1. Environment variable override
  if (process.env.VS_WASI_BINARY) {
    if (existsSync(process.env.VS_WASI_BINARY)) {
      return process.env.VS_WASI_BINARY;
    }
    throw new Error(`VS_WASI_BINARY not found: ${process.env.VS_WASI_BINARY}`);
  }

  // Get package directory
  const __dirname = dirname(fileURLToPath(import.meta.url));

  // 2. Workspace target (development)
  const workspaceTarget = join(__dirname, '..', '..', '..', 'target', 'wasm32-wasip1', 'release', 'vsc.wasm');
  if (existsSync(workspaceTarget)) {
    return workspaceTarget;
  }

  // 3. Package-local (published package)
  const packageLocal = join(__dirname, '..', 'wasm', 'vsc.wasm');
  if (existsSync(packageLocal)) {
    return packageLocal;
  }

  throw new Error(
    'WASI binary not found. Build with: cargo build --release --target wasm32-wasip1 -p vsc-cli'
  );
}

// =============================================================================
// Compile Function
// =============================================================================

/**
 * Compile a .vs file to standalone JavaScript
 *
 * @param vsSource - ViewScript source code
 * @param buildInfo - Optional pre-parsed VsBuildInfo
 * @returns Compilation result with generated code or error
 */
export async function compileVsToJs(
  vsSource: string,
  buildInfo?: VsBuildInfoInput
): Promise<CompileResult> {
  let tempDir: string | null = null;

  try {
    const wasmPath = await findWasiBinary();
    const wasmBinary = await readFile(wasmPath);

    // Create temporary directory for WASI file I/O
    tempDir = await mkdtemp(join(tmpdir(), 'vsc-'));
    const stdinPath = join(tempDir, 'stdin.json');
    const stdoutPath = join(tempDir, 'stdout.js');
    const stderrPath = join(tempDir, 'stderr.txt');

    // Prepare input data and write to stdin file
    const inputData = JSON.stringify(buildInfo || { version: 1, operations: [], optimization_runs: [] });
    writeFileSync(stdinPath, inputData);

    // Create output files
    writeFileSync(stdoutPath, '');
    writeFileSync(stderrPath, '');

    // Open file descriptors
    const stdinFd = openSync(stdinPath, 'r');
    const stdoutFd = openSync(stdoutPath, 'w');
    const stderrFd = openSync(stderrPath, 'w');

    try {
      // Create WASI instance with file-based I/O
      const wasi = new WASI({
        version: 'preview1',
        args: ['vsc', 'compile-js', '--stdin'],
        env: {},
        stdin: stdinFd,
        stdout: stdoutFd,
        stderr: stderrFd,
        preopens: {
          '/tmp': tempDir,
        },
      });

      // Compile and instantiate WASM
      const wasmModule = await WebAssembly.compile(wasmBinary);
      const instance = await WebAssembly.instantiate(
        wasmModule,
        wasi.getImportObject() as WebAssembly.Imports
      );

      // Run the WASM
      let exitCode = 0;
      try {
        wasi.start(instance);
      } catch (exitError) {
        // WASI exit is normal for CLI tools
        exitCode = (exitError as { code?: number }).code ?? 1;
      }

      // Close file descriptors
      closeSync(stdinFd);
      closeSync(stdoutFd);
      closeSync(stderrFd);

      // Read outputs
      const generatedCode = readFileSync(stdoutPath, 'utf-8');
      const stderrContent = readFileSync(stderrPath, 'utf-8');

      if (exitCode !== 0) {
        return {
          success: false,
          error: stderrContent || `WASI exited with code ${exitCode}`,
        };
      }

      // Parse metadata from stderr if present (JSON on last line)
      let metadata = {
        entityCount: 0,
        constraintCount: buildInfo?.operations?.length || 0,
        meshCount: 0,
      };

      const stderrLines = stderrContent.trim().split('\n');
      const lastLine = stderrLines[stderrLines.length - 1];
      if (lastLine?.startsWith('{')) {
        try {
          const parsed = JSON.parse(lastLine);
          if (parsed.entity_count !== undefined) {
            metadata = {
              entityCount: parsed.entity_count,
              constraintCount: parsed.constraint_count,
              meshCount: parsed.mesh_count || 0,
            };
          }
        } catch {
          // Ignore JSON parse errors in stderr
        }
      }

      return {
        success: true,
        code: generatedCode,
        metadata,
      };
    } catch (err) {
      // Ensure file descriptors are closed on error
      try { closeSync(stdinFd); } catch { /* ignore */ }
      try { closeSync(stdoutFd); } catch { /* ignore */ }
      try { closeSync(stderrFd); } catch { /* ignore */ }
      throw err;
    }
  } catch (err) {
    return {
      success: false,
      error: err instanceof Error ? err.message : String(err),
    };
  } finally {
    // Cleanup temp directory
    if (tempDir) {
      try {
        await unlink(join(tempDir, 'stdin.json')).catch(() => {});
        await unlink(join(tempDir, 'stdout.js')).catch(() => {});
        await unlink(join(tempDir, 'stderr.txt')).catch(() => {});
        await rmdir(tempDir).catch(() => {});
      } catch {
        // Ignore cleanup errors
      }
    }
  }
}

/**
 * Check if WASI compilation is available
 *
 * Returns true if the WASI binary exists and Node.js WASI is available.
 */
export async function isWasiAvailable(): Promise<boolean> {
  try {
    await findWasiBinary();
    return true;
  } catch {
    return false;
  }
}

/**
 * Generate placeholder JavaScript for Phase 1
 *
 * Until full WASI integration is complete, this generates a minimal
 * runtime that loads the VsBuildInfo and renders static meshes.
 */
export function generatePlaceholderJs(buildInfo: VsBuildInfoInput, id: string): string {
  const operationCount = buildInfo.operations?.length || 0;

  return `// ViewScript compiled output: ${id}
// Generated by @viewscript/vite-plugin (placeholder - WASI unavailable)
// Operations: ${operationCount}

import { initGpu, createRuntime } from '@viewscript/gpu-runtime';

// Placeholder entity IDs (WASI compilation required for actual values)
export const ENTITY_IDS = [];

// Initialize runtime when DOM is ready
export async function init(canvas) {
  const gpu = await initGpu(canvas);
  const runtime = createRuntime(gpu);

  // Render loop
  function animate() {
    runtime.render({ r: 1, g: 1, b: 1, a: 1 });
    requestAnimationFrame(animate);
  }
  animate();

  return runtime;
}

export default { init, ENTITY_IDS };
`;
}
