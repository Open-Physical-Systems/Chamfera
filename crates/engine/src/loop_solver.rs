//! A small dense Levenberg-Marquardt solver for joint loops (closed kinematic chains).
//!
//! The same method as the sketch solver (`chamfera_sketch`'s `levenberg_marquardt`):
//! damped normal equations `(JᵀJ + μI) δ = -Jᵀr` solved by Cholesky, a step kept when
//! the gain ratio is positive, and μ updated by Nielsen's rule (Madsen, Nielsen and
//! Tingleff, *Methods for Non-Linear Least Squares Problems*, 2004, section 3.2). Loops
//! have few unknowns (a few per joint) and their residuals come from composing
//! transforms, so the Jacobian is dense and taken by central differences.

/// Most iterations one solve may take.
const MAX_ITERATIONS: usize = 200;
/// Damping above this means no step can reduce the cost: stop.
const MAX_DAMPING: f64 = 1e16;
/// The step for central differences, in radians or mm.
const STEP: f64 = 1e-6;
/// A residual this small (mm) is as good as zero: stop iterating.
pub const SOLVE_TARGET: f64 = 1e-11;

/// How a solve ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    pub iterations: usize,
    /// The largest residual at the end (mm), `INFINITY` when it could not be evaluated.
    pub max_residual: f64,
}

