# QValue

A value from the Q-dimension (non-deterministic oracle).

## Variants

| Variant | Rust Type | Example |
|:--------|:----------|:--------|
| `Rational` | `Rational` | Exact numeric value |
| `Int` | `i64` | Integer sensor value |
| `Float` | `f64` | Continuous sensor value |
| `Bool` | `bool` | Binary state |
| `Bytes` | `Vec<u8>` | Font binary, image data |
| `Vec2` | `(Rational, Rational)` | 2D coordinate |
| `TextureHandle` | `TextureHandle` | GPU texture reference |
| `None` | — | Absent value |

## Conversion

All numeric variants convert to `Rational` via `to_rational()` before injection into the P-dimension solver.

## Related

- [QVariable](q-variable.md) — Binds QValue to solver variable
- [Q-Dimension](../concepts/q-dimension.md) — Architecture context
- [FFI](../concepts/ffi.md) — How QValues are provided by hosts
