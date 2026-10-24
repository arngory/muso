//! The design's icons: SVG files in `icons/`, drawn as vectors in the
//! widget's CSS colour at whatever size CSS gives them. No icon theme or SVG
//! loader is needed.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;

use crate::svg::Svg;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Shuffle,
    Prev,
    Play,
    Pause,
    Next,
    Repeat,
    Volume,
    Muted,
    Menu,
    Library,
    Collapse,
}

impl Icon {
    #[cfg(test)]
    pub const ALL: [Icon; 11] = [
        Icon::Shuffle,
        Icon::Prev,
        Icon::Play,
        Icon::Pause,
        Icon::Next,
        Icon::Repeat,
        Icon::Volume,
        Icon::Muted,
        Icon::Menu,
        Icon::Library,
        Icon::Collapse,
    ];

    pub fn source(self) -> &'static str {
        match self {
            Icon::Shuffle => include_str!("../icons/shuffle.svg"),
            Icon::Prev => include_str!("../icons/prev.svg"),
            Icon::Play => include_str!("../icons/play.svg"),
            Icon::Pause => include_str!("../icons/pause.svg"),
            Icon::Next => include_str!("../icons/next.svg"),
            Icon::Repeat => include_str!("../icons/repeat.svg"),
            Icon::Volume => include_str!("../icons/volume.svg"),
            Icon::Muted => include_str!("../icons/muted.svg"),
            Icon::Menu => include_str!("../icons/menu.svg"),
            Icon::Library => include_str!("../icons/library.svg"),
            Icon::Collapse => include_str!("../icons/collapse.svg"),
        }
    }

    fn svg(self) -> Rc<Svg> {
        thread_local! {
            static PARSED: RefCell<HashMap<Icon, Rc<Svg>>> = RefCell::default();
        }
        PARSED.with(|parsed| {
            parsed
                .borrow_mut()
                .entry(self)
                .or_insert_with(|| {
                    // The icons are part of the build and covered by a test.
                    Rc::new(Svg::parse(self.source()).expect("icon SVG parses"))
                })
                .clone()
        })
    }
}

/// A drawing area showing one icon; [`IconView::set`] swaps it. Its size
/// comes from CSS through the `class` given to [`IconView::new`].
#[derive(Clone)]
pub struct IconView {
    pub area: gtk::DrawingArea,
    icon: Rc<Cell<Icon>>,
}

impl IconView {
    pub fn new(icon: Icon, class: &str) -> Self {
        let area = gtk::DrawingArea::builder()
            .css_classes(["glyph", class])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .can_target(false)
            .build();
        let icon = Rc::new(Cell::new(icon));
        let shown = icon.clone();
        area.set_draw_func(move |area, cr, w, h| {
            let c = area.color();
            cr.set_source_rgba(
                c.red().into(),
                c.green().into(),
                c.blue().into(),
                c.alpha().into(),
            );
            shown.get().svg().draw(cr, w as f64, h as f64);
        });
        IconView { area, icon }
    }

    pub fn set(&self, icon: Icon) {
        if self.icon.replace(icon) != icon {
            self.area.queue_draw();
        }
    }
}
