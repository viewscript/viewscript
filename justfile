# ViewScript Monorepo Task Runner
# ================================
#
# ## Test Pyramid Architecture
#
# ```
#                         ┌─────────────────┐
#                         │    E2E Tests    │  (WASI sandbox)
#                         │   Slowest, but  │
#                         │   most realistic│
#                         └────────┬────────┘
#                                  │
#                         ┌────────▼────────┐
#                         │  Integration    │  (CLI black-box)
#                         │  Tests          │
#                         └────────┬────────┘
#                                  │
#              ┌───────────────────┼───────────────────┐
#              │                   │                   │
#     ┌────────▼────────┐ ┌────────▼────────┐ ┌────────▼────────┐
#     │   Proptest      │ │   Unit Tests    │ │   LEAN Proofs   │
#     │   (Fuzzing)     │ │   (Rust/TS)     │ │   (Deductive)   │
#     └─────────────────┘ └─────────────────┘ └─────────────────┘
# ```
#
# ## CI Pipeline (Fail-Fast Order)
#
# ```
#   lean-verify-strict ──┬──▶ rust-unit ──┬──▶ rust-integration ──▶ wasi-e2e
#                        │                │
#                        └──▶ proptest ───┘
#                        │
#                        └──▶ ts-unit
# ```
#
# Fast tests run first. If unit tests fail, we don't waste time on slow E2E.
# Proptest runs in parallel with unit tests (both are independent).

set shell := ["bash", "-euo", "pipefail", "-c"]

# Environment detection
CI := env_var_or_default("CI", "false")

# Default: show available commands
default:
    @just --list

# =============================================================================
# LEAN 4 Tasks (RFC Proofs) - Layer 0: Deductive Foundation
# =============================================================================

# Verify LEAN proofs (STRICT mode: sorry = Error, Exit 1)
lean-verify-strict:
    @echo "LEAN verification (STRICT): sorry = Error"
    cd rfc/lean && lake build 2>&1 | tee /tmp/lean-output.txt
    @if grep -q "sorry" /tmp/lean-output.txt; then \
        echo "ERROR: Incomplete proofs (sorry) detected."; \
        exit 1; \
    fi

