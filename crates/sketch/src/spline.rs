//! Splines in a sketch: smooth curves defined by ordinary sketch points.
//!
//! A [`Curve::Spline`] lists its points in order. With [`SplineForm::Fit`] (the default)
//! the curve passes through every point: global cubic interpolation with chord-length
//! parameters (The NURBS Book, §9.2; `chamfera_geom::BSplineCurve2::interpolate`). With
//! [`SplineForm::Control`] the points are the control polygon of a cubic B-spline with
//! clamped uniform knots, so the curve starts at the first point and ends at the last.
//! A closed spline closes on itself (a closed fit spline through all points with a
//! continuous tangent at the first; a closed control spline with periodic knots).
//!
//! The spline adds no unknowns and no equations to the solver: its geometry is a
//! function of its points, so constraints and dimensions on the points (fix, coincident,
//! horizontal distance, ...) shape it, and dragging a point reshapes it. An open
//! spline's first and last points are its end points, which join lines and arcs in
//! profiles; a closed spline is a loop of its own, like a circle.
//!
//! When the points do not make a curve (two consecutive points on top of each other while
//! dragging), the spline has no geometry for the moment: it is not drawn and bounds no
//! profile, and comes back as soon as the points separate.

use crate::entity::{Curve, CurveId, PointId};
use crate::sketch::{Sketch, SketchError};
use chamfera_geom::{BSplineCurve2, MAX_FIT_POINTS, Point2};
use serde::{Deserialize, Serialize};

/// Most points one spline may have.
pub const MAX_SPLINE_POINTS: usize = MAX_FIT_POINTS - 1;

/// How a spline uses its points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplineForm {
    /// The curve passes through the points.
    #[default]
    Fit,
    /// The points are the curve's control polygon.
    Control,
}

impl SplineForm {
    pub fn name(self) -> &'static str {
        match self {
            SplineForm::Fit => "fit",
            SplineForm::Control => "control",
        }
    }
}

/// Checks a spline's point list: enough points (2 open, 3 closed), at most
/// [`MAX_SPLINE_POINTS`], and no point twice in a row (nor the last equal to the first
/// of a closed spline).
pub(crate) fn check_points(points: &[PointId], closed: bool) -> Result<(), SketchError> {
    let least = if closed { 3 } else { 2 };
    if points.len() < least {
        return Err(SketchError::BadSpline(format!(
            "a{} spline needs at least {least} points",
            if closed { " closed" } else { "n open" }
        )));
    }
    if points.len() > MAX_SPLINE_POINTS {
        return Err(SketchError::BadSpline(format!(
            "a spline may have at most {MAX_SPLINE_POINTS} points"
        )));
    }
    if points.windows(2).any(|w| w[0] == w[1]) || (closed && points.first() == points.last()) {
        return Err(SketchError::BadSpline(
            "a spline may not use the same point twice in a row".into(),
        ));
    }
    Ok(())
}

/// The curve of a spline through (or on) `at`, or `None` if they do not make one.
pub fn spline_geometry(at: &[Point2], form: SplineForm, closed: bool) -> Option<BSplineCurve2> {
    match (form, closed) {
        (SplineForm::Fit, false) => BSplineCurve2::interpolate(at, None, None).ok(),
        (SplineForm::Fit, true) => BSplineCurve2::interpolate_closed(at).ok(),
        (SplineForm::Control, c) => BSplineCurve2::from_control_points(at, c).ok(),
    }
}

impl Sketch {
    /// Adds a spline on existing points (see the module documentation).
    pub fn add_spline(
        &mut self,
        points: Vec<PointId>,
        form: SplineForm,
        closed: bool,
    ) -> Result<CurveId, SketchError> {
        self.add_curve(Curve::Spline {
            points,
            form,
            closed,
            construction: false,
        })
    }

    /// The exact curve of a spline as its points are now, or `None` for another curve or
    /// points that do not make a curve.
    pub fn spline_curve(&self, id: CurveId) -> Option<BSplineCurve2> {
        self.curve(id).and_then(|c| self.spline_of(c))
    }

    /// The exact curve of `curve` if it is a spline.
    pub fn spline_of(&self, curve: &Curve) -> Option<BSplineCurve2> {
        let Curve::Spline {
            points,
            form,
            closed,
            ..
        } = curve
        else {
            return None;
        };
        let at: Option<Vec<Point2>> = points.iter().map(|p| self.point(*p)).collect();
        spline_geometry(&at?, *form, *closed)
    }
}

/// Points per knot span of a spline's display and containment polyline.
pub const SPLINE_SAMPLES_PER_SPAN: usize = 24;

/// The spline as a polyline from its start to its end: [`SPLINE_SAMPLES_PER_SPAN`] points
/// evenly spaced in the parameter on each knot span (display and the polygon tests that
/// already approximate arcs the same way).
pub fn spline_polyline(curve: &BSplineCurve2) -> Vec<Point2> {
    curve.polyline(SPLINE_SAMPLES_PER_SPAN)
}

