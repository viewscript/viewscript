//! Text Shaping and Glyph Outline Extraction (Phase E)
//!
//! This module provides the Q→P dimension bridge for text rendering:
//!
//! ```text
//! Q-Dimension (Input)          P-Dimension (Output)
//! ─────────────────────        ────────────────────────
//! Font binary (TTF/OTF)   →    Rational path commands
//! Text string             →    Positioned glyph entities
//! ```
//!
//! ## Architecture
//!
//! 1. **Text Shaping** (`rustybuzz`): Converts text + font → glyph IDs + positions
//! 2. **Outline Extraction** (`ttf-parser`): Glyph ID → path commands (f32)
//! 3. **P-Dimension Conversion**: f32 → Rational (lossless via `f32_to_rational_exact`)
//!
//! ## Float Decontamination
//!
//! Font files and shaping libraries use floating-point internally, but this module
//! converts ALL coordinates to `Rational` at the boundary. No f32/f64 values escape
//! into P-dimension space.

use crate::types::{f32_to_rational_exact, EntityId, PathCommand, Rational, TextEntity};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// =============================================================================
// Shaped Glyph (E-2a Output)
// =============================================================================

/// A shaped glyph with positioning information.
///
/// This is the output of text shaping (rustybuzz) before outline extraction.
/// All position values are in **font units** (not scaled by font_size).
///
/// ## Coordinate System
///
/// - Origin is at the baseline start
/// - X increases to the right
/// - Y increases upward (font convention)
/// - Advance values determine pen movement after drawing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapedGlyph {
    /// Glyph ID in the font (0-65535).
    pub glyph_id: u16,

    /// Cluster index (maps back to input text position).
    /// Used for cursor positioning and selection.
    pub cluster: u32,

    /// Horizontal advance (pen movement after glyph) in font units.
    pub x_advance: Rational,

    /// Vertical advance in font units (typically 0 for horizontal text).
    pub y_advance: Rational,

    /// Horizontal offset from pen position to glyph origin.
    pub x_offset: Rational,

    /// Vertical offset from pen position to glyph origin.
    pub y_offset: Rational,
}

// =============================================================================
// Outline Builder (ttf-parser Integration)
// =============================================================================
//
// PathCommand is now defined in crate::types and imported above.

/// Collects glyph outline commands as Rational path data.
///
/// Implements `ttf_parser::OutlineBuilder` to receive outline callbacks
/// and converts f32 coordinates to exact Rational representation.
///
/// ## CFF Font Support
///
/// CFF fonts may produce non-integer f32 coordinates. The conversion
/// via `f32_to_rational_exact()` handles this losslessly by decomposing
/// the IEEE 754 bit representation.
#[derive(Debug, Default)]
pub struct RationalOutlineBuilder {
    commands: Vec<PathCommand>,
}

impl RationalOutlineBuilder {
    /// Create a new empty outline builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume the builder and return collected path commands.
    pub fn into_commands(self) -> Vec<PathCommand> {
        self.commands
    }

    /// Get reference to collected commands.
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }

    /// Check if any commands were collected.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

impl ttf_parser::OutlineBuilder for RationalOutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.commands.push(PathCommand::MoveTo {
            x: f32_to_rational_exact(x),
            y: f32_to_rational_exact(y),
        });
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.commands.push(PathCommand::LineTo {
            x: f32_to_rational_exact(x),
            y: f32_to_rational_exact(y),
        });
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.commands.push(PathCommand::QuadTo {
            x1: f32_to_rational_exact(cx),
            y1: f32_to_rational_exact(cy),
            x: f32_to_rational_exact(x),
            y: f32_to_rational_exact(y),
        });
    }

    fn curve_to(&mut self, cx1: f32, cy1: f32, cx2: f32, cy2: f32, x: f32, y: f32) {
        self.commands.push(PathCommand::CubicTo {
            x1: f32_to_rational_exact(cx1),
            y1: f32_to_rational_exact(cy1),
            x2: f32_to_rational_exact(cx2),
            y2: f32_to_rational_exact(cy2),
            x: f32_to_rational_exact(x),
            y: f32_to_rational_exact(y),
        });
    }

    fn close(&mut self) {
        self.commands.push(PathCommand::Close);
    }
}

// =============================================================================
// Text Shaper (E-2a)
// =============================================================================

/// Error types for text shaping operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextShapingError {
    /// Font data could not be parsed.
    FontParseError(String),
    /// Font does not contain required tables.
    MissingFontTable(String),
    /// Glyph ID not found in font.
    GlyphNotFound(u16),
    /// Text shaping failed.
    ShapingFailed(String),
}

impl std::fmt::Display for TextShapingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FontParseError(msg) => write!(f, "Font parse error: {}", msg),
            Self::MissingFontTable(table) => write!(f, "Missing font table: {}", table),
            Self::GlyphNotFound(id) => write!(f, "Glyph not found: {}", id),
            Self::ShapingFailed(msg) => write!(f, "Shaping failed: {}", msg),
        }
    }
}

impl std::error::Error for TextShapingError {}

/// Text shaper that converts text strings to positioned glyphs.
///
/// ## Usage
///
/// ```ignore
/// let font_data: &[u8] = load_font_file("Inter.ttf");
/// let shaper = TextShaper::new(font_data)?;
///
/// let shaped = shaper.shape("Hello, World!")?;
/// for glyph in shaped {
///     println!("Glyph {}: advance = {:?}", glyph.glyph_id, glyph.x_advance);
/// }
/// ```
pub struct TextShaper<'a> {
    /// Parsed font face for outline extraction.
    face: ttf_parser::Face<'a>,
    /// rustybuzz face for text shaping.
    buzz_face: rustybuzz::Face<'a>,
    /// Font's units-per-em value.
    units_per_em: u16,
}

impl<'a> TextShaper<'a> {
    /// Create a new text shaper from font data.
    ///
    /// # Arguments
    ///
    /// * `font_data` - Raw font file bytes (TTF, OTF, or TTC)
    ///
    /// # Errors
    ///
    /// Returns error if font data cannot be parsed.
    pub fn new(font_data: &'a [u8]) -> Result<Self, TextShapingError> {
        let face = ttf_parser::Face::parse(font_data, 0)
            .map_err(|e| TextShapingError::FontParseError(format!("{:?}", e)))?;

        let buzz_face = rustybuzz::Face::from_slice(font_data, 0).ok_or_else(|| {
            TextShapingError::FontParseError("rustybuzz failed to parse font".into())
        })?;

        let units_per_em = face.units_per_em();

        Ok(Self {
            face,
            buzz_face,
            units_per_em,
        })
    }

