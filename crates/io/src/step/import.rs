//! STEP import: the advanced B-rep solids of an AP203/AP214/AP242 file as kernel bodies.
//!
//! Written from the public descriptions of ISO 10303-21 (the exchange structure),
//! ISO 10303-42 (geometric and topological resources) and the CAx-IF recommended
//! practices (units, representation contexts).
//!
//! # What is read
//! Every `MANIFOLD_SOLID_BREP` of the file becomes one body, named after the solid. Its
//! `CLOSED_SHELL` of `ADVANCED_FACE`s (or `FACE_SURFACE`s) maps to the kernel as the
//! writer's table in `super` describes, read backwards:
//!
//! | STEP | Kernel |
//! |---|---|
//! | `VERTEX_POINT` over a `CARTESIAN_POINT` | vertex |
//! | `EDGE_CURVE(name, start, end, curve, same_sense)` | edge running along the curve: from `start` to `end` when `same_sense`, else from `end` to `start` |
//! | `LINE` | a segment between the edge's vertices |
//! | `CIRCLE`, `ELLIPSE` | a circle or ellipse for an edge that starts and ends at one vertex, else an arc of it |
//! | `SURFACE_CURVE`, `SEAM_CURVE`, `INTERSECTION_CURVE`, `TRIMMED_CURVE` | their 3D (basis) curve; the edge's vertices bound it |
//! | `ORIENTED_EDGE(…, edge, orientation)` | a half-edge, along the edge when `orientation`; nested oriented edges compose |
//! | `EDGE_LOOP` in a `FACE_OUTER_BOUND` / `FACE_BOUND` | a loop, reversed when the bound's orientation is `.F.`; the outer loop is the one running counter-clockwise about the outward normal (`FACE_OUTER_BOUND` is tried first) |
//! | `VERTEX_LOOP` at a vertex the face's edges already reach (a pole) | dropped |
//! | `PLANE`, `CYLINDRICAL_SURFACE`, `CONICAL_SURFACE`, `SPHERICAL_SURFACE`, `TOROIDAL_SURFACE` | the kernel surface on the same `AXIS2_PLACEMENT_3D` frame, `same_sense` kept |
//! | `DEGENERATE_TOROIDAL_SURFACE` | a spindle torus (`select_outer` `.T.`), or the lemon torus of major radius `-R` with `same_sense` flipped (`.F.`), as `write::StepSurface` derives |
//! | `RECTANGULAR_TRIMMED_SURFACE` | its basis surface (the bounds trim it), `same_sense` flipped when exactly one of `u_sense`, `v_sense` is `.F.` |
//!
//! Complex instances are read by joining their partial records in the order of the
//! Part 42 supertype chain (`REPRESENTATION_ITEM` first, then the geometric or
//! topological item, ..., the leaf last), so `(… PLANE() REPRESENTATION_ITEM('') …)` reads
//! as `PLANE('', position)`.
//!
//! # Units
//! The solid's representation context (the representation that lists it, else the only
//! context of the file) gives the length and plane-angle units: `SI_UNIT` with any
//! prefix, or a `CONVERSION_BASED_UNIT` (inch, foot, degree, ...) through its
//! `…MEASURE_WITH_UNIT`, nested at most a few levels. Lengths are converted to
//! millimetres and angles (a cone's semi-angle) to radians. Without a unit the file is
//! read as millimetres and radians.
//!
//! # What is refused
//! A solid is imported only when it is complete and passes [`Body::validate`]; otherwise
//! it is reported as a [`SolidFailure`] naming the entities at fault, and the other
//! solids still import. Not supported yet ([`FailureKind::Unsupported`]): B-spline,
//! swept, offset and other free-form surfaces and curves (they come with K10), voids
//! (`BREP_WITH_VOIDS`), faceted B-reps (`POLY_LOOP`), faces without edges and
//! `ORIENTED_CLOSED_SHELL`s. Placements of assemblies (`MAPPED_ITEM`, representation
//! relationships with a transformation) are not applied: each solid comes in its own
//! coordinates. Files that break Part 42 (open shells, edges used other than twice,
//! vertices off their curves or surfaces by more than
//! [`chamfera_geom::LINEAR_TOLERANCE`]) give [`FailureKind::Invalid`].
//!
//! The file is hostile input: on top of the parser's limits, every reference is checked
//! for its entity type, nesting is bounded, and nothing panics.

use super::MAX_STEP_BYTES;
use super::read::{self, Exchange, Param};
use crate::IoError;
use chamfera_geom::{Frame, Point3, Vec3};
use chamfera_kernel::{
    Arc3, Body, Circle3, Curve, Ellipse3, EllipticalArc3, MAX_NAME_LENGTH, Segment3, Surface,
    TopologyError,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::f64::consts::TAU;
use std::fmt;

/// Deepest nesting of wrapped curves, oriented edges and unit conversions.
const MAX_NESTING: usize = 8;

/// Why a solid was not imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// It uses entities the kernel cannot hold yet (B-splines, voids, ...).
    Unsupported,
    /// It breaks the STEP rules or is not a valid solid.
    Invalid,
}

/// A solid of the file that was not imported, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct SolidFailure {
    /// The solid's instance number (`#n`).
    pub entity: u64,
    pub name: String,
    pub kind: FailureKind,
    /// One line per problem, each naming its entity (`face #34 lies on ...`).
    pub reasons: Vec<String>,
}

impl fmt::Display for SolidFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            FailureKind::Unsupported => "uses what is not supported yet",
            FailureKind::Invalid => "is not a valid solid",
        };
        write!(
            f,
            "solid #{} {:?} {what}: {}",
            self.entity,
            self.name,
            self.reasons.join("; ")
        )
    }
}

