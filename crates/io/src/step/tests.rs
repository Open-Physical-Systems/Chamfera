//! STEP export tests: counts derived by hand from the shapes, exact coordinate round trip
//! through our reader, header, units, escaping and hostile input.

use super::read::{self, Exchange, Param};
use super::*;
use chamfera_geom::{Frame, Point2, Point3, Vec3};
use chamfera_kernel::Body;

fn cuboid(min: Point3, size: Vec3) -> Body {
    Body::cuboid(min, size).unwrap()
}

fn box_50_30_10() -> Body {
    cuboid(Point3::ORIGIN, Vec3::new(50.0, 30.0, 10.0))
}

/// An 80 x 40 x 5 plate with two 10 x 10 square holes.
fn plate_with_two_holes() -> Body {
    let outer = [
        Point2::new(0.0, 0.0),
        Point2::new(80.0, 0.0),
        Point2::new(80.0, 40.0),
        Point2::new(0.0, 40.0),
    ];
    let square = |x: f64, y: f64| {
        vec![
            Point2::new(x, y),
            Point2::new(x + 10.0, y),
            Point2::new(x + 10.0, y + 10.0),
            Point2::new(x, y + 10.0),
        ]
    };
    Body::prism_with_holes(
        &Frame::XY,
        &outer,
        &[square(10.0, 10.0), square(50.0, 10.0)],
        5.0,
    )
    .unwrap()
}

fn export(bodies: &[(&str, &Body)]) -> (String, Exchange) {
    let text = write(bodies, &StepOptions::default()).unwrap();
    let ex = read::parse(&text).unwrap();
    (text, ex)
}

fn bits(p: [f64; 3]) -> [u64; 3] {
    [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()]
}

fn body_points(body: &Body) -> Vec<[u64; 3]> {
    let mut out: Vec<_> = body
        .vertices()
        .into_iter()
        .map(|v| {
            let p = body.vertex_point(v).unwrap();
            bits([p.x, p.y, p.z])
        })
        .collect();
    out.sort_unstable();
    out
}

fn file_points(s: &SolidSummary) -> Vec<[u64; 3]> {
    let mut out: Vec<_> = s.vertex_points.iter().copied().map(bits).collect();
    out.sort_unstable();
    out
}

#[test]
fn a_cuboid_has_six_faces_twelve_edges_and_eight_vertices() {
    let (_, ex) = export(&[("box", &box_50_30_10())]);
    assert_eq!(ex.count("ADVANCED_FACE"), 6);
    assert_eq!(ex.count("EDGE_CURVE"), 12);
    assert_eq!(ex.count("VERTEX_POINT"), 8);
    assert_eq!(ex.count("PLANE"), 6);
    assert_eq!(ex.count("LINE"), 12);
    // Four edges per face, each used by both of its faces: 6 * 4 = 24 = 2 * 12.
    assert_eq!(ex.count("ORIENTED_EDGE"), 24);
    assert_eq!(ex.count("EDGE_LOOP"), 6);
    assert_eq!(ex.count("FACE_OUTER_BOUND"), 6);
    assert_eq!(ex.count("FACE_BOUND"), 0);
    assert_eq!(ex.count("CLOSED_SHELL"), 1);
    assert_eq!(ex.count("MANIFOLD_SOLID_BREP"), 1);
    assert_eq!(ex.count("ADVANCED_BREP_SHAPE_REPRESENTATION"), 1);
    let s = summarize(&ex).unwrap();
    let [solid] = s.solids.as_slice() else {
        panic!("one solid expected");
    };
    assert_eq!(
        (solid.faces, solid.edges, solid.vertices),
        (6, 12, 8),
        "shared edges and vertices are written once"
    );
    assert_eq!(solid.oriented_edges, 24);
    // The corners of the 50 x 30 x 10 box at the origin, as a set.
    let mut corners = Vec::new();
    for x in [0.0, 50.0] {
        for y in [0.0, 30.0] {
            for z in [0.0, 10.0] {
                corners.push(bits([x, y, z]));
            }
        }
    }
    corners.sort_unstable();
    assert_eq!(file_points(solid), corners);
}

