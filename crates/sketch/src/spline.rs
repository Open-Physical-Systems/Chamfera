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
            start_handle: None,
            end_handle: None,
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
            start_handle,
            end_handle,
            ..
        } = curve
        else {
            return None;
        };
        let at: Option<Vec<Point2>> = points.iter().map(|p| self.point(*p)).collect();
        let at = at?;
        if *form == SplineForm::Fit && !*closed && (start_handle.is_some() || end_handle.is_some())
        {
            // A handle sets the end tangent: from the first point toward its handle, and
            // at the last point away from its handle. A handle on its end sets none.
            let toward = |end: Option<&PointId>, handle: &Option<PointId>, out: bool| {
                let e = self.point(*end?)?;
                let h = self.point((*handle)?)?;
                let v = if out { h - e } else { e - h };
                (v.length() > chamfera_geom::LINEAR_TOLERANCE).then_some(v)
            };
            let t0 = toward(points.first(), start_handle, true);
            let t1 = toward(points.last(), end_handle, false);
            return BSplineCurve2::interpolate(&at, t0, t1).ok();
        }
        spline_geometry(&at, *form, *closed)
    }

    /// Gives an open fit spline tangent handles at its ends (`start`, `end`), each a new
    /// point a third of the way along the first (last) chord from its end, in the
    /// direction the spline now leaves (arrives); returns the handle points. An end that
    /// already has a handle keeps it.
    pub fn add_spline_handles(
        &mut self,
        id: CurveId,
        start: bool,
        end: bool,
    ) -> Result<(Option<PointId>, Option<PointId>), SketchError> {
        let curve = self
            .curve(id)
            .cloned()
            .ok_or(SketchError::UnknownCurve(id))?;
        let Curve::Spline {
            points,
            form: SplineForm::Fit,
            closed: false,
            start_handle,
            end_handle,
            ..
        } = curve.clone()
        else {
            return Err(SketchError::BadSpline(
                "only an open fit spline has tangent handles (a control spline's tangents \
                 follow its control polygon)"
                    .into(),
            ));
        };
        let geometry = self
            .spline_of(&curve)
            .ok_or_else(|| SketchError::BadSpline("the spline has no shape now".into()))?;
        let (lo, hi) = geometry.domain();
        let chord = |i: usize, j: usize| match (
            points.get(i).and_then(|p| self.point(*p)),
            points.get(j).and_then(|p| self.point(*p)),
        ) {
            (Some(a), Some(b)) => a.distance(b),
            _ => 0.0,
        };
        let n = points.len();
        let place = |at_end: bool| -> Result<Point2, SketchError> {
            let (u, p, len) = if at_end {
                (hi, points.last(), chord(n.saturating_sub(2), n - 1))
            } else {
                (lo, points.first(), chord(0, 1))
            };
            let e = p
                .and_then(|p| self.point(*p))
                .ok_or_else(|| SketchError::BadSpline("the spline has no end".into()))?;
            let d = geometry
                .derivatives(u, 1)
                .get(1)
                .copied()
                .and_then(|v| v.normalized())
                .ok_or_else(|| SketchError::BadSpline("the spline has no tangent there".into()))?;
            let reach = (len / 3.0).max(chamfera_geom::LINEAR_TOLERANCE * 10.0);
            Ok(if at_end { e - d * reach } else { e + d * reach })
        };
        let want_start = match (start, start_handle) {
            (true, None) => Some(place(false)?),
            _ => None,
        };
        let want_end = match (end, end_handle) {
            (true, None) => Some(place(true)?),
            _ => None,
        };
        let s = match want_start {
            Some(at) => Some(self.add_point(at)?),
            None => start_handle,
        };
        let e = match want_end {
            Some(at) => Some(self.add_point(at)?),
            None => end_handle,
        };
        let mut next = curve;
        if let Curve::Spline {
            start_handle,
            end_handle,
            ..
        } = &mut next
        {
            *start_handle = s;
            *end_handle = e;
        }
        self.replace_curve(id, next)?;
        Ok((s, e))
    }

    /// Takes a spline's tangent handles away (the points stay, free).
    pub fn remove_spline_handles(&mut self, id: CurveId) -> Result<(), SketchError> {
        let mut curve = self
            .curve(id)
            .cloned()
            .ok_or(SketchError::UnknownCurve(id))?;
        let Curve::Spline {
            start_handle,
            end_handle,
            ..
        } = &mut curve
        else {
            return Err(SketchError::BadSpline("not a spline".into()));
        };
        *start_handle = None;
        *end_handle = None;
        self.replace_curve(id, curve)
    }

    /// Where spline `spline` meets curve `other` end to end, for a tangent constraint:
    /// the spline's end point there, the point that sets the spline's tangent there (its
    /// handle, or for a control spline the next control point in), whether it is the
    /// spline's first point, and the other curve's end that meets it. Ends meet when
    /// they are one point or tied by a coincident constraint.
    pub(crate) fn spline_join(&self, spline: CurveId, other: CurveId) -> Option<SplineJoin> {
        let Some(Curve::Spline {
            points,
            form,
            closed: false,
            start_handle,
            end_handle,
            ..
        }) = self.curve(spline)
        else {
            return None;
        };
        let (oa, ob) = self.curve(other)?.ends()?;
        let joined = |p: PointId, q: PointId| {
            p == q
                || self.constraints().any(|(_, c)| {
                    matches!(c, crate::Constraint::Coincident { a, b }
                        if (*a == p && *b == q) || (*a == q && *b == p))
                })
        };
        let n = points.len();
        let ends = [(true, points.first()), (false, points.last())];
        for (first, end) in ends {
            let Some(&e) = end else { continue };
            let Some(meet) = [oa, ob].into_iter().find(|q| joined(e, *q)) else {
                continue;
            };
            let tangent = match form {
                SplineForm::Control => {
                    if first {
                        points.get(1).copied()
                    } else {
                        points.get(n.saturating_sub(2)).copied()
                    }
                }
                SplineForm::Fit => {
                    if first {
                        *start_handle
                    } else {
                        *end_handle
                    }
                }
            };
            return Some(SplineJoin {
                end: e,
                tangent,
                first,
                other_end: meet,
            });
        }
        None
    }
}

/// See [`Sketch::spline_join`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SplineJoin {
    pub end: PointId,
    pub tangent: Option<PointId>,
    pub first: bool,
    pub other_end: PointId,
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
