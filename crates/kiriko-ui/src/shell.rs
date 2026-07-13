//! The application shell: menu bar, docked panels, status line.
//!
//! Layout per docs/07-UI-SPEC.md (Edit workspace): Project left, Viewer centre,
//! Effect Controls / Effects & Presets right, Timeline across the bottom.

use crate::app_state::AppState;
use crate::splash::{BootLine, Splash};
use crate::theme::Theme;
use egui_dock::{DockArea, DockState, NodeIndex, Style as DockStyle};
use kiriko_core::model::ProjectItem;
use serde::{Deserialize, Serialize};

/// The dockable panels. Names are glossary names (docs/01-GLOSSARY.md §7).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Panel {
    Project,
    Viewer,
    Timeline,
    EffectControls,
    EffectsAndPresets,
    Scopes,
    GraphEditor,
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
            Panel::GraphEditor => "Graph editor",
        }
    }
}

/// Build the default Edit workspace arrangement.
pub fn default_layout() -> DockState<Panel> {
    let mut state = DockState::new(vec![Panel::Viewer]);
    let surface = state.main_surface_mut();
    let [centre, _timeline] = surface.split_below(
        NodeIndex::root(),
        0.65,
        vec![Panel::Timeline, Panel::GraphEditor],
    );
    let [centre, _project] = surface.split_left(centre, 0.22, vec![Panel::Project]);
    let [_centre, _right] = surface.split_right(
        centre,
        0.78,
        vec![
            Panel::EffectControls,
            Panel::EffectsAndPresets,
            Panel::Scopes,
        ],
    );
    state
}

struct PanelViewer<'a> {
    theme: &'a Theme,
    app: &'a mut AppState,
    preview_display: Option<(egui::TextureId, egui::Vec2)>,
}

impl egui_dock::TabViewer for PanelViewer<'_> {
    type Tab = Panel;

    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Panel) {
        match tab {
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
            Panel::GraphEditor => graph_editor_panel(ui, self.theme, self.app),
            Panel::Scopes => empty_hint(
                ui,
                self.theme,
                "Scopes",
                "Waveform, vectorscope and histogram arrive with the render pipeline.",
            ),
        }
    }
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

fn project_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    let doc = app.store.snapshot();
    if doc.items.is_empty() {
        empty_hint(
            ui,
            theme,
            "No footage yet",
            "Drag files anywhere in the window, or use File → Import.",
        );
        return;
    }
    ui.add_space(4.0);
    let mut select = None;
    let mut add_to_comp: Option<uuid::Uuid> = None;
    let mut nest_comp: Option<uuid::Uuid> = None;
    for item in &doc.items {
        let (kind, colour) = match item {
            ProjectItem::Footage(_) => ("footage", theme.text_muted),
            ProjectItem::Folder(_) => ("folder", theme.text_muted),
            ProjectItem::Composition(_) => ("comp", theme.accent),
        };
        let selected = app.selected_comp == Some(item.id());
        let row = ui.selectable_label(
            selected,
            egui::RichText::new(format!("{}  ", item.name())).color(theme.text_secondary),
        );
        let row = row.on_hover_text(kind);
        ui.painter().text(
            row.rect.right_center() + egui::vec2(-4.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            kind,
            egui::FontId::monospace(10.0),
            colour,
        );
        if row.clicked() {
            match item {
                ProjectItem::Composition(_) => {
                    select = Some(item.id());
                    app.preview_comp = Some(item.id());
                    app.preview_item = None;
                    app.preview_frame = 0;
                    #[cfg(feature = "media")]
                    app.refresh_preview();
                }
                ProjectItem::Footage(_) => {
                    app.preview_item = Some(item.id());
                    app.preview_frame = 0;
                    #[cfg(feature = "media")]
                    {
                        app.refresh_preview();
                        app.request_preview_audio();
                    }
                }
                _ => {}
            }
        }
        if let ProjectItem::Composition(_) = item {
            let target = app.preview_comp.or(app.selected_comp);
            let nestable = target.is_some_and(|t| t != item.id());
            ui.indent(("nest", item.id()), |ui| {
                if ui
                    .add_enabled(nestable, egui::Button::new("Nest in comp").small())
                    .on_hover_text("Add as a Precomp layer of the selected composition")
                    .clicked()
                {
                    nest_comp = Some(item.id());
                }
            });
        }
        if let ProjectItem::Footage(_) = item {
            let target = app.preview_comp.or(app.selected_comp);
            ui.indent(("addto", item.id()), |ui| {
                if ui
                    .add_enabled(target.is_some(), egui::Button::new("Add to comp").small())
                    .on_hover_text("Add as the top layer of the selected composition")
                    .clicked()
                {
                    add_to_comp = Some(item.id());
                }
            });
        }
        #[cfg(feature = "media")]
        if let ProjectItem::Footage(_) = item {
            use crate::app_state::media::MediaStatus;
            ui.indent(item.id(), |ui| match app.media.map.get(&item.id()) {
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
            });
        }
    }
    if let Some(id) = select {
        app.selected_comp = Some(id);
    }
    if let Some(id) = add_to_comp {
        app.add_footage_to_comp(id);
    }
    if let Some(id) = nest_comp {
        app.add_precomp_to_comp(id);
    }
}

