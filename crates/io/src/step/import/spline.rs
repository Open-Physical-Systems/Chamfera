//! B-spline curves and surfaces of ISO 10303-42 (4.4.2 `b_spline_curve`, 4.4.36
//! `b_spline_surface` and their subtypes) read into `chamfera_geom` NURBS, and the free-form
//! surfaces mapped to exact kernel surfaces where one exists.
//!
//! # Entities read
//! - `B_SPLINE_CURVE_WITH_KNOTS(name, degree, control_points, form, closed, self_intersect,
//!   knot_multiplicities, knots, knot_spec)`, and the same as a complex instance of
//!   `B_SPLINE_CURVE(degree, control_points, form, closed, self_intersect)` with
//!   `B_SPLINE_CURVE_WITH_KNOTS(multiplicities, knots, spec)` and, when rational,
//!   `RATIONAL_B_SPLINE_CURVE(weights)`.
//! - `BEZIER_CURVE`, `UNIFORM_CURVE`, `QUASI_UNIFORM_CURVE` (simple or complex), whose
//!   knots Part 42 implies: a piecewise Bezier curve has knots `0, 1, .., k` with
//!   multiplicity `p + 1` at the ends and `p` inside; a uniform curve has distinct knots
//!   one apart; a quasi-uniform curve has distinct interior knots one apart and ends of
//!   multiplicity `p + 1`. Knot values only parameterise the curve: the edge's vertices
//!   bound it, so any affine shift of them gives the same edge.
//! - `POLYLINE(name, points)`: the degree-1 B-spline through its points (knots `0, 0, 1,
//!   .., n - 1, n - 1`), the same polygon exactly.
//! - The surface entities likewise: `B_SPLINE_SURFACE_WITH_KNOTS(name, u_degree, v_degree,
//!   control_points, form, u_closed, v_closed, self_intersect, u_multiplicities,
//!   v_multiplicities, u_knots, v_knots, spec)`, `BEZIER_SURFACE`, `UNIFORM_SURFACE`,
//!   `QUASI_UNIFORM_SURFACE` and `RATIONAL_B_SPLINE_SURFACE(weights)`.
//!
//! # Surfaces in the kernel
//! The kernel holds no free-form faces yet, so a B-spline surface is mapped to the exact
//! kernel surface it is, or refused:
//! - every control point on one plane (within [`LINEAR_TOLERANCE`]): the surface lies in
//!   that plane (the convex hull property, The NURBS Book §4.4, holds for positive
//!   weights), so it is a [`Surface::Plane`] whose normal is Part 42's `σ_u × σ_v`;
//! - degree 1 in one direction with two rows of control points, the second row the first
//!   moved by one vector `d`, with equal weights in each pair: then `S(u, t) = C(u) + t d`
//!   exactly (the weights cancel row by row), a surface of linear extrusion. The kernel's
//!   [`Extrusion`] holds the curve projected along `d` onto the plane across it (the
//!   same infinite surface), its normal `C' × d` is Part 42's when `d` runs along `v`
//!   and against it when `d` runs along `u`;
//! - anything else (doubly curved) is refused per face with [`Fail::Unsupported`].
//!
//! `SURFACE_OF_LINEAR_EXTRUSION(name, swept_curve, extrusion_axis)` (4.4.48,
//! `σ(u, v) = C(u) + v V`) maps the same way: a B-spline curve to an [`Extrusion`], a
//! `LINE` to the plane through it spanned with `V`, a `CIRCLE` swept along its own axis to
//! a cylinder.

use super::{Ctx, Fail, R, StepCurve, arg, invalid, list_of, real_of, ref_of};
use crate::step::read::{Instance, Param};
use chamfera_geom::{
    BSplineCurve2, BSplineCurve3, BSplineSurface3, Frame, LINEAR_TOLERANCE, MAX_CONTROL_POINTS,
    MAX_DEGREE, MAX_SURFACE_CONTROL_POINTS, Point2, Point3,
};
use chamfera_kernel::{Extrusion, Surface};

/// The subtype that gives a B-spline's knots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KnotKind {
    WithKnots,
    Bezier,
    Uniform,
    QuasiUniform,
}

