//! Sheet metal against hand-derived values.

use super::*;
use std::f64::consts::PI;

const T: f64 = 2.0;
const R: f64 = 2.0;
const K: f64 = 0.44;

/// The 100 x 50 base on the XY plane, x in 0..100, y in 0..50, z in 0..2. Its profile
/// edges: 0 along y = 0, 1 along x = 100, 2 along y = 50, 3 along x = 0.
fn base() -> Sheet {
    let profile = Profile::polygon(&[
        Point2::new(0.0, 0.0),
        Point2::new(100.0, 0.0),
        Point2::new(100.0, 50.0),
        Point2::new(0.0, 50.0),
    ]);
    Sheet::base(&Frame::XY, &profile, T, false).unwrap()
}

fn edge(body: &Body, name: &str) -> EdgeId {
    body.edges()
        .into_iter()
        .find(|e| body.edge_name(*e).unwrap() == Some(name))
        .unwrap_or_else(|| panic!("no edge {name}"))
}

fn spec(height: f64) -> FlangeSpec {
    FlangeSpec {
        height,
        angle: PI / 2.0,
        radius: R,
        k_factor: K,
        position: BendPosition::Inside,
        flip: false,
        offset_start: 0.0,
        offset_end: 0.0,
        relief: Relief {
            kind: ReliefKind::Rectangular,
            width: T,
            depth: T,
        },
    }
}

fn close(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} != {b} (tolerance {tol})");
}

#[test]
fn a_base_is_a_plate_of_the_thickness() {
    let s = base();
    // 100 * 50 * 2.
    close(s.folded().volume().unwrap(), 10_000.0, 1e-9);
    s.folded().validate().unwrap();
    let flat = s.flat().unwrap();
    close(flat.volume().unwrap(), 10_000.0, 1e-9);
}

#[test]
fn the_bend_allowance_and_setback_follow_the_formulas() {
    // BA = θ (R + K T) = π/2 (2 + 0.88); OSSB = (R + T) tan 45° = 4.
    close(bend_allowance(PI / 2.0, R, K, T), PI / 2.0 * 2.88, 1e-15);
    close(outside_setback(PI / 2.0, R, T), 4.0, 1e-12);
    close(
        position_trim(BendPosition::Outside, PI / 2.0, R, T),
        2.0,
        1e-12,
    );
    assert_eq!(
        position_trim(BendPosition::BendOutside, PI / 2.0, R, T),
        0.0
    );
}

#[test]
fn a_right_angle_flange_has_an_exact_bend_and_unfolds_by_the_bend_allowance() {
    let s = base();
    let e = edge(s.folded(), "top 1");
    let f = s.flange(e, &spec(30.0)).unwrap();
    let body = f.folded();
    body.validate().unwrap();
    // Inside position: the base is trimmed back by OSSB = 4 to x = 96; the bend is a
    // quarter annulus of radii 2 and 4 about the line x = 96, z = 4, 50 long; the flange's
    // straight part is 30 - 4 = 26 tall, 2 thick, 50 long.
    let base_v = 96.0 * 50.0 * 2.0;
    let bend_v = PI / 4.0 * ((R + T).powi(2) - R * R) * 50.0;
    let flange_v = 26.0 * 2.0 * 50.0;
    close(body.volume().unwrap(), base_v + bend_v + flange_v, 1e-6);
    // The part keeps its outline: x 0..100, z 0..30.
    let bb = body.bounding_box().unwrap();
    close(bb.max.x, 100.0, 1e-9);
    close(bb.max.z, 30.0, 1e-9);
    // Its faces are named for the flange.
    let names: Vec<String> = body
        .faces()
        .into_iter()
        .filter_map(|x| body.face_name(x).unwrap().map(str::to_owned))
        .collect();
    for n in [
        "bend outer",
        "bend inner",
        "flange outer",
        "flange inner",
        "flange end",
    ] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
    // Flat: (100 - setback) + BA + (flange straight part) = 96 + π/2 (2 + 0.44 * 2) + 26.
    let flat = f.flat().unwrap();
    flat.validate().unwrap();
    let ba = PI / 2.0 * (2.0 + 0.44 * 2.0);
    let bb = flat.bounding_box().unwrap();
    close(bb.max.x - bb.min.x, (100.0 - 4.0) + ba + 26.0, 1e-9);
    close(bb.max.y - bb.min.y, 50.0, 1e-9);
    close(bb.max.z - bb.min.z, T, 1e-9);
    close(
        flat.volume().unwrap(),
        ((100.0 - 4.0) + ba + 26.0) * 50.0 * T,
        1e-6,
    );
    assert_eq!(f.bends().len(), 1);
    close(f.bends()[0].allowance, ba, 1e-12);
}

