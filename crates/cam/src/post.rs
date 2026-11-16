//! The post processor: toolpaths to G-code, for a controller described as data.
//!
//! The G-code words come from the public NIST RS274/NGC interpreter specification,
//! version 3 (Kramer, Proctor and Messina, NISTIR 6556, 2000): G20/G21 inches or
//! millimetres, G90 absolute distances, G17 the XY plane, G0 rapid, G1 line, G2/G3 arcs
//! with the centre as I and J offsets from the start (or the radius as R, negative for
//! more than a half turn), F feed, S spindle speed, M3/M5 spindle on and off, M7/M8/M9
//! coolant, `T n M6` tool change, G43 tool length offset, G98 with the G81 (drill), G83
//! (peck drill) and G84 (tapping) canned cycles cancelled by G80, and M30 program end;
//! G33.1 rigid tapping and G91.1 incremental arc centres are documented by the
//! controllers that have them, M29 (rigid tapping before G84), % tape marks, O program
//! numbers and N sequence numbers by ISO 6983-1.
//!
//! What differs between controllers is data, a [`PostDef`]: the program's opening and
//! closing lines, how a tool is changed, coolant codes, arcs as I/J, R or short lines,
//! which canned cycles there are (the rest are written out as G0 and G1 moves), tapping,
//! units, decimals, line numbers and comments. The posts that ship are JSON files in
//! `crates/cam/posts/` ([`builtin_posts`]); a user's own is the same JSON
//! ([`PostDef::from_json`]).

use crate::CamError;
use crate::toolpath::{Move, Toolpath, expand_move};
use chamfera_geom::{Point2, Point3};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// The `format` of a post file.
pub const POST_FORMAT: &str = "chamfera-post";
/// Largest post file (bytes).
pub const MAX_POST_BYTES: usize = 64 * 1024;
/// Most lines one template may hold.
pub const MAX_TEMPLATE_LINES: usize = 64;
/// Longest template line (characters).
pub const MAX_TEMPLATE_CHARS: usize = 256;
/// Most G-code lines one program may have.
pub const MAX_LINES: usize = 20_000_000;

/// The posts that ship with Chamfera, as their JSON.
const BUILTIN: [&str; 5] = [
    include_str!("../posts/linuxcnc.json"),
    include_str!("../posts/grbl.json"),
    include_str!("../posts/pc-mill.json"),
    include_str!("../posts/iso-mill.json"),
    include_str!("../posts/vmc-mill.json"),
];

/// The units the G-code is written in (G21 or G20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutUnits {
    #[default]
    Mm,
    Inch,
}

impl OutUnits {
    pub fn parse(s: &str) -> Option<OutUnits> {
        match s {
            "mm" | "metric" => Some(OutUnits::Mm),
            "inch" | "in" => Some(OutUnits::Inch),
            _ => None,
        }
    }

    fn scale(self) -> f64 {
        match self {
            OutUnits::Mm => 1.0,
            OutUnits::Inch => 1.0 / 25.4,
        }
    }

    fn word(self) -> &'static str {
        match self {
            OutUnits::Mm => "G21",
            OutUnits::Inch => "G20",
        }
    }
}

/// How comments are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommentStyle {
    /// `(text)`.
    #[default]
    Parens,
    /// `; text`, to the end of the line.
    Semicolon,
    /// Left out.
    None,
}

/// How arcs are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArcStyle {
    /// G2/G3 with the centre as I and J offsets from the start.
    #[default]
    Ij,
    /// G2/G3 with the radius R (negative beyond a half turn; full turns in two halves).
    R,
    /// Short G1 lines within `arc_tolerance` of the arc.
    Linear,
}

/// N sequence numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineNumbers {
    pub start: u32,
    pub step: u32,
    /// After this the numbers start again at `start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
}

/// Coolant codes; a missing one is not available on the controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CoolantCodes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flood: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off: Option<String>,
}

/// How a tapping cycle is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TapStyle {
    /// `G84 X Y Z R F` (with the post's `before` lines, such as `M29 S..` for rigid
    /// tapping on controls that want it).
    G84,
    /// Rigid tapping by spindle-synchronised motion, `G33.1 Z K` (pitch K) from the
    /// retract plane; the tap comes back out to where it started.
    #[serde(rename = "g33_1")]
    G33_1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tapping {
    pub style: TapStyle,
    /// Lines before the first hole of a tapping cycle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<String>,
}

/// Which canned cycles the controller has; the rest are written out as moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cycles {
    /// G81.
    #[serde(default)]
    pub drill: bool,
    /// G83.
    #[serde(default)]
    pub peck: bool,
    /// The retract mode word written before each cycle (G98: back to the height before
    /// the cycle, which is how the toolpaths are made).
    #[serde(default = "g98")]
    pub retract: String,
    /// `None`: the controller cannot tap, and a tapping operation is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tapping: Option<Tapping>,
}

