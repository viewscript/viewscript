/**
 * LLM Prompt Drift Calculator
 *
 * This module quantifies behavioral drift between LLM executions
 * using a composite score based on:
 * - Step count deviation
 * - Tool call sequence similarity (Jaccard distance)
 * - Final AST hash match
 *
 * ## Scoring Model
 *
 * composite_score = 0.3 * step_drift + 0.3 * tool_drift + 0.4 * ast_drift
 *
 * Where:
 * - step_drift = |actual_steps - expected_steps| / expected_steps
 * - tool_drift = 1 - jaccard_similarity(actual_tools, expected_tools)
 * - ast_drift = 0 if hashes match, 1 otherwise
 *
 * ## Threshold
 *
 * score < 0.2 → PASS (acceptable variance)
 * score >= 0.2 → FAIL (significant drift detected)
 */

import * as crypto from 'crypto';

// =============================================================================
// Types
// =============================================================================

export interface Baseline {
  /** Human-readable scenario name */
  name: string;

  /** Seed prompt (deterministic) */
  prompt: string;

  /** Expected number of tool call steps */
  expectedSteps: number;

  /** Expected tool names in order */
  expectedTools: string[];

  /** SHA-256 hash of final AST */
  expectedAstHash: string;

  /** Tolerance settings */
  tolerance: {
    /** Max acceptable step deviation */
    steps: number;
    /** Min required Jaccard similarity */
    tools: number;
    /** Whether AST must match exactly */
    ast: 'exact' | 'flexible';
  };
}

export interface ExecutionTrace {
  /** Actual steps taken */
  steps: number;

  /** Actual tool calls */
  tools: string[];

  /** Final .vs file content */
  finalAst: string;

  /** Exit code (0 = success) */
  exitCode: number;

  /** Total execution time (ms) */
  executionTimeMs: number;

  /** Any errors encountered */
  errors: string[];
}

export interface DriftReport {
  /** Scenario name */
  scenario: string;

  /** Individual drift scores */
  scores: {
    step: number;
    tool: number;
    ast: number;
  };

  /** Composite drift score */
  composite: number;

  /** Pass/fail threshold */
  threshold: number;

  /** Result */
  passed: boolean;

  /** Detailed comparison */
  details: {
    expectedSteps: number;
    actualSteps: number;
    expectedTools: string[];
    actualTools: string[];
    expectedAstHash: string;
    actualAstHash: string;
    toolsInCommon: string[];
    toolsMissing: string[];
    toolsExtra: string[];
  };
}

// =============================================================================
// Calculator
// =============================================================================

export class DriftCalculator {
  private readonly threshold: number;

  constructor(threshold: number = 0.2) {
    this.threshold = threshold;
  }

  /**
   * Calculate drift between baseline and actual execution.
   */
  calculate(baseline: Baseline, actual: ExecutionTrace): DriftReport {
    // Step drift: normalized deviation
    const stepDrift = Math.abs(actual.steps - baseline.expectedSteps) / baseline.expectedSteps;

    // Tool drift: 1 - Jaccard similarity
    const toolDrift = 1 - this.jaccardSimilarity(
      new Set(baseline.expectedTools),
      new Set(actual.tools),
    );

    // AST drift: binary (match or not)
    const actualAstHash = this.hashAst(actual.finalAst);
    const astDrift = actualAstHash === baseline.expectedAstHash ? 0 : 1;

    // Composite score (weighted average)
    const composite = 0.3 * stepDrift + 0.3 * toolDrift + 0.4 * astDrift;

    // Detailed comparison
    const expectedSet = new Set(baseline.expectedTools);
    const actualSet = new Set(actual.tools);
    const toolsInCommon = [...expectedSet].filter(t => actualSet.has(t));
    const toolsMissing = [...expectedSet].filter(t => !actualSet.has(t));
    const toolsExtra = [...actualSet].filter(t => !expectedSet.has(t));

    return {
      scenario: baseline.name,
      scores: {
        step: stepDrift,
        tool: toolDrift,
        ast: astDrift,
      },
      composite,
      threshold: this.threshold,
      passed: composite < this.threshold,
      details: {
        expectedSteps: baseline.expectedSteps,
        actualSteps: actual.steps,
        expectedTools: baseline.expectedTools,
        actualTools: actual.tools,
        expectedAstHash: baseline.expectedAstHash,
        actualAstHash,
        toolsInCommon,
        toolsMissing,
        toolsExtra,
      },
    };
  }

