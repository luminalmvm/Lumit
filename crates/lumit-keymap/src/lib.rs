//! The keymap core: chords, contexts, and conflict detection (docs/07-UI-SPEC
//! §15). Pure logic — no windowing, no egui — so the whole remappable-keymap
//! promise (search, conflict detection, per-context display, a shareable file,
//! an After Effects preset) rests on rules an ordinary test can prove. The UI
//! layer maps a real key event to a [`Chord`] + active [`KeyContext`] and asks
//! [`Keymap::lookup`] what to do; Settings → Keymap edits the same structure.
//!
//! In plain terms: a *chord* is a key plus its held modifiers (`Mod+Shift+E`);
//! a *context* is where you are (the whole app, the timeline, the viewer…); a
//! *binding* ties a chord in a context to an *action*. Two bindings clash when
//! the same chord could fire two different actions at once — and because a
//! Global binding is live everywhere, it clashes with a same-chord binding in
//! any context. That clash rule is the one genuinely fiddly thing here, so it
//! is what the tests pin hardest.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub mod after_effects;
pub mod defaults;

/// Something a chord can be bound to, identified by a stable string (e.g.
/// `"playback.toggle"`). A string — not a giant enum — so new commands never
/// force a breaking change and a keymap file stays readable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionId(pub String);

