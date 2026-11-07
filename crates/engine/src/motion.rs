//! Assembly tools that change what is shown, not the design: driving joints by dragging,
//! playing a motion study, interference between bodies and exploded views.
//!
//! None of this is undo history. A drag or a motion time sets joint values for display
//! only ([`AssemblyView`]); the design's joints keep their own values until a
//! `joint.set_value` writes one. An exploded view moves the shown meshes, never the
//! exact bodies, so measures, exports and interference still see the assembled model.

use crate::Session;
use crate::assembly::{Placed, origin_frame};
use chamfera_doc::{Assembly, JointKind};
use chamfera_geom::{Aabb, Frame, LINEAR_TOLERANCE, Point3, Vec3};
use chamfera_kernel::{Body, KernelError, TessOptions, tessellate};
use chamfera_render::{Fill, FillStyle, Overlay, OverlayStyle};
use std::borrow::Cow;
use std::collections::BTreeMap;

/// Largest explode factor (each component shown this many times as far from the
/// assembly's centre along its axis).
pub const MAX_EXPLODE_FACTOR: f64 = 20.0;
/// Most bodies one interference check compares (so the pairs stay a bounded number).
pub const MAX_INTERFERENCE_BODIES: usize = 500;
/// Overlaps smaller than this (mm³) are touching, not interfering.
pub const MIN_OVERLAP_VOLUME: f64 = 1e-6;

/// Display-only assembly state: what a drag, a motion time and an exploded view show.
#[derive(Debug, Default, Clone)]
pub struct AssemblyView {
    /// Joint values a drag shows, by joint id (radians or mm), over everything else.
    pub drag: BTreeMap<u32, f64>,
    /// The motion study's time being shown, if any: keyed joints take their keys' value.
    pub motion_time: Option<f64>,
    /// The explode factor (1 shows the assembly as it is), if exploded.
    pub explode_factor: Option<f64>,
    /// Offsets added by hand, per component, in mm.
    pub explode_manual: BTreeMap<u32, Vec3>,
    /// The last interference check's overlaps, for drawing.
    pub overlaps: Vec<OverlapShape>,
    /// Construction features hidden in the viewport (`construct.visibility`): a view
    /// setting, so hiding one changes nothing built on it.
    pub hidden_constructs: std::collections::BTreeSet<u32>,
}

/// An overlap as drawn: its triangles and its edges.
#[derive(Debug, Default, Clone)]
pub struct OverlapShape {
    pub triangles: Vec<[Point3; 3]>,
    pub lines: Vec<[Point3; 2]>,
}

/// Where an exploded component is shown: its assembled centre and the offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Exploded {
    pub centre: Point3,
    pub offset: Vec3,
}

/// One pair of overlapping bodies.
#[derive(Debug, Clone)]
pub struct Interference {
    pub a: String,
    pub b: String,
    pub a_component: u32,
    pub b_component: u32,
    pub volume: f64,
    pub overlap: Body,
}

/// What an interference check found.
#[derive(Debug, Default, Clone)]
pub struct InterferenceReport {
    pub pairs: Vec<Interference>,
    /// Bodies compared, and candidate pairs whose boxes overlap.
    pub bodies: usize,
    pub candidates: usize,
    /// Pairs the kernel could not intersect, and why.
    pub errors: Vec<(String, String, String)>,
}

/// The axis (x, y or z) along which `d` is longest, as a unit vector times `d`'s part
/// along it: the auto-explode direction of a component whose centre is `d` from the
/// assembly's centre.
fn along_main_axis(d: Vec3) -> Vec3 {
    let parts = [d.x.abs(), d.y.abs(), d.z.abs()];
    let mut best = 0;
    for i in 1..3 {
        if parts[i] > parts[best] + LINEAR_TOLERANCE {
            best = i;
        }
    }
    match best {
        0 => Vec3::new(d.x, 0.0, 0.0),
        1 => Vec3::new(0.0, d.y, 0.0),
        _ => Vec3::new(0.0, 0.0, d.z),
    }
}