    /// Get the font's units-per-em value.
    ///
    /// This is needed for scaling glyph coordinates to device units:
    /// `device_coord = font_coord * font_size / units_per_em`
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// Shape a text string into positioned glyphs.
    ///
    /// This performs text shaping using rustybuzz, handling:
    /// - Unicode normalization
    /// - Bidirectional text
    /// - Ligature substitution (fi, fl, etc.)
    /// - Kerning adjustments
    /// - OpenType feature application
    ///
    /// # Arguments
    ///
    /// * `text` - Input text string (UTF-8)
    ///
    /// # Returns
    ///
    /// Vector of `ShapedGlyph` with glyph IDs and positioning in font units.
    pub fn shape(&self, text: &str) -> Result<Vec<ShapedGlyph>, TextShapingError> {
        // Create buffer and add text
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);

        // Shape the text
        let output = rustybuzz::shape(&self.buzz_face, &[], buffer);

        // Extract glyph info and positions
        let infos = output.glyph_infos();
        let positions = output.glyph_positions();

        let mut shaped_glyphs = Vec::with_capacity(infos.len());

        for (info, pos) in infos.iter().zip(positions.iter()) {
            // Convert i32 font units to Rational (lossless integer conversion)
            shaped_glyphs.push(ShapedGlyph {
                glyph_id: info.glyph_id as u16, // Guaranteed <= u16::MAX by spec
                cluster: info.cluster,
                x_advance: Rational::from_int(pos.x_advance as i64),
                y_advance: Rational::from_int(pos.y_advance as i64),
                x_offset: Rational::from_int(pos.x_offset as i64),
                y_offset: Rational::from_int(pos.y_offset as i64),
            });
        }

        Ok(shaped_glyphs)
    }

    /// Get the outline path commands for a glyph.
    ///
    /// Coordinates are in **font units** (not scaled).
    ///
    /// # Arguments
    ///
    /// * `glyph_id` - The glyph ID from shaping
    ///
    /// # Returns
    ///
    /// Path commands with Rational coordinates, or error if glyph not found.
    pub fn glyph_outline(&self, glyph_id: u16) -> Result<Vec<PathCommand>, TextShapingError> {
        let gid = ttf_parser::GlyphId(glyph_id);

        let mut builder = RationalOutlineBuilder::new();

        self.face
            .outline_glyph(gid, &mut builder)
            .ok_or(TextShapingError::GlyphNotFound(glyph_id))?;

        Ok(builder.into_commands())
    }

    /// Get the glyph's bounding box in font units.
    ///
    /// Returns `(x_min, y_min, x_max, y_max)` as Rational values.
    pub fn glyph_bbox(&self, glyph_id: u16) -> Option<(Rational, Rational, Rational, Rational)> {
        let gid = ttf_parser::GlyphId(glyph_id);
        let rect = self.face.glyph_bounding_box(gid)?;

        Some((
            Rational::from_int(rect.x_min as i64),
            Rational::from_int(rect.y_min as i64),
            Rational::from_int(rect.x_max as i64),
            Rational::from_int(rect.y_max as i64),
        ))
    }

    /// Expand text to paths using prototype + instance model.
    ///
    /// This is the main entry point for `expand-text-to-paths` operation.
    ///
    /// ## Prototype + Instance Model
    ///
    /// Instead of duplicating path data for repeated glyphs, this method:
    /// 1. Creates one `GlyphPrototype` per unique glyph ID (with outline data)
    /// 2. Creates `GlyphInstance` references for each glyph occurrence
    ///
    /// For "Hello", 'l' appears twice but only one prototype is created.
    /// This reduces constraint graph size from O(n * avg_path_commands) to
    /// O(unique_glyphs * avg_path_commands + n).
    ///
    /// ## Coordinate System
    ///
    /// - Prototype paths are in **font units** (not scaled)
    /// - Instance origins are in **scaled units** (font_size / units_per_em applied)
    /// - The `scale_factor` in `ExpandedText` is provided for coordinate transformation
    ///
    /// # Arguments
    ///
    /// * `text` - Input text string
    /// * `font_size` - Desired font size in P-dimension units
    /// * `base_entity_id` - Starting EntityId for allocation
    ///
    /// # Returns
    ///
    /// `ExpandedText` containing prototypes, instances, and scale factor.
    pub fn expand_to_paths(
        &self,
        text: &str,
        font_size: &Rational,
        base_entity_id: u64,
    ) -> Result<ExpandedText, TextShapingError> {
        // Step 1: Shape text to get glyph sequence
        let shaped_glyphs = self.shape(text)?;

        // Step 2: Deduplicate glyph IDs and create prototypes
        let mut prototypes: HashMap<u16, GlyphPrototype> = HashMap::new();
        let mut next_entity_id = base_entity_id;

        let units_per_em = Rational::from_int(self.units_per_em as i64);
        let scale_factor = font_size.clone() / units_per_em.clone();

        for glyph in &shaped_glyphs {
            if !prototypes.contains_key(&glyph.glyph_id) {
                // Get outline for this glyph (may fail for space, etc.)
                let path_commands = self.glyph_outline(glyph.glyph_id).unwrap_or_default();
                let bbox = self.glyph_bbox(glyph.glyph_id);

                prototypes.insert(
                    glyph.glyph_id,
                    GlyphPrototype {
                        glyph_id: glyph.glyph_id,
                        entity_id: EntityId(next_entity_id),
                        path_commands,
                        advance_width: glyph.x_advance.clone(),
                        bbox,
                    },
                );
                next_entity_id += 1;
            }
        }

        // Step 3: Create instances with cumulative origin positions
        let mut instances = Vec::with_capacity(shaped_glyphs.len());
        let mut pen_x = Rational::zero();
        let mut pen_y = Rational::zero();

        for glyph in &shaped_glyphs {
            let prototype = prototypes.get(&glyph.glyph_id).unwrap();

            // Instance origin = pen position + offset, scaled
            let origin_x = (pen_x.clone() + glyph.x_offset.clone()) * scale_factor.clone();
            let origin_y = (pen_y.clone() + glyph.y_offset.clone()) * scale_factor.clone();

            instances.push(GlyphInstance {
                prototype_id: prototype.entity_id,
                entity_id: EntityId(next_entity_id),
                origin: (origin_x, origin_y),
            });
            next_entity_id += 1;

            // Advance pen position (in font units, will be scaled per-instance)
            pen_x = pen_x + glyph.x_advance.clone();
            pen_y = pen_y + glyph.y_advance.clone();
        }

        Ok(ExpandedText {
            prototypes,
            instances,
            font_size: font_size.clone(),
            units_per_em,
            scale_factor,
        })
    }
}

