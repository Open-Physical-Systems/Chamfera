//! Holders, pencil and scallop finishing, and cancelling, against values derived by hand
//! (see each test's comment).

use crate::CamError;
use crate::drop::{DropModel, ToolShape};
use crate::holder::{NonCutting, PartCheck};
use crate::ops::Cutter;
use crate::ops3d::{self, Area, Cl, HolderUse};
use crate::sim::{self, Dexels, HolderHits};
use crate::toolpath::{Move, Toolpath};
use chamfera_doc::cam::{Holder, HolderCheck, HolderSegment, Tool, ToolKind};
use chamfera_geom::{Frame, Point2, Point3, Vec3};
use chamfera_kernel::{Body, Profile, ProfileEdge};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

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

fn cuboid(x: f64, y: f64, z: f64, sx: f64, sy: f64, sz: f64) -> Body {
    Body::cuboid(Point3::new(x, y, z), Vec3::new(sx, sy, sz)).unwrap()
}

fn feed_points(tp: &Toolpath) -> Vec<Point3> {
    tp.moves
        .iter()
        .filter_map(|m| match m {
            Move::Line { to, .. } | Move::Plunge { to, .. } => Some(*to),
            _ => None,
        })
        .collect()
}

/// A 6 mm tool of `kind` with 5 mm flutes, held 8 mm out of a 20 mm holder 30 mm long.
fn held_tool(kind: ToolKind) -> Tool {
    Tool {
        id: 1,
        number: 1,
        name: "held".into(),
        kind,
        diameter: 6.0,
        flute_length: 5.0,
        corner_radius: if kind == ToolKind::BallEndMill {
            3.0
        } else {
            0.0
        },
        flutes: 2,
        tip_angle: 0.0,
        spindle_rpm: 10_000.0,
        feed: 600.0,
        plunge_feed: 200.0,
        holder: Some(Holder {
            stickout: 8.0,
            shank_diameter: None,
            segments: vec![HolderSegment {
                diameter: 20.0,
                top_diameter: None,
                length: 30.0,
            }],
        }),
    }
}

#[test]
fn a_holder_is_a_stack_of_pieces_above_the_flutes() {
    // Flutes 0..5, shank r 3 from 5 to 8, holder r 10 from 8 to 38, then a cone from
    // r 10 to r 20 over 10 mm (38..48).
    let mut t = held_tool(ToolKind::FlatEndMill);
    if let Some(h) = t.holder.as_mut() {
        h.segments.push(HolderSegment {
            diameter: 20.0,
            top_diameter: Some(40.0),
            length: 10.0,
        });
    }
    let nc = NonCutting::of(&t).unwrap().unwrap();
    assert_eq!(nc.pieces.len(), 3);
    assert!((nc.lowest() - 5.0).abs() < 1e-12);
    assert!((nc.max_radius() - 20.0).abs() < 1e-12);
    // Inside the shank's radius the shank (from 5) is lowest; out to 10 the holder's
    // face at 8; in the cone at r 15 halfway up it: 38 + 5 = 43; past 20, nothing.
    assert!((nc.low(2.0).unwrap() - 5.0).abs() < 1e-12);
    assert!((nc.low(7.0).unwrap() - 8.0).abs() < 1e-12);
    assert!((nc.low(15.0).unwrap() - 43.0).abs() < 1e-12);
    assert!(nc.low(20.5).is_none());
    // The cone's steps contain it: each step as wide as the cone at its top.
    let cyl = nc.cylinders();
    assert_eq!(cyl.len(), 2 + crate::holder::CONE_STEPS);
    for (r, z0) in &cyl[2..] {
        let cone_r_at_top = 10.0 + (z0 - 38.0 + 10.0 / 8.0) * 1.0;
        assert!((r - cone_r_at_top).abs() < 1e-9, "{r} vs {cone_r_at_top}");
    }
    // No holder: nothing to check. A stickout shorter than the flutes is refused.
    let mut bare = t.clone();
    bare.holder = None;
    assert!(NonCutting::of(&bare).unwrap().is_none());
    if let Some(h) = t.holder.as_mut() {
        h.stickout = 2.0;
    }
    assert!(NonCutting::of(&t).is_err());
}

