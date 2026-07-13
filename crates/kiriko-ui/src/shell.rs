//! The application shell: menu bar, docked panels, status line.
//!
//! Layout per docs/07-UI-SPEC.md (Edit workspace): Project left, Viewer centre,
//! Effect Controls / Effects & Presets right, Timeline across the bottom.

use crate::app_state::{AppState, ShapeKind, ToolMode};
use crate::splash::{BootLine, Splash};
use crate::theme::Theme;
use kiriko_core::model::ProjectItem;
use serde::{Deserialize, Serialize};

/// The dockable panels. Names are glossary names (docs/01-GLOSSARY.md §7).
/// A dockable panel (a pane in the tiling tree). The Viewer is special: it is
/// the only pane kept out of any tab container, so it shows no tab bar (K-074,
/// Mack: the viewport must have no top bit); every other panel carries a tab
/// and can be dragged to re-arrange the workspace.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Panel {
    Project,
    Viewer,
    Timeline,
    EffectControls,
    EffectsAndPresets,
    Scopes,
}

impl Panel {
    fn title(&self) -> &'static str {
        match self {
            Panel::Project => "Project",
            Panel::Viewer => "Viewer",
            Panel::Timeline => "Timeline",
            Panel::EffectControls => "Effect controls",
            Panel::EffectsAndPresets => "Effects & presets",
            Panel::Scopes => "Scopes",
        }
    }
}

/// The default workspace: slim tool columns either side of a tall Viewer that
/// sits above the Timeline. The Viewer is a bare pane (no tab); Project/effects
/// share a tab group on the left, Scopes tabs on the right, Timeline tabs below.
pub fn default_layout() -> egui_tiles::Tree<Panel> {
    let mut tiles = egui_tiles::Tiles::default();
    let viewer = tiles.insert_pane(Panel::Viewer);
    let timeline = tiles.insert_pane(Panel::Timeline);
    let timeline_tabs = tiles.insert_tab_tile(vec![timeline]);
    let centre = tiles.insert_vertical_tile(vec![viewer, timeline_tabs]);
    let project = tiles.insert_pane(Panel::Project);
    let fx = tiles.insert_pane(Panel::EffectControls);
    let fxp = tiles.insert_pane(Panel::EffectsAndPresets);
    let left = tiles.insert_tab_tile(vec![project, fx, fxp]);
    let scopes = tiles.insert_pane(Panel::Scopes);
    let right = tiles.insert_tab_tile(vec![scopes]);
    let root = tiles.insert_horizontal_tile(vec![left, centre, right]);
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) =
        tiles.get_mut(root)
    {
        lin.shares.set_share(left, 0.22);
        lin.shares.set_share(centre, 0.58);
        lin.shares.set_share(right, 0.20);
    }
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) =
        tiles.get_mut(centre)
    {
        lin.shares.set_share(viewer, 0.68);
        lin.shares.set_share(timeline_tabs, 0.32);
    }
    egui_tiles::Tree::new("kiriko-dock", root, tiles)
}

/// Bridges the tiling tree to Kiriko's panels and house styling.
struct DockBehavior<'a> {
    theme: &'a Theme,
    app: &'a mut AppState,
    preview_display: Option<(egui::TextureId, egui::Vec2)>,
}

impl egui_tiles::Behavior<Panel> for DockBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut Panel,
    ) -> egui_tiles::UiResponse {
        match pane {
            Panel::Viewer => viewer_panel(ui, self.theme, self.app, self.preview_display),
            Panel::Project => project_panel(ui, self.theme, self.app),
            Panel::Timeline => timeline_panel(ui, self.theme, self.app),
            Panel::EffectControls => empty_hint(
                ui,
                self.theme,
                "No layer selected",
                "Select a layer to see its effect stack.",
            ),
            Panel::EffectsAndPresets => effects_panel(ui, self.theme),
            Panel::Scopes => empty_hint(
                ui,
                self.theme,
                "Scopes",
                "Waveform, vectorscope and histogram arrive with the render pipeline.",
            ),
        }
        egui_tiles::UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &Panel) -> egui::WidgetText {
        match pane {
            // The Timeline tab shows the open comp's name, so comps read as tabs.
            Panel::Timeline => self
                .app
                .selected_comp
                .and_then(|id| self.app.store.snapshot().comp(id).map(|c| c.name.clone()))
                .unwrap_or_else(|| "Timeline".into())
                .into(),
            other => other.title().into(),
        }
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        26.0
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        1.0
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            prune_empty_tabs: true,
            prune_empty_containers: true,
            // Keep single-pane tab groups (Timeline, Scopes) so they retain a
            // tab; but never force the Viewer into one (all_panes_… = false).
            prune_single_child_tabs: false,
            prune_single_child_containers: true,
            all_panes_must_have_tabs: false,
            join_nested_linear_containers: true,
        }
    }
}

/// A text field that mirrors a model value when idle and holds the user's
/// keystrokes while focused (so partial entry like "29.99" isn't clobbered
/// mid-type). Returns the response plus the buffer as it stands this frame;
/// the caller parses on `lost_focus()`.
fn text_field(
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

/// A dropdown that shows just its label — no down-caret (the house style).
/// Returns whatever the menu closure produces.
fn bare_dropdown<R>(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    ui.menu_button(label, add).inner
}

/// `HH:MM:SS:mmm` from seconds (docs: composition duration display).
fn fmt_duration(secs: f64) -> String {
    let total_ms = (secs.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let s = (total_ms / 1000) % 60;
    let m = (total_ms / 60_000) % 60;
    let h = total_ms / 3_600_000;
    format!("{h:02}:{m:02}:{s:02}:{ms:03}")
}

/// Parse a flexible duration: `SS(.sss)`, `MM:SS`, `HH:MM:SS`, or
/// `HH:MM:SS:mmm`. None on anything unparseable.
fn parse_duration(text: &str) -> Option<f64> {
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
fn aspect_ratio_label(w: u32, h: u32) -> String {
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

/// The Viewer: neutral surround + the empty-project card (docs/07-UI-SPEC.md §13.2).
#[cfg_attr(not(feature = "media"), allow(unused_variables))]
fn viewer_panel(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    tex: Option<(egui::TextureId, egui::Vec2)>,
) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, 0.0, theme.viewer_surround);
    accept_item_drop(ui, theme, app, rect);

    #[cfg(feature = "media")]
    if app.preview_item.is_some() || app.preview_comp.is_some() {
        viewer_footage(ui, theme, app, tex, rect);
        return;
    }

    let has_content = !app.store.snapshot().items.is_empty();

    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(rect.height() * 0.32);
                if has_content {
                    ui.label(
                        egui::RichText::new("Footage display arrives in slice 5.")
                            .small()
                            .color(theme.text_disabled),
                    );
                    return;
                }
                egui::Frame::group(ui.style())
                    .fill(theme.surface_1)
                    .stroke(egui::Stroke::new(1.0_f32, theme.hairline_strong))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(28, 20))
                    .show(ui, |ui| {
                        ui.set_max_width(340.0);
                        ui.label(
                            egui::RichText::new("Kiriko")
                                .heading()
                                .color(theme.text_primary),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new("Start with footage or a composition.")
                                .color(theme.text_muted),
                        );
                        ui.add_space(12.0);
                        ui.vertical_centered_justified(|ui| {
                            let b = |t: &str| egui::Button::new(t).min_size(egui::vec2(0.0, 28.0));
                            if ui.add(b("Import footage")).clicked() {
                                app.import_footage_dialog();
                            }
                            if ui.add(b("New composition")).clicked() {
                                app.new_composition();
                            }
                            if ui.add(b("Open project")).clicked() {
                                app.open_dialog();
                            }
                        });
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("Footage can be dropped anywhere in the window.")
                                .small()
                                .color(theme.text_disabled),
                        );
                    });
            });
        });
    });

    // Viewer bar placeholder (bottom): preview resolution + magnification stubs.
    let bar = egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.max.y - 26.0), rect.max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
        egui::Frame::new()
            .fill(theme.surface_1)
            .stroke(egui::Stroke::new(1.0_f32, theme.hairline))
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Full")
                            .small()
                            .color(theme.text_secondary),
                    );
                    ui.label(egui::RichText::new("·").small().color(theme.text_disabled));
                    ui.label(
                        egui::RichText::new("Fit")
                            .small()
                            .color(theme.text_secondary),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("sRGB display")
                                .small()
                                .color(theme.text_muted),
                        );
                    });
                });
            });
    });
}

/// What a Project panel row asked for this frame (applied after drawing —
/// rows only read state, so the borrow stays clean).
enum PanelAction {
    Select(uuid::Uuid),
    OpenComp(uuid::Uuid),
    PreviewFootage(uuid::Uuid),
    MoveTo {
        item: uuid::Uuid,
        target: Option<uuid::Uuid>,
    },
    CompSettings(uuid::Uuid),
    Delete(uuid::Uuid),
}

/// AE-style Project panel (docs/07-UI-SPEC.md §4): selected-item info at the
/// top, the folder tree below, everything drag-and-drop (rows drag onto
/// folders to file them; onto the Timeline or Viewer to make layers).
fn project_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    let doc = app.store.snapshot();
    let mut actions: Vec<PanelAction> = Vec::new();

    project_header(ui, theme, app, &doc, &mut actions);
    ui.separator();
    ui.horizontal(|ui| {
        if ui.small_button("+ Folder").clicked() {
            app.new_folder();
        }
        if ui.small_button("+ Composition").clicked() {
            app.open_new_comp_dialog(None);
        }
    });
    ui.add_space(2.0);

    if doc.items.is_empty() {
        empty_hint(
            ui,
            theme,
            "No footage yet",
            "Drag files anywhere in the window, or use File → Import.",
        );
        return;
    }

    // The tree, with the panel background as a "move to root" drop target.
    let bg_rect = ui.available_rect_before_wrap();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut visited = Vec::new();
            for id in doc.root_items() {
                item_rows(ui, theme, app, &doc, id, 0, &mut actions, &mut visited);
            }
            // Trailing space so there is always a root drop area.
            ui.allocate_space(egui::vec2(ui.available_width(), 40.0));
        });
    let bg = ui.interact(
        bg_rect,
        ui.id().with("panel-root-drop"),
        egui::Sense::hover(),
    );
    if let Some(payload) = bg.dnd_release_payload::<uuid::Uuid>() {
        // Row/folder drops claim the pointer first; reaching here means the
        // drop landed on empty panel space.
        actions.push(PanelAction::MoveTo {
            item: *payload,
            target: None,
        });
    }

    for action in actions {
        match action {
            PanelAction::Select(id) => {
                app.selected_item = Some(id);
                if doc.comp(id).is_some() {
                    app.selected_comp = Some(id);
                }
            }
            PanelAction::OpenComp(id) => {
                app.selected_comp = Some(id);
                app.preview_comp = Some(id);
                app.preview_item = None;
                app.preview_frame = 0;
                #[cfg(feature = "media")]
                app.refresh_preview();
            }
            PanelAction::PreviewFootage(id) => {
                app.preview_item = Some(id);
                app.preview_comp = None;
                app.preview_frame = 0;
                #[cfg(feature = "media")]
                {
                    app.refresh_preview();
                    app.request_preview_audio();
                }
            }
            PanelAction::MoveTo { item, target } => app.move_item_to_folder(item, target),
            PanelAction::CompSettings(id) => app.open_comp_settings(id),
            PanelAction::Delete(id) => {
                app.commit(kiriko_core::Op::RemoveItem { id });
                if app.selected_item == Some(id) {
                    app.selected_item = None;
                }
            }
        }
    }
}

/// The info header: what the selected item is, at a glance (AE's preview
/// area — the thumbnail joins when the cache can supply one cheaply).
fn project_header(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &AppState,
    doc: &kiriko_core::model::Document,
    actions: &mut Vec<PanelAction>,
) {
    ui.add_space(4.0);
    let Some(item) = app.selected_item.and_then(|id| doc.item(id)) else {
        ui.label(
            egui::RichText::new("Nothing selected")
                .small()
                .color(theme.text_disabled),
        );
        ui.add_space(2.0);
        return;
    };
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(item.name())
                .strong()
                .color(theme.text_primary),
        );
        let kind = match item {
            ProjectItem::Footage(_) => "footage",
            ProjectItem::Folder(_) => "folder",
            ProjectItem::Composition(_) => "comp",
            ProjectItem::Solid(_) => "solid",
        };
        ui.label(
            egui::RichText::new(kind)
                .monospace()
                .small()
                .color(theme.text_muted),
        );
    });
    match item {
        ProjectItem::Composition(c) => {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{}×{} · {:.2} fps · {:.1} s · {} layer{}",
                        c.width,
                        c.height,
                        c.frame_rate.fps(),
                        c.duration.0.to_f64(),
                        c.layers.len(),
                        if c.layers.len() == 1 { "" } else { "s" }
                    ))
                    .monospace()
                    .small()
                    .color(theme.text_muted),
                );
                if ui.small_button("Settings…").clicked() {
                    actions.push(PanelAction::CompSettings(c.id));
                }
            });
        }
        ProjectItem::Solid(s) => {
            ui.horizontal(|ui| {
                let px = crate::pixels::solid_rgba(s.colour);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, crate::theme::document_colour(px));
                ui.label(
                    egui::RichText::new(format!("{}×{}", s.width, s.height))
                        .monospace()
                        .small()
                        .color(theme.text_muted),
                );
            });
        }
        ProjectItem::Folder(f) => {
            ui.label(
                egui::RichText::new(format!(
                    "{} item{}",
                    f.children.len(),
                    if f.children.len() == 1 { "" } else { "s" }
                ))
                .monospace()
                .small()
                .color(theme.text_muted),
            );
        }
        ProjectItem::Footage(_) => {
            #[cfg(feature = "media")]
            {
                use crate::app_state::media::MediaStatus;
                match app.media.map.get(&item.id()) {
                    Some(MediaStatus::Probing) => {
                        ui.label(
                            egui::RichText::new("probing…")
                                .small()
                                .color(theme.text_disabled),
                        );
                    }
                    Some(MediaStatus::Ready { probe, frames, vfr }) => {
                        let mut line = String::new();
                        if let Some(v) = &probe.video {
                            line.push_str(&format!(
                                "{}×{} · {:.2} fps · {} frames",
                                v.width,
                                v.height,
                                v.fps(),
                                frames
                            ));
                        } else if let Some(a) = &probe.audio {
                            line.push_str(&format!("{} Hz · {} ch", a.sample_rate, a.channels));
                        }
                        line.push_str(&format!(" · {:.1} s", probe.duration_seconds));
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(line)
                                    .monospace()
                                    .small()
                                    .color(theme.text_muted),
                            );
                            if *vfr {
                                ui.label(
                                    egui::RichText::new("VFR")
                                        .monospace()
                                        .small()
                                        .color(theme.warning),
                                )
                                .on_hover_text("Variable frame rate: conformed to the median rate");
                            }
                        });
                    }
                    Some(MediaStatus::Failed(e)) => {
                        ui.label(
                            egui::RichText::new(format!("unreadable: {e}"))
                                .small()
                                .color(theme.warning),
                        );
                    }
                    None => {}
                }
            }
        }
    }
    ui.add_space(2.0);
}

