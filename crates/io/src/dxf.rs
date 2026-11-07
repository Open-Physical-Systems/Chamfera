//! 2D DXF import and export, from the public DXF Reference (Autodesk): ASCII files of
//! releases R12 to R2018.
//!
//! A DXF file is a list of group code / value line pairs. Reading takes `$INSUNITS` from
//! the HEADER section and the 2D entities of the ENTITIES section, in drawing units:
//! LINE, CIRCLE, ARC, LWPOLYLINE and POLYLINE (with bulges), POINT, ELLIPSE and SPLINE.
//! Entities whose object coordinate system is not parallel to the XY plane, paper space
//! entities and other entity types are counted in `skipped`. (Sketches are written with
//! the drawing crate's R12 writer, `chamfera_drawing::dxf`.)
//!
//! Files are hostile input: sizes and counts are capped and every number is checked.

use crate::IoError;
use chamfera_geom::{Point2, Vec2};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

/// Largest DXF file accepted (bytes).
pub const MAX_DXF_BYTES: u64 = 256 * 1024 * 1024;
/// Most entities read from one file.
pub const MAX_ENTITIES: usize = 10_000;
/// Most vertices (or spline points) in one entity.
pub const MAX_VERTICES: usize = 10_000;

fn bad(why: impl std::fmt::Display) -> IoError {
    IoError::Format(format!("DXF: {why}"))
}

/// A 2D entity, in drawing units. Angles are radians; arcs run counter-clockwise.
#[derive(Debug, Clone, PartialEq)]
pub enum Entity {
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
    /// Vertices with the bulge of the segment that starts at each (tan of a quarter of
    /// its included angle; positive is counter-clockwise).
    Polyline {
        vertices: Vec<(Point2, f64)>,
        closed: bool,
    },
    Point(Point2),
    /// The ellipse `centre + cos t * major + sin t * minor`, where `minor` is `major`
    /// turned a quarter counter-clockwise and scaled by `ratio`, from `start` to `end`.
    Ellipse {
        centre: Point2,
        major: Vec2,
        ratio: f64,
        start: f64,
        end: f64,
    },
    /// A spline through `fit` points if it has them, else on its `control` points.
    Spline {
        degree: u32,
        closed: bool,
        control: Vec<Point2>,
        fit: Vec<Point2>,
    },
}

/// What a DXF file holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawing {
    /// `$INSUNITS`, when the header has it.
    pub insunits: Option<i32>,
    pub entities: Vec<Entity>,
    /// Entities left out, by type (and why, for those that are not flat in XY).
    pub skipped: BTreeMap<String, usize>,
}

/// Millimetres per drawing unit for an `$INSUNITS` code, or `None` for unitless (0) and
/// codes without a fixed length.
pub fn insunits_mm(code: i32) -> Option<f64> {
    Some(match code {
        1 => 25.4,
        2 => 304.8,
        3 => 1_609_344.0,
        4 => 1.0,
        5 => 10.0,
        6 => 1000.0,
        7 => 1_000_000.0,
        8 => 25.4e-6,
        9 => 0.0254,
        10 => 914.4,
        11 => 1e-7,
        12 => 1e-6,
        13 => 1e-3,
        14 => 100.0,
        _ => return None,
    })
}

/// The group code / value pairs.
fn pairs(text: &str) -> Result<Vec<(i32, &str)>, IoError> {
    let mut lines = text.lines();
    let mut out = Vec::new();
    while let Some(code) = lines.next() {
        let code = code.trim();
        if code.is_empty() && out.is_empty() {
            continue;
        }
        let value = lines
            .next()
            .ok_or_else(|| bad("the file ends in the middle of a group"))?;
        let code: i32 = code
            .parse()
            .map_err(|_| bad(format!("{code:?} is not a group code")))?;
        out.push((code, value.trim()));
    }
    Ok(out)
}

