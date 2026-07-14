//! The application shell: menu bar, docked panels, status line.
//!
//! Layout per docs/07-UI-SPEC.md (Edit workspace): Project left, Viewer centre,
//! Effect Controls / Effects & Presets right, Timeline across the bottom.

use crate::app_state::{AppState, ShapeKind, ToolMode};
use crate::icons::Icon;
use crate::splash::{BootLine, Splash};
use crate::theme::Theme;
use kiriko_core::model::ProjectItem;
use serde::{Deserialize, Serialize};

/// The dockable panels. Names are glossary names (docs/01-GLOSSARY.md §7).
/// A dockable panel (a pane in the tiling tree). The Viewer is special: it is
/// the only pane kept out of any tab container, so it shows no tab bar (K-074,
/// Mack: the viewport must have no top bit); every other panel carries a tab
/// and can be dragged to re-arrange the workspace.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
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

/// The default workspace: a full-width Timeline strip along the bottom, beneath a
/// band of slim tool columns flanking a tall Viewer. The Viewer is a bare pane (no
/// tab); Project/effects share a tab group on the left, Scopes on the right, and
/// the Timeline tabs span the whole width below both — the editing-suite default.
pub fn default_layout() -> egui_tiles::Tree<Panel> {
    let mut tiles = egui_tiles::Tiles::default();
    let viewer = tiles.insert_pane(Panel::Viewer);

    let project = tiles.insert_pane(Panel::Project);
    let fx = tiles.insert_pane(Panel::EffectControls);
    let fxp = tiles.insert_pane(Panel::EffectsAndPresets);
    let left = tiles.insert_tab_tile(vec![project, fx, fxp]);
    let scopes = tiles.insert_pane(Panel::Scopes);
    let right = tiles.insert_tab_tile(vec![scopes]);

    // Upper band: the tool columns either side of the Viewer.
    let upper = tiles.insert_horizontal_tile(vec![left, viewer, right]);
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) =
        tiles.get_mut(upper)
    {
        lin.shares.set_share(left, 0.22);
        lin.shares.set_share(viewer, 0.58);
        lin.shares.set_share(right, 0.20);
    }

    // The Timeline is a direct child of the vertical root, so it spans the full
    // window width along the bottom rather than only the Viewer's column.
    let timeline = tiles.insert_pane(Panel::Timeline);
    let timeline_tabs = tiles.insert_tab_tile(vec![timeline]);
    let root = tiles.insert_vertical_tile(vec![upper, timeline_tabs]);
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) =
        tiles.get_mut(root)
    {
        lin.shares.set_share(upper, 0.68);
        lin.shares.set_share(timeline_tabs, 0.32);
    }

    egui_tiles::Tree::new("kiriko-dock", root, tiles)
}

/// Render one panel's body. Shared by the docked panes and the pop-out windows
/// so a panel looks the same wherever it lives. Only the Viewer needs the live
/// preview texture; it never pops out, so floating windows pass `None`.
fn render_panel(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    preview_display: Option<(egui::TextureId, egui::Vec2)>,
    panel: Panel,
) {
    match panel {
        Panel::Viewer => viewer_panel(ui, theme, app, preview_display),
        Panel::Project => project_panel(ui, theme, app),
        Panel::Timeline => timeline_panel(ui, theme, app),
        Panel::EffectControls => empty_hint(
            ui,
            theme,
            "No layer selected",
            "Select a layer to see its effect stack.",
        ),
        Panel::EffectsAndPresets => effects_panel(ui, theme),
        Panel::Scopes => empty_hint(
            ui,
            theme,
            "Scopes",
            "Waveform, vectorscope and histogram arrive with the render pipeline.",
        ),
    }
}

/// The tile holding `panel`, if it is in the tree (each panel appears once).
fn tile_id_of(tree: &egui_tiles::Tree<Panel>, panel: Panel) -> Option<egui_tiles::TileId> {
    tree.tiles.iter().find_map(|(id, tile)| match tile {
        egui_tiles::Tile::Pane(p) if *p == panel => Some(*id),
        _ => None,
    })
}

/// Bridges the tiling tree to Kiriko's panels and house styling.
struct DockBehavior<'a> {
    theme: &'a Theme,
    app: &'a mut AppState,
    preview_display: Option<(egui::TextureId, egui::Vec2)>,
    /// Set when the user clicks a tab group's pop-out button; applied after the
    /// tree is drawn (the panel is hidden here and shown in its own window).
    pop_out: Option<Panel>,
}

impl egui_tiles::Behavior<Panel> for DockBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut Panel,
    ) -> egui_tiles::UiResponse {
        render_panel(ui, self.theme, self.app, self.preview_display, *pane);
        egui_tiles::UiResponse::None
    }

    fn top_bar_right_ui(
        &mut self,
        tiles: &egui_tiles::Tiles<Panel>,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        tabs: &egui_tiles::Tabs,
        _scroll_offset: &mut f32,
    ) {
        // A pop-out button for the active tab (the Viewer has no tab, so it
        // never gets one). Detaches the panel into its own window.
        if let Some(active) = tabs.active {
            if let Some(egui_tiles::Tile::Pane(panel)) = tiles.get(active) {
                if ui
                    .small_button("⇱")
                    .on_hover_text("Pop out into its own window")
                    .clicked()
                {
                    self.pop_out = Some(*panel);
                }
            }
        }
    }

    fn tab_title_for_pane(&mut self, pane: &Panel) -> egui::WidgetText {
        // The Timeline carries its own in-panel tab strip — one tab per open
        // comp (07-UI-SPEC §4) — so the dock tab keeps the plain panel name.
        pane.title().into()
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

/// Short label for a speed-ramp ease.
fn ease_label(e: kiriko_core::retime::Ease) -> &'static str {
    use kiriko_core::retime::Ease;
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

/// `HH:MM:SS:FF` frame timecode from seconds at `fps` — the Retime value lens
/// reading (K-075: "which source frame is showing here"). The frame field wraps
/// at `fps`; a whole extra second is carried up so `59:24`→`00`+1s at 25 fps.
fn fmt_timecode_frames(secs: f64, fps: f64) -> String {
    let fps_i = fps.round().max(1.0) as u64;
    let total_frames = (secs.max(0.0) * fps).round() as u64;
    let ff = total_frames % fps_i;
    let total_s = total_frames / fps_i;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{h:02}:{m:02}:{s:02}:{ff:02}")
}

/// The (in, out, start_offset) after moving a layer by `delta` comp seconds: all
/// three shift together, so the bar and its content move as one. Shifting the
/// span without `start_offset` would *slip* the content instead of moving it.
fn moved_span(
    in_point: kiriko_core::time::CompTime,
    out_point: kiriko_core::time::CompTime,
    start_offset: kiriko_core::time::CompTime,
    delta: f64,
) -> (
    kiriko_core::time::CompTime,
    kiriko_core::time::CompTime,
    kiriko_core::time::CompTime,
) {
    let shift = |t: kiriko_core::time::CompTime| {
        kiriko_core::time::CompTime(rational_at(t.0.to_f64() + delta))
    };
    (shift(in_point), shift(out_point), shift(start_offset))
}

/// The lane-area horizontal view (07-UI-SPEC §4): pixels-per-second and the
/// clamped left-edge comp time, from a zoom (1.0 = the whole comp fits `track_w`;
/// larger zooms in) and a desired left time. The view never scrolls past the
/// comp ends, so at zoom 1 it always shows the whole comp from 0.
fn lane_view(track_w: f32, duration: f64, zoom: f64, view_start: f64) -> (f64, f64) {
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
fn drag_secs(dx_px: f64, px_per_sec: f64) -> f64 {
    dx_px / px_per_sec.max(1e-6)
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
        ui.spacing_mut().item_spacing.x = 2.0;
        if icon_button(ui, theme, Icon::Folder, false)
            .on_hover_text("New folder")
            .clicked()
        {
            app.new_folder();
        }
        // The composition button also accepts a dropped item: dragging footage
        // onto it opens the New composition dialogue pre-filled from that
        // footage's size/rate/duration, queuing it as the first layer (K-068).
        let new_comp = icon_button(ui, theme, Icon::Film, false)
            .on_hover_text("New composition — or drop footage here to match its settings");
        if new_comp.clicked() {
            app.open_new_comp_dialog(None);
        }
        if new_comp.dnd_hover_payload::<uuid::Uuid>().is_some() {
            ui.painter().rect_stroke(
                new_comp.rect.expand(2.0),
                3.0,
                egui::Stroke::new(1.0_f32, theme.accent),
                egui::StrokeKind::Outside,
            );
        }
        if let Some(payload) = new_comp.dnd_release_payload::<uuid::Uuid>() {
            app.open_new_comp_dialog(Some(*payload));
        }
    });
    ui.add_space(2.0);

    if doc.items.is_empty() {
        let bg_rect = ui.available_rect_before_wrap();
        empty_hint(
            ui,
            theme,
            "No footage yet",
            "Double-click here to import, drag files in, or use File → Import.",
        );
        let bg = ui.interact(
            bg_rect,
            ui.id().with("empty-backdrop"),
            egui::Sense::click(),
        );
        if bg.double_clicked() {
            app.import_footage_dialog();
        }
        return;
    }

    // The tree. A backdrop interaction sits UNDER the rows (created first, so
    // the rows drawn next claim their own clicks and drops): only input on empty
    // panel space reaches it. Double-click the backdrop to Import (AE
    // convention); releasing a dragged item here files it at the root.
    let bg_rect = ui.available_rect_before_wrap();
    let bg = ui.interact(
        bg_rect,
        ui.id().with("panel-backdrop"),
        egui::Sense::click(),
    );
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
    if bg.double_clicked() {
        app.import_footage_dialog();
    }
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
                // Selection only drives the info header/highlight. The active
                // Timeline comp changes only when a comp is opened (double-click
                // or a tab), so selecting a comp must not switch tabs underfoot.
                app.selected_item = Some(id);
            }
            PanelAction::OpenComp(id) => app.open_comp(id),
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
                // A deleted comp also loses its Timeline tab (neighbour takes
                // over, or the Timeline empties).
                app.close_comp_tab(id);
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

/// A row that is one widget yet both a click target and a drag source. egui's
/// `dnd_drag_source` lays a drag-only overlay over its contents, and that
/// overlay swallows plain clicks — Project-panel rows looked dead (you could not
/// open a comp or preview footage by clicking). A single `Button` that senses
/// click *and* drag keeps both; while dragged it registers `payload` so every
/// existing drop target (folders, Timeline, Viewer, the "+ Composition" button)
/// keeps working unchanged. `dnd_drag_source`'s ghost-under-cursor is dropped;
/// the drop targets' own hover highlight stands in for it.
fn draggable_row<P: std::any::Any + Send + Sync>(
    ui: &mut egui::Ui,
    id: egui::Id,
    payload: P,
    selected: bool,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    let resp = ui
        .push_id(id, |ui| {
            ui.add(
                egui::Button::new(text)
                    .selected(selected)
                    .frame(false)
                    .sense(egui::Sense::click_and_drag()),
            )
        })
        .inner;
    // Only actually sets the payload while this row is the one being dragged.
    resp.dnd_set_drag_payload(payload);
    resp
}

/// A compact icon button in the house toolbar style (docs/15-DESIGN.md §5): a
/// stroke glyph in `text_secondary`, brightening to `text_primary` on hover and
/// `accent` when `active`, over a faint surface chip. Returns the response so
/// the caller reads `.clicked()` and attaches a tooltip with `.on_hover_text`.
fn icon_button(ui: &mut egui::Ui, theme: &Theme, icon: Icon, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::click());
    let hovered = resp.hovered();
    if active || hovered {
        ui.painter().rect_filled(
            rect.shrink(1.0),
            4.0,
            if active {
                theme.surface_3
            } else {
                theme.surface_2
            },
        );
    }
    if active {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            4.0,
            egui::Stroke::new(1.0_f32, theme.accent),
            egui::StrokeKind::Inside,
        );
    }
    let color = if active {
        theme.accent
    } else if hovered {
        theme.text_primary
    } else {
        theme.text_secondary
    };
    crate::icons::paint(ui.painter(), rect, icon, color, 1.5);
    resp
}