fn g98() -> String {
    "G98".into()
}

fn default_decimals() -> u32 {
    3
}

fn default_arc_tolerance() -> f64 {
    0.01
}

fn yes() -> bool {
    true
}

/// A post processor as data. Template lines may use `{program}`, `{program_number}`,
/// `{tool}`, `{tool_name}`, `{rpm}`, `{units}` (G21 or G20) and `{comment:text}`
/// (the text, itself with placeholders, as a comment in the post's style).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostDef {
    /// Always `chamfera-post`.
    pub format: String,
    /// Always 1.
    pub version: u32,
    /// The short name it is chosen by: lower-case letters, digits, `-` and `_`.
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Other names it answers to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub units: OutUnits,
    /// Decimals of millimetre coordinates and feeds (one more in inches).
    #[serde(default = "default_decimals")]
    pub decimals: u32,
    /// Every coordinate has a decimal point (`X10.`): controls that read `X10` as ten
    /// least increments need it.
    #[serde(default)]
    pub trailing_point: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<LineNumbers>,
    #[serde(default)]
    pub comments: CommentStyle,
    /// Comments in capitals.
    #[serde(default)]
    pub upper_case: bool,
    /// A comment with each operation's name before it.
    #[serde(default = "yes")]
    pub operation_comments: bool,
    /// Lines before everything, never numbered (`%`, the O number).
    #[serde(default)]
    pub header: Vec<String>,
    /// The safe start: modes set before any motion.
    #[serde(default)]
    pub start: Vec<String>,
    /// The first tool's change; default `tool_change`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_tool_change: Option<Vec<String>>,
    /// A later tool change (it stops the spindle itself if it needs to).
    pub tool_change: Vec<String>,
    /// Written with the first Z move after a tool change, such as `G43 H{tool}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_offset: Option<String>,
    /// Starts the spindle, such as `S{rpm} M3`.
    pub spindle_on: String,
    #[serde(default)]
    pub coolant: CoolantCodes,
    #[serde(default)]
    pub arcs: ArcStyle,
    /// Largest distance of a line from its arc when arcs are linearised (mm).
    #[serde(default = "default_arc_tolerance")]
    pub arc_tolerance: f64,
    pub cycles: Cycles,
    /// The safe end: spindle and coolant off, home, program end.
    #[serde(default)]
    pub end: Vec<String>,
    /// Lines after everything, never numbered (`%`).
    #[serde(default)]
    pub footer: Vec<String>,
}

/// Which coolant runs while cutting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Coolant {
    #[default]
    Off,
    Flood,
    Mist,
}

impl Coolant {
    pub fn parse(s: &str) -> Option<Coolant> {
        match s {
            "none" | "off" => Some(Coolant::Off),
            "flood" => Some(Coolant::Flood),
            "mist" => Some(Coolant::Mist),
            _ => None,
        }
    }
}

/// Choices made when posting rather than in the post itself.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PostOptions {
    pub coolant: Coolant,
    /// Override the post's units.
    pub units: Option<OutUnits>,
    /// The O number where the post writes one (default 1000).
    pub program_number: Option<u32>,
}

/// The G-code and anything worth telling the user about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Posted {
    pub text: String,
    pub warnings: Vec<String>,
}

fn bad(s: impl Into<String>) -> CamError {
    CamError::Invalid(s.into())
}

/// The post the name or alias `name` names, among the built-in ones.
pub fn find_post(name: &str) -> Option<PostDef> {
    builtin_posts()
        .into_iter()
        .find(|p| p.name == name || p.aliases.iter().any(|a| a == name))
}

/// The posts that ship with Chamfera.
pub fn builtin_posts() -> Vec<PostDef> {
    BUILTIN
        .iter()
        .filter_map(|text| PostDef::from_json(text).ok())
        .collect()
}

impl PostDef {
    /// Reads and checks a post file: it is hostile input like any file.
    pub fn from_json(text: &str) -> Result<PostDef, CamError> {
        if text.len() > MAX_POST_BYTES {
            return Err(bad(format!(
                "a post file is at most {MAX_POST_BYTES} bytes"
            )));
        }
        let p: PostDef =
            serde_json::from_str(text).map_err(|e| bad(format!("not a post file: {e}")))?;
        p.validate()?;
        Ok(p)
    }

    /// Reads a post given as a JSON value (as a command parameter).
    pub fn from_value(v: &serde_json::Value) -> Result<PostDef, CamError> {
        let text = serde_json::to_string(v).map_err(|e| bad(e.to_string()))?;
        PostDef::from_json(&text)
    }

    pub fn to_json(&self) -> Result<String, CamError> {
        serde_json::to_string_pretty(self).map_err(|e| bad(e.to_string()))
    }

