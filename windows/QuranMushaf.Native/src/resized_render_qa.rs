//! Actual source layouts at non-canonical sizes, plus native raster captures.
use super::*;
impl Renderer {
    pub fn verify_resized_layout(
        &mut self,
        resources: &mut Resources,
        output: &Path,
    ) -> Result<()> {
        let mut rows = 0;
        let mut max_error = 0f32;
        for page in 1..=604 {
            let source = resources.page(page)?;
            for scale in [0.4, 0.75, 1.4] {
                for line in
                    self.page_ink(&source, resources, PAGE_WIDTH * scale, PAGE_HEIGHT * scale)?
                {
                    let original = source
                        .lines
                        .iter()
                        .find(|l| l.line_number == line.line_number)
                        .unwrap();
                    if original.kind != "ayah" {
                        continue;
                    }
                    let local = if let Some(ink) = &line.natural {
                        ink.bounds
                    } else {
                        ink_bounds(
                            &line.layout,
                            Rect {
                                x: 0.,
                                y: 0.,
                                w: line.rect.w / scale,
                                h: line.rect.h / scale,
                            },
                        )?
                    };
                    let left = line.origin.X + local.x * line.scale;
                    let right = left + local.w * line.scale;
                    let error = (line.rect.x - left)
                        .max(right - line.rect.x - line.rect.w)
                        .max(0.);
                    max_error = max_error.max(error);
                    if error > 1.1
                        || line.origin.Y + (local.y + local.h) * line.scale > PAGE_HEIGHT * scale
                    {
                        return Err(format!(
                            "Resized source ink exceeds page: {page}/{} scale {scale}: {error}",
                            line.line_number
                        )
                        .into());
                    }
                    rows += 1;
                }
            }
        }
        let saved = self.contextual;
        let mut captures = vec![];
        for contextual in [false, true] {
            self.contextual = contextual;
            for (width, height, density) in [
                (1276., 904., 1.),
                (960., 720., 1.),
                (700., 900., 1.25),
                (420., 600., 1.5),
                (1920., 1080., 1.),
            ] {
                for page in [1, 39, 603] {
                    let name = format!(
                        "resized-{}-{page}-{}x{}-{}.png",
                        if contextual { "contextual" } else { "natural" },
                        width as u32,
                        height as u32,
                        (density * 96.) as u32
                    );
                    self.snapshot_sized(
                        resources,
                        page,
                        0,
                        &output.join(&name),
                        density,
                        None,
                        width,
                        height,
                    )?;
                    for line in &self.visible {
                        if let Some(hits) = &line.hits {
                            for (bounds, key) in hits {
                                if bounds.x < line.rect.x - 1.1
                                    || bounds.x + bounds.w > line.rect.x + line.rect.w + 1.1
                                {
                                    return Err(format!(
                                        "Scaled native ayah hit region exceeds source row: {key:?}"
                                    )
                                    .into());
                                }
                            }
                        }
                    }
                    captures.push(name);
                }
            }
        }
        self.contextual = saved;
        std::fs::write(
            output.join("resized-rendering.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "passed":true,"pages":604,"scales":[0.4,0.75,1.4],"sourceRows":rows,"maximumInkOverflowDip":max_error,
                "captures":captures,"scaledNativeHitRegionsInsideRows":true,"authenticSourceModified":false,
                "scope":"Canonical source shaping with scaled native ink bounds on all 604 pages; actual natural/contextual raster captures and hit maps at five window/DPI sizes."
            }))?,
        )?;
        Ok(())
    }
}
