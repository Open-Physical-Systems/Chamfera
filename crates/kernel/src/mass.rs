//! Mass properties of a solid from its exact boundary: volume, centre of mass and the
//! inertia tensor about the centre of mass, with its principal moments and axes.
//!
//! # Method
//! Mirtich, "Fast and accurate computation of polyhedral mass properties" (Journal of
//! Graphics Tools 1(2), 1996) reduces each volume integral to a surface integral by the
//! divergence theorem, `∭_V div G dV = ∯_∂V G · n dA`, with a field `G` chosen per
//! integrand. Mirtich then sums over polygons; here the faces are exact trimmed analytic
//! surfaces, so the surface integrals are taken on the surfaces themselves. With
//! `q = p - c` (`c` the centre of the body's bounding box, which keeps the sums well
//! conditioned) the ten integrals and their fields are:
//!
//! | `∭ … dV` | `G` |
//! |---|---|
//! | `1` | `q / 3` |
//! | `q_x` (and `y`, `z` alike) | `(q_x² / 2, 0, 0)` |
//! | `q_x²` | `(q_x³ / 3, 0, 0)` |
//! | `q_x q_y`, `q_y q_z`, `q_z q_x` | `(q_x² q_y / 2, 0, 0)`, `(0, q_y² q_z / 2, 0)`, `(0, 0, q_z² q_x / 2)` |
//!
//! # A face integral as a boundary integral in `(u, v)`
//! On a face of a surface `S(u, v)` (plane, cylinder, cone, sphere, torus; `surface.rs`)
//! with domain `D`, `∬ G · n dA = σ ∬_D f du dv` with `f = G(S) · (S_u × S_v)` and
//! `σ = ±1` the face's orientation. Green's theorem turns this into `∮_∂D -F du` with
//! `F(u, v) = ∫_{v*}^{v} f(u, s) ds`, the same way `topology::measure` does for area and
//! volume: the loop images (`uv.rs`) run counter-clockwise in `(u, v)` exactly when
//! `σ = +1`, so the boundary integral is the face's flux `∬ G · n dA` itself, whatever
//! `σ` is. Edges with constant `u` add nothing (`du = 0`); edges with constant `v` (circles
//! about the axis, steps along a pole) add `-∫ F(u, v₀) du`; any other edge adds
//! `-∫ F(u(t), v(t)) u'(t) dt` along its curve.
//!
//! `F` is evaluated by Gauss-Legendre quadrature in `v` (Abramowitz and Stegun, *Handbook
//! of Mathematical Functions*, 25.4.29): on a plane, cylinder and cone `f` is a polynomial
//! of degree at most 4 in `v`, so one 5-point panel is exact; on a sphere and a torus `f`
//! is a trigonometric polynomial and panels of at most π/16 leave an error below 1e-15 of
//! the integral. The boundary integrals use adaptive 5-point Gauss-Legendre quadrature to
//! [`MASS_QUADRATURE_TOLERANCE`].
//!
//! Voids (shells with inward faces) subtract themselves, as they do from the volume.
//!
//! # From the integrals to the tensor
//! With `V`, `s = ∭ q` and `S_ij = ∭ q_i q_j`, the centre of mass is `c + s / V` and the
//! central second moments are `C_ij = S_ij - s_i s_j / V` (the parallel-axis theorem).
//! For density `ρ` the inertia tensor about the centre of mass is
//! `I_xx = ρ (C_yy + C_zz)`, `I_xy = -ρ C_xy` and so on (Goldstein, *Classical Mechanics*,
//! section 5.3). Its eigenvalues are the principal moments, by the cyclic Jacobi method
//! (Golub and Van Loan, *Matrix Computations*, section 8.5).

use crate::topology::{EdgeImage, LoopImage};
use crate::{Body, FaceId, KernelError, KernelResult, Surface};
use chamfera_geom::{LINEAR_TOLERANCE, Point3, Vec3};
use std::f64::consts::PI;

