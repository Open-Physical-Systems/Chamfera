//! A minimal Part 21 reader: enough to parse and check the files we write.
//!
//! [`parse`] turns the text into header records and DATA instances (`#id = NAME(...)`,
//! including complex instances `#id = (A(...) B(...))`), decoding strings, and checks that
//! every `#ref` names an instance. [`summarize`] walks the advanced B-rep from each
//! `MANIFOLD_SOLID_BREP` down to the vertex coordinates and checks the closed-shell
//! conditions. This is not STEP import: it does not build kernel bodies and knows only
//! the entities we write. Full import is a later task.
//!
//! The text is hostile input: size, instance count, nesting depth and string length are
//! limited, and every lookup is checked.

use super::{MAX_STEP_BYTES, MAX_STEP_ENTITIES};
use crate::IoError;
use std::collections::{BTreeMap, HashMap};

/// Deepest nesting of lists and typed parameters accepted.
const MAX_DEPTH: usize = 64;

/// A parameter of an entity instance.
#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    /// `#n`
    Ref(u64),
    Integer(i64),
    Real(f64),
    /// A decoded string.
    String(String),
    /// `.NAME.` (`.T.` and `.F.` are enumerations too), without the dots.
    Enum(String),
    /// `"hex"`, as written.
    Binary(String),
    List(Vec<Param>),
    /// A typed parameter, `NAME(value)`.
    Typed(String, Vec<Param>),
    /// `$`
    Unset,
    /// `*`
    Derived,
}

/// One entity record: `NAME(params)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub name: String,
    pub params: Vec<Param>,
}

/// A DATA instance: one record, or several for a complex instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub records: Vec<Record>,
}

impl Instance {
    /// The record of a simple instance, or the named partial record of a complex one.
    pub fn record(&self, name: &str) -> Option<&Record> {
        self.records.iter().find(|r| r.name == name)
    }
    /// The entity name of a simple instance (`None` for a complex one).
    pub fn simple_name(&self) -> Option<&str> {
        match self.records.as_slice() {
            [r] => Some(&r.name),
            _ => None,
        }
    }
}

/// A parsed exchange structure.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Exchange {
    pub header: Vec<Record>,
    pub data: BTreeMap<u64, Instance>,
}

impl Exchange {
    pub fn header_record(&self, name: &str) -> Option<&Record> {
        self.header.iter().find(|r| r.name == name)
    }
    /// How many simple instances of each entity name (complex instances count as the
    /// names of their records joined by `+`).
    pub fn counts(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for i in self.data.values() {
            let name = i
                .records
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join("+");
            *out.entry(name).or_insert(0) += 1;
        }
        out
    }
    pub fn count(&self, name: &str) -> usize {
        self.data
            .values()
            .filter(|i| i.simple_name() == Some(name))
            .count()
    }
}

fn err(msg: impl Into<String>) -> IoError {
    IoError::Format(format!("STEP: {}", msg.into()))
}

/// Parses a Part 21 real or integer token (`1.`, `-1.5E-07`).
pub fn parse_real(s: &str) -> Result<f64, IoError> {
    s.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| err(format!("{s:?} is not a finite number")))
}

