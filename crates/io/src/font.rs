//! Text outlines from a font, for sketch text.
//!
//! A line of text becomes closed contours of lines and cubic Bézier segments in
//! millimetres. Glyph outlines are read with `skrifa` (MIT OR Apache-2.0), unhinted, in
//! font units; quadratic segments (TrueType) are raised to cubic ones exactly (a quadratic
//! with control point `c` between `p0` and `p1` is the cubic with controls
//! `p0 + 2/3 (c - p0)` and `p1 + 2/3 (c - p1)`; Farin, *Curves and Surfaces for CAGD*,
//! degree elevation). Glyphs are placed one after another by their advance widths, with no
//! kerning or shaping, so each character is drawn as the font draws it on its own.
//!
//! The height is the height of the capital letters (the font's cap height, OS/2), so
//! `height: 10` gives an `H` 10 mm tall. The text starts at `(x, y)` on its baseline
//! (left, centre or right aligned); a `\n` starts a new line 1.4 heights below.
//!
//! The bundled font is Lato Regular (SIL Open Font License 1.1, `assets/fonts/`).
//! A font from the user is hostile input: it is size-capped by the caller, parsed by
//! `skrifa` (which validates tables as it reads them), and every coordinate it gives is
//! checked to be finite.

use chamfera_geom::Point2;

/// The bundled font: Lato Regular 2.015 (OFL 1.1; `assets/fonts/OFL-Lato.txt`).
pub const DEFAULT_FONT: &[u8] = include_bytes!("../../../assets/fonts/Lato-Regular.ttf");
/// Most characters of one text.
pub const MAX_TEXT_CHARS: usize = 256;
/// Most segments in all the outlines of one text.
pub const MAX_TEXT_SEGMENTS: usize = 20_000;
/// Largest font file read (bytes).
pub const MAX_FONT_BYTES: u64 = 32 * 1024 * 1024;
/// Line spacing, in text heights.
const LINE_SPACING: f64 = 1.4;
/// Segments shorter than this (mm) are dropped (a font's zero-length closing lines).
const MIN_SEGMENT: f64 = 1e-6;

/// One piece of an outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    Line(Point2, Point2),
    /// A cubic Bézier: start, two control points, end.
    Cubic([Point2; 4]),
}

impl Segment {
    pub fn start(&self) -> Point2 {
        match self {
            Segment::Line(a, _) => *a,
            Segment::Cubic(c) => c[0],
        }
    }

    pub fn end(&self) -> Point2 {
        match self {
            Segment::Line(_, b) => *b,
            Segment::Cubic(c) => c[3],
        }
    }
}

/// Where a line of text sits relative to its start point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
}

impl Align {
    pub fn parse(s: &str) -> Option<Align> {
        match s {
            "left" => Some(Align::Left),
            "centre" | "center" => Some(Align::Centre),
            "right" => Some(Align::Right),
            _ => None,
        }
    }
}

/// The outlines of a text: closed contours, each a chain of segments whose last end is
/// its first start.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextOutline {
    pub contours: Vec<Vec<Segment>>,
    /// Characters the font has no glyph for (drawn as nothing, their advance kept).
    pub missing: Vec<char>,
    /// The widest line's advance (mm).
    pub width: f64,
}

/// Why a text could not be outlined.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FontError {
    #[error("the font file could not be read: {0}")]
    BadFont(String),
    #[error("{0}")]
    BadText(String),
}

/// A pen that collects contours in font units.
#[derive(Default)]
struct Pen {
    contours: Vec<Vec<Segment>>,
    current: Vec<Segment>,
    start: Option<Point2>,
    at: Point2,
    scale: f64,
    origin: Point2,
    segments: usize,
    bad: bool,
}

impl Pen {
    fn map(&mut self, x: f32, y: f32) -> Point2 {
        let (x, y) = (f64::from(x), f64::from(y));
        if !(x.is_finite() && y.is_finite()) {
            self.bad = true;
        }
        Point2::new(
            self.origin.x + x * self.scale,
            self.origin.y + y * self.scale,
        )
    }

    fn push(&mut self, s: Segment) {
        if s.start().distance(s.end()) < MIN_SEGMENT && matches!(s, Segment::Line(..)) {
            return;
        }
        self.segments += 1;
        self.current.push(s);
        self.at = s.end();
    }

    fn finish(&mut self) {
        // Close the contour back to its start, as fonts leave it implied.
        if let (Some(start), Some(last)) = (self.start, self.current.last().copied())
            && last.end().distance(start) >= MIN_SEGMENT
        {
            self.push(Segment::Line(last.end(), start));
        }
        // The last end is the first start exactly, so the contour closes on one point.
        if let (Some(start), Some(last)) = (self.start, self.current.last_mut()) {
            match last {
                Segment::Line(_, b) => *b = start,
                Segment::Cubic(c) => c[3] = start,
            }
        }
        let contour = std::mem::take(&mut self.current);
        if !contour.is_empty() {
            self.contours.push(contour);
        }
        self.start = None;
    }
}

