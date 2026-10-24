//! The player window: wires the core's queue and config, mpv and the widgets
//! together. It has two modes: the album cover with the OSC on top
//! ([`crate::ui`]), and the library with the controls in a bar
//! ([`crate::library_ui`], wired up in [`library`]).

mod library;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use muso_core::config::{Config, Theme};
use muso_core::input::{Bindings, Command, View};
use muso_core::library::{Album, Artist, Found, Library};
use muso_core::player::{Player, PlayerEvent};
use muso_core::queue::{Queue, Repeat};
use muso_core::track::{self, Formats, TrackInfo};
use muso_mpv::Mpv;

use crate::covers::Covers;
use crate::i18n::{tr, tr_args};
use crate::icons::Icon;
use crate::library_ui::{self, LibraryUi};
use crate::ui::{self, Ui};
use crate::wakeup;
use crate::{format_time, keys, scanner};

/// The design's sizes, in its own units: the text is 16px (12pt), the
/// cover window 30 times that (480px at 96 dpi) and the library window 75
/// by 45 (1200 by 720px). Everything else is in `style.css`, in em, so it
/// follows these.
const BASE_FONT_PT: f64 = 12.0;
const COVER_EM: (f64, f64) = (30.0, 30.0);
const LIBRARY_EM: (f64, f64) = (75.0, 45.0);
/// How far the OSC grows and shrinks with the cover window, as mpv's does.
const SCALE_RANGE: (f64, f64) = (0.75, 2.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Cover,
    Library,
}

pub struct App {
    window: gtk::ApplicationWindow,
    ui: Ui,
    lib_ui: LibraryUi,
    modes: gtk::Stack,
    mode: Cell<Mode>,
    /// The window size each mode had when last left.
    cover_size: Cell<(i32, i32)>,
    library_size: Cell<(i32, i32)>,
    /// `None` if the library file can't be opened.
    library: RefCell<Option<Library>>,
    roots: Vec<PathBuf>,
    covers: Covers,
    scan: RefCell<Option<mpsc::Receiver<scanner::Update>>>,
    last_refresh: Cell<i64>,
    /// What the library view shows, to map rows back to data.
    artists: RefCell<Vec<Artist>>,
    albums: RefCell<Vec<(Album, gtk::Button)>>,
    album: RefCell<Option<Album>>,
    /// The album whose tracks the browser lists.
    tracks_of: RefCell<Option<Album>>,
    found: RefCell<Found>,
    /// The last album tile clicked and when, to tell a double click.
    last_click: RefCell<Option<(Album, i64)>>,
    scale_css: gtk::CssProvider,
    scale_pt: RefCell<String>,
    player: RefCell<Mpv>,
    queue: RefCell<Queue<PathBuf>>,
    config: Config,
    bindings: Bindings,
    formats: Formats,
    paused: Cell<bool>,
    volume: Cell<u8>,
    muted: Cell<bool>,
    position: Cell<Duration>,
    duration: Cell<Option<Duration>>,
    /// Tracks in a row that mpv couldn't play, to stop instead of looping.
    failures: Cell<usize>,
    osc_timer: RefCell<Option<glib::SourceId>>,
    osd_timer: Rc<RefCell<Option<glib::SourceId>>>,
    pointer_on_osc: Cell<bool>,
    pointer_on_volume: Cell<bool>,
}

