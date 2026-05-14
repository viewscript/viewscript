# GPU Rendering Benchmark Baseline

Phase I: Loop-Blinn Fill + SDF Stroke implementation benchmark results.

**Test Environment**: 100 Bezier curve segments

## Phase I Summary

| Phase | Method | lyon (baseline) | Optimized | Speedup | Vertex Reduction |
|-------|--------|-----------------|-----------|---------|------------------|
| I-1 | Quadratic Loop-Blinn Fill | 1.96ms | 162µs | **12.1x** | ~4x |
| I-2 | Cubic Loop-Blinn Fill | 6.35ms | 184µs | **34.5x** | ~7x |
| I-3 | Quadratic SDF Stroke | 1.46ms | 43µs | **33.9x** | **8.0x** |
| I-4 | Cubic SDF Stroke | 3.21ms | 62µs | **51.8x** | **18.0x** |

## Detailed Results

### I-1: Quadratic Loop-Blinn Fill

CPU tessellation time comparison for quadratic Bezier fills:

| Curves | lyon | Loop-Blinn | Speedup |
|--------|------|------------|---------|
| 10 | 117µs | 22µs | 5.3x |
| 50 | 974µs | 80µs | 12.2x |
| 100 | 1.96ms | 162µs | 12.1x |
| 200 | 3.92ms | 323µs | 12.1x |

### I-2: Cubic Loop-Blinn Fill

CPU tessellation time comparison for cubic Bezier fills:

| Curves | lyon | Loop-Blinn | Speedup |
|--------|------|------------|---------|
| 10 | 441µs | 23µs | 19.2x |
| 50 | 2.20ms | 96µs | 22.9x |
| 100 | 6.35ms | 184µs | 34.5x |
| 200 | 19.8ms | 366µs | 54.1x |

### I-3: Quadratic SDF Stroke

CPU tessellation time comparison for quadratic Bezier strokes:

| Curves | lyon | SDF | Speedup | lyon Vertices | SDF Vertices | Reduction |
|--------|------|-----|---------|---------------|--------------|-----------|
| 10 | 107µs | 4.4µs | 24.3x | 322 | 40 | 8.05x |
| 50 | 728µs | 21µs | 34.7x | 1602 | 200 | 8.01x |
| 100 | 1.46ms | 43µs | 33.9x | 3202 | 400 | 8.01x |
| 200 | 2.91ms | 86µs | 33.8x | 6402 | 800 | 8.00x |

### I-4: Cubic SDF Stroke

CPU tessellation time comparison for cubic Bezier strokes:

| Curves | lyon | SDF | Speedup | lyon Vertices | SDF Vertices | Reduction |
|--------|------|-----|---------|---------------|--------------|-----------|
| 10 | 241µs | 6.2µs | 38.9x | 722 | 40 | 18.05x |
| 50 | 1.60ms | 31µs | 51.6x | 3602 | 200 | 18.01x |
| 100 | 3.21ms | 62µs | 51.8x | 7202 | 400 | 18.00x |
| 200 | 6.41ms | 124µs | 51.7x | 14402 | 800 | 18.00x |

## Theoretical Analysis

### Fill Rendering (Loop-Blinn)

- **Quadratic (I-1)**: 3 vertices per curve segment (triangle), GPU evaluates `u² - v` SDF
- **Cubic (I-2)**: 3-6 vertices per curve segment, GPU evaluates `k³ - l·m` SDF

### Stroke Rendering (SDF)

- **SDF Stroke**: O(N) with exactly 4N vertices, 6N indices (bounding quad per segment)
- **lyon Stroke**: O(N × W) where W depends on stroke width and tessellation tolerance

SDF stroke moves complexity from CPU tessellation to GPU fragment shader:
- **Quadratic (I-3)**: Cardano's formula for distance calculation
- **Cubic (I-4)**: Newton's method (5-point sampling + 4 iterations) for distance calculation

## Running Benchmarks

```bash
# Loop-Blinn fill benchmarks
cargo bench --bench loop_blinn_bench

# SDF stroke benchmarks
cargo bench --bench sdf_stroke_bench
```

---
*Generated: 2026-05-13*
