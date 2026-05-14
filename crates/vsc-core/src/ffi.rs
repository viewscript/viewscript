//! Q-Dimension FFI Abstraction
//!
//! This module defines the interface for Q-dimension (external world) inputs
//! to flow into the P-dimension constraint system. All external values -
//! whether from user input, sensors, or static resources - are unified under
//! this abstraction.
//!
//! ## Design Principles
//!
//! 1. **P-dimension isolation**: The core solver knows nothing about JS, WASM,
//!    or native APIs. It only sees `QDimensionProvider` trait.
//!
//! 2. **Target-agnostic**: `vs-web` implements the provider with WebAPI,
//!    `vs-native` implements it with winit/OS APIs.
//!
//! 3. **Unified mutation path**: All T-vector mutations (hover, press, resize)
//!    flow through the same interface.
//!
//! ## Q-Variable Naming Convention
//!
//! - `input.pointer.x` - Pointer X coordinate
//! - `input.pointer.y` - Pointer Y coordinate
//! - `input.pointer.pressed` - Pointer button state
//! - `env.viewport.width` - Viewport width
//! - `env.viewport.height` - Viewport height
//! - `env.viewport.dpr` - Device pixel ratio
//! - `resource.font.*` - Font resources
//! - `style.defaults.*` - UA stylesheet values
//! - `resource.texture.*` - External texture handles

use crate::scene::{SceneBounds, SceneNode};
use crate::solver::VarId;
use crate::types::{EntityId, Rational};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// =============================================================================
// Q-Value: Values from the Q-Dimension
// =============================================================================

/// A value from the Q-dimension (external world).
///
/// Q-values represent inputs from outside the P-dimension constraint system.
/// They are converted to P-dimension types (Rational) when bound to T-variables.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "value")]
pub enum QValue {
    /// Exact rational value (preferred for P-dimension binding).
    Rational(Rational),

    /// Integer value (will be converted to Rational).
    Int(i64),

    /// Floating-point value from sensors/external APIs.
    /// WARNING: Only use before rasterization boundary. Convert to Rational
    /// for any P-dimension operations.
    Float(f64),

    /// Boolean state (e.g., pointer pressed, key held).
    Bool(bool),

    /// Binary data (e.g., font files, images).
    /// Serialized as base64 string.
    Bytes(QBytes),

    /// 2D vector (e.g., pointer position, viewport size).
    Vec2(Rational, Rational),

    /// External texture handle from host.
    ///
    /// The handle is opaque to P-dimension - it references a texture
    /// managed by the host (OffscreenCanvas, video element, native texture).
    /// Target-specific binding occurs at render time.
    TextureHandle(TextureHandle),

    /// No value / undefined.
    None,
}

/// Wrapper for binary data with base64 serialization.
#[derive(Debug, Clone, JsonSchema)]
pub struct QBytes(pub Vec<u8>);

impl Serialize for QBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&self.0);
        serializer.serialize_str(&encoded)
    }
}

impl<'de> Deserialize<'de> for QBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use base64::Engine;
        let encoded = String::deserialize(deserializer)?;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .map_err(serde::de::Error::custom)?;
        Ok(QBytes(decoded))
    }
}

// =============================================================================
// External Texture Handle
// =============================================================================

/// Handle to an external texture managed by the host.
///
/// The handle is opaque to the P-dimension constraint system. The actual
/// texture binding is performed by the target-specific renderer:
///
/// - **vs-web**: `importExternalTexture()` for video, `copyExternalImageToTexture()` for Canvas/ImageBitmap
/// - **vs-winit**: `queue.write_texture()` (pixel copy until wgpu supports shared memory)
/// - **vs-headless**: Handle reference only (no GPU binding)
///
/// ## JSON Format
///
/// ```json
/// {
///   "id": 12345,
///   "width": 1920,
///   "height": 1080,
///   "format": "rgba8unorm"
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextureHandle {
    /// Opaque identifier from the host.
    ///
    /// For vs-web, this could be an index into a JS-side texture registry.
    /// For native targets, this could be a memory address or HAL handle.
    pub id: u64,

    /// Texture width in pixels.
    pub width: u32,

    /// Texture height in pixels.
    pub height: u32,

    /// Pixel format of the texture.
    pub format: TextureFormat,
}