/// The groups of one entity, from its `0` group to the next.
struct Groups<'a>(&'a [(i32, &'a str)]);

impl Groups<'_> {
    fn text(&self, code: i32) -> Option<&str> {
        self.0.iter().find(|(c, _)| *c == code).map(|(_, v)| *v)
    }

    fn num(&self, code: i32) -> Result<Option<f64>, IoError> {
        match self.text(code) {
            None => Ok(None),
            Some(v) => number(v).map(Some),
        }
    }

    fn req(&self, code: i32, what: &str) -> Result<f64, IoError> {
        self.num(code)?
            .ok_or_else(|| bad(format!("{what} is missing group {code}")))
    }

    fn int(&self, code: i32) -> Result<i64, IoError> {
        match self.text(code) {
            None => Ok(0),
            Some(v) => v
                .parse()
                .map_err(|_| bad(format!("group {code} must be a whole number, not {v:?}"))),
        }
    }

    fn point(&self, x: i32, what: &str) -> Result<Point2, IoError> {
        Ok(Point2::new(self.req(x, what)?, self.req(x + 10, what)?))
    }

    /// The extrusion direction (210, 220, 230), default +Z.
    fn normal(&self) -> Result<[f64; 3], IoError> {
        Ok([
            self.num(210)?.unwrap_or(0.0),
            self.num(220)?.unwrap_or(0.0),
            self.num(230)?.unwrap_or(1.0),
        ])
    }

    /// Every value of `code`, in order.
    fn all(&self, code: i32) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .filter(move |(c, _)| *c == code)
            .map(|(_, v)| *v)
    }
}

fn number(v: &str) -> Result<f64, IoError> {
    v.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && v.abs() <= 1e12)
        .ok_or_else(|| bad(format!("{v:?} is not a finite number")))
}

/// How an object coordinate system maps onto XY: `Some(false)` for +Z, `Some(true)` for
/// -Z (x is mirrored, so turning directions swap), `None` when it is tilted.
fn ocs(normal: [f64; 3]) -> Option<bool> {
    let [x, y, z] = normal;
    let len = (x * x + y * y + z * z).sqrt();
    if len.is_nan() || len <= 0.0 {
        return None;
    }
    // The arbitrary axis algorithm: for N = (0, 0, -1), Ax = Wy x N = (-1, 0, 0) and
    // Ay = N x Ax = (0, 1, 0), so OCS (x, y) is WCS (-x, y).
    if (x / len).abs() > 1e-9 || (y / len).abs() > 1e-9 {
        return None;
    }
    Some(z < 0.0)
}

/// Reads a DXF file.
pub fn read(bytes: &[u8]) -> Result<Drawing, IoError> {
    if bytes.len() as u64 > MAX_DXF_BYTES {
        return Err(bad(format!(
            "the file is larger than {MAX_DXF_BYTES} bytes"
        )));
    }
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return Err(bad(
            "binary DXF is not supported; save the drawing as ASCII DXF",
        ));
    }
    let text = String::from_utf8_lossy(bytes);
    let pairs = pairs(&text)?;
    let mut drawing = Drawing::default();
    let mut section: Option<&str> = None;
    let mut i = 0;
    while i < pairs.len() {
        let (code, value) = pairs[i];
        if code == 0 && value == "SECTION" {
            section = pairs.get(i + 1).filter(|(c, _)| *c == 2).map(|(_, v)| *v);
            i += 2;
            continue;
        }
        if code == 0 && value == "ENDSEC" {
            section = None;
            i += 1;
            continue;
        }
        match section {
            Some("HEADER") if code == 9 && value == "$INSUNITS" => {
                if let Some((70, v)) = pairs.get(i + 1) {
                    drawing.insunits = v.parse().ok();
                }
                i += 2;
            }
            Some("ENTITIES") if code == 0 => {
                let end = next_entity(&pairs, i + 1);
                let kind = value;
                if kind == "POLYLINE" {
                    let (entity, after) = polyline(&pairs, i, end)?;
                    keep(&mut drawing, "POLYLINE", entity)?;
                    i = after;
                } else {
                    let g = Groups(pairs.get(i + 1..end).unwrap_or(&[]));
                    let entity = if g.int(67)? == 1 {
                        Err("in paper space".to_owned())
                    } else {
                        entity(kind, &g)?
                    };
                    keep(&mut drawing, kind, entity)?;
                    i = end;
                }
            }
            _ => i += 1,
        }
    }
    Ok(drawing)
}

