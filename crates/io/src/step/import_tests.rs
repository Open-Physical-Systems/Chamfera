//! STEP import tests: round trips through our writer for every K4 solid, and hand-written
//! files for units, entity order, complex instances, face variants, refusals and hostile
//! input. Expected measures are derived by hand in the comments.

use super::tests::curved::solids;
use super::tests::{box_50_30_10, plate_with_two_holes};
use super::*;
use chamfera_geom::{Point3, Vec3};
use chamfera_kernel::Curve;

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(f64::MIN_POSITIVE)
}

// --- Round trips through our own writer ---------------------------------------------------

#[test]
fn every_k4_solid_reads_back_with_its_topology_and_measures() {
    let mut all = solids();
    all.push(("box", box_50_30_10()));
    all.push(("plate with square holes", plate_with_two_holes()));
    for (name, body) in &all {
        let text = write(&[(name, body)], &StepOptions::default()).unwrap();
        let back = import(&text).unwrap();
        assert!(back.failed.is_empty(), "{name}: {:?}", back.failed);
        let [(got_name, got)] = back.solids.as_slice() else {
            panic!("{name}: one solid expected");
        };
        assert_eq!(got_name, name);
        got.validate().unwrap();
        // The same topology, entity for entity.
        assert_eq!(got.summary(), body.summary(), "{name}");
        assert_eq!(
            got.validate().unwrap().genus,
            body.validate().unwrap().genus,
            "{name}"
        );
        // The same exact measures.
        let (v0, v1) = (body.volume().unwrap(), got.volume().unwrap());
        let (a0, a1) = (body.area().unwrap(), got.area().unwrap());
        assert!(rel(v1, v0) < 1e-9, "{name}: volume {v1} vs {v0}");
        assert!(rel(a1, a0) < 1e-9, "{name}: area {a1} vs {a0}");
        // Face names survive.
        let mut n0: Vec<_> = body
            .faces()
            .iter()
            .map(|f| body.face_name(*f).unwrap().map(str::to_owned))
            .collect();
        let mut n1: Vec<_> = got
            .faces()
            .iter()
            .map(|f| got.face_name(*f).unwrap().map(str::to_owned))
            .collect();
        n0.sort();
        n1.sort();
        assert_eq!(n0, n1, "{name}");
    }
}

#[test]
fn several_solids_come_back_in_file_order() {
    let a = box_50_30_10();
    let b = plate_with_two_holes();
    let text = write(&[("a", &a), ("b", &b)], &StepOptions::default()).unwrap();
    let solids = read_bytes(text.as_bytes()).unwrap();
    let names: Vec<&str> = solids.iter().map(|s| s.0.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    // 50 * 30 * 10 = 15000; plate 80 * 40 * 5 - 2 * 10 * 10 * 5 = 15000.
    assert!(rel(solids[0].1.volume().unwrap(), 15_000.0) < 1e-12);
    assert!(rel(solids[1].1.volume().unwrap(), 15_000.0) < 1e-12);
}

#[test]
fn read_file_reads_a_written_file_and_refuses_devices() {
    let dir = std::env::temp_dir().join(format!("chamfera-step-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("box.step");
    write_file(&path, &[("box", &box_50_30_10())], &StepOptions::default()).unwrap();
    let solids = read_file(&path).unwrap();
    assert_eq!(solids.len(), 1);
    assert!(rel(solids[0].1.volume().unwrap(), 15_000.0) < 1e-12);
    std::fs::remove_dir_all(&dir).unwrap();
    for p in ["/dev/zero", "/"] {
        if std::path::Path::new(p).exists() {
            assert!(read_file(std::path::Path::new(p)).is_err());
        }
    }
}

// --- Hand-written files -------------------------------------------------------------------

/// Unit blocks: entities #500.. ending in the representation context #510.
const MM: &str = "#501=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
#502=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));
#510=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#501,#502))REPRESENTATION_CONTEXT('3D',''));";

/// The inch as a conversion-based unit of 25.4 mm (CAx-IF practice).
const INCH: &str = "#500=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
#503=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#500);
#504=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);
#501=(CONVERSION_BASED_UNIT('INCH',#503)LENGTH_UNIT()NAMED_UNIT(#504));
#502=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));
#510=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#501,#502))REPRESENTATION_CONTEXT('3D',''));";

/// Metres, to check SI prefixes: no prefix on METRE is 1000 mm.
const METRE: &str = "#501=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT($,.METRE.));
#510=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#501))REPRESENTATION_CONTEXT('3D',''));";

/// How to write the cube.
#[derive(Clone, Copy, Default)]
struct Style {
    /// Points, planes and faces as complex instances (partial records).
    complex: bool,
    /// `FACE_SURFACE` instead of `ADVANCED_FACE`.
    face_surface: bool,
    /// Every `EDGE_CURVE` from its end to its start with `same_sense .F.`, and every bound
    /// with orientation `.F.` over its edges in reverse order and sense.
    reversed_senses: bool,
}

/// The vertices `i = x + 2y + 4z` of a cube of side `s`, its 12 edges along the axes, and
/// each face's normal and loop (counter-clockwise about the outward normal, worked out by
/// hand as `(edge, along)` pairs; the same table as the kernel's `from_boundary` test).
const PAIRS: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];
/// (outward normal, a corner on the face, loop).
type CubeFace = ([i8; 3], usize, [(usize, bool); 4]);
const FACES: [CubeFace; 6] = [
    (
        [0, 0, -1],
        0,
        [(4, true), (1, true), (5, false), (0, false)],
    ),
    ([0, 0, 1], 7, [(2, true), (7, true), (3, false), (6, false)]),
    (
        [0, -1, 0],
        0,
        [(0, true), (9, true), (2, false), (8, false)],
    ),
    (
        [0, 1, 0],
        7,
        [(10, true), (3, true), (11, false), (1, false)],
    ),
    (
        [-1, 0, 0],
        0,
        [(8, true), (6, true), (10, false), (4, false)],
    ),
    (
        [1, 0, 0],
        7,
        [(5, true), (11, true), (7, false), (9, false)],
    ),
];

fn t(b: bool) -> &'static str {
    if b { ".T." } else { ".F." }
}

/// The DATA lines of a cube of side `s` (file units) at the origin, with `units` and a
/// representation of the solid in context #510.
fn cube_lines(s: &str, units: &str, style: Style) -> Vec<String> {
    let mut l: Vec<String> = Vec::new();
    let coord = |i: usize, bit: usize| {
        if i >> bit & 1 == 1 {
            s.to_owned()
        } else {
            "0.".to_owned()
        }
    };
    for i in 0..8 {
        let c = format!("({},{},{})", coord(i, 0), coord(i, 1), coord(i, 2));
        l.push(if style.complex {
            format!("#{}=(CARTESIAN_POINT({c})GEOMETRIC_REPRESENTATION_ITEM()POINT()REPRESENTATION_ITEM(''));", 1 + i)
        } else {
            format!("#{}=CARTESIAN_POINT('',{c});", 1 + i)
        });
        l.push(format!("#{}=VERTEX_POINT('',#{});", 11 + i, 1 + i));
    }
    for (k, (a, b)) in PAIRS.iter().enumerate() {
        let d = match b - a {
            1 => "(1.,0.,0.)",
            2 => "(0.,1.,0.)",
            _ => "(0.,0.,1.)",
        };
        l.push(format!("#{}=DIRECTION('',{d});", 100 + k));
        l.push(format!("#{}=VECTOR('',#{},{s});", 120 + k, 100 + k));
        l.push(format!("#{}=LINE('',#{},#{});", 140 + k, 1 + a, 120 + k));
        l.push(if style.reversed_senses {
            format!(
                "#{}=EDGE_CURVE('',#{},#{},#{},.F.);",
                160 + k,
                11 + b,
                11 + a,
                140 + k
            )
        } else {
            format!(
                "#{}=EDGE_CURVE('',#{},#{},#{},.T.);",
                160 + k,
                11 + a,
                11 + b,
                140 + k
            )
        });
    }
    for (f, (n, corner, uses)) in FACES.iter().enumerate() {
        let mut oes = Vec::new();
        let mut list: Vec<(usize, bool)> = uses.to_vec();
        if style.reversed_senses {
            // Reversed bound: the edges in reverse order and sense. A use along the
            // kernel cube's edge (a -> b) runs against the written edge (b -> a).
            list.reverse();
            list = list.into_iter().map(|(e, along)| (e, !along)).collect();
        }
        for (j, (e, along)) in list.iter().enumerate() {
            let along = if style.reversed_senses {
                !along
            } else {
                *along
            };
            let id = 200 + 10 * f + j;
            l.push(format!(
                "#{id}=ORIENTED_EDGE('',*,*,#{},{});",
                160 + e,
                t(along)
            ));
            oes.push(format!("#{id}"));
        }
        l.push(format!("#{}=EDGE_LOOP('',({}));", 300 + f, oes.join(",")));
        l.push(format!(
            "#{}=FACE_OUTER_BOUND('',#{},{});",
            310 + f,
            300 + f,
            t(!style.reversed_senses)
        ));
        let normal = format!("({}.,{}.,{}.)", n[0], n[1], n[2]);
        let reference = if n[0] != 0 {
            "(0.,1.,0.)"
        } else {
            "(1.,0.,0.)"
        };
        l.push(format!("#{}=DIRECTION('',{normal});", 330 + f));
        l.push(format!("#{}=DIRECTION('',{reference});", 340 + f));
        l.push(format!(
            "#{}=AXIS2_PLACEMENT_3D('',#{},#{},#{});",
            350 + f,
            1 + corner,
            330 + f,
            340 + f
        ));
        l.push(if style.complex {
            format!("#{}=(ELEMENTARY_SURFACE(#{})GEOMETRIC_REPRESENTATION_ITEM()PLANE()REPRESENTATION_ITEM('')SURFACE());", 360 + f, 350 + f)
        } else {
            format!("#{}=PLANE('',#{});", 360 + f, 350 + f)
        });
        let face = if style.face_surface {
            "FACE_SURFACE"
        } else {
            "ADVANCED_FACE"
        };
        l.push(if style.complex {
            format!("#{}=(ADVANCED_FACE()FACE((#{}))FACE_SURFACE(#{},.T.)REPRESENTATION_ITEM('side {f}')TOPOLOGICAL_REPRESENTATION_ITEM());", 370 + f, 310 + f, 360 + f)
        } else {
            format!("#{}={face}('side {f}',(#{}),#{},.T.);", 370 + f, 310 + f, 360 + f)
        });
    }
    l.push("#400=CLOSED_SHELL('',(#370,#371,#372,#373,#374,#375));".into());
    l.push("#401=MANIFOLD_SOLID_BREP('cube',#400);".into());
    l.extend(units.lines().map(str::to_owned));
    l.push("#520=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#401),#510);".into());
    l
}

fn file(lines: &[String]) -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('hand-written test'),'2;1');\nFILE_NAME('t','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n{}\nENDSEC;\nEND-ISO-10303-21;\n",
        lines.join("\n")
    )
}