const CURVE_KINDS: [(&str, KnotKind); 4] = [
    ("B_SPLINE_CURVE_WITH_KNOTS", KnotKind::WithKnots),
    ("BEZIER_CURVE", KnotKind::Bezier),
    ("UNIFORM_CURVE", KnotKind::Uniform),
    ("QUASI_UNIFORM_CURVE", KnotKind::QuasiUniform),
];

const SURFACE_KINDS: [(&str, KnotKind); 4] = [
    ("B_SPLINE_SURFACE_WITH_KNOTS", KnotKind::WithKnots),
    ("BEZIER_SURFACE", KnotKind::Bezier),
    ("UNIFORM_SURFACE", KnotKind::Uniform),
    ("QUASI_UNIFORM_SURFACE", KnotKind::QuasiUniform),
];

/// True when instance `id` is a B-spline curve (simple or complex) or a polyline.
pub(super) fn is_spline_curve(ctx: &Ctx<'_>, id: u64) -> bool {
    ctx.view.ex.data.get(&id).is_some_and(|i| {
        i.record("B_SPLINE_CURVE").is_some()
            || i.record("POLYLINE").is_some()
            || CURVE_KINDS.iter().any(|(n, _)| i.record(n).is_some())
    })
}

/// True when instance `id` is a B-spline surface (simple or complex).
pub(super) fn is_spline_surface(ctx: &Ctx<'_>, id: u64) -> bool {
    ctx.view.ex.data.get(&id).is_some_and(|i| {
        i.record("B_SPLINE_SURFACE").is_some()
            || SURFACE_KINDS.iter().any(|(n, _)| i.record(n).is_some())
    })
}

/// The attributes of a B-spline entity after its name, split into the `B_SPLINE_*`
/// supertype's (`degree.., control_points, form, closed.., self_intersect`), the knot
/// subtype's and the rational subtype's weights.
struct Parts {
    base: Vec<Param>,
    kind: KnotKind,
    knots: Vec<Param>,
    weights: Option<Param>,
}

/// Splits instance `id` into [`Parts`]: a simple `…_WITH_KNOTS`/`BEZIER_…`/… record (its
/// name first, then the supertype's `base_len` attributes, then the subtype's), or a
/// complex instance with the supertype `base` record, a knot subtype and optionally
/// `rational`.
fn parts(
    inst: &Instance,
    id: u64,
    base: &str,
    base_len: usize,
    kinds: &[(&str, KnotKind)],
    rational: &str,
) -> R<Parts> {
    let kind = kinds
        .iter()
        .find(|(n, _)| inst.record(n).is_some())
        .ok_or_else(|| {
            Fail::Unsupported(vec![format!(
                "#{id} is a {base} without knots (a WITH_KNOTS, BEZIER, UNIFORM or QUASI_UNIFORM form), which is not supported"
            )])
        })?;
    let sub = inst
        .record(kind.0)
        .map(|r| r.params.clone())
        .unwrap_or_default();
    if let [_] = inst.records.as_slice() {
        // Simple instance: (name, base.., subtype..).
        if sub.len() < 1 + base_len {
            return Err(invalid(format!(
                "#{id} ({}) has too few attributes",
                kind.0
            )));
        }
        return Ok(Parts {
            base: sub
                .get(1..1 + base_len)
                .map(<[Param]>::to_vec)
                .unwrap_or_default(),
            kind: kind.1,
            knots: sub
                .get(1 + base_len..)
                .map(<[Param]>::to_vec)
                .unwrap_or_default(),
            weights: None,
        });
    }
    let b = inst
        .record(base)
        .ok_or_else(|| invalid(format!("#{id}: a complex {} lacks its {base} part", kind.0)))?;
    if b.params.len() < base_len {
        return Err(invalid(format!("#{id} ({base}) has too few attributes")));
    }
    let weights = match inst.record(rational) {
        Some(r) => Some(
            r.params
                .first()
                .cloned()
                .ok_or_else(|| invalid(format!("#{id} ({rational}) has no weights")))?,
        ),
        None => None,
    };
    Ok(Parts {
        base: b.params.clone(),
        kind: kind.1,
        knots: sub,
        weights,
    })
}

fn degree_of(p: &Param, id: u64, what: &str) -> R<usize> {
    match p {
        Param::Integer(d) if (1..=MAX_DEGREE as i64).contains(d) => Ok(*d as usize),
        _ => Err(invalid(format!(
            "#{id}: its {what} must be an integer from 1 to {MAX_DEGREE}"
        ))),
    }
}

