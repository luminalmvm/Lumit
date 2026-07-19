//! Unit tests for `AppState` (moved verbatim from app_state.rs).

use super::*;

fn kf(t: f64, interp: lumit_core::anim::SideInterp) -> lumit_core::anim::Keyframe {
    lumit_core::anim::Keyframe {
        time: Rational::from_f64_on_grid(t, Rational::FLICK_DEN).unwrap(),
        value: 0.0,
        interp_in: interp,
        interp_out: interp,
    }
}

// Paste overwrites the existing key at a coinciding time and carries the
// pasted key's bezier handles (note 2.2).
#[test]
fn paste_overwrites_coincident_key_and_keeps_handles() {
    use lumit_core::anim::SideInterp;
    let bez = SideInterp::Bezier {
        speed: 2.0,
        influence: 0.4,
    };
    let existing = [
        kf(0.0, SideInterp::Linear),
        kf(1.0, SideInterp::Linear),
        kf(2.0, SideInterp::Linear),
    ];
    let pasted = [kf(1.0, bez)];
    let out = merge_paste_keys(&existing, &pasted, 0.5 / 30.0);
    assert_eq!(out.len(), 3);
    let at_one = out
        .iter()
        .find(|k| (k.time.to_f64() - 1.0).abs() < 1e-6)
        .unwrap();
    // The pasted key won the collision, carrying its handles.
    assert_eq!(at_one.interp_in, bez);
    assert_eq!(at_one.interp_out, bez);
}

// Pasting at a fresh time inserts without disturbing the others.
#[test]
fn paste_at_new_time_inserts() {
    use lumit_core::anim::SideInterp::Linear;
    let existing = [kf(0.0, Linear), kf(1.0, Linear)];
    let pasted = [kf(3.0, Linear)];
    let out = merge_paste_keys(&existing, &pasted, 0.5 / 30.0);
    let ts: Vec<f64> = out.iter().map(|k| k.time.to_f64()).collect();
    assert_eq!(ts, vec![0.0, 1.0, 3.0]);
}

#[test]
fn linked_axis_partner_maps_x_to_y_only() {
    use lumit_core::model::TransformProp;
    assert_eq!(
        linked_axis_partner(TransformProp::ScaleX),
        Some(TransformProp::ScaleY)
    );
    assert_eq!(linked_axis_partner(TransformProp::Opacity), None);
}

#[test]
fn draft_width_caps_for_instant_scrub_but_never_exceeds_specified() {
    // Full res, dragging: capped at the draft width for a fast decode.
    assert_eq!(decode_target_width(1920, true, false, 1.0, 1), Some(640));
    // Draft never coarser than needed: half res (960) already below no cap,
    // still above 640 -> draft caps to 640.
    assert_eq!(decode_target_width(1920, true, false, 1.0, 2), Some(640));
    // Quarter res (480) is finer than the draft cap: keep 480, don't raise.
    assert_eq!(decode_target_width(1920, true, false, 1.0, 4), Some(480));
    // Auto res zoomed right out (192) stays 192 under draft.
    assert_eq!(decode_target_width(1920, true, true, 0.1, 1), Some(192));
    // A source already smaller than the cap needs no draft decode.
    assert_eq!(decode_target_width(320, true, false, 1.0, 1), None);
}

#[test]
fn fill_walk_is_forward_biased_and_complete() {
    let order = fill_walk_order(5, 0, 10);
    assert_eq!(order[0], 5); // the playhead caches first
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..10).collect::<Vec<_>>()); // every frame once
                                                     // Of the four frames right after the playhead, at least three are ahead.
    let ahead = order[1..5].iter().filter(|&&f| f > 5).count();
    assert!(ahead >= 3, "expected a forward bias: {order:?}");
    // Playhead at the work-area start: everything is ahead, no panic.
    assert_eq!(fill_walk_order(0, 0, 4), vec![0, 1, 2, 3]);
    // Degenerate spans return cleanly.
    assert_eq!(fill_walk_order(0, 0, 1), vec![0]);
    assert!(fill_walk_order(0, 0, 0).is_empty());
}

#[test]
fn playback_lookahead_is_a_bounded_forward_window() {
    // Strictly forward, starting just past the playhead.
    assert_eq!(playback_lookahead(5, 100, 4), vec![6, 7, 8, 9]);
    // Clamps to the (exclusive) work-area end.
    assert_eq!(playback_lookahead(8, 10, 4), vec![9]);
    // Empty at or past the end, and with a zero lookahead.
    assert!(playback_lookahead(9, 10, 4).is_empty());
    assert!(playback_lookahead(10, 10, 4).is_empty());
    assert!(playback_lookahead(5, 100, 0).is_empty());
}

