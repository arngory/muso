//! A list of tracks: number, title, optionally artist and album, length.
//! Rows are made only for what's on screen, so a queue of the whole library
//! stays quick. The track playing now is shown inverted.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use muso_core::track::TrackInfo;

use crate::format_time;

#[derive(Clone)]
pub struct TrackList {
    pub root: gtk::ScrolledWindow,
    view: gtk::ListView,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    current: Rc<RefCell<Option<PathBuf>>>,
    /// The row widgets that exist and the track each shows. ListView keeps
    /// a row as it is when its item stays, so marking the playing track
    /// goes through these.
    rows: Rows,
}

type Rows = Rc<RefCell<Vec<(gtk::Box, PathBuf)>>>;

/// Which columns a list shows.
#[derive(Clone, Copy)]
pub struct Columns {
    pub number: bool,
    pub artist: bool,
    pub album: bool,
}

impl TrackList {
    pub fn new(columns: Columns) -> TrackList {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::builder()
            .model(&store)
            .autoselect(false)
            .can_unselect(true)
            .build();
        let current: Rc<RefCell<Option<PathBuf>>> = Rc::default();
        let rows: Rows = Rc::default();

        let factory = gtk::SignalListItemFactory::new();
        let made = rows.clone();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
            let row = gtk::Box::builder().css_classes(["track"]).build();
            let cell = |classes: &[&str], expand: bool, visible: bool| {
                // Numbers line up on the right, as in a table.
                let right = classes == ["number"];
                let label = gtk::Label::builder()
                    .css_classes(classes.to_vec())
                    .xalign(if right { 1.0 } else { 0.0 })
                    .hexpand(expand)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .single_line_mode(true)
                    .visible(visible)
                    .build();
                if expand {
                    // Text columns share the width equally, whatever they hold.
                    label.set_width_chars(1);
                    label.set_max_width_chars(1);
                }
                row.append(&label);
            };
            cell(&["number"], false, columns.number);
            cell(&["name"], true, true);
            cell(&["artist"], true, columns.artist);
            cell(&["album"], true, columns.album);
            cell(&["length"], false, true);
            item.set_child(Some(&row));
            made.borrow_mut().push((row, PathBuf::new()));
        });
        let made = rows.clone();
        factory.connect_teardown(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
            if let Some(row) = item.child() {
                made.borrow_mut()
                    .retain(|(r, _)| r.upcast_ref::<gtk::Widget>() != &row);
            }
        });
        let shown = current.clone();
        let made = rows.clone();
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
            let (Some(row), Some(object)) = (item.child(), item.item()) else {
                return;
            };
            let object = object
                .downcast::<glib::BoxedAnyObject>()
                .expect("the store holds tracks");
            let track = object.borrow::<TrackInfo>();
            let mut labels = std::iter::successors(row.first_child(), |w| w.next_sibling())
                .filter_map(|w| w.downcast::<gtk::Label>().ok());
            let mut set = |text: &str| {
                if let Some(label) = labels.next() {
                    label.set_label(text);
                }
            };
            set(&track
                .track_number
                .map(|n| n.to_string())
                .unwrap_or_default());
            set(&track.display_title());
            set(track
                .artist
                .as_deref()
                .or(track.album_artist.as_deref())
                .unwrap_or_default());
            set(track.album.as_deref().unwrap_or_default());
            set(&track.duration.map(format_time).unwrap_or_default());
            if let Some(entry) = made
                .borrow_mut()
                .iter_mut()
                .find(|(r, _)| r.upcast_ref::<gtk::Widget>() == &row)
            {
                entry.1 = track.path.clone();
            }
            mark(&row, shown.borrow().as_ref() == Some(&track.path));
        });

        let view = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .css_classes(["tracks"])
            .build();
        let root = gtk::ScrolledWindow::builder()
            .child(&view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        TrackList {
            root,
            view,
            store,
            selection,
            current,
            rows,
        }
    }

    pub fn set(&self, tracks: Vec<TrackInfo>) {
        let items: Vec<glib::BoxedAnyObject> =
            tracks.into_iter().map(glib::BoxedAnyObject::new).collect();
        self.store.splice(0, self.store.n_items(), &items);
        self.selection.set_selected(gtk::INVALID_LIST_POSITION);
    }

    pub fn len(&self) -> usize {
        self.store.n_items() as usize
    }

    pub fn get(&self, index: usize) -> Option<TrackInfo> {
        let object = self.store.item(index as u32)?;
        let object = object.downcast::<glib::BoxedAnyObject>().ok()?;
        Some(object.borrow::<TrackInfo>().clone())
    }

    pub fn tracks(&self) -> Vec<TrackInfo> {
        (0..self.len()).filter_map(|i| self.get(i)).collect()
    }

    pub fn selected(&self) -> Option<usize> {
        let i = self.selection.selected();
        (i != gtk::INVALID_LIST_POSITION).then_some(i as usize)
    }

    pub fn select(&self, index: usize) {
        if index < self.len() {
            self.selection.set_selected(index as u32);
            // ListView::scroll_to needs GTK 4.12; muso supports 4.10.
            let _ = self
                .view
                .activate_action("list.scroll-to-item", Some(&(index as u32).to_variant()));
        }
    }

    /// Marks the track at `path` as playing.
    pub fn set_current(&self, path: Option<&PathBuf>) {
        self.current.replace(path.cloned());
        for (row, shows) in self.rows.borrow().iter() {
            mark(row, Some(shows) == path);
        }
    }

    /// Runs `f` with the index of a row activated by double click or Enter.
    pub fn connect_activate(&self, f: impl Fn(usize) + 'static) {
        self.view.connect_activate(move |_, i| f(i as usize));
    }
}

fn mark(row: &impl IsA<gtk::Widget>, current: bool) {
    if current {
        row.add_css_class("current");
    } else {
        row.remove_css_class("current");
    }
}
