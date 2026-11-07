//! 3MF import and export, from the public 3MF Core Specification (3MF Consortium).
//!
//! A 3MF file is an OPC package (a ZIP, `crate::zip`) whose model part holds meshes:
//! `<vertices>` and `<triangles>` inside `<object>`s, objects built from other objects
//! with `<components>`, and the `<build>` items to make. Export writes one object per body
//! with its colour (a `<basematerials>` group) in millimetres. Import reads each build item
//! as one mesh, with component and item transforms applied and the model's unit turned
//! into millimetres. Everything read is checked: counts, indices, coordinates, nesting.

use crate::IoError;
use crate::mesh::{MeshBody, hex_colour};
use crate::xml::{self, Tag};
use chamfera_geom::{MAX_TRIANGLES, Point3, TriMesh};
use std::collections::HashMap;
use std::fmt::Write as _;

/// Largest 3MF file accepted (bytes).
pub const MAX_3MF_BYTES: u64 = 1024 * 1024 * 1024;
/// Largest model part accepted, uncompressed (bytes).
pub const MAX_MODEL_BYTES: u64 = 1024 * 1024 * 1024;
/// Most build items (bodies) one file may give.
pub const MAX_ITEMS: usize = 1000;
/// Deepest nesting of components.
const MAX_DEPTH: usize = 16;
/// Most object instances one file may build, counting each use of a shared object.
const MAX_VISITS: usize = 100_000;

const MODEL_PATH: &str = "3D/3dmodel.model";
const CORE_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const MODEL_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

fn bad(why: impl std::fmt::Display) -> IoError {
    IoError::Format(format!("3MF: {why}"))
}

/// Writes a 3MF package with one object per body, in millimetres.
pub fn write(bodies: &[MeshBody]) -> Result<Vec<u8>, IoError> {
    if bodies.is_empty() {
        return Err(bad("there is nothing to write"));
    }
    let mut model = String::new();
    let _ = write!(
        model,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"{CORE_NS}\">\n\
         <metadata name=\"Application\">Chamfera</metadata>\n<resources>\n\
         <basematerials id=\"1\">\n"
    );
    for b in bodies {
        let _ = writeln!(
            model,
            "<base name=\"{}\" displaycolor=\"{}\"/>",
            xml::escape(b.name),
            hex_colour(b.colour)
        );
    }
    model.push_str("</basematerials>\n");
    for (i, b) in bodies.iter().enumerate() {
        b.mesh
            .validate()
            .map_err(|e| bad(format!("{}: {e}", b.name)))?;
        let _ = writeln!(
            model,
            "<object id=\"{}\" type=\"model\" name=\"{}\" pid=\"1\" pindex=\"{i}\">\n<mesh>\n<vertices>",
            i + 2,
            xml::escape(b.name)
        );
        for p in &b.mesh.positions {
            let _ = writeln!(model, "<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", p.x, p.y, p.z);
        }
        model.push_str("</vertices>\n<triangles>\n");
        for [a, b, c] in &b.mesh.triangles {
            let _ = writeln!(model, "<triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>");
        }
        model.push_str("</triangles>\n</mesh>\n</object>\n");
    }
    model.push_str("</resources>\n<build>\n");
    for i in 0..bodies.len() {
        let _ = writeln!(model, "<item objectid=\"{}\"/>", i + 2);
    }
    model.push_str("</build>\n</model>\n");
    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n\
        <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n\
        <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n\
        </Types>\n";
    let rels = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n\
         <Relationship Target=\"/{MODEL_PATH}\" Id=\"rel0\" Type=\"{MODEL_REL}\"/>\n\
         </Relationships>\n"
    );
    crate::zip::write_stored(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        (MODEL_PATH, model.as_bytes()),
    ])
}

/// One mesh read from a 3MF build item, in millimetres.
#[derive(Debug, Clone)]
pub struct Item {
    pub name: String,
    pub mesh: TriMesh,
}

/// An affine transform, 3MF style: a point is the row vector `[x y z 1]` times the 4 x 3
/// matrix `m` (rows m00 m01 m02 / m10 m11 m12 / m20 m21 m22 / m30 m31 m32).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Transform([f64; 12]);

impl Transform {
    const IDENTITY: Transform = Transform([1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.]);

    fn parse(s: Option<&str>) -> Result<Self, IoError> {
        let Some(s) = s else {
            return Ok(Self::IDENTITY);
        };
        let mut m = [0.0; 12];
        let mut values = s.split_ascii_whitespace();
        for v in &mut m {
            *v = values
                .next()
                .and_then(|t| t.parse::<f64>().ok())
                .filter(|v| v.is_finite() && v.abs() <= 1e9)
                .ok_or_else(|| bad("a transform needs 12 finite numbers"))?;
        }
        if values.next().is_some() {
            return Err(bad("a transform has more than 12 numbers"));
        }
        Ok(Transform(m))
    }

