//! Small dense linear algebra for the solver, written for this crate (no dependency).
//!
//! - [`cholesky_solve`] solves the damped normal equations of a Levenberg-Marquardt step,
//!   `(JᵀJ + μI) δ = -Jᵀr`, which are symmetric positive definite for `μ > 0`.
//! - [`RowSpace`] is a Householder QR factorisation of `Jᵀ` built one row of `J` at a time,
//!   in a fixed order, skipping rows that add no rank. It gives the rank of `J` (degrees of
//!   freedom), which rows depend on earlier ones (redundant or conflicting constraints),
//!   whether a coordinate axis lies in the row space (a fully determined unknown), and the
//!   minimum-norm and null-space parts of a step (dragging).
//!
//! Method: Golub and Van Loan, *Matrix Computations*, sections 4.2 (Cholesky) and 5.1-5.2
//! (Householder reflections and QR).

/// A row of the Jacobian is taken as depending on the rows before it when the part of it
/// outside their span is shorter than this fraction of its own length.
///
/// Exactly dependent rows leave about 1e-15 of their length (rounding); genuinely independent
/// rows of a sketch with sensible proportions leave far more than 1e-8. A smaller value would
/// call nearly degenerate geometry independent; a larger one would merge distinct constraints
/// on long thin shapes.
pub(crate) const RANK_TOLERANCE: f64 = 1e-8;

pub(crate) fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub(crate) fn norm(a: &[f64]) -> f64 {
    dot(a, a).sqrt()
}

/// Solves `A x = b` for a symmetric positive definite `A` (row-major `n × n`, only the lower
/// triangle is read). `a` is overwritten by its Cholesky factor and `b` by the solution.
/// Returns false when `A` is not numerically positive definite or the result is not finite.
pub(crate) fn cholesky_solve(a: &mut [f64], n: usize, b: &mut [f64]) -> bool {
    if a.len() != n * n || b.len() != n {
        return false;
    }
    for j in 0..n {
        let mut d = a[j * n + j];
        for k in 0..j {
            d -= a[j * n + k] * a[j * n + k];
        }
        if d.is_nan() || d <= 0.0 || !d.is_finite() {
            return false;
        }
        let l = d.sqrt();
        a[j * n + j] = l;
        for i in j + 1..n {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= a[i * n + k] * a[j * n + k];
            }
            a[i * n + j] = s / l;
        }
    }
    // L y = b, then Lᵀ x = y.
    for i in 0..n {
        let mut s = b[i];
        for k in 0..i {
            s -= a[i * n + k] * b[k];
        }
        b[i] = s / a[i * n + i];
    }
    for i in (0..n).rev() {
        let mut s = b[i];
        for k in i + 1..n {
            s -= a[k * n + i] * b[k];
        }
        b[i] = s / a[i * n + i];
    }
    b.iter().all(|v| v.is_finite())
}

/// An orthonormal basis of the span of the rows pushed so far, as Householder reflectors:
/// with `Q = H₁ H₂ … H_r`, the accepted rows (as columns) are `Q [R; 0]`.
pub(crate) struct RowSpace {
    n: usize,
    /// Reflector `k` acts on entries `k..n`: `H_k x = x - β_k v_k (v_kᵀ x)`.
    vectors: Vec<Vec<f64>>,
    betas: Vec<f64>,
    /// Column `k` of the upper triangular `R`: entries `0..=k`.
    r_columns: Vec<Vec<f64>>,
}

impl RowSpace {
    /// An empty basis in `n`-dimensional space.
    pub(crate) fn new(n: usize) -> Self {
        Self {
            n,
            vectors: Vec::new(),
            betas: Vec::new(),
            r_columns: Vec::new(),
        }
    }

    /// The dimension of the span (the rank of the accepted rows).
    pub(crate) fn rank(&self) -> usize {
        self.vectors.len()
    }

    fn reflect(&self, k: usize, x: &mut [f64]) {
        let (Some(v), Some(beta), Some(tail)) =
            (self.vectors.get(k), self.betas.get(k), x.get_mut(k..))
        else {
            return;
        };
        let s = beta * dot(v, tail);
        for (t, vi) in tail.iter_mut().zip(v) {
            *t -= s * vi;
        }
    }

    /// `x ← Qᵀ x`. The first `rank` entries are then the coordinates of `x` in the basis,
    /// the rest its part outside the span.
    pub(crate) fn apply_qt(&self, x: &mut [f64]) {
        for k in 0..self.rank() {
            self.reflect(k, x);
        }
    }

    /// `x ← Q x`.
    pub(crate) fn apply_q(&self, x: &mut [f64]) {
        for k in (0..self.rank()).rev() {
            self.reflect(k, x);
        }
    }

