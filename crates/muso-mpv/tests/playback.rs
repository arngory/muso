//! Plays generated WAV files through mpv's null audio output. The files are
//! a second or so long: shorter ones end before mpv reports their duration.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use muso_core::player::{Player, PlayerEvent};
use muso_mpv::Mpv;

/// Writes `secs` of silence as 8 kHz mono 16-bit PCM.
fn wav(dir: &Path, name: &str, secs: f64) -> PathBuf {
    let rate = 8000u32;
    let samples = (rate as f64 * secs) as u32;
    let data_len = samples * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.resize(bytes.len() + data_len as usize, 0);
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("muso-mpv-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn player() -> Mpv {
    Mpv::new(&[("ao".into(), "null".into())]).unwrap()
}

/// Events until `last` or a timeout, leaving out position and pause updates.
/// Durations are collected separately, since they can come at any point
/// after the track started.
fn events_until(
    mpv: &mut Mpv,
    last: fn(&PlayerEvent) -> bool,
) -> (Vec<PlayerEvent>, Vec<Duration>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut events = Vec::new();
    let mut durations = Vec::new();
    while Instant::now() < deadline {
        while let Some(e) = mpv.poll_event() {
            if matches!(e, PlayerEvent::Position(_) | PlayerEvent::Paused(_)) {
                continue;
            }
            if let PlayerEvent::Duration(d) = e {
                durations.push(d);
                continue;
            }
            let done = last(&e);
            events.push(e);
            if done {
                return (events, durations);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out; got {events:?}");
}

#[test]
fn plays_a_file_to_the_end() {
    let dir = temp_dir("end");
    let mut mpv = player();
    mpv.load(&wav(&dir, "a.wav", 1.0)).unwrap();
    let (events, durations) = events_until(&mut mpv, |e| *e == PlayerEvent::Ended);
    assert_eq!(
        events,
        [PlayerEvent::Started { gapless: false }, PlayerEvent::Ended]
    );
    let ms = durations.first().expect("a duration").as_millis();
    assert!((990..=1010).contains(&ms), "{ms}");
}

#[test]
fn switches_to_the_queued_track_without_ending() {
    let dir = temp_dir("gapless");
    let mut mpv = player();
    mpv.load(&wav(&dir, "a.wav", 0.6)).unwrap();
    mpv.queue_next(Some(&wav(&dir, "b.wav", 0.6))).unwrap();
    let (events, _) = events_until(&mut mpv, |e| *e == PlayerEvent::Ended);
    assert_eq!(
        events,
        [
            PlayerEvent::Started { gapless: false },
            PlayerEvent::Started { gapless: true },
            PlayerEvent::Ended,
        ]
    );
}

#[test]
fn reports_files_it_cannot_play() {
    let dir = temp_dir("broken");
    let path = dir.join("broken.flac");
    std::fs::write(&path, b"not audio").unwrap();
    let mut mpv = player();
    mpv.load(&path).unwrap();
    let (events, _) = events_until(&mut mpv, |e| matches!(e, PlayerEvent::Failed(_)));
    assert_eq!(events.len(), 1, "{events:?}");
}

#[test]
fn wakeup_socket_fires_on_events() {
    use std::io::Read;
    let dir = temp_dir("wakeup");
    let mut mpv = player();
    let mut socket = mpv.wakeup_socket().unwrap();
    assert!(mpv.wakeup_socket().is_err());
    mpv.load(&wav(&dir, "a.wav", 0.2)).unwrap();
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = [0u8; 64];
    assert!(socket.read(&mut buf).unwrap() > 0);
}

#[test]
fn bad_options_are_reported() {
    let err = Mpv::new(&[("no-such-option".into(), "1".into())])
        .err()
        .unwrap();
    assert!(err.0.contains("no-such-option"), "{err}");
}