/// Adds an entity, or counts why it was left out.
fn keep(drawing: &mut Drawing, kind: &str, entity: Result<Entity, String>) -> Result<(), IoError> {
    match entity {
        Ok(e) => {
            if drawing.entities.len() >= MAX_ENTITIES {
                return Err(bad(format!("more than {MAX_ENTITIES} entities")));
            }
            drawing.entities.push(e);
        }
        Err(why) => {
            let key = if why.is_empty() {
                kind.to_owned()
            } else {
                format!("{kind} ({why})")
            };
            let key: String = key.chars().filter(|c| !c.is_control()).take(80).collect();
            if drawing.skipped.len() < 100 || drawing.skipped.contains_key(&key) {
                *drawing.skipped.entry(key).or_insert(0) += 1;
            }
        }
    }
    Ok(())
}

/// The index of the next `0` group at or after `from`.
fn next_entity(pairs: &[(i32, &str)], from: usize) -> usize {
    pairs
        .get(from..)
        .and_then(|rest| rest.iter().position(|(c, _)| *c == 0))
        .map_or(pairs.len(), |k| from + k)
}

const TILTED: &str = "not parallel to the XY plane";

/// One entity, or why it is left out (empty: an unsupported type).
fn entity(kind: &str, g: &Groups) -> Result<Result<Entity, String>, IoError> {
    let flip = |n| ocs(n).ok_or_else(|| TILTED.to_owned());
    Ok(match kind {
        "LINE" => Ok(Entity::Line {
            a: g.point(10, "LINE")?,
            b: g.point(11, "LINE")?,
        }),
        "POINT" => Ok(Entity::Point(g.point(10, "POINT")?)),
        "CIRCLE" | "ARC" => {
            let mirror = match flip(g.normal()?) {
                Ok(m) => m,
                Err(e) => return Ok(Err(e)),
            };
            let mut centre = g.point(10, kind)?;
            let radius = g.req(40, kind)?;
            if radius <= 0.0 {
                return Ok(Err("radius not above 0".into()));
            }
            if mirror {
                centre.x = -centre.x;
            }
            if kind == "CIRCLE" {
                Ok(Entity::Circle { centre, radius })
            } else {
                let mut start = g.req(50, kind)?.to_radians();
                let mut end = g.req(51, kind)?.to_radians();
                if mirror {
                    // Mirroring x maps the angle a to pi - a and reverses the turn.
                    (start, end) = (PI - end, PI - start);
                }
                Ok(Entity::Arc {
                    centre,
                    radius,
                    start,
                    end,
                })
            }
        }
        "LWPOLYLINE" => {
            let mirror = match flip(g.normal()?) {
                Ok(m) => m,
                Err(e) => return Ok(Err(e)),
            };
            let closed = g.int(70)? & 1 == 1;
            let mut vertices: Vec<(Point2, f64)> = Vec::new();
            let mut x: Option<f64> = None;
            for (code, value) in g.0 {
                match code {
                    10 => x = Some(number(value)?),
                    20 => {
                        let px = x.take().ok_or_else(|| bad("LWPOLYLINE: y before x"))?;
                        if vertices.len() >= MAX_VERTICES {
                            return Err(bad(format!(
                                "LWPOLYLINE has more than {MAX_VERTICES} vertices"
                            )));
                        }
                        vertices.push((Point2::new(px, number(value)?), 0.0));
                    }
                    42 => {
                        if let Some(last) = vertices.last_mut() {
                            last.1 = number(value)?;
                        }
                    }
                    _ => {}
                }
            }
            if mirror {
                for (p, b) in &mut vertices {
                    p.x = -p.x;
                    *b = -*b;
                }
            }
            Ok(Entity::Polyline { vertices, closed })
        }
        "ELLIPSE" => {
            let n = g.normal()?;
            let mirror = match flip(n) {
                Ok(m) => m,
                Err(e) => return Ok(Err(e)),
            };
            let centre = g.point(10, kind)?;
            let m = g.point(11, kind)?;
            let major = Vec2::new(m.x, m.y);
            let ratio = g.req(40, kind)?;
            if !(ratio > 0.0 && ratio <= 1.0 + 1e-9) || major.length() <= 0.0 {
                return Ok(Err("bad axes".into()));
            }
            let mut start = g.num(41)?.unwrap_or(0.0);
            let mut end = g.num(42)?.unwrap_or(TAU);
            if mirror {
                // Seen from +Z the parameter runs the other way: t becomes -t.
                (start, end) = (-end, -start);
            }
            Ok(Entity::Ellipse {
                centre,
                major,
                ratio,
                start,
                end,
            })
        }
        "SPLINE" => {
            let flags = g.int(70)?;
            let degree = u32::try_from(g.int(71)?).unwrap_or(3);
            let read = |code: i32| -> Result<Vec<Point2>, IoError> {
                let xs: Vec<&str> = g.all(code).collect();
                let ys: Vec<&str> = g.all(code + 10).collect();
                if xs.len() > MAX_VERTICES {
                    return Err(bad(format!("SPLINE has more than {MAX_VERTICES} points")));
                }
                xs.iter()
                    .zip(ys.iter())
                    .map(|(x, y)| Ok(Point2::new(number(x)?, number(y)?)))
                    .collect()
            };
            let n = g.normal()?;
            if ocs(n).is_none() && flags & 8 == 8 {
                return Ok(Err(TILTED.into()));
            }
            Ok(Entity::Spline {
                degree,
                closed: flags & 1 == 1,
                control: read(10)?,
                fit: read(11)?,
            })
        }
        _ => Err(String::new()),
    })
}