    fn apply(&self, p: Point3) -> Point3 {
        let m = &self.0;
        Point3::new(
            p.x * m[0] + p.y * m[3] + p.z * m[6] + m[9],
            p.x * m[1] + p.y * m[4] + p.z * m[7] + m[10],
            p.x * m[2] + p.y * m[5] + p.z * m[8] + m[11],
        )
    }

    /// `self` first, then `then`.
    fn then(&self, then: &Transform) -> Transform {
        let (a, b) = (&self.0, &then.0);
        let mut m = [0.0; 12];
        for r in 0..4 {
            for c in 0..3 {
                let mut v = a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c];
                if r == 3 {
                    v += b[9 + c];
                }
                m[r * 3 + c] = v;
            }
        }
        Transform(m)
    }
}

#[derive(Debug, Default)]
struct Object {
    name: Option<String>,
    mesh: TriMesh,
    components: Vec<(u32, Transform)>,
}

/// Millimetres per model unit.
fn unit_scale(unit: &str) -> Result<f64, IoError> {
    Ok(match unit {
        "micron" => 0.001,
        "millimeter" => 1.0,
        "centimeter" => 10.0,
        "inch" => 25.4,
        "foot" => 304.8,
        "meter" => 1000.0,
        other => return Err(bad(format!("unknown unit {other:?}"))),
    })
}

fn attr_u32(tag: &Tag, key: &str) -> Result<u32, IoError> {
    tag.attribute(key)
        .and_then(|v| v.trim().parse::<u32>().ok())
        .ok_or_else(|| bad(format!("`{key}` must be a whole number")))
}

fn attr_f64(tag: &Tag, key: &str) -> Result<f64, IoError> {
    tag.attribute(key)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .ok_or_else(|| bad(format!("vertex `{key}` must be a finite number")))
}

/// Reads a 3MF package: one mesh per build item.
pub fn read(bytes: &[u8]) -> Result<Vec<Item>, IoError> {
    if bytes.len() as u64 > MAX_3MF_BYTES {
        return Err(bad(format!(
            "the file is larger than {MAX_3MF_BYTES} bytes"
        )));
    }
    let zip = crate::zip::Archive::parse(bytes)?;
    // The package relationships name the model part; the usual place is the fallback.
    let mut path = MODEL_PATH.to_owned();
    if let Some(rels) = zip.find("_rels/.rels") {
        let text = zip.read(rels, 1024 * 1024)?;
        let text = String::from_utf8_lossy(&text);
        for tag in xml::tags(&text) {
            let tag = tag.map_err(bad)?;
            if let Tag::Start {
                name: "Relationship",
                ..
            } = tag
                && tag
                    .attribute("Type")
                    .is_some_and(|t| t.ends_with("/3dmodel"))
                && let Some(target) = tag.attribute("Target")
            {
                path = target.to_owned();
                break;
            }
        }
    }
    let entry = zip
        .find(&path)
        .ok_or_else(|| bad(format!("the package has no model part {path}")))?;
    let model = zip.read(entry, MAX_MODEL_BYTES)?;
    let text = std::str::from_utf8(&model).map_err(|_| bad("the model is not UTF-8 text"))?;
    read_model(text)
}

