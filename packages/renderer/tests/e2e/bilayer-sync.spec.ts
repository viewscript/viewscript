/**
 * Bilayer Synchronization E2E Tests
 *
 * This module verifies that the Canvas (visual) and DOM (interaction)
 * layers are perfectly synchronized. The critical invariant:
 *
 *   "Clicking a pixel on Canvas MUST trigger the correct DOM event handler"
 *
 * ## The Problem: Layer Desynchronization
 *
 * If a button visually appears at (100, 50) on Canvas but its DOM hit
 * region is at (100, 51), clicking the visual button misses the handler.
 * This is catastrophic UX failure.
 *
 * ## Test Strategy
 *
 * 1. Render a button with known bounds via constraint graph
 * 2. Click at visual boundary edges (1px inside each edge)
 * 3. Verify event fires and visual state changes
 * 4. Repeat for edge cases (animation mid-frame, rapid movement)
 */

import { test, expect, type Page, type Browser } from '@playwright/test';

// =============================================================================
// Test Configuration
// =============================================================================

interface ButtonBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

const TEST_BUTTON_BOUNDS: ButtonBounds = {
  x: 100,
  y: 100,
  width: 200,
  height: 50,
};

// Colors for state verification
const BUTTON_IDLE_COLOR = { r: 0, g: 100, b: 200 };    // Blue
const BUTTON_PRESSED_COLOR = { r: 200, g: 100, b: 0 }; // Orange

// =============================================================================
// Bilayer Synchronization Tests
// =============================================================================

