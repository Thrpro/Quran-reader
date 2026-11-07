//! Original opening glyph captures; no verse text or recording timing is synthesized.
use super::*;
impl App {
    pub(super) fn qa_opening26(&mut self, out: &std::path::Path) -> Result<()> {
        let show_clock = self.renderer.show_clock;
        self.renderer.show_clock = false;
        let result = (|| -> Result<()> {
            let mut cases = vec![];
            for theme in [0, 1] {
                for page in [2, 604] {
                    let mut normalized = BTreeMap::new();
                    for (width, height) in [(1600., 1100.), (1000., 760.)] {
                        let base = format!("opening26-base-{theme}-{page}-{width}.png");
                        self.renderer.snapshot_sized(
                            &mut self.resources,
                            page,
                            theme,
                            &out.join(&base),
                            1.,
                            None,
                            width,
                            height,
                        )?;
                        let page_width = crate::app_core::book_geometry(Rect {
                            x: 0.,
                            y: 12.,
                            w: width,
                            h: height - 24.,
                        })
                        .page_width;
                        for surah in if page == 2 {
                            vec![2, 9]
                        } else {
                            vec![112, 113, 114, 9, 1]
                        } {
                            let file = format!("opening26-{theme}-{page}-{surah}-{width}.png");
                            let rects = if width == 1600. {
                                self.renderer.snapshot_opening(
                                    &mut self.resources,
                                    page,
                                    theme,
                                    &out.join(&file),
                                    surah,
                                )?
                            } else {
                                self.renderer.snapshot_opening_sized(
                                    &mut self.resources,
                                    page,
                                    theme,
                                    &out.join(&file),
                                    surah,
                                    width,
                                    height,
                                )?
                            };
                            let expected = if (page == 2 && surah == 2)
                                || (page == 604 && (112..=114).contains(&surah))
                            {
                                1
                            } else {
                                0
                            };
                            if rects.len() != expected {
                                return Err(
                                    "Opening highlight targeted a different source header".into()
                                );
                            }
                            if let Some(rect) = rects.first() {
                                let size =
                                    ((rect.w - 10.) / page_width, (rect.h - 6.) / page_width);
                                if let Some((w, h)) = normalized.insert(surah, size) {
                                    if (w - size.0).abs() > 0.0001 || (h - size.1).abs() > 0.0001 {
                                        return Err("Opening highlight scaled the original ink more than once".into());
                                    }
                                }
                            }
                            cases.push(serde_json::json!({"theme":theme,"page":page,"surah":surah,"width":width,"height":height,"base":base,"file":file,"rects":rects.iter().map(|r| [r.x,r.y,r.w,r.h]).collect::<Vec<_>>(),"expectedPaintedOpenings":expected,"canonicalInkScaleChecked":true}));
                        }
                    }
                }
            }
            std::fs::write(
                out.join("opening-render26.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"cases":cases,"scope":"Original source glyph snapshots at two viewport sizes with a fixed clock; opening selection is a QA presentation fixture, not a claim of full-recording Bismillah timing. Native clip identity is checked separately."}),
                )?,
            )?;
            Ok(())
        })();
        self.renderer.show_clock = show_clock;
        result
    }
}
