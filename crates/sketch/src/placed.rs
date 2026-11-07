//! Shapes placed by points or by the curves they touch, and moving, rotating, scaling or
//! copying chosen geometry. Each is a [`crate::ToolOp`].
//!
//! # Three-point rectangle
//! The first two points are one side; the third sets how far the opposite side is (its
//! distance from the first side's line, on the third point's side). The four lines are
//! tied by one perpendicular and two parallel constraints: a parallelogram with one right
//! angle is a rectangle, so it keeps 5 degrees of freedom (place, turn, width, height).
//!
//! # Circles through points
//! Two points are the ends of a diameter: the centre is their midpoint. Three points give
//! the circumcentre, from `|c - p|² = |c - q|² = |c - s|²` solved relative to the first
//! point: with `m = q - p`, `e = s - p`, `c = p + (e.y |m|² - m.y |e|², m.x |e|² - e.x |m|²)
//! / (2 m × e)`.
//!
//! # Tangent circle
//! A circle tangent to two or three lines, circles or arcs is solved, not computed: the
//! circle starts at the guess (centre `near`; radius the given one with two curves, or the
//! mean distance from `near` to the three curves) and tangent constraints to each curve
//! are added, so the solver finds the nearest solution, on the side of each curve the
//! guess is on. With two curves the radius is a dimension.
//!
//! # Transforms
//! [`SketchTransform`] moves, turns or scales points. In place, the chosen curves' points
//! (and the chosen points) move; circles' radii scale; constraints among the moved
//! geometry are rewritten (fixed points move with it, horizontal and vertical swap under a
//! quarter turn and are dropped under any other turn, dimensions scale), and constraints
//! that tie moved geometry to geometry that stays are dropped, since they would pull it
//! back. A copy leaves the original where it is and copies its constraints the same way.

use crate::constraint::Constraint;
use crate::entity::{Curve, CurveId, PointId};
use crate::sketch::Sketch;
use crate::tools::{ToolError, check_length, check_point, refuse};
use chamfera_geom::{LINEAR_TOLERANCE, MAX_COORDINATE, Point2, Vec2};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::FRAC_PI_2;

/// A move, turn or scale of sketch geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SketchTransform {
    /// Moves by `(dx, dy)` mm.
    Translate { dx: f64, dy: f64 },
    /// Turns counter-clockwise by `angle` radians about `centre`.
    Rotate { centre: Point2, angle: f64 },
    /// Scales by `factor` (above 0) about `centre`.
    Scale { centre: Point2, factor: f64 },
}

impl SketchTransform {
    /// Where `p` goes.
    pub fn apply(&self, p: Point2) -> Point2 {
        match *self {
            SketchTransform::Translate { dx, dy } => p + Vec2::new(dx, dy),
            SketchTransform::Rotate { centre, angle } => {
                let (s, c) = angle.sin_cos();
                let d = p - centre;
                centre + Vec2::new(c * d.x - s * d.y, s * d.x + c * d.y)
            }
            SketchTransform::Scale { centre, factor } => centre + (p - centre) * factor,
        }
    }

    /// How lengths change.
    fn factor(&self) -> f64 {
        match *self {
            SketchTransform::Scale { factor, .. } => factor,
            _ => 1.0,
        }
    }

    /// How horizontal and vertical relations carry over: kept (`Some(false)`), swapped
    /// (`Some(true)`, a quarter turn) or lost (`None`, any other turn).
    fn axis_turns(&self) -> Option<bool> {
        match *self {
            SketchTransform::Rotate { angle, .. } => {
                let quarter = angle / FRAC_PI_2;
                let k = quarter.round();
                if (quarter - k).abs() > 1e-12 {
                    None
                } else {
                    Some((k as i64).rem_euclid(2) == 1)
                }
            }
            _ => Some(false),
        }
    }

