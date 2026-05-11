#!/usr/bin/env npx tsx
/**
 * LLM Drift Check Runner
 *
 * CLI tool for executing drift checks against baselines.
 *
 * Usage:
 *   npx tsx run-drift-check.ts --provider=anthropic --scenario=all --threshold=0.2
 */

import * as fs from 'fs/promises';
import * as path from 'path';
import { parseArgs } from 'util';
import { LLMExecutor, type ExecutorConfig } from './executor';
import { DriftCalculator, type Baseline, type DriftReport } from './drift-calculator';

// =============================================================================
// CLI Arguments
// =============================================================================

const { values: args } = parseArgs({
  options: {
    provider: { type: 'string', default: 'anthropic' },
    scenario: { type: 'string', default: 'all' },
    threshold: { type: 'string', default: '0.2' },
    output: { type: 'string', default: 'drift-report.json' },
    'work-dir': { type: 'string', default: '/tmp/vsc-drift-test' },
    'vsc-path': { type: 'string', default: './target/release/vsc' },
  },
});

// =============================================================================
// Main
// =============================================================================

interface FullReport {
  timestamp: string;
  provider: string;
  model: string;
  threshold: number;
  scenarios: DriftReport[];
  maxComposite: number;
  passed: boolean;
}

async function main(): Promise<void> {
  const provider = args.provider as 'anthropic' | 'openai';
  const threshold = parseFloat(args.threshold!);

  console.log('=== ViewScript LLM Drift Check ===');
  console.log(`Provider: ${provider}`);
  console.log(`Threshold: ${threshold}`);
  console.log('');

  // Load baselines
  const baselines = await loadBaselines(args.scenario!);
  console.log(`Loaded ${baselines.length} baseline(s)`);

  // Configure executor
  const config = getExecutorConfig(provider);
  const executor = new LLMExecutor(config);
  const calculator = new DriftCalculator(threshold);

  // Run checks
  const reports: DriftReport[] = [];

  for (const baseline of baselines) {
    console.log(`\nRunning: ${baseline.name}...`);

    const trace = await executor.execute(baseline);
    const report = calculator.calculate(baseline, trace);

    console.log(calculator.formatReport(report));
    reports.push(report);
  }

  // Build full report
  const maxComposite = Math.max(...reports.map(r => r.composite));
  const fullReport: FullReport = {
    timestamp: new Date().toISOString(),
    provider,
    model: config.model,
    threshold,
    scenarios: reports,
    maxComposite,
    passed: reports.every(r => r.passed),
  };

  // Write output
  await fs.writeFile(args.output!, JSON.stringify(fullReport, null, 2));
  console.log(`\nReport written to: ${args.output}`);

  // Summary
  console.log('\n=== Summary ===');
  console.log(`Scenarios: ${reports.length}`);
  console.log(`Passed: ${reports.filter(r => r.passed).length}`);
  console.log(`Failed: ${reports.filter(r => !r.passed).length}`);
  console.log(`Max Drift: ${(maxComposite * 100).toFixed(1)}%`);
  console.log(`Result: ${fullReport.passed ? 'PASS' : 'FAIL'}`);

  // Exit with appropriate code
  process.exit(fullReport.passed ? 0 : 1);
}

// =============================================================================
// Helpers
// =============================================================================

async function loadBaselines(scenario: string): Promise<Baseline[]> {
  const baselinesDir = path.join(__dirname, 'baselines');
  const files = await fs.readdir(baselinesDir);

  const baselines: Baseline[] = [];

  for (const file of files) {
    if (!file.endsWith('.json')) continue;

    const name = file.replace('.json', '');
    if (scenario !== 'all' && scenario !== name) continue;

    const content = await fs.readFile(path.join(baselinesDir, file), 'utf-8');
    baselines.push(JSON.parse(content));
  }

  return baselines;
}

function getExecutorConfig(provider: 'anthropic' | 'openai'): ExecutorConfig {
  if (provider === 'anthropic') {
    return {
      provider: 'anthropic',
      model: 'claude-sonnet-4-20250514',
      apiKey: process.env.ANTHROPIC_API_KEY || '',
      maxSteps: 15,
      stepTimeoutMs: 30000,
      workDir: args['work-dir']!,
      vscPath: args['vsc-path']!,
    };
  } else {
    return {
      provider: 'openai',
      model: 'gpt-4o',
      apiKey: process.env.OPENAI_API_KEY || '',
      maxSteps: 15,
      stepTimeoutMs: 30000,
      workDir: args['work-dir']!,
      vscPath: args['vsc-path']!,
    };
  }
}

// =============================================================================
// Run
// =============================================================================

main().catch((error) => {
  console.error('Fatal error:', error);
  process.exit(1);
});
