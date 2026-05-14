# EntityId

Opaque identifier for entities in the constraint graph.

## Definition

```rust
pub struct EntityId(pub u64);
```

## ID Ranges

| Range | Purpose |
|:------|:--------|
| 0-5 | System Q-variables (pointer, viewport, DPR) |
| 6-999 | Reserved for system entities |
| 1000+ | User components (allocated by `next_entity_id`) |

## Related

- [Constraint](constraint.md) — References entities by EntityId
- [PathSegment](path-segment.md) — Connects entities by EntityId
- [vsc add-component](../commands/add-component.md) — Creates entities
- [vsc search](../commands/search.md) — Queries entities by ID
