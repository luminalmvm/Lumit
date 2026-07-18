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
pub(crate) fn project_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
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
                item_rows(ui, theme, app, &doc, id, 0, &mut actions, &mut visited);
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
                app.commit(lumit_core::Op::RemoveItem { id });
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
pub(crate) fn project_header(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &AppState,
    doc: &lumit_core::model::Document,
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

/// One tree row (folders recurse). Rows are drag sources; folder rows are
/// drop targets.
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
            app.add_item_to_comp(dropped); // footage/solid → into the active comp
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

/// The effects browser (docs/07-UI-SPEC §7): the built-in catalogue
/// (docs/08-EFFECTS.md), grouped by category and filtered by the search
/// field, mirroring the Add-effect menu's grouping (`effects_rows`).
/// Each entry is a drag source (K-101): drag it onto a footage or
/// adjustment layer row in the Timeline to apply it there. Double-click
/// apply, presets and favourites are later steps of the same spec section.
pub(crate) fn effects_panel(ui: &mut egui::Ui, theme: &Theme) {
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
    let mut any_shown = false;
    egui::ScrollArea::vertical()
        .id_salt("effects-panel-scroll")
        .show(ui, |ui| {
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
                any_shown = true;
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
        });
    if !any_shown {
        ui.label(
            egui::RichText::new("No effects match.")
                .small()
                .color(theme.text_muted),
        );
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(
            "Drag an effect onto a layer in the Timeline, or add one from a layer's own Effects group there.",
        )
        .small()
        .color(theme.text_muted),
    );
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
    };
    let mut fx_edit = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_salt("effect-controls-scroll")
        .show(ui, |ui| {
            effects_rows(ui, &ctx, &mut pending, &mut fx_edit);
        });
    // Live preview while an effect value is dragged (cleared when not).
    app.fx_edit = fx_edit;
    if let Some(op) = pending {
        app.commit(op);
        #[cfg(feature = "media")]
        app.refresh_preview();
    }
}

/// Pixels + texture dims + natural size for any layer kind (preview path).
#[cfg(feature = "media")]
pub(crate) type LayerPixels = (Vec<u8>, u32, u32, (f32, f32));
