# Constraint

A relation between entity variables in the P-dimension.

## Definition

```rust
pub struct Constraint {
    pub id: u64,
    pub target: EntityId,
    pub component: VectorComponent,
    pub relation: RelationType,       // Eq | Le | Ge | Lt | Gt
    pub term: ConstraintTerm,
    pub priority: ConstraintPriority, // Hard | Soft
    pub source_scope: Option<String>, // Debug info (e.g., "RoundedRect::inst_42")
}
```

## CLI Usage

```bash
vsc add-constraint <TARGET> <COMPONENT> <RELATION> <TERM> [--intent "..."]
vsc add-constraint 1000 x eq '{"type":"const","value":"200"}'
```

## Conflict Detection

When a conflicting constraint is added, the solver returns `SolverError::ConflictingConstraint` with the existing constraint details and repair suggestions.

## Related

- [vsc add-constraint](../commands/add-constraint.md) — Add constraints
- [vsc patch-constraint](../commands/patch-constraint.md) — Modify constraints
- [Constraint Solver](../concepts/constraint-solver.md) — How constraints are solved
- [EntityId](entity-id.md) — Target of constraints
