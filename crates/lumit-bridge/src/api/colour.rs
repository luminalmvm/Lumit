//! Colour management across the seam (K-489, K-490, docs/impl/ocio.md §6.1).
//!
//! **In plain terms.** An OCIO config is a folder of colour recipes that the
//! whole industry shares: it names the colour spaces footage arrives in, and
//! the displays and views a picture can be shown through. The project stores
//! one path to one such file. Everything else — is it readable, what names does
//! it hold, can it deliver this export — is worked out from that file and never
//! stored, so this module is a **read of derived state** plus two ordinary
//! document edits.
//!
//! Three things about the shape here are deliberate.
//!
//! **The names are not translated.** `spaces` and `displays` are the config's
//! own words — someone else's file, on this machine — so they cross verbatim
//! and are shown as they arrived, exactly as a codec name is (K-303).
//!
//! **The refusal is not a sentence.** Every reason a config can be unusable
//! names something in the middle of it ("this config needs
//! `FixedFunctionTransform`", "the look-up table `shot.spi3d` was not found"),
//! so a whole-text lookup could never translate one. It crosses as a stable id
//! plus its facts by name, exactly as the After Effects import report's rows do
//! (K-005, docs/17); `problem_english` is the engine's own words, for a
//! frontend with no sentence for the id.
//!
//! **The colour state is synced from the document on every read**, which is one
//! file read and a hash comparison when nothing has changed, and a reparse when
//! the config on disk has been edited. That is why a config edited underneath a
//! running Lumit is picked up without anyone having to press anything — and why
//! none of these calls belongs in a widget's `build()` (K-183). The frontend
//! fetches on a document change and holds the answer.

use std::path::PathBuf;

use flutter_rust_bridge::frb;
use lumit_core::Op;
use lumit_render::colour::ColourState;

use crate::api::{
    footage::FootageReference, project::ProjectReference, state::LumitBridgeState, BridgeError,
};

/// One blank in a refusal's sentence, by name: `name` → `FixedFunctionTransform`,
/// `in_space` → `fancy`. Named rather than positional so a translation may put
/// them in whatever order its language wants.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeColourArg {
    pub name: String,
    pub value: String,
}

/// One display the config declares, and the views it can be shown through —
/// the two levels the Viewer's picker draws as a section and its rows.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeColourDisplay {
    pub name: String,
    pub views: Vec<String>,
}

/// Everything the frontend needs to know about the project's colour config, in
/// one read (K-451's one-call-per-structure rule).
///
/// The four states it describes, and how to tell them apart:
///
/// | state | `path` | `loaded` | `problem` |
/// |---|---|---|---|
/// | no config named — the built-in family, today's behaviour | empty | `false` | empty |
/// | loaded and usable | the path | `true` | empty |
/// | named but missing, unreadable or refused | the path | `false` | the id |
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BridgeColourSummary {
    /// The path the project names, as it would be shown — the relative one a
    /// saved `.lum` actually carries (K-173). Empty when no config is named.
    pub path: String,
    /// A config is named, was read, and can do colour.
    pub loaded: bool,
    /// The stable id of the refusal sentence to write, or empty when there is
    /// nothing wrong. See the module note.
    pub problem: String,
    pub problem_args: Vec<BridgeColourArg>,
    /// The engine's own English, for a frontend with no sentence for `problem`.
    pub problem_english: String,
    /// The config's active colour space names, in its own order. **Never
    /// translated.** Empty unless `loaded`.
    pub spaces: Vec<String>,
    /// Each display with its views, in the config's own order. Never
    /// translated either, and empty unless `loaded`.
    pub displays: Vec<BridgeColourDisplay>,
}

/// The project's colour state, brought into line with the document first.
///
/// Cheap when nothing has changed — one file read and a hash comparison — so
/// every read syncs rather than trusting a cache the frontend would have to
/// remember to invalidate. The lock is this state's own and is never held
/// across a GPU submit or an await (docs/14 §3): the closure reads names and
/// asks for baked tables, and does nothing else.
#[frb(ignore)]
pub(crate) fn with_colour<T>(
    state: &LumitBridgeState,
    read: impl FnOnce(&ColourState) -> T,
) -> Result<T, BridgeError> {
    let document = state.store.snapshot();
    let mut colour = state.colour.lock().map_err(|_| BridgeError::ReadFailed)?;
    colour.sync(&document);
    Ok(read(&colour))
}

