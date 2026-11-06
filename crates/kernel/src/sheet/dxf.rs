//! A minimal DXF (release 12, `AC1009`) writer for flat pattern outlines.
//!
//! Written from Autodesk's public *DXF Reference*: a file is a list of group-code/value
//! pairs, one per line each; this writes a `HEADER` section naming the version and the
//! units (`$INSUNITS` 4 = millimetres), then an `ENTITIES` section of `LINE` (codes 10/20/30
//! start, 11/21/31 end), `ARC` (10/20/30 centre, 40 radius, 50/51 start and end angles in
//! degrees, counter-clockwise) and `CIRCLE` entities on layer `0`, and `EOF`.

use super::FlatEdge;
use std::fmt::Write as _;

/// Shortest decimal that reads back to the same `f64` (Rust's `Display` for floats).
fn num(v: f64) -> String {
    format!("{v:?}")
}

/// The outline as DXF R12 text, on layer `0`, in millimetres.
pub fn flat_dxf(edges: &[FlatEdge]) -> String {
    let mut s = String::new();
    let mut pair = |code: u32, value: &str| {
        let _ = writeln!(s, "{code:>3}\n{value}");
    };
    pair(0, "SECTION");
    pair(2, "HEADER");
    pair(9, "$ACADVER");
    pair(1, "AC1009");
    pair(9, "$INSUNITS");
    pair(70, "4");
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "ENTITIES");
    for e in edges {
        match *e {
            FlatEdge::Line { a, b } => {
                pair(0, "LINE");
                pair(8, "0");
                pair(10, &num(a.x));
                pair(20, &num(a.y));
                pair(30, "0.0");
                pair(11, &num(b.x));
                pair(21, &num(b.y));
                pair(31, "0.0");
            }
            FlatEdge::Arc {
                centre,
                radius,
                start_angle,
                end_angle,
            } => {
                pair(0, "ARC");
                pair(8, "0");
                pair(10, &num(centre.x));
                pair(20, &num(centre.y));
                pair(30, "0.0");
                pair(40, &num(radius));
                pair(50, &num(start_angle.to_degrees()));
                pair(51, &num(end_angle.to_degrees()));
            }
            FlatEdge::Circle { centre, radius } => {
                pair(0, "CIRCLE");
                pair(8, "0");
                pair(10, &num(centre.x));
                pair(20, &num(centre.y));
                pair(30, "0.0");
                pair(40, &num(radius));
            }
        }
    }
    pair(0, "ENDSEC");
    pair(0, "EOF");
    s
}
