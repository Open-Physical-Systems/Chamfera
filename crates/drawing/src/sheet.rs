//! A sheet laid out on paper: border, title block, views and dimensions.
//!
//! The title block is Chamfera's own simple design: a projection-symbol cell on the left
//! (the ISO 128 truncated-cone symbol for first- or third-angle), then the part name
//! across the top and scale, units, date and drawer below.

use crate::annot::{self, PartRow};
use crate::hlr::{Basis, Curve2, ViewGeometry, project, project_basis};
use crate::text::{clean, width};
use crate::{Align, DrawingResult, Item, Layer, Shape, dims};
use crate::{refs, section};
use chamfera_doc::drawing::{
    AnnotationKind, ProjectionAngle, Sheet, SheetDimension, SheetView, ViewDirection,
};
use chamfera_geom::{Point2, Point3};
use chamfera_kernel::Body;
use std::collections::HashMap;

/// Margins of the drawing frame (mm): a wider filing margin on the left (ISO 5457).
pub const MARGIN_LEFT: f64 = 20.0;
pub const MARGIN: f64 = 10.0;
/// The title block's size (mm).
pub const TITLE_WIDTH: f64 = 180.0;
pub const TITLE_HEIGHT: f64 = 30.0;
/// How far centre marks and centre lines reach past their circle or cylinder (mm).
const CENTRE_OVERSHOOT: f64 = 2.0;
/// The gap between a base view and a projected view placed on top of it (mm), used
/// when a projected view is asked for at its base view's own position.
const DEFAULT_GAP: f64 = 80.0;

/// A view as placed on the sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedView {
    pub id: u32,
    pub direction: ViewDirection,
    /// The view's axes in model space (a section view's are its own).
    pub basis: Basis,
    /// Sheet mm per model mm.
    pub scale: f64,
    /// Where the model's centre goes on the sheet.
    pub centre: Point2,
    /// The model's centre in view coordinates.
    pub model_centre: Point2,
    /// The projected lines, in view coordinates (model mm).
    pub geometry: ViewGeometry,
    /// A detail view shows only what lies inside this circle (sheet mm: centre, radius).
    pub clip: Option<(Point2, f64)>,
    /// A section view (no hidden lines; its cut faces hatched).
    pub section: bool,
    /// Why the view is empty, when its body is missing or could not be projected.
    pub problem: Option<String>,
}

impl PlacedView {
    pub fn to_sheet(&self, p: Point2) -> Point2 {
        Point2::new(
            self.centre.x + (p.x - self.model_centre.x) * self.scale,
            self.centre.y + (p.y - self.model_centre.y) * self.scale,
        )
    }

    pub fn to_view(&self, p: Point2) -> Point2 {
        Point2::new(
            self.model_centre.x + (p.x - self.centre.x) / self.scale,
            self.model_centre.y + (p.y - self.centre.y) / self.scale,
        )
    }

    /// The view's box on the sheet.
    pub fn bounds(&self) -> Option<(Point2, Point2)> {
        if let Some((c, r)) = self.clip {
            return Some((Point2::new(c.x - r, c.y - r), Point2::new(c.x + r, c.y + r)));
        }
        let (lo, hi) = self.geometry.bounds()?;
        Some((self.to_sheet(lo), self.to_sheet(hi)))
    }
}

/// What a dimension shows once the sheet is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionState {
    pub id: u32,
    pub view: u32,
    /// mm, or degrees for an angle.
    pub value: Option<f64>,
    pub text: Option<String>,
    /// It follows the model through references.
    pub associative: bool,
    /// Why a reference no longer resolves: the dimension is drawn at its stored points.
    pub dangling: Option<String>,
}

/// A rendered sheet: everything to draw, in sheet mm (y up), and the placed views.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SheetGeometry {
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
    pub views: Vec<PlacedView>,
    pub dimensions: Vec<DimensionState>,
    /// The parts list's rows, when the sheet shows one.
    pub parts: Vec<PartRow>,
    /// Annotations that lost what they point at (a balloon's component, a centre line's
    /// circle), by id, with the reason.
    pub dangling_annotations: Vec<(u32, String)>,
    /// Which `items` each dimension, annotation and the parts list drew, for picking.
    pub owners: Vec<(Owner, std::ops::Range<usize>)>,
}

/// What a part of a laid-out sheet belongs to, for picking and moving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    View(u32),
    Dimension(u32),
    /// An annotation or the parts list.
    Annotation(u32),
}

impl Owner {
    /// The `drawing.*` parameter that names it (`view`, `dimension`, `annotation`) and
    /// its id.
    pub fn key(self) -> (&'static str, u32) {
        match self {
            Owner::View(id) => ("view", id),
            Owner::Dimension(id) => ("dimension", id),
            Owner::Annotation(id) => ("annotation", id),
        }
    }
}