#[test]
fn a_plate_with_two_holes_matches_its_topology() {
    let body = plate_with_two_holes();
    let t = body.summary();
    // By hand: outer 4-gon and two 4-gon holes, n + Σm = 12 profile points:
    // V = 2 * 12 = 24, E = 3 * 12 = 36, F = 4 + 2 + 8 = 14; each hole is a ring of the
    // top and of the bottom face, 4 rings; genus 2.
    assert_eq!((t.vertices, t.edges, t.faces, t.rings), (24, 36, 14, 4));
    let (_, ex) = export(&[("plate", &body)]);
    assert_eq!(ex.count("ADVANCED_FACE"), t.faces);
    assert_eq!(ex.count("EDGE_CURVE"), t.edges);
    assert_eq!(ex.count("VERTEX_POINT"), t.vertices);
    assert_eq!(ex.count("FACE_OUTER_BOUND"), t.faces);
    assert_eq!(ex.count("FACE_BOUND"), t.rings);
    assert_eq!(ex.count("ORIENTED_EDGE"), t.half_edges);
    let s = summarize(&ex).unwrap();
    let [solid] = s.solids.as_slice() else {
        panic!("one solid expected");
    };
    assert_eq!(solid.inner_bounds, 4);
    // Euler-Poincare with rings: V - E + F - R = 2 (S - G) = 2 (1 - 2) = -2.
    let euler =
        solid.vertices as i64 - solid.edges as i64 + solid.faces as i64 - solid.inner_bounds as i64;
    assert_eq!(euler, -2);
    assert_eq!(file_points(solid), body_points(&body));
    let mut names = solid.face_names.clone();
    names.sort();
    assert!(names.contains(&"top".to_string()) && names.contains(&"hole 1 side 3".to_string()));
}

#[test]
fn coordinates_round_trip_bit_for_bit() {
    // Corners with no short decimal form (1/3, 0.1 + 50, tiny offsets).
    let body = cuboid(
        Point3::new(0.1, -1.0 / 3.0, 1e-7 * 3.0),
        Vec3::new(50.0 + 1.0 / 7.0, 2.0f64.sqrt(), 10.0 / 3.0),
    );
    let (_, ex) = export(&[("odd", &body)]);
    let s = summarize(&ex).unwrap();
    assert_eq!(file_points(&s.solids[0]), body_points(&body));
}

/// The plane normal of a face, times its `same_sense`, read from the file.
fn face_normal(ex: &Exchange, face: &[Param]) -> [f64; 3] {
    let get = |id: &Param| match id {
        Param::Ref(r) => ex.data[r].records[0].params.clone(),
        _ => panic!("a reference expected"),
    };
    let plane = get(&face[2]);
    let axis = get(&plane[1]);
    let dir = get(&axis[2]);
    let Param::List(c) = &dir[1] else { panic!() };
    let n: Vec<f64> = c
        .iter()
        .map(|p| match p {
            Param::Real(v) => *v,
            _ => panic!(),
        })
        .collect();
    let s = if face[3] == Param::Enum("T".into()) {
        1.0
    } else {
        -1.0
    };
    [n[0] * s, n[1] * s, n[2] * s]
}

/// The polygon of a bound, from the start vertex of each oriented edge.
fn bound_polygon(ex: &Exchange, bound: u64) -> Vec<[f64; 3]> {
    let params = |r: u64| ex.data[&r].records[0].params.clone();
    let r = |p: &Param| match p {
        Param::Ref(r) => *r,
        _ => panic!(),
    };
    let bp = params(bound);
    let lp = params(r(&bp[1]));
    let Param::List(oes) = &lp[1] else { panic!() };
    oes.iter()
        .map(|oe| {
            let op = params(r(oe));
            let ep = params(r(&op[3]));
            let v = if op[4] == Param::Enum("T".into()) {
                r(&ep[1])
            } else {
                r(&ep[2])
            };
            let pp = params(r(&params(v)[1]));
            let Param::List(c) = &pp[1] else { panic!() };
            let f = |i: usize| match c[i] {
                Param::Real(x) => x,
                _ => panic!(),
            };
            [f(0), f(1), f(2)]
        })
        .collect()
}

