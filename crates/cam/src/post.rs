//! The post processor: toolpaths to RS274/NGC G-code.
//!
//! Written from the public NIST RS274/NGC interpreter specification, version 3 (Kramer,
//! Proctor and Messina, NISTIR 6556, 2000): G21 millimetres, G90 absolute distances, G17
//! the XY plane, G0 rapid, G1 line, G2/G3 arcs with the centre as I and J offsets from the
//! start (incremental, the default arc distance mode), F feed, S spindle speed, M3/M5
//! spindle on and off, `T n M6` tool change, G98 with G81 (drill) and G83 (peck drill)
//! canned cycles cancelled by G80, and M30 program end.
//!
//! Two dialects: LinuxCNC takes all of that. GRBL has no tool changer and no canned
//! cycles, so a tool change becomes a pause (M0) with a comment naming the tool, and
//! drill cycles are written out as the G0 and G1 moves the cycle would make.

use crate::toolpath::{Move, Toolpath};
use chamfera_geom::{Point2, Point3};

/// Which controller the G-code is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    LinuxCnc,
    Grbl,
}

impl Dialect {
    pub fn parse(s: &str) -> Option<Dialect> {
        match s {
            "linuxcnc" | "rs274ngc" | "generic" => Some(Dialect::LinuxCnc),
            "grbl" => Some(Dialect::Grbl),
            _ => None,
        }
    }
}

/// The work coordinate system: G-code coordinates are model points measured from
/// `origin`, in axes turned by `angle` (radians, about Z) from the model's.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Wcs {
    pub origin: Point3,
    pub angle: f64,
}

impl Wcs {
    /// A model point in work coordinates.
    pub fn to_work(self, p: Point3) -> Point3 {
        let (s, c) = self.angle.sin_cos();
        let (dx, dy) = (p.x - self.origin.x, p.y - self.origin.y);
        Point3::new(c * dx + s * dy, -s * dx + c * dy, p.z - self.origin.z)
    }

    fn to_work2(self, p: Point2) -> Point2 {
        let w = self.to_work(Point3::new(p.x, p.y, self.origin.z));
        Point2::new(w.x, w.y)
    }
}

/// A number for G-code: at most 3 decimals, trailing zeros and point dropped, no `-0`.
pub fn number(v: f64) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    if r == 0.0 || !r.is_finite() {
        return "0".into();
    }
    let s = format!("{r:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_owned()
}

/// Comment text without parentheses or line breaks (which would end a comment).
fn comment(s: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| !matches!(c, '(' | ')' | '\n' | '\r' | ';'))
        .take(120)
        .collect();
    format!("({})", clean.trim())
}

/// The modal state the controller is in, so unchanged words are not repeated.
struct State {
    pos: [Option<String>; 3],
    feed: Option<String>,
}

impl State {
    /// The word for axis `i` (0 X, 1 Y, 2 Z) at `v` if it changes (or `force`), updating
    /// the state.
    fn word(&mut self, i: usize, v: f64, force: bool) -> String {
        let letter = ["X", "Y", "Z"].get(i).copied().unwrap_or("Z");
        let n = number(v);
        match self.pos.get_mut(i) {
            Some(slot) if force || slot.as_deref() != Some(n.as_str()) => {
                let w = format!(" {letter}{n}");
                *slot = Some(n);
                w
            }
            _ => String::new(),
        }
    }

    /// The X, Y and Z words that change.
    fn axes(&mut self, p: Point3) -> String {
        let mut out = self.word(0, p.x, false);
        out.push_str(&self.word(1, p.y, false));
        out.push_str(&self.word(2, p.z, false));
        out
    }

    fn get(&self, i: usize) -> Option<f64> {
        self.pos
            .get(i)
            .and_then(|v| v.as_ref())
            .and_then(|v| v.parse().ok())
    }

    fn feed(&mut self, f: f64) -> String {
        let n = number(f);
        if self.feed.as_deref() == Some(n.as_str()) {
            String::new()
        } else {
            self.feed = Some(n.clone());
            format!(" F{n}")
        }
    }

    fn z(&self) -> Option<f64> {
        self.get(2)
    }
}

