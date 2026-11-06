//! Tests for face edits. Expected volumes are closed forms derived in the comments, never
//! the code's own output.

use super::*;
use crate::{Profile, ProfileEdge, chamfer, extrude, fillet, polygon_edges};
use chamfera_geom::{Frame, Point2, Point3, Vec3};
use std::f64::consts::PI;

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

/// 50 x 30 x 10 on the XY plane: 15000 mm³.
fn block() -> Body {
    extrude(&Frame::XY, &Profile::polygon(&rect(50.0, 30.0)), 10.0).unwrap()
}

/// The block with a through hole of diameter 6 at (25, 15).
fn plate_with_hole(radius: f64) -> Body {
    let p = Profile::new(polygon_edges(&rect(50.0, 30.0)), vec![])
        .with_hole(vec![ProfileEdge::circle(Point2::new(25.0, 15.0), radius)]);
    extrude(&Frame::XY, &p, 10.0).unwrap()
}

fn face(b: &Body, name: &str) -> FaceId {
    b.faces()
        .into_iter()
        .find(|f| b.face_name(*f).unwrap() == Some(name))
        .unwrap_or_else(|| panic!("no face named {name}"))
}

fn edge(b: &Body, name: &str) -> crate::EdgeId {
    b.edges()
        .into_iter()
        .find(|e| b.edge_name(*e).unwrap() == Some(name))
        .unwrap_or_else(|| panic!("no edge named {name}"))
}

fn faces_where(b: &Body, keep: impl Fn(&Surface) -> bool) -> Vec<FaceId> {
    b.faces()
        .into_iter()
        .filter(|f| keep(&b.face_surface(*f).unwrap().unwrap().0))
        .collect()
}

fn volume(b: &Body) -> f64 {
    b.validate().unwrap();
    b.volume().unwrap()
}

#[test]
fn offsetting_the_top_of_a_box_moves_it_and_stretches_the_sides() {
    let b = block();
    let top = face(&b, "top");
    // 50 x 30 x (10 + 5) and 50 x 30 x (10 - 4).
    let up = offset_faces(&b, &[top], 5.0).unwrap();
    assert_close("up", volume(&up), 22500.0);
    let down = offset_faces(&b, &[top], -4.0).unwrap();
    assert_close("down", volume(&down), 9000.0);
    // Same topology and names: the top is still `top`, now at z = 6.
    assert_eq!(down.faces().len(), 6);
    let t = face(&down, "top");
    assert_close("top area", down.face_area(t).unwrap(), 1500.0);
    let z = down
        .vertices()
        .into_iter()
        .map(|v| down.vertex_point(v).unwrap().z)
        .fold(f64::NEG_INFINITY, f64::max);
    assert_close("height", z, 6.0);
}

#[test]
fn offsetting_two_opposite_sides_widens_the_box() {
    let b = block();
    // side 0 is y = 0 and side 2 is y = 30: the width 30 grows by 2 x 3.
    let out = offset_faces(&b, &[face(&b, "side 0"), face(&b, "side 2")], 3.0).unwrap();
    assert_close("volume", volume(&out), 50.0 * 36.0 * 10.0);
}

#[test]
fn offsetting_a_hole_wall_changes_its_radius() {
    let b = plate_with_hole(3.0);
    let wall = faces_where(&b, |s| matches!(s, Surface::Cylinder(_)));
    assert_eq!(wall.len(), 1);
    // The wall's outward normal points into the hole, so -1 widens it: radius 3 -> 4,
    // volume 15000 - π 4² 10.
    let out = offset_faces(&b, &wall, -1.0).unwrap();
    assert_close("wider", volume(&out), 15000.0 - 16.0 * PI * 10.0);
    // +1 narrows it to radius 2.
    let out = offset_faces(&b, &wall, 1.0).unwrap();
    assert_close("narrower", volume(&out), 15000.0 - 4.0 * PI * 10.0);
    // +3 would close the hole: refused.
    assert!(offset_faces(&b, &wall, 3.0).is_err());
}

#[test]
fn offsetting_the_side_of_a_cylinder_changes_its_radius() {
    let b = extrude(
        &Frame::XY,
        &Profile::circle(Point2::new(0.0, 0.0), 10.0),
        20.0,
    )
    .unwrap();
    let side = faces_where(&b, |s| matches!(s, Surface::Cylinder(_)));
    let out = offset_faces(&b, &side, 2.0).unwrap();
    assert_close("volume", volume(&out), PI * 144.0 * 20.0);
}

