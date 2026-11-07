//! DirectWrite shapes source lines; HarfRust handles natural opening-page joins.
//! Direct2D paints the original glyphs once;
//! highlights only add ranges behind those glyphs, never a second text layer.
#[cfg(feature = "qa")]
#[path = "resized_render_qa.rs"]
mod resized_render_qa;
pub use crate::app_core::Rect;
use crate::app_core::{PAGE_HEIGHT, PAGE_WIDTH};
use crate::data::{Page, Resources, Result};
use serde::Deserialize;
#[cfg(feature = "qa")]
use std::path::Path;
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    io::Read,
    path::Path as AssetPath,
    rc::Rc,
};
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::*,
            Imaging::*,
        },
        System::{Com::*, SystemInformation::GetLocalTime},
    },
    core::{HSTRING, Interface, w},
};
use windows_numerics::Vector2;

// Use the same natural page coordinates as the released reader. Scaling the
// page must not independently enlarge the letters or widen its inner margins.
const CONTENT_WIDTH: f32 = 513.;
const FONT_SIZE: f32 = 30.6;
const PAD_TOP: f32 = 24.;
const PAD_BOTTOM: f32 = 34.;
// These are the exact decorative glyphs used by the released reader's original
// QUL v4 font. They are never substituted into Quran source lines.
const V4_BASMALA: &str = "\u{FCAA} \u{FCAB} \u{FCAE} \u{FCB4}";

const UI_LABEL_CACHE_CAPACITY: usize = 128;
const UI_LABEL_CACHE_MAX_TEXT_BYTES: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
struct UiLabelStyle {
    size: u32,
    width: u32,
    height: u32,
    quran_font: bool,
    rtl: bool,
    alignment: i32,
}

struct UiLabel {
    text: String,
    style: UiLabelStyle,
    layout: IDWriteTextLayout,
}

fn bounded_asset(path: &AssetPath, limit: u64) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit {
        return Err("Decorative resource exceeds its read limit".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("Decorative resource exceeds its read limit".into());
    }
    Ok(bytes)
}

