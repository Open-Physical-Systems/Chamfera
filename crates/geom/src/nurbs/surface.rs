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
//! This is the geometry an importer reads (STEP `B_SPLINE_SURFACE`); the kernel does not
//! hold free-form faces yet.

use super::basis::{ders_basis_funs, find_span};
use super::core::Nurbs;
use super::{MAX_WEIGHT, MIN_WEIGHT, NurbsError};
use crate::{Point3, Vec3};

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
