//! Edge blends (rung K11): constant-radius fillets and equal-distance chamfers.
//!
//! # Geometry
//! A constant-radius fillet is the envelope of a ball of radius `r` rolling along an
//! edge while touching both faces (the "rolling-ball blend" of Vida, Martin and Varady,
//! "A survey of blending methods that use parametric surfaces", *Computer-Aided Design*
//! 26(5), 1994, section 3; Choi and Ju, "Constant-radius blending in surface modelling",
//! *Computer-Aided Design* 21(4), 1989). Between two planes the ball's centre runs on the
//! *spine*, the line where the two planes offset by `r` toward the ball meet, and the
//! envelope is an exact [`Cylinder`](crate::Cylinder) about the spine. With `w_A`, `w_B`
//! the unit directions from the edge into each face (perpendicular to it) and `β` the
//! angle between them, the ball touches each face along a *contact line* at the setback
//! `s = r / tan(β / 2)` from the edge (`s = r` for a right angle). On a convex edge the
//! ball is inside the material and the fillet removes material; on a concave edge it is
//! outside and the fillet adds material. An equal-distance chamfer cuts the edge by the
//! plane through the two lines at setback `d` on each face.
//!
//! Every face next to a blended edge keeps its surface and its name; its loop is the old
//! one with each blended edge moved inward by its setback (Stroud, *Boundary
//! Representation Modelling Techniques*, 2006, chapter on blending: blends as local
//! replacement of edge and vertex neighbourhoods). What happens at an end vertex of a
//! blended edge depends on how many of the vertex's three edges are blended:
//!
//! | Blended at the vertex | Chamfer | Fillet |
//! |---|---|---|
//! | one (the third face is an end face) | the bevel's end is a segment in the end face | the cylinder's end is its section by the end face's plane: a circle when the plane is perpendicular to the edge, an ellipse otherwise |
//! | two (they share one face) | the two bevels meet along a segment | the two cylinders (equal radii, spines meeting at the point `P` at distance `r` from all three planes) meet along an ellipse in the plane through `P` bisecting their axes (K6) |
//! | three | a triangular corner face through the three face corners | a spherical corner patch (an exact [`Sphere`](crate::Sphere) of radius `r` about `P`), bounded by the three cylinders' cross-section circles through `P` |
//!
//! The rim of a circular edge between a plane and a cylinder whose axis is perpendicular
//! to it (the rim of a hole or of a boss, its one vertex on the cylinder's seam) gets an
//! exact [`Torus`](crate::Torus) (fillet) or [`Cone`](crate::Cone) (chamfer), with a
//! meridian seam edge so the band can be measured and tessellated. An *arc* of such a rim
//! (the rounded end of an extruded slot or rounded rectangle) gets a piece of the same
//! torus or cone, ending in its meridian section (`rim.rs`).
//!
//! A straight edge whose faces are ruled along it (planes, cylinders whose axis runs
//! along the edge, extruded splines extruded along it) is blended in its cross-section:
//! the 2D fillet of the two section curves is a circular arc, so the fillet is an exact
//! [`Cylinder`](crate::Cylinder) about a spine parallel to the edge, and the chamfer a
//! plane through two rulings (`ruled.rs`; the ball's contact points are solved by Newton
//! when a section is curved, then checked against the surfaces).
//!
//! Where two blended edges continue each other tangentially (a profile's line meeting
//! its arc, both on a planar top face, their side faces tangent), both blends have the
//! same section there: the ball's circle in the plane normal to the common tangent. That
//! quarter circle (or a segment, for chamfers) is their common end edge, and the tangent
//! edge between the side faces is cut short at the common contact point.
//!
//! A variable-radius fillet ([`fillet_variable`]) along a straight edge between planes,
//! with the radius linear from `r0` to `r1`, is the envelope of balls whose centres run
//! on a straight spine: an exact [`Cone`](crate::Cone) with `sin α = (r1 - r0) / |spine|`,
//! touching each face along a line through the apex on the edge's line; its ends are the
//! cone's elliptical sections by the end faces (the variable-radius rolling ball of Vida, Martin
//! and Varady, 1994).
//!
//! # Supported and refused
//!
//! | Edge | Faces | Fillet | Chamfer | Ends |
//! |---|---|---|---|---|
//! | straight | two planes, convex or concave | exact cylinder | plane | end faces (planar), two-blend corners, three-blend sphere or triangle corners |
//! | straight, along the rulings | plane or cylinder or extruded spline, any pair | exact cylinder | plane | planar end faces, tangent joins |
//! | straight, variable radius | two planes | exact cone | - | planar end faces only |
//! | circle, closed | plane and perpendicular cylinder | exact torus | cone | its seam vertex |
//! | arc | plane and perpendicular cylinder | exact torus piece | cone piece | tangent joins, planar end faces through the axis |
//! | ellipse or spline where a plane meets a cylinder or extrusion obliquely | | refused: the rolling-ball surface is free-form and needs NURBS blend surfaces | | |
//! | curve between two curved faces (pipes in a T) | | refused: no exact surface (needs NURBS blend surfaces); equal pipes also touch at two points where the blend would pinch | | |
//!
//! Nothing is approximated: every blend surface above is exact. Refused with
//! [`KernelError::Unsupported`]: the rows marked refused, seams, vertices with more than
//! three edges, a blend ending on a curved face (rather than continuing tangentially),
//! corners of several blends other than between planes or at tangent joins, corners
//! mixing convex and concave blended edges, two blends at a corner whose setbacks on the
//! unblended edge differ (faces meeting at unequal angles), variable radii on anything
//! but a single straight edge between planes. Refused with [`KernelError::InvalidInput`]:
//! a size that is not a finite length above [`LINEAR_TOLERANCE`], no edges, unknown
//! edges, a ball that cannot touch both faces, and a blend that would consume a face or
//! edge (a setback reaching another edge) or make a face cross itself. Every result
//! passes [`Body::validate`]; nothing half-built is ever returned. Not detected yet: a
//! blend surface passing through a distant, unrelated face.
//!
//! # Names (the start of persistent naming)
//! Every face, edge and vertex the blend keeps keeps its name, so a sketch attached to
//! `top` still finds its face. New entities are named from the edge or vertex that made
//! them (`<kind>` is `fillet` or `chamfer`; an unnamed edge is called by its faces,
//! `edge between <face> and <face>`):
//!
//! | Entity | Name |
//! |---|---|
//! | blend face of edge `e` | `<kind> of <e>` |
//! | corner face at vertex `v` | `<kind> corner at <v>` |
//! | contact edge of `e` on face `f` | `<kind> of <e> on <f>` |
//! | end edge of `e`'s blend at vertex `v` | `<kind> of <e> at <v>` |
//! | seam of a rim blend of `e` | `<kind> of <e> seam` |
//! | new vertex on edge `e` near `v` | `<kind> of <e> at <v>` |
//! | new vertex in face `f` near `v` | `<kind> of <f> at <v>` |

