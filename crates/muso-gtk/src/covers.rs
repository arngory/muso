//! Album covers for the library view, read and scaled down off the main
//! thread. The most recently shown are kept in memory, up to a budget, so a
//! library of thousands of albums doesn't keep thousands of images.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use muso_core::track::{self, Formats};

/// How much memory the kept covers may take. At the usual tile size that is
/// a few hundred albums, more than fit on screen.
const KEPT_BYTES: usize = 32 << 20;

/// A cover of one track at one size in device pixels.
type Key = (PathBuf, i32);

#[derive(Clone)]
pub struct Covers {
    formats: Formats,
    kept: Rc<RefCell<Kept>>,
    /// Images waiting for each cover being read.
    waiting: Rc<RefCell<HashMap<Key, Vec<glib::WeakRef<gtk::Image>>>>>,
}

/// Covers read lately; `None` for tracks without one.
#[derive(Default)]
struct Kept {
    covers: HashMap<Key, Option<gdk::Texture>>,
    /// Oldest use first.
    order: VecDeque<Key>,
    bytes: usize,
}

impl Kept {
    fn get(&mut self, key: &Key) -> Option<Option<gdk::Texture>> {
        let cover = self.covers.get(key)?.clone();
        if let Some(at) = self.order.iter().position(|k| k == key) {
            let key = self.order.remove(at)?;
            self.order.push_back(key);
        }
        Some(cover)
    }

    fn insert(&mut self, key: Key, cover: Option<gdk::Texture>) {
        self.bytes += cover.as_ref().map_or(0, texture_bytes);
        if let Some(old) = self.covers.insert(key.clone(), cover) {
            self.bytes -= old.as_ref().map_or(0, texture_bytes);
            self.order.retain(|k| k != &key);
        }
        self.order.push_back(key);
        while self.bytes > KEPT_BYTES && self.order.len() > 1 {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(Some(texture)) = self.covers.remove(&oldest) {
                self.bytes -= texture_bytes(&texture);
            }
        }
    }
}

fn texture_bytes(texture: &gdk::Texture) -> usize {
    texture.width() as usize * texture.height() as usize * 4
}

impl Covers {
    pub fn new(formats: Formats) -> Covers {
        Covers {
            formats,
            kept: Rc::default(),
            waiting: Rc::default(),
        }
    }

    /// Shows the cover of the track at `path` in `image` once it's read,
    /// scaled to the size CSS gives the image, so `image` must be in the
    /// window already. Until then, and if there is none, the image is empty
    /// and CSS shows the placeholder.
    pub fn show(&self, path: &Path, image: &gtk::Image) {
        image.set_paintable(None::<&gdk::Paintable>);
        let (_, natural, _, _) = image.measure(gtk::Orientation::Horizontal, -1);
        let key = (path.to_owned(), (natural * image.scale_factor()).max(1));
        // SAFETY: only ever set to a Key, here.
        unsafe { image.set_data("cover", key.clone()) };
        if let Some(cover) = self.kept.borrow_mut().get(&key) {
            image.set_paintable(cover.as_ref());
            return;
        }
        let mut waiting = self.waiting.borrow_mut();
        if let Some(images) = waiting.get_mut(&key) {
            images.push(image.downgrade());
            return;
        }
        waiting.insert(key.clone(), vec![image.downgrade()]);
        drop(waiting);

        let covers = self.clone();
        glib::spawn_future_local(async move {
            let formats = covers.formats.clone();
            let (from, size) = key.clone();
            let pixels = gio::spawn_blocking(move || read(&from, size, &formats))
                .await
                .ok()
                .flatten();
            let texture = pixels.map(|p| p.texture());
            covers
                .kept
                .borrow_mut()
                .insert(key.clone(), texture.clone());
            let images = covers.waiting.borrow_mut().remove(&key).unwrap_or_default();
            for image in images.iter().filter_map(|i| i.upgrade()) {
                // The image may show another album by now.
                // SAFETY: the key only ever holds a Key.
                let wanted = unsafe { image.data::<Key>("cover").map(|k| k.as_ref() == &key) };
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

/// The cover of `path` scaled to fit a `size` pixels square.
fn read(path: &Path, size: i32, formats: &Formats) -> Option<Pixels> {
    let bytes = glib::Bytes::from_owned(track::cover(path, formats)?);
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let pixbuf =
        Pixbuf::from_stream_at_scale(&stream, size, size, true, None::<&gio::Cancellable>).ok()?;
    Some(Pixels {
        bytes: pixbuf.read_pixel_bytes(),
        width: pixbuf.width(),
        height: pixbuf.height(),
        stride: pixbuf.rowstride() as usize,
        alpha: pixbuf.has_alpha(),
    })
}
