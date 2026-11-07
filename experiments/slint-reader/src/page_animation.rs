//! Theme presentation and a test-only reference for the retired software fold.
//! Page turns now retain two original textures and animate in the Slint renderer.
use crate::app_core::Rect;
use crate::page_turn;
use std::sync::Arc;

pub struct FoldScene {
    pub plane: [f32; 3],
    pub back: [f32; 3],
}
pub fn fold_scene(
    width: f32,
    height: f32,
    single: bool,
    direction: i32,
    progress: f32,
    top: bool,
) -> FoldScene {
    let w = if single { width } else { width / 2. };
    let x = if single || direction > 0 { 0. } else { w };
    let mut p = progress.clamp(0.001, 1.);
    if p > 0.9 {
        let t = (p - 0.9) / 0.1;
        p = 0.9 + 0.1 * (t + t * t - t * t * t);
    }
    let dx = if direction > 0 {
        2. * w * p
    } else {
        -2. * w * p
    };
    let dy = (p * std::f32::consts::PI).sin() * height.min(w) * 0.30 * if top { 1. } else { -1. };
    let length = dx.hypot(dy).max(0.001);
    let (nx, ny) = (dx / length, dy / length);
    let mx = if direction > 0 {
        x + dx / 2.
    } else {
        x + w + dx / 2.
    };
    let my = if top { dy / 2. } else { height + dy / 2. };
    let paper = page_turn::paper(
        Rect {
            x,
            y: 0.,
            w,
            h: height,
        },
        direction,
        progress,
        top,
    );
    let m = paper.back_transform;
    FoldScene {
        plane: [mx, my, (-nx).atan2(ny).to_degrees()],
        back: [
            m.M11 * x + m.M31,
            m.M12 * x + m.M32,
            m.M12.atan2(m.M11).to_degrees(),
        ],
    }
}

#[derive(Clone)]
pub struct Surface {
    pub pixels: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub source: Rect,
}
impl Surface {
    pub fn compatible(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height
            && [self.source.x, self.source.y, self.source.w, self.source.h]
                == [other.source.x, other.source.y, other.source.w, other.source.h]
            // Include the native result, two prepared faces and in-flight output.
            && self.pixels.len().saturating_mul(6) <= 48 * 1024 * 1024
    }
}

pub fn eased(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    t * t * t * (t * (t * 6. - 15.) + 10.)
}