fn max_abs(v: &[f64]) -> f64 {
    v.iter().fold(0.0_f64, |m, a| {
        if a.is_finite() {
            m.max(a.abs())
        } else {
            f64::INFINITY
        }
    })
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The Jacobian of `f` at `x` by central differences, row-major `m x n`, or `None` when
/// `f` fails near `x`.
pub fn jacobian(
    f: &dyn Fn(&[f64]) -> Option<Vec<f64>>,
    x: &[f64],
    m: usize,
) -> Option<Vec<f64>> {
    let n = x.len();
    let mut jac = vec![0.0; m * n];
    let mut probe = x.to_vec();
    for k in 0..n {
        let x0 = *x.get(k)?;
        *probe.get_mut(k)? = x0 + STEP;
        let up = f(&probe)?;
        *probe.get_mut(k)? = x0 - STEP;
        let down = f(&probe)?;
        *probe.get_mut(k)? = x0;
        if up.len() != m || down.len() != m {
            return None;
        }
        for (i, (u, d)) in up.iter().zip(&down).enumerate() {
            *jac.get_mut(i * n + k)? = (u - d) / (2.0 * STEP);
        }
    }
    Some(jac)
}

/// `JᵀJ` (n x n) and `Jᵀr`.
fn normal_equations(jac: &[f64], r: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let m = r.len();
    let mut a = vec![0.0; n * n];
    let mut g = vec![0.0; n];
    for i in 0..m {
        let row = jac.get(i * n..(i + 1) * n).unwrap_or(&[]);
        let ri = r.get(i).copied().unwrap_or(0.0);
        for (p, vp) in row.iter().enumerate() {
            if let Some(gp) = g.get_mut(p) {
                *gp += vp * ri;
            }
            for (q, vq) in row.iter().enumerate() {
                if let Some(slot) = a.get_mut(p * n + q) {
                    *slot += vp * vq;
                }
            }
        }
    }
    (a, g)
}

/// Solves `a x = b` in place for a symmetric positive definite `a` (n x n, row-major),
/// by Cholesky. False when `a` is not positive definite.
fn cholesky_solve(a: &mut [f64], n: usize, b: &mut [f64]) -> bool {
    if a.len() != n * n || b.len() != n {
        return false;
    }
    for j in 0..n {
        let mut d = a[j * n + j];
        for k in 0..j {
            d -= a[j * n + k] * a[j * n + k];
        }
        if !(d.is_finite() && d > 0.0) {
            return false;
        }
        let d = d.sqrt();
        a[j * n + j] = d;
        for i in j + 1..n {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= a[i * n + k] * a[j * n + k];
            }
            a[i * n + j] = s / d;
        }
    }
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

/// Levenberg-Marquardt on the residuals `f` (length `m`) over `x`, updating `x` in
/// place to the best point found.
pub fn levenberg_marquardt(
    f: &dyn Fn(&[f64]) -> Option<Vec<f64>>,
    x: &mut [f64],
    m: usize,
) -> Outcome {
    let n = x.len();
    let Some(mut r) = f(x) else {
        return Outcome {
            iterations: 0,
            max_residual: f64::INFINITY,
        };
    };
    let mut max_r = max_abs(&r);
    if max_r <= SOLVE_TARGET || n == 0 {
        return Outcome {
            iterations: 0,
            max_residual: max_r,
        };
    }
    let Some(mut jac) = jacobian(f, x, m) else {
        return Outcome {
            iterations: 0,
            max_residual: max_r,
        };
    };
    let mut cost = 0.5 * dot(&r, &r);
    let (mut a, mut g) = normal_equations(&jac, &r, n);
    let max_diag = (0..n)
        .filter_map(|i| a.get(i * n + i))
        .fold(0.0_f64, |m, v| m.max(*v));
    let mut mu = if max_diag > 0.0 {
        1e-3 * max_diag
    } else {
        1e-3
    };
    let mut nu = 2.0;
    let mut iterations = 0;
    while iterations < MAX_ITERATIONS {
        iterations += 1;
        let mut damped = a.clone();
        for i in 0..n {
            if let Some(d) = damped.get_mut(i * n + i) {
                *d += mu;
            }
        }
        let mut delta: Vec<f64> = g.iter().map(|v| -v).collect();
        if !cholesky_solve(&mut damped, n, &mut delta) {
            mu *= nu;
            nu *= 2.0;
            if mu > MAX_DAMPING {
                break;
            }
            continue;
        }
        let xnorm = dot(x, x).sqrt();
        if dot(&delta, &delta).sqrt() <= 1e-15 * (xnorm + 1.0) {
            break;
        }
        let trial: Vec<f64> = x.iter().zip(&delta).map(|(a, d)| a + d).collect();
        let r_new = f(&trial);
        let new_cost = r_new.as_ref().map_or(f64::INFINITY, |v| 0.5 * dot(v, v));
        // Predicted reduction of the linear model: ½ δᵀ(μδ - g).
        let predicted: f64 = 0.5
            * delta
                .iter()
                .zip(&g)
                .map(|(d, gi)| d * (mu * d - gi))
                .sum::<f64>();
        let rho = if predicted > 0.0 {
            (cost - new_cost) / predicted
        } else if new_cost < cost {
            1.0
        } else {
            -1.0
        };
        match r_new {
            Some(rn) if new_cost.is_finite() && rho > 0.0 => {
                x.copy_from_slice(&trial);
                r = rn;
                cost = new_cost;
                max_r = max_abs(&r);
                if max_r <= SOLVE_TARGET {
                    break;
                }
                match jacobian(f, x, m) {
                    Some(j) => jac = j,
                    None => break,
                }
                (a, g) = normal_equations(&jac, &r, n);
                let t = 2.0 * rho - 1.0;
                mu *= (1.0 - t * t * t).max(1.0 / 3.0);
                nu = 2.0;
            }
            _ => {
                mu *= nu;
                nu *= 2.0;
                if mu > MAX_DAMPING {
                    break;
                }
            }
        }
    }
    Outcome {
        iterations,
        max_residual: max_r,
    }
}

/// The rank of an `m x n` row-major matrix, by Gaussian elimination with full pivoting;
/// a pivot below `1e-8` times the largest entry counts as zero.
pub fn rank(mat: &[f64], m: usize, n: usize) -> usize {
    if mat.len() != m * n {
        return 0;
    }
    let mut a = mat.to_vec();
    let scale = max_abs(&a);
    if !(scale.is_finite() && scale > 0.0) {
        return 0;
    }
    let tol = 1e-8 * scale;
    let (mut rows, mut cols): (Vec<usize>, Vec<usize>) = ((0..m).collect(), (0..n).collect());
    let mut r = 0;
    while !rows.is_empty() && !cols.is_empty() {
        let mut best = (0usize, 0usize, 0.0_f64);
        for (ri, &i) in rows.iter().enumerate() {
            for (ci, &j) in cols.iter().enumerate() {
                let v = a.get(i * n + j).copied().unwrap_or(0.0).abs();
                if v > best.2 {
                    best = (ri, ci, v);
                }
            }
        }
        if best.2 <= tol {
            break;
        }
        let (pi, pj) = (rows.swap_remove(best.0), cols.swap_remove(best.1));
        let pivot = a.get(pi * n + pj).copied().unwrap_or(1.0);
        for &i in &rows {
            let factor = a.get(i * n + pj).copied().unwrap_or(0.0) / pivot;
            for &j in &cols {
                let p = a.get(pi * n + j).copied().unwrap_or(0.0);
                if let Some(v) = a.get_mut(i * n + j) {
                    *v -= factor * p;
                }
            }
        }
        r += 1;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenberg_marquardt_finds_the_root_of_a_circle_and_a_line() {
        // x² + y² = 25 and y = x + 1: from (2, 2) the near root is (3, 4)
        // (x² + (x + 1)² = 25 → x² + x - 12 = 0 → x = 3).
        let f = |v: &[f64]| -> Option<Vec<f64>> {
            let (x, y) = (*v.first()?, *v.get(1)?);
            Some(vec![x * x + y * y - 25.0, y - x - 1.0])
        };
        let mut x = vec![2.0, 2.0];
        let out = levenberg_marquardt(&f, &mut x, 2);
        assert!(out.max_residual <= SOLVE_TARGET, "{out:?}");
        assert!((x[0] - 3.0).abs() < 1e-9 && (x[1] - 4.0).abs() < 1e-9, "{x:?}");
    }

    #[test]
    fn inconsistent_equations_leave_a_residual() {
        // x = 1 and x = 3: the least-squares point is x = 2, residual 1 each.
        let f = |v: &[f64]| -> Option<Vec<f64>> {
            let x = *v.first()?;
            Some(vec![x - 1.0, x - 3.0])
        };
        let mut x = vec![0.0];
        let out = levenberg_marquardt(&f, &mut x, 2);
        assert!((x[0] - 2.0).abs() < 1e-6, "{x:?}");
        assert!((out.max_residual - 1.0).abs() < 1e-6, "{out:?}");
    }

    #[test]
    fn rank_counts_independent_rows() {
        // Row 3 = row 1 + row 2.
        let m = [1.0, 2.0, 0.0, 0.0, 1.0, 1.0, 1.0, 3.0, 1.0];
        assert_eq!(rank(&m, 3, 3), 2);
        assert_eq!(rank(&[0.0; 4], 2, 2), 0);
        assert_eq!(rank(&[1.0, 0.0, 0.0, 1.0], 2, 2), 2);
    }
}
