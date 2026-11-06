//! Expressions with units: `80 mm`, `width / 2 + 3`, `2 * 15 deg`, `sqrt(a^2 + b^2)`.
//!
//! - Numbers may carry a unit: mm, cm, m, in, ft (lengths) or deg, rad (angles). `°` and
//!   `"` (inch) are accepted too.
//! - Names are parameters; `pi` is π.
//! - Functions: sqrt, abs, min, max, sin, cos, tan, asin, acos, atan, atan2, round, floor,
//!   ceil. The trigonometric ones take angles and return plain numbers (asin… return angles).
//! - Operators: `+ - * / ^`, parentheses, unary minus. `^` takes a plain-number exponent.
//!
//! Every value has a dimension (powers of length and angle), checked as it is computed:
//! `10 mm + 5 deg` is an error. Internally lengths are mm and angles radians.
//!
//! A result with no unit takes the expected unit's default: a plain `50` for a length is
//! 50 mm, and for an angle is 50 degrees (the convention CAD users expect).

use std::collections::BTreeMap;

/// Longest expression accepted (characters).
pub const MAX_EXPRESSION_LEN: usize = 1000;
/// Deepest nesting of parentheses and operators.
pub const MAX_DEPTH: usize = 64;

/// Powers of length and angle: (1, 0) is a length, (0, 1) an angle, (0, 0) a plain number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dim {
    pub length: i8,
    pub angle: i8,
}

impl Dim {
    pub const NONE: Dim = Dim {
        length: 0,
        angle: 0,
    };
    pub const LENGTH: Dim = Dim {
        length: 1,
        angle: 0,
    };
    pub const ANGLE: Dim = Dim {
        length: 0,
        angle: 1,
    };

    fn mul(self, o: Dim) -> Option<Dim> {
        Some(Dim {
            length: self.length.checked_add(o.length)?,
            angle: self.angle.checked_add(o.angle)?,
        })
    }

    fn div(self, o: Dim) -> Option<Dim> {
        Some(Dim {
            length: self.length.checked_sub(o.length)?,
            angle: self.angle.checked_sub(o.angle)?,
        })
    }

    /// How a user reads the dimension: "mm", "mm²", "deg", "a plain number".
    pub fn describe(self) -> String {
        match (self.length, self.angle) {
            (0, 0) => "a plain number".into(),
            (1, 0) => "a length (mm)".into(),
            (0, 1) => "an angle".into(),
            (l, 0) => format!("mm^{l}"),
            (0, a) => format!("angle^{a}"),
            (l, a) => format!("mm^{l}·angle^{a}"),
        }
    }
}

/// A value with its dimension, in internal units (mm, radians).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub dim: Dim,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExprError {
    #[error("the expression is empty")]
    Empty,
    #[error("the expression is longer than {MAX_EXPRESSION_LEN} characters")]
    TooLong,
    #[error("the expression nests deeper than {MAX_DEPTH} levels")]
    TooDeep,
    #[error("unexpected {found:?} at position {at}")]
    Unexpected { found: String, at: usize },
    #[error("unknown unit {0:?}: use mm, cm, m, in, ft, deg or rad")]
    UnknownUnit(String),
    #[error("unknown name {0:?}")]
    UnknownName(String),
    #[error("unknown function {0:?}")]
    UnknownFunction(String),
    #[error("{function} takes {expected} argument(s), got {got}")]
    Arity {
        function: String,
        expected: usize,
        got: usize,
    },
    #[error("cannot {op} {left} and {right}")]
    Mismatch {
        op: &'static str,
        left: String,
        right: String,
    },
    #[error("{0} needs {1}")]
    BadArgument(String, String),
    #[error("the result is {got}, but {expected} is needed")]
    WrongUnit { expected: String, got: String },
    #[error("the result is not a finite number")]
    NotFinite,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
    LParen,
    RParen,
    Comma,
}

