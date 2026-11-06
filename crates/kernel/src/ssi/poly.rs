//! Real polynomials of low degree and their real roots on a closed interval.
//!
//! Root isolation by derivative recursion (the classical method behind Rolle's theorem,
//! e.g. Patrikalakis and Maekawa, *Shape Interrogation*, section 4.8): between two
//! consecutive real roots of `p'` the polynomial `p` is monotone, so it has at most one
//! root there, found by bisection when the ends differ in sign. The roots of `p'` come
//! from the same procedure one degree lower, down to a constant. Only evaluation and
//! bisection are used (no division by a leading coefficient), so a nearly vanishing
//! leading coefficient only adds roots far away instead of breaking the method.
//!
//! Work is bounded: the recursion depth is the degree (at most [`MAX_DEGREE`]) and each
//! root takes at most [`MAX_BISECTIONS`] halvings.

/// Highest degree the curve-surface intersections need: a conic on a torus gives
/// degree 8 in the half-angle tangent.
pub(crate) const MAX_DEGREE: usize = 8;
/// Halvings per root; 200 is far more than the 64 that exhaust an f64 interval.
const MAX_BISECTIONS: usize = 200;

/// A polynomial; coefficients lowest degree first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Poly(pub(crate) Vec<f64>);

impl Poly {
    pub(crate) fn constant(c: f64) -> Poly {
        Poly(vec![c])
    }

    /// The degree: the index of the last non-zero coefficient (0 for a constant).
    pub(crate) fn degree(&self) -> usize {
        self.0.iter().rposition(|c| *c != 0.0).unwrap_or(0)
    }

    pub(crate) fn add(&self, o: &Poly) -> Poly {
        let n = self.0.len().max(o.0.len());
        Poly(
            (0..n)
                .map(|k| self.0.get(k).copied().unwrap_or(0.0) + o.0.get(k).copied().unwrap_or(0.0))
                .collect(),
        )
    }

    pub(crate) fn sub(&self, o: &Poly) -> Poly {
        self.add(&o.scale(-1.0))
    }

    pub(crate) fn scale(&self, s: f64) -> Poly {
        Poly(self.0.iter().map(|c| c * s).collect())
    }

    pub(crate) fn mul(&self, o: &Poly) -> Poly {
        if self.0.is_empty() || o.0.is_empty() {
            return Poly::constant(0.0);
        }
        let mut out = vec![0.0; self.0.len() + o.0.len() - 1];
        for (i, a) in self.0.iter().enumerate() {
            for (j, b) in o.0.iter().enumerate() {
                if let Some(slot) = out.get_mut(i + j) {
                    *slot += a * b;
                }
            }
        }
        Poly(out)
    }

    /// Horner evaluation.
    pub(crate) fn eval(&self, x: f64) -> f64 {
        self.0.iter().rev().fold(0.0, |acc, c| acc * x + c)
    }

    pub(crate) fn derivative(&self) -> Poly {
        if self.0.len() <= 1 {
            return Poly::constant(0.0);
        }
        Poly(
            self.0
                .iter()
                .enumerate()
                .skip(1)
                .map(|(k, c)| c * k as f64)
                .collect(),
        )
    }
}

/// The real roots of `p` in `[lo, hi]` (finite, `lo <= hi`), ascending. A multiple root is
/// reported once when `p` changes sign or vanishes exactly there; an even-multiplicity
/// root where rounding keeps the sign shows up as a root of `p'` instead, which is how the
/// callers detect tangency. A constant polynomial has no isolated roots.
pub(crate) fn real_roots(p: &Poly, lo: f64, hi: f64) -> Vec<f64> {
    if !(lo.is_finite() && hi.is_finite() && lo <= hi) {
        return Vec::new();
    }
    roots_rec(p, lo, hi, 0)
}

fn roots_rec(p: &Poly, lo: f64, hi: f64, depth: usize) -> Vec<f64> {
    if p.degree() == 0 || depth > MAX_DEGREE {
        return Vec::new();
    }
    let crit = roots_rec(&p.derivative(), lo, hi, depth + 1);
    let mut knots = Vec::with_capacity(crit.len() + 2);
    knots.push(lo);
    knots.extend(crit.into_iter().filter(|c| *c > lo && *c < hi));
    knots.push(hi);
    let mut out: Vec<f64> = Vec::new();
    for w in knots.windows(2) {
        let (a, b) = match w {
            [a, b] => (*a, *b),
            _ => continue,
        };
        let (fa, fb) = (p.eval(a), p.eval(b));
        if fa == 0.0 {
            out.push(a);
        }
        if fb == 0.0 {
            out.push(b);
        } else if fa != 0.0 && (fa < 0.0) != (fb < 0.0) {
            out.push(bisect(p, a, b, fa));
        }
    }
    out.dedup_by(|x, y| x == y);
    out
}

/// A root of `p` in `[a, b]` where `p(a)` (= `fa`) and `p(b)` differ in sign.
fn bisect(p: &Poly, mut a: f64, mut b: f64, fa: f64) -> f64 {
    let neg_a = fa < 0.0;
    for _ in 0..MAX_BISECTIONS {
        let m = 0.5 * (a + b);
        if m <= a || m >= b {
            break;
        }
        let fm = p.eval(m);
        if fm == 0.0 {
            return m;
        }
        if (fm < 0.0) == neg_a {
            a = m;
        } else {
            b = m;
        }
    }
    0.5 * (a + b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_of_a_quartic_with_known_factors() {
        // (x - 1)(x + 2)(x - 3)(x + 0.5), built by multiplying the factors, so the roots
        // are known exactly: -2, -0.5, 1, 3.
        let f = |r: f64| Poly(vec![-r, 1.0]);
        let p = f(1.0).mul(&f(-2.0)).mul(&f(3.0)).mul(&f(-0.5));
        let r = real_roots(&p, -10.0, 10.0);
        let want = [-2.0, -0.5, 1.0, 3.0];
        assert_eq!(r.len(), 4, "{r:?}");
        for (g, w) in r.iter().zip(want) {
            assert!((g - w).abs() < 1e-12, "{g} vs {w}");
        }
        // Restricted to [0, 2]: only x = 1.
        assert_eq!(real_roots(&p, 0.0, 2.0).len(), 1);
    }

    #[test]
    fn a_double_root_is_a_critical_point() {
        // (x - 2)^2 + 1e-20 has no real root but its derivative vanishes at 2.
        let p = Poly(vec![4.0 + 1e-20, -4.0, 1.0]);
        assert!(real_roots(&p, -5.0, 5.0).len() <= 1);
        let c = real_roots(&p.derivative(), -5.0, 5.0);
        assert_eq!(c.len(), 1);
        assert!((c[0] - 2.0).abs() < 1e-12);
    }

    #[test]
    fn constants_and_bad_intervals_have_no_roots() {
        assert!(real_roots(&Poly::constant(0.0), 0.0, 1.0).is_empty());
        assert!(real_roots(&Poly(vec![-1.0, 1.0]), f64::NAN, 1.0).is_empty());
        assert!(real_roots(&Poly(vec![-1.0, 1.0]), 2.0, 1.0).is_empty());
        assert_eq!(real_roots(&Poly(vec![-1.0, 1.0]), 0.0, 1.0), vec![1.0]);
    }
}
