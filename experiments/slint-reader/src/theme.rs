//! Page colours per theme, on every platform. Themes 0 to 3 are the native
//! reader's palettes (a Windows test keeps them identical to render.rs);
//! theme 4, navy night, uses the WebView reader's colours.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: Color,
    pub paper: Color,
    pub ink: Color,
    pub muted: Color,
    pub border: Color,
    pub highlight: Color,
}

fn color((r, g, b): (u8, u8, u8)) -> Color {
    Color {
        r: r as f32 / 255.,
        g: g as f32 / 255.,
        b: b as f32 / 255.,
        a: 1.,
    }
}

impl Palette {
    /// 0 dark gray, 1 light, 2 sepia, 3 pure black, 4 navy night.
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
            // Navy ink has about 18:1 contrast against the page.
            4 => (
                (7, 17, 31),
                (10, 20, 34),
                (251, 253, 255),
                (170, 184, 199),
                (42, 69, 100),
                (26, 58, 80),
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
        Self {
            background: color(background),
            paper: color(paper),
            ink: color(ink),
            muted: color(muted),
            border: color(border),
            highlight: color(highlight),
        }
    }

    /// Colours `t` of the way from this palette to `other`, keeping the ink
    /// readable: a plain mix makes ink and paper the same gray midway between
    /// a dark and a light theme.
    pub fn blend(self, other: Self, t: f32) -> Self {
        let mix = |a: Color, b: Color| Color {
            r: a.r + (b.r - a.r) * t,
            g: a.g + (b.g - a.g) * t,
            b: a.b + (b.b - a.b) * t,
            a: 1.,
        };
        let background = mix(self.background, other.background);
        let paper = mix(self.paper, other.paper);
        let blended_ink = mix(self.ink, other.ink);
        let readability = |ink| contrast(ink, paper).min(contrast(ink, background));
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
            muted: mix(self.muted, other.muted),
            border: mix(self.border, other.border),
            highlight: mix(self.highlight, other.highlight),
        }
    }
}

/// WCAG contrast ratio between two colours.
fn contrast(a: Color, b: Color) -> f32 {
    let luminance = |c: Color| {
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

/// The same colours for the native DirectWrite renderer.
#[cfg(windows)]
impl From<Palette> for crate::render::Palette {
    fn from(p: Palette) -> Self {
        use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
        let c = |c: Color| D2D1_COLOR_F {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        };
        Self {
            background: c(p.background),
            paper: c(p.paper),
            ink: c(p.ink),
            muted: c(p.muted),
            border: c(p.border),
            highlight: c(p.highlight),
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn same(a: Palette, b: crate::render::Palette) -> bool {
        let n: crate::render::Palette = a.into();
        [
            (n.background, b.background),
            (n.paper, b.paper),
            (n.ink, b.ink),
            (n.muted, b.muted),
            (n.border, b.border),
            (n.highlight, b.highlight),
        ]
        .iter()
        .all(|(x, y)| x.r == y.r && x.g == y.g && x.b == y.b && x.a == y.a)
    }

    #[test]
    fn native_themes_match_the_native_renderer() {
        for theme in 0..4 {
            assert!(same(
                Palette::new(theme),
                crate::render::Palette::new(theme)
            ));
            for other in 0..4 {
                for t in [0., 0.25, 0.5, 0.75, 1.] {
                    let ours = Palette::new(theme).blend(Palette::new(other), t);
                    let native = crate::render::Palette::new(theme)
                        .blend(crate::render::Palette::new(other), t);
                    assert!(same(ours, native), "theme {theme} to {other} at {t}");
                }
            }
        }
    }
}
