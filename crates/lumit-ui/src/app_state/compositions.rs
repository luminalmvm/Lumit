//! `AppState` composition and project-item operations: solids, the
//! composition dialogue, opening/closing comps, folders and the work area.

use super::*;

impl AppState {
    /// Ops that guarantee the auto-filing folder for `kind` exists, plus its
    /// id. Tracks the folder by id (AE habit: renaming or nesting the Solids
    /// folder keeps it the Solids folder); a deleted one is recreated.
    fn ensure_auto_folder_ops(&self, kind: lumit_core::ops::AutoFolderKind) -> (Uuid, Vec<Op>) {
        use lumit_core::model::Folder;
        use lumit_core::ops::AutoFolderKind;
        let doc = self.store.snapshot();
        let slot = match kind {
            AutoFolderKind::Solids => doc.auto_folders.solids,
            AutoFolderKind::Compositions => doc.auto_folders.compositions,
        };
        if let Some(id) = slot {
            if doc.folder(id).is_some() {
                return (id, Vec::new());
            }
        }
        let id = Uuid::now_v7();
        let name = match kind {
            AutoFolderKind::Solids => "Solids",
            AutoFolderKind::Compositions => "Compositions",
        };
        (
            id,
            vec![
                Op::AddItem {
                    index: doc.items.len(),
                    item: Box::new(ProjectItem::Folder(Folder {
                        id,
                        name: name.into(),
                        children: Vec::new(),
                        extra: serde_json::Map::new(),
                    })),
                },
                Op::SetAutoFolder {
                    kind,
                    folder: Some(id),
                },
            ],
        )
    }

    /// The op that files `item` into `folder` (appended), given the ops in
    /// `prior` may have just created the folder.
    fn file_into_folder_op(&self, folder: Uuid, item: Uuid, prior: &[Op]) -> Op {
        let doc = self.store.snapshot();
        let mut children = doc
            .folder(folder)
            .map(|f| f.children.clone())
            .unwrap_or_default();
        // The folder may not exist yet (created earlier in this batch).
        let _ = prior;
        children.push(item);
        Op::SetFolderChildren { folder, children }
    }