/// The full knot vector of one direction: from the multiplicities and distinct knots of
/// a `…_WITH_KNOTS` entity, or implied by the other subtypes (see the module
/// documentation). `count` control points, degree `p`.
fn knot_vector(
    kind: KnotKind,
    mults: Option<&Param>,
    knots: Option<&Param>,
    p: usize,
    count: usize,
    id: u64,
) -> R<Vec<f64>> {
    let want = count + p + 1;
    match kind {
        KnotKind::WithKnots => {
            let (Some(m), Some(k)) = (mults, knots) else {
                return Err(invalid(format!("#{id} has no knots")));
            };
            let m = list_of(m, id, "knot multiplicities")?;
            let k = list_of(k, id, "knots")?;
            if m.len() != k.len() {
                return Err(invalid(format!(
                    "#{id} has {} knot multiplicities for {} knots",
                    m.len(),
                    k.len()
                )));
            }
            let mut out = Vec::new();
            for (mi, ki) in m.iter().zip(&k) {
                let times = match mi {
                    Param::Integer(t) if *t >= 1 && (*t as usize) <= want => *t as usize,
                    _ => {
                        return Err(invalid(format!(
                            "#{id}: knot multiplicities must be integers from 1 to {want}"
                        )));
                    }
                };
                let value = real_of(ki, id, "knot")?;
                if out.len() + times > want {
                    return Err(invalid(format!(
                        "#{id}: the multiplicities add up to more than the {want} knots {count} control points of degree {p} need"
                    )));
                }
                out.extend(std::iter::repeat_n(value, times));
            }
            Ok(out)
        }
        KnotKind::Uniform => Ok((0..want).map(|i| i as f64 - p as f64).collect()),
        KnotKind::QuasiUniform => {
            // Ends of multiplicity p + 1, interior knots 1, 2, ..: count - p - 1 of them.
            let inner = count
                .checked_sub(p + 1)
                .ok_or_else(|| invalid(format!("#{id}: too few control points for degree {p}")))?;
            let last = (inner + 1) as f64;
            let mut out = vec![0.0; p + 1];
            out.extend((1..=inner).map(|i| i as f64));
            out.extend(std::iter::repeat_n(last, p + 1));
            Ok(out)
        }
        KnotKind::Bezier => {
            // count = k p + 1 for k pieces.
            let pieces = (count.saturating_sub(1)) / p;
            if pieces == 0 || pieces * p + 1 != count {
                return Err(invalid(format!(
                    "#{id}: a piecewise Bezier form of degree {p} needs k·{p} + 1 control points, not {count}"
                )));
            }
            let mut out = vec![0.0; p + 1];
            for k in 1..pieces {
                out.extend(std::iter::repeat_n(k as f64, p));
            }
            out.extend(std::iter::repeat_n(pieces as f64, p + 1));
            Ok(out)
        }
    }
}

/// Weights as reals in `[MIN_WEIGHT, MAX_WEIGHT]` are checked by the geometry types.
fn reals(p: &Param, id: u64, what: &str) -> R<Vec<f64>> {
    list_of(p, id, what)?
        .iter()
        .map(|x| real_of(x, id, what))
        .collect()
}

