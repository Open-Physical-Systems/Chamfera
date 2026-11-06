//! Robust geometric predicates with exact signs: `orient2d`, `orient3d`, `incircle`.
//!
//! Written from J. R. Shewchuk, "Adaptive Precision Floating-Point Arithmetic and Fast
//! Robust Geometric Predicates", Discrete & Computational Geometry 18 (1997) 305-363.
//! Each predicate first evaluates its determinant in ordinary floating point and accepts
//! the sign when the paper's forward error bound (the "A" bounds, section 4) proves it
//! right. Otherwise the determinant is evaluated exactly with floating-point expansions
//! (Two-Sum, Two-Product with Dekker's split, Grow-Expansion and Scale-Expansion, sections
//! 2.4 to 2.6), and the sign of the most significant component is the exact sign.
//!
//! Exactness needs every partial product to be representable without overflow or
//! underflow, so each coordinate must be finite and either zero or of magnitude in
//! `[PREDICATE_MIN_MAGNITUDE, PREDICATE_MAX_MAGNITUDE]`; anything else gives `None`.

use crate::{Point2, Point3};

/// The exact sign of a predicate's determinant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sign {
    Negative,
    Zero,
    Positive,
}

impl Sign {
    /// The opposite sign (`Zero` stays `Zero`).
    pub fn reversed(self) -> Sign {
        match self {
            Sign::Negative => Sign::Positive,
            Sign::Zero => Sign::Zero,
            Sign::Positive => Sign::Negative,
        }
    }

    fn of(v: f64) -> Sign {
        if v > 0.0 {
            Sign::Positive
        } else if v < 0.0 {
            Sign::Negative
        } else {
            Sign::Zero
        }
    }
}

/// Largest coordinate magnitude the predicates accept: 2^200. A degree-4 product
/// (`incircle`) of such values stays below 2^800, far from overflow.
pub const PREDICATE_MAX_MAGNITUDE: f64 = 1.606_938_044_258_990_3e60;
/// Smallest non-zero coordinate magnitude the predicates accept: 2^-200. Every bit of a
/// degree-4 product of such values sits above 2^-1074, so no expansion component underflows.
pub const PREDICATE_MIN_MAGNITUDE: f64 = 6.223_015_277_861_142e-61;

/// Machine epsilon in Shewchuk's sense: half an ulp of 1, 2^-53.
const EPSILON: f64 = 1.110_223_024_625_156_5e-16;
/// Dekker's splitter for 53-bit mantissas: 2^27 + 1.
const SPLITTER: f64 = 134_217_729.0;
/// Shewchuk's error bound for the floating-point `orient2d`: (3 + 16e)e.
const CCW_ERRBOUND_A: f64 = (3.0 + 16.0 * EPSILON) * EPSILON;
/// Shewchuk's error bound for the floating-point `orient3d`: (7 + 56e)e.
const O3D_ERRBOUND_A: f64 = (7.0 + 56.0 * EPSILON) * EPSILON;
/// Shewchuk's error bound for the floating-point `incircle`: (10 + 96e)e.
const ICC_ERRBOUND_A: f64 = (10.0 + 96.0 * EPSILON) * EPSILON;

fn usable(v: f64) -> bool {
    v == 0.0
        || (v.is_finite() && (PREDICATE_MIN_MAGNITUDE..=PREDICATE_MAX_MAGNITUDE).contains(&v.abs()))
}

/// Sign of the area of triangle `abc`: `Positive` when a, b, c turn counter-clockwise,
/// `Negative` when clockwise, `Zero` when exactly collinear. `None` for unusable input.
pub fn orient2d(a: Point2, b: Point2, c: Point2) -> Option<Sign> {
    if ![a.x, a.y, b.x, b.y, c.x, c.y].into_iter().all(usable) {
        return None;
    }
    let det_left = (a.x - c.x) * (b.y - c.y);
    let det_right = (a.y - c.y) * (b.x - c.x);
    let det = det_left - det_right;
    let bound = CCW_ERRBOUND_A * (det_left.abs() + det_right.abs());
    if det > bound || -det > bound {
        return Some(Sign::of(det));
    }
    let acx = Expansion::diff(a.x, c.x);
    let acy = Expansion::diff(a.y, c.y);
    let bcx = Expansion::diff(b.x, c.x);
    let bcy = Expansion::diff(b.y, c.y);
    let exact = acx.mul(&bcy).sum(&acy.mul(&bcx).neg());
    Some(exact.sign())
}

