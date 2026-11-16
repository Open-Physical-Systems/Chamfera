//! DXF R12 (AC1009) ASCII export, from the public DXF reference: a HEADER, the LTYPE,
//! LAYER and STYLE tables, an empty BLOCKS section and LINE, CIRCLE, ARC, TEXT and
//! SOLID entities on the layers VISIBLE, HIDDEN, CENTER, DIM and BORDER. Units are mm.
//! The sheet's user layers follow with their colour (the nearest basic colour index),
//! line type (scaled for the sheet, named `CH-...`), frozen and locked flags, and a
//! negative colour when off; entities of layers that are off or frozen are left out.
//! R12 has no line weights, so those reach SVG and PDF only.

use crate::linetype;
use crate::sheet::SheetGeometry;
use crate::{Align, Layer, Shape};
use chamfera_doc::drafting::LineType;
use std::fmt::Write;

/// A number for DXF: plain decimal, at most 10 places, no trailing zeros. Sketches go out
/// through here too (`sketch.export_dxf`), so arcs keep their endpoints to well under a
/// micrometre even at a metre's radius.
fn num(v: f64) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let s = format!("{v:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

struct Dxf(String);

impl Dxf {
    fn pair(&mut self, code: u16, value: &str) {
        let _ = write!(self.0, "{code:>3}\n{value}\n");
    }
    fn num(&mut self, code: u16, v: f64) {
        self.pair(code, &num(v));
    }
    fn point(&mut self, base: u16, x: f64, y: f64) {
        self.num(base, x);
        self.num(base + 10, y);
        self.num(base + 20, 0.0);
    }
}

/// DXF text: `%%c` for Ø, `%%d` for °, `%%%` for %, other non-ASCII as `?`.
fn dxf_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            'Ø' | 'ø' => out.push_str("%%c"),
            '°' => out.push_str("%%d"),
            '%' => out.push_str("%%%"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

/// AutoCAD Color Index of each layer.
fn colour(layer: Layer) -> &'static str {
    match layer {
        Layer::Visible | Layer::Border => "7",
        Layer::Hidden => "4",
        Layer::Center => "1",
        Layer::Dim => "3",
        Layer::Hatch => "8",
        Layer::Dangling => "6",
    }
}

fn line_type(layer: Layer) -> &'static str {
    match layer {
        Layer::Hidden => "DASHED",
        Layer::Center => "CENTER",
        _ => "CONTINUOUS",
    }
}

/// The sheet as a DXF R12 file.
pub fn write(sheet: &SheetGeometry) -> String {
    let mut d = Dxf(String::new());
    d.pair(0, "SECTION");
    d.pair(2, "HEADER");
    d.pair(9, "$ACADVER");
    d.pair(1, "AC1009");
    d.pair(9, "$INSBASE");
    d.point(10, 0.0, 0.0);
    for (name, x, y) in [
        ("$EXTMIN", 0.0, 0.0),
        ("$EXTMAX", sheet.width, sheet.height),
        ("$LIMMIN", 0.0, 0.0),
        ("$LIMMAX", sheet.width, sheet.height),
    ] {
        d.pair(9, name);
        d.num(10, x);
        d.num(20, y);
    }
    d.pair(0, "ENDSEC");

    d.pair(0, "SECTION");
    d.pair(2, "TABLES");
    // Line types: (name, description, dash pattern with gaps negative).
    let types: [(&str, &str, &[f64]); 3] = [
        ("CONTINUOUS", "Solid line", &[]),
        ("DASHED", "Dashed __ __ __", &[3.0, -1.5]),
        ("CENTER", "Center ____ . ____", &[8.0, -1.5, 0.5, -1.5]),
    ];
    // The user's line types at this sheet's scale (gaps negative, as DXF wants).
    let user: Vec<(&str, &str, Vec<f64>)> = LineType::ALL
        .into_iter()
        .filter(|t| *t != LineType::Continuous)
        .map(|t| {
            let k = if sheet.linetype_scale > 0.0 {
                sheet.linetype_scale
            } else {
                1.0
            };
            let signed = linetype::scaled(t, k)
                .iter()
                .enumerate()
                .map(|(i, x)| if i % 2 == 0 { *x } else { -*x })
                .collect();
            (linetype::dxf_name(t), linetype::describe(t), signed)
        })
        .collect();
    let all: Vec<(&str, &str, &[f64])> = types
        .iter()
        .map(|(n, s, p)| (*n, *s, *p))
        .chain(user.iter().map(|(n, s, p)| (*n, *s, p.as_slice())))
        .collect();
    d.pair(0, "TABLE");
    d.pair(2, "LTYPE");
    d.pair(70, &all.len().to_string());
    for (name, desc, dashes) in all {
        d.pair(0, "LTYPE");
        d.pair(2, name);
        d.pair(70, "0");
        d.pair(3, desc);
        d.pair(72, "65");
        d.pair(73, &dashes.len().to_string());
        d.num(40, dashes.iter().map(|x| x.abs()).sum());
        for x in dashes {
            d.num(49, *x);
        }
    }
    d.pair(0, "ENDTAB");
    d.pair(0, "TABLE");
    d.pair(2, "LAYER");
    let zero_is_user = sheet.draft_layers.iter().any(|s| s.name == "0");
    let count = Layer::ALL.len() + sheet.draft_layers.len() + usize::from(!zero_is_user);
    d.pair(70, &count.to_string());
    if !zero_is_user {
        d.pair(0, "LAYER");
        d.pair(2, "0");
        d.pair(70, "0");
        d.pair(62, "7");
        d.pair(6, "CONTINUOUS");
    }
    // User layers: flags 1 frozen, 4 locked; a negative colour is a layer that is off.
    for s in &sheet.draft_layers {
        d.pair(0, "LAYER");
        d.pair(2, &s.name);
        let flags = i32::from(s.frozen) + 4 * i32::from(s.locked);
        d.pair(70, &flags.to_string());
        let aci = i32::from(nearest_aci(s.colour));
        d.pair(62, &(if s.on { aci } else { -aci }).to_string());
        d.pair(6, linetype::dxf_name(s.line_type));
    }
    for layer in Layer::ALL {
        d.pair(0, "LAYER");
        d.pair(2, layer.name());
        d.pair(70, "0");
        d.pair(62, colour(layer));
        d.pair(6, line_type(layer));
    }
    d.pair(0, "ENDTAB");
    d.pair(0, "TABLE");
    d.pair(2, "STYLE");
    d.pair(70, "1");
    d.pair(0, "STYLE");
    d.pair(2, "STANDARD");
    d.pair(70, "0");
    d.num(40, 0.0);
    d.num(41, 1.0);
    d.num(50, 0.0);
    d.pair(71, "0");
    d.num(42, 3.5);
    d.pair(3, "txt");
    d.pair(4, "");
    d.pair(0, "ENDTAB");
    d.pair(0, "ENDSEC");

    d.pair(0, "SECTION");
    d.pair(2, "BLOCKS");
    d.pair(0, "ENDSEC");

    d.pair(0, "SECTION");
    d.pair(2, "ENTITIES");
    for item in &sheet.items {
        entity(&mut d, item.layer.name(), &item.shape);
    }
    for s in &sheet.drafted {
        if let Some(style) = sheet.draft_layers.get(s.layer) {
            entity(&mut d, &style.name, &s.shape);
        }
    }
    d.pair(0, "ENDSEC");
    d.pair(0, "EOF");
    d.0
}