/// Relative accuracy of the boundary integrals (per group of integrals of one degree).
pub const MASS_QUADRATURE_TOLERANCE: f64 = 1e-12;
/// Largest density accepted (g/mm³ would be absurd beyond this; units are the caller's).
pub const MAX_DENSITY: f64 = 1e12;
/// Most quadrature panels one edge integral may use (hostile-input limit).
const MAX_PANELS: usize = 20_000;
/// Most Gauss panels for `F` along `v` on a sphere or torus.
const MAX_V_PANELS: usize = 512;
/// Widest Gauss panel along an angular `v` (sphere latitude, torus tube angle).
const V_PANEL: f64 = PI / 16.0;
/// Two image coordinates within this are the same (as in `topology::measure`).
const STRAIGHT_IMAGE_TOLERANCE: f64 = 1e-9;

/// The ten volume integrals: `V`, `∭ q_x, q_y, q_z`, `∭ q_x², q_y², q_z²`,
/// `∭ q_x q_y, q_y q_z, q_z q_x`.
type Moments = [f64; 10];

/// 5-point Gauss-Legendre nodes and weights on `[-1, 1]` (Abramowitz and Stegun, table
/// 25.4).
const GAUSS5: [(f64, f64); 5] = [
    (0.0, 0.568_888_888_888_888_9),
    (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
    (0.906_179_845_938_664, 0.236_926_885_056_189_1),
];

/// Mass properties of a solid (see the module documentation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassProperties {
    pub density: f64,
    pub volume: f64,
    pub area: f64,
    /// `density * volume`.
    pub mass: f64,
    pub centre_of_mass: Point3,
    /// The inertia tensor about the centre of mass in world axes, by rows:
    /// `[[I_xx, I_xy, I_xz], [I_xy, I_yy, I_yz], [I_xz, I_yz, I_zz]]` with the products
    /// of inertia `I_xy = -∭ ρ x y dV` (relative to the centre of mass).
    pub inertia: [[f64; 3]; 3],
    /// The principal moments, ascending.
    pub principal_moments: [f64; 3],
    /// Unit principal axes, one per moment (right-handed).
    pub principal_axes: [Vec3; 3],
}

fn add(a: &mut Moments, b: &Moments, k: f64) {
    for (x, y) in a.iter_mut().zip(b) {
        *x += k * y;
    }
}

/// `f = G(S(u, v)) · (S_u × S_v)` for the ten fields, `c` the reference point.
fn density(surface: &Surface, c: Point3, u: f64, v: f64) -> Moments {
    let q = surface.point_at(u, v) - c;
    let (su, sv) = surface.derivatives(u, v);
    let n = su.cross(sv);
    let (x, y, z) = (q.x, q.y, q.z);
    [
        q.dot(n) / 3.0,
        x * x * 0.5 * n.x,
        y * y * 0.5 * n.y,
        z * z * 0.5 * n.z,
        x * x * x / 3.0 * n.x,
        y * y * y / 3.0 * n.y,
        z * z * z / 3.0 * n.z,
        x * x * y * 0.5 * n.x,
        y * y * z * 0.5 * n.y,
        z * z * x * 0.5 * n.z,
    ]
}

/// What a face's integrand needs: its surface, the reference point and the base `v*`.
struct FaceIntegrand {
    surface: Surface,
    c: Point3,
    v_ref: f64,
    angular_v: bool,
}

impl FaceIntegrand {
    /// `F(u, v) = ∫_{v*}^{v} f(u, s) ds`.
    fn potential(&self, u: f64, v: f64) -> KernelResult<Moments> {
        let mut out = [0.0; 10];
        let span = v - self.v_ref;
        if !span.is_finite() {
            return Err(KernelError::InvalidInput(
                "a face's parameters are not finite".into(),
            ));
        }
        if span == 0.0 {
            return Ok(out);
        }
        let panels = if self.angular_v {
            let n = (span.abs() / V_PANEL).ceil() as usize;
            if n > MAX_V_PANELS {
                return Err(KernelError::InvalidInput(
                    "a face goes around its surface too many times".into(),
                ));
            }
            n.max(1)
        } else {
            1
        };
        let h = span / panels as f64;
        for i in 0..panels {
            let a = self.v_ref + h * i as f64;
            let m = a + 0.5 * h;
            for (x, w) in GAUSS5 {
                let f = density(&self.surface, self.c, u, m + 0.5 * h * x);
                add(&mut out, &f, 0.5 * h * w);
            }
        }
        Ok(out)
    }
}

