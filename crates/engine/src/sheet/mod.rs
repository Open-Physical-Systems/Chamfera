//! Building sheet metal features on the timeline: base flanges, flanges and flat patterns
//! (the geometry is the kernel's [`chamfera_kernel::Sheet`]).
//!
//! A base flange makes a sheet, kept by its feature id beside its folded solid in the
//! bodies built so far. A flange finds its edge by name in a sheet's solid, so it follows
//! the sheet when the base's sketch or rule changes. A flat pattern lays the sheet flat
//! and shows it as a body of its own beside the part; its outline is kept for DXF export.
//! Features other than sheet metal ones (a cut, a fillet) change the folded solid but are
//! not unfolded.

#[cfg(test)]
mod tests;

use crate::solids::{name_edges, name_faces};
use chamfera_doc::{
    Document, Feature, FeatureKind, FlangePosition, FlangeSide, ReliefShape, SheetBase,
    SheetFlange, SheetRule, SheetUnfold,
};
use chamfera_geom::{Point2, Transform};
use chamfera_kernel::{
    BendPosition, BendReport, Body, FlangeSpec, FlatEdge, Relief, ReliefKind, Sheet,
};
use std::collections::BTreeMap;

/// How far beside the part (mm, along the base plane's x) a flat pattern is shown.
pub const FLAT_PATTERN_GAP: f64 = 20.0;

/// A sheet built so far and the rule it follows.
#[derive(Debug, Clone)]
pub struct BuiltSheet {
    pub sheet: Sheet,
    pub rule: SheetRule,
}

/// A flat pattern as built: its outline in the base's sketch plane and its measures.
#[derive(Debug, Clone)]
pub struct FlatPattern {
    /// The base flange it unfolds.
    pub base: u32,
    pub frame: chamfera_geom::Frame,
    pub outline: Vec<FlatEdge>,
    /// The outline's extent in the sketch plane.
    pub min: Point2,
    pub max: Point2,
    /// The flat area (the flat solid's volume over the thickness).
    pub area: f64,
    pub thickness: f64,
    pub bends: Vec<BendReport>,
}

/// Sheets and flat patterns from the last rebuild, by feature id.
#[derive(Debug, Clone, Default)]
pub struct SheetState {
    pub sheets: BTreeMap<u32, BuiltSheet>,
    pub flats: BTreeMap<u32, FlatPattern>,
}

/// True for the sheet metal features this module builds.
pub fn is_sheet_feature(kind: &FeatureKind) -> bool {
    matches!(
        kind,
        FeatureKind::SheetBase(_) | FeatureKind::SheetFlange(_) | FeatureKind::SheetUnfold(_)
    )
}

/// Builds one sheet metal feature onto the bodies built so far. On an error nothing
/// changes.
pub(crate) fn build_feature(
    doc: &Document,
    feature: &Feature,
    made: &mut Vec<(u32, String, Body)>,
    state: &mut SheetState,
) -> Result<(), String> {
    match &feature.kind {
        FeatureKind::SheetBase(b) => {
            let built = build_base(doc, b, feature.id)?;
            made.push((
                feature.id,
                feature.name.clone(),
                built.sheet.folded().clone(),
            ));
            state.sheets.insert(feature.id, built);
            Ok(())
        }
        FeatureKind::SheetFlange(f) => build_flange(doc, f, feature.id, made, state),
        FeatureKind::SheetUnfold(u) => {
            let (body, flat) = build_unfold(u, made, state)?;
            made.push((feature.id, feature.name.clone(), body));
            state.flats.insert(feature.id, flat);
            Ok(())
        }
        _ => Err("not a sheet metal feature".into()),
    }
}

fn build_base(doc: &Document, b: &SheetBase, id: u32) -> Result<BuiltSheet, String> {
    let rule = doc.sheet_rule(&b.rule).map_err(|e| e.to_string())?.clone();
    let f = doc.sketch(b.sketch).map_err(|e| e.to_string())?;
    let profiles = crate::solids::feature_profiles(f, &b.profiles)?;
    let [profile] = profiles.as_slice() else {
        return Err("a base flange takes exactly one profile".into());
    };
    let mut sheet = Sheet::base(&f.plane, profile, rule.thickness, b.reversed)
        .map_err(|e| format!("base flange: {e}"))?;
    name_faces(sheet.folded_mut(), id);
    name_edges(sheet.folded_mut(), id);
    Ok(BuiltSheet { sheet, rule })
}

