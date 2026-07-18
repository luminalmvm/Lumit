//! The application Settings window (docs/07-UI-SPEC.md §15): a macOS
//! System-Settings-style surface — a sidebar of pages on the left, each page
//! a column of grouped "cards" of rows on the right, label on the left of a
//! row and its control on the right. It honours the Sharp/Round shape like
//! every other surface: Round gives the cards rounded corners and a fill,
//! Sharp gives them a hairline frame.
//!
//! In plain terms: this is the one place to change application-wide settings —
//! how Lumit looks (theme, shape, motion) and how hard it works your machine
//! (how much memory and disk it may use for its frame cache). Each left-hand
//! entry is a page; each page groups related settings under a heading. It
//! replaces the old cluster of theme toggles that used to live in the Window
//! menu — those now live on the Appearance page.
//!
//! All colours come from the theme snapshot passed in; this module constructs
//! no `Color32` of its own (the no-hex rule, docs/15-DESIGN.md).

use super::*;

/// Which Settings page is showing. Runtime-only — the window always opens on
/// [`SettingsPage::Appearance`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum SettingsPage {
    General,
    #[default]
    Appearance,
    Interface,
    Performance,
    /// Export defaults (K-119). Gated on the `media` feature like every
    /// other export concept (`crate::export` compiles to nothing without
    /// it), so this variant — and the page itself — simply doesn't exist on
    /// a `--no-default-features` build.
    #[cfg(feature = "media")]
    Export,
}

impl SettingsPage {
    #[cfg(feature = "media")]
    const ALL: [SettingsPage; 5] = [
        SettingsPage::General,
        SettingsPage::Appearance,
        SettingsPage::Interface,
        SettingsPage::Performance,
        SettingsPage::Export,
    ];
    #[cfg(not(feature = "media"))]
    const ALL: [SettingsPage; 4] = [
        SettingsPage::General,
        SettingsPage::Appearance,
        SettingsPage::Interface,
        SettingsPage::Performance,
    ];

    fn title(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Appearance => "Appearance",
            SettingsPage::Interface => "Interface",
            SettingsPage::Performance => "Performance",
            #[cfg(feature = "media")]
            SettingsPage::Export => "Export",
        }
    }
}

/// Application-wide performance settings — how much of the machine Lumit's
/// frame cache may use (docs/06-RENDER-PIPELINE.md §5). Persisted with the
/// workspace, like the theme choices. Defaults reproduce today's hardcoded
/// budgets exactly, so an existing install is unchanged until the user moves
/// a slider.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct PerformanceSettings {
    /// RAM frame-cache budget, in mebibytes (the `comp_frame_cache` LRU).
    pub ram_cache_mb: u32,
    /// Disk frame-cache cap, in mebibytes (the `.lum-cache` sidecar).
    pub disk_cache_mb: u32,
    /// Video-memory (VRAM) frame-cache budget, in mebibytes (the GPU
    /// display-texture tier, `GpuViewer::vram`, docs/06 §5).
    pub vram_cache_mb: u32,
    /// Whether Lumit fills the frame cache around the playhead while idle
    /// (docs/06 §5.4). On by default; off trades a colder cache for zero
    /// background decode/render work when the machine is busy elsewhere.
    pub background_fill: bool,
    /// Where the on-disk frame cache is stored. `None` (the default) keeps
    /// today's behaviour — a `<project>.lum-cache` folder beside the project
    /// file. `Some` redirects new project caches to a folder under the given
    /// root instead (e.g. a fast NVMe), per project
    /// (`lumit_cache::disk::cache_root_for`).
    pub cache_root: Option<std::path::PathBuf>,
}

