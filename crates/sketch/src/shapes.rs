//! Shapes built from lines, arcs and ellipses and tied by constraints, so they stay
//! parametric (each one is a [`crate::ToolOp`]); and curves fixed in place, for model
//! geometry projected into a sketch.
//!
//! # Polygon
//! A regular polygon of `n` sides about a centre point: every vertex lies on a
//! construction circle about that point (point on circle) and consecutive sides are equal
//! (`n` equal chords of a circle that close up subtend `2π/n` each, so the polygon is
//! regular). That leaves 4 degrees of freedom: the centre, the radius and the turn, so
//! fixing the centre, dimensioning the radius and levelling one side fixes the polygon.
//! Circumscribed, the circle the user sizes is the one inside: a second construction circle
//! about the centre that the first side touches (tangent); the other sides touch it too by
//! symmetry, so they are not constrained again (that would be redundant). Equal sides
//! tangent to one circle alone would not do: a rhombus has them too.
//!
//! # Slot
//! Two lines and two arcs about the end centres, each joint tangent, the arcs equal, and a
//! width dimension (from a point of the far side to the near side). Its degrees of freedom
//! are the two centres. A construction line joins the centres, for dimensioning.
//!
//! # Ellipse
//! An ellipse or elliptical arc with its centre, major and minor points (`entity.rs`).
//!
//! # Fixed curves
//! [`Sketch::add_fixed_curve`] adds a curve whose every point is fixed (and a circle's
//! radius dimensioned), so it has no degrees of freedom and other geometry can be
//! constrained to it; [`Sketch::move_fixed_curve`] moves it to new geometry by rewriting
//! those constraints. An arc's own equal-radius equation (and an elliptical arc's
//! on-ellipse equations) come last in the solver, so with all points fixed it is those
//! that count as dependent, not the fixes, and the curve reads as fully constrained.

use crate::constraint::Constraint;
use crate::ellipse::EllipseArc;
use crate::entity::{Curve, CurveId, PointId};
use crate::sketch::{Sketch, SketchError};
use crate::tools::{ToolError, check_length, check_point, refuse};
use chamfera_geom::{LINEAR_TOLERANCE, Point2, Vec2};
use std::f64::consts::{PI, TAU};

/// Fewest sides a polygon may have.
pub const MIN_POLYGON_SIDES: u32 = 3;
/// Most sides a polygon may have.
pub const MAX_POLYGON_SIDES: u32 = 64;

/// Exact geometry for a curve fixed in place. Arcs run counter-clockwise from `s` to `e`;
/// an ellipse's `minor` point must be square to its major axis, and an elliptical arc's
/// ends on its ellipse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FixedCurve {
    Line {
        a: Point2,
        b: Point2,
    },
    Circle {
        c: Point2,
        r: f64,
    },
    Arc {
        c: Point2,
        s: Point2,
        e: Point2,
    },
    Ellipse {
        c: Point2,
        major: Point2,
        minor: Point2,
    },
    EllipticalArc {
        c: Point2,
        major: Point2,
        minor: Point2,
        s: Point2,
        e: Point2,
    },
}

impl FixedCurve {
    /// The positions of the curve's points, in the order of [`Curve::points`].
    fn positions(&self) -> Vec<Point2> {
        match *self {
            FixedCurve::Line { a, b } => vec![a, b],
            FixedCurve::Circle { c, .. } => vec![c],
            FixedCurve::Arc { c, s, e } => vec![c, s, e],
            FixedCurve::Ellipse { c, major, minor } => vec![c, major, minor],
            FixedCurve::EllipticalArc {
                c,
                major,
                minor,
                s,
                e,
            } => vec![c, major, minor, s, e],
        }
    }

    fn same_kind(&self, curve: &Curve) -> bool {
        matches!(
            (self, curve),
            (FixedCurve::Line { .. }, Curve::Line { .. })
                | (FixedCurve::Circle { .. }, Curve::Circle { .. })
                | (FixedCurve::Arc { .. }, Curve::Arc { .. })
                | (FixedCurve::Ellipse { .. }, Curve::Ellipse { .. })
                | (
                    FixedCurve::EllipticalArc { .. },
                    Curve::EllipticalArc { .. }
                )
        )
    }
}

fn dir(angle: f64) -> Vec2 {
    Vec2::new(angle.cos(), angle.sin())
}

