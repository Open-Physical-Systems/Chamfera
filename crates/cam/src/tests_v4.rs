//! Holding tabs, tapping and posts as data, measured against values derived by hand
//! (see each test's comment) and G-code read back line by line.

use crate::ops::{self, Cutter, Hole};
use crate::path2::{Loop, Seg};
use crate::post::{
    ArcStyle, Coolant, LineNumbers, OutUnits, PostDef, PostOptions, Wcs, find_post, post_with,
};
use crate::toolpath::{Move, Toolpath};
use chamfera_doc::cam::{Side, TabShape, Tabs};
use chamfera_geom::{Point2, Point3};
use std::f64::consts::{FRAC_PI_4, PI};

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

fn cutter(d: f64, rpm: f64) -> Cutter {
    Cutter {
        number: 1,
        name: format!("{d} mm tool"),
        diameter: d,
        spindle_rpm: rpm,
        feed: 600.0,
        plunge_feed: 200.0,
    }
}

fn tabs(shape: TabShape, ramp_deg: f64) -> Tabs {
    Tabs {
        count: Some(4),
        spacing: None,
        width: 8.0,
        height: 3.0,
        shape,
        ramp_angle: ramp_deg.to_radians(),
    }
}

/// A 100 x 60 plate outline, a 10 mm tool outside it, conventional milling (the path
/// runs counter-clockwise from (0, -5)), cut from the stock top at 0 to -6 in passes of
/// 2 (at -2, -4 and -6), no lead arcs, with `t`.
fn plate(t: &Tabs) -> Result<Toolpath, crate::CamError> {
    ops::contour_with_tabs(
        &rect(0.0, 0.0, 100.0, 60.0),
        -6.0,
        Side::Right,
        false,
        &cutter(10.0, 10_000.0),
        0.0,
        2.0,
        0.0,
        0.0,
        10.0,
        Some(t),
    )
}