impl App {
    pub fn new(
        app: &gtk::Application,
        config: Config,
        bindings: Bindings,
        mut player: Mpv,
    ) -> Rc<App> {
        let formats = player.formats();
        let ui = ui::build();
        let lib_ui = library_ui::build();
        // Sized by the mode shown, so the cover window can be small.
        let modes = gtk::Stack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .build();
        modes.add_named(&ui.root, Some("cover"));
        modes.add_named(&lib_ui.root, Some("library"));
        let top = gtk::Overlay::builder().child(&modes).build();
        top.add_overlay(&ui.osd);
        top.add_overlay(&ui.help);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("muso")
            .css_classes(["muso"])
            .child(&top)
            .build();
        let roots = if config.library.is_empty() {
            glib::user_special_dir(glib::UserDirectory::Music)
                .into_iter()
                .collect()
        } else {
            config.library.clone()
        };
        if config.theme == Theme::Light {
            window.add_css_class("light");
        }
        let seed = glib::real_time() as u64;
        let app = Rc::new(App {
            window,
            ui,
            lib_ui,
            modes,
            mode: Cell::new(Mode::Cover),
            cover_size: Cell::new(em_px(COVER_EM)),
            library_size: Cell::new(em_px(LIBRARY_EM)),
            library: RefCell::default(),
            roots,
            covers: Covers::new(formats.clone()),
            scan: RefCell::default(),
            last_refresh: Cell::new(0),
            artists: RefCell::default(),
            albums: RefCell::default(),
            album: RefCell::default(),
            tracks_of: RefCell::default(),
            found: RefCell::default(),
            last_click: RefCell::default(),
            scale_css: gtk::CssProvider::new(),
            scale_pt: RefCell::default(),
            player: RefCell::new(player),
            queue: RefCell::new(Queue::new(seed)),
            config,
            bindings,
            formats,
            paused: Cell::new(true),
            volume: Cell::new(100),
            muted: Cell::new(false),
            position: Cell::new(Duration::ZERO),
            duration: Cell::new(None),
            failures: Cell::new(0),
            osc_timer: RefCell::default(),
            osd_timer: Rc::default(),
            pointer_on_osc: Cell::new(false),
            pointer_on_volume: Cell::new(false),
        });
        {
            let mut player = app.player.borrow_mut();
            player.set_replaygain(app.config.replaygain);
            player.set_volume(app.volume.get());
        }
        app.connect_scaling();
        app.connect_player();
        app.connect_widgets();
        app.connect_keys();
        app.connect_pointer();
        app.connect_drop();
        app.add_actions();
        app.connect_library();
        app.fill_help();
        app.show_empty();
        app.update_volume();
        app.load_library();
        if app.config.start_view == muso_core::config::StartView::Library {
            app.set_mode(Mode::Library);
        }
        app
    }

    pub fn present(&self) {
        self.window.present();
    }

    /// Replaces the queue with `paths` (folders are searched for music) and
    /// plays the first track.
    pub fn open(&self, paths: &[PathBuf]) {
        let files = track::collect(paths, &self.formats);
        if files.is_empty() {
            self.osd(&tr("No music found"));
            return;
        }
        self.replace_queue(files, 0);
    }

    /// Replaces the queue with `files` and plays the one at `start`.
    fn replace_queue(&self, files: Vec<PathBuf>, start: usize) {
        self.queue.borrow_mut().replace(files);
        self.play_index(start);
        self.queue_changed();
    }

    // Playback

    fn play_index(&self, index: usize) {
        let path = self.queue.borrow_mut().play(index).cloned();
        if let Some(path) = path {
            self.start(&path);
        }
    }

    fn start(&self, path: &std::path::Path) {
        if let Err(e) = self.player.borrow_mut().load(path) {
            self.osd(&e.to_string());
            return;
        }
        self.set_paused(false);
        self.show_track(&TrackInfo::read(path));
        self.mark_current();
    }

    fn stop(&self) {
        self.queue.borrow_mut().stop();
        self.mark_current();
        self.set_paused(true);
        self.position.set(Duration::ZERO);
        self.update_progress();
    }

    fn set_paused(&self, paused: bool) {
        self.paused.set(paused);
        self.player.borrow_mut().set_paused(paused);
        self.update_play_button();
    }

    /// Hands mpv the track that follows the current one, for gapless playback.
    fn refresh_upcoming(&self) {
        if !self.config.gapless {
            return;
        }
        let queue = self.queue.borrow();
        if queue.current().is_none() {
            return;
        }
        let next = queue.upcoming().cloned();
        drop(queue);
        if let Err(e) = self.player.borrow_mut().queue_next(next.as_deref()) {
            eprintln!("muso: {e}");
        }
    }

    fn connect_player(self: &Rc<Self>) {
        let app = Rc::downgrade(self);
        let watched = self.player.borrow_mut().wakeup_socket().and_then(|socket| {
            wakeup::watch(socket, "mpv events", move || {
                let Some(app) = app.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                loop {
                    let event = app.player.borrow_mut().poll_event();
                    match event {
                        Some(e) => app.handle(e),
                        None => break,
                    }
                }
                glib::ControlFlow::Continue
            })
            .map_err(std::io::Error::other)
        });
        if let Err(e) = watched {
            eprintln!("muso: can't watch mpv events: {e}");
        }
    }

