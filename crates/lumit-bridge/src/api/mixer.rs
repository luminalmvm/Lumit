//! The mixer across the bridge: a layer's gain, automation mode, solo safe
//! and lower behind, and a comp's master fader keys, master automation mode
//! and loudness target ([`lumit_core::mixer`]).
//!
//! A layer's settings ride in on [`crate::api::layer::BridgeLayerInfo`].
//! Every setter is one op, one undo step. Beside them, the live desk: a
//! control held and let go, the automation a pass writes, and the mix
//! measured and normalised.

use flutter_rust_bridge::frb;
use lumit_core::anim::{Animation, Property};
use lumit_core::mixer::{AutomationMode, Duck, LoudnessTarget};
use lumit_core::ops::Op;
use lumit_core::time::Rational;
use uuid::Uuid;

use crate::api::effect::BridgeScalar;
use crate::api::layer::LayerReference;
use crate::api::{composition::CompositionReference, BridgeError};
use crate::frb_generated::StreamSink;

/// How a fader writes its keys while the comp plays.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAutomationMode {
    Read,
    Touch,
    Latch,
    Write,
    Trim,
}

impl BridgeAutomationMode {
    #[frb(ignore)]
    pub(crate) fn of(mode: AutomationMode) -> Self {
        match mode {
            AutomationMode::Read => Self::Read,
            AutomationMode::Touch => Self::Touch,
            AutomationMode::Latch => Self::Latch,
            AutomationMode::Write => Self::Write,
            AutomationMode::Trim => Self::Trim,
        }
    }

    #[frb(ignore)]
    fn read(self) -> AutomationMode {
        match self {
            Self::Read => AutomationMode::Read,
            Self::Touch => AutomationMode::Touch,
            Self::Latch => AutomationMode::Latch,
            Self::Write => AutomationMode::Write,
            Self::Trim => AutomationMode::Trim,
        }
    }
}

/// **Lower behind**: the layer or bus whose sound lowers this layer, and how.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeDuck {
    pub behind: Uuid,
    pub depth_db: f64,
    pub threshold_db: f64,
    pub attack_ms: f64,
    pub hold_ms: f64,
    pub release_ms: f64,
}

impl BridgeDuck {
    #[frb(ignore)]
    pub(crate) fn of(duck: &Duck) -> Self {
        Self {
            behind: duck.behind,
            depth_db: duck.depth_db,
            threshold_db: duck.threshold_db,
            attack_ms: duck.attack_ms,
            hold_ms: duck.hold_ms,
            release_ms: duck.release_ms,
        }
    }

    #[frb(ignore)]
    fn read(&self) -> Result<Duck, BridgeError> {
        let all = [
            self.depth_db,
            self.threshold_db,
            self.attack_ms,
            self.hold_ms,
            self.release_ms,
        ];
        if !all.iter().all(|v| v.is_finite()) {
            return Err(BridgeError::ParamKindMismatch);
        }
        Ok(Duck {
            behind: self.behind,
            depth_db: self.depth_db,
            threshold_db: self.threshold_db,
            attack_ms: self.attack_ms,
            hold_ms: self.hold_ms,
            release_ms: self.release_ms,
        })
    }
}

/// `db` held to the range a Gain is set in: -60 to +24 dB, as a clip's
/// gain line is.
#[frb(ignore)]
pub(crate) fn gain_in_range(db: f64) -> f64 {
    db.clamp(-60.0, 24.0)
}

/// Lower behind `behind` with the usual settings, so the interface never
/// holds its own copy of them.
#[frb(sync)]
pub fn duck_behind(behind: Uuid) -> BridgeDuck {
    BridgeDuck::of(&Duck::behind(behind))
}

/// The loudness a comp's mix is made for, in LUFS, and its highest true
/// peak, in dBTP.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeLoudnessTarget {
    pub lufs: f64,
    pub true_peak_db: f64,
}

