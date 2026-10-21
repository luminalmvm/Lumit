//! The manual's Scripting reference page, written from [`COMMANDS`].
//!
//! # In plain terms
//!
//! Every command carries a one-line summary and a JSON Schema for its
//! arguments, the same ones MCP hands out as its tools. [`reference_page`]
//! turns that table into a page of the manual, a section per command with its
//! arguments in a table, so the page can't list an argument a command no
//! longer takes. A test fails when the page in `web-docs` differs from what
//! this writes, and an ignored one writes it.

use std::fmt::{self, Write as _};

use serde::de::{Deserializer, MapAccess, Visitor};
use serde::Deserialize;
use serde_json::Value;

use crate::mcp::tool_name;
use crate::COMMANDS;

/// What rewrites the page.
const REGENERATE: &str = "cargo test -p lumit-script regenerate_scripting_reference -- --ignored";

const INTRO: &str = "Every command a [script](/use/scripts/) or
[MCP](/use/command-line/#let-another-program-edit-a-project) can run, with its arguments. A
script also runs one as `lumit.command(\"layer.add\", { ... })`.

A time is a frame on the comp's rate, or exact seconds as `{ \"num\": 1001, \"den\": 24000 }`.
";

/// The page's exact text, with a trailing newline.
#[must_use]
pub fn reference_page() -> String {
    let mut page = format!(
        "---\ntitle: Scripting reference\ndescription: Every command a script or MCP can run, \
         and its arguments.\nsidebar:\n  order: 4\n---\n\n<!-- Written by `{REGENERATE}`\nfrom \
         the command table in crates/lumit-script. Edit the table, not this page. -->\n\n{INTRO}"
    );
    for c in COMMANDS {
        let schema: Json = serde_json::from_str(c.schema).unwrap_or(Json::Leaf(Value::Null));
        let mut rows = Vec::new();
        collect(&schema, "", &[], &mut rows);
        let _ = write!(page, "\n## {}\n\n{}.\n", c.name, c.summary);
        if let Some(note) = schema.get("description").and_then(Json::str) {
            let _ = write!(page, "\n{}.\n", capital(note));
        }
        let tool = tool_name(c.name);
        // A script's run is one undo step, so it can't undo or redo.
        if matches!(c.name, "undo" | "redo") {
            let _ = write!(page, "\nMCP tool: `{tool}`. A script cannot run it.\n");
        } else {
            let args = if rows.is_empty() { "" } else { "{ ... }" };
            let _ = write!(
                page,
                "\nScript: `lumit.{}({args})`. MCP tool: `{tool}`.\n",
                c.name
            );
        }
        if rows.is_empty() {
            page.push_str("\nNo arguments.\n");
            continue;
        }
        page.push_str("\n| Argument | Type | Required | Meaning |\n| --- | --- | --- | --- |\n");
        for row in &rows {
            let mut kinds: Vec<String> = Vec::new();
            if !row.values.is_empty() {
                kinds.push(row.values.join(", "));
            }
            kinds.extend(row.kinds.iter().cloned());
            let _ = writeln!(
                page,
                "| `{}` | {} | {} | {} |",
                row.path,
                cell(&kinds.join(" or ")),
                if row.required { "yes" } else { "" },
                cell(&row.about.join("; ")),
            );
        }
        let mut sets = Vec::new();
        choices(&schema, &mut sets);
        for set in sets {
            let _ = write!(page, "\nNeeds one of {}.\n", either(&set));
        }
    }
    page
}

/// One row of a command's argument table.
struct Row {
    path: String,
    /// The values it may take, as JSON, when the schema lists them.
    values: Vec<String>,
    kinds: Vec<String>,
    required: bool,
    about: Vec<String>,
}

/// Every argument of `s` as rows, nested ones as `switches.visible` or
/// `markers[].time`. An argument the schema's forms all name, as
/// `layer.keys`' do, is one row whose types and values are put together.
fn collect(s: &Json, prefix: &str, required: &[&str], out: &mut Vec<Row>) {
    let mut required = required.to_vec();
    required.extend(
        s.get("required")
            .map_or(&[][..], Json::items)
            .iter()
            .filter_map(Json::str),
    );
    for (name, p) in s.get("properties").map_or(&[][..], Json::entries) {
        let path = format!("{prefix}{name}");
        let items = p.get("items").filter(|i| i.get("properties").is_some());
        let kind = if items.is_some() {
            "list".to_owned()
        } else if p.get("properties").is_some() {
            "object".to_owned()
        } else {
            kind(p)
        };
        let values: Vec<String> = p
            .get("enum")
            .map_or(&[][..], Json::items)
            .iter()
            .map(literal)
            .collect();
        merge(
            out,
            Row {
                path: path.clone(),
                kinds: if values.is_empty() {
                    vec![kind]
                } else {
                    Vec::new()
                },
                values,
                required: required.contains(&name.as_str()),
                about: about(p).into_iter().collect(),
            },
        );
        if let Some(items) = items {
            collect(items, &format!("{path}[]."), &[], out);
        } else if p.get("properties").is_some() {
            collect(p, &format!("{path}."), &[], out);
        }
        // An ease's speed and influence get rows of their own, a time's
        // num and den don't, as they say nothing.
        for alt in alternatives(p) {
            let described = alt.get("properties").is_some_and(|props| {
                props
                    .entries()
                    .iter()
                    .any(|(_, v)| v.get("description").is_some())
            });
            if described {
                collect(alt, &format!("{path}."), &[], out);
            }
        }
    }
    for form in alternatives(s) {
        collect(form, prefix, &required, out);
    }
}

fn merge(out: &mut Vec<Row>, row: Row) {
    let Some(have) = out.iter_mut().find(|r| r.path == row.path) else {
        out.push(row);
        return;
    };
    for (into, from) in [
        (&mut have.values, row.values),
        (&mut have.kinds, row.kinds),
        (&mut have.about, row.about),
    ] {
        for v in from {
            if !into.contains(&v) {
                into.push(v);
            }
        }
    }
    have.required |= row.required;
}

/// The groups of arguments of which one must be given, as `layer.keys`'
/// value, keys or expression.
fn choices(s: &Json, out: &mut Vec<Vec<String>>) {
    let alts = alternatives(s);
    let names: Option<Vec<String>> = alts
        .iter()
        .map(
            |a| match (a.entries(), a.get("required").map(Json::items)) {
                ([_], Some([name])) => name.str().map(str::to_owned),
                _ => None,
            },
        )
        .collect();
    match names {
        Some(names) if !names.is_empty() => {
            if !out.contains(&names) {
                out.push(names);
            }
        }
        _ => alts.iter().for_each(|a| choices(a, out)),
    }
}

/// What a schema takes, in a few words: `integer (1 to 999) or {num, den}`.
fn kind(s: &Json) -> String {
    if let Some(values) = s.get("enum") {
        return values
            .items()
            .iter()
            .map(literal)
            .collect::<Vec<_>>()
            .join(", ");
    }
    let alts = alternatives(s);
    if !alts.is_empty() {
        return alts.iter().map(kind).collect::<Vec<_>>().join(" or ");
    }
    match s.get("type") {
        Some(Json::Array(types)) => types
            .iter()
            .filter_map(Json::str)
            .map(|t| word(t, s))
            .collect::<Vec<_>>()
            .join(" or "),
        Some(t) => t.str().map_or_else(|| "any".to_owned(), |t| word(t, s)),
        None => "any".to_owned(),
    }
}

fn word(t: &str, s: &Json) -> String {
    match t {
        "integer" | "number" => format!("{t}{}", bounds(s)),
        "array" => list(s),
        "object" => match s.get("properties") {
            Some(props) => {
                let names: Vec<&str> = props.entries().iter().map(|(n, _)| n.as_str()).collect();
                format!("`{{{}}}`", names.join(", "))
            }
            None => "object".to_owned(),
        },
        _ => t.to_owned(),
    }
}

fn list(s: &Json) -> String {
    let item = match s.get("items") {
        Some(i) => match i.get("type").and_then(Json::str) {
            Some(t @ ("string" | "integer" | "number" | "boolean")) => format!("{t}s{}", bounds(i)),
            _ => kind(i),
        },
        None => "anything".to_owned(),
    };
    let count = |key| s.get(key).and_then(Json::number);
    match (count("minItems"), count("maxItems")) {
        (Some(a), Some(b)) if a == b => format!("list of {a} {item}"),
        (Some(a), Some(b)) => format!("list of {a} to {b} {item}"),
        _ => format!("list of {item}"),
    }
}

/// A number's limits, as ` (16 to 16384)` or ` (over 0, at most 1)`.
fn bounds(s: &Json) -> String {
    let at = |key| s.get(key).and_then(Json::number);
    let parts: Vec<String> = match (at("minimum"), at("maximum")) {
        (Some(a), Some(b)) => vec![format!("{a} to {b}")],
        (min, max) => [
            min.map(|v| format!("at least {v}")),
            at("exclusiveMinimum").map(|v| format!("over {v}")),
            max.map(|v| format!("at most {v}")),
            at("exclusiveMaximum").map(|v| format!("under {v}")),
        ]
        .into_iter()
        .flatten()
        .collect(),
    };
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// A schema's description, or else its alternatives' joined: "A frame on
/// the comp's rate, or exact seconds".
fn about(s: &Json) -> Option<String> {
    if let Some(d) = s.get("description").and_then(Json::str) {
        return Some(d.to_owned());
    }
    let parts: Vec<&str> = alternatives(s)
        .iter()
        .filter_map(|a| a.get("description").and_then(Json::str))
        .collect();
    let (first, rest) = parts.split_first()?;
    Some(rest.iter().fold((*first).to_owned(), |text, part| {
        format!("{text}, or {}", lower(part))
    }))
}

fn alternatives(s: &Json) -> &[Json] {
    ["oneOf", "anyOf", "allOf"]
        .iter()
        .find_map(|key| s.get(key))
        .map_or(&[], Json::items)
}

fn literal(v: &Json) -> String {
    match v {
        Json::Leaf(v) => format!("`{v}`"),
        _ => "`...`".to_owned(),
    }
}

/// "`a`, `b` or `c`".
fn either(names: &[String]) -> String {
    let names: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => names.concat(),
    }
}

fn capital(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// Lower case at the start, unless the word is a name such as "ADBE".
fn lower(text: &str) -> String {
    let mut chars = text.chars();
    match (chars.next(), chars.clone().next()) {
        (Some(a), Some(b)) if !b.is_uppercase() => a.to_lowercase().chain(chars).collect(),
        _ => text.to_owned(),
    }
}

/// A table cell: a `|` would end it early.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// JSON with each object's keys in the order they were written, which
/// `serde_json::Value` doesn't keep, so the table reads as the schema does.
#[derive(Deserialize)]
#[serde(untagged)]
enum Json {
    Object(Ordered),
    Array(Vec<Json>),
    Leaf(Value),
}

struct Ordered(Vec<(String, Json)>);

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = Ordered;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Ordered(entries))
            }
        }
        d.deserialize_map(Entries)
    }
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        self.entries()
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    fn entries(&self) -> &[(String, Json)] {
        match self {
            Json::Object(Ordered(entries)) => entries,
            _ => &[],
        }
    }

    fn items(&self) -> &[Json] {
        match self {
            Json::Array(items) => items,
            _ => &[],
        }
    }

    fn str(&self) -> Option<&str> {
        match self {
            Json::Leaf(Value::String(s)) => Some(s),
            _ => None,
        }
    }

    fn number(&self) -> Option<String> {
        match self {
            Json::Leaf(v @ Value::Number(_)) => Some(v.to_string()),
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn page_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../web-docs/src/content/docs/reference/scripting.md")
    }

    /// The manual's page is the command table's own. A command added, or an
    /// argument changed, fails here until the page is written again.
    #[test]
    fn the_scripting_reference_matches_the_commands() {
        let got = std::fs::read_to_string(page_path()).unwrap_or_default();
        assert!(
            got.replace("\r\n", "\n") == reference_page(),
            "web-docs/src/content/docs/reference/scripting.md is stale. Write it again:\n  \
             {REGENERATE}"
        );
    }

    #[test]
    #[ignore = "writes the manual's page; run after changing a command"]
    fn regenerate_scripting_reference() {
        std::fs::write(page_path(), reference_page()).expect("write scripting.md");
    }
}
