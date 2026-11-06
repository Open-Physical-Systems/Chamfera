//! `sketch.import_dxf*` and `sketch.export_dxf*`: 2D DXF drawings into and out of sketches
//! (`chamfera_io::dxf`).
//!
//! Import draws each entity into the active sketch, on its plane, in sketch millimetres:
//! lines, circles, arcs, polylines (bulged segments become arcs), points, ellipses and
//! elliptical arcs, and splines (through their fit points, or on their control points).
//! Endpoints that meet within [`JOIN_TOLERANCE`] are joined with coincident constraints,
//! so closed outlines become profiles. Export writes the sketch's non-construction curves
//! with the drawing crate's R12 writer (`chamfera_drawing::dxf`): lines, circles and arcs
//! exactly, ellipses and splines as runs of short lines.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_drawing::Shape;
use chamfera_geom::{LINEAR_TOLERANCE, Point2, is_valid_coordinate};
use chamfera_io::dxf::{self, Entity};
use chamfera_sketch::{Constraint, Curve, PointId, Sketch, SplineForm};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::f64::consts::TAU;
use std::path::Path;

/// Endpoints closer than this (mm) are joined.
pub const JOIN_TOLERANCE: f64 = 1e-3;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "sketch.export_dxf",
        label: "Export Sketch as DXF",
        params: "path: string (required) - where to write the .dxf file (R12 ASCII, mm)\n\
                 sketch: integer (optional) - the sketch; default the one being edited",
        mutates: false,
        run: export_dxf,
    },
    CommandSpec {
        id: "sketch.export_dxf_bytes",
        label: "Export Sketch as DXF",
        params: "sketch: integer (optional) - the sketch; default the one being edited\n\
                 Replies with the .dxf file as base64 `data`; writes no file",
        mutates: false,
        run: export_dxf_bytes,
    },
    CommandSpec {
        id: "sketch.import_dxf",
        label: "Insert DXF",
        params: "path: string (required) - a 2D ASCII .dxf file; its entities are drawn into the \
                 active sketch\n\
                 units: string (optional) - \"mm\" or \"inch\"; default the file's $INSUNITS, \
                 else mm\n\
                 scale: number (optional) - a further factor, above 0; default 1",
        mutates: true,
        run: import_dxf,
    },
    CommandSpec {
        id: "sketch.import_dxf_bytes",
        label: "Insert DXF",
        params: "data: string (required) - the .dxf file's contents, base64\n\
                 units: string (optional) - \"mm\" or \"inch\"; default the file's $INSUNITS, \
                 else mm\n\
                 scale: number (optional) - a further factor, above 0; default 1",
        mutates: true,
        run: import_dxf_bytes,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn sketch_err(e: impl std::fmt::Display) -> CommandError {
    CommandError::Unavailable(e.to_string())
}

fn import_dxf(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let bytes = chamfera_io::read_limited(Path::new(path), dxf::MAX_DXF_BYTES)?;
    import(s, &bytes, p)
}

fn import_dxf_bytes(s: &mut Session, p: &Value) -> CmdResult {
    let bytes = super::design::decode_base64(params::req_str(p, "data")?, dxf::MAX_DXF_BYTES)?;
    import(s, &bytes, p)
}

/// The unit's name and millimetres per drawing unit.
fn units(p: &Value, insunits: Option<i32>) -> Result<(String, f64), CommandError> {
    match params::opt_str(p, "units")? {
        Some("mm") => Ok(("mm".into(), 1.0)),
        Some("inch") => Ok(("inch".into(), 25.4)),
        Some(other) => Err(bad(format!("`units` is \"mm\" or \"inch\", not {other:?}"))),
        None => Ok(
            match insunits.and_then(|c| dxf::insunits_mm(c).map(|mm| (c, mm))) {
                Some((1, mm)) => ("inch".into(), mm),
                Some((4, mm)) => ("mm".into(), mm),
                Some((code, mm)) => (format!("$INSUNITS {code}"), mm),
                None => ("mm".into(), 1.0),
            },
        ),
    }
}

fn import(s: &mut Session, bytes: &[u8], p: &Value) -> CmdResult {
    let scale = match p.get("scale") {
        None | Some(Value::Null) => 1.0,
        Some(_) => params::req_f64(p, "scale")?,
    };
    if !(scale.is_finite() && scale > 0.0 && scale <= 1e6) {
        return Err(bad("`scale` must be a number above 0 and at most 1e6"));
    }
    let drawing = dxf::read(bytes)?;
    let (unit, mm) = units(p, drawing.insunits)?;
    let k = mm * scale;
    if !(k.is_finite() && k > 0.0) {
        return Err(bad("the unit and scale give no usable size"));
    }
    let id = s.active_sketch().map(|f| f.id).ok_or_else(|| {
        CommandError::Unavailable("no sketch is being edited: Create Sketch or Edit Sketch".into())
    })?;
    let sk = &mut s.doc.sketch_mut(id)?.sketch;
    let mut b = Builder {
        sk,
        k,
        grid: HashMap::new(),
        curves: 0,
        points: 0,
        joined: 0,
        skipped: drawing.skipped.clone(),
    };
    for e in &drawing.entities {
        b.add(e)?;
    }
    let (curves, points, joined, skipped) = (b.curves, b.points, b.joined, b.skipped);
    if curves == 0 && points == 0 {
        return Err(CommandError::Unavailable(
            "the DXF file has no lines, arcs, circles, polylines, ellipses, splines or points \
             to import"
                .into(),
        ));
    }
    sk.solve().map_err(sketch_err)?;
    let profiles: Vec<f64> = sk.profiles().iter().map(|r| r.area).collect();
    Ok(json!({
        "sketch": id,
        "units": unit,
        "scale": k,
        "curves": curves,
        "points": points,
        "joined": joined,
        "skipped": skipped,
        "profiles": profiles,
    }))
}

/// Draws DXF entities into a sketch.
struct Builder<'a> {
    sk: &'a mut Sketch,
    /// Millimetres per drawing unit.
    k: f64,
    /// Endpoints so far, by grid cell of [`JOIN_TOLERANCE`].
    grid: HashMap<(i64, i64), Vec<(Point2, PointId)>>,
    curves: usize,
    points: usize,
    joined: usize,
    skipped: BTreeMap<String, usize>,
}

