//! Just enough SVG to draw muso's icons with cairo at any size: `<path>`
//! elements with `M L H V C Z A` commands, filled or stroked in the current
//! colour. Icons stay plain SVG files a designer can open and edit.

use std::f64::consts::{PI, TAU};

use gtk::cairo::Context;

#[derive(Debug, Clone, PartialEq)]
pub struct Svg {
    /// The viewBox: (x, y, width, height).
    view_box: (f64, f64, f64, f64),
    paths: Vec<Path>,
}

#[derive(Debug, Clone, PartialEq)]
struct Path {
    segments: Vec<Segment>,
    fill: bool,
    stroke: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Segment {
    Move(f64, f64),
    Line(f64, f64),
    Curve(f64, f64, f64, f64, f64, f64),
    /// An elliptical arc, as cairo wants it: centre, radii, rotation, and
    /// start and end angles in the ellipse's own frame.
    Arc {
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
        rotation: f64,
        from: f64,
        to: f64,
    },
    Close,
}

impl Svg {
    pub fn parse(source: &str) -> Result<Svg, String> {
        let root = tag(source, "svg").ok_or("no <svg> element")?;
        let view_box = match attr(root, "viewBox") {
            Some(v) => {
                let n = numbers(v)?;
                let [x, y, w, h] = n[..] else {
                    return Err(format!("bad viewBox {v:?}"));
                };
                (x, y, w, h)
            }
            None => (
                0.0,
                0.0,
                number_attr(root, "width")?.unwrap_or(16.0),
                number_attr(root, "height")?.unwrap_or(16.0),
            ),
        };
        let root_fill = attr(root, "fill").is_none_or(|f| f != "none");
        let root_stroke = attr(root, "stroke").is_some_and(|s| s != "none");
        let root_width = number_attr(root, "stroke-width")?.unwrap_or(1.0);

        let mut paths = Vec::new();
        let mut rest = source;
        while let Some(start) = rest.find("<path") {
            let end = rest[start..].find('>').ok_or("unclosed <path>")? + start;
            let element = &rest[start..end];
            rest = &rest[end..];
            let d = attr(element, "d").ok_or("<path> without d")?;
            let fill = attr(element, "fill").map_or(root_fill, |f| f != "none");
            let stroked = attr(element, "stroke").map_or(root_stroke, |s| s != "none");
            let width = number_attr(element, "stroke-width")?.unwrap_or(root_width);
            paths.push(Path {
                segments: parse_path(d)?,
                fill,
                stroke: stroked.then_some(width),
            });
        }
        Ok(Svg { view_box, paths })
    }

    /// Draws the icon scaled to `width`×`height` in the context's current
    /// source colour.
    pub fn draw(&self, cr: &Context, width: f64, height: f64) {
        let (x, y, w, h) = self.view_box;
        cr.save().ok();
        cr.scale(width / w, height / h);
        cr.translate(-x, -y);
        for path in &self.paths {
            cr.new_path();
            for segment in &path.segments {
                match *segment {
                    Segment::Move(x, y) => cr.move_to(x, y),
                    Segment::Line(x, y) => cr.line_to(x, y),
                    Segment::Curve(x1, y1, x2, y2, x, y) => cr.curve_to(x1, y1, x2, y2, x, y),
                    Segment::Arc {
                        cx,
                        cy,
                        rx,
                        ry,
                        rotation,
                        from,
                        to,
                    } => {
                        let m = cr.matrix();
                        cr.translate(cx, cy);
                        cr.rotate(rotation);
                        cr.scale(rx, ry);
                        if to >= from {
                            cr.arc(0.0, 0.0, 1.0, from, to);
                        } else {
                            cr.arc_negative(0.0, 0.0, 1.0, from, to);
                        }
                        cr.set_matrix(m);
                    }
                    Segment::Close => cr.close_path(),
                }
            }
            if path.fill {
                if path.stroke.is_some() {
                    cr.fill_preserve().ok();
                } else {
                    cr.fill().ok();
                }
            }
            if let Some(width) = path.stroke {
                cr.set_line_width(width);
                cr.stroke().ok();
            }
        }
        cr.restore().ok();
    }
}

/// The opening tag `<name ...>`, without the brackets.
fn tag<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let start = source.find(&format!("<{name}"))?;
    let end = source[start..].find('>')? + start;
    Some(&source[start..end])
}