fn tokenize(src: &str) -> Result<Vec<(Tok, usize)>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            while chars
                .get(i)
                .is_some_and(|c| c.is_ascii_digit() || *c == '.')
            {
                i += 1;
            }
            // Exponent: 1e3, 2.5E-2 (only when digits follow).
            if chars.get(i).is_some_and(|c| *c == 'e' || *c == 'E') {
                let mut j = i + 1;
                if chars.get(j).is_some_and(|c| *c == '+' || *c == '-') {
                    j += 1;
                }
                if chars.get(j).is_some_and(char::is_ascii_digit) {
                    i = j;
                    while chars.get(i).is_some_and(char::is_ascii_digit) {
                        i += 1;
                    }
                }
            }
            let text: String = chars.get(start..i).unwrap_or_default().iter().collect();
            let v: f64 = text.parse().map_err(|_| ExprError::Unexpected {
                found: text.clone(),
                at: start,
            })?;
            out.push((Tok::Num(v), start));
        } else if c.is_alphabetic() || c == '_' || c == '°' || c == '"' {
            let start = i;
            if c == '°' || c == '"' {
                i += 1;
            } else {
                while chars
                    .get(i)
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_')
                {
                    i += 1;
                }
            }
            let text: String = chars.get(start..i).unwrap_or_default().iter().collect();
            out.push((Tok::Ident(text), start));
        } else {
            let t = match c {
                '+' | '-' | '*' | '/' | '^' => Tok::Op(c),
                '×' => Tok::Op('*'),
                '(' => Tok::LParen,
                ')' => Tok::RParen,
                ',' => Tok::Comma,
                _ => {
                    return Err(ExprError::Unexpected {
                        found: c.to_string(),
                        at: i,
                    });
                }
            };
            out.push((t, i));
            i += 1;
        }
    }
    Ok(out)
}

/// A length or angle unit: its size in internal units and its dimension.
fn unit(name: &str) -> Option<(f64, Dim)> {
    Some(match name {
        "mm" => (1.0, Dim::LENGTH),
        "cm" => (10.0, Dim::LENGTH),
        "m" => (1000.0, Dim::LENGTH),
        "in" | "\"" => (25.4, Dim::LENGTH),
        "ft" => (304.8, Dim::LENGTH),
        "deg" | "°" => (std::f64::consts::PI / 180.0, Dim::ANGLE),
        "rad" => (1.0, Dim::ANGLE),
        _ => return None,
    })
}

/// Looks up parameter values by name.
pub trait Names {
    fn get(&self, name: &str) -> Option<Quantity>;
}

impl Names for BTreeMap<String, Quantity> {
    fn get(&self, name: &str) -> Option<Quantity> {
        BTreeMap::get(self, name).copied()
    }
}

/// No parameters (for constants).
pub struct NoNames;

impl Names for NoNames {
    fn get(&self, _: &str) -> Option<Quantity> {
        None
    }
}

struct Parser<'a, N: Names> {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    depth: usize,
    names: &'a N,
}

