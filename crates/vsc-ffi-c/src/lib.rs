//! # vsc-ffi-c: C ABI Host Bridge for ViewScript
//!
//! This crate provides a C-compatible ABI for embedding the ViewScript constraint
//! engine in non-Rust host applications (C, C++, Go, Python, Node.js).
//!
//! ## Architecture Position
//!
//! This is the **host bridge** for headless targets. It provides:
//! - Q-dimension input via `vsc_engine_tick()` (QSnapshot JSON)
//! - P-dimension output via `vsc_engine_get_scene_json()` (SceneNode JSON)
//!
//! Rendering is the host's responsibility. For GPU-integrated targets
//! (vs-web, vs-winit), use the target-specific crates instead.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    Target Architecture                          │
//! ├─────────────────┬──────────────────┬────────────────────────────┤
//! │ Target          │ RenderTarget     │ HostBridge (Q-dimension)   │
//! ├─────────────────┼──────────────────┼────────────────────────────┤
//! │ vs-web          │ wgpu WebGPU      │ QSnapshot via JS callback  │
//! │ vs-winit        │ wgpu native      │ winit EventLoop            │
//! │ vs-tauri        │ wgpu via webview │ Tauri IPC                  │
//! │ vs-headless     │ (none)           │ vsc-ffi-c (this crate)     │
//! └─────────────────┴──────────────────┴────────────────────────────┘
//! ```
//!
//! ## Usage from C
//!
//! ```c
//! #include "viewscript.h"
//!
//! VscEngine* engine = vsc_engine_create();
//! const char* snapshot = "{\"values\":{}}";
//! int result = vsc_engine_tick(engine, snapshot, strlen(snapshot));
//!
//! char buf[4096];
//! int len = vsc_engine_get_scene_json(engine, buf, sizeof(buf));
//! if (len > 0) {
//!     // buf contains scene JSON
//! }
//!
//! vsc_engine_destroy(engine);
//! ```
//!
//! ## Error Codes
//!
//! - `0`: Success
//! - `-1`: Null pointer error
//! - `-2`: UTF-8 decoding error
//! - `-3`: JSON parsing error
//! - `-4`: Solver error
//! - Negative values < -100: Buffer too small (absolute value = required size)

use std::collections::HashMap;
use std::ffi::c_char;

use vsc_core::{
    evaluate_derived, ConstraintSolver, DerivedQVariable, QSnapshot, QValue, QVariable,
    SceneBuilder, SceneNode, VariableState, VsBuildInfo,
};

// =============================================================================
// Opaque Engine Type
// =============================================================================

/// Opaque handle to a ViewScript engine instance.
///
/// This struct is not repr(C) because it is only passed as an opaque pointer.
/// The C API only sees `*mut VscEngine` and never accesses fields directly.
pub struct VscEngine {
    solver: ConstraintSolver,
    build_info: VsBuildInfo,
    /// Cached scene from last tick (for get_scene_json).
    cached_scene: Vec<SceneNode>,
    /// Cached Q-values from last tick (for derived variable evaluation).
    cached_q_values: HashMap<String, QValue>,
}

impl VscEngine {
    /// Create a new engine with default state.
    fn new() -> Self {
        Self {
            solver: ConstraintSolver::new(),
            build_info: VsBuildInfo::default(),
            cached_scene: Vec::new(),
            cached_q_values: HashMap::new(),
        }
    }

