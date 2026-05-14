/**
 * Project Scaffolding Logic
 *
 * Generates project files from templates with appropriate customization.
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const FALLBACK_VERSION = '0.1.0';

/**
 * Resolve the latest version of a package from npm registry.
 * Falls back to a default version if the fetch fails (e.g., offline).
 */
async function resolveLatestVersion(pkg: string): Promise<string> {
  try {
    const res = await fetch(`https://registry.npmjs.org/${pkg}/latest`);
    if (!res.ok) return FALLBACK_VERSION;
    const data = (await res.json()) as { version?: string };
    return data.version ?? FALLBACK_VERSION;
  } catch {
    return FALLBACK_VERSION;
  }
}

export interface ScaffoldOptions {
  projectName: string;
  targetDir: string;
  language: 'ts' | 'js';
  includeFfiSample: boolean;
}

/**
 * Scaffold a new ViewScript project.
 */
export async function scaffold(options: ScaffoldOptions): Promise<void> {
  const { projectName, targetDir, language, includeFfiSample } = options;

  // Resolve latest package versions from npm (parallel fetch)
  const [gpuRuntimeVersion, pluginVersion] = await Promise.all([
    resolveLatestVersion('@viewscript/gpu-runtime'),
    resolveLatestVersion('@viewscript/vite-plugin'),
  ]);

  // Ensure target directory exists
  fs.mkdirSync(targetDir, { recursive: true });

  // Template directories
  const templatesDir = path.resolve(__dirname, '..', 'templates');
  const baseDir = path.join(templatesDir, 'base');
  const langDir = path.join(templatesDir, language);

  // Copy base files
  copyDir(baseDir, targetDir);

  // Copy language-specific files
  copyDir(langDir, targetDir);

  // Generate package.json with resolved versions
  const packageJson = generatePackageJson(projectName, language, gpuRuntimeVersion, pluginVersion);
  fs.writeFileSync(
    path.join(targetDir, 'package.json'),
    JSON.stringify(packageJson, null, 2) + '\n'
  );

  // Generate FFI sample if requested
  if (includeFfiSample) {
    const ext = language === 'ts' ? 'ts' : 'js';
    const utilsDir = path.join(targetDir, 'src', 'utils');
    fs.mkdirSync(utilsDir, { recursive: true });

    const mathContent = generateMathUtils(language);
    fs.writeFileSync(path.join(utilsDir, `math.${ext}`), mathContent);

    // Update main.vs to include FFI import
    const mainVsPath = path.join(targetDir, 'src', 'main.vs');
    const mainVsContent = fs.readFileSync(mainVsPath, 'utf-8');
    const updatedMainVs = addFfiToMainVs(mainVsContent, language);
    fs.writeFileSync(mainVsPath, updatedMainVs);
  }

  // Rename _gitignore to .gitignore
  const gitignoreSrc = path.join(targetDir, '_gitignore');
  const gitignoreDest = path.join(targetDir, '.gitignore');
  if (fs.existsSync(gitignoreSrc)) {
    fs.renameSync(gitignoreSrc, gitignoreDest);
  }

  // Update index.html script src based on language
  const indexHtmlPath = path.join(targetDir, 'index.html');
  let indexHtml = fs.readFileSync(indexHtmlPath, 'utf-8');
  if (language === 'js') {
    indexHtml = indexHtml.replace('/src/app.ts', '/src/app.js');
  }
  fs.writeFileSync(indexHtmlPath, indexHtml);
}

/**
 * Recursively copy a directory.
 */
function copyDir(src: string, dest: string): void {
  if (!fs.existsSync(src)) return;

  fs.mkdirSync(dest, { recursive: true });

  for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);

    if (entry.isDirectory()) {
      copyDir(srcPath, destPath);
    } else {
      fs.copyFileSync(srcPath, destPath);
    }
  }
}

/**
 * Generate package.json content.
 */
function generatePackageJson(
  projectName: string,
  language: 'ts' | 'js',
  gpuRuntimeVersion: string,
  pluginVersion: string
): Record<string, unknown> {
  const base = {
    name: projectName,
    version: '0.0.1',
    private: true,
    type: 'module',
    scripts: {
      dev: 'vite',
      build: 'vite build',
      preview: 'vite preview',
    },
    dependencies: {
      '@viewscript/gpu-runtime': `^${gpuRuntimeVersion}`,
    },
    devDependencies: {
      '@viewscript/vite-plugin': `^${pluginVersion}`,
      vite: '^5.4.0',
    },
  };

  if (language === 'ts') {
    return {
      ...base,
      devDependencies: {
        ...base.devDependencies,
        typescript: '^5.4.0',
      },
    };
  }

  return base;
}

/**
 * Generate math utilities file content.
 */
function generateMathUtils(language: 'ts' | 'js'): string {
  if (language === 'ts') {
    return `/**
 * Math Utilities for ViewScript FFI
 *
 * These functions can be imported in .vs files and bound to Q-dimension variables.
 */

/**
 * Clamp a value between min and max.
 */
export function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/**
 * Linear interpolation between two values.
 */
export function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * clamp(t, 0, 1);
}

/**
 * Smooth step interpolation (ease in-out).
 */
export function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = clamp((x - edge0) / (edge1 - edge0), 0, 1);
  return t * t * (3 - 2 * t);
}
`;
  }

  return `/**
 * Math Utilities for ViewScript FFI
 *
 * These functions can be imported in .vs files and bound to Q-dimension variables.
 */

/**
 * Clamp a value between min and max.
 * @param {number} value
 * @param {number} min
 * @param {number} max
 * @returns {number}
 */
export function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

/**
 * Linear interpolation between two values.
 * @param {number} a
 * @param {number} b
 * @param {number} t
 * @returns {number}
 */
export function lerp(a, b, t) {
  return a + (b - a) * clamp(t, 0, 1);
}

/**
 * Smooth step interpolation (ease in-out).
 * @param {number} edge0
 * @param {number} edge1
 * @param {number} x
 * @returns {number}
 */
export function smoothstep(edge0, edge1, x) {
  const t = clamp((x - edge0) / (edge1 - edge0), 0, 1);
  return t * t * (3 - 2 * t);
}
`;
}

/**
 * Add FFI import and bind to main.vs.
 */
function addFfiToMainVs(content: string, language: 'ts' | 'js'): string {
  const ext = language === 'ts' ? 'ts' : 'js';
  const ffiImport = `import { clamp } from "./utils/math.${ext}"

`;

  // Use Q-variable from pointer input (defined in app.ts QSnapshot)
  const ffiBind = `
// FFI binding example: clamp pointer X to canvas bounds
// 'input.pointer.x' is a Q-variable set by the runtime in app.ts
q bind pointer_x_clamped = clamp(input.pointer.x, 0, 800)
`;

  // Insert import at the beginning, after any existing imports
  const lines = content.split('\n');
  let insertIndex = 0;

  // Find the end of existing imports
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].startsWith('import ')) {
      insertIndex = i + 1;
    }
  }

  // Insert FFI import
  lines.splice(insertIndex, 0, ffiImport);

  // Append FFI bind at the end
  return lines.join('\n') + ffiBind;
}
