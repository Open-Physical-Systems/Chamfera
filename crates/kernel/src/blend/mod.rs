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
//! # Unequal chamfers
//! [`chamfer_with`] cuts each edge by the plane through two lines at different setbacks:
//! `d1` on the edge's *reference face* and `d2` on the other. Given a distance `d` and an
//! angle `θ` instead (the angle between the reference face and the bevel), the second
//! setback follows from the triangle in the edge's cross-section, whose corner angle is
//! `β` (the angle between the faces on the side the bevel cuts): its angles are `β` at
//! the edge, `θ` at the reference face's contact and `π - β - θ` at the other, so by the
//! law of sines `d2 = d sin θ / sin(β + θ)` (`d tan θ` for a right angle). A rim between
//! a plane and a perpendicular cylinder is a right angle in every meridian section, so
//! its bevel is still an exact cone. Curved faces ruled along the edge take two distances
//! (each a straight distance from the edge, as for the equal chamfer) but not an angle,
//! whose bevel would need the faces' tangent planes rather than their sections.
//!
//! # Tangent chains
//! [`tangent_chain`] grows a set of edges by every edge that continues one of them
//! smoothly: at a shared vertex the two edges' tangents leaving the vertex point in
//! opposite directions (within [`TANGENT_CHAIN_TOLERANCE`]). Seams are never added.
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

use crate::topology::{Body, EdgeId, FaceId, MAX_NAME_LENGTH, VertexId};
use crate::{KernelError, KernelResult};
use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE, MAX_COORDINATE, Vec3};
use std::collections::BTreeSet;

/// Most edges one blend may take (hostile-input limit).
pub const MAX_BLEND_EDGES: usize = 10_000;

/// Largest sine of the angle between two edges' tangents at a shared vertex for
/// [`tangent_chain`] to count them as continuing each other. Sketch tangency is solved to
/// about 1e-9 mm over a part's size, which leaves directions parallel to well under this.
pub const TANGENT_CHAIN_TOLERANCE: f64 = 1e-6;

/// The second size of a chamfer, besides its distance on the reference face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChamferSecond {
    /// The same distance on both faces.
    Equal,
    /// This distance (mm) on the other face.
    Distance(f64),
    /// The bevel makes this angle (radians, `0 < θ < π`) with the reference face.
    Angle(f64),
}

/// How a chamfer cuts each of its edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChamferSpec {
    /// The setback (mm) on each edge's reference face.
    pub distance: f64,
    pub second: ChamferSecond,
    /// The reference face of every edge that bounds it; an edge that does not uses the
    /// face holding its first half-edge.
    pub reference: Option<FaceId>,
    /// Swap each edge's reference face for its other face.
    pub flip: bool,
}

impl ChamferSpec {
    /// An equal-distance chamfer.
    pub fn equal(distance: f64) -> ChamferSpec {
        ChamferSpec {
            distance,
            second: ChamferSecond::Equal,
            reference: None,
            flip: false,
        }
    }

    /// The index (0 or 1, into `body.edge_faces(e)`) of the reference face of an edge
    /// between `faces`.
    pub(super) fn reference_index(&self, faces: [FaceId; 2]) -> usize {
        let i = usize::from(
            self.reference
                .is_some_and(|r| r == faces[1] && r != faces[0]),
        );
        if self.flip { 1 - i } else { i }
    }

    /// The setbacks on the faces `[0]` and `[1]` of an edge whose reference face is
    /// `reference` and whose faces meet at `beta` (radians) on the side the bevel cuts.
    pub(super) fn setbacks(
        &self,
        reference: usize,
        beta: f64,
        label: &str,
    ) -> KernelResult<[f64; 2]> {
        let d = self.distance;
        let other = match self.second {
            ChamferSecond::Equal => d,
            ChamferSecond::Distance(d2) => d2,
            ChamferSecond::Angle(theta) => {
                let den = (beta + theta).sin();
                if beta + theta >= std::f64::consts::PI - ANGULAR_TOLERANCE.sqrt()
                    || den <= ANGULAR_TOLERANCE.sqrt()
                {
                    return Err(KernelError::InvalidInput(format!(
                        "the chamfer angle is too steep for {label}: with its faces at {:.3} degrees the bevel would never reach the other face; use an angle below {:.3} degrees",
                        beta.to_degrees(),
                        (std::f64::consts::PI - beta).to_degrees()
                    )));
                }
                d * theta.sin() / den
            }
        };
        check_size(other, Kind::Chamfer)?;
        Ok(if reference == 0 {
            [d, other]
        } else {
            [other, d]
        })
    }
}

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
    chamfer_with(body, edges, &ChamferSpec::equal(distance))
}

