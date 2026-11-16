//! Tests of 2D drafting: dash patterns, grips, layers in the exports. Every expected value
//! is worked out by hand in the comments.

use crate::drafting::{DraftShape, GripKind, drag_grip, grip_at, grips, pick_entity};
use crate::linetype::{dash_polyline, pattern, scaled};
use crate::tests::sheet_with;
use crate::*;
use chamfera_doc::drafting::{DraftLayer, Entity, EntityKind, LineType};
use std::f64::consts::{FRAC_PI_2, PI};

const EPS: f64 = 1e-9;

fn p(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

/// The dashes of a line from (0, 0) along +x, as (from, to) distances.
fn dashes_along_x(len: f64, pat: &[f64]) -> Vec<(f64, f64)> {
    dash_polyline(&[p(0.0, 0.0), p(len, 0.0)], pat)
        .iter()
        .map(|[a, b]| (a.x, b.x))
        .collect()
}

fn assert_dashes(got: &[(f64, f64)], want: &[(f64, f64)]) {
    assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g.0 - w.0).abs() < EPS && (g.1 - w.1).abs() < EPS,
            "{got:?} vs {want:?}"
        );
    }
}

#[test]
fn dashes_along_a_line_follow_each_pattern() {
    // Center 8, 1.5, 0.5, 1.5 on 20 mm: dash 0-8, gap to 9.5, dot 9.5-10, gap to 11.5,
    // dash 11.5-19.5, gap 19.5-21 runs off the end.
    assert_dashes(
        &dashes_along_x(20.0, pattern(LineType::Center)),
        &[(0.0, 8.0), (9.5, 10.0), (11.5, 19.5)],
    );
    // The same at line type scale 2 (16, 3, 1, 3): dash 0-16, gap to 19, dot 19-20.
    assert_dashes(
        &dashes_along_x(20.0, &scaled(LineType::Center, 2.0)),
        &[(0.0, 16.0), (19.0, 20.0)],
    );
    // Phantom 10, 1.5, 0.5, 1.5, 0.5, 1.5 (15.5 a period) on 30 mm: 0-10, 11.5-12,
    // 13.5-14, then the next period from 15.5: 15.5-25.5, 27-27.5, 29-29.5.
    assert_dashes(
        &dashes_along_x(30.0, pattern(LineType::Phantom)),
        &[
            (0.0, 10.0),
            (11.5, 12.0),
            (13.5, 14.0),
            (15.5, 25.5),
            (27.0, 27.5),
            (29.0, 29.5),
        ],
    );
    // Hidden 1.5, 0.75 on 5 mm: 0-1.5, 2.25-3.75, 4.5-5 (cut by the end).
    assert_dashes(
        &dashes_along_x(5.0, pattern(LineType::Hidden)),
        &[(0.0, 1.5), (2.25, 3.75), (4.5, 5.0)],
    );
    // Dashed 3, 1.5 and dot 0.25, 1.25 on 6 mm: 0-3, 4.5-6; dots at 0, 1.5, 3, 4.5.
    assert_dashes(
        &dashes_along_x(6.0, pattern(LineType::Dashed)),
        &[(0.0, 3.0), (4.5, 6.0)],
    );
    assert_dashes(
        &dashes_along_x(6.0, pattern(LineType::Dot)),
        &[(0.0, 0.25), (1.5, 1.75), (3.0, 3.25), (4.5, 4.75)],
    );
    // Continuous: one piece.
    assert_dashes(&dashes_along_x(6.0, &[]), &[(0.0, 6.0)]);
    // Dash lengths sum: on a 23 mm centre line (two whole periods of 11.5) the ink is
    // 2 x (8 + 0.5) = 17 mm.
    let ink: f64 = dash_polyline(&[p(0.0, 0.0), p(0.0, 23.0)], pattern(LineType::Center))
        .iter()
        .map(|[a, b]| a.distance(*b))
        .sum();
    assert!((ink - 17.0).abs() < EPS, "{ink}");
}

