//! Code Generation Module (Phase C)
//!
//! Compiles VsBuildInfo + solver output into standalone JavaScript code.
//!
//! ## Architecture
//!
//! ```text
//! VsBuildInfo + SolverResult
//!         │
//!         ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  codegen pipeline                                       │
//! │                                                         │
//! │  js_expr.rs     → ConstraintTerm → JS arithmetic expr   │
//! │  js_codegen.rs  → Complete module generation            │
//! └─────────────────────────────────────────────────────────┘
//!         │
//!         ▼
//! Standalone JavaScript (no WASM runtime)
//! ```
//!
//! ## Output Format
//!
//! Generated JS imports `@viewscript/gpu-runtime` for WebGPU drawing:
//!
//! ```javascript
//! import { initWebGPU, drawPath } from '@viewscript/gpu-runtime';
//!
//! // Q-dimension variables
//! let pointer_x = 0, pointer_y = 0;
//!
//! // P-dimension variables (constraint-resolved)
//! let bg_x = 100, bg_y = 80, ...;
//!
//! // Update chain (topologically sorted)
//! function update() {
//!   label_x = bg_x + bg_width / 2;
//!   // ...
//! }
//!
//! // Pre-tessellated mesh data
//! const bg_mesh = { ... };
//!
//! // Render function
//! function render(ctx) { ... }
//! ```

pub mod interactive;
pub mod js_codegen;
pub mod js_expr;

// Re-exports
pub use interactive::{DomElementKind, DomEventType, EventAction, EventBinding, InteractiveInfo};
pub use js_codegen::{generate_compiled_module, CycleError, GlyphData, TessellationOutput};
pub use js_expr::{rational_to_js, term_to_js_expr, VarNameMap};
