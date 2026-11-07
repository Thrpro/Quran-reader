//! Platform-neutral wheel accumulation for densely packed navigation lists.
#[derive(Default)]
pub struct WheelRows {
    remainder: i32,
}
impl WheelRows {
    pub const fn new() -> Self {
        Self { remainder: 0 }
    }
    /// One row per notch. Partial/high-resolution packets accumulate instead
    /// of each causing a full-page jump. Positive wheel motion scrolls upward.
    pub fn rows(&mut self, delta: i16) -> i32 {
        let total = self.remainder + i32::from(delta);
        let notches = total / 120;
        self.remainder = total % 120;
        -notches
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_packets_and_direction_changes_do_not_skip_rows() {
        let mut wheel = WheelRows::new();
        assert_eq!(wheel.rows(-40), 0);
        assert_eq!(wheel.rows(-40), 0);
        assert_eq!(wheel.rows(-40), 1);
        assert_eq!(wheel.rows(-240), 2);
        assert_eq!(wheel.rows(60), 0);
        assert_eq!(wheel.rows(-60), 0);
        assert_eq!(wheel.rows(120), -1);
    }
}
