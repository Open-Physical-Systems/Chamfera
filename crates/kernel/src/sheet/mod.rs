//! Sheet metal: a plate of constant thickness with cylindrical bends, and its flat pattern.
//!
//! # Model
//! A [`Sheet`] is a folded solid (`folded`, an ordinary valid [`Body`]) together with how
//! it unrolls: its flat pieces (the base plate and each flange's straight part), each with
//! the rigid motion that lays it into the base's plane, and each bend's strip in that
//! plane. Every flange is built from K4 sweeps and K7 booleans:
//!
//! 1. The picked edge is checked: a straight edge between a flat sheet face and a flat
//!    side face exactly one thickness `T` wide, at a right angle ([`Sheet::flange`]).
//! 2. The parent piece is trimmed back by the setback of the bend position (subtract),
//!    and bend reliefs are cut where the flange stops short of the edge's ends.
//! 3. The bend and the flange's straight part are one profile in the plane across the
//!    edge, extruded along the edge: an annular sector of inner radius `R` and outer
//!    radius `R + T` through the bend angle `θ` (exact cylinders), then the straight part
//!    (planes). It is joined to the trimmed sheet (union).
//!
//! # Formulas (standard sheet metal practice)
//! With inner bend radius `R`, thickness `T`, K-factor `K` (where the neutral layer lies
//! across the thickness, 0 at the inside, 1 at the outside) and bend angle `θ` (the angle
//! the flange turns through, 90° for a right-angle flange):
//!
//! - bend allowance `BA = θ (R + K T)`: the length of the neutral layer through the bend,
//!   which keeps its length when the sheet is bent, so it is the bend's length in the
//!   flat (e.g. *Machinery's Handbook*, "Sheet metal: bend allowance"; any sheet metal
//!   design text);
//! - outside setback `OSSB = (R + T) tan(θ / 2)`: from the outer virtual sharp (where the
//!   outer faces of the sheet and the flange, extended, meet) to the bend's tangent lines;
//!   inside setback `R tan(θ / 2)` for the inner faces.
//!
//! The flange height is measured along the flange from the outer virtual sharp, so the
//! straight part of the flange is `height - OSSB` long. The bend position says where the
//! flange sits relative to the picked edge's side face:
//!
//! - [`BendPosition::Inside`]: the outer virtual sharp is on the side face, so the part
//!   keeps its outline (a 100 mm base with a flange is still 100 mm over the flange); the
//!   sheet is trimmed back by `OSSB`;
//! - [`BendPosition::Outside`]: the inner virtual sharp is on the side face; trimmed by
//!   `R tan(θ / 2)`;
//! - [`BendPosition::BendOutside`]: the bend starts at the side face; nothing is trimmed.
//!
//! # Unfolding
//! A point of a flange's straight part unrolls by turning back through `-θ` about the
//! bend's axis and moving along the parent's face by `BA` (the neutral layer laid flat):
//! a rigid motion, composed with the parent's own unrolling, so flanges on flanges unfold
//! too. The bend becomes a flat strip `BA` long. The flat pattern is the union of every
//! piece laid flat (a planar solid of thickness `T`), and its outline is the boundary of
//! its face on the base's sketch plane ([`flat_outline`]); the engine writes it as DXF
//! with the drawing crate's writer.
//!
//! # Names
//! A flange's faces: `bend outer`, `bend inner`, `flange outer`, `flange inner`,
//! `flange end`, and the two side faces `near side` and `far side`; the edges of its end,
//! where a further flange can go, are `end outer` and `end inner`. Relief cuts are
//! `relief start` and `relief end`. The engine prefixes them with the feature's id.

mod flat;
#[cfg(test)]
mod tests;

pub use flat::{FlatEdge, flat_outline};

