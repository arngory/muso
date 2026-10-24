//! Keeping the library file up to date in a thread of its own: a scan at
//! start, then, with `watch=yes`, another each time files change.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use muso_core::library::{Library, Progress, ScanStats};
use muso_core::track::Formats;
use muso_core::watch::Watcher;

/// How long files must stay unchanged before a rescan, so copying in an
/// album is one rescan and not one per file.
const QUIET: Duration = Duration::from_secs(2);

pub enum Update {
    Progress(Progress),
    Done(ScanStats),
    Failed(String),
}

/// Starts the thread. It sends its updates on the returned channel and
/// calls `wake` after each.
pub fn start(
    db: PathBuf,
    roots: Vec<PathBuf>,
    formats: Formats,
    watch: bool,
    wake: impl Fn() + Send + 'static,
) -> mpsc::Receiver<Update> {
    let (tx, rx) = mpsc::channel();
    let send = move |update| {
        // The window is gone if this fails; the thread ends with the process.
        let _ = tx.send(update);
        wake();
    };
    let spawned = thread::Builder::new()
        .name("library".into())
        .spawn(move || {
            let mut library = match Library::open(&db) {
                Ok(l) => l,
                Err(e) => return send(Update::Failed(format!("{}: {e}", db.display()))),
            };
            let mut watcher = if watch {
                Watcher::new()
                    .inspect_err(|e| eprintln!("muso: can't watch the library: {e}"))
                    .ok()
            } else {
                None
            };
            loop {
                // Before the scan, so changes made during it aren't missed;
                // again each time, for new folders.
                if let Some(w) = &watcher
                    && let Err(e) = roots.iter().try_for_each(|r| w.add_tree(r))
                {
                    eprintln!("muso: can't watch the library: {e}");
                    watcher = None;
                }
                match library.scan(&roots, &formats, |p| send(Update::Progress(p))) {
                    Ok(stats) => send(Update::Done(stats)),
                    Err(e) => send(Update::Failed(e.to_string())),
                }
                match &mut watcher {
                    Some(w) => {
                        if let Err(e) = w.wait(QUIET) {
                            eprintln!("muso: can't watch the library: {e}");
                            return;
                        }
                    }
                    None => return,
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("muso: can't scan the library: {e}");
    }
    rx
}