/// Reads the model part's XML.
pub fn read_model(text: &str) -> Result<Vec<Item>, IoError> {
    let mut scale = 1.0;
    let mut objects: HashMap<u32, Object> = HashMap::new();
    let mut items: Vec<(u32, Transform)> = Vec::new();
    let mut current: Option<(u32, Object)> = None;
    let (mut vertices, mut triangles) = (0usize, 0usize);
    let mut in_build = false;
    for tag in xml::tags(text) {
        let tag = tag.map_err(bad)?;
        match &tag {
            Tag::Start { name: "model", .. } => {
                scale = unit_scale(tag.attribute("unit").unwrap_or("millimeter"))?;
            }
            Tag::Start {
                name: "object",
                empty,
                ..
            } => {
                let id = attr_u32(&tag, "id")?;
                let object = Object {
                    name: tag.attribute("name").map(str::to_owned),
                    ..Default::default()
                };
                if *empty {
                    objects.insert(id, object);
                } else {
                    current = Some((id, object));
                }
            }
            Tag::End { name: "object" } => {
                if let Some((id, o)) = current.take() {
                    objects.insert(id, o);
                }
            }
            Tag::Start { name: "vertex", .. } => {
                let Some((_, o)) = current.as_mut() else {
                    return Err(bad("a vertex outside an object"));
                };
                vertices += 1;
                if vertices > MAX_TRIANGLES * 3 {
                    return Err(bad(format!("more than {} vertices", MAX_TRIANGLES * 3)));
                }
                let p = Point3::new(
                    attr_f64(&tag, "x")?,
                    attr_f64(&tag, "y")?,
                    attr_f64(&tag, "z")?,
                );
                o.mesh.positions.push(p);
            }
            Tag::Start {
                name: "triangle", ..
            } => {
                let Some((_, o)) = current.as_mut() else {
                    return Err(bad("a triangle outside an object"));
                };
                triangles += 1;
                if triangles > MAX_TRIANGLES {
                    return Err(bad(format!("more than {MAX_TRIANGLES} triangles")));
                }
                let t = [
                    attr_u32(&tag, "v1")?,
                    attr_u32(&tag, "v2")?,
                    attr_u32(&tag, "v3")?,
                ];
                o.mesh.triangles.push(t);
            }
            Tag::Start {
                name: "component", ..
            } => {
                let Some((_, o)) = current.as_mut() else {
                    return Err(bad("a component outside an object"));
                };
                if o.components.len() >= MAX_ITEMS * 10 {
                    return Err(bad("too many components"));
                }
                let t = Transform::parse(tag.attribute("transform"))?;
                o.components.push((attr_u32(&tag, "objectid")?, t));
            }
            Tag::Start { name: "build", .. } => in_build = true,
            Tag::End { name: "build" } => in_build = false,
            Tag::Start { name: "item", .. } if in_build => {
                if items.len() >= MAX_ITEMS {
                    return Err(bad(format!("more than {MAX_ITEMS} build items")));
                }
                let t = Transform::parse(tag.attribute("transform"))?;
                items.push((attr_u32(&tag, "objectid")?, t));
            }
            _ => {}
        }
    }
    let to_mm = Transform([scale, 0., 0., 0., scale, 0., 0., 0., scale, 0., 0., 0.]);
    let mut out = Vec::new();
    let mut visits = 0usize;
    for (k, (id, t)) in items.iter().enumerate() {
        let mut mesh = TriMesh::default();
        flatten(&objects, *id, &t.then(&to_mm), 0, &mut visits, &mut mesh)?;
        mesh.validate()
            .map_err(|e| bad(format!("build item {}: {e}", k + 1)))?;
        let name = objects
            .get(id)
            .and_then(|o| o.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("3MF {}", k + 1));
        out.push(Item { name, mesh });
    }
    if out.is_empty() {
        return Err(bad("the build has no items"));
    }
    Ok(out)
}

