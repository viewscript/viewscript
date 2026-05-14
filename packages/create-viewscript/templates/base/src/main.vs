// =============================================================================
// ViewScript Application Entry Point
// =============================================================================
//
// This is your main ViewScript file. Components defined here will be
// rendered to the canvas.

// A simple rounded rectangle component
component HelloBox {
  param x: Rational = 100
  param y: Rational = 100
  param width: Rational = 300
  param height: Rational = 150
  param radius: Rational = 16

  // Visual appearance
  fill: "#4a90d9"
  stroke: "#2d5a87"
  stroke_width: 2
}

// Instantiate the component
HelloBox hello {
  x: 50
  y: 50
  radius: 24
}
