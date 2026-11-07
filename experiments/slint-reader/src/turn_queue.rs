//! Page turns requested while another is running, and how long each one takes.
//! Timing follows the earlier WebView reader, whose motion is the visual reference.
use std::collections::VecDeque;

pub const TURN_MS: i64 = 285;
const QUEUED_TURN_MS: i64 = 175;
const MIN_RELEASE_MS: i64 = 130;
const MAX_QUEUED: usize = 10;

pub fn turn_ms(more_queued: bool) -> i64 {
    if more_queued { QUEUED_TURN_MS } else { TURN_MS }
}

/// A released drag only travels what remains of the turn.
pub fn release_ms(progress: f32) -> i64 {
    MIN_RELEASE_MS.max((TURN_MS as f32 * (1. - progress.clamp(0., 1.))) as i64)
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
        assert_eq!(release_ms(0.), TURN_MS);
        assert_eq!(release_ms(1.), MIN_RELEASE_MS);
        assert!(release_ms(0.25) > release_ms(0.5));
    }
}
