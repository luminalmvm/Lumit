//! The remappable keymap: the table Settings → Keymap draws, and the lookup
//! every keypress goes through (docs/07-UI-SPEC.md §15, K-199).
//!
//! # In plain terms
//!
//! A *chord* is a key with its modifiers — `Space`, `Mod+D`, `Shift+F3`. A
//! *context* is where you are: the whole app, or one focused panel. A *binding*
//! ties a chord in a context to an *action*, named by a stable string like
//! `"playback.toggle"`. Pressing a key asks [`keymap_lookup`] which action that
//! chord means where you are, and the frontend runs it.
//!
//! **Why the model lives here and not in Dart.** Everything that has to be
//! *decided* — what a chord means, whether the focused panel outranks the
//! app-wide binding, whether two bindings clash, what the shareable file says —
//! is a rule, and rules live in the engine (K-181). The frontend turns a real
//! key event into chord text, draws the table, and forwards the edits. It holds
//! no opinion about any of it. `lumit-keymap` is the crate that knows; this
//! module is its window.
//!
//! **Where it is kept.** In memory here for the session, behind one lock. The
//! *file* is the frontend's, because a keymap is machine-local settings and the
//! frontend already owns that file (the workspace JSON): [`keymap_to_json`]
//! hands out the whole map as text and [`keymap_from_json`] takes it back, and
//! Dart never looks inside the string it stored. Same split as the shareable
//! export a user mails to a friend — one format, two reasons to write it.

use flutter_rust_bridge::frb;
use lumit_keymap::{ActionId, Chord, KeyContext, Keymap};
use std::sync::{Mutex, OnceLock};

use crate::api::BridgeError;

/// Where a binding is live. Mirrors `lumit_keymap::KeyContext` across the seam.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeKeyContext {
    Global,
    Tools,
    Project,
    Timeline,
    Viewer,
    Graph,
    Panels,
    Effects,
}

impl From<BridgeKeyContext> for KeyContext {
    fn from(c: BridgeKeyContext) -> Self {
        match c {
            BridgeKeyContext::Global => KeyContext::Global,
            BridgeKeyContext::Tools => KeyContext::Tools,
            BridgeKeyContext::Project => KeyContext::Project,
            BridgeKeyContext::Timeline => KeyContext::Timeline,
            BridgeKeyContext::Viewer => KeyContext::Viewer,
            BridgeKeyContext::Graph => KeyContext::Graph,
            BridgeKeyContext::Panels => KeyContext::Panels,
            BridgeKeyContext::Effects => KeyContext::Effects,
        }
    }
}

impl From<KeyContext> for BridgeKeyContext {
    fn from(c: KeyContext) -> Self {
        match c {
            KeyContext::Global => BridgeKeyContext::Global,
            KeyContext::Tools => BridgeKeyContext::Tools,
            KeyContext::Project => BridgeKeyContext::Project,
            KeyContext::Timeline => BridgeKeyContext::Timeline,
            KeyContext::Viewer => BridgeKeyContext::Viewer,
            KeyContext::Graph => BridgeKeyContext::Graph,
            KeyContext::Panels => BridgeKeyContext::Panels,
            KeyContext::Effects => BridgeKeyContext::Effects,
        }
    }
}

/// One row of the Settings → Keymap table: what the action is called, what it
/// is called internally, and the chord that runs it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeKeyBinding {
    pub context: BridgeKeyContext,
    /// The stable id, e.g. `"playback.toggle"` — what the frontend switches on.
    pub action: String,
    /// What the table shows in its left-hand column, e.g. "Play or pause".
    pub description: String,
    /// The chord in its canonical text form, e.g. `"Mod+Shift+P"`. Empty when
    /// the action is currently unbound, which is a state the table shows
    /// rather than hides. One chord per row — K-200 settled that no shipped
    /// action carries two, so a list here would be structure with nothing to
    /// hold.
    pub chord: String,
}

/// One context's worth of rows, with the heading to put above them — the shape
/// the settings page draws top to bottom.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeKeymapGroup {
    pub context: BridgeKeyContext,
    /// The heading, e.g. "Timeline" or "Anywhere".
    pub label: String,
    pub bindings: Vec<BridgeKeyBinding>,
}

/// One chord that could fire more than one action where both are live — what
/// the page warns about, and what the user resolves by rebinding one of them.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeKeyConflict {
    pub chord: String,
    /// The competing actions, already described for display.
    pub actions: Vec<String>,
}