#[test]
fn bend_positions_move_the_flange_against_the_edge() {
    let s = base();
    let e = edge(s.folded(), "top 1");
    // Outside: trimmed by R = 2, the flange's inner face on x = 100, outer at 102.
    let out = s
        .flange(
            e,
            &FlangeSpec {
                position: BendPosition::Outside,
                ..spec(30.0)
            },
        )
        .unwrap();
    close(out.folded().bounding_box().unwrap().max.x, 102.0, 1e-9);
    // Bend outside: nothing trimmed, the bend starts at x = 100, outer face at 104.
    let bo = s
        .flange(
            e,
            &FlangeSpec {
                position: BendPosition::BendOutside,
                ..spec(30.0)
            },
        )
        .unwrap();
    close(bo.folded().bounding_box().unwrap().max.x, 104.0, 1e-9);
    // Flat: 100 + BA + 26.
    let ba = PI / 2.0 * 2.88;
    let bb = bo.flat().unwrap().bounding_box().unwrap();
    close(bb.max.x - bb.min.x, 100.0 + ba + 26.0, 1e-9);
}

#[test]
fn a_u_channel_unfolds_to_the_sum_of_its_flats_and_bends() {
    let s = base();
    let one = s.flange(edge(s.folded(), "top 1"), &spec(30.0)).unwrap();
    let two = one
        .flange(edge(one.folded(), "top 3"), &spec(30.0))
        .unwrap();
    two.folded().validate().unwrap();
    // Folded: 100 wide over both flanges, 30 tall.
    let bb = two.folded().bounding_box().unwrap();
    close(bb.min.x, 0.0, 1e-9);
    close(bb.max.x, 100.0, 1e-9);
    close(bb.max.z, 30.0, 1e-9);
    // Flat width: the web 100 - 2 * 4 = 92, two bends of BA = π/2 (2 + 0.88) and two
    // straight parts of 30 - 4 = 26: 92 + 52 + π * 2.88.
    let flat = two.flat().unwrap();
    let bb = flat.bounding_box().unwrap();
    close(bb.max.x - bb.min.x, 92.0 + 52.0 + PI * 2.88, 1e-9);
    // The web stays where it was: from x = 4 - (BA + 26) to 96 + BA + 26.
    let ba = PI / 2.0 * 2.88;
    close(bb.min.x, 4.0 - ba - 26.0, 1e-9);
}

#[test]
fn a_flange_on_a_flange_unfolds_through_both_bends() {
    let s = base();
    let one = s.flange(edge(s.folded(), "top 1"), &spec(30.0)).unwrap();
    // The end of the first flange, at z = 30: a second 90° flange going back over the
    // base (toward -x), from its inner end edge.
    let e = edge(one.folded(), "end inner");
    let two = one.flange(e, &spec(20.0)).unwrap();
    two.folded().validate().unwrap();
    // Flat: 96 + BA + (30 - 4 - 4) + BA + (20 - 4): the first flange's straight part is
    // trimmed by the second bend's setback too.
    let ba = PI / 2.0 * 2.88;
    let bb = two.flat().unwrap().bounding_box().unwrap();
    close(bb.max.x - bb.min.x, 96.0 + ba + 22.0 + ba + 16.0, 1e-9);
}

#[test]
fn a_flange_on_a_curved_or_inner_edge_is_refused() {
    let profile = Profile::new(
        vec![
            ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(40.0, 0.0)),
            ProfileEdge::Arc {
                centre: Point2::new(40.0, 20.0),
                radius: 20.0,
                start_angle: -PI / 2.0,
                sweep: PI,
            },
            ProfileEdge::line(Point2::new(40.0, 40.0), Point2::new(0.0, 40.0)),
            ProfileEdge::line(Point2::new(0.0, 40.0), Point2::new(0.0, 0.0)),
        ],
        Vec::new(),
    );
    let s = Sheet::base(&Frame::XY, &profile, T, false).unwrap();
    let before = s.folded().volume().unwrap();
    // The arc's top edge is curved.
    assert!(s.flange(edge(s.folded(), "top 1"), &spec(30.0)).is_err());
    // A vertical edge (between two side faces) has no T-wide side face square to a sheet
    // face beside it.
    assert!(s.flange(edge(s.folded(), "sweep 0"), &spec(30.0)).is_err());
    close(s.folded().volume().unwrap(), before, 1e-12);
    // A flange too short for its bend, and hostile values, are refused.
    let e = edge(s.folded(), "top 0");
    assert!(s.flange(e, &spec(3.9)).is_err());
    for bad in [
        FlangeSpec {
            angle: 0.0,
            ..spec(30.0)
        },
        FlangeSpec {
            angle: PI,
            ..spec(30.0)
        },
        FlangeSpec {
            angle: f64::NAN,
            ..spec(30.0)
        },
        FlangeSpec {
            radius: -1.0,
            ..spec(30.0)
        },
        FlangeSpec {
            k_factor: 1.5,
            ..spec(30.0)
        },
        FlangeSpec {
            height: f64::INFINITY,
            ..spec(30.0)
        },
        FlangeSpec {
            offset_start: 30.0,
            offset_end: 20.0,
            ..spec(30.0)
        },
    ] {
        assert!(s.flange(e, &bad).is_err(), "{bad:?}");
    }
}