use crate::topology::{Body, Curve, EdgeId};
use crate::{KernelError, KernelResult, PointInSolid, Profile, ProfileEdge, classify_point};
use chamfera_geom::{
    ANGULAR_TOLERANCE, Frame, LINEAR_TOLERANCE, MAX_COORDINATE, Point2, Point3, Transform, Vec3,
};
use std::f64::consts::{FRAC_PI_2, PI};

/// How far from a right angle (as the cosine between face normals) a sheet face and its
/// side face may be: the faces come from sweeps and rigid motions, so they are square to
/// rounding.
pub const SQUARE_COS_TOLERANCE: f64 = 1e-9;
/// Most flanges one sheet may hold.
pub const MAX_FLANGES: usize = 1_000;

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

/// Where a flange sits relative to the side face of its edge (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BendPosition {
    /// The outer faces meet on the side face: the part keeps its outline.
    #[default]
    Inside,
    /// The inner faces meet on the side face.
    Outside,
    /// The bend starts at the side face.
    BendOutside,
}

/// The shape of the cut that frees a bend where it stops short of the edge's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReliefKind {
    /// A rectangular notch.
    #[default]
    Rectangular,
    /// A notch with a round end (a slot).
    Obround,
    /// No material removed: the sheet is only slit (a rip).
    Rip,
}

/// A bend relief: its shape, width along the edge and how far it reaches past the bend's
/// start (an obround relief's round end lies past that depth).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Relief {
    pub kind: ReliefKind,
    pub width: f64,
    pub depth: f64,
}

/// Everything a flange needs, in mm and radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlangeSpec {
    /// From the outer virtual sharp to the flange's end, along the flange.
    pub height: f64,
    /// The bend angle, above 0 and below 180 degrees.
    pub angle: f64,
    /// The inner bend radius.
    pub radius: f64,
    /// The K-factor, from 0 to 1.
    pub k_factor: f64,
    pub position: BendPosition,
    /// False: the flange rises on the side of the face the edge borders (picking a top
    /// edge bends up); true: the other way.
    pub flip: bool,
    /// How far the flange stops short of the edge's start and end (0: the whole edge).
    pub offset_start: f64,
    pub offset_end: f64,
    pub relief: Relief,
}

/// The bend allowance `θ (R + K T)`: the length of the neutral layer through a bend.
pub fn bend_allowance(angle: f64, radius: f64, k_factor: f64, thickness: f64) -> f64 {
    angle * (radius + k_factor * thickness)
}

/// The outside setback `(R + T) tan(θ / 2)`.
pub fn outside_setback(angle: f64, radius: f64, thickness: f64) -> f64 {
    (radius + thickness) * (angle * 0.5).tan()
}

/// How far the sheet is trimmed back from the edge's side face for a bend position.
pub fn position_trim(position: BendPosition, angle: f64, radius: f64, thickness: f64) -> f64 {
    match position {
        BendPosition::Inside => outside_setback(angle, radius, thickness),
        BendPosition::Outside => radius * (angle * 0.5).tan(),
        BendPosition::BendOutside => 0.0,
    }
}

fn check_length(name: &str, v: f64, zero_ok: bool) -> KernelResult<f64> {
    let ok = v.is_finite()
        && v <= MAX_COORDINATE
        && if zero_ok {
            v >= 0.0
        } else {
            v > LINEAR_TOLERANCE
        };
    if ok {
        Ok(v)
    } else if zero_ok {
        Err(invalid(format!(
            "the {name} must be a length from 0 to {MAX_COORDINATE} mm"
        )))
    } else {
        Err(invalid(format!(
            "the {name} must be a length above 0 and at most {MAX_COORDINATE} mm"
        )))
    }
}

/// Checks a thickness.
pub fn check_thickness(t: f64) -> KernelResult<f64> {
    check_length("thickness", t, false)
}

