# QVariable

Declares a Q-dimension input and binds it to a T-dimension solver variable.

## Definition

```rust
pub struct QVariable {
    pub name: String,        // "input.pointer.x"
    pub default: QValue,     // Fallback when host doesn't provide
    pub target_var: VarId,   // Solver variable to bind
}
```

## Standard Q-Variables

| Name | Type | EntityId | Purpose |
|:-----|:-----|:---------|:--------|
| `input.pointer.x` | Float | 0 | Pointer X coordinate |
| `input.pointer.y` | Float | 1 | Pointer Y coordinate |
| `input.pointer.pressed` | Bool | 2 | Pointer press state |
| `env.viewport.width` | Int | 3 | Viewport width |
| `env.viewport.height` | Int | 4 | Viewport height |

## Derived Q-Variables

Computed from P-dimension solutions and Q-values each frame.

```rust
pub struct DerivedQVariable {
    pub name: String,
    pub target_var: VarId,
    pub rule: DerivedRule,  // e.g., HitTest
}
```

## Related

- [QValue](q-value.md) — Value types
- [FFI](../concepts/ffi.md) — Host provides QValues
- [T-Dimension](../concepts/t-dimension.md) — Target of Q-variable binding