mod build;
mod rim;
mod ruled;
#[cfg(test)]
mod tests;

use crate::topology::{Body, EdgeId, MAX_NAME_LENGTH};
use crate::{KernelError, KernelResult};
use chamfera_geom::{LINEAR_TOLERANCE, MAX_COORDINATE};
use std::collections::BTreeSet;

/// Most edges one blend may take (hostile-input limit).
pub const MAX_BLEND_EDGES: usize = 10_000;

/// Which blend to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Chamfer,
    Fillet,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Chamfer => "chamfer",
            Kind::Fillet => "fillet",
        }
    }
}

/// `body` with each of `edges` cut by an equal-distance chamfer: a plane through the two
/// lines at `distance` from the edge on each of its faces. See the module documentation
/// for what is supported and how faces are named.
pub fn chamfer(body: &Body, edges: &[EdgeId], distance: f64) -> KernelResult<Body> {
    blend(body, edges, distance, Kind::Chamfer, None)
}

/// `body` with each of `edges` rounded by a constant-radius fillet of `radius`
/// (cylinders, spherical corners, tori on rims). See the module documentation for what is
/// supported and how faces are named.
pub fn fillet(body: &Body, edges: &[EdgeId], radius: f64) -> KernelResult<Body> {
    blend(body, edges, radius, Kind::Fillet, None)
}

/// `body` with the straight `edge` between two planar faces rounded by a fillet whose
/// radius changes linearly from `start` at the edge's first vertex
/// (`body.edge_vertices(edge)[0]`) to `end` at its second: an exact cone. Both ends must
/// meet plain planar end faces (no other blended edge). Equal radii give the
/// constant-radius fillet.
pub fn fillet_variable(body: &Body, edge: EdgeId, start: f64, end: f64) -> KernelResult<Body> {
    for r in [start, end] {
        check_size(r, Kind::Fillet)?;
    }
    blend(
        body,
        &[edge],
        start.max(end),
        Kind::Fillet,
        Some([start, end]),
    )
}

fn check_size(size: f64, kind: Kind) -> KernelResult<()> {
    let what = match kind {
        Kind::Chamfer => "distance",
        Kind::Fillet => "radius",
    };
    if !(size.is_finite() && size > LINEAR_TOLERANCE && size <= MAX_COORDINATE) {
        return Err(KernelError::InvalidInput(format!(
            "the {} {what} must be a finite length with {LINEAR_TOLERANCE} < {what} <= {MAX_COORDINATE} mm",
            kind.word()
        )));
    }
    Ok(())
}

fn blend(
    body: &Body,
    edges: &[EdgeId],
    size: f64,
    kind: Kind,
    variable: Option<[f64; 2]>,
) -> KernelResult<Body> {
    check_size(size, kind)?;
    if edges.is_empty() {
        return Err(KernelError::InvalidInput(format!(
            "a {} needs at least one edge",
            kind.word()
        )));
    }
    if edges.len() > MAX_BLEND_EDGES {
        return Err(KernelError::InvalidInput(format!(
            "a {} may take at most {MAX_BLEND_EDGES} edges",
            kind.word()
        )));
    }
    let selected: BTreeSet<EdgeId> = edges.iter().copied().collect();
    for e in &selected {
        body.edge_vertices(*e)?;
    }
    body.validate().map_err(|e| {
        KernelError::InvalidInput(format!("the body to blend is not a valid solid: {e}"))
    })?;
    // Equal radii are a constant-radius fillet.
    let variable = variable.filter(|[a, b]| (a - b).abs() > LINEAR_TOLERANCE);
    build::Blend::new(body, &selected, size, kind, variable)?.run()
}

/// `name` cut to at most [`MAX_NAME_LENGTH`] bytes on a character boundary.
fn bounded(name: String) -> String {
    if name.len() <= MAX_NAME_LENGTH {
        return name;
    }
    let mut end = MAX_NAME_LENGTH;
    while end > 0 && !name.is_char_boundary(end) {
        end -= 1;
    }
    name.get(..end).unwrap_or_default().to_owned()
}
