//! Embedded WASM artifacts for distribution.
//!
//! This module contains pre-built WASM binary and JavaScript glue code
//! embedded at compile time using `include_bytes!` and `include_str!`.
//!
//! These artifacts are extracted to the output directory during `vsc build`
//! so that end users don't need the ViewScript source code or wasm-pack.
//!
//! ## Feature Flag
//!
//! This module requires the `embedded-wasm` feature to be enabled.
//! Without it, `vsc build` will require wasm-pack to be installed.

#[cfg(feature = "embedded-wasm")]
mod inner {
    /// Pre-built WASM binary (vsc_wasm_bg.wasm)
    pub static WASM_BINARY: &[u8] = include_bytes!("../../vsc-wasm/pkg/pkg/vsc_wasm_bg.wasm");

    /// JavaScript glue code (vsc_wasm.js)
    pub static WASM_JS: &str = include_str!("../../vsc-wasm/pkg/pkg/vsc_wasm.js");

    /// TypeScript definitions (vsc_wasm.d.ts)
    pub static WASM_DTS: &str = include_str!("../../vsc-wasm/pkg/pkg/vsc_wasm.d.ts");

    /// WASM TypeScript definitions (vsc_wasm_bg.wasm.d.ts)
    pub static WASM_BG_DTS: &str = include_str!("../../vsc-wasm/pkg/pkg/vsc_wasm_bg.wasm.d.ts");

    /// Default index.html template
    pub static INDEX_HTML: &str = include_str!("../../vsc-wasm/pkg/pkg/index.html");

    /// Package.json for the WASM module
    pub static PACKAGE_JSON: &str = include_str!("../../vsc-wasm/pkg/pkg/package.json");
}

#[cfg(feature = "embedded-wasm")]
pub use inner::*;

/// Check if embedded WASM is available.
pub const fn is_available() -> bool {
    cfg!(feature = "embedded-wasm")
}