/// A POLYLINE and its VERTEX entities up to SEQEND; returns the index after them.
fn polyline(
    pairs: &[(i32, &str)],
    at: usize,
    end: usize,
) -> Result<(Result<Entity, String>, usize), IoError> {
    let g = Groups(pairs.get(at + 1..end).unwrap_or(&[]));
    let flags = g.int(70)?;
    let mirror = ocs(g.normal()?);
    let mut vertices = Vec::new();
    let mut i = end;
    while let Some((0, kind)) = pairs.get(i) {
        let next = next_entity(pairs, i + 1);
        if *kind != "VERTEX" {
            if *kind == "SEQEND" {
                i = next;
            }
            break;
        }
        let v = Groups(pairs.get(i + 1..next).unwrap_or(&[]));
        if vertices.len() >= MAX_VERTICES {
            return Err(bad(format!(
                "POLYLINE has more than {MAX_VERTICES} vertices"
            )));
        }
        // Spline frame points (flag 16) are not on the curve; fit vertices (8) are.
        if v.int(70)? & 16 == 0 {
            let p = v.point(10, "VERTEX")?;
            vertices.push((p, v.num(42)?.unwrap_or(0.0)));
        }
        i = next;
    }
    // 3D polylines (8), polygon meshes (16) and polyface meshes (64) are not 2D outlines.
    let entity = if flags & (8 | 16 | 64) != 0 {
        Err("3D or mesh".to_owned())
    } else if g.int(67)? == 1 {
        Err("in paper space".to_owned())
    } else {
        match mirror {
            None => Err(TILTED.to_owned()),
            Some(m) => {
                if m {
                    for (p, b) in &mut vertices {
                        p.x = -p.x;
                        *b = -*b;
                    }
                }
                Ok(Entity::Polyline {
                    vertices,
                    closed: flags & 1 == 1,
                })
            }
        }
    };
    Ok((entity, i))
}

