//! The player window: wires the core's queue and config, mpv and the widgets
//! together.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use muso_core::config::{Config, Theme};
use muso_core::input::{Bindings, Command};
use muso_core::player::{Player, PlayerEvent};
use muso_core::queue::{Queue, Repeat};
use muso_core::track::{self, TrackInfo};
use muso_mpv::Mpv;

use crate::icons::Icon;
use crate::keys;
use crate::ui::{self, Ui};

const OSD_TIME: Duration = Duration::from_millis(1200);
const VOLUME_STEP_SCROLL: i32 = 2;

pub struct App {
    window: gtk::ApplicationWindow,
    ui: Ui,
    player: RefCell<Mpv>,
    queue: RefCell<Queue<PathBuf>>,
    config: Config,
    bindings: Bindings,
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
    pub fn new(app: &gtk::Application, config: Config, bindings: Bindings, player: Mpv) -> Rc<App> {
        let ui = ui::build();
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("muso")
            .default_width(480)
            .default_height(480)
            .css_classes(["muso"])
            .child(&ui.root)
            .build();
        if config.theme == Theme::Light {
            window.add_css_class("light");
        }
        let seed = glib::real_time() as u64;
        let app = Rc::new(App {
            window,
            ui,
            player: RefCell::new(player),
            queue: RefCell::new(Queue::new(seed)),
            config,
            bindings,
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
        app.connect_player();
        app.connect_widgets();
        app.connect_keys();
        app.connect_pointer();
        app.connect_drop();
        app.add_actions();
        app.fill_help();
        app.show_empty();
        app.update_volume();
        app
    }

    pub fn present(&self) {
        self.window.present();
    }

    /// Replaces the queue with `paths` (folders are searched for music) and
    /// plays the first track.
    pub fn open(&self, paths: &[PathBuf]) {
        let files = track::collect(paths);
        if files.is_empty() {
            self.osd("No music found");
            return;
        }
        self.queue.borrow_mut().replace(files);
        self.play_index(0);
    }

    /// Fills the queue from `library` folders in muso.conf without playing.
    pub fn load_library(&self) {
        let files = track::collect(&self.config.library);
        if let Some(first) = files.first() {
            self.show_track(&TrackInfo::read(first));
            self.queue.borrow_mut().replace(files);
        }
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
    }

    fn stop(&self) {
        self.queue.borrow_mut().stop();
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
        let socket = match self.player.borrow_mut().wakeup_socket() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("muso: can't watch mpv events: {e}");
                return;
            }
        };
        let socket = match gio::Socket::from_fd(socket.into()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("muso: can't watch mpv events: {e}");
                return;
            }
        };
        // Draining it below must not wait for more.
        socket.set_blocking(false);
        let app = Rc::downgrade(self);
        let source = gio::prelude::SocketExtManual::create_source(
            &socket,
            glib::IOCondition::IN,
            None::<&gio::Cancellable>,
            Some("mpv events"),
            glib::Priority::DEFAULT,
            move |socket, _| {
                let mut buf = [0u8; 64];
                while matches!(socket.receive(&mut buf, None::<&gio::Cancellable>), Ok(n) if n > 0)
                {
                }
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
            },
        );
        source.attach(None);
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
                    self.osd(&format!("Can't play: {e}"));
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
                self.osd(if self.paused.get() {
                    "Paused"
                } else {
                    "Playing"
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
                self.osd(&format!("Volume {}%", self.volume.get()));
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
                self.osd(if self.muted.get() { "Mute" } else { "Unmute" });
            }
            Command::CycleShuffle => {
                let on = !self.queue.borrow().shuffle();
                self.queue.borrow_mut().set_shuffle(on);
                self.ui.shuffle.set_on(on);
                self.refresh_upcoming();
                self.osd(if on { "Shuffle on" } else { "Shuffle off" });
            }
            Command::CycleRepeat => {
                let repeat = self.queue.borrow().repeat().cycle();
                self.queue.borrow_mut().set_repeat(repeat);
                self.update_repeat();
                self.refresh_upcoming();
                self.osd(match repeat {
                    Repeat::Off => "Repeat off",
                    Repeat::All => "Repeat all",
                    Repeat::One => "Repeat one",
                });
            }
            Command::CycleOntop => self.osd("GTK 4 can't keep a window on top"),
            Command::Open => self.choose(false),
            Command::Help => self.ui.help.set_visible(!self.ui.help.is_visible()),
            Command::Show(_)
            | Command::Search
            | Command::PlayNext
            | Command::Move(_)
            | Command::Remove
            | Command::Settings => self.osd("Not in this version yet"),
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
            .set_label("Drop music here\nor press O to open");
        self.ui.cover_stack.set_visible_child_name("placeholder");
        self.ui.title.set_label("Nothing playing");
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

        let texture = track::cover(&info.path)
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
            .set_tooltip_text(Some(if playing { "Pause" } else { "Play" }));
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
        let id = glib::timeout_add_local_once(OSD_TIME, move || {
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
                .label(command.to_string())
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

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let app = Rc::downgrade(self);
        scroll.connect_scroll(move |_, _, dy| {
            if let Some(app) = app.upgrade() {
                app.add_volume(if dy < 0.0 {
                    VOLUME_STEP_SCROLL
                } else {
                    -VOLUME_STEP_SCROLL
                });
            }
            glib::Propagation::Stop
        });
        self.ui.volume.add_controller(scroll);

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
            match app.bindings.get(&name).cloned() {
                Some(command) => {
                    app.run(&command);
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        });
        self.window.add_controller(keys);
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
    }
}

/// `mm:ss`, or `h:mm:ss` from an hour up.
fn format_time(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_like_the_design() {
        assert_eq!(format_time(Duration::from_secs(0)), "00:00");
        assert_eq!(format_time(Duration::from_millis(134_900)), "02:14");
        assert_eq!(format_time(Duration::from_secs(3 * 3600 + 5)), "3:00:05");
    }
}