impl From<&str> for ActionId {
    fn from(s: &str) -> Self {
        ActionId(s.to_string())
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl ActionId {
    /// What this action is called in Settings → Keymap: the left-hand column of
    /// the table. Sentence case, British English, and it names the *effect*
    /// rather than transliterating the id — a reader who has never seen
    /// `playback.workarea.start` should still recognise "Go to work-area start".
    ///
    /// An unknown id falls back to its own text rather than a placeholder, so a
    /// keymap file carrying an action this build does not know still lists
    /// something a person can match against, and adding an action can never
    /// panic here for want of a row.
    #[must_use]
    pub fn description(&self) -> String {
        if let Some(n) = self.0.strip_prefix("workspace.switch.") {
            return format!("Switch to workspace {n}");
        }
        // The numbered markers. Twenty actions, so they are described by
        // their shape rather than listed one by one below — a table of twenty
        // near-identical rows is exactly what this prefix form is for.
        if let Some(n) = self.0.strip_prefix("marker.add.") {
            return format!("Add marker {n} at the playhead");
        }
        if let Some(n) = self.0.strip_prefix("marker.goto.") {
            return format!("Go to marker {n}");
        }
        let known = match self.0.as_str() {
            // Transport and navigation.
            "playback.toggle" => "Play or pause",
            "playback.shuttle.reverse" => "Shuttle backwards",
            "playback.shuttle.pause" => "Shuttle pause",
            "playback.shuttle.forward" => "Shuttle forwards",
            "playback.frame.next" => "Next frame",
            "playback.frame.prev" => "Previous frame",
            "playback.frame.next10" => "Forward ten frames",
            "playback.frame.prev10" => "Back ten frames",
            "playback.comp.start" => "Go to the start",
            "playback.loop.toggle" => "Turn looping on or off",
            "playback.comp.end" => "Go to the end",
            "playback.workarea.start" => "Go to work-area start",
            "playback.workarea.end" => "Go to work-area end",
            "playback.layer.in" => "Go to the layer's in point",
            "playback.layer.out" => "Go to the layer's out point",
            "keyframe.prev" => "Previous keyframe",
            "keyframe.next" => "Next keyframe",
            "edit.point.prev" => "Previous edit point",
            "edit.point.next" => "Next edit point",
            // Keyframes. The plain one acts on the picked property rows, the
            // five named ones on the selected layers.
            "keyframe.toggle" => "Add or remove a keyframe at the playhead",
            "keyframe.toggle.position" => "Add or remove a Position keyframe",
            "keyframe.toggle.scale" => "Add or remove a Scale keyframe",
            "keyframe.toggle.rotation" => "Add or remove a Rotation keyframe",
            "keyframe.toggle.opacity" => "Add or remove an Opacity keyframe",
            "keyframe.toggle.anchor" => "Add or remove an Anchor point keyframe",
            "keyframe.hold.toggle" => "Hold or release the keyframe",
            "keyframe.interpolation" => "Set the keyframe's interpolation",
            "keyframe.speed" => "Set the keyframe's speed",
            "expression.add" => "Add an expression",
            "cut.mark.in" => "Mark in",
            "cut.mark.out" => "Mark out",
            "cut.insert" => "Insert at the playhead",
            "cut.overwrite" => "Overwrite at the playhead",
            "cut.delete.ripple" => "Ripple delete",
            "cut.tool.select" => "Select clips",
            "cut.tool.razor" => "Cut clips with the razor",
            "cut.tool.ripple" => "Ripple trim",
            "cut.tool.roll" => "Roll an edit point",
            "cut.tool.slip" => "Slip a clip",
            "cut.tool.slide" => "Slide a clip",
            "cut.trim.start.ripple" => "Ripple trim the start to the playhead",
            "cut.trim.end.ripple" => "Ripple trim the end to the playhead",
            "cut.trim.start" => "Trim the start to the playhead",
            "cut.trim.end" => "Trim the end to the playhead",
            "cut.roll.prev" => "Roll the previous edit point to the playhead",
            "cut.roll.next" => "Roll the next edit point to the playhead",
            "cut.add.edit" => "Cut at the playhead",
            "cut.add.edit.all" => "Cut every track at the playhead",
            "cut.nudge.left" => "Nudge clips one frame earlier",
            "cut.nudge.right" => "Nudge clips one frame later",
            "cut.nudge.left.many" => "Nudge clips five frames earlier",
            "cut.nudge.right.many" => "Nudge clips five frames later",
            "cut.nudge.up" => "Move clips up a track",
            "cut.nudge.down" => "Move clips down a track",
            "cut.select.at.playhead" => "Select the clip at the playhead",
            "cut.snap.toggle" => "Turn snapping on or off",
            "cut.linked.toggle" => "Turn linked selection on or off",
            "cut.ripple.toggle" => "Turn auto ripple on or off",
            "cut.crossfade.toggle" => "Turn automatic crossfades on or off",
            "cut.quantise.toggle" => "Turn quantise to frames on or off",
            "cut.speed.toggle" => "Show or hide speed lines",
            "cut.group" => "Group the picked clips",
            "cut.ungroup" => "Take the picked clips out of their group",
            "cut.lift" => "Lift the work area",
            "cut.extract" => "Extract the work area",
            "cut.match.frame" => "Match frame",
            "cut.transition.default" => "Add the default transition",
            "workarea.set.start" => "Set work-area start to the playhead",
            "workarea.set.end" => "Set work-area end to the playhead",
            "marker.add" => "Add a marker at the playhead",
            "edit.delete.selection" => "Delete the selection",
            "edit.cut" => "Cut the selection",
            "edit.copy" => "Copy the selection",
            "edit.paste" => "Paste",
            "palette.open" => "Open the command palette",
            "console.open" => "Open the FX console",
            "comp.flowchart" => "Open the flowchart",
            "export.queue.add" => "Add to the export queue",
            "comp.settings" => "Composition settings",
            "comp.trim.workarea" => "Trim the composition to the work area",
            "export.queue.open" => "Open the export queue",
            "help.open" => "Open Lumit help",
            "edit.undo" => "Undo",
            "edit.redo" => "Redo",
            "edit.select.all" => "Select every layer",
            "edit.deselect.all" => "Deselect everything",
            "file.new" => "New project",
            "file.open" => "Open a project",
            "file.save" => "Save the project",
            "file.save.as" => "Save the project somewhere else",
            "file.save.version" => "Save a version with a name",
            "file.import" => "Import footage",
            "file.export" => "Export the composition",
            "comp.new" => "New composition",
            "app.settings" => "Open Settings",
            "project.settings" => "Open Project settings",
            "item.rename" => "Rename the selected item",
            "item.duplicate" => "Duplicate the selected item",
            "item.select.prev" => "Select the item above",
            "item.select.next" => "Select the item below",
            "item.select.prev.extend" => "Add the item above to the selection",
            "item.select.next.extend" => "Add the item below to the selection",
            "item.folder.close" => "Close the selected folder",
            "item.folder.open" => "Open the selected folder",
            "footage.interpret" => "Interpret the selected footage",
            "footage.interpretation.remember" => "Remember the footage's interpretation",
            "footage.interpretation.apply" => "Apply the remembered interpretation",
            "panel.maximise" => "Maximise the panel under the pointer",
            "graph.toggle" => "Show or hide the graph editor",
            // Tools.
            "tool.select" => "Selection tool",
            "tool.hand" => "Hand tool",
            "tool.zoom" => "Zoom tool",
            "tool.anchor" => "Anchor point tool",
            "tool.razor" => "Razor tool",
            "tool.shape" => "Shape tool",
            "tool.pen" => "Pen tool",
            "tool.rotate" => "Rotation tool",
            "tool.type" => "Type tool",
            "tool.paint" => "Brush tool",
            "tool.roto" => "Roto brush tool",
            "tool.puppet" => "Puppet tool",
            "tool.camera" => "Camera tool",
            // Timeline reveals and edits.
            "reveal.position" => "Reveal Position",
            "reveal.scale" => "Reveal Scale",
            "reveal.rotation" => "Reveal Rotation",
            "reveal.opacity" => "Reveal Opacity",
            "reveal.anchor" => "Reveal Anchor point",
            "reveal.effects" => "Reveal Effects",
            "reveal.masks" => "Reveal Masks",
            "reveal.animated" => "Reveal animated properties",
            "reveal.volume" => "Reveal Volume",
            "reveal.audio" => "Reveal Audio, again for the waveform",
            "layer.move.in" => "Move the layer's in point to the playhead",
            "layer.move.out" => "Move the layer's out point to the playhead",
            "layer.trim.in" => "Trim the layer's in point to the playhead",
            "layer.trim.out" => "Trim the layer's out point to the playhead",
            "layer.split" => "Split the layer at the playhead",
            "layer.duplicate" => "Duplicate the layer",
            "layer.precompose" => "Pre-compose the layer",
            "layer.group" => "Group the selected layers",
            "layer.ungroup" => "Ungroup",
            "layer.retime.enable" => "Give the layer a Retime",
            "layer.new.solid" => "New Solid layer",
            "layer.new.text" => "New Text layer",
            "layer.new.camera" => "New Camera layer",
            "layer.new.light.point" => "New Light layer",
            "layer.new.adjustment" => "New Adjustment layer",
            "layer.new.null" => "New Null layer",
            "layer.new.sequence" => "New Sequence layer",
            "layer.new.audio" => "New Audio layer",
            "layer.settings" => "Layer settings",
            "layer.blend.next" => "Next blend mode",
            "layer.blend.prev" => "Previous blend mode",
            "layer.lock.toggle" => "Lock or unlock the layer",
            "layer.unlock.all" => "Unlock every layer",
            "layer.select.next" => "Select the layer below",
            "layer.select.prev" => "Select the layer above",
            "layer.order.up" => "Move the layer up the stack",
            "layer.order.down" => "Move the layer down the stack",
            "layer.order.top" => "Move the layer to the top of the stack",
            "layer.order.bottom" => "Move the layer to the bottom of the stack",
            "layer.shift.earlier" => "Move the layer a frame earlier",
            "layer.shift.later" => "Move the layer a frame later",
            "layer.shift.earlier10" => "Move the layer ten frames earlier",
            "layer.shift.later10" => "Move the layer ten frames later",
            "layer.centre" => "Centre the layer in the composition",
            "layer.centre.anchor" => "Centre the anchor point in the layer",
            "mask.feather" => "Set the mask's feather",
            "mask.invert" => "Invert the mask",
            "timeline.zoom.in" => "Zoom in",
            "timeline.zoom.out" => "Zoom out",
            "timeline.zoom.fit" => "Zoom to fit",
            "timeline.fit.keys" => "Fit the keyframes to the panel",
            "layer.rename" => "Rename the layer",
            "effect.rename" => "Rename the selected effect",
            "layer.toggle.visible" => "Show or hide the layer",
            // Graph editor.
            "graph.ease" => "Easy ease",
            "graph.ease.in" => "Easy ease in",
            "graph.ease.out" => "Easy ease out",
            "graph.fit" => "Fit the curves to the pane",
            // Viewer.
            "viewer.zoom.fit" => "Fit the picture to the panel",
            "viewer.zoom.in" => "Zoom in",
            "viewer.zoom.out" => "Zoom out",
            "viewer.res.full" => "Full resolution",
            "viewer.res.half" => "Half resolution",
            "viewer.res.quarter" => "Quarter resolution",
            "viewer.rulers.toggle" => "Show or hide the rulers",
            "viewer.grid.toggle" => "Show or hide the grid",
            "viewer.new" => "Open another Viewer",
            "viewer.new.viewport" => "Open a 3D viewport",
            "viewer.view.next" => "Front the next view",
            "viewer.view.prev" => "Front the previous view",
            "viewer.lock.toggle" => "Lock or unlock this view",
            "viewer.layout.one" => "One view",
            "viewer.layout.two" => "Two views",
            "viewer.layout.four" => "Four views",
            "viewer.layout.orientation" => "Turn the two-view layout",
            "viewer.cinema" => "Fill the window with this view",
            "viewer.compare" => "Compare two views",
            "viewer.preview.always" => "Always preview this view",
            "viewer.controls.toggle" => "Show or hide the layer controls",
            "viewer.snap.grid.toggle" => "Turn snapping to the grid on or off",
            "viewer.3d.slot.1" => "Switch to 3D view shortcut 1",
            "viewer.3d.slot.2" => "Switch to 3D view shortcut 2",
            "viewer.3d.slot.3" => "Switch to 3D view shortcut 3",
            "viewer.3d.assign.1" => "Assign the 3D view to shortcut 1",
            "viewer.3d.assign.2" => "Assign the 3D view to shortcut 2",
            "viewer.3d.assign.3" => "Assign the 3D view to shortcut 3",
            "viewer.3d.last" => "Switch to the last 3D view",
            "viewer.3d.camera" => "Back to the active camera",
            "viewer.3d.look.selected" => "Look at the selected layers",
            "viewer.3d.look.all" => "Look at all layers",
            "viewer.3d.frame.all" => "Frame all layers in a 3D viewport",
            "viewer.3d.reset" => "Reset the 3D view",
            "viewer.3d.camera.new" => "Create a camera from the 3D view",
            "viewer.3d.camera.move" => "Move the active camera to the 3D view",
            "viewer.3d.place" => "Place the selected layers on a surface",
            // Panels.
            "panel.toggle.project" => "Show or hide the Project panel",
            "panel.toggle.effects" => "Show or hide Effect controls",
            "panel.toggle.presets" => "Show or hide Effects & presets",
            "panel.toggle.audio" => "Show or hide the Audio panel",
            "panel.toggle.text" => "Show or hide the Text panel",
            "panel.toggle.paragraph" => "Show or hide the Paragraph panel",
            "panel.focus.next" => "Focus the next panel",
            "panel.focus.prev" => "Focus the previous panel",
            "panel.search.focus" => "Focus the panel's search box",
            _ => return self.0.clone(),
        };
        known.to_string()
    }
}

/// Where a binding is live. `Global` is live in every context; the rest are the
/// focused panels a binding can be scoped to (docs/07 §15 "per-context").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyContext {
    Global,
    /// Tool selection (the toolbar): V/H/Z/… (docs/07 §15 "Tools").
    Tools,
    Project,
    Timeline,
    Viewer,
    Graph,
    /// Panel focus/search shortcuts (docs/07 §15 "Panels").
    Panels,
    Effects,
    /// The Cut workspace's timeline, where the editor's own letters shadow
    /// the app-wide ones.
    Cut,
}

impl KeyContext {
    /// Every context, in the order Settings → Keymap lists them: the ones live
    /// everywhere first, then the panels roughly as the workspace reads.
    pub const ALL: [KeyContext; 9] = [
        KeyContext::Global,
        KeyContext::Tools,
        KeyContext::Project,
        KeyContext::Timeline,
        KeyContext::Viewer,
        KeyContext::Graph,
        KeyContext::Panels,
        KeyContext::Effects,
        KeyContext::Cut,
    ];

