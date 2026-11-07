//! Native painting of the existing, pinned Digital Khatt build-time outlines.
//! No source text, marks, glyph outlines or elongation choices are regenerated.
use crate::{
    data::{Line, Result, read_json},
    render::{Palette, Rect},
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::Arc,
};
use windows::Win32::Graphics::Direct2D::{Common::*, *};
use windows_numerics::{Matrix3x2, Vector2};
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
    #[serde(skip)]
    ink_glyphs: Vec<Rect>,
    #[serde(skip)]
    geometry_key: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapedPage {
    page_number: u32,
    pub lines: BTreeMap<u32, Shape>,
    #[serde(skip)]
    gaps: Vec<(u32, f32)>,
}
struct PreparedLine {
    curves: ID2D1PathGeometry,
    optical_stroke: Option<ID2D1PathGeometry>,
}
pub struct Outlines {
    root: PathBuf,
    source: BTreeMap<u32, String>,
    paths: BTreeMap<u32, ID2D1PathGeometry>,
    lines: BTreeMap<u32, PreparedLine>,
    source_validated: BTreeSet<u32>,
    pages: VecDeque<(u32, Arc<ShapedPage>)>,
    factory: ID2D1Factory,
    rounded_stroke: ID2D1StrokeStyle,
}
pub struct OutlineView {
    pub rect: Rect,
    pub palette: Palette,
}

fn normalized_ink_bounds(shape: &Shape, glyph: &[f32; 4], bounds: D2D_RECT_F) -> Result<Rect> {
    // Finite JSON values can still overflow when multiplied or divided. Reject
    // malformed optional transforms before caching or submitting them to D2D.
    // This ceiling matches the existing source path coordinate parser and is
    // deliberately far beyond the original font's normalized page coordinates.
    let components = [
        1. / shape.width,
        shape.scale / shape.width,
        shape.shift / shape.width,
        shape.baseline_offset / 513.,
        shape.scale * glyph[1] / shape.width,
        shape.scale * glyph[2] / shape.width,
    ];
    if components
        .iter()
        .any(|n| !n.is_finite() || n.abs() > 1_000_000.)
    {
        return Err("Invalid derived outline transform".into());
    }
    let rect = Rect {
        x: (shape.width + shape.shift + shape.scale * (glyph[1] + bounds.left)) / shape.width,
        y: (350. - shape.scale * (glyph[2] + bounds.bottom)) / shape.width
            + shape.baseline_offset / 513.,
        w: shape.scale * (bounds.right - bounds.left) / shape.width,
        h: shape.scale * (bounds.bottom - bounds.top) / shape.width,
    };
    if [rect.x, rect.y, rect.w, rect.h]
        .iter()
        .any(|n| !n.is_finite() || n.abs() > 1_000_000.)
        || rect.w < 0.
        || rect.h < 0.
    {
        return Err("Invalid derived outline bounds".into());
    }
    Ok(rect)
}

