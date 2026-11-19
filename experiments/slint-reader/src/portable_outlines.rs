//! Contextual letter elongation, portably: Digital Khatt's own justified
//! outlines, pinned at build time in `experimental/digital-khatt/shaped`,
//! placed exactly as the native outlines.rs places them and rasterized with
//! tiny-skia. No source text, glyph or elongation choice is regenerated.
use crate::app_core::Rect;
use crate::data::{Line, Result};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    rc::Rc,
};
use tiny_skia::{Path, PathBuilder, Rect as SkRect, Transform};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shape {
    width: f32,
    scale: f32,
    shift: f32,
    #[serde(default)]
    baseline_offset: f32,
    glyphs: Vec<[f32; 4]>,
    word_bounds: BTreeMap<u32, [f32; 2]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShapedSource {
    page_number: u32,
    lines: BTreeMap<u32, Shape>,
}

/// A page's lines as single paths in normalized line coordinates (x from 0
/// to 1 across the line, y from its middle, in line widths), and the row fit.
pub struct ShapedPage {
    pub lines: BTreeMap<u32, (Shape, Rc<Path>)>,
    gaps: Vec<(u32, f32)>,
}

impl ShapedPage {
    /// Shrink the line width when glyphs of neighbouring rows would touch.
    pub fn fit(&self, width: f32, row: f32) -> f32 {
        let mut fit = 1f32;
        for (distance, overlap) in &self.gaps {
            let available = row * *distance as f32 - row * (3. / 57.4);
            fit = fit.min(available / (overlap * width).max(1.));
        }
        fit.min(1.)
    }
}

/// One SVG path (absolute M, L, C, Q, Z) in font units, y up.
fn parse(source: &str) -> Result<Path> {
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut command = b' ';
    let mut builder = PathBuilder::new();
    let number = |at: &mut usize| -> Result<f32> {
        while *at < bytes.len() && (bytes[*at].is_ascii_whitespace() || bytes[*at] == b',') {
            *at += 1;
        }
        let start = *at;
        while *at < bytes.len()
            && (bytes[*at].is_ascii_digit()
                || matches!(bytes[*at], b'.' | b'-' | b'+' | b'e' | b'E'))
        {
            if *at > start
                && matches!(bytes[*at], b'-' | b'+')
                && !matches!(bytes[*at - 1], b'e' | b'E')
            {
                break;
            }
            *at += 1;
        }
        let value: f32 = source[start..*at].parse()?;
        if !value.is_finite() || value.abs() > 1_000_000. {
            return Err("Invalid source path coordinate".into());
        }
        Ok(value)
    };
    loop {
        while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b',') {
            at += 1;
        }
        if at >= bytes.len() {
            break;
        }
        if bytes[at].is_ascii_alphabetic() {
            command = bytes[at];
            at += 1;
        }
        match command {
            b'M' => {
                let (x, y) = (number(&mut at)?, number(&mut at)?);
                builder.move_to(x, y);
                command = b'L';
            }
            b'L' => {
                let (x, y) = (number(&mut at)?, number(&mut at)?);
                builder.line_to(x, y);
            }
            b'Q' => {
                let v = [
                    number(&mut at)?,
                    number(&mut at)?,
                    number(&mut at)?,
                    number(&mut at)?,
                ];
                builder.quad_to(v[0], v[1], v[2], v[3]);
            }
            b'C' => {
                let v = [
                    number(&mut at)?,
                    number(&mut at)?,
                    number(&mut at)?,
                    number(&mut at)?,
                    number(&mut at)?,
                    number(&mut at)?,
                ];
                builder.cubic_to(v[0], v[1], v[2], v[3], v[4], v[5]);
            }
            b'Z' | b'z' => {
                builder.close();
                command = b' ';
            }
            _ => return Err("Unsupported source path command".into()),
        }
    }
    builder.finish().ok_or_else(|| "Empty source path".into())
}

pub struct Outlines {
    root: PathBuf,
    glyphs: BTreeMap<u32, (Path, SkRect)>,
    pages: VecDeque<(u32, Rc<ShapedPage>)>,
}

impl Outlines {
    pub fn new(root: PathBuf) -> Result<Self> {
        let source: BTreeMap<String, String> =
            crate::data::read_json(&root.join("experimental/digital-khatt/shaped/glyphs.json"))?;
        let mut glyphs = BTreeMap::new();
        for (key, path) in source {
            let path = parse(&path)?;
            let bounds = path.compute_tight_bounds().ok_or("Empty source glyph")?;
            glyphs.insert(key.parse()?, (path, bounds));
        }
        Ok(Self {
            root,
            glyphs,
            pages: VecDeque::new(),
        })
    }

