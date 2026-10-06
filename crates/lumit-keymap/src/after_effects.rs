//! Reading an After Effects shortcut file into a keymap.
//!
//! After Effects keeps its shortcuts as text: a `["Section"]` heading for each
//! panel, then one `"Command" = "(Ctrl+K)(F2)"` line per command, each pair of
//! brackets a chord. Only the commands Lumit has an action for are read, the
//! rest of the file is skipped.
//!
//! The result starts from [`after_effects_preset`], so an action the file
//! doesn't cover keeps the chord the preset gives it.

use std::fmt;

use crate::KeyContext::{Effects, Global, Graph, Panels, Project, Timeline, Tools, Viewer};
use crate::{after_effects_preset, ActionId, Chord, KeyContext, Keymap, Modifiers};

/// A keymap read from an After Effects shortcut file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AfterEffectsImport {
    pub keymap: Keymap,
    /// How many of Lumit's actions took their keys from the file.
    pub actions: usize,
}

/// The text had no After Effects command Lumit knows, so nothing was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAfterEffects;

impl fmt::Display for NotAfterEffects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("it holds no After Effects shortcuts")
    }
}

impl std::error::Error for NotAfterEffects {}

/// An After Effects section and command, and the Lumit action it stands for.
/// A command can feed two actions, and an action can take two commands.
const COMMANDS: &[(&str, &str, KeyContext, &str)] = &[
    // Transport and navigation.
    ("CCompTime", "TimeSpacebarPlay", Global, "playback.toggle"),
    (
        "CCompTime",
        "TimeStepForward",
        Global,
        "playback.frame.next",
    ),
    ("CCompTime", "TimeStepBack", Global, "playback.frame.prev"),
    (
        "CCompTime",
        "TimeStepForwardMore",
        Global,
        "playback.frame.next10",
    ),
    (
        "CCompTime",
        "TimeStepBackMore",
        Global,
        "playback.frame.prev10",
    ),
    ("CCompTime", "TimeRewind", Global, "playback.comp.start"),
    ("CCompTime", "TimeFastForward", Global, "playback.comp.end"),
    (
        "CCompTime",
        "TimeJumpToWAStart",
        Global,
        "playback.workarea.start",
    ),
    (
        "CCompTime",
        "TimeJumpToWAEnd",
        Global,
        "playback.workarea.end",
    ),
    ("CCompTime", "TimeJumpToIn", Global, "playback.layer.in"),
    ("CCompTime", "TimeJumpToOut", Global, "playback.layer.out"),
    ("CCompCompCmd", "PrevKeyframe", Global, "keyframe.prev"),
    ("CCompCompCmd", "NextKeyframe", Global, "keyframe.next"),
    ("CCompCmd", "SetWorkAreaStart", Global, "workarea.set.start"),
    ("CCompCmd", "SetWorkAreaEnd", Global, "workarea.set.end"),
    ("CCompCmd", "AddMarker", Global, "marker.add"),
    // Editing and files.
    ("CCompCmd", "Clear", Global, "edit.delete.selection"),
    ("CSwitchboard", "Cut", Global, "edit.cut"),
    ("CSwitchboard", "Copy", Global, "edit.copy"),
    ("CSwitchboard", "Paste", Global, "edit.paste"),
    ("CSwitchboard", "Undo", Global, "edit.undo"),
    ("CSwitchboard", "Redo", Global, "edit.redo"),
    ("CSwitchboard", "SelectAll", Global, "edit.select.all"),
    ("CSwitchboard", "DeselectAll", Global, "edit.deselect.all"),
    ("CSwitchboard", "New", Global, "file.new"),
    ("CSwitchboard", "Open", Global, "file.open"),
    ("CSwitchboard", "Save", Global, "file.save"),
    ("CSwitchboard", "SaveAs", Global, "file.save.as"),
    ("CSwitchboard", "ImportFootage", Global, "file.import"),
    (
        "CSwitchboard",
        "AddToAdobeMediaEncoderRenderQueue",
        Global,
        "file.export",
    ),
    (
        "CSwitchboard",
        "AddCompToRenderQueue",
        Global,
        "export.queue.add",
    ),
    ("CSwitchboard", "NewComp", Global, "comp.new"),
    ("CSwitchboard", "CompSettings", Global, "comp.settings"),
    ("CSwitchboard", "PrefsGeneral", Global, "app.settings"),
    (
        "CSwitchboard",
        "ProjectSettings",
        Global,
        "project.settings",
    ),
    (
        "AE_TopLevelWindow",
        "ToggleTabPanelMaximize",
        Global,
        "panel.maximise",
    ),
    ("CCompCompCmd", "ToggleGraph", Global, "graph.toggle"),
    // Layers.
    (
        "CSwitchboard",
        "EnableTimeRemap",
        Global,
        "layer.retime.enable",
    ),
    ("CSwitchboard", "NewSolidInComp", Global, "layer.new.solid"),
    ("CSwitchboard", "NewTextLayer", Global, "layer.new.text"),
    ("CSwitchboard", "NewCamera", Global, "layer.new.camera"),
    ("CSwitchboard", "NewLight", Global, "layer.new.light.point"),
    (
        "CSwitchboard",
        "NewEffectsLayer",
        Global,
        "layer.new.adjustment",
    ),
    ("CSwitchboard", "NewNullObject", Global, "layer.new.null"),
    // Tools. ToolPan is the pan-behind tool, which moves the anchor point.
    ("CEggAppTool", "ToolArrow", Tools, "tool.select"),
    ("CEggAppTool", "ToolHand", Tools, "tool.hand"),
    ("CEggAppTool", "ToolMagnify", Tools, "tool.zoom"),
    ("CEggAppTool", "ToolPan", Tools, "tool.anchor"),
    ("CEggAppTool", "ToolMask", Tools, "tool.shape"),
    ("CEggAppTool", "ToolPen", Tools, "tool.pen"),
    ("CEggAppTool", "ToolRotate", Tools, "tool.rotate"),
    ("CEggAppTool", "ToolText", Tools, "tool.type"),
    ("CEggAppTool", "ToolPaint", Tools, "tool.paint"),
    ("CEggAppTool", "ToolRotoBrush", Tools, "tool.roto"),
    ("CEggAppTool", "ToolPin", Tools, "tool.puppet"),
    ("CEggAppTool", "ToolCamera", Tools, "tool.camera"),
    // Timeline.
    (
        "CCompCompCmd",
        "CompTwirlPosition",
        Timeline,
        "reveal.position",
    ),
    ("CCompCompCmd", "CompTwirlScale", Timeline, "reveal.scale"),
    (
        "CCompCompCmd",
        "CompTwirlRotation",
        Timeline,
        "reveal.rotation",
    ),
    (
        "CCompCompCmd",
        "CompTwirlOpacity",
        Timeline,
        "reveal.opacity",
    ),
    (
        "CCompCompCmd",
        "CompTwirlAnchorPoint",
        Timeline,
        "reveal.anchor",
    ),
    (
        "CCompCompCmd",
        "CompToggleEffects",
        Timeline,
        "reveal.effects",
    ),
    (
        "CCompCompCmd",
        "CompToggleMaskShapes",
        Timeline,
        "reveal.masks",
    ),
    (
        "CCompCompCmd",
        "CompToggleUberAnimatingKeyframes",
        Timeline,
        "reveal.animated",
    ),
    ("CCompCompCmd", "CompTwirlAudio", Timeline, "reveal.audio"),
    ("CCompTime", "TimeSetIn", Timeline, "layer.move.in"),
    ("CCompTime", "TimeSetOut", Timeline, "layer.move.out"),
    ("CCompTime", "TimeTrimIn", Timeline, "layer.trim.in"),
    ("CCompTime", "TimeTrimOut", Timeline, "layer.trim.out"),
    ("CSwitchboard", "SplitLayer", Timeline, "layer.split"),
    ("CSwitchboard", "Duplicate", Timeline, "layer.duplicate"),
    ("CSwitchboard", "Compify", Timeline, "layer.precompose"),
    (
        "CSwitchboard",
        "ToggleVideo",
        Timeline,
        "layer.toggle.visible",
    ),
    (
        "CCompCompCmd",
        "CompTimeZoomIn",
        Timeline,
        "timeline.zoom.in",
    ),
    (
        "CCompCompCmd",
        "CompTimeZoomOut",
        Timeline,
        "timeline.zoom.out",
    ),
    (
        "CCompCompCmd",
        "CompTimeZoomToggleFullCustom",
        Timeline,
        "timeline.zoom.fit",
    ),
    // One Rename in After Effects, three places to rename in Lumit.
    ("COutline", "Rename", Timeline, "layer.rename"),
    ("COutline", "Rename", Project, "item.rename"),
    ("COutline", "Rename", Effects, "effect.rename"),
    // Graph editor.
    ("CSwitchboard", "EasyEase", Graph, "graph.ease"),
    ("CSwitchboard", "EasyEaseIn", Graph, "graph.ease.in"),
    ("CSwitchboard", "EasyEaseOut", Graph, "graph.ease.out"),
    // Viewer. Both of After Effects' zooms land on the one Lumit has.
    ("CPanoProjItem", "FitItemView", Viewer, "viewer.zoom.fit"),
    ("CPanoProjItem", "ZoomIn", Viewer, "viewer.zoom.in"),
    ("CPanoProjItem", "ZoomInResize", Viewer, "viewer.zoom.in"),
    ("CPanoProjItem", "ZoomOut", Viewer, "viewer.zoom.out"),
    ("CPanoProjItem", "ZoomOutResize", Viewer, "viewer.zoom.out"),
    ("CSwitchboard", "HighResolution", Viewer, "viewer.res.full"),
    ("CSwitchboard", "MedResolution", Viewer, "viewer.res.half"),
    (
        "CSwitchboard",
        "LowResolution",
        Viewer,
        "viewer.res.quarter",
    ),
    ("CSwitchboard", "ShowRulers", Viewer, "viewer.rulers.toggle"),
    (
        "CSwitchboard",
        "ToggleShowGrid",
        Viewer,
        "viewer.grid.toggle",
    ),
    ("CDirTabPanel", "NewViewer", Viewer, "viewer.new"),
    // Panels.
    ("CSwitchboard", "Find", Panels, "panel.search.focus"),
];

