//! The Part 21 writer: header, product structure, units and the advanced B-rep.

use super::format::{logical, point, real, string, vector};
use super::{MAX_STEP_TEXT, SCHEMA, StepOptions};
use crate::IoError;
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point3, Vec3};
use chamfera_kernel::{Body, Curve, EdgeId, HalfEdgeId, LoopId, Surface, VertexId};
use std::collections::HashMap;
use std::fmt::{self, Write};

/// An entity instance name, `#n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ref(u64);

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

fn err(msg: impl Into<String>) -> IoError {
    IoError::Format(format!("STEP: {}", msg.into()))
}

fn kernel(body: &str, e: impl fmt::Display) -> IoError {
    err(format!("body {body:?}: {e}"))
}

/// The DATA section as it grows, with the entity budget.
struct Out {
    text: String,
    next: u64,
    limit: usize,
}

impl Out {
    fn add(&mut self, entity: &str) -> Result<Ref, IoError> {
        if self.next > self.limit as u64 {
            return Err(err(format!(
                "the export needs more than {} entity instances, the limit",
                self.limit
            )));
        }
        let id = Ref(self.next);
        self.next += 1;
        let _ = writeln!(self.text, "{id}={entity};");
        Ok(id)
    }

    fn point(&mut self, p: Point3) -> Result<Ref, IoError> {
        if !p.is_valid() {
            return Err(err(format!(
                "point ({}, {}, {}) is not finite or outside the working range",
                p.x, p.y, p.z
            )));
        }
        self.add(&format!("CARTESIAN_POINT('',{})", point(p)?))
    }

    fn direction(&mut self, v: Vec3) -> Result<Ref, IoError> {
        if !(v.x.is_finite() && v.y.is_finite() && v.z.is_finite()) || v.length() == 0.0 {
            return Err(err("a direction must be finite and non-zero"));
        }
        self.add(&format!("DIRECTION('',{})", vector(v)?))
    }

    /// `AXIS2_PLACEMENT_3D(name, location, axis, ref_direction)`: origin, z and x.
    fn axis(&mut self, frame: &Frame) -> Result<Ref, IoError> {
        let o = self.point(frame.origin())?;
        let z = self.direction(frame.z())?;
        let x = self.direction(frame.x())?;
        self.add(&format!("AXIS2_PLACEMENT_3D('',{o},{z},{x})"))
    }
}

fn check_text(what: &str, s: &str) -> Result<(), IoError> {
    if s.len() > MAX_STEP_TEXT {
        return Err(err(format!(
            "the {what} may be at most {MAX_STEP_TEXT} bytes long"
        )));
    }
    Ok(())
}

/// An upper bound on the instances one body needs, to refuse oversized exports before
/// building anything: per vertex 2, per edge 9 (curve with placement), per half-edge 1,
/// per loop 2, per face 6 (surface with placement), per shell 2.
fn estimate(body: &Body) -> usize {
    let s = body.summary();
    s.vertices
        .saturating_mul(2)
        .saturating_add(s.edges.saturating_mul(9))
        .saturating_add(s.half_edges)
        .saturating_add(s.loops.saturating_mul(2))
        .saturating_add(s.faces.saturating_mul(6))
        .saturating_add(s.shells.saturating_mul(2))
}

