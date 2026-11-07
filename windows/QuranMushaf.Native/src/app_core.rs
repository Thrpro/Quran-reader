//! Portable application policy. No Windows, rendering or filesystem APIs.
use std::collections::VecDeque;
/// Original bundled study IDs: Arabic explanations 0/1, English translation 2.
/// Installed Arabic books keep their existing IDs starting at 3.
pub fn study_source_ids(arabic: bool, installed: impl IntoIterator<Item = usize>) -> Vec<usize> {
    let mut sources = if arabic { vec![0, 1] } else { vec![2] };
    sources.extend(
        installed
            .into_iter()
            .filter(|id| if arabic { *id < 4 } else { (4..7).contains(id) })
            .map(|id| 3 + id),
    );
    sources
}
pub const PAGE_WIDTH: f32 = 617.;
pub const PAGE_HEIGHT: f32 = 919.;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SearchKind {
    #[default]
    Ayah,
    Surah,
    Page,
    Juz,
    Hizb,
    Rub,
}
pub fn canvas_selection(
    study: bool,
    overlay: bool,
    hover: Option<(u32, u32)>,
    selected: Option<(u32, u32)>,
) -> Option<(u32, u32)> {
    if overlay {
        None
    } else if study {
        selected
    } else {
        hover
    }
}
/// The count includes the first playback; zero means until explicitly stopped.
#[derive(Default)]
pub struct RepeatCount {
    pub remaining: u32,
}
impl RepeatCount {
    pub fn new(total: u32) -> Self {
        Self {
            remaining: total.min(99),
        }
    }
    pub fn completed(&mut self) -> bool {
        if self.remaining == 0 {
            true
        } else if self.remaining > 1 {
            self.remaining -= 1;
            true
        } else {
            false
        }
    }
}
/// Endpoints are complete authentic ayahs. Page scope finishes the final ayah
/// appearing on that physical page, even if its text continues on the next one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListeningScope {
    ThisAyah,
    Page,
    Surah,
    Quran,
}
/// One bounded request for an existing single-surah recording adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListeningChunk {
    pub surah: u32,
    pub first: u32,
    pub last: u32,
    pub count: u32,
}
/// A captured request cannot change when the user edits the listening form.
/// At most one chunk per surah is retained; no full-Quran audio is materialized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListeningPlan {
    chunks: Vec<ListeningChunk>,
}
impl ListeningPlan {
    pub fn new(
        counts: &[u32],
        start: (u32, u32),
        page_end: Option<(u32, u32)>,
        scope: ListeningScope,
    ) -> Result<Self, &'static str> {
        if counts.len() != 114 || counts.iter().any(|count| !(1..=286).contains(count)) {
            return Err("Invalid authentic surah counts");
        }
        let valid = |key: (u32, u32)| {
            (1..=114).contains(&key.0) && key.1 > 0 && key.1 <= counts[(key.0 - 1) as usize]
        };
        if !valid(start) {
            return Err("Invalid starting ayah");
        }
        let end = match scope {
            ListeningScope::ThisAyah => start,
            ListeningScope::Page => page_end.ok_or("The physical page endpoint is unavailable")?,
            ListeningScope::Surah => (start.0, counts[(start.0 - 1) as usize]),
            ListeningScope::Quran => (114, counts[113]),
        };
        if !valid(end) || end < start {
            return Err("Invalid ending ayah");
        }
        let chunks = (start.0..=end.0)
            .map(|surah| {
                let count = counts[(surah - 1) as usize];
                ListeningChunk {
                    surah,
                    first: if surah == start.0 { start.1 } else { 1 },
                    last: if surah == end.0 { end.1 } else { count },
                    count,
                }
            })
            .collect();
        Ok(Self { chunks })
    }
    pub fn chunks(&self) -> &[ListeningChunk] {
        &self.chunks
    }
}
/// Successful decoder completion advances a chapter or repeats the whole plan.
/// The caller owns decoder failures, pause and cancellation; none should invoke
/// complete_current. Finished sessions never restart after later idle polls.
pub struct ListeningSession {
    plan: ListeningPlan,
    cursor: usize,
    repeats: RepeatCount,
    finished: bool,
}
impl ListeningSession {
    pub fn new(plan: ListeningPlan, total_plays: u32) -> Self {
        Self {
            plan,
            cursor: 0,
            repeats: RepeatCount::new(total_plays),
            finished: false,
        }
    }
    pub fn current(&self) -> Option<ListeningChunk> {
        if self.finished {
            None
        } else {
            self.plan.chunks.get(self.cursor).copied()
        }
    }
    pub fn complete_current(&mut self) -> Option<ListeningChunk> {
        if self.finished {
            return None;
        }
        if self.cursor + 1 < self.plan.chunks.len() {
            self.cursor += 1;
        } else if self.repeats.completed() {
            self.cursor = 0;
        } else {
            self.finished = true;
        }
        self.current()
    }
    pub fn finished(&self) -> bool {
        self.finished
    }
}
#[cfg(test)]
mod listening_plan_tests {
    use super::*;