impl Ctx<'_> {
    fn points(&self, p: &Param, id: u64, limit: usize) -> R<Vec<Point3>> {
        let list = list_of(p, id, "control points")?;
        if list.len() > limit {
            return Err(Fail::Unsupported(vec![format!(
                "#{id} has {} control points; at most {limit} are supported",
                list.len()
            )]));
        }
        list.iter()
            .map(|q| self.point(ref_of(q, id, "control point")?))
            .collect()
    }

    /// A B-spline curve or polyline entity as a [`BSplineCurve3`] (millimetres).
    pub(super) fn spline_curve(&self, id: u64) -> R<BSplineCurve3> {
        let inst = self
            .view
            .ex
            .data
            .get(&id)
            .ok_or_else(|| invalid(format!("#{id} is not in the file")))?;
        if let Some(r) = inst.record("POLYLINE") {
            let pts = self.points(
                r.params
                    .get(1)
                    .ok_or_else(|| invalid(format!("#{id} has no points")))?,
                id,
                MAX_CONTROL_POINTS,
            )?;
            let n = pts.len();
            if n < 2 {
                return Err(invalid(format!("polyline #{id} needs at least two points")));
            }
            let mut knots = vec![0.0];
            knots.extend((0..n).map(|i| i as f64));
            knots.push((n - 1) as f64);
            return BSplineCurve3::new(1, knots, pts, None)
                .map_err(|e| invalid(format!("polyline #{id}: {e}")));
        }
        let parts = parts(
            inst,
            id,
            "B_SPLINE_CURVE",
            5,
            &CURVE_KINDS,
            "RATIONAL_B_SPLINE_CURVE",
        )?;
        let degree = degree_of(
            parts
                .base
                .first()
                .ok_or_else(|| invalid(format!("#{id} has no degree")))?,
            id,
            "degree",
        )?;
        let pts = self.points(
            parts
                .base
                .get(1)
                .ok_or_else(|| invalid(format!("#{id} has no control points")))?,
            id,
            MAX_CONTROL_POINTS,
        )?;
        let knots = knot_vector(
            parts.kind,
            parts.knots.first(),
            parts.knots.get(1),
            degree,
            pts.len(),
            id,
        )?;
        let weights = parts
            .weights
            .as_ref()
            .map(|w| reals(w, id, "weights"))
            .transpose()?;
        BSplineCurve3::new(degree, knots, pts, weights)
            .map_err(|e| invalid(format!("B-spline curve #{id}: {e}")))
    }

    /// A B-spline surface entity as a [`BSplineSurface3`] (millimetres).
    pub(super) fn spline_surface(&self, id: u64) -> R<BSplineSurface3> {
        let inst = self
            .view
            .ex
            .data
            .get(&id)
            .ok_or_else(|| invalid(format!("#{id} is not in the file")))?;
        let parts = parts(
            inst,
            id,
            "B_SPLINE_SURFACE",
            7,
            &SURFACE_KINDS,
            "RATIONAL_B_SPLINE_SURFACE",
        )?;
        let du = degree_of(
            parts
                .base
                .first()
                .ok_or_else(|| invalid(format!("#{id} has no u_degree")))?,
            id,
            "u_degree",
        )?;
        let dv = degree_of(
            parts
                .base
                .get(1)
                .ok_or_else(|| invalid(format!("#{id} has no v_degree")))?,
            id,
            "v_degree",
        )?;
        let rows = list_of(
            parts
                .base
                .get(2)
                .ok_or_else(|| invalid(format!("#{id} has no control points")))?,
            id,
            "control points",
        )?;
        if rows.len() > MAX_CONTROL_POINTS {
            return Err(Fail::Unsupported(vec![format!(
                "#{id} has {} rows of control points; at most {MAX_CONTROL_POINTS} are supported",
                rows.len()
            )]));
        }
        let mut net = Vec::with_capacity(rows.len());
        let mut total = 0usize;
        for r in &rows {
            let row = self.points(r, id, MAX_CONTROL_POINTS)?;
            total = total.saturating_add(row.len());
            if total > MAX_SURFACE_CONTROL_POINTS {
                return Err(Fail::Unsupported(vec![format!(
                    "#{id} has more than {MAX_SURFACE_CONTROL_POINTS} control points"
                )]));
            }
            net.push(row);
        }
        let (nu, nv) = (net.len(), net.first().map_or(0, Vec::len));
        let ku = knot_vector(
            parts.kind,
            parts.knots.first(),
            parts.knots.get(2),
            du,
            nu,
            id,
        )?;
        let kv = knot_vector(
            parts.kind,
            parts.knots.get(1),
            parts.knots.get(3),
            dv,
            nv,
            id,
        )?;
        let weights = match &parts.weights {
            Some(w) => Some(
                list_of(w, id, "weights")?
                    .iter()
                    .map(|row| reals(row, id, "weights"))
                    .collect::<R<Vec<Vec<f64>>>>()?,
            ),
            None => None,
        };
        BSplineSurface3::new(du, dv, ku, kv, net, weights)
            .map_err(|e| invalid(format!("B-spline surface #{id}: {e}")))
    }

    /// The exact kernel surface of B-spline surface `id` and whether Part 42's normal is
    /// against the kernel's (see the module documentation), or why there is none.
    pub(super) fn spline_face_surface(&self, id: u64, what: &str) -> R<(Surface, bool)> {
        let s = self.spline_surface(id)?;
        if let Some(plane) = plane_of(&s) {
            return Ok((Surface::Plane(plane), false));
        }
        if let Some(r) = extrusion_of(&s) {
            let (frame, curve, flip) =
                r.map_err(|e| invalid(format!("the surface #{id} of {what}: {e}")))?;
            let x = Extrusion::new(frame, curve)
                .map_err(|e| invalid(format!("the surface #{id} of {what}: {e}")))?;
            return Ok((Surface::Extrusion(x), flip));
        }
        let (p, q) = s.degrees();
        let (n, m) = s.counts();
        Err(Fail::Unsupported(vec![format!(
            "the surface of {what} is #{id}, a free-form B-spline surface (degree {p} x {q}, {n} x {m} control points) that is neither a plane nor a linear extrusion; free-form faces come with kernel rung K10 (NURBS surfaces)"
        )]))
    }

    /// `SURFACE_OF_LINEAR_EXTRUSION(name, swept_curve, extrusion_axis)` as a kernel
    /// surface, with the flag (see the module documentation).
    pub(super) fn linear_extrusion(&self, id: u64, what: &str) -> R<(Surface, bool)> {
        let e = self.ent(id)?;
        let curve_id = ref_of(&arg(&e, 1, id)?, id, "swept_curve")?;
        let axis = ref_of(&arg(&e, 2, id)?, id, "extrusion_axis")?;
        drop(e);
        let ve = self.expect(axis, &["VECTOR"], "extrusion axis")?;
        let v = self.direction(ref_of(&arg(&ve, 1, axis)?, axis, "orientation")?)?;
        let magnitude = real_of(&arg(&ve, 2, axis)?, axis, "magnitude")?;
        drop(ve);
        let Some(d) = (if magnitude < 0.0 { v * -1.0 } else { v }).normalized() else {
            return Err(invalid(format!("the extrusion axis #{axis} has no length")));
        };
        let k = |r: chamfera_kernel::KernelResult<Surface>| {
            r.map_err(|x| invalid(format!("the surface #{id} of {what}: {x}")))
        };
        if is_spline_curve(self, curve_id) {
            let c = self.spline_curve(curve_id)?;
            let origin = c.point_at(c.domain().0);
            let frame = Frame::from_normal(origin, d)
                .ok_or_else(|| invalid(format!("the surface #{id}: no frame across its axis")))?;
            let plane = c
                .to_plane(&frame)
                .map_err(|x| invalid(format!("the surface #{id} of {what}: {x}")))?;
            let x = k(Extrusion::new(frame, plane).map(Surface::Extrusion))?;
            return Ok((x, false));
        }
        let ce = self.ent(curve_id)?;
        let name = ce.name.to_string();
        match name.as_str() {
            "LINE" => {
                // The plane through the line spanned by its direction and d; σ_u × σ_v
                // = dir × V.
                let at = self.point(ref_of(&arg(&ce, 1, curve_id)?, curve_id, "pnt")?)?;
                let vec = ref_of(&arg(&ce, 2, curve_id)?, curve_id, "dir")?;
                drop(ce);
                let vecent = self.expect(vec, &["VECTOR"], "line direction")?;
                let dir = self.direction(ref_of(&arg(&vecent, 1, vec)?, vec, "orientation")?)?;
                let n = dir.cross(d);
                let frame = Frame::from_normal_and_x(at, n, dir).ok_or_else(|| {
                    invalid(format!(
                        "the surface #{id} of {what} sweeps a line along itself"
                    ))
                })?;
                Ok((Surface::Plane(frame), false))
            }
            "CIRCLE" => {
                drop(ce);
                let StepCurve::Circle(frame, r) = self.curve(curve_id, what, 1)? else {
                    return Err(invalid(format!("#{curve_id} should be a circle")));
                };
                // A circle swept along its own axis is a cylinder; σ_u × σ_v = C' × V is
                // outward when V is the circle's z (C' runs counter-clockwise about it).
                let along = frame.z().dot(d);
                if (along.abs() - 1.0).abs() > chamfera_geom::ANGULAR_TOLERANCE {
                    return Err(Fail::Unsupported(vec![format!(
                        "the surface of {what} is #{id}, a circle swept obliquely (an elliptic cylinder), which is not supported yet"
                    )]));
                }
                Ok((k(Surface::cylinder(frame, r))?, along < 0.0))
            }
            other => Err(Fail::Unsupported(vec![format!(
                "the surface of {what} is #{id}, a linear extrusion of a {other} (#{curve_id}), which is not supported yet"
            )])),
        }
    }
}

