//! Data for a graphics template: a table of rows read from CSV, TSV or JSON
//! text, and each cell read as the control it feeds.
//!
//! Nothing here opens a file. The caller reads the text, on an IO thread and
//! under its own size limit, and hands it over.

use serde::{Deserialize, Serialize};

/// The most rows a table keeps. A file with more is refused, not cut short.
pub const MAX_ROWS: usize = 100_000;
/// The most columns a table keeps.
pub const MAX_COLUMNS: usize = 512;

/// A table: one name per column, and each row's cells in column order. Every
/// row is as long as `columns`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    /// The column names, from the file's first line or its objects' keys.
    pub columns: Vec<String>,
    /// The rows, each as long as `columns`.
    pub rows: Vec<Vec<String>>,
}

/// Why a file could not be read as a table.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TableError {
    /// Nothing in the file but white space.
    #[error("the file is empty")]
    Empty,
    /// More rows than [`MAX_ROWS`].
    #[error("the file has more than {MAX_ROWS} rows")]
    TooManyRows,
    /// More columns than [`MAX_COLUMNS`].
    #[error("the file has more than {MAX_COLUMNS} columns")]
    TooManyColumns,
    /// JSON that would not parse, with the parser's own words.
    #[error("the JSON could not be read: {0}")]
    Json(String),
    /// JSON that parsed and holds no list of rows.
    #[error("the JSON holds no list of rows")]
    NoRows,
}

/// How a data file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Cells parted by a comma, or by a semicolon where the comma is the
    /// decimal mark. Which one is read from the first line.
    Csv,
    /// Cells parted by a tab.
    Tsv,
    /// A list of objects, or a list of lists whose first is the names.
    Json,
}

impl Format {
    /// The format a file name's ending says, or `None` for any other ending.
    #[must_use]
    pub fn of_name(name: &str) -> Option<Format> {
        let ending = name.rsplit('.').next()?.to_ascii_lowercase();
        match ending.as_str() {
            "csv" => Some(Format::Csv),
            "tsv" | "tab" => Some(Format::Tsv),
            "json" => Some(Format::Json),
            _ => None,
        }
    }
}

/// Read `text` as a table.
///
/// # Errors
///
/// [`TableError`] when the text holds no table or one past the limits.
pub fn parse(text: &str, format: Format) -> Result<Table, TableError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim().is_empty() {
        return Err(TableError::Empty);
    }
    match format {
        Format::Csv => delimited(text, sniff(text)),
        Format::Tsv => delimited(text, '\t'),
        Format::Json => json(text),
    }
}

/// The character parting the cells of a CSV file's first line: whichever of
/// comma, semicolon and tab it holds most of outside quotes. A spreadsheet in
/// a country that writes 1,5 for one and a half saves its CSV with semicolons.
fn sniff(text: &str) -> char {
    let mut counts = [0usize; 3];
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' if !quoted => break,
            ',' if !quoted => counts[0] += 1,
            ';' if !quoted => counts[1] += 1,
            '\t' if !quoted => counts[2] += 1,
            _ => {}
        }
    }
    if counts[1] > counts[0] && counts[1] >= counts[2] {
        ';'
    } else if counts[2] > counts[0] {
        '\t'
    } else {
        ','
    }
}

