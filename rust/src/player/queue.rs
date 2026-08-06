use std::collections::VecDeque;
use std::path::PathBuf;

use rand::seq::SliceRandom;

/// How the player chooses a track after the current one ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackMode {
    Sequential,
    Shuffle,
    RepeatOne,
    RepeatAll,
}

/// Which queue the playing track comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuePosition {
    /// Playing the main queue entry at this index.
    Main(usize),
    /// Playing a track popped from the up-next queue.
    ///
    /// `path` keeps the track openable (and repeatable) after it left the
    /// up-next queue; `return_index` is the main-queue anchor to resume from
    /// once the up-next queue runs dry.
    Manual {
        path: PathBuf,
        return_index: Option<usize>,
    },
}

/// Pure queue state shared with callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSnapshot {
    pub main_paths: Vec<PathBuf>,
    pub up_next_paths: Vec<PathBuf>,
    pub current: Option<QueuePosition>,
    pub mode: PlaybackMode,
}

impl QueueSnapshot {
    /// Resolved path of the current track, from either queue.
    pub fn current_path(&self) -> Option<&PathBuf> {
        match &self.current {
            Some(QueuePosition::Main(index)) => self.main_paths.get(*index),
            Some(QueuePosition::Manual { path, .. }) => Some(path),
            None => None,
        }
    }
}

/// Owns queue navigation independently from decoding and output.
///
/// The main queue holds the list order: original in sequential modes, or
/// Fisher–Yates shuffled once (on replace / when switching into shuffle) in
/// [`PlaybackMode::Shuffle`], then advanced sequentially. The up-next queue
/// holds manually requested tracks (`push_back` = play next) and is drained
/// before the main queue advances; it never gets shuffled.
pub(crate) struct PlaybackQueue {
    main_queue: Vec<PathBuf>,
    up_next: VecDeque<PathBuf>,
    current: Option<QueuePosition>,
    mode: PlaybackMode,
    /// After switching into shuffle mid-session, the next advance starts at index 0.
    restart_from_start_on_next: bool,
}

impl PlaybackQueue {
    pub(crate) fn new() -> Self {
        Self {
            main_queue: Vec::new(),
            up_next: VecDeque::new(),
            current: None,
            mode: PlaybackMode::Sequential,
            restart_from_start_on_next: false,
        }
    }

    /// Replace the main queue and select `start_index` as the current track.
    ///
    /// Pending up-next tracks are dropped along with the old list. The clicked
    /// track is captured before any shuffle reorder, so the selection stays
    /// exact even in [`PlaybackMode::Shuffle`].
    pub(crate) fn replace(
        &mut self,
        paths: Vec<PathBuf>,
        start_index: Option<usize>,
    ) -> Option<PathBuf> {
        let clicked = start_index.and_then(|index| paths.get(index)).cloned();
        self.up_next.clear();
        self.restart_from_start_on_next = false;
        self.main_queue = paths;
        if self.mode == PlaybackMode::Shuffle {
            self.shuffle_main();
        }
        self.current = match (start_index, clicked) {
            // Ordered list: the clicked index is already exact (duplicate paths
            // included); shuffle mode must relocate by path after the reorder.
            (Some(index), Some(_)) if self.mode != PlaybackMode::Shuffle => {
                Some(QueuePosition::Main(index))
            }
            (_, Some(path)) => self.locate(&path).map(|position| {
                // Start the shuffled list at the clicked track, so playback
                // still walks every entry exactly once.
                self.main_queue.rotate_left(position);
                QueuePosition::Main(0)
            }),
            _ => None,
        };
        self.current_path().cloned()
    }

    pub(crate) fn select(&mut self, index: usize) -> Option<PathBuf> {
        if index >= self.main_queue.len() {
            return None;
        }
        self.restart_from_start_on_next = false;
        self.current = Some(QueuePosition::Main(index));
        self.current_path().cloned()
    }

    /// Queue a track to play after the current one (FIFO, never shuffled).
    pub(crate) fn enqueue_next(&mut self, path: PathBuf) {
        self.up_next.push_back(path);
    }

