//! SVG import for sketches, from the public W3C specification (SVG 1.1 Second Edition and
//! SVG 2: "Paths", "Basic Shapes", "Coordinate Systems, Transformations and Units", and
//! the implementation notes on elliptical arcs, F.6 of SVG 1.1).
//!
//! Reading keeps the geometry of the drawing and nothing else (no strokes, fills, text
//! or images): `path` (every command: M L H V C S Q T A Z, absolute and relative),
//! `line`, `polyline`, `polygon`, `rect` (with rounded corners), `circle` and `ellipse`,
//! inside `svg`, `g`, `a` and `switch` groups, each under its `transform` (matrix,
//! translate, scale, rotate, skewX, skewY). `defs`, `symbol`, `clipPath`, `mask`,
//! `pattern`, `marker` and the like are not drawn, so they are left out; `text`,
//! `image` and `use` are counted in `skipped`.
//!
//! Units: the root `svg` maps its `viewBox` onto its `width` and `height`
//! (`preserveAspectRatio`, default `xMidYMid meet`), and CSS units give millimetres
//! (96 px to the inch). Without them a user unit is a CSS pixel. A caller may instead
//! say what one user unit is ([`SvgUnits`]). SVG's y axis points down; the sketch's
//! points up, so the drawing is turned over about the bottom of its page, which lands on
//! the sketch's x axis with the page's left edge on its y axis.
//!
//! Curves stay exact: lines are lines; a cubic Bézier is a cubic B-spline with the four
//! control points and knots `0 0 0 0 1 1 1 1` (a clamped curve with one span is its
//! Bézier form, The NURBS Book §2.3); a quadratic is raised to the cubic with control
//! points `P0, P0 + 2/3 (P1 - P0), P2 + 2/3 (P1 - P2), P2` (degree elevation, §5.5),
//! which is the same curve. An affine map takes a Bézier to the Bézier of the mapped
//! control points, and an ellipse to an ellipse (circular arcs stay circular under
//! similarities), so transforms keep every curve exact.
//!
//! Files are hostile input: sizes, nesting and counts are capped and every number is
//! checked. Bad path data ends that path where it went wrong, as the specification asks.

use crate::IoError;
use crate::xml::{Tag, tags};
use chamfera_geom::{Point2, Vec2};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

/// Largest SVG file accepted (bytes).
pub const MAX_SVG_BYTES: u64 = 64 * 1024 * 1024;
/// Most shapes read from one file.
pub const MAX_SHAPES: usize = 10_000;
/// Deepest element nesting followed.
pub const MAX_DEPTH: usize = 256;
/// Millimetres per CSS pixel (96 px to the inch of 25.4 mm).
pub const MM_PER_PX: f64 = 25.4 / 96.0;

fn bad(why: impl std::fmt::Display) -> IoError {
    IoError::Format(format!("SVG: {why}"))
}

/// What one SVG user unit is, when the caller knows better than the file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SvgUnits {
    /// From the file's `width`, `height` and `viewBox` (CSS pixels when it says nothing).
    File,
    /// One user unit is this many millimetres.
    Mm(f64),
}

/// One curve of the drawing, in sketch millimetres (y up). Arcs and elliptical arcs run
/// counter-clockwise from `start` to `end` (angles in radians; for an ellipse, the
/// parameter `t` of `centre + cos t · major + sin t · minor`, where `minor` is `major`
/// turned a quarter counter-clockwise and scaled by `ratio`).
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Line {
        a: Point2,
        b: Point2,
    },
    Circle {
        centre: Point2,
        radius: f64,
    },
    Arc {
        centre: Point2,
        radius: f64,
        start: f64,
        end: f64,
    },
    /// A whole ellipse when `end - start` is 2π.
    Ellipse {
        centre: Point2,
        major: Vec2,
        ratio: f64,
        start: f64,
        end: f64,
    },
    /// A cubic Bézier by its four control points.
    Cubic([Point2; 4]),
}

/// What an SVG file holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawing {
    pub shapes: Vec<Shape>,
    /// Elements left out, by name (and why).
    pub skipped: BTreeMap<String, usize>,
    /// The page's width and height (mm), when the file gives them.
    pub page: Option<(f64, f64)>,
    /// Millimetres per user unit at the root (the square root of the scale's
    /// determinant, so a non-uniform `preserveAspectRatio="none"` gives the mean).
    pub mm_per_unit: f64,
}

