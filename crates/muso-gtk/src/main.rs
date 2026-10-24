//! muso for Linux: a GTK 4 window around the album cover, playing through
//! libmpv.

mod app;
mod covers;
mod i18n;
mod icons;
mod keys;
mod library_ui;
mod scanner;
mod slider;
mod svg;
mod track_list;
mod ui;
mod wakeup;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use muso_core::config::Config;
use muso_core::input::Bindings;
use muso_mpv::Mpv;

const APP_ID: &str = "io.github.arngory.muso";

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    let window: Rc<std::cell::RefCell<Option<Rc<app::App>>>> = Rc::default();

    app.connect_startup(|app| {
        let css = gtk::CssProvider::new();
        // load_from_string needs GTK 4.12; muso supports 4.10.
        #[allow(deprecated)]
        css.load_from_data(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("a display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let quit = gio::SimpleAction::new("quit", None);
        quit.connect_activate({
            let app = app.clone();
            move |_, _| app.quit()
        });
        app.add_action(&quit);
        app.set_accels_for_action("app.quit", &["<Ctrl>q"]);
    });

    app.connect_activate({
        let window = window.clone();
        move |app| {
            if let Some(w) = get_or_build(app, &window) {
                w.present();
            }
        }
    });
    app.connect_open({
        let window = window.clone();
        move |app, files, _| {
            let paths: Vec<PathBuf> = files.iter().filter_map(|f| f.path()).collect();
            if let Some(w) = get_or_build(app, &window) {
                w.present();
                w.open(&paths);
            }
        }
    });
    app.run()
}

/// The window, made on first use. A second `muso` run hands its files to
/// the first one through D-Bus, so there is only ever one.
fn get_or_build(
    app: &gtk::Application,
    slot: &std::cell::RefCell<Option<Rc<app::App>>>,
) -> Option<Rc<app::App>> {
    if let Some(w) = &*slot.borrow() {
        return Some(w.clone());
    }
    let dir = glib::user_config_dir().join("muso");
    let home = glib::home_dir();
    let (config, errors) = Config::parse(&read(&dir.join("muso.conf")), Some(&home));
    for e in errors {
        eprintln!("muso: {e}");
    }
    let (bindings, errors) = Bindings::parse(&read(&dir.join("input.conf")));
    for e in errors {
        eprintln!("muso: {e}");
    }

    let mut options = vec![(
        "gapless-audio".to_owned(),
        if config.gapless { "yes" } else { "no" }.to_owned(),
    )];
    options.extend(config.mpv.iter().cloned());
    let player = match Mpv::new(&options) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("muso: can't start mpv: {e}");
            app.quit();
            return None;
        }
    };

    let w = app::App::new(app, config, bindings, player);
    *slot.borrow_mut() = Some(w.clone());
    Some(w)
}

/// The file's text, or nothing if it doesn't exist or can't be read.
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// `mm:ss`, or `h:mm:ss` from an hour up.
pub fn format_time(d: Duration) -> String {
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
