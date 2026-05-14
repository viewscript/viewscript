# StrokeSpec

Specifies the outline of a path entity.

## Definition

```rust
pub struct StrokeSpec {
    pub color: String,
    pub width: Rational,
    pub line_cap: LineCap,   // Butt | Round | Square
    pub line_join: LineJoin, // Miter | Round | Bevel
    pub dash_array: Option<Vec<Rational>>,
}
```

## CLI Format

```bash
vsc add-component -t rectangle -s "2:#000000"
#                                  ^  ^^^^^^^
#                                  |  color
#                                  width (px)
```

## Related

- [FillSpec](fill-spec.md) — Fill style
- [vsc add-component](../commands/add-component.md) — Sets stroke via `-s` flag