/// A floor (0..60 x 0..40, top z = 5) with a wall on it (x 40..50, top z = 20).
fn floor_and_wall() -> Vec<Body> {
    vec![
        cuboid(0.0, 0.0, 0.0, 60.0, 40.0, 5.0),
        cuboid(40.0, 0.0, 0.0, 10.0, 40.0, 20.0),
    ]
}

#[test]
fn parallel_passes_lift_or_break_where_the_holder_would_hit_a_wall() {
    // Ball R = 3, holder r = 10 whose face is 8 above the tip. Over the floor (z = 5)
    // with its axis less than 10 from the wall's face (x > 30) the holder's face would be
    // at 13, under the wall's top (20): the tip must be at least 20 - 8 = 12 there.
    // Farther off (x < 30) it clears. Next to the wall (x > 37) the ball itself rides up
    // the wall's edge to at least 17, so the holder clears there too.
    let bodies = floor_and_wall();
    let m = DropModel::new(&bodies).unwrap();
    let tool = held_tool(ToolKind::BallEndMill);
    let nc = NonCutting::of(&tool).unwrap().unwrap();
    let check = PartCheck::new(&nc, 0.0).unwrap();
    let cl = Cl::new(&m, ToolShape::new(3.0, 3.0).unwrap(), 0.0, 0.0).unwrap();
    let area = Area {
        min: p2(5.0, 18.0),
        max: p2(36.0, 22.0),
    };
    let run = |mode| {
        ops3d::parallel_with(
            &cl,
            &cutter(6.0),
            area,
            2.0,
            0.0,
            0.01,
            40.0,
            Some(HolderUse {
                check: &check,
                mode,
            }),
        )
        .unwrap()
    };
    let plain =
        feed_points(&ops3d::parallel(&cl, &cutter(6.0), area, 2.0, 0.0, 0.01, 40.0).unwrap());
    assert!(plain.iter().all(|p| (p.z - 5.0).abs() < 1e-9));
    let lifted = feed_points(&run(HolderCheck::Lift));
    assert!(lifted.iter().any(|p| p.x > 31.0));
    for p in &lifted {
        if p.x < 29.99 {
            assert!((p.z - 5.0).abs() < 1e-9, "{p:?}");
        } else if p.x > 30.01 {
            assert!((p.z - 12.0).abs() < 1e-9, "{p:?}");
        }
    }
    let refused = feed_points(&run(HolderCheck::Refuse));
    assert!(!refused.is_empty());
    for p in &refused {
        assert!(p.x <= 30.0 + 1e-9, "{p:?}");
        assert!((p.z - 5.0).abs() < 1e-9);
    }
    // Ignore leaves the plain passes.
    assert_eq!(feed_points(&run(HolderCheck::Ignore)), plain);
}

