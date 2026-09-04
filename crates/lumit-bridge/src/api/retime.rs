//! How a layer's in-between frames are made.
//!
//! # In plain terms
//!
//! When a layer asks for a moment of its source that falls between two real
//! frames — which is what retiming does constantly, and what any mismatch of
//! rates does anyway — something has to decide which pixels to show. Nearest
//! shows the closer of the two frames, blend crossfades them, flow synthesises
//! a new one. That is all this file is (docs/04-RETIMING.md §10).
//!
//! **Retiming itself is not here.** The map from a layer's own clock to its
//! source's is the layer's `retime` property — an ordinary keyframable property
//! edited in the graph editor, reached through `layer.rs`. This file
//! used to hold a second, rival retime store with its own constant-speed,
//! reverse-gate and enable controls. That store is gone, leaving the one thing
//! that was never part of the map to begin with. §10 is explicit that the
//! policy and the map are orthogonal, and now the code says so too.

use flutter_rust_bridge::frb;
use lumit_core::retime::Interpolation;

use crate::api::{layer::LayerReference, BridgeError};

/// How a source frame is chosen when the map lands between two (docs/04 §10).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRetimeInterp {
    /// Round to the nearest source frame — crisp and deterministic.
    Nearest,
    /// Crossfade the two neighbours.
    Blend,
    /// Optical-flow synthesis: the engine measures how everything
    /// moved between the two frames and paints the one in between.
    Flow,
}

/// A footage layer's Flow group (docs/08 §3.1), flat for the bridge.
///
/// Every field is a picture-changing parameter, so every field is part of the
/// frame's identity — see `feed_interp`. Read and written whole: a group of
/// eight settings edited one at a time would need eight round trips and eight
/// undo steps to do what one does.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeFlowParams {
    /// 0 native, 1 half, 2 quarter — the size flow is *measured* at,
    /// independent of the preview quality tier.
    pub resolution: u32,
    /// 0 low, 1 medium, 2 high, 3 ultra — pyramid depth and refinement effort.
    pub detail: u32,
    /// 0–100. High means fewer tears and a gloopier field.
    pub smoothness: f64,
    /// 0 visible-only, 1 blend.
    pub occlusion: u32,
    /// 0 blend, 1 nearest — what shows where confidence is too low.
    pub fallback: u32,
    /// Bias static, detailed regions toward a plain blend, so a game HUD does
    /// not smear across the frame.
    pub hud_guard: bool,
    /// Force flow on where the engagement gate would decline it.
    pub always: bool,
}

impl BridgeFlowParams {
    fn from_core(p: &lumit_core::retime::FlowParams) -> Self {
        Self {
            resolution: p.resolution.code(),
            detail: p.detail.code(),
            smoothness: p.smoothness,
            occlusion: p.occlusion.code(),
            fallback: p.fallback.code(),
            hud_guard: p.hud_guard,
            always: p.always,
        }
    }

    /// Fold onto an existing set, so the keyframed input rate and any
    /// forward-compatible fields survive an edit of the plain ones.
    fn onto(&self, base: &lumit_core::retime::FlowParams) -> lumit_core::retime::FlowParams {
        use lumit_core::retime::{FlowFallback, FlowResolution, OcclusionMode, VectorDetail};
        let mut out = base.clone();
        // An unknown code keeps what was there rather than snapping to a
        // default: the UI and the engine disagreeing is a bug, not a reason to
        // silently change the user's picture.
        if let Some(v) = FlowResolution::from_code(self.resolution) {
            out.resolution = v;
        }
        if let Some(v) = VectorDetail::from_code(self.detail) {
            out.detail = v;
        }
        if let Some(v) = OcclusionMode::from_code(self.occlusion) {
            out.occlusion = v;
        }
        if let Some(v) = FlowFallback::from_code(self.fallback) {
            out.fallback = v;
        }
        out.smoothness = self.smoothness.clamp(0.0, 100.0);
        out.hud_guard = self.hud_guard;
        out.always = self.always;
        out
    }
}

impl LayerReference {
    /// How this layer's in-between frames are made.
    ///
    /// Every layer has an answer, retimed or not: a layer whose source runs at
    /// a different rate from its comp is already being asked for frames between
    /// the ones it has.
    #[frb(sync)]
    pub fn get_interpolation(&self) -> Result<BridgeRetimeInterp, BridgeError> {
        Ok(match self.item()?.interpolation {
            Interpolation::Nearest => BridgeRetimeInterp::Nearest,
            Interpolation::Blend => BridgeRetimeInterp::Blend,
            Interpolation::Flow(_) => BridgeRetimeInterp::Flow,
        })
    }