#[test]
fn offsetting_a_face_next_to_a_fillet_keeps_the_fillet() {
    // A 60 x 40 x 15 block with its four vertical edges filleted (r = 8): raising the
    // top by 5 stretches every fillet cylinder; the cross-section is the same rounded
    // rectangle, 60·40 - 4 (1 - π/4) 8², over 20 instead of 15.
    let b = extrude(&Frame::XY, &Profile::polygon(&rect(60.0, 40.0)), 15.0).unwrap();
    let edges: Vec<_> = (0..4).map(|k| edge(&b, &format!("sweep {k}"))).collect();
    let b = fillet(&b, &edges, 8.0).unwrap();
    let section = 2400.0 - 4.0 * (1.0 - PI / 4.0) * 64.0;
    assert_close("before", volume(&b), section * 15.0);
    let out = offset_faces(&b, &[face(&b, "top")], 5.0).unwrap();
    assert_close("after", volume(&out), section * 20.0);
}

#[test]
fn an_offset_that_would_cross_another_face_is_refused_and_changes_nothing() {
    let b = block();
    let before = b.clone();
    let top = face(&b, "top");
    // -10 puts the top on the bottom; -15 below it.
    assert!(offset_faces(&b, &[top], -10.0).is_err());
    assert!(offset_faces(&b, &[top], -15.0).is_err());
    assert_eq!(b, before);
    // A pocket 10 x 10, 4 deep in the top: pushing its floor 8 down crosses the bottom.
    let pocket = extrude(
        &Frame::XY.with_origin(Point3::new(0.0, 0.0, 6.0)).unwrap(),
        &Profile::polygon(&[
            Point2::new(20.0, 10.0),
            Point2::new(30.0, 10.0),
            Point2::new(30.0, 20.0),
            Point2::new(20.0, 20.0),
        ]),
        10.0,
    )
    .unwrap();
    let b = crate::subtract(&b, &pocket).unwrap();
    let floor: Vec<_> = b
        .faces()
        .into_iter()
        .filter(|f| {
            matches!(b.face_surface(*f).unwrap(), Some((Surface::Plane(p), _)) if (p.origin().z - 6.0).abs() < 1e-9 && p.z().z.abs() > 0.5)
        })
        .collect();
    assert_eq!(floor.len(), 1);
    // Down 2: the pocket is 6 deep, 15000 - 100·6.
    let ok = offset_faces(&b, &floor, -2.0).unwrap();
    assert_close("deeper pocket", volume(&ok), 15000.0 - 600.0);
    assert!(offset_faces(&b, &floor, -8.0).is_err());
}

#[test]
fn hostile_offsets_are_refused() {
    let b = block();
    let top = face(&b, "top");
    for d in [0.0, f64::NAN, f64::INFINITY, -f64::INFINITY, 1e300, 1e-12] {
        assert!(offset_faces(&b, &[top], d).is_err(), "{d}");
    }
    assert!(offset_faces(&b, &[], 1.0).is_err());
    let other = plate_with_hole(3.0);
    let alien = other.faces().into_iter().max().unwrap();
    if !b.faces().contains(&alien) {
        assert!(offset_faces(&b, &[alien], 1.0).is_err());
    }
}

#[test]
fn deleting_a_fillet_face_restores_the_sharp_corner() {
    // 60 x 40 x 15 block, four vertical edges filleted with r = 8 (the filleted-block
    // example without its hole). Each fillet removed 8 x 8 minus a quarter disc of r 8
    // from the section: (64 - 16π) per unit height.
    let b = extrude(&Frame::XY, &Profile::polygon(&rect(60.0, 40.0)), 15.0).unwrap();
    let edges: Vec<_> = (0..4).map(|k| edge(&b, &format!("sweep {k}"))).collect();
    let b = fillet(&b, &edges, 8.0).unwrap();
    let corner = (64.0 - 16.0 * PI) * 15.0;
    assert_close("filleted", volume(&b), 36000.0 - 4.0 * corner);
    let one = face(&b, "fillet of sweep 0");
    let out = delete_faces(&b, &[one]).unwrap();
    assert_close("one corner back", volume(&out), 36000.0 - 3.0 * corner);
    assert_eq!(out.faces().len(), b.faces().len() - 1);
    let all: Vec<_> = (0..4)
        .map(|k| face(&b, &format!("fillet of sweep {k}")))
        .collect();
    let out = delete_faces(&b, &all).unwrap();
    assert_close("sharp block", volume(&out), 36000.0);
    assert_eq!(out.faces().len(), 6);
    // The neighbours keep their names.
    face(&out, "top");
    face(&out, "side 0");
}

#[test]
fn deleting_a_box_edge_fillet_or_chamfer_gives_the_box_back() {
    let b = block();
    let e = edge(&b, "top 0");
    let f = fillet(&b, &[e], 3.0).unwrap();
    let out = delete_faces(&f, &[face(&f, "fillet of top 0")]).unwrap();
    assert_close("fillet", volume(&out), 15000.0);
    let c = chamfer(&b, &[e], 2.0).unwrap();
    let out = delete_faces(&c, &[face(&c, "chamfer of top 0")]).unwrap();
    assert_close("chamfer", volume(&out), 15000.0);
    assert_eq!(out.faces().len(), 6);
}