/// The plane a B-spline surface lies in, with Part 42's normal `σ_u × σ_v` as its z, when
/// every control point is on it.
fn plane_of(s: &BSplineSurface3) -> Option<Frame> {
    let (u0, u1) = s.domain_u();
    let (v0, v1) = s.domain_v();
    // The normal at a few interior points: the first that is regular.
    let normal = [
        (0.5, 0.5),
        (0.25, 0.75),
        (0.75, 0.25),
        (0.3, 0.3),
        (0.7, 0.7),
    ]
    .iter()
    .find_map(|(a, b)| {
        let (p, su, sv) = s.derivatives(u0 + a * (u1 - u0), v0 + b * (v1 - v0));
        su.cross(sv).normalized().map(|n| (p, n))
    })?;
    let (origin, n) = normal;
    let flat = s
        .control_points()
        .iter()
        .all(|q| (*q - origin).dot(n).abs() <= LINEAR_TOLERANCE);
    if !flat {
        return None;
    }
    Frame::from_normal(origin, n)
}

/// The kernel extrusion of a B-spline surface that is a linear extrusion (see the module
/// documentation): `None` when it is not one, else the frame (z along the extrusion), the
/// curve in the frame's plane and whether Part 42's normal is against the kernel's.
fn extrusion_of(
    s: &BSplineSurface3,
) -> Option<Result<(Frame, BSplineCurve2, bool), chamfera_geom::NurbsError>> {
    let (p, q) = s.degrees();
    let (n, m) = s.counts();
    // `true`: the ruling runs along v (two columns), else along u (two rows).
    let along_v = if q == 1 && m == 2 && s.knots_v().len() == 4 {
        true
    } else if p == 1 && n == 2 && s.knots_u().len() == 4 {
        false
    } else {
        return None;
    };
    let count = if along_v { n } else { m };
    let at = |k: usize, side: usize| {
        if along_v {
            (s.control_point(k, side), s.weight(k, side))
        } else {
            (s.control_point(side, k), s.weight(side, k))
        }
    };
    let (Some(a0), _) = at(0, 0) else {
        return None;
    };
    let (Some(b0), _) = at(0, 1) else {
        return None;
    };
    let d = b0 - a0;
    let dir = d.normalized()?;
    if d.length() <= LINEAR_TOLERANCE {
        return None;
    }
    let mut pts = Vec::with_capacity(count);
    let mut weights = Vec::with_capacity(count);
    for k in 0..count {
        let ((Some(a), Some(wa)), (Some(b), Some(wb))) = (at(k, 0), at(k, 1)) else {
            return None;
        };
        if ((b - a) - d).length() > LINEAR_TOLERANCE || (wa - wb).abs() > 1e-12 * wa.max(wb) {
            return None;
        }
        pts.push(a);
        weights.push(wa);
    }
    let (degree, knots) = if along_v {
        (p, s.knots_u().to_vec())
    } else {
        (q, s.knots_v().to_vec())
    };
    let frame = Frame::from_normal(a0, dir)?;
    let plane: Vec<Point2> = pts.iter().map(|x| frame.to_plane(*x)).collect();
    Some(BSplineCurve2::new(degree, knots, plane, Some(weights)).map(|c| (frame, c, !along_v)))
}

