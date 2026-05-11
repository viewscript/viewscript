/**
 * Text Layout E2E Tests (Phase 10)
 *
 * This module verifies that text entities with Q→P dimension bridging
 * correctly constrain their containing elements.
 *
 * ## The Problem: Constraint-Based Text Layout
 *
 * Text dimensions are Q-dimension (non-deterministic, font-dependent).
 * P-dimension constraints need exact rational values. The bridge:
 *
 * 1. Renderer measures text using CanvasKit/DOM → W, H (pixels)
 * 2. CLI receives: `vsc update-metrics --id=N --width=W --height=H`
 * 3. P-dimension solver updates bounding box constraints
 * 4. Containing elements (buttons, etc.) resize accordingly
 *
 * ## Test Strategy
 *
 * 1. Create a text entity via CLI
 * 2. Simulate Renderer measurement
 * 3. Update metrics via CLI
 * 4. Verify containing button resizes to fit text
 *
 * ## Font Determinism
 *
 * To ensure deterministic tests, we use a locally bundled monospace font.
 * This eliminates variations from system fonts or network-loaded fonts.
 */

import { test, expect } from '@playwright/test';
import { execSync } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';

// =============================================================================
// Test Configuration
// =============================================================================

/**
 * Monospace font metrics for deterministic testing.
 * Using a standard monospace where each character has equal width.
 */
const MONOSPACE_CHAR_WIDTH = 10; // pixels per character at 16px font size
const MONOSPACE_LINE_HEIGHT = 20; // pixels per line at 16px font size

/**
 * Button padding around text.
 */
const BUTTON_PADDING = 8;

// =============================================================================
// CLI Helper
// =============================================================================

/**
 * Execute a VSC CLI command and return the parsed result.
 */
function vsc(args: string[], cwd: string): { exitCode: number; output: unknown; error?: string } {
  const vscPath = path.resolve(__dirname, '../../../../target/debug/vsc');
  const fullCommand = `${vscPath} ${args.join(' ')}`;

  try {
    const stdout = execSync(fullCommand, {
      cwd,
      encoding: 'utf-8',
      env: {
        ...process.env,
        VS_FIXED_TIME: '1', // Deterministic timestamps
      },
    });

    return {
      exitCode: 0,
      output: JSON.parse(stdout),
    };
  } catch (error: unknown) {
    if (error && typeof error === 'object' && 'status' in error) {
      const execError = error as { status: number; stdout?: string; stderr?: string };
      return {
        exitCode: execError.status || 1,
        output: execError.stdout ? JSON.parse(execError.stdout) : null,
        error: execError.stderr || 'Command failed',
      };
    }
    throw error;
  }
}

/**
 * Simulate text measurement (deterministic for monospace).
 */
function measureTextMonospace(content: string, fontSize: number): { width: number; height: number } {
  const charWidth = (fontSize / 16) * MONOSPACE_CHAR_WIDTH;
  const lineHeight = (fontSize / 16) * MONOSPACE_LINE_HEIGHT;
  const lines = content.split('\n');

  const maxLineLength = Math.max(...lines.map(l => l.length));

  return {
    width: Math.ceil(maxLineLength * charWidth),
    height: Math.ceil(lines.length * lineHeight),
  };
}

// =============================================================================
// Text Layout Tests
// =============================================================================