impl ProjectReference {
    /// What the project's colour config is, and every name it puts in a picker.
    ///
    /// **Not for a rebuild path.** It reads the config file to see whether it
    /// has changed; the frontend asks on a document change and holds the
    /// answer, which is what the bridge-call budget test enforces (K-183).
    #[frb(sync)]
    pub fn colour_summary(&self) -> Result<BridgeColourSummary, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        let path = state
            .store
            .snapshot()
            .colour
            .config
            .as_ref()
            .map(|media| media.display_path().to_owned())
            .unwrap_or_default();

        with_colour(&state, |colour| {
            let Some(loaded) = colour.loaded() else {
                return BridgeColourSummary {
                    path,
                    ..BridgeColourSummary::default()
                };
            };
            let (spaces, displays) = loaded.vocabulary();
            BridgeColourSummary {
                path,
                loaded: loaded.usable(),
                problem: loaded
                    .refusal
                    .as_ref()
                    .map(|r| r.key.clone())
                    .unwrap_or_default(),
                problem_args: loaded
                    .refusal
                    .as_ref()
                    .map(|r| {
                        r.args
                            .iter()
                            .map(|(name, value)| BridgeColourArg {
                                name: name.clone(),
                                value: value.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                problem_english: loaded.problem.clone().unwrap_or_default(),
                spaces,
                displays: displays
                    .into_iter()
                    .map(|(name, views)| BridgeColourDisplay { name, views })
                    .collect(),
            }
        })
    }

    /// Point the project at an OCIO config, or at none (`None`, which is the
    /// built-in colour family and the behaviour of every project written before
    /// this existed).
    ///
    /// An ordinary op, so it is undoable, journalled, and travels in the `.lum`
    /// — colour management changes what a comp looks like, so it is the
    /// project's property and not the machine's (K-490). The path is stored as
    /// a `MediaRef` for the same reason footage is: the relative path is what a
    /// saved project carries, the absolute one is never serialised (K-173), and
    /// a config that moved relinks by fingerprint like any other file.
    #[frb(sync)]
    pub fn set_colour_config(&self, path: Option<String>) -> Result<(), BridgeError> {
        let path = path
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty());
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        let config = path.map(|path| {
            Box::new(crate::api::footage::media_ref_at(
                state.path.as_deref().and_then(|p| p.parent()),
                &path,
            ))
        });
        state
            .store
            .commit(Op::SetColourConfig { config })
            .map(|_| ())
            .map_err(BridgeError::OpError)
    }

    /// Whether a named colour space can actually be delivered right now — what
    /// the export dialogue's colour dropdown enables a row on (K-485's
    /// disabled-not-hidden rule), and the question the export itself refuses on.
    ///
    /// `false` for a config that is missing or refused and for a name it does
    /// not have, which is the half of K-490's asymmetry that says no: a preview
    /// degrades to the built-in transform, a delivery does not. The built-in
    /// space names are not asked about here — they are always deliverable, and
    /// `BridgeFormatCaps::colour_spaces` is what decides whether the *format*
    /// can state one.
    #[frb(sync)]
    pub fn can_deliver_colour_space(&self, name: String) -> Result<bool, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        with_colour(&state, |colour| colour.can_deliver(&name))
    }
}

impl FootageReference {
    /// The colour space this footage says it arrives in, by the config's own
    /// name, or `None` for the built-in interpretation defaults (video is
    /// Rec.709, stills are sRGB, the container's own metadata wins).
    ///
    /// A name is kept even when the config that defined it is missing: it is
    /// the user's statement about the file, and dropping it because a path
    /// moved would be a silent edit of their project.
    #[frb(sync)]
    pub fn colour_space(&self) -> Result<Option<String>, BridgeError> {
        let state = self.project()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        match state.store.snapshot().item(self.id) {
            Some(lumit_core::model::ProjectItem::Footage(f)) => Ok(f.colour_space.clone()),
            _ => Err(BridgeError::InvalidItem),
        }
    }

    /// Say what colour space this footage arrives in, or clear it back to the
    /// built-in defaults. One gesture, one op, one undo step.
    #[frb(sync)]
    pub fn set_colour_space(&self, space: Option<String>) -> Result<(), BridgeError> {
        let state = self.project()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        state
            .store
            .commit(Op::SetFootageColourSpace {
                id: self.id,
                space: space.filter(|s| !s.is_empty()),
            })
            .map(|_| ())
            .map_err(BridgeError::OpError)
    }
}