    /// Checks every value, and that every template expands.
    pub fn validate(&self) -> Result<(), CamError> {
        if self.format != POST_FORMAT || self.version != 1 {
            return Err(bad(format!(
                "a post file has \"format\": \"{POST_FORMAT}\" and \"version\": 1"
            )));
        }
        let name_ok = |n: &str| {
            !n.is_empty()
                && n.len() <= 64
                && n.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        };
        if !name_ok(&self.name) || !self.aliases.iter().all(|a| name_ok(a)) {
            return Err(bad(
                "a post's name and aliases are 1 to 64 lower-case letters, digits, - or _",
            ));
        }
        if self.aliases.len() > 16 {
            return Err(bad("a post has at most 16 aliases"));
        }
        for (what, s) in [("title", &self.title), ("description", &self.description)] {
            if s.chars().count() > 1000 || s.contains(['\n', '\r']) {
                return Err(bad(format!(
                    "the post's {what} is one line of at most 1000 characters"
                )));
            }
        }
        if self.decimals > 6 {
            return Err(bad("a post writes at most 6 decimals"));
        }
        if !(self.arc_tolerance.is_finite()
            && self.arc_tolerance >= 1e-4
            && self.arc_tolerance <= 1.0)
        {
            return Err(bad("the arc tolerance must be from 0.0001 to 1 mm"));
        }
        if let Some(n) = self.line_numbers {
            let max = n.max.unwrap_or(99_999_999);
            if n.step == 0 || n.step > 1000 || n.start > max || max > 99_999_999 {
                return Err(bad(
                    "line numbers need a step from 1 to 1000 and a start no larger than the \
                     maximum (at most 99999999)",
                ));
            }
        }
        let mut lists: Vec<(&str, &[String])> = vec![
            ("header", &self.header),
            ("start", &self.start),
            ("tool_change", &self.tool_change),
            ("end", &self.end),
            ("footer", &self.footer),
        ];
        if let Some(f) = &self.first_tool_change {
            lists.push(("first_tool_change", f));
        }
        if let Some(t) = &self.cycles.tapping {
            lists.push(("tapping before", &t.before));
        }
        let singles: Vec<(&str, Option<&String>)> = vec![
            ("spindle_on", Some(&self.spindle_on)),
            ("length_offset", self.length_offset.as_ref()),
            ("coolant flood", self.coolant.flood.as_ref()),
            ("coolant mist", self.coolant.mist.as_ref()),
            ("coolant off", self.coolant.off.as_ref()),
            ("cycles retract", Some(&self.cycles.retract)),
        ];
        let vars = Vars {
            program: "p",
            program_number: 1,
            tool: 1,
            tool_name: "t",
            rpm: 1,
            units: "G21",
        };
        let check = |what: &str, line: &str| -> Result<(), CamError> {
            if line.chars().count() > MAX_TEMPLATE_CHARS || line.contains(['\n', '\r']) {
                return Err(bad(format!(
                    "the post's {what} lines are single lines of at most {MAX_TEMPLATE_CHARS} \
                     characters"
                )));
            }
            self.fill(line, &vars)
                .map(|_| ())
                .map_err(|e| bad(format!("the post's {what}: {e}")))
        };
        for (what, lines) in lists {
            if lines.len() > MAX_TEMPLATE_LINES {
                return Err(bad(format!(
                    "the post's {what} has more than {MAX_TEMPLATE_LINES} lines"
                )));
            }
            for l in lines {
                check(what, l)?;
            }
        }
        for (what, line) in singles {
            if let Some(l) = line {
                check(what, l)?;
            }
        }
        Ok(())
    }

    /// `text` as a comment in this post's style, or nothing when it writes none.
    fn comment(&self, text: &str) -> String {
        let clean: String = text
            .chars()
            .filter(|c| !matches!(c, '(' | ')' | '\n' | '\r' | ';' | '%'))
            .take(120)
            .collect();
        let clean = if self.upper_case {
            clean.to_uppercase()
        } else {
            clean
        };
        match self.comments {
            CommentStyle::Parens => format!("({})", clean.trim()),
            CommentStyle::Semicolon => format!("; {}", clean.trim()),
            CommentStyle::None => String::new(),
        }
    }