#[test]
fn a_shank_plunged_deeper_than_its_flutes_collides_in_the_simulation() {
    // Stock 0..20 x 0..20, top z = 10. A flat tool (r 3, flutes 5, shank r 3 to 8, holder
    // r 10 from 8) plunges at (10, 10) to z = 1: 9 mm into the stock, deeper than its
    // 5 mm of flutes. Before the plunge cuts, the columns within 3 of the axis stand at
    // 10, above the shank's bottom at 1 + 5 = 6 (4 mm into it), and every column within 10
    // stands above the holder's face at 1 + 8 = 9 (1 mm into it): the deepest is 4 mm.
    let tool = held_tool(ToolKind::FlatEndMill);
    let nc = NonCutting::of(&tool).unwrap().unwrap();
    let mut d = Dexels::new(
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(20.0, 20.0, 10.0),
        100,
    )
    .unwrap();
    let part = vec![None; d.nx * d.ny];
    let tp = Toolpath {
        tool_number: 1,
        tool_name: "t".into(),
        spindle_rpm: 1.0,
        moves: vec![
            Move::Rapid {
                to: Point3::new(10.0, 10.0, 12.0),
            },
            Move::Plunge {
                to: Point3::new(10.0, 10.0, 1.0),
                feed: 100.0,
            },
        ],
    };
    let flat = ToolShape::new(3.0, 0.0).unwrap();
    let mut hits = HolderHits::new(&d);
    d.cut_toolpath_checked(&flat, Some((&nc, &part, 0.02, &mut hits)), &tp, 5000.0, 1.0);
    assert_eq!(hits.moves, 1);
    assert!((hits.max_depth - 4.0).abs() < 1e-9, "{}", hits.max_depth);
    // The touched columns are those within the holder's radius (10): about π 10² mm² of
    // 0.2 mm cells, so about 7854 of them.
    let area = hits.cells() as f64 * d.cell * d.cell;
    assert!((area - std::f64::consts::PI * 100.0).abs() < 4.0, "{area}");
    // A plunge to z = 6 (only the flutes in the stock, the holder's face at 14) is clean.
    let mut d2 = Dexels::new(
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(20.0, 20.0, 10.0),
        100,
    )
    .unwrap();
    let mut shallow = tp.clone();
    shallow.moves[1] = Move::Plunge {
        to: Point3::new(10.0, 10.0, 6.0),
        feed: 100.0,
    };
    let mut hits2 = HolderHits::new(&d2);
    d2.cut_toolpath_checked(
        &flat,
        Some((&nc, &part, 0.02, &mut hits2)),
        &shallow,
        5000.0,
        1.0,
    );
    assert_eq!(hits2.moves, 0);
    assert_eq!(hits2.cells(), 0);
    // Its holder-hit columns come out as their own mesh.
    let (_, _, held) = sim::meshes_with(&d, &part, 0.02, 256, &hits.hit);
    assert!(!held.triangles.is_empty());
}

#[test]
fn a_pencil_pass_runs_along_the_foot_of_a_wall_at_the_tool_radius() {
    // A ball R = 3 on the floor (z = 5) beside the wall's faces x = 40 and x = 50 touches
    // both the floor and the wall's top edge when its axis is 3 from the face: the tip at
    // (37, y, 5) and (53, y, 5). Those are the only concave corners (the part's outer
    // edges drop to the floor below the model, which is not machined).
    let bodies = floor_and_wall();
    let m = DropModel::new(&bodies).unwrap();
    let cl = Cl::new(&m, ToolShape::new(3.0, 3.0).unwrap(), 0.0, 0.0).unwrap();
    let area = Area {
        min: p2(-3.0, -3.0),
        max: p2(63.0, 43.0),
    };
    let tp =
        crate::pencil::pencil(&cl, &cutter(6.0), area, 0.01, 10f64.to_radians(), 40.0).unwrap();
    // Every feed move, as (from, to).
    let mut segs = Vec::new();
    let mut at: Option<Point3> = None;
    for m in &tp.moves {
        match m {
            Move::Rapid { to } => at = Some(*to),
            Move::Line { to, .. } | Move::Plunge { to, .. } => {
                if let Some(a) = at {
                    segs.push((a, *to));
                }
                at = Some(*to);
            }
            _ => {}
        }
    }
    assert!(!segs.is_empty());
    // Along the corners (away from the part's ends, where the tool also meets the
    // ends' edges), every point of every cut is on one of the two lines; together they
    // cover both lines from y = 4 to 36.
    let mut covered = [0.0f64; 2];
    for (a, b) in &segs {
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let p = *a + (*b - *a) * t;
            if !(4.0..=36.0).contains(&p.y) || p.z < 1.0 {
                continue;
            }
            assert!((p.z - 5.0).abs() < 0.01, "{p:?}");
            assert!(
                (p.x - 37.0).abs() < 0.01 || (p.x - 53.0).abs() < 0.01,
                "a pencil point off the corners: {p:?}"
            );
        }
        for (k, x) in [37.0, 53.0].iter().enumerate() {
            if (a.x - x).abs() < 0.01 && (b.x - x).abs() < 0.01 {
                let lo = a.y.min(b.y).max(4.0);
                let hi = a.y.max(b.y).min(36.0);
                covered[k] += (hi - lo).max(0.0);
            }
        }
    }
    for c in covered {
        assert!((c - 32.0).abs() < 1e-6, "{covered:?}");
    }
}

