//! DPI-aware chrome. Windows owns the buttons, focus and accessible names;
//! DirectWrite owns their text and Direct2D owns their vector strokes.
use crate::{
    data::Result,
    render::{Palette, Rect},
};
use std::{cell::RefCell, collections::VecDeque};
use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::*,
            Gdi::HDC,
        },
    },
    core::w,
};
use windows_numerics::Vector2;

#[derive(Clone, Copy)]
pub enum Icon {
    None,
    Search,
    Listen,
    Book,
    Settings,
    Minimize,
    Maximize,
    Restore,
    ExitFullscreen,
    Close,
    Back,
    Forward,
    Play,
    Pause,
    Stop,
    Menu,
    Bookmark,
    Expand,
    Collapse,
    SeekBack,
    SeekForward,
    Link,
}
pub struct Button<'a> {
    pub text: &'a str,
    pub detail: &'a str,
    pub quran_detail: bool,
    pub menu: bool,
    pub icon: Icon,
    pub active: bool,
    pub disabled: bool,
    pub focus: bool,
    pub flat: bool,
    pub rtl: bool,
    pub submenu: bool,
    pub check: Option<bool>,
}
type CachedLabel = (String, u32, u32, u32, bool, bool, IDWriteTextLayout);
pub struct Chrome {
    factory: ID2D1Factory,
    write: IDWriteFactory,
    target: RefCell<Option<ID2D1DCRenderTarget>>,
    labels: RefCell<VecDeque<CachedLabel>>,
    book: ID2D1PathGeometry,
    round_stroke: ID2D1StrokeStyle,
    seek: ID2D1PathGeometry,
    check: ID2D1PathGeometry,
}
impl Chrome {
    pub fn new(factory: ID2D1Factory) -> Result<Self> {
        // Keep the open-book outline inside its 16-DIP box, including its
        // rounded stroke. These factory resources survive target recreation.
        let (book, round_stroke) = unsafe {
            let book = factory.CreatePathGeometry()?;
            let sink = book.Open()?;
            let point = |x, y| Vector2 { X: x, Y: y };
            sink.BeginFigure(point(8., 3.5), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: point(6.7, 2.1),
                point2: point(4.3, 1.65),
                point3: point(1.75, 2.1),
            });
            sink.AddLine(point(1.75, 12.4));
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: point(4.4, 11.95),
                point2: point(6.6, 12.55),
                point3: point(8., 14.),
            });
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: point(9.4, 12.55),
                point2: point(11.6, 11.95),
                point3: point(14.25, 12.4),
            });
            sink.AddLine(point(14.25, 2.1));
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: point(11.7, 1.65),
                point2: point(9.3, 2.1),
                point3: point(8., 3.5),
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.BeginFigure(point(8., 3.5), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddLine(point(8., 14.));
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            let round_stroke = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )?;
            (book, round_stroke)
        };
        let seek = unsafe { crate::fluent_icons::seek(&factory)? };
        let check = unsafe { crate::fluent_icons::check(&factory)? };
        Ok(Self {
            seek,
            check,
            factory,
            write: unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? },
            target: RefCell::new(None),
            labels: RefCell::new(VecDeque::new()),
            book,
            round_stroke,
        })
    }
    fn layout(
        &self,
        text: &str,
        size: f32,
        rect: Rect,
        rtl: bool,
        centered: bool,
    ) -> Result<IDWriteTextLayout> {
        let key = (
            size.to_bits(),
            rect.w.to_bits(),
            rect.h.to_bits(),
            rtl,
            centered,
        );
        if let Some(v) = self
            .labels
            .borrow()
            .iter()
            .find(|v| v.0 == text && (v.1, v.2, v.3, v.4, v.5) == key)
        {
            return Ok(v.6.clone());
        }
        unsafe {
            let format = self.write.CreateTextFormat(
                w!("Segoe UI"),
                None,
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
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetTextAlignment(if centered {
                DWRITE_TEXT_ALIGNMENT_CENTER
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            })?;
            let layout = self.write.CreateTextLayout(
                &text.encode_utf16().collect::<Vec<_>>(),
                &format,
                rect.w.max(1.),
                rect.h.max(1.),
            )?;
            let trimming = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                ..Default::default()
            };
            let ellipsis = self.write.CreateEllipsisTrimmingSign(&format)?;
            layout.SetTrimming(&trimming, &ellipsis)?;
            let mut cache = self.labels.borrow_mut();
            // Cached COM layouts are shared by captions and menu rows. Keep
            // alignment in the key and never mutate a cached layout at paint
            // time, or a centered caption can move a later RTL menu label.
            cache.push_front((
                text.into(),
                key.0,
                key.1,
                key.2,
                key.3,
                key.4,
                layout.clone(),
            ));
            cache.truncate(128);
            Ok(layout)
        }
    }
    #[cfg(feature = "qa")]
    pub fn verify_label_direction(&self) -> Result<serde_json::Value> {
        let rect = Rect {
            x: 0.,
            y: 0.,
            w: 180.,
            h: 34.,
        };
        let mut cases = Vec::new();
        for (text, rtl) in [
            ("تفاسير عربية", true),
            ("تفاسير إنجليزية", true),
            ("Arabic tafsirs", false),
            ("English tafsirs", false),
        ] {
            // Exercise the same caption as an aligned menu row, a centered
            // button and an aligned row again. Check native shaping positions,
            // not just the localized strings or our requested format values.
            let leading = self.layout(text, 15., rect, rtl, false)?;
            let centered = self.layout(text, 15., rect, rtl, true)?;
            let reused = self.layout(text, 15., rect, rtl, false)?;
            unsafe {
                if leading.GetTextAlignment() != DWRITE_TEXT_ALIGNMENT_LEADING
                    || reused.GetTextAlignment() != DWRITE_TEXT_ALIGNMENT_LEADING
                    || centered.GetTextAlignment() != DWRITE_TEXT_ALIGNMENT_CENTER
                    || reused.GetReadingDirection()
                        != if rtl {
                            DWRITE_READING_DIRECTION_RIGHT_TO_LEFT
                        } else {
                            DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
                        }
                {
                    return Err("Chrome caption alignment leaked into a menu label".into());
                }
                let mut metrics = DWRITE_TEXT_METRICS::default();
                reused.GetMetrics(&mut metrics)?;
                let mut center_metrics = DWRITE_TEXT_METRICS::default();
                centered.GetMetrics(&mut center_metrics)?;
                let mut clusters = [DWRITE_CLUSTER_METRICS::default(); 64];
                let mut cluster_count = 0;
                reused.GetClusterMetrics(Some(&mut clusters), &mut cluster_count)?;
                let length = text.encode_utf16().count() as u32;
                let mut first = DWRITE_HIT_TEST_METRICS::default();
                let mut last = DWRITE_HIT_TEST_METRICS::default();
                let (mut first_x, mut first_y, mut last_x, mut last_y) = (0., 0., 0., 0.);
                reused.HitTestTextPosition(0, false, &mut first_x, &mut first_y, &mut first)?;
                reused.HitTestTextPosition(
                    length - 1,
                    false,
                    &mut last_x,
                    &mut last_y,
                    &mut last,
                )?;
                if !metrics.width.is_finite()
                    || metrics.width <= 0.
                    || metrics.width >= rect.w
                    || metrics.lineCount != 1
                    || first.isTrimmed.as_bool()
                    || last.isTrimmed.as_bool()
                    || (first.bidiLevel % 2 == 1) != rtl
                    || (last.bidiLevel % 2 == 1) != rtl
                    || (first_x > last_x) != rtl
                    || (metrics.left > center_metrics.left) != rtl
                    || clusters[..cluster_count as usize]
                        .iter()
                        .map(|cluster| u32::from(cluster.length))
                        .sum::<u32>()
                        != length
                {
                    return Err(
                        "Native Arabic/English chrome layout failed its direction check".into(),
                    );
                }
                cases.push(serde_json::json!({
                    "text":text,"rtl":rtl,"nativeBidiLevels":[first.bidiLevel,last.bidiLevel],
                    "leadingLeft":metrics.left,"centeredLeft":center_metrics.left,
                    "firstCharacterX":first_x,"lastCharacterX":last_x,
                    "width":metrics.width,"clusterCount":cluster_count,
                    "alignmentIsolated":true,"untrimmed":true
                }));
            }
        }
        Ok(serde_json::json!({
            "passed":true,"cases":cases,
            "scope":"Native DirectWrite cluster coverage, bidi hit positions and cached alignment isolation; visual manager captures complement these geometry checks"
        }))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn surah_row(
        &self,
        dc: HDC,
        pixels: RECT,
        dpi: f32,
        palette: Palette,
        number: u32,
        english: &str,
        arabic: &str,
        active: bool,
        focus: bool,
    ) -> Result<()> {
        self.button(
            dc,
            pixels,
            dpi,
            palette,
            Button {
                text: "",
                detail: "",
                quran_detail: false,
                menu: false,
                icon: Icon::None,
                active,
                disabled: false,
                focus,
                flat: true,
                rtl: false,
                submenu: false,
                check: None,
            },
        )?;
        let w = (pixels.right - pixels.left) as f32 * 96. / dpi;
        let h = (pixels.bottom - pixels.top) as f32 * 96. / dpi;
        let number_rect = Rect {
            x: 12.,
            y: 0.,
            w: 32.,
            h,
        };
        let name_rect = Rect {
            x: 54.,
            y: 0.,
            w: (w - 54.) * 0.58,
            h,
        };
        let arabic_rect = Rect {
            x: w * 0.66,
            y: 0.,
            w: w * 0.34 - 16.,
            h,
        };
        let n = self.layout(&number.to_string(), 13., number_rect, false, false)?;
        let en = self.layout(english, 15., name_rect, false, false)?;
        let ar = self.layout(arabic, 15., arabic_rect, true, false)?;
        unsafe {
            let target = self.target.borrow().as_ref().unwrap().clone();
            let ink = target.CreateSolidColorBrush(&palette.ink, None)?;
            let muted = target.CreateSolidColorBrush(&palette.muted, None)?;
            target.BeginDraw();
            for (layout, rect, brush) in [
                (&n, number_rect, &muted),
                (&en, name_rect, &ink),
                (&ar, arabic_rect, &ink),
            ] {
                target.DrawTextLayout(
                    Vector2 {
                        X: rect.x,
                        Y: rect.y,
                    },
                    layout,
                    brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            target.EndDraw(None, None)?;
        }
        Ok(())
    }
    pub fn button(
        &self,
        dc: HDC,
        pixels: RECT,
        dpi: f32,
        palette: Palette,
        button: Button<'_>,
    ) -> Result<()> {
        unsafe {
            if self.target.borrow().is_none() {
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: crate::control_vectors::dc_target_type(),
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    ..Default::default()
                };
                *self.target.borrow_mut() = Some(self.factory.CreateDCRenderTarget(&props)?);
            }
            let target = self.target.borrow().as_ref().unwrap().clone();
            target.BindDC(dc, &pixels)?;
            target.SetDpi(dpi, dpi);
            target.SetTransform(&windows_numerics::Matrix3x2::identity());
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
            let rect = Rect {
                x: 0.,
                y: 0.,
                w: (pixels.right - pixels.left) as f32 * 96. / dpi,
                h: (pixels.bottom - pixels.top) as f32 * 96. / dpi,
            };
            target.BeginDraw();
            target.Clear(Some(&palette.background));
            let result = self.paint_button(&target, rect, palette, button);
            if let Err(error) = target.EndDraw(None, None) {
                *self.target.borrow_mut() = None;
                return Err(error.into());
            }
            result
        }
    }
    /// Draw the identical vectors and layouts in the parent frame before its
    /// single Present. Native HWNDs retain input, focus and accessibility.
    pub fn paint_button(
        &self,
        target: &ID2D1RenderTarget,
        rect: Rect,
        palette: Palette,
        button: Button<'_>,
    ) -> Result<()> {
        unsafe {
            // Prepare layouts before BeginDraw, so errors cannot strand a drawing session.
            let has_icon = !matches!(button.icon, Icon::None | Icon::Link);
            let snippet = button.quran_detail;
            let caption_only = matches!(
                button.icon,
                Icon::Minimize
                    | Icon::Maximize
                    | Icon::Restore
                    | Icon::ExitFullscreen
                    | Icon::Close
                    | Icon::Back
                    | Icon::Forward
                    | Icon::Expand
                    | Icon::Collapse
                    | Icon::SeekBack
                    | Icon::SeekForward
            ) && rect.w < 70.
                || matches!(button.icon, Icon::Play | Icon::Pause) && rect.w <= 110.
                || matches!(button.icon, Icon::Stop) && rect.w < 60.;
            let inset = if has_icon {
                36.
            } else if rect.w < 50. {
                4.
            } else {
                12.
            };
            let mut text_rect = Rect {
                x: if button.rtl && rect.w >= 50. {
                    12.
                } else {
                    inset
                },
                y: if button.detail.is_empty() { 0. } else { 4. },
                w: rect.w
                    - inset
                    - if rect.w < 50. {
                        4.
                    } else if button.submenu {
                        32.
                    } else {
                        12.
                    },
                h: if button.detail.is_empty() {
                    rect.h
                } else {
                    25.
                },
            };
            if matches!(button.icon, Icon::Link) {
                text_rect.x = 4.;
                text_rect.w = rect.w - 8.;
            }
            if button.check.is_some() {
                text_rect.w -= 32.;
                if button.rtl {
                    text_rect.x += 32.;
                }
            }
            let layout = if caption_only {
                None
            } else {
                Some(self.layout(
                    button.text,
                    if matches!(button.icon, Icon::Link) {
                        12.
                    } else if button.flat && !has_icon && !button.menu {
                        13.
                    } else {
                        15.
                    },
                    text_rect,
                    button.rtl,
                    !button.flat || matches!(button.icon, Icon::Link),
                )?)
            };
            let detail_rect = Rect {
                y: 29.,
                h: 22.,
                ..text_rect
            };
            let detail = if button.detail.is_empty() {
                None
            } else {
                Some(self.layout(
                    button.detail,
                    if snippet { 17. } else { 12. },
                    detail_rect,
                    snippet || button.rtl,
                    false,
                )?)
            };
            let ink = if button.disabled {
                palette.muted
            } else {
                palette.ink
            };
            let fill = target.CreateSolidColorBrush(
                &if button.active {
                    palette.highlight
                } else if button.flat {
                    palette.background
                } else {
                    palette.paper
                },
                None,
            )?;
            let ink = if matches!(button.icon, Icon::Link) {
                let gray = (palette.muted.r + palette.muted.g + palette.muted.b) / 3.;
                D2D1_COLOR_F {
                    r: gray,
                    g: gray,
                    b: gray,
                    a: 1.,
                }
            } else {
                ink
            };
            let brush = target.CreateSolidColorBrush(&ink, None)?;
            let border = target.CreateSolidColorBrush(&palette.border, None)?;
            let muted = target
                .CreateSolidColorBrush(&if snippet { palette.ink } else { palette.muted }, None)?;
            let round = D2D1_ROUNDED_RECT {
                rect: Rect {
                    x: 1.,
                    y: 1.,
                    w: rect.w - 2.,
                    h: rect.h - 2.,
                }
                .d2d(),
                radiusX: 6.,
                radiusY: 6.,
            };
            if !button.flat || button.active {
                target.FillRoundedRectangle(&round, &fill);
            }
            if !button.flat {
                target.DrawRoundedRectangle(&round, &border, 1., None);
            }
            let x = if caption_only {
                rect.w / 2. - 8.
            } else if button.rtl {
                rect.w - 27.
            } else {
                11.
            };
            let y = if button.detail.is_empty() {
                rect.h / 2. - 8.
            } else {
                13.
            };
            let point = |a: f32, b: f32| Vector2 { X: x + a, Y: y + b };
            let line = |a: f32, b: f32, c: f32, d: f32| {
                target.DrawLine(point(a, b), point(c, d), &brush, 1.25, None)
            };
            match button.icon {
                Icon::Pause => {
                    line(5., 2., 5., 14.);
                    line(11., 2., 11., 14.);
                }
                Icon::Stop => {
                    line(3., 3., 13., 3.);
                    line(13., 3., 13., 13.);
                    line(13., 13., 3., 13.);
                    line(3., 13., 3., 3.);
                }
                Icon::Menu => {
                    line(1., 3., 15., 3.);
                    line(1., 8., 15., 8.);
                    line(1., 13., 15., 13.);
                }
                Icon::Collapse => {
                    line(3., 10., 8., 5.);
                    line(8., 5., 13., 10.);
                }
                Icon::Expand => {
                    line(3., 6., 8., 11.);
                    line(8., 11., 13., 6.);
                }
                Icon::Play => {
                    target.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: point(8., 8.),
                            radiusX: 7.,
                            radiusY: 7.,
                        },
                        &brush,
                        1.25,
                        None,
                    );
                    line(6., 4., 6., 12.);
                    line(6., 4., 11., 8.);
                    line(11., 8., 6., 12.);
                }
                Icon::Bookmark => {
                    line(3., 1., 13., 1.);
                    line(13., 1., 13., 15.);
                    line(13., 15., 8., 11.);
                    line(8., 11., 3., 15.);
                    line(3., 15., 3., 1.);
                }
                Icon::Book => {
                    let mut parent = windows_numerics::Matrix3x2::identity();
                    target.GetTransform(&mut parent);
                    target.SetTransform(
                        &(windows_numerics::Matrix3x2 {
                            M11: 1.,
                            M12: 0.,
                            M21: 0.,
                            M22: 1.,
                            M31: x,
                            M32: y,
                        } * parent),
                    );
                    target.DrawGeometry(&self.book, &brush, 1.25, &self.round_stroke);
                    target.SetTransform(&parent);
                }
                Icon::SeekBack | Icon::SeekForward => {
                    // The filled Fluent outline keeps its arrowhead at every DPI.
                    // Mirroring changes only the symbol, never the numeral or seek action.
                    let mirror = matches!(button.icon, Icon::SeekForward);
                    let mut parent = windows_numerics::Matrix3x2::identity();
                    target.GetTransform(&mut parent);
                    target.SetTransform(
                        &(windows_numerics::Matrix3x2 {
                            M11: if mirror { -1. } else { 1. },
                            M22: 1.,
                            M31: x - 4. + if mirror { 24. } else { 0. },
                            M32: y - 4.,
                            ..windows_numerics::Matrix3x2::identity()
                        } * parent),
                    );
                    target.FillGeometry(&self.seek, &brush, None);
                    target.SetTransform(&parent);
                    let number = self.layout(
                        "10",
                        9.,
                        Rect {
                            x: 0.,
                            y: 0.,
                            w: 16.,
                            h: 16.,
                        },
                        false,
                        true,
                    )?;
                    target.DrawTextLayout(
                        point(0., 0.),
                        &number,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    );
                }
                Icon::Search => {
                    target.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: point(6., 6.),
                            radiusX: 5.,
                            radiusY: 5.,
                        },
                        &brush,
                        1.25,
                        None,
                    );
                    line(10., 10., 15., 15.);
                }
                Icon::Settings => {
                    for (v, c) in [(4., 5.), (12., 11.)] {
                        line(0., v, c - 2., v);
                        line(c + 2., v, 16., v);
                        target.DrawEllipse(
                            &D2D1_ELLIPSE {
                                point: point(c, v),
                                radiusX: 2.,
                                radiusY: 2.,
                            },
                            &brush,
                            1.25,
                            None,
                        );
                    }
                }
                Icon::Listen => {
                    target.DrawRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: Rect {
                                x: x + 1.,
                                y: y + 1.,
                                w: 14.,
                                h: 14.,
                            }
                            .d2d(),
                            radiusX: 7.,
                            radiusY: 7.,
                        },
                        &brush,
                        1.25,
                        None,
                    );
                    target.FillRectangle(
                        &Rect {
                            x,
                            y: y + 9.,
                            w: 16.,
                            h: 7.,
                        }
                        .d2d(),
                        &fill,
                    );
                    for a in [1., 12.] {
                        target.DrawRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: Rect {
                                    x: x + a,
                                    y: y + 8.,
                                    w: 3.,
                                    h: 7.,
                                }
                                .d2d(),
                                radiusX: 1.,
                                radiusY: 1.,
                            },
                            &brush,
                            1.25,
                            None,
                        );
                    }
                }
                Icon::Minimize => line(2., 9., 14., 9.),
                Icon::Maximize => target.DrawRectangle(
                    &Rect {
                        x: x + 3.,
                        y: y + 3.,
                        w: 10.,
                        h: 10.,
                    }
                    .d2d(),
                    &brush,
                    1.,
                    None,
                ),
                Icon::ExitFullscreen => {
                    // Four inward corners distinguish leaving immersive mode
                    // from restoring a maximized window.
                    for (a, b, c) in [
                        ((2., 6.), (6., 6.), (6., 2.)),
                        ((10., 2.), (10., 6.), (14., 6.)),
                        ((2., 10.), (6., 10.), (6., 14.)),
                        ((10., 14.), (10., 10.), (14., 10.)),
                    ] {
                        line(a.0, a.1, b.0, b.1);
                        line(b.0, b.1, c.0, c.1);
                    }
                }
                Icon::Restore => {
                    target.DrawRectangle(
                        &Rect {
                            x: x + 5.,
                            y: y + 2.,
                            w: 9.,
                            h: 9.,
                        }
                        .d2d(),
                        &brush,
                        1.,
                        None,
                    );
                    target.FillRectangle(
                        &Rect {
                            x: x + 2.,
                            y: y + 5.,
                            w: 9.,
                            h: 9.,
                        }
                        .d2d(),
                        &fill,
                    );
                    target.DrawRectangle(
                        &Rect {
                            x: x + 2.,
                            y: y + 5.,
                            w: 9.,
                            h: 9.,
                        }
                        .d2d(),
                        &brush,
                        1.,
                        None,
                    );
                }
                Icon::Close => {
                    line(3., 3., 13., 13.);
                    line(13., 3., 3., 13.);
                }
                Icon::Back => {
                    line(10., 3., 5., 8.);
                    line(5., 8., 10., 13.);
                }
                Icon::Forward => {
                    line(6., 3., 11., 8.);
                    line(11., 8., 6., 13.);
                }
                Icon::None | Icon::Link => {}
            }
            if let Some(layout) = layout {
                target.DrawTextLayout(
                    Vector2 {
                        X: text_rect.x,
                        Y: text_rect.y,
                    },
                    &layout,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
                if matches!(button.icon, Icon::Link) {
                    let mut metrics = DWRITE_TEXT_METRICS::default();
                    layout.GetMetrics(&mut metrics)?;
                    let overhang = layout.GetOverhangMetrics()?;
                    let underline =
                        text_rect.y + metrics.top + metrics.height + overhang.bottom.max(0.) + 2.;
                    if underline < rect.h - 1. {
                        let start = text_rect.x + metrics.left;
                        target.DrawLine(
                            Vector2 {
                                X: start,
                                Y: underline,
                            },
                            Vector2 {
                                X: start + metrics.width,
                                Y: underline,
                            },
                            &brush,
                            0.75,
                            None,
                        );
                    }
                }
            }
            if let Some(layout) = detail {
                target.DrawTextLayout(
                    Vector2 {
                        X: detail_rect.x,
                        Y: detail_rect.y,
                    },
                    &layout,
                    &muted,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            if let Some(checked) = button.check {
                let x = if button.rtl { 12. } else { rect.w - 30. };
                let y = (rect.h - 18.) / 2.;
                let box_rect = D2D1_ROUNDED_RECT {
                    rect: Rect {
                        x,
                        y,
                        w: 18.,
                        h: 18.,
                    }
                    .d2d(),
                    radiusX: 4.,
                    radiusY: 4.,
                };
                if checked {
                    let accent = target.CreateSolidColorBrush(&palette.highlight, None)?;
                    target.FillRoundedRectangle(&box_rect, &accent);
                }
                target.DrawRoundedRectangle(&box_rect, &border, 1., None);
                if checked {
                    let mut parent = windows_numerics::Matrix3x2::identity();
                    target.GetTransform(&mut parent);
                    target.SetTransform(
                        &(windows_numerics::Matrix3x2 {
                            M31: x + 1.,
                            M32: y + 1.,
                            ..windows_numerics::Matrix3x2::identity()
                        } * parent),
                    );
                    target.FillGeometry(&self.check, &brush, None);
                    target.SetTransform(&parent);
                }
            }
            if button.focus {
                target.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: Rect {
                            x: 3.,
                            y: 3.,
                            w: rect.w - 6.,
                            h: rect.h - 6.,
                        }
                        .d2d(),
                        radiusX: 4.,
                        radiusY: 4.,
                    },
                    &brush,
                    1.,
                    None,
                );
            }
            if button.submenu {
                let x = if button.rtl { 12. } else { rect.w - 20. };
                let y = rect.h / 2.;
                let sign = if button.rtl { -1. } else { 1. };
                target.DrawLine(
                    Vector2 {
                        X: x - sign * 2.,
                        Y: y - 4.,
                    },
                    Vector2 {
                        X: x + sign * 2.,
                        Y: y,
                    },
                    &brush,
                    1.25,
                    None,
                );
                target.DrawLine(
                    Vector2 {
                        X: x + sign * 2.,
                        Y: y,
                    },
                    Vector2 {
                        X: x - sign * 2.,
                        Y: y + 4.,
                    },
                    &brush,
                    1.25,
                    None,
                );
            }
            Ok(())
        }
    }
}