impl TextureHandle {
    /// Create a new texture handle.
    pub fn new(id: u64, width: u32, height: u32, format: TextureFormat) -> Self {
        Self {
            id,
            width,
            height,
            format,
        }
    }
}

/// Pixel format for external textures.
///
/// Maps to wgpu::TextureFormat for GPU-integrated targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TextureFormat {
    /// 8-bit RGBA, normalized unsigned (most common).
    Rgba8Unorm,
    /// 8-bit BGRA, normalized unsigned (Windows/Metal native).
    Bgra8Unorm,
    /// 16-bit RGBA, floating point (HDR).
    Rgba16Float,
    /// 32-bit RGBA, floating point (high precision).
    Rgba32Float,
}

impl Default for TextureFormat {
    fn default() -> Self {
        TextureFormat::Rgba8Unorm
    }
}

impl QValue {
    /// Convert to Rational if possible.
    ///
    /// Returns `None` for non-numeric types (Bool, Bytes, TextureHandle, None).
    pub fn to_rational(&self) -> Option<Rational> {
        match self {
            QValue::Rational(r) => Some(r.clone()),
            QValue::Int(i) => Some(Rational::from_int(*i)),
            QValue::Float(f) => Some(crate::types::f64_to_rational(*f)),
            QValue::Bool(_) => None,
            QValue::Bytes(_) => None,
            QValue::Vec2(_, _) => None,
            QValue::TextureHandle(_) => None,
            QValue::None => None,
        }
    }

    /// Extract as Rational, panicking if not numeric.
    pub fn as_rational(&self) -> Option<&Rational> {
        match self {
            QValue::Rational(r) => Some(r),
            _ => None,
        }
    }

    /// Extract as bool.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            QValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Extract as bytes.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            QValue::Bytes(b) => Some(&b.0),
            _ => None,
        }
    }

    /// Extract as Vec2.
    pub fn as_vec2(&self) -> Option<(&Rational, &Rational)> {
        match self {
            QValue::Vec2(x, y) => Some((x, y)),
            _ => None,
        }
    }

    /// Extract as TextureHandle.
    pub fn as_texture_handle(&self) -> Option<&TextureHandle> {
        match self {
            QValue::TextureHandle(h) => Some(h),
            _ => None,
        }
    }

    /// Check if the value is None.
    pub fn is_none(&self) -> bool {
        matches!(self, QValue::None)
    }
}

// =============================================================================
// Q-Variable Declaration
// =============================================================================

/// Declaration of a Q-dimension variable.
///
/// P-dimension code declares what external inputs it expects. The target
/// (vs-web, vs-native) then provides implementations via `QDimensionProvider`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QVariable {
    /// Hierarchical name (e.g., "input.pointer.x", "env.viewport.width").
    pub name: String,

    /// Default value when the Q-dimension provider doesn't supply a value.
    pub default: QValue,

    /// T-dimension variable to bind this Q-value to.
    pub target_var: VarId,
}

impl QVariable {
    /// Create a new Q-variable declaration.
    pub fn new(name: impl Into<String>, default: QValue, target_var: VarId) -> Self {
        Self {
            name: name.into(),
            default,
            target_var,
        }
    }
}

// =============================================================================
// Q-Dimension Provider Trait
// =============================================================================

/// Provider for Q-dimension values.
///
/// This trait is implemented by targets (vs-web, vs-native) to supply
/// external values to the P-dimension constraint system.
///
/// ## Implementation Notes
///
/// - `get()` is for polling-style inputs (viewport size, DPR)
/// - `poll_events()` is for event-style inputs (pointer move, key press)
/// - Implementations should buffer events between `tick()` calls
pub trait QDimensionProvider: Send {
    /// Get current value of a Q-variable (polling style).
    ///
    /// Returns the current value, or `QError::UnknownVariable` if not provided.
    fn get(&self, name: &str) -> Result<QValue, QError>;

    /// Poll buffered events for an event-style Q-variable.
    ///
    /// Returns all events since the last poll, clearing the buffer.
    /// Returns empty vec if no events, or error if variable is unknown.
    fn poll_events(&self, name: &str) -> Result<Vec<QValue>, QError>;

    /// List all Q-variables this provider can supply.
    fn available(&self) -> Vec<String>;
}

// =============================================================================
// Error Types
// =============================================================================

