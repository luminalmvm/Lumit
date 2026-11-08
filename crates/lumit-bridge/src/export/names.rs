//! The suggested file name an export starts with: a preset's own, or the
//! filename template with its tokens filled in.

use super::preset_default_file_name;

/// The suggested file name for `preset` — a faithful port of
/// `shell::export_default_file_name`: with no (or a blank) template, the preset's
/// own default file name byte-for-byte; otherwise the template with `{comp}`/
/// `{preset}`/`{date}` substituted, sanitised, and forced to end in `.mp4`.
pub(super) fn export_default_file_name(
    preset: &str,
    comp_name: &str,
    template: Option<&str>,
) -> String {
    match template.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => {
            let stem = preset_default_file_name(preset).trim_end_matches(".mp4");
            render_filename_template(t, comp_name, stem)
        }
        None => preset_default_file_name(preset).to_string(),
    }
}

/// Substitute `{comp}`/`{preset}`/`{date}` in a filename template,
/// sanitise against characters Windows forbids, and guarantee a `.mp4` suffix —
/// a faithful port of `shell::render_filename_template`.
fn render_filename_template(template: &str, comp_name: &str, preset_stem: &str) -> String {
    let date = today_utc_date();
    let substituted = template
        .replace("{comp}", comp_name)
        .replace("{preset}", preset_stem)
        .replace("{date}", &date);
    let mut name = sanitise_windows_filename(&substituted);
    if !name.to_ascii_lowercase().ends_with(".mp4") {
        name.push_str(".mp4");
    }
    name
}

/// Replace characters illegal in a Windows file name (and control characters)
/// with `_`, falling back to `export` if nothing usable remains — a faithful
/// port of `shell::sanitise_windows_filename`.
fn sanitise_windows_filename(raw: &str) -> String {
    const ILLEGAL: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if ILLEGAL.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "export".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Today's UTC date as `YYYY-MM-DD` (the `{date}` token) — a faithful port
/// of `shell::today_utc_date` and its `civil_from_days`.
fn today_utc_date() -> String {
    let secs = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since the Unix epoch → (year, month, day), proleptic Gregorian
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
