//! Tests for K12 shells. Expected volumes are closed forms derived in the comments, never
//! the code's own output.

use super::*;
use crate::{Line2, Profile, ProfileEdge, extrude, fillet, revolve};
use chamfera_geom::{Frame, Point2, Vec2};
use std::f64::consts::{PI, TAU};

fn assert_close(what: &str, got: f64, want: f64) {
    assert!(
        (got - want).abs() <= 1e-6 * want.abs().max(1.0),
        "{what}: got {got}, want {want}"
    );
}

fn rect(w: f64, h: f64) -> Vec<Point2> {
    vec![
        Point2::new(0.0, 0.0),
        Point2::new(w, 0.0),
        Point2::new(w, h),
        Point2::new(0.0, h),
    ]
}

/// 50 x 30 x 10 on the XY plane, faces `bottom` (z = 0), `top` (z = 10), `side k`.
fn block() -> Body {
    extrude(&Frame::XY, &Profile::polygon(&rect(50.0, 30.0)), 10.0).unwrap()
}

fn face(b: &Body, name: &str) -> FaceId {
    b.faces()
        .into_iter()
        .find(|f| b.face_name(*f).unwrap() == Some(name))
        .unwrap_or_else(|| panic!("no face named {name}"))
}

fn names(b: &Body) -> BTreeSet<String> {
    b.faces()
        .into_iter()
        .filter_map(|f| b.face_name(f).unwrap().map(str::to_owned))
        .collect()
}

#[test]
fn a_box_shelled_2_with_its_top_open_is_a_tray() {
    let b = block();
    let s = shell(&b, &[face(&b, "top")], 2.0).unwrap();
    // The cavity is 46 x 26 (2 off each side) and 8 deep (2 off the bottom, open top).
    assert_close("volume", s.volume().unwrap(), 15000.0 - 46.0 * 26.0 * 8.0);
    let r = s.validate().unwrap();
    assert_eq!(s.shells().len(), 1);
    // One shell, genus 0: a tray is a ball topologically.
    assert_eq!(r.euler_characteristic(), 2);
    // Outside: 5 faces kept with their names, the top now a rim; inside: 5 walls.
    let n = names(&s);
    assert!(n.contains("top") && n.contains("bottom") && n.contains("side 0"));
    assert!(n.contains("shell of bottom") && n.contains("shell of side 3"));
    assert!(!n.contains("shell of top"));
    assert_eq!(s.faces().len(), 11);
    // The rim: the top face keeps 50·30 - 46·26 of area.
    assert_close(
        "rim",
        s.face_area(face(&s, "top")).unwrap(),
        50.0 * 30.0 - 46.0 * 26.0,
    );
}

#[test]
fn a_box_shelled_with_every_face_closed_is_hollow() {
    let b = block();
    let s = shell(&b, &[], 2.0).unwrap();
    // A closed void 46 x 26 x 6 inside.
    assert_close("volume", s.volume().unwrap(), 15000.0 - 46.0 * 26.0 * 6.0);
    s.validate().unwrap();
    assert_eq!(s.shells().len(), 2);
    assert_eq!(s.faces().len(), 12);
}

#[test]
fn a_cylinder_shelled_1_with_its_top_open_is_a_cup() {
    let b = extrude(
        &Frame::XY,
        &Profile::circle(Point2::new(0.0, 0.0), 10.0),
        20.0,
    )
    .unwrap();
    let s = shell(&b, &[face(&b, "top")], 1.0).unwrap();
    // π r² h minus the cavity of radius 9, depth 19.
    assert_close(
        "volume",
        s.volume().unwrap(),
        PI * (100.0 * 20.0 - 81.0 * 19.0),
    );
    s.validate().unwrap();
    assert_eq!(s.shells().len(), 1);
}

#[test]
fn a_too_thick_shell_is_refused_and_changes_nothing() {
    let b = block();
    let before = b.clone();
    let top = face(&b, "top");
    // 20 mm walls cross in every direction (the block is 10 high, 30 wide).
    assert!(matches!(
        shell(&b, &[top], 20.0),
        Err(KernelError::InvalidInput(_))
    ));
    // 15 mm: the walls of the 30 mm width meet exactly (zero cavity width).
    assert!(shell(&b, &[top], 15.0).is_err());
    // 10 mm with the top open: the floor reaches the open top.
    assert!(shell(&b, &[top], 10.0).is_err());
    // 5 closed: the void would be 40 x 20 x 0.
    assert!(shell(&b, &[], 5.0).is_err());
    assert_eq!(b, before);
}

#[test]
fn hostile_thicknesses_and_faces_are_refused() {
    let b = block();
    let top = face(&b, "top");
    for t in [
        0.0,
        1e-9,
        -0.0,
        f64::NAN,
        f64::INFINITY,
        -f64::INFINITY,
        1e7,
    ] {
        assert!(
            matches!(shell(&b, &[top], t), Err(KernelError::InvalidInput(_))),
            "thickness {t}"
        );
    }
    // A face of another body that does not exist here.
    let other: FaceId = serde_json::from_str("999").unwrap();
    assert!(shell(&b, &[other], 1.0).is_err());
    // Every face open leaves nothing.
    assert!(matches!(
        shell(&b, &b.faces(), 1.0),
        Err(KernelError::InvalidInput(_))
    ));
    // The same face twice counts once.
    let s = shell(&b, &[top, top], 2.0).unwrap();
    assert_close("volume", s.volume().unwrap(), 15000.0 - 46.0 * 26.0 * 8.0);
}

