//! Dock, keyframe-graph, timecode and value-lens tests for the shell
//! (moved verbatim from mod.rs).

use super::*;

/// Drive one widget through hover → press → release and report whether it
/// registered a click. Used to prove which drag-source pattern still lets a
/// plain click through (egui's `dnd_drag_source` does not).
fn simulate_click(build: impl Fn(&mut egui::Ui) -> egui::Response) -> bool {
    let ctx = egui::Context::default();
    let rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let clicked = std::cell::Cell::new(false);
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
                let r = build(ui);
                rect.set(r.rect);
                if r.clicked() {
                    clicked.set(true);
                }
            });
        });
    };
    let m = egui::Modifiers::default();
    let btn = egui::PointerButton::Primary;
    run(vec![]); // lay out so the widget rect is known
    let pos = rect.get().center();
    run(vec![egui::Event::PointerMoved(pos)]); // hover
    run(vec![egui::Event::PointerButton {
        pos,
        button: btn,
        pressed: true,
        modifiers: m,
    }]);
    run(vec![egui::Event::PointerButton {
        pos,
        button: btn,
        pressed: false,
        modifiers: m,
    }]);
    clicked.get()
}

/// Drive one widget through two quick clicks and report whether it saw a
/// double-click (how a comp row opens its comp).
fn simulate_double_click(build: impl Fn(&mut egui::Ui) -> egui::Response) -> bool {
    let ctx = egui::Context::default();
    let rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let dbl = std::cell::Cell::new(false);
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
                let r = build(ui);
                rect.set(r.rect);
                if r.double_clicked() {
                    dbl.set(true);
                }
            });
        });
    };
    let m = egui::Modifiers::default();
    let btn = egui::PointerButton::Primary;
    run(vec![]); // lay out
    let pos = rect.get().center();
    run(vec![egui::Event::PointerMoved(pos)]);
    for _ in 0..2 {
        run(vec![egui::Event::PointerButton {
            pos,
            button: btn,
            pressed: true,
            modifiers: m,
        }]);
        run(vec![egui::Event::PointerButton {
            pos,
            button: btn,
            pressed: false,
            modifiers: m,
        }]);
    }
    dbl.get()
}

/// A comp row opens its comp on a double-click, so the draggable row must
/// report `double_clicked()` (it senses click as well as drag).
#[test]
fn a_draggable_row_reports_a_double_click() {
    assert!(simulate_double_click(|ui| draggable_row(
        ui,
        egui::Id::new("row"),
        1u32,
        false,
        "row"
    )));
}

/// The Project panel opens comps and previews footage on a row click. egui's
/// `dnd_drag_source` puts a drag-sensing overlay on top of its contents, so
/// the click never reaches either the outer or the inner response — the row
/// looked dead. A single widget that senses click *and* drag keeps both,
/// which is what [`draggable_row`] uses.
#[test]
fn a_row_that_is_both_clickable_and_draggable_still_clicks() {
    // Control: a plain button clicks under this simulation.
    assert!(simulate_click(|ui| ui.button("x")));
    // The old pattern: the drag overlay eats the click.
    assert!(!simulate_click(|ui| {
        ui.dnd_drag_source(egui::Id::new("s"), 1u32, |ui| {
            ui.selectable_label(false, "x")
        })
        .response
    }));
    // The fix: one widget sensing click+drag still reports the click.
    assert!(simulate_click(|ui| draggable_row(
        ui,
        egui::Id::new("row"),
        1u32,
        false,
        "x"
    )));
}

/// The other half of [`draggable_row`]: dragging it must still deliver its
/// payload to a drop target, so dropping footage/comps into the Timeline,
/// Viewer or onto "+ Composition" keeps working after the click fix.
#[test]
fn dragging_a_row_delivers_its_payload_to_a_drop_target() {
    let ctx = egui::Context::default();
    let payload = uuid::Uuid::from_u128(0x1234_5678);
    let src_rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let zone_rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let got = std::cell::Cell::new(None);
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
                let src = draggable_row(ui, egui::Id::new("src"), payload, false, "source");
                src_rect.set(src.rect);
                ui.add_space(60.0);
                let (zr, zresp) =
                    ui.allocate_exact_size(egui::vec2(120.0, 40.0), egui::Sense::hover());
                zone_rect.set(zr);
                if let Some(p) = zresp.dnd_release_payload::<uuid::Uuid>() {
                    got.set(Some(*p));
                }
            });
        });
    };
    let m = egui::Modifiers::default();
    let btn = egui::PointerButton::Primary;
    run(vec![]); // lay out
    let from = src_rect.get().center();
    let to = zone_rect.get().center();
    run(vec![egui::Event::PointerMoved(from)]); // hover source
    run(vec![egui::Event::PointerButton {
        pos: from,
        button: btn,
        pressed: true,
        modifiers: m,
    }]);
    run(vec![egui::Event::PointerMoved(to)]); // drag across (past threshold)
    run(vec![egui::Event::PointerButton {
        pos: to,
        button: btn,
        pressed: false,
        modifiers: m,
    }]);
    assert_eq!(
        got.get(),
        Some(payload),
        "the drop target received the drag"
    );
}

/// Double-clicking empty Project-panel space opens Import, but double-clicking
/// a row must not — the row (drawn on top) claims the click. Mirrors the
/// backdrop-under-rows layout `project_panel` uses.
#[test]
fn backdrop_double_click_fires_only_off_the_rows() {
    fn scene(pick: impl Fn(egui::Rect, egui::Rect) -> egui::Pos2) -> bool {
        let ctx = egui::Context::default();
        let bg_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let row_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let bg_dbl = std::cell::Cell::new(false);
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
                    let full = ui.available_rect_before_wrap();
                    // Backdrop first (under the row), exactly as project_panel.
                    let bg = ui.interact(full, egui::Id::new("bg"), egui::Sense::click());
                    bg_rect.set(full);
                    let row = draggable_row(ui, egui::Id::new("row"), 1u32, false, "row");
                    row_rect.set(row.rect);
                    if bg.double_clicked() {
                        bg_dbl.set(true);
                    }
                });
            });
        };
        let m = egui::Modifiers::default();
        let btn = egui::PointerButton::Primary;
        run(vec![]); // lay out
        let pos = pick(bg_rect.get(), row_rect.get());
        run(vec![egui::Event::PointerMoved(pos)]);
        for _ in 0..2 {
            run(vec![egui::Event::PointerButton {
                pos,
                button: btn,
                pressed: true,
                modifiers: m,
            }]);
            run(vec![egui::Event::PointerButton {
                pos,
                button: btn,
                pressed: false,
                modifiers: m,
            }]);
        }
        bg_dbl.get()
    }
    // Empty space below the row → Import fires.
    assert!(scene(|bg, row| egui::pos2(
        bg.center().x,
        row.bottom() + 80.0
    )));
    // On the row → the row consumes it; the backdrop stays silent.
    assert!(!scene(|_bg, row| row.center()));
}

/// Regression for the effect drop that never landed: egui keeps ONE drag
/// payload for the whole app and `dnd_release_payload` takes it out of
/// the context *before* checking its type — so the Timeline's panel-wide
/// `uuid::Uuid` item zone (registered before the layer rows, containing
/// every release over the body) swallowed each `EffectDragPayload` drop
/// whole, and the row's own reader found nothing. The scene mirrors the
/// real structure: a browser row drags the effect, a body-wide item zone
/// reads first, the layer row reads second. Unguarded, the drop dies in
/// the item zone; through [`dnd_release_of`], it reaches the row.
fn effect_drop_scene(guarded: bool) -> bool {
    let ctx = egui::Context::default();
    let src_rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let row_rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let applied = std::cell::Cell::new(false);
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
                // The Effects & Presets browser row (the drag source).
                let src = draggable_row(
                    ui,
                    egui::Id::new("fx-src"),
                    EffectDragPayload("lumit.blur"),
                    false,
                    "Blur",
                );
                src_rect.set(src.rect);
                ui.add_space(40.0);
                // The Timeline body: its item zone spans everything below,
                // registered before the rows exactly as `timeline_panel`
                // registers `accept_item_drop` before its ScrollArea.
                let body = ui.available_rect_before_wrap();
                let zone = ui.interact(body, egui::Id::new("item-zone"), egui::Sense::hover());
                if guarded {
                    if let Some(p) = dnd_release_of::<uuid::Uuid>(&zone) {
                        let _ = *p; // a real drop would file the item
                    }
                } else if let Some(p) = zone.dnd_release_payload::<uuid::Uuid>() {
                    let _ = *p;
                }
                // A layer row inside the body, reading the effect release.
                let (rr, rresp) =
                    ui.allocate_exact_size(egui::vec2(160.0, 20.0), egui::Sense::hover());
                row_rect.set(rr);
                if dnd_release_of::<EffectDragPayload>(&rresp).is_some() {
                    applied.set(true);
                }
            });
        });
    };
    let m = egui::Modifiers::default();
    let btn = egui::PointerButton::Primary;
    run(vec![]); // lay out
    let from = src_rect.get().center();
    let to = row_rect.get().center();
    run(vec![egui::Event::PointerMoved(from)]); // hover the browser row
    run(vec![egui::Event::PointerButton {
        pos: from,
        button: btn,
        pressed: true,
        modifiers: m,
    }]);
    run(vec![egui::Event::PointerMoved(to)]); // drag onto the layer row
    run(vec![egui::Event::PointerButton {
        pos: to,
        button: btn,
        pressed: false,
        modifiers: m,
    }]);
    applied.get()
}

