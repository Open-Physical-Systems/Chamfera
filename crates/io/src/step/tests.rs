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
    // A body that is not a finished solid: MVFS's skeleton (a face of one vertex, with
    // no surface).
    let mut skeleton = Body::new();
    skeleton.mvfs(Point3::ORIGIN).unwrap();
    assert!(write(&[("skeleton", &skeleton)], &StepOptions::default()).is_err());
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

// --- Curved faces (K3 surfaces from K4 sweeps) ------------------------------------------

mod curved {
    use super::*;
    use chamfera_geom::Vec2;
    use chamfera_kernel::{
        Line2, Profile, ProfileEdge, Surface, TessOptions, extrude, polygon_edges, revolve,
        tessellate,
    };
    use std::f64::consts::{FRAC_PI_2, PI, TAU};

    fn y_axis() -> Line2 {
        Line2::new(Point2::ORIGIN, Vec2::Y).unwrap()
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

    fn cone_profile() -> Profile {
        Profile::polygon(&[Point2::ORIGIN, Point2::new(3.0, 0.0), Point2::new(0.0, 4.0)])
    }

    /// Every kind of K4 solid, with a name.
    fn solids() -> Vec<(&'static str, Body)> {
        let xy = Frame::XY;
        let rect = polygon_edges(&[
            Point2::new(0.0, 0.0),
            Point2::new(80.0, 0.0),
            Point2::new(80.0, 40.0),
            Point2::new(0.0, 40.0),
        ]);
        let plate = Profile::new(
            rect,
            vec![
                vec![ProfileEdge::circle(Point2::new(20.0, 20.0), 5.0)],
                vec![ProfileEdge::circle(Point2::new(60.0, 20.0), 8.0)],
            ],
        );
        let slot = Profile::new(
            vec![
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
            ],
            vec![],
        );
        let tube = Profile::polygon(&[
            Point2::new(3.0, 0.0),
            Point2::new(5.0, 0.0),
            Point2::new(5.0, 4.0),
            Point2::new(3.0, 4.0),
        ]);
        let torus = Profile::circle(Point2::new(5.0, 0.0), 2.0);
        // Spindle: the arc of the circle about (1, 0), radius 2, right of the axis.
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
        // Lemon: the circular segment between x = 1 and the arc about (-1, 0), radius √5.
        let half = 0.5f64.atan();
        let lemon = Profile::new(
            vec![
                ProfileEdge::Arc {
                    centre: Point2::new(-1.0, 0.0),
                    radius: 5f64.sqrt(),
                    start_angle: -half,
                    sweep: 2.0 * half,
                },
                ProfileEdge::line(Point2::new(1.0, 1.0), Point2::new(1.0, -1.0)),
            ],
            vec![],
        );
        let tilted = Frame::from_normal_and_x(
            Point3::new(-3.0, 7.0, 1.0),
            Vec3::new(-2.0, 1.0, 4.0),
            Vec3::new(1.0, 1.0, 0.0),
        )
        .unwrap();
        vec![
            (
                "cylinder",
                extrude(&xy, &Profile::circle(Point2::ORIGIN, 5.0), 10.0).unwrap(),
            ),
            (
                "plate with round holes",
                extrude(&xy, &plate, 10.0).unwrap(),
            ),
            ("slot", extrude(&xy, &slot, 10.0).unwrap()),
            ("tube", revolve(&xy, &tube, &y_axis(), TAU).unwrap()),
            (
                "sphere",
                revolve(&xy, &half_disc(3.0), &y_axis(), TAU).unwrap(),
            ),
            ("torus", revolve(&xy, &torus, &y_axis(), TAU).unwrap()),
            (
                "cone",
                revolve(&xy, &cone_profile(), &y_axis(), TAU).unwrap(),
            ),
            (
                "half ball",
                revolve(&xy, &half_disc(3.0), &y_axis(), PI).unwrap(),
            ),
            (
                "quarter torus",
                revolve(&xy, &torus, &y_axis(), FRAC_PI_2).unwrap(),
            ),
            (
                "quarter cone",
                revolve(&xy, &cone_profile(), &y_axis(), FRAC_PI_2).unwrap(),
            ),
            ("apple", revolve(&xy, &apple, &y_axis(), TAU).unwrap()),
            ("lemon", revolve(&xy, &lemon, &y_axis(), TAU).unwrap()),
            (
                "tilted torus part",
                revolve(&tilted, &torus, &y_axis(), -2.0).unwrap(),
            ),
        ]
    }

    /// The STEP entity a kernel surface should become (see `write::StepSurface`).
    fn entity_for(s: &Surface) -> &'static str {
        match s {
            Surface::Plane(_) => "PLANE",
            Surface::Cylinder(_) => "CYLINDRICAL_SURFACE",
            Surface::Cone(_) => "CONICAL_SURFACE",
            Surface::Sphere(_) => "SPHERICAL_SURFACE",
            Surface::Torus(t) if t.major_radius() >= t.minor_radius() => "TOROIDAL_SURFACE",
            Surface::Torus(_) => "DEGENERATE_TOROIDAL_SURFACE",
        }
    }

