//! Tests for K7 booleans. Expected volumes, areas and counts are closed forms derived in
//! the comments; every result is checked with `Body::validate`.

use super::*;
use crate::{Line2, Profile, ProfileEdge, Surface, extrude, polygon_edges, revolve};
use chamfera_geom::{Frame, Point2, Point3, Vec2};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * b.abs().max(1.0)
}

fn assert_close(what: &str, got: f64, want: f64) {
    assert!(close(got, want, 1e-9), "{what}: got {got}, want {want}");
}

/// Prefixes every face name with `tag:` (as the engine prefixes feature ids).
fn tagged(mut b: Body, tag: &str) -> Body {
    for f in b.faces() {
        let n = b.face_name(f).unwrap().unwrap_or("face").to_owned();
        b.set_face_name(f, &format!("{tag}:{n}")).unwrap();
    }
    b
}

/// The box `[x0, x0 + dx] × [y0, y0 + dy] × [z0, z0 + dz]`, faces `tag:bottom`, `tag:top`,
/// `tag:side 0` (y = y0), `tag:side 1` (x = x0 + dx), `tag:side 2` (y = y0 + dy),
/// `tag:side 3` (x = x0).
fn cuboid(tag: &str, min: [f64; 3], size: [f64; 3]) -> Body {
    let frame = Frame::XY
        .with_origin(Point3::new(0.0, 0.0, min[2]))
        .unwrap();
    let (x0, y0) = (min[0], min[1]);
    let (x1, y1) = (x0 + size[0], y0 + size[1]);
    let p = Profile::new(
        polygon_edges(&[
            Point2::new(x0, y0),
            Point2::new(x1, y0),
            Point2::new(x1, y1),
            Point2::new(x0, y1),
        ]),
        vec![],
    );
    tagged(extrude(&frame, &p, size[2]).unwrap(), tag)
}

/// A cylinder about the vertical line through `(cx, cy)` from `z0` to `z0 + h`.
fn cylinder(tag: &str, cx: f64, cy: f64, z0: f64, r: f64, h: f64) -> Body {
    let frame = Frame::XY.with_origin(Point3::new(0.0, 0.0, z0)).unwrap();
    let p = Profile::circle(Point2::new(cx, cy), r);
    tagged(extrude(&frame, &p, h).unwrap(), tag)
}

/// A sphere of radius `r` about the origin, its axis along z.
fn sphere(tag: &str, r: f64) -> Body {
    // The sketch plane XZ (x along X, y along Z); revolving about its y axis turns about Z.
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
    let axis = Line2::new(Point2::ORIGIN, Vec2::Y).unwrap();
    tagged(revolve(&Frame::XZ, &p, &axis, TAU).unwrap(), tag)
}

/// (V, E, F, shells, genus) after a full validity check.
fn counts(b: &Body) -> (usize, usize, usize, usize, usize) {
    let r = b.validate().unwrap();
    let s = r.summary;
    (s.vertices, s.edges, s.faces, s.shells, r.genus)
}

