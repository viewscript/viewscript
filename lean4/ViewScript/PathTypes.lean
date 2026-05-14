/-
  ViewScript: Path, Fill, and Stroke Type Definitions

  This module defines the core path and style types for ViewScript,
  mirroring the Rust implementation in `vsc-core/src/types.rs`.

  ## Design Notes

  - `EntityId` is imported from `PDimension` (defined in `rfc/lean/`).
    Here we re-declare a compatible alias for standalone use.
  - `Rational` is represented as Lean's built-in `Rat` (numerator / denominator
    in lowest terms), consistent with `ViewScriptRFC/PDimension.lean`.
  - Arc intrinsic parameters (`rx`, `ry`, `rotation`, `largeArc`, `sweep`) are
    stored directly in the segment — they are *not* constrainable entities,
    matching the Rust comment: "Arc parameters are intrinsic to the arc definition
    and not constrainable as entities."
-/

import Mathlib.Data.Rat.Basic

namespace ViewScript

/-! ## Entity Identifier -/

/-- Unique identifier for all ViewScript entities.
    Wraps a `Nat` (corresponding to `u64` in Rust). -/
structure EntityId where
  value : Nat
  deriving Repr, DecidableEq, Hashable, BEq

/-! ## Path Segments -/

/-- A single segment of a path.  Each variant references anchor and control
    points by `EntityId`, keeping geometry inside the constraint solver. -/
inductive PathSegment where
  /-- Straight line from `from` to `to`. -/
  | line (from to : EntityId)
  /-- Quadratic Bézier curve: one off-curve control handle. -/
  | quad (from handle to : EntityId)
  /-- Cubic Bézier curve: two off-curve control handles. -/
  | cubic (from handle1 handle2 to : EntityId)
  /-- Elliptical arc (SVG arc semantics).
      - `rx` / `ry` : semi-axis radii (exact rational).
      - `rotation`  : X-axis rotation in degrees (Float at rasterization boundary).
      - `largeArc`  : selects the large-arc solution when true.
      - `sweep`     : selects the clockwise-sweep solution when true. -/
  | arc (from to : EntityId) (rx ry : Rat) (rotation : Float) (largeArc sweep : Bool)
  deriving Repr

/-! ## Line Cap and Line Join -/

/-- Line cap style for stroked open paths (SVG/CSS standard).
    Determines how the endpoints of an open path are rendered. -/
inductive LineCap where
  /-- Flat edge precisely at the endpoint (default). -/
  | butt
  /-- Semicircular cap extending beyond the endpoint. -/
  | round
  /-- Square cap extending half the stroke width beyond the endpoint. -/
  | square
  deriving Repr, DecidableEq, BEq

/-- Line join style for stroked paths (SVG/CSS standard).
    Determines how corners are rendered where two segments meet. -/
inductive LineJoin where
  /-- Sharp mitered corner (default). -/
  | miter
  /-- Rounded corner. -/
  | round
  /-- Beveled (clipped) corner. -/
  | bevel
  deriving Repr, DecidableEq, BEq

/-! ## Fill Specification -/

/-- Fill specification for a path entity. -/
inductive FillSpec where
  /-- Solid color fill.  `color` is a CSS color string
      (e.g. `"#ff0000"`, `"rgb(255,0,0)"`). -/
  | solid (color : String)
  /-- Gradient fill.  `gradientId` must reference a
      `LinearGradient`, `RadialGradient`, or `ConicGradient` entity. -/
  | gradient (gradientId : EntityId)
  deriving Repr

/-! ## Stroke Specification -/

/-- Stroke specification for a path entity.
    All width values are exact rationals until rasterization. -/
structure StrokeSpec where
  /-- CSS color string. -/
  color    : String
  /-- Stroke width (exact rational). -/
  width    : Rat
  /-- Line cap style (default: `LineCap.butt`). -/
  lineCap  : LineCap  := LineCap.butt
  /-- Line join style (default: `LineJoin.miter`). -/
  lineJoin  : LineJoin       := LineJoin.miter
  /-- Dash pattern as a list of on/off lengths (exact rationals).
      `none` means a solid (unbroken) stroke. -/
  dashArray : Option (List Rat) := none
  deriving Repr

/-! ## Smart Constructors -/

/-- Create a `StrokeSpec` with default cap/join settings. -/
def StrokeSpec.mk' (color : String) (width : Rat) : StrokeSpec :=
  { color    := color
  , width    := width
  , lineCap  := LineCap.butt
  , lineJoin := LineJoin.miter }

end ViewScript