/// The identity glyph and §6.1 colour for a layer type. No longer drawn in the
/// outline (layer type reads from the lane bar's colour instead, Mack); kept for
/// that planned per-type lane colouring.
#[allow(dead_code)]
fn layer_type_style(kind: &kiriko_core::model::LayerKind, theme: &Theme) -> (Icon, egui::Color32) {
    use kiriko_core::model::LayerKind;
    match kind {
        LayerKind::Footage { .. } => (Icon::Footage, theme.layer.footage),
        LayerKind::Sequence { .. } => (Icon::Sequence, theme.layer.sequence),
        LayerKind::Precomp { .. } => (Icon::Comp, theme.layer.precomp),
        LayerKind::Solid { .. } => (Icon::Solid, theme.layer.solid),
        LayerKind::Text { .. } => (Icon::Text, theme.layer.text),
        LayerKind::Camera { .. } => (Icon::Camera, theme.layer.camera),
        // Reuses the solid glyph/colour for now (an adjustment layer is a
        // comp-sized effect container); a distinct glyph is a later refinement.
        LayerKind::Adjustment => (Icon::Solid, theme.layer.solid),
    }
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
    // Type glyph + tint carried on the left of the row (replaces the old text
    // tag): comps take the accent, the rest a muted tint (docs/15-DESIGN.md §5).
    let (type_icon, tag_colour) = match item {
        ProjectItem::Footage(_) => (Icon::Footage, theme.text_muted),
        ProjectItem::Folder(_) => (Icon::Folder, theme.text_muted),
        ProjectItem::Composition(_) => (Icon::Comp, theme.accent),
        ProjectItem::Solid(_) => (Icon::Solid, theme.text_muted),
    };
    let selected = app.selected_item == Some(id);
    let open_id = ui.id().with(("folder-open", id));
    let mut open = is_folder && ui.data(|d| d.get_temp::<bool>(open_id).unwrap_or(true));

    let row = ui
        .horizontal(|ui| {
            ui.add_space(12.0 * depth as f32 + 2.0);
            if is_folder {
                let (arrow_rect, arrow_resp) =
                    ui.allocate_exact_size(egui::vec2(13.0, 14.0), egui::Sense::click());
                crate::icons::disclosure(ui.painter(), arrow_rect, open, theme.text_muted);
                if arrow_resp.clicked() {
                    open = !open;
                    ui.data_mut(|d| d.insert_temp(open_id, open));
                }
            } else {
                ui.add_space(13.0); // align type glyphs under the folder rows'
            }
            let (icon_rect, _) =
                ui.allocate_exact_size(egui::vec2(15.0, 15.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), icon_rect, type_icon, tag_colour, 1.4);
            let label = egui::RichText::new(item.name()).color(theme.text_secondary);
            draggable_row(ui, ui.id().with(("row", id)), id, selected, label)
        })
        .inner;

    // Single click selects (info header, highlight); footage also previews so
    // browsing stays a one-click scrub. A comp opens in the Timeline on a
    // double-click (AE: single-click selects, double-click opens).
    if row.clicked() {
        actions.push(PanelAction::Select(id));
        if let ProjectItem::Footage(_) = item {
            actions.push(PanelAction::PreviewFootage(id));
        }
    }
    if row.double_clicked() {
        if let ProjectItem::Composition(_) = item {
            actions.push(PanelAction::OpenComp(id));
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
            // Dropping a comp with nothing open just opens it (as a tab).
            app.open_comp(item);
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

/// The Timeline's comp tab strip: one tab per open comp (07-UI-SPEC §4), the
/// active one highlighted. Clicking a tab switches to that comp; the × closes
/// its tab (the comp stays in the Project panel). Nothing is drawn when no comp
/// is open — the empty-state hint covers that.
fn comp_tab_strip(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    let doc = app.store.snapshot();
    // Open comps that still exist, in tab order. A comp deleted elsewhere just
    // drops out of the strip.
    let tabs: Vec<(uuid::Uuid, String)> = app
        .open_comps
        .iter()
        .filter_map(|id| doc.comp(*id).map(|c| (*id, c.name.clone())))
        .collect();
    if tabs.is_empty() {
        return;
    }
    let active = app.selected_comp;
    let mut activate: Option<uuid::Uuid> = None;
    let mut close: Option<uuid::Uuid> = None;
    let strip = egui::Frame::new()
        .fill(theme.surface_1)
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (id, name) in &tabs {
                    let is_active = active == Some(*id);
                    let name_btn = ui.add(
                        egui::Button::new(egui::RichText::new(trim_title(name)).small().color(
                            if is_active {
                                theme.text_primary
                            } else {
                                theme.text_secondary
                            },
                        ))
                        .fill(if is_active {
                            theme.surface_3
                        } else {
                            theme.surface_2
                        })
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            if is_active {
                                theme.accent
                            } else {
                                theme.hairline
                            },
                        )),
                    );
                    // Re-clicking the active tab is a no-op (don't reset its
                    // playhead); only switching tabs re-activates.
                    if name_btn.clicked() && !is_active {
                        activate = Some(*id);
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("×").small().color(theme.text_muted),
                            )
                            .frame(false),
                        )
                        .on_hover_text("Close this comp tab")
                        .clicked()
                    {
                        close = Some(*id);
                    }
                }
            });
        });
    // The strip (across the full panel width, so the empty space beside the
    // tabs counts) is a drop target: dropping a comp here opens it as its own
    // tab (separate from the comp already open); dropping anything else files
    // it into the active comp, matching the body. This is how you open a second
    // comp without nesting it — drop it beside the tabs, not into the timeline.
    let strip_rect = egui::Rect::from_min_max(
        egui::pos2(ui.max_rect().left(), strip.response.rect.top()),
        egui::pos2(ui.max_rect().right(), strip.response.rect.bottom()),
    );
    let strip_drop = ui.interact(
        strip_rect,
        ui.id().with("comp-strip-drop"),
        egui::Sense::hover(),
    );
    if strip_drop.dnd_hover_payload::<uuid::Uuid>().is_some() {
        ui.painter().rect_stroke(
            strip_rect,
            0.0,
            egui::Stroke::new(1.0_f32, theme.accent),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(id) = activate {
        app.open_comp(id);
    }
    if let Some(id) = close {
        app.close_comp_tab(id);
    }
    if let Some(payload) = strip_drop.dnd_release_payload::<uuid::Uuid>() {
        let dropped = *payload;
        if app.store.snapshot().comp(dropped).is_some() {
            app.open_comp(dropped); // open beside the tabs as a separate tab
        } else {
            app.add_item_to_comp(dropped); // footage/solid → into the active comp
        }
    }
}

fn timeline_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    // Comp tab strip first: it owns a drop zone (drop a comp here to open it as
    // a separate tab). The body below then accepts drops that file into the
    // open comp (footage → layer, comp → nested precomp). Splitting the two
    // regions is what lets a comp be opened *beside* the tabs versus added
    // *into* the open comp.
    comp_tab_strip(ui, theme, app);
    let body_rect = ui.available_rect_before_wrap();
    accept_item_drop(ui, theme, app, body_rect);
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
    // The comp tab strip above already names the open comp; no redundant
    // in-panel title, resolution or frame-rate line here (Mack).
    if comp.layers.is_empty() {
        ui.label(
            egui::RichText::new("Drag footage here to create the first layer.")
                .small()
                .color(theme.text_muted),
        );
        return;
    }
    use kiriko_core::anim::Animation;
    let mut pending: Option<kiriko_core::Op> = None;
    // A per-clip speed-ramp edit (start %, end %, ease), applied after the loop.
    let mut clip_ramp_edit: Option<(f64, f64, kiriko_core::retime::Ease)> = None;
    // A per-clip frame-interpolation edit, applied after the layer loop.
    let mut clip_interp_edit: Option<kiriko_core::retime::Interpolation> = None;

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
    // Horizontal view model (07-UI-SPEC §4): zoom + scrolled left edge. At zoom 1
    // this is exactly the old whole-comp-across-the-track mapping.
    let (px_per_sec, view_start) = lane_view(
        track_w,
        duration,
        app.timeline_zoom,
        app.timeline_view_start,
    );
    app.timeline_view_start = view_start; // persist the clamp
    let x_of = |seconds: f64| track_left + ((seconds - view_start) * px_per_sec) as f32;
    let seconds_of = |x: f32| view_start + (x - track_left) as f64 / px_per_sec.max(1e-6);

    // AE wheel shortcuts over the lane area: Alt = zoom (around the cursor),
    // Shift = scroll through time. Plain wheel falls through to the vertical
    // ScrollArea. Alt/Shift consume the scroll so it does not also scroll lanes.
    let lane_area = egui::Rect::from_min_max(
        egui::pos2(track_left, ui.max_rect().top()),
        egui::pos2(panel_right, ui.max_rect().bottom()),
    );
    let (scroll, mods, hover) =
        ui.input(|i| (i.raw_scroll_delta, i.modifiers, i.pointer.hover_pos()));
    if hover.is_some_and(|p| lane_area.contains(p)) {
        let cursor_x = hover.map(|p| p.x).unwrap_or(track_left);
        let consume = |ui: &mut egui::Ui| {
            ui.input_mut(|i| {
                i.raw_scroll_delta = egui::Vec2::ZERO;
                i.smooth_scroll_delta = egui::Vec2::ZERO;
            });
        };
        if mods.alt && scroll.y.abs() > 0.01 {
            // Alt-wheel: zoom the time axis around the cursor.
            let cursor_t = view_start + (cursor_x - track_left) as f64 / px_per_sec.max(1e-6);
            let factor = (scroll.y as f64 * 0.004).exp();
            app.timeline_zoom = (app.timeline_zoom * factor).clamp(1.0, 400.0);
            let (new_ppx, _) = lane_view(track_w, duration, app.timeline_zoom, view_start);
            app.timeline_view_start = cursor_t - (cursor_x - track_left) as f64 / new_ppx.max(1e-6);
            consume(ui);
        } else {
            // Horizontal scroll: a horizontal wheel (Shift-wheel arrives as
            // scroll.x on most platforms) or Shift + a vertical wheel. Plain
            // vertical wheel falls through to the ScrollArea.
            let h = if scroll.x.abs() > 0.01 {
                scroll.x
            } else if mods.shift {
                scroll.y
            } else {
                0.0
            };
            if h.abs() > 0.01 {
                app.timeline_view_start = view_start - h as f64 / px_per_sec.max(1e-6);
                consume(ui);
            }
        }
    }

    let (ruler_rect, ruler_resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 20.0),
        egui::Sense::click_and_drag(),
    );
    ui.painter().rect_filled(ruler_rect, 0.0, theme.surface_2);
    // Clip everything time-positioned (ruler ticks, bars, keyframes, markers,
    // playhead) to the lane area so a zoomed/scrolled view never bleeds left over
    // the layer outline. The outline columns replace this clip with their own
    // (via `place`), so they draw unaffected.
    let saved_clip = ui.clip_rect();
    ui.set_clip_rect(saved_clip.intersect(lane_area));
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
    // Detected tempo readout, right-aligned in the ruler.
    #[cfg(feature = "media")]
    if let Some((bc, bpm)) = app.detected_bpm {
        if bc == comp_id && bpm > 0.0 {
            ui.painter().text(
                egui::pos2(track_left + track_w - 2.0, ruler_rect.top() + 2.0),
                egui::Align2::RIGHT_TOP,
                format!("♪ {bpm:.0} BPM"),
                egui::FontId::monospace(9.0),
                theme.accent,
            );
        }
    }
    // Markers (docs 03 §11): beats are faint clay ticks fading by confidence;
    // user/chapter markers are full-height and solid.
    for m in &comp.markers {
        let x = x_of(m.time.0.to_f64());
        if x < track_left - 1.0 || x > track_left + track_w + 1.0 {
            continue;
        }
        let (col, top) = match m.kind {
            kiriko_core::markers::MarkerKind::Beat { confidence } => (
                theme
                    .accent
                    .gamma_multiply(0.25 + 0.55 * confidence.clamp(0.0, 1.0)),
                ruler_rect.top() + 9.0,
            ),
            _ => (theme.accent, ruler_rect.top()),
        };
        ui.painter().line_segment(
            [egui::pos2(x, top), egui::pos2(x, ruler_rect.bottom())],
            egui::Stroke::new(1.0_f32, col),
        );
    }
    if ruler_resp.clicked() || ruler_resp.dragged() {
        if let Some(pos) = ruler_resp.interact_pointer_pos() {
            // Map the cursor through the displayed (zoomed, scrolled) time axis,
            // so the playhead lands under the pointer at any zoom.
            let raw = seconds_of(pos.x).clamp(0.0, duration);
            // Snap the scrub to a nearby marker (within ~6 px) so the playhead
            // lands on the beat (docs/impl/beat-detection.md grid assist).
            let threshold = drag_secs(6.0, px_per_sec);
            let secs = kiriko_core::markers::snap_time(
                rational_at(raw),
                &comp.markers,
                rational_at(threshold),
            )
            .to_f64();
            app.preview_comp = Some(comp_id);
            app.comp_playback = None; // scrubbing pauses
            app.preview_frame =
                ((secs / duration * frames as f64) as usize).min(frames.saturating_sub(1));
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
    // Audio waveform strip (mono peaks) beneath the ruler, aligned to the same
    // time axis so beats and transients line up.
    #[cfg(feature = "media")]
    if let Some((wc, wf)) = &app.comp_waveform {
        if *wc == comp_id && !wf.is_empty() {
            let (wave_rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(track_left, wave_rect.top()),
                    egui::pos2(track_left + track_w, wave_rect.bottom()),
                ),
                0.0,
                theme.surface_0,
            );
            let cy = wave_rect.center().y;
            let half = wave_rect.height() * 0.45;
            let n = wf.len().max(1) as f32;
            let col = theme.text_muted.gamma_multiply(0.7);
            for (i, (lo, hi)) in wf.iter().enumerate() {
                let x = track_left + (i as f32 / n) * track_w;
                ui.painter().line_segment(
                    [egui::pos2(x, cy - hi * half), egui::pos2(x, cy - lo * half)],
                    egui::Stroke::new(1.0_f32, col),
                );
            }
        }
    }
    let rows_top = ui.cursor().top();

    // Graph mode (K-070) keeps everything around the lanes — the ruler, the
    // layer outline, the scrollbars and the bottom bar — and swaps only the
    // lane content for the curve editor (the AE shape). The rows below still
    // render their outline columns in both modes; only the drawing to the
    // right of `track_left` is suppressed, and the curve fills that area
    // after the ScrollArea.

    // Lanes scroll vertically when there are more layers than fit. Full-width clip
    // inside so `place` (which re-clips each outline column) sees the viewport.
    ui.set_clip_rect(saved_clip);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height((ui.available_height() - 38.0).max(48.0)) // leave the bottom bar + hscrollbar
        .id_salt(("timeline-lanes", comp_id))
        .show(ui, |ui| {
            // The scroll viewport (full width). Lane content clips to lane_area ∩ it;
            // outline columns clip to their own x but this viewport's y (never bleeding
            // above the ruler when a row is half-scrolled).
            let viewport = ui.clip_rect();
            ui.set_clip_rect(viewport.intersect(lane_area));
            for layer in &comp.layers {
                // The row response only hit-tests over the lane area (the clip
                // above), so in graph mode it goes inert — clicks there belong
                // to the curve, and the outline keeps its own controls.
                let row_sense = if app.timeline_graph_mode {
                    egui::Sense::hover()
                } else {
                    egui::Sense::click()
                };
                let (row_rect, row_resp) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), row_sense);
                if row_resp.clicked() {
                    app.selected_layer = Some(layer.id);
                }
                // Right-click a layer to add things (the house pattern: right-click or
                // menu, never scattered buttons).
                let mut ctx_op: Option<kiriko_core::Op> = None;
                let mut convert_layer = false;
                let mut trim_to_source = false;
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
                    // Trim to source end (K-022) — only offered for a retimed clip.
                    if matches!(
                        layer.kind,
                        kiriko_core::model::LayerKind::Footage {
                            retime: Some(_),
                            ..
                        }
                    ) && ui.button("Trim to source end").clicked()
                    {
                        trim_to_source = true;
                        ui.close_menu();
                    }
                });
                if ctx_op.is_some() {
                    pending = ctx_op;
                    app.selected_layer = Some(layer.id);
                }
                if trim_to_source {
                    app.selected_layer = Some(layer.id);
                    #[cfg(feature = "media")]
                    app.trim_selected_to_source_end();
                }
                if convert_layer {
                    app.selected_layer = Some(layer.id);
                    app.convert_to_sequenced_layer();
                }
                // Outline glyphs draw left of the lanes, so they paint through a
                // viewport-clipped painter, not the lane clip (which would hide
                // them). Child-UI columns (place/row_frame) already clip this way.
                // NB: with_clip_rect INTERSECTS with the ui's current clip, which
                // was narrowed to the lanes above — it would erase everything left
                // of the lanes (the twirl and every outline glyph). set_clip_rect
                // REPLACES the clip, so the full-width outline draws.
                let mut outline_painter = ui.painter().clone();
                outline_painter.set_clip_rect(viewport);
                // Selection highlight is the background — drawn first so the twirl
                // and glyphs on top of it stay visible.
                if app.selected_layer == Some(layer.id) {
                    outline_painter.rect_filled(
                        egui::Rect::from_min_max(
                            row_rect.min,
                            egui::pos2(row_rect.left() + name_w - 4.0, row_rect.bottom()),
                        ),
                        3.0,
                        theme.surface_2,
                    );
                }
                // Disclosure twirl: layer options hide until opened (AE behaviour).
                let twirl_id = ui.id().with(("twirl", layer.id));
                let mut expanded = ui.data(|d| d.get_temp::<bool>(twirl_id).unwrap_or(false));
                let tri = egui::Rect::from_min_size(
                    egui::pos2(row_rect.left() + 2.0, row_rect.top()),
                    egui::vec2(16.0, row_rect.height()),
                );
                // The ui's clip is the lanes, and egui hit-tests against rect ∩ clip
                // — so an outline interaction needs the clip widened, or the click
                // never registers (this is why the twirl looked dead).
                let tri_resp = {
                    let saved = ui.clip_rect();
                    ui.set_clip_rect(viewport);
                    let r = ui.interact(tri, twirl_id.with("hit"), egui::Sense::click());
                    ui.set_clip_rect(saved);
                    r
                };
                if tri_resp.clicked() {
                    expanded = !expanded;
                    ui.data_mut(|d| d.insert_temp(twirl_id, expanded));
                }
                // Secondary (not muted) so the twirl reads as a control against
                // the dark outline; brightens under the cursor like other glyphs.
                let twirl_col = if tri_resp.hovered() {
                    theme.text_primary
                } else {
                    theme.text_secondary
                };
                crate::icons::disclosure(&outline_painter, tri, expanded, twirl_col);
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
                let is_footage =
                    matches!(layer.kind, kiriko_core::model::LayerKind::Footage { .. });
                let eye_r = slot(row_rect.left() + 18.0, row_rect.left() + 36.0);
                let mute_r = slot(edge - 34.0, edge);
                let td_r = slot(edge - 60.0, edge - 38.0);
                let blend_r = slot(edge - 124.0, edge - 64.0);
                let matte_r = slot(edge - 178.0, edge - 128.0);
                // Layer type is encoded by the lane bar's colour (Mack) — no glyph
                // or colour tab in the outline.
                let title_r = slot(row_rect.left() + 58.0, edge - 182.0);
                let place =
                    |ui: &mut egui::Ui, r: egui::Rect, add: &mut dyn FnMut(&mut egui::Ui)| {
                        let mut child = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(r)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        // Clip to the column's own x but the scroll viewport's y, so an
                        // outline column in a half-scrolled row doesn't bleed past the ruler.
                        child.set_clip_rect(r.intersect(viewport));
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
                    matte_control(ui, theme, comp, comp_id, layer, &mut pending)
                });
                place(ui, blend_r, &mut |ui| {
                    blend_control(ui, comp_id, layer, &mut pending)
                });
                place(ui, td_r, &mut |ui| {
                    three_d_control(ui, comp_id, layer, &mut pending)
                });
                if is_footage {
                    place(ui, mute_r, &mut |ui| {
                        mute_control(ui, theme, comp_id, layer, &mut pending)
                    });
                }
                if select_this {
                    app.selected_layer = Some(layer.id);
                }
                // Lane content — the bar, its labels and its drag handles — only
                // draws in the layers view; in graph mode the lane area belongs
                // to the curve (drawn after the ScrollArea).
                if !app.timeline_graph_mode {
                    // A whole-layer move (dragging the bar body) slides the bar for preview;
                    // the snapped landing is what draws.
                    let move_dx = match app.move_edit {
                        Some((id, raw_in)) if id == layer.id => {
                            let thr = drag_secs(6.0, px_per_sec);
                            let snapped = kiriko_core::markers::snap_time(
                                rational_at(raw_in.max(0.0)),
                                &comp.markers,
                                rational_at(thr),
                            )
                            .to_f64();
                            snapped - layer.in_point.0.to_f64()
                        }
                        _ => 0.0,
                    };
                    let bar = egui::Rect::from_min_max(
                        egui::pos2(
                            x_of(layer.in_point.0.to_f64() + move_dx),
                            row_rect.top() + 2.0,
                        ),
                        egui::pos2(
                            x_of(layer.out_point.0.to_f64() + move_dx),
                            row_rect.bottom() - 2.0,
                        ),
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
                    // Overrun (K-022): a retimed clip that outruns its source holds the
                    // last frame — mark where and hatch the held tail in warning kraft
                    // (never a red alarm — house rule). Boundaries never move on their own.
                    #[cfg(feature = "media")]
                    if let kiriko_core::model::LayerKind::Footage {
                        item,
                        retime: Some(rt),
                    } = &layer.kind
                    {
                        use crate::app_state::media::MediaStatus;
                        if let Some(MediaStatus::Ready { probe, frames, .. }) =
                            app.media.map.get(item)
                        {
                            if let Some(v) = probe.video.as_ref() {
                                let src_dur = *frames as f64 / v.fps().max(1.0);
                                if let Some(ot) = rt.overrun_local_time(rational_at(src_dur)) {
                                    let ox = x_of(layer.start_offset.0.to_f64() + ot);
                                    if ox > bar.left() && ox < bar.right() - 0.5 {
                                        let hatch = theme.warning.gamma_multiply(0.5);
                                        let mut hx = ox;
                                        while hx < bar.right() {
                                            ui.painter().line_segment(
                                                [
                                                    egui::pos2(hx, bar.top()),
                                                    egui::pos2(
                                                        (hx + 6.0).min(bar.right()),
                                                        bar.bottom(),
                                                    ),
                                                ],
                                                egui::Stroke::new(1.0_f32, hatch),
                                            );
                                            hx += 6.0;
                                        }
                                        ui.painter().line_segment(
                                            [
                                                egui::pos2(ox, bar.top()),
                                                egui::pos2(ox, bar.bottom()),
                                            ],
                                            egui::Stroke::new(1.5_f32, theme.warning),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // Keyframe glyphs: a clay diamond on the bar at each keyframed time
                    // (across the layer's animated properties). Only when collapsed — when
                    // expanded, each property shows its own keys on its own row (K-072).
                    // Times are layer-local, so comp time = start_offset + keyframe time.
                    if !expanded {
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
                            let crect = egui::Rect::from_min_max(
                                egui::pos2(cs, bar.top() + 1.0),
                                egui::pos2(ce, bar.bottom() - 1.0),
                            );
                            // Click a clip to select it (for per-clip speed editing).
                            let cresp = ui.interact(
                                crect,
                                ui.id().with(("clip", clip.id)),
                                egui::Sense::click(),
                            );
                            if cresp.clicked() {
                                app.selected_clip = Some(clip.id);
                                app.selected_layer = Some(layer.id);
                            }
                            let sel = app.selected_clip == Some(clip.id);
                            ui.painter().rect(
                                crect,
                                2.0,
                                if sel {
                                    theme.surface_3
                                } else {
                                    theme.surface_2
                                },
                                egui::Stroke::new(
                                    if sel { 1.5_f32 } else { 1.0_f32 },
                                    if sel {
                                        theme.accent
                                    } else {
                                        theme.hairline_strong
                                    },
                                ),
                                egui::StrokeKind::Inside,
                            );
                            // A non-100% clip shows its speed.
                            if let Some(sp) = clip.constant_speed() {
                                if (sp - 1.0).abs() > 1e-6 && ce - cs > 24.0 {
                                    ui.painter().text(
                                        crect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        format!("{:.0}%", sp * 100.0),
                                        egui::FontId::monospace(8.0),
                                        theme.text_secondary,
                                    );
                                }
                            }
                            // Edit point (clip boundary) — the beat-sync landmark.
                            ui.painter().line_segment(
                                [egui::pos2(ce, bar.top()), egui::pos2(ce, bar.bottom())],
                                egui::Stroke::new(1.0_f32, theme.accent),
                            );
                        }
                    }

                    // Only bars visible in the (possibly zoomed/scrolled) lane area take
                    // pointer interaction, so an off-screen bar can't steal outline clicks.
                    let bar_visible = bar.right() > track_left + 0.5 && bar.left() < panel_right;
                    // Edge handles: drag to trim in/out (one SetLayerSpan op per release).
                    for out_edge in [false, true] {
                        if !bar_visible {
                            continue;
                        }
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
                                // Snap the trimmed edge to a nearby beat/marker (~6 px) so
                                // clips cut on the beat.
                                let threshold = drag_secs(6.0, px_per_sec);
                                let secs = kiriko_core::markers::snap_time(
                                    rational_at(seconds_of(pos.x)),
                                    &comp.markers,
                                    rational_at(threshold),
                                )
                                .to_f64();
                                app.trim_edit = Some((layer.id, out_edge, secs));
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
                                            in_point: kiriko_core::time::CompTime(rational_at(
                                                new_in,
                                            )),
                                            out_point: kiriko_core::time::CompTime(rational_at(
                                                new_out,
                                            )),
                                            start_offset: layer.start_offset,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    // Body drag: move the whole layer in comp time — shift in/out and
                    // start_offset together so the bar and its content move as one. Sequence
                    // layers keep their bodies for clip selection.
                    if bar_visible
                        && !matches!(layer.kind, kiriko_core::model::LayerKind::Sequence { .. })
                    {
                        let body = bar.shrink2(egui::vec2(6.0, 0.0));
                        if body.width() > 2.0 {
                            let resp = ui.interact(
                                body,
                                ui.id().with(("move", layer.id)),
                                egui::Sense::click_and_drag(),
                            );
                            if resp.dragged() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            } else if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                            if resp.clicked() {
                                app.selected_layer = Some(layer.id);
                            }
                            if resp.dragged() {
                                let dx_secs = drag_secs(resp.drag_delta().x as f64, px_per_sec);
                                let base = match app.move_edit {
                                    Some((id, s)) if id == layer.id => s,
                                    _ => layer.in_point.0.to_f64(),
                                };
                                app.move_edit = Some((layer.id, (base + dx_secs).max(0.0)));
                            }
                            if resp.drag_stopped() {
                                if let Some((id, raw_in)) = app.move_edit.take() {
                                    if id == layer.id {
                                        let thr = drag_secs(6.0, px_per_sec);
                                        let snapped = kiriko_core::markers::snap_time(
                                            rational_at(raw_in.max(0.0)),
                                            &comp.markers,
                                            rational_at(thr),
                                        )
                                        .to_f64();
                                        let delta = snapped - layer.in_point.0.to_f64();
                                        if delta.abs() > 1e-9 {
                                            let (in_point, out_point, start_offset) = moved_span(
                                                layer.in_point,
                                                layer.out_point,
                                                layer.start_offset,
                                                delta,
                                            );
                                            pending = Some(kiriko_core::Op::SetLayerSpan {
                                                comp: comp_id,
                                                layer: layer.id,
                                                in_point,
                                                out_point,
                                                start_offset,
                                            });
                                        }
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
                } // end of the lanes-view-only lane content
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
                                    ui.label(
                                        egui::RichText::new("Masks")
                                            .small()
                                            .color(theme.text_muted),
                                    );
                                    for (mi, mask) in layer.masks.iter().enumerate() {
                                        let mut masks = layer.masks.clone();
                                        if ui
                                            .selectable_label(
                                                mask.inverted,
                                                egui::RichText::new(format!("{} inv", mask.name))
                                                    .small(),
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
                                    ui.label(
                                        egui::RichText::new("Text").small().color(theme.text_muted),
                                    );
                                    let mut text = document.text.clone();
                                    let resp = ui.add(
                                        egui::TextEdit::singleline(&mut text).desired_width(180.0),
                                    );
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
                                    let lt = app.preview_frame as f64 / fps
                                        - layer.start_offset.0.to_f64();
                                    let committed = zoom.value_at(lt);
                                    let id = egui::Id::new(("zoom_edit", layer.id));
                                    let mut value =
                                        ui.data(|d| d.get_temp::<f64>(id)).unwrap_or(committed);
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
                        // Per-clip speed for the selected clip in a Sequence layer.
                        if let kiriko_core::model::LayerKind::Sequence { clips } = &layer.kind {
                            if let Some(cid) = app.selected_clip {
                                if let Some(clip) = clips.iter().find(|c| c.id == cid) {
                                    ui.indent(("clipspeed", layer.id), |ui| {
                                        // Speed ramp: start % → end % with an ease (equal
                                        // ends = constant speed). The montage gesture.
                                        ui.horizontal(|ui| {
                                            use kiriko_core::retime::Ease;
                                            ui.label(
                                                egui::RichText::new("Speed %")
                                                    .small()
                                                    .color(theme.text_muted),
                                            );
                                            let (rv0, rv1, rease) = clip
                                                .ramp_view()
                                                .map(|(a, b, e)| (a * 100.0, b * 100.0, e))
                                                .unwrap_or((100.0, 100.0, Ease::Linear));
                                            let id0 = egui::Id::new(("clipv0", cid));
                                            let mut s0 =
                                                ui.data(|d| d.get_temp::<f64>(id0)).unwrap_or(rv0);
                                            let r0 = ui.add(
                                                egui::DragValue::new(&mut s0)
                                                    .speed(1.0)
                                                    .range(-800.0..=800.0),
                                            );
                                            if r0.dragged() || r0.has_focus() {
                                                ui.data_mut(|d| d.insert_temp(id0, s0));
                                            }
                                            ui.label(egui::RichText::new("→").small());
                                            let id1 = egui::Id::new(("clipv1", cid));
                                            let mut s1 =
                                                ui.data(|d| d.get_temp::<f64>(id1)).unwrap_or(rv1);
                                            let r1 = ui.add(
                                                egui::DragValue::new(&mut s1)
                                                    .speed(1.0)
                                                    .range(-800.0..=800.0)
                                                    .suffix(" %"),
                                            );
                                            if r1.dragged() || r1.has_focus() {
                                                ui.data_mut(|d| d.insert_temp(id1, s1));
                                            }
                                            let mut new_ease = rease;
                                            bare_dropdown(ui, ease_label(rease), |ui| {
                                                for e in [
                                                    Ease::Linear,
                                                    Ease::Slow,
                                                    Ease::Fast,
                                                    Ease::Smooth,
                                                    Ease::Sharp,
                                                ] {
                                                    if ui
                                                        .selectable_label(e == rease, ease_label(e))
                                                        .clicked()
                                                    {
                                                        new_ease = e;
                                                        ui.close_menu();
                                                    }
                                                }
                                            });
                                            let released = r0.drag_stopped()
                                                || r0.lost_focus()
                                                || r1.drag_stopped()
                                                || r1.lost_focus();
                                            if released
                                                && ((s0 - rv0).abs() > 1e-6
                                                    || (s1 - rv1).abs() > 1e-6)
                                            {
                                                clip_ramp_edit = Some((s0, s1, rease));
                                                ui.data_mut(|d| {
                                                    d.remove::<f64>(id0);
                                                    d.remove::<f64>(id1);
                                                });
                                            }
                                            if new_ease != rease {
                                                clip_ramp_edit = Some((s0, s1, new_ease));
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            use kiriko_core::retime::{FlowParams, Interpolation};
                                            ui.label(
                                                egui::RichText::new("Frames")
                                                    .small()
                                                    .color(theme.text_muted),
                                            );
                                            for (label, val, active) in [
                                                (
                                                    "Nearest",
                                                    Interpolation::Nearest,
                                                    matches!(
                                                        clip.interpolation,
                                                        Interpolation::Nearest
                                                    ),
                                                ),
                                                (
                                                    "Blend",
                                                    Interpolation::Blend,
                                                    matches!(
                                                        clip.interpolation,
                                                        Interpolation::Blend
                                                    ),
                                                ),
                                                (
                                                    "Flow",
                                                    Interpolation::Flow(FlowParams::default()),
                                                    matches!(
                                                        clip.interpolation,
                                                        Interpolation::Flow(_)
                                                    ),
                                                ),
                                            ] {
                                                if ui.selectable_label(active, label).clicked()
                                                    && !active
                                                {
                                                    clip_interp_edit = Some(val);
                                                }
                                            }
                                        });
                                    });
                                }
                            }
                        }
                    });

                    // A retimed footage clip's frame-interpolation policy sits here as a
                    // compact row; the Transform group header and its rows follow below.
                    // A retimed footage clip shows its frame-interpolation policy here
                    // (K-021): Nearest is crisp; Blend crossfades neighbours for smoother
                    // slow motion. Only a retimed layer renders this row — every other
                    // layer has nothing here, so the Transform header sits right beneath.
                    if let kiriko_core::model::LayerKind::Footage {
                        retime: Some(rt), ..
                    } = &layer.kind
                    {
                        use kiriko_core::retime::{FlowParams, Interpolation};
                        ui.scope(|ui| {
                            ui.set_max_width(name_w - 10.0);
                            ui.indent(("txlabel", layer.id), |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("Frames")
                                            .small()
                                            .color(theme.text_muted),
                                    );
                                    let mut set: Option<Interpolation> = None;
                                    for (label, val, active) in [
                                        (
                                            "Nearest",
                                            Interpolation::Nearest,
                                            matches!(rt.interpolation, Interpolation::Nearest),
                                        ),
                                        (
                                            "Blend",
                                            Interpolation::Blend,
                                            matches!(rt.interpolation, Interpolation::Blend),
                                        ),
                                        (
                                            "Flow",
                                            Interpolation::Flow(FlowParams::default()),
                                            matches!(rt.interpolation, Interpolation::Flow(_)),
                                        ),
                                    ] {
                                        if ui.selectable_label(active, label).clicked() && !active {
                                            set = Some(val);
                                        }
                                    }
                                    if let Some(interp) = set {
                                        let mut r = rt.clone();
                                        r.interpolation = interp;
                                        pending = Some(kiriko_core::Op::SetLayerRetime {
                                            comp: comp_id,
                                            layer: layer.id,
                                            retime: Some(r),
                                        });
                                    }
                                });
                            });
                        });
                    }
                    // Transform group: its own twirl (open by default) revealing each
                    // animatable property as a timeline row — stopwatch/name/value in
                    // the left column, that property's keyframes on the track to the
                    // right; click a row to graph it (K-072).
                    let tf_id = ui.id().with(("transform-group", layer.id));
                    if group_header_row(ui, theme, "Transform", tf_id, true, viewport) {
                        transform_property_rows(
                            ui,
                            theme,
                            app,
                            comp,
                            comp_id,
                            layer,
                            name_w,
                            track_left,
                            track_w,
                            px_per_sec,
                            view_start,
                            viewport,
                            &mut pending,
                        );
                    }
                    // Effects group: the home for the layer's effect stack. The effects
                    // system itself lands later; the twirl is here so the shape is right.
                    let fx_id = ui.id().with(("effects-group", layer.id));
                    if group_header_row(ui, theme, "Effects", fx_id, false, viewport) {
                        ui.indent(("fx-empty", layer.id), |ui| {
                            ui.label(
                                egui::RichText::new(
                                    "No effects yet — the effects system is on the way.",
                                )
                                .small()
                                .color(theme.text_disabled),
                            );
                        });
                    }
                }
            }
        });
    // Time-positioned overlays (marker guides, playhead) re-clip to the lane area.
    ui.set_clip_rect(saved_clip.intersect(lane_area));
    // Vertical separator + drag handle: resizes the left column (Mack).
    let sep_bottom = ui.cursor().top();
    // Faint marker guide lines through the track rows, so beats line up across
    // every layer and the waveform (the ruler carries the bright ticks). Lanes
    // view only: the graph draws its own grid over that area.
    if !app.timeline_graph_mode {
        for m in &comp.markers {
            let x = x_of(m.time.0.to_f64());
            if x < track_left - 1.0 || x > track_left + track_w + 1.0 {
                continue;
            }
            let a = match m.kind {
                kiriko_core::markers::MarkerKind::Beat { confidence } => {
                    0.10 + 0.15 * confidence.clamp(0.0, 1.0)
                }
                _ => 0.4,
            };
            ui.painter().line_segment(
                [egui::pos2(x, rows_top), egui::pos2(x, sep_bottom)],
                egui::Stroke::new(1.0_f32, theme.accent.gamma_multiply(a)),
            );
        }
    }
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
    // Playhead over ruler and rows (clay, the one accent). In graph mode it
    // stops at the ruler: the curve draws its own playhead on its own axis.
    if app.preview_comp == Some(comp_id) {
        let x = x_of(app.preview_frame as f64 / comp.frame_rate.fps().max(1.0));
        let playhead_bottom = if app.timeline_graph_mode {
            rows_top
        } else {
            ui.cursor().top().max(rows_top)
        };
        ui.painter().line_segment(
            [
                egui::pos2(x, ruler_rect.top()),
                egui::pos2(x, playhead_bottom),
            ],
            egui::Stroke::new(1.5_f32, theme.accent),
        );
    }
    if let Some(op) = pending {
        follow_edit(app, &op); // the graph follows the key you just touched
        app.commit(op);
    }
    if let Some((v0, v1, ease)) = clip_ramp_edit {
        app.set_selected_clip_ramp(v0, v1, ease);
    }
    if let Some(interp) = clip_interp_edit {
        app.set_selected_clip_interp(interp);
    }
    // Graph mode (K-070): the curve editor fills the lane area — the outline,
    // ruler, scrollbars and bottom bar around it are the same in both views.
    // (Sharing the lanes' zoomed time axis is the next increment.)
    if app.timeline_graph_mode {
        let plot_rect = graph_lane_rect(track_left, track_w, rows_top, ui.max_rect().bottom());
        graph_lane_plot(ui, theme, app, comp, plot_rect);
    }
    ui.set_clip_rect(saved_clip); // release the lane clip for the bottom bar
    timeline_bottom_bar(
        ui,
        theme,
        app,
        track_left,
        panel_right,
        duration,
        view_start,
    );
}

/// The layer-view / graph-view switch (K-070), drawn right-anchored in `rect`.
fn graph_toggle(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState, rect: egui::Rect) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.set_clip_rect(rect);
    let graph = app.timeline_graph_mode;
    if icon_button(&mut child, theme, Icon::GraphCurve, graph)
        .on_hover_text("Graph editor")
        .clicked()
    {
        app.timeline_graph_mode = true;
    }
    if icon_button(&mut child, theme, Icon::TimelineBars, !graph)
        .on_hover_text("Layers")
        .clicked()
    {
        app.timeline_graph_mode = false;
    }
}

/// The lane-view bottom bar spanning the lanes area: zoom controls (`− + Fit` +
/// readout) on the left — joined in graph mode by the value/speed lens toggle,
/// its own group behind a hairline divider — the view toggle on the right, and
/// a draggable horizontal scrollbar just above it (shown when zoomed in).
fn timeline_bottom_bar(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    track_left: f32,
    panel_right: f32,
    duration: f64,
    view_start: f64,
) {
    let panel = ui.max_rect();
    let bar_top = panel.bottom() - 24.0;
    let sb_top = bar_top - 12.0;

    // Horizontal scrollbar (meaningful only when zoomed in), just above the bar.
    if app.timeline_zoom > 1.001 {
        let track = egui::Rect::from_min_max(
            egui::pos2(track_left, sb_top),
            egui::pos2(panel_right - 2.0, sb_top + 8.0),
        );
        ui.painter().rect_filled(track, 3.0, theme.surface_1);
        let visible = duration / app.timeline_zoom;
        let fs = (view_start / duration.max(1e-6)) as f32;
        let fl = (visible / duration.max(1e-6)) as f32;
        let thumb = egui::Rect::from_min_max(
            egui::pos2(track.left() + fs * track.width(), track.top()),
            egui::pos2(track.left() + (fs + fl) * track.width(), track.bottom()),
        );
        let resp = ui.interact(thumb, ui.id().with("h-scrollbar"), egui::Sense::drag());
        let col = if resp.hovered() || resp.dragged() {
            theme.accent
        } else {
            theme.text_muted
        };
        ui.painter().rect_filled(thumb, 3.0, col);
        if resp.dragged() {
            let dsec = resp.drag_delta().x as f64 / track.width().max(1.0) as f64 * duration;
            app.timeline_view_start = view_start + dsec;
        }
    }

    // Controls-bar background across the lanes.
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(track_left, bar_top),
            egui::pos2(panel_right, panel.bottom()),
        ),
        0.0,
        theme.surface_1,
    );

    // Zoom controls, bottom-left of the lanes (with room for the lens toggle
    // that joins them in graph mode).
    let zr = egui::Rect::from_min_max(
        egui::pos2(track_left + 4.0, bar_top),
        egui::pos2((panel_right - 64.0).max(track_left + 8.0), panel.bottom()),
    );
    let mut zc = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(zr)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    zc.set_clip_rect(zr);
    if zc
        .small_button("−")
        .on_hover_text("Zoom out (Alt-wheel)")
        .clicked()
    {
        app.timeline_zoom = (app.timeline_zoom / 1.4).max(1.0);
    }
    if zc
        .small_button("+")
        .on_hover_text("Zoom in (Alt-wheel)")
        .clicked()
    {
        app.timeline_zoom = (app.timeline_zoom * 1.4).min(400.0);
    }
    if zc
        .small_button("Fit")
        .on_hover_text("Fit the whole comp")
        .clicked()
    {
        app.timeline_zoom = 1.0;
        app.timeline_view_start = 0.0;
    }
    if app.timeline_zoom > 1.001 {
        zc.label(
            egui::RichText::new(format!("{:.0}%", app.timeline_zoom * 100.0))
                .small()
                .color(theme.text_muted),
        );
    }

    // The value/speed lens toggle (graph mode only): one lens shared by every
    // curve, so it lives here rather than in each plot's header. Its own group,
    // split from the zoom cluster by a hairline divider. The wording follows
    // the graphed channel: a retimed layer's Speed reads source timecode and
    // per cent, a transform property reads its value and rate of change.
    if app.timeline_graph_mode {
        zc.add_space(10.0);
        let x = zc.cursor().left();
        zc.painter().line_segment(
            [
                egui::pos2(x, bar_top + 5.0),
                egui::pos2(x, panel.bottom() - 5.0),
            ],
            egui::Stroke::new(1.0_f32, theme.hairline_strong),
        );
        zc.add_space(10.0);
        let retime = app.graph_retime;
        let (value_label, speed_label) = if retime {
            ("Source", "Speed %")
        } else {
            ("Value", "Speed")
        };
        if zc
            .selectable_label(!app.graph_speed_view, value_label)
            .on_hover_text(if retime {
                "Value lens — the source frame showing at each point (HH:MM:SS:FF)"
            } else {
                "Value lens — the property's value over time"
            })
            .clicked()
        {
            app.graph_speed_view = false;
        }
        if zc
            .selectable_label(app.graph_speed_view, speed_label)
            .on_hover_text(if retime {
                "Derivative lens — playback speed per cent (Vegas-style)"
            } else {
                "The rate-of-change view — drag a key to set its speed (K-070)"
            })
            .clicked()
        {
            app.graph_speed_view = true;
        }
    }

    // View toggle, bottom-right of the lanes.
    let gr = egui::Rect::from_min_max(
        egui::pos2(panel_right - 60.0, bar_top),
        egui::pos2(panel_right - 4.0, panel.bottom()),
    );
    graph_toggle(ui, theme, app, gr);
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
                    // Transport: play/pause the preview (also the space bar).
                    let playing = app.is_playing();
                    if icon_button(
                        ui,
                        theme,
                        if playing { Icon::Pause } else { Icon::Play },
                        playing,
                    )
                    .on_hover_text(if playing {
                        "Pause (space)"
                    } else {
                        "Play (space)"
                    })
                    .clicked()
                    {
                        app.toggle_play();
                    }
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

/// The lane-area rectangle the curve editor fills in graph mode: the lanes'
/// width, from just under the ruler to just above the bottom bar (the same
/// 38 px strip the lane ScrollArea reserves for the scrollbar and the bar).
fn graph_lane_rect(track_left: f32, track_w: f32, rows_top: f32, panel_bottom: f32) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(track_left, rows_top),
        egui::pos2(
            track_left + track_w,
            (panel_bottom - 38.0).max(rows_top + 24.0),
        ),
    )
}