fn cube(s: &str, units: &str, style: Style) -> String {
    file(&cube_lines(s, units, style))
}

fn only(text: &str) -> Body {
    let mut solids = read_bytes(text.as_bytes()).unwrap();
    assert_eq!(solids.len(), 1);
    solids.remove(0).1
}

#[test]
fn a_hand_written_cube_imports_in_millimetres() {
    let body = only(&cube("10.", MM, Style::default()));
    let s = body.summary();
    assert_eq!((s.vertices, s.edges, s.faces, s.loops), (8, 12, 6, 6));
    // 10^3 and 6 * 10^2.
    assert!(rel(body.volume().unwrap(), 1000.0) < 1e-12);
    assert!(rel(body.area().unwrap(), 600.0) < 1e-12);
    let names: Vec<_> = body
        .faces()
        .iter()
        .map(|f| body.face_name(*f).unwrap().unwrap().to_owned())
        .collect();
    assert_eq!(names[3], "side 3");
}

#[test]
fn inch_files_are_converted_to_millimetres() {
    // 1 in = 25.4 mm: a 1 in cube is 25.4^3 = 16387.064 mm^3, its area 6 * 645.16 mm^2.
    let body = only(&cube("1.", INCH, Style::default()));
    assert!(
        rel(body.volume().unwrap(), 16_387.064) < 1e-9,
        "{}",
        body.volume().unwrap()
    );
    assert!(rel(body.area().unwrap(), 3_870.96) < 1e-9);
    let b = body.bounding_box().unwrap();
    assert!((b.max.x - 25.4).abs() < 1e-12);
    // Metres: a 0.01 m cube is 10 mm on a side, 1000 mm^3.
    let body = only(&cube("0.01", METRE, Style::default()));
    assert!(rel(body.volume().unwrap(), 1000.0) < 1e-9);
    // No unit context at all: millimetres.
    let lines: Vec<String> = cube_lines("10.", "", Style::default())
        .into_iter()
        .filter(|l| !l.starts_with("#520"))
        .collect();
    assert!(rel(only(&file(&lines)).volume().unwrap(), 1000.0) < 1e-12);
}

#[test]
fn entities_in_reverse_order_import_the_same() {
    let mut lines = cube_lines("10.", MM, Style::default());
    lines.reverse();
    let body = only(&file(&lines));
    assert!(rel(body.volume().unwrap(), 1000.0) < 1e-12);
}

#[test]
fn complex_instances_read_as_their_leaf_entity() {
    let text = cube(
        "10.",
        MM,
        Style {
            complex: true,
            ..Style::default()
        },
    );
    assert!(text.contains("(ADVANCED_FACE()FACE((#310))FACE_SURFACE(#360,.T.)"));
    let body = only(&text);
    assert!(rel(body.volume().unwrap(), 1000.0) < 1e-12);
    // The face's name comes from its REPRESENTATION_ITEM record.
    let f = body.faces()[2];
    assert_eq!(body.face_name(f).unwrap(), Some("side 2"));
}

#[test]
fn face_surface_faces_and_reversed_senses_import_the_same() {
    for style in [
        Style {
            face_surface: true,
            ..Style::default()
        },
        Style {
            reversed_senses: true,
            ..Style::default()
        },
    ] {
        let body = only(&cube("10.", MM, style));
        assert!(rel(body.volume().unwrap(), 1000.0) < 1e-12);
        assert!(rel(body.area().unwrap(), 600.0) < 1e-12);
    }
}

