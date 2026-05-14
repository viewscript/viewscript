//! Interactive DOM Layer Types (Stage 8-11)
//!
//! Defines types for the transparent DOM layer in the bilayer architecture.
//! These types describe how entities map to interactive DOM elements.
//!
//! ## Bilayer Architecture
//!
//! ```text
//! ┌───────────────────────────────────────────────────────┐
//! │  WebGPU Canvas Layer (visual rendering)               │
//! │  - Loop-Blinn path tessellation                       │
//! │  - SDF stroke rendering                               │
//! │  - Text glyph rendering                               │
//! └───────────────────────────────────────────────────────┘
//!                         ▲
//!                         │ Position sync (translate3d)
//!                         ▼
//! ┌───────────────────────────────────────────────────────┐
//! │  Transparent DOM Layer (interaction + accessibility)  │
//! │  - Hit testing via native DOM events                  │
//! │  - Screen reader accessibility (ARIA)                 │
//! │  - Text selection support                             │
//! └───────────────────────────────────────────────────────┘
//! ```

use crate::types::{EntityId, Rational};

// =============================================================================
// Core Types
// =============================================================================

/// Information about an interactive entity in the DOM layer.
#[derive(Debug, Clone)]
pub struct InteractiveInfo {
    /// Human-readable name for the entity (used in generated code).
    pub entity_name: String,

    /// Unique entity ID (matches the visual layer entity).
    pub entity_id: EntityId,

    /// Accessible label for screen readers.
    pub aria_label: Option<String>,

    /// Type of DOM element to generate.
    pub dom_element: DomElementKind,

    /// Event bindings for this entity.
    pub event_bindings: Vec<EventBinding>,
}

/// Kind of DOM element to generate for the interaction layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomElementKind {
    /// Interactive button element.
    /// Generates: `<button>` with transparent background.
    Button,

    /// Text span for text selection.
    /// Generates: `<span>` with selectable text.
    TextSpan,

    /// Generic interactive region.
    /// Generates: `<div role="button">`.
    Region,
}

// =============================================================================
// Event Bindings
// =============================================================================

/// An event binding that wires a DOM event to a state mutation.
#[derive(Debug, Clone)]
pub struct EventBinding {
    /// Type of DOM event to listen for.
    pub event_type: DomEventType,

    /// Action to perform when the event fires.
    pub action: EventAction,
}

/// DOM event types supported by the interaction layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomEventType {
    /// Mouse/touch click event.
    Click,

    /// Pointer down event (start of drag).
    PointerDown,

    /// Pointer up event (end of drag).
    PointerUp,

    /// Element received focus.
    Focus,

    /// Element lost focus.
    Blur,

    /// Keyboard event (requires additional key specification).
    KeyDown,

    /// Keyboard event on key release.
    KeyUp,
}

/// Action to perform in response to a DOM event.
#[derive(Debug, Clone)]
pub enum EventAction {
    /// Increment a numeric variable by a delta.
    ///
    /// Generated code: `target_var += delta;`
    Increment {
        /// Name of the target variable (must exist in var_name_map).
        target_var: String,
        /// Amount to add (can be negative for decrement).
        delta: Rational,
    },

    /// Toggle a variable between two values.
    ///
    /// Generated code: `target_var = (target_var === a) ? b : a;`
    Toggle {
        /// Name of the target variable.
        target_var: String,
        /// The two values to toggle between.
        values: (Rational, Rational),
    },

    /// Set a variable to a constant value.
    ///
    /// Generated code: `target_var = value;`
    SetConstant {
        /// Name of the target variable.
        target_var: String,
        /// Value to set.
        value: Rational,
    },

    /// Call a custom JavaScript function.
    ///
    /// Generated code: `handler_name(event);`
    CallHandler {
        /// Name of the handler function to call.
        handler_name: String,
    },
}

// =============================================================================
// Convenience Constructors
// =============================================================================