    fn check(&self) -> Result<(), ToolError> {
        match *self {
            SketchTransform::Translate { dx, dy } => {
                for (what, v) in [("dx", dx), ("dy", dy)] {
                    if !(v.is_finite() && v.abs() <= MAX_COORDINATE) {
                        return refuse(format!("`{what}` must be a finite length"));
                    }
                }
            }
            SketchTransform::Rotate { centre, angle } => {
                check_point("the centre", centre)?;
                if !angle.is_finite() {
                    return refuse("the angle must be a finite number");
                }
            }
            SketchTransform::Scale { centre, factor } => {
                check_point("the centre", centre)?;
                if !(factor.is_finite() && factor > 1e-6 && factor <= 1e6) {
                    return refuse("the scale factor must be above 0 and at most 1e6");
                }
            }
        }
        Ok(())
    }

    /// `c` as it carries over to the moved geometry, ids mapped by `mp` and `mc`; `None`
    /// when it no longer holds (a horizontal line turned by 30 degrees).
    fn constraint(
        &self,
        c: &Constraint,
        mp: &dyn Fn(PointId) -> PointId,
        mc: &dyn Fn(CurveId) -> CurveId,
    ) -> Option<Constraint> {
        use Constraint as C;
        let f = self.factor();
        let turns = self.axis_turns();
        // Some(swapped) for a relation along an axis.
        let axis = |horizontal: bool| turns.map(|swap| horizontal != swap);
        Some(match *c {
            C::Horizontal { line } | C::Vertical { line } => {
                if axis(matches!(c, C::Horizontal { .. }))? {
                    C::Horizontal { line: mc(line) }
                } else {
                    C::Vertical { line: mc(line) }
                }
            }
            C::HorizontalPoints { a, b } | C::VerticalPoints { a, b } => {
                let (a, b) = (mp(a), mp(b));
                if axis(matches!(c, C::HorizontalPoints { .. }))? {
                    C::HorizontalPoints { a, b }
                } else {
                    C::VerticalPoints { a, b }
                }
            }
            C::HorizontalDistance { a, b, value } | C::VerticalDistance { a, b, value } => {
                let (a, b, value) = (mp(a), mp(b), value * f);
                if axis(matches!(c, C::HorizontalDistance { .. }))? {
                    C::HorizontalDistance { a, b, value }
                } else {
                    C::VerticalDistance { a, b, value }
                }
            }
            C::Fix { point, at } => C::Fix {
                point: mp(point),
                at: self.apply(at),
            },
            C::Coincident { a, b } => C::Coincident { a: mp(a), b: mp(b) },
            C::PointOnLine { point, line } => C::PointOnLine {
                point: mp(point),
                line: mc(line),
            },
            C::PointOnCircle { point, curve } => C::PointOnCircle {
                point: mp(point),
                curve: mc(curve),
            },
            C::Parallel { a, b } => C::Parallel { a: mc(a), b: mc(b) },
            C::Perpendicular { a, b } => C::Perpendicular { a: mc(a), b: mc(b) },
            C::Tangent { a, b } => C::Tangent { a: mc(a), b: mc(b) },
            C::Equal { a, b } => C::Equal { a: mc(a), b: mc(b) },
            C::Concentric { a, b } => C::Concentric { a: mc(a), b: mc(b) },
            C::Collinear { a, b } => C::Collinear { a: mc(a), b: mc(b) },
            C::Midpoint { point, line } => C::Midpoint {
                point: mp(point),
                line: mc(line),
            },
            C::Symmetric { a, b, line } => C::Symmetric {
                a: mp(a),
                b: mp(b),
                line: mc(line),
            },
            C::Distance { a, b, value } => C::Distance {
                a: mp(a),
                b: mp(b),
                value: value * f,
            },
            C::PointLineDistance { point, line, value } => C::PointLineDistance {
                point: mp(point),
                line: mc(line),
                value: value * f,
            },
            C::Length { line, value } => C::Length {
                line: mc(line),
                value: value * f,
            },
            C::Radius { curve, value } => C::Radius {
                curve: mc(curve),
                value: value * f,
            },
            C::Diameter { curve, value } => C::Diameter {
                curve: mc(curve),
                value: value * f,
            },
            C::ArcLength { curve, value } => C::ArcLength {
                curve: mc(curve),
                value: value * f,
            },
            C::Angle { a, b, value } => C::Angle {
                a: mc(a),
                b: mc(b),
                value,
            },
        })
    }
}