/// Every action `command` stands for.
fn targets(section: &str, command: &str) -> Vec<(KeyContext, ActionId)> {
    let mut out: Vec<(KeyContext, ActionId)> = COMMANDS
        .iter()
        .filter(|row| row.0 == section && row.1 == command)
        .map(|row| (row.2, ActionId::from(row.3)))
        .collect();
    // The numbered markers, ten of each, so they're matched by shape.
    if section == "CCompMarkerCmd" {
        let numbered = [("CompGotoMarker", "goto"), ("CompMarker", "add")];
        for (prefix, verb) in numbered {
            if let Some(n) = command.strip_prefix(prefix) {
                if n.len() == 1 && n.chars().all(|c| c.is_ascii_digit()) {
                    out.push((Global, ActionId(format!("marker.{verb}.{n}"))));
                }
            }
        }
    }
    out
}

/// Lumit's name for an After Effects key, or `None` for a key it has no name for.
fn key_name(name: &str) -> Option<String> {
    let named = match name {
        "Comma" => ",",
        "Backslash" => "\\",
        "SingleQuote" => "'",
        "Plus" | "PadPlus" => "+",
        "PadMinus" => "-",
        "PadMultiply" => "*",
        "PadDecimal" => ".",
        "HOME" => "Home",
        "END" => "End",
        "PageUP" => "PageUp",
        "PageDOWN" => "PageDown",
        // After Effects names these the Mac way: Delete is the Backspace key,
        // and the Delete key is FwdDel.
        "Delete" | "Backspace" => "Backspace",
        "FwdDel" => "Delete",
        // Return is the main key and Enter the numpad's. Lumit calls both Enter.
        "Return" | "Enter" => "Enter",
        "Esc" => "Escape",
        "LeftArrow" => "ArrowLeft",
        "RightArrow" => "ArrowRight",
        "UpArrow" => "ArrowUp",
        "DownArrow" => "ArrowDown",
        "Space" | "Tab" | "Insert" => name,
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_string());
    }
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c
            .is_ascii_graphic()
            .then(|| c.to_ascii_uppercase().to_string());
    }
    let function_key = name
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n));
    function_key.then(|| name.to_string())
}

