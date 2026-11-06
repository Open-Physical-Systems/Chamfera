//! Tests for K2 topology. Expected values are derived by hand in the comments.

use super::*;
use chamfera_geom::{Point2, Vec3};
use std::f64::consts::{PI, TAU};

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn counts(b: &Body) -> (usize, usize, usize, usize) {
    let s = b.summary();
    (s.vertices, s.edges, s.faces, s.loops)
}

/// Validates the topology and checks Euler-Poincare by hand from the counts:
/// V - E + F - (L - F) = 2 (S - G).
fn valid(b: &Body, genus: usize) -> TopologyReport {
    let r = b.validate_topology().unwrap();
    let s = r.summary;
    let lhs =
        s.vertices as i64 - s.edges as i64 + s.faces as i64 - (s.loops as i64 - s.faces as i64);
    assert_eq!(lhs, 2 * (s.shells as i64 - genus as i64));
    assert_eq!(r.genus, genus);
    assert_eq!(r.euler_characteristic(), lhs);
    r
}

fn square(x0: f64, y0: f64, side: f64) -> Vec<Point2> {
    vec![
        Point2::new(x0, y0),
        Point2::new(x0 + side, y0),
        Point2::new(x0 + side, y0 + side),
        Point2::new(x0, y0 + side),
    ]
}

fn face_named(b: &Body, name: &str) -> FaceId {
    b.faces()
        .into_iter()
        .find(|f| b.face_name(*f).unwrap() == Some(name))
        .unwrap()
}

fn outward_normal(b: &Body, f: FaceId) -> Vec3 {
    let (Surface::Plane(frame), same) = b.face_surface(f).unwrap().unwrap();
    if same { frame.z() } else { -frame.z() }
}

// ---------------------------------------------------------------- Euler operators

#[test]
fn mvfs_makes_a_valid_skeletal_body() {
    let mut b = Body::new();
    let m = b.mvfs(Point3::new(1.0, 2.0, 3.0)).unwrap();
    // V=1, E=0, F=1, L=1, S=1: 1 - 0 + 1 - 0 = 2 = 2(1 - 0).
    assert_eq!(counts(&b), (1, 0, 1, 1));
    valid(&b, 0);
    assert_eq!(
        b.vertex_point(m.vertex).unwrap(),
        Point3::new(1.0, 2.0, 3.0)
    );
    assert_eq!(b.vertex_fan(m.vertex).unwrap(), vec![m.half_edge]);
    assert_eq!(b.half_edge_edge(m.half_edge).unwrap(), None);
}

