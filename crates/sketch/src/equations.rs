//! Residual equations with analytic Jacobians.
//!
//! The unknowns are laid out in one vector `x`: every point contributes `x` and `y` (at
//! indices `p` and `p + 1`), every circle its radius. Each constraint compiles to one or
//! more [`Equation`]s, scalar functions of `x` that are zero when the constraint holds, each
//! with its gradient written out by hand.
//!
//! Every residual is a length in millimetres, so one tolerance (`LINEAR_TOLERANCE`) judges
//! them all. Angular residuals (parallel, perpendicular, angle) are dimensionless, so they
//! are multiplied by the sketch's characteristic size `S` (the bounding-box diagonal, at
//! least 1 mm); a residual of `S·sin θ` is how far the far end of a sketch-sized line would
//! be off.

use crate::constraint::Constraint;
use crate::entity::{ConstraintId, Curve, CurveId, PointId};
use crate::sketch::{Sketch, SketchError};
use chamfera_geom::{LINEAR_TOLERANCE, Vec2};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

/// Lines shorter than this (mm) have no reliable direction. Equations that divide by a
/// line's length use at least this length, so their values stay finite on a zero-length
/// line; a distance between (nearly) coincident points takes +X as its direction.
const MIN_LENGTH: f64 = LINEAR_TOLERANCE;

/// An endpoint shared by a line or arc `a` and an arc `b` (or the other way round), when
/// at least one of them is an arc: the point where they touch if they are tangent.
fn shared_end(sketch: &Sketch, a: CurveId, b: CurveId) -> Option<PointId> {
    let ends = |c: CurveId| match sketch.curve(c)? {
        Curve::Line { start, end, .. } | Curve::Arc { start, end, .. } => Some([*start, *end]),
        Curve::Circle { .. } => None,
    };
    let (ea, eb) = (ends(a)?, ends(b)?);
    let arcs = [a, b]
        .iter()
        .filter(|c| matches!(sketch.curve(**c), Some(Curve::Arc { .. })))
        .count();
    if arcs == 0 {
        return None;
    }
    ea.into_iter().find(|p| eb.contains(p))
}

/// A radius: a circle's unknown, or an arc's centre-to-start distance.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Radius {
    Var(usize),
    Arc { centre: usize, start: usize },
}

/// One scalar residual. Point fields are the index of the point's `x` in the unknowns.
#[derive(Debug, Clone)]
pub(crate) enum Residual {
    /// `Σ cᵢ xᵢ + constant`.
    Linear {
        terms: Vec<(usize, f64)>,
        constant: f64,
    },
    /// `|p - q| - value`.
    Distance { p: usize, q: usize, value: f64 },
    /// Signed distance of `p` from the line `a → b` (positive on the left) minus `value`.
    LineDistance {
        p: usize,
        a: usize,
        b: usize,
        value: f64,
    },
    /// `|p - centre| - r`.
    OnCircle {
        p: usize,
        centre: usize,
        radius: Radius,
    },
    /// Signed distance of `centre` from the line `a → b` minus `side · r`.
    LineTangent {
        a: usize,
        b: usize,
        centre: usize,
        radius: Radius,
        side: f64,
    },
    /// `|c1 - c2| - (s1 r1 + s2 r2)`: external tangency has `s1 = s2 = 1`, internal has
    /// opposite signs.
    CircleTangent {
        c1: usize,
        r1: Radius,
        s1: f64,
        c2: usize,
        r2: Radius,
        s2: f64,
    },
    /// `|b1 - a1| - |b2 - a2|`.
    LengthDiff {
        a1: usize,
        b1: usize,
        a2: usize,
        b2: usize,
    },
    /// `r1 - r2`.
    RadiusDiff { r1: Radius, r2: Radius },
    /// `factor · r - value`.
    RadiusValue {
        radius: Radius,
        factor: f64,
        value: f64,
    },
    /// `S · sin θ` between the two lines.
    Parallel {
        a1: usize,
        b1: usize,
        a2: usize,
        b2: usize,
        scale: f64,
    },
    /// `S · cos θ` between the two lines.
    Perpendicular {
        a1: usize,
        b1: usize,
        a2: usize,
        b2: usize,
        scale: f64,
    },
    /// `S · (θ - value)`, wrapped to (-π, π], with θ the counter-clockwise angle from line 1
    /// to line 2.
    Angle {
        a1: usize,
        b1: usize,
        a2: usize,
        b2: usize,
        value: f64,
        scale: f64,
    },
    /// Signed distance of the midpoint of `p q` from the line `a → b`.
    SymmetricMid {
        p: usize,
        q: usize,
        a: usize,
        b: usize,
    },
    /// `(q - p) · (b - a) / |b - a|`: `p q` is perpendicular to the line.
    SymmetricPerp {
        p: usize,
        q: usize,
        a: usize,
        b: usize,
    },
}

