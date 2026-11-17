//! `lumit-cli export-job <folder>`: a remote export another computer sent,
//! run apart from the Lumit that took it (`remote`), which starts this.
//!
//! The folder holds the job's document, its files laid out under `media/`,
//! its simulation bakes under `cache/`, and `job.json`, which says what to
//! export, how and where, and where each of the document's files is here.
//!
//! How far it has got goes on standard output one JSON line at a time, at
//! most ten a second: `{"frame":n,"total":t,"encoder":"..."}`, then
//! `{"done":{"seconds":s,"bytes":b}}` or `{"failed":"<words>"}`. It hears
//! `pause`, `go` and `stop` on standard input, a line each.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver};
use std::time::{Duration, Instant};

use lumit_core::model::{packed_id, ProjectItem};
use lumit_script::{ExportState, Host};
use serde_json::{json, Value};
use uuid::Uuid;

use super::host::Headless;

/// The fewest seconds between two progress lines.
const EVERY: Duration = Duration::from_millis(100);

pub(super) fn export_job(folder: &Path) -> Result<(), String> {
    let failed = |why: String| {
        out!("{}", json!({ "failed": why }));
        why
    };
    let order = std::fs::read(folder.join("job.json"))
        .map_err(|e| failed(format!("The job could not be read: {e}.")))?;
    let order: Value = serde_json::from_slice(&order)
        .map_err(|e| failed(format!("The job could not be read: {e}.")))?;
    let text = |name: &str| order.get(name).and_then(Value::as_str).unwrap_or_default();
    let comp =
        Uuid::try_parse(text("comp")).map_err(|_| failed("The job names no comp.".into()))?;
    let spec: lumit_render::export::ExportSpec =
        serde_json::from_value(order.get("spec").cloned().unwrap_or(Value::Null))
            .map_err(|e| failed(format!("The job's export settings could not be read: {e}.")))?;
    let out = PathBuf::from(text("out"));
    let mut doc = crate::remote::job_document(folder, text("schema"))
        .ok_or_else(|| failed("This Lumit can't read the job's project.".into()))?;
    // Each file is read from where it is on this computer, and from nowhere
    // else: every other path the document carries is cleared.
    let references: HashMap<Uuid, String> = order
        .get("references")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(reference, path)| Some((reference.parse().ok()?, path.as_str()?.to_owned())))
        .collect();
    let footage = doc.items.iter().filter_map(|item| match item {
        ProjectItem::Footage(f) => Some(f.id),
        _ => None,
    });
    let packed: Vec<Uuid> = footage
        .chain(doc.proxies.keys().map(|item| packed_id(*item)))
        .chain(std::iter::once(packed_id(doc.id)))
        .chain(doc.effect_files().map(|(id, _)| id))
        .collect();
    for id in packed {
        let path = references.get(&id).cloned().unwrap_or_default();
        if let Some(reference) = doc.packed_ref_mut(id) {
            if path.is_empty() {
                reference.relative_path.clear();
            }
            reference.absolute_path = path;
        } else if let Some(slot) = doc.packed_path_mut(id) {
            *slot = path;
        }
    }
    let cache = PathBuf::from(text("cache"));
    lumit_render::sim::bake::set_root(Some(cache));
    lumit_render::sent::warm(&doc);
    let host = Headless::from_document(doc);
    // Extension effects draw in an export as they do in Lumit.
    crate::api::extensions::register_effects();
    let job = host
        .start_export(comp, out, spec)
        .map_err(|e| failed(e.to_string()))?;
    let heard = listen();
    let started = Instant::now();
    let mut said = (Instant::now() - EVERY, u64::MAX);
    loop {
        for line in heard.try_iter() {
            match line.trim() {
                "pause" => host.export_hold(job, true),
                "go" => host.export_hold(job, false),
                "stop" => {
                    let _ = host.export_cancel(job);
                }
                _ => {}
            }
        }
        let status = host.export_status(job).map_err(|e| failed(e.to_string()))?;
        match status.state {
            ExportState::Running => {
                if said.1 != status.done && said.0.elapsed() >= EVERY {
                    said = (Instant::now(), status.done);
                    let encoder = host.export_encoder(job).unwrap_or_default();
                    out!(
                        "{}",
                        json!({ "frame": status.done, "total": status.total, "encoder": encoder })
                    );
                }
            }
            ExportState::Done => {
                let bytes = std::fs::metadata(&status.path).map_or(0, |m| m.len());
                let seconds = started.elapsed().as_secs();
                out!(
                    "{}",
                    json!({ "done": { "seconds": seconds, "bytes": bytes } })
                );
                return Ok(());
            }
            ExportState::Cancelled => return Err(failed("cancelled".into())),
            ExportState::Failed => {
                let why = status.message.unwrap_or_default();
                return Err(failed(format!("The export failed: {why}")));
            }
        }
    }
}

/// What is said on standard input, a line at a time. Its end is a stop: the
/// Lumit that started this has gone.
fn listen() -> Receiver<String> {
    let (tell, heard) = sync_channel(16);
    let _ = std::thread::Builder::new()
        .name("lumit-cli-job-input".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if tell.send(line).is_err() {
                    return;
                }
            }
            let _ = tell.send("stop".into());
        });
    heard
}
