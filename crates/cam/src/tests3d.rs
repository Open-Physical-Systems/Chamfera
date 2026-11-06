//! The drop-cutter, the 3D operations and the stock simulation against values derived
//! by hand (see each test's comment).

use crate::CamError;
use crate::drop::{DropModel, ToolShape};
use crate::ops::Cutter;
use crate::ops3d::{self, AdaptiveParams, Area, Cl};
use crate::sim::{self, Dexels};
use crate::toolpath::{Move, Toolpath};
use chamfera_geom::{Frame, Point2, Point3, Vec3};
use chamfera_kernel::{Body, Line2, Profile, ProfileEdge};
use std::f64::consts::TAU;

fn p2(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn cutter(d: f64) -> Cutter {
    Cutter {
        number: 1,
        name: "t".into(),
        diameter: d,
        spindle_rpm: 10_000.0,
        feed: 600.0,
        plunge_feed: 200.0,
    }
}

/// A box from (0, 0, 0) to (sx, sy, sz).
fn block(sx: f64, sy: f64, sz: f64) -> Body {
    Body::cuboid(Point3::new(0.0, 0.0, 0.0), Vec3::new(sx, sy, sz)).unwrap()
}

/// A solid hemisphere of radius `s` on the plane z = 0, centred at the origin: a quarter
/// disk in the XZ plane revolved about Z.
fn hemisphere(s: f64) -> Body {
    // Plane (u, v) -> world (u, 0, v): normal -Y, x along X (y = -Y × X = Z).
    let frame = Frame::from_normal_and_x(
        Point3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .unwrap();
    let arc = ProfileEdge::arc_from_points(p2(0.0, 0.0), p2(s, 0.0), p2(0.0, s)).unwrap();
    let profile = Profile::new(
        vec![
            ProfileEdge::line(p2(0.0, 0.0), p2(s, 0.0)),
            arc,
            ProfileEdge::line(p2(0.0, s), p2(0.0, 0.0)),
        ],
        Vec::new(),
    );
    let axis = Line2::new(p2(0.0, 0.0), chamfera_geom::Vec2::new(0.0, 1.0)).unwrap();
    chamfera_kernel::revolve(&frame, &profile, &axis, TAU).unwrap()
}

/// A solid cylinder of radius `rc` about the line x = 0, z = 0, from y = 0 to y = len.
fn lying_cylinder(rc: f64, len: f64) -> Body {
    let frame = Frame::from_normal_and_x(
        Point3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .unwrap();
    chamfera_kernel::extrude(&frame, &Profile::circle(p2(0.0, 0.0), rc), len).unwrap()
}

#[test]
fn a_ball_dropped_on_a_plane_rests_its_tip_on_it() {
    // Ball R = 3 over the middle of the top of a 40 x 40 x 10 block: the tip at z = 10.
    let m = DropModel::new(&[block(40.0, 40.0, 10.0)]).unwrap();
    let ball = ToolShape::new(3.0, 3.0).unwrap();
    for (x, y) in [(20.0, 20.0), (5.0, 31.0), (3.0, 3.0)] {
        let z = m.drop(&ball, x, y).unwrap();
        assert!((z - 10.0).abs() < 1e-9, "{z}");
    }
    // Missing the model entirely.
    assert_eq!(m.drop(&ball, 100.0, 100.0), None);
}

#[test]
fn a_ball_dropped_on_a_sphere_follows_the_offset_sphere() {
    // Ball R = 3 on a sphere of radius 20 centred at the origin, at XY offset d: the ball's
    // centre is on the sphere of radius 23, so the tip is at sqrt(23² - d²) - 3.
    let m = DropModel::new(&[hemisphere(20.0)]).unwrap();
    let ball = ToolShape::new(3.0, 3.0).unwrap();
    for d in [0.0, 2.5, 7.0, 12.0, 18.0, 21.0] {
        let want = (23.0f64 * 23.0 - d * d).sqrt() - 3.0;
        // Along a diagonal, so the seam is not special.
        let (x, y) = (d * 0.6, d * 0.8);
        let z = m.drop(&ball, x, y).unwrap();
        assert!((z - want).abs() < 1e-7, "d {d}: {z} vs {want}");
    }
}

#[test]
fn a_bull_nose_dropped_on_a_sphere_rests_its_ring_on_the_offset_sphere() {
    // Bull nose R = 4, corner r = 1 (its corner ring has radius e = 3, centred 1 above
    // the tip) on the sphere of radius 20 at XY offset d = 10: the ring's point nearest
    // the axis of the sphere, d - e = 7 out, is on the sphere of radius 21, so the ring is
    // at sqrt(21² - 7²) and the tip 1 lower. Within d <= e the ring's disk covers the top:
    // tip at 20 + 1 - 1 = 20.
    let m = DropModel::new(&[hemisphere(20.0)]).unwrap();
    let bull = ToolShape::new(4.0, 1.0).unwrap();
    let z = m.drop(&bull, 10.0, 0.0).unwrap();
    let want = (21.0f64 * 21.0 - 49.0).sqrt() - 1.0;
    assert!((z - want).abs() < 1e-7, "{z} vs {want}");
    let z = m.drop(&bull, 1.0, 1.0).unwrap();
    assert!((z - 20.0).abs() < 1e-7, "{z}");
}

#[test]
fn a_flat_tool_on_a_cylinders_side_rests_its_rim_on_it() {
    // A cylinder of radius 10 lying along Y with its axis at z = 0; a flat tool R = 3 at
    // y = 20 and x = d. For d <= 3 the top line (z = 10) is under the flat bottom. For
    // d > 3 the rim's nearest point is at x = d - 3, where the cylinder's top is
    // sqrt(100 - (d - 3)²): d = 8 gives sqrt(75), d = 12 sqrt(19).
    let m = DropModel::new(&[lying_cylinder(10.0, 40.0)]).unwrap();
    let flat = ToolShape::new(3.0, 0.0).unwrap();
    for (d, want) in [
        (0.0, 10.0),
        (2.5, 10.0),
        (8.0, 75f64.sqrt()),
        (12.0, 19f64.sqrt()),
    ] {
        let z = m.drop(&flat, d, 20.0).unwrap();
        assert!((z - want).abs() < 1e-7, "d {d}: {z} vs {want}");
    }
}

#[test]
fn edge_and_vertex_contacts_hold_the_tool_beside_a_block() {
    // A 20 x 20 x 10 block. A tool at x = 22, y = 10 overhangs the top edge x = 20 by
    // ρ = 2: flat R = 3 rests on it at 10; ball R = 3 at 10 - (3 - sqrt(9 - 4)) =
    // 7 + sqrt(5). Past the corner (22, 22) the vertex (20, 20, 10) is at ρ = sqrt(8):
    // ball tip 10 - 3 + sqrt(9 - 8) = 8.
    let m = DropModel::new(&[block(20.0, 20.0, 10.0)]).unwrap();
    let flat = ToolShape::new(3.0, 0.0).unwrap();
    let ball = ToolShape::new(3.0, 3.0).unwrap();
    assert!((m.drop(&flat, 22.0, 10.0).unwrap() - 10.0).abs() < 1e-9);
    let z = m.drop(&ball, 22.0, 10.0).unwrap();
    assert!((z - (7.0 + 5f64.sqrt())).abs() < 1e-9, "{z}");
    let z = m.drop(&ball, 22.0, 22.0).unwrap();
    assert!((z - 8.0).abs() < 1e-9, "{z}");
    // Bull nose R = 3, r = 1 (e = 2) at ρ = 2.5 from the edge: h = 1 - sqrt(1 - 0.25).
    let bull = ToolShape::new(3.0, 1.0).unwrap();
    let z = m.drop(&bull, 22.5, 10.0).unwrap();
    assert!((z - (9.0 + 0.75f64.sqrt())).abs() < 1e-9, "{z}");
}

#[test]
fn stock_to_leave_keeps_the_tool_that_far_off_the_part() {
    // Leaving l = 1 with a ball R = 3: the ball's centre is R + l from the surface. On the
    // plane z = 10 the tip is at 11; on the sphere of radius 20 at XY offset d the centre
    // is on the sphere of radius 24, the tip at sqrt(24² - d²) - 3.
    let ball = ToolShape::new(3.0, 3.0).unwrap();
    let plane = DropModel::new(&[block(40.0, 40.0, 10.0)]).unwrap();
    let cl = Cl::new(&plane, ball, 1.0, 0.0).unwrap();
    assert!((cl.at(20.0, 20.0) - 11.0).abs() < 1e-9);
    let dome = DropModel::new(&[hemisphere(20.0)]).unwrap();
    let cl = Cl::new(&dome, ball, 1.0, 0.0).unwrap();
    for d in [0.0, 9.0, 15.0] {
        let want = (24.0f64 * 24.0 - d * d).sqrt() - 3.0;
        assert!((cl.at(d, 0.0) - want).abs() < 1e-7, "{d}");
    }
    // A flat R = 4 leaving 0.5 beside the 20 x 20 x 10 block's edge (x = 20) at x = 22:
    // the tool's rim is 2 in, so its tip is 0.5 above the top: 10.5.
    let flat = ToolShape::new(4.0, 0.0).unwrap();
    let small = DropModel::new(&[block(20.0, 20.0, 10.0)]).unwrap();
    let cl = Cl::new(&small, flat, 0.5, 0.0).unwrap();
    assert!((cl.at(22.0, 10.0) - 10.5).abs() < 1e-9);
}

#[test]
fn the_scallop_stepover_is_two_root_two_r_h_minus_h_squared() {
    // R = 3, h = 0.01: s = 2 sqrt(0.06 - 0.0001) = 2 sqrt(0.0599). At h = R, s = 2R.
    let s = ops3d::scallop_stepover(3.0, 0.01).unwrap();
    assert!((s - 2.0 * 0.0599f64.sqrt()).abs() < 1e-12);
    assert!((ops3d::scallop_stepover(3.0, 3.0).unwrap() - 6.0).abs() < 1e-12);
    assert!(ops3d::scallop_stepover(3.0, 4.0).is_err());
    assert!(ops3d::scallop_stepover(3.0, f64::NAN).is_err());
}

/// Every feed position of a toolpath.
fn feed_points(tp: &Toolpath) -> Vec<Point3> {
    tp.moves
        .iter()
        .filter_map(|m| match m {
            Move::Line { to, .. } | Move::Plunge { to, .. } => Some(*to),
            _ => None,
        })
        .collect()
}

#[test]
fn a_parallel_pass_over_a_hemisphere_follows_the_offset_sphere() {
    // Ball R = 3 over a hemisphere of radius 20: wherever the ball touches the sphere
    // (d < 23) its tip is at sqrt(23² - d²) - 3, but never below the floor z = 0 (the
    // model's bottom). Every point is an exact drop; every chord's midpoint is within the
    // tolerance of the surface (each span was halved until it was).
    let m = DropModel::new(&[hemisphere(20.0)]).unwrap();
    let ball = ToolShape::new(3.0, 3.0).unwrap();
    let cl = Cl::new(&m, ball, 0.0, 0.0).unwrap();
    let tol = 0.01;
    let area = Area {
        min: p2(-24.0, -24.0),
        max: p2(24.0, 24.0),
    };
    let tp = ops3d::parallel(&cl, &cutter(6.0), area, 2.0, 0.0, tol, 30.0).unwrap();
    let want = |p: Point3| {
        let d = p.x.hypot(p.y);
        if d < 23.0 {
            ((23.0f64 * 23.0 - d * d).sqrt() - 3.0).max(0.0)
        } else {
            0.0
        }
    };
    let pts = feed_points(&tp);
    assert!(pts.len() > 100);
    for p in &pts {
        assert!((p.z - want(*p)).abs() < 1e-6, "{p:?} vs {}", want(*p));
    }
    for w in pts.windows(2) {
        let mid = Point3::new(
            (w[0].x + w[1].x) / 2.0,
            (w[0].y + w[1].y) / 2.0,
            (w[0].z + w[1].z) / 2.0,
        );
        // The chord lies under a convex surface: only check it does not stray far.
        assert!(
            (mid.z - want(mid)).abs() <= tol + 1e-6,
            "{mid:?} vs {}",
            want(mid)
        );
    }
    // The passes cover y from -24 to 24 at most 2 apart: 25 passes.
    let mut ys: Vec<i64> = pts.iter().map(|p| (p.y * 1000.0).round() as i64).collect();
    ys.sort_unstable();
    ys.dedup();
    assert!(ys.contains(&-24_000) && ys.contains(&24_000));
}

#[test]
fn a_waterline_contour_round_a_hemisphere_is_a_circle() {
    // Ball R = 3 round a hemisphere of radius 20 at tip height z: the ball's centre at
    // z + 3 lies on the sphere of radius 23, so the contour is the circle of radius
    // sqrt(23² - (z + 3)²) about the axis.
    let m = DropModel::new(&[hemisphere(20.0)]).unwrap();
    let ball = ToolShape::new(3.0, 3.0).unwrap();
    let cl = Cl::new(&m, ball, 0.0, 0.0).unwrap();
    let area = Area {
        min: p2(-26.0, -26.0),
        max: p2(26.0, 26.0),
    };
    let tol = 0.01;
    let tp = ops3d::waterline(&cl, &cutter(6.0), area, 20.0, 0.0, 5.0, tol, 30.0).unwrap();
    let pts = feed_points(&tp);
    assert!(!pts.is_empty());
    for p in pts {
        let want = (23.0f64 * 23.0 - (p.z + 3.0).powi(2)).sqrt();
        let d = p.x.hypot(p.y);
        assert!((d - want).abs() < tol, "{p:?}: {d} vs {want}");
    }
}

/// Recomputes the engagement along an adaptive path on its own material grid (finer
/// than the generator's): at each level, the stock minus the part; level moves are
/// sampled every `step` and measured before they cut; moves that change height (ramps,
/// helices, plunges) cut without being measured.
fn max_engagement(
    tp: &Toolpath,
    r: f64,
    stock: (Point2, Point2),
    part: &dyn Fn(Point2, f64) -> bool,
    cell: f64,
    depth: f64,
) -> f64 {
    let pad = r + 3.0;
    let g = crate::grid::Grid::covering(
        p2(stock.0.x - pad, stock.0.y - pad),
        p2(stock.1.x + pad, stock.1.y + pad),
        cell,
        1024,
        cell * 2.0,
    )
    .unwrap();
    let fresh = |z: f64| -> Vec<bool> {
        (0..g.len())
            .map(|k| {
                let q = g.at(k % g.nx, k / g.nx);
                q.x >= stock.0.x
                    && q.x <= stock.1.x
                    && q.y >= stock.0.y
                    && q.y <= stock.1.y
                    && !part(q, z)
            })
            .collect()
    };
    let mut worst = 0.0f64;
    let mut level = f64::NAN;
    let mut mat = Vec::new();
    let mut at = tp.start().unwrap();
    for piece in sim::segments(tp) {
        let (a, b) = (piece.a, piece.b);
        at = b;
        if piece.feed.is_none() {
            continue;
        }
        let flat = (a.z - b.z).abs() < 1e-9;
        if (level.is_nan() || (b.z - level).abs() > 1e-9) && flat {
            level = b.z;
            mat = fresh(level);
        }
        let n = ((p2(a.x, a.y).distance(p2(b.x, b.y)) / cell).ceil() as usize).max(1);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let c = p2(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
            if flat && !mat.is_empty() {
                let e = ops3d::engagement(&g, &mat, c, r, depth);
                worst = worst.max(e);
            }
            if !mat.is_empty() {
                // Cut the cells within the tool's reach.
                let ri = (r / g.cell).ceil() as i64 + 1;
                if let Some((ci, cj)) = g.nearest(c) {
                    for dj in -ri..=ri {
                        for di in -ri..=ri {
                            let (i, j) = (ci as i64 + di, cj as i64 + dj);
                            if i >= 0 && j >= 0 && (i as usize) < g.nx && (j as usize) < g.ny {
                                let k = g.index(i as usize, j as usize);
                                if g.at(i as usize, j as usize).distance(c) <= r {
                                    mat[k] = false;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let _ = at;
    worst
}

#[test]
fn adaptive_clearing_never_engages_more_than_its_limit() {
    // A 30 x 20 x 12 stock round a 10 x 6 x 10 boss standing on the stock's bottom at
    // z = 0, flat tool R = 3, 60 degree limit, 6 mm levels. Measured again here on a grid
    // of 3/4 of the generator's cells, on the tool's circumference before each step cuts,
    // the engagement stays within 60 degrees.
    let boss = Body::cuboid(Point3::new(10.0, 7.0, 0.0), Vec3::new(10.0, 6.0, 10.0)).unwrap();
    let m = DropModel::new(&[boss]).unwrap();
    let flat = ToolShape::new(3.0, 0.0).unwrap();
    let cl = Cl::new(&m, flat, 0.0, 0.0).unwrap();
    let limit = 60f64.to_radians();
    let res = ops3d::adaptive(
        &cl,
        &cutter(6.0),
        AdaptiveParams {
            stock_min: p2(0.0, 0.0),
            stock_max: p2(30.0, 20.0),
            stock_top: 12.0,
            bottom: 0.0,
            stepdown: 6.0,
            max_engagement: limit,
            clearance: 20.0,
        },
    )
    .unwrap();
    let part = |q: Point2, z: f64| {
        z < 10.0 - 1e-9 && q.x >= 10.0 && q.x <= 20.0 && q.y >= 7.0 && q.y <= 13.0
    };
    let worst = max_engagement(
        &res.toolpath,
        3.0,
        (p2(0.0, 0.0), p2(30.0, 20.0)),
        &part,
        res.cell * 0.75,
        0.0,
    );
    assert!(worst <= limit, "{} degrees", worst.to_degrees());
    // It does cut: simulated, no gouges, and nearly all of the stock minus the part,
    // 30·20·12 - 10·6·10 = 6600 mm³, is gone (the last level is the floor z = 0).
    let mut d = Dexels::new(
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(30.0, 20.0, 12.0),
        150,
    )
    .unwrap();
    d.cut_toolpath(&flat, &res.toolpath, 5000.0, 1.0);
    let heights = sim::part_heights(&m, &d).unwrap();
    let rep = sim::report(&d, &heights, 0.02);
    assert_eq!(rep.gouge_cells, 0, "{rep:?}");
    assert!(
        rep.removed_volume > 0.9 * 6600.0,
        "{rep:?} skipped {}",
        res.skipped
    );
}

#[test]
fn simulating_a_slot_removes_its_swept_volume() {
    // A flat tool R = 3 straight across a 50 x 30 x 10 stock at y = 15, 4 deep: it
    // removes a slot 50 long, 6 wide and 4 deep plus the two half disks past the ends,
    // which are outside the stock: 50 * 6 * 4 = 1200 mm³. The columns are counted at
    // their centres, so the error is at most the depth times the band of half a cell's
    // diagonal along the slot's two long sides: 4 * 2 * 50 * (h / sqrt 2).
    let mut d = Dexels::new(
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(50.0, 30.0, 10.0),
        250,
    )
    .unwrap();
    let flat = ToolShape::new(3.0, 0.0).unwrap();
    d.cut_line(
        &flat,
        Point3::new(-5.0, 15.0, 6.0),
        Point3::new(55.0, 15.0, 6.0),
    );
    let tol = 4.0 * 2.0 * 50.0 * d.cell / 2f64.sqrt();
    assert!(
        (d.removed_volume() - 1200.0).abs() <= tol,
        "{}",
        d.removed_volume()
    );
    // A ball R = 3 dropped to its centre at z = 8 along the same line cuts a half-round
    // groove: area π 9 / 2 over 50 mm (the cylinder's lower half below z = 8... the tip
    // at 5 touches 5): volume 50 * 9π/2.
    let mut d = Dexels::new(
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(50.0, 30.0, 8.0),
        250,
    )
    .unwrap();
    let ball = ToolShape::new(3.0, 3.0).unwrap();
    d.cut_line(
        &ball,
        Point3::new(-5.0, 15.0, 5.0),
        Point3::new(55.0, 15.0, 5.0),
    );
    let want = 50.0 * 9.0 * std::f64::consts::PI / 2.0;
    let tol = 3.0 * 2.0 * 50.0 * d.cell / 2f64.sqrt();
    assert!(
        (d.removed_volume() - want).abs() <= tol,
        "{} vs {want}",
        d.removed_volume()
    );
}

#[test]
fn a_cut_below_the_part_is_a_gouge() {
    // A 20 x 20 x 10 block as both stock and part; a flat tool R = 2 straight across at
    // z = 9 cuts 1 mm into the part: about 20 * 4 * 1 = 80 mm³ of gouge, max depth 1.
    let m = DropModel::new(&[block(20.0, 20.0, 10.0)]).unwrap();
    let mut d = Dexels::new(
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(20.0, 20.0, 10.0),
        100,
    )
    .unwrap();
    let flat = ToolShape::new(2.0, 0.0).unwrap();
    d.cut_line(
        &flat,
        Point3::new(-5.0, 10.0, 9.0),
        Point3::new(25.0, 10.0, 9.0),
    );
    let h = sim::part_heights(&m, &d).unwrap();
    let rep = sim::report(&d, &h, 0.02);
    assert!(rep.gouge_cells > 0);
    assert!((rep.max_gouge - 1.0).abs() < 1e-9, "{rep:?}");
    let tol = 1.0 * 2.0 * 20.0 * d.cell / 2f64.sqrt();
    assert!((rep.gouge_volume - 80.0).abs() <= tol, "{rep:?}");
    // The gouged tops show apart.
    let (_, bad) = sim::meshes(&d, &h, 0.02, 256);
    assert!(!bad.triangles.is_empty());
}

#[test]
fn capped_sizes_are_refused() {
    let lo = Point3::new(0.0, 0.0, 0.0);
    let hi = Point3::new(50.0, 30.0, 10.0);
    assert!(matches!(
        Dexels::new(lo, hi, 1025),
        Err(CamError::TooComplex(_))
    ));
    assert!(Dexels::new(lo, hi, 0).is_err());
    assert!(Dexels::new(lo, hi, 1024).is_ok());
    // A parallel op whose passes would need more drops than the cap.
    let m = DropModel::new(&[block(20.0, 20.0, 10.0)]).unwrap();
    let cl = Cl::new(&m, ToolShape::new(0.1, 0.1).unwrap(), 0.0, 0.0).unwrap();
    let area = Area {
        min: p2(-1000.0, -1000.0),
        max: p2(1000.0, 1000.0),
    };
    assert!(matches!(
        ops3d::parallel(&cl, &cutter(0.2), area, 0.01, 0.0, 0.01, 30.0),
        Err(CamError::TooComplex(_))
    ));
    // A grid coarser than the tool allows is refused.
    assert!(matches!(
        crate::grid::Grid::covering(p2(0.0, 0.0), p2(10_000.0, 10.0), 0.1, 1024, 1.0),
        Err(CamError::TooComplex(_))
    ));
}

#[test]
fn hostile_inputs_never_panic() {
    let m = DropModel::new(&[hemisphere(20.0)]).unwrap();
    let ball = ToolShape::new(3.0, 3.0).unwrap();
    for v in [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e300, -1e300] {
        assert!(m.drop(&ball, v, 0.0).is_none() || v.is_finite());
        let _ = m.drop(&ball, 0.0, v);
        assert!(ToolShape::new(v, 0.0).is_err() || v.is_finite());
        assert!(ToolShape::new(3.0, v).is_err());
        let cl = Cl::new(&m, ball, 0.0, 0.0).unwrap();
        let area = Area {
            min: p2(-10.0, -10.0),
            max: p2(10.0, 10.0),
        };
        let _ = ops3d::parallel(&cl, &cutter(6.0), area, v, 0.0, 0.01, 30.0);
        let _ = ops3d::parallel(&cl, &cutter(6.0), area, 2.0, v, 0.01, 30.0);
        let _ = ops3d::parallel(&cl, &cutter(6.0), area, 2.0, 0.0, v, 30.0);
        let _ = ops3d::waterline(&cl, &cutter(6.0), area, v, 0.0, 2.0, 0.01, 30.0);
        let _ = ops3d::waterline(&cl, &cutter(6.0), area, 20.0, 0.0, v, 0.01, 30.0);
        assert!(Cl::new(&m, ball, v, 0.0).is_err() || v.is_finite());
        let _ = Dexels::new(Point3::new(0.0, 0.0, 0.0), Point3::new(v, 1.0, 1.0), 10);
        let mut d = Dexels::new(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(10.0, 10.0, 10.0),
            10,
        )
        .unwrap();
        d.cut_line(&ball, Point3::new(v, 0.0, 0.0), Point3::new(1.0, v, 0.0));
        let tp = Toolpath {
            tool_number: 1,
            tool_name: "t".into(),
            spindle_rpm: 1.0,
            moves: vec![Move::Line {
                to: Point3::new(v, 1.0, 2.0),
                feed: v,
            }],
        };
        d.cut_toolpath(&ball, &tp, v, v);
        let _ = ops3d::adaptive(
            &cl,
            &cutter(6.0),
            AdaptiveParams {
                stock_min: p2(0.0, 0.0),
                stock_max: p2(10.0, 10.0),
                stock_top: 12.0,
                bottom: 0.0,
                stepdown: 4.0,
                max_engagement: v,
                clearance: 20.0,
            },
        );
    }
}
