//! The cover window's widgets: the cover, the OSC on top of it, the OSD and
//! the key help. Only structure lives here: sizes and spacing are in
//! `style.css`, relative to the font size, and [`crate::app`] fills the
//! widgets in.

use gtk::prelude::*;
use gtk::{Align, Orientation, gio};

use crate::i18n::tr;
use crate::icons::{Icon, IconView};
use crate::slider::Slider;

pub struct Ui {
    pub root: gtk::Overlay,
    pub cover: gtk::Picture,
    pub placeholder: gtk::Label,
    pub cover_stack: gtk::Stack,
    pub osc: gtk::Revealer,
    pub osc_panel: gtk::Box,
    pub title: gtk::Label,
    pub subline: gtk::Label,
    pub elapsed: gtk::Label,
    pub remaining: gtk::Label,
    pub progress: Slider,
    pub shuffle: ToggleIcon,
    pub prev: gtk::Button,
    pub play: gtk::Button,
    pub play_icon: IconView,
    pub next: gtk::Button,
    pub repeat: ToggleIcon,
    pub repeat_one: gtk::Label,
    pub volume: gtk::Button,
    pub volume_icon: IconView,
    pub volume_popup: gtk::Popover,
    pub volume_slider: Slider,
    pub volume_value: gtk::Label,
    pub menu: gtk::MenuButton,
    pub osd: gtk::Label,
    pub help: gtk::Box,
    pub help_grid: gtk::Grid,
}

/// An icon button that shows a dot under the icon when on.
pub struct ToggleIcon {
    pub button: gtk::Button,
    pub overlay: gtk::Overlay,
    dot: gtk::Box,
}

impl ToggleIcon {
    fn new(icon: Icon, tooltip: &str) -> Self {
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&IconView::new(icon, "small").area));
        let dot = gtk::Box::builder()
            .css_classes(["dot"])
            .halign(Align::Start)
            .valign(Align::End)
            .can_target(false)
            .build();
        overlay.add_overlay(&dot);
        let button = icon_button(&overlay, tooltip, &[]);
        let toggle = ToggleIcon {
            button,
            overlay,
            dot,
        };
        toggle.set_on(false);
        toggle
    }

    pub fn set_on(&self, on: bool) {
        self.dot.set_visible(on);
        if on {
            self.button.remove_css_class("inactive");
        } else {
            self.button.add_css_class("inactive");
        }
    }
}

