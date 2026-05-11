// =============================================================================
// ViewScript Standard Library: RoundedRect Component
// =============================================================================
//
// A rounded rectangle composed of 8 tangent points (where arcs meet edges),
// 4 corner arcs, and connecting line segments.
//
// ## Geometry
//
//            tl_top         tr_top
//                ●━━━━━━━━━━━━━━━━━━━━━●
//               ╱                       ╲
//   tl_left   ●       (arc_tl)       (arc_tr)●   tr_right
//             │                               │
//             │                               │
//             │                               │
//   bl_left   ●       (arc_bl)       (arc_br)●   br_right
//              ╲                             ╱
//                ●━━━━━━━━━━━━━━━━━━━━━●
//           bl_bottom              br_bottom
//
// ## Parameters
//   - x: X position of top-left corner (default: 0)
//   - y: Y position of top-left corner (default: 0)
//   - width: Width of rectangle (default: 100)
//   - height: Height of rectangle (default: 50)
//   - corner_radius: Default corner radius for all corners (default: 10)
//   - radius_tl: Top-left corner radius (overrides corner_radius)
//   - radius_tr: Top-right corner radius (overrides corner_radius)
//   - radius_br: Bottom-right corner radius (overrides corner_radius)
//   - radius_bl: Bottom-left corner radius (overrides corner_radius)
//
// ## Constraints
//   - Corner radius constraints are Soft (can be overridden)
//   - Edge alignment constraints are Hard (structural)
//
// ## Usage
//   import RoundedRect from "@viewscript/components/RoundedRect.vs"
//
//   const button = RoundedRect({ x: 10, y: 10, width: 120, height: 40 })
//   // Override top-right corner to be sharp (no rounding)
//   constraint button.radius_tr = 0
//

export component RoundedRect {
  // Position and size parameters
  param x: Rational = 0
  param y: Rational = 0
  param width: Rational = 100
  param height: Rational = 50

  // Corner radius parameters (Soft: can be overridden individually)
  param corner_radius: Rational = 10
  param radius_tl: Rational = corner_radius
  param radius_tr: Rational = corner_radius
  param radius_br: Rational = corner_radius
  param radius_bl: Rational = corner_radius

  // 8 tangent points where arcs meet straight edges
  controlpoint tl_top { role: anchor }
  controlpoint tl_left { role: anchor }
  controlpoint tr_top { role: anchor }
  controlpoint tr_right { role: anchor }
  controlpoint br_right { role: anchor }
  controlpoint br_bottom { role: anchor }
  controlpoint bl_bottom { role: anchor }
  controlpoint bl_left { role: anchor }

  // Top-left corner constraints
  constraint tl_top.x = x + radius_tl { priority: soft }
  constraint tl_top.y = y { priority: hard }
  constraint tl_left.x = x { priority: hard }
  constraint tl_left.y = y + radius_tl { priority: soft }

  // Top-right corner constraints
  constraint tr_top.x = x + width - radius_tr { priority: soft }
  constraint tr_top.y = y { priority: hard }
  constraint tr_right.x = x + width { priority: hard }
  constraint tr_right.y = y + radius_tr { priority: soft }

  // Bottom-right corner constraints
  constraint br_right.x = x + width { priority: hard }
  constraint br_right.y = y + height - radius_br { priority: soft }
  constraint br_bottom.x = x + width - radius_br { priority: soft }
  constraint br_bottom.y = y + height { priority: hard }

  // Bottom-left corner constraints
  constraint bl_bottom.x = x + radius_bl { priority: soft }
  constraint bl_bottom.y = y + height { priority: hard }
  constraint bl_left.x = x { priority: hard }
  constraint bl_left.y = y + height - radius_bl { priority: soft }

  // Edge alignment constraints (Hard: structural integrity)
  // Top edge is horizontal
  constraint tl_top.y = tr_top.y { priority: hard }
  // Bottom edge is horizontal
  constraint bl_bottom.y = br_bottom.y { priority: hard }
  // Left edge is vertical
  constraint tl_left.x = bl_left.x { priority: hard }
  // Right edge is vertical
  constraint tr_right.x = br_right.x { priority: hard }

  // Exported ports
  export tl_top
  export tl_left
  export tr_top
  export tr_right
  export br_right
  export br_bottom
  export bl_bottom
  export bl_left

  // Radius scalar exports (for overriding from parent)
  export radius_tl
  export radius_tr
  export radius_br
  export radius_bl
}
