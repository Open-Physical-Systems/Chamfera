//! Tests for K5 tessellation. Expected values are closed forms derived in the comments,
//! or the body's own exact measures (K2), never the mesh's own output.

use super::delaunay::Mesh2;
use super::domain::{SurfaceDomain, unwrap_loop};
use super::edges::{circle_step, discretise, segment_count};
use super::triangulate::{triangulate, twice_signed_area};
use super::*;
use crate::{Arc3, Circle3, Curve, Ellipse3};
use chamfera_geom::{Frame, Point2, Sign, Vec3, orient2d};
use std::collections::{HashMap, HashSet};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1.0)
}

/// Every directed mesh edge is used exactly once and its reverse exactly once, and no
/// triangle repeats a vertex: the mesh is closed and consistently oriented.
fn assert_watertight(mesh: &TriMesh) {
    let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
    for t in &mesh.triangles {
        assert!(
            t[0] != t[1] && t[1] != t[2] && t[2] != t[0],
            "degenerate {t:?}"
        );
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *directed.entry((a, b)).or_default() += 1;
        }
    }
    for (&(a, b), &n) in &directed {
        assert_eq!(n, 1, "edge {a}->{b} used {n} times in one direction");
        assert_eq!(
            directed.get(&(b, a)),
            Some(&1),
            "edge {a}->{b} has no opposite use"
        );
    }
}

/// No cracks between faces, for bodies whose faces may coincide (laminae): within each
/// face every interior side is used once each way; the remaining (boundary) sides of all
/// faces pair up exactly, each with its reverse on another face.
fn assert_crack_free(m: &BodyMesh) {
    let mut per_face: HashMap<FaceId, HashSet<(u32, u32)>> = HashMap::new();
    for (i, t) in m.mesh.triangles.iter().enumerate() {
        let sides = per_face.entry(m.face_of_triangle[i]).or_default();
        for e in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            assert!(sides.insert(e), "side {e:?} twice in one face");
        }
    }
    let mut boundary: HashMap<(u32, u32), FaceId> = HashMap::new();
    for (f, sides) in &per_face {
        for &(a, b) in sides {
            if !sides.contains(&(b, a)) {
                assert!(boundary.insert((a, b), *f).is_none());
            }
        }
    }
    for (&(a, b), f) in &boundary {
        let other = boundary
            .get(&(b, a))
            .expect("a boundary side without a partner");
        assert_ne!(other, f);
    }
}

/// The outward unit normal of a planar face.
fn outward(body: &Body, f: FaceId) -> Vec3 {
    let Some((Surface::Plane(frame), same)) = body.face_surface(f).unwrap() else {
        panic!("{f} is not planar");
    };
    if same { frame.z() } else { -frame.z() }
}

/// Every triangle has a normal, and it is its face's outward normal.
fn assert_normals_agree(body: &Body, m: &BodyMesh) {
    assert_eq!(m.face_of_triangle.len(), m.mesh.triangles.len());
    for (i, f) in m.face_of_triangle.iter().enumerate() {
        let n = m.mesh.triangle_normal(i).expect("non-degenerate triangle");
        assert!(n.dot(outward(body, *f)) > 1.0 - 1e-9, "triangle {i} of {f}");
    }
}

/// Every point of an edge's polyline is, bit for bit, a vertex of the triangles of both
/// faces of the edge, and its end points are the edge's vertex points.
fn assert_edges_shared(body: &Body, m: &BodyMesh) {
    let bits = |p: Point3| (p.x.to_bits(), p.y.to_bits(), p.z.to_bits());
    let mut by_face: HashMap<FaceId, HashSet<(u64, u64, u64)>> = HashMap::new();
    for (i, f) in m.face_of_triangle.iter().enumerate() {
        for p in m.mesh.triangle(i).unwrap() {
            by_face.entry(*f).or_default().insert(bits(p));
        }
    }
    assert_eq!(m.edge_polylines.len(), body.edges().len());
    for (e, line) in &m.edge_polylines {
        let [v0, v1] = body.edge_vertices(*e).unwrap();
        assert_eq!(bits(line[0]), bits(body.vertex_point(v0).unwrap()));
        assert_eq!(
            bits(*line.last().unwrap()),
            bits(body.vertex_point(v1).unwrap())
        );
        for f in body.edge_faces(*e).unwrap() {
            for p in line {
                assert!(
                    by_face[&f].contains(&bits(*p)),
                    "{e} point {p:?} missing on {f}"
                );
            }
        }
    }
}

/// The checks every tessellated body passes, against its exact volume and area.
fn check_body(body: &Body, volume: f64, area: f64) -> BodyMesh {
    let m = tessellate(body, &TessOptions::default()).unwrap();
    assert_watertight(&m.mesh);
    assert_normals_agree(body, &m);
    assert_edges_shared(body, &m);
    assert!(
        close(m.mesh.signed_volume(), volume, 1e-9),
        "{} vs {volume}",
        m.mesh.signed_volume()
    );
    assert!(
        close(m.mesh.area(), area, 1e-9),
        "{} vs {area}",
        m.mesh.area()
    );
    // And the body's own exact measures agree with the closed forms.
    assert!(close(body.volume().unwrap(), volume, 1e-9));
    assert!(close(body.area().unwrap(), area, 1e-9));
    m
}

fn square(x0: f64, y0: f64, side: f64) -> Vec<Point2> {
    vec![
        Point2::new(x0, y0),
        Point2::new(x0 + side, y0),
        Point2::new(x0 + side, y0 + side),
        Point2::new(x0, y0 + side),
    ]
}

fn pts(c: &[(f64, f64)]) -> Vec<Point2> {
    c.iter().map(|&(x, y)| Point2::new(x, y)).collect()
}

#[test]
fn a_cuboid_is_twelve_triangles_closed_with_its_exact_volume_and_area() {
    // 50 x 30 x 10: V = 15000, A = 2 (50·30 + 50·10 + 30·10) = 4600. Six quads, two
    // triangles each.
    let body = Body::cuboid(Point3::new(1.0, -2.0, 3.0), Vec3::new(50.0, 30.0, 10.0)).unwrap();
    let m = check_body(&body, 15000.0, 4600.0);
    assert_eq!(m.mesh.triangles.len(), 12);
    // No points beyond the 8 corners: straight edges add none.
    assert_eq!(m.mesh.positions.len(), 8);
}

#[test]
fn a_plate_with_a_square_hole_matches_its_exact_measures() {
    // 50 x 30 plate, 10 x 10 hole, 10 thick: V = (1500 - 100) 10 = 14000;
    // A = 2·1400 + (2·80)·10 + (4·10)·10 = 2800 + 1600 + 400 = 4800.
    // Triangles: top and bottom have m = 8 points and h = 1 hole, m + 2h - 2 = 8 each;
    // 8 side quads give 16: 32 in all.
    let outer = pts(&[(0.0, 0.0), (50.0, 0.0), (50.0, 30.0), (0.0, 30.0)]);
    let body =
        Body::prism_with_holes(&Frame::XY, &outer, &[square(20.0, 10.0, 10.0)], 10.0).unwrap();
    let m = check_body(&body, 14000.0, 4800.0);
    assert_eq!(m.mesh.triangles.len(), 32);
}

#[test]
fn a_plate_with_many_holes_in_a_grid_matches_its_exact_measures() {
    // Six 4 x 4 holes in two rows; holes in a column share their largest x (ties for the
    // bridging order). V = (1500 - 6·16) 10 = 14040;
    // A = 2·1404 + 160·10 + 6·16·10 = 2808 + 1600 + 960 = 5368.
    let outer = pts(&[(0.0, 0.0), (50.0, 0.0), (50.0, 30.0), (0.0, 30.0)]);
    let mut holes = Vec::new();
    for i in 0..3 {
        for j in 0..2 {
            holes.push(square(8.0 + 14.0 * i as f64, 6.0 + 12.0 * j as f64, 4.0));
        }
    }
    let body = Body::prism_with_holes(&Frame::XY, &outer, &holes, 10.0).unwrap();
    let m = check_body(&body, 14040.0, 5368.0);
    // Caps: m = 4 + 24 = 28, h = 6: 28 + 12 - 2 = 38 each; 28 side quads: 56. Total 132.
    assert_eq!(m.mesh.triangles.len(), 132);
}