fn cell(p: Point2) -> (i64, i64) {
    (
        (p.x / JOIN_TOLERANCE).floor() as i64,
        (p.y / JOIN_TOLERANCE).floor() as i64,
    )
}

impl Builder<'_> {
    fn skip(&mut self, why: &str) {
        *self.skipped.entry(why.to_owned()).or_insert(0) += 1;
    }

    /// A drawing point in sketch millimetres.
    fn map(&self, p: Point2) -> Result<Point2, CommandError> {
        let q = Point2::new(p.x * self.k, p.y * self.k);
        if !(is_valid_coordinate(q.x) && is_valid_coordinate(q.y)) {
            return Err(bad(format!(
                "a DXF point ({}, {}) is outside the working range of +/-{} mm",
                p.x,
                p.y,
                chamfera_geom::MAX_COORDINATE
            )));
        }
        Ok(q)
    }

    fn length(&self, v: f64) -> Result<f64, CommandError> {
        let l = v * self.k;
        if !is_valid_coordinate(l) {
            return Err(bad("a DXF size is outside the working range"));
        }
        Ok(l)
    }

    fn point(&mut self, at: Point2) -> Result<PointId, CommandError> {
        self.sk.add_point(at).map_err(sketch_err)
    }

    /// A new endpoint at `at`, joined by a coincident constraint to the first endpoint
    /// already within [`JOIN_TOLERANCE`], if any.
    fn endpoint(&mut self, at: Point2) -> Result<PointId, CommandError> {
        let id = self.point(at)?;
        let (cx, cy) = cell(at);
        let mut near = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                // Coordinates are within the working range, so the cells are far from
                // overflowing.
                if let Some(list) = self.grid.get(&(cx + dx, cy + dy))
                    && let Some((_, other)) =
                        list.iter().find(|(q, _)| q.distance(at) <= JOIN_TOLERANCE)
                {
                    near = Some(*other);
                    break 'search;
                }
            }
        }
        match near {
            Some(other) => {
                self.sk
                    .add_constraint(Constraint::Coincident { a: other, b: id })
                    .map_err(sketch_err)?;
                self.joined += 1;
            }
            None => self.grid.entry((cx, cy)).or_default().push((at, id)),
        }
        Ok(id)
    }

    fn add(&mut self, e: &Entity) -> Result<(), CommandError> {
        match e {
            Entity::Line { a, b } => {
                let (a, b) = (self.map(*a)?, self.map(*b)?);
                if a.distance(b) <= JOIN_TOLERANCE {
                    self.skip("LINE (zero length)");
                    return Ok(());
                }
                let (pa, pb) = (self.endpoint(a)?, self.endpoint(b)?);
                self.sk.add_line(pa, pb).map_err(sketch_err)?;
                self.curves += 1;
            }
            Entity::Circle { centre, radius } => self.circle(*centre, *radius)?,
            Entity::Arc {
                centre,
                radius,
                start,
                end,
            } => {
                let sweep = (end - start).rem_euclid(TAU);
                if sweep < 1e-9 {
                    // Equal start and end angles: the whole circle.
                    return self.circle(*centre, *radius);
                }
                let c = self.map(*centre)?;
                let r = self.length(*radius)?;
                if r <= LINEAR_TOLERANCE {
                    self.skip("ARC (zero radius)");
                    return Ok(());
                }
                let at = |t: f64| Point2::new(c.x + r * t.cos(), c.y + r * t.sin());
                let pc = self.point(c)?;
                let (ps, pe) = (self.endpoint(at(*start))?, self.endpoint(at(*end))?);
                self.sk.add_arc(pc, ps, pe).map_err(sketch_err)?;
                self.curves += 1;
            }
            Entity::Polyline { vertices, closed } => self.polyline(vertices, *closed)?,
            Entity::Point(p) => {
                let p = self.map(*p)?;
                self.point(p)?;
                self.points += 1;
            }
            Entity::Ellipse {
                centre,
                major,
                ratio,
                start,
                end,
            } => {
                let c = self.map(*centre)?;
                let m = Point2::new(c.x + major.x * self.k, c.y + major.y * self.k);
                let minor = major.perp() * *ratio;
                let n = Point2::new(c.x + minor.x * self.k, c.y + minor.y * self.k);
                let (m, n) = (self.map_mm(m)?, self.map_mm(n)?);
                if c.distance(n) <= LINEAR_TOLERANCE {
                    self.skip("ELLIPSE (flat)");
                    return Ok(());
                }
                let sweep = end - start;
                let (pc, pm, pn) = (self.point(c)?, self.point(m)?, self.point(n)?);
                if sweep.abs() < 1e-9 || (sweep.abs() - TAU).abs() < 1e-9 || sweep.abs() > TAU {
                    self.sk.add_ellipse(pc, pm, pn).map_err(sketch_err)?;
                } else {
                    let (mv, nv) = (m - c, n - c);
                    let at = |t: f64| c + mv * t.cos() + nv * t.sin();
                    let (ps, pe) = (self.endpoint(at(*start))?, self.endpoint(at(*end))?);
                    self.sk
                        .add_elliptical_arc(pc, pm, pn, ps, pe)
                        .map_err(sketch_err)?;
                }
                self.curves += 1;
            }
            Entity::Spline {
                closed,
                control,
                fit,
                ..
            } => {
                let (raw, form) = if fit.len() >= 2 {
                    (fit, SplineForm::Fit)
                } else {
                    (control, SplineForm::Control)
                };
                let mut at: Vec<Point2> = Vec::with_capacity(raw.len());
                for p in raw {
                    let q = self.map(*p)?;
                    if at.last().is_none_or(|l| l.distance(q) > JOIN_TOLERANCE) {
                        at.push(q);
                    }
                }
                if *closed
                    && at.len() > 1
                    && at
                        .first()
                        .zip(at.last())
                        .is_some_and(|(f, l)| f.distance(*l) <= JOIN_TOLERANCE)
                {
                    at.pop();
                }
                if at.len() < if *closed { 3 } else { 2 } {
                    self.skip("SPLINE (too few points)");
                    return Ok(());
                }
                let last = at.len() - 1;
                let mut ids = Vec::with_capacity(at.len());
                for (i, q) in at.iter().enumerate() {
                    let id = if !*closed && (i == 0 || i == last) {
                        self.endpoint(*q)?
                    } else {
                        self.point(*q)?
                    };
                    ids.push(id);
                }
                self.sk.add_spline(ids, form, *closed).map_err(sketch_err)?;
                self.curves += 1;
            }
        }
        Ok(())
    }

    /// Checks a point already in millimetres.
    fn map_mm(&self, q: Point2) -> Result<Point2, CommandError> {
        if !(is_valid_coordinate(q.x) && is_valid_coordinate(q.y)) {
            return Err(bad("a DXF point is outside the working range"));
        }
        Ok(q)
    }

    fn circle(&mut self, centre: Point2, radius: f64) -> Result<(), CommandError> {
        let c = self.map(centre)?;
        let r = self.length(radius)?;
        if r <= LINEAR_TOLERANCE {
            self.skip("CIRCLE (zero radius)");
            return Ok(());
        }
        let pc = self.point(c)?;
        self.sk.add_circle(pc, r).map_err(sketch_err)?;
        self.curves += 1;
        Ok(())
    }

    fn polyline(&mut self, vertices: &[(Point2, f64)], closed: bool) -> Result<(), CommandError> {
        // Drop repeated vertices: a zero-length segment's bulge means nothing, so the
        // vertex kept takes the bulge of the segment after the repeat.
        let mut v: Vec<(Point2, f64)> = Vec::with_capacity(vertices.len());
        for (p, bulge) in vertices {
            let q = self.map(*p)?;
            match v.last_mut() {
                Some(last) if last.0.distance(q) <= JOIN_TOLERANCE => last.1 = *bulge,
                _ => v.push((q, *bulge)),
            }
        }
        if closed
            && v.len() > 1
            && v.first()
                .zip(v.last())
                .is_some_and(|(f, l)| f.0.distance(l.0) <= JOIN_TOLERANCE)
        {
            v.pop();
        }
        if v.len() < 2 {
            self.skip("POLYLINE (fewer than two vertices)");
            return Ok(());
        }
        let mut ids = Vec::with_capacity(v.len());
        for (q, _) in &v {
            ids.push(self.endpoint(*q)?);
        }
        let segments = if closed { v.len() } else { v.len() - 1 };
        for i in 0..segments {
            let j = (i + 1) % v.len();
            let (Some(&(a, bulge)), Some(&(b, _)), Some(&ia), Some(&ib)) =
                (v.get(i), v.get(j), ids.get(i), ids.get(j))
            else {
                continue;
            };
            match dxf::bulge_arc(a, b, bulge) {
                Some((c, ..)) => {
                    let c = self.map_mm(c)?;
                    let pc = self.point(c)?;
                    let (s, e) = if bulge > 0.0 { (ia, ib) } else { (ib, ia) };
                    self.sk.add_arc(pc, s, e).map_err(sketch_err)?;
                }
                None => {
                    self.sk.add_line(ia, ib).map_err(sketch_err)?;
                }
            }
            self.curves += 1;
        }
        Ok(())
    }
}