#[test]
fn specified_width_is_unchanged_when_not_drafting() {
    assert_eq!(decode_target_width(1920, false, false, 1.0, 1), None);
    assert_eq!(decode_target_width(1920, false, false, 1.0, 2), Some(960));
    assert_eq!(decode_target_width(1000, false, true, 0.5, 1), Some(500));
}

/// K-068: solids are assets auto-filed into a "Solids" folder that is
/// followed by id (rename it, it still collects); comps auto-file into
/// "Compositions"; each creation is one undo step.
#[test]
fn pan_behind_keeps_the_layer_fixed() {
    // No rotation, 100% scale: position tracks the anchor 1:1.
    let p = pan_behind_position(
        (50.0, 50.0),
        (60.0, 50.0),
        (100.0, 100.0),
        (100.0, 100.0),
        0.0,
    );
    assert!((p.0 - 110.0).abs() < 1e-9 && (p.1 - 100.0).abs() < 1e-9);
    // 200% scale doubles the position shift for the same anchor move.
    let p = pan_behind_position((0.0, 0.0), (10.0, 0.0), (0.0, 0.0), (200.0, 200.0), 0.0);
    assert!((p.0 - 20.0).abs() < 1e-9 && p.1.abs() < 1e-9);
    // 90° rotation sends an x-move of the anchor into +y of position.
    let p = pan_behind_position((0.0, 0.0), (10.0, 0.0), (0.0, 0.0), (100.0, 100.0), 90.0);
    assert!(p.0.abs() < 1e-9 && (p.1 - 10.0).abs() < 1e-9);
}

#[test]
fn centred_transform_puts_origin_at_object_centre() {
    // A 1920×1080 object in a 1280×720 comp: anchor at the object's
    // centre, position at the comp's centre (AE default).
    let tr = centred_transform(1920.0, 1080.0, 1280, 720);
    assert_eq!(tr.anchor_x.value_at(0.0), 960.0);
    assert_eq!(tr.anchor_y.value_at(0.0), 540.0);
    assert_eq!(tr.position_x.value_at(0.0), 640.0);
    assert_eq!(tr.position_y.value_at(0.0), 360.0);
    // Scale/rotation stay neutral so only the origin/position changed.
    assert_eq!(tr.scale_x.value_at(0.0), 100.0);
    assert_eq!(tr.rotation.value_at(0.0), 0.0);
}

#[test]
fn auto_folders_collect_solids_and_comps() {
    let mut app = AppState::default();
    app.new_composition();
    app.confirm_comp_dialog();
    let doc = app.store.snapshot();
    let comps_folder = doc.auto_folders.compositions.expect("comps folder");
    assert_eq!(doc.folder(comps_folder).unwrap().children.len(), 1);

    app.add_solid_layer();
    let doc = app.store.snapshot();
    let solids_folder = doc.auto_folders.solids.expect("solids folder");
    let first_children = doc.folder(solids_folder).unwrap().children.clone();
    assert_eq!(first_children.len(), 1);
    assert!(doc.solid(first_children[0]).is_some());

    // Rename the folder: the habit follows the id, not the name.
    app.commit(Op::RenameItem {
        id: solids_folder,
        name: "My colours".into(),
    });
    app.add_solid_layer();
    let doc = app.store.snapshot();
    assert_eq!(doc.folder(solids_folder).unwrap().children.len(), 2);
    assert_eq!(doc.folder(solids_folder).unwrap().name, "My colours");

    // One undo removes the whole second solid creation (batch), and the
    // layer count in the comp drops with it.
    let comp_id = app.selected_comp.unwrap();
    assert_eq!(doc.comp(comp_id).unwrap().layers.len(), 2);
    app.undo();
    let doc = app.store.snapshot();
    assert_eq!(doc.folder(solids_folder).unwrap().children.len(), 1);
    assert_eq!(doc.comp(comp_id).unwrap().layers.len(), 1);

    // Deleting the folder recreates it on next use (fresh id).
    app.commit(Op::RemoveItem { id: solids_folder });
    app.add_solid_layer();
    let doc = app.store.snapshot();
    let new_folder = doc.auto_folders.solids.unwrap();
    assert_ne!(new_folder, solids_folder);
    assert_eq!(doc.folder(new_folder).unwrap().children.len(), 1);

    // Move-to-folder: filing a solid under Compositions then back to root.
    let solid_id = doc.folder(new_folder).unwrap().children[0];
    app.move_item_to_folder(solid_id, Some(comps_folder));
    let doc = app.store.snapshot();
    assert!(doc
        .folder(comps_folder)
        .unwrap()
        .children
        .contains(&solid_id));
    assert!(!doc.folder(new_folder).unwrap().children.contains(&solid_id));
    app.move_item_to_folder(solid_id, None);
    let doc = app.store.snapshot();
    assert!(doc.root_items().contains(&solid_id));

    // A folder cannot be filed into its own subtree.
    app.move_item_to_folder(comps_folder, Some(comps_folder));
    let doc = app.store.snapshot();
    assert!(!doc
        .folder(comps_folder)
        .unwrap()
        .children
        .contains(&comps_folder));
}