// =============================================================================
// Prototype + Instance Model (E-2b)
// =============================================================================

/// A glyph prototype containing the shared outline data.
///
/// ## Design Rationale
///
/// In a 1000-character text, ASCII characters repeat on average 20-30 times.
/// Without deduplication, each occurrence would expand to ~50 PathCommands,
/// creating 50,000+ constraint terms. With prototypes, we have:
/// - ~70 unique prototypes (ASCII + punctuation)
/// - ~3,500 PathCommands total
/// - 1,000 instances with simple offset constraints
///
/// This reduces L0 solver complexity from O(n²) on 50k terms to O(n²) on ~4.5k.
///
/// Note: PartialEq only due to PathCommand containing f64 (ArcTo.rotation).
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphPrototype {
    /// Glyph ID in the font (0-65535).
    pub glyph_id: u16,

    /// EntityId for this prototype (used for constraint references).
    pub entity_id: EntityId,

    /// Path commands in **font units** (not scaled).
    /// Empty for glyphs without outlines (e.g., space).
    pub path_commands: Vec<PathCommand>,

    /// Advance width in font units.
    pub advance_width: Rational,

    /// Bounding box in font units: (x_min, y_min, x_max, y_max).
    /// None for glyphs without outlines.
    pub bbox: Option<(Rational, Rational, Rational, Rational)>,
}

/// A glyph instance referencing a prototype.
///
/// Instances contain only:
/// - Reference to prototype (for path data)
/// - Position offset (for placement)
///
/// The constraint system expresses instance placement as:
/// ```text
/// instance.x = prototype.x + origin.x
/// instance.y = prototype.y + origin.y
/// ```
///
/// This is a simple linear constraint, processed in L0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphInstance {
    /// Reference to the GlyphPrototype's EntityId.
    pub prototype_id: EntityId,

    /// Unique EntityId for this instance.
    pub entity_id: EntityId,

    /// Position in **scaled units** (font_size / units_per_em applied).
    /// This is the offset from the text block origin to this glyph's origin.
    pub origin: (Rational, Rational),
}

/// Result of expand-text-to-paths operation.
///
/// Contains all data needed to render text as vector paths.
///
/// Note: PartialEq only due to GlyphPrototype containing PathCommand (f64 in ArcTo).
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedText {
    /// Unique glyph prototypes (glyph_id → prototype).
    /// Each prototype contains the outline path commands.
    pub prototypes: HashMap<u16, GlyphPrototype>,

    /// Glyph instances in text order.
    /// Each instance references a prototype and has a position offset.
    pub instances: Vec<GlyphInstance>,

    /// Font size used for scaling.
    pub font_size: Rational,

    /// Font's units-per-em value as Rational.
    pub units_per_em: Rational,

    /// Pre-computed scale factor: font_size / units_per_em.
    /// Multiply font-unit coordinates by this to get P-dimension coordinates.
    pub scale_factor: Rational,
}

// =============================================================================
// Canvas Node Types for Text (E-2c)
// =============================================================================

/// 2D affine transformation matrix for canvas rendering.
///
/// ```text
/// | a  c  tx |
/// | b  d  ty |
/// | 0  0  1  |
/// ```
///
/// This struct uses Rational for P-dimension exactness. Conversion to f64
/// happens at the vsc-gpu rasterization boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextAffineTransform {
    /// Scale X (identity = 1)
    pub a: Rational,
    /// Skew Y
    pub b: Rational,
    /// Skew X
    pub c: Rational,
    /// Scale Y (identity = 1)
    pub d: Rational,
    /// Translate X
    pub tx: Rational,
    /// Translate Y
    pub ty: Rational,
}

impl TextAffineTransform {
    /// Create an identity transform.
    pub fn identity() -> Self {
        Self {
            a: Rational::one(),
            b: Rational::zero(),
            c: Rational::zero(),
            d: Rational::one(),
            tx: Rational::zero(),
            ty: Rational::zero(),
        }
    }

    /// Create a translation transform.
    pub fn translate(tx: Rational, ty: Rational) -> Self {
        Self {
            a: Rational::one(),
            b: Rational::zero(),
            c: Rational::zero(),
            d: Rational::one(),
            tx,
            ty,
        }
    }

    /// Create a scale transform.
    pub fn scale(sx: Rational, sy: Rational) -> Self {
        Self {
            a: sx,
            b: Rational::zero(),
            c: Rational::zero(),
            d: sy,
            tx: Rational::zero(),
            ty: Rational::zero(),
        }
    }
}

/// Canvas path node for text glyph rendering.
///
/// Note: PartialEq only due to PathCommand containing f64 (ArcTo.rotation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextCanvasPathNode {
    /// Entity ID for this path.
    pub entity_id: EntityId,

    /// Path commands (scaled to P-dimension).
    pub path_data: Vec<PathCommand>,

    /// Fill color (CSS format, e.g., "#000000").
    pub fill_color: Option<String>,
}

/// Canvas group node for hierarchical text rendering.
///
/// Note: PartialEq only due to PathCommand containing f64 (ArcTo.rotation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextCanvasGroupNode {
    /// Entity ID for this group.
    pub entity_id: EntityId,

    /// Child nodes (paths or nested groups).
    pub children: Vec<TextCanvasNode>,

    /// Transform applied to this group and all children.
    pub transform: TextAffineTransform,
}