#[cfg(test)]
mod tests {
    use super::super::{Units, View};
    use super::*;

    /// A biquadratic patch in inches: control points `P_ij = (0.5 i, 0.5 j, 0)` except the
    /// middle one at height 0.4, written as `B_SPLINE_SURFACE_WITH_KNOTS` (#20) and as
    /// `BEZIER_SURFACE` (#21).
    fn dome() -> crate::step::read::Exchange {
        let mut data = String::new();
        for i in 0..3 {
            for j in 0..3 {
                let z = if i == 1 && j == 1 { "0.4" } else { "0." };
                data.push_str(&format!(
                    "#{}=CARTESIAN_POINT('',({},{},{z}));\n",
                    1 + 3 * i + j,
                    0.5 * i as f64,
                    0.5 * j as f64
                ));
            }
        }
        let net = "((#1,#2,#3),(#4,#5,#6),(#7,#8,#9)),.UNSPECIFIED.,.F.,.F.,.F.";
        data.push_str(&format!("#20=B_SPLINE_SURFACE_WITH_KNOTS('',2,2,{net},(3,3),(3,3),(0.,1.),(0.,1.),.UNSPECIFIED.);\n"));
        data.push_str(&format!("#21=BEZIER_SURFACE('',2,2,{net});\n"));
        let text = format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('t','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
        );
        crate::step::read::parse(&text).unwrap()
    }

