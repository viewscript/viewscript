/**
 * ViewScript Counter Demo
 *
 * This file defines the counter UI using ViewScript's declarative syntax.
 *
 * STATUS: Design specification (parsing not yet implemented)
 *
 * When @viewscript/vite-plugin implements .vs parsing, this file will compile to:
 * - WebGPU mesh data (tessellated paths)
 * - DOM layer code (mountDOM, updateDOM, bindEvents)
 * - Unified mount() entry point
 *
 * Current workaround: app.ts manually implements the bilayer architecture.
 */

// =============================================================================
// Imports
// =============================================================================

// FFI binding to TypeScript/JavaScript logic
import { increment, decrement, getCount } from "./logic"

// =============================================================================
// Q-Dimension Bindings
// =============================================================================

// Bind counter state from JS logic to Q-dimension variable
q bind count = getCount()

// Viewport dimensions (injected by runtime)
q env viewport.width: Int
q env viewport.height: Int
q env viewport.dpr: Float

// Pointer input (injected by runtime)
q input pointer.x: Float
q input pointer.y: Float
q input pointer.pressed: Bool

// =============================================================================
// Layout Calculations
// =============================================================================

// Panel dimensions
const panelWidth = 320
const panelHeight = 200
const panelX = (viewport.width - panelWidth) / 2
const panelY = (viewport.height - panelHeight) / 2

// Button dimensions
const btnWidth = 80
const btnHeight = 48
const btnY = panelY + panelHeight - btnHeight - 24
const btnSpacing = 40

// =============================================================================
// Components
// =============================================================================

// Background panel
component Background: RoundedRect {
  x: panelX
  y: panelY
  width: panelWidth
  height: panelHeight
  radius: 16
  fill: "#1e1e2e"
}

// Counter display
component CounterLabel: Text {
  x: panelX + panelWidth / 2 - 20
  y: panelY + 50
  content: String(count)  // Reactive binding to Q-dimension
  font_family: "Inter"
  font_size: 72
  fill: "#cdd6f4"
}

// Decrement button
component DecrementButton: RoundedRect {
  x: panelX + btnSpacing
  y: btnY
  width: btnWidth
  height: btnHeight
  radius: 8
  fill: "#f38ba8"

  // Click handler -> FFI call
  on click {
    decrement()
  }
}

component DecrementLabel: Text {
  x: DecrementButton.x + btnWidth / 2 - 8
  y: btnY + 8
  content: "-"
  font_family: "Inter"
  font_size: 32
  fill: "#1e1e2e"
}

// Increment button
component IncrementButton: RoundedRect {
  x: panelX + panelWidth - btnSpacing - btnWidth
  y: btnY
  width: btnWidth
  height: btnHeight
  radius: 8
  fill: "#a6e3a1"

  // Click handler -> FFI call
  on click {
    increment()
  }
}

component IncrementLabel: Text {
  x: IncrementButton.x + btnWidth / 2 - 8
  y: btnY + 8
  content: "+"
  font_family: "Inter"
  font_size: 32
  fill: "#1e1e2e"
}

// =============================================================================
// Scene Root
// =============================================================================

scene {
  Background
  CounterLabel
  DecrementButton
  DecrementLabel
  IncrementButton
  IncrementLabel
}
