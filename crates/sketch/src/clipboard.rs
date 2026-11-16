//! Copy and paste of sketch geometry, within a sketch and between sketches.
//!
//! [`Sketch::extract`] copies chosen curves and points (and every point of the curves)
//! into a sketch of their own, with the constraints that only refer to what is copied
//! (constraints to the rest of the sketch cannot come along: what they refer to is not
//! there). Driven dimensions stay driven. [`Sketch::paste`] adds such a clip to a sketch,
//! moved by an offset: points and curves get new ids, constraints refer to them, and a
//! fix moves with the offset, so the pasted copy has the same shape, the same constraints
//! and the same degrees of freedom as the original.

use crate::entity::{ConstraintId, CurveId, PointId};
use crate::placed::SketchTransform;
use crate::sketch::Sketch;
use crate::tools::{ToolError, ToolReport, check_point, refuse, report};
use chamfera_geom::Point2;
use std::collections::{BTreeMap, BTreeSet};

impl Sketch {
    /// A sketch holding copies of `curves` and `points` (and the curves' points), with the
    /// constraints among them. Ids are those of `self`, so they are unique in the clip.
    pub fn extract(&self, curves: &[CurveId], points: &[PointId]) -> Result<Sketch, ToolError> {
        let (cs, ps) = self.copy_set(curves, points, None)?;
        let mut clip = Sketch::new();
        let mut pmap = BTreeMap::new();
        for p in &ps {
            let at = self.point(*p).ok_or(crate::SketchError::UnknownPoint(*p))?;
            pmap.insert(*p, clip.add_point(at)?);
        }
        let mp = |p: PointId| pmap.get(&p).copied().unwrap_or(p);
        let mut cmap = BTreeMap::new();
        for c in &cs {
            let curve = self.curve(*c).ok_or(crate::SketchError::UnknownCurve(*c))?;
            cmap.insert(*c, clip.add_curve(curve.map_points(mp))?);
        }
        let mc = |k: CurveId| cmap.get(&k).copied().unwrap_or(k);
        let pset: BTreeSet<PointId> = ps.iter().copied().collect();
        let cset: BTreeSet<CurveId> = cs.iter().copied().collect();
        let same = SketchTransform::Translate { dx: 0.0, dy: 0.0 };
        for (id, c) in self.constraints() {
            let internal = c.point_refs().iter().all(|p| pset.contains(p))
                && c.curve_refs().iter().all(|k| cset.contains(k));
            if !internal {
                continue;
            }
            if let Some(copy) = same.constraint(c, &mp, &mc) {
                let new = clip.add_constraint(copy)?;
                if self.is_driven(id) {
                    clip.set_driven(new, true)?;
                }
            }
        }
        Ok(clip)
    }

    /// The middle of a clip's bounding box, or `None` for an empty one.
    pub fn centre(&self) -> Option<Point2> {
        let mut it = self.points().map(|(_, p)| p);
        let first = it.next()?;
        let (mut lo, mut hi) = (first, first);
        for p in it {
            lo = Point2::new(lo.x.min(p.x), lo.y.min(p.y));
            hi = Point2::new(hi.x.max(p.x), hi.y.max(p.y));
        }
        Some(lo.lerp(hi, 0.5))
    }