impl LayerReference {
    fn commit_mixer(&self, op: Op) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store.commit(op).map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Set the layer's own level before its rack and Volume, in dB, held to
    /// -60 to +24 as a clip's gain line is.
    #[frb(sync)]
    pub fn set_audio_gain_db(&self, db: f64) -> Result<(), BridgeError> {
        // Refused rather than written, as a clip's gain is: the save writes a
        // level that is not a number as null, which the reopen will not take.
        if !db.is_finite() {
            return Err(BridgeError::InvalidTime);
        }
        self.commit_mixer(Op::SetLayerAudioGain {
            comp: self.comp_id,
            layer: self.layer_id,
            db: gain_in_range(db),
        })
    }

    #[frb(sync)]
    pub fn set_automation_mode(&self, mode: BridgeAutomationMode) -> Result<(), BridgeError> {
        self.commit_mixer(Op::SetLayerAutomation {
            comp: self.comp_id,
            layer: self.layer_id,
            mode: mode.read(),
        })
    }

    /// A solo safe layer keeps sounding while another layer is soloed.
    #[frb(sync)]
    pub fn set_solo_safe(&self, enabled: bool) -> Result<(), BridgeError> {
        self.commit_mixer(Op::SetLayerSoloSafe {
            comp: self.comp_id,
            layer: self.layer_id,
            solo_safe: enabled,
        })
    }

    /// Lower this layer behind another layer or a bus, or stop, as one
    /// undoable step. Refused, with nothing changed, when the sound would go
    /// round in a circle.
    #[frb(sync)]
    pub fn set_duck(&self, duck: Option<BridgeDuck>) -> Result<(), BridgeError> {
        let duck = duck.map(|d| d.read()).transpose()?;
        self.commit_mixer(Op::SetLayerDuck {
            comp: self.comp_id,
            layer: self.layer_id,
            duck,
        })
    }
}

impl CompositionReference {
    fn commit_mixer(&self, op: Op) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store.commit(op).map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// The master fader: its keys when it has them, in comp time, else its
    /// static level in dB.
    #[frb(sync)]
    pub fn master_volume(&self) -> Result<BridgeScalar, BridgeError> {
        let comp = self.composition()?;
        Ok(match comp.master_keys {
            Some(animation) => BridgeScalar::read_at(
                &Property {
                    animation,
                    ..Property::zero()
                },
                Rational::ZERO,
            ),
            None => BridgeScalar::Static(comp.master_volume_db),
        })
    }

    /// Set the master fader, as one undoable step. A static level clears any
    /// keys; keys leave the static level as it is, for an older Lumit to read.
    #[frb(sync)]
    pub fn set_master_volume(&self, value: BridgeScalar) -> Result<(), BridgeError> {
        let op = match value.animation_at(Rational::ZERO)? {
            Animation::Static(db) if !db.is_finite() => return Err(BridgeError::InvalidTime),
            Animation::Static(db) => {
                let level = Op::SetMasterVolume { comp: self.id, db };
                if self.composition()?.master_keys.is_some() {
                    Op::Batch {
                        ops: vec![
                            Op::SetMasterAutomation {
                                comp: self.id,
                                keys: None,
                            },
                            level,
                        ],
                    }
                } else {
                    level
                }
            }
            keys @ Animation::Keyframed(_) => Op::SetMasterAutomation {
                comp: self.id,
                keys: Some(keys),
            },
            // The master is keyed or still, never driven by an expression.
            Animation::Expression(_) => return Err(BridgeError::ParamKindMismatch),
        };
        self.commit_mixer(op)
    }

    #[frb(sync)]
    pub fn master_automation_mode(&self) -> Result<BridgeAutomationMode, BridgeError> {
        Ok(BridgeAutomationMode::of(
            self.composition()?.master_automation,
        ))
    }

    #[frb(sync)]
    pub fn set_master_automation_mode(
        &self,
        mode: BridgeAutomationMode,
    ) -> Result<(), BridgeError> {
        self.commit_mixer(Op::SetMasterAutomationMode {
            comp: self.id,
            mode: mode.read(),
        })
    }