/// The arc of a polyline segment from `a` to `b` with `bulge`: centre, radius and the
/// start and end angles counter-clockwise (a negative bulge turns clockwise from `a`, so
/// its counter-clockwise arc runs from `b` to `a`). `None` when the segment is straight.
pub fn bulge_arc(a: Point2, b: Point2, bulge: f64) -> Option<(Point2, f64, f64, f64)> {
    let chord = b - a;
    let c = chord.length();
    if bulge.abs() < 1e-9 || c <= 0.0 {
        return None;
    }
    // The included angle is 4 atan(bulge); the centre is off the chord's midpoint, to the
    // left for a positive bulge, by (c / 2) (1 - bulge^2) / (2 bulge); the radius is
    // (c / 2) (1 + bulge^2) / (2 |bulge|).
    let half = c / 2.0;
    let mid = a + chord * 0.5;
    let left = chord.perp() * (1.0 / c);
    let centre = mid + left * (half * (1.0 - bulge * bulge) / (2.0 * bulge));
    let radius = half * (1.0 + bulge * bulge) / (2.0 * bulge.abs());
    let ang = |p: Point2| (p - centre).angle();
    if bulge > 0.0 {
        Some((centre, radius, ang(a), ang(b)))
    } else {
        Some((centre, radius, ang(b), ang(a)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dxf(header: &str, entities: &str) -> String {
        format!(
            "0\nSECTION\n2\nHEADER\n{header}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
        )
    }

    #[test]
    fn lines_circles_and_arcs_are_read() {
        let text = dxf(
            "9\n$INSUNITS\n70\n1\n",
            "0\nLINE\n8\n0\n10\n0\n20\n0\n30\n0\n11\n40\n21\n0\n31\n0\n\
             0\nCIRCLE\n10\n20\n20\n10\n40\n5\n\
             0\nARC\n10\n0\n20\n0\n40\n2\n50\n0\n51\n90\n\
             0\nTEXT\n1\nhello\n",
        );
        let d = read(text.as_bytes()).unwrap();
        assert_eq!(d.insunits, Some(1));
        assert_eq!(d.entities.len(), 3);
        assert_eq!(
            d.entities[1],
            Entity::Circle {
                centre: Point2::new(20.0, 10.0),
                radius: 5.0
            }
        );
        let Entity::Arc { start, end, .. } = d.entities[2] else {
            panic!()
        };
        assert!(start.abs() < 1e-12 && (end - PI / 2.0).abs() < 1e-12);
        assert_eq!(d.skipped.get("TEXT"), Some(&1));
    }

    #[test]
    fn a_bulge_of_one_is_a_semicircle() {
        // From (0, 0) to (10, 0): included angle 4 atan(1) = pi, so the centre is the
        // chord's midpoint (5, 0) and the radius is 5; counter-clockwise from angle pi to
        // angle 0 (2 pi), passing below the chord through (5, -5).
        let (c, r, s, e) = bulge_arc(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0), 1.0).unwrap();
        assert!(c.distance(Point2::new(5.0, 0.0)) < 1e-12);
        assert!((r - 5.0).abs() < 1e-12);
        assert!((s.abs() - PI).abs() < 1e-12 && e.abs() < 1e-12);
        // A quarter circle: bulge tan(pi / 8) from (1, 0) to (0, 1) around the origin.
        let (c, r, ..) = bulge_arc(
            Point2::new(1.0, 0.0),
            Point2::new(0.0, 1.0),
            (PI / 8.0).tan(),
        )
        .unwrap();
        assert!(c.distance(Point2::new(0.0, 0.0)) < 1e-12 && (r - 1.0).abs() < 1e-12);
    }

    #[test]
    fn lwpolylines_and_polylines_are_read() {
        let text = dxf(
            "",
            "0\nLWPOLYLINE\n90\n2\n70\n1\n10\n0\n20\n0\n42\n1\n10\n10\n20\n0\n\
             0\nPOLYLINE\n66\n1\n70\n1\n0\nVERTEX\n10\n0\n20\n0\n0\nVERTEX\n10\n5\n20\n0\n42\n-0.5\n\
             0\nVERTEX\n10\n5\n20\n5\n0\nSEQEND\n\
             0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n",
        );
        let d = read(text.as_bytes()).unwrap();
        assert_eq!(d.entities.len(), 3);
        assert_eq!(
            d.entities[0],
            Entity::Polyline {
                vertices: vec![(Point2::new(0.0, 0.0), 1.0), (Point2::new(10.0, 0.0), 0.0)],
                closed: true
            }
        );
        let Entity::Polyline { vertices, closed } = &d.entities[1] else {
            panic!()
        };
        assert!(*closed && vertices.len() == 3 && vertices[1].1 == -0.5);
        assert!(matches!(d.entities[2], Entity::Line { .. }));
    }

    #[test]
    fn a_mirrored_object_coordinate_system_flips_x() {
        // Extrusion (0, 0, -1): OCS (x, y) is WCS (-x, y); the arc 0..90 degrees becomes
        // 90..180 degrees.
        let text = dxf("", "0\nARC\n10\n3\n20\n4\n40\n1\n50\n0\n51\n90\n230\n-1\n");
        let d = read(text.as_bytes()).unwrap();
        let Entity::Arc {
            centre, start, end, ..
        } = d.entities[0]
        else {
            panic!()
        };
        assert_eq!(centre, Point2::new(-3.0, 4.0));
        assert!((start - PI / 2.0).abs() < 1e-12 && (end - PI).abs() < 1e-12);
        let tilted = dxf("", "0\nCIRCLE\n10\n0\n20\n0\n40\n1\n210\n1\n230\n0\n");
        let d = read(tilted.as_bytes()).unwrap();
        assert!(d.entities.is_empty() && d.skipped.len() == 1);
    }

    #[test]
    fn ellipses_and_splines_are_read() {
        let text = dxf(
            "",
            "0\nELLIPSE\n10\n0\n20\n0\n11\n10\n21\n0\n40\n0.5\n41\n0\n42\n6.283185307179586\n\
             0\nSPLINE\n70\n8\n71\n3\n10\n0\n20\n0\n10\n1\n20\n1\n10\n2\n20\n0\n10\n3\n20\n1\n",
        );
        let d = read(text.as_bytes()).unwrap();
        assert!(matches!(d.entities[0], Entity::Ellipse { ratio, .. } if ratio == 0.5));
        let Entity::Spline {
            control,
            fit,
            degree,
            closed,
        } = &d.entities[1]
        else {
            panic!()
        };
        assert_eq!(
            (control.len(), fit.len(), *degree, *closed),
            (4, 0, 3, false)
        );
    }

    #[test]
    fn hostile_files_are_refused_or_skipped_without_panicking() {
        let cases = [
            "0\nSECTION\n2\nENTITIES\n0\nLINE\n10\nnan\n20\n0\n11\n1\n21\n1\n0\nENDSEC\n"
                .to_owned(),
            "0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n".to_owned(),
            "0\nSECTION\n2\nENTITIES\n0\nCIRCLE\n10\n0\n20\n0\n40\n1e300\n".to_owned(),
            "zero\nSECTION\n".to_owned(),
            "0\n".to_owned(),
            dxf("", "0\nLWPOLYLINE\n20\n1\n"),
            dxf("", "0\nLWPOLYLINE\n10\n1\n20\ninf\n"),
        ];
        for c in &cases {
            assert!(read(c.as_bytes()).is_err(), "{c:?}");
        }
        // Too many entities.
        let many = "0\nPOINT\n10\n0\n20\n0\n".repeat(MAX_ENTITIES + 1);
        assert!(read(dxf("", &many).as_bytes()).is_err());
        // Garbage, and every cut of a real file, never panic.
        let _ = read(&[0xff; 64]);
        let good = dxf(
            "",
            "0\nLWPOLYLINE\n10\n0\n20\n0\n42\n1\n10\n10\n20\n0\n0\nPOLYLINE\n70\n0\n0\nVERTEX\n10\n0\n20\n0\n",
        );
        for n in 0..good.len() {
            let _ = read(&good.as_bytes()[..n]);
        }
        assert!(read(b"AutoCAD Binary DXF\r\n\x1a\0").is_err());
        // Zero radius is skipped, not an error.
        let d = read(dxf("", "0\nCIRCLE\n10\n0\n20\n0\n40\n0\n").as_bytes()).unwrap();
        assert!(d.entities.is_empty());
    }
}
