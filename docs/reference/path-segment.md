# PathSegment

Defines the topology of a path by connecting control points.

## Variants

| Variant | Control Points | Description |
|:--------|:--------------|:------------|
| `Line` | from, to | Straight line |
| `Quad` | from, handle, to | Quadratic Bézier |
| `Cubic` | from, handle1, handle2, to | Cubic Bézier |
| `Arc` | from, to + rx, ry, rotation | Elliptical arc |

## Relationship to PathCommand

`PathSegment` references `EntityId`s (topology). `PathCommand` contains resolved `Rational` coordinates (geometry). `resolve_path_commands()` converts segments to commands using solver solutions.

## Related

- [PathCommand](path-command.md) — Resolved coordinate commands
- [FillSpec](fill-spec.md) — Fill applied to paths
- [EntityId](entity-id.md) — Referenced by segments
