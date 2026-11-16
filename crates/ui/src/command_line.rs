//! The DRAWING workspace's command line: typed commands, short aliases and typed points.
//!
//! A line of text is a verb (by alias, such as `L` for a line) and its inputs, or a
//! registered command id with JSON parameters (`drawing.layer.add {"name": "Axes"}`).
//! Verbs that need points ask for them one at a time; each input is typed (`x,y`,
//! `@dx,dy` from the last point, `@length<angle` or `length<angle` in degrees) or clicked
//! on the sheet. Everything ends in a registered command, so typing, clicking the ribbon
//! and scripts make the same design. The aliases are ours ([`ALIASES`]).
//!
//! Parsing never panics and never runs anything itself: [`CommandLine::submit`] and
//! [`CommandLine::click`] return [`Act`]s that the app carries out. Input is capped at
//! [`MAX_INPUT`] characters and numbers must be finite and on the sheet.

use chamfera_doc::drawing::MAX_SHEET_COORDINATE;
use chamfera_drawing::Shape;
use chamfera_geom::Point2;
use serde_json::{Value, json};

/// Longest line the command line takes.
pub(crate) const MAX_INPUT: usize = 512;
/// Lines of history kept.
const MAX_HISTORY: usize = 64;

/// What a verb does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verb {
    Line,
    Circle,
    Arc,
    Polyline,
    Rectangle,
    Move,
    Copy,
    Erase,
    /// No inputs: the Layers panel; one: make that layer current.
    Layer,
    /// A new layer, made current: name, then optional line type and `#rrggbb`.
    LayerNew,
    LtScale,
    Select,
    Undo,
    Redo,
    Help,
}

/// Our aliases and full names, matched without case.
pub(crate) const ALIASES: [(&str, Verb); 32] = [
    ("L", Verb::Line),
    ("LINE", Verb::Line),
    ("C", Verb::Circle),
    ("CIRCLE", Verb::Circle),
    ("A", Verb::Arc),
    ("ARC", Verb::Arc),
    ("PL", Verb::Polyline),
    ("POLYLINE", Verb::Polyline),
    ("REC", Verb::Rectangle),
    ("RECT", Verb::Rectangle),
    ("RECTANGLE", Verb::Rectangle),
    ("M", Verb::Move),
    ("MOVE", Verb::Move),
    ("CP", Verb::Copy),
    ("COPY", Verb::Copy),
    ("E", Verb::Erase),
    ("ERASE", Verb::Erase),
    ("DEL", Verb::Erase),
    ("LA", Verb::Layer),
    ("LAYER", Verb::Layer),
    ("LN", Verb::LayerNew),
    ("NEWLAYER", Verb::LayerNew),
    ("LTS", Verb::LtScale),
    ("LTSCALE", Verb::LtScale),
    ("SEL", Verb::Select),
    ("SELECT", Verb::Select),
    ("U", Verb::Undo),
    ("UNDO", Verb::Undo),
    ("REDO", Verb::Redo),
    ("?", Verb::Help),
    ("HELP", Verb::Help),
    ("H", Verb::Help),
];

impl Verb {
    /// The verb an alias or name stands for.
    pub(crate) fn of(word: &str) -> Option<Verb> {
        ALIASES
            .iter()
            .find(|(a, _)| a.eq_ignore_ascii_case(word))
            .map(|(_, v)| *v)
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Verb::Line => "Line",
            Verb::Circle => "Circle",
            Verb::Arc => "Arc",
            Verb::Polyline => "Polyline",
            Verb::Rectangle => "Rectangle",
            Verb::Move => "Move",
            Verb::Copy => "Copy",
            Verb::Erase => "Erase",
            Verb::Layer => "Layer",
            Verb::LayerNew => "New Layer",
            Verb::LtScale => "Line Type Scale",
            Verb::Select => "Select",
            Verb::Undo => "Undo",
            Verb::Redo => "Redo",
            Verb::Help => "Help",
        }
    }

    /// The verb a ribbon tool `drawing.draw.<what>` starts.
    pub(crate) fn of_tool(what: &str) -> Option<Verb> {
        match what {
            "line" => Some(Verb::Line),
            "circle" => Some(Verb::Circle),
            "arc" => Some(Verb::Arc),
            "polyline" => Some(Verb::Polyline),
            "rectangle" => Some(Verb::Rectangle),
            _ => None,
        }
    }

    /// Takes points (typed or clicked) while pending.
    fn takes_points(self) -> bool {
        matches!(
            self,
            Verb::Line
                | Verb::Circle
                | Verb::Arc
                | Verb::Polyline
                | Verb::Rectangle
                | Verb::Move
                | Verb::Copy
        )
    }
}