    /// The heading Settings → Keymap puts above this context's table. Sentence
    /// case, and it says *where* the binding is live rather than naming the
    /// enum — "Anywhere" reads truthfully to someone who has never heard the
    /// word "global".
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            KeyContext::Global => "Anywhere",
            KeyContext::Tools => "Tools",
            KeyContext::Project => "Project panel",
            KeyContext::Timeline => "Timeline",
            KeyContext::Viewer => "Viewer",
            KeyContext::Graph => "Graph editor",
            KeyContext::Panels => "Panels",
            KeyContext::Effects => "Effect controls",
            KeyContext::Cut => "Cut timeline",
        }
    }
}

/// The modifier keys held with the main key. `primary` is Ctrl on Windows and
/// Cmd on macOS — the platform split lives in the UI layer, so the keymap and
/// its shared file stay platform-neutral (docs/07 §15 "Ctrl/Cmd").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
}

/// A key plus its modifiers, e.g. `Space`, `J`, `Shift+F3`, `Mod+Shift+E`.
///
/// The key is stored normalised (single letters upper-cased) so `mod+d` and
/// `Mod+D` are the same chord. Parsing is order-insensitive and accepts the
/// usual modifier spellings (`Ctrl`/`Cmd`/`Mod`, `Alt`/`Option`, `Shift`);
/// [`fmt::Display`] emits one canonical form that round-trips.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Chord {
    pub mods: Modifiers,
    pub key: String,
}