/// How many times the ray from `p` toward +x crosses the spline (see
/// `BSplineCurve2::ray_crossings`).
pub(crate) fn ray_crossings(c: &BSplineCurve2, a: Point2, b: Point2, p: Point2) -> usize {
    c.ray_crossings(a, b, p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn pt(s: &mut Sketch, x: f64, y: f64) -> PointId {
        s.add_point(Point2::new(x, y)).unwrap()
    }

    #[test]
    fn a_fit_spline_passes_through_its_points_and_follows_them() {
        let mut s = Sketch::new();
        let ids: Vec<PointId> = [(0.0, 0.0), (10.0, 8.0), (20.0, -2.0), (30.0, 5.0)]
            .iter()
            .map(|(x, y)| pt(&mut s, *x, *y))
            .collect();
        let c = s.add_spline(ids.clone(), SplineForm::Fit, false).unwrap();
        let curve = s.spline_curve(c).unwrap();
        for id in &ids {
            let q = s.point(*id).unwrap();
            assert!(curve.closest_point(q).distance < 1e-9);
        }
        // Moving a point reshapes the spline: it now passes through the new position.
        s.move_point(ids[1], Point2::new(12.0, 15.0)).unwrap();
        let moved = s.spline_curve(c).unwrap();
        assert!(moved.closest_point(Point2::new(12.0, 15.0)).distance < 1e-9);
        // Its points carry the degrees of freedom: 4 points, 8 DOF; fixing two leaves 4.
        assert_eq!(s.analyze().unwrap().dof, 8);
        s.fix_point(ids[0]).unwrap();
        s.fix_point(ids[3]).unwrap();
        assert_eq!(s.analyze().unwrap().dof, 4);
    }

    #[test]
    fn spline_point_lists_are_checked() {
        let mut s = Sketch::new();
        let a = pt(&mut s, 0.0, 0.0);
        let b = pt(&mut s, 1.0, 0.0);
        let c = pt(&mut s, 1.0, 1.0);
        assert!(s.add_spline(vec![a], SplineForm::Fit, false).is_err());
        assert!(s.add_spline(vec![a, b], SplineForm::Fit, true).is_err());
        assert!(s.add_spline(vec![a, a, b], SplineForm::Fit, false).is_err());
        assert!(
            s.add_spline(vec![a, b, c, a], SplineForm::Fit, true)
                .is_err()
        );
        assert!(
            s.add_spline(vec![a, PointId(999)], SplineForm::Fit, false)
                .is_err()
        );
        let many = [a, b].repeat(MAX_SPLINE_POINTS);
        assert!(s.add_spline(many, SplineForm::Control, false).is_err());
        assert!(
            s.add_spline(vec![a, b, c], SplineForm::Control, true)
                .is_ok()
        );
    }

    #[test]
    fn a_closed_spline_bounds_a_region_with_the_area_of_its_curve() {
        // The closed uniform cubic on the square (±10, ±10). Each span is
        // C(t) = Σ B_i(t) P_{k+i} with the uniform cubic basis B_0 = (1-t)³/6,
        // B_1 = (3t³ - 6t² + 4)/6, B_2 = (-3t³ + 3t² + 3t + 1)/6, B_3 = t³/6; integrating
        // ½ (x y' - y x') over the four spans in closed form gives 2440/9 mm².
        let mut s = Sketch::new();
        let ids: Vec<PointId> = [(10.0, -10.0), (10.0, 10.0), (-10.0, 10.0), (-10.0, -10.0)]
            .iter()
            .map(|(x, y)| pt(&mut s, *x, *y))
            .collect();
        let c = s.add_spline(ids, SplineForm::Control, true).unwrap();
        let regions = s.profiles();
        assert_eq!(regions.len(), 1);
        assert!(
            (regions[0].area - 2440.0 / 9.0).abs() < 1e-9,
            "{}",
            regions[0].area
        );
        assert_eq!(regions[0].outer.segments[0].curve, c);
    }

    #[test]
    fn an_open_spline_closed_by_a_line_is_a_region() {
        // A fit spline from (0, 0) over (10, 10) to (20, 0), closed by the line back: the
        // three points make a quadratic through them, the parabola y = x (20 - x) / 10,
        // whose area over the chord is 2/3 · 20 · 10 = 400/3 mm² (Archimedes).
        let mut s = Sketch::new();
        let a = pt(&mut s, 0.0, 0.0);
        let m = pt(&mut s, 10.0, 10.0);
        let b = pt(&mut s, 20.0, 0.0);
        s.add_spline(vec![a, m, b], SplineForm::Fit, false).unwrap();
        s.add_line(b, a).unwrap();
        let regions = s.profiles();
        assert_eq!(regions.len(), 1);
        assert!(
            (regions[0].area - 400.0 / 3.0).abs() < 1e-9,
            "{}",
            regions[0].area
        );
        // A point under the arch is in the region, one above it is not.
        assert!(s.region_at(Point2::new(10.0, 9.0)).is_some());
        assert!(s.region_at(Point2::new(10.0, 11.0)).is_none());
        let _ = PI;
    }
}