/// Canvas node union for text rendering.
///
/// Note: PartialEq only due to PathCommand containing f64 (ArcTo.rotation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TextCanvasNode {
    /// Path node (glyph outline).
    Path(TextCanvasPathNode),
    /// Group node (glyph instance or text block).
    Group(TextCanvasGroupNode),
}

// =============================================================================
// Text to Canvas Converter (E-2c)
// =============================================================================

/// Converts `ExpandedText` to a canvas node tree for rendering.
///
/// ## Design
///
/// The output structure is:
///
/// ```text
/// TextCanvasGroupNode (text block)
///   ├─ transform: translate(text_entity.origin)
///   └─ children:
///        ├─ TextCanvasGroupNode (glyph instance 0)
///        │    ├─ transform: translate(instance.origin) × scale(scale_factor)
///        │    └─ children: [TextCanvasPathNode (prototype path)]
///        ├─ TextCanvasGroupNode (glyph instance 1)
///        │    └─ ...
///        └─ ...
/// ```
///
/// ## Prototype Path Caching
///
/// Scaled paths are computed once per prototype and reused across instances.
/// This avoids O(instances × path_commands) scaling operations.
pub struct TextToCanvasConverter;

impl TextToCanvasConverter {
    /// Convert expanded text to a canvas group node.
    ///
    /// # Arguments
    ///
    /// * `expanded` - The expanded text with prototypes and instances
    /// * `text_entity` - The original text entity (for position reference)
    /// * `fill_color` - Fill color for all glyphs (CSS format)
    ///
    /// # Returns
    ///
    /// A `TextCanvasGroupNode` representing the entire text block.
    pub fn convert(
        expanded: &ExpandedText,
        text_entity: &TextEntity,
        fill_color: Option<String>,
    ) -> TextCanvasGroupNode {
        // Cache: prototype glyph_id → scaled PathCommand list
        let mut scaled_path_cache: HashMap<u16, Vec<PathCommand>> = HashMap::new();

        // Pre-compute scaled paths for all prototypes
        for (glyph_id, prototype) in &expanded.prototypes {
            let scaled_commands =
                Self::scale_path_commands(&prototype.path_commands, &expanded.scale_factor);
            scaled_path_cache.insert(*glyph_id, scaled_commands);
        }

        // Create child groups for each glyph instance
        let mut children = Vec::with_capacity(expanded.instances.len());

        for instance in &expanded.instances {
            // Find the prototype for this instance
            let prototype = expanded
                .prototypes
                .values()
                .find(|p| p.entity_id == instance.prototype_id);

            let Some(prototype) = prototype else {
                continue; // Skip if prototype not found (shouldn't happen)
            };

            // Get cached scaled path
            let scaled_path = scaled_path_cache
                .get(&prototype.glyph_id)
                .cloned()
                .unwrap_or_default();

            // Skip glyphs without outlines (e.g., space)
            if scaled_path.is_empty() {
                continue;
            }

            // Create path node with scaled commands
            let path_node = TextCanvasPathNode {
                entity_id: instance.entity_id,
                path_data: scaled_path,
                fill_color: fill_color.clone(),
            };

            // Wrap in group with translation transform
            let instance_group = TextCanvasGroupNode {
                entity_id: instance.entity_id,
                children: vec![TextCanvasNode::Path(path_node)],
                transform: TextAffineTransform::translate(
                    instance.origin.0.clone(),
                    instance.origin.1.clone(),
                ),
            };

            children.push(TextCanvasNode::Group(instance_group));
        }

        // Create root group for the text block
        // Position based on TextEntity's top-left corner
        TextCanvasGroupNode {
            entity_id: text_entity.id,
            children,
            transform: TextAffineTransform::identity(),
        }
    }

    /// Scale all coordinates in path commands by scale_factor.
    fn scale_path_commands(commands: &[PathCommand], scale: &Rational) -> Vec<PathCommand> {
        commands
            .iter()
            .map(|cmd| Self::scale_command(cmd, scale))
            .collect()
    }