/// What went wrong parsing a [`Chord`] from text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordError {
    /// The text had no key (empty, or only modifiers).
    Empty,
    /// A `+`-separated token before the key was not a known modifier.
    UnknownModifier(String),
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChordError::Empty => f.write_str("chord has no key"),
            ChordError::UnknownModifier(m) => write!(f, "unknown modifier: {m}"),
        }
    }
}

impl std::error::Error for ChordError {}

/// Normalise a bare key token: single ASCII letters upper-case (so `d` == `D`),
/// everything else (named keys, punctuation) kept verbatim after trimming.
fn normalise_key(raw: &str) -> String {
    let k = raw.trim();
    if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()) {
        k.to_ascii_uppercase()
    } else {
        k.to_string()
    }
}

impl FromStr for Chord {
    type Err = ChordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut mods = Modifiers::default();
        // The final token is always the key, even if it spells a modifier
        // word (so `Shift` alone is the Shift *key*). The plus key is written
        // with the sign that joins the others, so `+` and `Shift++` are read
        // for what they are before anything is split on it: otherwise the
        // chord would not read back, and a stored keymap holding one would
        // not load at all.
        let (held, key) = match s.strip_suffix('+') {
            Some(rest) if rest.is_empty() || rest.ends_with('+') => {
                (rest.strip_suffix('+').unwrap_or(""), "+")
            }
            _ => s.rsplit_once('+').unwrap_or(("", s)),
        };
        for tok in held.split('+').filter(|_| !held.is_empty()) {
            match tok.trim().to_ascii_lowercase().as_str() {
                "mod" | "cmd" | "command" | "ctrl" | "control" | "primary" => mods.primary = true,
                "shift" => mods.shift = true,
                "alt" | "option" | "opt" => mods.alt = true,
                other => return Err(ChordError::UnknownModifier(other.to_string())),
            }
        }
        match normalise_key(key) {
            k if !k.is_empty() => Ok(Chord { mods, key: k }),
            _ => Err(ChordError::Empty),
        }
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.primary {
            f.write_str("Mod+")?;
        }
        if self.mods.alt {
            f.write_str("Alt+")?;
        }
        if self.mods.shift {
            f.write_str("Shift+")?;
        }
        f.write_str(&self.key)
    }
}

