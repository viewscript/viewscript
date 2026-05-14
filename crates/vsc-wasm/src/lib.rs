//! ViewScript WASM Core
//!
//! WASI-P1 compatible WebAssembly module that provides the core
//! constraint engine functionality.
//!
//! ## Feature Flags
//!
//! - `gpu`: Enable GPU rendering via wgpu/WebGPU. Adds `WasmGpuRenderer`
//!   for browser-side rendering.

pub use vsc_core::*;

// =============================================================================
// GPU Module (enabled by "gpu" feature)
// =============================================================================

#[cfg(feature = "gpu")]
pub mod ffi_bridge;

#[cfg(feature = "gpu")]
pub mod gpu;

#[cfg(feature = "gpu")]
pub use ffi_bridge::{FfiManifest, PendingFfiCall, TickResult};

#[cfg(feature = "gpu")]
pub use gpu::WasmGpuRenderer;

#[cfg(feature = "gpu")]
pub use gpu::WasmViewScriptEngine;
// ViewScript WASM bindings for constraint-based graphics
