/**
 * LLM Agent Executor
 *
 * This module executes LLM agents against ViewScript CLI and collects
 * execution traces for drift analysis.
 *
 * ## Determinism Strategy
 *
 * To ensure reproducible results:
 * 1. temperature = 0 (no sampling randomness)
 * 2. seed = fixed value (where supported)
 * 3. max_tokens = bounded to prevent runaway
 * 4. Timeout enforcement
 *
 * ## Supported LLMs
 *
 * - Claude 3.5 Sonnet (via Anthropic API)
 * - GPT-4o (via OpenAI API)
 */

import { spawn, type ChildProcess } from 'child_process';
import * as fs from 'fs/promises';
import * as path from 'path';
import { type Baseline, type ExecutionTrace } from './drift-calculator';

// =============================================================================
// Types
// =============================================================================

export interface ExecutorConfig {
  /** LLM provider */
  provider: 'anthropic' | 'openai';

  /** Model identifier */
  model: string;

  /** API key (from env) */
  apiKey: string;

  /** Maximum steps before abort */
  maxSteps: number;

  /** Timeout per step (ms) */
  stepTimeoutMs: number;

  /** Working directory for vsc commands */
  workDir: string;

  /** Path to vsc binary */
  vscPath: string;
}

interface ToolCall {
  name: string;
  arguments: Record<string, unknown>;
  result?: string;
  exitCode?: number;
}

// =============================================================================
// System Prompt
// =============================================================================

const SYSTEM_PROMPT = `# ViewScript Constraint Agent

You are an AI agent that manipulates GUI layouts through mathematical constraints.

## Core Rules

1. ALWAYS search before acting: Use vsc_api_search to find relevant commands
2. ALWAYS verify before mutating: Use vsc_check_* before vsc_add_*
3. If a command fails, read the error and apply the suggested fix
4. Complete the task in as few steps as possible

## Available Tools

- vsc_api_search: Find relevant constraint commands
- vsc_check_where_fit: Verify spatial constraints
- vsc_add_entity: Create a new visual element
- vsc_add_constraint: Add a relationship between elements
- vsc_status: Get current state (use focus_entity to limit scope)

## Output

When the task is complete, output DONE and stop.`;

// =============================================================================
// Executor
// =============================================================================

export class LLMExecutor {
  private config: ExecutorConfig;
  private trace: ToolCall[] = [];
  private errors: string[] = [];

  constructor(config: ExecutorConfig) {
    this.config = config;
  }

  /**
   * Execute a baseline scenario and collect trace.
   */
  async execute(baseline: Baseline): Promise<ExecutionTrace> {
    this.trace = [];
    this.errors = [];

    const startTime = Date.now();

    try {
      // Initialize working directory
      await this.initWorkDir();

      // Run agent loop
      let stepCount = 0;
      let done = false;

      while (!done && stepCount < this.config.maxSteps) {
        stepCount++;

        const response = await this.callLLM(baseline.prompt, this.trace);

        if (response.done) {
          done = true;
          break;
        }

        if (response.toolCall) {
          const result = await this.executeToolCall(response.toolCall);
          this.trace.push({
            ...response.toolCall,
            result: result.output,
            exitCode: result.exitCode,
          });

          if (result.exitCode !== 0) {
            this.errors.push(`${response.toolCall.name}: ${result.output}`);
          }
        }
      }

      // Read final AST
      const finalAst = await this.readFinalAst();

      return {
        steps: stepCount,
        tools: this.trace.map(t => t.name),
        finalAst,
        exitCode: done ? 0 : 1,
        executionTimeMs: Date.now() - startTime,
        errors: this.errors,
      };
    } catch (error) {
      return {
        steps: this.trace.length,
        tools: this.trace.map(t => t.name),
        finalAst: '',
        exitCode: 1,
        executionTimeMs: Date.now() - startTime,
        errors: [...this.errors, String(error)],
      };
    }
  }

  /**
   * Initialize clean working directory.
   */
  private async initWorkDir(): Promise<void> {
    await fs.rm(this.config.workDir, { recursive: true, force: true });
    await fs.mkdir(this.config.workDir, { recursive: true });

    // Initialize vsc project
    await this.runVsc(['init']);
  }

  /**
   * Call LLM API and get response.
   */
  private async callLLM(
    prompt: string,
    history: ToolCall[],
  ): Promise<{ done: boolean; toolCall?: ToolCall }> {
    // Build messages
    const messages = this.buildMessages(prompt, history);

    if (this.config.provider === 'anthropic') {
      return this.callAnthropic(messages);
    } else {
      return this.callOpenAI(messages);
    }
  }

  /**
   * Build message history for LLM.
   */
  private buildMessages(prompt: string, history: ToolCall[]): any[] {
    const messages: any[] = [
      { role: 'user', content: prompt },
    ];

    for (const call of history) {
      messages.push({
        role: 'assistant',
        content: null,
        tool_calls: [{
          id: `call_${messages.length}`,
          type: 'function',
          function: {
            name: call.name,
            arguments: JSON.stringify(call.arguments),
          },
        }],
      });
      messages.push({
        role: 'tool',
        tool_call_id: `call_${messages.length - 1}`,
        content: call.result ?? '',
      });
    }

    return messages;
  }

