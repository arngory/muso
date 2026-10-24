//! The library mode: browsing artists and albums, search, the queue, and
//! keeping the library file in step with the music folders.

use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::Ordering;

use gtk::prelude::*;
use gtk::{gio, glib};
use muso_core::library::{Album, Found, Library};
use muso_core::track::TrackInfo;

use super::{App, Mode};
use crate::i18n::{tr, tr_args};
use crate::library_ui::{AlbumTile, SECTION_LIBRARY, SECTION_QUEUE};
use crate::scanner::{self, Update};
use crate::track_list::TrackList;
use crate::wakeup::Wakeup;

/// The most search results shown in each group.
const SEARCH_LIMIT: usize = 100;
/// How often the lists refresh while a scan adds to them, in microseconds.
const SCAN_REFRESH_US: i64 = 1_000_000;

impl App {
    /// Opens the library file, fills the queue from it if nothing else did,
    /// and starts the scan.
    pub(super) fn load_library(self: &Rc<Self>) {
        let db = self.library_file.clone();
        match Library::open(&db) {
            Ok(l) => *self.library.borrow_mut() = Some(l),
            Err(e) => eprintln!("muso: {}: {e}", db.display()),
        }
        self.refresh_library();
        self.fill_queue_from_library();
        self.queue_changed();

        let wakeup = match Wakeup::new() {
            Ok(w) => w,
            Err(e) => return eprintln!("muso: can't scan the library: {e}"),
        };
        let wake = match wakeup.sender() {
            Ok(w) => w,
            Err(e) => return eprintln!("muso: can't scan the library: {e}"),
        };
        let updates = scanner::start(
            db,
            self.roots.clone(),
            self.formats.clone(),
            self.config.watch,
            wake,
        );
        *self.scan.borrow_mut() = Some(updates);
        let app = Rc::downgrade(self);
        let watched = wakeup.watch("library scan", move || {
            let Some(app) = app.upgrade() else {
                return glib::ControlFlow::Break;
            };
            loop {
                let update = app.scan.borrow().as_ref().and_then(|rx| rx.try_recv().ok());
                match update {
                    Some(u) => app.scan_update(u),
                    None => break,
                }
            }
            glib::ControlFlow::Continue
        });
        if let Err(e) = watched {
            eprintln!("muso: can't scan the library: {e}");
        }
    }

    /// With nothing opened, the queue is the whole library, not playing.
    fn fill_queue_from_library(&self) {
        if !self.queue.borrow().is_empty() {
            return;
        }
        let all = match self.library.borrow().as_ref().map(|l| l.all_tracks()) {
            Some(Ok(all)) if !all.is_empty() => all,
            _ => return,
        };
        self.show_track(&TrackInfo::read(&all[0]));
        self.queue.borrow_mut().replace(all);
        self.queue_changed();
    }

    fn scan_update(self: &Rc<Self>, update: Update) {
        let ui = &self.lib_ui;
        match update {
            Update::Progress(p) => {
                let scanning = p.done < p.total;
                ui.scan.set_reveal_child(scanning);
                if scanning {
                    let roots: Vec<String> = self.roots.iter().map(|r| home_relative(r)).collect();
                    ui.scan_label
                        .set_label(&tr_args("Scanning {}", &[&roots.join(", ")]));
                    ui.scan_count
                        .set_label(&format!("{} / {}", p.done, p.total));
                    ui.scan_progress.set_value(p.done as f64 / p.total as f64);
                }
                let now = glib::monotonic_time();
                if p.done > 0 && now - self.last_refresh.get() >= SCAN_REFRESH_US {
                    self.refresh_library();
                }
            }
            Update::Done(stats) => {
                ui.scan.set_reveal_child(false);
                if stats.read > 0 || stats.removed > 0 {
                    self.refresh_library();
                    self.fill_queue_from_library();
                }
            }
            Update::Failed(e) => {
                ui.scan.set_reveal_child(false);
                eprintln!("muso: library scan: {e}");
                self.osd(&tr_args("Can't scan the library: {}", &[&e]));
            }
        }
    }

    // The library view