fn attr<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = element;
    loop {
        let at = rest.find(name)?;
        let before = rest[..at].chars().next_back();
        let after = &rest[at + name.len()..];
        if before.is_some_and(char::is_whitespace) && after.starts_with("=\"") {
            let value = &after[2..];
            return value.find('"').map(|end| &value[..end]);
        }
        rest = &rest[at + name.len()..];
    }
}

fn number_attr(element: &str, name: &str) -> Result<Option<f64>, String> {
    attr(element, name)
        .map(|v| {
            v.trim_end_matches("px")
                .parse()
                .map_err(|_| format!("bad {name} {v:?}"))
        })
        .transpose()
}

fn numbers(s: &str) -> Result<Vec<f64>, String> {
    let mut tokens = Tokens::new(s);
    let mut out = Vec::new();
    while tokens.more_numbers() {
        out.push(tokens.number()?);
    }
    Ok(out)
}

/// Path data reader: commands and numbers, where `-` and a second `.` can
/// start a new number without a separator (`2-2`, `.5.5`).
struct Tokens<'a> {
    s: &'a [u8],
    at: usize,
}

impl<'a> Tokens<'a> {
    fn new(s: &'a str) -> Self {
        Tokens {
            s: s.as_bytes(),
            at: 0,
        }
    }

    fn skip_separators(&mut self) {
        while self.at < self.s.len()
            && (self.s[self.at].is_ascii_whitespace() || self.s[self.at] == b',')
        {
            self.at += 1;
        }
    }

    fn more_numbers(&mut self) -> bool {
        self.skip_separators();
        self.s
            .get(self.at)
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
    }

    fn command(&mut self) -> Option<u8> {
        self.skip_separators();
        let c = *self.s.get(self.at)?;
        self.at += 1;
        Some(c)
    }

