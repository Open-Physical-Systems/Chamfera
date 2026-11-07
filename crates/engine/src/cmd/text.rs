//! `sketch.text`: a line (or lines) of text drawn into the active sketch as closed
//! profiles: the glyph outlines of a font (`chamfera_io::font`) as sketch lines and
//! cubic control splines (each Bézier piece a spline on its four control points), joined
//! end to end at shared points, so every letter is a region to extrude or cut.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_geom::Point2;
use chamfera_io::font::{self, Align, Segment};
use chamfera_sketch::{PointId, Sketch, SplineForm};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[CommandSpec {
    id: "sketch.text",
    label: "Text",
    params: "text: string (required) - up to 256 characters; \\n starts a new line\n\
             x, y: number (optional) - where the first line starts, on its baseline (default 0, 0)\n\
             height: number (optional) - the height of the capitals in mm (default 10)\n\
             align: string (optional) - left (default), centre or right of x\n\
             font: string (optional) - a TrueType or OpenType font file (default the bundled \
             Lato Regular)\n\
             fix: boolean (optional) - fix every point in place (default false)\n\
             replies with created_curves, created_points, contours, missing (characters the font \
             lacks), width and the sketch's profile areas",
    mutates: true,
    run: text,
}];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn unavailable(e: impl std::fmt::Display) -> CommandError {
    CommandError::Unavailable(e.to_string())
}

fn text(s: &mut Session, p: &Value) -> CmdResult {
    let text = params::req_str(p, "text")?.to_owned();
    let x = params::opt_f64(p, "x")?.unwrap_or(0.0);
    let y = params::opt_f64(p, "y")?.unwrap_or(0.0);
    let at = Point2::new(x, y);
    if !at.is_valid() {
        return Err(bad("`x`, `y` are outside the working range"));
    }
    let height = params::opt_f64(p, "height")?.unwrap_or(10.0);
    let align = match params::opt_str(p, "align")? {
        None => Align::Left,
        Some(a) => Align::parse(a)
            .ok_or_else(|| bad(format!("`align` is left, centre or right, not {a:?}")))?,
    };
    let fix = match p.get("fix") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "fix")?,
    };
    let own_font;
    let font_bytes: &[u8] = match params::opt_str(p, "font")? {
        None => font::DEFAULT_FONT,
        Some(path) => {
            own_font = chamfera_io::read_limited(std::path::Path::new(path), font::MAX_FONT_BYTES)?;
            &own_font
        }
    };
    let outline =
        font::text_outline(font_bytes, &text, at, height, align).map_err(|e| bad(e.to_string()))?;
    if outline.contours.is_empty() {
        return Err(bad(
            "the text has no outlines (only spaces or characters the font lacks)",
        ));
    }
    let sketch = active(s)?;
    let mut points = Vec::new();
    let mut curves = Vec::new();
    for contour in &outline.contours {
        let Some(first) = contour.first() else {
            continue;
        };
        let start = sketch.add_point(first.start()).map_err(unavailable)?;
        points.push(start);
        let mut at_id = start;
        for (k, seg) in contour.iter().enumerate() {
            let last = k + 1 == contour.len();
            let mut new_point = |sk: &mut Sketch, q: Point2| -> Result<PointId, CommandError> {
                let id = sk.add_point(q).map_err(unavailable)?;
                points.push(id);
                Ok(id)
            };
            let end = if last {
                start
            } else {
                new_point(sketch, seg.end())?
            };
            match seg {
                Segment::Line(..) => {
                    curves.push(sketch.add_line(at_id, end).map_err(unavailable)?);
                }
                Segment::Cubic(c) => {
                    let c1 = new_point(sketch, c[1])?;
                    let c2 = new_point(sketch, c[2])?;
                    let id = sketch
                        .add_spline(vec![at_id, c1, c2, end], SplineForm::Control, false)
                        .map_err(unavailable)?;
                    curves.push(id);
                }
            }
            at_id = end;
        }
    }
    if fix {
        for id in &points {
            sketch.fix_point(*id).map_err(unavailable)?;
        }
    }
    sketch.solve().map_err(unavailable)?;
    let areas: Vec<f64> = sketch.profiles().iter().map(|r| r.area).collect();
    Ok(json!({
        "created_curves": curves.iter().map(|c| c.0).collect::<Vec<_>>(),
        "created_points": points.iter().map(|q| q.0).collect::<Vec<_>>(),
        "contours": outline.contours.len(),
        "missing": outline.missing.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "width": outline.width,
        "profiles": areas,
    }))
}

fn active(s: &mut Session) -> Result<&mut Sketch, CommandError> {
    let id = s.active_sketch().map(|f| f.id).ok_or_else(|| {
        CommandError::Unavailable("no sketch is being edited: Create Sketch or Edit Sketch".into())
    })?;
    Ok(&mut s.doc.sketch_mut(id)?.sketch)
}
