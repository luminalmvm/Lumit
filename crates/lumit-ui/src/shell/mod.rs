//! The application shell: menu bar, docked panels, status line.
//!
//! Layout per docs/07-UI-SPEC.md (Edit workspace): Project left, Viewer centre,
//! Effect Controls / Effects & Presets right, Timeline across the bottom.

pub(crate) use crate::app_state::{AppState, ShapeKind, ToolMode};
pub(crate) use crate::icons::Icon;
pub(crate) use crate::splash::{BootLine, Splash};
pub(crate) use crate::theme::Theme;
pub(crate) use lumit_core::model::ProjectItem;
pub(crate) use serde::{Deserialize, Serialize};

mod command_palette;
mod dock;
mod draws;
mod gpu;
mod graph;
mod hierarchy;
mod inspector;
mod overlays;
mod panels;
mod scopes;
mod settings;
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

impl Shell {
    pub fn new(
        ctx: &egui::Context,
        restored: Option<Self>,
        boot_notes: Vec<String>,
        #[cfg(feature = "media")] render_state: Option<egui_wgpu::RenderState>,
    ) -> Self {
        let workspace_restored = restored.is_some();
        let mut shell = restored.unwrap_or_default();
        Theme::install_fonts(ctx);
        // Migrate a pre-K-097 save (see `migrated_scheme`).
        shell.color_scheme =
            migrated_scheme(shell.color_scheme, shell.theme_mode, shell.theme_variant);
        // Honour the saved picks (K-097 scheme × K-092 shape).
        shell.theme = Theme::for_scheme(shell.color_scheme, shell.theme_shape);
        if let Some(rgb) = shell.accent_override {
            shell.theme = shell.theme.with_accent(rgb);
        }
        shell.theme.apply(ctx);
        crate::theme::apply_animation_level(ctx, shell.animation_level);
        ctx.style_mut(|s| s.visuals.panel_fill = shell.theme.surface_0);
        // Honour saved cache budgets (Settings → Performance).
        shell.apply_cache_budgets();
        // Honour saved interface settings (Settings → Interface, K-117): a
        // saved non-default UI scale or a tooltips-off pick takes effect
        // before the first frame, not just after the next control touch.
        ctx.set_pixels_per_point(shell.interface.ui_scale);
        settings::apply_tooltips_enabled(ctx, shell.interface.show_tooltips);

        // The boot log (K-008): every line reflects real initialisation state.
        let mut lines = vec![
            BootLine::ok(format!(
                "Theme: {} ({})",
                shell.color_scheme.label(),
                match shell.theme_shape {
                    crate::theme::ThemeShape::Sharp => "sharp",
                    crate::theme::ThemeShape::Round => "round",
                }
            )),
            BootLine::ok(if workspace_restored {
                "Workspace: restored"
            } else {
                "Workspace: default (Edit)"
            }),
            BootLine::ok("Document store: ready"),
            BootLine::ok("Recovery journal: clean"),
        ];
        lines.extend(boot_notes.into_iter().map(BootLine::ok));
        #[cfg(feature = "media")]
        lines.push(BootLine::ok(format!(
            "Media engine: FFmpeg (libavformat {})",
            lumit_media::ffmpeg_version()
        )));
        #[cfg(feature = "media")]
        match render_state {
            Some(rs) => {
                shell.gpu = Some(GpuViewer::new(rs));
                lines.push(BootLine::ok(
                    "Togi render pipeline: GPU (sRGB → linear fp16 → display)",
                ));
            }
            None => lines.push(BootLine {
                text: "Togi render pipeline: CPU fallback (no wgpu render state)".into(),
                failed: true,
            }),
        }
        #[cfg(feature = "media")]
        lines.push(BootLine::ok("Kura cache: RAM tier ready (512 MB)"));
        #[cfg(feature = "media")]
        lines.push(BootLine::ok(
            "Hibiki audio: cpal (clock starts with playback)",
        ));
        lines.push(BootLine::ok(format!(
            "Effects: {} built-in registered",
            lumit_core::fx::BUILTINS.len()
        )));

        #[cfg(target_os = "macos")]
        {
            match crate::native_menu::NativeMenu::install() {
                Ok(menu) => {
                    shell.native_menu = Some(menu);
                    lines.push(BootLine::ok("Menu bar: native (macOS)"));
                }
                Err(e) => lines.push(BootLine {
                    text: format!("Menu bar: in-window fallback ({e})"),
                    failed: true,
                }),
            }
        }

        shell.splash = Some(Splash::new(lines));
        shell
    }