/// A residual and the constraint it comes from (`None` for an arc's own equal-radius
/// equation).
#[derive(Debug, Clone)]
pub(crate) struct Equation {
    pub residual: Residual,
    pub owner: Option<ConstraintId>,
}

fn val(x: &[f64], i: usize) -> f64 {
    x.get(i).copied().unwrap_or(f64::NAN)
}

fn pt(x: &[f64], p: usize) -> Vec2 {
    Vec2::new(val(x, p), val(x, p.saturating_add(1)))
}

fn push(g: &mut dyn FnMut(usize, f64), p: usize, v: Vec2) {
    g(p, v.x);
    g(p.saturating_add(1), v.y);
}

/// Length and unit direction; +X when the vector is (nearly) zero.
fn length_dir(v: Vec2) -> (f64, Vec2) {
    let l = v.length();
    if l.is_finite() && l >= MIN_LENGTH {
        (l, v * (1.0 / l))
    } else {
        (l, Vec2::X)
    }
}

/// Signed distance of `p` from the line `a → b` and its gradients with respect to p, a, b.
///
/// With `d = b - a`, `w = p - a`, `c = d × w`, `L = |d|`: `f = c / L`,
/// `∂f/∂p = perp(d) / L`, `∂f/∂d = -perp(w) / L - c d / L³`, `∂f/∂b = ∂f/∂d`,
/// `∂f/∂a = -∂f/∂d - ∂f/∂p`.
fn signed_distance(p: Vec2, a: Vec2, b: Vec2) -> (f64, Vec2, Vec2, Vec2) {
    let d = b - a;
    let w = p - a;
    let l = d.length().max(MIN_LENGTH);
    let c = d.cross(w);
    let gp = d.perp() * (1.0 / l);
    let gd = -w.perp() * (1.0 / l) - d * (c / (l * l * l));
    (c / l, gp, -gd - gp, gd)
}

fn radius_value(x: &[f64], r: Radius, k: f64, g: &mut dyn FnMut(usize, f64)) -> f64 {
    match r {
        Radius::Var(i) => {
            g(i, k);
            val(x, i)
        }
        Radius::Arc { centre, start } => {
            let (l, u) = length_dir(pt(x, start) - pt(x, centre));
            push(g, start, u * k);
            push(g, centre, u * -k);
            l
        }
    }
}

/// Wraps an angle to (-π, π].
fn wrap(a: f64) -> f64 {
    let w = a - TAU * (a / TAU).round();
    if w <= -PI { w + TAU } else { w }
}

impl Residual {
    /// The unknowns this residual depends on.
    pub(crate) fn vars(&self, out: &mut Vec<usize>) {
        let pts = |ps: &[usize], out: &mut Vec<usize>| {
            for &p in ps {
                out.push(p);
                out.push(p.saturating_add(1));
            }
        };
        let rad = |r: &Radius, out: &mut Vec<usize>| match r {
            Radius::Var(i) => out.push(*i),
            Radius::Arc { centre, start } => out.extend([
                *centre,
                centre.saturating_add(1),
                *start,
                start.saturating_add(1),
            ]),
        };
        match self {
            Residual::Linear { terms, .. } => out.extend(terms.iter().map(|t| t.0)),
            Residual::Distance { p, q, .. } => pts(&[*p, *q], out),
            Residual::LineDistance { p, a, b, .. } => pts(&[*p, *a, *b], out),
            Residual::OnCircle { p, centre, radius } => {
                pts(&[*p, *centre], out);
                rad(radius, out);
            }
            Residual::LineTangent {
                a,
                b,
                centre,
                radius,
                ..
            } => {
                pts(&[*a, *b, *centre], out);
                rad(radius, out);
            }
            Residual::CircleTangent { c1, r1, c2, r2, .. } => {
                pts(&[*c1, *c2], out);
                rad(r1, out);
                rad(r2, out);
            }
            Residual::LengthDiff { a1, b1, a2, b2 }
            | Residual::Parallel { a1, b1, a2, b2, .. }
            | Residual::Perpendicular { a1, b1, a2, b2, .. }
            | Residual::Angle { a1, b1, a2, b2, .. } => pts(&[*a1, *b1, *a2, *b2], out),
            Residual::RadiusDiff { r1, r2 } => {
                rad(r1, out);
                rad(r2, out);
            }
            Residual::RadiusValue { radius, .. } => rad(radius, out),
            Residual::SymmetricMid { p, q, a, b } | Residual::SymmetricPerp { p, q, a, b } => {
                pts(&[*p, *q, *a, *b], out)
            }
        }
    }