/// How far `p` is from a drawn shape, sheet mm (0 inside text and arrowheads).
pub fn shape_distance(shape: &Shape, p: Point2) -> f64 {
    match shape {
        Shape::Line { a, b } => {
            let d = *b - *a;
            let len2 = d.dot(d);
            let t = if len2 > 0.0 {
                ((p - *a).dot(d) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            p.distance(*a + d * t)
        }
        Shape::Circle { centre, radius } | Shape::Arc { centre, radius, .. } => {
            (p.distance(*centre) - radius).abs()
        }
        Shape::Fill { points } => {
            let (lo, hi) = points.iter().fold(
                (
                    Point2::new(f64::MAX, f64::MAX),
                    Point2::new(f64::MIN, f64::MIN),
                ),
                |(lo, hi), q| {
                    (
                        Point2::new(lo.x.min(q.x), lo.y.min(q.y)),
                        Point2::new(hi.x.max(q.x), hi.y.max(q.y)),
                    )
                },
            );
            box_distance(p, lo, hi)
        }
        Shape::Text {
            at,
            height,
            angle,
            align,
            text,
        } => {
            // Into the text's own frame: x along the baseline, y up.
            let v = p - *at;
            let (s, c) = angle.sin_cos();
            let local = Point2::new(v.x * c + v.y * s, -v.x * s + v.y * c);
            let w = crate::text::width(text, *height);
            let x0 = match align {
                Align::Left => 0.0,
                Align::Centre => -w * 0.5,
                Align::Right => -w,
            };
            box_distance(local, Point2::new(x0, 0.0), Point2::new(x0 + w, *height))
        }
    }
}

fn box_distance(p: Point2, lo: Point2, hi: Point2) -> f64 {
    let dx = (lo.x - p.x).max(0.0).max(p.x - hi.x);
    let dy = (lo.y - p.y).max(0.0).max(p.y - hi.y);
    dx.hypot(dy)
}

impl SheetGeometry {
    pub fn view(&self, id: u32) -> Option<&PlacedView> {
        self.views.iter().find(|v| v.id == id)
    }

    /// The view whose box (padded by `pad` mm) holds `p`, nearest centre first.
    pub fn view_at(&self, p: Point2, pad: f64) -> Option<&PlacedView> {
        self.views
            .iter()
            .filter(|v| {
                v.bounds().is_some_and(|(lo, hi)| {
                    p.x >= lo.x - pad && p.x <= hi.x + pad && p.y >= lo.y - pad && p.y <= hi.y + pad
                })
            })
            .min_by(|a, b| a.centre.distance(p).total_cmp(&b.centre.distance(p)))
    }

    /// What is at `p` (sheet mm): the dimension or annotation drawn nearest it within
    /// `tolerance` mm, else the view whose box holds it.
    pub fn pick(&self, p: Point2, tolerance: f64) -> Option<Owner> {
        let mut best: Option<(f64, Owner)> = None;
        for (owner, range) in &self.owners {
            for item in self.items.get(range.clone()).unwrap_or_default() {
                let d = shape_distance(&item.shape, p);
                if d <= tolerance && best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, *owner));
                }
            }
        }
        best.map(|b| b.1)
            .or_else(|| self.view_at(p, 0.0).map(|v| Owner::View(v.id)))
    }

    /// The items `owner` drew (a view: none, it is picked by its box).
    pub fn items_of(&self, owner: Owner) -> &[Item] {
        self.owners
            .iter()
            .find(|(o, _)| *o == owner)
            .and_then(|(_, r)| self.items.get(r.clone()))
            .unwrap_or_default()
    }
}

fn line(items: &mut Vec<Item>, layer: Layer, a: Point2, b: Point2) {
    items.push(Item {
        layer,
        shape: Shape::Line { a, b },
    });
}

fn text(items: &mut Vec<Item>, layer: Layer, at: Point2, height: f64, align: Align, s: &str) {
    if s.is_empty() {
        return;
    }
    items.push(Item {
        layer,
        shape: Shape::Text {
            at,
            height,
            angle: 0.0,
            align,
            text: s.to_owned(),
        },
    });
}

fn rect(items: &mut Vec<Item>, layer: Layer, lo: Point2, hi: Point2) {
    let (a, b, c, d) = (lo, Point2::new(hi.x, lo.y), hi, Point2::new(lo.x, hi.y));
    line(items, layer, a, b);
    line(items, layer, b, c);
    line(items, layer, c, d);
    line(items, layer, d, a);
}

