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
        // The numbered markers (K-254). Twenty actions, so they are described by
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
            "playback.comp.end" => "Go to the end",
            "playback.workarea.start" => "Go to work-area start",
            "playback.workarea.end" => "Go to work-area end",
            "playback.layer.in" => "Go to the layer's in point",
            "playback.layer.out" => "Go to the layer's out point",
            "keyframe.prev" => "Previous keyframe",
            "keyframe.next" => "Next keyframe",
            "edit.point.prev" => "Previous edit point",
            "edit.point.next" => "Next edit point",
            "workarea.set.start" => "Set work-area start to the playhead",
            "workarea.set.end" => "Set work-area end to the playhead",
            "marker.add" => "Add a marker at the playhead",
            "edit.delete.selection" => "Delete the selection",
            "edit.cut" => "Cut the selection",
            "edit.copy" => "Copy the selection",
            "edit.paste" => "Paste",
            "palette.open" => "Open the command palette",
            "console.open" => "Open the FX console",
            "export.queue.add" => "Add to the export queue",
            "comp.settings" => "Composition settings",
            "edit.undo" => "Undo",
            "edit.redo" => "Redo",
            "edit.select.all" => "Select every layer",
            "edit.deselect.all" => "Deselect everything",
            "file.new" => "New project",
            "file.open" => "Open a project",
            "file.save" => "Save the project",
            "file.save.as" => "Save the project somewhere else",
            "file.import" => "Import footage",
            "file.export" => "Export the composition",
            "comp.new" => "New composition",
            "app.settings" => "Open Settings",
            "project.settings" => "Open Project settings",
            "item.rename" => "Rename the selected item",
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
            "layer.retime.enable" => "Give the layer a Retime",
            "timeline.zoom.in" => "Zoom in",
            "timeline.zoom.out" => "Zoom out",
            "timeline.zoom.fit" => "Zoom to fit",
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
            // Panels.
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
}