    /// Process a tick with the given QSnapshot.
    ///
    /// Implements the same solve -> derive -> re-solve loop as WasmViewScriptEngine.
    fn tick_internal(&mut self, json_str: &str) -> Result<(), TickError> {
        // Parse QSnapshot
        let snapshot: QSnapshot =
            serde_json::from_str(json_str).map_err(|e| TickError::JsonParse(e.to_string()))?;

        // Phase 1: Q-dimension injection
        for q_var in &self.build_info.q_variables {
            if let Some(q_value) = snapshot.get(&q_var.name) {
                if let Some(rational) = q_value.to_rational() {
                    self.solver.register_variable(
                        q_var.target_var,
                        VariableState::Resolved { value: rational },
                    );
                }
            }
        }

        // Store Q-values for derived evaluation
        self.cached_q_values = snapshot.values.clone();

        // Phase 2: solve -> derive -> re-solve loop (max 2 iterations)
        const MAX_DERIVED_ITERATIONS: usize = 2;

        for iteration in 0..=MAX_DERIVED_ITERATIONS {
            // Solve constraints
            let solutions = self
                .solver
                .solve()
                .map_err(|e| TickError::Solver(format!("{:?}", e)))?;

            // Build scene
            let scene_nodes = SceneBuilder::new(&solutions, &self.build_info)
                .build_scene()
                .map_err(|e| TickError::SceneBuild(e.to_string()))?;

            // Check if we need to evaluate derived variables
            if iteration == MAX_DERIVED_ITERATIONS || self.build_info.derived_q_variables.is_empty()
            {
                self.cached_scene = scene_nodes;
                break;
            }

            // Evaluate derived Q-variables
            let mut any_changed = false;
            for derived in &self.build_info.derived_q_variables {
                let new_value =
                    evaluate_derived(&derived.rule, &self.cached_q_values, &scene_nodes);

                // Check if value changed
                let old_value = self.cached_q_values.get(&derived.name);
                let changed = match (old_value, &new_value) {
                    (Some(QValue::Float(old)), QValue::Float(new)) => (old - new).abs() > 1e-10,
                    (None, _) => true,
                    _ => true,
                };

                if changed {
                    any_changed = true;
                    self.cached_q_values
                        .insert(derived.name.clone(), new_value.clone());

                    // Re-inject into solver
                    if let Some(rational) = new_value.to_rational() {
                        self.solver.register_variable(
                            derived.target_var,
                            VariableState::Resolved { value: rational },
                        );
                    }
                }
            }

            // If nothing changed, we've converged
            if !any_changed {
                self.cached_scene = scene_nodes;
                break;
            }
        }

        Ok(())
    }

    /// Get the cached scene as JSON.
    fn get_scene_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.cached_scene)
    }

    /// Register a Q-variable.
    fn register_q_variable(&mut self, q_var: QVariable) {
        self.build_info.q_variables.push(q_var);
    }

    /// Register a derived Q-variable.
    fn register_derived_q_variable(&mut self, derived: DerivedQVariable) {
        self.build_info.derived_q_variables.push(derived);
    }
}

// =============================================================================
// Error Types
// =============================================================================

#[derive(Debug)]
enum TickError {
    JsonParse(String),
    Solver(String),
    SceneBuild(String),
}

// =============================================================================
// C ABI Functions
// =============================================================================

/// Create a new ViewScript engine instance.
///
/// # Returns
///
/// A pointer to a new engine instance. The caller is responsible for
/// calling `vsc_engine_destroy()` to free the memory.
///
/// # Safety
///
/// The returned pointer is valid until `vsc_engine_destroy()` is called.
#[no_mangle]
pub extern "C" fn vsc_engine_create() -> *mut VscEngine {
    let engine = Box::new(VscEngine::new());
    Box::into_raw(engine)
}