    fn handle(&self, event: PlayerEvent) {
        match event {
            PlayerEvent::Started { gapless } => {
                self.failures.set(0);
                self.position.set(Duration::ZERO);
                if gapless {
                    let path = self.queue.borrow_mut().track_ended().cloned();
                    if let Some(path) = path {
                        self.show_track(&TrackInfo::read(&path));
                        self.mark_current();
                    }
                }
                self.refresh_upcoming();
                self.update_progress();
            }
            PlayerEvent::Duration(d) => {
                self.duration.set(Some(d));
                self.update_progress();
            }
            PlayerEvent::Position(p) => {
                self.position.set(p);
                self.update_progress();
            }
            PlayerEvent::Paused(p) => {
                self.paused.set(p);
                self.update_play_button();
            }
            PlayerEvent::Ended => {
                let next = self.queue.borrow_mut().track_ended().cloned();
                match next {
                    Some(path) => self.start(&path),
                    None => self.stop(),
                }
            }
            PlayerEvent::Failed(e) => {
                let current = self.queue.borrow().current().cloned();
                if let Some(path) = &current {
                    eprintln!("muso: {}: {e}", path.display());
                }
                self.failures.set(self.failures.get() + 1);
                if self.failures.get() >= self.queue.borrow().len() {
                    self.osd(&tr_args("Can't play: {}", &[&e]));
                    self.stop();
                    return;
                }
                let next = self.queue.borrow_mut().next_track().cloned();
                match next {
                    Some(path) => self.start(&path),
                    None => self.stop(),
                }
            }
        }
    }

    // Commands

    fn run(self: &Rc<Self>, command: &Command) {
        match command {
            Command::CyclePause => {
                if self.queue.borrow().current().is_none() {
                    if !self.queue.borrow().is_empty() {
                        self.play_index(0);
                    }
                    return;
                }
                self.set_paused(!self.paused.get());
                self.osd(&if self.paused.get() {
                    tr("Paused")
                } else {
                    tr("Playing")
                });
            }
            Command::Seek(secs) => {
                if self.queue.borrow().current().is_none() {
                    return;
                }
                let now = self.position.get().as_secs_f64();
                let end = self.duration.get().map_or(f64::MAX, |d| d.as_secs_f64());
                let to = Duration::from_secs_f64((now + secs).clamp(0.0, end));
                self.seek(to);
                let total = self.duration.get().map(format_time).unwrap_or_default();
                self.osd(&format!("{} / {total}", format_time(to)));
            }
            Command::AddVolume(points) => {
                self.add_volume(*points);
                self.osd(&tr_args("Volume: {}%", &[&self.volume.get()]));
            }
            Command::PlaylistPrev => {
                let path = self.queue.borrow_mut().prev_track().cloned();
                if let Some(path) = path {
                    self.start(&path);
                }
            }
            Command::PlaylistNext => {
                let path = self.queue.borrow_mut().next_track().cloned();
                match path {
                    Some(path) => self.start(&path),
                    None => self.stop(),
                }
            }
            Command::CycleMute => {
                self.muted.set(!self.muted.get());
                self.player.borrow_mut().set_muted(self.muted.get());
                self.update_volume();
                self.osd(&if self.muted.get() {
                    tr("Mute")
                } else {
                    tr("Unmute")
                });
            }
            Command::CycleShuffle => {
                let on = !self.queue.borrow().shuffle();
                self.queue.borrow_mut().set_shuffle(on);
                self.ui.shuffle.set_on(on);
                self.refresh_upcoming();
                self.osd(&if on {
                    tr("Shuffle on")
                } else {
                    tr("Shuffle off")
                });
            }
            Command::CycleRepeat => {
                let repeat = self.queue.borrow().repeat().cycle();
                self.queue.borrow_mut().set_repeat(repeat);
                self.update_repeat();
                self.refresh_upcoming();
                self.osd(&match repeat {
                    Repeat::Off => tr("Repeat off"),
                    Repeat::All => tr("Repeat all"),
                    Repeat::One => tr("Repeat one"),
                });
            }
            Command::CycleOntop => self.osd(&tr("GTK 4 can't keep a window on top")),
            Command::Open => self.choose(false),
            Command::Help => self.ui.help.set_visible(!self.ui.help.is_visible()),
            Command::Show(View::Library) => self.toggle_section(library_ui::SECTION_LIBRARY),
            Command::Show(View::Queue) => self.toggle_section(library_ui::SECTION_QUEUE),
            Command::Search => self.start_search(),
            Command::PlayNext => self.play_selected_next(),
            Command::Show(View::Playlists)
            | Command::Move(_)
            | Command::Remove
            | Command::Settings => self.osd(&tr("Not in this version yet")),
            Command::Ignore => {}
        }
    }

