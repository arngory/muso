//! Noticing changes under the library folders, through Linux's inotify.
//! A watcher only says that something changed, not what: a library scan
//! finds that out cheaply by comparing file times and sizes.

use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::raw::{c_char, c_int, c_ulong};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Duration;

const IN_CLOEXEC: c_int = 0o2000000;
const IN_CLOSE_WRITE: u32 = 0x8;
const IN_MOVED_FROM: u32 = 0x40;
const IN_MOVED_TO: u32 = 0x80;
const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;
const IN_DELETE_SELF: u32 = 0x400;
const IN_MOVE_SELF: u32 = 0x800;
const IN_ONLYDIR: u32 = 0x0100_0000;
const POLLIN: i16 = 1;
const ENOSPC: i32 = 28;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int;
    fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
}

pub struct Watcher {
    inotify: File,
}

impl Watcher {
    pub fn new() -> io::Result<Watcher> {
        // SAFETY: plain syscall; the result is checked before use.
        let fd = unsafe { inotify_init1(IN_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd is a new descriptor that nothing else owns.
        Ok(Watcher {
            inotify: unsafe { File::from_raw_fd(fd) },
        })
    }

    /// Watches `root` and every folder under it. Adding a folder twice is
    /// harmless, so this can run again after a scan to pick up new folders.
    /// Folders that can't be read are skipped; running out of watches (the
    /// `fs.inotify.max_user_watches` limit) is an error.
    pub fn add_tree(&self, root: &Path) -> io::Result<()> {
        self.add(root)?;
        let Ok(entries) = fs::read_dir(root) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            // file_type doesn't follow symlinks, so linked folders can't loop.
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                self.add_tree(&entry.path())?;
            }
        }
        Ok(())
    }

    fn add(&self, dir: &Path) -> io::Result<()> {
        let Ok(path) = CString::new(dir.as_os_str().as_bytes()) else {
            return Ok(());
        };
        let mask = IN_CLOSE_WRITE
            | IN_MOVED_FROM
            | IN_MOVED_TO
            | IN_CREATE
            | IN_DELETE
            | IN_DELETE_SELF
            | IN_MOVE_SELF
            | IN_ONLYDIR;
        // SAFETY: path is a valid C string for the duration of the call.
        let wd = unsafe { inotify_add_watch(self.inotify.as_raw_fd(), path.as_ptr(), mask) };
        match io::Error::last_os_error() {
            e if wd < 0 && e.raw_os_error() == Some(ENOSPC) => Err(e),
            _ => Ok(()),
        }
    }

    /// Blocks until something changes, then until nothing has changed for
    /// `quiet`, so copying in an album is one change and not hundreds.
    pub fn wait(&mut self, quiet: Duration) -> io::Result<()> {
        self.drain()?;
        while self.readable(quiet)? {
            self.drain()?;
        }
        Ok(())
    }

    /// Reads (blocking) whatever events are queued.
    fn drain(&mut self) -> io::Result<()> {
        let mut buf = [0u8; 16 * 1024];
        loop {
            match self.inotify.read(&mut buf) {
                Ok(_) => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    fn readable(&self, timeout: Duration) -> io::Result<bool> {
        let mut fd = PollFd {
            fd: self.inotify.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(c_int::MAX as u128) as c_int;
        loop {
            // SAFETY: fd points to one valid pollfd.
            match unsafe { poll(&mut fd, 1, ms) } {
                n if n >= 0 => return Ok(n > 0),
                _ => {
                    let e = io::Error::last_os_error();
                    if e.kind() != io::ErrorKind::Interrupted {
                        return Err(e);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::sync::mpsc;
    use std::thread;

    /// Checks that `watcher` reports a change within a few seconds of
    /// `change`, and not before it.
    fn notices(watcher: Watcher, change: impl FnOnce()) -> Watcher {
        let (tx, rx) = mpsc::channel();
        let waiting = thread::spawn(move || {
            let mut watcher = watcher;
            watcher.wait(Duration::from_millis(50)).unwrap();
            tx.send(()).unwrap();
            watcher
        });
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        change();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("no change seen");
        waiting.join().unwrap()
    }

    #[test]
    fn notices_files_in_new_subfolders() {
        let dir = TempDir::new("watch");
        let root = dir.0.clone();
        fs::create_dir(root.join("a")).unwrap();
        let watcher = Watcher::new().unwrap();
        watcher.add_tree(&root).unwrap();

        let watcher = notices(watcher, || {
            fs::write(root.join("a/song.flac"), b"").unwrap()
        });
        let watcher = notices(watcher, || fs::create_dir(root.join("b")).unwrap());
        // A folder made after add_tree is watched once it runs again.
        watcher.add_tree(&root).unwrap();
        notices(watcher, || {
            fs::write(root.join("b/song.flac"), b"").unwrap()
        });
    }
}