  /**
   * Jaccard similarity coefficient.
   */
  private jaccardSimilarity(a: Set<string>, b: Set<string>): number {
    if (a.size === 0 && b.size === 0) return 1;

    const intersection = new Set([...a].filter(x => b.has(x)));
    const union = new Set([...a, ...b]);

    return intersection.size / union.size;
  }

  /**
   * Hash AST content for comparison.
   *
   * Canonicalization steps:
   * 1. Parse JSON to object
   * 2. Remove non-semantic fields (timestamps, UUIDs, etc.)
   * 3. Sort object keys recursively
   * 4. Serialize with stable formatting
   */
  private hashAst(content: string): string {
    try {
      const parsed = JSON.parse(content);
      const canonical = this.canonicalizeAst(parsed);
      const serialized = JSON.stringify(canonical);
      return crypto.createHash('sha256').update(serialized).digest('hex');
    } catch {
      // Fallback for non-JSON content: normalize whitespace
      const normalized = content.trim().replace(/\s+/g, ' ');
      return crypto.createHash('sha256').update(normalized).digest('hex');
    }
  }

  /**
   * Recursively canonicalize AST for semantic comparison.
   */
  private canonicalizeAst(obj: unknown): unknown {
    if (obj === null || obj === undefined) return null;
    if (typeof obj !== 'object') {
      // Normalize floating point: 1.0 -> 1
      if (typeof obj === 'number') {
        return Number.isInteger(obj) ? obj : parseFloat(obj.toFixed(6));
      }
      return obj;
    }

    if (Array.isArray(obj)) {
      return obj.map(item => this.canonicalizeAst(item));
    }

    // Object: sort keys and filter non-semantic fields
    const NON_SEMANTIC_FIELDS = ['timestamp', 'createdAt', 'updatedAt', 'uuid', 'id'];
    const entries = Object.entries(obj as Record<string, unknown>)
      .filter(([key]) => !NON_SEMANTIC_FIELDS.includes(key))
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, value]) => [key, this.canonicalizeAst(value)]);

    return Object.fromEntries(entries);
  }

  /**
   * Generate a human-readable report.
   */
  formatReport(report: DriftReport): string {
    const lines = [
      `=== Drift Report: ${report.scenario} ===`,
      '',
      `Composite Score: ${(report.composite * 100).toFixed(1)}% (threshold: ${(report.threshold * 100).toFixed(1)}%)`,
      `Result: ${report.passed ? 'PASS' : 'FAIL'}`,
      '',
      'Individual Scores:',
      `  Step Drift:  ${(report.scores.step * 100).toFixed(1)}%`,
      `  Tool Drift:  ${(report.scores.tool * 100).toFixed(1)}%`,
      `  AST Drift:   ${(report.scores.ast * 100).toFixed(1)}%`,
      '',
      'Details:',
      `  Expected Steps: ${report.details.expectedSteps}`,
      `  Actual Steps:   ${report.details.actualSteps}`,
      `  Tools in Common: ${report.details.toolsInCommon.join(', ') || '(none)'}`,
      `  Tools Missing:   ${report.details.toolsMissing.join(', ') || '(none)'}`,
      `  Tools Extra:     ${report.details.toolsExtra.join(', ') || '(none)'}`,
      `  AST Hash Match:  ${report.scores.ast === 0 ? 'Yes' : 'No'}`,
    ];

    return lines.join('\n');
  }
}

// =============================================================================
// Export
// =============================================================================

export const defaultCalculator = new DriftCalculator(0.2);
