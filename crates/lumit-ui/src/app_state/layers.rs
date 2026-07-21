//! `AppState` import and layer operations: importing footage, adding and
//! duplicating layers, keyframe copy/paste, and Sequence-layer editing.

use super::*;

impl AppState {
    pub fn import_footage_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter(
                "Media",
                &[
                    "mp4", "mov", "mkv", "avi", "webm", "png", "jpg", "jpeg", "wav", "mp3", "flac",
                ],
            )
            .pick_files();
        let Some(files) = picked else { return };
        self.import_paths(files);
    }

    /// Import media files (dialogue or drag-and-drop onto the window).
    pub fn import_paths(&mut self, files: Vec<PathBuf>) {
        let base = self.store.snapshot().items.len();
        let mut last_id = None;
        for (i, file) in files.into_iter().enumerate() {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "footage".into());
            let item = FootageItem {
                id: Uuid::now_v7(),
                name: name.clone(),
                extra: serde_json::Map::new(),
                media: MediaRef {
                    relative_path: name,
                    absolute_path: file.to_string_lossy().into_owned(),
                    fingerprint: None,
                    extra: serde_json::Map::new(),
                },
            };
            last_id = Some(item.id);
            #[cfg(feature = "media")]
            let probe_target = (item.id, file.clone());
            self.commit(Op::AddItem {
                index: base + i,
                item: Box::new(ProjectItem::Footage(item)),
            });
            #[cfg(feature = "media")]
            self.media.spawn_probe(probe_target.0, probe_target.1);
        }
        // UI-13: highlight the freshly imported footage and ask the shell to
        // bring the Project tab to the front, so the user sees where it landed.
        if let Some(id) = last_id {
            self.selected_item = Some(id);
            self.focus_project_tab = true;
        }
    }

    /// Add a footage item as a new top layer of the target comp
    /// (docs/16-ROADMAP.md phase 1: comps become buildable by hand).
    pub fn add_footage_to_comp(&mut self, item_id: Uuid) {
        use lumit_core::model::{Layer, LayerKind, Switches};
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(ProjectItem::Footage(f)) = doc.item(item_id) else {
            return;
        };

        // Span: the media's FULL duration when known (frame-exact via the comp
        // grid), positioned from the comp start; else the full comp. A clip
        // longer than the comp keeps its whole length (K-153) — it is not
        // trimmed to fit, only clipped by the comp window [0, comp_end) at
        // render time, so its tail is recoverable by sliding the layer.
        let comp_dur = comp.duration.0;
        #[cfg(feature = "media")]
        let out = match self.media.map.get(&item_id) {
            Some(media::MediaStatus::Ready { probe, .. }) => {
                let frames = (probe.duration_seconds * comp.frame_rate.fps()).round() as i64;
                comp.frame_rate
                    .time_of_frame(frames.max(1))
                    .map(|t| t.0)
                    .unwrap_or(comp_dur)
            }
            _ => comp_dur,
        };
        #[cfg(not(feature = "media"))]
        let out = comp_dur;

        // Origin (anchor) at the footage's centre, placed at the comp centre —
        // so it appears centred and scales/rotates about its middle (AE model).
        #[cfg(feature = "media")]
        let (nat_w, nat_h) = match self.media.map.get(&item_id) {
            Some(media::MediaStatus::Ready { probe, .. }) => probe
                .video
                .as_ref()
                .map(|v| (f64::from(v.width), f64::from(v.height)))
                .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
            _ => (f64::from(comp.width), f64::from(comp.height)),
        };
        #[cfg(not(feature = "media"))]
        let (nat_w, nat_h) = (f64::from(comp.width), f64::from(comp.height));
        let transform = centred_transform(nat_w, nat_h, comp.width, comp.height);

        let layer = Layer {
            id: Uuid::now_v7(),
            name: f.name.clone(),
            kind: LayerKind::Footage {
                item: item_id,
                retime: None,
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(out),
            start_offset: CompTime(Rational::ZERO),
            transform,
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Nest one comp inside another as a Precomp layer (cycle-guarded).
    pub fn add_precomp_to_comp(&mut self, nested_id: Uuid) {
        use lumit_core::model::{Layer, LayerKind, Switches};
        use lumit_core::time::CompTime;
        let Some(target_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        if target_id == nested_id || self.would_cycle(nested_id, target_id) {
            self.error = Some("that nesting would loop compositions".into());
            return;
        }
        let doc = self.store.snapshot();
        let (Some(target), Some(nested)) = (doc.comp(target_id), doc.comp(nested_id)) else {
            return;
        };
        // Keep the nested comp's full duration, positioned from the target's
        // start (K-153); a precomp longer than its parent is not trimmed to fit,
        // only clipped by the target window at render time.
        let out = nested.duration.0;
        let transform = centred_transform(
            f64::from(nested.width),
            f64::from(nested.height),
            target.width,
            target.height,
        );
        let layer = Layer {
            id: Uuid::now_v7(),
            name: nested.name.clone(),
            kind: LayerKind::Precomp { comp: nested_id },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(out),
            start_offset: CompTime(Rational::ZERO),
            transform,
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: target_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(target_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Would nesting `nested` inside `target` create a cycle? True if target
    /// is reachable from nested through Precomp layers.
    fn would_cycle(&self, nested: Uuid, target: Uuid) -> bool {
        use lumit_core::model::LayerKind;
        let doc = self.store.snapshot();
        let mut stack = vec![nested];
        let mut seen = vec![];
        while let Some(id) = stack.pop() {
            if id == target {
                return true;
            }
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            if let Some(c) = doc.comp(id) {
                for l in &c.layers {
                    if let LayerKind::Precomp { comp } = &l.kind {
                        stack.push(*comp);
                    }
                }
            }
        }
        false
    }

    /// Add a text layer with a starter document.
    pub fn add_text_layer(&mut self) {
        use lumit_core::model::{
            Layer, LayerKind, LinearColour, Switches, TextDocument, TransformGroup,
        };
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        // Centre the anchor on the text's estimated bounds (T23) so it rotates
        // and scales around its middle, not the top-left origin. A rough glyph
        // metric (proportional font ≈ 0.5 em advance, ≈ 1 em tall) — exact
        // metrics need the font at render time, a later refinement.
        let text = "Text";
        let size = 72.0_f64;
        let est_w = text.chars().count() as f64 * size * 0.5;
        let transform = TransformGroup {
            anchor_x: lumit_core::anim::Property::fixed(est_w * 0.5),
            anchor_y: lumit_core::anim::Property::fixed(size * 0.5),
            position_x: lumit_core::anim::Property::fixed(f64::from(comp.width) * 0.5),
            position_y: lumit_core::anim::Property::fixed(f64::from(comp.height) * 0.5),
            ..TransformGroup::default()
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "Text".into(),
            kind: LayerKind::Text {
                document: TextDocument {
                    text: text.into(),
                    size,
                    fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                    extra: serde_json::Map::new(),
                },
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            transform,
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Add a Camera layer at the comp centre. Default zoom follows the AE
    /// 50 mm model: comp width x 50/36 (full-frame film width in mm), so a
    /// fresh camera shows the comp exactly as it looked flat.
    pub fn add_camera_layer(&mut self) {
        use lumit_core::model::{Layer, LayerKind, Switches, TransformGroup};
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let transform = TransformGroup {
            position_x: lumit_core::anim::Property::fixed(f64::from(comp.width) * 0.5),
            position_y: lumit_core::anim::Property::fixed(f64::from(comp.height) * 0.5),
            ..TransformGroup::default()
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "Camera".into(),
            kind: LayerKind::Camera {
                zoom: lumit_core::anim::Property::fixed(f64::from(comp.width) * 50.0 / 36.0),
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            transform,
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Add an adjustment layer at the top of the stack — a comp-sized effect
    /// container whose stack applies to everything beneath it within its span
    /// (docs/01-GLOSSARY.md), staged and blended by coverage as of K-091.
    pub fn add_adjustment_layer(&mut self) {
        use lumit_core::model::{Layer, LayerKind, Switches};
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "Adjustment".into(),
            kind: LayerKind::Adjustment,
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            // Comp-sized layer: anchor at the comp centre so scale/rotation
            // pivot about the middle (FX-20, K-150). The net placement stays
            // identity, so the K-091 coverage staging is unchanged.
            transform: centred_transform(
                f64::from(comp.width),
                f64::from(comp.height),
                comp.width,
                comp.height,
            ),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Duplicate the selected layer: an exact copy with a fresh id and a
    /// "… copy" name, inserted directly above the original (the AE
    /// convention). The copy's effects get fresh instance ids so the two
    /// layers never share an effect instance; its parent/matte references
    /// still point at the same other layers, which remain valid, and its own
    /// new id means nothing it references can be itself.
    pub fn duplicate_layer(&mut self) {
        use lumit_core::model::Layer;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            self.error = Some("select a layer to duplicate".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(pos) = comp.layers.iter().position(|l| l.id == layer_id) else {
            return;
        };
        let mut copy: Layer = comp.layers[pos].clone();
        let new_id = Uuid::now_v7();
        copy.id = new_id;
        copy.name = format!("{} copy", copy.name);
        for e in &mut copy.effects {
            e.id = Uuid::now_v7();
        }
        // Insert at the original's index so the copy lands just above it in
        // the stack (index 0 is the topmost layer).
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: pos,
            layer: Box::new(copy),
        });
        self.selected_comp = Some(comp_id);
        self.selected_layer = Some(new_id);
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Move or trim the selected layer's span relative to the playhead — the
    /// `[` / `]` / `Alt+[` / `Alt+]` keys (docs/07-UI-SPEC §4.7). The span maths
    /// live in `lumit_core::ops::edit_layer_span` (tested there); this resolves
    /// the selection and playhead and commits the resulting `SetLayerSpan`. A
    /// degenerate trim (one that would invert the span) is silently ignored.
    pub fn edit_selected_layer_span(&mut self, edit: lumit_core::ops::SpanEdit) {
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Ok(playhead) = comp.frame_rate.time_of_frame(self.preview_frame as i64) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let Some((in_point, out_point, start_offset)) = lumit_core::ops::edit_layer_span(
            layer.in_point,
            layer.out_point,
            layer.start_offset,
            playhead,
            edit,
        ) else {
            return;
        };
        self.commit(Op::SetLayerSpan {
            comp: comp_id,
            layer: layer_id,
            in_point,
            out_point,
            start_offset,
        });
    }

    /// Delete the selected layer from its composition (one undoable step).
    /// Deliberately reachable only from the menu and the command palette, not
    /// a bare Delete key, so it can never fire while a value field has focus.
    pub fn delete_selected_layer(&mut self) {
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            self.error = Some("select a layer to delete".into());
            return;
        };
        let present = self
            .store
            .snapshot()
            .comp(comp_id)
            .is_some_and(|c| c.layers.iter().any(|l| l.id == layer_id));
        if !present {
            return;
        }
        self.commit(Op::RemoveLayer {
            comp: comp_id,
            layer: layer_id,
        });
        self.selected_layer = None;
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Copy the selected lane keyframes to the clipboard (note 2.2): each with
    /// its bezier handles and its offset from the earliest selected key. A key on
    /// a linked Anchor/Position/Scale row copies BOTH axes' keys at that time, so
    /// a paste rebuilds the pair. Nothing selected leaves the clipboard as it was.
    pub fn copy_selected_keyframes(&mut self) {
        use lumit_core::anim::{Animation, Keyframe};
        use lumit_core::model::{EffectValue, Layer, TransformProp};
        // The lane selection OR the graph-editor selection (T4): Ctrl+C in the
        // graph view keys off `graph_selection`, not the lanes.
        if self.lane_selection.is_empty() && self.graph_selection.is_none() {
            return;
        }
        let doc = self.store.snapshot();
        let Some(comp) = self.selected_comp.and_then(|id| doc.comp(id)) else {
            return;
        };
        let tol = 0.5 / comp.frame_rate.fps().max(1.0);
        let tf_key = |layer: &Layer, prop: TransformProp, t: f64| -> Option<Keyframe> {
            if let Animation::Keyframed(keys) = &layer.transform.get(prop).animation {
                keys.iter()
                    .find(|k| (k.time.to_f64() - t).abs() < tol)
                    .copied()
            } else {
                None
            }
        };
        let fx_key = |layer: &Layer, e: usize, p: usize, t: f64| -> Option<Keyframe> {
            let param = layer.effects.get(e)?.params.get(p)?;
            if let EffectValue::Float(prop) = &param.value {
                if let Animation::Keyframed(keys) = &prop.animation {
                    return keys
                        .iter()
                        .find(|k| (k.time.to_f64() - t).abs() < tol)
                        .copied();
                }
            }
            None
        };
        // (layer, row, absolute local time, key).
        let mut collected: Vec<(Uuid, PropRow, f64, Keyframe)> = Vec::new();
        // In GRAPH mode the graph-editor selection is what the user sees, so it
        // wins outright (owner T4 retest: a stale lane selection from the lane
        // view was shadowing it, leaving the clipboard holding the old keys).
        let graph_first = self.timeline_graph_mode;
        if graph_first {
            self.collect_graph_selection(comp, tol, &mut collected);
        }
        if !graph_first || collected.is_empty() {
            self.collect_lane_selection(comp, tol, &tf_key, &fx_key, &mut collected);
        }
        if collected.is_empty() {
            self.collect_graph_selection(comp, tol, &mut collected);
        }
        if collected.is_empty() {
            return;
        }
        let anchor = collected
            .iter()
            .map(|(_, _, t, _)| *t)
            .fold(f64::INFINITY, f64::min);
        self.keyframe_clipboard = collected
            .into_iter()
            .map(|(layer, row, t, key)| {
                // Record the bezier handles' ABSOLUTE lengths against the
                // source neighbours (T5), so paste can rebuild the influences.
                let (in_len, out_len) = bezier_side_lengths(comp, layer, row, t, tol);
                ClipboardKey {
                    layer,
                    row,
                    offset: t - anchor,
                    key,
                    in_len,
                    out_len,
                }
            })
            .collect();
    }

    /// The lane-selection half of [`Self::copy_selected_keyframes`]: resolve
    /// (see also [`bezier_side_lengths`], the T5 handle-length capture).
    /// every selected lane key against the document and push the found
    /// keyframes.
    #[allow(clippy::type_complexity)]
    fn collect_lane_selection(
        &self,
        comp: &lumit_core::model::Composition,
        _tol: f64,
        tf_key: &dyn Fn(
            &lumit_core::model::Layer,
            lumit_core::model::TransformProp,
            f64,
        ) -> Option<lumit_core::anim::Keyframe>,
        fx_key: &dyn Fn(
            &lumit_core::model::Layer,
            usize,
            usize,
            f64,
        ) -> Option<lumit_core::anim::Keyframe>,
        collected: &mut Vec<(Uuid, PropRow, f64, lumit_core::anim::Keyframe)>,
    ) {
        for s in &self.lane_selection {
            let Some(layer) = comp.layers.iter().find(|l| l.id == s.layer) else {
                continue;
            };
            let t = s.time.to_f64();
            match s.row {
                PropRow::Transform(prop) => {
                    if let Some(k) = tf_key(layer, prop, t) {
                        collected.push((s.layer, PropRow::Transform(prop), t, k));
                    }
                    // A linked pair carries its partner axis too.
                    if self
                        .lane_linked
                        .iter()
                        .any(|(l, pp)| *l == s.layer && *pp == prop)
                    {
                        if let Some(py) = linked_axis_partner(prop) {
                            if let Some(k) = tf_key(layer, py, t) {
                                collected.push((s.layer, PropRow::Transform(py), t, k));
                            }
                        }
                    }
                }
                PropRow::Effect { effect, param } => {
                    if let Some(k) = fx_key(layer, effect, param, t) {
                        collected.push((s.layer, s.row, t, k));
                    }
                }
                // The Retime channel's keys aren't drawn as selectable lane
                // glyphs, so a lane selection never carries one.
                PropRow::Retime => {}
            }
        }
    }

    /// The graph-editor half of [`Self::copy_selected_keyframes`] (T4): copy the
    /// graphed channel's selected transform keys. The (index, time) pins resolve
    /// exactly first; when they've gone stale (a drag re-timed keys since the
    /// marquee), each pin falls back to the key nearest its remembered time
    /// (within half a frame), so a copy straight after an edit still works.
    /// Retime's Time channel is skipped — paste has no Retime target.
    fn collect_graph_selection(
        &self,
        comp: &lumit_core::model::Composition,
        tol: f64,
        collected: &mut Vec<(Uuid, PropRow, f64, lumit_core::anim::Keyframe)>,
    ) {
        use lumit_core::anim::Animation;
        let Some(sel) = &self.graph_selection else {
            return;
        };
        if sel.retime {
            return;
        }
        let Some(layer) = comp.layers.iter().find(|l| l.id == sel.layer) else {
            return;
        };
        let Animation::Keyframed(keys) = &layer.transform.get(sel.prop).animation else {
            return;
        };
        if let Some(indices) = sel.indices_for(keys) {
            for i in indices {
                let k = keys[i];
                collected.push((sel.layer, PropRow::Transform(sel.prop), k.time.to_f64(), k));
            }
        } else {
            // Stale pins: match by remembered time within tolerance instead.
            for &(_, t) in &sel.keys {
                let ts = t.to_f64();
                if let Some(k) = keys.iter().find(|k| (k.time.to_f64() - ts).abs() < tol) {
                    collected.push((sel.layer, PropRow::Transform(sel.prop), k.time.to_f64(), *k));
                }
            }
        }
    }

    /// Paste the clipboard keyframes at the playhead (note 2.2): each lands on
    /// its own property at `playhead + offset` (layer-local), OVERWRITING any key
    /// whose time coincides and carrying its bezier handles. One Batch, so a
    /// paste is a single undo step; the pasted keys become the lane selection.
    pub fn paste_keyframes(&mut self) {
        use lumit_core::anim::{Animation, Keyframe};
        use lumit_core::model::{EffectValue, TransformProp};
        if self.keyframe_clipboard.is_empty() {
            return;
        }
        let doc = self.store.snapshot();
        let Some(comp_id) = self.selected_comp else {
            return;
        };
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let fps = comp.frame_rate.fps().max(1.0);
        let tol = 0.5 / fps;
        let playhead_comp = self.preview_frame as f64 / fps;
        let rt = |s: f64| {
            Rational::from_f64_on_grid(s.max(0.0), Rational::FLICK_DEN).unwrap_or(Rational::ZERO)
        };

        // Group the pasted keys per (layer, transform channel) and (layer, effect
        // index, param index), placing each at the playhead plus its offset.
        type Lens = Vec<(f64, Option<f64>, Option<f64>)>;
        let mut tf: Vec<(Uuid, TransformProp, Vec<Keyframe>, Lens)> = Vec::new();
        let mut fx: Vec<(Uuid, usize, usize, Vec<Keyframe>, Lens)> = Vec::new();
        let mut new_sel: Vec<LaneKeySel> = Vec::new();
        for c in &self.keyframe_clipboard {
            let Some(layer) = comp.layers.iter().find(|l| l.id == c.layer) else {
                continue;
            };
            let local = playhead_comp - layer.start_offset.0.to_f64() + c.offset;
            let time = rt(local);
            let mut key = c.key;
            key.time = time;
            new_sel.push(LaneKeySel {
                layer: c.layer,
                row: c.row,
                time,
            });
            let len_entry = (time.to_f64(), c.in_len, c.out_len);
            match c.row {
                PropRow::Transform(prop) => {
                    match tf
                        .iter_mut()
                        .find(|(l, p, _, _)| *l == c.layer && *p == prop)
                    {
                        Some((_, _, ks, ls)) => {
                            ks.push(key);
                            ls.push(len_entry);
                        }
                        None => tf.push((c.layer, prop, vec![key], vec![len_entry])),
                    }
                }
                PropRow::Effect { effect, param } => {
                    match fx
                        .iter_mut()
                        .find(|(l, e, p, _, _)| *l == c.layer && *e == effect && *p == param)
                    {
                        Some((_, _, _, ks, ls)) => {
                            ks.push(key);
                            ls.push(len_entry);
                        }
                        None => fx.push((c.layer, effect, param, vec![key], vec![len_entry])),
                    }
                }
                // Retime keys are not on the lane clipboard (see copy above).
                PropRow::Retime => {}
            }
        }

        let mut ops: Vec<Op> = Vec::new();
        for (layer_id, prop, pasted, lens) in &tf {
            let Some(layer) = comp.layers.iter().find(|l| l.id == *layer_id) else {
                continue;
            };
            let existing: Vec<Keyframe> = match &layer.transform.get(*prop).animation {
                Animation::Keyframed(k) => k.clone(),
                Animation::Static(_) => Vec::new(),
            };
            let mut merged = merge_paste_keys(&existing, pasted, tol);
            restore_handle_lengths(&mut merged, lens, tol);
            ops.push(Op::SetTransformProperty {
                comp: comp_id,
                layer: *layer_id,
                prop: *prop,
                animation: Animation::Keyframed(merged),
            });
        }
        // Effect params: one SetLayerEffects per layer, all its params folded in.
        let mut fx_layers: Vec<Uuid> = Vec::new();
        for (l, ..) in &fx {
            if !fx_layers.contains(l) {
                fx_layers.push(*l);
            }
        }
        for layer_id in fx_layers {
            let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
                continue;
            };
            let mut effects = layer.effects.clone();
            let mut touched = false;
            for (_l, e, p, pasted, lens) in fx.iter().filter(|(l, ..)| *l == layer_id) {
                if let Some(param) = effects.get_mut(*e).and_then(|inst| inst.params.get_mut(*p)) {
                    if let EffectValue::Float(prop) = &mut param.value {
                        let existing: Vec<Keyframe> = match &prop.animation {
                            Animation::Keyframed(k) => k.clone(),
                            Animation::Static(_) => Vec::new(),
                        };
                        let mut merged = merge_paste_keys(&existing, pasted, tol);
                        restore_handle_lengths(&mut merged, lens, tol);
                        prop.animation = Animation::Keyframed(merged);
                        touched = true;
                    }
                }
            }
            if touched {
                ops.push(Op::SetLayerEffects {
                    comp: comp_id,
                    layer: layer_id,
                    effects,
                });
            }
        }

        if ops.is_empty() {
            return;
        }
        let op = if ops.len() == 1 {
            ops.remove(0)
        } else {
            Op::Batch { ops }
        };
        self.commit(op);
        self.lane_selection = new_sel;
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Add a keyframe at the playhead to every property row in the selection
    /// (note 2.6) — the multi-select "key selected" path. One undo step keys a
    /// mixed set of transform, effect and Retime rows at the current frame, each
    /// holding its present value, so the user can key several channels at the
    /// same point at once. A static channel becomes a single key at the playhead
    /// (like flicking its stopwatch); an animated one gains or updates the key
    /// there. Nothing selected, or no open comp, is a no-op.
    pub fn key_selected_props(&mut self) {
        use lumit_core::anim::{Animation, Keyframe, SideInterp};
        use lumit_core::model::{EffectValue, LayerKind};
        if self.selected_props.is_empty() {
            return;
        }
        let doc = self.store.snapshot();
        let Some(comp_id) = self.selected_comp else {
            return;
        };
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let fps = comp.frame_rate.fps().max(1.0);
        let playhead_comp = self.preview_frame as f64 / fps;

        // Insert or replace a key at layer-local `lt` holding the value there.
        let keyed = |prop: &lumit_core::anim::Property, lt: f64| -> Animation {
            const EPS: f64 = 1.0 / 240.0;
            let v = prop.value_at(lt);
            let mut keys = match &prop.animation {
                Animation::Keyframed(k) => k.clone(),
                Animation::Static(_) => Vec::new(),
            };
            let t = Rational::from_f64_on_grid(lt.max(0.0), Rational::FLICK_DEN)
                .unwrap_or(Rational::ZERO);
            if let Some(k) = keys.iter_mut().find(|k| (k.time.to_f64() - lt).abs() < EPS) {
                k.value = v;
            } else {
                keys.push(Keyframe {
                    time: t,
                    value: v,
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                });
                keys.sort_by_key(|k| k.time);
            }
            Animation::Keyframed(keys)
        };

        let mut ops: Vec<Op> = Vec::new();
        // Transform channels commit independently; effects and Retime fold once
        // per layer (each op replaces the whole stack / whole retime).
        let mut fx_layers: Vec<Uuid> = Vec::new();
        let mut rt_layers: Vec<Uuid> = Vec::new();
        for sel in &self.selected_props {
            let Some(layer) = comp.layers.iter().find(|l| l.id == sel.layer) else {
                continue;
            };
            let lt = playhead_comp - layer.start_offset.0.to_f64();
            match sel.row {
                PropRow::Transform(prop) => ops.push(Op::SetTransformProperty {
                    comp: comp_id,
                    layer: sel.layer,
                    prop,
                    animation: keyed(layer.transform.get(prop), lt),
                }),
                PropRow::Effect { .. } => {
                    if !fx_layers.contains(&sel.layer) {
                        fx_layers.push(sel.layer);
                    }
                }
                PropRow::Retime => {
                    if !rt_layers.contains(&sel.layer) {
                        rt_layers.push(sel.layer);
                    }
                }
            }
        }

        // Effects: one whole-stack op per layer, folding every selected Float
        // parameter's new key in.
        for layer_id in fx_layers {
            let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
                continue;
            };
            let lt = playhead_comp - layer.start_offset.0.to_f64();
            let mut effects = layer.effects.clone();
            let mut touched = false;
            for sel in self.selected_props.iter().filter(|s| s.layer == layer_id) {
                if let PropRow::Effect { effect, param } = sel.row {
                    if let Some(p) = effects
                        .get_mut(effect)
                        .and_then(|e| e.params.get_mut(param))
                    {
                        if let EffectValue::Float(prop) = &mut p.value {
                            let anim = keyed(prop, lt);
                            prop.animation = anim;
                            touched = true;
                        }
                    }
                }
            }
            if touched {
                ops.push(Op::SetLayerEffects {
                    comp: comp_id,
                    layer: layer_id,
                    effects,
                });
            }
        }

        // Retime: a speed (velocity) key at the playhead holding the current
        // speed — lens-independent and media-free, so keying a mixed selection
        // stays deterministic.
        for layer_id in rt_layers {
            let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
                continue;
            };
            let LayerKind::Footage { retime, .. } = &layer.kind else {
                continue;
            };
            let lt = playhead_comp - layer.start_offset.0.to_f64();
            let dur = layer.out_point.0;
            let speed = retime.as_ref().map(|r| r.speed_at(lt)).unwrap_or(1.0);
            let sr = Rational::from_f64_on_grid(speed, 1000).unwrap_or(Rational::ONE);
            let mut keys = retime
                .as_ref()
                .and_then(|r| r.speed_keyframes())
                .unwrap_or_else(|| vec![(Rational::ZERO, Rational::ONE), (dur, Rational::ONE)]);
            let t = Rational::from_f64_on_grid(lt.clamp(0.0, dur.to_f64()), 1000)
                .unwrap_or(Rational::ZERO);
            if let Some(k) = keys.iter_mut().find(|k| k.0 == t) {
                k.1 = sr;
            } else {
                keys.push((t, sr));
                keys.sort_by_key(|k| k.0);
            }
            if let Some(new_retime) =
                lumit_core::retime::Retime::from_speed_keyframes(Rational::ZERO, &keys)
            {
                ops.push(Op::SetLayerRetime {
                    comp: comp_id,
                    layer: layer_id,
                    retime: Some(new_retime),
                });
            }
        }

        if ops.is_empty() {
            return;
        }
        let op = if ops.len() == 1 {
            ops.remove(0)
        } else {
            Op::Batch { ops }
        };
        self.commit(op);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Add a Sequence layer (Vegas-style clip row). If a footage item is
    /// selected in the Project panel it becomes the first clip spanning the
    /// footage; otherwise the layer starts empty. This is a first, simple
    /// build path — richer clip editing (drag, cut, trim) follows.
    pub fn add_sequence_layer(&mut self) {
        use lumit_core::model::{Layer, LayerKind, Switches};
        use lumit_core::sequence::{Clip, ClipSource};
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        // One clip from the selected footage item, if there is one.
        let mut clips = Vec::new();
        let mut name = "Sequence".to_string();
        if let Some(sel) = self.selected_item {
            if let Some(ProjectItem::Footage(f)) = doc.item(sel) {
                #[cfg(feature = "media")]
                let dur = match self.media.map.get(&sel) {
                    Some(media::MediaStatus::Ready { probe, .. }) => probe.duration_seconds,
                    _ => comp.duration.0.to_f64(),
                };
                #[cfg(not(feature = "media"))]
                let dur = comp.duration.0.to_f64();
                let dur = Rational::from_f64_on_grid(
                    dur.max(1.0 / comp.frame_rate.fps().max(1.0)),
                    Rational::FLICK_DEN,
                )
                .unwrap_or(comp.duration.0);
                clips.push(Clip::new(
                    ClipSource::Footage(sel),
                    Rational::ZERO,
                    dur,
                    Rational::ZERO,
                    dur,
                ));
                name = f.name.clone();
            }
        }
        let out = if let Some(c) = clips.first() {
            CompTime(c.place_end())
        } else {
            CompTime(comp.duration.0)
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name,
            kind: LayerKind::Sequence { clips },
            in_point: CompTime(Rational::ZERO),
            out_point: out,
            start_offset: CompTime(Rational::ZERO),
            transform: centred_transform(
                f64::from(comp.width),
                f64::from(comp.height),
                comp.width,
                comp.height,
            ),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Convert the selected imported-footage layer into a sequenced layer
    /// (K-071): its one footage becomes a single clip you can then cut and
    /// retime. Only footage layers qualify. One undo step; the layer keeps its
    /// id, transform, masks and span, carrying any existing retime into the
    /// clip.
    pub fn convert_to_sequenced_layer(&mut self) {
        use lumit_core::model::LayerKind;
        use lumit_core::sequence::{Clip, ClipSource};
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            self.error = Some("select a footage layer to convert".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(index) = comp.layers.iter().position(|l| l.id == layer_id) else {
            return;
        };
        let layer = &comp.layers[index];
        let LayerKind::Footage { item, retime } = &layer.kind else {
            self.error = Some("only footage layers convert to sequenced".into());
            return;
        };
        // Footage duration → the clip's source/place length.
        #[cfg(feature = "media")]
        let dur_s = match self.media.map.get(item) {
            Some(media::MediaStatus::Ready { probe, .. }) => probe.duration_seconds,
            _ => (layer.out_point.0.to_f64() - layer.in_point.0.to_f64()).max(0.04),
        };
        #[cfg(not(feature = "media"))]
        let dur_s = (layer.out_point.0.to_f64() - layer.in_point.0.to_f64()).max(0.04);
        let dur = Rational::from_f64_on_grid(dur_s.max(0.04), Rational::FLICK_DEN)
            .unwrap_or(layer.out_point.0);
        let clip = Clip {
            id: Uuid::now_v7(),
            source: ClipSource::Footage(*item),
            source_in: Rational::ZERO,
            source_out: dur,
            place_start: Rational::ZERO,
            place_duration: dur,
            retime: retime
                .clone()
                .unwrap_or_else(|| lumit_core::retime::Retime::identity(dur, Rational::ZERO)),
            interpolation: Default::default(),
            extra: serde_json::Map::new(),
        };
        let mut new_layer = layer.clone();
        new_layer.kind = LayerKind::Sequence { clips: vec![clip] };
        // One undo step: drop the footage layer, add the sequenced one in its
        // place (same id and index, so it's a true in-place conversion).
        self.commit(Op::Batch {
            ops: vec![
                Op::RemoveLayer {
                    comp: comp_id,
                    layer: layer_id,
                },
                Op::AddLayer {
                    comp: comp_id,
                    index,
                    layer: Box::new(new_layer),
                },
            ],
        });
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Razor: cut the selected Sequence layer's clip at the playhead into two
    /// (one undo step). The beat-sync covenant holds — clip places don't move.
    pub fn cut_sequence_at_playhead(&mut self) {
        use lumit_core::model::LayerKind;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            self.error = Some("select a sequence layer to cut".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let LayerKind::Sequence { clips } = &layer.kind else {
            self.error = Some("the razor needs a sequence layer".into());
            return;
        };
        // Exact layer-local cut time at the playhead.
        let Ok(comp_t) = comp.frame_rate.time_of_frame(self.preview_frame as i64) else {
            return;
        };
        let Ok(tau) = comp_t.0.checked_sub(layer.start_offset.0) else {
            return;
        };
        let Some(idx) = clips.iter().position(|c| c.contains(tau.to_f64())) else {
            self.error = Some("no clip under the playhead".into());
            return;
        };
        let Some((left, right)) = clips[idx].cut(tau) else {
            self.error = Some("can't cut an eased ramp here yet".into());
            return;
        };
        let mut new_clips = clips.clone();
        new_clips.splice(idx..=idx, [left, right]);
        self.commit(Op::SetSequenceClips {
            comp: comp_id,
            layer: layer_id,
            clips: new_clips,
        });
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Set the selected clip's speed ramp (start/end percent; 100 = source
    /// rate) with an ease, keeping its place on the layer (beat-sync covenant).
    /// Equal start/end with a Linear ease is a plain constant speed.
    pub fn set_selected_clip_ramp(
        &mut self,
        v0_pct: f64,
        v1_pct: f64,
        ease: lumit_core::retime::Ease,
    ) {
        use lumit_core::model::LayerKind;
        let (Some(comp_id), Some(layer_id), Some(clip_id)) =
            (self.selected_comp, self.selected_layer, self.selected_clip)
        else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let LayerKind::Sequence { clips } = &layer.kind else {
            return;
        };
        let Some(idx) = clips.iter().position(|c| c.id == clip_id) else {
            return;
        };
        let pct = |p: f64| {
            lumit_core::Rational::from_f64_on_grid(p / 100.0, 1000)
                .unwrap_or(lumit_core::Rational::ONE)
        };
        let mut new_clips = clips.clone();
        new_clips[idx] = new_clips[idx].with_ramp(pct(v0_pct), pct(v1_pct), ease);
        self.commit(Op::SetSequenceClips {
            comp: comp_id,
            layer: layer_id,
            clips: new_clips,
        });
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Set the selected clip's frame interpolation (Nearest / Blend / Flow).
    pub fn set_selected_clip_interp(&mut self, interp: lumit_core::retime::Interpolation) {
        use lumit_core::model::LayerKind;
        let (Some(comp_id), Some(layer_id), Some(clip_id)) =
            (self.selected_comp, self.selected_layer, self.selected_clip)
        else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let LayerKind::Sequence { clips } = &layer.kind else {
            return;
        };
        let Some(idx) = clips.iter().position(|c| c.id == clip_id) else {
            return;
        };
        let mut new_clips = clips.clone();
        new_clips[idx].interpolation = interp;
        self.commit(Op::SetSequenceClips {
            comp: comp_id,
            layer: layer_id,
            clips: new_clips,
        });
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Delete the clip under the playhead in the selected sequence layer,
    /// leaving a gap (the Vegas surface allows gaps — K-071).
    pub fn delete_clip_at_playhead(&mut self) {
        use lumit_core::model::LayerKind;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            self.error = Some("select a sequence layer".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let LayerKind::Sequence { clips } = &layer.kind else {
            self.error = Some("not a sequence layer".into());
            return;
        };
        let Ok(comp_t) = comp.frame_rate.time_of_frame(self.preview_frame as i64) else {
            return;
        };
        let Ok(tau) = comp_t.0.checked_sub(layer.start_offset.0) else {
            return;
        };
        let Some(idx) = clips.iter().position(|c| c.contains(tau.to_f64())) else {
            self.error = Some("no clip under the playhead".into());
            return;
        };
        let mut new_clips = clips.clone();
        new_clips.remove(idx);
        self.commit(Op::SetSequenceClips {
            comp: comp_id,
            layer: layer_id,
            clips: new_clips,
        });
        #[cfg(feature = "media")]
        self.refresh_preview();
    }
}

/// The ABSOLUTE lengths (seconds) of a key's bezier handles against its SOURCE
/// neighbours at copy time (T5): `influence × neighbour gap` per side. None for
/// a non-bezier side, an endpoint (no neighbour), or a channel that no longer
/// resolves. Pure — paste rebuilds influences from these against the
/// destination gaps (`restore_handle_lengths`).
fn bezier_side_lengths(
    comp: &lumit_core::model::Composition,
    layer_id: Uuid,
    row: PropRow,
    t: f64,
    tol: f64,
) -> (Option<f64>, Option<f64>) {
    use lumit_core::anim::{Animation, SideInterp};
    use lumit_core::model::EffectValue;
    let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
        return (None, None);
    };
    let keys: Vec<lumit_core::anim::Keyframe> = match row {
        PropRow::Transform(prop) => match &layer.transform.get(prop).animation {
            Animation::Keyframed(k) => k.clone(),
            Animation::Static(_) => return (None, None),
        },
        PropRow::Effect { effect, param } => {
            let Some(p) = layer.effects.get(effect).and_then(|e| e.params.get(param)) else {
                return (None, None);
            };
            match &p.value {
                EffectValue::Float(prop) => match &prop.animation {
                    Animation::Keyframed(k) => k.clone(),
                    Animation::Static(_) => return (None, None),
                },
                _ => return (None, None),
            }
        }
        PropRow::Retime => return (None, None),
    };
    let Some(i) = keys.iter().position(|k| (k.time.to_f64() - t).abs() < tol) else {
        return (None, None);
    };
    let in_len = (i > 0)
        .then(|| keys[i].time.to_f64() - keys[i - 1].time.to_f64())
        .and_then(|gap| match keys[i].interp_in {
            SideInterp::Bezier { influence, .. } => Some(influence * gap),
            _ => None,
        });
    let out_len = (i + 1 < keys.len())
        .then(|| keys[i + 1].time.to_f64() - keys[i].time.to_f64())
        .and_then(|gap| match keys[i].interp_out {
            SideInterp::Bezier { influence, .. } => Some(influence * gap),
            _ => None,
        });
    (in_len, out_len)
}
