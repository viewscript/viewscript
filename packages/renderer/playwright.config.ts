import { defineConfig, devices } from '@playwright/test';

/**
 * Playwright Configuration for ViewScript Renderer E2E Tests
 *
 * ## Determinism Strategy
 *
 * 1. Single browser (Chromium) for consistency
 * 2. Fixed viewport size (800x600)
 * 3. Disable GPU acceleration for visual tests
 * 4. Disable animations for timing predictability
 */

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: false, // Serial execution for determinism
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1, // Single worker for determinism
  reporter: [
    ['html', { open: 'never' }],
    ['json', { outputFile: 'test-results/results.json' }],
  ],
  use: {
    baseURL: 'http://localhost:3000',
    trace: 'on-first-retry',
    video: 'on-first-retry',
  },

  projects: [
    {
      name: 'visual-regression',
      testMatch: /visual-regression\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
        deviceScaleFactor: 1, // Fixed DPI
        // Force software rendering
        launchOptions: {
          args: [
            '--disable-gpu',
            '--disable-gpu-compositing',
            '--disable-gpu-rasterization',
            '--disable-software-rasterizer',
            '--use-gl=swiftshader',
            '--disable-accelerated-2d-canvas',
            '--disable-accelerated-video-decode',
          ],
        },
      },
    },
    {
      name: 'bilayer-sync',
      testMatch: /bilayer-sync\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
        deviceScaleFactor: 1, // Fixed DPI for coordinate determinism
      },
    },
    {
      name: 'performance',
      testMatch: /performance-profile\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
        // Enable GPU for realistic perf testing
        launchOptions: {
          args: [
            '--enable-gpu-rasterization',
            '--enable-zero-copy',
          ],
        },
      },
    },
    {
      name: 'memory-stability',
      testMatch: /memory-stability\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
      },
      timeout: 300000, // 5 minutes for stress test
    },
    {
      name: 'async-race',
      testMatch: /async-race\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
        deviceScaleFactor: 1, // Fixed DPI for coordinate determinism
      },
      timeout: 60000, // 1 minute for event storm processing
    },
    {
      name: 'screenshot',
      testMatch: /screenshot\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
        deviceScaleFactor: 1,
      },
    },
    {
      name: 'path-topology',
      testMatch: /path-topology\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
        deviceScaleFactor: 1, // Fixed DPI for pixel-perfect topology verification
      },
      timeout: 30000,
    },
    {
      name: 'g1-continuity',
      testMatch: /g1-continuity\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
        deviceScaleFactor: 1, // Fixed DPI for tangent smoothness verification
      },
      timeout: 30000,
    },
    {
      name: 'fullstack',
      testMatch: /fullstack\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
        deviceScaleFactor: 1, // Fixed DPI for coordinate determinism
      },
      timeout: 60000, // 1 minute for full pipeline tests
    },
    {
      name: 'text-layout',
      testMatch: /text-layout\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
        deviceScaleFactor: 1,
      },
    },
  ],

  webServer: {
    command: 'npx serve tests/e2e -l 3000 --no-clipboard',
    url: 'http://localhost:3000/test-harness.html',
    reuseExistingServer: !process.env.CI,
    timeout: 30000,
  },
});