#[test]
fn an_effect_drop_survives_the_item_zone_beneath_the_rows() {
    // The bug: an unguarded uuid zone under the rows eats the effect drop.
    assert!(!effect_drop_scene(false));
    // The fix: type-gated reads let the drop through to the layer row.
    assert!(effect_drop_scene(true));
}

/// Startup shows the Project tab (owner report): the dock tree persists
/// each tab group's active tab with the workspace, so whichever tab was
/// last in front — often Effect controls — greeted the next launch.
/// `Shell::new` now normalises the left group to Project once at startup.
#[test]
fn startup_brings_the_project_tab_to_the_front() {
    let mut tree = default_layout();
    let project = tile_id_of(&tree, Panel::Project).unwrap();
    let fx = tile_id_of(&tree, Panel::EffectControls).unwrap();
    // Simulate the restored workspace: Effect controls was left in front.
    let group = tree
        .tiles
        .iter()
        .find_map(|(id, tile)| match tile {
            egui_tiles::Tile::Container(egui_tiles::Container::Tabs(t))
                if t.children.contains(&project) =>
            {
                Some(*id)
            }
            _ => None,
        })
        .expect("Project sits in a tab group in the default layout");
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(t))) =
        tree.tiles.get_mut(group)
    {
        t.set_active(fx);
    }
    activate_panel_tab(&mut tree, Panel::Project);
    match tree.tiles.get(group) {
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(t))) => {
            assert_eq!(t.active, Some(project), "Project fronts the group");
        }
        _ => panic!("tab group vanished"),
    }
}

// The default workspace contains every panel, and the pop-out mechanism
// (hide the tile, show it again) round-trips — the basis of detaching a
// panel into its own window and docking it back (K-074).
#[test]
fn default_layout_has_every_panel_and_popout_round_trips() {
    let mut tree = default_layout();
    for panel in [
        Panel::Viewer,
        Panel::Project,
        Panel::Timeline,
        Panel::EffectControls,
        Panel::EffectsAndPresets,
        Panel::Scopes(ScopeKind::default()),
        Panel::Hierarchy,
    ] {
        let id = tile_id_of(&tree, panel).expect("panel present in default layout");
        assert!(tree.tiles.is_visible(id), "{panel:?} should start visible");
    }

    let project = tile_id_of(&tree, Panel::Project).unwrap();
    tree.tiles.set_visible(project, false); // pop out
    assert!(!tree.tiles.is_visible(project));
    tree.tiles.set_visible(project, true); // dock back
    assert!(tree.tiles.is_visible(project));
}

/// K-092's three new persisted fields (`theme_mode`, `theme_shape`,
/// `animation_level`) must not break loading a workspace saved before
/// they existed — an empty JSON object stands in for the oldest
/// possible save (every persisted `Shell` field already carries
/// `#[serde(default)]`), and every new field must land on its default.
#[test]
fn shell_deserializes_a_pre_k092_save_onto_the_new_fields_defaults() {
    let shell: Shell = serde_json::from_str("{}").expect("an empty save must still load");
    assert_eq!(shell.theme_mode, crate::theme::ThemeMode::Dark);
    assert_eq!(shell.theme_shape, crate::theme::ThemeShape::Sharp);
    assert_eq!(shell.animation_level, crate::theme::AnimationLevel::All);
}

#[test]
fn a_pre_k097_theme_pick_migrates_onto_color_scheme() {
    use crate::theme::{ColorScheme, ThemeMode, ThemeVariant};
    // Old Light / Dark-blue picks survive the upgrade to `ColorScheme`.
    assert_eq!(
        migrated_scheme(ColorScheme::Dark, ThemeMode::Light, ThemeVariant::Dark),
        ColorScheme::Light
    );
    assert_eq!(
        migrated_scheme(ColorScheme::Dark, ThemeMode::Dark, ThemeVariant::DarkBlue),
        ColorScheme::DarkBlue
    );
    assert_eq!(
        migrated_scheme(ColorScheme::Dark, ThemeMode::Dark, ThemeVariant::Dark),
        ColorScheme::Dark
    );
    // A newer save's explicit scheme is never second-guessed by stale
    // legacy fields.
    assert_eq!(
        migrated_scheme(
            ColorScheme::GruvboxDark,
            ThemeMode::Light,
            ThemeVariant::DarkBlue
        ),
        ColorScheme::GruvboxDark
    );
}

#[test]
fn an_open_settings_dialog_counts_as_a_modal() {
    // Gates the active-panel focus edge: while the dialog is up its
    // backdrop owns clicks, so a press must not re-focus a panel behind
    // it (the reported click-through bug).
    assert!(!Shell::default().any_modal_open());
    let shell = Shell {
        settings_open: true,
        ..Shell::default()
    };
    assert!(shell.any_modal_open());
}

#[test]
fn color_scheme_round_trips_through_a_save() {
    // `color_scheme` persists; the legacy mode/variant are read-only
    // (skip_serializing), so a saved-then-loaded Shell keeps its scheme.
    let shell = Shell {
        color_scheme: crate::theme::ColorScheme::CatppuccinMocha,
        ..Shell::default()
    };
    let json = serde_json::to_string(&shell).unwrap();
    let back: Shell = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.color_scheme,
        crate::theme::ColorScheme::CatppuccinMocha
    );
}

// The Timeline starts as a full-width strip along the bottom: its pane is
// a direct child of the vertical root (so it is as wide as the window) and
// the last child (the bottom band) — with no tab wrapper around it, since a
// solo panel renders bare (K-086). Guards the default workspace against a
// regression back to the Timeline nested inside the Viewer's column or
// re-wrapped in a needless single-tab group.
#[test]
fn timeline_starts_full_width_along_the_bottom_as_a_bare_pane() {
    let tree = default_layout();
    let root = tree.root().expect("layout has a root");
    let egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin)) =
        tree.tiles.get(root).expect("root tile exists")
    else {
        panic!("the root should be a vertical linear container");
    };
    assert_eq!(lin.dir, egui_tiles::LinearDir::Vertical);

    let timeline = tile_id_of(&tree, Panel::Timeline).expect("timeline present");
    assert!(
        lin.children.contains(&timeline),
        "timeline pane should be a direct child of the vertical root (full width, no tab wrapper)"
    );
    assert_eq!(
        lin.children.last(),
        Some(&timeline),
        "timeline pane should be the bottom-most child"
    );
}

/// True when any tab container holds exactly one child — a lone pane that
/// would show a needless tab bar (the shape K-086 removes).
fn has_solo_tab_group(tree: &egui_tiles::Tree<Panel>) -> bool {
    tree.tiles.iter().any(|(_, t)| {
        matches!(t, egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))
                if tabs.children.len() == 1)
    })
}