  /**
   * Call Anthropic Claude API.
   */
  private async callAnthropic(messages: any[]): Promise<{ done: boolean; toolCall?: ToolCall }> {
    const response = await fetch('https://api.anthropic.com/v1/messages', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'x-api-key': this.config.apiKey,
        'anthropic-version': '2023-06-01',
      },
      body: JSON.stringify({
        model: this.config.model,
        max_tokens: 1024,
        system: SYSTEM_PROMPT,
        messages,
        tools: this.getToolDefinitions(),
        temperature: 0,
      }),
    });

    const data = await response.json();

    if (data.stop_reason === 'end_turn') {
      const textContent = data.content?.find((c: any) => c.type === 'text');
      if (textContent?.text?.includes('DONE')) {
        return { done: true };
      }
    }

    const toolUse = data.content?.find((c: any) => c.type === 'tool_use');
    if (toolUse) {
      return {
        done: false,
        toolCall: {
          name: toolUse.name,
          arguments: toolUse.input,
        },
      };
    }

    return { done: true };
  }

  /**
   * Call OpenAI GPT API.
   */
  private async callOpenAI(messages: any[]): Promise<{ done: boolean; toolCall?: ToolCall }> {
    const response = await fetch('https://api.openai.com/v1/chat/completions', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${this.config.apiKey}`,
      },
      body: JSON.stringify({
        model: this.config.model,
        messages: [
          { role: 'system', content: SYSTEM_PROMPT },
          ...messages,
        ],
        tools: this.getToolDefinitions().map(t => ({ type: 'function', function: t })),
        temperature: 0,
        seed: 42,
      }),
    });

    const data = await response.json();
    const choice = data.choices?.[0];

    if (choice?.finish_reason === 'stop') {
      if (choice.message?.content?.includes('DONE')) {
        return { done: true };
      }
    }

    const toolCall = choice?.message?.tool_calls?.[0];
    if (toolCall) {
      return {
        done: false,
        toolCall: {
          name: toolCall.function.name,
          arguments: JSON.parse(toolCall.function.arguments),
        },
      };
    }

    return { done: true };
  }

  /**
   * Execute a tool call via vsc CLI.
   */
  private async executeToolCall(call: ToolCall): Promise<{ output: string; exitCode: number }> {
    const args = this.buildVscArgs(call);
    return this.runVsc(args);
  }

  /**
   * Build vsc CLI arguments from tool call.
   */
  private buildVscArgs(call: ToolCall): string[] {
    const args: string[] = [];

    switch (call.name) {
      case 'vsc_api_search':
        args.push('api-search', call.arguments.query as string);
        break;

      case 'vsc_check_where_fit':
        args.push('check-where-fit');
        args.push('--entity', String(call.arguments.entity));
        args.push('--container', String(call.arguments.container));
        break;

      case 'vsc_add_entity':
        args.push('add-entity');
        args.push('--type', String(call.arguments.type));
        args.push('--name', String(call.arguments.name));
        if (call.arguments.parent) {
          args.push('--parent', String(call.arguments.parent));
        }
        break;

      case 'vsc_add_constraint':
        args.push('add-constraint');
        args.push('--type', String(call.arguments.type));
        args.push('--target', String(call.arguments.target));
        if (call.arguments.reference) {
          args.push('--reference', String(call.arguments.reference));
        }
        break;

      case 'vsc_status':
        args.push('status', '--format=json');
        if (call.arguments.focus_entity) {
          args.push('--focus', String(call.arguments.focus_entity));
        }
        break;

      default:
        throw new Error(`Unknown tool: ${call.name}`);
    }

    return args;
  }

  /**
   * Run vsc CLI command.
   */
  private runVsc(args: string[]): Promise<{ output: string; exitCode: number }> {
    return new Promise((resolve) => {
      const proc = spawn(this.config.vscPath, args, {
        cwd: this.config.workDir,
        timeout: this.config.stepTimeoutMs,
      });

      let stdout = '';
      let stderr = '';

      proc.stdout?.on('data', (data) => { stdout += data; });
      proc.stderr?.on('data', (data) => { stderr += data; });

      proc.on('close', (code) => {
        resolve({
          output: stdout || stderr,
          exitCode: code ?? 1,
        });
      });

      proc.on('error', (err) => {
        resolve({
          output: err.message,
          exitCode: 1,
        });
      });
    });
  }

  /**
   * Read final .vs AST file.
   */
  private async readFinalAst(): Promise<string> {
    try {
      const vsFile = path.join(this.config.workDir, 'main.vs');
      return await fs.readFile(vsFile, 'utf-8');
    } catch {
      return '';
    }
  }

  /**
   * Tool definitions for LLM.
   */
  private getToolDefinitions(): any[] {
    return [
      {
        name: 'vsc_api_search',
        description: 'Search for relevant constraint commands',
        parameters: {
          type: 'object',
          properties: {
            query: { type: 'string', description: 'Natural language search query' },
          },
          required: ['query'],
        },
      },
      {
        name: 'vsc_check_where_fit',
        description: 'Check if entity fits in container',
        parameters: {
          type: 'object',
          properties: {
            entity: { type: 'string' },
            container: { type: 'string' },
          },
          required: ['entity', 'container'],
        },
      },
      {
        name: 'vsc_add_entity',
        description: 'Create a new visual element',
        parameters: {
          type: 'object',
          properties: {
            type: { type: 'string', enum: ['rect', 'text', 'image', 'group'] },
            name: { type: 'string' },
            parent: { type: 'string' },
          },
          required: ['type', 'name'],
        },
      },
      {
        name: 'vsc_add_constraint',
        description: 'Add a constraint between elements',
        parameters: {
          type: 'object',
          properties: {
            type: { type: 'string', enum: ['center', 'align', 'offset', 'stack'] },
            target: { type: 'string' },
            reference: { type: 'string' },
          },
          required: ['type', 'target'],
        },
      },
      {
        name: 'vsc_status',
        description: 'Get current constraint graph state',
        parameters: {
          type: 'object',
          properties: {
            focus_entity: { type: 'string' },
          },
        },
      },
    ];
  }
}