/// Select-on-edit: point the graph at whatever a timeline or graph edit just
/// touched. Committing a transform-property or Retime op selects that layer
/// and graphs that channel, so the curve follows the key you just added or
/// moved — a stopwatch click, a value scrub, or a key drag all land you on
/// the right graph. Ops that touch neither kind of channel pass through
/// untouched; a Batch follows its first property op (linked scale edits lead
/// with Scale x, matching a click on the row's name).
fn follow_edit(app: &mut AppState, op: &kiriko_core::Op) {
    match op {
        kiriko_core::Op::SetTransformProperty { layer, prop, .. } => {
            app.selected_layer = Some(*layer);
            app.graph_prop = Some(*prop);
            app.graph_retime = false;
        }
        kiriko_core::Op::SetLayerRetime { layer, .. } => {
            app.selected_layer = Some(*layer);
            app.graph_retime = true;
        }
        kiriko_core::Op::Batch { ops } => {
            if let Some(first) = ops.first() {
                follow_edit(app, first);
            }
        }
        _ => {}
    }
}

/// The graph editor drawn into the timeline's lane area (07-UI-SPEC; K-070):
/// the selected layer's picked channel as a live curve — the Retime speed
/// channel for retimed footage (K-075), else a transform property. Keys drag
/// (one op per release), double-click adds, right-click removes. The outline,
/// ruler, scrollbars and bottom bar around it are the timeline's own; what to
/// graph is picked in the outline's twirled-open property rows.
fn graph_lane_plot(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    comp: &kiriko_core::model::Composition,
    plot_rect: egui::Rect,
) {
    use kiriko_core::model::TransformProp;

    // The curve for the selected layer / property.
    let Some(layer_id) = app.selected_layer else {
        hint_in_rect(ui, theme, plot_rect, "Select a layer to edit its curves.");
        return;
    };
    let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
        hint_in_rect(ui, theme, plot_rect, "The selected layer is gone.");
        return;
    };
    // Retime channel (K-075): a retimed footage layer's Speed, graphed like a
    // property — value lens = source position as timecode, derivative = speed %.
    if app.graph_retime {
        if let kiriko_core::model::LayerKind::Footage {
            item,
            retime: Some(rt),
        } = &layer.kind
        {
            // Source frame rate for the timecode: the probed footage fps when
            // media is present, else the comp's rate as a reasonable fallback.
            #[cfg(feature = "media")]
            let src_fps = app
                .media
                .map
                .get(item)
                .and_then(|s| match s {
                    crate::app_state::media::MediaStatus::Ready { probe, .. } => {
                        probe.video.as_ref().map(|v| v.fps())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| comp.frame_rate.fps());
            #[cfg(not(feature = "media"))]
            let src_fps = comp.frame_rate.fps();
            graph_plot_retime(ui, theme, app, comp, layer, rt, src_fps, plot_rect);
            return;
        }
        app.graph_retime = false; // selected layer isn't retimed footage
    }
    // Any property is graphable (not only animated ones): a still property draws
    // a flat line you can double-click to add the first keyframe to.
    let current = app.graph_prop.unwrap_or(TransformProp::PositionX);
    app.graph_prop = Some(current);
    graph_plot(ui, theme, app, comp, layer, current, plot_rect);
}

/// The Retime channel graphed (K-075): the value lens plots the source position
/// read as `HH:MM:SS:FF` frame timecode, the derivative lens plots speed per
/// cent. In the speed lens the speed keyframes are draggable (2b) — the retime
/// rebuilds from the edited keyframe and downstream boundaries recompute (K-070).
/// The value lens is read-only for now, as are eased/Map ramps in the speed lens.
#[allow(clippy::too_many_arguments)]
fn graph_plot_retime(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    comp: &kiriko_core::model::Composition,
    layer: &kiriko_core::model::Layer,
    retime: &kiriko_core::retime::Retime,
    src_fps: f64,
    rect: egui::Rect,
) {
    // Header: the Vegas default-lens setting and (in the speed lens) the
    // ramp-preset shelf that eases the segment under the playhead. The
    // Source/Speed lens toggle lives in the timeline's bottom bar.
    let mut preset_ease: Option<kiriko_core::retime::Ease> = None;
    let header = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + 22.0));
    {
        use kiriko_core::retime::Ease;
        let mut h = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        h.set_clip_rect(header);
        h.checkbox(&mut app.vegas_default_lens, "Vegas")
            .on_hover_text("Open the Speed channel to the speed-% lens by default (K-075)");
        // Ramp presets ease the speed segment under the playhead (§9.2).
        if app.graph_speed_view {
            h.separator();
            h.label("Ramp");
            for (label, ease) in [
                ("Lin", Ease::Linear),
                ("Slow", Ease::Slow),
                ("Fast", Ease::Fast),
                ("Smth", Ease::Smooth),
                ("Shrp", Ease::Sharp),
            ] {
                if h.small_button(label)
                    .on_hover_text("Ease the speed ramp under the playhead")
                    .clicked()
                {
                    preset_ease = Some(ease);
                }
            }
        }
    }
    let rect = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 22.0), rect.max);
    ui.painter().rect_filled(rect, 0.0, theme.surface_0);

    let duration = comp.duration.0.to_f64().max(1e-6);
    let x_of = |t: f64| rect.left() + ((t / duration) as f32) * rect.width();

    let speed_view = app.graph_speed_view;

    // Speed keyframes in % (K-075, 2b): draggable in the speed lens. Present only
    // when the retime is a Linear-Rate keyframe store; eased/Map ramps stay
    // read-only until the §9.2 editing lands.
    let dur = layer.out_point.0;
    let kfs: Vec<(f64, f64)> = retime
        .speed_keyframes()
        .map(|ks| {
            ks.iter()
                .map(|(t, s)| (t.to_f64(), s.to_f64() * 100.0))
                .collect()
        })
        .unwrap_or_default();
    let pct_to_speed = |pct: f64| {
        kiriko_core::Rational::from_f64_on_grid(pct / 100.0, 1000)
            .unwrap_or(kiriko_core::Rational::ONE)
    };
    // While dragging a handle, a provisional retime drives the live curve.
    let provisional = app.graph_retime_edit.and_then(|(idx, pct)| {
        let &(t, _) = kfs.get(idx)?;
        speed_with_key(&Some(retime.clone()), dur, t, pct_to_speed(pct))
    });
    let sampled: &kiriko_core::retime::Retime = provisional.as_ref().unwrap_or(retime);

    // Sample the active lens across the pane width.
    let samples = (rect.width() as usize / 2).max(16);
    let values: Vec<(f64, f64)> = (0..=samples)
        .map(|i| {
            let t = duration * i as f64 / samples as f64;
            let v = if speed_view {
                sampled.speed_at(t) * 100.0
            } else {
                sampled.evaluate(t)
            };
            (t, v)
        })
        .collect();
    let (mut lo, mut hi) = values.iter().fold((f64::MAX, f64::MIN), |(l, h), (_, v)| {
        (l.min(*v), h.max(*v))
    });
    if speed_view {
        lo = lo.min(0.0); // always frame 0% and 100%
        hi = hi.max(100.0);
        if let Some((_, p)) = app.graph_retime_edit {
            lo = lo.min(p); // keep the dragged handle in frame
            hi = hi.max(p);
        }
    }
    let pad = ((hi - lo).abs().max(1.0)) * 0.12;
    let (lo, hi) = (lo - pad, hi + pad);
    let y_of = |v: f64| rect.bottom() - (((v - lo) / (hi - lo)) as f32) * rect.height();

    // Y-axis scale: labelled gridlines in the active lens's units.
    if speed_view {
        // The speed lens marks its 0% and 100% references.
        for val in [0.0_f64, 100.0] {
            let y = y_of(val);
            ui.painter().line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(0.5_f32, theme.hairline),
            );
        }
        graph_y_axis(ui, theme, rect, lo, hi, |v| format!("{v:.0}%"));
    } else {
        // The value lens reads as source-frame timecode.
        graph_y_axis(ui, theme, rect, lo, hi, |v| {
            fmt_timecode_frames(v.max(0.0), src_fps)
        });
    }

    // The curve.
    let points: Vec<egui::Pos2> = values
        .iter()
        .map(|(t, v)| egui::pos2(x_of(*t), y_of(*v)))
        .collect();
    let colour = if speed_view {
        theme.curve[1]
    } else {
        theme.curve[0]
    };
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5_f32, colour),
    ));

    // Draggable speed-keyframe handles (K-075, 2b) — the speed lens only.
    let mut pending: Option<kiriko_core::Op> = None;

    // Apply a ramp preset: ease the Rate segment under the playhead (§9.2).
    // Works on any Rate segment, including one already eased; a no-op over a Map
    // segment or when the playhead is outside the retime.
    if let Some(ease) = preset_ease {
        let lt = app.preview_frame as f64 / comp.frame_rate.fps().max(1.0)
            - layer.start_offset.0.to_f64();
        if let Some(new_rt) = retime.with_segment_ease(rational_at(lt.max(0.0)), ease) {
            pending = Some(kiriko_core::Op::SetLayerRetime {
                comp: comp.id,
                layer: layer.id,
                retime: Some(new_rt),
            });
        }
    }

    if speed_view {
        for (idx, &(t, pct)) in kfs.iter().enumerate() {
            let shown_pct = match app.graph_retime_edit {
                Some((i, p)) if i == idx => p,
                _ => pct,
            };
            let pos = egui::pos2(x_of(t), y_of(shown_pct));
            let resp = ui.interact(
                egui::Rect::from_center_size(pos, egui::vec2(12.0, 12.0)),
                ui.id().with(("rtkey", layer.id, idx)),
                egui::Sense::click_and_drag(),
            );
            let active = app.graph_retime_edit.is_some_and(|(i, _)| i == idx);
            let colour = if resp.hovered() || active {
                theme.accent
            } else {
                theme.curve[1]
            };
            ui.painter().circle_filled(pos, 4.0, colour);
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let frac = ((rect.bottom() - p.y) / rect.height()) as f64;
                    app.graph_retime_edit = Some((idx, lo + frac * (hi - lo)));
                }
            }
            if resp.drag_stopped() {
                if let Some((i, p)) = app.graph_retime_edit.take() {
                    if i == idx {
                        if let Some(new_rt) =
                            speed_with_key(&Some(retime.clone()), dur, t, pct_to_speed(p))
                        {
                            pending = Some(kiriko_core::Op::SetLayerRetime {
                                comp: comp.id,
                                layer: layer.id,
                                retime: Some(new_rt),
                            });
                        }
                    }
                }
            }
        }
    }

    // Playhead + a header readout (source timecode and speed %).
    if app.preview_comp == Some(comp.id) {
        let lt = app.preview_frame as f64 / comp.frame_rate.fps().max(1.0)
            - layer.start_offset.0.to_f64();
        let x = x_of(lt.clamp(0.0, duration));
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0_f32, theme.accent),
        );
        let readout = format!(
            "{}   {:.1}%",
            fmt_timecode_frames(sampled.evaluate(lt), src_fps),
            sampled.speed_at(lt) * 100.0
        );
        ui.painter().text(
            egui::pos2(rect.right() - 6.0, rect.top() + 4.0),
            egui::Align2::RIGHT_TOP,
            readout,
            egui::FontId::monospace(11.0),
            theme.text_secondary,
        );
    }

    if let Some(op) = pending {
        follow_edit(app, &op); // the graph follows the key you just touched
        app.commit(op);
    }
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

