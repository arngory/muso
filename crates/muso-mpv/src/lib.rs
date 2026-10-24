//! [`Player`] on top of libmpv, for desktop clients.
//!
//! mpv only ever holds the current track and, for gapless playback, the one
//! after it. The rest of the queue lives in [`muso_core::queue`].

#![allow(non_camel_case_types)]

mod ffi;

use std::ffi::{CStr, CString, c_int, c_void};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::ptr::{self, NonNull};
use std::time::Duration;

use muso_core::config::ReplayGain;
use muso_core::player::{Player, PlayerError, PlayerEvent};
use muso_core::track::Formats;

const OBSERVE_TIME_POS: u64 = 1;
const OBSERVE_PAUSE: u64 = 2;
const OBSERVE_DURATION: u64 = 3;

/// Position events closer together than this are dropped; mpv reports the
/// position for every audio chunk.
const POSITION_STEP: Duration = Duration::from_millis(250);

pub struct Mpv {
    handle: NonNull<ffi::mpv_handle>,
    /// A track is waiting in mpv's playlist for a gapless switch.
    next_queued: bool,
    /// [`Player::load`] was called and its file hasn't started yet.
    loading: bool,
    last_position: Option<Duration>,
    /// Read when the file loaded, sent with the next poll.
    pending_duration: Option<Duration>,
    /// Kept alive for mpv's wakeup callback, which writes to it.
    wakeup: Option<Box<UnixStream>>,
}

// mpv handles can be used from any thread.
unsafe impl Send for Mpv {}

impl Mpv {
    /// Starts mpv for audio only. `options` are mpv options (the `mpv-*`
    /// lines of `muso.conf`), applied over muso's own defaults.
    pub fn new(options: &[(String, String)]) -> Result<Mpv, PlayerError> {
        let major = unsafe { ffi::mpv_client_api_version() } >> 16;
        if major != 2 {
            return Err(PlayerError(format!(
                "libmpv client API {major}.x is not supported, muso needs 2.x"
            )));
        }
        let handle = NonNull::new(unsafe { ffi::mpv_create() })
            .ok_or_else(|| PlayerError("mpv_create failed".into()))?;
        // Built before anything can fail, so Drop cleans up the handle.
        let mut mpv = Mpv {
            handle,
            next_queued: false,
            loading: false,
            last_position: None,
            pending_duration: None,
            wakeup: None,
        };
        let defaults = [
            ("vid", "no"),
            ("audio-display", "no"),
            ("idle", "yes"),
            ("terminal", "no"),
            ("audio-client-name", "muso"),
        ];
        for (name, value) in defaults {
            mpv.set_option(name, value)?;
        }
        for (name, value) in options {
            mpv.set_option(name, value)?;
        }
        check(unsafe { ffi::mpv_initialize(mpv.handle.as_ptr()) })?;
        mpv.observe(OBSERVE_TIME_POS, c"time-pos", ffi::MPV_FORMAT_DOUBLE)?;
        mpv.observe(OBSERVE_PAUSE, c"pause", ffi::MPV_FORMAT_FLAG)?;
        mpv.observe(OBSERVE_DURATION, c"duration", ffi::MPV_FORMAT_DOUBLE)?;
        Ok(mpv)
    }

    /// A socket that becomes readable whenever mpv has events, so a client
    /// can watch it in its event loop and call
    /// [`poll_event`](Player::poll_event) only then. Read and discard what
    /// arrives before polling. Can be called once.
    pub fn wakeup_socket(&mut self) -> io::Result<UnixStream> {
        if self.wakeup.is_some() {
            return Err(io::Error::other("the wakeup socket was already taken"));
        }
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        let writer = Box::new(writer);
        let data = &*writer as *const UnixStream as *mut c_void;
        unsafe {
            ffi::mpv_set_wakeup_callback(self.handle.as_ptr(), Some(wake), data);
        }
        self.wakeup = Some(writer);
        Ok(reader)
    }