/// The sketch named by `sketch`, or the one being edited.
fn sketch_id(s: &Session, p: &Value) -> Result<u32, CommandError> {
    match p.get("sketch") {
        None | Some(Value::Null) => s.active_sketch().map(|f| f.id).ok_or_else(|| {
            CommandError::Unavailable("no sketch is being edited: name one with `sketch`".into())
        }),
        Some(_) => params::req_id(p, "sketch"),
    }
}

/// The sketch's non-construction curves as drawing shapes, in millimetres: lines, circles
/// and arcs exactly, ellipses and splines as runs of short lines.
fn shapes(sk: &Sketch) -> Vec<Shape> {
    let mut out = Vec::new();
    for (_, c) in sk.curves() {
        if c.is_construction() {
            continue;
        }
        match c {
            Curve::Line { start, end, .. } => {
                if let (Some(a), Some(b)) = (sk.point(*start), sk.point(*end)) {
                    out.push(Shape::Line { a, b });
                }
            }
            Curve::Circle { centre, radius, .. } => {
                if let Some(centre) = sk.point(*centre) {
                    out.push(Shape::Circle {
                        centre,
                        radius: *radius,
                    });
                }
            }
            Curve::Arc {
                centre, start, end, ..
            } => {
                if let (Some(c), Some(a), Some(b)) =
                    (sk.point(*centre), sk.point(*start), sk.point(*end))
                {
                    let start = (a - c).angle();
                    let mut sweep = ((b - c).angle() - start).rem_euclid(TAU);
                    if sweep <= 0.0 {
                        sweep = TAU;
                    }
                    out.push(Shape::Arc {
                        centre: c,
                        radius: c.distance(a),
                        start,
                        end: start + sweep,
                    });
                }
            }
            Curve::Ellipse { .. } | Curve::EllipticalArc { .. } | Curve::Spline { .. } => {
                let pts = crate::sketching::curve_polyline(sk, c);
                for w in pts.windows(2) {
                    if w[0].distance(w[1]) > LINEAR_TOLERANCE {
                        out.push(Shape::Line { a: w[0], b: w[1] });
                    }
                }
            }
        }
    }
    out
}