impl<N: Names> Parser<'_, N> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }

    fn unexpected(&self) -> ExprError {
        match self.toks.get(self.pos) {
            Some((t, at)) => ExprError::Unexpected {
                found: match t {
                    Tok::Num(v) => v.to_string(),
                    Tok::Ident(s) => s.clone(),
                    Tok::Op(c) => c.to_string(),
                    Tok::LParen => "(".into(),
                    Tok::RParen => ")".into(),
                    Tok::Comma => ",".into(),
                },
                at: *at,
            },
            None => ExprError::Unexpected {
                found: "the end".into(),
                at: self.toks.last().map_or(0, |t| t.1 + 1),
            },
        }
    }

    fn enter(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ExprError::TooDeep);
        }
        Ok(())
    }

    fn expr(&mut self) -> Result<Quantity, ExprError> {
        self.enter()?;
        let mut left = self.term()?;
        while let Some(Tok::Op(c @ ('+' | '-'))) = self.peek().cloned() {
            self.pos += 1;
            let right = self.term()?;
            if left.dim != right.dim {
                return Err(ExprError::Mismatch {
                    op: if c == '+' { "add" } else { "subtract" },
                    left: left.dim.describe(),
                    right: right.dim.describe(),
                });
            }
            left.value = if c == '+' {
                left.value + right.value
            } else {
                left.value - right.value
            };
        }
        self.depth -= 1;
        Ok(left)
    }

    fn term(&mut self) -> Result<Quantity, ExprError> {
        let mut left = self.power()?;
        while let Some(Tok::Op(c @ ('*' | '/'))) = self.peek().cloned() {
            self.pos += 1;
            let right = self.power()?;
            let overflow = || ExprError::Mismatch {
                op: "combine",
                left: "units".into(),
                right: "units this large".into(),
            };
            left = if c == '*' {
                Quantity {
                    value: left.value * right.value,
                    dim: left.dim.mul(right.dim).ok_or_else(overflow)?,
                }
            } else {
                Quantity {
                    value: left.value / right.value,
                    dim: left.dim.div(right.dim).ok_or_else(overflow)?,
                }
            };
        }
        Ok(left)
    }

    fn power(&mut self) -> Result<Quantity, ExprError> {
        let base = self.unary()?;
        if self.peek() == Some(&Tok::Op('^')) {
            self.pos += 1;
            self.enter()?;
            let exp = self.power()?;
            self.depth -= 1;
            if exp.dim != Dim::NONE {
                return Err(ExprError::BadArgument(
                    "^".into(),
                    "a plain-number exponent".into(),
                ));
            }
            // Units follow whole powers only: (2 mm)^2 is mm².
            let dim = if base.dim == Dim::NONE {
                Dim::NONE
            } else {
                let n = exp.value;
                if n.fract() != 0.0 || n.abs() > 8.0 {
                    return Err(ExprError::BadArgument(
                        "^".into(),
                        "a whole exponent from -8 to 8 for a value with a unit".into(),
                    ));
                }
                let n = n as i8;
                Dim {
                    length: base.dim.length.checked_mul(n).ok_or(ExprError::TooDeep)?,
                    angle: base.dim.angle.checked_mul(n).ok_or(ExprError::TooDeep)?,
                }
            };
            return Ok(Quantity {
                value: base.value.powf(exp.value),
                dim,
            });
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<Quantity, ExprError> {
        match self.peek() {
            Some(Tok::Op('-')) => {
                self.pos += 1;
                self.enter()?;
                let mut q = self.unary()?;
                self.depth -= 1;
                q.value = -q.value;
                Ok(q)
            }
            Some(Tok::Op('+')) => {
                self.pos += 1;
                self.enter()?;
                let q = self.unary();
                self.depth -= 1;
                q
            }
            _ => self.primary(),
        }
    }

    /// A unit name directly after a number or a closing parenthesis.
    fn apply_unit(&mut self, q: Quantity) -> Result<Quantity, ExprError> {
        if let Some(Tok::Ident(name)) = self.peek().cloned()
            && let Some((scale, dim)) = unit(&name)
        {
            if q.dim != Dim::NONE {
                return Err(ExprError::BadArgument(
                    name,
                    "a plain number before it".into(),
                ));
            }
            self.pos += 1;
            return Ok(Quantity {
                value: q.value * scale,
                dim,
            });
        }
        Ok(q)
    }

    fn primary(&mut self) -> Result<Quantity, ExprError> {
        match self.peek().cloned() {
            Some(Tok::Num(v)) => {
                self.pos += 1;
                self.apply_unit(Quantity {
                    value: v,
                    dim: Dim::NONE,
                })
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let q = self.expr()?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err(self.unexpected());
                }
                self.pos += 1;
                self.apply_unit(q)
            }
            Some(Tok::Ident(name)) => {
                self.pos += 1;
                if self.peek() == Some(&Tok::LParen) {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.expr()?);
                            match self.peek() {
                                Some(Tok::Comma) => self.pos += 1,
                                Some(Tok::RParen) => break,
                                _ => return Err(self.unexpected()),
                            }
                        }
                    }
                    self.pos += 1;
                    return call(&name, &args);
                }
                if name == "pi" {
                    return Ok(Quantity {
                        value: std::f64::consts::PI,
                        dim: Dim::NONE,
                    });
                }
                if unit(&name).is_some() {
                    return Err(ExprError::BadArgument(name, "a number before it".into()));
                }
                self.names.get(&name).ok_or(ExprError::UnknownName(name))
            }
            _ => Err(self.unexpected()),
        }
    }
}