/// Each feed move as (from, to).
fn feeds(tp: &Toolpath) -> Vec<(Point3, Point3)> {
    let mut out = Vec::new();
    let mut at = tp.start().unwrap();
    for m in tp.expanded() {
        match m {
            Move::Rapid { to } => at = to,
            Move::Line { to, .. } | Move::Plunge { to, .. } | Move::Arc { to, .. } => {
                out.push((at, to));
                at = to;
            }
            Move::Drill { .. } | Move::Tap { .. } => {}
        }
    }
    out
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// The tab centres on the plate's path, by hand. The path is 100 + 60 + 100 + 60 of
/// straight sides 5 out plus four quarter circles of radius 5: T = 320 + 10π. Four tabs
/// spread evenly sit at T/8, 3T/8, 5T/8 and 7T/8 along it, which is 40 + 1.25π along each
/// side from its start (every side starts after one more corner of 2.5π): on the bottom
/// side (y = -5) at x = 40 + 1.25π, on the right (x = 105) at y = 20 + 1.25π, on the top
/// (y = 65, run towards -X) at x = 60 - 1.25π, on the left (x = -5, run towards -Y) at
/// y = 40 - 1.25π. Each footprint is at most 12 long either side, inside its side.
fn hand_centres() -> [(f64, f64); 4] {
    let k = 1.25 * PI;
    [
        (40.0 + k, -5.0),
        (105.0, 20.0 + k),
        (60.0 - k, 65.0),
        (-5.0, 40.0 - k),
    ]
}

/// The runs of the path at height `z` that move in XY, as (from, to).
fn runs_at(tp: &Toolpath, z: f64) -> Vec<(Point3, Point3)> {
    feeds(tp)
        .into_iter()
        .filter(|(a, b)| close(a.z, z, 1e-9) && close(b.z, z, 1e-9))
        .filter(|(a, b)| (Point2::new(b.x, b.y) - Point2::new(a.x, a.y)).length() > 1e-6)
        .collect()
}

#[test]
fn rectangular_tabs_stand_where_and_as_high_as_worked_out_by_hand() {
    // Tab top: the cut's bottom -6 plus the 3 mm height = -3. The passes at -4 and -6 run
    // over each tab at -3 for the tab width plus the tool diameter, 8 + 10 = 18, centred on
    // the tab; the pass at -2 is above the tabs and never rises.
    let tp = plate(&tabs(TabShape::Rectangular, 90.0)).unwrap();
    let f = feeds(&tp);
    // The first cut starts at (0, -5): the plunge ends there.
    let (_, first) = f.first().copied().unwrap();
    assert!(close(first.x, 0.0, 1e-9) && close(first.y, -5.0, 1e-9) && close(first.z, -2.0, 1e-9));
    let top = runs_at(&tp, -3.0);
    assert_eq!(top.len(), 8, "{top:?}");
    let centres = hand_centres();
    for (i, (a, b)) in top.iter().enumerate() {
        let (cx, cy) = centres[i % 4];
        let mid = Point2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        assert!(
            close(mid.x, cx, 1e-9) && close(mid.y, cy, 1e-9),
            "tab {i}: {mid:?}"
        );
        let len = (Point2::new(b.x, b.y) - Point2::new(a.x, a.y)).length();
        assert!(close(len, 18.0, 1e-9), "tab {i}: {len}");
    }
    // Nothing is cut below the tab tops within 9 of a tab centre on the lower passes,
    // and nothing anywhere is higher than the pass or tab it belongs to.
    for (a, b) in &f {
        for t in [0.0, 0.5, 1.0] {
            let q = Point3::new(
                a.x + (b.x - a.x) * t,
                a.y + (b.y - a.y) * t,
                a.z + (b.z - a.z) * t,
            );
            for (cx, cy) in centres {
                let d = Point2::new(q.x, q.y).distance(Point2::new(cx, cy));
                if d < 9.0 - 1e-6 {
                    assert!(q.z >= -3.0 - 1e-9, "cut into a tab at {q:?}");
                }
            }
        }
    }
    // Straight up and straight down: each lower pass has 4 climbs and 4 descents.
    let verticals = f
        .iter()
        .filter(|(a, b)| close(a.x, b.x, 1e-12) && close(a.y, b.y, 1e-12) && b.z > a.z + 1e-9)
        .count();
    assert_eq!(verticals, 8);
}

#[test]
fn a_spacing_gives_the_same_tabs_as_the_count_it_works_out_to() {
    // The path is 351.4 long: a spacing of at most 100 needs ceil(3.514) = 4 tabs.
    let mut t = tabs(TabShape::Rectangular, 90.0);
    let by_count = plate(&t).unwrap();
    t.count = None;
    t.spacing = Some(100.0);
    assert_eq!(plate(&t).unwrap(), by_count);
}

#[test]
fn ramps_over_rectangular_tabs_climb_at_the_ramp_angle() {
    // 45°: on the pass at -6 the tool climbs 3 over 3 mm before the flat top (18 long) and
    // comes down over 3 after it; on the pass at -4 it climbs 1 over 1 mm.
    let tp = plate(&tabs(TabShape::Rectangular, 45.0)).unwrap();
    let f = feeds(&tp);
    let ramps: Vec<_> = f
        .iter()
        .filter(|(a, b)| (b.z - a.z).abs() > 1e-9)
        .filter(|(a, b)| (Point2::new(b.x, b.y) - Point2::new(a.x, a.y)).length() > 1e-9)
        .collect();
    // 4 tabs, 2 passes, up and down.
    assert_eq!(ramps.len(), 16);
    for (a, b) in &ramps {
        let run = (Point2::new(b.x, b.y) - Point2::new(a.x, a.y)).length();
        let rise = (b.z - a.z).abs();
        assert!(close(rise / run, FRAC_PI_4.tan(), 1e-9), "{a:?} {b:?}");
        assert!(close(rise, 3.0, 1e-9) || close(rise, 1.0, 1e-9));
    }
    for (a, b) in runs_at(&tp, -3.0) {
        assert!(close(
            (Point2::new(b.x, b.y) - Point2::new(a.x, a.y)).length(),
            18.0,
            1e-9
        ));
    }
}

#[test]
fn triangular_tabs_rise_along_their_sides() {
    // A triangle 8 wide at its foot and 3 tall: the tool centre stays at the apex height
    // while the tool still covers the apex (10 long: the tool diameter) and falls at
    // 3 / 4 per mm beyond. On the pass at -6 each side is 4 long; at -4 it is 4/3 long and
    // starts at -4.
    let tp = plate(&tabs(TabShape::Triangular, 90.0)).unwrap();
    let centres = hand_centres();
    let top = runs_at(&tp, -3.0);
    assert_eq!(top.len(), 8);
    for (i, (a, b)) in top.iter().enumerate() {
        let (cx, cy) = centres[i % 4];
        let mid = Point2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        assert!(close(mid.x, cx, 1e-9) && close(mid.y, cy, 1e-9));
        assert!(close(
            (Point2::new(b.x, b.y) - Point2::new(a.x, a.y)).length(),
            10.0,
            1e-9
        ));
    }
    let f = feeds(&tp);
    let mut sides = 0;
    for (a, b) in &f {
        let run = (Point2::new(b.x, b.y) - Point2::new(a.x, a.y)).length();
        let rise = (b.z - a.z).abs();
        if rise > 1e-9 && run > 1e-9 {
            sides += 1;
            assert!(close(rise / run, 0.75, 1e-9));
            assert!(
                close(run, 4.0, 1e-9) || close(run, 4.0 / 3.0, 1e-9),
                "{run}"
            );
        }
    }
    assert_eq!(sides, 16);
}

#[test]
fn too_many_tabs_for_the_path_are_refused() {
    // 20 tabs of footprint 18 need 360 mm; the path is 351.4.
    let mut t = tabs(TabShape::Rectangular, 90.0);
    t.count = Some(20);
    assert!(matches!(plate(&t), Err(crate::CamError::Refused(_))));
}

#[test]
fn hostile_tabs_are_refused_without_panicking() {
    let base = tabs(TabShape::Rectangular, 90.0);
    for v in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e300] {
        for t in [
            Tabs { width: v, ..base },
            Tabs { height: v, ..base },
            Tabs {
                ramp_angle: v,
                ..base
            },
            Tabs {
                count: None,
                spacing: Some(v),
                ..base
            },
        ] {
            assert!(plate(&t).is_err(), "{t:?}");
        }
    }
    for t in [
        Tabs {
            count: Some(0),
            ..base
        },
        Tabs {
            count: None,
            spacing: None,
            ..base
        },
        Tabs {
            spacing: Some(10.0),
            ..base
        },
        Tabs {
            count: Some(u32::MAX),
            ..base
        },
        Tabs {
            ramp_angle: PI,
            ..base
        },
        Tabs {
            count: None,
            spacing: Some(1e-9),
            ..base
        },
    ] {
        assert!(plate(&t).is_err(), "{t:?}");
    }
    // Tabs taller than the cut stop at the stock top (0): every pass runs over them there.
    let tall = plate(&Tabs {
        height: 50.0,
        ..base
    })
    .unwrap();
    assert_eq!(runs_at(&tall, 0.0).len(), 12);
    assert!(feeds(&tall).iter().all(|(_, b)| b.z <= 0.0 + 1e-9));
}

