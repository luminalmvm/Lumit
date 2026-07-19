//! `shell::panels` — split out of the monolithic shell.rs (mechanical,
//! no logic changes). Shared names resolve through the parent module
//! via `use super::*` and the glob re-exports in `shell/mod.rs`.

use super::*;

/// The Viewer: neutral surround + the empty-project card (docs/07-UI-SPEC.md §13.2).
#[cfg_attr(not(feature = "media"), allow(unused_variables))]
pub(crate) fn viewer_panel(
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
                            egui::RichText::new("Lumit")
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
pub(crate) enum PanelAction {
    Select(uuid::Uuid),
    /// Ctrl/Shift-click: toggle the item in the Project panel's multi-selection
    /// (A3), so several items can drag into a comp at once.
    ToggleSelect(uuid::Uuid),
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
pub(crate) fn project_panel(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    thumb: Option<(egui::TextureId, egui::Vec2)>,
) {
    let doc = app.store.snapshot();
    let mut actions: Vec<PanelAction> = Vec::new();

    // A live search across the top of the panel (UI-3, spec §3.1): filters the
    // tree by name, case-insensitive substring. Empty shows everything. Kept in
    // per-panel ui memory, like the Effects & Presets browser's field.
    let search_id = ui.id().with("project-panel-search");
    let mut query = ui
        .data_mut(|d| d.get_temp::<String>(search_id))
        .unwrap_or_default();
    ui.add_space(4.0);
    let resp = ui.add(
        egui::TextEdit::singleline(&mut query)
            .hint_text("Search project")
            .desired_width(f32::INFINITY),
    );
    if resp.changed() {
        ui.data_mut(|d| d.insert_temp(search_id, query.clone()));
    }
    let needle = query.trim().to_lowercase();

    project_header(ui, theme, app, &doc, &mut actions, thumb);
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
        if let Some(payload) = dnd_release_of::<uuid::Uuid>(&new_comp) {
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
                item_rows(
                    ui,
                    theme,
                    app,
                    &doc,
                    id,
                    0,
                    &mut actions,
                    &mut visited,
                    &needle,
                );
            }
            // A search that hides everything gets a calm note rather than a bare
            // panel, so it never looks broken (UI-3).
            if !needle.is_empty() {
                let mut v = Vec::new();
                let any = doc
                    .root_items()
                    .into_iter()
                    .any(|id| subtree_matches(&doc, id, &needle, &mut v));
                if !any {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("No items match your search")
                            .small()
                            .color(theme.text_muted),
                    );
                }
            }
            // Trailing space so there is always a root drop area.
            ui.allocate_space(egui::vec2(ui.available_width(), 40.0));
        });
    if bg.double_clicked() {
        app.import_footage_dialog();
    }
    if let Some(payload) = dnd_release_of::<uuid::Uuid>(&bg) {
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
                app.select_project_item(id);
            }
            PanelAction::ToggleSelect(id) => app.toggle_project_item(id),
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
                app.commit(lumit_core::Op::RemoveItem { id });
                // A deleted comp also loses its Timeline tab (neighbour takes
                // over, or the Timeline empties).
                app.close_comp_tab(id);
                if app.selected_item == Some(id) {
                    app.selected_item = None;
                }
                app.selected_items.retain(|x| *x != id);
            }
        }
    }
}

/// Fixed height of the selected-item info box, so choosing different items never
/// shifts the tree beneath it (UI-4). Sized to hold the footage thumbnail plus
/// its two text lines with a little room to breathe.
const PROJECT_HEADER_HEIGHT: f32 = 58.0;