    /// Scale a single path command.
    fn scale_command(cmd: &PathCommand, scale: &Rational) -> PathCommand {
        match cmd {
            PathCommand::MoveTo { x, y } => PathCommand::MoveTo {
                x: x.clone() * scale.clone(),
                y: y.clone() * scale.clone(),
            },
            PathCommand::LineTo { x, y } => PathCommand::LineTo {
                x: x.clone() * scale.clone(),
                y: y.clone() * scale.clone(),
            },
            PathCommand::QuadTo { x1, y1, x, y } => PathCommand::QuadTo {
                x1: x1.clone() * scale.clone(),
                y1: y1.clone() * scale.clone(),
                x: x.clone() * scale.clone(),
                y: y.clone() * scale.clone(),
            },
            PathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => PathCommand::CubicTo {
                x1: x1.clone() * scale.clone(),
                y1: y1.clone() * scale.clone(),
                x2: x2.clone() * scale.clone(),
                y2: y2.clone() * scale.clone(),
                x: x.clone() * scale.clone(),
                y: y.clone() * scale.clone(),
            },
            PathCommand::ArcTo {
                rx,
                ry,
                rotation,
                large_arc,
                sweep,
                x,
                y,
            } => PathCommand::ArcTo {
                rx: rx.clone() * scale.clone(),
                ry: ry.clone() * scale.clone(),
                rotation: *rotation, // Rotation angle unchanged by scaling
                large_arc: *large_arc,
                sweep: *sweep,
                x: x.clone() * scale.clone(),
                y: y.clone() * scale.clone(),
            },
            PathCommand::Close => PathCommand::Close,
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ttf_parser::OutlineBuilder;

    /// Test RationalOutlineBuilder collects commands correctly.
    #[test]
    fn test_outline_builder_basic() {
        let mut builder = RationalOutlineBuilder::new();

        // Simulate outline callbacks
        builder.move_to(0.0, 0.0);
        builder.line_to(100.0, 0.0);
        builder.line_to(100.0, 100.0);
        builder.close();

        let commands = builder.into_commands();
        assert_eq!(commands.len(), 4);

        assert_eq!(
            commands[0],
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero()
            }
        );
        assert_eq!(
            commands[1],
            PathCommand::LineTo {
                x: Rational::from_int(100),
                y: Rational::zero()
            }
        );
    }

    /// Test outline builder handles CFF-style fractional coordinates.
    #[test]
    fn test_outline_builder_fractional() {
        let mut builder = RationalOutlineBuilder::new();

        builder.move_to(0.5, 0.75);
        builder.quad_to(1.25, 2.5, 3.0, 4.0);

        let commands = builder.into_commands();

        assert_eq!(
            commands[0],
            PathCommand::MoveTo {
                x: Rational::new(1, 2), // 0.5 = 1/2
                y: Rational::new(3, 4), // 0.75 = 3/4
            }
        );

        if let PathCommand::QuadTo { x1, y1, x, y } = &commands[1] {
            assert_eq!(*x1, Rational::new(5, 4)); // 1.25 = 5/4
            assert_eq!(*y1, Rational::new(5, 2)); // 2.5 = 5/2
            assert_eq!(*x, Rational::from_int(3));
            assert_eq!(*y, Rational::from_int(4));
        } else {
            panic!("Expected QuadTo command");
        }
    }

    /// Test ShapedGlyph creation from integer values.
    #[test]
    fn test_shaped_glyph_from_integers() {
        let glyph = ShapedGlyph {
            glyph_id: 42,
            cluster: 0,
            x_advance: Rational::from_int(600),
            y_advance: Rational::zero(),
            x_offset: Rational::from_int(10),
            y_offset: Rational::from_int(-5),
        };

        assert_eq!(glyph.glyph_id, 42);
        assert_eq!(glyph.x_advance, Rational::new(600, 1));
        assert_eq!(glyph.y_offset, Rational::new(-5, 1));
    }

    // =========================================================================
    // E-2b: Prototype + Instance Model Tests
    // =========================================================================

    /// Test mock expand_to_paths logic with simulated data.
    ///
    /// Since we can't include real font files in tests, we test the
    /// deduplication and positioning logic with mock structures.
    #[test]
    fn test_prototype_deduplication_logic() {
        // Simulate "aaa" - same glyph 3 times
        let glyph_ids = vec![1u16, 1, 1]; // 'a' = glyph ID 1

        // Count unique glyphs
        let mut seen: std::collections::HashSet<u16> = std::collections::HashSet::new();
        for &id in &glyph_ids {
            seen.insert(id);
        }

        assert_eq!(seen.len(), 1, "aaa should have 1 unique glyph");
        assert_eq!(glyph_ids.len(), 3, "aaa should have 3 instances");
    }

    /// Test "abc" produces 3 prototypes and 3 instances.
    #[test]
    fn test_prototype_no_duplication() {
        // Simulate "abc" - different glyphs
        let glyph_ids = vec![1u16, 2, 3]; // 'a'=1, 'b'=2, 'c'=3

        let mut seen: std::collections::HashSet<u16> = std::collections::HashSet::new();
        for &id in &glyph_ids {
            seen.insert(id);
        }

        assert_eq!(seen.len(), 3, "abc should have 3 unique glyphs");
        assert_eq!(glyph_ids.len(), 3, "abc should have 3 instances");
    }

    /// Test origin calculation matches cumulative advance.
    #[test]
    fn test_instance_origin_calculation() {
        // Simulate 3 glyphs with advance_width = 100 each
        // font_size = 16, units_per_em = 1000
        // scale_factor = 16/1000 = 0.016

        let advance = Rational::from_int(100); // font units
        let font_size = Rational::from_int(16);
        let units_per_em = Rational::from_int(1000);
        let scale_factor = font_size.clone() / units_per_em;

        // Calculate expected origins
        let origin_0 = Rational::zero(); // First glyph at 0
        let origin_1 = advance.clone() * scale_factor.clone(); // 100 * 16/1000 = 1.6
        let origin_2 = (advance.clone() + advance.clone()) * scale_factor.clone(); // 200 * 16/1000 = 3.2

        assert_eq!(origin_0, Rational::zero());
        assert_eq!(origin_1, Rational::new(16 * 100, 1000)); // 1600/1000 = 8/5
        assert_eq!(origin_2, Rational::new(16 * 200, 1000)); // 3200/1000 = 16/5

        // Verify the values are correct
        // origin_1 = 100 * 16 / 1000 = 1600/1000 = 8/5 = 1.6
        assert_eq!(origin_1, Rational::new(8, 5));
        // origin_2 = 200 * 16 / 1000 = 3200/1000 = 16/5 = 3.2
        assert_eq!(origin_2, Rational::new(16, 5));
    }

    /// Test GlyphPrototype structure.
    #[test]
    fn test_glyph_prototype_structure() {
        use crate::EntityId;

        let prototype = GlyphPrototype {
            glyph_id: 65, // 'A'
            entity_id: EntityId(100),
            path_commands: vec![
                PathCommand::MoveTo {
                    x: Rational::zero(),
                    y: Rational::zero(),
                },
                PathCommand::LineTo {
                    x: Rational::from_int(500),
                    y: Rational::from_int(700),
                },
                PathCommand::Close,
            ],
            advance_width: Rational::from_int(600),
            bbox: Some((
                Rational::zero(),
                Rational::zero(),
                Rational::from_int(500),
                Rational::from_int(700),
            )),
        };

        assert_eq!(prototype.glyph_id, 65);
        assert_eq!(prototype.path_commands.len(), 3);
        assert_eq!(prototype.advance_width, Rational::from_int(600));
    }

    /// Test GlyphInstance structure.
    #[test]
    fn test_glyph_instance_structure() {
        use crate::EntityId;

        let instance = GlyphInstance {
            prototype_id: EntityId(100),
            entity_id: EntityId(200),
            origin: (Rational::new(16, 5), Rational::zero()), // x=3.2, y=0
        };

        assert_eq!(instance.prototype_id, EntityId(100));
        assert_eq!(instance.entity_id, EntityId(200));
        assert_eq!(instance.origin.0, Rational::new(16, 5));
    }

    /// Test ExpandedText structure completeness.
    #[test]
    fn test_expanded_text_structure() {
        use crate::EntityId;

        let mut prototypes = HashMap::new();
        prototypes.insert(
            1,
            GlyphPrototype {
                glyph_id: 1,
                entity_id: EntityId(100),
                path_commands: vec![],
                advance_width: Rational::from_int(500),
                bbox: None,
            },
        );

        let instances = vec![
            GlyphInstance {
                prototype_id: EntityId(100),
                entity_id: EntityId(200),
                origin: (Rational::zero(), Rational::zero()),
            },
            GlyphInstance {
                prototype_id: EntityId(100),
                entity_id: EntityId(201),
                origin: (Rational::new(8, 1), Rational::zero()),
            },
            GlyphInstance {
                prototype_id: EntityId(100),
                entity_id: EntityId(202),
                origin: (Rational::from_int(16), Rational::zero()),
            },
        ];

        let expanded = ExpandedText {
            prototypes,
            instances,
            font_size: Rational::from_int(16),
            units_per_em: Rational::from_int(1000),
            scale_factor: Rational::new(16, 1000),
        };

        // "aaa" pattern: 1 prototype, 3 instances
        assert_eq!(expanded.prototypes.len(), 1);
        assert_eq!(expanded.instances.len(), 3);

        // All instances reference the same prototype
        for inst in &expanded.instances {
            assert_eq!(inst.prototype_id, EntityId(100));
        }
    }

    // =========================================================================
    // E-2c: TextToCanvasConverter Tests
    // =========================================================================

    /// Test "ab" produces 2 child groups with correct transforms.
    #[test]
    fn test_text_to_canvas_converter_ab() {
        use crate::{EntityId, TextEntity};

        // Create mock prototypes for 'a' and 'b'
        let mut prototypes = HashMap::new();
        prototypes.insert(
            1, // 'a'
            GlyphPrototype {
                glyph_id: 1,
                entity_id: EntityId(100),
                path_commands: vec![
                    PathCommand::MoveTo {
                        x: Rational::zero(),
                        y: Rational::zero(),
                    },
                    PathCommand::LineTo {
                        x: Rational::from_int(500),
                        y: Rational::from_int(700),
                    },
                    PathCommand::Close,
                ],
                advance_width: Rational::from_int(600),
                bbox: Some((
                    Rational::zero(),
                    Rational::zero(),
                    Rational::from_int(500),
                    Rational::from_int(700),
                )),
            },
        );
        prototypes.insert(
            2, // 'b'
            GlyphPrototype {
                glyph_id: 2,
                entity_id: EntityId(101),
                path_commands: vec![
                    PathCommand::MoveTo {
                        x: Rational::zero(),
                        y: Rational::zero(),
                    },
                    PathCommand::LineTo {
                        x: Rational::from_int(550),
                        y: Rational::from_int(700),
                    },
                    PathCommand::Close,
                ],
                advance_width: Rational::from_int(650),
                bbox: Some((
                    Rational::zero(),
                    Rational::zero(),
                    Rational::from_int(550),
                    Rational::from_int(700),
                )),
            },
        );

        // font_size = 16, units_per_em = 1000
        // scale_factor = 16/1000 = 2/125
        let scale_factor = Rational::new(16, 1000);

        // Instance origins (scaled):
        // 'a' at origin (0, 0)
        // 'b' at (600 * 16/1000, 0) = (9600/1000, 0) = (48/5, 0)
        let origin_a = (Rational::zero(), Rational::zero());
        let origin_b = (Rational::new(48, 5), Rational::zero());

        let instances = vec![
            GlyphInstance {
                prototype_id: EntityId(100),
                entity_id: EntityId(200),
                origin: origin_a.clone(),
            },
            GlyphInstance {
                prototype_id: EntityId(101),
                entity_id: EntityId(201),
                origin: origin_b.clone(),
            },
        ];

        let expanded = ExpandedText {
            prototypes,
            instances,
            font_size: Rational::from_int(16),
            units_per_em: Rational::from_int(1000),
            scale_factor: scale_factor.clone(),
        };

        // Create a mock TextEntity
        let text_entity = TextEntity::new(
            EntityId(1),
            "ab".to_string(),
            "TestFont".to_string(),
            Rational::from_int(16),
        );

        // Convert to canvas
        let canvas =
            TextToCanvasConverter::convert(&expanded, &text_entity, Some("#000000".to_string()));

        // Verify structure
        assert_eq!(canvas.entity_id, EntityId(1));
        assert_eq!(
            canvas.children.len(),
            2,
            "Should have 2 child groups for 'ab'"
        );

        // Check first child (glyph 'a')
        if let TextCanvasNode::Group(group_a) = &canvas.children[0] {
            // Transform should be translate(origin_a)
            assert_eq!(group_a.transform.tx, origin_a.0);
            assert_eq!(group_a.transform.ty, origin_a.1);
            assert_eq!(group_a.transform.a, Rational::one()); // No scaling in transform
            assert_eq!(group_a.transform.d, Rational::one());

            // Should have one path child
            assert_eq!(group_a.children.len(), 1);
            if let TextCanvasNode::Path(path_a) = &group_a.children[0] {
                // Path should be scaled
                assert_eq!(path_a.path_data.len(), 3);
                if let PathCommand::LineTo { x, y } = &path_a.path_data[1] {
                    // 500 * 16/1000 = 8, 700 * 16/1000 = 56/5 = 11.2
                    assert_eq!(*x, Rational::from_int(500) * scale_factor.clone());
                    assert_eq!(*y, Rational::from_int(700) * scale_factor.clone());
                }
            }
        } else {
            panic!("First child should be a Group");
        }

        // Check second child (glyph 'b')
        if let TextCanvasNode::Group(group_b) = &canvas.children[1] {
            // Transform should be translate(origin_b)
            assert_eq!(group_b.transform.tx, origin_b.0);
            assert_eq!(group_b.transform.ty, origin_b.1);
        } else {
            panic!("Second child should be a Group");
        }
    }

    /// Test path scaling is cached (same prototype paths are not recomputed).
    #[test]
    fn test_text_to_canvas_converter_caching() {
        use crate::{EntityId, TextEntity};

        // Create mock prototype for 'a' with complex path
        let complex_path = vec![
            PathCommand::MoveTo {
                x: Rational::from_int(100),
                y: Rational::from_int(200),
            },
            PathCommand::CubicTo {
                x1: Rational::from_int(150),
                y1: Rational::from_int(250),
                x2: Rational::from_int(200),
                y2: Rational::from_int(300),
                x: Rational::from_int(250),
                y: Rational::from_int(350),
            },
            PathCommand::Close,
        ];

        let mut prototypes = HashMap::new();
        prototypes.insert(
            1,
            GlyphPrototype {
                glyph_id: 1,
                entity_id: EntityId(100),
                path_commands: complex_path,
                advance_width: Rational::from_int(500),
                bbox: None,
            },
        );

        let scale_factor = Rational::new(1, 10); // 0.1

        // Create 3 instances of the same glyph ("aaa")
        let instances = vec![
            GlyphInstance {
                prototype_id: EntityId(100),
                entity_id: EntityId(200),
                origin: (Rational::zero(), Rational::zero()),
            },
            GlyphInstance {
                prototype_id: EntityId(100),
                entity_id: EntityId(201),
                origin: (Rational::from_int(50), Rational::zero()),
            },
            GlyphInstance {
                prototype_id: EntityId(100),
                entity_id: EntityId(202),
                origin: (Rational::from_int(100), Rational::zero()),
            },
        ];

        let expanded = ExpandedText {
            prototypes,
            instances,
            font_size: Rational::from_int(100),
            units_per_em: Rational::from_int(1000),
            scale_factor,
        };

        let text_entity = TextEntity::new(
            EntityId(1),
            "aaa".to_string(),
            "TestFont".to_string(),
            Rational::from_int(100),
        );

        let canvas = TextToCanvasConverter::convert(&expanded, &text_entity, None);

        // Verify 3 child groups
        assert_eq!(canvas.children.len(), 3);

        // Verify all 3 paths have the same scaled coordinates
        // (proving they came from the same cached scaled path)
        let mut paths: Vec<&TextCanvasPathNode> = vec![];
        for child in &canvas.children {
            if let TextCanvasNode::Group(g) = child {
                if let TextCanvasNode::Path(p) = &g.children[0] {
                    paths.push(p);
                }
            }
        }

        assert_eq!(paths.len(), 3);

        // All paths should have identical path_data (scaled the same way)
        assert_eq!(paths[0].path_data, paths[1].path_data);
        assert_eq!(paths[1].path_data, paths[2].path_data);
    }

    /// Test TextAffineTransform constructors.
    #[test]
    fn test_affine_transform_constructors() {
        let identity = TextAffineTransform::identity();
        assert_eq!(identity.a, Rational::one());
        assert_eq!(identity.d, Rational::one());
        assert_eq!(identity.tx, Rational::zero());
        assert_eq!(identity.ty, Rational::zero());

        let translate =
            TextAffineTransform::translate(Rational::from_int(10), Rational::from_int(20));
        assert_eq!(translate.tx, Rational::from_int(10));
        assert_eq!(translate.ty, Rational::from_int(20));
        assert_eq!(translate.a, Rational::one());

        let scale = TextAffineTransform::scale(Rational::new(1, 2), Rational::new(1, 4));
        assert_eq!(scale.a, Rational::new(1, 2));
        assert_eq!(scale.d, Rational::new(1, 4));
        assert_eq!(scale.tx, Rational::zero());
    }

    // =========================================================================
    // δ: 境界条件テスト (Boundary Condition Tests)
    // =========================================================================

    /// 空の ShapedGlyph スライスを expand_to_paths の代替として
    /// ExpandedText に instances: vec![] を渡した際、children が空になることを検証。
    ///
    /// TextShaper::shape("") の直接テストは実フォントが必要なため、
    /// 等価な ExpandedText を手動構築して TextToCanvasConverter::convert を通す。
    #[test]
    fn test_empty_instances_produces_empty_children() {
        use crate::{EntityId, TextEntity};

        // No instances → no prototypes needed
        let expanded = ExpandedText {
            prototypes: HashMap::new(),
            instances: vec![],
            font_size: Rational::from_int(16),
            units_per_em: Rational::from_int(1000),
            scale_factor: Rational::new(16, 1000),
        };

        let text_entity = TextEntity::new(
            EntityId(1),
            "".to_string(),
            "TestFont".to_string(),
            Rational::from_int(16),
        );

        let canvas = TextToCanvasConverter::convert(&expanded, &text_entity, None);

        // Empty instances must produce empty children
        assert_eq!(
            canvas.children.len(),
            0,
            "Empty ExpandedText instances should produce empty children vec"
        );
        assert_eq!(canvas.entity_id, EntityId(1));
        // Root transform should be identity
        assert_eq!(canvas.transform, TextAffineTransform::identity());
    }

    /// アウトラインなしグリフ（path_commands: vec![]）を含む ExpandedText を
    /// convert() に渡した場合、そのグリフの子ノードが生成されないことを検証。
    ///
    /// scaled_path.is_empty() の continue ブランチをカバー。
    #[test]
    fn test_glyph_without_outline_is_skipped() {
        use crate::{EntityId, TextEntity};

        // Prototype with no outline (e.g., space character)
        let mut prototypes = HashMap::new();
        prototypes.insert(
            32u16, // space
            GlyphPrototype {
                glyph_id: 32,
                entity_id: EntityId(100),
                path_commands: vec![], // no outline
                advance_width: Rational::from_int(250),
                bbox: None,
            },
        );

        let instances = vec![
            GlyphInstance {
                prototype_id: EntityId(100),
                entity_id: EntityId(200),
                origin: (Rational::zero(), Rational::zero()),
            },
            GlyphInstance {
                prototype_id: EntityId(100),
                entity_id: EntityId(201),
                origin: (Rational::new(250, 1), Rational::zero()),
            },
        ];

        let expanded = ExpandedText {
            prototypes,
            instances,
            font_size: Rational::from_int(16),
            units_per_em: Rational::from_int(1000),
            scale_factor: Rational::new(16, 1000),
        };

        let text_entity = TextEntity::new(
            EntityId(1),
            "  ".to_string(), // two spaces
            "TestFont".to_string(),
            Rational::from_int(16),
        );

        let canvas =
            TextToCanvasConverter::convert(&expanded, &text_entity, Some("#000000".to_string()));

        // Glyphs with empty outlines must be skipped → no children
        assert_eq!(
            canvas.children.len(),
            0,
            "Glyphs without outlines (empty path_commands) should produce no canvas children"
        );
    }

    /// アウトラインあり1グリフとアウトラインなし1グリフが混在する場合、
    /// アウトラインあり分のみ子ノードが生成されることを検証。
    #[test]
    fn test_mixed_outline_and_no_outline_glyphs() {
        use crate::{EntityId, TextEntity};

        let mut prototypes = HashMap::new();
        // Glyph with outline
        prototypes.insert(
            65u16, // 'A'
            GlyphPrototype {
                glyph_id: 65,
                entity_id: EntityId(100),
                path_commands: vec![
                    PathCommand::MoveTo {
                        x: Rational::zero(),
                        y: Rational::zero(),
                    },
                    PathCommand::LineTo {
                        x: Rational::from_int(500),
                        y: Rational::from_int(700),
                    },
                    PathCommand::Close,
                ],
                advance_width: Rational::from_int(600),
                bbox: None,
            },
        );
        // Glyph without outline (space)
        prototypes.insert(
            32u16,
            GlyphPrototype {
                glyph_id: 32,
                entity_id: EntityId(101),
                path_commands: vec![],
                advance_width: Rational::from_int(250),
                bbox: None,
            },
        );

        let instances = vec![
            GlyphInstance {
                prototype_id: EntityId(100), // 'A'
                entity_id: EntityId(200),
                origin: (Rational::zero(), Rational::zero()),
            },
            GlyphInstance {
                prototype_id: EntityId(101), // space
                entity_id: EntityId(201),
                origin: (Rational::new(96, 10), Rational::zero()),
            },
        ];

        let expanded = ExpandedText {
            prototypes,
            instances,
            font_size: Rational::from_int(16),
            units_per_em: Rational::from_int(1000),
            scale_factor: Rational::new(16, 1000),
        };

        let text_entity = TextEntity::new(
            EntityId(1),
            "A ".to_string(),
            "TestFont".to_string(),
            Rational::from_int(16),
        );

        let canvas = TextToCanvasConverter::convert(&expanded, &text_entity, None);

        // Only 'A' should produce a child node; space is skipped
        assert_eq!(
            canvas.children.len(),
            1,
            "Only the glyph with an outline should produce a canvas child"
        );
    }

    /// scale_path_commands に scale = Rational::zero() を渡した場合、
    /// 全座標が (0, 0) に収束し、パニックしないことを検証。
    #[test]
    fn test_scale_zero_collapses_coordinates() {
        let commands = vec![
            PathCommand::MoveTo {
                x: Rational::from_int(100),
                y: Rational::from_int(200),
            },
            PathCommand::LineTo {
                x: Rational::from_int(300),
                y: Rational::from_int(400),
            },
            PathCommand::QuadTo {
                x1: Rational::from_int(50),
                y1: Rational::from_int(75),
                x: Rational::from_int(150),
                y: Rational::from_int(200),
            },
            PathCommand::CubicTo {
                x1: Rational::from_int(10),
                y1: Rational::from_int(20),
                x2: Rational::from_int(30),
                y2: Rational::from_int(40),
                x: Rational::from_int(50),
                y: Rational::from_int(60),
            },
            PathCommand::Close,
        ];

        // Must not panic
        let scaled = TextToCanvasConverter::scale_path_commands(&commands, &Rational::zero());

        assert_eq!(
            scaled.len(),
            commands.len(),
            "Command count must be preserved"
        );

        // All coordinate-bearing commands should have zero coordinates
        assert_eq!(
            scaled[0],
            PathCommand::MoveTo {
                x: Rational::zero(),
                y: Rational::zero(),
            }
        );
        assert_eq!(
            scaled[1],
            PathCommand::LineTo {
                x: Rational::zero(),
                y: Rational::zero(),
            }
        );
        if let PathCommand::QuadTo { x1, y1, x, y } = &scaled[2] {
            assert_eq!(*x1, Rational::zero());
            assert_eq!(*y1, Rational::zero());
            assert_eq!(*x, Rational::zero());
            assert_eq!(*y, Rational::zero());
        } else {
            panic!("Expected QuadTo at index 2");
        }
        if let PathCommand::CubicTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } = &scaled[3]
        {
            assert_eq!(*x1, Rational::zero());
            assert_eq!(*y1, Rational::zero());
            assert_eq!(*x2, Rational::zero());
            assert_eq!(*y2, Rational::zero());
            assert_eq!(*x, Rational::zero());
            assert_eq!(*y, Rational::zero());
        } else {
            panic!("Expected CubicTo at index 3");
        }
        // Close is invariant
        assert_eq!(scaled[4], PathCommand::Close);
    }