/// Application-wide interface settings (Settings → Interface, docs/07-UI-SPEC
/// §15): how large the chrome draws, and whether hover tooltips show at all.
/// Persisted with the workspace. Defaults reproduce today's implicit
/// behaviour exactly — native scale, tooltips on — so an existing install is
/// unchanged until the user touches a control.
#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct InterfaceSettings {
    /// Zoom factor applied on top of the display's native scale, via egui's
    /// own `Context::set_pixels_per_point` (K-117). 1.0 = native.
    pub ui_scale: f32,
    /// Whether hover tooltips show anywhere in the app (docs/07-UI-SPEC §13.2
    /// "a single setting disables all tooltips"). On by default.
    pub show_tooltips: bool,
}

impl Default for InterfaceSettings {
    fn default() -> Self {
        Self {
            ui_scale: 1.0,
            show_tooltips: true,
        }
    }
}

/// Apply the "show tooltips" setting to the live interaction style (K-117):
/// off pushes `tooltip_delay` to infinity, which the hover logic in
/// `egui::Response::should_show_hover_ui` reads as "the wait is never over" —
/// every code path that would show a tooltip instead requests a repaint after
/// `tooltip_delay - elapsed` seconds and returns `false`, and that repaint
/// request itself uses a fallible `Duration` conversion that silently no-ops
/// on an infinite input, so this never panics. On restores egui's own default
/// (0.5 s, `egui::Style::default().interaction.tooltip_delay`) rather than a
/// hardcoded guess, so a future egui upgrade that changes the default keeps
/// working. Called once at start-up (so a saved preference takes effect
/// before the first frame) and again whenever the checkbox changes.
pub(crate) fn apply_tooltips_enabled(ctx: &egui::Context, enabled: bool) {
    let delay = if enabled {
        egui::Style::default().interaction.tooltip_delay
    } else {
        f32::INFINITY
    };
    ctx.style_mut(|s| s.interaction.tooltip_delay = delay);
}

/// Autosave settings (Settings → General; docs/07-UI-SPEC §15). Persisted
/// with the workspace; defaults reproduce the previous hardcoded behaviour.
#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AutosaveSettings {
    /// Minutes between autosaves (floored at 1 when applied).
    pub interval_mins: u32,
    /// Backup copies kept.
    pub keep: u32,
}

impl Default for AutosaveSettings {
    fn default() -> Self {
        Self {
            // Matches `AUTOSAVE_INTERVAL_SECS` (300 s) and `AUTOSAVE_KEEP`.
            interval_mins: (crate::app_state::AUTOSAVE_INTERVAL_SECS / 60) as u32,
            keep: crate::app_state::AUTOSAVE_KEEP as u32,
        }
    }
}

impl Default for PerformanceSettings {
    fn default() -> Self {
        Self {
            // Matches `AppState`'s `ByteLru::new(512 * 1024 * 1024)`.
            ram_cache_mb: 512,
            // Matches `AppState::DEFAULT_CAP_BYTES` (50 GiB).
            disk_cache_mb: 50 * 1024,
            // Matches `gpu::VRAM_TIER_CAP` (512 MiB).
            vram_cache_mb: 512,
            // Matches today's unconditional idle-fill behaviour.
            background_fill: true,
            // Matches today's unconditional beside-the-project behaviour.
            cache_root: None,
        }
    }
}

/// Application-wide export settings (Settings → Export, docs/07-UI-SPEC §15,
/// K-119): the preset a plain "Export…" action stamps, and an optional
/// filename template for the export dialogue's suggested name. Persisted
/// with the workspace. Defaults reproduce today's implicit behaviour exactly
/// — Custom (the comp's own size) and no template (each preset's own file
/// name) — so an existing install is unchanged until the user visits the
/// page. Gated on the `media` feature: `ExportPreset` lives in
/// `crate::export`, which compiles to nothing without it.
#[cfg(feature = "media")]
#[derive(Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct ExportSettings {
    /// Preset a generic "Export…" action stamps: the File-menu "Export
    /// comp…" entry and its native-menu twin. Picking a specific preset from
    /// the "Export preset" submenu always uses that preset, regardless of
    /// this default.
    pub default_preset: crate::export::ExportPreset,
    /// Filename template for the export dialogue's suggested name. `{comp}`,
    /// `{preset}`, and `{date}` substitute the composition name, the
    /// preset's file stem, and today's date (YYYY-MM-DD). `None` (the
    /// default) keeps today's behaviour: each preset's own default file
    /// name, untouched.
    pub filename_template: Option<String>,
}

