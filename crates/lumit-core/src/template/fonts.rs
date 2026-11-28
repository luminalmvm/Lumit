//! The fonts a template draws with, so its file can carry them and a machine
//! without them can say which are missing.
//!
//! Runs wherever the document is read.

use uuid::Uuid;

use super::{Template, Value};
use crate::fx::effects::template as values;
use crate::model::{Document, LayerKind};

#[must_use]
pub fn used(doc: &Document, comp: Uuid) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    let mut add = |family: &str, face: &str| {
        let font = (family.to_owned(), face.to_owned());
        if !family.is_empty() && !found.contains(&font) {
            found.push(font);
        }
    };
    let mut seen: Vec<Uuid> = Vec::new();
    let mut queue = vec![comp];
    while let Some(id) = queue.pop() {
        let Some(c) = doc.comp(id).filter(|_| !seen.contains(&id)) else {
            continue;
        };
        seen.push(id);
        for layer in &c.layers {
            if let LayerKind::Text { document } = &layer.kind {
                add(&document.style.family, &document.style.face);
            }
        }
        for inst in c.effects().filter(|inst| values::comp_of(inst).is_some()) {
            let template = Template {
                controls: values::held(inst).controls,
                ..Template::default()
            };
            for setting in values::settings_at(inst, &template, 0.0, None, None) {
                if let Value::Font { family, face } = &setting.value {
                    add(family, face);
                }
            }
        }
        if let Some(template) = c.template() {
            let settings = template
                .controls
                .iter()
                .flat_map(|c| c.looks.iter().flatten());
            for setting in settings {
                if let Value::Font { family, face } = &setting.value {
                    add(family, face);
                }
            }
        }
        // Reversed, so the compositions are walked in the order they are met.
        queue.extend(crate::edits::nested_comps(doc, c).into_iter().rev());
    }
    found
}