/// G-code for `ops` (each an operation name and its toolpath), in order.
pub fn post(program: &str, ops: &[(String, Toolpath)], wcs: &Wcs, dialect: Dialect) -> String {
    let mut out: Vec<String> = vec![comment(program), "G21 G90 G17".into()];
    let mut st = State {
        pos: [None, None, None],
        feed: None,
    };
    let mut tool: Option<u32> = None;
    for (name, tp) in ops {
        out.push(comment(name));
        if tool != Some(tp.tool_number) {
            let label = format!("T{} {}", tp.tool_number, tp.tool_name);
            match dialect {
                Dialect::LinuxCnc => {
                    if tool.is_some() {
                        out.push("M5".into());
                    }
                    out.push(format!("T{} M6", tp.tool_number));
                }
                Dialect::Grbl => {
                    if tool.is_some() {
                        out.push("M5".into());
                        out.push(format!("M0 {}", comment(&format!("change to {label}"))));
                    } else {
                        out.push(comment(&label));
                    }
                }
            }
            tool = Some(tp.tool_number);
        }
        out.push(format!("S{} M3", number(tp.spindle_rpm)));
        let moves = match dialect {
            Dialect::LinuxCnc => tp.moves.clone(),
            Dialect::Grbl => tp.expanded(),
        };
        let mut in_cycle = false;
        for m in moves {
            if in_cycle && !matches!(m, Move::Drill { .. }) {
                out.push("G80".into());
                in_cycle = false;
            }
            match m {
                Move::Rapid { to } => {
                    let w = wcs.to_work(to);
                    // Lift before moving across; move across before going down.
                    if st.z().is_none_or(|z| w.z > z) {
                        let z = st.word(2, w.z, false);
                        if !z.is_empty() {
                            out.push(format!("G0{z}"));
                        }
                    }
                    let mut xy = st.word(0, w.x, false);
                    xy.push_str(&st.word(1, w.y, false));
                    if !xy.is_empty() {
                        out.push(format!("G0{xy}"));
                    }
                    let z = st.word(2, w.z, false);
                    if !z.is_empty() {
                        out.push(format!("G0{z}"));
                    }
                }
                Move::Line { to, feed } | Move::Plunge { to, feed } => {
                    let a = st.axes(wcs.to_work(to));
                    if !a.is_empty() {
                        let f = st.feed(feed);
                        out.push(format!("G1{a}{f}"));
                    }
                }
                Move::Arc {
                    to,
                    centre,
                    clockwise,
                    feed,
                } => {
                    let start = Point2::new(st.get(0).unwrap_or(0.0), st.get(1).unwrap_or(0.0));
                    let c = wcs.to_work2(centre);
                    let w = wcs.to_work(to);
                    let mut words = st.word(0, w.x, true);
                    words.push_str(&st.word(1, w.y, true));
                    words.push_str(&st.word(2, w.z, false));
                    let f = st.feed(feed);
                    let g = if clockwise { "G2" } else { "G3" };
                    out.push(format!(
                        "{g}{words} I{} J{}{f}",
                        number(c.x - start.x),
                        number(c.y - start.y)
                    ));
                }
                Move::Drill {
                    at,
                    r,
                    bottom,
                    peck,
                    feed,
                } => {
                    let w = wcs.to_work(Point3::new(at.x, at.y, bottom));
                    let rz = r - wcs.origin.z;
                    let f = st.feed(feed);
                    let line = match peck {
                        Some(q) => format!(
                            "G98 G83 X{} Y{} Z{} R{} Q{}{f}",
                            number(w.x),
                            number(w.y),
                            number(w.z),
                            number(rz),
                            number(q)
                        ),
                        None => format!(
                            "G98 G81 X{} Y{} Z{} R{}{f}",
                            number(w.x),
                            number(w.y),
                            number(w.z),
                            number(rz)
                        ),
                    };
                    out.push(line);
                    // After a G98 cycle the tool is back at the height it started from.
                    if let Some(slot) = st.pos.get_mut(0) {
                        *slot = Some(number(w.x));
                    }
                    if let Some(slot) = st.pos.get_mut(1) {
                        *slot = Some(number(w.y));
                    }
                    in_cycle = true;
                }
            }
        }
        if in_cycle {
            out.push("G80".into());
        }
    }
    out.push("M5".into());
    out.push("M30".into());
    let mut text = out.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_drop_trailing_zeros_and_negative_zero() {
        assert_eq!(number(10.0), "10");
        assert_eq!(number(-3.5), "-3.5");
        assert_eq!(number(0.12345), "0.123");
        assert_eq!(number(-0.0001), "0");
        assert_eq!(number(f64::NAN), "0");
        assert_eq!(number(2.0005), "2.001");
    }

    #[test]
    fn work_coordinates_turn_about_the_origin() {
        // Origin (10, 0, 5), axes turned 90°: the model point (10, 2, 5) lies along the
        // work X axis, 2 from the origin.
        let w = Wcs {
            origin: Point3::new(10.0, 0.0, 5.0),
            angle: std::f64::consts::FRAC_PI_2,
        };
        let p = w.to_work(Point3::new(10.0, 2.0, 5.0));
        assert!((p.x - 2.0).abs() < 1e-12 && p.y.abs() < 1e-12 && p.z.abs() < 1e-12);
    }

    #[test]
    fn comments_cannot_be_closed_early() {
        assert_eq!(comment("a (b) c\nd"), "(a b cd)");
    }
}