impl skrifa::outline::OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.start.is_some() {
            self.finish();
        }
        let p = self.map(x, y);
        self.start = Some(p);
        self.at = p;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.push(Segment::Line(self.at, p));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (c, p) = (self.map(cx0, cy0), self.map(x, y));
        let a = self.at;
        let k = 2.0 / 3.0;
        self.push(Segment::Cubic([a, a + (c - a) * k, p + (c - p) * k, p]));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (c0, c1, p) = (self.map(cx0, cy0), self.map(cx1, cy1), self.map(x, y));
        self.push(Segment::Cubic([self.at, c0, c1, p]));
    }

    fn close(&mut self) {
        self.finish();
    }
}

/// The outlines of `text` in `font` (TrueType or OpenType bytes), `height` mm tall
/// capitals, starting at `at` on the baseline. See the module documentation.
pub fn text_outline(
    font: &[u8],
    text: &str,
    at: Point2,
    height: f64,
    align: Align,
) -> Result<TextOutline, FontError> {
    use skrifa::MetadataProvider;
    use skrifa::instance::{LocationRef, Size};
    use skrifa::outline::DrawSettings;
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(FontError::BadText(format!(
            "a text may have at most {MAX_TEXT_CHARS} characters"
        )));
    }
    if text.trim().is_empty() {
        return Err(FontError::BadText("the text is empty".into()));
    }
    if !(height.is_finite() && height > 0.0 && height <= chamfera_geom::MAX_COORDINATE) {
        return Err(FontError::BadText(format!(
            "`height` must be a number above 0 and at most {} mm",
            chamfera_geom::MAX_COORDINATE
        )));
    }
    let f = skrifa::FontRef::new(font).map_err(|e| FontError::BadFont(e.to_string()))?;
    let metrics = f.metrics(Size::unscaled(), LocationRef::default());
    let upem = f64::from(metrics.units_per_em);
    if upem <= 0.0 {
        return Err(FontError::BadFont("the font has no units per em".into()));
    }
    let charmap = f.charmap();
    let outlines = f.outline_glyphs();
    let advances = f.glyph_metrics(Size::unscaled(), LocationRef::default());
    // Capitals `height` tall: the cap height, else the top of an H, else 0.7 em.
    let cap = metrics
        .cap_height
        .map(f64::from)
        .filter(|c| c.is_finite() && *c > 0.0)
        .or_else(|| {
            let h = charmap.map('H')?;
            advances
                .bounds(h)
                .map(|b| f64::from(b.y_max))
                .filter(|c| c.is_finite() && *c > 0.0)
        })
        .unwrap_or(0.7 * upem);
    let scale = height / cap;
    let mut out = TextOutline::default();
    let mut pen = Pen {
        scale,
        ..Pen::default()
    };
    for (row, line) in text.split('\n').enumerate() {
        // The line's advance first, for the alignment.
        let glyphs: Vec<(char, Option<skrifa::GlyphId>)> =
            line.chars().map(|c| (c, charmap.map(c))).collect();
        let advance_of = |g: Option<skrifa::GlyphId>| {
            g.and_then(|g| advances.advance_width(g))
                .map(f64::from)
                .filter(|a| a.is_finite())
                .unwrap_or(0.5 * upem)
        };
        let width: f64 = glyphs.iter().map(|(_, g)| advance_of(*g) * scale).sum();
        out.width = out.width.max(width);
        let shift = match align {
            Align::Left => 0.0,
            Align::Centre => -0.5 * width,
            Align::Right => -width,
        };
        let mut x = at.x + shift;
        let y = at.y - row as f64 * LINE_SPACING * height;
        for (ch, g) in glyphs {
            let Some(g) = g else {
                if !ch.is_whitespace() && !out.missing.contains(&ch) {
                    out.missing.push(ch);
                }
                x += advance_of(None) * scale;
                continue;
            };
            if let Some(glyph) = outlines.get(g) {
                pen.origin = Point2::new(x, y);
                glyph
                    .draw(
                        DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
                        &mut pen,
                    )
                    .map_err(|e| FontError::BadFont(format!("glyph {ch:?}: {e}")))?;
                if pen.start.is_some() {
                    pen.finish();
                }
                if pen.bad {
                    return Err(FontError::BadFont(format!(
                        "glyph {ch:?} has a coordinate that is not a number"
                    )));
                }
                if pen.segments > MAX_TEXT_SEGMENTS {
                    return Err(FontError::BadText(format!(
                        "the text's outlines have more than {MAX_TEXT_SEGMENTS} pieces"
                    )));
                }
            }
            x += advance_of(Some(g)) * scale;
        }
    }
    out.contours = pen.contours;
    if out
        .contours
        .iter()
        .flatten()
        .flat_map(|s| match s {
            Segment::Line(a, b) => vec![*a, *b],
            Segment::Cubic(c) => c.to_vec(),
        })
        .any(|p| {
            p.x.abs() > chamfera_geom::MAX_COORDINATE || p.y.abs() > chamfera_geom::MAX_COORDINATE
        })
    {
        return Err(FontError::BadText(format!(
            "the text reaches more than {} mm from the origin",
            chamfera_geom::MAX_COORDINATE
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(o: &TextOutline) -> (Point2, Point2) {
        let mut lo = Point2::new(f64::INFINITY, f64::INFINITY);
        let mut hi = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for s in o.contours.iter().flatten() {
            for p in [s.start(), s.end()] {
                lo = Point2::new(lo.x.min(p.x), lo.y.min(p.y));
                hi = Point2::new(hi.x.max(p.x), hi.y.max(p.y));
            }
        }
        (lo, hi)
    }

    #[test]
    fn an_h_is_one_closed_contour_as_tall_as_the_height() {
        // An H of straight strokes is one closed contour of lines; its top is the cap
        // height, so with height 10 it reaches from the baseline y = 0 to y = 10.
        let o = text_outline(DEFAULT_FONT, "H", Point2::new(0.0, 0.0), 10.0, Align::Left).unwrap();
        assert_eq!(o.contours.len(), 1, "{o:?}");
        let c = &o.contours[0];
        assert!(c.iter().all(|s| matches!(s, Segment::Line(..))));
        for w in c.windows(2) {
            assert_eq!(w[0].end(), w[1].start());
        }
        assert_eq!(c.last().unwrap().end(), c[0].start());
        let (lo, hi) = bounds(&o);
        assert!(
            lo.y.abs() < 1e-9 && (hi.y - 10.0).abs() < 1e-9,
            "{lo:?} {hi:?}"
        );
    }

    #[test]
    fn an_o_has_an_outer_and_an_inner_contour_and_text_aligns() {
        let o = text_outline(DEFAULT_FONT, "O", Point2::new(0.0, 0.0), 10.0, Align::Left).unwrap();
        assert_eq!(o.contours.len(), 2);
        assert!(
            o.contours
                .iter()
                .flatten()
                .any(|s| matches!(s, Segment::Cubic(..)))
        );
        // Centred and right-aligned text move left by half and all of the width.
        let l = text_outline(DEFAULT_FONT, "Hi", Point2::new(0.0, 0.0), 5.0, Align::Left).unwrap();
        let c = text_outline(
            DEFAULT_FONT,
            "Hi",
            Point2::new(0.0, 0.0),
            5.0,
            Align::Centre,
        )
        .unwrap();
        let r = text_outline(DEFAULT_FONT, "Hi", Point2::new(0.0, 0.0), 5.0, Align::Right).unwrap();
        let (bl, bc, br) = (bounds(&l).0.x, bounds(&c).0.x, bounds(&r).0.x);
        assert!((bl - bc - 0.5 * l.width).abs() < 1e-9 && (bl - br - l.width).abs() < 1e-9);
    }

    #[test]
    fn bad_input_is_refused_without_a_panic() {
        let at = Point2::new(0.0, 0.0);
        assert!(text_outline(b"not a font", "H", at, 10.0, Align::Left).is_err());
        assert!(text_outline(&[], "H", at, 10.0, Align::Left).is_err());
        assert!(text_outline(DEFAULT_FONT, "", at, 10.0, Align::Left).is_err());
        assert!(text_outline(DEFAULT_FONT, "H", at, -1.0, Align::Left).is_err());
        assert!(text_outline(DEFAULT_FONT, "H", at, f64::NAN, Align::Left).is_err());
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(text_outline(DEFAULT_FONT, &long, at, 10.0, Align::Left).is_err());
        let far = Point2::new(chamfera_geom::MAX_COORDINATE, 0.0);
        assert!(text_outline(DEFAULT_FONT, "H", far, 10.0, Align::Left).is_err());
        // A character the font lacks is reported and skipped.
        let o = text_outline(DEFAULT_FONT, "H\u{E000}", at, 10.0, Align::Left).unwrap();
        assert_eq!(o.missing, vec!['\u{E000}']);
        // Truncated font bytes.
        let cut = &DEFAULT_FONT[..DEFAULT_FONT.len() / 3];
        let _ = text_outline(cut, "Hg", at, 10.0, Align::Left);
    }
}
