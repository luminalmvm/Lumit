//! The Timeline's top row (TL4: current time, layer search, view + MB-master
//! toggles) and its bottom bar (zoom, the magnet and grid-mode controls, the
//! graph lens toggle).

use super::*;

/// The timeline's top row (TL4): the current time/frame on the left, a
/// layer-search box in the middle, and the graph-view toggle + the composition
/// motion-blur master on the right (both moved up from the bottom bar, T22).
/// Drawn above the ruler, making the time bar taller. Edits `app` in place.
pub(crate) fn timeline_top_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    comp_id: uuid::Uuid,
    fps: f64,
    frame: usize,
) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme.surface_1);
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.bottom()),
            egui::pos2(rect.right(), rect.bottom()),
        ],
        egui::Stroke::new(1.0_f32, theme.hairline),
    );

    // Current time / frame, top-left (m:ss:ff plus the raw frame number).
    let f = fps.max(1.0).round() as usize;
    let (mm, ss, ff) = (frame / (f * 60), (frame / f) % 60, frame % f);
    ui.painter().text(
        rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{mm}:{ss:02}:{ff:02}   f{frame}"),
        egui::FontId::proportional(12.0),
        theme.text_secondary,
    );

    // Right cluster: MB master then the view toggle (right-to-left, so the view
    // glyphs sit at the far right).
    let right = egui::Rect::from_min_max(
        egui::pos2(rect.right() - 118.0, rect.top()),
        egui::pos2(rect.right() - 4.0, rect.bottom()),
    );
    let mut rc = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(right)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    rc.set_clip_rect(right);
    let graph = app.timeline_graph_mode;
    if rc
        .selectable_label(graph, crate::icons::text(Icon::GraphCurve, 13.0))
        .on_hover_text("Graph editor")
        .clicked()
    {
        app.timeline_graph_mode = true;
    }
    if rc
        .selectable_label(!graph, crate::icons::text(Icon::TimelineBars, 13.0))
        .on_hover_text("Layers")
        .clicked()
    {
        app.timeline_graph_mode = false;
    }
    // Hide switched-off layers (TL4): declutter the outline to the live layers.
    rc.add_space(6.0);
    if rc
        .selectable_label(
            app.timeline_hide_invisible,
            crate::icons::text(Icon::EyeClosed, 13.0),
        )
        .on_hover_text("Hide switched-off layers")
        .clicked()
    {
        app.timeline_hide_invisible = !app.timeline_hide_invisible;
    }
    // Composition motion-blur master (T9/T22): the comp-wide enable the per-layer
    // MB switches need. With it on, every layer whose own MB switch is set blurs.
    let mut mb = app
        .store
        .snapshot()
        .comp(comp_id)
        .map(|c| c.motion_blur)
        .unwrap_or_default();
    rc.add_space(6.0);
    if rc
        .selectable_label(mb.enabled, egui::RichText::new("MB").small())
        .on_hover_text(
            "Composition motion blur (master): layers with their own motion-blur switch on then blur",
        )
        .clicked()
    {
        mb.enabled = !mb.enabled;
        app.commit(lumit_core::Op::SetCompMotionBlur {
            comp: comp_id,
            motion_blur: mb,
        });
    }

    // Layer-search box, middle.
    let sw = 170.0;
    let search = egui::Rect::from_min_max(
        egui::pos2(rect.center().x - sw * 0.5, rect.top() + 2.0),
        egui::pos2(rect.center().x + sw * 0.5, rect.bottom() - 2.0),
    );
    let mut sc = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(search)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    sc.set_clip_rect(search);
    sc.add(
        egui::TextEdit::singleline(&mut app.timeline_layer_search)
            .hint_text("Search layers")
            .desired_width(sw),
    );
}