    /// The residual at `x`; `g(i, v)` is called to add `v` to `∂residual/∂xᵢ` (an index may
    /// be reported more than once; contributions add up).
    pub(crate) fn eval(&self, x: &[f64], g: &mut dyn FnMut(usize, f64)) -> f64 {
        match self {
            Residual::Linear { terms, constant } => {
                let mut s = *constant;
                for &(i, c) in terms {
                    s += c * val(x, i);
                    g(i, c);
                }
                s
            }
            Residual::Distance { p, q, value } => {
                let (l, u) = length_dir(pt(x, *p) - pt(x, *q));
                push(g, *p, u);
                push(g, *q, -u);
                l - value
            }
            Residual::LineDistance { p, a, b, value } => {
                let (f, gp, ga, gb) = signed_distance(pt(x, *p), pt(x, *a), pt(x, *b));
                push(g, *p, gp);
                push(g, *a, ga);
                push(g, *b, gb);
                f - value
            }
            Residual::OnCircle { p, centre, radius } => {
                let (l, u) = length_dir(pt(x, *p) - pt(x, *centre));
                push(g, *p, u);
                push(g, *centre, -u);
                l - radius_value(x, *radius, -1.0, g)
            }
            Residual::LineTangent {
                a,
                b,
                centre,
                radius,
                side,
            } => {
                let (f, gp, ga, gb) = signed_distance(pt(x, *centre), pt(x, *a), pt(x, *b));
                push(g, *centre, gp);
                push(g, *a, ga);
                push(g, *b, gb);
                f - side * radius_value(x, *radius, -side, g)
            }
            Residual::CircleTangent {
                c1,
                r1,
                s1,
                c2,
                r2,
                s2,
            } => {
                let (l, u) = length_dir(pt(x, *c1) - pt(x, *c2));
                push(g, *c1, u);
                push(g, *c2, -u);
                let v1 = radius_value(x, *r1, -s1, g);
                let v2 = radius_value(x, *r2, -s2, g);
                l - s1 * v1 - s2 * v2
            }
            Residual::LengthDiff { a1, b1, a2, b2 } => {
                let (l1, u1) = length_dir(pt(x, *b1) - pt(x, *a1));
                let (l2, u2) = length_dir(pt(x, *b2) - pt(x, *a2));
                push(g, *b1, u1);
                push(g, *a1, -u1);
                push(g, *b2, -u2);
                push(g, *a2, u2);
                l1 - l2
            }
            Residual::RadiusDiff { r1, r2 } => {
                radius_value(x, *r1, 1.0, g) - radius_value(x, *r2, -1.0, g)
            }
            Residual::RadiusValue {
                radius,
                factor,
                value,
            } => factor * radius_value(x, *radius, *factor, g) - value,
            Residual::Parallel {
                a1,
                b1,
                a2,
                b2,
                scale,
            } => {
                // s = (d1 × d2) / (L1 L2); ∂s/∂d1 = -perp(d2)/(L1 L2) - s d1/L1²,
                // ∂s/∂d2 = perp(d1)/(L1 L2) - s d2/L2².
                let d1 = pt(x, *b1) - pt(x, *a1);
                let d2 = pt(x, *b2) - pt(x, *a2);
                let l1 = d1.length().max(MIN_LENGTH);
                let l2 = d2.length().max(MIN_LENGTH);
                let s = d1.cross(d2) / (l1 * l2);
                let g1 = (-d2.perp() * (1.0 / (l1 * l2)) - d1 * (s / (l1 * l1))) * *scale;
                let g2 = (d1.perp() * (1.0 / (l1 * l2)) - d2 * (s / (l2 * l2))) * *scale;
                push(g, *b1, g1);
                push(g, *a1, -g1);
                push(g, *b2, g2);
                push(g, *a2, -g2);
                scale * s
            }
            Residual::Perpendicular {
                a1,
                b1,
                a2,
                b2,
                scale,
            } => {
                // c = (d1 · d2) / (L1 L2); ∂c/∂d1 = d2/(L1 L2) - c d1/L1², likewise for d2.
                let d1 = pt(x, *b1) - pt(x, *a1);
                let d2 = pt(x, *b2) - pt(x, *a2);
                let l1 = d1.length().max(MIN_LENGTH);
                let l2 = d2.length().max(MIN_LENGTH);
                let c = d1.dot(d2) / (l1 * l2);
                let g1 = (d2 * (1.0 / (l1 * l2)) - d1 * (c / (l1 * l1))) * *scale;
                let g2 = (d1 * (1.0 / (l1 * l2)) - d2 * (c / (l2 * l2))) * *scale;
                push(g, *b1, g1);
                push(g, *a1, -g1);
                push(g, *b2, g2);
                push(g, *a2, -g2);
                scale * c
            }
            Residual::Angle {
                a1,
                b1,
                a2,
                b2,
                value,
                scale,
            } => {
                // θ = atan2(d2) - atan2(d1); ∂atan2(d)/∂d = perp(d)/|d|².
                let d1 = pt(x, *b1) - pt(x, *a1);
                let d2 = pt(x, *b2) - pt(x, *a2);
                let q1 = d1.dot(d1).max(MIN_LENGTH * MIN_LENGTH);
                let q2 = d2.dot(d2).max(MIN_LENGTH * MIN_LENGTH);
                let theta = wrap(d2.angle() - d1.angle() - value);
                let g1 = -d1.perp() * (*scale / q1);
                let g2 = d2.perp() * (*scale / q2);
                push(g, *b1, g1);
                push(g, *a1, -g1);
                push(g, *b2, g2);
                push(g, *a2, -g2);
                scale * theta
            }
            Residual::SymmetricMid { p, q, a, b } => {
                let m = (pt(x, *p) + pt(x, *q)) * 0.5;
                let (f, gm, ga, gb) = signed_distance(m, pt(x, *a), pt(x, *b));
                push(g, *p, gm * 0.5);
                push(g, *q, gm * 0.5);
                push(g, *a, ga);
                push(g, *b, gb);
                f
            }
            Residual::SymmetricPerp { p, q, a, b } => {
                // f = d · e / L with d = b - a, e = q - p: ∂f/∂e = d/L,
                // ∂f/∂d = e/L - f d/L².
                let d = pt(x, *b) - pt(x, *a);
                let e = pt(x, *q) - pt(x, *p);
                let l = d.length().max(MIN_LENGTH);
                let f = d.dot(e) / l;
                let ge = d * (1.0 / l);
                let gd = e * (1.0 / l) - d * (f / (l * l));
                push(g, *q, ge);
                push(g, *p, -ge);
                push(g, *b, gd);
                push(g, *a, -gd);
                f
            }
        }
    }