#[test]
fn duplicate_layer_makes_a_fresh_copy_above_the_original_and_undoes() {
    let mut app = AppState::default();
    app.new_composition();
    app.confirm_comp_dialog();
    app.add_solid_layer();
    let comp_id = app.selected_comp.unwrap();
    let orig = app.store.snapshot().comp(comp_id).unwrap().layers[0].id;
    app.selected_layer = Some(orig);

    app.duplicate_layer();
    let doc = app.store.snapshot();
    let layers = &doc.comp(comp_id).unwrap().layers;
    assert_eq!(layers.len(), 2, "a copy was added");
    // The copy has a fresh id, is now selected, and its name is derived.
    let new_id = app.selected_layer.unwrap();
    assert_ne!(new_id, orig);
    let original = layers.iter().find(|l| l.id == orig).unwrap();
    let copy = layers.iter().find(|l| l.id == new_id).unwrap();
    assert_eq!(copy.name, format!("{} copy", original.name));
    // It sits directly above the original (a lower index — index 0 is top).
    let ci = layers.iter().position(|l| l.id == new_id).unwrap();
    let oi = layers.iter().position(|l| l.id == orig).unwrap();
    assert_eq!(ci + 1, oi, "the copy is directly above the original");
    // One undo removes the duplicate.
    app.undo();
    assert_eq!(app.store.snapshot().comp(comp_id).unwrap().layers.len(), 1);
}

#[test]
fn delete_selected_layer_removes_it_and_undoes() {
    let mut app = AppState::default();
    app.new_composition();
    app.confirm_comp_dialog();
    app.add_solid_layer();
    let comp_id = app.selected_comp.unwrap();
    let layer = app.store.snapshot().comp(comp_id).unwrap().layers[0].id;
    app.selected_layer = Some(layer);

    app.delete_selected_layer();
    assert_eq!(
        app.store.snapshot().comp(comp_id).unwrap().layers.len(),
        0,
        "the layer is gone"
    );
    assert_eq!(app.selected_layer, None);
    app.undo();
    assert_eq!(
        app.store.snapshot().comp(comp_id).unwrap().layers.len(),
        1,
        "undo brings it back"
    );
}

/// K-068: the dialogue edits an existing comp's settings invertibly.
#[test]
fn comp_settings_dialog_edits_and_undoes() {
    let mut app = AppState::default();
    app.new_composition();
    app.confirm_comp_dialog();
    let comp_id = app.selected_comp.unwrap();

    app.open_comp_settings(comp_id);
    {
        let d = app.comp_dialog.as_mut().unwrap();
        assert_eq!(d.editing, Some(comp_id));
        assert_eq!((d.width, d.height), (1920, 1080));
        d.width = 1280;
        d.height = 720;
        d.fps = 23.976;
        d.name = "Retitled".into();
    }
    app.confirm_comp_dialog();
    let doc = app.store.snapshot();
    let comp = doc.comp(comp_id).unwrap();
    assert_eq!((comp.width, comp.height), (1280, 720));
    assert_eq!(comp.name, "Retitled");
    // NTSC snap: 23.976 becomes exactly 24000/1001.
    assert!((comp.frame_rate.fps() - 24000.0 / 1001.0).abs() < 1e-9);
    app.undo();
    let doc = app.store.snapshot();
    assert_eq!(doc.comp(comp_id).unwrap().width, 1920);
}

/// Regression: a freshly created composition is the active one, so the
/// next item dropped in lands in it — not in a comp opened earlier. The
/// bug was `preview_comp` (the add target) lagging behind `selected_comp`
/// after a second comp was created.
#[test]
fn importing_footage_selects_it_and_requests_the_project_tab() {
    // UI-13: an import highlights the new item in the Project panel and asks the
    // shell to bring the Project tab to the front.
    let mut app = AppState::default();
    app.import_paths(vec![
        std::path::PathBuf::from("a.mp4"),
        std::path::PathBuf::from("b.mp4"),
    ]);
    let last = app
        .store
        .snapshot()
        .items
        .iter()
        .rev()
        .find_map(|i| match i {
            ProjectItem::Footage(f) => Some(f.id),
            _ => None,
        })
        .unwrap();
    assert_eq!(app.selected_item, Some(last), "the last import is selected");
    assert!(
        app.focus_project_tab,
        "an import should request the Project tab"
    );
}

