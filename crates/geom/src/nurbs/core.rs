//! The dimension-generic rational B-spline curve behind [`BSplineCurve2`](super::BSplineCurve2)
//! and [`BSplineCurve3`](super::BSplineCurve3).
//!
//! Control points are stored as Cartesian points with separate weights; the algorithms
//! work in homogeneous coordinates `Pw = (w P, w)` (The NURBS Book, §4.2) and project back.

use super::basis::{binomial, ders_basis_funs, find_span};
use super::{
    KNOT_TOLERANCE, MAX_CONTROL_POINTS, MAX_DEGREE, MAX_KNOT, MAX_WEIGHT, MIN_WEIGHT, NurbsError,
};
use crate::MAX_COORDINATE;

/// Largest number of Newton steps one closest-point search takes (§6.1).
const NEWTON_STEPS: usize = 64;
/// Most intervals the subdivision fallback of the closest-point search visits.
const SUBDIVISION_BUDGET: usize = 4096;
/// Most panels one arc-length integral may use.
const MAX_LENGTH_PANELS: usize = 20_000;
/// Relative accuracy of arc lengths (adaptive Gauss-Legendre, see [`integrate`]).
pub const LENGTH_RELATIVE_TOLERANCE: f64 = 1e-12;

pub(super) type Arr<const D: usize> = [f64; D];

/// A rational B-spline curve in `D` dimensions: degree `p`, knots `U` (`len = n + p + 2`),
/// control points `P_0..P_n` and weights `w_0..w_n`. Valid by construction.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Nurbs<const D: usize> {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub points: Vec<Arr<D>>,
    pub weights: Vec<f64>,
}

fn invalid(msg: impl Into<String>) -> NurbsError {
    NurbsError::InvalidInput(msg.into())
}

pub(super) fn add<const D: usize>(a: Arr<D>, b: Arr<D>) -> Arr<D> {
    let mut o = a;
    for (x, y) in o.iter_mut().zip(b) {
        *x += y;
    }
    o
}
pub(super) fn sub<const D: usize>(a: Arr<D>, b: Arr<D>) -> Arr<D> {
    let mut o = a;
    for (x, y) in o.iter_mut().zip(b) {
        *x -= y;
    }
    o
}
pub(super) fn scale<const D: usize>(a: Arr<D>, s: f64) -> Arr<D> {
    let mut o = a;
    for x in o.iter_mut() {
        *x *= s;
    }
    o
}
pub(super) fn dot<const D: usize>(a: Arr<D>, b: Arr<D>) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
pub(super) fn norm<const D: usize>(a: Arr<D>) -> f64 {
    dot(a, a).sqrt()
}

