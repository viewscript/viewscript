# ViewScript Architectural Constraints

This document defines mandatory architectural constraints that MUST be enforced
by all ViewScript implementations. Violations of these constraints can cause
non-deterministic behavior, performance degradation, or correctness failures.

## Table of Contents

1. [Fourier-Motzkin Solver Limits](#fourier-motzkin-solver-limits)
2. [Text Metrics DAG Validation](#text-metrics-dag-validation)
3. [Occlusion Opacity Invariants](#occlusion-opacity-invariants)
4. [P/Q/T Dimension Boundaries](#pqt-dimension-boundaries)

---

## Fourier-Motzkin Solver Limits

### Background

ViewScript's P-dimension constraint solver uses Fourier-Motzkin (FM) elimination
for linear inequality solving. FM elimination has exponential worst-case complexity:
eliminating `n` variables from `m` constraints can produce up to `O(m^(2^n))` constraints.

### Mandatory Limits

```
CONSTRAINT_LIMIT_PER_ENTITY: 32
TOTAL_CONSTRAINT_LIMIT: 1024
MAX_ELIMINATION_DEPTH: 8
TIMEOUT_MS: 100
```

### Enforcement Points

1. **Parser/Validator**: Reject constraint additions that would exceed limits
2. **Solver**: Abort with error if elimination depth exceeds MAX_ELIMINATION_DEPTH
3. **CLI**: `vsc add-constraint` must check limits before applying

### Error Response

When limits are exceeded, the CLI MUST output:

```json
{
  "error_type": "constraint_limit_exceeded",
  "message": "Adding this constraint would exceed FM solver limits",
  "limits": {
    "current_count": 1020,
    "requested_addition": 8,
    "max_allowed": 1024
  },
  "suggestion": "Consider grouping constraints or using component boundaries"
}
```

### Rationale

Without limits, an LLM could inadvertently create a constraint system that:
- Takes minutes to solve (blocking render loop)
- Causes memory exhaustion (BigInt allocation for exact rationals)
- Produces uninterpretable error messages

---

## Text Metrics DAG Validation

### The Width-for-Height Problem

Text layout creates a bidirectional dependency between width and height:
- **Width affects Height**: Narrower containers cause more line wrapping
- **Height affects Width**: Multi-column layouts depend on available height

This creates a potential cycle in the constraint graph:

```
INVALID CYCLE:
  Text.width depends on Container.height
  Container.height depends on Text.height
  Text.height depends on Text.width (wrapping)

  Result: Text.width -> Container.height -> Text.height -> Text.width
```

### DAG Validation Rule

**All text metric dependencies MUST form a Directed Acyclic Graph (DAG).**

The validator MUST reject constraint additions that create cycles involving
text metrics entities.

### Validation Algorithm

```
function validateTextMetricsDAG(graph):
  textEntities = graph.entities.filter(e => e.hasTextMetrics)

  for each entity in textEntities:
    if hasCycle(entity, graph):
      return ValidationError("Text metrics cycle detected", cyclePath)

  return OK

function hasCycle(start, graph, visited = [], stack = []):
  if start in stack:
    return true  // Cycle found
  if start in visited:
    return false // Already validated this subgraph

  stack.push(start)

  for each dependency in graph.getDependencies(start):
    if hasCycle(dependency, graph, visited, stack):
      return true

  stack.pop()
  visited.push(start)
  return false
```

### Breaking Cycles

When a cycle is detected, the CLI suggests these resolution strategies:

1. **Fixed Dimension**: Make one dimension a constant (e.g., `text.width = 200`)
2. **Aspect Ratio**: Use a ratio constraint instead of bidirectional dependency
3. **Layout Phase**: Split into two-phase layout (measure then position)

### Error Response

```json
{
  "error_type": "text_metrics_cycle",
  "message": "Text metrics dependency cycle detected",
  "cycle_path": ["Text.width", "Container.height", "Text.height", "Text.width"],
  "suggestions": [
    {
      "action": "fix_dimension",
      "description": "Add constraint: Text.width = 200"
    },
    {
      "action": "use_aspect_ratio",
      "description": "Replace with: Text.height = Text.width * 1.5"
    }
  ]
}
```

---

## Occlusion Opacity Invariants

### The Invisible Click Problem

When a ViewScript entity has `opacity: 0` in the Canvas layer but `pointer-events: auto`
in the DOM layer, it becomes an "invisible button" - clickable but not visible.

This is SOMETIMES intentional (transparent hit regions) but USUALLY a bug.

### Validation Rules

#### Rule 1: Occlusion Check at Opacity Boundaries

When an entity's computed opacity crosses 0.5 (in either direction), the renderer
MUST check if any other entity is visually occluded.

```
function onOpacityChange(entity, oldOpacity, newOpacity):
  if (oldOpacity >= 0.5 && newOpacity < 0.5) ||
     (oldOpacity < 0.5 && newOpacity >= 0.5):

    occludedEntities = findOccludedEntities(entity)
    for each occluded in occludedEntities:
      if occluded.hasActivePointerEvents:
        emitWarning("Potentially invisible click target", occluded)
```

#### Rule 2: Z-Index Consistency

DOM proxy elements MUST have the same visual stacking order as their Canvas
counterparts. The renderer MUST maintain z-index synchronization.

```
INVARIANT: For all entity pairs (A, B):
  Canvas.zIndex(A) > Canvas.zIndex(B) <=> DOM.zIndex(A) > DOM.zIndex(B)
```

#### Rule 3: Hit Region Bounds

DOM proxy elements MUST NOT extend beyond their Canvas visual bounds.
This prevents "hit region overflow" where clicks register outside the
visible element.

```
INVARIANT: For all entities E:
  DOM.bounds(E) <= Canvas.bounds(E)
```

### Warning Output

```json
{
  "warning_type": "occlusion_opacity_mismatch",
  "entity_id": 42,
  "details": {
    "canvas_opacity": 0.1,
    "dom_pointer_events": "auto",
    "occluded_by": [15, 23],
    "recommendation": "Consider setting pointer-events: none or increasing opacity"
  }
}
```

---

## P/Q/T Dimension Boundaries

### The Ouroboros Prevention Principle

ViewScript separates concerns into three dimensions:

| Dimension | Purpose | Mutation Rules |
|-----------|---------|----------------|
| **P** | Spatial coordinates (X, Y, Z) | ONLY via constraint solver |
| **Q** | User input (mouse, keyboard) | Read-only (events consumed) |
| **T** | State vector (hover, scroll) | Q -> T allowed; P -> T forbidden |

### Critical Invariants

#### Invariant 1: Q Never Directly Mutates P

Q-dimension events (mouse coordinates) MUST NOT directly assign to P-dimension
coordinates. Instead, they mutate T-vector state, and P-dimension is derived
via constraint evaluation.

```
FORBIDDEN:
  onMouseMove(event):
    entity.x = event.clientX  // NEVER DO THIS

CORRECT:
  onMouseMove(event):
    entity.T.drag_progress = normalize(event.clientX, dragStart, dragEnd)

  // In constraints:
  entity.x = entity.T.drag_progress * 100 + offset
```

#### Invariant 2: P Never Mutates T

P-dimension constraint evaluation MUST NOT have side effects on T-vector state.
The T-vector is determined SOLELY by Q-dimension input.

```
FORBIDDEN (in constraint solver):
  if entity.x > threshold:
    entity.T.hover = 1  // NEVER DO THIS
```

#### Invariant 3: Float Decontamination

P-dimension values MUST be exact rationals (`Rational` type), never f64.
The ONLY place f64 is permitted is at the rasterization boundary.

```rust
// FORBIDDEN in P-dimension:
let x: f64 = 100.5;
entity.set_x(x);

// CORRECT:
let x: Rational = Rational::new(201, 2);  // Exact 100.5
entity.set_x(x);

// PERMITTED at rasterization:
let pixel_x: i32 = x.to_f64_for_rasterization().round() as i32;
```

### Enforcement

These invariants are enforced at compile time (Rust type system) and runtime
(TypeScript Proxy guards on DOM elements, T-vector state key restrictions).

---

## Document History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0 | 2026-05-10 | Architect | Initial document |

## References

- RFC: P-Dimension Axiomatization (`rfc/lean/ViewScriptRFC/PDimension.lean`)
- HMR Controller (`packages/dev-server/src/hmr-controller.ts`)
- Constraint Solver (`crates/vsc-core/src/solver.rs`)
- Event Backpressure (`packages/renderer/src/runtime/event-backpressure.ts`)
