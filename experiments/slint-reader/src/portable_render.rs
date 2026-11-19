//! The portable page renderer: draws the open book (paper, frame, surah
//! headers, basmala, Quran lines, highlights and page labels) without any
//! operating-system text or graphics API. It follows the native render.rs
//! layout rules; shaping is HarfRust, the engine Digital Khatt is made for,
//! which also gets the font's hamza-seat and tatweel rules right.
use crate::app_core::{PAGE_HEIGHT, PAGE_WIDTH, Rect, book_geometry, spread};
use crate::data::{Page, Resources, Result};
use crate::portable_outlines::Outlines;
use crate::portable_text::{Font, Placed, append, ink_bounds, place};
use serde::Deserialize;
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    rc::Rc,
};
use tiny_skia::{
    Color, FillRule, Paint, Path, PathBuilder, Pixmap, Rect as SkRect, Stroke, Transform,
};

const CONTENT_WIDTH: f32 = 513.;
const FONT_SIZE: f32 = 30.6;
const PAD_TOP: f32 = 24.;
const PAD_BOTTOM: f32 = 34.;
const ROW: f32 = (PAGE_HEIGHT - PAD_TOP - PAD_BOTTOM) / 15.;
const PAD_X: f32 = 52.;
// The QUL v4 decorative basmala ligatures, never used for Quran source lines.
const V4_BASMALA: &str = "\u{FCAA} \u{FCAB} \u{FCAE} \u{FCB4}";

/// What the app draws and how to colour it, as in render.rs's BookView.
pub struct View {
    pub page: u32,
    pub area: Rect,
    pub palette: crate::render::Palette,
    pub selection: Option<(u32, u32)>,
    pub recitation: Option<(u32, u32)>,
    pub opening: Option<u32>,
    pub arabic: bool,
}

/// A drawn line's box and its words, in the book's DIP coordinates.
#[derive(Clone)]
pub struct VisibleLine {
    pub rect: Rect,
    pub hits: Vec<(Rect, (u32, u32))>,
}

pub struct Raster {
    /// Premultiplied BGRA, transparent outside the book, as the native WIC raster.
    pub bgra: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub source: Rect,
    pub bounds: Rect,
    pub visible: Vec<VisibleLine>,
}

#[derive(Clone, Copy, PartialEq)]
enum FontId {
    Quran,
    Basmala,
    Ui,
}

#[derive(Clone)]
enum Draw {
    Glyphs {
        font: FontId,
        size: f32,
        placed: Vec<Placed>,
    },
    /// Colour header layers: path in page units, lightness toward the paper.
    Art(Vec<(Rc<Path>, f32)>),
    None,
}

