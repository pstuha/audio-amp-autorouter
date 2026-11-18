//! Minimal S-expression reader/writer for KiCad's text formats.
//!
//! KiCad `.kicad_pcb` files are S-expressions. We only need to read them, but a
//! small writer keeps the module testable and lets the GUI round-trip a board it
//! has modified.

use std::fmt::Write as _;

/// One S-expression node.
#[derive(Debug, Clone, PartialEq)]
pub enum Sexpr {
    Atom(String),
    List(Vec<Sexpr>),
}

impl Sexpr {
    /// First atom of a list — the node "kind" (`footprint`, `segment`, ...).
    pub fn kind(&self) -> Option<&str> {
        match self {
            Sexpr::List(items) => match items.first() {
                Some(Sexpr::Atom(a)) => Some(a.as_str()),
                _ => None,
            },
            Sexpr::Atom(a) => Some(a.as_str()),
        }
    }

    pub fn items(&self) -> &[Sexpr] {
        match self {
            Sexpr::List(items) => items,
            Sexpr::Atom(_) => &[],
        }
    }

    /// `true` when this is a list whose first atom equals `kind`.
    pub fn is(&self, kind: &str) -> bool {
        self.kind() == Some(kind)
    }

    /// Child lists of the given kind.
    pub fn children<'a, 'b>(&'a self, kind: &'b str) -> impl Iterator<Item = &'a Sexpr> + 'a
    where
        'b: 'a,
    {
        self.items().iter().filter(move |s| s.is(kind))
    }

    /// First child list of the given kind.
    pub fn child(&self, kind: &str) -> Option<&Sexpr> {
        self.items().iter().find(|s| s.is(kind))
    }

    /// Atom at `index`, if it is an atom.
    pub fn atom(&self, index: usize) -> Option<&str> {
        match self.items().get(index) {
            Some(Sexpr::Atom(a)) => Some(a.as_str()),
            _ => None,
        }
    }

    /// Number at `index`.
    pub fn number(&self, index: usize) -> Option<f64> {
        self.atom(index).and_then(|a| a.parse().ok())
    }

    /// The single number inside a list such as `(width 0.4)`.
    pub fn value(&self) -> Option<f64> {
        self.items().iter().find_map(|s| match s {
            Sexpr::Atom(a) => a.parse().ok(),
            Sexpr::List(_) => None,
        })
    }

    /// All atoms of this list after the head.
    pub fn atoms(&self) -> Vec<&str> {
        self.items()
            .iter()
            .filter_map(|s| match s {
                Sexpr::Atom(a) => Some(a.as_str()),
                Sexpr::List(_) => None,
            })
            .collect()
    }

    /// Render back to text.
    pub fn to_string(&self) -> String {
        let mut out = String::new();
        self.write_to(&mut out, 0);
        out
    }

    fn write_to(&self, out: &mut String, indent: usize) {
        match self {
            Sexpr::Atom(a) => {
                // Atoms containing whitespace, quotes or parentheses must be
                // re-quoted, or the text would parse back into several atoms.
                if a.is_empty()
                    || a.chars()
                        .any(|c| c.is_whitespace() || c == '"' || c == '(' || c == ')')
                {
                    out.push('"');
                    out.push_str(&a.replace('\\', "\\\\").replace('"', "\\\""));
                    out.push('"');
                } else {
                    out.push_str(a);
                }
            }
            Sexpr::List(items) => {
                let simple = items.iter().all(|s| matches!(s, Sexpr::Atom(_)));
                let _ = write!(out, "(");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 && !simple {
                        out.push('\n');
                        out.push_str(&"  ".repeat(indent + 1));
                    } else if i > 0 {
                        out.push(' ');
                    }
                    item.write_to(out, indent + 1);
                }
                out.push(')');
            }
        }
    }
}