/// Each component's exploded offset, from where its bodies are placed.
///
/// The assembly's principal axes are its own x, y and z. With centre `C` of the box
/// around every component and `c` the centre of one component's box, auto-explode moves
/// that component by `(factor - 1) · d`, where `d` is the part of `c - C` along the axis
/// it is furthest along. A manual offset is added on top.
pub fn explode_offsets(
    placed: &[Placed],
    factor: Option<f64>,
    manual: &BTreeMap<u32, Vec3>,
) -> BTreeMap<u32, Exploded> {
    let mut boxes: BTreeMap<u32, Aabb> = BTreeMap::new();
    for (_, component, _, body) in placed {
        if let Some(b) = body.bounding_box() {
            let merged = match boxes.get(component) {
                Some(o) => o.union(b),
                None => b,
            };
            boxes.insert(*component, merged);
        }
    }
    let all = boxes.values().copied().reduce(Aabb::union);
    let mut out = BTreeMap::new();
    let Some(all) = all else {
        return out;
    };
    let centre = all.center();
    let f = factor.unwrap_or(1.0);
    for (component, b) in boxes {
        let c = b.center();
        let mut offset = along_main_axis(c - centre) * (f - 1.0);
        if let Some(m) = manual.get(&component) {
            offset = offset + *m;
        }
        if offset.length() > LINEAR_TOLERANCE {
            out.insert(component, Exploded { centre: c, offset });
        }
    }
    out
}

/// True when the two boxes share some volume (not just a face, edge or corner).
fn boxes_overlap(a: &Aabb, b: &Aabb) -> bool {
    a.min.x < b.max.x - LINEAR_TOLERANCE
        && b.min.x < a.max.x - LINEAR_TOLERANCE
        && a.min.y < b.max.y - LINEAR_TOLERANCE
        && b.min.y < a.max.y - LINEAR_TOLERANCE
        && a.min.z < b.max.z - LINEAR_TOLERANCE
        && b.min.z < a.max.z - LINEAR_TOLERANCE
}

/// Finds the pairs of `bodies` (name, component, body) that share volume: boxes first,
/// then the exact intersection of each candidate pair.
pub fn interference(bodies: &[(String, u32, &Body)]) -> InterferenceReport {
    let mut report = InterferenceReport {
        bodies: bodies.len(),
        ..InterferenceReport::default()
    };
    let boxes: Vec<Option<Aabb>> = bodies.iter().map(|b| b.2.bounding_box()).collect();
    for i in 0..bodies.len() {
        for j in i + 1..bodies.len() {
            let (Some(Some(bi)), Some(Some(bj))) = (boxes.get(i), boxes.get(j)) else {
                continue;
            };
            let (Some(a), Some(b)) = (bodies.get(i), bodies.get(j)) else {
                continue;
            };
            if !boxes_overlap(bi, bj) {
                continue;
            }
            report.candidates += 1;
            match chamfera_kernel::intersect(a.2, b.2) {
                Ok(overlap) => {
                    let volume = overlap.volume().unwrap_or(0.0);
                    if volume > MIN_OVERLAP_VOLUME {
                        report.pairs.push(Interference {
                            a: a.0.clone(),
                            b: b.0.clone(),
                            a_component: a.1,
                            b_component: b.1,
                            volume,
                            overlap,
                        });
                    }
                }
                Err(KernelError::EmptyResult) => {}
                Err(e) => report
                    .errors
                    .push((a.0.clone(), b.0.clone(), e.to_string())),
            }
        }
    }
    report
}

/// An overlap's triangles and edges, for drawing.
pub fn overlap_shape(body: &Body) -> Option<OverlapShape> {
    let m = tessellate(body, &TessOptions::default()).ok()?;
    let triangles = m
        .mesh
        .triangles
        .iter()
        .filter_map(|t| {
            let p = |i: u32| m.mesh.positions.get(i as usize).copied();
            Some([p(t[0])?, p(t[1])?, p(t[2])?])
        })
        .collect();
    let lines = m
        .edge_polylines
        .iter()
        .flat_map(|(_, l)| l.windows(2).map(|w| [w[0], w[1]]).collect::<Vec<_>>())
        .collect();
    Some(OverlapShape { triangles, lines })
}