// Solo panels render bare (K-086): the default layout wraps no pane in a
// single-child tab group, and the simplification pass the dock runs on
// every draw strips such wrappers from a workspace saved before this rule
// — a stale persisted layout still loads, just without the lone tabs.
// Genuine stacks keep their tab bar.
#[test]
fn solo_tab_wrappers_are_pruned_and_stacks_keep_their_tabs() {
    assert!(
        !has_solo_tab_group(&default_layout()),
        "default layout should not wrap any lone pane in a tab group"
    );

    // A stale workspace: the pre-K-086 default, with Scopes and the
    // Timeline each wrapped in a single-child tab group.
    let mut tiles = egui_tiles::Tiles::default();
    let viewer = tiles.insert_pane(Panel::Viewer);
    let project = tiles.insert_pane(Panel::Project);
    let fx = tiles.insert_pane(Panel::EffectControls);
    let fxp = tiles.insert_pane(Panel::EffectsAndPresets);
    let left = tiles.insert_tab_tile(vec![project, fx, fxp]);
    let scopes = tiles.insert_pane(Panel::Scopes(ScopeKind::default()));
    let right = tiles.insert_tab_tile(vec![scopes]);
    let upper = tiles.insert_horizontal_tile(vec![left, viewer, right]);
    let timeline = tiles.insert_pane(Panel::Timeline);
    let timeline_tabs = tiles.insert_tab_tile(vec![timeline]);
    let root = tiles.insert_vertical_tile(vec![upper, timeline_tabs]);
    let mut stale = egui_tiles::Tree::new("stale-dock", root, tiles);

    stale.simplify(&dock_simplification_options());

    assert!(
        !has_solo_tab_group(&stale),
        "the dock's simplify pass should prune single-child tab groups"
    );
    // Every panel survives the pruning…
    for panel in [
        Panel::Viewer,
        Panel::Project,
        Panel::Timeline,
        Panel::EffectControls,
        Panel::EffectsAndPresets,
        Panel::Scopes(ScopeKind::default()),
    ] {
        assert!(
            tile_id_of(&stale, panel).is_some(),
            "{panel:?} should survive simplification"
        );
    }
    // …and the genuine three-panel stack keeps its tab group.
    let project_tile = tile_id_of(&stale, Panel::Project).unwrap();
    let in_tabs = stale.tiles.iter().any(|(_, t)| {
        matches!(t, egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))
                if tabs.children.contains(&project_tile))
    });
    assert!(in_tabs, "a stacked panel keeps its tab bar");
}

/// `bare_tile_ids` (the set `DockBehavior` wraps in `bare_pane_ui`)
/// matches tab membership on the real default layout: the three
/// tab-stacked panels are excluded, the three solo ones are included.
#[test]
fn bare_tile_ids_matches_tab_membership_on_the_default_layout() {
    let tree = default_layout();
    let bare = bare_tile_ids(&tree);
    for panel in [
        Panel::Viewer,
        Panel::Timeline,
        Panel::Scopes(ScopeKind::default()),
    ] {
        let id = tile_id_of(&tree, panel).unwrap();
        assert!(bare.contains(&id), "{panel:?} should render bare");
    }
    for panel in [
        Panel::Project,
        Panel::EffectControls,
        Panel::EffectsAndPresets,
    ] {
        let id = tile_id_of(&tree, panel).unwrap();
        assert!(!bare.contains(&id), "{panel:?} is tab-stacked, not bare");
    }
}

/// `DockBehavior::gap_width`/`resize_stroke` (K-092): Sharp reproduces
/// egui_tiles' own idle-state default exactly (a `tab_bar_color`-toned
/// line at `gap_width`); Round widens the gap and paints its idle state
/// as the canvas colour instead — `tab_bar_color` itself must stay
/// untouched by shape (it's also the real tab-bar background for
/// stacked groups).
#[test]
fn dock_behavior_gap_and_resize_stroke_are_shape_aware() {
    use egui_tiles::Behavior as _;
    let style = egui::Style::default();
    let mut app = AppState::default();

    let sharp = Theme::of(crate::theme::ThemeVariant::Dark);
    let mut behavior = DockBehavior {
        theme: &sharp,
        app: &mut app,
        preview_display: None,
        pop_out: None,
        panel_rects: Vec::new(),
        tree_id: egui::Id::new("test"),
        bare_tiles: Default::default(),
    };
    assert_eq!(behavior.gap_width(&style), 1.0_f32);
    assert_eq!(
        behavior.resize_stroke(&style, egui_tiles::ResizeState::Idle),
        egui::Stroke::new(1.0_f32, behavior.tab_bar_color(&style.visuals))
    );
    // Sharp keeps the rerun-style tab-bar fill one step above the panel.
    assert_eq!(behavior.tab_bar_color(&style.visuals), sharp.surface_2);

    let round = crate::theme::Theme::for_settings(
        crate::theme::ThemeMode::Dark,
        crate::theme::ThemeVariant::Dark,
        crate::theme::ThemeShape::Round,
    );
    behavior.theme = &round;
    assert_eq!(behavior.gap_width(&style), round.tokens.tile_gap);
    assert_eq!(
        behavior.resize_stroke(&style, egui_tiles::ResizeState::Idle),
        egui::Stroke::new(round.tokens.tile_gap, round.surface_0)
    );
    // Under Round the tab bar takes the canvas colour so the pill tabs
    // read as floating chips in a strip separated from the body card.
    assert_eq!(
        behavior.tab_bar_color(&style.visuals),
        round.surface_0,
        "Round tab bar should be the canvas colour so pills float in it"
    );
}

// The comp strip's "Pop out timeline" menu hangs off the strip's background
// (a click-sensing Ui registered before the tab buttons, expanded to the
// panel's right edge). Pins the egui layering it relies on: a right-click
// on empty strip space reaches the background; a right-click on a tab
// button does not (the button, drawn on top, claims it).
#[test]
fn strip_background_takes_the_right_click_only_off_the_buttons() {
    fn scene(pick: impl Fn(egui::Rect, egui::Rect) -> egui::Pos2) -> bool {
        let ctx = egui::Context::default();
        let bg_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let btn_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let bg_secondary = std::cell::Cell::new(false);
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
                    // comp_tab_strip in miniature: a sensed background Ui,
                    // a button inside it, the width claimed to the edge.
                    let bg = ui.scope_builder(
                        egui::UiBuilder::new().sense(egui::Sense::click()),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                btn_rect.set(ui.button("Comp 1").rect);
                            });
                            let claim = egui::Rect::from_min_max(
                                ui.min_rect().left_top(),
                                egui::pos2(ui.max_rect().right(), ui.min_rect().bottom()),
                            );
                            ui.expand_to_include_rect(claim);
                        },
                    );
                    bg_rect.set(bg.response.rect);
                    if bg.response.secondary_clicked() {
                        bg_secondary.set(true);
                    }
                });
            });
        };
        let m = egui::Modifiers::default();
        let btn = egui::PointerButton::Secondary;
        run(vec![]); // lay out twice so the background's rect has settled
        run(vec![]);
        let pos = pick(bg_rect.get(), btn_rect.get());
        run(vec![egui::Event::PointerMoved(pos)]);
        run(vec![egui::Event::PointerButton {
            pos,
            button: btn,
            pressed: true,
            modifiers: m,
        }]);
        run(vec![egui::Event::PointerButton {
            pos,
            button: btn,
            pressed: false,
            modifiers: m,
        }]);
        bg_secondary.get()
    }
    // Empty space right of the tab → the background sees the right-click.
    assert!(scene(|bg, btn| egui::pos2(
        (btn.right() + bg.right()) * 0.5,
        btn.center().y
    )));
    // On the tab button → the button wins; the background stays silent.
    assert!(!scene(|_bg, btn| btn.center()));
}

/// `bare_pane_ui`'s right-click affordance (owner request, K-091 era): a
/// bare pane's whole rect senses right-click for "pop out into its own
/// window", registered before the panel's own content — mirroring
/// `strip_background_takes_the_right_click_only_off_the_buttons`. A
/// button the content draws anywhere in the pane must still claim
/// right-clicks over its own footprint.
#[test]
fn bare_pane_background_right_click_pops_out_only_off_content() {
    fn scene(pick: impl Fn(egui::Rect, egui::Rect) -> egui::Pos2) -> bool {
        let ctx = egui::Context::default();
        let bg_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let btn_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let bg_secondary = std::cell::Cell::new(false);
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
                    // bare_pane_ui in miniature: the whole-pane background
                    // scope, one button drawn inside it as its content.
                    let bg = ui.scope_builder(
                        egui::UiBuilder::new().sense(egui::Sense::click()),
                        |ui| {
                            let pane_rect = ui.max_rect();
                            btn_rect.set(ui.button("content").rect);
                            ui.expand_to_include_rect(pane_rect);
                        },
                    );
                    bg_rect.set(bg.response.rect);
                    if bg.response.secondary_clicked() {
                        bg_secondary.set(true);
                    }
                });
            });
        };
        let m = egui::Modifiers::default();
        let btn = egui::PointerButton::Secondary;
        run(vec![]); // lay out twice so the background's rect has settled
        run(vec![]);
        let pos = pick(bg_rect.get(), btn_rect.get());
        run(vec![egui::Event::PointerMoved(pos)]);
        run(vec![egui::Event::PointerButton {
            pos,
            button: btn,
            pressed: true,
            modifiers: m,
        }]);
        run(vec![egui::Event::PointerButton {
            pos,
            button: btn,
            pressed: false,
            modifiers: m,
        }]);
        bg_secondary.get()
    }
    // Empty pane space below the button → the background pops out.
    assert!(scene(|bg, btn| egui::pos2(
        btn.center().x,
        (btn.bottom() + bg.bottom()) * 0.5
    )));
    // On the content button → the button wins; the background stays silent.
    assert!(!scene(|_bg, btn| btn.center()));
}

