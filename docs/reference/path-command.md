# PathCommand

SVG-compatible path commands with Rational coordinates.

## Variants

| Command | Fields | SVG Equivalent |
|:--------|:-------|:---------------|
| `MoveTo` | x, y | M |
| `LineTo` | x, y | L |
| `QuadTo` | x1, y1, x, y | Q |
| `CubicTo` | x1, y1, x2, y2, x, y | C |
| `ArcTo` | rx, ry, rotation, large_arc, sweep, x, y | A |
| `ClosePath` | — | Z |

## Coordinate Type

All coordinates are `Rational` except `ArcTo.rotation` which is `f64` (trigonometric input, intentional exception to Axiom 3).

## Related

- [PathSegment](path-segment.md) — Topology before resolution
- [Rational](rational.md) — Coordinate type
- [P-Dimension](../concepts/p-dimension.md) — Space where commands exist