#[test]
fn mev_mef_kef_kev_kvfs_build_and_unbuild_a_square_lamina() {
    let mut b = Body::new();
    let m = b.mvfs(Point3::new(0.0, 0.0, 0.0)).unwrap();
    let pts = [
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(1.0, 1.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ];
    let mut at = m.half_edge;
    let mut edges = Vec::new();
    let mut verts = vec![m.vertex];
    for (i, p) in pts.iter().enumerate() {
        let (e, v) = b.mev(at, *p).unwrap();
        // Each MEV adds one vertex and one edge: V = i + 2, E = i + 1, still F = L = 1.
        assert_eq!(counts(&b), (i + 2, i + 1, 1, 1));
        valid(&b, 0);
        assert_eq!(b.edge_vertices(e).unwrap(), [verts[i], v]);
        at = b.edge_half_edges(e).unwrap()[1];
        edges.push(e);
        verts.push(v);
    }
    let first = b.edge_half_edges(edges[0]).unwrap()[0];
    let (closing, f2) = b.mef(at, first).unwrap();
    // A square lamina: V=4, E=4, F=2, L=2: 4 - 4 + 2 - 0 = 2.
    assert_eq!(counts(&b), (4, 4, 2, 2));
    valid(&b, 0);
    let [fa, fb] = b.edge_faces(closing).unwrap();
    assert_eq!(fb, f2);
    assert_eq!(fa, m.face);
    // The two faces are bounded by the same four edges, run in opposite directions.
    assert_eq!(b.face_edges(m.face).unwrap().len(), 4);
    assert_eq!(b.face_edges(f2).unwrap().len(), 4);
    let ring_a = b.loop_vertices(b.face_outer_loop(m.face).unwrap()).unwrap();
    let ring_b = b.loop_vertices(b.face_outer_loop(f2).unwrap()).unwrap();
    assert_eq!(ring_a, vec![verts[3], verts[0], verts[1], verts[2]]);
    assert_eq!(ring_b, vec![verts[0], verts[3], verts[2], verts[1]]);
    // Each corner of the lamina has two edges.
    for v in &verts {
        assert_eq!(b.vertex_edges(*v).unwrap().len(), 2);
        assert_eq!(b.vertex_faces(*v).unwrap().len(), 2);
    }

    b.kef(closing).unwrap();
    assert_eq!(counts(&b), (4, 3, 1, 1));
    valid(&b, 0);
    for (e, v) in edges.iter().zip(&verts[1..]).rev() {
        b.kev(*e, *v).unwrap();
        valid(&b, 0);
    }
    assert_eq!(counts(&b), (1, 0, 1, 1));
    b.kvfs(m.shell).unwrap();
    assert_eq!(b.summary(), TopologySummary::default());
    valid(&b, 0);
}

#[test]
fn a_closed_edge_on_one_vertex_splits_its_face() {
    // MEF on a one-vertex loop makes a closed edge (a circle's topology): V=1, E=1, F=2,
    // L=2: 1 - 1 + 2 - 0 = 2.
    let mut b = Body::new();
    let m = b.mvfs(Point3::ORIGIN).unwrap();
    let (e, f) = b.mef(m.half_edge, m.half_edge).unwrap();
    assert_eq!(counts(&b), (1, 1, 2, 2));
    valid(&b, 0);
    assert_eq!(b.edge_vertices(e).unwrap(), [m.vertex, m.vertex]);
    assert_eq!(b.edge_faces(e).unwrap(), [m.face, f]);
    b.kef(e).unwrap();
    assert_eq!(counts(&b), (1, 0, 1, 1));
    valid(&b, 0);
}

#[test]
fn kemr_and_mekr_make_and_remove_a_ring() {
    let mut b = Body::cuboid(Point3::ORIGIN, Vec3::new(10.0, 10.0, 10.0)).unwrap();
    let top = face_named(&b, "top");
    let outer = b.face_outer_loop(top).unwrap();
    let start = b.loop_half_edges(outer).unwrap()[0];
    let (bridge, w0) = b.mev(start, Point3::new(4.0, 4.0, 10.0)).unwrap();
    let [out, back] = b.edge_half_edges(bridge).unwrap();
    let (e1, _) = b.mev(back, Point3::new(6.0, 4.0, 10.0)).unwrap();
    valid(&b, 0);
    // Two MEVs add a bridge and a strut to the top face: V = 8 + 2 = 10, E = 12 + 2 = 14.
    // KEMR on the bridge leaves the strut as a ring of the top face: E = 13, L = 7,
    // one ring: 10 - 13 + 6 - 1 = 2.
    let ring = b.kemr(out).unwrap();
    assert_eq!(counts(&b), (10, 13, 6, 7));
    let r = valid(&b, 0);
    assert_eq!(r.summary.rings, 1);
    assert_eq!(b.face_inner_loops(top).unwrap(), vec![ring]);
    assert_eq!(b.loop_face(ring).unwrap(), top);
    let ring_vertices = b.loop_vertices(ring).unwrap();
    assert_eq!(ring_vertices.len(), 2);
    assert!(ring_vertices.contains(&w0));
    // MEKR reconnects the ring to the outer loop.
    let from_ring = b.edge_half_edges(e1).unwrap()[0];
    let back_bridge = b.mekr(start, from_ring).unwrap();
    assert_eq!(counts(&b), (10, 14, 6, 6));
    valid(&b, 0);
    assert!(b.face_inner_loops(top).unwrap().is_empty());
    assert_eq!(b.edge_faces(back_bridge).unwrap(), [top, top]);
}

#[test]
fn kemr_on_a_strut_tip_leaves_a_one_vertex_ring() {
    let mut b = Body::cuboid(Point3::ORIGIN, Vec3::new(10.0, 10.0, 10.0)).unwrap();
    let top = face_named(&b, "top");
    let start = b.loop_half_edges(b.face_outer_loop(top).unwrap()).unwrap()[0];
    let (bridge, w0) = b.mev(start, Point3::new(5.0, 5.0, 10.0)).unwrap();
    let out = b.edge_half_edges(bridge).unwrap()[0];
    let ring = b.kemr(out).unwrap();
    // V=9, E=12, F=6, L=7: 9 - 12 + 6 - 1 = 2.
    assert_eq!(counts(&b), (9, 12, 6, 7));
    valid(&b, 0);
    assert_eq!(b.loop_vertices(ring).unwrap(), vec![w0]);
    // The lone vertex can join the outer loop again with MEKR.
    let lone = b.loop_half_edges(ring).unwrap()[0];
    b.mekr(start, lone).unwrap();
    assert_eq!(counts(&b), (9, 13, 6, 6));
    valid(&b, 0);
}

#[test]
fn mfkrh_and_kfmrh_open_and_close_a_through_hole() {
    let mut b = Body::prism_with_holes(
        &Frame::XY,
        &square(0.0, 0.0, 20.0),
        &[square(5.0, 5.0, 10.0)],
        20.0,
    )
    .unwrap();
    valid(&b, 1);
    let bottom = face_named(&b, "bottom");
    let ring = b.face_inner_loops(bottom).unwrap()[0];
    let cap = b.mfkrh(ring).unwrap();
    // One more face, one less ring: 16 - 24 + 11 - 1 = 2 = 2(1 - 0).
    assert_eq!(counts(&b), (16, 24, 11, 12));
    valid(&b, 0);
    assert!(b.face_inner_loops(bottom).unwrap().is_empty());
    b.kfmrh(bottom, cap).unwrap();
    assert_eq!(counts(&b), (16, 24, 10, 12));
    valid(&b, 1);
}

#[test]
fn kfmrh_between_two_shells_merges_them() {
    let mut b = Body::new();
    let m1 = b.mvfs(Point3::ORIGIN).unwrap();
    let m2 = b.mvfs(Point3::new(5.0, 0.0, 0.0)).unwrap();
    // Two skeletal shells: 2 - 0 + 2 - 0 = 4 = 2(2 - 0).
    valid(&b, 0);
    b.kfmrh(m1.face, m2.face).unwrap();
    // One shell whose face has an outer loop and a ring: 2 - 0 + 1 - 1 = 2 = 2(1 - 0).
    assert_eq!(b.summary().shells, 1);
    assert_eq!(counts(&b), (2, 0, 1, 2));
    valid(&b, 0);
}

#[test]
fn failing_operators_leave_the_body_unchanged() {
    let mut b = Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 2.0, 3.0)).unwrap();
    let before = b.clone();
    let top = face_named(&b, "top");
    let bottom = face_named(&b, "bottom");
    let ht = b.loop_half_edges(b.face_outer_loop(top).unwrap()).unwrap()[0];
    let hb = b
        .loop_half_edges(b.face_outer_loop(bottom).unwrap())
        .unwrap()[0];
    let e = b.edges()[0];
    let [v0, _] = b.edge_vertices(e).unwrap();

    assert!(b.mef(ht, hb).is_err(), "MEF across two loops");
    assert!(b.kemr(ht).is_err(), "KEMR on an edge between two faces");
    assert!(b.kev(e, v0).is_err(), "KEV on a vertex with three edges");
    assert!(b.mekr(ht, hb).is_err(), "MEKR across two faces");
    assert!(b.kvfs(b.shells()[0]).is_err(), "KVFS on a solid");
    assert!(b.kfmrh(top, top).is_err(), "KFMRH on one face");
    assert!(
        b.mfkrh(b.face_outer_loop(top).unwrap()).is_err(),
        "MFKRH on an outer loop"
    );
    assert!(
        b.mev(ht, Point3::new(f64::NAN, 0.0, 0.0)).is_err(),
        "MEV to NaN"
    );
    assert!(
        b.mev(ht, Point3::new(2e6, 0.0, 0.0)).is_err(),
        "MEV out of range"
    );
    assert!(b.mvfs(Point3::new(0.0, f64::INFINITY, 0.0)).is_err());
    assert_eq!(b, before);
    b.validate().unwrap();
}