fn gauss5(f: &dyn Fn(f64) -> KernelResult<Moments>, a: f64, b: f64) -> KernelResult<Moments> {
    let (m, h) = (0.5 * (a + b), 0.5 * (b - a));
    let mut s = [0.0; 10];
    for (x, w) in GAUSS5 {
        add(&mut s, &f(m + h * x)?, w * h);
    }
    Ok(s)
}

/// The groups of integrals with the same units: volume, first moments, second moments.
const GROUPS: [std::ops::Range<usize>; 3] = [0..1, 1..4, 4..10];

/// `∫_a^b f` for the ten integrals by adaptive Gauss-Legendre quadrature: a panel is
/// accepted when its estimate and the sum over its halves agree, in every integral, to
/// [`MASS_QUADRATURE_TOLERANCE`] of the integrals' scale (the largest in its group, so an
/// integral that is zero by symmetry does not demand an absolute accuracy of zero).
fn integrate(f: &dyn Fn(f64) -> KernelResult<Moments>, a: f64, b: f64) -> KernelResult<Moments> {
    if a == b {
        return Ok([0.0; 10]);
    }
    let width = b - a;
    let mut scale = [0.0f64; 10];
    for i in 0..16 {
        let (x0, x1) = (
            a + width * i as f64 / 16.0,
            a + width * (i + 1) as f64 / 16.0,
        );
        let s = gauss5(
            &|t| {
                let mut v = f(t)?;
                for x in &mut v {
                    *x = x.abs();
                }
                Ok(v)
            },
            x0,
            x1,
        )?;
        for (k, x) in scale.iter_mut().zip(s) {
            *k += x.abs();
        }
    }
    let mut tol = [0.0f64; 10];
    for g in GROUPS {
        let top = scale
            .get(g.clone())
            .map_or(0.0, |s| s.iter().fold(0.0f64, |m, x| m.max(*x)));
        for k in g {
            if let (Some(t), Some(s)) = (tol.get_mut(k), scale.get(k)) {
                *t = MASS_QUADRATURE_TOLERANCE * s.max(1e-6 * top).max(f64::MIN_POSITIVE);
            }
        }
    }
    // Start from the 16 panels: one panel over a whole turn can agree with its halves by
    // symmetry while both are wrong (cos⁴ u over [0, 2π] fools a single panel).
    let mut stack = Vec::with_capacity(32);
    for i in 0..16 {
        let (x0, x1) = (
            a + width * i as f64 / 16.0,
            a + width * (i + 1) as f64 / 16.0,
        );
        stack.push((x0, x1, gauss5(f, x0, x1)?));
    }
    let mut total = [0.0; 10];
    let mut panels = 0usize;
    while let Some((x0, x1, whole)) = stack.pop() {
        panels += 1;
        if panels > MAX_PANELS {
            return Err(KernelError::NoConvergence(
                "a mass integral along an edge needs more quadrature panels than allowed".into(),
            ));
        }
        let m = 0.5 * (x0 + x1);
        let (l, r) = (gauss5(f, x0, m)?, gauss5(f, m, x1)?);
        let share = ((x1 - x0) / width).abs();
        let good = (0..10).all(|k| {
            let (lk, rk, wk, tk) = (l[k], r[k], whole[k], tol[k]);
            (lk + rk - wk).abs() <= tk * share
        });
        if good || m <= x0 || m >= x1 {
            add(&mut total, &l, 1.0);
            add(&mut total, &r, 1.0);
        } else {
            stack.push((x0, m, l));
            stack.push((m, x1, r));
        }
    }
    Ok(total)
}

