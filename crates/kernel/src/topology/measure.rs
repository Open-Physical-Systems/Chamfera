//! Area and volume of faces and solids on planes, cylinders, cones, spheres and tori
//! (faces on extruded splines: `spline.rs`).
//!
//! # Planar faces (exact)
//! The vector area of a loop is `½ ∮ (p - r) × dp` (Stokes' theorem on the plane), summed
//! in closed form per edge ([`Curve::area_moment`](super::Curve::area_moment)). With outer
//! loops counter-clockwise and inner loops clockwise about the outward normal `n`, the
//! face area is `n · Σ_loops A_loop`: holes subtract themselves.
//!
//! # Faces on surfaces of revolution (Green's theorem in `(u, v)`)
//! On a surface `S(u, v) = o + ρ(v) e(u) + h(v) z` (see `surface.rs`), with a face's loops
//! mapped into the parameter plane (`uv.rs`), Green's theorem turns a surface integral
//! `∬_D f du dv` into `∮_∂D -F du` for any `F` with `∂F/∂v = f`:
//! `d(-F du) = ∂F/∂v du ∧ dv`.
//! - Area: `f = |S_u × S_v| = G'(v)`, a function of `v` only
//!   ([`Meridian::area_antiderivative`]: `r v` on a cylinder, `r v + s v²/2` on a cone,
//!   `r² sin v` on a sphere, `a (R v + a sin v)` on a torus).
//! - Volume: by the divergence theorem with `F(p) = (p - c)/3`, `V = ⅓ Σ_faces ∬ (p - c) ·
//!   n dA`, and on a curved face `(S - c) · (S_u × S_v) = A'(v) - c_r(u) B'(v) + c_z C'(v)`
//!   with `c_r(u) = c_x cos u + c_y sin u` in the surface's frame
//!   ([`Meridian::volume_antiderivatives`]).
//!
//! The boundary integral is taken edge by edge along the loop images. An edge whose
//! image is vertical (constant `u`: a line along or through the axis, a profile curve in a
//! plane through the axis) contributes nothing, as `du = 0`. One whose image is horizontal
//! (constant `v`: a circle about the axis) and each step along a singular line contribute
//! `-∫ F(u, v₀) du` in closed form. Every edge K4 builds is one of these, so the measures
//! of extruded and revolved solids are exact. Any other edge is integrated numerically:
//! `-∫ F(u(t), v(t)) u'(t) dt` along its curve, with `u' = (x y' - y x') / (x² + y²)` in the
//! surface frame, by adaptive Gauss-Legendre quadrature to a relative accuracy of
//! [`SURFACE_QUADRATURE_TOLERANCE`].
//!
//! A face's loops are counter-clockwise in `(u, v)` when its outward normal is
//! `S_u × S_v`, clockwise otherwise; the boundary integral of `-F du` is then `±` the
//! surface integral, which gives the sign of each term. A loop that goes once around the
//! `u` direction without a seam (a cylinder face between two circles) is fine: a virtual
//! seam at constant `u` would contribute nothing. Loops that go around the `v` direction
//! of a torus without a seam are not supported.
//!
//! Planar faces enter the volume as `((o_f - c) · n_f) A_f`, because `(p - c) · n_f` is
//! constant on the plane through `o_f`. `c` is any point; a point of the body keeps the sum
//! accurate.

use super::uv::{EdgeImage, LoopImage};
use super::{Body, FaceId, ShellId, Surface};
use crate::surface::Meridian;
use crate::{KernelError, KernelResult};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point3, Vec3};

/// Relative accuracy of the numeric boundary integrals of edges that are neither along nor
/// around a surface's axis. Each panel's 5-point Gauss-Legendre estimate is compared with
/// the sum over its halves, and refined until they agree to this fraction of the integral's
/// scale (shared out by panel width). The integrands are analytic on each edge, so the
/// halves' estimate is far more accurate than the difference, which bounds the error with
/// a wide margin.
pub const SURFACE_QUADRATURE_TOLERANCE: f64 = 1e-12;
/// Most quadrature panels one edge integral may use (hostile-input limit).
const MAX_PANELS: usize = 20_000;
/// Two image coordinates within this are the same (radians, or mm along a cylinder):
/// projections of an exactly horizontal or vertical edge differ only by rounding.
const STRAIGHT_IMAGE_TOLERANCE: f64 = 1e-9;

