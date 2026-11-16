//! The view cube's geometry, without any drawing: the 26 regions of a cube (6 faces,
//! 12 edges, 8 corners), which region a point of the screen is over, the view each
//! region looks from, and the patch each region covers on a face.
//!
//! The cube is the box [-1, 1]^3 in model axes (Z up, front is -Y, right is +X). A
//! region is named by the signs of the direction it looks from: (0, -1, 0) is the front
//! face, (1, -1, 0) the front-right edge, (1, -1, 1) the top-front-right corner.

use crate::{Basis, PITCH_LIMIT};
use chamfera_geom::Vec3;
use std::f64::consts::FRAC_PI_2;

/// How far an edge or corner region reaches into each face, as a fraction of the half
/// edge (the face's middle region is the square inside this band).
pub const BAND: f64 = 0.3;

/// One of the 26 regions of the view cube: each part is -1, 0 or 1 and at least one is
/// not 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CubeRegion(pub [i8; 3]);

/// Whether a region is a face, an edge or a corner of the cube.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    Face,
    Edge,
    Corner,
}

impl CubeRegion {
    /// Every region: the 26 sign triples that are not all zero.
    pub fn all() -> Vec<CubeRegion> {
        let mut out = Vec::with_capacity(26);
        for x in -1..=1i8 {
            for y in -1..=1i8 {
                for z in -1..=1i8 {
                    if (x, y, z) != (0, 0, 0) {
                        out.push(CubeRegion([x, y, z]));
                    }
                }
            }
        }
        out
    }

    pub fn kind(self) -> RegionKind {
        match self.0.iter().filter(|s| **s != 0).count() {
            1 => RegionKind::Face,
            2 => RegionKind::Edge,
            _ => RegionKind::Corner,
        }
    }

    /// The direction the eye looks from (from the target toward the eye), unit length.
    pub fn direction(self) -> Vec3 {
        let [x, y, z] = self.0.map(f64::from);
        let len = (x * x + y * y + z * z).sqrt().max(1.0);
        Vec3::new(x / len, y / len, z / len)
    }

    /// The camera's `(yaw, pitch)` for a view from this region, as `Camera` keeps them:
    /// yaw around +Z from +X, pitch up from the XY plane. Straight down or up keeps +Y
    /// up on screen, as the Top and Bottom views do.
    pub fn angles(self) -> (f64, f64) {
        let d = self.direction();
        let yaw = if self.0[0] == 0 && self.0[1] == 0 {
            -FRAC_PI_2
        } else {
            d.y.atan2(d.x)
        };
        let pitch = d.z.clamp(-1.0, 1.0).asin().clamp(-PITCH_LIMIT, PITCH_LIMIT);
        (yaw, pitch)
    }

    /// The region's name: its words from top or bottom, front or back, then left or
    /// right, joined by `_` ("front", "top_front", "top_front_right").
    pub fn name(self) -> String {
        let [x, y, z] = self.0;
        let words: Vec<&str> = [
            match z {
                1 => Some("top"),
                -1 => Some("bottom"),
                _ => None,
            },
            match y {
                -1 => Some("front"),
                1 => Some("back"),
                _ => None,
            },
            match x {
                1 => Some("right"),
                -1 => Some("left"),
                _ => None,
            },
        ]
        .into_iter()
        .flatten()
        .collect();
        words.join("_")
    }

    /// Reads a name made of the words top, bottom, front, back, left and right in any
    /// order, joined by `_`, `-` or spaces, each direction at most once.
    pub fn parse(s: &str) -> Option<CubeRegion> {
        let mut signs = [0i8; 3];
        let mut words = 0;
        for w in s.split(['_', '-', ' ']).filter(|w| !w.is_empty()) {
            words += 1;
            if words > 3 {
                return None;
            }
            let (axis, sign) = match w {
                "right" => (0, 1),
                "left" => (0, -1),
                "back" => (1, 1),
                "front" => (1, -1),
                "top" => (2, 1),
                "bottom" => (2, -1),
                _ => return None,
            };
            let slot = signs.get_mut(axis)?;
            if *slot != 0 {
                return None;
            }
            *slot = sign;
        }
        (words > 0).then_some(CubeRegion(signs))
    }

    /// The region a point on the cube's surface lies in.
    pub fn at_surface_point(p: Vec3) -> Option<CubeRegion> {
        let sign = |c: f64| {
            if c > 1.0 - BAND {
                1
            } else if c < -1.0 + BAND {
                -1
            } else {
                0
            }
        };
        let r = CubeRegion([sign(p.x), sign(p.y), sign(p.z)]);
        (r.0 != [0, 0, 0]).then_some(r)
    }