/// Process a single tick with the given QSnapshot JSON.
///
/// # Arguments
///
/// * `engine` - Pointer to the engine instance
/// * `snapshot_json` - Pointer to the JSON string (does not need to be null-terminated)
/// * `snapshot_len` - Length of the JSON string in bytes
///
/// # Returns
///
/// - `0`: Success
/// - `-1`: Null pointer (engine or snapshot_json is null)
/// - `-2`: Invalid UTF-8 in snapshot_json
/// - `-3`: JSON parsing error
/// - `-4`: Solver error
///
/// # Safety
///
/// * `engine` must be a valid pointer returned by `vsc_engine_create()`
/// * `snapshot_json` must point to valid memory of at least `snapshot_len` bytes
#[no_mangle]
pub unsafe extern "C" fn vsc_engine_tick(
    engine: *mut VscEngine,
    snapshot_json: *const c_char,
    snapshot_len: usize,
) -> i32 {
    // Null pointer checks
    if engine.is_null() || snapshot_json.is_null() {
        return -1;
    }

    let engine = &mut *engine;

    // Convert to Rust string
    let json_bytes = std::slice::from_raw_parts(snapshot_json as *const u8, snapshot_len);
    let json_str = match std::str::from_utf8(json_bytes) {
        Ok(s) => s,
        Err(_) => return -2,
    };

    // Process tick
    match engine.tick_internal(json_str) {
        Ok(()) => 0,
        Err(TickError::JsonParse(_)) => -3,
        Err(TickError::Solver(_)) | Err(TickError::SceneBuild(_)) => -4,
    }
}

/// Get the scene graph from the last tick as JSON.
///
/// # Arguments
///
/// * `engine` - Pointer to the engine instance
/// * `out_buf` - Buffer to write the JSON string into
/// * `buf_len` - Size of the buffer in bytes
///
/// # Returns
///
/// - Positive value: Number of bytes written (excluding null terminator)
/// - `-1`: Null pointer
/// - Negative value < -100: Buffer too small; absolute value minus 100 is the required size
///
/// # Safety
///
/// * `engine` must be a valid pointer returned by `vsc_engine_create()`
/// * `out_buf` must point to valid writable memory of at least `buf_len` bytes
#[no_mangle]
pub unsafe extern "C" fn vsc_engine_get_scene_json(
    engine: *mut VscEngine,
    out_buf: *mut c_char,
    buf_len: usize,
) -> i32 {
    // Null pointer checks
    if engine.is_null() || out_buf.is_null() {
        return -1;
    }

    let engine = &*engine;

    // Serialize scene to JSON
    let json = match engine.get_scene_json() {
        Ok(s) => s,
        Err(_) => return -4, // JSON serialization error
    };

    let json_bytes = json.as_bytes();
    let required_len = json_bytes.len() + 1; // +1 for null terminator

    // Check buffer size
    if buf_len < required_len {
        // Return negative required size (offset by 100 to distinguish from error codes)
        return -(100 + required_len as i32);
    }

    // Copy JSON to buffer
    let out_slice = std::slice::from_raw_parts_mut(out_buf as *mut u8, buf_len);
    out_slice[..json_bytes.len()].copy_from_slice(json_bytes);
    out_slice[json_bytes.len()] = 0; // Null terminator

    json_bytes.len() as i32
}

/// Add a component to the engine.
///
/// # Arguments
///
/// * `engine` - Pointer to the engine instance
/// * `component_type` - Null-terminated C string for component type (e.g., "Rectangle")
/// * `params_json` - Pointer to JSON parameters (does not need to be null-terminated)
/// * `params_len` - Length of the JSON parameters in bytes
///
/// # Returns
///
/// - Positive value: EntityId of the created component
/// - `-1`: Null pointer
/// - `-2`: Invalid UTF-8
/// - `-3`: JSON parsing error
/// - `-4`: Invalid component type
///
/// # Safety
///
/// * `engine` must be a valid pointer returned by `vsc_engine_create()`
/// * `component_type` must be a valid null-terminated C string
/// * `params_json` must point to valid memory of at least `params_len` bytes
#[no_mangle]
pub unsafe extern "C" fn vsc_engine_add_component(
    engine: *mut VscEngine,
    component_type: *const c_char,
    params_json: *const c_char,
    params_len: usize,
) -> i64 {
    // Null pointer checks
    if engine.is_null() || component_type.is_null() || params_json.is_null() {
        return -1;
    }

    let engine = &mut *engine;

    // Read component type (null-terminated)
    let type_cstr = std::ffi::CStr::from_ptr(component_type);
    let _type_str = match type_cstr.to_str() {
        Ok(s) => s,
        Err(_) => return -2,
    };

    // Read params JSON
    let params_bytes = std::slice::from_raw_parts(params_json as *const u8, params_len);
    let _params_str = match std::str::from_utf8(params_bytes) {
        Ok(s) => s,
        Err(_) => return -2,
    };

    // TODO: Implement component creation based on type
    // For now, return a placeholder entity ID
    let entity_id = engine.build_info.next_entity_id;
    engine.build_info.next_entity_id += 1;

    entity_id as i64
}

