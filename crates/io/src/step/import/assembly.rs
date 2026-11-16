//! The product structure of a STEP assembly as a tree of components with placements,
//! from the public descriptions of ISO 10303-44 (product structure), ISO 10303-43
//! (representation relationships) and the CAx-IF recommended practices for assemblies.
//!
//! # Entities read
//! - `PRODUCT_DEFINITION(id, description, formation, frame)` -> `PRODUCT_DEFINITION_FORMATION
//!   (…, of_product)` -> `PRODUCT(id, name, …)`: a part or assembly, named by its product.
//! - `SHAPE_DEFINITION_REPRESENTATION(definition, used_representation)` with
//!   `PRODUCT_DEFINITION_SHAPE(name, description, definition)`: the shape representation
//!   of a product definition. Representations joined by a `SHAPE_REPRESENTATION_RELATIONSHIP`
//!   without a transformation (a product's `SHAPE_REPRESENTATION` and the
//!   `ADVANCED_BREP_SHAPE_REPRESENTATION` that holds its solids) are one shape.
//! - `NEXT_ASSEMBLY_USAGE_OCCURRENCE(id, name, description, relating, related, designator)`:
//!   `related` is used once in `relating`. Each use becomes a component.
//! - `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(representation_relation,
//!   represented_product_relation)`: the product relation is the
//!   `PRODUCT_DEFINITION_SHAPE` of the usage; the representation relation is the complex
//!   `(REPRESENTATION_RELATIONSHIP(name, description, rep_1, rep_2)
//!   REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(operator)
//!   SHAPE_REPRESENTATION_RELATIONSHIP())`.
//! - `ITEM_DEFINED_TRANSFORMATION(name, description, item_1, item_2)` as the operator: the
//!   rigid motion taking `AXIS2_PLACEMENT_3D` `item_1` (in `rep_1`'s coordinates) onto
//!   `item_2` (in `rep_2`'s). With `L_k` the placement's local-to-world map, a point `p` of
//!   `rep_1` maps to `L_2(L_1⁻¹(p))` in `rep_2`. CAx-IF practice puts the child's shape in
//!   `rep_1` and the parent's in `rep_2`; when a writer has them the other way round (the
//!   child's shape is `rep_2`) the inverse is used, so the child always maps into its
//!   parent.
//!
//! # The tree
//! Each product definition that is never used in another is a root. A single root is the
//! design itself (its own solids stay at the top); several roots each become a top-level
//! component. Below a root, every usage becomes a component holding the solids of the
//! used product, placed into its parent component, recursively (at most
//! [`MAX_ASSEMBLY_DEPTH`] levels and [`MAX_STEP_COMPONENTS`] components; a product that
//! contains itself is refused). A product used twice gives two components, each with its
//! own copy of the solids. Usages whose placement cannot be read keep the identity and
//! say so in [`StepImport::warnings`](super::StepImport::warnings).

use super::{Ctx, Units, View, units_for};
use crate::step::read::{Exchange, Param};
use chamfera_geom::Transform;
use std::collections::{BTreeMap, HashMap};

/// Deepest assembly nesting read.
pub const MAX_ASSEMBLY_DEPTH: usize = 32;
/// Most components one import makes.
pub const MAX_STEP_COMPONENTS: usize = 2_000;

/// One component of an imported assembly.
#[derive(Debug, Clone, PartialEq)]
pub struct StepComponent {
    /// The product's name (or the usage's, when the product has none).
    pub name: String,
    /// The parent component, as an index into [`StepImport::components`](super::StepImport::components);
    /// `None` for a component at the top of the design.
    pub parent: Option<usize>,
    /// From the component's coordinates into its parent's (millimetres).
    pub placement: Transform,
    /// The solids it holds, as indices into [`StepImport::solids`](super::StepImport::solids).
    pub solids: Vec<usize>,
    /// The meshes it holds, as indices into [`StepImport::meshes`](super::StepImport::meshes).
    pub meshes: Vec<usize>,
}

/// A usage inside a product: (usage, used product definition, usage name).
type Usage = (u64, u64, Option<String>);

/// What a shape holds: (solids, meshes) by index into the import's lists.
type Held = (Vec<usize>, Vec<usize>);

/// Union-find over representations.
struct Groups(HashMap<u64, u64>);