/// The result of importing a file: the complete solids, and the ones left out.
#[derive(Debug, Clone, Default)]
pub struct StepImport {
    /// Each solid's name and body, in file order. Every body passed [`Body::validate`].
    pub solids: Vec<(String, Body)>,
    pub failed: Vec<SolidFailure>,
}

fn err(msg: impl Into<String>) -> IoError {
    IoError::Format(format!("STEP: {}", msg.into()))
}

/// Imports every B-rep solid of a Part 21 text. Fails only when the text is not a Part 21
/// file or holds no solid at all; solids that cannot be imported are listed in
/// [`StepImport::failed`].
pub fn import(text: &str) -> Result<StepImport, IoError> {
    let ex = read::parse(text)?;
    import_exchange(&ex)
}

/// [`import`] for the bytes of a file. Bytes that are not UTF-8 (Latin-1 names from
/// older writers) become replacement characters; they can only stand in strings and
/// comments.
pub fn import_bytes(bytes: &[u8]) -> Result<StepImport, IoError> {
    if bytes.len() > MAX_STEP_BYTES {
        return Err(err(format!(
            "the file is larger than {MAX_STEP_BYTES} bytes"
        )));
    }
    import(&String::from_utf8_lossy(bytes))
}

/// [`import`] for a file, read through [`crate::read_limited`].
pub fn import_file(path: &std::path::Path) -> Result<StepImport, IoError> {
    let bytes = crate::read_limited(path, MAX_STEP_BYTES as u64)?;
    import_bytes(&bytes)
}

/// The solids of a STEP file's bytes, when every one of them imports; otherwise an
/// error that lists each solid left out and why.
pub fn read_bytes(bytes: &[u8]) -> Result<Vec<(String, Body)>, IoError> {
    all_or_nothing(import_bytes(bytes)?)
}

/// The solids of a STEP file, when every one of them imports (see [`read_bytes`]).
pub fn read_file(path: &std::path::Path) -> Result<Vec<(String, Body)>, IoError> {
    all_or_nothing(import_file(path)?)
}

fn all_or_nothing(r: StepImport) -> Result<Vec<(String, Body)>, IoError> {
    if r.failed.is_empty() {
        return Ok(r.solids);
    }
    let lines: Vec<String> = r.failed.iter().map(ToString::to_string).collect();
    Err(err(format!(
        "{} of {} solids cannot be imported: {}",
        r.failed.len(),
        r.failed.len() + r.solids.len(),
        lines.join(" | ")
    )))
}

/// Imports the solids of a parsed file (see [`import`]).
pub fn import_exchange(ex: &Exchange) -> Result<StepImport, IoError> {
    let contexts = item_contexts(ex);
    let unit_contexts: Vec<u64> = ex
        .data
        .iter()
        .filter(|(_, i)| i.record("GLOBAL_UNIT_ASSIGNED_CONTEXT").is_some())
        .map(|(id, _)| *id)
        .collect();
    // A solid no representation lists takes the file's only unit context, if any.
    let fallback = match unit_contexts.as_slice() {
        [only] => Some(*only),
        _ => None,
    };
    let view = View { ex };
    let mut out = StepImport::default();
    let mut found = 0usize;
    for (&id, inst) in &ex.data {
        let kind = ["MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS", "FACETED_BREP"]
            .into_iter()
            .rev()
            .find(|n| inst.record(n).is_some());
        let Some(kind) = kind else {
            continue;
        };
        found += 1;
        let name = view
            .ent(id)
            .ok()
            .and_then(|e| e.params.first().and_then(text_of))
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("Solid {found}"));
        let result = match kind {
            "BREP_WITH_VOIDS" => Err(Fail::Unsupported(vec![format!(
                "#{id} is a BREP_WITH_VOIDS: solids with inner voids are not supported yet"
            )])),
            "FACETED_BREP" => Err(Fail::Unsupported(vec![format!(
                "#{id} is a FACETED_BREP: faceted solids (POLY_LOOP bounds) are not supported yet"
            )])),
            _ => units_for(ex, contexts.get(&id).copied().or(fallback))
                .map_err(Fail::Invalid)
                .and_then(|units| Build::new(Ctx { view, units }).solid(id)),
        };
        let (kind, reasons) = match result {
            Ok(body) => {
                out.solids.push((name, body));
                continue;
            }
            Err(Fail::Unsupported(reasons)) => (FailureKind::Unsupported, reasons),
            Err(Fail::Invalid(reason)) => (FailureKind::Invalid, vec![reason]),
        };
        out.failed.push(SolidFailure {
            entity: id,
            name,
            kind,
            reasons,
        });
    }
    if found == 0 {
        let surfaces = ex
            .data
            .values()
            .any(|i| i.record("SHELL_BASED_SURFACE_MODEL").is_some());
        return Err(err(if surfaces {
            "the file holds surface models but no B-rep solid (MANIFOLD_SOLID_BREP); surface models are not imported"
        } else {
            "the file holds no B-rep solid (MANIFOLD_SOLID_BREP)"
        }));
    }
    Ok(out)
}

// --- Failures ---------------------------------------------------------------------------

/// Why one solid stops: unsupported entities (all of them, collected), or the first
/// invalid one.
enum Fail {
    Unsupported(Vec<String>),
    Invalid(String),
}

type R<T> = Result<T, Fail>;

fn invalid(msg: impl Into<String>) -> Fail {
    Fail::Invalid(msg.into())
}

fn unsupported(msg: impl Into<String>) -> Fail {
    Fail::Unsupported(vec![msg.into()])
}

