//! Tests for K11 blends. Expected volumes and areas are closed forms derived in the
//! comments, never the code's own output.

use super::*;
use crate::{
    Curve, FaceId, KernelError, Profile, Surface, TessOptions, extrude, polygon_edges, tessellate,
};
use chamfera_geom::{Frame, Point2, Point3, TriMesh};
use std::collections::HashMap;
use std::f64::consts::PI;

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1.0)
}

fn assert_close(what: &str, got: f64, want: f64) {
    assert!(close(got, want, 1e-9), "{what}: got {got}, want {want}");
}

/// 50 x 30 x 10, named by K4's extrude: `side 0` is the face y = 0 and `top 0` its top
/// edge (length 50, along x).
fn block() -> Body {
    extrude(&Frame::XY, &Profile::polygon(&rect(50.0, 30.0)), 10.0).unwrap()
}

fn rect(w: f64, h: f64) -> Vec<Point2> {
    vec![
        Point2::new(0.0, 0.0),
        Point2::new(w, 0.0),
        Point2::new(w, h),
        Point2::new(0.0, h),
    ]
}

fn edge(b: &Body, name: &str) -> EdgeId {
    b.edges()
        .into_iter()
        .find(|e| b.edge_name(*e).unwrap() == Some(name))
        .unwrap_or_else(|| panic!("no edge named {name}"))
}

fn face(b: &Body, name: &str) -> FaceId {
    b.faces()
        .into_iter()
        .find(|f| b.face_name(*f).unwrap() == Some(name))
        .unwrap_or_else(|| panic!("no face named {name}"))
}

/// Every directed mesh edge is used once and its reverse once: closed and oriented.
fn assert_watertight(mesh: &TriMesh) {
    let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
    for t in &mesh.triangles {
        assert!(t[0] != t[1] && t[1] != t[2] && t[2] != t[0]);
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *directed.entry((a, b)).or_default() += 1;
        }
    }
    for (&(a, b), &n) in &directed {
        assert_eq!(n, 1, "mesh edge {a}->{b} used {n} times");
        assert_eq!(
            directed.get(&(b, a)),
            Some(&1),
            "mesh edge {a}->{b} is open"
        );
    }
}

/// Validates, checks Euler-Poincare for one genus-`g` shell, tessellates watertight with
/// a mesh volume near the exact one, and returns the exact volume.
fn check(b: &Body, genus: usize) -> f64 {
    let r = b.validate().unwrap();
    assert_eq!(r.genus, genus);
    assert_eq!(r.euler_characteristic(), 2 - 2 * genus as i64);
    let v = b.volume().unwrap();
    let m = tessellate(b, &TessOptions::default()).unwrap();
    assert_watertight(&m.mesh);
    // Every triangle is within the 0.01 mm chord tolerance of its surface, so the mesh
    // volume differs from the exact one by at most chord tolerance x area.
    let bound = TessOptions::default().chord_tolerance * b.area().unwrap();
    assert!(
        (m.mesh.signed_volume() - v).abs() <= bound,
        "mesh {} vs {v}",
        m.mesh.signed_volume()
    );
    v
}

fn faces(b: &Body) -> usize {
    b.summary().faces
}

fn surfaces(b: &Body) -> Vec<Surface> {
    b.faces()
        .into_iter()
        .map(|f| b.face_surface(f).unwrap().unwrap().0)
        .collect()
}

#[test]
fn chamfer_of_one_box_edge_removes_a_triangular_prism() {
    let b = block();
    let d = 2.0;
    let out = chamfer(&b, &[edge(&b, "top 0")], d).unwrap();
    // The cut is a right triangle with legs d along the whole 50 mm edge.
    assert_close("volume", check(&out, 0), 15000.0 - d * d / 2.0 * 50.0);
    assert_eq!(faces(&out), 7);
    // Area: top and side 0 each lose a 50 x d strip, the end faces a d²/2 triangle each,
    // and the bevel adds 50 x d√2.
    let area = 4600.0 - 2.0 * 50.0 * d - d * d + 50.0 * d * 2f64.sqrt();
    assert_close("area", out.area().unwrap(), area);
    let bevel = face(&out, "chamfer of top 0");
    assert!(matches!(
        out.face_surface(bevel).unwrap(),
        Some((Surface::Plane(_), _))
    ));
    assert_close(
        "bevel area",
        out.face_area(bevel).unwrap(),
        50.0 * d * 2f64.sqrt(),
    );
}

#[test]
fn fillet_of_one_box_edge_removes_the_corner_outside_a_quarter_circle() {
    let b = block();
    let r = 3.0;
    let out = fillet(&b, &[edge(&b, "top 0")], r).unwrap();
    // Cross-section removed: an r x r square minus a quarter disc, (1 - π/4) r².
    let v = 15000.0 - (1.0 - PI / 4.0) * r * r * 50.0;
    assert_close("volume", check(&out, 0), v);
    assert_eq!(faces(&out), 7);
    // Area 2(50·30 + 50·10 + 30·10) = 4600; top and side 0 each lose a 50 x r strip, the
    // quarter cylinder adds (π/2) r · 50, the two end faces each lose (1 - π/4) r².
    let area = 4600.0 - 100.0 * r + 25.0 * PI * r - 2.0 * (1.0 - PI / 4.0) * r * r;
    assert_close("area", out.area().unwrap(), area);
    let round = face(&out, "fillet of top 0");
    let Some((Surface::Cylinder(c), true)) = out.face_surface(round).unwrap() else {
        panic!("the fillet is not an outward cylinder");
    };
    assert_close("radius", c.radius(), r);
    assert_close("fillet area", out.face_area(round).unwrap(), 25.0 * PI * r);
    // The neighbours keep their names and shrink; the end faces are circles' sections.
    assert_close(
        "top",
        out.face_area(face(&out, "top")).unwrap(),
        50.0 * (30.0 - r),
    );
    assert_close(
        "side 0",
        out.face_area(face(&out, "side 0")).unwrap(),
        50.0 * (10.0 - r),
    );
    assert!(
        out.edges()
            .into_iter()
            .any(|e| matches!(out.edge_curve(e).unwrap(), Some((Curve::Circle(_), _, _))))
    );
}

#[test]
fn fillet_ending_on_oblique_faces_cuts_ellipses() {
    // A parallelogram (0,0),(50,0),(60,30),(10,30) extruded 10: the end faces of the
    // bottom-to-top edge `top 0` are parallel oblique planes 50 mm apart along x. A prism
    // cut by two parallel planes keeps its volume (Cavalieri), so the fillet removes
    // (1 - π/4) r² · 50 from 50 · 30 · 10.
    let profile = Profile::polygon(&[
        Point2::new(0.0, 0.0),
        Point2::new(50.0, 0.0),
        Point2::new(60.0, 30.0),
        Point2::new(10.0, 30.0),
    ]);
    let b = extrude(&Frame::XY, &profile, 10.0).unwrap();
    let r = 2.0;
    let out = fillet(&b, &[edge(&b, "top 0")], r).unwrap();
    assert_close(
        "volume",
        check(&out, 0),
        15000.0 - (1.0 - PI / 4.0) * r * r * 50.0,
    );
    assert!(
        out.edges()
            .into_iter()
            .any(|e| matches!(out.edge_curve(e).unwrap(), Some((Curve::Ellipse(_), _, _))))
    );
    let d = 1.5;
    let out = chamfer(&b, &[edge(&b, "top 0")], d).unwrap();
    assert_close("volume", check(&out, 0), 15000.0 - d * d / 2.0 * 50.0);
}