#[test]
fn a_tilted_plate_with_holes_matches_the_bodys_exact_measures() {
    // The same plate on a tilted frame: volume and area do not depend on placement.
    let frame = Frame::from_normal(Point3::new(5.0, -7.0, 2.0), Vec3::new(1.0, 2.0, 3.0)).unwrap();
    let outer = pts(&[(0.0, 0.0), (50.0, 0.0), (50.0, 30.0), (0.0, 30.0)]);
    let holes = vec![square(5.0, 5.0, 8.0), square(30.0, 12.0, 6.0)];
    let body = Body::prism_with_holes(&frame, &outer, &holes, 4.0).unwrap();
    // V = (1500 - 64 - 36)·4 = 5600; A = 2·1400 + 160·4 + (32 + 24)·4 = 3664.
    check_body(&body, 5600.0, 3664.0);
}

#[test]
fn collinear_profile_points_are_kept_and_no_triangle_is_degenerate() {
    // A 30 x 10 rectangle with extra points on two sides, 5 high:
    // V = 300·5 = 1500, A = 2·300 + 80·5 = 1000. The cap is a 7-gon: 5 triangles, each
    // with a normal (non-zero area), and every collinear point used (no T-junction, so
    // the mesh is closed).
    let profile = pts(&[
        (0.0, 0.0),
        (10.0, 0.0),
        (20.0, 0.0),
        (30.0, 0.0),
        (30.0, 10.0),
        (15.0, 10.0),
        (0.0, 10.0),
    ]);
    let body = Body::prism(&Frame::XY, &profile, 5.0).unwrap();
    let m = check_body(&body, 1500.0, 1000.0);
    // Caps 5 + 5, seven side quads 14.
    assert_eq!(m.mesh.triangles.len(), 24);
}

#[test]
fn a_profile_with_a_hairline_notch_and_a_sliver_prism_tessellate_exactly() {
    // A 40 x 20 rectangle with a notch 0.001 wide at the top running down to (20, 1):
    // area = 800 - ½·0.001·19 = 799.9905; 2 high: V = 1599.981.
    let notch = pts(&[
        (0.0, 0.0),
        (40.0, 0.0),
        (40.0, 20.0),
        (20.0005, 20.0),
        (20.0, 1.0),
        (19.9995, 20.0),
        (0.0, 20.0),
    ]);
    let body = Body::prism(&Frame::XY, &notch, 2.0).unwrap();
    let area = body.area().unwrap();
    check_body(&body, 1599.981, area);

    // A needle triangle: base 100, height 0.001: area 0.05, 3 high: V = 0.15.
    let sliver = pts(&[(0.0, 0.0), (100.0, 0.0), (50.0, 0.001)]);
    let body = Body::prism(&Frame::XY, &sliver, 3.0).unwrap();
    let m = check_body(&body, 0.15, body.area().unwrap());
    assert_eq!(m.mesh.triangles.len(), 8);
}

#[test]
fn a_hexagonal_prism_matches_its_closed_form() {
    // Regular hexagon of side 10: area = (3√3/2)·100; 7 high. Lateral area 6·10·7.
    let s = 10.0;
    let hex: Vec<Point2> = (0..6)
        .map(|i| {
            let a = TAU * i as f64 / 6.0;
            Point2::new(s * a.cos(), s * a.sin())
        })
        .collect();
    let body = Body::prism(&Frame::XY, &hex, 7.0).unwrap();
    let cap = 1.5 * 3f64.sqrt() * s * s;
    let m = check_body(&body, cap * 7.0, 2.0 * cap + 420.0);
    // Caps 4 + 4, six side quads 12.
    assert_eq!(m.mesh.triangles.len(), 20);
}

#[test]
fn a_concave_l_shaped_prism_matches_its_closed_form() {
    // L: 30 x 10 foot plus 10 x 30 upright: area 300 + 300 = 600, perimeter
    // 30 + 10 + 20 + 30 + 10 + 40 = 140; 5 high: V = 3000, A = 1200 + 700 = 1900.
    let l = pts(&[
        (0.0, 0.0),
        (30.0, 0.0),
        (30.0, 10.0),
        (10.0, 10.0),
        (10.0, 40.0),
        (0.0, 40.0),
    ]);
    let body = Body::prism(&Frame::XY, &l, 5.0).unwrap();
    check_body(&body, 3000.0, 1900.0);
    // Upside down (the frame flipped): the same measures.
    let body = Body::prism(&Frame::XY.flipped(), &l, 5.0).unwrap();
    check_body(&body, 3000.0, 1900.0);
}

// --- Curved edges -------------------------------------------------------------------

/// The distance from the chord `ab` to the curve: the largest over samples of the curve
/// between `t0` and `t1` of the distance to the segment.
fn chord_error(c: &Curve, t0: f64, t1: f64, a: Point3, b: Point3) -> f64 {
    let d = b - a;
    (0..=64)
        .map(|k| {
            let p = c.point_at(t0 + (t1 - t0) * k as f64 / 64.0);
            let s = ((p - a).dot(d) / d.dot(d)).clamp(0.0, 1.0);
            p.distance(a + d * s)
        })
        .fold(0.0, f64::max)
}

#[test]
fn a_circle_gets_the_segment_count_of_the_sagitta_formula_and_every_chord_is_within_tolerance() {
    // r = 10, tol = 0.01: a chord over φ has sagitta r (1 - cos(φ/2)) ≤ tol, so
    // φ ≤ 2 acos(1 - 0.001) = 0.0894..., and a full turn needs ceil(2π / φ) = 71.
    let (r, tol) = (10.0, 0.01);
    let opts = TessOptions {
        chord_tolerance: tol,
        angle_tolerance: PI,
        ..TessOptions::default()
    };
    let circle = Curve::Circle(Circle3::new(Frame::XY, r).unwrap());
    let phi = 2.0 * (1.0 - tol / r).acos();
    let n = segment_count(&circle, 0.0, TAU, &opts).unwrap();
    assert_eq!(n, (TAU / phi).ceil() as usize);
    assert_eq!(n, 71);
    // The cancellation-free step is the same angle.
    assert!(close(circle_step(r, tol, PI), phi, 1e-9));
    let p0 = circle.point_at(0.0);
    let line = discretise(&circle, 0.0, TAU, n, p0, p0);
    for (k, w) in line.windows(2).enumerate() {
        let (t0, t1) = (TAU * k as f64 / n as f64, TAU * (k + 1) as f64 / n as f64);
        let err = chord_error(&circle, t0, t1, w[0], w[1]);
        assert!(err <= tol, "chord {k}: {err}");
        // And close to it: n is the smallest count (with n - 1 the sagitta exceeds tol).
        assert!(err > 0.8 * tol);
    }
    assert!(r * (1.0 - (PI / (n - 1) as f64).cos()) > tol);

    // A quarter arc: ceil((π/2) / φ) = ceil(17.56) = 18.
    let arc = Curve::Arc(Arc3::new(Frame::XY, r, 0.3, FRAC_PI_2).unwrap());
    assert_eq!(
        segment_count(&arc, 0.3, 0.3 + FRAC_PI_2, &opts).unwrap(),
        18
    );
    // The angle limit: 0.05 rad per segment, ceil(2π / 0.05) = 126.
    let fine_angle = TessOptions {
        angle_tolerance: 0.05,
        ..opts
    };
    assert_eq!(segment_count(&circle, 0.0, TAU, &fine_angle).unwrap(), 126);
    // A coarse tolerance still gives a triangle for a full circle and two for any arc.
    let coarse = TessOptions {
        chord_tolerance: 100.0,
        angle_tolerance: PI,
        ..opts
    };
    assert_eq!(segment_count(&circle, 0.0, TAU, &coarse).unwrap(), 3);
    assert_eq!(
        segment_count(&arc, 0.3, 0.3 + FRAC_PI_2, &coarse).unwrap(),
        2
    );
    // A straight edge is always one segment.
    let seg =
        Curve::Segment(crate::Segment3::new(Point3::ORIGIN, Point3::new(1.0, 2.0, 3.0)).unwrap());
    assert_eq!(segment_count(&seg, 0.0, 1.0, &opts).unwrap(), 1);
}