/// Register a Q-variable with the engine.
///
/// # Arguments
///
/// * `engine` - Pointer to the engine instance
/// * `q_var_json` - Pointer to JSON QVariable (does not need to be null-terminated)
/// * `q_var_len` - Length of the JSON in bytes
///
/// # Returns
///
/// - `0`: Success
/// - `-1`: Null pointer
/// - `-2`: Invalid UTF-8
/// - `-3`: JSON parsing error
///
/// # Safety
///
/// * `engine` must be a valid pointer returned by `vsc_engine_create()`
/// * `q_var_json` must point to valid memory of at least `q_var_len` bytes
#[no_mangle]
pub unsafe extern "C" fn vsc_engine_register_q_variable(
    engine: *mut VscEngine,
    q_var_json: *const c_char,
    q_var_len: usize,
) -> i32 {
    if engine.is_null() || q_var_json.is_null() {
        return -1;
    }

    let engine = &mut *engine;

    let json_bytes = std::slice::from_raw_parts(q_var_json as *const u8, q_var_len);
    let json_str = match std::str::from_utf8(json_bytes) {
        Ok(s) => s,
        Err(_) => return -2,
    };

    let q_var: QVariable = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return -3,
    };

    engine.register_q_variable(q_var);
    0
}

/// Free a ViewScript engine instance.
///
/// # Arguments
///
/// * `engine` - Pointer to the engine instance (may be null)
///
/// # Safety
///
/// * If `engine` is non-null, it must be a valid pointer returned by `vsc_engine_create()`
/// * After calling this function, the pointer is invalid and must not be used
#[no_mangle]
pub unsafe extern "C" fn vsc_engine_destroy(engine: *mut VscEngine) {
    if !engine.is_null() {
        drop(Box::from_raw(engine));
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vsc_engine_create_returns_non_null() {
        let engine = vsc_engine_create();
        assert!(
            !engine.is_null(),
            "vsc_engine_create should return non-null pointer"
        );
        unsafe {
            vsc_engine_destroy(engine);
        }
    }

    #[test]
    fn test_vsc_engine_tick_empty_snapshot_succeeds() {
        let engine = vsc_engine_create();
        let snapshot = r#"{"values":{}}"#;

        unsafe {
            let result =
                vsc_engine_tick(engine, snapshot.as_ptr() as *const c_char, snapshot.len());
            assert_eq!(result, 0, "tick with empty snapshot should succeed");
            vsc_engine_destroy(engine);
        }
    }

    #[test]
    fn test_vsc_engine_tick_null_engine_returns_minus_1() {
        let snapshot = r#"{"values":{}}"#;
        unsafe {
            let result = vsc_engine_tick(
                std::ptr::null_mut(),
                snapshot.as_ptr() as *const c_char,
                snapshot.len(),
            );
            assert_eq!(result, -1, "tick with null engine should return -1");
        }
    }

    #[test]
    fn test_vsc_engine_tick_null_snapshot_returns_minus_1() {
        let engine = vsc_engine_create();
        unsafe {
            let result = vsc_engine_tick(engine, std::ptr::null(), 0);
            assert_eq!(result, -1, "tick with null snapshot should return -1");
            vsc_engine_destroy(engine);
        }
    }

    #[test]
    fn test_vsc_engine_tick_invalid_utf8_returns_minus_2() {
        let engine = vsc_engine_create();
        // Invalid UTF-8 sequence
        let invalid_bytes: [u8; 4] = [0xFF, 0xFE, 0x00, 0x01];

        unsafe {
            let result = vsc_engine_tick(
                engine,
                invalid_bytes.as_ptr() as *const c_char,
                invalid_bytes.len(),
            );
            assert_eq!(result, -2, "tick with invalid UTF-8 should return -2");
            vsc_engine_destroy(engine);
        }
    }

    #[test]
    fn test_vsc_engine_tick_invalid_json_returns_minus_3() {
        let engine = vsc_engine_create();
        let invalid_json = "not valid json";

        unsafe {
            let result = vsc_engine_tick(
                engine,
                invalid_json.as_ptr() as *const c_char,
                invalid_json.len(),
            );
            assert_eq!(result, -3, "tick with invalid JSON should return -3");
            vsc_engine_destroy(engine);
        }
    }

    #[test]
    fn test_vsc_engine_destroy_null_no_panic() {
        // Should not panic
        unsafe {
            vsc_engine_destroy(std::ptr::null_mut());
        }
    }

    #[test]
    fn test_vsc_engine_get_scene_json_empty() {
        let engine = vsc_engine_create();
        let snapshot = r#"{"values":{}}"#;

        unsafe {
            // Run a tick first
            vsc_engine_tick(engine, snapshot.as_ptr() as *const c_char, snapshot.len());

            // Get scene JSON
            let mut buf = vec![0u8; 1024];
            let len = vsc_engine_get_scene_json(engine, buf.as_mut_ptr() as *mut c_char, buf.len());

            assert!(len >= 0, "get_scene_json should succeed");

            // Parse the result
            let json_str = std::str::from_utf8(&buf[..len as usize]).unwrap();
            let scene: Vec<SceneNode> = serde_json::from_str(json_str).unwrap();
            assert!(
                scene.is_empty(),
                "Empty build_info should produce empty scene"
            );

            vsc_engine_destroy(engine);
        }
    }

    #[test]
    fn test_vsc_engine_get_scene_json_buffer_too_small() {
        let engine = vsc_engine_create();
        let snapshot = r#"{"values":{}}"#;

        unsafe {
            vsc_engine_tick(engine, snapshot.as_ptr() as *const c_char, snapshot.len());

            // Tiny buffer
            let mut buf = [0u8; 1];
            let result =
                vsc_engine_get_scene_json(engine, buf.as_mut_ptr() as *mut c_char, buf.len());

            // Should return negative value indicating required size
            assert!(result < -100, "Should return negative required size");

            vsc_engine_destroy(engine);
        }
    }

    #[test]
    fn test_vsc_engine_add_component_basic() {
        let engine = vsc_engine_create();
        let component_type = b"Rectangle\0";
        let params = r#"{"x":0,"y":0,"width":100,"height":100}"#;

        unsafe {
            let entity_id = vsc_engine_add_component(
                engine,
                component_type.as_ptr() as *const c_char,
                params.as_ptr() as *const c_char,
                params.len(),
            );

            assert!(
                entity_id >= 0,
                "add_component should return positive entity_id"
            );

            vsc_engine_destroy(engine);
        }
    }

    #[test]
    fn test_vsc_engine_register_q_variable() {
        let engine = vsc_engine_create();
        // EntityId serializes as raw u64, VectorComponent uses lowercase
        let q_var_json = r#"{
            "name": "input.pointer.x",
            "default": {"type": "Float", "value": 0.0},
            "target_var": {"entity": 0, "component": "value"}
        }"#;

        unsafe {
            let result = vsc_engine_register_q_variable(
                engine,
                q_var_json.as_ptr() as *const c_char,
                q_var_json.len(),
            );

            assert_eq!(result, 0, "register_q_variable should succeed");

            vsc_engine_destroy(engine);
        }
    }
}