/// One tree row (folders recurse). Rows are drag sources; folder rows are
/// drop targets.
#[allow(clippy::too_many_arguments)]
fn item_rows(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &AppState,
    doc: &kiriko_core::model::Document,
    id: uuid::Uuid,
    depth: usize,
    actions: &mut Vec<PanelAction>,
    visited: &mut Vec<uuid::Uuid>,
) {
    if visited.contains(&id) {
        return; // defensive: malformed folder cycles never hang the panel
    }
    let Some(item) = doc.item(id) else {
        return; // stale child id (deleted item): just don't draw it
    };
    let is_folder = matches!(item, ProjectItem::Folder(_));
    let (kind, tag_colour) = match item {
        ProjectItem::Footage(_) => ("footage", theme.text_muted),
        ProjectItem::Folder(_) => ("folder", theme.text_muted),
        ProjectItem::Composition(_) => ("comp", theme.accent),
        ProjectItem::Solid(_) => ("solid", theme.text_muted),
    };
    let selected = app.selected_item == Some(id);
    let open_id = ui.id().with(("folder-open", id));
    let mut open = is_folder && ui.data(|d| d.get_temp::<bool>(open_id).unwrap_or(true));

    let row = ui
        .horizontal(|ui| {
            ui.add_space(12.0 * depth as f32 + 2.0);
            if is_folder {
                let arrow = if open { "▾" } else { "▸" };
                if ui
                    .add(
                        egui::Label::new(egui::RichText::new(arrow).small())
                            .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    open = !open;
                    ui.data_mut(|d| d.insert_temp(open_id, open));
                }
            }
            let label = egui::RichText::new(item.name()).color(theme.text_secondary);
            let resp = ui
                .dnd_drag_source(ui.id().with(("drag", id)), id, |ui| {
                    ui.selectable_label(selected, label)
                })
                .response;
            ui.painter().text(
                ui.max_rect().right_center() + egui::vec2(-4.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                kind,
                egui::FontId::monospace(10.0),
                tag_colour,
            );
            resp
        })
        .inner;

    if row.clicked() {
        actions.push(PanelAction::Select(id));
        match item {
            ProjectItem::Composition(_) => actions.push(PanelAction::OpenComp(id)),
            ProjectItem::Footage(_) => actions.push(PanelAction::PreviewFootage(id)),
            _ => {}
        }
    }
    row.context_menu(|ui| {
        if let ProjectItem::Composition(_) = item {
            if ui.button("Composition settings…").clicked() {
                actions.push(PanelAction::CompSettings(id));
                ui.close_menu();
            }
        }
        if ui.button("Move to root").clicked() {
            actions.push(PanelAction::MoveTo {
                item: id,
                target: None,
            });
            ui.close_menu();
        }
        if ui.button("Delete").clicked() {
            actions.push(PanelAction::Delete(id));
            ui.close_menu();
        }
    });

    if is_folder {
        // Folder rows accept drops (file the dragged item here).
        if let Some(payload) = row.dnd_release_payload::<uuid::Uuid>() {
            if *payload != id {
                actions.push(PanelAction::MoveTo {
                    item: *payload,
                    target: Some(id),
                });
            }
        } else if row.dnd_hover_payload::<uuid::Uuid>().is_some() {
            ui.painter().rect_stroke(
                row.rect,
                2.0,
                egui::Stroke::new(1.0_f32, theme.accent),
                egui::StrokeKind::Inside,
            );
        }
        if open {
            if let Some(f) = doc.folder(id) {
                visited.push(id);
                for child in f.children.clone() {
                    item_rows(ui, theme, app, doc, child, depth + 1, actions, visited);
                }
                visited.pop();
            }
        }
    }
}

/// Accept a Project-panel item dropped on this panel's area: file it into
/// the active comp as a layer, or — with no comp yet — open the composition
/// dialogue pre-filled from the footage (K-068).
fn accept_item_drop(ui: &egui::Ui, theme: &Theme, app: &mut AppState, rect: egui::Rect) {
    let zone = ui.interact(rect, ui.id().with("item-drop-zone"), egui::Sense::hover());
    if zone.dnd_hover_payload::<uuid::Uuid>().is_some() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            2.0,
            egui::Stroke::new(1.0_f32, theme.accent),
            egui::StrokeKind::Inside,
        );
    }
    let Some(payload) = zone.dnd_release_payload::<uuid::Uuid>() else {
        return;
    };
    let item = *payload;
    let doc = app.store.snapshot();
    let has_comp = app
        .preview_comp
        .or(app.selected_comp)
        .and_then(|id| doc.comp(id))
        .is_some();
    match doc.item(item) {
        Some(ProjectItem::Composition(_)) if !has_comp => {
            // Dropping a comp with nothing open just opens it.
            app.selected_comp = Some(item);
            app.preview_comp = Some(item);
            app.preview_item = None;
            #[cfg(feature = "media")]
            app.refresh_preview();
        }
        Some(ProjectItem::Footage(_)) if !has_comp => {
            app.open_new_comp_dialog(Some(item));
        }
        Some(ProjectItem::Folder(_)) | None => {}
        Some(_) if !has_comp => {
            app.error = Some("create a composition first".into());
        }
        Some(_) => app.add_item_to_comp(item),
    }
}