#[test]
fn fillet_of_the_four_top_edges_mitres_the_corners() {
    let b = block();
    let r = 2.0;
    let top: Vec<EdgeId> = (0..4).map(|i| edge(&b, &format!("top {i}"))).collect();
    let out = fillet(&b, &top, r).unwrap();
    // Summing (1 - π/4) r² L over the four full edges counts each corner column
    // [0, r]² x [h - r, h] twice, (1 - π/4) r³ per edge. There the mitre plane x = y
    // splits it: at depth u = r - y from the cylinder's axis plane the strip of width u
    // (x from y to r) loses the height r - √(r² - u²), so the column loses
    // 2 ∫₀^r u (r - √(r² - u²)) du = 2 (r³/2 - r³/3) = r³/3.
    let naive = (1.0 - PI / 4.0) * r * r * 2.0 * (50.0 + 30.0);
    let per_corner = 2.0 * (1.0 - PI / 4.0) * r.powi(3) - r.powi(3) / 3.0;
    assert_close("volume", check(&out, 0), 15000.0 - naive + 4.0 * per_corner);
    assert_eq!(faces(&out), 10);
    assert!(
        out.edges()
            .into_iter()
            .any(|e| matches!(out.edge_curve(e).unwrap(), Some((Curve::Ellipse(_), _, _))))
    );
    // An attached sketch on `top` still finds its face: a 46 x 26 rectangle.
    assert_close(
        "top",
        out.face_area(face(&out, "top")).unwrap(),
        46.0 * 26.0,
    );
}

#[test]
fn fillet_of_all_twelve_box_edges_is_a_rounded_box() {
    let b = block();
    let r = 2.0;
    let all = b.edges();
    let out = fillet(&b, &all, r).unwrap();
    // A box with all edges rounded by r is the Minkowski sum of the box shrunk by r on
    // every side (a' = a - 2r, ...) with a ball of radius r (Steiner's formula):
    // V = a'b'c' + 2r(a'b' + b'c' + a'c') + π r² (a' + b' + c') + 4/3 π r³,
    // A = 2(a'b' + b'c' + a'c') + 2π r (a' + b' + c') + 4π r².
    let (a, bb, c) = (50.0 - 2.0 * r, 30.0 - 2.0 * r, 10.0 - 2.0 * r);
    let v = a * bb * c
        + 2.0 * r * (a * bb + bb * c + a * c)
        + PI * r * r * (a + bb + c)
        + 4.0 / 3.0 * PI * r.powi(3);
    let area = 2.0 * (a * bb + bb * c + a * c) + 2.0 * PI * r * (a + bb + c) + 4.0 * PI * r * r;
    assert_close("volume", check(&out, 0), v);
    assert_close("area", out.area().unwrap(), area);
    // 6 faces + 12 cylinders + 8 sphere octants.
    assert_eq!(faces(&out), 26);
    let spheres = surfaces(&out)
        .into_iter()
        .filter(|s| matches!(s, Surface::Sphere(_)))
        .count();
    assert_eq!(spheres, 8);
    for f in out.faces() {
        if out
            .face_name(f)
            .unwrap()
            .is_some_and(|n| n.starts_with("fillet corner"))
        {
            // One eighth of the sphere: 4π r² / 8.
            assert_close("octant", out.face_area(f).unwrap(), PI * r * r / 2.0);
        }
    }
}

#[test]
fn chamfer_of_all_twelve_box_edges_has_triangular_corners() {
    let b = block();
    let d = 2.0;
    let out = chamfer(&b, &b.edges(), d).unwrap();
    // Each edge loses a d²/2 prism between the corner cubes (length L - 2d); each corner
    // cube [0, d]³ keeps only x + y + z >= 2d (the corner triangle's plane), a
    // tetrahedron of d³/6, so it loses 5d³/6:
    // V = abc - 2d²(a + b + c) + 12 d³ - 8 · 5d³/6 = abc - 2d²(a + b + c) + 16/3 d³.
    let v = 15000.0 - 2.0 * d * d * 90.0 + 16.0 / 3.0 * d.powi(3);
    assert_close("volume", check(&out, 0), v);
    assert_eq!(faces(&out), 26);
    // The corner triangle is equilateral with side d√2: area √3/4 · 2d².
    let corner = out
        .faces()
        .into_iter()
        .find(|f| {
            out.face_name(*f)
                .unwrap()
                .is_some_and(|n| n.starts_with("chamfer corner"))
        })
        .unwrap();
    assert_close(
        "corner",
        out.face_area(corner).unwrap(),
        3f64.sqrt() / 2.0 * d * d,
    );
}

/// An L-bracket: the L (0,0),(40,0),(40,10),(10,10),(10,30),(0,30) (area 600) extruded
/// 20. Its inner, concave edge is `sweep 3` at (10, 10), between `side 2` and `side 3`.
fn bracket() -> Body {
    let l = Profile::polygon(&[
        Point2::new(0.0, 0.0),
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 10.0),
        Point2::new(10.0, 10.0),
        Point2::new(10.0, 30.0),
        Point2::new(0.0, 30.0),
    ]);
    extrude(&Frame::XY, &l, 20.0).unwrap()
}

#[test]
fn concave_fillet_and_chamfer_add_material() {
    let b = bracket();
    let inner = edge(&b, "sweep 3");
    let r = 4.0;
    let out = fillet(&b, &[inner], r).unwrap();
    // The fillet fills the r x r square outside the quarter disc along the 20 mm edge.
    assert_close(
        "volume",
        check(&out, 0),
        12000.0 + (1.0 - PI / 4.0) * r * r * 20.0,
    );
    let f = face(&out, "fillet of sweep 3");
    assert!(matches!(
        out.face_surface(f).unwrap(),
        Some((Surface::Cylinder(_), false))
    ));
    let d = 3.0;
    let out = chamfer(&b, &[inner], d).unwrap();
    assert_close("volume", check(&out, 0), 12000.0 + d * d / 2.0 * 20.0);
}

/// 50 x 30 x 10 with a through hole of radius 5 at (25, 15).
fn plate_with_hole() -> Body {
    let p = Profile::new(polygon_edges(&rect(50.0, 30.0)), vec![]).with_hole(vec![
        crate::ProfileEdge::circle(Point2::new(25.0, 15.0), 5.0),
    ]);
    extrude(&Frame::XY, &p, 10.0).unwrap()
}