/// The sketch as an R12 DXF, written by the drawing crate's writer.
fn dxf_text(s: &Session, p: &Value) -> Result<(String, usize), CommandError> {
    let id = sketch_id(s, p)?;
    let f = s.doc.sketch(id)?;
    let list = shapes(&f.sketch);
    if list.is_empty() {
        return Err(CommandError::Unavailable(format!(
            "{} has no curves to export (construction curves are left out)",
            f.name
        )));
    }
    // The sheet size only sets the header's extents: the sketch's far corner.
    let (mut w, mut h) = (0.0f64, 0.0f64);
    for (_, p) in f.sketch.points() {
        w = w.max(p.x);
        h = h.max(p.y);
    }
    let n = list.len();
    let sheet = chamfera_drawing::SheetGeometry {
        width: w,
        height: h,
        items: list
            .into_iter()
            .map(|shape| chamfera_drawing::Item {
                layer: chamfera_drawing::Layer::Visible,
                shape,
            })
            .collect(),
        ..Default::default()
    };
    Ok((chamfera_drawing::dxf::write(&sheet), n))
}

fn export_dxf(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let (text, n) = dxf_text(s, p)?;
    std::fs::write(path, text).map_err(chamfera_io::IoError::from)?;
    Ok(json!({ "path": path, "entities": n }))
}

