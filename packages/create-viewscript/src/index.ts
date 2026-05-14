#!/usr/bin/env node
/**
 * create-viewscript CLI
 *
 * Scaffolds a new ViewScript project with Vite.
 *
 * Usage:
 *   npm create viewscript
 *   pnpm create viewscript
 *   yarn create viewscript
 *
 * Headless mode (for CI):
 *   create-viewscript --name my-app --lang ts --ffi
 *   create-viewscript --name my-app --lang js --no-ffi
 */

import * as p from '@clack/prompts';
import { scaffold, type ScaffoldOptions } from './scaffold.js';
import path from 'node:path';
import fs from 'node:fs';

interface CliArgs {
  name?: string;
  lang?: 'ts' | 'js';
  ffi?: boolean;
  help?: boolean;
}

function parseArgs(argv: string[]): CliArgs {
  const args: CliArgs = {};
  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') {
      args.help = true;
    } else if (arg === '--name' && argv[i + 1]) {
      args.name = argv[++i];
    } else if (arg === '--lang' && argv[i + 1]) {
      const lang = argv[++i];
      if (lang === 'ts' || lang === 'js') {
        args.lang = lang;
      }
    } else if (arg === '--ffi') {
      args.ffi = true;
    } else if (arg === '--no-ffi') {
      args.ffi = false;
    }
  }
  return args;
}

function printHelp(): void {
  console.log(`
create-viewscript - Scaffold a new ViewScript project

Usage:
  npm create viewscript              Interactive mode
  create-viewscript [options]        Headless mode

Options:
  --name <name>    Project name (required for headless)
  --lang <ts|js>   Language: ts (TypeScript) or js (JavaScript)
  --ffi            Include FFI sample
  --no-ffi         Exclude FFI sample
  -h, --help       Show this help message

Examples:
  npm create viewscript
  create-viewscript --name my-app --lang ts --ffi
  create-viewscript --name my-app --lang js --no-ffi
`);
}

async function runHeadless(args: CliArgs): Promise<void> {
  const projectName = args.name!;
  const language = args.lang ?? 'ts';
  const includeFfiSample = args.ffi ?? true;
  const targetDir = path.resolve(process.cwd(), projectName);

  // Validate project name
  if (!/^[a-z0-9-_]+$/i.test(projectName)) {
    console.error('Error: Project name can only contain letters, numbers, hyphens, and underscores');
    process.exit(1);
  }

  console.log(`Creating ViewScript project: ${projectName}`);
  console.log(`  Language: ${language === 'ts' ? 'TypeScript' : 'JavaScript'}`);
  console.log(`  FFI sample: ${includeFfiSample ? 'yes' : 'no'}`);

  try {
    await scaffold({ projectName, targetDir, language, includeFfiSample });
    console.log(`Project created at ${targetDir}`);
  } catch (error) {
    console.error('Failed to create project:', error);
    process.exit(1);
  }
}

async function runInteractive(): Promise<void> {
  p.intro('Welcome to ViewScript');

  // Project name
  const projectName = await p.text({
    message: 'Project name:',
    placeholder: 'my-viewscript-app',
    defaultValue: 'my-viewscript-app',
    validate: (value) => {
      if (!value) return 'Project name is required';
      if (!/^[a-z0-9-_]+$/i.test(value)) {
        return 'Project name can only contain letters, numbers, hyphens, and underscores';
      }
      return undefined;
    },
  });

  if (p.isCancel(projectName)) {
    p.cancel('Operation cancelled.');
    process.exit(0);
  }

  // Check if directory exists
  const targetDir = path.resolve(process.cwd(), projectName);
  if (fs.existsSync(targetDir)) {
    const overwrite = await p.confirm({
      message: `Directory "${projectName}" already exists. Overwrite?`,
      initialValue: false,
    });

    if (p.isCancel(overwrite) || !overwrite) {
      p.cancel('Operation cancelled.');
      process.exit(0);
    }
  }

  // Language selection
  const language = await p.select({
    message: 'Language:',
    options: [
      { value: 'ts', label: 'TypeScript', hint: 'recommended' },
      { value: 'js', label: 'JavaScript' },
    ],
    initialValue: 'ts',
  });

  if (p.isCancel(language)) {
    p.cancel('Operation cancelled.');
    process.exit(0);
  }

  // FFI sample inclusion
  const includeFfiSample = await p.confirm({
    message: 'Include sample FFI function?',
    initialValue: true,
  });

  if (p.isCancel(includeFfiSample)) {
    p.cancel('Operation cancelled.');
    process.exit(0);
  }

  // Scaffold the project
  const spinner = p.spinner();
  spinner.start('Scaffolding project...');

  try {
    const options: ScaffoldOptions = {
      projectName,
      targetDir,
      language: language as 'ts' | 'js',
      includeFfiSample: includeFfiSample as boolean,
    };

    await scaffold(options);
    spinner.stop('Project created successfully!');
  } catch (error) {
    spinner.stop('Failed to create project');
    p.log.error(String(error));
    process.exit(1);
  }

  // Next steps
  p.note(
    [
      `cd ${projectName}`,
      'npm install',
      'npm run dev',
    ].join('\n'),
    'Next steps'
  );

  p.outro('Happy building!');
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv);

  if (args.help) {
    printHelp();
    process.exit(0);
  }

  // Headless mode if --name is provided
  if (args.name) {
    await runHeadless(args);
  } else {
    await runInteractive();
  }
}

main().catch((error) => {
  console.error('Unexpected error:', error);
  process.exit(1);
});
