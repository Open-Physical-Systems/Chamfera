//! DXF R12 (AC1009) ASCII export, from the public DXF reference: a HEADER, the LTYPE,
//! LAYER and STYLE tables, an empty BLOCKS section and LINE, CIRCLE, ARC, TEXT and
//! SOLID entities on the layers VISIBLE, HIDDEN, CENTER, DIM and BORDER. Units are mm.

use crate::sheet::SheetGeometry;
use crate::svg::num;
use crate::{Align, Layer, Shape};
use std::fmt::Write;

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
    d.pair(0, "TABLE");
    d.pair(2, "LTYPE");
    d.pair(70, &types.len().to_string());
    for (name, desc, dashes) in types {
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
    d.pair(70, &(Layer::ALL.len() + 1).to_string());
    d.pair(0, "LAYER");
    d.pair(2, "0");
    d.pair(70, "0");
    d.pair(62, "7");
    d.pair(6, "CONTINUOUS");
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
        let layer = item.layer.name();
        match &item.shape {
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
    d.pair(0, "ENDSEC");
    d.pair(0, "EOF");
    d.0
}