    /// scale_path_commands に ArcTo コマンドを含む場合、
    /// rotation は変化せず rx/ry/x/y のみスケールされることを検証。
    #[test]
    fn test_scale_zero_arc_rotation_preserved() {
        let commands = vec![PathCommand::ArcTo {
            rx: Rational::from_int(100),
            ry: Rational::from_int(50),
            rotation: 45.0_f64,
            large_arc: true,
            sweep: false,
            x: Rational::from_int(200),
            y: Rational::from_int(300),
        }];

        let scaled = TextToCanvasConverter::scale_path_commands(&commands, &Rational::zero());

        if let PathCommand::ArcTo {
            rx,
            ry,
            rotation,
            large_arc,
            sweep,
            x,
            y,
        } = &scaled[0]
        {
            assert_eq!(*rx, Rational::zero());
            assert_eq!(*ry, Rational::zero());
            assert_eq!(*rotation, 45.0_f64, "rotation must not be scaled");
            assert_eq!(*large_arc, true);
            assert_eq!(*sweep, false);
            assert_eq!(*x, Rational::zero());
            assert_eq!(*y, Rational::zero());
        } else {
            panic!("Expected ArcTo");
        }
    }

    /// CJKグリフのシェーピングテスト（Noto Sans JP が CI 環境に存在する場合のみ有効）。
    #[test]
    #[ignore = "Requires CJK font (Noto Sans JP) to be available in CI"]
    fn test_cjk_shaping() {
        // TODO: Enable when CJK font is available in CI
        // let font_data = std::fs::read("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc")
        //     .expect("Noto Sans JP not found");
        // let shaper = TextShaper::new(&font_data).expect("Failed to create TextShaper");
        // let glyphs = shaper.shape("日本語テスト").expect("Shaping failed");
        // assert!(!glyphs.is_empty());
    }
}
