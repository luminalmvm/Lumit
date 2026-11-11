//! Data feeding a template: which column feeds which control, and one row
//! read as the values it gives a copy.
//!
//! The render never reads data. A copy that follows a row has that row's
//! values written into its own ([`crate::fx::effects::template::feed_value`]),
//! and reloading the file writes them again. So a project opens and exports
//! the same with the file gone.

use serde::{Deserialize, Serialize};

use super::data::{self, Table};
use super::{Control, ControlKind, Template, Value};
use crate::model::{Composition, Document};

/// The data linked to a template: the file it came from, its rows as they
/// were last read, and what each column feeds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Data {
    /// The file the rows were read from, for **Reload**.
    pub file: String,
    /// The rows, kept in the project.
    pub table: Table,
    /// What each column feeds, in column order. A short list feeds nothing
    /// from the columns past its end.
    #[serde(default)]
    pub feeds: Vec<Feeds>,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// What a column feeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "feeds", content = "control", rename_all = "snake_case")]
pub enum Feeds {
    /// Nothing.
    Nothing,
    /// A control, by its id.
    Control(String),
    /// A text control's font.
    Font(String),
    /// A text control's size.
    Size(String),
    /// A text control's colour.
    Colour(String),
    /// How long that row's version runs, in seconds.
    Length,
    /// The name of that row's exported file.
    FileName,
}

/// What each of `columns` feeds when nobody has said: the control of the same
/// name, a text control's font, size or colour for a column named after it
/// (`Name size`), and the length and the file name for columns called that.
#[must_use]
pub fn matched(template: &Template, columns: &[String]) -> Vec<Feeds> {
    columns
        .iter()
        .map(|column| {
            let name = data::matched_name(column);
            for control in &template.controls {
                let label = data::matched_name(&control.label);
                if label.is_empty() {
                    continue;
                }
                if name == label {
                    return Feeds::Control(control.id.clone());
                }
                if control.kind != ControlKind::Text {
                    continue;
                }
                let rest = name.strip_prefix(label.as_str());
                match rest {
                    Some("font") if control.font => return Feeds::Font(control.id.clone()),
                    Some("size") if control.size => return Feeds::Size(control.id.clone()),
                    Some("colour" | "color") if control.colour => {
                        return Feeds::Colour(control.id.clone());
                    }
                    _ => {}
                }
            }
            match name.as_str() {
                "length" | "duration" | "seconds" => Feeds::Length,
                "filename" | "file" | "output" => Feeds::FileName,
                _ => Feeds::Nothing,
            }
        })
        .collect()
}

/// One cell that could not be read as what its column feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fault {
    /// The column, counting from 0.
    pub column: usize,
}

/// One row read as what it gives a copy.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowValues {
    /// Each control fed, by id, and its value.
    pub values: Vec<(String, Value)>,
    /// Each text control's font, size or colour fed: the row's id on a copy
    /// and its value.
    pub extras: Vec<(String, Value)>,
    /// Each footage control fed, by id, and the file the cell names. Bringing
    /// the file in and measuring it is the caller's.
    pub files: Vec<(String, String)>,
    /// How long this row's version runs, in seconds.
    pub length: Option<f64>,
    /// The name of this row's exported file.
    pub file_name: Option<String>,
    /// The cells that could not be read. The template's own value stands
    /// there.
    pub faults: Vec<Fault>,
}

/// A display colour's channel as a row keeps it: scene-linear.
fn to_linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn to_display(v: f64) -> f64 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.max(0.0).powf(1.0 / 2.4) - 0.055
    }
}

/// A colour cell as a colour row keeps it.
#[must_use]
pub fn row_colour(cell: &str) -> Option<[f64; 4]> {
    let [r, g, b, a] = data::colour(cell)?;
    Some([to_linear(r), to_linear(g), to_linear(b), a])
}

/// A font cell: the family, or the family and the face parted by a comma.
fn font(cell: &str) -> Value {
    let (family, face) = cell.split_once(',').unwrap_or((cell, ""));
    Value::Font {
        family: family.trim().to_owned(),
        face: face.trim().to_owned(),
    }
}

/// One cell read as the value of `control`. `None` when it cannot be.
fn cell_value(control: &Control, cell: &str) -> Option<Value> {
    Some(match control.kind {
        ControlKind::Text => Value::Text(cell.to_owned()),
        ControlKind::Number | ControlKind::Angle => {
            let v = data::number(cell)?;
            // A range is a wall on a copy, so it is one here.
            Value::Number(match control.range {
                Some((min, max)) => v.clamp(min.min(max), max.max(min)),
                None => v,
            })
        }
        ControlKind::Point => {
            let [x, y, z] = data::point(cell)?;
            Value::Point(if control.axes >= 3 {
                vec![x, y, z]
            } else {
                vec![x, y]
            })
        }
        ControlKind::Colour => Value::Colour(row_colour(cell)?),
        ControlKind::Checkbox => Value::Tick(data::tick(cell)?),
        ControlKind::Menu => Value::Choice(data::option(cell, &control.options)?),
        ControlKind::Font => font(cell),
        // A picture is a file to bring in, which is not a value yet.
        ControlKind::Footage => return None,
    })
}

