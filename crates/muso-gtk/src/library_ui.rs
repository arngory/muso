//! The library window's widgets: search on top, then sections, artists,
//! albums and tracks, and the controls in a bar at the bottom. As in
//! [`crate::ui`], only structure lives here; sizes are in `style.css`.

use gtk::prelude::*;
use gtk::{Align, Orientation};

use crate::i18n::tr;
use crate::icons::{Icon, IconView};
use crate::slider::Slider;
use crate::track_list::{Columns, TrackList};

pub struct LibraryUi {
    pub root: gtk::Box,
    pub topbar: gtk::Box,
    pub search: gtk::Entry,
    pub count: gtk::Label,
    pub scan: gtk::Revealer,
    pub scan_label: gtk::Label,
    pub scan_progress: Slider,
    pub scan_count: gtk::Label,
    pub sidebar: gtk::ListBox,
    pub library_count: gtk::Label,
    pub queue_count: gtk::Label,
    pub artists_column: gtk::Box,
    pub artists: gtk::ListView,
    pub artist_names: gtk::StringList,
    pub artist_selection: gtk::SingleSelection,
    pub content: gtk::Stack,
    pub albums: gtk::Box,
    pub tracks: TrackList,
    pub found_artists_title: gtk::Label,
    pub found_artists: gtk::Box,
    pub found_albums_title: gtk::Label,
    pub found_albums: gtk::Box,
    pub found_tracks_title: gtk::Label,
    pub found_tracks: TrackList,
    pub queue: TrackList,
    pub empty: gtk::Label,
    pub bar: gtk::CenterBox,
    pub bar_start: gtk::Box,
    pub thumb: gtk::Image,
    pub bar_end: gtk::Box,
    pub collapse: gtk::Button,
}

/// Rows of the sidebar, in order.
pub const SECTION_LIBRARY: i32 = 0;
pub const SECTION_QUEUE: i32 = 1;

/// One album in a row of covers: the cover, title and year.
pub struct AlbumTile {
    pub button: gtk::Button,
    pub cover: gtk::Image,
}

impl AlbumTile {
    pub fn new(title: &str, year: Option<u32>) -> AlbumTile {
        let cover = gtk::Image::builder().css_classes(["cover"]).build();
        let name = gtk::Label::builder()
            .label(title)
            .css_classes(["name"])
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .single_line_mode(true)
            .build();
        // The cover sets the width; a long title is cut, not widening it.
        name.set_max_width_chars(1);
        let year = gtk::Label::builder()
            .label(year.map(|y| y.to_string()).unwrap_or_default())
            .css_classes(["year"])
            .xalign(0.0)
            .build();
        let column = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .build();
        column.append(&cover);
        column.append(&name);
        column.append(&year);
        let button = gtk::Button::builder()
            .child(&column)
            .tooltip_text(title)
            .css_classes(["album"])
            .valign(Align::Start)
            .build();
        AlbumTile { button, cover }
    }
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text.to_uppercase())
        .css_classes(["heading"])
        .xalign(0.0)
        .build()
}

fn section(name: &str, count: &gtk::Label) -> gtk::ListBoxRow {
    let row = gtk::Box::new(Orientation::Horizontal, 0);
    row.append(
        &gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    row.append(count);
    gtk::ListBoxRow::builder().child(&row).build()
}

fn albums_row() -> (gtk::ScrolledWindow, gtk::Box) {
    let albums = gtk::Box::builder().css_classes(["albums"]).build();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&albums)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .build();
    (scroll, albums)
}