/// `bare_pane_ui`'s drag grip: a small top-right handle senses
/// `click_and_drag` and, on `drag_started`, hands off to egui_tiles' own
/// tile-drag id (`TileId::egui_id`) so the pane re-docks like a dragged
/// tab. It is interacted *after* the content (mirroring how the
/// right-click background's content wins its own footprint above), so a
/// widget the panel draws underneath the grip's corner keeps its clicks
/// everywhere *else*, and the grip still claims drags starting in its
/// own tiny footprint. This is deliberately NOT a drag sense spread over
/// a wider region: an earlier version tried exactly that (a top-strip
/// `click_and_drag` interact registered *before* content) and a plain
/// button drawn inside it had its click hijacked into a pane-drag once
/// the pointer moved past the click threshold — `Response::dragged()`
/// only needs the *sense*, not being topmost, so a click-only sibling
/// has nothing to contest a drag-sensing one with (confirmed against
/// egui_tiles' own tab bar, whose background AND its individual tab
/// buttons both sense `click_and_drag` — that symmetry is what lets one
/// yield to the other; a plain button can't).
#[test]
fn bare_pane_drag_grip_moves_the_pane_only_from_its_own_corner() {
    fn scene(press_pos: impl Fn(egui::Rect, egui::Rect) -> egui::Pos2) -> (bool, bool) {
        let ctx = egui::Context::default();
        let content_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let grip_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let content_clicked = std::cell::Cell::new(false);
        let tree_id = egui::Id::new("test-dock");
        let tile_id = egui_tiles::TileId::from_u64(7);
        let expect_id = tile_id.egui_id(tree_id);
        let dragged = std::cell::Cell::new(false);
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
                    let rect = ui.max_rect();
                    // Content: a plain button spanning the corner where
                    // the grip will sit, mirroring a panel whose header
                    // reaches the same area (Project's toolbar row).
                    let content = ui.put(
                        egui::Rect::from_min_size(
                            rect.left_top(),
                            egui::vec2(200.0, BARE_PANE_GRIP_SIZE),
                        ),
                        egui::Button::new("content"),
                    );
                    content_rect.set(content.rect);
                    if content.clicked() {
                        content_clicked.set(true);
                    }
                    // The grip, added last exactly as bare_pane_ui does.
                    let grip = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - BARE_PANE_GRIP_SIZE, rect.top()),
                        egui::vec2(BARE_PANE_GRIP_SIZE, BARE_PANE_GRIP_SIZE),
                    );
                    grip_rect.set(grip);
                    let handle = ui.interact(
                        grip,
                        ui.id().with("bare-pane-grip"),
                        egui::Sense::click_and_drag(),
                    );
                    if handle.drag_started() {
                        ui.ctx().set_dragged_id(expect_id);
                    }
                    dragged.set(ctx.is_being_dragged(expect_id));
                });
            });
        };
        let m = egui::Modifiers::default();
        let btn = egui::PointerButton::Primary;
        run(vec![]);
        run(vec![]);
        let pos = press_pos(content_rect.get(), grip_rect.get());
        run(vec![egui::Event::PointerMoved(pos)]);
        run(vec![egui::Event::PointerButton {
            pos,
            button: btn,
            pressed: true,
            modifiers: m,
        }]);
        // Move past the drag threshold while held (a plain press+release
        // at the same spot is a click, not a drag — same distinction
        // `dragging_a_row_delivers_its_payload_to_a_drop_target` relies on).
        let moved = pos + egui::vec2(8.0, 0.0);
        run(vec![egui::Event::PointerMoved(moved)]);
        let got_dragged = dragged.get();
        run(vec![egui::Event::PointerButton {
            pos: moved,
            button: btn,
            pressed: false,
            modifiers: m,
        }]);
        (content_clicked.get(), got_dragged)
    }
    // Dragging from the grip's own corner starts the tile drag.
    let (_, dragged) = scene(|_content, grip| grip.center());
    assert!(dragged, "dragging the grip should start the tile drag");
    // Dragging from elsewhere on the content (away from the grip
    // corner) must not — the button there keeps its own interaction.
    let (clicked, dragged) = scene(|content, _grip| content.left_center() + egui::vec2(20.0, 0.0));
    assert!(!dragged, "dragging the content must not hijack a pane-drag");
    assert!(!clicked, "a drag, even off the grip, is not a click");
}

// Each keyframe's glyph codes its interpolation (graph-editor ergonomics).
#[test]
fn key_shape_codes_interpolation() {
    use lumit_core::anim::{Keyframe, SideInterp};
    let key = |i: SideInterp, o: SideInterp| Keyframe {
        time: rational_at(0.0),
        value: 0.0,
        interp_in: i,
        interp_out: o,
    };
    assert_eq!(
        key_shape(&key(SideInterp::Linear, SideInterp::Linear)),
        KeyShape::Diamond
    );
    assert_eq!(
        key_shape(&key(SideInterp::Hold, SideInterp::Linear)),
        KeyShape::Square
    );
    assert_eq!(
        key_shape(&key(
            SideInterp::Linear,
            SideInterp::Bezier {
                speed: 0.0,
                influence: 0.33
            }
        )),
        KeyShape::Circle
    );
    // Hold wins over bezier (a held key never eases out visually).
    assert_eq!(
        key_shape(&key(
            SideInterp::Hold,
            SideInterp::Bezier {
                speed: 0.0,
                influence: 0.33
            }
        )),
        KeyShape::Square
    );
}

// The time grid subdivides with zoom: gridlines never crowd under ~70 px,
// and zooming in walks the ladder down to 10 ms.
#[test]
fn time_grid_step_follows_the_zoom() {
    assert_eq!(time_grid_step(10.0), 10.0); // zoomed out: 10 s lines
    assert_eq!(time_grid_step(80.0), 1.0); // ~normal: whole seconds
    assert_eq!(time_grid_step(300.0), 0.25); // zoomed: quarter seconds
    assert_eq!(time_grid_step(10_000.0), 0.01); // way in: 10 ms
                                                // Never denser than ~70 px between lines.
    for pps in [10.0, 80.0, 300.0, 10_000.0] {
        assert!(time_grid_step(pps) * pps >= 70.0 || time_grid_step(pps) == 0.01);
    }
}

// The linked scale control keeps the x:y ratio (K-072).
#[test]
fn linked_scale_keeps_ratio() {
    assert_eq!(linked_scale(100.0, 50.0, 200.0), (200.0, 100.0)); // 2:1 kept
    assert_eq!(linked_scale(100.0, 100.0, 150.0), (150.0, 150.0)); // 1:1 kept
    assert_eq!(linked_scale(0.0, 50.0, 80.0), (80.0, 80.0)); // undefined → uniform
}

/// A keyframed test property (linear keys at the given (time, value)s).
fn keyed(keys: &[(f64, f64)]) -> lumit_core::anim::Property {
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    let mut p = Property::fixed(0.0);
    p.animation = Animation::Keyframed(
        keys.iter()
            .map(|(t, v)| Keyframe {
                time: rational_at(*t),
                value: *v,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            })
            .collect(),
    );
    p
}