struct RegisteredFontLoader {
    factory: IDWriteFactory3,
    loader: IDWriteInMemoryFontFileLoader,
}
impl Drop for RegisteredFontLoader {
    fn drop(&mut self) {
        unsafe {
            let _ = self.factory.UnregisterFontFileLoader(&self.loader);
        }
    }
}
struct PackedFont {
    collection: IDWriteFontCollection,
    family: HSTRING,
    #[cfg(feature = "qa")]
    source_container: &'static str,
    // Collections and cached layouts are released before this registration.
    _registration: RegisteredFontLoader,
}
impl PackedFont {
    fn load(write: &IDWriteFactory3, path: &AssetPath) -> Result<Self> {
        let bytes = bounded_asset(path, 2 * 1024 * 1024)?;
        unsafe {
            // The source header uses native WOFF2 unpacking. Windows rejects
            // the original v4 color font's WOFF2 container, so packaging also
            // supplies its losslessly unpacked sfnt, with source tables checked
            // at build time. Neither route edits or regenerates glyphs.
            let factory: IDWriteFactory5 = write.cast()?;
            let container = factory.AnalyzeContainerType(bytes.as_ptr().cast(), bytes.len() as u32);
            let sfnt = bytes.starts_with(&[0, 1, 0, 0]);
            if container != DWRITE_CONTAINER_TYPE_WOFF2 && !sfnt {
                return Err("Unexpected decorative font container".into());
            }
            let loader = factory.CreateInMemoryFontFileLoader()?;
            write.RegisterFontFileLoader(&loader)?;
            let registration = RegisteredFontLoader {
                factory: write.clone(),
                loader,
            };
            // A null owner requests DirectWrite's own copy; neither the Rust
            // bytes nor an unpacker's fragment escapes this loading operation.
            let font = if sfnt {
                registration.loader.CreateInMemoryFontFileReference(
                    write,
                    bytes.as_ptr().cast(),
                    bytes.len() as u32,
                    None::<&windows::core::IUnknown>,
                )?
            } else {
                let stream = factory
                    .UnpackFontFile(container, bytes.as_ptr().cast(), bytes.len() as u32)
                    .map_err(|error| format!("UnpackFontFile: {error}"))?;
                let size = stream.GetFileSize()?;
                if size == 0 || size > 8 * 1024 * 1024 {
                    return Err("Unpacked decorative font exceeds its read limit".into());
                }
                let mut fragment = std::ptr::null_mut();
                let mut context = std::ptr::null_mut();
                stream.ReadFileFragment(&mut fragment, 0, size, &mut context)?;
                let font = registration.loader.CreateInMemoryFontFileReference(
                    write,
                    fragment,
                    size as u32,
                    None::<&windows::core::IUnknown>,
                );
                stream.ReleaseFileFragment(context);
                font?
            };
            let reference = write
                .CreateFontFaceReference(&font, 0, DWRITE_FONT_SIMULATIONS_NONE)
                .map_err(|error| format!("CreateFontFaceReference: {error}"))?;
            let builder = write.CreateFontSetBuilder()?;
            builder
                .AddFontFaceReference2(&reference)
                .map_err(|error| format!("AddFontFaceReference: {error}"))?;
            let collection: IDWriteFontCollection = write
                .CreateFontCollectionFromFontSet(&builder.CreateFontSet()?)?
                .cast()?;
            let names = collection.GetFontFamily(0)?.GetFamilyNames()?;
            let mut name = vec![0u16; names.GetStringLength(0)? as usize + 1];
            names.GetString(0, &mut name)?;
            let family = HSTRING::from(String::from_utf16(&name[..name.len() - 1])?);
            Ok(Self {
                collection,
                family,
                #[cfg(feature = "qa")]
                source_container: if sfnt { "sfnt" } else { "woff2" },
                _registration: registration,
            })
        }
    }
    #[cfg(feature = "qa")]
    fn verify_glyphs(&self, text: &str) -> Result<()> {
        let points: Vec<_> = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| c as u32)
            .collect();
        let mut glyphs = vec![0u16; points.len()];
        unsafe {
            self.collection
                .GetFontFamily(0)?
                .GetFont(0)?
                .CreateFontFace()?
                .GetGlyphIndices(points.as_ptr(), points.len() as u32, glyphs.as_mut_ptr())?;
        }
        if glyphs.contains(&0) {
            return Err("Original decorative font is missing a source glyph".into());
        }
        Ok(())
    }
    #[cfg(feature = "qa")]
    fn verify_v4_source_mapping(&self) -> Result<()> {
        let points = [0xFCAA, 0xFCAB, 0xFCAE, 0xFCB4];
        let mut glyphs = [0u16; 4];
        unsafe {
            let face = self
                .collection
                .GetFontFamily(0)?
                .GetFont(0)?
                .CreateFontFace()?;
            face.GetGlyphIndices(points.as_ptr(), points.len() as u32, glyphs.as_mut_ptr())?;
            // These are the original QUL v4 cmap entries, verified against its
            // included WOFF2. The build also compares every unpacked table.
            if face.GetGlyphCount() != 166 || glyphs != [150, 151, 154, 160] {
                return Err(
                    "Unpacked v4 font does not match the original source glyph mapping".into(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChromePage {
    page_number: u32,
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
struct PageChrome {
    pages: Vec<ChromePage>,
    hizb: BTreeMap<u32, u32>,
    rub: BTreeMap<u32, u32>,
}
impl PageChrome {
    fn load(root: &AssetPath) -> Result<Self> {
        let pages: Vec<ChromePage> = serde_json::from_slice(&bounded_asset(
            &root.join("data/qul/processed/page-index.json"),
            128 * 1024,
        )?)?;
        if pages.len() != 604
            || pages.iter().enumerate().any(|(i, p)| {
                p.page_number != i as u32 + 1
                    || !(1..=114).contains(&p.first_surah)
                    || !(1..=30).contains(&p.juz)
            })
        {
            return Err("Invalid original page metadata".into());
        }
        let sections = |path: &str, count: usize, rub: bool| -> Result<BTreeMap<u32, u32>> {
            let starts: Vec<SectionStart> =
                serde_json::from_slice(&bounded_asset(&root.join(path), 128 * 1024)?)?;
            if starts.len() != count
                || starts.iter().enumerate().any(|(i, s)| {
                    !(1..=604).contains(&s.page_number)
                        || if rub { s.rub } else { s.hizb } != i as u32 + 1
                })
                || starts
                    .windows(2)
                    .any(|pair| pair[0].page_number > pair[1].page_number)
            {
                return Err("Invalid original section metadata".into());
            }
            Ok(starts
                .into_iter()
                .map(|s| (s.page_number, if rub { s.rub } else { s.hizb }))
                .collect())
        };
        Ok(Self {
            pages,
            hizb: sections("data/qul/processed/hizb.json", 60, false)?,
            rub: sections("data/qul/processed/rub.json", 240, true)?,
        })
    }
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
        let position = (rub - 1) % 4;
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
        (labels[position as usize], rub.div_ceil(4))
    };
    Some(format!("{label} {}", metadata_number(number, arabic)))
}

fn page_rows(w: f32, h: f32) -> (f32, f32, f32, f32) {
    let scale = h / PAGE_HEIGHT;
    (
        (h - (PAD_TOP + PAD_BOTTOM) * scale) / 15.,
        w * CONTENT_WIDTH / PAGE_WIDTH,
        w * 52. / PAGE_WIDTH,
        PAD_TOP * scale,
    )
}

fn ink_bounds(layout: &IDWriteTextLayout, rect: Rect) -> Result<Rect> {
    let overhang = unsafe { layout.GetOverhangMetrics()? };
    Ok(Rect {
        x: rect.x - overhang.left,
        y: rect.y - overhang.top,
        w: rect.w + overhang.left + overhang.right,
        h: rect.h + overhang.top + overhang.bottom,
    })
}

impl Rect {
    pub fn d2d(self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.x,
            top: self.y,
            right: self.x + self.w,
            bottom: self.y + self.h,
        }
    }
}
#[derive(Clone, Copy)]
pub struct Palette {
    pub background: D2D1_COLOR_F,
    pub paper: D2D1_COLOR_F,
    pub ink: D2D1_COLOR_F,
    pub muted: D2D1_COLOR_F,
    pub border: D2D1_COLOR_F,
    pub highlight: D2D1_COLOR_F,
}
fn color(r: u8, g: u8, b: u8) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: r as f32 / 255.,
        g: g as f32 / 255.,
        b: b as f32 / 255.,
        a: 1.,
    }
}
impl Palette {
    pub fn new(theme: u32) -> Self {
        let (background, paper, ink, muted, border, highlight) = match theme {
            1 => (
                (241, 238, 232),
                (255, 253, 248),
                (22, 18, 13),
                (98, 108, 100),
                (215, 222, 213),
                (224, 234, 192),
            ),
            2 => (
                (233, 223, 207),
                (251, 241, 221),
                (24, 16, 8),
                (124, 111, 88),
                (225, 211, 178),
                (235, 222, 166),
            ),
            3 => (
                (0, 0, 0),
                (0, 0, 0),
                (255, 255, 255),
                (147, 164, 155),
                (30, 43, 35),
                (32, 65, 48),
            ),
            _ => (
                (32, 32, 32),
                (31, 31, 31),
                (251, 251, 251),
                (156, 169, 160),
                (57, 66, 61),
                (48, 73, 58),
            ),
        };
        let c = |(r, g, b)| color(r, g, b);
        Self {
            background: c(background),
            paper: c(paper),
            ink: c(ink),
            muted: c(muted),
            border: c(border),
            highlight: c(highlight),
        }
    }
    pub fn background_rgb(self) -> u32 {
        self.background.r.mul_add(255., 0.) as u32
            | ((self.background.g * 255.) as u32) << 8
            | ((self.background.b * 255.) as u32) << 16
    }
    pub fn ink_rgb(self) -> u32 {
        self.ink.r.mul_add(255., 0.) as u32
            | ((self.ink.g * 255.) as u32) << 8
            | ((self.ink.b * 255.) as u32) << 16
    }
    pub fn blend(self, other: Self, t: f32) -> Self {
        let c = |a: D2D1_COLOR_F, b: D2D1_COLOR_F| D2D1_COLOR_F {
            r: a.r + (b.r - a.r) * t,
            g: a.g + (b.g - a.g) * t,
            b: a.b + (b.b - a.b) * t,
            a: 1.,
        };
        let background = c(self.background, other.background);
        let paper = c(self.paper, other.paper);
        let blended_ink = c(self.ink, other.ink);
        let readability = |ink| contrast(ink, paper).min(contrast(ink, background));
        // Dark-to-light interpolation otherwise makes the ink and paper the
        // same gray midway. Fade surfaces smoothly while keeping text visible.
        let ink = if readability(blended_ink) >= 4.5 {
            blended_ink
        } else if readability(self.ink) >= readability(other.ink) {
            self.ink
        } else {
            other.ink
        };
        Self {
            background,
            paper,
            ink,
            muted: c(self.muted, other.muted),
            border: c(self.border, other.border),
            highlight: c(self.highlight, other.highlight),
        }
    }
}
fn contrast(a: D2D1_COLOR_F, b: D2D1_COLOR_F) -> f32 {
    let luminance = |c: D2D1_COLOR_F| {
        let channel = |n: f32| {
            if n <= 0.04045 {
                n / 12.92
            } else {
                ((n + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
    };
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
#[derive(Clone)]
pub struct Run {
    pub start: u32,
    pub len: u32,
    pub surah: u32,
    pub ayah: u32,
}
#[derive(Clone)]
pub struct InkLine {
    pub layout: IDWriteTextLayout,
    pub scale: f32,
    pub origin: Vector2,
    pub runs: Rc<[Run]>,
    pub rect: Rect,
    pub line_number: u32,
    pub hits: Option<Vec<(Rect, (u32, u32))>>,
    decoration: Option<Rc<DecorativeInk>>,
    natural: Option<Rc<NaturalInk>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HighlightStyle {
    Filled,
    Outlined,
}

/// The studied ayah keeps the familiar filled range. A different recitation is
/// outlined as a second location, including when both ayahs share a source row.
/// One location is always a normal highlight, without stacking the two styles.
fn highlight_targets(
    selection: Option<(u32, u32)>,
    recitation: Option<(u32, u32)>,
) -> [Option<((u32, u32), HighlightStyle)>; 2] {
    [
        selection.map(|key| (key, HighlightStyle::Filled)),
        recitation.filter(|key| Some(*key) != selection).map(|key| {
            (
                key,
                if selection.is_some() {
                    HighlightStyle::Outlined
                } else {
                    HighlightStyle::Filled
                },
            )
        }),
    ]
}

fn source_range_bounds(regions: &[(Rect, (u32, u32))], key: (u32, u32)) -> Option<Rect> {
    let mut chosen = regions.iter().filter(|(_, candidate)| *candidate == key);
    let (first, _) = chosen.next()?;
    let (left, right) = chosen.fold((first.x, first.x + first.w), |(left, right), (rect, _)| {
        (left.min(rect.x), right.max(rect.x + rect.w))
    });
    Some(Rect {
        x: left,
        y: first.y + first.h * 0.14,
        w: right - left,
        h: first.h * 0.72,
    })
}

fn ayah_range_bounds(line: &InkLine, key: (u32, u32)) -> Result<Vec<Rect>> {
    if let Some(regions) = &line.hits {
        return Ok(source_range_bounds(regions, key).into_iter().collect());
    }
    let mut selected = line.runs.iter().filter(|run| (run.surah, run.ayah) == key);
    let Some(first) = selected.next() else {
        return Ok(Vec::new());
    };
    let last = selected.next_back().unwrap_or(first);
    let mut count = 0;
    let mut bounds = [DWRITE_HIT_TEST_METRICS::default(); 64];
    unsafe {
        line.layout.HitTestTextRange(
            first.start,
            last.start + last.len - first.start,
            0.,
            0.,
            Some(&mut bounds),
            &mut count,
        )?;
    }
    Ok(bounds
        .iter()
        .take(count as usize)
        .map(|bound| Rect {
            x: line.origin.X + bound.left * line.scale - 2.,
            y: line.rect.y + row_inset(line.rect.h),
            w: bound.width * line.scale + 4.,
            h: line.rect.h * 0.72,
        })
        .collect())
}

#[cfg(feature = "qa")]
#[derive(Clone, Copy)]
struct HighlightRegion {
    key: (u32, u32),
    style: HighlightStyle,
    rect: Rect,
}

struct NaturalInk {
    indices: Vec<u16>,
    advances: Vec<f32>,
    offsets: Vec<DWRITE_GLYPH_OFFSET>,
    size: f32,
    bounds: Rect,
    words: Vec<(f32, f32, (u32, u32))>,
}
struct DecorativeLayer {
    geometry: ID2D1PathGeometry,
    lightness: f32,
}
struct DecorativeInk {
    layers: Vec<DecorativeLayer>,
    bounds: Rect,
}
#[derive(Clone)]
struct PageInk {
    number: u32,
    w: f32,
    h: f32,
    lines: Vec<InkLine>,
}
pub struct Renderer {
    pub factory: ID2D1Factory,
    write: IDWriteFactory3,
    fonts: IDWriteFontCollection,
    natural_shaper: crate::quran_shape::Shaper,
    natural_face: IDWriteFontFace,
    opening_light_params: IDWriteRenderingParams,
    pub font_family: HSTRING,
    cache: VecDeque<PageInk>,
    ui_labels: RefCell<VecDeque<UiLabel>>,
    pub visible: Vec<InkLine>,
    pub target: Option<ID2D1HwndRenderTarget>,
    window_software: bool,
    presentation_generation: u64,
    pub surface_dirty: bool,
    basmala: String,
    pub contextual: bool,
    pub contextual_unavailable: bool,
    outlines: Option<crate::outlines::Outlines>,
    pub show_clock: bool,
    pub clock_24: bool,
    chrome: Option<PageChrome>,
    header_ligatures: BTreeMap<u32, String>,
    header_art: BTreeMap<u32, Rc<DecorativeInk>>,
    // Keep these after the line cache so its layouts release their font
    // references before the in-memory loader registrations are removed.
    header_font: Option<PackedFont>,
    basmala_font: Option<PackedFont>,
    #[cfg(feature = "qa")]
    decorative_load_errors: Vec<String>,
    #[cfg(feature = "qa")]
    opening_highlights: Vec<Rect>,
    #[cfg(feature = "qa")]
    ayah_highlights: Vec<HighlightRegion>,
}
#[derive(Clone, Copy)]
pub struct BookView {
    pub page: u32,
    pub area: Rect,
    pub palette: Palette,
    pub selection: Option<(u32, u32)>,
    /// A currently recited ayah when study remains at a different location.
    pub recitation: Option<(u32, u32)>,
    /// Only an explicitly identified Bismillah clip; never a synthetic ayah.
    pub opening: Option<u32>,
    pub arabic: bool,
}
/// A presentation-only raster of the original book at one monitor density.
/// The caller owns its short-lived cache and device-specific uploaded bitmap.
/// Quran text, source fonts, outline data and word maps remain authoritative.
pub struct BookRaster {
    pub wic: IWICBitmap,
    pub visible: Vec<InkLine>,
    /// Absolute DIP bounds of the cropped pixels, aligned to physical pixels.
    pub source: Rect,
    pub bounds: Rect,
    pub dpi: f32,
    pub pixel_size: D2D_SIZE_U,
    pub contextual: bool,
    pub grayscale: bool,
}
impl BookRaster {
    pub fn pixel_bytes(&self) -> u64 {
        u64::from(self.pixel_size.width)
            .saturating_mul(u64::from(self.pixel_size.height))
            .saturating_mul(4)
    }
    fn validate_bitmap(&self, target: &ID2D1RenderTarget, bitmap: &ID2D1Bitmap) -> Result<()> {
        unsafe {
            let mut dpi_x = 0.;
            let mut dpi_y = 0.;
            target.GetDpi(&mut dpi_x, &mut dpi_y);
            let mut bitmap_dpi_x = 0.;
            let mut bitmap_dpi_y = 0.;
            bitmap.GetDpi(&mut bitmap_dpi_x, &mut bitmap_dpi_y);
            let pixels = bitmap.GetPixelSize();
            if (dpi_x - self.dpi).abs() > 0.01
                || (dpi_y - self.dpi).abs() > 0.01
                || (bitmap_dpi_x - self.dpi).abs() > 0.01
                || (bitmap_dpi_y - self.dpi).abs() > 0.01
                || pixels.width != self.pixel_size.width
                || pixels.height != self.pixel_size.height
            {
                return Err("Book raster and presentation dimensions differ".into());
            }
        }
        Ok(())
    }
    fn paint_pixels(&self, target: &ID2D1RenderTarget, bitmap: &ID2D1Bitmap, rotated: bool) {
        unsafe {
            target.DrawBitmap(
                bitmap,
                Some(&self.source.d2d()),
                1.,
                if rotated {
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR
                } else {
                    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR
                },
                None,
            );
        }
    }
    /// Upload once per presentation device. WIC's own DPI metadata is ignored
    /// by Direct2D, so preserve the raster's actual monitor density explicitly.
    pub fn upload(&self, target: &ID2D1RenderTarget) -> Result<ID2D1Bitmap> {
        unsafe {
            let maximum = target.GetMaximumBitmapSize();
            if self.pixel_size.width > maximum || self.pixel_size.height > maximum {
                return Err("Book raster exceeds the presentation device limit".into());
            }
            Ok(target.CreateBitmapFromWicBitmap(
                &self.wic,
                Some(&D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: if self.grayscale {
                            D2D1_ALPHA_MODE_PREMULTIPLIED
                        } else {
                            D2D1_ALPHA_MODE_IGNORE
                        },
                    },
                    dpiX: self.dpi,
                    dpiY: self.dpi,
                }),
            )?)
        }
    }
}
impl Renderer {
    pub fn new(resources: &mut Resources) -> Result<Self> {
        unsafe {
            let factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let write: IDWriteFactory3 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let path = HSTRING::from(
                resources
                    .root
                    .join("fonts/digital-khatt/DigitalKhattV2.otf")
                    .to_string_lossy()
                    .as_ref(),
            );
            let reference =
                write.CreateFontFaceReference2(&path, None, 0, DWRITE_FONT_SIMULATIONS_NONE)?;
            let builder = write.CreateFontSetBuilder()?;
            builder.AddFontFaceReference2(&reference)?;
            let fonts: IDWriteFontCollection = write
                .CreateFontCollectionFromFontSet(&builder.CreateFontSet()?)?
                .cast()?;
            let names = fonts.GetFontFamily(0)?.GetFamilyNames()?;
            let mut name = vec![0u16; names.GetStringLength(0)? as usize + 1];
            names.GetString(0, &mut name)?;
            let font_family = HSTRING::from(String::from_utf16(&name[..name.len() - 1])?);
            let mut font_bytes = Vec::new();
            std::fs::File::open(
                resources
                    .root
                    .join("fonts/digital-khatt/DigitalKhattV2.otf"),
            )?
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut font_bytes)?;
            let natural_shaper = crate::quran_shape::Shaper::new(font_bytes)?;
            let natural_face = fonts.GetFontFamily(0)?.GetFont(0)?.CreateFontFace()?;
            let page = resources.page(1)?;
            let basmala = page
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
            let header_ligatures = (|| -> Result<BTreeMap<u32, String>> {
                let original: BTreeMap<String, String> = serde_json::from_slice(&bounded_asset(
                    &resources.root.join("fonts/surah-header/ligatures.json"),
                    32 * 1024,
                )?)?;
                if original.len() != 114 {
                    return Err("Incomplete original surah-header map".into());
                }
                let mut map = BTreeMap::new();
                for number in 1..=114 {
                    let glyph = original
                        .get(&format!("surah-{number}"))
                        .ok_or("Missing source surah-header glyph")?
                        .trim();
                    if glyph.chars().count() != 1 {
                        return Err("Invalid source surah-header glyph".into());
                    }
                    map.insert(number, glyph.to_owned());
                }
                Ok(map)
            })()
            .unwrap_or_default();
            let unpacked_header = resources
                .root
                .join("fonts/surah-header/QCF_SurahHeader_COLOR-Regular.ttf");
            let header_path = if unpacked_header.is_file() {
                unpacked_header
            } else {
                resources
                    .root
                    .join("fonts/surah-header/QCF_SurahHeader_COLOR-Regular.woff2")
            };
            let header_result = PackedFont::load(&write, &header_path);
            let unpacked_basmala = resources.root.join("fonts/surah-names/v4/surah_names.ttf");
            let basmala_path = if unpacked_basmala.is_file() {
                unpacked_basmala
            } else {
                resources
                    .root
                    .join("fonts/surah-names/v4/surah_names.woff2")
            };
            let basmala_result = PackedFont::load(&write, &basmala_path);
            #[cfg(feature = "qa")]
            let decorative_load_errors = [
                (&header_result, &header_path),
                (&basmala_result, &basmala_path),
            ]
            .into_iter()
            .filter_map(|(result, path)| {
                result
                    .as_ref()
                    .err()
                    .map(|error| format!("{}: {error}", path.display()))
            })
            .collect();
            let header_font = header_result.ok();
            let basmala_font = basmala_result.ok();
            let chrome = PageChrome::load(&resources.root).ok();
            let base: IDWriteFactory = write.cast()?;
            let defaults = base.CreateRenderingParams()?;
            // Preserve the source face and all shaped positions. Light paper
            // does not need ClearType's extra contrast/optical emboldening.
            let opening_light_params = base.CreateCustomRenderingParams(
                defaults.GetGamma(),
                0.,
                defaults.GetClearTypeLevel(),
                defaults.GetPixelGeometry(),
                defaults.GetRenderingMode(),
            )?;
            Ok(Self {
                factory,
                write,
                fonts,
                natural_shaper,
                natural_face,
                opening_light_params,
                font_family,
                cache: VecDeque::new(),
                ui_labels: RefCell::new(VecDeque::new()),
                visible: vec![],
                target: None,
                window_software: false,
                presentation_generation: 0,
                surface_dirty: true,
                basmala,
                contextual: false,
                contextual_unavailable: false,
                outlines: None,
                show_clock: true,
                clock_24: false,
                chrome,
                header_ligatures,
                header_art: BTreeMap::new(),
                header_font,
                basmala_font,
                #[cfg(feature = "qa")]
                decorative_load_errors,
                #[cfg(feature = "qa")]
                opening_highlights: Vec::new(),
                #[cfg(feature = "qa")]
                ayah_highlights: Vec::new(),
            })
        }
    }
    fn original_header(&mut self, number: u32) -> Result<Rc<DecorativeInk>> {
        if let Some(art) = self.header_art.get(&number) {
            return Ok(art.clone());
        }
        let font = self
            .header_font
            .as_ref()
            .ok_or("Original layered header font unavailable")?;
        let text = self
            .header_ligatures
            .get(&number)
            .ok_or("Missing original header mapping")?;
        let code = text.chars().next().ok_or("Empty header mapping")? as u32;
        unsafe {
            let face = font
                .collection
                .GetFontFamily(0)?
                .GetFont(0)?
                .CreateFontFace()?;
            let mut glyph = 0u16;
            face.GetGlyphIndices(&code, 1, &mut glyph)?;
            if glyph == 0 {
                return Err("Missing original header glyph".into());
            }
            let write: IDWriteFactory2 = self.write.cast()?;
            let mut run = DWRITE_GLYPH_RUN {
                fontFace: std::mem::ManuallyDrop::new(Some(face)),
                fontEmSize: 138.4,
                glyphCount: 1,
                glyphIndices: &glyph,
                ..Default::default()
            };
            let enumeration = write.TranslateColorGlyphRun(
                0.,
                0.,
                &run,
                None,
                DWRITE_MEASURING_MODE_NATURAL,
                None,
                0,
            );
            std::mem::ManuallyDrop::drop(&mut run.fontFace);
            let enumeration = enumeration?;
            let mut layers = Vec::new();
            let mut bounds: Option<D2D_RECT_F> = None;
            while enumeration.MoveNext()?.as_bool() {
                if layers.len() >= 32 {
                    return Err("Original header has too many layers".into());
                }
                let current = enumeration.GetCurrentRun()?;
                let current = current.as_ref().ok_or("Missing original color run")?;
                let glyph_run = &current.glyphRun;
                if glyph_run.glyphCount == 0 || glyph_run.glyphCount > 32 {
                    return Err("Invalid header layer run".into());
                }
                let raw = self.factory.CreatePathGeometry()?;
                let sink = raw.Open()?;
                glyph_run
                    .fontFace
                    .as_ref()
                    .ok_or("Missing header layer font")?
                    .GetGlyphRunOutline(
                        glyph_run.fontEmSize,
                        glyph_run.glyphIndices,
                        (!glyph_run.glyphAdvances.is_null()).then_some(glyph_run.glyphAdvances),
                        (!glyph_run.glyphOffsets.is_null()).then_some(glyph_run.glyphOffsets),
                        glyph_run.glyphCount,
                        glyph_run.isSideways.as_bool(),
                        glyph_run.bidiLevel % 2 != 0,
                        &sink,
                    )?;
                sink.Close()?;
                let translated = self.factory.CreatePathGeometry()?;
                let sink = translated.Open()?;
                raw.Simplify(
                    D2D1_GEOMETRY_SIMPLIFICATION_OPTION_CUBICS_AND_LINES,
                    Some(&windows_numerics::Matrix3x2 {
                        M31: current.baselineOriginX,
                        M32: current.baselineOriginY,
                        ..windows_numerics::Matrix3x2::identity()
                    }),
                    0.001,
                    &sink,
                )?;
                sink.Close()?;
                let b = translated.GetBounds(None)?;
                if ![b.left, b.top, b.right, b.bottom]
                    .iter()
                    .all(|v| v.is_finite())
                {
                    return Err("Invalid header bounds".into());
                }
                bounds = Some(match bounds {
                    None => b,
                    Some(a) => D2D_RECT_F {
                        left: a.left.min(b.left),
                        top: a.top.min(b.top),
                        right: a.right.max(b.right),
                        bottom: a.bottom.max(b.bottom),
                    },
                });
                let c = current.runColor;
                let lightness = if current.paletteIndex == 0xffff {
                    0.
                } else {
                    (0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b).clamp(0., 1.)
                };
                layers.push(DecorativeLayer {
                    geometry: translated,
                    lightness,
                });
            }
            let b = bounds.ok_or("Original header contains no decorative layers")?;
            if b.right <= b.left || b.bottom <= b.top {
                return Err("Empty original header geometry".into());
            }
            let art = Rc::new(DecorativeInk {
                layers,
                bounds: Rect {
                    x: b.left,
                    y: b.top,
                    w: b.right - b.left,
                    h: b.bottom - b.top,
                },
            });
            self.header_art.insert(number, art.clone());
            Ok(art)
        }
    }
    fn text(
        &self,
        text: &str,
        size: f32,
        width: f32,
        height: f32,
        quran: bool,
        rtl: bool,
    ) -> Result<IDWriteTextLayout> {
        let collection = if quran { Some(&self.fonts) } else { None };
        // CSS aliases are not the original font's DirectWrite family name.
        let family = if quran {
            self.font_family.clone()
        } else {
            HSTRING::from("Segoe UI")
        };
        self.font_text(text, size, width, height, collection, &family, rtl)
    }
    #[allow(clippy::too_many_arguments)]
    fn font_text(
        &self,
        text: &str,
        size: f32,
        width: f32,
        height: f32,
        collection: Option<&IDWriteFontCollection>,
        family: &HSTRING,
        rtl: bool,
    ) -> Result<IDWriteTextLayout> {
        unsafe {
            let format = self.write.CreateTextFormat(
                family,
                collection,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                if rtl { w!("ar") } else { w!("en") },
            )?;
            format.SetReadingDirection(if rtl {
                DWRITE_READING_DIRECTION_RIGHT_TO_LEFT
            } else {
                DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
            })?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            let layout = self.write.CreateTextLayout(
                &text.encode_utf16().collect::<Vec<_>>(),
                &format,
                width,
                height,
            )?;
            Ok(layout)
        }
    }
    pub fn label(
        &self,
        target: &ID2D1RenderTarget,
        text: &str,
        rect: Rect,
        size: f32,
        ink: D2D1_COLOR_F,
        rtl: bool,
    ) -> Result<()> {
        unsafe {
            let layout =
                self.ui_label_layout(text, size, rect, false, rtl, DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let brush = target.CreateSolidColorBrush(&ink, None)?;
            target.DrawTextLayout(
                Vector2 {
                    X: rect.x,
                    Y: rect.y,
                },
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
            Ok(())
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn label_start(
        &self,
        target: &ID2D1RenderTarget,
        text: &str,
        rect: Rect,
        size: f32,
        ink: D2D1_COLOR_F,
        rtl: bool,
    ) -> Result<()> {
        self.chrome_label(target, text, rect, size, ink, rtl, !rtl, false)
    }
    #[allow(clippy::too_many_arguments)]
    fn chrome_label(
        &self,
        target: &ID2D1RenderTarget,
        text: &str,
        rect: Rect,
        size: f32,
        ink: D2D1_COLOR_F,
        rtl: bool,
        left: bool,
        quran_font: bool,
    ) -> Result<()> {
        unsafe {
            let layout = self.ui_label_layout(
                text,
                size,
                rect,
                quran_font,
                rtl,
                if left != rtl {
                    DWRITE_TEXT_ALIGNMENT_LEADING
                } else {
                    DWRITE_TEXT_ALIGNMENT_TRAILING
                },
            )?;
            target.DrawTextLayout(
                Vector2 {
                    X: rect.x,
                    Y: rect.y,
                },
                &layout,
                &target.CreateSolidColorBrush(&ink, None)?,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn ui_label_layout(
        &self,
        text: &str,
        size: f32,
        rect: Rect,
        quran_font: bool,
        rtl: bool,
        alignment: DWRITE_TEXT_ALIGNMENT,
    ) -> Result<IDWriteTextLayout> {
        let style = UiLabelStyle {
            size: size.to_bits(),
            width: rect.w.to_bits(),
            height: rect.h.to_bits(),
            quran_font,
            rtl,
            alignment: alignment.0,
        };
        {
            let mut labels = self.ui_labels.borrow_mut();
            if let Some(index) = labels
                .iter()
                .position(|entry| entry.text == text && entry.style == style)
            {
                let entry = labels.remove(index).unwrap();
                let layout = entry.layout.clone();
                labels.push_front(entry);
                return Ok(layout);
            }
        }
        // Only presentation captions share these immutable layouts. Authentic
        // Quran rows retain their separate mutable spacing/shape preparation.
        // Position, palette and monitor density do not change DIP text layout.
        let layout = self.text(text, size, rect.w, rect.h, quran_font, rtl)?;
        unsafe {
            layout.SetTextAlignment(alignment)?;
        }
        // A long diagnostic still renders in full without retaining a large
        // title in the small caption cache. Its font and text remain unchanged.
        if text.len() <= UI_LABEL_CACHE_MAX_TEXT_BYTES {
            let mut labels = self.ui_labels.borrow_mut();
            labels.push_front(UiLabel {
                text: text.to_owned(),
                style,
                layout: layout.clone(),
            });
            labels.truncate(UI_LABEL_CACHE_CAPACITY);
        }
        Ok(layout)
    }

    #[cfg(feature = "qa")]
    pub fn verify_ui_label_cache(&self) -> Result<serde_json::Value> {
        let rect = Rect {
            x: 0.,
            y: 0.,
            w: 400.,
            h: 34.,
        };
        let mut checked_positions = 0;
        let mut cases = Vec::new();
        for (text, rtl, quran_font) in [
            ("Quran, surah or page", false, false),
            ("القرآن أو السورة أو الصفحة", true, false),
            ("المصدر: Quran.com · تفسير عربي", true, false),
            ("الكهف", true, true),
            ("12:34", false, false),
        ] {
            let mut distinct = Vec::new();
            for alignment in [
                DWRITE_TEXT_ALIGNMENT_CENTER,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
            ] {
                let cached = self.ui_label_layout(text, 15., rect, quran_font, rtl, alignment)?;
                let translated = self.ui_label_layout(
                    text,
                    15.,
                    Rect {
                        x: 100.,
                        y: 200.,
                        ..rect
                    },
                    quran_font,
                    rtl,
                    alignment,
                )?;
                let fresh = self.text(text, 15., rect.w, rect.h, quran_font, rtl)?;
                unsafe {
                    fresh.SetTextAlignment(alignment)?;
                    if cached.as_raw() != translated.as_raw()
                        || cached.GetTextAlignment() != alignment
                        || cached.GetReadingDirection() != fresh.GetReadingDirection()
                        || distinct
                            .iter()
                            .any(|other: &IDWriteTextLayout| other.as_raw() == cached.as_raw())
                    {
                        return Err("UI caption cache reused an incompatible format".into());
                    }
                    let mut before = DWRITE_TEXT_METRICS::default();
                    let mut after = DWRITE_TEXT_METRICS::default();
                    fresh.GetMetrics(&mut before)?;
                    cached.GetMetrics(&mut after)?;
                    let geometry = |m: DWRITE_TEXT_METRICS| {
                        [
                            m.left,
                            m.top,
                            m.width,
                            m.widthIncludingTrailingWhitespace,
                            m.height,
                            m.layoutWidth,
                            m.layoutHeight,
                        ]
                        .map(f32::to_bits)
                    };
                    if geometry(before) != geometry(after)
                        || before.maxBidiReorderingDepth != after.maxBidiReorderingDepth
                        || before.lineCount != after.lineCount
                    {
                        return Err("Cached UI caption changed native text geometry".into());
                    }
                    for position in 0..text.encode_utf16().count() as u32 {
                        let mut a = DWRITE_HIT_TEST_METRICS::default();
                        let mut b = DWRITE_HIT_TEST_METRICS::default();
                        let (mut ax, mut ay, mut bx, mut by) = (0f32, 0f32, 0f32, 0f32);
                        fresh.HitTestTextPosition(position, false, &mut ax, &mut ay, &mut a)?;
                        cached.HitTestTextPosition(position, false, &mut bx, &mut by, &mut b)?;
                        if [ax, ay, a.left, a.top, a.width, a.height].map(f32::to_bits)
                            != [bx, by, b.left, b.top, b.width, b.height].map(f32::to_bits)
                            || a.textPosition != b.textPosition
                            || a.length != b.length
                            || a.bidiLevel != b.bidiLevel
                            || a.isText != b.isText
                            || a.isTrimmed != b.isTrimmed
                        {
                            return Err("Cached UI caption moved a native text cluster".into());
                        }
                        checked_positions += 1;
                    }
                }
                distinct.push(cached);
            }
            cases.push(serde_json::json!({
                "text":text,"rtl":rtl,"originalFontRole":quran_font,
                "alignmentVariants":distinct.len(),"nativeGeometryIdentical":true
            }));
        }
        let mut formats = Vec::new();
        for (size, width, height, quran_font, rtl) in [
            (15., 400., 34., false, false),
            (16., 400., 34., false, false),
            (15., 401., 34., false, false),
            (15., 400., 35., false, false),
            (15., 400., 34., true, false),
            (15., 400., 34., false, true),
        ] {
            let layout = self.ui_label_layout(
                "الكهف",
                size,
                Rect {
                    w: width,
                    h: height,
                    ..rect
                },
                quran_font,
                rtl,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
            if formats
                .iter()
                .any(|other: &IDWriteTextLayout| other.as_raw() == layout.as_raw())
            {
                return Err("UI caption cache did not isolate a native format dimension".into());
            }
            formats.push(layout);
        }
        let hot = self.ui_label_layout(
            "Recent caption",
            15.,
            rect,
            false,
            false,
            DWRITE_TEXT_ALIGNMENT_CENTER,
        )?;
        for index in 0..UI_LABEL_CACHE_CAPACITY + 1 {
            self.ui_label_layout(
                &format!("Cache bound {index}"),
                15.,
                rect,
                false,
                false,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
            let reused = self.ui_label_layout(
                "Recent caption",
                15.,
                rect,
                false,
                false,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
            if reused.as_raw() != hot.as_raw() {
                return Err("A recently used UI caption was evicted before older entries".into());
            }
        }
        let cache_len = self.ui_labels.borrow().len();
        if cache_len > UI_LABEL_CACHE_CAPACITY {
            return Err("UI caption cache exceeded its entry bound".into());
        }
        let oversized = "x".repeat(UI_LABEL_CACHE_MAX_TEXT_BYTES + 1);
        self.ui_label_layout(
            &oversized,
            15.,
            rect,
            false,
            false,
            DWRITE_TEXT_ALIGNMENT_CENTER,
        )?;
        if self
            .ui_labels
            .borrow()
            .iter()
            .any(|entry| entry.text == oversized)
        {
            return Err("UI caption cache retained an oversized diagnostic".into());
        }
        Ok(serde_json::json!({
            "passed":true,"cases":cases,"nativeHitPositionsCompared":checked_positions,
            "positionIndependentReuse":true,"recentlyUsedEntryRetained":true,
            "formatDimensionsIsolated":formats.len(),
            "cacheEntries":cache_len,"maximumEntries":UI_LABEL_CACHE_CAPACITY,
            "maximumTextBytesPerEntry":UI_LABEL_CACHE_MAX_TEXT_BYTES,
            "oversizedDiagnosticsNotRetained":true,
            "scope":"Native DirectWrite geometry and cluster identity against uncached captions; original Quran row shaping is excluded"
        }))
    }
    fn page_chrome(
        &self,
        target: &ID2D1RenderTarget,
        resources: &Resources,
        page: &Page,
        rect: Rect,
        palette: Palette,
        arabic: bool,
    ) -> Result<()> {
        let scale = rect.h / PAGE_HEIGHT;
        let inset = 30.4 * scale;
        let meta = self
            .chrome
            .as_ref()
            .and_then(|chrome| chrome.pages.get(page.page_number as usize - 1));
        let surah = meta.map(|m| m.first_surah).or_else(|| {
            page.lines
                .iter()
                .flat_map(|l| &l.words)
                .next()
                .map(|w| w.surah)
        });
        if let Some(surah) = surah.and_then(|n| resources.surahs.get(n as usize - 1)) {
            self.chrome_label(
                target,
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
                palette.muted,
                arabic,
                true,
                arabic,
            )?;
        }
        if self.show_clock {
            let now = unsafe { GetLocalTime() };
            let hour = if self.clock_24 {
                now.wHour
            } else {
                let hour = now.wHour % 12;
                if hour == 0 { 12 } else { hour }
            };
            self.label(
                target,
                &format!("{hour:02}:{:02}", now.wMinute),
                Rect {
                    x: rect.x + rect.w * 0.4,
                    y: rect.y + 4. * scale,
                    w: rect.w * 0.2,
                    h: 22. * scale,
                },
                10.24 * scale,
                palette.muted,
                false,
            )?;
        }
        if let Some(meta) = meta {
            self.chrome_label(
                target,
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
                palette.muted,
                arabic,
                false,
                false,
            )?;
        }
        self.label(
            target,
            &metadata_number(page.page_number, arabic),
            Rect {
                x: rect.x,
                y: rect.y + rect.h - 29. * scale,
                w: rect.w,
                h: 22. * scale,
            },
            if arabic { 15.04 * scale } else { 11.84 * scale },
            palette.muted,
            arabic,
        )?;
        if let Some(chrome) = &self.chrome {
            if let Some(mark) = section_mark(
                chrome.hizb.get(&page.page_number).copied(),
                chrome.rub.get(&page.page_number).copied(),
                arabic,
            ) {
                let right_page = page.page_number % 2 == 1;
                self.chrome_label(
                    target,
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
                    palette.muted,
                    arabic,
                    right_page,
                    false,
                )?;
            }
        }
        Ok(())
    }
    fn natural_ink(&self, line: &crate::data::Line, size: f32) -> Result<NaturalInk> {
        let shaped = self.natural_shaper.line(&line.text)?;
        let scale = size / shaped.units_per_em;
        let indices: Vec<_> = shaped.glyphs.iter().map(|g| g.id).collect();
        let advances: Vec<_> = shaped.glyphs.iter().map(|g| g.advance * scale).collect();
        let offsets: Vec<_> = shaped
            .glyphs
            .iter()
            .map(|g| DWRITE_GLYPH_OFFSET {
                advanceOffset: g.x * scale,
                ascenderOffset: g.y * scale,
            })
            .collect();
        let mut ranges = Vec::new();
        let mut byte = 0;
        for word in &line.words {
            ranges.push((byte, byte + word.text.len(), (word.surah, word.ayah)));
            byte += word.text.len() + 1;
        }
        let mut envelopes = vec![(f32::INFINITY, f32::NEG_INFINITY); ranges.len()];
        let mut pen = 0.;
        for (glyph, advance) in shaped.glyphs.iter().zip(&advances) {
            if let Some(index) = ranges
                .iter()
                .position(|(a, b, _)| (*a..*b).contains(&glyph.cluster))
            {
                let bounds = &mut envelopes[index];
                bounds.0 = bounds.0.min(pen);
                bounds.1 = bounds.1.max(pen + advance.max(0.));
            }
            pen += advance;
        }
        let bounds = unsafe {
            let geometry = self.factory.CreatePathGeometry()?;
            let sink = geometry.Open()?;
            self.natural_face.GetGlyphRunOutline(
                size,
                indices.as_ptr(),
                Some(advances.as_ptr()),
                Some(offsets.as_ptr()),
                indices.len() as u32,
                false,
                false,
                &sink,
            )?;
            sink.Close()?;
            let bounds = geometry.GetBounds(None)?;
            Rect {
                x: bounds.left,
                y: bounds.top,
                w: bounds.right - bounds.left,
                h: bounds.bottom - bounds.top,
            }
        };
        if ![bounds.x, bounds.y, bounds.w, bounds.h]
            .iter()
            .all(|v| v.is_finite())
            || bounds.w <= 0.
            || bounds.h <= 0.
            || envelopes
                .iter()
                .any(|(a, b)| !a.is_finite() || !b.is_finite() || b < a)
        {
            return Err("Invalid natural source glyph bounds".into());
        }
        Ok(NaturalInk {
            indices,
            advances,
            offsets,
            size,
            bounds,
            words: envelopes
                .into_iter()
                .zip(ranges)
                .map(|((a, b), (_, _, key))| (a, b, key))
                .collect(),
        })
    }
    fn page_ink(
        &mut self,
        page: &Page,
        resources: &Resources,
        w: f32,
        h: f32,
    ) -> Result<Vec<InkLine>> {
        // Shape each authentic line once in page coordinates. Direct2D
        // rasterizes its vector glyphs at the current monitor density; resize
        // never stretches an old bitmap or reshapes every line on every frame.
        if (w - PAGE_WIDTH).abs() > 0.01 || (h - PAGE_HEIGHT).abs() > 0.01 {
            let scale = w / PAGE_WIDTH;
            let mut lines = self.page_ink(page, resources, PAGE_WIDTH, PAGE_HEIGHT)?;
            for line in &mut lines {
                line.scale = scale;
                line.origin.X *= scale;
                line.origin.Y *= scale;
                line.rect.x *= scale;
                line.rect.y *= scale;
                line.rect.w *= scale;
                line.rect.h *= scale;
            }
            return Ok(lines);
        }
        if let Some(index) = self.cache.iter().position(|c| {
            c.number == page.page_number && (c.w - w).abs() < 0.01 && (c.h - h).abs() < 0.01
        }) {
            let cached = self.cache.remove(index).unwrap();
            let lines = cached.lines.clone();
            self.cache.push_back(cached);
            return Ok(lines);
        }
        let (row, content_width, pad_x, pad_top) = page_rows(w, h);
        let mut lines = Vec::new();
        // One font size across a page. Measure actual glyph ink, including marks,
        // rather than only advances, before uniformly reducing an unusually
        // wide or tall page. No Quran characters or outlines are altered.
        let mut font = FONT_SIZE * h / PAGE_HEIGHT;
        for line in &page.lines {
            if line.kind != "ayah" {
                continue;
            }
            let layout = self.text(&line.text, font, 2000., row, true, true)?;
            let bounds = ink_bounds(
                &layout,
                Rect {
                    x: 0.,
                    y: 0.,
                    w: 2000.,
                    h: row,
                },
            )?;
            if bounds.w > content_width {
                font *= content_width / bounds.w;
            }
        }
        let mut fit = 1f32;
        let mut previous: Option<(u32, f32)> = None;
        for line in &page.lines {
            if line.kind != "ayah" {
                previous = None;
                continue;
            }
            let layout = self.text(&line.text, font, content_width, row, true, true)?;
            let bounds = ink_bounds(
                &layout,
                Rect {
                    x: 0.,
                    y: -row / 2.,
                    w: content_width,
                    h: row,
                },
            )?;
            if let Some((last, lower)) = previous {
                let available = row * (line.line_number - last) as f32 - row * 0.06;
                fit = fit.min(available / (lower - bounds.y).max(1.));
            }
            previous = Some((line.line_number, bounds.y + bounds.h));
        }
        font *= fit.min(1.);
        if page.page_number <= 2 {
            let mut natural_fit = 1f32;
            for line in page.lines.iter().filter(|line| line.kind == "ayah") {
                let ink = self.natural_ink(line, font)?;
                natural_fit = natural_fit
                    .min(content_width / ink.bounds.w)
                    .min(row * 0.86 / ink.bounds.h);
            }
            font *= natural_fit.min(1.);
        }
        for line in &page.lines {
            let decoration = if line.kind == "surah_name" {
                self.original_header(line.surah_number.ok_or("Missing source header")?)
                    .ok()
            } else {
                None
            };
            let (text, mut size, quran, decoration_font) = match line.kind.as_str() {
                "surah_name" => {
                    let number = line.surah_number.ok_or("Missing surah header")?;
                    if let (Some(glyph), Some(font)) = (
                        self.header_ligatures.get(&number),
                        self.header_font.as_ref(),
                    ) {
                        (glyph.clone(), 138.4 * h / PAGE_HEIGHT, false, Some(font))
                    } else {
                        (
                            format!("سورة {}", resources.surahs[number as usize - 1].name_arabic),
                            row * 0.42,
                            false,
                            None,
                        )
                    }
                }
                "basmallah" => {
                    if let Some(font) = &self.basmala_font {
                        (
                            V4_BASMALA.to_owned(),
                            26.88 * h / PAGE_HEIGHT,
                            false,
                            Some(font),
                        )
                    } else {
                        (self.basmala.clone(), font * 0.86, true, None)
                    }
                }
                _ => (line.text.clone(), font, true, None),
            };
            let make_layout = |size| {
                if let Some(font) = decoration_font {
                    self.font_text(
                        &text,
                        size,
                        content_width,
                        row,
                        Some(&font.collection),
                        &font.family,
                        line.kind != "surah_name",
                    )
                } else {
                    self.text(&text, size, content_width, row, quran, true)
                }
            };
            let mut layout = make_layout(size)?;
            if line.kind != "ayah" {
                // Fit the original decorative glyph's ink, not its unusually
                // large em box. Keep a real clear gap around neighboring marks.
                let bounds = ink_bounds(
                    &layout,
                    Rect {
                        x: 0.,
                        y: 0.,
                        w: content_width,
                        h: row,
                    },
                )?;
                let allowed_width = if line.kind == "surah_name" {
                    content_width * 0.94
                } else {
                    content_width
                };
                size *= (allowed_width / bounds.w.max(1.))
                    .min(row * 0.78 / bounds.h.max(1.))
                    .min(1.);
                layout = make_layout(size)?;
            }
            if line.kind == "ayah" && !line.is_centered {
                let mut metrics = DWRITE_TEXT_METRICS::default();
                unsafe {
                    layout.GetMetrics(&mut metrics)?;
                }
                let spaces = line.text.encode_utf16().filter(|c| *c == 32).count();
                if spaces > 0 {
                    let spacing: IDWriteTextLayout1 = layout.cast()?;
                    let mut extra = 0.;
                    // Fit the visible source ink, not an arbitrary capped gap.
                    // Every full-width row shares the same endpoints; authentic
                    // centered rows bypass this presentation-only adjustment.
                    for _ in 0..3 {
                        let ink = ink_bounds(
                            &layout,
                            Rect {
                                x: 0.,
                                y: 0.,
                                w: content_width,
                                h: row,
                            },
                        )?;
                        let remaining = content_width - ink.w;
                        if remaining.abs() <= 0.02 {
                            break;
                        }
                        extra = (extra + remaining / spaces as f32).max(0.);
                        for (position, c) in line.text.encode_utf16().enumerate() {
                            if c == 32 {
                                unsafe {
                                    spacing.SetCharacterSpacing(
                                        0.,
                                        extra,
                                        0.,
                                        DWRITE_TEXT_RANGE {
                                            startPosition: position as u32,
                                            length: 1,
                                        },
                                    )?;
                                }
                            }
                        }
                    }
                }
            }
            let mut runs = Vec::new();
            let mut pos = 0;
            // Offsets are UTF-16, as required by DirectWrite (not UTF-8 bytes).
            for word in &line.words {
                let len = word.text.encode_utf16().count() as u32;
                runs.push(Run {
                    start: pos,
                    len,
                    surah: word.surah,
                    ayah: word.ayah,
                });
                pos += len + 1;
            }
            if !line.words.is_empty()
                && line
                    .words
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
                    != line.text
            {
                return Err(
                    format!("Source word/line mismatch on page {}", page.page_number).into(),
                );
            }
            let rect = Rect {
                x: pad_x,
                y: pad_top
                    + (line.line_number - 1) as f32 * row
                    + if page.page_number <= 2 {
                        200. * h / PAGE_HEIGHT
                    } else {
                        0.
                    },
                w: content_width,
                h: row,
            };
            let bounds = ink_bounds(
                &layout,
                Rect {
                    x: 0.,
                    y: 0.,
                    w: content_width,
                    h: row,
                },
            )?;
            let visual_shift = (content_width - bounds.w) / 2. - bounds.x;
            let vertical_shift = if line.kind == "ayah" {
                0.
            } else {
                (row - bounds.h) / 2. - bounds.y
            };
            // Start with the reported opening pages: HarfRust applies the
            // original font's Arabic joins and stacked mark attachments. The
            // rest of the natural renderer is retained pending wider fidelity QA.
            let natural = if page.page_number <= 2 && line.kind == "ayah" {
                Some(Rc::new(self.natural_ink(line, font)?))
            } else {
                None
            };
            let origin = natural
                .as_ref()
                .map(|ink| Vector2 {
                    X: rect.x + (rect.w - ink.bounds.w) / 2. - ink.bounds.x,
                    Y: rect.y + (rect.h - ink.bounds.h) / 2. - ink.bounds.y,
                })
                .unwrap_or(Vector2 {
                    X: rect.x + visual_shift,
                    Y: rect.y + vertical_shift,
                });
            lines.push(InkLine {
                layout,
                scale: 1.,
                origin,
                runs: runs.into(),
                rect,
                line_number: line.line_number,
                hits: None,
                decoration,
                natural,
            });
        }
        self.cache.push_back(PageInk {
            number: page.page_number,
            w,
            h,
            lines: lines.clone(),
        });
        while self.cache.len() > 12 {
            self.cache.pop_front();
        }
        Ok(lines)
    }
    pub fn prepare_page(
        &mut self,
        resources: &mut Resources,
        number: u32,
        weighted: bool,
    ) -> Result<()> {
        let page = resources.page(number)?;
        self.page_ink(&page, resources, PAGE_WIDTH, PAGE_HEIGHT)?;
        if self.contextual {
            if self.outlines.is_none() {
                self.outlines = Some(crate::outlines::Outlines::new(
                    resources.root.clone(),
                    self.factory.clone(),
                )?);
            }
            let outlines = self.outlines.as_mut().unwrap();
            let prepared = outlines.validated_page(&page)?;
            if weighted {
                outlines.prepare_strokes(&prepared)?;
            }
        }
        Ok(())
    }
    /// A geometric corner fold over authentic source pages. Final frames always
    /// use the normal vector renderer, so animation never leaves scaled text.
    #[allow(clippy::too_many_arguments)]
    pub fn fold(
        &mut self,
        target: &ID2D1RenderTarget,
        resources: &mut Resources,
        view: BookView,
        previous: u32,
        direction: i32,
        progress: f32,
        top: bool,
    ) -> Result<()> {
        use windows_numerics::Matrix3x2;
        // The elapsed-time ease is already at rest over its final fraction.
        // Use the exact destination surface there: a clamped crease still
        // rotated the reverse face and changed its grayscale text rasterization
        // on the last frame, making the landed page appear to load again.
        if progress >= 0.995 {
            return self.book(target, resources, view);
        }
        let area = view.area;
        let palette = view.palette;
        let arabic = view.arabic;
        let destination = view.page;
        let leaf = crate::page_turn::leaf_bounds(area, direction);
        #[cfg(feature = "qa")]
        let clip_base = !std::env::args().any(|a| a == "--qa-unclipped-base");
        #[cfg(not(feature = "qa"))]
        let clip_base = true;
        // The static old peer immediately covers the destination peer. Avoid
        // rasterizing that occluded page while keeping the full destination
        // interaction map and original vector/font rendering intact.
        unsafe {
            if clip_base {
                target.PushAxisAlignedClip(&leaf.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            }
        }
        let base = self.book(target, resources, view);
        unsafe {
            if clip_base {
                target.PopAxisAlignedClip();
            }
        }
        base?;
        let current_hits = std::mem::take(&mut self.visible);
        let bounds = crate::page_turn::bounds(area);
        let single = crate::app_core::book_geometry(area).single;
        let peer = if single {
            leaf
        } else {
            Rect {
                x: if direction > 0 {
                    leaf.x + leaf.w
                } else {
                    leaf.x - leaf.w
                },
                ..leaf
            }
        };
        let crate::page_turn::Paper {
            retained,
            reverse,
            back_transform,
        } = crate::page_turn::paper(leaf, direction, progress, top);
        unsafe {
            if !single {
                target.PushAxisAlignedClip(&peer.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                let result = self.book_faces_region(
                    target,
                    resources,
                    BookView {
                        page: previous,
                        area,
                        palette,
                        selection: None,
                        recitation: None,
                        opening: None,
                        arabic,
                    },
                    false,
                    Some(peer),
                );
                target.PopAxisAlignedClip();
                result?;
            }
            if retained.len() >= 3 {
                let mask = self.polygon(&retained)?;
                let mut parameters = D2D1_LAYER_PARAMETERS {
                    contentBounds: bounds.d2d(),
                    geometricMask: std::mem::ManuallyDrop::new(Some(mask.cast()?)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: Matrix3x2::identity(),
                    opacity: 1.,
                    ..Default::default()
                };
                target.PushLayer(&parameters, None);
                std::mem::ManuallyDrop::drop(&mut parameters.geometricMask);
                let result = self.book_faces_region(
                    target,
                    resources,
                    BookView {
                        page: previous,
                        area,
                        palette,
                        selection: None,
                        recitation: None,
                        opening: None,
                        arabic,
                    },
                    false,
                    Some(leaf),
                );
                target.PopLayer();
                result?;
            }
            if reverse.len() >= 3 {
                // Both faces use the same original font/outlines, transformed
                // as vectors. No screenshots, bitmap stretching or generated
                // text are needed to paint the back of the turning paper.
                let geometry = self.polygon(&reverse)?;
                let mut shade = palette.border;
                shade.a = 0.22 * (std::f32::consts::PI * progress).sin();
                let shadow = target.CreateSolidColorBrush(&shade, None)?;
                target.PushAxisAlignedClip(&bounds.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                target.DrawGeometry(&geometry, &shadow, 16., None);
                let paper = target.CreateSolidColorBrush(&palette.paper, None)?;
                target.FillGeometry(&geometry, &paper, None);
                let mut parameters = D2D1_LAYER_PARAMETERS {
                    contentBounds: bounds.d2d(),
                    geometricMask: std::mem::ManuallyDrop::new(Some(geometry.cast()?)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: Matrix3x2::identity(),
                    opacity: 1.,
                    ..Default::default()
                };
                target.PushLayer(&parameters, None);
                std::mem::ManuallyDrop::drop(&mut parameters.geometricMask);
                let antialias = target.GetTextAntialiasMode();
                target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                let translate = Matrix3x2 {
                    M31: if single {
                        0.
                    } else if direction > 0 {
                        -leaf.w
                    } else {
                        leaf.w
                    },
                    ..Matrix3x2::identity()
                };
                target.SetTransform(&(translate * back_transform));
                let result = self.book_faces_region(
                    target,
                    resources,
                    BookView {
                        page: destination,
                        area,
                        palette,
                        selection: None,
                        recitation: None,
                        opening: None,
                        arabic,
                    },
                    false,
                    Some(peer),
                );
                target.SetTransform(&Matrix3x2::identity());
                target.SetTextAntialiasMode(antialias);
                target.PopLayer();
                target.PopAxisAlignedClip();
                result?;
                target.PushAxisAlignedClip(&bounds.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                target.DrawGeometry(&geometry, &shadow, 1., None);
                target.PopAxisAlignedClip();
            }
        }
        self.visible = current_hits;
        Ok(())
    }
    /// Compose the same one-leaf fold from prepared original page pixels.
    /// The static/front faces retain ClearType; a separate grayscale reverse
    /// face prevents monitor-subpixel colors from rotating with the paper.
    /// Callers own preparation, device-generation keys and idle warmup.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_cached_fold(
        &mut self,
        target: &ID2D1RenderTarget,
        old_bitmap: &ID2D1Bitmap,
        new_bitmap: &ID2D1Bitmap,
        old_raster: &BookRaster,
        new_raster: &BookRaster,
        back: Option<(&ID2D1Bitmap, &BookRaster)>,
        view: BookView,
        direction: i32,
        progress: f32,
        top: bool,
    ) -> Result<()> {
        if !progress.is_finite() || !matches!(direction, -1 | 1) {
            return Err("Invalid cached fold state".into());
        }
        let bounds = crate::page_turn::bounds(view.area);
        let mut faces = vec![(old_bitmap, old_raster), (new_bitmap, new_raster)];
        if let Some(face) = back {
            faces.push(face);
        }
        let mut pixel_bytes = 0u64;
        for &(bitmap, raster) in &faces {
            raster.validate_bitmap(target, bitmap)?;
            if (raster.bounds.x - bounds.x).abs() > 0.02
                || (raster.bounds.y - bounds.y).abs() > 0.02
                || (raster.bounds.w - bounds.w).abs() > 0.02
                || (raster.bounds.h - bounds.h).abs() > 0.02
                || raster.contextual != new_raster.contextual
            {
                return Err("Cached fold faces have different original geometry".into());
            }
            // Count the caller-retained WIC pixels and uploaded pixels. This
            // bounds logical texture storage, not driver padding or process RAM.
            pixel_bytes +=
                u64::from(raster.pixel_size.width) * u64::from(raster.pixel_size.height) * 8;
        }
        if pixel_bytes > 64 * 1024 * 1024 {
            return Err("Cached fold exceeds its 64 MiB pixel-storage budget".into());
        }
        if old_raster.grayscale || new_raster.grayscale {
            return Err("Stationary fold faces require the original text mode".into());
        }
        if progress >= 0.995 {
            return self.draw_cached_book(target, new_bitmap, new_raster, Vector2::default());
        }
        let (back_bitmap, back_raster) =
            back.ok_or("Cached fold requires a grayscale reverse face")?;
        if !back_raster.grayscale {
            return Err("Rotated fold text requires grayscale antialiasing".into());
        }
        let leaf = crate::page_turn::leaf_bounds(view.area, direction);
        let single = crate::app_core::book_geometry(view.area).single;
        let peer = if single {
            leaf
        } else {
            Rect {
                x: if direction > 0 {
                    leaf.x + leaf.w
                } else {
                    leaf.x - leaf.w
                },
                ..leaf
            }
        };
        let crate::page_turn::Paper {
            retained,
            reverse,
            back_transform,
        } = crate::page_turn::paper(leaf, direction, progress, top);
        // Prepare fallible resources before pushing clips/layers; bitmap draw
        // commands themselves have no immediate failure result to unwind.
        let retained_geometry: Option<ID2D1Geometry> = if retained.len() >= 3 {
            Some(self.polygon(&retained)?.cast()?)
        } else {
            None
        };
        let reverse_geometry: Option<ID2D1Geometry> = if reverse.len() >= 3 {
            Some(self.polygon(&reverse)?.cast()?)
        } else {
            None
        };
        unsafe {
            use windows_numerics::Matrix3x2;
            let mut previous_transform = Matrix3x2::identity();
            target.GetTransform(&mut previous_transform);
            let mut shade = view.palette.border;
            shade.a = 0.22 * (std::f32::consts::PI * progress).sin();
            let shadow = target.CreateSolidColorBrush(&shade, None)?;
            let paper = target.CreateSolidColorBrush(&view.palette.paper, None)?;
            target.SetTransform(&Matrix3x2::identity());
            target.PushAxisAlignedClip(&leaf.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            new_raster.paint_pixels(target, new_bitmap, false);
            target.PopAxisAlignedClip();
            if !single {
                target.PushAxisAlignedClip(&peer.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                old_raster.paint_pixels(target, old_bitmap, false);
                target.PopAxisAlignedClip();
            }
            if let Some(mask) = retained_geometry {
                let mut parameters = D2D1_LAYER_PARAMETERS {
                    contentBounds: bounds.d2d(),
                    geometricMask: std::mem::ManuallyDrop::new(Some(mask.clone())),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: Matrix3x2::identity(),
                    opacity: 1.,
                    ..Default::default()
                };
                target.PushLayer(&parameters, None);
                std::mem::ManuallyDrop::drop(&mut parameters.geometricMask);
                old_raster.paint_pixels(target, old_bitmap, false);
                target.PopLayer();
            }
            if let Some(mask) = reverse_geometry {
                target.PushAxisAlignedClip(&bounds.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                target.DrawGeometry(&mask, &shadow, 16., None);
                target.FillGeometry(&mask, &paper, None);
                let mut parameters = D2D1_LAYER_PARAMETERS {
                    contentBounds: bounds.d2d(),
                    geometricMask: std::mem::ManuallyDrop::new(Some(mask.clone())),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: Matrix3x2::identity(),
                    opacity: 1.,
                    ..Default::default()
                };
                target.PushLayer(&parameters, None);
                std::mem::ManuallyDrop::drop(&mut parameters.geometricMask);
                let translate = Matrix3x2 {
                    M31: if single {
                        0.
                    } else if direction > 0 {
                        -leaf.w
                    } else {
                        leaf.w
                    },
                    ..Matrix3x2::identity()
                };
                target.SetTransform(&(translate * back_transform));
                back_raster.paint_pixels(target, back_bitmap, true);
                target.SetTransform(&Matrix3x2::identity());
                target.PopLayer();
                target.PopAxisAlignedClip();
                target.PushAxisAlignedClip(&bounds.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                target.DrawGeometry(&mask, &shadow, 1., None);
                target.PopAxisAlignedClip();
            }
            target.SetTransform(&previous_transform);
        }
        self.visible.clone_from(&new_raster.visible);
        Ok(())
    }
    fn polygon(&self, points: &[(f32, f32)]) -> Result<ID2D1PathGeometry> {
        unsafe {
            let geometry = self.factory.CreatePathGeometry()?;
            let sink = geometry.Open()?;
            sink.BeginFigure(
                Vector2 {
                    X: points[0].0,
                    Y: points[0].1,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            for &(x, y) in &points[1..] {
                sink.AddLine(Vector2 { X: x, Y: y });
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            Ok(geometry)
        }
    }
    pub fn book(
        &mut self,
        target: &ID2D1RenderTarget,
        resources: &mut Resources,
        view: BookView,
    ) -> Result<()> {
        self.book_faces(target, resources, view, true)
    }
    /// Rasterize only the book, keeping its existing subpixel phase and the
    /// same source-derived interaction coordinates as the normal renderer.
    /// This prepares content; the HWND presentation target never rasterizes
    /// contextual curves when it subsequently draws the uploaded bitmap.
    pub fn raster_book(
        &mut self,
        resources: &mut Resources,
        view: BookView,
        dpi: f32,
    ) -> Result<BookRaster> {
        self.raster_book_with_aa(resources, view, dpi, false)
    }
    /// A rotated paper face uses a grayscale raster. ClearType's subpixel
    /// colors belong to the stationary monitor axes and must not be rotated.
    pub fn raster_book_with_aa(
        &mut self,
        resources: &mut Resources,
        view: BookView,
        dpi: f32,
        grayscale: bool,
    ) -> Result<BookRaster> {
        if !dpi.is_finite() || !(48. ..=768.).contains(&dpi) {
            return Err("Invalid book raster density".into());
        }
        let bounds = crate::app_core::book_geometry(view.area).bounds;
        if ![bounds.x, bounds.y, bounds.w, bounds.h]
            .iter()
            .all(|number| number.is_finite())
            || bounds.w <= 0.
            || bounds.h <= 0.
        {
            return Err("Invalid book raster bounds".into());
        }
        let density = dpi / 96.;
        // Include the antialiased frame edge. Align the *origin* rather than
        // moving glyphs within the crop, preserving the HWND's pixel phase.
        let left = ((bounds.x - 2.) * density).floor();
        let top = ((bounds.y - 2.) * density).floor();
        let right = ((bounds.x + bounds.w + 2.) * density).ceil();
        let bottom = ((bounds.y + bounds.h + 2.) * density).ceil();
        let width = right - left;
        let height = bottom - top;
        if !width.is_finite()
            || !height.is_finite()
            || !(1. ..=16384.).contains(&width)
            || !(1. ..=16384.).contains(&height)
        {
            return Err("Book raster dimensions exceed their limits".into());
        }
        let pixel_size = D2D_SIZE_U {
            width: width as u32,
            height: height as u32,
        };
        if u64::from(pixel_size.width) * u64::from(pixel_size.height) * 4 > 32 * 1024 * 1024 {
            return Err("Book raster exceeds its 32 MiB allocation limit".into());
        }
        let source = Rect {
            x: left / density,
            y: top / density,
            w: width / density,
            h: height / density,
        };
        let previous_visible = std::mem::take(&mut self.visible);
        let result = (|| -> Result<BookRaster> {
            unsafe {
                let wic: IWICImagingFactory =
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
                // Stationary faces retain opaque BGR/ClearType. The grayscale
                // reverse face has a transparent exterior, so its rounded
                // corners leave the underlying paper polygon intact.
                let bitmap = wic.CreateBitmap(
                    pixel_size.width,
                    pixel_size.height,
                    if grayscale {
                        &GUID_WICPixelFormat32bppPBGRA
                    } else {
                        &GUID_WICPixelFormat32bppBGR
                    },
                    WICBitmapCacheOnLoad,
                )?;
                let target = self.factory.CreateWicBitmapRenderTarget(
                    &bitmap,
                    &D2D1_RENDER_TARGET_PROPERTIES {
                        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: if grayscale {
                                D2D1_ALPHA_MODE_PREMULTIPLIED
                            } else {
                                D2D1_ALPHA_MODE_IGNORE
                            },
                        },
                        dpiX: dpi,
                        dpiY: dpi,
                        ..Default::default()
                    },
                )?;
                target.SetTextAntialiasMode(if grayscale {
                    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE
                } else {
                    D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE
                });
                target.SetTransform(&windows_numerics::Matrix3x2 {
                    M31: -source.x,
                    M32: -source.y,
                    ..windows_numerics::Matrix3x2::identity()
                });
                target.BeginDraw();
                target.Clear(Some(&if grayscale {
                    D2D1_COLOR_F::default()
                } else {
                    view.palette.background
                }));
                let rendering = self.book(&target, resources, view);
                // Always close a begun drawing session, including a source or
                // optional-outline error. Failed preparation remains a normal
                // vector-rendering fallback at the call site.
                let flushing = target.EndDraw(None, None);
                rendering?;
                flushing?;
                Ok(BookRaster {
                    wic: bitmap,
                    visible: std::mem::take(&mut self.visible),
                    source,
                    bounds,
                    dpi,
                    pixel_size,
                    contextual: self.contextual,
                    grayscale,
                })
            }
        })();
        self.visible = previous_visible;
        result
    }
    /// Compose a prepared book at its original size. Translation is snapped
    /// to physical pixels; the original hit map follows that exact offset.
    /// Callers invalidate their cache for resize, DPI, palette, source-mode,
    /// selection or presentation-device changes instead of scaling old text.
    pub fn draw_cached_book(
        &mut self,
        target: &ID2D1RenderTarget,
        bitmap: &ID2D1Bitmap,
        raster: &BookRaster,
        offset: Vector2,
    ) -> Result<()> {
        if !offset.X.is_finite() || !offset.Y.is_finite() {
            return Err("Invalid book raster translation".into());
        }
        unsafe {
            let mut dpi_x = 0.;
            let mut dpi_y = 0.;
            target.GetDpi(&mut dpi_x, &mut dpi_y);
            let pixels = bitmap.GetPixelSize();
            if (dpi_x - raster.dpi).abs() > 0.01
                || (dpi_y - raster.dpi).abs() > 0.01
                || pixels.width != raster.pixel_size.width
                || pixels.height != raster.pixel_size.height
            {
                return Err("Book raster and presentation dimensions differ".into());
            }
            let density = raster.dpi / 96.;
            let dx = (offset.X * density).round() / density;
            let dy = (offset.Y * density).round() / density;
            let destination = Rect {
                x: raster.source.x + dx,
                y: raster.source.y + dy,
                ..raster.source
            };
            target.DrawBitmap(
                bitmap,
                Some(&destination.d2d()),
                1.,
                D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
            );
            self.visible.clone_from(&raster.visible);
            for line in &mut self.visible {
                line.origin.X += dx;
                line.origin.Y += dy;
                line.rect.x += dx;
                line.rect.y += dy;
                if let Some(regions) = &mut line.hits {
                    for (rect, _) in regions {
                        rect.x += dx;
                        rect.y += dy;
                    }
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn paint_ayah_highlights(
        &mut self,
        target: &ID2D1RenderTarget,
        line: &InkLine,
        selection: Option<(u32, u32)>,
        recitation: Option<(u32, u32)>,
        filled: &ID2D1SolidColorBrush,
        outlined: &ID2D1SolidColorBrush,
        _interactive: bool,
    ) -> Result<()> {
        for (key, style) in highlight_targets(selection, recitation)
            .into_iter()
            .flatten()
        {
            for rect in ayah_range_bounds(line, key)? {
                let shape = D2D1_ROUNDED_RECT {
                    rect: rect.d2d(),
                    radiusX: 4.,
                    radiusY: 4.,
                };
                unsafe {
                    match style {
                        HighlightStyle::Filled => target.FillRoundedRectangle(&shape, filled),
                        HighlightStyle::Outlined => {
                            target.DrawRoundedRectangle(&shape, outlined, 1.25, None);
                        }
                    }
                }
                #[cfg(feature = "qa")]
                if _interactive {
                    self.ayah_highlights
                        .push(HighlightRegion { key, style, rect });
                }
            }
        }
        Ok(())
    }

    fn book_faces(
        &mut self,
        target: &ID2D1RenderTarget,
        resources: &mut Resources,
        view: BookView,
        interactive: bool,
    ) -> Result<()> {
        self.book_faces_region(target, resources, view, interactive, None)
    }
    fn book_faces_region(
        &mut self,
        target: &ID2D1RenderTarget,
        resources: &mut Resources,
        view: BookView,
        interactive: bool,
        region: Option<Rect>,
    ) -> Result<()> {
        let BookView {
            page,
            area,
            palette,
            selection,
            recitation,
            opening,
            arabic,
        } = view;
        unsafe {
            let mut parent = windows_numerics::Matrix3x2::identity();
            target.GetTransform(&mut parent);
            if interactive {
                self.visible.clear();
                #[cfg(feature = "qa")]
                self.opening_highlights.clear();
                #[cfg(feature = "qa")]
                self.ayah_highlights.clear();
            }
            let geometry = crate::app_core::book_geometry(area);
            let single = geometry.single;
            let w = geometry.page_width;
            let h = geometry.bounds.h;
            let left = geometry.bounds.x;
            let top = geometry.bounds.y;
            let paper = target.CreateSolidColorBrush(&palette.paper, None)?;
            let border = target.CreateSolidColorBrush(&palette.border, None)?;
            let frame = D2D1_ROUNDED_RECT {
                rect: geometry.bounds.d2d(),
                radiusX: 12.,
                radiusY: 12.,
            };
            target.FillRoundedRectangle(&frame, &paper);
            target.DrawRoundedRectangle(&frame, &border, 1., None);
            if !single {
                target.DrawLine(
                    Vector2 {
                        X: left + w,
                        Y: top + 6.,
                    },
                    Vector2 {
                        X: left + w,
                        Y: top + h - 6.,
                    },
                    &border,
                    1.,
                    None,
                );
            }
            let (right_page, left_page) = crate::data::spread(page);
            let pages = if single {
                [(Some(page), left), (None, left)]
            } else {
                [(Some(right_page), left + w), (left_page, left)]
            };
            for (number, x) in pages {
                let Some(number) = number else {
                    continue;
                };
                let rect = Rect { x, y: top, w, h };
                if region
                    .is_some_and(|r| rect.x >= r.x + r.w - 0.01 || rect.x + rect.w <= r.x + 0.01)
                {
                    continue;
                }
                let page = resources.page(number)?;
                let lines = self.page_ink(&page, resources, w, h)?;
                let shaped = if self.contextual {
                    let optional = (|| -> Result<_> {
                        if self.outlines.is_none() {
                            self.outlines = Some(crate::outlines::Outlines::new(
                                resources.root.clone(),
                                self.factory.clone(),
                            )?);
                        }
                        let shaped = self.outlines.as_mut().unwrap().validated_page(&page)?;
                        Ok(shaped)
                    })();
                    match optional {
                        Ok(page) => Some(page),
                        Err(_) => {
                            // Optional presentation data must never make the
                            // authentic source page unreadable. Keep the font
                            // layout and notify the UI to explain the fallback.
                            self.contextual = false;
                            self.contextual_unavailable = true;
                            self.outlines = None;
                            None
                        }
                    }
                } else {
                    None
                };
                let (row, content_width, _, _) = page_rows(w, h);
                let outline_fit = shaped
                    .as_ref()
                    .map(|p| crate::outlines::Outlines::page_fit(p, content_width, row))
                    .unwrap_or(1.);
                let ink = target.CreateSolidColorBrush(&palette.ink, None)?;
                let highlight = target.CreateSolidColorBrush(&palette.highlight, None)?;
                let recitation_brush = target.CreateSolidColorBrush(&palette.muted, None)?;
                for mut line in lines {
                    line.origin.X += x;
                    line.origin.Y += top;
                    line.rect.x += x;
                    line.rect.y += top;
                    if let Some(art) = &line.decoration {
                        let factor = (line.rect.w * 0.94 / art.bounds.w)
                            .min(line.rect.h * 0.78 / art.bounds.h);
                        target.SetTransform(
                            &(windows_numerics::Matrix3x2 {
                                M11: factor,
                                M22: factor,
                                M31: line.rect.x + (line.rect.w - art.bounds.w * factor) / 2.
                                    - art.bounds.x * factor,
                                M32: line.rect.y + (line.rect.h - art.bounds.h * factor) / 2.
                                    - art.bounds.y * factor,
                                ..windows_numerics::Matrix3x2::identity()
                            } * parent),
                        );
                        for layer in &art.layers {
                            let t = layer.lightness;
                            let color = D2D1_COLOR_F {
                                r: palette.ink.r * (1. - t) + palette.paper.r * t,
                                g: palette.ink.g * (1. - t) + palette.paper.g * t,
                                b: palette.ink.b * (1. - t) + palette.paper.b * t,
                                a: 1.,
                            };
                            ink.SetColor(&color);
                            target.FillGeometry(&layer.geometry, &ink, None);
                        }
                        ink.SetColor(&palette.ink);
                        target.SetTransform(&parent);
                        if interactive {
                            self.visible.push(line);
                        }
                        continue;
                    }
                    let source_line = page
                        .lines
                        .iter()
                        .find(|l| l.line_number == line.line_number)
                        .ok_or("Missing source line")?;
                    if source_line.kind == "basmallah"
                        && opening.is_some_and(|surah| {
                            page.lines
                                .iter()
                                .take_while(|source| source.line_number < source_line.line_number)
                                .filter_map(|source| source.surah_number)
                                .last()
                                == Some(surah)
                        })
                    {
                        let bounds = ink_bounds(
                            &line.layout,
                            Rect {
                                x: 0.,
                                y: 0.,
                                // Cached layouts retain canonical source-page
                                // coordinates while line.rect has already been
                                // scaled to the viewport. Measure that layout
                                // box here, then scale its ink exactly once.
                                w: line.layout.GetMaxWidth(),
                                h: line.layout.GetMaxHeight(),
                            },
                        )?;
                        let rect = Rect {
                            x: line.origin.X + bounds.x * line.scale - 5.,
                            y: line.origin.Y + bounds.y * line.scale - 3.,
                            w: bounds.w * line.scale + 10.,
                            h: bounds.h * line.scale + 6.,
                        };
                        target.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: rect.d2d(),
                                radiusX: 4.,
                                radiusY: 4.,
                            },
                            &highlight,
                        );
                        #[cfg(feature = "qa")]
                        if interactive {
                            self.opening_highlights.push(rect);
                        }
                    }
                    if let Some(shape) = shaped
                        .as_ref()
                        .and_then(|p| p.lines.get(&source_line.line_number))
                    {
                        let mut outline_rect = line.rect;
                        outline_rect.x += outline_rect.w * (1. - outline_fit) / 2.;
                        outline_rect.w *= outline_fit;
                        // Both paths use the original source word map. Paint
                        // the range first so neither style redraws the Quran.
                        line.hits = if interactive || selection.is_some() || recitation.is_some() {
                            Some(crate::outlines::Outlines::hit_regions(
                                shape,
                                source_line,
                                outline_rect,
                            )?)
                        } else {
                            None
                        };
                        self.paint_ayah_highlights(
                            target,
                            &line,
                            selection,
                            recitation,
                            &highlight,
                            &recitation_brush,
                            interactive,
                        )?;
                        self.outlines.as_mut().unwrap().paint(
                            target,
                            shape,
                            crate::outlines::OutlineView {
                                rect: outline_rect,
                                palette,
                            },
                            &ink,
                        )?;
                        if interactive {
                            self.visible.push(line);
                        }
                        continue;
                    }
                    if let Some(natural) = &line.natural {
                        let hits: Vec<_> = natural
                            .words
                            .iter()
                            .map(|(left, right, key)| {
                                (
                                    Rect {
                                        x: line.origin.X + left * line.scale,
                                        y: line.rect.y,
                                        w: (right - left) * line.scale,
                                        h: line.rect.h,
                                    },
                                    *key,
                                )
                            })
                            .collect();
                        line.hits = Some(hits);
                        self.paint_ayah_highlights(
                            target,
                            &line,
                            selection,
                            recitation,
                            &highlight,
                            &recitation_brush,
                            interactive,
                        )?;
                        let mut run = DWRITE_GLYPH_RUN {
                            fontFace: std::mem::ManuallyDrop::new(Some(self.natural_face.clone())),
                            fontEmSize: natural.size,
                            glyphCount: natural.indices.len() as u32,
                            glyphIndices: natural.indices.as_ptr(),
                            glyphAdvances: natural.advances.as_ptr(),
                            glyphOffsets: natural.offsets.as_ptr(),
                            ..Default::default()
                        };
                        let previous = target.GetTextRenderingParams().ok();
                        #[cfg(feature = "qa")]
                        let tuned = !std::env::args().any(|arg| arg == "--qa-original-contrast");
                        #[cfg(not(feature = "qa"))]
                        let tuned = true;
                        if palette.paper.r > 0.8 && tuned {
                            target.SetTextRenderingParams(&self.opening_light_params);
                        }
                        // Canonical glyph advances, offsets and em size share the
                        // same transform as their origin and interactive regions.
                        target.SetTransform(
                            &(windows_numerics::Matrix3x2 {
                                M11: line.scale,
                                M22: line.scale,
                                M31: line.origin.X,
                                M32: line.origin.Y,
                                ..windows_numerics::Matrix3x2::identity()
                            } * parent),
                        );
                        target.DrawGlyphRun(
                            Vector2 { X: 0., Y: 0. },
                            &run,
                            &ink,
                            DWRITE_MEASURING_MODE_NATURAL,
                        );
                        target.SetTransform(&parent);
                        target.SetTextRenderingParams(previous.as_ref());
                        std::mem::ManuallyDrop::drop(&mut run.fontFace);
                        if interactive {
                            self.visible.push(line);
                        }
                        continue;
                    }
                    self.paint_ayah_highlights(
                        target,
                        &line,
                        selection,
                        recitation,
                        &highlight,
                        &recitation_brush,
                        interactive,
                    )?;
                    target.SetTransform(
                        &(windows_numerics::Matrix3x2 {
                            M11: line.scale,
                            M22: line.scale,
                            M31: line.origin.X,
                            M32: line.origin.Y,
                            ..windows_numerics::Matrix3x2::identity()
                        } * parent),
                    );
                    target.DrawTextLayout(
                        Vector2 { X: 0., Y: 0. },
                        &line.layout,
                        &ink,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                    target.SetTransform(&parent);
                    if interactive {
                        self.visible.push(line);
                    }
                }
                self.page_chrome(target, resources, &page, rect, palette, arabic)?;
            }
            Ok(())
        }
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<(u32, u32)> {
        unsafe {
            let line = self.visible.iter().find(|line| line.rect.contains(x, y))?;
            if let Some(regions) = &line.hits {
                return regions
                    .iter()
                    .find(|(rect, _)| rect.contains(x, y))
                    .map(|(_, key)| *key);
            }
            let mut trailing = windows::core::BOOL::default();
            let mut inside = windows::core::BOOL::default();
            let mut metrics = DWRITE_HIT_TEST_METRICS::default();
            line.layout
                .HitTestPoint(
                    (x - line.origin.X) / line.scale,
                    (y - line.origin.Y) / line.scale,
                    &mut trailing,
                    &mut inside,
                    &mut metrics,
                )
                .ok()?;
            if !inside.as_bool() {
                return None;
            }
            let run = line.runs.iter().find(|r| {
                metrics.textPosition >= r.start && metrics.textPosition < r.start + r.len + 1
            })?;
            Some((run.surah, run.ayah))
        }
    }
    #[cfg(feature = "qa")]
    pub fn verify_layout(&mut self, resources: &mut Resources) -> Result<serde_json::Value> {
        let ui_label_cache = self.verify_ui_label_cache()?;
        if !self.decorative_load_errors.is_empty() {
            return Err(format!(
                "Original decorative font loading failed:\n{}",
                self.decorative_load_errors.join("\n")
            )
            .into());
        }
        let header = self
            .header_font
            .as_ref()
            .ok_or("Original ornate surah-header font is unavailable")?;
        if self.header_ligatures.len() != 114 {
            return Err("Original surah-header mapping is incomplete".into());
        }
        for glyph in self.header_ligatures.values() {
            header.verify_glyphs(glyph)?;
        }
        let header_container = header.source_container;
        let basmala = self
            .basmala_font
            .as_ref()
            .ok_or("Original v4 basmala font is unavailable")?;
        basmala.verify_glyphs(V4_BASMALA)?;
        basmala.verify_v4_source_mapping()?;
        let basmala_container = basmala.source_container;
        if self
            .chrome
            .as_ref()
            .is_none_or(|chrome| chrome.pages.len() != 604)
        {
            return Err("Original page metadata is unavailable".into());
        }
        let mut minimum_clearance = f32::MAX;
        let mut closest = (0, 0, 0);
        let mut checked_lines = 0;
        let mut checked_words = 0;
        let mut natural_pairs = 0;
        let mut minimum_outline_fit = 1f32;
        let mut minimum_outline_clearance = f32::MAX;
        let mut outlined_words = 0;
        let mut source_characters = std::collections::BTreeSet::new();
        let mut full_width_rows = 0;
        let mut maximum_endpoint_error = 0f32;
        let mut outlines =
            crate::outlines::Outlines::new(resources.root.clone(), self.factory.clone())?;
        for n in 1..=604 {
            let page = resources.page(n)?;
            for word in page.lines.iter().flat_map(|line| &line.words) {
                source_characters.extend(word.text.chars().map(|c| c as u32));
            }
            let w = PAGE_WIDTH;
            let h = PAGE_HEIGHT;
            let (row, content_width, pad_x, pad_top) = page_rows(w, h);
            let lines = self.page_ink(&page, resources, w, h)?;
            let mut previous: Option<(u32, Rect)> = None;
            for line in lines {
                let mut metrics = DWRITE_TEXT_METRICS::default();
                unsafe {
                    line.layout.GetMetrics(&mut metrics)?;
                }
                checked_lines += 1;
                checked_words += line.runs.len();
                if metrics.width > line.rect.w + 1. {
                    return Err(format!(
                        "Native line exceeds page width: {n}/{}",
                        line.line_number
                    )
                    .into());
                }
                let bounds = if let Some(ink) = &line.natural {
                    Rect {
                        x: line.origin.X + ink.bounds.x,
                        y: line.origin.Y + ink.bounds.y,
                        ..ink.bounds
                    }
                } else {
                    ink_bounds(
                        &line.layout,
                        Rect {
                            x: line.origin.X,
                            y: line.origin.Y,
                            ..line.rect
                        },
                    )?
                };
                let source_line = page
                    .lines
                    .iter()
                    .find(|source| source.line_number == line.line_number)
                    .ok_or("Missing source row")?;
                if source_line.kind == "ayah"
                    && !source_line.is_centered
                    && source_line.text.contains(' ')
                {
                    full_width_rows += 1;
                    let error = (bounds.x - line.rect.x)
                        .abs()
                        .max((bounds.x + bounds.w - line.rect.x - line.rect.w).abs());
                    maximum_endpoint_error = maximum_endpoint_error.max(error);
                    if error > 1. {
                        return Err(format!(
                            "Mushaf row endpoints differ: {n}/{} ({error:.3} DIP)",
                            line.line_number
                        )
                        .into());
                    }
                }
                if ![bounds.x, bounds.y, bounds.w, bounds.h]
                    .iter()
                    .all(|n| n.is_finite())
                    || bounds.x < line.rect.x - 1.
                    || bounds.x + bounds.w > line.rect.x + line.rect.w + 1.
                    || bounds.y < 0.
                    || bounds.y + bounds.h > h
                {
                    return Err(format!(
                        "Native glyph ink exceeds page: {n}/{} ink={bounds:?} row={:?}",
                        line.line_number, line.rect
                    )
                    .into());
                }
                if let Some((last, lower)) = previous {
                    let gap = bounds.y - lower.y - lower.h;
                    natural_pairs += 1;
                    if gap < minimum_clearance {
                        minimum_clearance = gap;
                        closest = (n, last, line.line_number);
                    }
                    // Bounding boxes conservatively include blank space. The
                    // safety margin still guarantees separated glyph envelopes.
                    if gap < -0.02 {
                        return Err(format!(
                            "Native glyph envelopes overlap: {n}/{last}-{} ({gap:.3} DIP)",
                            line.line_number
                        )
                        .into());
                    }
                }
                previous = Some((line.line_number, bounds));
            }
            let shaped = outlines.page(n)?;
            let fit = crate::outlines::Outlines::page_fit(&shaped, content_width, row);
            minimum_outline_fit = minimum_outline_fit.min(fit);
            minimum_outline_clearance = minimum_outline_clearance.min(
                crate::outlines::Outlines::clearance(&shaped, content_width * fit, row),
            );
            for source in &page.lines {
                if let Some(shape) = shaped.lines.get(&source.line_number) {
                    let rect = Rect {
                        x: pad_x + content_width * (1. - fit) / 2.,
                        y: pad_top + (source.line_number - 1) as f32 * row,
                        w: content_width * fit,
                        h: row,
                    };
                    let hits = crate::outlines::Outlines::hit_regions(shape, source, rect)?;
                    outlined_words += hits.len();
                    if hits
                        .iter()
                        .any(|(r, _)| r.x < rect.x - 0.02 || r.x + r.w > rect.x + rect.w + 0.02)
                    {
                        return Err(format!(
                            "Source outline hit area exceeds line: {n}/{}",
                            source.line_number
                        )
                        .into());
                    }
                    if let Some(ink) = crate::outlines::Outlines::ink_envelope(shape, rect) {
                        if ink.x < rect.x - 0.02
                            || ink.x + ink.w > rect.x + rect.w + 0.02
                            || ink.y < 0.
                            || ink.y + ink.h > h
                        {
                            return Err(format!(
                                "Source outline ink exceeds page: {n}/{}",
                                source.line_number
                            )
                            .into());
                        }
                    }
                }
            }
        }
        let codepoints: Vec<_> = source_characters.into_iter().collect();
        let mut glyph_ids = vec![0u16; codepoints.len()];
        unsafe {
            let face = self.fonts.GetFontFamily(0)?.GetFont(0)?.CreateFontFace()?;
            face.GetGlyphIndices(
                codepoints.as_ptr(),
                codepoints.len() as u32,
                glyph_ids.as_mut_ptr(),
            )?;
        }
        let missing: Vec<_> = codepoints
            .iter()
            .zip(&glyph_ids)
            .filter(|(_, id)| **id == 0)
            .map(|(c, _)| format!("U+{c:04X}"))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "Authentic font has missing source character mappings: {}",
                missing.join(", ")
            )
            .into());
        }
        Ok(serde_json::json!({
            "pages":604,"naturalLines":checked_lines,"naturalWords":checked_words,
            "naturalAdjacentPairs":natural_pairs,"minimumNaturalInkGapDip":minimum_clearance,
            "closestNaturalPair":{"page":closest.0,"previousLine":closest.1,"nextLine":closest.2},
            "minimumOutlineFit":minimum_outline_fit,"minimumOutlineInkGapDip":minimum_outline_clearance,
            "outlineWordsChecked":outlined_words,"naturalPageWidth":PAGE_WIDTH,"naturalPageHeight":PAGE_HEIGHT,
            "quranFontFamily":self.font_family.to_string(),"sourceWordOffsetsUtf16":true,
            "sourceCharacterMappingsChecked":codepoints.len(),"missingSourceCharacterMappings":missing,
            "highlightRepaintsGlyphs":false,
            "ornateSurahHeaderGlyphsVerified":self.header_ligatures.len(),
            "fullWidthRowsVerified":full_width_rows,"maximumEndpointErrorDip":maximum_endpoint_error,
            "layeredHeadersPrepared":self.header_art.len(),"originalHeaderLayerCounts":self.header_art.values().map(|art|art.layers.len()).collect::<Vec<_>>(),
            "originalV4BasmalaGlyphsVerified":4,"originalPageMetadataVerified":604,
            "uiLabelCache":ui_label_cache,
            "nativeWoff2HeaderUnpacking":header_container=="woff2","headerFontContainer":header_container,"basmalaFontContainer":basmala_container,
            "originalBasmalaGlyphIndicesVerified":[150,151,154,160],"openingPagesCentered":true
        }))
    }
    #[cfg(feature = "qa")]
    pub fn opening_snapshots(
        &mut self,
        resources: &mut Resources,
        output: &std::path::Path,
    ) -> Result<()> {
        std::fs::create_dir_all(output)?;
        for contextual in [false, true] {
            self.contextual = contextual;
            for theme in 0..=2 {
                for density in [1., 1.25, 2.] {
                    self.snapshot_case(
                        resources,
                        1,
                        theme,
                        &output.join(format!(
                            "opening-{}-theme-{theme}-dpi-{}.png",
                            if contextual { "contextual" } else { "natural" },
                            (96. * density) as u32
                        )),
                        density,
                        None,
                    )?;
                }
            }
        }
        std::fs::write(
            output.join("opening-rendering.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "passed":true,"snapshots":18,"originalContrast":std::env::args().any(|arg|arg=="--qa-original-contrast"),
                "gamma":unsafe {self.opening_light_params.GetGamma()},"enhancedContrast":unsafe {self.opening_light_params.GetEnhancedContrast()},
                "scope":"Original shaped glyphs and positions, natural/contextual, light/sepia/dark, 96/120/192 DPI. Presentation contrast only."
            }))?,
        )?;
        Ok(())
    }
    pub fn window_target(
        &mut self,
        hwnd: HWND,
        width: u32,
        height: u32,
        dpi: f32,
        _compose_bitmap: bool,
    ) -> Result<ID2D1RenderTarget> {
        unsafe {
            // Keep one presentation resource domain through contextual motion
            // and its settled vector frame. Recreating it at each boundary
            // adds a cold full-surface paint and invalidates uploaded resources.
            // Natural DirectWrite retains its existing default target.
            let software = self.contextual;
            #[cfg(feature = "qa")]
            let software = software || std::env::args().any(|arg| arg == "--qa-software-target");
            #[cfg(feature = "qa")]
            let software =
                software && !std::env::args().any(|arg| arg == "--qa-accelerated-context");
            if self.window_software != software {
                self.discard_target();
                self.window_software = software;
            }
            if let Some(target) = &self.target {
                let pixels = target.GetPixelSize();
                if pixels.width != width.max(1) || pixels.height != height.max(1) {
                    target.Resize(&D2D_SIZE_U {
                        width: width.max(1),
                        height: height.max(1),
                    })?;
                    self.surface_dirty = true;
                }
                target.SetDpi(dpi, dpi);
                return Ok(target.cast()?);
            }
            let kind = if software {
                D2D1_RENDER_TARGET_TYPE_SOFTWARE
            } else {
                D2D1_RENDER_TARGET_TYPE_DEFAULT
            };
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: kind,
                dpiX: dpi,
                dpiY: dpi,
                ..Default::default()
            };
            let target = self.factory.CreateHwndRenderTarget(
                &properties,
                &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd,
                    pixelSize: D2D_SIZE_U {
                        width: width.max(1),
                        height: height.max(1),
                    },
                    presentOptions: D2D1_PRESENT_OPTIONS_RETAIN_CONTENTS
                        | D2D1_PRESENT_OPTIONS_IMMEDIATELY,
                },
            )?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
            let result = target.cast()?;
            self.target = Some(target);
            Ok(result)
        }
    }
    pub fn discard_target(&mut self) {
        self.presentation_generation = self.presentation_generation.wrapping_add(1);
        self.target = None;
        self.surface_dirty = true;
    }
    pub fn presentation_generation(&self) -> u64 {
        self.presentation_generation
    }
    #[cfg(feature = "qa")]
    pub fn verify_optional_fallback(
        &self,
        resources: &Resources,
        output: &Path,
    ) -> Result<serde_json::Value> {
        // Page 604 exercises several original ornate headers and basmala rows
        // together. Check both source-font and contextual views at real monitor
        // densities; these are authentic render captures, never generated text.
        let decorations = output.join("ornate-header-snapshots");
        std::fs::create_dir_all(&decorations)?;
        let mut original = Resources::open(resources.root.clone())?;
        let mut renderer = Self::new(&mut original)?;
        let mut decoration_captures = 0;
        for contextual in [false, true] {
            renderer.contextual = contextual;
            for theme in 0..=3 {
                for density in [1.25, 2.] {
                    renderer.snapshot_case(
                        &mut original,
                        604,
                        theme,
                        &decorations.join(format!(
                            "page-604-{}-theme-{theme}-dpi-{}.png",
                            if contextual { "contextual" } else { "natural" },
                            (density * 96.) as u32
                        )),
                        density,
                        Some((113, 2)),
                    )?;
                    decoration_captures += 1;
                }
            }
        }
        let mut cases = Vec::new();
        for name in [
            "missing-contextual-assets",
            "corrupt-contextual-json",
            "missing-contextual-word-bounds",
            "wrong-contextual-word-id",
        ] {
            let directory = output.join(name);
            let root = directory.join("Resources");
            for relative in [
                "data/qul/processed/surahs.json",
                "fonts/digital-khatt/DigitalKhattV2.otf",
                "experimental/digital-khatt/pages/p1.json",
                "experimental/digital-khatt/pages/p45.json",
                "experimental/digital-khatt/pages/p46.json",
            ] {
                let original = std::fs::read(resources.root.join(relative))?;
                let destination = root.join(relative);
                std::fs::create_dir_all(destination.parent().ok_or("Invalid QA fixture path")?)?;
                std::fs::write(&destination, &original)?;
                if std::fs::read(destination)? != original {
                    return Err("Copied authentic QA source bytes differ".into());
                }
            }
            if name != "missing-contextual-assets" {
                let original = std::fs::read(
                    resources
                        .root
                        .join("experimental/digital-khatt/shaped/glyphs.json"),
                )?;
                let shaped = root.join("experimental/digital-khatt/shaped");
                std::fs::create_dir_all(&shaped)?;
                std::fs::write(shaped.join("glyphs.json"), &original)?;
                if std::fs::read(shaped.join("glyphs.json"))? != original {
                    return Err("Copied original QA outlines differ".into());
                }
                let damaged = if name == "corrupt-contextual-json" {
                    b"{corrupt optional QA fixture}".to_vec()
                } else {
                    let mut page: serde_json::Value = crate::data::read_json(
                        &resources
                            .root
                            .join("experimental/digital-khatt/shaped/p45.json"),
                    )?;
                    let line = page["lines"]
                        .as_object_mut()
                        .and_then(|lines| lines.values_mut().next())
                        .ok_or("Missing QA shaped line")?;
                    if name == "missing-contextual-word-bounds" {
                        line["wordBounds"] = serde_json::json!({});
                    } else {
                        line["glyphs"][0][3] = serde_json::json!(0);
                    }
                    serde_json::to_vec(&page)?
                };
                std::fs::write(shaped.join("p45.json"), damaged)?;
            }
            let mut fixture = Resources::open(root)?;
            let mut renderer = Self::new(&mut fixture)?;
            renderer.contextual = true;
            renderer.snapshot_case(
                &mut fixture,
                45,
                0,
                &directory.join("natural-fallback.png"),
                1.,
                Some((2, 265)),
            )?;
            if renderer.contextual
                || !renderer.contextual_unavailable
                || renderer.visible.iter().any(|line| line.hits.is_some())
                || renderer
                    .visible
                    .iter()
                    .map(|line| line.runs.len())
                    .sum::<usize>()
                    == 0
            {
                return Err("Optional contextual rendering did not safely fall back".into());
            }
            // Mandatory source-page errors remain explicit errors, even after
            // an optional presentation fallback. Never fabricate missing text.
            if fixture.page(2).is_ok() {
                return Err("Missing mandatory source page was silently accepted".into());
            }
            cases.push(serde_json::json!({"case":name,"naturalSourceFallback":true,
                "visibleAuthenticWords":renderer.visible.iter().map(|line| line.runs.len()).sum::<usize>(),
                "sourceCopiesByteIdentical":true,"mandatorySourceErrorsRemainFatal":true}));
        }
        Ok(
            serde_json::json!({"cases":cases,"optionalDataCannotBlockReading":true,
            "originalDecorativeCaptures":decoration_captures,"decorativeCaptureDensities":[120,192]}),
        )
    }
    #[cfg(feature = "qa")]
    pub fn snapshot(
        &mut self,
        resources: &mut Resources,
        page: u32,
        theme: u32,
        path: &Path,
    ) -> Result<()> {
        self.snapshot_case(resources, page, theme, path, 1., None)
    }
    #[cfg(feature = "qa")]
    pub fn snapshot_case(
        &mut self,
        resources: &mut Resources,
        page: u32,
        theme: u32,
        path: &Path,
        density: f32,
        selection: Option<(u32, u32)>,
    ) -> Result<()> {
        if !density.is_finite() || !(0.5..=4.).contains(&density) {
            return Err("Invalid QA display density".into());
        }
        self.snapshot_sized(
            resources, page, theme, path, density, selection, 1600., 1100.,
        )
    }
    #[cfg(feature = "qa")]
    #[allow(clippy::too_many_arguments)]
    pub fn snapshot_sized(
        &mut self,
        resources: &mut Resources,
        page: u32,
        theme: u32,
        path: &Path,
        density: f32,
        selection: Option<(u32, u32)>,
        width: f32,
        height: f32,
    ) -> Result<()> {
        self.snapshot_sized_with_opening(
            resources, page, theme, path, density, selection, width, height, None, None,
        )
    }

    /// Exercise the same live highlight paths without modifying source glyphs,
    /// page geometry, text ranges or native hit testing.
    #[cfg(feature = "qa")]
    #[allow(clippy::too_many_arguments)]
    pub fn snapshot_dual_highlights(
        &mut self,
        resources: &mut Resources,
        page: u32,
        theme: u32,
        path: &Path,
        selection: Option<(u32, u32)>,
        recitation: Option<(u32, u32)>,
    ) -> Result<serde_json::Value> {
        self.snapshot_sized_with_opening(
            resources, page, theme, path, 1., selection, 1600., 1100., None, recitation,
        )?;
        let mut ranges = Vec::new();
        for region in &self.ayah_highlights {
            let rect = region.rect;
            if ![rect.x, rect.y, rect.w, rect.h]
                .iter()
                .all(|value| value.is_finite())
                || rect.w <= 0.
                || rect.h <= 0.
                || rect.x < 1.
                || rect.y < 1.
                || rect.x + rect.w > 1599.
                || rect.y + rect.h > 1099.
            {
                return Err("Ayah highlight has invalid or off-snapshot geometry".into());
            }
            ranges.push(serde_json::json!({
                "ayah":[region.key.0,region.key.1],
                "style":match region.style { HighlightStyle::Filled => "filled", HighlightStyle::Outlined => "outlined" },
                "rect":[rect.x,rect.y,rect.w,rect.h],
                "strokeOutsetDip":if region.style == HighlightStyle::Outlined { 0.625 } else { 0. }
            }));
        }
        Ok(serde_json::json!(ranges))
    }

    #[cfg(feature = "qa")]
    pub fn verify_dual_highlights(
        &mut self,
        resources: &mut Resources,
        output: &Path,
    ) -> Result<serde_json::Value> {
        std::fs::create_dir_all(output)?;
        let previous_contextual = self.contextual;
        let previous_clock = self.show_clock;
        self.show_clock = false;
        let result = (|| -> Result<_> {
            let mut series = Vec::new();
            for (name, contextual, page, studied, recited) in [
                ("opening-natural", false, 1, (1, 2), (1, 3)),
                ("directwrite", false, 45, (2, 265), (2, 266)),
                ("contextual", true, 45, (2, 265), (2, 266)),
            ] {
                self.contextual = contextual;
                for theme in [0, 1, 2] {
                    let mut cases = Vec::new();
                    let mut original_hits = None;
                    let mut normal_pixels = None;
                    for (state, selection, recitation) in [
                        ("none", None, None),
                        ("selection", Some(studied), None),
                        ("recitation", None, Some(studied)),
                        ("same", Some(studied), Some(studied)),
                        ("different", Some(studied), Some(recited)),
                    ] {
                        let filename = format!("dual-{name}-theme-{theme}-{state}.png");
                        let path = output.join(&filename);
                        let ranges = self.snapshot_dual_highlights(
                            resources, page, theme, &path, selection, recitation,
                        )?;
                        if self.contextual != contextual {
                            return Err(
                                "Dual highlight QA unexpectedly used outline fallback".into()
                            );
                        }
                        // Include exact source text positions and every live
                        // native hit box; visual feedback must not change them.
                        let hits: Vec<_> = self.visible.iter().map(|line| {
                            serde_json::json!({
                                "line":line.line_number,
                                "rect":[line.rect.x,line.rect.y,line.rect.w,line.rect.h],
                                "origin":[line.origin.X,line.origin.Y],"scale":line.scale,
                                "runs":line.runs.iter().map(|run| [run.start,run.len,run.surah,run.ayah]).collect::<Vec<_>>(),
                                "hits":line.hits.as_ref().map(|hits| hits.iter().map(|(rect,key)| {
                                    serde_json::json!({"ayah":[key.0,key.1],"rect":[rect.x,rect.y,rect.w,rect.h]})
                                }).collect::<Vec<_>>())
                            })
                        }).collect();
                        if let Some(original) = &original_hits {
                            if *original != hits {
                                return Err(
                                    "Dual highlights changed original glyph positions or hit maps"
                                        .into(),
                                );
                            }
                        } else {
                            original_hits = Some(hits);
                        }
                        let range_list = ranges.as_array().ok_or("Missing highlight QA ranges")?;
                        let filled = range_list
                            .iter()
                            .filter(|range| range["style"] == "filled")
                            .count();
                        let outlined = range_list
                            .iter()
                            .filter(|range| range["style"] == "outlined")
                            .count();
                        if (state == "none" && !range_list.is_empty())
                            || (state != "none" && filled == 0)
                            || (state == "different" && outlined == 0)
                            || (state != "different" && outlined != 0)
                        {
                            return Err(
                                "Dual highlight precedence or visible source ranges differ".into(),
                            );
                        }
                        let pixels = std::fs::read(&path)?;
                        if state == "selection" {
                            normal_pixels = Some(pixels);
                        } else if matches!(state, "same" | "recitation")
                            && normal_pixels.as_ref() != Some(&pixels)
                        {
                            return Err("Same-ayah or recitation-only pixels differ from the normal highlight".into());
                        }
                        cases.push(
                            serde_json::json!({"state":state,"file":filename,"ranges":ranges}),
                        );
                    }
                    series.push(serde_json::json!({
                        "path":name,"contextual":contextual,"page":page,"theme":theme,
                        "studied":[studied.0,studied.1],"recited":[recited.0,recited.1],"cases":cases,
                        "originalGlyphPositionsAndHitMapsUnchanged":true,
                        "sameAndRecitationOnlyPixelsEqualNormalHighlight":true
                    }));
                }
            }
            Ok(serde_json::json!({"passed":true,"series":series,
                "outlinedStrokeDip":1.25,"highlightsRepaintGlyphs":false,
                "scope":"Original native font, natural opening glyph runs and contextual outline paths; fixed 1600x1100 native WIC target. No source content is regenerated."}))
        })();
        self.contextual = previous_contextual;
        self.show_clock = previous_clock;
        result
    }

    /// Capture the real opening-highlight path and report the exact DIP bounds
    /// it painted. A page without that surah's unnumbered Bismillah reports no
    /// bounds, so QA can check multi-surah pages and absent openings alike.
    #[cfg(feature = "qa")]
    pub fn snapshot_opening(
        &mut self,
        resources: &mut Resources,
        page: u32,
        theme: u32,
        path: &Path,
        surah: u32,
    ) -> Result<Vec<Rect>> {
        self.snapshot_opening_sized(resources, page, theme, path, surah, 1600., 1100.)
    }
    #[cfg(feature = "qa")]
    #[allow(clippy::too_many_arguments)]
    pub fn snapshot_opening_sized(
        &mut self,
        resources: &mut Resources,
        page: u32,
        theme: u32,
        path: &Path,
        surah: u32,
        width: f32,
        height: f32,
    ) -> Result<Vec<Rect>> {
        if !width.is_finite()
            || !height.is_finite()
            || !(128. ..=4096.).contains(&width)
            || !(128. ..=4096.).contains(&height)
        {
            return Err("Invalid QA opening snapshot dimensions".into());
        }
        self.snapshot_sized_with_opening(
            resources,
            page,
            theme,
            path,
            1.,
            None,
            width,
            height,
            Some(surah),
            None,
        )?;
        if self.opening_highlights.iter().any(|rect| {
            ![rect.x, rect.y, rect.w, rect.h]
                .iter()
                .all(|value| value.is_finite())
                || rect.w <= 0.
                || rect.h <= 0.
                || rect.x < 0.
                || rect.y < 0.
                || rect.x + rect.w > width
                || rect.y + rect.h > height
        }) {
            return Err("Opening highlight has invalid or off-snapshot geometry".into());
        }
        Ok(self.opening_highlights.clone())
    }
    #[cfg(feature = "qa")]
    #[allow(clippy::too_many_arguments)]
    fn snapshot_sized_with_opening(
        &mut self,
        resources: &mut Resources,
        page: u32,
        theme: u32,
        path: &Path,
        density: f32,
        selection: Option<(u32, u32)>,
        width: f32,
        height: f32,
        opening: Option<u32>,
        recitation: Option<(u32, u32)>,
    ) -> Result<()> {
        let pixel_width = (width * density).round() as u32;
        let pixel_height = (height * density).round() as u32;
        unsafe {
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let bitmap = wic.CreateBitmap(
                pixel_width,
                pixel_height,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )?;
            let target = self.factory.CreateWicBitmapRenderTarget(
                &bitmap,
                &D2D1_RENDER_TARGET_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96. * density,
                    dpiY: 96. * density,
                    ..Default::default()
                },
            )?;
            let palette = Palette::new(theme);
            target.BeginDraw();
            target.Clear(Some(&palette.background));
            self.book(
                &target,
                resources,
                BookView {
                    page,
                    area: Rect {
                        x: 0.,
                        y: 12.,
                        w: width,
                        h: height - 24.,
                    },
                    palette,
                    selection,
                    recitation,
                    opening,
                    arabic: true,
                },
            )?;
            target.EndDraw(None, None)?;
            let stream = wic.CreateStream()?;
            stream.InitializeFromFilename(
                &HSTRING::from(path.to_string_lossy().as_ref()),
                0x40000000,
            )?;
            let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let mut frame = None;
            let mut options = None;
            encoder.CreateNewFrame(&mut frame, &mut options)?;
            let frame = frame.ok_or("Missing PNG frame")?;
            frame.Initialize(options.as_ref())?;
            frame.SetSize(pixel_width, pixel_height)?;
            let mut format = GUID_WICPixelFormat32bppPBGRA;
            frame.SetPixelFormat(&mut format)?;
            frame.WriteSource(&bitmap, std::ptr::null())?;
            frame.Commit()?;
            encoder.Commit()?;
            Ok(())
        }
    }
}
fn row_inset(row: f32) -> f32 {
    row * 0.14
}

pub fn color_rgb(c: D2D1_COLOR_F) -> u32 {
    ((c.r * 255.).round() as u32)
        | (((c.g * 255.).round() as u32) << 8)
        | (((c.b * 255.).round() as u32) << 16)
}

#[cfg(test)]
mod tests {
    use super::{
        HighlightStyle, PageChrome, Palette, Rect, contrast, highlight_targets, metadata_number,
        section_mark, source_range_bounds,
    };
    #[test]
    fn study_and_recitation_keep_distinct_locations_without_stacking() {
        let study = (2, 265);
        let recitation = (2, 266);
        assert_eq!(highlight_targets(None, None), [None, None]);
        assert_eq!(
            highlight_targets(Some(study), Some(recitation)),
            [
                Some((study, HighlightStyle::Filled)),
                Some((recitation, HighlightStyle::Outlined))
            ]
        );
        assert_eq!(
            highlight_targets(Some(study), Some(study)),
            [Some((study, HighlightStyle::Filled)), None]
        );
        assert_eq!(
            highlight_targets(None, Some(recitation)),
            [None, Some((recitation, HighlightStyle::Filled))]
        );
    }
    #[test]
    fn same_row_highlights_stop_at_their_authentic_ayah_boundaries() {
        let regions = [
            (
                Rect {
                    x: 85.,
                    y: 40.,
                    w: 20.,
                    h: 30.,
                },
                (2, 265),
            ),
            (
                Rect {
                    x: 55.,
                    y: 40.,
                    w: 25.,
                    h: 30.,
                },
                (2, 265),
            ),
            (
                Rect {
                    x: 15.,
                    y: 40.,
                    w: 35.,
                    h: 30.,
                },
                (2, 266),
            ),
        ];
        let study = source_range_bounds(&regions, (2, 265)).unwrap();
        let recitation = source_range_bounds(&regions, (2, 266)).unwrap();
        assert_eq!((study.x, study.w), (55., 50.));
        assert_eq!((recitation.x, recitation.w), (15., 35.));
        assert!(recitation.x + recitation.w < study.x);
        assert_eq!(study.y, 44.2);
        assert_eq!(study.h, 21.6);
        assert!(source_range_bounds(&regions, (9, 1)).is_none());
    }
    #[test]
    fn theme_fades_keep_reading_ink_visible() {
        for from in 0..=3 {
            for to in 0..=3 {
                for step in 0..=100 {
                    let palette = Palette::new(from).blend(Palette::new(to), step as f32 / 100.);
                    assert!(
                        contrast(palette.ink, palette.paper) >= 4.,
                        "paper contrast {from}->{to}/{step}"
                    );
                    assert!(
                        contrast(palette.ink, palette.background) >= 4.,
                        "control contrast {from}->{to}/{step}"
                    );
                }
            }
        }
    }
    #[test]
    fn section_footer_matches_original_hizb_quarters() {
        assert_eq!(
            section_mark(Some(5), Some(19), false).as_deref(),
            Some("Hizb 5")
        );
        assert_eq!(
            section_mark(None, Some(18), false).as_deref(),
            Some("Quarter of Hizb 5")
        );
        assert_eq!(
            section_mark(None, Some(19), true).as_deref(),
            Some("نصف الحزب ٥")
        );
        assert_eq!(
            section_mark(None, Some(20), true).as_deref(),
            Some("ثلاثة أرباع الحزب ٥")
        );
        assert_eq!(section_mark(None, None, true), None);
        assert_eq!(metadata_number(604, true), "٦٠٤");
    }
    #[test]
    fn source_metadata_matches_the_released_page_marks() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public-qul");
        let chrome = PageChrome::load(&root).unwrap();
        assert_eq!(chrome.pages.len(), 604);
        assert_eq!((chrome.pages[44].first_surah, chrome.pages[44].juz), (2, 3));
        assert_eq!(
            section_mark(
                chrome.hizb.get(&46).copied(),
                chrome.rub.get(&46).copied(),
                false
            )
            .as_deref(),
            Some("Half of Hizb 5")
        );
        assert_eq!(chrome.pages[534].juz, 27);
    }
}
