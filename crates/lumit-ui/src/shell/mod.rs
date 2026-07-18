//! The application shell: menu bar, docked panels, status line.
//!
//! Layout per docs/07-UI-SPEC.md (Edit workspace): Project left, Viewer centre,
//! Effect Controls / Effects & Presets right, Timeline across the bottom.

pub(crate) use crate::app_state::{
    AppState, EyedropperMode, EyedropperTarget, ShapeKind, ToolMode,
};
pub(crate) use crate::icons::Icon;
pub(crate) use crate::splash::{BootLine, Splash};
pub(crate) use crate::theme::Theme;
pub(crate) use lumit_core::model::ProjectItem;
pub(crate) use serde::{Deserialize, Serialize};

mod app_update;
mod command_palette;
mod dialogs;
mod dock;
mod draws;
#[cfg(feature = "media")]
mod export_actions;
mod eyedropper;
mod gpu;
mod graph;
mod hierarchy;
mod inspector;
mod overlays;
mod panels;
mod scopes;
mod settings;
mod shortcuts;
mod timeline;
mod widgets;

pub(crate) use command_palette::*;
pub(crate) use dock::*;
pub(crate) use draws::*;
pub(crate) use gpu::*;
pub(crate) use graph::*;
pub(crate) use hierarchy::*;
pub(crate) use inspector::*;
pub(crate) use overlays::*;
pub(crate) use panels::*;
pub(crate) use scopes::*;
pub(crate) use timeline::*;
pub(crate) use widgets::*;

/// The dockable panels. Names are glossary names (docs/01-GLOSSARY.md §7).
/// A dockable panel (a pane in the tiling tree). A panel that sits alone shows
/// no tab bar at all — the Viewer's bare look (K-074, Mack: the viewport must
/// have no top bit) extended to every solo pane (K-086). A tab bar appears
/// only where panels are stacked into a tab group, and those tabs can be
/// dragged to re-arrange the workspace.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Panel {
    Project,
    Viewer,
    Timeline,
    EffectControls,
    EffectsAndPresets,
    Scopes(ScopeKind),
    Hierarchy,
}

impl Panel {
    fn title(&self) -> &'static str {
        match self {
            Panel::Project => "Project",
            Panel::Viewer => "Viewer",
            Panel::Timeline => "Timeline",
            Panel::EffectControls => "Effect controls",
            Panel::EffectsAndPresets => "Effects & presets",
            Panel::Scopes(_) => "Scopes",
            Panel::Hierarchy => "Hierarchy",
        }
    }
}