fn timeline_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    accept_item_drop(ui, theme, app, ui.max_rect());
    let doc = app.store.snapshot();
    let comp = app.selected_comp.and_then(|id| doc.comp(id));
    let Some(comp) = comp else {
        empty_hint(
            ui,
            theme,
            "No composition open",
            "Create one with Composition → New, or drag footage here from the Project panel.",
        );
        return;
    };
    let comp_id = comp.id;
    // The dock tab already names the open comp; no redundant in-panel title,
    // resolution or frame-rate line here (Mack).
    if comp.layers.is_empty() {
        ui.label(
            egui::RichText::new("Drag footage here to create the first layer.")
                .small()
                .color(theme.text_muted),
        );
        return;
    }
    use kiriko_core::anim::Animation;
    use kiriko_core::model::TransformProp;
    let mut pending: Option<kiriko_core::Op> = None;

    // ---- ruler + time geometry (07-UI-SPEC Timeline) --------------------
    let panel_left = ui.max_rect().left();
    let panel_right = ui.max_rect().right();
    // Draggable left-column width (Mack): clamp so it can't swallow the track.
    let name_w = app
        .timeline_name_w
        .clamp(96.0, (panel_right - panel_left - 120.0).max(96.0));
    let duration = comp.duration.0.to_f64().max(1e-6);
    let frames = app.comp_frame_count(comp).max(1);
    let track_left = panel_left + name_w;
    let track_w = (panel_right - track_left - 8.0).max(40.0);
    let x_of = |seconds: f64| track_left + (seconds / duration) as f32 * track_w;

    let (ruler_rect, ruler_resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 20.0),
        egui::Sense::click_and_drag(),
    );
    ui.painter().rect_filled(ruler_rect, 0.0, theme.surface_2);
    if let Some((a, b)) = comp.work_area {
        let band = egui::Rect::from_min_max(
            egui::pos2(x_of(a.0.to_f64()), ruler_rect.top()),
            egui::pos2(x_of(b.0.to_f64()), ruler_rect.top() + 4.0),
        );
        ui.painter().rect_filled(band, 0.0, theme.success);
    }
    // Kura cache bar: mint runs along the ruler's base where frames are
    // banked (never a warning colour — an empty bar is normal, not a fault).
    #[cfg(feature = "media")]
    if let Some(bars) = app.cache_bar(comp) {
        let fps = comp.frame_rate.fps().max(1.0);
        let mut run_start: Option<usize> = None;
        for f in 0..=bars.len() {
            let cached = f < bars.len() && bars[f];
            match (cached, run_start) {
                (true, None) => run_start = Some(f),
                (false, Some(s)) => {
                    let band = egui::Rect::from_min_max(
                        egui::pos2(x_of(s as f64 / fps), ruler_rect.bottom() - 2.0),
                        egui::pos2(x_of(f as f64 / fps), ruler_rect.bottom()),
                    );
                    ui.painter().rect_filled(band, 0.0, theme.success);
                    run_start = None;
                }
                _ => {}
            }
        }
    }

    let label_every = (duration / 10.0).ceil().max(1.0) as usize;
    for s in 0..=duration.floor() as usize {
        let x = x_of(s as f64);
        ui.painter().line_segment(
            [
                egui::pos2(x, ruler_rect.bottom() - 6.0),
                egui::pos2(x, ruler_rect.bottom()),
            ],
            egui::Stroke::new(1.0_f32, theme.hairline_strong),
        );
        if s % label_every == 0 {
            ui.painter().text(
                egui::pos2(x + 3.0, ruler_rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{s}s"),
                egui::FontId::monospace(9.0),
                theme.text_muted,
            );
        }
    }
    if ruler_resp.clicked() || ruler_resp.dragged() {
        if let Some(pos) = ruler_resp.interact_pointer_pos() {
            let frac = ((pos.x - track_left) / track_w).clamp(0.0, 1.0) as f64;
            app.preview_comp = Some(comp_id);
            app.comp_playback = None; // scrubbing pauses
            app.preview_frame = ((frac * frames as f64) as usize).min(frames.saturating_sub(1));
            // Dragging is scrubbing: decode a coarse draft for instant feedback.
            // A plain click jumps once and wants the specified resolution.
            app.preview_draft = ruler_resp.dragged();
            #[cfg(feature = "media")]
            app.refresh_preview();
        }
    }
    if ruler_resp.drag_stopped() {
        // Scrub finished: reload the frame at the specified resolution.
        app.preview_draft = false;
        #[cfg(feature = "media")]
        app.refresh_preview();
    }
    let rows_top = ui.cursor().top();

    // Graph mode (K-070): the right area becomes the curve editor. The ruler
    // above stays for scrubbing; the bottom-right toggle swaps the two views.
    // (Per-property rows and a shared x-axis are the next step.)
    if app.timeline_graph_mode {
        graph_editor_panel(ui, theme, app);
        timeline_mode_toggle(ui, theme, app);
        return;
    }

    for layer in &comp.layers {
        let (row_rect, row_resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::click());
        if row_resp.clicked() {
            app.selected_layer = Some(layer.id);
        }
        // Right-click a layer to add things (the house pattern: right-click or
        // menu, never scattered buttons).
        let mut ctx_op: Option<kiriko_core::Op> = None;
        let mut convert_layer = false;
        row_resp.context_menu(|ui| {
            ui.menu_button("Add mask", |ui| {
                let (w, h) = mask_space(layer, app, comp);
                let mut new_mask = None;
                if ui.button("Rectangle").clicked() {
                    new_mask = Some(kiriko_core::mask::Mask::rectangle(
                        w * 0.25,
                        h * 0.25,
                        w * 0.5,
                        h * 0.5,
                    ));
                    ui.close_menu();
                }
                if ui.button("Ellipse").clicked() {
                    new_mask = Some(kiriko_core::mask::Mask::ellipse(
                        w * 0.5,
                        h * 0.5,
                        w * 0.3,
                        h * 0.3,
                    ));
                    ui.close_menu();
                }
                if ui.button("Star").clicked() {
                    new_mask = Some(kiriko_core::mask::Mask::star(
                        w * 0.5,
                        h * 0.5,
                        w * 0.32,
                        w * 0.14,
                        5,
                    ));
                    ui.close_menu();
                }
                if let Some(m) = new_mask {
                    let mut masks = layer.masks.clone();
                    masks.push(m);
                    ctx_op = Some(kiriko_core::Op::SetLayerMasks {
                        comp: comp_id,
                        layer: layer.id,
                        masks,
                    });
                }
            });
            // Footage → sequenced layer (K-071).
            if matches!(layer.kind, kiriko_core::model::LayerKind::Footage { .. })
                && ui.button("Convert to sequenced layer").clicked()
            {
                convert_layer = true;
                ui.close_menu();
            }
        });
        if ctx_op.is_some() {
            pending = ctx_op;
            app.selected_layer = Some(layer.id);
        }
        if convert_layer {
            app.selected_layer = Some(layer.id);
            app.convert_to_sequenced_layer();
        }
        // Disclosure twirl: layer options hide until opened (AE behaviour).
        let twirl_id = ui.id().with(("twirl", layer.id));
        let mut expanded = ui.data(|d| d.get_temp::<bool>(twirl_id).unwrap_or(false));
        let tri = egui::Rect::from_min_size(
            egui::pos2(row_rect.left() + 2.0, row_rect.top()),
            egui::vec2(16.0, row_rect.height()),
        );
        let tri_resp = ui.interact(tri, twirl_id.with("hit"), egui::Sense::click());
        if tri_resp.clicked() {
            expanded = !expanded;
            ui.data_mut(|d| d.insert_temp(twirl_id, expanded));
        }
        ui.painter().text(
            tri.center(),
            egui::Align2::CENTER_CENTER,
            if expanded { "▾" } else { "▸" },
            egui::FontId::proportional(11.0),
            theme.text_muted,
        );
        if app.selected_layer == Some(layer.id) {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    row_rect.min,
                    egui::pos2(row_rect.left() + name_w - 4.0, row_rect.bottom()),
                ),
                3.0,
                theme.surface_2,
            );
        }
        let seconds_of = |x: f32| ((x - track_left) / track_w).clamp(0.0, 1.0) as f64 * duration;
        // Left-column subcolumns (Mack): [visibility][title…][matte][blend][3D]
        // [mute]. Switches are right-anchored so they align across every row;
        // the title flexes and truncates. Each is clipped to its slot, so a
        // narrow column just crops controls off the edge.
        let top = row_rect.top();
        let bot = row_rect.bottom();
        let edge = track_left - 6.0;
        let slot = |x0: f32, x1: f32| {
            egui::Rect::from_min_max(
                egui::pos2(x0.max(row_rect.left() + 18.0), top),
                egui::pos2(x1.max(x0 + 1.0), bot),
            )
        };
        let is_footage = matches!(layer.kind, kiriko_core::model::LayerKind::Footage { .. });
        let eye_r = slot(row_rect.left() + 18.0, row_rect.left() + 36.0);
        let mute_r = slot(edge - 34.0, edge);
        let td_r = slot(edge - 60.0, edge - 38.0);
        let blend_r = slot(edge - 124.0, edge - 64.0);
        let matte_r = slot(edge - 178.0, edge - 128.0);
        let title_r = slot(row_rect.left() + 40.0, edge - 182.0);
        let place = |ui: &mut egui::Ui, r: egui::Rect, add: &mut dyn FnMut(&mut egui::Ui)| {
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(r)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            child.set_clip_rect(r);
            add(&mut child);
        };
        let mut select_this = false;
        place(ui, eye_r, &mut |ui| {
            visible_control(ui, theme, comp_id, layer, &mut pending)
        });
        place(ui, title_r, &mut |ui| {
            if ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(trim_title(&layer.name))
                            .small()
                            .color(theme.text_secondary),
                    )
                    .truncate()
                    .sense(egui::Sense::click()),
                )
                .clicked()
            {
                select_this = true;
            }
        });
        place(ui, matte_r, &mut |ui| {
            matte_control(ui, comp, comp_id, layer, &mut pending)
        });
        place(ui, blend_r, &mut |ui| {
            blend_control(ui, comp_id, layer, &mut pending)
        });
        place(ui, td_r, &mut |ui| {
            three_d_control(ui, comp_id, layer, &mut pending)
        });
        if is_footage {
            place(ui, mute_r, &mut |ui| {
                mute_control(ui, comp_id, layer, &mut pending)
            });
        }
        if select_this {
            app.selected_layer = Some(layer.id);
        }
        let bar = egui::Rect::from_min_max(
            egui::pos2(x_of(layer.in_point.0.to_f64()), row_rect.top() + 2.0),
            egui::pos2(x_of(layer.out_point.0.to_f64()), row_rect.bottom() - 2.0),
        );
        ui.painter().rect(
            bar,
            3.0,
            theme.surface_3,
            egui::Stroke::new(1.0_f32, theme.hairline_strong),
            egui::StrokeKind::Inside,
        );
        if layer.matte.is_some() {
            ui.painter().text(
                egui::pos2(bar.right() - 4.0, bar.center().y),
                egui::Align2::RIGHT_CENTER,
                "matte",
                egui::FontId::monospace(8.0),
                theme.text_muted,
            );
        }
        // Keyframe glyphs: a clay diamond on the bar at each keyframed time
        // (across the layer's animated properties). Times are layer-local, so
        // comp time = start_offset + keyframe time.
        {
            let off = layer.start_offset.0.to_f64();
            let cy = bar.center().y;
            for kt in layer_keyframe_times(layer) {
                let x = x_of(off + kt);
                if x >= bar.left() - 1.0 && x <= bar.right() + 1.0 {
                    let d = 3.5;
                    ui.painter().add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(x, cy - d),
                            egui::pos2(x + d, cy),
                            egui::pos2(x, cy + d),
                            egui::pos2(x - d, cy),
                        ],
                        theme.accent,
                        egui::Stroke::new(1.0_f32, theme.surface_0),
                    ));
                }
            }
        }
        // Sequence layers show their clips as sub-bars; gaps show the darker
        // base bar; each edit point gets a clay tick.
        if let kiriko_core::model::LayerKind::Sequence { clips } = &layer.kind {
            let off = layer.start_offset.0.to_f64();
            for clip in clips {
                let cs = x_of(off + clip.place_start.to_f64());
                let ce = x_of(off + clip.place_end().to_f64());
                ui.painter().rect(
                    egui::Rect::from_min_max(
                        egui::pos2(cs, bar.top() + 1.0),
                        egui::pos2(ce, bar.bottom() - 1.0),
                    ),
                    2.0,
                    theme.surface_2,
                    egui::Stroke::new(1.0_f32, theme.hairline_strong),
                    egui::StrokeKind::Inside,
                );
                // Edit point (clip boundary) — the beat-sync landmark.
                ui.painter().line_segment(
                    [egui::pos2(ce, bar.top()), egui::pos2(ce, bar.bottom())],
                    egui::Stroke::new(1.0_f32, theme.accent),
                );
            }
        }

        // Edge handles: drag to trim in/out (one SetLayerSpan op per release).
        for out_edge in [false, true] {
            let edge_x = if out_edge { bar.right() } else { bar.left() };
            let handle = egui::Rect::from_center_size(
                egui::pos2(edge_x, bar.center().y),
                egui::vec2(8.0, bar.height()),
            );
            let resp = ui.interact(
                handle,
                ui.id().with(("trim", layer.id, out_edge)),
                egui::Sense::drag(),
            );
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    app.trim_edit = Some((layer.id, out_edge, seconds_of(pos.x)));
                }
            }
            if resp.drag_stopped() {
                if let Some((id, is_out, secs)) = app.trim_edit.take() {
                    if id == layer.id && is_out == out_edge {
                        let (mut new_in, mut new_out) =
                            (layer.in_point.0.to_f64(), layer.out_point.0.to_f64());
                        if is_out {
                            new_out = secs;
                        } else {
                            new_in = secs;
                        }
                        let min_len = 1.0 / comp.frame_rate.fps().max(1.0);
                        if new_out - new_in >= min_len {
                            pending = Some(kiriko_core::Op::SetLayerSpan {
                                comp: comp_id,
                                layer: layer.id,
                                in_point: kiriko_core::time::CompTime(rational_at(new_in)),
                                out_point: kiriko_core::time::CompTime(rational_at(new_out)),
                                start_offset: layer.start_offset,
                            });
                        }
                    }
                }
            }
        }
        // Live trim feedback: provisional edge drawn in clay.
        if let Some((id, _is_out, secs)) = app.trim_edit {
            if id == layer.id {
                let x = x_of(secs);
                ui.painter().line_segment(
                    [egui::pos2(x, bar.top()), egui::pos2(x, bar.bottom())],
                    egui::Stroke::new(2.0_f32, theme.accent),
                );
            }
        }
        if expanded {
            // Layer options live in the left column only — the track area to
            // the right of the separator stays for the bar and keyframes.
            ui.scope(|ui| {
                ui.set_max_width(name_w - 10.0);
                ui.indent(("layer-opts", layer.id), |ui| {
                    // Masks only appear once the layer has one (add via right-click or
                    // the toolbar's mask tool).
                    if !layer.masks.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new("Masks").small().color(theme.text_muted));
                            for (mi, mask) in layer.masks.iter().enumerate() {
                                let mut masks = layer.masks.clone();
                                if ui
                                    .selectable_label(
                                        mask.inverted,
                                        egui::RichText::new(format!("{} inv", mask.name)).small(),
                                    )
                                    .clicked()
                                {
                                    masks[mi].inverted = !masks[mi].inverted;
                                    pending = Some(kiriko_core::Op::SetLayerMasks {
                                        comp: comp_id,
                                        layer: layer.id,
                                        masks,
                                    });
                                } else if ui
                                    .small_button("×")
                                    .on_hover_text("Remove mask")
                                    .clicked()
                                {
                                    masks.remove(mi);
                                    pending = Some(kiriko_core::Op::SetLayerMasks {
                                        comp: comp_id,
                                        layer: layer.id,
                                        masks,
                                    });
                                }
                            }
                        });
                    }
                });
                if let kiriko_core::model::LayerKind::Text { document } = &layer.kind {
                    ui.indent(("text", layer.id), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Text").small().color(theme.text_muted));
                            let mut text = document.text.clone();
                            let resp =
                                ui.add(egui::TextEdit::singleline(&mut text).desired_width(180.0));
                            let mut size = document.size;
                            let size_resp = ui.add(
                                egui::DragValue::new(&mut size)
                                    .speed(1.0)
                                    .range(4.0..=512.0)
                                    .suffix(" px"),
                            );
                            if (resp.lost_focus() && text != document.text)
                                || (size_resp.drag_stopped() || size_resp.lost_focus())
                                    && (size - document.size).abs() > f64::EPSILON
                            {
                                let mut doc_new = document.clone();
                                doc_new.text = text;
                                doc_new.size = size;
                                pending = Some(kiriko_core::Op::SetTextDocument {
                                    comp: comp_id,
                                    layer: layer.id,
                                    document: doc_new,
                                });
                            }
                        });
                    });
                }
                if let kiriko_core::model::LayerKind::Camera { zoom } = &layer.kind {
                    ui.indent(("camera", layer.id), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Zoom px")
                                    .small()
                                    .color(theme.text_muted),
                            );
                            let fps = comp.frame_rate.fps().max(1.0);
                            let lt = app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
                            let committed = zoom.value_at(lt);
                            let id = egui::Id::new(("zoom_edit", layer.id));
                            let mut value = ui.data(|d| d.get_temp::<f64>(id)).unwrap_or(committed);
                            let resp = ui.add(
                                egui::DragValue::new(&mut value)
                                    .speed(4.0)
                                    .range(1.0..=100_000.0)
                                    .max_decimals(1),
                            );
                            if resp.dragged() || resp.has_focus() {
                                ui.data_mut(|d| d.insert_temp(id, value));
                            }
                            if resp.drag_stopped() || resp.lost_focus() {
                                if (value - committed).abs() > f64::EPSILON {
                                    let animation = if zoom.is_animated() {
                                        Animation::Keyframed(upsert_key(zoom, lt, value))
                                    } else {
                                        Animation::Static(value)
                                    };
                                    pending = Some(kiriko_core::Op::SetCameraZoom {
                                        comp: comp_id,
                                        layer: layer.id,
                                        animation,
                                    });
                                }
                                ui.data_mut(|d| d.remove::<f64>(id));
                            }
                        });
                    });
                }
                ui.indent(("transform", layer.id), |ui| {
                    egui::CollapsingHeader::new(
                        egui::RichText::new("Transform")
                            .small()
                            .color(theme.text_muted),
                    )
                    .id_salt(("transform-hdr", layer.id))
                    .default_open(true)
                    .show(ui, |ui| {
                        // Footage speed: single value, here in Transform.
                        if let kiriko_core::model::LayerKind::Footage { retime, .. } = &layer.kind {
                            speed_row(ui, theme, comp_id, layer, retime, &mut pending);
                        }
                        egui::Grid::new(("txgrid", layer.id))
                            .num_columns(2)
                            .spacing(egui::vec2(12.0, 2.0))
                            .show(ui, |ui| {
                                let is_camera = matches!(
                                    layer.kind,
                                    kiriko_core::model::LayerKind::Camera { .. }
                                );
                                let mut rows: Vec<(&str, TransformProp, f64)> = Vec::new();
                                // Origin (anchor): the point transforms pivot
                                // about. First, like AE. Cameras have no anchor.
                                if !is_camera {
                                    rows.push(("Anchor x", TransformProp::AnchorX, 1.0));
                                    rows.push(("Anchor y", TransformProp::AnchorY, 1.0));
                                }
                                rows.extend([
                                    ("Position x", TransformProp::PositionX, 1.0),
                                    ("Position y", TransformProp::PositionY, 1.0),
                                    ("Scale x %", TransformProp::ScaleX, 0.5),
                                    ("Scale y %", TransformProp::ScaleY, 0.5),
                                    ("Rotation °", TransformProp::Rotation, 0.5),
                                    ("Opacity %", TransformProp::Opacity, 0.5),
                                ]);
                                if layer.switches.three_d || is_camera {
                                    rows.extend([
                                        ("Position z", TransformProp::PositionZ, 1.0),
                                        ("Rotation x °", TransformProp::RotationX, 0.5),
                                        ("Rotation y °", TransformProp::RotationY, 0.5),
                                    ]);
                                }
                                // Layer time at the playhead: where keyframes land
                                // (AE behaviour: editing an animated value writes a
                                // key at the current time).
                                let fps = comp.frame_rate.fps().max(1.0);
                                let lt =
                                    app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
                                for (label, prop, speed) in rows {
                                    let slot = layer.transform.get(prop);
                                    let animated = slot.is_animated();
                                    ui.horizontal(|ui| {
                                        let clock = if animated { "⏱" } else { "◦" };
                                        if ui
                                            .selectable_label(
                                                animated,
                                                egui::RichText::new(clock).small(),
                                            )
                                            .on_hover_text(if animated {
                                                "Remove animation (freeze current value)"
                                            } else {
                                                "Animate: keyframe at the playhead"
                                            })
                                            .clicked()
                                        {
                                            let animation = if animated {
                                                Animation::Static(slot.value_at(lt))
                                            } else {
                                                Animation::Keyframed(vec![
                                                    kiriko_core::anim::Keyframe {
                                                        time: rational_at(lt),
                                                        value: slot.value_at(lt),
                                                        interp_in:
                                                            kiriko_core::anim::SideInterp::Linear,
                                                        interp_out:
                                                            kiriko_core::anim::SideInterp::Linear,
                                                    },
                                                ])
                                            };
                                            pending = Some(kiriko_core::Op::SetTransformProperty {
                                                comp: comp_id,
                                                layer: layer.id,
                                                prop,
                                                animation,
                                            });
                                        }
                                        ui.label(
                                            egui::RichText::new(label)
                                                .small()
                                                .color(theme.text_muted),
                                        );
                                    });
                                    {
                                        let committed = slot.value_at(lt);
                                        let mut value = match app.prop_edit {
                                            Some((l, p, v)) if l == layer.id && p == prop => v,
                                            _ => committed,
                                        };
                                        let resp = ui.add(
                                            egui::DragValue::new(&mut value)
                                                .speed(speed)
                                                .max_decimals(2),
                                        );
                                        if resp.dragged() || resp.has_focus() {
                                            app.prop_edit = Some((layer.id, prop, value));
                                        }
                                        if resp.drag_stopped() || resp.lost_focus() {
                                            if (value - committed).abs() > f64::EPSILON {
                                                let animation = if animated {
                                                    Animation::Keyframed(upsert_key(
                                                        slot, lt, value,
                                                    ))
                                                } else {
                                                    Animation::Static(value)
                                                };
                                                pending =
                                                    Some(kiriko_core::Op::SetTransformProperty {
                                                        comp: comp_id,
                                                        layer: layer.id,
                                                        prop,
                                                        animation,
                                                    });
                                            }
                                            app.prop_edit = None;
                                        }
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                });
            });
        }
    }
    // Vertical separator + drag handle: resizes the left column (Mack).
    let sep_bottom = ui.cursor().top();
    let sep_x = track_left - 4.0;
    let handle = egui::Rect::from_min_max(
        egui::pos2(sep_x - 3.0, rows_top),
        egui::pos2(sep_x + 3.0, sep_bottom.max(rows_top + 1.0)),
    );
    let hresp = ui.interact(
        handle,
        ui.id().with("name-col-resize"),
        egui::Sense::click_and_drag(),
    );
    if hresp.hovered() || hresp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if hresp.dragged() {
        app.timeline_name_w = (app.timeline_name_w + hresp.drag_delta().x).clamp(96.0, 900.0);
    }
    ui.painter().line_segment(
        [egui::pos2(sep_x, rows_top), egui::pos2(sep_x, sep_bottom)],
        egui::Stroke::new(
            1.0_f32,
            if hresp.hovered() || hresp.dragged() {
                theme.accent
            } else {
                theme.hairline
            },
        ),
    );
    // Playhead over ruler and rows (clay, the one accent).
    if app.preview_comp == Some(comp_id) {
        let x = x_of(app.preview_frame as f64 / comp.frame_rate.fps().max(1.0));
        ui.painter().line_segment(
            [
                egui::pos2(x, ruler_rect.top()),
                egui::pos2(x, ui.cursor().top().max(rows_top)),
            ],
            egui::Stroke::new(1.5_f32, theme.accent),
        );
    }
    if let Some(op) = pending {
        app.commit(op);
    }
    timeline_mode_toggle(ui, theme, app);
}

/// Layer-view / graph-view switch, bottom-right of the Timeline (K-070). Small
/// glyphs for now; the designed icons come later.
fn timeline_mode_toggle(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    let panel = ui.max_rect();
    let r = egui::Rect::from_min_max(
        egui::pos2(panel.right() - 58.0, panel.bottom() - 22.0),
        egui::pos2(panel.right() - 6.0, panel.bottom() - 4.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(r)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.set_clip_rect(r);
    let graph = app.timeline_graph_mode;
    if child
        .selectable_label(graph, egui::RichText::new("〜").small())
        .on_hover_text("Graph editor")
        .clicked()
    {
        app.timeline_graph_mode = true;
    }
    if child
        .selectable_label(!graph, egui::RichText::new("▤").small())
        .on_hover_text("Layers")
        .clicked()
    {
        app.timeline_graph_mode = false;
    }
    let _ = theme;
}

/// Footage preview: the frame fit to the surround, scrub bar, resolution picker.
#[cfg(feature = "media")]
/// The layer↔screen mapping the Viewer overlays share: the layer's evaluated
/// 2D transform at the playhead, then the view placement.
#[cfg(feature = "media")]
struct LayerMap {
    px: f64,
    py: f64,
    ax: f64,
    ay: f64,
    sx: f64,
    sy: f64,
    sin: f64,
    cos: f64,
    origin: egui::Pos2,
    view_scale: f32,
}

#[cfg(feature = "media")]
impl LayerMap {
    fn of(layer: &kiriko_core::model::Layer, lt: f64, draw: egui::Rect, scale: f32) -> Self {
        let tr = &layer.transform;
        let rot = tr.rotation.value_at(lt).to_radians();
        let (sin, cos) = rot.sin_cos();
        Self {
            px: tr.position_x.value_at(lt),
            py: tr.position_y.value_at(lt),
            ax: tr.anchor_x.value_at(lt),
            ay: tr.anchor_y.value_at(lt),
            sx: (tr.scale_x.value_at(lt) / 100.0).max(1e-6),
            sy: (tr.scale_y.value_at(lt) / 100.0).max(1e-6),
            sin,
            cos,
            origin: draw.min,
            view_scale: scale,
        }
    }

    /// Layer space → screen.
    fn to_screen(&self, p: (f64, f64)) -> egui::Pos2 {
        let (dx, dy) = ((p.0 - self.ax) * self.sx, (p.1 - self.ay) * self.sy);
        let (rx, ry) = (dx * self.cos - dy * self.sin, dx * self.sin + dy * self.cos);
        self.origin + egui::vec2((self.px + rx) as f32, (self.py + ry) as f32) * self.view_scale
    }

    /// Screen → layer space (drag and pen positions come back through this).
    fn layer_of(&self, pos: egui::Pos2) -> (f64, f64) {
        let c = (pos - self.origin) / self.view_scale;
        let (dx, dy) = (f64::from(c.x) - self.px, f64::from(c.y) - self.py);
        let (rx, ry) = (
            dx * self.cos + dy * self.sin,
            -dx * self.sin + dy * self.cos,
        );
        (rx / self.sx + self.ax, ry / self.sy + self.ay)
    }
}

/// Mask outlines and draggable vertices over the previewed comp — the seed
/// of the pen tool (07-UI-SPEC §Viewer tools). Outline follows the cursor
/// mid-drag; pixels update on release (one SetLayerMasks per drag, one undo).
#[cfg(feature = "media")]
fn mask_overlay(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    draw: egui::Rect,
    scale: f32,
) {
    let Some(comp_id) = app.preview_comp else {
        return;
    };
    let Some(layer_id) = app.selected_layer else {
        return;
    };
    let doc = app.store.snapshot();
    let Some(comp) = doc.comp(comp_id) else {
        return;
    };
    let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
        return;
    };
    if layer.masks.is_empty() || layer.switches.three_d {
        return; // 3D overlay geometry arrives with the object tools
    }
    let fps = comp.frame_rate.fps().max(1.0);
    let lt = app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
    let map = LayerMap::of(layer, lt, draw, scale);
    let to_screen = |p: (f64, f64)| map.to_screen(p);
    let from_screen = |pos: egui::Pos2| map.layer_of(pos);

    let stroke = egui::Stroke::new(1.0_f32, theme.accent);
    let mut committed: Option<Vec<kiriko_core::mask::Mask>> = None;
    for (mi, mask) in layer.masks.iter().enumerate() {
        // The path being dragged previews with the moved vertex.
        let mut path = mask.path.clone();
        if let Some((dm, dv, pos)) = app.mask_drag {
            if dm == mi && dv < path.vertices.len() {
                path.vertices[dv].pos = pos;
            }
        }
        // Flatten each cubic span exactly as the rasteriser does.
        let n = path.vertices.len();
        if n >= 2 {
            let mut points: Vec<egui::Pos2> = Vec::with_capacity(n * 24 + 1);
            for i in 0..n {
                let a = &path.vertices[i];
                let b = &path.vertices[(i + 1) % n];
                for s in 0..24 {
                    let t = f64::from(s) / 24.0;
                    let u = 1.0 - t;
                    let p0 = a.pos;
                    let p1 = (a.pos.0 + a.tan_out.0, a.pos.1 + a.tan_out.1);
                    let p2 = (b.pos.0 + b.tan_in.0, b.pos.1 + b.tan_in.1);
                    let p3 = b.pos;
                    let x = u * u * u * p0.0
                        + 3.0 * u * u * t * p1.0
                        + 3.0 * u * t * t * p2.0
                        + t * t * t * p3.0;
                    let y = u * u * u * p0.1
                        + 3.0 * u * u * t * p1.1
                        + 3.0 * u * t * t * p2.1
                        + t * t * t * p3.1;
                    points.push(to_screen((x, y)));
                }
            }
            if let Some(first) = points.first().copied() {
                points.push(first);
            }
            ui.painter().add(egui::Shape::line(points, stroke));
        }
        // Vertex handles: 8px squares, draggable.
        for (vi, v) in path.vertices.iter().enumerate() {
            let centre = to_screen(v.pos);
            let handle = egui::Rect::from_center_size(centre, egui::vec2(8.0, 8.0));
            let resp = ui.interact(
                handle,
                ui.id().with(("mask-vtx", layer_id, mi, vi)),
                egui::Sense::click_and_drag(),
            );
            let active = app
                .mask_drag
                .is_some_and(|(dm, dv, _)| dm == mi && dv == vi);
            ui.painter().rect_filled(
                handle.shrink(if active || resp.hovered() { 0.0 } else { 2.0 }),
                1.0,
                theme.accent,
            );
            if resp.drag_started() || resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    app.mask_drag = Some((mi, vi, from_screen(pos)));
                }
            }
            if resp.drag_stopped() {
                if let Some((dm, dv, pos)) = app.mask_drag.take() {
                    if dm == mi && dv == vi {
                        let mut masks = layer.masks.clone();
                        if let Some(vtx) =
                            masks.get_mut(dm).and_then(|m| m.path.vertices.get_mut(dv))
                        {
                            vtx.pos = pos;
                            committed = Some(masks);
                        }
                    }
                }
            }
            // Right-click removes the vertex (a closed path keeps ≥ 3).
            if resp.secondary_clicked() && path.vertices.len() > 3 {
                let mut masks = layer.masks.clone();
                if let Some(m) = masks.get_mut(mi) {
                    if vi < m.path.vertices.len() {
                        m.path.vertices.remove(vi);
                        committed = Some(masks);
                    }
                }
            }
        }
    }
    if let Some(masks) = committed {
        app.commit(kiriko_core::Op::SetLayerMasks {
            comp: comp_id,
            layer: layer_id,
            masks,
        });
        app.refresh_preview();
    }
}

