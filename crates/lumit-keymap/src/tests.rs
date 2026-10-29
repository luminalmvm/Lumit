#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::defaults::*;
use super::*;

fn chord(s: &str) -> Chord {
    s.parse().unwrap()
}

#[test]
fn chords_parse_case_and_order_insensitively_and_round_trip() {
    // Order and case do not matter on the way in.
    assert_eq!(chord("mod+shift+d"), chord("Shift+Mod+D"));
    // Ctrl / Cmd / Mod are the same primary modifier.
    assert_eq!(chord("Ctrl+D"), chord("Cmd+D"));
    assert_eq!(chord("Ctrl+D"), chord("Mod+D"));
    // The last token is always the key, even when it is a modifier word.
    let shift_key = chord("Shift");
    assert!(!shift_key.mods.shift && shift_key.key == "Shift");
    // Display is canonical and re-parses to the same chord.
    for s in [
        "Space",
        "Mod+D",
        "Shift+F3",
        "Mod+Alt+Shift+K",
        "=",
        "+",
        "Shift++",
        "Numpad 1",
    ] {
        let c = chord(s);
        assert_eq!(chord(&c.to_string()), c, "round-trip failed for {s}");
    }
    // Empty / modifier-only inputs error rather than panic.
    assert_eq!("".parse::<Chord>(), Err(ChordError::Empty));
    assert!(matches!(
        "Hyper+A".parse::<Chord>(),
        Err(ChordError::UnknownModifier(_))
    ));
}

#[test]
fn tab_opens_the_flowchart_from_any_panel() {
    let km = default_keymap();
    let tab: Chord = "Tab".parse().unwrap();
    for context in KeyContext::ALL {
        assert_eq!(
            km.lookup(context, &tab),
            Some(&ActionId::from("comp.flowchart")),
            "{context:?}"
        );
    }
    assert!(km.conflicts().is_empty(), "the shipped map ships clean");
}

#[test]
fn conflicts_flag_same_context_and_global_overlap_but_not_disjoint_contexts() {
    // Same context, two actions on one chord → conflict.
    let mut km = Keymap::default();
    km.bind(KeyContext::Timeline, chord("Mod+E"), "a".into());
    // bind() replaces the exact (context, chord), so push a second directly.
    km.bindings.push(Binding {
        context: KeyContext::Timeline,
        chord: chord("Mod+E"),
        action: "b".into(),
    });
    assert_eq!(km.conflicts().len(), 1);

    // Global under a scoped binding on the same chord → NOT a conflict:
    // the panel gets first refusal and Global is the fallback, so the
    // chord runs one action and which one is never in doubt. It is
    // reported as a *shadow* instead, because the app-wide meaning does
    // stop working in that one panel.
    let mut km = Keymap::default();
    km.bind(KeyContext::Global, chord("G"), "global".into());
    km.bind(KeyContext::Timeline, chord("G"), "timeline".into());
    assert!(km.conflicts().is_empty());
    let shadows = km.shadows();
    assert_eq!(shadows.len(), 1);
    assert_eq!(shadows[0].context, KeyContext::Timeline);
    assert_eq!(shadows[0].action, ActionId::from("timeline"));
    assert_eq!(shadows[0].shadowed, ActionId::from("global"));
    assert_eq!(
        km.lookup(KeyContext::Timeline, &chord("G")),
        Some(&ActionId::from("timeline"))
    );
    assert_eq!(
        km.lookup(KeyContext::Viewer, &chord("G")),
        Some(&ActionId::from("global"))
    );

    // Two *different* scoped contexts on the same chord → NOT a conflict
    // (the chord means different things in different panels).
    let mut km = Keymap::default();
    km.bind(KeyContext::Timeline, chord("H"), "timeline".into());
    km.bind(KeyContext::Viewer, chord("H"), "viewer".into());
    assert!(km.conflicts().is_empty());

    // The same action bound twice is neither a conflict nor a shadow.
    let mut km = Keymap::default();
    km.bind(KeyContext::Global, chord("Mod+S"), "file.save".into());
    km.bind(KeyContext::Timeline, chord("Mod+S"), "file.save".into());
    assert!(km.conflicts().is_empty());
    assert!(km.shadows().is_empty());
}

