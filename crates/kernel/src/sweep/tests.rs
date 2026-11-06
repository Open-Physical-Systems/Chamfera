//! Tests for K4 sweeps. Expected values are closed forms, derived in the comments.

use super::*;
use crate::{PointOnFace, Profile, ProfileEdge, polygon_edges};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, PI};

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * b.abs().max(1.0)
}

fn assert_close(what: &str, got: f64, want: f64) {
    assert!(close(got, want, 1e-9), "{what}: got {got}, want {want}");
}

fn rect(x0: f64, y0: f64, w: f64, h: f64) -> Vec<ProfileEdge> {
    polygon_edges(&[
        Point2::new(x0, y0),
        Point2::new(x0 + w, y0),
        Point2::new(x0 + w, y0 + h),
        Point2::new(x0, y0 + h),
    ])
}

/// (V, E, F, genus) after a full validity check.
fn counts(b: &Body) -> (usize, usize, usize, usize) {
    let r = b.validate().unwrap();
    let s = r.summary;
    (s.vertices, s.edges, s.faces, r.genus)
}

fn face_named(b: &Body, name: &str) -> FaceId {
    b.faces()
        .into_iter()
        .find(|f| b.face_name(*f).unwrap() == Some(name))
        .unwrap_or_else(|| panic!("no face named {name}"))
}

fn face_names(b: &Body) -> BTreeSet<String> {
    b.faces()
        .into_iter()
        .map(|f| b.face_name(f).unwrap().unwrap().to_owned())
        .collect()
}

fn edge_names(b: &Body) -> BTreeSet<String> {
    b.edges()
        .into_iter()
        .map(|e| b.edge_name(e).unwrap().unwrap().to_owned())
        .collect()
}