impl FlangeSpec {
    /// Checks every value (not yet against the edge).
    pub fn check(&self) -> KernelResult<()> {
        check_length("flange height", self.height, false)?;
        check_length("bend radius", self.radius, false)?;
        check_length("start offset", self.offset_start, true)?;
        check_length("end offset", self.offset_end, true)?;
        if !(self.angle.is_finite()
            && self.angle > ANGULAR_TOLERANCE
            && self.angle < PI - ANGULAR_TOLERANCE)
        {
            return Err(invalid(
                "the bend angle must be above 0 and below 180 degrees",
            ));
        }
        if !(self.k_factor.is_finite() && (0.0..=1.0).contains(&self.k_factor)) {
            return Err(invalid("the K-factor must be from 0 to 1"));
        }
        if self.relief.kind != ReliefKind::Rip {
            check_length("relief width", self.relief.width, false)?;
            check_length("relief depth", self.relief.depth, true)?;
        }
        Ok(())
    }
}

/// A flat piece of the sheet: its solid where it lies folded, and how it lies flat.
#[derive(Debug, Clone)]
struct Piece {
    body: Body,
    to_flat: Transform,
}

/// One bend, for reports: its angle, inner radius, allowance and length along the edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BendReport {
    pub angle: f64,
    pub radius: f64,
    pub allowance: f64,
    pub length: f64,
}

/// A sheet metal part: the folded solid and how it unfolds (see the module docs).
#[derive(Debug, Clone)]
pub struct Sheet {
    thickness: f64,
    /// The base's sketch plane: the flat pattern lies on it.
    frame: Frame,
    folded: Body,
    pieces: Vec<Piece>,
    /// Bend strips, already laid flat.
    strips: Vec<Body>,
    bends: Vec<BendReport>,
}

/// A picked edge in the frame of its flange: `u` out of the side face, `w` the way the
/// flange rises, `d = u × w` along the edge; `origin` on the side face's plane, on the
/// sheet's outer face (away from the flange), at the edge's start.
#[derive(Debug, Clone, Copy)]
struct EdgeFrame {
    origin: Point3,
    u: Vec3,
    w: Vec3,
    d: Vec3,
    length: f64,
}

impl EdgeFrame {
    fn at(&self, u: f64, w: f64, d: f64) -> Point3 {
        self.origin + self.u * u + self.w * w + self.d * d
    }

    /// The frame of the plane across the edge at `d` along it: x = u, y = w, normal d.
    fn cross_frame(&self, d: f64) -> KernelResult<Frame> {
        Frame::from_normal_and_x(self.at(0.0, 0.0, d), self.d, self.u)
            .ok_or_else(|| invalid("the edge leaves the working range"))
    }

    /// The frame of the sheet's outer face: x = u, y = d, normal -w (so a profile in it
    /// extruded by a negative distance goes through the sheet toward +w).
    fn face_frame(&self) -> KernelResult<Frame> {
        Frame::from_normal_and_x(self.origin, -self.w, self.u)
            .ok_or_else(|| invalid("the edge leaves the working range"))
    }

    /// A box `u0..u1` by `w0..w1` by `d0..d1` in this frame.
    fn slab(&self, u: (f64, f64), w: (f64, f64), d: (f64, f64)) -> KernelResult<Body> {
        let f = self.cross_frame(d.0)?;
        let profile = Profile::polygon(&[
            Point2::new(u.0, w.0),
            Point2::new(u.1, w.0),
            Point2::new(u.1, w.1),
            Point2::new(u.0, w.1),
        ]);
        crate::extrude(&f, &profile, d.1 - d.0)
    }
}

/// The plane of a face as (outward normal, a point on it), or `None` if it is not flat.
fn face_plane(body: &Body, face: crate::FaceId) -> KernelResult<Option<(Vec3, Point3)>> {
    Ok(match body.face_surface(face)? {
        Some((crate::Surface::Plane(frame), same_sense)) => {
            let n = if same_sense { frame.z() } else { -frame.z() };
            Some((n, frame.origin()))
        }
        _ => None,
    })
}