    /// What mpv plays and takes as cover art: its `audio-exts`, `image-exts`
    /// and `cover-art-whitelist`, or mpv's defaults for them (`formats.conf`)
    /// on versions before 0.39, which don't have these options.
    pub fn formats(&mut self) -> Formats {
        let read = |mpv: &mut Mpv, name: &CStr| mpv.string_property(name).map(|v| split_list(&v));
        match read(self, c"audio-exts") {
            Some(audio) => Formats {
                audio,
                image: read(self, c"image-exts").unwrap_or_default(),
                cover_names: read(self, c"cover-art-whitelist").unwrap_or_default(),
            },
            None => {
                let defaults = include_str!("formats.conf");
                let list = |name: &str| {
                    defaults
                        .lines()
                        .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
                        .map(split_list)
                        .unwrap_or_default()
                };
                Formats {
                    audio: list("audio-exts"),
                    image: list("image-exts"),
                    cover_names: list("cover-art-whitelist"),
                }
            }
        }
    }

    fn string_property(&mut self, name: &CStr) -> Option<String> {
        let mut value: *mut std::ffi::c_char = ptr::null_mut();
        let r = unsafe {
            ffi::mpv_get_property(
                self.handle.as_ptr(),
                name.as_ptr(),
                ffi::MPV_FORMAT_STRING,
                &mut value as *mut _ as *mut c_void,
            )
        };
        if r < 0 || value.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::mpv_free(value as *mut c_void) };
        Some(s)
    }

    fn set_option(&mut self, name: &str, value: &str) -> Result<(), PlayerError> {
        let (n, v) = (cstring(name)?, cstring(value)?);
        check(unsafe { ffi::mpv_set_option_string(self.handle.as_ptr(), n.as_ptr(), v.as_ptr()) })
            .map_err(|e| PlayerError(format!("mpv option {name}={value}: {e}")))
    }

    fn set_property(&mut self, name: &CStr, value: &str) {
        let v = cstring(value).expect("property values have no NUL");
        // Only fails for values muso doesn't send.
        unsafe { ffi::mpv_set_property_string(self.handle.as_ptr(), name.as_ptr(), v.as_ptr()) };
    }

    fn observe(&mut self, id: u64, name: &CStr, format: c_int) -> Result<(), PlayerError> {
        check(unsafe { ffi::mpv_observe_property(self.handle.as_ptr(), id, name.as_ptr(), format) })
    }

    fn command(&mut self, args: &[&[u8]]) -> Result<(), PlayerError> {
        let owned = args
            .iter()
            .map(|a| CString::new(*a).map_err(|_| PlayerError("argument has a NUL byte".into())))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ptrs: Vec<*const _> = owned.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(ptr::null());
        check(unsafe { ffi::mpv_command(self.handle.as_ptr(), ptrs.as_mut_ptr()) })
    }

    fn duration(&mut self) -> Option<Duration> {
        let mut secs = 0.0f64;
        let r = unsafe {
            ffi::mpv_get_property(
                self.handle.as_ptr(),
                c"duration".as_ptr(),
                ffi::MPV_FORMAT_DOUBLE,
                &mut secs as *mut f64 as *mut c_void,
            )
        };
        if r < 0 {
            return None;
        }
        Duration::try_from_secs_f64(secs).ok()
    }

    fn position_event(&mut self, secs: f64) -> Option<PlayerEvent> {
        let position = Duration::try_from_secs_f64(secs).ok()?;
        let due = match self.last_position {
            None => true,
            Some(last) => position < last || position - last >= POSITION_STEP,
        };
        due.then(|| {
            self.last_position = Some(position);
            PlayerEvent::Position(position)
        })
    }
}

impl Player for Mpv {
    fn load(&mut self, path: &Path) -> Result<(), PlayerError> {
        self.command(&[b"loadfile", path.as_os_str().as_bytes(), b"replace"])?;
        // `replace` also drops a track queued for gapless playback.
        self.next_queued = false;
        self.loading = true;
        self.last_position = None;
        self.pending_duration = None;
        Ok(())
    }

    fn queue_next(&mut self, path: Option<&Path>) -> Result<(), PlayerError> {
        // Clears everything but the current file.
        self.command(&[b"playlist-clear"])?;
        self.next_queued = false;
        if let Some(path) = path {
            self.command(&[b"loadfile", path.as_os_str().as_bytes(), b"append"])?;
            self.next_queued = true;
        }
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) {
        self.set_property(c"pause", yes_no(paused));
    }

