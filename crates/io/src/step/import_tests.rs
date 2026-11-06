//! STEP import tests: round trips through our writer for every K4 solid, and hand-written
//! files for units, entity order, complex instances, face variants, refusals and hostile
//! input. Expected measures are derived by hand in the comments.

use super::tests::curved::solids;
use super::tests::{box_50_30_10, plate_with_two_holes};
use super::*;

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

/// A bicubic B-spline patch as a complex instance, as other writers produce it.
const BSPLINE: &str = "#360=(BOUNDED_SURFACE()B_SPLINE_SURFACE(1,1,((#1,#2),(#3,#4)),.UNSPECIFIED.,.F.,.F.,.F.)B_SPLINE_SURFACE_WITH_KNOTS((2,2),(2,2),(0.,1.),(0.,1.),.UNSPECIFIED.)GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_SURFACE(((1.,1.),(1.,1.)))REPRESENTATION_ITEM('')SURFACE());";

#[test]
fn a_b_spline_face_is_refused_with_a_clear_message() {
    let lines: Vec<String> = cube_lines("10.", MM, Style::default())
        .into_iter()
        .map(|l| {
            if l.starts_with("#360=") {
                BSPLINE.to_owned()
            } else {
                l
            }
        })
        .collect();
    let text = file(&lines);
    let r = import(&text).unwrap();
    assert!(r.solids.is_empty());
    let [f] = r.failed.as_slice() else {
        panic!("one failure expected");
    };
    assert_eq!(
        (f.entity, f.name.as_str(), f.kind),
        (401, "cube", FailureKind::Unsupported)
    );
    let msg = f.to_string();
    assert!(msg.contains("face #370"), "{msg}");
    assert!(msg.contains("B_SPLINE_SURFACE_WITH_KNOTS"), "{msg}");
    assert!(msg.contains("K10"), "{msg}");
    // read_* refuse the file as a whole, saying why.
    let e = read_bytes(text.as_bytes()).unwrap_err().to_string();
    assert!(
        e.contains("1 of 1 solids") && e.contains("face #370"),
        "{e}"
    );
    // A simple B_SPLINE_CURVE_WITH_KNOTS under an edge is refused the same way.
    let lines: Vec<String> = cube_lines("10.", MM, Style::default())
        .into_iter()
        .map(|l| {
            if l.starts_with("#140=") {
                "#140=B_SPLINE_CURVE_WITH_KNOTS('',1,(#1,#2),.POLYLINE_FORM.,.F.,.F.,(2,2),(0.,1.),.UNSPECIFIED.);".to_owned()
            } else {
                l
            }
        })
        .collect();
    let r = import(&file(&lines)).unwrap();
    let msg = r.failed[0].to_string();
    assert!(
        msg.contains("edge #160") && msg.contains("B_SPLINE_CURVE_WITH_KNOTS"),
        "{msg}"
    );
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
