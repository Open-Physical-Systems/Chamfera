//! Making curves: global interpolation through fit points (The NURBS Book, §9.2) and
//! control-point splines with uniform knots.

use super::basis::{basis_funs, ders_basis_funs, find_span};
use super::core::{Arr, Nurbs, norm, scale, sub};
use super::{MAX_FIT_POINTS, NurbsError};
use crate::LINEAR_TOLERANCE;

/// The degree used for splines when the caller does not choose one: cubic.
pub const DEFAULT_SPLINE_DEGREE: usize = 3;

fn invalid(msg: impl Into<String>) -> NurbsError {
    NurbsError::InvalidInput(msg.into())
}

/// Solves `A X = B` for `X` (`A` is `n × n`, `B` is `n × D`) by Gaussian elimination with
/// partial pivoting (Golub and Van Loan, *Matrix Computations*, §3.4). The interpolation
/// matrices of §9.2 are totally positive and banded, so this is stable; a pivot below
/// `1e-14` of the row scale means the system is singular (e.g. repeated parameters).
fn solve<const D: usize>(
    mut a: Vec<Vec<f64>>,
    mut b: Vec<Arr<D>>,
) -> Result<Vec<Arr<D>>, NurbsError> {
    let n = a.len();
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))
            .unwrap_or(col);
        let scale_row = a[pivot].iter().fold(0.0f64, |m, x| m.max(x.abs()));
        let big_enough = a[pivot][col].abs() > 1e-14 * scale_row.max(f64::MIN_POSITIVE);
        if !big_enough {
            return Err(NurbsError::Singular(
                "the interpolation system is singular: are two fit points the same?".into(),
            ));
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..n {
            let f = a[row][col] / a[col][col];
            if f == 0.0 {
                continue;
            }
            let pivot_row = a[col].clone();
            for (x, y) in a[row].iter_mut().zip(&pivot_row).skip(col) {
                *x -= f * y;
            }
            let bc = b[col];
            b[row] = sub(b[row], scale(bc, f));
        }
    }
    let mut x = vec![[0.0; D]; n];
    for row in (0..n).rev() {
        let mut v = b[row];
        for k in row + 1..n {
            v = sub(v, scale(x[k], a[row][k]));
        }
        x[row] = scale(v, 1.0 / a[row][row]);
    }
    if x.iter().any(|p| p.iter().any(|c| !c.is_finite())) {
        return Err(NurbsError::Singular(
            "the interpolation system has no finite solution".into(),
        ));
    }
    Ok(x)
}

/// Chord-length parameters `ū_k` on `[0, 1]` (eq. 9.5): `ū_0 = 0`, `ū_n = 1`,
/// `ū_k = ū_{k-1} + |Q_k - Q_{k-1}| / d` with `d` the total chord length.
fn chord_parameters<const D: usize>(q: &[Arr<D>]) -> Result<(Vec<f64>, f64), NurbsError> {
    let chords: Vec<f64> = q.windows(2).map(|w| norm(sub(w[1], w[0]))).collect();
    if chords.iter().any(|c| *c <= LINEAR_TOLERANCE) {
        return Err(invalid(format!(
            "consecutive fit points must be more than {LINEAR_TOLERANCE} mm apart"
        )));
    }
    let d: f64 = chords.iter().sum();
    let mut out = Vec::with_capacity(q.len());
    out.push(0.0);
    let mut acc = 0.0;
    for c in &chords {
        acc += c;
        out.push(acc / d);
    }
    if let Some(last) = out.last_mut() {
        *last = 1.0;
    }
    Ok((out, d))
}