/// Why an entity is not supported, with the rung that brings it for B-splines.
fn not_supported(what: &str, id: u64, name: &str) -> Fail {
    let later = if name.contains("B_SPLINE") || name.contains("BEZIER") {
        " (B-splines come with kernel rung K10)"
    } else {
        ""
    };
    unsupported(format!(
        "{what} is #{id}, a {name}, which is not supported yet{later}"
    ))
}

// --- Entities -----------------------------------------------------------------------------

/// The Part 42 depth of the entities whose complex instances we read: a supertype
/// before its subtypes, so the partial records join into the leaf's attribute order.
const RANKS: &[(&str, u8)] = &[
    ("REPRESENTATION_ITEM", 0),
    ("GEOMETRIC_REPRESENTATION_ITEM", 1),
    ("TOPOLOGICAL_REPRESENTATION_ITEM", 1),
    ("POINT", 2),
    ("DIRECTION", 2),
    ("VECTOR", 2),
    ("PLACEMENT", 2),
    ("CURVE", 2),
    ("SURFACE", 2),
    ("VERTEX", 2),
    ("EDGE", 2),
    ("LOOP", 2),
    ("PATH", 2),
    ("FACE", 2),
    ("FACE_BOUND", 2),
    ("CONNECTED_FACE_SET", 2),
    ("SOLID_MODEL", 2),
    ("CARTESIAN_POINT", 3),
    ("AXIS2_PLACEMENT_3D", 3),
    ("LINE", 3),
    ("CONIC", 3),
    ("BOUNDED_CURVE", 3),
    ("SURFACE_CURVE", 3),
    ("ELEMENTARY_SURFACE", 3),
    ("BOUNDED_SURFACE", 3),
    ("VERTEX_POINT", 3),
    ("EDGE_CURVE", 3),
    ("ORIENTED_EDGE", 3),
    ("EDGE_LOOP", 3),
    ("VERTEX_LOOP", 3),
    ("FACE_SURFACE", 3),
    ("FACE_OUTER_BOUND", 3),
    ("CLOSED_SHELL", 3),
    ("MANIFOLD_SOLID_BREP", 3),
    ("CIRCLE", 4),
    ("ELLIPSE", 4),
    ("TRIMMED_CURVE", 4),
    ("SEAM_CURVE", 4),
    ("INTERSECTION_CURVE", 4),
    ("PLANE", 4),
    ("CYLINDRICAL_SURFACE", 4),
    ("CONICAL_SURFACE", 4),
    ("SPHERICAL_SURFACE", 4),
    ("TOROIDAL_SURFACE", 4),
    ("RECTANGULAR_TRIMMED_SURFACE", 4),
    ("ADVANCED_FACE", 4),
    ("DEGENERATE_TOROIDAL_SURFACE", 5),
];

fn rank(name: &str) -> Option<u8> {
    RANKS.iter().find(|(n, _)| *n == name).map(|(_, r)| *r)
}

/// An instance read as one entity: its leaf name and attributes in order.
struct Ent<'a> {
    name: Cow<'a, str>,
    params: Cow<'a, [Param]>,
}

#[derive(Clone, Copy)]
struct View<'a> {
    ex: &'a Exchange,
}

impl<'a> View<'a> {
    /// `#id` as one entity. A complex instance whose records are all in [`RANKS`] reads
    /// as its deepest record with the attributes joined in supertype order; any other
    /// complex instance gets its record names joined by `+` (which no reader accepts,
    /// so it is reported by name).
    fn ent(&self, id: u64) -> R<Ent<'a>> {
        let inst = self
            .ex
            .data
            .get(&id)
            .ok_or_else(|| invalid(format!("#{id} is not in the file")))?;
        match inst.records.as_slice() {
            [r] => Ok(Ent {
                name: Cow::Borrowed(&r.name),
                params: Cow::Borrowed(&r.params),
            }),
            rs => {
                let ranks: Option<Vec<u8>> = rs.iter().map(|r| rank(&r.name)).collect();
                let Some(ranks) = ranks else {
                    let names: Vec<&str> = rs.iter().map(|r| r.name.as_str()).collect();
                    return Ok(Ent {
                        name: Cow::Owned(names.join("+")),
                        params: Cow::Owned(Vec::new()),
                    });
                };
                let mut order: Vec<usize> = (0..rs.len()).collect();
                order.sort_by_key(|&i| ranks.get(i).copied().unwrap_or(u8::MAX));
                let leaf = order.last().and_then(|&i| rs.get(i));
                let params: Vec<Param> = order
                    .iter()
                    .filter_map(|&i| rs.get(i))
                    .flat_map(|r| r.params.iter().cloned())
                    .collect();
                Ok(Ent {
                    name: Cow::Borrowed(leaf.map_or("", |r| r.name.as_str())),
                    params: Cow::Owned(params),
                })
            }
        }
    }
}

fn arg(e: &Ent<'_>, i: usize, id: u64) -> R<Param> {
    e.params
        .get(i)
        .cloned()
        .ok_or_else(|| invalid(format!("#{id} ({}) has too few attributes", e.name)))
}

fn ref_of(p: &Param, id: u64, what: &str) -> R<u64> {
    match p {
        Param::Ref(r) => Ok(*r),
        _ => Err(invalid(format!("#{id}: its {what} must be a reference"))),
    }
}

fn real_of(p: &Param, id: u64, what: &str) -> R<f64> {
    let v = match p {
        Param::Real(v) => *v,
        Param::Integer(i) => *i as f64,
        Param::Typed(_, inner) => match inner.as_slice() {
            [Param::Real(v)] => *v,
            [Param::Integer(i)] => *i as f64,
            _ => return Err(invalid(format!("#{id}: its {what} must be a number"))),
        },
        _ => return Err(invalid(format!("#{id}: its {what} must be a number"))),
    };
    if v.is_finite() {
        Ok(v)
    } else {
        Err(invalid(format!("#{id}: its {what} must be finite")))
    }
}