    /// One template line with its placeholders filled in.
    fn fill(&self, template: &str, v: &Vars) -> Result<String, String> {
        let mut out = String::new();
        let mut rest = template;
        while let Some(i) = rest.find('{') {
            out.push_str(rest.get(..i).unwrap_or(""));
            let after = rest.get(i + 1..).unwrap_or("");
            // The matching close brace (comments may hold placeholders).
            let mut depth = 1usize;
            let mut end = None;
            for (j, c) in after.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(j) = end else {
                return Err(format!("an unclosed {{ in {template:?}"));
            };
            let key = after.get(..j).unwrap_or("");
            match key {
                "program" => out.push_str(v.program),
                "program_number" => out.push_str(&v.program_number.to_string()),
                "tool" => out.push_str(&v.tool.to_string()),
                "tool_name" => out.push_str(v.tool_name),
                "rpm" => out.push_str(&v.rpm.to_string()),
                "units" => out.push_str(v.units),
                k if k.starts_with("comment:") => {
                    let inner = self.fill(k.get(8..).unwrap_or(""), v)?;
                    out.push_str(&self.comment(&inner));
                }
                k => return Err(format!("unknown placeholder {{{k}}}")),
            }
            rest = after.get(j + 1..).unwrap_or("");
        }
        out.push_str(rest);
        Ok(out.trim().to_owned())
    }
}

/// The values the templates' placeholders take.
struct Vars<'a> {
    program: &'a str,
    program_number: u32,
    tool: u32,
    tool_name: &'a str,
    rpm: u64,
    units: &'a str,
}

/// The work coordinate system: G-code coordinates are model points measured from
/// `origin`, in axes turned by `angle` (radians, about Z) from the model's.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Wcs {
    pub origin: Point3,
    pub angle: f64,
}

impl Wcs {
    /// A model point in work coordinates.
    pub fn to_work(self, p: Point3) -> Point3 {
        let (s, c) = self.angle.sin_cos();
        let (dx, dy) = (p.x - self.origin.x, p.y - self.origin.y);
        Point3::new(c * dx + s * dy, -s * dx + c * dy, p.z - self.origin.z)
    }

    fn to_work2(self, p: Point2) -> Point2 {
        let w = self.to_work(Point3::new(p.x, p.y, self.origin.z));
        Point2::new(w.x, w.y)
    }
}

/// A number for G-code: at most 3 decimals, trailing zeros and point dropped, no `-0`.
pub fn number(v: f64) -> String {
    format_number(v, 3, false)
}

/// A number with at most `decimals` decimals, trailing zeros dropped, no `-0`; with
/// `point` a decimal point always (`10.`).
pub fn format_number(v: f64, decimals: u32, point: bool) -> String {
    let d = decimals.min(6);
    let k = 10f64.powi(d as i32);
    let r = (v * k).round() / k;
    let s = if r == 0.0 || !r.is_finite() {
        "0".to_owned()
    } else {
        let s = format!("{r:.prec$}", prec = d as usize);
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            s
        }
    };
    if point && !s.contains('.') {
        format!("{s}.")
    } else {
        s
    }
}

/// The modal state the controller is in, so unchanged words are not repeated.
struct State {
    pos: [Option<String>; 3],
    feed: Option<String>,
    decimals: u32,
    point: bool,
    scale: f64,
}

impl State {
    fn num(&self, v: f64) -> String {
        format_number(v * self.scale, self.decimals, self.point)
    }

    /// The word for axis `i` (0 X, 1 Y, 2 Z) at `v` (mm) if it changes (or `force`),
    /// updating the state.
    fn word(&mut self, i: usize, v: f64, force: bool) -> String {
        let letter = ["X", "Y", "Z"].get(i).copied().unwrap_or("Z");
        let n = self.num(v);
        match self.pos.get_mut(i) {
            Some(slot) if force || slot.as_deref() != Some(n.as_str()) => {
                let w = format!(" {letter}{n}");
                *slot = Some(n);
                w
            }
            _ => String::new(),
        }
    }

    fn set(&mut self, i: usize, v: f64) {
        let n = self.num(v);
        if let Some(slot) = self.pos.get_mut(i) {
            *slot = Some(n);
        }
    }

    /// The X, Y and Z words that change.
    fn axes(&mut self, p: Point3) -> String {
        let mut out = self.word(0, p.x, false);
        out.push_str(&self.word(1, p.y, false));
        out.push_str(&self.word(2, p.z, false));
        out
    }

    /// The position on axis `i` as written, in the output units.
    fn get(&self, i: usize) -> Option<f64> {
        self.pos
            .get(i)
            .and_then(|v| v.as_ref())
            .and_then(|v| v.parse().ok())
    }

    fn feed(&mut self, f: f64) -> String {
        let n = self.num(f);
        if self.feed.as_deref() == Some(n.as_str()) {
            String::new()
        } else {
            self.feed = Some(n.clone());
            format!(" F{n}")
        }
    }

    /// Z as written, back in mm.
    fn z(&self) -> Option<f64> {
        self.get(2).map(|z| z / self.scale)
    }
}

/// The lines of a program, numbered as the post says.
struct Out<'a> {
    def: &'a PostDef,
    lines: Vec<String>,
    next: Option<u32>,
}