/// The dialog's fixed content width — the same on every page, so switching
/// pages never resizes it.
const SETTINGS_WIDTH: f32 = 680.0;
/// The dialog's fixed body height (below the title bar), so a short page and
/// a long one make the same-sized dialog; long pages scroll inside it.
const SETTINGS_BODY_HEIGHT: f32 = 420.0;
/// The sidebar column width.
const SETTINGS_SIDEBAR_WIDTH: f32 = 150.0;

impl Shell {
    /// Draw the Settings dialog when it is open. A true modal (`egui::Modal`):
    /// a dimmed backdrop eats clicks to the app behind it, and it draws in the
    /// foreground layer — above the panel-focus edge, which sits at
    /// `Order::Middle`. Fixed size so every page makes the same dialog.
    pub(crate) fn settings_modal(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        // Theme is `Copy`, so a snapshot lets the body read colours while it
        // mutates `self` (theme picks) with no borrow clash.
        let theme = self.theme;
        let modal = egui::Modal::new(egui::Id::new("lumit-settings")).show(ctx, |ui| {
            ui.set_width(SETTINGS_WIDTH);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Settings")
                        .heading()
                        .color(theme.text_primary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Done").clicked() {
                        self.settings_open = false;
                    }
                });
            });
            ui.separator();
            ui.horizontal_top(|ui| {
                self.settings_sidebar(ui, &theme);
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_min_height(SETTINGS_BODY_HEIGHT);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(SETTINGS_BODY_HEIGHT)
                        .show(ui, |ui| {
                            match self.settings_page {
                                SettingsPage::General => self.settings_general(ui, &theme),
                                SettingsPage::Appearance => {
                                    self.settings_appearance(ui, &theme, ctx)
                                }
                                SettingsPage::Interface => self.settings_interface(ui, &theme, ctx),
                                SettingsPage::Performance => self.settings_performance(ui, &theme),
                                #[cfg(feature = "media")]
                                SettingsPage::Export => self.settings_export(ui, &theme),
                            }
                            ui.add_space(8.0);
                        });
                });
            });
        });
        // Clicking the dimmed backdrop or pressing Escape closes it.
        if modal.should_close() {
            self.settings_open = false;
        }
    }

    fn settings_sidebar(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        ui.vertical(|ui| {
            ui.set_width(SETTINGS_SIDEBAR_WIDTH);
            ui.add_space(4.0);
            for page in SettingsPage::ALL {
                let selected = self.settings_page == page;
                let text = if selected {
                    egui::RichText::new(page.title()).color(theme.text_primary)
                } else {
                    egui::RichText::new(page.title()).color(theme.text_secondary)
                };
                if ui
                    .add_sized(
                        [ui.available_width(), 26.0],
                        egui::SelectableLabel::new(selected, text),
                    )
                    .clicked()
                {
                    self.settings_page = page;
                }
            }
        });
    }

    // --- General -----------------------------------------------------------

    fn settings_general(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        page_heading(ui, theme, "General");

        settings_group(ui, theme, "Workspace", |ui| {
            settings_row(
                ui,
                theme,
                "Panel layout",
                Some("Return every panel to its default place and size."),
                |ui| {
                    if ui.button("Reset workspace").clicked() {
                        self.dock = default_layout();
                    }
                },
            );
        });

        settings_group(ui, theme, "Autosave", |ui| {
            settings_row(
                ui,
                theme,
                "Every",
                Some("Minutes between automatic saves of a saved project."),
                |ui| {
                    ui.label(egui::RichText::new("min").color(theme.text_muted));
                    ui.add(egui::DragValue::new(&mut self.autosave.interval_mins).range(1..=60));
                },
            );
            settings_divider(ui, theme);
            settings_row(
                ui,
                theme,
                "Copies kept",
                Some("How many timestamped backups to keep."),
                |ui| {
                    ui.add(egui::DragValue::new(&mut self.autosave.keep).range(1..=50));
                },
            );
        });

        settings_group(ui, theme, "About", |ui| {
            settings_row(ui, theme, "Version", None, |ui| {
                ui.label(
                    egui::RichText::new(env!("CARGO_PKG_VERSION")).color(theme.text_secondary),
                );
            });
        });
    }

    // --- Appearance --------------------------------------------------------

    fn settings_appearance(&mut self, ui: &mut egui::Ui, theme: &Theme, ctx: &egui::Context) {
        page_heading(ui, theme, "Appearance");

        settings_group(ui, theme, "Theme", |ui| {
            // Colour scheme: one dropdown over every built-in scheme (K-097),
            // folding in the old light/dark and background-ramp choices.
            let mut scheme = self.color_scheme;
            settings_row(
                ui,
                theme,
                "Colour scheme",
                Some("The whole palette — light, dark, and community themes."),
                |ui| {
                    bare_dropdown(ui, scheme.label(), |ui| {
                        for s in crate::theme::ColorScheme::ALL {
                            if ui.selectable_label(scheme == s, s.label()).clicked() {
                                scheme = s;
                                ui.close_menu();
                            }
                        }
                    });
                },
            );
            if scheme != self.color_scheme {
                self.color_scheme = scheme;
                self.recompose(ctx);
            }

            // Accent colour.
            settings_divider(ui, theme);
            settings_row(
                ui,
                theme,
                "Accent",
                Some("The single highlight colour."),
                |ui| {
                    let mut rgb = self
                        .accent_override
                        .unwrap_or(crate::theme::Theme::default_accent_rgb());
                    if self.accent_override.is_some() && ui.small_button("Reset").clicked() {
                        self.accent_override = None;
                        self.recompose(ctx);
                    } else if ui.color_edit_button_srgb(&mut rgb).changed() {
                        self.accent_override = Some(rgb);
                        self.recompose(ctx);
                    }
                },
            );
        });

        settings_group(ui, theme, "Shape and motion", |ui| {
            // Sharp or Round panel geometry (K-092).
            let mut shape = self.theme_shape;
            settings_row(
                ui,
                theme,
                "Panel shape",
                Some("Sharp edge-to-edge, or rounded floating cards."),
                |ui| {
                    bare_dropdown(ui, shape_label(shape), |ui| {
                        for s in [
                            crate::theme::ThemeShape::Sharp,
                            crate::theme::ThemeShape::Round,
                        ] {
                            if ui.selectable_label(shape == s, shape_label(s)).clicked() {
                                shape = s;
                                ui.close_menu();
                            }
                        }
                    });
                },
            );
            if shape != self.theme_shape {
                self.theme_shape = shape;
                self.recompose(ctx);
            }

            settings_divider(ui, theme);
            let mut anim = self.animation_level;
            settings_row(
                ui,
                theme,
                "Interface motion",
                Some("How much the chrome animates."),
                |ui| {
                    bare_dropdown(ui, anim_label(anim), |ui| {
                        for a in [
                            crate::theme::AnimationLevel::All,
                            crate::theme::AnimationLevel::Minimal,
                            crate::theme::AnimationLevel::None,
                        ] {
                            if ui.selectable_label(anim == a, anim_label(a)).clicked() {
                                anim = a;
                                ui.close_menu();
                            }
                        }
                    });
                },
            );
            if anim != self.animation_level {
                self.animation_level = anim;
                crate::theme::apply_animation_level(ctx, anim);
            }
        });
    }

    // --- Interface -----------------------------------------------------------

    fn settings_interface(&mut self, ui: &mut egui::Ui, theme: &Theme, ctx: &egui::Context) {
        page_heading(ui, theme, "Interface");

        settings_group(ui, theme, "Display", |ui| {
            let mut scale = self.interface.ui_scale;
            settings_row(
                ui,
                theme,
                "UI scale",
                Some("How large Lumit's interface draws relative to your display's native scale."),
                |ui| {
                    ui.add(
                        egui::Slider::new(&mut scale, 0.75..=2.0)
                            .step_by(0.05)
                            .fixed_decimals(2)
                            .suffix("×"),
                    );
                },
            );
            if scale != self.interface.ui_scale {
                self.interface.ui_scale = scale;
                ctx.set_pixels_per_point(scale);
            }

            settings_divider(ui, theme);
            let mut show_tooltips = self.interface.show_tooltips;
            settings_row(
                ui,
                theme,
                "Show tooltips",
                Some("Show hover tooltips throughout the app."),
                |ui| {
                    ui.checkbox(&mut show_tooltips, "");
                },
            );
            if show_tooltips != self.interface.show_tooltips {
                self.interface.show_tooltips = show_tooltips;
                apply_tooltips_enabled(ctx, show_tooltips);
            }
        });
    }

    // --- Performance -------------------------------------------------------

    fn settings_performance(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        page_heading(ui, theme, "Performance");

        settings_group(ui, theme, "Frame cache", |ui| {
            let mut ram = self.settings.ram_cache_mb;
            settings_row(
                ui,
                theme,
                "Memory budget",
                Some("How much RAM the frame cache may hold."),
                |ui| {
                    ui.label(egui::RichText::new("MB").color(theme.text_muted));
                    ui.add(egui::DragValue::new(&mut ram).speed(64).range(128..=32768));
                },
            );
            if ram != self.settings.ram_cache_mb {
                self.settings.ram_cache_mb = ram;
                self.apply_cache_budgets();
            }

            settings_divider(ui, theme);
            let mut disk = self.settings.disk_cache_mb;
            settings_row(
                ui,
                theme,
                "Disk budget",
                Some("Cap on the on-disk frame cache (.lum-cache)."),
                |ui| {
                    ui.label(egui::RichText::new("MB").color(theme.text_muted));
                    ui.add(
                        egui::DragValue::new(&mut disk)
                            .speed(256)
                            .range(0..=1_048_576),
                    );
                },
            );
            if disk != self.settings.disk_cache_mb {
                self.settings.disk_cache_mb = disk;
                self.apply_cache_budgets();
            }

            settings_divider(ui, theme);
            let mut vram = self.settings.vram_cache_mb;
            settings_row(
                ui,
                theme,
                "Video memory budget",
                Some("How much VRAM the displayed-frame cache may hold."),
                |ui| {
                    ui.label(egui::RichText::new("MB").color(theme.text_muted));
                    ui.add(egui::DragValue::new(&mut vram).speed(64).range(128..=16384));
                },
            );
            if vram != self.settings.vram_cache_mb {
                self.settings.vram_cache_mb = vram;
                self.apply_cache_budgets();
            }
        });

        settings_group(ui, theme, "Cache", |ui| {
            settings_row(
                ui,
                theme,
                "Clear cache",
                Some("Empty the RAM and video-memory frame caches now."),
                |ui| {
                    if ui.button("Clear cache").clicked() {
                        self.clear_frame_caches();
                    }
                },
            );
            settings_divider(ui, theme);
            settings_row(
                ui,
                theme,
                "Background fill",
                Some(
                    "Decode ahead around the playhead while idle, so scrubbing hits a warm cache.",
                ),
                |ui| {
                    ui.checkbox(&mut self.settings.background_fill, "");
                },
            );
            settings_divider(ui, theme);
            settings_row(
                ui,
                theme,
                "Cache root folder",
                Some(
                    "Where the on-disk frame cache is stored. Choosing a folder moves new \
                     project caches there instead of next to the project file.",
                ),
                |ui| {
                    if self.settings.cache_root.is_some() && ui.button("Use default").clicked() {
                        self.settings.cache_root = None;
                    }
                    if ui.button("Choose…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.settings.cache_root = Some(path);
                        }
                    }
                    let label = match &self.settings.cache_root {
                        Some(p) => p.display().to_string(),
                        None => "Default (next to the project file)".to_string(),
                    };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(label).small().color(theme.text_muted),
                        )
                        .truncate(),
                    );
                },
            );
        });
    }

    // --- Export --------------------------------------------------------------

    #[cfg(feature = "media")]
    fn settings_export(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        page_heading(ui, theme, "Export");

        settings_group(ui, theme, "Defaults", |ui| {
            let mut preset = self.settings_export.default_preset;
            settings_row(
                ui,
                theme,
                "Default preset",
                Some(
                    "The preset a plain \"Export…\" action stamps. Picking a specific \
                     preset from the Export preset menu always uses that preset instead.",
                ),
                |ui| {
                    bare_dropdown(ui, preset.label(), |ui| {
                        for p in crate::export::ExportPreset::ALL {
                            if ui.selectable_label(preset == p, p.label()).clicked() {
                                preset = p;
                                ui.close_menu();
                            }
                        }
                    });
                },
            );
            if preset != self.settings_export.default_preset {
                self.settings_export.default_preset = preset;
            }

            settings_divider(ui, theme);
            let mut template = self
                .settings_export
                .filename_template
                .clone()
                .unwrap_or_default();
            settings_row(
                ui,
                theme,
                "Filename template",
                Some(
                    "{comp}, {preset}, and {date} stand for the composition name, the \
                     preset's file name, and today's date (YYYY-MM-DD). Leave blank to \
                     use each preset's own default file name.",
                ),
                |ui| {
                    ui.add(egui::TextEdit::singleline(&mut template).desired_width(180.0));
                },
            );
            let normalised = (!template.trim().is_empty()).then_some(template);
            if normalised != self.settings_export.filename_template {
                self.settings_export.filename_template = normalised;
            }
        });
    }

    /// Push the current cache budgets to their stores: the RAM
    /// `comp_frame_cache` directly, the disk cache through its worker (which
    /// remembers the cap across project switches). Called at start-up and
    /// whenever a Performance slider moves.
    pub(crate) fn apply_cache_budgets(&mut self) {
        let ram = (self.settings.ram_cache_mb as usize).saturating_mul(1024 * 1024);
        self.app.comp_frame_cache.set_budget(ram);
        let disk = (self.settings.disk_cache_mb as u64).saturating_mul(1024 * 1024);
        if let Some(io) = &self.app.disk_io {
            let _ = io.tx.send(crate::app_state::diskio::Cmd::SetCap(disk));
        }
        #[cfg(feature = "media")]
        {
            let vram = (self.settings.vram_cache_mb as u64).saturating_mul(1024 * 1024);
            if let Some(gpu) = &mut self.gpu {
                gpu.set_vram_cap(vram);
            }
        }
    }

    /// Empty the RAM and VRAM frame-cache tiers immediately (Settings →
    /// Performance "Clear cache", K-100) and bump the cache epoch so the
    /// cache bar and any live views notice the tiers are now empty.
    pub(crate) fn clear_frame_caches(&mut self) {
        self.app.comp_frame_cache.clear();
        #[cfg(feature = "media")]
        if let Some(gpu) = &mut self.gpu {
            gpu.clear_vram();
        }
        self.app.cache_epoch += 1;
    }

    /// Rebuild and re-apply the theme from the current appearance fields, plus
    /// any accent override. The single funnel every Appearance control uses
    /// (was an inline closure in the Window menu before the Settings window).
    pub(crate) fn recompose(&mut self, ctx: &egui::Context) {
        self.theme = Theme::for_scheme(self.color_scheme, self.theme_shape);
        if let Some(rgb) = self.accent_override {
            self.theme = self.theme.with_accent(rgb);
        }
        self.theme.apply(ctx);
        let s0 = self.theme.surface_0;
        ctx.style_mut(|s| s.visuals.panel_fill = s0);
    }
}