    /// Adds a row if it is independent of the rows already accepted ([`RANK_TOLERANCE`]).
    /// Returns whether it was accepted.
    pub(crate) fn push(&mut self, row: &[f64]) -> bool {
        let k = self.rank();
        if row.len() != self.n || k >= self.n {
            return false;
        }
        let norm0 = norm(row);
        if !norm0.is_finite() || norm0 <= 0.0 {
            return false;
        }
        let mut v = row.to_vec();
        self.apply_qt(&mut v);
        let Some(tail) = v.get(k..) else {
            return false;
        };
        let sigma = norm(tail);
        if sigma <= RANK_TOLERANCE * norm0 {
            return false;
        }
        let head = tail.first().copied().unwrap_or(0.0);
        // Choose the sign that avoids cancellation in v₀ - α.
        let alpha = if head > 0.0 { -sigma } else { sigma };
        let mut u = tail.to_vec();
        if let Some(u0) = u.first_mut() {
            *u0 -= alpha;
        }
        let uu = dot(&u, &u);
        if uu <= 0.0 || !uu.is_finite() {
            return false;
        }
        let mut r_col: Vec<f64> = v.iter().take(k).copied().collect();
        r_col.push(alpha);
        self.vectors.push(u);
        self.betas.push(2.0 / uu);
        self.r_columns.push(r_col);
        true
    }

    /// Solves `Rᵀ z = rhs` by forward substitution (`rhs` has `rank` entries).
    pub(crate) fn solve_rt(&self, rhs: &[f64]) -> Option<Vec<f64>> {
        let r = self.rank();
        if rhs.len() != r {
            return None;
        }
        let mut z = vec![0.0; r];
        for j in 0..r {
            let col = self.r_columns.get(j)?;
            let mut s = rhs[j];
            for (c, zi) in col.iter().zip(&z).take(j) {
                s -= c * zi;
            }
            let d = *col.get(j)?;
            if d == 0.0 {
                return None;
            }
            z[j] = s / d;
        }
        z.iter().all(|v| v.is_finite()).then_some(z)
    }

    /// The length of the part of unit vector `e_i` outside the span: 0 when coordinate `i`
    /// is fully determined by the rows, 1 when no row involves it.
    pub(crate) fn axis_freedom(&self, i: usize) -> f64 {
        let mut e = vec![0.0; self.n];
        let Some(slot) = e.get_mut(i) else {
            return 1.0;
        };
        *slot = 1.0;
        self.apply_qt(&mut e);
        e.get(self.rank()..).map(norm).unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cholesky_solves_a_known_system() {
        // A = [[4, 2], [2, 3]], b = [2, 1]: det 8, x = (3·2 - 2·1, 4·1 - 2·2)/8 = (0.5, 0).
        let mut a = vec![4.0, 2.0, 2.0, 3.0];
        let mut b = vec![2.0, 1.0];
        assert!(cholesky_solve(&mut a, 2, &mut b));
        assert!((b[0] - 0.5).abs() < 1e-15 && b[1].abs() < 1e-15);
    }

    #[test]
    fn cholesky_refuses_indefinite_and_mismatched_input() {
        let mut a = vec![1.0, 2.0, 2.0, 1.0]; // eigenvalues 3 and -1
        let mut b = vec![1.0, 1.0];
        assert!(!cholesky_solve(&mut a, 2, &mut b));
        let mut short = vec![1.0];
        assert!(!cholesky_solve(&mut short, 2, &mut b));
        let mut nan = vec![f64::NAN];
        let mut one = vec![1.0];
        assert!(!cholesky_solve(&mut nan, 1, &mut one));
    }

    #[test]
    fn row_space_finds_rank_and_dependent_rows() {
        let mut rs = RowSpace::new(3);
        assert!(rs.push(&[1.0, 1.0, 0.0]));
        assert!(rs.push(&[0.0, 1.0, 0.0]));
        // (2, 3, 0) = 2·row0 + row1: dependent.
        assert!(!rs.push(&[2.0, 3.0, 0.0]));
        assert!(!rs.push(&[0.0, 0.0, 0.0]));
        assert_eq!(rs.rank(), 2);
        // x and y are in the span, z is not.
        assert!(rs.axis_freedom(0) < 1e-12);
        assert!(rs.axis_freedom(1) < 1e-12);
        assert!((rs.axis_freedom(2) - 1.0).abs() < 1e-12);
        assert!(rs.push(&[0.0, 0.0, 5.0]));
        assert!(!rs.push(&[1.0, 2.0, 3.0]));
    }

    #[test]
    fn q_and_r_reproduce_the_rows() {
        let rows = [[3.0, 4.0, 0.0], [1.0, 0.0, 2.0]];
        let mut rs = RowSpace::new(3);
        for r in &rows {
            assert!(rs.push(r));
        }
        // Row j = Q [R col j; 0].
        for (j, row) in rows.iter().enumerate() {
            let mut w = vec![0.0; 3];
            for (i, v) in rs.r_columns[j].iter().enumerate() {
                w[i] = *v;
            }
            rs.apply_q(&mut w);
            for i in 0..3 {
                assert!((w[i] - row[i]).abs() < 1e-14);
            }
        }
        // Rᵀ z = b solved: check by multiplying back. R = [[r00, r01], [0, r11]].
        let z = rs.solve_rt(&[1.0, 2.0]).unwrap();
        let r00 = rs.r_columns[0][0];
        let r01 = rs.r_columns[1][0];
        let r11 = rs.r_columns[1][1];
        assert!((r00 * z[0] - 1.0).abs() < 1e-14);
        assert!((r01 * z[0] + r11 * z[1] - 2.0).abs() < 1e-14);
    }
}
