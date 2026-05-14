/**
 * Node.js WASI E2E Test Runner
 *
 * Alternative to the shell script for environments without wasmtime CLI.
 * Uses Node.js built-in WASI support (experimental).
 *
 * Usage: node --experimental-wasi-unstable-preview1 runner.mjs
 */

import { WASI } from 'wasi';
import { readFile, writeFile, mkdir, rm } from 'fs/promises';
import { tmpdir } from 'os';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
import { existsSync, openSync, closeSync, readFileSync, writeSync } from 'fs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const PROJECT_ROOT = join(__dirname, '../..');
const WASM_PATH = join(PROJECT_ROOT, 'target/wasm32-wasip1/debug/vsc_wasm.wasm');

// Test results
let testsRun = 0;
let testsPassed = 0;
let testsFailed = 0;

/**
 * Run vsc in WASI sandbox.
 *
 * stdout/stderr capture strategy:
 *   Node.js WASI accepts integer file descriptors for stdout/stderr via
 *   the constructor options { stdout: fd, stderr: fd }. We open two
 *   temporary files before constructing WASI, pass their fds, then read
 *   the files back after the WASM module has exited.
 */
async function runVscWasi(workdir, args) {
    const tmpBase = join(tmpdir(), `vsc-wasi-${Date.now()}-${Math.random().toString(36).slice(2)}`);
    const stdoutPath = `${tmpBase}.stdout`;
    const stderrPath = `${tmpBase}.stderr`;

    const stdoutFd = openSync(stdoutPath, 'w');
    const stderrFd = openSync(stderrPath, 'w');

    let stdout = '';
    let stderr = '';
    let exitCode = 0;

    try {
        const wasi = new WASI({
            version: 'preview1',
            args: ['vsc', ...args],
            env: {
                PWD: workdir,
            },
            preopens: {
                '.': workdir,
            },
            stdout: stdoutFd,
            stderr: stderrFd,
        });

        const wasmBuffer = await readFile(WASM_PATH);
        const wasmModule = await WebAssembly.compile(wasmBuffer);

        try {
            const instance = await WebAssembly.instantiate(wasmModule, {
                wasi_snapshot_preview1: wasi.wasiImport,
            });
            exitCode = wasi.start(instance) ?? 0;
        } catch (err) {
            exitCode = 1;
            // Write the JS-level error into the captured stderr file
            try {
                writeSync(stderrFd, `runner-error: ${err.message}\n`);
            } catch (_) { /* fd may already be closed on proc_exit */ }
        }
    } finally {
        // Close fds before reading so all buffered bytes are flushed
        try { closeSync(stdoutFd); } catch (_) { /* ignore */ }
        try { closeSync(stderrFd); } catch (_) { /* ignore */ }

        // Read captured output
        try { stdout = readFileSync(stdoutPath, 'utf8'); } catch (_) { /* empty */ }
        try { stderr = readFileSync(stderrPath, 'utf8'); } catch (_) { /* empty */ }

        // Clean up temp capture files
        try { await rm(stdoutPath, { force: true }); } catch (_) { /* ignore */ }
        try { await rm(stderrPath, { force: true }); } catch (_) { /* ignore */ }
    }

    return { stdout, stderr, exitCode };
}

/**
 * Create a temporary test directory.
 */
async function createTestDir() {
    const dir = join(tmpdir(), `vs-wasi-test-${Date.now()}-${Math.random().toString(36).slice(2)}`);
    await mkdir(dir, { recursive: true });
    return dir;
}

/**
 * Clean up test directory.
 */
async function cleanupTestDir(dir) {
    await rm(dir, { recursive: true, force: true });
}

/**
 * Test: Init creates files.
 */
async function testInitCreatesFiles() {
    console.log('[TEST] test_init_creates_files');
    testsRun++;

    const workdir = await createTestDir();
    try {
        await runVscWasi(workdir, ['init', '--name', 'test-wasi']);

        if (existsSync(join(workdir, 'vsconfig.json'))) {
            console.log('  ✓ vsconfig.json exists');
            testsPassed++;
            console.log('  PASSED');
        } else {
            console.log('  ✗ vsconfig.json missing');
            testsFailed++;
            console.log('  FAILED');
        }
    } finally {
        await cleanupTestDir(workdir);
    }
}

/**
 * Test: WASM module loads without panic.
 */
async function testWasmLoads() {
    console.log('[TEST] test_wasm_loads');
    testsRun++;

    try {
        const wasmBuffer = await readFile(WASM_PATH);
        const wasmModule = await WebAssembly.compile(wasmBuffer);

        console.log('  ✓ WASM module compiled successfully');
        testsPassed++;
        console.log('  PASSED');
    } catch (err) {
        console.log(`  ✗ WASM compilation failed: ${err.message}`);
        testsFailed++;
        console.log('  FAILED');
    }
}

/**
 * Main entry point.
 */
async function main() {
    console.log('==========================================');
    console.log('Node.js WASI E2E Test Suite');
    console.log('==========================================');

    // Check WASM binary exists
    if (!existsSync(WASM_PATH)) {
        console.error(`WASM binary not found: ${WASM_PATH}`);
        console.error('Build with: just rust-wasm');
        process.exit(1);
    }

    console.log(`Using WASM: ${WASM_PATH}`);
    console.log('');

    // Run tests
    await testWasmLoads();
    await testInitCreatesFiles();

    // Summary
    console.log('');
    console.log('==========================================');
    console.log('Test Summary');
    console.log('==========================================');
    console.log(`Total:  ${testsRun}`);
    console.log(`Passed: ${testsPassed}`);
    console.log(`Failed: ${testsFailed}`);

    process.exit(testsFailed > 0 ? 1 : 0);
}

main().catch(err => {
    console.error('Test runner error:', err);
    process.exit(1);
});
