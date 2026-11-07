//! Cosmetic threads: a thread attribute on a cylindrical face. The solid is not changed;
//! the rebuild works out where the thread runs on the face as built, the viewport draws
//! it as rings one pitch apart, and drawings call it out (`solid.threads`, and
//! `drawing.add_leader` with a `thread`).
//!
//! The thread runs along the cylinder's axis from one end of the face. Which end: the
//! axis is taken pointing up (toward +Z; toward +Y when it lies flat, then +X), and the
//! thread starts at the face's upper end and runs down, so a hole drilled from the top
//! is threaded from its mouth. `flip` starts it at the lower end instead. The face's
//! extent along the axis is measured on its boundary edges. An internal thread (a hole:
//! the face looks toward the axis) or an external one (a boss) is decided by the face's
//! orientation.

use crate::Session;
use crate::naming::Want;
use chamfera_doc::thread::Thread;
use chamfera_doc::{Document, FeatureKind};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point3, Vec3};
use chamfera_kernel::{Body, Surface};
use chamfera_render::{Overlay, OverlayStyle};

/// Points sampled along each boundary edge to find the face's extent along the axis.
const EDGE_SAMPLES: usize = 32;
/// Most rings drawn for one thread (one per pitch, then thinned out).
pub const MAX_THREAD_RINGS: usize = 200;
/// Segments of one drawn ring.
const RING_SEGMENTS: usize = 48;
/// How far (a fraction of the radius) the rings are drawn off the face, toward the
/// viewer's side of it (inside a hole, outside a boss), so they never fight the face for
/// depth.
const RING_LIFT: f64 = 0.01;

/// Where a built thread is, in model coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltThread {
    /// The point on the axis where the thread starts.
    pub start: Point3,
    /// Unit direction along the axis the thread runs in.
    pub direction: Vec3,
    /// The cylinder's radius: the major radius of an external thread, the minor (drilled)
    /// radius of an internal one.
    pub radius: f64,
    pub pitch: f64,
    pub length: f64,
    /// A hole's thread (the face looks toward the axis).
    pub internal: bool,
    /// The drawing callout, such as `M6x1 - 10 deep`.
    pub callout: String,
}

/// A length for a callout: no trailing zeros, at most three decimals.
fn mm(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_owned()
}

/// The axis pointing up: toward +Z, or +Y when it lies flat, then +X.
fn upward(z: Vec3) -> Vec3 {
    let tol = 1e-9;
    let flip = if z.z.abs() > tol {
        z.z < 0.0
    } else if z.y.abs() > tol {
        z.y < 0.0
    } else {
        z.x < 0.0
    };
    if flip { -z } else { z }
}

/// The extent of face `f` of `body` along the line through `o` with direction `d`:
/// the smallest and largest `(p - o) · d` over its boundary.
fn extent_along(
    body: &Body,
    f: chamfera_kernel::FaceId,
    o: Point3,
    d: Vec3,
) -> Result<(f64, f64), String> {
    let k = |e: chamfera_kernel::KernelError| e.to_string();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for lp in body.face_loops(f).map_err(k)? {
        for h in body.loop_half_edges(lp).map_err(k)? {
            let Some(e) = body.half_edge_edge(h).map_err(k)? else {
                continue;
            };
            let Some((curve, t0, t1)) = body.edge_curve(e).map_err(k)? else {
                continue;
            };
            for i in 0..=EDGE_SAMPLES {
                let t = t0 + (t1 - t0) * i as f64 / EDGE_SAMPLES as f64;
                let s = (curve.point_at(t) - o).dot(d);
                if s.is_finite() {
                    lo = lo.min(s);
                    hi = hi.max(s);
                }
            }
        }
    }
    if !(lo.is_finite() && hi.is_finite()) || hi - lo <= LINEAR_TOLERANCE {
        return Err("the face has no length along its axis".into());
    }
    Ok((lo, hi))
}

/// Works out thread `t` of feature `feature` on the bodies built so far.
pub(crate) fn build_thread(
    doc: &Document,
    t: &Thread,
    feature: u32,
    made: &[(u32, String, Body)],
) -> Result<BuiltThread, String> {
    let values = doc.thread_values(t).map_err(|e| e.to_string())?;
    let bodies = crate::naming::bodies(made);
    let name = crate::naming::face_ref_name(&t.face);
    let (_, body, f) = crate::naming::one_face(doc, feature, &bodies, &t.face, Want::Surface)
        .map_err(|e| match e {
            crate::naming::NameError::Gone { .. } => {
                format!("the face {name} the thread is on is gone")
            }
            e => e.to_string(),
        })?;
    let Some((Surface::Cylinder(c), same)) = body.face_surface(f).map_err(|e| e.to_string())?
    else {
        return Err(format!(
            "a thread goes on a cylindrical face; {name} is not cylindrical"
        ));
    };
    let o = c.frame().origin();
    let up = upward(c.frame().z());
    let (lo, hi) = extent_along(body, f, o, up)?;
    let face_length = hi - lo;
    let length = match values.length {
        Some(l) if l > face_length + LINEAR_TOLERANCE => {
            return Err(format!(
                "the thread ({} mm) is longer than the face {name} ({} mm)",
                mm(l),
                mm(face_length)
            ));
        }
        Some(l) => l,
        None => face_length,
    };
    let (start, direction) = if t.flip {
        (o + up * lo, up)
    } else {
        (o + up * hi, -up)
    };
    let radius = c.radius();
    let size = if t.size.trim().is_empty() {
        format!("M{}", mm(2.0 * radius))
    } else {
        t.size.trim().to_owned()
    };
    let depth = if values.length.is_none() {
        "full length".to_owned()
    } else {
        format!("{} deep", mm(length))
    };
    Ok(BuiltThread {
        start,
        direction,
        radius,
        pitch: values.pitch,
        length,
        // The cylinder's own normal points away from the axis; a face that looks the
        // other way is the wall of a hole.
        internal: !same,
        callout: format!("{size}x{} - {depth}", mm(values.pitch)),
    })
}