fn holes() -> [Hole; 2] {
    [
        Hole {
            centre: p(0.0, 0.0),
            diameter: 5.0,
            top: 0.0,
            bottom: -10.0,
        },
        Hole {
            centre: p(20.0, 0.0),
            diameter: 5.0,
            top: 0.0,
            bottom: -10.0,
        },
    ]
}

/// M6 x 1 tapping of two 5 mm holes 10 deep (0 to -10) and 0.5 past, retract plane 2,
/// clearance 10, at 500 rpm: the feed is 1 x 500 = 500 mm/min.
fn tapped() -> Toolpath {
    ops::tap(&holes(), &cutter(6.0, 500.0), 2.0, 1.0, 0.5, 10.0).unwrap()
}

fn posted(
    post: &str,
    ops: &[(String, Toolpath)],
    opts: &PostOptions,
) -> Result<String, crate::CamError> {
    post_with(
        "holes",
        ops,
        &Wcs::default(),
        &find_post(post).unwrap(),
        opts,
    )
    .map(|p| p.text)
}

#[test]
fn a_tapping_cycle_feeds_one_pitch_per_turn_in_and_out() {
    let tp = tapped();
    // Each hole: in from R 2 to -10.5 and out again, 12.5 each way, at 500 mm/min.
    let s = tp.stats(5000.0);
    assert!(close(s.feed_length, 4.0 * 12.5, 1e-9));
    let feeds: Vec<f64> = tp
        .expanded()
        .iter()
        .filter_map(|m| match m {
            Move::Plunge { feed, .. } | Move::Line { feed, .. } => Some(*feed),
            _ => None,
        })
        .collect();
    assert_eq!(feeds, vec![500.0; 4]);
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 11.0] {
        assert!(ops::tap(&holes(), &cutter(6.0, 500.0), 2.0, bad, 0.5, 10.0).is_err());
    }
    // 10 mm pitch at 200000 rpm would feed 2 000 000 mm/min.
    assert!(ops::tap(&holes(), &cutter(6.0, 200_000.0), 2.0, 10.0, 0.5, 10.0).is_err());
    assert_eq!(
        ops::tap_warnings(6.0, false, "drill", &holes()),
        ["the tool is a drill, not a tap: tapping feeds it in and out at the pitch".to_owned()]
    );
    assert_eq!(ops::tap_warnings(5.0, true, "tap", &holes()).len(), 1);
}