/// Errors from Q-dimension operations.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize, JsonSchema)]
pub enum QError {
    /// The requested Q-variable is not known to the provider.
    #[error("unknown Q variable: {0}")]
    UnknownVariable(String),

    /// The provider encountered an error fetching the value.
    #[error("provider error: {0}")]
    ProviderError(String),

    /// Type mismatch when extracting Q-value.
    #[error("type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },
}

// =============================================================================
// Q-Snapshot: Frame-level Q-dimension State
// =============================================================================

/// Snapshot of Q-dimension values for a single frame.
///
/// JS provides this snapshot each tick, containing all Q-variable values
/// for the current frame. This avoids repeated JS→WASM→JS calls.
///
/// ## JSON Format
///
/// ```json
/// {
///   "values": {
///     "input.pointer.x": { "type": "Float", "value": 100.5 },
///     "input.pointer.y": { "type": "Float", "value": 200.0 },
///     "input.pointer.pressed": { "type": "Bool", "value": true },
///     "env.viewport.width": { "type": "Int", "value": 1920 }
///   },
///   "mutations": [
///     { "type": "SetPosition", "entity_id": 1000, "x": 100, "y": 200 }
///   ]
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QSnapshot {
    /// Q-variable name → value mapping for this frame.
    pub values: HashMap<String, QValue>,

    /// Optional legacy mutations to apply alongside Q-values.
    /// This allows combining Q-dimension updates with entity-specific operations.
    #[serde(default)]
    pub mutations: Vec<serde_json::Value>,
}

impl QSnapshot {
    /// Create an empty snapshot.
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            mutations: Vec::new(),
        }
    }

    /// Create a snapshot with initial values.
    pub fn with_values(values: HashMap<String, QValue>) -> Self {
        Self {
            values,
            mutations: Vec::new(),
        }
    }

    /// Get a Q-value by name.
    pub fn get(&self, name: &str) -> Option<&QValue> {
        self.values.get(name)
    }

    /// Insert a Q-value.
    pub fn insert(&mut self, name: impl Into<String>, value: QValue) {
        self.values.insert(name.into(), value);
    }
}

impl Default for QSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Derived Q-Variables: Computed from P-dimension + Q-dimension
// =============================================================================

/// A derived Q-variable whose value is computed from a rule.
///
/// Unlike regular Q-variables that receive values from external sources,
/// derived Q-variables compute their values from:
/// - Other Q-variables (e.g., pointer position)
/// - Resolved P-dimension values (e.g., entity bounds)
///
/// The canonical use case is hover detection:
/// ```text
/// T.hover = HitTest(pointer_x, pointer_y, entity_bounds)
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DerivedQVariable {
    /// Hierarchical name (e.g., "component.1000.hover").
    pub name: String,

    /// T-dimension variable to bind the derived value to.
    pub target_var: VarId,

    /// Rule for computing the derived value.
    pub rule: DerivedRule,
}

impl DerivedQVariable {
    /// Create a new derived Q-variable.
    pub fn new(name: impl Into<String>, target_var: VarId, rule: DerivedRule) -> Self {
        Self {
            name: name.into(),
            target_var,
            rule,
        }
    }

    /// Create a hover-detecting derived variable for a component.
    pub fn hover(component_id: u64, target_var: VarId, entity_id: EntityId) -> Self {
        Self {
            name: format!("component.{}.hover", component_id),
            target_var,
            rule: DerivedRule::HitTest {
                pointer_x: "input.pointer.x".to_string(),
                pointer_y: "input.pointer.y".to_string(),
                entity_id,
            },
        }
    }
}

/// Rule for computing a derived Q-variable's value.
///
/// Rules are evaluated after P-dimension constraints are solved,
/// using both Q-values and resolved P-dimension values.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DerivedRule {
    /// Hit test: is the pointer inside the entity's bounding box?
    ///
    /// Returns `QValue::Float(1.0)` if inside, `QValue::Float(0.0)` if outside.
    /// Uses Q-variables for pointer position and scene nodes for entity bounds.
    HitTest {
        /// Q-variable name for pointer X coordinate.
        pointer_x: String,
        /// Q-variable name for pointer Y coordinate.
        pointer_y: String,
        /// Entity ID whose bounding box to test against.
        entity_id: EntityId,
    },
    // Future extensions:
    // /// Threshold: returns 1.0 if source > threshold, else 0.0
    // Threshold {
    //     source: String,
    //     threshold: Rational,
    // },
    // /// Logical AND of two rules
    // And {
    //     left: Box<DerivedRule>,
    //     right: Box<DerivedRule>,
    // },
}