    pub(crate) fn set_mode(&mut self, mode: PlaybackMode) {
        let switching_to_shuffle =
            mode == PlaybackMode::Shuffle && self.mode != PlaybackMode::Shuffle;
        self.mode = mode;
        if !matches!(mode, PlaybackMode::Shuffle) {
            self.restart_from_start_on_next = false;
            return;
        }
        if !switching_to_shuffle || self.main_queue.is_empty() {
            return;
        }
        // Anchor positions by path so the playing track and the manual return
        // point survive the reorder.
        let playing_path = self.current_path().cloned();
        let anchor_path = self.main_anchor_path().cloned();
        self.shuffle_main();
        self.current = match self.current.take() {
            Some(QueuePosition::Manual { path, .. }) => Some(QueuePosition::Manual {
                path,
                return_index: anchor_path.and_then(|anchor| self.locate(&anchor)),
            }),
            _ => playing_path
                .and_then(|path| self.locate(&path))
                .map(QueuePosition::Main),
        };
        // Keep highlighting the still-playing file; next advance starts at 0.
        self.restart_from_start_on_next = true;
    }

    pub(crate) fn current_path(&self) -> Option<&PathBuf> {
        match &self.current {
            Some(QueuePosition::Main(index)) => self.main_queue.get(*index),
            Some(QueuePosition::Manual { path, .. }) => Some(path),
            None => None,
        }
    }

    pub(crate) fn next(&mut self, ended: bool) -> Option<PathBuf> {
        // RepeatOne replays the current track on natural end, from either queue.
        if ended && self.mode == PlaybackMode::RepeatOne {
            return self.current_path().cloned();
        }
        // Manually queued tracks always play before the main queue advances.
        if let Some(path) = self.up_next.pop_front() {
            let return_index = self.main_anchor_index();
            self.current = Some(QueuePosition::Manual { path, return_index });
            return self.current_path().cloned();
        }
        self.advance_main()
    }

    pub(crate) fn previous(&mut self) -> Option<PathBuf> {
        self.restart_from_start_on_next = false;
        // Stepping back from a manual track replays the interrupted main entry.
        if let Some(QueuePosition::Manual { return_index, .. }) = &self.current {
            let index = (*return_index).filter(|index| *index < self.main_queue.len())?;
            self.current = Some(QueuePosition::Main(index));
            return self.current_path().cloned();
        }
        if self.main_queue.is_empty() {
            self.current = None;
            return None;
        }
        let current = self
            .main_anchor_index()
            .unwrap_or(0)
            .min(self.main_queue.len() - 1);
        let previous = match self.mode {
            PlaybackMode::RepeatAll if current == 0 => self.main_queue.len() - 1,
            _ if current == 0 => return None,
            _ => current - 1,
        };
        self.current = Some(QueuePosition::Main(previous));
        self.current_path().cloned()
    }

    pub(crate) fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            main_paths: self.main_queue.clone(),
            up_next_paths: self.up_next.iter().cloned().collect(),
            current: self.current.clone(),
            mode: self.mode,
        }
    }

    /// Advance within the main queue from the current anchor position.
    ///
    /// RepeatOne's natural-end replay is the early return in [`Self::next`], so
    /// reaching here with RepeatOne means a manual skip past the current track.
    fn advance_main(&mut self) -> Option<PathBuf> {
        if self.main_queue.is_empty() {
            self.current = None;
            self.restart_from_start_on_next = false;
            return None;
        }
        if self.restart_from_start_on_next {
            self.restart_from_start_on_next = false;
            self.current = Some(QueuePosition::Main(0));
            return self.current_path().cloned();
        }
        let current = self
            .main_anchor_index()
            .unwrap_or(0)
            .min(self.main_queue.len() - 1);
        let next = match self.mode {
            PlaybackMode::RepeatAll => (current + 1) % self.main_queue.len(),
            PlaybackMode::Sequential | PlaybackMode::RepeatOne | PlaybackMode::Shuffle => {
                if current + 1 >= self.main_queue.len() {
                    return None;
                }
                current + 1
            }
        };
        self.current = Some(QueuePosition::Main(next));
        self.current_path().cloned()
    }

    /// Main-queue index playback resumes from once the up-next queue runs dry.
    fn main_anchor_index(&self) -> Option<usize> {
        match &self.current {
            Some(QueuePosition::Main(index)) => Some(*index),
            Some(QueuePosition::Manual { return_index, .. }) => *return_index,
            None => None,
        }
    }

    /// Path of the main-queue anchor entry, if any.
    fn main_anchor_path(&self) -> Option<&PathBuf> {
        self.main_anchor_index()
            .and_then(|index| self.main_queue.get(index))
    }

    /// First main-queue position of `path`.
    fn locate(&self, path: &PathBuf) -> Option<usize> {
        self.main_queue
            .iter()
            .position(|candidate| candidate == path)
    }

    fn shuffle_main(&mut self) {
        self.main_queue.shuffle(&mut rand::rng());
    }
}