/// Y-axis scale for a graph pane: a few horizontal gridlines with their values
/// labelled down the left inside edge, in small muted monospace so the curve
/// stays the focus. `fmt` turns a gridline's value into its label — the units
/// live there (%, °, timecode, per second). Drawn before the curve, which
/// paints over the gridlines.
fn graph_y_axis(
    ui: &egui::Ui,
    theme: &Theme,
    rect: egui::Rect,
    lo: f64,
    hi: f64,
    fmt: impl Fn(f64) -> String,
) {
    const LINES: u32 = 4; // evenly spaced, clear of the pane's edges
    for i in 1..=LINES {
        let frac = i as f32 / (LINES + 1) as f32;
        let y = rect.bottom() - frac * rect.height();
        let v = lo + f64::from(frac) * (hi - lo);
        ui.painter().line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(0.5_f32, theme.hairline),
        );
        ui.painter().text(
            egui::pos2(rect.left() + 4.0, y - 1.0),
            egui::Align2::LEFT_BOTTOM,
            fmt(v),
            egui::FontId::monospace(9.0),
            theme.text_muted,
        );
    }
}

/// A y-axis value formatted to suit the axis span: whole numbers once the
/// range is wide, more decimals as it narrows.
fn fmt_axis_value(v: f64, span: f64) -> String {
    if span.abs() >= 20.0 {
        format!("{v:.0}")
    } else if span.abs() >= 2.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

/// The unit a transform property's y-axis labels carry ("" for the pixel
/// properties, which read cleaner bare).
fn prop_unit(prop: kiriko_core::model::TransformProp) -> &'static str {
    use kiriko_core::model::TransformProp as P;
    match prop {
        P::ScaleX | P::ScaleY | P::Opacity => "%",
        P::Rotation | P::RotationX | P::RotationY => "°",
        _ => "",
    }
}