impl Out<'_> {
    /// A body line (numbered when the post numbers lines); empty lines are dropped.
    fn push(&mut self, line: String) -> Result<(), CamError> {
        let line = line.trim().to_owned();
        if line.is_empty() {
            return Ok(());
        }
        if self.lines.len() >= MAX_LINES {
            return Err(CamError::TooComplex(format!(
                "the program would be longer than {MAX_LINES} lines"
            )));
        }
        match (self.def.line_numbers, self.next) {
            (Some(n), Some(k)) => {
                self.lines.push(format!("N{k} {line}"));
                let max = n.max.unwrap_or(99_999_999);
                let next = k.saturating_add(n.step);
                self.next = Some(if next > max { n.start } else { next });
            }
            _ => self.lines.push(line),
        }
        Ok(())
    }

    /// A line never numbered (header, footer).
    fn raw(&mut self, line: String) {
        let line = line.trim().to_owned();
        if !line.is_empty() {
            self.lines.push(line);
        }
    }

    fn template(&mut self, lines: &[String], v: &Vars) -> Result<(), CamError> {
        for l in lines {
            let text = self.def.fill(l, v).map_err(bad)?;
            self.push(text)?;
        }
        Ok(())
    }
}

/// The sweep (radians, positive) of an XY arc from `from` to `to` about `c`, a full turn
/// when the ends meet.
fn sweep_of(from: Point2, to: Point2, c: Point2, clockwise: bool) -> f64 {
    let a0 = (from - c).angle();
    let a1 = (to - c).angle();
    let r = from.distance(c);
    let mut sweep = crate::path2::norm_angle(if clockwise { a0 - a1 } else { a1 - a0 });
    if sweep * r <= crate::path2::JOIN_TOLERANCE {
        sweep = TAU;
    }
    sweep
}

/// The point at angle `t` of the way along an arc (z linear).
fn arc_point(from: Point3, to: Point3, c: Point2, clockwise: bool, sweep: f64, t: f64) -> Point3 {
    let r = Point2::new(from.x, from.y).distance(c);
    let a0 = (Point2::new(from.x, from.y) - c).angle();
    let a = if clockwise {
        a0 - sweep * t
    } else {
        a0 + sweep * t
    };
    Point3::new(
        c.x + r * a.cos(),
        c.y + r * a.sin(),
        from.z + (to.z - from.z) * t,
    )
}

/// G-code for `ops` (each an operation name and its toolpath), in order, for the
/// controller `def`.
pub fn post_with(
    program: &str,
    ops: &[(String, Toolpath)],
    wcs: &Wcs,
    def: &PostDef,
    opts: &PostOptions,
) -> Result<Posted, CamError> {
    def.validate()?;
    let units = opts.units.unwrap_or(def.units);
    let decimals = match units {
        OutUnits::Mm => def.decimals,
        OutUnits::Inch => (def.decimals + 1).min(6),
    };
    let program_number = opts.program_number.unwrap_or(1000);
    let mut warnings = Vec::new();
    let mut out = Out {
        def,
        lines: Vec::new(),
        next: def.line_numbers.map(|n| n.start),
    };
    let mut vars = Vars {
        program,
        program_number,
        tool: 0,
        tool_name: "",
        rpm: 0,
        units: units.word(),
    };
    for l in &def.header {
        let text = def.fill(l, &vars).map_err(bad)?;
        out.raw(text);
    }
    out.template(&def.start, &vars)?;
    let coolant_on = match opts.coolant {
        Coolant::Off => None,
        Coolant::Flood => match &def.coolant.flood {
            Some(c) => Some(c.clone()),
            None => {
                warnings.push(format!("the {} post has no flood coolant code", def.title));
                None
            }
        },
        Coolant::Mist => match &def.coolant.mist {
            Some(c) => Some(c.clone()),
            None => {
                warnings.push(format!("the {} post has no mist coolant code", def.title));
                None
            }
        },
    };
    let mut st = State {
        pos: [None, None, None],
        feed: None,
        decimals,
        point: def.trailing_point,
        scale: units.scale(),
    };
    let mut tool: Option<u32> = None;
    let mut coolant_running = false;
    let mut at = Point3::new(0.0, 0.0, 0.0);
    for (name, tp) in ops {
        if def.operation_comments {
            let c = def.comment(name);
            out.push(c)?;
        }
        vars.tool = tp.tool_number;
        vars.tool_name = &tp.tool_name;
        vars.rpm = if tp.spindle_rpm.is_finite() && tp.spindle_rpm > 0.0 {
            tp.spindle_rpm.round().min(1e9) as u64
        } else {
            0
        };
        let mut length_offset_due = false;
        if tool != Some(tp.tool_number) {
            if coolant_running {
                if let Some(off) = &def.coolant.off {
                    out.push(off.clone())?;
                }
                coolant_running = false;
            }
            let lines = match (&tool, &def.first_tool_change) {
                (None, Some(first)) => first,
                _ => &def.tool_change,
            };
            out.template(lines, &vars)?;
            tool = Some(tp.tool_number);
            length_offset_due = def.length_offset.is_some();
        }
        let spindle = def.fill(&def.spindle_on, &vars).map_err(bad)?;
        out.push(spindle)?;
        if let Some(c) = &coolant_on
            && !coolant_running
        {
            out.push(c.clone())?;
            coolant_running = true;
        }
        let mut in_cycle = false;
        let mut started = false;
        for m in &tp.moves {
            if !started {
                at = tp.start().unwrap_or(at);
                started = true;
            }
            let mut expanded = Vec::new();
            let native = match m {
                Move::Drill { peck: Some(_), .. } => def.cycles.peck,
                Move::Drill { peck: None, .. } => def.cycles.drill,
                Move::Tap { .. } => {
                    if def.cycles.tapping.is_none() {
                        return Err(CamError::Refused(format!(
                            "the {} post has no tapping cycle: choose a post that taps, or tap \
                             by hand",
                            def.title
                        )));
                    }
                    true
                }
                _ => true,
            };
            if native {
                expanded.push(*m);
            } else {
                expand_move(m, at, &mut expanded);
            }
            for m in expanded {
                let cycle = matches!(m, Move::Drill { .. } | Move::Tap { .. });
                if in_cycle && !cycle {
                    out.push("G80".into())?;
                    in_cycle = false;
                }
                at = emit(
                    &m,
                    at,
                    wcs,
                    def,
                    &vars,
                    &mut st,
                    &mut out,
                    &mut in_cycle,
                    &mut length_offset_due,
                )?;
            }
        }
        if in_cycle {
            out.push("G80".into())?;
        }
    }
    if coolant_running && let Some(off) = &def.coolant.off {
        out.push(off.clone())?;
    }
    out.template(&def.end, &vars)?;
    for l in &def.footer {
        let text = def.fill(l, &vars).map_err(bad)?;
        out.raw(text);
    }
    let mut text = out.lines.join("\n");
    text.push('\n');
    Ok(Posted { text, warnings })
}

