# ViewScript CLI Reference

## Commands

- [vsc init](commands/init.md) — Initialize a new ViewScript project
- [vsc api-search](commands/api-search.md) — Search API functions by natural language query
- [vsc check-where](commands/check-where.md) — Check where an object can be placed (XY constraints)
- [vsc check-when](commands/check-when.md) — Check when a constraint is satisfied (T constraints)
- [vsc add-object](commands/add-object.md) — Add a new object to the scene
- [vsc add-constraint](commands/add-constraint.md) — Add a constraint between entities
- [vsc optimize](commands/optimize.md) — Optimize the IR by removing redundant constraints
- [vsc build](commands/build.md) — Build the project for a target renderer
- [vsc dev](commands/dev.md) — Start development server with live preview
- [vsc patch-constraint](commands/patch-constraint.md) — Modify an existing constraint on an entity
- [vsc add-layout](commands/add-layout.md) — Add a layout combinator (alias for apply-layout)
- [vsc add-entity](commands/add-entity.md) — Add a new entity to the constraint graph (Phase 10)
- [vsc add-component](commands/add-component.md) — Add a visual component (RoundedRect, Circle, etc.)
- [vsc update-metrics](commands/update-metrics.md) — Update text metrics from Renderer measurement (Phase 10 Q→P bridge)
- [vsc apply-layout](commands/apply-layout.md) — Apply a layout combinator to arrange instances (Phase 13)
- [vsc remove-constraint](commands/remove-constraint.md) — Remove a constraint or layout macro (Phase 13)
- [vsc export-schema](commands/export-schema.md) — Export OpenAPI schema for LLM agent initialization (Phase 14)
- [vsc generate-schema](commands/generate-schema.md) — Generate JSON Schema from Rust type definitions (D-16)
- [vsc status](commands/status.md) — Get current project status
- [vsc search](commands/search.md) — Search and query objects in the constraint graph
- [vsc check](commands/check.md) — Check constraint graph integrity (D-05)
- [vsc run-command](commands/run-command.md) — Run a CODL command file (Phase 15)
- [vsc target](commands/target.md) — Manage render targets
- [vsc style](commands/style.md) — Manage stylesheets (UA stylesheets like vs-style-chrome)
- [vsc help](commands/help.md) — Print this message or the help of the given subcommand(s)

## Concepts

- [P-Dimension](concepts/p-dimension.md) — Deterministic rational number space
- [Q-Dimension](concepts/q-dimension.md) — Non-deterministic oracle, QSnapshot, FFI
- [T-Dimension](concepts/t-dimension.md) — State vector, mutations
- [Constraint Solver](concepts/constraint-solver.md) — L0/L1/L2 pipeline
- [Scene Graph](concepts/scene-graph.md) — SceneNode, SceneBuilder
- [Render Target](concepts/render-target.md) — RenderTarget trait, vs-web
- [Box Model](concepts/box-model.md) — CSS Box Model constraints
- [Component](concepts/component.md) — Component system
- [CODL](concepts/codl.md) — Constraint Operation Description Language
- [FFI](concepts/ffi.md) — QDimensionProvider, C ABI, language bindings

## Reference

- [Rational](reference/rational.md) — Exact rational number type
- [EntityId](reference/entity-id.md) — Entity identifier
- [Constraint](reference/constraint.md) — Constraint definition
- [PathSegment](reference/path-segment.md) — Path topology segment
- [FillSpec](reference/fill-spec.md) — Fill style specification
- [StrokeSpec](reference/stroke-spec.md) — Stroke style specification
- [PathCommand](reference/path-command.md) — SVG-compatible path command
- [QValue](reference/q-value.md) — Q-dimension value types
- [QVariable](reference/q-variable.md) — Q-dimension variable binding
