# cosmic-text Investigation Report

## Track 5: Text Rendering Integration for vsc-gpu

**Investigation Date:** 2026-05-12
**Status:** Research Complete

---

## 1. Crate Information

### cosmic-text
- **Current Version:** 0.19.0 (latest as of docs.rs)
- **License:** MIT OR Apache-2.0 (dual-licensed)
- **Repository:** https://github.com/pop-os/cosmic-text
- **Description:** "Pure Rust multi-line text handling" with advanced text shaping, layout, and rendering

### Key Dependencies
| Dependency | Purpose |
|------------|---------|
| fontdb 0.23 | Font discovery and database |
| harfrust 0.5.0 | Text shaping (pure Rust HarfBuzz port) |
| swash 0.2.6 | Glyph rasterization (optional) |
| unicode-bidi | Bidirectional text support |
| unicode-linebreak | Line breaking rules |
| unicode-script | Script detection |

### Feature Flags
- **Default:** `std`, `swash`, `fontconfig`
- **Available:** `no_std`, `monospace_fallback`, `shape-run-cache`, `wasm-web`, `warn_on_missing_glyphs`, `vi`, `peniko`

---

## 2. Core API Summary

### Type Hierarchy

```
FontSystem (application singleton)
    |
    +-- Database (fontdb) - font storage
    |
    +-- Buffer (per text widget)
            |
            +-- Metrics - font size & line height
            +-- BufferLine[] - individual lines
            +-- LayoutRun[] - shaped glyph runs
                    |
                    +-- LayoutGlyph - positioned glyphs

SwashCache (application singleton)
    |
    +-- get_image() -> SwashImage (rasterized glyph)
```

### Key Types

#### FontSystem
- Created once per application
- Manages font discovery and fallback
- **Warning:** `FontSystem::new()` scans all system fonts - can take 1+ seconds

```rust
// Fast initialization with custom fonts only
let mut db = fontdb::Database::new();
db.load_font_file("path/to/font.ttf")?;
db.load_font_data(include_bytes!("embedded.otf"));
let font_system = FontSystem::new_with_locale_and_db("en-US", db);
```

#### Buffer
- One per text widget
- Handles shaping and layout

```rust
let mut buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 20.0));
buffer.set_size(&mut font_system, Some(800.0), Some(600.0));
buffer.set_text(&mut font_system, "Hello, world!", Attrs::new(), Shaping::Advanced);
```

#### SwashCache
- Caches rasterized glyphs
- Supports three output formats:
  - **Mask:** 8-bit alpha (1 byte/pixel)
  - **SubpixelMask:** 32-bit RGBA for LCD subpixel (4 bytes/pixel)
  - **Color:** 32-bit RGBA for emoji (4 bytes/pixel)

#### Metrics
```rust
Metrics::new(font_size: f32, line_height: f32)
Metrics::relative(font_size: f32, line_height_scale: f32)
```

---

## 3. Integration Path: wgpu via glyphon