/// Sign of det[a-d; b-d; c-d]: `Positive` when `d` lies below the plane through a, b, c
/// (with a, b, c counter-clockwise seen from above, i.e. from the side their right-hand
/// normal points to), `Negative` above, `Zero` when exactly coplanar. `None` for
/// unusable input.
pub fn orient3d(a: Point3, b: Point3, c: Point3, d: Point3) -> Option<Sign> {
    let coords = [a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z, d.x, d.y, d.z];
    if !coords.into_iter().all(usable) {
        return None;
    }
    let (adx, ady, adz) = (a.x - d.x, a.y - d.y, a.z - d.z);
    let (bdx, bdy, bdz) = (b.x - d.x, b.y - d.y, b.z - d.z);
    let (cdx, cdy, cdz) = (c.x - d.x, c.y - d.y, c.z - d.z);
    let (bdxcdy, cdxbdy) = (bdx * cdy, cdx * bdy);
    let (cdxady, adxcdy) = (cdx * ady, adx * cdy);
    let (adxbdy, bdxady) = (adx * bdy, bdx * ady);
    let det = adz * (bdxcdy - cdxbdy) + bdz * (cdxady - adxcdy) + cdz * (adxbdy - bdxady);
    let permanent = (bdxcdy.abs() + cdxbdy.abs()) * adz.abs()
        + (cdxady.abs() + adxcdy.abs()) * bdz.abs()
        + (adxbdy.abs() + bdxady.abs()) * cdz.abs();
    let bound = O3D_ERRBOUND_A * permanent;
    if det > bound || -det > bound {
        return Some(Sign::of(det));
    }
    let e = |p: f64, q: f64| Expansion::diff(p, q);
    let (adx, ady, adz) = (e(a.x, d.x), e(a.y, d.y), e(a.z, d.z));
    let (bdx, bdy, bdz) = (e(b.x, d.x), e(b.y, d.y), e(b.z, d.z));
    let (cdx, cdy, cdz) = (e(c.x, d.x), e(c.y, d.y), e(c.z, d.z));
    let bc = bdx.mul(&cdy).sum(&cdx.mul(&bdy).neg());
    let ca = cdx.mul(&ady).sum(&adx.mul(&cdy).neg());
    let ab = adx.mul(&bdy).sum(&bdx.mul(&ady).neg());
    let exact = adz.mul(&bc).sum(&bdz.mul(&ca)).sum(&cdz.mul(&ab));
    Some(exact.sign())
}

