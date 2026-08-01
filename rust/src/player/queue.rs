use std::path::PathBuf;

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
pub(crate) struct PlaybackQueue {
    paths: Vec<PathBuf>,
    current_index: Option<usize>,
    mode: PlaybackMode,
    shuffle_state: u64,
}

impl PlaybackQueue {
    pub(crate) fn new() -> Self {
        Self {
            paths: Vec::new(),
            current_index: None,
            mode: PlaybackMode::Sequential,
            shuffle_state: 0x9e37_79b9_7f4a_7c15,
        }
    }

    pub(crate) fn replace(
        &mut self,
        paths: Vec<PathBuf>,
        start_index: Option<usize>,
    ) -> Option<PathBuf> {
        self.paths = paths;
        self.current_index = start_index.filter(|index| *index < self.paths.len());
        self.current_path().cloned()
    }

    pub(crate) fn select(&mut self, index: usize) -> Option<PathBuf> {
        if index >= self.paths.len() {
            return None;
        }
        self.current_index = Some(index);
        self.current_path().cloned()
    }

    pub(crate) fn set_mode(&mut self, mode: PlaybackMode) {
        self.mode = mode;
    }

    pub(crate) fn current_path(&self) -> Option<&PathBuf> {
        self.current_index.and_then(|index| self.paths.get(index))
    }

    pub(crate) fn next(&mut self, ended: bool) -> Option<PathBuf> {
        if self.paths.is_empty() {
            self.current_index = None;
            return None;
        }
        let current = self.current_index.unwrap_or(0).min(self.paths.len() - 1);
        let next = match self.mode {
            PlaybackMode::RepeatOne if ended => current,
            PlaybackMode::Shuffle => self.next_shuffle_index(current),
            PlaybackMode::RepeatAll => (current + 1) % self.paths.len(),
            PlaybackMode::Sequential | PlaybackMode::RepeatOne => {
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
            return None;
        }
        let current = self.current_index.unwrap_or(0).min(self.paths.len() - 1);
        let previous = match self.mode {
            PlaybackMode::Shuffle => self.next_shuffle_index(current),
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

    fn next_shuffle_index(&mut self, current: usize) -> usize {
        if self.paths.len() <= 1 {
            return 0;
        }
        self.shuffle_state ^= self.shuffle_state << 13;
        self.shuffle_state ^= self.shuffle_state >> 7;
        self.shuffle_state ^= self.shuffle_state << 17;
        let candidate = self.shuffle_state as usize % (self.paths.len() - 1);
        if candidate >= current {
            candidate + 1
        } else {
            candidate
        }
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
    fn shuffle_does_not_repeat_current_when_possible() {
        let mut queue = queue(PlaybackMode::Shuffle);
        for _ in 0..20 {
            let before = queue.snapshot().current_index;
            queue.next(true);
            assert_ne!(queue.snapshot().current_index, before);
        }
    }
}
