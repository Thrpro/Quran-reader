//! Read-only adapter around the reader's unchanged DirectWrite/Direct2D code.
//! All COM resources live on the thread that creates Engine. Only owned pixels
//! and ordinary strings leave that thread; no HWND, COM pointer or profile does.
use crate::{
    app_core::{self, Rect, SearchKind},
    data::{Resources, Result},
    render::{BookView, Palette, Renderer},
};
use std::{path::PathBuf, sync::Arc, time::Instant};
use windows::Win32::{
    Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS,
    Graphics::Imaging::GUID_WICPixelFormat32bppPBGRA,
    System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize},
};

const MAX_PIXEL_BYTES: usize = 32 * 1024 * 1024;

/// Fail closed for optional downloads while reusing the native data module.
/// The prototype only opens its three original bundled study resources.
pub(crate) mod bundled_only {
    pub struct Source {
        pub id: &'static str,
    }
    pub const SOURCES: [Source; 0] = [];
    pub struct Library;
    impl Library {
        pub fn read_ayah(
            &self,
            _source: usize,
            _surah: u32,
            _ayah: u32,
        ) -> crate::data::Result<Option<String>> {
            Err("Optional study downloads are unavailable in this prototype".into())
        }
    }
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        // Engine is created and destroyed by its owning worker. Renderer and
        // Resources are declared before this guard and therefore drop first.
        unsafe { RoUninitialize() };
    }
}

pub struct Engine {
    resources: Resources,
    renderer: Renderer,
    image_source: Rect,
    hover_pixels: Vec<u8>,
    hover_size: (u32, u32),
    hover_theme: u32,
    hover_cached: Option<((u32, u32), Arc<HoverResult>)>,
    _apartment: Apartment,
}

pub struct Rendered {
    /// Opaque sRGB RGBA in original physical-pixel order, without resampling.
    pub rgba: Vec<u8>,
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// Position and size in the requested canvas's logical DIP coordinates.
    pub source: Rect,
    pub book_bounds: Rect,
    pub single_page: bool,
    pub elapsed_ms: f64,
    pub interactive_lines: usize,
    pub rgba_fingerprint: u64,
}
pub struct Prepared {
    pub raster: Rendered,
    visible: Vec<crate::render::InkLine>,
    style: RenderStyle,
}

