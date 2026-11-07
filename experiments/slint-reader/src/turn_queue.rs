//! Page turns requested while another is running, and how long each one takes.
//! The curve follows the earlier WebView reader. Its 285 ms felt fast here, where
//! the turn starts at once, so a single turn is a little slower by default.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, Ordering};

pub const DEFAULT_TURN_MS: i64 = 380;
const MIN_TURN_MS: i64 = 150;
const MAX_TURN_MS: i64 = 900;
const MIN_RELEASE_MS: i64 = 130;
const MAX_QUEUED: usize = 10;

static TURN_MS: AtomicI64 = AtomicI64::new(DEFAULT_TURN_MS);

/// Set the length of one unhurried turn, for example from `--turn-ms`.
pub fn set_turn_ms(ms: i64) {
    TURN_MS.store(ms.clamp(MIN_TURN_MS, MAX_TURN_MS), Ordering::Relaxed);
}

fn base_ms() -> i64 {
    TURN_MS.load(Ordering::Relaxed)
}

/// Turns waiting behind this one run at the WebView reader's 175:285 ratio.
pub fn turn_ms(more_queued: bool) -> i64 {
    if more_queued {
        base_ms() * 175 / 285
    } else {
        base_ms()
    }
}

/// A released drag only travels what remains of the turn.
pub fn release_ms(progress: f32) -> i64 {
    MIN_RELEASE_MS.max((base_ms() as f32 * (1. - progress.clamp(0., 1.))) as i64)
}

#[derive(Default)]
pub struct TurnQueue {
    entries: VecDeque<(i32, bool)>,
}
impl TurnQueue {
    /// Reversing direction abandons the turns still waiting in the old one.
    pub fn push(&mut self, delta: i32, repeated: bool) {
        if self.entries.back().is_some_and(|(last, _)| *last != delta) {
            self.entries.clear();
        }
        if self.entries.len() < MAX_QUEUED {
            self.entries.push_back((delta, repeated));
        }
    }
    pub fn pop(&mut self) -> Option<i32> {
        self.entries.pop_front().map(|(delta, _)| delta)
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    /// Auto-repeat must not outlive key-up. Deliberate presses stay queued.
    pub fn release_repeats(&mut self) {
        self.entries.retain(|(_, repeated)| !repeated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_release_stops_a_held_key_but_keeps_deliberate_presses() {
        let mut queue = TurnQueue::default();
        queue.push(1, false);
        queue.push(1, true);
        queue.push(1, true);
        queue.push(1, false);
        queue.release_repeats();
        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), None);
    }
    #[test]
    fn reversing_direction_abandons_the_old_direction() {
        let mut queue = TurnQueue::default();
        queue.push(1, false);
        queue.push(1, false);
        queue.push(-1, false);
        assert_eq!(queue.pop(), Some(-1));
        assert!(queue.is_empty());
    }
    #[test]
    fn the_queue_is_bounded() {
        let mut queue = TurnQueue::default();
        for _ in 0..50 {
            queue.push(1, false);
        }
        let mut turns = 0;
        while queue.pop().is_some() {
            turns += 1;
        }
        assert_eq!(turns, MAX_QUEUED);
    }
    #[test]
    fn released_drags_only_animate_the_remaining_travel() {
        assert_eq!(release_ms(0.), DEFAULT_TURN_MS);
        assert_eq!(release_ms(1.), MIN_RELEASE_MS);
        assert!(release_ms(0.25) > release_ms(0.5));
    }
    #[test]
    fn queued_turns_are_quicker_than_a_single_turn() {
        assert_eq!(turn_ms(false), DEFAULT_TURN_MS);
        assert!(turn_ms(true) < turn_ms(false));
        assert_eq!(turn_ms(true), DEFAULT_TURN_MS * 175 / 285);
    }
}