#[test]
fn kef_on_an_edge_inside_one_face_is_refused() {
    let mut b = Body::cuboid(Point3::ORIGIN, Vec3::new(10.0, 10.0, 10.0)).unwrap();
    let top = face_named(&b, "top");
    let start = b.loop_half_edges(b.face_outer_loop(top).unwrap()).unwrap()[0];
    let (strut, _) = b.mev(start, Point3::new(5.0, 5.0, 10.0)).unwrap();
    let before = b.clone();
    assert!(b.kef(strut).is_err());
    assert_eq!(b, before);
}

#[test]
fn unknown_ids_are_errors_not_panics() {
    let mut b = Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 1.0, 1.0)).unwrap();
    let before = b.clone();
    let he = HalfEdgeId(9_999);
    let e = EdgeId(9_999);
    let f = FaceId(9_999);
    let v = VertexId(9_999);
    assert!(b.mev(he, Point3::ORIGIN).is_err());
    assert!(b.mef(he, he).is_err());
    assert!(b.kef(e).is_err());
    assert!(b.kev(e, v).is_err());
    assert!(b.kemr(he).is_err());
    assert!(b.mekr(he, he).is_err());
    assert!(b.kfmrh(f, FaceId(0)).is_err());
    assert!(b.mfkrh(LoopId(9_999)).is_err());
    assert!(b.kvfs(ShellId(9_999)).is_err());
    assert!(b.face_area(f).is_err());
    assert!(b.vertex_fan(v).is_err());
    assert!(b.edge_faces(e).is_err());
    assert!(b.set_edge_line(e).is_err());
    assert!(
        b.set_face_name(FaceId(0), &"x".repeat(MAX_NAME_LENGTH + 1))
            .is_err()
    );
    assert_eq!(b, before);
}

// ---------------------------------------------------------------- builders and measures

#[test]
fn box_50_30_10_has_the_counts_volume_and_area_of_a_cuboid() {
    let b = Body::cuboid(Point3::ORIGIN, Vec3::new(50.0, 30.0, 10.0)).unwrap();
    // V=8, E=12, F=6, L=6: 8 - 12 + 6 - 0 = 2 = 2(1 - 0).
    assert_eq!(counts(&b), (8, 12, 6, 6));
    let r = b.validate().unwrap();
    assert_eq!(r.genus, 0);
    valid(&b, 0);
    // Volume 50 * 30 * 10 = 15000; area 2(50*30 + 50*10 + 30*10) = 2 * 2300 = 4600.
    assert!(close(b.volume().unwrap(), 15_000.0, 1e-9));
    assert!(close(b.area().unwrap(), 4_600.0, 1e-9));
    let bottom = face_named(&b, "bottom");
    let top = face_named(&b, "top");
    assert!(close(b.face_area(bottom).unwrap(), 1_500.0, 1e-9));
    assert!(close(b.face_area(top).unwrap(), 1_500.0, 1e-9));
    assert!(close(outward_normal(&b, bottom).z, -1.0, 1e-15));
    assert!(close(outward_normal(&b, top).z, 1.0, 1e-15));
    // Side 0 is swept from the first profile edge (0,0)-(50,0): outward normal -Y, area 500.
    let side0 = face_named(&b, "side 0");
    assert!(close(outward_normal(&b, side0).y, -1.0, 1e-15));
    assert!(close(b.face_area(side0).unwrap(), 500.0, 1e-9));
    let bb = b.bounding_box().unwrap();
    assert_eq!(bb.min, Point3::ORIGIN);
    assert_eq!(bb.max, Point3::new(50.0, 30.0, 10.0));
}