struct Parser<'a> {
    b: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn fail(&self, what: &str) -> IoError {
        err(format!("{what} at byte {}", self.at))
    }

    /// Skips whitespace and `/* comments */`.
    fn skip(&mut self) -> Result<(), IoError> {
        loop {
            while self.b.get(self.at).is_some_and(u8::is_ascii_whitespace) {
                self.at += 1;
            }
            if self.b.get(self.at..self.at + 2) == Some(b"/*") {
                let rest = self.b.get(self.at + 2..).unwrap_or_default();
                let end = rest
                    .windows(2)
                    .position(|w| w == b"*/")
                    .ok_or_else(|| self.fail("an unterminated comment"))?;
                self.at += 2 + end + 2;
            } else {
                return Ok(());
            }
        }
    }

    fn peek(&mut self) -> Result<Option<u8>, IoError> {
        self.skip()?;
        Ok(self.b.get(self.at).copied())
    }

    fn expect(&mut self, c: u8) -> Result<(), IoError> {
        if self.peek()? == Some(c) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.fail(&format!("expected '{}'", c as char)))
        }
    }

    /// A keyword: letters, digits, `_`, `-` (for `END-ISO-10303-21`), upper-cased.
    fn keyword(&mut self) -> Result<String, IoError> {
        self.skip()?;
        let start = self.at;
        while self
            .b
            .get(self.at)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
        {
            self.at += 1;
        }
        if self.at == start || self.b.get(start).is_some_and(u8::is_ascii_digit) {
            return Err(self.fail("expected a keyword"));
        }
        let k = std::str::from_utf8(self.b.get(start..self.at).unwrap_or_default())
            .map_err(|_| self.fail("a keyword must be ASCII"))?;
        Ok(k.to_ascii_uppercase())
    }

    fn statement(&mut self, word: &str) -> Result<(), IoError> {
        if self.keyword()? != word {
            return Err(self.fail(&format!("expected {word}")));
        }
        self.expect(b';')
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.b.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        self.at - start
    }

    fn instance_name(&mut self) -> Result<u64, IoError> {
        self.expect(b'#')?;
        let start = self.at;
        if self.digits() == 0 {
            return Err(self.fail("expected an instance number after '#'"));
        }
        let s = std::str::from_utf8(self.b.get(start..self.at).unwrap_or_default())
            .map_err(|_| self.fail("bad instance number"))?;
        s.parse::<u64>()
            .map_err(|_| self.fail("an instance number is too large"))
    }

    fn number(&mut self) -> Result<Param, IoError> {
        let start = self.at;
        if matches!(self.b.get(self.at), Some(b'+' | b'-')) {
            self.at += 1;
        }
        if self.digits() == 0 {
            return Err(self.fail("expected a number"));
        }
        let mut is_real = false;
        if self.b.get(self.at) == Some(&b'.') {
            is_real = true;
            self.at += 1;
            self.digits();
            if self.b.get(self.at) == Some(&b'E') {
                self.at += 1;
                if matches!(self.b.get(self.at), Some(b'+' | b'-')) {
                    self.at += 1;
                }
                if self.digits() == 0 {
                    return Err(self.fail("expected exponent digits"));
                }
            }
        }
        let s = std::str::from_utf8(self.b.get(start..self.at).unwrap_or_default())
            .map_err(|_| self.fail("bad number"))?;
        if is_real {
            Ok(Param::Real(parse_real(s)?))
        } else {
            s.parse::<i64>()
                .map(Param::Integer)
                .map_err(|_| self.fail("an integer is too large"))
        }
    }

    fn hex(&mut self, n: usize) -> Result<u32, IoError> {
        let s = self
            .b
            .get(self.at..self.at + n)
            .and_then(|h| std::str::from_utf8(h).ok())
            .ok_or_else(|| self.fail("a string escape is cut short"))?;
        let v = u32::from_str_radix(s, 16).map_err(|_| self.fail("bad hex in a string"))?;
        self.at += n;
        Ok(v)
    }

    /// A string literal (opening apostrophe already current), decoded.
    fn string(&mut self) -> Result<String, IoError> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let c = *self
                .b
                .get(self.at)
                .ok_or_else(|| self.fail("an unterminated string"))?;
            self.at += 1;
            match c {
                b'\'' => {
                    if self.b.get(self.at) == Some(&b'\'') {
                        self.at += 1;
                        out.push('\'');
                    } else {
                        return Ok(out);
                    }
                }
                b'\\' => self.escape(&mut out)?,
                b' '..=b'~' => out.push(char::from(c)),
                _ => return Err(self.fail("a string holds a character outside printable ASCII")),
            }
            if out.len() > MAX_STEP_BYTES {
                return Err(self.fail("a string is too long"));
            }
        }
    }

    /// After a backslash in a string: `\\`, `\X\hh`, `\X2\…\X0\`, `\X4\…\X0\`, `\S\c`.
    fn escape(&mut self, out: &mut String) -> Result<(), IoError> {
        let rest = self.b.get(self.at..).unwrap_or_default();
        let bad = |p: &Self| p.fail("a code point in a string escape is not a character");
        if rest.first() == Some(&b'\\') {
            self.at += 1;
            out.push('\\');
        } else if rest.starts_with(b"X\\") {
            self.at += 2;
            let v = self.hex(2)?;
            out.push(char::from_u32(v).ok_or_else(|| bad(self))?);
        } else if rest.starts_with(b"S\\") {
            self.at += 2;
            let c = *self
                .b
                .get(self.at)
                .ok_or_else(|| self.fail("a string escape is cut short"))?;
            self.at += 1;
            out.push(char::from_u32(u32::from(c) + 128).ok_or_else(|| bad(self))?);
        } else if rest.starts_with(b"X2\\") || rest.starts_with(b"X4\\") {
            let wide = rest.get(1) == Some(&b'4');
            self.at += 3;
            let mut units = Vec::new();
            while self.b.get(self.at) != Some(&b'\\') {
                if wide {
                    let v = self.hex(8)?;
                    out.push(char::from_u32(v).ok_or_else(|| bad(self))?);
                } else {
                    units.push(u16::try_from(self.hex(4)?).map_err(|_| bad(self))?);
                }
            }
            if self.b.get(self.at..self.at + 4) != Some(b"\\X0\\") {
                return Err(self.fail("an \\X2\\ or \\X4\\ group must end with \\X0\\"));
            }
            self.at += 4;
            out.push_str(&String::from_utf16(&units).map_err(|_| bad(self))?);
        } else {
            return Err(self.fail("an unknown string escape"));
        }
        Ok(())
    }

    fn param(&mut self, depth: usize) -> Result<Param, IoError> {
        if depth > MAX_DEPTH {
            return Err(self.fail("parameters are nested too deeply"));
        }
        match self.peek()? {
            Some(b'#') => Ok(Param::Ref(self.instance_name()?)),
            Some(b'\'') => Ok(Param::String(self.string()?)),
            Some(b'$') => {
                self.at += 1;
                Ok(Param::Unset)
            }
            Some(b'*') => {
                self.at += 1;
                Ok(Param::Derived)
            }
            Some(b'.') => {
                self.at += 1;
                let start = self.at;
                while self
                    .b
                    .get(self.at)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    self.at += 1;
                }
                let s = std::str::from_utf8(self.b.get(start..self.at).unwrap_or_default())
                    .map_err(|_| self.fail("bad enumeration"))?
                    .to_owned();
                if s.is_empty() || self.b.get(self.at) != Some(&b'.') {
                    return Err(self.fail("an enumeration must look like .NAME."));
                }
                self.at += 1;
                Ok(Param::Enum(s))
            }
            Some(b'"') => {
                self.at += 1;
                let start = self.at;
                while self.b.get(self.at).is_some_and(u8::is_ascii_hexdigit) {
                    self.at += 1;
                }
                let s = std::str::from_utf8(self.b.get(start..self.at).unwrap_or_default())
                    .map_err(|_| self.fail("bad binary"))?
                    .to_owned();
                self.expect(b'"')?;
                Ok(Param::Binary(s))
            }
            Some(b'(') => Ok(Param::List(self.list(depth + 1)?)),
            Some(c) if c == b'+' || c == b'-' || c.is_ascii_digit() => self.number(),
            Some(c) if c.is_ascii_alphabetic() => {
                let name = self.keyword()?;
                Ok(Param::Typed(name, self.list(depth + 1)?))
            }
            _ => Err(self.fail("expected a parameter")),
        }
    }

    /// `( p, p, ... )`, possibly empty.
    fn list(&mut self, depth: usize) -> Result<Vec<Param>, IoError> {
        self.expect(b'(')?;
        let mut out = Vec::new();
        if self.peek()? == Some(b')') {
            self.at += 1;
            return Ok(out);
        }
        loop {
            out.push(self.param(depth)?);
            match self.peek()? {
                Some(b',') => self.at += 1,
                Some(b')') => {
                    self.at += 1;
                    return Ok(out);
                }
                _ => return Err(self.fail("expected ',' or ')'")),
            }
        }
    }

    fn record(&mut self) -> Result<Record, IoError> {
        let name = self.keyword()?;
        let params = self.list(0)?;
        Ok(Record { name, params })
    }
}