impl Sketch {
    /// Adds a curve fixed in place: a fix on each of its points, and a radius dimension
    /// for a circle. Nothing is added if any part is invalid.
    pub fn add_fixed_curve(&mut self, g: &FixedCurve) -> Result<CurveId, SketchError> {
        let backup = self.clone();
        let result = self.add_fixed_unchecked(g);
        if result.is_err() {
            *self = backup;
        }
        result
    }

    fn add_fixed_unchecked(&mut self, g: &FixedCurve) -> Result<CurveId, SketchError> {
        let mut ids = Vec::new();
        for p in g.positions() {
            ids.push(self.add_point(p)?);
        }
        let at = |k: usize| ids.get(k).copied().ok_or(SketchError::IdsExhausted);
        let id = match *g {
            FixedCurve::Line { .. } => self.add_line(at(0)?, at(1)?)?,
            FixedCurve::Circle { r, .. } => {
                let id = self.add_circle(at(0)?, r)?;
                self.add_constraint(Constraint::Radius {
                    curve: id,
                    value: r,
                })?;
                id
            }
            FixedCurve::Arc { .. } => self.add_arc(at(0)?, at(1)?, at(2)?)?,
            FixedCurve::Ellipse { .. } => self.add_ellipse(at(0)?, at(1)?, at(2)?)?,
            FixedCurve::EllipticalArc { .. } => {
                self.add_elliptical_arc(at(0)?, at(1)?, at(2)?, at(3)?, at(4)?)?
            }
        };
        for p in &ids {
            self.fix_point(*p)?;
        }
        Ok(id)
    }

    /// Moves a curve added by [`Sketch::add_fixed_curve`] to new geometry of the same kind:
    /// its points move and their fixes (and a circle's radius dimension) follow. Does not
    /// solve. Refused, changing nothing, for a curve of another kind.
    pub fn move_fixed_curve(&mut self, id: CurveId, g: &FixedCurve) -> Result<(), SketchError> {
        let curve = self
            .curve(id)
            .cloned()
            .ok_or(SketchError::UnknownCurve(id))?;
        if !g.same_kind(&curve) {
            return Err(SketchError::WrongCurveKind {
                curve: id,
                found: curve.kind_name(),
                expected: match g {
                    FixedCurve::Line { .. } => "line",
                    FixedCurve::Circle { .. } => "circle",
                    FixedCurve::Arc { .. } => "arc",
                    FixedCurve::Ellipse { .. } => "ellipse",
                    FixedCurve::EllipticalArc { .. } => "elliptical arc",
                },
            });
        }
        let backup = self.clone();
        let moves: Vec<(PointId, Point2)> = curve.points().into_iter().zip(g.positions()).collect();
        for (p, at) in &moves {
            if !at.is_valid() {
                *self = backup;
                return Err(SketchError::BadCoordinate {
                    what: "a projected point",
                    value: if at.x.is_finite() { at.y } else { at.x },
                });
            }
            self.set_point_unchecked(*p, *at);
        }
        let radius = match g {
            FixedCurve::Circle { r, .. } => Some(*r),
            _ => None,
        };
        if let Some(r) = radius {
            self.set_radius_unchecked(id, r);
        }
        self.rewrite_constraints(|c| match c {
            Constraint::Fix { point, .. } => moves
                .iter()
                .find(|(p, _)| p == point)
                .map(|(p, at)| Constraint::Fix { point: *p, at: *at }),
            Constraint::Radius { curve, .. } if *curve == id => {
                radius.map(|value| Constraint::Radius { curve: id, value })
            }
            _ => None,
        });
        if let Err(e) = self.validate() {
            *self = backup;
            return Err(e);
        }
        Ok(())
    }

    // ----- shapes (tools) -----