#[test]
fn box_queries_see_three_faces_and_edges_at_every_corner() {
    let b = Body::cuboid(Point3::new(-1.0, -2.0, -3.0), Vec3::new(2.0, 4.0, 6.0)).unwrap();
    assert_eq!(b.faces().len(), 6);
    assert_eq!(b.shell_faces(b.shells()[0]).unwrap().len(), 6);
    for f in b.faces() {
        // Every box face is a quadrilateral.
        assert_eq!(b.face_edges(f).unwrap().len(), 4);
        assert_eq!(b.face_shell(f).unwrap(), b.shells()[0]);
    }
    for e in b.edges() {
        let [f, g] = b.edge_faces(e).unwrap();
        assert_ne!(f, g);
        // Adjacent box faces are perpendicular.
        assert!(close(
            outward_normal(&b, f).dot(outward_normal(&b, g)),
            0.0,
            1e-15
        ));
        let [h0, h1] = b.edge_half_edges(e).unwrap();
        assert_eq!(b.half_edge_mate(h0).unwrap(), Some(h1));
        assert_eq!(
            b.half_edge_origin(h0).unwrap(),
            b.half_edge_end(h1).unwrap()
        );
    }
    for v in b.vertices() {
        let fan = b.vertex_fan(v).unwrap();
        assert_eq!(fan.len(), 3);
        assert!(fan.iter().all(|h| b.half_edge_origin(*h).unwrap() == v));
        assert_eq!(b.vertex_edges(v).unwrap().len(), 3);
        assert_eq!(b.vertex_faces(v).unwrap().len(), 3);
    }
    // Volume 2 * 4 * 6 = 48 wherever the box is.
    assert!(close(b.volume().unwrap(), 48.0, 1e-12));
}

#[test]
fn hexagonal_prism_matches_the_closed_forms() {
    let (s, h) = (7.0, 3.0);
    let hex: Vec<Point2> = (0..6)
        .map(|k| {
            let a = f64::from(k) * PI / 3.0;
            Point2::new(s * a.cos(), s * a.sin())
        })
        .collect();
    let b = Body::prism(&Frame::XY, &hex, h).unwrap();
    // n = 6: V = 2n = 12, E = 3n = 18, F = n + 2 = 8: 12 - 18 + 8 = 2.
    assert_eq!(counts(&b), (12, 18, 8, 8));
    b.validate().unwrap();
    valid(&b, 0);
    // A regular hexagon of side s is six equilateral triangles: area (3√3/2) s².
    let hex_area = 1.5 * 3f64.sqrt() * s * s;
    assert!(close(b.volume().unwrap(), hex_area * h, 1e-9));
    // Two caps plus six s x h rectangles.
    assert!(close(b.area().unwrap(), 2.0 * hex_area + 6.0 * s * h, 1e-9));
    for f in b.faces() {
        let n = b.face_edges(f).unwrap().len();
        let name = b.face_name(f).unwrap().unwrap();
        assert_eq!(
            n,
            if name == "top" || name == "bottom" {
                6
            } else {
                4
            }
        );
    }
}

#[test]
fn a_clockwise_or_tilted_profile_gives_the_same_prism() {
    // A 3-4-5 right triangle: area 6, extruded 2 along a tilted frame's normal: volume 12.
    let mut tri = vec![
        Point2::new(0.0, 0.0),
        Point2::new(3.0, 0.0),
        Point2::new(0.0, 4.0),
    ];
    tri.reverse();
    let frame = Frame::from_normal(Point3::new(10.0, -5.0, 2.0), Vec3::new(1.0, 2.0, 2.0)).unwrap();
    let b = Body::prism(&frame, &tri, 2.0).unwrap();
    b.validate().unwrap();
    assert_eq!(counts(&b), (6, 9, 5, 5));
    assert!(close(b.volume().unwrap(), 12.0, 1e-9));
    // Caps 2 * 6, sides (3 + 4 + 5) * 2 = 24: area 36.
    assert!(close(b.area().unwrap(), 36.0, 1e-9));
}

#[test]
fn cube_with_a_square_through_hole_has_genus_one() {
    let b = Body::prism_with_holes(
        &Frame::XY,
        &square(0.0, 0.0, 20.0),
        &[square(5.0, 5.0, 10.0)],
        20.0,
    )
    .unwrap();
    // Counting: V = 8 outer + 4 + 4 hole corners = 16; E = 12 cube edges + 4 + 4 hole
    // rims + 4 hole edges along z = 24; F = 6 cube faces + 4 hole walls = 10; L = 10 outer
    // loops + 2 rings (top and bottom) = 12. 16 - 24 + 10 - 2 = 0 = 2(S - G) = 2(1 - 1).
    assert_eq!(counts(&b), (16, 24, 10, 12));
    let r = b.validate().unwrap();
    assert_eq!(r.genus, 1);
    assert_eq!(r.summary.rings, 2);
    valid(&b, 1);
    // Volume 20³ - 10² * 20 = 8000 - 2000 = 6000.
    assert!(close(b.volume().unwrap(), 6_000.0, 1e-9));
    // Area: cube 6 * 400 = 2400, minus the two 10 x 10 openings, plus four 10 x 20 walls:
    // 2400 - 200 + 800 = 3000.
    assert!(close(b.area().unwrap(), 3_000.0, 1e-9));
    let top = face_named(&b, "top");
    assert!(close(b.face_area(top).unwrap(), 300.0, 1e-9));
    assert_eq!(b.face_inner_loops(top).unwrap().len(), 1);
    assert_eq!(b.face_edges(top).unwrap().len(), 8);
    // Hole walls face into the hole: wall 0 is swept from the hole edge (5,5)-(15,5), so
    // its outward normal is +Y.
    let wall = face_named(&b, "hole 0 side 0");
    assert!(close(outward_normal(&b, wall).y, 1.0, 1e-15));
    assert!(close(b.face_area(wall).unwrap(), 200.0, 1e-9));
}