/// Parses a Part 21 exchange structure (one DATA section) and checks every reference.
pub fn parse(text: &str) -> Result<Exchange, IoError> {
    if text.len() > MAX_STEP_BYTES {
        return Err(err(format!(
            "the file is larger than {MAX_STEP_BYTES} bytes"
        )));
    }
    let mut p = Parser {
        b: text.as_bytes(),
        at: 0,
    };
    p.statement("ISO-10303-21")?;
    p.statement("HEADER")?;
    let mut ex = Exchange::default();
    loop {
        let r = p.record_or_endsec()?;
        match r {
            Some(r) => ex.header.push(r),
            None => break,
        }
    }
    if p.keyword()? != "DATA" {
        return Err(p.fail("expected DATA"));
    }
    if p.peek()? == Some(b'(') {
        p.list(0)?; // Part 21 edition 3: DATA(name, schemas)
    }
    p.expect(b';')?;
    loop {
        if p.peek()? != Some(b'#') {
            p.statement("ENDSEC")?;
            break;
        }
        let id = p.instance_name()?;
        p.expect(b'=')?;
        let records = if p.peek()? == Some(b'(') {
            p.at += 1;
            let mut rs = Vec::new();
            while p.peek()? != Some(b')') {
                rs.push(p.record()?);
            }
            p.at += 1;
            if rs.is_empty() {
                return Err(p.fail("a complex instance needs records"));
            }
            rs
        } else {
            vec![p.record()?]
        };
        p.expect(b';')?;
        if ex.data.len() >= MAX_STEP_ENTITIES {
            return Err(err(format!(
                "the file has more than {MAX_STEP_ENTITIES} instances"
            )));
        }
        if ex.data.insert(id, Instance { records }).is_some() {
            return Err(err(format!("instance #{id} is defined twice")));
        }
    }
    p.statement("END-ISO-10303-21")?;
    if p.peek()?.is_some() {
        return Err(p.fail("text after END-ISO-10303-21"));
    }
    check_references(&ex)?;
    Ok(ex)
}