    #[cfg(target_os = "macos")]
    fn native_menu_frame(&mut self) {
        use crate::native_menu::MenuAction;
        let Some(menu) = &self.native_menu else {
            return;
        };
        let actions = menu.poll();
        let (can_undo, can_redo) = (self.app.store.can_undo(), self.app.store.can_redo());
        menu.sync(can_undo, can_redo);
        for action in actions {
            match action {
                MenuAction::NewProject => self.app.new_project(),
                MenuAction::OpenProject => self.app.open_dialog(),
                MenuAction::ImportFootage => self.app.import_footage_dialog(),
                MenuAction::Save => self.app.save(),
                MenuAction::ExportComp => {
                    // A generic "Export…" action stamps the Settings →
                    // Export default preset (K-119); an explicit preset
                    // pick below always keeps its own preset regardless.
                    #[cfg(feature = "media")]
                    self.open_export_dialog(self.settings_export.default_preset);
                }
                MenuAction::ExportYouTube1080 => {
                    #[cfg(feature = "media")]
                    self.open_export_dialog(crate::export::ExportPreset::Youtube1080p60);
                }
                MenuAction::ExportVertical => {
                    #[cfg(feature = "media")]
                    self.open_export_dialog(crate::export::ExportPreset::Vertical1080p60);
                }
                MenuAction::ShareExport50 => {
                    #[cfg(feature = "media")]
                    self.start_share_export(50.0);
                }
                MenuAction::ShareExport10 => {
                    #[cfg(feature = "media")]
                    self.start_share_export(10.0);
                }
                MenuAction::Undo => self.app.undo(),
                MenuAction::Redo => self.app.redo(),
                MenuAction::NewComposition => self.app.new_composition(),
                MenuAction::AddSolidLayer => self.app.add_solid_layer(),
                MenuAction::AddTextLayer => self.app.add_text_layer(),
                MenuAction::AddCameraLayer => self.app.add_camera_layer(),
                MenuAction::AddAdjustmentLayer => self.app.add_adjustment_layer(),
                MenuAction::AddSequenceLayer => self.app.add_sequence_layer(),
                MenuAction::CutClip => self.app.cut_sequence_at_playhead(),
                MenuAction::DeleteClip => self.app.delete_clip_at_playhead(),
                MenuAction::AddMarker => self.app.add_marker_at_playhead(),
                MenuAction::ClearBeatMarkers => self.app.clear_beat_markers(),
                MenuAction::DetectBeats => {
                    #[cfg(feature = "media")]
                    if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                        self.app.detect_beats(id, 1.5);
                    }
                }
                MenuAction::DetectBeatsMore => {
                    #[cfg(feature = "media")]
                    if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                        self.app.detect_beats(id, 1.1);
                    }
                }
                MenuAction::AddMaskRectangle => self.add_mask_to_selected(ShapeKind::Rectangle),
                MenuAction::AddMaskEllipse => self.add_mask_to_selected(ShapeKind::Ellipse),
                MenuAction::AddMaskStar => self.add_mask_to_selected(ShapeKind::Star),
                MenuAction::CompSettings => {
                    if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                        self.app.open_comp_settings(id);
                    }
                }
                MenuAction::ResetWorkspace => self.dock = default_layout(),
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        const REDO: KeyboardShortcut =
            KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
        const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        // The macOS-standard Settings shortcut (Cmd/Ctrl+comma).
        const SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
        // Command palette (Cmd/Ctrl+Shift+P, per docs/07-UI-SPEC §12/§15).
        const PALETTE: KeyboardShortcut =
            KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::P);
        // Order matters: consume the more-modified shortcut first.
        if ctx.input_mut(|i| i.consume_shortcut(&REDO)) {
            self.app.redo();
        } else if ctx.input_mut(|i| i.consume_shortcut(&UNDO)) {
            self.app.undo();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SAVE)) {
            self.app.save();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SETTINGS)) {
            self.settings_open = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&PALETTE)) {
            self.open_command_palette();
        }
    }

    /// Size-targeted share export (K-037): bitrate from the byte budget,
    /// with the audio track's share subtracted first.
    #[cfg(feature = "media")]
    fn start_share_export(&mut self, target_mb: f64) {
        let Some(comp_id) = self.app.preview_comp.or(self.app.selected_comp) else {
            self.app.error = Some("select a composition to export".into());
            return;
        };
        let doc = self.app.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let duration = match comp.work_area {
            Some((a, b)) => (b.0.to_f64() - a.0.to_f64()).max(0.1),
            None => comp.duration.0.to_f64().max(0.1),
        };
        // 8% container/overhead headroom; a leaner AAC rate than the
        // delivery presets because every audio bit comes out of the budget.
        let audio_bit_rate: i64 = 192_000;
        let has_audio = !self.app.comp_audio_jobs(&doc, comp).is_empty();
        let mut bits = target_mb * 1_000_000.0 * 8.0 * 0.92;
        if has_audio {
            bits -= audio_bit_rate as f64 * duration;
        }
        let bit_rate = ((bits / duration) as i64).max(100_000);
        let spec = crate::export::ExportSpec {
            codec: lumit_media::encode::VideoCodec::H264,
            target: (comp.width, comp.height),
            bit_rate: Some(bit_rate),
            max_rate: None,
            include_audio: true,
            audio_bit_rate,
        };
        self.enqueue_export(comp_id, spec, &format!("share-{}mb.mp4", target_mb as u64));
    }

    /// Open the export dialogue for the current comp, with `preset` applied
    /// (Custom = the comp's own size and the encoder's default quality).
    #[cfg(feature = "media")]
    fn open_export_dialog(&mut self, preset: crate::export::ExportPreset) {
        let Some(comp_id) = self.app.preview_comp.or(self.app.selected_comp) else {
            self.app.error = Some("select a composition to export".into());
            return;
        };
        let doc = self.app.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let mut dialog = ExportDialogState {
            comp_id,
            comp_size: (comp.width, comp.height),
            comp_name: comp.name.clone(),
            filename_template: self.settings_export.filename_template.clone(),
            preset,
            stamped: None,
            codec: lumit_media::encode::VideoCodec::H264,
            size: None,
            bitrate_mbps: String::new(),
            include_audio: true,
            default_name: "export.mp4".to_string(),
        };
        dialog.apply(preset);
        self.export_dialog = Some(dialog);
    }

    /// Ask where to save, then queue one export. Queue items snapshot the
    /// document now (docs/06 §7.1); the queue runs them one after another.
    #[cfg(feature = "media")]
    fn enqueue_export(
        &mut self,
        comp_id: uuid::Uuid,
        spec: crate::export::ExportSpec,
        default_name: &str,
    ) {
        let doc = self.app.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let picked = rfd::FileDialog::new()
            .add_filter("MP4 video", &["mp4"])
            .set_file_name(default_name)
            .save_file();
        let Some(path) = picked else { return };
        let items = crate::export::item_infos(&doc, &self.app.media);
        let audio = self.app.comp_audio_jobs(&doc, comp);
        self.export_queue.push_back(crate::export::QueuedExport {
            doc,
            comp_id,
            items,
            audio,
            out_path: path,
            spec,
        });
        self.try_start_next_export();
    }

    /// Start the next queued export if none is running.
    #[cfg(feature = "media")]
    fn try_start_next_export(&mut self) {
        if self.export.is_some() {
            return;
        }
        let Some(next) = self.export_queue.pop_front() else {
            return;
        };
        let Some(gpu) = &self.gpu else {
            self.app.error = Some("export needs the GPU pipeline".into());
            self.export_queue.clear();
            return;
        };
        self.export_name = Some(
            next.out_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "export".into()),
        );
        self.export_encoder = None;
        self.export_progress = Some((0, 0));
        self.export = Some(crate::export::start(
            next.doc,
            next.comp_id,
            next.items,
            next.audio,
            gpu.export_context(),
            next.out_path,
            next.spec,
        ));
    }

    /// The composition settings dialogue (create + edit — K-068).
    /// Add a mask of `kind` to the selected layer, centred (the menu path;
    /// the toolbar's shape tool is the draw-a-box path).
    fn add_mask_to_selected(&mut self, kind: ShapeKind) {
        let doc = self.app.store.snapshot();
        let Some(comp_id) = self.app.selected_comp else {
            self.app.error = Some("select a composition first".into());
            return;
        };
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer_id) = self.app.selected_layer else {
            self.app.error = Some("select a layer first".into());
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let (w, h) = mask_space(layer, &self.app, comp);
        let mask = match kind {
            ShapeKind::Rectangle => {
                lumit_core::mask::Mask::rectangle(w * 0.25, h * 0.25, w * 0.5, h * 0.5)
            }
            ShapeKind::Ellipse => {
                lumit_core::mask::Mask::ellipse(w * 0.5, h * 0.5, w * 0.3, h * 0.3)
            }
            ShapeKind::Star => {
                lumit_core::mask::Mask::star(w * 0.5, h * 0.5, w * 0.32, w * 0.14, 5)
            }
        };
        let mut masks = layer.masks.clone();
        masks.push(mask);
        self.app.commit(lumit_core::Op::SetLayerMasks {
            comp: comp_id,
            layer: layer_id,
            masks,
        });
        #[cfg(feature = "media")]
        self.app.refresh_preview();
    }

    /// The export dialogue: preset, codec, frame, bitrate, audio — the
    /// stamped preset numbers stay editable (the custom path). Confirming
    /// asks where to save and queues the export.
    #[cfg(feature = "media")]
    fn export_dialog_modal(&mut self, ctx: &egui::Context) {
        use crate::export::ExportPreset;
        use lumit_media::encode::VideoCodec;
        let Some(dialog) = &mut self.export_dialog else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Export")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                let theme = &self.theme;
                egui::Grid::new("export-dialog")
                    .num_columns(2)
                    .spacing(egui::vec2(12.0, 8.0))
                    .show(ui, |ui| {
                        ui.label("Preset");
                        bare_dropdown(ui, dialog.preset.label(), |ui| {
                            for preset in ExportPreset::ALL {
                                if ui.button(preset.label()).clicked() {
                                    dialog.apply(preset);
                                    ui.close_menu();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("Codec");
                        bare_dropdown(ui, dialog.codec.label(), |ui| {
                            for codec in [VideoCodec::H264, VideoCodec::Hevc] {
                                if ui.button(codec.label()).clicked() {
                                    dialog.codec = codec;
                                    ui.close_menu();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("Frame");
                        ui.horizontal(|ui| {
                            let (w, h) = dialog.size.unwrap_or(dialog.comp_size);
                            let suffix = if dialog.size.is_none() {
                                " (comp size)"
                            } else {
                                ""
                            };
                            ui.label(
                                egui::RichText::new(format!("{w}×{h}{suffix}"))
                                    .monospace()
                                    .color(theme.text_muted),
                            );
                            if dialog.size.is_some() && ui.small_button("Use comp size").clicked() {
                                dialog.size = None;
                            }
                        });
                        ui.end_row();

                        ui.label("Bitrate");
                        ui.horizontal(|ui| {
                            let (resp, buf) = text_field(
                                ui,
                                egui::Id::new("export-bitrate"),
                                &dialog.bitrate_mbps,
                                72.0,
                            );
                            if resp.changed() {
                                dialog.bitrate_mbps = buf;
                            }
                            ui.label(
                                egui::RichText::new("Mbps — empty for default quality")
                                    .small()
                                    .color(theme.text_muted),
                            );
                        });
                        ui.end_row();

                        ui.label("Audio");
                        ui.checkbox(&mut dialog.include_audio, "Include audio (AAC 320 kbps)");
                        ui.end_row();
                    });
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Frame rate follows the composition. Exports queue and run in order.",
                    )
                    .small()
                    .color(self.theme.text_disabled),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Export…").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        confirm = true;
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        cancel = true;
                    }
                });
            });
        if confirm {
            if let Some(dialog) = self.export_dialog.take() {
                let spec = dialog.spec();
                self.enqueue_export(dialog.comp_id, spec, &dialog.default_name);
            }
        } else if cancel {
            self.export_dialog = None;
        }
    }

    fn comp_dialog_modal(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.app.comp_dialog else {
            return;
        };
        let creating = dialog.editing.is_none();
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(if creating {
            "New composition"
        } else {
            "Composition settings"
        })
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            let theme = &self.theme;
            egui::Grid::new("comp-dialog")
                .num_columns(2)
                .spacing(egui::vec2(12.0, 8.0))
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(220.0));
                    ui.end_row();

                    // Width × Height on one line, with a ratio lock.
                    ui.label("Size");
                    ui.horizontal(|ui| {
                        let (w_resp, w_buf) = text_field(
                            ui,
                            egui::Id::new("comp-w"),
                            &dialog.width.to_string(),
                            60.0,
                        );
                        ui.label(egui::RichText::new("×").color(theme.text_muted));
                        let (h_resp, h_buf) = text_field(
                            ui,
                            egui::Id::new("comp-h"),
                            &dialog.height.to_string(),
                            60.0,
                        );
                        if w_resp.lost_focus() {
                            if let Ok(w) = w_buf.trim().parse::<u32>() {
                                dialog.width = w.clamp(16, 16384);
                                if dialog.lock_ratio {
                                    dialog.height =
                                        ((f64::from(dialog.width) / dialog.aspect).round() as u32)
                                            .clamp(16, 16384);
                                } else {
                                    dialog.aspect =
                                        f64::from(dialog.width) / f64::from(dialog.height).max(1.0);
                                }
                            }
                        }
                        if h_resp.lost_focus() {
                            if let Ok(h) = h_buf.trim().parse::<u32>() {
                                dialog.height = h.clamp(16, 16384);
                                if dialog.lock_ratio {
                                    dialog.width =
                                        ((f64::from(dialog.height) * dialog.aspect).round() as u32)
                                            .clamp(16, 16384);
                                } else {
                                    dialog.aspect =
                                        f64::from(dialog.width) / f64::from(dialog.height).max(1.0);
                                }
                            }
                        }
                        let lock = dialog.lock_ratio;
                        if icon_button(
                            ui,
                            theme,
                            if lock { Icon::Lock } else { Icon::Unlock },
                            lock,
                        )
                        .on_hover_text("Lock aspect ratio")
                        .clicked()
                        {
                            dialog.lock_ratio = !lock;
                            dialog.aspect =
                                f64::from(dialog.width) / f64::from(dialog.height).max(1.0);
                        }
                        ui.label(
                            egui::RichText::new(aspect_ratio_label(dialog.width, dialog.height))
                                .small()
                                .monospace()
                                .color(theme.text_muted),
                        );
                    });
                    ui.end_row();

                    // Frame rate: free text, plus a preset dropdown (arbitrary
                    // values such as 29.9997 are accepted).
                    ui.label("Frame rate");
                    ui.horizontal(|ui| {
                        let shown = format!("{:.4}", dialog.fps);
                        let shown = shown.trim_end_matches('0').trim_end_matches('.');
                        let (resp, buf) = text_field(ui, egui::Id::new("comp-fps"), shown, 72.0);
                        if resp.lost_focus() {
                            if let Ok(f) = buf.trim().parse::<f64>() {
                                dialog.fps = f.clamp(1.0, 1000.0);
                            }
                        }
                        ui.label(egui::RichText::new("fps").small().color(theme.text_muted));
                        bare_dropdown(ui, "Presets", |ui| {
                            for preset in
                                [23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 59.94, 60.0, 120.0]
                            {
                                if ui.button(format!("{preset}")).clicked() {
                                    dialog.fps = preset;
                                    ui.close_menu();
                                }
                            }
                        });
                    });
                    ui.end_row();

                    // Duration as HH:MM:SS:mmm.
                    ui.label("Duration");
                    let (d_resp, d_buf) = text_field(
                        ui,
                        egui::Id::new("comp-dur"),
                        &fmt_duration(dialog.duration_s),
                        110.0,
                    );
                    if d_resp.lost_focus() {
                        if let Some(secs) = parse_duration(&d_buf) {
                            dialog.duration_s = secs.clamp(0.04, 86_400.0);
                        }
                    }
                    ui.end_row();
                });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Duration is HH:MM:SS:mmm.")
                    .small()
                    .color(self.theme.text_disabled),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button(if creating { "Create" } else { "Apply" })
                    .clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    confirm = true;
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        });
        if confirm {
            self.app.confirm_comp_dialog();
        } else if cancel {
            self.app.comp_dialog = None;
        }
    }

    /// True while any modal dialog is up (Settings, the composition or export
    /// dialogs, or the recovery prompt). Callers that read the raw pointer —
    /// the active-panel focus edge — skip themselves so a click meant for the
    /// dialog never reaches a panel behind it.
    fn any_modal_open(&self) -> bool {
        if self.settings_open
            || self.palette_open
            || self.app.comp_dialog.is_some()
            || self.app.pending_recovery.is_some()
        {
            return true;
        }
        #[cfg(feature = "media")]
        if self.export_dialog.is_some() {
            return true;
        }
        false
    }

    fn recovery_modal(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.app.pending_recovery else {
            return;
        };
        let n = pending.ops.len();
        let mut choice: Option<bool> = None;
        egui::Window::new("Recover changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "The last session ended without saving. {n} change{} can be restored.",
                    if n == 1 { "" } else { "s" }
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(format!(
                            "Restore {n} change{}",
                            if n == 1 { "" } else { "s" }
                        ))
                        .clicked()
                    {
                        choice = Some(true);
                    }
                    if ui.button("Open last save").clicked() {
                        choice = Some(false);
                    }
                });
            });
        if let Some(recover) = choice {
            self.app.resolve_recovery(recover);
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if let Some(splash) = &self.splash {
            if crate::splash::show(ctx, &self.theme, splash) {
                // Boot finished: the splash window becomes the application window.
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1440.0, 900.0)));
                self.splash = None;
            }
            return;
        }
        self.app.autosave_tick(
            self.autosave.interval_mins as u64 * 60,
            self.autosave.keep as usize,
        );
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.app.import_paths(dropped);
        }
        #[cfg(feature = "media")]
        {
            self.app.poll_audio();
            self.app.poll_comp_audio();
            self.app.poll_beats();
            // Transport keys (07-UI-SPEC keymap; shuttle speeds arrive with
            // the ring buffer — J/left step back, L plays, K/Space pause).
            if (self.app.preview_item.is_some() || self.app.preview_comp.is_some())
                && !ctx.wants_keyboard_input()
            {
                let (space, k, l, left, right, j, home) = ctx.input(|i| {
                    (
                        i.key_pressed(egui::Key::Space),
                        i.key_pressed(egui::Key::K),
                        i.key_pressed(egui::Key::L),
                        i.key_pressed(egui::Key::ArrowLeft),
                        i.key_pressed(egui::Key::ArrowRight),
                        i.key_pressed(egui::Key::J),
                        i.key_pressed(egui::Key::Home),
                    )
                });
                if space {
                    self.app.toggle_play();
                }
                if k && self.app.is_playing() {
                    self.app.toggle_play();
                }
                if l && !self.app.is_playing() {
                    self.app.toggle_play();
                }
                let (b, n) =
                    ctx.input(|i| (i.key_pressed(egui::Key::B), i.key_pressed(egui::Key::N)));
                if b {
                    self.app.set_work_area_edge(false);
                }
                if n {
                    self.app.set_work_area_edge(true);
                }
                let step: i64 = i64::from(right) - i64::from(left || j);
                if step != 0 || home {
                    if self.app.is_playing() {
                        self.app.toggle_play(); // stepping implies pause
                    }
                    let frame = if home {
                        0
                    } else {
                        self.app.preview_frame.saturating_add_signed(step as isize)
                    };
                    self.app.preview_frame = frame;
                    self.app.refresh_preview();
                }
            }
            if let Some(export) = &self.export {
                let mut encoder_seen = None;
                let mut finished: Option<Result<std::path::PathBuf, String>> = None;
                while let Ok(ev) = export.events.try_recv() {
                    match ev {
                        crate::export::ExportEvent::Encoder(label) => {
                            encoder_seen = Some(label);
                        }
                        crate::export::ExportEvent::Progress { frame, total } => {
                            self.export_progress = Some((frame, total));
                        }
                        crate::export::ExportEvent::Done(path) => {
                            finished = Some(Ok(path));
                        }
                        crate::export::ExportEvent::Failed(e) => {
                            finished = Some(Err(e));
                        }
                    }
                }
                if let Some(label) = encoder_seen {
                    self.export_encoder = Some(label);
                }
                match finished {
                    Some(Ok(path)) => {
                        let with = self
                            .export_encoder
                            .map(|l| format!(" — encoded with {l}"))
                            .unwrap_or_default();
                        self.app.error = Some(format!("exported {}{with}", path.display()));
                        self.export = None;
                        self.export_progress = None;
                        self.export_encoder = None;
                        self.export_name = None;
                        // The queue carries on with the next item.
                        self.try_start_next_export();
                    }
                    Some(Err(e)) => {
                        self.app.error = Some(format!("export: {e}"));
                        self.export = None;
                        self.export_progress = None;
                        self.export_encoder = None;
                        self.export_name = None;
                        // One failed or cancelled item never stalls the rest.
                        self.try_start_next_export();
                    }
                    None => {
                        ctx.request_repaint_after(std::time::Duration::from_millis(120));
                    }
                }
            }
            if self.app.comp_playback_tick() {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            if self.app.is_playing() && self.app.preview_comp.is_none() {
                if let (Some(clock), Some(fps)) =
                    (self.app.playback_clock(), self.app.preview_fps())
                {
                    let frame = (clock * fps) as usize;
                    if frame != self.app.preview_frame {
                        self.app.preview_frame = frame;
                        self.app.refresh_preview();
                    }
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            self.app.media.poll();
            if self.app.media.any_probing() {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
            if (self.app.preview_item.is_some() || self.app.preview_comp.is_some())
                && self.preview_display.is_none()
            {
                // Selection made before probe finished: retry until Ready.
                self.app.refresh_preview();
            }
            // The disk tier follows the project's save path (and the user's
            // cache-root override, Settings → Performance → Cache), and any
            // frames it promoted land in RAM here (docs/06 §5.4).
            self.app.disk_sync_root(self.settings.cache_root.as_deref());
            if self.app.drain_disk_loads() {
                ctx.request_repaint();
            }
            // Kura warm path: a cached frame presents as a plain upload.
            if let Some(key) = self.app.cached_present.take() {
                if let Some(gpu) = &mut self.gpu {
                    // VRAM first (docs/06 §5): a still-resident display texture
                    // re-presents with zero GPU work; a RAM hit re-uploads and
                    // joins the VRAM tier for next time.
                    if let Some(hit) = gpu.present_vram(key) {
                        self.preview_display = Some(hit);
                    } else if let Some(frame) = self.app.comp_frame_cache.get(&key) {
                        let (w, h, rgba) = (frame.width, frame.height, frame.rgba.clone());
                        self.preview_display = Some(gpu.present_keyed(key, &rgba, w, h));
                    }
                }
            }
            // Idle: fill the work area around the playhead, one frame at a
            // time (any real request supersedes the fill mid-flight). Paused
            // while scrubbing/dragging so fills don't fight the interaction.
            if self.settings.background_fill
                && !self.app.is_playing()
                && !self.app.is_interacting()
                && self.app.fill_in_flight.is_none()
            {
                if let Some(comp_id) = self.app.preview_comp {
                    if let Some(frame) = self.app.next_fill_frame(comp_id) {
                        // Disk-first (docs/06 §5.4: promote, never re-render
                        // what is already parked): a frame the disk tier holds
                        // is loaded back instead of rendered; the render path
                        // only runs on frames no tier has.
                        let promoted = match self.app.frame_key_for(comp_id, frame) {
                            Some(key) if self.app.disk_has(key) => {
                                self.app.disk_request_load(key);
                                true
                            }
                            _ => false,
                        };
                        if !promoted {
                            self.app.request_fill_frame(comp_id, frame);
                        }
                        ctx.request_repaint_after(std::time::Duration::from_millis(30));
                    }
                }
            }
            let mut newest = None;
            while let Ok(result) = self.app.preview_engine.results.try_recv() {
                newest = Some(result);
            }
            use crate::app_state::preview::PreviewResult;
            match newest {
                Some(Ok(PreviewResult::Comp(cf))) if Some(cf.comp) == self.app.preview_comp => {
                    // Only the frame under the playhead is presented; any other
                    // frame (a background fill, or a stale render that arrived
                    // after an edit moved on) is banked, never shown — otherwise
                    // the viewport jumps to whatever fill just finished.
                    let is_fill = cf.frame != self.app.preview_frame;
                    if let Some(gpu) = &mut self.gpu {
                        let doc = self.app.store.snapshot();
                        if let Some(comp) = doc.comp(cf.comp) {
                            let t_comp = cf.frame as f64 / comp.frame_rate.fps().max(1.0);
                            let pixels_by_layer: std::collections::HashMap<_, _> =
                                cf.layers.iter().map(|lp| (lp.layer, lp)).collect();
                            let mut visited = vec![comp.id];
                            let draws = build_comp_draws(
                                &doc,
                                comp,
                                t_comp,
                                &pixels_by_layer,
                                &mut visited,
                            );
                            let bg = comp.background.0;
                            let background = [
                                f64::from(bg[0]),
                                f64::from(bg[1]),
                                f64::from(bg[2]),
                                f64::from(bg[3]),
                            ];
                            let pose = comp.camera_pose(t_comp);
                            if is_fill {
                                // Background fill: readback, store, don't show.
                                if let (Some(key), Some(rgba)) = (
                                    self.app.frame_key_for(cf.comp, cf.frame),
                                    gpu.realise_to_bytes(
                                        pose,
                                        comp.width,
                                        comp.height,
                                        background,
                                        &draws,
                                    ),
                                ) {
                                    self.app.disk_store_behind(
                                        key,
                                        comp.width,
                                        comp.height,
                                        rgba.clone(),
                                    );
                                    self.app.comp_frame_cache.insert(
                                        key,
                                        crate::app_state::CachedCompFrame {
                                            width: comp.width,
                                            height: comp.height,
                                            rgba,
                                        },
                                    );
                                    self.app.cache_epoch += 1;
                                }
                                self.app.fill_in_flight = None;
                            } else {
                                self.preview_display = Some(gpu.present_comp(
                                    pose,
                                    comp.width,
                                    comp.height,
                                    background,
                                    &draws,
                                ));
                                // Paused: bank the frame while it's hot (playback
                                // misses skip the readback to protect the frame
                                // budget; draft frames are never banked — the
                                // cache holds specified-resolution frames only).
                                if !self.app.is_playing() && !self.app.preview_draft {
                                    if let Some(key) = self.app.frame_key_for(cf.comp, cf.frame) {
                                        if !self.app.comp_frame_cache.contains_key(&key) {
                                            if let Some(rgba) = gpu.realise_to_bytes(
                                                pose,
                                                comp.width,
                                                comp.height,
                                                background,
                                                &draws,
                                            ) {
                                                self.app.disk_store_behind(
                                                    key,
                                                    comp.width,
                                                    comp.height,
                                                    rgba.clone(),
                                                );
                                                self.app.comp_frame_cache.insert(
                                                    key,
                                                    crate::app_state::CachedCompFrame {
                                                        width: comp.width,
                                                        height: comp.height,
                                                        rgba,
                                                    },
                                                );
                                                self.app.cache_epoch += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Retain the presented frame's decoded pixels so a value
                    // drag can re-composite from them without re-decoding.
                    if !is_fill {
                        self.last_comp = Some(cf);
                    }
                }
                Some(Ok(PreviewResult::Footage(px)))
                    if Some(px.item) == self.app.preview_item
                        && self.app.preview_comp.is_none() =>
                {
                    if let Some(gpu) = &mut self.gpu {
                        self.preview_display = Some(gpu.present(&px.rgba, px.width, px.height));
                    } else {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [px.width as usize, px.height as usize],
                            &px.rgba,
                        );
                        let tex =
                            ctx.load_texture("viewer-frame", image, egui::TextureOptions::LINEAR);
                        self.preview_display = Some((tex.id(), tex.size_vec2()));
                        self.preview_tex = Some(tex);
                    }
                }
                Some(Err(e)) => self.app.error = Some(format!("preview: {e}")),
                _ => {}
            }
            // Edits (commits/undo) re-render the comp preview automatically.
            let doc_ptr = std::sync::Arc::as_ptr(&self.app.store.snapshot()) as usize;
            if self.app.preview_comp.is_some() && self.last_doc_ptr != doc_ptr {
                self.last_doc_ptr = doc_ptr;
                self.app.refresh_preview();
            } else {
                self.last_doc_ptr = doc_ptr;
            }

            // Live edit preview: while a transform value OR a graph keyframe is
            // being dragged, re-composite the retained frame with the provisional
            // value patched in for this frame only — instant feedback with no
            // re-decode, since a transform change never alters which footage
            // frame a layer shows.
            if let Some(comp_id) = self.app.preview_comp {
                if let (Some(gpu), Some(cf)) = (&mut self.gpu, &self.last_comp) {
                    if cf.comp == comp_id && cf.frame == self.app.preview_frame {
                        let doc = self.app.store.snapshot();
                        if let Some(comp) = doc.comp(comp_id) {
                            let t_comp = cf.frame as f64 / comp.frame_rate.fps().max(1.0);
                            use lumit_core::model::TransformProp;
                            // A linked-scale drag moves both axes at once, so
                            // patch both; otherwise a direct value drag gives
                            // (layer, prop, value), and a graph keyframe drag
                            // gives the property's provisional value at the
                            // playhead.
                            let patched = if let Some((sl, nx, ny)) = self.app.scale_preview {
                                Some(patch_layer_prop(
                                    &patch_layer_prop(comp, sl, TransformProp::ScaleX, nx),
                                    sl,
                                    TransformProp::ScaleY,
                                    ny,
                                ))
                            } else if let Some((lid, ei, pi, val)) = self.app.fx_edit {
                                // Live effect-value drag: patch the effect
                                // param and let the stack re-run with it.
                                Some(patch_layer_effect_param(comp, lid, ei, pi, val))
                            } else {
                                self.app
                                    .prop_edit
                                    .or_else(|| {
                                        let (idx, kt, kv) = self.app.graph_edit?;
                                        let prop = self.app.graph_prop?;
                                        let layer_id = self.app.selected_layer?;
                                        let layer =
                                            comp.layers.iter().find(|l| l.id == layer_id)?;
                                        let lumit_core::anim::Animation::Keyframed(keys) =
                                            &layer.transform.get(prop).animation
                                        else {
                                            return None;
                                        };
                                        let mut keys = keys.clone();
                                        let k = keys.get_mut(idx)?;
                                        k.time = rational_at(kt.max(0.0));
                                        k.value = kv;
                                        keys.sort_by_key(|k| k.time);
                                        let lt = t_comp - layer.start_offset.0.to_f64();
                                        let value = lumit_core::anim::evaluate(&keys, lt)?;
                                        Some((layer_id, prop, value))
                                    })
                                    .map(|(edit_layer, prop, value)| {
                                        patch_layer_prop(comp, edit_layer, prop, value)
                                    })
                            };
                            if let Some(patched) = patched {
                                let pixels_by_layer: std::collections::HashMap<_, _> =
                                    cf.layers.iter().map(|lp| (lp.layer, lp)).collect();
                                let mut visited = vec![comp_id];
                                let draws = build_comp_draws(
                                    &doc,
                                    &patched,
                                    t_comp,
                                    &pixels_by_layer,
                                    &mut visited,
                                );
                                let bg = comp.background.0;
                                let background = [
                                    f64::from(bg[0]),
                                    f64::from(bg[1]),
                                    f64::from(bg[2]),
                                    f64::from(bg[3]),
                                ];
                                let pose = patched.camera_pose(t_comp);
                                self.preview_display = Some(gpu.present_comp(
                                    pose,
                                    comp.width,
                                    comp.height,
                                    background,
                                    &draws,
                                ));
                                ctx.request_repaint();
                            }
                        }
                    }
                }
            }
        }
        #[cfg(target_os = "macos")]
        self.native_menu_frame();
        #[cfg(not(target_os = "macos"))]
        self.shortcuts(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.app.project_title()));

        #[cfg(not(target_os = "macos"))]
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New project").clicked() {
                        self.app.new_project();
                        ui.close_menu();
                    }
                    if ui.button("Open project…").clicked() {
                        self.app.open_dialog();
                        ui.close_menu();
                    }
                    if ui.button("Import footage…").clicked() {
                        self.app.import_footage_dialog();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Save").clicked() {
                        self.app.save();
                        ui.close_menu();
                    }
                    #[cfg(feature = "media")]
                    if ui.button("Export comp…").clicked() {
                        // Settings → Export default preset (K-119); the
                        // "Export preset" submenu below always keeps its
                        // own explicit pick regardless of this default.
                        self.open_export_dialog(self.settings_export.default_preset);
                        ui.close_menu();
                    }
                    #[cfg(feature = "media")]
                    ui.menu_button("Export preset", |ui| {
                        for preset in [
                            crate::export::ExportPreset::Youtube1080p60,
                            crate::export::ExportPreset::Youtube4k60,
                            crate::export::ExportPreset::Vertical1080p60,
                        ] {
                            if ui.button(preset.label()).clicked() {
                                self.open_export_dialog(preset);
                                ui.close_menu();
                            }
                        }
                    });
                    #[cfg(feature = "media")]
                    ui.menu_button("Export for sharing", |ui| {
                        if ui.button("Discord 50 MB").clicked() {
                            self.start_share_export(50.0);
                            ui.close_menu();
                        }
                        if ui.button("Small 10 MB").clicked() {
                            self.start_share_export(10.0);
                            ui.close_menu();
                        }
                    });
                });
                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(self.app.store.can_undo(), egui::Button::new("Undo"))
                        .clicked()
                    {
                        self.app.undo();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(self.app.store.can_redo(), egui::Button::new("Redo"))
                        .clicked()
                    {
                        self.app.redo();
                        ui.close_menu();
                    }
                });
                ui.menu_button("Composition", |ui| {
                    if ui.button("New composition").clicked() {
                        self.app.new_composition();
                        ui.close_menu();
                    }
                    if ui.button("Add solid layer").clicked() {
                        self.app.add_solid_layer();
                        ui.close_menu();
                    }
                    if ui.button("Add text layer").clicked() {
                        self.app.add_text_layer();
                        ui.close_menu();
                    }
                    if ui.button("Add camera layer").clicked() {
                        self.app.add_camera_layer();
                        ui.close_menu();
                    }
                    if ui.button("Add adjustment layer").clicked() {
                        self.app.add_adjustment_layer();
                        ui.close_menu();
                    }
                    if ui.button("Add sequence layer").clicked() {
                        self.app.add_sequence_layer();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.app.selected_layer.is_some(),
                            egui::Button::new("Duplicate layer"),
                        )
                        .clicked()
                    {
                        self.app.duplicate_layer();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(
                            self.app.selected_layer.is_some(),
                            egui::Button::new("Delete layer"),
                        )
                        .clicked()
                    {
                        self.app.delete_selected_layer();
                        ui.close_menu();
                    }
                    if ui.button("Cut clip at playhead").clicked() {
                        self.app.cut_sequence_at_playhead();
                        ui.close_menu();
                    }
                    if ui.button("Delete clip at playhead").clicked() {
                        self.app.delete_clip_at_playhead();
                        ui.close_menu();
                    }
                    if ui.button("Add marker at playhead").clicked() {
                        self.app.add_marker_at_playhead();
                        ui.close_menu();
                    }
                    #[cfg(feature = "media")]
                    ui.menu_button("Detect beats", |ui| {
                        if ui.button("Standard").clicked() {
                            if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                                self.app.detect_beats(id, 1.5);
                            }
                            ui.close_menu();
                        }
                        if ui.button("More markers").clicked() {
                            if let Some(id) = self.app.preview_comp.or(self.app.selected_comp) {
                                self.app.detect_beats(id, 1.1);
                            }
                            ui.close_menu();
                        }
                    });
                    if ui.button("Clear beat markers").clicked() {
                        self.app.clear_beat_markers();
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.add_enabled_ui(self.app.selected_layer.is_some(), |ui| {
                        ui.menu_button("Add mask", |ui| {
                            for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Star]
                            {
                                if ui.button(kind.label()).clicked() {
                                    self.add_mask_to_selected(kind);
                                    ui.close_menu();
                                }
                            }
                        });
                    });
                });
                ui.menu_button("Window", |ui| {
                    if ui.button("Command palette…").clicked() {
                        self.open_command_palette();
                        ui.close_menu();
                    }
                    if ui.button("Reset workspace").clicked() {
                        self.dock = default_layout();
                        ui.close_menu();
                    }
                    ui.separator();
                    // Theme, shape, motion and performance now live in the
                    // Settings window (Appearance / Performance pages).
                    if ui.button("Settings…").clicked() {
                        self.settings_open = true;
                        ui.close_menu();
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("Edit")
                            .small()
                            .color(self.theme.text_muted),
                    )
                    .on_hover_text("Workspace — presets arrive with the panel set");
                });
            });
        });

        // Tool strip: the pointer's mode (docs/07-UI-SPEC toolbar). Object
        // tools join as they land; today: navigation and mask drawing.
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            // Under Round the tools sit in their own rounded bar (owner
            // request); under Sharp the wrap is transparent with no margin,
            // so the row is pixel-identical to before.
            let theme = self.theme;
            let round = theme.shape == crate::theme::ThemeShape::Round;
            let t = theme.tokens;
            let wrap = egui::Frame::new()
                .fill(if round {
                    theme.surface_2
                } else {
                    egui::Color32::TRANSPARENT
                })
                .corner_radius(if round { t.control_radius } else { 0 })
                .shadow(if round {
                    t.card_shadow
                } else {
                    egui::Shadow::NONE
                })
                .inner_margin(if round {
                    egui::Margin::symmetric(5, 2)
                } else {
                    egui::Margin::ZERO
                });
            if round {
                ui.add_space(3.0);
            }
            ui.horizontal(|ui| {
                if round {
                    ui.add_space(t.window_inset);
                }
                wrap.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let tool = self.app.tool;
                        if icon_button(ui, &theme, Icon::Pointer, tool == ToolMode::Select)
                            .on_hover_text("Select / move the view (V)")
                            .clicked()
                        {
                            self.app.tool = ToolMode::Select;
                        }
                        if icon_button(ui, &theme, Icon::Move, tool == ToolMode::Hand)
                            .on_hover_text("Drag to pan the view (H)")
                            .clicked()
                        {
                            self.app.tool = ToolMode::Hand;
                        }
                        // The Shape button wears the current shape; right-click to switch.
                        let shape_icon = match self.app.shape_kind {
                            ShapeKind::Rectangle => Icon::Rectangle,
                            ShapeKind::Ellipse => Icon::Ellipse,
                            ShapeKind::Star => Icon::Star,
                        };
                        let shape_resp =
                            icon_button(ui, &theme, shape_icon, tool == ToolMode::Shape)
                                .on_hover_text(format!(
                                    "Draw a {} mask — right-click to pick a shape (Q)",
                                    self.app.shape_kind.label().to_lowercase()
                                ));
                        if shape_resp.clicked() {
                            self.app.tool = ToolMode::Shape;
                        }
                        shape_resp.context_menu(|ui| {
                            for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Star]
                            {
                                if ui
                                    .selectable_label(self.app.shape_kind == kind, kind.label())
                                    .clicked()
                                {
                                    self.app.shape_kind = kind;
                                    self.app.tool = ToolMode::Shape;
                                    ui.close_menu();
                                }
                            }
                        });
                        if icon_button(ui, &theme, Icon::Pen, tool == ToolMode::Pen)
                            .on_hover_text(
                                "Click points to draw a mask; click the first to close (G)",
                            )
                            .clicked()
                        {
                            self.app.tool = if tool == ToolMode::Pen {
                                ToolMode::Select
                            } else {
                                ToolMode::Pen
                            };
                            self.app.pen_path.clear();
                        }
                    });
                });
            });
        });
        // Single-key tool shortcuts, ignored while a text field has focus.
        if !ctx.wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_pressed(egui::Key::V) {
                    self.app.tool = ToolMode::Select;
                }
                if i.key_pressed(egui::Key::H) {
                    self.app.tool = ToolMode::Hand;
                }
                if i.key_pressed(egui::Key::Q) {
                    self.app.tool = ToolMode::Shape;
                }
                if i.key_pressed(egui::Key::G) {
                    self.app.tool = ToolMode::Pen;
                    self.app.pen_path.clear();
                }
            });
        }
        // Razor (Cmd/Ctrl+Shift+D). On macOS the native menu's accelerator
        // handles it, so this keyboard path is the Windows/in-window one.
        #[cfg(not(target_os = "macos"))]
        if !ctx.wants_keyboard_input()
            && ctx
                .input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::D))
        {
            self.app.cut_sequence_at_playhead();
        }

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let status = if self.app.dirty {
                    "Unsaved changes"
                } else {
                    "Ready"
                };
                ui.label(
                    egui::RichText::new(status)
                        .small()
                        .color(self.theme.text_muted),
                );
                #[cfg(feature = "media")]
                if let Some((frame, total)) = self.export_progress {
                    ui.separator();
                    let name = self.export_name.as_deref().unwrap_or("export");
                    let mut line = format!("exporting {name} {frame}/{total}");
                    if let Some(enc) = self.export_encoder {
                        line.push_str(&format!(" · {enc}"));
                    }
                    if !self.export_queue.is_empty() {
                        line.push_str(&format!(" · {} queued", self.export_queue.len()));
                    }
                    ui.label(
                        egui::RichText::new(line)
                            .monospace()
                            .small()
                            .color(self.theme.accent),
                    );
                    if ui.small_button("Cancel").clicked() {
                        if let Some(export) = &self.export {
                            export.cancel();
                        }
                    }
                }
                if let Some(err) = self.app.error.clone() {
                    ui.separator();
                    ui.label(egui::RichText::new(&err).small().color(self.theme.warning));
                    if ui.small_button("Dismiss").clicked() {
                        self.app.error = None;
                    }
                }
            });
        });

        self.recovery_modal(ctx);
        self.comp_dialog_modal(ctx);
        #[cfg(feature = "media")]
        self.export_dialog_modal(ctx);
        self.settings_modal(ctx);
        self.command_palette_modal(ctx);
        // Read before the borrow below splits `self` apart (used by the
        // active-panel focus edge further down).
        let modal_open = self.any_modal_open();

        // The tiling dock fills the window: a solo pane renders bare with no
        // tab bar — the Viewer's look (K-074) on every lone panel (K-086) —
        // while stacked panels carry tabs and can be dragged to re-arrange
        // the workspace.
        let Shell {
            dock,
            floating,
            theme,
            app,
            preview_display,
            ..
        } = self;
        let preview_display = *preview_display;
        let tree_id = dock.id();
        let bare_tiles = bare_tile_ids(dock);
        let (pop_out, panel_rects) = {
            let mut behavior = DockBehavior {
                theme,
                app,
                preview_display,
                pop_out: None,
                panel_rects: Vec::new(),
                tree_id,
                bare_tiles,
            };
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::default()
                        .fill(theme.surface_0)
                        .inner_margin(theme.tokens.window_inset),
                )
                .show(ctx, |ui| dock.ui(&mut behavior, ui));
            (behavior.pop_out, behavior.panel_rects)
        };

        // Active-panel boundary (owner request): a press inside a pane makes
        // it the focused one, and it wears a 1px accent edge so the eye always
        // knows where shortcuts land. Skipped while a modal is open — its
        // backdrop owns the click, so a press over the dialog must not reach
        // through and re-focus a panel behind it.
        if !modal_open {
            if let Some(pos) = ctx.input(|i| i.pointer.press_origin()) {
                if let Some((panel, _)) = panel_rects.iter().find(|(_, r)| r.contains(pos)) {
                    self.active_panel = Some(*panel);
                }
            }
        }
        if let Some(active) = self.active_panel {
            if let Some((_, rect)) = panel_rects.iter().find(|(p, _)| *p == active) {
                // `Order::Middle` sits above the panels (which draw in the
                // background layer) but below menus, popups and tooltips
                // (foreground and up), so the accent edge no longer paints
                // over an open menu.
                ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Middle,
                    egui::Id::new("active-panel-edge"),
                ))
                .rect_stroke(
                    rect.shrink(0.5),
                    theme.tokens.card_radius,
                    egui::Stroke::new(1.0_f32, theme.accent.gamma_multiply(0.55)),
                    egui::StrokeKind::Inside,
                );
            }
        }

        // Apply a pop-out request: hide the panel in the dock, float it. A
        // solo Timeline has no dock tab (K-086), so its request arrives from
        // the comp strip's context menu through AppState rather than from a
        // tab's pop-out button.
        let pop_out = pop_out
            .or_else(|| std::mem::take(&mut app.pop_out_timeline).then_some(Panel::Timeline));
        if let Some(panel) = pop_out {
            if let Some(tile) = tile_id_of(dock, panel) {
                dock.tiles.set_visible(tile, false);
            }
            if !floating.contains(&panel) {
                floating.push(panel);
            }
        }

        // Render each floating panel in its own OS window (an immediate
        // viewport, so it can borrow the live app state). Closing the window
        // docks the panel back into the tree where it came from.
        let mut dock_back: Vec<Panel> = Vec::new();
        for panel in floating.iter_mut() {
            let vid = egui::ViewportId::from_hash_of(("lumit-float", panel.title()));
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("Lumit — {}", panel.title()))
                .with_inner_size([640.0, 420.0]);
            ctx.show_viewport_immediate(vid, builder, |ctx, _class| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default().fill(theme.surface_0))
                    .show(ctx, |ui| {
                        render_panel(ui, theme, app, preview_display, panel)
                    });
                if ctx.input(|i| i.viewport().close_requested()) {
                    dock_back.push(*panel);
                }
            });
        }
        for panel in dock_back {
            floating.retain(|p| *p != panel);
            if let Some(tile) = tile_id_of(dock, panel) {
                dock.tiles.set_visible(tile, true);
            }
        }
    }
}

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod geometry_tests {
    use super::*;
    use crate::app_state::preview::CompLayerPixels;
    use lumit_core::model::{
        Composition, Document, Layer, LayerKind, LinearColour, Switches, TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
    use std::collections::HashMap;
    use uuid::Uuid;

    // Regression: under auto res a footage layer decodes at a reduced size that
    // changes with viewport zoom. Its comp-space geometry must use the *native*
    // source size, not the decoded size — otherwise a small layer balloons as
    // you zoom in (the auto-res bug Mack reported, 2026-07-13).
    #[test]
    fn footage_geometry_uses_native_size_not_decoded_size() {
        let item = Uuid::now_v7();
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "clip".into(),
            kind: LayerKind::Footage { item, retime: None },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Comp".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer.clone()],
            markers: Vec::new(),
            extra: serde_json::Map::new(),
        };
        // Native 1920x1080, decoded 480x270 (zoomed out, quarter res).
        let lp = CompLayerPixels {
            layer: layer.id,
            width: 480,
            height: 270,
            rgba: vec![0u8; 480 * 270 * 4],
            natural_w: 1920,
            natural_h: 1080,
            temporal: Vec::new(),
            flow_field: None,
        };
        let mut map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        map.insert(layer.id, &lp);
        let doc = Document::new();
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(&doc, &comp, 0.0, &map, &mut visited);

        assert_eq!(draws.len(), 1);
        // Geometry uses native size (zoom-independent), not the 480x270 decode.
        assert_eq!(draws[0].natural_size, (1920.0, 1080.0));
        // The texture still carries the decoded dimensions.
        match &draws[0].source {
            DrawSource::Pixels { tex_w, tex_h, .. } => assert_eq!((*tex_w, *tex_h), (480, 270)),
            _ => panic!("expected a pixel source for a footage layer"),
        }
    }

    // Collapse (docs/06 §1.4): a collapsed Precomp splices its inner draws
    // into the parent list with the parent's placement multiplied in front —
    // no Nested intermediate. Off (or forced by a mask) renders Nested.
    #[test]
    fn collapsed_precomp_splices_inner_draws_with_parent_placement() {
        use lumit_core::model::{ProjectItem, TextDocument};
        let text_layer = || Layer {
            id: Uuid::now_v7(),
            name: "inner".into(),
            kind: LayerKind::Text {
                document: TextDocument {
                    text: "hi".into(),
                    size: 24.0,
                    fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                    extra: serde_json::Map::new(),
                },
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let nested = Composition {
            id: Uuid::now_v7(),
            name: "Nested".into(),
            width: 640,
            height: 360,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![text_layer()],
            markers: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let nested_id = nested.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(nested));

        let mut pre_layer = text_layer();
        pre_layer.kind = LayerKind::Precomp { comp: nested_id };
        pre_layer.switches.collapse = true;
        pre_layer.transform.position_x = lumit_core::anim::Property::fixed(100.0);
        pre_layer.transform.scale_x = lumit_core::anim::Property::fixed(200.0);
        let parent = Composition {
            id: Uuid::now_v7(),
            name: "Parent".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![pre_layer.clone()],
            markers: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let mut visited = vec![parent.id];
        let draws = build_comp_draws(&doc, &parent, 0.0, &map, &mut visited);
        // Spliced: one draw, pixel source (the inner text), pre = the parent
        // Precomp layer's placement matrix — exactly the compositor's maths.
        assert_eq!(draws.len(), 1);
        assert!(matches!(draws[0].source, DrawSource::Pixels { .. }));
        let tr = &pre_layer.transform;
        let expect = lumit_gpu::place_matrix(
            (
                tr.position_x.value_at(0.0) as f32,
                tr.position_y.value_at(0.0) as f32,
            ),
            (
                tr.anchor_x.value_at(0.0) as f32,
                tr.anchor_y.value_at(0.0) as f32,
            ),
            (
                tr.scale_x.value_at(0.0) as f32,
                tr.scale_y.value_at(0.0) as f32,
            ),
            0.0,
            0.0,
            0.0,
            0.0,
        );
        assert_eq!(draws[0].pre, Some(expect));

        // Switch off → the Nested intermediate as before, no pre.
        let mut off = parent.clone();
        off.layers[0].switches.collapse = false;
        let mut visited = vec![off.id];
        let draws = build_comp_draws(&doc, &off, 0.0, &map, &mut visited);
        assert_eq!(draws.len(), 1);
        assert!(matches!(draws[0].source, DrawSource::Nested { .. }));
        assert!(draws[0].pre.is_none());

        // A mask on the Precomp layer forces the intermediate (§1.4) even
        // with the switch set.
        let mut forced = parent.clone();
        forced.layers[0]
            .masks
            .push(lumit_core::mask::Mask::rectangle(0.0, 0.0, 10.0, 10.0));
        let mut visited = vec![forced.id];
        let draws = build_comp_draws(&doc, &forced, 0.0, &map, &mut visited);
        assert_eq!(draws.len(), 1);
        assert!(matches!(draws[0].source, DrawSource::Nested { .. }));
    }

    // The VRAM tier's eviction policy (docs/06 §5): oldest entries drop until
    // the incoming frame fits the byte budget; an oversize frame clears all.
    #[test]
    fn vram_eviction_drops_oldest_until_the_budget_fits() {
        // 3 entries of 100 bytes under a 350 cap: adding 100 evicts nothing.
        assert_eq!(vram_evict_count(&[100, 100, 100], 300, 100, 350), 1);
        assert_eq!(vram_evict_count(&[100, 100, 100], 300, 40, 350), 0);
        // Adding a huge frame drops everything (and still admits it).
        assert_eq!(vram_evict_count(&[100, 100, 100], 300, 1000, 350), 3);
        // Empty tier: nothing to drop whatever the sizes.
        assert_eq!(vram_evict_count(&[], 0, 500, 350), 0);
    }

    // `GpuViewer::set_vram_cap` (Settings → Performance, K-100) reuses this
    // same helper with nothing incoming (`incoming = 0`) to evict down to a
    // freshly lowered budget.
    #[test]
    fn vram_evict_count_drops_to_fit_a_lowered_cap_with_nothing_incoming() {
        // 3 entries of 100 bytes, cap dropped to 150: two must go.
        assert_eq!(vram_evict_count(&[100, 100, 100], 300, 0, 150), 2);
        // Cap dropped below a single entry's size: everything goes.
        assert_eq!(vram_evict_count(&[100, 100, 100], 300, 0, 50), 3);
        // Cap raised (or unchanged): nothing is evicted.
        assert_eq!(vram_evict_count(&[100, 100, 100], 300, 0, 1024), 0);
    }

    // The live value-drag preview renders a comp patched with the provisional
    // value. Patching a layer's Position X to 500 must show through as the
    // draw's position, without touching the committed document.
    #[test]
    fn patch_layer_prop_overrides_the_previewed_value() {
        use lumit_core::model::TransformProp;
        let item = Uuid::now_v7();
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "clip".into(),
            kind: LayerKind::Footage { item, retime: None },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Comp".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer.clone()],
            markers: Vec::new(),
            extra: serde_json::Map::new(),
        };

        let patched = patch_layer_prop(&comp, layer.id, TransformProp::PositionX, 500.0);
        // The committed comp is untouched (default position 0).
        assert_eq!(comp.layers[0].transform.position_x.value_at(0.0), 0.0);

        let lp = CompLayerPixels {
            layer: layer.id,
            width: 1920,
            height: 1080,
            rgba: vec![0u8; 16],
            natural_w: 1920,
            natural_h: 1080,
            temporal: Vec::new(),
            flow_field: None,
        };
        let mut map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        map.insert(layer.id, &lp);
        let doc = Document::new();
        let mut visited = vec![patched.id];
        let draws = build_comp_draws(&doc, &patched, 0.0, &map, &mut visited);
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].position.0, 500.0);
    }

    /// An adjustment layer with a live stack emits an Adjust staging draw
    /// above the content beneath it (docs/06 §1.5), carrying its resolved
    /// effects, comp-sized geometry, and a comp-sized mask coverage; a dead
    /// stack (fx switch off, everything disabled, or no effects) emits
    /// nothing at all.
    #[test]
    fn a_live_adjustment_layer_emits_a_staging_draw() {
        let solid_def = Uuid::now_v7();
        let base = Layer {
            id: Uuid::now_v7(),
            name: "under".into(),
            kind: LayerKind::Solid { def: solid_def },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let mut adj = base.clone();
        adj.id = Uuid::now_v7();
        adj.name = "adjust".into();
        adj.kind = LayerKind::Adjustment;
        adj.effects
            .push(lumit_core::fx::instantiate("saturation").unwrap());
        adj.masks
            .push(lumit_core::mask::Mask::rectangle(0.0, 0.0, 960.0, 1080.0));
        let mut doc = Document::new();
        doc.items.push(lumit_core::model::ProjectItem::Solid(
            lumit_core::model::SolidDef {
                id: solid_def,
                name: "red".into(),
                colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
                width: 1920,
                height: 1080,
                extra: serde_json::Map::new(),
            },
        ));
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Comp".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            // Index 0 = top: the adjustment sits above the solid.
            layers: vec![adj.clone(), base.clone()],
            markers: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(&doc, &comp, 0.0, &map, &mut visited);
        // Bottom-up: the solid first, then the staging point above it.
        assert_eq!(draws.len(), 2);
        assert!(matches!(draws[0].source, DrawSource::Pixels { .. }));
        assert!(matches!(draws[1].source, DrawSource::Adjust));
        assert_eq!(draws[1].natural_size, (1920.0, 1080.0));
        assert_eq!(draws[1].fx.len(), 1);
        let (_, cov_w, cov_h) = draws[1].mask_cov.as_ref().unwrap();
        assert_eq!((*cov_w, *cov_h), (1920, 1080));

        // Dead stacks emit nothing: fx switch off, all effects disabled,
        // or an empty stack.
        for edit in [
            &(|l: &mut Layer| l.switches.fx = false) as &dyn Fn(&mut Layer),
            &|l: &mut Layer| l.effects[0].enabled = false,
            &|l: &mut Layer| l.effects.clear(),
        ] {
            let mut dead = adj.clone();
            edit(&mut dead);
            let mut comp = comp.clone();
            comp.layers[0] = dead;
            let mut visited = vec![comp.id];
            let draws = build_comp_draws(&doc, &comp, 0.0, &map, &mut visited);
            assert_eq!(draws.len(), 1, "a dead adjustment stack must not stage");
            assert!(matches!(draws[0].source, DrawSource::Pixels { .. }));
        }
    }

    // --- K-119: Settings → Export filename template ------------------------

    // The byte-identical-default-behaviour regression test: no template (or
    // a blank one) must reproduce `preset.default_file_name()` exactly, for
    // more than one preset, so an existing install's suggested export name
    // never shifts just because the setting now exists.
    #[test]
    fn no_template_reproduces_the_presets_own_default_file_name() {
        use crate::export::ExportPreset;
        for preset in [ExportPreset::Custom, ExportPreset::Youtube4k60] {
            assert_eq!(
                export_default_file_name(preset, "My Comp", None),
                preset.default_file_name()
            );
            // A template that's blank once trimmed is the same as None.
            assert_eq!(
                export_default_file_name(preset, "My Comp", Some("   ")),
                preset.default_file_name()
            );
        }
    }

    #[test]
    fn template_substitutes_comp_and_preset_tokens_and_ends_in_mp4() {
        use crate::export::ExportPreset;
        let name = export_default_file_name(
            ExportPreset::Youtube1080p60,
            "My Comp",
            Some("{comp}-{preset}"),
        );
        assert_eq!(name, "My Comp-youtube-1080p60.mp4");
    }

    #[test]
    fn template_expands_the_date_token_to_todays_utc_date() {
        let name = render_filename_template("{date}", "comp", "stem");
        assert_eq!(name, format!("{}.mp4", today_utc_date()));
    }

    // A comp name is free text: the user can put a `:` or `/` in it, and the
    // result must be sanitised, not passed through raw into a path.
    #[test]
    fn illegal_windows_filename_characters_are_sanitised_not_passed_through() {
        let name = render_filename_template("{comp}", "My:Comp/Name?", "stem");
        for illegal in ['<', '>', ':', '"', '/', '\\', '|', '?', '*'] {
            assert!(
                !name.contains(illegal),
                "{name:?} still contains illegal character {illegal:?}"
            );
        }
        assert!(name.ends_with(".mp4"));
    }

    #[test]
    fn civil_from_days_matches_known_calendar_dates() {
        // The Unix epoch itself — proves the +719468 day-count offset lines
        // up, the most bug-prone constant in Hinnant's algorithm.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // January has 31 days: day index 31 (0-based) is the first of Feb.
        assert_eq!(civil_from_days(31), (1970, 2, 1));
        // 1970 is not a leap year (365 days): day 365 rolls into 1971.
        assert_eq!(civil_from_days(365), (1971, 1, 1));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod lane_tests {
    use super::*;
    use lumit_core::model::LayerKind;
    use lumit_core::retime::Retime;
    use lumit_core::Rational;

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    // 2× over a 4 s layer eats 4 s of source by local 2 s — with the layer
    // starting at comp 1 s, the held tail runs from comp 3 s to the out point.
    #[test]
    fn overrun_span_runs_from_exhaustion_to_the_out_point() {
        let rt = Retime::constant_speed(rat(4, 1), rat(0, 1), rat(2, 1));
        let (start, end) = overrun_span_secs(&rt, 4.0, 1.0, 1.0, 5.0).unwrap();
        assert!((start - 3.0).abs() < 1e-3, "held tail starts at {start}");
        assert!((end - 5.0).abs() < 1e-9, "held tail ends at {end}");
    }

    // Plenty of source at 1× — nothing to indicate.
    #[test]
    fn no_overrun_means_no_span() {
        let rt = Retime::constant_speed(rat(4, 1), rat(0, 1), rat(1, 1));
        assert_eq!(overrun_span_secs(&rt, 10.0, 0.0, 0.0, 4.0), None);
    }

    // Source-in already past the media's end: the whole visible bar is a
    // hold, and the span clamps to start at the in point (the bar's left
    // edge), not at layer time zero.
    #[test]
    fn overrun_before_the_in_point_clamps_to_the_bar() {
        let rt = Retime::identity(rat(4, 1), rat(5, 1));
        let (start, end) = overrun_span_secs(&rt, 3.0, 0.0, 2.0, 4.0).unwrap();
        assert!(
            (start - 2.0).abs() < 1e-9,
            "span clamps to in point, got {start}"
        );
        assert!(
            (end - 4.0).abs() < 1e-9,
            "span ends at out point, got {end}"
        );
    }

    // The source runs out only past the layer's out point — the overrun is
    // real in the map but invisible on the bar, so no span.
    #[test]
    fn overrun_past_the_out_point_is_not_drawn() {
        let rt = Retime::constant_speed(rat(4, 1), rat(0, 1), rat(2, 1));
        assert_eq!(overrun_span_secs(&rt, 4.0, 0.0, 0.0, 1.5), None);
    }

    // §6.1: every layer kind paints its own identity colour on the lane bar
    // (the adjustment layer borrows the solid's until it earns its own).
    #[test]
    fn every_layer_kind_paints_its_identity_colour() {
        let theme = Theme::dark();
        let id = uuid::Uuid::now_v7();
        let text = lumit_core::model::TextDocument {
            text: String::new(),
            size: 12.0,
            fill: lumit_core::model::LinearColour::BLACK,
            extra: serde_json::Map::new(),
        };
        let cases = [
            (
                LayerKind::Footage {
                    item: id,
                    retime: None,
                },
                theme.layer.footage,
            ),
            (
                LayerKind::Sequence { clips: Vec::new() },
                theme.layer.sequence,
            ),
            (LayerKind::Precomp { comp: id }, theme.layer.precomp),
            (LayerKind::Solid { def: id }, theme.layer.solid),
            (LayerKind::Text { document: text }, theme.layer.text),
            (
                LayerKind::Camera {
                    zoom: lumit_core::anim::Property::fixed(1000.0),
                },
                theme.layer.camera,
            ),
            (LayerKind::Adjustment, theme.layer.solid),
        ];
        for (kind, want) in cases {
            assert_eq!(layer_type_style(&kind, &theme).1, want);
        }
    }

    // The identity family must stay distinct siblings — two types sharing a
    // colour would make the timeline lie about what a bar is.
    #[test]
    fn identity_colours_are_distinct() {
        let t = Theme::dark();
        let all = [
            t.layer.footage,
            t.layer.sequence,
            t.layer.precomp,
            t.layer.solid,
            t.layer.text,
            t.layer.camera,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "identity colours collide");
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod dock_tests {
    use super::*;

    /// Drive one widget through hover → press → release and report whether it
    /// registered a click. Used to prove which drag-source pattern still lets a
    /// plain click through (egui's `dnd_drag_source` does not).
    fn simulate_click(build: impl Fn(&mut egui::Ui) -> egui::Response) -> bool {
        let ctx = egui::Context::default();
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let clicked = std::cell::Cell::new(false);
        let run = |events: Vec<egui::Event>| {
            let ri = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(ri, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let r = build(ui);
                    rect.set(r.rect);
                    if r.clicked() {
                        clicked.set(true);
                    }
                });
            });
        };
        let m = egui::Modifiers::default();
        let btn = egui::PointerButton::Primary;
        run(vec![]); // lay out so the widget rect is known
        let pos = rect.get().center();
        run(vec![egui::Event::PointerMoved(pos)]); // hover
        run(vec![egui::Event::PointerButton {
            pos,
            button: btn,
            pressed: true,
            modifiers: m,
        }]);
        run(vec![egui::Event::PointerButton {
            pos,
            button: btn,
            pressed: false,
            modifiers: m,
        }]);
        clicked.get()
    }

    /// Drive one widget through two quick clicks and report whether it saw a
    /// double-click (how a comp row opens its comp).
    fn simulate_double_click(build: impl Fn(&mut egui::Ui) -> egui::Response) -> bool {
        let ctx = egui::Context::default();
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let dbl = std::cell::Cell::new(false);
        let run = |events: Vec<egui::Event>| {
            let ri = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(ri, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let r = build(ui);
                    rect.set(r.rect);
                    if r.double_clicked() {
                        dbl.set(true);
                    }
                });
            });
        };
        let m = egui::Modifiers::default();
        let btn = egui::PointerButton::Primary;
        run(vec![]); // lay out
        let pos = rect.get().center();
        run(vec![egui::Event::PointerMoved(pos)]);
        for _ in 0..2 {
            run(vec![egui::Event::PointerButton {
                pos,
                button: btn,
                pressed: true,
                modifiers: m,
            }]);
            run(vec![egui::Event::PointerButton {
                pos,
                button: btn,
                pressed: false,
                modifiers: m,
            }]);
        }
        dbl.get()
    }

    /// A comp row opens its comp on a double-click, so the draggable row must
    /// report `double_clicked()` (it senses click as well as drag).
    #[test]
    fn a_draggable_row_reports_a_double_click() {
        assert!(simulate_double_click(|ui| draggable_row(
            ui,
            egui::Id::new("row"),
            1u32,
            false,
            "row"
        )));
    }

    /// The Project panel opens comps and previews footage on a row click. egui's
    /// `dnd_drag_source` puts a drag-sensing overlay on top of its contents, so
    /// the click never reaches either the outer or the inner response — the row
    /// looked dead. A single widget that senses click *and* drag keeps both,
    /// which is what [`draggable_row`] uses.
    #[test]
    fn a_row_that_is_both_clickable_and_draggable_still_clicks() {
        // Control: a plain button clicks under this simulation.
        assert!(simulate_click(|ui| ui.button("x")));
        // The old pattern: the drag overlay eats the click.
        assert!(!simulate_click(|ui| {
            ui.dnd_drag_source(egui::Id::new("s"), 1u32, |ui| {
                ui.selectable_label(false, "x")
            })
            .response
        }));
        // The fix: one widget sensing click+drag still reports the click.
        assert!(simulate_click(|ui| draggable_row(
            ui,
            egui::Id::new("row"),
            1u32,
            false,
            "x"
        )));
    }

    /// The other half of [`draggable_row`]: dragging it must still deliver its
    /// payload to a drop target, so dropping footage/comps into the Timeline,
    /// Viewer or onto "+ Composition" keeps working after the click fix.
    #[test]
    fn dragging_a_row_delivers_its_payload_to_a_drop_target() {
        let ctx = egui::Context::default();
        let payload = uuid::Uuid::from_u128(0x1234_5678);
        let src_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let zone_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let got = std::cell::Cell::new(None);
        let run = |events: Vec<egui::Event>| {
            let ri = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(ri, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let src = draggable_row(ui, egui::Id::new("src"), payload, false, "source");
                    src_rect.set(src.rect);
                    ui.add_space(60.0);
                    let (zr, zresp) =
                        ui.allocate_exact_size(egui::vec2(120.0, 40.0), egui::Sense::hover());
                    zone_rect.set(zr);
                    if let Some(p) = zresp.dnd_release_payload::<uuid::Uuid>() {
                        got.set(Some(*p));
                    }
                });
            });
        };
        let m = egui::Modifiers::default();
        let btn = egui::PointerButton::Primary;
        run(vec![]); // lay out
        let from = src_rect.get().center();
        let to = zone_rect.get().center();
        run(vec![egui::Event::PointerMoved(from)]); // hover source
        run(vec![egui::Event::PointerButton {
            pos: from,
            button: btn,
            pressed: true,
            modifiers: m,
        }]);
        run(vec![egui::Event::PointerMoved(to)]); // drag across (past threshold)
        run(vec![egui::Event::PointerButton {
            pos: to,
            button: btn,
            pressed: false,
            modifiers: m,
        }]);
        assert_eq!(
            got.get(),
            Some(payload),
            "the drop target received the drag"
        );
    }

    /// Double-clicking empty Project-panel space opens Import, but double-clicking
    /// a row must not — the row (drawn on top) claims the click. Mirrors the
    /// backdrop-under-rows layout `project_panel` uses.
    #[test]
    fn backdrop_double_click_fires_only_off_the_rows() {
        fn scene(pick: impl Fn(egui::Rect, egui::Rect) -> egui::Pos2) -> bool {
            let ctx = egui::Context::default();
            let bg_rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let row_rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let bg_dbl = std::cell::Cell::new(false);
            let run = |events: Vec<egui::Event>| {
                let ri = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(400.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(ri, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let full = ui.available_rect_before_wrap();
                        // Backdrop first (under the row), exactly as project_panel.
                        let bg = ui.interact(full, egui::Id::new("bg"), egui::Sense::click());
                        bg_rect.set(full);
                        let row = draggable_row(ui, egui::Id::new("row"), 1u32, false, "row");
                        row_rect.set(row.rect);
                        if bg.double_clicked() {
                            bg_dbl.set(true);
                        }
                    });
                });
            };
            let m = egui::Modifiers::default();
            let btn = egui::PointerButton::Primary;
            run(vec![]); // lay out
            let pos = pick(bg_rect.get(), row_rect.get());
            run(vec![egui::Event::PointerMoved(pos)]);
            for _ in 0..2 {
                run(vec![egui::Event::PointerButton {
                    pos,
                    button: btn,
                    pressed: true,
                    modifiers: m,
                }]);
                run(vec![egui::Event::PointerButton {
                    pos,
                    button: btn,
                    pressed: false,
                    modifiers: m,
                }]);
            }
            bg_dbl.get()
        }
        // Empty space below the row → Import fires.
        assert!(scene(|bg, row| egui::pos2(
            bg.center().x,
            row.bottom() + 80.0
        )));
        // On the row → the row consumes it; the backdrop stays silent.
        assert!(!scene(|_bg, row| row.center()));
    }

    // The default workspace contains every panel, and the pop-out mechanism
    // (hide the tile, show it again) round-trips — the basis of detaching a
    // panel into its own window and docking it back (K-074).
    #[test]
    fn default_layout_has_every_panel_and_popout_round_trips() {
        let mut tree = default_layout();
        for panel in [
            Panel::Viewer,
            Panel::Project,
            Panel::Timeline,
            Panel::EffectControls,
            Panel::EffectsAndPresets,
            Panel::Scopes(ScopeKind::default()),
            Panel::Hierarchy,
        ] {
            let id = tile_id_of(&tree, panel).expect("panel present in default layout");
            assert!(tree.tiles.is_visible(id), "{panel:?} should start visible");
        }

        let project = tile_id_of(&tree, Panel::Project).unwrap();
        tree.tiles.set_visible(project, false); // pop out
        assert!(!tree.tiles.is_visible(project));
        tree.tiles.set_visible(project, true); // dock back
        assert!(tree.tiles.is_visible(project));
    }

    /// K-092's three new persisted fields (`theme_mode`, `theme_shape`,
    /// `animation_level`) must not break loading a workspace saved before
    /// they existed — an empty JSON object stands in for the oldest
    /// possible save (every persisted `Shell` field already carries
    /// `#[serde(default)]`), and every new field must land on its default.
    #[test]
    fn shell_deserializes_a_pre_k092_save_onto_the_new_fields_defaults() {
        let shell: Shell = serde_json::from_str("{}").expect("an empty save must still load");
        assert_eq!(shell.theme_mode, crate::theme::ThemeMode::Dark);
        assert_eq!(shell.theme_shape, crate::theme::ThemeShape::Sharp);
        assert_eq!(shell.animation_level, crate::theme::AnimationLevel::All);
    }

    #[test]
    fn a_pre_k097_theme_pick_migrates_onto_color_scheme() {
        use crate::theme::{ColorScheme, ThemeMode, ThemeVariant};
        // Old Light / Dark-blue picks survive the upgrade to `ColorScheme`.
        assert_eq!(
            migrated_scheme(ColorScheme::Dark, ThemeMode::Light, ThemeVariant::Dark),
            ColorScheme::Light
        );
        assert_eq!(
            migrated_scheme(ColorScheme::Dark, ThemeMode::Dark, ThemeVariant::DarkBlue),
            ColorScheme::DarkBlue
        );
        assert_eq!(
            migrated_scheme(ColorScheme::Dark, ThemeMode::Dark, ThemeVariant::Dark),
            ColorScheme::Dark
        );
        // A newer save's explicit scheme is never second-guessed by stale
        // legacy fields.
        assert_eq!(
            migrated_scheme(
                ColorScheme::GruvboxDark,
                ThemeMode::Light,
                ThemeVariant::DarkBlue
            ),
            ColorScheme::GruvboxDark
        );
    }

    #[test]
    fn an_open_settings_dialog_counts_as_a_modal() {
        // Gates the active-panel focus edge: while the dialog is up its
        // backdrop owns clicks, so a press must not re-focus a panel behind
        // it (the reported click-through bug).
        assert!(!Shell::default().any_modal_open());
        let shell = Shell {
            settings_open: true,
            ..Shell::default()
        };
        assert!(shell.any_modal_open());
    }

    #[test]
    fn color_scheme_round_trips_through_a_save() {
        // `color_scheme` persists; the legacy mode/variant are read-only
        // (skip_serializing), so a saved-then-loaded Shell keeps its scheme.
        let shell = Shell {
            color_scheme: crate::theme::ColorScheme::CatppuccinMocha,
            ..Shell::default()
        };
        let json = serde_json::to_string(&shell).unwrap();
        let back: Shell = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.color_scheme,
            crate::theme::ColorScheme::CatppuccinMocha
        );
    }

    // The Timeline starts as a full-width strip along the bottom: its pane is
    // a direct child of the vertical root (so it is as wide as the window) and
    // the last child (the bottom band) — with no tab wrapper around it, since a
    // solo panel renders bare (K-086). Guards the default workspace against a
    // regression back to the Timeline nested inside the Viewer's column or
    // re-wrapped in a needless single-tab group.
    #[test]
    fn timeline_starts_full_width_along_the_bottom_as_a_bare_pane() {
        let tree = default_layout();
        let root = tree.root().expect("layout has a root");
        let egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin)) =
            tree.tiles.get(root).expect("root tile exists")
        else {
            panic!("the root should be a vertical linear container");
        };
        assert_eq!(lin.dir, egui_tiles::LinearDir::Vertical);

        let timeline = tile_id_of(&tree, Panel::Timeline).expect("timeline present");
        assert!(
            lin.children.contains(&timeline),
            "timeline pane should be a direct child of the vertical root (full width, no tab wrapper)"
        );
        assert_eq!(
            lin.children.last(),
            Some(&timeline),
            "timeline pane should be the bottom-most child"
        );
    }

    /// True when any tab container holds exactly one child — a lone pane that
    /// would show a needless tab bar (the shape K-086 removes).
    fn has_solo_tab_group(tree: &egui_tiles::Tree<Panel>) -> bool {
        tree.tiles.iter().any(|(_, t)| {
            matches!(t, egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))
                if tabs.children.len() == 1)
        })
    }

    // Solo panels render bare (K-086): the default layout wraps no pane in a
    // single-child tab group, and the simplification pass the dock runs on
    // every draw strips such wrappers from a workspace saved before this rule
    // — a stale persisted layout still loads, just without the lone tabs.
    // Genuine stacks keep their tab bar.
    #[test]
    fn solo_tab_wrappers_are_pruned_and_stacks_keep_their_tabs() {
        assert!(
            !has_solo_tab_group(&default_layout()),
            "default layout should not wrap any lone pane in a tab group"
        );

        // A stale workspace: the pre-K-086 default, with Scopes and the
        // Timeline each wrapped in a single-child tab group.
        let mut tiles = egui_tiles::Tiles::default();
        let viewer = tiles.insert_pane(Panel::Viewer);
        let project = tiles.insert_pane(Panel::Project);
        let fx = tiles.insert_pane(Panel::EffectControls);
        let fxp = tiles.insert_pane(Panel::EffectsAndPresets);
        let left = tiles.insert_tab_tile(vec![project, fx, fxp]);
        let scopes = tiles.insert_pane(Panel::Scopes(ScopeKind::default()));
        let right = tiles.insert_tab_tile(vec![scopes]);
        let upper = tiles.insert_horizontal_tile(vec![left, viewer, right]);
        let timeline = tiles.insert_pane(Panel::Timeline);
        let timeline_tabs = tiles.insert_tab_tile(vec![timeline]);
        let root = tiles.insert_vertical_tile(vec![upper, timeline_tabs]);
        let mut stale = egui_tiles::Tree::new("stale-dock", root, tiles);

        stale.simplify(&dock_simplification_options());

        assert!(
            !has_solo_tab_group(&stale),
            "the dock's simplify pass should prune single-child tab groups"
        );
        // Every panel survives the pruning…
        for panel in [
            Panel::Viewer,
            Panel::Project,
            Panel::Timeline,
            Panel::EffectControls,
            Panel::EffectsAndPresets,
            Panel::Scopes(ScopeKind::default()),
        ] {
            assert!(
                tile_id_of(&stale, panel).is_some(),
                "{panel:?} should survive simplification"
            );
        }
        // …and the genuine three-panel stack keeps its tab group.
        let project_tile = tile_id_of(&stale, Panel::Project).unwrap();
        let in_tabs = stale.tiles.iter().any(|(_, t)| {
            matches!(t, egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))
                if tabs.children.contains(&project_tile))
        });
        assert!(in_tabs, "a stacked panel keeps its tab bar");
    }

    /// `bare_tile_ids` (the set `DockBehavior` wraps in `bare_pane_ui`)
    /// matches tab membership on the real default layout: the three
    /// tab-stacked panels are excluded, the three solo ones are included.
    #[test]
    fn bare_tile_ids_matches_tab_membership_on_the_default_layout() {
        let tree = default_layout();
        let bare = bare_tile_ids(&tree);
        for panel in [
            Panel::Viewer,
            Panel::Timeline,
            Panel::Scopes(ScopeKind::default()),
        ] {
            let id = tile_id_of(&tree, panel).unwrap();
            assert!(bare.contains(&id), "{panel:?} should render bare");
        }
        for panel in [
            Panel::Project,
            Panel::EffectControls,
            Panel::EffectsAndPresets,
        ] {
            let id = tile_id_of(&tree, panel).unwrap();
            assert!(!bare.contains(&id), "{panel:?} is tab-stacked, not bare");
        }
    }

    /// `DockBehavior::gap_width`/`resize_stroke` (K-092): Sharp reproduces
    /// egui_tiles' own idle-state default exactly (a `tab_bar_color`-toned
    /// line at `gap_width`); Round widens the gap and paints its idle state
    /// as the canvas colour instead — `tab_bar_color` itself must stay
    /// untouched by shape (it's also the real tab-bar background for
    /// stacked groups).
    #[test]
    fn dock_behavior_gap_and_resize_stroke_are_shape_aware() {
        use egui_tiles::Behavior as _;
        let style = egui::Style::default();
        let mut app = AppState::default();

        let sharp = Theme::of(crate::theme::ThemeVariant::Dark);
        let mut behavior = DockBehavior {
            theme: &sharp,
            app: &mut app,
            preview_display: None,
            pop_out: None,
            panel_rects: Vec::new(),
            tree_id: egui::Id::new("test"),
            bare_tiles: Default::default(),
        };
        assert_eq!(behavior.gap_width(&style), 1.0_f32);
        assert_eq!(
            behavior.resize_stroke(&style, egui_tiles::ResizeState::Idle),
            egui::Stroke::new(1.0_f32, behavior.tab_bar_color(&style.visuals))
        );
        // Sharp keeps the rerun-style tab-bar fill one step above the panel.
        assert_eq!(behavior.tab_bar_color(&style.visuals), sharp.surface_2);

        let round = crate::theme::Theme::for_settings(
            crate::theme::ThemeMode::Dark,
            crate::theme::ThemeVariant::Dark,
            crate::theme::ThemeShape::Round,
        );
        behavior.theme = &round;
        assert_eq!(behavior.gap_width(&style), round.tokens.tile_gap);
        assert_eq!(
            behavior.resize_stroke(&style, egui_tiles::ResizeState::Idle),
            egui::Stroke::new(round.tokens.tile_gap, round.surface_0)
        );
        // Under Round the tab bar takes the canvas colour so the pill tabs
        // read as floating chips in a strip separated from the body card.
        assert_eq!(
            behavior.tab_bar_color(&style.visuals),
            round.surface_0,
            "Round tab bar should be the canvas colour so pills float in it"
        );
    }

    // The comp strip's "Pop out timeline" menu hangs off the strip's background
    // (a click-sensing Ui registered before the tab buttons, expanded to the
    // panel's right edge). Pins the egui layering it relies on: a right-click
    // on empty strip space reaches the background; a right-click on a tab
    // button does not (the button, drawn on top, claims it).
    #[test]
    fn strip_background_takes_the_right_click_only_off_the_buttons() {
        fn scene(pick: impl Fn(egui::Rect, egui::Rect) -> egui::Pos2) -> bool {
            let ctx = egui::Context::default();
            let bg_rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let btn_rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let bg_secondary = std::cell::Cell::new(false);
            let run = |events: Vec<egui::Event>| {
                let ri = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(400.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(ri, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        // comp_tab_strip in miniature: a sensed background Ui,
                        // a button inside it, the width claimed to the edge.
                        let bg = ui.scope_builder(
                            egui::UiBuilder::new().sense(egui::Sense::click()),
                            |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    btn_rect.set(ui.button("Comp 1").rect);
                                });
                                let claim = egui::Rect::from_min_max(
                                    ui.min_rect().left_top(),
                                    egui::pos2(ui.max_rect().right(), ui.min_rect().bottom()),
                                );
                                ui.expand_to_include_rect(claim);
                            },
                        );
                        bg_rect.set(bg.response.rect);
                        if bg.response.secondary_clicked() {
                            bg_secondary.set(true);
                        }
                    });
                });
            };
            let m = egui::Modifiers::default();
            let btn = egui::PointerButton::Secondary;
            run(vec![]); // lay out twice so the background's rect has settled
            run(vec![]);
            let pos = pick(bg_rect.get(), btn_rect.get());
            run(vec![egui::Event::PointerMoved(pos)]);
            run(vec![egui::Event::PointerButton {
                pos,
                button: btn,
                pressed: true,
                modifiers: m,
            }]);
            run(vec![egui::Event::PointerButton {
                pos,
                button: btn,
                pressed: false,
                modifiers: m,
            }]);
            bg_secondary.get()
        }
        // Empty space right of the tab → the background sees the right-click.
        assert!(scene(|bg, btn| egui::pos2(
            (btn.right() + bg.right()) * 0.5,
            btn.center().y
        )));
        // On the tab button → the button wins; the background stays silent.
        assert!(!scene(|_bg, btn| btn.center()));
    }

    /// `bare_pane_ui`'s right-click affordance (owner request, K-091 era): a
    /// bare pane's whole rect senses right-click for "pop out into its own
    /// window", registered before the panel's own content — mirroring
    /// `strip_background_takes_the_right_click_only_off_the_buttons`. A
    /// button the content draws anywhere in the pane must still claim
    /// right-clicks over its own footprint.
    #[test]
    fn bare_pane_background_right_click_pops_out_only_off_content() {
        fn scene(pick: impl Fn(egui::Rect, egui::Rect) -> egui::Pos2) -> bool {
            let ctx = egui::Context::default();
            let bg_rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let btn_rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let bg_secondary = std::cell::Cell::new(false);
            let run = |events: Vec<egui::Event>| {
                let ri = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(400.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(ri, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        // bare_pane_ui in miniature: the whole-pane background
                        // scope, one button drawn inside it as its content.
                        let bg = ui.scope_builder(
                            egui::UiBuilder::new().sense(egui::Sense::click()),
                            |ui| {
                                let pane_rect = ui.max_rect();
                                btn_rect.set(ui.button("content").rect);
                                ui.expand_to_include_rect(pane_rect);
                            },
                        );
                        bg_rect.set(bg.response.rect);
                        if bg.response.secondary_clicked() {
                            bg_secondary.set(true);
                        }
                    });
                });
            };
            let m = egui::Modifiers::default();
            let btn = egui::PointerButton::Secondary;
            run(vec![]); // lay out twice so the background's rect has settled
            run(vec![]);
            let pos = pick(bg_rect.get(), btn_rect.get());
            run(vec![egui::Event::PointerMoved(pos)]);
            run(vec![egui::Event::PointerButton {
                pos,
                button: btn,
                pressed: true,
                modifiers: m,
            }]);
            run(vec![egui::Event::PointerButton {
                pos,
                button: btn,
                pressed: false,
                modifiers: m,
            }]);
            bg_secondary.get()
        }
        // Empty pane space below the button → the background pops out.
        assert!(scene(|bg, btn| egui::pos2(
            btn.center().x,
            (btn.bottom() + bg.bottom()) * 0.5
        )));
        // On the content button → the button wins; the background stays silent.
        assert!(!scene(|_bg, btn| btn.center()));
    }

    /// `bare_pane_ui`'s drag grip: a small top-right handle senses
    /// `click_and_drag` and, on `drag_started`, hands off to egui_tiles' own
    /// tile-drag id (`TileId::egui_id`) so the pane re-docks like a dragged
    /// tab. It is interacted *after* the content (mirroring how the
    /// right-click background's content wins its own footprint above), so a
    /// widget the panel draws underneath the grip's corner keeps its clicks
    /// everywhere *else*, and the grip still claims drags starting in its
    /// own tiny footprint. This is deliberately NOT a drag sense spread over
    /// a wider region: an earlier version tried exactly that (a top-strip
    /// `click_and_drag` interact registered *before* content) and a plain
    /// button drawn inside it had its click hijacked into a pane-drag once
    /// the pointer moved past the click threshold — `Response::dragged()`
    /// only needs the *sense*, not being topmost, so a click-only sibling
    /// has nothing to contest a drag-sensing one with (confirmed against
    /// egui_tiles' own tab bar, whose background AND its individual tab
    /// buttons both sense `click_and_drag` — that symmetry is what lets one
    /// yield to the other; a plain button can't).
    #[test]
    fn bare_pane_drag_grip_moves_the_pane_only_from_its_own_corner() {
        fn scene(press_pos: impl Fn(egui::Rect, egui::Rect) -> egui::Pos2) -> (bool, bool) {
            let ctx = egui::Context::default();
            let content_rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let grip_rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let content_clicked = std::cell::Cell::new(false);
            let tree_id = egui::Id::new("test-dock");
            let tile_id = egui_tiles::TileId::from_u64(7);
            let expect_id = tile_id.egui_id(tree_id);
            let dragged = std::cell::Cell::new(false);
            let run = |events: Vec<egui::Event>| {
                let ri = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(400.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(ri, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let rect = ui.max_rect();
                        // Content: a plain button spanning the corner where
                        // the grip will sit, mirroring a panel whose header
                        // reaches the same area (Project's toolbar row).
                        let content = ui.put(
                            egui::Rect::from_min_size(
                                rect.left_top(),
                                egui::vec2(200.0, BARE_PANE_GRIP_SIZE),
                            ),
                            egui::Button::new("content"),
                        );
                        content_rect.set(content.rect);
                        if content.clicked() {
                            content_clicked.set(true);
                        }
                        // The grip, added last exactly as bare_pane_ui does.
                        let grip = egui::Rect::from_min_size(
                            egui::pos2(rect.right() - BARE_PANE_GRIP_SIZE, rect.top()),
                            egui::vec2(BARE_PANE_GRIP_SIZE, BARE_PANE_GRIP_SIZE),
                        );
                        grip_rect.set(grip);
                        let handle = ui.interact(
                            grip,
                            ui.id().with("bare-pane-grip"),
                            egui::Sense::click_and_drag(),
                        );
                        if handle.drag_started() {
                            ui.ctx().set_dragged_id(expect_id);
                        }
                        dragged.set(ctx.is_being_dragged(expect_id));
                    });
                });
            };
            let m = egui::Modifiers::default();
            let btn = egui::PointerButton::Primary;
            run(vec![]);
            run(vec![]);
            let pos = press_pos(content_rect.get(), grip_rect.get());
            run(vec![egui::Event::PointerMoved(pos)]);
            run(vec![egui::Event::PointerButton {
                pos,
                button: btn,
                pressed: true,
                modifiers: m,
            }]);
            // Move past the drag threshold while held (a plain press+release
            // at the same spot is a click, not a drag — same distinction
            // `dragging_a_row_delivers_its_payload_to_a_drop_target` relies on).
            let moved = pos + egui::vec2(8.0, 0.0);
            run(vec![egui::Event::PointerMoved(moved)]);
            let got_dragged = dragged.get();
            run(vec![egui::Event::PointerButton {
                pos: moved,
                button: btn,
                pressed: false,
                modifiers: m,
            }]);
            (content_clicked.get(), got_dragged)
        }
        // Dragging from the grip's own corner starts the tile drag.
        let (_, dragged) = scene(|_content, grip| grip.center());
        assert!(dragged, "dragging the grip should start the tile drag");
        // Dragging from elsewhere on the content (away from the grip
        // corner) must not — the button there keeps its own interaction.
        let (clicked, dragged) =
            scene(|content, _grip| content.left_center() + egui::vec2(20.0, 0.0));
        assert!(!dragged, "dragging the content must not hijack a pane-drag");
        assert!(!clicked, "a drag, even off the grip, is not a click");
    }

    // Each keyframe's glyph codes its interpolation (graph-editor ergonomics).
    #[test]
    fn key_shape_codes_interpolation() {
        use lumit_core::anim::{Keyframe, SideInterp};
        let key = |i: SideInterp, o: SideInterp| Keyframe {
            time: rational_at(0.0),
            value: 0.0,
            interp_in: i,
            interp_out: o,
        };
        assert_eq!(
            key_shape(&key(SideInterp::Linear, SideInterp::Linear)),
            KeyShape::Diamond
        );
        assert_eq!(
            key_shape(&key(SideInterp::Hold, SideInterp::Linear)),
            KeyShape::Square
        );
        assert_eq!(
            key_shape(&key(
                SideInterp::Linear,
                SideInterp::Bezier {
                    speed: 0.0,
                    influence: 0.33
                }
            )),
            KeyShape::Circle
        );
        // Hold wins over bezier (a held key never eases out visually).
        assert_eq!(
            key_shape(&key(
                SideInterp::Hold,
                SideInterp::Bezier {
                    speed: 0.0,
                    influence: 0.33
                }
            )),
            KeyShape::Square
        );
    }

    // The time grid subdivides with zoom: gridlines never crowd under ~70 px,
    // and zooming in walks the ladder down to 10 ms.
    #[test]
    fn time_grid_step_follows_the_zoom() {
        assert_eq!(time_grid_step(10.0), 10.0); // zoomed out: 10 s lines
        assert_eq!(time_grid_step(80.0), 1.0); // ~normal: whole seconds
        assert_eq!(time_grid_step(300.0), 0.25); // zoomed: quarter seconds
        assert_eq!(time_grid_step(10_000.0), 0.01); // way in: 10 ms
                                                    // Never denser than ~70 px between lines.
        for pps in [10.0, 80.0, 300.0, 10_000.0] {
            assert!(time_grid_step(pps) * pps >= 70.0 || time_grid_step(pps) == 0.01);
        }
    }

    // The linked scale control keeps the x:y ratio (K-072).
    #[test]
    fn linked_scale_keeps_ratio() {
        assert_eq!(linked_scale(100.0, 50.0, 200.0), (200.0, 100.0)); // 2:1 kept
        assert_eq!(linked_scale(100.0, 100.0, 150.0), (150.0, 150.0)); // 1:1 kept
        assert_eq!(linked_scale(0.0, 50.0, 80.0), (80.0, 80.0)); // undefined → uniform
    }

    /// A keyframed test property (linear keys at the given (time, value)s).
    fn keyed(keys: &[(f64, f64)]) -> lumit_core::anim::Property {
        use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
        let mut p = Property::fixed(0.0);
        p.animation = Animation::Keyframed(
            keys.iter()
                .map(|(t, v)| Keyframe {
                    time: rational_at(*t),
                    value: *v,
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                })
                .collect(),
        );
        p
    }

    // Every linked two-axis row (Scale, Position, Anchor) commits both axes
    // as ONE undo step: a Batch of two SetTransformProperty ops, x then y,
    // each addressing its own channel — values stay independent.
    #[test]
    fn two_prop_batch_sets_both_axes_in_one_undo_step() {
        use lumit_core::anim::Animation;
        use lumit_core::model::TransformProp;
        let comp = uuid::Uuid::from_u128(0xC0);
        let layer = uuid::Uuid::from_u128(0x1A);
        let op = two_prop_batch(
            comp,
            layer,
            (TransformProp::PositionX, Animation::Static(10.0)),
            (TransformProp::PositionY, Animation::Static(20.0)),
        );
        assert_eq!(
            op,
            lumit_core::Op::Batch {
                ops: vec![
                    lumit_core::Op::SetTransformProperty {
                        comp,
                        layer,
                        prop: TransformProp::PositionX,
                        animation: Animation::Static(10.0),
                    },
                    lumit_core::Op::SetTransformProperty {
                        comp,
                        layer,
                        prop: TransformProp::PositionY,
                        animation: Animation::Static(20.0),
                    },
                ],
            }
        );
    }

    // The linked row's navigator works the union of both axes' keys: times
    // merge sorted, near-coincident keys count once, static axes add nothing.
    #[test]
    fn union_key_times_merges_sorted_and_dedupes() {
        use lumit_core::anim::Property;
        let tol = 0.5 / 30.0; // half a frame at 30 fps
        let x = keyed(&[(0.0, 1.0), (2.0, 3.0)]);
        let y = keyed(&[(1.0, 5.0), (2.001, 6.0)]); // 2.001 ≈ 2.0 within tol
        let times = union_key_times(&x, &y, tol);
        assert_eq!(times.len(), 3);
        assert!((times[0] - 0.0).abs() < 1e-9);
        assert!((times[1] - 1.0).abs() < 1e-9);
        assert!((times[2] - 2.0).abs() < 1e-3);
        // A static axis contributes nothing; two statics mean no navigator.
        let s = Property::fixed(7.0);
        assert_eq!(union_key_times(&x, &s, tol).len(), 2);
        assert!(union_key_times(&s, &s, tol).is_empty());
    }

    // Walking that union from the playhead: previous strictly before, next
    // strictly after, and "on a key" within the half-frame tolerance.
    #[test]
    fn key_nav_targets_walks_the_union() {
        let tol = 0.5 / 30.0;
        let times = [0.0, 1.0, 2.0];
        // On the middle key: prev is 0, next is 2.
        let (prev, on, next) = key_nav_targets(&times, 1.0, tol);
        assert_eq!(prev, Some(0.0));
        assert!(on);
        assert_eq!(next, Some(2.0));
        // Between keys: nearest each side, not "on" anything.
        let (prev, on, next) = key_nav_targets(&times, 0.5, tol);
        assert_eq!(prev, Some(0.0));
        assert!(!on);
        assert_eq!(next, Some(1.0));
        // At the ends there is nowhere further to go.
        let (prev, _, _) = key_nav_targets(&times, 0.0, tol);
        assert_eq!(prev, None);
        let (_, _, next) = key_nav_targets(&times, 2.0, tol);
        assert_eq!(next, None);
    }

    // The linked diamond's per-axis toggle: adding upserts a key at the
    // playhead on each axis; removing strips only keys at the playhead,
    // freezing an axis when its last key goes and never touching a static one.
    #[test]
    fn toggle_key_at_keys_or_clears_one_axis() {
        use lumit_core::anim::{Animation, Property};
        let tol = 0.5 / 30.0;
        // Add on an animated axis: the playhead key joins the existing ones.
        let x = keyed(&[(0.0, 1.0), (2.0, 3.0)]);
        let Animation::Keyframed(keys) = toggle_key_at(&x, 1.0, tol, false) else {
            panic!("adding must keep the axis keyframed");
        };
        assert_eq!(keys.len(), 3);
        assert!((keys[1].time.to_f64() - 1.0).abs() < 1e-6);
        assert!((keys[1].value - 2.0).abs() < 1e-6); // the interpolated value
                                                     // Add on a static axis: it becomes keyframed at its current value.
        let s = Property::fixed(7.0);
        let Animation::Keyframed(keys) = toggle_key_at(&s, 1.0, tol, false) else {
            panic!("adding must animate a static axis");
        };
        assert!(keys.iter().any(|k| (k.time.to_f64() - 1.0).abs() < 1e-6));
        assert!(keys.iter().all(|k| (k.value - 7.0).abs() < 1e-9));
        // Remove at a key: only that key goes, the others stay.
        let Animation::Keyframed(keys) = toggle_key_at(&x, 2.0, tol, true) else {
            panic!("an axis with keys left must stay keyframed");
        };
        assert_eq!(keys.len(), 1);
        assert!((keys[0].time.to_f64()).abs() < 1e-9);
        // Removing the last key freezes the axis at its current value.
        let one = keyed(&[(1.0, 4.0)]);
        assert_eq!(toggle_key_at(&one, 1.0, tol, true), Animation::Static(4.0));
        // A static axis is left untouched by a union-driven remove.
        assert_eq!(toggle_key_at(&s, 1.0, tol, true), Animation::Static(7.0));
    }

    // A keyframe side reports its bezier influence, or the easy-ease third.
    #[test]
    fn side_influence_reads_bezier_or_defaults() {
        use lumit_core::anim::SideInterp;
        assert_eq!(
            side_influence(SideInterp::Bezier {
                speed: 5.0,
                influence: 0.5
            }),
            0.5
        );
        assert!((side_influence(SideInterp::Linear) - 1.0 / 3.0).abs() < 1e-9);
        assert!((side_influence(SideInterp::Hold) - 1.0 / 3.0).abs() < 1e-9);
    }

    // The tangent drag's mirroring: decided at drag start from the key's
    // unification, toggled once by Alt (latched — releasing Alt mid-drag never
    // snaps handles back together), and applied by apply_tangent (Mack).
    #[test]
    fn tangent_drag_unifies_by_default_and_alt_toggles_latched() {
        use lumit_core::anim::{Keyframe, SideInterp::Bezier, EASY_EASE};
        // The mode table: unified stays unified, Alt breaks it — and the break
        // survives Alt being released (alt_seen latches). A broken key stays
        // broken on a plain drag; Alt on a broken key re-unifies it.
        assert!(tangent_mirrors(true, false)); // unified, no Alt → mirror
        assert!(!tangent_mirrors(true, true)); // unified, Alt seen → broken
        assert!(!tangent_mirrors(false, false)); // broken, no Alt → stays broken
        assert!(tangent_mirrors(false, true)); // broken, Alt seen → re-unified

        let base = || Keyframe {
            time: rational_at(1.0),
            value: 0.0,
            interp_in: EASY_EASE, // speed 0, influence 1/3
            interp_out: EASY_EASE,
        };
        // Mirroring drag of the out handle sets both slopes; reaches preserved.
        let mut k = base();
        apply_tangent(&mut k, true, 5.0, 0.5, tangent_mirrors(true, false), None);
        assert_eq!(side_speed(k.interp_out), Some(5.0));
        assert_eq!(side_speed(k.interp_in), Some(5.0)); // mirrored
        assert!((side_influence(k.interp_out) - 0.5).abs() < 1e-9);
        assert!((side_influence(k.interp_in) - 1.0 / 3.0).abs() < 1e-9); // in reach kept
                                                                         // Alt seen during the drag breaks: only the dragged side changes.
        let mut k = base();
        apply_tangent(&mut k, true, 5.0, 0.5, tangent_mirrors(true, true), None);
        assert_eq!(side_speed(k.interp_out), Some(5.0));
        assert_eq!(side_speed(k.interp_in), Some(0.0)); // untouched
        let broken = || Keyframe {
            interp_in: Bezier {
                speed: 2.0,
                influence: 1.0 / 3.0,
            },
            interp_out: Bezier {
                speed: -3.0,
                influence: 1.0 / 3.0,
            },
            ..base()
        };
        // A broken key stays broken on a plain drag…
        let mut k = broken();
        apply_tangent(&mut k, false, 9.0, 0.4, tangent_mirrors(false, false), None);
        assert_eq!(side_speed(k.interp_in), Some(9.0));
        assert_eq!(side_speed(k.interp_out), Some(-3.0)); // stays broken
                                                          // …and an Alt-drag on it re-unifies: both sides take the dragged slope.
        let mut k = broken();
        apply_tangent(&mut k, false, 9.0, 0.4, tangent_mirrors(false, true), None);
        assert_eq!(side_speed(k.interp_in), Some(9.0));
        assert_eq!(side_speed(k.interp_out), Some(9.0)); // re-unified
        assert!((side_influence(k.interp_out) - 1.0 / 3.0).abs() < 1e-9); // own reach kept
    }

    // Vertical wheel maths (K-079): a plain wheel pans (span kept, view shifts);
    // Ctrl-wheel zooms about the cursor value (cursor pinned, span changes).
    #[test]
    fn graph_vertical_pan_and_zoom() {
        // Pan: span stays 10, wheel-up shifts the whole range up by dy/height·span.
        let (lo, hi) = graph_v_pan_zoom((0.0, 10.0), 20.0, false, 5.0, 200.0);
        assert!(((hi - lo) - 10.0).abs() < 1e-9); // span preserved
        assert!((lo - 1.0).abs() < 1e-9 && (hi - 11.0).abs() < 1e-9); // shifted +1
                                                                      // Zoom in about the cursor value 5 (wheel up): cursor stays, span shrinks.
        let (zlo, zhi) = graph_v_pan_zoom((0.0, 10.0), 100.0, true, 5.0, 200.0);
        assert!(zhi - zlo < 10.0); // zoomed in
        let cursor_frac = (5.0 - zlo) / (zhi - zlo);
        assert!((cursor_frac - 0.5).abs() < 1e-9); // cursor value pinned in view
    }

    // The auto-fit reads tangent-handle endpoints, not just key values: a flat
    // two-key curve with a steep out-handle must widen the range past the keys,
    // and an in-handle widens it the other way (endpoint = v ± speed·reach).
    #[test]
    fn fit_includes_tangent_handle_endpoints() {
        use lumit_core::anim::{Keyframe, SideInterp};
        let key = |t: f64, i: SideInterp, o: SideInterp| Keyframe {
            time: rational_at(t),
            value: 10.0,
            interp_in: i,
            interp_out: o,
        };
        let steep = SideInterp::Bezier {
            speed: 60.0,
            influence: 0.5,
        };
        // Flat pair of keys at 10, first key's out-handle climbing at 60 u/s
        // over a reach of 0.5 · 2 s: its endpoint sits at 10 + 60·1 = 70.
        let keys = vec![
            key(0.0, SideInterp::Linear, steep),
            key(2.0, SideInterp::Linear, SideInterp::Linear),
        ];
        let (lo, hi) = fit_values_with_handles(&keys);
        assert!((lo - 10.0).abs() < 1e-9, "flat keys floor the range: {lo}");
        assert!((hi - 70.0).abs() < 1e-9, "out-handle endpoint missed: {hi}");
        // The same handle on the second key's *in* side reaches backwards and
        // downwards: endpoint 10 − 60·1 = −50.
        let keys = vec![
            key(0.0, SideInterp::Linear, SideInterp::Linear),
            key(2.0, steep, SideInterp::Linear),
        ];
        let (lo, hi) = fit_values_with_handles(&keys);
        assert!(
            (lo - (-50.0)).abs() < 1e-9,
            "in-handle endpoint missed: {lo}"
        );
        assert!((hi - 10.0).abs() < 1e-9);
        // A bezier side with no neighbour grows no handle: the last key's
        // out-side (and the first key's in-side) never widen the fit.
        let keys = vec![
            key(0.0, steep, SideInterp::Linear),
            key(2.0, SideInterp::Linear, steep),
        ];
        assert_eq!(fit_values_with_handles(&keys), (10.0, 10.0));
        // Linear keys alone reduce to the plain value min/max.
        let mut keys = vec![
            key(0.0, SideInterp::Linear, SideInterp::Linear),
            key(2.0, SideInterp::Linear, SideInterp::Linear),
        ];
        keys[1].value = 25.0;
        assert_eq!(fit_values_with_handles(&keys), (10.0, 25.0));
    }

    // A manual y-range answers a panel resize by keeping its value scale:
    // the range grows or shrinks about its centre by the height ratio, so
    // units-per-pixel hold and more height shows more curve, not a stretch.
    #[test]
    fn manual_range_rescales_with_plot_height() {
        // Doubling the height doubles the span about the same centre.
        let (lo, hi) = rescale_range_for_height((0.0, 10.0), 100.0, 200.0);
        assert!((lo - (-5.0)).abs() < 1e-9 && (hi - 15.0).abs() < 1e-9);
        assert!(((lo + hi) * 0.5 - 5.0).abs() < 1e-9); // centre preserved
        assert!(((hi - lo) / 200.0 - 10.0 / 100.0).abs() < 1e-9); // units/px held
                                                                  // Shrinking the plot narrows the span symmetrically.
        let (slo, shi) = rescale_range_for_height((0.0, 10.0), 200.0, 100.0);
        assert!((slo - 2.5).abs() < 1e-9 && (shi - 7.5).abs() < 1e-9);
        // Degenerate heights leave the range untouched.
        assert_eq!(
            rescale_range_for_height((0.0, 10.0), 0.0, 100.0),
            (0.0, 10.0)
        );
        assert_eq!(
            rescale_range_for_height((0.0, 10.0), 100.0, 0.0),
            (0.0, 10.0)
        );
    }

    // A unified partner handle rotates but keeps its on-screen length when the
    // dragged side steepens: partner_influence trades reach for slope so the
    // pixel length reach·√(sx²+speed²·sy²) is conserved (Mack, bezier #2).
    #[test]
    fn partner_influence_preserves_screen_length() {
        use lumit_core::anim::SideInterp::Bezier;
        let (sx, sy, seg) = (3.0, 5.0, 2.0);
        let screen_len = |inf: f64, sp: f64| inf * seg * (sx * sx + sp * sp * sy * sy).sqrt();
        // Partner at rest (flat), dragged side goes to a steep slope.
        let partner = Bezier {
            speed: 0.0,
            influence: 1.0 / 3.0,
        };
        let before = screen_len(side_influence(partner), 0.0);
        let inf_new = partner_influence(partner, seg, 8.0, sx, sy);
        let after = screen_len(inf_new, 8.0);
        assert!((before - after).abs() < 1e-9, "{before} vs {after}");
        // A degenerate segment leaves the influence untouched.
        assert_eq!(partner_influence(partner, 0.0, 8.0, sx, sy), 1.0 / 3.0);
    }

    // K-070: setting a key's speed (what a speed-lens drag commits — both sides
    // to Bezier{speed}) is what the derivative reads back. Guards the lossless
    // round-trip promised for the speed lens.
    #[test]
    fn setting_key_speed_round_trips_through_the_derivative() {
        use lumit_core::anim::{evaluate, Keyframe, SideInterp};
        let target = 40.0_f64; // value-units per second at the middle key
        let side = SideInterp::Bezier {
            speed: target,
            influence: 1.0 / 3.0,
        };
        let keys = vec![
            Keyframe {
                time: rational_at(0.0),
                value: 0.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
            Keyframe {
                time: rational_at(1.0),
                value: 50.0,
                interp_in: side,
                interp_out: side,
            },
            Keyframe {
                time: rational_at(2.0),
                value: 60.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
        ];
        let h = 1.0 / 1000.0;
        let a = evaluate(&keys, 1.0 - h).unwrap();
        let b = evaluate(&keys, 1.0 + h).unwrap();
        let measured = (b - a) / (2.0 * h);
        assert!(
            (measured - target).abs() < 1.0,
            "derivative at the key was {measured}, expected ≈ {target}"
        );
    }

    // The Retime value lens reads source position as HH:MM:SS:FF frame timecode
    // (K-075); the frame field wraps at the source fps.
    #[test]
    fn frame_timecode_formats_and_wraps() {
        assert_eq!(fmt_timecode_frames(0.0, 25.0), "00:00:00:00");
        assert_eq!(fmt_timecode_frames(2.0, 30.0), "00:00:02:00");
        assert_eq!(fmt_timecode_frames(1.0 / 25.0, 25.0), "00:00:00:01");
        // Frame field wraps at fps: 24/25 s is frame 24; the 25th rolls the second.
        assert_eq!(fmt_timecode_frames(24.0 / 25.0, 25.0), "00:00:00:24");
        assert_eq!(fmt_timecode_frames(1.0, 25.0), "00:00:01:00");
        // Hours / minutes / seconds compose.
        assert_eq!(fmt_timecode_frames(3661.0, 24.0), "01:01:01:00");
    }

    // The value-lens timecode parser is the inverse of the formatter, and
    // tolerates shorter colon forms and a bare frame count.
    #[test]
    fn timecode_parses_and_round_trips_with_the_formatter() {
        // Full HH:MM:SS:FF → the same frame count the formatter came from.
        assert_eq!(parse_timecode_frames("00:00:02:00", 30.0), Some(60.0));
        assert_eq!(parse_timecode_frames("01:01:01:00", 24.0), Some(87864.0));
        // Shorter colon forms and a bare frame count.
        assert_eq!(parse_timecode_frames("02:14", 25.0), Some(64.0)); // SS:FF
        assert_eq!(parse_timecode_frames("1:00:00", 30.0), Some(1800.0)); // MM:SS:FF
        assert_eq!(parse_timecode_frames("72", 24.0), Some(72.0)); // frames
                                                                   // Round-trips through the formatter for a spread of frame counts.
        for &(frames, fps) in &[(0.0, 25.0), (1.0, 24.0), (64.0, 25.0), (87864.0, 24.0)] {
            let s = fmt_timecode_frames(frames / fps, fps);
            assert_eq!(parse_timecode_frames(&s, fps), Some(frames), "{s} @ {fps}");
        }
        // Rubbish yields None so the drag value keeps its previous reading.
        assert_eq!(parse_timecode_frames("nope", 24.0), None);
        assert_eq!(parse_timecode_frames("", 24.0), None);
    }

    // Enabling the Time lens keyframe seeds identity endpoints plus a key at the
    // playhead, and the resulting store passes through every value key exactly.
    #[test]
    fn value_key_upsert_builds_a_passthrough_with_a_playhead_key() {
        use lumit_core::retime::Retime;
        use lumit_core::Rational;
        let dur = Rational::new(4, 1).unwrap();
        let mut keys = vec![(Rational::ZERO, Rational::ZERO), (dur, dur)];
        // At 24 fps, a playhead at 1.0 s over an identity clip keys source 1.0 s.
        upsert_value_key(&mut keys, 1.0, 1.0, dur, 24.0, 24.0);
        assert_eq!(keys.len(), 3);
        let r = Retime::from_value_keyframes(&keys).unwrap();
        assert!((r.evaluate(1.0) - 1.0).abs() < 1e-9);
        // Re-keying the same frame replaces rather than duplicates it.
        upsert_value_key(&mut keys, 1.0, 2.0, dur, 24.0, 24.0);
        assert_eq!(keys.len(), 3);
        let r = Retime::from_value_keyframes(&keys).unwrap();
        assert!((r.evaluate(1.0) - 2.0).abs() < 1e-9);
    }

    // The value lens counts source frames at the footage's rate, not the comp's:
    // source time snaps to the footage grid, and the timecode's frame field wraps
    // and pads to that rate (600 fps → frames 0..599, three digits).
    #[test]
    fn value_lens_uses_the_source_frame_rate() {
        use lumit_core::retime::Retime;
        use lumit_core::Rational;
        let dur = Rational::new(2, 1).unwrap();
        let mut keys = vec![(Rational::ZERO, Rational::ZERO), (dur, dur)];
        // Comp at 30 fps, footage at 600 fps: a playhead 0.1 s in, keying source
        // 0.105 s, snaps source to the 600-grid (exactly 63/600 s = frame 63).
        upsert_value_key(&mut keys, 0.1, 0.105, dur, 30.0, 600.0);
        let interior = keys.iter().find(|(t, _)| *t != Rational::ZERO && *t != dur);
        let (_t, s) = interior.expect("interior key");
        assert_eq!(*s, Rational::new(63, 600).unwrap());
        assert!(Retime::from_value_keyframes(&keys).is_some());
        // The timecode reads that as frame 63, three digits wide at 600 fps.
        assert_eq!(fmt_timecode_frames(63.0 / 600.0, 600.0), "00:00:00:063");
        // A 1000 fps clip pads the frame field to four digits.
        assert_eq!(fmt_timecode_frames(5.0 / 1000.0, 1000.0), "00:00:00:0005");
    }

    // Regression: enabling Time keyframes with the playhead at the layer's very
    // start or end re-pins an endpoint rather than adding an interior key — the
    // store must still build (the stopwatch lights and the first/last keys
    // show), not silently no-op.
    #[test]
    fn value_key_upsert_at_the_endpoints_still_builds() {
        use lumit_core::retime::Retime;
        use lumit_core::Rational;
        let dur = Rational::new(4, 1).unwrap();
        // Playhead at t = 0 on an un-retimed layer: keys stay the endpoint pair.
        let mut keys = vec![(Rational::ZERO, Rational::ZERO), (dur, dur)];
        upsert_value_key(&mut keys, 0.0, 0.0, dur, 24.0, 24.0);
        assert_eq!(keys.len(), 2);
        let r = Retime::from_value_keyframes(&keys).unwrap();
        assert!((r.evaluate(2.0) - 2.0).abs() < 1e-9); // identity pass-through
                                                       // Playhead at t = dur (and past it — upsert clamps): same story.
        let mut keys = vec![(Rational::ZERO, Rational::ZERO), (dur, dur)];
        upsert_value_key(&mut keys, 5.0, 4.0, dur, 24.0, 24.0);
        assert_eq!(keys.len(), 2);
        assert!(Retime::from_value_keyframes(&keys).is_some());
    }

    // K-075 2b: dragging a speed keyframe in the % lens (via speed_with_key)
    // authors a ramp — the speed set is the speed read back, and the segment
    // start is pinned (K-070 frame-pinning: only downstream recomputes).
    #[test]
    fn retime_speed_keyframe_edit_round_trips() {
        use lumit_core::retime::Retime;
        use lumit_core::Rational;
        let dur = Rational::from_f64_on_grid(2.0, 1000).unwrap();
        let base = Some(Retime::constant_speed(dur, Rational::ZERO, Rational::ONE));
        // Drag the end keyframe (t = 2 s) to 50% — a 100% → 50% ramp.
        let speed = Rational::from_f64_on_grid(0.5, 1000).unwrap();
        let edited = speed_with_key(&base, dur, 2.0, speed).expect("retime rebuilds");
        let end = edited.speed_at(2.0 - 1e-6) * 100.0;
        assert!((end - 50.0).abs() < 1.0, "end speed {end} ≈ 50");
        let start = edited.speed_at(1e-6) * 100.0;
        assert!((start - 100.0).abs() < 1.0, "start speed {start} ≈ 100");
    }

    // Select-on-edit: committing a property or Retime op from the timeline or
    // graph points the graph at the channel that was just touched, so the curve
    // follows the key you just added or moved.
    #[test]
    fn follow_edit_points_the_graph_at_the_touched_channel() {
        use lumit_core::anim::Animation;
        use lumit_core::model::TransformProp;
        let comp = uuid::Uuid::from_u128(0xC0);
        let layer = uuid::Uuid::from_u128(0x1A);
        let mut app = AppState::default();

        // A Retime edit selects the layer and graphs the Speed channel.
        follow_edit(
            &mut app,
            &lumit_core::Op::SetLayerRetime {
                comp,
                layer,
                retime: None,
            },
        );
        assert_eq!(app.selected_layer, Some(layer));
        assert!(app.graph_retime);

        // A transform-property edit swaps the graph to that property.
        follow_edit(
            &mut app,
            &lumit_core::Op::SetTransformProperty {
                comp,
                layer,
                prop: TransformProp::Rotation,
                animation: Animation::Static(0.0),
            },
        );
        assert_eq!(app.selected_layer, Some(layer));
        assert_eq!(app.graph_prop, Some(TransformProp::Rotation));
        assert!(!app.graph_retime);

        // A Batch follows its first property op (linked scale leads with x).
        follow_edit(
            &mut app,
            &lumit_core::Op::Batch {
                ops: vec![
                    lumit_core::Op::SetTransformProperty {
                        comp,
                        layer,
                        prop: TransformProp::ScaleX,
                        animation: Animation::Static(100.0),
                    },
                    lumit_core::Op::SetTransformProperty {
                        comp,
                        layer,
                        prop: TransformProp::ScaleY,
                        animation: Animation::Static(100.0),
                    },
                ],
            },
        );
        assert_eq!(app.graph_prop, Some(TransformProp::ScaleX));

        // Ops that touch neither kind of channel leave the graph alone.
        follow_edit(
            &mut app,
            &lumit_core::Op::RenameLayer {
                comp,
                layer: uuid::Uuid::from_u128(0x2B),
                name: "other".into(),
            },
        );
        assert_eq!(app.selected_layer, Some(layer));
        assert_eq!(app.graph_prop, Some(TransformProp::ScaleX));
        assert!(!app.graph_retime);
    }

    // The y-axis labels: decimals adapt to the axis span, and the unit comes
    // from the property (per cent, degrees, bare for the pixel properties).
    #[test]
    fn y_axis_labels_format_to_span_and_unit() {
        assert_eq!(fmt_axis_value(150.0, 300.0), "150");
        assert_eq!(fmt_axis_value(1.25, 5.0), "1.2");
        assert_eq!(fmt_axis_value(0.347, 0.5), "0.35");
        use lumit_core::model::TransformProp as P;
        assert_eq!(prop_unit(P::Opacity), "%");
        assert_eq!(prop_unit(P::ScaleX), "%");
        assert_eq!(prop_unit(P::Rotation), "°");
        assert_eq!(prop_unit(P::PositionX), "");
    }

    /// A keyframe at (t, v) with linear sides, for the marquee tests.
    fn marquee_key(t: f64, v: f64) -> lumit_core::anim::Keyframe {
        use lumit_core::anim::{Keyframe, SideInterp};
        Keyframe {
            time: rational_at(t),
            value: v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        }
    }

    // Marquee selection: exactly the plotted points inside the band are hit.
    #[test]
    fn marquee_selects_only_the_points_inside_the_band() {
        let points = vec![
            egui::pos2(10.0, 10.0),
            egui::pos2(50.0, 50.0),
            egui::pos2(90.0, 10.0),
        ];
        let band = egui::Rect::from_min_max(egui::pos2(40.0, 40.0), egui::pos2(60.0, 60.0));
        assert_eq!(keys_in_band(&points, band), vec![1]);
        let all = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
        assert_eq!(keys_in_band(&points, all), vec![0, 1, 2]);
        let none = egui::Rect::from_min_max(egui::pos2(0.0, 90.0), egui::pos2(5.0, 95.0));
        assert!(keys_in_band(&points, none).is_empty());
    }

    // The relative multi-drag: one delta on the selected keys, nothing else
    // touched, and a stale (out-of-range) index is a no-op — never a panic.
    #[test]
    fn nudge_moves_only_the_selected_keys_and_ignores_stale_indices() {
        let mut keys = vec![
            marquee_key(0.0, 10.0),
            marquee_key(1.0, 20.0),
            marquee_key(2.0, 30.0),
        ];
        nudge_selected_values(&mut keys, &[0, 2, 99], 5.0);
        assert_eq!(keys[0].value, 15.0);
        assert_eq!(keys[1].value, 20.0); // unselected: untouched
        assert_eq!(keys[2].value, 35.0);
        assert_eq!(keys[0].time, rational_at(0.0)); // times never move
    }

    // The absolute set (a typed value in the property row): every selected
    // key lands on exactly that value, and the change flag skips no-op ops.
    #[test]
    fn set_all_sets_the_exact_value_and_reports_whether_anything_changed() {
        let mut keys = vec![marquee_key(0.0, 10.0), marquee_key(1.0, 20.0)];
        assert!(set_selected_values(&mut keys, &[0, 1], 42.0));
        assert_eq!(keys[0].value, 42.0);
        assert_eq!(keys[1].value, 42.0);
        assert!(!set_selected_values(&mut keys, &[0, 1], 42.0)); // already there
        assert!(!set_selected_values(&mut keys, &[7], 1.0)); // stale index: no-op
    }

    // A selection pins each index to its key's time: removing or inserting a
    // key breaks the pins and the whole selection reads as stale (None), so
    // it can never edit the wrong keyframes.
    #[test]
    fn a_selection_reads_stale_once_the_keys_change_underneath() {
        use crate::app_state::GraphSelection;
        let keys = vec![marquee_key(0.0, 1.0), marquee_key(1.0, 2.0)];
        let sel = GraphSelection {
            layer: uuid::Uuid::nil(),
            prop: lumit_core::model::TransformProp::PositionX,
            retime: false,
            keys: vec![(0, keys[0].time), (1, keys[1].time)],
        };
        assert_eq!(sel.indices_for(&keys), Some(vec![0, 1]));
        // A key removed: index 1 is gone.
        assert_eq!(sel.indices_for(&keys[..1]), None);
        // A key inserted between them shifts index 1 onto the wrong key.
        let shifted = vec![keys[0], marquee_key(0.5, 9.0), keys[1]];
        assert_eq!(sel.indices_for(&shifted), None);
        // Value edits keep the pins intact (times unchanged).
        let mut revalued = keys.clone();
        assert!(set_selected_values(&mut revalued, &[0, 1], 7.0));
        assert_eq!(sel.indices_for(&revalued), Some(vec![0, 1]));
    }

    // Moving a layer shifts in/out AND start_offset by the same delta — a move,
    // not a slip: duration and the in→start_offset alignment are preserved.
    #[test]
    fn moving_a_layer_shifts_the_whole_span_not_slips_it() {
        use lumit_core::time::CompTime;
        let ct = |s: f64| CompTime(rational_at(s));
        let (i, o, so) = moved_span(ct(2.0), ct(5.0), ct(1.0), 1.5);
        assert!((i.0.to_f64() - 3.5).abs() < 1e-6, "in shifts by delta");
        assert!(
            (so.0.to_f64() - 2.5).abs() < 1e-6,
            "start_offset shifts too"
        );
        // Duration preserved.
        assert!(((o.0.to_f64() - i.0.to_f64()) - 3.0).abs() < 1e-6);
        // in→start_offset alignment preserved (content moves with the bar).
        assert!(((i.0.to_f64() - so.0.to_f64()) - 1.0).abs() < 1e-6);
    }

    // The lane-area view (07-UI-SPEC §4): zoom scales pixels-per-second and the
    // view never scrolls past the comp ends.
    #[test]
    fn lane_view_zooms_and_clamps_the_scroll() {
        // Zoom 1: the whole comp fits; no scroll possible.
        let (ppx, start) = lane_view(1000.0, 10.0, 1.0, 5.0);
        assert!((ppx - 100.0).abs() < 1e-6);
        assert!(start.abs() < 1e-6);
        // Zoom 2: half visible, pixels double, scroll clamps to [0, dur - visible].
        let (ppx2, start2) = lane_view(1000.0, 10.0, 2.0, 100.0);
        assert!((ppx2 - 200.0).abs() < 1e-6);
        assert!((start2 - 5.0).abs() < 1e-6);
        // Zoom below 1 is clamped to 1 (can't zoom out past the whole comp).
        let (ppx3, _) = lane_view(1000.0, 10.0, 0.2, 0.0);
        assert!((ppx3 - 100.0).abs() < 1e-6);
    }

    // Graph mode (K-070): the curve fills exactly the lane area — the lanes'
    // width, from under the ruler to just above the bottom bar, sparing the
    // same 38 px strip the lane ScrollArea reserves (scrollbar + bar).
    #[test]
    fn graph_lane_rect_fills_the_lanes_and_spares_the_bottom_bar() {
        let r = graph_lane_rect(200.0, 800.0, 46.0, 600.0);
        assert_eq!(r.left(), 200.0);
        assert_eq!(r.right(), 1000.0);
        assert_eq!(r.top(), 46.0);
        assert_eq!(r.bottom(), 562.0);
        // A panel too short to fit the plot never inverts the rectangle.
        let tiny = graph_lane_rect(200.0, 800.0, 46.0, 50.0);
        assert!(tiny.bottom() >= tiny.top());
    }

    // Regression (layer move outran the cursor): a lane drag converts pixels to
    // seconds at the *displayed* zoom. The same pixel delta must yield half the
    // seconds at zoom 2 as at zoom 1 — the old `dx / track_w * duration` ignored
    // zoom and made drags (and 6 px snap tolerances) run zoom× too fast.
    #[test]
    fn drag_secs_follows_the_displayed_zoom() {
        let (ppx1, _) = lane_view(1000.0, 10.0, 1.0, 0.0);
        let (ppx2, _) = lane_view(1000.0, 10.0, 2.0, 0.0);
        let at_zoom_1 = drag_secs(50.0, ppx1);
        let at_zoom_2 = drag_secs(50.0, ppx2);
        assert!(
            (at_zoom_1 - 0.5).abs() < 1e-9,
            "zoom 1: 50 px over 100 px/s"
        );
        assert!(
            (at_zoom_2 - at_zoom_1 / 2.0).abs() < 1e-9,
            "zoom 2 shows twice the pixels per second, so the same drag is half the time"
        );
        // The unzoomed conversion would have (wrongly) said 0.5 s at any zoom.
        assert!((at_zoom_2 - 0.25).abs() < 1e-9);
        // A degenerate px_per_sec never divides by zero.
        assert!(drag_secs(50.0, 0.0).is_finite());
    }
}