#[test]
fn every_ellipse_chord_is_within_tolerance() {
    let tol = 0.01;
    let opts = TessOptions {
        chord_tolerance: tol,
        ..TessOptions::default()
    };
    let e = Curve::Ellipse(Ellipse3::new(Frame::XY, 20.0, 4.0).unwrap());
    let n = segment_count(&e, 0.0, TAU, &opts).unwrap();
    let p0 = e.point_at(0.0);
    let line = discretise(&e, 0.0, TAU, n, p0, p0);
    for (k, w) in line.windows(2).enumerate() {
        let (t0, t1) = (TAU * k as f64 / n as f64, TAU * (k + 1) as f64 / n as f64);
        assert!(chord_error(&e, t0, t1, w[0], w[1]) <= tol);
    }
}

/// A flat disc of radius `r` as a lamina: one vertex, one circle edge, two faces (top
/// and bottom) back to back. V - E + F = 1 - 1 + 2 = 2. Built as MVFS + MEF on the lone
/// half-edge, as `Body::cylinder_topology` starts.
fn disc_lamina(r: f64) -> Body {
    let mut body = Body::new();
    let p0 = Point3::new(r, 0.0, 0.0);
    let m = body.mvfs(p0).unwrap();
    let (edge, other) = body.mef(m.half_edge, m.half_edge).unwrap();
    body.set_edge_curve(
        edge,
        Curve::Circle(Circle3::new(Frame::XY, r).unwrap()),
        0.0,
        TAU,
    )
    .unwrap();
    body.set_face_plane_from_loop(m.face).unwrap();
    body.set_face_plane_from_loop(other).unwrap();
    body.validate_topology().unwrap();
    body
}

#[test]
fn a_disc_lamina_with_a_circle_edge_is_crack_free_and_has_the_inscribed_polygon_area() {
    // n = 71 segments (see the formula test); each face is the inscribed regular 71-gon
    // of area (n/2) r² sin(2π/n), triangulated into 69 triangles. The two faces share the
    // circle's points, so their boundaries pair up and the volume is zero. (The faces
    // coincide, so an interior side is used by both faces: the per-face check applies.)
    let r = 10.0;
    let body = disc_lamina(r);
    let opts = TessOptions {
        angle_tolerance: PI,
        ..TessOptions::default()
    };
    let m = tessellate(&body, &opts).unwrap();
    assert_crack_free(&m);
    assert_normals_agree(&body, &m);
    assert_edges_shared(&body, &m);
    let n = 71.0;
    let polygon = n / 2.0 * r * r * (TAU / n).sin();
    assert!(close(m.mesh.area(), 2.0 * polygon, 1e-12));
    assert!(m.mesh.signed_volume().abs() < 1e-9);
    assert_eq!(m.mesh.triangles.len(), 2 * 69);
    assert_eq!(m.mesh.positions.len(), 71);
    // The polygon is within the chord tolerance of the circle: every vertex on it.
    for p in &m.mesh.positions {
        assert!((p.distance(Point3::ORIGIN) - r).abs() < 1e-12);
    }
}

#[test]
fn a_half_disc_lamina_with_an_arc_and_a_diameter_has_the_inscribed_area() {
    // MVFS at (r, 0), MEV to (-r, 0) (the diameter), MEF back to (r, 0) (the arc through
    // (0, -r)): two half-disc faces with two edges each. The arc over π gets
    // ceil(π / 0.0894) = 36 segments; the inscribed polygon (fan of 36 triangles about
    // the centre, which is on the diameter) has area (36/2) r² sin(π/36).
    let r = 10.0;
    let mut body = Body::new();
    let p0 = Point3::new(r, 0.0, 0.0);
    let p1 = Point3::new(-r, 0.0, 0.0);
    let m = body.mvfs(p0).unwrap();
    let (diameter, _) = body.mev(m.half_edge, p1).unwrap();
    let [a, b] = body.edge_half_edges(diameter).unwrap();
    let (arc_edge, other) = body.mef(b, a).unwrap();
    body.set_edge_line(diameter).unwrap();
    // The arc's first half-edge runs from (-r, 0) at angle π to (r, 0) at 2π.
    let arc = Curve::Arc(Arc3::new(Frame::XY, r, PI, PI).unwrap());
    body.set_edge_curve(arc_edge, arc, PI, TAU).unwrap();
    body.set_face_plane_from_loop(m.face).unwrap();
    body.set_face_plane_from_loop(other).unwrap();
    body.validate_topology().unwrap();

    let opts = TessOptions {
        angle_tolerance: PI,
        ..TessOptions::default()
    };
    let mesh = tessellate(&body, &opts).unwrap();
    assert_crack_free(&mesh);
    assert_normals_agree(&body, &mesh);
    assert_edges_shared(&body, &mesh);
    let polygon = 18.0 * r * r * (PI / 36.0).sin();
    assert!(close(mesh.mesh.area(), 2.0 * polygon, 1e-12));
    // 37 boundary points per face: 35 triangles each.
    assert_eq!(mesh.mesh.triangles.len(), 70);
}

#[test]
fn a_finer_tolerance_converges_to_the_exact_disc_area() {
    // The inscribed polygon's deficit is at most perimeter × sagitta = 2πr·tol, so the
    // mesh area of each face approaches πr² from below within that bound.
    let r = 10.0;
    let body = disc_lamina(r);
    let mut last = 0.0;
    for tol in [0.1, 0.01, 0.001, 0.0001] {
        let opts = TessOptions {
            chord_tolerance: tol,
            angle_tolerance: PI,
            ..TessOptions::default()
        };
        let a = tessellate(&body, &opts).unwrap().mesh.area() / 2.0;
        assert!(
            a < PI * r * r && PI * r * r - a <= TAU * r * tol,
            "tol {tol}: {a}"
        );
        assert!(a > last);
        last = a;
    }
}

#[test]
fn a_face_without_a_surface_is_unsupported() {
    // MVFS makes a valid skeletal body whose one face has no surface yet.
    let mut body = Body::new();
    body.mvfs(Point3::ORIGIN).unwrap();
    assert!(matches!(
        tessellate(&body, &TessOptions::default()),
        Err(KernelError::Unsupported(_))
    ));
}

// --- Hostile input ------------------------------------------------------------------

#[test]
fn degenerate_tolerances_and_budgets_are_refused() {
    let body = Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 1.0, 1.0)).unwrap();
    let d = TessOptions::default();
    let bad = [
        TessOptions {
            chord_tolerance: 0.0,
            ..d
        },
        TessOptions {
            chord_tolerance: -0.01,
            ..d
        },
        TessOptions {
            chord_tolerance: f64::NAN,
            ..d
        },
        TessOptions {
            chord_tolerance: f64::INFINITY,
            ..d
        },
        TessOptions {
            chord_tolerance: 1e-12,
            ..d
        },
        TessOptions {
            angle_tolerance: 0.0,
            ..d
        },
        TessOptions {
            angle_tolerance: -1.0,
            ..d
        },
        TessOptions {
            angle_tolerance: f64::NAN,
            ..d
        },
        TessOptions {
            angle_tolerance: 4.0,
            ..d
        },
        TessOptions {
            max_triangles: 0,
            ..d
        },
        TessOptions {
            max_triangles: MAX_TRIANGLES + 1,
            ..d
        },
        TessOptions {
            max_triangles: usize::MAX,
            ..d
        },
    ];
    for o in bad {
        assert!(
            matches!(tessellate(&body, &o), Err(KernelError::InvalidInput(_))),
            "{o:?}"
        );
    }
}