impl Groups {
    fn find(&self, mut r: u64) -> u64 {
        // Paths are bounded by the number of entries.
        for _ in 0..=self.0.len() {
            match self.0.get(&r) {
                Some(&p) if p != r => r = p,
                _ => return r,
            }
        }
        r
    }
    fn join(&mut self, a: u64, b: u64) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0.insert(ra, rb);
        }
    }
}

fn refs(p: Option<&Param>) -> Option<u64> {
    match p {
        Some(Param::Ref(r)) => Some(*r),
        _ => None,
    }
}

fn text(p: Option<&Param>) -> Option<String> {
    match p {
        Some(Param::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// A component name: no control characters, at most 256 characters, never empty.
fn clean(name: &str) -> String {
    let s: String = name.chars().filter(|c| !c.is_control()).take(256).collect();
    if s.trim().is_empty() {
        "Component".into()
    } else {
        s
    }
}

/// The assembly read from a file.
pub(super) struct Tree {
    pub components: Vec<StepComponent>,
    pub warnings: Vec<String>,
}

/// The components of `ex` for the solids (`solid_ids[k]` is the instance of solid `k`) and
/// meshes (`mesh_ids`) already imported. Empty when the file has no assembly usages.
pub(super) fn read(ex: &Exchange, solid_ids: &[u64], mesh_ids: &[u64]) -> Tree {
    let mut tree = Tree {
        components: Vec::new(),
        warnings: Vec::new(),
    };
    let usages: Vec<(u64, u64, u64, Option<String>)> = ex
        .data
        .iter()
        .filter_map(|(id, i)| {
            let r = i.record("NEXT_ASSEMBLY_USAGE_OCCURRENCE")?;
            let parent = refs(r.params.get(3))?;
            let child = refs(r.params.get(4))?;
            Some((
                *id,
                parent,
                child,
                text(r.params.get(1)).or(text(r.params.first())),
            ))
        })
        .collect();
    if usages.is_empty() {
        return tree;
    }
    // Representations: which items they list, their context, and the groups joined by
    // relationships without a transformation.
    let mut groups = Groups(HashMap::new());
    let mut item_reps: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut rep_context: HashMap<u64, u64> = HashMap::new();
    for (id, inst) in &ex.data {
        for r in &inst.records {
            if r.name.ends_with("REPRESENTATION")
                && !r.name.ends_with("SHAPE_DEFINITION_REPRESENTATION")
                && let [_, Param::List(items), Param::Ref(ctx), ..] = r.params.as_slice()
            {
                rep_context.insert(*id, *ctx);
                for item in items {
                    if let Param::Ref(i) = item {
                        item_reps.entry(*i).or_default().push(*id);
                    }
                }
            }
        }
        let plain = inst
            .record("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION")
            .is_none();
        let rel = inst
            .record("SHAPE_REPRESENTATION_RELATIONSHIP")
            .filter(|r| r.params.len() >= 4)
            .or_else(|| inst.record("REPRESENTATION_RELATIONSHIP"));
        if plain
            && let Some(rel) = rel
            && let (Some(a), Some(b)) = (refs(rel.params.get(2)), refs(rel.params.get(3)))
        {
            groups.join(a, b);
        }
    }
    // Product definition shapes: PDS -> what it defines.
    let pds: HashMap<u64, u64> = ex
        .data
        .iter()
        .filter_map(|(id, i)| {
            let r = i.record("PRODUCT_DEFINITION_SHAPE")?;
            Some((*id, refs(r.params.get(2))?))
        })
        .collect();
    // Shape group -> product definition.
    let mut group_pd: HashMap<u64, u64> = HashMap::new();
    let mut pd_groups: HashMap<u64, Vec<u64>> = HashMap::new();
    for inst in ex.data.values() {
        let Some(r) = inst.record("SHAPE_DEFINITION_REPRESENTATION") else {
            continue;
        };
        let (Some(def), Some(rep)) = (refs(r.params.first()), refs(r.params.get(1))) else {
            continue;
        };
        if let Some(&pd) = pds.get(&def) {
            let g = groups.find(rep);
            group_pd.entry(g).or_insert(pd);
            pd_groups.entry(pd).or_default().push(g);
        }
    }
    // What each product definition holds.
    let mut held: HashMap<u64, Held> = HashMap::new();
    let owner = |item: u64| {
        item_reps
            .get(&item)
            .into_iter()
            .flatten()
            .find_map(|rep| group_pd.get(&groups.find(*rep)).copied())
    };
    for (k, id) in solid_ids.iter().enumerate() {
        if let Some(pd) = owner(*id) {
            held.entry(pd).or_default().0.push(k);
        }
    }
    for (k, id) in mesh_ids.iter().enumerate() {
        if let Some(pd) = owner(*id) {
            held.entry(pd).or_default().1.push(k);
        }
    }
    // Usage -> its placement (child into parent).
    let mut placements: HashMap<u64, Result<Transform, String>> = HashMap::new();
    for inst in ex.data.values() {
        let Some(r) = inst.record("CONTEXT_DEPENDENT_SHAPE_REPRESENTATION") else {
            continue;
        };
        let (Some(rel), Some(pds_id)) = (refs(r.params.first()), refs(r.params.get(1))) else {
            continue;
        };
        let Some(&usage) = pds.get(&pds_id) else {
            continue;
        };
        let Some(&(_, _, child, _)) = usages.iter().find(|u| u.0 == usage) else {
            continue;
        };
        let child_groups: Vec<u64> = pd_groups.get(&child).cloned().unwrap_or_default();
        placements.insert(
            usage,
            placement(ex, &groups, &rep_context, rel, &child_groups),
        );
    }
    // Roots: definitions that use others and are used by none.
    let used: Vec<u64> = usages.iter().map(|u| u.2).collect();
    let mut roots: Vec<u64> = usages
        .iter()
        .map(|u| u.1)
        .filter(|p| !used.contains(p))
        .collect();
    roots.sort_unstable();
    roots.dedup();
    if roots.is_empty() {
        tree.warnings.push(
            "the assembly structure is a cycle (every product is used in another); it is ignored"
                .into(),
        );
        return tree;
    }
    let names = product_names(ex);
    let mut children: BTreeMap<u64, Vec<Usage>> = BTreeMap::new();
    for (usage, parent, child, name) in &usages {
        children
            .entry(*parent)
            .or_default()
            .push((*usage, *child, name.clone()));
    }
    let single = roots.len() == 1;
    for root in roots {
        let parent = if single {
            None
        } else {
            let name = clean(names.get(&root).map_or("Assembly", String::as_str));
            let (solids, meshes) = held.get(&root).cloned().unwrap_or_default();
            tree.components.push(StepComponent {
                name,
                parent: None,
                placement: Transform::IDENTITY,
                solids,
                meshes,
            });
            Some(tree.components.len() - 1)
        };
        let mut stack = vec![root];
        expand(
            &mut tree,
            &Expand {
                children: &children,
                held: &held,
                placements: &placements,
                names: &names,
            },
            root,
            parent,
            &mut stack,
        );
    }
    tree
}

struct Expand<'a> {
    children: &'a BTreeMap<u64, Vec<Usage>>,
    held: &'a HashMap<u64, Held>,
    placements: &'a HashMap<u64, Result<Transform, String>>,
    names: &'a HashMap<u64, String>,
}

/// Adds a component for every usage inside product definition `pd` (placed in component
/// `parent`), and recursively their insides.
fn expand(tree: &mut Tree, x: &Expand<'_>, pd: u64, parent: Option<usize>, stack: &mut Vec<u64>) {
    let Some(list) = x.children.get(&pd) else {
        return;
    };
    for (usage, child, usage_name) in list {
        if stack.contains(child) {
            tree.warnings.push(format!(
                "usage #{usage} puts product definition #{child} inside itself; it is left out"
            ));
            continue;
        }
        if stack.len() > MAX_ASSEMBLY_DEPTH {
            tree.warnings.push(format!(
                "usage #{usage} is nested deeper than {MAX_ASSEMBLY_DEPTH} levels; it is left out"
            ));
            continue;
        }
        if tree.components.len() >= MAX_STEP_COMPONENTS {
            tree.warnings.push(format!(
                "the assembly has more than {MAX_STEP_COMPONENTS} components; the rest are left out"
            ));
            return;
        }
        let placement = match x.placements.get(usage) {
            Some(Ok(t)) => *t,
            Some(Err(why)) => {
                tree.warnings.push(format!(
                    "usage #{usage}: {why}; it is placed at its parent's origin"
                ));
                Transform::IDENTITY
            }
            None => {
                tree.warnings.push(format!(
                    "usage #{usage} has no placement (CONTEXT_DEPENDENT_SHAPE_REPRESENTATION); it is placed at its parent's origin"
                ));
                Transform::IDENTITY
            }
        };
        let name = x
            .names
            .get(child)
            .cloned()
            .or_else(|| usage_name.clone())
            .unwrap_or_else(|| format!("Component {}", tree.components.len() + 1));
        let (solids, meshes) = x.held.get(child).cloned().unwrap_or_default();
        tree.components.push(StepComponent {
            name: clean(&name),
            parent,
            placement,
            solids,
            meshes,
        });
        let me = tree.components.len() - 1;
        stack.push(*child);
        expand(tree, x, *child, Some(me), stack);
        stack.pop();
    }
}

/// Product definition -> its product's name (or id).
fn product_names(ex: &Exchange) -> HashMap<u64, String> {
    let mut out = HashMap::new();
    for (id, inst) in &ex.data {
        let Some(r) = inst.record("PRODUCT_DEFINITION") else {
            continue;
        };
        let product = refs(r.params.get(2))
            .and_then(|f| ex.data.get(&f))
            .and_then(|f| {
                f.records
                    .iter()
                    .find(|r| r.name.starts_with("PRODUCT_DEFINITION_FORMATION"))
            })
            .and_then(|f| refs(f.params.get(2)))
            .and_then(|p| ex.data.get(&p))
            .and_then(|p| p.record("PRODUCT"));
        if let Some(p) = product
            && let Some(name) = text(p.params.get(1)).or_else(|| text(p.params.first()))
        {
            out.insert(*id, name);
        }
    }
    out
}

/// The placement of a usage from its representation relationship `rel`: child into parent.
fn placement(
    ex: &Exchange,
    groups: &Groups,
    rep_context: &HashMap<u64, u64>,
    rel: u64,
    child_groups: &[u64],
) -> Result<Transform, String> {
    let inst = ex
        .data
        .get(&rel)
        .ok_or_else(|| format!("the representation relationship #{rel} is not in the file"))?;
    let rr = inst
        .record("REPRESENTATION_RELATIONSHIP")
        .or_else(|| inst.record("SHAPE_REPRESENTATION_RELATIONSHIP"))
        .filter(|r| r.params.len() >= 4)
        .ok_or_else(|| format!("#{rel} is not a representation relationship"))?;
    let (rep_1, rep_2) = (
        refs(rr.params.get(2)).ok_or("its rep_1 is not a reference")?,
        refs(rr.params.get(3)).ok_or("its rep_2 is not a reference")?,
    );
    let op = inst
        .record("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION")
        .and_then(|r| refs(r.params.first()))
        .ok_or_else(|| format!("#{rel} has no transformation"))?;
    let idt = ex
        .data
        .get(&op)
        .and_then(|i| i.record("ITEM_DEFINED_TRANSFORMATION"))
        .ok_or_else(|| {
            format!("the transformation #{op} is not an ITEM_DEFINED_TRANSFORMATION, which is the only kind read")
        })?;
    let (item_1, item_2) = (
        refs(idt.params.get(2)).ok_or("its transform_item_1 is not a reference")?,
        refs(idt.params.get(3)).ok_or("its transform_item_2 is not a reference")?,
    );
    let frame = |item: u64, rep: u64| -> Result<Transform, String> {
        let units: Units = units_for(ex, rep_context.get(&rep).copied())?;
        let ctx = Ctx {
            view: View { ex },
            units,
        };
        ctx.placement(item)
            .map(|f| f.local_to_world())
            .map_err(|f| match f {
                super::Fail::Invalid(s) => s,
                super::Fail::Unsupported(v) => v.join("; "),
            })
    };
    let (l1, l2) = (frame(item_1, rep_1)?, frame(item_2, rep_2)?);
    let child_is_2 =
        child_groups.contains(&groups.find(rep_2)) && !child_groups.contains(&groups.find(rep_1));
    let (from, to) = if child_is_2 { (l2, l1) } else { (l1, l2) };
    let t = from
        .inverse_rigid()
        .ok_or("a placement is not a rigid frame")?
        .then(&to);
    if t.is_rigid()
        && [
            t.translation_part().x,
            t.translation_part().y,
            t.translation_part().z,
        ]
        .iter()
        .all(|c| c.is_finite() && c.abs() <= chamfera_geom::MAX_COORDINATE)
    {
        Ok(t)
    } else {
        Err("the placement is not a rigid move within the working range".into())
    }
}