impl KeyContext {
    /// Every context, in the order Settings → Keymap lists them: the ones live
    /// everywhere first, then the panels roughly as the workspace reads.
    pub const ALL: [KeyContext; 8] = [
        KeyContext::Global,
        KeyContext::Tools,
        KeyContext::Project,
        KeyContext::Timeline,
        KeyContext::Viewer,
        KeyContext::Graph,
        KeyContext::Panels,
        KeyContext::Effects,
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
        let mut key: Option<String> = None;
        let tokens: Vec<&str> = s.split('+').collect();
        let last = tokens.len().saturating_sub(1);
        for (i, tok) in tokens.iter().enumerate() {
            let t = tok.trim();
            if i == last {
                // The final token is always the key, even if it spells a
                // modifier word (so `Shift` alone is the Shift *key*).
                key = Some(normalise_key(t));
                break;
            }
            match t.to_ascii_lowercase().as_str() {
                "mod" | "cmd" | "command" | "ctrl" | "control" | "primary" => mods.primary = true,
                "shift" => mods.shift = true,
                "alt" | "option" | "opt" => mods.alt = true,
                other => return Err(ChordError::UnknownModifier(other.to_string())),
            }
        }
        match key {
            Some(k) if !k.is_empty() => Ok(Chord { mods, key: k }),
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

/// One chord a panel takes over from an app-wide binding (K-281).
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

    /// Actions deliberately left with **no** chord (K-302), so a stored keymap
    /// can tell "I took that key away" apart from "that action did not exist
    /// when I was written". [`with_new_defaults`] needs the difference: the
    /// first must stay unbound, the second must pick up its default.
    ///
    /// Absent from an older file, which is exactly right — nothing in one was
    /// ever a deliberate unbind that outlived a restart, because the whole map
    /// was replaced on load.
    #[serde(default)]
    pub unbound: Vec<(KeyContext, ActionId)>,
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
            // A `Global` binding under a scoped one is **not** one of those
            // (K-281, superseding the original rule): `lookup` resolves it by a
            // stated precedence — the focused panel first, `Global` as the
            // fallback — so the chord runs one action and which one is never
            // ambiguous. Flagging it as a clash made the shipped default
            // unable to give a panel a plain letter that transport already
            // used, which is how `L` could not mean "reveal Audio" in the
            // Timeline while J/K/L still shuttled everywhere else.
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

    /// Every chord a panel takes over from an app-wide binding (K-281) — what
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
    pub fn bind(&mut self, context: KeyContext, chord: Chord, action: ActionId) {
        self.bindings
            .retain(|b| !(b.context == context && b.chord == chord));
        self.bindings.push(Binding {
            context,
            chord,
            action,
        });
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
    /// (K-200 settled that Retime is no exception).
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
        // silent ones (K-302).
        self.unbound
            .retain(|(c, a)| !(*c == context && a == action));
        self.bind(context, chord, action.clone());
    }

    /// Leave `action` with no chord at all, and **remember that it was meant**
    /// (K-302) — otherwise the next start would hand it its default back, since
    /// a missing binding is also what an action added since this file was
    /// written looks like.
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
    use KeyContext::{Effects, Global, Graph, Panels, Project, Timeline, Tools, Viewer};
    let rows = [
        // --- Global: transport, navigation, app-wide commands ---
        row(Global, "Space", "playback.toggle"),
        row(Global, "J", "playback.shuttle.reverse"),
        row(Global, "K", "playback.shuttle.pause"),
        row(Global, "L", "playback.shuttle.forward"),
        // Stepping a frame is `Mod`+arrow, not the bare arrow (K-282). The
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
        row(Global, "B", "workarea.set.start"),
        row(Global, "N", "workarea.set.end"),
        row(Global, "*", "marker.add"),
        // `Shift+M` is the second way in (K-254). Premiere and Vegas both drop a
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
        // Cut, copy and paste had menu rows and no chords at all (K-300): the
        // three keys everyone's fingers reach for first did nothing to a
        // selected layer. Global, because what they act on is whatever is
        // selected — keyframes in the Timeline, else an effect, else the layer.
        row(Global, "Mod+X", "edit.cut"),
        row(Global, "Mod+C", "edit.copy"),
        row(Global, "Mod+V", "edit.paste"),
        row(Global, "Mod+Shift+P", "palette.open"),
        // The FX console (K-324): Video Copilot's own chord, and the one the
        // owner asked for.
        row(Global, "Mod+Space", "console.open"),
        row(Global, "Mod+M", "export.queue.add"),
        row(Global, "Mod+K", "comp.settings"),
        row(Global, "Mod+Z", "edit.undo"),
        row(Global, "Mod+Shift+Z", "edit.redo"),
        row(Global, "Mod+S", "file.save"),
        // The rest of the menu bar's own commands (K-244). After Effects'
        // chords where it has one and Lumit has not already spent the key:
        // Mod+N makes a *composition* there and Mod+Alt+N a project, which is
        // the pair anyone arriving from AE has in their fingers. Settings is
        // AE's Preferences chord for the same reason.
        row(Global, "Mod+Alt+N", "file.new"),
        row(Global, "Mod+O", "file.open"),
        row(Global, "Mod+Shift+S", "file.save.as"),
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
        // a misremembering (K-200, superseding that half of K-198), and it
        // collided with the Windows input-language switch anyway. One chord,
        // like every other action; anyone who wants a second can bind it.
        row(Global, "Mod+Alt+T", "layer.retime.enable"),
        // --- Tools ---
        row(Tools, "V", "tool.select"),
        row(Tools, "H", "tool.hand"),
        row(Tools, "Z", "tool.zoom"),
        row(Tools, "Y", "tool.anchor"),
        row(Tools, "C", "tool.razor"),
        row(Tools, "Q", "tool.shape"),
        row(Tools, "G", "tool.pen"),
        // The rest of the toolbar (K-216). After Effects' own chords wherever
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
        // a third shuts the layer again (K-281). It shadows the J/K/L shuttle
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
        row(Timeline, "=", "timeline.zoom.in"),
        row(Timeline, "-", "timeline.zoom.out"),
        row(Timeline, "\\", "timeline.zoom.fit"),
        row(Timeline, "Enter", "layer.rename"),
        // Enter renames the selected thing wherever one is selected (K-321):
        // the item in the Project panel, the effect in Effect controls — the
        // same key the Timeline has always used for its layers.
        row(Project, "Enter", "item.rename"),
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
        // --- Panels ---
        row(Panels, "Mod+F6", "panel.focus.next"),
        row(Panels, "Mod+Shift+F6", "panel.focus.prev"),
        row(Panels, "Mod+F", "panel.search.focus"),
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
    // Numbered markers (K-254): `Shift+N` drops marker *N* at the playhead and
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
    }
}

/// A **stored** keymap laid over the shipped defaults (K-302): the file's chord
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
    // AE has no J/K/L shuttle; the letters go back to their AE meanings.
    for k in ["J", "K", "L"] {
        if let Ok(chord) = k.parse::<Chord>() {
            km.unbind(KeyContext::Global, &chord);
        }
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
    km
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
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
        for s in ["Space", "Mod+D", "Shift+F3", "Mod+Alt+Shift+K", "="] {
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
    fn the_fx_console_has_its_own_chord_and_does_not_clash() {
        // K-324: Ctrl+Space opens the console. Video Copilot's own chord, and
        // the shipped map must stay conflict-free with it in.
        let km = default_keymap();
        assert_eq!(
            km.lookup(KeyContext::Global, &"Mod+Space".parse().unwrap()),
            Some(&ActionId::from("console.open"))
        );
        // The bare space bar still plays; the console took the modified one.
        assert_eq!(
            km.lookup(KeyContext::Global, &"Space".parse().unwrap()),
            Some(&ActionId::from("playback.toggle"))
        );
        assert!(km.conflicts().is_empty(), "the shipped map ships clean");
    }

    #[test]
    fn enter_renames_the_selection_in_each_panel_that_has_one() {
        // K-321: the same key the Timeline always used for its layers is
        // bound for the Project panel's items and Effect controls' effects.
        let km = default_keymap();
        let enter: Chord = "Enter".parse().unwrap();
        assert_eq!(
            km.lookup(KeyContext::Timeline, &enter),
            Some(&ActionId::from("layer.rename"))
        );
        assert_eq!(
            km.lookup(KeyContext::Project, &enter),
            Some(&ActionId::from("item.rename"))
        );
        assert_eq!(
            km.lookup(KeyContext::Effects, &enter),
            Some(&ActionId::from("effect.rename"))
        );
    }

    #[test]
    fn lookup_prefers_the_active_context_then_falls_back_to_global() {
        let mut km = Keymap::default();
        km.bind(KeyContext::Global, chord("Mod+K"), "global.k".into());
        km.bind(KeyContext::Timeline, chord("Mod+K"), "timeline.k".into());
        // In the timeline, the scoped binding wins.
        assert_eq!(
            km.lookup(KeyContext::Timeline, &chord("Mod+K")),
            Some(&"timeline.k".into())
        );
        // Elsewhere, the global one is the fallback.
        assert_eq!(
            km.lookup(KeyContext::Viewer, &chord("Mod+K")),
            Some(&"global.k".into())
        );
        // Unbound chord resolves to nothing.
        assert_eq!(km.lookup(KeyContext::Viewer, &chord("Mod+J")), None);
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

        // Global under a scoped binding on the same chord → NOT a conflict
        // (K-281): the panel gets first refusal and Global is the fallback, so
        // the chord runs one action and which one is never in doubt. It is
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

    /// Stepping a frame is `Mod`+arrow (K-282), and the bare arrows are free.
    #[test]
    fn a_frame_step_takes_the_primary_modifier() {
        let km = default_keymap();
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("Mod+ArrowRight")),
            Some(&ActionId::from("playback.frame.next"))
        );
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("Mod+ArrowLeft")),
            Some(&ActionId::from("playback.frame.prev"))
        );
        assert_eq!(km.lookup(KeyContext::Global, &chord("ArrowRight")), None);
        assert_eq!(km.lookup(KeyContext::Timeline, &chord("ArrowLeft")), None);
        // The page keys still step a frame unmodified — the chord moved, the
        // other way of doing it did not.
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("PageDown")),
            Some(&ActionId::from("playback.frame.next"))
        );
    }

    /// `L` reveals a layer's Audio in the Timeline and shuttles forward
    /// everywhere else (K-281) — the one shadow the default ships with, and it
    /// is deliberate.
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
            vec!["L".to_string()],
            "one deliberate shadow, and it is named in the docs"
        );
    }

    #[test]
    fn bind_replaces_and_unbind_removes_the_exact_entry() {
        let mut km = Keymap::default();
        km.bind(KeyContext::Global, chord("Mod+D"), "one".into());
        km.bind(KeyContext::Global, chord("Mod+D"), "two".into());
        assert_eq!(km.bindings.len(), 1, "rebind replaces, not duplicates");
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("Mod+D")),
            Some(&"two".into())
        );
        assert!(km.unbind(KeyContext::Global, &chord("Mod+D")));
        assert!(!km.unbind(KeyContext::Global, &chord("Mod+D")));
        assert!(km.lookup(KeyContext::Global, &chord("Mod+D")).is_none());
    }

    #[test]
    fn search_matches_action_and_chord_text() {
        let km = default_keymap();
        assert!(km.search("undo").iter().any(|b| b.action.0 == "edit.undo"));
        assert!(km
            .search("shift+f3")
            .iter()
            .any(|b| b.action.0 == "graph.toggle"));
        assert!(km.search("nonexistent-xyz").is_empty());
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

    /// The numbered markers (K-254). `Shift+N` sets and the bare `N` returns, for
    /// all ten digits including zero — and `M` must still reveal Masks in the
    /// Timeline, which is the whole reason the marker key is `Shift+M`.
    #[test]
    fn numbered_markers_bind_set_and_return_for_every_digit() {
        let km = default_keymap();
        for d in 0..=9u8 {
            assert_eq!(
                km.lookup(KeyContext::Global, &chord(&format!("Shift+{d}"))),
                Some(&ActionId(format!("marker.add.{d}"))),
                "Shift+{d} should set marker {d}"
            );
            assert_eq!(
                km.lookup(KeyContext::Global, &chord(&format!("{d}"))),
                Some(&ActionId(format!("marker.goto.{d}"))),
                "{d} should return to marker {d}"
            );
        }
        assert_eq!(
            km.lookup(KeyContext::Global, &chord("Shift+M")),
            Some(&"marker.add".into())
        );
        assert_eq!(
            km.lookup(KeyContext::Timeline, &chord("M")),
            Some(&"reveal.masks".into())
        );
    }

    /// **A keymap stored by an older build must not hide a new action**
    /// (K-302). This is the bug the owner hit: `Ctrl+C` did nothing in their
    /// app while every test passed, because their saved keymap — written before
    /// `edit.copy` existed — replaced the whole map on start, and an action
    /// that is not in the file had no chord at all.
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
    }

    /// The After Effects preset re-points the default's deviations — J/K to
    /// keyframe navigation with the shuttle gone, the camera/razor letter swap
    /// — and leaves the chords the two programs already agree on alone.
    #[test]
    fn the_after_effects_preset_repoints_the_ae_habits() {
        let km = after_effects_preset();
        // J/K are AE's keyframe navigation, app-wide; the shuttle is gone.
        assert_eq!(
            km.lookup(KeyContext::Timeline, &chord("J")),
            Some(&"keyframe.prev".into())
        );
        assert_eq!(
            km.lookup(KeyContext::Viewer, &chord("K")),
            Some(&"keyframe.next".into())
        );
        assert!(!km
            .bindings
            .iter()
            .any(|b| b.action.0.starts_with("playback.shuttle")));
        // `,` / `.` still work as the second way in.
        assert_eq!(
            km.lookup(KeyContext::Global, &chord(",")),
            Some(&"keyframe.prev".into())
        );
        // The camera/razor swap: AE's `C` cycles cameras.
        assert_eq!(
            km.lookup(KeyContext::Tools, &chord("C")),
            Some(&"tool.camera".into())
        );
        assert_eq!(
            km.lookup(KeyContext::Tools, &chord("Shift+C")),
            Some(&"tool.razor".into())
        );
        // `L` still reveals a layer's Audio in the Timeline — AE's `L` too —
        // and no longer shadows anything, since the shuttle is gone.
        assert_eq!(
            km.lookup(KeyContext::Timeline, &chord("L")),
            Some(&"reveal.audio".into())
        );
        assert!(km.shadows().is_empty());
        // Chords the two programs agree on are untouched.
        for (context, chord_text, action) in [
            (KeyContext::Global, "Space", "playback.toggle"),
            (KeyContext::Tools, "V", "tool.select"),
            (KeyContext::Tools, "Y", "tool.anchor"),
            (KeyContext::Tools, "G", "tool.pen"),
            (KeyContext::Timeline, "Mod+D", "layer.duplicate"),
            (KeyContext::Timeline, "[", "layer.move.in"),
            (KeyContext::Timeline, "Alt+]", "layer.trim.out"),
            (KeyContext::Global, "Home", "playback.comp.start"),
            (KeyContext::Global, "Mod+Alt+T", "layer.retime.enable"),
            (KeyContext::Graph, "F9", "graph.ease"),
        ] {
            assert_eq!(
                km.lookup(context, &chord(chord_text)),
                Some(&action.into()),
                "{chord_text} should still run {action}"
            );
        }
    }

    #[test]
    fn a_keymap_serialises_to_a_shareable_file_and_back() {
        let km = default_keymap();
        let json = serde_json::to_string_pretty(&km).unwrap();
        // Chords serialise as their readable string form.
        assert!(json.contains("\"Shift+F3\""));
        let back: Keymap = serde_json::from_str(&json).unwrap();
        assert_eq!(back, km);
    }

    /// Settings → Keymap shows a description, never a raw id, so every action
    /// the app ships needs one. This is the test that fails when an action is
    /// added and its row forgotten — the table would silently start showing
    /// `layer.something.new` to a user who has never read the source.
    #[test]
    fn every_shipped_action_has_a_written_description() {
        for km in [default_keymap(), after_effects_preset()] {
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

    /// Taking a chord another action already holds is never refused — refusing
    /// would make swapping two actions' keys impossible, since the swap needs a
    /// moment where one chord is claimed twice. What happens to the old owner
    /// depends on whether it could still fire, and both halves are pinned here
    /// because the settings table has to show the difference.
    #[test]
    fn taking_a_held_chord_evicts_within_a_context_and_conflicts_across_them() {
        // Same context: the old owner loses the key and its row goes blank.
        // Visible in the table, so nothing is lost silently.
        let mut km = Keymap::default();
        km.bind(KeyContext::Timeline, chord("D"), "a".into());
        km.bind(KeyContext::Timeline, chord("F"), "b".into());
        km.rebind_action(KeyContext::Timeline, &"b".into(), chord("D"));
        assert_eq!(
            km.binding_for(KeyContext::Timeline, &"a".into()),
            None,
            "the previous owner shows as unbound rather than answering invisibly"
        );
        assert_eq!(
            km.lookup(KeyContext::Timeline, &chord("D")),
            Some(&ActionId::from("b"))
        );
        assert!(
            km.conflicts().is_empty(),
            "one owner, so nothing to resolve"
        );

        // Across contexts: a Global binding stays live everywhere else, so both
        // survive — and the panel taking the chord over is reported as a shadow
        // rather than a clash (K-281), because which action fires is never in
        // doubt.
        let mut km = Keymap::default();
        km.bind(KeyContext::Global, chord("D"), "global.thing".into());
        km.bind(KeyContext::Timeline, chord("F"), "timeline.thing".into());
        km.rebind_action(KeyContext::Timeline, &"timeline.thing".into(), chord("D"));
        assert!(km.conflicts().is_empty(), "nothing ambiguous to resolve");
        let shadows = km.shadows();
        assert_eq!(shadows.len(), 1, "the takeover is still said out loud");
        assert_eq!(shadows[0].chord, chord("D"));
        assert_eq!(
            km.lookup(KeyContext::Timeline, &chord("D")),
            Some(&ActionId::from("timeline.thing")),
            "and the focused panel still gets first refusal meanwhile"
        );
        assert_eq!(
            km.lookup(KeyContext::Viewer, &chord("D")),
            Some(&ActionId::from("global.thing")),
            "while the app-wide meaning is untouched everywhere else"
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

    /// Every context the settings page can head a table with is one the default
    /// keymap actually uses, and vice versa — a heading with nothing under it,
    /// or bindings with no heading, are both bugs the page cannot show.
    #[test]
    fn the_context_list_matches_what_the_default_keymap_uses() {
        let km = default_keymap();
        for context in KeyContext::ALL {
            assert!(!context.label().is_empty());
        }
        for b in &km.bindings {
            assert!(
                KeyContext::ALL.contains(&b.context),
                "{:?} is bound but not listed in KeyContext::ALL",
                b.context
            );
        }
    }
}