// --- Shared drawing helpers ------------------------------------------------

fn page_heading(ui: &mut egui::Ui, theme: &Theme, title: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(title)
            .heading()
            .color(theme.text_primary),
    );
}

/// A titled group of rows, drawn as a card: rounded and filled under Round,
/// hairline-framed under Sharp — the same elevation language as the docked
/// panels (docs/15-DESIGN.md §7).
fn settings_group(ui: &mut egui::Ui, theme: &Theme, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(12.0);
    ui.label(egui::RichText::new(title).small().color(theme.text_muted));
    ui.add_space(4.0);
    let mut frame = egui::Frame::new()
        .fill(theme.surface_2)
        .inner_margin(egui::Margin::symmetric(12, 6));
    frame = if theme.shape == crate::theme::ThemeShape::Round {
        frame.corner_radius(theme.tokens.card_radius)
    } else {
        frame.stroke(egui::Stroke::new(1.0_f32, theme.hairline))
    };
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

/// One labelled row: label (and optional hint under it) on the left, the
/// control right-aligned.
fn settings_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: &str,
    hint: Option<&str>,
    control: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(label).color(theme.text_primary));
            if let Some(h) = hint {
                ui.label(egui::RichText::new(h).small().color(theme.text_muted));
            }
            ui.add_space(4.0);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
}