/// One shape as a DXF entity on `layer`.
fn entity(d: &mut Dxf, layer: &str, shape: &Shape) {
    match shape {
        Shape::Line { a, b } => {
            d.pair(0, "LINE");
            d.pair(8, layer);
            d.point(10, a.x, a.y);
            d.point(11, b.x, b.y);
        }
        Shape::Circle { centre, radius } => {
            d.pair(0, "CIRCLE");
            d.pair(8, layer);
            d.point(10, centre.x, centre.y);
            d.num(40, *radius);
        }
        Shape::Arc {
            centre,
            radius,
            start,
            end,
        } => {
            d.pair(0, "ARC");
            d.pair(8, layer);
            d.point(10, centre.x, centre.y);
            d.num(40, *radius);
            d.num(50, start.to_degrees().rem_euclid(360.0));
            d.num(51, end.to_degrees().rem_euclid(360.0));
        }
        Shape::Text {
            at,
            height,
            angle,
            align,
            text,
        } => {
            d.pair(0, "TEXT");
            d.pair(8, layer);
            d.point(10, at.x, at.y);
            d.num(40, *height);
            d.pair(1, &dxf_text(text));
            d.num(50, angle.to_degrees());
            let justify = match align {
                Align::Left => "0",
                Align::Centre => "1",
                Align::Right => "2",
            };
            if justify != "0" {
                d.pair(72, justify);
                d.point(11, at.x, at.y);
            }
        }
        Shape::Fill { points } => {
            // A triangle is a SOLID with its third corner given twice.
            d.pair(0, "SOLID");
            d.pair(8, layer);
            let [a, b, c] = *points;
            d.point(10, a.x, a.y);
            d.point(11, b.x, b.y);
            d.point(12, c.x, c.y);
            d.point(13, c.x, c.y);
        }
    }
}

/// The nearest of the seven basic AutoCAD Color Index colours (1 red, 2 yellow, 3 green,
/// 4 cyan, 5 blue, 6 magenta, 7 white or black): what DXF R12 can store.
pub fn nearest_aci(c: Option<[u8; 3]>) -> u8 {
    let Some([r, g, b]) = c else {
        return 7;
    };
    let basic: [(u8, [i32; 3]); 8] = [
        (1, [255, 0, 0]),
        (2, [255, 255, 0]),
        (3, [0, 255, 0]),
        (4, [0, 255, 255]),
        (5, [0, 0, 255]),
        (6, [255, 0, 255]),
        (7, [255, 255, 255]),
        (7, [0, 0, 0]),
    ];
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    basic
        .iter()
        .min_by_key(|(_, [x, y, z])| (r - x).pow(2) + (g - y).pow(2) + (b - z).pow(2))
        .map_or(7, |(i, _)| *i)
}