/// The cube's bottom face (z = 0, outward -z) as a bilinear B-spline patch, a complex
/// instance as other writers produce it. Its net is the bottom corners: `P_00 = (0,0)`,
/// `P_01 = (10,0)`, `P_10 = (0,10)`, `P_11 = (10,10)`, so `σ_u = (0,10,0)`, `σ_v =
/// (10,0,0)` and `σ_u × σ_v = (0,0,-100)`: outward, as `same_sense .T.` says.
const BSPLINE: &str = "#360=(BOUNDED_SURFACE()B_SPLINE_SURFACE(1,1,((#1,#2),(#3,#4)),.UNSPECIFIED.,.F.,.F.,.F.)B_SPLINE_SURFACE_WITH_KNOTS((2,2),(2,2),(0.,1.),(0.,1.),.UNSPECIFIED.)GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_SURFACE(((1.,1.),(1.,1.)))REPRESENTATION_ITEM('')SURFACE());";

fn replaced(prefix: &str, line: &str) -> String {
    let lines: Vec<String> = cube_lines("10.", MM, Style::default())
        .into_iter()
        .map(|l| {
            if l.starts_with(prefix) {
                line.to_owned()
            } else {
                l
            }
        })
        .collect();
    file(&lines)
}

#[test]
fn a_flat_b_spline_face_and_b_spline_edges_import_as_exact_planes_and_splines() {
    // The bottom face on its B-spline patch: still a 10 mm cube, 1000 mm^3, 600 mm^2.
    let body = only(&replaced("#360=", BSPLINE));
    assert!(rel(body.volume().unwrap(), 1000.0) < 1e-12);
    assert!(rel(body.area().unwrap(), 600.0) < 1e-12);
    let bottom = body.faces()[0];
    let (s, _) = body.face_surface(bottom).unwrap().unwrap();
    let chamfera_kernel::Surface::Plane(frame) = s else {
        panic!("a plane expected, got {s:?}");
    };
    assert!((frame.z() - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-12);
    // An edge on a degree-1 B_SPLINE_CURVE_WITH_KNOTS (and one on a POLYLINE) keeps its
    // B-spline: the cube is unchanged.
    for line in [
        "#140=B_SPLINE_CURVE_WITH_KNOTS('',1,(#1,#2),.POLYLINE_FORM.,.F.,.F.,(2,2),(0.,1.),.UNSPECIFIED.);",
        "#140=POLYLINE('',(#1,#2));",
        "#140=QUASI_UNIFORM_CURVE('',1,(#1,#2),.POLYLINE_FORM.,.F.,.F.);",
        "#140=BEZIER_CURVE('',1,(#1,#2),.POLYLINE_FORM.,.F.,.F.);",
    ] {
        let body = only(&replaced("#140=", line));
        assert!(rel(body.volume().unwrap(), 1000.0) < 1e-12, "{line}");
        let splines = body
            .edges()
            .into_iter()
            .filter(|e| matches!(body.edge_curve(*e), Ok(Some((Curve::BSpline(_), _, _)))))
            .count();
        assert_eq!(splines, 1, "{line}");
    }
}

#[test]
fn a_doubly_curved_b_spline_face_imports_as_a_free_form_face() {
    // The bottom face's patch with its middle pushed down to -4: a biquadratic dome. Its
    // rows run along y (u) and its columns along x (v), so σ_u × σ_v = y × x = -z, out of
    // the cube. The bottom is z = -4 B1(u) B1(v) with B1(t) = 2t(1 - t), ∫ B1 = 1/3, over
    // x = 10v, y = 10u (linear precision), so the dome adds 4 · 100 / 9 = 400/9 mm³:
    // V = 1000 + 400/9. Its lowest point is z = -4 · ¼ = -1 at (5, 5).
    let dome = "#360=B_SPLINE_SURFACE_WITH_KNOTS('',2,2,((#1,#600,#2),(#601,#602,#603),(#3,#604,#4)),.UNSPECIFIED.,.F.,.F.,.F.,(3,3),(3,3),(0.,1.),(0.,1.),.UNSPECIFIED.);
#600=CARTESIAN_POINT('',(5.,0.,0.));
#601=CARTESIAN_POINT('',(0.,5.,0.));
#602=CARTESIAN_POINT('',(5.,5.,-4.));
#603=CARTESIAN_POINT('',(10.,5.,0.));
#604=CARTESIAN_POINT('',(5.,10.,0.));";
    let text = replaced("#360=", dome);
    let body = only(&text);
    let v = body.volume().unwrap();
    assert!(rel(v, 1000.0 + 400.0 / 9.0) < 1e-10, "{v}");
    let bottom = body.faces()[0];
    let (s, _) = body.face_surface(bottom).unwrap().unwrap();
    assert!(matches!(s, chamfera_kernel::Surface::Nurbs(_)), "{s:?}");
    let b = body.bounding_box().unwrap();
    assert!(b.min.z <= -1.0 + 1e-9 && b.min.z >= -4.0 - 1e-9, "{b:?}");
    // And it goes back out as the same B-spline surface.
    let again = only(&write(&[("dome", &body)], &StepOptions::default()).unwrap());
    assert!(rel(again.volume().unwrap(), 1000.0 + 400.0 / 9.0) < 1e-10);
    assert_eq!(again.summary(), body.summary());
}

/// The sphere of radius `r` about the origin as one free-form face (a rational surface of
/// revolution, The NURBS Book §8.5: the nine-point circle along u times the half-circle
/// meridian along v), with a seam meridian used twice and the two poles as vertices.
fn nurbs_sphere(r: f64) -> Body {
    use chamfera_geom::{BSplineCurve3, BSplineSurface3};
    let w = std::f64::consts::FRAC_1_SQRT_2;
    let circle = [
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (-1.0, 1.0),
        (-1.0, 0.0),
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (1.0, 0.0),
    ];
    let cw = [1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0];
    let mer = [(0.0, -r), (r, -r), (r, 0.0), (r, r), (0.0, r)];
    let mw = [1.0, w, 1.0, w, 1.0];
    let ku = vec![
        0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
    ];
    let kv = vec![0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0];
    let net = circle
        .iter()
        .map(|(x, y)| {
            mer.iter()
                .map(|(rho, z)| Point3::new(rho * x, rho * y, *z))
                .collect()
        })
        .collect();
    let weights = cw
        .iter()
        .map(|a| mw.iter().map(|b| a * b).collect())
        .collect();
    let surface = BSplineSurface3::new(2, 2, ku, kv.clone(), net, Some(weights)).unwrap();
    let seam = BSplineCurve3::new(
        2,
        kv,
        mer.iter()
            .map(|(rho, z)| Point3::new(*rho, 0.0, *z))
            .collect(),
        Some(mw.to_vec()),
    )
    .unwrap();
    Body::from_boundary(
        &[Point3::new(0.0, 0.0, -r), Point3::new(0.0, 0.0, r)],
        &[(0, 1, Curve::BSpline(seam), 0.0, 1.0)],
        &[(
            chamfera_kernel::Surface::nurbs(surface).unwrap(),
            true,
            vec![vec![(0, true), (0, false)]],
        )],
    )
    .unwrap()
}

#[test]
fn a_free_form_sphere_round_trips_through_step_with_its_measures() {
    // r = 3: V = 4/3 π 27 = 36π, A = 4π 9 = 36π. The file holds the rational surface
    // as the complex B_SPLINE_SURFACE instance, closed in u.
    let body = nurbs_sphere(3.0);
    let pi = std::f64::consts::PI;
    assert!(rel(body.volume().unwrap(), 36.0 * pi) < 1e-9);
    let text = write(&[("ball", &body)], &StepOptions::default()).unwrap();
    assert!(text.contains("RATIONAL_B_SPLINE_SURFACE"), "{text}");
    assert!(text.contains("B_SPLINE_SURFACE(2,2,"), "{text}");
    let back = only(&text);
    assert_eq!(back.summary(), body.summary());
    let v = back.volume().unwrap();
    assert!(rel(v, 36.0 * pi) < 1e-9, "{v}");
    let a = back.area().unwrap();
    assert!(rel(a, 36.0 * pi) < 1e-9, "{a}");
    let (s, same) = back.face_surface(back.faces()[0]).unwrap().unwrap();
    assert!(same && matches!(s, chamfera_kernel::Surface::Nurbs(_)));
}

// --- A quarter cylinder on rational B-splines ---------------------------------------------

/// `√2/2`, the middle weight of a rational quadratic quarter circle.
const W: &str = "0.7071067811865476";

/// A wedge face: first oriented edge id, its uses, its surface lines, its name.
type WedgeFace<'a> = (u32, &'a [(u32, bool)], &'a str, &'a str);