fn call(name: &str, args: &[Quantity]) -> Result<Quantity, ExprError> {
    let arity = |n: usize| {
        if args.len() == n {
            Ok(())
        } else {
            Err(ExprError::Arity {
                function: name.into(),
                expected: n,
                got: args.len(),
            })
        }
    };
    let plain = |v: f64| Quantity {
        value: v,
        dim: Dim::NONE,
    };
    let need = |q: Quantity, dim: Dim, what: &str| {
        if q.dim == dim {
            Ok(q.value)
        } else {
            Err(ExprError::BadArgument(name.into(), what.into()))
        }
    };
    // The argument of a trigonometric function is an angle; a plain number means degrees.
    let angle = |q: Quantity| -> Result<f64, ExprError> {
        match q.dim {
            d if d == Dim::ANGLE => Ok(q.value),
            d if d == Dim::NONE => Ok(q.value.to_radians()),
            _ => Err(ExprError::BadArgument(name.into(), "an angle".into())),
        }
    };
    let first = || {
        args.first().copied().ok_or(ExprError::Arity {
            function: name.into(),
            expected: 1,
            got: 0,
        })
    };
    match name {
        "sqrt" => {
            arity(1)?;
            let q = first()?;
            if q.dim.length % 2 != 0 || q.dim.angle % 2 != 0 {
                return Err(ExprError::BadArgument(
                    name.into(),
                    "an even power of its unit".into(),
                ));
            }
            Ok(Quantity {
                value: q.value.sqrt(),
                dim: Dim {
                    length: q.dim.length / 2,
                    angle: q.dim.angle / 2,
                },
            })
        }
        "abs" | "round" | "floor" | "ceil" => {
            arity(1)?;
            let q = first()?;
            let v = match name {
                "abs" => q.value.abs(),
                "round" => q.value.round(),
                "floor" => q.value.floor(),
                _ => q.value.ceil(),
            };
            Ok(Quantity {
                value: v,
                dim: q.dim,
            })
        }
        "min" | "max" => {
            if args.is_empty() {
                return Err(ExprError::Arity {
                    function: name.into(),
                    expected: 1,
                    got: 0,
                });
            }
            let d = first()?.dim;
            if args.iter().any(|a| a.dim != d) {
                return Err(ExprError::BadArgument(
                    name.into(),
                    "arguments of one unit".into(),
                ));
            }
            let it = args.iter().map(|a| a.value);
            let v = if name == "min" {
                it.fold(f64::INFINITY, f64::min)
            } else {
                it.fold(f64::NEG_INFINITY, f64::max)
            };
            Ok(Quantity { value: v, dim: d })
        }
        "sin" | "cos" | "tan" => {
            arity(1)?;
            let a = angle(first()?)?;
            Ok(plain(match name {
                "sin" => a.sin(),
                "cos" => a.cos(),
                _ => a.tan(),
            }))
        }
        "asin" | "acos" | "atan" => {
            arity(1)?;
            let v = need(first()?, Dim::NONE, "a plain number")?;
            Ok(Quantity {
                value: match name {
                    "asin" => v.asin(),
                    "acos" => v.acos(),
                    _ => v.atan(),
                },
                dim: Dim::ANGLE,
            })
        }
        "atan2" => {
            arity(2)?;
            let (y, x) = (args[0], args[1]);
            if y.dim != x.dim {
                return Err(ExprError::BadArgument(
                    name.into(),
                    "two values of one unit".into(),
                ));
            }
            Ok(Quantity {
                value: y.value.atan2(x.value),
                dim: Dim::ANGLE,
            })
        }
        _ => Err(ExprError::UnknownFunction(name.into())),
    }
}

/// Evaluates an expression to a quantity.
pub fn evaluate(src: &str, names: &impl Names) -> Result<Quantity, ExprError> {
    if src.chars().count() > MAX_EXPRESSION_LEN {
        return Err(ExprError::TooLong);
    }
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return Err(ExprError::Empty);
    }
    let mut p = Parser {
        toks,
        pos: 0,
        depth: 0,
        names,
    };
    let q = p.expr()?;
    if p.pos != p.toks.len() {
        return Err(p.unexpected());
    }
    if !q.value.is_finite() {
        return Err(ExprError::NotFinite);
    }
    Ok(q)
}

/// Evaluates an expression that must be a length (mm) or angle (radians); a plain number
/// means mm or degrees.
pub fn evaluate_as(src: &str, expected: Dim, names: &impl Names) -> Result<f64, ExprError> {
    let q = evaluate(src, names)?;
    if q.dim == expected {
        return Ok(q.value);
    }
    if q.dim == Dim::NONE {
        return Ok(if expected == Dim::ANGLE {
            q.value.to_radians()
        } else {
            q.value
        });
    }
    Err(ExprError::WrongUnit {
        expected: expected.describe(),
        got: q.dim.describe(),
    })
}

