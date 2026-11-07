//! `drawing.add_ordinate`, `drawing.add_baseline` and `drawing.add_chain`: sets of
//! dimensions made in one step (one undo step), each an ordinary associative dimension
//! afterwards. The picks are snapped as `drawing.add_dimension` snaps them, so a set is
//! ordered by where its points really are, not where the clicks fell.

use super::{bad, coordinate, opt_id, push_dimensions, render, sheet_of_view};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::drawing::{self as dd, DimensionKind};
use chamfera_geom::Point2;
use serde_json::{Value, json};

/// Most points one set may take.
const MAX_SET_POINTS: usize = 64;

fn view_sheet(s: &Session, p: &Value) -> Result<(u32, u32), CommandError> {
    let view = params::req_id(p, "view")?;
    let sheet = match opt_id(p, "sheet")? {
        Some(id) => {
            if s.doc.drawing.sheet(id)?.view(view).is_none() {
                return Err(bad(format!("no view {view} on sheet {id}")));
            }
            id
        }
        None => sheet_of_view(s, view)?,
    };
    Ok((view, sheet))
}

/// `points`: a list of `{x, y}` sheet points, `min` to [`MAX_SET_POINTS`] of them.
fn points(p: &Value, min: usize) -> Result<Vec<Point2>, CommandError> {
    let list = match p.get("points") {
        None | Some(Value::Null) if min == 0 => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        _ => return Err(bad("`points` must be a list of {x, y} sheet points")),
    };
    if list.len() < min || list.len() > MAX_SET_POINTS {
        return Err(bad(format!(
            "`points` holds {min} to {MAX_SET_POINTS} points"
        )));
    }
    list.iter()
        .map(|q| Ok(Point2::new(coordinate(q, "x")?, coordinate(q, "y")?)))
        .collect()
}

fn offset(p: &Value, key: &str, default: f64) -> Result<f64, CommandError> {
    match params::opt_f64(p, key)? {
        None => Ok(default),
        Some(v) if dd::is_valid_sheet_coordinate(v) => Ok(v),
        Some(_) => Err(bad(format!("`{key}` must be a finite number of mm"))),
    }
}

/// `kind`: horizontal or vertical.
fn linear_kind(p: &Value) -> Result<DimensionKind, CommandError> {
    match params::req_str(p, "kind")? {
        "horizontal" => Ok(DimensionKind::Horizontal),
        "vertical" => Ok(DimensionKind::Vertical),
        k => Err(bad(format!(
            "unknown kind {k:?}: a baseline or chain is horizontal or vertical"
        ))),
    }
}

/// Where each pick snaps to on the view (sheet mm), as `drawing.add_dimension` would
/// snap it; a pick that snaps to nothing is refused by name.
fn snapped(
    s: &Session,
    sheet: u32,
    view: u32,
    picks: &[Point2],
) -> Result<Vec<Point2>, CommandError> {
    let g = render(s, sheet)?;
    let v = g
        .view(view)
        .ok_or_else(|| bad(format!("no view {view} on sheet {sheet}")))?;
    picks
        .iter()
        .map(|q| {
            // A zero-length ordinate from the pick to itself snaps it.
            chamfera_drawing::plan_dimension(v, DimensionKind::OrdinateX, &[*q, *q])
                .ok()
                .and_then(|plan| plan.points.first().map(|x| v.to_sheet(*x)))
                .ok_or_else(|| {
                    CommandError::Unavailable(format!(
                        "no vertex or edge of view {view} near ({:.2}, {:.2})",
                        q.x, q.y
                    ))
                })
        })
        .collect()
}

pub(super) fn add_ordinate(s: &mut Session, p: &Value) -> CmdResult {
    let (view, sheet) = view_sheet(s, p)?;
    let kind = match params::req_str(p, "axis")? {
        "x" => DimensionKind::OrdinateX,
        "y" => DimensionKind::OrdinateY,
        a => return Err(bad(format!("unknown axis {a:?}: use x or y"))),
    };
    let origin = Point2::new(coordinate(p, "x0")?, coordinate(p, "y0")?);
    let rest = points(p, 0)?;
    let off = offset(p, "offset", -10.0)?;
    let mut specs = vec![(kind, vec![origin, origin], off)];
    specs.extend(rest.into_iter().map(|q| (kind, vec![origin, q], off)));
    let made = push_dimensions(s, sheet, view, &specs)?;
    Ok(json!({ "sheet": sheet, "dimensions": made }))
}