/// The info header: what the selected item is, at a glance (AE's preview area).
/// The box keeps a constant height whatever is selected (UI-4), and for footage
/// it shows a small thumbnail on the left — the Viewer's own decoded frame,
/// reused, never a fresh decode. `thumb` is that Viewer texture, if any.
pub(crate) fn project_header(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &AppState,
    doc: &lumit_core::model::Document,
    actions: &mut Vec<PanelAction>,
    thumb: Option<(egui::TextureId, egui::Vec2)>,
) {
    // Reserve a constant-height box regardless of what is (or isn't) selected,
    // and draw into it under a clip so nothing ever spills into the tree below
    // (the info placement staying put between selections — UI-4).
    let full = ui.available_rect_before_wrap();
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(full.width(), PROJECT_HEADER_HEIGHT),
        egui::Sense::hover(),
    );
    let content = rect.shrink2(egui::vec2(2.0, 5.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(content);
    let ui = &mut child;

    let Some(item) = app.selected_item.and_then(|id| doc.item(id)) else {
        ui.label(
            egui::RichText::new("Nothing selected")
                .small()
                .color(theme.text_disabled),
        );
        return;
    };

    // A footage thumbnail sits to the left of the readout, but only when the
    // Viewer's texture really is this item's picture (not a comp's or another
    // clip's); otherwise a neutral placeholder stands in. Reuses whatever the
    // app already decoded — it never starts a decode of its own (UI-4).
    let is_footage = matches!(item, ProjectItem::Footage(_));
    let footage_tex =
        if is_footage && app.preview_comp.is_none() && app.preview_item == Some(item.id()) {
            thumb
        } else {
            None
        };

    ui.horizontal(|ui| {
        if is_footage {
            footage_thumbnail(ui, theme, footage_tex);
            ui.add_space(8.0);
        }
        ui.vertical(|ui| {
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
                                    line.push_str(&format!(
                                        "{} Hz · {} ch",
                                        a.sample_rate, a.channels
                                    ));
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
                                        .on_hover_text(
                                            "Variable frame rate: conformed to the median rate",
                                        );
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
        });
    });
}

/// The footage thumbnail box in the info header (UI-4): a decoded frame shown
/// aspect-fitted, or — with no frame to hand — a neutral placeholder carrying
/// the footage glyph. Reuses the Viewer's preview texture; never decodes.
fn footage_thumbnail(ui: &mut egui::Ui, theme: &Theme, tex: Option<(egui::TextureId, egui::Vec2)>) {
    let (box_rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 48.0), egui::Sense::hover());
    let radius = egui::CornerRadius::same(theme.tokens.control_radius);
    ui.painter().rect_filled(box_rect, radius, theme.surface_3);
    match tex.filter(|(_, s)| s.x > 0.0 && s.y > 0.0) {
        Some((id, size)) => {
            // Contain the frame within the box, preserving its aspect ratio.
            let scale = (box_rect.width() / size.x).min(box_rect.height() / size.y);
            let fit = egui::Rect::from_center_size(
                box_rect.center(),
                egui::vec2(size.x * scale, size.y * scale),
            );
            ui.painter().image(
                id,
                fit,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            crate::icons::paint(
                ui.painter(),
                box_rect.shrink(16.0),
                Icon::Footage,
                theme.text_disabled,
                1.4,
            );
        }
    }
    ui.painter().rect_stroke(
        box_rect,
        radius,
        egui::Stroke::new(1.0_f32, theme.hairline),
        egui::StrokeKind::Inside,
    );
}

/// A row that is one widget yet both a click target and a drag source. egui's
/// `dnd_drag_source` lays a drag-only overlay over its contents, and that
/// overlay swallows plain clicks — Project-panel rows looked dead (you could not
/// open a comp or preview footage by clicking). A single `Button` that senses
/// click *and* drag keeps both; while dragged it registers `payload` so every
/// existing drop target (folders, Timeline, Viewer, the "+ Composition" button)
/// keeps working unchanged. `dnd_drag_source`'s ghost-under-cursor is dropped;
/// the drop targets' own hover highlight stands in for it.
pub(crate) fn draggable_row<P: std::any::Any + Send + Sync>(
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

/// Read a drop release of type `P` without being able to destroy another
/// type's drag. egui keeps ONE drag payload for the whole app, and
/// `Response::dnd_release_payload` *takes* that payload out of the context
/// before it checks the type — so a `uuid::Uuid` zone that merely contains
/// the release point swallows an `EffectDragPayload` drag whole, and every
/// zone reading after it sees nothing. That is exactly how the Timeline's
/// full-body item zone (registered before the layer rows) ate each effect
/// drop: the effect landed on a row, the item zone took-and-discarded it
/// first, and the row's own reader found the slot empty. Gating on the
/// payload's type first means only a matching drop is ever taken; a
/// mismatched drag sails past untouched to whichever zone it belongs to.
pub(crate) fn dnd_release_of<P: std::any::Any + Send + Sync>(
    resp: &egui::Response,
) -> Option<std::sync::Arc<P>> {
    if egui::DragAndDrop::has_payload_of_type::<P>(&resp.ctx) {
        resp.dnd_release_payload::<P>()
    } else {
        None
    }
}

/// A compact icon button in the house toolbar style (docs/15-DESIGN.md §5): a
/// stroke glyph in `text_secondary`, brightening to `text_primary` on hover and
/// `accent` when `active`, over a faint surface chip. Returns the response so
/// the caller reads `.clicked()` and attaches a tooltip with `.on_hover_text`.
pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    theme: &Theme,
    icon: Icon,
    active: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 26.0), egui::Sense::click());
    let hovered = resp.hovered();
    // The chip radius follows the theme (K-092) rather than a hardcoded 4px,
    // so icon buttons round with everything else under the Round shape.
    let radius = theme.tokens.control_radius;
    if active || hovered {
        ui.painter().rect_filled(
            rect.shrink(1.0),
            radius,
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
            radius,
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
    // Paint the glyph into an inset rect so it keeps breathing room from the
    // chip edge instead of filling it corner-to-corner.
    crate::icons::paint(ui.painter(), rect.shrink(6.0), icon, color, 1.5);
    resp
}

/// The identity glyph and §6.1 colour for a layer type. Not drawn in the
/// outline (Mack): the type reads from the lane bar itself — its tonal wash
/// and the 3px tab on the bar's left edge.
pub(crate) fn layer_type_style(
    kind: &lumit_core::model::LayerKind,
    theme: &Theme,
) -> (Icon, egui::Color32) {
    use lumit_core::model::LayerKind;
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

/// Whether `id`'s subtree carries a name matching `needle` (already lowercased
/// and non-empty). A folder matches when its own name matches or any descendant
/// does, so the path down to a hit stays visible under the search (UI-3). Cheap:
/// the project tree is small, and `visited` guards malformed cycles.
pub(crate) fn subtree_matches(
    doc: &lumit_core::model::Document,
    id: uuid::Uuid,
    needle: &str,
    visited: &mut Vec<uuid::Uuid>,
) -> bool {
    if visited.contains(&id) {
        return false;
    }
    let Some(item) = doc.item(id) else {
        return false;
    };
    if item.name().to_lowercase().contains(needle) {
        return true;
    }
    if let Some(f) = doc.folder(id) {
        visited.push(id);
        let hit = f
            .children
            .clone()
            .into_iter()
            .any(|c| subtree_matches(doc, c, needle, visited));
        visited.pop();
        return hit;
    }
    false
}

/// One tree row (folders recurse). Rows are drag sources; folder rows are
/// drop targets. `needle` is the active search (UI-3): empty shows everything,
/// otherwise only subtrees with a name match are drawn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn item_rows(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &AppState,
    doc: &lumit_core::model::Document,
    id: uuid::Uuid,
    depth: usize,
    actions: &mut Vec<PanelAction>,
    visited: &mut Vec<uuid::Uuid>,
    needle: &str,
) {
    if visited.contains(&id) {
        return; // defensive: malformed folder cycles never hang the panel
    }
    let Some(item) = doc.item(id) else {
        return; // stale child id (deleted item): just don't draw it
    };
    // Search filter (UI-3): a non-matching subtree is skipped entirely.
    let searching = !needle.is_empty();
    let name_match = item.name().to_lowercase().contains(needle);
    if searching {
        let mut v = Vec::new();
        if !subtree_matches(doc, id, needle, &mut v) {
            return;
        }
    }
    let is_folder = matches!(item, ProjectItem::Folder(_));
    // Type glyph + tint carried on the left of the row (replaces the old text
    // tag): comps take the accent, the rest a muted tint (docs/15-DESIGN.md §5).
    let (type_icon, tag_colour) = match item {
        ProjectItem::Footage(_) => (Icon::Footage, theme.text_muted),
        ProjectItem::Folder(_) => (Icon::Folder, theme.text_muted),
        ProjectItem::Composition(_) => (Icon::Comp, theme.accent),
        ProjectItem::Solid(_) => (Icon::Solid, theme.text_muted),
    };
    let selected = app.is_item_selected(id);
    let open_id = ui.id().with(("folder-open", id));
    // While searching, folders force open so matches beneath them are visible;
    // the stored open state is left untouched for when the search clears (UI-3).
    let mut open =
        is_folder && (searching || ui.data(|d| d.get_temp::<bool>(open_id).unwrap_or(true)));

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
        // Ctrl/Cmd or Shift extends the multi-selection (A3); a plain click
        // selects just this item. A multi-select doesn't scrub the previewer —
        // the user is gathering items to drag, not browsing.
        let mods = ui.input(|i| i.modifiers);
        if mods.command || mods.shift {
            actions.push(PanelAction::ToggleSelect(id));
        } else {
            actions.push(PanelAction::Select(id));
            if let ProjectItem::Footage(_) = item {
                actions.push(PanelAction::PreviewFootage(id));
            }
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
        if let Some(payload) = dnd_release_of::<uuid::Uuid>(&row) {
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
                // A folder whose own name matched reveals its whole contents;
                // otherwise keep filtering, so only the path to a hit shows.
                let child_needle = if name_match { "" } else { needle };
                for child in f.children.clone() {
                    item_rows(
                        ui,
                        theme,
                        app,
                        doc,
                        child,
                        depth + 1,
                        actions,
                        visited,
                        child_needle,
                    );
                }
                visited.pop();
            }
        }
    }
}