fn bool_of(p: &Param, id: u64, what: &str) -> R<bool> {
    match p {
        Param::Enum(e) if e == "T" => Ok(true),
        Param::Enum(e) if e == "F" => Ok(false),
        _ => Err(invalid(format!("#{id}: its {what} must be .T. or .F."))),
    }
}

fn list_of(p: &Param, id: u64, what: &str) -> R<Vec<Param>> {
    match p {
        Param::List(l) => Ok(l.clone()),
        _ => Err(invalid(format!("#{id}: its {what} must be a list"))),
    }
}

fn text_of(p: &Param) -> Option<String> {
    match p {
        Param::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// A name cut to the kernel's limit on a character boundary.
fn kernel_name(s: &str) -> Option<String> {
    if s.trim().is_empty() {
        return None;
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.len() + c.len_utf8() > MAX_NAME_LENGTH {
            break;
        }
        out.push(c);
    }
    Some(out)
}

// --- Units --------------------------------------------------------------------------------

/// Millimetres per length unit and radians per plane-angle unit of a context.
#[derive(Debug, Clone, Copy)]
struct Units {
    length: f64,
    angle: f64,
}

const MM_RAD: Units = Units {
    length: 1.0,
    angle: 1.0,
};

/// Each representation item's context: `(…REPRESENTATION(name, items, context))`.
fn item_contexts(ex: &Exchange) -> HashMap<u64, u64> {
    let mut out = HashMap::new();
    for inst in ex.data.values() {
        for r in &inst.records {
            if !r.name.ends_with("REPRESENTATION") {
                continue;
            }
            if let [_, Param::List(items), Param::Ref(ctx), ..] = r.params.as_slice() {
                for item in items {
                    if let Param::Ref(i) = item {
                        out.entry(*i).or_insert(*ctx);
                    }
                }
            }
        }
    }
    out
}

/// The units of representation context `ctx` (millimetres and radians without one).
fn units_for(ex: &Exchange, ctx: Option<u64>) -> Result<Units, String> {
    let Some(ctx) = ctx else {
        return Ok(MM_RAD);
    };
    let Some(units) = ex
        .data
        .get(&ctx)
        .and_then(|i| i.record("GLOBAL_UNIT_ASSIGNED_CONTEXT"))
    else {
        return Ok(MM_RAD);
    };
    let mut out = MM_RAD;
    let list = match units.params.first() {
        Some(Param::List(l)) => l.as_slice(),
        _ => return Err(format!("the unit list of context #{ctx} is not a list")),
    };
    for u in list {
        let Param::Ref(u) = u else {
            return Err(format!(
                "context #{ctx} lists a unit that is not a reference"
            ));
        };
        let Some(inst) = ex.data.get(u) else {
            continue;
        };
        if inst.record("LENGTH_UNIT").is_some() {
            out.length = unit_scale(ex, *u, UnitKind::Length, 0)?;
        } else if inst.record("PLANE_ANGLE_UNIT").is_some() {
            out.angle = unit_scale(ex, *u, UnitKind::Angle, 0)?;
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitKind {
    Length,
    Angle,
}

/// The size of unit `#id` in millimetres (lengths) or radians (angles).
fn unit_scale(ex: &Exchange, id: u64, kind: UnitKind, depth: usize) -> Result<f64, String> {
    if depth > MAX_NESTING {
        return Err(format!("unit #{id}: conversions are nested too deeply"));
    }
    let inst = ex
        .data
        .get(&id)
        .ok_or_else(|| format!("unit #{id} is not in the file"))?;
    let value = if let Some(si) = inst.record("SI_UNIT") {
        let prefix = match si.params.first() {
            Some(Param::Enum(p)) => {
                si_prefix(p).ok_or_else(|| format!("unit #{id} has the unknown SI prefix .{p}."))?
            }
            _ => 1.0,
        };
        let (want, base) = match kind {
            UnitKind::Length => ("METRE", 1000.0),
            UnitKind::Angle => ("RADIAN", 1.0),
        };
        match si.params.get(1) {
            Some(Param::Enum(n)) if n == want => prefix * base,
            _ => return Err(format!("unit #{id} should be an SI_UNIT of .{want}.")),
        }
    } else if let Some(cb) = inst.record("CONVERSION_BASED_UNIT") {
        let Some(Param::Ref(m)) = cb.params.get(1) else {
            return Err(format!("unit #{id} has no conversion factor"));
        };
        let measure = ex
            .data
            .get(m)
            .and_then(|i| {
                i.records
                    .iter()
                    .find(|r| r.name.ends_with("MEASURE_WITH_UNIT") && r.params.len() >= 2)
            })
            .ok_or_else(|| format!("unit #{id}: #{m} should be a MEASURE_WITH_UNIT"))?;
        let v = measure
            .params
            .first()
            .and_then(|p| real_of(p, *m, "value").ok())
            .ok_or_else(|| format!("unit #{id}: #{m} has no numeric value"))?;
        let Some(Param::Ref(base)) = measure.params.get(1) else {
            return Err(format!("unit #{id}: #{m} has no unit"));
        };
        v * unit_scale(ex, *base, kind, depth + 1)?
    } else {
        return Err(format!(
            "unit #{id} is neither an SI_UNIT nor a CONVERSION_BASED_UNIT"
        ));
    };
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(format!(
            "unit #{id} has a size that is not a positive number"
        ))
    }
}

/// The SI prefixes of ISO 10303-41.
fn si_prefix(p: &str) -> Option<f64> {
    Some(match p {
        "EXA" => 1e18,
        "PETA" => 1e15,
        "TERA" => 1e12,
        "GIGA" => 1e9,
        "MEGA" => 1e6,
        "KILO" => 1e3,
        "HECTO" => 1e2,
        "DECA" => 1e1,
        "DECI" => 1e-1,
        "CENTI" => 1e-2,
        "MILLI" => 1e-3,
        "MICRO" => 1e-6,
        "NANO" => 1e-9,
        "PICO" => 1e-12,
        "FEMTO" => 1e-15,
        "ATTO" => 1e-18,
        _ => return None,
    })
}

// --- Geometry -----------------------------------------------------------------------------

struct Ctx<'a> {
    view: View<'a>,
    units: Units,
}

/// A curve under an edge, before the edge's vertices bound it.
enum StepCurve {
    Line,
    Circle(Frame, f64),
    Ellipse(Frame, f64, f64),
}

impl Ctx<'_> {
    fn ent(&self, id: u64) -> R<Ent<'_>> {
        self.view.ent(id)
    }

    fn expect<'s>(&'s self, id: u64, names: &[&str], what: &str) -> R<Ent<'s>> {
        let e = self.ent(id)?;
        if names.contains(&e.name.as_ref()) {
            Ok(e)
        } else {
            Err(invalid(format!(
                "{what} #{id} is a {}, expected {}",
                e.name,
                names.join(" or ")
            )))
        }
    }

    fn triple(&self, e: &Ent<'_>, id: u64, what: &str) -> R<[f64; 3]> {
        match list_of(&arg(e, 1, id)?, id, what)?.as_slice() {
            [x, y, z] => Ok([
                real_of(x, id, what)?,
                real_of(y, id, what)?,
                real_of(z, id, what)?,
            ]),
            _ => Err(invalid(format!("#{id}: a 3D model needs three {what}"))),
        }
    }

    /// A `CARTESIAN_POINT`, in millimetres.
    fn point(&self, id: u64) -> R<Point3> {
        let e = self.expect(id, &["CARTESIAN_POINT"], "point")?;
        let [x, y, z] = self.triple(&e, id, "coordinates")?;
        let s = self.units.length;
        let p = Point3::new(x * s, y * s, z * s);
        if p.is_valid() {
            Ok(p)
        } else {
            Err(invalid(format!(
                "point #{id} lies outside the working range of ±{} mm",
                chamfera_geom::MAX_COORDINATE
            )))
        }
    }

    /// A `DIRECTION` (not normalised).
    fn direction(&self, id: u64) -> R<Vec3> {
        let e = self.expect(id, &["DIRECTION"], "direction")?;
        let [x, y, z] = self.triple(&e, id, "direction ratios")?;
        let v = Vec3::new(x, y, z);
        if v.normalized().is_some() {
            Ok(v)
        } else {
            Err(invalid(format!("direction #{id} has no length")))
        }
    }

    /// An `AXIS2_PLACEMENT_3D` as a frame: origin, z = `axis` (default +z), x = the part
    /// of `ref_direction` across the axis (default: +x, or +z when the axis is along x,
    /// as Part 42's `first_proj_axis`).
    fn placement(&self, id: u64) -> R<Frame> {
        let e = self.expect(id, &["AXIS2_PLACEMENT_3D"], "placement")?;
        let origin = self.point(ref_of(&arg(&e, 1, id)?, id, "location")?)?;
        let z = match e.params.get(2) {
            Some(Param::Ref(r)) => self.direction(*r)?,
            _ => Vec3::Z,
        };
        let x = match e.params.get(3) {
            Some(Param::Ref(r)) => self.direction(*r)?,
            _ => {
                let along_x = z
                    .normalized()
                    .is_some_and(|u| (u.dot(Vec3::X).abs() - 1.0).abs() < 1e-12);
                if along_x { Vec3::Z } else { Vec3::X }
            }
        };
        Frame::from_normal_and_x(origin, z, x).ok_or_else(|| {
            invalid(format!(
                "placement #{id}: its ref_direction is parallel to its axis"
            ))
        })
    }

    fn length(&self, p: &Param, id: u64, what: &str) -> R<f64> {
        let v = real_of(p, id, what)? * self.units.length;
        if v.is_finite() && v > 0.0 {
            Ok(v)
        } else {
            Err(invalid(format!("#{id}: its {what} must be positive")))
        }
    }

    /// The curve of an edge (`what` names the edge for messages).
    fn curve(&self, id: u64, what: &str, depth: usize) -> R<StepCurve> {
        if depth > MAX_NESTING {
            return Err(invalid(format!("{what}: curves are nested too deeply")));
        }
        let e = self.ent(id)?;
        match e.name.as_ref() {
            "LINE" => {
                // The edge's vertices bound the line; its point and direction are only
                // checked.
                self.point(ref_of(&arg(&e, 1, id)?, id, "pnt")?)?;
                let v = ref_of(&arg(&e, 2, id)?, id, "dir")?;
                let ve = self.expect(v, &["VECTOR"], "line direction")?;
                self.direction(ref_of(&arg(&ve, 1, v)?, v, "orientation")?)?;
                Ok(StepCurve::Line)
            }
            "CIRCLE" => {
                let frame = self.placement(ref_of(&arg(&e, 1, id)?, id, "position")?)?;
                Ok(StepCurve::Circle(
                    frame,
                    self.length(&arg(&e, 2, id)?, id, "radius")?,
                ))
            }
            "ELLIPSE" => {
                let frame = self.placement(ref_of(&arg(&e, 1, id)?, id, "position")?)?;
                Ok(StepCurve::Ellipse(
                    frame,
                    self.length(&arg(&e, 2, id)?, id, "semi_axis_1")?,
                    self.length(&arg(&e, 3, id)?, id, "semi_axis_2")?,
                ))
            }
            "SURFACE_CURVE" | "SEAM_CURVE" | "INTERSECTION_CURVE" | "TRIMMED_CURVE" => {
                self.curve(ref_of(&arg(&e, 1, id)?, id, "3D curve")?, what, depth + 1)
            }
            other => Err(not_supported(&format!("the curve of {what}"), id, other)),
        }
    }

    /// The surface of a face, and whether Part 42's normal is against the kernel's.
    fn surface(&self, id: u64, what: &str, depth: usize) -> R<(Surface, bool)> {
        if depth > MAX_NESTING {
            return Err(invalid(format!("{what}: surfaces are nested too deeply")));
        }
        let e = self.ent(id)?;
        let kernel = |r: chamfera_kernel::KernelResult<Surface>| {
            r.map_err(|k| invalid(format!("the surface #{id} of {what}: {k}")))
        };
        let frame = || self.placement(ref_of(&arg(&e, 1, id)?, id, "position")?);
        let len = |i: usize, name: &str| self.length(&arg(&e, i, id)?, id, name);
        Ok(match e.name.as_ref() {
            "PLANE" => (Surface::Plane(frame()?), false),
            "CYLINDRICAL_SURFACE" => (
                kernel(Surface::cylinder(frame()?, len(2, "radius")?))?,
                false,
            ),
            "CONICAL_SURFACE" => {
                // radius may be zero (apex at the placement); the semi-angle is an angle.
                let r = real_of(&arg(&e, 2, id)?, id, "radius")? * self.units.length;
                let a = real_of(&arg(&e, 3, id)?, id, "semi_angle")? * self.units.angle;
                (kernel(Surface::cone(frame()?, r, a))?, false)
            }
            "SPHERICAL_SURFACE" => (kernel(Surface::sphere(frame()?, len(2, "radius")?))?, false),
            "TOROIDAL_SURFACE" => (
                kernel(Surface::torus(
                    frame()?,
                    len(2, "major_radius")?,
                    len(3, "minor_radius")?,
                ))?,
                false,
            ),
            "DEGENERATE_TOROIDAL_SURFACE" => {
                let (big, a) = (len(2, "major_radius")?, len(3, "minor_radius")?);
                if bool_of(&arg(&e, 4, id)?, id, "select_outer")? {
                    (kernel(Surface::torus(frame()?, big, a))?, false)
                } else {
                    (kernel(Surface::torus(frame()?, -big, a))?, true)
                }
            }
            "RECTANGULAR_TRIMMED_SURFACE" => {
                let basis = ref_of(&arg(&e, 1, id)?, id, "basis_surface")?;
                let (s, flip) = self.surface(basis, what, depth + 1)?;
                let u = bool_of(&arg(&e, 6, id)?, id, "usense")?;
                let v = bool_of(&arg(&e, 7, id)?, id, "vsense")?;
                (s, flip != (u != v))
            }
            other => return Err(not_supported(&format!("the surface of {what}"), id, other)),
        })
    }
}