    #[test]
    fn a_b_spline_patch_read_in_inches_evaluates_to_hand_derived_points() {
        let ex = dome();
        let ctx = Ctx {
            view: View { ex: &ex },
            units: Units {
                length: 25.4,
                angle: 1.0,
            },
        };
        for id in [20, 21] {
            let s = ctx.spline_surface(id).ok().unwrap();
            assert_eq!(s.degrees(), (2, 2));
            // Degree-2 Bernstein weights at t: (1-t)², 2t(1-t), t². The control x and y are
            // linear in i and j (linear precision), so x = u in, y = v in. Only the middle
            // point is raised, by 0.4 in, with weight B1(u) B1(v):
            // (½, ½): B1 = ½ each, z = 0.4 / 4 = 0.1 in = 2.54 mm, x = y = 12.7 mm;
            // (¼, ½): B1(¼) = 2·¼·¾ = 3/8, z = 0.4 · 3/8 · ½ = 0.075 in = 1.905 mm.
            let c = s.point_at(0.5, 0.5);
            assert!(
                c.distance(Point3::new(12.7, 12.7, 2.54)) < 1e-12,
                "#{id}: {c:?}"
            );
            let q = s.point_at(0.25, 0.5);
            assert!(
                q.distance(Point3::new(6.35, 12.7, 1.905)) < 1e-12,
                "#{id}: {q:?}"
            );
            // The corner interpolates its control point: (1 in, 0, 0).
            assert!(s.point_at(1.0, 0.0).distance(Point3::new(25.4, 0.0, 0.0)) < 1e-12);
            // Doubly curved: no exact kernel face for it yet, a clear refusal.
            match ctx.spline_face_surface(id, "face #99") {
                Err(Fail::Unsupported(why)) => {
                    assert!(
                        why[0].contains("free-form") && why[0].contains("face #99"),
                        "{why:?}"
                    );
                }
                _ => panic!("#{id} should be refused as unsupported"),
            }
        }
    }

    #[test]
    fn implied_knot_vectors_follow_part_42() {
        // Piecewise Bezier, degree 2, 5 control points: two pieces, knots 0 0 0 1 1 2 2 2.
        let k = knot_vector(KnotKind::Bezier, None, None, 2, 5, 1)
            .ok()
            .unwrap();
        assert_eq!(k, [0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 2.0]);
        // Quasi-uniform, degree 2, 5 control points: 0 0 0 1 2 3 3 3.
        let k = knot_vector(KnotKind::QuasiUniform, None, None, 2, 5, 1)
            .ok()
            .unwrap();
        assert_eq!(k, [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0]);
        // Uniform, degree 1, 3 control points: 5 distinct knots one apart.
        let k = knot_vector(KnotKind::Uniform, None, None, 1, 3, 1)
            .ok()
            .unwrap();
        assert_eq!(k, [-1.0, 0.0, 1.0, 2.0, 3.0]);
        // A Bezier form with a count that is not k p + 1 is refused.
        assert!(knot_vector(KnotKind::Bezier, None, None, 2, 4, 1).is_err());
        // Multiplicities that overflow are refused before any allocation.
        let m = Param::List(vec![Param::Integer(1_000_000_000)]);
        let v = Param::List(vec![Param::Real(0.0)]);
        assert!(knot_vector(KnotKind::WithKnots, Some(&m), Some(&v), 1, 2, 1).is_err());
    }
}