/// The glyph a keyframe draws with, coding its interpolation at a glance:
/// a square holds, a diamond is linear, a circle is a bezier (eased) key.
#[derive(Debug, PartialEq, Eq)]
enum KeyShape {
    Square,
    Diamond,
    Circle,
}

fn key_shape(k: &kiriko_core::anim::Keyframe) -> KeyShape {
    use kiriko_core::anim::SideInterp;
    if matches!(k.interp_in, SideInterp::Hold) || matches!(k.interp_out, SideInterp::Hold) {
        KeyShape::Square
    } else if matches!(k.interp_in, SideInterp::Bezier { .. })
        || matches!(k.interp_out, SideInterp::Bezier { .. })
    {
        KeyShape::Circle
    } else {
        KeyShape::Diamond
    }
}

/// A keyframe side's influence (bezier handle reach), defaulting to the AE
/// easy-ease third for Linear/Hold sides that carry none.
fn side_influence(side: kiriko_core::anim::SideInterp) -> f64 {
    match side {
        kiriko_core::anim::SideInterp::Bezier { influence, .. } => influence,
        _ => 1.0 / 3.0,
    }
}

/// Draw one keyframed property's value/speed curve inside `rect`, with a
/// compact Ease/Linear header and draggable keys (the Value/Speed lens toggle
/// lives in the timeline's bottom bar). In the speed lens each key's tangent
/// is draggable (K-070); the derivative curve updates live and the release
/// writes bezier speeds back to the keyframes.
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

    // Compact header: blanket ease/linear (the value/speed lens toggle lives
    // in the timeline's bottom bar).
    let header = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + 22.0));
    let mut set_sides: Option<SideInterp> = None;
    {
        let mut h = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        h.set_clip_rect(header);
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
    // A still (Static) property has no keys yet: graph a flat line at its value
    // that you can double-click to add the first keyframe to (Mack).
    let static_val = slot.value_at(0.0);
    let empty_keys: Vec<Keyframe> = Vec::new();
    let keys: &Vec<Keyframe> = match &slot.animation {
        Animation::Keyframed(k) => k,
        _ => &empty_keys,
    };

    // ---- plot geometry: x = layer time over the comp span, y = value ----
    ui.painter().rect_filled(rect, 0.0, theme.surface_0);
    let duration = comp.duration.0.to_f64().max(1e-6);
    let (mut vmin, mut vmax) = keys.iter().fold((f64::MAX, f64::MIN), |(lo, hi), k| {
        (lo.min(k.value), hi.max(k.value))
    });
    if keys.is_empty() {
        vmin = static_val;
        vmax = static_val;
    }
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
    // A speed-lens drag re-tangents one key so the whole derivative curve moves
    // live; the value/time are untouched (K-070 — a lens on the same store).
    if let Some((idx, sp)) = app.graph_speed_edit {
        if let Some(k) = shown.get_mut(idx) {
            let side = SideInterp::Bezier {
                speed: sp,
                influence: side_influence(k.interp_out),
            };
            k.interp_in = side;
            k.interp_out = side;
        }
    }

    // Curve polyline: value, or its derivative in the speed lens (central
    // difference at half-frame steps — display-first; exact closed forms
    // arrive with Retime's segment maths).
    let samples = (rect.width() as usize / 2).max(16);
    let fps_est = comp.frame_rate.fps().max(1.0);
    let sample_at = |t: f64| -> f64 {
        if app.graph_speed_view {
            let h = 0.5 / fps_est;
            let a = kiriko_core::anim::evaluate(&shown, t - h).unwrap_or(static_val);
            let b = kiriko_core::anim::evaluate(&shown, t + h).unwrap_or(static_val);
            (b - a) / (2.0 * h)
        } else {
            kiriko_core::anim::evaluate(&shown, t).unwrap_or(static_val)
        }
    };
    let values: Vec<(f64, f64)> = (0..=samples)
        .map(|i| {
            let t = duration * i as f64 / samples as f64;
            (t, sample_at(t))
        })
        .collect();
    // The speed lens scales to its own sampled range; `speed_of` inverts the
    // mapping so a dragged handle reads back as value-units/second.
    let (s_lo, s_hi) = {
        let (mut lo, mut hi) = values.iter().fold((f64::MAX, f64::MIN), |(l, h), (_, v)| {
            (l.min(*v), h.max(*v))
        });
        if let Some((_, sp)) = app.graph_speed_edit {
            lo = lo.min(sp);
            hi = hi.max(sp);
        }
        let pad = ((hi - lo).abs().max(1.0)) * 0.15;
        (lo - pad, hi + pad)
    };
    let speed_y =
        move |v: f64| rect.bottom() - (((v - s_lo) / (s_hi - s_lo)) as f32) * rect.height();
    let speed_of = move |y: f32| {
        s_lo + ((rect.bottom() - y) / rect.height()).clamp(0.0, 1.0) as f64 * (s_hi - s_lo)
    };
    // Y-axis scale: labelled gridlines in the active lens's units — the value
    // itself, or its rate of change per second in the speed lens.
    {
        let unit = prop_unit(current);
        if app.graph_speed_view {
            graph_y_axis(ui, theme, rect, s_lo, s_hi, |v| {
                format!("{}{unit}/s", fmt_axis_value(v, s_hi - s_lo))
            });
        } else {
            graph_y_axis(ui, theme, rect, vmin, vmax, |v| {
                format!("{}{unit}", fmt_axis_value(v, vmax - vmin))
            });
        }
    }
    let points: Vec<egui::Pos2> = values
        .iter()
        .map(|(t, v)| {
            let y = if app.graph_speed_view {
                speed_y(*v)
            } else {
                y_of(*v)
            };
            egui::pos2(x_of(*t), y)
        })
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

    // Keys: draggable squares (value lens); draggable speed handles in the
    // speed lens (K-070 — editing the tangent, round-tripping to the store).
    let mut pending: Option<Vec<Keyframe>> = None;
    if app.graph_speed_view {
        for (idx, key) in keys.iter().enumerate() {
            let x = x_of(key.time.to_f64());
            ui.painter().line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(0.5_f32, theme.hairline_strong),
            );
            // Where the handle rests: the key's bezier speed, or the sampled
            // derivative for a Linear/Hold key that carries no single speed.
            let rest = match (key.interp_out, key.interp_in) {
                (SideInterp::Bezier { speed, .. }, _) => speed,
                (_, SideInterp::Bezier { speed, .. }) => speed,
                _ => sample_at(key.time.to_f64()),
            };
            let sp = match app.graph_speed_edit {
                Some((i, s)) if i == idx => s,
                _ => rest,
            };
            let pos = egui::pos2(x, speed_y(sp));
            let resp = ui.interact(
                egui::Rect::from_center_size(pos, egui::vec2(12.0, 12.0)),
                ui.id().with(("gspd", layer_id, idx)),
                egui::Sense::click_and_drag(),
            );
            let active = app.graph_speed_edit.is_some_and(|(i, _)| i == idx);
            let colour = if resp.hovered() || active {
                theme.accent
            } else {
                theme.curve[1]
            };
            ui.painter().circle_filled(pos, 4.0, colour);
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    app.graph_speed_edit = Some((idx, speed_of(p.y)));
                }
            }
            if resp.drag_stopped() {
                if let Some((i, s)) = app.graph_speed_edit.take() {
                    if i == idx {
                        let mut new_keys = keys.clone();
                        let side = SideInterp::Bezier {
                            speed: s,
                            influence: side_influence(new_keys[i].interp_out),
                        };
                        new_keys[i].interp_in = side;
                        new_keys[i].interp_out = side;
                        pending = Some(new_keys);
                    }
                }
            }
        }
    }
    for (idx, key) in keys.iter().enumerate() {
        if app.graph_speed_view {
            break; // the speed lens is handled above; this loop is the value lens
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
        match key_shape(key) {
            KeyShape::Square => {
                ui.painter().rect_filled(
                    egui::Rect::from_center_size(pos, egui::vec2(7.0, 7.0)),
                    1.0,
                    colour,
                );
            }
            KeyShape::Circle => {
                ui.painter().circle_filled(pos, 4.0, colour);
            }
            KeyShape::Diamond => {
                let d = 4.5;
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(pos.x, pos.y - d),
                        egui::pos2(pos.x + d, pos.y),
                        egui::pos2(pos.x, pos.y + d),
                        egui::pos2(pos.x - d, pos.y),
                    ],
                    colour,
                    egui::Stroke::NONE,
                ));
            }
        }
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
        // Right-click a key: set its interpolation, or delete it.
        resp.context_menu(|ui| {
            let mut sides: Option<SideInterp> = None;
            let mut delete = false;
            if ui.button("Easy ease").clicked() {
                sides = Some(kiriko_core::anim::EASY_EASE);
                ui.close_menu();
            }
            if ui.button("Linear").clicked() {
                sides = Some(SideInterp::Linear);
                ui.close_menu();
            }
            if ui.button("Hold").clicked() {
                sides = Some(SideInterp::Hold);
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Delete key").clicked() {
                delete = true;
                ui.close_menu();
            }
            if let Some(si) = sides {
                let mut new_keys = keys.clone();
                new_keys[idx].interp_in = si;
                new_keys[idx].interp_out = si;
                pending = Some(new_keys);
            } else if delete {
                let mut new_keys = keys.clone();
                new_keys.remove(idx);
                pending = Some(new_keys);
            }
        });
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
        let op = kiriko_core::Op::SetTransformProperty {
            comp: comp.id,
            layer: layer_id,
            prop: current,
            animation,
        };
        follow_edit(app, &op); // the graph follows the key you just touched
        app.commit(op);
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
            // An adjustment layer has no pixels of its own; until its effect
            // stack exists it is a pass-through and draws nothing.
            LayerKind::Adjustment => return None,
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
    let col = if vis {
        theme.text_secondary
    } else {
        theme.text_disabled
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
    crate::icons::paint(ui.painter(), rect, Icon::Eye, col, 1.4);
    if resp.on_hover_text("Show / hide this layer").clicked() {
        *pending = Some(kiriko_core::Op::SetLayerVisible {
            comp: comp_id,
            layer: layer.id,
            visible: !vis,
        });
    }
}

