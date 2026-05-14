# FillSpec

Specifies how a path entity is filled.

## Variants

| Variant | Description | CLI Example |
|:--------|:------------|:------------|
| `Solid` | Single color | `-f "#ff6b6b"` |
| `Gradient` | References a gradient entity | `-f "linear-gradient(to right, #f00, #00f)"` |
| `ExternalTexture` | Q-dimension texture reference | Programmatic only |

## JSON Format

```json
{ "type": "solid", "color": "#ff6b6b" }
{ "type": "gradient", "gradient_id": 2000 }
{ "type": "external_texture", "handle_name": "resource.texture.1" }
```

## Related

- [StrokeSpec](stroke-spec.md) — Outline style
- [PathEntityEntry](path-segment.md) — Contains FillSpec
- [vsc add-component](../commands/add-component.md) — Sets fill via `-f` flag
- [Q-Dimension](../concepts/q-dimension.md) — ExternalTexture source
