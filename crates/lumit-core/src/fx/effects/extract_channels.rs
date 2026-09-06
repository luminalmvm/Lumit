//! Extract channels (docs/08 §3.97): the layer's picture read out of channels
//! its file holds but its RGB does not — After Effects' EXtractoR.
//!
//! **In plain terms.** A render leaves far more in an EXR than a picture.
//! Beside the red, green and blue there is usually a `Z` saying how far away
//! every pixel is, often normals, an object id, a light group per lamp. Open
//! the file normally and all of that is dropped on the floor, because a picture
//! has four channels and the file has thirty.
//!
//! Drop this on the layer and its four rows say which of the file's channels
//! become red, green, blue and alpha. Point red at `Z` and the layer becomes
//! its own depth pass, ready to drive a Depth of field. Point all three at a
//! light group and the layer becomes that lamp's contribution alone.
//!
//! **The rows are the file's, not the effect's.** Every other effect in the
//! catalogue has the same controls wherever it is dropped; this one cannot,
//! because what it offers is a fact about the file underneath it. So the four
//! dropdowns are *derived* rows (§1.5, the carriage the Custom shader opened),
//! built from a channel list read off the file and kept on the instance under
//! `extra.extract_channels`. That list rides through save, load, undo,
//! copy/paste and the `.lumfx` preset with no format work at all.
//!
//! **Why the list is stored rather than read every time.** A dropdown's value
//! is the index of the option chosen, so the options have to hold still or the
//! stored choice quietly comes to mean a different channel. Reading the file on
//! every build would let a re-render upstream renumber somebody's project
//! between one frame and the next. The list is therefore taken once, when the
//! effect is added, and re-taken only when the user presses **Reload channels**
//! — which is exactly the moment they are expecting the shape to have changed.
//!
//! **It does not run on the graphics card, and has no kernel at all.** What it
//! changes is which numbers get decoded, not what happens to them afterwards,
//! so the selection travels to the decode worker (docs/impl/media-io.md §5b)
//! and the effect itself is a passthrough. That is the same shape the Flow
//! retiming parameters have: an effect whose settings are read before the
//! picture exists.

use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamKind, ParamSchema, Unit, CHOICE_UNGROUPED,
};
use crate::model::EffectInstance;
use lumit_fx_macros::Effect;

/// The `extra` key the channel list lives under.
pub const EXTRA_KEY: &str = "extract_channels";

/// What a slot set to nothing reads as, and the first option of every dropdown.
pub const NONE_OPTION: &str = "None";

/// The four derived rows' ids, in red, green, blue, alpha order.
pub const SLOT_IDS: [&str; 4] = [
    "derived.red_from",
    "derived.green_from",
    "derived.blue_from",
    "derived.alpha_from",
];

/// Extract channels' declared controls — the two that are the same on every
/// file. The four that are not are derived; see the module note.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "extract_channels",
    label = "Extract channels",
    version = 1,
    category = Utility,
    // Nothing is computed here: the decode reads different numbers and the
    // effect passes them through.
    cost = Trivial,
    roi = Exact,
    premultiplied = false,
)]
pub struct ExtractChannels {
    /// Read the file's channel list again and rebuild the four dropdowns.
    ///
    /// Needed when the render changed shape under the project — a pass added,
    /// a light group renamed. It is a button rather than automatic for the
    /// reason in the module note: silently renumbering somebody's choices is
    /// worse than making them ask.
    #[action(label = "Reload channels")]
    pub reload: (),

    /// Straight through, ignoring the four rows entirely.
    ///
    /// The one honest way to compare what was extracted against the picture the
    /// file opens as, without taking the effect off and losing the selection.
    #[toggle(label = "Bypass", default = false)]
    pub bypass: bool,
}