/// Accept a Project-panel item dropped on this panel's area: file it into
/// the active comp as a layer, or — with no comp yet — open the composition
/// dialogue pre-filled from the footage (K-068).
pub(crate) fn accept_item_drop(ui: &egui::Ui, theme: &Theme, app: &mut AppState, rect: egui::Rect) {
    let zone = ui.interact(rect, ui.id().with("item-drop-zone"), egui::Sense::hover());
    if zone.dnd_hover_payload::<uuid::Uuid>().is_some() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            2.0,
            egui::Stroke::new(1.0_f32, theme.accent),
            egui::StrokeKind::Inside,
        );
    }
    // Guarded read: this zone spans a whole panel body, so an unguarded
    // `dnd_release_payload::<uuid::Uuid>` here would take-and-discard an
    // effect drag released over it (the slot is shared; see `dnd_release_of`).
    let Some(payload) = dnd_release_of::<uuid::Uuid>(&zone) else {
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
pub(crate) fn comp_tab_strip(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
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
            // The strip's background is a click-sensing Ui registered before
            // the tab buttons, so the buttons keep their own clicks and only
            // genuinely empty strip space answers to the background — the
            // Project panel's backdrop layering. Its context menu replaces
            // the dock tab's pop-out button, gone now that a solo Timeline
            // renders bare (K-086).
            let bg = ui.scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (id, name) in &tabs {
                        let is_active = active == Some(*id);
                        // Name and close (×) share one rounded pill (owner
                        // request): the Frame is the pill; the label inside
                        // activates the tab, the × inside closes it.
                        let pill = egui::Frame::new()
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
                            ))
                            .corner_radius(theme.tokens.control_radius)
                            .inner_margin(egui::Margin::symmetric(7, 2))
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                let name_resp = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(trim_title(name)).small().color(
                                            if is_active {
                                                theme.text_primary
                                            } else {
                                                theme.text_secondary
                                            },
                                        ),
                                    )
                                    .sense(egui::Sense::click())
                                    .selectable(false),
                                );
                                let close_resp = ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("×")
                                                .small()
                                                .color(theme.text_muted),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text("Close this comp tab");
                                (name_resp, close_resp)
                            });
                        let (name_resp, close_resp) = pill.inner;
                        // Re-clicking the active tab is a no-op (don't reset its
                        // playhead); only switching tabs re-activates.
                        if name_resp.clicked() && !is_active {
                            activate = Some(*id);
                        }
                        if close_resp.clicked() {
                            close = Some(*id);
                        }
                    }
                });
                // Claim the empty width right of the tabs, so a
                // right-click there lands on the strip's background.
                let claim = egui::Rect::from_min_max(
                    ui.min_rect().left_top(),
                    egui::pos2(ui.max_rect().right(), ui.min_rect().bottom()),
                );
                ui.expand_to_include_rect(claim);
            });
            bg.response
        });
    // Right-clicking an empty spot on the strip (not a tab) offers the
    // Timeline's pop-out, since a solo Timeline has no dock tab to host the
    // button (K-086). The strip renders deep inside the panel, so the request
    // travels to the shell through AppState.
    strip.inner.context_menu(|ui| {
        if ui.button("Pop out timeline").clicked() {
            app.pop_out_timeline = true;
            ui.close_menu();
        }
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
    if let Some(payload) = dnd_release_of::<uuid::Uuid>(&strip_drop) {
        let dropped = *payload;
        if app.store.snapshot().comp(dropped).is_some() {
            app.open_comp(dropped); // open beside the tabs as a separate tab
        } else {
            // Dragging one of a multi-selection brings the whole set in at once
            // (A3); dragging an unselected item brings just it.
            let sel = app.project_selection();
            if sel.len() > 1 && sel.contains(&dropped) {
                app.add_items_to_comp(&sel);
            } else {
                app.add_item_to_comp(dropped); // footage/solid → into the active comp
            }
        }
    }
}

/// A drag payload carrying a built-in effect's stable `match_name` (K-101):
/// dragging an entry out of the Effects & Presets browser and releasing it
/// over a Timeline layer row applies that effect there. A distinct type from
/// the Project panel's `uuid::Uuid` item payload, so a drop target can tell
/// "an effect" and "a project item" apart from the payload's type alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct EffectDragPayload(pub &'static str);

/// The effects browser (docs/07-UI-SPEC §7): a user-preset library (K-129) over
/// the built-in catalogue (docs/08-EFFECTS.md), both grouped and filtered by the
/// search field. Effect entries mirror the Add-effect menu's grouping
/// (`effects_rows`) and are drag sources (K-101): drag one onto a footage or
/// adjustment layer row in the Timeline to apply it. Preset entries apply on a
/// click, appending their whole saved stack to the selected layer.
pub(crate) fn effects_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    use lumit_core::fx;
    ui.add_space(6.0);
    let search_id = ui.id().with("effects-panel-search");
    let mut search = ui
        .data_mut(|d| d.get_temp::<String>(search_id))
        .unwrap_or_default();
    let resp = ui.add(
        egui::TextEdit::singleline(&mut search)
            .hint_text("Search effects and presets")
            .desired_width(f32::INFINITY),
    );
    if resp.changed() {
        ui.data_mut(|d| d.insert_temp(search_id, search.clone()));
    }
    ui.add_space(8.0);
    let needle = search.trim().to_lowercase();

    // The preset library, scanned fresh each paint so a just-saved preset shows
    // straight away. A missing directory or read error yields an empty list —
    // the section then shows a hint, never a failure.
    let presets: Vec<crate::preset::PresetEntry> = lumit_project::presets_dir()
        .as_deref()
        .map(crate::preset::list_presets)
        .unwrap_or_default();
    let shown_presets: Vec<&crate::preset::PresetEntry> = presets
        .iter()
        .filter(|p| needle.is_empty() || p.name.to_lowercase().contains(&needle))
        .collect();
    // Applying needs a selected layer; read it now so the scroll closure never
    // borrows `app`, and the chosen preset is applied after drawing.
    let has_layer = app.selected_comp.is_some() && app.selected_layer.is_some();
    let mut apply: Option<std::path::PathBuf> = None;

    let mut any_effect = false;
    egui::ScrollArea::vertical()
        // Fill the panel's full width so the scrollbar hugs the far-right edge
        // rather than shrinking to the content and sitting mid-panel, which
        // clipped effect/preset names (owner-reported). Matches the Project
        // panel's tree scroll.
        .auto_shrink([false, false])
        .id_salt("effects-panel-scroll")
        .show(ui, |ui| {
            // Presets first — the user's own looks sit above the built-ins.
            egui::CollapsingHeader::new(
                egui::RichText::new("Presets")
                    .small()
                    .color(theme.text_secondary),
            )
            .default_open(true)
            .show(ui, |ui| {
                if shown_presets.is_empty() {
                    let hint = if presets.is_empty() {
                        "No presets yet. Save a layer's effect stack as a preset \
                         (Effect Controls → Presets) to build your library."
                    } else {
                        "No presets match your search."
                    };
                    ui.label(egui::RichText::new(hint).small().color(theme.text_muted));
                } else {
                    for entry in &shown_presets {
                        // A plain click applies the preset to the selected layer.
                        // Frameless so it reads like the effect rows beside it.
                        let row = ui.add(
                            egui::Button::new(
                                egui::RichText::new(&entry.name)
                                    .small()
                                    .color(theme.text_primary),
                            )
                            .frame(false),
                        );
                        let row = if has_layer {
                            row.on_hover_text("Apply this preset to the selected layer")
                        } else {
                            row.on_hover_text("Select a layer, then click to apply this preset")
                        };
                        if row.clicked() {
                            apply = Some(entry.path.clone());
                        }
                    }
                }
            });

            for cat in fx::FxCategory::ALL {
                let members: Vec<_> = fx::BUILTINS
                    .iter()
                    .filter(|s| {
                        s.category == cat && fuzzy_score(&needle, s.label, s.match_name).is_some()
                    })
                    .collect();
                if members.is_empty() {
                    continue;
                }
                any_effect = true;
                egui::CollapsingHeader::new(
                    egui::RichText::new(cat.label())
                        .small()
                        .color(theme.text_secondary),
                )
                .default_open(true)
                .show(ui, |ui| {
                    for schema in members {
                        // A drag source carrying the effect's match_name
                        // (K-101); `draggable_row` is the same click-and-drag
                        // row used for footage/comp items above, so a plain
                        // click still does nothing here rather than looking
                        // dead under a drag-only overlay.
                        let id = ui.id().with(("fx-browser-drag", schema.match_name));
                        draggable_row(
                            ui,
                            id,
                            EffectDragPayload(schema.match_name),
                            false,
                            egui::RichText::new(schema.label)
                                .small()
                                .color(theme.text_primary),
                        );
                    }
                });
            }
            if !any_effect && !needle.is_empty() {
                ui.label(
                    egui::RichText::new("No effects match.")
                        .small()
                        .color(theme.text_muted),
                );
            }
        });

    if let Some(path) = apply {
        apply_preset_to_selected_layer(app, &path);
    }

    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(
            "Drag an effect onto a layer in the Timeline, or click a preset to append its stack.",
        )
        .small()
        .color(theme.text_muted),
    );
}