fn timeline_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    let doc = app.store.snapshot();
    let comp = app.selected_comp.and_then(|id| doc.comp(id));
    let Some(comp) = comp else {
        empty_hint(
            ui,
            theme,
            "No composition open",
            "Create one with Composition → New, or drop footage here.",
        );
        return;
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&comp.name).color(theme.text_primary));
        ui.label(
            egui::RichText::new(format!(
                "{}×{}  {:.2} fps",
                comp.width,
                comp.height,
                comp.frame_rate.fps()
            ))
            .small()
            .color(theme.text_muted),
        );
    });
    ui.separator();
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
    let comp_id = comp.id;
    let mut pending: Option<kiriko_core::Op> = None;

    // ---- ruler + time geometry (07-UI-SPEC Timeline) --------------------
    let name_w = 180.0_f32;
    let duration = comp.duration.0.to_f64().max(1e-6);
    let frames = app.comp_frame_count(comp).max(1);
    let panel_left = ui.max_rect().left();
    let panel_right = ui.max_rect().right();
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
            #[cfg(feature = "media")]
            app.refresh_preview();
        }
    }
    let rows_top = ui.cursor().top();

    for layer in &comp.layers {
        let (row_rect, row_resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::click());
        if row_resp.clicked() {
            app.selected_layer = Some(layer.id);
        }
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
        ui.painter().text(
            egui::pos2(row_rect.left() + 4.0, row_rect.center().y),
            egui::Align2::LEFT_CENTER,
            &layer.name,
            egui::FontId::proportional(12.0),
            theme.text_secondary,
        );
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
        ui.indent(("matte", layer.id), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Matte").small().color(theme.text_muted));
                let current_name = layer
                    .matte
                    .as_ref()
                    .and_then(|m| comp.layers.iter().find(|l| l.id == m.layer))
                    .map(|l| l.name.clone())
                    .unwrap_or_else(|| "None".into());
                let mut set: Option<Option<kiriko_core::model::MatteRef>> = None;
                egui::ComboBox::from_id_salt(("matte-src", layer.id))
                    .selected_text(current_name)
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(layer.matte.is_none(), "None").clicked() {
                            set = Some(None);
                        }
                        for other in comp.layers.iter().filter(|l| l.id != layer.id) {
                            let selected = layer.matte.is_some_and(|m| m.layer == other.id);
                            if ui.selectable_label(selected, &other.name).clicked() {
                                set = Some(Some(kiriko_core::model::MatteRef {
                                    layer: other.id,
                                    channel: layer
                                        .matte
                                        .map(|m| m.channel)
                                        .unwrap_or(kiriko_core::model::MatteChannel::Alpha),
                                    inverted: layer.matte.is_some_and(|m| m.inverted),
                                }));
                            }
                        }
                    });
                if let Some(mut m) = layer.matte {
                    let luma = matches!(m.channel, kiriko_core::model::MatteChannel::Luma);
                    if ui
                        .selectable_label(luma, egui::RichText::new("luma").small())
                        .on_hover_text("Luma matte (else alpha)")
                        .clicked()
                    {
                        m.channel = if luma {
                            kiriko_core::model::MatteChannel::Alpha
                        } else {
                            kiriko_core::model::MatteChannel::Luma
                        };
                        set = Some(Some(m));
                    }
                    if ui
                        .selectable_label(m.inverted, egui::RichText::new("invert").small())
                        .clicked()
                    {
                        m.inverted = !m.inverted;
                        set = Some(Some(m));
                    }
                }
                if let Some(matte) = set {
                    pending = Some(kiriko_core::Op::SetLayerMatte {
                        comp: comp_id,
                        layer: layer.id,
                        matte,
                    });
                }
                ui.separator();
                ui.label(egui::RichText::new("Blend").small().color(theme.text_muted));
                use kiriko_core::model::BlendMode;
                let blend_name = |b: BlendMode| match b {
                    BlendMode::Normal => "Normal",
                    BlendMode::Add => "Add",
                    BlendMode::Multiply => "Multiply",
                    BlendMode::Screen => "Screen",
                };
                egui::ComboBox::from_id_salt(("blend", layer.id))
                    .selected_text(blend_name(layer.blend))
                    .width(90.0)
                    .show_ui(ui, |ui| {
                        for mode in [
                            BlendMode::Normal,
                            BlendMode::Add,
                            BlendMode::Multiply,
                            BlendMode::Screen,
                        ] {
                            if ui
                                .selectable_label(layer.blend == mode, blend_name(mode))
                                .clicked()
                                && layer.blend != mode
                            {
                                pending = Some(kiriko_core::Op::SetLayerBlend {
                                    comp: comp_id,
                                    layer: layer.id,
                                    blend: mode,
                                });
                            }
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Masks").small().color(theme.text_muted));
                if ui.small_button("+ Rect").clicked() {
                    let mut masks = layer.masks.clone();
                    let (w, h) = mask_space(layer, app, comp);
                    masks.push(kiriko_core::mask::Mask::rectangle(
                        w * 0.25,
                        h * 0.25,
                        w * 0.5,
                        h * 0.5,
                    ));
                    pending = Some(kiriko_core::Op::SetLayerMasks {
                        comp: comp_id,
                        layer: layer.id,
                        masks,
                    });
                }
                if ui.small_button("+ Ellipse").clicked() {
                    let mut masks = layer.masks.clone();
                    let (w, h) = mask_space(layer, app, comp);
                    masks.push(kiriko_core::mask::Mask::ellipse(
                        w * 0.5,
                        h * 0.5,
                        w * 0.3,
                        h * 0.3,
                    ));
                    pending = Some(kiriko_core::Op::SetLayerMasks {
                        comp: comp_id,
                        layer: layer.id,
                        masks,
                    });
                }
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
                    } else if ui.small_button("×").on_hover_text("Remove mask").clicked() {
                        masks.remove(mi);
                        pending = Some(kiriko_core::Op::SetLayerMasks {
                            comp: comp_id,
                            layer: layer.id,
                            masks,
                        });
                    }
                }
            });
        });
        if let kiriko_core::model::LayerKind::Text { document } = &layer.kind {
            ui.indent(("text", layer.id), |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Text").small().color(theme.text_muted));
                    let mut text = document.text.clone();
                    let resp = ui.add(egui::TextEdit::singleline(&mut text).desired_width(180.0));
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
        ui.indent(("transform", layer.id), |ui| {
            ui.collapsing(
                egui::RichText::new("Transform")
                    .small()
                    .color(theme.text_muted),
                |ui| {
                    egui::Grid::new(("txgrid", layer.id))
                        .num_columns(2)
                        .spacing(egui::vec2(12.0, 2.0))
                        .show(ui, |ui| {
                            let rows: [(&str, TransformProp, f64); 6] = [
                                ("Position x", TransformProp::PositionX, 1.0),
                                ("Position y", TransformProp::PositionY, 1.0),
                                ("Scale x %", TransformProp::ScaleX, 0.5),
                                ("Scale y %", TransformProp::ScaleY, 0.5),
                                ("Rotation °", TransformProp::Rotation, 0.5),
                                ("Opacity %", TransformProp::Opacity, 0.5),
                            ];
                            // Layer time at the playhead: where keyframes land
                            // (AE behaviour: editing an animated value writes a
                            // key at the current time).
                            let fps = comp.frame_rate.fps().max(1.0);
                            let lt = app.preview_frame as f64 / fps - layer.start_offset.0.to_f64();
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
                                        egui::RichText::new(label).small().color(theme.text_muted),
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
                                                Animation::Keyframed(upsert_key(slot, lt, value))
                                            } else {
                                                Animation::Static(value)
                                            };
                                            pending = Some(kiriko_core::Op::SetTransformProperty {
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
                },
            );
        });
    }
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
}

/// Footage preview: the frame fit to the surround, scrub bar, resolution picker.
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
            if view.dragged() {
                app.view_pan += view.drag_delta();
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
                    egui::ComboBox::from_id_salt("preview-res")
                        .selected_text(current)
                        .width(72.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(app.preview_auto_res, "Auto")
                                .on_hover_text("Decode at the displayed size, capped at 100%")
                                .clicked()
                            {
                                app.preview_auto_res = true;
                                app.refresh_preview();
                            }
                            for (i, label) in labels.iter().enumerate() {
                                let div = i as u32 + 1;
                                let selected = !app.preview_auto_res && app.preview_divisor == div;
                                if ui.selectable_label(selected, *label).clicked() {
                                    app.preview_auto_res = false;
                                    app.preview_divisor = div;
                                    app.refresh_preview();
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
fn graph_editor_panel(ui: &mut egui::Ui, theme: &Theme, app: &mut AppState) {
    use kiriko_core::anim::{Animation, Keyframe, SideInterp};
    use kiriko_core::model::TransformProp;

    let doc = app.store.snapshot();
    let comp = app
        .preview_comp
        .or(app.selected_comp)
        .and_then(|id| doc.comp(id));
    let (Some(comp), Some(layer_id)) = (comp, app.selected_layer) else {
        empty_hint(
            ui,
            theme,
            "Graph editor",
            "Select a layer in the Timeline to edit its curves.",
        );
        return;
    };
    let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
        empty_hint(ui, theme, "Graph editor", "The selected layer is gone.");
        return;
    };

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
    let animated: Vec<(TransformProp, &str)> = PROPS
        .iter()
        .copied()
        .filter(|(p, _)| layer.transform.get(*p).is_animated())
        .collect();
    if animated.is_empty() {
        empty_hint(
            ui,
            theme,
            &layer.name,
            "No animated properties — click a stopwatch in the Timeline first.",
        );
        return;
    }
    let current = app
        .graph_prop
        .filter(|p| animated.iter().any(|(a, _)| a == p))
        .unwrap_or(animated[0].0);
    app.graph_prop = Some(current);

    let mut set_sides: Option<SideInterp> = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&layer.name).color(theme.text_secondary));
        egui::ComboBox::from_id_salt("graph-prop")
            .selected_text(
                animated
                    .iter()
                    .find(|(p, _)| *p == current)
                    .map(|(_, n)| *n)
                    .unwrap_or("?"),
            )
            .show_ui(ui, |ui| {
                for (p, name) in &animated {
                    if ui.selectable_label(*p == current, *name).clicked() {
                        app.graph_prop = Some(*p);
                    }
                }
            });
        ui.separator();
        if ui
            .selectable_label(!app.graph_speed_view, "Value")
            .clicked()
        {
            app.graph_speed_view = false;
        }
        if ui
            .selectable_label(app.graph_speed_view, "Speed")
            .on_hover_text("The derivative view — editing here arrives with Retime")
            .clicked()
        {
            app.graph_speed_view = true;
        }
        ui.separator();
        if ui
            .small_button("Ease")
            .on_hover_text("Easy-ease every key of this curve (AE's F9)")
            .clicked()
        {
            set_sides = Some(kiriko_core::anim::EASY_EASE);
        }
        if ui
            .small_button("Linear")
            .on_hover_text("Straighten every key of this curve")
            .clicked()
        {
            set_sides = Some(SideInterp::Linear);
        }
    });

    let slot = layer.transform.get(current);
    let Animation::Keyframed(keys) = &slot.animation else {
        return;
    };

    // ---- plot geometry: x = layer time over the comp span, y = value ----
    let rect = ui.available_rect_before_wrap();
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
            LayerKind::Footage { .. } => pixels_by_layer.get(&layer.id).map(|lp| {
                (
                    lp.rgba.clone(),
                    lp.width,
                    lp.height,
                    (lp.width as f32, lp.height as f32),
                )
            }),
            LayerKind::Solid { colour } => in_span(layer).then(|| {
                let px = crate::export::solid_rgba(*colour);
                let (tw, th) = if layer.masks.is_empty() {
                    (8, 8)
                } else {
                    (comp.width, comp.height)
                };
                (
                    crate::export::px_tile(&px, tw, th),
                    tw,
                    th,
                    (comp.width as f32, comp.height as f32),
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
            matte,
            blend: blend_of(layer.blend),
        });
    }
    draws
}

/// The layer's natural pixel space (mask coordinates live here).
fn mask_space(
    layer: &kiriko_core::model::Layer,
    app: &AppState,
    comp: &kiriko_core::model::Composition,
) -> (f64, f64) {
    match &layer.kind {
        kiriko_core::model::LayerKind::Solid { .. }
        | kiriko_core::model::LayerKind::Precomp { .. }
        | kiriko_core::model::LayerKind::Text { .. } => {
            (f64::from(comp.width), f64::from(comp.height))
        }
        #[cfg(feature = "media")]
        kiriko_core::model::LayerKind::Footage { item } => match app.media.map.get(item) {
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
    match b {
        kiriko_core::model::BlendMode::Normal => kiriko_gpu::Blend::Normal,
        kiriko_core::model::BlendMode::Add => kiriko_gpu::Blend::Add,
        kiriko_core::model::BlendMode::Multiply => kiriko_gpu::Blend::Multiply,
        kiriko_core::model::BlendMode::Screen => kiriko_gpu::Blend::Screen,
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
    pub matte: Option<MatteDraw>,
    pub blend: kiriko_gpu::Blend,
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
                } => self.realise(*width, *height, *background, draws),
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
                    self.compositor.composite(
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
                        }],
                    )
                })
            })
            .collect();
        let comp_layers: Vec<kiriko_gpu::CompositeLayer> = linear_textures
            .iter()
            .zip(layers)
            .zip(&matte_textures)
            .map(|((texture, l), matte_tex)| kiriko_gpu::CompositeLayer {
                texture,
                size: l.natural_size,
                position: l.position,
                anchor: l.anchor,
                scale: l.scale,
                rotation_deg: l.rotation_deg,
                opacity: l.opacity,
                matte: matte_tex.as_ref().map(|mt| kiriko_gpu::MatteInput {
                    texture: mt,
                    luma: l.matte.as_ref().is_some_and(|m| m.luma),
                    inverted: l.matte.as_ref().is_some_and(|m| m.inverted),
                }),
                blend: l.blend,
            })
            .collect();
        self.compositor
            .composite(&self.ctx, width, height, background, &comp_layers)
    }

    /// Realise a comp's draws and register the frame for painting.
    fn present_comp(
        &mut self,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> (egui::TextureId, egui::Vec2) {
        let linear = self.realise(width, height, background, layers);
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

/// Persisted UI state (dock layout only; app state is runtime).
#[derive(Serialize, Deserialize)]
pub struct Shell {
    dock: DockState<Panel>,
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
                    "Colour pipeline: GPU (sRGB → linear fp16 → display)",
                ));
            }
            None => lines.push(BootLine {
                text: "Colour pipeline: CPU fallback (no wgpu render state)".into(),
                failed: true,
            }),
        }
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
            // Transport keys (07-UI-SPEC keymap; shuttle speeds arrive with
            // the ring buffer — J/left step back, L plays, K/Space pause).
            if self.app.preview_item.is_some() && !ctx.wants_keyboard_input() {
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
            let mut newest = None;
            while let Ok(result) = self.app.preview_engine.results.try_recv() {
                newest = Some(result);
            }
            use crate::app_state::preview::PreviewResult;
            match newest {
                Some(Ok(PreviewResult::Comp(cf))) if Some(cf.comp) == self.app.preview_comp => {
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
                            self.preview_display = Some(gpu.present_comp(
                                comp.width,
                                comp.height,
                                [
                                    f64::from(bg[0]),
                                    f64::from(bg[1]),
                                    f64::from(bg[2]),
                                    f64::from(bg[3]),
                                ],
                                &draws,
                            ));
                        }
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

        let mut style = DockStyle::from_egui(&ctx.style());
        style.tab_bar.bg_fill = self.theme.surface_0;
        style.tab_bar.hline_color = self.theme.hairline;

        let Shell {
            dock,
            theme,
            app,
            preview_display,
            ..
        } = self;
        DockArea::new(dock).style(style).show(
            ctx,
            &mut PanelViewer {
                theme,
                app,
                preview_display: *preview_display,
            },
        );
    }
}
