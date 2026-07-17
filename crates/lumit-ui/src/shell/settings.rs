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
    Performance,
}

impl SettingsPage {
    const ALL: [SettingsPage; 3] = [
        SettingsPage::General,
        SettingsPage::Appearance,
        SettingsPage::Performance,
    ];

    fn title(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Appearance => "Appearance",
            SettingsPage::Performance => "Performance",
        }
    }
}

/// Application-wide performance settings — how much of the machine Lumit's
/// frame cache may use (docs/06-RENDER-PIPELINE.md §5). Persisted with the
/// workspace, like the theme choices. Defaults reproduce today's hardcoded
/// budgets exactly, so an existing install is unchanged until the user moves
/// a slider.
#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PerformanceSettings {
    /// RAM frame-cache budget, in mebibytes (the `comp_frame_cache` LRU).
    pub ram_cache_mb: u32,
    /// Disk frame-cache cap, in mebibytes (the `.lum-cache` sidecar).
    pub disk_cache_mb: u32,
}

impl Default for PerformanceSettings {
    fn default() -> Self {
        Self {
            // Matches `AppState`'s `ByteLru::new(512 * 1024 * 1024)`.
            ram_cache_mb: 512,
            // Matches `AppState::DEFAULT_CAP_BYTES` (50 GiB).
            disk_cache_mb: 50 * 1024,
        }
    }
}

impl Shell {
    /// Draw the Settings window when it is open. Mirrors the other modals
    /// (`export_dialog_modal`), invoked once per frame from `update`.
    pub(crate) fn settings_modal(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        // Theme is `Copy`, so a snapshot lets the window body read colours
        // while it mutates `self` (theme picks) with no borrow clash.
        let theme = self.theme;
        let mut open = self.settings_open;
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(true)
            .default_size([720.0, 520.0])
            .min_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    self.settings_sidebar(ui, &theme);
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_min_width(360.0);
                            ui.add_space(2.0);
                            match self.settings_page {
                                SettingsPage::General => self.settings_general(ui, &theme),
                                SettingsPage::Appearance => {
                                    self.settings_appearance(ui, &theme, ctx)
                                }
                                SettingsPage::Performance => self.settings_performance(ui, &theme),
                            }
                            ui.add_space(8.0);
                        });
                });
            });
        self.settings_open = open;
    }

    fn settings_sidebar(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        ui.vertical(|ui| {
            ui.set_width(160.0);
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
            // Mode: light or dark chrome (K-092).
            let mut mode = self.theme_mode;
            settings_row(ui, theme, "Mode", Some("Light or dark chrome."), |ui| {
                bare_dropdown(ui, mode_label(mode), |ui| {
                    for m in [
                        crate::theme::ThemeMode::Dark,
                        crate::theme::ThemeMode::Light,
                    ] {
                        if ui.selectable_label(mode == m, mode_label(m)).clicked() {
                            mode = m;
                            ui.close_menu();
                        }
                    }
                });
            });
            if mode != self.theme_mode {
                self.theme_mode = mode;
                self.recompose(ctx);
            }

            // Background ramp — meaningful only under Dark (one light ramp).
            if self.theme_mode == crate::theme::ThemeMode::Dark {
                settings_divider(ui, theme);
                let mut variant = self.theme_variant;
                settings_row(
                    ui,
                    theme,
                    "Background",
                    Some("Which dark ramp the chrome uses."),
                    |ui| {
                        bare_dropdown(ui, variant_label(variant), |ui| {
                            for v in [
                                crate::theme::ThemeVariant::Dark,
                                crate::theme::ThemeVariant::DarkBlue,
                            ] {
                                if ui
                                    .selectable_label(variant == v, variant_label(v))
                                    .clicked()
                                {
                                    variant = v;
                                    ui.close_menu();
                                }
                            }
                        });
                    },
                );
                if variant != self.theme_variant {
                    self.theme_variant = variant;
                    self.recompose(ctx);
                }
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
    }

    /// Rebuild and re-apply the theme from the current appearance fields, plus
    /// any accent override. The single funnel every Appearance control uses
    /// (was an inline closure in the Window menu before the Settings window).
    pub(crate) fn recompose(&mut self, ctx: &egui::Context) {
        self.theme = Theme::for_settings(self.theme_mode, self.theme_variant, self.theme_shape);
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

fn mode_label(m: crate::theme::ThemeMode) -> &'static str {
    match m {
        crate::theme::ThemeMode::Dark => "Dark",
        crate::theme::ThemeMode::Light => "Light",
    }
}

fn variant_label(v: crate::theme::ThemeVariant) -> &'static str {
    match v {
        crate::theme::ThemeVariant::Dark => "Dark",
        crate::theme::ThemeVariant::DarkBlue => "Dark blue",
    }
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
    }

    #[test]
    fn every_page_has_a_title() {
        for page in SettingsPage::ALL {
            assert!(!page.title().is_empty());
        }
    }
}