#[test]
fn a_plate_with_two_holes_has_genus_two() {
    // 40 x 20 x 5 plate, a 5 x 5 square hole and a triangle hole of legs 6 and 4.
    let tri = vec![
        Point2::new(25.0, 5.0),
        Point2::new(25.0, 9.0),
        Point2::new(31.0, 5.0),
    ];
    let outer = vec![
        Point2::new(0.0, 0.0),
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 20.0),
        Point2::new(0.0, 20.0),
    ];
    let b = Body::prism_with_holes(&Frame::XY, &outer, &[square(5.0, 5.0, 5.0), tri], 5.0).unwrap();
    // n = 4 + 4 + 3 = 11 profile points: V = 22, E = 33, F = 4 + 2 + 7 = 13, L = 13 + 4.
    // 22 - 33 + 13 - 4 = -2 = 2(1 - 2).
    assert_eq!(counts(&b), (22, 33, 13, 17));
    let r = b.validate().unwrap();
    assert_eq!(r.genus, 2);
    // Volume (800 - 25 - 12) * 5 = 3815.
    assert!(close(b.volume().unwrap(), 3_815.0, 1e-9));
}

#[test]
fn cylinder_topology_has_two_circles_and_a_seam() {
    let (r, h) = (5.0, 10.0);
    let b = Body::cylinder_topology(&Frame::XY, r, h).unwrap();
    // V=2, E=3, F=3, L=3: 2 - 3 + 3 - 0 = 2.
    assert_eq!(counts(&b), (2, 3, 3, 3));
    valid(&b, 0);
    let lateral = face_named(&b, "lateral");
    let bottom = face_named(&b, "bottom");
    let top = face_named(&b, "top");
    // The lateral face waits for K3's cylinder surface.
    assert_eq!(b.validate(), Err(TopologyError::MissingSurface(lateral)));
    // Caps are discs bounded by one closed edge: area πr² exactly from the arc formula.
    assert!(close(b.face_area(bottom).unwrap(), PI * r * r, 1e-12));
    assert!(close(b.face_area(top).unwrap(), PI * r * r, 1e-12));
    assert!(close(outward_normal(&b, bottom).z, -1.0, 1e-15));
    assert!(close(outward_normal(&b, top).z, 1.0, 1e-15));
    // The seam is used twice by the lateral face; each circle joins the lateral face to
    // a cap.
    let seam = b
        .edges()
        .into_iter()
        .find(|e| b.edge_faces(*e).unwrap() == [lateral, lateral])
        .unwrap();
    let [v0, v1] = b.edge_vertices(seam).unwrap();
    assert_eq!(b.vertex_point(v0).unwrap(), Point3::new(r, 0.0, 0.0));
    assert_eq!(b.vertex_point(v1).unwrap(), Point3::new(r, 0.0, h));
    assert_eq!(b.face_edges(lateral).unwrap().len(), 3);
    for e in b.edges().into_iter().filter(|e| *e != seam) {
        let (c, t0, t1) = b.edge_curve(e).unwrap().unwrap();
        assert!(matches!(c, Curve::Circle(_)));
        assert!(close(t1 - t0, TAU, 0.0));
    }
    // Box of a circle of radius 5 about Z, from z = 0 to 10.
    let bb = b.bounding_box().unwrap();
    assert!(bb.min.coincident(Point3::new(-r, -r, 0.0)));
    assert!(bb.max.coincident(Point3::new(r, r, h)));
}

#[test]
fn a_box_far_from_the_origin_keeps_its_volume() {
    let b = Body::cuboid(Point3::new(1e5, -2e5, 3e5), Vec3::new(50.0, 30.0, 10.0)).unwrap();
    b.validate().unwrap();
    assert!(close(b.volume().unwrap(), 15_000.0, 1e-6));
    assert!(close(b.area().unwrap(), 4_600.0, 1e-6));
}

#[test]
fn near_degenerate_sizes_around_the_tolerance() {
    // 1e-6 mm is ten tolerances: a valid (tiny) box of volume 1e-18.
    let b = Body::cuboid(Point3::ORIGIN, Vec3::new(1e-6, 1e-6, 1e-6)).unwrap();
    b.validate().unwrap();
    assert!(close(b.volume().unwrap(), 1e-18, 1e-30));
    // Below the tolerance the box is refused.
    assert!(Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 1.0, LINEAR_TOLERANCE / 2.0)).is_err());
    assert!(Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 0.0, 1.0)).is_err());
    assert!(Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, -1.0, 1.0)).is_err());
}

