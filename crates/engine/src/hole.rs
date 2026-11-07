//! The Hole feature: simple, counterbore and countersink holes drilled into a flat face.
//!
//! Each hole is a tool solid: its half-section (radius across, depth down from the face)
//! revolved a full turn about the hole's axis, then subtracted from the body the face
//! belongs to. The face is found by name on every rebuild, so the holes follow it when the
//! body changes (a thicker plate keeps its holes, and through holes still go through).
//!
//! Faces the hole makes are named `hole k <role>` (prefixed with the feature, like every
//! other feature's faces), `k` counting the hole's points from 0:
//!
//! | Role | Face |
//! |---|---|
//! | `wall` | the drilled cylinder |
//! | `floor` | a flat bottom |
//! | `tip` | the cone of a drill point |
//! | `counterbore wall`, `counterbore floor` | the counterbore's cylinder and shoulder |
//! | `countersink` | the countersink's cone |

use crate::Session;
use crate::naming::{Kind, Want};
use crate::solids::{face_plane, name_edges, name_faces, split_face_key};
use chamfera_doc::{Document, Hole, HoleSizes};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point2, Point3, Vec2};
use chamfera_kernel::{Body, KernelError, Line2, PointOnFace, Profile, polygon_edges};

/// How far a through hole's tool goes past the far side of the body, in mm, so its end
/// is never flush with a face.
pub const THROUGH_MARGIN: f64 = 1.0;

/// The half-section of one hole in its own plane: x is the distance from the axis, y the
/// depth below the face. Returns the section's corners in order (from the axis at the
/// face) and the role of each edge (edge `i` runs from corner `i` to `i + 1`; the last one
/// closes the loop along the axis). `depth` is the drilled depth to use (a through hole's
/// depth is worked out from the body).
pub fn hole_section(s: &HoleSizes, depth: f64, through: bool) -> (Vec<Point2>, Vec<&'static str>) {
    let r = s.diameter * 0.5;
    let mut pts = vec![Point2::new(0.0, 0.0)];
    let mut roles = Vec::new();
    if let Some((cd, ch)) = s.counterbore {
        pts.extend([
            Point2::new(cd * 0.5, 0.0),
            Point2::new(cd * 0.5, ch),
            Point2::new(r, ch),
        ]);
        roles.extend(["top", "counterbore wall", "counterbore floor"]);
    } else if let Some((cd, angle)) = s.countersink {
        let sink = (cd * 0.5 - r) / (angle * 0.5).tan();
        pts.extend([Point2::new(cd * 0.5, 0.0), Point2::new(r, sink)]);
        roles.extend(["top", "countersink"]);
    } else {
        pts.push(Point2::new(r, 0.0));
        roles.push("top");
    }
    pts.push(Point2::new(r, depth));
    roles.push("wall");
    match s.tip_angle {
        Some(t) if !through => {
            pts.push(Point2::new(0.0, depth + r / (t * 0.5).tan()));
            roles.push("tip");
        }
        _ => {
            pts.push(Point2::new(0.0, depth));
            roles.push("floor");
        }
    }
    roles.push("axis");
    (pts, roles)
}

/// The depth of the mouth (counterbore or countersink) below the face.
fn mouth_depth(s: &HoleSizes) -> f64 {
    if let Some((_, ch)) = s.counterbore {
        ch
    } else if let Some((cd, angle)) = s.countersink {
        (cd - s.diameter) * 0.5 / (angle * 0.5).tan()
    } else {
        0.0
    }
}

/// The standalone points of a sketch (points no curve uses), in sketch coordinates.
pub fn sketch_hole_points(sketch: &chamfera_sketch::Sketch) -> Vec<Point2> {
    let mut used = std::collections::BTreeSet::new();
    for (_, c) in sketch.curves() {
        used.extend(c.points());
    }
    sketch
        .points()
        .filter(|(id, _)| !used.contains(id))
        .map(|(_, p)| p)
        .collect()
}