/// What a curved face's boundary integral measures.
#[derive(Debug, Clone, Copy)]
enum Integrand {
    Area,
    /// `∬ (p - c) · (S_u × S_v) du dv`, `c` in the surface's local coordinates.
    Volume(Point3),
}

/// 5-point Gauss-Legendre nodes and weights on `[-1, 1]`.
const GAUSS5: [(f64, f64); 5] = [
    (0.0, 0.568_888_888_888_888_9),
    (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
    (0.906_179_845_938_664, 0.236_926_885_056_189_1),
];

fn gauss5(f: &dyn Fn(f64) -> KernelResult<f64>, a: f64, b: f64) -> KernelResult<f64> {
    let (m, h) = (0.5 * (a + b), 0.5 * (b - a));
    let mut s = 0.0;
    for (x, w) in GAUSS5 {
        s += w * f(m + h * x)?;
    }
    Ok(s * h)
}

/// `∫_a^b f` by adaptive Gauss-Legendre quadrature (see [`SURFACE_QUADRATURE_TOLERANCE`]).
fn integrate(f: &dyn Fn(f64) -> KernelResult<f64>, a: f64, b: f64) -> KernelResult<f64> {
    if a == b {
        return Ok(0.0);
    }
    // The scale: |f| integrated coarsely over 16 panels.
    let width = b - a;
    let mut scale = 0.0;
    for i in 0..16 {
        let (x0, x1) = (
            a + width * i as f64 / 16.0,
            a + width * (i + 1) as f64 / 16.0,
        );
        scale += gauss5(&|t| Ok(f(t)?.abs()), x0, x1)?.abs();
    }
    let tol = SURFACE_QUADRATURE_TOLERANCE * scale.max(f64::MIN_POSITIVE);
    let mut stack = vec![(a, b, gauss5(f, a, b)?)];
    let mut total = 0.0;
    let mut panels = 0usize;
    while let Some((x0, x1, whole)) = stack.pop() {
        panels += 1;
        if panels > MAX_PANELS {
            return Err(KernelError::NoConvergence(
                "an edge's boundary integral needs more quadrature panels than allowed".into(),
            ));
        }
        let m = 0.5 * (x0 + x1);
        let (l, r) = (gauss5(f, x0, m)?, gauss5(f, m, x1)?);
        if (l + r - whole).abs() <= tol * ((x1 - x0) / width).abs() || m <= x0 || m >= x1 {
            total += l + r;
        } else {
            stack.push((x0, m, l));
            stack.push((m, x1, r));
        }
    }
    Ok(total)
}

/// `-∫_{u0}^{u1} F(u, v) du` at constant `v`, in closed form.
fn horizontal_term(m: &Meridian, kind: Integrand, v: f64, u0: f64, u1: f64) -> f64 {
    match kind {
        Integrand::Area => -m.area_antiderivative(v) * (u1 - u0),
        Integrand::Volume(c) => {
            let (a, b, cc) = m.volume_antiderivatives(v);
            // ∫ c_r du = c_x (sin u1 - sin u0) - c_y (cos u1 - cos u0).
            let cr = c.x * (u1.sin() - u0.sin()) - c.y * (u1.cos() - u0.cos());
            -((a + c.z * cc) * (u1 - u0) - b * cr)
        }
    }
}

/// `F(u, v)` itself.
fn potential(m: &Meridian, kind: Integrand, u: f64, v: f64) -> f64 {
    match kind {
        Integrand::Area => m.area_antiderivative(v),
        Integrand::Volume(c) => {
            let (a, b, cc) = m.volume_antiderivatives(v);
            a - (c.x * u.cos() + c.y * u.sin()) * b + c.z * cc
        }
    }
}

/// `-∫ F du` along one edge image, by quadrature on its curve.
fn edge_quadrature(
    surface: &Surface,
    frame: &Frame,
    m: &Meridian,
    kind: Integrand,
    e: &EdgeImage,
) -> KernelResult<f64> {
    let n = e.uv.len().saturating_sub(1).max(1);
    let pv = surface.period_v();
    let f = |t: f64| -> KernelResult<f64> {
        let p = frame.to_local(e.curve.point_at(t));
        let d = frame.vector_to_local(e.curve.d1(t));
        let r2 = p.x * p.x + p.y * p.y;
        if r2 <= LINEAR_TOLERANCE * LINEAR_TOLERANCE {
            return Err(KernelError::Unsupported(format!(
                "{} passes through the axis of its face's surface",
                e.edge
            )));
        }
        let du = (p.x * d.y - p.y * d.x) / r2;
        let q = surface.parameters(frame.to_world(p));
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
        Ok(-potential(m, kind, q.u, v) * du)
    };
    let along = integrate(&f, e.t0, e.t1)?;
    Ok(if e.forward { along } else { -along })
}

/// The contribution of one edge image: zero, closed form or quadrature.
fn edge_term(
    surface: &Surface,
    frame: &Frame,
    m: &Meridian,
    kind: Integrand,
    e: &EdgeImage,
) -> KernelResult<f64> {
    let (Some(&(u0, v0)), Some(&(u1, _))) = (e.uv.first(), e.uv.last()) else {
        return Ok(0.0);
    };
    let tol = |x: f64| STRAIGHT_IMAGE_TOLERANCE * x.abs().max(1.0);
    // Singular end samples borrow u, so they never spoil the vertical test.
    if e.uv.iter().all(|q| (q.0 - u0).abs() <= tol(u0)) {
        return Ok(0.0);
    }
    if e.uv.iter().all(|q| (q.1 - v0).abs() <= tol(v0)) {
        return Ok(horizontal_term(m, kind, v0, u0, u1));
    }
    edge_quadrature(surface, frame, m, kind, e)
}

impl Body {
    /// Outward unit normal and a point of a planar face.
    fn planar_face(&self, face: FaceId) -> KernelResult<Option<(Vec3, Point3)>> {
        match self.face_surface(face)? {
            Some((Surface::Plane(frame), same_sense)) => {
                let n = if same_sense { frame.z() } else { -frame.z() };
                Ok(Some((n, frame.origin())))
            }
            Some(_) => Ok(None),
            None => Err(KernelError::InvalidInput(format!(
                "{face} has no surface yet"
            ))),
        }
    }

    /// `∮ -F du` over the loops of a face on a surface of revolution, with `F` for
    /// `kind` (see the module documentation).
    fn curved_integral(&self, face: FaceId, kind: Integrand) -> KernelResult<(f64, f64)> {
        let (surface, orient) = self.face_orientation(face)?;
        if let Surface::Extrusion(e) = &surface {
            let centre = match kind {
                Integrand::Area => None,
                Integrand::Volume(c) => Some(c),
            };
            let total = self.extrusion_integral(face, &surface, e, orient, centre)?;
            return Ok((total, orient));
        }
        let m = surface.meridian().ok_or_else(|| {
            KernelError::InvalidInput(format!("{face} is planar, not on a curved surface"))
        })?;
        let frame = *surface.frame();
        let mut total = 0.0;
        for l in self.face_loops(face)? {
            let img: LoopImage = self.loop_image(l, &surface, orient)?;
            if img.wraps_v {
                return Err(KernelError::Unsupported(format!(
                    "{l} goes around a torus's tube without a seam"
                )));
            }
            for e in &img.edges {
                total += edge_term(&surface, &frame, &m, kind, e)?;
            }
            for s in img.pole_steps() {
                total += horizontal_term(&m, kind, s.v, s.u0, s.u1);
            }
        }
        Ok((total, orient))
    }

    /// The area of a face (outer loop minus holes): exact on planes and for faces whose
    /// edges run along or around their surface's axis (everything K4 builds), otherwise
    /// to [`SURFACE_QUADRATURE_TOLERANCE`].
    pub fn face_area(&self, face: FaceId) -> KernelResult<f64> {
        if let Some((n, _)) = self.planar_face(face)? {
            let mut sum = Vec3::ZERO;
            for l in self.face_loops(face)? {
                sum = sum + self.loop_vector_area(l)?;
            }
            return Ok(sum.dot(n));
        }
        let (integral, orient) = self.curved_integral(face, Integrand::Area)?;
        Ok(integral * orient)
    }

    /// The total area of every face of the body.
    pub fn area(&self) -> KernelResult<f64> {
        self.faces()
            .into_iter()
            .map(|f| self.face_area(f))
            .sum::<KernelResult<f64>>()
    }

    /// `∬_face (p - c) · n dA`.
    fn face_flux(&self, face: FaceId, c: Point3) -> KernelResult<f64> {
        if let Some((n, o)) = self.planar_face(face)? {
            return Ok((o - c).dot(n) * self.face_area(face)?);
        }
        let (surface, _) = self.face_orientation(face)?;
        let local = surface.frame().to_local(c);
        // The loops' direction already carries the face's orientation.
        Ok(self.curved_integral(face, Integrand::Volume(local))?.0)
    }

    /// The volume enclosed by one closed shell.
    pub fn shell_volume(&self, shell: ShellId) -> KernelResult<f64> {
        let faces = self.shell_faces(shell)?;
        let Some(first) = faces.first() else {
            return Ok(0.0);
        };
        let c = match self.face_loops(*first)?.first() {
            Some(l) => match self.loop_vertices(*l)?.first() {
                Some(v) => self.vertex_point(*v)?,
                None => Point3::ORIGIN,
            },
            None => Point3::ORIGIN,
        };
        let mut v = 0.0;
        for f in faces {
            v += self.face_flux(f, c)?;
        }
        Ok(v / 3.0)
    }

    /// The volume of the body: the sum over its shells.
    pub fn volume(&self) -> KernelResult<f64> {
        self.shells()
            .into_iter()
            .map(|s| self.shell_volume(s))
            .sum::<KernelResult<f64>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Circle3;
    use crate::topology::Curve;
    use std::f64::consts::{PI, TAU};

    #[test]
    fn quadrature_agrees_with_the_closed_form_on_a_circle_about_the_axis() {
        // A parallel of a torus R = 5, a = 2 at v = 1, run once around: the closed form
        // and the numeric integral of -F du must agree, for area and volume terms.
        let s = Surface::torus(Frame::XY, 5.0, 2.0).unwrap();
        let m = s.meridian().unwrap();
        let v = 1.0f64;
        let rho = 5.0 + 2.0 * v.cos();
        let centre = Point3::new(0.0, 0.0, 2.0 * v.sin());
        let circle = Circle3::new(Frame::XY.with_origin(centre).unwrap(), rho).unwrap();
        let n = 64;
        let uv: Vec<(f64, f64)> = (0..=n).map(|i| (TAU * i as f64 / n as f64, v)).collect();
        let e = EdgeImage {
            edge: super::super::EdgeId::from_index(0),
            curve: Curve::Circle(circle),
            t0: 0.0,
            t1: TAU,
            forward: true,
            uv,
            singular_start: false,
            singular_end: false,
        };
        for kind in [
            Integrand::Area,
            Integrand::Volume(Point3::new(1.0, -2.0, 0.5)),
        ] {
            let closed = horizontal_term(&m, kind, v, 0.0, TAU);
            let numeric = edge_quadrature(&s, &Frame::XY, &m, kind, &e).unwrap();
            assert!(
                (closed - numeric).abs() <= 1e-10 * closed.abs().max(1.0),
                "{kind:?}"
            );
        }
        // The area term alone is -G(v) 2π with G = a (R v + a sin v).
        let g = 2.0 * (5.0 * v + 2.0 * v.sin());
        assert!((horizontal_term(&m, Integrand::Area, v, 0.0, TAU) + g * TAU).abs() < 1e-12);
    }

    #[test]
    fn gauss_legendre_integrates_polynomials_and_trigonometry() {
        // ∫_0^2 x^7 dx = 2^8 / 8 = 32; ∫_0^π sin x dx = 2.
        let p = integrate(&|x: f64| Ok(x.powi(7)), 0.0, 2.0).unwrap();
        assert!((p - 32.0).abs() < 1e-12);
        let s = integrate(&|x: f64| Ok(x.sin()), 0.0, PI).unwrap();
        assert!((s - 2.0).abs() < 1e-13);
        let e = integrate(&|_| Err(KernelError::InvalidInput("x".into())), 0.0, 1.0);
        assert!(e.is_err());
    }
}