/// The kernel's flange values from a flange feature and its sheet's rule.
fn flange_spec(doc: &Document, f: &SheetFlange, rule: &SheetRule) -> Result<FlangeSpec, String> {
    let v = doc.flange_values(f).map_err(|e| e.to_string())?;
    Ok(FlangeSpec {
        height: v.height,
        angle: v.angle,
        radius: rule.bend_radius,
        k_factor: rule.k_factor,
        position: match f.position {
            FlangePosition::Inside => BendPosition::Inside,
            FlangePosition::Outside => BendPosition::Outside,
            FlangePosition::BendOutside => BendPosition::BendOutside,
        },
        flip: f.side == FlangeSide::Opposite,
        offset_start: v.offset_start,
        offset_end: v.offset_end,
        relief: Relief {
            kind: match rule.relief {
                ReliefShape::Rectangular => ReliefKind::Rectangular,
                ReliefShape::Obround => ReliefKind::Obround,
                ReliefShape::Rip => ReliefKind::Rip,
            },
            width: rule.relief_width,
            depth: rule.relief_depth,
        },
    })
}

/// The sheet (its base id) whose solid has the edge named `edge`, the index of that solid
/// in `made`, and the edge.
/// The edge goes through the resolver (rung K8), with the flange's signature.
fn find_sheet_edge(
    doc: &Document,
    owner: u32,
    made: &[(u32, String, Body)],
    state: &SheetState,
    edge: &str,
) -> Option<(u32, usize, chamfera_kernel::EdgeId)> {
    let bodies = crate::naming::bodies(made);
    let f = crate::naming::resolve(
        doc,
        owner,
        &bodies,
        crate::naming::Kind::Edge,
        edge,
        crate::naming::Want::One,
    )
    .ok()?
    .into_iter()
    .next()?;
    let feature = made.get(f.body)?.0;
    state
        .sheets
        .contains_key(&feature)
        .then_some((feature, f.body, f.edge()?))
}

fn build_flange(
    doc: &Document,
    f: &SheetFlange,
    id: u32,
    made: &mut [(u32, String, Body)],
    state: &mut SheetState,
) -> Result<(), String> {
    let (base, index, edge) = find_sheet_edge(doc, id, made, state, &f.edge).ok_or_else(|| {
        format!(
            "flange: the edge {:?} is not an edge of a sheet metal part (any more)",
            f.edge
        )
    })?;
    let built = state.sheets.get(&base).ok_or("flange: the sheet is gone")?;
    let spec = flange_spec(doc, f, &built.rule)?;
    // The folded solid as built so far (later features such as cuts are kept).
    let mut sheet = built.sheet.clone();
    if let Some((_, _, body)) = made.get(index) {
        *sheet.folded_mut() = body.clone();
    }
    let mut next = sheet
        .flange(edge, &spec)
        .map_err(|e| format!("flange: {e}"))?;
    name_faces(next.folded_mut(), id);
    name_edges(next.folded_mut(), id);
    if let Some(m) = made.get_mut(index) {
        m.2 = next.folded().clone();
    }
    let rule = built.rule.clone();
    state.sheets.insert(base, BuiltSheet { sheet: next, rule });
    Ok(())
}