/// A quarter of a cylinder of radius 10 about the z axis, height 10, in the first
/// quadrant: vertices `V0 = (0,0,0)`, `V1 = (10,0,0)`, `V2 = (0,10,0)` and `V3..V5` above
/// them at z = 10. The arcs `V1 -> V2` and `V4 -> V5` are rational quadratic B-splines
/// (control points `(10,0)`, `(10,10)`, `(0,10)`, weights 1, √2/2, 1: exactly the quarter
/// circle, The NURBS Book §7.3). Loops run counter-clockwise about each outward normal,
/// worked out by hand: bottom `V0 V2 V1`, top `V3 V4 V5`, the side on y = 0 `V0 V1 V4 V3`
/// (x × z = -y), the side on x = 0 `V0 V3 V5 V2` (z × y = -x), the round side `V1 V2 V5
/// V4` (at V1: +y then +z, y × z = +x, outward). `round` is the round face's surface
/// line(s), `#192`, and its `same_sense`.
fn wedge(round: &str, same_sense: bool) -> String {
    let rational_arc = |id: u32, a: u32, m: u32, b: u32| {
        format!(
            "#{id}=(BOUNDED_CURVE()B_SPLINE_CURVE(2,(#{a},#{m},#{b}),.CIRCULAR_ARC.,.F.,.F.)B_SPLINE_CURVE_WITH_KNOTS((3,3),(0.,1.),.UNSPECIFIED.)CURVE()GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_CURVE((1.,{W},1.))REPRESENTATION_ITEM(''));"
        )
    };
    let mut l: Vec<String> = vec![
        "#1=CARTESIAN_POINT('',(0.,0.,0.));".into(),
        "#2=CARTESIAN_POINT('',(10.,0.,0.));".into(),
        "#3=CARTESIAN_POINT('',(0.,10.,0.));".into(),
        "#4=CARTESIAN_POINT('',(0.,0.,10.));".into(),
        "#5=CARTESIAN_POINT('',(10.,0.,10.));".into(),
        "#6=CARTESIAN_POINT('',(0.,10.,10.));".into(),
        "#61=CARTESIAN_POINT('',(10.,10.,0.));".into(),
        "#62=CARTESIAN_POINT('',(10.,10.,10.));".into(),
        "#20=DIRECTION('',(1.,0.,0.));".into(),
        "#21=DIRECTION('',(0.,1.,0.));".into(),
        "#22=DIRECTION('',(0.,0.,1.));".into(),
        "#23=DIRECTION('',(0.,0.,-1.));".into(),
        "#24=DIRECTION('',(-1.,0.,0.));".into(),
        "#25=DIRECTION('',(0.,-1.,0.));".into(),
        "#30=VECTOR('',#20,1.);".into(),
        "#31=VECTOR('',#21,1.);".into(),
        "#32=VECTOR('',#22,1.);".into(),
        "#40=LINE('',#1,#30);".into(),
        "#41=LINE('',#1,#31);".into(),
        rational_arc(42, 2, 61, 3),
        "#43=LINE('',#4,#30);".into(),
        "#44=LINE('',#4,#31);".into(),
        rational_arc(45, 5, 62, 6),
        "#46=LINE('',#1,#32);".into(),
        "#47=LINE('',#2,#32);".into(),
        "#48=LINE('',#3,#32);".into(),
    ];
    for v in 0..6 {
        l.push(format!("#{}=VERTEX_POINT('',#{});", 11 + v, 1 + v));
    }
    // (edge id, from vertex, to vertex, curve)
    for (e, a, b, c) in [
        (50, 11, 12, 40),
        (51, 11, 13, 41),
        (52, 12, 13, 42),
        (53, 14, 15, 43),
        (54, 14, 16, 44),
        (55, 15, 16, 45),
        (56, 11, 14, 46),
        (57, 12, 15, 47),
        (58, 13, 16, 48),
    ] {
        l.push(format!("#{e}=EDGE_CURVE('',#{a},#{b},#{c},.T.);"));
    }
    // (first oriented edge id, uses, bound id, surface lines, face id, name, same_sense)
    let faces: [WedgeFace; 5] = [
        (
            100,
            &[(51, true), (52, false), (50, false)],
            "#112=AXIS2_PLACEMENT_3D('',#1,#23,#20);\n#113=PLANE('',#112);",
            "bottom",
        ),
        (
            120,
            &[(53, true), (55, true), (54, false)],
            "#132=AXIS2_PLACEMENT_3D('',#4,#22,#20);\n#133=PLANE('',#132);",
            "top",
        ),
        (
            140,
            &[(50, true), (57, true), (53, false), (56, false)],
            "#152=AXIS2_PLACEMENT_3D('',#1,#25,#20);\n#153=PLANE('',#152);",
            "y0",
        ),
        (
            160,
            &[(56, true), (54, true), (58, false), (51, false)],
            "#172=AXIS2_PLACEMENT_3D('',#1,#24,#21);\n#173=PLANE('',#172);",
            "x0",
        ),
        (
            180,
            &[(52, true), (58, true), (55, false), (57, false)],
            round,
            "round",
        ),
    ];
    let mut face_ids = Vec::new();
    for (first, uses, surface, name) in faces {
        let mut oes = Vec::new();
        for (k, (e, along)) in uses.iter().enumerate() {
            let id = first + k as u32;
            l.push(format!("#{id}=ORIENTED_EDGE('',*,*,#{e},{});", t(*along)));
            oes.push(format!("#{id}"));
        }
        l.push(format!(
            "#{}=EDGE_LOOP('',({}));",
            first + 10,
            oes.join(",")
        ));
        l.push(format!(
            "#{}=FACE_OUTER_BOUND('',#{},.T.);",
            first + 11,
            first + 10
        ));
        l.extend(surface.lines().map(str::to_owned));
        let sense = if name == "round" { same_sense } else { true };
        l.push(format!(
            "#{}=ADVANCED_FACE('{name}',(#{}),#{},{});",
            first + 14,
            first + 11,
            first + 13 - u32::from(name == "round"),
            t(sense)
        ));
        face_ids.push(format!("#{}", first + 14));
    }
    l.push(format!("#200=CLOSED_SHELL('',({}));", face_ids.join(",")));
    l.push("#201=MANIFOLD_SOLID_BREP('wedge',#200);".into());
    l.extend(MM.lines().map(str::to_owned));
    l.push("#520=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#201),#510);".into());
    file(&l)
}

