//! A text document (`btds`): Adobe's text engine record, a dictionary in PDF's
//! syntax whose keys are numbers, read into the words and settings the After
//! Effects script writes for the same layer, so the mapping reads one shape
//! whichever way the project came in.
//!
//! Only the keys the two ways in were checked against are read: the words, the
//! font, its size and tracking, the fill and the outline, the alignment and a
//! box's size and corner. Anything else keeps the script's own default.

use serde_json::{json, Map, Value};

use super::*;

/// A text property: the metadata in a `tdbs` beside one `btdk` per value, key
/// *n*'s time from the `tdbs` and its value from the *n*th `btdk`, as a path's
/// are.
pub(super) fn read_text(
    match_name: &str,
    btds: &Chunk<'_>,
    run: &[Chunk<'_>],
    ctx: Ctx<'_>,
    skipped: &mut Vec<Unreadable>,
) -> Property {
    let inside: Vec<Chunk<'_>> = btds.children().ok().collect();
    let documents: Vec<Value> = inside
        .iter()
        .filter(|chunk| chunk.is_list(b"btdk"))
        .filter_map(|chunk| document(chunk.body))
        .collect();
    let Some(first) = documents.first().cloned() else {
        return unreadable_node(match_name, "the text document could not be read");
    };
    let mut leaf = match inside.iter().find(|chunk| chunk.is_list(b"tdbs")) {
        Some(tdbs) if documents.len() > 1 => read_leaf(match_name, tdbs, run, ctx, skipped),
        _ => Property {
            match_name: Some(match_name.to_string()),
            ..Property::default()
        },
    };
    leaf.value_type = Some("text_document".to_string());
    leaf.unreadable = None;
    leaf.value = Some(first);
    if let Some(keys) = leaf.keyframes.as_mut() {
        for (key, document) in keys.iter_mut().zip(documents) {
            key.v = Some(document);
        }
    }
    leaf
}

/// One document's words and settings, in the names the script uses.
fn document(bytes: &[u8]) -> Option<Value> {
    let root = Parser {
        bytes,
        at: 0,
        depth: 0,
    }
    .entries(None)?;
    let fonts: Vec<&str> = root
        .get("0")?
        .get("1")?
        .get("0")?
        .items()
        .iter()
        .filter_map(|font| font.get("0")?.get("0")?.get("0")?.text())
        .collect();
    let story = root.get("1")?.get("1")?.items().first()?;
    let content = story.get("0")?;
    let mut out = Map::new();
    // The record closes the last paragraph with a return the script leaves off.
    let text = content.get("0")?.text()?;
    out.insert(
        "text".into(),
        json!(text.strip_suffix('\r').unwrap_or(text)),
    );

    let first_run = |key: &str| {
        let run = content.get(key)?.get("0")?.items().first()?;
        run.get("0")?.get("0")?.get(key)
    };
    if let Some(style) = first_run("6") {
        let font = style.number("0").and_then(|i| fonts.get(i as usize));
        if let Some(font) = font {
            out.insert("font".into(), json!(font));
        }
        for (key, name) in [
            ("1", "fontSize"),
            ("6", "horizontalScale"),
            ("7", "verticalScale"),
            ("8", "tracking"),
            ("63", "strokeWidth"),
        ] {
            if let Some(v) = style.number(key) {
                out.insert(name.into(), json!(v));
            }
        }
        for (key, name) in [
            ("2", "fauxBold"),
            ("3", "fauxItalic"),
            ("56", "applyFill"),
            ("57", "applyStroke"),
        ] {
            if let Some(Cos::Bool(on)) = style.get(key) {
                out.insert(name.into(), json!(on));
            }
        }
        for (key, name) in [("53", "fillColor"), ("54", "strokeColor")] {
            if let Some(colour) = style.get(key).and_then(paint) {
                out.insert(name.into(), json!(colour));
            }
        }
    }
    // Left, right, centre, then the four that justify a box, which After
    // Effects' scripting numbers from 7413.
    if let Some(align) = first_run("5").and_then(|style| style.number("0")) {
        out.insert("justification".into(), json!(7413 + align as i64));
    }
    // Box text keeps its box on the layout's frame: the corner, then the size.
    let frame = story
        .get("1")
        .and_then(|layout| layout.get("2")?.items().first()?.get("6")?.items().first());
    let bounds = frame.and_then(|f| f.get("1")).map(Cos::numbers);
    match (
        frame.and_then(|f| f.get("0")?.get("0")).map(Cos::numbers),
        bounds,
    ) {
        (Some(corner), Some(b)) if corner.len() == 2 && b.len() == 4 => {
            out.insert("boxText".into(), json!(true));
            out.insert("boxTextSize".into(), json!([b[2] - b[0], b[3] - b[1]]));
            out.insert("boxTextPos".into(), json!(corner));
        }
        _ => {
            out.insert("boxText".into(), json!(false));
        }
    }
    Some(Value::Object(out))
}

/// A `SimplePaint`'s colour, red first, as the script writes it. The record
/// keeps alpha first.
fn paint(value: &Cos) -> Option<[f64; 3]> {
    let argb = value.get("0")?.get("1")?.numbers();
    match argb.as_slice() {
        [_, r, g, b] => Some([*r, *g, *b]),
        _ => None,
    }
}

/// One value of the record.
#[derive(Debug, Clone, PartialEq)]
enum Cos {
    Dict(Vec<(String, Cos)>),
    List(Vec<Cos>),
    Number(f64),
    Text(String),
    Name(String),
    Bool(bool),
}

impl Cos {
    fn get(&self, key: &str) -> Option<&Cos> {
        match self {
            Cos::Dict(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn items(&self) -> &[Cos] {
        match self {
            Cos::List(items) => items,
            _ => &[],
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Cos::Text(text) => Some(text),
            _ => None,
        }
    }

    fn number(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            Cos::Number(n) => Some(*n),
            _ => None,
        }
    }

    fn numbers(&self) -> Vec<f64> {
        self.items()
            .iter()
            .filter_map(|item| match item {
                Cos::Number(n) => Some(*n),
                _ => None,
            })
            .collect()
    }
}

/// How deep the record may nest before it is refused. The ones After Effects
/// writes reach about fifteen.
const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.at + ahead).copied()
    }

    fn skip_space(&mut self) {
        while self
            .peek(0)
            .is_some_and(|b| b.is_ascii_whitespace() || b == 0)
        {
            self.at += 1;
        }
    }

    /// Key and value pairs up to `end`, or for the record's own top level,
    /// which opens with no brackets, up to a closing one or the end of the
    /// bytes.
    fn entries(&mut self, end: Option<&[u8; 2]>) -> Option<Cos> {
        let mut out = Vec::new();
        let close = end.unwrap_or(b">>");
        loop {
            self.skip_space();
            match self.peek(0) {
                None if end.is_none() => return Some(Cos::Dict(out)),
                None => return None,
                Some(a) if a == close[0] && self.peek(1) == Some(close[1]) => {
                    self.at += 2;
                    return Some(Cos::Dict(out));
                }
                Some(b'/') => {
                    let Cos::Name(key) = self.name()? else {
                        return None;
                    };
                    let value = self.value()?;
                    out.push((key, value));
                }
                _ => return None,
            }
        }
    }

    fn value(&mut self) -> Option<Cos> {
        self.skip_space();
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let value = match self.peek(0)? {
            b'<' if self.peek(1) == Some(b'<') => {
                self.at += 2;
                self.entries(Some(b">>"))
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_space();
                    if self.peek(0)? == b']' {
                        self.at += 1;
                        break Some(Cos::List(items));
                    }
                    items.push(self.value()?);
                }
            }
            b'(' => self.string(),
            b'/' => self.name(),
            b't' | b'f' => self.word(),
            _ => self.number(),
        };
        self.depth -= 1;
        value
    }

    fn name(&mut self) -> Option<Cos> {
        self.at += 1;
        let start = self.at;
        while self
            .peek(0)
            .is_some_and(|b| !b.is_ascii_whitespace() && !b"/<>[]()".contains(&b))
        {
            self.at += 1;
        }
        let name = self.bytes.get(start..self.at)?;
        Some(Cos::Name(String::from_utf8_lossy(name).into_owned()))
    }

    fn word(&mut self) -> Option<Cos> {
        let rest = self.bytes.get(self.at..)?;
        if rest.starts_with(b"true") {
            self.at += 4;
            Some(Cos::Bool(true))
        } else if rest.starts_with(b"false") {
            self.at += 5;
            Some(Cos::Bool(false))
        } else {
            None
        }
    }

    fn number(&mut self) -> Option<Cos> {
        let start = self.at;
        while self
            .peek(0)
            .is_some_and(|b| b.is_ascii_digit() || b"+-.eE".contains(&b))
        {
            self.at += 1;
        }
        let text = std::str::from_utf8(self.bytes.get(start..self.at)?).ok()?;
        text.parse().ok().map(Cos::Number)
    }

    /// A string in brackets, with backslash escapes. One that starts with
    /// the byte order mark is UTF-16, which is how the words are kept.
    fn string(&mut self) -> Option<Cos> {
        self.at += 1;
        let mut raw = Vec::new();
        loop {
            match self.peek(0)? {
                b')' => {
                    self.at += 1;
                    break;
                }
                b'\\' => {
                    let next = self.peek(1)?;
                    raw.push(match next {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        other => other,
                    });
                    self.at += 2;
                }
                other => {
                    raw.push(other);
                    self.at += 1;
                }
            }
        }
        let text = match raw.strip_prefix(&[0xFE, 0xFF]) {
            Some(wide) => {
                let units: Vec<u16> = wide
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                String::from_utf16_lossy(&units)
            }
            None => String::from_utf8_lossy(&raw).into_owned(),
        };
        Some(Cos::Text(text))
    }
}
