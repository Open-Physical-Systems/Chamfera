//! Blends of a straight edge whose two faces are ruled along it: planes, cylinders whose
//! axis runs along the edge, and surfaces of linear extrusion (extruded splines) whose
//! extrusion direction runs along it.
//!
//! Every plane perpendicular to the edge cuts such a face in the same curve (a line, a
//! circle or the extruded spline), moved along the edge. A ball of radius `r` rolling
//! along the edge therefore touches each face along a ruling, and its centre runs on a
//! line parallel to the edge: the fillet is the extrusion of the 2D fillet of the two
//! section curves, which is a circular arc, so the blend is an exact
//! [`Cylinder`](crate::Cylinder) (the rolling-ball blend of Vida, Martin and Varady,
//! 1994, section 3, in the special case of a translational sweep). Likewise an
//! equal-distance chamfer cuts both faces along rulings and is a plane.
//!
//! In the section plane through the edge's first end `p`, with `q_F(σ)` the section curve
//! of face `F` and `N_F(σ)` its outward unit normal, the ball centre is
//! `c = q_A(σ_A) - k r N_A(σ_A) = q_B(σ_B) - k r N_B(σ_B)` (`k = +1` on a convex edge,
//! where the ball is inside the material, `-1` on a concave one): two equations in the
//! plane for `σ_A`, `σ_B`. Between two planes they are linear and solved in closed form
//! (the setback `r / tan(β/2)`); otherwise by damped Newton from the tangent-plane
//! estimate (the curvature of the section curves only moves the contact points), and the
//! result is checked: the ball must touch each surface at its foot point (its distance to
//! the surface is `r`). A chamfer's contact point on each face is the point of the
//! section curve at straight distance `d` from the edge, found the same way.

use super::Kind;
use super::build::{face_label, internal, too_large, unsupported};
use super::rim::AXIS_PARALLEL_TOLERANCE;
use crate::{Body, FaceId, KernelResult, Surface};
use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE, Point3, Vec3};

/// Most Newton steps for one contact solve; quadratic convergence needs a handful.
const MAX_NEWTON_STEPS: usize = 64;
/// Most step halvings in one damped Newton step.
const MAX_HALVINGS: usize = 40;
/// Residual (mm) at which a contact solve has converged: far below the model tolerance,
/// so the contact lines lie on their surfaces to rounding.
const SOLVE_TOLERANCE: f64 = LINEAR_TOLERANCE * 1e-4;
/// Length (mm) along a section curve of the finite-difference step for the derivative of
/// its normal.
const DIFFERENCE_STEP: f64 = 1e-6;

/// One face's section by the plane through the edge perpendicular to it.
#[derive(Debug, Clone)]
enum Section {
    /// `p + σ w`, with outward normal `n`.
    Plane { p: Point3, w: Vec3, n: Vec3 },
    /// `S(σ, v)` at the edge's height `v`, outward normal `sense · normal_at`.
    Curved {
        surface: Surface,
        v: f64,
        sense: f64,
    },
}

impl Section {
    fn point(&self, s: f64) -> Point3 {
        match self {
            Section::Plane { p, w, .. } => *p + *w * s,
            Section::Curved { surface, v, .. } => surface.point_at(s, *v),
        }
    }
    fn tangent(&self, s: f64) -> Vec3 {
        match self {
            Section::Plane { w, .. } => *w,
            Section::Curved { surface, v, .. } => surface.derivatives(s, *v).0,
        }
    }
    fn normal(&self, s: f64) -> Vec3 {
        match self {
            Section::Plane { n, .. } => *n,
            Section::Curved { surface, v, sense } => surface.normal_at(s, *v) * *sense,
        }
    }
    fn param(&self, q: Point3) -> f64 {
        match self {
            Section::Plane { p, w, .. } => (q - *p).dot(*w),
            Section::Curved { surface, .. } => surface.parameters(q).u,
        }
    }
    fn distance(&self, q: Point3) -> f64 {
        match self {
            Section::Plane { p, n, .. } => (q - *p).dot(*n).abs(),
            Section::Curved { surface, .. } => surface.distance(q),
        }
    }
    /// A parameter step that moves about `len` mm along the curve at `s`.
    fn step(&self, s: f64, len: f64) -> f64 {
        let speed = self.tangent(s).length();
        if speed > LINEAR_TOLERANCE {
            len / speed
        } else {
            len
        }
    }
}