/// Pen tool (slice 2): while armed, Viewer clicks place vertices of a new
/// mask on the selected layer; clicking the first vertex closes it into a
/// mask (one undo); Escape cancels.
/// Draw the selected 2D layer's origin (anchor point) as a crosshair, and let
/// it be dragged — AE's pan-behind: moving the origin keeps the layer visually
/// fixed (position compensates), committed as one undo step.
#[cfg(feature = "media")]
fn anchor_overlay(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    draw: egui::Rect,
    scale: f32,
) {
    use kiriko_core::anim::Animation;
    use kiriko_core::model::TransformProp;
    let Some(comp_id) = app.preview_comp else {
        return;
    };
    let doc = app.store.snapshot();
    let Some(comp) = doc.comp(comp_id) else {
        return;
    };
    let Some(layer) = app
        .selected_layer
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))
    else {
        app.origin_drag = None;
        return;
    };
    if layer.switches.three_d || matches!(layer.kind, kiriko_core::model::LayerKind::Camera { .. })
    {
        return; // 3D anchor gizmo arrives with the object tools
    }
    let fps = comp.frame_rate.fps().max(1.0);
    let lt = app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
    let map = LayerMap::of(layer, lt, draw, scale);
    let tr = &layer.transform;
    let anchor = (tr.anchor_x.value_at(lt), tr.anchor_y.value_at(lt));

    // The crosshair sits at the live anchor (or the dragged one).
    let shown_anchor = app.origin_drag.unwrap_or(anchor);
    let c = map.to_screen(shown_anchor);
    let handle = egui::Rect::from_center_size(c, egui::vec2(18.0, 18.0));
    let resp = ui.interact(
        handle,
        ui.id().with(("origin", layer.id)),
        egui::Sense::click_and_drag(),
    );
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
    }
    if resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            app.origin_drag = Some(map.layer_of(pos));
        }
    }
    if resp.drag_stopped() {
        if let Some(new_anchor) = app.origin_drag.take() {
            let position = (tr.position_x.value_at(lt), tr.position_y.value_at(lt));
            let scale_pct = (tr.scale_x.value_at(lt), tr.scale_y.value_at(lt));
            let new_pos = crate::app_state::pan_behind_position(
                anchor,
                new_anchor,
                position,
                scale_pct,
                tr.rotation.value_at(lt),
            );
            // One op per touched property; if the property is animated, write a
            // key at the playhead, else move the static value.
            let mk = |prop: TransformProp, value: f64| {
                let slot = tr.get(prop);
                let animation = if slot.is_animated() {
                    Animation::Keyframed(upsert_key(slot, lt, value))
                } else {
                    Animation::Static(value)
                };
                kiriko_core::Op::SetTransformProperty {
                    comp: comp_id,
                    layer: layer.id,
                    prop,
                    animation,
                }
            };
            let ops = vec![
                mk(TransformProp::AnchorX, new_anchor.0),
                mk(TransformProp::AnchorY, new_anchor.1),
                mk(TransformProp::PositionX, new_pos.0),
                mk(TransformProp::PositionY, new_pos.1),
            ];
            app.commit(kiriko_core::Op::Batch { ops });
            app.refresh_preview();
        }
    }

    let stroke = egui::Stroke::new(1.5_f32, theme.accent);
    let r = 6.0;
    ui.painter().circle_stroke(c, r, stroke);
    ui.painter().line_segment(
        [c - egui::vec2(r + 4.0, 0.0), c + egui::vec2(r + 4.0, 0.0)],
        stroke,
    );
    ui.painter().line_segment(
        [c - egui::vec2(0.0, r + 4.0), c + egui::vec2(0.0, r + 4.0)],
        stroke,
    );
}

/// Shape tool: drag a rubber-band in the Viewer to create a rectangle,
/// ellipse or star mask (the current [`ShapeKind`]) on the selected layer.
#[cfg(feature = "media")]
fn shape_overlay(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    draw: egui::Rect,
    scale: f32,
    view: &egui::Response,
) {
    if app.tool != ToolMode::Shape || app.mask_drag.is_some() {
        return;
    }
    if view.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    let Some(comp_id) = app.preview_comp else {
        return;
    };
    let doc = app.store.snapshot();
    let Some(comp) = doc.comp(comp_id) else {
        return;
    };
    let Some(layer) = app
        .selected_layer
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))
    else {
        return;
    };
    if layer.switches.three_d {
        return;
    }
    let fps = comp.frame_rate.fps().max(1.0);
    let lt = app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
    let map = LayerMap::of(layer, lt, draw, scale);

    if view.drag_started() {
        if let Some(pos) = view.interact_pointer_pos() {
            app.shape_drag = Some(map.layer_of(pos));
        }
    }
    if let Some(start) = app.shape_drag {
        let now = view
            .interact_pointer_pos()
            .map(|p| map.layer_of(p))
            .unwrap_or(start);
        // Preview outline in clay.
        let a = map.to_screen(start);
        let b = map.to_screen(now);
        ui.painter().rect_stroke(
            egui::Rect::from_two_pos(a, b),
            2.0,
            egui::Stroke::new(1.0_f32, theme.accent),
            egui::StrokeKind::Inside,
        );
        if view.drag_stopped() {
            app.shape_drag = None;
            let (x0, x1) = (start.0.min(now.0), start.0.max(now.0));
            let (y0, y1) = (start.1.min(now.1), start.1.max(now.1));
            let (w, h) = (x1 - x0, y1 - y0);
            if w > 2.0 && h > 2.0 {
                let mask = match app.shape_kind {
                    ShapeKind::Rectangle => kiriko_core::mask::Mask::rectangle(x0, y0, w, h),
                    ShapeKind::Ellipse => kiriko_core::mask::Mask::ellipse(
                        x0 + w * 0.5,
                        y0 + h * 0.5,
                        w * 0.5,
                        h * 0.5,
                    ),
                    ShapeKind::Star => {
                        let outer = w.min(h) * 0.5;
                        kiriko_core::mask::Mask::star(
                            x0 + w * 0.5,
                            y0 + h * 0.5,
                            outer,
                            outer * 0.42,
                            5,
                        )
                    }
                };
                let mut masks = layer.masks.clone();
                masks.push(mask);
                app.commit(kiriko_core::Op::SetLayerMasks {
                    comp: comp_id,
                    layer: layer.id,
                    masks,
                });
                app.refresh_preview();
            }
        }
    }
}