/// The hole centres of `h` in face coordinates (`face` is the face's plane): its own
/// points, then the sketch's standalone points projected onto the face.
fn hole_centres(doc: &Document, h: &Hole, face: &Frame) -> Result<Vec<Point2>, String> {
    let mut out = h.points.clone();
    if let Some(id) = h.sketch {
        let f = doc.sketch(id).map_err(|e| e.to_string())?;
        out.extend(
            sketch_hole_points(&f.sketch)
                .into_iter()
                .map(|p| face.to_plane(f.plane.from_plane(p))),
        );
    }
    if out.is_empty() {
        return Err("there are no hole points (the sketch has no standalone points)".into());
    }
    if out.len() > chamfera_doc::feature::MAX_HOLE_POINTS {
        return Err(format!(
            "at most {} hole points",
            chamfera_doc::feature::MAX_HOLE_POINTS
        ));
    }
    Ok(out)
}

/// How far the body reaches below the plane (along `-normal`), from its bounding box.
fn reach_below(body: &Body, plane: &Frame) -> Result<f64, String> {
    let b = body
        .bounding_box()
        .ok_or("the body has no extent to drill through")?;
    let mut most: f64 = 0.0;
    for i in 0..8 {
        let pick = |bit: usize, lo: f64, hi: f64| if i & bit == 0 { lo } else { hi };
        let c = Point3::new(
            pick(1, b.min.x, b.max.x),
            pick(2, b.min.y, b.max.y),
            pick(4, b.min.z, b.max.z),
        );
        most = most.max(-plane.to_local(c).z);
    }
    Ok(most)
}

/// How deep a hole on the face `plane` goes to reach the flat face `to`: the distance
/// between the parallel planes, which must lie below the hole's face (into the
/// material) and deeper than its counterbore or countersink.
fn depth_to(
    doc: &Document,
    feature: u32,
    bodies: &[&Body],
    plane: &Frame,
    to: &chamfera_doc::FaceRef,
    sizes: &HoleSizes,
) -> Result<f64, String> {
    let name = crate::naming::face_ref_name(to);
    let (_, b, f) =
        crate::naming::one_face(doc, feature, bodies, to, Want::Surface).map_err(|e| match e {
            crate::naming::NameError::Gone { .. } => {
                format!("the face {name} the hole goes to is gone")
            }
            e => e.to_string(),
        })?;
    let target = face_plane(b, f)
        .ok_or_else(|| format!("a hole goes to a flat face; {name} is not flat"))?;
    if target.z().cross(plane.z()).length() > chamfera_geom::ANGULAR_TOLERANCE.sqrt() {
        return Err(format!(
            "the face {name} is not parallel to the hole's face; a hole goes to a parallel face"
        ));
    }
    // Below the face is along -normal.
    let depth = -(target.origin() - plane.origin()).dot(plane.z());
    if !depth.is_finite() || depth <= LINEAR_TOLERANCE {
        return Err(format!(
            "the face {name} does not lie below the hole's face"
        ));
    }
    if mouth_depth(sizes) >= depth - LINEAR_TOLERANCE {
        return Err(format!(
            "the counterbore or countersink is deeper than the hole down to {name}"
        ));
    }
    Ok(depth)
}

/// One hole's tool solid at `centre` (face coordinates), its faces and edges named
/// `<feature>:hole <k> <role>`.
fn hole_tool(
    face: &Frame,
    centre: Point2,
    s: &HoleSizes,
    depth: f64,
    through: bool,
    feature: u32,
    k: usize,
) -> Result<Body, String> {
    let at = face.from_plane(centre);
    // The section's plane holds the axis: x across the face, y into the material.
    let frame = Frame::from_normal_and_x(at, face.y(), face.x())
        .ok_or("the hole's position is outside the working range")?;
    let (pts, roles) = hole_section(s, depth, through);
    let profile = Profile::new(polygon_edges(&pts), Vec::new());
    let axis = Line2::new(Point2::ORIGIN, Vec2::Y).map_err(|e| format!("hole: {e}"))?;
    let mut tool = chamfera_kernel::revolve(&frame, &profile, &axis, std::f64::consts::TAU)
        .map_err(|e| format!("hole: {e}"))?;
    for f in tool.faces() {
        let role = tool
            .face_name(f)
            .ok()
            .flatten()
            .and_then(|n| n.strip_prefix("side "))
            .and_then(|i| i.parse::<usize>().ok())
            .and_then(|i| roles.get(i).copied());
        if let Some(role) = role {
            let _ = tool.set_face_name(f, &format!("hole {k} {role}"));
        }
    }
    for e in tool.edges() {
        let name = tool.edge_name(e).ok().flatten().map(str::to_owned);
        if let Some(name) = name {
            let _ = tool.set_edge_name(e, &format!("hole {k} {name}"));
        }
    }
    name_faces(&mut tool, feature);
    name_edges(&mut tool, feature);
    Ok(tool)
}