/// `1:1`, `1:2`, `2:1`, `1:2.5`.
pub fn scale_text(scale: f64) -> String {
    let n = |v: f64| {
        let s = format!("{:.3}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    if scale >= 1.0 {
        format!("{}:1", n(scale))
    } else {
        format!("1:{}", n(1.0 / scale))
    }
}

/// The frame and the title block.
fn frame(sheet: &Sheet, w: f64, h: f64, items: &mut Vec<Item>) {
    let lo = Point2::new(MARGIN_LEFT, MARGIN);
    let hi = Point2::new(w - MARGIN, h - MARGIN);
    rect(items, Layer::Border, lo, hi);
    let tw = TITLE_WIDTH.min(hi.x - lo.x);
    let t_lo = Point2::new(hi.x - tw, lo.y);
    let t_hi = Point2::new(hi.x, lo.y + TITLE_HEIGHT);
    rect(items, Layer::Border, t_lo, t_hi);
    // Columns: the projection symbol, then the fields.
    let sym_w = tw * 0.2;
    let x1 = t_lo.x + sym_w;
    line(
        items,
        Layer::Border,
        Point2::new(x1, t_lo.y),
        Point2::new(x1, t_hi.y),
    );
    let mid_y = t_lo.y + 14.0;
    line(
        items,
        Layer::Border,
        Point2::new(x1, mid_y),
        Point2::new(t_hi.x, mid_y),
    );
    let cell = (t_hi.x - x1) / 4.0;
    for k in 1..4 {
        let x = x1 + cell * f64::from(k);
        line(
            items,
            Layer::Border,
            Point2::new(x, t_lo.y),
            Point2::new(x, mid_y),
        );
    }
    let label = 1.8;
    let value = 3.5;
    let title = &sheet.title;
    let part = if title.part_name.is_empty() {
        clean(&sheet.name, 64)
    } else {
        clean(&title.part_name, 64)
    };
    text(
        items,
        Layer::Border,
        Point2::new(x1 + 1.5, t_hi.y - 3.0),
        label,
        Align::Left,
        "PART",
    );
    text(
        items,
        Layer::Border,
        Point2::new(t_hi.x - 1.5, t_hi.y - 3.0),
        label,
        Align::Right,
        "CHAMFERA",
    );
    text(
        items,
        Layer::Border,
        Point2::new(x1 + 1.5, mid_y + 3.0),
        5.0,
        Align::Left,
        &part,
    );
    let fields = [
        ("SCALE", scale_text(sheet.scale)),
        ("UNITS", "mm".to_owned()),
        ("DATE", clean(&title.date, 64)),
        ("DRAWN BY", clean(&title.drawn_by, 64)),
    ];
    for (k, (name, v)) in fields.iter().enumerate() {
        let x = x1 + cell * k as f64;
        text(
            items,
            Layer::Border,
            Point2::new(x + 1.5, mid_y - 3.0),
            label,
            Align::Left,
            name,
        );
        // Long values shrink to fit their cell.
        let mut hv = value;
        let room = cell - 3.0;
        let wv = width(v, hv);
        if wv > room && wv > 0.0 {
            hv = (hv * room / wv).max(1.0);
        }
        text(
            items,
            Layer::Border,
            Point2::new(x + 1.5, t_lo.y + 3.0),
            hv,
            Align::Left,
            v,
        );
    }
    // The projection symbol (ISO 128-30): a truncated cone, small end left, and its end
    // view as two circles, on the left for third angle and on the right for first angle.
    let (name, third) = match sheet.projection {
        ProjectionAngle::Third => ("THIRD ANGLE", true),
        ProjectionAngle::First => ("FIRST ANGLE", false),
    };
    let cx = t_lo.x + sym_w * 0.5;
    let cy = t_lo.y + 17.0;
    text(
        items,
        Layer::Border,
        Point2::new(cx, t_lo.y + 3.0),
        label,
        Align::Centre,
        name,
    );
    let r = 5.0;
    let gap = 3.0;
    let total = 2.0 * r + gap + 2.0 * r;
    let x0 = cx - total * 0.5;
    let (circle_x, cone_x) = if third {
        (x0 + r, x0 + 2.0 * r + gap)
    } else {
        (x0 + 2.0 * r + r + gap, x0)
    };
    for radius in [r, r * 0.5] {
        items.push(Item {
            layer: Layer::Border,
            shape: Shape::Circle {
                centre: Point2::new(circle_x, cy),
                radius,
            },
        });
    }
    let len = 2.0 * r;
    let corners = [
        Point2::new(cone_x, cy - r * 0.5),
        Point2::new(cone_x + len, cy - r),
        Point2::new(cone_x + len, cy + r),
        Point2::new(cone_x, cy + r * 0.5),
    ];
    for k in 0..4 {
        line(items, Layer::Border, corners[k], corners[(k + 1) % 4]);
    }
    let ext = 1.5;
    line(
        items,
        Layer::Center,
        Point2::new(x0 - ext, cy),
        Point2::new(x0 + total + ext, cy),
    );
    line(
        items,
        Layer::Center,
        Point2::new(circle_x, cy - r - ext),
        Point2::new(circle_x, cy + r + ext),
    );
}

/// The position of each view on the sheet: a projected view sits beside or above its
/// base, on the side the projection angle says, keeping its distance from it.
fn placement(sheet: &Sheet, v: &SheetView) -> (Point2, f64) {
    let own = Point2::new(v.x, v.y);
    let Some(base) = v.parent.and_then(|p| sheet.view(p)) else {
        return (own, v.scale.unwrap_or(sheet.scale));
    };
    let scale = base.scale.unwrap_or(sheet.scale);
    let at = Point2::new(base.x, base.y);
    let bp = Basis::of(base.direction);
    let bc = Basis::of(v.direction);
    let sx = bc.toward.dot(bp.right);
    let sy = bc.toward.dot(bp.up);
    let flip = match sheet.projection {
        ProjectionAngle::Third => 1.0,
        ProjectionAngle::First => -1.0,
    };
    let dist = |d: f64| if d.abs() < 1.0 { DEFAULT_GAP } else { d.abs() };
    if sx.abs() > 0.5 {
        let x = at.x + sx.signum() * flip * dist(v.x - at.x);
        (Point2::new(x, at.y), scale)
    } else if sy.abs() > 0.5 {
        let y = at.y + sy.signum() * flip * dist(v.y - at.y);
        (Point2::new(at.x, y), scale)
    } else {
        (own, scale)
    }
}

/// True when a view seen from `child` can be projected from a base seen from `base`:
/// perpendicular directions sharing a paper axis.
pub fn projects_from(base: ViewDirection, child: ViewDirection) -> bool {
    if base == ViewDirection::Iso || child == ViewDirection::Iso {
        return false;
    }
    let bp = Basis::of(base);
    let bc = Basis::of(child);
    bc.toward.dot(bp.toward).abs() < 1e-9
        && (bc.toward.dot(bp.right).abs() > 0.5 || bc.toward.dot(bp.up).abs() > 0.5)
}

/// The bodies a view shows: some of the model's, or the pieces a section cut left.
pub enum Shown<'a> {
    Given(Vec<&'a Body>),
    Cut(Vec<Body>),
}

impl Shown<'_> {
    pub fn bodies(&self) -> Vec<&Body> {
        match self {
            Shown::Given(v) => v.clone(),
            Shown::Cut(v) => v.iter().collect(),
        }
    }
}