fn export_dxf_bytes(s: &mut Session, p: &Value) -> CmdResult {
    let (text, n) = dxf_text(s, p)?;
    Ok(json!({
        "data": super::design::encode_base64(text.as_bytes()),
        "bytes": text.len(),
        "entities": n,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn dxf(header: &str, entities: &str) -> String {
        format!(
            "0\nSECTION\n2\nHEADER\n{header}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
        )
    }

    fn line(x1: f64, y1: f64, x2: f64, y2: f64) -> String {
        format!("0\nLINE\n8\n0\n10\n{x1}\n20\n{y1}\n11\n{x2}\n21\n{y2}\n")
    }

    fn import(s: &mut Session, text: &str, extra: Value) -> CmdResult {
        let mut p = json!({ "data": super::super::design::encode_base64(text.as_bytes()) });
        if let (Some(m), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            m.extend(e.clone());
        }
        s.execute("sketch.import_dxf_bytes", &p)
    }

    fn sketching() -> Session {
        let mut s = Session::new();
        s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
        s
    }

    fn areas(reply: &Value) -> Vec<f64> {
        let mut a: Vec<f64> = reply["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        a.sort_by(f64::total_cmp);
        a
    }

    fn rectangle_and_circle() -> String {
        let mut e = String::new();
        e += &line(0.0, 0.0, 40.0, 0.0);
        e += &line(40.0, 0.0, 40.0, 20.0);
        e += &line(40.0, 20.0, 0.0, 20.0);
        e += &line(0.0, 20.0, 0.0, 0.0);
        e += "0\nCIRCLE\n10\n20\n20\n10\n40\n5\n";
        e
    }

    #[test]
    fn a_rectangle_of_four_lines_and_a_circle_give_two_profiles() {
        let mut s = sketching();
        let r = import(&mut s, &dxf("", &rectangle_and_circle()), json!({})).unwrap();
        assert_eq!(r["curves"], 5);
        // Four corners, each the end of one line and the start of the next.
        assert_eq!(r["joined"], 4);
        assert_eq!(r["units"], "mm");
        // The plate with a hole: 40 * 20 - 25 pi; the disc: 25 pi.
        let a = areas(&r);
        assert_eq!(a.len(), 2);
        assert!((a[0] - 25.0 * PI).abs() < 1e-6, "{a:?}");
        assert!((a[1] - (800.0 - 25.0 * PI)).abs() < 1e-6, "{a:?}");
        // One undo step takes it all away.
        s.execute("edit.undo", &json!({})).unwrap();
        let g = s.execute("sketch.get", &json!({})).unwrap();
        assert!(g["curves"].as_array().unwrap().is_empty());
    }

    #[test]
    fn inches_from_insunits_scale_by_25_4() {
        let mut s = sketching();
        let header = "9\n$INSUNITS\n70\n1\n";
        let r = import(&mut s, &dxf(header, &rectangle_and_circle()), json!({})).unwrap();
        assert_eq!(r["units"], "inch");
        assert_eq!(r["scale"], 25.4);
        // Areas scale by 25.4^2.
        let a = areas(&r);
        let k2 = 25.4f64 * 25.4;
        assert!((a[0] - 25.0 * PI * k2).abs() < 1e-3, "{a:?}");
        assert!((a[1] - (800.0 - 25.0 * PI) * k2).abs() < 1e-3, "{a:?}");
        // `units` overrides the header, and `scale` multiplies.
        let mut s = sketching();
        let r = import(
            &mut s,
            &dxf(header, &rectangle_and_circle()),
            json!({"units": "mm", "scale": 2}),
        )
        .unwrap();
        let a = areas(&r);
        assert!((a[0] - 100.0 * PI).abs() < 1e-6, "{a:?}");
    }

    #[test]
    fn an_lwpolyline_with_a_bulge_of_one_gives_a_semicircle() {
        // (0, 0) to (10, 0) with bulge 1, then straight back: a half disc of radius 5,
        // area 25 pi / 2.
        let mut s = sketching();
        let text = dxf(
            "",
            "0\nLWPOLYLINE\n90\n2\n70\n1\n10\n0\n20\n0\n42\n1\n10\n10\n20\n0\n",
        );
        let r = import(&mut s, &text, json!({})).unwrap();
        assert_eq!(r["curves"], 2);
        let a = areas(&r);
        assert_eq!(a.len(), 1);
        assert!((a[0] - 12.5 * PI).abs() < 1e-6, "{a:?}");
        // The arc: centre (5, 0), radius 5.
        let f = s.active_sketch().unwrap();
        let (id, c) = f
            .sketch
            .curves()
            .find(|(_, c)| matches!(c, Curve::Arc { .. }))
            .unwrap();
        assert!((f.sketch.radius(id).unwrap() - 5.0).abs() < 1e-9);
        let Curve::Arc { centre, .. } = c else {
            unreachable!()
        };
        assert!(
            f.sketch
                .point(*centre)
                .unwrap()
                .distance(Point2::new(5.0, 0.0))
                < 1e-9
        );
    }

    #[test]
    fn what_is_exported_imports_as_the_same_profiles() {
        let mut s = sketching();
        import(&mut s, &dxf("", &rectangle_and_circle()), json!({})).unwrap();
        s.execute(
            "sketch.arc",
            &json!({"cx": 100, "cy": 0, "x1": 110, "y1": 0, "x2": 100, "y2": 10}),
        )
        .unwrap();
        let out = s.execute("sketch.export_dxf_bytes", &json!({})).unwrap();
        assert_eq!(out["entities"], 6);
        let text = String::from_utf8(
            super::super::design::decode_base64(out["data"].as_str().unwrap(), 1 << 20).unwrap(),
        )
        .unwrap();
        let mut t = sketching();
        let r = import(&mut t, &text, json!({})).unwrap();
        assert_eq!(r["units"], "mm");
        let a = areas(&r);
        assert!((a[0] - 25.0 * PI).abs() < 1e-6 && (a[1] - (800.0 - 25.0 * PI)).abs() < 1e-6);
    }

    #[test]
    fn hostile_dxf_input_changes_nothing() {
        let mut s = sketching();
        let before = s.doc.clone();
        let cases = [
            json!({"data": "!!"}),
            json!({"data": super::super::design::encode_base64(b"0\nSECTION\n")}),
            json!({"data": super::super::design::encode_base64(dxf("", &line(0.0, 0.0, 1e300, 0.0)).as_bytes())}),
            json!({"data": super::super::design::encode_base64(dxf("", &line(0.0, 0.0, 9e5, 0.0)).as_bytes()), "units": "inch"}),
            json!({"data": super::super::design::encode_base64(dxf("", &rectangle_and_circle()).as_bytes()), "units": "furlong"}),
            json!({"data": super::super::design::encode_base64(dxf("", &rectangle_and_circle()).as_bytes()), "scale": 0}),
            json!({"data": super::super::design::encode_base64(dxf("", &rectangle_and_circle()).as_bytes()), "scale": f64::MAX}),
            json!({"data": super::super::design::encode_base64(dxf("", "0\nTEXT\n1\nx\n").as_bytes())}),
            json!({"data": super::super::design::encode_base64(dxf("", "0\nLINE\n10\nnan\n20\n0\n11\n1\n21\n1\n").as_bytes())}),
        ];
        for c in &cases {
            assert!(s.execute("sketch.import_dxf_bytes", c).is_err(), "{c}");
        }
        assert_eq!(s.doc, before);
        // No active sketch.
        let mut idle = Session::new();
        assert!(import(&mut idle, &dxf("", &rectangle_and_circle()), json!({})).is_err());
        assert!(idle.execute("sketch.export_dxf_bytes", &json!({})).is_err());
    }
}