impl Default for PlaybackQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(mode: PlaybackMode) -> PlaybackQueue {
        let mut queue = PlaybackQueue::new();
        queue.replace(vec!["a".into(), "b".into(), "c".into()], Some(0));
        queue.set_mode(mode);
        queue
    }

    #[test]
    fn sequential_stops_at_the_end() {
        let mut queue = queue(PlaybackMode::Sequential);
        assert_eq!(queue.next(false), Some("b".into()));
        assert_eq!(queue.next(false), Some("c".into()));
        assert_eq!(queue.next(true), None);
        assert_eq!(queue.snapshot().current, Some(QueuePosition::Main(2)));
    }

    #[test]
    fn repeat_modes_observe_boundaries() {
        let mut all = queue(PlaybackMode::RepeatAll);
        all.select(2);
        assert_eq!(all.next(true), Some("a".into()));
        assert_eq!(all.previous(), Some("c".into()));

        let mut one = queue(PlaybackMode::RepeatOne);
        one.select(1);
        assert_eq!(one.next(true), Some("b".into()));
        assert_eq!(one.next(false), Some("c".into()));
    }

    #[test]
    fn replace_in_sequential_keeps_the_clicked_index() {
        let mut queue = PlaybackQueue::new();
        let selected = queue.replace(vec!["a".into(), "b".into(), "c".into()], Some(2));
        assert_eq!(selected, Some("c".into()));
        assert_eq!(queue.snapshot().current, Some(QueuePosition::Main(2)));
        assert_eq!(queue.previous(), Some("b".into()));
    }

    #[test]
    fn replace_in_shuffle_plays_the_clicked_track() {
        let mut queue = PlaybackQueue::new();
        queue.set_mode(PlaybackMode::Shuffle);
        let selected = queue.replace(vec!["a".into(), "b".into(), "c".into()], Some(2));
        assert_eq!(selected, Some("c".into()));
        assert_eq!(queue.current_path(), Some(&PathBuf::from("c")));
        let snapshot = queue.snapshot();
        assert_eq!(snapshot.main_paths.len(), 3);
        // The clicked track heads the rotated shuffled order.
        assert_eq!(snapshot.current, Some(QueuePosition::Main(0)));
        let second = snapshot.main_paths[1].clone();
        assert_eq!(queue.next(false), Some(second));
    }

    #[test]
    fn switching_to_shuffle_starts_next_from_front() {
        let mut queue = PlaybackQueue::new();
        queue.replace(vec!["a".into(), "b".into(), "c".into()], Some(1));
        assert_eq!(queue.current_path(), Some(&PathBuf::from("b")));

        queue.set_mode(PlaybackMode::Shuffle);
        let snapshot = queue.snapshot();
        assert_eq!(snapshot.mode, PlaybackMode::Shuffle);
        assert_eq!(snapshot.main_paths.len(), 3);
        // Still pointing at the playing path after reorder.
        assert_eq!(queue.current_path(), Some(&PathBuf::from("b")));

        let first = snapshot.main_paths[0].clone();
        assert_eq!(queue.next(false), Some(first));
        assert_eq!(queue.snapshot().current, Some(QueuePosition::Main(0)));
    }

    #[test]
    fn shuffle_advances_sequentially_after_start() {
        let mut queue = PlaybackQueue::new();
        queue.set_mode(PlaybackMode::Shuffle);
        queue.replace(vec!["a".into(), "b".into(), "c".into()], Some(0));
        let paths = queue.snapshot().main_paths.clone();
        // Every track plays exactly once, in shuffled order, then the queue ends.
        let mut seen = vec![queue.current_path().cloned().expect("current track")];
        while let Some(path) = queue.next(false) {
            seen.push(path);
        }
        assert_eq!(seen, paths);
        assert_eq!(queue.next(true), None);
    }

    #[test]
    fn up_next_drains_before_the_main_queue() {
        let mut queue = queue(PlaybackMode::Sequential);
        queue.enqueue_next("x".into());
        queue.enqueue_next("y".into());
        assert_eq!(queue.next(false), Some("x".into()));
        assert_eq!(queue.next(false), Some("y".into()));
        // Up-next exhausted: resume the main queue after the anchor.
        assert_eq!(queue.next(false), Some("b".into()));
        assert_eq!(queue.snapshot().current, Some(QueuePosition::Main(1)));
    }

    #[test]
    fn up_next_drains_into_the_original_shuffle_order() {
        let mut queue = PlaybackQueue::new();
        queue.set_mode(PlaybackMode::Shuffle);
        queue.replace(vec!["a".into(), "b".into(), "c".into()], Some(0));
        let paths = queue.snapshot().main_paths.clone();
        queue.enqueue_next("x".into());
        queue.enqueue_next("y".into());
        assert_eq!(queue.next(false), Some("x".into()));
        // The interrupted main entry stays anchored while manual tracks play.
        assert_eq!(
            queue.snapshot().current,
            Some(QueuePosition::Manual {
                path: "x".into(),
                return_index: Some(0),
            }),
        );
        assert_eq!(queue.next(false), Some("y".into()));
        // Up-next exhausted: resume right after the anchor, order untouched.
        assert_eq!(queue.next(false), Some(paths[1].clone()));
        assert_eq!(queue.snapshot().current, Some(QueuePosition::Main(1)));
        assert_eq!(queue.snapshot().main_paths, paths);
    }

    #[test]
    fn up_next_plays_without_a_main_queue() {
        let mut queue = PlaybackQueue::new();
        queue.enqueue_next("x".into());
        assert_eq!(queue.next(false), Some("x".into()));
        assert_eq!(queue.next(false), None);
    }

    #[test]
    fn previous_from_a_manual_track_returns_to_the_anchor() {
        let mut queue = queue(PlaybackMode::Sequential);
        queue.select(1);
        queue.enqueue_next("x".into());
        assert_eq!(queue.next(false), Some("x".into()));
        assert_eq!(queue.previous(), Some("b".into()));
        assert_eq!(queue.snapshot().current, Some(QueuePosition::Main(1)));
    }

    #[test]
    fn repeat_one_replays_a_manual_track_on_end() {
        let mut queue = queue(PlaybackMode::RepeatOne);
        queue.enqueue_next("x".into());
        assert_eq!(queue.next(false), Some("x".into()));
        assert_eq!(queue.next(true), Some("x".into()));
        // Manual skip still advances past the repeated track.
        assert_eq!(queue.next(false), Some("b".into()));
    }

    #[test]
    fn replace_drops_pending_up_next() {
        let mut queue = queue(PlaybackMode::Sequential);
        queue.enqueue_next("x".into());
        queue.replace(vec!["d".into(), "e".into()], Some(0));
        assert!(queue.snapshot().up_next_paths.is_empty());
        assert_eq!(queue.next(false), Some("e".into()));
    }

    #[test]
    fn set_mode_shuffle_relocates_a_manual_anchor() {
        let mut queue = queue(PlaybackMode::Sequential);
        queue.select(1);
        queue.enqueue_next("x".into());
        assert_eq!(queue.next(false), Some("x".into()));

        queue.set_mode(PlaybackMode::Shuffle);
        // The manual track keeps playing; its anchor still points at "b".
        assert_eq!(queue.current_path(), Some(&PathBuf::from("x")));
        assert_eq!(queue.previous(), Some("b".into()));
    }
}