    fn seek(&self, to: Duration) {
        self.player.borrow_mut().seek(to);
        self.position.set(to);
        self.update_progress();
    }

    fn add_volume(&self, points: i32) {
        let v = (self.volume.get() as i32 + points).clamp(0, 100) as u8;
        self.set_volume(v);
    }

    fn set_volume(&self, v: u8) {
        self.volume.set(v);
        self.player.borrow_mut().set_volume(v);
        self.update_volume();
    }

    fn choose(self: &Rc<Self>, folders: bool) {
        let dialog = gtk::FileDialog::builder().modal(true).build();
        let app = Rc::downgrade(self);
        let done = move |result: Result<gio::ListModel, glib::Error>| {
            let Ok(list) = result else { return };
            let paths: Vec<PathBuf> = list
                .iter::<gio::File>()
                .flatten()
                .filter_map(|f| f.path())
                .collect();
            if let Some(app) = app.upgrade() {
                app.open(&paths);
            }
        };
        if folders {
            dialog.select_multiple_folders(Some(&self.window), None::<&gio::Cancellable>, done);
        } else {
            dialog.open_multiple(Some(&self.window), None::<&gio::Cancellable>, done);
        }
    }

    // Showing state

    fn show_empty(&self) {
        self.ui
            .placeholder
            .set_label(&tr("Drop music here\nor press O to open"));
        self.ui.cover_stack.set_visible_child_name("placeholder");
        self.ui.title.set_label(&tr("Nothing playing"));
        self.ui.subline.set_label("");
        self.update_progress();
        self.update_play_button();
    }

    fn show_track(&self, info: &TrackInfo) {
        let title = info.display_title();
        self.ui.title.set_label(&title);
        let artist = info.artist.as_deref().or(info.album_artist.as_deref());
        let subline = match (artist, info.album.as_deref()) {
            (Some(a), Some(b)) => format!("{a} — {b}"),
            (Some(a), None) => a.to_owned(),
            (None, Some(b)) => b.to_owned(),
            (None, None) => String::new(),
        };
        self.ui.subline.set_label(&subline);
        self.window.set_title(Some(&match artist {
            Some(a) => format!("{title} — {a}"),
            None => title.clone(),
        }));
        self.duration.set(info.duration);

        self.covers.show(&info.path, &self.lib_ui.thumb);
        let texture = track::cover(&info.path, &self.formats)
            .and_then(|bytes| gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok());
        match texture {
            Some(t) => {
                self.ui.cover.set_paintable(Some(&t));
                self.ui.cover_stack.set_visible_child_name("cover");
            }
            None => {
                self.ui
                    .placeholder
                    .set_label(info.album.as_deref().unwrap_or(&title));
                self.ui.cover_stack.set_visible_child_name("placeholder");
            }
        }
        self.update_progress();
    }

    fn update_progress(&self) {
        let pos = self.position.get();
        self.ui.elapsed.set_label(&format_time(pos));
        match self.duration.get() {
            Some(d) if !d.is_zero() => {
                let left = d.saturating_sub(pos);
                self.ui
                    .remaining
                    .set_label(&format!("\u{2212}{}", format_time(left)));
                self.ui
                    .progress
                    .set_value(pos.as_secs_f64() / d.as_secs_f64());
            }
            _ => {
                self.ui.remaining.set_label("");
                self.ui.progress.set_value(0.0);
            }
        }
    }

    fn update_play_button(&self) {
        let playing = !self.paused.get() && self.queue.borrow().current().is_some();
        self.ui
            .play_icon
            .set(if playing { Icon::Pause } else { Icon::Play });
        self.ui
            .play
            .set_tooltip_text(Some(&if playing { tr("Pause") } else { tr("Play") }));
    }

    fn update_repeat(&self) {
        let repeat = self.queue.borrow().repeat();
        self.ui.repeat.set_on(repeat != Repeat::Off);
        self.ui.repeat_one.set_visible(repeat == Repeat::One);
    }