/// `L` reveals a layer's Audio in the Timeline and shuttles forward
/// everywhere else. That, the Cut timeline's editing keys and a camera
/// tool's Look at all are the shadows the default ships with, and they
/// are deliberate.
#[test]
fn the_default_gives_the_timeline_l_and_leaves_the_shuttle_elsewhere() {
    let km = default_keymap();
    assert_eq!(
        km.lookup(KeyContext::Timeline, &chord("L")),
        Some(&ActionId::from("reveal.audio"))
    );
    assert_eq!(
        km.lookup(KeyContext::Viewer, &chord("L")),
        Some(&ActionId::from("playback.shuttle.forward"))
    );
    // The After Effects habit reaches the same cycle.
    assert_eq!(
        km.lookup(KeyContext::Timeline, &chord("Shift+L")),
        Some(&ActionId::from("reveal.audio"))
    );
    let shadows = km.shadows();
    assert_eq!(
        shadows
            .iter()
            .map(|s| s.chord.to_string())
            .collect::<Vec<_>>(),
        [
            "Mod+Shift+F",
            "L",
            "I",
            "O",
            ",",
            ".",
            "B",
            "N",
            "Mod+K",
            "Mod+Shift+K"
        ],
        "the deliberate shadows, and no others"
    );
}

#[test]
fn the_default_keymap_covers_the_contexts_and_resolves() {
    let km = default_keymap();
    // A representative binding from each context resolves as expected.
    assert_eq!(
        km.lookup(KeyContext::Global, &chord("Space")),
        Some(&"playback.toggle".into())
    );
    assert_eq!(
        km.lookup(KeyContext::Tools, &chord("V")),
        Some(&"tool.select".into())
    );
    assert_eq!(
        km.lookup(KeyContext::Timeline, &chord("Mod+D")),
        Some(&"layer.duplicate".into())
    );
    assert_eq!(
        km.lookup(KeyContext::Viewer, &chord("Mod+=")),
        Some(&"viewer.zoom.in".into())
    );
    assert_eq!(
        km.lookup(KeyContext::Graph, &chord("F9")),
        Some(&"graph.ease".into())
    );
    assert_eq!(
        km.lookup(KeyContext::Panels, &chord("Mod+F")),
        Some(&"panel.search.focus".into())
    );
    // All nine workspace switches are present.
    for d in 1..=9u8 {
        assert!(km
            .search(&format!("workspace.switch.{d}"))
            .iter()
            .any(|b| b.context == KeyContext::Global));
    }
}

/// **A keymap stored by an older build must not hide a new action**. This
/// is the bug the owner hit: `Ctrl+C` did nothing in their app while every
/// test passed, because their saved keymap — written before `edit.copy`
/// existed — replaced the whole map on start, and an action that is not in
/// the file had no chord at all.
#[test]
fn a_stored_keymap_keeps_the_defaults_for_actions_it_never_heard_of() {
    // Their file, in miniature: today's map with the copy family removed,
    // and one chord moved so the file's own opinions can be checked too.
    let mut older = default_keymap();
    older
        .bindings
        .retain(|b| !matches!(b.action.0.as_str(), "edit.copy" | "edit.cut" | "edit.paste"));
    older.rebind_action(
        KeyContext::Global,
        &"edit.undo".into(),
        "Mod+Alt+Z".parse().unwrap(),
    );
    // And one they deliberately took away.
    older.unbind_action(KeyContext::Global, &"marker.add".into());

    let restored = with_new_defaults(older);
    assert_eq!(
        restored.lookup(KeyContext::Global, &"Mod+C".parse().unwrap()),
        Some(&"edit.copy".into()),
        "an action added since the file was written takes its default"
    );
    assert_eq!(
        restored.binding_for(KeyContext::Global, &"edit.undo".into()),
        Some(&"Mod+Alt+Z".parse().unwrap()),
        "and the file's own rebinding still wins"
    );
    assert_eq!(
        restored.binding_for(KeyContext::Global, &"marker.add".into()),
        None,
        "a key taken away on purpose stays away — that is what unbound is for"
    );
}

#[test]
fn the_default_keymap_is_conflict_free() {
    assert!(
        default_keymap().conflicts().is_empty(),
        "the shipped default must not ship with clashes"
    );
    assert!(after_effects_preset().conflicts().is_empty());
    assert!(vegas_preset().conflicts().is_empty());
}

/// The After Effects preset takes the shuttle off J/K/L, and a restart
/// must not hand it back on top of the keyframe keys.
#[test]
fn the_presets_survive_the_stored_file() {
    let restored = with_new_defaults(after_effects_preset());
    assert!(restored.conflicts().is_empty());
    assert_eq!(
        restored.lookup(KeyContext::Viewer, &chord("L")),
        None,
        "the shuttle stays off"
    );
    assert_eq!(restored, after_effects_preset());

    // The Vegas preset moves keys about and takes none away, so nothing
    // of the default's comes back beside it.
    let restored = with_new_defaults(vegas_preset());
    assert!(restored.conflicts().is_empty());
    assert_eq!(restored, vegas_preset());
}