impl Parser<'_> {
    /// A header record, or `None` after `ENDSEC;`.
    fn record_or_endsec(&mut self) -> Result<Option<Record>, IoError> {
        let name = self.keyword()?;
        if name == "ENDSEC" {
            self.expect(b';')?;
            return Ok(None);
        }
        let params = self.list(0)?;
        self.expect(b';')?;
        Ok(Some(Record { name, params }))
    }
}

fn check_references(ex: &Exchange) -> Result<(), IoError> {
    fn walk(ex: &Exchange, from: u64, p: &Param) -> Result<(), IoError> {
        match p {
            Param::Ref(r) if !ex.data.contains_key(r) => Err(err(format!(
                "#{from} refers to #{r}, which is not in the file"
            ))),
            Param::List(l) | Param::Typed(_, l) => l.iter().try_for_each(|q| walk(ex, from, q)),
            _ => Ok(()),
        }
    }
    for (id, inst) in &ex.data {
        for r in &inst.records {
            for p in &r.params {
                walk(ex, *id, p)?;
            }
        }
    }
    Ok(())
}

/// One `MANIFOLD_SOLID_BREP` as rebuilt from the file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SolidSummary {
    pub name: String,
    pub faces: usize,
    /// `FACE_OUTER_BOUND`s.
    pub outer_bounds: usize,
    /// `FACE_BOUND`s (inner loops).
    pub inner_bounds: usize,
    /// Distinct `EDGE_CURVE`s.
    pub edges: usize,
    /// `ORIENTED_EDGE` uses.
    pub oriented_edges: usize,
    /// Distinct `VERTEX_POINT`s.
    pub vertices: usize,
    /// Coordinates of the distinct vertices, in instance-number order.
    pub vertex_points: Vec<[f64; 3]>,
    pub face_names: Vec<String>,
    /// Each face's surface: its entity name (`PLANE`, `CYLINDRICAL_SURFACE`, ...) and
    /// numeric parameters (radii, angles) after the placement, in face order.
    pub face_surfaces: Vec<SurfaceSummary>,
}

