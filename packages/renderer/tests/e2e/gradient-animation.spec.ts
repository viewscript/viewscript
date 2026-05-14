/**
 * Gradient Animation E2E Tests: Phase 17 Validation
 *
 * This module validates the complete gradient pipeline from CSS input
 * to GPU shader output, including T-vector animation binding.
 *
 * ## Test Coverage
 *
 * 1. Static Gradient Rendering
 *    - Linear gradients (directional)
 *    - Radial gradients (circular, elliptical)
 *    - Conic gradients (sweep)
 *
 * 2. T-Vector Animation
 *    - Color stop position interpolation
 *    - Gradient angle animation
 *    - Rotation animation for conic gradients
 *
 * 3. P-Dimension Integrity
 *    - Rational color values preserved until rasterization
 *    - No floating-point contamination in constraint evaluation
 *    - Topology-preserving rounding at GPU boundary
 *
 * 4. Tile Mode Behavior
 *    - Clamp (default)
 *    - Repeat
 *    - Mirror
 *    - Decal
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

const CANVASKIT_DETERMINISTIC_CONFIG = {
  disableWebGL: true,
  preferLowPowerToHighPerformance: false,
  useSubpixelText: false,
  devicePixelRatio: 1.0,
};

// =============================================================================
// Linear Gradient Tests
// =============================================================================

test.describe('Gradient Rendering: Linear Gradients', () => {
  test.beforeAll(async () => {
    ensureDirectories();
  });

  /**
   * Test: Simple horizontal linear gradient
   *
   * CSS: linear-gradient(to right, red, blue)
   * Expected: Smooth horizontal transition from red to blue
   */
  test('renders horizontal linear gradient', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderGradient(page, {
      type: 'linear-gradient',
      css: 'linear-gradient(to right, red, blue)',
      bounds: { x: 0, y: 0, width: 200, height: 100 },
    });

    // Verify color transition: left edge should be red, right edge should be blue
    const { leftEdgeColor, rightEdgeColor } = analyzeHorizontalGradient(pixelBuffer, 200, 100);

    expect(leftEdgeColor.r).toBeGreaterThan(200); // Red
    expect(leftEdgeColor.g).toBeLessThan(50);
    expect(leftEdgeColor.b).toBeLessThan(50);

    expect(rightEdgeColor.r).toBeLessThan(50);
    expect(rightEdgeColor.g).toBeLessThan(50);
    expect(rightEdgeColor.b).toBeGreaterThan(200); // Blue

    // Golden hash comparison
    await compareGolden(pixelBuffer, 'linear-horizontal');
  });

  /**
   * Test: 45-degree linear gradient
   *
   * CSS: linear-gradient(45deg, #FF0000, #0000FF)
   * Expected: Diagonal gradient from bottom-left to top-right
   */
  test('renders 45-degree linear gradient', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderGradient(page, {
      type: 'linear-gradient',
      css: 'linear-gradient(45deg, #FF0000, #0000FF)',
      bounds: { x: 0, y: 0, width: 100, height: 100 },
    });

    // For a 45deg gradient in a square, bottom-left should be red, top-right should be blue
    const bottomLeft = getPixelColor(pixelBuffer, 100, 100, 10, 90);
    const topRight = getPixelColor(pixelBuffer, 100, 100, 90, 10);

    expect(bottomLeft.r).toBeGreaterThan(150); // More red
    expect(topRight.b).toBeGreaterThan(150); // More blue

    await compareGolden(pixelBuffer, 'linear-45deg');
  });

  /**
   * Test: Multi-stop linear gradient
   *
   * CSS: linear-gradient(to right, red 0%, yellow 50%, blue 100%)
   * Expected: Red -> Yellow -> Blue transition
   */
  test('renders multi-stop linear gradient', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderGradient(page, {
      type: 'linear-gradient',
      css: 'linear-gradient(to right, red 0%, yellow 50%, blue 100%)',
      bounds: { x: 0, y: 0, width: 200, height: 50 },
    });

    // Sample at 25% (red-yellow mix), 50% (yellow), 75% (yellow-blue mix)
    const at25 = getPixelColor(pixelBuffer, 200, 50, 50, 25);
    const at50 = getPixelColor(pixelBuffer, 200, 50, 100, 25);
    const at75 = getPixelColor(pixelBuffer, 200, 50, 150, 25);

    // At 50%, should be yellow (high R, high G, low B)
    expect(at50.r).toBeGreaterThan(200);
    expect(at50.g).toBeGreaterThan(200);
    expect(at50.b).toBeLessThan(100);

    await compareGolden(pixelBuffer, 'linear-multi-stop');
  });
});

