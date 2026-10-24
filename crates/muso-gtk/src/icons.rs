//! The design's 16-unit stroke icons, drawn with cairo in the widget's CSS
//! colour, so they need no icon theme or SVG loader.

use std::cell::Cell;
use std::f64::consts::{FRAC_PI_2, PI};
use std::rc::Rc;

use gtk::cairo::Context;
use gtk::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// A drawing area showing one icon at `size` pixels; [`IconView::set`]
/// swaps it.
#[derive(Clone)]
pub struct IconView {
    pub area: gtk::DrawingArea,
    icon: Rc<Cell<Icon>>,
}

impl IconView {
    pub fn new(icon: Icon, size: i32) -> Self {
        let area = gtk::DrawingArea::builder()
            .content_width(size)
            .content_height(size)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .can_target(false)
            .build();
        let icon = Rc::new(Cell::new(icon));
        let shown = icon.clone();
        area.set_draw_func(move |area, cr, w, h| {
            let color = area.color();
            cr.set_source_rgba(
                color.red().into(),
                color.green().into(),
                color.blue().into(),
                color.alpha().into(),
            );
            cr.scale(w as f64 / 16.0, h as f64 / 16.0);
            cr.set_line_width(1.5);
            draw(cr, shown.get());
        });
        IconView { area, icon }
    }

    pub fn set(&self, icon: Icon) {
        if self.icon.replace(icon) != icon {
            self.area.queue_draw();
        }
    }
}

fn draw(cr: &Context, icon: Icon) {
    let lines = |points: &[(f64, f64)]| {
        cr.move_to(points[0].0, points[0].1);
        for &(x, y) in &points[1..] {
            cr.line_to(x, y);
        }
    };
    match icon {
        Icon::Shuffle => {
            lines(&[(2.0, 4.0), (5.0, 4.0), (11.0, 12.0), (14.0, 12.0)]);
            lines(&[(2.0, 12.0), (5.0, 12.0), (11.0, 4.0), (14.0, 4.0)]);
            lines(&[(12.0, 2.0), (14.0, 4.0), (12.0, 6.0)]);
            lines(&[(12.0, 10.0), (14.0, 12.0), (12.0, 14.0)]);
        }
        Icon::Prev => {
            lines(&[(3.0, 3.0), (3.0, 13.0)]);
            lines(&[(13.0, 3.0), (6.0, 8.0), (13.0, 13.0)]);
            cr.close_path();
        }
        Icon::Play => {
            lines(&[(4.0, 2.5), (4.0, 13.5), (13.0, 8.0)]);
            cr.close_path();
        }
        Icon::Pause => {
            lines(&[(5.0, 3.0), (5.0, 13.0)]);
            lines(&[(11.0, 3.0), (11.0, 13.0)]);
        }
        Icon::Next => {
            lines(&[(13.0, 3.0), (13.0, 13.0)]);
            lines(&[(3.0, 3.0), (10.0, 8.0), (3.0, 13.0)]);
            cr.close_path();
        }
        Icon::Repeat => {
            cr.move_to(3.0, 8.0);
            cr.line_to(3.0, 6.0);
            cr.arc(5.0, 6.0, 2.0, PI, 1.5 * PI);
            cr.line_to(13.0, 4.0);
            lines(&[(11.0, 2.0), (13.0, 4.0), (11.0, 6.0)]);
            cr.move_to(13.0, 8.0);
            cr.line_to(13.0, 10.0);
            cr.arc(11.0, 10.0, 2.0, 0.0, FRAC_PI_2);
            cr.line_to(3.0, 12.0);
            lines(&[(5.0, 14.0), (3.0, 12.0), (5.0, 10.0)]);
        }
        Icon::Volume | Icon::Muted => {
            lines(&[
                (2.0, 6.0),
                (5.0, 6.0),
                (9.0, 3.0),
                (9.0, 13.0),
                (5.0, 10.0),
                (2.0, 10.0),
            ]);
            cr.close_path();
            if icon == Icon::Volume {
                // The two sound waves: arcs through (x, 8 ± h) of radius r.
                for (x, h, r) in [(11.5, 2.5, 3.5), (13.5, 4.5, 6.5)] {
                    let d = f64::sqrt(r * r - h * h);
                    let a = f64::atan2(h, d);
                    cr.new_sub_path();
                    cr.arc(x - d, 8.0, r, -a, a);
                }
            } else {
                lines(&[(11.0, 6.0), (15.0, 10.0)]);
                lines(&[(15.0, 6.0), (11.0, 10.0)]);
            }
        }
        Icon::Menu => {
            for x in [3.0, 8.0, 13.0] {
                cr.new_sub_path();
                cr.arc(x, 8.0, 1.3, 0.0, 2.0 * PI);
            }
            let _ = cr.fill();
            return;
        }
    }
    let _ = cr.stroke();
}