    /// The residual alone.
    pub(crate) fn value(&self, x: &[f64]) -> f64 {
        self.eval(x, &mut |_, _| {})
    }

    /// True when the move from `before` to `after` flips a vector this residual measures a
    /// length along (its direction turns by a right angle or more) or drives a circle's
    /// radius to zero. `|p - q| = d` holds at both `q + d u` and `q - d u`; a step that
    /// jumps through `p = q` to the mirror image is what makes a sketch flip, so the solver
    /// rejects such steps and takes a shorter one.
    pub(crate) fn flips(&self, before: &[f64], after: &[f64]) -> bool {
        let dir = |p: usize, q: usize| {
            let b = pt(before, p) - pt(before, q);
            let a = pt(after, p) - pt(after, q);
            b.length() >= MIN_LENGTH && b.dot(a) <= 0.0
        };
        let rad = |r: &Radius| match *r {
            Radius::Var(i) => val(after, i) <= 0.0,
            Radius::Arc { centre, start } => dir(start, centre),
        };
        match self {
            Residual::Distance { p, q, .. } => dir(*p, *q),
            Residual::OnCircle { p, centre, radius } => dir(*p, *centre) || rad(radius),
            Residual::CircleTangent { c1, r1, c2, r2, .. } => dir(*c1, *c2) || rad(r1) || rad(r2),
            Residual::LengthDiff { a1, b1, a2, b2 } => dir(*b1, *a1) || dir(*b2, *a2),
            Residual::RadiusDiff { r1, r2 } => rad(r1) || rad(r2),
            Residual::RadiusValue { radius, .. } | Residual::LineTangent { radius, .. } => {
                rad(radius)
            }
            _ => false,
        }
    }
}