/// Writes `bodies` (each with a name) as one product in a Part 21 file and returns the
/// text, which is plain ASCII. Every body must pass [`Body::validate`]; the file is not
/// produced at all if any body cannot be written.
pub fn write(bodies: &[(&str, &Body)], options: &StepOptions) -> Result<String, IoError> {
    if bodies.is_empty() {
        return Err(err("there are no bodies to export"));
    }
    check_text("file name", &options.file_name)?;
    check_text("product name", &options.product_name)?;
    check_text("author", &options.author)?;
    check_text("organization", &options.organization)?;
    let timestamp = options
        .timestamp
        .clone()
        .unwrap_or_else(|| "1970-01-01T00:00:00".into());
    check_text("timestamp", &timestamp)?;
    let mut needed: usize = 32;
    for (name, body) in bodies {
        check_text("body name", name)?;
        body.validate().map_err(|e| kernel(name, e))?;
        if body.shells().is_empty() {
            return Err(err(format!("body {name:?} has no solid to export")));
        }
        needed = needed.saturating_add(estimate(body));
    }
    if needed > options.max_entities {
        return Err(err(format!(
            "the export would need about {needed} entity instances, the limit is {}",
            options.max_entities
        )));
    }

    let mut out = Out {
        text: String::new(),
        next: 1,
        limit: options.max_entities,
    };
    let (context, product_definition_shape) = product_and_context(&mut out, options)?;
    let mut items = Vec::new();
    for (name, body) in bodies {
        items.extend(body_solids(&mut out, name, body)?);
    }
    let origin = out.axis(&Frame::XY)?;
    items.push(origin);
    let items = items
        .iter()
        .map(Ref::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let rep = out.add(&format!(
        "ADVANCED_BREP_SHAPE_REPRESENTATION({},({items}),{context})",
        string(&options.product_name)
    ))?;
    out.add(&format!(
        "SHAPE_DEFINITION_REPRESENTATION({product_definition_shape},{rep})"
    ))?;

    let mut file = String::with_capacity(out.text.len() + 1024);
    file.push_str("ISO-10303-21;\nHEADER;\n");
    let _ = writeln!(file, "FILE_DESCRIPTION(('Chamfera B-rep export'),'2;1');");
    let _ = writeln!(
        file,
        "FILE_NAME({},{},({}),({}),{},'Chamfera','');",
        string(&options.file_name),
        string(&timestamp),
        string(&options.author),
        string(&options.organization),
        string(&format!("Chamfera {}", env!("CARGO_PKG_VERSION"))),
    );
    let _ = writeln!(file, "FILE_SCHEMA(('{SCHEMA}'));");
    file.push_str("ENDSEC;\nDATA;\n");
    file.push_str(&out.text);
    file.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    Ok(file)
}

/// The application context, product, its definition and shape, units and the geometric
/// context. Returns the representation context and the `PRODUCT_DEFINITION_SHAPE`.
fn product_and_context(out: &mut Out, options: &StepOptions) -> Result<(Ref, Ref), IoError> {
    let name = string(&options.product_name);
    let app =
        out.add("APPLICATION_CONTEXT('core data for automotive mechanical design processes')")?;
    out.add(&format!(
        "APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,{app})"
    ))?;
    let product_context = out.add(&format!("PRODUCT_CONTEXT('',{app},'mechanical')"))?;
    let product = out.add(&format!("PRODUCT({name},{name},'',({product_context}))"))?;
    out.add(&format!(
        "PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,({product}))"
    ))?;
    let formation = out.add(&format!("PRODUCT_DEFINITION_FORMATION('','',{product})"))?;
    let definition_context = out.add(&format!(
        "PRODUCT_DEFINITION_CONTEXT('part definition',{app},'design')"
    ))?;
    let definition = out.add(&format!(
        "PRODUCT_DEFINITION('design','',{formation},{definition_context})"
    ))?;
    let shape = out.add(&format!("PRODUCT_DEFINITION_SHAPE('','',{definition})"))?;

    // Units: millimetre, radian, steradian (SI_UNIT(prefix, name)); complex instances list
    // their partial entities in alphabetical order (Part 21, external mapping).
    let mm = out.add("(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.))")?;
    let rad = out.add("(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.))")?;
    let sr = out.add("(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT())")?;
    let uncertainty = out.add(&format!(
        "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE({}),{mm},'distance_accuracy_value','maximum gap between points considered coincident')",
        real(LINEAR_TOLERANCE)?
    ))?;
    let context = out.add(&format!(
        "(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT(({uncertainty}))GLOBAL_UNIT_ASSIGNED_CONTEXT(({mm},{rad},{sr}))REPRESENTATION_CONTEXT('3D',''))"
    ))?;
    Ok((context, shape))
}

/// Per-body maps from kernel entities to written instances, so shared vertices and
/// edges are written once.
struct BodyWriter<'a> {
    name: &'a str,
    body: &'a Body,
    vertices: HashMap<VertexId, Ref>,
    edges: HashMap<EdgeId, Ref>,
}