// Every linked two-axis row (Scale, Position, Anchor) commits both axes
// as ONE undo step: a Batch of two SetTransformProperty ops, x then y,
// each addressing its own channel — values stay independent.
#[test]
fn two_prop_batch_sets_both_axes_in_one_undo_step() {
    use lumit_core::anim::Animation;
    use lumit_core::model::TransformProp;
    let comp = uuid::Uuid::from_u128(0xC0);
    let layer = uuid::Uuid::from_u128(0x1A);
    let op = two_prop_batch(
        comp,
        layer,
        (TransformProp::PositionX, Animation::Static(10.0)),
        (TransformProp::PositionY, Animation::Static(20.0)),
    );
    assert_eq!(
        op,
        lumit_core::Op::Batch {
            ops: vec![
                lumit_core::Op::SetTransformProperty {
                    comp,
                    layer,
                    prop: TransformProp::PositionX,
                    animation: Animation::Static(10.0),
                },
                lumit_core::Op::SetTransformProperty {
                    comp,
                    layer,
                    prop: TransformProp::PositionY,
                    animation: Animation::Static(20.0),
                },
            ],
        }
    );
}

// The linked row's navigator works the union of both axes' keys: times
// merge sorted, near-coincident keys count once, static axes add nothing.
#[test]
fn union_key_times_merges_sorted_and_dedupes() {
    use lumit_core::anim::Property;
    let tol = 0.5 / 30.0; // half a frame at 30 fps
    let x = keyed(&[(0.0, 1.0), (2.0, 3.0)]);
    let y = keyed(&[(1.0, 5.0), (2.001, 6.0)]); // 2.001 ≈ 2.0 within tol
    let times = union_key_times(&x, &y, tol);
    assert_eq!(times.len(), 3);
    assert!((times[0] - 0.0).abs() < 1e-9);
    assert!((times[1] - 1.0).abs() < 1e-9);
    assert!((times[2] - 2.0).abs() < 1e-3);
    // A static axis contributes nothing; two statics mean no navigator.
    let s = Property::fixed(7.0);
    assert_eq!(union_key_times(&x, &s, tol).len(), 2);
    assert!(union_key_times(&s, &s, tol).is_empty());
}

// Walking that union from the playhead: previous strictly before, next
// strictly after, and "on a key" within the half-frame tolerance.
#[test]
fn key_nav_targets_walks_the_union() {
    let tol = 0.5 / 30.0;
    let times = [0.0, 1.0, 2.0];
    // On the middle key: prev is 0, next is 2.
    let (prev, on, next) = key_nav_targets(&times, 1.0, tol);
    assert_eq!(prev, Some(0.0));
    assert!(on);
    assert_eq!(next, Some(2.0));
    // Between keys: nearest each side, not "on" anything.
    let (prev, on, next) = key_nav_targets(&times, 0.5, tol);
    assert_eq!(prev, Some(0.0));
    assert!(!on);
    assert_eq!(next, Some(1.0));
    // At the ends there is nowhere further to go.
    let (prev, _, _) = key_nav_targets(&times, 0.0, tol);
    assert_eq!(prev, None);
    let (_, _, next) = key_nav_targets(&times, 2.0, tol);
    assert_eq!(next, None);
}

// The linked diamond's per-axis toggle: adding upserts a key at the
// playhead on each axis; removing strips only keys at the playhead,
// freezing an axis when its last key goes and never touching a static one.
#[test]
fn toggle_key_at_keys_or_clears_one_axis() {
    use lumit_core::anim::{Animation, Property};
    let tol = 0.5 / 30.0;
    // Add on an animated axis: the playhead key joins the existing ones.
    let x = keyed(&[(0.0, 1.0), (2.0, 3.0)]);
    let Animation::Keyframed(keys) = toggle_key_at(&x, 1.0, tol, false) else {
        panic!("adding must keep the axis keyframed");
    };
    assert_eq!(keys.len(), 3);
    assert!((keys[1].time.to_f64() - 1.0).abs() < 1e-6);
    assert!((keys[1].value - 2.0).abs() < 1e-6); // the interpolated value
                                                 // Add on a static axis: it becomes keyframed at its current value.
    let s = Property::fixed(7.0);
    let Animation::Keyframed(keys) = toggle_key_at(&s, 1.0, tol, false) else {
        panic!("adding must animate a static axis");
    };
    assert!(keys.iter().any(|k| (k.time.to_f64() - 1.0).abs() < 1e-6));
    assert!(keys.iter().all(|k| (k.value - 7.0).abs() < 1e-9));
    // Remove at a key: only that key goes, the others stay.
    let Animation::Keyframed(keys) = toggle_key_at(&x, 2.0, tol, true) else {
        panic!("an axis with keys left must stay keyframed");
    };
    assert_eq!(keys.len(), 1);
    assert!((keys[0].time.to_f64()).abs() < 1e-9);
    // Removing the last key freezes the axis at its current value.
    let one = keyed(&[(1.0, 4.0)]);
    assert_eq!(toggle_key_at(&one, 1.0, tol, true), Animation::Static(4.0));
    // A static axis is left untouched by a union-driven remove.
    assert_eq!(toggle_key_at(&s, 1.0, tol, true), Animation::Static(7.0));
}

// A keyframe side reports its bezier influence, or the easy-ease third.
#[test]
fn side_influence_reads_bezier_or_defaults() {
    use lumit_core::anim::SideInterp;
    assert_eq!(
        side_influence(SideInterp::Bezier {
            speed: 5.0,
            influence: 0.5
        }),
        0.5
    );
    assert!((side_influence(SideInterp::Linear) - 1.0 / 3.0).abs() < 1e-9);
    assert!((side_influence(SideInterp::Hold) - 1.0 / 3.0).abs() < 1e-9);
}

// The tangent drag's mirroring: decided at drag start from the key's
// unification, toggled once by Alt (latched — releasing Alt mid-drag never
// snaps handles back together), and applied by apply_tangent (Mack).
#[test]
fn tangent_drag_unifies_by_default_and_alt_toggles_latched() {
    use lumit_core::anim::{Keyframe, SideInterp::Bezier, EASY_EASE};
    // The mode table: unified stays unified, Alt breaks it — and the break
    // survives Alt being released (alt_seen latches). A broken key stays
    // broken on a plain drag; Alt on a broken key re-unifies it.
    assert!(tangent_mirrors(true, false)); // unified, no Alt → mirror
    assert!(!tangent_mirrors(true, true)); // unified, Alt seen → broken
    assert!(!tangent_mirrors(false, false)); // broken, no Alt → stays broken
    assert!(tangent_mirrors(false, true)); // broken, Alt seen → re-unified

    let base = || Keyframe {
        time: rational_at(1.0),
        value: 0.0,
        interp_in: EASY_EASE, // speed 0, influence 1/3
        interp_out: EASY_EASE,
    };
    // Mirroring drag of the out handle sets both slopes; reaches preserved.
    let mut k = base();
    apply_tangent(&mut k, true, 5.0, 0.5, tangent_mirrors(true, false), None);
    assert_eq!(side_speed(k.interp_out), Some(5.0));
    assert_eq!(side_speed(k.interp_in), Some(5.0)); // mirrored
    assert!((side_influence(k.interp_out) - 0.5).abs() < 1e-9);
    assert!((side_influence(k.interp_in) - 1.0 / 3.0).abs() < 1e-9); // in reach kept
                                                                     // Alt seen during the drag breaks: only the dragged side changes.
    let mut k = base();
    apply_tangent(&mut k, true, 5.0, 0.5, tangent_mirrors(true, true), None);
    assert_eq!(side_speed(k.interp_out), Some(5.0));
    assert_eq!(side_speed(k.interp_in), Some(0.0)); // untouched
    let broken = || Keyframe {
        interp_in: Bezier {
            speed: 2.0,
            influence: 1.0 / 3.0,
        },
        interp_out: Bezier {
            speed: -3.0,
            influence: 1.0 / 3.0,
        },
        ..base()
    };
    // A broken key stays broken on a plain drag…
    let mut k = broken();
    apply_tangent(&mut k, false, 9.0, 0.4, tangent_mirrors(false, false), None);
    assert_eq!(side_speed(k.interp_in), Some(9.0));
    assert_eq!(side_speed(k.interp_out), Some(-3.0)); // stays broken
                                                      // …and an Alt-drag on it re-unifies: both sides take the dragged slope.
    let mut k = broken();
    apply_tangent(&mut k, false, 9.0, 0.4, tangent_mirrors(false, true), None);
    assert_eq!(side_speed(k.interp_in), Some(9.0));
    assert_eq!(side_speed(k.interp_out), Some(9.0)); // re-unified
    assert!((side_influence(k.interp_out) - 1.0 / 3.0).abs() < 1e-9); // own reach kept
}