    /// Add a Solid layer backed by a SolidDef asset filed in the Solids
    /// auto-folder (docs/03-DATA-MODEL.md §2: solids are assets so they
    /// dedupe). One batch, one undo step.
    pub fn add_solid_layer(&mut self) {
        use lumit_core::model::{Layer, LayerKind, LinearColour, SolidDef, Switches};
        use lumit_core::ops::AutoFolderKind;
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let (folder_id, mut ops) = self.ensure_auto_folder_ops(AutoFolderKind::Solids);
        let def_id = Uuid::now_v7();
        let n_solids = doc
            .items
            .iter()
            .filter(|i| matches!(i, ProjectItem::Solid(_)))
            .count();
        let added = ops
            .iter()
            .filter(|o| matches!(o, Op::AddItem { .. }))
            .count();
        ops.push(Op::AddItem {
            index: doc.items.len() + added,
            item: Box::new(ProjectItem::Solid(SolidDef {
                id: def_id,
                name: format!("White solid {}", n_solids + 1),
                colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
                width: comp.width,
                height: comp.height,
                extra: serde_json::Map::new(),
            })),
        });
        ops.push(self.file_into_folder_op(folder_id, def_id, &ops));
        let layer = Layer {
            id: Uuid::now_v7(),
            name: format!("White solid {}", n_solids + 1),
            kind: LayerKind::Solid { def: def_id },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            // A comp-sized solid: anchor at its own centre, placed at the comp
            // centre (FX-20, K-150).
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
        ops.push(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.commit(Op::Batch { ops });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Open the settings dialogue for a new comp. Defaults match the pending
    /// footage when a drop starts the comp; otherwise the house defaults.
    pub fn open_new_comp_dialog(&mut self, pending_item: Option<Uuid>) {
        let mut dialog = CompDialog {
            editing: None,
            name: format!("Comp {}", self.comp_counter + 1),
            width: 1920,
            height: 1080,
            fps: 60.0,
            duration_s: 30.0,
            lock_ratio: true,
            aspect: 1920.0 / 1080.0,
            pending_item,
            motion_blur: lumit_core::model::MotionBlur::default(),
        };
        #[cfg(feature = "media")]
        if let Some(item) = pending_item {
            if let Some(media::MediaStatus::Ready { probe, frames, .. }) = self.media.map.get(&item)
            {
                if let Some(v) = &probe.video {
                    dialog.width = v.width;
                    dialog.height = v.height;
                    dialog.aspect = f64::from(v.width) / f64::from(v.height).max(1.0);
                    dialog.fps = v.fps();
                    dialog.duration_s = *frames as f64 / v.fps().max(1.0);
                }
            }
        }
        self.comp_dialog = Some(dialog);
    }

    /// Open the settings dialogue pre-filled from an existing comp.
    pub fn open_comp_settings(&mut self, comp_id: Uuid) {
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        self.comp_dialog = Some(CompDialog {
            editing: Some(comp_id),
            name: comp.name.clone(),
            width: comp.width,
            height: comp.height,
            fps: comp.frame_rate.fps(),
            duration_s: comp.duration.0.to_f64(),
            lock_ratio: true,
            aspect: f64::from(comp.width) / f64::from(comp.height).max(1.0),
            pending_item: None,
            motion_blur: comp.motion_blur,
        });
    }

    /// fps as a rational: exact when whole, NTSC-snapped near x/1.001,
    /// millifps otherwise.
    fn frame_rate_of(fps: f64) -> Option<FrameRate> {
        let fps = fps.clamp(1.0, 1000.0);
        let whole = fps.round();
        if (fps - whole).abs() < 0.001 {
            return FrameRate::new(whole as u32, 1).ok();
        }
        let ntsc_base = (fps * 1.001).round();
        if (fps - ntsc_base * 1000.0 / 1001.0).abs() < 0.001 {
            return FrameRate::new(ntsc_base as u32 * 1000, 1001).ok();
        }
        FrameRate::new((fps * 1000.0).round() as u32, 1000).ok()
    }

    /// Apply the open dialogue: create the comp (filed in the Compositions
    /// auto-folder, one undo step) or update the existing one.
    pub fn confirm_comp_dialog(&mut self) {
        use lumit_core::ops::AutoFolderKind;
        let Some(dialog) = self.comp_dialog.take() else {
            return;
        };
        let Some(frame_rate) = Self::frame_rate_of(dialog.fps) else {
            self.error = Some("invalid frame rate".into());
            return;
        };
        let duration = Duration(
            Rational::from_f64_on_grid(dialog.duration_s.max(0.04), Rational::FLICK_DEN)
                .unwrap_or(rat(30, 1)),
        );
        let width = dialog.width.clamp(16, 16384);
        let height = dialog.height.clamp(16, 16384);
        if let Some(comp_id) = dialog.editing {
            let doc = self.store.snapshot();
            let Some(comp) = doc.comp(comp_id) else {
                return;
            };
            let mb_changed = comp.motion_blur != dialog.motion_blur;
            self.commit(Op::SetCompSettings {
                comp: comp_id,
                name: dialog.name,
                width,
                height,
                frame_rate,
                duration,
                background: comp.background,
            });
            if mb_changed {
                self.commit(Op::SetCompMotionBlur {
                    comp: comp_id,
                    motion_blur: dialog.motion_blur,
                });
            }
            #[cfg(feature = "media")]
            self.refresh_preview();
            return;
        }
        self.comp_counter += 1;
        let comp = Composition {
            id: Uuid::now_v7(),
            name: dialog.name,
            width,
            height,
            frame_rate,
            duration,
            background: LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        };
        let id = comp.id;
        let doc = self.store.snapshot();
        let (folder_id, mut ops) = self.ensure_auto_folder_ops(AutoFolderKind::Compositions);
        let added = ops
            .iter()
            .filter(|o| matches!(o, Op::AddItem { .. }))
            .count();
        ops.push(Op::AddItem {
            index: doc.items.len() + added,
            item: Box::new(ProjectItem::Composition(comp)),
        });
        ops.push(self.file_into_folder_op(folder_id, id, &ops));
        self.commit(Op::Batch { ops });
        // A brand-new comp opens as the active Timeline tab and the viewed
        // comp, so it is the target for the next add. Without this, items kept
        // landing in a comp opened earlier: the old `preview_comp` lagged
        // behind `selected_comp`, and there was no tab to switch back with.
        self.open_comp(id);
        if let Some(item) = dialog.pending_item {
            // A pending drop that was part of a multi-selection brings the
            // whole set into the fresh comp (A3).
            let items = self.drag_expansion(item);
            self.add_items_to_comp(&items);
        }
    }

    /// Make `id` the active comp: shown in the Timeline and the Viewer, and
    /// listed as an open Timeline tab. The Timeline shows one tab per open comp
    /// (07-UI-SPEC §4), so opening a comp adds its tab rather than replacing
    /// whichever comp was open before. No-op for a non-comp id.
    pub fn open_comp(&mut self, id: Uuid) {
        if self.store.snapshot().comp(id).is_none() {
            return;
        }
        if !self.open_comps.contains(&id) {
            self.open_comps.push(id);
        }
        self.selected_comp = Some(id);
        self.selected_item = Some(id);
        self.preview_comp = Some(id);
        self.preview_item = None;
        self.preview_frame = 0;
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Close an open comp's Timeline tab. The comp itself stays in the project;
    /// only its tab closes. If it was the active tab, its neighbour takes over
    /// (or the Timeline empties when the last tab closes).
    pub fn close_comp_tab(&mut self, id: Uuid) {
        let Some(pos) = self.open_comps.iter().position(|c| *c == id) else {
            return;
        };
        self.open_comps.remove(pos);
        if self.selected_comp != Some(id) {
            return;
        }
        // Prefer the tab that shifted into this slot (the one to the right),
        // else the new last tab, else nothing left to show.
        match self
            .open_comps
            .get(pos)
            .or_else(|| self.open_comps.last())
            .copied()
        {
            Some(next) => {
                self.selected_comp = Some(next);
                self.selected_item = Some(next);
                self.preview_comp = Some(next);
                self.preview_item = None;
                self.preview_frame = 0;
                #[cfg(feature = "media")]
                self.refresh_preview();
            }
            None => {
                self.selected_comp = None;
                self.preview_comp = None;
            }
        }
    }

    /// Add any project item to the active comp as a new top layer (footage,
    /// solid, or another comp as a Precomp — the drag-and-drop entry point).
    pub fn add_item_to_comp(&mut self, item_id: Uuid) {
        let doc = self.store.snapshot();
        match doc.item(item_id) {
            Some(ProjectItem::Footage(_)) => self.add_footage_to_comp(item_id),
            Some(ProjectItem::Composition(_)) => self.add_precomp_to_comp(item_id),
            Some(ProjectItem::Solid(_)) => self.add_solid_def_layer(item_id),
            _ => {}
        }
    }

    /// Add several project items to the active comp in one gesture (A3: dragging
    /// a multi-selection in). Each lands as a new top layer; they stack in
    /// reverse of the given order (every insert goes to index 0), matching how
    /// dropping items one after another would stack them. Folders are skipped
    /// (there is nothing to instantiate) and a Precomp that would loop is
    /// rejected per-item by `add_precomp_to_comp`.
    pub fn add_items_to_comp(&mut self, ids: &[Uuid]) {
        for id in ids {
            self.add_item_to_comp(*id);
        }
    }

    /// The items one dragged project item stands for (A3): the whole
    /// multi-selection when the dragged item is part of it, else just itself.
    /// Every drop-into-a-comp path expands through this, so dragging one of a
    /// multi-selection brings the whole set in.
    pub fn drag_expansion(&self, dropped: Uuid) -> Vec<Uuid> {
        let sel = self.project_selection();
        if sel.len() > 1 && sel.contains(&dropped) {
            sel
        } else {
            vec![dropped]
        }
    }

    /// The Project panel's effective selection (A3): the multi-selection set if
    /// one is built, otherwise the single `selected_item` (or nothing).
    pub fn project_selection(&self) -> Vec<Uuid> {
        if self.selected_items.is_empty() {
            self.selected_item.into_iter().collect()
        } else {
            self.selected_items.clone()
        }
    }

    /// Is `id` shown as selected in the Project panel (A3)? True if it is in the
    /// multi-selection, or (when there is no multi-selection) it is the single
    /// `selected_item`.
    pub fn is_item_selected(&self, id: Uuid) -> bool {
        if self.selected_items.is_empty() {
            self.selected_item == Some(id)
        } else {
            self.selected_items.contains(&id)
        }
    }

    /// After applying an effect (owner): select the fresh effect — every one of
    /// its parameter rows joins the selection, exactly as clicking its title
    /// does (T6) — and ask the shell to bring the Effect Controls tab to the
    /// front so the user lands on its controls.
    pub fn focus_applied_effect(&mut self, layer: Uuid, effect: usize, n_params: usize) {
        use super::{PropRow, PropSel};
        self.selected_layer = Some(layer);
        self.selected_props = (0..n_params)
            .map(|param| PropSel {
                layer,
                row: PropRow::Effect { effect, param },
            })
            .collect();
        self.selected_prop = self.selected_props.first().copied();
        self.focus_effects_tab = true;
    }

    /// Plain click in the Project panel: select just `id` (A3 clears any
    /// multi-selection).
    pub fn select_project_item(&mut self, id: Uuid) {
        self.selected_item = Some(id);
        self.selected_items.clear();
    }

    /// Ctrl/Shift-click in the Project panel (A3): toggle `id` in the
    /// multi-selection, seeding the set from the current single selection so the
    /// first Ctrl-click keeps what was already highlighted. `id` becomes the
    /// primary (the info header follows it).
    pub fn toggle_project_item(&mut self, id: Uuid) {
        if self.selected_items.is_empty() {
            if let Some(prev) = self.selected_item {
                self.selected_items.push(prev);
            }
        }
        if let Some(pos) = self.selected_items.iter().position(|x| *x == id) {
            self.selected_items.remove(pos);
        } else {
            self.selected_items.push(id);
        }
        self.selected_item = Some(id);
    }

    /// Add a layer referencing an existing SolidDef (dragging a solid asset
    /// back into a comp — the def dedupes, no new asset).
    pub fn add_solid_def_layer(&mut self, def_id: Uuid) {
        use lumit_core::model::{Layer, LayerKind, Switches};
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let (Some(comp), Some(def)) = (doc.comp(comp_id), doc.solid(def_id)) else {
            return;
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: def.name.clone(),
            kind: LayerKind::Solid { def: def_id },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            // Anchor at the solid's own centre, placed at the comp centre
            // (FX-20, K-150).
            transform: centred_transform(
                f64::from(def.width),
                f64::from(def.height),
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

    /// Manual New composition: always the dialogue (K-068 flow).
    pub fn new_composition(&mut self) {
        self.open_new_comp_dialog(None);
    }

    /// True when `candidate` sits inside `ancestor`'s folder subtree.
    fn folder_contains(doc: &Document, ancestor: Uuid, candidate: Uuid) -> bool {
        let mut stack = vec![ancestor];
        let mut seen = Vec::new();
        while let Some(id) = stack.pop() {
            if seen.contains(&id) {
                continue; // defensive: malformed cycles never hang the UI
            }
            seen.push(id);
            if let Some(f) = doc.folder(id) {
                for c in &f.children {
                    if *c == candidate {
                        return true;
                    }
                    stack.push(*c);
                }
            }
        }
        false
    }

    /// Move an item into a folder (None = the panel root): one undo step
    /// removing it from every folder that lists it, then filing it. Dropping
    /// a folder into itself or its own subtree is refused quietly.
    pub fn move_item_to_folder(&mut self, item: Uuid, target: Option<Uuid>) {
        let doc = self.store.snapshot();
        if Some(item) == target {
            return;
        }
        if let Some(t) = target {
            if doc.folder(t).is_none() || Self::folder_contains(&doc, item, t) {
                return;
            }
        }
        let mut ops = Vec::new();
        for pi in &doc.items {
            if let ProjectItem::Folder(f) = pi {
                if f.children.contains(&item) && Some(f.id) != target {
                    ops.push(Op::SetFolderChildren {
                        folder: f.id,
                        children: f.children.iter().copied().filter(|c| *c != item).collect(),
                    });
                }
            }
        }
        if let Some(t) = target {
            if let Some(f) = doc.folder(t) {
                if !f.children.contains(&item) {
                    let mut children = f.children.clone();
                    children.push(item);
                    ops.push(Op::SetFolderChildren {
                        folder: t,
                        children,
                    });
                }
            }
        }
        match ops.len() {
            0 => {}
            1 => {
                if let Some(op) = ops.pop() {
                    self.commit(op);
                }
            }
            _ => self.commit(Op::Batch { ops }),
        }
    }

    /// Create an empty folder at the panel root.
    pub fn new_folder(&mut self) {
        use lumit_core::model::Folder;
        let doc = self.store.snapshot();
        let n = doc
            .items
            .iter()
            .filter(|i| matches!(i, ProjectItem::Folder(_)))
            .count();
        self.commit(Op::AddItem {
            index: doc.items.len(),
            item: Box::new(ProjectItem::Folder(Folder {
                id: Uuid::now_v7(),
                name: format!("Folder {}", n + 1),
                children: Vec::new(),
                extra: serde_json::Map::new(),
            })),
        });
    }
}