    /// A page checked against its source lines: every word has bounds.
    pub fn page(&mut self, number: u32, source: &crate::data::Page) -> Result<Rc<ShapedPage>> {
        if let Some(index) = self.pages.iter().position(|(n, _)| *n == number) {
            let item = self.pages.remove(index).ok_or("Missing cached page")?;
            let page = item.1.clone();
            self.pages.push_back(item);
            return Ok(page);
        }
        let shaped: ShapedSource = crate::data::read_json(
            &self
                .root
                .join(format!("experimental/digital-khatt/shaped/p{number}.json")),
        )?;
        if shaped.page_number != number || shaped.lines.keys().any(|n| !(1..=15).contains(n)) {
            return Err("Wrong shaped page".into());
        }
        let mut lines = BTreeMap::new();
        let mut ink: BTreeMap<u32, Vec<SkRect>> = BTreeMap::new();
        for (n, shape) in shaped.lines {
            if !shape.width.is_finite()
                || shape.width <= 0.
                || !shape.scale.is_finite()
                || shape.scale <= 0.
            {
                return Err("Invalid shaped line".into());
            }
            if let Some(line) = source.lines.iter().find(|l| l.line_number == n)
                && line
                    .words
                    .iter()
                    .any(|w| !shape.word_bounds.contains_key(&w.id))
            {
                return Err("Shaped line does not match its source words".into());
            }
            let mut builder = PathBuilder::new();
            let scale = shape.scale / shape.width;
            for glyph in &shape.glyphs {
                let (path, bounds) = self
                    .glyphs
                    .get(&(glyph[0] as u32))
                    .ok_or("Missing source glyph")?;
                // outlines.rs: x = 1 + (shift + scale * gx) / width, y flipped.
                let transform = Transform::from_row(
                    scale,
                    0.,
                    0.,
                    -scale,
                    1. + (shape.shift + shape.scale * glyph[1]) / shape.width,
                    (350. - shape.scale * glyph[2]) / shape.width + shape.baseline_offset / 513.,
                );
                if let Some(placed) = path.clone().transform(transform) {
                    builder.push_path(&placed);
                }
                if let Some(b) = bounds.transform(transform) {
                    ink.entry(n).or_default().push(b);
                }
            }
            if let Some(path) = builder.finish() {
                lines.insert(n, (shape, Rc::new(path)));
            }
        }
        // Vertical overlap of horizontally overlapping glyphs in neighbouring rows.
        let mut gaps = Vec::new();
        let numbers: Vec<_> = ink.keys().copied().collect();
        for pair in numbers.windows(2) {
            let mut overlap = f32::NEG_INFINITY;
            for a in &ink[&pair[0]] {
                for b in &ink[&pair[1]] {
                    if a.right() >= b.left() && b.right() >= a.left() {
                        overlap = overlap.max(a.bottom() - b.top());
                    }
                }
            }
            if overlap.is_finite() {
                gaps.push((pair[1] - pair[0], overlap));
            }
        }
        let page = Rc::new(ShapedPage { lines, gaps });
        self.pages.push_back((number, page.clone()));
        while self.pages.len() > 12 {
            self.pages.pop_front();
        }
        Ok(page)
    }

    /// Word rectangles from the source word bounds, as outlines.rs makes them.
    pub fn hit_regions(shape: &Shape, line: &Line, rect: Rect) -> Result<Vec<(Rect, (u32, u32))>> {
        let mut regions = Vec::new();
        for word in &line.words {
            let bounds = shape
                .word_bounds
                .get(&word.id)
                .ok_or("Missing source word bounds")?;
            let left = (bounds[0] * shape.scale + shape.shift + shape.width) / shape.width;
            let right = (bounds[1] * shape.scale + shape.shift + shape.width) / shape.width;
            if !left.is_finite() || !right.is_finite() || right < left {
                return Err("Invalid source word bounds".into());
            }
            regions.push((
                Rect {
                    x: rect.x + left * rect.w,
                    y: rect.y,
                    w: (right - left) * rect.w,
                    h: rect.h,
                },
                (word.surah, word.ayah),
            ));
        }
        Ok(regions)
    }
}