### Recommended Approach
Use **glyphon** (https://github.com/grovesNL/glyphon) - a thin wrapper that integrates cosmic-text with wgpu.

- **Version:** 0.11.0
- **License:** MIT OR Apache-2.0 OR Zlib
- **wgpu Version:** 29.0.0

### glyphon Architecture
1. cosmic-text handles shaping/layout
2. etagere packs glyphs into texture atlas
3. wgpu renders glyph quads

### Key glyphon Types
| Type | Purpose |
|------|---------|
| TextAtlas | GPU texture atlas for cached glyphs |
| TextRenderer | Prepares and renders text in render pass |
| TextArea | Defines text region with bounds/overflow |
| Viewport | Clipping and resolution management |
| Cache | Shared pipelines and shaders |

### Integration Pseudocode

```rust
// === Initialization (once) ===
let mut font_system = FontSystem::new();  // or custom fonts
let mut swash_cache = SwashCache::new();
let cache = Cache::new(&device);
let mut atlas = TextAtlas::new(&device, &queue, &cache, format, ColorMode::Accurate);
let mut text_renderer = TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
let mut viewport = Viewport::new(&device, &cache);

// === Per-frame ===
// 1. Update viewport
viewport.update(&queue, Resolution { width, height });

// 2. Prepare text
let mut buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 20.0));
buffer.set_text(&mut font_system, text, Attrs::new(), Shaping::Advanced);
buffer.shape_until_scroll(&mut font_system, false);

text_renderer.prepare(
    &device, &queue, &mut font_system, &mut atlas, &viewport,
    [TextArea {
        buffer: &buffer,
        left: 10.0,
        top: 10.0,
        scale: 1.0,
        bounds: TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 },
        default_color: Color::rgb(255, 255, 255),
        custom_glyphs: &[],
    }],
    &mut swash_cache,
)?;

// 3. Render in pass
text_renderer.render(&atlas, &viewport, &mut render_pass)?;
```

### Direct Integration (without glyphon)
If more control is needed, implement manually:

```rust
// 1. Iterate layout runs
for run in buffer.layout_runs() {
    for glyph in run.glyphs {
        // glyph.physical((x, y), scale) -> (cache_key, x, y)
        let (cache_key, glyph_x, glyph_y) = glyph.physical((0.0, 0.0), 1.0);

        // 2. Rasterize to image
        let image = swash_cache.get_image(&mut font_system, cache_key);

        // 3. Upload to atlas texture (manage yourself)
        // 4. Generate quad vertices with UV coordinates
    }
}
```

---

## 4. Constraints Verification

### Pure Rust Operation (No web_sys)
**VERIFIED:** cosmic-text is pure Rust with explicit `no_std` support:
```rust
#[cfg_attr(not(feature = "std"), no_std)]
```

The `wasm-web` feature only enables `sys-locale/js` for locale detection - not required for core functionality. No web_sys dependency in the core library.

### Custom Font Loading
**VERIFIED:** Full support via fontdb:
```rust
// From file
db.load_font_file("path/to/font.ttf")?;

// From bytes (embedded fonts)
db.load_font_data(include_bytes!("font.otf"));

// From directory (recursive)
db.load_fonts_dir("fonts/");
```

### Unicode/CJK Support
**VERIFIED with caveats:**
- Full Unicode support via harfrust shaping
- CJK text works correctly
- **Known Issue #485:** Korean Hangul jamo may render decomposed if:
  - IME passes decomposed text (toolkit issue, not cosmic-text)
  - Font lacks pre-composed glyphs
- **Workaround:** Use fonts with full composed glyph support (e.g., Noto Sans CJK)

---

## 5. Performance Characteristics

### FontSystem Initialization
**CONCERN:** `FontSystem::new()` is slow (1+ seconds on typical systems)
- Cause: Memory-maps all system fonts in `/usr/share/fonts`
- Issue #505 tracks this as a regression

**Mitigation:**
```rust
// Use empty database + explicit fonts for fast startup
let mut db = fontdb::Database::new();
db.load_font_data(include_bytes!("bundled_font.ttf"));
let font_system = FontSystem::new_with_locale_and_db("en-US", db);
```

### Glyph Caching
- SwashCache uses FxHashMap for O(1) lookups
- Cache key includes: font_id, glyph_id, size, subpixel offset
- Both cached and uncached rasterization available

### Memory Footprint
- Font data: ~1-5MB per font family loaded
- Glyph atlas: Depends on unique glyphs rendered
- glyphon uses LRU eviction for atlas management

### Shaping Performance
- harfrust is pure Rust port of HarfBuzz
- Comparable performance to native HarfBuzz
- Complex scripts (Arabic, Indic) fully supported

---

## 6. Risks and Concerns

### High Priority
| Risk | Severity | Mitigation |
|------|----------|------------|
| FontSystem::new() startup time | Medium | Use custom font database, avoid system font scan |
| wgpu version coupling (glyphon requires 29.0) | Medium | Pin versions or maintain fork |

### Medium Priority
| Risk | Severity | Notes |
|------|----------|-------|
| Korean Hangul decomposition | Low | Font selection solves this |
| No COLRv1 font support (#446) | Low | Color emoji still work via bitmap fallback |
| Pixel font alignment (#493) | Low | Affects retro/pixel fonts only |

### Low Priority
- Variable font axes incomplete (#406)
- Font unloading API unclear (#489)

---

## 7. Recommendation

### Decision: GO for Phase E Text Implementation

**Rationale:**
1. **Pure Rust:** No web_sys dependency, works in native and WASM contexts
2. **Proven Integration:** glyphon provides battle-tested wgpu integration
3. **Full Unicode:** CJK, RTL, complex scripts all supported
4. **Active Maintenance:** Regular releases, responsive maintainers (Pop!_OS team)
5. **Flexible Fonts:** Load from bytes/files/directories, skip slow system scan

**Implementation Strategy:**
1. Use **glyphon** for initial integration (simplest path)
2. Bundle fonts with application (avoid FontSystem::new() slowness)
3. If glyphon proves limiting, extract patterns for custom atlas management
4. Pin cosmic-text and glyphon versions to avoid API churn

### Suggested Dependencies for Phase E
```toml
[dependencies]
cosmic-text = { version = "0.19", default-features = false, features = ["swash"] }
glyphon = "0.11"
```

### Alternative Considered
- **fontdue + manual atlas:** More control but significant implementation effort
- **ab_glyph:** Simpler API but less feature-complete (no shaping)

cosmic-text + glyphon provides the best balance of features and integration effort.

---

## Appendix: Reference Links

- cosmic-text docs: https://docs.rs/cosmic-text
- glyphon docs: https://docs.rs/glyphon
- fontdb docs: https://docs.rs/fontdb
- swash docs: https://docs.rs/swash
- GitHub Issues: https://github.com/pop-os/cosmic-text/issues