pub struct HoverPatch {
    pub rect: Rect,
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
#[derive(Default)]
pub struct HoverResult {
    pub regions: Vec<Rect>,
    pub patches: Vec<HoverPatch>,
}

#[derive(Clone)]
pub struct SearchResult {
    pub badge: String,
    pub title: String,
    pub secondary: String,
    pub text: String,
    pub page: u32,
    pub key: Option<(u32, u32)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderStyle {
    pub theme: u32,
    pub contextual: bool,
    pub show_clock: bool,
    pub clock_24: bool,
    pub single_page: bool,
    pub study_source: u32,
    pub recitation: Option<(u32, u32)>,
    pub opening: Option<u32>,
}

impl Engine {
    pub fn open(root: PathBuf) -> Result<Self> {
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED)? };
        let apartment = Apartment;
        let mut resources = Resources::open(root.canonicalize()?)?;
        let mut renderer = Renderer::new(&mut resources)?;
        // Original natural script; a user profile is deliberately never read.
        renderer.contextual = false;
        renderer.show_clock = false;
        Ok(Self {
            resources,
            renderer,
            image_source: Rect::default(),
            hover_pixels: Vec::new(),
            hover_size: (0, 0),
            hover_theme: 0,
            hover_cached: None,
            _apartment: apartment,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_styled(
        &mut self,
        page: u32,
        width_dip: f32,
        height_dip: f32,
        scale: f32,
        arabic: bool,
        selection: Option<(u32, u32)>,
        style: RenderStyle,
    ) -> Result<Rendered> {
        let prepared =
            self.prepare_styled(page, width_dip, height_dip, scale, arabic, selection, style)?;
        Ok(self.activate(prepared))
    }

    pub fn activate(&mut self, prepared: Prepared) -> Rendered {
        let r = prepared.raster;
        self.image_source = r.source;
        self.hover_pixels = r.rgba.clone();
        self.hover_size = (r.pixel_width, r.pixel_height);
        self.hover_theme = prepared.style.theme;
        self.hover_cached = None;
        self.renderer.visible = prepared.visible;
        self.renderer.contextual = prepared.style.contextual;
        self.renderer.show_clock = prepared.style.show_clock;
        self.renderer.clock_24 = prepared.style.clock_24;
        r
    }

    /// Prepares pixels and native hit maps without replacing the active page.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_styled(
        &mut self,
        page: u32,
        width_dip: f32,
        height_dip: f32,
        scale: f32,
        arabic: bool,
        selection: Option<(u32, u32)>,
        style: RenderStyle,
    ) -> Result<Prepared> {
        let began = Instant::now();
        if !(1..=604).contains(&page)
            || !width_dip.is_finite()
            || !height_dip.is_finite()
            || !(200. ..=4096.).contains(&width_dip)
            || !(160. ..=4096.).contains(&height_dip)
            || !scale.is_finite()
            || !(0.5..=4.).contains(&scale)
            || selection
                .is_some_and(|(s, a)| !crate::data::valid_reference(&self.resources.surahs, s, a))
            || style.theme > 2
            || style
                .recitation
                .is_some_and(|(s, a)| !crate::data::valid_reference(&self.resources.surahs, s, a))
            || style
                .opening
                .is_some_and(|s| !(1..=114).contains(&s) || s == 9)
        {
            return Err("Reader render request is outside prototype limits".into());
        }
        self.renderer.contextual = style.contextual;
        self.renderer.show_clock = style.show_clock;
        self.renderer.clock_24 = style.clock_24;
        let drawing_width = if style.single_page {
            width_dip.min(649.)
        } else {
            width_dip
        };
        let area = Rect {
            x: (width_dip - drawing_width) / 2.,
            y: 4.,
            w: drawing_width,
            h: height_dip - 8.,
        };
        let geometry = app_core::book_geometry(area);
        let raster = self.renderer.raster_book_with_aa(
            &mut self.resources,
            BookView {
                page,
                area,
                palette: Palette::new(style.theme),
                selection,
                recitation: style.recitation,
                opening: style.opening,
                arabic,
            },
            96. * scale,
            true,
        )?;
        let pixel_width = raster.pixel_size.width;
        let pixel_height = raster.pixel_size.height;
        let bytes = pixel_byte_count(pixel_width, pixel_height)?;
        let stride = pixel_width.checked_mul(4).ok_or("Pixel stride overflow")?;
        let mut rgba = vec![0; bytes];
        unsafe {
            if raster.wic.GetPixelFormat()? != GUID_WICPixelFormat32bppPBGRA {
                return Err("Unexpected native book pixel format".into());
            }
            raster.wic.CopyPixels(std::ptr::null(), stride, &mut rgba)?;
        }
        let bg = Palette::new(style.theme).background;
        let rgba_fingerprint = premultiplied_to_rgba(
            &mut rgba,
            [bg.r, bg.g, bg.b].map(|v| (v.clamp(0., 1.) * 255.).round() as u8),
        )?;
        let interactive_lines = raster.visible.len();
        Ok(Prepared {
            visible: raster.visible,
            style,
            raster: Rendered {
                rgba,
                pixel_width,
                pixel_height,
                source: raster.source,
                book_bounds: raster.bounds,
                single_page: geometry.single,
                elapsed_ms: began.elapsed().as_secs_f64() * 1000.,
                interactive_lines,
                rgba_fingerprint,
            },
        })
    }

    /// Coordinates are logical DIP positions within the displayed image,
    /// whose display size must be exactly Rendered::source.w/h.
    pub fn hit(&self, image_x: f32, image_y: f32) -> Option<(u32, u32)> {
        if !image_x.is_finite()
            || !image_y.is_finite()
            || image_x < 0.
            || image_y < 0.
            || image_x > self.image_source.w
            || image_y > self.image_source.h
        {
            return None;
        }
        self.renderer
            .hit(image_x + self.image_source.x, image_y + self.image_source.y)
    }

    /// Actual native verse bounds, local to the transferred book image.
    /// The same source hit regions or DirectWrite ranges drive selection.
    pub fn hover_regions(&self, x: f32, y: f32) -> Vec<Rect> {
        let Some(key) = self.hit(x, y) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for line in &self.renderer.visible {
            if let Some(regions) = &line.hits {
                let mut selected = regions.iter().filter(|(_, k)| *k == key);
                if let Some((first, _)) = selected.next() {
                    let (left, right) = selected
                        .fold((first.x, first.x + first.w), |(l, r), (v, _)| {
                            (l.min(v.x), r.max(v.x + v.w))
                        });
                    result.push(Rect {
                        x: left - self.image_source.x,
                        y: first.y + first.h * 0.14 - self.image_source.y,
                        w: right - left,
                        h: first.h * 0.72,
                    });
                }
            } else {
                let mut selected = line.runs.iter().filter(|run| (run.surah, run.ayah) == key);
                if let Some(first) = selected.next() {
                    let last = selected.next_back().unwrap_or(first);
                    let mut count = 0;
                    let mut bounds = [DWRITE_HIT_TEST_METRICS::default(); 64];
                    let hit = unsafe {
                        line.layout.HitTestTextRange(
                            first.start,
                            last.start + last.len - first.start,
                            0.,
                            0.,
                            Some(&mut bounds),
                            &mut count,
                        )
                    };
                    if hit.is_ok() {
                        for b in bounds.iter().take(count.min(64) as usize) {
                            result.push(Rect {
                                x: line.origin.X + b.left * line.scale - 2. - self.image_source.x,
                                y: line.rect.y + line.rect.h * 0.14 - self.image_source.y,
                                w: b.width * line.scale + 4.,
                                h: line.rect.h * 0.72,
                            });
                        }
                    }
                }
            }
        }
        result.truncate(64);
        result
    }

    pub fn hover_result(&mut self, x: f32, y: f32) -> Arc<HoverResult> {
        let Some(key) = self.hit(x, y) else {
            return Arc::new(HoverResult::default());
        };
        if let Some((previous, result)) = &self.hover_cached
            && *previous == key
        {
            return result.clone();
        }
        let regions = self.hover_regions(x, y);
        let (width, height) = self.hover_size;
        let sx = width as f32 / self.image_source.w;
        let sy = height as f32 / self.image_source.h;
        let palette = Palette::new(self.hover_theme);
        let rgb = |c: windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F| {
            [c.r, c.g, c.b].map(|v| v * 255.)
        };
        let (paper, ink, fill) = (rgb(palette.paper), rgb(palette.ink), rgb(palette.highlight));
        let mut patches = Vec::new();
        for r in &regions {
            let left = (r.x * sx).floor().clamp(0., width as f32) as u32;
            let top = (r.y * sy).floor().clamp(0., height as f32) as u32;
            let right = ((r.x + r.w) * sx).ceil().clamp(left as f32, width as f32) as u32;
            let bottom = ((r.y + r.h) * sy).ceil().clamp(top as f32, height as f32) as u32;
            let (w, h) = (right - left, bottom - top);
            if w == 0 || h == 0 {
                continue;
            }
            let mut pixels = Vec::with_capacity((w * h * 4) as usize);
            let radius = (3. * sx.min(sy)).min(w.min(h) as f32 / 2.);
            for yy in 0..h {
                for xx in 0..w {
                    let at = (((top + yy) * width + left + xx) * 4) as usize;
                    let mut pixel: [u8; 4] = self.hover_pixels[at..at + 4].try_into().unwrap();
                    let dx = (radius - (xx as f32 + 0.5).min(w as f32 - xx as f32 - 0.5)).max(0.);
                    let dy = (radius - (yy as f32 + 0.5).min(h as f32 - yy as f32 - 0.5)).max(0.);
                    if dx * dx + dy * dy <= radius * radius {
                        pixel = highlight_background(pixel, paper, ink, fill);
                    }
                    pixels.extend_from_slice(&pixel);
                }
            }
            patches.push(HoverPatch {
                rect: Rect {
                    x: left as f32 / sx,
                    y: top as f32 / sy,
                    w: w as f32 / sx,
                    h: h as f32 / sy,
                },
                pixels,
                width: w,
                height: h,
            });
        }
        let result = Arc::new(HoverResult { regions, patches });
        self.hover_cached = Some((key, result.clone()));
        result
    }

    /// Bounded QA probe through the same authoritative hit path as pointer
    /// interaction. Returned coordinates are local to the displayed image.
    pub fn probe_hit(&self) -> Option<(f32, f32, (u32, u32))> {
        for line in self.renderer.visible.iter().take(32) {
            if let Some(regions) = &line.hits {
                for (rect, expected) in regions.iter().take(128) {
                    let x = rect.x + rect.w / 2. - self.image_source.x;
                    let y = rect.y + rect.h / 2. - self.image_source.y;
                    if self.hit(x, y) == Some(*expected) {
                        return Some((x, y, *expected));
                    }
                }
            }
            // Natural opening pages use exact DirectWrite character hits.
            // Accept sampled points only when the native hit test finds a run.
            for row in 1..=4 {
                for column in 1..=48 {
                    let x = line.rect.x + line.rect.w * column as f32 / 49. - self.image_source.x;
                    let y = line.rect.y + line.rect.h * row as f32 / 5. - self.image_source.y;
                    if let Some(key) = self.hit(x, y) {
                        return Some((x, y, key));
                    }
                }
            }
        }
        None
    }

    pub fn search_mode(
        &mut self,
        query: &str,
        arabic: bool,
        mode: i32,
    ) -> Result<Vec<SearchResult>> {
        // Native bounded search accepts Arabic/English chapter names, a page
        // number, a verse reference, or text. Source strings remain unchanged.
        if mode == 2 {
            let pages: Vec<u32> = if query.trim().is_empty() {
                (1..=604).collect()
            } else {
                crate::page_number(query).into_iter().collect()
            };
            return Ok(pages
                .into_iter()
                .take(604)
                .map(|page| SearchResult {
                    badge: page.to_string(),
                    title: if arabic {
                        format!("الصفحة {page}")
                    } else {
                        format!("Page {page}")
                    },
                    text: String::new(),
                    secondary: String::new(),
                    page,
                    key: None,
                })
                .collect());
        }
        let hits = match mode {
            0 if query.trim().is_empty() => self.resources.browse_surahs(query)?,
            0 if crate::page_number(query).is_some_and(|n| n <= 114) => {
                let mut hits = self.resources.browse_surahs(query)?;
                hits.extend(self.resources.browse_search(query)?);
                hits
            }
            0 => self.resources.browse_search(query)?,
            1 => self.resources.browse_surahs(query)?,
            3 => self.resources.browse_sections(SearchKind::Juz, query)?,
            4 => self.resources.browse_sections(SearchKind::Hizb, query)?,
            5 => self.resources.browse_sections(SearchKind::Rub, query)?,
            _ => return Err("Unknown search category".into()),
        };
        Ok(hits
            .into_iter()
            .take(if mode == 0 && !query.trim().is_empty() {
                80
            } else {
                240
            })
            .map(|hit| {
                let key = (matches!(hit.kind, SearchKind::Ayah) && hit.surah > 0 && hit.ayah > 0)
                    .then_some((hit.surah, hit.ayah));
                let surah = (mode == 1 || matches!(hit.kind, SearchKind::Surah))
                    .then(|| self.resources.surahs.iter().find(|s| s.id == hit.surah))
                    .flatten();
                let title = if mode >= 3 {
                    let label = match (mode, arabic) {
                        (3, true) => "الجزء",
                        (4, true) => "الحزب",
                        (5, true) => "الربع",
                        (3, false) => "Juz",
                        (4, false) => "Hizb",
                        _ => "Quarter",
                    };
                    format!("{label} {}", hit.key)
                } else if matches!(hit.kind, SearchKind::Page) {
                    if arabic {
                        format!("الصفحة {}", hit.page_number)
                    } else {
                        format!("Page {}", hit.page_number)
                    }
                } else if let Some(s) = surah {
                    if arabic {
                        s.name_arabic.clone()
                    } else {
                        s.name_simple.clone()
                    }
                } else {
                    self.reference_label((hit.surah, hit.ayah), arabic)
                };
                SearchResult {
                    badge: surah.map(|s| s.id.to_string()).unwrap_or_default(),
                    title,
                    secondary: surah
                        .map(|s| {
                            if arabic {
                                s.name_simple.clone()
                            } else {
                                s.name_arabic.clone()
                            }
                        })
                        .unwrap_or_default(),
                    text: surah
                        .map(|s| {
                            if arabic {
                                format!("{} آيات · الصفحة {}", s.verses_count, hit.page_number)
                            } else {
                                format!("{} verses · Page {}", s.verses_count, hit.page_number)
                            }
                        })
                        .unwrap_or(hit.text),
                    page: hit.page_number,
                    key,
                }
            })
            .collect())
    }

    pub fn study_source(&self, key: (u32, u32), source: u32) -> Result<String> {
        let name = match source {
            0 => "mukhtasar",
            1 => "saadi",
            2 => "sahih",
            _ => return Err("Unknown bundled study source".into()),
        };
        let text = self.resources.study(name, key.0, key.1)?;
        Ok(if source == 2 {
            format_translation_notes(text)
        } else {
            text
        })
    }

    pub fn page_for(&mut self, key: (u32, u32)) -> Result<u32> {
        if !crate::data::valid_reference(&self.resources.surahs, key.0, key.1) {
            return Err("Invalid verse reference".into());
        }
        self.resources
            .search(&format!("{}:{}", key.0, key.1))?
            .first()
            .map(|hit| hit.page_number)
            .ok_or_else(|| "Verse is not in the original index".into())
    }

    pub fn step_ayah(&mut self, key: (u32, u32), direction: i32) -> Result<(u32, u32)> {
        if !crate::data::valid_reference(&self.resources.surahs, key.0, key.1) {
            return Err("Invalid verse reference".into());
        }
        let mut surah = key.0;
        let mut ayah = key.1;
        if direction > 0 {
            if ayah < self.resources.surahs[(surah - 1) as usize].verses_count {
                ayah += 1;
            } else if surah < 114 {
                surah += 1;
                ayah = 1;
            }
        } else if direction < 0 {
            if ayah > 1 {
                ayah -= 1;
            } else if surah > 1 {
                surah -= 1;
                ayah = self.resources.surahs[(surah - 1) as usize].verses_count;
            }
        }
        Ok((surah, ayah))
    }

    pub fn reference_label(&self, key: (u32, u32), arabic: bool) -> String {
        self.resources
            .surahs
            .iter()
            .find(|surah| surah.id == key.0)
            .map(|surah| {
                if arabic {
                    format!("{} · الآية {}", surah.name_arabic, key.1)
                } else {
                    format!("{} · Ayah {}", surah.name_simple, key.1)
                }
            })
            .unwrap_or_else(|| format!("{}:{}", key.0, key.1))
    }
}

fn highlight_background(
    mut pixel: [u8; 4],
    paper: [f32; 3],
    ink: [f32; 3],
    fill: [f32; 3],
) -> [u8; 4] {
    let direction = [ink[0] - paper[0], ink[1] - paper[1], ink[2] - paper[2]];
    let norm = direction.iter().map(|v| v * v).sum::<f32>().max(1.);
    let alpha = (direction
        .iter()
        .enumerate()
        .map(|(i, v)| (f32::from(pixel[i]) - paper[i]) * v)
        .sum::<f32>()
        / norm)
        .clamp(0., 1.);
    // Leave existing coloured selection/recitation backgrounds and solid ink untouched.
    if alpha >= 0.98
        || (0..3).any(|i| (f32::from(pixel[i]) - (paper[i] + alpha * direction[i])).abs() > 8.)
    {
        return pixel;
    }
    for i in 0..3 {
        pixel[i] = (fill[i] * (1. - alpha) + ink[i] * alpha)
            .round()
            .clamp(0., 255.) as u8;
    }
    pixel
}

fn format_translation_notes(text: String) -> String {
    // Native Saheeh entries append their validated, numbered notes after the
    // translated verse. Add a quiet section heading without rewriting either.
    if let Some(index) = text.find("\r\n\r\n1. ") {
        let mut result = String::with_capacity(text.len() + 24);
        result.push_str(&text[..index]);
        result.push_str("\r\n\r\nFootnotes\r\n────────\r\n");
        result.push_str(&text[index + 4..]);
        result
    } else {
        text
    }
}

#[cfg(test)]
mod footnote_format_tests {
    use super::format_translation_notes;
    #[test]
    fn notes_get_a_heading_without_losing_or_reordering_source_text() {
        let source = "Translation.\r\n\r\n1. First note.\r\n\r\n2. Second note.";
        let formatted = format_translation_notes(source.to_owned());
        assert_eq!(
            formatted.replace("\r\n\r\nFootnotes\r\n────────\r\n", "\r\n\r\n"),
            source
        );
        assert_eq!(
            format_translation_notes("Translation without notes.".into()),
            "Translation without notes."
        );
    }
}

fn pixel_byte_count(width: u32, height: u32) -> Result<usize> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes > 0 && *bytes <= MAX_PIXEL_BYTES as u64)
        .ok_or("Native book pixels exceed the 32 MiB transfer limit")?;
    Ok(usize::try_from(bytes)?)
}