    pub(crate) fn polygon(
        &mut self,
        centre: Point2,
        corner: Point2,
        sides: u32,
        inscribed: bool,
    ) -> Result<(), ToolError> {
        check_point("the centre", centre)?;
        check_point("the corner", corner)?;
        if !(MIN_POLYGON_SIDES..=MAX_POLYGON_SIDES).contains(&sides) {
            return refuse(format!(
                "a polygon has {MIN_POLYGON_SIDES} to {MAX_POLYGON_SIDES} sides, got {sides}"
            ));
        }
        let r = centre.distance(corner);
        check_length("the polygon's radius", r)?;
        let n = f64::from(sides);
        let phi = (corner - centre).angle();
        // Inscribed: a vertex at the corner. Circumscribed: the middle of side 0 at the
        // corner, so its vertices are half a step either side, at r / cos(π/n).
        let (first, reach) = if inscribed {
            (phi, r)
        } else {
            (phi - PI / n, r / (PI / n).cos())
        };
        let mut pts = Vec::new();
        for k in 0..sides {
            let at = centre + dir(first + TAU * f64::from(k) / n) * reach;
            check_point("a polygon vertex", at)?;
            pts.push(self.add_point(at)?);
        }
        let mid = self.add_point(centre)?;
        let circle = self.add_circle(mid, reach)?;
        self.set_construction(circle, true)?;
        let mut lines = Vec::new();
        for k in 0..pts.len() {
            let (Some(a), Some(b)) = (pts.get(k), pts.get((k + 1) % pts.len())) else {
                return refuse("the polygon lost a vertex");
            };
            lines.push(self.add_line(*a, *b)?);
        }
        for (k, line) in lines.iter().enumerate() {
            self.add_constraint(Constraint::PointOnCircle {
                point: pts.get(k).copied().ok_or(SketchError::IdsExhausted)?,
                curve: circle,
            })?;
            if let Some(next) = lines.get(k + 1) {
                self.add_constraint(Constraint::Equal { a: *line, b: *next })?;
            }
        }
        if !inscribed {
            let inner = self.add_circle(mid, r)?;
            self.set_construction(inner, true)?;
            let first = lines.first().copied().ok_or(SketchError::IdsExhausted)?;
            self.add_constraint(Constraint::Tangent { a: first, b: inner })?;
        }
        Ok(())
    }

    pub(crate) fn slot(
        &mut self,
        start: Point2,
        end: Point2,
        width: f64,
        overall: bool,
    ) -> Result<(), ToolError> {
        check_point("the start", start)?;
        check_point("the end", end)?;
        check_length("the width", width)?;
        let r = width / 2.0;
        let Some(d) = (end - start).normalized() else {
            return refuse("a slot needs two different points");
        };
        // Overall length runs tip to tip: the centres are a radius in from each tip.
        let (c1, c2) = if overall {
            (start + d * r, end - d * r)
        } else {
            (start, end)
        };
        if (c2 - c1).dot(d) <= LINEAR_TOLERANCE {
            return refuse("the slot is shorter than its width: draw a circle");
        }
        let n = d.perp();
        let corners = [c1 - n * r, c2 - n * r, c2 + n * r, c1 + n * r];
        for p in corners {
            check_point("a slot corner", p)?;
        }
        let a = self.add_point(c1)?;
        let b = self.add_point(c2)?;
        let mut q = Vec::new();
        for p in corners {
            q.push(self.add_point(p)?);
        }
        let [q1, q2, p2, p1] = q.as_slice() else {
            return refuse("the slot lost a corner");
        };
        let (q1, q2, p2, p1) = (*q1, *q2, *p2, *p1);
        let axis = self.add_line(a, b)?;
        self.set_construction(axis, true)?;
        let near = self.add_line(q1, q2)?;
        let end_arc = self.add_arc(b, q2, p2)?;
        let far = self.add_line(p2, p1)?;
        let start_arc = self.add_arc(a, p1, q1)?;
        for (x, y) in [
            (near, end_arc),
            (end_arc, far),
            (far, start_arc),
            (start_arc, near),
        ] {
            self.add_constraint(Constraint::Tangent { a: x, b: y })?;
        }
        self.add_constraint(Constraint::Equal {
            a: start_arc,
            b: end_arc,
        })?;
        self.add_constraint(Constraint::PointLineDistance {
            point: p2,
            line: near,
            value: width,
        })?;
        Ok(())
    }