/// Sign telling where `d` lies against the circle through a, b, c (which must turn
/// counter-clockwise for this reading): `Positive` inside, `Negative` outside, `Zero`
/// exactly on it. If a, b, c turn clockwise the sign is reversed. `None` for unusable
/// input.
pub fn incircle(a: Point2, b: Point2, c: Point2, d: Point2) -> Option<Sign> {
    if ![a.x, a.y, b.x, b.y, c.x, c.y, d.x, d.y]
        .into_iter()
        .all(usable)
    {
        return None;
    }
    let (adx, ady) = (a.x - d.x, a.y - d.y);
    let (bdx, bdy) = (b.x - d.x, b.y - d.y);
    let (cdx, cdy) = (c.x - d.x, c.y - d.y);
    let (bdxcdy, cdxbdy) = (bdx * cdy, cdx * bdy);
    let (cdxady, adxcdy) = (cdx * ady, adx * cdy);
    let (adxbdy, bdxady) = (adx * bdy, bdx * ady);
    let alift = adx * adx + ady * ady;
    let blift = bdx * bdx + bdy * bdy;
    let clift = cdx * cdx + cdy * cdy;
    let det = alift * (bdxcdy - cdxbdy) + blift * (cdxady - adxcdy) + clift * (adxbdy - bdxady);
    let permanent = (bdxcdy.abs() + cdxbdy.abs()) * alift
        + (cdxady.abs() + adxcdy.abs()) * blift
        + (adxbdy.abs() + bdxady.abs()) * clift;
    let bound = ICC_ERRBOUND_A * permanent;
    if det > bound || -det > bound {
        return Some(Sign::of(det));
    }
    let e = |p: f64, q: f64| Expansion::diff(p, q);
    let (adx, ady) = (e(a.x, d.x), e(a.y, d.y));
    let (bdx, bdy) = (e(b.x, d.x), e(b.y, d.y));
    let (cdx, cdy) = (e(c.x, d.x), e(c.y, d.y));
    let alift = adx.mul(&adx).sum(&ady.mul(&ady));
    let blift = bdx.mul(&bdx).sum(&bdy.mul(&bdy));
    let clift = cdx.mul(&cdx).sum(&cdy.mul(&cdy));
    let bc = bdx.mul(&cdy).sum(&cdx.mul(&bdy).neg());
    let ca = cdx.mul(&ady).sum(&adx.mul(&cdy).neg());
    let ab = adx.mul(&bdy).sum(&bdx.mul(&ady).neg());
    let exact = alift.mul(&bc).sum(&blift.mul(&ca)).sum(&clift.mul(&ab));
    Some(exact.sign())
}

/// Two-Sum (Knuth; Shewchuk Theorem 7): `a + b = x + y` exactly, `x = fl(a + b)`.
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let b_virtual = x - a;
    let a_virtual = x - b_virtual;
    let b_roundoff = b - b_virtual;
    let a_roundoff = a - a_virtual;
    (x, a_roundoff + b_roundoff)
}

/// Two-Diff: `a - b = x + y` exactly.
fn two_diff(a: f64, b: f64) -> (f64, f64) {
    two_sum(a, -b)
}

/// Dekker's split (Shewchuk Theorem 17): `a = hi + lo`, each with at most 26 bits.
fn split(a: f64) -> (f64, f64) {
    let c = SPLITTER * a;
    let a_big = c - a;
    let hi = c - a_big;
    (hi, a - hi)
}

/// Two-Product (Shewchuk Theorem 18): `a * b = x + y` exactly, `x = fl(a * b)`.
fn two_product(a: f64, b: f64) -> (f64, f64) {
    let x = a * b;
    let (a_hi, a_lo) = split(a);
    let (b_hi, b_lo) = split(b);
    let err1 = x - a_hi * b_hi;
    let err2 = err1 - a_lo * b_hi;
    let err3 = err2 - a_hi * b_lo;
    (x, a_lo * b_lo - err3)
}

/// A nonoverlapping floating-point expansion: the exact value is the sum of the
/// components, which are non-zero and sorted by increasing magnitude.
struct Expansion(Vec<f64>);

impl Expansion {
    fn diff(a: f64, b: f64) -> Expansion {
        let (x, y) = two_diff(a, b);
        Expansion([y, x].into_iter().filter(|v| *v != 0.0).collect())
    }

    /// Grow-Expansion with zero elimination (Shewchuk Theorem 10).
    fn grow(&mut self, b: f64) {
        let mut q = b;
        let mut out = Vec::with_capacity(self.0.len() + 1);
        for &e in &self.0 {
            let (sum, err) = two_sum(q, e);
            if err != 0.0 {
                out.push(err);
            }
            q = sum;
        }
        if q != 0.0 {
            out.push(q);
        }
        self.0 = out;
    }

    /// Expansion-Sum (Shewchuk Theorem 12): grow by each component of `f`.
    fn sum(mut self, f: &Expansion) -> Expansion {
        for &b in &f.0 {
            self.grow(b);
        }
        self
    }