/// What the app should do after an input.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Act {
    /// Run a registered command (the app adds the shown sheet to `drawing.*` drafting
    /// commands).
    Run(String, Value),
    /// Move (or copy) the selected entities.
    MoveSelection {
        dx: f64,
        dy: f64,
        copy: bool,
    },
    EraseSelection,
    SelectAll,
    SelectNone,
    Select(Vec<u32>),
    ToggleLayers,
    /// Show this in the history.
    Note(String),
    /// Show this as an error.
    Fail(String),
}

/// A verb waiting for more input.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pending {
    pub verb: Verb,
    /// Points given so far, sheet mm.
    pub points: Vec<Point2>,
    /// Segments a Line has made (so `U` can take one back).
    pub segments: usize,
}

/// The command line's state.
#[derive(Debug, Clone, Default)]
pub(crate) struct CommandLine {
    /// The text being typed.
    pub input: String,
    pub pending: Option<Pending>,
    /// What was typed and answered, oldest first.
    pub history: Vec<String>,
    /// The last verb started, repeated by Enter on an empty line.
    pub last: Option<Verb>,
    /// Take the keyboard on the next frame.
    pub focus: bool,
}

/// A finite number, as typed.
pub(crate) fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() || s.len() > 32 || !s.chars().all(|c| c.is_ascii_digit() || "+-.eE".contains(c))
    {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn on_sheet(p: Point2) -> Result<Point2, String> {
    if p.x.is_finite()
        && p.y.is_finite()
        && p.x.abs() <= MAX_SHEET_COORDINATE
        && p.y.abs() <= MAX_SHEET_COORDINATE
    {
        Ok(p)
    } else {
        Err(format!(
            "points must be within ±{MAX_SHEET_COORDINATE} mm of the sheet's corner"
        ))
    }
}

/// A typed point: `x,y`; `@dx,dy` from `last`; `length<angle` (degrees, from +x,
/// counter-clockwise) from the sheet's corner, or `@length<angle` from `last`.
pub(crate) fn parse_point(token: &str, last: Option<Point2>) -> Result<Point2, String> {
    let t = token.trim();
    let (relative, rest) = match t.strip_prefix('@') {
        Some(r) => (true, r),
        None => (false, t),
    };
    let base = if relative {
        last.unwrap_or(Point2::ORIGIN)
    } else {
        Point2::ORIGIN
    };
    let bad = || {
        format!("{t:?} is not a point: type x,y or @dx,dy or @length<angle (a point for decimals)")
    };
    let p = if let Some((len, angle)) = rest.split_once('<') {
        let len = parse_number(len).ok_or_else(bad)?;
        let deg = parse_number(angle).ok_or_else(bad)?;
        if deg.abs() > 1e6 {
            return Err(bad());
        }
        let a = deg.to_radians();
        Point2::new(base.x + len * a.cos(), base.y + len * a.sin())
    } else {
        let (x, y) = rest.split_once(',').ok_or_else(bad)?;
        let x = parse_number(x).ok_or_else(bad)?;
        let y = parse_number(y).ok_or_else(bad)?;
        Point2::new(base.x + x, base.y + y)
    };
    on_sheet(p)
}

fn split_head(text: &str) -> (&str, &str) {
    match text.split_once(char::is_whitespace) {
        Some((h, r)) => (h, r.trim()),
        None => (text, ""),
    }
}

fn line_params(a: Point2, b: Point2) -> Value {
    json!({ "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y })
}

fn points_json(points: &[Point2]) -> Value {
    Value::Array(
        points
            .iter()
            .map(|p| json!({ "x": p.x, "y": p.y }))
            .collect(),
    )
}

/// Every alias, for Help.
fn help() -> String {
    let mut out = String::from("Commands:");
    let mut seen: Vec<Verb> = Vec::new();
    for (alias, verb) in ALIASES {
        if seen.contains(&verb) {
            continue;
        }
        seen.push(verb);
        let names: Vec<&str> = ALIASES
            .iter()
            .filter(|(_, v)| *v == verb)
            .map(|(a, _)| *a)
            .collect();
        let _ = alias;
        out.push_str(&format!(" {} ({});", verb.name(), names.join(", ")));
    }
    out.push_str(" or a command id with JSON, such as drawing.layer.list {}");
    out
}

impl CommandLine {
    fn log(&mut self, line: String) {
        self.history.push(line);
        if self.history.len() > MAX_HISTORY {
            let extra = self.history.len() - MAX_HISTORY;
            self.history.drain(..extra);
        }
    }

    /// Writes the notes and errors of `acts` to the history.
    fn record(&mut self, acts: &[Act]) {
        for a in acts {
            match a {
                Act::Note(t) => self.log(t.clone()),
                Act::Fail(t) => self.log(format!("! {t}")),
                _ => {}
            }
        }
    }

    /// True while a verb waits for a point (so a click on the sheet goes here).
    pub(crate) fn wants_point(&self) -> bool {
        self.pending.as_ref().is_some_and(|p| p.verb.takes_points())
    }

    /// What the command line asks for now.
    pub(crate) fn prompt(&self) -> String {
        let Some(p) = &self.pending else {
            return "Command".into();
        };
        let n = p.points.len();
        let ask = match (p.verb, n) {
            (Verb::Line, 0) => "first point",
            (Verb::Line, _) => "next point, C closes, U takes one back, Enter ends",
            (Verb::Circle, 0) => "centre",
            (Verb::Circle, _) => "radius or a point on the circle",
            (Verb::Arc, 0) => "centre",
            (Verb::Arc, 1) => "start point (sets the radius)",
            (Verb::Arc, _) => "end point (counter-clockwise from the start)",
            (Verb::Polyline, 0) => "first point",
            (Verb::Polyline, _) => "next point, C closes, U takes one back, Enter ends",
            (Verb::Rectangle, 0) => "first corner",
            (Verb::Rectangle, _) => "opposite corner (@width,height from the first)",
            (Verb::Move | Verb::Copy, 0) => "base point",
            (Verb::Move | Verb::Copy, _) => "second point",
            (Verb::LtScale, _) => "scale (0.01 to 100)",
            _ => "input",
        };
        format!("{}: {ask}", p.verb.name())
    }

    /// Esc: drops what is pending.
    pub(crate) fn cancel(&mut self) {
        if let Some(p) = self.pending.take() {
            self.log(format!("{}: cancelled", p.verb.name()));
        }
        self.input.clear();
    }

    /// Starts a verb from the ribbon, as typing its alias would.
    pub(crate) fn start(&mut self, verb: Verb, has_selection: bool) -> Vec<Act> {
        self.log(format!("> {}", verb.name()));
        let acts = self.begin(verb, &[], has_selection);
        self.record(&acts);
        self.focus = true;
        acts
    }

    /// A line typed and Enter pressed.
    pub(crate) fn submit(&mut self, text: &str, has_selection: bool) -> Vec<Act> {
        let text: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_INPUT)
            .collect();
        let trimmed = text.trim();
        self.log(format!("> {trimmed}"));
        let (head, _) = split_head(trimmed);
        let keyword = matches!(
            head.to_ascii_uppercase().as_str(),
            "C" | "CLOSE" | "U" | "UNDO"
        );
        // A new verb (not one of the pending verb's keywords) ends the pending one first.
        let interrupts = self.pending.is_some()
            && !keyword
            && parse_number(head).is_none()
            && (Verb::of(head).is_some() || head.contains('.'));
        let acts = if trimmed.is_empty() {
            self.finish(has_selection)
        } else if interrupts {
            let mut acts = self.finish(has_selection);
            acts.extend(self.fresh(trimmed, has_selection));
            acts
        } else if self.pending.is_some() {
            let mut acts = Vec::new();
            for token in trimmed.split_whitespace().take(MAX_INPUT) {
                let more = self.feed(token);
                let failed = more.iter().any(|a| matches!(a, Act::Fail(_)));
                acts.extend(more);
                if failed || self.pending.is_none() {
                    break;
                }
            }
            acts
        } else {
            self.fresh(trimmed, has_selection)
        };
        self.record(&acts);
        acts
    }

    /// A line typed with nothing pending: a verb and its inputs, or a command id.
    fn fresh(&mut self, trimmed: &str, has_selection: bool) -> Vec<Act> {
        let (head, rest) = split_head(trimmed);
        if head.contains('.') {
            return vec![raw_command(head, rest)];
        }
        match Verb::of(head) {
            Some(v) => {
                let args: Vec<&str> = rest.split_whitespace().take(MAX_INPUT).collect();
                self.begin(v, &args, has_selection)
            }
            None => vec![Act::Fail(format!(
                "unknown command {:?}: type ? for the list",
                head.chars().take(40).collect::<String>()
            ))],
        }
    }

    /// A click on the sheet at `p` while a verb waits for a point.
    pub(crate) fn click(&mut self, p: Point2) -> Vec<Act> {
        let acts = match on_sheet(p) {
            Ok(p) if self.wants_point() => {
                self.log(format!("> {:.3},{:.3}", p.x, p.y));
                self.point(p)
            }
            Ok(_) => Vec::new(),
            Err(e) => vec![Act::Fail(e)],
        };
        self.record(&acts);
        acts
    }

    fn begin(&mut self, verb: Verb, args: &[&str], has_selection: bool) -> Vec<Act> {
        self.pending = None;
        match verb {
            Verb::Help => vec![Act::Note(help())],
            Verb::Undo => vec![Act::Run("edit.undo".into(), json!({}))],
            Verb::Redo => vec![Act::Run("edit.redo".into(), json!({}))],
            Verb::Erase if !has_selection => {
                vec![Act::Fail(
                    "Erase: select entities on the sheet first".into(),
                )]
            }
            Verb::Erase => vec![Act::EraseSelection],
            Verb::Layer => match args.first() {
                None => vec![Act::ToggleLayers],
                Some(name) => vec![Act::Run(
                    "drawing.layer.current".into(),
                    json!({ "layer": name }),
                )],
            },
            Verb::LayerNew => {
                let Some(name) = args.first() else {
                    return vec![Act::Fail(
                        "New Layer: type LN NAME, then a line type and #rrggbb if wanted".into(),
                    )];
                };
                let mut p = json!({ "name": name, "current": true });
                for a in args.iter().skip(1) {
                    if a.starts_with('#') {
                        p["colour"] = json!(a);
                    } else if let Ok(w) = a.parse::<f64>()
                        && w.is_finite()
                    {
                        p["weight"] = json!(w);
                    } else {
                        p["line_type"] = json!(a);
                    }
                }
                vec![Act::Run("drawing.layer.add".into(), p)]
            }
            Verb::Select => match args.first().map(|a| a.to_ascii_uppercase()) {
                Some(a) if a == "ALL" => vec![Act::SelectAll],
                Some(a) if a == "NONE" => vec![Act::SelectNone],
                Some(_) => {
                    let ids: Option<Vec<u32>> = args
                        .iter()
                        .map(|a| a.parse::<u32>().ok().filter(|v| *v > 0))
                        .collect();
                    match ids {
                        Some(ids) => vec![Act::Select(ids)],
                        None => vec![Act::Fail(
                            "Select: type SEL ALL, SEL NONE or entity ids".into(),
                        )],
                    }
                }
                None => vec![Act::Fail(
                    "Select: type SEL ALL, SEL NONE or entity ids".into(),
                )],
            },
            Verb::Move | Verb::Copy if !has_selection => vec![Act::Fail(format!(
                "{}: select entities on the sheet first",
                verb.name()
            ))],
            _ => {
                self.last = Some(verb);
                self.pending = Some(Pending {
                    verb,
                    points: Vec::new(),
                    segments: 0,
                });
                let mut acts = Vec::new();
                for a in args {
                    let more = self.feed(a);
                    let failed = more.iter().any(|x| matches!(x, Act::Fail(_)));
                    acts.extend(more);
                    if failed || self.pending.is_none() {
                        break;
                    }
                }
                acts
            }
        }
    }

    /// Enter on an empty line: ends a line or polyline, or repeats the last verb.
    fn finish(&mut self, has_selection: bool) -> Vec<Act> {
        match self.pending.take() {
            Some(p) if p.verb == Verb::Polyline && p.points.len() >= 2 => vec![Act::Run(
                "drawing.draw.polyline".into(),
                json!({ "points": points_json(&p.points), "closed": false }),
            )],
            Some(p) if p.verb == Verb::Line => {
                vec![Act::Note(format!("Line: {} segments", p.segments))]
            }
            Some(p) => vec![Act::Note(format!("{}: cancelled", p.verb.name()))],
            None => match self.last {
                Some(v) => self.begin(v, &[], has_selection),
                None => Vec::new(),
            },
        }
    }

    /// One input for the pending verb.
    fn feed(&mut self, token: &str) -> Vec<Act> {
        let Some(p) = self.pending.as_mut() else {
            return Vec::new();
        };
        let word = token.to_ascii_uppercase();
        match (p.verb, word.as_str()) {
            (Verb::Line | Verb::Polyline, "C" | "CLOSE") => {
                let closed = p.points.len() >= 3 || (p.verb == Verb::Line && p.points.len() >= 2);
                if !closed {
                    return vec![Act::Fail(format!(
                        "{}: close needs at least {} points",
                        p.verb.name(),
                        if p.verb == Verb::Line { 2 } else { 3 }
                    ))];
                }
                let Some(pending) = self.pending.take() else {
                    return Vec::new();
                };
                return match pending.verb {
                    Verb::Line => match (pending.points.last(), pending.points.first()) {
                        (Some(a), Some(b)) => {
                            vec![Act::Run("drawing.draw.line".into(), line_params(*a, *b))]
                        }
                        _ => Vec::new(),
                    },
                    _ => vec![Act::Run(
                        "drawing.draw.polyline".into(),
                        json!({ "points": points_json(&pending.points), "closed": true }),
                    )],
                };
            }
            (Verb::Line, "U" | "UNDO") => {
                if p.points.pop().is_none() {
                    return vec![Act::Fail("Line: nothing to take back".into())];
                }
                if p.segments > 0 {
                    p.segments -= 1;
                    return vec![Act::Run("edit.undo".into(), json!({}))];
                }
                return Vec::new();
            }
            (Verb::Polyline, "U" | "UNDO") => {
                p.points.pop();
                return Vec::new();
            }
            (Verb::LtScale, _) => {
                return match parse_number(token) {
                    Some(k) => {
                        self.pending = None;
                        vec![Act::Run(
                            "drawing.linetype_scale".into(),
                            json!({ "scale": k }),
                        )]
                    }
                    None => vec![Act::Fail(format!("{token:?} is not a number"))],
                };
            }
            (Verb::Circle, _)
                if p.points.len() == 1 && !token.contains(',') && !token.contains('<') =>
            {
                let centre = p.points.first().copied();
                return match (parse_number(token), centre) {
                    (Some(r), Some(c)) if r > 0.0 && r <= MAX_SHEET_COORDINATE => {
                        self.pending = None;
                        vec![Act::Run(
                            "drawing.draw.circle".into(),
                            json!({ "cx": c.x, "cy": c.y, "radius": r }),
                        )]
                    }
                    _ => vec![Act::Fail(format!(
                        "{token:?} is not a radius: a number above 0, or a point"
                    ))],
                };
            }
            _ => {}
        }
        let last = p.points.last().copied();
        match parse_point(token, last) {
            Ok(q) => self.point(q),
            Err(e) => vec![Act::Fail(e)],
        }
    }

    /// One point for the pending verb.
    fn point(&mut self, q: Point2) -> Vec<Act> {
        let Some(p) = self.pending.as_mut() else {
            return Vec::new();
        };
        let mut acts = Vec::new();
        match p.verb {
            Verb::Line => {
                if let Some(prev) = p.points.last().copied() {
                    if prev.distance(q) <= 1e-9 {
                        return vec![Act::Fail("Line: the next point is the same point".into())];
                    }
                    acts.push(Act::Run("drawing.draw.line".into(), line_params(prev, q)));
                    p.segments += 1;
                }
                p.points.push(q);
            }
            Verb::Polyline => {
                if p.points.len() >= chamfera_doc::drafting::MAX_POLYLINE_POINTS {
                    return vec![Act::Fail("Polyline: too many points".into())];
                }
                p.points.push(q);
            }
            Verb::Circle => {
                if let Some(c) = p.points.first().copied() {
                    let r = c.distance(q);
                    self.pending = None;
                    acts.push(Act::Run(
                        "drawing.draw.circle".into(),
                        json!({ "cx": c.x, "cy": c.y, "radius": r }),
                    ));
                } else {
                    p.points.push(q);
                }
            }
            Verb::Arc => {
                p.points.push(q);
                if let [c, a, b] = p.points[..] {
                    self.pending = None;
                    let r = c.distance(a);
                    let start = (a.y - c.y).atan2(a.x - c.x);
                    let mut end = (b.y - c.y).atan2(b.x - c.x);
                    while end <= start + 1e-12 {
                        end += std::f64::consts::TAU;
                    }
                    acts.push(Act::Run(
                        "drawing.draw.arc".into(),
                        json!({ "cx": c.x, "cy": c.y, "radius": r, "start": start, "end": end }),
                    ));
                }
            }
            Verb::Rectangle => {
                if let Some(a) = p.points.first().copied() {
                    self.pending = None;
                    acts.push(Act::Run("drawing.draw.rectangle".into(), line_params(a, q)));
                } else {
                    p.points.push(q);
                }
            }
            Verb::Move | Verb::Copy => {
                if let Some(a) = p.points.first().copied() {
                    let copy = p.verb == Verb::Copy;
                    self.pending = None;
                    acts.push(Act::MoveSelection {
                        dx: q.x - a.x,
                        dy: q.y - a.y,
                        copy,
                    });
                } else {
                    p.points.push(q);
                }
            }
            _ => {}
        }
        acts
    }

    /// What the pending verb would draw with the pointer at `hover` (sheet mm).
    pub(crate) fn preview(&self, hover: Point2) -> Vec<Shape> {
        let Some(p) = &self.pending else {
            return Vec::new();
        };
        let line = |a: Point2, b: Point2| Shape::Line { a, b };
        match (p.verb, p.points.as_slice()) {
            (Verb::Line | Verb::Move | Verb::Copy, [.., last]) => vec![line(*last, hover)],
            (Verb::Polyline, pts @ [.., last]) => {
                let mut v: Vec<Shape> = pts.windows(2).map(|w| line(w[0], w[1])).collect();
                v.push(line(*last, hover));
                v
            }
            (Verb::Circle, [c]) => vec![
                Shape::Circle {
                    centre: *c,
                    radius: c.distance(hover).max(1e-6),
                },
                line(*c, hover),
            ],
            (Verb::Arc, [c]) => vec![line(*c, hover)],
            (Verb::Arc, [c, a]) => {
                let start = (a.y - c.y).atan2(a.x - c.x);
                let mut end = (hover.y - c.y).atan2(hover.x - c.x);
                while end <= start + 1e-12 {
                    end += std::f64::consts::TAU;
                }
                vec![Shape::Arc {
                    centre: *c,
                    radius: c.distance(*a).max(1e-6),
                    start,
                    end,
                }]
            }
            (Verb::Rectangle, [a]) => {
                let (b, c, d) = (Point2::new(hover.x, a.y), hover, Point2::new(a.x, hover.y));
                vec![line(*a, b), line(b, c), line(c, d), line(d, *a)]
            }
            _ => Vec::new(),
        }
    }
}

/// `id {json}`: a registered command with its parameters.
fn raw_command(id: &str, rest: &str) -> Act {
    let known = chamfera_engine::registry().iter().any(|c| c.id == id);
    if !known {
        return Act::Fail(format!(
            "no command {:?}: type ? for the aliases",
            id.chars().take(64).collect::<String>()
        ));
    }
    if rest.is_empty() {
        return Act::Run(id.to_owned(), json!({}));
    }
    match serde_json::from_str::<Value>(rest) {
        Ok(v @ Value::Object(_)) => Act::Run(id.to_owned(), v),
        Ok(_) => Act::Fail(format!("{id}: the parameters must be a JSON object")),
        Err(e) => Act::Fail(format!("{id}: the parameters are not JSON ({e})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    fn runs(acts: &[Act]) -> Vec<(String, Value)> {
        acts.iter()
            .filter_map(|a| match a {
                Act::Run(id, v) => Some((id.clone(), v.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn typed_points_are_absolute_relative_and_polar() {
        assert_eq!(parse_point("10,20", None).unwrap(), p(10.0, 20.0));
        assert_eq!(parse_point(" -5.5,+2 ", None).unwrap(), p(-5.5, 2.0));
        assert_eq!(
            parse_point("@3,4", Some(p(10.0, 10.0))).unwrap(),
            p(13.0, 14.0)
        );
        // @50<90 from (10, 10): straight up 50 mm.
        let q = parse_point("@50<90", Some(p(10.0, 10.0))).unwrap();
        assert!(q.distance(p(10.0, 60.0)) < 1e-9);
        // 10<180 from the corner: (-10, 0).
        assert!(parse_point("10<180", None).unwrap().distance(p(-10.0, 0.0)) < 1e-9);
        for bad in [
            "", "10", "a,b", "1,2,3", "nan,1", "inf,0", "1e400,0", "0x10,1", "1,", ",1", "@", "<",
            "1<", "@1<1e9", "20000,0", "١,٢",
        ] {
            assert!(parse_point(bad, Some(p(0.0, 0.0))).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn aliases_route_to_registered_commands() {
        let mut c = CommandLine::default();
        // L with three typed points: two line segments, the second from the first's end.
        let acts = c.submit("L 0,0 50,0 @0,30", false);
        let r = runs(&acts);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].0, "drawing.draw.line");
        assert_eq!(r[0].1, json!({"x1": 0.0, "y1": 0.0, "x2": 50.0, "y2": 0.0}));
        assert_eq!(
            r[1].1,
            json!({"x1": 50.0, "y1": 0.0, "x2": 50.0, "y2": 30.0})
        );
        assert!(c.wants_point());
        // C closes back to the first point and ends.
        let r = runs(&c.submit("c", false));
        assert_eq!(
            r[0].1,
            json!({"x1": 50.0, "y1": 30.0, "x2": 0.0, "y2": 0.0})
        );
        assert!(c.pending.is_none());

        // Circle by centre and radius, and by a point on it: radius |(3, 4)| = 5.
        let r = runs(&c.submit("C 100,100 25", false));
        assert_eq!(
            r[0],
            (
                "drawing.draw.circle".into(),
                json!({"cx": 100.0, "cy": 100.0, "radius": 25.0})
            )
        );
        let r = runs(&c.submit("circle 0,0 @3,4", false));
        assert_eq!(r[0].1["radius"], 5.0);

        // Arc: centre (0,0), start (10,0), end (0,10): r 10 from 0 to pi/2.
        let r = runs(&c.submit("A 0,0 10,0 0,10", false));
        assert_eq!(r[0].0, "drawing.draw.arc");
        assert_eq!(r[0].1["radius"], 10.0);
        assert!((r[0].1["end"].as_f64().unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-12);

        // Polyline: points, then an empty Enter ends it open.
        assert!(runs(&c.submit("PL 0,0 10,0 10,10", false)).is_empty());
        let r = runs(&c.submit("", false));
        assert_eq!(r[0].0, "drawing.draw.polyline");
        assert_eq!(r[0].1["points"].as_array().map(Vec::len), Some(3));
        assert_eq!(r[0].1["closed"], false);

        // Rectangle with a relative corner.
        let r = runs(&c.submit("rec 20,20 @60,30", false));
        assert_eq!(
            r[0].1,
            json!({"x1": 20.0, "y1": 20.0, "x2": 80.0, "y2": 50.0})
        );

        // Layers and scale.
        assert_eq!(c.submit("LA", false), vec![Act::ToggleLayers]);
        let r = runs(&c.submit("LN Axes center #ff0000 0.35", false));
        assert_eq!(
            r[0].1,
            json!({"name": "Axes", "current": true, "line_type": "center", "colour": "#ff0000", "weight": 0.35})
        );
        let r = runs(&c.submit("la Axes", false));
        assert_eq!(
            r[0],
            ("drawing.layer.current".into(), json!({"layer": "Axes"}))
        );
        let r = runs(&c.submit("LTS 2.5", false));
        assert_eq!(
            r[0],
            ("drawing.linetype_scale".into(), json!({"scale": 2.5}))
        );

        // A registered id with JSON goes through as is.
        let r = runs(&c.submit(r#"drawing.layer.add {"name": "Notes"}"#, false));
        assert_eq!(r[0], ("drawing.layer.add".into(), json!({"name": "Notes"})));
    }

    #[test]
    fn move_needs_a_selection_and_clicks_feed_points() {
        let mut c = CommandLine::default();
        assert!(matches!(c.submit("M", false)[..], [Act::Fail(_)]));
        assert!(matches!(c.submit("E", false)[..], [Act::Fail(_)]));
        assert_eq!(c.submit("E", true), vec![Act::EraseSelection]);
        assert!(c.submit("M", true).is_empty());
        assert!(c.click(p(10.0, 10.0)).is_empty());
        assert_eq!(
            c.click(p(15.0, 7.0)),
            vec![Act::MoveSelection {
                dx: 5.0,
                dy: -3.0,
                copy: false
            }]
        );
        // A ribbon tool starts the same verb; clicks then draw.
        c.start(Verb::Rectangle, false);
        c.click(p(0.0, 0.0));
        let r = runs(&c.click(p(10.0, 5.0)));
        assert_eq!(r[0].0, "drawing.draw.rectangle");
        // Enter on an empty line repeats the last verb.
        c.submit("", false);
        assert_eq!(c.pending.as_ref().map(|p| p.verb), Some(Verb::Rectangle));
        c.cancel();
        assert!(c.pending.is_none());
        // A new verb typed while a line waits ends the line and starts the verb; C
        // there still closes the line.
        c.submit("L 0,0 10,0", false);
        let r = runs(&c.submit("REC 0,0 5,5", false));
        assert_eq!(r[0].0, "drawing.draw.rectangle");
        c.submit("L 0,0 10,0 10,10", false);
        let r = runs(&c.submit("C", false));
        assert_eq!(
            r[0].1,
            json!({"x1": 10.0, "y1": 10.0, "x2": 0.0, "y2": 0.0})
        );
    }

    #[test]
    fn hostile_input_never_panics() {
        let long = "9".repeat(10_000);
        let nested = "[".repeat(10_000);
        let inputs = [
            String::new(),
            " ".into(),
            "\u{0}\u{7}\u{1b}[2J".into(),
            long.clone(),
            format!("L {long},{long}"),
            format!("L 0,0 {}", "@1,1 ".repeat(2000)),
            "PL ".to_string() + &"1,1 ".repeat(5000),
            "L nan,nan inf,-inf 1e308,1e308".into(),
            "C 0,0 -5".into(),
            "C 0,0 0".into(),
            "C 0,0 1e400".into(),
            "A 0,0 0,0 0,0".into(),
            "A 1,1 1,1 2,2".into(),
            "REC 0,0 0,0".into(),
            "drawing.layer.add".into(),
            format!("drawing.layer.add {nested}"),
            "drawing.layer.add {\"name\": ".into(),
            "drawing.layer.add [1,2]".into(),
            "no.such.command {}".into(),
            "SEL 0 -1 99999999999".into(),
            "LTS".into(),
            "LTS abc".into(),
            "LN".into(),
            "LN \u{202e}evil #zz".into(),
            "日本語 ①,②".into(),
            "@<,".into(),
            "L @1<1e300".into(),
            "L 1<nan".into(),
            "?".into(),
            "U".into(),
        ];
        for text in &inputs {
            let mut c = CommandLine::default();
            let _ = c.submit(text, true);
            let _ = c.submit(text, false);
            let _ = c.submit("", true);
            let _ = c.click(p(f64::NAN, 0.0));
            let _ = c.click(p(1e300, 1e300));
            let _ = c.preview(p(f64::INFINITY, 0.0));
            let _ = c.prompt();
            assert!(c.history.len() <= MAX_HISTORY);
        }
        // Every run a hostile line produces still has finite numbers on the sheet.
        let mut c = CommandLine::default();
        for a in c.submit("L 0,0 1e400,0 nan,1 5,5", false) {
            if let Act::Run(_, v) = a {
                for k in ["x1", "y1", "x2", "y2"] {
                    assert!(v[k].as_f64().is_some_and(f64::is_finite));
                }
            }
        }
    }
}