#[test]
fn reliefs_free_a_flange_that_stops_short_of_the_edge() {
    let s = base();
    let e = edge(s.folded(), "top 1");
    // A flange 30 long in the middle of the 50 mm edge (10 off each end), relief 2 wide
    // reaching 2 past the bend's start: each notch is 2 (along y) by (4 + 2) (along x)
    // by 2 thick.
    let sp = FlangeSpec {
        offset_start: 10.0,
        offset_end: 10.0,
        ..spec(30.0)
    };
    let f = s.flange(e, &sp).unwrap();
    f.folded().validate().unwrap();
    let base_v = 100.0 * 50.0 * 2.0 - 4.0 * 30.0 * 2.0 - 2.0 * (2.0 * 6.0 * 2.0);
    let bend_v = PI / 4.0 * (16.0 - 4.0) * 30.0;
    let flange_v = 26.0 * 2.0 * 30.0;
    close(
        f.folded().volume().unwrap(),
        base_v + bend_v + flange_v,
        1e-6,
    );
    // Obround, 6 deep: each notch is 2 wide from the side face to 4 + 6 = 10 behind it,
    // then a half circle of radius 1: (2 * 10 + π/2) * 2 thick, twice.
    let ob = FlangeSpec {
        relief: Relief {
            kind: ReliefKind::Obround,
            width: 2.0,
            depth: 6.0,
        },
        ..sp
    };
    let g = s.flange(e, &ob).unwrap();
    g.folded().validate().unwrap();
    let base_ob = 100.0 * 50.0 * 2.0 - 4.0 * 30.0 * 2.0 - 2.0 * (20.0 + PI / 2.0) * 2.0;
    close(
        g.folded().volume().unwrap(),
        base_ob + bend_v + flange_v,
        1e-6,
    );
    // Closer to the bend, its round end would need crossing cylinders (K10): refused
    // with a clear reason, nothing built.
    let near = FlangeSpec {
        relief: Relief {
            kind: ReliefKind::Obround,
            width: 2.0,
            depth: 2.0,
        },
        ..sp
    };
    let err = s.flange(e, &near).unwrap_err().to_string();
    assert!(err.contains("rectangular"), "{err}");
    // Its flat pattern outline has the round ends as arcs.
    let flat = g.flat().unwrap();
    let outline = flat_outline(&flat, &Frame::XY).unwrap();
    let arcs = outline
        .iter()
        .filter(|x| matches!(x, FlatEdge::Arc { .. }))
        .count();
    assert_eq!(arcs, 2);
    // A rip takes nothing away.
    let rip = FlangeSpec {
        relief: Relief {
            kind: ReliefKind::Rip,
            ..sp.relief
        },
        ..sp
    };
    let h = s.flange(e, &rip).unwrap();
    close(
        h.folded().volume().unwrap(),
        100.0 * 50.0 * 2.0 - 4.0 * 30.0 * 2.0 + bend_v + flange_v,
        1e-6,
    );
}

#[test]
fn a_flat_outline_is_written_as_dxf() {
    let s = base();
    let f = s.flange(edge(s.folded(), "top 1"), &spec(30.0)).unwrap();
    let flat = f.flat().unwrap();
    let outline = flat_outline(&flat, &Frame::XY).unwrap();
    // A rectangle 0..(96 + BA + 26) by 0..50: four lines.
    assert_eq!(outline.len(), 4, "{outline:?}");
    let len = 96.0 + PI / 2.0 * 2.88 + 26.0;
    let max_x = outline
        .iter()
        .filter_map(|e| match e {
            FlatEdge::Line { a, b } => Some(a.x.max(b.x)),
            _ => None,
        })
        .fold(f64::MIN, f64::max);
    close(max_x, len, 1e-9);
    let dxf = flat_dxf(&outline);
    assert!(dxf.starts_with("  0\nSECTION\n  2\nHEADER\n"));
    assert!(dxf.contains("AC1009"));
    assert_eq!(dxf.matches("\nLINE\n").count(), 4);
    assert!(dxf.ends_with("  0\nEOF\n"));
}