#[test]
fn a_negative_thickness_shells_outward() {
    let b = block();
    let s = shell(&b, &[face(&b, "top")], -2.0).unwrap();
    // The outside grows 2 on each side and below (54 x 34 x 12, top flush at z = 10);
    // the block itself is the cavity.
    assert_close("volume", s.volume().unwrap(), 54.0 * 34.0 * 12.0 - 15000.0);
    s.validate().unwrap();
    let bb = s.bounding_box().unwrap();
    assert_close("min z", bb.min.z, -2.0);
    assert_close("max z", bb.max.z, 10.0);
}

#[test]
fn a_plate_with_a_hole_keeps_a_wall_around_the_hole() {
    // 50 x 30 x 10 with a through hole r = 5 at (25, 15), bottom open, walls of 1.
    let p = Profile::polygon(&rect(50.0, 30.0))
        .with_hole(vec![ProfileEdge::circle(Point2::new(25.0, 15.0), 5.0)]);
    let b = extrude(&Frame::XY, &p, 10.0).unwrap();
    let s = shell(&b, &[face(&b, "bottom")], 1.0).unwrap();
    // Solid: 15000 - 250π. Cavity: 48 x 28 x 9 less the tube around the hole, radius 6
    // (the hole's wall moves 1 into the material), 9 high: 12096 - 324π.
    let want = (15000.0 - 250.0 * PI) - (48.0 * 28.0 * 9.0 - 36.0 * 9.0 * PI);
    assert_close("volume", s.volume().unwrap(), want);
    s.validate().unwrap();
}

#[test]
fn a_cone_frustum_shells_along_its_slant() {
    // Revolve (0,0) (10,0) (5,10) (0,10) about the y axis: a frustum of radii 10 and 5,
    // height 10. `side 1` is the cone, `side 2` the top disc.
    let p = Profile::polygon(&[
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(5.0, 10.0),
        Point2::new(0.0, 10.0),
    ]);
    let axis = Line2::new(Point2::ORIGIN, Vec2::Y).unwrap();
    let b = revolve(&Frame::XY, &p, &axis, TAU).unwrap();
    let s = shell(&b, &[face(&b, "side 2")], 1.0).unwrap();
    // The slant ρ + z/2 = 10 moves inward by 1 along its unit normal (1, 1/2)/√1.25, so
    // the inner slant is ρ = a - z/2 with a = 10 - √1.25, from z = 1 to z = 10.
    // Frustum volume π h (R² + R r + r²) / 3.
    let frustum =
        |h: f64, big: f64, small: f64| PI * h * (big * big + big * small + small * small) / 3.0;
    let a = 10.0 - 1.25f64.sqrt();
    let want = frustum(10.0, 10.0, 5.0) - frustum(9.0, a - 0.5, a - 5.0);
    assert_close("volume", s.volume().unwrap(), want);
    s.validate().unwrap();
}

#[test]
fn a_block_with_filleted_vertical_edges_shells_with_round_inner_corners() {
    // Fillet the four vertical edges of the block with r = 5, then shell 2 with the top
    // open. The outline is a rounded rectangle of area 50·30 - (4 - π) 5²; the cavity
    // outline offsets it by 2: 46 x 26 with corner radius 3, area 46·26 - (4 - π) 3².
    let b = block();
    let vertical: Vec<_> = b
        .edges()
        .into_iter()
        .filter(|e| {
            let [v0, v1] = b.edge_vertices(*e).unwrap();
            let (p0, p1) = (b.vertex_point(v0).unwrap(), b.vertex_point(v1).unwrap());
            (p0.z - p1.z).abs() > 1.0
        })
        .collect();
    assert_eq!(vertical.len(), 4);
    let f = fillet(&b, &vertical, 5.0).unwrap();
    let s = shell(&f, &[face(&f, "top")], 2.0).unwrap();
    let outer = 50.0 * 30.0 - (4.0 - PI) * 25.0;
    let inner = 46.0 * 26.0 - (4.0 - PI) * 9.0;
    assert_close("volume", s.volume().unwrap(), outer * 10.0 - inner * 8.0);
    s.validate().unwrap();
    assert!(
        names(&s)
            .iter()
            .any(|n| n.starts_with("shell of fillet of"))
    );
}

#[test]
fn curved_faces_other_than_cylinders_and_cones_are_refused() {
    // A full sphere: revolve a half disc about the y axis.
    let p = Profile::new(
        vec![
            ProfileEdge::arc_from_points(
                Point2::new(0.0, 0.0),
                Point2::new(0.0, -5.0),
                Point2::new(0.0, 5.0),
            )
            .unwrap(),
            ProfileEdge::line(Point2::new(0.0, 5.0), Point2::new(0.0, -5.0)),
        ],
        vec![],
    );
    let axis = Line2::new(Point2::ORIGIN, Vec2::Y).unwrap();
    let b = revolve(&Frame::XY, &p, &axis, TAU).unwrap();
    assert!(matches!(
        shell(&b, &[], 1.0),
        Err(KernelError::Unsupported(_))
    ));
}
