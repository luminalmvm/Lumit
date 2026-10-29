//! The shipped key tables: Lumit's default keymap and the After Effects and
//! Vegas Pro presets laid over it.

use crate::{
    ActionId, Binding, Chord, HandleModifier, KeyContext, Keymap, WheelAction, WheelKeys,
    WheelModifier,
};

/// Parse a `(context, "chord", "action")` row into a [`Binding`], or `None` if
/// the literal chord is malformed (the default tables below `flatten()` those
/// away). For the built-in tables, not user input.
fn row(context: KeyContext, chord: &str, action: &str) -> Option<Binding> {
    Some(Binding {
        context,
        chord: chord.parse().ok()?,
        action: action.into(),
    })
}

/// Lumit's default keymap — the docs/07 §15 table. Global transport and
/// navigation, tool selection, timeline reveals/edits, graph, viewer and panel
/// shortcuts. `Ctrl` here is the platform-neutral primary (`Mod`); the UI maps
/// it to Cmd on macOS. Ships conflict-free (proven in tests).
#[must_use]
pub fn default_keymap() -> Keymap {
    use KeyContext::{Cut, Effects, Global, Graph, Panels, Project, Timeline, Tools, Viewer};
    let rows = [
        // --- Global: transport, navigation, app-wide commands ---
        row(Global, "Space", "playback.toggle"),
        row(Global, "J", "playback.shuttle.reverse"),
        row(Global, "K", "playback.shuttle.pause"),
        row(Global, "L", "playback.shuttle.forward"),
        // Stepping a frame is `Mod`+arrow, not the bare arrow. The
        // bare arrows used to do it, which meant the app-wide transport owned
        // the two keys every list, field and canvas wants for moving *within*
        // itself — so nothing else could ever be given them. `Mod` is the
        // platform's primary modifier, so this is Ctrl+arrow on Windows and
        // Linux and Cmd+arrow on macOS, like every other `Mod` chord here.
        row(Global, "Mod+ArrowRight", "playback.frame.next"),
        row(Global, "Mod+ArrowLeft", "playback.frame.prev"),
        row(Global, "PageDown", "playback.frame.next"),
        row(Global, "PageUp", "playback.frame.prev"),
        row(Global, "Shift+PageDown", "playback.frame.next10"),
        row(Global, "Shift+PageUp", "playback.frame.prev10"),
        row(Global, "Home", "playback.comp.start"),
        row(Global, "End", "playback.comp.end"),
        row(Global, "Shift+Home", "playback.workarea.start"),
        row(Global, "Shift+End", "playback.workarea.end"),
        row(Global, "I", "playback.layer.in"),
        row(Global, "O", "playback.layer.out"),
        row(Global, ",", "keyframe.prev"),
        row(Global, ".", "keyframe.next"),
        row(Global, "Mod+,", "edit.point.prev"),
        row(Global, "Mod+.", "edit.point.next"),
        // A keyframe at the playhead on whichever property rows are picked, or
        // the one already there taken away. After Effects has no key for this,
        // only the per-property five below, so the chord is Lumit's own.
        row(Global, "Alt+K", "keyframe.toggle"),
        // After Effects' own five: the reveal letter with Alt and Shift keys
        // that property on the selected layers, and shows its row.
        row(Global, "Alt+Shift+P", "keyframe.toggle.position"),
        row(Global, "Alt+Shift+S", "keyframe.toggle.scale"),
        row(Global, "Alt+Shift+R", "keyframe.toggle.rotation"),
        row(Global, "Alt+Shift+T", "keyframe.toggle.opacity"),
        row(Global, "Alt+Shift+A", "keyframe.toggle.anchor"),
        // The rest of the Animation menu's keyframe rows, on After Effects'
        // chords. They act on the keys under the playhead.
        row(Global, "Mod+Alt+H", "keyframe.hold.toggle"),
        row(Global, "Mod+Alt+K", "keyframe.interpolation"),
        row(Global, "Mod+Shift+K", "keyframe.speed"),
        row(Global, "Alt+Shift+=", "expression.add"),
        row(Global, "B", "workarea.set.start"),
        row(Global, "N", "workarea.set.end"),
        row(Global, "*", "marker.add"),
        // `Shift+M` is the second way in. Premiere and Vegas both drop a
        // marker on `M`, which is the habit the owner arrives with — but `M`
        // reveals Masks in the Timeline and that is After Effects' oldest
        // reflex, so the letter stays where it is and the marker takes Shift.
        // Two chords for one action is not a clash; `*` still works.
        row(Global, "Shift+M", "marker.add"),
        // Delete-removes-the-selection was missing from the §15 table
        // entirely (TF-6, first outside tester): keyframes when any are
        // selected, else the selected layer. Backspace is its usual sibling.
        row(Global, "Delete", "edit.delete.selection"),
        row(Global, "Backspace", "edit.delete.selection"),
        // Cut, copy and paste had menu rows and no chords at all: the
        // three keys everyone's fingers reach for first did nothing to a
        // selected layer. Global, because what they act on is whatever is
        // selected — keyframes in the Timeline, else an effect, else the layer.
        row(Global, "Mod+X", "edit.cut"),
        row(Global, "Mod+C", "edit.copy"),
        row(Global, "Mod+V", "edit.paste"),
        row(Global, "Mod+Shift+P", "palette.open"),
        // The FX console: Video Copilot's own chord, and the one the
        // owner asked for.
        row(Global, "Mod+Space", "console.open"),
        // The flowchart: After Effects' own key for its mini-flowchart.
        row(Global, "Tab", "comp.flowchart"),
        row(Global, "Mod+M", "export.queue.add"),
        row(Global, "Mod+K", "comp.settings"),
        row(Global, "Mod+Shift+X", "comp.trim.workarea"),
        row(Global, "Mod+Alt+0", "export.queue.open"),
        row(Global, "F1", "help.open"),
        row(Global, "Mod+Z", "edit.undo"),
        row(Global, "Mod+Shift+Z", "edit.redo"),
        row(Global, "Mod+S", "file.save"),
        // The rest of the menu bar's own commands. After Effects'
        // chords where it has one and Lumit has not already spent the key:
        // Mod+N makes a *composition* there and Mod+Alt+N a project, which is
        // the pair anyone arriving from AE has in their fingers. Settings is
        // AE's Preferences chord for the same reason.
        row(Global, "Mod+Alt+N", "file.new"),
        row(Global, "Mod+O", "file.open"),
        row(Global, "Mod+Shift+S", "file.save.as"),
        row(Global, "Mod+Alt+S", "file.save.version"),
        row(Global, "Mod+I", "file.import"),
        row(Global, "Mod+Alt+M", "file.export"),
        row(Global, "Mod+N", "comp.new"),
        row(Global, "Mod+A", "edit.select.all"),
        row(Global, "Mod+Shift+A", "edit.deselect.all"),
        row(Global, "Mod+Alt+;", "app.settings"),
        // After Effects' own chord for the same window, and one nothing else
        // wants: Mod+K alone is Composition settings, a comp away from this.
        row(Global, "Mod+Alt+Shift+K", "project.settings"),
        row(Global, "`", "panel.maximise"),
        row(Global, "Shift+F3", "graph.toggle"),
        // Retime is app-wide, not Timeline-scoped: the shell runs it whatever
        // panel is fronted, and Composition ▸ Enable Retime carries the same
        // command. Mod+Alt+T is After Effects' own Time Remap chord, and it is
        // also one Windows cannot steal — the briefly-shipped Alt+Shift+T was
        // a misremembering, and it collided with the Windows input-language
        // switch anyway. One chord, like every other action; anyone who wants
        // a second can bind it.
        row(Global, "Mod+Alt+T", "layer.retime.enable"),
        // Layer ▸ New. After Effects' own chords, so the rows read the way
        // anyone arriving from there expects. Spot and Area lights have no
        // chord there and ship without one here.
        row(Global, "Mod+Y", "layer.new.solid"),
        row(Global, "Mod+Alt+Shift+T", "layer.new.text"),
        row(Global, "Mod+Alt+Shift+C", "layer.new.camera"),
        row(Global, "Mod+Alt+Shift+L", "layer.new.light.point"),
        row(Global, "Mod+Alt+Y", "layer.new.adjustment"),
        row(Global, "Mod+Alt+Shift+Y", "layer.new.null"),
        // An empty row for clips, picture or sound. After Effects has neither,
        // so they take the family's chord with their own initial.
        row(Global, "Mod+Alt+Shift+S", "layer.new.sequence"),
        row(Global, "Mod+Alt+Shift+A", "layer.new.audio"),
        // What else After Effects does to the selected layers from the
        // keyboard, on its own chords. App-wide like the ones above, since a
        // layer picked in the Viewer is as selected as one picked in the
        // Timeline.
        row(Global, "Mod+Shift+Y", "layer.settings"),
        row(Global, "Shift+=", "layer.blend.next"),
        row(Global, "Shift+-", "layer.blend.prev"),
        row(Global, "Mod+L", "layer.lock.toggle"),
        row(Global, "Mod+Shift+L", "layer.unlock.all"),
        row(Global, "Mod+ArrowDown", "layer.select.next"),
        row(Global, "Mod+ArrowUp", "layer.select.prev"),
        row(Global, "Mod+]", "layer.order.up"),
        row(Global, "Mod+[", "layer.order.down"),
        row(Global, "Mod+Shift+]", "layer.order.top"),
        row(Global, "Mod+Shift+[", "layer.order.bottom"),
        row(Global, "Alt+PageUp", "layer.shift.earlier"),
        row(Global, "Alt+PageDown", "layer.shift.later"),
        row(Global, "Alt+Shift+PageUp", "layer.shift.earlier10"),
        row(Global, "Alt+Shift+PageDown", "layer.shift.later10"),
        row(Global, "Mod+Home", "layer.centre"),
        row(Global, "Mod+Alt+Home", "layer.centre.anchor"),
        // The mask whose row is picked in the Timeline.
        row(Global, "Mod+Shift+F", "mask.feather"),
        row(Global, "Mod+Shift+I", "mask.invert"),
        // The 3D views on After Effects' own keys, which work from any panel
        // there too.
        row(Global, "F10", "viewer.3d.slot.1"),
        row(Global, "F11", "viewer.3d.slot.2"),
        row(Global, "F12", "viewer.3d.slot.3"),
        row(Global, "Shift+F10", "viewer.3d.assign.1"),
        row(Global, "Shift+F11", "viewer.3d.assign.2"),
        row(Global, "Shift+F12", "viewer.3d.assign.3"),
        row(Global, "Mod+Alt+Shift+\\", "viewer.3d.look.selected"),
        row(Global, "Mod+Shift+\\", "viewer.3d.look.all"),
        // --- Tools ---
        row(Tools, "V", "tool.select"),
        row(Tools, "H", "tool.hand"),
        row(Tools, "Z", "tool.zoom"),
        row(Tools, "Y", "tool.anchor"),
        row(Tools, "C", "tool.razor"),
        row(Tools, "Q", "tool.shape"),
        row(Tools, "G", "tool.pen"),
        // The rest of the toolbar. After Effects' own chords wherever
        // Lumit has not already spent the key: W rotates and Alt+W is the roto
        // brush there too. Its camera cycle is `C`, which docs/07 §15 gave to
        // the razor long before there was a camera tool, so the camera group
        // takes `Shift+C` rather than either key changing meaning.
        row(Tools, "W", "tool.rotate"),
        row(Tools, "Mod+T", "tool.type"),
        row(Tools, "Mod+B", "tool.paint"),
        row(Tools, "Alt+W", "tool.roto"),
        row(Tools, "Mod+P", "tool.puppet"),
        row(Tools, "Shift+C", "tool.camera"),
        // With a camera tool in hand over the Viewer, After Effects' F and
        // Ctrl+Shift+F look at the selected layers and at all of them.
        row(Tools, "F", "viewer.3d.look.selected"),
        row(Tools, "Mod+Shift+F", "viewer.3d.look.all"),
        // And in a 3D viewport with no tool, F too, with Shift+F to frame
        // everything.
        row(Tools, "Shift+F", "viewer.3d.frame.all"),
        // --- Timeline: reveals and edits ---
        row(Timeline, "P", "reveal.position"),
        row(Timeline, "S", "reveal.scale"),
        row(Timeline, "R", "reveal.rotation"),
        row(Timeline, "T", "reveal.opacity"),
        row(Timeline, "A", "reveal.anchor"),
        row(Timeline, "E", "reveal.effects"),
        row(Timeline, "M", "reveal.masks"),
        row(Timeline, "U", "reveal.animated"),
        // `L` opens a layer's Audio group; a second `L` adds its waveform lane,
        // a third shuts the layer again. It shadows the J/K/L shuttle
        // inside the Timeline and nowhere else — the panel where you reach for
        // a layer's sound is the panel where you are least often shuttling.
        // `Shift+L` keeps the After Effects habit pointed at the same cycle.
        row(Timeline, "L", "reveal.audio"),
        row(Timeline, "Shift+L", "reveal.audio"),
        row(Timeline, "[", "layer.move.in"),
        row(Timeline, "]", "layer.move.out"),
        row(Timeline, "Alt+[", "layer.trim.in"),
        row(Timeline, "Alt+]", "layer.trim.out"),
        row(Timeline, "Mod+Shift+D", "layer.split"),
        row(Timeline, "Mod+D", "layer.duplicate"),
        row(Timeline, "Mod+Shift+C", "layer.precompose"),
        // Ctrl+G folds the selected run into a group and Ctrl+Shift+G takes the
        // fold away — the chord every editor with groups in it uses,
        // and free in Lumit: `G` alone is the pen in the Tools context, which
        // is a different context and an unmodified key. It sits beside
        // Pre-compose deliberately: the two are the light fold and the heavy
        // one, and they belong under the same finger.
        row(Timeline, "Mod+G", "layer.group"),
        row(Timeline, "Mod+Shift+G", "layer.ungroup"),
        row(Timeline, "=", "timeline.zoom.in"),
        row(Timeline, "-", "timeline.zoom.out"),
        row(Timeline, "\\", "timeline.zoom.fit"),
        // Layers mode's own F, beside the graph's: the keys of the selected
        // layers, or of every layer, across the lanes.
        row(Timeline, "F", "timeline.fit.keys"),
        row(Timeline, "Enter", "layer.rename"),
        // Enter renames the selected thing wherever one is selected:
        // the item in the Project panel, the effect in Effect controls — the
        // same key the Timeline has always used for its layers.
        row(Project, "Enter", "item.rename"),
        row(Project, "Mod+D", "item.duplicate"),
        row(Project, "ArrowUp", "item.select.prev"),
        row(Project, "ArrowDown", "item.select.next"),
        row(Project, "Shift+ArrowUp", "item.select.prev.extend"),
        row(Project, "Shift+ArrowDown", "item.select.next.extend"),
        row(Project, "ArrowLeft", "item.folder.close"),
        row(Project, "ArrowRight", "item.folder.open"),
        // After Effects' own three for Interpret Footage.
        row(Project, "Mod+Alt+G", "footage.interpret"),
        row(Project, "Mod+Alt+C", "footage.interpretation.remember"),
        row(Project, "Mod+Alt+V", "footage.interpretation.apply"),
        row(Effects, "Enter", "effect.rename"),
        row(Timeline, "X", "layer.toggle.visible"),
        // --- Graph editor ---
        row(Graph, "F9", "graph.ease"),
        row(Graph, "Shift+F9", "graph.ease.in"),
        row(Graph, "Mod+Shift+F9", "graph.ease.out"),
        row(Graph, "F", "graph.fit"),
        // --- Viewer ---
        row(Viewer, "Shift+/", "viewer.zoom.fit"),
        row(Viewer, "Mod+=", "viewer.zoom.in"),
        row(Viewer, "Mod+-", "viewer.zoom.out"),
        row(Viewer, "Mod+J", "viewer.res.full"),
        row(Viewer, "Mod+Shift+J", "viewer.res.half"),
        row(Viewer, "Mod+Alt+J", "viewer.res.quarter"),
        row(Viewer, "Mod+R", "viewer.rulers.toggle"),
        row(Viewer, "Mod+'", "viewer.grid.toggle"),
        // Several views (docs/impl/multi-viewer.md §5). The layout numbers
        // follow After Effects' own; the rest are chords nothing else claims.
        row(Viewer, "Mod+Alt+V", "viewer.new"),
        row(Viewer, "Mod+Shift+.", "viewer.view.next"),
        row(Viewer, "Mod+Shift+,", "viewer.view.prev"),
        row(Viewer, "Mod+Alt+L", "viewer.lock.toggle"),
        row(Viewer, "Alt+1", "viewer.layout.one"),
        row(Viewer, "Alt+2", "viewer.layout.two"),
        row(Viewer, "Alt+4", "viewer.layout.four"),
        row(Viewer, "Alt+3", "viewer.layout.orientation"),
        row(Viewer, "Mod+Alt+F", "viewer.cinema"),
        row(Viewer, "Mod+Alt+C", "viewer.compare"),
        row(Viewer, "Mod+Alt+P", "viewer.preview.always"),
        row(Viewer, "Mod+Shift+H", "viewer.controls.toggle"),
        row(Viewer, "Mod+Shift+'", "viewer.snap.grid.toggle"),
        // Escape goes back to the last 3D view, as in After Effects, and
        // only when nothing else on screen wanted it.
        row(Viewer, "Escape", "viewer.3d.last"),
        // --- Panels ---
        row(Panels, "Mod+F6", "panel.focus.next"),
        row(Panels, "Mod+Shift+F6", "panel.focus.prev"),
        row(Panels, "Mod+F", "panel.search.focus"),
        // Putting a panel up and taking it down, on the numbers After Effects
        // gives its own.
        row(Panels, "Mod+0", "panel.toggle.project"),
        row(Panels, "F3", "panel.toggle.effects"),
        row(Panels, "Mod+5", "panel.toggle.presets"),
        row(Panels, "Mod+4", "panel.toggle.audio"),
        row(Panels, "Mod+6", "panel.toggle.text"),
        row(Panels, "Mod+7", "panel.toggle.paragraph"),
        // --- Cut timeline ---
        // An editor's own keys, the ones Premiere uses. I, O and the two
        // punctuation keys are spoken for app-wide, by the layer's in and out
        // points and the keyframe steps, so they are bound here and shadow
        // those inside the Cut timeline and nowhere else, the way `L` does
        // in the Timeline.
        row(Cut, "I", "cut.mark.in"),
        row(Cut, "O", "cut.mark.out"),
        row(Cut, ",", "cut.insert"),
        row(Cut, ".", "cut.overwrite"),
        row(Cut, "Shift+Delete", "cut.delete.ripple"),
        // The tools, the trims to the playhead and the nudges, on the
        // letters editors already have under their fingers. Each shadows an
        // app-wide key inside the Cut timeline alone.
        row(Cut, "V", "cut.tool.select"),
        row(Cut, "C", "cut.tool.razor"),
        row(Cut, "B", "cut.tool.ripple"),
        row(Cut, "N", "cut.tool.roll"),
        row(Cut, "Y", "cut.tool.slip"),
        row(Cut, "U", "cut.tool.slide"),
        row(Cut, "Q", "cut.trim.start.ripple"),
        row(Cut, "W", "cut.trim.end.ripple"),
        row(Cut, "Alt+Q", "cut.trim.start"),
        row(Cut, "Alt+W", "cut.trim.end"),
        row(Cut, "Shift+Q", "cut.roll.prev"),
        row(Cut, "Shift+W", "cut.roll.next"),
        row(Cut, "Mod+K", "cut.add.edit"),
        row(Cut, "Mod+Shift+K", "cut.add.edit.all"),
        row(Cut, "Alt+ArrowLeft", "cut.nudge.left"),
        row(Cut, "Alt+ArrowRight", "cut.nudge.right"),
        row(Cut, "Alt+Shift+ArrowLeft", "cut.nudge.left.many"),
        row(Cut, "Alt+Shift+ArrowRight", "cut.nudge.right.many"),
        row(Cut, "Alt+ArrowUp", "cut.nudge.up"),
        row(Cut, "Alt+ArrowDown", "cut.nudge.down"),
        row(Cut, "D", "cut.select.at.playhead"),
        row(Cut, "S", "cut.snap.toggle"),
        row(Cut, "Shift+L", "cut.linked.toggle"),
        // The three switches Vegas has. Its own keys for the first two are
        // taken app-wide here, so they sit on those in the Vegas preset.
        row(Cut, "Shift+B", "cut.ripple.toggle"),
        row(Cut, "Shift+X", "cut.crossfade.toggle"),
        row(Cut, "Alt+F8", "cut.quantise.toggle"),
        row(Cut, ";", "cut.lift"),
        row(Cut, "'", "cut.extract"),
        row(Cut, "F", "cut.match.frame"),
        row(Cut, "Mod+D", "cut.transition.default"),
        row(Cut, "=", "timeline.zoom.in"),
        row(Cut, "-", "timeline.zoom.out"),
        row(Cut, "\\", "timeline.zoom.fit"),
        row(Cut, "ArrowUp", "edit.point.prev"),
        row(Cut, "ArrowDown", "edit.point.next"),
    ];
    let mut bindings: Vec<Binding> = rows.into_iter().flatten().collect();
    // Alt+Shift+1…9 switch workspace.
    for d in 1..=9u8 {
        if let Some(b) = row(
            Global,
            &format!("Alt+Shift+{d}"),
            &format!("workspace.switch.{d}"),
        ) {
            bindings.push(b);
        }
    }
    // Numbered markers: `Shift+N` drops marker *N* at the playhead and
    // the bare digit jumps back to it — After Effects' own pairing. The pairing
    // is the point: the key that sets a cue is the key that returns to it, with
    // the modifier as the only difference.
    for d in 0..=9u8 {
        if let Some(b) = row(Global, &format!("Shift+{d}"), &format!("marker.add.{d}")) {
            bindings.push(b);
        }
        if let Some(b) = row(Global, &format!("{d}"), &format!("marker.goto.{d}")) {
            bindings.push(b);
        }
    }
    Keymap {
        bindings,
        unbound: Vec::new(),
        wheel: WheelKeys::default(),
        break_handles: HandleModifier::default(),
    }
}