/// Read delimiter-parted text (RFC 4180): a cell in double quotes may hold the
/// delimiter, line breaks and `""` for one quote. The first line is the names.
fn delimited(text: &str, delimiter: char) -> Result<Table, TableError> {
    let mut lines: Vec<Vec<String>> = Vec::new();
    let mut line: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    // Whether the cell being read opened with a quote, so `a"b` keeps its
    // quote and `"a"` loses both.
    let mut opened = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cell.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                cell.push(c);
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() && !opened => {
                quoted = true;
                opened = true;
            }
            c if c == delimiter => {
                line.push(std::mem::take(&mut cell));
                opened = false;
                if line.len() > MAX_COLUMNS {
                    return Err(TableError::TooManyColumns);
                }
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                line.push(std::mem::take(&mut cell));
                opened = false;
                lines.push(std::mem::take(&mut line));
                if lines.len() > MAX_ROWS + 1 {
                    return Err(TableError::TooManyRows);
                }
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || opened || !line.is_empty() {
        line.push(cell);
        lines.push(line);
    }
    // A blank line is not a row: a file ends with one, and people leave them.
    lines.retain(|l| l.iter().any(|c| !c.trim().is_empty()));
    table_of(lines)
}

/// A table from its lines, the first being the names. Short rows are padded
/// and long ones grow the table, under a name made from the column's number.
fn table_of(mut lines: Vec<Vec<String>>) -> Result<Table, TableError> {
    if lines.is_empty() {
        return Err(TableError::Empty);
    }
    let mut columns: Vec<String> = lines.remove(0);
    for name in &mut columns {
        *name = name.trim().to_owned();
    }
    let width = lines.iter().map(Vec::len).fold(columns.len(), usize::max);
    if width > MAX_COLUMNS {
        return Err(TableError::TooManyColumns);
    }
    if lines.len() > MAX_ROWS {
        return Err(TableError::TooManyRows);
    }
    for n in columns.len()..width {
        columns.push((n + 1).to_string());
    }
    for (n, name) in columns.iter_mut().enumerate() {
        if name.is_empty() {
            *name = (n + 1).to_string();
        }
    }
    for row in &mut lines {
        row.resize(width, String::new());
    }
    Ok(Table {
        columns,
        rows: lines,
    })
}

/// One JSON object with its keys in the order the file has them. The JSON
/// reader's own object sorts its keys, and a table's columns are in the order
/// their file names them.
struct Ordered<V>(Vec<(String, V)>);

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Ordered<V> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Keys<V>(std::marker::PhantomData<V>);
        impl<'de, V: Deserialize<'de>> serde::de::Visitor<'de> for Keys<V> {
            type Value = Ordered<V>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Ordered<V>, A::Error> {
                let mut pairs = Vec::new();
                while let Some(pair) = map.next_entry::<String, V>()? {
                    pairs.push(pair);
                }
                Ok(Ordered(pairs))
            }
        }
        d.deserialize_map(Keys(std::marker::PhantomData))
    }
}

/// One member of a JSON list of rows: an object, or a list of cells.
#[derive(Deserialize)]
#[serde(untagged)]
enum Member {
    Row(Ordered<serde_json::Value>),
    Cells(Vec<serde_json::Value>),
}

/// What an object holding the list has under one of its keys: the list, or
/// something else, which is passed over.
#[derive(Deserialize)]
#[serde(untagged)]
enum Held {
    List(Vec<Member>),
    Other(serde::de::IgnoredAny),
}

/// A JSON file's outermost value: the list itself, or an object holding it.
#[derive(Deserialize)]
#[serde(untagged)]
enum Outer {
    List(Vec<Member>),
    Holder(Ordered<Held>),
}

/// Read JSON: a list of objects, whose keys are the columns in the order they
/// are first met, or a list of lists, whose first is the names. Either may sit
/// under one key of an object, as `{"rows": [...]}`.
fn json(text: &str) -> Result<Table, TableError> {
    let list = match serde_json::from_str::<Outer>(text) {
        Ok(Outer::List(list)) => list,
        Ok(Outer::Holder(Ordered(pairs))) => pairs
            .into_iter()
            .find_map(|(_, held)| match held {
                Held::List(list) if !list.is_empty() => Some(list),
                _ => None,
            })
            .ok_or(TableError::NoRows)?,
        // Well-formed JSON that is neither is a file with no rows in it.
        Err(e) => {
            return Err(match serde_json::from_str::<serde::de::IgnoredAny>(text) {
                Ok(_) => TableError::NoRows,
                Err(_) => TableError::Json(e.to_string()),
            });
        }
    };
    if list.len() > MAX_ROWS + 1 {
        return Err(TableError::TooManyRows);
    }
    if list.iter().all(|m| matches!(m, Member::Cells(_))) {
        let lines = list
            .into_iter()
            .map(|row| match row {
                Member::Cells(cells) => cells.iter().map(cell_of).collect(),
                Member::Row(_) => Vec::new(),
            })
            .collect();
        return table_of(lines);
    }
    let mut columns: Vec<String> = Vec::new();
    for row in &list {
        let Member::Row(Ordered(pairs)) = row else {
            return Err(TableError::NoRows);
        };
        for (key, _) in pairs {
            if !columns.iter().any(|c| c == key) {
                columns.push(key.clone());
                if columns.len() > MAX_COLUMNS {
                    return Err(TableError::TooManyColumns);
                }
            }
        }
    }
    let rows = list
        .iter()
        .map(|row| {
            let pairs: &[(String, serde_json::Value)] = match row {
                Member::Row(Ordered(pairs)) => pairs,
                Member::Cells(_) => &[],
            };
            columns
                .iter()
                .map(|c| {
                    pairs
                        .iter()
                        .find(|(key, _)| key == c)
                        .map(|(_, v)| cell_of(v))
                        .unwrap_or_default()
                })
                .collect()
        })
        .collect();
    Ok(Table { columns, rows })
}