    #[frb(sync)]
    pub fn loudness_target(&self) -> Result<Option<BridgeLoudnessTarget>, BridgeError> {
        Ok(self
            .composition()?
            .loudness_target
            .map(|t| BridgeLoudnessTarget {
                lufs: t.lufs,
                true_peak_db: t.true_peak_db,
            }))
    }

    #[frb(sync)]
    pub fn set_loudness_target(
        &self,
        target: Option<BridgeLoudnessTarget>,
    ) -> Result<(), BridgeError> {
        if target.is_some_and(|t| !t.lufs.is_finite() || !t.true_peak_db.is_finite()) {
            return Err(BridgeError::ParamKindMismatch);
        }
        self.commit_mixer(Op::SetLoudnessTarget {
            comp: self.id,
            target: target.map(|t| LoudnessTarget {
                lufs: t.lufs,
                true_peak_db: t.true_peak_db,
            }),
        })
    }

    /// Whether `layer` may send or output to `bus` without the sound going
    /// round in a circle, for a menu to ask before it offers the bus.
    #[frb(sync)]
    pub fn route_allowed(&self, layer: Uuid, bus: Uuid) -> bool {
        self.composition()
            .is_ok_and(|comp| lumit_core::routes::route_allowed(&comp, layer, bus))
    }
}

/// A control on the mixer that can be held: a layer's or bus's Volume or
/// Pan, one of its sends, or the master fader.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMixTarget {
    Volume { layer: Uuid },
    Pan { layer: Uuid },
    Send { layer: Uuid, bus: Uuid },
    Master,
}

/// One point of a line being written, in comp seconds.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeAutomationPoint {
    pub seconds: f64,
    /// dB for a Volume, a send or the master; -100 to 100 for a Pan.
    pub value: f64,
}

/// What one control has written so far this pass.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAutomationLine {
    pub target: BridgeMixTarget,
    pub points: Vec<BridgeAutomationPoint>,
}

/// What a pass has drawn since a point the caller holds.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAutomationPassSince {
    /// Which pass is being written. Another than the caller asked about:
    /// its lines are dropped, and these points are from the pass's start.
    pub pass: u32,
    /// How many points the pass has drawn: the `from` of the next ask.
    pub next: u32,
    /// The points drawn since, by control, in the order drawn.
    pub lines: Vec<BridgeAutomationLine>,
}

/// A comp's whole mix, measured.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeMixLoudness {
    /// Integrated loudness, in LUFS.
    pub integrated: f64,
    /// The loudest true peak, in dBTP.
    pub true_peak_db: f64,
    /// Loudness range, in LU.
    pub range: f64,
    /// The loudest three seconds, in LUFS.
    pub short_term_max: f64,
}

/// How far a measure of the mix has got, 0 to 1.
pub type MixMeasureStream = StreamSink<f64>;