#[test]
fn a_new_composition_becomes_the_active_add_target() {
    let mut app = AppState::default();

    // First comp, with one footage layer.
    app.new_composition();
    app.confirm_comp_dialog();
    let comp1 = app.selected_comp.unwrap();
    app.import_paths(vec![std::path::PathBuf::from("clip.mp4")]);
    let footage = app
        .store
        .snapshot()
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Footage(f) => Some(f.id),
            _ => None,
        })
        .unwrap();
    app.add_item_to_comp(footage);
    assert_eq!(app.store.snapshot().comp(comp1).unwrap().layers.len(), 1);

    // Second comp: creating it makes it the active comp everywhere.
    app.new_composition();
    app.confirm_comp_dialog();
    let comp2 = app.selected_comp.unwrap();
    assert_ne!(comp1, comp2);
    assert_eq!(
        app.preview_comp,
        Some(comp2),
        "a new comp should also become the viewed/edited comp"
    );

    // The next add must land in comp2, and must not touch comp1.
    app.add_item_to_comp(footage);
    let doc = app.store.snapshot();
    assert_eq!(
        doc.comp(comp2).unwrap().layers.len(),
        1,
        "layer must land in the newly created composition"
    );
    assert_eq!(
        doc.comp(comp1).unwrap().layers.len(),
        1,
        "the earlier composition must not receive the new layer"
    );
}

/// 07-UI-SPEC §4: the Timeline keeps one tab per open comp. Creating comps
/// opens their tabs; the active tab follows the newest; closing a tab hands
/// the active comp to a neighbour and never deletes the comp itself.
#[test]
fn comps_open_as_timeline_tabs_and_close_cleanly() {
    let mut app = AppState::default();
    app.new_composition();
    app.confirm_comp_dialog();
    let comp1 = app.selected_comp.unwrap();
    app.new_composition();
    app.confirm_comp_dialog();
    let comp2 = app.selected_comp.unwrap();

    // Both comps are open; the newest is active.
    assert_eq!(app.open_comps, vec![comp1, comp2]);
    assert_eq!(app.selected_comp, Some(comp2));
    assert_eq!(app.preview_comp, Some(comp2));

    // Switching back to the first comp's tab re-activates it without
    // re-opening (no duplicate tab).
    app.open_comp(comp1);
    assert_eq!(app.open_comps, vec![comp1, comp2]);
    assert_eq!(app.selected_comp, Some(comp1));

    // Closing the active tab hands off to its neighbour; the comp survives.
    app.close_comp_tab(comp1);
    assert_eq!(app.open_comps, vec![comp2]);
    assert_eq!(app.selected_comp, Some(comp2));
    assert!(app.store.snapshot().comp(comp1).is_some());

    // Closing the last tab empties the Timeline.
    app.close_comp_tab(comp2);
    assert!(app.open_comps.is_empty());
    assert_eq!(app.selected_comp, None);
    assert_eq!(app.preview_comp, None);

    // Deleting a comp also drops its tab if it happened to be open.
    app.open_comp(comp2);
    app.commit(Op::RemoveItem { id: comp2 });
    app.close_comp_tab(comp2);
    assert!(app.open_comps.is_empty());
}

/// The slice 3 drill: save, edit past the save, crash (drop without
/// saving), reopen — the journal restores every post-save change.
#[test]
fn kill_and_recover_drill() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drill.lum");

    let doc_id;
    let final_json;
    {
        let mut app = AppState::default();
        doc_id = app.store.snapshot().id;
        app.new_composition();
        app.confirm_comp_dialog();
        app.path = Some(path.clone());
        app.save();
        assert!(!app.dirty);

        // Edits after the save — journalled, never saved.
        app.new_composition();
        app.confirm_comp_dialog();
        app.new_composition();
        app.confirm_comp_dialog();
        assert!(app.dirty);
        final_json = serde_json::to_string(&*app.store.snapshot()).unwrap();
        // "kill -9": app dropped here with dirty state.
    }

    let mut app2 = AppState::default();
    app2.open_path(&path);
    let pending = app2.pending_recovery.as_ref().expect("recovery offered");
    assert_eq!(pending.ops.len(), 2);
    app2.resolve_recovery(true);
    assert_eq!(
        serde_json::to_string(&*app2.store.snapshot()).unwrap(),
        final_json,
        "recovered document equals the pre-crash document"
    );
    assert!(app2.dirty, "recovered state needs a save");

    // Saving clears the journal: a fresh open offers no recovery.
    app2.save();
    let mut app3 = AppState::default();
    app3.open_path(&path);
    assert!(app3.pending_recovery.is_none());

    let _ = JournalFile::for_document(doc_id).map(|j| j.clear());
}