/// The distance from `p` to a line, circle or arc's carrier (the infinite line or full
/// circle), or `None` for another kind of curve.
fn carrier_distance(s: &Sketch, curve: &Curve, p: Point2) -> Option<f64> {
    match curve {
        Curve::Line { start, end, .. } => {
            let (a, b) = (s.point(*start)?, s.point(*end)?);
            let u = (b - a).normalized()?;
            Some(u.cross(p - a).abs())
        }
        Curve::Circle { centre, .. } | Curve::Arc { centre, .. } => {
            let c = s.point(*centre)?;
            let r = match curve {
                Curve::Circle { radius, .. } => *radius,
                Curve::Arc { start, .. } => c.distance(s.point(*start)?),
                _ => return None,
            };
            Some((c.distance(p) - r).abs())
        }
        _ => None,
    }
}

impl Sketch {
    pub(crate) fn three_point_rectangle(
        &mut self,
        a: Point2,
        b: Point2,
        c: Point2,
    ) -> Result<(), ToolError> {
        for p in [a, b, c] {
            check_point("a rectangle point", p)?;
        }
        let Some(u) = (b - a).normalized() else {
            return refuse("the first side has no length");
        };
        check_length("the first side", a.distance(b))?;
        let n = u.perp();
        let h = n.dot(c - a);
        if h.abs() <= LINEAR_TOLERANCE {
            return refuse(
                "the third point is on the first side's line: a rectangle needs a height",
            );
        }
        let corners = [a, b, b + n * h, a + n * h];
        let mut pts = Vec::with_capacity(4);
        for p in corners {
            check_point("a rectangle corner", p)?;
            pts.push(self.add_point(p)?);
        }
        let mut lines = Vec::with_capacity(4);
        for k in 0..4 {
            let (Some(s), Some(e)) = (pts.get(k), pts.get((k + 1) % 4)) else {
                return refuse("the rectangle lost a corner");
            };
            lines.push(self.add_line(*s, *e)?);
        }
        let [l0, l1, l2, l3] = lines.as_slice() else {
            return refuse("the rectangle lost a side");
        };
        self.add_constraint(Constraint::Perpendicular { a: *l0, b: *l1 })?;
        self.add_constraint(Constraint::Parallel { a: *l0, b: *l2 })?;
        self.add_constraint(Constraint::Parallel { a: *l1, b: *l3 })?;
        Ok(())
    }

    pub(crate) fn two_point_circle(&mut self, a: Point2, b: Point2) -> Result<(), ToolError> {
        check_point("a circle point", a)?;
        check_point("a circle point", b)?;
        let r = 0.5 * a.distance(b);
        check_length("the circle's radius", r)?;
        let centre = self.add_point(a.lerp(b, 0.5))?;
        self.add_circle(centre, r)?;
        Ok(())
    }

    pub(crate) fn three_point_circle(
        &mut self,
        p: Point2,
        q: Point2,
        s: Point2,
    ) -> Result<(), ToolError> {
        for k in [p, q, s] {
            check_point("a circle point", k)?;
        }
        let (m, e) = (q - p, s - p);
        let den = 2.0 * m.cross(e);
        if den.abs() <= LINEAR_TOLERANCE * e.length().max(m.length()) {
            return refuse("the three points are in a line");
        }
        let (mm, ee) = (m.dot(m), e.dot(e));
        let c = p + Vec2::new(e.y * mm - m.y * ee, m.x * ee - e.x * mm) * (1.0 / den);
        check_point("the circle's centre", c)?;
        let r = c.distance(p);
        check_length("the circle's radius", r)?;
        let centre = self.add_point(c)?;
        self.add_circle(centre, r)?;
        Ok(())
    }

