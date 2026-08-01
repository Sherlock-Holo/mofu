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

/// Pure queue state shared with callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSnapshot {
    pub paths: Vec<PathBuf>,
    pub current_index: Option<usize>,
    pub mode: PlaybackMode,
}

/// Owns queue navigation independently from decoding and output.
///
/// In [`PlaybackMode::Shuffle`], the path list is Fisher–Yates shuffled once
/// (on queue replace or when switching into shuffle), then advanced sequentially.
pub(crate) struct PlaybackQueue {
    paths: Vec<PathBuf>,
    current_index: Option<usize>,
    mode: PlaybackMode,
    /// After switching into shuffle mid-session, the next advance starts at index 0.
    restart_from_start_on_next: bool,
}

impl PlaybackQueue {
    pub(crate) fn new() -> Self {
        Self {
            paths: Vec::new(),
            current_index: None,
            mode: PlaybackMode::Sequential,
            restart_from_start_on_next: false,
        }
    }

    pub(crate) fn replace(
        &mut self,
        paths: Vec<PathBuf>,
        start_index: Option<usize>,
    ) -> Option<PathBuf> {
        self.paths = paths;
        self.restart_from_start_on_next = false;
        if self.mode == PlaybackMode::Shuffle {
            self.shuffle_paths();
            // Shuffled order is the play order; always start from the front.
            self.current_index = if self.paths.is_empty() { None } else { Some(0) };
        } else {
            self.current_index = start_index.filter(|index| *index < self.paths.len());
        }
        self.current_path().cloned()
    }

    pub(crate) fn select(&mut self, index: usize) -> Option<PathBuf> {
        if index >= self.paths.len() {
            return None;
        }
        self.restart_from_start_on_next = false;
        self.current_index = Some(index);
        self.current_path().cloned()
    }

    pub(crate) fn set_mode(&mut self, mode: PlaybackMode) {
        let switching_to_shuffle =
            mode == PlaybackMode::Shuffle && self.mode != PlaybackMode::Shuffle;
        self.mode = mode;
        if !matches!(mode, PlaybackMode::Shuffle) {
            self.restart_from_start_on_next = false;
            return;
        }
        if !switching_to_shuffle || self.paths.is_empty() {
            return;
        }
        let current_path = self.current_path().cloned();
        self.shuffle_paths();
        // Keep highlighting the still-playing file; next advance starts at 0.
        self.current_index = current_path
            .and_then(|path| self.paths.iter().position(|candidate| candidate == &path));
        self.restart_from_start_on_next = true;
    }

    pub(crate) fn current_path(&self) -> Option<&PathBuf> {
        self.current_index.and_then(|index| self.paths.get(index))
    }

    pub(crate) fn next(&mut self, ended: bool) -> Option<PathBuf> {
        if self.paths.is_empty() {
            self.current_index = None;
            self.restart_from_start_on_next = false;
            return None;
        }
        let current = self.current_index.unwrap_or(0).min(self.paths.len() - 1);
        let next = match self.mode {
            PlaybackMode::RepeatOne if ended => current,
            PlaybackMode::RepeatAll => (current + 1) % self.paths.len(),
            PlaybackMode::Sequential | PlaybackMode::RepeatOne | PlaybackMode::Shuffle => {
                if current + 1 >= self.paths.len() {
                    return None;
                }
                current + 1
            }
        };
        self.current_index = Some(next);
        self.current_path().cloned()
    }

    pub(crate) fn previous(&mut self) -> Option<PathBuf> {
        if self.paths.is_empty() {
            self.current_index = None;
            self.restart_from_start_on_next = false;
            return None;
        }
        self.restart_from_start_on_next = false;
        let current = self.current_index.unwrap_or(0).min(self.paths.len() - 1);
        let previous = match self.mode {
            PlaybackMode::RepeatAll if current == 0 => self.paths.len() - 1,
            _ if current == 0 => return None,
            _ => current - 1,
        };
        self.current_index = Some(previous);
        self.current_path().cloned()
    }

    pub(crate) fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            paths: self.paths.clone(),
            current_index: self.current_index,
            mode: self.mode,
        }
    }

    fn shuffle_paths(&mut self) {
        self.paths.shuffle(&mut rand::rng());
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
        assert_eq!(queue.snapshot().current_index, Some(2));
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
    fn replace_in_shuffle_starts_at_front() {
        let mut queue = PlaybackQueue::new();
        queue.set_mode(PlaybackMode::Shuffle);
        let selected = queue.replace(vec!["a".into(), "b".into(), "c".into()], Some(2));
        let snapshot = queue.snapshot();
        assert_eq!(snapshot.current_index, Some(0));
        assert_eq!(selected.as_ref(), snapshot.paths.first());
        // After shuffle, sequential next walks the (possibly reordered) list.
        let second = snapshot.paths[1].clone();
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
        assert_eq!(snapshot.paths.len(), 3);
        // Still pointing at the playing path after reorder.
        assert_eq!(queue.current_path(), Some(&PathBuf::from("b")));

        let first = snapshot.paths[0].clone();
        assert_eq!(queue.next(false), Some(first));
        assert_eq!(queue.snapshot().current_index, Some(0));
    }

    #[test]
    fn shuffle_advances_sequentially_after_start() {
        let mut queue = PlaybackQueue::new();
        queue.set_mode(PlaybackMode::Shuffle);
        queue.replace(vec!["a".into(), "b".into(), "c".into()], Some(0));
        let paths = queue.snapshot().paths.clone();
        assert_eq!(queue.next(false), Some(paths[1].clone()));
        assert_eq!(queue.next(false), Some(paths[2].clone()));
        assert_eq!(queue.next(true), None);
    }
}