// =============================================================================
// Radial Gradient Tests
// =============================================================================

test.describe('Gradient Rendering: Radial Gradients', () => {
  test.beforeAll(async () => {
    ensureDirectories();
  });

  /**
   * Test: Circle radial gradient at center
   *
   * CSS: radial-gradient(circle at center, white, black)
   * Expected: White center fading to black edges
   */
  test('renders centered circle radial gradient', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderGradient(page, {
      type: 'radial-gradient',
      css: 'radial-gradient(circle at center, white, black)',
      bounds: { x: 0, y: 0, width: 100, height: 100 },
    });

    // Center should be white, corners should be dark
    const center = getPixelColor(pixelBuffer, 100, 100, 50, 50);
    const corner = getPixelColor(pixelBuffer, 100, 100, 5, 5);

    expect(center.r).toBeGreaterThan(200);
    expect(center.g).toBeGreaterThan(200);
    expect(center.b).toBeGreaterThan(200);

    expect(corner.r).toBeLessThan(100);
    expect(corner.g).toBeLessThan(100);
    expect(corner.b).toBeLessThan(100);

    await compareGolden(pixelBuffer, 'radial-circle-center');
  });

  /**
   * Test: Offset focal point radial gradient
   *
   * CSS: radial-gradient(circle at 25% 25%, red, blue)
   * Expected: Gradient center offset to top-left quadrant
   */
  test('renders offset radial gradient', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderGradient(page, {
      type: 'radial-gradient',
      css: 'radial-gradient(circle at 25% 25%, red, blue)',
      bounds: { x: 0, y: 0, width: 100, height: 100 },
    });

    // Point at 25%, 25% should be red (center of gradient)
    const focalPoint = getPixelColor(pixelBuffer, 100, 100, 25, 25);

    expect(focalPoint.r).toBeGreaterThan(200);
    expect(focalPoint.b).toBeLessThan(100);

    await compareGolden(pixelBuffer, 'radial-offset');
  });
});

// =============================================================================
// Conic Gradient Tests
// =============================================================================

test.describe('Gradient Rendering: Conic (Sweep) Gradients', () => {
  test.beforeAll(async () => {
    ensureDirectories();
  });

  /**
   * Test: Color wheel conic gradient
   *
   * CSS: conic-gradient(from 0deg, red, yellow, lime, cyan, blue, magenta, red)
   * Expected: Full color wheel around center
   */
  test('renders color wheel conic gradient', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderGradient(page, {
      type: 'conic-gradient',
      css: 'conic-gradient(from 0deg, red, yellow, lime, cyan, blue, magenta, red)',
      bounds: { x: 0, y: 0, width: 100, height: 100 },
    });

    // CSS conic-gradient: 0deg = top (north), angles increase clockwise.
    // With "from 0deg", red starts at top and transitions clockwise:
    //   top (0deg) = red, right (90deg) = yellow/lime, bottom (180deg) = cyan, left (270deg) = blue/magenta
    const top = getPixelColor(pixelBuffer, 100, 100, 50, 5);    // 0deg - red
    const right = getPixelColor(pixelBuffer, 100, 100, 95, 50); // 90deg - between yellow and lime
    const bottom = getPixelColor(pixelBuffer, 100, 100, 50, 95); // 180deg - cyan
    const left = getPixelColor(pixelBuffer, 100, 100, 5, 50);   // 270deg - blue/magenta

    // Top edge (0deg) should be red
    expect(top.r).toBeGreaterThan(150);

    await compareGolden(pixelBuffer, 'conic-color-wheel');
  });

  /**
   * Test: Rotated conic gradient
   *
   * CSS: conic-gradient(from 90deg at center, red, blue)
   * Expected: Gradient starts from bottom instead of right
   */
  test('renders rotated conic gradient', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderGradient(page, {
      type: 'conic-gradient',
      css: 'conic-gradient(from 90deg at center, red, blue)',
      bounds: { x: 0, y: 0, width: 100, height: 100 },
    });

    // With 90deg rotation, red should be at bottom, blue at top
    const bottom = getPixelColor(pixelBuffer, 100, 100, 50, 95);
    const top = getPixelColor(pixelBuffer, 100, 100, 50, 5);

    expect(bottom.r).toBeGreaterThan(bottom.b); // More red at bottom
    expect(top.b).toBeGreaterThan(top.r); // More blue at top

    await compareGolden(pixelBuffer, 'conic-rotated');
  });
});

