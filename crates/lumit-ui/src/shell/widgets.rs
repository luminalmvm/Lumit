//! `shell::widgets` — split out of the monolithic shell.rs (mechanical,
//! no logic changes). Shared names resolve through the parent module
//! via `use super::*` and the glob re-exports in `shell/mod.rs`.

use super::*;

/// A text field that mirrors a model value when idle and holds the user's
/// keystrokes while focused (so partial entry like "29.99" isn't clobbered
/// mid-type). Returns the response plus the buffer as it stands this frame;
/// the caller parses on `lost_focus()`.
pub(crate) fn text_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    model_str: &str,
    width: f32,
) -> (egui::Response, String) {
    let mut buf = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| model_str.to_owned());
    let resp = ui.add(
        egui::TextEdit::singleline(&mut buf)
            .id(id)
            .desired_width(width),
    );
    // While editing, remember the keystrokes; otherwise mirror the model so
    // an external change (e.g. the ratio lock) shows immediately.
    let keep = if resp.has_focus() {
        buf.clone()
    } else {
        model_str.to_owned()
    };
    ui.data_mut(|d| d.insert_temp(id, keep));
    (resp, buf)
}

/// The time-grid step (seconds) for a zoom level: the largest step from the
/// editing-friendly ladder that keeps gridlines at least ~70 px apart, down
/// to 10 ms when zoomed right in.
pub(crate) fn time_grid_step(px_per_sec: f64) -> f64 {
    const LADDER: [f64; 12] = [
        0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 300.0,
    ];
    for step in LADDER {
        if step * px_per_sec >= 70.0 {
            return step;
        }
    }
    600.0
}

/// Short label for a speed-ramp ease.
pub(crate) fn ease_label(e: lumit_core::retime::Ease) -> &'static str {
    use lumit_core::retime::Ease;
    match e {
        Ease::Linear => "Linear",
        Ease::Slow => "Slow",
        Ease::Fast => "Fast",
        Ease::Smooth => "Smooth",
        Ease::Sharp => "Sharp",
    }
}

/// A dropdown that shows just its label — no down-caret (the house style).
/// Returns whatever the menu closure produces.
pub(crate) fn bare_dropdown<R>(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    ui.menu_button(label, add).inner
}