/// One `Ctrl+Alt+HOME` from the file as a chord, or `None` when Lumit can't
/// spell it.
fn chord(text: &str) -> Option<Chord> {
    let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let key = key_name(parts.pop()?)?;
    let mut mods = Modifiers::default();
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" => mods.primary = true,
            "alt" => mods.alt = true,
            "shift" => mods.shift = true,
            // macControl is the Control key on a Mac, which Lumit has no
            // modifier for.
            _ => return None,
        }
    }
    Some(Chord { mods, key })
}

/// What the file says about one action.
struct Slot {
    context: KeyContext,
    action: ActionId,
    chords: Vec<Chord>,
    /// The file gave it a chord Lumit can't spell.
    unreadable: bool,
}

/// Read an After Effects shortcut file (the `.txt` in its `aeks` folder).
///
/// An action the file gives chords to takes exactly those. One the file leaves
/// empty is left with no chord, since that is what its owner chose. A chord
/// that lands on a key another action in the same context held takes it, and
/// that action is left unbound rather than getting its default back later.
pub fn import_after_effects_shortcuts(text: &str) -> Result<AfterEffectsImport, NotAfterEffects> {
    let mut slots: Vec<Slot> = Vec::new();
    let mut section = String::new();
    // A command whose value runs on to the next line.
    let mut open: Option<(String, String)> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        // Everything that matters on a line is inside double quotes.
        let mut quoted = line.split('"').skip(1).step_by(2);
        if line.starts_with('[') {
            section = quoted.next().unwrap_or_default().to_string();
            continue;
        }
        let (command, mut value) = match open.take() {
            Some(entry) => entry,
            None => match quoted.next() {
                Some(command) => (command.to_string(), String::new()),
                None => continue,
            },
        };
        value.extend(quoted);
        if line.ends_with('\\') {
            open = Some((command, value));
            continue;
        }
        for (context, action) in targets(&section, &command) {
            let found = slots
                .iter()
                .position(|s| s.context == context && s.action == action);
            let index = found.unwrap_or_else(|| {
                slots.push(Slot {
                    context,
                    action,
                    chords: Vec::new(),
                    unreadable: false,
                });
                slots.len() - 1
            });
            let Some(slot) = slots.get_mut(index) else {
                continue;
            };
            let texts = value
                .split(')')
                .filter_map(|c| c.trim().strip_prefix('('))
                .filter(|c| !c.is_empty());
            for text in texts {
                match chord(text) {
                    Some(c) if !slot.chords.contains(&c) => slot.chords.push(c),
                    Some(_) => {}
                    None => slot.unreadable = true,
                }
            }
        }
    }
    if slots.is_empty() {
        return Err(NotAfterEffects);
    }

    let preset = after_effects_preset();
    let mut keymap = preset.clone();
    let mut actions = 0;
    for slot in slots {
        if slot.chords.is_empty() {
            // Only chords Lumit can't spell: the preset's chord is the better guess.
            if !slot.unreadable {
                keymap.unbind_action(slot.context, &slot.action);
                actions += 1;
            }
            continue;
        }
        keymap
            .bindings
            .retain(|b| !(b.context == slot.context && b.action == slot.action));
        for chord in slot.chords {
            keymap.bind(slot.context, chord, slot.action.clone());
        }
        actions += 1;
    }
    // An action that lost its only key to another is unbound on purpose, or
    // the next start would hand it its default back on top of the new owner.
    for b in &preset.bindings {
        if keymap.binding_for(b.context, &b.action).is_none() {
            keymap.unbind_action(b.context, &b.action);
        }
    }
    Ok(AfterEffectsImport { keymap, actions })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    /// A cut-down file in After Effects' own layout: the comment header, a
    /// section per panel, two chords on a line, and a value that runs on.
    const SAMPLE: &str = r#"# Text File Version 1.1