/// Writes one move (a drill or tapping cycle as its canned cycle); returns where the
/// tool ends, in model coordinates.
#[allow(clippy::too_many_arguments)]
fn emit(
    m: &Move,
    at: Point3,
    wcs: &Wcs,
    def: &PostDef,
    vars: &Vars,
    st: &mut State,
    out: &mut Out,
    in_cycle: &mut bool,
    length_offset_due: &mut bool,
) -> Result<Point3, CamError> {
    match *m {
        Move::Rapid { to } => {
            let w = wcs.to_work(to);
            // Lift before moving across; move across before going down. The first Z
            // move after a tool change carries the length offset.
            let z_first = st.z().is_none_or(|z| w.z > z) || *length_offset_due;
            if z_first {
                let z = st.word(2, w.z, *length_offset_due);
                if !z.is_empty() {
                    out.push(rapid_z(def, vars, &z, length_offset_due)?)?;
                }
            }
            let mut xy = st.word(0, w.x, false);
            xy.push_str(&st.word(1, w.y, false));
            if !xy.is_empty() {
                out.push(format!("G0{xy}"))?;
            }
            let z = st.word(2, w.z, false);
            if !z.is_empty() {
                out.push(rapid_z(def, vars, &z, length_offset_due)?)?;
            }
            Ok(to)
        }
        Move::Line { to, feed } | Move::Plunge { to, feed } => {
            let a = st.axes(wcs.to_work(to));
            if !a.is_empty() {
                let f = st.feed(feed);
                out.push(format!("G1{a}{f}"))?;
            }
            Ok(to)
        }
        Move::Arc {
            to,
            centre,
            clockwise,
            feed,
        } => {
            let sweep = sweep_of(
                Point2::new(at.x, at.y),
                Point2::new(to.x, to.y),
                centre,
                clockwise,
            );
            let g = if clockwise { "G2" } else { "G3" };
            match def.arcs {
                ArcStyle::Ij => {
                    let start = Point2::new(st.get(0).unwrap_or(0.0), st.get(1).unwrap_or(0.0));
                    let c = wcs.to_work2(centre);
                    let s = st.scale;
                    let w = wcs.to_work(to);
                    let mut words = st.word(0, w.x, true);
                    words.push_str(&st.word(1, w.y, true));
                    words.push_str(&st.word(2, w.z, false));
                    let f = st.feed(feed);
                    let i = format_number(c.x * s - start.x, st.decimals, st.point);
                    let j = format_number(c.y * s - start.y, st.decimals, st.point);
                    out.push(format!("{g}{words} I{i} J{j}{f}"))?;
                }
                ArcStyle::R => {
                    // R cannot say which full turn: two halves. Past a half turn R is
                    // negative.
                    let pieces = if sweep >= TAU - 1e-9 { 2 } else { 1 };
                    let r = Point2::new(at.x, at.y).distance(centre);
                    for k in 1..=pieces {
                        let t = f64::from(k) / f64::from(pieces);
                        let piece_sweep = sweep / f64::from(pieces);
                        let p = if k == pieces {
                            to
                        } else {
                            arc_point(at, to, centre, clockwise, sweep, t)
                        };
                        let w = wcs.to_work(p);
                        let mut words = st.word(0, w.x, true);
                        words.push_str(&st.word(1, w.y, true));
                        words.push_str(&st.word(2, w.z, false));
                        let f = st.feed(feed);
                        let signed = if piece_sweep > PI + 1e-9 { -r } else { r };
                        out.push(format!("{g}{words} R{}{f}", st.num(signed)))?;
                    }
                }
                ArcStyle::Linear => {
                    let r = Point2::new(at.x, at.y).distance(centre);
                    // Chords within the tolerance: sagitta r (1 - cos(θ/2)) <= tol.
                    let tol = def.arc_tolerance;
                    let step = if r > tol {
                        2.0 * (1.0 - tol / r).clamp(-1.0, 1.0).acos()
                    } else {
                        PI
                    };
                    let n = ((sweep / step.max(1e-6)).ceil() as usize).clamp(1, 100_000);
                    for k in 1..=n {
                        let t = k as f64 / n as f64;
                        let p = if k == n {
                            to
                        } else {
                            arc_point(at, to, centre, clockwise, sweep, t)
                        };
                        let a = st.axes(wcs.to_work(p));
                        if !a.is_empty() {
                            let f = st.feed(feed);
                            out.push(format!("G1{a}{f}"))?;
                        }
                    }
                }
            }
            Ok(to)
        }
        Move::Drill {
            at: xy,
            r,
            bottom,
            peck,
            feed,
        } => {
            let w = wcs.to_work(Point3::new(xy.x, xy.y, bottom));
            let rz = r - wcs.origin.z;
            let f = st.feed(feed);
            let (x, y, z, rr) = (st.num(w.x), st.num(w.y), st.num(w.z), st.num(rz));
            let ret = &def.cycles.retract;
            let line = match peck {
                Some(q) => format!("{ret} G83 X{x} Y{y} Z{z} R{rr} Q{}{f}", st.num(q)),
                None => format!("{ret} G81 X{x} Y{y} Z{z} R{rr}{f}"),
            };
            out.push(line)?;
            // After a G98 cycle the tool is back at the height it started from.
            st.set(0, w.x);
            st.set(1, w.y);
            *in_cycle = true;
            Ok(Point3::new(xy.x, xy.y, at.z))
        }
        Move::Tap {
            at: xy,
            r,
            bottom,
            pitch,
            feed,
        } => {
            let tapping = def.cycles.tapping.as_ref().ok_or_else(|| {
                CamError::Refused(format!("the {} post has no tapping cycle", def.title))
            })?;
            let w = wcs.to_work(Point3::new(xy.x, xy.y, bottom));
            let wr = wcs.to_work(Point3::new(xy.x, xy.y, r));
            match tapping.style {
                TapStyle::G84 => {
                    if !*in_cycle {
                        out.template(&tapping.before, vars)?;
                    }
                    let f = st.feed(feed);
                    let (x, y, z, rr) = (st.num(w.x), st.num(w.y), st.num(w.z), st.num(wr.z));
                    let ret = &def.cycles.retract;
                    out.push(format!("{ret} G84 X{x} Y{y} Z{z} R{rr}{f}"))?;
                    st.set(0, w.x);
                    st.set(1, w.y);
                    *in_cycle = true;
                }
                TapStyle::G33_1 => {
                    // Over the hole, down to the retract plane, tap (G33.1 comes back out
                    // to where it started), back up to the height before.
                    let start_z = at.z;
                    emit(
                        &Move::Rapid {
                            to: Point3::new(xy.x, xy.y, start_z),
                        },
                        at,
                        wcs,
                        def,
                        vars,
                        st,
                        out,
                        in_cycle,
                        &mut false,
                    )?;
                    let z = st.word(2, wr.z, false);
                    if !z.is_empty() {
                        out.push(format!("G0{z}"))?;
                    }
                    out.push(format!("G33.1 Z{} K{}", st.num(w.z), st.num(pitch)))?;
                    let z = st.word(2, wcs.to_work(Point3::new(0.0, 0.0, start_z)).z, true);
                    out.push(format!("G0{z}"))?;
                }
            }
            Ok(Point3::new(xy.x, xy.y, at.z))
        }
    }
}

