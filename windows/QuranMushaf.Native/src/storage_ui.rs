//! Compact storage details, independent of the explanation's reading font.
use super::*;

impl App {
    /// The RichEdit is shared with Tafsir, whose formatting rectangle has a
    /// different height. Apply padding after the actual HWND move, without
    /// reformatting the report or resetting its selection on every layout.
    pub(super) unsafe fn layout_storage_padding(&self) {
        if self.displayed_panel() != Panel::Storage {
            return;
        }
        unsafe {
            let study = self.controls[&STUDY];
            let mut client = RECT::default();
            if GetClientRect(study, &mut client).is_err() || client.right <= 0 || client.bottom <= 0
            {
                return;
            }
            let horizontal = (16. * self.dpi / 96.).round() as i32;
            let vertical = (12. * self.dpi / 96.).round() as i32;
            let left = horizontal.min((client.right - 1).max(0));
            let top = vertical.min((client.bottom - 1).max(0));
            let wanted = RECT {
                left,
                top,
                right: (client.right - horizontal).max(left + 1).min(client.right),
                bottom: (client.bottom - vertical).max(top + 1).min(client.bottom),
            };
            let mut previous = RECT::default();
            SendMessageW(
                study,
                EM_GETRECT,
                None,
                Some(LPARAM((&mut previous as *mut RECT) as isize)),
            );
            if previous != wanted {
                SendMessageW(
                    study,
                    windows::Win32::UI::Controls::EM_SETRECTNP,
                    None,
                    Some(LPARAM((&wanted as *const RECT) as isize)),
                );
                let _ = InvalidateRect(Some(study), None, false);
            }
        }
    }

    pub(super) unsafe fn storage_details(&mut self) -> Result<()> {
        unsafe {
            let cache = self.cache()?;
            self.set_panel(Panel::Storage);
            self.search = false;
            self.verse_text_end = 0;
            self.study_attribution_start = 0;
            // The shared RichEdit must restore the explanation font next time
            // it displays a tafsir, even if the preferred size did not change.
            self.study_font_key = None;
            {
                let _redraw = crate::reading_control::RedrawBatch::begin(self.controls[&STUDY]);
                self.set_text(
                    STUDY,
                    &format!(
                        "{}\r\n\r\n{}\r\n{}",
                        self.tr("Calculating storage…", "جارٍ حساب مساحة التخزين…"),
                        self.tr("Storage folder", "مجلد التخزين"),
                        cache.folder().display(),
                    ),
                );
                self.format_storage_details();
                SendMessageW(
                    self.controls[&STUDY],
                    EM_SCROLL,
                    Some(WPARAM(SB_TOP.0 as usize)),
                    None,
                );
            }
            self.resize();
            if self.storage_pending.is_none() {
                let (sender, receiver) = std::sync::mpsc::channel();
                std::thread::Builder::new()
                    .name("recitation-storage".into())
                    .stack_size(256 * 1024)
                    .spawn(move || {
                        let _ = sender.send(cache.stats());
                    })?;
                self.storage_pending = Some(receiver);
                self.set_poll_interval(100);
            }
            Ok(())
        }
    }