impl From<Chord> for String {
    fn from(c: Chord) -> Self {
        c.to_string()
    }
}

impl TryFrom<String> for Chord {
    type Error = ChordError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// One entry of a keymap: a chord, in a context, runs an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub context: KeyContext,
    pub chord: Chord,
    pub action: ActionId,
}

/// One chord a panel takes over from an app-wide binding.
///
/// Not a conflict: [`Keymap::lookup`] resolves it by a stated rule — the
/// focused panel gets first refusal, and `Global` is the fallback — so the
/// chord always runs exactly one action and which one is never in doubt.
/// Worth *saying*, though, because the app-wide meaning silently stops working
/// in that one panel, which is a thing somebody reading their keymap should be
/// able to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadow {
    pub chord: Chord,
    /// The panel whose binding wins while it is focused.
    pub context: KeyContext,
    /// What the chord does there.
    pub action: ActionId,
    /// What it does everywhere else.
    pub shadowed: ActionId,
}

/// A set of chords sharing one chord that resolves to more than one action —
/// what Settings → Keymap flags for the user to resolve (docs/07 §15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub chord: Chord,
    /// The distinct actions competing for this chord, in first-seen order.
    pub actions: Vec<ActionId>,
}

/// The whole keymap: an ordered list of bindings plus the operations Settings →
/// Keymap needs (lookup, conflict detection, search, rebinding).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keymap {
    pub bindings: Vec<Binding>,

    /// Actions deliberately left with **no** chord, so a stored keymap
    /// can tell "I took that key away" apart from "that action did not exist
    /// when I was written". [`with_new_defaults`] needs the difference: the
    /// first must stay unbound, the second must pick up its default.
    ///
    /// Absent from an older file, which is exactly right — nothing in one was
    /// ever a deliberate unbind that outlived a restart, because the whole map
    /// was replaced on load.
    #[serde(default)]
    pub unbound: Vec<(KeyContext, ActionId)>,

    /// Which modifier turns the scroll wheel into each of its actions. An older
    /// file without it gets the shipped ones.
    #[serde(default)]
    pub wheel: WheelKeys,

    /// The modifier that breaks or joins a keyframe's handles when it is held as
    /// a handle drag begins in the Graph editor. An older file without it gets Alt.
    #[serde(default)]
    pub break_handles: HandleModifier,
}