#[test]
fn the_wheel_modifiers_ship_as_the_spec_says_and_travel_in_the_file() {
    let mut km = default_keymap();
    assert_eq!(
        km.wheel.modifier(WheelAction::ZoomTime),
        WheelModifier::Ctrl
    );
    assert_eq!(
        km.wheel.modifier(WheelAction::ScrollSideways),
        WheelModifier::Shift
    );
    assert_eq!(
        km.wheel.modifier(WheelAction::ZoomValues),
        WheelModifier::Alt
    );
    assert_eq!(
        km.wheel.modifier(WheelAction::DropperSample),
        WheelModifier::Shift
    );
    assert_eq!(
        km.wheel.modifier(WheelAction::CameraDolly),
        WheelModifier::Alt
    );

    km.wheel.set(WheelAction::ZoomTime, WheelModifier::Alt);
    let json = serde_json::to_string(&km).unwrap();
    let back: Keymap = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.wheel.modifier(WheelAction::ZoomTime),
        WheelModifier::Alt
    );

    // A file from before the wheel was remappable still reads.
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value.as_object_mut().unwrap().remove("wheel");
    let older: Keymap = serde_json::from_value(value).unwrap();
    assert_eq!(older.wheel, WheelKeys::default());
}

#[test]
fn the_handle_modifier_ships_as_alt_and_travels_in_the_file() {
    let mut km = default_keymap();
    assert_eq!(km.break_handles, HandleModifier::Alt);

    km.break_handles = HandleModifier::Ctrl;
    let json = serde_json::to_string(&km).unwrap();
    let back: Keymap = serde_json::from_str(&json).unwrap();
    assert_eq!(back.break_handles, HandleModifier::Ctrl);

    // A file from before the modifier could be chosen still reads.
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value.as_object_mut().unwrap().remove("break_handles");
    let older: Keymap = serde_json::from_value(value).unwrap();
    assert_eq!(older.break_handles, HandleModifier::Alt);
}

/// Settings → Keymap shows a description, never a raw id, so every action
/// the app ships needs one. This is the test that fails when an action is
/// added and its row forgotten — the table would silently start showing
/// `layer.something.new` to a user who has never read the source.
#[test]
fn every_shipped_action_has_a_written_description() {
    for km in [default_keymap(), after_effects_preset(), vegas_preset()] {
        for b in &km.bindings {
            let described = b.action.description();
            assert_ne!(
                described, b.action.0,
                "{} falls through to its id; give it a description",
                b.action
            );
            assert!(
                !described.is_empty(),
                "{} has an empty description",
                b.action
            );
        }
    }
    // The fallback still answers for an action this build does not know,
    // rather than showing nothing.
    assert_eq!(
        ActionId::from("some.future.action").description(),
        "some.future.action"
    );
    // Workspaces are generated, not listed, so they are described by rule.
    assert_eq!(
        ActionId::from("workspace.switch.4").description(),
        "Switch to workspace 4"
    );
}

/// The table is one row per (context, action), so rebinding a row must move
/// that action's chord — not add a second one. A `bind` here would leave the
/// old key working with nothing on screen to say so.
#[test]
fn rebinding_an_action_moves_its_chord_rather_than_adding_one() {
    let mut km = default_keymap();
    let action = ActionId::from("layer.duplicate");
    let was = km
        .binding_for(KeyContext::Timeline, &action)
        .expect("the default binds it")
        .clone();
    assert_eq!(was, chord("Mod+D"));

    km.rebind_action(KeyContext::Timeline, &action, chord("Mod+Alt+D"));
    assert_eq!(
        km.binding_for(KeyContext::Timeline, &action),
        Some(&chord("Mod+Alt+D"))
    );
    assert_eq!(
        km.lookup(KeyContext::Timeline, &was),
        None,
        "the old chord stopped working when the row changed"
    );
    assert_eq!(
        km.bindings
            .iter()
            .filter(|b| b.context == KeyContext::Timeline && b.action == action)
            .count(),
        1,
        "one row, one chord"
    );
}

/// The search box sits above a table of descriptions, so it has to match
/// what the reader can see, not only the ids underneath.
#[test]
fn search_matches_the_words_the_table_shows() {
    let km = default_keymap();
    let hits = km.search("command palette");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].action.0, "palette.open");
    // And still matches ids and chord text (the original contract).
    assert!(km
        .search("Shift+F3")
        .iter()
        .any(|b| b.action.0 == "graph.toggle"));
    assert!(km
        .search("tool.pen")
        .iter()
        .any(|b| b.action.0 == "tool.pen"));
}
