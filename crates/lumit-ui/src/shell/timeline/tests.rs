use super::*;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod effect_drop_tests {
    use super::*;
    use lumit_core::model::LayerKind;

    // Regression for K-101: the two layer kinds an effect stack is dropped
    // onto from the Effects & Presets browser must keep accepting the drop.
    #[test]
    fn footage_and_adjustment_layers_accept_an_effect_drop() {
        assert!(accepts_effect_drop(&LayerKind::Footage {
            item: uuid::Uuid::nil(),
            retime: None,
        }));
        assert!(accepts_effect_drop(&LayerKind::Adjustment));
    }

    // Other layer kinds still gain effects through the existing "Add effect"
    // row (untouched by K-101); a drop on their Timeline row is a no-op.
    #[test]
    fn other_layer_kinds_do_not_accept_an_effect_drop() {
        assert!(!accepts_effect_drop(&LayerKind::Solid {
            def: uuid::Uuid::nil(),
        }));
        assert!(!accepts_effect_drop(&LayerKind::Precomp {
            comp: uuid::Uuid::nil(),
        }));
        assert!(!accepts_effect_drop(&LayerKind::Sequence {
            clips: Vec::new(),
        }));
    }

    /// Regression for the invisible, undraggable outline/lane divider: egui
    /// hit-tests a widget against its rect ∩ the ui clip, and the timeline
    /// narrows its clip to the lane area (x ≥ track_left) for the
    /// time-positioned overlays — the divider handle sits just LEFT of that,
    /// so under the lane clip its rect ∩ clip was empty and it neither drew
    /// nor dragged. The scene reproduces the geometry in miniature: a handle
    /// left of a lane clip is dead until the clip is widened around it, which
    /// is exactly what `timeline_panel` now does.
    fn divider_drag_delta(widen_clip: bool) -> f32 {
        let ctx = egui::Context::default();
        let moved = std::cell::Cell::new(0.0_f32);
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
                    let full = ui.max_rect();
                    let track_left = 200.0;
                    let lane =
                        egui::Rect::from_min_max(egui::pos2(track_left, full.top()), full.max);
                    let saved = ui.clip_rect();
                    ui.set_clip_rect(saved.intersect(lane));
                    if widen_clip {
                        ui.set_clip_rect(saved); // the fix
                    }
                    // The handle straddles sep_x = track_left - 4, entirely
                    // left of the lane clip — the original geometry, whose
                    // rect ∩ clip came out empty.
                    let handle = egui::Rect::from_min_max(
                        egui::pos2(track_left - 7.0, full.top()),
                        egui::pos2(track_left - 1.0, full.bottom()),
                    );
                    let r = ui.interact(
                        handle,
                        egui::Id::new("name-col-resize"),
                        egui::Sense::click_and_drag(),
                    );
                    if r.dragged() {
                        moved.set(moved.get() + r.drag_delta().x);
                    }
                });
            });
        };
        let m = egui::Modifiers::default();
        let btn = egui::PointerButton::Primary;
        run(vec![]); // lay out
        let from = egui::pos2(196.0, 200.0);
        let to = egui::pos2(236.0, 200.0);
        run(vec![egui::Event::PointerMoved(from)]);
        run(vec![egui::Event::PointerButton {
            pos: from,
            button: btn,
            pressed: true,
            modifiers: m,
        }]);
        run(vec![egui::Event::PointerMoved(to)]); // drag right, past threshold
        run(vec![egui::Event::PointerButton {
            pos: to,
            button: btn,
            pressed: false,
            modifiers: m,
        }]);
        moved.get()
    }

    #[test]
    fn the_divider_drags_only_once_the_lane_clip_is_widened_around_it() {
        // The bug: under the lane clip the handle's rect ∩ clip is empty.
        assert_eq!(divider_drag_delta(false), 0.0);
        // The fix: with the clip widened, the drag registers and reports
        // the pointer's travel.
        assert!(divider_drag_delta(true) > 0.0);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod lane_drag_tests {
    use super::*;
    use crate::app_state::{LaneKeySel, PropRow};
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    use lumit_core::model::{
        BlendMode, Composition, Layer, LayerKind, LinearColour, Switches, TransformGroup,
        TransformProp,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
    use uuid::Uuid;

    fn kf_prop(times: &[f64]) -> Property {
        let keys = times
            .iter()
            .map(|&t| Keyframe {
                time: rational_at(t),
                value: 0.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            })
            .collect();
        Property {
            animation: Animation::Keyframed(keys),
            extra: serde_json::Map::new(),
        }
    }

    fn comp_one_layer(tf: TransformGroup) -> (Composition, Uuid) {
        let lid = Uuid::now_v7();
        let layer = Layer {
            id: lid,
            name: "L".into(),
            kind: LayerKind::Solid { def: Uuid::nil() },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: tf,
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: BlendMode::Normal,
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "C".into(),
            width: 100,
            height: 100,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0; 4]),
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        (comp, lid)
    }

    fn tsel(layer: Uuid, prop: TransformProp, t: f64) -> LaneKeySel {
        LaneKeySel {
            layer,
            row: PropRow::Transform(prop),
            time: rational_at(t),
        }
    }

    // A single transform-channel selection slides just that channel's keys.
    #[test]
    fn transform_selection_shifts_that_channel() {
        let tf = TransformGroup {
            rotation: kf_prop(&[1.0, 2.0]),
            ..Default::default()
        };
        let (comp, lid) = comp_one_layer(tf);
        let sel = [tsel(lid, TransformProp::Rotation, 1.0)];
        let op = build_lane_drag_op(&comp, &sel, &[], 0.5, 30.0).unwrap();
        match op {
            lumit_core::Op::SetTransformProperty {
                prop, animation, ..
            } => {
                assert_eq!(prop, TransformProp::Rotation);
                let Animation::Keyframed(k) = animation else {
                    panic!("expected keyframed");
                };
                let ts: Vec<f64> = k.iter().map(|x| x.time.to_f64()).collect();
                assert_eq!(ts, vec![1.5, 2.0]);
            }
            other => panic!("expected SetTransformProperty, got {other:?}"),
        }
    }

    // A linked pair listed in the register moves both axes' keys in one Batch.
    #[test]
    fn linked_pair_moves_both_axes() {
        let tf = TransformGroup {
            position_x: kf_prop(&[1.0]),
            position_y: kf_prop(&[1.0]),
            ..Default::default()
        };
        let (comp, lid) = comp_one_layer(tf);
        let sel = [tsel(lid, TransformProp::PositionX, 1.0)];
        let linked = [(lid, TransformProp::PositionX)];
        let op = build_lane_drag_op(&comp, &sel, &linked, 1.0, 30.0).unwrap();
        let lumit_core::Op::Batch { ops } = op else {
            panic!("expected a Batch across both axes");
        };
        let props: Vec<TransformProp> = ops
            .iter()
            .filter_map(|o| match o {
                lumit_core::Op::SetTransformProperty { prop, .. } => Some(*prop),
                _ => None,
            })
            .collect();
        assert!(props.contains(&TransformProp::PositionX));
        assert!(props.contains(&TransformProp::PositionY));
    }

    // The same channel, NOT in the register (unlinked), moves only itself.
    #[test]
    fn unlinked_axis_moves_only_itself() {
        let tf = TransformGroup {
            position_x: kf_prop(&[1.0]),
            position_y: kf_prop(&[1.0]),
            ..Default::default()
        };
        let (comp, lid) = comp_one_layer(tf);
        let sel = [tsel(lid, TransformProp::PositionX, 1.0)];
        let op = build_lane_drag_op(&comp, &sel, &[], 1.0, 30.0).unwrap();
        match op {
            lumit_core::Op::SetTransformProperty { prop, .. } => {
                assert_eq!(prop, TransformProp::PositionX);
            }
            other => panic!("expected a single SetTransformProperty, got {other:?}"),
        }
    }

    #[test]
    fn partner_maps_x_to_y_only() {
        assert_eq!(
            linked_partner(TransformProp::ScaleX),
            Some(TransformProp::ScaleY)
        );
        assert_eq!(
            linked_partner(TransformProp::AnchorX),
            Some(TransformProp::AnchorY)
        );
        assert_eq!(linked_partner(TransformProp::Rotation), None);
        assert_eq!(linked_partner(TransformProp::ScaleY), None);
    }

    /// A footage layer whose Time-remap has one interior value key at comp-time
    /// 5s (source curve 0→2→10 over 0→5→10s), on a 10s comp.
    fn comp_footage_retime() -> (Composition, Uuid) {
        let lid = Uuid::now_v7();
        let r = |n: i64| Rational::new(n, 1).unwrap();
        let vk = [(r(0), r(0)), (r(5), r(2)), (r(10), r(10))];
        let layer = Layer {
            id: lid,
            name: "F".into(),
            kind: LayerKind::Footage {
                item: Uuid::nil(),
                retime: lumit_core::retime::Retime::from_value_keyframes(&vk),
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(r(10)),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: BlendMode::Normal,
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "C".into(),
            width: 100,
            height: 100,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(r(10)),
            background: LinearColour([0.0; 4]),
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        (comp, lid)
    }

    /// A4: a lane drag on a Retime Time (value) key slides that interior key's
    /// screen time; the [0, dur] endpoints stay put.
    #[test]
    fn retime_time_key_drags_the_interior_value_key() {
        let (comp, lid) = comp_footage_retime();
        let sel = [LaneKeySel {
            layer: lid,
            row: PropRow::Retime,
            time: rational_at(5.0),
        }];
        let op = build_lane_drag_op(&comp, &sel, &[], 1.0, 30.0).unwrap();
        let lumit_core::Op::SetLayerRetime {
            retime: Some(rt), ..
        } = op
        else {
            panic!("expected SetLayerRetime, got {op:?}");
        };
        let vk = rt.value_keyframes();
        // Endpoints unchanged; the interior key moved from 5s to ~6s.
        assert_eq!(vk.first().unwrap().0.to_f64(), 0.0);
        assert_eq!(vk.last().unwrap().0.to_f64(), 10.0);
        assert!(
            (vk[1].0.to_f64() - 6.0).abs() < 0.05,
            "interior key moved to ~6s: {:?}",
            vk[1].0.to_f64()
        );
    }

    /// A4: dragging a Retime ENDPOINT does nothing — the structural [0, dur]
    /// keys are protected (only interior keys move).
    #[test]
    fn retime_endpoint_key_does_not_move() {
        let (comp, lid) = comp_footage_retime();
        let sel = [LaneKeySel {
            layer: lid,
            row: PropRow::Retime,
            time: rational_at(0.0), // the start endpoint
        }];
        // No interior key selected, so nothing moves → no op.
        assert!(build_lane_drag_op(&comp, &sel, &[], 1.0, 30.0).is_none());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod lane_marquee_interaction_tests {
    // The lane marquee relies on egui's hit-test order: the full-lane background
    // (click+drag) is registered BEFORE the layer rows (click only), so a press
    // on empty lane space that then moves opens the marquee instead of reading as
    // a click on the row underneath. If egui ever changed that fall-through, the
    // marquee would silently stop working — this pins the behaviour.
    fn drive() -> (bool, bool) {
        let ctx = egui::Context::default();
        let bg_drag = std::cell::Cell::new(false);
        let row_click = std::cell::Cell::new(false);
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
                    let full = ui.max_rect();
                    // Marquee background: added first, senses click and drag.
                    let bg = ui.interact(full, egui::Id::new("bg"), egui::Sense::click_and_drag());
                    if bg.dragged() {
                        bg_drag.set(true);
                    }
                    // A layer row on top: added after, senses click only.
                    let row = ui.interact(full, egui::Id::new("row"), egui::Sense::click());
                    if row.clicked() {
                        row_click.set(true);
                    }
                });
            });
        };
        let m = egui::Modifiers::default();
        let btn = egui::PointerButton::Primary;
        run(vec![]);
        let from = egui::pos2(100.0, 100.0);
        let to = egui::pos2(170.0, 150.0);
        run(vec![egui::Event::PointerMoved(from)]);
        run(vec![egui::Event::PointerButton {
            pos: from,
            button: btn,
            pressed: true,
            modifiers: m,
        }]);
        run(vec![egui::Event::PointerMoved(to)]);
        run(vec![egui::Event::PointerButton {
            pos: to,
            button: btn,
            pressed: false,
            modifiers: m,
        }]);
        (bg_drag.get(), row_click.get())
    }

    #[test]
    fn a_drag_on_empty_lane_opens_the_marquee_not_a_row_click() {
        let (bg_dragged, row_clicked) = drive();
        assert!(bg_dragged, "the drag must reach the marquee background");
        assert!(!row_clicked, "a drag must not read as a click on the row");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod span_move_tests {
    use super::*;
    use lumit_core::time::{CompTime, Rational};

    fn ct(s: f64) -> CompTime {
        CompTime(Rational::from_f64_on_grid(s, Rational::FLICK_DEN).unwrap())
    }

    /// GEN-3 (K-153): moving a layer left past comp 0 lands a NEGATIVE in point
    /// (and start offset) — no longer clamped to 0. In/out/offset shift together
    /// so the bar and its content move as one; the comp window clips the pre-0
    /// head at render time.
    #[test]
    fn moving_a_layer_before_comp_start_keeps_a_negative_in_point() {
        // Layer at [1, 4) with offset 1; drag left by 3 s.
        let (in_p, out_p, off) = moved_span(ct(1.0), ct(4.0), ct(1.0), -3.0);
        assert!(in_p.0.is_negative(), "in point crosses before 0: {in_p:?}");
        assert!((in_p.0.to_f64() - (-2.0)).abs() < 1e-9);
        assert!((out_p.0.to_f64() - 1.0).abs() < 1e-9);
        assert!((off.0.to_f64() - (-2.0)).abs() < 1e-9);
        // Span length and the in↔offset relation are preserved by the move.
        assert!(((out_p.0.to_f64() - in_p.0.to_f64()) - 3.0).abs() < 1e-9);
        assert!((in_p.0.to_f64() - off.0.to_f64()).abs() < 1e-9);
    }

    /// The out point may also cross past the comp end (unbounded above).
    #[test]
    fn moving_a_layer_right_lets_the_out_point_pass_the_comp_end() {
        let (in_p, out_p, _off) = moved_span(ct(0.0), ct(5.0), ct(0.0), 8.0);
        assert!((in_p.0.to_f64() - 8.0).abs() < 1e-9);
        assert!((out_p.0.to_f64() - 13.0).abs() < 1e-9);
    }
}

// UI-8: the wheel routes differently per timeline mode. In the layers view the
// outline and lanes share one scroll (a plain wheel anywhere over the lanes
// falls through to it); in the graph view the curve and the layer list are
// decoupled — a plain wheel over the curve pans it and never scrolls the list.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod wheel_route_tests {
    use super::*;

    fn plain() -> egui::Modifiers {
        egui::Modifiers::default()
    }

    #[test]
    fn lane_view_plain_wheel_over_the_lane_feeds_the_shared_scroll() {
        // Layers view: a plain vertical wheel over the lane area is left to the
        // one shared ScrollArea, so the outline and the lanes move together.
        let r = timeline_wheel_route(false, true, plain(), false, true);
        assert_eq!(r, TimelineWheel::Scroll);
    }

    #[test]
    fn graph_view_plain_wheel_over_the_curve_goes_to_the_graph_not_the_list() {
        // Graph view: the same wheel belongs to the curve — the layer list is
        // untouched (its own scroll area stops at the outline's right edge).
        let r = timeline_wheel_route(true, true, plain(), false, true);
        assert_eq!(r, TimelineWheel::Graph);
    }

    #[test]
    fn a_wheel_over_the_outline_column_always_feeds_a_scroll_area() {
        // Over the outline (not the lane) the routing is the same in both modes:
        // a ScrollArea owns it (shared in lane view, outline-only in graph view).
        for graph in [false, true] {
            let r = timeline_wheel_route(graph, false, plain(), false, true);
            assert_eq!(r, TimelineWheel::Scroll, "graph_mode = {graph}");
        }
    }

    #[test]
    fn alt_wheel_over_the_lane_zooms_time_in_either_mode() {
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        for graph in [false, true] {
            let r = timeline_wheel_route(graph, true, alt, false, true);
            assert_eq!(r, TimelineWheel::ZoomTime, "graph_mode = {graph}");
        }
    }

    #[test]
    fn shift_or_horizontal_wheel_over_the_lane_pans_time() {
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        // Shift + vertical wheel pans time (over the lane, either mode).
        assert_eq!(
            timeline_wheel_route(true, true, shift, false, true),
            TimelineWheel::PanTime
        );
        // A horizontal wheel pans time without any modifier.
        assert_eq!(
            timeline_wheel_route(false, true, plain(), true, false),
            TimelineWheel::PanTime
        );
    }

    #[test]
    fn time_pan_and_zoom_never_fire_over_the_outline_column() {
        // Alt/Shift only zoom or pan the time axis over the lane — over the
        // outline the wheel always scrolls the list, whatever the modifiers.
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            timeline_wheel_route(true, false, alt, false, true),
            TimelineWheel::Scroll
        );
    }
}
