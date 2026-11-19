//! Fonts for the portable page renderer: HarfRust shapes, Skrifa reads the
//! unchanged outlines, and glyphs are cached as tiny-skia paths in font units.
//! Nothing here depends on the operating system.
use crate::data::Result;
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
    raw::TableProvider,
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use tiny_skia::{Path, PathBuilder, Rect as SkRect, Transform};

/// A shaped glyph: id, advance and offsets in font units, and its UTF-8 cluster.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub id: u16,
    pub advance: f32,
    pub x: f32,
    pub y: f32,
    pub cluster: usize,
}

/// One glyph's cached outline (font units, y down) and its tight ink box.
#[derive(Clone)]
pub struct Outline {
    pub path: Rc<Path>,
    pub bounds: SkRect,
}

struct Pen(PathBuilder);
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, -y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, -y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(cx0, -cy0, x, -y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, -cy0, cx1, -cy1, x, -y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

pub struct Font {
    font: FontRef<'static>,
    shaper: harfrust::Font,
    pub units_per_em: f32,
    /// DirectWrite's line box for this font, in em: ascent, descent, line gap.
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    outlines: RefCell<HashMap<u16, Option<Outline>>>,
    /// Shaped runs by text and direction. Laying out a page fits its size and
    /// stretches its spaces by shaping the same lines several times; shaping
    /// does not depend on either, so each line is shaped once.
    shaped: RefCell<HashMap<(String, bool), Vec<Glyph>>>,
}

impl Font {
    pub fn load(path: &std::path::Path, limit: u64) -> Result<Self> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(limit + 1)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() as u64 > limit {
            return Err(format!("{} exceeds its read limit", path.display()).into());
        }
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        // The parsed tables borrow the bytes for the life of the app.
        let leaked: &'static [u8] = Box::leak(bytes.clone().into_boxed_slice());
        let font = FontRef::new(leaked)?;
        let units_per_em = font.head()?.units_per_em() as f32;
        let os2 = font.os2()?;
        let hhea = font.hhea()?;
        // DirectWrite: typographic metrics when USE_TYPO_METRICS is set,
        // otherwise the Windows ascent and descent with the hhea line gap. The
        // whole line gap sits above the ascent (measured against DirectWrite).
        let (ascent, descent, line_gap) = if os2.fs_selection().bits() & (1 << 7) != 0 {
            (
                os2.s_typo_ascender() as f32,
                -(os2.s_typo_descender() as f32),
                os2.s_typo_line_gap() as f32,
            )
        } else {
            (
                os2.us_win_ascent() as f32,
                os2.us_win_descent() as f32,
                hhea.line_gap().to_i16().max(0) as f32,
            )
        };
        Ok(Self {
            font,
            shaper: harfrust::Font::new(bytes, 0).ok_or("Invalid font for shaping")?,
            units_per_em,
            ascent: ascent / units_per_em,
            descent: descent / units_per_em,
            line_gap: line_gap / units_per_em,
            outlines: RefCell::new(HashMap::new()),
            shaped: RefCell::new(HashMap::new()),
        })
    }

    /// Shapes one run. Right-to-left runs come back in visual order.
    pub fn shape(&self, text: &str, rtl: bool) -> Result<Vec<Glyph>> {
        if text.is_empty() || text.len() > 16 * 1024 {
            return Err("Shaping text exceeds its read limit".into());
        }
        let key = (text.to_owned(), rtl);
        if let Some(glyphs) = self.shaped.borrow().get(&key) {
            return Ok(glyphs.clone());
        }
        let glyphs = self.shape_uncached(text, rtl)?;
        // Bounded: about the lines of a dozen pages and their labels.
        let mut cache = self.shaped.borrow_mut();
        if cache.len() >= 1024 {
            cache.clear();
        }
        cache.insert(key, glyphs.clone());
        Ok(glyphs)
    }

    fn shape_uncached(&self, text: &str, rtl: bool) -> Result<Vec<Glyph>> {
        let font = harfrust::ShaperFont::new(&self.shaper);
        let mut buffer = harfrust::Buffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        buffer.set_direction(if rtl {
            harfrust::Direction::RightToLeft
        } else {
            harfrust::Direction::LeftToRight
        });
        harfrust::shape(&font, &mut buffer, harfrust::ShapeOptions::default())?;
        if buffer.glyph_infos().len() > 4096 {
            return Err("Shaping output exceeds its glyph limit".into());
        }
        Ok(buffer
            .glyph_infos()
            .iter()
            .zip(buffer.glyph_positions())
            .map(|(info, position)| Glyph {
                id: info.glyph_id as u16,
                advance: position.x_advance as f32,
                x: position.x_offset as f32,
                y: position.y_offset as f32,
                cluster: info.cluster as usize,
            })
            .collect())
    }

    /// The glyph for one character, without shaping.
    pub fn glyph_for(&self, c: char) -> Option<u16> {
        self.font
            .charmap()
            .map(c)
            .map(|g| g.to_u32() as u16)
            .filter(|g| *g != 0)
    }

    pub fn outline(&self, id: u16) -> Result<Option<Outline>> {
        if let Some(cached) = self.outlines.borrow().get(&id) {
            return Ok(cached.clone());
        }
        let glyph = self.font.outline_glyphs().get(GlyphId::new(id as u32));
        let outline = match glyph {
            Some(glyph) => {
                let mut pen = Pen(PathBuilder::new());
                glyph.draw(
                    DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
                    &mut pen,
                )?;
                pen.0.finish().and_then(|path| {
                    path.compute_tight_bounds().map(|bounds| Outline {
                        path: Rc::new(path),
                        bounds,
                    })
                })
            }
            None => None,
        };
        // A bounded cache: each font has a few thousand glyphs at most.
        let mut cache = self.outlines.borrow_mut();
        if cache.len() > 8192 {
            cache.clear();
        }
        cache.insert(id, outline.clone());
        Ok(outline)
    }

    /// COLR version 0 layers of a glyph: each layer glyph and the lightness
    /// of its palette colour (0 for the foreground colour), as DirectWrite's
    /// TranslateColorGlyphRun reports them with the default palette.
    pub fn color_layers(&self, id: u16) -> Result<Vec<(u16, f32)>> {
        let colr = self.font.colr()?;
        let cpal = self.font.cpal()?;
        let colors = cpal
            .color_records_array()
            .ok_or("Missing colour palette")??;
        let first = cpal
            .color_record_indices()
            .first()
            .map(|i| i.get())
            .unwrap_or(0) as usize;
        let base = colr
            .base_glyph_records()
            .ok_or("Missing colour glyphs")??
            .iter()
            .find(|record| record.glyph_id().to_u32() == id as u32)
            .ok_or("Glyph has no colour layers")?;
        let layers = colr.layer_records().ok_or("Missing colour layers")??;
        let start = base.first_layer_index() as usize;
        let count = base.num_layers() as usize;
        if count > 32 {
            return Err("Colour glyph has too many layers".into());
        }
        let mut out = Vec::with_capacity(count);
        for layer in layers
            .get(start..start + count)
            .ok_or("Invalid colour layer range")?
        {
            let palette = layer.palette_index();
            let lightness = if palette == 0xffff {
                0.
            } else {
                let c = colors
                    .get(first + palette as usize)
                    .ok_or("Invalid palette colour")?;
                (0.2126 * c.red as f32 + 0.7152 * c.green as f32 + 0.0722 * c.blue as f32) / 255.
            };
            out.push((layer.glyph_id().to_u32() as u16, lightness.clamp(0., 1.)));
        }
        Ok(out)
    }
}

