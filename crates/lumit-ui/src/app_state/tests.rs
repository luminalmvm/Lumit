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