    fn number(&mut self) -> Result<f64, String> {
        self.skip_separators();
        let start = self.at;
        if matches!(self.s.get(self.at), Some(b'-' | b'+')) {
            self.at += 1;
        }
        let mut dot = false;
        while let Some(&c) = self.s.get(self.at) {
            if c.is_ascii_digit() {
                self.at += 1;
            } else if c == b'.' && !dot {
                dot = true;
                self.at += 1;
            } else if (c == b'e' || c == b'E') && self.at > start {
                self.at += 1;
                if matches!(self.s.get(self.at), Some(b'-' | b'+')) {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.at]).unwrap_or("");
        text.parse().map_err(|_| format!("bad number at {start}"))
    }

    /// Arc flags are single digits that may run together (`0 01`).
    fn flag(&mut self) -> Result<bool, String> {
        self.skip_separators();
        match self.s.get(self.at) {
            Some(b'0') => {
                self.at += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.at += 1;
                Ok(true)
            }
            _ => Err(format!("bad arc flag at {}", self.at)),
        }
    }
}

fn parse_path(d: &str) -> Result<Vec<Segment>, String> {
    let mut t = Tokens::new(d);
    let mut out = Vec::new();
    let (mut x, mut y) = (0.0, 0.0);
    let (mut start_x, mut start_y) = (0.0, 0.0);
    let mut command = None;
    loop {
        // A command letter may be left out when it repeats.
        let c = if t.more_numbers() {
            match command {
                Some(b'M') => b'L',
                Some(b'm') => b'l',
                Some(c) => c,
                None => return Err("path data starts with a number".into()),
            }
        } else {
            match t.command() {
                Some(c) => c,
                None => break,
            }
        };
        command = Some(c);
        let relative = c.is_ascii_lowercase();
        let (ox, oy) = if relative { (x, y) } else { (0.0, 0.0) };
        match c.to_ascii_uppercase() {
            b'M' => {
                x = ox + t.number()?;
                y = oy + t.number()?;
                (start_x, start_y) = (x, y);
                out.push(Segment::Move(x, y));
            }
            b'L' => {
                x = ox + t.number()?;
                y = oy + t.number()?;
                out.push(Segment::Line(x, y));
            }
            b'H' => {
                x = ox + t.number()?;
                out.push(Segment::Line(x, y));
            }
            b'V' => {
                y = oy + t.number()?;
                out.push(Segment::Line(x, y));
            }
            b'C' => {
                let mut p = [0.0; 6];
                for (i, v) in p.iter_mut().enumerate() {
                    *v = if i % 2 == 0 { ox } else { oy } + t.number()?;
                }
                (x, y) = (p[4], p[5]);
                out.push(Segment::Curve(p[0], p[1], p[2], p[3], p[4], p[5]));
            }
            b'A' => {
                let (rx, ry, angle) = (t.number()?, t.number()?, t.number()?);
                let (large, sweep) = (t.flag()?, t.flag()?);
                let (x2, y2) = (ox + t.number()?, oy + t.number()?);
                out.push(arc(x, y, rx, ry, angle, large, sweep, x2, y2));
                (x, y) = (x2, y2);
            }
            b'Z' => {
                out.push(Segment::Close);
                (x, y) = (start_x, start_y);
            }
            other => return Err(format!("unsupported path command {:?}", other as char)),
        }
    }
    Ok(out)
}

/// SVG's endpoint arc to a centre arc (SVG 1.1, appendix F.6.5).
#[allow(clippy::too_many_arguments)]
fn arc(
    x1: f64,
    y1: f64,
    rx: f64,
    ry: f64,
    angle_deg: f64,
    large: bool,
    sweep: bool,
    x2: f64,
    y2: f64,
) -> Segment {
    if rx == 0.0 || ry == 0.0 || (x1 == x2 && y1 == y2) {
        return Segment::Line(x2, y2);
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    let phi = angle_deg.to_radians();
    let (sin, cos) = phi.sin_cos();
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let x1p = cos * dx + sin * dy;
    let y1p = -sin * dx + cos * dy;
    // Radii too small for the endpoints grow until they fit.
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut k = (num / den).max(0.0).sqrt();
    if large == sweep {
        k = -k;
    }
    let cxp = k * rx * y1p / ry;
    let cyp = -k * ry * x1p / rx;
    let cx = cos * cxp - sin * cyp + (x1 + x2) / 2.0;
    let cy = sin * cxp + cos * cyp + (y1 + y2) / 2.0;
    let from = f64::atan2((y1p - cyp) / ry, (x1p - cxp) / rx);
    let mut delta = f64::atan2((-y1p - cyp) / ry, (-x1p - cxp) / rx) - from;
    if sweep && delta < 0.0 {
        delta += TAU;
    } else if !sweep && delta > 0.0 {
        delta -= TAU;
    }
    debug_assert!(delta.abs() <= TAU + PI * f64::EPSILON);
    Segment::Arc {
        cx,
        cy,
        rx,
        ry,
        rotation: phi,
        from,
        to: from + delta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn reads_attributes_and_commands() {
        let svg = Svg::parse(
            r#"<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
                 <path d="M2 4h3l6 8h3M2 12V10z"/>
                 <path fill="currentColor" stroke="none" d="m1 1 2 2"/>
               </svg>"#,
        )
        .unwrap();
        assert_eq!(svg.view_box, (0.0, 0.0, 16.0, 16.0));
        assert_eq!(
            svg.paths[0],
            Path {
                segments: vec![
                    Segment::Move(2.0, 4.0),
                    Segment::Line(5.0, 4.0),
                    Segment::Line(11.0, 12.0),
                    Segment::Line(14.0, 12.0),
                    Segment::Move(2.0, 12.0),
                    Segment::Line(2.0, 10.0),
                    Segment::Close,
                ],
                fill: false,
                stroke: Some(1.5),
            }
        );
        // An implicit command after `m` is a relative line.
        assert_eq!(
            svg.paths[1],
            Path {
                segments: vec![Segment::Move(1.0, 1.0), Segment::Line(3.0, 3.0)],
                fill: true,
                stroke: None,
            }
        );
    }

    #[test]
    fn numbers_without_separators() {
        let segments = parse_path("M12 2l2 2-2 2M.5.5").unwrap();
        assert_eq!(
            segments,
            [
                Segment::Move(12.0, 2.0),
                Segment::Line(14.0, 4.0),
                Segment::Line(12.0, 6.0),
                Segment::Move(0.5, 0.5),
            ]
        );
    }

    #[test]
    fn quarter_arc_finds_its_centre() {
        // From (3,6) clockwise to (5,4) on a circle of radius 2: centre (5,6).
        let Segment::Arc {
            cx,
            cy,
            rx,
            from,
            to,
            ..
        } = arc(3.0, 6.0, 2.0, 2.0, 0.0, false, true, 5.0, 4.0)
        else {
            panic!()
        };
        assert!(close(cx, 5.0) && close(cy, 6.0) && close(rx, 2.0));
        assert!(close(from, PI) || close(from, -PI));
        assert!(close(to - from, PI / 2.0));
    }

    #[test]
    fn every_icon_parses() {
        for icon in crate::icons::Icon::ALL {
            Svg::parse(icon.source()).unwrap_or_else(|e| panic!("{icon:?}: {e}"));
        }
    }
}