    fn seek(&mut self, position: Duration) {
        let secs = format!("{:.3}", position.as_secs_f64());
        // Fails only when nothing is playing, which is fine to ignore.
        let _ = self.command(&[b"seek", secs.as_bytes(), b"absolute"]);
    }

    fn set_volume(&mut self, percent: u8) {
        self.set_property(c"volume", &percent.min(100).to_string());
    }

    fn set_muted(&mut self, muted: bool) {
        self.set_property(c"mute", yes_no(muted));
    }

    fn set_replaygain(&mut self, mode: ReplayGain) {
        let mode = match mode {
            ReplayGain::Off => "no",
            ReplayGain::Track => "track",
            ReplayGain::Album => "album",
        };
        self.set_property(c"replaygain", mode);
    }

    fn poll_event(&mut self) -> Option<PlayerEvent> {
        if let Some(d) = self.pending_duration.take() {
            return Some(PlayerEvent::Duration(d));
        }
        loop {
            let event = unsafe { &*ffi::mpv_wait_event(self.handle.as_ptr(), 0.0) };
            match event.event_id {
                ffi::MPV_EVENT_NONE | ffi::MPV_EVENT_SHUTDOWN => return None,
                ffi::MPV_EVENT_FILE_LOADED => {
                    let gapless = !self.loading;
                    if gapless {
                        // mpv moved on to the queued track by itself.
                        self.next_queued = false;
                    }
                    self.loading = false;
                    self.last_position = None;
                    // The observed duration can be coalesced away when a
                    // short file ends quickly, so read it now as well.
                    self.pending_duration = self.duration();
                    return Some(PlayerEvent::Started { gapless });
                }
                ffi::MPV_EVENT_END_FILE => {
                    let end = unsafe { &*(event.data as *const ffi::mpv_event_end_file) };
                    match end.reason {
                        ffi::MPV_END_FILE_REASON_EOF if !self.next_queued => {
                            return Some(PlayerEvent::Ended);
                        }
                        ffi::MPV_END_FILE_REASON_ERROR => {
                            if !self.loading {
                                // The failed file was the queued one.
                                self.next_queued = false;
                            }
                            self.loading = false;
                            return Some(PlayerEvent::Failed(PlayerError(error_string(end.error))));
                        }
                        _ => {}
                    }
                }
                ffi::MPV_EVENT_PROPERTY_CHANGE => {
                    let prop = unsafe { &*(event.data as *const ffi::mpv_event_property) };
                    if prop.data.is_null() {
                        continue;
                    }
                    match (event.reply_userdata, prop.format) {
                        (OBSERVE_TIME_POS, ffi::MPV_FORMAT_DOUBLE) => {
                            let secs = unsafe { *(prop.data as *const f64) };
                            if let Some(e) = self.position_event(secs) {
                                return Some(e);
                            }
                        }
                        (OBSERVE_DURATION, ffi::MPV_FORMAT_DOUBLE) => {
                            let secs = unsafe { *(prop.data as *const f64) };
                            if let Ok(d) = Duration::try_from_secs_f64(secs) {
                                return Some(PlayerEvent::Duration(d));
                            }
                        }
                        (OBSERVE_PAUSE, ffi::MPV_FORMAT_FLAG) => {
                            let paused = unsafe { *(prop.data as *const c_int) } != 0;
                            return Some(PlayerEvent::Paused(paused));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // Waits for mpv's threads, so the wakeup socket outlives every callback.
        unsafe { ffi::mpv_terminate_destroy(self.handle.as_ptr()) };
    }
}

unsafe extern "C" fn wake(data: *mut c_void) {
    let writer = unsafe { &*(data as *const UnixStream) };
    // A full socket already means "wake up", so a failed write is fine.
    let _ = (&*writer).write(&[1]);
}

fn check(code: c_int) -> Result<(), PlayerError> {
    if code < 0 {
        Err(PlayerError(error_string(code)))
    } else {
        Ok(())
    }
}

fn error_string(code: c_int) -> String {
    let s = unsafe { CStr::from_ptr(ffi::mpv_error_string(code)) };
    s.to_string_lossy().into_owned()
}

fn cstring(s: &str) -> Result<CString, PlayerError> {
    CString::new(s).map_err(|_| PlayerError(format!("{s:?} has a NUL byte")))
}

/// mpv's string lists come comma-separated.
fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect()
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