#[test]
fn discarding_recovery_opens_last_save_and_clears_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drill2.lum");
    let saved_json;
    {
        let mut app = AppState::default();
        app.new_composition();
        app.confirm_comp_dialog();
        app.path = Some(path.clone());
        app.save();
        saved_json = serde_json::to_string(&*app.store.snapshot()).unwrap();
        app.new_composition(); // journalled, then "crash"
        app.confirm_comp_dialog();
    }
    let mut app2 = AppState::default();
    app2.open_path(&path);
    assert!(app2.pending_recovery.is_some());
    app2.resolve_recovery(false);
    assert_eq!(
        serde_json::to_string(&*app2.store.snapshot()).unwrap(),
        saved_json
    );
    let mut app3 = AppState::default();
    app3.open_path(&path);
    assert!(
        app3.pending_recovery.is_none(),
        "journal cleared on discard"
    );
}

/// Regression (owner bug): an effect-value drag in the layer area only updated
/// the preview on frames that had a keyframe at the playhead. Cause — a frame
/// served from the composite cache is presented without a decode, so it never
/// populates `last_comp`, which the live patch re-composites from; a keyframe at
/// the playhead invalidated the cache entry and forced the decode, masking the
/// bug. Fix: while a live value edit is active, `refresh_comp_preview` skips the
/// cache shortcut and decodes, so `last_comp` is populated.
#[cfg(feature = "media")]
#[test]
fn a_live_edit_decodes_instead_of_taking_the_composite_cache() {
    let mut app = AppState::default();
    app.new_composition();
    app.confirm_comp_dialog();
    app.add_solid_layer();
    let comp_id = app.selected_comp.unwrap();
    app.preview_comp = Some(comp_id);
    app.preview_frame = 0;

    // Warm the composite cache for the current frame.
    let key = app.frame_key_for(comp_id, 0).expect("frame key");
    app.comp_frame_cache.insert(
        key,
        CachedCompFrame {
            width: 8,
            height: 8,
            rgba: vec![0u8; 8 * 8 * 4],
        },
    );

    // No live edit: the cache hit presents from the cache (no decode).
    app.cached_present = None;
    app.refresh_comp_preview();
    assert_eq!(
        app.cached_present,
        Some(key),
        "a cache hit presents from the composite cache"
    );

    // A live edit active: it must NOT take the shortcut — it decodes so the
    // live patch has `last_comp` (before the fix this asserted Some(key)).
    app.cached_present = None;
    app.fx_edit = Some((comp_id, 0, 0, 1.0));
    app.refresh_comp_preview();
    assert_eq!(
        app.cached_present, None,
        "a live edit forces a decode rather than a cache hit"
    );
}

/// UI-7 end-to-end (logic level): copy a lane keyframe selection, move the
/// playhead, paste — the copied keys reappear at the new playhead preserving
/// their relative offsets, on the correct property, leaving the originals in
/// place. This exercises `copy_selected_keyframes` → `paste_keyframes` directly
/// (the shortcut routing is covered separately in `shell::shortcuts`).
#[test]
fn copy_move_paste_replays_keys_at_the_playhead() {
    use lumit_core::anim::{Animation, Keyframe, SideInterp};
    use lumit_core::model::TransformProp;
    let mut app = AppState::default();
    app.new_composition();
    app.confirm_comp_dialog();
    app.add_solid_layer();
    let comp_id = app.selected_comp.unwrap();
    let layer_id = app.store.snapshot().comp(comp_id).unwrap().layers[0].id;
    // Put two rotation keys at 1.0 and 2.0 (layer-local; start_offset = 0).
    let keys = vec![
        Keyframe {
            time: Rational::from_f64_on_grid(1.0, Rational::FLICK_DEN).unwrap(),
            value: 10.0,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        },
        Keyframe {
            time: Rational::from_f64_on_grid(2.0, Rational::FLICK_DEN).unwrap(),
            value: 20.0,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        },
    ];
    app.commit(Op::SetTransformProperty {
        comp: comp_id,
        layer: layer_id,
        prop: TransformProp::Rotation,
        animation: Animation::Keyframed(keys),
    });
    // Select both lane keys.
    app.lane_selection = vec![
        LaneKeySel {
            layer: layer_id,
            row: PropRow::Transform(TransformProp::Rotation),
            time: Rational::from_f64_on_grid(1.0, Rational::FLICK_DEN).unwrap(),
        },
        LaneKeySel {
            layer: layer_id,
            row: PropRow::Transform(TransformProp::Rotation),
            time: Rational::from_f64_on_grid(2.0, Rational::FLICK_DEN).unwrap(),
        },
    ];
    app.copy_selected_keyframes();
    assert_eq!(app.keyframe_clipboard.len(), 2, "both keys must be copied");
    // Move the playhead to 3.0 s (fps 60 => frame 180).
    let fps = app.store.snapshot().comp(comp_id).unwrap().frame_rate.fps();
    app.preview_frame = (3.0 * fps).round() as usize;
    app.paste_keyframes();
    let comp = app.store.snapshot();
    let layer = comp
        .comp(comp_id)
        .unwrap()
        .layers
        .iter()
        .find(|l| l.id == layer_id)
        .unwrap()
        .clone();
    let Animation::Keyframed(ks) = &layer.transform.get(TransformProp::Rotation).animation else {
        panic!("expected keyframed rotation");
    };
    let ts: Vec<f64> = ks.iter().map(|k| k.time.to_f64()).collect();
    // The two originals stay put; the two pasted keys land at 3.0 and 4.0
    // (playhead + each key's offset from the copy anchor).
    assert!(
        ts.iter().any(|t| (t - 1.0).abs() < 0.05),
        "the original key at 1.0 must remain: {ts:?}"
    );
    assert!(
        ts.iter().any(|t| (t - 2.0).abs() < 0.05),
        "the original key at 2.0 must remain: {ts:?}"
    );
    assert!(
        ts.iter().any(|t| (t - 3.0).abs() < 0.05),
        "expected a pasted key near 3.0: {ts:?}"
    );
    assert!(
        ts.iter().any(|t| (t - 4.0).abs() < 0.05),
        "expected a pasted key near 4.0: {ts:?}"
    );
}