test.describe('Bilayer Synchronization: Canvas-DOM Coherence', () => {

  /**
   * Test: Click at visual center triggers event
   *
   * Basic sanity check that clicking the center of a button works.
   */
  test('click at button center triggers event and changes visual state', async ({ page }) => {
    await setupTestPage(page);

    // Render a button at known position
    await renderButton(page, TEST_BUTTON_BOUNDS);

    // Calculate center coordinates
    const centerX = TEST_BUTTON_BOUNDS.x + TEST_BUTTON_BOUNDS.width / 2;
    const centerY = TEST_BUTTON_BOUNDS.y + TEST_BUTTON_BOUNDS.height / 2;

    // Verify initial color (idle)
    const initialColor = await samplePixel(page, centerX, centerY);
    expectColorMatch(initialColor, BUTTON_IDLE_COLOR);

    // Click the center
    await page.mouse.click(centerX, centerY);

    // Wait for next frame (T-vector update + render)
    await waitForNextFrame(page);

    // Verify color changed (pressed)
    const finalColor = await samplePixel(page, centerX, centerY);
    expectColorMatch(finalColor, BUTTON_PRESSED_COLOR);
  });

  /**
   * Test: Click at visual boundary (1px inside left edge)
   *
   * Critical edge case: Does the DOM hit region extend exactly
   * to the visual boundary?
   */
  test('click 1px inside left edge triggers event', async ({ page }) => {
    await setupTestPage(page);
    await renderButton(page, TEST_BUTTON_BOUNDS);

    // 1px inside left edge
    const edgeX = TEST_BUTTON_BOUNDS.x + 1;
    const centerY = TEST_BUTTON_BOUNDS.y + TEST_BUTTON_BOUNDS.height / 2;

    // Click at edge
    await page.mouse.click(edgeX, centerY);
    await waitForNextFrame(page);

    // Verify event fired (color changed)
    const color = await samplePixel(page, edgeX, centerY);
    expectColorMatch(color, BUTTON_PRESSED_COLOR);
  });

  /**
   * Test: Click at visual boundary (1px inside right edge)
   */
  test('click 1px inside right edge triggers event', async ({ page }) => {
    await setupTestPage(page);
    await renderButton(page, TEST_BUTTON_BOUNDS);

    const edgeX = TEST_BUTTON_BOUNDS.x + TEST_BUTTON_BOUNDS.width - 1;
    const centerY = TEST_BUTTON_BOUNDS.y + TEST_BUTTON_BOUNDS.height / 2;

    await page.mouse.click(edgeX, centerY);
    await waitForNextFrame(page);

    const color = await samplePixel(page, edgeX, centerY);
    expectColorMatch(color, BUTTON_PRESSED_COLOR);
  });

  /**
   * Test: Click at visual boundary (1px inside top edge)
   */
  test('click 1px inside top edge triggers event', async ({ page }) => {
    await setupTestPage(page);
    await renderButton(page, TEST_BUTTON_BOUNDS);

    const centerX = TEST_BUTTON_BOUNDS.x + TEST_BUTTON_BOUNDS.width / 2;
    const edgeY = TEST_BUTTON_BOUNDS.y + 1;

    await page.mouse.click(centerX, edgeY);
    await waitForNextFrame(page);

    const color = await samplePixel(page, centerX, edgeY);
    expectColorMatch(color, BUTTON_PRESSED_COLOR);
  });

  /**
   * Test: Click at visual boundary (1px inside bottom edge)
   */
  test('click 1px inside bottom edge triggers event', async ({ page }) => {
    await setupTestPage(page);
    await renderButton(page, TEST_BUTTON_BOUNDS);

    const centerX = TEST_BUTTON_BOUNDS.x + TEST_BUTTON_BOUNDS.width / 2;
    const edgeY = TEST_BUTTON_BOUNDS.y + TEST_BUTTON_BOUNDS.height - 1;

    await page.mouse.click(centerX, edgeY);
    await waitForNextFrame(page);

    const color = await samplePixel(page, centerX, edgeY);
    expectColorMatch(color, BUTTON_PRESSED_COLOR);
  });

  /**
   * Test: Click 1px OUTSIDE visual boundary does NOT trigger event
   *
   * Negative test: Verify that the DOM region doesn't extend
   * beyond the visual bounds.
   */
  test('click 1px outside left edge does NOT trigger event', async ({ page }) => {
    await setupTestPage(page);
    await renderButton(page, TEST_BUTTON_BOUNDS);

    const outsideX = TEST_BUTTON_BOUNDS.x - 1;
    const centerY = TEST_BUTTON_BOUNDS.y + TEST_BUTTON_BOUNDS.height / 2;

    // Click outside
    await page.mouse.click(outsideX, centerY);
    await waitForNextFrame(page);

    // Sample inside the button to verify it didn't change
    const insideX = TEST_BUTTON_BOUNDS.x + 10;
    const color = await samplePixel(page, insideX, centerY);
    expectColorMatch(color, BUTTON_IDLE_COLOR); // Still idle!
  });

  /**
   * Test: Rapid clicks during animation
   *
   * Stress test: Click while button is animating (T-vector changing).
   * Verifies that DOM position updates atomically with Canvas.
   *
   * DETERMINISM: Query actual button position from renderer state
   * rather than assuming time-based position.
   */
  test('click during animation hits moving target', async ({ page }) => {
    await setupTestPage(page);

    // Start animation: button moves from x=100 to x=300 over 500ms
    await startButtonAnimation(page, {
      from: { ...TEST_BUTTON_BOUNDS },
      to: { ...TEST_BUTTON_BOUNDS, x: 300 },
      durationMs: 500,
    });

    // Wait for animation to reach midpoint and query actual position
    await page.waitForTimeout(250);

    // Query the ACTUAL current position from renderer state (deterministic)
    const currentBounds = await page.evaluate(() => {
      const renderer = (window as any).__VS_RENDERER__;
      return renderer.getEntityBounds(1);
    });

    // Click at actual current center (not assumed position)
    const actualCenterX = currentBounds.x + TEST_BUTTON_BOUNDS.width / 2;
    const centerY = TEST_BUTTON_BOUNDS.y + TEST_BUTTON_BOUNDS.height / 2;

    await page.mouse.click(actualCenterX, centerY);
    await waitForNextFrame(page);

    // Verify the click registered (check global event counter)
    const clickCount = await page.evaluate(() => (window as any).__VS_CLICK_COUNT__);
    expect(clickCount).toBeGreaterThan(0);
  });

  /**
   * Test: All four corners (subpixel precision boundary)
   *
   * Tests the exact corner pixels to verify topology-preserving
   * rounding produces correct hit regions.
   */
  test('all four corner pixels are clickable', async ({ page }) => {
    await setupTestPage(page);
    await renderButton(page, TEST_BUTTON_BOUNDS);

    const corners = [
      { x: TEST_BUTTON_BOUNDS.x + 1, y: TEST_BUTTON_BOUNDS.y + 1 },                                                          // Top-left
      { x: TEST_BUTTON_BOUNDS.x + TEST_BUTTON_BOUNDS.width - 1, y: TEST_BUTTON_BOUNDS.y + 1 },                               // Top-right
      { x: TEST_BUTTON_BOUNDS.x + 1, y: TEST_BUTTON_BOUNDS.y + TEST_BUTTON_BOUNDS.height - 1 },                              // Bottom-left
      { x: TEST_BUTTON_BOUNDS.x + TEST_BUTTON_BOUNDS.width - 1, y: TEST_BUTTON_BOUNDS.y + TEST_BUTTON_BOUNDS.height - 1 },   // Bottom-right
    ];

    for (const corner of corners) {
      // Reset button state
      await resetButtonState(page);

      // Click corner
      await page.mouse.click(corner.x, corner.y);
      await waitForNextFrame(page);

      // Verify click registered
      const clickCount = await page.evaluate(() => (window as any).__VS_CLICK_COUNT__);
      expect(clickCount).toBeGreaterThan(0);
    }
  });
});

