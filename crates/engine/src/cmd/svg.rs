//! `sketch.import_svg*`: an SVG drawing's geometry into the active sketch
//! (`chamfera_io::svg`).
//!
//! Lines stay lines, circular arcs arcs, circles circles; ellipses and elliptical arcs
//! (also circles under a stretching transform) become the sketch's ellipses; cubic and
//! quadratic Béziers become control-point splines on their four control points, which
//! is the same curve. Endpoints that meet within `JOIN_TOLERANCE` are joined with
//! coincident constraints (the DXF builder, `dxf.rs`), so closed outlines are profiles.

use super::CommandSpec;
use super::dxf::Builder;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_io::dxf::Entity;
use chamfera_io::svg::{self, Shape, SvgUnits};
use serde_json::{Value, json};
use std::path::Path;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "sketch.import_svg",
        label: "Insert SVG",
        params: "path: string (required) - an .svg file; its paths, lines, polylines, polygons, \
                 rectangles, circles and ellipses are drawn into the active sketch\n\
                 units: string (optional) - what one user unit is: \"file\" (default: from the \
                 file's width, height and viewBox, else CSS px), \"mm\", \"px\" (1/96 inch) or \
                 \"inch\"\n\
                 scale: number (optional) - a further factor, above 0; default 1",
        mutates: true,
        run: import_svg,
    },
    CommandSpec {
        id: "sketch.import_svg_bytes",
        label: "Insert SVG",
        params: "data: string (required) - the .svg file's contents, base64\n\
                 units: string (optional) - \"file\" (default), \"mm\", \"px\" or \"inch\"\n\
                 scale: number (optional) - a further factor, above 0; default 1",
        mutates: true,
        run: import_svg_bytes,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn import_svg(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let bytes = chamfera_io::read_limited(Path::new(path), svg::MAX_SVG_BYTES)?;
    import(s, &bytes, p)
}

fn import_svg_bytes(s: &mut Session, p: &Value) -> CmdResult {
    let bytes = super::design::decode_base64(params::req_str(p, "data")?, svg::MAX_SVG_BYTES)?;
    import(s, &bytes, p)
}

fn import(s: &mut Session, bytes: &[u8], p: &Value) -> CmdResult {
    let scale = match p.get("scale") {
        None | Some(Value::Null) => 1.0,
        Some(_) => params::req_f64(p, "scale")?,
    };
    if !(scale.is_finite() && scale > 0.0 && scale <= 1e6) {
        return Err(bad("`scale` must be a number above 0 and at most 1e6"));
    }
    let (unit, units) = match params::opt_str(p, "units")? {
        None | Some("file") => ("file", SvgUnits::File),
        Some("mm") => ("mm", SvgUnits::Mm(1.0)),
        Some("px") => ("px", SvgUnits::Mm(svg::MM_PER_PX)),
        Some("inch") => ("inch", SvgUnits::Mm(25.4)),
        Some(other) => {
            return Err(bad(format!(
                "`units` is \"file\", \"mm\", \"px\" or \"inch\", not {other:?}"
            )));
        }
    };
    let drawing = svg::read(bytes, units)?;
    let id = s.active_sketch().map(|f| f.id).ok_or_else(|| {
        CommandError::Unavailable("no sketch is being edited: Create Sketch or Edit Sketch".into())
    })?;
    let sk = &mut s.doc.sketch_mut(id)?.sketch;
    let mut b = Builder::new(sk, scale, drawing.skipped.clone());
    for shape in &drawing.shapes {
        match shape {
            Shape::Line { a, b: e } => b.add(&Entity::Line { a: *a, b: *e })?,
            Shape::Circle { centre, radius } => b.add(&Entity::Circle {
                centre: *centre,
                radius: *radius,
            })?,
            Shape::Arc {
                centre,
                radius,
                start,
                end,
            } => b.add(&Entity::Arc {
                centre: *centre,
                radius: *radius,
                start: *start,
                end: *end,
            })?,
            Shape::Ellipse {
                centre,
                major,
                ratio,
                start,
                end,
            } => b.add(&Entity::Ellipse {
                centre: *centre,
                major: *major,
                ratio: *ratio,
                start: *start,
                end: *end,
            })?,
            Shape::Cubic(q) => b.cubic(q)?,
        }
    }
    let (curves, joined, skipped) = (b.curves, b.joined, b.skipped);
    if curves == 0 {
        return Err(CommandError::Unavailable(
            "the SVG file has no paths, lines, polylines, polygons, rectangles, circles or \
             ellipses to import"
                .into(),
        ));
    }
    let sk = &mut s.doc.sketch_mut(id)?.sketch;
    sk.solve()
        .map_err(|e| CommandError::Unavailable(e.to_string()))?;
    let profiles: Vec<f64> = sk.profiles().iter().map(|r| r.area).collect();
    Ok(json!({
        "sketch": id,
        "units": unit,
        "mm_per_unit": drawing.mm_per_unit * scale,
        "page": drawing.page.map(|(w, h)| json!({"width": w * scale, "height": h * scale})),
        "scale": scale,
        "curves": curves,
        "joined": joined,
        "skipped": skipped,
        "profiles": profiles,
    }))
}