impl CompositionReference {
    /// The document as it stands.
    #[frb(ignore)]
    fn mixer_snapshot(&self) -> Result<std::sync::Arc<lumit_core::model::Document>, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(proj.store.snapshot())
    }

    /// The document without waiting: while an edit is being written (a
    /// pass committing, say), the one the playing mix was built from. The
    /// controls are moved on the interface's own thread, which must never
    /// stand still behind an edit.
    #[frb(ignore)]
    fn live_snapshot(&self) -> Result<std::sync::Arc<lumit_core::model::Document>, BridgeError> {
        let proj = self.project()?;
        if let Ok(proj) = proj.try_read() {
            return Ok(proj.store.snapshot());
        }
        #[cfg(feature = "media")]
        if let Some(doc) = crate::audio::automation::loaded_doc(self.id) {
            return Ok(doc);
        }
        self.mixer_snapshot()
    }

    /// The pointer is on `target` at `value` (dB, or -100 to 100 for a Pan):
    /// call it on every move while held. Heard at once. While the comp plays
    /// and the strip (or the master) is in Touch, Latch, Write or Trim, the
    /// moves are written as keys when playback stops.
    #[frb(sync)]
    pub fn mixer_touch(&self, target: BridgeMixTarget, value: f64) -> Result<(), BridgeError> {
        if !value.is_finite() {
            return Err(BridgeError::ParamKindMismatch);
        }
        let doc = self.live_snapshot()?;
        #[cfg(feature = "media")]
        crate::audio::automation::touch(self.id, target, value, &doc);
        #[cfg(not(feature = "media"))]
        let _ = (target, doc);
        Ok(())
    }

    /// The pointer has left `target`. Stopped, or in Read, the value is
    /// written as one undo step: a key at the playhead when the property is
    /// keyed, else its value. In Touch and Trim the control glides back to
    /// its keys over the touch return; in Latch and Write it holds until
    /// playback stops.
    ///
    /// `playhead_seconds` is where the playhead stands, for that key; without
    /// it the sound's own clock is taken.
    #[frb(sync)]
    pub fn mixer_release(
        &self,
        target: BridgeMixTarget,
        playhead_seconds: Option<f64>,
    ) -> Result<(), BridgeError> {
        let doc = self.live_snapshot()?;
        #[cfg(feature = "media")]
        if let Some((at, value)) = crate::audio::automation::release(self.id, target, &doc) {
            let at = playhead_seconds.filter(|s| s.is_finite()).unwrap_or(at);
            let op = crate::audio::record::value_op(&doc, self.id, target, at, value);
            let proj = self.project()?;
            let comp = self.id;
            // Written now unless an edit is being written already, when it
            // goes on a thread of its own rather than holding the interface
            // up; the value stays held until its mix lands either way.
            if let Ok(state) = proj.try_write() {
                let done = op.map_or(Ok(()), |op| state.store.commit(op).map(|_| ()));
                let after = state.store.snapshot();
                drop(state);
                crate::audio::automation::written(comp, after);
                return done.map_err(BridgeError::OpError);
            }
            crate::audio::automation::later(move || {
                let Ok(state) = proj.write() else {
                    return;
                };
                if let Some(op) = op {
                    let _ = state.store.commit(op);
                }
                let after = state.store.snapshot();
                drop(state);
                crate::audio::automation::written(comp, after);
            });
            return Ok(());
        }
        #[cfg(not(feature = "media"))]
        let _ = (target, doc, playhead_seconds);
        Ok(())
    }

    /// The points written so far this pass, for drawing the line being
    /// written. Empty when nothing records. Copies the whole pass:
    /// [`Self::automation_pass_since`] is the form to poll.
    #[frb(sync)]
    pub fn automation_pass(&self) -> Vec<BridgeAutomationLine> {
        #[cfg(feature = "media")]
        {
            lines(crate::audio::automation::pass(self.id))
        }
        #[cfg(not(feature = "media"))]
        Vec::new()
    }

    /// The points the pass has drawn since point `from` of pass `pass`, to
    /// poll every frame: the caller keeps the pass and the `next` it was
    /// last given. A new pass answers from its own start, under its own
    /// number. Copies only what is new and waits on no edit.
    #[frb(sync)]
    pub fn automation_pass_since(&self, pass: u32, from: u32) -> BridgeAutomationPassSince {
        #[cfg(feature = "media")]
        {
            let (pass, next, new) =
                crate::audio::automation::pass_since(self.id, pass, from as usize);
            BridgeAutomationPassSince {
                pass,
                next: u32::try_from(next).unwrap_or(u32::MAX),
                lines: lines(new),
            }
        }
        #[cfg(not(feature = "media"))]
        {
            let _ = (pass, from);
            BridgeAutomationPassSince {
                pass: 0,
                next: 0,
                lines: Vec::new(),
            }
        }
    }

    /// Play this comp's whole mix offline, as an export does, and measure it.
    /// Stopped by [`cancel_measure_mix`]. A silent mix is refused.
    pub fn measure_mix(
        &self,
        progress: Option<MixMeasureStream>,
    ) -> Result<BridgeMixLoudness, BridgeError> {
        let doc = self.mixer_snapshot()?;
        #[cfg(feature = "media")]
        {
            let m = measured(progress, |on_progress| {
                crate::audio::measure::measure(&doc, self.id, on_progress)
            })?;
            Ok(BridgeMixLoudness {
                integrated: m.integrated,
                true_peak_db: m.true_peak_db,
                range: m.range,
                short_term_max: m.short_term_max,
            })
        }
        #[cfg(not(feature = "media"))]
        {
            let _ = (doc, progress);
            Err(BridgeError::NoAudio)
        }
    }

    /// Measure the mix as playback hears it, through the limiter, then move
    /// the master by what takes it to `target_lufs`: its value, or every key
    /// by the same amount. Measured again after the move, and moved once
    /// more where the limiter kept it over half a unit off. One undo step.
    /// Answers the mix's loudness as measured after it.
    pub fn normalise_mix(
        &self,
        target_lufs: f64,
        progress: Option<MixMeasureStream>,
    ) -> Result<f64, BridgeError> {
        if !target_lufs.is_finite() {
            return Err(BridgeError::ParamKindMismatch);
        }
        let doc = self.mixer_snapshot()?;
        #[cfg(feature = "media")]
        {
            let (delta, m) = measured(progress, |on_progress| {
                crate::audio::measure::normalise(&doc, self.id, target_lufs, on_progress)
            })?;
            let comp = self.composition()?;
            let op = match comp.master_keys {
                Some(Animation::Keyframed(mut keys)) => {
                    for k in &mut keys {
                        k.value += delta;
                    }
                    Op::SetMasterAutomation {
                        comp: self.id,
                        keys: Some(Animation::Keyframed(keys)),
                    }
                }
                Some(Animation::Static(db)) => Op::SetMasterAutomation {
                    comp: self.id,
                    keys: Some(Animation::Static(db + delta)),
                },
                _ => Op::SetMasterVolume {
                    comp: self.id,
                    db: comp.master_volume_db + delta,
                },
            };
            let proj = self.project()?;
            let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
            let own = proj.store.begin_own_undo_group();
            let done = proj.store.commit(op);
            if own {
                proj.store.name_undo_group("Normalise mix");
                proj.store.end_undo_group();
            }
            done.map_err(BridgeError::OpError)?;
            Ok(m.integrated)
        }
        #[cfg(not(feature = "media"))]
        {
            let _ = (doc, progress);
            Err(BridgeError::NoAudio)
        }
    }
}

