//! An extension editing the open project: one of Lumit's commands, asked for
//! by the page of an extension the person let edit when they installed it.
//!
//! It runs through the same [`AppHost`] as File > Run script and the live
//! endpoint, and keeps to the same rules: never inside a script's run, and
//! never inside a change being made in the window, such as a drag. Each call
//! is one undo step named after the extension. In front of the host sits
//! [`EditHost`], which never saves, exports or says where the project is,
//! whatever the command, and every path in an answer is cut down to the
//! file's name.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lumit_core::model::{Composition, Layer};
use lumit_core::time::CompTime;
use lumit_core::{DocumentStore, Op};
use lumit_script::{commands::COMMANDS, ExportStatus, Host, ScriptError};
use serde_json::Value;
use uuid::Uuid;

use crate::api::extensions::{BridgeExtensionCommand, BridgeExtensionEdit};
use crate::api::project::ProjectReference;
use crate::api::script::BridgeScriptEvent;
use crate::script::AppHost;

/// Commands an extension never runs. Undo and redo would take back the
/// person's own steps, and opening a comp is the Viewer's business.
const REFUSED: &[&str] = &["project.save", "undo", "redo", "comp.open"];

fn refused(command: &str) -> bool {
    REFUSED.contains(&command) || command.starts_with("export.")
}

/// The commands an extension that may edit can run. `import` is whether it
/// may import as well.
pub(crate) fn commands(import: bool) -> Vec<BridgeExtensionCommand> {
    COMMANDS
        .iter()
        .filter(|c| !refused(c.name) && (import || c.name != "footage.import"))
        .map(|c| BridgeExtensionCommand {
            name: c.name.into(),
            summary: c.summary.into(),
            schema: c.schema.into(),
        })
        .collect()
}

fn refusal(code: &str, message: impl Into<String>) -> BridgeExtensionEdit {
    BridgeExtensionEdit::Refused {
        code: code.into(),
        message: message.into(),
    }
}

/// Run `command` with `args`, JSON text, on `project` for the extension
/// called `extension`. `import` is whether it may import files.
pub(crate) fn run(
    project: &ProjectReference,
    extension: &str,
    import: bool,
    command: &str,
    args: &str,
) -> BridgeExtensionEdit {
    if !COMMANDS.iter().any(|c| c.name == command) {
        return refusal("unknown-method", format!("Lumit has no edit.{command}"));
    }
    if refused(command) {
        return refusal("not-allowed", format!("an extension cannot use {command}"));
    }
    if command == "footage.import" && !import {
        return refusal(
            "not-allowed",
            "this extension did not ask for the import permission",
        );
    }
    let Ok(args) = serde_json::from_str::<Value>(args) else {
        return refusal("bad-params", "the arguments are not JSON");
    };
    let store = project
        .state()
        .ok()
        .and_then(|state| Some(state.read().ok()?.store.clone()));
    let Some(store) = store else {
        return refusal("failed", "no project is open");
    };
    // Nobody hears what the host says: comp.open, the one command that
    // speaks, is refused above.
    let quiet = |_: BridgeScriptEvent| false;
    let host = EditHost {
        app: AppHost::new(project.clone(), store, Arc::default(), &quiet),
        import,
    };
    let Some(_held) = crate::script::hold(Arc::default()) else {
        return refusal("failed", crate::live::SCRIPT_RUNNING);
    };
    if !crate::live::drag_over(&host.app) {
        return refusal("failed", crate::live::BUSY);
    }
    let store = host.store();
    let done = lumit_script::one_step(store, || {
        let answer = lumit_script::dispatch(&host, command, args)?;
        if let Some(step) = store.undo_group_name() {
            store.name_undo_group(format!("{extension}: {step}"));
        }
        Ok(answer)
    });
    match done {
        Ok(mut answer) => {
            scrub(&mut answer);
            BridgeExtensionEdit::Done {
                result: answer.to_string(),
            }
        }
        Err(e) => {
            let code = match e {
                ScriptError::BadArgs(_) | ScriptError::NotFound(_) => "bad-params",
                ScriptError::UnknownCommand(_) => "unknown-method",
                ScriptError::Failed(_) | ScriptError::Stopped => "failed",
            };
            refusal(code, e.to_string())
        }
    }
}