/// How far a face reaches from the line through `a` in direction `dir`.
fn face_reach(body: &Body, face: crate::FaceId, a: Point3, dir: Vec3) -> KernelResult<f64> {
    let mut reach: f64 = 0.0;
    for lp in body.face_loops(face)? {
        for v in body.loop_vertices(lp)? {
            reach = reach.max((body.vertex_point(v)? - a).dot(dir));
        }
    }
    Ok(reach)
}

/// The edge a flange goes on, checked: straight, between a flat sheet face and a flat side
/// face `thickness` wide, square to each other, with material behind both.
fn edge_frame(body: &Body, edge: EdgeId, thickness: f64, flip: bool) -> KernelResult<EdgeFrame> {
    let not_sheet = |why: &str| {
        invalid(format!(
            "a flange needs a straight edge of the sheet: {why}"
        ))
    };
    let straight = matches!(
        body.edge_curve(edge)?,
        Some((Curve::Line(_) | Curve::Segment(_), _, _))
    );
    if !straight {
        return Err(not_sheet("this edge is curved"));
    }
    let [va, vb] = body.edge_vertices(edge)?;
    let (a, b) = (body.vertex_point(va)?, body.vertex_point(vb)?);
    let [f1, f2] = body.edge_faces(edge)?;
    let (Some((n1, _)), Some((n2, _))) = (face_plane(body, f1)?, face_plane(body, f2)?) else {
        return Err(not_sheet("a face beside it is not flat"));
    };
    if n1.dot(n2).abs() > SQUARE_COS_TOLERANCE {
        return Err(not_sheet("its two faces are not at a right angle"));
    }
    // Each face lies on the far side of the other's normal; the side face is the one that
    // reaches exactly one thickness from the edge.
    let w1 = face_reach(body, f1, a, -n2)?;
    let w2 = face_reach(body, f2, a, -n1)?;
    let is_t = |w: f64| (w - thickness).abs() <= LINEAR_TOLERANCE;
    let (side, sheet) = match (is_t(w1), is_t(w2)) {
        (true, false) => (n1, n2),
        (false, true) => (n2, n1),
        (true, true) => {
            return Err(not_sheet(
                "both faces beside it are one thickness wide, so the side face is ambiguous",
            ));
        }
        (false, false) => {
            return Err(not_sheet(&format!(
                "neither face beside it is a side face {thickness} mm wide"
            )));
        }
    };
    // Material must be behind both faces (a convex edge of the sheet).
    let mid = a + (b - a) * 0.5;
    let inside = mid + side * (-thickness * 0.5) + sheet * (-thickness * 0.5);
    if classify_point(body, inside)? != PointInSolid::Inside {
        return Err(not_sheet("there is no sheet material behind it"));
    }
    let (w, outer) = if flip {
        (-sheet, mid)
    } else {
        (sheet, mid + sheet * (-thickness))
    };
    let d = side.cross(w);
    let d = d
        .normalized()
        .ok_or_else(|| not_sheet("its faces have no clear directions"))?;
    let (ta, tb) = ((a - mid).dot(d), (b - mid).dot(d));
    let start = ta.min(tb);
    let length = (tb - ta).abs();
    Ok(EdgeFrame {
        origin: outer + d * start,
        u: side,
        w,
        d,
        length,
    })
}

/// The bend and the flange's straight part as one profile across the edge (in the
/// [`EdgeFrame::cross_frame`]): the bend about `(ub, T + R)` from the sheet's plane
/// through `θ`, then `flat` mm straight on.
fn bend_profile(ub: f64, t: f64, r: f64, angle: f64, flat: f64) -> Profile {
    let c = Point2::new(ub, t + r);
    let (s, co) = angle.sin_cos();
    let radial = |rho: f64| Point2::new(c.x + rho * s, c.y - rho * co);
    let tangent = chamfera_geom::Vec2::new(co, s);
    let p1 = radial(r + t);
    let p2 = p1 + tangent * flat;
    let p4 = radial(r);
    let p3 = p4 + tangent * flat;
    let p0 = Point2::new(ub, 0.0);
    let p5 = Point2::new(ub, t);
    Profile::new(
        vec![
            ProfileEdge::Arc {
                centre: c,
                radius: r + t,
                start_angle: -FRAC_PI_2,
                sweep: angle,
            },
            ProfileEdge::line(p1, p2),
            ProfileEdge::line(p2, p3),
            ProfileEdge::line(p3, p4),
            ProfileEdge::Arc {
                centre: c,
                radius: r,
                start_angle: angle - FRAC_PI_2,
                sweep: -angle,
            },
            ProfileEdge::line(p5, p0),
        ],
        Vec::new(),
    )
}