#[test]
fn bounds_run_counter_clockwise_about_the_outward_normal() {
    // Newell's vector area of each bound polygon must point along the face's outward
    // normal (surface normal times same_sense) for an outer bound and against it for a
    // hole: the face lies to the left of its bounds.
    let (_, ex) = export(&[("plate", &plate_with_two_holes())]);
    let mut checked = 0;
    for inst in ex.data.values() {
        if inst.simple_name() != Some("ADVANCED_FACE") {
            continue;
        }
        let face = &inst.records[0].params;
        let n = face_normal(&ex, face);
        let Param::List(bounds) = &face[1] else {
            panic!()
        };
        for b in bounds {
            let Param::Ref(b) = b else { panic!() };
            let poly = bound_polygon(&ex, *b);
            let mut a = [0.0; 3];
            for i in 0..poly.len() {
                let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
                a[0] += (p[1] - q[1]) * (p[2] + q[2]);
                a[1] += (p[2] - q[2]) * (p[0] + q[0]);
                a[2] += (p[0] - q[0]) * (p[1] + q[1]);
            }
            let dot = a[0] * n[0] + a[1] * n[1] + a[2] * n[2];
            let outer = ex.data[b].simple_name() == Some("FACE_OUTER_BOUND");
            assert!(if outer { dot > 0.0 } else { dot < 0.0 }, "bound #{b}");
            checked += 1;
        }
    }
    assert_eq!(checked, 14 + 4);
}

