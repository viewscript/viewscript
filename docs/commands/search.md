# vsc search

Search and query objects in the constraint graph.

## Usage

```
vsc search [OPTIONS]
```

## Options

```
  -t, --object-type <OBJECT_TYPE>    Object type filter: constraint, path, control-point, text, gradient, q-variable, derived, all
  -e, --entity-id <ENTITY_ID>        Filter by entity ID
  -c, --component <COMPONENT>        Filter by component (x, y, width, height, etc.)
  -w, --where-clause <WHERE_CLAUSE>  Constraint satisfaction filter (e.g., "x > 100", "width == 200")
  -l, --limit <LIMIT>                Maximum results to return [default: 100]
  -h, --help                         Print help
```

## Object Types

| Type | Description |
|:-----|:------------|
| `constraint` | Constraint operations |
| `path` | Path entities (SVG-like paths) |
| `control-point` | Gradient control points |
| `text` | Text entities |
| `gradient` | Linear and radial gradients |
| `all` | All object types (default) |

## Examples

```bash
vsc search                          # List all objects
vsc search -t constraint            # List only constraints
vsc search -e 1000                  # Objects related to entity 1000
vsc search -t constraint -c x       # X-component constraints only
vsc search -l 10                    # Limit to 10 results
```

## Output Format

```json
{
  "status": "success",
  "object_type": "all",
  "entity_id_filter": null,
  "component_filter": null,
  "count": 1,
  "limit": 100,
  "results": [
    {
      "type": "constraint",
      "id": 1000,
      "seq": 0,
      "target": 1000,
      "component": "X",
      "relation": "Eq",
      "term": "Const { value: 50/1 }",
      "intent": "RoundedRect component at (50, 100)",
      "timestamp": "2026-05-10T00:00:00Z"
    }
  ]
}
```

## Related

- [vsc add-component](add-component.md) — Creates entities to search
- [vsc status](status.md) — Summary counts
- [EntityId](../reference/entity-id.md) — Entity identifier
- [Index](../index.md)