# After Effects Shortcut Preferences (modify at your own risk)
#  #
#  A

["** header **"]
	"major_version" = "109"

["CCompCmd"]
	"AddMarker" = "(PadMultiply)(macControl+8)"
	"Clear" = "(Delete)(FwdDel)"
	"FlipHorizontal" = "()"

["CCompMarkerCmd"]
	"CompGotoMarker3" = "(Alt+3)"
	"CompMarker3" = "(Shift+3)"

["CCompTime"]
	"TimeRewind" = "(Ctrl+Alt+LeftArrow)(HOME)"
	"TimeStepForward" = "(Ctrl+RightArrow)(PageDOWN)"
	"TimeSetIn" = "(Umlaut_a)"

["CEggAppTool"]
	"ToolHand" = "()"
	"ToolPaint" = "(Shift+C)"

["CPanoProjItem"]
	"ZoomIn" = "(.)(Ctrl+Alt+=)"
	"ZoomInResize" = "(Ctrl+=)(Alt+.)"
	"ZoomOut" = "(Comma)"

["CSwitchboard"]
	"DeselectAll" = "(F2)(Ctrl+Shift+A)"
	"SplitLayer" = "(Ctrl+Shift+X)"
	"NewComp" = "(Ctrl"\
		"+Alt+Shi"\
		"ft+N)"

