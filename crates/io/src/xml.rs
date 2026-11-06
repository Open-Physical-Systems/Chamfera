//! Just enough XML to read 3MF parts: a forward scanner over tags and their attributes.
//!
//! Text content, DTDs and namespaces are not interpreted (a prefix is cut from each name);
//! comments, processing instructions and CDATA are skipped. Input is hostile: the scanner
//! never recurses, every step moves forward, and attribute counts are capped.

/// Most attributes one tag may have.
const MAX_ATTRIBUTES: usize = 256;

/// A start or end tag.
#[derive(Debug, Clone, PartialEq)]
pub enum Tag<'a> {
    /// `<name a="v">`, or `<name a="v"/>` when `empty`.
    Start {
        name: &'a str,
        attributes: Vec<(&'a str, String)>,
        empty: bool,
    },
    End {
        name: &'a str,
    },
}

impl Tag<'_> {
    /// The value of an attribute (prefix ignored), if present.
    pub fn attribute(&self, key: &str) -> Option<&str> {
        match self {
            Tag::Start { attributes, .. } => attributes
                .iter()
                .find(|(k, _)| local(k) == key)
                .map(|(_, v)| v.as_str()),
            Tag::End { .. } => None,
        }
    }
}

/// The name without its namespace prefix.
pub fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// The tags of `text`, in order.
pub struct Tags<'a> {
    text: &'a str,
    at: usize,
}

pub fn tags(text: &str) -> Tags<'_> {
    Tags { text, at: 0 }
}

impl<'a> Iterator for Tags<'a> {
    type Item = Result<Tag<'a>, String>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let rest = self.text.get(self.at..)?;
            let open = rest.find('<')?;
            let start = self.at + open;
            let tail = self.text.get(start..)?;
            let skip_to = |end: &str| tail.find(end).map(|i| start + i + end.len());
            let skipped = if tail.starts_with("<!--") {
                Some(skip_to("-->"))
            } else if tail.starts_with("<![CDATA[") {
                Some(skip_to("]]>"))
            } else if tail.starts_with("<?") {
                Some(skip_to("?>"))
            } else if tail.starts_with("<!") {
                Some(skip_to(">"))
            } else {
                None
            };
            if let Some(next) = skipped {
                match next {
                    Some(n) => {
                        self.at = n;
                        continue;
                    }
                    None => {
                        self.at = self.text.len();
                        return Some(Err("XML: a comment or declaration is not closed".into()));
                    }
                }
            }
            let Some(close) = tail.find('>') else {
                self.at = self.text.len();
                return Some(Err("XML: a tag is not closed".into()));
            };
            self.at = start + close + 1;
            let inner = tail.get(1..close).unwrap_or("");
            return Some(parse_tag(inner));
        }
    }
}

fn parse_tag(inner: &str) -> Result<Tag<'_>, String> {
    if let Some(name) = inner.strip_prefix('/') {
        return Ok(Tag::End {
            name: local(name.trim()),
        });
    }
    let (body, empty) = match inner.strip_suffix('/') {
        Some(b) => (b, true),
        None => (inner, false),
    };
    let body = body.trim_start();
    let name_end = body
        .find(|c: char| c.is_ascii_whitespace())
        .unwrap_or(body.len());
    let name = body.get(..name_end).unwrap_or("");
    if name.is_empty() {
        return Err("XML: a tag has no name".into());
    }
    let mut rest = body.get(name_end..).unwrap_or("");
    let mut attributes = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if attributes.len() >= MAX_ATTRIBUTES {
            return Err(format!("XML: <{name}> has too many attributes"));
        }
        let eq = rest
            .find('=')
            .ok_or_else(|| format!("XML: an attribute of <{name}> has no value"))?;
        let key = rest.get(..eq).unwrap_or("").trim();
        let after = rest.get(eq + 1..).unwrap_or("").trim_start();
        let quote = after
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'')
            .ok_or_else(|| format!("XML: attribute {key} of <{name}> is not quoted"))?;
        let value_and_rest = after.get(1..).unwrap_or("");
        let end = value_and_rest
            .find(quote)
            .ok_or_else(|| format!("XML: attribute {key} of <{name}> is not closed"))?;
        attributes.push((key, unescape(value_and_rest.get(..end).unwrap_or(""))));
        rest = value_and_rest.get(end + 1..).unwrap_or("");
    }
    Ok(Tag::Start {
        name: local(name),
        attributes,
        empty,
    })
}

/// Replaces the predefined entities and numeric character references. Anything else is
/// kept as written.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let tail = rest.get(i..).unwrap_or("");
        let semi = tail.find(';').filter(|&j| j <= 12);
        let decoded = semi.and_then(|j| {
            let entity = tail.get(1..j)?;
            let c = match entity {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => {
                    let code = if let Some(h) = entity.strip_prefix("#x") {
                        u32::from_str_radix(h, 16).ok()?
                    } else {
                        entity.strip_prefix('#')?.parse().ok()?
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, j + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = tail.get(len..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = tail.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// Escapes text for an attribute value.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_attributes_are_read() {
        let text = r#"<?xml version="1.0"?><!-- c --><m:model unit='inch' a="x &amp; y"><v x="1"/></m:model>"#;
        let t: Vec<Tag> = tags(text).collect::<Result<_, _>>().unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].attribute("unit"), Some("inch"));
        assert_eq!(t[0].attribute("a"), Some("x & y"));
        assert!(matches!(
            t[1],
            Tag::Start {
                name: "v",
                empty: true,
                ..
            }
        ));
        assert_eq!(t[2], Tag::End { name: "model" });
    }

    #[test]
    fn escaping_round_trips() {
        let s = "a<b>&\"c\" 'd' &#65;";
        assert_eq!(unescape(&escape(s)), s);
        assert_eq!(unescape("&#x41;&#66;&bogus;&"), "AB&bogus;&");
    }

    #[test]
    fn broken_xml_does_not_panic() {
        for text in [
            "<",
            "<a",
            "<a b>",
            "<a b=>",
            "<a b=\"1>",
            "<!--",
            "<>",
            "</",
            "<a/",
            "&",
            "<é ü='ö'/>",
        ] {
            let _: Vec<_> = tags(text).collect();
        }
    }
}