/// Glyphs placed for drawing: id and baseline origin, in page units.
#[derive(Clone)]
pub struct Placed {
    pub id: u16,
    pub x: f32,
    pub y: f32,
}

/// Places shaped glyphs from a pen position and baseline, at `size` page
/// units per em, adding `extra` after every glyph `is_space` marks.
pub fn place(
    glyphs: &[Glyph],
    is_space: &[bool],
    font: &Font,
    size: f32,
    extra: f32,
    pen_x: f32,
    baseline: f32,
) -> (Vec<Placed>, f32) {
    let scale = size / font.units_per_em;
    let mut pen = pen_x;
    let mut out = Vec::with_capacity(glyphs.len());
    for (i, g) in glyphs.iter().enumerate() {
        out.push(Placed {
            id: g.id,
            x: pen + g.x * scale,
            y: baseline - g.y * scale,
        });
        pen += g.advance * scale
            + if is_space.get(i).copied().unwrap_or(false) {
                extra
            } else {
                0.
            };
    }
    (out, pen - pen_x)
}

/// The union of the placed glyphs' ink, in page units.
pub fn ink_bounds(font: &Font, placed: &[Placed], size: f32) -> Result<Option<SkRect>> {
    let scale = size / font.units_per_em;
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    for p in placed {
        if let Some(outline) = font.outline(p.id)? {
            let b = outline.bounds;
            let (l, t, r, btm) = (
                p.x + b.left() * scale,
                p.y + b.top() * scale,
                p.x + b.right() * scale,
                p.y + b.bottom() * scale,
            );
            bounds = Some(match bounds {
                None => (l, t, r, btm),
                Some((a, b2, c, d)) => (a.min(l), b2.min(t), c.max(r), d.max(btm)),
            });
        }
    }
    Ok(bounds.and_then(|(l, t, r, b)| SkRect::from_ltrb(l, t, r, b)))
}

/// Appends placed glyph outlines to a path, through `transform` (page units
/// to wherever the caller draws).
pub fn append(
    builder: &mut PathBuilder,
    font: &Font,
    placed: &[Placed],
    size: f32,
    transform: Transform,
) -> Result<()> {
    let scale = size / font.units_per_em;
    for p in placed {
        if let Some(outline) = font.outline(p.id)? {
            let local = Transform::from_row(scale, 0., 0., scale, p.x, p.y).post_concat(transform);
            if let Some(path) = outline.path.as_ref().clone().transform(local) {
                builder.push_path(&path);
            }
        }
    }
    Ok(())
}
