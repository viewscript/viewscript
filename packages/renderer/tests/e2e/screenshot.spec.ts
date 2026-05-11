/**
 * Visual Demo Screenshot Test
 *
 * Takes a screenshot of the ViewScript renderer displaying IR constructed
 * through the standard pipeline.
 */

import { test, expect } from '@playwright/test';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';
import { existsSync, mkdirSync, statSync } from 'fs';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

test.describe('Visual Demo Screenshot', () => {
  test('capture ViewScript rendering', async ({ page, baseURL }) => {
    // Navigate to the visual demo via webServer
    await page.goto(`${baseURL}/visual-demo.html`);

    // Wait for renderer to initialize
    await page.waitForFunction(() => window.__VS_DEMO_READY__ === true, {
      timeout: 5000
    });

    // Wait a bit for animation to reach a good frame
    await page.waitForTimeout(500);

    // Take screenshot
    const screenshotPath = join(__dirname, '../../screenshots/visual-demo.png');

    // Ensure screenshots directory exists
    const screenshotDir = dirname(screenshotPath);
    if (!existsSync(screenshotDir)) {
      mkdirSync(screenshotDir, { recursive: true });
    }

    await page.screenshot({
      path: screenshotPath,
      fullPage: false,
      clip: { x: 0, y: 0, width: 800, height: 600 }
    });

    console.log(`Screenshot saved to: ${screenshotPath}`);

    // Verify screenshot was created
    expect(existsSync(screenshotPath)).toBe(true);

    // Verify file size is reasonable (not empty)
    const stats = statSync(screenshotPath);
    expect(stats.size).toBeGreaterThan(1000); // At least 1KB
  });
});

// Extend Window interface for TypeScript
declare global {
  interface Window {
    __VS_DEMO_READY__?: boolean;
  }
}