/// A **stored** keymap laid over the shipped defaults: the file's chord
/// wins for every action it names, and an action it never heard of — one added
/// to Lumit after that file was written — keeps its default binding.
///
/// **The bug this exists for.** A stored keymap used to replace the map whole,
/// so the first release to add an action left it unbound for everybody who had
/// ever saved a keymap: their app simply did not have the key. It was found
/// when `Ctrl+C` did nothing in a build whose tests all passed — the tests
/// start from the defaults, and only a real session has a stored file.
///
/// An action the user deliberately unbound stays unbound: that is what
/// [`Keymap::unbound`] records, and why it had to exist.
#[must_use]
pub fn with_new_defaults(stored: Keymap) -> Keymap {
    let mut out = stored;
    for binding in default_keymap().bindings {
        let known = out
            .bindings
            .iter()
            .any(|b| b.context == binding.context && b.action == binding.action)
            || out
                .unbound
                .iter()
                .any(|(c, a)| *c == binding.context && *a == binding.action);
        if !known {
            out.bindings.push(binding);
        }
    }
    out
}

/// The "After Effects" muscle-memory preset (docs/07 §15).
///
/// Starts from the shipped default — which already follows After Effects
/// wherever the two agree: the V/H/Z/Y/Q/G/W tool letters, the P/S/R/T/A/E/M/U
/// reveals, `[` / `]` and Alt+`[` / Alt+`]` for layer in/out and trims,
/// Home/End and the Page keys, Space transport, I/O to the layer's in/out
/// point, B/N for the work area, Mod+D duplicate, Mod+Shift+D split,
/// Mod+Shift+C precompose, the F9 eases, and Mod+Alt+T for Retime — and
/// re-points the places where the default deviates from AE:
///
/// - **J / K** go to the previous / next keyframe, app-wide (AE's habit), and
///   the Vegas-style J/K/L shuttle is dropped — AE has no shuttle, and freeing
///   those letters is the preset's whole point. In the Timeline, `L` keeps
///   revealing a layer's Audio, which is also AE's `L`.
/// - **C** selects the camera tool and **Shift+C** the razor: AE's `C` cycles
///   its camera tools, and the razor — which AE does not have as a tool —
///   takes the modified key the two swapped.
///
/// Every binding re-points an action id the app already ships; the preset
/// adds no new actions and no user-facing strings of its own.
#[must_use]
pub fn after_effects_preset() -> Keymap {
    let mut km = default_keymap();
    // AE has no J/K/L shuttle; the letters go back to their AE meanings. Unbound
    // by action, so the stored file remembers it and a restart doesn't put the
    // shuttle back.
    for direction in ["reverse", "pause", "forward"] {
        let action = ActionId(format!("playback.shuttle.{direction}"));
        km.unbind_action(KeyContext::Global, &action);
    }
    // `bind`, not `rebind_action`, for the keyframe pair: `,` / `.` stay as a
    // second way in, exactly as the default keeps `*` beside `Shift+M`.
    let extra = [
        (KeyContext::Global, "J", "keyframe.prev"),
        (KeyContext::Global, "K", "keyframe.next"),
    ];
    for (context, chord, action) in extra {
        if let Ok(chord) = chord.parse::<Chord>() {
            km.bind(context, chord, action.into());
        }
    }
    // The camera/razor swap. Camera first: rebinding it to `C` evicts the
    // razor from the letter, so the razor can then take `Shift+C` cleanly.
    let swaps = [
        (KeyContext::Tools, "C", "tool.camera"),
        (KeyContext::Tools, "Shift+C", "tool.razor"),
    ];
    for (context, chord, action) in swaps {
        if let Ok(chord) = chord.parse::<Chord>() {
            km.rebind_action(context, &action.into(), chord);
        }
    }
    // AE zooms time on Alt and the graph's values on Ctrl.
    km.wheel.set(WheelAction::ZoomTime, WheelModifier::Alt);
    km.wheel.set(WheelAction::ZoomValues, WheelModifier::Ctrl);
    km
}