// =============================================================================
// Derived Rule Evaluation
// =============================================================================

/// Evaluate a derived rule to produce a Q-value.
///
/// # Arguments
///
/// * `rule` - The rule to evaluate
/// * `q_values` - Current Q-variable values (from QSnapshot)
/// * `scene_nodes` - Resolved scene graph (for entity bounds lookup)
///
/// # Returns
///
/// The computed Q-value, or `QValue::Float(0.0)` if evaluation fails.
pub fn evaluate_derived(
    rule: &DerivedRule,
    q_values: &HashMap<String, QValue>,
    scene_nodes: &[SceneNode],
) -> QValue {
    match rule {
        DerivedRule::HitTest {
            pointer_x,
            pointer_y,
            entity_id,
        } => {
            // Get pointer coordinates from Q-values
            let px = match q_values.get(pointer_x).and_then(|v| v.to_rational()) {
                Some(r) => r,
                None => return QValue::Float(0.0),
            };
            let py = match q_values.get(pointer_y).and_then(|v| v.to_rational()) {
                Some(r) => r,
                None => return QValue::Float(0.0),
            };

            // Find entity bounds in scene nodes
            let bounds = match find_entity_bounds(*entity_id, scene_nodes) {
                Some(b) => b,
                None => return QValue::Float(0.0),
            };

            // Hit test: check if pointer is within bounds
            let inside = px >= bounds.x_min
                && px <= bounds.x_max
                && py >= bounds.y_min
                && py <= bounds.y_max;

            QValue::Float(if inside { 1.0 } else { 0.0 })
        }
    }
}

/// Find the bounding box for an entity in the scene graph.
///
/// Recursively searches through scene nodes to find the entity by ID.
fn find_entity_bounds(entity_id: EntityId, scene_nodes: &[SceneNode]) -> Option<SceneBounds> {
    for node in scene_nodes {
        if node.entity_id() == entity_id {
            return Some(node_bounds(node));
        }

        // Recursively search in groups
        if let SceneNode::Group(group) = node {
            if let Some(bounds) = find_entity_bounds(entity_id, &group.children) {
                return Some(bounds);
            }
        }
    }
    None
}

