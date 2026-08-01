use std::path::PathBuf;
use std::time::Duration;

use crossbeam_channel::Sender;

use super::error::PlayerError;

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
    /// Shut down the worker thread.
    Shutdown,
}