#[test]
fn every_reference_resolves_and_dangling_ones_are_refused() {
    let (text, ex) = export(&[("box", &box_50_30_10())]);
    // `parse` checks every reference; check again by hand here.
    fn refs(p: &Param, out: &mut Vec<u64>) {
        match p {
            Param::Ref(r) => out.push(*r),
            Param::List(l) | Param::Typed(_, l) => l.iter().for_each(|q| refs(q, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    for i in ex.data.values() {
        for r in &i.records {
            r.params.iter().for_each(|p| refs(p, &mut all));
        }
    }
    assert!(!all.is_empty());
    assert!(all.iter().all(|r| ex.data.contains_key(r)));
    let last = *ex.data.keys().last().unwrap();
    let broken = text.replacen("#1)", &format!("#{})", last + 1), 1);
    assert_ne!(broken, text);
    assert!(read::parse(&broken).is_err());
}

#[test]
fn the_header_names_the_file_schema_and_system() {
    let options = StepOptions {
        file_name: "box.step".into(),
        product_name: "Box".into(),
        author: "A. Author".into(),
        organization: "Open Physical Systems".into(),
        timestamp: Some("2026-11-06T12:00:00".into()),
        ..StepOptions::default()
    };
    let text = write(&[("box", &box_50_30_10())], &options).unwrap();
    assert!(text.starts_with("ISO-10303-21;\nHEADER;\n"));
    assert!(text.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"));
    let ex = read::parse(&text).unwrap();
    let d = ex.header_record("FILE_DESCRIPTION").unwrap();
    assert_eq!(d.params[1], Param::String("2;1".into()));
    let n = ex.header_record("FILE_NAME").unwrap();
    assert_eq!(n.params[0], Param::String("box.step".into()));
    assert_eq!(n.params[1], Param::String("2026-11-06T12:00:00".into()));
    assert_eq!(
        n.params[2],
        Param::List(vec![Param::String("A. Author".into())])
    );
    assert_eq!(
        n.params[3],
        Param::List(vec![Param::String("Open Physical Systems".into())])
    );
    assert_eq!(n.params[5], Param::String("Chamfera".into()));
    let s = ex.header_record("FILE_SCHEMA").unwrap();
    assert_eq!(
        s.params[0],
        Param::List(vec![Param::String("AUTOMOTIVE_DESIGN".into())])
    );
    for e in [
        "APPLICATION_CONTEXT",
        "APPLICATION_PROTOCOL_DEFINITION",
        "PRODUCT_CONTEXT",
        "PRODUCT",
        "PRODUCT_DEFINITION_FORMATION",
        "PRODUCT_DEFINITION_CONTEXT",
        "PRODUCT_DEFINITION",
        "PRODUCT_DEFINITION_SHAPE",
        "SHAPE_DEFINITION_REPRESENTATION",
    ] {
        assert_eq!(ex.count(e), 1, "{e}");
    }
    // Without a timestamp the pure writer uses the epoch (it has no clock).
    let t = write(&[("box", &box_50_30_10())], &StepOptions::default()).unwrap();
    assert!(t.contains("'1970-01-01T00:00:00'"));
}

#[test]
fn the_file_is_plain_ascii() {
    let (text, _) = export(&[("Gehäuse 日本 😀", &plate_with_two_holes())]);
    assert!(text.is_ascii());
    assert!(!text.contains('\t') && !text.contains('\r'));
}

#[test]
fn units_are_millimetres_radians_and_steradians() {
    let (_, ex) = export(&[("box", &box_50_30_10())]);
    let enums = |r: &read::Record| -> Vec<Param> { r.params.clone() };
    let unit = |kind: &str| {
        let inst = ex
            .data
            .values()
            .find(|i| i.record(kind).is_some())
            .unwrap_or_else(|| panic!("{kind}"));
        enums(inst.record("SI_UNIT").unwrap())
    };
    assert_eq!(
        unit("LENGTH_UNIT"),
        vec![Param::Enum("MILLI".into()), Param::Enum("METRE".into())]
    );
    assert_eq!(
        unit("PLANE_ANGLE_UNIT"),
        vec![Param::Unset, Param::Enum("RADIAN".into())]
    );
    assert_eq!(
        unit("SOLID_ANGLE_UNIT"),
        vec![Param::Unset, Param::Enum("STERADIAN".into())]
    );
    let u = ex
        .data
        .values()
        .find_map(|i| i.record("UNCERTAINTY_MEASURE_WITH_UNIT"))
        .unwrap();
    assert_eq!(
        u.params[0],
        Param::Typed(
            "LENGTH_MEASURE".into(),
            vec![Param::Real(chamfera_geom::LINEAR_TOLERANCE)]
        )
    );
    let ctx = ex
        .data
        .values()
        .find(|i| i.record("GLOBAL_UNIT_ASSIGNED_CONTEXT").is_some())
        .unwrap();
    assert!(
        ctx.record("GEOMETRIC_REPRESENTATION_CONTEXT")
            .unwrap()
            .params
            == [Param::Integer(3)]
    );
    assert!(ctx.record("GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT").is_some());
}

#[test]
fn names_with_quotes_and_non_ascii_read_back() {
    let name = "O'Brien's \\ \"bracket\" Düsseldorf 日本 😀 \n end";
    let options = StepOptions {
        product_name: name.into(),
        ..StepOptions::default()
    };
    let text = write(&[(name, &box_50_30_10())], &options).unwrap();
    assert!(text.is_ascii());
    let ex = read::parse(&text).unwrap();
    let s = summarize(&ex).unwrap();
    assert_eq!(s.solids[0].name, name);
    let product = ex
        .data
        .values()
        .find_map(|i| (i.simple_name() == Some("PRODUCT")).then(|| &i.records[0]))
        .unwrap();
    assert_eq!(product.params[0], Param::String(name.into()));
}

#[test]
fn several_bodies_become_several_solids_of_one_product() {
    let a = box_50_30_10();
    let b = cuboid(Point3::new(100.0, 0.0, 0.0), Vec3::new(5.0, 5.0, 5.0));
    let (_, ex) = export(&[("a", &a), ("b", &b)]);
    assert_eq!(ex.count("PRODUCT"), 1);
    assert_eq!(ex.count("MANIFOLD_SOLID_BREP"), 2);
    assert_eq!(ex.count("ADVANCED_FACE"), 12);
    assert_eq!(ex.count("VERTEX_POINT"), 16);
    let s = summarize(&ex).unwrap();
    let names: Vec<_> = s.solids.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(file_points(&s.solids[1]), body_points(&b));
}

#[test]
fn write_file_fills_in_the_name_and_the_time() {
    let dir = std::env::temp_dir().join(format!("chamfera-step-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("box.step");
    write_file(&path, &[("box", &box_50_30_10())], &StepOptions::default()).unwrap();
    let bytes = crate::read_limited(&path, MAX_STEP_BYTES as u64).unwrap();
    let ex = read::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
    let n = ex.header_record("FILE_NAME").unwrap();
    assert_eq!(n.params[0], Param::String("box.step".into()));
    let Param::String(t) = &n.params[1] else {
        panic!()
    };
    assert_eq!(t.len(), "2026-11-06T12:00:00".len());
    assert_ne!(t, "1970-01-01T00:00:00");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hostile_exports_give_errors_not_panics() {
    let body = box_50_30_10();
    // Over the entity budget.
    let small = StepOptions {
        max_entities: 50,
        ..StepOptions::default()
    };
    assert!(write(&[("box", &body)], &small).is_err());
    // Nothing to write.
    assert!(write(&[], &StepOptions::default()).is_err());
    assert!(write(&[("empty", &Body::new())], &StepOptions::default()).is_err());
    // Oversized names.
    let long = "x".repeat(MAX_STEP_TEXT + 1);
    assert!(write(&[(long.as_str(), &body)], &StepOptions::default()).is_err());
    let options = StepOptions {
        author: long.clone(),
        ..StepOptions::default()
    };
    assert!(write(&[("box", &body)], &options).is_err());
    // A body that is not a finished solid (the cylinder's lateral face has no surface
    // until K3).
    let cyl = Body::cylinder_topology(&Frame::XY, 5.0, 10.0).unwrap();
    assert!(write(&[("cyl", &cyl)], &StepOptions::default()).is_err());
    // A body with a NaN coordinate cannot be built: constructors and deserialization
    // validate every point. Tampering with a saved body is refused on load, so the
    // writer never sees it (it also checks every number it writes, see format.rs).
    let json = serde_json::to_string(&body).unwrap();
    let tampered = json.replacen("50.0", "1e400", 1);
    assert_ne!(json, tampered);
    assert!(serde_json::from_str::<Body>(&tampered).is_err());
}

#[test]
fn hostile_step_text_gives_errors_not_panics() {
    let (text, _) = export(&[("box", &box_50_30_10())]);
    // Every truncation of a valid file.
    for cut in (0..text.len()).step_by(7) {
        assert!(read::parse(&text[..cut]).is_err(), "cut at {cut}");
    }
    let deep = format!(
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A({}{});ENDSEC;END-ISO-10303-21;",
        "(".repeat(10_000),
        ")".repeat(10_000)
    );
    let cases = [
        String::new(),
        "garbage".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A(#2);ENDSEC;END-ISO-10303-21;".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A();#1=B();ENDSEC;END-ISO-10303-21;".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A('open);ENDSEC;END-ISO-10303-21;".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A('\\X2\\D800\\X0\\');ENDSEC;END-ISO-10303-21;".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A('\\X4\\FFFFFFFF\\X0\\');ENDSEC;END-ISO-10303-21;"
            .into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A('\\Q\\');ENDSEC;END-ISO-10303-21;".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A(1.E999);ENDSEC;END-ISO-10303-21;".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A(99999999999999999999);ENDSEC;END-ISO-10303-21;"
            .into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#99999999999999999999=A();ENDSEC;END-ISO-10303-21;".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=();ENDSEC;END-ISO-10303-21;".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A(.T);ENDSEC;END-ISO-10303-21;".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;/* never closed".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A('é');ENDSEC;END-ISO-10303-21;".into(),
        "ISO-10303-21;HEADER;ENDSEC;DATA;ENDSEC;END-ISO-10303-21; trailing".into(),
        deep,
    ];
    for c in &cases {
        assert!(read::parse(c).is_err(), "{c:.80}");
    }
    // Well-formed but not a closed shell: one oriented edge flipped.
    let ex = read::parse(&text).unwrap();
    let id = ex
        .data
        .iter()
        .find(|(_, i)| i.simple_name() == Some("ORIENTED_EDGE"))
        .map(|(id, _)| *id)
        .unwrap();
    let line = text
        .lines()
        .find(|l| l.starts_with(&format!("#{id}=ORIENTED_EDGE")))
        .unwrap();
    let swapped = if line.ends_with(".T.);") {
        line.replace(".T.);", ".F.);")
    } else {
        line.replace(".F.);", ".T.);")
    };
    let bad = text.replace(line, &swapped);
    let ex = read::parse(&bad).unwrap();
    assert!(summarize(&ex).is_err());
    // A minimal valid file with comments and an edition-3 DATA header parses.
    let ok = "ISO-10303-21;/*c*/HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA(('d'),('X'));#1=A(1,-2.5E+01,'',$,*,.T.,\"0F\",(#1),B(3.));ENDSEC;END-ISO-10303-21;";
    let ex = read::parse(ok).unwrap();
    assert_eq!(ex.data[&1].records[0].params.len(), 9);
}

/// Runs an external Part 21 validator when one is configured: set
/// `CHAMFERA_STEP_VALIDATOR` to an executable (a permissively licensed checker, e.g. a
/// STEPcode `p21read` built for AP214) that takes a file path and exits 0 for a valid
/// file. Skipped otherwise; no validator is part of the repository or CI.
#[test]
fn an_external_validator_accepts_the_file_when_installed() {
    let Some(tool) = std::env::var_os("CHAMFERA_STEP_VALIDATOR") else {
        eprintln!("skipped: CHAMFERA_STEP_VALIDATOR is not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("chamfera-step-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plate.step");
    write_file(
        &path,
        &[("plate", &plate_with_two_holes())],
        &StepOptions::default(),
    )
    .unwrap();
    let status = std::process::Command::new(tool)
        .arg(&path)
        .status()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(status.success(), "the validator rejected the file");
}