    pub(crate) fn ellipse(
        &mut self,
        centre: Point2,
        major: Point2,
        minor: f64,
        arc: Option<(Point2, Point2)>,
    ) -> Result<(), ToolError> {
        check_point("the centre", centre)?;
        check_point("the major axis end", major)?;
        check_length("the minor semi-axis", minor)?;
        check_length("the major semi-axis", centre.distance(major))?;
        let Some(u) = (major - centre).normalized() else {
            return refuse("the major axis has no length");
        };
        let n_at = centre + u.perp() * minor;
        check_point("the minor axis end", n_at)?;
        let Some(shape) = EllipseArc::full(centre, major, n_at) else {
            return refuse("the ellipse has no size");
        };
        let ends = match arc {
            None => None,
            Some((s, e)) => {
                let (s, e) = (
                    shape.point_at(shape.param_of(s)),
                    shape.point_at(shape.param_of(e)),
                );
                if s.distance(e) <= LINEAR_TOLERANCE {
                    return refuse("an elliptical arc needs different start and end directions");
                }
                Some((s, e))
            }
        };
        let c = self.add_point(centre)?;
        let m = self.add_point(major)?;
        let n = self.add_point(n_at)?;
        match ends {
            None => {
                self.add_ellipse(c, m, n)?;
            }
            Some((s, e)) => {
                let s = self.add_point(s)?;
                let e = self.add_point(e)?;
                self.add_elliptical_arc(c, m, n, s, e)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConstraintStatus, ToolOp};
    use std::f64::consts::PI;

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    fn only_area(s: &Sketch) -> f64 {
        let regions = s.profiles();
        assert_eq!(regions.len(), 1, "{regions:?}");
        regions[0].area
    }

    fn line_ids(s: &Sketch) -> Vec<CurveId> {
        s.curves()
            .filter(|(_, c)| c.is_line() && !c.is_construction())
            .map(|(id, _)| id)
            .collect()
    }

    #[test]
    fn an_inscribed_hexagon_has_its_area_and_is_fixed_by_centre_radius_and_turn() {
        let mut s = Sketch::new();
        s.apply_tool(&ToolOp::Polygon {
            centre: p(0.0, 0.0),
            corner: p(10.0, 0.0),
            sides: 6,
            inscribed: true,
        })
        .unwrap();
        // Six equilateral triangles of side 10: 6 · (√3/4) · 100 = (3√3/2) · 100.
        let expected = 1.5 * 3f64.sqrt() * 100.0;
        assert!((only_area(&s) - expected).abs() < 1e-9);
        // Centre, radius and turn are free: 4 degrees of freedom.
        assert_eq!(s.analyze().unwrap().dof, 4);
        let (centre, circle) = s
            .curves()
            .find_map(|(id, c)| match c {
                Curve::Circle { centre, .. } => Some((*centre, id)),
                _ => None,
            })
            .unwrap();
        s.fix_point(centre).unwrap();
        s.try_add_constraint(Constraint::Radius {
            curve: circle,
            value: 10.0,
        })
        .unwrap();
        // Vertices at 0°, 60°, ...: the side from 60° to 120° is level.
        let level = line_ids(&s)
            .into_iter()
            .find(|l| {
                let Some(Curve::Line { start, end, .. }) = s.curve(*l) else {
                    return false;
                };
                (s.point(*start).unwrap().y - s.point(*end).unwrap().y).abs() < 1e-9
                    && s.point(*start).unwrap().y > 0.0
            })
            .unwrap();
        let report = s
            .try_add_constraint(Constraint::Horizontal { line: level })
            .unwrap();
        let _ = report;
        let dof = s.analyze().unwrap();
        assert_eq!(dof.dof, 0);
        assert_eq!(dof.status, ConstraintStatus::FullyConstrained);
        // Changing the radius keeps it a regular hexagon: area scales with r².
        let r = s
            .constraints()
            .find_map(|(id, c)| matches!(c, Constraint::Radius { .. }).then_some(id))
            .unwrap();
        s.try_set_dimension(r, 20.0).unwrap();
        assert!((only_area(&s) - 4.0 * expected).abs() < 1e-6);
    }

    #[test]
    fn a_circumscribed_square_touches_its_circle_at_the_corner_click() {
        let mut s = Sketch::new();
        s.apply_tool(&ToolOp::Polygon {
            centre: p(0.0, 0.0),
            corner: p(5.0, 0.0),
            sides: 4,
            inscribed: false,
        })
        .unwrap();
        // About a circle of radius 5: a 10 x 10 square with a side through (5, 0).
        assert!((only_area(&s) - 100.0).abs() < 1e-9);
        assert!(s.points().any(|(_, q)| q.distance(p(5.0, 5.0)) < 1e-9));
        assert_eq!(s.analyze().unwrap().dof, 4);
    }

    #[test]
    fn polygons_refuse_bad_side_counts_and_sizes() {
        let mut s = Sketch::new();
        for sides in [0, 2, 65, u32::MAX] {
            assert!(
                s.apply_tool(&ToolOp::Polygon {
                    centre: p(0.0, 0.0),
                    corner: p(5.0, 0.0),
                    sides,
                    inscribed: true,
                })
                .is_err()
            );
        }
        assert!(
            s.apply_tool(&ToolOp::Polygon {
                centre: p(1.0, 1.0),
                corner: p(1.0, 1.0),
                sides: 5,
                inscribed: true,
            })
            .is_err()
        );
        assert!(s.is_empty());
        // 64 sides work, and approach the circle: n/2 · r² · sin(2π/n).
        s.apply_tool(&ToolOp::Polygon {
            centre: p(0.0, 0.0),
            corner: p(0.0, 10.0),
            sides: 64,
            inscribed: true,
        })
        .unwrap();
        let expected = 32.0 * 100.0 * (TAU / 64.0).sin();
        assert!((only_area(&s) - expected).abs() < 1e-6);
    }

    #[test]
    fn a_slot_is_a_rectangle_and_a_circle_of_its_width() {
        let mut s = Sketch::new();
        s.apply_tool(&ToolOp::Slot {
            start: p(0.0, 0.0),
            end: p(20.0, 0.0),
            width: 6.0,
            overall: false,
        })
        .unwrap();
        // 20 x 6 between the centres, plus two half discs of radius 3: 120 + 9π.
        let expected = 20.0 * 6.0 + 9.0 * PI;
        assert!((only_area(&s) - expected).abs() < 1e-9);
        // Only the two centres are free.
        assert_eq!(s.analyze().unwrap().dof, 4);
        // The width dimension drives it: width 10 gives 200 + 25π.
        let w = s
            .constraints()
            .find_map(|(id, c)| matches!(c, Constraint::PointLineDistance { .. }).then_some(id))
            .unwrap();
        s.try_set_dimension(w, 10.0).unwrap();
        // The solver stops within its tolerance (1e-7 mm per equation), which moves the
        // area by up to about the perimeter times that.
        assert!((only_area(&s) - (200.0 + 25.0 * PI)).abs() < 1e-4);
    }

    #[test]
    fn an_overall_slot_measures_tip_to_tip() {
        let mut s = Sketch::new();
        // Tips 26 apart, width 6: centres 20 apart, at (3, 5) and (3, 25) going up.
        s.apply_tool(&ToolOp::Slot {
            start: p(3.0, 2.0),
            end: p(3.0, 28.0),
            width: 6.0,
            overall: true,
        })
        .unwrap();
        assert!((only_area(&s) - (120.0 + 9.0 * PI)).abs() < 1e-9);
        assert!(s.points().any(|(_, q)| q.distance(p(3.0, 25.0)) < 1e-9));
        let mut t = Sketch::new();
        assert!(
            t.apply_tool(&ToolOp::Slot {
                start: p(0.0, 0.0),
                end: p(5.0, 0.0),
                width: 6.0,
                overall: true,
            })
            .is_err()
        );
        assert!(
            t.apply_tool(&ToolOp::Slot {
                start: p(0.0, 0.0),
                end: p(5.0, 0.0),
                width: f64::NAN,
                overall: false,
            })
            .is_err()
        );
        assert!(t.is_empty());
    }

    #[test]
    fn an_ellipse_has_area_pi_a_b_and_five_degrees_of_freedom() {
        let mut s = Sketch::new();
        // Axes at 30° so nothing lines up with x and y.
        let u = dir(PI / 6.0);
        s.apply_tool(&ToolOp::Ellipse {
            centre: p(4.0, -2.0),
            major: p(4.0, -2.0) + u * 10.0,
            minor: 5.0,
        })
        .unwrap();
        assert!((only_area(&s) - 50.0 * PI).abs() < 1e-9);
        assert_eq!(s.analyze().unwrap().dof, 5);
        // A point inside is in the region, one just outside the minor vertex is not.
        let inside = p(4.0, -2.0) + u.perp() * 4.9;
        let outside = p(4.0, -2.0) + u.perp() * 5.1;
        assert!(s.region_at(inside).is_some());
        assert!(s.region_at(outside).is_none());
    }

    #[test]
    fn an_elliptical_arc_closed_by_its_axis_is_half_an_ellipse() {
        let mut s = Sketch::new();
        s.apply_tool(&ToolOp::EllipticalArc {
            centre: p(0.0, 0.0),
            major: p(10.0, 0.0),
            minor: 5.0,
            start: p(10.0, 0.0),
            end: p(-3.0, 0.0),
        })
        .unwrap();
        let (start, end) = s
            .curves()
            .find_map(|(_, c)| match c {
                Curve::EllipticalArc { start, end, .. } => Some((*start, *end)),
                _ => None,
            })
            .unwrap();
        // The end lands on the ellipse at t = π: (-10, 0).
        assert!(s.point(end).unwrap().distance(p(-10.0, 0.0)) < 1e-9);
        s.add_line(end, start).unwrap();
        // The upper half: π · 10 · 5 / 2.
        assert!((only_area(&s) - 25.0 * PI).abs() < 1e-9);
        // Ellipse 5 + its ends' two parameters.
        assert_eq!(s.analyze().unwrap().dof, 7);
        // The solver keeps the ends on the ellipse when the major point moves.
        let major = s
            .curves()
            .find_map(|(_, c)| match c {
                Curve::EllipticalArc { major, .. } => Some(*major),
                _ => None,
            })
            .unwrap();
        s.drag(major, p(12.0, 0.0)).unwrap();
        let e = s
            .curves()
            .find_map(|(id, c)| c.is_elliptical().then_some(id))
            .and_then(|id| s.ellipse_arc(id))
            .unwrap();
        for q in [start, end] {
            let q = s.point(q).unwrap();
            assert!(e.point_at(e.param_of(q)).distance(q) < 1e-6);
        }
    }

    #[test]
    fn fixed_curves_have_no_freedom_and_move_with_new_geometry() {
        let mut s = Sketch::new();
        let line = s
            .add_fixed_curve(&FixedCurve::Line {
                a: p(0.0, 0.0),
                b: p(50.0, 0.0),
            })
            .unwrap();
        let circle = s
            .add_fixed_curve(&FixedCurve::Circle {
                c: p(25.0, 15.0),
                r: 4.0,
            })
            .unwrap();
        s.add_fixed_curve(&FixedCurve::Arc {
            c: p(0.0, 30.0),
            s: p(5.0, 30.0),
            e: p(0.0, 35.0),
        })
        .unwrap();
        s.add_fixed_curve(&FixedCurve::EllipticalArc {
            c: p(0.0, 60.0),
            major: p(10.0, 60.0),
            minor: p(0.0, 65.0),
            s: p(10.0, 60.0),
            e: p(-10.0, 60.0),
        })
        .unwrap();
        s.solve().unwrap();
        let dof = s.analyze().unwrap();
        assert_eq!(dof.dof, 0);
        assert_eq!(dof.status, ConstraintStatus::FullyConstrained);
        s.move_fixed_curve(
            circle,
            &FixedCurve::Circle {
                c: p(30.0, 15.0),
                r: 6.0,
            },
        )
        .unwrap();
        s.move_fixed_curve(
            line,
            &FixedCurve::Line {
                a: p(0.0, 0.0),
                b: p(60.0, 0.0),
            },
        )
        .unwrap();
        s.solve().unwrap();
        assert_eq!(s.radius(circle), Some(6.0));
        let Some(Curve::Line { end, .. }) = s.curve(line) else {
            panic!("the line is gone");
        };
        assert_eq!(s.point(*end), Some(p(60.0, 0.0)));
        // Another kind is refused and changes nothing.
        let before = s.clone();
        assert!(
            s.move_fixed_curve(
                line,
                &FixedCurve::Circle {
                    c: p(0.0, 0.0),
                    r: 1.0
                }
            )
            .is_err()
        );
        assert!(
            s.move_fixed_curve(
                line,
                &FixedCurve::Line {
                    a: p(f64::NAN, 0.0),
                    b: p(1.0, 0.0)
                }
            )
            .is_err()
        );
        assert_eq!(s, before);
    }
}