/// Row `row` of `data` read as what it gives a copy of `template`. An empty
/// cell gives nothing, so the template's own value stands; a cell that cannot
/// be read is a fault and gives nothing either. `None` past the last row.
#[must_use]
pub fn row_values(template: &Template, data: &Data, row: usize) -> Option<RowValues> {
    let cells = data.table.rows.get(row)?;
    let mut out = RowValues::default();
    for (column, cell) in cells.iter().enumerate() {
        let feeds = data.feeds.get(column).unwrap_or(&Feeds::Nothing);
        if cell.trim().is_empty() || *feeds == Feeds::Nothing {
            continue;
        }
        let fault = Fault { column };
        match feeds {
            Feeds::Nothing => {}
            Feeds::Control(id) => {
                let Some(control) = template.control(id) else {
                    continue;
                };
                if control.kind == ControlKind::Footage {
                    out.files.push((id.clone(), cell.trim().to_owned()));
                } else {
                    match cell_value(control, cell) {
                        Some(value) => out.values.push((id.clone(), value)),
                        None => out.faults.push(fault),
                    }
                }
            }
            Feeds::Font(id) => out.extras.push((format!("{id}_font"), font(cell))),
            Feeds::Size(id) => match data::number(cell) {
                Some(v) if v > 0.0 => out.extras.push((format!("{id}_size"), Value::Number(v))),
                _ => out.faults.push(fault),
            },
            Feeds::Colour(id) => match row_colour(cell) {
                Some(c) => out.extras.push((format!("{id}_colour"), Value::Colour(c))),
                None => out.faults.push(fault),
            },
            Feeds::Length => match data::number(cell) {
                Some(v) if v > 0.0 => out.length = Some(v),
                _ => out.faults.push(fault),
            },
            Feeds::FileName => out.file_name = Some(cell.trim().to_owned()),
        }
    }
    Some(out)
}

/// A number as a cell holds it: no more decimals than it needs.
fn number_cell(v: f64) -> String {
    let rounded = (v * 1000.0).round() / 1000.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded}")
    }
}

fn colour_cell(c: [f64; 4]) -> String {
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b) = (
        byte(to_display(c[0])),
        byte(to_display(c[1])),
        byte(to_display(c[2])),
    );
    if c[3] < 1.0 {
        format!("#{r:02X}{g:02X}{b:02X}{:02X}", byte(c[3]))
    } else {
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

/// A value as the cell that would read back as it.
fn value_cell(control: &Control, value: &Value) -> String {
    match value {
        Value::Number(v) => number_cell(*v),
        Value::Point(axes) => axes
            .iter()
            .map(|v| number_cell(*v))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Colour(c) => colour_cell(*c),
        Value::Tick(on) => if *on { "yes" } else { "no" }.to_owned(),
        Value::Choice(n) => control
            .options
            .get(*n as usize)
            .cloned()
            .unwrap_or_else(|| (n + 1).to_string()),
        Value::Text(t) => t.clone(),
        Value::Font { family, face } if face.is_empty() => family.clone(),
        Value::Font { family, face } => format!("{family}, {face}"),
        // A picture's cell is a file's path, which the template does not know.
        Value::Picture(_) => String::new(),
    }
}

/// A table to start a data file from: one column for each control, named as
/// the control is, and one row holding the template's own values. A text
/// control that lets a copy change its font, size or colour has a column for
/// each.
#[must_use]
pub fn starting_table(doc: &Document, comp: &Composition, template: &Template) -> Table {
    let mut columns: Vec<String> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    for control in &template.controls {
        let own = super::rows::read(doc, comp, &control.target);
        columns.push(control.label.clone());
        row.push(own.map(|v| value_cell(control, &v)).unwrap_or_default());
        if control.kind != ControlKind::Text {
            continue;
        }
        let layer = control.target.layer();
        let read = |target: super::Target| super::rows::read(doc, comp, &target);
        if control.font {
            columns.push(format!("{} font", control.label));
            row.push(
                read(super::Target::Font { layer })
                    .map(|v| value_cell(control, &v))
                    .unwrap_or_default(),
            );
        }
        if control.size {
            columns.push(format!("{} size", control.label));
            row.push(
                read(super::Target::TextRow {
                    layer,
                    value: super::TextRow::Size,
                })
                .map(|v| value_cell(control, &v))
                .unwrap_or_default(),
            );
        }
        if control.colour {
            columns.push(format!("{} colour", control.label));
            row.push(
                read(super::Target::TextRow {
                    layer,
                    value: super::TextRow::Fill,
                })
                .map(|v| value_cell(control, &v))
                .unwrap_or_default(),
            );
        }
    }
    Table {
        columns,
        rows: vec![row],
    }
}