// Vertical wheel maths (K-079): a plain wheel pans (span kept, view shifts);
// Ctrl-wheel zooms about the cursor value (cursor pinned, span changes).
#[test]
fn graph_vertical_pan_and_zoom() {
    // Pan: span stays 10, wheel-up shifts the whole range up by dy/height·span.
    let (lo, hi) = graph_v_pan_zoom((0.0, 10.0), 20.0, false, 5.0, 200.0);
    assert!(((hi - lo) - 10.0).abs() < 1e-9); // span preserved
    assert!((lo - 1.0).abs() < 1e-9 && (hi - 11.0).abs() < 1e-9); // shifted +1
                                                                  // Zoom in about the cursor value 5 (wheel up): cursor stays, span shrinks.
    let (zlo, zhi) = graph_v_pan_zoom((0.0, 10.0), 100.0, true, 5.0, 200.0);
    assert!(zhi - zlo < 10.0); // zoomed in
    let cursor_frac = (5.0 - zlo) / (zhi - zlo);
    assert!((cursor_frac - 0.5).abs() < 1e-9); // cursor value pinned in view
}

// The auto-fit reads tangent-handle endpoints, not just key values: a flat
// two-key curve with a steep out-handle must widen the range past the keys,
// and an in-handle widens it the other way (endpoint = v ± speed·reach).
#[test]
fn fit_includes_tangent_handle_endpoints() {
    use lumit_core::anim::{Keyframe, SideInterp};
    let key = |t: f64, i: SideInterp, o: SideInterp| Keyframe {
        time: rational_at(t),
        value: 10.0,
        interp_in: i,
        interp_out: o,
    };
    let steep = SideInterp::Bezier {
        speed: 60.0,
        influence: 0.5,
    };
    // Flat pair of keys at 10, first key's out-handle climbing at 60 u/s
    // over a reach of 0.5 · 2 s: its endpoint sits at 10 + 60·1 = 70.
    let keys = vec![
        key(0.0, SideInterp::Linear, steep),
        key(2.0, SideInterp::Linear, SideInterp::Linear),
    ];
    let (lo, hi) = fit_values_with_handles(&keys);
    assert!((lo - 10.0).abs() < 1e-9, "flat keys floor the range: {lo}");
    assert!((hi - 70.0).abs() < 1e-9, "out-handle endpoint missed: {hi}");
    // The same handle on the second key's *in* side reaches backwards and
    // downwards: endpoint 10 − 60·1 = −50.
    let keys = vec![
        key(0.0, SideInterp::Linear, SideInterp::Linear),
        key(2.0, steep, SideInterp::Linear),
    ];
    let (lo, hi) = fit_values_with_handles(&keys);
    assert!(
        (lo - (-50.0)).abs() < 1e-9,
        "in-handle endpoint missed: {lo}"
    );
    assert!((hi - 10.0).abs() < 1e-9);
    // A bezier side with no neighbour grows no handle: the last key's
    // out-side (and the first key's in-side) never widen the fit.
    let keys = vec![
        key(0.0, steep, SideInterp::Linear),
        key(2.0, SideInterp::Linear, steep),
    ];
    assert_eq!(fit_values_with_handles(&keys), (10.0, 10.0));
    // Linear keys alone reduce to the plain value min/max.
    let mut keys = vec![
        key(0.0, SideInterp::Linear, SideInterp::Linear),
        key(2.0, SideInterp::Linear, SideInterp::Linear),
    ];
    keys[1].value = 25.0;
    assert_eq!(fit_values_with_handles(&keys), (10.0, 25.0));
}

// A manual y-range answers a panel resize by keeping its value scale:
// the range grows or shrinks about its centre by the height ratio, so
// units-per-pixel hold and more height shows more curve, not a stretch.
#[test]
fn manual_range_rescales_with_plot_height() {
    // Doubling the height doubles the span about the same centre.
    let (lo, hi) = rescale_range_for_height((0.0, 10.0), 100.0, 200.0);
    assert!((lo - (-5.0)).abs() < 1e-9 && (hi - 15.0).abs() < 1e-9);
    assert!(((lo + hi) * 0.5 - 5.0).abs() < 1e-9); // centre preserved
    assert!(((hi - lo) / 200.0 - 10.0 / 100.0).abs() < 1e-9); // units/px held
                                                              // Shrinking the plot narrows the span symmetrically.
    let (slo, shi) = rescale_range_for_height((0.0, 10.0), 200.0, 100.0);
    assert!((slo - 2.5).abs() < 1e-9 && (shi - 7.5).abs() < 1e-9);
    // Degenerate heights leave the range untouched.
    assert_eq!(
        rescale_range_for_height((0.0, 10.0), 0.0, 100.0),
        (0.0, 10.0)
    );
    assert_eq!(
        rescale_range_for_height((0.0, 10.0), 100.0, 0.0),
        (0.0, 10.0)
    );
}

// A unified partner handle rotates but keeps its on-screen length when the
// dragged side steepens: partner_influence trades reach for slope so the
// pixel length reach·√(sx²+speed²·sy²) is conserved (Mack, bezier #2).
#[test]
fn partner_influence_preserves_screen_length() {
    use lumit_core::anim::SideInterp::Bezier;
    let (sx, sy, seg) = (3.0, 5.0, 2.0);
    let screen_len = |inf: f64, sp: f64| inf * seg * (sx * sx + sp * sp * sy * sy).sqrt();
    // Partner at rest (flat), dragged side goes to a steep slope.
    let partner = Bezier {
        speed: 0.0,
        influence: 1.0 / 3.0,
    };
    let before = screen_len(side_influence(partner), 0.0);
    let inf_new = partner_influence(partner, seg, 8.0, sx, sy);
    let after = screen_len(inf_new, 8.0);
    assert!((before - after).abs() < 1e-9, "{before} vs {after}");
    // A degenerate segment leaves the influence untouched.
    assert_eq!(partner_influence(partner, 0.0, 8.0, sx, sy), 1.0 / 3.0);
}

// K-070: setting a key's speed (what a speed-lens drag commits — both sides
// to Bezier{speed}) is what the derivative reads back. Guards the lossless
// round-trip promised for the speed lens.
#[test]
fn setting_key_speed_round_trips_through_the_derivative() {
    use lumit_core::anim::{evaluate, Keyframe, SideInterp};
    let target = 40.0_f64; // value-units per second at the middle key
    let side = SideInterp::Bezier {
        speed: target,
        influence: 1.0 / 3.0,
    };
    let keys = vec![
        Keyframe {
            time: rational_at(0.0),
            value: 0.0,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        },
        Keyframe {
            time: rational_at(1.0),
            value: 50.0,
            interp_in: side,
            interp_out: side,
        },
        Keyframe {
            time: rational_at(2.0),
            value: 60.0,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        },
    ];
    let h = 1.0 / 1000.0;
    let a = evaluate(&keys, 1.0 - h).unwrap();
    let b = evaluate(&keys, 1.0 + h).unwrap();
    let measured = (b - a) / (2.0 * h);
    assert!(
        (measured - target).abs() < 1.0,
        "derivative at the key was {measured}, expected ≈ {target}"
    );
}

// The Retime value lens reads source position as HH:MM:SS:FF frame timecode
// (K-075); the frame field wraps at the source fps.
#[test]
fn frame_timecode_formats_and_wraps() {
    assert_eq!(fmt_timecode_frames(0.0, 25.0), "00:00:00:00");
    assert_eq!(fmt_timecode_frames(2.0, 30.0), "00:00:02:00");
    assert_eq!(fmt_timecode_frames(1.0 / 25.0, 25.0), "00:00:00:01");
    // Frame field wraps at fps: 24/25 s is frame 24; the 25th rolls the second.
    assert_eq!(fmt_timecode_frames(24.0 / 25.0, 25.0), "00:00:00:24");
    assert_eq!(fmt_timecode_frames(1.0, 25.0), "00:00:01:00");
    // Hours / minutes / seconds compose.
    assert_eq!(fmt_timecode_frames(3661.0, 24.0), "01:01:01:00");
}