    fn only_solid(ex: &Exchange) -> SolidSummary {
        let s = summarize(ex).unwrap();
        let [solid] = s.solids.as_slice() else {
            panic!("one solid expected");
        };
        solid.clone()
    }

    #[test]
    fn every_k4_solid_exports_with_counts_matching_its_topology() {
        for (name, body) in solids() {
            let (_, ex) = export(&[(name, &body)]);
            let t = body.summary();
            let s = only_solid(&ex);
            // One-to-one topology: faces, edges, vertices, a bound per loop (inner ones
            // for rings), an oriented edge per half-edge.
            assert_eq!(
                (s.faces, s.edges, s.vertices),
                (t.faces, t.edges, t.vertices),
                "{name}"
            );
            assert_eq!(s.outer_bounds + s.inner_bounds, t.loops, "{name}");
            assert_eq!(s.inner_bounds, t.rings, "{name}");
            assert_eq!(s.oriented_edges, t.half_edges, "{name}");
            assert_eq!(ex.count("ADVANCED_FACE"), t.faces, "{name}");
            assert_eq!(ex.count("EDGE_CURVE"), t.edges, "{name}");
            assert_eq!(ex.count("VERTEX_POINT"), t.vertices, "{name}");
            // Vertex coordinates round-trip exactly.
            assert_eq!(file_points(&s), body_points(&body), "{name}");
            // Each face's surface is the entity of the table, and the counts per entity
            // match the body's faces.
            let mut want: Vec<&str> = body
                .faces()
                .into_iter()
                .map(|f| entity_for(&body.face_surface(f).unwrap().unwrap().0))
                .collect();
            let mut got: Vec<&str> = s.face_surfaces.iter().map(|f| f.entity.as_str()).collect();
            want.sort_unstable();
            got.sort_unstable();
            assert_eq!(got, want, "{name}");
        }
    }

    fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
    fn arr(v: Vec3) -> [f64; 3] {
        [v.x, v.y, v.z]
    }

    /// Local coordinates `(x, y, z)` of `p` in the written placement (y = z × x).
    fn local(f: &SurfaceSummary, p: [f64; 3]) -> [f64; 3] {
        let y = cross(f.axis, f.ref_direction);
        let d = sub(p, f.origin);
        [dot(d, f.ref_direction), dot(d, y), dot(d, f.axis)]
    }

    /// Part 42's `σ(u, v)` of the written surface, in its placement's local coordinates
    /// (the formulas of ISO 10303-42, written out here independently of the writer).
    fn sigma(f: &SurfaceSummary, u: f64, v: f64) -> [f64; 3] {
        let (c, s) = (u.cos(), u.sin());
        let at = |rho: f64, h: f64| [rho * c, rho * s, h];
        match (f.entity.as_str(), f.values.as_slice()) {
            ("CYLINDRICAL_SURFACE", [r]) => at(*r, v),
            ("CONICAL_SURFACE", [r, a]) => at(r + v * a.tan(), v),
            ("SPHERICAL_SURFACE", [r]) => at(r * v.cos(), r * v.sin()),
            (_, [big, a]) => at(big + a * v.cos(), a * v.sin()),
            _ => panic!("no σ for {}", f.entity),
        }
    }

    /// Part 42's `(u, v)` of a local point `l` of the written surface.
    fn parameters(f: &SurfaceSummary, l: [f64; 3]) -> (f64, f64) {
        let rho = l[0].hypot(l[1]);
        let u = l[1].atan2(l[0]);
        match (f.entity.as_str(), f.values.as_slice()) {
            ("CYLINDRICAL_SURFACE", _) | ("CONICAL_SURFACE", _) => (u, l[2]),
            ("SPHERICAL_SURFACE", _) => (u, l[2].atan2(rho)),
            (_, [big, _]) if f.select_outer == Some(false) => {
                // The inner (lemon) part: the point's tube circle is centred at -R e(u),
                // so it is σ(u + π, v) with R + a cos v = -ρ.
                (u + PI, l[2].atan2(-rho - big))
            }
            (_, [big, _]) => (u, l[2].atan2(rho - big)),
            _ => panic!("no parameters for {}", f.entity),
        }
    }

