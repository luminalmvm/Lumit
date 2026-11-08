//! Masks on the clipboard: copied off one layer, pasted onto any, and
//! duplicated in place.

use flutter_rust_bridge::frb;
use lumit_core::mask::Mask;
use uuid::Uuid;

use crate::api::{layer::LayerReference, BridgeError};

impl LayerReference {
    /// The masks named by `masks`, in stack order, as a document for
    /// [`Self::paste_masks`]. Their keys keep the layer's own times.
    #[frb(sync)]
    pub fn copy_masks(&self, masks: Vec<Uuid>) -> Result<String, BridgeError> {
        let layer = self.leaving()?;
        let taken: Vec<&Mask> = (layer.masks.iter())
            .filter(|m| masks.contains(&m.id))
            .collect();
        if taken.is_empty() {
            return Err(BridgeError::NoSuchMask);
        }
        serde_json::to_string(&serde_json::json!({
            "format": 1,
            "kind": "masks",
            "masks": taken,
        }))
        .map_err(|_| BridgeError::NoSuchMask)
    }

    /// Put the masks of a [`Self::copy_masks`] document on top of this layer's
    /// stack, each with a fresh id and a name of its own, as one undo step.
    /// Answers their ids.
    #[frb(sync)]
    pub fn paste_masks(&self, text: String) -> Result<Vec<Uuid>, BridgeError> {
        #[derive(serde::Deserialize)]
        struct Copied {
            kind: String,
            masks: Vec<Mask>,
        }
        let copied: Copied = serde_json::from_str(&text).map_err(|_| BridgeError::NoSuchMask)?;
        if copied.kind != "masks" || copied.masks.is_empty() {
            return Err(BridgeError::NoSuchMask);
        }
        let mut masks = self.item()?.masks;
        let copies = fresh(&masks, copied.masks);
        let made = copies.iter().map(|m| m.id).collect();
        masks.extend(copies);
        self.commit_layer_masks(masks)?;
        Ok(made)
    }

    /// Copy each of `masks` in place, each copy right after its original, as
    /// one undo step. Answers the copies' ids, in stack order.
    #[frb(sync)]
    pub fn duplicate_masks(&self, masks: Vec<Uuid>) -> Result<Vec<Uuid>, BridgeError> {
        let held = self.item()?.masks;
        let taken: Vec<Mask> = (held.iter())
            .filter(|m| masks.contains(&m.id))
            .cloned()
            .collect();
        if taken.is_empty() {
            return Err(BridgeError::NoSuchMask);
        }
        let copies = fresh(&held, taken);
        let made = copies.iter().map(|m| m.id).collect();
        let mut copies = copies.into_iter();
        let mut stack = Vec::with_capacity(held.len() * 2);
        for mask in held {
            let copied = masks.contains(&mask.id);
            stack.push(mask);
            if copied {
                stack.extend(copies.next());
            }
        }
        self.commit_layer_masks(stack)?;
        Ok(made)
    }

    #[frb(ignore)]
    fn commit_layer_masks(&self, masks: Vec<Mask>) -> Result<(), BridgeError> {
        self.commit(lumit_core::ops::Op::SetLayerMasks {
            comp: self.comp_id,
            layer: self.layer_id,
            masks,
        })
    }
}

/// `masks` with fresh ids and names none of `stack` has, so a mask asked for
/// by name finds the one that was meant.
fn fresh(stack: &[Mask], masks: Vec<Mask>) -> Vec<Mask> {
    let mut taken: Vec<String> = stack.iter().map(|m| m.name.to_lowercase()).collect();
    masks
        .into_iter()
        .map(|mut mask| {
            mask.id = Uuid::now_v7();
            if let Some(free) = lumit_core::fx::names::free_name(&mask.name, &taken) {
                mask.name = free;
            }
            taken.push(mask.name.to_lowercase());
            mask
        })
        .collect()
}