/// Get the bounding box from a scene node.
fn node_bounds(node: &SceneNode) -> SceneBounds {
    match node {
        SceneNode::Path(path) => path.bounds.clone(),
        SceneNode::Group(group) => group.bounds.clone(),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EntityId, VectorComponent};

    #[test]
    fn test_qvalue_rational_extraction() {
        let r = Rational::new(3, 4);
        let qv = QValue::Rational(r.clone());

        // as_rational should return reference to inner Rational
        assert_eq!(qv.as_rational(), Some(&r));

        // to_rational should return owned clone
        assert_eq!(qv.to_rational(), Some(r));
    }

    #[test]
    fn test_qvalue_int_to_rational() {
        let qv = QValue::Int(42);

        let r = qv.to_rational().expect("Int should convert to Rational");
        assert_eq!(r, Rational::from_int(42));
    }

    #[test]
    fn test_qvalue_float_to_rational() {
        let qv = QValue::Float(1.5);

        let r = qv.to_rational().expect("Float should convert to Rational");
        // 1.5 = 3/2
        assert_eq!(r, Rational::new(3, 2));
    }

    #[test]
    fn test_qvalue_bool_no_rational() {
        let qv = QValue::Bool(true);
        assert!(qv.to_rational().is_none());
        assert_eq!(qv.as_bool(), Some(true));
    }

    #[test]
    fn test_qvalue_bytes_extraction() {
        let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let qv = QValue::Bytes(QBytes(data.clone()));

        assert_eq!(qv.as_bytes(), Some(data.as_slice()));
        assert!(qv.to_rational().is_none());
    }

    #[test]
    fn test_qvalue_vec2_extraction() {
        let x = Rational::new(10, 1);
        let y = Rational::new(20, 1);
        let qv = QValue::Vec2(x.clone(), y.clone());

        let (rx, ry) = qv.as_vec2().expect("Should extract Vec2");
        assert_eq!(rx, &x);
        assert_eq!(ry, &y);
    }

    #[test]
    fn test_qvalue_none() {
        let qv = QValue::None;
        assert!(qv.is_none());
        assert!(qv.to_rational().is_none());
    }

    #[test]
    fn test_qvariable_serde_roundtrip() {
        let var = QVariable {
            name: "input.pointer.x".to_string(),
            default: QValue::Int(0),
            target_var: VarId::new(EntityId(100), VectorComponent::X),
        };

        let json = serde_json::to_string(&var).expect("serialize");
        let parsed: QVariable = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.name, var.name);
        assert_eq!(parsed.target_var.entity, var.target_var.entity);
        assert_eq!(parsed.target_var.component, var.target_var.component);
    }

    #[test]
    fn test_qvalue_serde_roundtrip() {
        // Test all QValue variants
        let values = vec![
            QValue::Rational(Rational::new(1, 3)),
            QValue::Int(-42),
            QValue::Float(3.14159),
            QValue::Bool(true),
            QValue::Bytes(QBytes(vec![1, 2, 3, 4])),
            QValue::Vec2(Rational::from_int(10), Rational::from_int(20)),
            QValue::TextureHandle(TextureHandle::new(
                12345,
                1920,
                1080,
                TextureFormat::Rgba8Unorm,
            )),
            QValue::None,
        ];

        for original in values {
            let json = serde_json::to_string(&original).expect("serialize");
            let parsed: QValue = serde_json::from_str(&json).expect("deserialize");

            // Type-specific equality check
            match (&original, &parsed) {
                (QValue::Rational(a), QValue::Rational(b)) => assert_eq!(a, b),
                (QValue::Int(a), QValue::Int(b)) => assert_eq!(a, b),
                (QValue::Float(a), QValue::Float(b)) => assert!((a - b).abs() < 1e-10),
                (QValue::Bool(a), QValue::Bool(b)) => assert_eq!(a, b),
                (QValue::Bytes(a), QValue::Bytes(b)) => assert_eq!(a.0, b.0),
                (QValue::Vec2(ax, ay), QValue::Vec2(bx, by)) => {
                    assert_eq!(ax, bx);
                    assert_eq!(ay, by);
                }
                (QValue::TextureHandle(a), QValue::TextureHandle(b)) => assert_eq!(a, b),
                (QValue::None, QValue::None) => {}
                _ => panic!("Type mismatch after roundtrip"),
            }
        }
    }

    #[test]
    fn test_qsnapshot_serde_roundtrip() {
        let mut snapshot = QSnapshot::new();
        snapshot.insert("input.pointer.x", QValue::Float(100.5));
        snapshot.insert("input.pointer.y", QValue::Float(200.0));
        snapshot.insert("input.pointer.pressed", QValue::Bool(true));
        snapshot.insert("env.viewport.width", QValue::Int(1920));

        let json = serde_json::to_string(&snapshot).expect("serialize");
        let parsed: QSnapshot = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.values.len(), 4);

        // Check Float values
        match parsed.get("input.pointer.x") {
            Some(QValue::Float(v)) => assert!((v - 100.5).abs() < 1e-10),
            _ => panic!("Expected Float for input.pointer.x"),
        }

        // Check Bool value
        match parsed.get("input.pointer.pressed") {
            Some(QValue::Bool(v)) => assert!(*v),
            _ => panic!("Expected Bool for input.pointer.pressed"),
        }

        // Check Int value
        match parsed.get("env.viewport.width") {
            Some(QValue::Int(v)) => assert_eq!(*v, 1920),
            _ => panic!("Expected Int for env.viewport.width"),
        }
    }

    #[test]
    fn test_qsnapshot_from_json() {
        // Test parsing from JS-style JSON
        let json = r#"{
            "values": {
                "input.pointer.x": { "type": "Float", "value": 100.5 },
                "input.pointer.pressed": { "type": "Bool", "value": false }
            }
        }"#;

        let snapshot: QSnapshot = serde_json::from_str(json).expect("parse QSnapshot");
        assert_eq!(snapshot.values.len(), 2);

        match snapshot.get("input.pointer.x") {
            Some(QValue::Float(v)) => assert!((v - 100.5).abs() < 1e-10),
            _ => panic!("Expected Float"),
        }

        match snapshot.get("input.pointer.pressed") {
            Some(QValue::Bool(v)) => assert!(!v),
            _ => panic!("Expected Bool"),
        }
    }

    // =========================================================================
    // Derived Q-Variable Tests
    // =========================================================================

    use crate::scene::{SceneBounds, SceneNode, ScenePathNode};
    use crate::types::{FillRule, PathCommand};

    /// Create a test scene node with given bounds.
    fn create_test_path_node(
        entity_id: u64,
        x_min: i64,
        y_min: i64,
        x_max: i64,
        y_max: i64,
    ) -> SceneNode {
        SceneNode::Path(ScenePathNode {
            entity_id: EntityId(entity_id),
            z_order: 0,
            bounds: SceneBounds::new(
                Rational::from_int(x_min),
                Rational::from_int(y_min),
                Rational::from_int(x_max),
                Rational::from_int(y_max),
            ),
            path_data: vec![],
            closed: true,
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: None,
        })
    }

    #[test]
    fn test_evaluate_derived_hit_test_inside() {
        // Create a 100x100 box at (50, 50) to (150, 150)
        let scene_nodes = vec![create_test_path_node(1000, 50, 50, 150, 150)];

        // Pointer at (100, 100) - inside the box
        let mut q_values = HashMap::new();
        q_values.insert("input.pointer.x".to_string(), QValue::Float(100.0));
        q_values.insert("input.pointer.y".to_string(), QValue::Float(100.0));

        let rule = DerivedRule::HitTest {
            pointer_x: "input.pointer.x".to_string(),
            pointer_y: "input.pointer.y".to_string(),
            entity_id: EntityId(1000),
        };

        let result = evaluate_derived(&rule, &q_values, &scene_nodes);

        match result {
            QValue::Float(v) => assert!(
                (v - 1.0).abs() < 1e-10,
                "Expected 1.0 for inside, got {}",
                v
            ),
            _ => panic!("Expected Float result"),
        }
    }

    #[test]
    fn test_evaluate_derived_hit_test_outside() {
        // Create a 100x100 box at (50, 50) to (150, 150)
        let scene_nodes = vec![create_test_path_node(1000, 50, 50, 150, 150)];

        // Pointer at (200, 200) - outside the box
        let mut q_values = HashMap::new();
        q_values.insert("input.pointer.x".to_string(), QValue::Float(200.0));
        q_values.insert("input.pointer.y".to_string(), QValue::Float(200.0));

        let rule = DerivedRule::HitTest {
            pointer_x: "input.pointer.x".to_string(),
            pointer_y: "input.pointer.y".to_string(),
            entity_id: EntityId(1000),
        };

        let result = evaluate_derived(&rule, &q_values, &scene_nodes);

        match result {
            QValue::Float(v) => assert!(v.abs() < 1e-10, "Expected 0.0 for outside, got {}", v),
            _ => panic!("Expected Float result"),
        }
    }

    #[test]
    fn test_evaluate_derived_hit_test_on_edge() {
        // Create a box at (50, 50) to (150, 150)
        let scene_nodes = vec![create_test_path_node(1000, 50, 50, 150, 150)];

        // Pointer exactly on the edge (50, 100) - should be inside (inclusive)
        let mut q_values = HashMap::new();
        q_values.insert("input.pointer.x".to_string(), QValue::Float(50.0));
        q_values.insert("input.pointer.y".to_string(), QValue::Float(100.0));

        let rule = DerivedRule::HitTest {
            pointer_x: "input.pointer.x".to_string(),
            pointer_y: "input.pointer.y".to_string(),
            entity_id: EntityId(1000),
        };

        let result = evaluate_derived(&rule, &q_values, &scene_nodes);

        match result {
            QValue::Float(v) => {
                assert!((v - 1.0).abs() < 1e-10, "Expected 1.0 for edge, got {}", v)
            }
            _ => panic!("Expected Float result"),
        }
    }

    #[test]
    fn test_evaluate_derived_hit_test_missing_entity() {
        // Empty scene - no entities
        let scene_nodes: Vec<SceneNode> = vec![];

        let mut q_values = HashMap::new();
        q_values.insert("input.pointer.x".to_string(), QValue::Float(100.0));
        q_values.insert("input.pointer.y".to_string(), QValue::Float(100.0));

        let rule = DerivedRule::HitTest {
            pointer_x: "input.pointer.x".to_string(),
            pointer_y: "input.pointer.y".to_string(),
            entity_id: EntityId(9999), // Non-existent entity
        };

        let result = evaluate_derived(&rule, &q_values, &scene_nodes);

        // Should return 0.0 when entity not found
        match result {
            QValue::Float(v) => assert!(
                v.abs() < 1e-10,
                "Expected 0.0 for missing entity, got {}",
                v
            ),
            _ => panic!("Expected Float result"),
        }
    }

    #[test]
    fn test_evaluate_derived_hit_test_missing_pointer() {
        let scene_nodes = vec![create_test_path_node(1000, 50, 50, 150, 150)];

        // Missing pointer_y
        let mut q_values = HashMap::new();
        q_values.insert("input.pointer.x".to_string(), QValue::Float(100.0));
        // input.pointer.y is missing

        let rule = DerivedRule::HitTest {
            pointer_x: "input.pointer.x".to_string(),
            pointer_y: "input.pointer.y".to_string(),
            entity_id: EntityId(1000),
        };

        let result = evaluate_derived(&rule, &q_values, &scene_nodes);

        // Should return 0.0 when pointer data is incomplete
        match result {
            QValue::Float(v) => assert!(
                v.abs() < 1e-10,
                "Expected 0.0 for missing pointer, got {}",
                v
            ),
            _ => panic!("Expected Float result"),
        }
    }

    #[test]
    fn test_derived_qvariable_hover_constructor() {
        let target_var = VarId::new(EntityId(2000), VectorComponent::Value);
        let derived = DerivedQVariable::hover(1000, target_var, EntityId(1000));

        assert_eq!(derived.name, "component.1000.hover");
        assert_eq!(derived.target_var.entity, EntityId(2000));

        match &derived.rule {
            DerivedRule::HitTest {
                pointer_x,
                pointer_y,
                entity_id,
            } => {
                assert_eq!(pointer_x, "input.pointer.x");
                assert_eq!(pointer_y, "input.pointer.y");
                assert_eq!(*entity_id, EntityId(1000));
            }
        }
    }

    // =========================================================================
    // External Texture Tests
    // =========================================================================

    #[test]
    fn test_texture_handle_serde_roundtrip() {
        let handle = TextureHandle {
            id: 12345,
            width: 1920,
            height: 1080,
            format: TextureFormat::Rgba8Unorm,
        };

        let json = serde_json::to_string(&handle).expect("serialize");
        let parsed: TextureHandle = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed, handle);
    }

    #[test]
    fn test_texture_handle_from_json() {
        // Test parsing from JS-style JSON
        let json = r#"{
            "id": 99999,
            "width": 640,
            "height": 480,
            "format": "bgra8unorm"
        }"#;

        let handle: TextureHandle = serde_json::from_str(json).expect("parse TextureHandle");
        assert_eq!(handle.id, 99999);
        assert_eq!(handle.width, 640);
        assert_eq!(handle.height, 480);
        assert_eq!(handle.format, TextureFormat::Bgra8Unorm);
    }

    #[test]
    fn test_qvalue_texture_handle_extraction() {
        let handle = TextureHandle::new(42, 100, 200, TextureFormat::Rgba16Float);
        let qv = QValue::TextureHandle(handle.clone());

        assert_eq!(qv.as_texture_handle(), Some(&handle));
        assert!(qv.to_rational().is_none()); // TextureHandle is not numeric
    }

    #[test]
    fn test_texture_format_default() {
        assert_eq!(TextureFormat::default(), TextureFormat::Rgba8Unorm);
    }

    #[test]
    fn test_qsnapshot_with_texture() {
        let json = r#"{
            "values": {
                "resource.texture.video0": {
                    "type": "TextureHandle",
                    "value": {
                        "id": 1,
                        "width": 1920,
                        "height": 1080,
                        "format": "rgba8unorm"
                    }
                }
            }
        }"#;

        let snapshot: QSnapshot = serde_json::from_str(json).expect("parse QSnapshot");

        match snapshot.get("resource.texture.video0") {
            Some(QValue::TextureHandle(h)) => {
                assert_eq!(h.id, 1);
                assert_eq!(h.width, 1920);
                assert_eq!(h.height, 1080);
                assert_eq!(h.format, TextureFormat::Rgba8Unorm);
            }
            _ => panic!("Expected TextureHandle"),
        }
    }
}