/// The parameter names an expression refers to (for dependency order and "used by").
pub fn names_in(src: &str) -> Vec<String> {
    let Ok(toks) = tokenize(src) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, (t, _)) in toks.iter().enumerate() {
        if let Tok::Ident(name) = t {
            let is_call = toks.get(i + 1).is_some_and(|n| n.0 == Tok::LParen);
            let after_number_or_paren = i > 0
                && toks
                    .get(i - 1)
                    .is_some_and(|p| matches!(p.0, Tok::Num(_) | Tok::RParen));
            let is_unit = unit(name).is_some() && after_number_or_paren;
            if !is_call && !is_unit && name != "pi" && !out.contains(name) {
                out.push(name.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn eval(s: &str) -> Result<Quantity, ExprError> {
        evaluate(s, &NoNames)
    }

    fn length(s: &str) -> f64 {
        evaluate_as(s, Dim::LENGTH, &NoNames).unwrap()
    }

    #[test]
    fn numbers_units_and_precedence() {
        assert_eq!(length("80"), 80.0);
        assert_eq!(length("80 mm"), 80.0);
        assert_eq!(length("8cm"), 80.0);
        assert_eq!(length("1 in"), 25.4);
        assert_eq!(length("2\""), 50.8);
        assert_eq!(length("0.5 m + 2 cm"), 520.0);
        assert_eq!(length("2 + 3 * 4"), 14.0);
        assert_eq!(length("(2 + 3) * 4"), 20.0);
        assert_eq!(length("2 ^ 3 ^ 2"), 512.0, "^ is right-associative");
        assert_eq!(
            length("-2 ^ 2"),
            4.0,
            "unary minus binds tighter here, as written"
        );
        assert_eq!(length("1.5e2 mm"), 150.0);
        assert_eq!(length("(10 + 5) mm"), 15.0);
    }

    #[test]
    fn angles_default_to_degrees_and_convert() {
        let a = evaluate_as("90", Dim::ANGLE, &NoNames).unwrap();
        assert!((a - PI / 2.0).abs() < 1e-15);
        let b = evaluate_as("30 deg * 3", Dim::ANGLE, &NoNames).unwrap();
        assert!((b - PI / 2.0).abs() < 1e-15);
        let c = evaluate_as("45°", Dim::ANGLE, &NoNames).unwrap();
        assert!((c - PI / 4.0).abs() < 1e-15);
        assert!((eval("sin(30 deg)").unwrap().value - 0.5).abs() < 1e-15);
        assert!(
            (eval("cos(60)").unwrap().value - 0.5).abs() < 1e-15,
            "plain = degrees"
        );
        let d = evaluate_as("atan2(1 mm, 1 mm)", Dim::ANGLE, &NoNames).unwrap();
        assert!((d - PI / 4.0).abs() < 1e-15);
    }

    #[test]
    fn units_are_checked() {
        assert!(matches!(
            eval("10 mm + 5 deg"),
            Err(ExprError::Mismatch { .. })
        ));
        assert!(matches!(
            evaluate_as("10 mm * 10 mm", Dim::LENGTH, &NoNames),
            Err(ExprError::WrongUnit { .. })
        ));
        // sqrt of an area is a length: the hypotenuse of a 3-4-5 triangle.
        assert_eq!(length("sqrt((3 mm)^2 + (4 mm)^2)"), 5.0);
        assert!(eval("sqrt(4 mm)").is_err(), "odd power under a root");
        assert!(eval("(2 mm) ^ 0.5").is_err(), "fractional power of a unit");
        assert!(eval("2 ^ (1 mm)").is_err());
        assert!(eval("5 mm mm").is_err());
        assert!(eval("mm").is_err());
    }

    #[test]
    fn parameters_are_looked_up() {
        let mut p = BTreeMap::new();
        p.insert(
            "width".to_owned(),
            Quantity {
                value: 80.0,
                dim: Dim::LENGTH,
            },
        );
        p.insert(
            "n".to_owned(),
            Quantity {
                value: 4.0,
                dim: Dim::NONE,
            },
        );
        assert_eq!(evaluate_as("width / n", Dim::LENGTH, &p).unwrap(), 20.0);
        assert_eq!(
            evaluate_as("max(width / 2, 30 mm)", Dim::LENGTH, &p).unwrap(),
            40.0
        );
        assert!(matches!(
            evaluate("height", &p),
            Err(ExprError::UnknownName(_))
        ));
        assert_eq!(
            names_in("width / n + 3 mm * pi + sqrt(width)"),
            vec!["width", "n"]
        );
        // A parameter named like a unit is a name when it isn't right after a number.
        assert_eq!(names_in("m * 2"), vec!["m"]);
    }

    #[test]
    fn bad_expressions_are_errors_not_panics() {
        for bad in [
            "",
            " ",
            "(",
            ")",
            "1 +",
            "* 2",
            "1 2",
            "sqrt()",
            "sqrt(1, 2)",
            "nope(1)",
            "1 / 0",
            "0/0",
            "1e999",
            "@",
            "max()",
            "atan2(1)",
            "acos(2)",
            "1..2",
            ".",
            "1e",
            "°",
        ] {
            assert!(eval(bad).is_err(), "{bad:?} should fail");
        }
        assert_eq!(eval(&"(".repeat(200)), Err(ExprError::TooDeep));
        assert_eq!(eval(&"1+".repeat(600)), Err(ExprError::TooLong));
        assert!(eval(&"-".repeat(500).replace("--", "- ")).is_err());
    }
}