/// The round side as a rational B-spline patch of degree 2 along u (the arc) and 1 along
/// v (up), rows `(arc point at z = 0, at z = 10)`: `σ_u × σ_v` is outward.
fn round_patch() -> String {
    format!(
        "#192=(BOUNDED_SURFACE()B_SPLINE_SURFACE(2,1,((#2,#5),(#61,#62),(#3,#6)),.UNSPECIFIED.,.F.,.F.,.F.)B_SPLINE_SURFACE_WITH_KNOTS((3,3),(2,2),(0.,1.),(0.,1.),.UNSPECIFIED.)GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_SURFACE(((1.,1.),({W},{W}),(1.,1.)))REPRESENTATION_ITEM('')SURFACE());"
    )
}

/// Checks the wedge against its closed forms: volume `πr²h/4 = 250π`; area two quarter
/// discs `2 · 25π`, two 10 x 10 squares and the round side `(πr/2) h = 50π`, in all
/// `100π + 200`.
fn check_wedge(text: &str) -> Body {
    let body = only(text);
    let pi = std::f64::consts::PI;
    let v = body.volume().unwrap();
    assert!(rel(v, 250.0 * pi) < 1e-9, "volume {v}");
    let a = body.area().unwrap();
    assert!(rel(a, 100.0 * pi + 200.0) < 1e-9, "area {a}");
    let s = body.summary();
    assert_eq!((s.vertices, s.edges, s.faces), (6, 9, 5));
    let round = body
        .faces()
        .into_iter()
        .find(|f| body.face_name(*f).unwrap() == Some("round"))
        .unwrap();
    let (surface, _) = body.face_surface(round).unwrap().unwrap();
    assert!(matches!(surface, chamfera_kernel::Surface::Extrusion(_)));
    body
}

#[test]
fn a_rational_b_spline_quarter_cylinder_imports_as_an_exact_extrusion() {
    let body = check_wedge(&wedge(&round_patch(), true));
    // The arc edges stay rational B-splines: the midpoint of the bottom one is at 45°.
    let arc = body
        .edges()
        .into_iter()
        .find_map(|e| match body.edge_curve(e) {
            Ok(Some((Curve::BSpline(c), t0, t1))) if c.point_at(t0).z.abs() < 1e-12 => {
                Some(c.point_at(0.5 * (t0 + t1)))
            }
            _ => None,
        })
        .unwrap();
    let h = 10.0 * std::f64::consts::FRAC_1_SQRT_2;
    assert!(arc.distance(Point3::new(h, h, 0.0)) < 1e-12, "{arc:?}");
}

#[test]
fn a_patch_ruled_along_u_turns_the_normal_and_same_sense_turns_it_back() {
    // The same patch with u and v swapped: rows (z = 0, z = 10), the arc along v. Now
    // σ_u = (0,0,10) and σ_v runs along the arc, so σ_u × σ_v points at the axis and the
    // face says .F. to be outward.
    let swapped = format!(
        "#192=(BOUNDED_SURFACE()B_SPLINE_SURFACE(1,2,((#2,#61,#3),(#5,#62,#6)),.UNSPECIFIED.,.F.,.F.,.F.)B_SPLINE_SURFACE_WITH_KNOTS((2,2),(3,3),(0.,1.),(0.,1.),.UNSPECIFIED.)GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_SURFACE(((1.,{W},1.),(1.,{W},1.)))REPRESENTATION_ITEM('')SURFACE());"
    );
    check_wedge(&wedge(&swapped, false));
    // With .T. the face would point inward: not a valid solid.
    let r = import(&wedge(&swapped, true)).unwrap();
    assert!(r.solids.is_empty());
    assert_eq!(r.failed[0].kind, FailureKind::Invalid);
}

#[test]
fn a_surface_of_linear_extrusion_of_a_b_spline_imports_exactly() {
    // The bottom arc swept along +z by 10: σ_u × σ_v = C' × z, outward.
    check_wedge(&wedge(
        "#196=VECTOR('',#22,10.);\n#192=SURFACE_OF_LINEAR_EXTRUSION('',#42,#196);",
        true,
    ));
    // A circle swept along its axis is a cylinder.
    let text = wedge(
        "#196=VECTOR('',#22,10.);\n#197=AXIS2_PLACEMENT_3D('',#1,#22,#20);\n#198=CIRCLE('',#197,10.);\n#192=SURFACE_OF_LINEAR_EXTRUSION('',#198,#196);",
        true,
    );
    let body = only(&text);
    assert!(rel(body.volume().unwrap(), 250.0 * std::f64::consts::PI) < 1e-9);
}

#[test]
fn hostile_b_spline_entities_are_refused_without_panicking() {
    let arc = |body: &str| {
        format!(
            "#42=(BOUNDED_CURVE()B_SPLINE_CURVE({body})CURVE()GEOMETRIC_REPRESENTATION_ITEM()REPRESENTATION_ITEM(''));"
        )
    };
    let good = wedge(&round_patch(), true);
    let line42 = good
        .lines()
        .find(|l| l.starts_with("#42="))
        .unwrap()
        .to_owned();
    let line192 = good
        .lines()
        .find(|l| l.starts_with("#192="))
        .unwrap()
        .to_owned();
    let hostile = [
        (
            line42.clone(),
            line42.replace("B_SPLINE_CURVE(2,", "B_SPLINE_CURVE(99,"),
        ),
        (
            line42.clone(),
            line42.replace("B_SPLINE_CURVE(2,", "B_SPLINE_CURVE(-1,"),
        ),
        (
            line42.clone(),
            line42.replace("((3,3),(0.,1.)", "((3,4000000000),(0.,1.)"),
        ),
        (
            line42.clone(),
            line42.replace("((3,3),(0.,1.)", "((3,3),(1.,0.)"),
        ),
        (
            line42.clone(),
            line42.replace("((3,3),(0.,1.)", "((3),(0.,1.)"),
        ),
        (
            line42.clone(),
            line42.replace("(1.,0.7071067811865476,1.)", "(1.,-1.,1.)"),
        ),
        (
            line42.clone(),
            line42.replace("(1.,0.7071067811865476,1.)", "(1.,1.)"),
        ),
        (
            line42.clone(),
            line42.replace("(#2,#61,#3)", "(#2,#61,#200)"),
        ),
        (line42.clone(), line42.replace("(#2,#61,#3)", "()")),
        (line42.clone(), arc("2,(#2,#61,#3),.UNSPECIFIED.,.F.,.F.")),
        (
            line192.clone(),
            line192.replace("(3,3),(2,2)", "(3,3),(2,9)"),
        ),
        (
            line192.clone(),
            line192.replace("((#2,#5),(#61,#62),(#3,#6))", "((#2,#5),(#61),(#3,#6))"),
        ),
        (line192.clone(), line192.replace("((1.,1.),", "((1.),")),
        (
            line192.clone(),
            line192.replace("B_SPLINE_SURFACE(2,1,", "B_SPLINE_SURFACE(2,0,"),
        ),
        (
            line192.clone(),
            "#192=B_SPLINE_SURFACE('',1,1,((#2,#5),(#3,#6)),.UNSPECIFIED.,.F.,.F.,.F.);".to_owned(),
        ),
        (
            line192.clone(),
            "#192=SURFACE_OF_LINEAR_EXTRUSION('',#42,#1);".to_owned(),
        ),
        (
            line192.clone(),
            "#192=SURFACE_OF_LINEAR_EXTRUSION('',#192,#32);".to_owned(),
        ),
    ];
    for (from, to) in hostile {
        let r = import(&good.replace(&from, &to)).unwrap();
        assert!(r.solids.is_empty(), "{to}");
        assert_eq!(r.failed.len(), 1, "{to}");
    }
    for cut in (0..good.len()).step_by(7) {
        let _ = import(&good[..cut]);
    }
}