// Convex polygon intersections are calculated once per scanline, not per pixel.
#[cfg(test)]
fn span(poly: &[(f32, f32)], y: f32) -> Option<(f32, f32)> {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for (a, b) in poly
        .iter()
        .zip(poly.iter().cycle().skip(1))
        .take(poly.len())
    {
        if (a.1 <= y && b.1 > y) || (b.1 <= y && a.1 > y) {
            let x = a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1);
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    (lo < hi).then_some((lo, hi))
}
#[cfg(test)]
fn columns(source: Rect, density: f32, width: u32, lo: f32, hi: f32) -> (usize, usize) {
    let a = ((lo - source.x) * density - 0.5)
        .ceil()
        .clamp(0., width as f32) as usize;
    let b = ((hi - source.x) * density - 0.5)
        .ceil()
        .clamp(0., width as f32) as usize;
    (a.min(b), b)
}

/// Same crease, retained face and readable reverse-face rotation as native.
#[cfg(test)]
pub fn fold(old: &Surface, new: &Surface, area: Rect, direction: i32, p: f32) -> Vec<u8> {
    if p <= 0. {
        return old.pixels.as_ref().clone();
    }
    if p >= 0.995 {
        return new.pixels.as_ref().clone();
    }
    let density = new.width as f32 / new.source.w;
    let leaf = page_turn::leaf_bounds(area, direction);
    let single = crate::app_core::book_geometry(area).single;
    let paper = page_turn::paper(leaf, direction, p, false);
    let m = paper.back_transform;
    let translate = if single {
        0.
    } else if direction > 0 {
        -leaf.w
    } else {
        leaf.w
    };
    let mut result = new.pixels.as_ref().clone();
    let stride = new.width as usize * 4;
    let bounds = page_turn::bounds(area);
    let peer = if direction > 0 {
        (leaf.x + leaf.w, bounds.x + bounds.w)
    } else {
        (bounds.x, leaf.x)
    };
    for row in 0..new.height as usize {
        let y = new.source.y + (row as f32 + 0.5) / density;
        if y < bounds.y || y >= bounds.y + bounds.h {
            continue;
        }
        if !single {
            let (a, b) = columns(new.source, density, new.width, peer.0, peer.1);
            let range = row * stride + a * 4..row * stride + b * 4;
            result[range.clone()].copy_from_slice(&old.pixels[range]);
        }
        if let Some((lo, hi)) = span(&paper.retained, y) {
            let (a, b) = columns(new.source, density, new.width, lo, hi);
            let range = row * stride + a * 4..row * stride + b * 4;
            result[range.clone()].copy_from_slice(&old.pixels[range]);
        }
        if let Some((lo, hi)) = span(&paper.reverse, y) {
            let (a, b) = columns(
                new.source,
                density,
                new.width,
                lo.max(bounds.x),
                hi.min(bounds.x + bounds.w),
            );
            // Advance the inverse-mapped point across each row. Calculating
            // the transform and DIP-to-pixel conversion for every component
            // of every pixel made large folds needlessly expensive.
            let x = new.source.x + (a as f32 + 0.5) / density;
            let px = x - m.M31;
            let py = y - m.M32;
            let mut u = ((m.M22 * px - m.M21 * py - translate) - new.source.x) * density - 0.5;
            let mut v = ((-m.M12 * px + m.M11 * py) - new.source.y) * density - 0.5;
            for col in a..b {
                // Inverse of native's facing * reflection rotation, then undo
                // the peer-to-leaf translation. Determinant is exactly one.
                let sample_u = u.clamp(0., new.width as f32 - 1.);
                let sample_v = v.clamp(0., new.height as f32 - 1.);
                let ix = sample_u as usize;
                let iy = sample_v as usize;
                let jx = (ix + 1).min(new.width as usize - 1);
                let jy = (iy + 1).min(new.height as usize - 1);
                let fx = ((sample_u - ix as f32) * 256.).round() as u32;
                let fy = ((sample_v - iy as f32) * 256.).round() as u32;
                let dest = row * stride + col * 4;
                for channel in 0..3 {
                    let sample =
                        |x: usize, y: usize| u32::from(new.pixels[y * stride + x * 4 + channel]);
                    let top = sample(ix, iy) * (256 - fx) + sample(jx, iy) * fx;
                    let bottom = sample(ix, jy) * (256 - fx) + sample(jx, jy) * fx;
                    result[dest + channel] = ((top * (256 - fy) + bottom * fy + 32768) >> 16) as u8;
                }
                result[dest + 3] = 255;
                u += m.M22;
                v -= m.M12;
            }
        }
    }
    result
}

pub fn blend(old: &Surface, new: &Surface, p: f32, from: u32, to: u32) -> Vec<u8> {
    if p <= 0. {
        return old.pixels.as_ref().clone();
    }
    if p >= 1. {
        return new.pixels.as_ref().clone();
    }
    let a = crate::render::Palette::new(from);
    let b = crate::render::Palette::new(to);
    let readable = a.blend(b, p);
    let correction = [
        (readable.ink.r - (a.ink.r + (b.ink.r - a.ink.r) * p)) * 255.,
        (readable.ink.g - (a.ink.g + (b.ink.g - a.ink.g) * p)) * 255.,
        (readable.ink.b - (a.ink.b + (b.ink.b - a.ink.b) * p)) * 255.,
    ];
    // Recover coverage from the two original grayscale rasters. This changes
    // temporary presentation color, never glyph positions or source content.
    let old_span = 1. / ((a.ink.r - a.paper.r) * 255.);
    let new_span = 1. / ((b.ink.r - b.paper.r) * 255.);
    let mut pixels = Vec::with_capacity(new.pixels.len());
    for (old, new) in old
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(new.pixels.as_chunks::<4>().0.iter())
    {
        let coverage = (((old[0] as f32 - a.paper.r * 255.) * old_span).clamp(0., 1.)
            + ((new[0] as f32 - b.paper.r * 255.) * new_span).clamp(0., 1.))
            * 0.5;
        for channel in 0..3 {
            let base = old[channel] as f32 + (new[channel] as f32 - old[channel] as f32) * p;
            pixels.push(
                (base + correction[channel] * coverage)
                    .round()
                    .clamp(0., 255.) as u8,
            );
        }
        pixels.push(255);
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    fn face(value: u8) -> Surface {
        Surface {
            pixels: Arc::new(vec![value; 64 * 80 * 4]),
            width: 64,
            height: 80,
            source: Rect {
                x: 0.,
                y: 0.,
                w: 64.,
                h: 80.,
            },
        }
    }
    #[test]
    fn original_pixels_are_exact_at_both_landings_in_both_directions() {
        let (old, new) = (face(23), face(201));
        for dir in [-1, 1] {
            assert_eq!(fold(&old, &new, new.source, dir, 0.), *old.pixels);
            assert_eq!(fold(&old, &new, new.source, dir, 1.), *new.pixels);
            let middle = fold(&old, &new, new.source, dir, 0.4);
            assert!(middle.contains(&23) && middle.contains(&201));
        }
    }
    #[test]
    fn rotating_a_uniform_colored_page_preserves_its_color_and_alpha() {
        let rgba = [31, 147, 219, 255];
        let mut page = face(0);
        page.pixels = Arc::new(rgba.repeat(page.width as usize * page.height as usize));
        for direction in [-1, 1] {
            for progress in [0.1, 0.35, 0.5, 0.75, 0.9] {
                let pixels = fold(&page, &page, page.source, direction, progress);
                assert!(pixels.as_chunks::<4>().0.iter().all(|pixel| *pixel == rgba));
            }
        }
    }
    #[test]
    fn timing_is_monotonic_and_has_no_jump_on_key_release() {
        assert_eq!(eased(0.), 0.);
        assert_eq!(eased(1.), 1.);
        let mut previous = 0.;
        for i in 0..=300 {
            let next = eased(i as f32 / 300.);
            assert!(next >= previous - 0.000001);
            previous = next;
        }
    }
    #[test]
    fn theme_transition_never_loses_the_native_ink_against_paper() {
        let surface = |theme| {
            let p = crate::render::Palette::new(theme);
            Surface {
                pixels: Arc::new(
                    [p.paper, p.ink]
                        .iter()
                        .flat_map(|c| {
                            [
                                (c.r * 255.).round() as u8,
                                (c.g * 255.).round() as u8,
                                (c.b * 255.).round() as u8,
                                255,
                            ]
                        })
                        .collect(),
                ),
                width: 2,
                height: 1,
                source: Rect {
                    x: 0.,
                    y: 0.,
                    w: 2.,
                    h: 1.,
                },
            }
        };
        let luminance = |pixel: &[u8]| {
            let linear = |v: u8| {
                let v = v as f32 / 255.;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(pixel[0]) + 0.7152 * linear(pixel[1]) + 0.0722 * linear(pixel[2])
        };
        for from in 0..3 {
            for to in 0..3 {
                let (old, new) = (surface(from), surface(to));
                assert_eq!(blend(&old, &new, 0., from, to), *old.pixels);
                assert_eq!(blend(&old, &new, 1., from, to), *new.pixels);
                for step in 0..=100 {
                    let pixels = blend(&old, &new, step as f32 / 100., from, to);
                    let (paper, ink) = (luminance(&pixels[..4]), luminance(&pixels[4..]));
                    assert!(
                        (paper.max(ink) + 0.05) / (paper.min(ink) + 0.05) >= 3.95,
                        "{from} to {to}, step {step}"
                    );
                }
            }
        }
    }
}