#[test]
fn tapping_posts_as_each_controller_taps() {
    let ops = vec![("Tap".to_owned(), tapped())];
    let opts = PostOptions::default();
    // LinuxCNC: rigid tapping G33.1 from the retract plane (it comes back out to Z2), then
    // up to where the cycle started.
    assert_eq!(
        posted("linuxcnc", &ops, &opts).unwrap(),
        "(holes)\nG21 G90 G17\n(Tap)\nT1 M6\nS500 M3\nG0 Z10\nG0 X0 Y0\nG0 Z2\nG33.1 Z-10.5 K1\n\
         G0 Z10\nG0 X20\nG0 Z2\nG33.1 Z-10.5 K1\nG0 Z10\nM5\nM30\n"
    );
    // Fanuc-style: rigid tapping with M29 before each G84 cycle, sequence numbers in 10s,
    // decimal points, the length offset with the first Z move, home at the end.
    assert_eq!(
        posted("iso-mill", &ops, &opts).unwrap(),
        "%\nO1000 (HOLES)\nN10 G21 G17 G40 G49 G80 G90\nN20 G54\nN30 (TAP)\nN40 T1 M6\n\
         N50 S500 M3\nN60 G0 G43 H1 Z10.\nN70 G0 X0. Y0.\nN80 M29 S500\n\
         N90 G98 G84 X0. Y0. Z-10.5 R2. F500.\nN100 G80\nN110 G0 X20.\nN120 M29 S500\n\
         N130 G98 G84 X20. Y0. Z-10.5 R2.\nN140 G80\nN150 M5\nN160 G91 G28 Z0.\nN170 G90\n\
         N180 M30\n%\n"
    );
    // Mach-style: G84 without M29 (a floating tap holder), incremental arc centres set.
    assert_eq!(
        posted("pc-mill", &ops, &opts).unwrap(),
        "(holes)\nG21 G90 G17 G40 G49 G80\nG91.1\n(Tap)\nT1 M6\nS500 M3\nG0 G43 H1 Z10\n\
         G0 X0 Y0\nG98 G84 X0 Y0 Z-10.5 R2 F500\nG80\nG0 X20\nG98 G84 X20 Y0 Z-10.5 R2\nG80\n\
         M5\nM30\n"
    );
    // GRBL cannot tap: refused, not written out as unsynchronised moves.
    assert!(matches!(
        posted("grbl", &ops, &opts),
        Err(crate::CamError::Refused(_))
    ));
}

/// The square contour of `the_post_for_a_square_contour_matches_hand_written_g_code`: a
/// 20 x 20 square, 6 mm tool outside, one depth at -2, clearance 10.
fn square() -> Vec<(String, Toolpath)> {
    let tp = ops::contour(
        &rect(0.0, 0.0, 20.0, 20.0),
        -2.0,
        Side::Right,
        false,
        &cutter(6.0, 10_000.0),
        0.0,
        5.0,
        0.0,
        0.0,
        10.0,
    )
    .unwrap();
    vec![("Contour".to_owned(), tp)]
}

#[test]
fn the_haas_style_post_writes_arcs_by_radius_and_points_on_every_number() {
    // The corners are quarter circles of radius 3: R3.
    let expected = "%\nO1000 (holes)\nG21 G17 G40 G49 G80 G90\nG54\n(Contour)\nT1 M6\n\
                    S10000 M3\nG0 G43 H1 Z10.\nG0 X0. Y-3.\nG0 Z1.\nG1 Z-2. F200.\n\
                    G1 X20. F600.\nG3 X23. Y0. R3.\nG1 Y20.\nG3 X20. Y23. R3.\nG1 X0.\n\
                    G3 X-3. Y20. R3.\nG1 Y0.\nG3 X0. Y-3. R3.\nG0 Z10.\nM5\nG91 G28 Z0.\nG90\n\
                    M30\n%\n";
    assert_eq!(
        posted("vmc-mill", &square(), &PostOptions::default()).unwrap(),
        expected
    );
}