#[test]
fn spline_solids_we_export_read_back_with_their_measures() {
    // The arch (a parabola through (0,0), (10,10), (20,0) closed by a line, extruded by 5):
    // the area under y = 10 - (x - 10)²/10 on [0, 20] is 200 - 2000/30 = 400/3, so the
    // volume is 2000/3. The disc (a rational unit circle extruded by 1) is π.
    for (name, body, volume) in [
        ("arch", super::tests::splines::arch(), 2000.0 / 3.0),
        ("disc", super::tests::splines::disc(), std::f64::consts::PI),
    ] {
        let text = write(&[(name, &body)], &StepOptions::default()).unwrap();
        let back = only(&text);
        assert_eq!(back.summary(), body.summary(), "{name}");
        let v = back.volume().unwrap();
        assert!(rel(v, volume) < 1e-9, "{name}: {v}");
        assert!(
            rel(back.area().unwrap(), body.area().unwrap()) < 1e-9,
            "{name}"
        );
    }
}

#[test]
fn a_bad_solid_is_reported_and_the_good_ones_still_import() {
    // A second cube (ids + 1000) without its top face: its edges are used once.
    let good = cube_lines("10.", MM, Style::default());
    let shift = |l: &str| {
        let mut out = String::new();
        let mut chars = l.chars().peekable();
        while let Some(c) = chars.next() {
            out.push(c);
            if c == '#' {
                let mut n = String::new();
                while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                    n.push(*d);
                    chars.next();
                }
                out.push_str(&(n.parse::<u64>().unwrap() + 1000).to_string());
            }
        }
        out
    };
    let mut lines = good.clone();
    for l in &good {
        let l = shift(l).replace("'cube'", "'open'");
        lines.push(if l.starts_with("#1400=") {
            "#1400=CLOSED_SHELL('',(#1370,#1372,#1373,#1374,#1375));".to_owned()
        } else {
            l
        });
    }
    let r = import(&file(&lines)).unwrap();
    assert_eq!(r.solids.len(), 1);
    assert_eq!(r.solids[0].0, "cube");
    let [f] = r.failed.as_slice() else {
        panic!("one failure expected");
    };
    assert_eq!(
        (f.entity, f.name.as_str(), f.kind),
        (1401, "open", FailureKind::Invalid)
    );
    assert!(f.reasons[0].starts_with("edge #11"), "{f}");
    // Voids are refused for now.
    let mut lines = good;
    lines.push("#402=ORIENTED_CLOSED_SHELL('',*,#400,.F.);".into());
    lines.push("#403=BREP_WITH_VOIDS('hollow',#400,(#402));".into());
    let r = import(&file(&lines)).unwrap();
    assert_eq!(r.solids.len(), 1);
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].kind, FailureKind::Unsupported);
    assert!(r.failed[0].to_string().contains("voids"));
}

#[test]
fn truncated_garbage_and_hostile_files_are_refused_without_panicking() {
    let text = cube("1.", INCH, Style::default());
    for cut in (0..text.len()).step_by(5) {
        assert!(import(&text[..cut]).is_err(), "cut at {cut}");
    }
    // Pseudo-random bytes (an LCG), and random bytes after a valid header.
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for n in [0usize, 1, 17, 1000, 20_000] {
        let bytes: Vec<u8> = (0..n)
            .map(|_| {
                x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                (x >> 56) as u8
            })
            .collect();
        assert!(import_bytes(&bytes).is_err());
        let mut b = b"ISO-10303-21;HEADER;ENDSEC;DATA;".to_vec();
        b.extend_from_slice(&bytes);
        assert!(import_bytes(&b).is_err());
    }
    // Well-formed files with hostile content: each solid is refused, nothing panics.
    let hostile = [
        ("#1=", "#1=CARTESIAN_POINT('',(0.,0.));"),
        ("#2=", "#2=CARTESIAN_POINT('',(1.E300,0.,0.));"),
        ("#100=", "#100=DIRECTION('',(0.,0.,0.));"),
        ("#330=", "#330=DIRECTION('',(1.,0.,0.));"),
        ("#360=", "#360=PLANE('',#11);"),
        ("#370=", "#370=ADVANCED_FACE('',(),#360,.T.);"),
        ("#300=", "#300=EDGE_LOOP('',(#300));"),
        ("#200=", "#200=ORIENTED_EDGE('',*,*,#200,.T.);"),
        ("#160=", "#160=EDGE_CURVE('',#11,#11,#140,.T.);"),
        ("#140=", "#140=CIRCLE('',#350,-1.);"),
        ("#401=", "#401=MANIFOLD_SOLID_BREP('cube',#1);"),
        (
            "#503=",
            "#503=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(-25.4),#500);",
        ),
        (
            "#503=",
            "#503=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.),#501);",
        ),
    ];
    for (prefix, replacement) in hostile {
        let lines: Vec<String> = cube_lines("1.", INCH, Style::default())
            .into_iter()
            .map(|l| {
                if l.starts_with(prefix) {
                    replacement.to_owned()
                } else {
                    l
                }
            })
            .collect();
        let r = import(&file(&lines)).unwrap();
        assert!(r.solids.is_empty(), "{replacement}");
        assert_eq!(r.failed.len(), 1, "{replacement}");
    }
    // No solid at all.
    assert!(import(&file(&["#1=CARTESIAN_POINT('',(0.,0.,0.));".to_owned()])).is_err());
}

// --- Faceted B-reps -------------------------------------------------------------------------

