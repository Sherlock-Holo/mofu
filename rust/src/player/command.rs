use std::path::PathBuf;
use std::time::Duration;

use flume::Sender;

use super::error::PlayerError;
use super::queue::PlaybackMode;

/// One-shot reply from the decode worker for a command.
pub type CommandReply = Sender<Result<(), PlayerError>>;

/// Control messages sent from the player handle to the decode worker.
#[derive(Debug)]
pub enum PlayerCommand {
    /// Open a local media file and prepare decoding (does not start playback).
    Open { path: PathBuf, reply: CommandReply },
    /// Start or resume playback.
    Play { reply: CommandReply },
    /// Pause playback (keeps the open media).
    Pause { reply: CommandReply },
    /// Stop playback and close the current media.
    Stop { reply: CommandReply },
    /// Seek to an absolute position from the start of the track.
    Seek {
        position: Duration,
        reply: CommandReply,
    },
    /// Seek relative to the current position.
    SeekRelative { delta_ms: i64, reply: CommandReply },
    /// Replace the complete queue and optionally prepare one entry.
    SetQueue {
        paths: Vec<PathBuf>,
        start_index: Option<usize>,
        autoplay: bool,
        reply: CommandReply,
    },
    /// Select and start one queue entry.
    PlayIndex { index: usize, reply: CommandReply },
    /// Append a track to the up-next queue (plays after the current track).
    EnqueueNext { path: PathBuf, reply: CommandReply },
    /// Move to the next queue entry.
    Next { reply: CommandReply },
    /// Move to the previous queue entry.
    Previous { reply: CommandReply },
    /// Change automatic queue navigation.
    SetPlaybackMode {
        mode: PlaybackMode,
        reply: CommandReply,
    },
    /// Change output gain, clamped to the range 0–1.
    SetVolume { volume: f32, reply: CommandReply },
    /// Shut down the worker thread.
    Shutdown,
}
