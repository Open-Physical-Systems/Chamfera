//! A sheet laid out on paper: border, title block, views and dimensions.
//!
//! The title block is Chamfera's own simple design: a projection-symbol cell on the left
//! (the ISO 128 truncated-cone symbol for first- or third-angle), then the part name
//! across the top and scale, units, date and drawer below.

use crate::hlr::{Basis, Curve2, ViewGeometry, project};
use crate::text::{clean, width};
use crate::{Align, DrawingResult, Item, Layer, Shape, dims};
use chamfera_doc::drawing::{ProjectionAngle, Sheet, SheetView, ViewDirection};
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
    /// Sheet mm per model mm.
    pub scale: f64,
    /// Where the model's centre goes on the sheet.
    pub centre: Point2,
    /// The model's centre in view coordinates.
    pub model_centre: Point2,
    /// The projected lines, in view coordinates (model mm).
    pub geometry: ViewGeometry,
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
        let (lo, hi) = self.geometry.bounds()?;
        Some((self.to_sheet(lo), self.to_sheet(hi)))
    }
}

/// A rendered sheet: everything to draw, in sheet mm (y up), and the placed views.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SheetGeometry {
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
    pub views: Vec<PlacedView>,
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

type Projection = Result<(ViewGeometry, Point3), String>;

/// Projects the bodies a view shows (all of them for `None`).
fn project_bodies(
    body: Option<&str>,
    direction: ViewDirection,
    bodies: &[(&str, &Body)],
) -> Projection {
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
    let mut bbox: Option<chamfera_geom::Aabb> = None;
    for b in &chosen {
        if let Some(x) = b.bounding_box() {
            bbox = Some(match bbox {
                None => x,
                Some(o) => o.union(x),
            });
        }
    }
    let centre = bbox.map_or(Point3::new(0.0, 0.0, 0.0), |b| b.center());
    let g = project(&chosen, direction).map_err(|e| e.to_string())?;
    Ok((g, centre))
}

/// Lays out `sheet` with the solids `bodies` (by name). A view whose body is missing or
/// cannot be projected stays empty, with its [`PlacedView::problem`] set and a note on
/// the sheet; the rest of the sheet is still drawn.
pub fn render_sheet(sheet: &Sheet, bodies: &[(&str, &Body)]) -> DrawingResult<SheetGeometry> {
    let (w, h) = sheet.size();
    let mut out = SheetGeometry {
        width: w,
        height: h,
        ..Default::default()
    };
    frame(sheet, w, h, &mut out.items);
    let mut cache: HashMap<(Option<String>, ViewDirection), Projection> = HashMap::new();
    for v in &sheet.views {
        let key = (v.body.clone(), v.direction);
        let projected = cache
            .entry(key)
            .or_insert_with(|| project_bodies(v.body.as_deref(), v.direction, bodies))
            .clone();
        let (centre, scale) = placement(sheet, v);
        let basis = Basis::of(v.direction);
        let placed = match projected {
            Ok((geometry, c3)) => PlacedView {
                id: v.id,
                direction: v.direction,
                scale,
                centre,
                model_centre: basis.project(c3),
                geometry,
                problem: None,
            },
            Err(problem) => PlacedView {
                id: v.id,
                direction: v.direction,
                scale,
                centre,
                model_centre: Point2::new(0.0, 0.0),
                geometry: ViewGeometry::default(),
                problem: Some(problem),
            },
        };
        view_items(&placed, &mut out.items);
        out.views.push(placed);
    }
    for d in &sheet.dimensions {
        if let Some(v) = out.views.iter().find(|v| v.id == d.view) {
            dims::draw(d, v, &mut out.items);
        }
    }
    Ok(out)
}

/// The lines, centre marks and centre lines of a placed view.
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
    let pictorial = v.direction == ViewDirection::Iso;
    for c in &v.geometry.curves {
        if c.hidden && pictorial {
            continue;
        }
        let layer = if c.hidden {
            Layer::Hidden
        } else {
            Layer::Visible
        };
        let shape = match c.curve {
            Curve2::Line { a, b } => Shape::Line {
                a: v.to_sheet(a),
                b: v.to_sheet(b),
            },
            Curve2::Circle { centre, radius } => Shape::Circle {
                centre: v.to_sheet(centre),
                radius: radius * v.scale,
            },
            Curve2::Arc {
                centre,
                radius,
                start,
                end,
            } => Shape::Arc {
                centre: v.to_sheet(centre),
                radius: radius * v.scale,
                start,
                end,
            },
        };
        items.push(Item { layer, shape });
    }
    if pictorial {
        return;
    }
    for (c, r) in &v.geometry.centre_marks {
        let c = v.to_sheet(*c);
        let reach = r * v.scale + CENTRE_OVERSHOOT;
        line(
            items,
            Layer::Center,
            Point2::new(c.x - reach, c.y),
            Point2::new(c.x + reach, c.y),
        );
        line(
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
        line(items, Layer::Center, a - u, b + u);
    }
}