/// Matte subcolumn: a labelled "Matte" dropdown (accent when a matte is set)
/// with a drawn caret to show it opens a menu — source pick + luma/invert flags.
fn matte_control(
    ui: &mut egui::Ui,
    theme: &Theme,
    comp: &kiriko_core::model::Composition,
    comp_id: uuid::Uuid,
    layer: &kiriko_core::model::Layer,
    pending: &mut Option<kiriko_core::Op>,
) {
    use kiriko_core::model::{MatteChannel, MatteRef};
    let has_matte = layer.matte.is_some();
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::click());
    let base = if has_matte {
        theme.accent
    } else {
        theme.text_secondary
    };
    let colour = if resp.hovered() {
        theme.text_primary
    } else {
        base
    };
    ui.painter().text(
        rect.left_center() + egui::vec2(2.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "Matte",
        egui::FontId::proportional(11.0),
        colour,
    );
    crate::icons::caret_down(
        ui.painter(),
        egui::pos2(rect.right() - 6.0, rect.center().y),
        colour,
    );
    let popup_id = ui.make_persistent_id(("matte-popup", layer.id));
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }
    let mut set: Option<Option<MatteRef>> = None;
    egui::popup::popup_below_widget(
        ui,
        popup_id,
        &resp,
        egui::PopupCloseBehavior::CloseOnClick,
        |ui| {
            ui.set_min_width(150.0);
            if ui.selectable_label(layer.matte.is_none(), "None").clicked() {
                set = Some(None);
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
        },
    );
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
    theme: &Theme,
    comp_id: uuid::Uuid,
    layer: &kiriko_core::model::Layer,
    pending: &mut Option<kiriko_core::Op>,
) {
    let muted = !layer.switches.audible;
    let (icon, col) = if muted {
        (Icon::Mute, theme.text_muted)
    } else {
        (Icon::Audio, theme.text_secondary)
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
    crate::icons::paint(ui.painter(), rect, icon, col, 1.4);
    if resp
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

/// Read-only context shared by every property row in a layer's twirl-down.
struct RowCtx<'a> {
    theme: &'a Theme,
    comp_id: uuid::Uuid,
    layer: &'a kiriko_core::model::Layer,
    lt: f64,
    off: f64,
    fps: f64,
    /// The lane scroll viewport, so property-row outlines clip to their own x
    /// but the viewport's y (no vertical bleed when a row is half-scrolled).
    viewport: egui::Rect,
    track_left: f32,
    track_w: f32,
    /// The displayed time axis (zoom + scroll), so property-row keyframe
    /// diamonds sit exactly under the layer bars at any zoom.
    px_per_sec: f64,
    view_start: f64,
    /// True in graph mode (K-070): the outline half of every row still draws,
    /// but nothing is painted on the lane side — the curve owns that area.
    graph_mode: bool,
}

/// New (scale_x, scale_y) when the linked Scale control is dragged so x becomes
/// `new_x`, keeping the x:y ratio. A ~zero old x has no defined ratio, so both
/// take the new value (uniform).
fn linked_scale(old_x: f64, old_y: f64, new_x: f64) -> (f64, f64) {
    if old_x.abs() < 1e-9 {
        (new_x, new_x)
    } else {
        (new_x, old_y * new_x / old_x)
    }
}

/// A collapsible sub-group header inside a layer's twirl-down ("Transform",
/// "Effects", …): a disclosure triangle and label, indented under the layer and
/// full width so it reads as a band. Persists and returns its open state.
fn group_header_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: &str,
    id: egui::Id,
    default_open: bool,
    viewport: egui::Rect,
) -> bool {
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(default_open);
    // The header lives in the outline, but the ui's clip is the lanes and egui
    // hit-tests against rect ∩ clip — so widen the clip or the twirl won't click.
    let (rect, resp) = {
        let saved = ui.clip_rect();
        ui.set_clip_rect(viewport);
        let r =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::click());
        ui.set_clip_rect(saved);
        r
    };
    // A group header sits in the outline column (left of the lanes). set_clip_rect
    // replaces the lane clip; with_clip_rect would intersect it and hide the row.
    let mut p = ui.painter().clone();
    p.set_clip_rect(viewport);
    if resp.hovered() {
        p.rect_filled(rect, 0.0, theme.surface_1);
    }
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    let cy = rect.center().y;
    let tx = rect.left() + 22.0;
    let tri = egui::Rect::from_center_size(egui::pos2(tx, cy), egui::vec2(12.0, 12.0));
    crate::icons::disclosure(&p, tri, open, theme.text_muted);
    p.text(
        egui::pos2(tx + 10.0, cy),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        theme.text_secondary,
    );
    open
}

/// The layer's transform properties as full-width timeline rows (K-072): each
/// row shows its stopwatch/name/value in the left column and its own keyframes
/// as diamonds on the track to the right; clicking a row's name graphs it.
/// Scale x/y share one row with a ratio lock (default on); unlocking splits
/// them into two independent rows with a relink control.
#[allow(clippy::too_many_arguments)]
fn transform_property_rows(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    comp: &kiriko_core::model::Composition,
    comp_id: uuid::Uuid,
    layer: &kiriko_core::model::Layer,
    _name_w: f32,
    track_left: f32,
    track_w: f32,
    px_per_sec: f64,
    view_start: f64,
    viewport: egui::Rect,
    pending: &mut Option<kiriko_core::Op>,
) {
    use kiriko_core::model::{LayerKind, TransformProp};

    let is_camera = matches!(layer.kind, LayerKind::Camera { .. });
    let three_d = layer.switches.three_d || is_camera;
    let fps = comp.frame_rate.fps().max(1.0);
    let ctx = RowCtx {
        theme,
        comp_id,
        layer,
        lt: app.preview_frame as f64 / fps - layer.start_offset.0.to_f64(),
        off: layer.start_offset.0.to_f64(),
        fps,
        viewport,
        track_left,
        track_w,
        px_per_sec,
        view_start,
        graph_mode: app.timeline_graph_mode,
    };

    // Footage speed is a keyframable property too (K-072): its own row above
    // the transform, its keys building the retime's speed lens.
    if let LayerKind::Footage { retime, .. } = &layer.kind {
        speed_property_row(ui, app, &ctx, retime, pending);
    }

    if !is_camera {
        prop_row(
            ui,
            app,
            &ctx,
            "Anchor x",
            TransformProp::AnchorX,
            1.0,
            pending,
        );
        prop_row(
            ui,
            app,
            &ctx,
            "Anchor y",
            TransformProp::AnchorY,
            1.0,
            pending,
        );
    }
    prop_row(
        ui,
        app,
        &ctx,
        "Position x",
        TransformProp::PositionX,
        1.0,
        pending,
    );
    prop_row(
        ui,
        app,
        &ctx,
        "Position y",
        TransformProp::PositionY,
        1.0,
        pending,
    );

    // Scale with a ratio lock (default on). Locked: one row edits both, keeping
    // the ratio. Unlocked: two independent rows plus a relink control.
    let scale_id = ui.id().with(("scale-unlink", layer.id));
    let mut unlinked = ui.data(|d| d.get_temp::<bool>(scale_id)).unwrap_or(false);
    if unlinked {
        prop_row(
            ui,
            app,
            &ctx,
            "Scale x %",
            TransformProp::ScaleX,
            0.5,
            pending,
        );
        prop_row(
            ui,
            app,
            &ctx,
            "Scale y %",
            TransformProp::ScaleY,
            0.5,
            pending,
        );
        if link_toggle_row(ui, &ctx) {
            unlinked = false;
        }
    } else {
        combined_scale_row(ui, app, &ctx, pending, &mut unlinked);
    }
    ui.data_mut(|d| d.insert_temp(scale_id, unlinked));

    prop_row(
        ui,
        app,
        &ctx,
        "Rotation °",
        TransformProp::Rotation,
        0.5,
        pending,
    );
    prop_row(
        ui,
        app,
        &ctx,
        "Opacity %",
        TransformProp::Opacity,
        0.5,
        pending,
    );
    if three_d {
        prop_row(
            ui,
            app,
            &ctx,
            "Position z",
            TransformProp::PositionZ,
            1.0,
            pending,
        );
        prop_row(
            ui,
            app,
            &ctx,
            "Rotation x °",
            TransformProp::RotationX,
            0.5,
            pending,
        );
        prop_row(
            ui,
            app,
            &ctx,
            "Rotation y °",
            TransformProp::RotationY,
            0.5,
            pending,
        );
    }
}