/// An affine map `x' = a x + c y + e`, `y' = b x + d y + f` (SVG's `matrix(a b c d e f)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn scale(sx: f64, sy: f64) -> Affine {
        Affine {
            a: sx,
            d: sy,
            ..Affine::IDENTITY
        }
    }

    fn translate(tx: f64, ty: f64) -> Affine {
        Affine {
            e: tx,
            f: ty,
            ..Affine::IDENTITY
        }
    }

    /// `self · o`: `o` applied first.
    pub fn then_after(&self, o: &Affine) -> Affine {
        Affine {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }

    pub fn apply(&self, p: Point2) -> Point2 {
        Point2::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }

    pub fn apply_vec(&self, v: Vec2) -> Vec2 {
        Vec2::new(self.a * v.x + self.c * v.y, self.b * v.x + self.d * v.y)
    }

    pub fn det(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    fn is_finite(&self) -> bool {
        [self.a, self.b, self.c, self.d, self.e, self.f]
            .iter()
            .all(|v| v.is_finite())
    }
}

// ----- numbers, lengths and transforms -----

/// A number scanner over SVG's number grammar (`-1.5e3`, `.5`, `1.5.5` is two numbers).
struct Scan<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Scan<'a> {
    fn new(s: &'a str) -> Scan<'a> {
        Scan {
            s: s.as_bytes(),
            i: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }

    /// Skips white space and at most one comma.
    fn skip_sep(&mut self) {
        self.skip_space();
        if self.peek() == Some(b',') {
            self.i += 1;
            self.skip_space();
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip_space();
        self.i >= self.s.len()
    }

    fn digits(&mut self) -> usize {
        let from = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.i - from
    }

    /// The next number, after separators; `None` (nothing consumed) if there is none.
    fn number(&mut self) -> Option<f64> {
        self.skip_sep();
        let start = self.i;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.i += 1;
        }
        let mut n = self.digits();
        if self.peek() == Some(b'.') {
            self.i += 1;
            n += self.digits();
        }
        if n == 0 {
            self.i = start;
            return None;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mark = self.i;
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                self.i = mark;
            }
        }
        let text = std::str::from_utf8(self.s.get(start..self.i)?).ok()?;
        let v: f64 = text.parse().ok()?;
        if v.is_finite() {
            Some(v)
        } else {
            self.i = start;
            None
        }
    }

    /// An arc flag: `0` or `1`, which may run into the next number (`a1 1 0 0110 10`).
    fn flag(&mut self) -> Option<bool> {
        self.skip_sep();
        match self.peek() {
            Some(b'0') => {
                self.i += 1;
                Some(false)
            }
            Some(b'1') => {
                self.i += 1;
                Some(true)
            }
            _ => None,
        }
    }

    /// A number with an optional unit, as text after it.
    fn unit(&mut self) -> &'a str {
        let from = self.i;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'%')
        {
            self.i += 1;
        }
        std::str::from_utf8(self.s.get(from..self.i).unwrap_or(&[])).unwrap_or("")
    }
}

/// A length attribute in CSS pixels: a number with an absolute unit (px, mm, cm, in, pt,
/// pc, Q) or none. Percentages and font-relative units give `None`.
pub fn length_px(text: &str) -> Option<f64> {
    let mut s = Scan::new(text.trim());
    let v = s.number()?;
    let px = match s.unit() {
        "" | "px" => 1.0,
        "mm" => 96.0 / 25.4,
        "cm" => 960.0 / 25.4,
        "in" => 96.0,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "Q" => 24.0 / 25.4,
        _ => return None,
    };
    if !s.at_end() {
        return None;
    }
    Some(v * px).filter(|x| x.is_finite())
}

/// A plain coordinate attribute (user units); a unit other than px is not interpreted.
fn coord(tag: &Tag, key: &str) -> f64 {
    tag.attribute(key)
        .and_then(|t| {
            let mut s = Scan::new(t.trim());
            let v = s.number()?;
            matches!(s.unit(), "" | "px").then_some(v)
        })
        .unwrap_or(0.0)
}

fn numbers(text: &str, limit: usize) -> Vec<f64> {
    let mut s = Scan::new(text);
    let mut out = Vec::new();
    while out.len() < limit {
        match s.number() {
            Some(v) => out.push(v),
            None => break,
        }
    }
    out
}

/// A `transform` attribute; `None` when it cannot be read (the element is then left out,
/// as the specification says of an invalid transform).
pub fn parse_transform(text: &str) -> Option<Affine> {
    let mut m = Affine::IDENTITY;
    let mut rest = text.trim();
    let mut count = 0;
    while !rest.is_empty() {
        count += 1;
        if count > 64 {
            return None;
        }
        let open = rest.find('(')?;
        let close = rest.find(')')?;
        if close < open {
            return None;
        }
        let name = rest.get(..open)?.trim().trim_start_matches(',').trim();
        let args = numbers(rest.get(open + 1..close)?, 7);
        let t = match (name, args.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine {
                a: *a,
                b: *b,
                c: *c,
                d: *d,
                e: *e,
                f: *f,
            },
            ("translate", [tx]) => Affine::translate(*tx, 0.0),
            ("translate", [tx, ty]) => Affine::translate(*tx, *ty),
            ("scale", [s]) => Affine::scale(*s, *s),
            ("scale", [sx, sy]) => Affine::scale(*sx, *sy),
            ("rotate", [deg]) => rotation(*deg),
            ("rotate", [deg, cx, cy]) => Affine::translate(*cx, *cy)
                .then_after(&rotation(*deg))
                .then_after(&Affine::translate(-cx, -cy)),
            ("skewX", [deg]) => Affine {
                c: deg.to_radians().tan(),
                ..Affine::IDENTITY
            },
            ("skewY", [deg]) => Affine {
                b: deg.to_radians().tan(),
                ..Affine::IDENTITY
            },
            _ => return None,
        };
        m = m.then_after(&t);
        rest = rest.get(close + 1..)?.trim().trim_start_matches(',').trim();
    }
    m.is_finite().then_some(m)
}

fn rotation(deg: f64) -> Affine {
    let (s, c) = deg.to_radians().sin_cos();
    Affine {
        a: c,
        b: s,
        c: -s,
        d: c,
        e: 0.0,
        f: 0.0,
    }
}

// ----- path data -----

/// A segment of a path in user units, before any transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    Line(Point2, Point2),
    Cubic([Point2; 4]),
    /// An elliptical arc from `from` to `to` with radii `rx`, `ry`, x axis turned by `phi`
    /// (radians) and the large-arc and sweep flags.
    Arc {
        from: Point2,
        to: Point2,
        rx: f64,
        ry: f64,
        phi: f64,
        large: bool,
        sweep: bool,
    },
}

