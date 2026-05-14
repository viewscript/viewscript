# Rational

Exact rational number type based on `num_rational::Ratio<BigInt>`. All P-dimension coordinates and dimensions are represented as `Rational` to eliminate IEEE 754 floating-point non-determinism.

## Definition

```rust
pub struct Rational(pub Ratio<BigInt>);
```

## Construction

```rust
Rational::new(numerator: i64, denominator: i64)
Rational::from_int(n: i64)
f32_to_rational_exact(v: f32)  // IEEE 754 bit-exact conversion
```

## Rasterization Boundary

`Rational` values are converted to `f64` only at the rasterization boundary via `to_f64_for_rasterization()`. This is the single point where precision loss may occur.

## Related

- [P-Dimension](../concepts/p-dimension.md) — The space where Rational coordinates live
- [EntityId](entity-id.md) — Identifies entities whose coordinates are Rational
- [Constraint](constraint.md) — Uses Rational for coefficients and constants