/// 5-point Gauss-Legendre nodes and weights on `[-1, 1]` (Abramowitz and Stegun, 25.4.30).
const GAUSS5: [(f64, f64); 5] = [
    (0.0, 0.568_888_888_888_888_9),
    (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
    (0.906_179_845_938_664, 0.236_926_885_056_189_1),
];

fn gauss5(f: &dyn Fn(f64) -> f64, a: f64, b: f64) -> f64 {
    let (m, h) = (0.5 * (a + b), 0.5 * (b - a));
    GAUSS5.iter().map(|(x, w)| w * f(m + h * x)).sum::<f64>() * h
}

/// `∫_a^b f` by adaptive Gauss-Legendre quadrature: each panel's 5-point estimate is
/// compared with the sum over its halves and split until they agree to
/// `rel` of the integral's scale or the absolute floor `abs` (shared out by width; the
/// floor stops an integrand that is zero up to rounding from splitting forever). Returns
/// `None` when the panel budget runs out or a value is not finite.
pub(super) fn integrate(f: &dyn Fn(f64) -> f64, a: f64, b: f64, rel: f64, abs: f64) -> Option<f64> {
    if a == b {
        return Some(0.0);
    }
    let width = b - a;
    let mut scale = 0.0;
    for i in 0..8 {
        let x0 = a + width * i as f64 / 8.0;
        let x1 = a + width * (i + 1) as f64 / 8.0;
        scale += gauss5(&|t| f(t).abs(), x0, x1).abs();
    }
    if !scale.is_finite() {
        return None;
    }
    let tol = (rel * scale).max(abs).max(f64::MIN_POSITIVE);
    let mut stack = vec![(a, b, gauss5(f, a, b))];
    let mut total = 0.0;
    let mut panels = 0usize;
    while let Some((x0, x1, whole)) = stack.pop() {
        panels += 1;
        if panels > MAX_LENGTH_PANELS {
            return None;
        }
        let m = 0.5 * (x0 + x1);
        let (l, r) = (gauss5(f, x0, m), gauss5(f, m, x1));
        if (l + r - whole).abs() <= tol * ((x1 - x0) / width).abs() || m <= x0 || m >= x1 {
            total += l + r;
        } else {
            stack.push((x0, m, l));
            stack.push((m, x1, r));
        }
    }
    total.is_finite().then_some(total)
}

impl<const D: usize> Nurbs<D> {
    /// Validates and builds a curve. Rules: degree `1..=MAX_DEGREE`; `p + 1 ..=
    /// MAX_CONTROL_POINTS` control points, finite and within the working range; as many
    /// weights, each finite in `[MIN_WEIGHT, MAX_WEIGHT]`; `n + p + 2` finite knots,
    /// non-decreasing, no interior knot repeated more than `p` times nor an end knot more
    /// than `p + 1` times, and a non-empty domain `[U_p, U_{n+1}]`.
    pub fn new(
        degree: usize,
        knots: Vec<f64>,
        points: Vec<Arr<D>>,
        weights: Vec<f64>,
    ) -> Result<Self, NurbsError> {
        if !(1..=MAX_DEGREE).contains(&degree) {
            return Err(invalid(format!(
                "the degree must be between 1 and {MAX_DEGREE}, not {degree}"
            )));
        }
        let count = points.len();
        if count < degree + 1 {
            return Err(invalid(format!(
                "a degree {degree} curve needs at least {} control points, not {count}",
                degree + 1
            )));
        }
        if count > MAX_CONTROL_POINTS {
            return Err(invalid(format!(
                "a curve may have at most {MAX_CONTROL_POINTS} control points"
            )));
        }
        if points
            .iter()
            .any(|p| p.iter().any(|x| !x.is_finite() || x.abs() > MAX_COORDINATE))
        {
            return Err(invalid(
                "control points must be finite and inside the model's working range",
            ));
        }
        if weights.len() != count {
            return Err(invalid(format!(
                "there must be one weight per control point ({count}), not {}",
                weights.len()
            )));
        }
        if weights
            .iter()
            .any(|w| !(w.is_finite() && (MIN_WEIGHT..=MAX_WEIGHT).contains(w)))
        {
            return Err(invalid(format!(
                "weights must be finite numbers between {MIN_WEIGHT} and {MAX_WEIGHT}"
            )));
        }
        if knots.len() != count + degree + 1 {
            return Err(invalid(format!(
                "a degree {degree} curve with {count} control points needs {} knots, not {}",
                count + degree + 1,
                knots.len()
            )));
        }
        if knots
            .iter()
            .any(|k| !(k.is_finite() && k.abs() <= MAX_KNOT))
        {
            return Err(invalid(format!(
                "knots must be finite numbers of magnitude at most {MAX_KNOT}"
            )));
        }
        if knots.windows(2).any(|w| w[1] < w[0]) {
            return Err(invalid("knots must not decrease"));
        }
        let (lo, hi) = (knots[degree], knots[count]);
        if hi - lo <= KNOT_TOLERANCE * (1.0 + lo.abs().max(hi.abs())) {
            return Err(invalid("the knots must leave a non-empty parameter domain"));
        }
        // Multiplicities: at most p inside the domain, p + 1 at its ends.
        let mut i = 0;
        while i < knots.len() {
            let mut j = i;
            while j + 1 < knots.len() && knots[j + 1] == knots[i] {
                j += 1;
            }
            let mult = j - i + 1;
            let k = knots[i];
            let interior = k > lo && k < hi;
            if (interior && mult > degree) || mult > degree + 1 {
                return Err(invalid(format!(
                    "the knot {k} is repeated {mult} times; at most {} is allowed there",
                    if interior { degree } else { degree + 1 }
                )));
            }
            i = j + 1;
        }
        Ok(Nurbs {
            degree,
            knots,
            points,
            weights,
        })
    }

    /// Index of the last control point.
    pub fn n(&self) -> usize {
        self.points.len().saturating_sub(1)
    }

    /// The parameter domain `[U_p, U_{n+1}]`.
    pub fn domain(&self) -> (f64, f64) {
        let p = self.degree;
        (
            self.knots.get(p).copied().unwrap_or(0.0),
            self.knots.get(self.n() + 1).copied().unwrap_or(1.0),
        )
    }

    pub fn clamp(&self, u: f64) -> f64 {
        let (lo, hi) = self.domain();
        if u.is_nan() { lo } else { u.clamp(lo, hi) }
    }

    pub fn is_rational(&self) -> bool {
        let w0 = self.weights.first().copied().unwrap_or(1.0);
        self.weights.iter().any(|w| (w - w0).abs() > 1e-15 * w0)
    }

    fn weighted(&self, i: usize) -> (Arr<D>, f64) {
        let w = self.weights.get(i).copied().unwrap_or(1.0);
        let p = self.points.get(i).copied().unwrap_or([0.0; D]);
        (scale(p, w), w)
    }

    /// Derivatives of the homogeneous curve `Cw(u)` up to order `d` (A3.2,
    /// *CurveDerivsAlg1*, applied to `Pw`), as (`A^(k)`, `w^(k)`).
    fn homogeneous_derivs(&self, u: f64, d: usize) -> Vec<(Arr<D>, f64)> {
        let p = self.degree;
        let u = self.clamp(u);
        let span = find_span(self.n(), p, u, &self.knots);
        let ders = ders_basis_funs(span, u, p, d, &self.knots);
        let mut out = vec![([0.0; D], 0.0); d + 1];
        for (k, row) in ders.iter().enumerate().take(d.min(p) + 1) {
            let (mut a, mut w) = ([0.0; D], 0.0);
            for (j, nkj) in row.iter().enumerate() {
                let (pw, ww) = self.weighted(span + j - p);
                a = add(a, scale(pw, *nkj));
                w += nkj * ww;
            }
            out[k] = (a, w);
        }
        out
    }

    /// `C(u), C'(u), .., C^(d)(u)` (A4.2, *RatCurveDerivs*, eq. 4.8):
    /// `C^(k) = (A^(k) - Σ_{i=1..k} C(k,i) w^(i) C^(k-i)) / w`. `u` is clamped to the domain.
    pub fn derivs(&self, u: f64, d: usize) -> Vec<Arr<D>> {
        let h = self.homogeneous_derivs(u, d);
        let w0 = h.first().map_or(1.0, |x| x.1);
        let mut ck: Vec<Arr<D>> = Vec::with_capacity(d + 1);
        for k in 0..=d {
            let mut v = h[k].0;
            for i in 1..=k {
                v = sub(v, scale(ck[k - i], binomial(k, i) * h[i].1));
            }
            ck.push(scale(v, 1.0 / w0));
        }
        ck
    }

    /// `C(u)` (A4.1, *CurvePoint*).
    pub fn point(&self, u: f64) -> Arr<D> {
        self.derivs(u, 0).first().copied().unwrap_or([0.0; D])
    }

    /// How many knots equal `u` exactly.
    fn multiplicity(&self, u: f64) -> usize {
        self.knots.iter().filter(|k| **k == u).count()
    }

    /// `u` moved onto a knot when it is within the knot tolerance of one, so splitting
    /// never makes slivers.
    fn snapped(&self, u: f64) -> f64 {
        let (lo, hi) = self.domain();
        let tol = KNOT_TOLERANCE * (hi - lo).max(1.0);
        self.knots
            .iter()
            .copied()
            .find(|k| (k - u).abs() <= tol)
            .unwrap_or(u)
    }

    /// The curve with the knot `u` inserted `r` more times (A5.1, *CurveKnotIns*, in
    /// homogeneous coordinates); the curve itself does not change. `u` must be inside the
    /// domain, below its end, and its multiplicity may reach at most `p`.
    pub fn insert_knot(&self, u: f64, r: usize) -> Result<Self, NurbsError> {
        let p = self.degree;
        let (lo, hi) = self.domain();
        if !(u.is_finite() && u >= lo && u < hi) {
            return Err(invalid(format!(
                "a knot can only be inserted inside the domain [{lo}, {hi})"
            )));
        }
        if r == 0 {
            return Ok(self.clone());
        }
        let s = self.multiplicity(u);
        if r + s > p {
            return Err(invalid(format!(
                "the knot {u} would be repeated {} times; at most {p} is allowed",
                r + s
            )));
        }
        if self.points.len() + r > MAX_CONTROL_POINTS {
            return Err(invalid(format!(
                "a curve may have at most {MAX_CONTROL_POINTS} control points"
            )));
        }
        let np = self.n();
        let mp = np + p + 1;
        let k = find_span(np, p, u, &self.knots);
        let up = &self.knots;
        let mut uq = vec![0.0; mp + r + 1];
        uq[..=k].copy_from_slice(&up[..=k]);
        for i in 1..=r {
            uq[k + i] = u;
        }
        uq[(k + 1 + r)..=(mp + r)].copy_from_slice(&up[(k + 1)..=mp]);
        let pw: Vec<(Arr<D>, f64)> = (0..=np).map(|i| self.weighted(i)).collect();
        let mut qw = vec![([0.0; D], 0.0); np + r + 1];
        qw[..=(k - p)].copy_from_slice(&pw[..=(k - p)]);
        qw[(k - s + r)..=(np + r)].copy_from_slice(&pw[(k - s)..=np]);
        let mut rw: Vec<(Arr<D>, f64)> = (0..=(p - s)).map(|i| pw[k - p + i]).collect();
        let mut l = k - p;
        for j in 1..=r {
            l = k - p + j;
            for i in 0..=(p - j - s) {
                let den = up[i + k + 1] - up[l + i];
                let alpha = if den != 0.0 {
                    (u - up[l + i]) / den
                } else {
                    0.0
                };
                let (a, wa) = rw[i + 1];
                let (b, wb) = rw[i];
                rw[i] = (
                    add(scale(a, alpha), scale(b, 1.0 - alpha)),
                    alpha * wa + (1.0 - alpha) * wb,
                );
            }
            qw[l] = rw[0];
            qw[k + r - j - s] = rw[p - j - s];
        }
        if l + 1 < k - s {
            qw[(l + 1)..(k - s)].copy_from_slice(&rw[1..(k - s - l)]);
        }
        Ok(from_homogeneous(p, uq, &qw))
    }

    /// The part of the curve on `[u, end of domain]`, clamped at `u` (the knot `u` raised
    /// to multiplicity `p`, the control points before it dropped).
    pub fn cut_from(&self, u: f64) -> Result<Self, NurbsError> {
        let p = self.degree;
        let (lo, hi) = self.domain();
        let u = self.snapped(u);
        if !(u.is_finite() && u >= lo && u < hi) {
            return Err(invalid(format!(
                "the cut must be inside the domain [{lo}, {hi})"
            )));
        }
        let s = self.multiplicity(u);
        let q = if s < p {
            self.insert_knot(u, p - s)?
        } else {
            self.clone()
        };
        // The last knot equal to u; the curve passes through Q[last - p] there.
        let last = q
            .knots
            .iter()
            .rposition(|k| *k == u)
            .ok_or_else(|| invalid("knot insertion lost the knot"))?;
        let first_point = last
            .checked_sub(p)
            .ok_or_else(|| invalid("the cut knot is too early"))?;
        let mut knots = vec![u; p + 1];
        knots.extend_from_slice(q.knots.get(last + 1..).unwrap_or(&[]));
        let points = q.points.get(first_point..).unwrap_or(&[]).to_vec();
        let weights = q.weights.get(first_point..).unwrap_or(&[]).to_vec();
        Nurbs::new(p, knots, points, weights)
    }

    /// The same curve run backwards: `C'(u) = C(U_0 + U_m - u)`.
    pub fn reversed(&self) -> Self {
        let a = self.knots.first().copied().unwrap_or(0.0);
        let b = self.knots.last().copied().unwrap_or(1.0);
        self.reversed_about(a + b)
    }

    /// The curve run backwards with `u' = c - u`.
    fn reversed_about(&self, c: f64) -> Self {
        Nurbs {
            degree: self.degree,
            knots: self.knots.iter().rev().map(|k| c - k).collect(),
            points: self.points.iter().rev().copied().collect(),
            weights: self.weights.iter().rev().copied().collect(),
        }
    }

    /// The parameter of the reversed curve at the point with parameter `u` here.
    pub fn reversed_parameter(&self, u: f64) -> f64 {
        let a = self.knots.first().copied().unwrap_or(0.0);
        let b = self.knots.last().copied().unwrap_or(1.0);
        a + b - u
    }

    /// The part of the curve on `[a, b]` (inside the domain, `a < b`), clamped at both
    /// ends, keeping the parameterisation (§5.2: split by knot insertion).
    pub fn segment(&self, a: f64, b: f64) -> Result<Self, NurbsError> {
        let (lo, hi) = self.domain();
        if !(a.is_finite() && b.is_finite() && lo <= a && a < b && b <= hi) {
            return Err(invalid(format!(
                "a segment [{a}, {b}] must be increasing and inside the domain [{lo}, {hi}]"
            )));
        }
        let right = self.cut_from(a)?;
        // Reverse with u' = c - u, cut, and reverse back with the same c.
        let c = right.knots.first().copied().unwrap_or(0.0)
            + right.knots.last().copied().unwrap_or(1.0);
        let cut = right.reversed_about(c).cut_from(c - b)?;
        let mut out = cut.reversed_about(c);
        // The ends are a and b exactly, whatever `c - (c - b)` rounds to.
        let p = out.degree;
        let len = out.knots.len();
        for (i, k) in out.knots.iter_mut().enumerate() {
            if i <= p {
                *k = a;
            } else if i + p + 1 >= len {
                *k = b;
            }
        }
        Ok(out)
    }

    /// Splits the curve at `u`, strictly inside the domain, into the parts before and after.
    pub fn split(&self, u: f64) -> Result<(Self, Self), NurbsError> {
        let (lo, hi) = self.domain();
        let u = self.snapped(u);
        if !(u > lo && u < hi) {
            return Err(invalid(format!(
                "a split parameter must lie strictly inside the domain ({lo}, {hi})"
            )));
        }
        Ok((self.segment(lo, u)?, self.segment(u, hi)?))
    }

    /// The box around the control points: it contains the curve, because a NURBS curve
    /// with positive weights lies in the convex hull of its control points (§4.2, the
    /// strong convex hull property).
    pub fn hull_box(&self) -> (Arr<D>, Arr<D>) {
        let mut lo = [f64::INFINITY; D];
        let mut hi = [f64::NEG_INFINITY; D];
        for p in &self.points {
            for k in 0..D {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        (lo, hi)
    }

    /// The diagonal of the control-point box, at least 1 (mm): the curve's scale.
    pub fn size(&self) -> f64 {
        let (a, b) = self.hull_box();
        norm(sub(b, a)).max(1.0)
    }

    /// The distinct knots inside the domain, with its ends: the spans the curve is
    /// polynomial (rational) on.
    pub fn breaks(&self) -> Vec<f64> {
        let (lo, hi) = self.domain();
        let mut out = vec![lo];
        for k in &self.knots {
            if *k > lo && *k < hi && out.last().is_some_and(|l| k > l) {
                out.push(*k);
            }
        }
        out.push(hi);
        out
    }

    /// The arc length over `[a, b]` (clamped to the domain), by adaptive Gauss-Legendre
    /// quadrature of `|C'(u)|` on each knot span.
    pub fn length(&self, a: f64, b: f64) -> Result<f64, NurbsError> {
        let (a, b) = (self.clamp(a), self.clamp(b));
        let (a, b, sign) = if a <= b { (a, b, 1.0) } else { (b, a, -1.0) };
        let speed = |u: f64| self.derivs(u, 1).get(1).map_or(0.0, |d| norm(*d));
        let mut total = 0.0;
        let br = self.breaks();
        let size = self.size();
        for w in br.windows(2) {
            let (s, e) = (w[0].max(a), w[1].min(b));
            if e > s {
                total += integrate(&speed, s, e, LENGTH_RELATIVE_TOLERANCE, 1e-15 * size)
                    .ok_or_else(|| {
                        NurbsError::NoConvergence("the arc length integral did not converge".into())
                    })?;
            }
        }
        Ok(total * sign)
    }

    /// Newton iteration for the closest point (§6.1, eq. 6.3): with
    /// `f(u) = C'(u) · (C(u) - P)`, `u ← u - f / f'`, `f' = C'' · (C - P) + |C'|²`, until
    /// the point coincides, the zero-cosine test holds or the step stops moving.
    fn newton(&self, p: Arr<D>, u0: f64, eps: f64) -> Option<f64> {
        let mut u = self.clamp(u0);
        for _ in 0..NEWTON_STEPS {
            let d = self.derivs(u, 2);
            let (c, c1, c2) = (d[0], d[1], d[2]);
            let diff = sub(c, p);
            let dist = norm(diff);
            if dist <= eps {
                return Some(u);
            }
            let f = dot(c1, diff);
            let speed = norm(c1);
            if speed > 0.0 && (f / (speed * dist)).abs() <= 1e-12 {
                return Some(u);
            }
            let fp = dot(c2, diff) + speed * speed;
            if !(fp.is_finite() && fp > 0.0) {
                return None;
            }
            let next = self.clamp(u - f / fp);
            if ((next - u) * speed).abs() <= eps {
                return Some(next);
            }
            u = next;
        }
        None
    }

    /// The parameter and distance of the point of the curve nearest to `p` (point
    /// inversion, §6.1). Seeds: samples on every knot span; each of the best few is
    /// refined by [`Nurbs::newton`]. If none converges the domain is subdivided instead,
    /// discarding pieces whose control-point box is farther than the best point so far,
    /// until the pieces are smaller than the tolerance.
    pub fn closest(&self, p: Arr<D>) -> (f64, f64) {
        let (lo, hi) = self.domain();
        let br = self.breaks();
        let per_span = 2 * (self.degree + 1);
        let mut samples: Vec<(f64, f64)> = Vec::new();
        for w in br.windows(2) {
            for i in 0..=per_span {
                let u = w[0] + (w[1] - w[0]) * i as f64 / per_span as f64;
                samples.push((u, norm(sub(self.point(u), p))));
            }
        }
        samples.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut best = samples.first().copied().unwrap_or((lo, f64::INFINITY));
        let size = self.size();
        let eps = 1e-13 * size;
        let mut converged = false;
        for (u0, _) in samples.iter().take(4) {
            if let Some(u) = self.newton(p, *u0, eps) {
                converged = true;
                let d = norm(sub(self.point(u), p));
                if d < best.1 {
                    best = (u, d);
                }
            }
        }
        if converged {
            return best;
        }
        // Subdivision fallback.
        let mut stack = vec![(lo, hi)];
        let mut visited = 0usize;
        while let Some((a, b)) = stack.pop() {
            visited += 1;
            if visited > SUBDIVISION_BUDGET {
                break;
            }
            let Ok(piece) = self.segment(a, b) else {
                continue;
            };
            let (bl, bh) = piece.hull_box();
            let mut gap = [0.0; D];
            for k in 0..D {
                gap[k] = (bl[k] - p[k]).max(p[k] - bh[k]).max(0.0);
            }
            if norm(gap) >= best.1 {
                continue;
            }
            let m = 0.5 * (a + b);
            let dm = norm(sub(self.point(m), p));
            if dm < best.1 {
                best = (m, dm);
            }
            if norm(sub(bh, bl)) > eps && m > a && m < b {
                stack.push((a, m));
                stack.push((m, b));
            }
        }
        best
    }

    /// The same curve with every control point mapped by `f` (an affine map keeps a NURBS
    /// curve a NURBS curve with the same weights and knots, §4.2, affine invariance).
    pub fn map<const E: usize>(
        &self,
        f: impl Fn(Arr<D>) -> Arr<E>,
    ) -> Result<Nurbs<E>, NurbsError> {
        Nurbs::new(
            self.degree,
            self.knots.clone(),
            self.points.iter().map(|p| f(*p)).collect(),
            self.weights.clone(),
        )
    }
}

/// A curve from homogeneous control points (`(w P, w)`), projected back.
fn from_homogeneous<const D: usize>(p: usize, knots: Vec<f64>, qw: &[(Arr<D>, f64)]) -> Nurbs<D> {
    let points = qw
        .iter()
        .map(|(a, w)| if *w != 0.0 { scale(*a, 1.0 / w) } else { *a })
        .collect();
    let weights = qw.iter().map(|(_, w)| *w).collect();
    Nurbs {
        degree: p,
        knots,
        points,
        weights,
    }
}
