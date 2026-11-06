//! Selection filters: which kinds of thing a click may pick. `select.pick` and
//! `sketch.select` take an optional `filter` list; without one they pick as before.
//!
//! On solids a filtered pick looks for an edge first (within
//! [`crate::solids::EDGE_PICK_PIXELS`]), then for the face or body under the pixel. In a
//! sketch, `vertices` are its points, `sketch` its ordinary curves and `construction` its
//! construction curves.

use crate::{CommandError, Session};
use chamfera_render::Hit;
use serde::Serialize;
use serde_json::Value;

/// What a pick may select. Every kind is allowed by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PickFilter {
    pub bodies: bool,
    pub faces: bool,
    pub edges: bool,
    pub vertices: bool,
    pub sketch: bool,
    pub construction: bool,
}

impl Default for PickFilter {
    fn default() -> Self {
        PickFilter::ALL
    }
}

/// The filter's kinds, as the `filter` parameter names them.
pub const KINDS: [&str; 6] = [
    "bodies",
    "faces",
    "edges",
    "vertices",
    "sketch",
    "construction",
];

impl PickFilter {
    pub const ALL: PickFilter = PickFilter {
        bodies: true,
        faces: true,
        edges: true,
        vertices: true,
        sketch: true,
        construction: true,
    };
    pub const NONE: PickFilter = PickFilter {
        bodies: false,
        faces: false,
        edges: false,
        vertices: false,
        sketch: false,
        construction: false,
    };

    /// The switch for one kind, by its name in [`KINDS`].
    pub fn kind_mut(&mut self, kind: &str) -> Option<&mut bool> {
        Some(match kind {
            "bodies" => &mut self.bodies,
            "faces" => &mut self.faces,
            "edges" => &mut self.edges,
            "vertices" => &mut self.vertices,
            "sketch" => &mut self.sketch,
            "construction" => &mut self.construction,
            _ => return None,
        })
    }

    /// Whether one kind is allowed, by its name in [`KINDS`].
    pub fn allows(&self, kind: &str) -> bool {
        let mut f = *self;
        f.kind_mut(kind).is_some_and(|b| *b)
    }

    /// The allowed kinds, as the `filter` parameter lists them.
    pub fn names(&self) -> Vec<&'static str> {
        KINDS.into_iter().filter(|k| self.allows(k)).collect()
    }

    /// The optional `filter` parameter: a list of kinds from [`KINDS`]. `None` when it is
    /// missing or null.
    pub fn from_params(p: &Value) -> Result<Option<PickFilter>, CommandError> {
        let list = match p.get("filter") {
            None | Some(Value::Null) => return Ok(None),
            Some(Value::Array(list)) => list,
            Some(_) => {
                return Err(CommandError::BadParams(
                    "`filter` must be a list such as [\"faces\", \"edges\"]".into(),
                ));
            }
        };
        if list.len() > KINDS.len() * 4 {
            return Err(CommandError::BadParams(
                "`filter` lists too many kinds".into(),
            ));
        }
        let mut f = PickFilter::NONE;
        for k in list {
            let name = k.as_str().unwrap_or("");
            let Some(slot) = f.kind_mut(name) else {
                return Err(CommandError::BadParams(format!(
                    "`filter` kinds are {}, not {k}",
                    KINDS.join(", ")
                )));
            };
            *slot = true;
        }
        Ok(Some(f))
    }
}

/// What a filtered pick found under a pixel.
#[derive(Debug, Clone, PartialEq)]
pub enum Picked {
    /// A true edge of a solid: the solid's index in [`Session::solids`], the edge's name
    /// and the scene body that shows the solid.
    Edge {
        solid: usize,
        name: String,
        body: Option<u32>,
    },
    /// A named face of a solid, and where the ray hit it.
    Face {
        hit: Hit,
        name: String,
    },
    /// A body (a mesh, or a solid when faces are filtered out).
    Body(Hit),
    Nothing,
}

impl Picked {
    /// The scene body the pick is on, if any.
    pub fn body(&self) -> Option<u32> {
        match self {
            Picked::Edge { body, .. } => *body,
            Picked::Face { hit, .. } | Picked::Body(hit) => Some(hit.body),
            Picked::Nothing => None,
        }
    }
}

impl Session {
    /// The name of the solid face under the last pick, if it hit one.
    pub fn selected_face(&self) -> Option<String> {
        self.scene.selection.and_then(|h| self.face_name_of(&h))
    }

    /// The face's name under a hit, for a solid's named face.
    fn face_name_of(&self, hit: &Hit) -> Option<String> {
        let solid = self
            .solids()
            .iter()
            .find(|s| s.scene_id == Some(hit.body))?;
        let face = *solid.faces.get(hit.triangle)?;
        solid.body.face_name(face).ok().flatten().map(str::to_owned)
    }

    /// What pixel `(x, y)` of a `w` x `h` viewport picks with this filter: an edge first,
    /// then the face, then the body. Read-only, so hover can call it on every move.
    pub fn pick_filtered(&self, x: f64, y: f64, w: f64, h: f64, filter: &PickFilter) -> Picked {
        if !(w >= 1.0 && h >= 1.0 && x.is_finite() && y.is_finite()) {
            return Picked::Nothing;
        }
        if filter.edges
            && let Some((solid, _, name)) = self.edge_at(x, y, w, h)
        {
            let body = self.solids().get(solid).and_then(|s| s.scene_id);
            return Picked::Edge { solid, name, body };
        }
        if !(filter.faces || filter.bodies) {
            return Picked::Nothing;
        }
        let bounds = self.scene.bounds();
        let Some(hit) = self
            .camera
            .ray_through(x, y, w, h, bounds.as_ref())
            .and_then(|ray| self.scene.pick(&ray))
        else {
            return Picked::Nothing;
        };
        match self.face_name_of(&hit) {
            Some(name) if filter.faces => Picked::Face { hit, name },
            _ if filter.bodies => Picked::Body(hit),
            _ => Picked::Nothing,
        }
    }
}