/// The segments of path data `d`, and whether it was read to the end (bad data stops it,
/// keeping what came before). At most `limit` segments.
pub fn parse_path(d: &str, limit: usize) -> (Vec<Segment>, bool) {
    let mut s = Scan::new(d);
    let mut out = Vec::new();
    let mut cmd: Option<u8> = None;
    let mut cur = Point2::new(0.0, 0.0);
    let mut start = cur;
    // The second control point of the last cubic (S) or the control of the last
    // quadratic (T), for reflection.
    let mut last_cubic: Option<Point2> = None;
    let mut last_quad: Option<Point2> = None;
    let mut ok = true;
    loop {
        if out.len() >= limit {
            ok = false;
            break;
        }
        s.skip_sep();
        if s.at_end() {
            break;
        }
        let Some(c) = s.peek() else { break };
        if c.is_ascii_alphabetic() {
            s.i += 1;
            if matches!(c, b'Z' | b'z') {
                if !cur.coincident(start) {
                    out.push(Segment::Line(cur, start));
                }
                cur = start;
                cmd = None;
                last_cubic = None;
                last_quad = None;
                continue;
            }
            cmd = Some(c);
        }
        let Some(k) = cmd else {
            ok = false;
            break;
        };
        let rel = k.is_ascii_lowercase();
        let base = if rel {
            Vec2::new(cur.x, cur.y)
        } else {
            Vec2::ZERO
        };
        let pair = |s: &mut Scan| -> Option<Point2> {
            let x = s.number()?;
            let y = s.number()?;
            Some(Point2::new(x, y) + base)
        };
        let step: Option<()> = (|| {
            match k.to_ascii_uppercase() {
                b'M' => {
                    let p = pair(&mut s)?;
                    cur = p;
                    start = p;
                    // Further pairs are line-tos.
                    cmd = Some(if rel { b'l' } else { b'L' });
                    last_cubic = None;
                    last_quad = None;
                }
                b'L' => {
                    let p = pair(&mut s)?;
                    out.push(Segment::Line(cur, p));
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                b'H' => {
                    let x = s.number()? + base.x;
                    let p = Point2::new(x, cur.y);
                    out.push(Segment::Line(cur, p));
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                b'V' => {
                    let y = s.number()? + base.y;
                    let p = Point2::new(cur.x, y);
                    out.push(Segment::Line(cur, p));
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                b'C' => {
                    let (p1, p2, p3) = (pair(&mut s)?, pair(&mut s)?, pair(&mut s)?);
                    out.push(Segment::Cubic([cur, p1, p2, p3]));
                    last_cubic = Some(p2);
                    last_quad = None;
                    cur = p3;
                }
                b'S' => {
                    let p1 = match last_cubic {
                        Some(c2) => cur + (cur - c2),
                        None => cur,
                    };
                    let (p2, p3) = (pair(&mut s)?, pair(&mut s)?);
                    out.push(Segment::Cubic([cur, p1, p2, p3]));
                    last_cubic = Some(p2);
                    last_quad = None;
                    cur = p3;
                }
                b'Q' => {
                    let (q, p) = (pair(&mut s)?, pair(&mut s)?);
                    out.push(Segment::Cubic(elevate(cur, q, p)));
                    last_quad = Some(q);
                    last_cubic = None;
                    cur = p;
                }
                b'T' => {
                    let q = match last_quad {
                        Some(q0) => cur + (cur - q0),
                        None => cur,
                    };
                    let p = pair(&mut s)?;
                    out.push(Segment::Cubic(elevate(cur, q, p)));
                    last_quad = Some(q);
                    last_cubic = None;
                    cur = p;
                }
                b'A' => {
                    let rx = s.number()?;
                    let ry = s.number()?;
                    let deg = s.number()?;
                    let large = s.flag()?;
                    let sweep = s.flag()?;
                    let p = pair(&mut s)?;
                    if !cur.coincident(p) {
                        out.push(Segment::Arc {
                            from: cur,
                            to: p,
                            rx,
                            ry,
                            phi: deg.to_radians(),
                            large,
                            sweep,
                        });
                    }
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                _ => return None,
            }
            Some(())
        })();
        if step.is_none() {
            ok = false;
            break;
        }
    }
    (out, ok)
}

/// The cubic with the same curve as the quadratic `p0 q p2` (degree elevation).
fn elevate(p0: Point2, q: Point2, p2: Point2) -> [Point2; 4] {
    [
        p0,
        p0 + (q - p0) * (2.0 / 3.0),
        p2 + (q - p2) * (2.0 / 3.0),
        p2,
    ]
}

/// An arc segment as centre, radii and angles (SVG 1.1 F.6.5, with the radii scaled up
/// when too small, F.6.6): the ellipse `c + R(phi) (rx cos θ, ry sin θ)` from `theta1`
/// through `dtheta` (positive counter-clockwise in user space, where y points down, so
/// "positive-angle direction" of the specification). `None` when a radius is zero (the
/// specification then draws a line).
pub fn arc_centre(
    from: Point2,
    to: Point2,
    rx: f64,
    ry: f64,
    phi: f64,
    large: bool,
    sweep: bool,
) -> Option<(Point2, f64, f64, f64, f64)> {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if !(rx > 0.0 && ry > 0.0 && rx.is_finite() && ry.is_finite()) {
        return None;
    }
    let (sp, cp) = phi.sin_cos();
    let hx = (from.x - to.x) / 2.0;
    let hy = (from.y - to.y) / 2.0;
    let x1 = cp * hx + sp * hy;
    let y1 = -sp * hx + cp * hy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let k = lambda.sqrt();
        rx *= k;
        ry *= k;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    if den <= 0.0 {
        return None;
    }
    let mut co = (num.max(0.0) / den).sqrt();
    if large == sweep {
        co = -co;
    }
    let cxp = co * rx * y1 / ry;
    let cyp = -co * ry * x1 / rx;
    let mx = (from.x + to.x) / 2.0;
    let my = (from.y + to.y) / 2.0;
    let c = Point2::new(cp * cxp - sp * cyp + mx, sp * cxp + cp * cyp + my);
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let ux = (x1 - cxp) / rx;
    let uy = (y1 - cyp) / ry;
    let vx = (-x1 - cxp) / rx;
    let vy = (-y1 - cyp) / ry;
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut dtheta = angle(ux, uy, vx, vy).rem_euclid(TAU);
    if !sweep && dtheta > 0.0 {
        dtheta -= TAU;
    }
    let all = [c.x, c.y, rx, ry, theta1, dtheta];
    all.iter()
        .all(|v| v.is_finite())
        .then_some((c, rx, ry, theta1, dtheta))
}

// ----- conics under an affine map -----

/// Relative difference of the semi-axes below which an ellipse is taken as a circle.
const ROUND: f64 = 1e-9;

/// The curve `centre + A (cos θ, sin θ)` for θ from `theta1` through `dtheta` (the whole
/// curve when `full`), with `A`'s columns `a1`, `a2` (conjugate semi-diameters), as a
/// circle, arc, ellipse or elliptical arc running counter-clockwise. `None` if flat.
fn conic(
    centre: Point2,
    a1: Vec2,
    a2: Vec2,
    theta1: f64,
    dtheta: f64,
    full: bool,
) -> Option<Shape> {
    let det = a1.cross(a2);
    let size = a1.length().max(a2.length());
    if !(size.is_finite() && size > 0.0) || det.abs() <= 1e-12 * size * size {
        return None;
    }
    // The parameter of a principal axis: |P(θ) - c|² is extreme at
    // 2θ0 = atan2(2 a1·a2, |a1|² - |a2|²) (largest there).
    let t0 = 0.5 * (2.0 * a1.dot(a2)).atan2(a1.dot(a1) - a2.dot(a2));
    let at = |t: f64| a1 * t.cos() + a2 * t.sin();
    let major = at(t0);
    let minor = at(t0 + PI / 2.0);
    let (ma, mi) = (major.length(), minor.length());
    // Counter-clockwise in the result: with det < 0 the map mirrors, so θ runs the other
    // way round.
    let sign = if det > 0.0 { 1.0 } else { -1.0 };
    if (ma - mi).abs() <= ROUND * ma {
        let r = 0.5 * (ma + mi);
        if full {
            return Some(Shape::Circle { centre, radius: r });
        }
        let a = at(theta1).angle();
        let b = at(theta1 + dtheta).angle();
        let (start, end) = if sign * dtheta > 0.0 { (a, b) } else { (b, a) };
        return Some(Shape::Arc {
            centre,
            radius: r,
            start,
            end,
        });
    }
    let ratio = mi / ma;
    if full {
        return Some(Shape::Ellipse {
            centre,
            major,
            ratio,
            start: 0.0,
            end: TAU,
        });
    }
    // In the result's parameter t = sign (θ - θ0), the curve is
    // c + cos t major + sin t (major turned a quarter, × ratio).
    let t1 = sign * (theta1 - t0);
    let dt = sign * dtheta;
    let (start, end) = if dt > 0.0 {
        (t1, t1 + dt)
    } else {
        (t1 + dt, t1)
    };
    Some(Shape::Ellipse {
        centre,
        major,
        ratio,
        start,
        end,
    })
}

// ----- the reader -----

/// Elements whose contents are never drawn directly.
const HIDDEN: &[&str] = &[
    "defs",
    "symbol",
    "clipPath",
    "mask",
    "pattern",
    "marker",
    "linearGradient",
    "radialGradient",
    "filter",
    "style",
    "script",
    "title",
    "desc",
    "metadata",
    "foreignObject",
];
/// Elements with geometry we do not import, counted.
const UNSUPPORTED: &[&str] = &["text", "image", "use"];
/// Groups whose children are drawn.
const GROUPS: &[&str] = &["svg", "g", "a", "switch"];

struct Reader {
    out: Drawing,
    /// User units of the root to sketch millimetres (y up).
    root: Affine,
}

impl Reader {
    fn skip(&mut self, why: &str) {
        *self.out.skipped.entry(why.to_owned()).or_insert(0) += 1;
    }

    fn push(&mut self, s: Shape) -> Result<(), IoError> {
        if self.out.shapes.len() >= MAX_SHAPES {
            return Err(bad(format!("the file has more than {MAX_SHAPES} curves")));
        }
        self.out.shapes.push(s);
        Ok(())
    }

    /// Adds path segments drawn under `ctm` (user units of the element to the root's).
    fn segments(&mut self, segs: &[Segment], ctm: &Affine, what: &str) -> Result<(), IoError> {
        let m = self.root.then_after(ctm);
        for seg in segs {
            match *seg {
                Segment::Line(a, b) => {
                    let (a, b) = (m.apply(a), m.apply(b));
                    if !a.coincident(b) {
                        self.push(Shape::Line { a, b })?;
                    }
                }
                Segment::Cubic(p) => {
                    let q = p.map(|x| m.apply(x));
                    if q.iter().all(|x| x.coincident(q[0])) {
                        continue;
                    }
                    self.push(Shape::Cubic(q))?;
                }
                Segment::Arc {
                    from,
                    to,
                    rx,
                    ry,
                    phi,
                    large,
                    sweep,
                } => match arc_centre(from, to, rx, ry, phi, large, sweep) {
                    Some((c, rx, ry, theta1, dtheta)) => {
                        let (sp, cp) = phi.sin_cos();
                        let a1 = m.apply_vec(Vec2::new(cp * rx, sp * rx));
                        let a2 = m.apply_vec(Vec2::new(-sp * ry, cp * ry));
                        match conic(m.apply(c), a1, a2, theta1, dtheta, false) {
                            Some(s) => self.push(s)?,
                            None => self.skip(&format!("{what} (flat arc)")),
                        }
                    }
                    None => {
                        // A zero radius: a straight line (F.6.2).
                        let (a, b) = (m.apply(from), m.apply(to));
                        if !a.coincident(b) {
                            self.push(Shape::Line { a, b })?;
                        }
                    }
                },
            }
        }
        Ok(())
    }

    fn path_data(&mut self, d: &str, ctm: &Affine, what: &str) -> Result<(), IoError> {
        let room = MAX_SHAPES.saturating_sub(self.out.shapes.len()) + 1;
        let (segs, ok) = parse_path(d, room);
        if !ok {
            self.skip(&format!("{what} (bad path data, the rest left out)"));
        }
        self.segments(&segs, ctm, what)
    }

    fn conic_element(
        &mut self,
        c: Point2,
        rx: f64,
        ry: f64,
        ctm: &Affine,
        what: &str,
    ) -> Result<(), IoError> {
        if !(rx > 0.0 && ry > 0.0 && rx.is_finite() && ry.is_finite()) {
            self.skip(&format!("{what} (zero radius)"));
            return Ok(());
        }
        let m = self.root.then_after(ctm);
        let a1 = m.apply_vec(Vec2::new(rx, 0.0));
        let a2 = m.apply_vec(Vec2::new(0.0, ry));
        match conic(m.apply(c), a1, a2, 0.0, TAU, true) {
            Some(s) => self.push(s),
            None => {
                self.skip(&format!("{what} (flat)"));
                Ok(())
            }
        }
    }

    fn element(&mut self, tag: &Tag, name: &str, ctm: &Affine) -> Result<(), IoError> {
        match name {
            "path" => {
                if let Some(d) = tag.attribute("d") {
                    self.path_data(d, ctm, "path")?;
                }
            }
            "line" => {
                let a = Point2::new(coord(tag, "x1"), coord(tag, "y1"));
                let b = Point2::new(coord(tag, "x2"), coord(tag, "y2"));
                self.segments(&[Segment::Line(a, b)], ctm, "line")?;
            }
            "polyline" | "polygon" => {
                let v = numbers(tag.attribute("points").unwrap_or(""), 2 * MAX_SHAPES + 2);
                let (pairs, _) = v.as_chunks::<2>();
                let pts: Vec<Point2> = pairs.iter().map(|[x, y]| Point2::new(*x, *y)).collect();
                let mut segs: Vec<Segment> =
                    pts.windows(2).map(|w| Segment::Line(w[0], w[1])).collect();
                if name == "polygon"
                    && let (Some(f), Some(l)) = (pts.first(), pts.last())
                    && pts.len() > 2
                    && !f.coincident(*l)
                {
                    segs.push(Segment::Line(*l, *f));
                }
                self.segments(&segs, ctm, name)?;
            }
            "rect" => {
                let (x, y) = (coord(tag, "x"), coord(tag, "y"));
                let (w, h) = (coord(tag, "width"), coord(tag, "height"));
                if !(w > 0.0 && h > 0.0) {
                    self.skip("rect (no size)");
                    return Ok(());
                }
                let rxa = tag.attribute("rx").map(|_| coord(tag, "rx").max(0.0));
                let rya = tag.attribute("ry").map(|_| coord(tag, "ry").max(0.0));
                let (rx, ry) = match (rxa, rya) {
                    (Some(a), Some(b)) => (a, b),
                    (Some(a), None) => (a, a),
                    (None, Some(b)) => (b, b),
                    (None, None) => (0.0, 0.0),
                };
                let (rx, ry) = (rx.min(w / 2.0), ry.min(h / 2.0));
                let p = |px: f64, py: f64| Point2::new(px, py);
                let mut segs = Vec::new();
                if rx > 0.0 && ry > 0.0 {
                    let arc = |from: Point2, to: Point2| Segment::Arc {
                        from,
                        to,
                        rx,
                        ry,
                        phi: 0.0,
                        large: false,
                        sweep: true,
                    };
                    let corners = [
                        (p(x + rx, y), p(x + w - rx, y), p(x + w, y + ry)),
                        (p(x + w, y + ry), p(x + w, y + h - ry), p(x + w - rx, y + h)),
                        (p(x + w - rx, y + h), p(x + rx, y + h), p(x, y + h - ry)),
                        (p(x, y + h - ry), p(x, y + ry), p(x + rx, y)),
                    ];
                    for (a, b, c) in corners {
                        if !a.coincident(b) {
                            segs.push(Segment::Line(a, b));
                        }
                        segs.push(arc(b, c));
                    }
                } else {
                    let c = [p(x, y), p(x + w, y), p(x + w, y + h), p(x, y + h)];
                    for i in 0..4 {
                        segs.push(Segment::Line(c[i], c[(i + 1) % 4]));
                    }
                }
                self.segments(&segs, ctm, "rect")?;
            }
            "circle" => {
                let c = Point2::new(coord(tag, "cx"), coord(tag, "cy"));
                let r = coord(tag, "r");
                self.conic_element(c, r, r, ctm, "circle")?;
            }
            "ellipse" => {
                let c = Point2::new(coord(tag, "cx"), coord(tag, "cy"));
                self.conic_element(c, coord(tag, "rx"), coord(tag, "ry"), ctm, "ellipse")?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// The root's map from user units to sketch millimetres (y up) and its page size (mm).
fn root_map(tag: &Tag, units: SvgUnits) -> (Affine, Option<(f64, f64)>) {
    let vb = tag.attribute("viewBox").map(|t| numbers(t, 5));
    let vb = match vb.as_deref() {
        Some([x, y, w, h]) if *w > 0.0 && *h > 0.0 => Some((*x, *y, *w, *h)),
        _ => None,
    };
    // User units to millimetres with y down, and the page (x0, y0, w, h) in those mm.
    let (m, page) = match units {
        SvgUnits::Mm(k) => {
            let page = vb.or_else(|| {
                let w = coord(tag, "width");
                let h = coord(tag, "height");
                (w > 0.0 && h > 0.0).then_some((0.0, 0.0, w, h))
            });
            (
                Affine::scale(k, k),
                page.map(|(x, y, w, h)| (x * k, y * k, w * k, h * k)),
            )
        }
        SvgUnits::File => {
            let w = tag
                .attribute("width")
                .and_then(length_px)
                .filter(|v| *v > 0.0);
            let h = tag
                .attribute("height")
                .and_then(length_px)
                .filter(|v| *v > 0.0);
            match vb {
                Some((vx, vy, vw, vh)) => {
                    let (w, h) = match (w, h) {
                        (Some(w), Some(h)) => (w, h),
                        (Some(w), None) => (w, w * vh / vw),
                        (None, Some(h)) => (h * vw / vh, h),
                        (None, None) => (vw, vh),
                    };
                    let (sx, sy) = (w / vw, h / vh);
                    let par = tag.attribute("preserveAspectRatio").unwrap_or("");
                    let mut words = par.split_whitespace();
                    let mut align = words.next().unwrap_or("xMidYMid");
                    if align == "defer" {
                        align = words.next().unwrap_or("xMidYMid");
                    }
                    let slice = words.next() == Some("slice");
                    let fit = if align == "none" {
                        Affine {
                            a: sx,
                            d: sy,
                            e: -vx * sx,
                            f: -vy * sy,
                            ..Affine::IDENTITY
                        }
                    } else {
                        let s = if slice { sx.max(sy) } else { sx.min(sy) };
                        let fx = if align.starts_with("xMin") {
                            0.0
                        } else if align.starts_with("xMax") {
                            1.0
                        } else {
                            0.5
                        };
                        let fy = if align.ends_with("YMin") {
                            0.0
                        } else if align.ends_with("YMax") {
                            1.0
                        } else {
                            0.5
                        };
                        Affine {
                            a: s,
                            d: s,
                            e: -vx * s + (w - vw * s) * fx,
                            f: -vy * s + (h - vh * s) * fy,
                            ..Affine::IDENTITY
                        }
                    };
                    (
                        Affine::scale(MM_PER_PX, MM_PER_PX).then_after(&fit),
                        Some((0.0, 0.0, w * MM_PER_PX, h * MM_PER_PX)),
                    )
                }
                None => (
                    Affine::scale(MM_PER_PX, MM_PER_PX),
                    w.zip(h)
                        .map(|(w, h)| (0.0, 0.0, w * MM_PER_PX, h * MM_PER_PX)),
                ),
            }
        }
    };
    // Turn over about the page's bottom edge, its left edge on the y axis.
    let flip = match page {
        Some((x0, y0, _, h)) => Affine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            e: -x0,
            f: y0 + h,
        },
        None => Affine::scale(1.0, -1.0),
    };
    (flip.then_after(&m), page.map(|(_, _, w, h)| (w, h)))
}

/// Reads an SVG file's geometry (see the module documentation).
pub fn read(bytes: &[u8], units: SvgUnits) -> Result<Drawing, IoError> {
    if bytes.len() as u64 > MAX_SVG_BYTES {
        return Err(bad(format!(
            "the file is larger than {MAX_SVG_BYTES} bytes"
        )));
    }
    if let SvgUnits::Mm(k) = units
        && !(k.is_finite() && k > 0.0)
    {
        return Err(bad("a user unit must be a size above 0"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| bad("the file is not UTF-8 text"))?;
    let mut reader: Option<Reader> = None;
    // Open elements: name, the transform inside them, and whether they are drawn.
    let mut stack: Vec<(String, Affine, bool)> = Vec::new();
    for tag in tags(text) {
        let tag = tag.map_err(bad)?;
        match &tag {
            Tag::End { .. } => {
                stack.pop();
            }
            Tag::Start { name, empty, .. } => {
                let name: &str = name;
                let (parent, drawn) = stack
                    .last()
                    .map_or((Affine::IDENTITY, true), |(_, m, d)| (*m, *d));
                if reader.is_none() {
                    if name != "svg" {
                        return Err(bad(format!("the first element is <{name}>, not <svg>")));
                    }
                    let (root, page) = root_map(&tag, units);
                    reader = Some(Reader {
                        out: Drawing {
                            page,
                            mm_per_unit: root.det().abs().sqrt(),
                            ..Drawing::default()
                        },
                        root,
                    });
                }
                let Some(r) = reader.as_mut() else {
                    continue;
                };
                let mut here = drawn && tag.attribute("display") != Some("none");
                let mut ctm = parent;
                if let Some(t) = tag.attribute("transform") {
                    match parse_transform(t) {
                        Some(m) => ctm = ctm.then_after(&m),
                        None => {
                            if here {
                                r.skip(&format!("{name} (unreadable transform)"));
                            }
                            here = false;
                        }
                    }
                }
                if name == "svg" && !stack.is_empty() {
                    // A nested viewport: its origin moves to (x, y).
                    let at = Affine::translate(coord(&tag, "x"), coord(&tag, "y"));
                    ctm = ctm.then_after(&at);
                }
                if here {
                    if HIDDEN.contains(&name) {
                        here = false;
                    } else if UNSUPPORTED.contains(&name) {
                        r.skip(name);
                        here = false;
                    } else if !GROUPS.contains(&name) {
                        r.element(&tag, name, &ctm)?;
                    }
                }
                if !*empty {
                    if stack.len() >= MAX_DEPTH {
                        return Err(bad(format!("elements nest deeper than {MAX_DEPTH}")));
                    }
                    stack.push((name.to_owned(), ctm, here));
                }
            }
        }
    }
    reader
        .map(|r| r.out)
        .ok_or_else(|| bad("there is no <svg> element"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_mm(svg: &str) -> Drawing {
        read(svg.as_bytes(), SvgUnits::Mm(1.0)).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn pclose(p: Point2, x: f64, y: f64) -> bool {
        close(p.x, x) && close(p.y, y)
    }

    #[test]
    fn numbers_follow_the_svg_grammar() {
        assert_eq!(numbers("1.5.5-2e1,3", 10), vec![1.5, 0.5, -20.0, 3.0]);
        assert_eq!(numbers(".5e", 10), vec![0.5]);
        let (segs, ok) = parse_path("M0 0a1 1 0 0110 0", 10);
        assert!(ok);
        assert!(matches!(
            segs.as_slice(),
            [Segment::Arc {
                large: false,
                sweep: true,
                ..
            }]
        ));
    }

    #[test]
    fn lengths_convert_to_pixels_at_96_to_the_inch() {
        assert!(close(length_px("1in").unwrap(), 96.0));
        assert!(close(length_px("25.4mm").unwrap(), 96.0));
        assert!(close(length_px("72pt").unwrap(), 96.0));
        assert!(close(length_px("10").unwrap(), 10.0));
        assert!(length_px("50%").is_none());
        assert!(length_px("2em").is_none());
    }

    #[test]
    fn a_page_in_mm_with_a_view_box_maps_user_units_to_mm_with_y_up() {
        // 100 x 50 mm page showing the user box 0..200 x 0..100: 0.5 mm per unit. The
        // user point (20, 10) is 10 mm right and 5 mm down from the top-left, so 45 mm
        // above the page's bottom edge.
        let d = read(
            br#"<svg width="100mm" height="50mm" viewBox="0 0 200 100"><line x1="20" y1="10" x2="200" y2="100"/></svg>"#,
            SvgUnits::File,
        )
        .unwrap();
        assert_eq!(
            d.page.map(|(w, h)| (w.round(), h.round())),
            Some((100.0, 50.0))
        );
        assert!(close(d.mm_per_unit, 0.5));
        let Some(Shape::Line { a, b }) = d.shapes.first() else {
            panic!("{d:?}")
        };
        assert!(pclose(*a, 10.0, 45.0), "{a:?}");
        assert!(pclose(*b, 100.0, 0.0), "{b:?}");
    }

    #[test]
    fn a_file_without_units_is_in_css_pixels() {
        // 96 user units are an inch, 25.4 mm.
        let d = read(
            br#"<svg><line x1="0" y1="0" x2="96" y2="0"/></svg>"#,
            SvgUnits::File,
        )
        .unwrap();
        let Some(Shape::Line { a, b }) = d.shapes.first() else {
            panic!()
        };
        assert!(close(a.distance(*b), 25.4));
    }

    #[test]
    fn a_quarter_arc_command_is_a_circular_arc_of_radius_10() {
        // From (10, 0) to (0, 10) with radius 10, small arc, sweep 1: in SVG (y down) it
        // turns clockwise on screen about (0, 0), the quarter through (7.07, 7.07). With y
        // turned up (page-less: y' = -y) the ends are (10, 0) and (0, -10) about (0, 0),
        // counter-clockwise from (0, -10) at -90° to (10, 0) at 0°: length 10 π / 2.
        let d = read_mm(r#"<svg><path d="M10 0 A10 10 0 0 1 0 10"/></svg>"#);
        let Some(Shape::Arc {
            centre,
            radius,
            start,
            end,
        }) = d.shapes.first()
        else {
            panic!("{d:?}")
        };
        assert!(pclose(*centre, 0.0, 0.0), "{centre:?}");
        assert!(close(*radius, 10.0));
        assert!(close(*start, -PI / 2.0), "{start}");
        assert!(close(*end, 0.0), "{end}");
        let sweep = (end - start).rem_euclid(TAU);
        assert!(close(radius * sweep, 5.0 * PI));
    }

    #[test]
    fn a_large_arc_flag_takes_the_long_way_round() {
        // Same ends and radius, large-arc 1: three quarters, length 15 π.
        let d = read_mm(r#"<svg><path d="M10 0 A10 10 0 1 1 0 10"/></svg>"#);
        let Some(Shape::Arc {
            radius, start, end, ..
        }) = d.shapes.first()
        else {
            panic!("{d:?}")
        };
        let sweep = (end - start).rem_euclid(TAU);
        assert!(close(radius * sweep, 15.0 * PI), "{}", radius * sweep);
    }

    #[test]
    fn radii_too_small_are_scaled_up_to_a_half_circle() {
        // Ends 20 apart with radius 1: scaled to 10, a half circle about (10, 0).
        let d = read_mm(r#"<svg><path d="M0 0 A1 1 0 0 0 20 0"/></svg>"#);
        let Some(Shape::Arc { centre, radius, .. }) = d.shapes.first() else {
            panic!("{d:?}")
        };
        assert!(pclose(*centre, 10.0, 0.0) && close(*radius, 10.0));
    }

    #[test]
    fn a_circle_under_a_stretch_is_an_ellipse() {
        // r = 5 under scale(2, 1): semi-axes 10 along x and 5 along y.
        let d = read_mm(r#"<svg><circle cx="0" cy="0" r="5" transform="scale(2,1)"/></svg>"#);
        let Some(Shape::Ellipse {
            major,
            ratio,
            start,
            end,
            ..
        }) = d.shapes.first()
        else {
            panic!("{d:?}")
        };
        assert!(close(major.length(), 10.0) && close(major.y, 0.0));
        assert!(close(*ratio, 0.5));
        assert!(close(end - start, TAU));
    }

    #[test]
    fn a_rotated_circle_stays_a_circle() {
        let d = read_mm(
            r#"<svg><g transform="translate(10 20) rotate(30)"><circle cx="5" cy="0" r="3"/></g></svg>"#,
        );
        // The centre (5, 0) turned 30° then moved by (10, 20): (10 + 5 cos 30°,
        // 20 + 5 sin 30°), and y turned up.
        let Some(Shape::Circle { centre, radius }) = d.shapes.first() else {
            panic!("{d:?}")
        };
        let (s, c) = (PI / 6.0).sin_cos();
        assert!(
            pclose(*centre, 10.0 + 5.0 * c, -(20.0 + 5.0 * s)),
            "{centre:?}"
        );
        assert!(close(*radius, 3.0));
    }

    #[test]
    fn beziers_keep_their_control_points_and_quadratics_are_raised_exactly() {
        let d = read_mm(
            r#"<svg><path d="M0 0 C 10 0 20 10 30 10 S 50 20 60 0 Q 70 -20 80 0 T 100 0"/></svg>"#,
        );
        let cubics: Vec<[Point2; 4]> = d
            .shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Cubic(p) => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(cubics.len(), 4);
        // C: as written, y turned up.
        assert!(pclose(cubics[0][1], 10.0, 0.0) && pclose(cubics[0][3], 30.0, -10.0));
        // S: the first control is (20, 10) reflected about (30, 10): (40, 10).
        assert!(pclose(cubics[1][1], 40.0, -10.0));
        // Q (70, -20) from (60, 0) to (80, 0): controls 60 + 2/3·10, 0 + 2/3·(-20) and
        // 80 - 2/3·10, -40/3; y up flips the sign.
        assert!(pclose(cubics[2][1], 60.0 + 20.0 / 3.0, 40.0 / 3.0));
        assert!(pclose(cubics[2][2], 80.0 - 20.0 / 3.0, 40.0 / 3.0));
        // T: the control (70, -20) reflected about (80, 0) is (90, 20).
        assert!(pclose(cubics[3][1], 80.0 + 20.0 / 3.0, -40.0 / 3.0));
    }

    #[test]
    fn rects_polygons_and_closing_make_closed_outlines() {
        let d = read_mm(
            r#"<svg><rect x="0" y="0" width="10" height="5"/><polygon points="0,0 4,0 0,3"/><path d="M0 0 h10 v10 z"/></svg>"#,
        );
        let lines = d
            .shapes
            .iter()
            .filter(|s| matches!(s, Shape::Line { .. }))
            .count();
        assert_eq!(lines, 4 + 3 + 3);
        let rounded = read_mm(r#"<svg><rect width="20" height="10" rx="2"/></svg>"#);
        let arcs = rounded
            .shapes
            .iter()
            .filter(|s| matches!(s, Shape::Arc { .. }))
            .count();
        assert_eq!((rounded.shapes.len(), arcs), (8, 4));
    }

    #[test]
    fn hidden_and_unsupported_elements_are_left_out() {
        let d = read_mm(
            r#"<svg><defs><circle r="4"/></defs><text>hi</text><use href="x"/><g display="none"><circle r="1"/></g><circle r="2"/></svg>"#,
        );
        assert_eq!(d.shapes.len(), 1);
        assert_eq!(d.skipped.get("text"), Some(&1));
        assert_eq!(d.skipped.get("use"), Some(&1));
    }

    #[test]
    fn hostile_files_never_panic() {
        let cases: Vec<String> = vec![
            String::new(),
            "<svg".into(),
            "<notsvg/>".into(),
            "<svg><path d=\"M 1e400 0 L 0 0\"/></svg>".into(),
            "<svg><path d=\"L 1 1 M\"/></svg>".into(),
            "<svg><path d=\"M0 0 A 0 0 0 0 0 1 1 A 1e308 1e308 0 1 1 2 2 A -1 5 999 1 0 0 0\"/></svg>".into(),
            "<svg><path d=\"M0 0 C\"/><path d=\"Z Z Z m\"/><path d=\"M0,0,,1\"/></svg>".into(),
            "<svg><circle r=\"NaN\"/><circle r=\"-1\"/><ellipse rx=\"1\" ry=\"0\"/></svg>".into(),
            "<svg><g transform=\"matrix(0 0 0 0 0 0)\"><circle r=\"1\"/></g></svg>".into(),
            "<svg><g transform=\"scale(1e308) scale(1e308)\"><circle r=\"1\"/></g></svg>".into(),
            "<svg><g transform=\"rotate(\"><circle r=\"1\"/></g></svg>".into(),
            "<svg viewBox=\"0 0 0 0\" width=\"-3mm\"><rect width=\"1e308\" height=\"1e308\" rx=\"1e308\"/></svg>".into(),
            format!("<svg>{}</svg>", "<g>".repeat(MAX_DEPTH + 5)),
            format!("<svg><path d=\"M0 0{}\"/></svg>", " l1 1".repeat(MAX_SHAPES + 5)),
            format!("<svg><polyline points=\"{}\"/></svg>", "1 2 ".repeat(30_000)),
        ];
        for c in &cases {
            let _ = read(c.as_bytes(), SvgUnits::File);
            let _ = read(c.as_bytes(), SvgUnits::Mm(1.0));
        }
        assert!(read(b"<svg/>", SvgUnits::Mm(f64::NAN)).is_err());
        assert!(read(&[0xff, 0xfe], SvgUnits::File).is_err());
    }
}