#[test]
fn bad_profiles_are_refused() {
    let f = Frame::XY;
    let bowtie = [
        Point2::new(0.0, 0.0),
        Point2::new(1.0, 1.0),
        Point2::new(1.0, 0.0),
        Point2::new(0.0, 1.0),
    ];
    assert!(Body::prism(&f, &bowtie, 1.0).is_err());
    assert!(Body::prism(&f, &[Point2::new(0.0, 0.0), Point2::new(1.0, 0.0)], 1.0).is_err());
    let collinear = [
        Point2::new(0.0, 0.0),
        Point2::new(1.0, 0.0),
        Point2::new(2.0, 0.0),
    ];
    assert!(Body::prism(&f, &collinear, 1.0).is_err());
    let spike = [
        Point2::new(0.0, 0.0),
        Point2::new(2.0, 0.0),
        Point2::new(1.0, 0.0),
        Point2::new(1.0, 1.0),
    ];
    assert!(Body::prism(&f, &spike, 1.0).is_err());
    let dup = [
        Point2::new(0.0, 0.0),
        Point2::new(0.0, 0.0),
        Point2::new(1.0, 0.0),
        Point2::new(0.0, 1.0),
    ];
    assert!(Body::prism(&f, &dup, 1.0).is_err());
    let nan = [
        Point2::new(0.0, 0.0),
        Point2::new(f64::NAN, 0.0),
        Point2::new(0.0, 1.0),
    ];
    assert!(Body::prism(&f, &nan, 1.0).is_err());
    let sq = square(0.0, 0.0, 10.0);
    for hgt in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(Body::prism(&f, &sq, hgt).is_err());
    }
    // Holes outside, touching the outline, overlapping each other or nested.
    let bad_holes = [
        vec![square(20.0, 20.0, 1.0)],
        vec![square(0.0, 2.0, 1.0)],
        vec![square(2.0, 2.0, 3.0), square(4.0, 4.0, 3.0)],
        vec![square(2.0, 2.0, 6.0), square(3.0, 3.0, 1.0)],
        vec![vec![Point2::new(1.0, 1.0)]],
    ];
    for holes in bad_holes {
        assert!(Body::prism_with_holes(&f, &sq, &holes, 1.0).is_err());
    }
    let too_many: Vec<Point2> = (0..=MAX_PROFILE_VERTICES)
        .map(|k| {
            let a = TAU * k as f64 / (MAX_PROFILE_VERTICES + 1) as f64;
            Point2::new(a.cos(), a.sin())
        })
        .collect();
    assert!(Body::prism(&f, &too_many, 1.0).is_err());
    assert!(Body::cylinder_topology(&f, 0.0, 1.0).is_err());
    assert!(Body::cylinder_topology(&f, 1.0, -1.0).is_err());
}

#[test]
fn geometry_setters_check_their_input() {
    let mut b = Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 1.0, 1.0)).unwrap();
    let before = b.clone();
    let e = b.edges()[0];
    let far = crate::Segment3::new(Point3::new(5.0, 5.0, 5.0), Point3::new(6.0, 5.0, 5.0)).unwrap();
    assert!(b.set_edge_curve(e, Curve::Segment(far), 0.0, 1.0).is_err());
    let [v0, v1] = b.edge_vertices(e).unwrap();
    let seg =
        crate::Segment3::new(b.vertex_point(v0).unwrap(), b.vertex_point(v1).unwrap()).unwrap();
    assert!(b.set_edge_curve(e, Curve::Segment(seg), 0.0, 2.0).is_err());
    assert!(b.set_edge_curve(e, Curve::Segment(seg), 1.0, 0.0).is_err());
    assert!(
        b.set_edge_curve(e, Curve::Segment(seg), f64::NAN, 1.0)
            .is_err()
    );
    assert_eq!(b, before);
}

// ---------------------------------------------------------------- corrupted topology

fn good_box() -> Body {
    Body::cuboid(Point3::ORIGIN, Vec3::new(50.0, 30.0, 10.0)).unwrap()
}

#[test]
fn a_dangling_coedge_fails_validation() {
    let mut b = good_box();
    let h = b.edge_half_edges(EdgeId(3)).unwrap()[0];
    b.half_edges.get_mut(h.index()).unwrap().next = HalfEdgeId(123_456);
    assert!(matches!(
        b.validate_topology(),
        Err(TopologyError::Dangling { .. })
    ));
    assert!(b.validate().is_err());

    let mut b = good_box();
    b.half_edges
        .remove(b.edge_half_edges(EdgeId(5)).unwrap()[1].index());
    assert!(b.validate_topology().is_err());

    // A half-edge whose next skips one: the skipped half-edge belongs to no cycle.
    let mut b = good_box();
    let h = b
        .loop_half_edges(b.face_outer_loop(FaceId(0)).unwrap())
        .unwrap()[0];
    let skip = b.half_edge_next(b.half_edge_next(h).unwrap()).unwrap();
    b.half_edges.get_mut(h.index()).unwrap().next = skip;
    b.half_edges.get_mut(skip.index()).unwrap().prev = h;
    assert!(b.validate_topology().is_err());
}

#[test]
fn a_loop_that_never_closes_fails_without_hanging() {
    let mut b = good_box();
    // Make a half-edge point to itself in the middle of a loop: the walk from the loop's
    // first half-edge never returns to it.
    let lp = b.face_outer_loop(FaceId(0)).unwrap();
    let hs = b.loop_half_edges(lp).unwrap();
    let mid = hs[2];
    b.half_edges.get_mut(mid.index()).unwrap().next = mid;
    assert!(b.validate_topology().is_err());
    assert!(b.loop_half_edges(lp).is_err());
}

#[test]
fn a_missing_twin_fails_validation() {
    let mut b = good_box();
    let other = b.edge_half_edges(EdgeId(1)).unwrap()[0];
    b.edges.get_mut(0).unwrap().halves[1] = other;
    assert!(matches!(
        b.validate_topology(),
        Err(TopologyError::MissingTwin(_))
    ));

    let mut b = good_box();
    let [h0, _] = b.edge_half_edges(EdgeId(2)).unwrap();
    b.half_edges.get_mut(h0.index()).unwrap().edge = None;
    assert!(b.validate_topology().is_err());
}