    /// The written face's normal (Part 42: along `σ_u × σ_v`, by central differences;
    /// a plane's is its axis) times its `same_sense`, in local coordinates.
    fn written_outward(f: &SurfaceSummary, p: [f64; 3]) -> [f64; 3] {
        let n = if f.entity == "PLANE" {
            [0.0, 0.0, 1.0]
        } else {
            let (u, v) = parameters(f, local(f, p));
            // σ reproduces the point.
            let back = sigma(f, u, v);
            assert!(
                sub(back, local(f, p)).iter().all(|d| d.abs() < 1e-9),
                "{} does not pass through {p:?}",
                f.entity
            );
            let h = 1e-6;
            let su = sub(sigma(f, u + h, v), sigma(f, u - h, v));
            let sv = sub(sigma(f, u, v + h), sigma(f, u, v - h));
            cross(su, sv)
        };
        let s = if f.same_sense { 1.0 } else { -1.0 };
        // Back to world coordinates.
        let y = cross(f.axis, f.ref_direction);
        [0, 1, 2].map(|i| s * (f.ref_direction[i] * n[0] + y[i] * n[1] + f.axis[i] * n[2]))
    }

    #[test]
    fn same_sense_agrees_with_the_outward_normal_on_every_face() {
        // For each face, at a mesh vertex of the face away from the surface's axis: the
        // written surface passes through it, and its Part 42 normal times same_sense
        // points along the kernel's outward normal there.
        for (name, body) in solids() {
            let (_, ex) = export(&[(name, &body)]);
            let s = only_solid(&ex);
            let mesh = tessellate(&body, &TessOptions::default()).unwrap();
            let faces = body.faces();
            assert_eq!(s.face_surfaces.len(), faces.len());
            // Faces pair up by their (unique) names.
            for f in &faces {
                let fname = body.face_name(*f).unwrap().unwrap();
                let k = s.face_names.iter().position(|n| n == fname).unwrap();
                let written = &s.face_surfaces[k];
                let (surf, same) = body.face_surface(*f).unwrap().unwrap();
                assert_eq!(written.entity, entity_for(&surf), "{name}");
                let sample = (0..mesh.mesh.triangles.len())
                    .filter(|&i| mesh.face_of_triangle[i] == *f)
                    .flat_map(|i| mesh.mesh.triangle(i).unwrap())
                    .find(|p| {
                        let l = local(written, arr(*p - Point3::ORIGIN));
                        l[0].hypot(l[1]) > 1e-3
                    })
                    .expect("a sample point off the axis");
                let q = surf.parameters(sample);
                let n = surf.normal_at(q.u, q.v);
                let outward = arr(if same { n } else { -n });
                let w = written_outward(written, arr(sample - Point3::ORIGIN));
                assert!(dot(w, outward) > 0.0, "{name}: face {f}");
            }
        }
    }

    fn surface_of(ex: &Exchange, entity: &str) -> SurfaceSummary {
        let s = only_solid(ex);
        let found: Vec<_> = s
            .face_surfaces
            .into_iter()
            .filter(|f| f.entity == entity)
            .collect();
        let [one] = found.as_slice() else {
            panic!("one {entity} expected, found {}", found.len());
        };
        one.clone()
    }

    #[test]
    fn an_extruded_circle_writes_its_cylinder() {
        // Circle r = 5 about the origin extruded 10 along +z: the cylinder about the z
        // axis, radius exactly 5; two planes, two circles and the seam line.
        let body = extrude(&Frame::XY, &Profile::circle(Point2::ORIGIN, 5.0), 10.0).unwrap();
        let (_, ex) = export(&[("cylinder", &body)]);
        assert_eq!(ex.count("CYLINDRICAL_SURFACE"), 1);
        assert_eq!(ex.count("PLANE"), 2);
        assert_eq!(ex.count("CIRCLE"), 2);
        assert_eq!(ex.count("LINE"), 1);
        let c = surface_of(&ex, "CYLINDRICAL_SURFACE");
        assert_eq!(c.values, vec![5.0]);
        assert_eq!(cross(c.axis, [0.0, 0.0, 1.0]), [0.0, 0.0, 0.0]);
        assert_eq!((c.origin[0], c.origin[1]), (0.0, 0.0));
        // The outward normal points away from the axis, as Part 42's does.
        assert!(c.same_sense);
    }

