//! The platform-independent part of muso: play queue, tags and covers, config
//! files and the interface every playback backend implements.
//!
//! Clients (GTK on Linux, Compose on Android, SwiftUI on Apple platforms)
//! hold the state from here and send it commands; they never decide playback
//! order or parse config themselves.

pub mod config;
pub mod input;
pub mod library;
pub mod player;
pub mod queue;
pub mod track;
#[cfg(target_os = "linux")]
pub mod watch;

#[cfg(test)]
mod testutil;