/// Allocate one 18px timeline row and return (row_rect, left-column child ui).
/// The child is clipped so widgets never spill into the track area.
fn row_frame(ui: &mut egui::Ui, ctx: &RowCtx, highlight: bool) -> (egui::Rect, egui::Ui) {
    let (row_rect, _resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
    if highlight {
        // Left of the lanes → replace the clip; with_clip_rect would intersect the
        // lane clip and hide this highlight.
        let mut hp = ui.painter().clone();
        hp.set_clip_rect(ctx.viewport);
        hp.rect_filled(
            egui::Rect::from_min_max(
                row_rect.min,
                egui::pos2(ctx.track_left - 6.0, row_rect.bottom()),
            ),
            2.0,
            ctx.theme.surface_2,
        );
    }
    let left_rect = egui::Rect::from_min_max(
        egui::pos2(row_rect.left() + 24.0, row_rect.top()),
        egui::pos2(
            (ctx.track_left - 6.0).max(row_rect.left() + 25.0),
            row_rect.bottom(),
        ),
    );
    let mut c = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    // Clip to the outline column, but bounded by the scroll viewport's y so a
    // half-scrolled property row doesn't bleed past the ruler.
    c.set_clip_rect(left_rect.intersect(ctx.viewport));
    (row_rect, c)
}

/// Draw clay diamonds for `keys` on the track portion of `row_rect`.
fn draw_key_diamonds(
    ui: &egui::Ui,
    ctx: &RowCtx,
    row_rect: egui::Rect,
    keys: &[kiriko_core::anim::Keyframe],
) {
    // In graph mode the lane side belongs to the curve — no diamonds there.
    if ctx.graph_mode {
        return;
    }
    let cy = row_rect.center().y;
    // The same displayed (zoomed, scrolled) axis as the layer bars, so a
    // property's diamonds stay under its layer's keys at any zoom.
    let x_of = |s: f64| ctx.track_left + ((s - ctx.view_start) * ctx.px_per_sec) as f32;
    for k in keys {
        let x = x_of(ctx.off + k.time.to_f64());
        if x >= ctx.track_left - 1.0 && x <= ctx.track_left + ctx.track_w + 1.0 {
            let d = 3.0;
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(x, cy - d),
                    egui::pos2(x + d, cy),
                    egui::pos2(x, cy + d),
                    egui::pos2(x - d, cy),
                ],
                ctx.theme.accent,
                egui::Stroke::new(1.0_f32, ctx.theme.surface_0),
            ));
        }
    }
}

/// The stopwatch toggle. Returns the new Animation if clicked (animate at the
/// playhead / freeze to the current value), else None.
fn stopwatch(
    ui: &mut egui::Ui,
    slot: &kiriko_core::anim::Property,
    lt: f64,
) -> Option<kiriko_core::anim::Animation> {
    use kiriko_core::anim::{Animation, Keyframe, SideInterp};
    let animated = slot.is_animated();
    let clock = if animated { "⏱" } else { "◦" };
    if ui
        .selectable_label(animated, egui::RichText::new(clock).small())
        .on_hover_text(if animated {
            "Remove animation (freeze current value)"
        } else {
            "Animate: keyframe at the playhead"
        })
        .clicked()
    {
        Some(if animated {
            Animation::Static(slot.value_at(lt))
        } else {
            Animation::Keyframed(vec![Keyframe {
                time: rational_at(lt),
                value: slot.value_at(lt),
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            }])
        })
    } else {
        None
    }
}

/// AE-style keyframe navigator for an animated property, shown next to the
/// stopwatch: ◄ jumps the playhead to the previous keyframe, the diamond adds a
/// keyframe at the playhead (filled ◆ when one is already there — clicking then
/// removes it), ► jumps to the next keyframe.
fn keyframe_nav(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    prop: kiriko_core::model::TransformProp,
    slot: &kiriko_core::anim::Property,
    pending: &mut Option<kiriko_core::Op>,
) {
    use kiriko_core::anim::Animation;
    let Animation::Keyframed(keys) = &slot.animation else {
        return;
    };
    let tol = 0.5 / ctx.fps.max(1.0); // within half a frame counts as "on" it
    let small = |g: &str| egui::Button::new(egui::RichText::new(g).small()).frame(false);
    let mut jump_to: Option<f64> = None;

    let has_prev = keys.iter().any(|k| k.time.to_f64() < ctx.lt - tol);
    if ui
        .add_enabled(has_prev, small("◄"))
        .on_hover_text("Previous keyframe")
        .clicked()
    {
        jump_to = keys
            .iter()
            .rev()
            .find(|k| k.time.to_f64() < ctx.lt - tol)
            .map(|k| k.time.to_f64());
    }

    let on_key = keys.iter().any(|k| (k.time.to_f64() - ctx.lt).abs() < tol);
    if ui
        .add(small(if on_key { "◆" } else { "◇" }))
        .on_hover_text(if on_key {
            "Remove keyframe here"
        } else {
            "Add keyframe here"
        })
        .clicked()
    {
        let animation = if on_key {
            let kept: Vec<_> = keys
                .iter()
                .filter(|k| (k.time.to_f64() - ctx.lt).abs() >= tol)
                .cloned()
                .collect();
            if kept.is_empty() {
                Animation::Static(slot.value_at(ctx.lt))
            } else {
                Animation::Keyframed(kept)
            }
        } else {
            Animation::Keyframed(upsert_key(slot, ctx.lt, slot.value_at(ctx.lt)))
        };
        *pending = Some(kiriko_core::Op::SetTransformProperty {
            comp: ctx.comp_id,
            layer: ctx.layer.id,
            prop,
            animation,
        });
    }

    let has_next = keys.iter().any(|k| k.time.to_f64() > ctx.lt + tol);
    if ui
        .add_enabled(has_next, small("►"))
        .on_hover_text("Next keyframe")
        .clicked()
    {
        jump_to = keys
            .iter()
            .find(|k| k.time.to_f64() > ctx.lt + tol)
            .map(|k| k.time.to_f64());
    }

    if let Some(kt) = jump_to {
        app.preview_frame = ((kt + ctx.off) * ctx.fps).round().max(0.0) as usize;
        #[cfg(feature = "media")]
        app.refresh_preview();
    }
}

/// One generic property row.
fn prop_row(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    label: &str,
    prop: kiriko_core::model::TransformProp,
    speed: f64,
    pending: &mut Option<kiriko_core::Op>,
) {
    use kiriko_core::anim::Animation;
    let slot = ctx.layer.transform.get(prop);
    let is_graphed = app.selected_layer == Some(ctx.layer.id)
        && !app.graph_retime
        && app.graph_prop == Some(prop);
    let (row_rect, mut c) = row_frame(ui, ctx, is_graphed);

    if let Some(animation) = stopwatch(&mut c, slot, ctx.lt) {
        *pending = Some(kiriko_core::Op::SetTransformProperty {
            comp: ctx.comp_id,
            layer: ctx.layer.id,
            prop,
            animation,
        });
    }
    keyframe_nav(&mut c, app, ctx, prop, slot, pending);
    if c.add(
        egui::Label::new(egui::RichText::new(label).small().color(if is_graphed {
            ctx.theme.accent
        } else {
            ctx.theme.text_muted
        }))
        .sense(egui::Sense::click()),
    )
    .clicked()
    {
        app.selected_layer = Some(ctx.layer.id);
        app.graph_prop = Some(prop);
        app.graph_retime = false; // switching to a transform property
    }
    {
        let committed = slot.value_at(ctx.lt);
        let mut value = match app.prop_edit {
            Some((l, p, v)) if l == ctx.layer.id && p == prop => v,
            _ => committed,
        };
        let resp = c.add(
            egui::DragValue::new(&mut value)
                .speed(speed)
                .max_decimals(2),
        );
        if resp.dragged() || resp.has_focus() {
            app.prop_edit = Some((ctx.layer.id, prop, value));
        }
        if resp.drag_stopped() || resp.lost_focus() {
            if (value - committed).abs() > f64::EPSILON {
                let animation = if slot.is_animated() {
                    Animation::Keyframed(upsert_key(slot, ctx.lt, value))
                } else {
                    Animation::Static(value)
                };
                *pending = Some(kiriko_core::Op::SetTransformProperty {
                    comp: ctx.comp_id,
                    layer: ctx.layer.id,
                    prop,
                    animation,
                });
            }
            app.prop_edit = None;
        }
    }
    if let Animation::Keyframed(keys) = &slot.animation {
        draw_key_diamonds(ui, ctx, row_rect, keys);
    }
}

/// A Batch op setting both scale axes as one undo step.
fn scale_batch(
    comp: uuid::Uuid,
    layer: uuid::Uuid,
    x: kiriko_core::anim::Animation,
    y: kiriko_core::anim::Animation,
) -> kiriko_core::Op {
    use kiriko_core::model::TransformProp;
    kiriko_core::Op::Batch {
        ops: vec![
            kiriko_core::Op::SetTransformProperty {
                comp,
                layer,
                prop: TransformProp::ScaleX,
                animation: x,
            },
            kiriko_core::Op::SetTransformProperty {
                comp,
                layer,
                prop: TransformProp::ScaleY,
                animation: y,
            },
        ],
    }
}

/// The combined "Scale %" row (ratio locked): edits both axes keeping the
/// ratio, with a chain-link button to unlink. Sets `*unlinked` = true when
/// unlinked.
fn combined_scale_row(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    pending: &mut Option<kiriko_core::Op>,
    unlinked: &mut bool,
) {
    use kiriko_core::anim::Animation;
    use kiriko_core::model::TransformProp;
    let sx = ctx.layer.transform.get(TransformProp::ScaleX);
    let sy = ctx.layer.transform.get(TransformProp::ScaleY);
    let is_graphed = app.selected_layer == Some(ctx.layer.id)
        && !app.graph_retime
        && matches!(
            app.graph_prop,
            Some(TransformProp::ScaleX | TransformProp::ScaleY)
        );
    let (row_rect, mut c) = row_frame(ui, ctx, is_graphed);

    // Stopwatch drives both axes together.
    let animated = sx.is_animated() || sy.is_animated();
    let clock = if animated { "⏱" } else { "◦" };
    if c.selectable_label(animated, egui::RichText::new(clock).small())
        .on_hover_text(if animated {
            "Remove animation"
        } else {
            "Animate both scale axes"
        })
        .clicked()
    {
        let (ax, ay) = if animated {
            (
                Animation::Static(sx.value_at(ctx.lt)),
                Animation::Static(sy.value_at(ctx.lt)),
            )
        } else {
            (
                Animation::Keyframed(upsert_key(sx, ctx.lt, sx.value_at(ctx.lt))),
                Animation::Keyframed(upsert_key(sy, ctx.lt, sy.value_at(ctx.lt))),
            )
        };
        *pending = Some(scale_batch(ctx.comp_id, ctx.layer.id, ax, ay));
    }
    if c.add(
        egui::Label::new(egui::RichText::new("Scale %").small().color(if is_graphed {
            ctx.theme.accent
        } else {
            ctx.theme.text_muted
        }))
        .sense(egui::Sense::click()),
    )
    .clicked()
    {
        app.selected_layer = Some(ctx.layer.id);
        app.graph_prop = Some(TransformProp::ScaleX);
        app.graph_retime = false; // switching to a transform property
    }
    if icon_button(&mut c, ctx.theme, Icon::Link, true)
        .on_hover_text("Unlink scale (edit x and y separately)")
        .clicked()
    {
        *unlinked = true;
    }
    {
        let old_x = sx.value_at(ctx.lt);
        let old_y = sy.value_at(ctx.lt);
        let mut value = match app.prop_edit {
            Some((l, p, v)) if l == ctx.layer.id && p == TransformProp::ScaleX => v,
            _ => old_x,
        };
        let resp = c.add(egui::DragValue::new(&mut value).speed(0.5).max_decimals(2));
        if resp.dragged() || resp.has_focus() {
            app.prop_edit = Some((ctx.layer.id, TransformProp::ScaleX, value));
        }
        if resp.drag_stopped() || resp.lost_focus() {
            if (value - old_x).abs() > f64::EPSILON {
                let (nx, ny) = linked_scale(old_x, old_y, value);
                let ax = if sx.is_animated() {
                    Animation::Keyframed(upsert_key(sx, ctx.lt, nx))
                } else {
                    Animation::Static(nx)
                };
                let ay = if sy.is_animated() {
                    Animation::Keyframed(upsert_key(sy, ctx.lt, ny))
                } else {
                    Animation::Static(ny)
                };
                *pending = Some(scale_batch(ctx.comp_id, ctx.layer.id, ax, ay));
            }
            app.prop_edit = None;
        }
    }
    // Track: the union of both axes' keys.
    let mut keys: Vec<kiriko_core::anim::Keyframe> = Vec::new();
    for slot in [sx, sy] {
        if let Animation::Keyframed(k) = &slot.animation {
            keys.extend(k.iter().cloned());
        }
    }
    draw_key_diamonds(ui, ctx, row_rect, &keys);
}

/// A thin row holding the "link scale" button; true when clicked.
fn link_toggle_row(ui: &mut egui::Ui, ctx: &RowCtx) -> bool {
    let (_row_rect, mut c) = row_frame(ui, ctx, false);
    let clicked = icon_button(&mut c, ctx.theme, Icon::Link, false)
        .on_hover_text("Re-lock the x:y ratio and edit scale as one value")
        .clicked();
    c.label(
        egui::RichText::new("Link scale")
            .small()
            .color(ctx.theme.text_muted),
    );
    clicked
}

/// Insert or replace a speed keyframe at local time `lt` (seconds) with `speed`
/// (1.0 = 100%), keeping the [0, dur] endpoints, and rebuild the retime store.
fn speed_with_key(
    retime: &Option<kiriko_core::retime::Retime>,
    dur: kiriko_core::Rational,
    lt: f64,
    speed: kiriko_core::Rational,
) -> Option<kiriko_core::retime::Retime> {
    use kiriko_core::Rational;
    let mut keys = retime
        .as_ref()
        .and_then(|r| r.speed_keyframes())
        .unwrap_or_else(|| vec![(Rational::ZERO, Rational::ONE), (dur, Rational::ONE)]);
    let t = Rational::from_f64_on_grid(lt.clamp(0.0, dur.to_f64()), 1000).unwrap_or(Rational::ZERO);
    if let Some(k) = keys.iter_mut().find(|k| k.0 == t) {
        k.1 = speed;
    } else {
        keys.push((t, speed));
        keys.sort_by_key(|k| k.0);
    }
    kiriko_core::retime::Retime::from_speed_keyframes(Rational::ZERO, &keys)
}

