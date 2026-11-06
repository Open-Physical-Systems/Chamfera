//! How the model is displayed: the visual style, the section plane (both view state, not
//! design) and body appearances (stored in the design). See `chamfera_render::display`.

use crate::{CmdResult, CommandError, Session, params};
use chamfera_geom::{Frame, MAX_COORDINATE};
use chamfera_render::display::{PRESETS, preset_by_id};
use chamfera_render::{Rgba, Section, VisualStyle};
use serde_json::{Value, json};

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

/// The scene's form of a stored appearance.
pub fn scene_appearance(a: &chamfera_doc::Appearance) -> chamfera_render::Appearance {
    chamfera_render::Appearance {
        colour: a.colour.map(|[r, g, b]| Rgba(r, g, b, 255)),
        opacity: a.opacity as f32,
    }
}

impl Session {
    /// Gives a freshly shown solid its exact edges and its feature's stored appearance.
    pub(crate) fn dress_body(
        &mut self,
        id: Option<u32>,
        feature: u32,
        edges: Vec<Vec<chamfera_geom::Point3>>,
    ) {
        let appearance = self
            .doc
            .appearances
            .get(&feature)
            .map(scene_appearance)
            .unwrap_or_default();
        if let Some(b) = id.and_then(|id| self.scene.body_mut(id)) {
            b.edges = edges;
            b.appearance = appearance;
        }
    }
}

/// The display state as a reply.
pub fn display_json(s: &Session) -> Value {
    let d = &s.scene.display;
    json!({
        "style": d.style.id(),
        "section": d.section.map(|x| json!({ "origin": x.origin, "normal": x.normal })),
    })
}

pub(crate) fn style(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "style")?;
    let style = VisualStyle::parse(name).ok_or_else(|| {
        let all: Vec<&str> = VisualStyle::ALL.iter().map(|v| v.id()).collect();
        bad(format!("unknown style {name:?}: use {}", all.join(", ")))
    })?;
    s.scene.display.style = style;
    Ok(display_json(s))
}

/// The plane a section starts from: an origin plane or a flat face by its full name.
fn base_plane(s: &Session, name: &str) -> Result<Frame, CommandError> {
    match name {
        "xy" => return Ok(Frame::XY),
        "xz" => return Ok(Frame::XZ),
        "yz" => return Ok(Frame::YZ),
        _ => {}
    }
    if crate::solids::split_face_key(name).is_none() {
        return Err(bad(format!(
            "unknown plane {name:?}: use xy, xz, yz or a face name such as \"2:top\""
        )));
    }
    for solid in s.solids() {
        for face in solid.body.faces() {
            if solid.body.face_name(face).ok().flatten() == Some(name) {
                return crate::solids::face_plane(&solid.body, face)
                    .ok_or_else(|| bad(format!("face {name:?} is not flat")));
            }
        }
    }
    Err(bad(format!("no face named {name:?}")))
}

pub(crate) fn section(s: &mut Session, p: &Value) -> CmdResult {
    let plane = base_plane(s, params::req_str(p, "plane")?)?;
    let offset = params::opt_f64(p, "offset")?.unwrap_or(0.0);
    if offset.abs() > MAX_COORDINATE {
        return Err(bad(format!("`offset` must be within ±{MAX_COORDINATE} mm")));
    }
    let flip = match p.get("flip") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "flip")?,
    };
    let n = plane.z();
    let origin = plane.origin() + n * offset;
    let normal = if flip { -n } else { n };
    s.scene.display.section =
        Some(Section::new(origin, normal).ok_or_else(|| bad("the plane is out of range"))?);
    Ok(display_json(s))
}

pub(crate) fn section_clear(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    s.scene.display.section = None;
    Ok(display_json(s))
}

/// `#rrggbb` to bytes.
fn parse_colour(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

pub(crate) fn appearance(s: &mut Session, p: &Value) -> CmdResult {
    let body = params::req_id(p, "body")?;
    if s.scene.body(body).is_none() {
        return Err(bad(format!("no body {body}")));
    }
    let preset = params::opt_str(p, "preset")?;
    let colour = params::opt_str(p, "colour")?;
    let opacity = params::opt_f64(p, "opacity")?;
    if preset.is_none() && colour.is_none() && opacity.is_none() {
        return Err(bad("give `preset`, `colour` or `opacity`"));
    }
    let feature = s
        .solids()
        .iter()
        .find(|x| x.scene_id == Some(body))
        .map(|x| x.feature);
    let mut a = match feature {
        Some(f) => s.doc.appearances.get(&f).cloned().unwrap_or_default(),
        None => s
            .scene
            .body(body)
            .map(|b| chamfera_doc::Appearance {
                colour: b.appearance.colour.map(|c| [c.0, c.1, c.2]),
                opacity: f64::from(b.appearance.opacity()),
                preset: None,
            })
            .unwrap_or_default(),
    };
    if let Some(name) = preset {
        if name == "default" {
            a = chamfera_doc::Appearance::default();
        } else {
            let found = preset_by_id(name).ok_or_else(|| {
                let all: Vec<&str> = PRESETS.iter().map(|x| x.id).collect();
                bad(format!(
                    "unknown preset {name:?}: use default, {}",
                    all.join(", ")
                ))
            })?;
            a.colour = found.appearance.colour.map(|c| [c.0, c.1, c.2]);
            // Rounded so the f32 preset reads back as the number it was written as.
            a.opacity = (f64::from(found.appearance.opacity) * 1000.0).round() / 1000.0;
            a.preset = Some(found.id.to_owned());
        }
    }
    if let Some(text) = colour {
        a.colour = if text == "default" {
            None
        } else {
            Some(parse_colour(text).ok_or_else(|| {
                bad(format!(
                    "`colour` must be #rrggbb or default, not {:?}",
                    text.chars().take(16).collect::<String>()
                ))
            })?)
        };
        a.preset = None;
    }
    if let Some(o) = opacity {
        let min = chamfera_doc::appearance::MIN_OPACITY;
        if !(min..=1.0).contains(&o) {
            return Err(bad(format!("`opacity` must be {min} to 1")));
        }
        a.opacity = o;
        if preset.is_none() {
            a.preset = None;
        }
    }
    let shown = scene_appearance(&a);
    match feature {
        Some(f) => {
            if a.is_default() {
                s.doc.appearances.remove(&f);
            } else {
                s.doc.appearances.insert(f, a.clone());
            }
        }
        None => {
            if let Some(b) = s.scene.body_mut(body) {
                b.appearance = shown;
            }
        }
    }
    Ok(json!({
        "body": body,
        "feature": feature,
        "saved": feature.is_some(),
        "colour": a.colour.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")),
        "opacity": a.opacity,
        "preset": a.preset,
    }))
}

#[cfg(test)]
mod tests {
    use super::parse_colour;

    #[test]
    fn colours_are_six_hex_digits_after_a_hash() {
        assert_eq!(parse_colour("#ff8000"), Some([255, 128, 0]));
        assert_eq!(parse_colour("#FF8000"), Some([255, 128, 0]));
        for bad in [
            "ff8000", "#ff800", "#ff80000", "#gg0000", "#", "", "#ffé000",
        ] {
            assert_eq!(parse_colour(bad), None, "{bad}");
        }
    }
}