/// The channel names this instance was built against, in the file's own order.
///
/// Empty for an instance whose layer is not an EXR, one whose file could not be
/// read, and one carried in from a preset that never saw this file — all of
/// which show the same thing in the panel: four dropdowns offering None alone,
/// and a Reload channels button that will fill them in.
#[must_use]
pub fn stored_channels(inst: &EffectInstance) -> Vec<String> {
    inst.extra
        .get(EXTRA_KEY)
        .and_then(|v| v.get("channels"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Build the `extra` block holding `channels`, for whoever has just read them
/// off a file — the add-effect path and the Reload channels action.
#[must_use]
pub fn channels_extra(channels: &[String]) -> serde_json::Value {
    serde_json::json!({ "channels": channels })
}

/// Which channel each of red, green, blue and alpha is to be read from, by
/// name, or `None` for a slot left at None.
///
/// This is what the decode worker is handed. `None` for the whole thing means
/// the effect is doing nothing an ordinary decode does not already do — it is
/// bypassed, or every slot is None — and the layer decodes as the picture it
/// always was, which is also what a layer with no such effect gets.
#[must_use]
pub fn selection(inst: &EffectInstance) -> Option<[Option<String>; 4]> {
    let bypassed = matches!(
        inst.param("bypass"),
        Some(crate::model::EffectValue::Bool(true))
    );
    if !inst.enabled || bypassed {
        return None;
    }
    let names = stored_channels(inst);
    let mut out: [Option<String>; 4] = [None, None, None, None];
    let mut any = false;
    for (slot, id) in SLOT_IDS.iter().enumerate() {
        // Option 0 is None; every option after it is `names[i - 1]`.
        let Some(crate::model::EffectValue::Choice(chosen)) = inst.param(id) else {
            continue;
        };
        if *chosen == 0 {
            continue;
        }
        let Some(name) = names.get(*chosen as usize - 1) else {
            continue;
        };
        out[slot] = Some(name.clone());
        any = true;
    }
    any.then_some(out)
}

/// Extract channels' behaviour.
pub struct ExtractChannelsDef;

impl EffectDef for ExtractChannelsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ExtractChannels as EffectMetadata>::SCHEMA
    }

    /// The four dropdowns this instance's own file earns, each offering None
    /// and then every channel the file holds.
    ///
    /// Interned per distinct channel list, exactly as the Custom shader interns
    /// per distinct source and for the same reason: these are `&'static`, and a
    /// project with forty of these effects on the same render must not leak
    /// forty copies of one list.
    fn derived(&self, inst: &EffectInstance) -> &'static [ParamSchema] {
        rows_for(&stored_channels(inst))
    }

    /// **Not an image operation at all**, which is why it has no GPU pass and
    /// no CPU reference.
    ///
    /// It changes which numbers are decoded, not what happens to them
    /// afterwards: by the time the stack runs, the pixels are already the ones
    /// that were asked for. A kernel here would be an identity pass, and the
    /// two registries would then name a way of copying a texture.
    fn is_image_op(&self) -> bool {
        false
    }
}