/// Global interpolation through `q` (A9.1 with the end-derivative rows of §9.2.2):
/// chord-length parameters (eq. 9.5), knots by averaging (eq. 9.8; with end derivatives,
/// eq. 9.22, which is averaging over the parameters with the derivative ends repeated),
/// and the linear system `C(ū_k) = Q_k`, `C'(0) = D_0`, `C'(1) = D_n` solved for the
/// control points. End derivatives are scaled by the total chord length `d`, so a unit
/// tangent gives a curve that leaves along it at the natural speed (the advice after
/// eq. 9.21). The degree is `min(degree, unknowns - 1)`.
pub(super) fn interpolate<const D: usize>(
    q: &[Arr<D>],
    degree: usize,
    start: Option<Arr<D>>,
    end: Option<Arr<D>>,
) -> Result<Nurbs<D>, NurbsError> {
    if q.len() < 2 {
        return Err(invalid("a spline needs at least two fit points"));
    }
    if q.len() > MAX_FIT_POINTS {
        return Err(invalid(format!(
            "a spline may have at most {MAX_FIT_POINTS} fit points"
        )));
    }
    let (ubar, d) = chord_parameters(q)?;
    let tangent = |t: Arr<D>| -> Result<Arr<D>, NurbsError> {
        let l = norm(t);
        if !(l.is_finite() && l > 0.0) {
            return Err(invalid("an end tangent must be a finite, non-zero vector"));
        }
        Ok(scale(t, d / l))
    };
    let d0 = start.map(tangent).transpose()?;
    let dn = end.map(tangent).transpose()?;
    let count = q.len() + usize::from(d0.is_some()) + usize::from(dn.is_some());
    let p = degree.min(count - 1).max(1);
    // Parameters with derivative ends repeated, then averaged into knots.
    let mut t = Vec::with_capacity(count);
    if d0.is_some() {
        t.push(0.0);
    }
    t.extend_from_slice(&ubar);
    if dn.is_some() {
        t.push(1.0);
    }
    let mut knots = vec![0.0; p + 1];
    for j in 1..count - p {
        let s: f64 = t[j..j + p].iter().sum();
        knots.push(s / p as f64);
    }
    knots.extend(std::iter::repeat_n(1.0, p + 1));
    let n = count - 1;
    let mut a = Vec::with_capacity(count);
    let mut b = Vec::with_capacity(count);
    let mut row_at = |u: f64, order: usize, value: Arr<D>| {
        let span = find_span(n, p, u, &knots);
        let mut row = vec![0.0; count];
        let vals = if order == 0 {
            basis_funs(span, u, p, &knots)
        } else {
            ders_basis_funs(span, u, p, 1, &knots)
                .get(1)
                .cloned()
                .unwrap_or_default()
        };
        for (j, v) in vals.iter().enumerate() {
            if let Some(slot) = row.get_mut(span + j - p) {
                *slot = *v;
            }
        }
        a.push(row);
        b.push(value);
    };
    for (k, qk) in q.iter().enumerate() {
        row_at(ubar[k], 0, *qk);
        if k == 0
            && let Some(d0) = d0
        {
            row_at(0.0, 1, d0);
        }
    }
    if let Some(dn) = dn {
        row_at(1.0, 1, dn);
    }
    let points = solve(a, b)?;
    let weights = vec![1.0; points.len()];
    let curve = Nurbs::new(p, knots, points, weights)?;
    Ok(curve)
}

/// A closed curve through `q` (first point not repeated): interpolation through
/// `q_0..q_{n-1}, q_0` with the same end derivative at both ends, along `q_1 - q_{n-1}`
/// (the central difference at the closing point), so the curve closes with a continuous
/// tangent.
pub(super) fn interpolate_closed<const D: usize>(
    q: &[Arr<D>],
    degree: usize,
) -> Result<Nurbs<D>, NurbsError> {
    if q.len() < 3 {
        return Err(invalid("a closed spline needs at least three fit points"));
    }
    if q.len() >= MAX_FIT_POINTS {
        return Err(invalid(format!(
            "a spline may have at most {} fit points",
            MAX_FIT_POINTS - 1
        )));
    }
    let mut closed = q.to_vec();
    closed.push(q[0]);
    let dir = sub(q[1], q[q.len() - 1]);
    interpolate(&closed, degree, Some(dir), Some(dir))
}

/// The open control-point spline of `degree` (lowered to fit the point count) on `points`,
/// with clamped uniform knots: it starts at the first point and ends at the last,
/// tangent to the first and last legs of the control polygon (§3.3: a clamped curve
/// interpolates its end control points).
pub(super) fn control_open<const D: usize>(
    points: &[Arr<D>],
    degree: usize,
) -> Result<Nurbs<D>, NurbsError> {
    if points.len() < 2 {
        return Err(invalid("a control-point spline needs at least two points"));
    }
    let p = degree.min(points.len() - 1).max(1);
    let n = points.len() - 1;
    let spans = n - p + 1;
    let mut knots = vec![0.0; p + 1];
    for j in 1..spans {
        knots.push(j as f64 / spans as f64);
    }
    knots.extend(std::iter::repeat_n(1.0, p + 1));
    Nurbs::new(p, knots, points.to_vec(), vec![1.0; points.len()])
}

/// The closed control-point spline on `points` (the polygon closes from the last point to
/// the first): uniform periodic knots with the first `p` points repeated at the end, so
/// the curve closes with `p - 1` continuous derivatives (§12.2, unclamped curves), then
/// clamped at its start by knot insertion so the domain starts and ends at one point.
pub(super) fn control_closed<const D: usize>(
    points: &[Arr<D>],
    degree: usize,
) -> Result<Nurbs<D>, NurbsError> {
    if points.len() < 3 {
        return Err(invalid(
            "a closed control-point spline needs at least three points",
        ));
    }
    let p = degree.min(points.len() - 1).max(1);
    let mut pts = points.to_vec();
    pts.extend_from_slice(&points[..p]);
    let m = pts.len() + p + 1;
    let knots: Vec<f64> = (0..m).map(|i| i as f64).collect();
    let curve = Nurbs::new(p, knots, pts, vec![1.0; points.len() + p])?;
    let (lo, hi) = curve.domain();
    curve.segment(lo, hi)
}