/// A set of linear dimensions of `kind` from picks: `pairs` gives, for the snapped
/// points sorted along the axis, which two each dimension measures and how many
/// spacings out its line goes. Every line is placed from the outermost point of the
/// whole set (a dimension's own line sits `offset` beyond the outer of its own two
/// points), so a chain stays on one line and a baseline's lines are evenly stacked.
fn linear_set(
    s: &mut Session,
    p: &Value,
    pairs: fn(usize) -> Vec<(usize, usize, f64)>,
) -> CmdResult {
    let (view, sheet) = view_sheet(s, p)?;
    let kind = linear_kind(p)?;
    let picks = points(p, 2)?;
    let off = offset(p, "offset", 10.0)?;
    let spacing = match params::opt_f64(p, "spacing")? {
        None => 7.0,
        Some(v) if v.is_finite() && (2.0..=50.0).contains(&v) => v,
        Some(_) => return Err(bad("`spacing` must be 2 to 50 mm")),
    };
    let at = snapped(s, sheet, view, &picks)?;
    let horizontal = kind == DimensionKind::Horizontal;
    let along = |q: &Point2| if horizontal { q.x } else { q.y };
    let across = |q: &Point2| if horizontal { q.y } else { q.x };
    let side = if off < 0.0 { -1.0 } else { 1.0 };
    let outer = at.iter().map(across).fold(-side * f64::MAX, |m, v| {
        if side > 0.0 { m.max(v) } else { m.min(v) }
    });
    let mut specs = Vec::new();
    for (i, j, out) in pairs(picks.len()) {
        let (Some(a), Some(b), Some(pa), Some(pb)) =
            (at.get(i), at.get(j), picks.get(i), picks.get(j))
        else {
            continue;
        };
        if (along(a) - along(b)).abs() < 1e-6 {
            return Err(bad(format!(
                "two points are at one place along the {} axis",
                if horizontal { "x" } else { "y" }
            )));
        }
        let own = if side > 0.0 {
            across(a).max(across(b))
        } else {
            across(a).min(across(b))
        };
        specs.push((
            kind,
            vec![*pa, *pb],
            off + (outer - own) + side * spacing * out,
        ));
    }
    let made = push_dimensions(s, sheet, view, &specs)?;
    Ok(json!({ "sheet": sheet, "dimensions": made }))
}

pub(super) fn add_baseline(s: &mut Session, p: &Value) -> CmdResult {
    // From the first point to each other one, in the order given; the caller's points
    // are sorted by distance from the first below, so the shortest is innermost.
    let sorted = sort_from_first(s, p)?;
    linear_set(s, &sorted, |n| {
        (1..n).map(|k| (0, k, (k - 1) as f64)).collect()
    })
}

pub(super) fn add_chain(s: &mut Session, p: &Value) -> CmdResult {
    let sorted = sort_along(s, p)?;
    linear_set(s, &sorted, |n| (1..n).map(|k| (k - 1, k, 0.0)).collect())
}

/// `p` with its points reordered by `key` of their snapped place (first kept first when
/// `keep_first`).
fn reorder(s: &Session, p: &Value, keep_first: bool) -> Result<Value, CommandError> {
    let (view, sheet) = view_sheet(s, p)?;
    let kind = linear_kind(p)?;
    let picks = points(p, 2)?;
    let at = snapped(s, sheet, view, &picks)?;
    let along = |q: &Point2| {
        if kind == DimensionKind::Horizontal {
            q.x
        } else {
            q.y
        }
    };
    let base = at.first().map(along).unwrap_or(0.0);
    let mut order: Vec<(f64, Point2)> = at
        .iter()
        .zip(&picks)
        .map(|(a, q)| {
            let v = along(a);
            (if keep_first { (v - base).abs() } else { v }, *q)
        })
        .collect();
    if keep_first {
        let first = order.remove(0);
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        order.insert(0, first);
    } else {
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let mut out = p.clone();
    out["points"] = json!(
        order
            .iter()
            .map(|(_, q)| json!({ "x": q.x, "y": q.y }))
            .collect::<Vec<_>>()
    );
    Ok(out)
}

fn sort_from_first(s: &Session, p: &Value) -> Result<Value, CommandError> {
    reorder(s, p, true)
}

fn sort_along(s: &Session, p: &Value) -> Result<Value, CommandError> {
    reorder(s, p, false)
}