// =============================================================================
// T-Vector Animation Tests
// =============================================================================

test.describe('Gradient Animation: T-Vector Binding', () => {
  test.beforeAll(async () => {
    ensureDirectories();
  });

  /**
   * Test: Animated gradient angle
   *
   * T-vector controls the gradient angle: as T increases, angle rotates.
   * This validates that P-dimension constraints properly propagate to GPU.
   */
  test('animates gradient angle via T-vector', async ({ page }) => {
    await setupDeterministicRenderer(page);

    // Frame 1: T=0, angle=0deg
    const frame1 = await renderAnimatedGradient(page, {
      type: 'linear-gradient',
      baseAngle: 0,
      tValue: 0,
      anglePerT: 90, // 90 degrees per T unit
      colors: ['red', 'blue'],
      bounds: { x: 0, y: 0, width: 100, height: 100 },
    });

    // Frame 2: T=1, angle=90deg
    const frame2 = await renderAnimatedGradient(page, {
      type: 'linear-gradient',
      baseAngle: 0,
      tValue: 1,
      anglePerT: 90,
      colors: ['red', 'blue'],
      bounds: { x: 0, y: 0, width: 100, height: 100 },
    });

    // Frame 1 should be horizontal (red on left)
    const f1Left = getPixelColor(frame1, 100, 100, 10, 50);
    expect(f1Left.r).toBeGreaterThan(f1Left.b);

    // Frame 2 should be vertical (red on top)
    const f2Top = getPixelColor(frame2, 100, 100, 50, 10);
    expect(f2Top.r).toBeGreaterThan(f2Top.b);

    // Frames should be different
    const hash1 = computeHash(frame1);
    const hash2 = computeHash(frame2);
    expect(hash1).not.toBe(hash2);
  });

  /**
   * Test: Animated color stop positions
   *
   * T-vector controls color stop positions for dynamic effects.
   */
  test('animates color stop positions via T-vector', async ({ page }) => {
    await setupDeterministicRenderer(page);

    // Frame 1: First stop at 30%
    const frame1 = await renderAnimatedGradient(page, {
      type: 'linear-gradient',
      baseAngle: 90, // Top to bottom
      tValue: 0.3,
      colors: ['red', 'blue'],
      stopPositions: [{ base: 0, tFactor: 1 }, { base: 1, tFactor: 0 }],
      bounds: { x: 0, y: 0, width: 100, height: 100 },
    });

    // Frame 2: First stop at 70%
    const frame2 = await renderAnimatedGradient(page, {
      type: 'linear-gradient',
      baseAngle: 90,
      tValue: 0.7,
      colors: ['red', 'blue'],
      stopPositions: [{ base: 0, tFactor: 1 }, { base: 1, tFactor: 0 }],
      bounds: { x: 0, y: 0, width: 100, height: 100 },
    });

    // In frame 1, red extends further (to ~30% from top)
    // In frame 2, red extends much further (to ~70% from top)
    const f1Mid = getPixelColor(frame1, 100, 100, 50, 50);
    const f2Mid = getPixelColor(frame2, 100, 100, 50, 50);

    // Frame 2 middle should have more red than frame 1 middle
    expect(f2Mid.r).toBeGreaterThan(f1Mid.r);
  });
});