#[test]
fn a_mesh_over_the_triangle_budget_is_refused() {
    // The cuboid needs 12 triangles.
    let body = Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 1.0, 1.0)).unwrap();
    let tight = TessOptions {
        max_triangles: 11,
        ..TessOptions::default()
    };
    assert!(matches!(
        tessellate(&body, &tight),
        Err(KernelError::InvalidInput(_))
    ));
    let exact = TessOptions {
        max_triangles: 12,
        ..TessOptions::default()
    };
    assert_eq!(tessellate(&body, &exact).unwrap().mesh.triangles.len(), 12);

    // A 1 km circle at the finest tolerance needs 2π / (4 asin(sqrt(1e-7 / 2e6))) ≈ 7e5
    // segments: refused at the edge, before any allocation, under a small budget.
    let big = disc_lamina(1.0e6);
    let fine = TessOptions {
        chord_tolerance: LINEAR_TOLERANCE,
        max_triangles: 1000,
        ..TessOptions::default()
    };
    assert!(matches!(
        tessellate(&big, &fine),
        Err(KernelError::InvalidInput(_))
    ));
    // Fine enough for the edge (≈ 7025 segments for r = 10 at 1e-7) but not for the two
    // faces of ≈ 7023 triangles each.
    let small = disc_lamina(10.0);
    let budget = TessOptions {
        chord_tolerance: LINEAR_TOLERANCE,
        max_triangles: 10_000,
        ..TessOptions::default()
    };
    assert!(matches!(
        tessellate(&small, &budget),
        Err(KernelError::InvalidInput(_))
    ));
}

// --- The polygon triangulator ----------------------------------------------------------

/// A small deterministic generator (xorshift64*), so the property tests are repeatable.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
}