// =============================================================================
// Helper Functions
// =============================================================================

/**
 * Setup the test page with VS renderer.
 */
async function setupTestPage(page: Page): Promise<void> {
  await page.goto('/test-harness.html');
  await page.waitForFunction(() => (window as any).__VS_RENDERER_READY__ === true, {
    timeout: 10000,
  });

  // Initialize click counter
  await page.evaluate(() => {
    (window as any).__VS_CLICK_COUNT__ = 0;
  });
}

/**
 * Render a clickable button at specified bounds.
 */
async function renderButton(page: Page, bounds: ButtonBounds): Promise<void> {
  await page.evaluate((b) => {
    const renderer = (window as any).__VS_RENDERER__;

    // Create button entity with click handler
    renderer.render({
      entities: [
        {
          id: 1,
          type: 'rect',
          bounds: b,
          fill: '#0064C8', // BUTTON_IDLE_COLOR
          interactive: true,
          onClick: () => {
            (window as any).__VS_CLICK_COUNT__++;
            // Change color to indicate pressed
            renderer.updateEntity(1, { fill: '#C86400' }); // BUTTON_PRESSED_COLOR
          },
        },
      ],
      constraints: [],
    });
  }, bounds);

  // Wait for render
  await waitForNextFrame(page);
}

/**
 * Start an animation on the button.
 */
async function startButtonAnimation(
  page: Page,
  config: { from: ButtonBounds; to: ButtonBounds; durationMs: number },
): Promise<void> {
  await page.evaluate((cfg) => {
    const renderer = (window as any).__VS_RENDERER__;

    // First render the button at starting position
    renderer.render({
      entities: [
        {
          id: 1,
          type: 'rect',
          bounds: cfg.from,
          fill: '#0064C8',
          interactive: true,
          onClick: () => {
            (window as any).__VS_CLICK_COUNT__++;
          },
        },
      ],
      constraints: [],
    });

    // Start animation via T-vector binding
    renderer.animate(1, 'x', cfg.from.x, cfg.to.x, cfg.durationMs);
  }, config);
}

/**
 * Reset button to idle state.
 */
async function resetButtonState(page: Page): Promise<void> {
  await page.evaluate(() => {
    (window as any).__VS_CLICK_COUNT__ = 0;
    const renderer = (window as any).__VS_RENDERER__;
    renderer.updateEntity(1, { fill: '#0064C8' });
  });
  await waitForNextFrame(page);
}

/**
 * Sample a pixel color at (x, y).
 *
 * CRITICAL: Accounts for devicePixelRatio when sampling canvas pixels.
 * Viewport coordinates must be scaled to canvas backing store coordinates.
 */
async function samplePixel(
  page: Page,
  x: number,
  y: number,
): Promise<{ r: number; g: number; b: number }> {
  return await page.evaluate(({ px, py }) => {
    const canvas = document.getElementById('vs-canvas') as HTMLCanvasElement;
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('No 2D context');

    // Account for canvas position in viewport
    const rect = canvas.getBoundingClientRect();
    const canvasX = px - rect.left;
    const canvasY = py - rect.top;

    // Scale by devicePixelRatio for backing store coordinates
    const dpr = window.devicePixelRatio || 1;
    const backingX = Math.floor(canvasX * dpr);
    const backingY = Math.floor(canvasY * dpr);

    const imageData = ctx.getImageData(backingX, backingY, 1, 1);
    return {
      r: imageData.data[0],
      g: imageData.data[1],
      b: imageData.data[2],
    };
  }, { px: x, py: y });
}

/**
 * Wait for next animation frame to ensure render completed.
 */
async function waitForNextFrame(page: Page): Promise<void> {
  await page.evaluate(() => {
    return new Promise<void>((resolve) => {
      requestAnimationFrame(() => {
        requestAnimationFrame(() => resolve());
      });
    });
  });
}

/**
 * Assert color match with tolerance for antialiasing.
 */
function expectColorMatch(
  actual: { r: number; g: number; b: number },
  expected: { r: number; g: number; b: number },
  tolerance: number = 5,
): void {
  expect(Math.abs(actual.r - expected.r)).toBeLessThanOrEqual(tolerance);
  expect(Math.abs(actual.g - expected.g)).toBeLessThanOrEqual(tolerance);
  expect(Math.abs(actual.b - expected.b)).toBeLessThanOrEqual(tolerance);
}