/// Where each point and circle radius lives in the unknown vector.
pub(crate) struct Layout {
    point_index: BTreeMap<PointId, usize>,
    radius_index: BTreeMap<CurveId, usize>,
    /// The unknowns, read from the sketch.
    pub x: Vec<f64>,
    /// Characteristic size `S` (mm) used to scale angular residuals.
    pub scale: f64,
}

impl Layout {
    pub(crate) fn new(sketch: &Sketch) -> Self {
        let mut x = Vec::new();
        let mut point_index = BTreeMap::new();
        let (mut lo, mut hi) = (Vec2::new(f64::MAX, f64::MAX), Vec2::new(f64::MIN, f64::MIN));
        let mut max_r: f64 = 0.0;
        for (id, p) in sketch.points() {
            point_index.insert(id, x.len());
            x.push(p.x);
            x.push(p.y);
            lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
            hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
        }
        let mut radius_index = BTreeMap::new();
        for (id, c) in sketch.curves() {
            if let Curve::Circle { radius, .. } = c {
                radius_index.insert(id, x.len());
                x.push(*radius);
                max_r = max_r.max(*radius);
            }
        }
        let diag = if point_index.is_empty() {
            0.0
        } else {
            (hi - lo).length()
        };
        let scale = diag.max(2.0 * max_r).max(1.0);
        Layout {
            point_index,
            radius_index,
            x,
            scale,
        }
    }

    pub(crate) fn p(&self, id: PointId) -> Result<usize, SketchError> {
        self.point_index
            .get(&id)
            .copied()
            .ok_or(SketchError::UnknownPoint(id))
    }

    /// The points and their `x` indices.
    pub(crate) fn points(&self) -> impl Iterator<Item = (PointId, usize)> + '_ {
        self.point_index.iter().map(|(k, v)| (*k, *v))
    }

    /// The circles and their radius indices.
    pub(crate) fn radii(&self) -> impl Iterator<Item = (CurveId, usize)> + '_ {
        self.radius_index.iter().map(|(k, v)| (*k, *v))
    }

    fn line(&self, sketch: &Sketch, id: CurveId) -> Result<(usize, usize), SketchError> {
        match sketch.curve(id) {
            Some(Curve::Line { start, end, .. }) => Ok((self.p(*start)?, self.p(*end)?)),
            Some(c) => Err(SketchError::WrongCurveKind {
                curve: id,
                found: c.kind_name(),
                expected: "line",
            }),
            None => Err(SketchError::UnknownCurve(id)),
        }
    }

    /// Centre index and radius of a circle or arc.
    fn circular(&self, sketch: &Sketch, id: CurveId) -> Result<(usize, Radius), SketchError> {
        match sketch.curve(id) {
            Some(Curve::Circle { centre, .. }) => {
                let r = self
                    .radius_index
                    .get(&id)
                    .copied()
                    .ok_or(SketchError::UnknownCurve(id))?;
                Ok((self.p(*centre)?, Radius::Var(r)))
            }
            Some(Curve::Arc { centre, start, .. }) => {
                let c = self.p(*centre)?;
                Ok((
                    c,
                    Radius::Arc {
                        centre: c,
                        start: self.p(*start)?,
                    },
                ))
            }
            Some(c) => Err(SketchError::WrongCurveKind {
                curve: id,
                found: c.kind_name(),
                expected: "circle or arc",
            }),
            None => Err(SketchError::UnknownCurve(id)),
        }
    }

    fn radius_now(&self, r: Radius) -> f64 {
        radius_value(&self.x, r, 0.0, &mut |_, _| {})
    }
}

fn sign_of(v: f64) -> f64 {
    if v < 0.0 { -1.0 } else { 1.0 }
}