/// The angle of `p` about a conic's frame, in `[0, 2π)`, with the ellipse's semi-axes
/// (`rx`, `ry`; equal for a circle): `p = o + rx cos t x + ry sin t y`.
fn conic_angle(frame: &Frame, rx: f64, ry: f64, p: Point3) -> f64 {
    let l = frame.to_local(p);
    let t = (l.y / ry).atan2(l.x / rx);
    let t = if t < 0.0 { t + TAU } else { t };
    if t >= TAU { 0.0 } else { t }
}

// --- One solid ----------------------------------------------------------------------------

/// A loop as kernel edge uses `(edge, forward)`.
type LoopUses = Vec<(usize, bool)>;
/// A face of [`Body::from_boundary`]'s description: surface, `same_sense`, loops.
type BoundaryFace = (Surface, bool, Vec<LoopUses>);

/// The boundary description of one solid as it is read, with the STEP instance behind
/// each kernel index (for error messages and names).
struct Build<'a> {
    ctx: Ctx<'a>,
    points: Vec<Point3>,
    vertex_ids: Vec<u64>,
    vertex_names: Vec<Option<String>>,
    vertex_of: HashMap<u64, usize>,
    edges: Vec<(usize, usize, Curve, f64, f64)>,
    edge_ids: Vec<u64>,
    edge_names: Vec<Option<String>>,
    /// STEP edge -> (kernel edge, `same_sense`).
    edge_of: HashMap<u64, (usize, bool)>,
    faces: Vec<BoundaryFace>,
    face_ids: Vec<u64>,
    face_names: Vec<Option<String>>,
    /// Per kernel loop: (face, bound) instances.
    loop_ids: Vec<(u64, u64)>,
    unsupported: Vec<String>,
}