    pub(super) unsafe fn poll_storage(&mut self) -> Result<()> {
        unsafe {
            let Some(receiver) = &self.storage_pending else {
                return Ok(());
            };
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(()),
                Err(_) => Err("Unable to calculate download storage".into()),
            };
            self.storage_pending = None;
            let stats = result?;
            if self.panel != Panel::Storage {
                return Ok(());
            }
            let cache = self.cache()?;
            let mib = self.tr("MiB", "ميبيبايت");
            let gib = self.tr("GiB", "غيبيبايت");
            let limit = if stats.library_limit == u64::MAX {
                self.tr("No limit", "بلا حد").to_owned()
            } else {
                format!("{} {gib}", stats.library_limit / (1024 * 1024 * 1024))
            };
            let text = format!(
                "{}\r\n{:.1} {mib} {} {:.0} {mib}\r\n\r\n{}\r\n{:.1} {mib}\r\n{}: {limit}\r\n\r\n{}\r\n\r\n{}\r\n\r\n{}\r\n{}",
                self.tr("Temporary playback", "التخزين المؤقت للاستماع"),
                stats.cache_bytes as f64 / 1_048_576.,
                self.tr("of", "من"),
                stats.cache_limit as f64 / 1_048_576.,
                self.tr("Saved recitations", "التلاوات المحفوظة"),
                stats.library_bytes as f64 / 1_048_576.,
                self.tr("Download limit", "حد التنزيل"),
                self.tr(
                    "Online continuous playback streams small parts of the recording. Temporary verse clips can be replaced as you listen.",
                    "يبث الاستماع المتصل أجزاء صغيرة من التسجيل. ويمكن استبدال مقاطع الآيات المؤقتة أثناء الاستماع.",
                ),
                self.tr(
                    "Change the download limit in Options > Listening. Lowering it keeps your saved recitations.",
                    "يمكنك تغيير حد التنزيل من الخيارات ثم الاستماع. تبقى التلاوات المحفوظة عند خفض الحد.",
                ),
                self.tr("Storage folder", "مجلد التخزين"),
                cache.folder().display(),
            );
            let _redraw = crate::reading_control::RedrawBatch::begin(self.controls[&STUDY]);
            self.study_attribution_start = 0;
            self.study_font_key = None;
            self.set_text(STUDY, &text);
            self.format_storage_details();
            Ok(())
        }
    }

    /// Reapply after language/font changes while this panel is active. Ordinary
    /// tafsir color updates need not run this: they change ink, not font metrics.
    pub(super) unsafe fn format_storage_details(&self) {
        if self.panel != Panel::Storage {
            return;
        }
        unsafe {
            let study = self.controls[&STUDY];
            // RichEdit's legacy bidi mode can reverse ASCII date components
            // inside an LTR file path. Use UBA as well as paragraph direction.
            let bidi = BIDIOPTIONS {
                cbSize: std::mem::size_of::<BIDIOPTIONS>() as u32,
                wMask: BOM_UNICODEBIDI as u16,
                wEffects: BOE_UNICODEBIDI as u16,
            };
            SendMessageW(
                study,
                EM_SETBIDIOPTIONS,
                None,
                Some(LPARAM((&bidi as *const BIDIOPTIONS) as isize)),
            );
            // STUDY also displays tafsir attribution. Remove its decoration
            // when reusing the control for a storage report.
            let _ = RemovePropW(study, w!("QuranMushaf.AttributionIndex"));
            let _ = RemovePropW(study, w!("QuranMushaf.AttributionColor"));
            let direction_mask =
                (WS_EX_RTLREADING | WS_EX_RIGHT | WS_EX_LEFTSCROLLBAR | WS_EX_LAYOUTRTL).0 as isize;
            let previous_style = GetWindowLongPtrW(study, GWL_EXSTYLE);
            let style = (previous_style & !direction_mask)
                | if self.prefs.arabic {
                    (WS_EX_RTLREADING | WS_EX_LEFTSCROLLBAR).0 as isize
                } else {
                    0
                };
            if style != previous_style {
                SetWindowLongPtrW(study, GWL_EXSTYLE, style);
                let _ = SetWindowPos(
                    study,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
            let mut previous = CHARRANGE::default();
            SendMessageW(
                study,
                EM_EXGETSEL,
                None,
                Some(LPARAM((&mut previous as *mut CHARRANGE) as isize)),
            );
            SendMessageW(study, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            let paragraph = storage_paragraph(self.prefs.arabic);
            SendMessageW(
                study,
                EM_SETPARAFORMAT,
                None,
                Some(LPARAM((&paragraph as *const PARAFORMAT2) as isize)),
            );
            let body = storage_charformat(
                if self.prefs.arabic { 240 } else { 220 },
                400,
                self.palette().ink_rgb(),
            );
            for scope in [0, SCF_ALL as usize] {
                SendMessageW(
                    study,
                    EM_SETCHARFORMAT,
                    Some(WPARAM(scope)),
                    Some(LPARAM((&body as *const CHARFORMAT2W) as isize)),
                );
            }
            // RichEdit uses one UTF-16 unit per paragraph terminator. Normalize
            // WM_GETTEXT's CR/LF representation before locating format ranges.
            let text = self.text(STUDY).replace("\r\n", "\n").replace('\r', "\n");
            let headings = [
                self.tr("Temporary playback", "التخزين المؤقت للاستماع"),
                self.tr("Saved recitations", "التلاوات المحفوظة"),
                self.tr("Storage folder", "مجلد التخزين"),
            ];
            let heading = storage_charformat(
                if self.prefs.arabic { 240 } else { 220 },
                600,
                self.palette().ink_rgb(),
            );
            let mut offset = 0usize;
            for line in text.split_inclusive('\n') {
                let content = line.trim_end_matches('\n');
                if headings.contains(&content) {
                    storage_select(study, offset, offset + content.encode_utf16().count());
                    SendMessageW(
                        study,
                        EM_SETCHARFORMAT,
                        Some(WPARAM(SCF_SELECTION as usize)),
                        Some(LPARAM((&heading as *const CHARFORMAT2W) as isize)),
                    );
                }
                offset += line.encode_utf16().count();
            }
            if let Some(path_start) = text.rfind('\n') {
                let path = &text[path_start + 1..];
                storage_select(
                    study,
                    text[..path_start + 1].encode_utf16().count(),
                    text.encode_utf16().count(),
                );
                let mut path_paragraph = storage_paragraph(false);
                path_paragraph.dySpaceAfter = 0;
                SendMessageW(
                    study,
                    EM_SETPARAFORMAT,
                    None,
                    Some(LPARAM((&path_paragraph as *const PARAFORMAT2) as isize)),
                );
                if !path.is_empty() {
                    let muted = self.palette().muted;
                    let color = ((muted.r * 255.) as u32)
                        | (((muted.g * 255.) as u32) << 8)
                        | (((muted.b * 255.) as u32) << 16);
                    let path_font = storage_charformat(190, 400, color);
                    SendMessageW(
                        study,
                        EM_SETCHARFORMAT,
                        Some(WPARAM(SCF_SELECTION as usize)),
                        Some(LPARAM((&path_font as *const CHARFORMAT2W) as isize)),
                    );
                }
            }
            SendMessageW(
                study,
                EM_EXSETSEL,
                None,
                Some(LPARAM((&previous as *const CHARRANGE) as isize)),
            );
            crate::reading_control::hide_caret(study);
        }
    }
}

fn storage_paragraph(arabic: bool) -> PARAFORMAT2 {
    PARAFORMAT2 {
        Base: PARAFORMAT {
            cbSize: std::mem::size_of::<PARAFORMAT2>() as u32,
            dwMask: PFM_ALIGNMENT
                | PFM_RTLPARA
                | PFM_LINESPACING
                | PFM_SPACEBEFORE
                | PFM_SPACEAFTER
                | PFM_STARTINDENT
                | PFM_RIGHTINDENT
                | PFM_OFFSET,
            Anonymous: PARAFORMAT_0 {
                // PFE_RTLPARA is PFM_RTLPARA >> 16 in the Windows SDK.
                wEffects: u16::from(arabic),
            },
            wAlignment: if arabic { PFA_RIGHT } else { PFA_LEFT },
            ..Default::default()
        },
        dyLineSpacing: 24,
        dySpaceAfter: 60,
        bLineSpacingRule: 5,
        ..Default::default()
    }
}

fn storage_charformat(size: i32, weight: u16, color: u32) -> CHARFORMAT2W {
    let mut format = CHARFORMAT2W {
        Base: CHARFORMATW {
            cbSize: std::mem::size_of::<CHARFORMAT2W>() as u32,
            dwMask: CFM_FACE | CFM_SIZE | CFM_WEIGHT | CFM_COLOR | CFM_ITALIC,
            yHeight: size,
            crTextColor: COLORREF(color),
            ..Default::default()
        },
        wWeight: weight,
        ..Default::default()
    };
    for (target, unit) in format
        .Base
        .szFaceName
        .iter_mut()
        .zip("Segoe UI".encode_utf16())
    {
        *target = unit;
    }
    format
}

unsafe fn storage_select(study: HWND, start: usize, end: usize) {
    unsafe {
        SendMessageW(
            study,
            EM_SETSEL,
            Some(WPARAM(start)),
            Some(LPARAM(end as isize)),
        );
    }
}