/// Compiles the sketch into equations: every constraint in id order, then each arc's
/// equal-radius equation (curve id order). The arcs come last so that when constraints
/// already imply an arc's equal radii (an arc tangent to two placed lines at its ends), it
/// is the arc's own equation that counts as dependent, not a constraint the user added. Branch choices (sides, inside or outside
/// tangency) are taken from the current geometry in `layout.x`.
pub(crate) fn compile(sketch: &Sketch, layout: &Layout) -> Result<Vec<Equation>, SketchError> {
    let mut eqs = Vec::new();
    let x = &layout.x;
    let scale = layout.scale;
    for (id, c) in sketch.constraints() {
        let mut add = |residual: Residual| {
            eqs.push(Equation {
                residual,
                owner: Some(id),
            })
        };
        let diff = |i: usize, j: usize| Residual::Linear {
            terms: vec![(i, 1.0), (j, -1.0)],
            constant: 0.0,
        };
        match c {
            Constraint::Coincident { a, b } => {
                let (a, b) = (layout.p(*a)?, layout.p(*b)?);
                add(diff(a, b));
                add(diff(a + 1, b + 1));
            }
            Constraint::PointOnLine { point, line } => {
                let (a, b) = layout.line(sketch, *line)?;
                add(Residual::LineDistance {
                    p: layout.p(*point)?,
                    a,
                    b,
                    value: 0.0,
                });
            }
            Constraint::PointOnCircle { point, curve } => {
                let (centre, radius) = layout.circular(sketch, *curve)?;
                add(Residual::OnCircle {
                    p: layout.p(*point)?,
                    centre,
                    radius,
                });
            }
            Constraint::Horizontal { line } => {
                let (a, b) = layout.line(sketch, *line)?;
                add(diff(b + 1, a + 1));
            }
            Constraint::Vertical { line } => {
                let (a, b) = layout.line(sketch, *line)?;
                add(diff(b, a));
            }
            Constraint::Parallel { a, b } => {
                let ((a1, b1), (a2, b2)) = (layout.line(sketch, *a)?, layout.line(sketch, *b)?);
                add(Residual::Parallel {
                    a1,
                    b1,
                    a2,
                    b2,
                    scale,
                });
            }
            Constraint::Perpendicular { a, b } => {
                let ((a1, b1), (a2, b2)) = (layout.line(sketch, *a)?, layout.line(sketch, *b)?);
                add(Residual::Perpendicular {
                    a1,
                    b1,
                    a2,
                    b2,
                    scale,
                });
            }
            Constraint::Angle { a, b, value } => {
                let ((a1, b1), (a2, b2)) = (layout.line(sketch, *a)?, layout.line(sketch, *b)?);
                add(Residual::Angle {
                    a1,
                    b1,
                    a2,
                    b2,
                    value: *value,
                    scale,
                });
            }
            Constraint::Tangent { a, b } => {
                let a_is_line = sketch.curve(*a).is_some_and(Curve::is_line);
                let b_is_line = sketch.curve(*b).is_some_and(Curve::is_line);
                if let Some(shared) = shared_end(sketch, *a, *b) {
                    // The curves meet at a common endpoint, which lies on both: tangency
                    // is then the radius at that point being square to the line (or the
                    // two centres being in line with it). This form stays well
                    // conditioned, where "distance from the centre equals the radius" has a
                    // zero gradient along the line at the touching point and would report a
                    // freedom that is not there.
                    let p = layout.p(shared)?;
                    if a_is_line || b_is_line {
                        let (line, circ) = if a_is_line { (*a, *b) } else { (*b, *a) };
                        let (la, lb) = layout.line(sketch, line)?;
                        let (centre, _) = layout.circular(sketch, circ)?;
                        add(Residual::SymmetricPerp {
                            p,
                            q: centre,
                            a: la,
                            b: lb,
                        });
                    } else {
                        let (c1, _) = layout.circular(sketch, *a)?;
                        let (c2, _) = layout.circular(sketch, *b)?;
                        add(Residual::LineDistance {
                            p,
                            a: c1,
                            b: c2,
                            value: 0.0,
                        });
                    }
                } else if a_is_line || b_is_line {
                    let (line, circ) = if a_is_line { (*a, *b) } else { (*b, *a) };
                    let (la, lb) = layout.line(sketch, line)?;
                    let (centre, radius) = layout.circular(sketch, circ)?;
                    // Keep the circle on the side of the line it is on now.
                    let (f, ..) = signed_distance(pt(x, centre), pt(x, la), pt(x, lb));
                    add(Residual::LineTangent {
                        a: la,
                        b: lb,
                        centre,
                        radius,
                        side: sign_of(f),
                    });
                } else {
                    let (c1, r1) = layout.circular(sketch, *a)?;
                    let (c2, r2) = layout.circular(sketch, *b)?;
                    // Outside or inside tangency, whichever is nearer now.
                    let d = (pt(x, c1) - pt(x, c2)).length();
                    let (v1, v2) = (layout.radius_now(r1), layout.radius_now(r2));
                    let external = (d - (v1 + v2)).abs() <= (d - (v1 - v2).abs()).abs();
                    let (s1, s2) = if external {
                        (1.0, 1.0)
                    } else if v1 >= v2 {
                        (1.0, -1.0)
                    } else {
                        (-1.0, 1.0)
                    };
                    add(Residual::CircleTangent {
                        c1,
                        r1,
                        s1,
                        c2,
                        r2,
                        s2,
                    });
                }
            }
            Constraint::Equal { a, b } => {
                if sketch.curve(*a).is_some_and(Curve::is_line) {
                    let ((a1, b1), (a2, b2)) = (layout.line(sketch, *a)?, layout.line(sketch, *b)?);
                    add(Residual::LengthDiff { a1, b1, a2, b2 });
                } else {
                    let (_, r1) = layout.circular(sketch, *a)?;
                    let (_, r2) = layout.circular(sketch, *b)?;
                    add(Residual::RadiusDiff { r1, r2 });
                }
            }
            Constraint::Fix { point, at } => {
                let p = layout.p(*point)?;
                add(Residual::Linear {
                    terms: vec![(p, 1.0)],
                    constant: -at.x,
                });
                add(Residual::Linear {
                    terms: vec![(p + 1, 1.0)],
                    constant: -at.y,
                });
            }
            Constraint::Midpoint { point, line } => {
                let m = layout.p(*point)?;
                let (a, b) = layout.line(sketch, *line)?;
                for k in 0..2 {
                    add(Residual::Linear {
                        terms: vec![(m + k, 1.0), (a + k, -0.5), (b + k, -0.5)],
                        constant: 0.0,
                    });
                }
            }
            Constraint::Symmetric { a, b, line } => {
                let (p, q) = (layout.p(*a)?, layout.p(*b)?);
                let (la, lb) = layout.line(sketch, *line)?;
                add(Residual::SymmetricMid { p, q, a: la, b: lb });
                add(Residual::SymmetricPerp { p, q, a: la, b: lb });
            }
            Constraint::Distance { a, b, value } => {
                add(Residual::Distance {
                    p: layout.p(*a)?,
                    q: layout.p(*b)?,
                    value: *value,
                });
            }
            Constraint::PointLineDistance { point, line, value } => {
                let p = layout.p(*point)?;
                let (a, b) = layout.line(sketch, *line)?;
                // Keep the point on the side of the line it is on now.
                let (f, ..) = signed_distance(pt(x, p), pt(x, a), pt(x, b));
                add(Residual::LineDistance {
                    p,
                    a,
                    b,
                    value: sign_of(f) * value,
                });
            }
            Constraint::Length { line, value } => {
                let (a, b) = layout.line(sketch, *line)?;
                add(Residual::Distance {
                    p: b,
                    q: a,
                    value: *value,
                });
            }
            Constraint::Radius { curve, value } | Constraint::Diameter { curve, value } => {
                let factor = if matches!(c, Constraint::Diameter { .. }) {
                    2.0
                } else {
                    1.0
                };
                let (_, radius) = layout.circular(sketch, *curve)?;
                add(Residual::RadiusValue {
                    radius,
                    factor,
                    value: *value,
                });
            }
        }
    }
    for (_, c) in sketch.curves() {
        if let Curve::Arc {
            centre, start, end, ..
        } = c
        {
            let (c, s, e) = (layout.p(*centre)?, layout.p(*start)?, layout.p(*end)?);
            eqs.push(Equation {
                residual: Residual::RadiusDiff {
                    r1: Radius::Arc {
                        centre: c,
                        start: e,
                    },
                    r2: Radius::Arc {
                        centre: c,
                        start: s,
                    },
                },
                owner: None,
            });
        }
    }
    Ok(eqs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks every analytic gradient against central differences at a generic point.
    #[test]
    fn analytic_gradients_match_finite_differences() {
        // Points at x[0..2], x[2..4], x[4..6], x[6..8], x[8..10]; a circle radius at x[10].
        let x = vec![0.3, -0.2, 4.1, 1.3, 1.7, 3.9, -2.2, 0.8, 2.5, 2.9, 1.6];
        let r = Radius::Var(10);
        let arc_r = Radius::Arc {
            centre: 4,
            start: 8,
        };
        let residuals = vec![
            Residual::Linear {
                terms: vec![(0, 1.0), (3, -0.5)],
                constant: 2.0,
            },
            Residual::Distance {
                p: 0,
                q: 2,
                value: 1.0,
            },
            Residual::LineDistance {
                p: 4,
                a: 0,
                b: 2,
                value: 0.5,
            },
            Residual::OnCircle {
                p: 6,
                centre: 4,
                radius: r,
            },
            Residual::OnCircle {
                p: 6,
                centre: 4,
                radius: arc_r,
            },
            Residual::LineTangent {
                a: 0,
                b: 2,
                centre: 6,
                radius: r,
                side: -1.0,
            },
            Residual::CircleTangent {
                c1: 6,
                r1: r,
                s1: 1.0,
                c2: 4,
                r2: arc_r,
                s2: -1.0,
            },
            Residual::LengthDiff {
                a1: 0,
                b1: 2,
                a2: 4,
                b2: 6,
            },
            Residual::RadiusDiff { r1: r, r2: arc_r },
            Residual::RadiusValue {
                radius: arc_r,
                factor: 2.0,
                value: 3.0,
            },
            Residual::Parallel {
                a1: 0,
                b1: 2,
                a2: 4,
                b2: 6,
                scale: 7.0,
            },
            Residual::Perpendicular {
                a1: 0,
                b1: 2,
                a2: 4,
                b2: 6,
                scale: 7.0,
            },
            Residual::Angle {
                a1: 0,
                b1: 2,
                a2: 4,
                b2: 6,
                value: 0.4,
                scale: 7.0,
            },
            Residual::SymmetricMid {
                p: 4,
                q: 6,
                a: 0,
                b: 2,
            },
            Residual::SymmetricPerp {
                p: 4,
                q: 6,
                a: 0,
                b: 2,
            },
        ];
        let h = 1e-6;
        for res in &residuals {
            let mut grad = vec![0.0; x.len()];
            res.eval(&x, &mut |i, v| grad[i] += v);
            for i in 0..x.len() {
                let mut xp = x.clone();
                let mut xm = x.clone();
                xp[i] += h;
                xm[i] -= h;
                let fd = (res.value(&xp) - res.value(&xm)) / (2.0 * h);
                assert!(
                    (fd - grad[i]).abs() < 1e-6,
                    "{res:?}: d/dx{i} analytic {} numeric {fd}",
                    grad[i]
                );
            }
        }
    }

    #[test]
    fn degenerate_geometry_gives_finite_values() {
        // All points at the origin, zero radius.
        let x = vec![0.0; 9];
        let r = Radius::Var(8);
        let residuals = [
            Residual::Distance {
                p: 0,
                q: 2,
                value: 1.0,
            },
            Residual::LineDistance {
                p: 4,
                a: 0,
                b: 2,
                value: 0.0,
            },
            Residual::CircleTangent {
                c1: 0,
                r1: r,
                s1: 1.0,
                c2: 2,
                r2: r,
                s2: 1.0,
            },
            Residual::Parallel {
                a1: 0,
                b1: 2,
                a2: 4,
                b2: 6,
                scale: 1.0,
            },
            Residual::Angle {
                a1: 0,
                b1: 2,
                a2: 4,
                b2: 6,
                value: 1.0,
                scale: 1.0,
            },
        ];
        for res in &residuals {
            let mut ok = true;
            let v = res.eval(&x, &mut |_, g| ok &= g.is_finite());
            assert!(v.is_finite() && ok, "{res:?}");
        }
    }

    #[test]
    fn angles_wrap_to_a_half_open_turn() {
        assert!((wrap(3.0 * PI) - PI).abs() < 1e-12);
        assert!((wrap(-PI) - PI).abs() < 1e-12);
        assert!((wrap(0.5) - 0.5).abs() < 1e-15);
        assert!((wrap(TAU + 0.5) - 0.5).abs() < 1e-12);
    }
}