#[test]
fn deleting_a_hole_rim_chamfer_gives_the_sharp_rim_back() {
    // The plate with a hole of r 5 (15000 - 250π), chamfered 1.5 at the top rim: a
    // ring of right-triangle section (legs 1.5) at centroid radius 5 + 0.5 is removed.
    let b = plate_with_hole(5.0);
    let c = chamfer(&b, &[edge(&b, "hole 0 top 0")], 1.5).unwrap();
    assert_close(
        "chamfered",
        volume(&c),
        15000.0 - 250.0 * PI - PI * 1.5 * 1.5 * 5.5,
    );
    let cone = faces_where(&c, |s| matches!(s, Surface::Cone(_)));
    assert_eq!(cone.len(), 1);
    let out = delete_faces(&c, &cone).unwrap();
    assert_close("sharp rim", volume(&out), 15000.0 - 250.0 * PI);
    // A rim fillet (a torus) deletes the same way.
    let f = fillet(&b, &[edge(&b, "hole 0 top 0")], 2.0).unwrap();
    let torus = faces_where(&f, |s| matches!(s, Surface::Torus(_)));
    let out = delete_faces(&f, &torus).unwrap();
    assert_close("sharp rim", volume(&out), 15000.0 - 250.0 * PI);
}

#[test]
fn deleting_a_blind_holes_wall_and_floor_fills_it() {
    // A hole of r 3, from z = 4 up through the top of the 50 x 30 x 10 block.
    let tool = extrude(
        &Frame::XY.with_origin(Point3::new(0.0, 0.0, 4.0)).unwrap(),
        &Profile::circle(Point2::new(25.0, 15.0), 3.0),
        10.0,
    )
    .unwrap();
    let b = crate::subtract(&block(), &tool).unwrap();
    assert_close("drilled", volume(&b), 15000.0 - 9.0 * PI * 6.0);
    let wall = faces_where(&b, |s| matches!(s, Surface::Cylinder(_)));
    let floor: Vec<_> = b
        .faces()
        .into_iter()
        .filter(|f| {
            matches!(b.face_surface(*f).unwrap(), Some((Surface::Plane(p), _)) if (p.origin().z - 4.0).abs() < 1e-9 && p.z().z.abs() > 0.5)
        })
        .collect();
    assert_eq!((wall.len(), floor.len()), (1, 1));
    let both: Vec<_> = wall.iter().chain(&floor).copied().collect();
    let out = delete_faces(&b, &both).unwrap();
    assert_close("filled", volume(&out), 15000.0);
    assert_eq!(out.faces().len(), 6);
    // The wall alone cannot be healed: its floor stays.
    assert!(delete_faces(&b, &wall).is_err());
    // A through hole's wall alone fills it.
    let p = plate_with_hole(3.0);
    let wall = faces_where(&p, |s| matches!(s, Surface::Cylinder(_)));
    assert_close(
        "through",
        volume(&delete_faces(&p, &wall).unwrap()),
        15000.0,
    );
}

#[test]
fn faces_that_cannot_be_healed_are_refused() {
    let b = block();
    // A side of a box: its neighbours never close the gap.
    assert!(delete_faces(&b, &[face(&b, "side 0")]).is_err());
    assert!(delete_faces(&b, &b.faces()).is_err());
}

#[test]
fn moving_the_top_face_tilts_or_lifts_it() {
    let b = block();
    let top = face(&b, "top");
    // Lifted 5: 50 x 30 x 15.
    let t = Transform::translation(Vec3::new(0.0, 0.0, 5.0)).unwrap();
    assert_close(
        "lifted",
        volume(&move_faces(&b, &[top], &t).unwrap()),
        22500.0,
    );
    // Tilted 10° about the line through its centre along x (y = 15, z = 10): the wedge
    // gained over y > 15 equals the wedge lost over y < 15, so the volume stays 15000.
    let t = Transform::rotation_about(
        Point3::new(0.0, 15.0, 10.0),
        Vec3::new(1.0, 0.0, 0.0),
        10f64.to_radians(),
    )
    .unwrap();
    let out = move_faces(&b, &[top], &t).unwrap();
    assert_close("tilted", volume(&out), 15000.0);
    // The top is a 50 x (30 / cos 10°) rectangle.
    assert_close(
        "tilted top",
        out.face_area(face(&out, "top")).unwrap(),
        50.0 * 30.0 / 10f64.to_radians().cos(),
    );
    // Turned 80° the top would cut through the bottom: refused.
    let t = Transform::rotation_about(
        Point3::new(0.0, 15.0, 10.0),
        Vec3::new(1.0, 0.0, 0.0),
        80f64.to_radians(),
    )
    .unwrap();
    assert!(move_faces(&b, &[top], &t).is_err());
    // Curved faces do not move yet.
    let p = plate_with_hole(3.0);
    let wall = faces_where(&p, |s| matches!(s, Surface::Cylinder(_)));
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0)).unwrap();
    assert!(move_faces(&p, &wall, &t).is_err());
}