/// A rapid Z line, with the length offset when one is due.
fn rapid_z(def: &PostDef, vars: &Vars, z: &str, due: &mut bool) -> Result<String, CamError> {
    if *due && let Some(t) = &def.length_offset {
        *due = false;
        let lo = def.fill(t, vars).map_err(bad)?;
        return Ok(format!("G0 {lo}{z}"));
    }
    Ok(format!("G0{z}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_drop_trailing_zeros_and_negative_zero() {
        assert_eq!(number(10.0), "10");
        assert_eq!(number(-3.5), "-3.5");
        assert_eq!(number(0.12345), "0.123");
        assert_eq!(number(-0.0001), "0");
        assert_eq!(number(f64::NAN), "0");
        assert_eq!(number(2.0005), "2.001");
        assert_eq!(format_number(10.0, 3, true), "10.");
        assert_eq!(format_number(-2.5, 3, true), "-2.5");
        assert_eq!(format_number(0.0, 4, true), "0.");
        assert_eq!(format_number(1.0 / 25.4, 4, false), "0.0394");
    }

    #[test]
    fn work_coordinates_turn_about_the_origin() {
        // Origin (10, 0, 5), axes turned 90°: the model point (10, 2, 5) lies along the
        // work X axis, 2 from the origin.
        let w = Wcs {
            origin: Point3::new(10.0, 0.0, 5.0),
            angle: std::f64::consts::FRAC_PI_2,
        };
        let p = w.to_work(Point3::new(10.0, 2.0, 5.0));
        assert!((p.x - 2.0).abs() < 1e-12 && p.y.abs() < 1e-12 && p.z.abs() < 1e-12);
    }

    #[test]
    fn comments_cannot_be_closed_early() {
        let p = find_post("linuxcnc").unwrap();
        assert_eq!(p.comment("a (b) c\nd"), "(a b cd)");
    }

    #[test]
    fn every_built_in_post_reads_and_answers_to_its_names() {
        let posts = builtin_posts();
        assert_eq!(posts.len(), BUILTIN.len());
        for (name, alias) in [
            ("linuxcnc", "rs274ngc"),
            ("grbl", "grbl"),
            ("pc-mill", "mach3"),
            ("iso-mill", "fanuc"),
            ("vmc-mill", "haas"),
        ] {
            assert_eq!(find_post(alias).unwrap().name, name);
        }
        assert!(find_post("nope").is_none());
    }

    #[test]
    fn templates_fill_nested_comments_and_refuse_unknown_placeholders() {
        let p = find_post("iso-mill").unwrap();
        let v = Vars {
            program: "plate",
            program_number: 1234,
            tool: 3,
            tool_name: "6 mm (flat)",
            rpm: 9000,
            units: "G21",
        };
        assert_eq!(
            p.fill(
                "O{program_number} {comment:{program} T{tool} {tool_name}}",
                &v
            )
            .unwrap(),
            "O1234 (PLATE T3 6 MM FLAT)"
        );
        assert!(p.fill("{nope}", &v).is_err());
        assert!(p.fill("{comment:x", &v).is_err());
    }

    #[test]
    fn hostile_post_files_are_refused() {
        let good = find_post("pc-mill").unwrap().to_json().unwrap();
        assert!(PostDef::from_json(&good).is_ok());
        let bad_files = [
            String::new(),
            "[]".into(),
            "{\"format\": \"chamfera-post\"}".into(),
            good.replace("\"pc-mill\"", "\"PC MILL\""),
            good.replace("\"version\": 1", "\"version\": 2"),
            good.replace("\"decimals\": 3", "\"decimals\": 99"),
            good.replace("\"S{rpm} M3\"", "\"S{rpm M3\""),
            good.replace("\"S{rpm} M3\"", "\"S{speed} M3\""),
            good.replace("\"arc_tolerance\": 0.01", "\"arc_tolerance\": -1.0"),
            good.replace("\"arcs\": \"ij\"", "\"arcs\": \"spline\""),
            good.replace("\"title\"", "\"unknown\": 1, \"title\""),
            "x".repeat(MAX_POST_BYTES + 1),
        ];
        for f in &bad_files {
            assert!(PostDef::from_json(f).is_err(), "accepted {f}");
        }
        let mut long = find_post("pc-mill").unwrap();
        long.start = vec!["G0".into(); MAX_TEMPLATE_LINES + 1];
        assert!(long.validate().is_err());
        long.start = vec!["G0 X1\nM30".into()];
        assert!(long.validate().is_err());
        long.start = vec![];
        long.line_numbers = Some(LineNumbers {
            start: 10,
            step: 0,
            max: None,
        });
        assert!(long.validate().is_err());
    }
}