    #[test]
    fn a_revolved_half_disc_writes_a_sphere_and_a_triangle_a_cone() {
        let sphere = revolve(&Frame::XY, &half_disc(3.0), &y_axis(), TAU).unwrap();
        let (_, ex) = export(&[("sphere", &sphere)]);
        assert_eq!(ex.count("ADVANCED_FACE"), 1);
        let s = surface_of(&ex, "SPHERICAL_SURFACE");
        assert_eq!(s.values, vec![3.0]);
        assert!(s.origin.iter().all(|x| x.abs() < 1e-12));
        assert!(s.same_sense);
        // The cone of the right triangle (0, 0), (3, 0), (0, 4) about y: apex (0, 4, 0),
        // base radius 3 at y = 0: semi-angle atan(3/4), and both the apex and the base
        // circle's vertex (3, 0, 0) satisfy ρ = r + h tan α in the written placement.
        let body = revolve(&Frame::XY, &cone_profile(), &y_axis(), TAU).unwrap();
        let (_, ex) = export(&[("cone", &body)]);
        let c = surface_of(&ex, "CONICAL_SURFACE");
        let [r, a] = c.values.as_slice() else {
            panic!()
        };
        assert!((a - (3.0f64 / 4.0).atan()).abs() < 1e-12, "{a}");
        assert!(*a > 0.0 && *a < FRAC_PI_2);
        for p in [[0.0, 4.0, 0.0], [3.0, 0.0, 0.0]] {
            let l = local(&c, p);
            let off = l[0].hypot(l[1]) - (r + l[2] * a.tan());
            assert!(off.abs() < 1e-12, "{p:?}: {off}");
        }
        assert!(c.same_sense);
    }

    #[test]
    fn tori_write_their_radii_and_spindle_parts_are_degenerate() {
        let cases = solids();
        let get = |n: &str| &cases.iter().find(|(name, _)| *name == n).unwrap().1;
        let (_, ex) = export(&[("torus", get("torus"))]);
        let t = surface_of(&ex, "TOROIDAL_SURFACE");
        assert_eq!(t.values, vec![5.0, 2.0]);
        assert!(t.same_sense);
        // The apple: tube centre 1 from the axis, radius 2: the outer part.
        let (_, ex) = export(&[("apple", get("apple"))]);
        let t = surface_of(&ex, "DEGENERATE_TOROIDAL_SURFACE");
        assert_eq!(
            (t.values.as_slice(), t.select_outer),
            (&[1.0, 2.0][..], Some(true))
        );
        // The lemon: tube centre 1 beyond the axis, radius √5: the inner part, whose
        // Part 42 normal points into the tube, against ours (same_sense flipped).
        let (_, ex) = export(&[("lemon", get("lemon"))]);
        let t = surface_of(&ex, "DEGENERATE_TOROIDAL_SURFACE");
        assert!((t.values[0] - 1.0).abs() < 1e-12 && (t.values[1] - 5f64.sqrt()).abs() < 1e-12);
        assert_eq!(t.select_outer, Some(false));
        let lemon = get("lemon");
        let f = lemon
            .faces()
            .into_iter()
            .find(|f| {
                matches!(
                    lemon.face_surface(*f).unwrap(),
                    Some((Surface::Torus(_), _))
                )
            })
            .unwrap();
        let (_, ours) = lemon.face_surface(f).unwrap().unwrap();
        assert_eq!(t.same_sense, !ours);
    }

    #[test]
    fn bad_surfaces_in_a_file_are_refused() {
        let body = extrude(&Frame::XY, &Profile::circle(Point2::ORIGIN, 5.0), 10.0).unwrap();
        let (text, _) = export(&[("cylinder", &body)]);
        let line = text
            .lines()
            .find(|l| l.contains("CYLINDRICAL_SURFACE("))
            .unwrap()
            .to_owned();
        for bad in [
            line.replace(",5.);", ",-5.);"),
            line.replace(",5.);", ",0.);"),
            line.replace("CYLINDRICAL_SURFACE", "WOBBLY_SURFACE"),
        ] {
            let ex = read::parse(&text.replace(&line, &bad)).unwrap();
            assert!(summarize(&ex).is_err(), "{bad}");
        }
    }
}