/// The footage speed as a full-width, keyframable property row (K-072): a
/// stopwatch toggles keyframing; editing sets a constant speed or, once
/// animated, a speed keyframe at the playhead; keys show on the track. Linear
/// speed ramps read back as keyframes; smooth-eased ramps live in the graph
/// editor and here read as a constant. Clicking the name graphs the Retime
/// speed channel (K-075), like clicking a transform property's name.
fn speed_property_row(
    ui: &mut egui::Ui,
    app: &mut AppState,
    ctx: &RowCtx,
    retime: &Option<kiriko_core::retime::Retime>,
    pending: &mut Option<kiriko_core::Op>,
) {
    use kiriko_core::Rational;
    let dur = ctx.layer.out_point.0;
    let keys = retime.as_ref().and_then(|r| r.speed_keyframes());
    // Animated = an internal key, or differing endpoint speeds (a ramp).
    let animated = keys
        .as_ref()
        .is_some_and(|k| k.len() > 2 || k.first().map(|f| f.1) != k.last().map(|l| l.1));
    let current = retime
        .as_ref()
        .map(|r| r.speed_at(ctx.lt) * 100.0)
        .unwrap_or(100.0);
    let to_speed =
        |pct: f64| Rational::from_f64_on_grid(pct / 100.0, 1000).unwrap_or(Rational::ONE);

    let is_graphed = app.selected_layer == Some(ctx.layer.id) && app.graph_retime;
    let (row_rect, mut c) = row_frame(ui, ctx, is_graphed);

    let clock = if animated { "⏱" } else { "◦" };
    if c.selectable_label(animated, egui::RichText::new(clock).small())
        .on_hover_text(if animated {
            "Freeze speed (constant at the current value)"
        } else {
            "Animate speed: keyframe at the playhead"
        })
        .clicked()
    {
        let new_retime = if animated {
            if (current - 100.0).abs() < 1e-6 {
                None
            } else {
                Some(kiriko_core::retime::Retime::constant_speed(
                    dur,
                    Rational::ZERO,
                    to_speed(current),
                ))
            }
        } else {
            speed_with_key(retime, dur, ctx.lt, to_speed(current))
        };
        *pending = Some(kiriko_core::Op::SetLayerRetime {
            comp: ctx.comp_id,
            layer: ctx.layer.id,
            retime: new_retime,
        });
    }
    if c.add(
        egui::Label::new(egui::RichText::new("Speed %").small().color(if is_graphed {
            ctx.theme.accent
        } else {
            ctx.theme.text_muted
        }))
        .sense(egui::Sense::click()),
    )
    .clicked()
    {
        app.selected_layer = Some(ctx.layer.id);
        app.graph_retime = true; // graph the Retime speed channel (K-075)
        app.graph_speed_view = app.vegas_default_lens; // open to the preferred lens
    }

    // Temp-value pattern: a drag is one commit.
    let id = egui::Id::new(("speedv", ctx.layer.id));
    let mut v = c.data(|d| d.get_temp::<f64>(id)).unwrap_or(current);
    let resp = c.add(
        egui::DragValue::new(&mut v)
            .speed(1.0)
            .range(-800.0..=800.0)
            .suffix(" %"),
    );
    if resp.dragged() || resp.has_focus() {
        c.data_mut(|d| d.insert_temp(id, v));
    }
    if resp.drag_stopped() || resp.lost_focus() {
        if (v - current).abs() > 1e-6 {
            let new_retime = if animated {
                speed_with_key(retime, dur, ctx.lt, to_speed(v))
            } else if (v - 100.0).abs() < 1e-6 {
                None
            } else {
                Some(kiriko_core::retime::Retime::constant_speed(
                    dur,
                    Rational::ZERO,
                    to_speed(v),
                ))
            };
            *pending = Some(kiriko_core::Op::SetLayerRetime {
                comp: ctx.comp_id,
                layer: ctx.layer.id,
                retime: new_retime,
            });
        }
        c.data_mut(|d| d.remove::<f64>(id));
    }

    // Track: speed keyframes as diamonds (meaningful once animated).
    if animated {
        if let Some(keys) = &keys {
            let kf: Vec<kiriko_core::anim::Keyframe> = keys
                .iter()
                .map(|(t, _)| kiriko_core::anim::Keyframe {
                    time: *t,
                    value: 0.0,
                    interp_in: kiriko_core::anim::SideInterp::Linear,
                    interp_out: kiriko_core::anim::SideInterp::Linear,
                })
                .collect();
            draw_key_diamonds(ui, ctx, row_rect, &kf);
        }
    }
}

fn mask_space(
    layer: &kiriko_core::model::Layer,
    app: &AppState,
    comp: &kiriko_core::model::Composition,
) -> (f64, f64) {
    match &layer.kind {
        // An adjustment layer is comp-sized: its masks live in comp space.
        kiriko_core::model::LayerKind::Adjustment => {
            (f64::from(comp.width), f64::from(comp.height))
        }
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
    /// Panels currently detached into their own OS windows. Hidden in the dock
    /// while floating; closing the window docks them back.
    #[serde(default)]
    floating: Vec<Panel>,
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
            floating: Vec::new(),
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
                MenuAction::ExportYouTube1080 => {
                    #[cfg(feature = "media")]
                    self.start_export_preset(
                        crate::export::ExportPreset::Youtube1080,
                        "youtube-1080.mp4",
                    );
                }
                MenuAction::ExportVertical => {
                    #[cfg(feature = "media")]
                    self.start_export_preset(
                        crate::export::ExportPreset::Vertical1080,
                        "vertical.mp4",
                    );
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
        self.start_export_with(
            Some(bit_rate),
            &format!("share-{}mb.mp4", target_mb as u64),
            crate::export::ExportPreset::Native,
        );
    }

    #[cfg(feature = "media")]
    fn start_export(&mut self) {
        self.start_export_with(None, "export.mp4", crate::export::ExportPreset::Native);
    }

    #[cfg(feature = "media")]
    fn start_export_preset(&mut self, preset: crate::export::ExportPreset, default_name: &str) {
        self.start_export_with(None, default_name, preset);
    }

    #[cfg(feature = "media")]
    fn start_export_with(
        &mut self,
        bit_rate: Option<i64>,
        default_name: &str,
        preset: crate::export::ExportPreset,
    ) {
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
        let doc = self.app.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let target = preset.target(comp.width, comp.height);
        let picked = rfd::FileDialog::new()
            .add_filter("MP4 video", &["mp4"])
            .set_file_name(default_name)
            .save_file();
        let Some(path) = picked else { return };
        let items = crate::export::item_infos(&doc, &self.app.media);
        self.export = Some(crate::export::start(
            doc,
            comp_id,
            items,
            gpu.export_context(),
            path,
            bit_rate,
            target,
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
                            // A direct value drag gives (layer, prop, value)
                            // outright; a graph keyframe drag gives the property's
                            // provisional value at the playhead instead.
                            let live = self.app.prop_edit.or_else(|| {
                                let (idx, kt, kv) = self.app.graph_edit?;
                                let prop = self.app.graph_prop?;
                                let layer_id = self.app.selected_layer?;
                                let layer = comp.layers.iter().find(|l| l.id == layer_id)?;
                                let kiriko_core::anim::Animation::Keyframed(keys) =
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
                                let value = kiriko_core::anim::evaluate(&keys, lt)?;
                                Some((layer_id, prop, value))
                            });
                            if let Some((edit_layer, prop, value)) = live {
                                let patched = patch_layer_prop(comp, edit_layer, prop, value);
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
                        self.start_export();
                        ui.close_menu();
                    }
                    #[cfg(feature = "media")]
                    ui.menu_button("Export preset", |ui| {
                        if ui.button("YouTube 1080p").clicked() {
                            self.start_export_preset(
                                crate::export::ExportPreset::Youtube1080,
                                "youtube-1080.mp4",
                            );
                            ui.close_menu();
                        }
                        if ui.button("YouTube 4K").clicked() {
                            self.start_export_preset(
                                crate::export::ExportPreset::Youtube2160,
                                "youtube-4k.mp4",
                            );
                            ui.close_menu();
                        }
                        if ui.button("Vertical 1080×1920").clicked() {
                            self.start_export_preset(
                                crate::export::ExportPreset::Vertical1080,
                                "vertical.mp4",
                            );
                            ui.close_menu();
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
                ui.spacing_mut().item_spacing.x = 2.0;
                let tool = self.app.tool;
                if icon_button(ui, &self.theme, Icon::Pointer, tool == ToolMode::Select)
                    .on_hover_text("Select / move the view (V)")
                    .clicked()
                {
                    self.app.tool = ToolMode::Select;
                }
                if icon_button(ui, &self.theme, Icon::Move, tool == ToolMode::Hand)
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
                let shape_resp = icon_button(ui, &self.theme, shape_icon, tool == ToolMode::Shape)
                    .on_hover_text(format!(
                        "Draw a {} mask — right-click to pick a shape (Q)",
                        self.app.shape_kind.label().to_lowercase()
                    ));
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
                if icon_button(ui, &self.theme, Icon::Pen, tool == ToolMode::Pen)
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
            floating,
            theme,
            app,
            preview_display,
            ..
        } = self;
        let preview_display = *preview_display;
        let pop_out = {
            let mut behavior = DockBehavior {
                theme,
                app,
                preview_display,
                pop_out: None,
            };
            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(theme.surface_0))
                .show(ctx, |ui| dock.ui(&mut behavior, ui));
            behavior.pop_out
        };

        // Apply a pop-out request: hide the panel in the dock, float it.
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
        for &panel in floating.iter() {
            let vid = egui::ViewportId::from_hash_of(("kiriko-float", panel.title()));
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("Kiriko — {}", panel.title()))
                .with_inner_size([640.0, 420.0]);
            ctx.show_viewport_immediate(vid, builder, |ctx, _class| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default().fill(theme.surface_0))
                    .show(ctx, |ui| {
                        render_panel(ui, theme, app, preview_display, panel)
                    });
                if ctx.input(|i| i.viewport().close_requested()) {
                    dock_back.push(panel);
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
            Panel::Scopes,
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

    // The Timeline starts as a full-width strip along the bottom: its tile is a
    // direct child of the vertical root (so it is as wide as the window) and the
    // last child (the bottom band). Guards the default workspace against a
    // regression back to the Timeline nested inside the Viewer's column.
    #[test]
    fn timeline_starts_full_width_along_the_bottom() {
        let tree = default_layout();
        let root = tree.root().expect("layout has a root");
        let egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin)) =
            tree.tiles.get(root).expect("root tile exists")
        else {
            panic!("the root should be a vertical linear container");
        };
        assert_eq!(lin.dir, egui_tiles::LinearDir::Vertical);

        let timeline = tile_id_of(&tree, Panel::Timeline).expect("timeline present");
        let timeline_band = tree
            .tiles
            .iter()
            .find_map(|(id, tile)| match tile {
                egui_tiles::Tile::Container(c) if c.children().any(|ch| *ch == timeline) => {
                    Some(*id)
                }
                _ => None,
            })
            .expect("timeline sits in a container");

        assert!(
            lin.children.contains(&timeline_band),
            "timeline band should be a direct child of the vertical root (full width)"
        );
        assert_eq!(
            lin.children.last(),
            Some(&timeline_band),
            "timeline band should be the bottom-most child"
        );
    }

    // Each keyframe's glyph codes its interpolation (graph-editor ergonomics).
    #[test]
    fn key_shape_codes_interpolation() {
        use kiriko_core::anim::{Keyframe, SideInterp};
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

    // The linked scale control keeps the x:y ratio (K-072).
    #[test]
    fn linked_scale_keeps_ratio() {
        assert_eq!(linked_scale(100.0, 50.0, 200.0), (200.0, 100.0)); // 2:1 kept
        assert_eq!(linked_scale(100.0, 100.0, 150.0), (150.0, 150.0)); // 1:1 kept
        assert_eq!(linked_scale(0.0, 50.0, 80.0), (80.0, 80.0)); // undefined → uniform
    }

    // A keyframe side reports its bezier influence, or the easy-ease third.
    #[test]
    fn side_influence_reads_bezier_or_defaults() {
        use kiriko_core::anim::SideInterp;
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

    // K-070: setting a key's speed (what a speed-lens drag commits — both sides
    // to Bezier{speed}) is what the derivative reads back. Guards the lossless
    // round-trip promised for the speed lens.
    #[test]
    fn setting_key_speed_round_trips_through_the_derivative() {
        use kiriko_core::anim::{evaluate, Keyframe, SideInterp};
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

    // K-075 2b: dragging a speed keyframe in the % lens (via speed_with_key)
    // authors a ramp — the speed set is the speed read back, and the segment
    // start is pinned (K-070 frame-pinning: only downstream recomputes).
    #[test]
    fn retime_speed_keyframe_edit_round_trips() {
        use kiriko_core::retime::Retime;
        use kiriko_core::Rational;
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
        use kiriko_core::anim::Animation;
        use kiriko_core::model::TransformProp;
        let comp = uuid::Uuid::from_u128(0xC0);
        let layer = uuid::Uuid::from_u128(0x1A);
        let mut app = AppState::default();

        // A Retime edit selects the layer and graphs the Speed channel.
        follow_edit(
            &mut app,
            &kiriko_core::Op::SetLayerRetime {
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
            &kiriko_core::Op::SetTransformProperty {
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
            &kiriko_core::Op::Batch {
                ops: vec![
                    kiriko_core::Op::SetTransformProperty {
                        comp,
                        layer,
                        prop: TransformProp::ScaleX,
                        animation: Animation::Static(100.0),
                    },
                    kiriko_core::Op::SetTransformProperty {
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
            &kiriko_core::Op::RenameLayer {
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
        use kiriko_core::model::TransformProp as P;
        assert_eq!(prop_unit(P::Opacity), "%");
        assert_eq!(prop_unit(P::ScaleX), "%");
        assert_eq!(prop_unit(P::Rotation), "°");
        assert_eq!(prop_unit(P::PositionX), "");
    }

    // Moving a layer shifts in/out AND start_offset by the same delta — a move,
    // not a slip: duration and the in→start_offset alignment are preserved.
    #[test]
    fn moving_a_layer_shifts_the_whole_span_not_slips_it() {
        use kiriko_core::time::CompTime;
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
