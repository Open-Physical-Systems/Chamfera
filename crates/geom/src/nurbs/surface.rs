//! Rational tensor-product B-spline surfaces (The NURBS Book, chapters 3.4 and 4.4).
//!
//! `S(u, v) = Σ_i Σ_j N_{i,p}(u) N_{j,q}(v) w_ij P_ij / Σ_i Σ_j N_{i,p}(u) N_{j,q}(v) w_ij`
//! (eq. 4.15), evaluated in homogeneous coordinates: with `A(u, v)` the numerator and
//! `W(u, v)` the denominator, `S = A / W` and `S_u = (A_u - W_u S) / W`, likewise for `v`
//! (eq. 4.19 for first derivatives). The surface is checked when it is made: each
//! direction's knot vector, degree and control-point count obey the curve rules
//! ([`super::BSplineCurve3::new`]), every control point lies inside the working range and
//! every weight in `[`[`MIN_WEIGHT`]`, `[`MAX_WEIGHT`]`]`, and the net holds at most
//! [`MAX_SURFACE_CONTROL_POINTS`] points.
//!
//! Second derivatives come from the same homogeneous sums (eq. 4.20 with `k + l <= 2`):
//! `S_uu = (A_uu - 2 W_u S_u - W_uu S) / W`, `S_uv = (A_uv - W_u S_v - W_v S_u - W_uv S) /
//! W` and `S_vv` like `S_uu`. Sub-patches ([`BSplineSurface3::sub_patch`]) cut every
//! column, then every row, by knot insertion (§5.2 applied in each direction, as §5.3
//! does for surfaces); with positive weights a surface lies in the convex hull of its
//! control points (§4.4), so a sub-patch's control-point box bounds that part of the
//! surface ([`BSplineSurface3::hull_box`]).
//!
//! This is the geometry an importer reads (STEP `B_SPLINE_SURFACE`) and the kernel's
//! free-form faces lie on.

use super::basis::{ders_basis_funs, find_span};
use super::core::Nurbs;
use super::{MAX_WEIGHT, MIN_WEIGHT, NurbsError};
use crate::{Point3, Transform, Vec3};

/// A surface point with its first and second partial derivatives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceDerivatives {
    pub point: Point3,
    pub su: Vec3,
    pub sv: Vec3,
    pub suu: Vec3,
    pub suv: Vec3,
    pub svv: Vec3,
}

/// Most control points one surface may have (both directions together).
pub const MAX_SURFACE_CONTROL_POINTS: usize = 65_536;

/// A rational B-spline surface: degrees `(p, q)`, knot vectors `U`, `V`, an `(n + 1) x
/// (m + 1)` net of control points `P_ij` (`i` along `u`) with weights. Valid by
/// construction.
#[derive(Debug, Clone, PartialEq)]
pub struct BSplineSurface3 {
    degree_u: usize,
    degree_v: usize,
    knots_u: Vec<f64>,
    knots_v: Vec<f64>,
    /// Row-major: `P_ij` at `i * count_v + j`.
    points: Vec<Point3>,
    weights: Vec<f64>,
    count_u: usize,
    count_v: usize,
}

fn invalid(msg: impl Into<String>) -> NurbsError {
    NurbsError::InvalidInput(msg.into())
}