/// Adds object `id`'s triangles, and its components', moved by `t`, to `out`.
fn flatten(
    objects: &HashMap<u32, Object>,
    id: u32,
    t: &Transform,
    depth: usize,
    visits: &mut usize,
    out: &mut TriMesh,
) -> Result<(), IoError> {
    if depth > MAX_DEPTH {
        return Err(bad(format!("components nest deeper than {MAX_DEPTH}")));
    }
    // Components may share objects, so a small file can name a huge tree: cap the visits.
    *visits += 1;
    if *visits > MAX_VISITS {
        return Err(bad(format!(
            "the build uses objects more than {MAX_VISITS} times"
        )));
    }
    let o = objects
        .get(&id)
        .ok_or_else(|| bad(format!("no object with id {id}")))?;
    if out.triangles.len().saturating_add(o.mesh.triangles.len()) > MAX_TRIANGLES {
        return Err(bad(format!("more than {MAX_TRIANGLES} triangles")));
    }
    let n = o.mesh.positions.len();
    if let Some(tri) = o
        .mesh
        .triangles
        .iter()
        .find(|t| t.iter().any(|&v| v as usize >= n))
    {
        return Err(bad(format!(
            "a triangle refers to a missing vertex: {tri:?}"
        )));
    }
    let base = u32::try_from(out.positions.len()).map_err(|_| bad("too many vertices"))?;
    out.positions
        .extend(o.mesh.positions.iter().map(|p| t.apply(*p)));
    out.triangles.extend(
        o.mesh
            .triangles
            .iter()
            .map(|tri| [tri[0] + base, tri[1] + base, tri[2] + base]),
    );
    for (child, ct) in &o.components {
        flatten(objects, *child, &ct.then(t), depth + 1, visits, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> TriMesh {
        TriMesh::cuboid(Point3::ORIGIN, Point3::new(50.0, 30.0, 10.0))
    }

    #[test]
    fn a_box_round_trips_with_12_triangles_and_volume_15000() {
        let m = block();
        let bytes = write(&[MeshBody {
            name: "Box & lid",
            mesh: &m,
            colour: [10, 20, 30],
        }])
        .unwrap();
        let items = read(&bytes).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Box & lid");
        assert_eq!(items[0].mesh.triangles.len(), 12);
        // 50 * 30 * 10 = 15000 mm^3.
        assert!((items[0].mesh.signed_volume() - 15000.0).abs() < 1e-9);
        let zip = crate::zip::Archive::parse(&bytes).unwrap();
        let model = zip.read(zip.find(MODEL_PATH).unwrap(), 1 << 20).unwrap();
        assert!(
            String::from_utf8(model)
                .unwrap()
                .contains("displaycolor=\"#0A141E\"")
        );
    }

    #[test]
    fn units_and_transforms_are_applied() {
        // A unit triangle in inches, moved 1 inch along x by its build item, made of a
        // component scaled 2x: the corner (1, 0, 0) lands at (2 + 1) * 25.4 mm.
        let text = r#"<model unit="inch"><resources>
            <object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/>
            <vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/>
            </triangles></mesh></object>
            <object id="2"><components><component objectid="1" transform="2 0 0 0 2 0 0 0 2 0 0 0"/>
            </components></object></resources>
            <build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 1 0 0"/></build></model>"#;
        let items = read_model(text).unwrap();
        let p = items[0].mesh.positions[1];
        assert!((p.x - 3.0 * 25.4).abs() < 1e-9 && p.y.abs() < 1e-12);
        // Area: (2 * 25.4)^2 / 2.
        assert!((items[0].mesh.area() - (2.0f64 * 25.4).powi(2) / 2.0).abs() < 1e-9);
    }

    #[test]
    fn hostile_models_are_refused_without_panicking() {
        let tri = |v: &str, t: &str| {
            format!(
                "<model><resources><object id=\"1\"><mesh><vertices>{v}</vertices>\
                 <triangles>{t}</triangles></mesh></object></resources>\
                 <build><item objectid=\"1\"/></build></model>"
            )
        };
        let v3 = "<vertex x=\"0\" y=\"0\" z=\"0\"/><vertex x=\"1\" y=\"0\" z=\"0\"/><vertex x=\"0\" y=\"1\" z=\"0\"/>";
        assert!(read_model(&tri(v3, "<triangle v1=\"0\" v2=\"1\" v3=\"2\"/>")).is_ok());
        let cases = [
            tri(v3, "<triangle v1=\"0\" v2=\"1\" v3=\"9\"/>"),
            tri(v3, "<triangle v1=\"0\" v2=\"1\" v3=\"-1\"/>"),
            tri(v3, ""),
            tri("<vertex x=\"NaN\" y=\"0\" z=\"0\"/>", "<triangle v1=\"0\" v2=\"0\" v3=\"0\"/>"),
            tri("<vertex x=\"1e300\" y=\"0\" z=\"0\"/><vertex x=\"0\" y=\"0\" z=\"0\"/><vertex x=\"0\" y=\"1\" z=\"0\"/>", "<triangle v1=\"0\" v2=\"1\" v3=\"2\"/>"),
            "<model unit=\"parsec\"/>".to_owned(),
            "<model><build><item objectid=\"7\"/></build></model>".to_owned(),
            // An object made of itself.
            "<model><resources><object id=\"1\"><components><component objectid=\"1\"/></components></object></resources><build><item objectid=\"1\"/></build></model>".to_owned(),
            "<model><build/></model>".to_owned(),
            // Ten objects, each made of ten copies of the next: 10^10 instances.
            {
                let mut t = String::from("<model><resources>");
                for i in 1..=10 {
                    t.push_str(&format!("<object id=\"{i}\"><components>"));
                    for _ in 0..10 {
                        t.push_str(&format!("<component objectid=\"{}\"/>", i + 1));
                    }
                    t.push_str("</components></object>");
                }
                t.push_str("<object id=\"11\"/></resources><build><item objectid=\"1\"/></build></model>");
                t
            },
            "<vertex x=\"1\"/>".to_owned(),
        ];
        for c in &cases {
            assert!(read_model(c).is_err(), "{c}");
        }
        for b in [&b""[..], b"PK\x03\x04", &[0u8; 64]] {
            assert!(read(b).is_err());
        }
        // A truncated package, at every length.
        let m = block();
        let bytes = write(&[MeshBody {
            name: "b",
            mesh: &m,
            colour: [0; 3],
        }])
        .unwrap();
        for n in (0..bytes.len()).step_by(7) {
            assert!(read(&bytes[..n]).is_err());
        }
    }
}