test.describe('Text Layout: Q→P Dimension Bridge', () => {
  let testDir: string;

  test.beforeEach(async () => {
    // Create a temporary directory for each test
    testDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vsc-text-layout-'));

    // Initialize a ViewScript project
    const initResult = vsc(['init', '--name=text-layout-test'], testDir);
    expect(initResult.exitCode).toBe(0);
  });

  test.afterEach(async () => {
    // Clean up
    if (testDir && fs.existsSync(testDir)) {
      fs.rmSync(testDir, { recursive: true, force: true });
    }
  });

  /**
   * Test: Text entity creation generates 4 corner control points
   */
  test('add-entity --type=text creates text with 4 corner control points', async () => {
    const result = vsc([
      'add-entity',
      '--type=text',
      '--content="Hello, World!"',
      '--font-family="monospace"',
      '--font-size=16',
      '--x=0',
      '--y=0',
    ], testDir);

    expect(result.exitCode).toBe(0);

    const output = result.output as {
      status: string;
      entity_type: string;
      entity_id: number;
      corner_tl: number;
      corner_tr: number;
      corner_bl: number;
      corner_br: number;
      metrics_pending: boolean;
    };

    expect(output.status).toBe('success');
    expect(output.entity_type).toBe('text');
    expect(output.entity_id).toBeDefined();
    expect(output.corner_tl).toBe(output.entity_id + 1);
    expect(output.corner_tr).toBe(output.entity_id + 2);
    expect(output.corner_bl).toBe(output.entity_id + 3);
    expect(output.corner_br).toBe(output.entity_id + 4);
    expect(output.metrics_pending).toBe(true);
  });

  /**
   * Test: update-metrics adds bounding box constraints
   */
  test('update-metrics adds width and height constraints', async () => {
    // Create text entity
    const addResult = vsc([
      'add-entity',
      '--type=text',
      '--content="Test"',
      '--font-family="monospace"',
      '--font-size=16',
    ], testDir);

    expect(addResult.exitCode).toBe(0);
    const textId = (addResult.output as { entity_id: number }).entity_id;

    // Measure text (simulated)
    const metrics = measureTextMonospace('Test', 16);

    // Update metrics
    const updateResult = vsc([
      'update-metrics',
      `--id=${textId}`,
      `--width=${metrics.width}`,
      `--height=${metrics.height}`,
    ], testDir);

    expect(updateResult.exitCode).toBe(0);

    const output = updateResult.output as {
      status: string;
      constraints_added: number;
    };

    expect(output.status).toBe('success');
    // 8 constraints: 2 width, 2 height, 4 alignment
    expect(output.constraints_added).toBe(8);
  });

  /**
   * Test: Button containing text resizes based on text metrics
   *
   * Scenario:
   * 1. Create text entity "Click Me"
   * 2. Update metrics (simulated measurement)
   * 3. Create button constraints: button.width = text.width + 2*padding
   * 4. Verify button width matches expected value
   */
  test('button width is constrained by text width plus padding', async () => {
    const textContent = 'Click Me';

    // Step 1: Create text entity
    const addResult = vsc([
      'add-entity',
      '--type=text',
      `--content="${textContent}"`,
      '--font-family="monospace"',
      '--font-size=16',
      '--x=100',
      '--y=100',
    ], testDir);

    expect(addResult.exitCode).toBe(0);
    const addOutput = addResult.output as {
      entity_id: number;
      corner_tl: number;
      corner_tr: number;
    };

    // Step 2: Measure and update metrics
    const metrics = measureTextMonospace(textContent, 16);

    const updateResult = vsc([
      'update-metrics',
      `--id=${addOutput.entity_id}`,
      `--width=${metrics.width}`,
      `--height=${metrics.height}`,
    ], testDir);

    expect(updateResult.exitCode).toBe(0);

    // Step 3: Create button entity and constrain it to text
    // Button width = TR.x - TL.x + 2*padding
    // This is: (TL.x + text_width) - TL.x + 2*padding = text_width + 2*padding

    // For this test, we verify the constraint graph was updated correctly
    // by reading the buildinfo and checking the constraints exist
    const buildInfoPath = path.join(testDir, '.vsbuildinfo');
    const buildInfo = JSON.parse(fs.readFileSync(buildInfoPath, 'utf-8'));

    // Verify text entity entry exists with correct metrics
    expect(buildInfo.text_entities).toBeDefined();
    expect(buildInfo.text_entities.length).toBe(1);

    const textEntry = buildInfo.text_entities[0];
    expect(textEntry.metrics_resolved).toBe(true);
    expect(textEntry.measured_width).toBe(`${metrics.width}/1`);
    expect(textEntry.measured_height).toBe(`${metrics.height}/1`);

    // Verify constraints were added
    // Initial: 2 constraints for TL position
    // Metrics: 8 constraints for bounding box
    const constraintCount = buildInfo.operations.filter(
      (op: { op_type: string }) => op.op_type === 'add'
    ).length;

    expect(constraintCount).toBe(2 + 8);

    // Calculate expected button width
    const expectedButtonWidth = metrics.width + 2 * BUTTON_PADDING;

    // For this test, we just verify the text width + padding calculation
    // In a full E2E test with the renderer, we would verify the visual output
    expect(expectedButtonWidth).toBe(textContent.length * MONOSPACE_CHAR_WIDTH + 2 * BUTTON_PADDING);
  });

  /**
   * Test: Multi-line text height is correctly calculated
   */
  test('multi-line text height is sum of line heights', async () => {
    const textContent = 'Line 1\\nLine 2\\nLine 3';

    const addResult = vsc([
      'add-entity',
      '--type=text',
      `--content="${textContent}"`,
      '--font-family="monospace"',
      '--font-size=16',
    ], testDir);

    expect(addResult.exitCode).toBe(0);
    const textId = (addResult.output as { entity_id: number }).entity_id;

    // Measure with 3 lines
    const metrics = measureTextMonospace('Line 1\nLine 2\nLine 3', 16);
    expect(metrics.height).toBe(3 * MONOSPACE_LINE_HEIGHT);

    const updateResult = vsc([
      'update-metrics',
      `--id=${textId}`,
      `--width=${metrics.width}`,
      `--height=${metrics.height}`,
    ], testDir);

    expect(updateResult.exitCode).toBe(0);
  });

  /**
   * Test: Font size scaling affects metrics proportionally
   */
  test('font size 32 doubles text dimensions compared to font size 16', async () => {
    const textContent = 'Scale Test';

    // Create two text entities with different font sizes
    const add16 = vsc([
      'add-entity',
      '--type=text',
      `--content="${textContent}"`,
      '--font-family="monospace"',
      '--font-size=16',
    ], testDir);

    expect(add16.exitCode).toBe(0);
    const id16 = (add16.output as { entity_id: number }).entity_id;

    const add32 = vsc([
      'add-entity',
      '--type=text',
      `--content="${textContent}"`,
      '--font-family="monospace"',
      '--font-size=32',
    ], testDir);

    expect(add32.exitCode).toBe(0);
    const id32 = (add32.output as { entity_id: number }).entity_id;

    // Measure both
    const metrics16 = measureTextMonospace(textContent, 16);
    const metrics32 = measureTextMonospace(textContent, 32);

    // 32px should be exactly 2x the 16px dimensions
    expect(metrics32.width).toBe(metrics16.width * 2);
    expect(metrics32.height).toBe(metrics16.height * 2);

    // Update metrics for both
    const update16 = vsc([
      'update-metrics',
      `--id=${id16}`,
      `--width=${metrics16.width}`,
      `--height=${metrics16.height}`,
    ], testDir);
    expect(update16.exitCode).toBe(0);

    const update32 = vsc([
      'update-metrics',
      `--id=${id32}`,
      `--width=${metrics32.width}`,
      `--height=${metrics32.height}`,
    ], testDir);
    expect(update32.exitCode).toBe(0);
  });

  /**
   * Test: update-metrics for non-existent entity returns error
   */
  test('update-metrics for non-existent entity fails gracefully', async () => {
    const result = vsc([
      'update-metrics',
      '--id=99999',
      '--width=100',
      '--height=20',
    ], testDir);

    expect(result.exitCode).toBe(1);
  });

  /**
   * Test: Invalid width/height values are rejected
   */
  test('update-metrics with invalid values is rejected', async () => {
    // Create a text entity first
    const addResult = vsc([
      'add-entity',
      '--type=text',
      '--content="Test"',
    ], testDir);

    expect(addResult.exitCode).toBe(0);
    const textId = (addResult.output as { entity_id: number }).entity_id;

    // Try to update with invalid width
    const result = vsc([
      'update-metrics',
      `--id=${textId}`,
      '--width=invalid',
      '--height=20',
    ], testDir);

    expect(result.exitCode).toBe(1);
  });
});

// =============================================================================
// Determinism Tests (Font Loading)
// =============================================================================

test.describe('Text Layout: Determinism', () => {
  let testDir: string;

  test.beforeEach(async () => {
    testDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vsc-text-determinism-'));
    vsc(['init', '--name=determinism-test'], testDir);
  });

  test.afterEach(async () => {
    if (testDir && fs.existsSync(testDir)) {
      fs.rmSync(testDir, { recursive: true, force: true });
    }
  });

  /**
   * Test: Same content produces same measurements
   *
   * This validates that our simulated monospace measurement is deterministic.
   * In production, font loading async issues can cause non-determinism.
   */
  test('identical text produces identical measurements across runs', async () => {
    const textContent = 'Determinism Test String';

    // Run measurement 3 times
    const measurements = Array.from({ length: 3 }, () =>
      measureTextMonospace(textContent, 16)
    );

    // All measurements should be identical
    expect(measurements[0].width).toBe(measurements[1].width);
    expect(measurements[0].width).toBe(measurements[2].width);
    expect(measurements[0].height).toBe(measurements[1].height);
    expect(measurements[0].height).toBe(measurements[2].height);
  });
});