/// The bodies built so far in a rebuild: (feature, name, body).
type Made = Vec<(u32, String, Body)>;

/// Drills the holes of feature `feature` into the body that has the face `h.face`.
/// Returns the bodies afterwards and the hole tools (so a pattern can repeat them as cuts).
pub(crate) fn build_hole(
    doc: &Document,
    h: &Hole,
    feature: u32,
    made: &[(u32, String, Body)],
) -> Result<(Made, Vec<Body>), String> {
    let sizes = doc.hole_sizes(h).map_err(|e| e.to_string())?;
    // The face through the resolver (rung K8): a piece by its `#k` name, or every piece
    // of a face that split (possibly into several bodies, when a cut divided the solid):
    // the hole centres pick the piece, as the point picked the face.
    let bodies = crate::naming::bodies(made);
    let found = crate::naming::resolve_face(doc, feature, &bodies, &h.face, Want::All).map_err(
        |e| match e {
            crate::naming::NameError::Gone { .. } => format!(
                "the face {:?} the holes are on is gone (feature {} changed or failed)",
                h.face.face, h.face.feature
            ),
            e => e.to_string(),
        },
    )?;
    let first = found.first().ok_or("the face is gone")?;
    let plane = bodies
        .get(first.body)
        .zip(first.face())
        .and_then(|(b, f)| face_plane(b, f))
        .ok_or_else(|| format!("holes go on a flat face; {:?} is not flat", h.face.face))?;
    let centres = hole_centres(doc, h, &plane)?;
    let mut index = None;
    for c in &centres {
        let p = plane.from_plane(*c);
        let on = found.iter().find(|f| {
            bodies.get(f.body).zip(f.face()).is_some_and(|(b, face)| {
                b.classify_point_on_face(face, p)
                    .is_ok_and(|x| x == PointOnFace::Inside)
            })
        });
        let Some(on) = on else {
            return Err(format!(
                "the hole centre ({:.3}, {:.3}) is not on the face {:?}",
                c.x, c.y, h.face.face
            ));
        };
        match index {
            None => index = Some(on.body),
            Some(i) if i != on.body => {
                return Err(format!(
                    "the hole centres on {:?} fall on different bodies; make one hole feature per body",
                    h.face.face
                ));
            }
            Some(_) => {}
        }
    }
    let index = index.unwrap_or(first.body);
    let body = &made.get(index).ok_or("the body is gone")?.2;
    let (depth, through) = match (sizes.depth, &h.to) {
        (_, Some(to)) => (depth_to(doc, feature, &bodies, &plane, to, &sizes)?, true),
        (Some(d), None) => (d, false),
        (None, None) => {
            let reach = reach_below(body, &plane)?;
            (
                reach.max(mouth_depth(&sizes) + LINEAR_TOLERANCE) + THROUGH_MARGIN,
                true,
            )
        }
    };
    let mut result = body.clone();
    let mut tools = Vec::new();
    for (k, c) in centres.iter().enumerate() {
        let tool = hole_tool(&plane, *c, &sizes, depth, through, feature, k)?;
        result = match crate::perf::timed(crate::perf::Phase::Booleans, || {
            chamfera_kernel::subtract(&result, &tool)
        }) {
            Ok(b) => b,
            Err(KernelError::EmptyResult) => {
                return Err("the holes would remove the whole body".into());
            }
            Err(e) => return Err(format!("hole {k}: {e}")),
        };
        tools.push(tool);
    }
    name_faces(&mut result, feature);
    name_edges(&mut result, feature);
    let mut next = made.to_vec();
    if let Some(m) = next.get_mut(index) {
        m.2 = result;
    }
    Ok((next, tools))
}

impl Session {
    /// The flat solid face under pixel `(x, y)` and the point on it there: (feature, face
    /// role, the point in the face's plane coordinates).
    pub fn face_point_at(&self, x: f64, y: f64, w: f64, h: f64) -> Option<(u32, String, Point2)> {
        let (feature, role, plane) = self.face_at(x, y, w, h)?;
        let bounds = self.scene.bounds();
        let ray = self.camera.ray_through(x, y, w, h, bounds.as_ref())?;
        let n = plane.z();
        let denom = n.dot(ray.direction);
        if denom.abs() < 1e-9 {
            return None;
        }
        let t = n.dot(plane.origin() - ray.origin) / denom;
        let p = plane.to_plane(ray.at(t));
        p.is_valid().then_some((feature, role, p))
    }