    pub(crate) fn tangent_circle(
        &mut self,
        curves: &[CurveId],
        near: Point2,
        radius: Option<f64>,
    ) -> Result<(), ToolError> {
        check_point("the guess for the centre", near)?;
        if !(2..=3).contains(&curves.len()) {
            return refuse("a tangent circle touches two or three curves");
        }
        let distinct: BTreeSet<CurveId> = curves.iter().copied().collect();
        if distinct.len() != curves.len() {
            return refuse("a tangent circle touches different curves");
        }
        let mut gaps = Vec::new();
        for id in curves {
            let c = self.curve_of_tool(*id)?;
            if !(c.is_line() || c.is_circular()) {
                return refuse(format!(
                    "{id} is a {}: a tangent circle touches lines, circles and arcs",
                    c.kind_name()
                ));
            }
            if let Some(d) = carrier_distance(self, &c, near) {
                gaps.push(d);
            }
        }
        let r = match (curves.len(), radius) {
            (2, Some(r)) => {
                check_length("the radius", r)?;
                r
            }
            (2, None) => return refuse("a circle tangent to two curves needs a radius"),
            (_, Some(_)) => return refuse("a circle tangent to three curves takes no radius"),
            _ => gaps.iter().sum::<f64>() / gaps.len().max(1) as f64,
        };
        check_length("the first guess of the radius", r)?;
        let centre = self.add_point(near)?;
        let circle = self.add_circle(centre, r)?;
        for id in curves {
            self.add_constraint(Constraint::Tangent { a: *id, b: circle })?;
        }
        if curves.len() == 2 {
            self.add_constraint(Constraint::Radius {
                curve: circle,
                value: r,
            })?;
        }
        Ok(())
    }

    fn curve_of_tool(&self, id: CurveId) -> Result<Curve, ToolError> {
        self.curve(id)
            .cloned()
            .ok_or_else(|| crate::SketchError::UnknownCurve(id).into())
    }

    pub(crate) fn transform(
        &mut self,
        curves: &[CurveId],
        points: &[PointId],
        t: SketchTransform,
        copy: bool,
    ) -> Result<(), ToolError> {
        t.check()?;
        let (cs, ps) = self.copy_set(curves, points, None)?;
        let moved: BTreeSet<PointId> = ps.iter().copied().collect();
        // Curves all of whose points move move too, chosen or not.
        let moved_curves: BTreeSet<CurveId> = self
            .curves()
            .filter(|(_, c)| c.points().iter().all(|p| moved.contains(p)))
            .map(|(id, _)| id)
            .collect();
        let internal = |c: &Constraint| {
            c.point_refs().iter().all(|p| moved.contains(p))
                && c.curve_refs().iter().all(|k| moved_curves.contains(k))
        };
        let touches = |c: &Constraint| {
            c.point_refs().iter().any(|p| moved.contains(p))
                || c.curve_refs().iter().any(|k| moved_curves.contains(k))
        };
        if copy {
            let mut map = BTreeMap::new();
            for p in &ps {
                let q = t.apply(self.pos_of(*p)?);
                check_point("a copied point", q)?;
                map.insert(*p, self.add_point(q)?);
            }
            let curve_map = self.copy_curves(&cs, &map, false)?;
            // A copy keeps the shape of what it copies: its circles are scaled and the
            // constraints among the copied geometry carry over.
            for new in curve_map.values() {
                self.scale_radius(*new, t.factor())?;
            }
            let mp = |p: PointId| map.get(&p).copied().unwrap_or(p);
            let mc = |k: CurveId| curve_map.get(&k).copied().unwrap_or(k);
            let copied_curves: BTreeSet<CurveId> = cs.iter().copied().collect();
            let originals: Vec<Constraint> = self
                .constraints()
                .filter(|(id, _)| !self.is_driven(*id))
                .map(|(_, c)| c.clone())
                .filter(|c| {
                    c.point_refs().iter().all(|p| moved.contains(p))
                        && c.curve_refs().iter().all(|k| copied_curves.contains(k))
                })
                .collect();
            for c in originals {
                if let Some(copy) = t.constraint(&c, &mp, &mc) {
                    self.add_constraint(copy)?;
                }
            }
            return Ok(());
        }
        for p in &ps {
            let q = t.apply(self.pos_of(*p)?);
            check_point("a moved point", q)?;
            self.set_point_unchecked(*p, q);
        }
        for k in &moved_curves {
            self.scale_radius(*k, t.factor())?;
        }
        let same_p = |p: PointId| p;
        let same_c = |k: CurveId| k;
        self.retain_constraints(|c| {
            !touches(c) || (internal(c) && t.constraint(c, &same_p, &same_c).is_some())
        });
        self.rewrite_constraints(|c| {
            if internal(c) {
                t.constraint(c, &same_p, &same_c)
            } else {
                None
            }
        });
        Ok(())
    }

