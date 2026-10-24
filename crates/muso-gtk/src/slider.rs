//! The design's sliders: a 2px line filled up to the value, with an optional
//! round knob. Used for the progress bar and the volume popup.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

type ChangeHandler = Box<dyn Fn(f64)>;

#[derive(Clone)]
pub struct Slider {
    pub area: gtk::DrawingArea,
    value: Rc<Cell<f64>>,
    on_change: Rc<RefCell<Option<ChangeHandler>>>,
}

impl Slider {
    /// `knob` draws a 10px dot at the value. Horizontal sliders fill from
    /// the left, vertical ones from the bottom.
    pub fn new(orientation: gtk::Orientation, knob: bool) -> Self {
        let vertical = orientation == gtk::Orientation::Vertical;
        let area = gtk::DrawingArea::new();
        if vertical {
            area.set_content_width(12);
            area.set_vexpand(true);
        } else {
            // Taller than the line so it's easy to hit.
            area.set_content_height(12);
            area.set_hexpand(true);
        }
        area.set_cursor_from_name(Some("pointer"));
        let value = Rc::new(Cell::new(0.0));
        let shown = value.clone();
        area.set_draw_func(move |area, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let c = area.color();
            let (r, g, b, a) = (
                c.red() as f64,
                c.green() as f64,
                c.blue() as f64,
                c.alpha() as f64,
            );
            let v = shown.get();
            cr.set_source_rgba(r, g, b, a * 0.25);
            if vertical {
                cr.rectangle(w / 2.0 - 1.0, 0.0, 2.0, h);
            } else {
                cr.rectangle(0.0, h / 2.0 - 1.0, w, 2.0);
            }
            let _ = cr.fill();
            cr.set_source_rgba(r, g, b, a);
            if vertical {
                cr.rectangle(w / 2.0 - 1.0, h * (1.0 - v), 2.0, h * v);
            } else {
                cr.rectangle(0.0, h / 2.0 - 1.0, w * v, 2.0);
            }
            let _ = cr.fill();
            if knob {
                let (x, y) = if vertical {
                    (w / 2.0, (h * (1.0 - v)).clamp(5.0, h - 5.0))
                } else {
                    ((w * v).clamp(5.0, w - 5.0), h / 2.0)
                };
                cr.arc(x, y, 5.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
        });

        let slider = Slider {
            area,
            value,
            on_change: Rc::default(),
        };
        let drag = gtk::GestureDrag::new();
        let pick = {
            let slider = slider.clone();
            move |x: f64, y: f64| {
                let area = &slider.area;
                let v = if vertical {
                    1.0 - y / area.height().max(1) as f64
                } else {
                    x / area.width().max(1) as f64
                };
                let v = v.clamp(0.0, 1.0);
                slider.set_value(v);
                if let Some(f) = &*slider.on_change.borrow() {
                    f(v);
                }
            }
        };
        let start = Rc::new(Cell::new((0.0, 0.0)));
        drag.connect_drag_begin({
            let (pick, start) = (pick.clone(), start.clone());
            move |_, x, y| {
                start.set((x, y));
                pick(x, y);
            }
        });
        drag.connect_drag_update(move |_, dx, dy| {
            let (x, y) = start.get();
            pick(x + dx, y + dy);
        });
        slider.area.add_controller(drag);
        slider
    }

    pub fn set_value(&self, v: f64) {
        let v = v.clamp(0.0, 1.0);
        if (self.value.replace(v) - v).abs() > f64::EPSILON {
            self.area.queue_draw();
        }
    }

    /// Called with the new value while the user drags.
    pub fn connect_change(&self, f: impl Fn(f64) + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(f));
    }
}