    /// The patch this region covers on the face whose outward normal is `sign` along
    /// `axis` (0 = X, 1 = Y, 2 = Z): four corners on the face, or `None` when the region
    /// does not touch that face.
    pub fn patch(self, axis: usize, sign: i8) -> Option<[Vec3; 4]> {
        if self.0.get(axis) != Some(&sign) || sign == 0 {
            return None;
        }
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let span = |s: i8| match s {
            1 => (1.0 - BAND, 1.0),
            -1 => (-1.0, -1.0 + BAND),
            _ => (-1.0 + BAND, 1.0 - BAND),
        };
        let (u0, u1) = span(*self.0.get(u)?);
        let (v0, v1) = span(*self.0.get(v)?);
        let at = |a: f64, b: f64| {
            let mut c = [0.0; 3];
            c[axis] = f64::from(sign);
            c[u] = a;
            c[v] = b;
            Vec3::new(c[0], c[1], c[2])
        };
        Some([at(u0, v0), at(u1, v0), at(u1, v1), at(u0, v1)])
    }
}

/// The region under a point of the screen, for a cube drawn with `basis` (orthographic,
/// as the view cube is). `sx` is right and `sy` up from the cube's centre on screen, in
/// half edges. `None` off the cube.
pub fn region_at(basis: &Basis, sx: f64, sy: f64) -> Option<CubeRegion> {
    if !sx.is_finite() || !sy.is_finite() {
        return None;
    }
    // A ray along the view through that point, starting outside the cube (its corners
    // are sqrt(3) from the centre).
    let d = basis.forward;
    let o = basis.right * sx + basis.up * sy - d * 4.0;
    let (o, d) = ([o.x, o.y, o.z], [d.x, d.y, d.z]);
    let (mut near, mut far) = (f64::NEG_INFINITY, f64::INFINITY);
    for i in 0..3 {
        if d[i].abs() < 1e-12 {
            if o[i].abs() > 1.0 {
                return None;
            }
            continue;
        }
        let (a, b) = ((-1.0 - o[i]) / d[i], (1.0 - o[i]) / d[i]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    if near > far || !near.is_finite() {
        return None;
    }
    let p = Vec3::new(o[0] + d[0] * near, o[1] + d[1] * near, o[2] + d[2] * near);
    CubeRegion::at_surface_point(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, StandardView};

    fn basis(view: StandardView) -> Basis {
        let mut c = Camera::default();
        c.set_view(view);
        c.basis()
    }

    #[test]
    fn there_are_6_faces_12_edges_and_8_corners() {
        let all = CubeRegion::all();
        assert_eq!(all.len(), 26);
        let count = |k| all.iter().filter(|r| r.kind() == k).count();
        assert_eq!(count(RegionKind::Face), 6);
        assert_eq!(count(RegionKind::Edge), 12);
        assert_eq!(count(RegionKind::Corner), 8);
    }

    #[test]
    fn names_read_back_and_are_all_different() {
        let all = CubeRegion::all();
        let mut names: Vec<String> = all.iter().map(|r| r.name()).collect();
        for r in &all {
            assert_eq!(CubeRegion::parse(&r.name()), Some(*r), "{}", r.name());
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 26);
        assert_eq!(CubeRegion([1, -1, 1]).name(), "top_front_right");
        assert_eq!(CubeRegion([0, 1, -1]).name(), "bottom_back");
        // Any order and separator; no repeats, opposites or unknown words.
        assert_eq!(
            CubeRegion::parse("right-front top"),
            Some(CubeRegion([1, -1, 1]))
        );
        assert_eq!(CubeRegion::parse("top_bottom"), None);
        assert_eq!(CubeRegion::parse("top_top"), None);
        assert_eq!(CubeRegion::parse("up"), None);
        assert_eq!(CubeRegion::parse(""), None);
        assert_eq!(CubeRegion::parse("top_front_right_left"), None);
    }

    #[test]
    fn each_region_looks_from_its_direction() {
        use std::f64::consts::{FRAC_PI_4, PI};
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        // The front-right edge: from (1, -1, 0), halfway between front (-pi/2) and right (0).
        let (y, p) = CubeRegion([1, -1, 0]).angles();
        assert!(close(y, -FRAC_PI_4) && close(p, 0.0));
        // The top-front edge: from (0, -1, 1), 45 degrees up from the front.
        let (y, p) = CubeRegion([0, -1, 1]).angles();
        assert!(close(y, -FRAC_PI_2) && close(p, FRAC_PI_4));
        // The back-left edge: from (-1, 1, 0), yaw 3pi/4.
        let (y, p) = CubeRegion([-1, 1, 0]).angles();
        assert!(close(y, 3.0 * FRAC_PI_4) && close(p, 0.0));
        // The left face: yaw pi.
        let (y, _) = CubeRegion([-1, 0, 0]).angles();
        assert!(close(y, PI));
        // A corner sits atan(1/sqrt 2) = asin(1/sqrt 3) above the XY plane.
        let (y, p) = CubeRegion([-1, 1, -1]).angles();
        assert!(close(y, 3.0 * FRAC_PI_4) && close(p, -(1.0 / 2f64.sqrt()).atan()));
        // The top-front-right corner is the isometric view; top and bottom match theirs.
        for (r, v) in [
            ([1, -1, 1], StandardView::Iso),
            ([0, 0, 1], StandardView::Top),
            ([0, 0, -1], StandardView::Bottom),
            ([0, -1, 0], StandardView::Front),
        ] {
            let ((y1, p1), (y2, p2)) = (CubeRegion(r).angles(), v.angles());
            assert!(close(y1, y2) && close(p1, p2), "{r:?}");
        }
        // For every region off the poles, the camera's eye direction is the region's.
        for r in CubeRegion::all().into_iter().filter(|r| r.0[..2] != [0, 0]) {
            let (yaw, pitch) = r.angles();
            let eye = Vec3::new(
                pitch.cos() * yaw.cos(),
                pitch.cos() * yaw.sin(),
                pitch.sin(),
            );
            assert!((eye - r.direction()).length() < 1e-12, "{}", r.name());
        }
    }

    #[test]
    fn the_screen_point_picks_the_region_under_it() {
        // Front view: right is +X, up is +Z, the front face (y = -1) faces the eye.
        let b = basis(StandardView::Front);
        assert_eq!(region_at(&b, 0.0, 0.0), Some(CubeRegion([0, -1, 0])));
        // Inside the band near the top-right corner of the front face.
        assert_eq!(region_at(&b, 0.9, 0.9), Some(CubeRegion([1, -1, 1])));
        assert_eq!(region_at(&b, 0.9, 0.0), Some(CubeRegion([1, -1, 0])));
        assert_eq!(region_at(&b, 0.0, -0.9), Some(CubeRegion([0, -1, -1])));
        // Just inside the middle square (1 - BAND = 0.7) is still the face.
        assert_eq!(region_at(&b, 0.69, 0.0), Some(CubeRegion([0, -1, 0])));
        assert_eq!(region_at(&b, 1.2, 0.0), None);
        assert_eq!(region_at(&b, f64::NAN, 0.0), None);
        // Top view: right is +X, up is +Y; the centre is the top face.
        let b = basis(StandardView::Top);
        assert_eq!(region_at(&b, 0.0, 0.0), Some(CubeRegion([0, 0, 1])));
        assert_eq!(region_at(&b, -0.9, 0.9), Some(CubeRegion([-1, 1, 1])));
        // Isometric: the ray through the centre meets the cube at the corner it looks
        // from, (1, -1, 1).
        let b = basis(StandardView::Iso);
        assert_eq!(region_at(&b, 0.0, 0.0), Some(CubeRegion([1, -1, 1])));
        // 1.2 half edges straight up on screen: the point is up * 1.2 + s * (1, -1, 1)/sqrt3
        // with up = (-1, 1, 2)/sqrt6, so x = -0.49 + 0.577s, y = 0.49 - 0.577s,
        // z = 0.98 + 0.577s. Coming from the eye (large s), z = 1 is met first, at
        // s = 0.035, where x = -0.47 and y = 0.47: inside the top face's middle square.
        assert_eq!(region_at(&b, 0.0, 1.2), Some(CubeRegion([0, 0, 1])));
    }

    #[test]
    fn patches_tile_each_face() {
        // On the front face (axis Y, sign -1) the nine regions touching it cover the
        // square [-1, 1]^2: their areas add up to 4.
        let mut area = 0.0;
        for r in CubeRegion::all() {
            if let Some([a, b, c, _]) = r.patch(1, -1) {
                area += (b - a).length() * (c - b).length();
                for q in [a, b, c] {
                    assert!((q.y + 1.0).abs() < 1e-12);
                }
            }
        }
        assert!((area - 4.0).abs() < 1e-12);
        assert!(CubeRegion([0, 1, 0]).patch(1, -1).is_none());
        // The front face's own patch is the middle square, 2 * (1 - BAND) wide.
        let [a, b, ..] = CubeRegion([0, -1, 0]).patch(1, -1).unwrap();
        assert!(((b - a).length() - 2.0 * (1.0 - BAND)).abs() < 1e-12);
    }
}