    fn pos_of(&self, id: PointId) -> Result<Point2, ToolError> {
        self.point(id)
            .ok_or_else(|| crate::SketchError::UnknownPoint(id).into())
    }

    /// Multiplies a circle's radius by `factor` (other curves have none of their own).
    fn scale_radius(&mut self, id: CurveId, factor: f64) -> Result<(), ToolError> {
        if let Some(Curve::Circle {
            centre,
            radius,
            construction,
        }) = self.curve(id).cloned()
            && (factor - 1.0).abs() > 0.0
        {
            self.replace_curve(
                id,
                Curve::Circle {
                    centre,
                    radius: radius * factor,
                    construction,
                },
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Constraint, Sketch, ToolOp};
    use chamfera_geom::Point2;
    use std::f64::consts::{FRAC_PI_2, PI};

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    fn areas(s: &Sketch) -> Vec<f64> {
        let mut a: Vec<f64> = s.profiles().iter().map(|r| r.area).collect();
        a.sort_by(f64::total_cmp);
        a
    }

    fn circle(s: &Sketch) -> (Point2, f64) {
        s.curves()
            .find_map(|(_, c)| match c {
                crate::Curve::Circle {
                    centre,
                    radius,
                    construction: false,
                } => Some((s.point(*centre).unwrap(), *radius)),
                _ => None,
            })
            .unwrap()
    }

    /// A rectangle at 30 degrees: first side (0, 0) to (40 cos 30°, 40 sin 30°), height
    /// 15 to the left: area 40 × 15 = 600, and 5 degrees of freedom.
    #[test]
    fn a_three_point_rectangle_is_a_turned_rectangle_with_five_freedoms() {
        let mut s = Sketch::new();
        let (sn, c) = (PI / 6.0).sin_cos();
        let b = p(40.0 * c, 40.0 * sn);
        // A third point 15 off the first side, and anywhere along it.
        let third = p(10.0 * c - 15.0 * sn, 10.0 * sn + 15.0 * c);
        s.apply_tool(&ToolOp::ThreePointRectangle {
            a: p(0.0, 0.0),
            b,
            c: third,
        })
        .unwrap();
        let a = areas(&s);
        assert_eq!(a.len(), 1);
        assert!((a[0] - 600.0).abs() < 1e-9, "{a:?}");
        assert_eq!(s.analyze().unwrap().dof, 5);
        // On the first side's line it has no height.
        assert!(
            s.apply_tool(&ToolOp::ThreePointRectangle {
                a: p(0.0, 0.0),
                b: p(10.0, 0.0),
                c: p(20.0, 0.0),
            })
            .is_err()
        );
    }

    /// Two points (0, 0) and (6, 8) at the ends of a diameter: centre (3, 4), radius 5.
    /// Three points (0, 0), (8, 0), (0, 6) on a right angle: the hypotenuse is a diameter,
    /// so centre (4, 3) and radius 5.
    #[test]
    fn circles_through_two_and_three_points() {
        let mut s = Sketch::new();
        s.apply_tool(&ToolOp::TwoPointCircle {
            a: p(0.0, 0.0),
            b: p(6.0, 8.0),
        })
        .unwrap();
        let (c, r) = circle(&s);
        assert!(c.distance(p(3.0, 4.0)) < 1e-12 && (r - 5.0).abs() < 1e-12);
        let mut s = Sketch::new();
        s.apply_tool(&ToolOp::ThreePointCircle {
            a: p(0.0, 0.0),
            b: p(8.0, 0.0),
            c: p(0.0, 6.0),
        })
        .unwrap();
        let (c, r) = circle(&s);
        assert!(c.distance(p(4.0, 3.0)) < 1e-12 && (r - 5.0).abs() < 1e-12);
        // Points in a line, and a zero diameter, are refused.
        for op in [
            ToolOp::ThreePointCircle {
                a: p(0.0, 0.0),
                b: p(1.0, 1.0),
                c: p(2.0, 2.0),
            },
            ToolOp::TwoPointCircle {
                a: p(1.0, 1.0),
                b: p(1.0, 1.0),
            },
        ] {
            assert!(Sketch::new().apply_tool(&op).is_err());
        }
    }

    /// A triangle with legs 30 and 40 (hypotenuse 50) has inradius (30 + 40 - 50) / 2 = 10
    /// and its incentre at (10, 10).
    #[test]
    fn a_circle_tangent_to_three_lines_is_the_incircle() {
        let mut s = Sketch::new();
        let a = s.add_point(p(0.0, 0.0)).unwrap();
        let b = s.add_point(p(30.0, 0.0)).unwrap();
        let c = s.add_point(p(0.0, 40.0)).unwrap();
        let l1 = s.add_line(a, b).unwrap();
        let l2 = s.add_line(b, c).unwrap();
        let l3 = s.add_line(c, a).unwrap();
        for p in [a, b, c] {
            s.fix_point(p).unwrap();
        }
        s.apply_tool(&ToolOp::TangentCircle {
            curves: vec![l1, l2, l3],
            near: p(8.0, 12.0),
            radius: None,
        })
        .unwrap();
        let (centre, r) = circle(&s);
        assert!(centre.distance(p(10.0, 10.0)) < 1e-7, "{centre:?}");
        assert!((r - 10.0).abs() < 1e-7, "{r}");
        assert_eq!(s.analyze().unwrap().dof, 0);
    }

    /// Two perpendicular lines along the axes and radius 5: the circle in the corner has
    /// its centre at (5, 5).
    #[test]
    fn a_circle_tangent_to_two_lines_takes_a_radius() {
        let mut s = Sketch::new();
        let o = s.add_point(p(0.0, 0.0)).unwrap();
        let x = s.add_point(p(50.0, 0.0)).unwrap();
        let y = s.add_point(p(0.0, 50.0)).unwrap();
        let lx = s.add_line(o, x).unwrap();
        let ly = s.add_line(o, y).unwrap();
        for k in [o, x, y] {
            s.fix_point(k).unwrap();
        }
        let no_radius = ToolOp::TangentCircle {
            curves: vec![lx, ly],
            near: p(4.0, 6.0),
            radius: None,
        };
        assert!(s.apply_tool(&no_radius).is_err());
        s.apply_tool(&ToolOp::TangentCircle {
            curves: vec![lx, ly],
            near: p(4.0, 6.0),
            radius: Some(5.0),
        })
        .unwrap();
        let (centre, r) = circle(&s);
        assert!(centre.distance(p(5.0, 5.0)) < 1e-7, "{centre:?}");
        assert!((r - 5.0).abs() < 1e-9);
        assert!(
            s.apply_tool(&ToolOp::TangentCircle {
                curves: vec![lx, lx],
                near: p(4.0, 6.0),
                radius: Some(5.0),
            })
            .is_err()
        );
    }

    /// A 20 x 10 rectangle with its corner fixed at (0, 0) and its sides dimensioned.
    fn plate(s: &mut Sketch) -> Vec<crate::CurveId> {
        let pts: Vec<_> = [(0.0, 0.0), (20.0, 0.0), (20.0, 10.0), (0.0, 10.0)]
            .iter()
            .map(|(x, y)| s.add_point(p(*x, *y)).unwrap())
            .collect();
        let lines: Vec<_> = (0..4)
            .map(|k| s.add_line(pts[k], pts[(k + 1) % 4]).unwrap())
            .collect();
        s.fix_point(pts[0]).unwrap();
        for (k, l) in lines.iter().enumerate() {
            s.add_constraint(if k % 2 == 0 {
                Constraint::Horizontal { line: *l }
            } else {
                Constraint::Vertical { line: *l }
            })
            .unwrap();
        }
        s.add_constraint(Constraint::Length {
            line: lines[0],
            value: 20.0,
        })
        .unwrap();
        s.add_constraint(Constraint::Length {
            line: lines[1],
            value: 10.0,
        })
        .unwrap();
        s.solve().unwrap();
        lines
    }

    fn bounds(s: &Sketch) -> (Point2, Point2) {
        s.points().fold(
            (p(f64::MAX, f64::MAX), p(f64::MIN, f64::MIN)),
            |(lo, hi), (_, q)| {
                (
                    p(lo.x.min(q.x), lo.y.min(q.y)),
                    p(hi.x.max(q.x), hi.y.max(q.y)),
                )
            },
        )
    }

    #[test]
    fn moving_turning_and_scaling_keep_the_shape_and_its_constraints() {
        use crate::SketchTransform as T;
        // Moved by (5, 7): the corners go to (5, 7) and (25, 17), still fully fixed.
        let mut s = Sketch::new();
        let lines = plate(&mut s);
        s.apply_tool(&ToolOp::Transform {
            curves: lines.clone(),
            points: vec![],
            transform: T::Translate { dx: 5.0, dy: 7.0 },
            copy: false,
        })
        .unwrap();
        let (lo, hi) = bounds(&s);
        assert!(lo.distance(p(5.0, 7.0)) < 1e-9 && hi.distance(p(25.0, 17.0)) < 1e-9);
        assert_eq!(s.analyze().unwrap().dof, 0);
        // Turned a quarter about the origin: (x, y) -> (-y, x), so the box is
        // x -10..0, y 0..20; horizontal and vertical swap, and it stays fixed.
        let mut s = Sketch::new();
        let lines = plate(&mut s);
        s.apply_tool(&ToolOp::Transform {
            curves: lines.clone(),
            points: vec![],
            transform: T::Rotate {
                centre: p(0.0, 0.0),
                angle: FRAC_PI_2,
            },
            copy: false,
        })
        .unwrap();
        let (lo, hi) = bounds(&s);
        assert!(lo.distance(p(-10.0, 0.0)) < 1e-9 && hi.distance(p(0.0, 20.0)) < 1e-9);
        assert_eq!(s.analyze().unwrap().dof, 0);
        // Turned 30 degrees, the horizontal and vertical constraints no longer hold and
        // are dropped: the area stays 200.
        let mut s = Sketch::new();
        let lines = plate(&mut s);
        s.apply_tool(&ToolOp::Transform {
            curves: lines.clone(),
            points: vec![],
            transform: T::Rotate {
                centre: p(0.0, 0.0),
                angle: PI / 6.0,
            },
            copy: false,
        })
        .unwrap();
        assert!((areas(&s)[0] - 200.0).abs() < 1e-9);
        assert!(
            !s.constraints()
                .any(|(_, c)| matches!(c, Constraint::Horizontal { .. }))
        );
        // Scaled by 2 about the origin: 40 x 20, area 800, the lengths dimensioned 40, 20.
        let mut s = Sketch::new();
        let lines = plate(&mut s);
        s.apply_tool(&ToolOp::Transform {
            curves: lines.clone(),
            points: vec![],
            transform: T::Scale {
                centre: p(0.0, 0.0),
                factor: 2.0,
            },
            copy: false,
        })
        .unwrap();
        assert!((areas(&s)[0] - 800.0).abs() < 1e-9);
        let mut lengths: Vec<f64> = s
            .constraints()
            .filter_map(|(_, c)| match c {
                Constraint::Length { value, .. } => Some(*value),
                _ => None,
            })
            .collect();
        lengths.sort_by(f64::total_cmp);
        assert_eq!(lengths, vec![20.0, 40.0]);
        // A copy moved by (30, 0) leaves the original: two plates of 200.
        let mut s = Sketch::new();
        let lines = plate(&mut s);
        s.apply_tool(&ToolOp::Transform {
            curves: lines,
            points: vec![],
            transform: T::Translate { dx: 30.0, dy: 0.0 },
            copy: true,
        })
        .unwrap();
        let a = areas(&s);
        assert_eq!(a.len(), 2);
        assert!(a.iter().all(|v| (v - 200.0).abs() < 1e-9), "{a:?}");
        let (lo, hi) = bounds(&s);
        assert!(lo.distance(p(0.0, 0.0)) < 1e-9 && hi.distance(p(50.0, 10.0)) < 1e-9);
        assert_eq!(s.analyze().unwrap().dof, 0);
        // A scale of 0 is refused.
        let mut s = Sketch::new();
        let lines = plate(&mut s);
        assert!(
            s.apply_tool(&ToolOp::Transform {
                curves: lines,
                points: vec![],
                transform: T::Scale {
                    centre: p(0.0, 0.0),
                    factor: 0.0,
                },
                copy: false,
            })
            .is_err()
        );
    }
}
