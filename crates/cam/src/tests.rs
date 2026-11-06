//! Toolpaths measured against values derived by hand (see each test's comment).

use crate::ops::{self, Cutter, Region};
use crate::path2::{Loop, Seg, distance_to, winding};
use crate::post::{Dialect, Wcs, post};
use crate::toolpath::Move;
use crate::{CamError, model};
use chamfera_doc::cam::{Entry, Side};
use chamfera_geom::{Frame, Point2, Point3};
use chamfera_kernel::{Body, Profile, ProfileEdge};
use std::f64::consts::TAU;

fn p(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Loop {
    vec![
        Seg::line(p(x0, y0), p(x1, y0)),
        Seg::line(p(x1, y0), p(x1, y1)),
        Seg::line(p(x1, y1), p(x0, y1)),
        Seg::line(p(x0, y1), p(x0, y0)),
    ]
}

fn cutter(d: f64) -> Cutter {
    Cutter {
        number: 1,
        name: format!("{d} mm flat end mill"),
        diameter: d,
        spindle_rpm: 10_000.0,
        feed: 600.0,
        plunge_feed: 200.0,
    }
}

/// Points along every feed move, on the exact arcs (not their chords).
fn feed_points(tp: &crate::Toolpath) -> Vec<Point3> {
    let mut out = Vec::new();
    let Some(mut at) = tp.start() else {
        return out;
    };
    for m in tp.expanded() {
        match m {
            Move::Rapid { to } => at = to,
            Move::Line { to, .. } | Move::Plunge { to, .. } => {
                for i in 0..=20 {
                    let t = f64::from(i) / 20.0;
                    out.push(Point3::new(
                        at.x + (to.x - at.x) * t,
                        at.y + (to.y - at.y) * t,
                        at.z + (to.z - at.z) * t,
                    ));
                }
                at = to;
            }
            Move::Arc {
                to,
                centre,
                clockwise,
                ..
            } => {
                let s = Seg::arc_between(centre, p(at.x, at.y), p(to.x, to.y), !clockwise);
                for i in 0..=200 {
                    let t = f64::from(i) / 200.0;
                    let q = s.point_at(t);
                    out.push(Point3::new(q.x, q.y, at.z + (to.z - at.z) * t));
                }
                at = to;
            }
            Move::Drill { .. } => {}
        }
    }
    out
}

#[test]
fn facing_a_50_by_30_stock_with_a_10_mm_tool_at_half_stepover() {
    // Stepover 5 over a 30 wide stock: ceil(30 / 5) = 6 steps, so 7 passes. Each pass
    // runs from r + 1 before the stock to r + 1 past it: 50 + 2 (5 + 1) = 62. Between
    // passes a 5 mm step along Y. Cut length: 7 * 62 + 6 * 5 = 464.
    let tp = ops::facing(
        p(0.0, 0.0),
        p(50.0, 30.0),
        2.0,
        0.0,
        &cutter(10.0),
        0.5,
        5.0,
        12.0,
    )
    .unwrap();
    let mut at = tp.start().unwrap();
    let (mut passes, mut along_x, mut along_y) = (0, 0.0, 0.0);
    for m in &tp.moves {
        match *m {
            Move::Line { to, .. } => {
                if (to.y - at.y).abs() < 1e-12 {
                    passes += 1;
                    along_x += (to.x - at.x).abs();
                } else {
                    along_y += (to.y - at.y).abs();
                }
                assert!(to.z.abs() < 1e-12, "one depth: 2 mm in one stepdown of 5");
                at = to;
            }
            Move::Rapid { to } | Move::Plunge { to, .. } => at = to,
            _ => panic!("facing has only lines, rapids and plunges"),
        }
    }
    assert_eq!(passes, 7);
    assert!((along_x - 434.0).abs() < 1e-9);
    assert!((along_y - 30.0).abs() < 1e-9);
    // Feed length also counts the plunge from 1 above the top (2 + 1) down to 0: 3 mm.
    let s = tp.stats(5000.0);
    assert!((s.feed_length - 467.0).abs() < 1e-9, "{}", s.feed_length);
}

#[test]
fn a_contour_round_a_50_by_30_rectangle_has_radius_3_corners_and_length_160_plus_6_pi() {
    // A 6 mm tool outside the rectangle: the path is 3 from it, straight along the sides
    // and quarter circles of radius 3 at the corners: 2 (50 + 30) + 2π·3.
    let tp = ops::contour(
        &rect(0.0, 0.0, 50.0, 30.0),
        -5.0,
        Side::Right,
        true,
        &cutter(6.0),
        0.0,
        10.0,
        0.0,
        0.0,
        10.0,
    )
    .unwrap();
    let mut at = tp.start().unwrap();
    let mut length = 0.0;
    let mut arcs = 0;
    for m in &tp.moves {
        match *m {
            Move::Line { to, .. } => {
                length += (to - at).length();
                at = to;
            }
            Move::Arc {
                to,
                centre,
                clockwise,
                ..
            } => {
                let r = Point2::new(at.x, at.y).distance(centre);
                assert!((r - 3.0).abs() < 1e-9);
                // Climb milling outside: clockwise round the part.
                assert!(clockwise);
                length += crate::toolpath::arc_length(at, to, centre, clockwise);
                arcs += 1;
                at = to;
            }
            Move::Rapid { to } | Move::Plunge { to, .. } => at = to,
            Move::Drill { .. } => panic!("no drilling in a contour"),
        }
    }
    assert_eq!(arcs, 4);
    let expected = 160.0 + TAU * 3.0;
    assert!((length - expected).abs() < 1e-9, "{length} vs {expected}");
}

#[test]
fn a_40_by_20_pocket_with_a_6_mm_tool_stays_3_from_the_walls_and_ends_on_the_wall_offset() {
    let region = Region {
        outer: rect(0.0, 0.0, 40.0, 20.0),
        islands: Vec::new(),
    };
    let tp = ops::pocket(
        &region,
        -6.0,
        0.0,
        &cutter(6.0),
        0.5,
        3.0,
        Entry::Helix,
        0.05,
        1.5,
        10.0,
    )
    .unwrap();
    let walls = [region.outer.clone()];
    let mut cut_below_top = 0;
    for q in feed_points(&tp) {
        if q.z < -1e-9 {
            cut_below_top += 1;
            let d = distance_to(&walls, p(q.x, q.y));
            assert!(d >= 3.0 - 1e-6, "{q:?} is {d} from the walls");
            assert!(
                winding(&walls[0], p(q.x, q.y)) == 1,
                "{q:?} left the pocket"
            );
        }
    }
    assert!(cut_below_top > 0);
    // Passes 3, 6 and 9 from the walls (12 would leave nothing: the pocket is 10 from its
    // centre line). The last full loop at the floor is the wall offset: the rectangle
    // (3, 3)-(37, 17), perimeter 2 (34 + 14) = 96.
    let passes = ops::pocket_passes(&region, 3.0, 3.0).unwrap();
    let ds: Vec<f64> = passes.iter().map(|x| x.0).collect();
    assert_eq!(ds, vec![9.0, 6.0, 3.0]);
    let last = tp
        .moves
        .iter()
        .rev()
        .filter_map(|m| match m {
            Move::Line { to, .. } => Some(*to),
            _ => None,
        })
        .take(4)
        .collect::<Vec<_>>();
    for q in last {
        assert!((q.z + 6.0).abs() < 1e-12);
        let on_offset = (q.x - 3.0).abs() < 1e-9
            || (q.x - 37.0).abs() < 1e-9
            || (q.y - 3.0).abs() < 1e-9
            || (q.y - 17.0).abs() < 1e-9;
        assert!(on_offset, "{q:?} is not on the wall offset");
    }
}

#[test]
fn a_pocket_with_a_round_island_keeps_the_tool_clear_of_it() {
    // An island of radius 5 at (30, 15) in a 60 x 30 pocket; a 6 mm tool must keep its
    // centre at least 5 + 3 = 8 from the island's centre at every cutting depth.
    let island = vec![Seg::Arc {
        c: p(30.0, 15.0),
        r: 5.0,
        start: 0.0,
        sweep: TAU,
    }];
    let region = Region {
        outer: rect(0.0, 0.0, 60.0, 30.0),
        islands: vec![island],
    };
    for entry in [Entry::Helix, Entry::Ramp, Entry::Plunge] {
        let tp = ops::pocket(
            &region,
            -4.0,
            0.0,
            &cutter(6.0),
            0.4,
            2.0,
            entry,
            0.05,
            2.0,
            10.0,
        )
        .unwrap();
        let mut n = 0;
        for q in feed_points(&tp) {
            if q.z < -1e-9 {
                n += 1;
                let d = p(q.x, q.y).distance(p(30.0, 15.0));
                assert!(
                    d >= 8.0 - 1e-6,
                    "{entry:?}: {q:?} is {d} from the island centre"
                );
                assert!(
                    distance_to(std::slice::from_ref(&region.outer), p(q.x, q.y)) >= 3.0 - 1e-6
                );
            }
        }
        assert!(n > 0);
    }
}

/// A 60 x 40 x 10 plate with a 40 x 20 pocket 5 deep (from x 10, y 10) and two
/// through holes of diameter 6 at (5, 5) and (55, 35).
fn pocketed_plate() -> Body {
    let plate = Profile::polygon(&[p(0.0, 0.0), p(60.0, 0.0), p(60.0, 40.0), p(0.0, 40.0)])
        .with_hole(vec![ProfileEdge::circle(p(5.0, 5.0), 3.0)])
        .with_hole(vec![ProfileEdge::circle(p(55.0, 35.0), 3.0)]);
    let body = chamfera_kernel::extrude(&Frame::XY, &plate, 10.0).unwrap();
    let cutter_frame =
        Frame::from_normal(Point3::new(0.0, 0.0, 5.0), chamfera_geom::Vec3::Z).unwrap();
    let pocket = Profile::polygon(&[p(10.0, 10.0), p(50.0, 10.0), p(50.0, 30.0), p(10.0, 30.0)]);
    let tool = chamfera_kernel::extrude(&cutter_frame, &pocket, 10.0).unwrap();
    chamfera_kernel::subtract(&body, &tool).unwrap()
}

fn faces_at(body: &Body, z: f64, up: bool) -> Vec<chamfera_kernel::FaceId> {
    body.faces()
        .into_iter()
        .filter(|f| {
            model::horizontal_face_z(body, *f).is_ok_and(|(fz, u)| (fz - z).abs() < 1e-9 && u == up)
        })
        .collect()
}

#[test]
fn the_pocket_region_is_read_from_its_floor_and_walls() {
    let body = pocketed_plate();
    let floor = faces_at(&body, 5.0, true);
    assert_eq!(floor.len(), 1);
    let pk = model::pocket_region(&body, floor[0]).unwrap();
    assert!((pk.floor_z - 5.0).abs() < 1e-12);
    assert!((pk.top_z - 10.0).abs() < 1e-9);
    assert!((crate::path2::signed_area(&pk.region.outer) - 800.0).abs() < 1e-9);
    assert!(pk.region.islands.is_empty());
    // The plate's top is open all round (its walls go down): not a pocket.
    let top = faces_at(&body, 10.0, true);
    assert!(matches!(
        model::pocket_region(&body, top[0]),
        Err(CamError::Refused(_))
    ));
}

#[test]
fn drilling_finds_both_vertical_holes_of_the_plate() {
    let body = pocketed_plate();
    let mut holes: Vec<_> = model::find_holes(&body, None)
        .unwrap()
        .into_iter()
        .map(|h| h.0)
        .collect();
    holes.sort_by(|a, b| a.centre.x.total_cmp(&b.centre.x));
    assert_eq!(holes.len(), 2);
    assert!(holes[0].centre.distance(p(5.0, 5.0)) < 1e-9);
    assert!(holes[1].centre.distance(p(55.0, 35.0)) < 1e-9);
    for h in &holes {
        assert!((h.diameter - 6.0).abs() < 1e-9);
        assert!((h.top - 10.0).abs() < 1e-9 && h.bottom.abs() < 1e-9);
    }
    // An 8 mm drill is refused for 6 mm holes.
    let err = ops::drill(&holes, &cutter(8.0), 12.0, None, 0.0, 20.0).unwrap_err();
    assert!(matches!(err, CamError::Refused(_)));
}

#[test]
fn a_face_facing_down_is_no_pocket_floor() {
    let body = pocketed_plate();
    let bottom = faces_at(&body, 0.0, false);
    assert!(model::pocket_region(&body, bottom[0]).is_err());
}

#[test]
fn the_post_for_a_square_contour_matches_hand_written_g_code() {
    // 20 x 20 square, a 6 mm tool outside it, conventional (counter-clockwise), one depth
    // at -2 from a stock top at 0, clearance 10, no lead arcs. The path starts at
    // (0, -3), runs along the sides 3 out, with G3 quarter circles round each corner.
    let tp = ops::contour(
        &rect(0.0, 0.0, 20.0, 20.0),
        -2.0,
        Side::Right,
        false,
        &cutter(6.0),
        0.0,
        5.0,
        0.0,
        0.0,
        10.0,
    )
    .unwrap();
    let text = post(
        "square",
        &[("Contour".into(), tp.clone())],
        &Wcs::default(),
        Dialect::LinuxCnc,
    );
    let expected = "\
(square)
G21 G90 G17
(Contour)
T1 M6
S10000 M3
G0 Z10
G0 X0 Y-3
G0 Z1
G1 Z-2 F200
G1 X20 F600
G3 X23 Y0 I0 J3
G1 Y20
G3 X20 Y23 I-3 J0
G1 X0
G3 X-3 Y20 I0 J-3
G1 Y0
G3 X0 Y-3 I3 J0
G0 Z10
M5
M30
";
    assert_eq!(text, expected);
    // GRBL: the same moves, with the tool named in a comment instead of an M6.
    let grbl = post(
        "square",
        &[("Contour".into(), tp)],
        &Wcs::default(),
        Dialect::Grbl,
    );
    assert_eq!(grbl, expected.replace("T1 M6", "(T1 6 mm flat end mill)"));
}

#[test]
fn grbl_gets_no_canned_cycles_and_no_tool_changes() {
    let holes = [
        ops::Hole {
            centre: p(0.0, 0.0),
            diameter: 6.0,
            top: 0.0,
            bottom: -10.0,
        },
        ops::Hole {
            centre: p(20.0, 0.0),
            diameter: 6.0,
            top: 0.0,
            bottom: -10.0,
        },
    ];
    let a = ops::drill(&holes, &cutter(5.0), 2.0, Some(3.0), 0.5, 10.0).unwrap();
    let mut b = ops::drill(&holes, &cutter(5.0), 2.0, None, 0.5, 10.0).unwrap();
    b.tool_number = 2;
    let ops = vec![("Peck".to_owned(), a), ("Drill".to_owned(), b)];
    let linuxcnc = post("holes", &ops, &Wcs::default(), Dialect::LinuxCnc);
    assert!(
        linuxcnc.contains("G98 G83 X0 Y0 Z-10.5 R2 Q3 F200"),
        "{linuxcnc}"
    );
    assert!(linuxcnc.contains("G98 G81 X20 Y0 Z-10.5 R2"), "{linuxcnc}");
    assert!(linuxcnc.contains("T2 M6") && linuxcnc.contains("G80"));
    let grbl = post("holes", &ops, &Wcs::default(), Dialect::Grbl);
    for word in ["G81", "G83", "G80", "M6", "G98"] {
        assert!(!grbl.contains(word), "{word} in\n{grbl}");
    }
    // The pecks are written out: 10.5 deep from R 2 in pecks of 3 is ceil(12.5 / 3) = 5
    // strokes per hole, two holes; the plain cycle is one stroke per hole: 2 * 5 + 2.
    let strokes = grbl.lines().filter(|l| l.starts_with("G1 Z")).count();
    assert_eq!(strokes, 12, "{grbl}");
}

#[test]
fn hostile_numbers_are_refused_without_panicking() {
    let bad = [0.0, -1.0, f64::NAN, f64::INFINITY, 1e300, f64::MIN_POSITIVE];
    let sq = rect(0.0, 0.0, 10.0, 10.0);
    let region = Region {
        outer: sq.clone(),
        islands: Vec::new(),
    };
    for v in bad {
        let c = cutter(v);
        let _ = ops::facing(p(0.0, 0.0), p(10.0, 10.0), 1.0, 0.0, &c, 0.5, 1.0, 5.0);
        let _ = ops::facing(p(0.0, 0.0), p(10.0, 10.0), v, 0.0, &cutter(3.0), v, v, v);
        let _ = ops::contour(&sq, 0.0, Side::Left, true, &c, 1.0, 1.0, 1.0, 0.0, 5.0);
        let _ = ops::contour(&sq, v, Side::Right, false, &cutter(3.0), v, v, v, v, v);
        let _ = ops::pocket(
            &region,
            -1.0,
            0.0,
            &c,
            0.5,
            1.0,
            Entry::Helix,
            0.1,
            1.0,
            5.0,
        );
        let _ = ops::pocket(&region, v, v, &cutter(2.0), v, v, Entry::Ramp, v, v, v);
        let _ = ops::drill(&[], &c, v, Some(v), v, v);
        let _ = crate::offset::offset_left(std::slice::from_ref(&sq), v);
        let _ = ops::depths(v, -v, v);
    }
    // Empty and degenerate loops.
    let _ = ops::contour(
        &[],
        0.0,
        Side::Left,
        true,
        &cutter(3.0),
        1.0,
        1.0,
        0.0,
        0.0,
        5.0,
    );
    let dot = vec![Seg::line(p(1.0, 1.0), p(1.0, 1.0))];
    let _ = ops::pocket(
        &Region {
            outer: dot,
            islands: Vec::new(),
        },
        -1.0,
        0.0,
        &cutter(2.0),
        0.5,
        1.0,
        Entry::Helix,
        0.1,
        1.0,
        5.0,
    );
    // A stepover far too small is refused, not ground through.
    let r = ops::pocket(
        &region,
        -1.0,
        0.0,
        &cutter(2.0),
        1e-9,
        1.0,
        Entry::Plunge,
        0.1,
        1.0,
        5.0,
    );
    assert!(r.is_err());
}