// The value-lens timecode parser is the inverse of the formatter, and
// tolerates shorter colon forms and a bare frame count.
#[test]
fn timecode_parses_and_round_trips_with_the_formatter() {
    // Full HH:MM:SS:FF → the same frame count the formatter came from.
    assert_eq!(parse_timecode_frames("00:00:02:00", 30.0), Some(60.0));
    assert_eq!(parse_timecode_frames("01:01:01:00", 24.0), Some(87864.0));
    // Shorter colon forms and a bare frame count.
    assert_eq!(parse_timecode_frames("02:14", 25.0), Some(64.0)); // SS:FF
    assert_eq!(parse_timecode_frames("1:00:00", 30.0), Some(1800.0)); // MM:SS:FF
    assert_eq!(parse_timecode_frames("72", 24.0), Some(72.0)); // frames
                                                               // Round-trips through the formatter for a spread of frame counts.
    for &(frames, fps) in &[(0.0, 25.0), (1.0, 24.0), (64.0, 25.0), (87864.0, 24.0)] {
        let s = fmt_timecode_frames(frames / fps, fps);
        assert_eq!(parse_timecode_frames(&s, fps), Some(frames), "{s} @ {fps}");
    }
    // Rubbish yields None so the drag value keeps its previous reading.
    assert_eq!(parse_timecode_frames("nope", 24.0), None);
    assert_eq!(parse_timecode_frames("", 24.0), None);
}

// Enabling the Time lens keyframe seeds identity endpoints plus a key at the
// playhead, and the resulting store passes through every value key exactly.
#[test]
fn value_key_upsert_builds_a_passthrough_with_a_playhead_key() {
    use lumit_core::retime::Retime;
    use lumit_core::Rational;
    let dur = Rational::new(4, 1).unwrap();
    let mut keys = vec![(Rational::ZERO, Rational::ZERO), (dur, dur)];
    // At 24 fps, a playhead at 1.0 s over an identity clip keys source 1.0 s.
    upsert_value_key(&mut keys, 1.0, 1.0, dur, 24.0, 24.0);
    assert_eq!(keys.len(), 3);
    let r = Retime::from_value_keyframes(&keys).unwrap();
    assert!((r.evaluate(1.0) - 1.0).abs() < 1e-9);
    // Re-keying the same frame replaces rather than duplicates it.
    upsert_value_key(&mut keys, 1.0, 2.0, dur, 24.0, 24.0);
    assert_eq!(keys.len(), 3);
    let r = Retime::from_value_keyframes(&keys).unwrap();
    assert!((r.evaluate(1.0) - 2.0).abs() < 1e-9);
}

// The value lens counts source frames at the footage's rate, not the comp's:
// source time snaps to the footage grid, and the timecode's frame field wraps
// and pads to that rate (600 fps → frames 0..599, three digits).
#[test]
fn value_lens_uses_the_source_frame_rate() {
    use lumit_core::retime::Retime;
    use lumit_core::Rational;
    let dur = Rational::new(2, 1).unwrap();
    let mut keys = vec![(Rational::ZERO, Rational::ZERO), (dur, dur)];
    // Comp at 30 fps, footage at 600 fps: a playhead 0.1 s in, keying source
    // 0.105 s, snaps source to the 600-grid (exactly 63/600 s = frame 63).
    upsert_value_key(&mut keys, 0.1, 0.105, dur, 30.0, 600.0);
    let interior = keys.iter().find(|(t, _)| *t != Rational::ZERO && *t != dur);
    let (_t, s) = interior.expect("interior key");
    assert_eq!(*s, Rational::new(63, 600).unwrap());
    assert!(Retime::from_value_keyframes(&keys).is_some());
    // The timecode reads that as frame 63, three digits wide at 600 fps.
    assert_eq!(fmt_timecode_frames(63.0 / 600.0, 600.0), "00:00:00:063");
    // A 1000 fps clip pads the frame field to four digits.
    assert_eq!(fmt_timecode_frames(5.0 / 1000.0, 1000.0), "00:00:00:0005");
}

// Regression: enabling Time keyframes with the playhead at the layer's very
// start or end re-pins an endpoint rather than adding an interior key — the
// store must still build (the stopwatch lights and the first/last keys
// show), not silently no-op.
#[test]
fn value_key_upsert_at_the_endpoints_still_builds() {
    use lumit_core::retime::Retime;
    use lumit_core::Rational;
    let dur = Rational::new(4, 1).unwrap();
    // Playhead at t = 0 on an un-retimed layer: keys stay the endpoint pair.
    let mut keys = vec![(Rational::ZERO, Rational::ZERO), (dur, dur)];
    upsert_value_key(&mut keys, 0.0, 0.0, dur, 24.0, 24.0);
    assert_eq!(keys.len(), 2);
    let r = Retime::from_value_keyframes(&keys).unwrap();
    assert!((r.evaluate(2.0) - 2.0).abs() < 1e-9); // identity pass-through
                                                   // Playhead at t = dur (and past it — upsert clamps): same story.
    let mut keys = vec![(Rational::ZERO, Rational::ZERO), (dur, dur)];
    upsert_value_key(&mut keys, 5.0, 4.0, dur, 24.0, 24.0);
    assert_eq!(keys.len(), 2);
    assert!(Retime::from_value_keyframes(&keys).is_some());
}

// K-075 2b: dragging a speed keyframe in the % lens (via speed_with_key)
// authors a ramp — the speed set is the speed read back, and the segment
// start is pinned (K-070 frame-pinning: only downstream recomputes).
#[test]
fn retime_speed_keyframe_edit_round_trips() {
    use lumit_core::retime::Retime;
    use lumit_core::Rational;
    let dur = Rational::from_f64_on_grid(2.0, 1000).unwrap();
    let base = Some(Retime::constant_speed(dur, Rational::ZERO, Rational::ONE));
    // Drag the end keyframe (t = 2 s) to 50% — a 100% → 50% ramp.
    let speed = Rational::from_f64_on_grid(0.5, 1000).unwrap();
    let edited = speed_with_key(&base, dur, 2.0, speed).expect("retime rebuilds");
    let end = edited.speed_at(2.0 - 1e-6) * 100.0;
    assert!((end - 50.0).abs() < 1.0, "end speed {end} ≈ 50");
    let start = edited.speed_at(1e-6) * 100.0;
    assert!((start - 100.0).abs() < 1.0, "start speed {start} ≈ 100");
}

// Select-on-edit: committing a property or Retime op from the timeline or
// graph points the graph at the channel that was just touched, so the curve
// follows the key you just added or moved.
#[test]
fn follow_edit_points_the_graph_at_the_touched_channel() {
    use lumit_core::anim::Animation;
    use lumit_core::model::TransformProp;
    let comp = uuid::Uuid::from_u128(0xC0);
    let layer = uuid::Uuid::from_u128(0x1A);
    let mut app = AppState::default();

    // A Retime edit selects the layer and graphs the Speed channel.
    follow_edit(
        &mut app,
        &lumit_core::Op::SetLayerRetime {
            comp,
            layer,
            retime: None,
        },
    );
    assert_eq!(app.selected_layer, Some(layer));
    assert!(app.graph_retime);

    // A transform-property edit swaps the graph to that property.
    follow_edit(
        &mut app,
        &lumit_core::Op::SetTransformProperty {
            comp,
            layer,
            prop: TransformProp::Rotation,
            animation: Animation::Static(0.0),
        },
    );
    assert_eq!(app.selected_layer, Some(layer));
    assert_eq!(app.graph_prop, Some(TransformProp::Rotation));
    assert!(!app.graph_retime);

    // A Batch follows its first property op (linked scale leads with x).
    follow_edit(
        &mut app,
        &lumit_core::Op::Batch {
            ops: vec![
                lumit_core::Op::SetTransformProperty {
                    comp,
                    layer,
                    prop: TransformProp::ScaleX,
                    animation: Animation::Static(100.0),
                },
                lumit_core::Op::SetTransformProperty {
                    comp,
                    layer,
                    prop: TransformProp::ScaleY,
                    animation: Animation::Static(100.0),
                },
            ],
        },
    );
    assert_eq!(app.graph_prop, Some(TransformProp::ScaleX));

    // Ops that touch neither kind of channel leave the graph alone.
    follow_edit(
        &mut app,
        &lumit_core::Op::RenameLayer {
            comp,
            layer: uuid::Uuid::from_u128(0x2B),
            name: "other".into(),
        },
    );
    assert_eq!(app.selected_layer, Some(layer));
    assert_eq!(app.graph_prop, Some(TransformProp::ScaleX));
    assert!(!app.graph_retime);
}