impl<'a> Build<'a> {
    fn new(ctx: Ctx<'a>) -> Self {
        Build {
            ctx,
            points: Vec::new(),
            vertex_ids: Vec::new(),
            vertex_names: Vec::new(),
            vertex_of: HashMap::new(),
            edges: Vec::new(),
            edge_ids: Vec::new(),
            edge_names: Vec::new(),
            edge_of: HashMap::new(),
            faces: Vec::new(),
            face_ids: Vec::new(),
            face_names: Vec::new(),
            loop_ids: Vec::new(),
            unsupported: Vec::new(),
        }
    }

    fn solid(&mut self, id: u64) -> R<Body> {
        let e = self.ctx.expect(id, &["MANIFOLD_SOLID_BREP"], "solid")?;
        let shell = ref_of(&arg(&e, 1, id)?, id, "outer shell")?;
        let se = self.ctx.ent(shell)?;
        if se.name != "CLOSED_SHELL" {
            return Err(not_supported("the shell of the solid", shell, &se.name));
        }
        let faces = list_of(&arg(&se, 1, shell)?, shell, "faces")?;
        drop(e);
        drop(se);
        for f in &faces {
            let f = ref_of(f, shell, "face")?;
            match self.face(f) {
                Ok(()) => {}
                Err(Fail::Unsupported(why)) => {
                    for w in why {
                        if !self.unsupported.contains(&w) {
                            self.unsupported.push(w);
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }
        if !self.unsupported.is_empty() {
            return Err(Fail::Unsupported(std::mem::take(&mut self.unsupported)));
        }
        let mut body = Body::from_boundary(&self.points, &self.edges, &self.faces)
            .map_err(|e| invalid(self.locate(&e)))?;
        for (f, name) in body.faces().into_iter().zip(&self.face_names) {
            if let Some(n) = name {
                let _ = body.set_face_name(f, n);
            }
        }
        for (e, name) in body.edges().into_iter().zip(&self.edge_names) {
            if let Some(n) = name {
                let _ = body.set_edge_name(e, n);
            }
        }
        for v in body.vertices() {
            if let Some(Some(n)) = self.vertex_names.get(v.index()) {
                let _ = body.set_vertex_name(v, n);
            }
        }
        Ok(body)
    }

    /// The STEP entity behind a kernel validation error, with the error.
    fn locate(&self, e: &TopologyError) -> String {
        use TopologyError as T;
        let face = |i: usize| self.face_ids.get(i).map(|id| format!("face #{id}"));
        let edge = |i: usize| self.edge_ids.get(i).map(|id| format!("edge #{id}"));
        let vertex = |i: usize| self.vertex_ids.get(i).map(|id| format!("vertex #{id}"));
        let at = match e {
            T::MissingSurface(f)
            | T::OffSurface(f)
            | T::VertexLoop(f)
            | T::WrongOrientation(f)
            | T::DegenerateFace(f) => face(f.index()),
            T::MissingTwin(x)
            | T::TwinsNotOpposite(x)
            | T::MissingCurve(x)
            | T::BadCurveRange(x, _)
            | T::VertexOffCurve(x)
            | T::DegenerateEdge(x)
            | T::WireEdge(x) => edge(x.index()),
            T::VertexFan(v) | T::InvalidPoint(v) => vertex(v.index()),
            T::OpenLoop(l) => self
                .loop_ids
                .get(l.index())
                .map(|(f, b)| format!("bound #{b} of face #{f}")),
            _ => None,
        };
        match at {
            Some(a) => format!("{a}: {e}"),
            None => e.to_string(),
        }
    }

    fn vertex(&mut self, id: u64) -> R<usize> {
        if let Some(&i) = self.vertex_of.get(&id) {
            return Ok(i);
        }
        let e = self.ctx.expect(id, &["VERTEX_POINT"], "vertex")?;
        let name = e.params.first().and_then(text_of);
        let p = self
            .ctx
            .point(ref_of(&arg(&e, 1, id)?, id, "vertex_geometry")?)?;
        let i = self.points.len();
        self.points.push(p);
        self.vertex_ids.push(id);
        self.vertex_names
            .push(name.as_deref().and_then(kernel_name));
        self.vertex_of.insert(id, i);
        Ok(i)
    }

    /// The kernel edge of an `EDGE_CURVE`, running along its curve, and its
    /// `same_sense`.
    fn edge(&mut self, id: u64) -> R<(usize, bool)> {
        if let Some(&x) = self.edge_of.get(&id) {
            return Ok(x);
        }
        let what = format!("edge #{id}");
        let e = self.ctx.expect(id, &["EDGE_CURVE"], "edge")?;
        let name = e.params.first().and_then(text_of);
        let start = ref_of(&arg(&e, 1, id)?, id, "edge_start")?;
        let end = ref_of(&arg(&e, 2, id)?, id, "edge_end")?;
        let curve_id = ref_of(&arg(&e, 3, id)?, id, "edge_geometry")?;
        let same_sense = bool_of(&arg(&e, 4, id)?, id, "same_sense")?;
        drop(e);
        let curve = self.ctx.curve(curve_id, &what, 0)?;
        let (vs, ve) = (self.vertex(start)?, self.vertex(end)?);
        let (from, to) = if same_sense { (vs, ve) } else { (ve, vs) };
        let pf = self.points.get(from).copied().unwrap_or(Point3::ORIGIN);
        let pt = self.points.get(to).copied().unwrap_or(Point3::ORIGIN);
        let closed = from == to;
        let k = |r: chamfera_kernel::KernelResult<Curve>| {
            r.map_err(|x| invalid(format!("{what}: {x}")))
        };
        let (c, t0, t1) = match curve {
            StepCurve::Line => {
                if closed {
                    return Err(invalid(format!(
                        "{what} is straight but starts and ends at one vertex"
                    )));
                }
                (k(Segment3::new(pf, pt).map(Curve::Segment))?, 0.0, 1.0)
            }
            StepCurve::Circle(frame, r) => {
                let a = conic_angle(&frame, r, r, pf);
                if closed {
                    (k(Circle3::new(frame, r).map(Curve::Circle))?, a, a + TAU)
                } else {
                    let sweep = (conic_angle(&frame, r, r, pt) - a).rem_euclid(TAU);
                    let arc = k(Arc3::new(frame, r, a, sweep).map(Curve::Arc))?;
                    let (lo, hi) = arc.range();
                    (arc, lo, hi)
                }
            }
            StepCurve::Ellipse(frame, rx, ry) => {
                let a = conic_angle(&frame, rx, ry, pf);
                if closed {
                    (
                        k(Ellipse3::new(frame, rx, ry).map(Curve::Ellipse))?,
                        a,
                        a + TAU,
                    )
                } else {
                    let sweep = (conic_angle(&frame, rx, ry, pt) - a).rem_euclid(TAU);
                    let arc =
                        k(EllipticalArc3::new(frame, rx, ry, a, sweep).map(Curve::EllipticalArc))?;
                    let (lo, hi) = arc.range();
                    (arc, lo, hi)
                }
            }
        };
        let i = self.edges.len();
        self.edges.push((from, to, c, t0, t1));
        self.edge_ids.push(id);
        self.edge_names.push(name.as_deref().and_then(kernel_name));
        self.edge_of.insert(id, (i, same_sense));
        Ok((i, same_sense))
    }

    /// The `EDGE_CURVE` under an `ORIENTED_EDGE` and whether the use runs from its
    /// `edge_start` to its `edge_end`.
    fn oriented(&self, id: u64, depth: usize) -> R<(u64, bool)> {
        if depth > MAX_NESTING {
            return Err(invalid(format!(
                "#{id}: oriented edges are nested too deeply"
            )));
        }
        let e = self.ctx.ent(id)?;
        match e.name.as_ref() {
            "ORIENTED_EDGE" => {
                let inner = ref_of(&arg(&e, 3, id)?, id, "edge_element")?;
                let along = bool_of(&arg(&e, 4, id)?, id, "orientation")?;
                let (edge, a) = self.oriented(inner, depth + 1)?;
                Ok((edge, a == along))
            }
            "EDGE_CURVE" if depth > 0 => Ok((id, true)),
            other => Err(invalid(format!(
                "#{id} is a {other}, expected an ORIENTED_EDGE (of an EDGE_CURVE)"
            ))),
        }
    }

    fn face(&mut self, id: u64) -> R<()> {
        let what = format!("face #{id}");
        let e = self
            .ctx
            .expect(id, &["ADVANCED_FACE", "FACE_SURFACE"], "face")
            .map_err(|f| match f {
                Fail::Invalid(_) => match self.ctx.ent(id) {
                    Ok(x) if x.name.contains("FACE") => not_supported("the face", id, &x.name),
                    _ => f,
                },
                other => other,
            })?;
        let name = e.params.first().and_then(text_of);
        let bounds = list_of(&arg(&e, 1, id)?, id, "bounds")?;
        let surface_id = ref_of(&arg(&e, 2, id)?, id, "face_geometry")?;
        let same_sense = bool_of(&arg(&e, 3, id)?, id, "same_sense")?;
        drop(e);
        let (surface, flip) = self.ctx.surface(surface_id, &what, 0)?;
        // (outer?, bound, uses) per edge loop; vertices of vertex loops.
        let mut loops: Vec<(bool, u64, LoopUses)> = Vec::new();
        let mut vertex_loops: Vec<u64> = Vec::new();
        for b in &bounds {
            let b = ref_of(b, id, "bound")?;
            let be = self
                .ctx
                .expect(b, &["FACE_OUTER_BOUND", "FACE_BOUND"], "bound")?;
            let outer = be.name == "FACE_OUTER_BOUND";
            let lp = ref_of(&arg(&be, 1, b)?, b, "bound")?;
            let orientation = bool_of(&arg(&be, 2, b)?, b, "orientation")?;
            drop(be);
            let le = self.ctx.ent(lp)?;
            match le.name.as_ref() {
                "EDGE_LOOP" => {}
                "VERTEX_LOOP" => {
                    let v = ref_of(&arg(&le, 1, lp)?, lp, "loop_vertex")?;
                    vertex_loops.push(v);
                    continue;
                }
                other => return Err(not_supported(&format!("a bound of {what}"), lp, other)),
            }
            let list = list_of(&arg(&le, 1, lp)?, lp, "edge_list")?;
            drop(le);
            if list.is_empty() {
                return Err(invalid(format!("the edge loop #{lp} of {what} is empty")));
            }
            let mut uses = Vec::with_capacity(list.len());
            for oe in &list {
                let oe = ref_of(oe, lp, "edge")?;
                let (edge, along) = self.oriented(oe, 0)?;
                let (k, same_sense) = self.edge(edge)?;
                uses.push((k, along == same_sense));
            }
            if !orientation {
                uses.reverse();
                for u in &mut uses {
                    u.1 = !u.1;
                }
            }
            loops.push((outer, b, uses));
        }
        // A vertex loop at a vertex the edge loops reach (a pole) adds nothing.
        for v in vertex_loops {
            let reached = self.vertex_of.get(&v).is_some_and(|&k| {
                loops
                    .iter()
                    .flat_map(|l| l.2.iter())
                    .any(|&(e, _)| self.edges.get(e).is_some_and(|x| x.0 == k || x.1 == k))
            });
            if !reached {
                return Err(unsupported(format!(
                    "{what} has a bound that is a lone vertex (VERTEX_LOOP #{v}), which is not supported yet"
                )));
            }
        }
        if loops.is_empty() {
            return Err(unsupported(format!(
                "{what} has no edge loop (a closed surface without edges), which is not supported yet"
            )));
        }
        // FACE_OUTER_BOUND first: it is tried first as the outer loop.
        loops.sort_by_key(|l| !l.0);
        for (_, b, _) in &loops {
            self.loop_ids.push((id, *b));
        }
        self.faces.push((
            surface,
            same_sense != flip,
            loops.into_iter().map(|l| l.2).collect(),
        ));
        self.face_ids.push(id);
        self.face_names.push(name.as_deref().and_then(kernel_name));
        Ok(())
    }
}
