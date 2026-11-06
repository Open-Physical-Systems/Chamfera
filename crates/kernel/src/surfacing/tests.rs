use super::*;
use crate::{TessOptions, TopologyError, tessellate};
use chamfera_geom::{Point3, Vec3};
use std::f64::consts::PI;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

fn rect() -> Profile {
    Profile::polygon(&[
        Point2::new(0.0, 0.0),
        Point2::new(50.0, 0.0),
        Point2::new(50.0, 30.0),
        Point2::new(0.0, 30.0),
    ])
}

/// The open sides of the 50 x 30 rectangle extruded by 10.
fn tube() -> Body {
    surface_extrude(&Frame::XY, &SurfaceProfile::Closed(rect()), 10.0).unwrap()
}

/// The cylinder of radius 10 and height 20 about the y axis: the line x = 10 from y = 0 to
/// y = 20 turned about the y axis.
fn cylinder() -> Body {
    let line = vec![ProfileEdge::line(Point2::new(10.0, 0.0), Point2::new(10.0, 20.0))];
    let axis = Line2::new(Point2::ORIGIN, Vec2::new(0.0, 1.0)).unwrap();
    surface_revolve(
        &Frame::XY,
        &SurfaceProfile::Open(line),
        &axis,
        2.0 * PI,
    )
    .unwrap()
}

#[test]
fn a_surface_extrude_of_a_rectangle_is_four_open_sides() {
    let b = tube();
    // Sides: 2 (50 + 30) x 10 = 1600; rims: the 4 bottom and 4 top edges.
    assert!(close(b.area().unwrap(), 1600.0));
    assert_eq!(b.faces().len(), 4);
    assert_eq!(b.boundary_edges().len(), 8);
    assert!(b.is_open());
    assert!(b.volume().is_err());
    assert!(b.mass_properties(1.0).is_err());
    assert!(matches!(b.validate(), Err(TopologyError::OpenShell(_))));
    let r = b.validate_as(Validation::Surface).unwrap();
    // V - E + F of an open tube (an annulus): 8 - 12 + 4 = 0 = 2 - 2g - b with b = 2.
    let s = r.summary;
    assert_eq!(
        s.vertices as i64 - s.edges as i64 + s.faces as i64,
        0,
        "{s:?}"
    );
    let mesh = tessellate(&b, &TessOptions::default()).unwrap();
    assert!(close(mesh.mesh.area(), 1600.0));
    assert_eq!(mesh.edge_polylines.len(), 12);
}

#[test]
fn an_open_chain_extrudes_into_its_own_sides_only() {
    // (0,0) -> (50,0) -> (50,30) by 10: 50 x 10 + 30 x 10 = 800, rims: 2 + 2 + 2 ends.
    let chain = vec![
        ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(50.0, 0.0)),
        ProfileEdge::line(Point2::new(50.0, 0.0), Point2::new(50.0, 30.0)),
    ];
    let b = surface_extrude(&Frame::XY, &SurfaceProfile::Open(chain), 10.0).unwrap();
    assert!(close(b.area().unwrap(), 800.0));
    assert_eq!(b.faces().len(), 2);
    assert_eq!(b.boundary_edges().len(), 6);
    // A single straight line too (its chord would be degenerate): 40 x 5.
    let one = vec![ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(40.0, 0.0))];
    let b = surface_extrude(&Frame::XY, &SurfaceProfile::Open(one), 5.0).unwrap();
    assert!(close(b.area().unwrap(), 200.0));
    assert_eq!(b.boundary_edges().len(), 4);
}

#[test]
fn a_revolved_open_line_is_a_cylinder_surface() {
    let b = cylinder();
    // 2 pi r h = 2 pi 10 20 = 400 pi; rims: the two circles.
    assert!(close(b.area().unwrap(), 400.0 * PI));
    assert_eq!(b.faces().len(), 1);
    assert_eq!(b.boundary_edges().len(), 2);
    tessellate(&b, &TessOptions::default()).unwrap();
}