// The y-axis labels: decimals adapt to the axis span, and the unit comes
// from the property (per cent, degrees, bare for the pixel properties).
#[test]
fn y_axis_labels_format_to_span_and_unit() {
    assert_eq!(fmt_axis_value(150.0, 300.0), "150");
    assert_eq!(fmt_axis_value(1.25, 5.0), "1.2");
    assert_eq!(fmt_axis_value(0.347, 0.5), "0.35");
    use lumit_core::model::TransformProp as P;
    assert_eq!(prop_unit(P::Opacity), "%");
    assert_eq!(prop_unit(P::ScaleX), "%");
    assert_eq!(prop_unit(P::Rotation), "°");
    assert_eq!(prop_unit(P::PositionX), "");
}

/// A keyframe at (t, v) with linear sides, for the marquee tests.
fn marquee_key(t: f64, v: f64) -> lumit_core::anim::Keyframe {
    use lumit_core::anim::{Keyframe, SideInterp};
    Keyframe {
        time: rational_at(t),
        value: v,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    }
}

// Marquee selection: exactly the plotted points inside the band are hit.
#[test]
fn marquee_selects_only_the_points_inside_the_band() {
    let points = vec![
        egui::pos2(10.0, 10.0),
        egui::pos2(50.0, 50.0),
        egui::pos2(90.0, 10.0),
    ];
    let band = egui::Rect::from_min_max(egui::pos2(40.0, 40.0), egui::pos2(60.0, 60.0));
    assert_eq!(keys_in_band(&points, band), vec![1]);
    let all = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
    assert_eq!(keys_in_band(&points, all), vec![0, 1, 2]);
    let none = egui::Rect::from_min_max(egui::pos2(0.0, 90.0), egui::pos2(5.0, 95.0));
    assert!(keys_in_band(&points, none).is_empty());
}

// The relative multi-drag: one delta on the selected keys, nothing else
// touched, and a stale (out-of-range) index is a no-op — never a panic.
#[test]
fn nudge_moves_only_the_selected_keys_and_ignores_stale_indices() {
    let mut keys = vec![
        marquee_key(0.0, 10.0),
        marquee_key(1.0, 20.0),
        marquee_key(2.0, 30.0),
    ];
    nudge_selected_values(&mut keys, &[0, 2, 99], 5.0);
    assert_eq!(keys[0].value, 15.0);
    assert_eq!(keys[1].value, 20.0); // unselected: untouched
    assert_eq!(keys[2].value, 35.0);
    assert_eq!(keys[0].time, rational_at(0.0)); // times never move
}

// The absolute set (a typed value in the property row): every selected
// key lands on exactly that value, and the change flag skips no-op ops.
#[test]
fn set_all_sets_the_exact_value_and_reports_whether_anything_changed() {
    let mut keys = vec![marquee_key(0.0, 10.0), marquee_key(1.0, 20.0)];
    assert!(set_selected_values(&mut keys, &[0, 1], 42.0));
    assert_eq!(keys[0].value, 42.0);
    assert_eq!(keys[1].value, 42.0);
    assert!(!set_selected_values(&mut keys, &[0, 1], 42.0)); // already there
    assert!(!set_selected_values(&mut keys, &[7], 1.0)); // stale index: no-op
}

// A selection pins each index to its key's time: removing or inserting a
// key breaks the pins and the whole selection reads as stale (None), so
// it can never edit the wrong keyframes.
#[test]
fn a_selection_reads_stale_once_the_keys_change_underneath() {
    use crate::app_state::GraphSelection;
    let keys = vec![marquee_key(0.0, 1.0), marquee_key(1.0, 2.0)];
    let sel = GraphSelection {
        layer: uuid::Uuid::nil(),
        prop: lumit_core::model::TransformProp::PositionX,
        retime: false,
        keys: vec![(0, keys[0].time), (1, keys[1].time)],
    };
    assert_eq!(sel.indices_for(&keys), Some(vec![0, 1]));
    // A key removed: index 1 is gone.
    assert_eq!(sel.indices_for(&keys[..1]), None);
    // A key inserted between them shifts index 1 onto the wrong key.
    let shifted = vec![keys[0], marquee_key(0.5, 9.0), keys[1]];
    assert_eq!(sel.indices_for(&shifted), None);
    // Value edits keep the pins intact (times unchanged).
    let mut revalued = keys.clone();
    assert!(set_selected_values(&mut revalued, &[0, 1], 7.0));
    assert_eq!(sel.indices_for(&revalued), Some(vec![0, 1]));
}

// Moving a layer shifts in/out AND start_offset by the same delta — a move,
// not a slip: duration and the in→start_offset alignment are preserved.
#[test]
fn moving_a_layer_shifts_the_whole_span_not_slips_it() {
    use lumit_core::time::CompTime;
    let ct = |s: f64| CompTime(rational_at(s));
    let (i, o, so) = moved_span(ct(2.0), ct(5.0), ct(1.0), 1.5);
    assert!((i.0.to_f64() - 3.5).abs() < 1e-6, "in shifts by delta");
    assert!(
        (so.0.to_f64() - 2.5).abs() < 1e-6,
        "start_offset shifts too"
    );
    // Duration preserved.
    assert!(((o.0.to_f64() - i.0.to_f64()) - 3.0).abs() < 1e-6);
    // in→start_offset alignment preserved (content moves with the bar).
    assert!(((i.0.to_f64() - so.0.to_f64()) - 1.0).abs() < 1e-6);
}

// The lane-area view (07-UI-SPEC §4): zoom scales pixels-per-second and the
// view never scrolls past the comp ends.
#[test]
fn lane_view_zooms_and_clamps_the_scroll() {
    // Zoom 1: the whole comp fits; no scroll possible.
    let (ppx, start) = lane_view(1000.0, 10.0, 1.0, 5.0);
    assert!((ppx - 100.0).abs() < 1e-6);
    assert!(start.abs() < 1e-6);
    // Zoom 2: half visible, pixels double, scroll clamps to [0, dur - visible].
    let (ppx2, start2) = lane_view(1000.0, 10.0, 2.0, 100.0);
    assert!((ppx2 - 200.0).abs() < 1e-6);
    assert!((start2 - 5.0).abs() < 1e-6);
    // Zoom below 1 is clamped to 1 (can't zoom out past the whole comp).
    let (ppx3, _) = lane_view(1000.0, 10.0, 0.2, 0.0);
    assert!((ppx3 - 100.0).abs() < 1e-6);
}

// Graph mode (K-070): the curve fills exactly the lane area — the lanes'
// width, from under the ruler to just above the bottom bar, sparing the
// same 38 px strip the lane ScrollArea reserves (scrollbar + bar).
#[test]
fn graph_lane_rect_fills_the_lanes_and_spares_the_bottom_bar() {
    let r = graph_lane_rect(200.0, 800.0, 46.0, 600.0);
    assert_eq!(r.left(), 200.0);
    assert_eq!(r.right(), 1000.0);
    assert_eq!(r.top(), 46.0);
    assert_eq!(r.bottom(), 562.0);
    // A panel too short to fit the plot never inverts the rectangle.
    let tiny = graph_lane_rect(200.0, 800.0, 46.0, 50.0);
    assert!(tiny.bottom() >= tiny.top());
}

// Regression (layer move outran the cursor): a lane drag converts pixels to
// seconds at the *displayed* zoom. The same pixel delta must yield half the
// seconds at zoom 2 as at zoom 1 — the old `dx / track_w * duration` ignored
// zoom and made drags (and 6 px snap tolerances) run zoom× too fast.
#[test]
fn drag_secs_follows_the_displayed_zoom() {
    let (ppx1, _) = lane_view(1000.0, 10.0, 1.0, 0.0);
    let (ppx2, _) = lane_view(1000.0, 10.0, 2.0, 0.0);
    let at_zoom_1 = drag_secs(50.0, ppx1);
    let at_zoom_2 = drag_secs(50.0, ppx2);
    assert!(
        (at_zoom_1 - 0.5).abs() < 1e-9,
        "zoom 1: 50 px over 100 px/s"
    );
    assert!(
        (at_zoom_2 - at_zoom_1 / 2.0).abs() < 1e-9,
        "zoom 2 shows twice the pixels per second, so the same drag is half the time"
    );
    // The unzoomed conversion would have (wrongly) said 0.5 s at any zoom.
    assert!((at_zoom_2 - 0.25).abs() < 1e-9);
    // A degenerate px_per_sec never divides by zero.
    assert!(drag_secs(50.0, 0.0).is_finite());
}