/// The section of face `f` (outward normal `n` at the edge point `p`) by the plane
/// through `p` perpendicular to `dir`, if the face is ruled along `dir`.
fn section(body: &Body, f: FaceId, p: Point3, dir: Vec3, w: Vec3) -> KernelResult<Option<Section>> {
    let Some((surface, same)) = body.face_surface(f)? else {
        return Err(internal("a face has no surface"));
    };
    let sense = if same { 1.0 } else { -1.0 };
    let along = match &surface {
        Surface::Plane(frame) => {
            return Ok(Some(Section::Plane {
                p,
                w,
                n: frame.z() * sense,
            }));
        }
        Surface::Cylinder(c) => c.frame().z(),
        Surface::Extrusion(e) => e.frame().z(),
        _ => return Ok(None),
    };
    if along.cross(dir).length() > AXIS_PARALLEL_TOLERANCE {
        return Ok(None);
    }
    let v = surface.parameters(p).v;
    Ok(Some(Section::Curved { surface, v, sense }))
}

/// The analysed cross-section of a straight edge: everything as offsets from the edge
/// point `p`, perpendicular to the edge.
#[derive(Debug, Clone, Copy)]
pub(super) struct Ruled {
    /// Outward unit normals of the faces at the edge.
    pub normal: [Vec3; 2],
    /// From the edge to each face's contact line.
    pub contact: [Vec3; 2],
    /// From the edge to the ball centre (fillets).
    pub spine: Vec3,
    pub convex: bool,
    /// Both faces planar.
    pub planar: bool,
}

/// Analyses the straight edge through `p` along `dir` between `faces` (`faces[0]` holds
/// its first half-edge) for a blend of `size`.
pub(super) fn analyse(
    body: &Body,
    label: &str,
    faces: [FaceId; 2],
    p: Point3,
    dir: Vec3,
    kind: Kind,
    size: f64,
) -> KernelResult<Ruled> {
    // Outward normals at p, then the directions into each face: face A (first half-edge)
    // lies on the left of `dir` seen from outside, so into it is n_A × dir; face B's
    // half-edge runs the other way.
    let mut normal = [Vec3::ZERO; 2];
    for (i, f) in faces.iter().enumerate() {
        let Some((surface, same)) = body.face_surface(*f)? else {
            return Err(internal("a face has no surface"));
        };
        let q = surface.parameters(p);
        let n = surface.normal_at(q.u, q.v);
        normal[i] = if same { n } else { -n };
    }
    let unit = |v: Vec3| {
        v.normalized()
            .ok_or_else(|| internal("the direction into a face is undefined"))
    };
    let inward = [unit(normal[0].cross(dir))?, unit(normal[1].cross(-dir))?];
    let mut sections = Vec::with_capacity(2);
    for (i, f) in faces.iter().enumerate() {
        let Some(s) = section(body, *f, p, dir, inward[i])? else {
            return Err(unsupported(format!(
                "{label}: a straight edge can be blended only between planes, cylinders whose axis runs along it and extruded splines extruded along it; {} is not such a face",
                face_label(body, *f)?
            )));
        };
        sections.push(s);
    }
    let [sa, sb] = [&sections[0], &sections[1]];
    // The angle between the faces' inward directions, measured on the side the ball sits
    // (the material for a convex edge, the empty side for a concave one).
    let beta = inward[0].angle_to(inward[1]).unwrap_or(0.0);
    if beta <= ANGULAR_TOLERANCE.sqrt() || std::f64::consts::PI - beta <= ANGULAR_TOLERANCE.sqrt() {
        return Err(unsupported(format!(
            "the faces at {label} meet tangentially or fold back; there is no corner to blend"
        )));
    }
    let convex = inward[0].dot(normal[1]) < 0.0;
    let side = if convex { 1.0 } else { -1.0 };
    let setback = match kind {
        Kind::Chamfer => size,
        Kind::Fillet => size / (0.5 * beta).tan(),
    };
    if !(setback.is_finite() && setback > LINEAR_TOLERANCE) {
        return Err(too_large(
            kind,
            format!("its setback on {label} is not a usable length"),
        ));
    }
    let planar = matches!(sa, Section::Plane { .. }) && matches!(sb, Section::Plane { .. });
    if planar {
        let contact = [inward[0] * setback, inward[1] * setback];
        let spine = contact[0] + normal[0] * (-side * size);
        return Ok(Ruled {
            normal,
            contact,
            spine,
            convex,
            planar,
        });
    }
    let start = [
        sa.param(p + inward[0] * setback),
        sb.param(p + inward[1] * setback),
    ];
    let (contact, spine) = match kind {
        Kind::Fillet => {
            let [ta, tb] =
                fillet_contacts(sa, sb, start, side * size, inward[0], dir).ok_or_else(|| {
                    too_large(
                        kind,
                        format!("no ball of that radius touches both faces at {label}"),
                    )
                })?;
            let (qa, qb) = (sa.point(ta), sb.point(tb));
            let c = qa + sa.normal(ta) * (-side * size);
            for (sec, q) in [(sa, qa), (sb, qb)] {
                if (sec.distance(c) - size).abs() > 1e3 * LINEAR_TOLERANCE
                    || (c.distance(q) - size).abs() > 1e3 * LINEAR_TOLERANCE
                {
                    return Err(too_large(
                        kind,
                        format!("the ball at {label} would reach past a face's section"),
                    ));
                }
            }
            ([qa - p, qb - p], c - p)
        }
        Kind::Chamfer => {
            let mut q = [p; 2];
            for (i, sec) in [sa, sb].iter().enumerate() {
                let t = chord_point(sec, start[i], p, size).ok_or_else(|| {
                    too_large(
                        kind,
                        format!(
                            "no point of {} lies that far from {label}",
                            face_label(body, faces[i]).unwrap_or_default()
                        ),
                    )
                })?;
                q[i] = sec.point(t);
            }
            ([q[0] - p, q[1] - p], Vec3::ZERO)
        }
    };
    for i in 0..2 {
        if contact[i].dot(inward[i]) <= LINEAR_TOLERANCE {
            return Err(too_large(
                kind,
                format!("its contact on {label} would leave the face"),
            ));
        }
    }
    Ok(Ruled {
        normal,
        contact,
        spine,
        convex,
        planar,
    })
}