/// A face's surface as written.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SurfaceSummary {
    pub entity: String,
    /// The placement: origin, axis (z) and ref_direction (x).
    pub origin: [f64; 3],
    pub axis: [f64; 3],
    pub ref_direction: [f64; 3],
    /// The numbers after the placement (radius; radius and semi-angle; major and minor
    /// radius), in order.
    pub values: Vec<f64>,
    /// `select_outer` of a `DEGENERATE_TOROIDAL_SURFACE`.
    pub select_outer: Option<bool>,
    /// The face's `same_sense`.
    pub same_sense: bool,
}

/// Entities a face may lie on, with how many numbers follow the placement.
const SURFACES: [(&str, usize); 6] = [
    ("PLANE", 0),
    ("CYLINDRICAL_SURFACE", 1),
    ("CONICAL_SURFACE", 2),
    ("SPHERICAL_SURFACE", 1),
    ("TOROIDAL_SURFACE", 2),
    ("DEGENERATE_TOROIDAL_SURFACE", 2),
];

/// The B-rep content of a file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrepSummary {
    pub solids: Vec<SolidSummary>,
}

struct View<'a> {
    ex: &'a Exchange,
}

impl<'a> View<'a> {
    /// The parameters of `#id`, which must be a simple instance of entity `name`.
    fn get(&self, id: u64, name: &str) -> Result<&'a [Param], IoError> {
        match self.get_any(id) {
            Ok((n, ps)) if n == name => Ok(ps),
            _ => Err(err(format!("#{id} should be a {name}"))),
        }
    }
    fn get_any(&self, id: u64) -> Result<(&'a str, &'a [Param]), IoError> {
        let inst = self
            .ex
            .data
            .get(&id)
            .ok_or_else(|| err(format!("#{id} is missing")))?;
        match inst.records.as_slice() {
            [r] => Ok((&r.name, &r.params)),
            _ => Err(err(format!("#{id} should be a simple instance"))),
        }
    }
}

fn param(ps: &[Param], i: usize, of: u64) -> Result<&Param, IoError> {
    ps.get(i)
        .ok_or_else(|| err(format!("#{of} has too few parameters")))
}
fn as_ref(p: &Param, of: u64) -> Result<u64, IoError> {
    match p {
        Param::Ref(r) => Ok(*r),
        _ => Err(err(format!("#{of}: expected a reference"))),
    }
}
fn as_list(p: &Param, of: u64) -> Result<&[Param], IoError> {
    match p {
        Param::List(l) => Ok(l),
        _ => Err(err(format!("#{of}: expected a list"))),
    }
}
fn as_bool(p: &Param, of: u64) -> Result<bool, IoError> {
    match p {
        Param::Enum(e) if e == "T" => Ok(true),
        Param::Enum(e) if e == "F" => Ok(false),
        _ => Err(err(format!("#{of}: expected .T. or .F."))),
    }
}
fn as_string(p: &Param, of: u64) -> Result<String, IoError> {
    match p {
        Param::String(s) => Ok(s.clone()),
        _ => Err(err(format!("#{of}: expected a string"))),
    }
}
fn as_real(p: &Param, of: u64) -> Result<f64, IoError> {
    match p {
        Param::Real(v) => Ok(*v),
        Param::Integer(i) => Ok(*i as f64),
        _ => Err(err(format!("#{of}: expected a number"))),
    }
}