/// The lane-view bottom bar spanning the lanes area: zoom controls (`− + Fit` +
/// readout) on the left — joined in graph mode by the value/speed lens toggle,
/// its own group behind a hairline divider — the view toggle on the right, and
/// a draggable horizontal scrollbar just above it (shown when zoomed in).
pub(crate) fn timeline_bottom_bar(
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

    // Controls-bar background across the lanes — a faint step above the panel
    // (rerun's bottom-bar treatment, K-084), parted from it by a hairline.
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(track_left, bar_top),
            egui::pos2(panel_right, panel.bottom()),
        ),
        0.0,
        theme.surface_2,
    );
    ui.painter().line_segment(
        [
            egui::pos2(track_left, bar_top),
            egui::pos2(panel_right, bar_top),
        ],
        egui::Stroke::new(1.0_f32, theme.hairline),
    );

    // Zoom controls, bottom-left of the lanes (with room for the lens toggle
    // that joins them in graph mode). The right edge stops just before the view
    // toggle's true 56 px footprint (K-070) — the old 64 px reservation held back
    // an item-gap the toggle no longer spends, and clipped the graph-option
    // buttons at the cluster's right (UI-14).
    let zr = egui::Rect::from_min_max(
        egui::pos2(track_left + 4.0, bar_top),
        egui::pos2((panel_right - 60.0).max(track_left + 8.0), panel.bottom()),
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

    // Guide-line mode for the lanes (owner request): beats, the time grid, or
    // nothing. Lives with the zoom cluster since both are about reading time.
    {
        use crate::app_state::TimelineGrid;
        let label = match app.timeline_grid {
            TimelineGrid::Beats => "Grid: beats",
            TimelineGrid::Time => "Grid: time",
            TimelineGrid::Off => "Grid: off",
        };
        zc.menu_button(egui::RichText::new(label).small(), |ui| {
            for (mode, name) in [
                (TimelineGrid::Beats, "Beats"),
                (TimelineGrid::Time, "Time (seconds and subdivisions)"),
                (TimelineGrid::Off, "Off"),
            ] {
                if ui.radio(app.timeline_grid == mode, name).clicked() {
                    app.timeline_grid = mode;
                    ui.close_menu();
                }
            }
        });
    }

    // Magnet (K-007 snapping toggle): on by default, shared by both the lane and
    // graph views (this cluster draws in both). When on, a dragged keyframe
    // snaps its time to the nearest whole frame. Lives here beside the grid pick
    // since both are about reading and landing on time.
    if zc
        .selectable_label(app.magnet_snap, crate::icons::text(Icon::Magnet, 13.0))
        .on_hover_text("Magnet: snap dragged keyframes to whole frames")
        .clicked()
    {
        app.magnet_snap = !app.magnet_snap;
    }

    // (The composition motion-blur master and the view toggle moved up to the
    // timeline's top row — TL4/T22.)

    // The value/speed lens toggle (graph mode only): one lens shared by every
    // curve, so it lives here rather than in each plot's header. Its own group,
    // split from the zoom cluster by a hairline divider. The wording follows
    // the graphed channel: a retimed layer's Speed reads source timecode and
    // per cent, a transform property reads its value and rate of change.
    if app.timeline_graph_mode {
        zc.add_space(6.0);
        let x = zc.cursor().left();
        zc.painter().line_segment(
            [
                egui::pos2(x, bar_top + 5.0),
                egui::pos2(x, panel.bottom() - 5.0),
            ],
            egui::Stroke::new(1.0_f32, theme.hairline_strong),
        );
        zc.add_space(6.0);
        let retime = app.graph_retime;
        let (value_label, speed_label) = if retime {
            ("Time", "Velocity") // K-076: the Retime channel's lenses
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
            app.graph_reset_fit(); // a manual range doesn't carry across lenses
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
            app.graph_reset_fit();
        }

        // Fit toggle (K-079): the value graph keeps re-fitting the curve (and
        // its tangent handles) vertically while this is on — lit — and
        // scrolling or zooming vertically switches it off with a manual range.
        // Clicking it while lit freezes the view where it is; clicking it back
        // on drops the manual range and resumes fitting. Only the value lens
        // has a manual range.
        if !app.graph_speed_view {
            zc.add_space(6.0);
            if zc
                .selectable_label(app.graph_auto_fit, "Fit")
                .on_hover_text("Keep the curve and its handles fitted to the graph height. Click to freeze the view; scroll or zoom vertically to take over.")
                .clicked()
            {
                if app.graph_auto_fit {
                    // Freeze: last frame's fit becomes the manual range; the
                    // plot stamps the height it is framed at next pass.
                    if let Some(fit) = app.graph_last_fit {
                        app.graph_auto_fit = false;
                        app.graph_view_y = Some(fit);
                        app.graph_view_h = None;
                    }
                } else {
                    app.graph_reset_fit();
                }
            }
        }

        // Interpolation buttons for a graphed transform property (not the Retime
        // channel): convert the selected keys — or all of them — to straight or
        // eased. Linear is the default key; Bezier is AE's easy-ease (also F9),
        // after which the key's yellow tangent handles are draggable.
        if !retime && app.graph_prop.is_some() {
            zc.add_space(6.0);
            let x = zc.cursor().left();
            zc.painter().line_segment(
                [
                    egui::pos2(x, bar_top + 5.0),
                    egui::pos2(x, panel.bottom() - 5.0),
                ],
                egui::Stroke::new(1.0_f32, theme.hairline_strong),
            );
            zc.add_space(6.0);
            if zc
                .small_button("Linear")
                .on_hover_text("Straighten the selected keys (or all)")
                .clicked()
            {
                app.graph_set_interp = Some(lumit_core::anim::SideInterp::Linear);
            }
            if zc
                .small_button("Bezier")
                .on_hover_text("Easy-ease the selected keys (or all) — AE's F9")
                .clicked()
            {
                app.graph_set_interp = Some(lumit_core::anim::EASY_EASE);
            }
            if zc
                .small_button("Hold")
                .on_hover_text("Hold the selected keys (or all) — value steps at the next key")
                .clicked()
            {
                app.graph_set_interp = Some(lumit_core::anim::SideInterp::Hold);
            }
        }
    }

    // (The view toggle moved up to the timeline's top row — TL4.)
}