/// Append a preset's saved stack (fresh ids) to the selected layer — one
/// undoable `SetLayerEffects`, the same append the Effect Controls "Load
/// preset…" commits. A read error or no selected layer surfaces as a status
/// hint and leaves the document untouched (applying a preset is never a
/// half-done edit).
fn apply_preset_to_selected_layer(app: &mut AppState, path: &std::path::Path) {
    let Some(added) = crate::preset::load_instantiated(path) else {
        app.error = Some("that preset could not be read".into());
        return;
    };
    let (Some(comp_id), Some(layer_id)) = (app.selected_comp, app.selected_layer) else {
        app.error = Some("select a layer to apply a preset".into());
        return;
    };
    let doc = app.store.snapshot();
    let Some(layer) = doc
        .comp(comp_id)
        .and_then(|c| c.layers.iter().find(|l| l.id == layer_id))
    else {
        app.error = Some("select a layer to apply a preset".into());
        return;
    };
    let mut effects = layer.effects.clone();
    effects.extend(added);
    app.commit(lumit_core::Op::SetLayerEffects {
        comp: comp_id,
        layer: layer_id,
        effects,
    });
    #[cfg(feature = "media")]
    app.refresh_preview();
}

pub(crate) fn empty_hint(ui: &mut egui::Ui, theme: &Theme, title: &str, hint: &str) {
    ui.add_space(10.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(title).color(theme.text_secondary));
        ui.add_space(2.0);
        ui.label(egui::RichText::new(hint).small().color(theme.text_muted));
    });
}