impl Outlines {
    pub fn new(root: PathBuf, factory: ID2D1Factory) -> Result<Self> {
        let source = read_json(&root.join("experimental/digital-khatt/shaped/glyphs.json"))?;
        let rounded_stroke = unsafe {
            factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )?
        };
        Ok(Self {
            root,
            source,
            paths: BTreeMap::new(),
            lines: BTreeMap::new(),
            source_validated: BTreeSet::new(),
            pages: VecDeque::new(),
            factory,
            rounded_stroke,
        })
    }
    pub fn page(&mut self, number: u32) -> Result<Arc<ShapedPage>> {
        if !(1..=604).contains(&number) {
            return Err("Invalid shaped page".into());
        }
        if let Some(index) = self.pages.iter().position(|(n, _)| *n == number) {
            let item = self.pages.remove(index).unwrap();
            let page = item.1.clone();
            self.pages.push_back(item);
            return Ok(page);
        }
        let mut page: ShapedPage = read_json(
            &self
                .root
                .join(format!("experimental/digital-khatt/shaped/p{number}.json")),
        )?;
        if page.page_number != number {
            return Err("Wrong shaped page reference".into());
        }
        if page.lines.len() > 15 || page.lines.keys().any(|n| !(1..=15).contains(n)) {
            return Err("Invalid shaped line reference".into());
        }
        for (line_number, shape) in &mut page.lines {
            if !shape.width.is_finite()
                || shape.width <= 0.
                || !shape.scale.is_finite()
                || shape.scale <= 0.
                || !shape.shift.is_finite()
                || !shape.baseline_offset.is_finite()
                || shape.glyphs.len() > 2000
                || shape.glyphs.iter().flatten().any(|n| !n.is_finite())
            {
                return Err("Invalid outline geometry".into());
            }
            // Measure the original curves once per cached page. The display
            // uses a uniform page scale so tall marks retain breathing room;
            // individual letters and the source outlines are never stretched.
            for glyph in &shape.glyphs {
                if glyph[0] < 0. || glyph[0].fract() != 0. {
                    return Err("Invalid source glyph ID".into());
                }
                let bounds = unsafe { self.geometry(glyph[0] as u32)?.GetBounds(None)? };
                shape
                    .ink_glyphs
                    .push(normalized_ink_bounds(shape, glyph, bounds)?);
            }
            // Record the original curves once in normalized line coordinates.
            // Simplify retains cubic curves: no bitmap or regenerated glyph.
            unsafe {
                let combined = self.factory.CreatePathGeometry()?;
                let sink = combined.Open()?;
                sink.SetFillMode(D2D1_FILL_MODE_WINDING);
                for glyph in &shape.glyphs {
                    let scale = shape.scale / shape.width;
                    let transform = Matrix3x2 {
                        M11: scale,
                        M22: -scale,
                        M31: 1. + (shape.shift + shape.scale * glyph[1]) / shape.width,
                        M32: (350. - shape.scale * glyph[2]) / shape.width
                            + shape.baseline_offset / 513.,
                        ..Matrix3x2::identity()
                    };
                    self.geometry(glyph[0] as u32)?.Simplify(
                        D2D1_GEOMETRY_SIMPLIFICATION_OPTION_CUBICS_AND_LINES,
                        Some(&transform),
                        0.001,
                        &sink,
                    )?;
                }
                sink.Close()?;
                shape.geometry_key = number * 16 + line_number;
                self.lines.insert(
                    shape.geometry_key,
                    PreparedLine {
                        curves: combined,
                        optical_stroke: None,
                    },
                );
            }
        }
        // The upstream shaper compares only glyphs that overlap horizontally.
        // Whole-line envelopes would shrink perfectly separated letters at
        // opposite ends of neighboring rows and make the page needlessly small.
        // Cache the costly pair comparisons once per page, not on every paint.
        let mut previous: Option<(u32, &Shape)> = None;
        for (number, shape) in &page.lines {
            if let Some((last, lower)) = previous {
                let mut overlap = f32::NEG_INFINITY;
                for a in &lower.ink_glyphs {
                    for b in &shape.ink_glyphs {
                        if a.x + a.w >= b.x && b.x + b.w >= a.x {
                            overlap = overlap.max(a.y + a.h - b.y);
                        }
                    }
                }
                if overlap.is_finite() {
                    page.gaps.push((*number - last, overlap));
                }
            }
            previous = Some((*number, shape));
        }
        let page = Arc::new(page);
        self.pages.push_back((number, page.clone()));
        while self.pages.len() > 12 {
            if let Some((number, _)) = self.pages.pop_front() {
                self.lines.retain(|key, _| key / 16 != number);
                self.source_validated.remove(&number);
            }
        }
        Ok(page)
    }
    pub fn validated_page(&mut self, source: &crate::data::Page) -> Result<Arc<ShapedPage>> {
        let page = self.page(source.page_number)?;
        if !self.source_validated.contains(&source.page_number) {
            Self::validate_source_page(&page, source)?;
            self.source_validated.insert(source.page_number);
        }
        Ok(page)
    }
    fn prepare_stroke(&mut self, key: u32) -> Result<()> {
        let line = self
            .lines
            .get_mut(&key)
            .ok_or("Missing prepared source curves")?;
        if line.optical_stroke.is_none() {
            unsafe {
                let stroke = self.factory.CreatePathGeometry()?;
                let sink = stroke.Open()?;
                line.curves
                    .Widen(0.12 / 513., &self.rounded_stroke, None, 0.0002, &sink)?;
                sink.Close()?;
                line.optical_stroke = Some(stroke);
            }
        }
        Ok(())
    }
    pub fn prepare_strokes(&mut self, page: &ShapedPage) -> Result<()> {
        for shape in page.lines.values() {
            self.prepare_stroke(shape.geometry_key)?;
        }
        Ok(())
    }
    pub fn page_fit(page: &ShapedPage, width: f32, row: f32) -> f32 {
        let mut fit = 1f32;
        for (distance, overlap) in &page.gaps {
            let available = row * *distance as f32 - row * (3. / 57.4);
            fit = fit.min(available / (overlap * width).max(1.));
        }
        fit.min(1.)
    }
    #[cfg(feature = "qa")]
    pub fn clearance(page: &ShapedPage, width: f32, row: f32) -> f32 {
        let mut minimum = f32::MAX;
        for (distance, overlap) in &page.gaps {
            minimum = minimum.min(row * *distance as f32 - overlap * width);
        }
        minimum
    }
    #[cfg(feature = "qa")]
    pub fn ink_envelope(shape: &Shape, rect: Rect) -> Option<Rect> {
        let first = shape.ink_glyphs.first()?;
        let mut left = first.x;
        let mut top = first.y;
        let mut right = first.x + first.w;
        let mut bottom = first.y + first.h;
        for glyph in &shape.ink_glyphs {
            left = left.min(glyph.x);
            top = top.min(glyph.y);
            right = right.max(glyph.x + glyph.w);
            bottom = bottom.max(glyph.y + glyph.h);
        }
        Some(Rect {
            x: rect.x + left * rect.w,
            y: rect.y + rect.h / 2. + top * rect.w,
            w: (right - left) * rect.w,
            h: (bottom - top) * rect.w,
        })
    }
    fn geometry(&mut self, id: u32) -> Result<&ID2D1PathGeometry> {
        if !self.paths.contains_key(&id) {
            let source = self.source.get(&id).ok_or("Missing source glyph outline")?;
            let geometry = compile(&self.factory, source)?;
            self.paths.insert(id, geometry);
        }
        Ok(&self.paths[&id])
    }
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
    pub fn validate_source_page(page: &ShapedPage, source: &crate::data::Page) -> Result<()> {
        for (number, shape) in &page.lines {
            let line = source
                .lines
                .iter()
                .find(|line| line.line_number == *number && line.kind == "ayah")
                .ok_or("Shaped line is not an authentic ayah line")?;
            let ids: std::collections::BTreeSet<_> =
                line.words.iter().map(|word| word.id).collect();
            let glyph_words: std::collections::BTreeSet<_> =
                shape.glyphs.iter().map(|glyph| glyph[3] as u32).collect();
            if ids.len() != shape.word_bounds.len()
                || shape.word_bounds.keys().any(|id| !ids.contains(id))
                || glyph_words != ids
                || shape.glyphs.iter().any(|glyph| {
                    glyph[3] < 0. || glyph[3].fract() != 0. || !ids.contains(&(glyph[3] as u32))
                })
            {
                return Err("Source outline/word mapping mismatch".into());
            }
            Self::hit_regions(
                shape,
                line,
                Rect {
                    x: 0.,
                    y: 0.,
                    w: 513.,
                    h: 57.4,
                },
            )?;
        }
        Ok(())
    }
    pub fn paint(
        &mut self,
        target: &ID2D1RenderTarget,
        shape: &Shape,
        view: OutlineView,
        ink: &ID2D1SolidColorBrush,
    ) -> Result<()> {
        unsafe {
            let OutlineView { rect, palette } = view;
            let mut parent = Matrix3x2::identity();
            target.GetTransform(&mut parent);
            let mut dpi_x = 0.;
            let mut dpi_y = 0.;
            target.GetDpi(&mut dpi_x, &mut dpi_y);
            // Match the released renderer: low-density dark pages get a small
            // optical weight, scaled with the natural page. At high density the
            // unweighted source curves already resolve the thin marks cleanly.
            let optical_weight = if palette.paper.r < 0.25 && dpi_x < 144. && dpi_y < 144. {
                0.12 / 513.
            } else {
                0.
            };
            if optical_weight > 0. {
                self.prepare_stroke(shape.geometry_key)?;
            }
            let prepared = self
                .lines
                .get(&shape.geometry_key)
                .ok_or("Missing prepared source curves")?;
            target.SetTransform(
                &(Matrix3x2 {
                    M11: rect.w,
                    M22: rect.w,
                    M31: rect.x,
                    M32: rect.y + rect.h / 2.,
                    ..Matrix3x2::identity()
                } * parent),
            );
            target.FillGeometry(&prepared.curves, ink, None);
            if optical_weight > 0. {
                target.FillGeometry(prepared.optical_stroke.as_ref().unwrap(), ink, None);
            }
            target.SetTransform(&parent);
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    pub fn verify(&mut self) -> Result<usize> {
        let ids: Vec<_> = self.source.keys().copied().collect();
        for id in &ids {
            self.geometry(*id)?;
        }
        for n in 1..=604 {
            let shaped = self.page(n)?;
            let page: crate::data::Page = read_json(
                &self
                    .root
                    .join(format!("experimental/digital-khatt/pages/p{n}.json")),
            )?;
            Self::validate_source_page(&shaped, &page)?;
        }
        Ok(ids.len())
    }
}
/// The pinned CFF-to-path generator emits absolute M/L/C/Q/Z commands. Reject
/// unknown syntax rather than paint a different Quran glyph. Coordinates are
/// parsed into native cubic/quadratic geometry without flattening to bitmaps.
fn compile(factory: &ID2D1Factory, source: &str) -> Result<ID2D1PathGeometry> {
    unsafe {
        let geometry = factory.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        let mut parser = Parser {
            bytes: source.as_bytes(),
            at: 0,
        };
        let mut command = b' ';
        let mut open = false;
        while parser.more() {
            if parser.bytes[parser.at].is_ascii_alphabetic() {
                command = parser.bytes[parser.at];
                parser.at += 1;
            }
            match command {
                b'M' => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    let point = parser.point()?;
                    sink.BeginFigure(point, D2D1_FIGURE_BEGIN_FILLED);
                    open = true;
                    command = b'L';
                }
                b'L' => {
                    if !open {
                        return Err("Line before source contour".into());
                    }
                    sink.AddLine(parser.point()?);
                }
                b'C' => {
                    let a = parser.point()?;
                    let b = parser.point()?;
                    let c = parser.point()?;
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: a,
                        point2: b,
                        point3: c,
                    });
                }
                b'Q' => {
                    let a = parser.point()?;
                    let b = parser.point()?;
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
                        point1: a,
                        point2: b,
                    });
                }
                b'Z' => {
                    if !open {
                        return Err("Close before source contour".into());
                    }
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    open = false;
                    command = b' ';
                }
                _ => {
                    return Err(
                        format!("Unsupported source outline command {}", command as char).into(),
                    );
                }
            }
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        sink.Close()?;
        Ok(geometry)
    }
}
struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Parser<'_> {
    fn more(&mut self) -> bool {
        while self.at < self.bytes.len()
            && (self.bytes[self.at].is_ascii_whitespace() || self.bytes[self.at] == b',')
        {
            self.at += 1;
        }
        self.at < self.bytes.len()
    }
    fn number(&mut self) -> Result<f32> {
        if !self.more() {
            return Err("Truncated source outline".into());
        }
        let start = self.at;
        if matches!(self.bytes[self.at], b'-' | b'+') {
            self.at += 1;
        }
        while self.at < self.bytes.len()
            && (self.bytes[self.at].is_ascii_digit() || self.bytes[self.at] == b'.')
        {
            self.at += 1;
        }
        if self.at < self.bytes.len() && matches!(self.bytes[self.at], b'e' | b'E') {
            self.at += 1;
            if self.at < self.bytes.len() && matches!(self.bytes[self.at], b'-' | b'+') {
                self.at += 1;
            }
            while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_digit() {
                self.at += 1;
            }
        }
        let value = std::str::from_utf8(&self.bytes[start..self.at])?.parse::<f32>()?;
        if !value.is_finite() || value.abs() > 1_000_000. {
            return Err("Invalid source outline coordinate".into());
        }
        Ok(value)
    }
    fn point(&mut self) -> Result<Vector2> {
        Ok(Vector2 {
            X: self.number()?,
            Y: self.number()?,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_path_numbers() {
        let mut p = Parser {
            bytes: b"-0.000045, 151.1355 1.2e-3",
            at: 0,
        };
        assert!((p.number().unwrap() + 0.000045).abs() < 0.000001);
        assert!((p.number().unwrap() - 151.1355).abs() < 0.0001);
        assert!((p.number().unwrap() - 0.0012).abs() < 0.000001);
    }
    #[test]
    fn reject_invalid_coordinates() {
        let mut p = Parser {
            bytes: b"1e20",
            at: 0,
        };
        assert!(p.number().is_err());
    }
    fn fixture_shape() -> Shape {
        Shape {
            width: 513.,
            scale: 1.,
            shift: -513.,
            baseline_offset: 0.,
            glyphs: vec![],
            word_bounds: BTreeMap::new(),
            ink_glyphs: vec![],
            geometry_key: 0,
        }
    }
    #[test]
    fn normalized_bounds_keep_original_coordinates() {
        let shape = fixture_shape();
        let bounds = D2D_RECT_F {
            left: 0.,
            top: 0.,
            right: 10.,
            bottom: 20.,
        };
        let rect = normalized_ink_bounds(&shape, &[1., 100., 0., 1.], bounds).unwrap();
        assert!((rect.x - 100. / 513.).abs() < 0.000001);
        assert!((rect.y - 330. / 513.).abs() < 0.000001);
        assert!((rect.w - 10. / 513.).abs() < 0.000001);
        assert!((rect.h - 20. / 513.).abs() < 0.000001);
    }
    #[test]
    fn finite_source_values_cannot_overflow_ink_transforms() {
        let bounds = D2D_RECT_F {
            left: 0.,
            top: 0.,
            right: 10.,
            bottom: 20.,
        };
        let glyph = [1., 100., 0., 1.];
        let mut shape = fixture_shape();
        shape.scale = f32::MAX;
        assert!(normalized_ink_bounds(&shape, &glyph, bounds).is_err());
        shape = fixture_shape();
        shape.width = f32::MIN_POSITIVE;
        assert!(normalized_ink_bounds(&shape, &glyph, bounds).is_err());
        shape = fixture_shape();
        assert!(normalized_ink_bounds(&shape, &[1., f32::MAX, 0., 1.], bounds).is_err());
        assert!(
            normalized_ink_bounds(
                &shape,
                &glyph,
                D2D_RECT_F {
                    right: f32::INFINITY,
                    ..bounds
                }
            )
            .is_err()
        );
    }
}