/// Rebuilds every `MANIFOLD_SOLID_BREP` of the file and checks it: faces, bounds, loops,
/// oriented edges, edges and vertices have the expected entity types; each loop closes
/// (every oriented edge ends where the next starts); and in each closed shell every edge
/// is used by exactly two oriented edges that run in opposite directions.
pub fn summarize(ex: &Exchange) -> Result<BrepSummary, IoError> {
    let v = View { ex };
    let mut out = BrepSummary::default();
    for (id, inst) in &ex.data {
        if inst.simple_name() != Some("MANIFOLD_SOLID_BREP") {
            continue;
        }
        out.solids.push(solid(&v, *id)?);
    }
    Ok(out)
}

fn solid(v: &View<'_>, id: u64) -> Result<SolidSummary, IoError> {
    let ps = v.get(id, "MANIFOLD_SOLID_BREP")?;
    let mut s = SolidSummary {
        name: as_string(param(ps, 0, id)?, id)?,
        ..SolidSummary::default()
    };
    let shell = as_ref(param(ps, 1, id)?, id)?;
    let sp = v.get(shell, "CLOSED_SHELL")?;
    // Per edge: uses as (runs from start to end?) .
    let mut uses: BTreeMap<u64, Vec<bool>> = BTreeMap::new();
    let mut vertices: BTreeMap<u64, [f64; 3]> = BTreeMap::new();
    let mut edge_ends: HashMap<u64, (u64, u64)> = HashMap::new();
    for f in as_list(param(sp, 1, shell)?, shell)? {
        let f = as_ref(f, shell)?;
        let fp = v.get(f, "ADVANCED_FACE")?;
        s.faces += 1;
        s.face_names.push(as_string(param(fp, 0, f)?, f)?);
        let same_sense = as_bool(param(fp, 3, f)?, f)?;
        let surface = as_ref(param(fp, 2, f)?, f)?;
        s.face_surfaces
            .push(surface_summary(v, surface, same_sense)?);
        for b in as_list(param(fp, 1, f)?, f)? {
            let b = as_ref(b, f)?;
            let (kind, bp) = v.get_any(b)?;
            match kind {
                "FACE_OUTER_BOUND" => s.outer_bounds += 1,
                "FACE_BOUND" => s.inner_bounds += 1,
                _ => return Err(err(format!("#{b} should be a face bound"))),
            }
            let bound_sense = as_bool(param(bp, 2, b)?, b)?;
            let lp = as_ref(param(bp, 1, b)?, b)?;
            let lpp = v.get(lp, "EDGE_LOOP")?;
            let mut chain: Vec<(u64, u64)> = Vec::new();
            for oe in as_list(param(lpp, 1, lp)?, lp)? {
                let oe = as_ref(oe, lp)?;
                let op = v.get(oe, "ORIENTED_EDGE")?;
                let e = as_ref(param(op, 3, oe)?, oe)?;
                let along = as_bool(param(op, 4, oe)?, oe)?;
                let (start, end) = match edge_ends.get(&e) {
                    Some(se) => *se,
                    None => {
                        let ep = v.get(e, "EDGE_CURVE")?;
                        let a = as_ref(param(ep, 1, e)?, e)?;
                        let z = as_ref(param(ep, 2, e)?, e)?;
                        as_bool(param(ep, 4, e)?, e)?;
                        v.get_any(as_ref(param(ep, 3, e)?, e)?)?;
                        for vx in [a, z] {
                            if let std::collections::btree_map::Entry::Vacant(slot) =
                                vertices.entry(vx)
                            {
                                slot.insert(vertex_point(v, vx)?);
                            }
                        }
                        edge_ends.insert(e, (a, z));
                        (a, z)
                    }
                };
                s.oriented_edges += 1;
                // Direction of use along the face boundary: the oriented edge's sense,
                // reversed when the bound itself is reversed.
                let forward = along == bound_sense;
                uses.entry(e).or_default().push(forward);
                chain.push(if along { (start, end) } else { (end, start) });
            }
            if !bound_sense {
                chain.reverse();
                for c in &mut chain {
                    *c = (c.1, c.0);
                }
            }
            for (i, (_, end)) in chain.iter().enumerate() {
                let next = chain
                    .get((i + 1) % chain.len())
                    .map(|c| c.0)
                    .unwrap_or(*end);
                if next != *end {
                    return Err(err(format!("the loop #{lp} does not close at #{end}")));
                }
            }
        }
    }
    for (e, u) in &uses {
        if !matches!(u.as_slice(), [a, b] if a != b) {
            return Err(err(format!(
                "edge #{e} must be used twice in opposite directions in a closed shell, it is used {} time(s)",
                u.len()
            )));
        }
    }
    s.edges = uses.len();
    s.vertices = vertices.len();
    s.vertex_points = vertices.into_values().collect();
    Ok(s)
}

