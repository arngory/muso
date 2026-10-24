//! Album covers for the library view, read and scaled down off the main
//! thread and kept in memory while the window is open.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use muso_core::track::{self, Formats};

/// The longest side covers are scaled to: enough for the largest tile on a
/// display at twice the usual density. The full image would cost tens of
/// megabytes per album.
const THUMB_PX: i32 = 512;

#[derive(Clone)]
pub struct Covers {
    formats: Formats,
    /// By track path; `None` for tracks without a cover.
    loaded: Rc<RefCell<HashMap<PathBuf, Option<gdk::Texture>>>>,
    /// Images waiting for each cover being read.
    waiting: Rc<RefCell<HashMap<PathBuf, Vec<glib::WeakRef<gtk::Image>>>>>,
}

impl Covers {
    pub fn new(formats: Formats) -> Covers {
        Covers {
            formats,
            loaded: Rc::default(),
            waiting: Rc::default(),
        }
    }

    /// Shows the cover of the track at `path` in `image` once it's read.
    /// Until then, and if there is none, the image is empty and CSS shows
    /// the placeholder.
    pub fn show(&self, path: &Path, image: &gtk::Image) {
        image.set_paintable(None::<&gdk::Paintable>);
        // SAFETY: only ever set to a PathBuf, here.
        unsafe { image.set_data("cover", path.to_owned()) };
        if let Some(cover) = self.loaded.borrow().get(path) {
            image.set_paintable(cover.as_ref());
            return;
        }
        let mut waiting = self.waiting.borrow_mut();
        if let Some(images) = waiting.get_mut(path) {
            images.push(image.downgrade());
            return;
        }
        waiting.insert(path.to_owned(), vec![image.downgrade()]);
        drop(waiting);

        let covers = self.clone();
        let path = path.to_owned();
        glib::spawn_future_local(async move {
            let formats = covers.formats.clone();
            let from = path.clone();
            let pixels = gio::spawn_blocking(move || read(&from, &formats))
                .await
                .ok()
                .flatten();
            let texture = pixels.map(|p| p.texture());
            covers
                .loaded
                .borrow_mut()
                .insert(path.clone(), texture.clone());
            let images = covers
                .waiting
                .borrow_mut()
                .remove(&path)
                .unwrap_or_default();
            for image in images.iter().filter_map(|i| i.upgrade()) {
                // The image may show another album by now.
                // SAFETY: the key only ever holds a PathBuf.
                let wanted = unsafe { image.data::<PathBuf>("cover").map(|p| p.as_ref() == &path) };
                if wanted == Some(true) {
                    image.set_paintable(texture.as_ref());
                }
            }
        });
    }
}

/// A decoded cover. Pixbufs can't cross threads, their pixels can.
struct Pixels {
    bytes: glib::Bytes,
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
}

impl Pixels {
    fn texture(&self) -> gdk::Texture {
        let format = if self.alpha {
            gdk::MemoryFormat::R8g8b8a8
        } else {
            gdk::MemoryFormat::R8g8b8
        };
        gdk::MemoryTexture::new(self.width, self.height, format, &self.bytes, self.stride).upcast()
    }
}

fn read(path: &Path, formats: &Formats) -> Option<Pixels> {
    let bytes = glib::Bytes::from_owned(track::cover(path, formats)?);
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let pixbuf =
        Pixbuf::from_stream_at_scale(&stream, THUMB_PX, THUMB_PX, true, None::<&gio::Cancellable>)
            .ok()?;
    Some(Pixels {
        bytes: pixbuf.read_pixel_bytes(),
        width: pixbuf.width(),
        height: pixbuf.height(),
        stride: pixbuf.rowstride() as usize,
        alpha: pixbuf.has_alpha(),
    })
}
