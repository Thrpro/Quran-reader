//! Portable layout and timeline arithmetic. No native controls or media APIs.
pub fn progress_label(continuous: bool, arabic: bool, elapsed: &str, total: &str) -> String {
    if continuous {
        format!("{elapsed} / {total}")
    } else {
        format!(
            "{} {elapsed} / {total}",
            if arabic { "الآية" } else { "Ayah" }
        )
    }
}
pub fn clock(milliseconds: u64) -> String {
    let s = milliseconds / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
#[derive(Clone, Copy)]
pub struct ListenLayout;
impl ListenLayout {
    pub fn new(_height: f32) -> Self {
        Self
    }
    pub fn y(self, normal: f32) -> f32 {
        normal
    }
    pub fn button_height(self) -> f32 {
        34.
    }
    pub fn bottom(self, active: bool) -> f32 {
        if active {
            self.y(500.) + 150.
        } else {
            self.y(472.) + 22.
        }
    }
    pub fn scroll_max(self, height: f32, active: bool) -> f32 {
        (self.bottom(active) - (height - 35.)).max(0.)
    }
}
/// Native limited items expose a range-relative position, in milliseconds.
/// Leave a small decoder tail when seeking so an edit cannot finish a range.
pub fn seek_target(position: u64, duration: u64, delta: i64) -> u64 {
    position
        .saturating_add_signed(delta)
        .min(duration.saturating_sub(50))
}
pub fn slider_target(value: u32, duration: u64, offset: u64, range: u64) -> u64 {
    let absolute = ((duration as u128 * value.min(10000) as u128) / 10000) as u64;
    absolute
        .saturating_sub(offset)
        .min(range.saturating_sub(50))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_only_scrolls_when_its_actual_content_exceeds_the_viewport() {
        for height in [720., 800., 900., 1080.] {
            assert!(ListenLayout::new(height).scroll_max(height, true) < 0.01);
        }
        assert!(ListenLayout::new(500.).scroll_max(500., true) > 0.);
        assert!(
            ListenLayout::new(500.).scroll_max(500., false)
                < ListenLayout::new(500.).scroll_max(500., true)
        );
    }
    #[test]
    fn chapter_clocks_show_hours_without_losing_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(59999), "0:59");
        assert_eq!(clock(7247412), "2:00:47");
    }
    #[test]
    fn seeking_is_bounded_to_the_active_original_range() {
        assert_eq!(seek_target(2000, 60000, -10000), 0);
        assert_eq!(seek_target(58000, 60000, 10000), 59950);
        assert_eq!(seek_target(u64::MAX, 60000, i64::MAX), 59950);
        assert_eq!(slider_target(1000, 100000, 20000, 30000), 0);
        assert_eq!(slider_target(4000, 100000, 20000, 30000), 20000);
        assert_eq!(slider_target(u32::MAX, u64::MAX, 0, 30000), 29950);
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    #[test]
    fn full_surah_clocks_are_one_line_and_clip_clocks_are_explicit() {
        assert_eq!(
            progress_label(true, true, "1:03:24", "2:00:47"),
            "1:03:24 / 2:00:47"
        );
        assert_eq!(
            progress_label(false, true, "0:15", "0:18"),
            "الآية 0:15 / 0:18"
        );
    }
}