#[test]
fn a_dash_runs_on_round_a_polyline_corner() {
    // (0,0) -> (2,0) -> (2,4), dashed 3, 1.5: the first dash is 2 mm along x and 1 mm
    // up; the gap runs to 4.5 mm along the path, i.e. y = 2.5; the next dash to 6 (y = 4).
    let d = dash_polyline(
        &[p(0.0, 0.0), p(2.0, 0.0), p(2.0, 4.0)],
        pattern(LineType::Dashed),
    );
    let want = [
        [p(0.0, 0.0), p(2.0, 0.0)],
        [p(2.0, 0.0), p(2.0, 1.0)],
        [p(2.0, 2.5), p(2.0, 4.0)],
    ];
    assert_eq!(d.len(), want.len(), "{d:?}");
    for (g, w) in d.iter().zip(want) {
        assert!(
            g[0].distance(w[0]) < EPS && g[1].distance(w[1]) < EPS,
            "{d:?}"
        );
    }
}

#[test]
fn hostile_patterns_never_hang_or_panic() {
    let line = [p(0.0, 0.0), p(1e4, 0.0)];
    for bad in [
        vec![f64::NAN, 1.0],
        vec![0.0, 0.0],
        vec![-1.0, 2.0],
        vec![f64::INFINITY, 1.0],
        vec![1.0],
    ] {
        assert_eq!(
            dash_polyline(&line, &bad).len(),
            1,
            "{bad:?} keeps the line whole"
        );
    }
    // A tiny pattern on a long line stops at the dash limit and draws the rest whole.
    let d = dash_polyline(&line, &[1e-6, 1e-6]);
    assert!(d.len() <= linetype::MAX_DASHES + 1);
    assert!(dash_polyline(&[], &[1.0, 1.0]).is_empty());
    assert!(dash_polyline(&[p(f64::NAN, 0.0), p(1.0, 0.0)], &[1.0, 1.0]).is_empty());
}