/// The parameters `(σ_A, σ_B)` with `q_A - k N_A = q_B - k N_B` (`k = ±r`), by damped
/// Newton from `start`, in the section plane spanned by `e1` and `dir × e1`.
fn fillet_contacts(
    a: &Section,
    b: &Section,
    start: [f64; 2],
    k: f64,
    e1: Vec3,
    dir: Vec3,
) -> Option<[f64; 2]> {
    let e2 = dir.cross(e1).normalized()?;
    let centre = |sec: &Section, s: f64| sec.point(s) + sec.normal(s) * (-k);
    let residual = |t: [f64; 2]| {
        let f = centre(a, t[0]) - centre(b, t[1]);
        [f.dot(e1), f.dot(e2)]
    };
    let norm = |f: [f64; 2]| f[0].hypot(f[1]);
    let dcentre = |sec: &Section, s: f64| {
        let h = sec.step(s, DIFFERENCE_STEP * k.abs().max(1.0));
        (centre(sec, s + h) - centre(sec, s - h)) * (0.5 / h)
    };
    let mut t = start;
    let mut f = residual(t);
    for _ in 0..MAX_NEWTON_STEPS {
        if !(f[0].is_finite() && f[1].is_finite()) {
            return None;
        }
        if norm(f) <= SOLVE_TOLERANCE {
            return Some(t);
        }
        let ja = dcentre(a, t[0]);
        let jb = dcentre(b, t[1]) * -1.0;
        let (j11, j21) = (ja.dot(e1), ja.dot(e2));
        let (j12, j22) = (jb.dot(e1), jb.dot(e2));
        let det = j11 * j22 - j12 * j21;
        if !det.is_finite() || det.abs() <= f64::MIN_POSITIVE {
            return None;
        }
        let d0 = (f[0] * j22 - f[1] * j12) / det;
        let d1 = (j11 * f[1] - j21 * f[0]) / det;
        let mut lambda = 1.0;
        let mut improved = false;
        for _ in 0..MAX_HALVINGS {
            let next = [t[0] - lambda * d0, t[1] - lambda * d1];
            let g = residual(next);
            if norm(g) < norm(f) {
                t = next;
                f = g;
                improved = true;
                break;
            }
            lambda *= 0.5;
        }
        if !improved {
            break;
        }
    }
    (norm(f) <= SOLVE_TOLERANCE * 1e2).then_some(t)
}

/// The parameter of the point of `sec` at straight distance `d` from `p`, near `start`,
/// by Newton on `|q(σ) - p|² - d²`.
fn chord_point(sec: &Section, start: f64, p: Point3, d: f64) -> Option<f64> {
    let mut t = start;
    for _ in 0..MAX_NEWTON_STEPS {
        let w = sec.point(t) - p;
        let g = w.dot(w) - d * d;
        if !g.is_finite() {
            return None;
        }
        if g.abs() <= SOLVE_TOLERANCE * d {
            return Some(t);
        }
        let dg = 2.0 * w.dot(sec.tangent(t));
        if !dg.is_finite() || dg.abs() <= f64::MIN_POSITIVE {
            return None;
        }
        t -= g / dg;
    }
    None
}