#[test]
fn hole_rim_fillet_is_a_torus_and_chamfer_a_cone() {
    let b = plate_with_hole();
    let rim = edge(&b, "hole 0 top 0");
    let v0 = 15000.0 - 250.0 * PI;
    let (rc, r) = (5.0, 2.0);
    let out = fillet(&b, &[rim], r).unwrap();
    // Pappus: the removed meridian region is the square [Rc, Rc + r] x [h - r, h] minus
    // the quarter disc about (Rc + r, h - r), area (1 - π/4) r². Its first moment about
    // ρ = Rc: the square's r³/2 minus the quarter disc's ∫ (r - x) dA = π r³/4 - r³/3,
    // i.e. r³ (5/6 - π/4); it sweeps 2π (Rc (1 - π/4) r² + r³ (5/6 - π/4)).
    let removed = 2.0 * PI * (rc * (1.0 - PI / 4.0) * r * r + r.powi(3) * (5.0 / 6.0 - PI / 4.0));
    assert_close("volume", check(&out, 1), v0 - removed);
    assert!(
        surfaces(&out)
            .iter()
            .any(|s| matches!(s, Surface::Torus(_)))
    );
    let d = 1.5;
    let out = chamfer(&b, &[rim], d).unwrap();
    // A right triangle of legs d, centroid at ρ = Rc + d/3: 2π (Rc + d/3) d²/2.
    assert_close("volume", check(&out, 1), v0 - PI * d * d * (rc + d / 3.0));
    assert!(surfaces(&out).iter().any(|s| matches!(s, Surface::Cone(_))));
}

#[test]
fn boss_rim_fillet_and_chamfer_follow_pappus() {
    let b = extrude(
        &Frame::XY,
        &Profile::circle(Point2::new(0.0, 0.0), 10.0),
        20.0,
    )
    .unwrap();
    let top = face(&b, "top");
    let rim = b.face_edges(top).unwrap()[0];
    let (rc, r) = (10.0, 3.0);
    let v0 = PI * rc * rc * 20.0;
    let out = fillet(&b, &[rim], r).unwrap();
    // The removed region is the square [Rc - r, Rc] x [h - r, h] minus the quarter disc
    // about (Rc - r, h - r); its first moment about ρ = Rc - r is r³/2 - r³/3 = r³/6.
    let removed = 2.0 * PI * ((rc - r) * (1.0 - PI / 4.0) * r * r + r.powi(3) / 6.0);
    assert_close("volume", check(&out, 0), v0 - removed);
    assert_close(
        "top",
        out.face_area(face(&out, "top")).unwrap(),
        PI * (rc - r).powi(2),
    );
    let d = 2.0;
    let out = chamfer(&b, &[rim], d).unwrap();
    assert_close("volume", check(&out, 0), v0 - PI * d * d * (rc - d / 3.0));
    // A radius that swallows the top face is refused.
    assert!(matches!(
        fillet(&b, &[rim], 10.0),
        Err(KernelError::InvalidInput(_))
    ));
}

#[test]
fn neighbouring_faces_keep_their_names() {
    let b = block();
    let out = fillet(&b, &[edge(&b, "top 0")], 2.0).unwrap();
    let mut names: Vec<String> = out
        .faces()
        .into_iter()
        .map(|f| out.face_name(f).unwrap().unwrap().to_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "bottom",
            "fillet of top 0",
            "side 0",
            "side 1",
            "side 2",
            "side 3",
            "top"
        ]
    );
    // Kept edges keep their names; the blend's edges are named from it.
    assert!(
        out.edges()
            .into_iter()
            .any(|e| out.edge_name(e).unwrap() == Some("top 2"))
    );
    assert!(
        out.edges()
            .into_iter()
            .any(|e| out.edge_name(e).unwrap() == Some("fillet of top 0 on top"))
    );
}

#[test]
fn hostile_sizes_are_refused() {
    let b = block();
    let e = edge(&b, "top 0");
    for s in [
        0.0,
        -1.0,
        f64::NAN,
        f64::INFINITY,
        -f64::INFINITY,
        1e-12,
        1e300,
    ] {
        assert!(
            matches!(fillet(&b, &[e], s), Err(KernelError::InvalidInput(_))),
            "{s}"
        );
        assert!(
            matches!(chamfer(&b, &[e], s), Err(KernelError::InvalidInput(_))),
            "{s}"
        );
    }
    assert!(matches!(
        fillet(&b, &[], 1.0),
        Err(KernelError::InvalidInput(_))
    ));
    // An edge of another, larger body does not exist here.
    let big = extrude(
        &Frame::XY,
        &Profile::polygon(
            &(0..40)
                .map(|i| {
                    let t = i as f64 / 40.0 * std::f64::consts::TAU;
                    Point2::new(t.cos(), t.sin())
                })
                .collect::<Vec<_>>(),
        ),
        1.0,
    )
    .unwrap();
    let alien = *big.edges().last().unwrap();
    assert!(fillet(&b, &[alien], 1.0).is_err());
}

#[test]
fn blends_that_would_consume_a_face_are_refused() {
    let b = block();
    let e = edge(&b, "top 0");
    // side 0 is 10 high: a setback of 10 or more would consume it.
    for s in [10.0, 12.0, 40.0] {
        assert!(
            matches!(fillet(&b, &[e], s), Err(KernelError::InvalidInput(_))),
            "{s}"
        );
        assert!(
            matches!(chamfer(&b, &[e], s), Err(KernelError::InvalidInput(_))),
            "{s}"
        );
    }
    // Just below the limit still works.
    check(&fillet(&b, &[e], 9.9).unwrap(), 0);
    // Two opposite top edges of a 30-wide top meet at 15.
    let long = [edge(&b, "top 0"), edge(&b, "top 2")];
    assert!(fillet(&b, &long, 9.0).is_ok());
    let thin = extrude(&Frame::XY, &Profile::polygon(&rect(50.0, 6.0)), 10.0).unwrap();
    let long = [edge(&thin, "top 0"), edge(&thin, "top 2")];
    assert!(matches!(
        fillet(&thin, &long, 3.5),
        Err(KernelError::InvalidInput(_))
    ));
}

#[test]
fn unsupported_edges_are_refused_cleanly() {
    let b = extrude(
        &Frame::XY,
        &Profile::circle(Point2::new(0.0, 0.0), 10.0),
        20.0,
    )
    .unwrap();
    // The cylinder's seam has the lateral face on both sides.
    let seam = edge(&b, "sweep 0");
    assert!(matches!(
        fillet(&b, &[seam], 1.0),
        Err(KernelError::Unsupported(_))
    ));
    // A straight edge meeting an arc at its end (a slot's flat side).
    let slot = Profile::new(
        vec![
            crate::ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0)),
            crate::ProfileEdge::arc_from_points(
                Point2::new(20.0, 5.0),
                Point2::new(20.0, 0.0),
                Point2::new(20.0, 10.0),
            )
            .unwrap(),
            crate::ProfileEdge::line(Point2::new(20.0, 10.0), Point2::new(0.0, 10.0)),
            crate::ProfileEdge::line(Point2::new(0.0, 10.0), Point2::new(0.0, 0.0)),
        ],
        vec![],
    );
    let s = extrude(&Frame::XY, &slot, 5.0).unwrap();
    assert!(matches!(
        fillet(&s, &[edge(&s, "top 0")], 1.0),
        Err(KernelError::Unsupported(_))
    ));
    // Mixed convex and concave blended edges at one corner.
    let l = bracket();
    let mixed = [edge(&l, "sweep 3"), edge(&l, "top 2"), edge(&l, "top 3")];
    assert!(matches!(
        fillet(&l, &mixed, 1.0),
        Err(KernelError::Unsupported(_))
    ));
}