/// Parse a whole document into a single S-expression.
pub fn parse(input: &str) -> Result<Sexpr, String> {
    let mut p = Parser {
        bytes: input.as_bytes(),
        pos: 0,
        depth: 0,
    };
    p.skip_ws();
    let node = p.parse_node()?;
    p.skip_ws();
    if p.pos < p.bytes.len() {
        return Err(format!("trailing input at byte {}", p.pos));
    }
    Ok(node)
}

/// Maximum nesting we accept, to fail fast on malformed input instead of
/// recursing until the stack overflows.
const MAX_DEPTH: usize = 200;

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn parse_node(&mut self) -> Result<Sexpr, String> {
        self.skip_ws();
        if self.pos >= self.bytes.len() {
            return Err("unexpected end of input".into());
        }
        match self.bytes[self.pos] {
            b'(' => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(format!("nesting deeper than {MAX_DEPTH}"));
                }
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    if self.pos >= self.bytes.len() {
                        return Err("unterminated list".into());
                    }
                    if self.bytes[self.pos] == b')' {
                        self.pos += 1;
                        self.depth -= 1;
                        return Ok(Sexpr::List(items));
                    }
                    items.push(self.parse_node()?);
                }
            }
            b'"' => self.parse_quoted(),
            b')' => Err(format!("unexpected ')' at byte {}", self.pos)),
            _ => self.parse_atom(),
        }
    }

    fn parse_quoted(&mut self) -> Result<Sexpr, String> {
        self.pos += 1; // opening quote
        let mut s = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            match b {
                b'"' => {
                    self.pos += 1;
                    return Ok(Sexpr::Atom(s));
                }
                b'\\' => {
                    self.pos += 1;
                    if self.pos >= self.bytes.len() {
                        return Err("unterminated escape".into());
                    }
                    let esc = self.bytes[self.pos];
                    s.push(match esc {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        other => other as char,
                    });
                    self.pos += 1;
                }
                _ => {
                    // KiCad files are UTF-8; copy whole characters.
                    let ch_len = utf8_len(b);
                    let end = (self.pos + ch_len).min(self.bytes.len());
                    s.push_str(&String::from_utf8_lossy(&self.bytes[self.pos..end]));
                    self.pos = end;
                }
            }
        }
        Err("unterminated string".into())
    }

    fn parse_atom(&mut self) -> Result<Sexpr, String> {
        let start = self.pos;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b'"' => break,
                _ => self.pos += 1,
            }
        }
        if self.pos == start {
            return Err(format!("empty atom at byte {start}"));
        }
        Ok(Sexpr::Atom(
            String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned(),
        ))
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_lists_and_atoms() {
        let s = parse("(a (b 1) c)").unwrap();
        assert_eq!(s.kind(), Some("a"));
        assert_eq!(s.items().len(), 3);
        let b = s.child("b").unwrap();
        assert_eq!(b.value(), Some(1.0));
        assert_eq!(s.atom(2), Some("c"));
    }

    #[test]
    fn handles_quoted_strings_with_spaces() {
        let s = parse(r#"(footprint "R Test" (at 1 2))"#).unwrap();
        assert_eq!(s.atom(1), Some("R Test"));
        assert_eq!(s.child("at").unwrap().number(1), Some(1.0));
        assert_eq!(s.child("at").unwrap().number(2), Some(2.0));
    }

    #[test]
    fn handles_escaped_quotes() {
        let s = parse(r#"(a "x\"y")"#).unwrap();
        assert_eq!(s.atom(1), Some("x\"y"));
    }

    #[test]
    fn round_trips_through_text() {
        let src = "(a (b 1) (c \"q r\"))";
        let once = parse(src).unwrap();
        let again = parse(&once.to_string()).unwrap();
        assert_eq!(once, again);
    }

    #[test]
    fn rejects_unbalanced_input() {
        assert!(parse("(a (b 1)").is_err());
        assert!(parse("(a))").is_err());
    }

    #[test]
    fn rejects_pathological_nesting() {
        let deep = "(".repeat(500) + &")".repeat(500);
        assert!(parse(&deep).is_err());
    }
}