/// The faces of a cube of side 10 as corner rings `i = x + 2y + 4z`, each running
/// counter-clockwise about its outward normal (worked out by hand: for -y, 0 -> 1 is +x
/// and 1 -> 5 is +z, x × z = -y; the others likewise).
const CUBE_RINGS: [[usize; 4]; 6] = [
    [0, 2, 3, 1], // -z
    [4, 5, 7, 6], // +z
    [0, 1, 5, 4], // -y
    [2, 6, 7, 3], // +y
    [0, 4, 6, 2], // -x
    [1, 3, 7, 5], // +x
];

fn corner(i: usize) -> String {
    let c = |bit: usize| if i >> bit & 1 == 1 { "10." } else { "0." };
    format!("({},{},{})", c(0), c(1), c(2))
}

/// A faceted cube: `quads` gives one POLY_LOOP per face over shared points, else two
/// triangles per face, each with its own copies of its points (as mesh converters write
/// them). `skip` leaves faces out; `shell` is `CLOSED_SHELL` or `OPEN_SHELL`; the shell
/// is #900 and the item #901 is written by `item`.
fn faceted_cube(quads: bool, skip: &[usize], shell: &str, item: &str) -> String {
    let mut l: Vec<String> = (0..8)
        .map(|i| format!("#{}=CARTESIAN_POINT('',{});", 1 + i, corner(i)))
        .collect();
    let mut faces = Vec::new();
    let mut next = 100;
    for (f, ring) in CUBE_RINGS.iter().enumerate() {
        if skip.contains(&f) {
            continue;
        }
        let polys: Vec<Vec<usize>> = if quads {
            vec![ring.to_vec()]
        } else {
            vec![
                vec![ring[0], ring[1], ring[2]],
                vec![ring[0], ring[2], ring[3]],
            ]
        };
        for poly in polys {
            let pts: Vec<String> = poly
                .iter()
                .map(|c| {
                    if quads {
                        format!("#{}", 1 + c)
                    } else {
                        next += 1;
                        l.push(format!("#{next}=CARTESIAN_POINT('',{});", corner(*c)));
                        format!("#{next}")
                    }
                })
                .collect();
            let (lp, bound, face) = (next + 1, next + 2, next + 3);
            next += 3;
            l.push(format!("#{lp}=POLY_LOOP('',({}));", pts.join(",")));
            l.push(format!("#{bound}=FACE_OUTER_BOUND('',#{lp},.T.);"));
            l.push(format!("#{face}=FACE('',(#{bound}));"));
            faces.push(format!("#{face}"));
        }
    }
    l.push(format!("#900={shell}('',({}));", faces.join(",")));
    l.push(item.to_owned());
    l.extend(MM.lines().map(str::to_owned));
    l.push("#520=FACETED_BREP_SHAPE_REPRESENTATION('',(#901),#510);".into());
    file(&l)
}

#[test]
fn a_faceted_cube_imports_as_an_exact_planar_solid() {
    // 10^3 = 1000 mm^3 and 6 * 100 = 600 mm^2, quads over shared points or triangles
    // with their own copies of the points.
    for quads in [true, false] {
        let text = faceted_cube(
            quads,
            &[],
            "CLOSED_SHELL",
            "#901=FACETED_BREP('facets',#900);",
        );
        let body = only(&text);
        assert!(rel(body.volume().unwrap(), 1000.0) < 1e-12, "quads {quads}");
        assert!(rel(body.area().unwrap(), 600.0) < 1e-12, "quads {quads}");
        // The 8 corners are merged however many copies the file has.
        assert_eq!(body.summary().vertices, 8, "quads {quads}");
        assert_eq!(body.summary().faces, if quads { 6 } else { 12 });
    }
    // A closed shell-based surface model of polygons is a solid too.
    let text = faceted_cube(
        true,
        &[],
        "CLOSED_SHELL",
        "#901=SHELL_BASED_SURFACE_MODEL('model',(#900));",
    );
    assert!(rel(only(&text).volume().unwrap(), 1000.0) < 1e-12);
}

#[test]
fn an_open_or_broken_faceted_shell_comes_in_as_a_mesh() {
    // Without its top: five faces, 10 triangles, area 500, open.
    let text = faceted_cube(
        false,
        &[1],
        "OPEN_SHELL",
        "#901=SHELL_BASED_SURFACE_MODEL('cup',(#900));",
    );
    let r = import(&text).unwrap();
    assert!(r.solids.is_empty() && r.failed.is_empty(), "{:?}", r.failed);
    let [(name, mesh)] = r.meshes.as_slice() else {
        panic!("one mesh expected");
    };
    assert_eq!(name, "cup");
    assert_eq!(mesh.triangles.len(), 10);
    assert!(rel(mesh.area(), 500.0) < 1e-12);
    assert!(r.warnings[0].contains("open shell"), "{:?}", r.warnings);
    // read_bytes wants solids only, and says why not.
    let e = read_bytes(text.as_bytes()).unwrap_err().to_string();
    assert!(e.contains("meshes"), "{e}");
    // A FACETED_BREP whose shell is missing a face is not a solid: a mesh, with the
    // reason from the kernel.
    let text = faceted_cube(
        true,
        &[1],
        "CLOSED_SHELL",
        "#901=FACETED_BREP('broken',#900);",
    );
    let r = import(&text).unwrap();
    assert_eq!(r.meshes.len(), 1);
    assert!(
        r.warnings[0].contains("not a valid solid"),
        "{:?}",
        r.warnings
    );
    // Faces mixing polygons and edge loops, and degenerate polygons, are refused.
    let text = faceted_cube(
        true,
        &[],
        "CLOSED_SHELL",
        "#901=FACETED_BREP('facets',#900);",
    );
    let first = text
        .lines()
        .find(|l| l.contains("POLY_LOOP"))
        .unwrap()
        .to_owned();
    for bad in [
        first.replace("POLY_LOOP('',(#1,#3,#4,#2))", "POLY_LOOP('',(#1,#3))"),
        first.replace("POLY_LOOP('',(#1,#3,#4,#2))", "POLY_LOOP('',(#1,#1,#1,#1))"),
        first.replace(
            "POLY_LOOP('',(#1,#3,#4,#2))",
            "POLY_LOOP('',(#1,#3,#520,#2))",
        ),
        first.replace("POLY_LOOP('',(#1,#3,#4,#2))", "POLY_LOOP('',#1)"),
    ] {
        assert_ne!(bad, first);
        let r = import(&text.replace(&first, &bad)).unwrap();
        assert!(r.solids.is_empty() && r.meshes.is_empty(), "{bad}");
        assert_eq!(r.failed.len(), 1, "{bad}");
    }
    for cut in (0..text.len()).step_by(7) {
        let _ = import(&text[..cut]);
    }
}

// --- Assemblies -----------------------------------------------------------------------------