/// UI-7 supporting fact: a lane-key glyph is drawn with `Sense::click_and_drag`,
/// which egui marks focusable — but selecting one (a click) does not leave it
/// holding keyboard focus. So `keyframe_clipboard_shortcuts`' "skip while a
/// widget is focused" guard is not tripped just because keys are selected, and
/// the copy/paste gesture still routes to the timeline.
#[test]
fn clicking_a_lane_key_glyph_does_not_hold_focus() {
    let ctx = egui::Context::default();
    let focused_after = std::cell::Cell::new(true);
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
                let rect =
                    egui::Rect::from_min_size(egui::pos2(50.0, 50.0), egui::vec2(12.0, 14.0));
                let _ = ui.interact(
                    rect,
                    egui::Id::new("lanekey"),
                    egui::Sense::click_and_drag(),
                );
            });
            focused_after.set(ctx.memory(|m| m.focused()).is_some());
        });
    };
    let p = egui::pos2(56.0, 57.0);
    run(vec![egui::Event::PointerMoved(p)]);
    run(vec![egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    }]);
    run(vec![egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    }]);
    // One more idle frame to settle focus.
    run(vec![]);
    assert!(
        !focused_after.get(),
        "selecting a lane key must not leave a widget holding focus"
    );
}

// --- GEN-4 audio: the comp mix must follow the current comp state ---------
//
// The four owner-reported bugs (mute does nothing, moving a layer does not move
// its audio, audio bleeds beyond the active span, a deleted layer keeps
// sounding) shared one cause: the comp mix was baked once and never re-derived
// from the document. These exercise the scheduler that decides *which* layers
// contribute (`comp_audio_jobs`) and the reconciliation that keeps a loaded mix
// in step (`comp_audio_sync` / `sync_comp_audio`) — device-free, since the
// gating is pure arithmetic over the document.

/// A composition with one audible footage layer (5 s of injected audio, no
/// real decode): returns the app plus the comp and layer ids.
#[cfg(feature = "media")]
fn app_with_audio_layer() -> (AppState, Uuid, Uuid) {
    use lumit_core::model::{FootageItem, MediaRef, ProjectItem};
    let mut app = AppState::default();
    app.new_composition();
    app.confirm_comp_dialog();
    let comp_id = app.selected_comp.unwrap();
    let item_id = Uuid::now_v7();
    app.commit(Op::AddItem {
        index: app.store.snapshot().items.len(),
        item: Box::new(ProjectItem::Footage(FootageItem {
            id: item_id,
            name: "clip.wav".into(),
            extra: serde_json::Map::new(),
            media: MediaRef {
                relative_path: "clip.wav".into(),
                absolute_path: "/tmp/clip.wav".into(),
                extra: serde_json::Map::new(),
            },
        })),
    });
    // A Ready probe with an audio stream — enough for `comp_audio_jobs` to place
    // the layer, without touching a file or a device.
    app.media.map.insert(
        item_id,
        media::MediaStatus::Ready {
            probe: lumit_media::MediaProbe {
                duration_seconds: 5.0,
                container: "wav".into(),
                video: None,
                audio: Some(lumit_media::AudioInfo {
                    sample_rate: 48_000,
                    channels: 2,
                    codec: "pcm".into(),
                }),
            },
            frames: 0,
            vfr: false,
        },
    );
    app.preview_comp = Some(comp_id);
    app.add_footage_to_comp(item_id);
    let layer_id = app.store.snapshot().comp(comp_id).unwrap().layers[0].id;
    (app, comp_id, layer_id)
}

