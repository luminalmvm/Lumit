//! The Timeline's bottom bar: the magnet and grid-mode controls, and the
//! graph-view toggle.

use super::*;

/// The layer-view / graph-view switch (K-070), drawn right-anchored in `rect`.
pub(crate) fn graph_toggle(
    ui: &mut egui::Ui,
    _theme: &Theme,
    app: &mut AppState,
    rect: egui::Rect,
) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.set_clip_rect(rect);
    // Render the two views as selectable text glyphs — the same look as the
    // magnet, which sits perfectly — and inset a few px from the panel's right
    // edge so the rightmost glyph isn't shaved by the clip (T10; the framed
    // icon-button chips sat 1-2 px into it).
    child.add_space(3.0);
    let graph = app.timeline_graph_mode;
    if child
        .selectable_label(graph, crate::icons::text(Icon::GraphCurve, 13.0))
        .on_hover_text("Graph editor")
        .clicked()
    {
        app.timeline_graph_mode = true;
    }
    if child
        .selectable_label(!graph, crate::icons::text(Icon::TimelineBars, 13.0))
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

    // Composition motion-blur master (T9): the comp-wide enable that the
    // per-layer motion-blur switches need — previously reachable only in Comp
    // settings, so a per-layer switch looked dead. Surfaced here as a master
    // toggle (AE's timeline motion-blur button); with it on, every layer whose
    // own MB switch is set blurs along its motion.
    if let Some(comp_id) = app.preview_comp {
        let mut mb = app
            .store
            .snapshot()
            .comp(comp_id)
            .map(|c| c.motion_blur)
            .unwrap_or_default();
        if zc
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
    }

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

    // View toggle, bottom-right of the lanes.
    let gr = egui::Rect::from_min_max(
        egui::pos2(panel_right - 60.0, bar_top),
        egui::pos2(panel_right - 4.0, panel.bottom()),
    );
    graph_toggle(ui, theme, app, gr);
}