    fn update_volume(&self) {
        let v = self.volume.get();
        self.ui.volume_value.set_label(&v.to_string());
        self.ui.volume_slider.set_value(v as f64 / 100.0);
        let muted = self.muted.get() || v == 0;
        self.ui
            .volume_icon
            .set(if muted { Icon::Muted } else { Icon::Volume });
    }

    fn osd(&self, text: &str) {
        self.ui.osd.set_label(text);
        self.ui.osd.set_visible(true);
        if let Some(id) = self.osd_timer.take() {
            id.remove();
        }
        let osd = self.ui.osd.clone();
        let timer = self.osd_timer.clone();
        let id = glib::timeout_add_local_once(self.config.osd_duration, move || {
            timer.take();
            osd.set_visible(false);
        });
        *self.osd_timer.borrow_mut() = Some(id);
    }

    fn fill_help(&self) {
        for (row, (key, command)) in self.bindings.iter().enumerate() {
            let key = gtk::Label::builder()
                .label(key)
                .css_classes(["key"])
                .xalign(0.0)
                .build();
            let command = gtk::Label::builder()
                .label(describe(command))
                .css_classes(["action"])
                .xalign(0.0)
                .build();
            self.ui.help_grid.attach(&key, 0, row as i32, 1, 1);
            self.ui.help_grid.attach(&command, 1, row as i32, 1, 1);
        }
    }

    // Wiring