/// Persisted UI state (the dockable-panel layout; app state is runtime).
#[derive(Serialize, Deserialize)]
pub struct Shell {
    /// The tiling layout: which panels sit where, and their sizes.
    #[serde(default = "default_layout")]
    dock: egui_tiles::Tree<Panel>,
    /// Panels currently detached into their own OS windows. Hidden in the dock
    /// while floating; closing the window docks them back.
    #[serde(default)]
    floating: Vec<Panel>,
    #[serde(skip, default)]
    theme: Theme,
    /// Legacy background-ramp pick (K-092), superseded by `color_scheme`
    /// (K-097). Kept only to migrate an older save on load, and never
    /// written back (`skip_serializing`), so a new save carries only
    /// `color_scheme`.
    #[serde(default, skip_serializing)]
    theme_variant: crate::theme::ThemeVariant,
    /// A custom accent colour, when picked (Settings → Appearance). None =
    /// the clay default. Persisted with the workspace.
    #[serde(default)]
    accent_override: Option<[u8; 3]>,
    /// Legacy light/dark pick (K-092), superseded by `color_scheme` (K-097).
    /// Migration-only, like `theme_variant`.
    #[serde(default, skip_serializing)]
    theme_mode: crate::theme::ThemeMode,
    /// The colour scheme (K-097): Dark, Dark blue, Light, Gruvbox dark/light,
    /// Catppuccin Mocha/Latte. Supersedes the old mode × variant pair, whose
    /// saved value is migrated into this on first load. Persisted.
    #[serde(default)]
    color_scheme: crate::theme::ColorScheme,
    /// Sharp (edge-to-edge) or Round (floating card) panel geometry (Window
    /// menu, K-092). Persisted with the workspace.
    #[serde(default)]
    theme_shape: crate::theme::ThemeShape,
    /// How much UI-chrome motion to show (Settings → Appearance, K-092).
    /// Persisted with the workspace.
    #[serde(default)]
    animation_level: crate::theme::AnimationLevel,
    /// Application-wide performance settings (Settings → Performance): frame
    /// cache budgets and GPU acceleration. Persisted with the workspace.
    #[serde(default)]
    settings: settings::PerformanceSettings,
    /// Autosave settings (Settings → General). Persisted with the workspace.
    #[serde(default)]
    autosave: settings::AutosaveSettings,
    /// Interface settings (Settings → Interface): UI scale and whether hover
    /// tooltips show. Persisted with the workspace.
    #[serde(default)]
    interface: settings::InterfaceSettings,
    /// Export settings (Settings → Export, K-119): the default preset a
    /// generic "Export…" action stamps, and an optional filename template.
    /// Persisted with the workspace. Gated on the `media` feature like every
    /// other export concept.
    #[cfg(feature = "media")]
    #[serde(default)]
    settings_export: settings::ExportSettings,
    /// Whether the Settings window is open (runtime only).
    #[serde(skip, default)]
    settings_open: bool,
    /// Which Settings page is showing (runtime only).
    #[serde(skip, default)]
    settings_page: settings::SettingsPage,
    /// Command palette (Ctrl/Cmd+P) state — all runtime only.
    #[serde(skip, default)]
    palette_open: bool,
    #[serde(skip, default)]
    palette_query: String,
    #[serde(skip, default)]
    palette_sel: usize,
    /// Set on open so the search field grabs focus for one frame.
    #[serde(skip, default)]
    palette_focus: bool,
    /// The panel that last took a click — it wears the accent boundary so the
    /// keyboard's home is always visible (AE's focused-panel edge).
    #[serde(skip, default)]
    active_panel: Option<Panel>,
    #[serde(skip, default)]
    app: AppState,
    /// Boot splash (K-008); None once the application window has expanded.
    #[serde(skip, default)]
    splash: Option<Splash>,
    /// Current Viewer frame texture (uploaded on the UI thread from
    /// background-decoded pixels; a memcpy, not a decode — K-017 holds).
    #[serde(skip, default)]
    preview_tex: Option<egui::TextureHandle>,
    /// What the Viewer paints: id + pixel size (GPU path or CPU fallback).
    #[serde(skip, default)]
    preview_display: Option<(egui::TextureId, egui::Vec2)>,
    #[cfg(feature = "media")]
    #[serde(skip, default)]
    gpu: Option<GpuViewer>,
    /// The last presented comp frame (its decoded per-layer pixels), retained
    /// so a value drag can re-composite live from it with the provisional
    /// value patched in — transform edits change geometry only, never which
    /// footage frame each layer shows, so no re-decode is needed.
    #[cfg(feature = "media")]
    #[serde(skip, default)]
    last_comp: Option<crate::app_state::preview::CompFrame>,
    #[serde(skip, default)]
    last_doc_ptr: usize,
    #[cfg(feature = "media")]
    #[serde(skip, default)]
    export: Option<crate::export::ExportHandle>,
    #[cfg(feature = "media")]
    #[serde(skip, default)]
    export_progress: Option<(usize, usize)>,
    /// Which encoder the running export settled on ("NVENC", …).
    #[cfg(feature = "media")]
    #[serde(skip, default)]
    export_encoder: Option<&'static str>,
    /// File name of the running export, for the status line.
    #[cfg(feature = "media")]
    #[serde(skip, default)]
    export_name: Option<String>,
    /// Exports waiting their turn; each was snapshotted when queued
    /// (docs/06 §7.1) and starts when the running one finishes.
    #[cfg(feature = "media")]
    #[serde(skip, default)]
    export_queue: std::collections::VecDeque<crate::export::QueuedExport>,
    #[cfg(feature = "media")]
    #[serde(skip, default)]
    export_dialog: Option<ExportDialogState>,
    /// Native macOS menu bar; None on other platforms (07-UI-SPEC).
    #[cfg(target_os = "macos")]
    #[serde(skip, default)]
    native_menu: Option<crate::native_menu::NativeMenu>,
}