fn vertex_names(b: &Body) -> BTreeSet<String> {
    b.vertices()
        .into_iter()
        .map(|v| b.vertex_name(v).unwrap().unwrap().to_owned())
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

/// The y axis of the sketch plane.
fn y_axis() -> Line2 {
    Line2::new(Point2::ORIGIN, Vec2::Y).unwrap()
}

// ---------------------------------------------------------------- extrude

#[test]
fn a_50_by_30_rectangle_extruded_by_10_is_the_box() {
    let p = Profile::new(rect(0.0, 0.0, 50.0, 30.0), vec![]);
    let b = extrude(&Frame::XY, &p, 10.0).unwrap();
    // V = 2n, E = 3n, F = n + 2 for n = 4.
    assert_eq!(counts(&b), (8, 12, 6, 0));
    assert_close("volume", b.volume().unwrap(), 15_000.0);
    // 2 (50·30) + 2 (50·10) + 2 (30·10) = 4600.
    assert_close("area", b.area().unwrap(), 4_600.0);
    assert_eq!(
        face_names(&b),
        set(&["bottom", "top", "side 0", "side 1", "side 2", "side 3"])
    );
    // Side 0 is swept by the edge from (0, 0) to (50, 0): it faces -y, area 500.
    let s0 = face_named(&b, "side 0");
    let Some((Surface::Plane(f), same)) = b.face_surface(s0).unwrap() else {
        panic!("side 0 is planar");
    };
    let n = if same { f.z() } else { -f.z() };
    assert!(close(n.y, -1.0, 1e-15));
    assert_close("side 0", b.face_area(s0).unwrap(), 500.0);
    let bb = b.bounding_box().unwrap();
    assert!(bb.min.coincident(Point3::ORIGIN) && bb.max.coincident(Point3::new(50.0, 30.0, 10.0)));
    assert_eq!(
        edge_names(&b),
        set(&[
            "bottom 0", "bottom 1", "bottom 2", "bottom 3", "top 0", "top 1", "top 2", "top 3",
            "sweep 0", "sweep 1", "sweep 2", "sweep 3"
        ])
    );
    assert_eq!(
        vertex_names(&b),
        set(&[
            "bottom 0", "bottom 1", "bottom 2", "bottom 3", "top 0", "top 1", "top 2", "top 3"
        ])
    );
}

#[test]
fn negative_and_symmetric_extrusions_go_the_right_way() {
    let p = Profile::new(rect(0.0, 0.0, 50.0, 30.0), vec![]);
    let down = extrude(&Frame::XY, &p, -10.0).unwrap();
    down.validate().unwrap();
    assert_close("volume", down.volume().unwrap(), 15_000.0);
    let bb = down.bounding_box().unwrap();
    assert!(bb.min.coincident(Point3::new(0.0, 0.0, -10.0)));
    assert!(bb.max.coincident(Point3::new(50.0, 30.0, 0.0)));
    // The bottom stays on the sketch plane, facing +z (into the empty side).
    let bottom = face_named(&down, "bottom");
    let Some((Surface::Plane(f), true)) = down.face_surface(bottom).unwrap() else {
        panic!()
    };
    assert!(close(f.z().z, 1.0, 1e-15) && close(f.origin().z, 0.0, 1e-15));
    // Side 0 is still swept by input edge 0 (y = 0).
    let s0 = face_named(&down, "side 0");
    let c = down.face_edges(s0).unwrap();
    for e in c {
        let [v0, v1] = down.edge_vertices(e).unwrap();
        assert!(down.vertex_point(v0).unwrap().y.abs() < 1e-12);
        assert!(down.vertex_point(v1).unwrap().y.abs() < 1e-12);
    }
    let both = extrude_symmetric(&Frame::XY, &p, 10.0).unwrap();
    both.validate().unwrap();
    let bb = both.bounding_box().unwrap();
    assert!(bb.min.coincident(Point3::new(0.0, 0.0, -5.0)));
    assert!(bb.max.coincident(Point3::new(50.0, 30.0, 5.0)));
    assert_close("volume", both.volume().unwrap(), 15_000.0);
}

#[test]
fn a_rectangle_with_a_round_hole_extruded() {
    let hole = vec![ProfileEdge::circle(Point2::new(25.0, 15.0), 5.0)];
    let p = Profile::new(rect(0.0, 0.0, 50.0, 30.0), vec![hole]);
    let b = extrude(&Frame::XY, &p, 10.0).unwrap();
    // Box 8/12/6 plus the hole: 2 vertices (bottom, top), 3 edges (two circles and the
    // seam), 1 face: V - E + F - rings = 10 - 15 + 7 - 2 = 0 = 2 (1 - 1).
    assert_eq!(counts(&b), (10, 15, 7, 1));
    // V = 10 (1500 - 25π).
    assert_close("volume", b.volume().unwrap(), 10.0 * (1500.0 - 25.0 * PI));
    // Area: caps 2 (1500 - 25π), outer sides 2 (500 + 300) = 1600, hole wall 2π·5·10 =
    // 100π: 4600 + 50π.
    assert_close("area", b.area().unwrap(), 4_600.0 + 50.0 * PI);
    let wall = face_named(&b, "hole 0 side 0");
    assert_close("wall", b.face_area(wall).unwrap(), 100.0 * PI);
    // The wall's outward normal points into the hole (toward the circle's centre).
    let Some((Surface::Cylinder(c), false)) = b.face_surface(wall).unwrap() else {
        panic!("the hole wall is a cylinder facing its axis");
    };
    assert_eq!(c.radius(), 5.0);
    assert!(edge_names(&b).contains("hole 0 sweep 0"));
    assert!(edge_names(&b).contains("hole 0 top 0"));
    assert!(vertex_names(&b).contains("hole 0 bottom 0"));
    // The top face holes: the hole's centre is outside it, a corner region inside.
    let top = face_named(&b, "top");
    let at = |p: Point3| b.classify_point_on_face(top, p).unwrap();
    assert_eq!(at(Point3::new(25.0, 15.0, 10.0)), PointOnFace::Outside);
    assert_eq!(at(Point3::new(2.0, 2.0, 10.0)), PointOnFace::Inside);
    assert_eq!(at(Point3::new(30.0, 15.0, 10.0)), PointOnFace::Boundary);
}

#[test]
fn a_circle_extruded_is_a_cylinder() {
    let p = Profile::circle(Point2::ORIGIN, 5.0);
    let b = extrude(&Frame::XY, &p, 10.0).unwrap();
    assert_eq!(counts(&b), (2, 3, 3, 0));
    // π r² h = 250π; 2π r h + 2 π r² = 100π + 50π.
    assert_close("volume", b.volume().unwrap(), 250.0 * PI);
    assert_close("area", b.area().unwrap(), 150.0 * PI);
    assert_eq!(face_names(&b), set(&["bottom", "top", "side 0"]));
    assert_eq!(edge_names(&b), set(&["bottom 0", "top 0", "sweep 0"]));
    let side = face_named(&b, "side 0");
    assert_eq!(
        b.classify_point_on_face(side, Point3::new(-5.0, 0.0, 3.0))
            .unwrap(),
        PointOnFace::Inside
    );
}

#[test]
fn a_slot_extruded() {
    // Centres 20 apart, r = 5: lines y = ±5 from x = 0 to 20, half circles at both ends.
    let slot = vec![
        ProfileEdge::line(Point2::new(0.0, -5.0), Point2::new(20.0, -5.0)),
        ProfileEdge::Arc {
            centre: Point2::new(20.0, 0.0),
            radius: 5.0,
            start_angle: -FRAC_PI_2,
            sweep: PI,
        },
        ProfileEdge::line(Point2::new(20.0, 5.0), Point2::new(0.0, 5.0)),
        ProfileEdge::Arc {
            centre: Point2::new(0.0, 0.0),
            radius: 5.0,
            start_angle: FRAC_PI_2,
            sweep: PI,
        },
    ];
    let p = Profile::new(slot, vec![]);
    let b = extrude(&Frame::XY, &p, 10.0).unwrap();
    assert_eq!(counts(&b), (8, 12, 6, 0));
    // Profile area 20·10 + π 5² = 200 + 25π; volume 10 times that.
    assert_close("volume", b.volume().unwrap(), 2_000.0 + 250.0 * PI);
    // Caps 2 (200 + 25π), flat sides 2·20·10 = 400, half cylinders 2·(π·5·10) = 100π.
    assert_close("area", b.area().unwrap(), 800.0 + 150.0 * PI);
    for i in [1, 3] {
        let f = face_named(&b, &format!("side {i}"));
        assert!(matches!(
            b.face_surface(f).unwrap(),
            Some((Surface::Cylinder(_), true))
        ));
        assert_close("half cylinder", b.face_area(f).unwrap(), 50.0 * PI);
    }
}

#[test]
fn a_tilted_profile_with_arcs_and_holes_keeps_area_times_height() {
    let frame = Frame::from_normal_and_x(
        Point3::new(10.0, -20.0, 5.0),
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(3.0, 0.0, -1.0),
    )
    .unwrap();
    // A 40 x 20 rectangle with a round hole and a square hole, given clockwise.
    let mut outer = rect(0.0, 0.0, 40.0, 20.0);
    outer.reverse();
    let outer: Vec<ProfileEdge> = outer.iter().map(ProfileEdge::reversed).collect();
    let p = Profile::new(
        outer,
        vec![
            vec![ProfileEdge::circle(Point2::new(10.0, 10.0), 4.0)],
            rect(25.0, 5.0, 10.0, 10.0),
        ],
    );
    // Area 800 - 16π - 100 = 700 - 16π.
    let area = 700.0 - 16.0 * PI;
    assert_close("profile area", p.area().unwrap(), area);
    let b = extrude(&frame, &p, -7.5).unwrap();
    assert_eq!(b.validate().unwrap().genus, 2);
    assert_close("volume", b.volume().unwrap(), area * 7.5);
    // Area: 2 caps, outer walls 7.5·120, round wall 2π·4·7.5, square walls 7.5·40.
    let want = 2.0 * area + 7.5 * 120.0 + 60.0 * PI + 7.5 * 40.0;
    assert_close("area", b.area().unwrap(), want);
    assert!(face_names(&b).contains("hole 1 side 3"));
}

// ---------------------------------------------------------------- revolve

#[test]
fn a_rectangle_revolved_a_full_turn_is_a_tube() {
    let (r1, r2, h) = (3.0, 5.0, 4.0);
    let p = Profile::new(rect(r1, 0.0, r2 - r1, h), vec![]);
    let b = revolve(&Frame::XY, &p, &y_axis(), TAU).unwrap();
    // 4 vertices (each on a circle), 6 edges (the seams of the two cylinders and 4
    // circles; the annuli have no seam, their inner circles are rings), 4 faces, 2 rings,
    // genus 1: 4 - 6 + 4 - 2 = 0.
    assert_eq!(counts(&b), (4, 6, 4, 1));
    assert_eq!(b.summary().rings, 2);
    // π (r2² - r1²) h = π·16·4 = 64π.
    assert_close("volume", b.volume().unwrap(), PI * (r2 * r2 - r1 * r1) * h);
    // Outer and inner cylinders 2π r h, two annuli π (r2² - r1²).
    let want = TAU * (r1 + r2) * h + 2.0 * PI * (r2 * r2 - r1 * r1);
    assert_close("area", b.area().unwrap(), want);
    assert_eq!(
        face_names(&b),
        set(&["side 0", "side 1", "side 2", "side 3"])
    );
    assert_eq!(
        edge_names(&b),
        set(&[
            "seam 1", "seam 3", "sweep 0", "sweep 1", "sweep 2", "sweep 3"
        ])
    );
    // Side 1 (x = r2) is the outer cylinder, side 3 (x = r1) the inner one facing in.
    let outer = face_named(&b, "side 1");
    assert!(matches!(
        b.face_surface(outer).unwrap(),
        Some((Surface::Cylinder(_), true))
    ));
    assert!(matches!(
        b.face_surface(face_named(&b, "side 3")).unwrap(),
        Some((Surface::Cylinder(_), false))
    ));
    // The bottom annulus (side 0) lies in the plane y = 0.
    assert!(matches!(
        b.face_surface(face_named(&b, "side 0")).unwrap(),
        Some((Surface::Plane(_), _))
    ));
    // A point on the outer cylinder half way round (x = -5, y = 2) is inside that face.
    assert_eq!(
        b.classify_point_on_face(outer, Point3::new(0.0, 2.0, 5.0))
            .unwrap(),
        PointOnFace::Inside
    );
}

#[test]
fn a_quarter_turn_gives_a_quarter_tube_with_planar_caps() {
    let (r1, r2, h) = (3.0, 5.0, 4.0);
    let p = Profile::new(rect(r1, 0.0, r2 - r1, h), vec![]);
    let b = revolve(&Frame::XY, &p, &y_axis(), FRAC_PI_2).unwrap();
    // Like a prism of the rectangle: 8, 12, 6.
    assert_eq!(counts(&b), (8, 12, 6, 0));
    assert_close(
        "volume",
        b.volume().unwrap(),
        PI * (r2 * r2 - r1 * r1) * h / 4.0,
    );
    // A quarter of the tube's curved and annular area, plus two caps (r2 - r1) h.
    let curved = TAU * (r1 + r2) * h + 2.0 * PI * (r2 * r2 - r1 * r1);
    let caps = 2.0 * (r2 - r1) * h;
    assert_close("area", b.area().unwrap(), curved / 4.0 + caps);
    assert!(face_names(&b).contains("start cap") && face_names(&b).contains("end cap"));
    let start = face_named(&b, "start cap");
    let end = face_named(&b, "end cap");
    assert_close("start cap", b.face_area(start).unwrap(), (r2 - r1) * h);
    assert_close("end cap", b.face_area(end).unwrap(), (r2 - r1) * h);
    // The rotation about +y turns +x toward -z; a quarter turn puts the end cap in the
    // plane x = 0 on the -z side.
    let bb = b.bounding_box().unwrap();
    assert!(bb.min.coincident(Point3::new(0.0, 0.0, -r2)), "{bb:?}");
    assert!(bb.max.coincident(Point3::new(r2, h, 0.0)), "{bb:?}");
    assert!(edge_names(&b).contains("start 0") && edge_names(&b).contains("end 3"));
    // A negative angle turns the other way, toward +z.
    let back = revolve(&Frame::XY, &p, &y_axis(), -FRAC_PI_2).unwrap();
    back.validate().unwrap();
    let bb = back.bounding_box().unwrap();
    assert!(bb.max.coincident(Point3::new(r2, h, r2)), "{bb:?}");
}

#[test]
fn a_half_disc_revolved_is_a_sphere() {
    let r = 3.0;
    // The arc from the south pole (0, -r) counter-clockwise through (r, 0) to the north
    // pole, closed by the diameter on the axis.
    let p = Profile::new(
        vec![
            ProfileEdge::Arc {
                centre: Point2::ORIGIN,
                radius: r,
                start_angle: -FRAC_PI_2,
                sweep: PI,
            },
            ProfileEdge::line(Point2::new(0.0, r), Point2::new(0.0, -r)),
        ],
        vec![],
    );
    let b = revolve(&Frame::XY, &p, &y_axis(), TAU).unwrap();
    // Two poles, the seam, one face: 2 - 1 + 1 = 2.
    assert_eq!(counts(&b), (2, 1, 1, 0));
    assert_close("volume", b.volume().unwrap(), 4.0 / 3.0 * PI * r * r * r);
    assert_close("area", b.area().unwrap(), 4.0 * PI * r * r);
    assert_eq!(vertex_names(&b), set(&["axis 0", "axis 1"]));
    let s = face_named(&b, "side 0");
    assert!(matches!(
        b.face_surface(s).unwrap(),
        Some((Surface::Sphere(_), true))
    ));
    assert_eq!(
        b.classify_point_on_face(s, Point3::new(-r, 0.0, 0.0))
            .unwrap(),
        PointOnFace::Inside
    );
    // Half a turn: a half ball, 2/3 π r³, area 2π r² + two half discs π r².
    let half = revolve(&Frame::XY, &p, &y_axis(), PI).unwrap();
    // Both arc ends are poles on the axis: the two caps meet the sphere along the start
    // and end arcs and each other along the diameter. V = 2, E = 3, F = 3.
    assert_eq!(counts(&half), (2, 3, 3, 0));
    // The turn about +y takes +x toward -z: the half ball lies at z <= 0.
    let s = face_named(&half, "side 0");
    let at = |p: Point3| half.classify_point_on_face(s, p).unwrap();
    assert_eq!(at(Point3::new(0.0, 0.0, -r)), PointOnFace::Inside);
    assert_eq!(at(Point3::new(0.0, 0.0, r)), PointOnFace::Outside);
    assert_eq!(at(Point3::new(r, 0.0, 0.0)), PointOnFace::Boundary);
    assert_eq!(at(Point3::new(0.0, r, 0.0)), PointOnFace::Boundary);
    let q = r * 0.5f64.sqrt();
    assert_eq!(at(Point3::new(0.0, q, -q)), PointOnFace::Inside);
    assert_eq!(at(Point3::new(-q, -q, 0.0)), PointOnFace::Boundary);
    assert_eq!(at(Point3::new(-q, -q, 1e-3)), PointOnFace::Outside);
    assert_close("volume", half.volume().unwrap(), 2.0 / 3.0 * PI * r * r * r);
    assert_close("area", half.area().unwrap(), 3.0 * PI * r * r);
    assert!(edge_names(&half).contains("axis 1"));
}

#[test]
fn a_circle_revolved_is_a_torus() {
    let (big, a) = (5.0, 2.0);
    let p = Profile::circle(Point2::new(big, 0.0), a);
    let b = revolve(&Frame::XY, &p, &y_axis(), TAU).unwrap();
    // One vertex, the meridian and parallel seams, one face, genus 1: 1 - 2 + 1 = 0.
    assert_eq!(counts(&b), (1, 2, 1, 1));
    // 2π² R a² and 4π² R a.
    assert_close("volume", b.volume().unwrap(), 2.0 * PI * PI * big * a * a);
    assert_close("area", b.area().unwrap(), 4.0 * PI * PI * big * a);
    let f = face_named(&b, "side 0");
    assert!(matches!(
        b.face_surface(f).unwrap(),
        Some((Surface::Torus(_), true))
    ));
    // The tube's innermost point half way round, at x = -(R - a).
    assert_eq!(
        b.classify_point_on_face(f, Point3::new(-(big - a), 0.0, 0.0))
            .unwrap(),
        PointOnFace::Inside
    );
    // A third of a turn: a third of the volume, plus two discs π a² in the area.
    let part = revolve(&Frame::XY, &p, &y_axis(), TAU / 3.0).unwrap();
    assert_eq!(counts(&part), (2, 3, 3, 0));
    assert_close(
        "volume",
        part.volume().unwrap(),
        2.0 * PI * PI * big * a * a / 3.0,
    );
    let want = 4.0 * PI * PI * big * a / 3.0 + 2.0 * PI * a * a;
    assert_close("area", part.area().unwrap(), want);
}

#[test]
fn points_on_a_partial_torus_are_classified_by_their_angle() {
    // The circle (5 + 2 cos φ, 2 sin φ) turned by θ about +y is
    // (ρ cos θ, 2 sin φ, -ρ sin θ), ρ = 5 + 2 cos φ. Turned by 2 rad, the side face holds
    // exactly the points with 0 < θ < 2.
    let p = Profile::circle(Point2::new(5.0, 0.0), 2.0);
    let b = revolve(&Frame::XY, &p, &y_axis(), 2.0).unwrap();
    let f = face_named(&b, "side 0");
    for i in 0..24 {
        for j in 0..12 {
            let theta = 0.05 + i as f64 * (TAU - 0.1) / 23.0;
            let phi = 0.3 + j as f64 * TAU / 12.0;
            let rho = 5.0 + 2.0 * phi.cos();
            let q = Point3::new(rho * theta.cos(), 2.0 * phi.sin(), -rho * theta.sin());
            let want = if theta < 2.0 {
                PointOnFace::Inside
            } else {
                PointOnFace::Outside
            };
            assert_eq!(
                b.classify_point_on_face(f, q).unwrap(),
                want,
                "θ {theta} φ {phi}"
            );
        }
    }
    // On the end circle (θ = 2) and the circle swept by the start vertex (φ = 0).
    let end = Point3::new(3.0 * 2f64.cos(), 0.0, -3.0 * 2f64.sin());
    assert_eq!(
        b.classify_point_on_face(f, end).unwrap(),
        PointOnFace::Boundary
    );
    let path = Point3::new(7.0 * 1f64.cos(), 0.0, -7.0 * 1f64.sin());
    assert_eq!(
        b.classify_point_on_face(f, path).unwrap(),
        PointOnFace::Boundary
    );
}

#[test]
fn a_right_triangle_revolved_is_a_cone() {
    let (r, h) = (3.0, 4.0);
    let p = Profile::polygon(&[Point2::ORIGIN, Point2::new(r, 0.0), Point2::new(0.0, h)]);
    let b = revolve(&Frame::XY, &p, &y_axis(), TAU).unwrap();
    // The apex and the base circle's vertex; the cone's seam and the circle (the base
    // disc keeps no seam, so its centre is no vertex): V = 2, E = 2, F = 2.
    assert_eq!(counts(&b), (2, 2, 2, 0));
    assert_eq!(vertex_names(&b), set(&["axis 2", "vertex 1"]));
    assert_close("volume", b.volume().unwrap(), PI * r * r * h / 3.0);
    // Base π r², lateral π r s with slant s = 5.
    assert_close("area", b.area().unwrap(), PI * r * r + PI * r * 5.0);
    let lateral = face_named(&b, "side 1");
    assert!(matches!(
        b.face_surface(lateral).unwrap(),
        Some((Surface::Cone(_), true))
    ));
    assert_close("lateral", b.face_area(lateral).unwrap(), 15.0 * PI);
    // A quarter turn: a quarter cone with two triangular caps r h / 2.
    let q = revolve(&Frame::XY, &p, &y_axis(), FRAC_PI_2).unwrap();
    // Axis vertices 2, base corner twice: V = 4; edges: 3 start, 2 end, 1 arc: E = 6;
    // faces: 2 caps, disc quarter, cone quarter: F = 4. 4 - 6 + 4 = 2.
    assert_eq!(counts(&q), (4, 6, 4, 0));
    assert_close("volume", q.volume().unwrap(), PI * r * r * h / 12.0);
    let want = (PI * r * r + 15.0 * PI) / 4.0 + r * h;
    assert_close("area", q.area().unwrap(), want);
}

#[test]
fn pappus_holds_for_offset_profiles_and_any_angle() {
    // A slot (stadium) centred 10 from the axis, its long side along the axis direction:
    // area 2·4·12... sides 12 long, radius 2: A = 12·4 + 4π, centroid at its centre.
    let (c, len, rr) = (10.0, 12.0, 2.0);
    let slot = vec![
        ProfileEdge::line(Point2::new(c + rr, 0.0), Point2::new(c + rr, len)),
        ProfileEdge::Arc {
            centre: Point2::new(c, len),
            radius: rr,
            start_angle: 0.0,
            sweep: PI,
        },
        ProfileEdge::line(Point2::new(c - rr, len), Point2::new(c - rr, 0.0)),
        ProfileEdge::Arc {
            centre: Point2::new(c, 0.0),
            radius: rr,
            start_angle: PI,
            sweep: PI,
        },
    ];
    let area = len * 2.0 * rr + PI * rr * rr;
    let p = Profile::new(slot, vec![]);
    assert_close("area", p.area().unwrap(), area);
    for angle in [0.3, 1.0, 2.5, PI, 5.0, TAU] {
        let b = revolve(&Frame::XY, &p, &y_axis(), angle).unwrap();
        // Pappus: V = A · (distance of the centroid) · angle.
        assert_close(
            &format!("slot by {angle}"),
            b.volume().unwrap(),
            area * c * angle,
        );
    }
    // An annulus about (8, 3): a² - b² = 9 - 1, centroid at its centre; partial turns
    // only (a full one would enclose a void).
    let annulus = Profile::circle(Point2::new(8.0, 3.0), 3.0)
        .with_hole(vec![ProfileEdge::circle(Point2::new(8.0, 3.0), 1.0)]);
    for angle in [0.7, 4.0] {
        let b = revolve(&Frame::XY, &annulus, &y_axis(), angle).unwrap();
        assert_eq!(b.validate().unwrap().genus, 1);
        assert_close("annulus", b.volume().unwrap(), 8.0 * PI * 8.0 * angle);
    }
    // The same slot in a tilted frame about a tilted axis gives the same volume.
    let frame = Frame::from_normal_and_x(
        Point3::new(-3.0, 7.0, 1.0),
        Vec3::new(-2.0, 1.0, 4.0),
        Vec3::new(1.0, 1.0, 0.0),
    )
    .unwrap();
    let b = revolve(&frame, &p, &y_axis(), 1.7).unwrap();
    assert_close("tilted", b.volume().unwrap(), area * c * 1.7);
}

#[test]
fn a_circular_segment_beyond_the_axis_centre_makes_a_lemon_torus() {
    // The region between the line x = 1 (y from -1 to 1) and the arc through (1, ±1)
    // centred at (-1, 0), radius √5, which bulges to x = √5 - 1. Its torus has a negative
    // major radius (-1).
    let r = 5f64.sqrt();
    let half = 0.5f64.atan(); // half the central angle: atan(1 / 2)
    let p = Profile::new(
        vec![
            ProfileEdge::Arc {
                centre: Point2::new(-1.0, 0.0),
                radius: r,
                start_angle: -half,
                sweep: 2.0 * half,
            },
            ProfileEdge::line(Point2::new(1.0, 1.0), Point2::new(1.0, -1.0)),
        ],
        vec![],
    );
    // Segment area r²/2 (θ - sin θ), centroid 4 r sin³(θ/2) / (3 (θ - sin θ)) from the
    // circle's centre, θ = 2 atan(1/2).
    let theta = 2.0 * half;
    let area = r * r / 2.0 * (theta - theta.sin());
    let centroid = -1.0 + 4.0 * r * (theta / 2.0).sin().powi(3) / (3.0 * (theta - theta.sin()));
    assert_close("area", p.area().unwrap(), area);
    let b = revolve(&Frame::XY, &p, &y_axis(), TAU).unwrap();
    assert_eq!(counts(&b), (2, 4, 2, 1));
    let torus = face_named(&b, "side 0");
    let Some((Surface::Torus(t), _)) = b.face_surface(torus).unwrap() else {
        panic!("the arc sweeps a torus");
    };
    assert!(close(t.major_radius(), -1.0, 1e-12));
    assert_close("volume", b.volume().unwrap(), area * TAU * centroid);
    // A partial turn too.
    let part = revolve(&Frame::XY, &p, &y_axis(), 2.0).unwrap();
    assert_close("partial", part.volume().unwrap(), area * 2.0 * centroid);
}

#[test]
fn the_profile_may_lie_on_either_side_of_the_axis() {
    // The tube's rectangle on the -x side, and about the axis running -y: same volume.
    let p = Profile::new(rect(-5.0, 0.0, 2.0, 4.0), vec![]);
    let want = PI * (25.0 - 9.0) * 4.0;
    for axis in [y_axis(), Line2::new(Point2::ORIGIN, -Vec2::Y).unwrap()] {
        for angle in [1.0, TAU] {
            let b = revolve(&Frame::XY, &p, &axis, angle).unwrap();
            assert_close("volume", b.volume().unwrap(), want * angle / TAU);
        }
    }
    // Revolving in the XZ plane about its x axis.
    let p = Profile::new(rect(0.0, 2.0, 6.0, 1.0), vec![]);
    let x_axis = Line2::new(Point2::ORIGIN, Vec2::X).unwrap();
    let b = revolve(&Frame::XZ, &p, &x_axis, TAU).unwrap();
    assert_close("volume", b.volume().unwrap(), PI * (9.0 - 4.0) * 6.0);
}

#[test]
fn revolved_bodies_round_trip_through_json() {
    let p = Profile::circle(Point2::new(5.0, 0.0), 2.0);
    for angle in [1.0, TAU] {
        let b = revolve(&Frame::XY, &p, &y_axis(), angle).unwrap();
        let json = serde_json::to_string(&b).unwrap();
        let back: Body = serde_json::from_str(&json).unwrap();
        back.validate().unwrap();
        assert_close("volume", back.volume().unwrap(), b.volume().unwrap());
    }
}

// ---------------------------------------------------------------- hostile input

#[test]
fn bad_sweeps_are_refused_without_panicking() {
    let p = Profile::new(rect(1.0, 0.0, 2.0, 2.0), vec![]);
    let f = Frame::XY;
    for d in [0.0, 1e-9, f64::NAN, f64::INFINITY, -f64::INFINITY, 1e300] {
        assert!(extrude(&f, &p, d).is_err(), "distance {d}");
        assert!(
            extrude_symmetric(&f, &p, d).is_err() || d == 1e300,
            "distance {d}"
        );
    }
    let bowtie = Profile::polygon(&[
        Point2::new(0.0, 0.0),
        Point2::new(1.0, 1.0),
        Point2::new(1.0, 0.0),
        Point2::new(0.0, 1.0),
    ]);
    assert!(extrude(&f, &bowtie, 1.0).is_err());
    let nan = Profile::polygon(&[
        Point2::new(0.0, 0.0),
        Point2::new(f64::NAN, 1.0),
        Point2::new(1.0, 0.0),
    ]);
    assert!(extrude(&f, &nan, 1.0).is_err());
    let far = Profile::new(rect(0.9e6, 0.0, 1.0, 1.0), vec![]);
    assert!(extrude(&f, &far, 1.5e6).is_err());
    let outside = Profile::new(rect(0.0, 0.0, 2.0, 2.0), vec![rect(5.0, 5.0, 1.0, 1.0)]);
    assert!(extrude(&f, &outside, 1.0).is_err());
    assert!(extrude(&f, &Profile::default(), 1.0).is_err());

    let axis = y_axis();
    for angle in [
        0.0,
        1e-12,
        -1e-12,
        TAU + 1e-3,
        -7.0,
        f64::NAN,
        f64::INFINITY,
    ] {
        assert!(revolve(&f, &p, &axis, angle).is_err(), "angle {angle}");
    }
    // Crossing the axis.
    let across = Profile::new(rect(-1.0, 0.0, 2.0, 1.0), vec![]);
    assert!(revolve(&f, &across, &axis, 1.0).is_err());
    // A circle tangent to the axis, an arc touching it inside, and an arc crossing it.
    let tangent = Profile::circle(Point2::new(2.0, 0.0), 2.0);
    assert!(revolve(&f, &tangent, &axis, 1.0).is_err());
    let bulge = Profile::new(
        vec![
            ProfileEdge::line(Point2::new(3.0, -2.0), Point2::new(3.0, 2.0)),
            ProfileEdge::Arc {
                centre: Point2::new(3.0, 0.0),
                radius: 2.0,
                start_angle: FRAC_PI_2,
                sweep: PI,
            },
        ],
        vec![],
    );
    // The half disc left of x = 3 reaches x = 1: fine. One reaching x = 0 touches the
    // axis at (0, 0), inside its arc.
    assert!(revolve(&f, &bulge, &axis, 1.0).is_ok());
    let touching = Profile::new(
        vec![
            ProfileEdge::line(Point2::new(3.0, -3.0), Point2::new(3.0, 3.0)),
            ProfileEdge::Arc {
                centre: Point2::new(3.0, 0.0),
                radius: 3.0,
                start_angle: FRAC_PI_2,
                sweep: PI,
            },
        ],
        vec![],
    );
    assert!(revolve(&f, &touching, &axis, 1.0).is_err());
    // Full turns that would enclose a void or pinch.
    let annulus = Profile::circle(Point2::new(8.0, 0.0), 3.0)
        .with_hole(vec![ProfileEdge::circle(Point2::new(8.0, 0.0), 1.0)]);
    assert!(matches!(
        revolve(&f, &annulus, &axis, TAU),
        Err(KernelError::Unsupported(_))
    ));
    let diamond = Profile::polygon(&[
        Point2::new(0.0, 0.0),
        Point2::new(2.0, -1.0),
        Point2::new(3.0, 0.0),
        Point2::new(2.0, 1.0),
    ]);
    assert!(matches!(
        revolve(&f, &diamond, &axis, TAU),
        Err(KernelError::Unsupported(_))
    ));
    // The same diamond by part of a turn is fine (a cone apex at the axis vertex).
    let ok = revolve(&f, &diamond, &axis, 2.0).unwrap();
    ok.validate().unwrap();
    // Pappus: the kite has area 3 (diagonals 3 and 2) and is two triangles, area 2 with
    // centroid x = 4/3 and area 1 with x = 7/3: centroid x = (8/3 + 7/3) / 3 = 5/3.
    // V = 3 · 5/3 · 2 = 10.
    assert_close("kite", ok.volume().unwrap(), 10.0);
    // A U touching the axis along two separate edges would enclose a void.
    let u = Profile::polygon(&[
        Point2::new(0.0, 0.0),
        Point2::new(3.0, 0.0),
        Point2::new(3.0, 3.0),
        Point2::new(0.0, 3.0),
        Point2::new(0.0, 2.0),
        Point2::new(2.0, 2.0),
        Point2::new(2.0, 1.0),
        Point2::new(0.0, 1.0),
    ]);
    assert!(revolve(&f, &u, &axis, TAU).is_err());
    // By part of a turn it is fine: two hinge edges on the axis. The U is the 3 x 3 square
    // (centroid x 1.5) less the 2 x 1 notch (x 1): area 7, moment 13.5 - 2 = 11.5, so by
    // one radian V = 11.5.
    let part = revolve(&f, &u, &axis, 1.0).unwrap();
    assert_close("U", part.volume().unwrap(), 11.5);
    assert!(edge_names(&part).contains("axis 3") && edge_names(&part).contains("axis 7"));
}

// ---------------------------------------------------------------- tapered extrude

fn deg(a: f64) -> f64 {
    a * PI / 180.0
}

#[test]
fn a_tapered_rectangle_is_the_frustum_of_the_prismatoid() {
    let p = Profile::new(rect(0.0, 0.0, 50.0, 30.0), vec![]);
    let (h, theta) = (10.0, deg(5.0));
    let b = extrude_tapered(&Frame::XY, &p, h, theta).unwrap();
    assert_eq!(counts(&b), (8, 12, 6, 0));
    // Every edge moves in by d = h tan θ at the top, so the section at height z is
    // (50 - 2δ)(30 - 2δ) with δ = z tan θ, quadratic in z: Simpson (the prismatoid
    // formula) is exact. V = h/6 [1500 + 4 (50 - d)(30 - d) + (50 - 2d)(30 - 2d)]
    // = h/6 [9000 - 480 d + 8 d²] = h (1500 - 80 d + 4/3 d²).
    let d = h * theta.tan();
    let want = h * (1500.0 - 80.0 * d + 4.0 / 3.0 * d * d);
    assert_close("volume", b.volume().unwrap(), want);
    let top = face_named(&b, "top");
    assert_close(
        "top",
        b.face_area(top).unwrap(),
        (50.0 - 2.0 * d) * (30.0 - 2.0 * d),
    );
    // A long side leans in by θ: its slant height is h / cos θ, its area the trapezoid
    // (50 + 50 - 2d) / 2 · h / cos θ.
    let side = face_named(&b, "side 0");
    assert_close(
        "side",
        b.face_area(side).unwrap(),
        (50.0 - d) * h / theta.cos(),
    );
    let bb = b.bounding_box().unwrap();
    assert_close("min z", bb.min.z, 0.0);
    assert_close("max z", bb.max.z, 10.0);
    assert_eq!(
        face_names(&b),
        face_names(&extrude(&Frame::XY, &p, h).unwrap())
    );
    // Downward the same frustum, mirrored; a negative angle grows the profile instead.
    let down = extrude_tapered(&Frame::XY, &p, -h, theta).unwrap();
    assert_close("down", down.volume().unwrap(), want);
    assert_close("down min z", down.bounding_box().unwrap().min.z, -10.0);
    let grown = extrude_tapered(&Frame::XY, &p, h, -theta).unwrap();
    assert_close(
        "grown",
        grown.volume().unwrap(),
        h * (1500.0 + 80.0 * d + 4.0 / 3.0 * d * d),
    );
}

#[test]
fn a_tapered_circle_is_a_cone_frustum() {
    let p = Profile::circle(Point2::ORIGIN, 5.0);
    let (h, theta) = (10.0, deg(10.0));
    let b = extrude_tapered(&Frame::XY, &p, h, theta).unwrap();
    assert_eq!(counts(&b), (2, 3, 3, 0));
    // r₂ = r₁ - h tan θ; V = π h / 3 (r₁² + r₁ r₂ + r₂²).
    let (r1, r2) = (5.0, 5.0 - h * theta.tan());
    assert_close(
        "volume",
        b.volume().unwrap(),
        PI * h / 3.0 * (r1 * r1 + r1 * r2 + r2 * r2),
    );
    // The side is a cone: lateral area π (r₁ + r₂) s with slant s = h / cos θ.
    let side = face_named(&b, "side 0");
    assert!(matches!(
        b.face_surface(side).unwrap(),
        Some((Surface::Cone(_), _))
    ));
    assert_close(
        "side",
        b.face_area(side).unwrap(),
        PI * (r1 + r2) * h / theta.cos(),
    );
}

#[test]
fn a_tapered_plate_with_a_hole_opens_the_hole_as_it_closes_the_outside() {
    let hole = vec![ProfileEdge::circle(Point2::new(25.0, 15.0), 5.0)];
    let p = Profile::new(rect(0.0, 0.0, 50.0, 30.0), vec![hole]);
    let (h, theta) = (10.0, deg(3.0));
    let b = extrude_tapered(&Frame::XY, &p, h, theta).unwrap();
    assert_eq!(counts(&b), (10, 15, 7, 1));
    // The hole's radius grows to 5 + δ: π ∫₀ʰ (5 + z tan θ)² dz = π h (25 + 5 d + d²/3).
    let d = h * theta.tan();
    let outer = h * (1500.0 - 80.0 * d + 4.0 / 3.0 * d * d);
    let hole = PI * h * (25.0 + 5.0 * d + d * d / 3.0);
    assert_close("volume", b.volume().unwrap(), outer - hole);
}

#[test]
fn a_tapered_slot_keeps_its_tangent_ends() {
    let slot = vec![
        ProfileEdge::line(Point2::new(0.0, -5.0), Point2::new(20.0, -5.0)),
        ProfileEdge::Arc {
            centre: Point2::new(20.0, 0.0),
            radius: 5.0,
            start_angle: -FRAC_PI_2,
            sweep: PI,
        },
        ProfileEdge::line(Point2::new(20.0, 5.0), Point2::new(0.0, 5.0)),
        ProfileEdge::Arc {
            centre: Point2::new(0.0, 0.0),
            radius: 5.0,
            start_angle: FRAC_PI_2,
            sweep: PI,
        },
    ];
    let (h, theta) = (10.0, deg(8.0));
    let b = extrude_tapered(&Frame::XY, &Profile::new(slot, vec![]), h, theta).unwrap();
    assert_eq!(counts(&b), (8, 12, 6, 0));
    // The section with half-width w = 5 - δ is 20 · 2w + π w², quadratic in δ, so Simpson
    // is exact over the height.
    let d = h * theta.tan();
    let area = |delta: f64| {
        let w = 5.0 - delta;
        40.0 * w + PI * w * w
    };
    let want = h / 6.0 * (area(0.0) + 4.0 * area(d / 2.0) + area(d));
    assert_close("volume", b.volume().unwrap(), want);
}

#[test]
fn tapers_that_vanish_overlap_or_need_conic_edges_are_refused() {
    let f = Frame::XY;
    let plate = Profile::new(rect(0.0, 0.0, 50.0, 30.0), vec![]);
    // d = 10 tan 60° ≈ 17.3 > 15: the 30 mm side closes up before the top.
    assert!(extrude_tapered(&f, &plate, 10.0, deg(60.0)).is_err());
    // r₂ = 5 - 10 tan 30° < 0.
    let disc = Profile::circle(Point2::ORIGIN, 5.0);
    assert!(extrude_tapered(&f, &disc, 10.0, deg(30.0)).is_err());
    // The hole grows and the outside shrinks by d each: they meet when 2d ≥ 10 (the hole
    // is 10 from the long sides), here d = 10 tan 30° ≈ 5.77.
    let holed = Profile::new(
        rect(0.0, 0.0, 50.0, 30.0),
        vec![vec![ProfileEdge::circle(Point2::new(25.0, 15.0), 5.0)]],
    );
    assert!(extrude_tapered(&f, &holed, 10.0, deg(30.0)).is_err());
    // A half disc has sharp corners where its diameter meets the arc: their struts would
    // be conics.
    let half = Profile::new(
        vec![
            ProfileEdge::line(Point2::new(5.0, 0.0), Point2::new(-5.0, 0.0)),
            ProfileEdge::Arc {
                centre: Point2::ORIGIN,
                radius: 5.0,
                start_angle: PI,
                sweep: PI,
            },
        ],
        vec![],
    );
    assert!(matches!(
        extrude_tapered(&f, &half, 10.0, deg(2.0)),
        Err(KernelError::Unsupported(_))
    ));
    // Without a taper it is a plain extrusion.
    assert!(extrude_tapered(&f, &half, 10.0, 0.0).is_ok());
    for t in [f64::NAN, f64::INFINITY, deg(89.5), -deg(90.0), 1e300] {
        assert!(extrude_tapered(&f, &plate, 10.0, t).is_err(), "taper {t}");
    }
    for d in [0.0, f64::NAN, 1e300] {
        assert!(
            extrude_tapered(&f, &plate, d, deg(5.0)).is_err(),
            "distance {d}"
        );
    }
}