/// Reads G-code back: the X, Y, Z position after each motion line (G0, G1, G2, G3), in
/// the program's units.
fn read_back(text: &str) -> Vec<(String, [f64; 3])> {
    let mut at = [0.0; 3];
    let mut out = Vec::new();
    let mut motion = String::new();
    for line in text.lines() {
        let code = line.split(['(', ';']).next().unwrap_or("");
        let mut moved = false;
        for w in code.split_whitespace() {
            let (letter, value) = w.split_at(1);
            match letter {
                "G" if ["0", "1", "2", "3"].contains(&value) => motion = format!("G{value}"),
                "X" | "Y" | "Z" => {
                    let i = "XYZ".find(letter).unwrap();
                    at[i] = value.parse().unwrap();
                    moved = true;
                }
                _ => {}
            }
        }
        if moved {
            out.push((motion.clone(), at));
        }
    }
    out
}

#[test]
fn tabs_read_back_from_the_g_code_stand_where_worked_out_by_hand() {
    // The plate posted for LinuxCNC (work origin at the model's), read back: the path is
    // at the tab top Z-3 only over the tabs, 9 either side of each hand-worked centre.
    let tp = plate(&tabs(TabShape::Rectangular, 90.0)).unwrap();
    let text = posted(
        "linuxcnc",
        &[("Profile".into(), tp)],
        &PostOptions::default(),
    )
    .unwrap();
    let pts = read_back(&text);
    let at_top: Vec<[f64; 3]> = pts
        .iter()
        .filter(|(g, p)| g == "G1" && close(p[2], -3.0, 1e-9))
        .map(|(_, p)| *p)
        .collect();
    // Per pass and tab: the climb's end and the run's end, both at -3: 2 x 4 x 2.
    assert_eq!(at_top.len(), 16, "{text}");
    let centres = hand_centres();
    for pair in at_top.chunks(2) {
        let (a, b) = (pair[0], pair[1]);
        let mid = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
        assert!(
            centres
                .iter()
                .any(|c| close(c.0, mid.0, 2e-3) && close(c.1, mid.1, 2e-3)),
            "{mid:?}"
        );
        assert!(close((a[0] - b[0]).hypot(a[1] - b[1]), 18.0, 2e-3));
    }
}

#[test]
fn inch_output_scales_coordinates_and_feeds() {
    // 3 mm is 0.1181 in, 600 mm/min is 23.622 in/min, 200 is 7.874.
    let opts = PostOptions {
        units: Some(OutUnits::Inch),
        ..PostOptions::default()
    };
    let text = posted("linuxcnc", &square(), &opts).unwrap();
    for line in [
        "G20 G90 G17",
        "G0 Z0.3937",
        "G0 X0 Y-0.1181",
        "G1 Z-0.0787 F7.874",
        "G1 X0.7874 F23.622",
        "G3 X0.9055 Y0 I0 J0.1181",
    ] {
        assert!(text.lines().any(|l| l == line), "{line} not in\n{text}");
    }
}