/// The Effect Controls panel (docs/07-UI-SPEC §6): the selected layer's effect
/// stack, editable, in its own dock panel. Reuses the Timeline's `effects_rows`
/// through a panel-mode `RowCtx` — the whole panel width is the control column
/// (`track_left` at the right edge) and `graph_mode` suppresses the lane-side
/// keyframe painting, since a panel has no time lane. The same rows, the same
/// ops, the same undo — just a roomier home than the Timeline row.
pub(crate) fn effect_controls_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    let doc = app.store.snapshot();
    let (Some(comp_id), Some(layer_id)) = (app.selected_comp, app.selected_layer) else {
        empty_hint(
            ui,
            theme,
            "Effect controls",
            "Select a layer to see and edit its effect stack.",
        );
        return;
    };
    let Some(layer) = doc.comp(comp_id).and_then(|c| {
        c.layers
            .iter()
            .find(|l| l.id == layer_id)
            .map(|l| (c.frame_rate.fps().max(1.0), l))
    }) else {
        empty_hint(
            ui,
            theme,
            "Effect controls",
            "Select a layer to see and edit its effect stack.",
        );
        return;
    };
    let (fps, layer) = layer;

    // Effect drop (K-101): dropping an effect dragged from the Effects & Presets
    // browser anywhere in this panel appends it to the shown layer — the same
    // SetLayerEffects the Timeline row drop commits. The zone exists only while a
    // drag is live, so it steals no ordinary input; `contains_pointer` ignores
    // the widgets drawn over it.
    if egui::DragAndDrop::has_payload_of_type::<EffectDragPayload>(ui.ctx()) {
        let rect = ui.max_rect();
        let drop = ui.interact(
            rect,
            ui.id().with(("fx-controls-drop", layer_id)),
            egui::Sense::hover(),
        );
        if let Some(payload) = dnd_release_of::<EffectDragPayload>(&drop) {
            if let Some(inst) = lumit_core::fx::instantiate(payload.0) {
                let mut effects = layer.effects.clone();
                effects.push(inst);
                app.commit(lumit_core::Op::SetLayerEffects {
                    comp: comp_id,
                    layer: layer_id,
                    effects,
                });
                #[cfg(feature = "media")]
                app.refresh_preview();
            }
        } else if drop.dnd_hover_payload::<EffectDragPayload>().is_some() {
            ui.painter().rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0_f32, theme.accent),
                egui::StrokeKind::Inside,
            );
        }
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(trim_title(&layer.name)).color(theme.text_primary));
    });
    ui.add_space(2.0);
    ui.separator();

    let mut pending: Option<lumit_core::Op> = None;

    // Parent (K-103): follow another layer's transform. The dropdown lists the
    // comp's other layers, hiding any that would form a cycle, plus None to
    // clear. Committing a `SetLayerParent` is one ordinary undo step.
    if let Some(comp) = doc.comp(comp_id) {
        let current = layer
            .parent
            .and_then(|pid| comp.layers.iter().find(|l| l.id == pid))
            .map(|l| trim_title(&l.name))
            .unwrap_or_else(|| "None".to_string());
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Parent")
                    .small()
                    .color(theme.text_muted),
            );
            bare_dropdown(ui, egui::RichText::new(current).small(), |ui| {
                if ui
                    .selectable_label(layer.parent.is_none(), "None")
                    .clicked()
                {
                    if layer.parent.is_some() {
                        pending = Some(lumit_core::Op::SetLayerParent {
                            comp: comp_id,
                            layer: layer_id,
                            parent: None,
                        });
                    }
                    ui.close_menu();
                }
                for cand in &comp.layers {
                    if cand.id == layer_id
                        || lumit_core::model::parenting_would_cycle(comp, layer_id, cand.id)
                    {
                        continue;
                    }
                    let sel = layer.parent == Some(cand.id);
                    if ui.selectable_label(sel, trim_title(&cand.name)).clicked() {
                        pending = Some(lumit_core::Op::SetLayerParent {
                            comp: comp_id,
                            layer: layer_id,
                            parent: Some(cand.id),
                        });
                        ui.close_menu();
                    }
                }
            });
        });
        ui.add_space(2.0);
    }

    // Solo / isolate (K-105): while on, only soloed layers render.
    let mut solo = layer.switches.solo;
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Solo").small().color(theme.text_muted));
        if ui
            .checkbox(&mut solo, "")
            .on_hover_text("Isolate: while any layer is soloed, only soloed layers render")
            .changed()
        {
            pending = Some(lumit_core::Op::SetLayerSolo {
                comp: comp_id,
                layer: layer_id,
                solo,
            });
        }
    });
    ui.add_space(2.0);

    let panel = ui.max_rect();
    // The comp, for a Layer effect parameter's picker (K-123). Present here (the
    // layer above was derived from it), handled safely regardless.
    let Some(comp) = doc.comp(comp_id) else {
        return;
    };
    let ctx = RowCtx {
        theme,
        comp_id,
        comp,
        layer,
        lt: app.preview_frame as f64 / fps - layer.start_offset.0.to_f64(),
        off: layer.start_offset.0.to_f64(),
        fps,
        viewport: panel,
        // The whole panel is the control column; there is no time lane, so
        // put its left edge at the panel's right and skip lane painting.
        track_left: panel.right(),
        track_w: 0.0,
        px_per_sec: 1.0,
        view_start: 0.0,
        graph_mode: true,
        selected_prop: app.selected_prop,
        selected_props: app.selected_props.clone(),
    };
    let mut fx_edit = None;
    let mut nav_jump = None;
    // The property-row draw order is rebuilt each frame for range-select (note
    // 2.6b). This panel draws only effect rows, so it owns the order while it
    // renders: clear before, resolve after (mirrors the Timeline). Whichever
    // panel the click lands in fills and resolves its own order in one pass, so
    // the two never tread on each other.
    app.prop_row_order.clear();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_salt("effect-controls-scroll")
        .show(ui, |ui| {
            effects_rows(ui, app, &ctx, &mut pending, &mut fx_edit, &mut nav_jump);
        });
    // A navigator arrow jumps the playhead to the neighbouring effect key.
    if let Some(kt) = nav_jump {
        app.preview_frame = ((kt + ctx.off) * fps).round().max(0.0) as usize;
        #[cfg(feature = "media")]
        app.refresh_preview();
    }
    // Live preview while an effect value is dragged. Only WRITE when this panel
    // has an active drag — the Timeline draws the same effect rows in the same
    // frame, and an unconditional `= None` here would clobber its drag (or vice
    // versa). The shell clears fx_edit once at the top of the frame.
    if fx_edit.is_some() {
        app.fx_edit = fx_edit;
    }
    // Row selection (notes 2.8.1/2.8.7/2.6): clicks on effect rows are applied
    // inside `effects_rows` now (plain / Ctrl / Shift gestures, UI-6). A
    // Shift-click marks a range target; resolve it against this panel's freshly
    // built draw order, exactly as the Timeline does.
    if let Some(target) = app.prop_range_target.take() {
        let (range, anchor_to_target) = prop_range(&app.prop_row_order, app.selected_prop, target);
        if anchor_to_target {
            app.selected_prop = Some(target);
        }
        app.selected_props = range;
    }
    if let Some(op) = pending {
        app.commit(op);
        #[cfg(feature = "media")]
        app.refresh_preview();
    }
}

/// Pixels + texture dims + natural size for any layer kind (preview path).
#[cfg(feature = "media")]
pub(crate) type LayerPixels = (Vec<u8>, u32, u32, (f32, f32));