/// One `MANIFOLD_SOLID_BREP` per shell of the body.
fn body_solids(out: &mut Out, name: &str, body: &Body) -> Result<Vec<Ref>, IoError> {
    let mut w = BodyWriter {
        name,
        body,
        vertices: HashMap::new(),
        edges: HashMap::new(),
    };
    let mut solids = Vec::new();
    for shell in body.shells() {
        let mut faces = Vec::new();
        for f in body.shell_faces(shell).map_err(|e| kernel(name, e))? {
            faces.push(w.face(out, f)?);
        }
        let faces = faces
            .iter()
            .map(Ref::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let shell = out.add(&format!("CLOSED_SHELL('',({faces}))"))?;
        solids.push(out.add(&format!("MANIFOLD_SOLID_BREP({},{shell})", string(name)))?);
    }
    Ok(solids)
}

impl BodyWriter<'_> {
    fn face(&mut self, out: &mut Out, face: chamfera_kernel::FaceId) -> Result<Ref, IoError> {
        let body = self.body;
        let k = |e| kernel(self.name, e);
        let (surf, same_sense) = body
            .face_surface(face)
            .map_err(k)?
            .ok_or_else(|| kernel(self.name, format!("{face} has no surface")))?;
        let outer = body.face_outer_loop(face).map_err(k)?;
        let mut bounds = vec![self.bound(out, outer, true)?];
        for lp in body.face_inner_loops(face).map_err(k)? {
            bounds.push(self.bound(out, lp, false)?);
        }
        let surface = surface(out, &surf, self.name)?;
        let bounds = bounds
            .iter()
            .map(Ref::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let name = body.face_name(face).map_err(k)?.unwrap_or("");
        out.add(&format!(
            "ADVANCED_FACE({},({bounds}),{surface},{})",
            string(name),
            logical(same_sense)
        ))
    }

    /// `FACE_OUTER_BOUND` / `FACE_BOUND` over the loop's `EDGE_LOOP`. Our loops run
    /// with the face on their left seen against the outward normal, which is the Part 42
    /// sense of a bound with orientation `.T.`.
    fn bound(&mut self, out: &mut Out, lp: LoopId, outer: bool) -> Result<Ref, IoError> {
        let hs = self
            .body
            .loop_half_edges(lp)
            .map_err(|e| kernel(self.name, e))?;
        let mut oriented = Vec::with_capacity(hs.len());
        for h in hs {
            oriented.push(self.oriented_edge(out, h)?);
        }
        let list = oriented
            .iter()
            .map(Ref::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let edge_loop = out.add(&format!("EDGE_LOOP('',({list}))"))?;
        let kind = if outer {
            "FACE_OUTER_BOUND"
        } else {
            "FACE_BOUND"
        };
        out.add(&format!("{kind}('',{edge_loop},.T.)"))
    }

    /// `ORIENTED_EDGE(name, *, *, edge, orientation)`: `.T.` when the half-edge is the
    /// edge's first half-edge (runs from the edge's start to its end).
    fn oriented_edge(&mut self, out: &mut Out, h: HalfEdgeId) -> Result<Ref, IoError> {
        let k = |e| kernel(self.name, e);
        let e = self.body.half_edge_edge(h).map_err(k)?.ok_or_else(|| {
            kernel(
                self.name,
                format!("{h} is a one-vertex loop, which a finished solid does not have"),
            )
        })?;
        let along = self.body.edge_half_edges(e).map_err(k)?[0] == h;
        let edge = self.edge(out, e)?;
        out.add(&format!("ORIENTED_EDGE('',*,*,{edge},{})", logical(along)))
    }

    fn vertex(&mut self, out: &mut Out, v: VertexId) -> Result<Ref, IoError> {
        if let Some(r) = self.vertices.get(&v) {
            return Ok(*r);
        }
        let k = |e| kernel(self.name, e);
        let p = out.point(self.body.vertex_point(v).map_err(k)?)?;
        let name = self.body.vertex_name(v).map_err(k)?.unwrap_or("");
        let r = out.add(&format!("VERTEX_POINT({},{p})", string(name)))?;
        self.vertices.insert(v, r);
        Ok(r)
    }

    /// `EDGE_CURVE(name, start, end, curve, .T.)`: the edge runs from `c(t0)` to `c(t1)`
    /// with `t0 < t1`, the direction in which the STEP curve's parameter grows.
    fn edge(&mut self, out: &mut Out, e: EdgeId) -> Result<Ref, IoError> {
        if let Some(r) = self.edges.get(&e) {
            return Ok(*r);
        }
        let k = |err| kernel(self.name, err);
        let [v0, v1] = self.body.edge_vertices(e).map_err(k)?;
        let start = self.vertex(out, v0)?;
        let end = self.vertex(out, v1)?;
        let (c, _, _) = self
            .body
            .edge_curve(e)
            .map_err(k)?
            .ok_or_else(|| kernel(self.name, format!("{e} has no curve")))?;
        let curve = curve(out, &c, self.name)?;
        let name = self.body.edge_name(e).map_err(k)?.unwrap_or("");
        let r = out.add(&format!(
            "EDGE_CURVE({},{start},{end},{curve},.T.)",
            string(name)
        ))?;
        self.edges.insert(e, r);
        Ok(r)
    }
}

/// `LINE(name, pnt, VECTOR(name, orientation, magnitude))`: the point at parameter `u`
/// is `pnt + u magnitude orientation`.
fn line(out: &mut Out, origin: Point3, direction: Vec3, magnitude: f64) -> Result<Ref, IoError> {
    let p = out.point(origin)?;
    let d = out.direction(direction)?;
    let v = out.add(&format!("VECTOR('',{d},{})", real(magnitude)?))?;
    out.add(&format!("LINE('',{p},{v})"))
}

/// The STEP curve under an edge. Circles and ellipses keep their frame: in STEP as here,
/// the parameter is the angle from the placement's x axis counter-clockwise about its z
/// axis.
fn curve(out: &mut Out, c: &Curve, body: &str) -> Result<Ref, IoError> {
    match c {
        Curve::Line(l) => line(out, l.origin(), l.direction(), 1.0),
        Curve::Segment(s) => {
            let d = s.end() - s.start();
            let length = d.length();
            let unit = d
                .normalized()
                .ok_or_else(|| kernel(body, "a segment is shorter than the length tolerance"))?;
            line(out, s.start(), unit, length)
        }
        Curve::Circle(c) => circle(out, c.frame(), c.radius()),
        Curve::Arc(a) => circle(out, a.circle().frame(), a.circle().radius()),
        Curve::Ellipse(e) => ellipse(out, e.frame(), e.rx(), e.ry()),
        Curve::EllipticalArc(a) => {
            let e = a.ellipse();
            ellipse(out, e.frame(), e.rx(), e.ry())
        }
    }
}

fn circle(out: &mut Out, frame: &Frame, radius: f64) -> Result<Ref, IoError> {
    let a = out.axis(frame)?;
    out.add(&format!("CIRCLE('',{a},{})", real(radius)?))
}

/// `ELLIPSE(name, position, semi_axis_1, semi_axis_2)`, `semi_axis_1` along the
/// placement's x axis.
fn ellipse(out: &mut Out, frame: &Frame, rx: f64, ry: f64) -> Result<Ref, IoError> {
    let a = out.axis(frame)?;
    out.add(&format!("ELLIPSE('',{a},{},{})", real(rx)?, real(ry)?))
}

/// The STEP surface under a face.
///
/// Mapping of kernel surfaces (all elementary surfaces of ISO 10303-42 over an
/// `AXIS2_PLACEMENT_3D` built from the surface's frame with [`Out::axis`]: origin,
/// z = axis, x = ref_direction):
///
/// | Kernel `Surface` | STEP entity |
/// |---|---|
/// | `Plane(frame)` | `PLANE(name, position)`, normal = position z |
/// | cylinder (K3) | `CYLINDRICAL_SURFACE(name, position, radius)`, axis = position z |
/// | cone (K3) | `CONICAL_SURFACE(name, position, radius, semi_angle)`: `radius` at the placement's origin plane, `semi_angle` in radians (the plane angle unit) |
/// | sphere (K3) | `SPHERICAL_SURFACE(name, position, radius)` |
/// | torus (K3) | `TOROIDAL_SURFACE(name, position, major_radius, minor_radius)` |
///
/// Each K3 surface's parameterisation and normal direction must be checked against
/// Part 42's definition (e.g. the cylinder's normal points away from the axis); when it
/// is the opposite of the kernel's, flip the face's `same_sense` flag rather than the
/// placement.
fn surface(out: &mut Out, s: &Surface, body: &str) -> Result<Ref, IoError> {
    #[allow(
        unreachable_patterns,
        reason = "K3 adds surfaces in parallel; until they are mapped here they are refused"
    )]
    match s {
        Surface::Plane(frame) => {
            let a = out.axis(frame)?;
            out.add(&format!("PLANE('',{a})"))
        }
        // ---- K3: add Cylinder, Cone, Sphere and Torus here (see the table above). ----
        _ => Err(kernel(
            body,
            format!("STEP export of the surface {s:?} is not supported yet"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_kernel::{Arc3, Circle3, Ellipse3, EllipticalArc3, Line3};

    fn out(limit: usize) -> Out {
        Out {
            text: String::new(),
            next: 1,
            limit,
        }
    }

    #[test]
    fn conics_keep_their_frame_and_radii() {
        // Expected text written by hand from the Part 42 definitions: centre, axis z,
        // ref_direction x, then the radius / semi-axes.
        let f = Frame::XY.with_origin(Point3::new(1.0, 2.0, 3.0)).unwrap();
        let placement = "#1=CARTESIAN_POINT('',(1.,2.,3.));\n#2=DIRECTION('',(0.,0.,1.));\n#3=DIRECTION('',(1.,0.,0.));\n#4=AXIS2_PLACEMENT_3D('',#1,#2,#3);\n";
        let cases = [
            (
                Curve::Circle(Circle3::new(f, 5.0).unwrap()),
                "#5=CIRCLE('',#4,5.);\n",
            ),
            (
                Curve::Arc(Arc3::new(f, 5.0, 0.5, 1.0).unwrap()),
                "#5=CIRCLE('',#4,5.);\n",
            ),
            (
                Curve::Ellipse(Ellipse3::new(f, 4.0, 2.5).unwrap()),
                "#5=ELLIPSE('',#4,4.,2.5);\n",
            ),
            (
                Curve::EllipticalArc(EllipticalArc3::new(f, 4.0, 2.5, 0.0, 1.0).unwrap()),
                "#5=ELLIPSE('',#4,4.,2.5);\n",
            ),
        ];
        for (c, last) in cases {
            let mut o = out(100);
            curve(&mut o, &c, "t").unwrap();
            assert_eq!(o.text, format!("{placement}{last}"));
        }
        let mut o = out(100);
        let l = Line3::new(Point3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
        curve(&mut o, &Curve::Line(l), "t").unwrap();
        assert_eq!(
            o.text,
            "#1=CARTESIAN_POINT('',(1.,2.,3.));\n#2=DIRECTION('',(0.,0.,-1.));\n#3=VECTOR('',#2,1.);\n#4=LINE('',#1,#3);\n"
        );
    }

    #[test]
    fn the_entity_budget_is_enforced() {
        let mut o = out(2);
        assert!(o.add("A()").is_ok());
        assert!(o.add("B()").is_ok());
        assert!(o.add("C()").is_err());
    }
}