/// `du/dt` along a curve with point `p` and tangent `d` (world), on `surface`.
fn du_dt(surface: &Surface, p: Point3, d: Vec3) -> Option<f64> {
    let frame = surface.frame();
    let d = frame.vector_to_local(d);
    if let Surface::Plane(_) = surface {
        return Some(d.x);
    }
    let l = frame.to_local(p);
    let r2 = l.x * l.x + l.y * l.y;
    if r2 <= LINEAR_TOLERANCE * LINEAR_TOLERANCE {
        return None;
    }
    Some((l.x * d.y - l.y * d.x) / r2)
}

impl FaceIntegrand {
    /// `-∫_{u0}^{u1} F(u, v) du` at constant `v`.
    fn horizontal(&self, v: f64, u0: f64, u1: f64) -> KernelResult<Moments> {
        let mut m = integrate(&|u| self.potential(u, v), u0, u1)?;
        for x in &mut m {
            *x = -*x;
        }
        Ok(m)
    }

    /// `-∫ F du` along one edge image: zero, at constant `v`, or along its curve.
    fn edge(&self, e: &EdgeImage) -> KernelResult<Moments> {
        let (Some(&(u0, v0)), Some(&(u1, _))) = (e.uv.first(), e.uv.last()) else {
            return Ok([0.0; 10]);
        };
        let tol = |x: f64| STRAIGHT_IMAGE_TOLERANCE * x.abs().max(1.0);
        if e.uv.iter().all(|q| (q.0 - u0).abs() <= tol(u0)) {
            return Ok([0.0; 10]);
        }
        if e.uv.iter().all(|q| (q.1 - v0).abs() <= tol(v0)) {
            return self.horizontal(v0, u0, u1);
        }
        let n = e.uv.len().saturating_sub(1).max(1);
        let pv = self.surface.period_v();
        let f = |t: f64| -> KernelResult<Moments> {
            let p = e.curve.point_at(t);
            let du = du_dt(&self.surface, p, e.curve.d1(t)).ok_or_else(|| {
                KernelError::Unsupported(format!(
                    "{} passes through the axis of its face's surface",
                    e.edge
                ))
            })?;
            let q = self.surface.parameters(p);
            let mut v = q.v;
            if let Some(period) = pv {
                // The unwrapped v of the nearest sample picks the representative.
                let s = ((t - e.t0) / (e.t1 - e.t0)).clamp(0.0, 1.0);
                let k = if e.forward { s } else { 1.0 - s };
                let idx = ((k * n as f64).round() as usize).min(n);
                if let Some(&(_, vs)) = e.uv.get(idx) {
                    v += ((vs - v) / period).round() * period;
                }
            }
            let mut m = self.potential(q.u, v)?;
            for x in &mut m {
                *x *= -du;
            }
            Ok(m)
        };
        let mut along = integrate(&f, e.t0, e.t1)?;
        if !e.forward {
            for x in &mut along {
                *x = -*x;
            }
        }
        Ok(along)
    }
}

/// The ten integrals' contributions of one face: its flux of each field.
fn face_moments(body: &Body, face: FaceId, c: Point3) -> KernelResult<Moments> {
    let (surface, orient) = body.face_orientation(face)?;
    let angular_v = matches!(surface, Surface::Sphere(_) | Surface::Torus(_));
    let (lo, hi) = surface.v_range();
    let v_ref = match surface {
        Surface::Sphere(_) | Surface::Torus(_) => 0.0,
        _ => surface.parameters(c).v.clamp(lo, hi),
    };
    let integrand = FaceIntegrand {
        surface,
        c,
        v_ref,
        angular_v,
    };
    let mut total = [0.0; 10];
    for l in body.face_loops(face)? {
        let img: LoopImage = body.loop_image(l, &integrand.surface, orient)?;
        if img.wraps_v {
            return Err(KernelError::Unsupported(format!(
                "{l} goes around a torus's tube without a seam"
            )));
        }
        for e in &img.edges {
            add(&mut total, &integrand.edge(e)?, 1.0);
        }
        for s in img.pole_steps() {
            add(&mut total, &integrand.horizontal(s.v, s.u0, s.u1)?, 1.0);
        }
    }
    Ok(total)
}