/// Lines of points as the bridge carries them.
#[cfg(feature = "media")]
fn lines(lines: Vec<crate::audio::automation::PassLine>) -> Vec<BridgeAutomationLine> {
    lines
        .into_iter()
        .map(|(target, points)| BridgeAutomationLine {
            target,
            points: points
                .into_iter()
                .map(|(seconds, value)| BridgeAutomationPoint { seconds, value })
                .collect(),
        })
        .collect()
}

/// Run a measure of the mix, telling `progress` as it goes.
#[cfg(feature = "media")]
fn measured<T>(
    progress: Option<MixMeasureStream>,
    run: impl FnOnce(&mut dyn FnMut(f64)) -> Result<T, crate::audio::measure::Refused>,
) -> Result<T, BridgeError> {
    use crate::audio::measure::Refused;
    run(&mut |fraction| {
        if let Some(sink) = progress.as_ref() {
            // A dropped report costs the bar one step and nothing else.
            let _ = sink.add(fraction);
        }
    })
    .map_err(|refused| match refused {
        Refused::Busy => BridgeError::AnalysisBusy,
        Refused::Stopped => BridgeError::Stopped,
        Refused::Silent => BridgeError::NoAudio,
    })
}

/// Stop the measure of a mix that is running.
#[frb(sync)]
pub fn cancel_measure_mix() {
    #[cfg(feature = "media")]
    crate::audio::measure::stop();
}

/// How long a fader let go in Touch takes to glide back to its keys, in
/// milliseconds: Settings, Audio, Touch return.
#[frb(sync)]
pub fn set_touch_return_ms(ms: u32) {
    #[cfg(feature = "media")]
    crate::audio::automation::set_touch_return_ms(ms);
    #[cfg(not(feature = "media"))]
    let _ = ms;
}