/// Whether answer field `key` holds a file's path, such as a footage item's
/// `path`.
fn names_a_file(key: &str) -> bool {
    key.ends_with("path") || key.ends_with("paths") || key == "file" || key == "out"
}

/// `path` cut down to the file's name. Either slash is a separator, as a
/// project made on Windows can be open on a Mac.
fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Cut every path in `answer` down to the file's name, wherever it sits,
/// so an extension never learns where anything is kept on this computer.
fn scrub(answer: &mut Value) {
    match answer {
        Value::Object(fields) => {
            for (key, value) in fields.iter_mut() {
                if !names_a_file(key) {
                    scrub(value);
                    continue;
                }
                match value {
                    Value::String(path) => *path = file_name(path),
                    Value::Array(paths) => {
                        for path in paths.iter_mut() {
                            if let Value::String(text) = path {
                                *text = file_name(text);
                            }
                        }
                    }
                    other => scrub(other),
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(scrub),
        _ => {}
    }
}

/// Lumit's host as an extension has it: the project and nothing around it.
struct EditHost<'a> {
    app: AppHost<'a>,
    import: bool,
}

fn not_for_extensions() -> ScriptError {
    ScriptError::Failed("An extension cannot save or export.".into())
}

impl Host for EditHost<'_> {
    fn store(&self) -> &DocumentStore {
        self.app.store()
    }

    // Where the project is kept is not the extension's to know.
    fn path(&self) -> Option<PathBuf> {
        None
    }

    fn save(&self, _: Option<&Path>) -> Result<PathBuf, ScriptError> {
        Err(not_for_extensions())
    }

    fn import_footage(&self, paths: &[PathBuf]) -> Result<Vec<Uuid>, ScriptError> {
        if !self.import {
            return Err(ScriptError::Failed(
                "This extension cannot import files.".into(),
            ));
        }
        self.app.import_footage(paths)
    }

    fn footage_layers(&self, comp: &Composition, item: Uuid) -> Result<Vec<Layer>, ScriptError> {
        self.app.footage_layers(comp, item)
    }

    fn footage_info(&self, item: Uuid) -> Option<lumit_script::FootageInfo> {
        self.app.footage_info(item)
    }

    fn export_start(&self, _: Uuid, _: Option<&Path>) -> Result<u64, ScriptError> {
        Err(not_for_extensions())
    }

    fn export_status(&self, _: u64) -> Result<ExportStatus, ScriptError> {
        Err(not_for_extensions())
    }

    fn export_cancel(&self, _: u64) -> Result<(), ScriptError> {
        Err(not_for_extensions())
    }

    fn parent_ops(&self, comp: Uuid, layer: Uuid, parent: Option<Uuid>, at: CompTime) -> Vec<Op> {
        self.app.parent_ops(comp, layer, parent, at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::state::LumitBridgeState;

    // An extension never saves the project or exports it, by the command's
    // name or through the host behind it, and never learns where a file is.
    #[test]
    #[allow(clippy::expect_used, clippy::panic)]
    fn an_extension_cannot_save_export_or_see_paths() {
        let project = LumitBridgeState::new_project(None).expect("a new project");
        let dir = tempfile::tempdir().expect("temp dir");
        let to = dir.path().join("taken.lum");
        let args = serde_json::json!({ "path": to }).to_string();
        for command in ["project.save", "export.start"] {
            let done = run(&project, "Test", true, command, &args);
            assert!(
                matches!(&done, BridgeExtensionEdit::Refused { code, .. } if code == "not-allowed"),
                "{command}: {done:?}"
            );
        }
        let store = project
            .state()
            .expect("state")
            .read()
            .expect("read")
            .store
            .clone();
        let quiet = |_: BridgeScriptEvent| false;
        let host = EditHost {
            app: AppHost::new(project.clone(), store, Arc::default(), &quiet),
            import: true,
        };
        assert!(host.save(Some(&to)).is_err());
        assert!(host.export_start(Uuid::nil(), Some(&to)).is_err());
        assert!(!to.exists());

        project
            .import_footage("C:/Users/Someone/Footage/clip.mp4".into())
            .expect("imported");
        let BridgeExtensionEdit::Done { result } = run(&project, "Test", false, "item.list", "{}")
        else {
            panic!("item.list was refused");
        };
        assert!(result.contains(r#""path":"clip.mp4""#), "{result}");
        assert!(!result.contains("Someone"), "{result}");
    }
}