    /// Choose how in-between frames are found. One undo step.
    ///
    /// Leaving Flow **parks** the group on the layer rather than dropping it,
    /// and coming back to Flow takes it out again, so comparing a flow shot
    /// against the plain one costs nothing (`Layer::parked_flow`). Both fields
    /// move in the same op, so one undo puts both back.
    #[frb(sync)]
    pub fn set_interpolation(&self, interpolation: BridgeRetimeInterp) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let (interpolation, parked_flow) = match interpolation {
            BridgeRetimeInterp::Flow => (
                Interpolation::Flow(self.live_flow_params(&layer)),
                // The tuning is live again; nothing left in the shed.
                None,
            ),
            other => {
                let parked = match &layer.interpolation {
                    Interpolation::Flow(p) => Some(Box::new(p.clone())),
                    // Already off — keep whatever was parked before.
                    _ => layer.parked_flow.clone(),
                };
                let policy = match other {
                    BridgeRetimeInterp::Blend => Interpolation::Blend,
                    _ => Interpolation::Nearest,
                };
                (policy, parked)
            }
        };
        self.commit(lumit_core::Op::SetLayerInterpolation {
            comp: self.comp_id,
            layer: self.layer_id,
            interpolation,
            parked_flow,
        })
    }

    /// The Flow parameters an edit should build on: the live ones, else the
    /// parked ones, else the defaults.
    fn live_flow_params(&self, layer: &lumit_core::model::Layer) -> lumit_core::retime::FlowParams {
        match &layer.interpolation {
            Interpolation::Flow(p) => p.clone(),
            _ => layer
                .parked_flow
                .as_deref()
                .cloned()
                .unwrap_or_else(Default::default),
        }
    }

    /// This layer's Flow group: the live one, else the parked one it would get
    /// back on, else the defaults — so the panel can show the controls it
    /// *would* have without the policy being Flow yet.
    #[frb(sync)]
    pub fn get_flow_params(&self) -> Result<BridgeFlowParams, BridgeError> {
        Ok(BridgeFlowParams::from_core(
            &self.live_flow_params(&self.item()?),
        ))
    }

    /// Write the Flow group. One undo step.
    ///
    /// Setting parameters *turns flow on* if it was off: the group is only
    /// reachable from a layer whose flow is live, and a write that silently did
    /// nothing would be worse than one that means what it says.
    #[frb(sync)]
    pub fn set_flow_params(&self, params: BridgeFlowParams) -> Result<(), BridgeError> {
        let base = self.live_flow_params(&self.item()?);
        self.commit(lumit_core::Op::SetLayerInterpolation {
            comp: self.comp_id,
            layer: self.layer_id,
            interpolation: Interpolation::Flow(params.onto(&base)),
            parked_flow: None,
        })
    }

    /// The rate this clip is *interpreted* at for flow — the
    /// Flow group's Input rate, as a keyframeable scalar.
    ///
    /// `0` reads as **Auto**: adjacent source frames, the clip's own rate. Any
    /// positive rate below native conforms the clip, so flow brackets the
    /// source frames spaced `1/rate` apart and interpolates between *those*.
    ///
    /// Two quite different footage problems want this, from opposite ends.
    /// High-speed capture — a 600 fps phone clip — has neighbours under two
    /// thousandths of a second apart, so there is almost no motion to
    /// interpolate and slow-motion looks frozen. **Animation drawn on 2s or 3s**
    /// has the mirror problem: the same frame is held two or three times, so
    /// half the pairs flow between a frame and its own duplicate (no motion at
    /// all) and the rest carry double, which reads as judder rather than smooth
    /// slow motion. Conforming to the rate the animation was *drawn* at — 12 fps
    /// for 2s of 24, 8 fps for 3s — makes every bracket span real motion.
    ///
    /// Keyframeable because a scene's cadence is not always constant: anime
    /// commonly switches between 2s and 3s within a cut, and a ramp lets the
    /// conform follow it.
    #[frb(sync)]
    pub fn get_flow_input_rate(&self) -> Result<crate::api::effect::BridgeScalar, BridgeError> {
        let layer = self.item()?;
        let p = self.live_flow_params(&layer).input_fps;
        Ok(crate::api::effect::BridgeScalar::read_at(
            &p,
            layer.start_offset.0,
        ))
    }

    /// Write the Flow input rate. One undo step. Turns flow on if it was off,
    /// for the same reason [`Self::set_flow_params`] does.
    #[frb(sync)]
    pub fn set_flow_input_rate(
        &self,
        value: crate::api::effect::BridgeScalar,
    ) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let animation = value.animation_at(layer.start_offset.0)?;
        let mut params = self.live_flow_params(&layer);
        params.input_fps.animation = animation;
        self.commit(lumit_core::Op::SetLayerInterpolation {
            comp: self.comp_id,
            layer: self.layer_id,
            interpolation: Interpolation::Flow(params),
            parked_flow: None,
        })
    }

    /// Whether flow is live on this layer — the switch-cluster toggle.
    #[frb(sync)]
    pub fn get_flow_enabled(&self) -> Result<bool, BridgeError> {
        Ok(matches!(self.item()?.interpolation, Interpolation::Flow(_)))
    }

    /// Turn flow on or off. Off returns the layer to Nearest — the
    /// policy it had before flow is not recorded, and Nearest is the crisp
    /// default docs/04 §10 names.
    ///
    /// **Turning it off parks the Flow group, it does not discard it.** The
    /// parameters live inside the `Flow` variant of the policy, so while the
    /// policy is Nearest they wait in `Layer::parked_flow` and come back out
    /// when flow does: comparing a flow shot against the plain one is an
    /// ordinary thing to do and does not cost the tuning that got you there.
    /// Parked on the document, not in the view — it serialises and undoes with
    /// everything else, and both fields move in one op, so a single undo puts
    /// the policy and its tuning back together.
    #[frb(sync)]
    pub fn set_flow_enabled(&self, on: bool) -> Result<(), BridgeError> {
        if on == self.get_flow_enabled()? {
            return Ok(());
        }
        self.set_interpolation(if on {
            BridgeRetimeInterp::Flow
        } else {
            BridgeRetimeInterp::Nearest
        })
    }
}
