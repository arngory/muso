//! The platform-independent part of muso: play queue, config files and the
//! interface every playback backend implements.
//!
//! Clients (GTK on Linux, Compose on Android, SwiftUI on Apple platforms)
//! hold the state from here and send it commands; they never decide playback
//! order or parse config themselves.

pub mod config;
pub mod input;
pub mod player;
pub mod queue;