fn icon_button(child: &impl IsA<gtk::Widget>, tooltip: &str, classes: &[&str]) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(child)
        .tooltip_text(tooltip)
        .focusable(false)
        .css_classes(["icon"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    for class in classes {
        button.add_css_class(class);
    }
    button
}

fn label(classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .css_classes(classes.to_vec())
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .single_line_mode(true)
        .build()
}

pub fn build() -> Ui {
    let cover = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .build();
    let placeholder = gtk::Label::builder()
        .css_classes(["cover-placeholder"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    let cover_stack = gtk::Stack::new();
    cover_stack.add_named(&placeholder, Some("placeholder"));
    cover_stack.add_named(&cover, Some("cover"));

    let title = label(&["title"]);
    let subline = label(&["subline"]);
    let info = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["info"])
        .build();
    info.append(&title);
    info.append(&subline);

    let elapsed = gtk::Label::builder().css_classes(["time"]).build();
    let remaining = gtk::Label::builder()
        .css_classes(["time", "remaining"])
        .build();
    let progress = Slider::new(Orientation::Horizontal, false);
    let progress_row = gtk::Box::builder().css_classes(["progress-row"]).build();
    progress_row.append(&elapsed);
    progress_row.append(&progress.area);
    progress_row.append(&remaining);

    let shuffle = ToggleIcon::new(Icon::Shuffle, &tr("Shuffle"));
    let prev = icon_button(
        &IconView::new(Icon::Prev, "medium").area,
        &tr("Previous"),
        &["wide"],
    );
    let play_icon = IconView::new(Icon::Play, "large");
    let play = icon_button(&play_icon.area, &tr("Play"), &["play"]);
    let next = icon_button(
        &IconView::new(Icon::Next, "medium").area,
        &tr("Next"),
        &["wide"],
    );
    let repeat = ToggleIcon::new(Icon::Repeat, &tr("Repeat"));
    let repeat_one = gtk::Label::builder()
        .label("1")
        .css_classes(["repeat-one"])
        .halign(Align::End)
        .valign(Align::Start)
        .can_target(false)
        .visible(false)
        .build();
    repeat.overlay.add_overlay(&repeat_one);
    let buttons = gtk::Box::builder().halign(Align::Center).build();
    for b in [&shuffle.button, &prev, &play, &next, &repeat.button] {
        buttons.append(b);
    }

    let transport = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["transport"])
        .build();
    transport.append(&progress_row);
    transport.append(&buttons);

    let volume_icon = IconView::new(Icon::Volume, "small");
    let volume = icon_button(&volume_icon.area, &tr("Volume"), &[]);
    let volume_value = gtk::Label::builder().css_classes(["value"]).build();
    let volume_slider = Slider::new(Orientation::Vertical, true);
    let volume_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["volume-box"])
        .build();
    volume_box.append(&volume_value);
    volume_box.append(&volume_slider.area);
    let volume_popup = gtk::Popover::builder()
        .child(&volume_box)
        .has_arrow(false)
        .autohide(false)
        .position(gtk::PositionType::Top)
        .css_classes(["volume-popup"])
        .build();
    volume_popup.set_parent(&volume);

    let menu_model = gio::Menu::new();
    menu_model.append(Some(&tr("Open files…")), Some("win.open"));
    menu_model.append(Some(&tr("Open folder…")), Some("win.open-folder"));
    menu_model.append(Some(&tr("Keyboard shortcuts")), Some("win.help"));
    menu_model.append(Some(&tr("Quit")), Some("app.quit"));
    let menu = gtk::MenuButton::builder()
        .child(&IconView::new(Icon::Menu, "small").area)
        .menu_model(&menu_model)
        .tooltip_text(tr("Menu"))
        .focusable(false)
        .css_classes(["icon"])
        .build();
    menu.set_direction(gtk::ArrowType::Up);
    if let Some(popover) = menu.popover() {
        popover.set_has_arrow(false);
    }
    if let Some(button) = menu.first_child() {
        button.add_css_class("icon");
    }

    let right = gtk::Box::builder()
        .halign(Align::End)
        .valign(Align::End)
        .build();
    right.append(&volume);
    right.append(&menu);

    let controls = gtk::CenterBox::new();
    controls.set_center_widget(Some(&transport));
    controls.set_end_widget(Some(&right));

    let osc_panel = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["osc"])
        .build();
    osc_panel.append(&info);
    osc_panel.append(&controls);
    let osc = gtk::Revealer::builder()
        .child(&osc_panel)
        .transition_type(gtk::RevealerTransitionType::Crossfade)
        .transition_duration(150)
        .valign(Align::End)
        .css_classes(["osc-revealer"])
        .build();

    let osd = gtk::Label::builder()
        .css_classes(["osd"])
        .halign(Align::Start)
        .valign(Align::Start)
        .can_target(false)
        .visible(false)
        .build();

    let help_grid = gtk::Grid::new();
    let help_scroll = gtk::ScrolledWindow::builder()
        .child(&help_grid)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(true)
        .propagate_natural_height(true)
        .build();
    let help = gtk::Box::builder()
        .css_classes(["help"])
        .halign(Align::Center)
        .valign(Align::Center)
        .visible(false)
        .build();
    help.append(&help_scroll);

    let root = gtk::Overlay::new();
    root.set_child(Some(&cover_stack));
    root.add_overlay(&osc);
    root.add_overlay(&osd);
    root.add_overlay(&help);

    Ui {
        root,
        cover,
        placeholder,
        cover_stack,
        osc,
        osc_panel,
        title,
        subline,
        elapsed,
        remaining,
        progress,
        shuffle,
        prev,
        play,
        play_icon,
        next,
        repeat,
        repeat_one,
        volume,
        volume_icon,
        volume_popup,
        volume_slider,
        volume_value,
        menu,
        osd,
        help,
        help_grid,
    }
}