#[test]
fn wrong_orientation_fails_validation() {
    // Twins running the same way.
    let mut b = good_box();
    let [h0, h1] = b.edge_half_edges(EdgeId(4)).unwrap();
    let o0 = b.half_edge_origin(h0).unwrap();
    b.half_edges.get_mut(h1.index()).unwrap().origin = o0;
    assert!(matches!(
        b.validate_topology(),
        Err(TopologyError::TwinsNotOpposite(_))
    ));

    // A face claiming the opposite normal to its loops.
    let mut b = good_box();
    b.faces.get_mut(2).unwrap().same_sense = false;
    b.validate_topology().unwrap();
    assert_eq!(
        b.validate(),
        Err(TopologyError::WrongOrientation(FaceId(2)))
    );

    // The same plane described by a flipped frame and the opposite sense is still valid;
    // flipping only the senses turns every face's normal against its loops.
    let mut b = good_box();
    for f in b.faces() {
        let (Surface::Plane(frame), _) = b.face_surface(f).unwrap().unwrap();
        let flipped = frame.flipped();
        b.set_face_surface(f, Surface::Plane(flipped), false)
            .unwrap();
    }
    b.validate().unwrap();
    for face in b.faces.items.iter_mut().flatten() {
        face.same_sense = !face.same_sense;
    }
    assert!(b.validate().is_err());
}

#[test]
fn bad_points_and_geometry_fail_validation() {
    let mut b = good_box();
    *b.points.get_mut(0).unwrap() = Point3::new(f64::NAN, 0.0, 0.0);
    b.validate_topology().unwrap();
    assert!(matches!(b.validate(), Err(TopologyError::InvalidPoint(_))));

    let mut b = good_box();
    let p = b.points.get_mut(1).unwrap();
    p.x += 1.0;
    assert!(matches!(
        b.validate(),
        Err(TopologyError::VertexOffCurve(_))
    ));

    let mut b = good_box();
    b.edges.get_mut(0).unwrap().curve = None;
    assert_eq!(b.validate(), Err(TopologyError::MissingCurve(EdgeId(0))));

    let mut b = good_box();
    b.faces.get_mut(0).unwrap().surface = None;
    assert_eq!(b.validate(), Err(TopologyError::MissingSurface(FaceId(0))));

    // A plane moved off the face.
    let mut b = good_box();
    let f = FaceId(1);
    let (Surface::Plane(frame), same) = b.face_surface(f).unwrap().unwrap();
    let moved = frame.with_origin(frame.origin() + frame.z() * 0.5).unwrap();
    b.set_face_surface(f, Surface::Plane(moved), same).unwrap();
    assert_eq!(b.validate(), Err(TopologyError::OffSurface(f)));
}

#[test]
fn broken_ownership_fails_validation() {
    let mut b = good_box();
    b.shells.get_mut(0).unwrap().faces.pop();
    assert!(matches!(
        b.validate_topology(),
        Err(TopologyError::Orphan(_))
    ));

    let mut b = good_box();
    let f = b.faces()[0];
    b.shells.get_mut(0).unwrap().faces.push(f);
    assert!(matches!(
        b.validate_topology(),
        Err(TopologyError::BadOwnership(_))
    ));

    // A face split off into a shell of its own is not connected to the rest.
    let mut b = good_box();
    b.shells
        .get_mut(0)
        .unwrap()
        .faces
        .retain(|x| *x != FaceId(0));
    b.shells.push(Shell {
        faces: vec![FaceId(0)],
    });
    b.faces.get_mut(0).unwrap().shell = ShellId(1);
    assert!(b.validate_topology().is_err());
}

// ---------------------------------------------------------------- serde

#[test]
fn bodies_round_trip_through_json() {
    for b in [
        good_box(),
        Body::prism_with_holes(
            &Frame::XY,
            &square(0.0, 0.0, 20.0),
            &[square(5.0, 5.0, 10.0)],
            20.0,
        )
        .unwrap(),
        Body::cylinder_topology(&Frame::XY, 2.0, 3.0).unwrap(),
    ] {
        let json = serde_json::to_string(&b).unwrap();
        let back: Body = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
        back.validate_topology().unwrap();
    }
}

fn box_json() -> serde_json::Value {
    serde_json::to_value(good_box()).unwrap()
}

fn refused(v: serde_json::Value) -> bool {
    serde_json::from_value::<Body>(v).is_err()
}

#[test]
fn hostile_json_is_refused() {
    for text in [
        "",
        "{}",
        "[]",
        "null",
        "{\"shells\": 5}",
        "{\"shells\": [[[[[[",
    ] {
        assert!(serde_json::from_str::<Body>(text).is_err(), "{text}");
    }
    let mut v = box_json();
    v["half_edges"][0]["next"] = serde_json::json!(4_000_000_000u64);
    assert!(refused(v));
    let mut v = box_json();
    v["half_edges"][0]["next"] = serde_json::json!(-1);
    assert!(refused(v));
    let mut v = box_json();
    v["half_edges"][0]["origin"] = serde_json::json!(7);
    assert!(refused(v));
    let mut v = box_json();
    v["edges"][0]["halves"] = serde_json::json!([0, 0]);
    assert!(refused(v));
    let mut v = box_json();
    v["vertices"][0] = serde_json::Value::Null;
    assert!(refused(v));
    let mut v = box_json();
    v["shells"] = serde_json::json!([]);
    assert!(refused(v));
    let mut v = box_json();
    v["points"][0]["x"] = serde_json::json!(1e300);
    assert!(refused(v));
    let mut v = box_json();
    v["points"][0]["x"] = serde_json::Value::Null;
    assert!(refused(v));
    let mut v = box_json();
    v["edges"][0]["curve"]["t1"] = serde_json::json!(5.0);
    assert!(refused(v));
    let mut v = box_json();
    v["curves"][0] =
        serde_json::json!({"type": "circle", "frame": v["surfaces"][0]["plane"], "radius": -1.0});
    assert!(refused(v));
    let mut v = box_json();
    v["curves"][0]["type"] = serde_json::json!("spline");
    assert!(refused(v));
    let mut v = box_json();
    v["surfaces"][0] = serde_json::json!({"plane": {"origin": {"x": 0, "y": 0, "z": 0}, "normal": {"x": 0, "y": 0, "z": 0}, "x": {"x": 1, "y": 0, "z": 0}}});
    assert!(refused(v));
    let mut v = box_json();
    v["faces"][0]["name"] = serde_json::json!("n".repeat(MAX_NAME_LENGTH + 1));
    assert!(refused(v));
    let mut v = box_json();
    v["faces"][0]["colour"] = serde_json::json!("red");
    assert!(refused(v));
    let mut v = box_json();
    v["extra"] = serde_json::json!(1);
    assert!(refused(v));
    // A cycle in next links that skips the loop's first half-edge.
    let mut v = box_json();
    v["half_edges"][1]["next"] = serde_json::json!(1);
    assert!(refused(v));
    // The untouched value still loads.
    let back: Body = serde_json::from_value(box_json()).unwrap();
    back.validate().unwrap();
}