/// Mark the current comp mix as loaded (as `poll_comp_audio` would), returning
/// its signature, so an edit afterwards can be seen to stale it.
#[cfg(feature = "media")]
fn mark_mix_loaded(app: &mut AppState, comp_id: Uuid) -> u64 {
    let doc = app.store.snapshot();
    let comp = doc.comp(comp_id).unwrap();
    let jobs = app.comp_audio_jobs(&doc, comp);
    let sig = super::audio_jobs_signature(&jobs, comp.duration.0.to_f64());
    drop(doc);
    app.audio_loaded_comp = Some(comp_id);
    app.audio_loaded_sig = Some(sig);
    sig
}

/// The reconciliation decision for the comp as it stands now.
#[cfg(feature = "media")]
fn audio_sync_decision(app: &AppState, comp_id: Uuid) -> AudioSync {
    let doc = app.store.snapshot();
    let comp = doc.comp(comp_id).unwrap();
    let jobs = app.comp_audio_jobs(&doc, comp);
    super::comp_audio_sync(
        app.audio_loaded_comp,
        app.audio_loaded_sig,
        app.audio_preparing,
        comp_id,
        &jobs,
        comp.duration.0.to_f64(),
    )
}

/// The comp audio job for the sole footage layer, if any.
#[cfg(feature = "media")]
fn only_audio_job(app: &AppState, comp_id: Uuid) -> Option<crate::export::AudioJob> {
    let doc = app.store.snapshot();
    let comp = doc.comp(comp_id).unwrap();
    app.comp_audio_jobs(&doc, comp).into_iter().next()
}

/// Bug 1: a muted layer must drop out of the mix. The mixer is fed from
/// `comp_audio_jobs`, so muting must remove the layer's contribution, and a
/// loaded mix must reconcile to silence.
#[cfg(feature = "media")]
#[test]
fn muting_an_audio_layer_drops_it_from_the_mix() {
    let (mut app, comp_id, layer_id) = app_with_audio_layer();
    assert!(only_audio_job(&app, comp_id).is_some(), "audible to start");

    mark_mix_loaded(&mut app, comp_id);
    assert_eq!(audio_sync_decision(&app, comp_id), AudioSync::UpToDate);

    app.commit(Op::SetLayerAudible {
        comp: comp_id,
        layer: layer_id,
        audible: false,
    });
    assert!(
        only_audio_job(&app, comp_id).is_none(),
        "a muted layer contributes nothing"
    );
    assert_eq!(
        audio_sync_decision(&app, comp_id),
        AudioSync::Silence,
        "the loaded mix must unload once its only audio is muted"
    );
}

/// Bug 1, wired: `sync_comp_audio` unloads a comp that muting has silenced, so
/// the engine stops sounding it (and playback continues on the wall clock).
#[cfg(feature = "media")]
#[test]
fn sync_unloads_a_comp_silenced_by_muting() {
    let (mut app, comp_id, layer_id) = app_with_audio_layer();
    mark_mix_loaded(&mut app, comp_id);
    app.comp_playback = Some((Instant::now(), 0));

    app.commit(Op::SetLayerAudible {
        comp: comp_id,
        layer: layer_id,
        audible: false,
    });
    app.sync_comp_audio();

    assert_eq!(app.audio_loaded_comp, None, "the silenced mix is unloaded");
    assert_eq!(app.audio_loaded_sig, None);
    assert!(
        app.comp_playback.is_some(),
        "playback keeps going on the wall clock"
    );
}

/// Bug 2: moving the layer in time must move its audio — the placement follows
/// the layer's in-point and start offset, so a loaded mix stales and re-bakes.
#[cfg(feature = "media")]
#[test]
fn moving_an_audio_layer_moves_its_audio_and_rebakes() {
    use lumit_core::time::CompTime;
    let (mut app, comp_id, layer_id) = app_with_audio_layer();
    let job = only_audio_job(&app, comp_id).unwrap();
    assert!((job.offset_s - 0.0).abs() < 1e-9 && (job.in_s - 0.0).abs() < 1e-9);

    let sig = mark_mix_loaded(&mut app, comp_id);
    // Slide the whole layer 2 s later (in, out and start offset together).
    let at = |t: f64| CompTime(Rational::from_f64_on_grid(t, 1000).unwrap());
    app.commit(Op::SetLayerSpan {
        comp: comp_id,
        layer: layer_id,
        in_point: at(2.0),
        out_point: at(7.0),
        start_offset: at(2.0),
    });

    let moved = only_audio_job(&app, comp_id).unwrap();
    assert!(
        (moved.offset_s - 2.0).abs() < 1e-9 && (moved.in_s - 2.0).abs() < 1e-9,
        "the audio now starts 2 s later: in={} offset={}",
        moved.in_s,
        moved.offset_s
    );
    match audio_sync_decision(&app, comp_id) {
        AudioSync::Rebake(new_sig) => assert_ne!(new_sig, sig, "the mix must change"),
        other => panic!("moving a layer should re-bake, got {other:?}"),
    }
}