# Verify LEAN proofs (WARN mode: sorry = Warning, Exit 0)
lean-verify-warn:
    @echo "LEAN verification (WARN): sorry = Warning"
    cd rfc/lean && lake build || true
    @grep -rn "sorry" rfc/lean/ViewScriptRFC/*.lean 2>/dev/null || true

# Auto-select verification mode based on CI environment
lean-verify:
    @if [ "{{CI}}" = "true" ]; then just lean-verify-strict; else just lean-verify-warn; fi

# Count incomplete proofs
lean-sorry-count:
    @grep -rn "sorry" rfc/lean/ViewScriptRFC/*.lean 2>/dev/null | wc -l || echo "0"

lean-clean:
    cd rfc/lean && lake clean

# =============================================================================
# Rust Tasks - Layer 1: Core Implementation
# =============================================================================

rust-core: lean-verify
    cargo build -p vsc-core

rust-cli: rust-core
    cargo build -p vsc-cli

rust-wasm: rust-core
    cargo build -p vsc-wasm --target wasm32-wasip1

rust-all: rust-cli rust-wasm

rust-clean:
    cargo clean

# =============================================================================
# Unit Tests - Layer 2: Fast, Isolated Verification
# =============================================================================

# Rust unit tests (fast, no external dependencies)
rust-unit: lean-verify
    @echo "Running Rust unit tests..."
    cargo test --workspace --lib --bins -- --test-threads=4

# TypeScript unit tests
ts-unit: ts-install
    @echo "Running TypeScript unit tests..."
    pnpm --filter '*' test

# =============================================================================
# Property-Based Testing - Layer 2: Inductive Verification (Parallel)
# =============================================================================

# Run proptest (standard: 256 cases)
proptest: rust-core
    @echo "Running property-based tests (proptest)..."
    cargo test --package vsc-core --features proptest-tests -- proptest --test-threads=2

# Run extended proptest (CI: 10000 cases)
proptest-extended: rust-core
    @echo "Running extended property-based tests (10000 cases)..."
    PROPTEST_CASES=10000 cargo test --package vsc-core --features proptest-tests -- proptest

# List proptest regressions
proptest-regressions:
    @find . -name "*.proptest-regressions" -type f 2>/dev/null || echo "No regressions found"

# Promote proptest regressions to CLI integration tests
promote-regressions: rust-core
    @echo "Promoting proptest regressions to integration tests..."
    @cargo run --package vsc-core --bin promote-regressions 2>/dev/null || echo "No regressions to promote"

# =============================================================================
# Integration Tests - Layer 3: CLI Black-Box Behavioral Testing
# =============================================================================

# Run CLI integration tests (requires built binary)
rust-integration: rust-cli
    @echo "Running CLI integration tests..."
    cargo test --package vsc-cli --test '*' -- --test-threads=1

# Run integration tests with verbose output
rust-integration-verbose: rust-cli
    @echo "Running CLI integration tests (verbose)..."
    cargo test --package vsc-cli --test '*' -- --test-threads=1 --nocapture

# =============================================================================
# WASI E2E Tests - Layer 4: Full Sandbox Validation
# =============================================================================

# Run WASI E2E tests using wasmtime
wasi-e2e: rust-wasm
    @echo "Running WASI E2E tests..."
    @if command -v wasmtime &> /dev/null; then \
        ./tests/wasi-e2e/run_wasi_tests.sh; \
    else \
        echo "wasmtime not found, using Node.js runner..."; \
        cd tests/wasi-e2e && node --experimental-wasi-unstable-preview1 runner.mjs; \
    fi

# Run WASI E2E tests with Node.js only
wasi-e2e-node: rust-wasm ts-install
    @echo "Running WASI E2E tests (Node.js)..."
    cd tests/wasi-e2e && node --experimental-wasi-unstable-preview1 runner.mjs

# Run deterministic WASI tests (Zero-Flakiness Policy)
wasi-deterministic: rust-wasm
    @echo "Running deterministic WASI E2E tests..."
    ./tests/wasi-e2e/deterministic_runner.sh

# =============================================================================
# WASM Binding
# =============================================================================

wasm-bind: rust-wasm
    @echo "Binding WASM to platform CLI wrappers..."
    @mkdir -p dist/wasm
    cp target/wasm32-wasip1/debug/vsc_wasm.wasm dist/wasm/

# =============================================================================
# WASM Optimization (Phase 5: Binary Size Reduction)
# =============================================================================

# Size limit for WASM binary (35MB)
WASM_SIZE_LIMIT := "36700160"

# Build optimized WASM with release profile
wasm-release: lean-verify
    @echo "Building optimized WASM (release profile)..."
    cargo build -p vsc-wasm --target wasm32-wasip1 --release

# Optimize WASM binary with wasm-opt
wasm-optimize: wasm-release
    @echo "Running wasm-opt for size optimization..."
    @mkdir -p dist/wasm
    @if command -v wasm-opt &> /dev/null; then \
        wasm-opt -Oz -c --strip-debug --strip-producers \
            target/wasm32-wasip1/release/vsc_wasm.wasm \
            -o dist/wasm/vsc.wasm; \
        echo "Optimized binary: dist/wasm/vsc.wasm"; \
    else \
        echo "wasm-opt not found, copying unoptimized binary..."; \
        cp target/wasm32-wasip1/release/vsc_wasm.wasm dist/wasm/vsc.wasm; \
    fi

# Check WASM binary size against limit (CI gate)
wasm-size-check: wasm-optimize
    @echo "Checking WASM binary size..."
    @SIZE=$$(stat -c%s dist/wasm/vsc.wasm 2>/dev/null || stat -f%z dist/wasm/vsc.wasm); \
    echo "WASM size: $$SIZE bytes (limit: {{WASM_SIZE_LIMIT}} bytes = 35MB)"; \
    if [ "$$SIZE" -gt "{{WASM_SIZE_LIMIT}}" ]; then \
        echo "ERROR: WASM binary exceeds 35MB limit!"; \
        exit 1; \
    else \
        echo "PASS: WASM binary is within size limit."; \
    fi

# Full WASM pipeline: build, optimize, check
wasm-dist: wasm-size-check
    @echo "WASM distribution ready: dist/wasm/vsc.wasm"

# =============================================================================
# TypeScript Tasks
# =============================================================================

ts-install:
    pnpm install

ts-renderer: ts-install wasm-bind
    pnpm --filter @viewscript/renderer build

ts-browser: ts-install
    pnpm --filter @viewscript/browser-defaults build

ts-all: ts-renderer ts-browser

ts-clean:
    pnpm --filter '*' exec rm -rf dist

# =============================================================================
# Linting
# =============================================================================

rust-lint:
    cargo clippy --workspace -- -D warnings
    cargo fmt --check

ts-lint: ts-install
    pnpm --filter '*' lint

lint: rust-lint ts-lint static-analysis
    @echo "All lints passed."

# =============================================================================
# Static Analysis (vsc-linter) - Phase 3: Mathematical Integrity
# =============================================================================

# Build the static analysis linter
static-analysis-build:
    cargo build --package vsc-linter

# Run all static analysis checks on vsc-core
static-analysis: static-analysis-build
    @echo "Running static analysis for mathematical integrity..."
    cargo run --package vsc-linter --bin vsc-lint -- --all ./crates/vsc-core/src

# Run float contamination check only
static-analysis-float: static-analysis-build
    @echo "Checking for float contamination in P-dimension..."
    cargo run --package vsc-linter --bin vsc-lint -- --check float-contamination ./crates/vsc-core/src

# Run global state immutability check only
static-analysis-state: static-analysis-build
    @echo "Checking for global mutable state..."
    cargo run --package vsc-linter --bin vsc-lint -- --check global-state ./crates/vsc-core/src

# Run cycle detection logic verification only
static-analysis-cycle: static-analysis-build
    @echo "Verifying cycle detection algorithm structure..."
    cargo run --package vsc-linter --bin vsc-lint -- --check cycle-detection ./crates/vsc-core/src

# Run static analysis with JSON output (for CI parsing)
static-analysis-json: static-analysis-build
    cargo run --package vsc-linter --bin vsc-lint -- --all --format json ./crates/vsc-core/src

# Run static analysis in strict mode (warnings as errors)
static-analysis-strict: static-analysis-build
    @echo "Running static analysis (STRICT mode)..."
    cargo run --package vsc-linter --bin vsc-lint -- --all --warnings-as-errors ./crates/vsc-core/src

# Run vsc-linter unit tests
static-analysis-test:
    cargo test --package vsc-linter

# =============================================================================
# Combined Test Commands
# =============================================================================

# Full test pyramid (all layers)
test-all: rust-unit proptest ts-unit rust-integration wasi-e2e ts-e2e
    @echo "All tests passed (full pyramid)."

# Fast tests only (unit + proptest, skip integration/e2e)
test-fast: rust-unit proptest ts-unit
    @echo "Fast tests passed."

# Integration and E2E only (assumes unit tests passed)
test-slow: rust-integration wasi-e2e ts-e2e
    @echo "Slow tests passed."

# Alias for backwards compatibility
test: test-all

# =============================================================================
# Build Commands
# =============================================================================

# Full build
build: lean-verify rust-all wasm-bind ts-all
    @echo "Full build complete."

# Clean all
clean: lean-clean rust-clean ts-clean
    rm -rf dist

# =============================================================================
# Development Commands
# =============================================================================

# Fast dev build (skip LEAN)
dev-fast:
    cargo build -p vsc-cli
    @echo "Fast build complete (LEAN skipped)"

# Watch mode
dev-rust:
    cargo watch -x "build -p vsc-cli"

dev-ts:
    pnpm --filter @viewscript/renderer exec tsc --watch

# =============================================================================
# Release
# =============================================================================

release: lean-verify-strict
    cargo build --release -p vsc-cli
    cargo build --release -p vsc-wasm --target wasm32-wasip1

# =============================================================================
# TypeScript E2E Tests - Renderer Layer (Phase 3.5)
# =============================================================================

# Visual Regression: Bit-perfect Canvas rendering verification
# Uses CPU-only software rendering for determinism
ts-e2e-visual: ts-renderer
    @echo "Running visual regression tests (CPU rendering)..."
    cd packages/renderer && npx playwright test --project=visual-regression

# Bilayer Sync: Canvas-DOM layer coherence verification
# Tests that clicking visual bounds triggers correct DOM handlers
ts-e2e-sync: ts-renderer
    @echo "Running bilayer synchronization tests..."
    cd packages/renderer && npx playwright test --project=bilayer-sync

# Performance Profiling: Jank & backpressure verification
# Uses CDP to assert p99 frame time < 16.6ms, zero layout thrashing
ts-perf-profile: ts-renderer
    @echo "Running performance profile tests..."
    cd packages/renderer && npx playwright test --project=performance

# All TypeScript E2E tests
ts-e2e: ts-e2e-visual ts-e2e-sync ts-perf-profile
    @echo "All TypeScript E2E tests passed."

# Install Playwright browsers (run once)
ts-e2e-setup:
    cd packages/renderer && npx playwright install chromium

# =============================================================================
# CI Pipeline - Fail-Fast Optimized
# =============================================================================

# CI pipeline with fail-fast ordering
#
# Execution order (serial with early exit):
# 1. lean-verify-strict     - Fastest, foundational
# 2. rust-unit + proptest   - Parallel, fast
# 3. ts-unit                - Fast
# 4. rust-integration       - Medium
# 5. wasi-e2e               - Slowest
# 6. ts-e2e                 - Browser E2E (visual, sync, perf)
# 7. lint                   - Can run last (doesn't affect correctness)
#
# If any step fails, subsequent steps are skipped.

ci-unit: lean-verify-strict rust-unit ts-unit
    @echo "Unit tests passed."

ci-fuzz: proptest-extended
    @echo "Fuzzing passed."

ci-integration: rust-integration
    @echo "Integration tests passed."

ci-e2e: wasi-e2e
    @echo "WASI E2E tests passed."

ci-e2e-browser: ts-e2e
    @echo "Browser E2E tests passed."

ci-lint: lint
    @echo "Linting passed."

# Static analysis (mathematical integrity verification)
ci-static-analysis: static-analysis-strict
    @echo "Static analysis passed."

# Full CI pipeline (fail-fast, serial)
ci: ci-static-analysis ci-unit ci-fuzz ci-integration ci-e2e ci-e2e-browser ci-lint
    @echo "=========================================="
    @echo "CI Pipeline Complete"
    @echo "=========================================="

# Parallel CI (for faster execution on multi-core CI runners)
# Uses background jobs where possible
ci-parallel:
    @echo "Running CI in parallel mode..."
    just lean-verify-strict
    # Unit tests and fuzzing in parallel
    just rust-unit &
    just proptest-extended &
    just ts-unit &
    wait
    # Integration and E2E (serial, as they share resources)
    just rust-integration
    just wasi-e2e
    # Browser E2E (requires exclusive Playwright access)
    just ts-e2e
    just lint
    @echo "CI Pipeline Complete (parallel mode)"

# Pre-merge check (local, fast)
pre-merge: lean-verify-strict rust-unit proptest rust-integration
    @echo "Pre-merge checks passed."