/// The export dialogue's working state: a preset stamp plus freely editable
/// fields (the custom path). Confirming queues the export.
#[cfg(feature = "media")]
struct ExportDialogState {
    comp_id: uuid::Uuid,
    comp_size: (u32, u32),
    /// The comp's own name, for the `{comp}` filename-template token
    /// (K-119). Snapshotted once at open time, like `comp_size`.
    comp_name: String,
    /// Settings → Export's filename template, snapshotted once at open time
    /// (K-119). `None`/blank = today's behaviour: each preset's own default
    /// file name, untouched.
    filename_template: Option<String>,
    /// The last preset applied (display only; the fields below are truth).
    preset: crate::export::ExportPreset,
    /// What that preset stamped, kept to preserve its VBR peak while the
    /// stamped numbers stand unedited.
    stamped: Option<crate::export::PresetParams>,
    codec: lumit_media::encode::VideoCodec,
    /// None = the comp's own size; Some = a delivery frame.
    size: Option<(u32, u32)>,
    /// Average bitrate in Mbps as typed; empty = encoder default quality.
    bitrate_mbps: String,
    include_audio: bool,
    default_name: String,
}

#[cfg(feature = "media")]
impl ExportDialogState {
    /// Stamp a preset's parameters over the editable fields.
    fn apply(&mut self, preset: crate::export::ExportPreset) {
        self.preset = preset;
        self.stamped = preset.params();
        self.default_name =
            export_default_file_name(preset, &self.comp_name, self.filename_template.as_deref());
        match self.stamped {
            Some(p) => {
                self.codec = p.codec;
                self.size = Some(p.size);
                self.bitrate_mbps = (p.target_bps / 1_000_000).to_string();
            }
            None => {
                self.size = None;
                self.bitrate_mbps.clear();
            }
        }
    }

    /// Resolve the fields into the spec one queued export runs with.
    fn spec(&self) -> crate::export::ExportSpec {
        let target = self.size.unwrap_or(self.comp_size);
        let bit_rate = self
            .bitrate_mbps
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|m| *m > 0.0)
            .map(|m| (m * 1_000_000.0) as i64);
        // The preset's peak applies while its numbers stand unedited;
        // an edited bitrate gets the customary 1.5× peak instead.
        let max_rate = match (self.stamped, bit_rate) {
            (Some(p), Some(b)) if b == p.target_bps && self.codec == p.codec => Some(p.peak_bps),
            (_, Some(b)) => Some(b.saturating_mul(3) / 2),
            (_, None) => None,
        };
        crate::export::ExportSpec {
            codec: self.codec,
            target,
            bit_rate,
            max_rate,
            include_audio: self.include_audio,
            audio_bit_rate: crate::export::PRESET_AUDIO_BPS,
        }
    }
}

/// The export dialogue's suggested file name for `preset` (K-119, Settings →
/// Export → Filename template). `template` is `self.settings_export
/// .filename_template`, snapshotted onto the dialogue at open time. `None`,
/// or a template that's blank once trimmed, must reproduce
/// `preset.default_file_name()` byte-for-byte — this is load-bearing: an
/// existing install's suggested names must not shift under it just because
/// the setting now exists.
#[cfg(feature = "media")]
fn export_default_file_name(
    preset: crate::export::ExportPreset,
    comp_name: &str,
    template: Option<&str>,
) -> String {
    match template.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => {
            let stem = preset.default_file_name().trim_end_matches(".mp4");
            render_filename_template(t, comp_name, stem)
        }
        None => preset.default_file_name().to_string(),
    }
}