    pub(super) fn connect_library(self: &Rc<Self>) {
        let ui = &self.lib_ui;
        let app = Rc::downgrade(self);
        ui.queue.set_lookup(move |path| {
            app.upgrade()
                .and_then(|app| app.library.borrow().as_ref()?.track(path).ok()?)
                .unwrap_or_else(|| TrackInfo {
                    path: path.to_owned(),
                    ..TrackInfo::default()
                })
        });
        let app = Rc::downgrade(self);
        ui.sidebar.connect_row_selected(move |_, row| {
            if let (Some(app), Some(row)) = (app.upgrade(), row) {
                app.show_section(row.index());
            }
        });
        let app = Rc::downgrade(self);
        ui.artist_selection.connect_selected_notify(move |s| {
            if let Some(app) = app.upgrade() {
                app.show_artist(s.selected() as usize);
            }
        });

        let app = Rc::downgrade(self);
        ui.tracks.connect_activate(move |i| {
            if let Some(app) = app.upgrade() {
                app.play_list(&app.lib_ui.tracks, i);
            }
        });
        let app = Rc::downgrade(self);
        ui.found_tracks.connect_activate(move |i| {
            if let Some(app) = app.upgrade() {
                app.play_list(&app.lib_ui.found_tracks, i);
            }
        });
        let app = Rc::downgrade(self);
        ui.queue.connect_activate(move |i| {
            if let Some(app) = app.upgrade() {
                app.play_index(i);
            }
        });

        let app = Rc::downgrade(self);
        ui.search.connect_changed(move |_| {
            if let Some(app) = app.upgrade() {
                app.update_search();
            }
        });

        let app = Rc::downgrade(self);
        self.ui.library.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                app.set_mode(Mode::Library);
            }
        });
        let app = Rc::downgrade(self);
        ui.collapse.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                app.set_mode(Mode::Cover);
            }
        });
        ui.sidebar
            .select_row(ui.sidebar.row_at_index(SECTION_LIBRARY).as_ref());
    }

    /// Switches between the cover and the library, moving the controls
    /// between the OSC and the bar, and gives the window that mode's size.
    pub(super) fn set_mode(&self, mode: Mode) {
        let was = self.mode.replace(mode);
        if was == mode {
            return;
        }
        let size = (self.window.default_width(), self.window.default_height());
        match was {
            Mode::Cover => self.cover_size.set(size),
            Mode::Library => self.library_size.set(size),
        }
        let (ui, lib) = (&self.ui, &self.lib_ui);
        match mode {
            Mode::Library => {
                ui.osc_panel.remove(&ui.info);
                lib.bar_start.append(&ui.info);
                ui.info.set_valign(gtk::Align::Center);
                ui.controls.set_center_widget(None::<&gtk::Widget>);
                lib.bar.set_center_widget(Some(&ui.transport));
                ui.transport.set_valign(gtk::Align::Center);
                ui.right.remove(&ui.volume);
                lib.bar_end.prepend(&ui.volume);
                ui.right.remove(&ui.menu);
                lib.topbar.append(&ui.menu);
                ui.menu.set_direction(gtk::ArrowType::Down);
                self.modes.set_visible_child_name("library");
            }
            Mode::Cover => {
                lib.bar_start.remove(&ui.info);
                ui.osc_panel.prepend(&ui.info);
                ui.info.set_valign(gtk::Align::Fill);
                lib.bar.set_center_widget(None::<&gtk::Widget>);
                ui.controls.set_center_widget(Some(&ui.transport));
                ui.transport.set_valign(gtk::Align::Fill);
                lib.bar_end.remove(&ui.volume);
                ui.right.prepend(&ui.volume);
                lib.topbar.remove(&ui.menu);
                ui.right.append(&ui.menu);
                ui.menu.set_direction(gtk::ArrowType::Up);
                self.modes.set_visible_child_name("cover");
                GtkWindowExt::set_focus(&self.window, None::<&gtk::Widget>);
            }
        }
        if !self.window.is_maximized() && !self.window.is_fullscreen() {
            let (w, h) = match mode {
                Mode::Cover => self.cover_size.get(),
                Mode::Library => self.library_size.get(),
            };
            self.window.set_default_size(w, h);
        }
        self.update_scale();
    }

    /// `b` and `q`: open that section, or go back to the cover if it's
    /// what the window shows already.
    pub(super) fn toggle_section(&self, section: i32) {
        let showing = self.mode.get() == Mode::Library
            && self.selected_section() == section
            && self.lib_ui.search.text().is_empty();
        if showing {
            self.set_mode(Mode::Cover);
        } else {
            self.lib_ui.search.set_text("");
            self.set_mode(Mode::Library);
            self.show_section(section);
        }
    }

    fn selected_section(&self) -> i32 {
        self.lib_ui
            .sidebar
            .selected_row()
            .map_or(SECTION_LIBRARY, |r| r.index())
    }

    pub(super) fn show_section(&self, section: i32) {
        let ui = &self.lib_ui;
        if self.selected_section() != section {
            // Comes back here through row-selected.
            ui.sidebar
                .select_row(ui.sidebar.row_at_index(section).as_ref());
            return;
        }
        let searching = !ui.search.text().is_empty();
        ui.artists_column
            .set_visible(section == SECTION_LIBRARY && !searching);
        if section == SECTION_QUEUE {
            if searching {
                ui.search.set_text("");
            }
            self.queue_changed();
            ui.content.set_visible_child_name("queue");
            return;
        }
        let tracks = self.library.borrow().as_ref().map(|l| l.track_count());
        let empty = match tracks {
            None => Some(tr("Can't open the library file")),
            Some(Ok(0)) if self.roots.is_empty() => {
                Some(tr("No music folder.\nSet one with library= in muso.conf."))
            }
            Some(Ok(0)) => {
                let roots: Vec<String> = self.roots.iter().map(|r| home_relative(r)).collect();
                Some(tr_args("No music in {} yet", &[&roots.join(", ")]))
            }
            _ => None,
        };
        if searching {
            self.show_found();
        } else if let Some(text) = empty {
            ui.empty.set_label(&text);
            ui.content.set_visible_child_name("empty");
        } else {
            ui.content.set_visible_child_name("browse");
        }
    }

    /// Reloads the artists and counts, keeping what's selected.
    fn refresh_library(self: &Rc<Self>) {
        self.last_refresh.set(glib::monotonic_time());
        let ui = &self.lib_ui;
        let (artists, count) = match self.library.borrow().as_ref() {
            Some(l) => (
                l.artists().unwrap_or_default(),
                l.track_count().unwrap_or_default(),
            ),
            None => (Vec::new(), 0),
        };
        ui.count.set_label(&tr_args("Tracks: {}", &[&count]));
        ui.library_count.set_label(&count.to_string());

        let selected = ui.artist_selection.selected() as usize;
        let previous = self.artists.borrow().get(selected).map(|a| a.name.clone());
        if *self.artists.borrow() != artists {
            let names: Vec<String> = artists
                .iter()
                .map(|a| {
                    if a.name.is_empty() {
                        tr("Unknown artist")
                    } else {
                        a.name.clone()
                    }
                })
                .collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            *self.artists.borrow_mut() = artists;
            let keep = previous
                .and_then(|p| self.artists.borrow().iter().position(|a| a.name == p))
                .unwrap_or(0);
            // Splicing unselects; selecting below shows the artist.
            ui.artist_names.splice(0, ui.artist_names.n_items(), &names);
            if !names.is_empty() {
                ui.artist_selection.set_selected(keep as u32);
            }
        } else {
            self.show_artist(selected);
        }
        self.show_section(self.selected_section());
        if !ui.search.text().is_empty() {
            self.update_search();
        }
    }

    fn show_artist(self: &Rc<Self>, index: usize) {
        let Some(artist) = self.artists.borrow().get(index).cloned() else {
            return;
        };
        let albums = match self.library.borrow().as_ref() {
            Some(l) => l.albums(Some(&artist.name)).unwrap_or_default(),
            None => return,
        };
        let same = {
            let shown = self.albums.borrow();
            shown.len() == albums.len() && shown.iter().zip(&albums).all(|((a, _), b)| a == b)
        };
        if !same {
            let tiles = self.album_tiles(&self.lib_ui.albums, &albums);
            self.connect_tiles(&tiles);
            *self.albums.borrow_mut() = tiles;
        }
        let keep = self
            .album
            .borrow()
            .as_ref()
            .and_then(|a| albums.iter().find(|b| same_album(a, b)))
            .or(albums.first())
            .cloned();
        if let Some(album) = keep {
            self.show_album(&album);
        }
    }

    /// Fills `row` with a tile per album and returns them.
    fn album_tiles(&self, row: &gtk::Box, albums: &[Album]) -> Vec<(Album, gtk::Button)> {
        while let Some(child) = row.first_child() {
            row.remove(&child);
        }
        let mut tiles = Vec::new();
        for album in albums {
            let tile = AlbumTile::new(&album.title, album.year);
            row.append(&tile.button);
            self.covers.show(&album.first, &tile.cover);
            tiles.push((album.clone(), tile.button));
        }
        tiles
    }

    fn show_album(&self, album: &Album) {
        for (a, button) in self.albums.borrow().iter() {
            if same_album(a, album) {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
        *self.album.borrow_mut() = Some(album.clone());
        let changed = self.tracks_of.borrow().as_ref() != Some(album);
        if changed {
            *self.tracks_of.borrow_mut() = Some(album.clone());
            let tracks = match self.library.borrow().as_ref() {
                Some(l) => l.tracks(album).unwrap_or_default(),
                None => return,
            };
            self.lib_ui.tracks.set(tracks);
            self.mark_current();
        }
    }

    /// Clicks on album tiles: one shows the album, two play it.
    fn connect_tiles(self: &Rc<Self>, tiles: &[(Album, gtk::Button)]) {
        for (album, button) in tiles {
            let app = Rc::downgrade(self);
            let album = album.clone();
            button.connect_clicked(move |_| {
                let Some(app) = app.upgrade() else { return };
                let now = glib::monotonic_time();
                let double_click_us =
                    gtk::Settings::default().map_or(400, |s| s.gtk_double_click_time()) as i64
                        * 1000;
                let double = app
                    .last_click
                    .replace(Some((album.clone(), now)))
                    .is_some_and(|(a, t)| a == album && now - t <= double_click_us);
                if double {
                    app.last_click.take();
                    app.play_album(&album);
                } else {
                    app.lib_ui.search.set_text("");
                    app.show_album_of(&album);
                }
            });
        }
    }

    /// Shows `album` in the browser, selecting its artist.
    fn show_album_of(self: &Rc<Self>, album: &Album) {
        *self.album.borrow_mut() = Some(album.clone());
        let index = self
            .artists
            .borrow()
            .iter()
            .position(|a| a.name == album.artist);
        let ui = &self.lib_ui;
        if let Some(i) = index {
            if ui.artist_selection.selected() as usize == i {
                self.show_artist(i);
            } else {
                ui.artist_selection.set_selected(i as u32);
                // ListView::scroll_to needs GTK 4.12; muso supports 4.10.
                let _ = ui
                    .artists
                    .activate_action("list.scroll-to-item", Some(&(i as u32).to_variant()));
            }
        }
        self.show_section(SECTION_LIBRARY);
    }

    fn play_album(&self, album: &Album) {
        let tracks = match self.library.borrow().as_ref() {
            Some(l) => l.tracks(album).unwrap_or_default(),
            None => return,
        };
        if !tracks.is_empty() {
            self.replace_queue(tracks.into_iter().map(|t| t.path).collect(), 0);
        }
    }

    /// Plays the tracks of `list` from `index`.
    fn play_list(&self, list: &TrackList, index: usize) {
        self.replace_queue(list.paths(), index);
    }

    /// `n`: the selected track plays after the current one.
    pub(super) fn play_selected_next(&self) {
        match self.selected_track() {
            Some(track) => {
                self.queue.borrow_mut().push_next(track.path.clone());
                self.refresh_upcoming();
                self.queue_changed();
                self.osd(&tr_args("Plays next: {}", &[&track.display_title()]));
            }
            None => self.osd(&tr("Select a track in the library first")),
        }
    }

    fn selected_track(&self) -> Option<TrackInfo> {
        if self.mode.get() != Mode::Library {
            return None;
        }
        let ui = &self.lib_ui;
        let list = match ui.content.visible_child_name().as_deref() {
            Some("browse") => &ui.tracks,
            Some("search") => &ui.found_tracks,
            Some("queue") => &ui.queue,
            _ => return None,
        };
        list.get(list.selected()?)
    }

    // Search

    pub(super) fn start_search(&self) {
        self.set_mode(Mode::Library);
        self.show_section(SECTION_LIBRARY);
        self.lib_ui.search.grab_focus();
    }

    /// Whether a text field has the keyboard.
    pub(super) fn typing(&self) -> bool {
        gtk::prelude::GtkWindowExt::focus(&self.window).is_some_and(|w| w.is::<gtk::Text>())
    }

    /// Keys that mean something while typing a search; `false` lets the
    /// text field have the key.
    pub(super) fn search_key(&self, name: &str) -> bool {
        let ui = &self.lib_ui;
        let found = &ui.found_tracks;
        match name {
            "ESC" => {
                ui.search.set_text("");
                GtkWindowExt::set_focus(&self.window, None::<&gtk::Widget>);
            }
            "DOWN" | "UP" => {
                let next = match (found.selected(), name) {
                    (None, _) => 0,
                    (Some(i), "DOWN") => i + 1,
                    (Some(i), _) => i.saturating_sub(1),
                };
                found.select(next.min(found.len().saturating_sub(1)));
            }
            "ENTER" | "Shift+ENTER" => {
                let index = found.selected().unwrap_or(0);
                let Some(track) = found.get(index) else {
                    return true;
                };
                if name == "ENTER" {
                    self.play_list(found, index);
                } else {
                    self.queue.borrow_mut().push(track.path.clone());
                    self.refresh_upcoming();
                    self.queue_changed();
                    self.osd(&tr_args(
                        "Added to the queue: {}",
                        &[&track.display_title()],
                    ));
                }
            }
            _ => return false,
        }
        true
    }

    /// Searches on another thread, with a connection of its own: in a
    /// library of tens of thousands of tracks a search takes long enough to
    /// stall typing.
    fn update_search(self: &Rc<Self>) {
        let query = self.lib_ui.search.text().to_string();
        if self.selected_section() != SECTION_LIBRARY && !query.is_empty() {
            self.show_section(SECTION_LIBRARY);
        }
        let search = self.searches.fetch_add(1, Ordering::Relaxed) + 1;
        let searches = self.searches.clone();
        let db = self.library_file.clone();
        let app = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(move || {
                // A newer search started while this one waited.
                if searches.load(Ordering::Relaxed) != search {
                    return None;
                }
                Library::open(&db)
                    .and_then(|l| l.search(&query, SEARCH_LIMIT))
                    .ok()
            })
            .await
            .ok()
            .flatten();
            let Some(app) = app.upgrade() else { return };
            if app.searches.load(Ordering::Relaxed) == search {
                app.show_search(found.unwrap_or_default());
            }
        });
    }

    fn show_search(self: &Rc<Self>, found: Found) {
        let ui = &self.lib_ui;
        let changed = *self.found.borrow() != found;
        if changed {
            while let Some(child) = ui.found_artists.first_child() {
                ui.found_artists.remove(&child);
            }
            for artist in &found.artists {
                let name = if artist.name.is_empty() {
                    tr("Unknown artist")
                } else {
                    artist.name.clone()
                };
                let button = gtk::Button::builder()
                    .label(&name)
                    .css_classes(["artist"])
                    .build();
                let app = Rc::downgrade(self);
                let artist = artist.name.clone();
                button.connect_clicked(move |_| {
                    let Some(app) = app.upgrade() else { return };
                    let index = app.artists.borrow().iter().position(|a| a.name == artist);
                    app.lib_ui.search.set_text("");
                    if let Some(i) = index {
                        app.lib_ui.artist_selection.set_selected(i as u32);
                    }
                });
                ui.found_artists.append(&button);
            }
            let tiles = self.album_tiles(&ui.found_albums, &found.albums);
            self.connect_tiles(&tiles);
            ui.found_tracks.set(found.tracks.clone());
            self.mark_current();
            for (title, label, n) in [
                (tr("Artists"), &ui.found_artists_title, found.artists.len()),
                (tr("Albums"), &ui.found_albums_title, found.albums.len()),
                (tr("Tracks"), &ui.found_tracks_title, found.tracks.len()),
            ] {
                label.set_label(&format!("{} · {n}", title.to_uppercase()));
                label.set_visible(n > 0);
            }
            ui.found_artists_row.set_visible(!found.artists.is_empty());
            ui.found_albums_row.set_visible(!found.albums.is_empty());
            ui.found_tracks.root.set_visible(!found.tracks.is_empty());
            *self.found.borrow_mut() = found;
        }
        self.show_section(SECTION_LIBRARY);
    }

    fn show_found(&self) {
        let ui = &self.lib_ui;
        if self.found.borrow().is_empty() {
            ui.empty.set_label(&tr("Nothing found"));
            ui.content.set_visible_child_name("empty");
        } else {
            ui.content.set_visible_child_name("search");
        }
    }

    // Keeping lists in step with playback

    /// Marks the playing track in every list.
    pub(super) fn mark_current(&self) {
        let current = self.queue.borrow().current().cloned();
        let ui = &self.lib_ui;
        for list in [&ui.tracks, &ui.found_tracks, &ui.queue] {
            list.set_current(current.as_ref());
        }
    }

    /// Shows the queue's new contents, if the queue view is open.
    pub(super) fn queue_changed(&self) {
        let ui = &self.lib_ui;
        let queue = self.queue.borrow();
        ui.queue_count.set_label(&queue.len().to_string());
        if self.selected_section() != SECTION_QUEUE {
            return;
        }
        ui.queue.set_paths(queue.items());
        drop(queue);
        self.mark_current();
    }
}

fn same_album(a: &Album, b: &Album) -> bool {
    a.artist == b.artist && a.title == b.title
}

/// `path` with the home folder as `~`.
fn home_relative(path: &Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