impl BSplineSurface3 {
    /// Builds a surface from its net `points[i][j]` (rows along `u`) and, for a rational
    /// surface, the matching `weights[i][j]` (`None`: all 1). See the module
    /// documentation for the rules.
    pub fn new(
        degree_u: usize,
        degree_v: usize,
        knots_u: Vec<f64>,
        knots_v: Vec<f64>,
        points: Vec<Vec<Point3>>,
        weights: Option<Vec<Vec<f64>>>,
    ) -> Result<Self, NurbsError> {
        let count_u = points.len();
        let count_v = points.first().map_or(0, Vec::len);
        if count_u == 0 || count_v == 0 {
            return Err(invalid("a surface needs control points"));
        }
        if points.iter().any(|r| r.len() != count_v) {
            return Err(invalid(
                "every row of the control net must have the same number of points",
            ));
        }
        if count_u.saturating_mul(count_v) > MAX_SURFACE_CONTROL_POINTS {
            return Err(invalid(format!(
                "a surface may have at most {MAX_SURFACE_CONTROL_POINTS} control points"
            )));
        }
        let weights = match weights {
            Some(w) => {
                if w.len() != count_u || w.iter().any(|r| r.len() != count_v) {
                    return Err(invalid(
                        "there must be one weight per control point, in the net's shape",
                    ));
                }
                w
            }
            None => vec![vec![1.0; count_v]; count_u],
        };
        let flat_points: Vec<Point3> = points.into_iter().flatten().collect();
        let flat_weights: Vec<f64> = weights.into_iter().flatten().collect();
        if flat_points.iter().any(|p| !p.is_valid()) {
            return Err(invalid(
                "control points must be finite and inside the model's working range",
            ));
        }
        if flat_weights
            .iter()
            .any(|w| !(w.is_finite() && (MIN_WEIGHT..=MAX_WEIGHT).contains(w)))
        {
            return Err(invalid(format!(
                "weights must be finite numbers between {MIN_WEIGHT} and {MAX_WEIGHT}"
            )));
        }
        // Each direction's degree, count and knots obey the curve rules: check them on
        // one column (u) and one row (v) of the net.
        let column: Vec<[f64; 1]> = vec![[0.0]; count_u];
        Nurbs::<1>::new(degree_u, knots_u.clone(), column, vec![1.0; count_u])
            .map_err(|e| invalid(format!("along u: {e}")))?;
        let row: Vec<[f64; 1]> = vec![[0.0]; count_v];
        Nurbs::<1>::new(degree_v, knots_v.clone(), row, vec![1.0; count_v])
            .map_err(|e| invalid(format!("along v: {e}")))?;
        Ok(BSplineSurface3 {
            degree_u,
            degree_v,
            knots_u,
            knots_v,
            points: flat_points,
            weights: flat_weights,
            count_u,
            count_v,
        })
    }

    /// `(p, q)`.
    pub fn degrees(&self) -> (usize, usize) {
        (self.degree_u, self.degree_v)
    }
    pub fn knots_u(&self) -> &[f64] {
        &self.knots_u
    }
    pub fn knots_v(&self) -> &[f64] {
        &self.knots_v
    }
    /// `(n + 1, m + 1)`: control points along `u` and along `v`.
    pub fn counts(&self) -> (usize, usize) {
        (self.count_u, self.count_v)
    }
    /// `P_ij`, if it exists.
    pub fn control_point(&self, i: usize, j: usize) -> Option<Point3> {
        if i >= self.count_u || j >= self.count_v {
            return None;
        }
        self.points.get(i * self.count_v + j).copied()
    }
    /// `w_ij`, if it exists.
    pub fn weight(&self, i: usize, j: usize) -> Option<f64> {
        if i >= self.count_u || j >= self.count_v {
            return None;
        }
        self.weights.get(i * self.count_v + j).copied()
    }
    /// Every control point, row by row.
    pub fn control_points(&self) -> &[Point3] {
        &self.points
    }
    pub fn is_rational(&self) -> bool {
        let w0 = self.weights.first().copied().unwrap_or(1.0);
        self.weights.iter().any(|w| (w - w0).abs() > 1e-15 * w0)
    }
    /// The parameter domain along `u`, `[U_p, U_{n+1}]`.
    pub fn domain_u(&self) -> (f64, f64) {
        domain(&self.knots_u, self.degree_u, self.count_u)
    }
    /// The parameter domain along `v`.
    pub fn domain_v(&self) -> (f64, f64) {
        domain(&self.knots_v, self.degree_v, self.count_v)
    }

    /// `S(u, v)`; parameters are clamped to the domain.
    pub fn point_at(&self, u: f64, v: f64) -> Point3 {
        self.derivatives(u, v).0
    }