// =============================================================================
// Tile Mode Tests
// =============================================================================

test.describe('Gradient Rendering: Tile Modes', () => {
  test.beforeAll(async () => {
    ensureDirectories();
  });

  /**
   * Test: Repeat tile mode
   *
   * Gradient should repeat beyond 100% position
   */
  test('applies repeat tile mode correctly', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderGradient(page, {
      type: 'linear-gradient',
      css: 'linear-gradient(to right, red 0%, blue 25%)',
      tileMode: 'repeat',
      bounds: { x: 0, y: 0, width: 200, height: 50 },
    });

    // With repeat, the gradient at 50% should look like gradient at 0%
    const at0 = getPixelColor(pixelBuffer, 200, 50, 5, 25);
    const at50 = getPixelColor(pixelBuffer, 200, 50, 105, 25);

    // Both should be similar (both at start of repeat cycle)
    expect(Math.abs(at0.r - at50.r)).toBeLessThan(30);

    await compareGolden(pixelBuffer, 'tile-repeat');
  });

  /**
   * Test: Mirror tile mode
   *
   * Gradient should reverse direction at 100%
   */
  test('applies mirror tile mode correctly', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderGradient(page, {
      type: 'linear-gradient',
      css: 'linear-gradient(to right, red 0%, blue 50%)',
      tileMode: 'mirror',
      bounds: { x: 0, y: 0, width: 200, height: 50 },
    });

    // With mirror, color at 75% of total width should be same as at 25%
    const at25 = getPixelColor(pixelBuffer, 200, 50, 50, 25);
    const at75 = getPixelColor(pixelBuffer, 200, 50, 150, 25);

    // Mirror means 75% in second half = 25% position
    expect(Math.abs(at25.r - at75.r)).toBeLessThan(30);

    await compareGolden(pixelBuffer, 'tile-mirror');
  });
});

// =============================================================================
// P-Dimension Integrity Tests
// =============================================================================

test.describe('Gradient Rendering: P-Dimension Integrity', () => {
  test.beforeAll(async () => {
    ensureDirectories();
  });

  /**
   * Test: Exact rational color preservation
   *
   * Color specified as rational 255/3 (~85) should not drift due to
   * floating-point operations in constraint evaluation.
   */
  test('preserves exact rational color values', async ({ page }) => {
    await setupDeterministicRenderer(page);

    // Specify color as exact rational: RGB(255/3, 255/3, 255/3) = gray ~85
    const pixelBuffer = await renderGradient(page, {
      type: 'solid',
      rationalColor: {
        r: { numerator: 255n, denominator: 3n },
        g: { numerator: 255n, denominator: 3n },
        b: { numerator: 255n, denominator: 3n },
      },
      bounds: { x: 0, y: 0, width: 100, height: 100 },
    });

    // Sample center pixel
    const center = getPixelColor(pixelBuffer, 100, 100, 50, 50);

    // 255/3 = 85 (floor) - should be exactly 85, not 84 or 86
    expect(center.r).toBe(85);
    expect(center.g).toBe(85);
    expect(center.b).toBe(85);
  });

  /**
   * Test: Topology-preserving color clamping
   *
   * Colors outside [0, 255] should be clamped, preserving ordering.
   */
  test('clamps out-of-range colors while preserving order', async ({ page }) => {
    await setupDeterministicRenderer(page);

    const pixelBuffer = await renderGradient(page, {
      type: 'linear-gradient',
      colors: [
        { r: -10, g: 0, b: 0 },    // Should clamp to 0
        { r: 300, g: 0, b: 0 },    // Should clamp to 255
      ],
      bounds: { x: 0, y: 0, width: 100, height: 50 },
    });

    const left = getPixelColor(pixelBuffer, 100, 50, 5, 25);
    const right = getPixelColor(pixelBuffer, 100, 50, 95, 25);

    // Left should be clamped to black (r≈0, allowing 1-2 for subpixel tolerance)
    expect(left.r).toBeLessThanOrEqual(2);

    // Right should be clamped to bright red (r≈255, allowing 1-2 for subpixel tolerance)
    expect(right.r).toBeGreaterThanOrEqual(253);
  });
});