/// The fingerprint detects accidental test differences; it is not an
/// authenticity hash. Authentic resource integrity is checked separately.
#[cfg(test)]
fn opaque_bgr_to_rgba(bytes: &mut [u8]) -> Result<u64> {
    if bytes.is_empty() || bytes.len() > MAX_PIXEL_BYTES || !bytes.len().is_multiple_of(4) {
        return Err("Invalid native pixel buffer".into());
    }
    let mut fingerprint = 0xcbf29ce484222325_u64;
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
        for byte in pixel {
            fingerprint ^= u64::from(*byte);
            fingerprint = fingerprint.wrapping_mul(0x100000001b3);
        }
    }
    Ok(fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flat_highlight_changes_paper_not_ink_or_existing_selection() {
        let (paper, ink, fill) = ([31., 31., 31.], [251., 251., 251.], [48., 73., 58.]);
        assert_eq!(
            highlight_background([31, 31, 31, 255], paper, ink, fill),
            [48, 73, 58, 255]
        );
        assert_eq!(
            highlight_background([251, 251, 251, 255], paper, ink, fill),
            [251, 251, 251, 255]
        );
        assert_eq!(
            highlight_background([48, 73, 58, 255], paper, ink, fill),
            [48, 73, 58, 255]
        );
        assert_eq!(
            highlight_background([141, 141, 141, 255], paper, ink, fill),
            [150, 162, 155, 255]
        );
    }

    #[test]
    fn transfer_bounds_reject_empty_and_excessive_buffers() {
        assert!(pixel_byte_count(0, 600).is_err());
        assert!(pixel_byte_count(u32::MAX, u32::MAX).is_err());
        assert!(pixel_byte_count(4096, 4096).is_err());
        assert_eq!(pixel_byte_count(4096, 2048).unwrap(), MAX_PIXEL_BYTES);
        assert!(pixel_byte_count(4096, 2049).is_err());
        assert_eq!(pixel_byte_count(1024, 768).unwrap(), 3_145_728);
        assert!(opaque_bgr_to_rgba(&mut []).is_err());
        assert!(opaque_bgr_to_rgba(&mut [0, 1, 2]).is_err());
    }

    #[test]
    fn conversion_keeps_order_and_forces_opaque_alpha() {
        let mut pixels = [10, 20, 30, 0, 200, 150, 100, 72];
        let fingerprint = opaque_bgr_to_rgba(&mut pixels).unwrap();
        assert_eq!(pixels, [30, 20, 10, 255, 100, 150, 200, 255]);
        let mut same_colors = [10, 20, 30, 200, 200, 150, 100, 255];
        assert_eq!(opaque_bgr_to_rgba(&mut same_colors).unwrap(), fingerprint);
    }
}