/// `HH:MM:SS:mmm` from seconds (docs: composition duration display).
pub(crate) fn fmt_duration(secs: f64) -> String {
    let total_ms = (secs.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let s = (total_ms / 1000) % 60;
    let m = (total_ms / 60_000) % 60;
    let h = total_ms / 3_600_000;
    format!("{h:02}:{m:02}:{s:02}:{ms:03}")
}

/// `HH:MM:SS:FF` frame timecode from seconds at `fps` — the Retime value lens
/// reading (K-075: "which source frame is showing here"). The frame field wraps
/// at `fps`; a whole extra second is carried up so `59:24`→`00`+1s at 25 fps.
/// The frame field is padded to the digit-width of `fps` (min two), so a 600 fps
/// clip reads `…:599` and a 1000 fps clip `…:0999`, whatever the comp's own rate.
pub(crate) fn fmt_timecode_frames(secs: f64, fps: f64) -> String {
    let fps_i = fps.round().max(1.0) as u64;
    let total_frames = (secs.max(0.0) * fps).round() as u64;
    let ff = total_frames % fps_i;
    let total_s = total_frames / fps_i;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    let width = fps_i.to_string().len().max(2);
    format!("{h:02}:{m:02}:{s:02}:{ff:0width$}")
}

/// Parse a source frame count from a timecode the user typed into the Retime
/// value lens: `HH:MM:SS:FF`, any shorter colon form (`MM:SS:FF`, `SS:FF`), or a
/// bare frame number. None when it does not parse — egui then keeps the old
/// value. The frame field is read at `fps`, matching [`fmt_timecode_frames`].
pub(crate) fn parse_timecode_frames(s: &str, fps: f64) -> Option<f64> {
    let fps_i = fps.round().max(1.0) as u64;
    let nums: Vec<u64> = s
        .trim()
        .split(':')
        .map(|p| p.trim().parse::<u64>())
        .collect::<Result<_, _>>()
        .ok()?;
    let (h, m, sec, ff) = match nums.as_slice() {
        [ff] => (0, 0, 0, *ff),
        [sec, ff] => (0, 0, *sec, *ff),
        [m, sec, ff] => (0, *m, *sec, *ff),
        [h, m, sec, ff] => (*h, *m, *sec, *ff),
        _ => return None,
    };
    Some(((h * 3600 + m * 60 + sec) * fps_i + ff) as f64)
}

/// The (in, out, start_offset) after moving a layer by `delta` comp seconds: all
/// three shift together, so the bar and its content move as one. Shifting the
/// span without `start_offset` would *slip* the content instead of moving it.
/// Sign is preserved (K-153): the moved in point may land before comp time 0 and
/// the out point past the comp duration; only the [0, comp_end) overlap renders.
pub(crate) fn moved_span(
    in_point: lumit_core::time::CompTime,
    out_point: lumit_core::time::CompTime,
    start_offset: lumit_core::time::CompTime,
    delta: f64,
) -> (
    lumit_core::time::CompTime,
    lumit_core::time::CompTime,
    lumit_core::time::CompTime,
) {
    let shift = |t: lumit_core::time::CompTime| {
        lumit_core::time::CompTime(rational_at_signed(t.0.to_f64() + delta))
    };
    (shift(in_point), shift(out_point), shift(start_offset))
}

/// The lane-area horizontal view (07-UI-SPEC §4): pixels-per-second and the
/// clamped left-edge comp time, from a zoom (1.0 = the whole comp fits `track_w`;
/// larger zooms in) and a desired left time. The view never scrolls past the
/// comp ends, so at zoom 1 it always shows the whole comp from 0.
pub(crate) fn lane_view(track_w: f32, duration: f64, zoom: f64, view_start: f64) -> (f64, f64) {
    let zoom = zoom.clamp(1.0, 400.0);
    let px_per_sec = track_w as f64 * zoom / duration.max(1e-6);
    let visible = duration / zoom;
    let start = view_start.clamp(0.0, (duration - visible).max(0.0));
    (px_per_sec, start)
}

/// A horizontal pixel distance in the lane area, as seconds — at the *displayed*
/// zoom. Every lane drag and snap tolerance must convert through this: the naive
/// `dx / track_w * duration` is only right at zoom 1, and makes a drag run
/// `zoom×` faster than the cursor once zoomed in.
pub(crate) fn drag_secs(dx_px: f64, px_per_sec: f64) -> f64 {
    dx_px / px_per_sec.max(1e-6)
}

/// Parse a flexible duration: `SS(.sss)`, `MM:SS`, `HH:MM:SS`, or
/// `HH:MM:SS:mmm`. None on anything unparseable.
pub(crate) fn parse_duration(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let parts: Vec<&str> = t.split(':').collect();
    let (h, m, s, ms) = match parts.as_slice() {
        [s] => (0.0, 0.0, s.parse::<f64>().ok()?, 0.0),
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?, 0.0),
        [h, m, s] => (
            h.parse::<f64>().ok()?,
            m.parse::<f64>().ok()?,
            s.parse::<f64>().ok()?,
            0.0,
        ),
        [h, m, s, ms] => (
            h.parse::<f64>().ok()?,
            m.parse::<f64>().ok()?,
            s.parse::<f64>().ok()?,
            ms.parse::<f64>().ok()?,
        ),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + s + ms / 1000.0)
}

/// Simplify a width:height pair for display (e.g. 1920×1080 → 16:9).
pub(crate) fn aspect_ratio_label(w: u32, h: u32) -> String {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    let g = gcd(w.max(1), h.max(1)).max(1);
    format!("{}:{}", w / g, h / g)
}