/// The "Vegas Pro" muscle-memory preset.
///
/// Starts from the shipped default, which already agrees with Vegas on Space,
/// the J/K/L shuttle, Mod+X/C/V, Delete, undo and redo, the bare digits that
/// go to a marker, I and O for the marked span in the Cut timeline and
/// Alt+`[` / Alt+`]` for a layer's trims, and re-points the rest. The Cut
/// timeline first, since that is where a Vegas editor works:
///
/// - **S** cuts at the playhead, on the picked clips' tracks or on every
///   track with nothing picked, which is what Vegas's split does. Snapping
///   moves to **F8**, where Vegas has it.
/// - **Alt+`[`** and **Alt+`]`** trim a clip's start and end to the playhead.
///   The ripple trims go beside them with Shift, which takes them off Q and
///   W: Q is Vegas's loop key and **W** goes to the start, and neither
///   should cut.
/// - **Up** and **Down** zoom, **Alt+Left** and **Alt+Right** step a frame,
///   and with Mod as well they go to the edit point either side.
/// - The numeric keypad nudges the picked clips: **1** and **3** a frame
///   earlier and later, **8** and **2** up and down a track.
/// - **Mod+D** is the select tool, Vegas's normal edit tool, and the default
///   transition takes Shift with it. **Mod+Shift+U** turns linked selection
///   on and off, the chord that ignores event grouping there.
/// - **Mod+L** turns auto ripple on and off and **Mod+Shift+X** automatic
///   crossfades. Quantise to frames is on Alt+F8 already.
/// - **M** adds a marker, anywhere.
/// - **Mod+Q**, **Mod+Shift+Q** and **Mod+Alt+Shift+Q** add an empty Audio,
///   Sequence or Adjustment layer, anywhere. Not on a Mac, where Cmd+Q quits
///   and Cmd+Shift+Q logs out before Lumit hears either.
///
/// In the Timeline, **S** splits the layer, **I** and **O** set the work
/// area as they do in the Cut timeline, Up and Down zoom, and `L` shuttles
/// as it does everywhere else. Scale, Masks and Audio are revealed with
/// Shift held instead.
///
/// Every binding re-points an action id the app already ships; the preset
/// adds no new actions and no user-facing strings of its own.
#[must_use]
pub fn vegas_preset() -> Keymap {
    use KeyContext::{Cut, Global, Timeline};
    let mut km = default_keymap();
    // In order: a row that gives up a key comes before the row that takes it,
    // so nothing is left without one.
    let moves = [
        (Cut, "F8", "cut.snap.toggle"),
        (Cut, "S", "cut.add.edit"),
        (Cut, "Alt+[", "cut.trim.start"),
        (Cut, "Alt+]", "cut.trim.end"),
        (Cut, "Alt+Shift+[", "cut.trim.start.ripple"),
        (Cut, "Alt+Shift+]", "cut.trim.end.ripple"),
        (Cut, "W", "playback.comp.start"),
        // The keypad's digits, spelt the way a keypress names them.
        (Cut, "Numpad 1", "cut.nudge.left"),
        (Cut, "Numpad 3", "cut.nudge.right"),
        (Cut, "Numpad 8", "cut.nudge.up"),
        (Cut, "Numpad 2", "cut.nudge.down"),
        (Cut, "Alt+ArrowLeft", "playback.frame.prev"),
        (Cut, "Alt+ArrowRight", "playback.frame.next"),
        (Cut, "Mod+Alt+ArrowLeft", "edit.point.prev"),
        (Cut, "Mod+Alt+ArrowRight", "edit.point.next"),
        (Cut, "ArrowUp", "timeline.zoom.in"),
        (Cut, "ArrowDown", "timeline.zoom.out"),
        (Cut, "Mod+Shift+D", "cut.transition.default"),
        (Cut, "Mod+D", "cut.tool.select"),
        (Cut, "Mod+Shift+U", "cut.linked.toggle"),
        (Cut, "Mod+L", "cut.ripple.toggle"),
        (Cut, "Mod+Shift+X", "cut.crossfade.toggle"),
        (Global, "M", "marker.add"),
        (Timeline, "Shift+M", "reveal.masks"),
        (Timeline, "Shift+S", "reveal.scale"),
        (Timeline, "S", "layer.split"),
        (Timeline, "Shift+L", "reveal.audio"),
        (Timeline, "I", "workarea.set.start"),
        (Timeline, "O", "workarea.set.end"),
        (Timeline, "ArrowUp", "timeline.zoom.in"),
        (Timeline, "ArrowDown", "timeline.zoom.out"),
        (Global, "Mod+Q", "layer.new.audio"),
        (Global, "Mod+Shift+Q", "layer.new.sequence"),
        (Global, "Mod+Alt+Shift+Q", "layer.new.adjustment"),
    ];
    for (context, chord, action) in moves {
        if cfg!(target_os = "macos") && chord.ends_with("+Q") {
            continue;
        }
        if let Ok(chord) = chord.parse::<Chord>() {
            km.rebind_action(context, &action.into(), chord);
        }
    }
    km
}
