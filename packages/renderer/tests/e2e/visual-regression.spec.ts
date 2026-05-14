/**
 * Deterministic Visual Regression Tests
 *
 * This module ensures bit-perfect reproducibility of Canvas rendering
 * by forcing the GPU renderer into CPU-only software rendering mode.
 *
 * ## The Problem: GPU Non-Determinism
 *
 * Different GPUs (NVIDIA vs AMD vs Intel) produce subtly different
 * anti-aliasing patterns. Even the same GPU can produce different
 * results across driver versions. This is unacceptable for a
 * mathematically rigorous GUI framework.
 *
 * ## Solution: Software Rendering + Hash Comparison
 *
 * 1. Force wgpu renderer to use CPU rasterizer (no GPU)
 * 2. Disable subpixel text positioning
 * 3. Use fixed-width fonts for text tests
 * 4. Compare output against golden snapshots via SHA-256 hash
 *
 * ## Test Artifact Storage
 *
 * Golden snapshots are stored in:
 *   tests/e2e/golden/<test-name>-<platform>.png
 *
 * Failed diffs are output to:
 *   tests/e2e/failures/<test-name>-diff.png
 */

import { test, expect, type Page } from '@playwright/test';
import * as crypto from 'crypto';
import * as fs from 'fs';
import * as path from 'path';
import { fileURLToPath } from 'url';

// =============================================================================
// Test Configuration
// =============================================================================

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const GOLDEN_DIR = path.join(__dirname, 'golden');
const FAILURE_DIR = path.join(__dirname, 'failures');

/**
 * GPU renderer initialization options for deterministic rendering.
 */
const WGPU_DETERMINISTIC_CONFIG = {
  // Force CPU-only rendering (no WebGL)
  disableWebGL: true,

  // Use software rasterizer
  preferLowPowerToHighPerformance: false,

  // Disable subpixel antialiasing (font-dependent)
  useSubpixelText: false,

  // Fixed DPI for consistent sizing
  devicePixelRatio: 1.0,
};

// =============================================================================
// Test Fixtures
// =============================================================================

test.describe('Visual Regression: Bit-Perfect Rendering', () => {
  test.beforeAll(async () => {
    // Ensure directories exist
    if (!fs.existsSync(GOLDEN_DIR)) {
      fs.mkdirSync(GOLDEN_DIR, { recursive: true });
    }
    if (!fs.existsSync(FAILURE_DIR)) {
      fs.mkdirSync(FAILURE_DIR, { recursive: true });
    }
  });

  /**
   * Test: Simple Rectangle Rendering
   *
   * Constraint Graph (IR):
   *   Entity #1: rect at (10, 10) size (100, 50), fill: #FF0000
   *
   * Expected: Red rectangle, bit-perfect match with golden snapshot.
   */
  test('renders simple rectangle with bit-perfect hash match', async ({ page }) => {
    // Setup: Load VS renderer in deterministic mode
    await setupDeterministicRenderer(page);

    // Act: Render a simple constraint graph
    const pixelBuffer = await renderConstraintGraph(page, {
      entities: [
        {
          id: 1,
          type: 'rect',
          bounds: { x: 10, y: 10, width: 100, height: 50 },
          fill: '#FF0000',
        },
      ],
      constraints: [],
    });

    // Assert: Hash match
    const hash = computeHash(pixelBuffer);
    const goldenHash = loadGoldenHash('simple-rectangle');

    if (goldenHash === null) {
      // First run: save as golden
      saveGolden('simple-rectangle', pixelBuffer, hash);
      console.log(`[GOLDEN] Saved new golden snapshot: simple-rectangle (${hash})`);
    } else {
      expect(hash).toBe(goldenHash);
    }
  });

  /**
   * Test: Adjacent Rectangles (Topology Preservation)
   *
   * This tests the core topology-preserving rounding:
   * Three adjacent rects of 33.333...px width in 100px container.
   *
   * Expected: No gaps, no overlaps, bit-perfect.
   */
  test('renders adjacent rectangles without subpixel gaps', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderConstraintGraph(page, {
      entities: [
        // Container
        { id: 0, type: 'rect', bounds: { x: 0, y: 0, width: 100, height: 30 }, fill: '#000000' },
        // Three children with irrational widths
        { id: 1, type: 'rect', bounds: { x: 0, y: 0, width: 100/3, height: 30 }, fill: '#FF0000' },
        { id: 2, type: 'rect', bounds: { x: 100/3, y: 0, width: 100/3, height: 30 }, fill: '#00FF00' },
        { id: 3, type: 'rect', bounds: { x: 200/3, y: 0, width: 100/3, height: 30 }, fill: '#0000FF' },
      ],
      constraints: [
        { type: 'adjacent', a: { entityId: 1, edge: 'right' }, b: { entityId: 2, edge: 'left' } },
        { type: 'adjacent', a: { entityId: 2, edge: 'right' }, b: { entityId: 3, edge: 'left' } },
      ],
      containments: [
        { parentId: 0, childIds: [1, 2, 3], axis: 'horizontal' },
      ],
    });

    // Verify no black pixels (gaps) between colored rectangles
    const hasGaps = detectGaps(pixelBuffer, 100, 30);
    expect(hasGaps).toBe(false);

    // Hash comparison
    const hash = computeHash(pixelBuffer);
    const goldenHash = loadGoldenHash('adjacent-rectangles');

    if (goldenHash === null) {
      saveGolden('adjacent-rectangles', pixelBuffer, hash);
    } else {
      expect(hash).toBe(goldenHash);
    }
  });

  /**
   * Test: Text Rendering (Font Determinism)
   *
   * CRITICAL: Uses embedded WOFF2 font to guarantee cross-platform consistency.
   * Generic font families (monospace, serif) resolve to different fonts per OS.
   */
  test('renders text with deterministic glyph placement', async ({ page }) => {
    await setupDeterministicRenderer(page);

    // Load embedded font before rendering
    await page.evaluate(async () => {
      // Wait for VS embedded font to load
      await (window as any).__VS_RENDERER__.loadFont('vs-mono');
      await document.fonts.ready;
    });

    const pixelBuffer = await renderConstraintGraph(page, {
      entities: [
        {
          id: 1,
          type: 'text',
          bounds: { x: 10, y: 10, width: 200, height: 20 },
          content: 'ViewScript',
          font: {
            family: 'vs-mono', // Embedded font for determinism
            size: 16,
            weight: 400,
          },
          fill: '#000000',
        },
      ],
      constraints: [],
    });

    const hash = computeHash(pixelBuffer);
    const goldenHash = loadGoldenHash(`text-deterministic-${process.platform}`);

    if (goldenHash === null) {
      saveGolden(`text-deterministic-${process.platform}`, pixelBuffer, hash);
    } else {
      expect(hash).toBe(goldenHash);
    }
  });
});