/// The extent of an outline in its plane.
fn outline_bounds(outline: &[FlatEdge]) -> Option<(Point2, Point2)> {
    let mut lo = Point2::new(f64::INFINITY, f64::INFINITY);
    let mut hi = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut add = |p: Point2| {
        lo = Point2::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Point2::new(hi.x.max(p.x), hi.y.max(p.y));
    };
    let at = |c: Point2, r: f64, a: f64| Point2::new(c.x + r * a.cos(), c.y + r * a.sin());
    for e in outline {
        match *e {
            FlatEdge::Line { a, b } => {
                add(a);
                add(b);
            }
            FlatEdge::Circle { centre, radius } => {
                add(Point2::new(centre.x - radius, centre.y - radius));
                add(Point2::new(centre.x + radius, centre.y + radius));
            }
            FlatEdge::Arc {
                centre,
                radius,
                start_angle,
                end_angle,
            } => {
                add(at(centre, radius, start_angle));
                add(at(centre, radius, end_angle));
                let sweep = (end_angle - start_angle).rem_euclid(std::f64::consts::TAU);
                for k in 0..4 {
                    let a = f64::from(k) * std::f64::consts::FRAC_PI_2;
                    if (a - start_angle).rem_euclid(std::f64::consts::TAU) < sweep {
                        add(at(centre, radius, a));
                    }
                }
            }
        }
    }
    (lo.x <= hi.x && lo.y <= hi.y).then_some((lo, hi))
}

fn build_unfold(
    u: &SheetUnfold,
    made: &[(u32, String, Body)],
    state: &SheetState,
) -> Result<(Body, FlatPattern), String> {
    let built = state
        .sheets
        .get(&u.base)
        .ok_or("flat pattern: its base flange is suppressed or could not be built")?;
    let mut sheet = built.sheet.clone();
    if let Some((_, _, body)) = made.iter().find(|m| m.0 == u.base) {
        *sheet.folded_mut() = body.clone();
    }
    let k = |e: chamfera_kernel::KernelError| format!("flat pattern: {e}");
    let flat = sheet.flat().map_err(k)?;
    let frame = *sheet.frame();
    let outline = chamfera_kernel::flat_outline(&flat, &frame).map_err(k)?;
    let (min, max) = outline_bounds(&outline).ok_or("flat pattern: the outline is empty")?;
    let t = sheet.thickness();
    let area = flat.volume().map_err(k)? / t;
    // Shown beside the part, along the sketch plane's x.
    let shift = Transform::translation(frame.x() * (max.x - min.x + FLAT_PATTERN_GAP))
        .ok_or("flat pattern: it would lie outside the working range")?;
    let shown = flat.transformed(&shift).map_err(k)?;
    Ok((
        shown,
        FlatPattern {
            base: u.base,
            frame,
            outline,
            min,
            max,
            area,
            thickness: t,
            bends: sheet.bends().to_vec(),
        },
    ))
}

/// A flat pattern's outline as DXF R12, by the drawing crate's writer: on the VISIBLE
/// layer, moved so its lower-left corner is at the origin.
pub fn flat_dxf(f: &FlatPattern) -> String {
    use chamfera_drawing::{Item, Layer, Shape, SheetGeometry};
    let at = |p: Point2| Point2::new(p.x - f.min.x, p.y - f.min.y);
    let items = f
        .outline
        .iter()
        .map(|e| {
            let shape = match *e {
                FlatEdge::Line { a, b } => Shape::Line { a: at(a), b: at(b) },
                FlatEdge::Circle { centre, radius } => Shape::Circle {
                    centre: at(centre),
                    radius,
                },
                FlatEdge::Arc {
                    centre,
                    radius,
                    start_angle,
                    end_angle,
                } => {
                    let sweep = (end_angle - start_angle).rem_euclid(std::f64::consts::TAU);
                    Shape::Arc {
                        centre: at(centre),
                        radius,
                        start: start_angle,
                        end: start_angle + sweep,
                    }
                }
            };
            Item {
                layer: Layer::Visible,
                shape,
            }
        })
        .collect();
    chamfera_drawing::dxf::write(&SheetGeometry {
        width: f.max.x - f.min.x,
        height: f.max.y - f.min.y,
        items,
        views: Vec::new(),
    })
}

impl crate::Session {
    /// The flat patterns of the last rebuild, by feature id.
    pub fn flat_patterns(&self) -> &BTreeMap<u32, FlatPattern> {
        &self.built.sheet.flats
    }

    /// The sheet metal parts of the last rebuild, by base flange id.
    pub fn sheets(&self) -> &BTreeMap<u32, BuiltSheet> {
        &self.built.sheet.sheets
    }
}