/// Substitute `{comp}`, `{preset}`, and `{date}` in a filename template
/// (K-119), sanitise the result against characters Windows forbids in file
/// names, and guarantee it ends in `.mp4`. `comp_name` is free text (a
/// composition name), so it — not just the template — can carry an illegal
/// character; both go through the same sanitising pass.
#[cfg(feature = "media")]
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

/// Replace characters illegal in a Windows file name (`< > : " / \ | ? *`,
/// and control characters) with `_`, and fall back to `export` if nothing
/// but such characters (or leading/trailing whitespace) is left.
#[cfg(feature = "media")]
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

/// Today's UTC date as `YYYY-MM-DD` (K-119's `{date}` filename-template
/// token). Hand-rolled rather than a new dependency: `SystemTime` for the
/// clock, then Howard Hinnant's `civil_from_days` to turn a day count since
/// the Unix epoch into a calendar date — the standard, widely-used
/// constant-time algorithm for this conversion (no leap-year branching to
/// get wrong). A clock before 1970 (never in practice) degrades to the
/// epoch date rather than panicking.
#[cfg(feature = "media")]
fn today_utc_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since the Unix epoch → (year, month, day), proleptic Gregorian.
/// Howard Hinnant's `civil_from_days`: <https://howardhinnant.github.io/date_algorithms.html>.
#[cfg(feature = "media")]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Reconcile the persisted colour scheme with a pre-K-097 save's legacy
/// mode × variant pair. A newer save carries `color_scheme` directly (the
/// legacy fields are no longer written, so they deserialize to their
/// defaults) and this returns it untouched; only when `color_scheme` is still
/// its default is the legacy pair consulted, so an older Light or Dark-blue
/// pick survives the upgrade.
fn migrated_scheme(
    current: crate::theme::ColorScheme,
    mode: crate::theme::ThemeMode,
    variant: crate::theme::ThemeVariant,
) -> crate::theme::ColorScheme {
    use crate::theme::{ColorScheme, ThemeMode, ThemeVariant};
    if current != ColorScheme::default() {
        return current;
    }
    match (mode, variant) {
        (ThemeMode::Light, _) => ColorScheme::Light,
        (ThemeMode::Dark, ThemeVariant::DarkBlue) => ColorScheme::DarkBlue,
        _ => ColorScheme::Dark,
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            dock: default_layout(),
            floating: Vec::new(),
            theme: Theme::dark(),
            theme_variant: crate::theme::ThemeVariant::default(),
            accent_override: None,
            theme_mode: crate::theme::ThemeMode::default(),
            color_scheme: crate::theme::ColorScheme::default(),
            theme_shape: crate::theme::ThemeShape::default(),
            animation_level: crate::theme::AnimationLevel::default(),
            settings: settings::PerformanceSettings::default(),
            autosave: settings::AutosaveSettings::default(),
            interface: settings::InterfaceSettings::default(),
            #[cfg(feature = "media")]
            settings_export: settings::ExportSettings::default(),
            settings_open: false,
            settings_page: settings::SettingsPage::default(),
            palette_open: false,
            palette_query: String::new(),
            palette_sel: 0,
            palette_focus: false,
            active_panel: None,
            app: AppState::default(),
            splash: None,
            preview_tex: None,
            preview_display: None,
            #[cfg(feature = "media")]
            gpu: None,
            #[cfg(feature = "media")]
            last_comp: None,
            last_doc_ptr: 0,
            #[cfg(feature = "media")]
            export: None,
            #[cfg(feature = "media")]
            export_progress: None,
            #[cfg(feature = "media")]
            export_encoder: None,
            #[cfg(feature = "media")]
            export_name: None,
            #[cfg(feature = "media")]
            export_queue: std::collections::VecDeque::new(),
            #[cfg(feature = "media")]
            export_dialog: None,
            #[cfg(target_os = "macos")]
            native_menu: None,
        }
    }
}

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod geometry_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod lane_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod dock_tests;