fn names(b: &Body) -> BTreeSet<String> {
    b.faces()
        .into_iter()
        .map(|f| b.face_name(f).unwrap().unwrap_or("").to_owned())
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

/// The faces of `b` named `name`.
fn named(b: &Body, name: &str) -> Vec<crate::FaceId> {
    b.faces()
        .into_iter()
        .filter(|f| b.face_name(*f).unwrap() == Some(name))
        .collect()
}

// ------------------------------------------------------------------ boxes

#[test]
fn two_overlapping_boxes_unite_into_one_solid() {
    let a = cuboid("A", [0.0; 3], [10.0; 3]);
    let b = cuboid("B", [5.0; 3], [10.0; 3]);
    let u = union(&a, &b).unwrap();
    // Volume: 1000 + 1000 - 5³ (the overlap cube) = 1875.
    assert_close("volume", u.volume().unwrap(), 1875.0);
    // Area: 600 + 600 minus the three 5×5 squares of each cube inside the other = 1050.
    assert_close("area", u.area().unwrap(), 1050.0);
    // Faces: 3 untouched + 3 notched per cube = 12. Vertices: 7 corners per cube outside
    // the other, plus 6 where an edge of one cube pierces a face of the other = 20.
    // Euler: E = V + F - 2 = 30.
    assert_eq!(counts(&u), (20, 30, 12, 1, 0));
}

#[test]
fn a_box_minus_a_box_through_one_edge_leaves_a_notch() {
    let a = cuboid("A", [0.0; 3], [10.0; 3]);
    let b = cuboid("B", [5.0, -1.0, 5.0], [10.0, 12.0, 10.0]);
    let d = subtract(&a, &b).unwrap();
    // 1000 - 5·10·5 = 750.
    assert_close("volume", d.volume().unwrap(), 750.0);
    // An L-shaped prism: two L ends of 75, bottom 100, back x = 0 100, top 50, front
    // x = 10 50, and the notch's floor and wall 50 each: 550.
    assert_close("area", d.area().unwrap(), 550.0);
    // An L hexagon prism: V = 12, E = 18, F = 8.
    assert_eq!(counts(&d), (12, 18, 8, 1, 0));
}

#[test]
fn two_overlapping_boxes_intersect_in_their_common_box() {
    let a = cuboid("A", [0.0; 3], [10.0; 3]);
    let b = cuboid("B", [5.0; 3], [10.0; 3]);
    let i = intersect(&a, &b).unwrap();
    assert_close("volume", i.volume().unwrap(), 125.0);
    assert_close("area", i.area().unwrap(), 150.0);
    assert_eq!(counts(&i), (8, 12, 6, 1, 0));
    // Three faces from each cube: x, y, z = 10 from A, x, y, z = 5 from B.
    assert_eq!(
        names(&i),
        set(&[
            "A:top", "A:side 1", "A:side 2", "B:bottom", "B:side 0", "B:side 3"
        ])
    );
}

// ------------------------------------------------------------------ plate and boss

/// The 50 × 30 × 10 plate.
fn plate() -> Body {
    cuboid("A", [0.0; 3], [50.0, 30.0, 10.0])
}

#[test]
fn a_boss_standing_on_a_plate_joins_into_one_shell_without_an_internal_face() {
    let h = 5.0;
    let boss = cuboid("B", [20.0, 10.0, 10.0], [10.0, 10.0, h]);
    let u = union(&plate(), &boss).unwrap();
    // Volume: 50·30·10 + 10·10·h = 15000 + 100 h.
    assert_close("volume", u.volume().unwrap(), 15_000.0 + 100.0 * h);
    // Area: the plate's 4600, the boss's top replaces its footprint, plus four walls 10 h.
    assert_close("area", u.area().unwrap(), 4600.0 + 40.0 * h);
    // Faces: plate 6 (its top with a ring) + boss top + 4 boss sides = 11, no boss bottom.
    // V = 8 + 8 = 16, E = 12 + 12 = 24; 16 - 24 + 11 - 1 ring = 2.
    assert_eq!(counts(&u), (16, 24, 11, 1, 0));
    assert!(named(&u, "B:bottom").is_empty());
    // The plate's top is still one face called `A:top`, with the boss's footprint as a
    // ring: area 1500 - 100.
    let top = named(&u, "A:top");
    assert_eq!(top.len(), 1);
    assert_eq!(u.face_inner_loops(top[0]).unwrap().len(), 1);
    assert_close("top area", u.face_area(top[0]).unwrap(), 1400.0);
    assert_eq!(
        names(&u),
        set(&[
            "A:bottom", "A:top", "A:side 0", "A:side 1", "A:side 2", "A:side 3", "B:top",
            "B:side 0", "B:side 1", "B:side 2", "B:side 3",
        ])
    );
}

#[test]
fn subtracting_a_boss_that_only_touches_the_plate_leaves_the_plate() {
    let boss = cuboid("B", [20.0, 10.0, 10.0], [10.0, 10.0, 5.0]);
    let d = subtract(&plate(), &boss).unwrap();
    assert_close("volume", d.volume().unwrap(), 15_000.0);
    // The split top merges back: the plate's own 8, 12, 6.
    assert_eq!(counts(&d), (8, 12, 6, 1, 0));
    assert_eq!(names(&d), names(&plate()));
    assert!(matches!(
        intersect(&plate(), &boss),
        Err(KernelError::EmptyResult)
    ));
}

#[test]
fn a_through_hole_makes_genus_one() {
    let r = 5.0;
    let tool = cylinder("B", 25.0, 15.0, -1.0, r, 12.0);
    let d = subtract(&plate(), &tool).unwrap();
    // Volume: 15000 - π r² · 10 = 15000 - 250π.
    assert_close("volume", d.volume().unwrap(), 15_000.0 - 250.0 * PI);
    // Area: 4600 - 2 π r² (top and bottom) + 2 π r · 10 (the bore) = 4600 + 50π.
    assert_close("area", d.area().unwrap(), 4600.0 + 50.0 * PI);
    // V: 8 + 2 (the bore's circles meet its seam); E: 12 + 2 circles + 1 seam = 15;
    // F: 6 + bore = 7; rings 2: 10 - 15 + 7 - 2 = 0 = 2 (1 - G), G = 1.
    assert_eq!(counts(&d), (10, 15, 7, 1, 1));
    let bore = named(&d, "B:side 0");
    assert_eq!(bore.len(), 1);
    assert!(matches!(
        d.face_surface(bore[0]).unwrap(),
        Some((Surface::Cylinder(_), false))
    ));
}

#[test]
fn a_through_hole_flush_with_both_faces_is_the_same_hole() {
    // The tool's caps lie exactly in the plate's top and bottom.
    let tool = cylinder("B", 25.0, 15.0, 0.0, 5.0, 10.0);
    let d = subtract(&plate(), &tool).unwrap();
    assert_close("volume", d.volume().unwrap(), 15_000.0 - 250.0 * PI);
    assert_close("area", d.area().unwrap(), 4600.0 + 50.0 * PI);
    assert_eq!(counts(&d), (10, 15, 7, 1, 1));
}

#[test]
fn a_blind_hole_has_a_floor() {
    // Depth 4 from the top: the tool runs from z = 6 to above the plate.
    let tool = cylinder("B", 25.0, 15.0, 6.0, 5.0, 10.0);
    let d = subtract(&plate(), &tool).unwrap();
    // Volume: 15000 - 25π · 4 = 15000 - 100π.
    assert_close("volume", d.volume().unwrap(), 15_000.0 - 100.0 * PI);
    // Area: 4600 - 25π (opening) + 2π·5·4 (wall) + 25π (floor) = 4600 + 40π.
    assert_close("area", d.area().unwrap(), 4600.0 + 40.0 * PI);
    // F: 6 + wall + floor = 8, one ring; V: 8 + 2, E: 12 + 2 circles + 1 seam = 15:
    // 10 - 15 + 8 - 1 = 2, genus 0.
    assert_eq!(counts(&d), (10, 15, 8, 1, 0));
    let floor = named(&d, "B:bottom");
    assert_eq!(floor.len(), 1);
    assert_close("floor", d.face_area(floor[0]).unwrap(), 25.0 * PI);
}

#[test]
fn a_countersink_near_the_plate_sides_cuts_without_meeting_them() {
    // A countersunk through hole at (25, 15) in the 50 x 30 x 10 plate: a cone from
    // r = 6 at the top down to r = 3 at depth 3 (90°), then r = 3 to below the plate. The
    // cone's surface would cut the side planes y = 0 and y = 30 in hyperbolas, but its
    // face (r <= 6) ends 9 mm from them, so those pairs must not be intersected.
    let (x, y) = (chamfera_geom::Vec3::X, chamfera_geom::Vec3::Y);
    let frame = Frame::from_normal_and_x(Point3::new(25.0, 15.0, 10.0), y, x).unwrap();
    let section = [(0.0, 0.0), (6.0, 0.0), (3.0, 3.0), (3.0, 11.0), (0.0, 11.0)];
    let p = Profile::new(
        polygon_edges(&section.map(|(x, y)| Point2::new(x, y))),
        vec![],
    );
    let axis = Line2::new(Point2::ORIGIN, Vec2::Y).unwrap();
    let tool = tagged(revolve(&frame, &p, &axis, TAU).unwrap(), "B");
    let d = subtract(&plate(), &tool).unwrap();
    // Frustum π h (R² + R r + r²) / 3 = π (36 + 18 + 9) = 63π, and 9π · 7 below it.
    assert_close(
        "volume",
        d.volume().unwrap(),
        15_000.0 - 63.0 * PI - 63.0 * PI,
    );
    let cone = named(&d, "B:side 1");
    assert_eq!(cone.len(), 1);
    assert!(matches!(
        d.face_surface(cone[0]).unwrap(),
        Some((Surface::Cone(_), _))
    ));
}

#[test]
fn a_pocket_flush_with_the_top_and_a_cut_flush_with_the_bottom() {
    // A 10 × 10 pocket, 5 deep, whose tool's top is the plate's top.
    let pocket = cuboid("B", [20.0, 10.0, 5.0], [10.0, 10.0, 5.0]);
    let d = subtract(&plate(), &pocket).unwrap();
    assert_close("volume", d.volume().unwrap(), 15_000.0 - 500.0);
    // Area: 4600 - 100 (opening) + 4·10·5 (walls) + 100 (floor) = 4800.
    assert_close("area", d.area().unwrap(), 4800.0);
    assert_eq!(counts(&d), (16, 24, 11, 1, 0));
    // A through cut whose tool's bottom is the plate's bottom and top above it.
    let cut = cuboid("B", [20.0, 10.0, 0.0], [10.0, 10.0, 12.0]);
    let d = subtract(&plate(), &cut).unwrap();
    assert_close("volume", d.volume().unwrap(), 15_000.0 - 1000.0);
    // Area: 4600 - 2·100 + 4·10·10 = 4800; a square through hole: genus 1.
    assert_close("area", d.area().unwrap(), 4800.0);
    assert_eq!(counts(&d), (16, 24, 10, 1, 1));
}

// ------------------------------------------------------------------ curved solids

#[test]
fn a_sphere_and_a_slab_unite_and_intersect_along_the_equator() {
    let r = 5.0;
    let s = sphere("S", r);
    // The slab [-10, 10]² × [0, 10] holds the upper hemisphere.
    let slab = cuboid("B", [-10.0, -10.0, 0.0], [20.0, 20.0, 10.0]);
    let u = union(&s, &slab).unwrap();
    // Volume: the slab's 4000 plus the lower hemisphere 2π r³/3 = 250π/3.
    assert_close("volume", u.volume().unwrap(), 4000.0 + 250.0 * PI / 3.0);
    // Area: the slab's 1600 - π r² (the equator disc) + 2π r² (the lower hemisphere).
    assert_close("area", u.area().unwrap(), 1600.0 + 25.0 * PI);
    counts(&u);
    let i = intersect(&s, &slab).unwrap();
    // The upper hemisphere: 250π/3, area 2π r² + π r² = 75π.
    assert_close("volume", i.volume().unwrap(), 250.0 * PI / 3.0);
    assert_close("area", i.area().unwrap(), 75.0 * PI);
    // The hemisphere: its pole and the seam's end on the equator; seam and equator; dome
    // and disc: 2 - 2 + 2 = 2.
    assert_eq!(counts(&i), (2, 2, 2, 1, 0));
    assert_eq!(names(&i), set(&["S:side 0", "B:bottom"]));
}

#[test]
fn a_cylinder_intersected_with_a_box_is_half_a_cylinder() {
    let c = cylinder("C", 0.0, 0.0, 0.0, 5.0, 10.0);
    let b = cuboid("B", [0.0, -10.0, 2.0], [10.0, 20.0, 6.0]);
    let i = intersect(&c, &b).unwrap();
    // Half of π r² · 6: 75π.
    assert_close("volume", i.volume().unwrap(), 75.0 * PI);
    // Two half discs (25π), half the mantle (π r · 6 = 30π), the 10 × 6 cut face.
    assert_close("area", i.area().unwrap(), 55.0 * PI + 60.0);
    counts(&i);
    let d = subtract(&c, &b).unwrap();
    // The cylinder's 250π less the half: 175π.
    assert_close("volume", d.volume().unwrap(), 175.0 * PI);
    counts(&d);
}

#[test]
fn a_tube_is_a_cylinder_minus_a_coaxial_cylinder() {
    let outer = cylinder("A", 0.0, 0.0, 0.0, 10.0, 10.0);
    let inner = cylinder("B", 0.0, 0.0, -1.0, 4.0, 12.0);
    let t = subtract(&outer, &inner).unwrap();
    // π (10² - 4²) · 10 = 840π.
    assert_close("volume", t.volume().unwrap(), 840.0 * PI);
    // Rings 2π(100 - 16) = 168π, mantles 2π·10·10 + 2π·4·10 = 280π: 448π.
    assert_close("area", t.area().unwrap(), 448.0 * PI);
    assert_eq!(counts(&t).4, 1);
    // Equal coaxial cylinders of different heights: coincident mantles.
    let low = cylinder("A", 0.0, 0.0, 0.0, 5.0, 10.0);
    let high = cylinder("B", 0.0, 0.0, 5.0, 5.0, 10.0);
    let u = union(&low, &high).unwrap();
    // π 25 · 15 = 375π; area 2 · 25π + 2π·5·15 = 200π.
    assert_close("volume", u.volume().unwrap(), 375.0 * PI);
    assert_close("area", u.area().unwrap(), 200.0 * PI);
    counts(&u);
    let i = intersect(&low, &high).unwrap();
    assert_close("volume", i.volume().unwrap(), 125.0 * PI);
    counts(&i);
}

// ------------------------------------------------------------------ special results

#[test]
fn disjoint_solids_unite_into_two_lumps_and_have_no_intersection() {
    let a = cuboid("A", [0.0; 3], [10.0; 3]);
    let b = cuboid("B", [20.0, 0.0, 0.0], [5.0; 3]);
    let u = union(&a, &b).unwrap();
    assert_close("volume", u.volume().unwrap(), 1125.0);
    assert_close("area", u.area().unwrap(), 750.0);
    // Two cubes: 16 - 24 + 12 = 4 = 2 · 2 shells.
    assert_eq!(counts(&u), (16, 24, 12, 2, 0));
    assert!(matches!(intersect(&a, &b), Err(KernelError::EmptyResult)));
    let d = subtract(&a, &b).unwrap();
    assert_close("volume", d.volume().unwrap(), 1000.0);
}

#[test]
fn a_solid_with_itself() {
    let a = cuboid("A", [0.0; 3], [10.0; 3]);
    assert!(matches!(subtract(&a, &a), Err(KernelError::EmptyResult)));
    for r in [union(&a, &a).unwrap(), intersect(&a, &a).unwrap()] {
        assert_close("volume", r.volume().unwrap(), 1000.0);
        assert_eq!(counts(&r), (8, 12, 6, 1, 0));
        assert_eq!(names(&r), names(&a));
    }
    let c = cylinder("C", 0.0, 0.0, 0.0, 5.0, 10.0);
    let u = union(&c, &c).unwrap();
    assert_close("volume", u.volume().unwrap(), 250.0 * PI);
    assert_eq!(counts(&u), counts(&c));
}

#[test]
fn a_solid_strictly_inside_another_is_subtracted_as_a_void() {
    let a = cuboid("A", [0.0; 3], [10.0; 3]);
    let b = cuboid("B", [4.0; 3], [2.0; 3]);
    let d = subtract(&a, &b).unwrap();
    // 1000 - 8, area 600 + 24 (the void's walls count).
    assert_close("volume", d.volume().unwrap(), 992.0);
    assert_close("area", d.area().unwrap(), 624.0);
    assert_eq!(counts(&d), (16, 24, 12, 2, 0));
    let u = union(&a, &b).unwrap();
    assert_eq!(counts(&u), (8, 12, 6, 1, 0));
    let i = intersect(&a, &b).unwrap();
    assert_close("volume", i.volume().unwrap(), 8.0);
}

#[test]
fn boxes_touching_along_an_edge_or_at_a_corner_stay_separate_lumps() {
    let a = cuboid("A", [0.0; 3], [1.0; 3]);
    let edge = cuboid("B", [1.0, 1.0, 0.0], [1.0; 3]);
    let u = union(&a, &edge).unwrap();
    assert_close("volume", u.volume().unwrap(), 2.0);
    assert_eq!(counts(&u), (16, 24, 12, 2, 0));
    assert!(matches!(
        intersect(&a, &edge),
        Err(KernelError::EmptyResult)
    ));
    let corner = cuboid("B", [1.0; 3], [1.0; 3]);
    let u = union(&a, &corner).unwrap();
    assert_eq!(counts(&u), (16, 24, 12, 2, 0));
    // Side by side along a whole face: one box.
    let side = cuboid("B", [1.0, 0.0, 0.0], [1.0; 3]);
    let u = union(&a, &side).unwrap();
    assert_close("volume", u.volume().unwrap(), 2.0);
    assert_close("area", u.area().unwrap(), 10.0);
    assert_eq!(counts(&u), (8, 12, 6, 1, 0));
}

#[test]
fn a_prism_standing_on_its_edge_touches_the_plate_along_a_line() {
    // A triangular prism along y whose lower edge (x = 25, z = 10) lies on the top.
    // Frame::XZ: x along X, y along Z, normal -Y; extruding 20 runs from y = 25 to 5.
    let frame = Frame::XZ.with_origin(Point3::new(0.0, 25.0, 0.0)).unwrap();
    let tri = Profile::new(
        polygon_edges(&[
            Point2::new(25.0, 10.0),
            Point2::new(30.0, 15.0),
            Point2::new(20.0, 15.0),
        ]),
        vec![],
    );
    let prism = tagged(extrude(&frame, &tri, 20.0).unwrap(), "B");
    // Triangle area 10·5/2 = 25, times 20.
    assert_close("prism", prism.volume().unwrap(), 500.0);
    let u = union(&plate(), &prism).unwrap();
    assert_close("volume", u.volume().unwrap(), 15_500.0);
    assert_eq!(counts(&u).3, 2);
    let d = subtract(&plate(), &prism).unwrap();
    assert_eq!(counts(&d), (8, 12, 6, 1, 0));
    assert!(matches!(
        intersect(&plate(), &prism),
        Err(KernelError::EmptyResult)
    ));
}

#[test]
fn crossing_cylinders_of_different_radii_are_refused_cleanly() {
    let a = cylinder("A", 0.0, 0.0, -10.0, 5.0, 20.0);
    // A cylinder along x through the first one: a quartic intersection.
    let frame = Frame::YZ.with_origin(Point3::new(-10.0, 0.0, 0.0)).unwrap();
    let b = tagged(
        extrude(&frame, &Profile::circle(Point2::ORIGIN, 3.0), 20.0).unwrap(),
        "B",
    );
    for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
        match boolean(&a, &b, op) {
            Err(KernelError::Unsupported(msg)) => {
                assert!(
                    msg.contains("A:side 0") && msg.contains("B:side 0"),
                    "{msg}"
                );
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

#[test]
fn a_sphere_and_a_box_unite_where_the_cuts_run_through_the_poles() {
    // The box [0, 10]³ holds one octant of the sphere about the origin; two of its faces
    // cut the sphere along meridians through both poles, the third along the equator.
    let r = 5.0;
    let s = sphere("S", r);
    let b = cuboid("B", [0.0; 3], [10.0; 3]);
    let u = union(&s, &b).unwrap();
    // Volume: 1000 + 7/8 · 4π r³/3 = 1000 + 875π/6.
    assert_close("volume", u.volume().unwrap(), 1000.0 + 875.0 * PI / 6.0);
    // Area: 600 - 3 quarter discs (3 π r²/4) + 7/8 · 4π r² = 600 + 68.75π.
    assert_close("area", u.area().unwrap(), 600.0 + 68.75 * PI);
    counts(&u);
    let i = intersect(&s, &b).unwrap();
    // One octant: π r³/6, area π r²/2 + 3 π r²/4.
    assert_close("volume", i.volume().unwrap(), 125.0 * PI / 6.0);
    assert_close("area", i.area().unwrap(), 31.25 * PI);
    counts(&i);
}

// ------------------------------------------------------------------ names and nesting

#[test]
fn faces_keep_their_names_through_nested_booleans() {
    // Join a boss onto the plate, then drill through the boss and the plate.
    let boss = cuboid("boss", [20.0, 10.0, 10.0], [10.0, 10.0, 5.0]);
    let part = union(&plate(), &boss).unwrap();
    let drill = cylinder("hole", 25.0, 15.0, -1.0, 2.0, 20.0);
    let part = subtract(&part, &drill).unwrap();
    // 15000 + 500 - π 4 · 15 = 15500 - 60π.
    assert_close("volume", part.volume().unwrap(), 15_500.0 - 60.0 * PI);
    // The plate's top is still `A:top`, now with the boss's footprint as its ring; the
    // drill's wall is `hole:side 0`, one face through boss and plate.
    let top = named(&part, "A:top");
    assert_eq!(top.len(), 1);
    assert_close("top", part.face_area(top[0]).unwrap(), 1400.0);
    let boss_top = named(&part, "boss:top");
    assert_eq!(boss_top.len(), 1);
    // 100 - 4π.
    assert_close(
        "boss top",
        part.face_area(boss_top[0]).unwrap(),
        100.0 - 4.0 * PI,
    );
    let wall = named(&part, "hole:side 0");
    assert_eq!(wall.len(), 1);
    // 2π · 2 · 15 = 60π.
    assert_close("wall", part.face_area(wall[0]).unwrap(), 60.0 * PI);
    assert_eq!(counts(&part).4, 1);
    // Every face name comes from an operand face.
    let mut want: BTreeSet<String> = names(&plate());
    want.extend(names(&boss).into_iter().filter(|n| n != "boss:bottom"));
    want.insert("hole:side 0".into());
    assert_eq!(names(&part), want);
    // An intersection edge is named after the two faces that made it.
    assert!(
        part.edges()
            .into_iter()
            .any(|e| { part.edge_name(e).unwrap() == Some("boss:top x hole:side 0") })
    );
}

#[test]
fn a_slot_through_a_face_names_both_pieces_after_it() {
    // A slot across the whole plate splits its top into two faces, both `A:top`.
    let slot = cuboid("B", [20.0, -1.0, 5.0], [10.0, 32.0, 10.0]);
    let d = subtract(&plate(), &slot).unwrap();
    assert_close("volume", d.volume().unwrap(), 15_000.0 - 10.0 * 30.0 * 5.0);
    let tops = named(&d, "A:top");
    assert_eq!(tops.len(), 2);
    for t in tops {
        assert_close("top piece", d.face_area(t).unwrap(), 20.0 * 30.0);
    }
    assert_eq!(counts(&d).2, 10);
}

#[test]
fn invalid_operands_are_refused() {
    let a = cuboid("A", [0.0; 3], [1.0; 3]);
    let empty = Body::new();
    assert!(matches!(
        union(&a, &empty),
        Err(KernelError::InvalidInput(_))
    ));
    assert!(matches!(
        subtract(&empty, &a),
        Err(KernelError::InvalidInput(_))
    ));
}

#[test]
fn planar_results_tessellate() {
    // A pocket: faces with rings and split edges (tessellation covers planes so far).
    let pocket = cuboid("B", [20.0, 10.0, 5.0], [10.0, 10.0, 8.0]);
    let d = subtract(&plate(), &pocket).unwrap();
    let mesh = crate::tessellate(&d, &crate::TessOptions::default()).unwrap();
    assert!(!mesh.mesh.triangles.is_empty());
}

// ------------------------------------------------------------------ property test

/// A small deterministic generator (xorshift64*), so the test needs no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn int(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// How the property test draws its solids.
#[derive(Clone, Copy)]
struct Draw {
    /// Cylinders as well as boxes.
    cylinders: bool,
    /// Cylinders along x, y or z (otherwise always along z).
    any_axis: bool,
    /// Integer coordinates and sizes (so faces often coincide, edges touch and corners
    /// meet) instead of continuous ones.
    grid: bool,
}

/// A cylinder of radius `r` and length `h` along axis `k` (0 = x, 1 = y, 2 = z) from the
/// base centre `c`.
fn cylinder_along(tag: &str, k: usize, c: [f64; 3], r: f64, h: f64) -> Body {
    let (frame, centre) = match k {
        // Frame::YZ: plane x along world Y, y along Z, normal +X.
        0 => (
            Frame::YZ.with_origin(Point3::new(c[0], 0.0, 0.0)).unwrap(),
            Point2::new(c[1], c[2]),
        ),
        // Frame::XZ: plane x along X, y along Z, normal -Y: start at the far end.
        1 => (
            Frame::XZ
                .with_origin(Point3::new(0.0, c[1] + h, 0.0))
                .unwrap(),
            Point2::new(c[0], c[2]),
        ),
        _ => return cylinder(tag, c[0], c[1], c[2], r, h),
    };
    tagged(
        extrude(&frame, &Profile::circle(centre, r), h).unwrap(),
        tag,
    )
}

/// A random solid with its exact volume: an axis-aligned box or an axis-parallel
/// cylinder.
fn random_solid(rng: &mut Rng, tag: &str, draw: Draw) -> (Body, f64) {
    let grid = draw.grid;
    let coord = |rng: &mut Rng| {
        if grid {
            rng.int(0, 4) as f64
        } else {
            4.0 * rng.unit()
        }
    };
    let size = |rng: &mut Rng| {
        if grid {
            rng.int(1, 3) as f64
        } else {
            0.5 + 2.5 * rng.unit()
        }
    };
    if draw.cylinders && rng.next().is_multiple_of(2) {
        let c = [coord(rng), coord(rng), coord(rng)];
        let r = if grid {
            [0.5, 1.0, 1.5][rng.int(0, 2) as usize]
        } else {
            0.3 + 1.5 * rng.unit()
        };
        let h = size(rng);
        let k = if draw.any_axis {
            rng.int(0, 2) as usize
        } else {
            2
        };
        (cylinder_along(tag, k, c, r, h), PI * r * r * h)
    } else {
        let min = [coord(rng), coord(rng), coord(rng)];
        let s = [size(rng), size(rng), size(rng)];
        (cuboid(tag, min, s), s[0] * s[1] * s[2])
    }
}

/// The volume of a boolean's result (0 when empty), after a full validity check.
fn volume_of(r: KernelResult<Body>) -> KernelResult<f64> {
    match r {
        Ok(b) => {
            b.validate()?;
            b.volume()
        }
        Err(KernelError::EmptyResult) => Ok(0.0),
        Err(e) => Err(e),
    }
}

/// Runs `n` random pairs; returns (operations attempted, operations refused). Every
/// successful triple must satisfy V(A ∪ B) + V(A ∩ B) = V(A) + V(B) and
/// V(A - B) = V(A) - V(A ∩ B) to 1e-9 of V(A) + V(B) (the measures are exact closed
/// forms for these faces, so only rounding separates the two sides). A failure must be
/// a clean refusal of an intersection K6 does not support yet.
fn property_run(seed: u64, n: usize, draw: Draw) -> (usize, usize) {
    let mut rng = Rng(seed);
    let mut failed = 0;
    for _ in 0..n {
        let (a, va) = random_solid(&mut rng, "A", draw);
        let (b, vb) = random_solid(&mut rng, "B", draw);
        let results = [
            volume_of(union(&a, &b)),
            volume_of(intersect(&a, &b)),
            volume_of(subtract(&a, &b)),
        ];
        for r in &results {
            if let Err(e) = r {
                assert!(
                    matches!(e, KernelError::Unsupported(_)),
                    "{e} for A = {:?}, B = {:?}",
                    a.bounding_box(),
                    b.bounding_box()
                );
                failed += 1;
            }
        }
        let tol = 1e-9 * (va + vb);
        if let [Ok(u), Ok(i), _] = &results {
            assert!(
                (u + i - va - vb).abs() <= tol,
                "union + intersection: {u} + {i} vs {va} + {vb}"
            );
        }
        if let [_, Ok(i), Ok(d)] = &results {
            assert!((d - (va - i)).abs() <= tol, "difference: {d} vs {va} - {i}");
        }
    }
    (3 * n, failed)
}

#[test]
fn random_boxes_on_a_grid_satisfy_the_volume_identities_without_any_failure() {
    let draw = Draw {
        cylinders: false,
        any_axis: false,
        grid: true,
    };
    let (ops, failed) = property_run(0x9e37_79b9_7f4a_7c15, 200, draw);
    println!("boxes on a grid: {failed} of {ops} operations refused");
    assert_eq!(failed, 0);
    let (ops, failed) = property_run(
        0x5851_f42d_4c95_7f2d,
        100,
        Draw {
            grid: false,
            ..draw
        },
    );
    println!("boxes, continuous: {failed} of {ops} operations refused");
    assert_eq!(failed, 0);
}

#[test]
fn random_boxes_and_vertical_cylinders_satisfy_the_volume_identities() {
    let draw = Draw {
        cylinders: true,
        any_axis: false,
        grid: false,
    };
    let (ops, failed) = property_run(0x2545_f491_4f6c_dd1d, 100, draw);
    println!("boxes and vertical cylinders, continuous: {failed} of {ops} operations refused");
    assert_eq!(failed, 0);
    let (ops, failed) = property_run(0x1234_5678_9abc_def1, 100, Draw { grid: true, ..draw });
    println!("boxes and vertical cylinders on a grid: {failed} of {ops} operations refused");
    assert_eq!(failed, 0);
}

#[test]
fn random_cylinders_along_any_axis_are_exact_or_cleanly_refused() {
    // Crossing cylinders along different axes meet in a quartic (unless equal radii and
    // meeting axes), which K6 refuses; everything else must succeed.
    let draw = Draw {
        cylinders: true,
        any_axis: true,
        grid: true,
    };
    let (ops, failed) = property_run(0x0dd1_7e55_c0ff_ee00, 100, draw);
    println!("boxes and cylinders along x, y, z on a grid: {failed} of {ops} operations refused");
    assert!(failed < ops);
}