/// A modifier held as a handle drag begins. Shift isn't offered, since it
/// already constrains the drag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HandleModifier {
    #[default]
    Alt,
    Ctrl,
}

/// A modifier held with the scroll wheel. Ctrl is the Control key on every
/// platform, since that is what the wheel has always read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WheelModifier {
    Ctrl,
    Alt,
    Shift,
    /// The wheel on its own, with no key held: Vegas zooms time on it.
    Bare,
}

/// Something the scroll wheel does while a modifier is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WheelAction {
    /// Zoom time about the pointer, in the Timeline, Audio timeline and Graph editor.
    ZoomTime,
    /// Scroll the lanes sideways, in the same three places.
    ScrollSideways,
    /// Zoom the value axis in the Graph editor when auto fit is off.
    ZoomValues,
    /// Grow or shrink the dropper's sample in the Viewer.
    DropperSample,
    /// Dolly a 3D view toward the pointer in the Viewer, with any tool.
    CameraDolly,
}

impl WheelAction {
    /// Every wheel action, in the order Settings lists them.
    pub const ALL: [WheelAction; 5] = [
        WheelAction::ZoomTime,
        WheelAction::ScrollSideways,
        WheelAction::ZoomValues,
        WheelAction::DropperSample,
        WheelAction::CameraDolly,
    ];

    /// Whether both actions are live in one panel, so they can't share a modifier.
    /// The first three all meet in the Graph editor, the last two in the Viewer.
    /// Zooming time and values can share one, and the Graph editor then zooms both.
    fn shares_a_panel_with(self, other: WheelAction) -> bool {
        use WheelAction::{CameraDolly, DropperSample, ZoomTime, ZoomValues};
        let viewer = |a| matches!(a, DropperSample | CameraDolly);
        self != other
            && viewer(self) == viewer(other)
            && !matches!(
                (self, other),
                (ZoomTime, ZoomValues) | (ZoomValues, ZoomTime)
            )
    }
}

/// The modifier for each wheel action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WheelKeys {
    pub zoom_time: WheelModifier,
    pub scroll_sideways: WheelModifier,
    pub zoom_values: WheelModifier,
    pub dropper_sample: WheelModifier,
    pub camera_dolly: WheelModifier,
}

impl Default for WheelKeys {
    fn default() -> Self {
        WheelKeys {
            zoom_time: WheelModifier::Ctrl,
            scroll_sideways: WheelModifier::Shift,
            zoom_values: WheelModifier::Alt,
            dropper_sample: WheelModifier::Shift,
            camera_dolly: WheelModifier::Alt,
        }
    }
}

impl WheelKeys {
    /// The modifier `action` answers to.
    #[must_use]
    pub fn modifier(&self, action: WheelAction) -> WheelModifier {
        match action {
            WheelAction::ZoomTime => self.zoom_time,
            WheelAction::ScrollSideways => self.scroll_sideways,
            WheelAction::ZoomValues => self.zoom_values,
            WheelAction::DropperSample => self.dropper_sample,
            WheelAction::CameraDolly => self.camera_dolly,
        }
    }