    /// `(S, S_u, S_v)` at `(u, v)` (clamped to the domain), by eq. 4.19.
    pub fn derivatives(&self, u: f64, v: f64) -> (Point3, Vec3, Vec3) {
        let (p, q) = (self.degree_u, self.degree_v);
        let u = clamp(u, self.domain_u());
        let v = clamp(v, self.domain_v());
        let su = find_span(self.count_u - 1, p, u, &self.knots_u);
        let sv = find_span(self.count_v - 1, q, v, &self.knots_v);
        let nu = ders_basis_funs(su, u, p, 1, &self.knots_u);
        let nv = ders_basis_funs(sv, v, q, 1, &self.knots_v);
        let row = |r: &Vec<Vec<f64>>, k: usize| r.get(k).cloned().unwrap_or_default();
        let (nu0, nu1, nv0, nv1) = (row(&nu, 0), row(&nu, 1), row(&nv, 0), row(&nv, 1));
        // Homogeneous sums: (A, W) and their u and v derivatives.
        let (mut a, mut a_u, mut a_v) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
        let (mut w, mut w_u, mut w_v) = (0.0, 0.0, 0.0);
        for (k, (b0, b1)) in nu0
            .iter()
            .zip(nu1.iter().chain(std::iter::repeat(&0.0)))
            .enumerate()
        {
            let i = su + k - p;
            for (l, (c0, c1)) in nv0
                .iter()
                .zip(nv1.iter().chain(std::iter::repeat(&0.0)))
                .enumerate()
            {
                let j = sv + l - q;
                let (Some(pt), Some(wt)) = (self.control_point(i, j), self.weight(i, j)) else {
                    continue;
                };
                let pw = (pt - Point3::ORIGIN) * wt;
                a = a + pw * (b0 * c0);
                a_u = a_u + pw * (b1 * c0);
                a_v = a_v + pw * (b0 * c1);
                w += wt * b0 * c0;
                w_u += wt * b1 * c0;
                w_v += wt * b0 * c1;
            }
        }
        if w.abs() < f64::MIN_POSITIVE {
            return (Point3::ORIGIN, Vec3::ZERO, Vec3::ZERO);
        }
        let s = a * (1.0 / w);
        let s_u = (a_u - s * w_u) * (1.0 / w);
        let s_v = (a_v - s * w_v) * (1.0 / w);
        (Point3::ORIGIN + s, s_u, s_v)
    }

    /// `S` and its partial derivatives up to second order at `(u, v)` (clamped to the
    /// domain), by eq. 4.20 on the homogeneous sums (see the module documentation).
    pub fn derivatives2(&self, u: f64, v: f64) -> SurfaceDerivatives {
        let (p, q) = (self.degree_u, self.degree_v);
        let u = clamp(u, self.domain_u());
        let v = clamp(v, self.domain_v());
        let su = find_span(self.count_u - 1, p, u, &self.knots_u);
        let sv = find_span(self.count_v - 1, q, v, &self.knots_v);
        let nu = ders_basis_funs(su, u, p, 2, &self.knots_u);
        let nv = ders_basis_funs(sv, v, q, 2, &self.knots_v);
        let at = |r: &[Vec<f64>], d: usize, k: usize| {
            r.get(d).and_then(|row| row.get(k)).copied().unwrap_or(0.0)
        };
        // Homogeneous sums A^(k,l) and W^(k,l) for k + l <= 2, indexed [k][l].
        let mut a = [[Vec3::ZERO; 3]; 3];
        let mut w = [[0.0f64; 3]; 3];
        for k in 0..=p {
            let i = su + k - p;
            for l in 0..=q {
                let j = sv + l - q;
                let (Some(pt), Some(wt)) = (self.control_point(i, j), self.weight(i, j)) else {
                    continue;
                };
                let pw = (pt - Point3::ORIGIN) * wt;
                for (du, (a_row, w_row)) in a.iter_mut().zip(w.iter_mut()).enumerate() {
                    for (dv, (a_kl, w_kl)) in a_row.iter_mut().zip(w_row.iter_mut()).enumerate() {
                        if du + dv > 2 {
                            continue;
                        }
                        let b = at(&nu, du, k) * at(&nv, dv, l);
                        *a_kl = *a_kl + pw * b;
                        *w_kl += wt * b;
                    }
                }
            }
        }
        let w0 = w[0][0];
        if w0.abs() < f64::MIN_POSITIVE {
            return SurfaceDerivatives {
                point: Point3::ORIGIN,
                su: Vec3::ZERO,
                sv: Vec3::ZERO,
                suu: Vec3::ZERO,
                suv: Vec3::ZERO,
                svv: Vec3::ZERO,
            };
        }
        let inv = 1.0 / w0;
        let s = a[0][0] * inv;
        let s_u = (a[1][0] - s * w[1][0]) * inv;
        let s_v = (a[0][1] - s * w[0][1]) * inv;
        let s_uu = (a[2][0] - s_u * (2.0 * w[1][0]) - s * w[2][0]) * inv;
        let s_uv = (a[1][1] - s_v * w[1][0] - s_u * w[0][1] - s * w[1][1]) * inv;
        let s_vv = (a[0][2] - s_v * (2.0 * w[0][1]) - s * w[0][2]) * inv;
        SurfaceDerivatives {
            point: Point3::ORIGIN + s,
            su: s_u,
            sv: s_v,
            suu: s_uu,
            suv: s_uv,
            svv: s_vv,
        }
    }