// ---------------------------------------------------------------- random operator runs

/// A small deterministic generator (Knuth's MMIX linear congruential constants).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> Option<T> {
        if items.is_empty() {
            None
        } else {
            items.get(self.next() as usize % items.len()).copied()
        }
    }
}

#[test]
fn random_euler_operator_sequences_keep_the_topology_valid() {
    // Whatever an operator is asked, it either succeeds and leaves a structurally valid
    // body (Euler-Poincare included), or fails and changes nothing.
    let mut kinds = [0usize; 9];
    for seed in 0..8u64 {
        let mut rng = Lcg(seed);
        let mut b = Body::cuboid(Point3::ORIGIN, Vec3::new(4.0, 3.0, 2.0)).unwrap();
        b.mvfs(Point3::new(9.0, 9.0, 9.0)).unwrap();
        let mut successes = 0;
        for step in 0..250 {
            let before = b.clone();
            let hes: Vec<HalfEdgeId> = b
                .half_edges
                .live()
                .map(|(i, _)| HalfEdgeId::from_index(i))
                .collect();
            let edges = b.edges();
            let faces = b.faces();
            let loops: Vec<LoopId> = b.loops.live().map(|(i, _)| LoopId::from_index(i)).collect();
            let p = Point3::new(
                (rng.next() % 100) as f64 * 0.1,
                (rng.next() % 100) as f64 * 0.1,
                (rng.next() % 100) as f64 * 0.1,
            );
            let grow = hes.len() < 300;
            let kind = rng.next() % 9;
            let result: KernelResult<()> = match kind {
                0 | 1 if grow => match rng.pick(&hes) {
                    Some(h) => b.mev(h, p).map(|_| ()),
                    None => Ok(()),
                },
                2 | 3 if grow => match rng.pick(&loops) {
                    Some(l) => {
                        let hs = b.loop_half_edges(l).unwrap();
                        let (h1, h2) = (rng.pick(&hs).unwrap(), rng.pick(&hs).unwrap());
                        b.mef(h1, h2).map(|_| ())
                    }
                    None => Ok(()),
                },
                4 => match rng.pick(&edges) {
                    Some(e) => b.kef(e),
                    None => Ok(()),
                },
                5 => match rng.pick(&edges) {
                    Some(e) => {
                        let vs = b.edge_vertices(e).unwrap();
                        b.kev(e, rng.pick(&vs).unwrap())
                    }
                    None => Ok(()),
                },
                6 => match rng.pick(&hes) {
                    Some(h) => b.kemr(h).map(|_| ()),
                    None => Ok(()),
                },
                7 => match (rng.pick(&faces), rng.pick(&faces)) {
                    (Some(f), Some(g)) => b.kfmrh(f, g),
                    _ => Ok(()),
                },
                _ => {
                    let rings: Vec<LoopId> = faces
                        .iter()
                        .flat_map(|f| b.face_inner_loops(*f).unwrap())
                        .collect();
                    match rng.pick(&rings) {
                        Some(r) if rng.next().is_multiple_of(2) => b.mfkrh(r).map(|_| ()),
                        Some(r) => {
                            let f = b.loop_face(r).unwrap();
                            let outer = b.face_outer_loop(f).unwrap();
                            let h1 = b.loop_half_edges(outer).unwrap()[0];
                            let h2 = b.loop_half_edges(r).unwrap()[0];
                            b.mekr(h1, h2).map(|_| ())
                        }
                        None => Ok(()),
                    }
                }
            };
            match result {
                Ok(()) => {
                    successes += 1;
                    kinds[kind as usize] += 1;
                    if let Err(e) = b.validate_topology() {
                        panic!("seed {seed} step {step}: {e}");
                    }
                    // serde_json may round decimal floats by an ulp, so compare the
                    // structure rather than every coordinate.
                    let json = serde_json::to_string(&b).unwrap();
                    let back = serde_json::from_str::<Body>(&json).unwrap();
                    assert_eq!(back.summary(), b.summary());
                    assert_eq!(back.validate_topology(), b.validate_topology());
                }
                Err(_) => assert_eq!(
                    b, before,
                    "seed {seed} step {step}: a failure changed the body"
                ),
            }
        }
        assert!(
            successes > 100,
            "seed {seed}: only {successes} operators succeeded"
        );
    }
    // Every kind of operator succeeded somewhere (kinds 0-1 MEV, 2-3 MEF, 4 KEF, 5 KEV,
    // 6 KEMR, 7 KFMRH, 8 MFKRH or MEKR).
    eprintln!("successes per kind: {kinds:?}");
    assert!(kinds.iter().all(|k| *k > 0), "{kinds:?}");
}