    fn slot(&mut self, action: WheelAction) -> &mut WheelModifier {
        match action {
            WheelAction::ZoomTime => &mut self.zoom_time,
            WheelAction::ScrollSideways => &mut self.scroll_sideways,
            WheelAction::ZoomValues => &mut self.zoom_values,
            WheelAction::DropperSample => &mut self.dropper_sample,
            WheelAction::CameraDolly => &mut self.camera_dolly,
        }
    }

    /// Give `action` a new modifier. An action in the same panel already on it
    /// takes the old one, so the two swap and the wheel never means two things.
    pub fn set(&mut self, action: WheelAction, modifier: WheelModifier) {
        let old = self.modifier(action);
        for other in WheelAction::ALL {
            if action.shares_a_panel_with(other) && self.modifier(other) == modifier {
                *self.slot(other) = old;
            }
        }
        *self.slot(action) = modifier;
    }
}

impl Keymap {
    /// The action a chord runs while `active` is the focused context: a binding
    /// in `active` wins over a `Global` one (the focused panel gets first
    /// refusal), and `Global` is the fallback. `None` when nothing matches.
    #[must_use]
    pub fn lookup(&self, active: KeyContext, chord: &Chord) -> Option<&ActionId> {
        let exact = self
            .bindings
            .iter()
            .find(|b| b.context == active && &b.chord == chord);
        if let Some(b) = exact {
            return Some(&b.action);
        }
        self.bindings
            .iter()
            .find(|b| b.context == KeyContext::Global && &b.chord == chord)
            .map(|b| &b.action)
    }

    /// Every chord that could fire more than one action in overlapping contexts
    /// (docs/07 §15 conflict detection). Empty when the keymap is unambiguous.
    #[must_use]
    pub fn conflicts(&self) -> Vec<Conflict> {
        // Group bindings by chord once (first-seen chord order preserved),
        // instead of re-scanning the whole list per binding.
        let mut groups: std::collections::HashMap<&Chord, Vec<&Binding>> =
            std::collections::HashMap::new();
        let mut order: Vec<&Chord> = Vec::new();
        for b in &self.bindings {
            let group = groups.entry(&b.chord).or_default();
            if group.is_empty() {
                order.push(&b.chord);
            }
            group.push(b);
        }
        let mut out: Vec<Conflict> = Vec::new();
        for chord in order {
            let same = &groups[chord];
            // Collect the distinct actions that can genuinely collide: two
            // bindings in the *same* context, which nothing can tell apart.
            //
            // A `Global` binding under a scoped one is **not** one of those:
            // `lookup` resolves it by a stated precedence — the focused panel
            // first, `Global` as the fallback — so the chord runs one action
            // and which one is never ambiguous. Flagging it as a clash made
            // the shipped default unable to give a panel a plain letter that
            // transport already used, which is how `L` could not mean "reveal
            // Audio" in the Timeline while J/K/L still shuttled everywhere
            // else.
            // [`Keymap::shadows`] reports those pairs instead.
            let mut per_context: std::collections::HashMap<KeyContext, usize> =
                std::collections::HashMap::new();
            for b in same {
                *per_context.entry(b.context).or_default() += 1;
            }
            let mut actions: Vec<ActionId> = Vec::new();
            for b in same {
                if per_context[&b.context] > 1 && !actions.contains(&b.action) {
                    actions.push(b.action.clone());
                }
            }
            if actions.len() > 1 {
                out.push(Conflict {
                    chord: chord.clone(),
                    actions,
                });
            }
        }
        out
    }

    /// Every chord a panel takes over from an app-wide binding — what
    /// Settings → Keymap says beside the row rather than flagging as a clash.
    #[must_use]
    pub fn shadows(&self) -> Vec<Shadow> {
        let mut out = Vec::new();
        for b in &self.bindings {
            if b.context == KeyContext::Global {
                continue;
            }
            let Some(global) = self
                .bindings
                .iter()
                .find(|o| o.context == KeyContext::Global && o.chord == b.chord)
            else {
                continue;
            };
            // The same action bound twice shadows nothing: the chord does the
            // same thing either way.
            if global.action == b.action {
                continue;
            }
            out.push(Shadow {
                chord: b.chord.clone(),
                context: b.context,
                action: b.action.clone(),
                shadowed: global.action.clone(),
            });
        }
        out
    }