    fn counts() -> [u32; 114] {
        // Untouched source counts for the chapters involved in these fixtures.
        // Other chapters need only a valid stand-in for endpoint validation.
        let mut counts = [1; 114];
        for (surah, count) in [
            (1, 7),
            (2, 286),
            (4, 176),
            (5, 120),
            (112, 4),
            (113, 5),
            (114, 6),
        ] {
            counts[surah - 1] = count;
        }
        counts
    }
    #[test]
    fn physical_page_106_crosses_the_surah_boundary() {
        let plan =
            ListeningPlan::new(&counts(), (4, 176), Some((5, 2)), ListeningScope::Page).unwrap();
        assert_eq!(
            plan.chunks(),
            [
                ListeningChunk {
                    surah: 4,
                    first: 176,
                    last: 176,
                    count: 176
                },
                ListeningChunk {
                    surah: 5,
                    first: 1,
                    last: 2,
                    count: 120
                },
            ]
        );
    }
    #[test]
    fn physical_page_604_and_quran_tail_have_the_same_complete_endpoints() {
        let page =
            ListeningPlan::new(&counts(), (112, 2), Some((114, 6)), ListeningScope::Page).unwrap();
        let quran = ListeningPlan::new(&counts(), (112, 2), None, ListeningScope::Quran).unwrap();
        assert_eq!(page, quran);
        assert_eq!(
            quran.chunks(),
            [
                ListeningChunk {
                    surah: 112,
                    first: 2,
                    last: 4,
                    count: 4
                },
                ListeningChunk {
                    surah: 113,
                    first: 1,
                    last: 5,
                    count: 5
                },
                ListeningChunk {
                    surah: 114,
                    first: 1,
                    last: 6,
                    count: 6
                },
            ]
        );
    }
    #[test]
    fn ayah_surah_and_final_quran_ayah_are_bounded() {
        let ayah = ListeningPlan::new(&counts(), (2, 255), None, ListeningScope::ThisAyah).unwrap();
        assert_eq!(
            ayah.chunks(),
            [ListeningChunk {
                surah: 2,
                first: 255,
                last: 255,
                count: 286
            }]
        );
        let surah = ListeningPlan::new(&counts(), (2, 255), None, ListeningScope::Surah).unwrap();
        assert_eq!(
            surah.chunks(),
            [ListeningChunk {
                surah: 2,
                first: 255,
                last: 286,
                count: 286
            }]
        );
        let final_ayah =
            ListeningPlan::new(&counts(), (114, 6), None, ListeningScope::Quran).unwrap();
        assert_eq!(
            final_ayah.chunks(),
            [ListeningChunk {
                surah: 114,
                first: 6,
                last: 6,
                count: 6
            }]
        );
        let full = ListeningPlan::new(&counts(), (1, 1), None, ListeningScope::Quran).unwrap();
        assert_eq!(full.chunks().len(), 114);
        assert!(
            full.chunks()
                .windows(2)
                .all(|chunks| chunks[0].surah + 1 == chunks[1].surah)
        );
        assert!(
            full.chunks()
                .iter()
                .all(|chunk| chunk.first == 1 && chunk.last == chunk.count)
        );
    }
    #[test]
    fn invalid_or_reversed_endpoints_never_construct_a_partial_plan() {
        for start in [(0, 1), (115, 1), (1, 0), (1, 8), (u32::MAX, u32::MAX)] {
            assert!(ListeningPlan::new(&counts(), start, None, ListeningScope::Quran).is_err());
        }
        for end in [
            None,
            Some((0, 1)),
            Some((115, 1)),
            Some((114, 7)),
            Some((4, 175)),
            Some((3, 1)),
        ] {
            assert!(ListeningPlan::new(&counts(), (4, 176), end, ListeningScope::Page).is_err());
        }
        for malformed in [
            vec![],
            vec![1; 113],
            vec![1; 115],
            vec![0; 114],
            vec![287; 114],
        ] {
            assert!(ListeningPlan::new(&malformed, (1, 1), None, ListeningScope::Quran).is_err());
        }
    }
    #[test]
    fn finite_repeat_restarts_the_whole_captured_plan_and_then_finishes() {
        let mut original_counts = counts();
        let plan =
            ListeningPlan::new(&original_counts, (112, 4), None, ListeningScope::Quran).unwrap();
        // The captured plan cannot borrow edited form inputs or metadata buffers.
        original_counts[113] = 1;
        let edited =
            ListeningPlan::new(&original_counts, (112, 4), None, ListeningScope::Quran).unwrap();
        assert_ne!(plan, edited);
        let mut session = ListeningSession::new(plan, 2);
        let mut played = vec![session.current().unwrap()];
        while let Some(next) = session.complete_current() {
            played.push(next);
        }
        assert_eq!(
            played.iter().map(|chunk| chunk.surah).collect::<Vec<_>>(),
            [112, 113, 114, 112, 113, 114]
        );
        assert_eq!(played[2].last, 6);
        assert_eq!(played[5].last, 6);
        assert!(session.finished());
        assert_eq!(session.current(), None);
        assert_eq!(session.complete_current(), None);
    }
    #[test]
    fn endless_repeat_remains_bounded_and_observation_does_not_advance() {
        let plan = ListeningPlan::new(&counts(), (114, 6), None, ListeningScope::Quran).unwrap();
        let mut session = ListeningSession::new(plan, 0);
        let current = session.current().unwrap();
        for _ in 0..1000 {
            assert_eq!(session.current(), Some(current));
            assert_eq!(session.complete_current(), Some(current));
            assert!(!session.finished());
        }
    }
}
#[cfg(test)]
mod repeat_count_tests {
    use super::*;
    #[test]
    fn total_plays_includes_first_and_terminal_observations_do_not_restart() {
        for count in 1..=99 {
            let mut repeats = RepeatCount::new(count);
            let mut plays = 1;
            while repeats.completed() {
                plays += 1;
            }
            assert_eq!(plays, count);
            assert!(!repeats.completed());
        }
        let mut forever = RepeatCount::new(0);
        for _ in 0..1000 {
            assert!(forever.completed());
        }
    }
}
/// Playback observation is independent of the reader's selected study ayah.
/// A deliberate navigation suspends following until the user rejoins or starts
/// a new range. Repeated polling must never undo that navigation.
#[derive(Default)]
pub struct PlaybackFollow {
    pub current: Option<(u32, u32)>,
    pub following: bool,
}
impl PlaybackFollow {
    pub fn begin(&mut self) {
        self.current = None;
        self.following = true;
    }
    pub fn suspend(&mut self) {
        self.following = false;
    }
    pub fn observe(&mut self, current: Option<(u32, u32)>) -> bool {
        let changed = self.current != current;
        self.current = current;
        changed && self.following
    }
}
#[cfg(test)]
mod playback_follow_tests {
    use super::*;
    #[test]
    fn browsing_survives_same_and_subsequent_ayahs_and_can_rejoin() {
        let mut follow = PlaybackFollow::default();
        follow.begin();
        assert!(follow.observe(Some((4, 12))));
        follow.suspend();
        assert!(!follow.observe(Some((4, 12))));
        assert!(!follow.observe(Some((4, 13))));
        assert_eq!(follow.current, Some((4, 13)));
        follow.begin();
        assert!(follow.observe(Some((4, 13))));
        assert!(!follow.observe(Some((4, 13))));
        assert!(follow.observe(None));
    }
}
#[cfg(test)]
mod study_selection_tests {
    use super::*;
    #[test]
    fn study_is_an_explicit_highlight_without_sticky_clicked_or_audio_highlights() {
        assert_eq!(
            canvas_selection(true, false, Some((3, 40)), Some((3, 39))),
            Some((3, 39))
        );
        assert_eq!(
            canvas_selection(false, false, Some((3, 40)), Some((3, 39))),
            Some((3, 40))
        );
        assert_eq!(canvas_selection(false, false, None, Some((3, 39))), None);
        assert_eq!(
            canvas_selection(true, true, Some((3, 40)), Some((3, 39))),
            None
        );
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}
/// One layout contract for drawing, corner dragging and spread navigation.
/// Source page proportions stay fixed; the joined spread has no artificial gap.
pub struct BookGeometry {
    pub bounds: Rect,
    pub page_width: f32,
    pub single: bool,
}
pub fn book_geometry(area: Rect) -> BookGeometry {
    let single = area.w < 650.;
    let count = if single { 1. } else { 2. };
    let page_width = (area.h.clamp(1., 1120.) * PAGE_WIDTH / PAGE_HEIGHT)
        .min((area.w - 24.).max(1.) / count)
        .max(0.5);
    let height = page_width * PAGE_HEIGHT / PAGE_WIDTH;
    let width = count * page_width;
    BookGeometry {
        bounds: Rect {
            x: area.x + (area.w - width) / 2.,
            y: area.y,
            w: width,
            h: height,
        },
        page_width,
        single,
    }
}
pub fn spread(number: u32) -> (u32, Option<u32>) {
    let right = if number <= 2 {
        1
    } else if number % 2 == 0 {
        number - 1
    } else {
        number
    };
    (right, (right < 604).then_some(right + 1))
}
pub fn destination(page: u32, direction: i32, single: bool) -> u32 {
    if single {
        (page.clamp(1, 604) as i32)
            .saturating_add(direction)
            .clamp(1, 604) as u32
    } else {
        (spread(page.clamp(1, 604)).0 as i32)
            .saturating_add(direction.saturating_mul(2))
            .clamp(1, 603) as u32
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TurnSource {
    Keyboard,
    Discrete,
}
#[derive(Clone, Copy)]
pub struct TurnRequest {
    pub direction: i32,
    pub repeated: bool,
    pub source: TurnSource,
}
#[derive(Default)]
pub struct TurnQueue {
    pending: VecDeque<TurnRequest>,
}
impl TurnQueue {
    pub fn enqueue(&mut self, request: TurnRequest) {
        if self
            .pending
            .back()
            .is_some_and(|last| last.direction != request.direction)
        {
            self.pending.pop_back();
        } else if !(request.repeated
            && self
                .pending
                .iter()
                .any(|item| item.repeated && item.source == request.source))
            && self.pending.len() < 10
        {
            self.pending.push_back(request);
        }
    }
    pub fn release_keyboard(&mut self) {
        self.pending
            .retain(|item| item.source != TurnSource::Keyboard);
    }
    pub fn clear(&mut self) {
        self.pending.clear();
    }
    pub fn next(&mut self) -> Option<TurnRequest> {
        self.pending.pop_front()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn study_language_filters_installed_books_by_their_actual_language() {
        assert_eq!(study_source_ids(false, [0, 2, 4, 6]), [2, 7, 9]);
        assert_eq!(study_source_ids(true, [0, 2, 4, 6]), [0, 1, 3, 5]);
        assert_eq!(study_source_ids(false, []), [2]);
    }
    #[test]
    fn joined_book_preserves_source_aspect_and_fits_reader_at_breakpoints() {
        for width in [130., 322., 649., 650., 928., 1824.] {
            for height in [140., 540., 1037., 1400.] {
                let area = Rect {
                    x: 48.,
                    y: 38.,
                    w: width,
                    h: height,
                };
                let book = book_geometry(area);
                assert_eq!(book.single, width < 650.);
                assert!((book.bounds.h / book.page_width - 919. / 617.).abs() < 0.00001);
                assert!((book.bounds.x + book.bounds.w / 2. - area.x - area.w / 2.).abs() < 0.0001);
                assert!(book.bounds.x >= area.x && book.bounds.y >= area.y);
                assert!(book.bounds.w <= area.w && book.bounds.h <= area.h + 0.0001);
                assert!(
                    (book.bounds.w - book.page_width * if book.single { 1. } else { 2. }).abs()
                        < 0.0001
                );
            }
        }
    }
    #[test]
    fn key_release_preserves_explicit_clicks_and_drops_repeats() {
        let mut q = TurnQueue::default();
        for repeated in [false, true, true] {
            q.enqueue(TurnRequest {
                direction: 1,
                repeated,
                source: TurnSource::Keyboard,
            });
        }
        q.enqueue(TurnRequest {
            direction: 1,
            repeated: false,
            source: TurnSource::Discrete,
        });
        q.release_keyboard();
        assert!(q.next().is_some_and(|r| r.source == TurnSource::Discrete));
        assert!(q.next().is_none());
    }
    #[test]
    fn opposite_requests_cancel_without_accumulating_old_turns() {
        let mut q = TurnQueue::default();
        for direction in [1, -1] {
            q.enqueue(TurnRequest {
                direction,
                repeated: false,
                source: TurnSource::Discrete,
            });
        }
        assert!(q.next().is_none());
        for _ in 0..100 {
            q.enqueue(TurnRequest {
                direction: 1,
                repeated: true,
                source: TurnSource::Keyboard,
            });
        }
        assert!(q.next().is_some());
        assert!(q.next().is_none());
    }
    #[test]
    fn source_spreads_and_extreme_navigation_stay_in_the_mushaf() {
        for n in 1..=604 {
            let (right, left) = spread(n);
            assert!(right % 2 == 1);
            assert_eq!(left, Some(right + 1));
        }
        assert_eq!(destination(604, 1, true), 604);
        assert_eq!(destination(1, -1, false), 1);
        assert_eq!(destination(45, 1, false), 47);
        assert_eq!(destination(45, 1, true), 46);
        assert_eq!(destination(45, i32::MAX, false), 603);
        assert_eq!(destination(45, i32::MIN, true), 1);
    }
}