/// A hairline between rows in a group.
fn settings_divider(ui: &mut egui::Ui, theme: &Theme) {
    let rect = ui.available_rect_before_wrap();
    let y = ui.cursor().top();
    ui.painter().hline(
        rect.left()..=rect.right(),
        y,
        egui::Stroke::new(1.0_f32, theme.hairline),
    );
}

fn shape_label(s: crate::theme::ThemeShape) -> &'static str {
    match s {
        crate::theme::ThemeShape::Sharp => "Sharp",
        crate::theme::ThemeShape::Round => "Round",
    }
}

fn anim_label(a: crate::theme::AnimationLevel) -> &'static str {
    match a {
        crate::theme::AnimationLevel::All => "All",
        crate::theme::AnimationLevel::Minimal => "Minimal",
        crate::theme::AnimationLevel::None => "None",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_defaults_match_the_hardcoded_budgets() {
        let p = PerformanceSettings::default();
        // These must stay in step with AppState's constants, so a fresh
        // install behaves exactly as before the settings surface existed.
        assert_eq!(p.ram_cache_mb, 512);
        assert_eq!(p.disk_cache_mb, 50 * 1024);
        // Matches `gpu::VRAM_TIER_CAP`.
        assert_eq!(p.vram_cache_mb, 512);
        assert!(p.background_fill);
    }

    #[test]
    fn every_page_has_a_title() {
        for page in SettingsPage::ALL {
            assert!(!page.title().is_empty());
        }
    }

    #[test]
    fn interface_defaults_are_a_no_op_for_existing_installs() {
        let i = InterfaceSettings::default();
        assert_eq!(i.ui_scale, 1.0);
        assert!(i.show_tooltips);
    }

    #[cfg(feature = "media")]
    #[test]
    fn export_defaults_are_a_no_op_for_existing_installs() {
        let e = ExportSettings::default();
        assert_eq!(e.default_preset, crate::export::ExportPreset::Custom);
        assert_eq!(e.filename_template, None);
    }

    #[test]
    fn tooltips_off_pushes_the_delay_to_infinity_and_on_restores_the_egui_default() {
        let ctx = egui::Context::default();
        apply_tooltips_enabled(&ctx, false);
        ctx.style_mut(|s| assert_eq!(s.interaction.tooltip_delay, f32::INFINITY));
        apply_tooltips_enabled(&ctx, true);
        let egui_default = egui::Style::default().interaction.tooltip_delay;
        ctx.style_mut(|s| assert_eq!(s.interaction.tooltip_delay, egui_default));
    }
}