/// A face's surface: one of [`SURFACES`] on an `AXIS2_PLACEMENT_3D`, with positive
/// finite radii (and, for a cone, `0 <= radius`, `0 < semi_angle < π/2`; for a
/// degenerate torus `major < minor`).
fn surface_summary(v: &View<'_>, id: u64, same_sense: bool) -> Result<SurfaceSummary, IoError> {
    let (name, ps) = v.get_any(id)?;
    let Some(&(_, count)) = SURFACES.iter().find(|(n, _)| *n == name) else {
        return Err(err(format!("#{id} is a {name}, not a surface we know")));
    };
    let place = as_ref(param(ps, 1, id)?, id)?;
    let pp = v.get(place, "AXIS2_PLACEMENT_3D")?;
    let point = |i: usize| -> Result<[f64; 3], IoError> {
        let r = as_ref(param(pp, i, place)?, place)?;
        let (kind, cp) = v.get_any(r)?;
        if (i == 1) != (kind == "CARTESIAN_POINT") || (i > 1 && kind != "DIRECTION") {
            return Err(err(format!("#{r} has the wrong type in a placement")));
        }
        match as_list(param(cp, 1, r)?, r)? {
            [x, y, z] => Ok([as_real(x, r)?, as_real(y, r)?, as_real(z, r)?]),
            _ => Err(err(format!("#{r} should have three coordinates"))),
        }
    };
    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        let x = as_real(param(ps, 2 + i, id)?, id)?;
        if !x.is_finite() || x < 0.0 {
            return Err(err(format!(
                "#{id}: its numbers must be finite and not negative"
            )));
        }
        values.push(x);
    }
    let select_outer = if name == "DEGENERATE_TOROIDAL_SURFACE" {
        Some(as_bool(param(ps, 4, id)?, id)?)
    } else {
        None
    };
    let ok = match (name, values.as_slice()) {
        ("CONICAL_SURFACE", [_, a]) => *a > 0.0 && *a < std::f64::consts::FRAC_PI_2,
        ("DEGENERATE_TOROIDAL_SURFACE", [big, small]) => *big > 0.0 && big < small,
        (_, vs) => vs.iter().all(|x| *x > 0.0),
    };
    if !ok {
        return Err(err(format!("#{id}: its radii or angle are out of range")));
    }
    Ok(SurfaceSummary {
        entity: name.to_owned(),
        origin: point(1)?,
        axis: point(2)?,
        ref_direction: point(3)?,
        values,
        select_outer,
        same_sense,
    })
}

fn vertex_point(v: &View<'_>, id: u64) -> Result<[f64; 3], IoError> {
    let vp = v.get(id, "VERTEX_POINT")?;
    let p = as_ref(param(vp, 1, id)?, id)?;
    let pp = v.get(p, "CARTESIAN_POINT")?;
    let c = as_list(param(pp, 1, p)?, p)?;
    match c {
        [x, y, z] => Ok([as_real(x, p)?, as_real(y, p)?, as_real(z, p)?]),
        _ => Err(err(format!("#{p} should have three coordinates"))),
    }
}