/// The derived rows for one channel list, built once per distinct list.
fn rows_for(channels: &[String]) -> &'static [ParamSchema] {
    use std::collections::HashMap;
    use std::sync::{OnceLock, RwLock};

    type Cache = RwLock<HashMap<u64, &'static [ParamSchema]>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Cache::default);

    let key = {
        let mut hasher = blake3::Hasher::new();
        for c in channels {
            hasher.update(c.as_bytes());
            hasher.update(&[0]);
        }
        let bytes = hasher.finalize();
        u64::from_le_bytes(<[u8; 8]>::try_from(&bytes.as_bytes()[..8]).unwrap_or([0; 8]))
    };
    if let Ok(map) = cache.read() {
        if let Some(hit) = map.get(&key) {
            return hit;
        }
    }

    // None first, so a fresh instance changes nothing until somebody chooses.
    let mut options: Vec<&'static str> = vec![NONE_OPTION];
    options.extend(
        channels
            .iter()
            .map(|c| &*Box::leak(c.clone().into_boxed_str())),
    );
    let options: &'static [&'static str] = Box::leak(options.into_boxed_slice());

    let labels = ["Red from", "Green from", "Blue from", "Alpha from"];
    let rows: Vec<ParamSchema> = SLOT_IDS
        .iter()
        .zip(labels)
        .map(|(id, label)| ParamSchema {
            id,
            label,
            kind: ParamKind::Choice {
                options,
                default: 0,
                dividers_after: CHOICE_UNGROUPED,
            },
            unit: Unit::Raw,
        })
        .collect();
    let rows: &'static [ParamSchema] = Box::leak(rows.into_boxed_slice());

    if let Ok(mut map) = cache.write() {
        map.insert(key, rows);
    }
    rows
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn instance(channels: &[&str]) -> EffectInstance {
        let names: Vec<String> = channels.iter().map(|c| (*c).to_owned()).collect();
        let mut inst = crate::fx::builtins::instantiate("extract_channels").unwrap();
        inst.extra
            .insert(EXTRA_KEY.to_owned(), channels_extra(&names));
        inst
    }

    /// Pick option `n` for slot `slot` on this instance.
    fn choose(inst: &mut EffectInstance, slot: usize, n: u32) {
        let id = SLOT_IDS[slot];
        inst.params.retain(|p| p.id != id);
        inst.params.push(crate::model::EffectParam {
            id: id.to_owned(),
            value: crate::model::EffectValue::Choice(n),
            extra: serde_json::Map::new(),
        });
    }

    /// The dropdowns are the file's channels with None in front, so a fresh
    /// instance sits on None and changes nothing.
    #[test]
    fn the_rows_offer_none_then_every_channel_of_the_file() {
        let rows = rows_for(&["R".into(), "Z".into()]);
        assert_eq!(rows.len(), 4);
        let ParamKind::Choice {
            options, default, ..
        } = rows[0].kind
        else {
            panic!("a slot is a dropdown");
        };
        assert_eq!(options, [NONE_OPTION, "R", "Z"]);
        assert_eq!(default, 0);
    }

    /// One list, one set of rows — a project with many of these on the same
    /// render must not leak a copy of the list per instance.
    #[test]
    fn one_channel_list_is_interned_once() {
        let a = rows_for(&["R".into(), "Z".into()]);
        let b = rows_for(&["R".into(), "Z".into()]);
        assert!(std::ptr::eq(a, b));
        let c = rows_for(&["R".into(), "N.X".into()]);
        assert!(!std::ptr::eq(a, c));
    }

    /// A fresh instance selects nothing, so the layer decodes as it always did.
    #[test]
    fn every_slot_at_none_is_no_selection_at_all() {
        assert_eq!(selection(&instance(&["R", "Z"])), None);
    }

    /// The index is read against the stored list, so option 2 of `[None, R, Z]`
    /// is `Z`. An unset slot stays unset rather than defaulting to a channel.
    #[test]
    fn a_chosen_slot_names_the_channel_it_points_at() {
        let mut inst = instance(&["R", "Z"]);
        choose(&mut inst, 0, 2);
        assert_eq!(selection(&inst), Some([Some("Z".into()), None, None, None]));
    }

    /// Bypass is the honest comparison: the selection stops being applied
    /// without the choices being lost.
    #[test]
    fn bypass_stops_the_selection_without_forgetting_it() {
        let mut inst = instance(&["R", "Z"]);
        choose(&mut inst, 0, 2);
        for p in &mut inst.params {
            if p.id == "bypass" {
                p.value = crate::model::EffectValue::Bool(true);
            }
        }
        assert_eq!(selection(&inst), None);
    }

    /// A disabled effect is not a quietly-still-applied one.
    #[test]
    fn a_disabled_effect_selects_nothing() {
        let mut inst = instance(&["R", "Z"]);
        choose(&mut inst, 0, 2);
        inst.enabled = false;
        assert_eq!(selection(&inst), None);
    }

    /// An index past the end of the list — a preset from a project whose render
    /// had more passes — reads as unset rather than as a fault or a wrong
    /// channel.
    #[test]
    fn an_index_past_the_list_reads_as_unset() {
        let mut inst = instance(&["R"]);
        choose(&mut inst, 0, 9);
        assert_eq!(selection(&inst), None);
    }
}