#[test]
fn dragging_a_grip_moves_exactly_that_point() {
    let line = EntityKind::Line {
        a: p(0.0, 0.0),
        b: p(10.0, 0.0),
    };
    let g = grips(&line);
    assert_eq!(g.len(), 3);
    assert_eq!(g[2].kind, GripKind::Mid);
    assert!(g[2].at.distance(p(5.0, 0.0)) < EPS);
    // End grip 0 to (7, 8): a moves there, b stays.
    assert_eq!(
        drag_grip(&line, 0, p(7.0, 8.0)).unwrap(),
        EntityKind::Line {
            a: p(7.0, 8.0),
            b: p(10.0, 0.0)
        }
    );
    // The midpoint (5, 0) to (5, 5): the whole line up 5.
    assert_eq!(
        drag_grip(&line, 2, p(5.0, 5.0)).unwrap(),
        EntityKind::Line {
            a: p(0.0, 5.0),
            b: p(10.0, 5.0)
        }
    );
    // An end onto the other end is refused: no zero-length line.
    assert!(drag_grip(&line, 0, p(10.0, 0.0)).is_err());
    assert!(drag_grip(&line, 3, p(1.0, 1.0)).is_err(), "no grip 3");

    // Circle centre (10, 0) r 2: quadrant grip 1 is (10, 2); dragged to (13, 4) the
    // radius is |(3, 4)| = 5 and the centre stays.
    let circle = EntityKind::Circle {
        centre: p(10.0, 0.0),
        radius: 2.0,
    };
    assert!(grips(&circle)[2].at.distance(p(10.0, 2.0)) < EPS);
    assert_eq!(
        drag_grip(&circle, 2, p(13.0, 4.0)).unwrap(),
        EntityKind::Circle {
            centre: p(10.0, 0.0),
            radius: 5.0
        }
    );
    assert_eq!(
        drag_grip(&circle, 0, p(1.0, 1.0)).unwrap(),
        EntityKind::Circle {
            centre: p(1.0, 1.0),
            radius: 2.0
        }
    );

    // Arc centre origin, r 10, 0 to 90°: end grip 2 at (0, 10) dragged to (-5, 0) ends
    // the arc at 180°, radius kept.
    let arc = EntityKind::Arc {
        centre: p(0.0, 0.0),
        radius: 10.0,
        start: 0.0,
        end: FRAC_PI_2,
    };
    let EntityKind::Arc {
        radius, start, end, ..
    } = drag_grip(&arc, 2, p(-5.0, 0.0)).unwrap()
    else {
        panic!("an arc stays an arc")
    };
    assert!((radius - 10.0).abs() < EPS && start.abs() < EPS && (end - PI).abs() < EPS);
    // The start grip to (0, -3): starts at 270° (-90° taken a turn up), ends at
    // 90° + 360° = 450°, so the arc sweeps 180°.
    let EntityKind::Arc { start, end, .. } = drag_grip(&arc, 1, p(0.0, -3.0)).unwrap() else {
        panic!("an arc stays an arc")
    };
    assert!((start - 1.5 * PI).abs() < EPS && (end - 2.5 * PI).abs() < EPS);
    // The arc's midpoint grip (at 45°) to (0, 4): radius 4, angles kept.
    assert_eq!(
        drag_grip(&arc, 3, p(0.0, 4.0)).unwrap(),
        EntityKind::Arc {
            centre: p(0.0, 0.0),
            radius: 4.0,
            start: 0.0,
            end: FRAC_PI_2
        }
    );
    // Onto the centre: a zero radius is refused.
    assert!(drag_grip(&arc, 3, p(0.0, 0.0)).is_err());

    // A closed square polyline: vertex grip 2 moves only that vertex; the closing
    // segment's midpoint (grip 4 + 3 = 7, at (0, 5)) moved to (-2, 5) shifts vertices 3
    // and 0 by (-2, 0).
    let square = EntityKind::Polyline {
        points: vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)],
        closed: true,
    };
    assert_eq!(grips(&square).len(), 8);
    assert_eq!(
        drag_grip(&square, 2, p(12.0, 11.0)).unwrap(),
        EntityKind::Polyline {
            points: vec![p(0.0, 0.0), p(10.0, 0.0), p(12.0, 11.0), p(0.0, 10.0)],
            closed: true
        }
    );
    assert!(grips(&square)[7].at.distance(p(0.0, 5.0)) < EPS);
    assert_eq!(
        drag_grip(&square, 7, p(-2.0, 5.0)).unwrap(),
        EntityKind::Polyline {
            points: vec![p(-2.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(-2.0, 10.0)],
            closed: true
        }
    );
    // A grip is found by where it is.
    assert_eq!(grip_at(&square, p(10.2, 9.9), 0.5), Some(2));
    assert_eq!(grip_at(&square, p(5.0, 5.0), 0.5), None);
    // Hostile targets are refused, not drawn.
    assert!(drag_grip(&line, 0, p(f64::NAN, 0.0)).is_err());
    assert!(drag_grip(&line, 0, p(1e300, 0.0)).is_err());
}

/// A sheet with layers `Axes` (center, red, 0.35), `Notes` (dashed) and their entities.
fn drafted_sheet() -> Sheet {
    let mut s = sheet_with(vec![], vec![]);
    let d = &mut s.drafting;
    let mut axes = DraftLayer::new("Axes");
    axes.line_type = LineType::Center;
    axes.colour = Some([255, 0, 0]);
    axes.weight = 0.35;
    let mut notes = DraftLayer::new("Notes");
    notes.line_type = LineType::Dashed;
    d.layers.push(axes);
    d.layers.push(notes);
    d.entities.push(Entity {
        id: 2,
        layer: "Axes".into(),
        kind: EntityKind::Line {
            a: p(100.0, 100.0),
            b: p(200.0, 100.0),
        },
    });
    d.entities.push(Entity {
        id: 3,
        layer: "Notes".into(),
        kind: EntityKind::Circle {
            centre: p(150.0, 150.0),
            radius: 20.0,
        },
    });
    s
}