/// Eigenvalues (ascending) and unit eigenvectors of a symmetric 3x3 matrix by the cyclic
/// Jacobi method (Golub and Van Loan, *Matrix Computations*, algorithm 8.5.3).
pub(crate) fn symmetric_eigen(m: [[f64; 3]; 3]) -> ([f64; 3], [Vec3; 3]) {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let norm: f64 = a.iter().flatten().map(|x| x * x).sum::<f64>().sqrt();
    for _sweep in 0..64 {
        let off = (a[0][1] * a[0][1] + a[0][2] * a[0][2] + a[1][2] * a[1][2]).sqrt();
        if off <= 1e-15 * norm || off == 0.0 {
            break;
        }
        for (p, q) in [(0usize, 1usize), (0, 2), (1, 2)] {
            let apq = a[p][q];
            if apq == 0.0 {
                continue;
            }
            // The rotation that zeroes a[p][q] (Golub and Van Loan, algorithm 8.5.1).
            let tau = (a[q][q] - a[p][p]) / (2.0 * apq);
            let t = tau.signum() / (tau.abs() + (1.0 + tau * tau).sqrt());
            let t = if tau == 0.0 { 1.0 } else { t };
            let c = 1.0 / (1.0 + t * t).sqrt();
            let s = t * c;
            // A J: columns p and q.
            for row in &mut a {
                let (akp, akq) = (row[p], row[q]);
                row[p] = c * akp - s * akq;
                row[q] = s * akp + c * akq;
            }
            // Jᵀ (A J): rows p and q.
            let (rp, rq) = (a[p], a[q]);
            a[p] = [0, 1, 2].map(|k| c * rp[k] - s * rq[k]);
            a[q] = [0, 1, 2].map(|k| s * rp[k] + c * rq[k]);
            for row in &mut v {
                let (vp, vq) = (row[p], row[q]);
                row[p] = c * vp - s * vq;
                row[q] = s * vp + c * vq;
            }
        }
    }
    let mut order = [0usize, 1, 2];
    order.sort_by(|i, j| a[*i][*i].total_cmp(&a[*j][*j]));
    let values = order.map(|i| a[i][i]);
    let mut axes = order.map(|i| Vec3::new(v[0][i], v[1][i], v[2][i]));
    // Right-handed: the third axis is the cross product of the first two.
    let third = axes[0].cross(axes[1]);
    if third.dot(axes[2]) < 0.0 {
        axes[2] = -axes[2];
    }
    (values, axes)
}