/// One chord a panel takes over from an app-wide binding (K-281).
///
/// **Not a clash.** The focused panel gets first refusal and the app-wide
/// binding is the fallback, so the chord runs exactly one action and which one
/// is never in doubt. It is reported because the app-wide meaning does stop
/// working in that one panel, and somebody reading their keymap should be able
/// to see that rather than discover it by pressing the key.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeKeyShadow {
    pub chord: String,
    /// Where the takeover applies, e.g. "Timeline".
    pub context: String,
    /// What the chord does there, described for display.
    pub action: String,
    /// What it does everywhere else.
    pub shadowed: String,
}

/// Which shipped keymap to load wholesale.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeKeymapPreset {
    /// Lumit's own defaults (docs/07 §15).
    Lumit,
    /// The After Effects muscle-memory alternate.
    AfterEffects,
}

/// The session's keymap. One window means one keymap, and every call here is a
/// handful of string comparisons, so a plain mutex is the whole story — it is
/// never held across a render, an await or the document lock.
static KEYMAP: OnceLock<Mutex<Keymap>> = OnceLock::new();

/// Run `f` against the session keymap, starting from the shipped default the
/// first time. A poisoned lock is recovered rather than propagated: a keymap is
/// settings, and losing every shortcut because some other call panicked would
/// be a far worse outcome than carrying on with the map as it stands.
#[frb(ignore)]
fn with_keymap<R>(f: impl FnOnce(&mut Keymap) -> R) -> R {
    let mutex = KEYMAP.get_or_init(|| Mutex::new(lumit_keymap::default_keymap()));
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
    f(&mut guard)
}

#[frb(ignore)]
fn row(km: &Keymap, context: KeyContext, action: &ActionId) -> BridgeKeyBinding {
    BridgeKeyBinding {
        context: context.into(),
        action: action.0.clone(),
        description: action.description(),
        chord: km
            .binding_for(context, action)
            .map(ToString::to_string)
            .unwrap_or_default(),
    }
}

/// Every binding, grouped by context in the order the page lists them — the
/// whole table in one call.
///
/// Rebuilt from the map each time rather than cached: it is a few hundred short
/// strings, it is read once when the page opens and once per edit, and a cache
/// here would be one more thing that can disagree with the truth.
#[frb(sync)]
#[must_use]
pub fn keymap_groups() -> Vec<BridgeKeymapGroup> {
    with_keymap(|km| {
        KeyContext::ALL
            .iter()
            .map(|context| {
                // One row per action in this context, in the map's own order,
                // which is the order docs/07 §15 lists them.
                let mut actions: Vec<ActionId> = Vec::new();
                for b in &km.bindings {
                    if b.context == *context && !actions.contains(&b.action) {
                        actions.push(b.action.clone());
                    }
                }
                BridgeKeymapGroup {
                    context: (*context).into(),
                    label: context.label().to_string(),
                    bindings: actions.iter().map(|a| row(km, *context, a)).collect(),
                }
            })
            .filter(|g| !g.bindings.is_empty())
            .collect()
    })
}

/// The rows matching `query` across every context — the search box above the
/// table. An empty query matches everything.
#[frb(sync)]
#[must_use]
pub fn keymap_search(query: String) -> Vec<BridgeKeyBinding> {
    with_keymap(|km| {
        km.search(&query)
            .into_iter()
            .map(|b| BridgeKeyBinding {
                context: b.context.into(),
                action: b.action.0.clone(),
                description: b.action.description(),
                chord: b.chord.to_string(),
            })
            .collect()
    })
}

/// Every chord that could fire two actions at once, described for display.
/// Empty when the keymap is unambiguous, which is the shipped state.
#[frb(sync)]
#[must_use]
pub fn keymap_conflicts() -> Vec<BridgeKeyConflict> {
    with_keymap(|km| {
        km.conflicts()
            .into_iter()
            .map(|c| BridgeKeyConflict {
                chord: c.chord.to_string(),
                actions: c.actions.iter().map(ActionId::description).collect(),
            })
            .collect()
    })
}

/// Every chord a panel takes over from an app-wide binding, described for
/// display (K-281). Said out loud beside the table rather than flagged as
/// something to fix — the shipped default carries one on purpose (`L`).
#[frb(sync)]
#[must_use]
pub fn keymap_shadows() -> Vec<BridgeKeyShadow> {
    with_keymap(|km| {
        km.shadows()
            .into_iter()
            .map(|s| BridgeKeyShadow {
                chord: s.chord.to_string(),
                context: s.context.label().to_string(),
                action: s.action.description(),
                shadowed: s.shadowed.description(),
            })
            .collect()
    })
}