/// Bug 3: audio must not sound outside the layer's active span. Trimming the
/// out-point shorter changes the placed span, so the loaded mix stales and the
/// re-bake confines the audio to the new span.
#[cfg(feature = "media")]
#[test]
fn trimming_an_audio_layer_confines_the_audio_to_the_new_span() {
    use lumit_core::time::CompTime;
    let (mut app, comp_id, layer_id) = app_with_audio_layer();
    let job = only_audio_job(&app, comp_id).unwrap();
    assert!((job.out_s - 5.0).abs() < 0.02, "out at ~5 s: {}", job.out_s);

    let sig = mark_mix_loaded(&mut app, comp_id);
    // Trim the out-point back to 2 s (in and offset unchanged).
    app.commit(Op::SetLayerSpan {
        comp: comp_id,
        layer: layer_id,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::from_f64_on_grid(2.0, 1000).unwrap()),
        start_offset: CompTime(Rational::ZERO),
    });

    let trimmed = only_audio_job(&app, comp_id).unwrap();
    assert!(
        (trimmed.out_s - 2.0).abs() < 1e-9,
        "the audible span shrank to [0, 2): {}",
        trimmed.out_s
    );
    match audio_sync_decision(&app, comp_id) {
        AudioSync::Rebake(new_sig) => assert_ne!(new_sig, sig),
        other => panic!("trimming a layer should re-bake, got {other:?}"),
    }
}

/// Bug 4: deleting the audio layer must stop its sound — with no audio layer
/// left, the comp is silent and the loaded mix reconciles to silence.
#[cfg(feature = "media")]
#[test]
fn deleting_the_audio_layer_silences_the_comp() {
    let (mut app, comp_id, layer_id) = app_with_audio_layer();
    mark_mix_loaded(&mut app, comp_id);

    app.selected_layer = Some(layer_id);
    app.delete_selected_layer();

    assert!(
        only_audio_job(&app, comp_id).is_none(),
        "the deleted layer no longer contributes"
    );
    assert_eq!(
        audio_sync_decision(&app, comp_id),
        AudioSync::Silence,
        "a comp with no audio layer must unload its mix"
    );
}

/// GEN-3 (K-153): importing a clip longer than the comp keeps its FULL media
/// duration — positioned from the comp start — instead of being trimmed to fit.
/// The comp window clips it at render time; the model keeps the whole layer, so
/// its tail is recoverable by sliding the layer across the comp end.
#[cfg(feature = "media")]
#[test]
fn a_long_import_keeps_its_full_media_duration() {
    use lumit_core::model::{FootageItem, MediaRef, ProjectItem};
    let mut app = AppState::default();
    app.new_composition();
    app.confirm_comp_dialog();
    let comp_id = app.selected_comp.unwrap();
    let comp_dur = app
        .store
        .snapshot()
        .comp(comp_id)
        .unwrap()
        .duration
        .0
        .to_f64();
    let item_id = Uuid::now_v7();
    app.commit(Op::AddItem {
        index: app.store.snapshot().items.len(),
        item: Box::new(ProjectItem::Footage(FootageItem {
            id: item_id,
            name: "long.mp4".into(),
            extra: serde_json::Map::new(),
            media: MediaRef {
                relative_path: "long.mp4".into(),
                absolute_path: "/tmp/long.mp4".into(),
                extra: serde_json::Map::new(),
            },
        })),
    });
    // A Ready probe far longer than the comp (the default comp is 30 s).
    let media_dur = comp_dur + 20.0;
    app.media.map.insert(
        item_id,
        media::MediaStatus::Ready {
            probe: lumit_media::MediaProbe {
                duration_seconds: media_dur,
                container: "mp4".into(),
                video: Some(lumit_media::VideoInfo {
                    width: 1920,
                    height: 1080,
                    fps_num: 60,
                    fps_den: 1,
                    codec: "h264".into(),
                }),
                audio: None,
            },
            frames: (media_dur * 60.0).round() as usize,
            vfr: false,
        },
    );
    app.preview_comp = Some(comp_id);
    app.add_footage_to_comp(item_id);

    let layer = app.store.snapshot().comp(comp_id).unwrap().layers[0].clone();
    // Placed from the comp start, keeping its whole length — NOT trimmed to fit.
    assert!(layer.in_point.0.is_zero(), "placed at the comp start");
    assert!(
        (layer.out_point.0.to_f64() - media_dur).abs() < 0.05,
        "keeps its full {media_dur} s, not the {comp_dur} s comp: {}",
        layer.out_point.0.to_f64()
    );
    assert!(
        layer.out_point.0.to_f64() > comp_dur,
        "a long import must extend past the comp end"
    );
}