/// Product structure (entities #1000..): the assembly `asm` uses the part `block` (the
/// 10 mm cube of [`cube_lines`], solid #401 in the representation #520) twice. Usage 1
/// moves it by (100, 0, 0); usage 2 turns it a quarter turn about z (its x axis onto +y)
/// and moves it by (0, 50, 0). `swap` writes usage 2's relationship with the parent's
/// representation as `rep_1` (the other convention some writers use).
fn assembly(swap: bool) -> String {
    let mut l = cube_lines("10.", MM, Style::default());
    l.extend(
        [
            "#1000=APPLICATION_CONTEXT('automotive design');",
            "#1001=PRODUCT_CONTEXT('',#1000,'mechanical');",
            "#1002=PRODUCT_DEFINITION_CONTEXT('part definition',#1000,'design');",
            "#1003=CARTESIAN_POINT('',(0.,0.,0.));",
            "#1010=PRODUCT('asm','Assembly','',(#1001));",
            "#1011=PRODUCT_DEFINITION_FORMATION('','',#1010);",
            "#1012=PRODUCT_DEFINITION('design','',#1011,#1002);",
            "#1013=PRODUCT_DEFINITION_SHAPE('','',#1012);",
            "#1015=AXIS2_PLACEMENT_3D('',#1003,$,$);",
            "#1014=SHAPE_REPRESENTATION('',(#1015),#510);",
            "#1016=SHAPE_DEFINITION_REPRESENTATION(#1013,#1014);",
            "#1020=PRODUCT('block','Block','',(#1001));",
            "#1021=PRODUCT_DEFINITION_FORMATION('','',#1020);",
            "#1022=PRODUCT_DEFINITION('design','',#1021,#1002);",
            "#1023=PRODUCT_DEFINITION_SHAPE('','',#1022);",
            "#1025=AXIS2_PLACEMENT_3D('',#1003,$,$);",
            "#1024=SHAPE_REPRESENTATION('',(#1025),#510);",
            "#1026=SHAPE_DEFINITION_REPRESENTATION(#1023,#1024);",
            "#1027=SHAPE_REPRESENTATION_RELATIONSHIP('','',#1024,#520);",
            "#1030=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','block:1','',#1012,#1022,$);",
            "#1031=PRODUCT_DEFINITION_SHAPE('','',#1030);",
            "#1033=CARTESIAN_POINT('',(100.,0.,0.));",
            "#1032=AXIS2_PLACEMENT_3D('',#1033,$,$);",
            "#1034=ITEM_DEFINED_TRANSFORMATION('','',#1025,#1032);",
            "#1035=(REPRESENTATION_RELATIONSHIP('','',#1024,#1014)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#1034)SHAPE_REPRESENTATION_RELATIONSHIP());",
            "#1036=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#1035,#1031);",
            "#1040=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','block:2','',#1012,#1022,$);",
            "#1041=PRODUCT_DEFINITION_SHAPE('','',#1040);",
            "#1043=CARTESIAN_POINT('',(0.,50.,0.));",
            "#1044=DIRECTION('',(0.,0.,1.));",
            "#1045=DIRECTION('',(0.,1.,0.));",
            "#1042=AXIS2_PLACEMENT_3D('',#1043,#1044,#1045);",
            "#1048=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#1047,#1041);",
        ]
        .map(str::to_owned),
    );
    l.push(if swap {
        "#1046=ITEM_DEFINED_TRANSFORMATION('','',#1042,#1025);\n#1047=(REPRESENTATION_RELATIONSHIP('','',#1014,#1024)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#1046)SHAPE_REPRESENTATION_RELATIONSHIP());".to_owned()
    } else {
        "#1046=ITEM_DEFINED_TRANSFORMATION('','',#1025,#1042);\n#1047=(REPRESENTATION_RELATIONSHIP('','',#1024,#1014)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#1046)SHAPE_REPRESENTATION_RELATIONSHIP());".to_owned()
    });
    file(
        &l.iter()
            .flat_map(|x| x.lines().map(str::to_owned))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn a_two_part_assembly_places_each_usage_by_its_transformation() {
    for swap in [false, true] {
        let r = import(&assembly(swap)).unwrap();
        assert!(
            r.failed.is_empty() && r.warnings.is_empty(),
            "{:?}",
            r.warnings
        );
        assert_eq!(r.solids.len(), 1);
        let [a, b] = r.components.as_slice() else {
            panic!("two components expected, got {:?}", r.components);
        };
        // The single root (asm) is the design itself: both are at the top.
        assert_eq!((a.parent, b.parent), (None, None));
        assert_eq!((a.name.as_str(), b.name.as_str()), ("Block", "Block"));
        assert_eq!(
            (a.solids.as_slice(), b.solids.as_slice()),
            (&[0][..], &[0][..])
        );
        // Usage 1: a translation by (100, 0, 0): the corner (10, 0, 0) goes to (110, 0, 0).
        let p = a.placement.apply_point(Point3::new(10.0, 0.0, 0.0));
        assert!(p.distance(Point3::new(110.0, 0.0, 0.0)) < 1e-12, "{p:?}");
        // Usage 2: x onto +y, so (10, 0, 0) -> (0, 10, 0) + (0, 50, 0) = (0, 60, 0) and
        // (0, 10, 0) -> (-10, 0, 0) + (0, 50, 0) = (-10, 50, 0).
        let p = b.placement.apply_point(Point3::new(10.0, 0.0, 0.0));
        assert!(
            p.distance(Point3::new(0.0, 60.0, 0.0)) < 1e-12,
            "swap {swap}: {p:?}"
        );
        let p = b.placement.apply_point(Point3::new(0.0, 10.0, 0.0));
        assert!(
            p.distance(Point3::new(-10.0, 50.0, 0.0)) < 1e-12,
            "swap {swap}: {p:?}"
        );
        // The placed body keeps its volume.
        let placed = r.solids[0].1.transformed(&b.placement).unwrap();
        assert!(rel(placed.volume().unwrap(), 1000.0) < 1e-12);
        let bb = placed.bounding_box().unwrap();
        assert!(
            bb.min.distance(Point3::new(-10.0, 50.0, 0.0)) < 1e-9,
            "{bb:?}"
        );
        assert!(
            bb.max.distance(Point3::new(0.0, 60.0, 10.0)) < 1e-9,
            "{bb:?}"
        );
    }
}

#[test]
fn hostile_assemblies_never_panic_and_say_what_was_left_out() {
    let good = assembly(false);
    // A usage without a placement stays at its parent's origin, with a warning; one
    // whose transformation is not an item-defined one likewise.
    let r = import(&good.replace(
        "#1036=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#1035,#1031);",
        "",
    ))
    .unwrap();
    assert_eq!(r.components.len(), 2);
    assert_eq!(
        r.components[0].placement,
        chamfera_geom::Transform::IDENTITY
    );
    assert!(
        r.warnings.iter().any(|w| w.contains("#1030")),
        "{:?}",
        r.warnings
    );
    let r = import(&good.replace(
        "#1034=ITEM_DEFINED_TRANSFORMATION('','',#1025,#1032);",
        "#1034=CARTESIAN_POINT('',(1.,2.,3.));",
    ))
    .unwrap();
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("ITEM_DEFINED_TRANSFORMATION")),
        "{:?}",
        r.warnings
    );
    // A product that contains itself: the cycle is cut, nothing hangs.
    let cyclic = good.replace(
        "#1040=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','block:2','',#1012,#1022,$);",
        "#1040=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','loop','',#1022,#1012,$);",
    );
    let r = import(&cyclic).unwrap();
    assert_eq!(r.solids.len(), 1);
    assert!(!r.warnings.is_empty());
    for cut in (0..good.len()).step_by(11) {
        let _ = import(&good[..cut]);
    }
}