    /// The plane of the named face (`2:top`) in the solids built last, if it is flat.
    pub fn named_face_plane(&self, name: &str) -> Option<Frame> {
        split_face_key(name)?;
        let found = self
            .resolve_reference(None, Kind::Face, name, Want::Surface)
            .ok()?;
        let f = found.first()?;
        face_plane(&self.solids().get(f.body)?.body, f.face()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use serde_json::{Value, json};
    use std::f64::consts::PI;

    /// A 50 x 30 plate, `t` thick (a parameter), from the XY plane up: feature 2 (sketch 1).
    fn plate(s: &mut Session) {
        s.execute("param.set", &json!({ "name": "t", "expression": "10 mm" }))
            .unwrap();
        s.execute("sketch.create", &json!({ "plane": "xy" }))
            .unwrap();
        s.execute(
            "sketch.rectangle",
            &json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
        )
        .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute(
            "solid.extrude",
            &json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }], "distance": "t" }),
        )
        .unwrap();
    }

    /// The top face's coordinates of world point (x, y, t).
    fn top_point(s: &Session, x: f64, y: f64) -> Value {
        let plane = s.named_face_plane("2:top").unwrap();
        let p = plane.to_plane(chamfera_geom::Point3::new(x, y, 10.0));
        json!({ "x": p.x, "y": p.y })
    }

    fn volume(r: &Value) -> f64 {
        r["solids"][0]["volume"].as_f64().unwrap()
    }

    fn assert_volume(r: &Value, want: f64) {
        let v = volume(r);
        assert!((v - want).abs() < 1e-6, "volume {v}, want {want}");
    }

    fn hole(s: &mut Session, extra: Value) -> Result<Value, crate::CommandError> {
        let mut p = json!({ "face": "2:top", "points": [top_point(s, 25.0, 15.0)], "diameter": 6 });
        if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                p.insert(k.clone(), v.clone());
            }
        }
        s.execute("solid.hole", &p)
    }

    #[test]
    fn a_through_hole_removes_its_cylinder_and_follows_a_thicker_plate() {
        let mut s = Session::new();
        plate(&mut s);
        let r = hole(&mut s, json!({ "through_all": true })).unwrap();
        // 50 * 30 * 10 less the cylinder r = 3 through 10 mm: 15000 - 9π·10.
        assert_volume(&r, 15_000.0 - 90.0 * PI);
        let names: Vec<String> = s.solids()[0]
            .body
            .faces()
            .into_iter()
            .filter_map(|f| {
                s.solids()[0]
                    .body
                    .face_name(f)
                    .ok()
                    .flatten()
                    .map(str::to_owned)
            })
            .collect();
        assert!(names.contains(&"3:hole 0 wall".to_owned()), "{names:?}");
        // The plate gets thicker: the hole still goes through the whole 16 mm.
        s.execute("param.set", &json!({ "name": "t", "expression": "16 mm" }))
            .unwrap();
        assert!(s.feature_errors().is_empty(), "{:?}", s.feature_errors());
        let v = s.solids()[0].body.volume().unwrap();
        let want = 50.0 * 30.0 * 16.0 - 9.0 * PI * 16.0;
        assert!((v - want).abs() < 1e-6, "{v} vs {want}");
        // Undo puts the 10 mm plate back, still drilled.
        s.execute("edit.undo", &json!({})).unwrap();
        let v = s.solids()[0].body.volume().unwrap();
        assert!((v - (15_000.0 - 90.0 * PI)).abs() < 1e-6, "{v}");
    }

    #[test]
    fn a_blind_flat_hole_removes_a_short_cylinder() {
        let mut s = Session::new();
        plate(&mut s);
        let r = hole(&mut s, json!({ "depth": 4, "tip_angle": "flat" })).unwrap();
        // r = 3, depth 4: 9π·4 = 36π.
        assert_volume(&r, 15_000.0 - 36.0 * PI);
    }

    #[test]
    fn a_drill_point_adds_its_cone() {
        let mut s = Session::new();
        plate(&mut s);
        let r = hole(&mut s, json!({ "depth": 4 })).unwrap();
        // The default 118° point is a cone of radius 3 and height 3 / tan(59°) below the
        // 4 mm cylinder: volume π r² h / 3 = 3π h.
        let h = 3.0 / 59f64.to_radians().tan();
        assert_volume(&r, 15_000.0 - 36.0 * PI - 3.0 * PI * h);
        let body = &s.solids()[0].body;
        let tip = body
            .faces()
            .into_iter()
            .find(|f| body.face_name(*f).ok().flatten() == Some("3:hole 0 tip"));
        assert!(tip.is_some());
    }

    #[test]
    fn a_counterbore_removes_both_cylinders() {
        let mut s = Session::new();
        plate(&mut s);
        let r = hole(
            &mut s,
            json!({ "kind": "counterbore", "through_all": true, "cb_diameter": 10, "cb_depth": 3 }),
        )
        .unwrap();
        // d 6 below the counterbore (7 mm) and d 10 for 3 mm: π(9·7 + 25·3).
        assert_volume(&r, 15_000.0 - PI * (9.0 * 7.0 + 25.0 * 3.0));
    }

    #[test]
    fn a_countersink_removes_its_cone_frustum() {
        let mut s = Session::new();
        plate(&mut s);
        let r = hole(
            &mut s,
            json!({ "kind": "countersink", "through_all": true, "cs_diameter": 12, "cs_angle": 90 }),
        )
        .unwrap();
        // A 90° countersink from r 6 down to r 3 is (6 - 3) / tan 45° = 3 deep; its
        // frustum is π h (R² + R r + r²) / 3 = π (36 + 18 + 9) = 63π, the hole below it
        // 9π·(10 - 3).
        assert_volume(&r, 15_000.0 - PI * 9.0 * 7.0 - 63.0 * PI);
    }

    #[test]
    fn several_points_and_sketch_points_drill_several_holes() {
        let mut s = Session::new();
        plate(&mut s);
        let a = top_point(&s, 10.0, 10.0);
        let b = top_point(&s, 40.0, 20.0);
        let r = s
            .execute(
                "solid.hole",
                &json!({ "face": "2:top", "points": [a, b], "diameter": 4, "through_all": true }),
            )
            .unwrap();
        // Two r = 2 holes through 10 mm: 2 · 4π · 10.
        assert_volume(&r, 15_000.0 - 80.0 * PI);
        // Points of a sketch on the face drill too (the rectangle's corners do not).
        let mut s = Session::new();
        plate(&mut s);
        let sk = s
            .execute(
                "sketch.create_on_face",
                &json!({ "feature": 2, "face": "top" }),
            )
            .unwrap()["sketch"]
            .as_u64()
            .unwrap();
        let p = s.named_face_plane("2:top").unwrap();
        for (x, y) in [(10.0, 10.0), (40.0, 20.0), (25.0, 15.0)] {
            let q = p.to_plane(chamfera_geom::Point3::new(x, y, 10.0));
            s.execute("sketch.point", &json!({ "x": q.x, "y": q.y }))
                .unwrap();
        }
        // A rectangle's corners are not hole points.
        s.execute(
            "sketch.rectangle",
            &json!({ "x1": -2, "y1": -2, "x2": 2, "y2": 2 }),
        )
        .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        let r = s
            .execute(
                "solid.hole",
                &json!({ "face": "2:top", "sketch": sk, "diameter": 4, "through_all": true }),
            )
            .unwrap();
        // Three r = 2 holes through 10 mm.
        assert_volume(&r, 15_000.0 - 120.0 * PI);
    }

    #[test]
    fn a_linear_pattern_of_a_hole_drills_a_row() {
        let mut s = Session::new();
        plate(&mut s);
        let a = top_point(&s, 10.0, 15.0);
        let id = s
            .execute(
                "solid.hole",
                &json!({ "face": "2:top", "points": [a], "diameter": 4, "through_all": true }),
            )
            .unwrap()["feature"]
            .as_u64()
            .unwrap();
        let r = s
            .execute(
                "solid.pattern_linear",
                &json!({ "feature": id, "count": 3, "direction": { "x": 1, "y": 0, "z": 0 }, "spacing": 15 }),
            )
            .unwrap();
        // Holes at x = 10, 25 and 40, each r = 2 through 10 mm.
        assert_volume(&r, 15_000.0 - 120.0 * PI);
    }

    #[test]
    fn editing_a_hole_changes_its_size_and_kind() {
        let mut s = Session::new();
        plate(&mut s);
        let id = hole(&mut s, json!({ "through_all": true })).unwrap()["feature"]
            .as_u64()
            .unwrap();
        let r = s
            .execute(
                "feature.edit",
                &json!({ "feature": id, "diameter": "8 mm" }),
            )
            .unwrap();
        // r = 4 through 10: 160π.
        assert_volume(&r, 15_000.0 - 160.0 * PI);
        let r = s
            .execute(
                "feature.edit",
                &json!({ "feature": id, "depth": 5, "tip_angle": "flat" }),
            )
            .unwrap();
        assert_volume(&r, 15_000.0 - 80.0 * PI);
        let r = s
            .execute(
                "feature.edit",
                &json!({ "feature": id, "kind": "counterbore", "cb_diameter": 12, "cb_depth": 2, "through_all": true }),
            )
            .unwrap();
        // d 12 for 2 mm, d 8 for the other 8: π(36·2 + 16·8).
        assert_volume(&r, 15_000.0 - PI * (72.0 + 128.0));
        // A bad edit is refused and changes nothing.
        assert!(
            s.execute("feature.edit", &json!({ "feature": id, "cb_diameter": 4 }))
                .is_err()
        );
        let v = s.solids()[0].body.volume().unwrap();
        assert!((v - (15_000.0 - PI * 200.0)).abs() < 1e-6);
        // The hole is saved and opened with the design.
        let back = chamfera_doc::Document::from_json(&s.doc.to_json().unwrap()).unwrap();
        assert_eq!(back, s.doc);
        // The plate cannot be deleted while the hole is on its face.
        assert!(
            s.execute("feature.delete", &json!({ "feature": 2 }))
                .is_err()
        );
    }

    /// The name of the flat face of the first solid at height `z` facing up.
    fn face_up_at(s: &Session, z: f64) -> String {
        let b = &s.solids()[0].body;
        b.faces()
            .into_iter()
            .find(|f| {
                crate::solids::face_plane(b, *f)
                    .is_some_and(|p| p.z().z > 1.0 - 1e-12 && (p.origin().z - z).abs() < 1e-9)
            })
            .and_then(|f| b.face_name(f).ok().flatten().map(str::to_owned))
            .unwrap()
    }

    #[test]
    fn a_hole_to_a_face_ends_on_its_plane_and_follows_it() {
        let mut s = Session::new();
        plate(&mut s);
        // A pocket 20 x 10, 4 deep, from the top in the plate's right half.
        let sk = s
            .execute("sketch.create", &json!({ "plane": "xy" }))
            .unwrap()["sketch"]
            .clone();
        s.execute(
            "sketch.rectangle",
            &json!({ "x1": 30, "y1": 5, "x2": 50, "y2": 15 }),
        )
        .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        let pocket = s
            .execute(
                "solid.extrude",
                &json!({ "sketch": sk, "profiles": [{ "x": 40, "y": 10 }], "distance": 4,
                         "direction": "reversed", "start_offset": "t", "operation": "cut" }),
            )
            .unwrap()["feature"]
            .as_u64()
            .unwrap();
        let floor = face_up_at(&s, 6.0);
        // The hole at (12, 15), away from the pocket, down to the pocket floor's plane:
        // 4 deep, flat, r = 3: 15000 - 20·10·4 - 9π·4.
        let mut p = json!({ "face": "2:top", "points": [top_point(&s, 12.0, 15.0)],
                            "diameter": 6, "to": floor });
        let r = s.execute("solid.hole", &p).unwrap();
        assert_volume(&r, 15_000.0 - 800.0 - 36.0 * PI);
        // A deeper pocket takes the hole down with it: 15000 - 1000 - 9π·5.
        s.execute("feature.edit", &json!({ "feature": pocket, "distance": 5 }))
            .unwrap();
        assert!(s.feature_errors().is_empty(), "{:?}", s.feature_errors());
        let v = s.solids()[0].body.volume().unwrap();
        let want = 15_000.0 - 1000.0 - 45.0 * PI;
        assert!((v - want).abs() < 1e-6, "{v} vs {want}");
        // Down to the plate's bottom is the whole 10 mm.
        let id = r["feature"].as_u64().unwrap();
        let r = s
            .execute("feature.edit", &json!({ "feature": id, "to": "2:bottom" }))
            .unwrap();
        assert_volume(&r, 15_000.0 - 1000.0 - 90.0 * PI);
        // Faces not below the hole's face, or not parallel to it, are refused.
        let n = s.doc.features.len();
        for to in ["2:top", "2:side 0", "9:nowhere", "top"] {
            p["to"] = json!(to);
            assert!(s.execute("solid.hole", &p).is_err(), "{to}");
        }
        p["to"] = json!(5);
        assert!(s.execute("solid.hole", &p).is_err());
        p["to"] = json!("2:bottom");
        p["depth"] = json!(3);
        assert!(s.execute("solid.hole", &p).is_err());
        assert_eq!(s.doc.features.len(), n);
    }

    #[test]
    fn bad_holes_are_refused_and_change_nothing() {
        let mut s = Session::new();
        plate(&mut s);
        let off = json!({ "x": 1e4, "y": 1e4 });
        for extra in [
            json!({ "through_all": true, "points": [off] }),
            json!({ "through_all": true, "face": "2:side 0", "points": [{ "x": 1e4, "y": 0 }] }),
            json!({ "through_all": true, "face": "2:nowhere" }),
            json!({ "through_all": true, "face": "nowhere" }),
            json!({ "through_all": true, "diameter": 0 }),
            json!({ "through_all": true, "diameter": "5 deg" }),
            json!({ "depth": -3 }),
            json!({}),
            json!({ "depth": 4, "through_all": true }),
            json!({ "through_all": true, "kind": "counterbore", "cb_diameter": 5, "cb_depth": 2 }),
            json!({ "through_all": true, "kind": "counterbore", "cb_diameter": 10 }),
            json!({ "depth": 3, "kind": "counterbore", "cb_diameter": 10, "cb_depth": 3 }),
            json!({ "through_all": true, "kind": "countersink", "cs_diameter": 12, "cs_angle": 180 }),
            json!({ "depth": 2, "kind": "countersink", "cs_diameter": 12 }),
            json!({ "through_all": true, "kind": "spiral" }),
            json!({ "depth": 4, "tip_angle": 0 }),
            json!({ "depth": 4, "tip_angle": "round" }),
            json!({ "through_all": "yes" }),
            json!({ "through_all": true, "points": [] }),
            json!({ "through_all": true, "points": "all" }),
            json!({ "through_all": true, "points": [{ "x": f64::NAN, "y": 0 }] }),
            json!({ "through_all": true, "sketch": 99 }),
            json!({ "through_all": true, "pick": { "x": 1, "y": 1, "width": 10, "height": 10 } }),
        ] {
            let before = s.doc.clone();
            let r = hole(&mut s, extra.clone());
            assert!(r.is_err(), "{extra} was accepted: {r:?}");
            assert_eq!(s.doc, before, "{extra}");
        }
    }

    #[test]
    fn clicking_the_top_face_places_a_hole_point_there() {
        let mut s = Session::new();
        plate(&mut s);
        let (w, h) = (800.0, 600.0);
        s.execute("view.normal_to", &json!({ "x": 0, "y": 0, "z": 1 }))
            .unwrap();
        s.execute("view.fit", &json!({ "width": w, "height": h }))
            .unwrap();
        // Looking straight down after a fit, the middle of the viewport is the middle of
        // the plate, (25, 15) on its top.
        let r = s
            .execute(
                "solid.hole",
                &json!({
                    "pick": { "x": w / 2.0, "y": h / 2.0, "width": w, "height": h },
                    "points": [{ "x": w / 2.0, "y": h / 2.0, "width": w, "height": h }],
                    "diameter": 6, "depth": 4, "tip_angle": "flat",
                }),
            )
            .unwrap();
        assert_volume(&r, 15_000.0 - 36.0 * PI);
        let p = r["definition"]["points"][0].clone();
        let plane = s.named_face_plane("2:top").unwrap();
        let x = p["x"].as_f64().unwrap();
        let y = p["y"].as_f64().unwrap();
        let world = plane.from_plane(chamfera_geom::Point2::new(x, y));
        // Within a small part of a pixel (the plate spans hundreds of pixels after the fit).
        assert!(
            (world.x - 25.0).abs() < 1e-2 && (world.y - 15.0).abs() < 1e-2,
            "{world:?}"
        );
    }
}