    /// Bind `chord` in `context` to `action`, replacing any existing binding for
    /// the exact same `(context, chord)` so a rebind never silently duplicates.
    ///
    /// An action that loses its only chord here is remembered as unbound on
    /// purpose. Otherwise the next start would give it its default back, the
    /// very chord just taken from it.
    pub fn bind(&mut self, context: KeyContext, chord: Chord, action: ActionId) {
        let displaced: Vec<ActionId> = self
            .bindings
            .iter()
            .filter(|b| b.context == context && b.chord == chord && b.action != action)
            .map(|b| b.action.clone())
            .collect();
        self.bindings
            .retain(|b| !(b.context == context && b.chord == chord));
        self.bindings.push(Binding {
            context,
            chord,
            action,
        });
        for lost in displaced {
            if self.binding_for(context, &lost).is_none() {
                self.unbind_action(context, &lost);
            }
        }
    }

    /// Remove the binding for an exact `(context, chord)`, if any. Returns
    /// whether something was removed.
    pub fn unbind(&mut self, context: KeyContext, chord: &Chord) -> bool {
        let before = self.bindings.len();
        self.bindings
            .retain(|b| !(b.context == context && &b.chord == chord));
        self.bindings.len() != before
    }

    /// The chord currently bound to `action` in `context`, if any. The table in
    /// Settings → Keymap is one row per (context, action), so this is what fills
    /// its right-hand column — one action, one chord, like everything else
    /// (Retime is no exception).
    #[must_use]
    pub fn binding_for(&self, context: KeyContext, action: &ActionId) -> Option<&Chord> {
        self.bindings
            .iter()
            .find(|b| b.context == context && &b.action == action)
            .map(|b| &b.chord)
    }

    /// Point `action` at `chord` in `context` — what clicking a row's chord and
    /// pressing a new one does.
    ///
    /// This is *by action*, not by chord, and the difference is the whole
    /// reason it exists: [`bind`](Self::bind) would leave the action's old
    /// chord in place and the row would then answer to two keys, one of them
    /// invisible in the table. So the action's previous binding in this context
    /// goes first, and the new chord replaces whatever else held it there.
    ///
    /// A chord already spoken for is never *refused*, and what happens to its
    /// previous owner depends on where that owner lives:
    ///
    /// - **Same context**: it loses the chord, and its row in the table goes
    ///   blank. Nothing is hidden — the user can see the key moved, and can
    ///   give the emptied row a new one.
    /// - **Overlapping context** (one of them `Global`): both keep their
    ///   binding and the pair is reported by [`conflicts`](Self::conflicts) for
    ///   the user to resolve.
    ///
    /// Refusing instead would make some rebinding orders impossible to type
    /// your way out of — swapping two actions' keys needs an intermediate state
    /// where one chord is claimed twice.
    pub fn rebind_action(&mut self, context: KeyContext, action: &ActionId, chord: Chord) {
        self.bindings
            .retain(|b| !(b.context == context && &b.action == action));
        // It has a key again, so it is no longer one of the deliberately
        // silent ones.
        self.unbound
            .retain(|(c, a)| !(*c == context && a == action));
        self.bind(context, chord, action.clone());
    }

    /// Leave `action` with no chord at all, and **remember that it was meant** —
    /// otherwise the next start would hand it its default back, since a missing
    /// binding is also what an action added since this file was written looks
    /// like.
    pub fn unbind_action(&mut self, context: KeyContext, action: &ActionId) {
        self.bindings
            .retain(|b| !(b.context == context && &b.action == action));
        if !self
            .unbound
            .iter()
            .any(|(c, a)| *c == context && a == action)
        {
            self.unbound.push((context, action.clone()));
        }
    }

    /// Bindings whose action id or chord text contains `query`
    /// (case-insensitive) — the Settings → Keymap search box.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&Binding> {
        let q = query.to_ascii_lowercase();
        self.bindings
            .iter()
            .filter(|b| {
                b.action.0.to_ascii_lowercase().contains(&q)
                    || b.chord.to_string().to_ascii_lowercase().contains(&q)
                    // The description too, because it is what the table shows:
                    // searching for the words on screen and getting nothing is
                    // the sort of thing that makes a search box feel broken.
                    || b.action.description().to_ascii_lowercase().contains(&q)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