/// A star-shaped polygon about `c`, counter-clockwise: `n` angles jittered by up to 0.4
/// of their spacing (so consecutive gaps stay below 1.8 · 2π/n) and radii in
/// `[r_lo, r_hi]`. Equal radii make it convex.
fn star(rng: &mut Rng, c: Point2, n: usize, r_lo: f64, r_hi: f64) -> Vec<Point2> {
    (0..n)
        .map(|i| {
            let a = TAU * (i as f64 + rng.range(-0.4, 0.4)) / n as f64;
            let r = rng.range(r_lo, r_hi);
            Point2::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

/// Triangulates the outer polygon minus the holes and checks it exactly: every
/// triangle turns strictly left, the count is m + 2h - 2, the triangles cover each
/// boundary edge once in its direction and every other edge once each way, and the
/// area is the shoelace area of the region.
fn check_triangulation(outer: &[Point2], holes: &[Vec<Point2>]) {
    let mut points: Vec<Point2> = outer.to_vec();
    let outer_ring: Vec<u32> = (0..outer.len() as u32).collect();
    let mut hole_rings = Vec::new();
    for h in holes {
        let start = points.len() as u32;
        // Holes run clockwise.
        points.extend(h.iter().rev());
        hole_rings.push((start..start + h.len() as u32).collect::<Vec<u32>>());
    }
    let t = triangulate(&points, &outer_ring, &hole_rings).unwrap();
    assert_eq!(t.forced, 0);
    assert_eq!(t.triangles.len(), points.len() + 2 * holes.len() - 2);
    let mut area = 0.0;
    let mut directed = HashSet::new();
    for tri in &t.triangles {
        let [a, b, c] = tri.map(|i| points[i as usize]);
        assert_eq!(orient2d(a, b, c), Some(Sign::Positive), "{tri:?}");
        area += (b - a).cross(c - a) / 2.0;
        for e in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            assert!(directed.insert(e), "edge {e:?} twice");
        }
    }
    let mut boundary = HashSet::new();
    for ring in std::iter::once(&outer_ring).chain(&hole_rings) {
        for i in 0..ring.len() {
            boundary.insert((ring[i], ring[(i + 1) % ring.len()]));
        }
    }
    for &(a, b) in &directed {
        assert!(
            boundary.contains(&(a, b)) || directed.contains(&(b, a)),
            "edge {a}->{b} is neither boundary nor shared"
        );
    }
    assert!(boundary.iter().all(|e| directed.contains(e)));
    let exact: f64 = twice_signed_area(&points, &outer_ring) / 2.0
        + hole_rings
            .iter()
            .map(|h| twice_signed_area(&points, h) / 2.0)
            .sum::<f64>();
    assert!(close(area, exact, 1e-9), "{area} vs {exact}");
    // And the Delaunay flips keep all of this.
    let n = points.len();
    let mut mesh = Mesh2::new(points.clone(), vec![Point3::ORIGIN; n], t.triangles);
    mesh.make_delaunay();
    let mut flipped = 0.0;
    for tri in &mesh.triangles {
        let [a, b, c] = tri.map(|i| points[i as usize]);
        assert_eq!(orient2d(a, b, c), Some(Sign::Positive));
        flipped += (b - a).cross(c - a) / 2.0;
    }
    assert!(close(flipped, exact, 1e-9));
}

#[test]
fn random_star_and_convex_polygons_with_random_holes_triangulate_exactly() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..300 {
        let n = 12 + (rng.next() * 30.0) as usize;
        let convex = case % 3 == 0;
        let outer = if convex {
            star(&mut rng, Point2::ORIGIN, n, 100.0, 100.0)
        } else {
            star(&mut rng, Point2::ORIGIN, n, 50.0, 100.0)
        };
        // Gaps below 1.8 · 2π/12 ≈ 0.94 rad keep every edge at least 50 cos(0.47) ≈ 44.6
        // from the centre, so a 3 x 3 grid of 20 x 20 cells in [-30, 30]² (corner 42.4)
        // lies inside; a hole in a cell stays within radius 8 of its centre.
        let mut holes = Vec::new();
        for i in 0..3 {
            for j in 0..3 {
                if rng.next() < 0.5 {
                    let c = Point2::new(-20.0 + 20.0 * i as f64, -20.0 + 20.0 * j as f64);
                    let m = 3 + (rng.next() * 10.0) as usize;
                    let lo = if rng.next() < 0.5 { 8.0 } else { 3.0 };
                    holes.push(star(&mut rng, c, m.max(6), lo, 8.0));
                }
            }
        }
        check_triangulation(&outer, &holes);
    }
}

#[test]
fn polygons_with_many_collinear_points_and_aligned_holes_triangulate_exactly() {
    // A 40 x 40 square with 10 points on every side (collinear runs), and a 3 x 3 grid
    // of axis-aligned square holes, also with collinear side points: many ties in x and
    // y for the bridging and many zero orientations for the ears.
    let side = |x0: f64, y0: f64, s: f64, k: usize| -> Vec<Point2> {
        let mut out = Vec::new();
        let corners = [(x0, y0), (x0 + s, y0), (x0 + s, y0 + s), (x0, y0 + s)];
        for c in 0..4 {
            let (a, b) = (corners[c], corners[(c + 1) % 4]);
            for i in 0..k {
                let t = i as f64 / k as f64;
                out.push(Point2::new(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            }
        }
        out
    };
    let outer = side(0.0, 0.0, 40.0, 10);
    let mut holes = Vec::new();
    for i in 0..3 {
        for j in 0..3 {
            holes.push(side(
                5.0 + 12.0 * i as f64,
                5.0 + 12.0 * j as f64,
                6.0,
                1 + (i + j) % 3,
            ));
        }
    }
    check_triangulation(&outer, &holes);
    // A comb of 20 teeth 1 wide and 9 tall on a base 1 high: 40 reflex corners, and the
    // base points (2k, 1) all on one line.
    let mut comb = vec![Point2::new(0.0, 0.0), Point2::new(40.0, 0.0)];
    for k in (0..20).rev() {
        let x = 2.0 * k as f64;
        comb.push(Point2::new(x + 2.0, 10.0));
        comb.push(Point2::new(x + 1.0, 10.0));
        comb.push(Point2::new(x + 1.0, 1.0));
        comb.push(Point2::new(x, 1.0));
    }
    check_triangulation(&comb, &[]);
}

#[test]
fn random_prisms_with_holes_match_the_bodys_exact_volume() {
    // The same random profiles as real bodies (prism_with_holes checks they are simple
    // and apart), on a tilted frame.
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let frame = Frame::from_normal(Point3::new(1.0, 2.0, 3.0), Vec3::new(-1.0, 0.5, 2.0)).unwrap();
    let mut built = 0;
    for _ in 0..20 {
        let n = 12 + (rng.next() * 12.0) as usize;
        let outer = star(&mut rng, Point2::ORIGIN, n, 50.0, 100.0);
        let mut holes = Vec::new();
        for i in 0..3 {
            for j in 0..3 {
                if rng.next() < 0.4 {
                    let c = Point2::new(-20.0 + 20.0 * i as f64, -20.0 + 20.0 * j as f64);
                    holes.push(star(&mut rng, c, 6, 3.0, 8.0));
                }
            }
        }
        let Ok(body) = Body::prism_with_holes(&frame, &outer, &holes, 7.5) else {
            continue;
        };
        built += 1;
        check_body(&body, body.volume().unwrap(), body.area().unwrap());
    }
    assert!(built >= 18, "only {built} random profiles were valid");
}

// --- The (u, v) path for curved surfaces -----------------------------------------------

#[test]
fn loops_crossing_a_seam_are_unwrapped_and_loops_winding_around_are_refused() {
    // Period 10 in u: a loop stepping 9 -> 1 crosses the seam and continues at 11.
    let mut uv = pts(&[(8.0, 0.0), (9.0, 0.0), (1.0, 0.0), (1.0, 1.0), (9.0, 1.0)]);
    unwrap_loop(&mut uv, Some(10.0), None).unwrap();
    let u: Vec<f64> = uv.iter().map(|p| p.x).collect();
    assert_eq!(u, vec![8.0, 9.0, 11.0, 11.0, 9.0]);
    // A loop going all the way round, 0, 3, 6, 9 and back to 0 = 12 in period 12, does
    // not close in (u, v): refused.
    let mut band = pts(&[(0.0, 0.0), (3.0, 0.0), (6.0, 0.0), (9.0, 0.0)]);
    assert!(matches!(
        unwrap_loop(&mut band, Some(12.0), None),
        Err(KernelError::Unsupported(_))
    ));
    // Without a period the same points are an ordinary (if flat) ring.
    assert!(unwrap_loop(&mut band, None, None).is_ok());
}

/// A test-only curved surface: the sphere of radius `R` about the origin with
/// u = R · longitude, v = R · latitude (isometric at the equator).
struct TestSphere(f64);

impl SurfaceDomain for TestSphere {
    fn surface_point(&self, uv: Point2) -> Point3 {
        let (lon, lat) = (uv.x / self.0, uv.y / self.0);
        Point3::new(
            self.0 * lat.cos() * lon.cos(),
            self.0 * lat.cos() * lon.sin(),
            self.0 * lat.sin(),
        )
    }
    fn surface_uv(&self, p: Point3) -> Point2 {
        let lon = p.y.atan2(p.x);
        let lat = (p.z / self.0).clamp(-1.0, 1.0).asin();
        Point2::new(self.0 * lon, self.0 * lat)
    }
    fn surface_normal(&self, uv: Point2) -> Vec3 {
        self.surface_point(uv) - Point3::ORIGIN
    }
}

#[test]
fn a_curved_patch_is_refined_until_it_is_within_tolerance_of_its_surface() {
    // The patch longitude [0, 0.6], latitude [-0.3, 0.3] of a sphere of radius 10, with
    // 32 boundary points per side: steps of at most 6/32 mm, so boundary chords deviate
    // at most (6/32)² / (8 · 10 cos 0.3) ≈ 0.00046 < tol, as edge discretisation would
    // guarantee. Exact area: R² Δlon (sin lat1 - sin lat0) = 100 · 0.6 · 2 sin 0.3.
    let r = 10.0;
    let sphere = TestSphere(r);
    let k = 32;
    let (u0, u1, v0, v1) = (0.0, 0.6 * r, -0.3 * r, 0.3 * r);
    let mut uv = Vec::new();
    for i in 0..k {
        uv.push(Point2::new(u0 + (u1 - u0) * i as f64 / k as f64, v0));
    }
    for i in 0..k {
        uv.push(Point2::new(u1, v0 + (v1 - v0) * i as f64 / k as f64));
    }
    for i in 0..k {
        uv.push(Point2::new(u1 - (u1 - u0) * i as f64 / k as f64, v1));
    }
    for i in 0..k {
        uv.push(Point2::new(u0, v1 - (v1 - v0) * i as f64 / k as f64));
    }
    let ring: Vec<u32> = (0..uv.len() as u32).collect();
    let xyz: Vec<Point3> = uv.iter().map(|p| sphere.surface_point(*p)).collect();
    let t = triangulate(&uv, &ring, &[]).unwrap();
    let mut mesh = Mesh2::new(uv, xyz, t.triangles);
    mesh.make_delaunay();
    let tol = 0.001;
    mesh.refine(&sphere, tol, 0.1, 100_000).unwrap();
    assert!(mesh.triangles.len() > 4 * k - 2, "refinement added points");

    let mut area = 0.0;
    let mut directed = HashSet::new();
    for tri in &mesh.triangles {
        let [a, b, c] = tri.map(|i| mesh.uv[i as usize]);
        assert_eq!(orient2d(a, b, c), Some(Sign::Positive));
        let [pa, pb, pc] = tri.map(|i| mesh.xyz[i as usize]);
        area += (pb - pa).cross(pc - pa).length() / 2.0;
        // The triangle's centroid is within the tolerance of the sphere (inside it).
        let g = Point3::ORIGIN
            + ((pa - Point3::ORIGIN) + (pb - Point3::ORIGIN) + (pc - Point3::ORIGIN)) * (1.0 / 3.0);
        assert!(r - g.distance(Point3::ORIGIN) <= tol + 1e-12);
        for e in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            assert!(directed.insert(e));
        }
        // Every point is on the sphere.
        for p in [pa, pb, pc] {
            assert!((p.distance(Point3::ORIGIN) - r).abs() < 1e-9);
        }
    }
    // The boundary is unchanged: each ring edge is still a triangle side.
    for i in 0..ring.len() {
        assert!(directed.contains(&(ring[i], ring[(i + 1) % ring.len()])));
    }
    let exact = r * r * 0.6 * 2.0 * 0.3f64.sin();
    // Inscribed triangles: a little less than the exact area, by at most a few
    // tolerances over the patch (relative error of order tol / R).
    assert!(
        area < exact && area > exact * (1.0 - 10.0 * tol / r),
        "{area} vs {exact}"
    );
}

// --- Curved faces (K3 surfaces from K4 sweeps) ------------------------------------------

mod curved {
    use super::*;
    use crate::{Line2, Profile, ProfileEdge, extrude, polygon_edges, revolve};
    use chamfera_geom::Vec2;

    fn y_axis() -> Line2 {
        Line2::new(Point2::ORIGIN, Vec2::Y).unwrap()
    }

    fn rect(x0: f64, y0: f64, w: f64, h: f64) -> Vec<ProfileEdge> {
        polygon_edges(&[
            Point2::new(x0, y0),
            Point2::new(x0 + w, y0),
            Point2::new(x0 + w, y0 + h),
            Point2::new(x0, y0 + h),
        ])
    }

    fn half_disc(r: f64) -> Profile {
        Profile::new(
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
        )
    }

    /// A named test body with its exact volume and area (closed forms), and the
    /// coefficient `k` of its area error bound `|A_mesh - A| <= k · tol`.
    ///
    /// # Error bounds
    /// Every mesh vertex lies on the body's surface and the refinement keeps every
    /// triangle within `tol` of it (edge chords by the sagitta formula, triangles at their
    /// centroid and side midpoints), so mesh and solid differ only inside the shell of
    /// thickness `tol` around the boundary:
    ///
    /// - **Volume:** `|V_mesh - V| <= A · tol` (the shell's volume, to first order).
    /// - **Area:** a flat facet whose vertices lie on a surface of mean curvature `H` and
    ///   which strays at most `δ` from it is smaller than the surface by about `H δ` per
    ///   unit area (a spherical cap of depth `δ` over its base: `2πrδ - π(2rδ - δ²) =
    ///   πδ²`, i.e. `δ / 2r` relative, `H = 1/r`); and a planar face bounded by a curved
    ///   edge loses at most `chord × sagitta <= tol` per unit length of that edge. With a
    ///   factor 2 for the facets' error not being uniform:
    ///   `|A_mesh - A| <= tol · (2 ∫|H| dA + L)`, `L` the length of the curved edges of
    ///   planar faces. `∫|H| dA`: a cylinder `π h`, a sphere `4π r`, a cone `π s cos α`
    ///   (slant `s`, half-angle `α` to the axis), a torus with `R > 2a`
    ///   `∫∫ (R + 2a cos v) / 2 du dv = 2π² R`.
    struct Case {
        name: &'static str,
        body: Body,
        volume: f64,
        area: f64,
        area_k: f64,
    }

    fn cases() -> Vec<Case> {
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
                centre: Point2::ORIGIN,
                radius: 5.0,
                start_angle: FRAC_PI_2,
                sweep: PI,
            },
        ];
        let plate = Profile::new(
            rect(0.0, 0.0, 80.0, 40.0),
            vec![
                vec![ProfileEdge::circle(Point2::new(20.0, 20.0), 5.0)],
                vec![ProfileEdge::circle(Point2::new(60.0, 20.0), 8.0)],
            ],
        );
        let cone =
            Profile::polygon(&[Point2::ORIGIN, Point2::new(3.0, 0.0), Point2::new(0.0, 4.0)]);
        let tube = Profile::new(rect(3.0, 0.0, 2.0, 4.0), vec![]);
        let torus = Profile::circle(Point2::new(5.0, 0.0), 2.0);
        let pi2 = PI * PI;
        let xy = Frame::XY;
        // Area bound coefficients `k = 2 ∫|H| dA + L` (see `Case`), per body.
        vec![
            // π r² h = 250π; 2π r h + 2π r² = 150π. k: 2 π h + two circles 2π r each.
            Case {
                name: "cylinder",
                body: extrude(&xy, &Profile::circle(Point2::ORIGIN, 5.0), 10.0).unwrap(),
                volume: 250.0 * PI,
                area: 150.0 * PI,
                area_k: 20.0 * PI + 20.0 * PI,
            },
            // 10 (3200 - 25π - 64π); caps 2 (3200 - 89π), outer walls 2 (80 + 40) 10,
            // hole walls 2π (5 + 8) 10. k: 2 (10π + 10π) + circles 2 · 2π (5 + 8).
            Case {
                name: "plate with two round holes",
                body: extrude(&xy, &plate, 10.0).unwrap(),
                volume: 10.0 * (3200.0 - 89.0 * PI),
                area: 2.0 * (3200.0 - 89.0 * PI) + 2400.0 + 260.0 * PI,
                area_k: 40.0 * PI + 52.0 * PI,
            },
            // 10 (200 + 25π); caps 2 (200 + 25π), flat sides 400, half cylinders 100π.
            // k: 2 (two half cylinders, π h / 2 each) + two half circles on each cap.
            Case {
                name: "slot",
                body: extrude(&xy, &Profile::new(slot, vec![]), 10.0).unwrap(),
                volume: 2000.0 + 250.0 * PI,
                area: 800.0 + 150.0 * PI,
                area_k: 20.0 * PI + 20.0 * PI,
            },
            // π (5² - 3²) 4 = 64π; cylinders 2π (3 + 5) 4 = 64π, annuli 2π (25 - 9) = 32π.
            // k: 2 (4π + 4π) + the annuli's four circles 2 · 2π (3 + 5).
            Case {
                name: "tube",
                body: revolve(&xy, &tube, &y_axis(), TAU).unwrap(),
                volume: 64.0 * PI,
                area: 96.0 * PI,
                area_k: 16.0 * PI + 32.0 * PI,
            },
            // 4/3 π 3³ = 36π; 4π 3² = 36π. k: 2 · 4π r.
            Case {
                name: "sphere",
                body: revolve(&xy, &half_disc(3.0), &y_axis(), TAU).unwrap(),
                volume: 36.0 * PI,
                area: 36.0 * PI,
                area_k: 24.0 * PI,
            },
            // 2π² R a² = 40π²; 4π² R a = 40π². k: 2 · 2π² R.
            Case {
                name: "torus",
                body: revolve(&xy, &torus, &y_axis(), TAU).unwrap(),
                volume: 40.0 * pi2,
                area: 40.0 * pi2,
                area_k: 20.0 * pi2,
            },
            // π r² h / 3 = 12π; base π r² = 9π, lateral π r s = 15π (slant s = 5).
            // k: 2 π s cos α with cos α = h / s = 4/5, plus the base circle 2π r.
            Case {
                name: "cone",
                body: revolve(&xy, &cone, &y_axis(), TAU).unwrap(),
                volume: 12.0 * PI,
                area: 24.0 * PI,
                area_k: 8.0 * PI + 6.0 * PI,
            },
            // Half a turn: 2/3 π r³ = 18π; 2π r² plus two half discs π r² = 27π.
            // k: half the sphere's, plus the caps' two half circles π r.
            Case {
                name: "half ball",
                body: revolve(&xy, &half_disc(3.0), &y_axis(), PI).unwrap(),
                volume: 18.0 * PI,
                area: 27.0 * PI,
                area_k: 12.0 * PI + 6.0 * PI,
            },
            // A quarter of the torus, 10π²; a quarter of its area, 10π², plus two
            // discs π a² = 8π. k: a quarter of the torus's, plus the discs' circles.
            Case {
                name: "quarter torus",
                body: revolve(&xy, &torus, &y_axis(), FRAC_PI_2).unwrap(),
                volume: 10.0 * pi2,
                area: 10.0 * pi2 + 8.0 * PI,
                area_k: 5.0 * pi2 + 8.0 * PI,
            },
            // A quarter cone: 3π; (9π + 15π) / 4 plus two triangles r h / 2 = 6 each.
            // k: a quarter of the cone's lateral term, plus the base's quarter circle.
            Case {
                name: "quarter cone",
                body: revolve(&xy, &cone, &y_axis(), FRAC_PI_2).unwrap(),
                volume: 3.0 * PI,
                area: 6.0 * PI + 12.0,
                area_k: 2.0 * PI + 1.5 * PI,
            },
        ]
    }

    /// The outward unit normal of face `f` at the surface point nearest to `p`.
    fn outward_at(body: &Body, f: FaceId, p: Point3) -> Vec3 {
        let (s, same) = body.face_surface(f).unwrap().unwrap();
        let q = s.parameters(p);
        let n = s.normal_at(q.u, q.v);
        if same { n } else { -n }
    }

    fn centroid([a, b, c]: [Point3; 3]) -> Point3 {
        Point3::ORIGIN
            + ((a - Point3::ORIGIN) + (b - Point3::ORIGIN) + (c - Point3::ORIGIN)) * (1.0 / 3.0)
    }

    /// The largest angle between a triangle's normal and its face's outward normal at
    /// the triangle's centroid; every triangle has a normal and none is turned inside
    /// out; every mesh vertex lies on the surface of each face that uses it.
    fn worst_normal(body: &Body, m: &BodyMesh) -> f64 {
        let mut worst: f64 = 0.0;
        for (i, f) in m.face_of_triangle.iter().enumerate() {
            let n = m.mesh.triangle_normal(i).expect("a triangle without area");
            let corners = m.mesh.triangle(i).unwrap();
            let (s, _) = body.face_surface(*f).unwrap().unwrap();
            for p in corners {
                assert!(s.distance(p) < 1e-9, "{p:?} is off its surface");
            }
            let out = outward_at(body, *f, centroid(corners));
            assert!(n.dot(out) > 0.0, "triangle {i} of {f} is inside out");
            worst = worst.max(n.angle_to(out).unwrap());
        }
        worst
    }

    fn with_tol(tol: f64) -> TessOptions {
        TessOptions {
            chord_tolerance: tol,
            ..TessOptions::default()
        }
    }

    #[test]
    fn every_sweep_tessellates_watertight_with_normals_along_its_surface() {
        let d = TessOptions::default();
        for c in cases() {
            let m = tessellate(&c.body, &d).unwrap();
            assert_watertight(&m.mesh);
            assert_edges_shared(&c.body, &m);
            // The refinement lets the surface normal turn by at most twice the angle
            // tolerance along an interior side and a facet's normal lies between its
            // corners' normals, so no facet is further than that from the surface;
            // (thin facets on a curved edge chord or at a pole come closest).
            let worst = worst_normal(&c.body, &m);
            assert!(worst <= 2.0 * d.angle_tolerance, "{}: {worst}", c.name);
            // The derived bounds (see `Case`) at the default tolerance.
            let dv = (m.mesh.signed_volume() - c.volume).abs();
            assert!(
                dv <= c.area * d.chord_tolerance,
                "{}: volume off by {dv}",
                c.name
            );
            let da = (m.mesh.area() - c.area).abs();
            assert!(
                da <= c.area_k * d.chord_tolerance,
                "{}: area off by {da}",
                c.name
            );
            // And the body's own exact measures (K3) agree with the closed forms.
            assert!(
                close(c.body.volume().unwrap(), c.volume, 1e-9),
                "{}",
                c.name
            );
            assert!(close(c.body.area().unwrap(), c.area, 1e-9), "{}", c.name);
        }
    }

    #[test]
    fn volume_and_area_converge_within_the_derived_bounds() {
        // Errors at 0.1, 0.01 and 0.002 mm are within `A tol` and `k tol`, and fall
        // about linearly with the tolerance: by at least half the ratio of tolerances
        // from 0.01 to 0.002 (both chord-limited, unlike 0.1 on small circles, where
        // the 15° angle limit takes over).
        for c in cases() {
            let mut errors = Vec::new();
            for tol in [0.1, 0.01, 0.002] {
                let m = tessellate(&c.body, &with_tol(tol)).unwrap();
                assert_watertight(&m.mesh);
                let dv = (m.mesh.signed_volume() - c.volume).abs();
                let da = (m.mesh.area() - c.area).abs();
                assert!(
                    dv <= c.area * tol,
                    "{} at {tol}: volume off by {dv}",
                    c.name
                );
                assert!(
                    da <= c.area_k * tol,
                    "{} at {tol}: area off by {da}",
                    c.name
                );
                errors.push((dv, da));
            }
            let (v1, a1) = errors[1];
            let (v2, a2) = errors[2];
            assert!(v2 <= v1 / 2.5, "{}: volume {v1} -> {v2}", c.name);
            assert!(a2 <= a1 / 2.5, "{}: area {a1} -> {a2}", c.name);
        }
    }

    #[test]
    fn an_extruded_circle_is_the_inscribed_prism_of_the_sagitta_formula() {
        // r = 5, tol = 0.01: φ = 4 asin(sqrt(0.01 / 10)) = 0.12650 rad, so the circles
        // get n = ceil(2π / φ) = 50 segments (the 15° angle limit is looser). The side's
        // triangles span a top and a bottom point: their centroids are at most
        // r (1 - sqrt(5 + 4 cos φ) / 3) = 0.0088 and their side midpoints the sagitta
        // r (1 - cos(π/50)) = 0.0099 from the cylinder, both within 0.01, so nothing is
        // inserted and the mesh is exactly the prism over the inscribed 50-gon:
        // V = (n/2) r² sin(2π/n) h, A = n r² sin(2π/n) + 2 n r sin(π/n) h.
        let (r, h, n) = (5.0, 10.0, 50.0);
        let body = extrude(&Frame::XY, &Profile::circle(Point2::ORIGIN, r), h).unwrap();
        let m = tessellate(&body, &TessOptions::default()).unwrap();
        let s = (TAU / n).sin();
        assert!(close(
            m.mesh.signed_volume(),
            n / 2.0 * r * r * s * h,
            1e-12
        ));
        let area = n * r * r * s + 2.0 * n * r * (PI / n).sin() * h;
        assert!(close(m.mesh.area(), area, 1e-12));
        // The inscribed polygon contains the circle of radius r cos(π/n) >= r - tol, so
        // the volume lost is at most π (r² - (r - tol)²) h = π tol (2r - tol) h.
        let lost = 250.0 * PI - m.mesh.signed_volume();
        assert!(
            lost > 0.0 && lost <= PI * 0.01 * (2.0 * r - 0.01) * h,
            "{lost}"
        );
        // 2 (n - 2) cap triangles and 2n on the side.
        assert_eq!(m.mesh.triangles.len(), 2 * 48 + 100);
    }

    /// The angles of the triangles of `m` at the mesh vertex at `p`, summed.
    fn angle_sum_at(m: &BodyMesh, p: Point3) -> f64 {
        let mut sum = 0.0;
        for i in 0..m.mesh.triangles.len() {
            let t = m.mesh.triangle(i).unwrap();
            for k in 0..3 {
                if t[k].distance(p) < 1e-9 {
                    let (a, b) = (t[(k + 1) % 3] - t[k], t[(k + 2) % 3] - t[k]);
                    sum += a.angle_to(b).unwrap();
                }
            }
        }
        sum
    }

    #[test]
    fn poles_and_apexes_become_closed_fans() {
        // A smooth point: the facets around it unfold to about a full turn (a little
        // less on a convex fan; a thin facet tilted along a meridian chord can wrinkle
        // it a little above), within 1%. A cone's apex unfolds to the developed cone's
        // sector, 2π r / s = 2π · 3/5 for r = 3, s = 5; the inscribed fan is a little
        // less.
        let d = TessOptions::default();
        let sphere = revolve(&Frame::XY, &half_disc(3.0), &y_axis(), TAU).unwrap();
        let m = tessellate(&sphere, &d).unwrap();
        for pole in [Point3::new(0.0, 3.0, 0.0), Point3::new(0.0, -3.0, 0.0)] {
            let sum = angle_sum_at(&m, pole);
            assert!((sum - TAU).abs() < 0.01 * TAU, "pole {pole:?}: {sum}");
        }
        let cone =
            Profile::polygon(&[Point2::ORIGIN, Point2::new(3.0, 0.0), Point2::new(0.0, 4.0)]);
        let body = revolve(&Frame::XY, &cone, &y_axis(), TAU).unwrap();
        let m = tessellate(&body, &d).unwrap();
        let sector = TAU * 3.0 / 5.0;
        let sum = angle_sum_at(&m, Point3::new(0.0, 4.0, 0.0));
        assert!(
            sum < sector && sum > 0.99 * sector,
            "apex: {sum} vs {sector}"
        );
        // At the apex of the quarter cone the curved face's fan spans a quarter of the
        // sector, and the two triangular caps add their angles there, atan(3/4) each.
        let quarter = revolve(&Frame::XY, &cone, &y_axis(), FRAC_PI_2).unwrap();
        let m = tessellate(&quarter, &d).unwrap();
        let want = sector / 4.0 + 2.0 * (3.0f64 / 4.0).atan();
        let sum = angle_sum_at(&m, Point3::new(0.0, 4.0, 0.0));
        assert!(
            sum < want && sum > want - 0.01 * sector,
            "quarter apex: {sum} vs {want}"
        );
    }

    #[test]
    fn more_revolves_match_the_bodys_exact_measures() {
        let tilted = Frame::from_normal_and_x(
            Point3::new(-3.0, 7.0, 1.0),
            Vec3::new(-2.0, 1.0, 4.0),
            Vec3::new(1.0, 1.0, 0.0),
        )
        .unwrap();
        // A spindle (apple) torus: the arc of the circle about (1, 0) of radius 2 right
        // of the axis, from (0, -√3) through (3, 0) to (0, √3), closed along the axis;
        // its tips are poles of a torus with R = 1 < a = 2.
        let apple = Profile::new(
            vec![
                ProfileEdge::Arc {
                    centre: Point2::new(1.0, 0.0),
                    radius: 2.0,
                    start_angle: -2.0 * PI / 3.0,
                    sweep: 4.0 * PI / 3.0,
                },
                ProfileEdge::line(
                    Point2::new(0.0, 3f64.sqrt()),
                    Point2::new(0.0, -(3f64.sqrt())),
                ),
            ],
            vec![],
        );
        // The lemon torus of the K4 tests: a circular segment beyond the axis's centre.
        let rr = 5f64.sqrt();
        let half = 0.5f64.atan();
        let lemon = Profile::new(
            vec![
                ProfileEdge::Arc {
                    centre: Point2::new(-1.0, 0.0),
                    radius: rr,
                    start_angle: -half,
                    sweep: 2.0 * half,
                },
                ProfileEdge::line(Point2::new(1.0, 1.0), Point2::new(1.0, -1.0)),
            ],
            vec![],
        );
        // A quarter disc turned fully: a hemisphere on a flat disc.
        let quarter_disc = Profile::new(
            vec![
                ProfileEdge::line(Point2::ORIGIN, Point2::new(4.0, 0.0)),
                ProfileEdge::Arc {
                    centre: Point2::ORIGIN,
                    radius: 4.0,
                    start_angle: 0.0,
                    sweep: FRAC_PI_2,
                },
                ProfileEdge::line(Point2::new(0.0, 4.0), Point2::ORIGIN),
            ],
            vec![],
        );
        let bodies = [
            (
                "apple",
                revolve(&Frame::XY, &apple, &y_axis(), TAU).unwrap(),
            ),
            (
                "apple part",
                revolve(&Frame::XY, &apple, &y_axis(), 2.0).unwrap(),
            ),
            (
                "lemon",
                revolve(&Frame::XY, &lemon, &y_axis(), TAU).unwrap(),
            ),
            (
                "hemisphere",
                revolve(&Frame::XY, &quarter_disc, &y_axis(), TAU).unwrap(),
            ),
            (
                "tilted sphere",
                revolve(&tilted, &half_disc(2.0), &y_axis(), TAU).unwrap(),
            ),
            (
                "tilted half ball",
                revolve(&tilted, &half_disc(2.0), &y_axis(), -2.5).unwrap(),
            ),
            (
                "tilted torus part",
                revolve(
                    &tilted,
                    &Profile::circle(Point2::new(6.0, 1.0), 1.5),
                    &y_axis(),
                    4.0,
                )
                .unwrap(),
            ),
        ];
        let d = TessOptions::default();
        for (name, body) in &bodies {
            let m = tessellate(body, &d).unwrap();
            assert_watertight(&m.mesh);
            assert_edges_shared(body, &m);
            let worst = worst_normal(body, &m);
            assert!(worst <= 2.0 * d.angle_tolerance, "{name}: {worst}");
            let (v, a) = (body.volume().unwrap(), body.area().unwrap());
            let dv = (m.mesh.signed_volume() - v).abs();
            assert!(dv <= a * d.chord_tolerance, "{name}: volume off by {dv}");
            // Area: within 0.5% (the bound needs ∫|H| dA, which the spindle's
            // self-crossing torus makes awkward; the shell argument still holds).
            let da = (m.mesh.area() - a).abs();
            assert!(da <= 5e-3 * a, "{name}: area off by {da} of {a}");
        }
    }

    // --- Hostile and degenerate curved input ------------------------------------------

    #[test]
    fn tiny_curved_bodies_near_the_tolerance_tessellate_closed() {
        // Radii a few hundred times the linear tolerance: the chord tolerance (0.01 or
        // the finest, 1e-7) is far above or near the radius, so the angle limit decides.
        for r in [1e-5, 3e-5] {
            let bodies = [
                extrude(&Frame::XY, &Profile::circle(Point2::ORIGIN, r), 2.0 * r).unwrap(),
                revolve(&Frame::XY, &half_disc(r), &y_axis(), TAU).unwrap(),
                revolve(
                    &Frame::XY,
                    &Profile::circle(Point2::new(3.0 * r, 0.0), r),
                    &y_axis(),
                    TAU,
                )
                .unwrap(),
            ];
            for body in &bodies {
                for tol in [0.01, LINEAR_TOLERANCE] {
                    let m = tessellate(body, &with_tol(tol)).unwrap();
                    assert_watertight(&m.mesh);
                    assert_edges_shared(body, &m);
                    let (v, a) = (body.volume().unwrap(), body.area().unwrap());
                    // The shell bound with the effective tolerance: with the chord
                    // tolerance above the radius, a side is limited by the normal
                    // turning at most twice the 15° angle along it, a sagitta of at most
                    // r (1 - cos 15°) < 0.035 r.
                    let eff = tol.min(0.035 * r);
                    let dv = (m.mesh.signed_volume() - v).abs();
                    assert!(dv <= a * eff, "r {r}, tol {tol}: {dv} vs {}", a * eff);
                }
            }
        }
    }

    #[test]
    fn a_long_thin_cylinder_stays_within_the_budget_or_is_refused() {
        // r = 0.5, h = 10⁶ (the working range): at tol 0.01 the chord allows
        // 4 asin(sqrt(0.01)) = 0.40 rad, the angle 15°, so n = 24 and the mesh is the
        // inscribed 24-gon prism: 2 (24 - 2) + 48 = 92 triangles, however long.
        let (r, h, n) = (0.5, 1.0e6, 24.0);
        let body = extrude(&Frame::XY, &Profile::circle(Point2::ORIGIN, r), h).unwrap();
        let m = tessellate(&body, &TessOptions::default()).unwrap();
        assert_eq!(m.mesh.triangles.len(), 92);
        assert_watertight(&m.mesh);
        assert!(close(
            m.mesh.signed_volume(),
            n / 2.0 * r * r * (TAU / n).sin() * h,
            1e-9
        ));
        // Under a budget of 91 triangles it is refused, cleanly.
        let tight = TessOptions {
            max_triangles: 91,
            ..TessOptions::default()
        };
        assert!(matches!(
            tessellate(&body, &tight),
            Err(KernelError::InvalidInput(_))
        ));
        // A finer tolerance needs more segments, and a sphere at the finest tolerance
        // far more triangles than a small budget allows: refused, not truncated.
        let sphere = revolve(&Frame::XY, &half_disc(100.0), &y_axis(), TAU).unwrap();
        // r = 100 at 1e-6: each half meridian needs π / (4 asin(sqrt(5e-9))) ≈ 11100
        // segments, so the face's boundary alone has more points than 20000 triangles
        // can hold.
        let fine = TessOptions {
            chord_tolerance: 1e-6,
            max_triangles: 20_000,
            ..TessOptions::default()
        };
        assert!(matches!(
            tessellate(&sphere, &fine),
            Err(KernelError::InvalidInput(_))
        ));
    }

    #[test]
    fn coarse_tolerances_still_give_closed_meshes_without_folds() {
        // The coarsest allowed angle (π) and a chord tolerance larger than the body:
        // the quarter-turn span limit alone keeps triangles from folding over a pole.
        // A coarse facet's centroid may lie nearer the far side of a thin part, so its
        // normal is compared with the outward normals at its corners (on the surface).
        let coarse = TessOptions {
            chord_tolerance: 100.0,
            angle_tolerance: PI,
            ..TessOptions::default()
        };
        for c in cases() {
            let m = tessellate(&c.body, &coarse).unwrap();
            assert_watertight(&m.mesh);
            assert_edges_shared(&c.body, &m);
            for (i, f) in m.face_of_triangle.iter().enumerate() {
                let n = m.mesh.triangle_normal(i).unwrap();
                let corners = m.mesh.triangle(i).unwrap();
                let sum = corners
                    .iter()
                    .fold(Vec3::ZERO, |s, p| s + outward_at(&c.body, *f, *p));
                assert!(n.dot(sum) > 0.0, "{}: triangle {i}", c.name);
            }
        }
    }
}
