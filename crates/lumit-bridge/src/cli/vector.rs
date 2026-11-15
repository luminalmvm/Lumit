//! `lumit-cli export --format lottie|dotlottie|svg`: comps written as vector
//! files through the same export the Lottie and SVG window runs, with what
//! won't come across told on standard error, a line per thing.

use std::path::{Path, PathBuf};

use lumit_lottie::settings::{Format, Settings, TextMode};
use lumit_script::Host;

use super::Export;

/// The vector format `how` asks for: named with `--format`, or given by an
/// `--out` file ending `.json`, `.lottie` or `.svg`.
pub(super) fn format_of(how: &Export) -> Option<Format> {
    if let Some(key) = &how.format {
        return Format::from_key(key);
    }
    let ext = how
        .out
        .as_ref()?
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    match ext.as_str() {
        "json" => Some(Format::Lottie),
        "lottie" => Some(Format::DotLottie),
        "svg" => Some(Format::Svg),
        _ => None,
    }
}

pub(super) fn export(project: &Path, how: &Export, format: Format) -> Result<(), String> {
    let host = super::open(project)?;
    let comps = super::chosen_comps(&host, how)?;
    let doc = host.store().snapshot();
    let project_path = host.path();
    let beside = project_path
        .as_ref()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let folder = match &how.out {
        Some(out) if comps.len() > 1 => {
            if !out.is_dir() {
                return Err(format!(
                    "With several comps, --out is the folder they go in, and {} is not one.",
                    out.display()
                ));
            }
            std::path::absolute(out).unwrap_or_else(|_| out.clone())
        }
        _ => beside,
    };
    let mut failed = 0;
    for (comp, name) in &comps {
        let file = match &how.out {
            Some(out) if comps.len() == 1 => {
                let out = std::path::absolute(out).unwrap_or_else(|_| out.clone());
                out.with_extension(format.extension())
            }
            _ => free_name(&folder, name, format),
        };
        let settings = Settings {
            format,
            text: if how.text_as_text {
                TextMode::Text
            } else {
                TextMode::Shapes
            },
            minify: how.minify,
            file_stem: file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "animation".into()),
            ..Settings::default()
        };
        match crate::vector_host::write_to(&doc, project_path.as_deref(), *comp, &settings, &file) {
            Ok(issues) => {
                for line in issues {
                    note!("{name}: {line}");
                }
                out!("{}", file.display());
            }
            Err(why) if comps.len() == 1 => return Err(why),
            Err(why) => {
                note!("{name}: {why}");
                failed += 1;
            }
        }
    }
    match failed {
        0 => Ok(()),
        n => Err(format!("{n} of {} exports failed.", comps.len())),
    }
}

/// `name.ext` in `dir`, or `name_2.ext` and on when that is taken.
fn free_name(dir: &Path, name: &str, format: Format) -> PathBuf {
    let ext = format.extension();
    let stem: String = name
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .collect();
    let stem = if stem.trim().is_empty() {
        "animation".to_owned()
    } else {
        stem.trim().to_owned()
    };
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..10_000)
        .map(|n| dir.join(format!("{stem}_{n}.{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}