    /// Scale-Expansion with zero elimination (Shewchuk Theorem 19). `Two-Sum` replaces
    /// `Fast-Two-Sum` in the second step; it is exact without an ordering precondition.
    fn scale(&self, b: f64) -> Expansion {
        let mut out = Vec::with_capacity(2 * self.0.len());
        let mut comps = self.0.iter();
        let Some(&first) = comps.next() else {
            return Expansion(out);
        };
        let (mut q, h) = two_product(first, b);
        if h != 0.0 {
            out.push(h);
        }
        for &e in comps {
            let (t_hi, t_lo) = two_product(e, b);
            let (sum, h) = two_sum(q, t_lo);
            if h != 0.0 {
                out.push(h);
            }
            let (sum2, h) = two_sum(t_hi, sum);
            if h != 0.0 {
                out.push(h);
            }
            q = sum2;
        }
        if q != 0.0 {
            out.push(q);
        }
        Expansion(out)
    }

    fn mul(&self, f: &Expansion) -> Expansion {
        let mut acc = Expansion(Vec::new());
        for &b in &f.0 {
            acc = acc.sum(&self.scale(b));
        }
        acc
    }

    fn neg(mut self) -> Expansion {
        for v in &mut self.0 {
            *v = -*v;
        }
        self
    }