    fn connect_widgets(self: &Rc<Self>) {
        let on = |button: &gtk::Button, command: Command| {
            let app = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.run_quietly(&command);
                }
            });
        };
        on(&self.ui.play, Command::CyclePause);
        on(&self.ui.prev, Command::PlaylistPrev);
        on(&self.ui.next, Command::PlaylistNext);
        on(&self.ui.shuffle.button, Command::CycleShuffle);
        on(&self.ui.repeat.button, Command::CycleRepeat);
        on(&self.ui.volume, Command::CycleMute);

        let app = Rc::downgrade(self);
        self.ui.progress.connect_change(move |v| {
            let Some(app) = app.upgrade() else { return };
            if let Some(d) = app.duration.get() {
                app.seek(d.mul_f64(v));
            }
        });
        let app = Rc::downgrade(self);
        self.ui.volume_slider.connect_change(move |v| {
            if let Some(app) = app.upgrade() {
                app.set_volume((v * 100.0).round() as u8);
            }
        });

        // The volume popup opens while the pointer is on the button or the
        // popup itself.
        let hover = gtk::EventControllerMotion::new();
        let app = Rc::downgrade(self);
        hover.connect_enter(move |_, _, _| {
            if let Some(app) = app.upgrade() {
                app.pointer_on_volume.set(true);
                app.ui.volume_popup.popup();
            }
        });
        let app = Rc::downgrade(self);
        hover.connect_leave(move |_| {
            if let Some(app) = app.upgrade() {
                app.leave_volume();
            }
        });
        self.ui.volume.add_controller(hover);
        let hover = gtk::EventControllerMotion::new();
        let app = Rc::downgrade(self);
        hover.connect_enter(move |_, _, _| {
            if let Some(app) = app.upgrade() {
                app.pointer_on_volume.set(true);
            }
        });
        let app = Rc::downgrade(self);
        hover.connect_leave(move |_| {
            if let Some(app) = app.upgrade() {
                app.leave_volume();
            }
        });
        self.ui.volume_popup.add_controller(hover);

        let popup = self.ui.volume_popup.clone();
        self.ui.volume.connect_destroy(move |_| popup.unparent());

        // Double click on the cover toggles fullscreen, as in mpv.
        let click = gtk::GestureClick::new();
        let window = self.window.clone();
        click.connect_pressed(move |_, n, _, _| {
            if n == 2 {
                window.set_fullscreened(!window.is_fullscreen());
            }
        });
        self.ui.cover_stack.add_controller(click);
    }

    fn leave_volume(self: &Rc<Self>) {
        self.pointer_on_volume.set(false);
        // Give the pointer time to cross from the button to the popup.
        let app = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(app) = app.upgrade()
                && !app.pointer_on_volume.get()
            {
                app.ui.volume_popup.popdown();
                app.schedule_osc_hide();
            }
        });
    }

    /// Runs a command from a button: same as the key, without the OSD.
    fn run_quietly(self: &Rc<Self>, command: &Command) {
        self.run(command);
        if let Some(id) = self.osd_timer.take() {
            id.remove();
        }
        self.ui.osd.set_visible(false);
    }

    /// The window's text size, and with it every size in `style.css`,
    /// follows the display's text scaling and, in the cover mode, grows
    /// with the window.
    fn connect_scaling(self: &Rc<Self>) {
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(&self.window),
            &self.scale_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        let (w, h) = self.cover_size.get();
        self.window.set_default_size(w, h);
        self.update_scale();
        let app = Rc::downgrade(self);
        let update = move || {
            if let Some(app) = app.upgrade() {
                app.update_scale();
            }
        };
        let update = Rc::new(update);
        let u = update.clone();
        self.window.connect_default_width_notify(move |_| u());
        let u = update.clone();
        self.window.connect_default_height_notify(move |_| u());
        if let Some(settings) = gtk::Settings::default() {
            settings.connect_gtk_xft_dpi_notify(move |_| update());
        }
    }

    fn update_scale(&self) {
        let scale = match self.mode.get() {
            Mode::Cover => {
                let (base, _) = em_px(COVER_EM);
                let size = self
                    .window
                    .default_width()
                    .min(self.window.default_height());
                if size > 0 {
                    (size as f64 / base as f64).clamp(SCALE_RANGE.0, SCALE_RANGE.1)
                } else {
                    1.0
                }
            }
            Mode::Library => 1.0,
        };
        let pt = format!("{:.2}", BASE_FONT_PT * scale);
        if self.scale_pt.replace(pt.clone()) != pt {
            // load_from_string needs GTK 4.12; muso supports 4.10.
            #[allow(deprecated)]
            self.scale_css
                .load_from_data(&format!("window.muso {{ font-size: {pt}pt; }}"));
        }
    }

    fn connect_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let app = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(app) = app.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let Some(name) = keys::name(key, state) else {
                return glib::Propagation::Proceed;
            };
            if name == "ESC" && app.ui.help.is_visible() {
                app.ui.help.set_visible(false);
                return glib::Propagation::Stop;
            }
            // Typed text goes to the search field, not to the bindings.
            if app.typing() {
                return if app.search_key(&name) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                };
            }
            match app.bindings.get(&name).cloned() {
                Some(command) => {
                    app.run(&command);
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        });
        self.window.add_controller(keys);

        // The mouse wheel goes through input.conf too, as WHEEL_UP and
        // WHEEL_DOWN. Touchpads scroll in fractions; a whole step is one press.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let app = Rc::downgrade(self);
        let pending = Cell::new(0.0);
        scroll.connect_scroll(move |scroll, _, dy| {
            let Some(app) = app.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let state = scroll.current_event_state();
            pending.set(pending.get() + dy);
            while pending.get().abs() >= 1.0 {
                let (wheel, step) = if pending.get() < 0.0 {
                    ("WHEEL_UP", 1.0)
                } else {
                    ("WHEEL_DOWN", -1.0)
                };
                pending.set(pending.get() + step);
                let name = keys::with_modifiers(wheel, state);
                if let Some(command) = app.bindings.get(&name).cloned() {
                    app.run(&command);
                }
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(scroll);
    }

    /// The OSC shows while the pointer moves over the window and hides
    /// `osc-hide` seconds after it stops, along with the pointer.
    fn connect_pointer(self: &Rc<Self>) {
        let motion = gtk::EventControllerMotion::new();
        let app = Rc::downgrade(self);
        let last = Cell::new((f64::NAN, f64::NAN));
        motion.connect_motion(move |_, x, y| {
            // Some compositors repeat the last position; that's not movement.
            if last.replace((x, y)) == (x, y) {
                return;
            }
            if let Some(app) = app.upgrade() {
                app.show_osc();
            }
        });
        let app = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(app) = app.upgrade()
                && !app.ui.volume_popup.is_visible()
                && !app.ui.menu.is_active()
            {
                app.ui.osc.set_reveal_child(false);
            }
        });
        self.ui.root.add_controller(motion);

        let on_panel = gtk::EventControllerMotion::new();
        let app = Rc::downgrade(self);
        on_panel.connect_enter(move |_, _, _| {
            if let Some(app) = app.upgrade() {
                app.pointer_on_osc.set(true);
            }
        });
        let app = Rc::downgrade(self);
        on_panel.connect_leave(move |_| {
            if let Some(app) = app.upgrade() {
                app.pointer_on_osc.set(false);
            }
        });
        self.ui.osc_panel.add_controller(on_panel);

        let app = Rc::downgrade(self);
        self.ui.menu.connect_active_notify(move |menu| {
            if !menu.is_active()
                && let Some(app) = app.upgrade()
            {
                app.schedule_osc_hide();
            }
        });
    }

    fn show_osc(self: &Rc<Self>) {
        self.ui.osc.set_reveal_child(true);
        self.ui.root.set_cursor(None);
        self.schedule_osc_hide();
    }

    fn schedule_osc_hide(self: &Rc<Self>) {
        if let Some(id) = self.osc_timer.take() {
            id.remove();
        }
        let app = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(self.config.osc_hide, move || {
            let Some(app) = app.upgrade() else { return };
            app.osc_timer.take();
            let busy = app.pointer_on_osc.get()
                || app.ui.volume_popup.is_visible()
                || app.ui.menu.is_active();
            if !busy {
                app.ui.osc.set_reveal_child(false);
                app.ui.root.set_cursor_from_name(Some("none"));
            }
        });
        *self.osc_timer.borrow_mut() = Some(id);
    }

    fn connect_drop(self: &Rc<Self>) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let app = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let (Some(app), Ok(files)) = (app.upgrade(), value.get::<gdk::FileList>()) else {
                return false;
            };
            let paths: Vec<PathBuf> = files.files().iter().filter_map(|f| f.path()).collect();
            app.open(&paths);
            true
        });
        self.window.add_controller(target);
    }

    fn add_actions(self: &Rc<Self>) {
        let action = |name: &str, f: fn(&Rc<App>)| {
            let a = gio::SimpleAction::new(name, None);
            let app = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(app) = app.upgrade() {
                    f(&app);
                }
            });
            self.window.add_action(&a);
        };
        action("open", |app| app.choose(false));
        action("open-folder", |app| app.choose(true));
        action("help", |app| app.ui.help.set_visible(true));
        action("library", |app| {
            app.set_mode(Mode::Library);
            app.show_section(library_ui::SECTION_LIBRARY);
        });
        action("queue", |app| {
            app.set_mode(Mode::Library);
            app.show_section(library_ui::SECTION_QUEUE);
        });
    }
}

