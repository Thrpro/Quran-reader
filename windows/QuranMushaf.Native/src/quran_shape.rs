//! Portable shaping of unchanged source text with the unchanged source font.
//! Positions are font units; rasterization and hit rectangles belong to adapters.
use crate::data::Result;

pub struct Shaper {
    font: harfrust::Font,
}
#[derive(Clone, Debug)]
#[cfg_attr(feature = "qa", derive(serde::Serialize))]
pub struct Glyph {
    pub id: u16,
    pub advance: f32,
    pub x: f32,
    pub y: f32,
    pub cluster: usize,
}
#[cfg(feature = "qa")]
pub fn verify(resources: &mut crate::data::Resources) -> Result<serde_json::Value> {
    let bytes = std::fs::read(
        resources
            .root
            .join("fonts/digital-khatt/DigitalKhattV2.otf"),
    )?;
    let shaper = Shaper::new(bytes)?;
    let mut lines = Vec::new();
    for page in 1..=2 {
        for line in resources
            .page(page)?
            .lines
            .iter()
            .filter(|line| line.kind == "ayah")
        {
            let shaped = shaper.line(&line.text)?;
            lines.push(
                serde_json::json!({"page":page,"line":line.line_number,"text":line.text,
                "unitsPerEm":shaped.units_per_em,"glyphs":shaped.glyphs}),
            );
        }
    }
    Ok(
        serde_json::json!({"scope":"Unmodified opening-page Unicode and original font, natural shaping only","lines":lines}),
    )
}
pub struct Shaped {
    pub glyphs: Vec<Glyph>,
    pub units_per_em: f32,
}
impl Shaper {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > 2 * 1024 * 1024 {
            return Err("Original shaping font exceeds its read limit".into());
        }
        Ok(Self {
            font: harfrust::Font::new(bytes, 0).ok_or("Invalid original shaping font")?,
        })
    }
    pub fn line(&self, text: &str) -> Result<Shaped> {
        if text.is_empty() || text.len() > 16 * 1024 {
            return Err("Source shaping line exceeds its read limit".into());
        }
        let font = harfrust::ShaperFont::new(&self.font);
        let mut buffer = harfrust::Buffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        buffer.set_direction(harfrust::Direction::RightToLeft);
        harfrust::shape(&font, &mut buffer, harfrust::ShapeOptions::default())?;
        if buffer.glyph_infos().len() > 4096 {
            return Err("Source shaping output exceeds its glyph limit".into());
        }
        let mut glyphs = Vec::with_capacity(buffer.glyph_infos().len());
        for (info, position) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
            if info.glyph_id == 0 || !text.is_char_boundary(info.cluster as usize) {
                return Err("Source shaping has a missing glyph or invalid cluster".into());
            }
            glyphs.push(Glyph {
                id: u16::try_from(info.glyph_id)?,
                advance: position.x_advance as f32,
                x: position.x_offset as f32,
                y: position.y_offset as f32,
                cluster: info.cluster as usize,
            });
        }
        Ok(Shaped {
            glyphs,
            units_per_em: self.font.units_per_em() as f32,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_or_oversized_font_input() {
        assert!(Shaper::new(vec![]).is_err());
        assert!(Shaper::new(vec![0; 64]).is_err());
        assert!(Shaper::new(vec![0; 2 * 1024 * 1024 + 1]).is_err());
    }
}