/// One JSON value as a cell's text: a string as it is, a number or a boolean
/// as written, nothing for null, a list as its members parted by commas (a
/// point is `[960, 540]`), and anything else as its JSON.
fn cell_of(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(list) => list.iter().map(cell_of).collect::<Vec<_>>().join(", "),
        Value::Object(_) => value.to_string(),
    }
}

/// Write a table as CSV, quoting only the cells that need it. What **Save a
/// starting file** writes, and what a spreadsheet opens.
#[must_use]
pub fn to_csv(table: &Table) -> String {
    let line = |cells: &[String]| -> String {
        cells
            .iter()
            .map(|cell| {
                if cell.contains([',', '"', '\n', '\r']) || cell.trim() != cell {
                    format!("\"{}\"", cell.replace('"', "\"\""))
                } else {
                    cell.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut out = line(&table.columns);
    out.push_str("\r\n");
    for row in &table.rows {
        out.push_str(&line(row));
        out.push_str("\r\n");
    }
    out
}

/// A number as a cell writes it: `1.5` or `1,5`, with or without a unit such
/// as `%` or `px` after it, and spaces anywhere. `None` for anything else.
#[must_use]
pub fn number(cell: &str) -> Option<f64> {
    let digits: String = cell
        .trim()
        .trim_end_matches(|c: char| c.is_alphabetic() || c == '%' || c == '°')
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    // A comma is the decimal mark when it is the only mark there is. With a
    // point as well it parts thousands, as in 1,250.5.
    let plain = if digits.contains('.') {
        digits.replace(',', "")
    } else if digits.matches(',').count() == 1 {
        digits.replace(',', ".")
    } else {
        digits.replace(',', "")
    };
    plain.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A tick as a cell writes it: yes or no, true or false, on or off, 1 or 0, in
/// any case. `None` for anything else.
#[must_use]
pub fn tick(cell: &str) -> Option<bool> {
    match cell.trim().to_ascii_lowercase().as_str() {
        "yes" | "y" | "true" | "on" | "1" => Some(true),
        "no" | "n" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// A colour as a cell writes it: `#RGB`, `#RRGGBB` or `#RRGGBBAA`, with or
/// without the hash, as display values from 0 to 1. `None` for anything else.
#[must_use]
pub fn colour(cell: &str) -> Option<[f64; 4]> {
    let hex = cell.trim().trim_start_matches('#');
    if !hex.is_ascii() {
        return None;
    }
    let pair = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    let nibble = |at: usize| {
        let v = u8::from_str_radix(hex.get(at..=at)?, 16).ok()?;
        Some(v * 17)
    };
    let bytes = match hex.len() {
        3 => [nibble(0)?, nibble(1)?, nibble(2)?, 255],
        6 => [pair(0)?, pair(2)?, pair(4)?, 255],
        8 => [pair(0)?, pair(2)?, pair(4)?, pair(6)?],
        _ => return None,
    };
    Some(bytes.map(|b| f64::from(b) / 255.0))
}

/// A point as a cell writes it: two or three numbers parted by a comma, a
/// semicolon or a space, with brackets round them or not. `None` for anything
/// else. A missing third number is 0.
#[must_use]
pub fn point(cell: &str) -> Option<[f64; 3]> {
    let inner = cell
        .trim()
        .trim_start_matches(['(', '['])
        .trim_end_matches([')', ']']);
    let parted: Vec<&str> = if inner.contains(';') {
        inner.split(';').collect()
    } else if inner.contains(',') {
        inner.split(',').collect()
    } else {
        inner.split_whitespace().collect()
    };
    let numbers: Option<Vec<f64>> = parted
        .iter()
        .map(|p| p.trim().parse::<f64>().ok())
        .collect();
    match numbers?.as_slice() {
        [x, y] => Some([*x, *y, 0.0]),
        [x, y, z] => Some([*x, *y, *z]),
        _ => None,
    }
}

/// Which of `options` a cell picks: by name, in any case, or by number from 1.
#[must_use]
pub fn option(cell: &str, options: &[String]) -> Option<u32> {
    let wanted = cell.trim();
    if let Some(at) = options
        .iter()
        .position(|o| o.trim().eq_ignore_ascii_case(wanted))
    {
        return u32::try_from(at).ok();
    }
    let n = wanted.parse::<usize>().ok()?;
    (1..=options.len())
        .contains(&n)
        .then(|| u32::try_from(n - 1).ok())
        .flatten()
}

/// A name as columns and controls are matched by: lower case, with nothing but
/// its letters and digits, so `First Name`, `first_name` and `firstname` are
/// one name.
#[must_use]
pub fn matched_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The cells a spreadsheet writes that a careless reader breaks on: the
    /// delimiter, a quote and a line break inside quotes, a trailing blank
    /// line, a short row, and a semicolon file.
    #[test]
    fn a_csv_keeps_quoted_commas_quotes_and_line_breaks() {
        let text = "\u{feff}Name,Title,Note\r\n\"Lovelace, Ada\",\"The \"\"first\"\"\",\"two\nlines\"\r\nGrace,,\r\nShort\r\n\r\n";
        let table = parse(text, Format::Csv).unwrap();
        assert_eq!(table.columns, ["Name", "Title", "Note"]);
        assert_eq!(
            table.rows,
            [
                vec!["Lovelace, Ada", "The \"first\"", "two\nlines"],
                vec!["Grace", "", ""],
                vec!["Short", "", ""],
            ]
        );
        // What was written reads back the same.
        assert_eq!(parse(&to_csv(&table), Format::Csv).unwrap(), table);

        let semicolons = parse("Name;Size\nAda;1,5\n", Format::Csv).unwrap();
        assert_eq!(semicolons.columns, ["Name", "Size"]);
        assert_eq!(semicolons.rows, [vec!["Ada", "1,5"]]);
        assert_eq!(number("1,5"), Some(1.5));
        assert_eq!(number("1,250.5 px"), Some(1250.5));
    }

    /// JSON as people have it: a list of objects whose keys differ from row
    /// to row, numbers and booleans unquoted, and the list under a key.
    #[test]
    fn json_objects_become_rows_under_the_union_of_their_keys() {
        let text = r##"{"rows":[{"Name":"Ada","Size":72,"On":true},{"Name":"Grace","Colour":"#f80","At":[960,540]}]}"##;
        let table = parse(text, Format::Json).unwrap();
        assert_eq!(table.columns, ["Name", "Size", "On", "Colour", "At"]);
        assert_eq!(
            table.rows,
            [
                vec!["Ada", "72", "true", "", ""],
                vec!["Grace", "", "", "#f80", "960, 540"],
            ]
        );
        assert_eq!(point("960, 540"), Some([960.0, 540.0, 0.0]));
        assert_eq!(colour("#f80"), Some([1.0, 136.0 / 255.0, 0.0, 1.0]));
        assert_eq!(tick("Yes"), Some(true));
        assert_eq!(
            option("dark", &["Light".to_owned(), "Dark".to_owned()]),
            Some(1)
        );
    }
}