["TextLayerUI"]
	"TextCommit" = "(Enter)(Ctrl+Return)"
"#;

    #[test]
    fn a_rebound_command_moves_its_action() {
        let import = import_after_effects_shortcuts(SAMPLE).unwrap();
        let km = import.keymap;
        assert_eq!(
            km.lookup(KeyContext::Timeline, &chord("Mod+Shift+X")),
            Some(&"layer.split".into())
        );
        assert_eq!(
            km.lookup(KeyContext::Timeline, &chord("Mod+Shift+D")),
            None,
            "the chord the preset gave it stopped working"
        );
        // The numbered markers are matched by shape.
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("Alt+3")),
            Some(&"marker.goto.3".into())
        );
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("Shift+3")),
            Some(&"marker.add.3".into())
        );
    }

    #[test]
    fn every_chord_on_a_line_is_kept_and_its_keys_are_renamed() {
        let km = import_after_effects_shortcuts(SAMPLE).unwrap().keymap;
        for text in ["Mod+Alt+ArrowLeft", "Home"] {
            assert_eq!(
                km.lookup(KeyContext::Global, &chord(text)),
                Some(&"playback.comp.start".into()),
                "{text}"
            );
        }
        for text in ["Mod+ArrowRight", "PageDown"] {
            assert_eq!(
                km.lookup(KeyContext::Global, &chord(text)),
                Some(&"playback.frame.next".into()),
                "{text}"
            );
        }
        // After Effects' Delete is Backspace, and its FwdDel is Delete.
        for text in ["Backspace", "Delete"] {
            assert_eq!(
                km.lookup(KeyContext::Global, &chord(text)),
                Some(&"edit.delete.selection".into()),
                "{text}"
            );
        }
        assert_eq!(
            km.lookup(KeyContext::Viewer, &chord(",")),
            Some(&"viewer.zoom.out".into())
        );
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("F2")),
            Some(&"edit.deselect.all".into())
        );
    }

    #[test]
    fn two_commands_for_one_action_pool_their_chords() {
        let km = import_after_effects_shortcuts(SAMPLE).unwrap().keymap;
        for text in [".", "Mod+Alt+=", "Mod+=", "Alt+."] {
            assert_eq!(
                km.lookup(KeyContext::Viewer, &chord(text)),
                Some(&"viewer.zoom.in".into()),
                "{text}"
            );
        }
    }

    #[test]
    fn a_value_split_across_lines_reads_as_one() {
        let km = import_after_effects_shortcuts(SAMPLE).unwrap().keymap;
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("Mod+Alt+Shift+N")),
            Some(&"comp.new".into())
        );
        assert_eq!(km.lookup(KeyContext::Global, &chord("Mod+N")), None);
    }

    #[test]
    fn a_command_left_empty_stays_unbound() {
        let km = import_after_effects_shortcuts(SAMPLE).unwrap().keymap;
        let hand = ActionId::from("tool.hand");
        assert_eq!(km.binding_for(KeyContext::Tools, &hand), None);
        assert!(km.unbound.contains(&(KeyContext::Tools, hand)));
        // And it stays that way through the stored file.
        let restored = crate::with_new_defaults(km);
        assert_eq!(restored.lookup(KeyContext::Tools, &chord("H")), None);
    }

    #[test]
    fn a_chord_lumit_cannot_spell_is_left_out() {
        let km = import_after_effects_shortcuts(SAMPLE).unwrap().keymap;
        // The Mac-only Control chord goes and the numpad one stays.
        assert_eq!(
            km.binding_for(KeyContext::Global, &"marker.add".into()),
            Some(&chord("*"))
        );
        // Nothing readable at all, so the preset's chord stands.
        assert_eq!(
            km.lookup(KeyContext::Timeline, &chord("[")),
            Some(&"layer.move.in".into())
        );
    }

    #[test]
    fn what_the_file_does_not_name_keeps_the_preset() {
        let import = import_after_effects_shortcuts(SAMPLE).unwrap();
        let km = &import.keymap;
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("J")),
            Some(&"keyframe.prev".into())
        );
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("Mod+Shift+P")),
            Some(&"palette.open".into())
        );
        // TimeSetIn had nothing readable, so it isn't counted.
        assert_eq!(import.actions, 13);
        // A section Lumit has no commands in is skipped whole.
        assert_eq!(km.lookup(KeyContext::Global, &chord("Mod+Enter")), None);
    }

    #[test]
    fn taking_another_actions_key_leaves_no_clash_now_or_after_a_restart() {
        // The file puts the brush on Shift+C, which the preset gave the razor.
        let km = import_after_effects_shortcuts(SAMPLE).unwrap().keymap;
        assert_eq!(
            km.lookup(KeyContext::Tools, &chord("Shift+C")),
            Some(&"tool.paint".into())
        );
        assert_eq!(
            km.binding_for(KeyContext::Tools, &"tool.razor".into()),
            None
        );
        assert!(km.conflicts().is_empty());
        let restored = crate::with_new_defaults(km);
        assert!(
            restored.conflicts().is_empty(),
            "the razor must not get C back on top of the camera"
        );
    }

    #[test]
    fn text_that_is_not_a_shortcut_file_is_refused() {
        for junk in [
            "",
            "{\"bindings\": []}",
            "[\"CCompTime\"]\n\t\"Nope\" = \"(K)\"",
        ] {
            assert_eq!(import_after_effects_shortcuts(junk), Err(NotAfterEffects));
        }
    }
}