#[cfg(feature = "media")]
fn pen_overlay(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    draw: egui::Rect,
    scale: f32,
    view: &egui::Response,
) {
    if app.tool != ToolMode::Pen {
        return;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.tool = ToolMode::Select;
        app.pen_path.clear();
        return;
    }
    if view.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    let Some(comp_id) = app.preview_comp else {
        return;
    };
    let doc = app.store.snapshot();
    let Some(comp) = doc.comp(comp_id) else {
        return;
    };
    let Some(layer) = app
        .selected_layer
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))
    else {
        return;
    };
    if layer.switches.three_d {
        return;
    }
    let fps = comp.frame_rate.fps().max(1.0);
    let lt = app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
    let map = LayerMap::of(layer, lt, draw, scale);

    // The in-progress path: open polyline plus handles, first vertex ringed.
    let stroke = egui::Stroke::new(1.0_f32, theme.accent);
    let screen_pts: Vec<egui::Pos2> = app.pen_path.iter().map(|v| map.to_screen(v.pos)).collect();
    if screen_pts.len() >= 2 {
        ui.painter()
            .add(egui::Shape::line(screen_pts.clone(), stroke));
    }
    for (i, pt) in screen_pts.iter().enumerate() {
        let r = egui::Rect::from_center_size(*pt, egui::vec2(8.0, 8.0));
        if i == 0 && screen_pts.len() >= 3 {
            ui.painter().rect_stroke(
                r,
                1.0,
                egui::Stroke::new(1.5_f32, theme.accent),
                egui::StrokeKind::Inside,
            );
        } else {
            ui.painter().rect_filled(r.shrink(2.0), 1.0, theme.accent);
        }
    }

    if view.clicked() {
        if let Some(pos) = view.interact_pointer_pos() {
            let close_hit = screen_pts
                .first()
                .is_some_and(|f| (*f - pos).length() <= 8.0)
                && app.pen_path.len() >= 3;
            if close_hit {
                let mut masks = layer.masks.clone();
                masks.push(kiriko_core::mask::Mask {
                    id: uuid::Uuid::now_v7(),
                    name: format!("Path {}", masks.len() + 1),
                    path: kiriko_core::mask::BezierPath {
                        vertices: std::mem::take(&mut app.pen_path),
                        closed: true,
                    },
                    inverted: false,
                    opacity: 100.0,
                    extra: serde_json::Map::new(),
                });
                app.tool = ToolMode::Select;
                app.commit(kiriko_core::Op::SetLayerMasks {
                    comp: comp_id,
                    layer: layer.id,
                    masks,
                });
                app.refresh_preview();
            } else {
                app.pen_path.push(kiriko_core::mask::Vertex {
                    pos: map.layer_of(pos),
                    tan_in: (0.0, 0.0),
                    tan_out: (0.0, 0.0),
                });
            }
        }
    }
}

#[cfg(feature = "media")]
fn viewer_footage(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    tex: Option<(egui::TextureId, egui::Vec2)>,
    rect: egui::Rect,
) {
    use crate::app_state::media::MediaStatus;
    let frames = if let Some(comp_id) = app.preview_comp {
        app.store
            .snapshot()
            .comp(comp_id)
            .map(|c| app.comp_frame_count(c))
            .unwrap_or(0)
    } else if let Some(id) = app.preview_item {
        match app.media.map.get(&id) {
            Some(MediaStatus::Ready { frames, .. }) => *frames,
            _ => 0,
        }
    } else {
        return;
    };

    let bar_h = 30.0;
    let image_area = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - bar_h));

    if let Some((id, size)) = tex {
        if size.x > 0.0 && size.y > 0.0 {
            // Natural size drives layout: dropping decode resolution keeps the
            // image the same size on screen, just softer (07-UI-SPEC §Viewer).
            let natural = if let Some(comp_id) = app.preview_comp {
                app.store
                    .snapshot()
                    .comp(comp_id)
                    .map(|c| egui::vec2(c.width as f32, c.height as f32))
                    .unwrap_or(size)
            } else if let Some(item) = app.preview_item {
                match app.media.map.get(&item) {
                    Some(MediaStatus::Ready { probe, .. }) => probe
                        .video
                        .as_ref()
                        .map(|v| egui::vec2(v.width as f32, v.height as f32))
                        .unwrap_or(size),
                    _ => size,
                }
            } else {
                size
            };

            // View controls: scroll zooms, drag pans (the hand — object tools
            // arrive later), double-click resets to fit. View-only: never part
            // of any render.
            let view = ui.interact(
                image_area,
                ui.id().with("viewer-view"),
                egui::Sense::click_and_drag(),
            );
            if view.hovered() {
                let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
                if scroll.abs() > 0.1 {
                    let factor = (scroll * 0.003).exp();
                    app.view_zoom = (app.view_zoom * factor).clamp(0.05, 32.0);
                    if app.preview_auto_res {
                        app.refresh_preview();
                    }
                }
            }
            // Drag pans in Select/Hand; Shape and Pen intercept it below.
            if view.dragged() && matches!(app.tool, ToolMode::Select | ToolMode::Hand) {
                app.view_pan += view.drag_delta();
            }
            if matches!(app.tool, ToolMode::Hand) && view.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if view.double_clicked() {
                app.view_zoom = 1.0;
                app.view_pan = egui::Vec2::ZERO;
                if app.preview_auto_res {
                    app.refresh_preview();
                }
            }

            let fit = (image_area.width() / natural.x)
                .min(image_area.height() / natural.y)
                .min(1.0);
            let scale = fit * app.view_zoom;
            app.last_display_scale = scale;
            let draw =
                egui::Rect::from_center_size(image_area.center() + app.view_pan, natural * scale);
            ui.painter().image(
                id,
                draw,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            #[cfg(feature = "media")]
            mask_overlay(ui, theme, app, draw, scale);
            #[cfg(feature = "media")]
            pen_overlay(ui, theme, app, draw, scale, &view);
            #[cfg(feature = "media")]
            shape_overlay(ui, theme, app, draw, scale, &view);
            #[cfg(feature = "media")]
            anchor_overlay(ui, theme, app, draw, scale);
        }
    } else {
        ui.painter().text(
            image_area.center(),
            egui::Align2::CENTER_CENTER,
            if frames == 0 {
                "probing…"
            } else {
                "decoding…"
            },
            egui::FontId::monospace(10.0),
            theme.text_disabled,
        );
    }

    // Viewer bar: resolution picker · scrub · frame readout (07-UI-SPEC §2).
    let bar = egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.max.y - bar_h), rect.max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
        egui::Frame::new()
            .fill(theme.surface_1)
            .stroke(egui::Stroke::new(1.0_f32, theme.hairline))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let labels = ["Full", "Half", "Third", "Quarter"];
                    let current = if app.preview_auto_res {
                        "Auto"
                    } else {
                        labels[(app.preview_divisor as usize - 1).min(3)]
                    };
                    bare_dropdown(ui, current, |ui| {
                        if ui
                            .selectable_label(app.preview_auto_res, "Auto")
                            .on_hover_text("Decode at the displayed size, capped at 100%")
                            .clicked()
                        {
                            app.preview_auto_res = true;
                            app.refresh_preview();
                            ui.close_menu();
                        }
                        for (i, label) in labels.iter().enumerate() {
                            let div = i as u32 + 1;
                            let selected = !app.preview_auto_res && app.preview_divisor == div;
                            if ui.selectable_label(selected, *label).clicked() {
                                app.preview_auto_res = false;
                                app.preview_divisor = div;
                                app.refresh_preview();
                                ui.close_menu();
                            }
                        }
                    });
                    ui.label(
                        egui::RichText::new(format!("{:.0}%", app.last_display_scale * 100.0))
                            .monospace()
                            .small()
                            .color(theme.text_muted),
                    );
                    if frames > 1 {
                        let mut frame = app.preview_frame;
                        let slider = egui::Slider::new(&mut frame, 0..=frames - 1)
                            .show_value(false)
                            .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.4 });
                        let response =
                            ui.add_sized(egui::vec2(ui.available_width() - 96.0, 18.0), slider);
                        if response.changed() {
                            app.preview_frame = frame;
                            // Dragging the scrub slider decodes a draft; releasing
                            // it reloads at the specified resolution.
                            app.preview_draft = response.dragged();
                            app.refresh_preview();
                        }
                        if response.drag_stopped() {
                            app.preview_draft = false;
                            app.refresh_preview();
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} / {}",
                                app.preview_frame,
                                frames.saturating_sub(1)
                            ))
                            .monospace()
                            .small()
                            .color(theme.text_muted),
                        );
                    });
                });
            });
    });
}

/// The graph editor (07-UI-SPEC; value view v1 — the speed view joins with
/// Retime). Draws the selected layer's animated property as a live curve;
/// keys drag (one op per release), double-click adds, right-click removes.
/// Timeline graph mode (K-070): keep the left column (the layer / property
/// list — Mack) and show the selected property's curve in the track area.
/// Clicking a property in the left column graphs it.
fn graph_editor_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    use kiriko_core::model::TransformProp;

    const PROPS: [(TransformProp, &str); 8] = [
        (TransformProp::PositionX, "Position x"),
        (TransformProp::PositionY, "Position y"),
        (TransformProp::ScaleX, "Scale x"),
        (TransformProp::ScaleY, "Scale y"),
        (TransformProp::Rotation, "Rotation"),
        (TransformProp::Opacity, "Opacity"),
        (TransformProp::AnchorX, "Anchor x"),
        (TransformProp::AnchorY, "Anchor y"),
    ];

    let doc = app.store.snapshot();
    let comp = app
        .preview_comp
        .or(app.selected_comp)
        .and_then(|id| doc.comp(id));
    let Some(comp) = comp else {
        empty_hint(
            ui,
            theme,
            "Graph editor",
            "Open a composition to edit curves.",
        );
        return;
    };

    let area = ui.available_rect_before_wrap();
    let name_w = app
        .timeline_name_w
        .clamp(96.0, (area.width() - 120.0).max(96.0));
    let left_rect =
        egui::Rect::from_min_max(area.min, egui::pos2(area.left() + name_w, area.bottom()));
    let plot_rect =
        egui::Rect::from_min_max(egui::pos2(area.left() + name_w + 6.0, area.top()), area.max);
    ui.painter().line_segment(
        [
            egui::pos2(area.left() + name_w + 2.0, area.top()),
            egui::pos2(area.left() + name_w + 2.0, area.bottom()),
        ],
        egui::Stroke::new(1.0_f32, theme.hairline),
    );

    // Left column: every layer, and the selected layer's animated properties.
    // Deferred so the immutable borrow of the document (comp) and the mutable
    // selection state don't overlap.
    let mut picked_layer: Option<uuid::Uuid> = None;
    let mut picked_prop: Option<TransformProp> = None;
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    left.set_clip_rect(left_rect);
    left.spacing_mut().item_spacing.y = 2.0;
    for layer in &comp.layers {
        let selected = app.selected_layer == Some(layer.id);
        if left
            .selectable_label(selected, trim_title(&layer.name))
            .clicked()
        {
            picked_layer = Some(layer.id);
        }
        if selected {
            let mut any = false;
            for (p, name) in PROPS.iter().copied() {
                if !layer.transform.get(p).is_animated() {
                    continue;
                }
                any = true;
                let is_cur = app.graph_prop == Some(p);
                left.horizontal(|ui| {
                    ui.add_space(16.0);
                    if ui.selectable_label(is_cur, name).clicked() {
                        picked_prop = Some(p);
                    }
                });
            }
            if !any {
                left.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new("no animated properties")
                            .small()
                            .italics()
                            .color(theme.text_muted),
                    );
                });
            }
        }
    }
    if let Some(l) = picked_layer {
        app.selected_layer = Some(l);
    }
    if let Some(p) = picked_prop {
        app.graph_prop = Some(p);
    }

    // Right area: the curve for the selected layer / property.
    let Some(layer_id) = app.selected_layer else {
        hint_in_rect(ui, theme, plot_rect, "Select a layer to edit its curves.");
        return;
    };
    let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
        hint_in_rect(ui, theme, plot_rect, "The selected layer is gone.");
        return;
    };
    let animated: Vec<TransformProp> = PROPS
        .iter()
        .map(|(p, _)| *p)
        .filter(|p| layer.transform.get(*p).is_animated())
        .collect();
    let Some(&first) = animated.first() else {
        hint_in_rect(
            ui,
            theme,
            plot_rect,
            "Click a stopwatch in the layer view to animate a property.",
        );
        return;
    };
    let current = app
        .graph_prop
        .filter(|p| animated.contains(p))
        .unwrap_or(first);
    app.graph_prop = Some(current);
    graph_plot(ui, theme, app, comp, layer, current, plot_rect);
}

/// Centre a muted hint inside `rect` (empty state of a sub-pane).
fn hint_in_rect(ui: &egui::Ui, theme: &Theme, rect: egui::Rect, msg: &str) {
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        msg,
        egui::FontId::proportional(12.0),
        theme.text_muted,
    );
}

