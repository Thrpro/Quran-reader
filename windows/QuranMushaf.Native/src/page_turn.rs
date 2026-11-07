//! Page-turn state and paper geometry contain no Quran text or font changes.
use crate::render::Rect;
use std::time::Instant;
#[derive(Clone, Copy)]
pub struct Drag {
    pub old: u32,
    pub new: u32,
    pub direction: i32,
    pub progress: f32,
    pub top: bool,
}
#[derive(Clone, Copy)]
pub struct Turn {
    pub start: Instant,
    pub old: u32,
    pub new: u32,
    pub direction: i32,
    pub from: f32,
    pub to: f32,
    pub top: bool,
    pub first_frame: bool,
}
impl Turn {
    pub fn progress(self) -> (f32, bool) {
        self.progress_at(self.start.elapsed().as_secs_f32())
    }
    pub fn release(&mut self) {
        // Input release discards queued keyboard turns in the caller. Let the
        // leaf already on screen finish its original curve: restarting it with
        // a shorter duration made a tap much faster than holding the same key,
        // and changed velocity abruptly in the middle of a turn.
    }
    fn progress_at(self, elapsed: f32) -> (f32, bool) {
        if self.first_frame {
            return (self.from, false);
        }
        let duration = 0.12 + 0.18 * (self.to - self.from).abs().sqrt();
        let t = (elapsed / duration).clamp(0., 1.);
        let eased = t * t * t * (t * (t * 6. - 15.) + 10.);
        (self.from + (self.to - self.from) * eased, t >= 1.)
    }
}
pub fn bounds(area: Rect) -> Rect {
    crate::app_core::book_geometry(area).bounds
}
/// A spread turns one leaf around its central seam; a narrow view uses its page.
pub fn leaf_bounds(area: Rect, direction: i32) -> Rect {
    let geometry = crate::app_core::book_geometry(area);
    let mut rect = geometry.bounds;
    if !geometry.single {
        rect.w = geometry.page_width;
        if direction < 0 {
            rect.x += geometry.page_width;
        }
    }
    rect
}
/// Split the book rectangle along the crease: the perpendicular bisector of
/// the fixed corner and dragged corner. Reflect the lifted polygon to form
/// the paper's reverse side, rather than sliding a triangular overlay.
type Polygon = Vec<(f32, f32)>;
#[derive(Debug, PartialEq)]
pub struct Paper {
    pub retained: Polygon,
    pub reverse: Polygon,
    pub back_transform: windows_numerics::Matrix3x2,
}
pub fn paper(rect: Rect, direction: i32, progress: f32, top: bool) -> Paper {
    let p = progress.clamp(0.001, 1.);
    // Match position and velocity at the final approach, then settle the
    // geometric corner to rest. The previous .999 clamp left a visible offset
    // just before the authoritative landed surface replaced the reverse face.
    let p = if p > 0.9 {
        let t = (p - 0.9) / 0.1;
        0.9 + 0.1 * (t + t * t - t * t * t)
    } else {
        p
    };
    let (cx, cy) = (
        if direction > 0 {
            rect.x
        } else {
            rect.x + rect.w
        },
        if top { rect.y } else { rect.y + rect.h },
    );
    let sign = if direction > 0 { 1. } else { -1. };
    let lift = (p * std::f32::consts::PI).sin() * rect.h.min(rect.w) * 0.30;
    let dx = sign * rect.w * 2. * p;
    let dy = if top { lift } else { -lift };
    let len = (dx * dx + dy * dy).sqrt();
    let n = (dx / len, dy / len);
    let mid = (cx + dx / 2., cy + dy / 2.);
    let corners = [
        (rect.x, rect.y),
        (rect.x + rect.w, rect.y),
        (rect.x + rect.w, rect.y + rect.h),
        (rect.x, rect.y + rect.h),
    ];
    let distance = |v: (f32, f32)| (v.0 - mid.0) * n.0 + (v.1 - mid.1) * n.1;
    let clip = |positive: bool| {
        let mut out = Vec::new();
        for i in 0..4 {
            let a = corners[i];
            let b = corners[(i + 1) % 4];
            let da = distance(a);
            let db = distance(b);
            let inside = if positive { da >= 0. } else { da <= 0. };
            let next = if positive { db >= 0. } else { db <= 0. };
            if inside {
                out.push(a);
            }
            if inside != next {
                let t = da / (da - db);
                out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            }
        }
        out
    };
    let retained = clip(true);
    let reverse = clip(false)
        .into_iter()
        .map(|v| {
            let d = distance(v);
            (v.0 - 2. * d * n.0, v.1 - 2. * d * n.1)
        })
        .collect();
    let dot = mid.0 * n.0 + mid.1 * n.1;
    let reflection = windows_numerics::Matrix3x2 {
        M11: 1. - 2. * n.0 * n.0,
        M12: -2. * n.0 * n.1,
        M21: -2. * n.0 * n.1,
        M22: 1. - 2. * n.1 * n.1,
        M31: 2. * dot * n.0,
        M32: 2. * dot * n.1,
    };
    // Two reflections compose to a rotation. The source page on the reverse
    // remains readable text, rather than becoming mirrored Arabic glyphs.
    let facing = windows_numerics::Matrix3x2 {
        M11: -1.,
        M22: 1.,
        M31: 2. * rect.x + rect.w,
        ..windows_numerics::Matrix3x2::identity()
    };
    Paper {
        retained,
        reverse,
        back_transform: facing * reflection,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cold_preparation_cannot_spend_the_first_visible_turn() {
        let mut turn = Turn {
            start: Instant::now(),
            old: 1,
            new: 3,
            direction: 1,
            from: 0.,
            to: 1.,
            top: false,
            first_frame: true,
        };
        assert_eq!(turn.progress_at(10.), (0., false));
        let start = turn.start;
        turn.release();
        assert_eq!(turn.start, start);
        assert_eq!(turn.progress_at(10.), (0., false));
        turn.first_frame = false;
        assert!(turn.progress_at(0.15).0 > 0.4);
        assert!(!turn.progress_at(0.299).1);
        assert_eq!(turn.progress_at(0.301), (1., true));
    }
    #[test]
    fn key_release_preserves_phase_and_velocity_throughout_the_turn() {
        for elapsed in [0., 0.02, 0.075, 0.15, 0.225, 0.29] {
            let held = Turn {
                start: Instant::now() - std::time::Duration::from_secs_f32(elapsed),
                old: 1,
                new: 3,
                direction: 1,
                from: 0.,
                to: 1.,
                top: false,
                first_frame: false,
            };
            let mut released = held;
            released.release();
            assert_eq!(released.start, held.start);
            assert_eq!(released.from, held.from);
            // Sampling immediately around release also verifies velocity, not
            // only a coincident position at the point where the key is lifted.
            for offset in [-0.001, 0., 0.001, 0.01] {
                let sample = (elapsed + offset).max(0.);
                assert_eq!(released.progress_at(sample), held.progress_at(sample));
            }
        }
    }
    #[test]
    fn fold_stays_finite_and_changes_with_the_corner() {
        let r = Rect {
            x: 50.,
            y: 38.,
            w: 1000.,
            h: 900.,
        };
        for d in [-1, 1] {
            for p in [0., 0.01, 0.25, 0.5, 0.9, 1.] {
                for top in [false, true] {
                    let Paper {
                        retained: a,
                        reverse: b,
                        back_transform: m,
                    } = paper(r, d, p, top);
                    assert!((m.M11 * m.M22 - m.M12 * m.M21 - 1.).abs() < 0.00001);
                    assert!(
                        a.iter()
                            .chain(&b)
                            .all(|v| v.0.is_finite() && v.1.is_finite())
                    );
                    assert!(a.iter().all(|v| v.0 >= r.x - 0.01
                        && v.0 <= r.x + r.w + 0.01
                        && v.1 >= r.y - 0.01
                        && v.1 <= r.y + r.h + 0.01));
                }
            }
        }
        assert_ne!(paper(r, 1, 0.3, true), paper(r, 1, 0.3, false));
    }
    #[test]
    fn page_bounds_use_the_same_centered_aspect_ratio() {
        let r = bounds(Rect {
            x: 48.,
            y: 38.,
            w: 1824.,
            h: 1037.,
        });
        assert!((r.y - 38.).abs() < 0.01);
        assert!((r.h - 1037.).abs() < 0.01);
        assert!((r.x + r.w / 2. - 960.).abs() < 0.01);
    }
    #[test]
    fn interrupted_animation_starts_at_its_drag_position() {
        let t = Turn {
            first_frame: false,
            start: Instant::now(),
            old: 45,
            new: 47,
            direction: 1,
            from: 0.35,
            to: 0.,
            top: false,
        };
        let (p, finished) = t.progress();
        assert!((p - 0.35).abs() < 0.01);
        assert!(!finished);
    }
    #[test]
    fn leaf_geometry_and_settling_are_bounded_and_monotonic() {
        let area = Rect {
            x: 20.,
            y: 40.,
            w: 1400.,
            h: 900.,
        };
        let book = bounds(area);
        let left = leaf_bounds(area, 1);
        let right = leaf_bounds(area, -1);
        assert!((left.w * 2. - book.w).abs() < 0.001);
        assert!((left.x + left.w - right.x).abs() < 0.001);
        for (from, to) in [(0., 1.), (0.35, 0.), (0.9, 1.)] {
            let turn = Turn {
                first_frame: false,
                start: Instant::now(),
                old: 45,
                new: 47,
                direction: 1,
                from,
                to,
                top: false,
            };
            assert_eq!(turn.progress_at(0.), (from, false));
            assert_eq!(turn.progress_at(1.), (to, true));
            let mut previous = from;
            for i in 1..=100 {
                let (p, _) = turn.progress_at(i as f32 / 100.);
                assert!(p >= from.min(to) - 0.00001 && p <= from.max(to) + 0.00001);
                assert!(if to >= from {
                    p >= previous - 0.00001
                } else {
                    p <= previous + 0.00001
                });
                previous = p;
            }
        }
    }
}