/// One line laid out in page units (a 617 x 919 page).
#[derive(Clone)]
struct LineInk {
    line_number: u32,
    kind: String,
    rect: Rect,
    draw: Draw,
    /// Words (left, right) in page units, with their verse.
    words: Vec<(f32, f32, (u32, u32))>,
    /// Natural opening lines highlight exactly; justified lines pad like DirectWrite ranges.
    natural: bool,
    ink: Option<SkRect>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChromePage {
    first_surah: u32,
    juz: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SectionStart {
    page_number: u32,
    #[serde(default)]
    hizb: u32,
    #[serde(default)]
    rub: u32,
}
struct Chrome {
    pages: Vec<ChromePage>,
    hizb: BTreeMap<u32, u32>,
    rub: BTreeMap<u32, u32>,
}

pub struct Renderer {
    quran: Font,
    basmala_font: Option<Font>,
    header_font: Option<Font>,
    ui: Option<Font>,
    header_glyphs: BTreeMap<u32, u16>,
    basmala: String,
    chrome: Option<Chrome>,
    outlines: Option<Outlines>,
    outlines_failed: bool,
    pages: RefCell<VecDeque<(u32, Rc<Vec<LineInk>>)>>,
    pub contextual: bool,
    pub show_clock: bool,
    pub clock_24: bool,
}

fn metadata_number(number: u32, arabic: bool) -> String {
    number
        .to_string()
        .chars()
        .map(|c| {
            if arabic {
                char::from_u32('٠' as u32 + c.to_digit(10).unwrap_or(0)).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

fn section_mark(hizb: Option<u32>, rub: Option<u32>, arabic: bool) -> Option<String> {
    let (label, number) = if let Some(hizb) = hizb {
        (if arabic { "الحزب" } else { "Hizb" }, hizb)
    } else {
        let rub = rub?;
        let labels = if arabic {
            ["الحزب", "ربع الحزب", "نصف الحزب", "ثلاثة أرباع الحزب"]
        } else {
            [
                "Hizb",
                "Quarter of Hizb",
                "Half of Hizb",
                "Three quarters of Hizb",
            ]
        };
        (labels[((rub - 1) % 4) as usize], rub.div_ceil(4))
    };
    Some(format!("{label} {}", metadata_number(number, arabic)))
}

fn color(c: windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F) -> Color {
    Color::from_rgba(c.r.clamp(0., 1.), c.g.clamp(0., 1.), c.b.clamp(0., 1.), 1.)
        .unwrap_or(Color::BLACK)
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

fn rounded(r: Rect, radius: f32) -> Option<Path> {
    let radius = radius.min(r.w / 2.).min(r.h / 2.).max(0.);
    let k = radius * 0.552_284_8;
    let (l, t, rt, b) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let mut p = PathBuilder::new();
    p.move_to(l + radius, t);
    p.line_to(rt - radius, t);
    p.cubic_to(rt - radius + k, t, rt, t + radius - k, rt, t + radius);
    p.line_to(rt, b - radius);
    p.cubic_to(rt, b - radius + k, rt - radius + k, b, rt - radius, b);
    p.line_to(l + radius, b);
    p.cubic_to(l + radius - k, b, l, b - radius + k, l, b - radius);
    p.line_to(l, t + radius);
    p.cubic_to(l, t + radius - k, l + radius - k, t, l + radius, t);
    p.close();
    p.finish()
}

impl Renderer {
    pub fn new(resources: &mut Resources) -> Result<Self> {
        let root = resources.root.clone();
        let quran = Font::load(
            &root.join("fonts/digital-khatt/DigitalKhattV2.otf"),
            2 * 1024 * 1024,
        )?;
        let basmala_font = Font::load(
            &root.join("fonts/surah-names/v4/surah_names.ttf"),
            4 * 1024 * 1024,
        )
        .ok();
        let header_font = Font::load(
            &root.join("fonts/surah-header/QCF_SurahHeader_COLOR-Regular.ttf"),
            4 * 1024 * 1024,
        )
        .ok();
        // Page labels use Segoe UI where Windows provides it, as today.
        let ui = std::env::var_os("WINDIR")
            .map(|dir| {
                std::path::PathBuf::from(dir)
                    .join("Fonts")
                    .join("segoeui.ttf")
            })
            .and_then(|path| Font::load(&path, 8 * 1024 * 1024).ok());
        let mut header_glyphs = BTreeMap::new();
        if let Some(font) = &header_font {
            let map: BTreeMap<String, String> =
                crate::data::read_json(&root.join("fonts/surah-header/ligatures.json"))?;
            for number in 1..=114u32 {
                if let Some(glyph) = map
                    .get(&format!("surah-{number}"))
                    .and_then(|text| text.chars().next())
                    .and_then(|c| font.glyph_for(c))
                {
                    header_glyphs.insert(number, glyph);
                }
            }
        }
        let first = resources.page(1)?;
        let basmala = first
            .lines
            .iter()
            .find(|line| line.words.iter().any(|w| w.surah == 1 && w.ayah == 1))
            .ok_or("Missing authentic basmala")?
            .words
            .iter()
            .filter(|w| w.surah == 1 && w.ayah == 1 && !w.text.starts_with('۝'))
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let chrome = (|| -> Result<Chrome> {
            let pages: Vec<ChromePage> =
                crate::data::read_json(&root.join("data/qul/processed/page-index.json"))?;
            if pages.len() != 604 {
                return Err("Invalid page metadata".into());
            }
            let sections = |path: &str, rub: bool| -> Result<BTreeMap<u32, u32>> {
                let starts: Vec<SectionStart> = crate::data::read_json(&root.join(path))?;
                Ok(starts
                    .into_iter()
                    .map(|s| (s.page_number, if rub { s.rub } else { s.hizb }))
                    .collect())
            };
            Ok(Chrome {
                pages,
                hizb: sections("data/qul/processed/hizb.json", false)?,
                rub: sections("data/qul/processed/rub.json", true)?,
            })
        })()
        .ok();
        Ok(Self {
            quran,
            basmala_font,
            header_font,
            ui,
            header_glyphs,
            basmala,
            chrome,
            outlines: None,
            outlines_failed: false,
            pages: RefCell::new(VecDeque::new()),
            contextual: false,
            show_clock: false,
            clock_24: false,
        })
    }

    fn font(&self, id: FontId) -> &Font {
        match id {
            FontId::Quran => &self.quran,
            FontId::Basmala => self.basmala_font.as_ref().unwrap_or(&self.quran),
            FontId::Ui => self.ui.as_ref().unwrap_or(&self.quran),
        }
    }

    /// Glyphs centred in a `width` x `height` box as DirectWrite centres a
    /// single no-wrap line: by advance width, and by the font's line box.
    #[allow(clippy::too_many_arguments)]
    fn centred(
        &self,
        id: FontId,
        text: &str,
        rtl: bool,
        size: f32,
        width: f32,
        height: f32,
        extra: f32,
    ) -> Result<(Vec<Placed>, Vec<bool>)> {
        let font = self.font(id);
        let glyphs = font.shape(text, rtl)?;
        let is_space: Vec<bool> = glyphs
            .iter()
            .map(|g| text[g.cluster..].starts_with(' '))
            .collect();
        let scale = size / font.units_per_em;
        let total: f32 = glyphs
            .iter()
            .zip(&is_space)
            .map(|(g, s)| g.advance * scale + if *s { extra } else { 0. })
            .sum();
        let line = (font.ascent + font.descent + font.line_gap) * size;
        let baseline = (height - line) / 2. + (font.ascent + font.line_gap) * size;
        let (placed, _) = place(
            &glyphs,
            &is_space,
            font,
            size,
            extra,
            (width - total) / 2.,
            baseline,
        );
        Ok((placed, is_space))
    }

    fn ink(&self, id: FontId, placed: &[Placed], size: f32) -> Result<SkRect> {
        ink_bounds(self.font(id), placed, size)?.ok_or_else(|| "Line without ink".into())
    }

    /// Words of a Quran line as pen spans: each word's glyphs, in page units.
    fn word_spans(
        &self,
        line: &crate::data::Line,
        text_glyphs: &[crate::portable_text::Glyph],
        placed: &[Placed],
        size: f32,
    ) -> Vec<(f32, f32, (u32, u32))> {
        let scale = size / self.quran.units_per_em;
        let mut ranges = Vec::new();
        let mut byte = 0;
        for word in &line.words {
            ranges.push((byte, byte + word.text.len(), (word.surah, word.ayah)));
            byte += word.text.len() + 1;
        }
        let mut spans = vec![(f32::INFINITY, f32::NEG_INFINITY); ranges.len()];
        for (glyph, at) in text_glyphs.iter().zip(placed) {
            if let Some(index) = ranges
                .iter()
                .position(|(a, b, _)| (*a..*b).contains(&glyph.cluster))
            {
                let pen = at.x - glyph.x * scale;
                spans[index].0 = spans[index].0.min(pen);
                spans[index].1 = spans[index].1.max(pen + (glyph.advance * scale).max(0.));
            }
        }
        spans
            .into_iter()
            .zip(ranges)
            .filter(|((a, b), _)| a.is_finite() && b.is_finite() && b >= a)
            .map(|((a, b), (_, _, key))| (a, b, key))
            .collect()
    }

    /// The page laid out in page units, as render.rs's page_ink does.
    fn page_lines(&self, page: &Page, resources: &Resources) -> Result<Rc<Vec<LineInk>>> {
        if let Some((_, lines)) = self
            .pages
            .borrow()
            .iter()
            .find(|(n, _)| *n == page.page_number)
        {
            return Ok(lines.clone());
        }
        let opening = page.page_number <= 2;
        let ayahs: Vec<_> = page.lines.iter().filter(|l| l.kind == "ayah").collect();
        // One font size per page: fit the widest line, then the row gaps.
        let mut font = FONT_SIZE;
        for line in &ayahs {
            let (placed, _) =
                self.centred(FontId::Quran, &line.text, true, font, 2000., ROW, 0.)?;
            let ink = self.ink(FontId::Quran, &placed, font)?;
            if ink.width() > CONTENT_WIDTH {
                font *= CONTENT_WIDTH / ink.width();
            }
        }
        let mut fit = 1f32;
        let mut previous: Option<(u32, f32)> = None;
        for line in &ayahs {
            let (placed, _) = self.centred(
                FontId::Quran,
                &line.text,
                true,
                font,
                CONTENT_WIDTH,
                ROW,
                0.,
            )?;
            let ink = self.ink(FontId::Quran, &placed, font)?;
            let top = ink.top() - ROW / 2.;
            if let Some((last, lower)) = previous {
                let available = ROW * (line.line_number - last) as f32 - ROW * 0.06;
                fit = fit.min(available / (lower - top).max(1.));
            }
            previous = Some((line.line_number, ink.bottom() - ROW / 2.));
        }
        font *= fit.min(1.);
        if opening {
            let mut natural_fit = 1f32;
            for line in &ayahs {
                let glyphs = self.quran.shape(&line.text, true)?;
                let (placed, _) = place(&glyphs, &[], &self.quran, font, 0., 0., 0.);
                let ink = self.ink(FontId::Quran, &placed, font)?;
                natural_fit = natural_fit
                    .min(CONTENT_WIDTH / ink.width())
                    .min(ROW * 0.86 / ink.height());
            }
            font *= natural_fit.min(1.);
        }
        let mut lines = Vec::new();
        for line in &page.lines {
            let rect = Rect {
                x: PAD_X,
                y: PAD_TOP + (line.line_number - 1) as f32 * ROW + if opening { 200. } else { 0. },
                w: CONTENT_WIDTH,
                h: ROW,
            };
            let mut ink_line = LineInk {
                line_number: line.line_number,
                kind: line.kind.clone(),
                rect,
                draw: Draw::None,
                words: Vec::new(),
                natural: opening,
                ink: None,
            };
            match line.kind.as_str() {
                "surah_name" => {
                    let number = line.surah_number.ok_or("Missing surah header")?;
                    if let (Some(header), Some(glyph)) =
                        (&self.header_font, self.header_glyphs.get(&number))
                    {
                        let layers = header.color_layers(*glyph)?;
                        let mut paths = Vec::new();
                        let mut bounds: Option<SkRect> = None;
                        for (id, lightness) in layers {
                            if let Some(outline) = header.outline(id)? {
                                bounds = Some(match bounds {
                                    None => outline.bounds,
                                    Some(b) => SkRect::from_ltrb(
                                        b.left().min(outline.bounds.left()),
                                        b.top().min(outline.bounds.top()),
                                        b.right().max(outline.bounds.right()),
                                        b.bottom().max(outline.bounds.bottom()),
                                    )
                                    .unwrap_or(b),
                                });
                                paths.push((outline.path.clone(), lightness));
                            }
                        }
                        let b = bounds.ok_or("Surah header without ink")?;
                        let factor = (rect.w * 0.94 / b.width()).min(rect.h * 0.78 / b.height());
                        let transform = Transform::from_row(
                            factor,
                            0.,
                            0.,
                            factor,
                            rect.x + (rect.w - b.width() * factor) / 2. - b.left() * factor,
                            rect.y + (rect.h - b.height() * factor) / 2. - b.top() * factor,
                        );
                        let art = paths
                            .into_iter()
                            .filter_map(|(p, l)| {
                                p.as_ref()
                                    .clone()
                                    .transform(transform)
                                    .map(|p| (Rc::new(p), l))
                            })
                            .collect();
                        ink_line.draw = Draw::Art(art);
                    } else if self.ui.is_some() {
                        let text =
                            format!("سورة {}", resources.surahs[number as usize - 1].name_arabic);
                        ink_line.draw =
                            self.fitted(FontId::Ui, &text, ROW * 0.42, rect, CONTENT_WIDTH * 0.94)?;
                    }
                }
                "basmallah" => {
                    ink_line.draw = if self.basmala_font.is_some() {
                        self.fitted(FontId::Basmala, V4_BASMALA, 26.88, rect, CONTENT_WIDTH)?
                    } else {
                        self.fitted(
                            FontId::Quran,
                            &self.basmala.clone(),
                            font * 0.86,
                            rect,
                            CONTENT_WIDTH,
                        )?
                    };
                    if let Draw::Glyphs {
                        font: id,
                        size,
                        placed,
                    } = &ink_line.draw
                    {
                        ink_line.ink = ink_bounds(self.font(*id), placed, *size)?;
                    }
                }
                _ if opening => {
                    // HarfRust's natural joins, centred by their ink (render.rs natural_ink).
                    let glyphs = self.quran.shape(&line.text, true)?;
                    let (placed, _) = place(&glyphs, &[], &self.quran, font, 0., 0., 0.);
                    let ink = self.ink(FontId::Quran, &placed, font)?;
                    let dx = rect.x + (rect.w - ink.width()) / 2. - ink.left();
                    let dy = rect.y + (rect.h - ink.height()) / 2. - ink.top();
                    let placed: Vec<Placed> = placed
                        .into_iter()
                        .map(|p| Placed {
                            id: p.id,
                            x: p.x + dx,
                            y: p.y + dy,
                        })
                        .collect();
                    ink_line.words = self.word_spans(line, &glyphs, &placed, font);
                    ink_line.draw = Draw::Glyphs {
                        font: FontId::Quran,
                        size: font,
                        placed,
                    };
                }
                _ => {
                    let mut extra = 0.;
                    let (mut placed, _) = self.centred(
                        FontId::Quran,
                        &line.text,
                        true,
                        font,
                        CONTENT_WIDTH,
                        ROW,
                        0.,
                    )?;
                    // Full-width rows stretch their spaces until the ink spans the row.
                    let spaces = line.text.encode_utf16().filter(|c| *c == 32).count();
                    if !line.is_centered && spaces > 0 {
                        for _ in 0..3 {
                            let ink = self.ink(FontId::Quran, &placed, font)?;
                            let remaining = CONTENT_WIDTH - ink.width();
                            if remaining.abs() <= 0.02 {
                                break;
                            }
                            extra = (extra + remaining / spaces as f32).max(0.);
                            placed = self
                                .centred(
                                    FontId::Quran,
                                    &line.text,
                                    true,
                                    font,
                                    CONTENT_WIDTH,
                                    ROW,
                                    extra,
                                )?
                                .0;
                        }
                    }
                    let ink = self.ink(FontId::Quran, &placed, font)?;
                    let dx = rect.x + (CONTENT_WIDTH - ink.width()) / 2. - ink.left();
                    let placed: Vec<Placed> = placed
                        .into_iter()
                        .map(|p| Placed {
                            id: p.id,
                            x: p.x + dx,
                            y: p.y + rect.y,
                        })
                        .collect();
                    let glyphs = self.quran.shape(&line.text, true)?;
                    ink_line.words = self.word_spans(line, &glyphs, &placed, font);
                    ink_line.draw = Draw::Glyphs {
                        font: FontId::Quran,
                        size: font,
                        placed,
                    };
                }
            }
            lines.push(ink_line);
        }
        let lines = Rc::new(lines);
        let mut cache = self.pages.borrow_mut();
        cache.push_back((page.page_number, lines.clone()));
        while cache.len() > 12 {
            cache.pop_front();
        }
        Ok(lines)
    }

    /// A decorative line fitted to its ink and centred in the row, as
    /// render.rs fits headers and the basmala.
    fn fitted(&self, id: FontId, text: &str, size: f32, rect: Rect, allowed: f32) -> Result<Draw> {
        let (placed, _) = self.centred(id, text, true, size, rect.w, rect.h, 0.)?;
        let ink = self.ink(id, &placed, size)?;
        let size = size
            * (allowed / ink.width().max(1.))
                .min(rect.h * 0.78 / ink.height().max(1.))
                .min(1.);
        let (placed, _) = self.centred(id, text, true, size, rect.w, rect.h, 0.)?;
        let ink = self.ink(id, &placed, size)?;
        let dx = rect.x + (rect.w - ink.width()) / 2. - ink.left();
        let dy = rect.y + (rect.h - ink.height()) / 2. - ink.top();
        Ok(Draw::Glyphs {
            font: id,
            size,
            placed: placed
                .into_iter()
                .map(|p| Placed {
                    id: p.id,
                    x: p.x + dx,
                    y: p.y + dy,
                })
                .collect(),
        })
    }

    /// A page label, as render.rs's chrome_label and label draw them.
    #[allow(clippy::too_many_arguments)]
    fn label(
        &self,
        pixmap: &mut Pixmap,
        to_px: Transform,
        id: FontId,
        text: &str,
        rect: Rect,
        size: f32,
        ink: Color,
        rtl: bool,
        align: i8,
    ) -> Result<()> {
        if (id == FontId::Ui && self.ui.is_none()) || text.is_empty() {
            return Ok(());
        }
        let font = self.font(id);
        let glyphs = shape_label(font, text, rtl)?;
        let scale = size / font.units_per_em;
        let width: f32 = glyphs.iter().map(|g| g.advance * scale).sum();
        let x = match align {
            -1 => rect.x,
            1 => rect.x + rect.w - width,
            _ => rect.x + (rect.w - width) / 2.,
        };
        let line = (font.ascent + font.descent + font.line_gap) * size;
        let baseline = rect.y + (rect.h - line) / 2. + (font.ascent + font.line_gap) * size;
        let (placed, _) = place(&glyphs, &[], font, size, 0., x, baseline);
        let placed: Vec<Placed> = placed.into_iter().filter(|p| p.id != 0).collect();
        let mut builder = PathBuilder::new();
        append(&mut builder, font, &placed, size, to_px)?;
        if let Some(path) = builder.finish() {
            pixmap.fill_path(
                &path,
                &paint(ink),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn page_chrome(
        &self,
        pixmap: &mut Pixmap,
        to_px: Transform,
        resources: &Resources,
        page: &Page,
        rect: Rect,
        muted: Color,
        arabic: bool,
    ) -> Result<()> {
        let scale = rect.h / PAGE_HEIGHT;
        let inset = 30.4 * scale;
        let meta = self
            .chrome
            .as_ref()
            .and_then(|c| c.pages.get(page.page_number as usize - 1));
        let surah = meta.map(|m| m.first_surah).or_else(|| {
            page.lines
                .iter()
                .flat_map(|l| &l.words)
                .next()
                .map(|w| w.surah)
        });
        if let Some(surah) = surah.and_then(|n| resources.surahs.get(n as usize - 1)) {
            self.label(
                pixmap,
                to_px,
                if arabic { FontId::Quran } else { FontId::Ui },
                if arabic {
                    &surah.name_arabic
                } else {
                    &surah.name_simple
                },
                Rect {
                    x: rect.x + inset,
                    y: rect.y + 4. * scale,
                    w: rect.w * 0.34,
                    h: 22. * scale,
                },
                if arabic { 16.32 * scale } else { 11.52 * scale },
                muted,
                arabic,
                -1,
            )?;
        }
        if self.show_clock
            && let Some((hour, minute)) = local_time()
        {
            let hour = if self.clock_24 {
                hour
            } else if hour % 12 == 0 {
                12
            } else {
                hour % 12
            };
            self.label(
                pixmap,
                to_px,
                FontId::Ui,
                &format!("{hour:02}:{minute:02}"),
                Rect {
                    x: rect.x + rect.w * 0.4,
                    y: rect.y + 4. * scale,
                    w: rect.w * 0.2,
                    h: 22. * scale,
                },
                10.24 * scale,
                muted,
                false,
                0,
            )?;
        }
        if let Some(meta) = meta {
            self.label(
                pixmap,
                to_px,
                FontId::Ui,
                &format!(
                    "{} {}",
                    if arabic { "الجزء" } else { "Juz" },
                    metadata_number(meta.juz, arabic)
                ),
                Rect {
                    x: rect.x + rect.w * 0.65 - inset,
                    y: rect.y + 4. * scale,
                    w: rect.w * 0.35,
                    h: 22. * scale,
                },
                11.2 * scale,
                muted,
                arabic,
                1,
            )?;
        }
        self.label(
            pixmap,
            to_px,
            FontId::Ui,
            &metadata_number(page.page_number, arabic),
            Rect {
                x: rect.x,
                y: rect.y + rect.h - 29. * scale,
                w: rect.w,
                h: 22. * scale,
            },
            if arabic { 15.04 * scale } else { 11.84 * scale },
            muted,
            arabic,
            0,
        )?;
        if let Some(chrome) = &self.chrome
            && let Some(mark) = section_mark(
                chrome.hizb.get(&page.page_number).copied(),
                chrome.rub.get(&page.page_number).copied(),
                arabic,
            )
        {
            let right_page = page.page_number % 2 == 1;
            self.label(
                pixmap,
                to_px,
                FontId::Ui,
                &mark,
                Rect {
                    x: if right_page {
                        rect.x + inset
                    } else {
                        rect.x + rect.w * 0.52 - inset
                    },
                    y: rect.y + rect.h - 29. * scale,
                    w: rect.w * 0.48,
                    h: 22. * scale,
                },
                if arabic { 14.4 * scale } else { 11.04 * scale },
                muted,
                arabic,
                if right_page { -1 } else { 1 },
            )?;
        }
        Ok(())
    }

    /// Draws the open book at `dpi`, as render.rs's raster_book_with_aa.
    /// Digital Khatt's pinned outlines for a page, if they load. A failure
    /// turns elongation off for the session rather than failing pages.
    fn shaped_page(
        &mut self,
        resources: &Resources,
        number: u32,
        page: &Page,
    ) -> Option<Rc<crate::portable_outlines::ShapedPage>> {
        if self.outlines_failed {
            return None;
        }
        let attempt = (|| -> Result<_> {
            if self.outlines.is_none() {
                self.outlines = Some(Outlines::new(resources.root.clone())?);
            }
            self.outlines
                .as_mut()
                .ok_or("Missing outlines")?
                .page(number, page)
        })();
        match attempt {
            Ok(page) => Some(page),
            Err(_) => {
                self.outlines_failed = true;
                self.outlines = None;
                None
            }
        }
    }

    /// Loads, shapes and lays out a spread's pages (and their elongation
    /// outlines) ahead of drawing. None of it depends on the window, so it can
    /// run while the window and its GPU are still starting.
    pub fn warm(&mut self, resources: &mut Resources, number: u32, contextual: bool) -> Result<()> {
        let (right, left) = spread(number);
        for n in std::iter::once(right).chain(left) {
            let page = resources.page(n)?;
            self.page_lines(&page, resources)?;
            if contextual {
                self.shaped_page(resources, n, &page);
            }
        }
        Ok(())
    }

    pub fn raster_book(
        &mut self,
        resources: &mut Resources,
        view: View,
        dpi: f32,
    ) -> Result<Raster> {
        if !dpi.is_finite() || !(48. ..=768.).contains(&dpi) {
            return Err("Invalid book raster density".into());
        }
        let geometry = book_geometry(view.area);
        let bounds = geometry.bounds;
        if ![bounds.x, bounds.y, bounds.w, bounds.h]
            .iter()
            .all(|n| n.is_finite())
            || bounds.w <= 0.
            || bounds.h <= 0.
        {
            return Err("Invalid book raster bounds".into());
        }
        let began = std::time::Instant::now();
        let mut timing = String::new();
        let density = dpi / 96.;
        let left = ((bounds.x - 2.) * density).floor();
        let top = ((bounds.y - 2.) * density).floor();
        let width = ((bounds.x + bounds.w + 2.) * density).ceil() - left;
        let height = ((bounds.y + bounds.h + 2.) * density).ceil() - top;
        if !(1. ..=16384.).contains(&width) || !(1. ..=16384.).contains(&height) {
            return Err("Book raster dimensions exceed their limits".into());
        }
        let source = Rect {
            x: left / density,
            y: top / density,
            w: width / density,
            h: height / density,
        };
        let mut pixmap =
            Pixmap::new(width as u32, height as u32).ok_or("Invalid book raster size")?;
        // DIP to pixels.
        let to_px = Transform::from_row(density, 0., 0., density, -left, -top);
        let palette = view.palette;
        let (paper, ink, border, muted, highlight) = (
            color(palette.paper),
            color(palette.ink),
            color(palette.border),
            color(palette.muted),
            color(palette.highlight),
        );
        let single = geometry.single;
        let w = geometry.page_width;
        let (h, x0, y0) = (bounds.h, bounds.x, bounds.y);
        if let Some(frame) = rounded(bounds, 12.) {
            pixmap.fill_path(&frame, &paint(paper), FillRule::Winding, to_px, None);
            pixmap.stroke_path(
                &frame,
                &paint(border),
                &Stroke {
                    width: 1.,
                    ..Stroke::default()
                },
                to_px,
                None,
            );
        }
        if !single {
            let mut spine = PathBuilder::new();
            spine.move_to(x0 + w, y0 + 6.);
            spine.line_to(x0 + w, y0 + h - 6.);
            if let Some(spine) = spine.finish() {
                pixmap.stroke_path(
                    &spine,
                    &paint(border),
                    &Stroke {
                        width: 1.,
                        ..Stroke::default()
                    },
                    to_px,
                    None,
                );
            }
        }
        let (right_page, left_page) = spread(view.page);
        let pages = if single {
            [(Some(view.page), x0), (None, x0)]
        } else {
            [(Some(right_page), x0 + w), (left_page, x0)]
        };
        let mut visible = Vec::new();
        for (number, x) in pages {
            let Some(number) = number else { continue };
            let step = std::time::Instant::now();
            let page = resources.page(number)?;
            let lines = self.page_lines(&page, resources)?;
            let layout_ms = step.elapsed().as_secs_f64() * 1000.;
            let scale = w / PAGE_WIDTH;
            // Page units to DIP for this page.
            let to_dip = Transform::from_row(scale, 0., 0., scale, x, y0);
            let page_to_px = to_dip.post_concat(to_px);
            let dip = |r: Rect| Rect {
                x: x + r.x * scale,
                y: y0 + r.y * scale,
                w: r.w * scale,
                h: r.h * scale,
            };
            // Contextual elongation: Digital Khatt's pinned outlines, if they load.
            let shaped = if self.contextual {
                self.shaped_page(resources, number, &page)
            } else {
                None
            };
            let outlines_ms = step.elapsed().as_secs_f64() * 1000. - layout_ms;
            let step = std::time::Instant::now();
            let outline_fit = shaped
                .as_ref()
                .map(|p| p.fit(CONTENT_WIDTH * scale, ROW * scale))
                .unwrap_or(1.);
            for line in lines.iter() {
                let rect = dip(line.rect);
                let source_line = page
                    .lines
                    .iter()
                    .find(|l| l.line_number == line.line_number);
                // The basmala of a surah being opened is marked, as in render.rs.
                if line.kind == "basmallah"
                    && let Some(b) = line.ink
                    && view.opening.is_some_and(|surah| {
                        page.lines
                            .iter()
                            .take_while(|s| s.line_number < line.line_number)
                            .filter_map(|s| s.surah_number)
                            .last()
                            == Some(surah)
                    })
                {
                    let r = Rect {
                        x: x + b.left() * scale - 5.,
                        y: y0 + b.top() * scale - 3.,
                        w: b.width() * scale + 10.,
                        h: b.height() * scale + 6.,
                    };
                    if let Some(path) = rounded(r, 4.) {
                        pixmap.fill_path(&path, &paint(highlight), FillRule::Winding, to_px, None);
                    }
                }
                let contextual = shaped
                    .as_ref()
                    .zip(source_line)
                    .and_then(|(p, s)| p.lines.get(&line.line_number).map(|l| (l, s)));
                if line.kind == "ayah"
                    && let Some(((shape, path), source)) = contextual
                {
                    let mut outline_rect = rect;
                    outline_rect.x += outline_rect.w * (1. - outline_fit) / 2.;
                    outline_rect.w *= outline_fit;
                    let hits = Outlines::hit_regions(shape, source, outline_rect)?;
                    self.highlights(
                        &mut pixmap,
                        to_px,
                        &hits,
                        view.selection,
                        view.recitation,
                        highlight,
                        muted,
                        false,
                    );
                    let to = Transform::from_row(
                        outline_rect.w,
                        0.,
                        0.,
                        outline_rect.w,
                        outline_rect.x,
                        outline_rect.y + outline_rect.h / 2.,
                    )
                    .post_concat(to_px);
                    pixmap.fill_path(path, &paint(ink), FillRule::Winding, to, None);
                    // A small optical weight on dark paper at low density, as
                    // outlines.rs widens its ink by 0.12/513 of the line. A
                    // second fill at 30% darkens only the anti-aliased edges by
                    // about the ink that outline adds (measured against a
                    // tiny-skia hairline of that width: under one grey level of
                    // 255 apart on average), at a fraction of the hairline's cost.
                    if palette.paper.r < 0.25 && dpi < 144. {
                        let mut faint = ink;
                        faint.apply_opacity(0.3);
                        pixmap.fill_path(path, &paint(faint), FillRule::Winding, to, None);
                    }
                    visible.push(VisibleLine { rect, hits });
                    continue;
                }
                let hits: Vec<(Rect, (u32, u32))> = line
                    .words
                    .iter()
                    .map(|(a, b, key)| {
                        (
                            Rect {
                                x: x + a * scale,
                                y: rect.y,
                                w: (b - a) * scale,
                                h: rect.h,
                            },
                            *key,
                        )
                    })
                    .collect();
                if line.kind == "ayah" {
                    self.highlights(
                        &mut pixmap,
                        to_px,
                        &hits,
                        view.selection,
                        view.recitation,
                        highlight,
                        muted,
                        !line.natural,
                    );
                }
                match &line.draw {
                    Draw::Glyphs { font, size, placed } => {
                        let mut builder = PathBuilder::new();
                        append(&mut builder, self.font(*font), placed, *size, page_to_px)?;
                        if let Some(path) = builder.finish() {
                            pixmap.fill_path(
                                &path,
                                &paint(ink),
                                FillRule::Winding,
                                Transform::identity(),
                                None,
                            );
                        }
                    }
                    Draw::Art(layers) => {
                        for (path, lightness) in layers {
                            let t = *lightness;
                            let c = Color::from_rgba(
                                ink.red() * (1. - t) + paper.red() * t,
                                ink.green() * (1. - t) + paper.green() * t,
                                ink.blue() * (1. - t) + paper.blue() * t,
                                1.,
                            )
                            .unwrap_or(ink);
                            pixmap.fill_path(path, &paint(c), FillRule::Winding, page_to_px, None);
                        }
                    }
                    Draw::None => {}
                }
                visible.push(VisibleLine { rect, hits });
            }
            let draw_ms = step.elapsed().as_secs_f64() * 1000.;
            let step = std::time::Instant::now();
            self.page_chrome(
                &mut pixmap,
                to_px,
                resources,
                &page,
                Rect { x, y: y0, w, h },
                muted,
                view.arabic,
            )?;
            if timing_enabled() {
                timing += &format!(
                    " p{number}: layout {layout_ms:.1} outlines {outlines_ms:.1} draw {draw_ms:.1} labels {:.1};",
                    step.elapsed().as_secs_f64() * 1000.
                );
            }
        }
        if timing_enabled() {
            eprintln!(
                "[timing] spread {} {}x{} total {:.1} ms;{timing}",
                view.page,
                width,
                height,
                began.elapsed().as_secs_f64() * 1000.
            );
        }
        // tiny-skia is premultiplied RGBA; the app expects premultiplied BGRA.
        let mut bgra = pixmap.take();
        for p in bgra.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
        }
        Ok(Raster {
            bgra,
            width: width as u32,
            height: height as u32,
            source,
            bounds,
            visible,
        })
    }

    /// Selection fills and recitation outlines, as render.rs's highlights.
    #[allow(clippy::too_many_arguments)]
    fn highlights(
        &self,
        pixmap: &mut Pixmap,
        to_px: Transform,
        hits: &[(Rect, (u32, u32))],
        selection: Option<(u32, u32)>,
        recitation: Option<(u32, u32)>,
        fill: Color,
        outline: Color,
        pad: bool,
    ) {
        let targets = [
            selection.map(|key| (key, true)),
            recitation
                .filter(|key| Some(*key) != selection)
                .map(|key| (key, selection.is_none())),
        ];
        for (key, filled) in targets.into_iter().flatten() {
            let mut chosen = hits.iter().filter(|(_, k)| *k == key);
            let Some((first, _)) = chosen.next() else {
                continue;
            };
            let (l, r) = chosen.fold((first.x, first.x + first.w), |(l, r), (v, _)| {
                (l.min(v.x), r.max(v.x + v.w))
            });
            let (l, r) = if pad { (l - 2., r + 2.) } else { (l, r) };
            let rect = Rect {
                x: l,
                y: first.y + first.h * 0.14,
                w: r - l,
                h: first.h * 0.72,
            };
            if let Some(path) = rounded(rect, 4.) {
                if filled {
                    pixmap.fill_path(&path, &paint(fill), FillRule::Winding, to_px, None);
                } else {
                    pixmap.stroke_path(
                        &path,
                        &paint(outline),
                        &Stroke {
                            width: 1.25,
                            ..Stroke::default()
                        },
                        to_px,
                        None,
                    );
                }
            }
        }
    }
}

/// A label's glyphs in visual order. In a right-to-left label, numbers keep
/// their left-to-right digit order (the Unicode bidirectional rule for
/// numbers): runs of digits are shaped left to right, and runs are placed
/// right to left.
fn shape_label(font: &Font, text: &str, rtl: bool) -> Result<Vec<crate::portable_text::Glyph>> {
    if !rtl {
        return font.shape(text, false);
    }
    let digit = |c: char| {
        c.is_ascii_digit()
            || ('\u{0660}'..='\u{0669}').contains(&c)
            || ('\u{06F0}'..='\u{06F9}').contains(&c)
    };
    let mut runs: Vec<(bool, String)> = Vec::new();
    for c in text.chars() {
        match runs.last_mut() {
            Some((is_digit, run)) if *is_digit == digit(c) => run.push(c),
            _ => runs.push((digit(c), c.to_string())),
        }
    }
    let mut glyphs = Vec::new();
    for (is_digit, run) in runs.iter().rev() {
        glyphs.extend(font.shape(run, !is_digit)?);
    }
    Ok(glyphs)
}

/// Hour and minute of the local clock.
#[cfg(windows)]
fn local_time() -> Option<(u16, u16)> {
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    Some((now.wHour, now.wMinute))
}
#[cfg(not(windows))]
fn local_time() -> Option<(u16, u16)> {
    None
}

/// The word under a point: the word box containing it, or within a space's
/// width of it, as DirectWrite's hit test gives the nearest character.
pub fn hit(visible: &[VisibleLine], x: f32, y: f32) -> Option<(u32, u32)> {
    let line = visible.iter().find(|l| l.rect.contains(x, y))?;
    if let Some((_, key)) = line.hits.iter().find(|(r, _)| r.contains(x, y)) {
        return Some(*key);
    }
    let reach = line.rect.h * 0.4;
    line.hits
        .iter()
        .map(|(r, key)| ((r.x - x).max(x - (r.x + r.w)).max(0.), *key))
        .filter(|(d, _)| *d <= reach)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, key)| key)
}

/// `QR_TIMING=1` prints how long each part of drawing a spread takes.
pub fn timing_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("QR_TIMING").is_some())
}