    /// The exact sign: that of the most significant (last) component.
    fn sign(&self) -> Sign {
        self.0.last().map_or(Sign::Zero, |v| Sign::of(*v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Half an ulp of 0.5: points 0.5 + i*U are exactly representable.
    const U: f64 = EPSILON;

    #[test]
    fn simple_turns_have_the_expected_sign() {
        let o = Point2::ORIGIN;
        let x = Point2::new(1.0, 0.0);
        let y = Point2::new(0.0, 1.0);
        assert_eq!(orient2d(o, x, y), Some(Sign::Positive));
        assert_eq!(orient2d(o, y, x), Some(Sign::Negative));
        assert_eq!(orient2d(o, x, Point2::new(2.0, 0.0)), Some(Sign::Zero));
    }

    #[test]
    fn nearly_collinear_points_get_the_exact_sign() {
        // Shewchuk's test pattern (paper, figure 1): a = (0.5 + i U, 0.5 + j U) with
        // b = (12, 12), c = (24, 24) on the line y = x. Expanding the determinant,
        // (ax - 24)(12 - 24) - (ay - 24)(12 - 24) = 12 (ay - ax) = 12 U (j - i),
        // so the exact sign is sign(j - i). Naive floating point gets many of these wrong.
        let b = Point2::new(12.0, 12.0);
        let c = Point2::new(24.0, 24.0);
        let mut naive_wrong = 0;
        for i in 0..64 {
            for j in 0..64 {
                let a = Point2::new(0.5 + f64::from(i) * U, 0.5 + f64::from(j) * U);
                let expected = Sign::of(f64::from(j - i));
                assert_eq!(orient2d(a, b, c), Some(expected), "i={i} j={j}");
                // Cyclic permutations keep the sign, swaps flip it.
                assert_eq!(orient2d(b, c, a), Some(expected));
                let flipped = orient2d(b, a, c);
                assert_eq!(flipped.map(Sign::reversed), Some(expected));
                let naive = (a.x - c.x) * (b.y - c.y) - (a.y - c.y) * (b.x - c.x);
                if Sign::of(naive) != expected {
                    naive_wrong += 1;
                }
            }
        }
        assert!(naive_wrong > 0, "the test must exercise the exact path");
    }

    #[test]
    fn nearly_coplanar_points_get_the_exact_sign() {
        // a = (12, 0, 12), b = (24, 0, 24), c = (0, 24, 0) lie on the plane z = x.
        // d = (0.5 + i U, 0.5, 0.5 + k U). det[a-d; b-d; c-d] = (a-d) . ((b-a) x (c-a)),
        // (b-a) x (c-a) = (12,0,12) x (-12,24,-12) = (-288, 0, 288), so
        // det = -288 (11.5 - i U) + 288 (11.5 - k U) = 288 U (i - k): sign(i - k).
        let a = Point3::new(12.0, 0.0, 12.0);
        let b = Point3::new(24.0, 0.0, 24.0);
        let c = Point3::new(0.0, 24.0, 0.0);
        for i in 0..32 {
            for k in 0..32 {
                let d = Point3::new(0.5 + f64::from(i) * U, 0.5, 0.5 + f64::from(k) * U);
                assert_eq!(
                    orient3d(a, b, c, d),
                    Some(Sign::of(f64::from(i - k))),
                    "i={i} k={k}"
                );
            }
        }
        // Below the plane of a counter-clockwise triangle is positive.
        let (o, x, y) = (
            Point3::ORIGIN,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        );
        assert_eq!(
            orient3d(o, x, y, Point3::new(0.0, 0.0, -1.0)),
            Some(Sign::Positive)
        );
        assert_eq!(
            orient3d(o, x, y, Point3::new(0.3, 0.3, 1.0)),
            Some(Sign::Negative)
        );
    }

    #[test]
    fn points_next_to_a_circle_get_the_exact_sign() {
        // a, b, c on x^2 + y^2 = 25, counter-clockwise. d = (4 + i V, 3 + j V) with V the
        // ulp of 4 (2^-50, so 4 + i V and 3 + j V are exact). Inside iff |d|^2 < 25:
        // 25 - |d|^2 = -(8 i + 6 j) V - (i^2 + j^2) V^2, whose sign is -sign(8i + 6j)
        // when 8i + 6j != 0, negative when 8i + 6j = 0 with (i, j) != 0, zero at (0, 0).
        let v = 4.0 * EPSILON * 2.0;
        let a = Point2::new(5.0, 0.0);
        let b = Point2::new(0.0, 5.0);
        let c = Point2::new(-3.0, -4.0);
        for i in -8..=8 {
            for j in -8..=8 {
                let d = Point2::new(4.0 + f64::from(i) * v, 3.0 + f64::from(j) * v);
                let lin = 8 * i + 6 * j;
                let expected = if lin != 0 {
                    Sign::of(f64::from(-lin))
                } else if i != 0 || j != 0 {
                    Sign::Negative
                } else {
                    Sign::Zero
                };
                assert_eq!(incircle(a, b, c, d), Some(expected), "i={i} j={j}");
            }
        }
        assert_eq!(incircle(a, b, c, Point2::ORIGIN), Some(Sign::Positive));
        assert_eq!(
            incircle(a, b, c, Point2::new(10.0, 0.0)),
            Some(Sign::Negative)
        );
    }

    #[test]
    fn hostile_input_gives_none_instead_of_a_wrong_sign() {
        let o = Point2::ORIGIN;
        let x = Point2::new(1.0, 0.0);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, 1e-300] {
            let p = Point2::new(bad, 1.0);
            assert_eq!(orient2d(o, x, p), None);
            assert_eq!(incircle(o, x, Point2::new(0.0, 1.0), p), None);
            let q = Point3::new(0.0, bad, 0.0);
            assert_eq!(
                orient3d(Point3::ORIGIN, Point3::ORIGIN, Point3::ORIGIN, q),
                None
            );
        }
        // Huge but accepted coordinates still give exact answers: three collinear points.
        let big = 1e50;
        assert_eq!(
            orient2d(
                Point2::new(big, big),
                Point2::new(-big, -big),
                Point2::new(3.0, 3.0)
            ),
            Some(Sign::Zero)
        );
        // Degenerate input is a valid question with answer Zero.
        assert_eq!(orient2d(o, o, o), Some(Sign::Zero));
        assert_eq!(incircle(o, o, o, o), Some(Sign::Zero));
    }

    #[test]
    fn expansions_are_exact_sums() {
        // 1 + 2^-60 - 1 is lost in floating point but kept by an expansion.
        let tiny = 2f64.powi(-60);
        let mut e = Expansion(vec![]);
        e.grow(1.0);
        e.grow(tiny);
        e.grow(-1.0);
        assert_eq!(e.0, vec![tiny]);
        // (2^27 + 1)^2 = 2^54 + 2^28 + 1: the low bit needs a second component.
        let (x, y) = two_product(SPLITTER, SPLITTER);
        assert_eq!(x, 2f64.powi(54) + 2f64.powi(28));
        assert_eq!(y, 1.0);
    }
}