pub fn build() -> LibraryUi {
    let prompt = gtk::Label::builder()
        .label("/")
        .css_classes(["prompt"])
        .build();
    let search = gtk::Entry::builder()
        .placeholder_text(tr("Search: artist, album, title"))
        .hexpand(true)
        .css_classes(["search"])
        .build();
    let count = gtk::Label::builder().css_classes(["count"]).build();
    let topbar = gtk::Box::builder().css_classes(["topbar"]).build();
    topbar.append(&prompt);
    topbar.append(&search);
    topbar.append(&count);

    let scan_label = gtk::Label::builder()
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    let scan_progress = Slider::new(Orientation::Horizontal, false);
    scan_progress.area.set_can_target(false);
    scan_progress.area.set_hexpand(false);
    let scan_count = gtk::Label::new(None);
    let scan_note = gtk::Label::builder()
        .label(tr("You can listen while it scans"))
        .hexpand(true)
        .xalign(1.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let scan_row = gtk::Box::builder().css_classes(["scanbar"]).build();
    scan_row.append(&scan_label);
    scan_row.append(&scan_progress.area);
    scan_row.append(&scan_count);
    scan_row.append(&scan_note);
    let scan = gtk::Revealer::builder().child(&scan_row).build();

    let library_count = gtk::Label::builder().css_classes(["count"]).build();
    let queue_count = gtk::Label::builder().css_classes(["count"]).build();
    let sidebar = gtk::ListBox::builder()
        .css_classes(["sidebar"])
        .selection_mode(gtk::SelectionMode::Browse)
        .build();
    sidebar.append(&section(&tr("Library"), &library_count));
    sidebar.append(&section(&tr("Queue"), &queue_count));

    let artist_names = gtk::StringList::new(&[]);
    let artist_selection = gtk::SingleSelection::builder()
        .model(&artist_names)
        .autoselect(false)
        .build();
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let label = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .single_line_mode(true)
            .build();
        item.set_child(Some(&label));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let (Some(label), Some(name)) = (item.child(), item.item()) else {
            return;
        };
        let (Ok(label), Ok(name)) = (
            label.downcast::<gtk::Label>(),
            name.downcast::<gtk::StringObject>(),
        ) else {
            return;
        };
        label.set_label(&name.string());
    });
    let artists = gtk::ListView::builder()
        .model(&artist_selection)
        .factory(&factory)
        .single_click_activate(true)
        .css_classes(["artists"])
        .build();
    let artists_column = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["artists-column"])
        .build();
    artists_column.append(&heading(&tr("Artists")));
    artists_column.append(
        &gtk::ScrolledWindow::builder()
            .child(&artists)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build(),
    );

    let (albums_scroll, albums) = albums_row();
    let tracks = TrackList::new(Columns {
        number: true,
        artist: false,
        album: false,
    });
    let browse = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["browse"])
        .build();
    browse.append(&albums_scroll);
    browse.append(&gtk::Separator::new(Orientation::Horizontal));
    browse.append(&tracks.root);

    let found_artists_title = heading("");
    let found_artists = gtk::Box::builder().css_classes(["found-artists"]).build();
    let found_albums_title = heading("");
    let (found_albums_scroll, found_albums) = albums_row();
    let found_tracks_title = heading("");
    let found_tracks = TrackList::new(Columns {
        number: false,
        artist: true,
        album: true,
    });
    let found = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["found"])
        .build();
    for w in [
        found_artists_title.upcast_ref::<gtk::Widget>(),
        found_artists.upcast_ref(),
        found_albums_title.upcast_ref(),
        found_albums_scroll.upcast_ref(),
        found_tracks_title.upcast_ref(),
        found_tracks.root.upcast_ref(),
    ] {
        found.append(w);
    }

    let queue = TrackList::new(Columns {
        number: false,
        artist: true,
        album: true,
    });
    queue.root.add_css_class("queue");
    let empty = gtk::Label::builder()
        .css_classes(["empty"])
        .justify(gtk::Justification::Center)
        .wrap(true)
        .build();

    let content = gtk::Stack::builder().hexpand(true).build();
    content.add_named(&browse, Some("browse"));
    content.add_named(&found, Some("search"));
    content.add_named(&queue.root, Some("queue"));
    content.add_named(&empty, Some("empty"));

    let body = gtk::Box::builder().vexpand(true).build();
    body.append(&sidebar);
    body.append(&artists_column);
    body.append(&content);

    let thumb = gtk::Image::builder()
        .css_classes(["cover", "thumb"])
        .build();
    let bar_start = gtk::Box::builder()
        .css_classes(["now"])
        .valign(Align::Center)
        .build();
    bar_start.append(&thumb);
    let collapse = gtk::Button::builder()
        .child(&IconView::new(Icon::Collapse, "small").area)
        .tooltip_text(tr("Back to the cover"))
        .focusable(false)
        .css_classes(["icon"])
        .build();
    let bar_end = gtk::Box::builder()
        .halign(Align::End)
        .valign(Align::Center)
        .build();
    bar_end.append(&collapse);
    let bar = gtk::CenterBox::builder().css_classes(["bar"]).build();
    bar.set_start_widget(Some(&bar_start));
    bar.set_end_widget(Some(&bar_end));

    let root = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["library"])
        .build();
    root.append(&topbar);
    root.append(&scan);
    root.append(&body);
    root.append(&bar);

    LibraryUi {
        root,
        topbar,
        search,
        count,
        scan,
        scan_label,
        scan_progress,
        scan_count,
        sidebar,
        library_count,
        queue_count,
        artists_column,
        artists,
        artist_names,
        artist_selection,
        content,
        albums,
        tracks,
        found_artists_title,
        found_artists,
        found_albums_title,
        found_albums,
        found_tracks_title,
        found_tracks,
        queue,
        empty,
        bar,
        bar_start,
        thumb,
        bar_end,
        collapse,
    }
}