// =============================================================================
// Helper Functions
// =============================================================================

function ensureDirectories(): void {
  if (!fs.existsSync(GOLDEN_DIR)) {
    fs.mkdirSync(GOLDEN_DIR, { recursive: true });
  }
  if (!fs.existsSync(FAILURE_DIR)) {
    fs.mkdirSync(FAILURE_DIR, { recursive: true });
  }
}

async function setupDeterministicRenderer(page: Page): Promise<void> {
  await page.goto('/test-harness.html', { waitUntil: 'networkidle' });

  await page.evaluate((config) => {
    (window as any).__VS_CANVASKIT_CONFIG__ = config;
  }, CANVASKIT_DETERMINISTIC_CONFIG);

  await page.waitForFunction(() => (window as any).__VS_RENDERER_READY__ === true, {
    timeout: 10000,
  });
}

async function renderGradient(
  page: Page,
  spec: Record<string, unknown>,
): Promise<Uint8Array> {
  const base64 = await page.evaluate(async (gradientSpec) => {
    const renderer = (window as any).__VS_RENDERER__;
    await renderer.renderGradient(gradientSpec);

    const canvas = document.getElementById('vs-canvas') as HTMLCanvasElement;
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('No 2D context');

    const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);
    const buffer = imageData.data;

    let binary = '';
    const chunkSize = 8192;
    for (let i = 0; i < buffer.length; i += chunkSize) {
      const chunk = buffer.subarray(i, Math.min(i + chunkSize, buffer.length));
      binary += String.fromCharCode.apply(null, Array.from(chunk));
    }
    return btoa(binary);
  }, spec);

  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

async function renderAnimatedGradient(
  page: Page,
  spec: Record<string, unknown>,
): Promise<Uint8Array> {
  return renderGradient(page, { ...spec, animated: true });
}

function computeHash(buffer: Uint8Array): string {
  return crypto.createHash('sha256').update(buffer).digest('hex');
}

function getPixelColor(
  buffer: Uint8Array,
  width: number,
  height: number,
  x: number,
  y: number,
): { r: number; g: number; b: number; a: number } {
  const offset = (y * width + x) * 4;
  return {
    r: buffer[offset],
    g: buffer[offset + 1],
    b: buffer[offset + 2],
    a: buffer[offset + 3],
  };
}

function analyzeHorizontalGradient(
  buffer: Uint8Array,
  width: number,
  height: number,
): { leftEdgeColor: { r: number; g: number; b: number }; rightEdgeColor: { r: number; g: number; b: number } } {
  const midY = Math.floor(height / 2);
  return {
    leftEdgeColor: getPixelColor(buffer, width, height, 5, midY),
    rightEdgeColor: getPixelColor(buffer, width, height, width - 5, midY),
  };
}

async function compareGolden(buffer: Uint8Array, testName: string): Promise<void> {
  const hash = computeHash(buffer);
  const hashFile = path.join(GOLDEN_DIR, `${testName}.sha256`);

  if (fs.existsSync(hashFile)) {
    const goldenHash = fs.readFileSync(hashFile, 'utf-8').trim();
    if (hash !== goldenHash) {
      // Save failure for debugging
      const failFile = path.join(FAILURE_DIR, `${testName}-fail.raw`);
      fs.writeFileSync(failFile, buffer);
    }
    expect(hash).toBe(goldenHash);
  } else {
    // First run: save as golden
    const rawFile = path.join(GOLDEN_DIR, `${testName}.raw`);
    fs.writeFileSync(rawFile, buffer);
    fs.writeFileSync(hashFile, hash);
    console.log(`[GOLDEN] Saved: ${testName} (${hash})`);
  }
}