#[test]
fn a_chain_of_bracket_edges_validates() {
    // All six top edges of the bracket (five convex, one concave) at corners with one
    // blended and two plain edges or two blended edges of one kind.
    let l = bracket();
    let tops: Vec<EdgeId> = [0, 1, 4, 5]
        .iter()
        .map(|i| edge(&l, &format!("top {i}")))
        .collect();
    let out = fillet(&l, &tops, 1.0).unwrap();
    check(&out, 0);
    let out = chamfer(&l, &tops, 1.0).unwrap();
    check(&out, 0);
}

#[test]
fn three_fillets_at_one_corner_meet_in_a_sphere_octant() {
    let b = block();
    let v = b.vertices()[0];
    assert_eq!(b.vertex_point(v).unwrap(), Point3::ORIGIN);
    let es = b.vertex_edges(v).unwrap();
    for r in [1.0, 2.0, 3.0] {
        let out = fillet(&b, &es, r).unwrap();
        // Each edge loses (1 - π/4) r² along L - r (its far end meets a plain end face);
        // the corner cube [0, r]³ keeps only the ball octant about (r, r, r), losing
        // r³ - π r³ / 6.
        let removed = (1.0 - PI / 4.0) * r * r * (90.0 - 3.0 * r) + r.powi(3) * (1.0 - PI / 6.0);
        assert_close("volume", check(&out, 0), 15000.0 - removed);
        assert_eq!(faces(&out), 10);
    }
}

/// The edge between the vertices at `p` and `q` (either way round).
fn edge_at(b: &Body, p: Point3, q: Point3) -> EdgeId {
    b.edges()
        .into_iter()
        .find(|e| {
            let [v0, v1] = b.edge_vertices(*e).unwrap();
            let (a, c) = (b.vertex_point(v0).unwrap(), b.vertex_point(v1).unwrap());
            (a.coincident(p) && c.coincident(q)) || (a.coincident(q) && c.coincident(p))
        })
        .unwrap_or_else(|| panic!("no edge from {p:?} to {q:?}"))
}

/// The edges of face `name`'s loop `k` (0 the outer loop).
fn loop_edges(b: &Body, name: &str, k: usize) -> Vec<EdgeId> {
    let f = face(b, name);
    let lp = b.face_loops(f).unwrap()[k];
    b.loop_half_edges(lp)
        .unwrap()
        .into_iter()
        .map(|h| b.half_edge_edge(h).unwrap().unwrap())
        .collect()
}

/// The slot of length 2a + 2R and width 2R about the origin: lines y = ±R for
/// |x| <= a, semicircles of radius R about (±a, 0).
fn slot_edges(a: f64, r: f64) -> Vec<crate::ProfileEdge> {
    use crate::ProfileEdge;
    use std::f64::consts::FRAC_PI_2;
    vec![
        ProfileEdge::line(Point2::new(-a, -r), Point2::new(a, -r)),
        ProfileEdge::Arc {
            centre: Point2::new(a, 0.0),
            radius: r,
            start_angle: -FRAC_PI_2,
            sweep: PI,
        },
        ProfileEdge::line(Point2::new(a, r), Point2::new(-a, r)),
        ProfileEdge::Arc {
            centre: Point2::new(-a, 0.0),
            radius: r,
            start_angle: FRAC_PI_2,
            sweep: PI,
        },
    ]
}

#[test]
fn the_top_rim_of_a_slot_is_filleted_by_cylinders_and_tori() {
    // The 40 x 20 slot (lines of L = 20, semicircles of R = 10), 10 high.
    let (l, rr, h) = (20.0, 10.0, 10.0);
    let b = extrude(
        &Frame::XY,
        &Profile::new(slot_edges(l / 2.0, rr), vec![]),
        h,
    )
    .unwrap();
    let v0 = (2.0 * rr * l + PI * rr * rr) * h;
    let rim = loop_edges(&b, "top", 0);
    assert_eq!(rim.len(), 4);
    for r in [1.0, 2.0, 4.0] {
        let out = fillet(&b, &rim, r).unwrap();
        // Along each line the section loses the box edge's (1 - π/4) r². The two
        // semicircles are half turns about two axes, together one whole turn of the boss
        // rim's meridian region: the square [R - r, R] x [h - r, h] minus the quarter disc
        // about (R - r, h - r), area (1 - π/4) r², first moment about ρ = R - r of
        // r³/2 - r³/3 = r³/6, so by Pappus 2π ((R - r)(1 - π/4) r² + r³/6). The joins are
        // tangent, so the pieces meet in a common quarter circle and nothing else is cut.
        let removed = 2.0 * l * (1.0 - PI / 4.0) * r * r
            + 2.0 * PI * ((rr - r) * (1.0 - PI / 4.0) * r * r + r.powi(3) / 6.0);
        assert_close("volume", check(&out, 0), v0 - removed);
        // Top, bottom, four sides and four blend faces.
        assert_eq!(faces(&out), 10);
        // The top shrinks to the slot of R - r.
        assert_close(
            "top",
            out.face_area(face(&out, "top")).unwrap(),
            2.0 * (rr - r) * l + PI * (rr - r) * (rr - r),
        );
        let s = surfaces(&out);
        assert_eq!(
            s.iter().filter(|s| matches!(s, Surface::Torus(_))).count(),
            2
        );
        assert_eq!(
            s.iter()
                .filter(|s| matches!(s, Surface::Cylinder(c) if (c.radius() - r).abs() < 1e-12))
                .count(),
            2
        );
    }
    let d = 1.5;
    let out = chamfer(&b, &rim, d).unwrap();
    // Lines lose d²/2; the cone pieces form one turn of the boss rim's right triangle of
    // legs d with its centroid at ρ = R - d/3: 2π (R - d/3) d²/2.
    let removed = 2.0 * l * d * d / 2.0 + PI * d * d * (rr - d / 3.0);
    assert_close("volume", check(&out, 0), v0 - removed);
    assert_eq!(
        surfaces(&out)
            .iter()
            .filter(|s| matches!(s, Surface::Cone(_)))
            .count(),
        2
    );
}

