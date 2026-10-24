//! Waking the GTK main loop from another thread without polling: the thread
//! writes a byte to a socket, and the loop runs a callback when the socket
//! has data.

use std::io::Write;
use std::os::unix::net::UnixStream;

use gtk::gio::prelude::*;
use gtk::{gio, glib};

/// A socket pair: write to [`Wakeup::sender`] from any thread to make the
/// callback given to [`Wakeup::watch`] run on the main loop.
pub struct Wakeup {
    sender: UnixStream,
    receiver: UnixStream,
}

impl Wakeup {
    pub fn new() -> std::io::Result<Wakeup> {
        let (sender, receiver) = UnixStream::pair()?;
        // A full socket already means a wakeup is pending.
        sender.set_nonblocking(true)?;
        Ok(Wakeup { sender, receiver })
    }

    /// Something that wakes the loop from other threads.
    pub fn sender(&self) -> std::io::Result<impl Fn() + Send + 'static> {
        let sender = self.sender.try_clone()?;
        Ok(move || {
            let _ = (&sender).write(&[1]);
        })
    }

    /// Runs `f` on the main loop whenever woken, until it breaks.
    pub fn watch(
        self,
        name: &str,
        f: impl FnMut() -> glib::ControlFlow + 'static,
    ) -> Result<(), glib::Error> {
        watch(self.receiver, name, f)
    }
}

/// Runs `f` on the main loop whenever `stream` has data, after draining it.
pub fn watch(
    stream: UnixStream,
    name: &str,
    mut f: impl FnMut() -> glib::ControlFlow + 'static,
) -> Result<(), glib::Error> {
    let socket = gio::Socket::from_fd(stream.into())?;
    // Draining it below must not wait for more.
    socket.set_blocking(false);
    let source = SocketExtManual::create_source(
        &socket,
        glib::IOCondition::IN,
        None::<&gio::Cancellable>,
        Some(name),
        glib::Priority::DEFAULT,
        move |socket, _| {
            let mut buf = [0u8; 64];
            while matches!(socket.receive(&mut buf, None::<&gio::Cancellable>), Ok(n) if n > 0) {}
            f()
        },
    );
    source.attach(None);
    Ok(())
}