/// What `chord` does while `context` is focused, or `None` for nothing bound.
///
/// This is the dispatch path: every keypress the frontend sees becomes chord
/// text and comes through here. Sync and lock-light for that reason — it is a
/// linear scan of a few hundred bindings, which is nothing beside the frame it
/// happens in, and it must never be the reason a keypress feels late.
#[frb(sync)]
#[must_use]
pub fn keymap_lookup(context: BridgeKeyContext, chord: String) -> Option<String> {
    let Ok(parsed) = chord.parse::<Chord>() else {
        return None;
    };
    with_keymap(|km| km.lookup(context.into(), &parsed).map(|a| a.0.clone()))
}

/// Point an action at a new chord, and hand back the table as it now stands.
///
/// Rejects only chord text that is not a chord; a chord another action already
/// holds is accepted deliberately, because refusing it would make swapping two
/// actions' keys impossible. Within one context the previous owner is left
/// unbound and its row goes blank; a panel-scoped binding taking an app-wide
/// chord leaves both alive, the panel's winning where it is focused, and
/// [`keymap_shadows`] says so (K-281).
pub fn keymap_rebind(
    context: BridgeKeyContext,
    action: String,
    chord: String,
) -> Result<Vec<BridgeKeymapGroup>, BridgeError> {
    let parsed = chord
        .parse::<Chord>()
        .map_err(|e| BridgeError::InvalidKeyChord(e.to_string()))?;
    with_keymap(|km| km.rebind_action(context.into(), &ActionId(action), parsed));
    Ok(keymap_groups())
}

/// Leave an action with no chord at all, and hand back the table.
pub fn keymap_unbind(context: BridgeKeyContext, action: String) -> Vec<BridgeKeymapGroup> {
    let action = ActionId(action);
    let context: KeyContext = context.into();
    with_keymap(|km| km.unbind_action(context, &action));
    keymap_groups()
}

/// Put one action back to the chord the shipped default gives it, and hand
/// back the table. Nothing else in the map is touched — this is the per-row
/// reset, not [`keymap_load_preset`]. Always the Lumit default: "reset" on a
/// settings row means "what the app ships with", whichever preset was loaded
/// since.
pub fn keymap_reset_binding(context: BridgeKeyContext, action: String) -> Vec<BridgeKeymapGroup> {
    let action = ActionId(action);
    let ctx: KeyContext = context.into();
    match lumit_keymap::default_keymap()
        .binding_for(ctx, &action)
        .cloned()
    {
        Some(chord) => with_keymap(|km| km.rebind_action(ctx, &action, chord)),
        // The default does not bind it, so "reset" means unbound.
        None => return keymap_unbind(context, action.0),
    }
    keymap_groups()
}

/// Replace the whole keymap with a shipped preset, and hand back the table.
pub fn keymap_load_preset(preset: BridgeKeymapPreset) -> Vec<BridgeKeymapGroup> {
    let shipped = preset_map(preset);
    with_keymap(|km| *km = shipped);
    keymap_groups()
}

#[frb(ignore)]
fn preset_map(preset: BridgeKeymapPreset) -> Keymap {
    match preset {
        BridgeKeymapPreset::Lumit => lumit_keymap::default_keymap(),
        BridgeKeymapPreset::AfterEffects => lumit_keymap::after_effects_preset(),
    }
}

/// The whole keymap as JSON — what the frontend stores between sessions and
/// what "Export keymap…" writes to a file the user can share. One format for
/// both, so a keymap that survives a restart is the same keymap that travels.
#[frb(sync)]
#[must_use]
pub fn keymap_to_json() -> String {
    with_keymap(|km| serde_json::to_string_pretty(km).unwrap_or_else(|_| String::from("{}")))
}

/// Take a keymap back from JSON — a restored session, or an imported file — and
/// hand back the table.
///
/// Rejects anything that is not a keymap rather than half-applying it, so a
/// corrupt stored blob or somebody else's JSON leaves the current map alone.
///
/// **Laid over the shipped defaults, not swapped for them** (K-302). A file
/// only knows the actions that existed when it was written, and it used to
/// replace the map whole — so every action added since was left with no chord
/// at all for anyone who had ever saved a keymap. That is how `Ctrl+C` came to
/// do nothing in a build whose every test passed. An action the file names
/// keeps the file's chord and an action it deliberately unbound stays unbound;
/// only the ones it never heard of take their default.
pub fn keymap_from_json(json: String) -> Result<Vec<BridgeKeymapGroup>, BridgeError> {
    let parsed: Keymap =
        serde_json::from_str(&json).map_err(|e| BridgeError::InvalidKeymapFile(e.to_string()))?;
    if parsed.bindings.is_empty() {
        return Err(BridgeError::InvalidKeymapFile(
            "the file holds no bindings".to_string(),
        ));
    }
    with_keymap(|km| *km = lumit_keymap::with_new_defaults(parsed));
    Ok(keymap_groups())
}
