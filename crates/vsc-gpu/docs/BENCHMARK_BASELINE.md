# Loop-Blinn Optimization Benchmark Baseline

**Date**: 2026-05-12
**Phase**: I-1 (Quadratic Bezier Fill)
**Commit**: Post Phase I-1 implementation

## Test Environment

- CPU: (run `lscpu` for specifics)
- Rust: stable
- Profile: `--release` (criterion default)

## Benchmark: Synthetic QuadTo Path

Input: Wavy path with N quadratic Bezier curves, closed to form filled shape.

```
MoveTo(0, 100)
QuadTo(10, 150, 20, 100)   // curve 0
QuadTo(30, 50, 40, 100)    // curve 1
... (N curves)
LineTo(N*20, 0)
LineTo(0, 0)
Close
```

---

## Vertex Count Comparison

| Curves | Lyon Vertices | LB + Interior | Reduction | Ratio |
|--------|---------------|---------------|-----------|-------|
| 10     | 163           | 43            | 120       | 3.79x |
| 50     | 803           | 203           | 600       | 3.96x |
| 100    | 1,603         | 403           | 1,200     | 3.98x |
| 200    | 3,203         | 803           | 2,400     | 3.99x |
| 500    | 8,003         | 2,003         | 6,000     | 4.00x |

**Observation**: Consistent ~4x vertex reduction across all sizes.

---

## Triangle Count Comparison

| Curves | Lyon Triangles | LB + Interior | Reduction |
|--------|----------------|---------------|-----------|
| 10     | 161            | 21            | 140       |
| 50     | 801            | 101           | 700       |
| 100    | 1,601          | 201           | 1,400     |
| 200    | 3,201          | 401           | 2,800     |
| 500    | 8,001          | 1,001         | 7,000     |

**Observation**: ~8x triangle reduction for curve-heavy paths.

---

## CPU Tessellation Time

| Curves | Lyon Only | LB + Interior | Speedup |
|--------|-----------|---------------|---------|
| 10     | 146 µs    | 28 µs         | 5.2x    |
| 50     | 917 µs    | 115 µs        | 8.0x    |
| 100    | 2.19 ms   | 217 µs        | 10.1x   |
| 200    | 5.93 ms   | 431 µs        | 13.8x   |

**Observation**: Lyon scales super-linearly O(n log n) or worse; Loop-Blinn + interior scales linearly O(n).

---

## Scaling Analysis

```
Lyon tessellation time growth:
  10 → 50 curves:  6.3x time increase (5x input)
  50 → 100 curves: 2.4x time increase (2x input)
  100 → 200 curves: 2.7x time increase (2x input)

Loop-Blinn + interior growth:
  10 → 50 curves:  4.1x time increase (5x input)
  50 → 100 curves: 1.9x time increase (2x input)
  100 → 200 curves: 2.0x time increase (2x input)
```

Loop-Blinn maintains near-linear scaling while lyon exhibits super-linear growth.

---

## Real-World Implications

Japanese glyphs typically contain 50-100 curve segments per character.
For a 100-character text block:

| Metric | Lyon Only | Loop-Blinn |
|--------|-----------|------------|
| Triangles (100 glyphs × 80 curves) | ~640,000 | ~80,000 |
| Tessellation time | ~175 ms | ~17 ms |

---

## Reproduction

```bash
cargo bench -p vsc-gpu --bench loop_blinn_bench
```

---

## Future Comparisons

After implementing additional phases, re-run the same benchmark and compare:

- **I-2** (Cubic Bezier): Add `CubicTo` path variant to benchmark
- **I-3** (SDF Stroke): Add stroke benchmark (separate metric)
- **I-4** (Instanced Glyphs): Add glyph atlas benchmark

Record results in this file with date stamps for historical comparison.
