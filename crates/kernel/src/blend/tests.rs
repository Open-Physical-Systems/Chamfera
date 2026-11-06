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