impl InteractiveInfo {
    /// Create a new button entity with a click handler.
    pub fn button(
        entity_name: impl Into<String>,
        entity_id: EntityId,
        aria_label: impl Into<String>,
        on_click: EventAction,
    ) -> Self {
        Self {
            entity_name: entity_name.into(),
            entity_id,
            aria_label: Some(aria_label.into()),
            dom_element: DomElementKind::Button,
            event_bindings: vec![EventBinding {
                event_type: DomEventType::Click,
                action: on_click,
            }],
        }
    }

    /// Create a text span entity (for text selection).
    pub fn text_span(
        entity_name: impl Into<String>,
        entity_id: EntityId,
        aria_label: Option<String>,
    ) -> Self {
        Self {
            entity_name: entity_name.into(),
            entity_id,
            aria_label,
            dom_element: DomElementKind::TextSpan,
            event_bindings: vec![],
        }
    }
}

impl EventBinding {
    /// Create a click binding.
    pub fn click(action: EventAction) -> Self {
        Self {
            event_type: DomEventType::Click,
            action,
        }
    }

    /// Create a pointer down binding.
    pub fn pointer_down(action: EventAction) -> Self {
        Self {
            event_type: DomEventType::PointerDown,
            action,
        }
    }
}

// =============================================================================
// JS Code Generation Helpers
// =============================================================================

impl DomElementKind {
    /// Get the HTML tag name for this element kind.
    pub fn tag_name(&self) -> &'static str {
        match self {
            DomElementKind::Button => "button",
            DomElementKind::TextSpan => "span",
            DomElementKind::Region => "div",
        }
    }

    /// Get ARIA role attribute if needed.
    pub fn aria_role(&self) -> Option<&'static str> {
        match self {
            DomElementKind::Button => None, // <button> has implicit role
            DomElementKind::TextSpan => None,
            DomElementKind::Region => Some("button"),
        }
    }
}

impl DomEventType {
    /// Get the DOM event name (for addEventListener).
    pub fn event_name(&self) -> &'static str {
        match self {
            DomEventType::Click => "click",
            DomEventType::PointerDown => "pointerdown",
            DomEventType::PointerUp => "pointerup",
            DomEventType::Focus => "focus",
            DomEventType::Blur => "blur",
            DomEventType::KeyDown => "keydown",
            DomEventType::KeyUp => "keyup",
        }
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_button_constructor() {
        let btn = InteractiveInfo::button(
            "increment_btn",
            EntityId(1000),
            "Increment counter",
            EventAction::Increment {
                target_var: "counter".to_string(),
                delta: Rational::from_int(1),
            },
        );

        assert_eq!(btn.entity_name, "increment_btn");
        assert_eq!(btn.entity_id, EntityId(1000));
        assert_eq!(btn.aria_label, Some("Increment counter".to_string()));
        assert_eq!(btn.dom_element, DomElementKind::Button);
        assert_eq!(btn.event_bindings.len(), 1);
        assert_eq!(btn.event_bindings[0].event_type, DomEventType::Click);
    }

    #[test]
    fn test_dom_element_kind_tag_names() {
        assert_eq!(DomElementKind::Button.tag_name(), "button");
        assert_eq!(DomElementKind::TextSpan.tag_name(), "span");
        assert_eq!(DomElementKind::Region.tag_name(), "div");
    }

    #[test]
    fn test_dom_event_type_names() {
        assert_eq!(DomEventType::Click.event_name(), "click");
        assert_eq!(DomEventType::PointerDown.event_name(), "pointerdown");
        assert_eq!(DomEventType::Focus.event_name(), "focus");
    }

    #[test]
    fn test_text_span_constructor() {
        let span = InteractiveInfo::text_span("label_text", EntityId(2000), None);

        assert_eq!(span.entity_name, "label_text");
        assert_eq!(span.dom_element, DomElementKind::TextSpan);
        assert!(span.event_bindings.is_empty());
    }
}