fn svg_of(s: &Sheet) -> String {
    svg::write(&render_sheet(s, &[]).unwrap())
}

fn dxf_pairs(text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    lines
        .chunks(2)
        .map(|c| {
            (
                c[0].trim().to_owned(),
                c.get(1).copied().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

/// (entity type, layer) of every DXF entity.
fn dxf_entities(text: &str) -> Vec<(String, String)> {
    let pairs = dxf_pairs(text);
    let start = pairs
        .iter()
        .position(|(c, v)| c == "2" && v == "ENTITIES")
        .unwrap();
    let mut out = Vec::new();
    let mut kind = None;
    for (c, v) in &pairs[start..] {
        if c == "0" {
            kind = Some(v.clone());
        } else if c == "8"
            && let Some(k) = kind.take()
        {
            out.push((k, v.clone()));
        }
    }
    out
}

#[test]
fn user_layers_reach_svg_dxf_and_pdf_with_colour_type_and_weight() {
    let s = drafted_sheet();
    let g = render_sheet(&s, &[]).unwrap();
    assert_eq!(g.draft_layers.len(), 3);
    assert_eq!(g.drafted.len(), 2);
    let svg = svg::write(&g);
    assert!(svg.contains(
        r##"<g id="layer-Axes" fill="none" stroke="#ff0000" stroke-width="0.35" stroke-linecap="round" stroke-dasharray="8 1.5 0.5 1.5">"##
    ), "{svg}");
    assert!(
        svg.contains(r#"<line x1="100" y1="197" x2="200" y2="197"/>"#),
        "A3 is 297 high: y 100 -> 197"
    );
    assert!(svg.contains(r##"<g id="layer-Notes" fill="none" stroke="#000000" stroke-width="0.25" stroke-linecap="round" stroke-dasharray="3 1.5">"##));
    assert!(svg.contains(r#"<circle cx="150" cy="147" r="20"/>"#));

    let dxf = dxf::write(&g);
    let ents = dxf_entities(&dxf);
    assert!(ents.contains(&("LINE".into(), "Axes".into())));
    assert!(ents.contains(&("CIRCLE".into(), "Notes".into())));
    // The LAYER table: Axes red (1) with CH-CENTER; the line type 8, -1.5, 0.5, -1.5.
    assert!(
        dxf.contains("  0\nLAYER\n  2\nAxes\n 70\n0\n 62\n1\n  6\nCH-CENTER\n"),
        "{dxf}"
    );
    assert!(dxf.contains("  2\nCH-CENTER\n 70\n0\n  3\nCenter  ____ _ ____\n 72\n65\n 73\n4\n 40\n11.5\n 49\n8\n 49\n-1.5\n 49\n0.5\n 49\n-1.5\n"));

    let pdf = String::from_utf8_lossy(&pdf::write(&g)).into_owned();
    assert!(
        pdf.contains("0.35 w [8 1.5 0.5 1.5] 0 d 1 0 0 RG 1 0 0 rg"),
        "{pdf}"
    );
}

#[test]
fn a_layer_that_is_off_or_frozen_is_hidden_from_every_export() {
    let shown = drafted_sheet();
    let mut off = drafted_sheet();
    off.drafting.layers[2].on = false;
    let mut frozen = drafted_sheet();
    frozen.drafting.layers[2].frozen = true;
    // The same sheet with the Notes circle gone altogether, for comparison.
    let mut without = drafted_sheet();
    without.drafting.entities.retain(|e| e.id != 3);
    without.drafting.layers[2].on = false;

    assert!(svg_of(&shown).contains(r#"r="20""#));
    for s in [&off, &frozen] {
        let g = render_sheet(s, &[]).unwrap();
        assert_eq!(g.drafted.len(), 1, "only the Axes line is drawn");
        let svg = svg::write(&g);
        assert!(!svg.contains("layer-Notes"), "no group for a hidden layer");
        assert!(!svg.contains(r#"r="20""#));
        let ents = dxf_entities(&dxf::write(&g));
        assert!(!ents.iter().any(|(_, l)| l == "Notes"), "{ents:?}");
        assert!(ents.iter().any(|(_, l)| l == "Axes"));
    }
    // PDF: exactly the bytes of the sheet without the circle.
    let pdf = |s: &Sheet| pdf::write(&render_sheet(s, &[]).unwrap());
    assert_eq!(pdf(&off), pdf(&without));
    assert_ne!(pdf(&shown), pdf(&without));
    // DXF keeps the layer itself, with a negative colour for off and flag 1 for frozen.
    let dxf_off = dxf::write(&render_sheet(&off, &[]).unwrap());
    assert!(
        dxf_off.contains("  2\nNotes\n 70\n0\n 62\n-7\n"),
        "{dxf_off}"
    );
    let dxf_frozen = dxf::write(&render_sheet(&frozen, &[]).unwrap());
    assert!(dxf_frozen.contains("  2\nNotes\n 70\n1\n 62\n7\n"));
}

#[test]
fn the_line_type_scale_multiplies_every_dash() {
    let mut s = drafted_sheet();
    s.drafting.linetype_scale = 2.0;
    // Center at scale 2: 16, 3, 1, 3 (a period of 23).
    assert!(svg_of(&s).contains(r#"stroke-dasharray="16 3 1 3""#));
    let dxf = dxf::write(&render_sheet(&s, &[]).unwrap());
    assert!(dxf.contains("  2\nCH-CENTER\n 70\n0\n  3\nCenter  ____ _ ____\n 72\n65\n 73\n4\n 40\n23\n 49\n16\n 49\n-3\n 49\n1\n 49\n-3\n"));
    // The sheet's own dashed and centre lines keep their scale.
    assert!(dxf.contains("  2\nCENTER\n 70\n0\n"));
    assert!(dxf.contains(" 40\n11.5\n 49\n8\n"));
}

#[test]
fn picking_finds_the_nearest_entity_but_not_on_a_locked_layer() {
    let mut s = drafted_sheet();
    let g = render_sheet(&s, &[]).unwrap();
    // 0.3 mm above the Axes line.
    assert_eq!(
        pick_entity(&g.draft_layers, &g.drafted, p(150.0, 100.3), 1.0),
        Some(2)
    );
    // On the circle (150, 150) r 20 at its right quadrant.
    assert_eq!(
        pick_entity(&g.draft_layers, &g.drafted, p(170.2, 150.0), 1.0),
        Some(3)
    );
    assert_eq!(
        pick_entity(&g.draft_layers, &g.drafted, p(10.0, 10.0), 1.0),
        None
    );
    s.drafting.layers[1].locked = true;
    let g = render_sheet(&s, &[]).unwrap();
    assert_eq!(
        pick_entity(&g.draft_layers, &g.drafted, p(150.0, 100.3), 1.0),
        None
    );
    // A locked layer is still drawn and exported.
    assert!(svg::write(&g).contains("layer-Axes"));
    let _: &[DraftShape] = &g.drafted;
}

#[test]
fn nearest_basic_colours_go_to_dxf() {
    assert_eq!(dxf::nearest_aci(None), 7);
    assert_eq!(dxf::nearest_aci(Some([250, 10, 10])), 1);
    assert_eq!(dxf::nearest_aci(Some([240, 230, 20])), 2);
    assert_eq!(dxf::nearest_aci(Some([0, 0, 200])), 5);
    assert_eq!(dxf::nearest_aci(Some([20, 20, 20])), 7);
}