impl BuiltThread {
    /// The rings drawn for the thread: one per pitch along its length (thinned to at
    /// most [`MAX_THREAD_RINGS`]), just off the face on its open side.
    pub fn rings(&self) -> Vec<[Point3; 2]> {
        let Some(frame) = Frame::from_normal(self.start, self.direction) else {
            return Vec::new();
        };
        let r = self.radius
            * if self.internal {
                1.0 - RING_LIFT
            } else {
                1.0 + RING_LIFT
            };
        let pitches = (self.length / self.pitch).floor();
        let count = if pitches.is_finite() && pitches >= 0.0 {
            (pitches as usize).saturating_add(1)
        } else {
            1
        };
        let n = count.clamp(2, MAX_THREAD_RINGS);
        let step = if count <= MAX_THREAD_RINGS {
            self.pitch
        } else {
            self.length / (n - 1) as f64
        };
        let mut out = Vec::with_capacity(n * RING_SEGMENTS);
        for i in 0..n {
            let along = (step * i as f64).min(self.length);
            let centre = self.start + self.direction * along;
            let at = |k: usize| {
                let a = std::f64::consts::TAU * k as f64 / RING_SEGMENTS as f64;
                centre + (frame.x() * a.cos() + frame.y() * a.sin()) * r
            };
            out.extend((0..RING_SEGMENTS).map(|k| [at(k), at(k + 1)]));
        }
        out
    }
}

impl Session {
    /// The threads built last, by feature id.
    pub fn threads(&self) -> &std::collections::BTreeMap<u32, BuiltThread> {
        &self.built.threads
    }

    /// The viewport rings of every built, unsuppressed thread.
    pub(crate) fn thread_overlays(&self) -> Vec<Overlay> {
        let lines: Vec<[Point3; 2]> = self
            .built
            .threads
            .iter()
            .filter(|(id, _)| {
                self.doc
                    .feature(**id)
                    .is_ok_and(|f| matches!(f.kind, FeatureKind::Thread(_)) && !f.suppressed)
            })
            .flat_map(|(_, t)| t.rings())
            .collect();
        if lines.is_empty() {
            return Vec::new();
        }
        vec![Overlay {
            style: OverlayStyle::Thread,
            lines,
            points: Vec::new(),
            on_top: false,
        }]
    }
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use serde_json::{Value, json};
    use std::f64::consts::PI;

    /// A 50 x 30 plate `t` thick (feature 2) with a Ø6 through hole at (25, 15)
    /// (feature 3, its wall `3:hole 0 wall`).
    fn drilled_plate(s: &mut Session) {
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
        let plane = s.named_face_plane("2:top").unwrap();
        let p = plane.to_plane(chamfera_geom::Point3::new(25.0, 15.0, 10.0));
        s.execute(
            "solid.hole",
            &json!({ "face": "2:top", "points": [{ "x": p.x, "y": p.y }], "diameter": 6,
                     "through_all": true }),
        )
        .unwrap();
    }

    fn close(a: &Value, b: f64) -> bool {
        a.as_f64().is_some_and(|a| (a - b).abs() < 1e-9)
    }