/// The flange's straight part alone (for the flat pattern), across the edge.
fn flat_part_profile(ub: f64, t: f64, r: f64, angle: f64, flat: f64) -> Profile {
    let c = Point2::new(ub, t + r);
    let (s, co) = angle.sin_cos();
    let radial = |rho: f64| Point2::new(c.x + rho * s, c.y - rho * co);
    let tangent = chamfera_geom::Vec2::new(co, s);
    let p1 = radial(r + t);
    let p4 = radial(r);
    Profile::polygon(&[p1, p1 + tangent * flat, p4 + tangent * flat, p4])
}

/// Names the faces and edges of an extruded bend profile (see the module docs).
fn name_flange(body: &mut Body) -> KernelResult<()> {
    let faces = [
        ("side 0", "bend outer"),
        ("side 1", "flange outer"),
        ("side 2", "flange end"),
        ("side 3", "flange inner"),
        ("side 4", "bend inner"),
        ("side 5", "bend start"),
        ("bottom", "near side"),
        ("top", "far side"),
    ];
    for f in body.faces() {
        let name = body.face_name(f)?.map(str::to_owned);
        if let Some((_, new)) = faces.iter().find(|(old, _)| Some(*old) == name.as_deref()) {
            body.set_face_name(f, new)?;
        }
    }
    let edges = [("sweep 2", "end outer"), ("sweep 3", "end inner")];
    for e in body.edges() {
        let name = body.edge_name(e)?.map(str::to_owned);
        if let Some((_, new)) = edges.iter().find(|(old, _)| Some(*old) == name.as_deref()) {
            body.set_edge_name(e, new)?;
        }
    }
    Ok(())
}

/// A relief notch at one end of the flange: across `d0..d1` along the edge, from beyond
/// the side face back to `reach` (a negative `u`), through the sheet.
fn relief_tool(
    ef: &EdgeFrame,
    relief: &Relief,
    d: (f64, f64),
    reach: f64,
    t: f64,
    name: &str,
) -> KernelResult<Body> {
    let margin = t.max(1.0);
    let frame = ef.face_frame()?;
    // In the face frame x = u, y = d; it is extruded toward +w through the sheet.
    let width = d.1 - d.0;
    let profile = match relief.kind {
        ReliefKind::Obround if width > 2.0 * LINEAR_TOLERANCE => {
            let rr = width * 0.5;
            let cy = (d.0 + d.1) * 0.5;
            // The straight sides reach `reach`; the round end lies past it, so it stays
            // clear of the bend's cylinders.
            let cx = reach;
            Profile::new(
                vec![
                    ProfileEdge::line(Point2::new(margin, d.0), Point2::new(margin, d.1)),
                    ProfileEdge::line(Point2::new(margin, d.1), Point2::new(cx, d.1)),
                    ProfileEdge::Arc {
                        centre: Point2::new(cx, cy),
                        radius: rr,
                        start_angle: FRAC_PI_2,
                        sweep: PI,
                    },
                    ProfileEdge::line(Point2::new(cx, d.0), Point2::new(margin, d.0)),
                ],
                Vec::new(),
            )
        }
        _ => Profile::polygon(&[
            Point2::new(reach, d.0),
            Point2::new(margin, d.0),
            Point2::new(margin, d.1),
            Point2::new(reach, d.1),
        ]),
    };
    let start = frame
        .with_origin(frame.origin() + ef.w * (t + margin))
        .ok_or_else(|| invalid("the relief leaves the working range"))?;
    let mut tool = crate::extrude(&start, &profile, t + 2.0 * margin)?;
    for f in tool.faces() {
        tool.set_face_name(f, name)?;
    }
    Ok(tool)
}