#[test]
fn linearised_arcs_stay_within_the_tolerance() {
    // Quarter circles of radius 3 within 0.01: a chord of angle θ strays r (1 - cos θ/2)
    // from its arc, so θ <= 2 acos(1 - 0.01/3) = 0.16336 and a quarter turn takes
    // ceil((π/2) / 0.16336) = 10 chords. Four corners: 40 chords, plus 4 sides and the
    // plunge: 45 G1 lines and no G2 or G3.
    let mut def = find_post("linuxcnc").unwrap();
    def.arcs = ArcStyle::Linear;
    let text = post_with(
        "sq",
        &square(),
        &Wcs::default(),
        &def,
        &PostOptions::default(),
    )
    .unwrap()
    .text;
    assert!(
        !text
            .lines()
            .any(|l| l.starts_with("G2 ") || l.starts_with("G3 "))
    );
    assert_eq!(
        text.lines().filter(|l| l.starts_with("G1")).count(),
        45,
        "{text}"
    );
    // Every chord end is on a corner circle or a side; every chord's middle is within
    // 0.01 of its circle.
    let pts = read_back(&text);
    let corners = [p(20.0, 0.0), p(20.0, 20.0), p(0.0, 20.0), p(0.0, 0.0)];
    for w in pts.windows(2) {
        let (a, b) = (w[0].1, w[1].1);
        let mid = p((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
        for c in corners {
            let (da, db) = (p(a[0], a[1]).distance(c), p(b[0], b[1]).distance(c));
            if close(da, 3.0, 2e-3)
                && close(db, 3.0, 2e-3)
                && p(a[0], a[1]).distance(p(b[0], b[1])) > 1e-6
            {
                assert!(3.0 - mid.distance(c) <= 0.01 + 2e-3);
            }
        }
    }
}

#[test]
fn coolant_and_line_numbers_follow_the_post_data() {
    let opts = PostOptions {
        coolant: Coolant::Flood,
        ..PostOptions::default()
    };
    let text = posted("linuxcnc", &square(), &opts).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    // Flood on after the spindle starts, off before it stops at the end.
    let on = lines.iter().position(|l| *l == "M8").unwrap();
    assert_eq!(lines[on - 1], "S10000 M3");
    let n = lines.len();
    assert_eq!(&lines[n - 3..], ["M9", "M5", "M30"]);
    // Sequence numbers from 10 in steps of 10, starting again at 10 past 50.
    let mut def = find_post("linuxcnc").unwrap();
    def.line_numbers = Some(LineNumbers {
        start: 10,
        step: 10,
        max: Some(50),
    });
    let text = post_with(
        "sq",
        &square(),
        &Wcs::default(),
        &def,
        &PostOptions::default(),
    )
    .unwrap()
    .text;
    let numbers: Vec<u32> = text
        .lines()
        .filter_map(|l| l.strip_prefix('N'))
        .filter_map(|l| l.split(' ').next()?.parse().ok())
        .collect();
    let expected: Vec<u32> = (0..numbers.len())
        .map(|i| 10 * (i as u32 % 5 + 1))
        .collect();
    assert_eq!(numbers, expected);
    // The header comment is not numbered.
    assert_eq!(text.lines().next(), Some("(sq)"));
    // A post without mist says so and writes none.
    let mut dry = find_post("grbl").unwrap();
    dry.coolant.mist = None;
    let r = post_with(
        "sq",
        &square(),
        &Wcs::default(),
        &dry,
        &PostOptions {
            coolant: Coolant::Mist,
            ..PostOptions::default()
        },
    )
    .unwrap();
    assert!(!r.text.contains("M7") && r.warnings.len() == 1, "{r:?}");
}

#[test]
fn a_post_of_our_own_is_data_too() {
    // A semicolon-comment post with R arcs and no tool changer, written as JSON.
    let json = r#"{
        "format": "chamfera-post", "version": 1, "name": "my-router",
        "title": "My router", "comments": "semicolon", "decimals": 2,
        "header": ["{comment:{program}}"], "start": ["{units} G90"],
        "tool_change": ["M0 {comment:load T{tool}}"], "spindle_on": "M3 S{rpm}",
        "arcs": "r", "cycles": {"drill": false}, "end": ["M2"]
    }"#;
    let def = PostDef::from_json(json).unwrap();
    let text = post_with(
        "sq",
        &square(),
        &Wcs::default(),
        &def,
        &PostOptions::default(),
    )
    .unwrap()
    .text;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        &lines[..5],
        ["; sq", "G21 G90", "; Contour", "M0 ; load T1", "M3 S10000"]
    );
    assert!(lines.contains(&"G3 X23 Y0 R3"));
    assert_eq!(lines.last(), Some(&"M2"));
    // A full turn by radius goes in two halves.
    let circle = Toolpath {
        tool_number: 1,
        tool_name: "t".into(),
        spindle_rpm: 1000.0,
        moves: vec![
            Move::Rapid {
                to: Point3::new(10.0, 0.0, 0.0),
            },
            Move::Arc {
                to: Point3::new(10.0, 0.0, 0.0),
                centre: p(0.0, 0.0),
                clockwise: false,
                feed: 100.0,
            },
        ],
    };
    let text = post_with(
        "c",
        &[("c".into(), circle)],
        &Wcs::default(),
        &def,
        &PostOptions::default(),
    )
    .unwrap()
    .text;
    assert!(
        text.contains("G3 X-10 Y0 R10 F100\nG3 X10 Y0 R10"),
        "{text}"
    );
}