/// The signed angle from screen vector `a` to `b` (radians, counter-clockwise on screen
/// positive, with y pointing down the screen).
fn screen_angle(a: (f64, f64), b: (f64, f64)) -> Option<f64> {
    let (ax, ay) = (a.0, -a.1);
    let (bx, by) = (b.0, -b.1);
    if ax.hypot(ay) < 1.0 || bx.hypot(by) < 1.0 {
        return None;
    }
    Some((ax * by - ay * bx).atan2(ax * bx + ay * by))
}

impl Session {
    /// The assembly as it is shown: the design's joints with the motion study's and a
    /// drag's values put in.
    pub(crate) fn shown_assembly(&self) -> Cow<'_, Assembly> {
        let v = &self.view_state;
        if v.drag.is_empty() && (v.motion_time.is_none() || self.doc.motion.is_empty()) {
            return Cow::Borrowed(&self.doc.assembly);
        }
        let mut asm = self.doc.assembly.clone();
        for j in &mut asm.joints {
            if let Some(t) = v.motion_time
                && let Some(value) = self.doc.motion.value_at(j.id, t)
            {
                j.value = value;
            }
            if let Some(value) = v.drag.get(&j.id) {
                j.value = *value;
            }
        }
        Cow::Owned(asm)
    }

    /// The display-only assembly state.
    pub fn assembly_view(&self) -> &AssemblyView {
        &self.view_state
    }

    /// A joint's value as shown now (radians or mm).
    pub fn shown_joint_value(&self, joint: u32) -> Option<f64> {
        self.shown_assembly().joint(joint).ok().map(|j| j.value)
    }

    /// Each exploded component's assembled centre and offset (empty when not exploded).
    pub fn exploded(&self) -> &BTreeMap<u32, Exploded> {
        &self.built.exploded
    }

    /// The bodies an interference check compares: every built solid, or those of the
    /// given components or with the given names.
    pub fn interference_bodies(
        &self,
        components: Option<&[u32]>,
        names: Option<&[String]>,
    ) -> Vec<(String, u32, &Body)> {
        self.solids()
            .iter()
            .filter(|s| components.is_none_or(|c| c.contains(&s.component)))
            .filter(|s| names.is_none_or(|n| n.contains(&s.name)))
            .map(|s| (s.name.clone(), s.component, &s.body))
            .collect()
    }

    /// The joint origin frame on component A of `joint`, in the world, as placed now.
    fn joint_world_frame(&self, joint: u32) -> Option<(Frame, bool)> {
        let asm = self.shown_assembly();
        let j = asm.joint(joint).ok()?;
        let a = j.a.component;
        let frame = match &j.a.at {
            chamfera_doc::OriginAt::Point { point, axis } => {
                let t = self.placements().of(a);
                Frame::from_normal(t.apply_point(*point), t.apply_vector(*axis))?
            }
            at => origin_frame(
                at,
                self.solids()
                    .iter()
                    .filter(move |s| s.component == a)
                    .map(|s| &s.body),
            )
            .ok()?,
        };
        // When A is the side that moves, a drag turns it the other way relative to B.
        let forward = asm
            .driver_of(j.b.component)
            .is_some_and(|d| d.joint == joint);
        Some((frame, forward))
    }

    /// A world point as a pixel of a `w` x `h` viewport.
    pub(crate) fn world_to_screen(&self, p: Point3, w: f64, h: f64) -> Option<(f64, f64)> {
        use chamfera_render::{mat_mul, transform_point};
        let bounds = self.scene.bounds();
        let vp = mat_mul(
            &self.camera.projection_matrix(w / h, bounds.as_ref()),
            &self.camera.view_matrix(),
        );
        let ndc = transform_point(&vp, p)?;
        Some(((ndc[0] + 1.0) * 0.5 * w, (1.0 - ndc[1]) * 0.5 * h))
    }

    /// How much a drag from pixel `from` to `to` changes a joint's value: for a turning
    /// joint, the angle swept about its axis as seen on screen; for a slider, the
    /// distance along its axis that the drag's part along the axis's screen direction
    /// stands for.
    pub fn joint_drag_delta(
        &self,
        joint: u32,
        from: (f64, f64),
        to: (f64, f64),
        w: f64,
        h: f64,
    ) -> Option<f64> {
        let kind = self.doc.assembly.joint(joint).ok()?.kind;
        let (frame, forward) = self.joint_world_frame(joint)?;
        let sign = if forward { 1.0 } else { -1.0 };
        let o = self.world_to_screen(frame.origin(), w, h)?;
        match kind {
            JointKind::Rigid => None,
            JointKind::Slider => {
                // Screen pixels per mm along the axis, from a 1 mm step.
                let tip = self.world_to_screen(frame.origin() + frame.z(), w, h)?;
                let (dx, dy) = (tip.0 - o.0, tip.1 - o.1);
                let len2 = dx * dx + dy * dy;
                if len2 < 1e-12 {
                    return None;
                }
                let along = ((to.0 - from.0) * dx + (to.1 - from.1) * dy) / len2;
                Some(sign * along)
            }
            JointKind::Revolute | JointKind::Cylindrical | JointKind::Planar => {
                let swept = screen_angle((from.0 - o.0, from.1 - o.1), (to.0 - o.0, to.1 - o.1))?;
                // A counter-clockwise turn on screen is a positive turn about an axis that
                // points at the viewer.
                let toward_viewer = -self.camera.basis().forward;
                let facing = if frame.z().dot(toward_viewer) >= 0.0 {
                    1.0
                } else {
                    -1.0
                };
                Some(sign * facing * swept)
            }
        }
    }

    /// Overlays and fills for the exploded view's trails and the interference overlaps.
    pub(crate) fn assembly_overlays(&self) -> (Vec<Overlay>, Vec<Fill>) {
        let mut overlays = Vec::new();
        let mut fills = Vec::new();
        let exploded = self.exploded();
        if !exploded.is_empty() {
            let lines = exploded
                .values()
                .map(|e| [e.centre, e.centre + e.offset])
                .collect();
            let points = exploded.values().map(|e| e.centre).collect();
            overlays.push(Overlay {
                style: OverlayStyle::ExplodeTrail,
                lines,
                points,
                on_top: true,
            });
        }
        let overlaps = &self.view_state.overlaps;
        if !overlaps.is_empty() {
            fills.push(Fill {
                style: FillStyle::Interference,
                triangles: overlaps.iter().flat_map(|o| o.triangles.clone()).collect(),
                on_top: true,
            });
            overlays.push(Overlay {
                style: OverlayStyle::Interference,
                lines: overlaps.iter().flat_map(|o| o.lines.clone()).collect(),
                points: Vec::new(),
                on_top: true,
            });
        }
        (overlays, fills)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_component_explodes_along_the_axis_it_is_furthest_along() {
        // (10, -3, 2) is furthest along x: only its x part is kept.
        let d = along_main_axis(Vec3::new(10.0, -3.0, 2.0));
        assert_eq!((d.x, d.y, d.z), (10.0, 0.0, 0.0));
        let d = along_main_axis(Vec3::new(1.0, 0.0, -4.0));
        assert_eq!((d.x, d.y, d.z), (0.0, 0.0, -4.0));
    }

    #[test]
    fn a_quarter_turn_on_screen_is_a_quarter_of_pi() {
        // From pointing right to pointing up the screen (y down): +90°.
        let a = screen_angle((10.0, 0.0), (0.0, -10.0)).unwrap();
        assert!((a - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }
}