/// Draw one keyframed property's value/speed curve inside `rect`, with a
/// compact Value/Speed + Ease/Linear header and draggable keys.
fn graph_plot(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    comp: &kiriko_core::model::Composition,
    layer: &kiriko_core::model::Layer,
    current: kiriko_core::model::TransformProp,
    rect: egui::Rect,
) {
    use kiriko_core::anim::{Animation, Keyframe, SideInterp};
    let layer_id = layer.id;

    // Compact header: value/speed lens and blanket ease/linear.
    let header = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + 22.0));
    let mut set_sides: Option<SideInterp> = None;
    {
        let mut h = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        h.set_clip_rect(header);
        if h.selectable_label(!app.graph_speed_view, "Value").clicked() {
            app.graph_speed_view = false;
        }
        if h.selectable_label(app.graph_speed_view, "Speed")
            .on_hover_text("The derivative view — editing here arrives with Retime")
            .clicked()
        {
            app.graph_speed_view = true;
        }
        h.separator();
        if h.small_button("Ease")
            .on_hover_text("Easy-ease every key of this curve (AE's F9)")
            .clicked()
        {
            set_sides = Some(kiriko_core::anim::EASY_EASE);
        }
        if h.small_button("Linear")
            .on_hover_text("Straighten every key of this curve")
            .clicked()
        {
            set_sides = Some(SideInterp::Linear);
        }
    }
    let rect = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 22.0), rect.max);

    let slot = layer.transform.get(current);
    let Animation::Keyframed(keys) = &slot.animation else {
        return;
    };

    // ---- plot geometry: x = layer time over the comp span, y = value ----
    ui.painter().rect_filled(rect, 0.0, theme.surface_0);
    let duration = comp.duration.0.to_f64().max(1e-6);
    let (mut vmin, mut vmax) = keys.iter().fold((f64::MAX, f64::MIN), |(lo, hi), k| {
        (lo.min(k.value), hi.max(k.value))
    });
    if let Some((_, _, v)) = app.graph_edit {
        vmin = vmin.min(v);
        vmax = vmax.max(v);
    }
    let pad = ((vmax - vmin).abs().max(1.0)) * 0.15;
    let (vmin, vmax) = (vmin - pad, vmax + pad);
    let x_of = |t: f64| rect.left() + ((t / duration) as f32) * rect.width();
    let y_of = |v: f64| rect.bottom() - (((v - vmin) / (vmax - vmin)) as f32) * rect.height();
    let t_of = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * duration;
    let v_of = |y: f32| {
        vmin + ((rect.bottom() - y) / rect.height()).clamp(0.0, 1.0) as f64 * (vmax - vmin)
    };

    // Provisional keys during a drag (visual only until release).
    let mut shown: Vec<Keyframe> = keys.clone();
    if let Some((idx, kt, kv)) = app.graph_edit {
        if let Some(k) = shown.get_mut(idx) {
            k.time = rational_at(kt);
            k.value = kv;
        }
        shown.sort_by_key(|k| k.time);
    }

    // Curve polyline: value, or its derivative in the speed lens (central
    // difference at half-frame steps — display-first; exact closed forms
    // arrive with Retime's segment maths).
    let samples = (rect.width() as usize / 2).max(16);
    let fps_est = comp.frame_rate.fps().max(1.0);
    let sample_at = |t: f64| -> f64 {
        if app.graph_speed_view {
            let h = 0.5 / fps_est;
            let a = kiriko_core::anim::evaluate(&shown, t - h).unwrap_or(0.0);
            let b = kiriko_core::anim::evaluate(&shown, t + h).unwrap_or(0.0);
            (b - a) / (2.0 * h)
        } else {
            kiriko_core::anim::evaluate(&shown, t).unwrap_or(0.0)
        }
    };
    let values: Vec<(f64, f64)> = (0..=samples)
        .map(|i| {
            let t = duration * i as f64 / samples as f64;
            (t, sample_at(t))
        })
        .collect();
    // The speed lens scales to its own sampled range.
    let speed_y: Box<dyn Fn(f64) -> f32> = if app.graph_speed_view {
        let (mut lo, mut hi) = values.iter().fold((f64::MAX, f64::MIN), |(l, h), (_, v)| {
            (l.min(*v), h.max(*v))
        });
        let pad = ((hi - lo).abs().max(1.0)) * 0.15;
        lo -= pad;
        hi += pad;
        Box::new(move |v: f64| rect.bottom() - (((v - lo) / (hi - lo)) as f32) * rect.height())
    } else {
        Box::new(y_of)
    };
    let points: Vec<egui::Pos2> = values
        .iter()
        .map(|(t, v)| egui::pos2(x_of(*t), speed_y(*v)))
        .collect();
    let stroke_colour = if app.graph_speed_view {
        theme.curve[1]
    } else {
        theme.curve[0]
    };
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5_f32, stroke_colour),
    ));

    // Playhead (layer time).
    if app.preview_comp == Some(comp.id) {
        let lt = app.preview_frame as f64 / comp.frame_rate.fps().max(1.0)
            - layer.start_offset.0.to_f64();
        let x = x_of(lt);
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0_f32, theme.accent),
        );
    }

    // Keys: draggable squares (value lens); read-only ticks in the speed lens.
    let mut pending: Option<Vec<Keyframe>> = None;
    if app.graph_speed_view {
        for key in keys {
            let x = x_of(key.time.to_f64());
            ui.painter().line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(0.5_f32, theme.hairline_strong),
            );
        }
    }
    for (idx, key) in keys.iter().enumerate() {
        if app.graph_speed_view {
            break; // speed-lens editing arrives with Retime's segment model
        }
        let (kt, kv) = match app.graph_edit {
            Some((i, t, v)) if i == idx => (t, v),
            _ => (key.time.to_f64(), key.value),
        };
        let pos = egui::pos2(x_of(kt), y_of(kv));
        let hit = egui::Rect::from_center_size(pos, egui::vec2(12.0, 12.0));
        let resp = ui.interact(
            hit,
            ui.id().with(("gkey", layer_id, idx)),
            egui::Sense::click_and_drag(),
        );
        let colour = if resp.hovered() || app.graph_edit.is_some_and(|(i, ..)| i == idx) {
            theme.accent
        } else {
            theme.text_secondary
        };
        ui.painter().rect_filled(
            egui::Rect::from_center_size(pos, egui::vec2(7.0, 7.0)),
            1.0,
            colour,
        );
        if resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                app.graph_edit = Some((idx, t_of(p.x), v_of(p.y)));
            }
        }
        if resp.drag_stopped() {
            if let Some((i, kt, kv)) = app.graph_edit.take() {
                if i == idx {
                    let mut new_keys = keys.clone();
                    new_keys[i].time = rational_at(kt.max(0.0));
                    new_keys[i].value = kv;
                    new_keys.sort_by_key(|k| k.time);
                    new_keys.dedup_by(|a, b| a.time == b.time);
                    pending = Some(new_keys);
                }
            }
        }
        if resp.secondary_clicked() {
            let mut new_keys = keys.clone();
            new_keys.remove(idx);
            pending = Some(new_keys);
        }
    }
    let bg = ui.interact(
        rect,
        ui.id().with(("graph-bg", layer_id)),
        egui::Sense::click(),
    );
    if bg.double_clicked() {
        if let Some(p) = bg.interact_pointer_pos() {
            let mut new_keys = keys.clone();
            new_keys.push(Keyframe {
                time: rational_at(t_of(p.x)),
                value: v_of(p.y),
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            });
            new_keys.sort_by_key(|k| k.time);
            pending = Some(new_keys);
        }
    }

    if let Some(sides) = set_sides {
        let mut new_keys = keys.clone();
        for k in &mut new_keys {
            k.interp_in = sides;
            k.interp_out = sides;
        }
        pending = Some(new_keys);
    }
    if let Some(new_keys) = pending {
        let animation = if new_keys.is_empty() {
            Animation::Static(slot.value_at(0.0))
        } else {
            Animation::Keyframed(new_keys)
        };
        app.commit(kiriko_core::Op::SetTransformProperty {
            comp: comp.id,
            layer: layer_id,
            prop: current,
            animation,
        });
    }
}

/// A copy of `comp` with one layer's transform property overridden to a fixed
/// `value` — the live value-drag preview renders this so the provisional value
/// shows before the edit is committed. Only the previewed frame is rendered, so
/// pinning the property to a constant is exactly its value at that instant.
#[cfg(feature = "media")]
fn patch_layer_prop(
    comp: &kiriko_core::model::Composition,
    layer: uuid::Uuid,
    prop: kiriko_core::model::TransformProp,
    value: f64,
) -> kiriko_core::model::Composition {
    let mut patched = comp.clone();
    if let Some(l) = patched.layers.iter_mut().find(|l| l.id == layer) {
        *l.transform.get_mut(prop) = kiriko_core::anim::Property::fixed(value);
    }
    patched
}

/// Build a comp's draw list recursively (preview side of Precomp layers).
/// Bottom-up order; matte sources come from decoded pixels (precomp mattes
/// await the GPU mask pass, mirroring export).
#[cfg(feature = "media")]
fn build_comp_draws(
    doc: &kiriko_core::model::Document,
    comp: &kiriko_core::model::Composition,
    t_comp: f64,
    pixels_by_layer: &std::collections::HashMap<
        uuid::Uuid,
        &crate::app_state::preview::CompLayerPixels,
    >,
    visited: &mut Vec<uuid::Uuid>,
) -> Vec<CompLayerDraw> {
    use kiriko_core::model::LayerKind;
    let in_span = |l: &kiriko_core::model::Layer| {
        t_comp >= l.in_point.0.to_f64() && t_comp < l.out_point.0.to_f64()
    };
    let pixels_for = |layer: &kiriko_core::model::Layer| -> Option<LayerPixels> {
        let raw = match &layer.kind {
            // Footage and Sequence footage clips both arrive decoded, keyed by
            // the layer id (collect_comp_jobs pushes one job per layer/frame).
            LayerKind::Footage { .. } | LayerKind::Sequence { .. } => {
                pixels_by_layer.get(&layer.id).map(|lp| {
                    // Geometry uses the native source size, never the decoded
                    // size: under auto res the decode shrinks and grows with
                    // viewport zoom, and sizing the layer by that made it
                    // scale with zoom (a small layer ballooned when zoomed in).
                    (
                        lp.rgba.clone(),
                        lp.width,
                        lp.height,
                        (lp.natural_w as f32, lp.natural_h as f32),
                    )
                })
            }
            LayerKind::Solid { def } => doc.solid(*def).filter(|_| in_span(layer)).map(|sd| {
                let px = crate::export::solid_rgba(sd.colour);
                let (tw, th) = if layer.masks.is_empty() {
                    (8, 8)
                } else {
                    (sd.width, sd.height)
                };
                (
                    crate::export::px_tile(&px, tw, th),
                    tw,
                    th,
                    (sd.width as f32, sd.height as f32),
                )
            }),
            LayerKind::Text { document } => in_span(layer).then(|| {
                let fill = crate::export::solid_rgba(document.fill);
                let r = kiriko_text::rasterise_line(
                    &document.text,
                    document.size as f32,
                    [fill[0], fill[1], fill[2]],
                );
                (r.rgba, r.width, r.height, (r.width as f32, r.height as f32))
            }),
            LayerKind::Precomp { .. } => None, // handled as Nested below
            LayerKind::Camera { .. } => None,  // shapes the view, draws nothing
        };
        raw.map(|(mut rgba, w, h, natural)| {
            kiriko_core::mask::apply_masks(
                &mut rgba,
                w,
                h,
                f64::from(natural.0),
                f64::from(natural.1),
                &layer.masks,
            );
            (rgba, w, h, natural)
        })
    };

    let mut draws: Vec<CompLayerDraw> = Vec::new();
    for layer in comp.layers.iter().rev() {
        if !layer.switches.visible || !in_span(layer) {
            continue;
        }
        let lt = t_comp - layer.start_offset.0.to_f64();
        let tr = &layer.transform;

        let (source, natural) = match &layer.kind {
            LayerKind::Precomp { comp: nested_id } => {
                if visited.contains(nested_id) {
                    continue; // cycle guard
                }
                let Some(nested) = doc.comp(*nested_id) else {
                    continue;
                };
                visited.push(*nested_id);
                let nested_draws = build_comp_draws(doc, nested, lt, pixels_by_layer, visited);
                visited.pop();
                let nbg = nested.background.0;
                (
                    DrawSource::Nested {
                        width: nested.width,
                        height: nested.height,
                        background: [
                            f64::from(nbg[0]),
                            f64::from(nbg[1]),
                            f64::from(nbg[2]),
                            f64::from(nbg[3]),
                        ],
                        draws: nested_draws,
                        camera: nested.camera_pose(lt),
                    },
                    (nested.width as f32, nested.height as f32),
                )
            }
            _ => {
                let Some((rgba, w, h, natural)) = pixels_for(layer) else {
                    continue;
                };
                (
                    DrawSource::Pixels {
                        rgba,
                        tex_w: w,
                        tex_h: h,
                    },
                    natural,
                )
            }
        };

        let matte = layer.matte.as_ref().and_then(|mr| {
            let src = comp.layers.iter().find(|l| l.id == mr.layer)?;
            let (m_rgba, m_w, m_h, m_nat) = pixels_for(src)?;
            let mlt = t_comp - src.start_offset.0.to_f64();
            let mtr = &src.transform;
            Some(MatteDraw {
                rgba: m_rgba,
                tex_w: m_w,
                tex_h: m_h,
                natural_size: m_nat,
                position: (
                    mtr.position_x.value_at(mlt) as f32,
                    mtr.position_y.value_at(mlt) as f32,
                ),
                anchor: (
                    mtr.anchor_x.value_at(mlt) as f32,
                    mtr.anchor_y.value_at(mlt) as f32,
                ),
                scale: (
                    mtr.scale_x.value_at(mlt) as f32,
                    mtr.scale_y.value_at(mlt) as f32,
                ),
                rotation_deg: mtr.rotation.value_at(mlt) as f32,
                opacity: mtr.opacity.value_at(mlt) as f32,
                z: mtr.position_z.value_at(mlt) as f32,
                rotation_x_deg: mtr.rotation_x.value_at(mlt) as f32,
                rotation_y_deg: mtr.rotation_y.value_at(mlt) as f32,
                three_d: src.switches.three_d,
                luma: matches!(mr.channel, kiriko_core::model::MatteChannel::Luma),
                inverted: mr.inverted,
            })
        });

        draws.push(CompLayerDraw {
            source,
            natural_size: natural,
            position: (
                tr.position_x.value_at(lt) as f32,
                tr.position_y.value_at(lt) as f32,
            ),
            anchor: (
                tr.anchor_x.value_at(lt) as f32,
                tr.anchor_y.value_at(lt) as f32,
            ),
            scale: (
                tr.scale_x.value_at(lt) as f32,
                tr.scale_y.value_at(lt) as f32,
            ),
            rotation_deg: tr.rotation.value_at(lt) as f32,
            opacity: tr.opacity.value_at(lt) as f32,
            z: tr.position_z.value_at(lt) as f32,
            rotation_x_deg: tr.rotation_x.value_at(lt) as f32,
            rotation_y_deg: tr.rotation_y.value_at(lt) as f32,
            three_d: layer.switches.three_d,
            matte,
            blend: blend_of(layer.blend),
            mask_cov: match &layer.kind {
                LayerKind::Precomp { .. } if !layer.masks.is_empty() => {
                    let (w, h) = (natural.0 as u32, natural.1 as u32);
                    Some((
                        crate::export::mask_rgba(&kiriko_core::mask::combined_coverage(
                            &layer.masks,
                            w,
                            h,
                            f64::from(w),
                            f64::from(h),
                        )),
                        w,
                        h,
                    ))
                }
                _ => None,
            },
        });
    }
    draws
}

/// The layer's natural pixel space (mask coordinates live here).
/// Compact matte / blend / 3D / mute controls for a layer's title line
/// (left column). Sets `pending` on any change.
/// Trim a layer title for display: people type what they like, but past a
/// cap the shown value ends with "…" (Mack).
fn trim_title(name: &str) -> String {
    const MAX: usize = 48;
    if name.chars().count() <= MAX {
        name.to_owned()
    } else {
        let mut s: String = name.chars().take(MAX - 1).collect();
        s.push('…');
        s
    }
}

/// Visibility (eye) toggle — its own left-column subcolumn.
fn visible_control(
    ui: &mut egui::Ui,
    theme: &Theme,
    comp_id: uuid::Uuid,
    layer: &kiriko_core::model::Layer,
    pending: &mut Option<kiriko_core::Op>,
) {
    let vis = layer.switches.visible;
    let glyph = if vis { "◉" } else { "○" };
    let col = if vis {
        theme.text_secondary
    } else {
        theme.text_disabled
    };
    if ui
        .add(egui::Label::new(egui::RichText::new(glyph).color(col)).sense(egui::Sense::click()))
        .on_hover_text("Show / hide this layer")
        .clicked()
    {
        *pending = Some(kiriko_core::Op::SetLayerVisible {
            comp: comp_id,
            layer: layer.id,
            visible: !vis,
        });
    }
}