    /// Adds `clip` (from [`Sketch::extract`]) moved by `(dx, dy)` mm, and solves. On
    /// failure the sketch is unchanged.
    pub fn paste(&mut self, clip: &Sketch, dx: f64, dy: f64) -> Result<ToolReport, ToolError> {
        if clip.is_empty() {
            return refuse("there is nothing to paste: copy some sketch geometry first");
        }
        let t = SketchTransform::Translate { dx, dy };
        let backup = self.clone();
        let result = (|| -> Result<(), ToolError> {
            let mut pmap = BTreeMap::new();
            for (id, at) in clip.points() {
                let q = t.apply(at);
                check_point("a pasted point", q)?;
                pmap.insert(id, self.add_point(q)?);
            }
            let mp = |p: PointId| pmap.get(&p).copied().unwrap_or(p);
            let mut cmap = BTreeMap::new();
            for (id, curve) in clip.curves() {
                cmap.insert(id, self.add_curve(curve.map_points(mp))?);
            }
            let mc = |k: CurveId| cmap.get(&k).copied().unwrap_or(k);
            let mut driven: Vec<ConstraintId> = Vec::new();
            for (id, c) in clip.constraints() {
                if let Some(copy) = t.constraint(c, &mp, &mc) {
                    let new = self.add_constraint(copy)?;
                    if clip.is_driven(id) {
                        driven.push(new);
                    }
                }
            }
            for d in driven {
                self.set_driven(d, true)?;
            }
            self.validate()?;
            self.solve()?;
            Ok(())
        })();
        match result {
            Ok(()) => Ok(report(&backup, self)),
            Err(e) => {
                *self = backup;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Constraint, Sketch};
    use chamfera_geom::Point2;

    /// A 30 x 20 rectangle with horizontal, vertical, length and a fix on one corner.
    fn rectangle(s: &mut Sketch) -> Vec<crate::CurveId> {
        let c = [(0.0, 0.0), (30.0, 0.0), (30.0, 20.0), (0.0, 20.0)];
        let p: Vec<_> = c
            .iter()
            .map(|(x, y)| s.add_point(Point2::new(*x, *y)).unwrap())
            .collect();
        let l: Vec<_> = (0..4)
            .map(|i| s.add_line(p[i], p[(i + 1) % 4]).unwrap())
            .collect();
        for k in [0, 2] {
            s.add_constraint(Constraint::Horizontal { line: l[k] })
                .unwrap();
        }
        for k in [1, 3] {
            s.add_constraint(Constraint::Vertical { line: l[k] })
                .unwrap();
        }
        s.add_constraint(Constraint::Length {
            line: l[0],
            value: 30.0,
        })
        .unwrap();
        s.add_constraint(Constraint::Length {
            line: l[1],
            value: 20.0,
        })
        .unwrap();
        s.fix_point(p[0]).unwrap();
        l
    }

    #[test]
    fn a_pasted_rectangle_keeps_its_constraints_and_is_fully_constrained() {
        // The rectangle is fully constrained: 8 coordinates, 4 H/V, 2 lengths and a fix
        // (2) = 8 equations. Its copy 50 mm to the right brings all seven constraints, its
        // fix moved to (50, 0), so the sketch still has 0 degrees of freedom and two
        // profiles of 600 mm².
        let mut s = Sketch::new();
        let lines = rectangle(&mut s);
        assert_eq!(s.analyze().unwrap().dof, 0);
        let clip = s.extract(&lines, &[]).unwrap();
        assert_eq!(clip.constraints().count(), 7);
        let r = s.paste(&clip, 50.0, 0.0).unwrap();
        assert_eq!(r.created_curves.len(), 4);
        assert_eq!(r.created_constraints.len(), 7);
        assert_eq!(s.analyze().unwrap().dof, 0);
        let mut areas: Vec<f64> = s.profiles().iter().map(|p| p.area).collect();
        areas.sort_by(f64::total_cmp);
        assert_eq!(areas.len(), 2);
        assert!(areas.iter().all(|a| (a - 600.0).abs() < 1e-9), "{areas:?}");
        let moved = r
            .created_points
            .iter()
            .filter_map(|p| s.point(*p))
            .any(|q| q.distance(Point2::new(50.0, 0.0)) < 1e-9);
        assert!(moved);
        // Changing the copy's width dimension resizes only the copy.
        let width = r
            .created_constraints
            .iter()
            .copied()
            .find(|c| matches!(s.constraint(*c), Some(Constraint::Length { value, .. }) if (*value - 30.0).abs() < 1e-12))
            .unwrap();
        s.set_dimension(width, 40.0).unwrap();
        s.solve().unwrap();
        let mut areas: Vec<f64> = s.profiles().iter().map(|p| p.area).collect();
        areas.sort_by(f64::total_cmp);
        assert!((areas[0] - 600.0).abs() < 1e-9 && (areas[1] - 800.0).abs() < 1e-9);
    }

    #[test]
    fn constraints_to_geometry_left_behind_are_not_copied() {
        // Copy only the bottom side: its horizontal, length and the corner fix come
        // along; the vertical sides' constraints do not.
        let mut s = Sketch::new();
        let lines = rectangle(&mut s);
        let clip = s.extract(&lines[..1], &[]).unwrap();
        assert_eq!(clip.curves().count(), 1);
        assert_eq!(clip.constraints().count(), 3);
        // Pasting into another sketch works the same.
        let mut other = Sketch::new();
        other.paste(&clip, 0.0, 10.0).unwrap();
        assert_eq!(other.constraints().count(), 3);
        assert!(other.paste(&Sketch::new(), 0.0, 0.0).is_err());
        assert!(other.paste(&clip, 1e300, 0.0).is_err());
        assert_eq!(other.curves().count(), 1);
    }
}