/// A ramp rising at `angle` from z = 0 at x = 0 to x = 30, from y = -80 to y = 0, with a
/// vertical back at x = 30.
fn ramp(angle: f64) -> Body {
    let frame = Frame::from_normal_and_x(
        Point3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .unwrap();
    let h = 30.0 * angle.tan();
    let profile = Profile::new(
        vec![
            ProfileEdge::line(p2(0.0, 0.0), p2(30.0, 0.0)),
            ProfileEdge::line(p2(30.0, 0.0), p2(30.0, h)),
            ProfileEdge::line(p2(30.0, h), p2(0.0, 0.0)),
        ],
        Vec::new(),
    );
    chamfera_kernel::extrude(&frame, &profile, 80.0).unwrap()
}

/// Where the toolpath's feed moves cross the line y = `y`, sorted.
fn crossings(tp: &Toolpath, y: f64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut at: Option<Point3> = None;
    for m in &tp.moves {
        let to = match m {
            Move::Rapid { to } => {
                at = Some(*to);
                continue;
            }
            Move::Line { to, .. } | Move::Plunge { to, .. } => *to,
            _ => continue,
        };
        if let Some(a) = at
            && (a.y - y) * (to.y - y) < 0.0
        {
            let t = (y - a.y) / (to.y - a.y);
            out.push(a.x + (to.x - a.x) * t);
        }
        at = Some(to);
    }
    out.sort_by(f64::total_cmp);
    out
}

#[test]
fn scallop_passes_are_a_stepover_apart_along_a_slope() {
    // On a plane rising at 30 degrees, passes a constant 3 mm apart along the surface are
    // 3 cos 30° = 2.598 mm apart in plan, across the slope. Far from the ramp's sides
    // (half way along its 80 mm), the passes from its low edge are lines across X; the
    // distance along the surface grows exactly linearly along X there (the grid steps
    // along X are exact chords of the plane), so their crossings of y = -40.3 are 2.598
    // apart. The first pass (half a cell in from the border) is not a stepover from the
    // next, so the spacing is checked from the second pass on.
    let a = 30f64.to_radians();
    let m = DropModel::new(&[ramp(a)]).unwrap();
    let cl = Cl::new(&m, ToolShape::new(3.0, 3.0).unwrap(), 0.0, 0.0).unwrap();
    let area = Area {
        min: p2(-3.0, -83.0),
        max: p2(33.0, 3.0),
    };
    let tp = crate::scallop::scallop(&cl, &cutter(6.0), area, 3.0, 0.01, 60f64.to_radians(), 40.0)
        .unwrap();
    let xs: Vec<f64> = crossings(&tp, -40.3)
        .into_iter()
        .filter(|x| *x < 12.0)
        .collect();
    assert!(xs.len() >= 4, "{xs:?}");
    let want = 3.0 * a.cos();
    for w in xs[1..4].windows(2) {
        assert!((w[1] - w[0] - want).abs() < 0.01 * want, "{xs:?}");
    }
    // Every point is on the part (dropped exactly), none in the air beside it.
    assert!(feed_points(&tp).iter().all(|p| p.z > 0.0));
}

#[test]
fn scallop_refuses_what_it_cannot_do() {
    // Off the part everywhere: nothing to machine.
    let m = DropModel::new(&[cuboid(0.0, 0.0, 0.0, 10.0, 10.0, 5.0)]).unwrap();
    let cl = Cl::new(&m, ToolShape::new(3.0, 3.0).unwrap(), 0.0, 0.0).unwrap();
    let away = Area {
        min: p2(100.0, 100.0),
        max: p2(120.0, 120.0),
    };
    assert!(matches!(
        crate::scallop::scallop(&cl, &cutter(6.0), away, 1.0, 0.01, 1.0, 40.0),
        Err(CamError::Refused(_))
    ));
    // Many separate islands (more than MAX_REGIONS) are too fragmented.
    let mut pins = Vec::new();
    for i in 0..9 {
        for j in 0..9 {
            pins.push(cuboid(
                f64::from(i) * 12.0,
                f64::from(j) * 12.0,
                0.0,
                4.0,
                4.0,
                10.0,
            ));
        }
    }
    let m = DropModel::new(&pins).unwrap();
    let cl = Cl::new(&m, ToolShape::new(0.5, 0.5).unwrap(), 0.0, 0.0).unwrap();
    let area = Area {
        min: p2(-1.0, -1.0),
        max: p2(105.0, 105.0),
    };
    assert!(matches!(
        crate::scallop::scallop(&cl, &cutter(1.0), area, 0.5, 0.01, 1.0, 40.0),
        Err(CamError::Refused(_))
    ));
}

#[test]
fn a_job_whose_generation_is_no_longer_wanted_stops() {
    let m = DropModel::new(&floor_and_wall()).unwrap();
    let cl = Cl::new(&m, ToolShape::new(3.0, 3.0).unwrap(), 0.0, 0.0).unwrap();
    let area = Area {
        min: p2(0.0, 0.0),
        max: p2(60.0, 40.0),
    };
    let latest = Arc::new(AtomicU64::new(2));
    let stale = crate::cancel::Token {
        latest: latest.clone(),
        generation: 1,
    };
    let r = crate::cancel::with(stale, || {
        ops3d::parallel(&cl, &cutter(6.0), area, 2.0, 0.0, 0.01, 40.0)
    });
    assert_eq!(r, Err(CamError::Cancelled));
    let current = crate::cancel::Token {
        latest,
        generation: 2,
    };
    let ok = crate::cancel::with(current, || {
        ops3d::parallel(&cl, &cutter(6.0), area, 2.0, 0.0, 0.01, 40.0)
    });
    // Outside any job (no token) it is never cancelled, and gives the same toolpath.
    assert_eq!(
        ok,
        ops3d::parallel(&cl, &cutter(6.0), area, 2.0, 0.0, 0.01, 40.0)
    );
    assert!(crate::cancel::check().is_ok());
}

#[test]
fn hostile_inputs_to_holders_pencil_and_scallop_never_panic() {
    let m = DropModel::new(&floor_and_wall()).unwrap();
    let cl = Cl::new(&m, ToolShape::new(3.0, 3.0).unwrap(), 0.0, 0.0).unwrap();
    let area = Area {
        min: p2(0.0, 0.0),
        max: p2(60.0, 40.0),
    };
    for v in [
        f64::NAN,
        f64::INFINITY,
        -f64::INFINITY,
        1e300,
        -1e300,
        0.0,
        -1.0,
    ] {
        let _ = crate::pencil::pencil(&cl, &cutter(6.0), area, v, 0.2, 40.0);
        let _ = crate::pencil::pencil(&cl, &cutter(6.0), area, 0.01, v, 40.0);
        let _ = crate::scallop::scallop(&cl, &cutter(6.0), area, v, 0.01, 1.0, 40.0);
        let _ = crate::scallop::scallop(&cl, &cutter(6.0), area, 1.0, v, 1.0, 40.0);
        let _ = crate::scallop::scallop(&cl, &cutter(6.0), area, 1.0, 0.01, v, 40.0);
        let mut t = held_tool(ToolKind::FlatEndMill);
        if let Some(h) = t.holder.as_mut() {
            h.stickout = v;
            h.shank_diameter = Some(v);
            h.segments.push(HolderSegment {
                diameter: v,
                top_diameter: Some(v),
                length: v,
            });
        }
        if let Ok(Some(nc)) = NonCutting::of(&t) {
            let _ = nc.low(v);
            if let Ok(c) = PartCheck::new(&nc, v) {
                let _ = c.required(&m, v, v);
            }
        }
        let tool = held_tool(ToolKind::FlatEndMill);
        let nc = NonCutting::of(&tool).unwrap().unwrap();
        let mut d = Dexels::new(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(10.0, 10.0, 10.0),
            10,
        )
        .unwrap();
        let part = vec![Some(v); d.nx * d.ny];
        let mut hits = HolderHits::new(&d);
        let tp = Toolpath {
            tool_number: 1,
            tool_name: "t".into(),
            spindle_rpm: 1.0,
            moves: vec![Move::Line {
                to: Point3::new(v, 1.0, v),
                feed: v,
            }],
        };
        d.cut_toolpath_checked(
            &ToolShape::new(3.0, 0.0).unwrap(),
            Some((&nc, &part, v, &mut hits)),
            &tp,
            v,
            v,
        );
    }
}