    #[test]
    fn a_thread_in_a_hole_runs_from_its_mouth_and_follows_the_plate() {
        let mut s = Session::new();
        drilled_plate(&mut s);
        let v0 = s.solids()[0].body.volume().unwrap();
        let r = s
            .execute(
                "solid.thread",
                &json!({ "face": "3:hole 0 wall", "pitch": 1, "length": 8 }),
            )
            .unwrap();
        assert_eq!(r["callout"], json!("M6x1 - 8 deep"));
        let id = r["feature"].as_u64().unwrap();
        // Cosmetic: the solid is unchanged, 15000 - 9π·10.
        let v = s.solids()[0].body.volume().unwrap();
        assert!((v - v0).abs() < 1e-9 && (v - (15_000.0 - 90.0 * PI)).abs() < 1e-6);
        let list = s.execute("solid.threads", &json!({})).unwrap();
        let t = &list["threads"][0];
        assert_eq!(t["internal"], json!(true));
        // From the mouth at the top, (25, 15, 10), downward.
        assert!(close(&t["start"][0], 25.0) && close(&t["start"][1], 15.0));
        assert!(close(&t["start"][2], 10.0) && close(&t["direction"][2], -1.0));
        assert!(close(&t["diameter"], 6.0) && close(&t["length"], 8.0));
        // Rings one pitch apart: 0, 1, ..., 8 mm down, 9 of them.
        let built = s.threads().get(&(id as u32)).unwrap().clone();
        assert_eq!(built.rings().len(), 9 * super::RING_SEGMENTS);
        assert!(
            s.scene
                .overlays
                .iter()
                .any(|o| o.style == chamfera_render::OverlayStyle::Thread)
        );
        // The whole face, then a thicker plate: the thread follows it to 16 mm.
        s.execute("feature.edit", &json!({ "feature": id, "length": "full" }))
            .unwrap();
        s.execute("param.set", &json!({ "name": "t", "expression": "16 mm" }))
            .unwrap();
        let t = s.threads().get(&(id as u32)).unwrap();
        assert!((t.length - 16.0).abs() < 1e-9 && (t.start.z - 16.0).abs() < 1e-9);
        assert_eq!(t.callout, "M6x1 - full length");
        // Flipped, it starts at the bottom and runs up.
        s.execute(
            "feature.edit",
            &json!({ "feature": id, "flip": true, "size": "M6", "pitch": "1.25 mm" }),
        )
        .unwrap();
        let t = s.threads().get(&(id as u32)).unwrap();
        assert!(t.start.z.abs() < 1e-9 && (t.direction.z - 1.0).abs() < 1e-12);
        assert_eq!(t.callout, "M6x1.25 - full length");
        // A drawing leader takes the callout as its text.
        s.execute(
            "drawing.add_view",
            &json!({ "view": "top", "x": 150, "y": 200 }),
        )
        .unwrap();
        s.execute(
            "drawing.add_leader",
            &json!({ "points": [{ "x": 150, "y": 200 }, { "x": 180, "y": 230 }], "thread": id }),
        )
        .unwrap();
        let saved = serde_json::to_string(&s.doc.drawing).unwrap();
        assert!(saved.contains("M6x1.25 - full length"), "{saved}");
        // Undo and save/open keep it.
        s.execute("edit.undo", &json!({})).unwrap();
        let json = s.doc.to_json().unwrap();
        let back = chamfera_doc::Document::from_json(&json).unwrap();
        assert_eq!(back, s.doc);
    }

    #[test]
    fn a_thread_on_a_boss_is_external_and_bad_threads_are_refused() {
        let mut s = Session::new();
        s.execute("sketch.create", &json!({ "plane": "xy" }))
            .unwrap();
        s.execute("sketch.circle", &json!({ "cx": 0, "cy": 0, "radius": 5 }))
            .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute(
            "solid.extrude",
            &json!({ "sketch": 1, "profiles": [{ "x": 0, "y": 0 }], "distance": 20 }),
        )
        .unwrap();
        let side = s
            .solids()
            .first()
            .map(|x| {
                x.body
                    .faces()
                    .into_iter()
                    .find(|f| {
                        matches!(
                            x.body.face_surface(*f),
                            Ok(Some((chamfera_kernel::Surface::Cylinder(_), _)))
                        )
                    })
                    .and_then(|f| x.body.face_name(f).ok().flatten().map(str::to_owned))
                    .unwrap()
            })
            .unwrap();
        let r = s
            .execute("solid.thread", &json!({ "face": side, "pitch": "1.5 mm" }))
            .unwrap();
        assert_eq!(r["callout"], json!("M10x1.5 - full length"));
        let t = s.threads().values().next().unwrap();
        assert!(!t.internal && (t.length - 20.0).abs() < 1e-9);
        let n = s.doc.features.len();
        for bad in [
            json!({ "face": "2:top", "pitch": 1 }),
            json!({ "face": side, "pitch": 0 }),
            json!({ "face": side, "pitch": -1 }),
            json!({ "face": side }),
            json!({ "face": side, "pitch": 1, "length": 25 }),
            json!({ "face": side, "pitch": 1, "length": "q" }),
            json!({ "face": side, "pitch": 1, "size": "M6\u{7}" }),
            json!({ "face": side, "pitch": 1, "size": "x".repeat(41) }),
            json!({ "face": side, "pitch": 1, "flip": "yes" }),
            json!({ "face": "9:nothing", "pitch": 1 }),
            json!({ "face": "nothing", "pitch": 1 }),
            json!({ "pitch": 1 }),
        ] {
            assert!(s.execute("solid.thread", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.doc.features.len(), n);
        let id = r["feature"].as_u64().unwrap();
        assert!(
            s.execute("feature.edit", &json!({ "feature": id, "radius": 3 }))
                .is_err()
        );
        assert!(
            s.execute(
                "drawing.add_leader",
                &json!({ "points": [{ "x": 0, "y": 0 }, { "x": 1, "y": 1 }], "thread": 2 })
            )
            .is_err()
        );
    }
}