#[test]
fn patching_both_ends_and_stitching_closes_the_box() {
    let b = tube();
    let rim = |z: f64| -> Vec<EdgeId> {
        b.boundary_edges()
            .into_iter()
            .filter(|e| {
                let [v0, _] = b.edge_vertices(*e).unwrap();
                (b.vertex_point(v0).unwrap().z - z).abs() < 1e-9
            })
            .collect()
    };
    let bottom = patch_edges(&b, &rim(0.0)).unwrap();
    let top = patch_edges(&b, &rim(10.0)).unwrap();
    assert!(close(bottom.area().unwrap(), 1500.0));
    // Two patches alone share nothing.
    assert!(stitch(&[&bottom, &top]).is_err());
    let solid = stitch(&[&b, &bottom, &top]).unwrap();
    assert!(!solid.is_open());
    solid.validate().unwrap();
    // 50 x 30 x 10 = 15000.
    assert!(close(solid.volume().unwrap(), 15000.0));
    // One end only: still a surface, with the other rim left.
    let cup = stitch(&[&b, &bottom]).unwrap();
    assert!(cup.is_open());
    assert_eq!(cup.boundary_edges().len(), 4);
    assert!(close(cup.area().unwrap(), 3100.0));
}

#[test]
fn a_patch_fills_a_sketch_profile_facing_the_sketch_normal() {
    let p = patch_profile(&Frame::XY, &rect()).unwrap();
    assert!(close(p.area().unwrap(), 1500.0));
    let f = p.faces()[0];
    let Some((Surface::Plane(frame), same)) = p.face_surface(f).unwrap() else {
        panic!("not planar")
    };
    let n = if same { frame.z() } else { -frame.z() };
    assert!(n.dot(Vec3::Z) > 0.999);
    assert_eq!(p.face_name(f).unwrap(), Some("patch"));
}

#[test]
fn trimming_a_cylinder_at_half_height_halves_it() {
    let c = cylinder();
    let plane = Frame::from_normal(Point3::new(0.0, 10.0, 0.0), Vec3::Y).unwrap();
    let below = trim_by_plane(&c, &plane, true).unwrap();
    // 400 pi / 2.
    assert!(close(below.area().unwrap(), 200.0 * PI), "{}", below.area().unwrap());
    assert!(below.is_open());
    let above = trim_by_plane(&c, &plane, false).unwrap();
    assert!(close(above.area().unwrap(), 200.0 * PI));
    // A plane missing the surface leaves it whole or nothing.
    let far = Frame::from_normal(Point3::new(0.0, 50.0, 0.0), Vec3::Y).unwrap();
    assert!(close(
        trim_by_plane(&c, &far, true).unwrap().area().unwrap(),
        400.0 * PI
    ));
    assert!(matches!(
        trim_by_plane(&c, &far, false),
        Err(KernelError::EmptyResult)
    ));
}

#[test]
fn thickening_a_patch_by_2_gives_3000() {
    let p = patch_profile(&Frame::XY, &rect()).unwrap();
    let f = p.faces()[0];
    // 50 x 30 x 2.
    let solid = crate::thicken(&p, f, 0.0, 2.0).unwrap();
    assert!(close(solid.volume().unwrap(), 3000.0));
}

#[test]
fn hostile_or_impossible_surfaces_are_refused() {
    let k = |p: SurfaceProfile, d: f64| surface_extrude(&Frame::XY, &p, d);
    assert!(k(SurfaceProfile::Open(Vec::new()), 10.0).is_err());
    assert!(k(SurfaceProfile::Closed(rect()), 0.0).is_err());
    assert!(k(SurfaceProfile::Closed(rect()), f64::NAN).is_err());
    let gap = vec![
        ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(1.0, 0.0)),
        ProfileEdge::line(Point2::new(2.0, 0.0), Point2::new(3.0, 0.0)),
    ];
    assert!(k(SurfaceProfile::Open(gap), 10.0).is_err());
    let b = tube();
    assert!(patch_edges(&b, &[]).is_err());
    // Three rim edges do not close.
    let rim = b.boundary_edges();
    assert!(patch_edges(&b, &rim[..3]).is_err());
    let solid = crate::extrude(&Frame::XY, &rect(), 10.0).unwrap();
    assert!(stitch(&[&solid, &b]).is_err());
    assert!(stitch(&[]).is_err());
}

#[test]
fn an_open_body_survives_a_json_round_trip() {
    let b = tube();
    let text = serde_json::to_string(&b).unwrap();
    let back: Body = serde_json::from_str(&text).unwrap();
    assert_eq!(back, b);
    assert!(back.is_open());
}
