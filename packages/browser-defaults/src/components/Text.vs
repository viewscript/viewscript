// =============================================================================
// ViewScript Standard Library: Text Component
// =============================================================================
//
// A text entity with 4 bounding box control points (TL, TR, BL, BR).
// This component is a macro that expands to 4 ControlPoints with structural
// constraints ensuring the bounding box remains rectangular.
//
// ## Parameters
//   - x: Initial X position of top-left corner (default: 0)
//   - y: Initial Y position of top-left corner (default: 0)
//   - content: Text content (string, default: "")
//   - font_family: Font family name (string, default: "sans-serif")
//   - font_size: Font size in P-dimension units (default: 16)
//
// ## Exports (Control Points)
//   - TL: Top-left corner
//   - TR: Top-right corner
//   - BL: Bottom-left corner
//   - BR: Bottom-right corner
//
// ## Constraints
//   - Width/height are set via `update-metrics` from Renderer measurement
//   - Structural alignment constraints are Hard (cannot be shadowed)
//   - Position constraints are Soft (can be overridden by parent scope)
//
// ## Usage
//   import Text from "@viewscript/components/Text.vs"
//
//   const label = Text({ x: 100, y: 50, content: "Hello, World!" })
//   // Access corners: label.TL, label.TR, label.BL, label.BR
//

export component Text {
  // Parameters with defaults
  param x: Rational = 0
  param y: Rational = 0
  param content: String = ""
  param font_family: String = "sans-serif"
  param font_size: Rational = 16

  // Control points (4 corners of bounding box)
  controlpoint TL { role: anchor }
  controlpoint TR { role: anchor }
  controlpoint BL { role: anchor }
  controlpoint BR { role: anchor }

  // Position constraints (Soft: can be overridden)
  constraint TL.x = x { priority: soft }
  constraint TL.y = y { priority: soft }

  // Structural constraints (Hard: cannot be shadowed)
  // Top edge horizontal
  constraint TR.y = TL.y { priority: hard }
  // Bottom edge horizontal
  constraint BR.y = BL.y { priority: hard }
  // Left edge vertical
  constraint BL.x = TL.x { priority: hard }
  // Right edge vertical
  constraint BR.x = TR.x { priority: hard }

  // Width/height constraints are added dynamically via update-metrics
  // These are Soft because the actual measured dimensions may differ
  // from any initial estimates.

  // Exported ports for external reference
  export TL
  export TR
  export BL
  export BR
}
