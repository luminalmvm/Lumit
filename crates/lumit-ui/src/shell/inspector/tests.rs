use super::*;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod section_bar_tests {
    use super::*;

    /// Regression for the invisible effect-title bars: the bar was painted in
    /// `surface_1`, which is the very colour the Round shape fills each pane
    /// card with and sits within a few RGB steps of the Sharp background
    /// (`surface_0`) — so in the Effect Controls panel the bar could not be
    /// seen at all. The fill must stand apart from BOTH pane backgrounds in
    /// every colour scheme, or effect boundaries vanish again.
    #[test]
    fn the_effect_title_bar_stands_apart_from_both_pane_backgrounds() {
        use crate::theme::{ColorScheme, ThemeShape};
        for scheme in ColorScheme::ALL {
            let theme = Theme::for_scheme(scheme, ThemeShape::Sharp);
            let fill = section_bar_fill(&theme);
            assert_ne!(
                fill, theme.surface_0,
                "{scheme:?}: the bar must not match the Sharp panel background"
            );
            assert_ne!(
                fill, theme.surface_1,
                "{scheme:?}: the bar must not match the Round pane card fill"
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod lane_key_tests {
    use super::*;
    use crate::app_state::{LaneKeySel, PropRow};
    use lumit_core::anim::{Keyframe, SideInterp};
    use lumit_core::model::TransformProp;

    fn key(t: f64, interp: SideInterp) -> Keyframe {
        Keyframe {
            time: rational_at(t),
            value: 0.0,
            interp_in: interp,
            interp_out: interp,
        }
    }

    // A lane drag shifts only the keys at the named times, and by the whole
    // delta — the group slides rigidly (note 2.1).
    #[test]
    fn shift_moves_only_the_named_times() {
        let keys = [
            key(0.0, SideInterp::Linear),
            key(1.0, SideInterp::Linear),
            key(2.0, SideInterp::Linear),
        ];
        let out = shift_keys_time(&keys, &[rational_at(1.0)], 0.5, 30.0);
        let times: Vec<f64> = out.iter().map(|k| k.time.to_f64()).collect();
        assert_eq!(times, vec![0.0, 1.5, 2.0]);
    }

    // A key dragged onto another key's time collapses to one (the collision rule
    // the graph editor uses), never a duplicate-time pair.
    #[test]
    fn shift_dedups_on_collision() {
        let keys = [
            key(0.0, SideInterp::Linear),
            key(1.0, SideInterp::Linear),
            key(2.0, SideInterp::Linear),
        ];
        let out = shift_keys_time(&keys, &[rational_at(1.0)], 1.0, 30.0);
        assert_eq!(out.len(), 2);
        assert!(out
            .iter()
            .all(|k| k.time.to_f64() == 0.0 || k.time.to_f64() == 2.0));
    }

    // Time never goes negative, and bezier handles ride along with the key.
    #[test]
    fn shift_clamps_and_keeps_handles() {
        let bez = SideInterp::Bezier {
            speed: 3.5,
            influence: 0.4,
        };
        let keys = [key(0.5, bez)];
        let out = shift_keys_time(&keys, &[rational_at(0.5)], -2.0, 30.0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].time.to_f64(), 0.0);
        assert_eq!(out[0].interp_in, bez);
        assert_eq!(out[0].interp_out, bez);
    }

    fn sel(t: f64) -> LaneKeySel {
        LaneKeySel {
            layer: uuid::Uuid::nil(),
            row: PropRow::Transform(TransformProp::Rotation),
            time: rational_at(t),
        }
    }

    #[test]
    fn plain_click_replaces_the_selection() {
        let mut s = vec![sel(1.0), sel(2.0)];
        lane_select_click(&mut s, sel(3.0), egui::Modifiers::default());
        assert_eq!(s, vec![sel(3.0)]);
    }

    #[test]
    fn ctrl_click_toggles_membership() {
        let mut s = vec![sel(1.0)];
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        lane_select_click(&mut s, sel(2.0), ctrl); // add
        assert_eq!(s, vec![sel(1.0), sel(2.0)]);
        lane_select_click(&mut s, sel(1.0), ctrl); // remove
        assert_eq!(s, vec![sel(2.0)]);
    }

    #[test]
    fn shift_click_toggles_membership_like_ctrl() {
        // UI-5: Shift now toggles too, so it can deselect (it used to only add).
        let mut s = vec![sel(1.0)];
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        lane_select_click(&mut s, sel(2.0), shift); // add
        assert_eq!(s, vec![sel(1.0), sel(2.0)]);
        lane_select_click(&mut s, sel(2.0), shift); // already in — removes it
        assert_eq!(s, vec![sel(1.0)]);
    }

    fn psel(prop: TransformProp) -> crate::app_state::PropSel {
        crate::app_state::PropSel {
            layer: uuid::Uuid::nil(),
            row: PropRow::Transform(prop),
        }
    }

    // Shift-click ranges over the drawn order between anchor and target,
    // inclusive, whichever way round they sit (note 2.6b).
    #[test]
    fn prop_range_covers_the_rows_between() {
        let order = vec![
            psel(TransformProp::AnchorX),
            psel(TransformProp::PositionX),
            psel(TransformProp::ScaleX),
            psel(TransformProp::Rotation),
            psel(TransformProp::Opacity),
        ];
        let (range, to_anchor) = prop_range(
            &order,
            Some(psel(TransformProp::PositionX)),
            psel(TransformProp::Rotation),
        );
        assert!(!to_anchor);
        assert_eq!(
            range,
            vec![
                psel(TransformProp::PositionX),
                psel(TransformProp::ScaleX),
                psel(TransformProp::Rotation),
            ]
        );
        // Reversed (target above the anchor) gives the same inclusive span.
        let (range_rev, _) = prop_range(
            &order,
            Some(psel(TransformProp::Rotation)),
            psel(TransformProp::PositionX),
        );
        assert_eq!(range_rev.len(), 3);
        assert_eq!(range_rev.first(), Some(&psel(TransformProp::PositionX)));
    }

    // No usable anchor: Shift-click falls back to selecting just the target.
    #[test]
    fn prop_range_without_anchor_selects_target() {
        let order = vec![psel(TransformProp::Rotation)];
        let (range, to_anchor) = prop_range(&order, None, psel(TransformProp::Rotation));
        assert!(to_anchor);
        assert_eq!(range, vec![psel(TransformProp::Rotation)]);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod prop_select_gesture_tests {
    //! The shared list-select gesture (`prop_click_select`) drives transform,
    //! effect and Retime rows alike (UI-6), so a mixed selection is possible.
    use super::*;
    use crate::app_state::{PropRow, PropSel};
    use lumit_core::model::TransformProp;

    fn tf(prop: TransformProp) -> PropSel {
        PropSel {
            layer: uuid::Uuid::nil(),
            row: PropRow::Transform(prop),
        }
    }
    fn eff(effect: usize, param: usize) -> PropSel {
        PropSel {
            layer: uuid::Uuid::nil(),
            row: PropRow::Effect { effect, param },
        }
    }
    fn retime() -> PropSel {
        PropSel {
            layer: uuid::Uuid::nil(),
            row: PropRow::Retime,
        }
    }
    fn ctrl() -> egui::Modifiers {
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        }
    }
    fn shift() -> egui::Modifiers {
        egui::Modifiers {
            shift: true,
            ..Default::default()
        }
    }

    // A plain click on an effect row single-selects it (resetting the set) and
    // reports the plain click so the caller can open its curve.
    #[test]
    fn plain_click_on_an_effect_row_single_selects() {
        let mut anchor = Some(tf(TransformProp::Rotation));
        let mut set = vec![tf(TransformProp::Rotation), eff(0, 0)];
        let mut target = None;
        let plain = prop_click_select(
            &mut anchor,
            &mut set,
            &mut target,
            eff(1, 2),
            egui::Modifiers::default(),
        );
        assert!(
            plain,
            "a plain click reports true (open curve / single select)"
        );
        assert_eq!(set, vec![eff(1, 2)]);
        assert_eq!(anchor, Some(eff(1, 2)));
        assert_eq!(target, None);
    }

    // Ctrl-click toggles an effect row's membership without disturbing the rest.
    #[test]
    fn ctrl_click_toggles_an_effect_row() {
        let mut anchor = Some(tf(TransformProp::PositionX));
        let mut set = vec![tf(TransformProp::PositionX)];
        let mut target = None;
        // Add the effect row.
        assert!(!prop_click_select(
            &mut anchor,
            &mut set,
            &mut target,
            eff(0, 0),
            ctrl()
        ));
        assert_eq!(set, vec![tf(TransformProp::PositionX), eff(0, 0)]);
        assert_eq!(anchor, Some(eff(0, 0)));
        // Ctrl-click it again removes it.
        assert!(!prop_click_select(
            &mut anchor,
            &mut set,
            &mut target,
            eff(0, 0),
            ctrl()
        ));
        assert_eq!(set, vec![tf(TransformProp::PositionX)]);
    }

    // Shift-click an effect row marks it as the range target (resolved later
    // against the draw order), never a plain click.
    #[test]
    fn shift_click_an_effect_row_sets_the_range_target() {
        let mut anchor = Some(tf(TransformProp::PositionX));
        let mut set = vec![tf(TransformProp::PositionX)];
        let mut target = None;
        let plain = prop_click_select(&mut anchor, &mut set, &mut target, eff(2, 1), shift());
        assert!(!plain);
        assert_eq!(target, Some(eff(2, 1)));
    }

    // The Retime row selects exactly like the other row types: plain click
    // single-selects it; Ctrl-click adds it to a mixed selection.
    #[test]
    fn the_retime_row_selects_like_the_others() {
        let mut anchor = None;
        let mut set = Vec::new();
        let mut target = None;
        let plain = prop_click_select(
            &mut anchor,
            &mut set,
            &mut target,
            retime(),
            egui::Modifiers::default(),
        );
        assert!(plain);
        assert_eq!(set, vec![retime()]);
        assert_eq!(anchor, Some(retime()));
        // Ctrl-click a transform row: a mixed transform + Retime selection.
        prop_click_select(
            &mut anchor,
            &mut set,
            &mut target,
            tf(TransformProp::Opacity),
            ctrl(),
        );
        assert_eq!(set, vec![retime(), tf(TransformProp::Opacity)]);
    }

    // A Ctrl-built selection can mix all three row kinds at once.
    #[test]
    fn a_mixed_selection_spans_transform_effect_and_retime() {
        let mut anchor = None;
        let mut set = Vec::new();
        let mut target = None;
        for sel in [tf(TransformProp::PositionX), eff(0, 0), retime()] {
            prop_click_select(&mut anchor, &mut set, &mut target, sel, ctrl());
        }
        assert_eq!(set, vec![tf(TransformProp::PositionX), eff(0, 0), retime()]);
    }

    // A Shift-range only spans within one section (T7): a Shift-click whose
    // target is in a different effect (or a different kind) than the anchor just
    // picks the target; a range within one effect covers its rows.
    #[test]
    fn range_stops_at_section_boundaries() {
        let order = vec![
            retime(),
            tf(TransformProp::PositionX),
            eff(0, 0),
            eff(0, 1),
            eff(1, 0),
        ];
        // Anchor in the Retime section, target in effect 0 -> just the target.
        let (range, to_anchor) = prop_range(&order, Some(retime()), eff(0, 0));
        assert_eq!(range, vec![eff(0, 0)]);
        assert!(to_anchor);
        // Anchor and target both in effect 0 -> the range within that effect.
        let (range, _) = prop_range(&order, Some(eff(0, 0)), eff(0, 1));
        assert_eq!(range, vec![eff(0, 0), eff(0, 1)]);
        // Anchor in effect 0, target in effect 1 -> just the target (no sweep).
        let (range, _) = prop_range(&order, Some(eff(0, 0)), eff(1, 0));
        assert_eq!(range, vec![eff(1, 0)]);
        // Two transform props share the transform section -> they range.
        let order2 = vec![tf(TransformProp::PositionX), tf(TransformProp::Rotation)];
        let (range, _) = prop_range(
            &order2,
            Some(tf(TransformProp::PositionX)),
            tf(TransformProp::Rotation),
        );
        assert_eq!(
            range,
            vec![tf(TransformProp::PositionX), tf(TransformProp::Rotation)]
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod keyframe_navigator_tests {
    //! The decision core shared by the one consolidated ◄ ◆ ► navigator now used
    //! by transform, linked-pair, Retime and effect rows alike (owner parity
    //! rule): which key a prev/next arrow jumps to, and whether a key sits at the
    //! playhead (so the diamond reads add vs remove), with the half-frame
    //! tolerance every row used to reimplement.
    use super::*;

    #[test]
    fn between_keys_points_at_the_neighbours() {
        let times = [0.0, 1.0, 2.0];
        let tol = 0.5 / 30.0;
        let (prev, on, next) = key_nav_targets(&times, 1.4, tol);
        assert_eq!(prev, Some(1.0));
        assert!(!on, "1.4 s is not on a key");
        assert_eq!(next, Some(2.0));
    }

    #[test]
    fn within_half_a_frame_reads_as_on_the_key() {
        let times = [0.0, 1.0, 2.0];
        let tol = 0.5 / 30.0;
        let (prev, on, next) = key_nav_targets(&times, 1.0 + tol * 0.4, tol);
        assert!(on, "just inside the tolerance counts as on the key");
        assert_eq!(prev, Some(0.0), "the on-key itself is not the prev target");
        assert_eq!(next, Some(2.0));
    }

    #[test]
    fn before_first_and_after_last_have_no_wrap() {
        let times = [0.0, 1.0, 2.0];
        let tol = 0.5 / 30.0;
        let (prev, on, next) = key_nav_targets(&times, -1.0, tol);
        assert_eq!(prev, None);
        assert!(!on);
        assert_eq!(next, Some(0.0));
        let (prev, _, next) = key_nav_targets(&times, 5.0, tol);
        assert_eq!(prev, Some(2.0));
        assert_eq!(next, None);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod motion_blur_switch_tests {
    use super::*;
    use crate::theme::{ColorScheme, ThemeShape};
    use lumit_core::model::Layer;
    use uuid::Uuid;

    /// Click the motion-blur switch (drawn into a known slot) and return the op it
    /// commits, if any.
    fn click_motion_blur(comp_id: Uuid, layer: &Layer) -> Option<lumit_core::Op> {
        let ctx = egui::Context::default();
        let theme = Theme::for_scheme(ColorScheme::ALL[0], ThemeShape::Sharp);
        let pending: std::cell::RefCell<Option<lumit_core::Op>> = std::cell::RefCell::new(None);
        let slot = egui::Rect::from_min_size(egui::pos2(50.0, 50.0), egui::vec2(40.0, 16.0));
        let run = |events: Vec<egui::Event>| {
            let ri = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(200.0, 200.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(ri, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(slot)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    motion_blur_control(
                        &mut child,
                        &theme,
                        comp_id,
                        layer,
                        &mut pending.borrow_mut(),
                    );
                });
            });
        };
        let c = slot.center();
        run(vec![]); // lay out
        run(vec![egui::Event::PointerMoved(c)]);
        run(vec![egui::Event::PointerButton {
            pos: c,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        }]);
        run(vec![egui::Event::PointerButton {
            pos: c,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        }]);
        pending.into_inner()
    }

    /// UI-12 regression: the per-layer motion-blur switch must draw and, when
    /// clicked, flip the layer's `motion_blur` flag (committing through
    /// `SetLayerMotionBlur`, so it persists). Driving it end-to-end through
    /// `AppState` also proves the op reaches the document.
    #[test]
    fn clicking_the_switch_toggles_the_layers_motion_blur_flag() {
        let mut app = AppState::default();
        app.new_composition();
        app.confirm_comp_dialog();
        app.add_solid_layer();
        let comp_id = app.selected_comp.unwrap();
        let layer = app.store.snapshot().comp(comp_id).unwrap().layers[0].clone();
        assert!(
            !layer.switches.motion_blur,
            "a fresh layer starts with motion blur off"
        );

        // Off -> the click commits an op turning it on.
        let op = click_motion_blur(comp_id, &layer).expect("the switch must emit an op");
        assert!(matches!(
            op,
            lumit_core::Op::SetLayerMotionBlur {
                motion_blur: true,
                ..
            }
        ));
        app.commit(op);
        let after = app.store.snapshot().comp(comp_id).unwrap().layers[0].clone();
        assert!(
            after.switches.motion_blur,
            "committing the switch op must set the flag"
        );

        // On -> clicking again turns it back off.
        let op = click_motion_blur(comp_id, &after).expect("the switch must emit an op");
        assert!(matches!(
            op,
            lumit_core::Op::SetLayerMotionBlur {
                motion_blur: false,
                ..
            }
        ));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod channel_picker_tests {
    use super::*;
    use lumit_core::fx::ParamKind;

    // The three-colour channel picker (P2/K-143) finds its group by the stable
    // `channel_colour_1/2/3` ids. Chromatic aberration (K-144) and RGB split
    // (K-161, T17) both adopt it: each schema must declare exactly those three
    // ids as Colour params with red / green / blue defaults, or the picker
    // silently stops finding them (and the classic split defaults break).
    fn assert_declares_channel_picker_group(match_name: &str) {
        let schema = lumit_core::fx::schema(match_name).unwrap();
        let defaults = [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        ];
        for (id, want) in CHANNEL_COLOUR_IDS.iter().zip(defaults.iter()) {
            let ps = schema
                .params
                .iter()
                .find(|p| &p.id == id)
                .unwrap_or_else(|| panic!("{match_name}: missing channel colour param {id}"));
            match ps.kind {
                ParamKind::Colour { default, .. } => {
                    assert_eq!(&default, want, "{match_name}: {id} default")
                }
                _ => panic!("{match_name}: {id} must be a Colour parameter"),
            }
        }
    }

    #[test]
    fn chromatic_aberration_declares_the_channel_picker_group() {
        assert_declares_channel_picker_group("chromatic_aberration");
    }

    #[test]
    fn rgb_split_declares_the_channel_picker_group() {
        assert_declares_channel_picker_group("rgb_split");
    }
}

/// T14 combined X/Y rows.
#[cfg(test)]
mod xy_row_tests {
    use crate::shell::inspector::effect_rows::xy_label;

    #[test]
    fn xy_label_capitalises_and_despaces() {
        assert_eq!(xy_label("centre"), "Centre");
        assert_eq!(xy_label("position"), "Position");
        assert_eq!(xy_label("anchor"), "Anchor");
    }

    /// The effects the combined X/Y row targets must declare matching `_x`/`_y`
    /// Float pairs, or the pairing silently falls back to two separate rows.
    #[test]
    fn effects_declare_matching_xy_pairs() {
        use lumit_core::fx::ParamKind;
        let has_float = |name: &str, id: &str| {
            lumit_core::fx::schema(name)
                .unwrap()
                .params
                .iter()
                .any(|p| p.id == id && matches!(p.kind, ParamKind::Float { .. }))
        };
        for (name, base) in [
            ("radial_blur", "centre"),
            ("transform", "anchor"),
            ("transform", "position"),
            ("transform", "scale"),
        ] {
            assert!(has_float(name, &format!("{base}_x")), "{name}: {base}_x");
            assert!(has_float(name, &format!("{base}_y")), "{name}: {base}_y");
        }
    }
}