    /// The distinct knots inside the `u` domain, with its ends: the spans on which the
    /// surface is rational in `u`.
    pub fn breaks_u(&self) -> Vec<f64> {
        breaks(&self.knots_u, self.domain_u())
    }
    /// The same along `v`.
    pub fn breaks_v(&self) -> Vec<f64> {
        breaks(&self.knots_v, self.domain_v())
    }

    /// The box around the control points, `(min, max)`, which holds the whole surface
    /// (the convex hull property, §4.4: every weight is positive).
    pub fn hull_box(&self) -> (Point3, Point3) {
        let mut lo = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut hi = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in &self.points {
            lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        (lo, hi)
    }

    /// The part of the surface over `[u0, u1] x [v0, v1]` (clamped to the domain, each
    /// range increasing), with the same parameterisation: every column is cut along `u`,
    /// then every row along `v`, by knot insertion.
    pub fn sub_patch(&self, u0: f64, u1: f64, v0: f64, v1: f64) -> Result<Self, NurbsError> {
        let (nu, nv) = (self.count_u, self.count_v);
        // Columns (fixed j) cut along u.
        let mut columns: Vec<Nurbs<3>> = Vec::with_capacity(nv);
        for j in 0..nv {
            let mut pts = Vec::with_capacity(nu);
            let mut ws = Vec::with_capacity(nu);
            for i in 0..nu {
                let (Some(pt), Some(wt)) = (self.control_point(i, j), self.weight(i, j)) else {
                    return Err(invalid("the control net is incomplete"));
                };
                pts.push([pt.x, pt.y, pt.z]);
                ws.push(wt);
            }
            let c = Nurbs::new(self.degree_u, self.knots_u.clone(), pts, ws)?;
            columns.push(segment_or_whole(&c, u0, u1)?);
        }
        let knots_u = columns
            .first()
            .map(|c| c.knots.clone())
            .ok_or_else(|| invalid("a surface needs control points"))?;
        let rows_u = columns.first().map_or(0, |c| c.points.len());
        let mut net = Vec::with_capacity(rows_u);
        let mut weights = Vec::with_capacity(rows_u);
        let mut knots_v = Vec::new();
        for i in 0..rows_u {
            let mut pts = Vec::with_capacity(nv);
            let mut ws = Vec::with_capacity(nv);
            for c in &columns {
                let (Some(pt), Some(wt)) = (c.points.get(i), c.weights.get(i)) else {
                    return Err(invalid("knot insertion gave columns of different lengths"));
                };
                pts.push(*pt);
                ws.push(*wt);
            }
            let r = Nurbs::new(self.degree_v, self.knots_v.clone(), pts, ws)?;
            let r = segment_or_whole(&r, v0, v1)?;
            knots_v.clone_from(&r.knots);
            net.push(
                r.points
                    .iter()
                    .map(|a| Point3::new(a[0], a[1], a[2]))
                    .collect(),
            );
            weights.push(r.weights.clone());
        }
        BSplineSurface3::new(
            self.degree_u,
            self.degree_v,
            knots_u,
            knots_v,
            net,
            Some(weights),
        )
    }

    /// The surface with its parameters swapped: `S'(s, t) = S(t, s)`. Its normal
    /// `S'_s × S'_t = S_v × S_u` is the opposite of this surface's.
    pub fn transposed(&self) -> Self {
        let (nu, nv) = (self.count_u, self.count_v);
        let mut points = Vec::with_capacity(self.points.len());
        let mut weights = Vec::with_capacity(self.weights.len());
        for j in 0..nv {
            for i in 0..nu {
                points.push(self.control_point(i, j).unwrap_or(Point3::ORIGIN));
                weights.push(self.weight(i, j).unwrap_or(1.0));
            }
        }
        BSplineSurface3 {
            degree_u: self.degree_v,
            degree_v: self.degree_u,
            knots_u: self.knots_v.clone(),
            knots_v: self.knots_u.clone(),
            points,
            weights,
            count_u: nv,
            count_v: nu,
        }
    }

    /// The surface with `u` running backwards: `S'(u, v) = S(U_0 + U_last - u, v)`. Its
    /// normal is the opposite of this surface's.
    pub fn reversed_u(&self) -> Self {
        let a = self.knots_u.first().copied().unwrap_or(0.0);
        let b = self.knots_u.last().copied().unwrap_or(1.0);
        let (nu, nv) = (self.count_u, self.count_v);
        let mut points = Vec::with_capacity(self.points.len());
        let mut weights = Vec::with_capacity(self.weights.len());
        for i in (0..nu).rev() {
            for j in 0..nv {
                points.push(self.control_point(i, j).unwrap_or(Point3::ORIGIN));
                weights.push(self.weight(i, j).unwrap_or(1.0));
            }
        }
        BSplineSurface3 {
            knots_u: self.knots_u.iter().rev().map(|k| a + b - k).collect(),
            points,
            weights,
            ..self.clone()
        }
    }

    /// The surface moved by `t` (affine invariance, §4.4: the control points are mapped,
    /// knots and weights kept). Fails when a moved control point leaves the working range.
    pub fn transformed(&self, t: &Transform) -> Result<Self, NurbsError> {
        let points: Vec<Point3> = self.points.iter().map(|p| t.apply_point(*p)).collect();
        if points.iter().any(|p| !p.is_valid()) {
            return Err(invalid(
                "the moved control points would leave the model's working range",
            ));
        }
        Ok(BSplineSurface3 {
            points,
            ..self.clone()
        })
    }

    /// The control net as rows along `u`, and the weights in the same shape (the
    /// arguments [`BSplineSurface3::new`] takes).
    pub fn net(&self) -> (Vec<Vec<Point3>>, Vec<Vec<f64>>) {
        let rows = (0..self.count_u)
            .map(|i| {
                (0..self.count_v)
                    .map(|j| self.control_point(i, j).unwrap_or(Point3::ORIGIN))
                    .collect()
            })
            .collect();
        let weights = (0..self.count_u)
            .map(|i| {
                (0..self.count_v)
                    .map(|j| self.weight(i, j).unwrap_or(1.0))
                    .collect()
            })
            .collect();
        (rows, weights)
    }
}

/// The curve cut to `[a, b]` (clamped to its domain), or itself when that is all of it.
fn segment_or_whole(c: &Nurbs<3>, a: f64, b: f64) -> Result<Nurbs<3>, NurbsError> {
    let (lo, hi) = c.domain();
    if a <= lo && b >= hi {
        return Ok(c.clone());
    }
    c.segment(a.max(lo), b.min(hi))
}

fn breaks(knots: &[f64], (lo, hi): (f64, f64)) -> Vec<f64> {
    let mut out = vec![lo];
    for k in knots {
        if *k > lo && *k < hi && out.last().is_some_and(|l| k > l) {
            out.push(*k);
        }
    }
    out.push(hi);
    out
}

fn domain(knots: &[f64], p: usize, count: usize) -> (f64, f64) {
    (
        knots.get(p).copied().unwrap_or(0.0),
        knots.get(count).copied().unwrap_or(1.0),
    )
}

fn clamp(t: f64, (lo, hi): (f64, f64)) -> f64 {
    if t.is_nan() { lo } else { t.clamp(lo, hi) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3::new(x, y, z)
    }

    /// A biquadratic Bezier patch over `[0, 1]²`: heights 0 at the border control points
    /// and `h` at the middle one.
    fn dome(h: f64) -> BSplineSurface3 {
        let k = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let net = (0..3)
            .map(|i| {
                (0..3)
                    .map(|j| {
                        let z = if i == 1 && j == 1 { h } else { 0.0 };
                        p(5.0 * i as f64, 5.0 * j as f64, z)
                    })
                    .collect()
            })
            .collect();
        BSplineSurface3::new(2, 2, k.clone(), k, net, None).unwrap()
    }

    #[test]
    fn a_biquadratic_patch_evaluates_to_its_bernstein_sums() {
        // Bernstein weights of degree 2 at t: (1-t)², 2t(1-t), t². At (½, ½) the middle
        // control point carries (2·½·½)² = ¼, so z = h / 4; x = 10 · ½ = 5 (the control x
        // are 0, 5, 10, linear precision). At (¼, ½): x = 10 · ¼ = 2.5, z = h · 2·¼·¾ ·
        // ½ = 3h/16.
        let s = dome(8.0);
        let c = s.point_at(0.5, 0.5);
        assert!(c.distance(p(5.0, 5.0, 2.0)) < 1e-12, "{c:?}");
        let q = s.point_at(0.25, 0.5);
        assert!(q.distance(p(2.5, 5.0, 1.5)) < 1e-12, "{q:?}");
        // Corners interpolate the corner control points.
        assert!(s.point_at(1.0, 0.0).distance(p(10.0, 0.0, 0.0)) < 1e-12);
        // S_u at the centre: x' = 10; z = h B1(u) B1(v), B1' (½) = 2 - 4·½ = 0.
        let (_, su, sv) = s.derivatives(0.5, 0.5);
        assert!((su - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-12, "{su:?}");
        assert!((sv - Vec3::new(0.0, 10.0, 0.0)).length() < 1e-12, "{sv:?}");
    }

    #[test]
    fn a_rational_patch_is_an_exact_quarter_cylinder() {
        // The rational quadratic (1,0), (1,1), (0,1) with weights 1, √2/2, 1 is a quarter
        // of the unit circle; its middle (u = ½) is (√2/2, √2/2). Times radius 10 and
        // swept linearly over z in [0, 5].
        let r = 10.0;
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let ring = [p(r, 0.0, 0.0), p(r, r, 0.0), p(0.0, r, 0.0)];
        let net = ring.iter().map(|c| vec![*c, p(c.x, c.y, 5.0)]).collect();
        let weights = vec![vec![1.0, 1.0], vec![h, h], vec![1.0, 1.0]];
        let s = BSplineSurface3::new(
            2,
            1,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            net,
            Some(weights),
        )
        .unwrap();
        assert!(s.is_rational());
        let m = s.point_at(0.5, 0.4);
        assert!(m.distance(p(r * h, r * h, 2.0)) < 1e-12, "{m:?}");
        for k in 0..=10 {
            let q = s.point_at(k as f64 / 10.0, 1.0);
            assert!((q.x.hypot(q.y) - r).abs() < 1e-12);
            assert!((q.z - 5.0).abs() < 1e-12);
        }
        // The normal S_u × S_v points away from the axis at the middle.
        let (pt, su, sv) = s.derivatives(0.5, 0.5);
        let n = su.cross(sv).normalized().unwrap();
        let out = Vec3::new(pt.x, pt.y, 0.0).normalized().unwrap();
        assert!((n - out).length() < 1e-12, "{n:?}");
    }

    #[test]
    fn second_derivatives_of_the_dome_are_its_bernstein_derivatives() {
        // z = h B1(u) B1(v) with B1(t) = 2t(1 - t): B1' = 2 - 4t, B1'' = -4. At (½, ½):
        // z_uu = h (-4)(½) = -2h, z_uv = h B1'(½)² = 0, z_vv = -2h; x = 10u and y = 10v
        // have no second derivatives. At (¼, ¾): z_uv = h (2 - 1)(2 - 3) = -h.
        let h = 8.0;
        let s = dome(h);
        let d = s.derivatives2(0.5, 0.5);
        assert!(d.point.distance(p(5.0, 5.0, 2.0)) < 1e-12);
        assert!((d.suu - Vec3::new(0.0, 0.0, -2.0 * h)).length() < 1e-12);
        assert!(d.suv.length() < 1e-12);
        assert!((d.svv - Vec3::new(0.0, 0.0, -2.0 * h)).length() < 1e-12);
        let d = s.derivatives2(0.25, 0.75);
        assert!((d.suv - Vec3::new(0.0, 0.0, -h)).length() < 1e-12, "{d:?}");
        // A rational patch: second derivatives match central differences of the first.
        let r = 10.0;
        let w = std::f64::consts::FRAC_1_SQRT_2;
        let ring = [p(r, 0.0, 0.0), p(r, r, 0.0), p(0.0, r, 0.0)];
        let net = ring
            .iter()
            .map(|c| vec![*c, p(c.x, c.y, 5.0), p(c.x + 1.0, c.y, 9.0)])
            .collect();
        let weights = vec![vec![1.0, 2.0, 1.0], vec![w, 0.5, w], vec![1.0, 1.5, 1.0]];
        let k = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let s = BSplineSurface3::new(2, 2, k.clone(), k, net, Some(weights)).unwrap();
        let e = 1e-6;
        for (u, v) in [(0.3, 0.4), (0.7, 0.2), (0.5, 0.9)] {
            let d = s.derivatives2(u, v);
            let (_, su, sv) = s.derivatives(u, v);
            assert!((d.su - su).length() < 1e-12 && (d.sv - sv).length() < 1e-12);
            let (_, su_p, sv_p) = s.derivatives(u + e, v);
            let (_, su_m, sv_m) = s.derivatives(u - e, v);
            let (_, _, sv_vp) = s.derivatives(u, v + e);
            let (_, _, sv_vm) = s.derivatives(u, v - e);
            assert!(((su_p - su_m) * (0.5 / e) - d.suu).length() < 1e-5);
            assert!(((sv_p - sv_m) * (0.5 / e) - d.suv).length() < 1e-5);
            assert!(((sv_vp - sv_vm) * (0.5 / e) - d.svv).length() < 1e-5);
        }
    }

    #[test]
    fn sub_patches_transposes_and_reversals_are_the_same_surface() {
        let s = dome(8.0);
        // The sub-patch over [¼, ¾] x [0, ½] passes through the same points.
        let sub = s.sub_patch(0.25, 0.75, 0.0, 0.5).unwrap();
        assert_eq!(sub.domain_u(), (0.25, 0.75));
        assert_eq!(sub.domain_v(), (0.0, 0.5));
        for (u, v) in [(0.25, 0.0), (0.5, 0.25), (0.6, 0.5), (0.75, 0.1)] {
            assert!(sub.point_at(u, v).distance(s.point_at(u, v)) < 1e-12);
        }
        // Its control box bounds that part: z in [0, h/4] there (the peak h/4 at the
        // centre), and the box is far tighter than the whole net's [0, 8].
        let (lo, hi) = sub.hull_box();
        assert!(lo.z >= -1e-12 && hi.z <= 8.0 * 0.75 + 1e-12 && hi.z >= 2.0 - 1e-12);
        assert!(lo.x >= 2.5 - 1e-12 && hi.x <= 7.5 + 1e-12 && hi.y <= 5.0 + 1e-12);
        // Transposed: S'(s, t) = S(t, s); the normal flips.
        let t = s.transposed();
        let (pa, sua, sva) = s.derivatives(0.2, 0.7);
        let (pb, sub_, svb) = t.derivatives(0.7, 0.2);
        assert!(pa.distance(pb) < 1e-12);
        assert!((sua.cross(sva) + sub_.cross(svb)).length() < 1e-9);
        // Reversed in u: S'(u, v) = S(1 - u, v); the normal flips.
        let r = s.reversed_u();
        let (pr, sur, svr) = r.derivatives(0.8, 0.7);
        assert!(pr.distance(pa) < 1e-12);
        assert!((sua.cross(sva) + sur.cross(svr)).length() < 1e-9);
        assert_eq!(s.breaks_u(), vec![0.0, 1.0]);
        // Moving the net moves the surface.
        let m = Transform::translation(Vec3::new(1.0, 2.0, 3.0)).unwrap();
        let moved = s.transformed(&m).unwrap();
        assert!(moved.point_at(0.5, 0.5).distance(p(6.0, 7.0, 5.0)) < 1e-12);
        let far = Transform::translation(Vec3::new(1e7, 0.0, 0.0)).unwrap();
        assert!(s.transformed(&far).is_err());
        let (net, w) = s.net();
        let again = BSplineSurface3::new(
            2,
            2,
            s.knots_u().to_vec(),
            s.knots_v().to_vec(),
            net,
            Some(w),
        )
        .unwrap();
        assert_eq!(again, s);
    }

    #[test]
    fn hostile_surfaces_are_refused() {
        let k = vec![0.0, 0.0, 1.0, 1.0];
        let net = || {
            vec![
                vec![p(0., 0., 0.), p(1., 0., 0.)],
                vec![p(0., 1., 0.), p(1., 1., 0.)],
            ]
        };
        assert!(BSplineSurface3::new(1, 1, k.clone(), k.clone(), net(), None).is_ok());
        // Ragged net, empty net, wrong knot counts, bad weights, huge points, degree 0.
        let ragged = vec![vec![p(0., 0., 0.)], vec![p(0., 1., 0.), p(1., 1., 0.)]];
        assert!(BSplineSurface3::new(1, 1, k.clone(), k.clone(), ragged, None).is_err());
        assert!(BSplineSurface3::new(1, 1, k.clone(), k.clone(), vec![], None).is_err());
        assert!(BSplineSurface3::new(1, 1, vec![0.0, 1.0], k.clone(), net(), None).is_err());
        let w = Some(vec![vec![1.0, 0.0], vec![1.0, 1.0]]);
        assert!(BSplineSurface3::new(1, 1, k.clone(), k.clone(), net(), w).is_err());
        let w = Some(vec![vec![1.0, f64::NAN], vec![1.0, 1.0]]);
        assert!(BSplineSurface3::new(1, 1, k.clone(), k.clone(), net(), w).is_err());
        let mut far = net();
        far[0][0] = p(1e300, 0.0, 0.0);
        assert!(BSplineSurface3::new(1, 1, k.clone(), k.clone(), far, None).is_err());
        assert!(BSplineSurface3::new(0, 1, k.clone(), k.clone(), net(), None).is_err());
        let rev = vec![1.0, 1.0, 0.0, 0.0];
        assert!(BSplineSurface3::new(1, 1, rev, k.clone(), net(), None).is_err());
        // Evaluation outside the domain and at NaN clamps instead of failing.
        let s = BSplineSurface3::new(1, 1, k.clone(), k, net(), None).unwrap();
        // u = NaN is the domain start, v = 7 the end: P_01.
        assert!(s.point_at(f64::NAN, 7.0).distance(p(1.0, 0.0, 0.0)) < 1e-12);
    }
}