impl Sheet {
    /// A base flange: `profile` (in `frame`'s plane) extruded by `thickness`, along the
    /// normal or, `reversed`, against it.
    pub fn base(
        frame: &Frame,
        profile: &Profile,
        thickness: f64,
        reversed: bool,
    ) -> KernelResult<Sheet> {
        let t = check_thickness(thickness)?;
        let body = crate::extrude(frame, profile, if reversed { -t } else { t })?;
        Ok(Sheet {
            thickness: t,
            frame: *frame,
            folded: body.clone(),
            pieces: vec![Piece {
                body,
                to_flat: Transform::IDENTITY,
            }],
            strips: Vec::new(),
            bends: Vec::new(),
        })
    }

    pub fn thickness(&self) -> f64 {
        self.thickness
    }

    /// The plane the flat pattern lies on (the base's sketch plane).
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn folded(&self) -> &Body {
        &self.folded
    }

    /// The folded solid, to rename its faces or to swap in a copy that later features
    /// changed (the flat pattern still unfolds the sheet metal features only).
    pub fn folded_mut(&mut self) -> &mut Body {
        &mut self.folded
    }

    pub fn bends(&self) -> &[BendReport] {
        &self.bends
    }

    /// The sheet with a flange on `edge` of its folded solid. The edge must be a straight
    /// edge of the sheet (see the module docs); nothing changes on an error.
    pub fn flange(&self, edge: EdgeId, spec: &FlangeSpec) -> KernelResult<Sheet> {
        spec.check()?;
        if self.bends.len() >= MAX_FLANGES {
            return Err(invalid(format!(
                "a sheet holds at most {MAX_FLANGES} flanges"
            )));
        }
        let t = self.thickness;
        let (r, angle) = (spec.radius, spec.angle);
        let ef = edge_frame(&self.folded, edge, t, spec.flip)?;
        let (d0, d1) = (spec.offset_start, ef.length - spec.offset_end);
        if d1 - d0 <= LINEAR_TOLERANCE {
            return Err(invalid(format!(
                "the offsets ({} and {} mm) leave nothing of the {:.3} mm edge",
                spec.offset_start, spec.offset_end, ef.length
            )));
        }
        let setback = outside_setback(angle, r, t);
        let flat = spec.height - setback;
        if flat <= LINEAR_TOLERANCE {
            return Err(invalid(format!(
                "the flange height must be more than the bend's outside setback, {setback:.4} mm"
            )));
        }
        let trim = position_trim(spec.position, angle, r, t);
        let ub = -trim;
        // The piece the edge belongs to: the one with material just behind its middle.
        let probe = ef.at(-t * 0.5, t * 0.5, ef.length * 0.5);
        let mut owner = None;
        for (i, p) in self.pieces.iter().enumerate() {
            if classify_point(&p.body, probe)? == PointInSolid::Inside {
                owner = Some(i);
                break;
            }
        }
        let owner = owner.ok_or_else(|| {
            invalid("a flange goes on an edge of the base or of a flange's straight part")
        })?;
        let parent = self
            .pieces
            .get(owner)
            .ok_or_else(|| invalid("the sheet lost a piece"))?;
        // There must be sheet left behind the bend.
        let reliefs = spec.relief.kind != ReliefKind::Rip;
        let reach = ub - if reliefs { spec.relief.depth } else { 0.0 };
        let round_end = if spec.relief.kind == ReliefKind::Obround {
            spec.relief.width * 0.5
        } else {
            0.0
        };
        let behind = ef.at(
            reach - round_end - LINEAR_TOLERANCE.max(t * 1e-3),
            t * 0.5,
            (d0 + d1) * 0.5,
        );
        if classify_point(&parent.body, behind)? != PointInSolid::Inside {
            return Err(invalid(format!(
                "the sheet is too short behind this edge for the bend ({trim:.4} mm trimmed)"
            )));
        }
        // Tools that take material away: the trim and the reliefs.
        let margin = t.max(1.0);
        let mut cuts = Vec::new();
        if trim > LINEAR_TOLERANCE {
            cuts.push(ef.slab((ub, margin), (-margin, t + margin), (d0, d1))?);
        }
        if reliefs {
            let tol = LINEAR_TOLERANCE;
            if d0 > tol {
                let w = spec.relief.width.min(d0);
                cuts.push(relief_tool(
                    &ef,
                    &spec.relief,
                    (d0 - w, d0),
                    reach,
                    t,
                    "relief start",
                )?);
            }
            if ef.length - d1 > tol {
                let w = spec.relief.width.min(ef.length - d1);
                cuts.push(relief_tool(
                    &ef,
                    &spec.relief,
                    (d1, d1 + w),
                    reach,
                    t,
                    "relief end",
                )?);
            }
        }
        let mut folded = self.folded.clone();
        let mut piece = parent.body.clone();
        for c in &cuts {
            folded = crate::subtract(&folded, c)?;
            piece = crate::subtract(&piece, c)?;
        }
        let mut flange = crate::extrude(
            &ef.cross_frame(d0)?,
            &bend_profile(ub, t, r, angle, flat),
            d1 - d0,
        )?;
        name_flange(&mut flange)?;
        let folded = crate::union(&folded, &flange).map_err(|e| match e {
            KernelError::Unsupported(why) if round_end > 0.0 => KernelError::Unsupported(format!(
                "an obround relief this close to the bend: {why}; use a rectangular relief or \
                 a rip, or a relief depth over R + T + width / 2 ({:.4} mm)",
                r + t + round_end
            )),
            other => other,
        })?;
        // How the new straight part lies flat: back through the bend, then along by BA.
        let ba = bend_allowance(angle, r, spec.k_factor, t);
        let axis = ef.at(ub, t + r, 0.0);
        let unbend = Transform::rotation_about(axis, ef.d, -angle)
            .and_then(|x| Transform::translation(ef.u * ba).map(|m| x.then(&m)))
            .ok_or_else(|| invalid("the bend leaves the working range"))?;
        let to_flat = unbend.then(&parent.to_flat);
        let part = crate::extrude(
            &ef.cross_frame(d0)?,
            &flat_part_profile(ub, t, r, angle, flat),
            d1 - d0,
        )?;
        let strip = ef
            .slab((ub, ub + ba), (0.0, t), (d0, d1))?
            .transformed(&parent.to_flat)?;
        let mut next = self.clone();
        next.folded = folded;
        if let Some(p) = next.pieces.get_mut(owner) {
            p.body = piece;
        }
        next.pieces.push(Piece {
            body: part,
            to_flat,
        });
        if ba > LINEAR_TOLERANCE {
            next.strips.push(strip);
        }
        next.bends.push(BendReport {
            angle,
            radius: r,
            allowance: ba,
            length: d1 - d0,
        });
        Ok(next)
    }

    /// The flat pattern: every piece and bend strip laid into the base's plane and joined,
    /// a planar solid of the sheet's thickness.
    pub fn flat(&self) -> KernelResult<Body> {
        let mut parts = Vec::with_capacity(self.pieces.len() + self.strips.len());
        for p in &self.pieces {
            parts.push(p.body.transformed(&p.to_flat)?);
        }
        parts.extend(self.strips.iter().cloned());
        let mut iter = parts.into_iter();
        let mut acc = iter
            .next()
            .ok_or_else(|| invalid("the sheet has no pieces"))?;
        for p in iter {
            acc = crate::union(&acc, &p)?;
        }
        Ok(acc)
    }
}
