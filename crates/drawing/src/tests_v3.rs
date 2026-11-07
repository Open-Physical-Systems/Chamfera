//! Tests of drawings version 3: ordinate dimensions, hole callouts and the title block's
//! new fields. Expected values are derived by hand in each test's comments.

use crate::tests::{block, plate_with_hole, sheet_with, svg_count, view};
use crate::*;
use chamfera_doc::drawing::{Annotation, AnnotationKind, SheetDimension};
use chamfera_geom::Point2;

const EPS: f64 = 1e-6;

fn near(a: Point2, b: Point2) -> bool {
    a.distance(b) < EPS
}

fn texts(out: &SheetGeometry, layer: Layer) -> Vec<String> {
    out.items
        .iter()
        .filter(|i| i.layer == layer)
        .filter_map(|i| match &i.shape {
            Shape::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn ordinate_dimensions_of_the_box_read_0_50_and_10_from_its_corner() {
    let b = block();
    let s = sheet_with(
        vec![view(2, ViewDirection::Front, 150.0, 150.0, None)],
        vec![],
    );
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let v = out.view(2).unwrap();
    // The front view of the 50 x 30 x 10 box at (150, 150), 1:1: x 125..175, z 145..155
    // on paper. From the lower-left corner (125, 145): the lower-right corner is 50
    // along x, the upper-left 10 along y, and the corner itself 0.
    let origin = Point2::new(125.3, 145.2);
    let x = plan_dimension(
        v,
        DimensionKind::OrdinateX,
        &[origin, Point2::new(174.8, 144.9)],
    )
    .unwrap();
    assert!((x.value - 50.0).abs() < 1e-9, "{}", x.value);
    let y = plan_dimension(
        v,
        DimensionKind::OrdinateY,
        &[origin, Point2::new(125.1, 155.2)],
    )
    .unwrap();
    assert!((y.value - 10.0).abs() < 1e-9);
    let zero = plan_dimension(v, DimensionKind::OrdinateX, &[origin, origin]).unwrap();
    assert!(
        zero.value.abs() < 1e-9,
        "an ordinate at the origin is 0, not refused"
    );
    assert_eq!(format_value(DimensionKind::OrdinateX, zero.value), "0");
    // A point left of the origin reads negative: from the lower-right corner, the
    // lower-left one is at -50.
    let back = plan_dimension(
        v,
        DimensionKind::OrdinateX,
        &[Point2::new(175.0, 145.0), Point2::new(125.0, 145.0)],
    )
    .unwrap();
    assert_eq!(format_value(back.kind, back.value), "-50");

    // Drawn with offset -10: the lines end 10 mm below the origin, at y = 135. The 50
    // has one extension line from (175, 144) (1 mm gap) down to (175, 135) and its text
    // there, upright (90°); the 0 also has the origin's circle of radius 1 just beyond
    // its line's end (125, 135): centred at (125, 134).
    let dim = |id: u32, plan: &DimensionPlan| SheetDimension {
        id,
        view: 2,
        kind: plan.kind,
        points: plan.points.clone(),
        offset: -10.0,
        refs: Vec::new(),
    };
    let s = sheet_with(s.views.clone(), vec![dim(3, &zero), dim(4, &x)]);
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let dims: Vec<&Item> = out.items.iter().filter(|i| i.layer == Layer::Dim).collect();
    assert!(dims.iter().any(|i| matches!(i.shape, Shape::Line { a, b }
        if near(a, Point2::new(175.0, 144.0)) && near(b, Point2::new(175.0, 135.0)))));
    assert!(dims.iter().any(|i| matches!(&i.shape,
        Shape::Text { text, angle, .. } if text == "50" && (angle - std::f64::consts::FRAC_PI_2).abs() < EPS)));
    let circles: Vec<(Point2, f64)> = dims
        .iter()
        .filter_map(|i| match i.shape {
            Shape::Circle { centre, radius } => Some((centre, radius)),
            _ => None,
        })
        .collect();
    assert_eq!(
        circles.len(),
        1,
        "only the zero marks the origin: {circles:?}"
    );
    assert!(near(circles[0].0, Point2::new(125.0, 134.0)) && (circles[0].1 - 1.0).abs() < EPS);
    let state: Vec<Option<String>> = out.dimensions.iter().map(|d| d.text.clone()).collect();
    assert_eq!(state, vec![Some("0".into()), Some("50".into())]);
    // A drag 5 mm down moves the line 5 mm further: offset -15.
    let d4 = &s.dimensions[1];
    let moved = moved_offset(
        d4,
        out.view(2).unwrap(),
        chamfera_geom::Vec2::new(3.0, -5.0),
    );
    assert_eq!(moved, Some(-15.0));
}

fn callout_sheet(feature: u32) -> chamfera_doc::drawing::Sheet {
    let mut s = sheet_with(
        vec![view(2, ViewDirection::Top, 150.0, 150.0, None)],
        vec![],
    );
    s.annotations.push(Annotation {
        id: 3,
        kind: AnnotationKind::HoleCallout {
            view: 2,
            feature,
            at: Point2::new(190.0, 180.0),
            // The hole's centre (25, 15) in the top view's coordinates (x right, y up).
            target: Point2::new(25.0, 15.0),
            refs: Vec::new(),
        },
    });
    s
}

#[test]
fn a_hole_callout_points_at_the_hole_outline_and_writes_its_lines() {
    let b = plate_with_hole();
    let notes = [HoleNote {
        feature: 7,
        lines: vec!["Ø6 THRU".into(), "CBORE Ø11 x 6.4 DEEP".into()],
    }];
    let out = render_full(&callout_sheet(7), &[("Body1", &b)], &[], &notes).unwrap();
    // The plate's box centre (25, 15) is at (150, 150); the Ø6 hole is a circle of
    // radius 3 there. The leader runs from (190, 180) towards the centre, direction
    // (-0.8, -0.6) (a 3-4-5 triangle: 40 across, 30 up, 50 long), so its arrow's tip is
    // on the outline at (150 + 3 x 0.8, 150 + 3 x 0.6) = (152.4, 151.8).
    let tip = Point2::new(152.4, 151.8);
    assert!(
        out.items.iter().any(|i| i.layer == Layer::Dim
            && matches!(i.shape, Shape::Fill { points } if near(points[0], tip))),
        "no arrow at {tip:?}"
    );
    assert!(
        out.items
            .iter()
            .any(|i| matches!(i.shape, Shape::Line { a, b }
        if near(a, tip) && near(b, Point2::new(190.0, 180.0))))
    );
    let t = texts(&out, Layer::Dim);
    assert!(t.contains(&"Ø6 THRU".to_owned()) && t.contains(&"CBORE Ø11 x 6.4 DEEP".to_owned()));
    assert!(out.dangling_annotations.is_empty());
    // Picking near its text picks it.
    assert_eq!(
        out.pick(Point2::new(195.0, 182.0), 2.0),
        Some(Owner::Annotation(3))
    );

    // When the feature is no hole any more, the callout stays, dangling, with a "?".
    let out = render_full(&callout_sheet(8), &[("Body1", &b)], &[], &notes).unwrap();
    assert_eq!(out.dangling_annotations.len(), 1);
    assert!(texts(&out, Layer::Dangling).contains(&"?".to_owned()));
}

#[test]
fn hole_features_are_found_in_reference_names() {
    assert_eq!(
        refs::hole_feature("between 5:hole 0 wall and 1:top"),
        Some(5)
    );
    assert_eq!(
        refs::hole_feature("12:hole 3 counterbore floor 0"),
        Some(12)
    );
    assert_eq!(refs::hole_feature("2:top 0"), None);
    assert_eq!(refs::hole_feature("x:hole 1 wall"), None);
    assert_eq!(refs::hole_feature("3:hole wall"), None);
    assert_eq!(refs::hole_feature("é:hole 1 2:hole 4 wall"), Some(2));
    assert_eq!(refs::hole_feature(":hole "), None);
    assert_eq!(refs::hole_feature(""), None);
}

#[test]
fn the_title_block_shows_number_revision_material_and_checker() {
    let b = block();
    let mut s = sheet_with(
        vec![view(2, ViewDirection::Front, 150.0, 150.0, None)],
        vec![],
    );
    s.title.drawing_number = "CH-0042".into();
    s.title.revision = "B".into();
    s.title.material = "AlMg3".into();
    s.title.checked_by = "QA".into();
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let t = texts(&out, Layer::Border);
    for want in [
        "CH-0042",
        "B",
        "AlMg3",
        "QA",
        "DRAWING NO.",
        "REV",
        "MATERIAL",
        "CHECKED",
    ] {
        assert!(t.contains(&want.to_owned()), "{want} missing from {t:?}");
    }
    // 9 labels, the 9 values (Block, CH-0042, B; 1:1, mm, AlMg3, the date, Test, QA),
    // CHAMFERA and the projection's name: 20 texts, and the same in the SVG.
    assert_eq!(t.len(), 20);
    assert_eq!(svg_count(&svg::write(&out), "BORDER", "text"), 20);
    // Every value stays inside the title block (x 230..410, y 10..40 on A3).
    for i in out.items.iter().filter(|i| i.layer == Layer::Border) {
        if let Shape::Text {
            at,
            height,
            text,
            align: Align::Left,
            ..
        } = &i.shape
        {
            if at.y < 40.0 && at.x > 230.0 {
                let right = at.x + text_width(text, *height);
                assert!(right <= 410.0 + EPS, "{text} runs out to {right}");
            }
        }
    }
    // A long value shrinks to fit its cell rather than running over the next one.
    s.title.material = "A VERY LONG MATERIAL NAME INDEED".into();
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let h = out
        .items
        .iter()
        .find_map(|i| match &i.shape {
            Shape::Text { text, height, .. } if text.starts_with("A VERY") => Some(*height),
            _ => None,
        })
        .unwrap();
    assert!(h < 3.5, "{h}");
}