/// `body` with each of `edges` cut by a chamfer as `spec` describes: equal distances, two
/// distances, or a distance and an angle (see the module documentation).
pub fn chamfer_with(body: &Body, edges: &[EdgeId], spec: &ChamferSpec) -> KernelResult<Body> {
    check_size(spec.distance, Kind::Chamfer)?;
    match spec.second {
        ChamferSecond::Equal => {}
        ChamferSecond::Distance(d) => check_size(d, Kind::Chamfer)?,
        ChamferSecond::Angle(a) => {
            if !(a.is_finite()
                && a > ANGULAR_TOLERANCE.sqrt()
                && a < std::f64::consts::PI - ANGULAR_TOLERANCE.sqrt())
            {
                return Err(KernelError::InvalidInput(
                    "the chamfer angle must be above 0 and below 180 degrees".into(),
                ));
            }
        }
    }
    if let Some(f) = spec.reference {
        body.face_surface(f)?;
    }
    blend_with(body, edges, spec.distance, Kind::Chamfer, None, Some(*spec))
}

/// `seeds` and every edge of `body` that continues one of them tangentially, directly
/// or through others (see the module documentation), in the order found: the seeds
/// first. Seams are never added. At most [`MAX_BLEND_EDGES`] edges.
pub fn tangent_chain(body: &Body, seeds: &[EdgeId]) -> KernelResult<Vec<EdgeId>> {
    let mut out: Vec<EdgeId> = Vec::new();
    let mut seen = BTreeSet::new();
    for e in seeds {
        body.edge_vertices(*e)?;
        if seen.insert(*e) {
            out.push(*e);
        }
    }
    let mut next = 0;
    while let Some(&e) = out.get(next) {
        next += 1;
        let [v0, v1] = body.edge_vertices(e)?;
        for v in [v0, v1] {
            let Some(te) = leaving_tangent(body, e, v)? else {
                continue;
            };
            for f in body.vertex_edges(v)? {
                if seen.contains(&f) {
                    continue;
                }
                let [fa, fb] = body.edge_faces(f)?;
                if fa == fb {
                    continue;
                }
                let Some(tf) = leaving_tangent(body, f, v)? else {
                    continue;
                };
                if te.dot(tf) < 0.0 && te.cross(tf).length() <= TANGENT_CHAIN_TOLERANCE {
                    if out.len() >= MAX_BLEND_EDGES {
                        return Err(KernelError::InvalidInput(format!(
                            "the tangent chain has more than {MAX_BLEND_EDGES} edges"
                        )));
                    }
                    seen.insert(f);
                    out.push(f);
                }
            }
        }
    }
    Ok(out)
}

/// The unit tangent of edge `e` leaving its end vertex `v` (into the edge), or `None`
/// when it has none there.
fn leaving_tangent(body: &Body, e: EdgeId, v: VertexId) -> KernelResult<Option<Vec3>> {
    let [v0, v1] = body.edge_vertices(e)?;
    let Some((c, t0, t1)) = body.edge_curve(e)? else {
        return Ok(None);
    };
    // A closed edge leaves its vertex both ways; its start direction stands for it.
    Ok(if v == v0 {
        c.d1(t0).normalized()
    } else if v == v1 {
        c.d1(t1).normalized().map(|t| -t)
    } else {
        None
    })
}

/// `body` with each of `edges` rounded by a constant-radius fillet of `radius`
/// (cylinders, spherical corners, tori on rims). See the module documentation for what is
/// supported and how faces are named.
pub fn fillet(body: &Body, edges: &[EdgeId], radius: f64) -> KernelResult<Body> {
    blend_with(body, edges, radius, Kind::Fillet, None, None)
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
    blend_with(
        body,
        &[edge],
        start.max(end),
        Kind::Fillet,
        Some([start, end]),
        None,
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

fn blend_with(
    body: &Body,
    edges: &[EdgeId],
    size: f64,
    kind: Kind,
    variable: Option<[f64; 2]>,
    chamfer: Option<ChamferSpec>,
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
    build::Blend::new(body, &selected, size, kind, variable, chamfer)?.run()
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