impl Body {
    /// The body's mass properties for a uniform `density` (see [`MassProperties`] and the
    /// module documentation). Exact up to the quadrature tolerance on every face the
    /// measures support; refuses empty or inside-out bodies and hostile densities.
    pub fn mass_properties(&self, density: f64) -> KernelResult<MassProperties> {
        if !(density.is_finite() && density > 0.0 && density <= MAX_DENSITY) {
            return Err(KernelError::InvalidInput(format!(
                "`density` must be a positive number up to {MAX_DENSITY}"
            )));
        }
        if self.is_open() {
            return Err(crate::topology::open_volume());
        }
        let bounds = self
            .bounding_box()
            .ok_or_else(|| KernelError::InvalidInput("the body has no vertices".into()))?;
        let c = bounds.center();
        let mut m = [0.0; 10];
        for f in self.faces() {
            add(&mut m, &face_moments(self, f, c)?, 1.0);
        }
        let volume = m[0];
        let extent = (bounds.max - bounds.min).length().max(LINEAR_TOLERANCE);
        if !(volume.is_finite() && volume > LINEAR_TOLERANCE * extent * extent) {
            return Err(KernelError::InvalidInput(
                "the body encloses no volume (not a closed solid, or inside out)".into(),
            ));
        }
        let s = Vec3::new(m[1], m[2], m[3]);
        let d = s * (1.0 / volume);
        let centre_of_mass = c + d;
        // Central second moments C_ij = S_ij - V d_i d_j.
        let cxx = m[4] - volume * d.x * d.x;
        let cyy = m[5] - volume * d.y * d.y;
        let czz = m[6] - volume * d.z * d.z;
        let cxy = m[7] - volume * d.x * d.y;
        let cyz = m[8] - volume * d.y * d.z;
        let czx = m[9] - volume * d.z * d.x;
        let r = density;
        let inertia = [
            [r * (cyy + czz), -r * cxy, -r * czx],
            [-r * cxy, r * (cxx + czz), -r * cyz],
            [-r * czx, -r * cyz, r * (cxx + cyy)],
        ];
        let (principal_moments, principal_axes) = symmetric_eigen(inertia);
        Ok(MassProperties {
            density,
            volume,
            area: self.area()?,
            mass: density * volume,
            centre_of_mass,
            inertia,
            principal_moments,
            principal_axes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Line2, Profile, ProfileEdge, extrude, revolve, subtract};
    use chamfera_geom::{Frame, Point2, Vec2};
    use std::f64::consts::TAU;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * b.abs().max(1.0)
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Profile {
        let pts = [
            Point2::new(x0, y0),
            Point2::new(x1, y0),
            Point2::new(x1, y1),
            Point2::new(x0, y1),
        ];
        Profile::polygon(&pts)
    }

    #[test]
    fn a_box_has_its_centre_in_the_middle_and_the_textbook_inertia() {
        // 50 x 30 x 10 from the origin: m = ρ 15000, centre (25, 15, 5),
        // I_xx = m (b² + c²) / 12 with b = 30, c = 10, and so on; no products.
        let body = extrude(&Frame::XY, &rect(0.0, 0.0, 50.0, 30.0), 10.0).unwrap();
        let rho = 2.5;
        let p = body.mass_properties(rho).unwrap();
        let m = rho * 15_000.0;
        assert!(close(p.volume, 15_000.0, 1e-12));
        assert!(close(p.mass, m, 1e-12));
        assert!(close(p.area, 2.0 * (1500.0 + 500.0 + 300.0), 1e-12));
        let c = p.centre_of_mass;
        assert!(close(c.x, 25.0, 1e-12) && close(c.y, 15.0, 1e-12) && close(c.z, 5.0, 1e-12));
        let (ixx, iyy, izz) = (
            m * (30.0f64.powi(2) + 10.0f64.powi(2)) / 12.0,
            m * (50.0f64.powi(2) + 10.0f64.powi(2)) / 12.0,
            m * (50.0f64.powi(2) + 30.0f64.powi(2)) / 12.0,
        );
        assert!(close(p.inertia[0][0], ixx, 1e-11), "{:?}", p.inertia);
        assert!(close(p.inertia[1][1], iyy, 1e-11));
        assert!(close(p.inertia[2][2], izz, 1e-11));
        for (i, j) in [(0, 1), (0, 2), (1, 2)] {
            assert!(p.inertia[i][j].abs() <= 1e-9 * izz, "{:?}", p.inertia);
        }
        // Principal moments ascending: I_xx < I_yy < I_zz, along x, y, z.
        assert!(close(p.principal_moments[0], ixx, 1e-11));
        assert!(close(p.principal_moments[2], izz, 1e-11));
        assert!(p.principal_axes[0].x.abs() > 1.0 - 1e-9);
        assert!(p.principal_axes[2].z.abs() > 1.0 - 1e-9);
    }

    #[test]
    fn a_far_away_box_keeps_its_accuracy() {
        // The same box moved 100 m away: the moments about the centre do not change.
        let body = extrude(&Frame::XY, &rect(1e5, -1e5, 1e5 + 50.0, -1e5 + 30.0), 10.0).unwrap();
        let p = body.mass_properties(1.0).unwrap();
        let ixx = 15_000.0 * (900.0 + 100.0) / 12.0;
        assert!(close(p.inertia[0][0], ixx, 1e-9), "{}", p.inertia[0][0]);
        assert!(close(p.centre_of_mass.x, 1e5 + 25.0, 1e-14));
    }

    #[test]
    fn a_cylinder_has_its_centre_on_the_axis_and_i_zz_is_m_r_squared_over_two() {
        // r = 5, h = 10 about z: V = 250π, centre (0, 0, 5), I_zz = m r²/2,
        // I_xx = I_yy = m (3 r² + h²) / 12.
        let body = extrude(
            &Frame::XY,
            &Profile::circle(Point2::new(0.0, 0.0), 5.0),
            10.0,
        )
        .unwrap();
        let p = body.mass_properties(1.0).unwrap();
        let m = 250.0 * PI;
        assert!(close(p.volume, m, 1e-12), "{}", p.volume);
        let c = p.centre_of_mass;
        assert!(
            c.x.abs() < 1e-9 && c.y.abs() < 1e-9 && close(c.z, 5.0, 1e-12),
            "{c:?}"
        );
        assert!(
            close(p.inertia[2][2], m * 25.0 / 2.0, 1e-11),
            "{:?}",
            p.inertia
        );
        let ixx = m * (3.0 * 25.0 + 100.0) / 12.0;
        assert!(close(p.inertia[0][0], ixx, 1e-11));
        assert!(close(p.inertia[1][1], ixx, 1e-11));
        assert!(p.inertia[0][1].abs() < 1e-8 * ixx);
    }

    #[test]
    fn a_plate_with_a_hole_subtracts_the_hole() {
        // 40 x 20 x 5 plate on the origin, a through hole r = 4 at (10, 10):
        // V = 4000 - 80π; the centre moves away from the hole along x:
        // x̄ = (4000 · 20 - 80π · 10) / V, ȳ = 10, z̄ = 2.5.
        // I_zz about the centre = plate - hole, each by the parallel-axis theorem:
        // plate 4000 (40² + 20²)/12 + 4000 (20 - x̄)², hole 80π · 16/2 + 80π (10 - x̄)².
        let plate = extrude(&Frame::XY, &rect(0.0, 0.0, 40.0, 20.0), 5.0).unwrap();
        let p0 = plate.mass_properties(1.0).unwrap();
        let prof = rect(0.0, 0.0, 40.0, 20.0)
            .with_hole(vec![ProfileEdge::circle(Point2::new(10.0, 10.0), 4.0)]);
        let body = extrude(&Frame::XY, &prof, 5.0).unwrap();
        let p = body.mass_properties(1.0).unwrap();
        let (mp, mh) = (4000.0, 80.0 * PI);
        let v = mp - mh;
        let xb = (mp * 20.0 - mh * 10.0) / v;
        assert!(close(p.volume, v, 1e-12));
        let c = p.centre_of_mass;
        assert!(close(c.x, xb, 1e-12) && close(c.y, 10.0, 1e-12) && close(c.z, 2.5, 1e-12));
        let izz = mp * (1600.0 + 400.0) / 12.0 + mp * (20.0 - xb).powi(2)
            - (mh * 16.0 / 2.0 + mh * (10.0 - xb).powi(2));
        assert!(
            close(p.inertia[2][2], izz, 1e-11),
            "{} {izz}",
            p.inertia[2][2]
        );
        assert!(p.inertia[2][2] < p0.inertia[2][2]);
        // The same plate made by a boolean cut agrees.
        let pin = extrude(
            &Frame::XY,
            &Profile::circle(Point2::new(10.0, 10.0), 4.0),
            5.0,
        )
        .unwrap();
        let cut = subtract(&plate, &pin).unwrap();
        let q = cut.mass_properties(1.0).unwrap();
        assert!(close(q.inertia[2][2], izz, 1e-10));
        assert!(close(q.centre_of_mass.x, xb, 1e-11));
    }

    #[test]
    fn a_sphere_and_a_torus_match_their_closed_forms() {
        // Sphere r = 3: a half disc above the sketch x axis revolved about it:
        // V = 4/3 π r³, I = 2/5 m r² about any axis through the centre.
        let half = Profile::new(
            vec![
                ProfileEdge::line(Point2::new(-3.0, 0.0), Point2::new(3.0, 0.0)),
                ProfileEdge::Arc {
                    centre: Point2::new(0.0, 0.0),
                    radius: 3.0,
                    start_angle: 0.0,
                    sweep: PI,
                },
            ],
            Vec::new(),
        );
        let axis = Line2::new(Point2::new(0.0, 0.0), Vec2::new(1.0, 0.0)).unwrap();
        let sphere = revolve(&Frame::XY, &half, &axis, TAU).unwrap();
        let p = sphere.mass_properties(1.0).unwrap();
        let m = 4.0 / 3.0 * PI * 27.0;
        assert!(close(p.volume, m, 1e-11), "{}", p.volume);
        for k in 0..3 {
            assert!(
                close(p.inertia[k][k], 0.4 * m * 9.0, 1e-10),
                "{:?}",
                p.inertia
            );
        }
        assert!(p.centre_of_mass.x.abs() < 1e-9 && p.centre_of_mass.y.abs() < 1e-9);
        // Torus: a circle r = 1 at distance R = 4 from the axis, revolved about x:
        // V = 2π² R r², I_axis = m (R² + 3/4 r²), I_diameter = m (R²/2 + 5/8 r²)
        // (Goldstein, problem 5; or by Pappus and the parallel-axis theorem).
        let disc = Profile::circle(Point2::new(0.0, 4.0), 1.0);
        let torus = revolve(&Frame::XY, &disc, &axis, TAU).unwrap();
        let p = torus.mass_properties(1.0).unwrap();
        let m = 2.0 * PI * PI * 4.0;
        assert!(close(p.volume, m, 1e-11), "{}", p.volume);
        assert!(
            close(p.inertia[0][0], m * (16.0 + 0.75), 1e-10),
            "{:?}",
            p.inertia
        );
        assert!(close(p.inertia[1][1], m * (8.0 + 0.625), 1e-10));
        assert!(close(p.inertia[2][2], m * (8.0 + 0.625), 1e-10));
    }

    #[test]
    fn hostile_densities_are_refused() {
        let body = extrude(&Frame::XY, &rect(0.0, 0.0, 1.0, 1.0), 1.0).unwrap();
        for d in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e300] {
            assert!(body.mass_properties(d).is_err(), "{d}");
        }
        assert!(Body::new().mass_properties(1.0).is_err());
    }

    #[test]
    fn jacobi_finds_the_eigenvalues_of_a_rotated_diagonal_matrix() {
        // R diag(1, 2, 3) Rᵀ for a rotation by 30° about z has eigenvalues 1, 2, 3.
        let (s, c) = (PI / 6.0).sin_cos();
        let m = [
            [c * c * 1.0 + s * s * 2.0, c * s * (1.0 - 2.0), 0.0],
            [c * s * (1.0 - 2.0), s * s * 1.0 + c * c * 2.0, 0.0],
            [0.0, 0.0, 3.0],
        ];
        let (e, axes) = symmetric_eigen(m);
        assert!(close(e[0], 1.0, 1e-14) && close(e[1], 2.0, 1e-14) && close(e[2], 3.0, 1e-14));
        assert!((axes[0].x.abs() - c).abs() < 1e-12);
        assert!(axes[0].cross(axes[1]).dot(axes[2]) > 0.999);
    }
}