/// Matte subcolumn: source pick + luma/invert flags under one dropdown.
fn matte_control(
    ui: &mut egui::Ui,
    comp: &kiriko_core::model::Composition,
    comp_id: uuid::Uuid,
    layer: &kiriko_core::model::Layer,
    pending: &mut Option<kiriko_core::Op>,
) {
    use kiriko_core::model::{MatteChannel, MatteRef};
    let label = layer
        .matte
        .as_ref()
        .and_then(|m| comp.layers.iter().find(|l| l.id == m.layer))
        .map(|l| format!("⬓ {}", l.name))
        .unwrap_or_else(|| "⬓".into());
    let mut set: Option<Option<MatteRef>> = None;
    bare_dropdown(ui, egui::RichText::new(label).small(), |ui| {
        if ui.selectable_label(layer.matte.is_none(), "None").clicked() {
            set = Some(None);
            ui.close_menu();
        }
        for other in comp.layers.iter().filter(|l| l.id != layer.id) {
            let selected = layer.matte.is_some_and(|m| m.layer == other.id);
            if ui.selectable_label(selected, &other.name).clicked() {
                set = Some(Some(MatteRef {
                    layer: other.id,
                    channel: layer
                        .matte
                        .map(|m| m.channel)
                        .unwrap_or(MatteChannel::Alpha),
                    inverted: layer.matte.is_some_and(|m| m.inverted),
                }));
                ui.close_menu();
            }
        }
        if let Some(mut m) = layer.matte {
            ui.separator();
            let luma = matches!(m.channel, MatteChannel::Luma);
            if ui.selectable_label(luma, "Luma matte").clicked() {
                m.channel = if luma {
                    MatteChannel::Alpha
                } else {
                    MatteChannel::Luma
                };
                set = Some(Some(m));
            }
            if ui.selectable_label(m.inverted, "Inverted").clicked() {
                m.inverted = !m.inverted;
                set = Some(Some(m));
            }
        }
    });
    if let Some(matte) = set {
        *pending = Some(kiriko_core::Op::SetLayerMatte {
            comp: comp_id,
            layer: layer.id,
            matte,
        });
    }
}

fn blend_name(b: kiriko_core::model::BlendMode) -> &'static str {
    use kiriko_core::model::BlendMode;
    match b {
        BlendMode::Normal => "Normal",
        BlendMode::Add => "Add",
        BlendMode::Multiply => "Multiply",
        BlendMode::Screen => "Screen",
        BlendMode::Overlay => "Overlay",
        BlendMode::SoftLight => "Soft light",
        BlendMode::HardLight => "Hard light",
        BlendMode::Lighten => "Lighten",
        BlendMode::Darken => "Darken",
    }
}

/// Blend-mode subcolumn.
fn blend_control(
    ui: &mut egui::Ui,
    comp_id: uuid::Uuid,
    layer: &kiriko_core::model::Layer,
    pending: &mut Option<kiriko_core::Op>,
) {
    use kiriko_core::model::BlendMode;
    bare_dropdown(
        ui,
        egui::RichText::new(blend_name(layer.blend)).small(),
        |ui| {
            for mode in [
                BlendMode::Normal,
                BlendMode::Add,
                BlendMode::Multiply,
                BlendMode::Screen,
                BlendMode::Overlay,
                BlendMode::SoftLight,
                BlendMode::HardLight,
                BlendMode::Lighten,
                BlendMode::Darken,
            ] {
                if ui
                    .selectable_label(layer.blend == mode, blend_name(mode))
                    .clicked()
                {
                    if layer.blend != mode {
                        *pending = Some(kiriko_core::Op::SetLayerBlend {
                            comp: comp_id,
                            layer: layer.id,
                            blend: mode,
                        });
                    }
                    ui.close_menu();
                }
            }
        },
    );
}

/// 3D-switch subcolumn.
fn three_d_control(
    ui: &mut egui::Ui,
    comp_id: uuid::Uuid,
    layer: &kiriko_core::model::Layer,
    pending: &mut Option<kiriko_core::Op>,
) {
    if ui
        .selectable_label(layer.switches.three_d, egui::RichText::new("3D").small())
        .on_hover_text("Place this layer in z-space (needs a Camera layer)")
        .clicked()
    {
        *pending = Some(kiriko_core::Op::SetLayerThreeD {
            comp: comp_id,
            layer: layer.id,
            three_d: !layer.switches.three_d,
        });
    }
}

/// Mute subcolumn (footage layers).
fn mute_control(
    ui: &mut egui::Ui,
    comp_id: uuid::Uuid,
    layer: &kiriko_core::model::Layer,
    pending: &mut Option<kiriko_core::Op>,
) {
    let muted = !layer.switches.audible;
    if ui
        .selectable_label(muted, egui::RichText::new("Mute").small())
        .on_hover_text("Silence this layer in playback and export")
        .clicked()
    {
        *pending = Some(kiriko_core::Op::SetLayerAudible {
            comp: comp_id,
            layer: layer.id,
            audible: muted,
        });
    }
}

/// Every keyframe time (layer-local seconds) across a layer's animated
/// properties — for the timeline's keyframe glyphs.
fn layer_keyframe_times(layer: &kiriko_core::model::Layer) -> Vec<f64> {
    use kiriko_core::anim::Animation;
    use kiriko_core::model::{LayerKind, TransformProp};
    let mut times = Vec::new();
    let mut collect = |anim: &Animation| {
        if let Animation::Keyframed(keys) = anim {
            times.extend(keys.iter().map(|k| k.time.to_f64()));
        }
    };
    for prop in [
        TransformProp::AnchorX,
        TransformProp::AnchorY,
        TransformProp::PositionX,
        TransformProp::PositionY,
        TransformProp::PositionZ,
        TransformProp::ScaleX,
        TransformProp::ScaleY,
        TransformProp::Rotation,
        TransformProp::RotationX,
        TransformProp::RotationY,
        TransformProp::Opacity,
    ] {
        collect(&layer.transform.get(prop).animation);
    }
    if let LayerKind::Camera { zoom } = &layer.kind {
        collect(&zoom.animation);
    }
    times
}

/// A single "Speed %" row for a footage layer, inside Transform. Editing sets
/// a constant-speed retime (100% clears it); ramps are shown read-only and
/// edited in the graph editor (K-070).
fn speed_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    comp_id: uuid::Uuid,
    layer: &kiriko_core::model::Layer,
    retime: &Option<kiriko_core::retime::Retime>,
    pending: &mut Option<kiriko_core::Op>,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Speed %")
                .small()
                .color(theme.text_muted),
        );
        let current = retime
            .as_ref()
            .map(|r| r.speed_at(0.0) * 100.0)
            .unwrap_or(100.0);
        let id = egui::Id::new(("speed", layer.id));
        let mut v = ui.data(|d| d.get_temp::<f64>(id)).unwrap_or(current);
        let resp = ui.add(
            egui::DragValue::new(&mut v)
                .speed(1.0)
                .range(-800.0..=800.0)
                .suffix(" %"),
        );
        if resp.dragged() || resp.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, v));
        }
        if resp.drag_stopped() || resp.lost_focus() {
            if (v - current).abs() > 1e-6 {
                let new_retime = if (v - 100.0).abs() < 1e-6 {
                    None
                } else {
                    let d = layer.out_point.0;
                    let speed = kiriko_core::Rational::from_f64_on_grid(v / 100.0, 1000)
                        .unwrap_or(kiriko_core::Rational::ONE);
                    Some(kiriko_core::retime::Retime::constant_speed(
                        d,
                        kiriko_core::Rational::ZERO,
                        speed,
                    ))
                };
                *pending = Some(kiriko_core::Op::SetLayerRetime {
                    comp: comp_id,
                    layer: layer.id,
                    retime: new_retime,
                });
            }
            ui.data_mut(|d| d.remove::<f64>(id));
        }
        // If the map is an actual ramp (start ≠ end), flag it read-only.
        if retime
            .as_ref()
            .and_then(|r| r.single_ramp_view())
            .is_some_and(|(a, b, _)| (a - b).abs() > 1e-9)
        {
            ui.label(
                egui::RichText::new("ramp")
                    .small()
                    .color(theme.text_disabled),
            )
            .on_hover_text("Ramps are edited in the graph editor");
        }
    });
}

fn mask_space(
    layer: &kiriko_core::model::Layer,
    app: &AppState,
    comp: &kiriko_core::model::Composition,
) -> (f64, f64) {
    match &layer.kind {
        kiriko_core::model::LayerKind::Solid { def } => app
            .store
            .snapshot()
            .solid(*def)
            .map(|sd| (f64::from(sd.width), f64::from(sd.height)))
            .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
        kiriko_core::model::LayerKind::Precomp { comp: nested } => app
            .store
            .snapshot()
            .comp(*nested)
            .map(|n| (f64::from(n.width), f64::from(n.height)))
            .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
        kiriko_core::model::LayerKind::Camera { .. }
        | kiriko_core::model::LayerKind::Sequence { .. }
        | kiriko_core::model::LayerKind::Text { .. } => {
            (f64::from(comp.width), f64::from(comp.height))
        }
        #[cfg(feature = "media")]
        kiriko_core::model::LayerKind::Footage { item, .. } => match app.media.map.get(item) {
            Some(crate::app_state::media::MediaStatus::Ready { probe, .. }) => probe
                .video
                .as_ref()
                .map(|v| (f64::from(v.width), f64::from(v.height)))
                .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
            _ => (f64::from(comp.width), f64::from(comp.height)),
        },
        #[cfg(not(feature = "media"))]
        kiriko_core::model::LayerKind::Footage { .. } => {
            (f64::from(comp.width), f64::from(comp.height))
        }
    }
}

#[cfg(feature = "media")]
fn blend_of(b: kiriko_core::model::BlendMode) -> kiriko_gpu::Blend {
    use kiriko_core::model::BlendMode;
    match b {
        BlendMode::Normal => kiriko_gpu::Blend::Normal,
        BlendMode::Add => kiriko_gpu::Blend::Add,
        BlendMode::Multiply => kiriko_gpu::Blend::Multiply,
        BlendMode::Screen => kiriko_gpu::Blend::Screen,
        BlendMode::Overlay => kiriko_gpu::Blend::Overlay,
        BlendMode::SoftLight => kiriko_gpu::Blend::SoftLight,
        BlendMode::HardLight => kiriko_gpu::Blend::HardLight,
        BlendMode::Lighten => kiriko_gpu::Blend::Lighten,
        BlendMode::Darken => kiriko_gpu::Blend::Darken,
    }
}

/// Layer time → rational on the flick grid (the only f64→rational route).
fn rational_at(seconds: f64) -> kiriko_core::Rational {
    kiriko_core::Rational::from_f64_on_grid(seconds.max(0.0), kiriko_core::Rational::FLICK_DEN)
        .unwrap_or(kiriko_core::Rational::ZERO)
}

/// Insert or replace a keyframe at layer time `lt` with `value`, keeping the
/// list sorted and times unique (half-frame tolerance for "same time").
fn upsert_key(
    slot: &kiriko_core::anim::Property,
    lt: f64,
    value: f64,
) -> Vec<kiriko_core::anim::Keyframe> {
    use kiriko_core::anim::{Animation, Keyframe, SideInterp};
    let mut keys = match &slot.animation {
        Animation::Keyframed(k) => k.clone(),
        Animation::Static(v) => vec![Keyframe {
            time: rational_at(0.0),
            value: *v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        }],
    };
    const EPS: f64 = 1.0 / 240.0;
    if let Some(existing) = keys.iter_mut().find(|k| (k.time.to_f64() - lt).abs() < EPS) {
        existing.value = value;
    } else {
        keys.push(Keyframe {
            time: rational_at(lt),
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        });
        keys.sort_by_key(|k| k.time);
    }
    keys
}

fn effects_panel(ui: &mut egui::Ui, theme: &Theme) {
    ui.add_space(6.0);
    let mut search = String::new();
    ui.add(
        egui::TextEdit::singleline(&mut search)
            .hint_text("Search effects and presets")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("The effect suite arrives in phase 3.")
            .small()
            .color(theme.text_muted),
    );
}

fn empty_hint(ui: &mut egui::Ui, theme: &Theme, title: &str, hint: &str) {
    ui.add_space(10.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(title).color(theme.text_secondary));
        ui.add_space(2.0);
        ui.label(egui::RichText::new(hint).small().color(theme.text_muted));
    });
}

/// Pixels + texture dims + natural size for any layer kind (preview path).
#[cfg(feature = "media")]
type LayerPixels = (Vec<u8>, u32, u32, (f32, f32));

/// One decoded layer ready to composite (evaluator v0).
#[cfg(feature = "media")]
pub struct MatteDraw {
    pub rgba: Vec<u8>,
    pub tex_w: u32,
    pub tex_h: u32,
    pub natural_size: (f32, f32),
    pub position: (f32, f32),
    pub anchor: (f32, f32),
    pub scale: (f32, f32),
    pub rotation_deg: f32,
    pub opacity: f32,
    pub z: f32,
    pub rotation_x_deg: f32,
    pub rotation_y_deg: f32,
    pub three_d: bool,
    pub luma: bool,
    pub inverted: bool,
}

/// Where a draw's pixels come from: decoded/synthesised bytes, or a nested
/// comp realised recursively on the GPU (Precomp layers).
#[cfg(feature = "media")]
pub enum DrawSource {
    Pixels {
        rgba: Vec<u8>,
        tex_w: u32,
        tex_h: u32,
    },
    Nested {
        width: u32,
        height: u32,
        background: [f64; 4],
        draws: Vec<CompLayerDraw>,
        /// The nested comp's own active camera at this time.
        camera: Option<kiriko_core::model::CameraPose>,
    },
}

#[cfg(feature = "media")]
pub struct CompLayerDraw {
    pub source: DrawSource,
    /// The layer's natural pixel size — transforms act in comp pixels even
    /// when the texture was decoded at a reduced preview resolution.
    pub natural_size: (f32, f32),
    pub position: (f32, f32),
    pub anchor: (f32, f32),
    pub scale: (f32, f32),
    pub rotation_deg: f32,
    pub opacity: f32,
    pub z: f32,
    pub rotation_x_deg: f32,
    pub rotation_y_deg: f32,
    pub three_d: bool,
    pub matte: Option<MatteDraw>,
    pub blend: kiriko_gpu::Blend,
    /// Layer-space mask coverage (white RGBA, alpha = coverage) for
    /// GPU-sourced layers — Precomps, whose pixels never exist CPU-side.
    pub mask_cov: Option<(Vec<u8>, u32, u32)>,
}

/// GPU display path (slice 5 completion): decoded sRGB bytes → linear fp16
/// working texture → display texture registered with egui. Falls back to the
/// CPU/egui-texture path when no wgpu render state exists.
#[cfg(feature = "media")]
pub struct GpuViewer {
    ctx: kiriko_gpu::GpuContext,
    engine: kiriko_gpu::ColourEngine,
    compositor: kiriko_gpu::Compositor,
    render_state: egui_wgpu::RenderState,
    /// Keep the display texture alive while egui samples it.
    current: Option<(egui_wgpu::wgpu::Texture, egui::TextureId)>,
}

#[cfg(feature = "media")]
impl GpuViewer {
    pub fn new(render_state: egui_wgpu::RenderState) -> Self {
        let ctx = kiriko_gpu::GpuContext::from_parts(
            render_state.device.clone(),
            render_state.queue.clone(),
        );
        let engine = kiriko_gpu::ColourEngine::new(&ctx);
        let compositor = kiriko_gpu::Compositor::new(&ctx);
        Self {
            ctx,
            engine,
            compositor,
            render_state,
            current: None,
        }
    }