#[test]
fn the_rim_of_a_slot_hole_and_of_a_rounded_rectangle_follow_pappus() {
    // A slot-shaped hole (a = 10, R = 5) through a 60 x 40 x 10 plate: the rim is convex
    // all round, the arcs are hole rims (the plane face lies outside the circle).
    let (a, rr, h) = (10.0, 5.0, 10.0);
    let plate = Profile::new(
        polygon_edges(&[
            Point2::new(-30.0, -20.0),
            Point2::new(30.0, -20.0),
            Point2::new(30.0, 20.0),
            Point2::new(-30.0, 20.0),
        ]),
        vec![],
    )
    .with_hole(slot_edges(a, rr));
    let b = extrude(&Frame::XY, &plate, h).unwrap();
    let v0 = 60.0 * 40.0 * h - (4.0 * a * rr + PI * rr * rr) * h;
    let rim = loop_edges(&b, "top", 1);
    assert_eq!(rim.len(), 4);
    let r = 2.0;
    let out = fillet(&b, &rim, r).unwrap();
    // Lines: (1 - π/4) r² over 2a each. Arcs: one whole turn of the hole rim's meridian
    // region, the square [R, R + r] x [h - r, h] minus the quarter disc about
    // (R + r, h - r), first moment about ρ = R of r³/2 - (π r³/4 - r³/3) (see
    // `hole_rim_fillet_is_a_torus_and_chamfer_a_cone`).
    let removed = 2.0 * (2.0 * a) * (1.0 - PI / 4.0) * r * r
        + 2.0 * PI * (rr * (1.0 - PI / 4.0) * r * r + r.powi(3) * (5.0 / 6.0 - PI / 4.0));
    assert_close("volume", check(&out, 1), v0 - removed);

    // A rounded rectangle 40 x 30 with corner radius 6, 8 high: four lines and four
    // quarter arcs about four axes, together one whole turn of the boss rim.
    let (w, d, rc, h) = (40.0, 30.0, 6.0, 8.0);
    use crate::ProfileEdge;
    use std::f64::consts::FRAC_PI_2;
    let (x0, x1, y0, y1) = (rc, w - rc, rc, d - rc);
    let rounded = Profile::new(
        vec![
            ProfileEdge::line(Point2::new(x0, 0.0), Point2::new(x1, 0.0)),
            ProfileEdge::Arc {
                centre: Point2::new(x1, y0),
                radius: rc,
                start_angle: -FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            ProfileEdge::line(Point2::new(w, y0), Point2::new(w, y1)),
            ProfileEdge::Arc {
                centre: Point2::new(x1, y1),
                radius: rc,
                start_angle: 0.0,
                sweep: FRAC_PI_2,
            },
            ProfileEdge::line(Point2::new(x1, d), Point2::new(x0, d)),
            ProfileEdge::Arc {
                centre: Point2::new(x0, y1),
                radius: rc,
                start_angle: FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            ProfileEdge::line(Point2::new(0.0, y1), Point2::new(0.0, y0)),
            ProfileEdge::Arc {
                centre: Point2::new(x0, y0),
                radius: rc,
                start_angle: PI,
                sweep: FRAC_PI_2,
            },
        ],
        vec![],
    );
    let b = extrude(&Frame::XY, &rounded, h).unwrap();
    let v0 = (w * d - (4.0 - PI) * rc * rc) * h;
    let rim = loop_edges(&b, "top", 0);
    assert_eq!(rim.len(), 8);
    let lines = 2.0 * (w - 2.0 * rc) + 2.0 * (d - 2.0 * rc);
    for r in [1.0, 3.0] {
        let out = fillet(&b, &rim, r).unwrap();
        let removed = lines * (1.0 - PI / 4.0) * r * r
            + 2.0 * PI * ((rc - r) * (1.0 - PI / 4.0) * r * r + r.powi(3) / 6.0);
        assert_close("volume", check(&out, 0), v0 - removed);
        assert_eq!(faces(&out), 18);
    }
    // The bottom rim too: two separate chains on one body.
    let mut both = rim.clone();
    both.extend(loop_edges(&b, "bottom", 0));
    let r = 1.0;
    let out = fillet(&b, &both, r).unwrap();
    let removed = lines * (1.0 - PI / 4.0) * r * r
        + 2.0 * PI * ((rc - r) * (1.0 - PI / 4.0) * r * r + r.powi(3) / 6.0);
    assert_close("volume", check(&out, 0), v0 - 2.0 * removed);
}

/// A D: the circle of radius 10 about the origin cut by the chord y = -6 (from (-8, -6)
/// to (8, -6)), extruded by `h`.
fn d_shape(h: f64) -> Body {
    let half = 0.6f64.asin();
    let p = Profile::new(
        vec![
            crate::ProfileEdge::line(Point2::new(-8.0, -6.0), Point2::new(8.0, -6.0)),
            crate::ProfileEdge::Arc {
                centre: Point2::ORIGIN,
                radius: 10.0,
                start_angle: -half,
                sweep: PI + 2.0 * half,
            },
        ],
        vec![],
    );
    extrude(&Frame::XY, &p, h).unwrap()
}

/// Twice the signed area of the triangle `a b c`.
fn tri2(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

#[test]
fn a_straight_seam_between_a_plane_and_a_cylinder_is_filleted_by_a_cylinder() {
    let h = 10.0;
    let b = d_shape(h);
    let half = 0.6f64.asin();
    let v0 = h * (100.0 / 2.0 * (PI + 2.0 * half) + 0.5 * 16.0 * 6.0);
    let seam = edge_at(&b, Point3::new(8.0, -6.0, 0.0), Point3::new(8.0, -6.0, h));
    let r = 2.0;
    let out = fillet(&b, &[seam], r).unwrap();
    // In the section: the corner P = (8, -6) between the line y = -6 and the circle
    // x² + y² = 100. The ball centre is r above the line and r inside the circle:
    // c = (√((10 - r)² - (6 - r)²), -6 + r) = (√48, -4). It touches the line at
    // T_l = (√48, -6) and the circle at T_c = c · 10 / 8 = (1.25 √48, -5). The removed
    // region is the triangle T_l P T_c plus the circle's segment over the chord P T_c
    // (central angle θ = ∠T_c - ∠P) minus the fillet circle's segment over T_l T_c
    // (central angle π/3: T_l - c = (0, -2), T_c - c = (√3, -1)). A segment of angle θ in
    // a circle of radius ρ has area ρ² (θ - sin θ) / 2.
    let s48 = 48f64.sqrt();
    let (tl, p, tc) = ((s48, -6.0), (8.0, -6.0), (1.25 * s48, -5.0));
    let theta = (-5.0f64).atan2(1.25 * s48) - (-6.0f64).atan2(8.0);
    let seg = |rho: f64, t: f64| rho * rho * (t - t.sin()) / 2.0;
    let area = tri2(tl, p, tc).abs() / 2.0 + seg(10.0, theta) - seg(r, PI / 3.0);
    assert_close("volume", check(&out, 0), v0 - area * h);
    let round = face(
        &out,
        &format!("fillet of {}", b.edge_name(seam).unwrap().unwrap()),
    );
    let Some((Surface::Cylinder(c), true)) = out.face_surface(round).unwrap() else {
        panic!("the fillet is not an outward cylinder");
    };
    assert_close("radius", c.radius(), r);
    let o = c.frame().origin();
    assert_close("axis x", o.x, s48);
    assert_close("axis y", o.y, -4.0);
    assert!(c.frame().z().cross(chamfera_geom::Vec3::Z).length() < 1e-12);

    let d = 2.0;
    let out = chamfer(&b, &[seam], d).unwrap();
    // T_l = (6, -6) and T_c at chord d from P on the circle: the angle grows by
    // Δ = 2 asin(d / 20). Removed: the triangle plus the circle's segment over P T_c.
    let delta = 2.0 * (d / 20.0).asin();
    let tc = (10.0 * (-half + delta).cos(), 10.0 * (-half + delta).sin());
    let area = tri2((6.0, -6.0), p, tc).abs() / 2.0 + seg(10.0, delta);
    assert_close("volume", check(&out, 0), v0 - area * h);
    // A ball too large for the chord face is refused.
    let big = fillet(&b, &[seam], 9.0);
    assert!(
        matches!(big, Err(KernelError::InvalidInput(_))),
        "{:?}",
        big.err()
    );
}

#[test]
fn an_extruded_spline_edge_is_filleted_by_its_2d_fillet_extruded() {
    // The plate under the parabola through (30, 10), (15, 16), (0, 10): equal chords give
    // the fit parameters 0, ½, 1, so the quadratic is x = 30 (1 - t),
    // y = 10 + 24 t (1 - t), i.e. y(x) = 10 + k x (30 - x) with k = 24/900. Closed by
    // the lines (0, 10) - (0, 0) - (30, 0) - (30, 10) and extruded by h = 5.
    let h = 5.0;
    let s = chamfera_geom::BSplineCurve2::interpolate(
        &[
            Point2::new(30.0, 10.0),
            Point2::new(15.0, 16.0),
            Point2::new(0.0, 10.0),
        ],
        None,
        None,
    )
    .unwrap();
    let p = Profile::new(
        vec![
            crate::ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(30.0, 0.0)),
            crate::ProfileEdge::line(Point2::new(30.0, 0.0), Point2::new(30.0, 10.0)),
            crate::ProfileEdge::Spline(s),
            crate::ProfileEdge::line(Point2::new(0.0, 10.0), Point2::new(0.0, 0.0)),
        ],
        vec![],
    );
    let b = extrude(&Frame::XY, &p, h).unwrap();
    let k = 24.0 / 900.0;
    let y = |x: f64| 10.0 + k * x * (30.0 - x);
    let dy = |x: f64| k * (30.0 - 2.0 * x);
    // ∫ y dx = 10 x + k (15 x² - x³/3).
    let big_y = |x: f64| 10.0 * x + k * (15.0 * x * x - x.powi(3) / 3.0);
    let v0 = big_y(30.0) * h;
    assert_close("plate", b.volume().unwrap(), v0);
    let edge = edge_at(&b, Point3::new(30.0, 10.0, 0.0), Point3::new(30.0, 10.0, h));
    let r = 2.0;
    let out = fillet(&b, &[edge], r).unwrap();
    // The ball centre is r left of x = 30 and r below the parabola: from the foot
    // (s, y(s)) along the inward normal (y'(s), -1)/√(1 + y'²), so
    // s + r y'(s)/√(1 + y'(s)²) = 30 - r, solved by bisection on s in [20, 30].
    let cx = |s: f64| s + r * dy(s) / (1.0 + dy(s) * dy(s)).sqrt();
    let (mut lo, mut hi) = (20.0, 30.0);
    for _ in 0..200 {
        let m = 0.5 * (lo + hi);
        if cx(m) < 30.0 - r {
            lo = m;
        } else {
            hi = m;
        }
    }
    let sf = 0.5 * (lo + hi);
    let cy = y(sf) - r / (1.0 + dy(sf) * dy(sf)).sqrt();
    let ccx = 30.0 - r;
    // The removed region, for s <= x <= 30, lies between the fillet arc
    // y = cy + √(r² - (x - cx)²) (from the foot on the parabola to (30, cy)) and the
    // parabola: ∫ y dx - cy (30 - s) - ∫ √(r² - u²) du over u from s - cx to r, with
    // ∫ √(r² - u²) du = (u √(r² - u²) + r² asin(u / r)) / 2.
    let disc = |u: f64| 0.5 * (u * (r * r - u * u).max(0.0).sqrt() + r * r * (u / r).asin());
    let area = big_y(30.0) - big_y(sf) - cy * (30.0 - sf) - (disc(r) - disc(sf - ccx));
    assert!(area > 0.0 && area < r * r);
    let got = check(&out, 0);
    assert!(
        close(got, v0 - area * h, 1e-9),
        "volume: got {got}, want {}",
        v0 - area * h
    );
    // The fillet is a cylinder along z; the spline face stays an extrusion.
    let s = surfaces(&out);
    assert!(
        s.iter()
            .any(|s| matches!(s, Surface::Cylinder(c) if (c.radius() - r).abs() < 1e-12))
    );
    assert!(s.iter().any(|s| matches!(s, Surface::Extrusion(_))));
    // The chamfer is planar.
    let out = chamfer(&b, &[edge], 1.0).unwrap();
    check(&out, 0);
    assert!(out.volume().unwrap() < v0);
}

/// The area removed per r² by a fillet whose radius grows at `k` per unit length along a
/// right-angled edge (see `a_variable_radius_fillet_is_a_cone`).
fn variable_section(k: f64) -> f64 {
    let k2 = k * k;
    let ab = (1.0 + k2).sqrt() / (1.0 - k2).powf(1.5);
    let phi = ((1.0 - k2) / 2.0).sqrt().asin();
    2.0 / (1.0 - k2) - 0.5 - ab * (phi + phi.sin() * phi.cos())
}

#[test]
fn a_variable_radius_fillet_is_a_cone() {
    // The box edge `top 0` along x (length L = 50) between the top z = 10 and the side
    // y = 0, radius ρ(x) = r1 + k x with k = (r2 - r1) / L. With y into the top and
    // w = 10 - z into the side, the balls have centres (x, ρ, ρ) and radii ρ; their
    // envelope is the cone with apex (-r1/k, 0, 0), axis (1, k, k) and cos²α =
    // (1 + k²)/(1 + 2k²). Its section by the plane x = const is the conic
    // y² + w² - 2k² y w - 2ρ (y + w) + ρ² = 0 (tangent to y = 0 at w = ρ and to w = 0 at
    // y = ρ), which scales with ρ, so the removed volume is A(k) ∫ ρ² dx =
    // A(k) L (r1² + r1 r2 + r2²)/3. In s = (y+w)/√2, d = (y-w)/√2 with ρ = 1 the conic is
    // the ellipse (1-k²)(s - s0)² + (1+k²) d² = (1+k²)/(1-k²), s0 = √2/(1-k²), with
    // semi-axes a = √(1+k²)/(1-k²) and b = 1/√(1-k²); the region between the corner and
    // the near arc for |d| <= 1/√2 has area
    // A = 2/(1-k²) - 1/2 - a b (φ + sin φ cos φ), sin φ = √((1-k²)/2), which is 1 - π/4
    // at k = 0 (the constant fillet).
    let b = block();
    let e = edge(&b, "top 0");
    let (r1, r2, l) = (1.0, 3.0, 50.0);
    // The formula is symmetric in r1 and r2, so either end may be the first vertex.
    let out = fillet_variable(&b, e, r1, r2).unwrap();
    let k = (r2 - r1) / l;
    let removed = variable_section(k) * l * (r1 * r1 + r1 * r2 + r2 * r2) / 3.0;
    assert_close("volume", check(&out, 0), 15000.0 - removed);
    // Not the cylinder's section times the mean square radius: A(k) differs from
    // 1 - π/4 at second order in k.
    assert!((variable_section(k) - (1.0 - PI / 4.0)).abs() > 1e-6);
    let round = face(&out, "fillet of top 0");
    let Some((Surface::Cone(c), true)) = out.face_surface(round).unwrap() else {
        panic!("the variable fillet is not an outward cone");
    };
    // The apex lies on the edge's line, where the radius would be zero.
    let apex = c.apex();
    assert!(
        apex.y.abs() < 1e-9 && (apex.z - 10.0).abs() < 1e-9,
        "{apex:?}"
    );
    let [v0, _] = b.edge_vertices(e).unwrap();
    let x0 = b.vertex_point(v0).unwrap().x;
    let sign = if x0 < 25.0 { 1.0 } else { -1.0 };
    assert_close("apex", apex.x, x0 - sign * r1 / k);
    // Equal radii are the constant fillet; hostile radii and other edges are refused.
    let same = fillet_variable(&b, e, 2.0, 2.0).unwrap();
    assert_close(
        "constant",
        same.volume().unwrap(),
        15000.0 - (1.0 - PI / 4.0) * 4.0 * 50.0,
    );
    for bad in [0.0, -1.0, f64::NAN, 1e300] {
        assert!(matches!(
            fillet_variable(&b, e, 1.0, bad),
            Err(KernelError::InvalidInput(_))
        ));
    }
    assert!(matches!(
        fillet_variable(&b, e, 1.0, 12.0),
        Err(KernelError::InvalidInput(_))
    ));
    let slot = extrude(
        &Frame::XY,
        &Profile::new(slot_edges(10.0, 10.0), vec![]),
        10.0,
    )
    .unwrap();
    let arc = loop_edges(&slot, "top", 0)[1];
    assert!(matches!(
        fillet_variable(&slot, arc, 1.0, 2.0),
        Err(KernelError::Unsupported(_))
    ));
}

#[test]
fn blends_without_an_exact_surface_are_refused_and_leave_the_body_alone() {
    // Two equal pipes in a T: the main one along x, the branch along z ending on its axis.
    let main = extrude(
        &Frame::YZ.with_origin(Point3::new(-20.0, 0.0, 0.0)).unwrap(),
        &Profile::circle(Point2::ORIGIN, 5.0),
        40.0,
    )
    .unwrap();
    let branch = extrude(&Frame::XY, &Profile::circle(Point2::ORIGIN, 5.0), 20.0).unwrap();
    let t = crate::union(&main, &branch).unwrap();
    let before = (t.summary(), t.volume().unwrap());
    let joins: Vec<EdgeId> = t
        .edges()
        .into_iter()
        .filter(|e| {
            let [a, c] = t.edge_faces(*e).unwrap();
            let cyl = |f| matches!(t.face_surface(f).unwrap(), Some((Surface::Cylinder(_), _)));
            a != c && cyl(a) && cyl(c)
        })
        .collect();
    assert!(!joins.is_empty());
    for e in &joins {
        for result in [fillet(&t, &[*e], 1.0), chamfer(&t, &[*e], 1.0)] {
            let Err(KernelError::Unsupported(why)) = result else {
                panic!("a pipe junction blend was not refused");
            };
            assert!(why.contains("NURBS"), "{why}");
        }
    }
    assert_eq!((t.summary(), t.volume().unwrap()), before);
    // The pipes' own end rims are still plain boss rims.
    check(&fillet(&t, &[edge(&t, "bottom 0")], 1.0).unwrap(), 0);

    // A cylinder cut by an oblique plane: its rim is an ellipse.
    let cyl = extrude(&Frame::XY, &Profile::circle(Point2::ORIGIN, 5.0), 20.0).unwrap();
    let tilted = Frame::from_normal(
        Point3::new(0.0, 0.0, 15.0),
        chamfera_geom::Vec3::new(0.3, 0.0, 1.0),
    )
    .unwrap();
    let (low, _) = crate::split_body(&cyl, &tilted, "cut").unwrap();
    let low = if low.volume().unwrap() > 0.5 * cyl.volume().unwrap() {
        low
    } else {
        crate::split_body(&cyl, &tilted, "cut").unwrap().1
    };
    let ellipse = low
        .edges()
        .into_iter()
        .find(|e| {
            matches!(
                low.edge_curve(*e).unwrap(),
                Some((Curve::Ellipse(_) | Curve::EllipticalArc(_), _, _))
            )
        })
        .unwrap();
    let before = (low.summary(), low.volume().unwrap());
    let Err(KernelError::Unsupported(why)) = fillet(&low, &[ellipse], 1.0) else {
        panic!("an elliptical rim fillet was not refused");
    };
    assert!(why.contains("NURBS"), "{why}");
    assert_eq!((low.summary(), low.volume().unwrap()), before);

    // A slot's line filleted alone stops at the tangent join, where the end face is the
    // curved side: refused, not guessed.
    let slot = extrude(
        &Frame::XY,
        &Profile::new(slot_edges(10.0, 10.0), vec![]),
        10.0,
    )
    .unwrap();
    let line = loop_edges(&slot, "top", 0)[0];
    assert!(matches!(
        fillet(&slot, &[line], 1.0),
        Err(KernelError::Unsupported(_))
    ));
}

fn spec(
    distance: f64,
    second: ChamferSecond,
    reference: Option<FaceId>,
    flip: bool,
) -> ChamferSpec {
    ChamferSpec {
        distance,
        second,
        reference,
        flip,
    }
}

#[test]
fn a_two_distance_chamfer_cuts_a_right_triangle_of_unequal_legs() {
    let b = block();
    let top = face(&b, "top");
    let e = edge(&b, "top 0");
    let out = chamfer_with(
        &b,
        &[e],
        &spec(2.0, ChamferSecond::Distance(3.0), Some(top), false),
    )
    .unwrap();
    // Legs 2 on the top and 3 down side 0, along the whole 50 mm edge.
    assert_close("volume", check(&out, 0), 15000.0 - 0.5 * 2.0 * 3.0 * 50.0);
    assert_close(
        "top",
        out.face_area(face(&out, "top")).unwrap(),
        1500.0 - 2.0 * 50.0,
    );
    assert_close(
        "side 0",
        out.face_area(face(&out, "side 0")).unwrap(),
        500.0 - 3.0 * 50.0,
    );
    // Flipped, the 2 goes down the side and the 3 on the top.
    let out = chamfer_with(
        &b,
        &[e],
        &spec(2.0, ChamferSecond::Distance(3.0), Some(top), true),
    )
    .unwrap();
    assert_close(
        "top",
        out.face_area(face(&out, "top")).unwrap(),
        1500.0 - 3.0 * 50.0,
    );
    // An equal second distance is the equal chamfer: 15000 - 2²/2 · 50.
    let a = chamfer_with(
        &b,
        &[e],
        &spec(2.0, ChamferSecond::Distance(2.0), None, false),
    )
    .unwrap();
    assert_close("equal", a.volume().unwrap(), 15000.0 - 2.0 * 50.0);
}

#[test]
fn a_distance_and_angle_chamfer_follows_the_law_of_sines() {
    let b = block();
    let top = face(&b, "top");
    let e = edge(&b, "top 0");
    // A right angle: the other leg is d tan θ.
    let theta = 30f64.to_radians();
    let out = chamfer_with(
        &b,
        &[e],
        &spec(2.0, ChamferSecond::Angle(theta), Some(top), false),
    )
    .unwrap();
    let leg = 2.0 * theta.tan();
    assert_close("volume", check(&out, 0), 15000.0 - 0.5 * 2.0 * leg * 50.0);
    assert_close(
        "side 0",
        out.face_area(face(&out, "side 0")).unwrap(),
        500.0 - leg * 50.0,
    );
    // Faces at 60 degrees: the equilateral prism of side 10, 20 high. Its edge at the
    // origin, d = 2 on side 0 at θ = 90°: d2 = 2 sin 90° / sin 150° = 4, so the cut-off
    // triangle has sides 2 and 4 at 60°, area ½·2·4·sin 60° = 2√3, over 20 mm.
    let tri = vec![
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(5.0, 5.0 * 3f64.sqrt()),
    ];
    let p = extrude(&Frame::XY, &Profile::polygon(&tri), 20.0).unwrap();
    let v0 = 25.0 * 3f64.sqrt() * 20.0;
    let corner = edge_at(&p, Point3::new(0.0, 0.0, 0.0), Point3::new(0.0, 0.0, 20.0));
    let side0 = face(&p, "side 0");
    let out = chamfer_with(
        &p,
        &[corner],
        &spec(2.0, ChamferSecond::Angle(PI / 2.0), Some(side0), false),
    )
    .unwrap();
    assert_close("volume", check(&out, 0), v0 - 2.0 * 3f64.sqrt() * 20.0);
    // The bevel stands at right angles to side 0 (the plane x = 2), from y = 0 to the
    // point 4 along the other side, (2, 2√3): 2√3 wide, 20 high.
    let bevel = out
        .faces()
        .into_iter()
        .find(|f| {
            out.face_name(*f)
                .unwrap()
                .is_some_and(|n| n.starts_with("chamfer of"))
        })
        .unwrap();
    assert_close(
        "bevel",
        out.face_area(bevel).unwrap(),
        2.0 * 3f64.sqrt() * 20.0,
    );
    // An angle the corner cannot take (60° + 130° > 180°) is refused.
    assert!(matches!(
        chamfer_with(
            &p,
            &[corner],
            &spec(
                2.0,
                ChamferSecond::Angle(130f64.to_radians()),
                Some(side0),
                false
            )
        ),
        Err(KernelError::InvalidInput(_))
    ));
}

#[test]
fn an_unequal_rim_chamfer_is_a_cone_by_pappus() {
    let b = plate_with_hole();
    let rim = edge(&b, "hole 0 top 0");
    let top = face(&b, "top");
    let (rc, a, c) = (5.0, 1.0, 2.0);
    let out = chamfer_with(
        &b,
        &[rim],
        &spec(a, ChamferSecond::Distance(c), Some(top), false),
    )
    .unwrap();
    // The meridian triangle has legs a across the top and c down the hole; its centroid
    // is at ρ = Rc + a/3, so it sweeps 2π (Rc + a/3) a c / 2.
    let v0 = 15000.0 - 250.0 * PI;
    assert_close("volume", check(&out, 1), v0 - PI * a * c * (rc + a / 3.0));
    assert!(surfaces(&out).iter().any(|s| matches!(s, Surface::Cone(_))));
    // Flipped: c across the top, a down the hole, centroid at ρ = Rc + c/3.
    let out = chamfer_with(
        &b,
        &[rim],
        &spec(a, ChamferSecond::Distance(c), Some(top), true),
    )
    .unwrap();
    assert_close("flipped", check(&out, 1), v0 - PI * a * c * (rc + c / 3.0));
    // 45 degrees from the top is the equal chamfer.
    let out = chamfer_with(
        &b,
        &[rim],
        &spec(a, ChamferSecond::Angle(PI / 4.0), Some(top), false),
    )
    .unwrap();
    assert_close("volume", check(&out, 1), v0 - PI * a * a * (rc + a / 3.0));
}

#[test]
fn hostile_chamfer_specs_are_refused() {
    let b = block();
    let e = edge(&b, "top 0");
    for second in [
        ChamferSecond::Distance(-1.0),
        ChamferSecond::Distance(f64::NAN),
        ChamferSecond::Distance(0.0),
        ChamferSecond::Angle(0.0),
        ChamferSecond::Angle(PI),
        ChamferSecond::Angle(f64::INFINITY),
        ChamferSecond::Angle(-0.5),
    ] {
        assert!(
            chamfer_with(&b, &[e], &spec(2.0, second, None, false)).is_err(),
            "{second:?}"
        );
    }
    assert!(chamfer_with(&b, &[e], &spec(f64::NAN, ChamferSecond::Equal, None, false)).is_err());
    // Too big for the 10 mm side.
    assert!(
        chamfer_with(
            &b,
            &[e],
            &spec(2.0, ChamferSecond::Distance(11.0), None, false)
        )
        .is_err()
    );
    // A reference face of another body with more faces does not exist here.
    let p = Profile::polygon(
        &(0..12)
            .map(|i| {
                let t = i as f64 / 12.0 * std::f64::consts::TAU;
                Point2::new(t.cos(), t.sin())
            })
            .collect::<Vec<_>>(),
    );
    let big = extrude(&Frame::XY, &p, 1.0).unwrap();
    let alien = *big.faces().last().unwrap();
    assert!(
        chamfer_with(
            &b,
            &[e],
            &spec(2.0, ChamferSecond::Equal, Some(alien), false)
        )
        .is_err()
    );
}

#[test]
fn the_tangent_chain_of_a_slot_line_is_its_whole_rim() {
    let (l, rr, h) = (20.0, 10.0, 10.0);
    let b = extrude(
        &Frame::XY,
        &Profile::new(slot_edges(l / 2.0, rr), vec![]),
        h,
    )
    .unwrap();
    let rim = loop_edges(&b, "top", 0);
    let chain = tangent_chain(&b, &rim[..1]).unwrap();
    assert_eq!(chain.len(), 4);
    assert_eq!(chain[0], rim[0]);
    for e in &rim {
        assert!(chain.contains(e));
    }
    // The bottom rim meets the top one only through vertical edges at right angles.
    let bottom = loop_edges(&b, "bottom", 0);
    assert!(bottom.iter().all(|e| !chain.contains(e)));
    // A box edge meets its neighbours at right angles: it is its own chain.
    let bx = block();
    let e = edge(&bx, "top 0");
    assert_eq!(tangent_chain(&bx, &[e]).unwrap(), vec![e]);
    // And the whole slot rim, filleted as a chain, is the slot fillet.
    assert!(fillet(&b, &chain, 1.0).is_ok());
}
