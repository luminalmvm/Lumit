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

/// Two contexts overlap when a binding in one can fire while the other is
/// active — i.e. they are equal, or either is `Global` (live everywhere).
fn contexts_overlap(a: KeyContext, b: KeyContext) -> bool {
    a == b || a == KeyContext::Global || b == KeyContext::Global
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
        let mut out: Vec<Conflict> = Vec::new();
        let mut seen_chords: Vec<&Chord> = Vec::new();
        for b in &self.bindings {
            if seen_chords.contains(&&b.chord) {
                continue;
            }
            seen_chords.push(&b.chord);
            // Every binding on this chord, and whether any is Global.
            let same: Vec<&Binding> = self
                .bindings
                .iter()
                .filter(|o| o.chord == b.chord)
                .collect();
            let has_global = same.iter().any(|o| o.context == KeyContext::Global);
            // Collect the distinct actions that can collide. With a Global
            // binding present, all of them overlap; otherwise only bindings
            // sharing a context do.
            let mut actions: Vec<ActionId> = Vec::new();
            for x in &same {
                let clashes = has_global
                    || same
                        .iter()
                        .any(|y| !std::ptr::eq(*x, *y) && contexts_overlap(x.context, y.context));
                if clashes && !actions.contains(&x.action) {
                    actions.push(x.action.clone());
                }
            }
            if actions.len() > 1 {
                out.push(Conflict {
                    chord: b.chord.clone(),
                    actions,
                });
            }
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
    use KeyContext::{Global, Graph, Panels, Timeline, Tools, Viewer};
    let rows = [
        // --- Global: transport, navigation, app-wide commands ---
        row(Global, "Space", "playback.toggle"),
        row(Global, "J", "playback.shuttle.reverse"),
        row(Global, "K", "playback.shuttle.pause"),
        row(Global, "L", "playback.shuttle.forward"),
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
        row(Global, "Mod+Shift+P", "palette.open"),
        row(Global, "Mod+M", "export.queue.add"),
        row(Global, "Mod+K", "comp.settings"),
        row(Global, "Mod+Z", "edit.undo"),
        row(Global, "Mod+Shift+Z", "edit.redo"),
        row(Global, "Mod+S", "file.save"),
        row(Global, "`", "panel.maximise"),
        row(Global, "Shift+F3", "graph.toggle"),
        // --- Tools ---
        row(Tools, "V", "tool.select"),
        row(Tools, "H", "tool.hand"),
        row(Tools, "Z", "tool.zoom"),
        row(Tools, "Y", "tool.anchor"),
        row(Tools, "C", "tool.razor"),
        row(Tools, "Q", "tool.shape"),
        row(Tools, "G", "tool.pen"),
        // --- Timeline: reveals and edits ---
        row(Timeline, "P", "reveal.position"),
        row(Timeline, "S", "reveal.scale"),
        row(Timeline, "R", "reveal.rotation"),
        row(Timeline, "T", "reveal.opacity"),
        row(Timeline, "A", "reveal.anchor"),
        row(Timeline, "E", "reveal.effects"),
        row(Timeline, "M", "reveal.masks"),
        row(Timeline, "U", "reveal.animated"),
        row(Timeline, "Shift+L", "reveal.volume"),
        row(Timeline, "[", "layer.move.in"),
        row(Timeline, "]", "layer.move.out"),
        row(Timeline, "Alt+[", "layer.trim.in"),
        row(Timeline, "Alt+]", "layer.trim.out"),
        row(Timeline, "Mod+Shift+D", "layer.split"),
        row(Timeline, "Mod+D", "layer.duplicate"),
        row(Timeline, "Mod+Shift+C", "layer.precompose"),
        row(Timeline, "Mod+Alt+T", "layer.retime.enable"),
        row(Timeline, "=", "timeline.zoom.in"),
        row(Timeline, "-", "timeline.zoom.out"),
        row(Timeline, "\\", "timeline.zoom.fit"),
        row(Timeline, "Enter", "layer.rename"),
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
    Keymap { bindings }
}

/// The "After Effects" muscle-memory preset (docs/07 §15): starts from the
/// default and re-points the deviating transport/navigation keys to their AE
/// meanings, so J/K/L become keyframe-ish habits again. A representative subset.
#[must_use]
pub fn after_effects_preset() -> Keymap {
    let mut km = default_keymap();
    // AE has no J/K/L shuttle; drop them so they don't clash with AE habits.
    for k in ["J", "K", "L"] {
        if let Ok(chord) = k.parse::<Chord>() {
            km.unbind(KeyContext::Global, &chord);
        }
    }
    // Keyframe nav returns to J/K in AE muscle memory (illustrative).
    if let Ok(chord) = "J".parse::<Chord>() {
        km.bind(KeyContext::Timeline, chord, "keyframe.prev".into());
    }
    if let Ok(chord) = "K".parse::<Chord>() {
        km.bind(KeyContext::Timeline, chord, "keyframe.next".into());
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

        // Global vs a scoped binding on the same chord → conflict (global fires
        // in every context).
        let mut km = Keymap::default();
        km.bind(KeyContext::Global, chord("G"), "global".into());
        km.bind(KeyContext::Timeline, chord("G"), "timeline".into());
        let c = km.conflicts();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].actions.len(), 2);

        // Two *different* scoped contexts on the same chord → NOT a conflict
        // (the chord means different things in different panels).
        let mut km = Keymap::default();
        km.bind(KeyContext::Timeline, chord("H"), "timeline".into());
        km.bind(KeyContext::Viewer, chord("H"), "viewer".into());
        assert!(km.conflicts().is_empty());

        // The same action bound twice is not a conflict.
        let mut km = Keymap::default();
        km.bind(KeyContext::Global, chord("Mod+S"), "file.save".into());
        km.bind(KeyContext::Timeline, chord("Mod+S"), "file.save".into());
        assert!(km.conflicts().is_empty());
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

    #[test]
    fn the_default_keymap_is_conflict_free() {
        assert!(
            default_keymap().conflicts().is_empty(),
            "the shipped default must not ship with clashes"
        );
        assert!(after_effects_preset().conflicts().is_empty());
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
}