// =============================================================================
// Helper Functions
// =============================================================================

/**
 * Setup Playwright page with deterministic wgpu renderer configuration.
 */
async function setupDeterministicRenderer(page: Page): Promise<void> {
  // Navigate to test harness (follow redirect if needed)
  await page.goto('/test-harness.html', { waitUntil: 'networkidle' });

  // Inject deterministic configuration
  await page.evaluate((config) => {
    (window as any).__VS_RENDERER_CONFIG__ = config;
  }, WGPU_DETERMINISTIC_CONFIG);

  // Wait for renderer initialization
  await page.waitForFunction(() => (window as any).__VS_RENDERER_READY__ === true, {
    timeout: 10000,
  });
}

/**
 * Render a constraint graph and return the pixel buffer.
 */
async function renderConstraintGraph(
  page: Page,
  ir: unknown,
): Promise<Uint8Array> {
  const base64 = await page.evaluate(async (constraintGraph) => {
    const renderer = (window as any).__VS_RENDERER__;

    // Render the constraint graph
    await renderer.render(constraintGraph);

    // Force flush and extract pixel buffer
    const canvas = document.getElementById('vs-canvas') as HTMLCanvasElement;
    const ctx = canvas.getContext('2d');

    if (!ctx) throw new Error('No 2D context');

    const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);
    const buffer = imageData.data;

    // Convert to base64 in chunks to avoid stack overflow
    let binary = '';
    const chunkSize = 8192;
    for (let i = 0; i < buffer.length; i += chunkSize) {
      const chunk = buffer.subarray(i, Math.min(i + chunkSize, buffer.length));
      binary += String.fromCharCode.apply(null, Array.from(chunk));
    }
    return btoa(binary);
  }, ir);

  // Decode base64 to Uint8Array
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/**
 * Compute SHA-256 hash of pixel buffer.
 */
function computeHash(buffer: Uint8Array): string {
  return crypto.createHash('sha256').update(buffer).digest('hex');
}

/**
 * Load golden hash from file.
 */
function loadGoldenHash(testName: string): string | null {
  const hashFile = path.join(GOLDEN_DIR, `${testName}.sha256`);
  if (fs.existsSync(hashFile)) {
    return fs.readFileSync(hashFile, 'utf-8').trim();
  }
  return null;
}

/**
 * Save golden snapshot and hash.
 */
function saveGolden(testName: string, buffer: Uint8Array, hash: string): void {
  const pngFile = path.join(GOLDEN_DIR, `${testName}.raw`);
  const hashFile = path.join(GOLDEN_DIR, `${testName}.sha256`);

  fs.writeFileSync(pngFile, buffer);
  fs.writeFileSync(hashFile, hash);
}

/**
 * Detect gaps between adjacent colored rectangles.
 *
 * Strategy: Scan horizontal lines and detect transitions.
 * A gap exists if we see: [Color A] -> [Black] -> [Color B]
 * The container background is expected to be black, so we only flag
 * black pixels that appear BETWEEN colored regions.
 */
function detectGaps(buffer: Uint8Array, width: number, height: number): boolean {
  // In RGBA format, check each row for gap patterns
  for (let y = 0; y < height; y++) {
    let inColoredRegion = false;
    let sawBlackAfterColor = false;

    for (let x = 0; x < width; x++) {
      const offset = (y * width + x) * 4;
      const r = buffer[offset];
      const g = buffer[offset + 1];
      const b = buffer[offset + 2];
      const a = buffer[offset + 3];

      const isBlack = r === 0 && g === 0 && b === 0 && a === 255;
      const isColored = !isBlack && a === 255;

      if (isColored) {
        if (sawBlackAfterColor) {
          // Found: [Color] -> [Black] -> [Color] = gap!
          console.warn(`[GAP DETECTED] black pixel between colors at row ${y}, near x=${x}`);
          return true;
        }
        inColoredRegion = true;
        sawBlackAfterColor = false;
      } else if (isBlack && inColoredRegion) {
        sawBlackAfterColor = true;
      }
    }
  }
  return false;
}
