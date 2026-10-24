//! What a playback backend has to do. On desktop it is libmpv; on Android and
//! Apple platforms, the system player.
//!
//! The backend only plays what it is told to. Which track comes next is the
//! [`Queue`](crate::queue::Queue)'s call: when a track ends the client asks
//! the queue and loads the answer, and for gapless playback it hands the
//! backend the queue's [`upcoming`](crate::queue::Queue::upcoming) track
//! ahead of time.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use crate::config::ReplayGain;

pub trait Player {
    /// Starts playing `path` from the beginning, replacing whatever played.
    fn load(&mut self, path: &Path) -> Result<(), PlayerError>;

    /// The track to switch to without a gap when the current one ends, or
    /// `None` to stop there. Replaces any earlier call.
    fn queue_next(&mut self, path: Option<&Path>) -> Result<(), PlayerError>;

    fn set_paused(&mut self, paused: bool);

    /// Jumps to `position` from the start of the track.
    fn seek(&mut self, position: Duration);

    /// 0 to 100.
    fn set_volume(&mut self, percent: u8);

    fn set_muted(&mut self, muted: bool);

    fn set_replaygain(&mut self, mode: ReplayGain);

    /// The next thing that happened since the last call, if anything. Clients
    /// call it from their event loop until it returns `None`.
    fn poll_event(&mut self) -> Option<PlayerEvent>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// A track started, after [`Player::load`] or a gapless switch.
    Started {
        gapless: bool,
    },
    /// The current track's length, once the backend knows it. Streams may
    /// never send it, or send it again as it changes.
    Duration(Duration),
    /// Where playback is now, sent a few times a second while playing.
    Position(Duration),
    Paused(bool),
    /// The track played to its end and nothing was queued after it.
    Ended,
    /// The track couldn't be played; the client usually skips to the next one.
    Failed(PlayerError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerError(pub String);

impl fmt::Display for PlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PlayerError {}