/// A size in the design's em in pixels, at the display's text scale
/// (GNOME's "Large text" and the like raise its dpi).
fn em_px((w, h): (f64, f64)) -> (i32, i32) {
    let dpi = gtk::Settings::default()
        .map(|s| s.gtk_xft_dpi())
        .filter(|d| *d > 0)
        .map_or(96.0, |d| d as f64 / 1024.0);
    let px = |em: f64| (em * BASE_FONT_PT * dpi / 72.0).round() as i32;
    (px(w), px(h))
}

/// What a command does, for the key help.
fn describe(command: &Command) -> String {
    use muso_core::input::Direction;
    let signed = |n: f64| {
        if n < 0.0 {
            format!("\u{2212}{}", -n)
        } else {
            format!("+{n}")
        }
    };
    match command {
        Command::CyclePause => tr("Play or pause"),
        Command::Seek(secs) => tr_args("Seek {} s", &[&signed(*secs)]),
        Command::AddVolume(points) => tr_args("Volume {}", &[&signed(*points as f64)]),
        Command::PlaylistPrev => tr("Previous track"),
        Command::PlaylistNext => tr("Next track"),
        Command::CycleMute => tr("Mute"),
        Command::CycleShuffle => tr("Shuffle"),
        Command::CycleRepeat => tr("Repeat"),
        Command::CycleOntop => tr("Always on top"),
        Command::Show(View::Playlists) => tr("Playlists"),
        Command::Show(View::Library) => tr("Library"),
        Command::Show(View::Queue) => tr("Queue"),
        Command::Search => tr("Search"),
        Command::Open => tr("Open files…"),
        Command::PlayNext => tr("Play next"),
        Command::Move(Direction::Up) => tr("Move up"),
        Command::Move(Direction::Down) => tr("Move down"),
        Command::Remove => tr("Remove"),
        Command::Settings => tr("Settings"),
        Command::Help => tr("Keyboard shortcuts"),
        Command::Ignore => String::new(),
    }
}