/// The model's bodies a view names (all of them for `None`).
fn chosen<'a>(body: Option<&str>, bodies: &[(&str, &'a Body)]) -> Result<Vec<&'a Body>, String> {
    let chosen: Vec<&Body> = match body {
        None => bodies.iter().map(|(_, b)| *b).collect(),
        Some(name) => bodies
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, b)| *b)
            .collect(),
    };
    if chosen.is_empty() {
        return Err(match body {
            Some(n) => format!("no solid named {n:?}"),
            None => "there are no solids".to_owned(),
        });
    }
    Ok(chosen)
}

/// What view `id` shows and along which axes: its bodies (a section's cut), its basis and,
/// for a section, the cut. A detail shows what its source shows.
pub fn shown<'a>(
    sheet: &Sheet,
    id: u32,
    bodies: &[(&str, &'a Body)],
) -> Result<(Shown<'a>, Basis, Option<section::Cut>), String> {
    let v = sheet.view(id).ok_or_else(|| format!("no view {id}"))?;
    if let Some(d) = &v.detail {
        let src = sheet
            .view(d.from)
            .ok_or_else(|| format!("no view {}", d.from))?;
        if src.detail.is_some() {
            return Err("a detail of a detail".into());
        }
        return shown(sheet, d.from, bodies);
    }
    if let Some(c) = &v.section {
        let src = sheet
            .view(c.from)
            .ok_or_else(|| format!("no view {}", c.from))?;
        if !src.is_plain() {
            return Err("a section is cut on a plain view".into());
        }
        if src.direction == ViewDirection::Iso {
            return Err("a section is cut on an orthographic view, not the isometric one".into());
        }
        let parent = Basis::of(src.direction);
        let cut = section::cut_of(&parent, c).ok_or("the cutting line has no length")?;
        let given = chosen(src.body.as_deref(), bodies)?;
        let pieces = section::cut_bodies(&given, &cut.plane)?;
        return Ok((Shown::Cut(pieces), cut.basis, Some(cut)));
    }
    Ok((
        Shown::Given(chosen(v.body.as_deref(), bodies)?),
        Basis::of(v.direction),
        None,
    ))
}

/// The model references for a new dimension on view `view` (see `refs::make_refs`).
pub fn dimension_refs(
    sheet: &Sheet,
    bodies: &[(&str, &Body)],
    view: u32,
    kind: chamfera_doc::drawing::DimensionKind,
    points: &[Point2],
) -> Vec<chamfera_doc::drawing::ModelRef> {
    match shown(sheet, view, bodies) {
        Ok((s, basis, _)) => refs::make_refs(kind, points, &basis, &s.bodies()),
        Err(_) => Vec::new(),
    }
}

/// The model references for a centre line between two circles of view `view`.
pub fn centre_line_refs(
    sheet: &Sheet,
    bodies: &[(&str, &Body)],
    view: u32,
    a: (Point2, f64),
    b: (Point2, f64),
) -> Vec<chamfera_doc::drawing::ModelRef> {
    match shown(sheet, view, bodies) {
        Ok((s, basis, _)) => refs::centre_refs(a, b, &basis, &s.bodies()),
        Err(_) => Vec::new(),
    }
}

fn centre_of(bodies: &[&Body]) -> Point3 {
    let mut bbox: Option<chamfera_geom::Aabb> = None;
    for b in bodies {
        if let Some(x) = b.bounding_box() {
            bbox = Some(match bbox {
                None => x,
                Some(o) => o.union(x),
            });
        }
    }
    bbox.map_or(Point3::new(0.0, 0.0, 0.0), |b| b.center())
}

/// Lays out `sheet` with the solids `bodies` (by name). A view whose body is missing or
/// cannot be projected stays empty, with its [`PlacedView::problem`] set and a note on
/// the sheet; the rest of the sheet is still drawn.
pub fn render_sheet(sheet: &Sheet, bodies: &[(&str, &Body)]) -> DrawingResult<SheetGeometry> {
    render_with_parts(sheet, bodies, &[])
}

/// [`render_sheet`] with the design's parts, for the parts list and balloons.
pub fn render_with_parts(
    sheet: &Sheet,
    bodies: &[(&str, &Body)],
    parts: &[PartRow],
) -> DrawingResult<SheetGeometry> {
    let (w, h) = sheet.size();
    let mut out = SheetGeometry {
        width: w,
        height: h,
        ..Default::default()
    };
    frame(sheet, w, h, &mut out.items);
    let mut cache: HashMap<(Option<String>, ViewDirection), Result<ViewGeometry, String>> =
        HashMap::new();
    let mut models: HashMap<u32, (Shown, Basis, Option<section::Cut>)> = HashMap::new();
    let mut placed: HashMap<u32, PlacedView> = HashMap::new();
    let mut extra: Vec<Item> = Vec::new();
    // Plain views first, then sections (cut on plain views), then details (of either).
    for pass in 0..3 {
        for v in &sheet.views {
            let p = match (pass, v.section.is_some(), v.detail.is_some()) {
                (0, false, false) => plain_view(sheet, v, bodies, &mut cache, &mut models),
                (1, true, _) => section_view(sheet, v, bodies, &placed, &mut models, &mut extra),
                (2, _, true) => detail_view(v, &placed, &mut extra),
                _ => continue,
            };
            placed.insert(v.id, p);
        }
    }
    for v in &sheet.views {
        if let Some(p) = placed.remove(&v.id) {
            view_items(&p, &mut out.items);
            out.views.push(p);
        }
    }
    out.items.extend(extra);
    for d in &sheet.dimensions {
        let Some(v) = out.views.iter().find(|v| v.id == d.view) else {
            continue;
        };
        let model = match models.get(&d.view) {
            Some(m) => Ok(m),
            None => sheet
                .view(d.view)
                .and_then(|x| x.detail.as_ref())
                .and_then(|dd| models.get(&dd.from))
                .ok_or_else(|| "its view shows no model".to_owned()),
        };
        let resolved = match (&v.problem, model) {
            (Some(p), _) => Err(p.clone()),
            (None, Err(e)) => Err(e),
            (None, Ok((s, basis, _))) => {
                refs::resolve_points(d.kind, &d.points, &d.refs, basis, &s.bodies())
            }
        };
        let (points, dangling) = match resolved {
            Ok(p) => (p, None),
            Err(e) => (d.points.clone(), Some(e)),
        };
        let shown_d = SheetDimension {
            points: points.clone(),
            ..d.clone()
        };
        let mut items = Vec::new();
        dims::draw(&shown_d, v, &mut items);
        if dangling.is_some() {
            for i in &mut items {
                i.layer = Layer::Dangling;
            }
        }
        let start = out.items.len();
        out.items.extend(items);
        out.owners
            .push((Owner::Dimension(d.id), start..out.items.len()));
        let value = dims::measure(d.kind, &points);
        out.dimensions.push(DimensionState {
            id: d.id,
            view: d.view,
            value,
            text: value.map(|x| dims::format_value(d.kind, x)),
            associative: !d.refs.is_empty(),
            dangling,
        });
    }
    let views = std::mem::take(&mut out.views);
    annotations(sheet, &views, &models, parts, &mut out);
    out.views = views;
    if let Some(pl) = &sheet.parts_list {
        let start = out.items.len();
        annot::parts_list(&mut out.items, Point2::new(pl.x, pl.y), parts);
        out.owners
            .push((Owner::Annotation(pl.id), start..out.items.len()));
        out.parts = parts.to_vec();
    }
    Ok(out)
}

fn plain_view<'a>(
    sheet: &Sheet,
    v: &SheetView,
    bodies: &[(&str, &'a Body)],
    cache: &mut HashMap<(Option<String>, ViewDirection), Result<ViewGeometry, String>>,
    models: &mut HashMap<u32, (Shown<'a>, Basis, Option<section::Cut>)>,
) -> PlacedView {
    let (centre, scale) = placement(sheet, v);
    let basis = Basis::of(v.direction);
    let empty = |problem: String| PlacedView {
        id: v.id,
        direction: v.direction,
        basis,
        scale,
        centre,
        model_centre: Point2::new(0.0, 0.0),
        geometry: ViewGeometry::default(),
        clip: None,
        section: false,
        problem: Some(problem),
    };
    let given = match chosen(v.body.as_deref(), bodies) {
        Ok(g) => g,
        Err(e) => return empty(e),
    };
    let geometry = cache
        .entry((v.body.clone(), v.direction))
        .or_insert_with(|| project(&given, v.direction).map_err(|e| e.to_string()))
        .clone();
    let c3 = centre_of(&given);
    models.insert(v.id, (Shown::Given(given), basis, None));
    match geometry {
        Ok(geometry) => keep_on_sheet(
            sheet,
            v,
            PlacedView {
                id: v.id,
                direction: v.direction,
                basis,
                scale,
                centre,
                model_centre: basis.project(c3),
                geometry,
                clip: None,
                section: false,
                problem: None,
            },
        ),
        Err(e) => empty(e),
    }
}

/// Moves a projected view toward its base, along the axis it is projected on, just far
/// enough that its box stays inside the sheet's border (first angle mirrors a view's
/// distance to the other side, which could put a view placed for third angle off the
/// sheet). Its alignment with the base is kept; a view larger than the sheet stays put.
fn keep_on_sheet(sheet: &Sheet, v: &SheetView, mut p: PlacedView) -> PlacedView {
    let Some(base) = v.parent.and_then(|id| sheet.view(id)) else {
        return p;
    };
    let Some((lo, hi)) = p.bounds() else {
        return p;
    };
    let (w, h) = sheet.size();
    let base_at = placement(sheet, base).0;
    // The shift that brings [lo, hi] inside [min, max], when it fits.
    let shift = |lo: f64, hi: f64, min: f64, max: f64| -> f64 {
        if hi - lo > max - min {
            0.0
        } else if lo < min {
            min - lo
        } else if hi > max {
            max - hi
        } else {
            0.0
        }
    };
    if (p.centre.x - base_at.x).abs() < chamfera_geom::LINEAR_TOLERANCE {
        p.centre.y += shift(lo.y, hi.y, MARGIN, h - MARGIN);
    } else {
        p.centre.x += shift(lo.x, hi.x, MARGIN_LEFT, w - MARGIN);
    }
    p
}

fn section_view<'a>(
    sheet: &Sheet,
    v: &SheetView,
    bodies: &[(&str, &'a Body)],
    placed: &HashMap<u32, PlacedView>,
    models: &mut HashMap<u32, (Shown<'a>, Basis, Option<section::Cut>)>,
    extra: &mut Vec<Item>,
) -> PlacedView {
    let centre = Point2::new(v.x, v.y);
    let src = v
        .section
        .as_ref()
        .and_then(|c| placed.get(&c.from).map(|p| (c, p)));
    let scale = v.scale.or(src.map(|(_, p)| p.scale)).unwrap_or(sheet.scale);
    let empty = |problem: String, basis: Basis| PlacedView {
        id: v.id,
        direction: v.direction,
        basis,
        scale,
        centre,
        model_centre: Point2::new(0.0, 0.0),
        geometry: ViewGeometry::default(),
        clip: None,
        section: true,
        problem: Some(problem),
    };
    let fallback = Basis::of(v.direction);
    let Some((cut, src)) = src else {
        return empty("its cutting line's view is missing".into(), fallback);
    };
    let (shown_bodies, basis, c3d) = match shown(sheet, v.id, bodies) {
        Ok(x) => x,
        Err(e) => return empty(e, fallback),
    };
    if let Some(c) = &c3d {
        section::cutting_line(
            src.to_sheet(cut.a),
            src.to_sheet(cut.b),
            c.sight,
            &clean(&cut.label, 3),
            extra,
        );
    }
    let list = shown_bodies.bodies();
    let projected = project_basis(&list, basis).map(|mut g| {
        // A section shows no hidden lines (ISO 128-44 practice).
        g.curves.retain(|c| !c.hidden);
        let (parts, regions): (Vec<usize>, Vec<_>) =
            section::regions(&list, &basis).into_iter().unzip();
        g.regions = regions;
        g.region_parts = parts;
        g
    });
    let c3 = centre_of(&list);
    drop(list);
    models.insert(v.id, (shown_bodies, basis, c3d));
    let geometry = match projected {
        Ok(g) => g,
        Err(e) => return empty(e.to_string(), basis),
    };
    let p = PlacedView {
        id: v.id,
        direction: v.direction,
        basis,
        scale,
        centre,
        model_centre: basis.project(c3),
        geometry,
        clip: None,
        section: true,
        problem: None,
    };
    if let Some((_, lo)) = p.bounds().map(|(lo, hi)| (hi, lo)) {
        let l = clean(&cut.label, 3);
        text(
            extra,
            Layer::Dim,
            Point2::new(centre.x, lo.y - 10.0),
            5.0,
            Align::Centre,
            &format!("SECTION {l}-{l}"),
        );
    }
    p
}

fn detail_view(
    v: &SheetView,
    placed: &HashMap<u32, PlacedView>,
    extra: &mut Vec<Item>,
) -> PlacedView {
    let centre = Point2::new(v.x, v.y);
    let src = v
        .detail
        .as_ref()
        .and_then(|d| placed.get(&d.from).map(|p| (d, p)));
    let Some((d, src)) = src else {
        return PlacedView {
            id: v.id,
            direction: v.direction,
            basis: Basis::of(v.direction),
            scale: v.scale.unwrap_or(1.0),
            centre,
            model_centre: Point2::new(0.0, 0.0),
            geometry: ViewGeometry::default(),
            clip: None,
            section: false,
            problem: Some("the view it enlarges is missing".into()),
        };
    };
    let scale = v.scale.unwrap_or(src.scale * 2.0);
    let r = d.radius * scale;
    let label = clean(&d.label, 3);
    // On the source view: the circle, narrow and continuous, and its letter outside it.
    let at = src.to_sheet(d.centre);
    let rs = d.radius * src.scale;
    extra.push(Item {
        layer: Layer::Dim,
        shape: Shape::Circle {
            centre: at,
            radius: rs,
        },
    });
    let k = std::f64::consts::FRAC_1_SQRT_2;
    text(
        extra,
        Layer::Dim,
        Point2::new(at.x + (rs + 2.0) * k, at.y + (rs + 2.0) * k),
        5.0,
        Align::Left,
        &label,
    );
    // On the detail: its boundary and its label.
    extra.push(Item {
        layer: Layer::Dim,
        shape: Shape::Circle { centre, radius: r },
    });
    text(
        extra,
        Layer::Dim,
        Point2::new(centre.x, centre.y - r - 9.0),
        5.0,
        Align::Centre,
        &format!("DETAIL {label} ({})", scale_text(scale)),
    );
    PlacedView {
        id: v.id,
        direction: src.direction,
        basis: src.basis,
        scale,
        centre,
        model_centre: d.centre,
        geometry: src.geometry.clone(),
        clip: Some((centre, r)),
        section: src.section,
        problem: src.problem.clone(),
    }
}

/// The notes, leaders, symbols, frames, centre lines and balloons of a sheet.
fn annotations(
    sheet: &Sheet,
    views: &[PlacedView],
    models: &HashMap<u32, (Shown, Basis, Option<section::Cut>)>,
    parts: &[PartRow],
    out: &mut SheetGeometry,
) {
    let mut owners = Vec::new();
    let items = &mut out.items;
    for a in &sheet.annotations {
        let start = items.len();
        match &a.kind {
            AnnotationKind::Note { at, text, height } => annot::note(items, *at, text, *height),
            AnnotationKind::Leader { points, text } => annot::leader(items, points, text),
            AnnotationKind::SurfaceFinish { at, text, removal } => {
                annot::surface_finish(items, *at, text, *removal);
            }
            AnnotationKind::Gdt {
                at,
                characteristic,
                tolerance,
                diameter,
                datums,
                leader,
            } => annot::gdt(
                items,
                *at,
                *characteristic,
                *tolerance,
                *diameter,
                datums,
                *leader,
            ),
            AnnotationKind::CentreLine {
                view,
                points,
                refs: r,
            } => {
                let Some(v) = views.iter().find(|x| x.id == *view) else {
                    continue;
                };
                let model = models.get(view).or_else(|| {
                    sheet
                        .view(*view)
                        .and_then(|x| x.detail.as_ref())
                        .and_then(|d| models.get(&d.from))
                });
                let now = match model {
                    Some((s, basis, _)) => refs::resolve_centres(points, r, basis, &s.bodies()),
                    None => Err("its view shows no model".into()),
                };
                let (pts, layer) = match now {
                    Ok(p) => (p, Layer::Center),
                    Err(e) => {
                        out.dangling_annotations.push((a.id, e));
                        (*points, Layer::Dangling)
                    }
                };
                annot::centre_line(items, v.to_sheet(pts[0]), v.to_sheet(pts[1]), layer);
            }
            AnnotationKind::Balloon {
                view,
                component,
                at,
                target,
            } => {
                let Some(v) = views.iter().find(|x| x.id == *view) else {
                    continue;
                };
                let item = parts.iter().position(|r| r.components.contains(component));
                let (number, layer) = match item {
                    Some(i) => ((i + 1).to_string(), Layer::Dim),
                    None => {
                        out.dangling_annotations.push((
                            a.id,
                            format!("component {component} is not in the parts list"),
                        ));
                        ("?".to_owned(), Layer::Dangling)
                    }
                };
                annot::balloon(items, *at, v.to_sheet(*target), &number, layer);
            }
        }
        owners.push((Owner::Annotation(a.id), start..items.len()));
    }
    out.owners.extend(owners);
}

/// Clips a sheet line to the view's detail circle, if it has one.
fn clipped(v: &PlacedView, a: Point2, b: Point2) -> Option<[Point2; 2]> {
    match v.clip {
        None => Some([a, b]),
        Some((c, r)) => annot::clip_line(a, b, c, r),
    }
}

/// The lines, centre marks, centre lines and hatching of a placed view.
fn view_items(v: &PlacedView, items: &mut Vec<Item>) {
    if let Some(p) = &v.problem {
        text(
            items,
            Layer::Dim,
            v.centre,
            3.5,
            Align::Centre,
            &clean(&format!("View {}: {p}", v.id), 120),
        );
        return;
    }
    // Pictorial views show no hidden lines and no centre lines (ISO 128-30 practice).
    let pictorial = v.direction == ViewDirection::Iso && !v.section;
    let push_line = |items: &mut Vec<Item>, layer: Layer, a: Point2, b: Point2| {
        if let Some([a, b]) = clipped(v, a, b) {
            line(items, layer, a, b);
        }
    };
    for c in &v.geometry.curves {
        if c.hidden && pictorial {
            continue;
        }
        let layer = if c.hidden {
            Layer::Hidden
        } else {
            Layer::Visible
        };
        match c.curve {
            Curve2::Line { a, b } => push_line(items, layer, v.to_sheet(a), v.to_sheet(b)),
            Curve2::Circle { centre, radius } => {
                let (k, r) = (v.to_sheet(centre), radius * v.scale);
                match v.clip {
                    None => items.push(Item {
                        layer,
                        shape: Shape::Circle {
                            centre: k,
                            radius: r,
                        },
                    }),
                    Some((cc, cr)) => {
                        for (s, e) in annot::clip_arc(k, r, 0.0, std::f64::consts::TAU, cc, cr) {
                            items.push(arc_item(layer, k, r, s, e));
                        }
                    }
                }
            }
            Curve2::Arc {
                centre,
                radius,
                start,
                end,
            } => {
                let (k, r) = (v.to_sheet(centre), radius * v.scale);
                match v.clip {
                    None => items.push(arc_item(layer, k, r, start, end)),
                    Some((cc, cr)) => {
                        for (s, e) in annot::clip_arc(k, r, start, end, cc, cr) {
                            items.push(arc_item(layer, k, r, s, e));
                        }
                    }
                }
            }
        }
    }
    let spacing = section::hatch_spacing(v.scale);
    for (k, region) in v.geometry.regions.iter().enumerate() {
        let part = v.geometry.region_parts.get(k).copied().unwrap_or(0);
        let loops: Vec<Vec<Point2>> = region
            .iter()
            .map(|l| l.iter().map(|p| v.to_sheet(*p)).collect())
            .collect();
        for [a, b] in section::hatch(&loops, spacing, section::hatch_angle(part)) {
            push_line(items, Layer::Hatch, a, b);
        }
    }
    if pictorial {
        return;
    }
    for (c, r) in &v.geometry.centre_marks {
        let c = v.to_sheet(*c);
        let reach = r * v.scale + CENTRE_OVERSHOOT;
        push_line(
            items,
            Layer::Center,
            Point2::new(c.x - reach, c.y),
            Point2::new(c.x + reach, c.y),
        );
        push_line(
            items,
            Layer::Center,
            Point2::new(c.x, c.y - reach),
            Point2::new(c.x, c.y + reach),
        );
    }
    for [a, b] in &v.geometry.centre_lines {
        let (a, b) = (v.to_sheet(*a), v.to_sheet(*b));
        let d = b - a;
        let len = d.length();
        if len <= 0.0 {
            continue;
        }
        let u = d * (CENTRE_OVERSHOOT / len);
        push_line(items, Layer::Center, a - u, b + u);
    }
}

fn arc_item(layer: Layer, centre: Point2, radius: f64, start: f64, end: f64) -> Item {
    Item {
        layer,
        shape: Shape::Arc {
            centre,
            radius,
            start,
            end,
        },
    }
}