    /// A second handle to the shared device for the export thread.
    pub fn export_context(&self) -> kiriko_gpu::GpuContext {
        kiriko_gpu::GpuContext::from_parts(self.ctx.device.clone(), self.ctx.queue.clone())
    }

    /// Composite a comp frame (evaluator v0) and register it for painting.
    /// `layers` is bottom-up draw order.
    /// Realise a draw list into a linear comp texture (recursive for Nested).
    fn realise(
        &self,
        camera: Option<kiriko_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> egui_wgpu::wgpu::Texture {
        let linear_textures: Vec<egui_wgpu::wgpu::Texture> = layers
            .iter()
            .map(|l| match &l.source {
                DrawSource::Pixels { rgba, tex_w, tex_h } => {
                    let src = self.engine.upload_srgb8(&self.ctx, rgba, *tex_w, *tex_h);
                    self.engine.linearise(&self.ctx, &src)
                }
                DrawSource::Nested {
                    width,
                    height,
                    background,
                    draws,
                    camera,
                } => self.realise(*camera, *width, *height, *background, draws),
            })
            .collect();
        let cam_mat = camera.map(|pose| crate::export::camera_mat(width, height, pose));
        // Layer-space mask textures (Precomp masks — GPU mask pass).
        let mask_textures: Vec<Option<egui_wgpu::wgpu::Texture>> = layers
            .iter()
            .map(|l| {
                l.mask_cov
                    .as_ref()
                    .map(|(rgba, w, h)| self.engine.upload_srgb8(&self.ctx, rgba, *w, *h))
            })
            .collect();
        // Matte layers render alone into comp space (one texture per consumer;
        // the shared-matte cache optimisation arrives with the evaluator).
        let matte_textures: Vec<Option<egui_wgpu::wgpu::Texture>> = layers
            .iter()
            .map(|l| {
                l.matte.as_ref().map(|m| {
                    let src = self
                        .engine
                        .upload_srgb8(&self.ctx, &m.rgba, m.tex_w, m.tex_h);
                    let linear = self.engine.linearise(&self.ctx, &src);
                    self.compositor.composite_with_camera(
                        &self.ctx,
                        width,
                        height,
                        [0.0, 0.0, 0.0, 0.0],
                        &[kiriko_gpu::CompositeLayer {
                            texture: &linear,
                            size: m.natural_size,
                            position: m.position,
                            anchor: m.anchor,
                            scale: m.scale,
                            rotation_deg: m.rotation_deg,
                            opacity: m.opacity,
                            matte: None,
                            blend: kiriko_gpu::Blend::Normal,
                            z: m.z,
                            rotation_x_deg: m.rotation_x_deg,
                            rotation_y_deg: m.rotation_y_deg,
                            three_d: m.three_d,
                            layer_mask: None,
                        }],
                        cam_mat,
                    )
                })
            })
            .collect();
        let comp_layers: Vec<kiriko_gpu::CompositeLayer> = linear_textures
            .iter()
            .zip(layers)
            .zip(&matte_textures)
            .zip(&mask_textures)
            .map(
                |(((texture, l), matte_tex), mask_tex)| kiriko_gpu::CompositeLayer {
                    texture,
                    size: l.natural_size,
                    position: l.position,
                    anchor: l.anchor,
                    scale: l.scale,
                    rotation_deg: l.rotation_deg,
                    opacity: l.opacity,
                    z: l.z,
                    rotation_x_deg: l.rotation_x_deg,
                    rotation_y_deg: l.rotation_y_deg,
                    three_d: l.three_d,
                    matte: matte_tex.as_ref().map(|mt| kiriko_gpu::MatteInput {
                        texture: mt,
                        luma: l.matte.as_ref().is_some_and(|m| m.luma),
                        inverted: l.matte.as_ref().is_some_and(|m| m.inverted),
                    }),
                    blend: l.blend,
                    layer_mask: mask_tex.as_ref(),
                },
            )
            .collect();
        self.compositor.composite_with_camera(
            &self.ctx,
            width,
            height,
            background,
            &comp_layers,
            cam_mat,
        )
    }

    /// Realise a comp frame straight to display-ready sRGB bytes (Kura's
    /// cache-fill path — nothing is registered for painting).
    fn realise_to_bytes(
        &self,
        camera: Option<kiriko_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> Option<Vec<u8>> {
        let linear = self.realise(camera, width, height, background, layers);
        let shown = self.engine.display(&self.ctx, &linear);
        self.engine.readback8(&self.ctx, &shown).ok()
    }

    /// Realise a comp's draws and register the frame for painting.
    fn present_comp(
        &mut self,
        camera: Option<kiriko_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> (egui::TextureId, egui::Vec2) {
        let linear = self.realise(camera, width, height, background, layers);
        let shown = self.engine.display(&self.ctx, &linear);
        let view = shown.create_view(&Default::default());
        let id = self.render_state.renderer.write().register_native_texture(
            &self.ctx.device,
            &view,
            egui_wgpu::wgpu::FilterMode::Linear,
        );
        if let Some((_, old)) = self.current.replace((shown, id)) {
            self.render_state.renderer.write().free_texture(&old);
        }
        (id, egui::vec2(width as f32, height as f32))
    }

    /// Upload a decoded frame through the colour pipeline; returns the egui
    /// texture id + size to paint.
    fn present(&mut self, rgba: &[u8], w: u32, h: u32) -> (egui::TextureId, egui::Vec2) {
        let src = self.engine.upload_srgb8(&self.ctx, rgba, w, h);
        let linear = self.engine.linearise(&self.ctx, &src);
        let shown = self.engine.display(&self.ctx, &linear);
        let view = shown.create_view(&Default::default());
        let id = self.render_state.renderer.write().register_native_texture(
            &self.ctx.device,
            &view,
            egui_wgpu::wgpu::FilterMode::Linear,
        );
        if let Some((_, old)) = self.current.replace((shown, id)) {
            self.render_state.renderer.write().free_texture(&old);
        }
        (id, egui::vec2(w as f32, h as f32))
    }
}

/// Persisted UI state (the dockable-panel layout; app state is runtime).
#[derive(Serialize, Deserialize)]
pub struct Shell {
    /// The tiling layout: which panels sit where, and their sizes.
    #[serde(default = "default_layout")]
    dock: egui_tiles::Tree<Panel>,
    #[serde(skip, default)]
    theme: Theme,
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
    /// Native macOS menu bar; None on other platforms (07-UI-SPEC).
    #[cfg(target_os = "macos")]
    #[serde(skip, default)]
    native_menu: Option<crate::native_menu::NativeMenu>,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            dock: default_layout(),
            theme: Theme::dark(),
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
        shell.theme.apply(ctx);
        ctx.style_mut(|s| s.visuals.panel_fill = shell.theme.surface_0);

        // The boot log (K-008): every line reflects real initialisation state.
        let mut lines = vec![
            BootLine::ok("Theme: aizome-dark"),
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
            kiriko_media::ffmpeg_version()
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
        lines.push(BootLine::ok(
            "Effects: none registered — suite arrives in phase 3",
        ));

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
                    #[cfg(feature = "media")]
                    self.start_export();
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
                MenuAction::AddSequenceLayer => self.app.add_sequence_layer(),
                MenuAction::CutClip => self.app.cut_sequence_at_playhead(),
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
        // Order matters: consume the more-modified shortcut first.
        if ctx.input_mut(|i| i.consume_shortcut(&REDO)) {
            self.app.redo();
        } else if ctx.input_mut(|i| i.consume_shortcut(&UNDO)) {
            self.app.undo();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SAVE)) {
            self.app.save();
        }
    }

    /// Size-targeted share export (K-037): bitrate from the byte budget.
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
        // 8% container/overhead headroom; audio joins the budget when comps
        // gain audio. Work area replaces whole-comp when it lands.
        let bits = target_mb * 1_000_000.0 * 8.0 * 0.92;
        let bit_rate = (bits / duration) as i64;
        self.start_export_with(Some(bit_rate), &format!("share-{}mb.mp4", target_mb as u64));
    }

    #[cfg(feature = "media")]
    fn start_export(&mut self) {
        self.start_export_with(None, "export.mp4");
    }

    #[cfg(feature = "media")]
    fn start_export_with(&mut self, bit_rate: Option<i64>, default_name: &str) {
        if self.export.is_some() {
            return;
        }
        let Some(comp_id) = self.app.preview_comp.or(self.app.selected_comp) else {
            self.app.error = Some("select a composition to export".into());
            return;
        };
        let Some(gpu) = &self.gpu else {
            self.app.error = Some("export needs the GPU pipeline".into());
            return;
        };
        let picked = rfd::FileDialog::new()
            .add_filter("MP4 video", &["mp4"])
            .set_file_name(default_name)
            .save_file();
        let Some(path) = picked else { return };
        let doc = self.app.store.snapshot();
        let items = crate::export::item_infos(&doc, &self.app.media);
        self.export = Some(crate::export::start(
            doc,
            comp_id,
            items,
            gpu.export_context(),
            path,
            bit_rate,
        ));
        self.export_progress = Some((0, 0));
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
                kiriko_core::mask::Mask::rectangle(w * 0.25, h * 0.25, w * 0.5, h * 0.5)
            }
            ShapeKind::Ellipse => {
                kiriko_core::mask::Mask::ellipse(w * 0.5, h * 0.5, w * 0.3, h * 0.3)
            }
            ShapeKind::Star => {
                kiriko_core::mask::Mask::star(w * 0.5, h * 0.5, w * 0.32, w * 0.14, 5)
            }
        };
        let mut masks = layer.masks.clone();
        masks.push(mask);
        self.app.commit(kiriko_core::Op::SetLayerMasks {
            comp: comp_id,
            layer: layer_id,
            masks,
        });
        #[cfg(feature = "media")]
        self.app.refresh_preview();
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
                        if ui
                            .selectable_label(lock, if lock { "🔒" } else { "🔓" })
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
        self.app.autosave_tick();
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
                let mut finished: Option<Result<std::path::PathBuf, String>> = None;
                while let Ok(ev) = export.events.try_recv() {
                    match ev {
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
                match finished {
                    Some(Ok(path)) => {
                        self.export = None;
                        self.export_progress = None;
                        self.app.error = Some(format!("exported {}", path.display()));
                    }
                    Some(Err(e)) => {
                        self.export = None;
                        self.export_progress = None;
                        self.app.error = Some(format!("export: {e}"));
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
            // Kura warm path: a cached frame presents as a plain upload.
            if let Some(key) = self.app.cached_present.take() {
                if let Some(gpu) = &mut self.gpu {
                    if let Some(frame) = self.app.comp_frame_cache.get(&key) {
                        let (w, h, rgba) = (frame.width, frame.height, frame.rgba.clone());
                        self.preview_display = Some(gpu.present(&rgba, w, h));
                    }
                }
            }
            // Idle: fill the work area around the playhead, one frame at a
            // time (any real request supersedes the fill mid-flight). Paused
            // while scrubbing/dragging so fills don't fight the interaction.
            if !self.app.is_playing()
                && !self.app.is_interacting()
                && self.app.fill_in_flight.is_none()
            {
                if let Some(comp_id) = self.app.preview_comp {
                    if let Some(frame) = self.app.next_fill_frame(comp_id) {
                        self.app.request_fill_frame(comp_id, frame);
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

            // Live value-drag preview: while a transform value is being dragged,
            // re-composite the retained frame with the provisional value patched
            // in for this frame only — instant feedback with no re-decode, since
            // a transform change never alters which footage frame a layer shows.
            if let (Some((edit_layer, prop, value)), Some(comp_id)) =
                (self.app.prop_edit, self.app.preview_comp)
            {
                if let (Some(gpu), Some(cf)) = (&mut self.gpu, &self.last_comp) {
                    if cf.comp == comp_id && cf.frame == self.app.preview_frame {
                        let doc = self.app.store.snapshot();
                        if let Some(comp) = doc.comp(comp_id) {
                            let patched = patch_layer_prop(comp, edit_layer, prop, value);
                            let t_comp = cf.frame as f64 / comp.frame_rate.fps().max(1.0);
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
                        self.start_export();
                        ui.close_menu();
                    }
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
                    if ui.button("Add sequence layer").clicked() {
                        self.app.add_sequence_layer();
                        ui.close_menu();
                    }
                    if ui.button("Cut clip at playhead").clicked() {
                        self.app.cut_sequence_at_playhead();
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
                    if ui.button("Reset workspace").clicked() {
                        self.dock = default_layout();
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
            ui.horizontal(|ui| {
                let tool = self.app.tool;
                if ui
                    .selectable_label(tool == ToolMode::Select, "Select")
                    .on_hover_text("Select / move the view (V)")
                    .clicked()
                {
                    self.app.tool = ToolMode::Select;
                }
                if ui
                    .selectable_label(tool == ToolMode::Hand, "Hand")
                    .on_hover_text("Drag to pan the view (H)")
                    .clicked()
                {
                    self.app.tool = ToolMode::Hand;
                }
                let shape_resp = ui
                    .selectable_label(
                        tool == ToolMode::Shape,
                        format!("Shape · {}", self.app.shape_kind.label()),
                    )
                    .on_hover_text(
                        "Drag in the Viewer to draw a mask — right-click to pick a shape (Q)",
                    );
                if shape_resp.clicked() {
                    self.app.tool = ToolMode::Shape;
                }
                shape_resp.context_menu(|ui| {
                    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Star] {
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
                if ui
                    .selectable_label(tool == ToolMode::Pen, "Pen")
                    .on_hover_text("Click points to draw a mask; click the first to close (G)")
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
                    ui.label(
                        egui::RichText::new(format!("exporting {frame}/{total}"))
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

        // The tiling dock fills the window: the Viewer is a bare pane with no
        // tab (K-074), every other panel carries a tab and can be dragged to
        // re-arrange the workspace.
        let Shell {
            dock,
            theme,
            app,
            preview_display,
            ..
        } = self;
        let mut behavior = DockBehavior {
            theme,
            app,
            preview_display: *preview_display,
        };
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme.surface_0))
            .show(ctx, |ui| dock.ui(&mut behavior, ui));
    }
}

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod geometry_tests {
    use super::*;
    use crate::app_state::preview::CompLayerPixels;
    use kiriko_core::model::{
        Composition, Document, Layer, LayerKind, LinearColour, Switches, TransformGroup,
    };
    use kiriko_core::time::{CompTime, Duration, FrameRate, Rational};
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
            blend: Default::default(),
            masks: Vec::new(),
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

    // The live value-drag preview renders a comp patched with the provisional
    // value. Patching a layer's Position X to 500 must show through as the
    // draw's position, without touching the committed document.
    #[test]
    fn patch_layer_prop_overrides_the_previewed_value() {
        use kiriko_core::model::TransformProp;
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
            blend: Default::default(),
            masks: Vec::new(),
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
        };
        let mut map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        map.insert(layer.id, &lp);
        let doc = Document::new();
        let mut visited = vec![patched.id];
        let draws = build_comp_draws(&doc, &patched, 0.0, &map, &mut visited);
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].position.0, 500.0);
    }
}
