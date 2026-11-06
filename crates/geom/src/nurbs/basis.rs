//! B-spline basis functions (Piegl and Tiller, *The NURBS Book*, 2nd ed., chapter 2).
//!
//! Every function here assumes a knot vector that passed `check_knots` in `mod.rs`
//! (non-decreasing, finite, `len = n + p + 2`), so indices stay in range; they still use
//! `get` and return zeros rather than panic if called with anything else.

/// The knot span index `i` with `U[i] <= u < U[i+1]` (A2.1, *FindSpan*), clamped to the
/// domain `[U[p], U[n+1]]`: `u` at or past the end gives the last non-empty span `n`.
/// `n` is the index of the last control point.
pub(super) fn find_span(n: usize, p: usize, u: f64, knots: &[f64]) -> usize {
    let at = |i: usize| knots.get(i).copied().unwrap_or(0.0);
    if u >= at(n + 1) {
        // The last span with U[i] < U[i+1].
        let mut i = n;
        while i > p && at(i) >= at(n + 1) {
            i -= 1;
        }
        return i;
    }
    if u <= at(p) {
        // The first span with U[i] < U[i+1].
        let mut i = p;
        while i < n && at(i + 1) <= at(p) {
            i += 1;
        }
        return i;
    }
    let (mut low, mut high) = (p, n + 1);
    let mut mid = (low + high) / 2;
    // Binary search: at most log2(len) steps, bounded for safety.
    let mut guard = 0;
    while (u < at(mid) || u >= at(mid + 1)) && guard < 128 {
        if u < at(mid) {
            high = mid;
        } else {
            low = mid;
        }
        mid = (low + high) / 2;
        guard += 1;
    }
    mid
}

/// The `p + 1` non-vanishing basis functions `N_{i-p,p}(u) .. N_{i,p}(u)` in span `i`
/// (A2.2, *BasisFuns*), by the triangular Cox-de Boor recurrence (eq. 2.5).
pub(super) fn basis_funs(i: usize, u: f64, p: usize, knots: &[f64]) -> Vec<f64> {
    ders_basis_funs(i, u, p, 0, knots)
        .into_iter()
        .next()
        .unwrap_or_default()
}

/// The non-vanishing basis functions and their derivatives up to order `d` in span `i`
/// (A2.3, *DersBasisFuns*): `out[k][j] = N^(k)_{i-p+j,p}(u)` for `k <= d`, `j <= p`. Rows
/// for `k > p` are zero.
pub(super) fn ders_basis_funs(
    i: usize,
    u: f64,
    p: usize,
    d: usize,
    knots: &[f64],
) -> Vec<Vec<f64>> {
    let at = |k: usize| knots.get(k).copied().unwrap_or(0.0);
    let mut ndu = vec![vec![0.0; p + 1]; p + 1];
    let mut left = vec![0.0; p + 1];
    let mut right = vec![0.0; p + 1];
    ndu[0][0] = 1.0;
    for j in 1..=p {
        left[j] = u - at((i + 1).saturating_sub(j));
        right[j] = at(i + j) - u;
        let mut saved = 0.0;
        for r in 0..j {
            // Lower triangle: knot differences; upper: basis functions.
            ndu[j][r] = right[r + 1] + left[j - r];
            let temp = if ndu[j][r] != 0.0 {
                ndu[r][j - 1] / ndu[j][r]
            } else {
                0.0
            };
            ndu[r][j] = saved + right[r + 1] * temp;
            saved = left[j - r] * temp;
        }
        ndu[j][j] = saved;
    }
    let mut ders = vec![vec![0.0; p + 1]; d + 1];
    for j in 0..=p {
        ders[0][j] = ndu[j][p];
    }
    // Derivatives (eq. 2.9), alternating between two rows of `a`.
    let top = d.min(p);
    for r in 0..=p {
        let (mut s1, mut s2) = (0usize, 1usize);
        let mut a = vec![vec![0.0; p + 1]; 2];
        a[0][0] = 1.0;
        for k in 1..=top {
            let mut dsum = 0.0;
            let rk = r as isize - k as isize;
            let pk = p - k;
            if r >= k {
                let den = ndu[pk + 1][rk as usize];
                a[s2][0] = if den != 0.0 { a[s1][0] / den } else { 0.0 };
                dsum = a[s2][0] * ndu[rk as usize][pk];
            }
            let j1 = if rk >= -1 { 1 } else { (-rk) as usize };
            let j2 = if (r as isize - 1) <= pk as isize {
                k - 1
            } else {
                p - r
            };
            for j in j1..=j2 {
                let idx = (rk + j as isize) as usize;
                let den = ndu[pk + 1][idx];
                a[s2][j] = if den != 0.0 {
                    (a[s1][j] - a[s1][j - 1]) / den
                } else {
                    0.0
                };
                dsum += a[s2][j] * ndu[idx][pk];
            }
            if r <= pk {
                let den = ndu[pk + 1][r];
                a[s2][k] = if den != 0.0 { -a[s1][k - 1] / den } else { 0.0 };
                dsum += a[s2][k] * ndu[r][pk];
            }
            ders[k][r] = dsum;
            std::mem::swap(&mut s1, &mut s2);
        }
    }
    // Multiply by p! / (p - k)!.
    let mut factor = p as f64;
    for (k, row) in ders.iter_mut().enumerate().take(top + 1).skip(1) {
        for x in row.iter_mut() {
            *x *= factor;
        }
        factor *= (p - k) as f64;
    }
    ders
}

/// The binomial coefficient `C(n, k)` as a float (small arguments only: `n <= 2 MAX_DEGREE`).
pub(super) fn binomial(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    let mut out = 1.0;
    for i in 0..k {
        out = out * (n - i) as f64 / (i + 1) as f64;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_follow_example_2_3_of_the_book() {
        // U = {0,0,0,1,2,3,4,4,5,5,5}, p = 2, n = 7 (Example 2.3): u = 5/2 is in span 4.
        let u = [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 5.0, 5.0, 5.0];
        assert_eq!(find_span(7, 2, 2.5, &u), 4);
        assert_eq!(find_span(7, 2, 0.0, &u), 2);
        assert_eq!(find_span(7, 2, 5.0, &u), 7);
        assert_eq!(find_span(7, 2, 4.0, &u), 7);
        // Example 2.3: N_{2,2}(5/2) = 1/8, N_{3,2} = 6/8, N_{4,2} = 1/8.
        let n = basis_funs(4, 2.5, 2, &u);
        for (got, want) in n.iter().zip([0.125, 0.75, 0.125]) {
            assert!((got - want).abs() < 1e-15);
        }
    }

    #[test]
    fn derivatives_of_the_quadratic_basis_match_hand_values() {
        // On U = {0,0,0,1,1,1} the basis is Bernstein: (1-u)², 2u(1-u), u²; derivatives
        // -2(1-u), 2 - 4u, 2u; second derivatives 2, -4, 2.
        let k = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let u = 0.3;
        let d = ders_basis_funs(2, u, 2, 3, &k);
        let want = [
            [(1.0 - u) * (1.0 - u), 2.0 * u * (1.0 - u), u * u],
            [-2.0 * (1.0 - u), 2.0 - 4.0 * u, 2.0 * u],
            [2.0, -4.0, 2.0],
            [0.0, 0.0, 0.0],
        ];
        for (row, w) in d.iter().zip(want) {
            for (a, b) in row.iter().zip(w) {
                assert!((a - b).abs() < 1e-14, "{a} vs {b}");
            }
        }
        assert_eq!(binomial(5, 2), 10.0);
    }
}