fn premultiplied_to_rgba(bytes: &mut [u8], background: [u8; 3]) -> Result<u64> {
    if bytes.is_empty() || bytes.len() > MAX_PIXEL_BYTES || !bytes.len().is_multiple_of(4) {
        return Err("Invalid native grayscale pixel buffer".into());
    }
    let mut fingerprint = 0xcbf29ce484222325_u64;
    for p in bytes.as_chunks_mut::<4>().0 {
        let inverse = 255 - u16::from(p[3]);
        let rgb = [p[2], p[1], p[0]];
        for i in 0..3 {
            p[i] = (u16::from(rgb[i]) + (u16::from(background[i]) * inverse + 127) / 255).min(255)
                as u8;
        }
        p[3] = 255;
        for byte in p {
            fingerprint ^= u64::from(*byte);
            fingerprint = fingerprint.wrapping_mul(0x100000001b3);
        }
    }
    Ok(fingerprint)
}
#[cfg(test)]
mod grayscale_tests {
    use super::*;
    #[test]
    fn transparent_edges_composite_over_theme_and_opaque_ink_is_exact() {
        let mut p = [0, 0, 0, 0, 20, 30, 40, 255, 50, 50, 50, 128];
        premultiplied_to_rgba(&mut p, [200, 100, 0]).unwrap();
        assert_eq!(p, [200, 100, 0, 255, 40, 30, 20, 255, 150, 100, 50, 255]);
        assert!(premultiplied_to_rgba(&mut [0; 3], [0; 3]).is_err());
    }
}
