//! A described plugin, as an entry in the effect catalogue, and the driver
//! that actually plays sound through it.
//!
//! # In plain terms
//!
//! [`schema`](crate::schema) wrote a plugin's *declaration* in Lumit's words;
//! this writes its *behaviour*. Two halves, and they are deliberately apart:
//!
//! * [`AudioEffectDef`] is the catalogue entry — one value implementing the
//!   same [`EffectDef`] trait every built-in implements, so nothing
//!   downstream can tell an EQ plugin from a Gaussian blur except by what it
//!   does. **Both plugin standards collapse into this one type**: VST3 mints
//!   the same declaration as CLAP, and nothing after describe changed to let it.
//! * [`AudioHost`] is where the sound goes. One method per thing the chain
//!   worker needs — a block, the latency, the blob to save — and two
//!   implementations behind it: [`LocalHost`], which loads the plugin into this
//!   process and is for tests, and the broker host AP2 lands, which is what
//!   ships. The seam exists now so that the shipping one drops in without
//!   anything above it noticing (the OFX lesson).
//!
//! # Why the lock is not a lock across FFI
//!
//! [`LocalHost`] keeps its instance behind a mutex because a `&self` method has
//! to reach a plugin that both standards insist is single-threaded. That mutex is held
//! across a call into somebody else's code, which docs/14 §7 forbids **where
//! the plugin can call back and want the same lock**. It cannot: the only host
//! functions a CLAP plugin can reach from inside `process` are the three
//! request flags, and those are atomics
//! ([`HostFlags`](crate::instance::HostFlags)) that take nothing; VST3's
//! component handler is the same shape, one atomic flag. The host offers no
//! extensions at all in v1, so there is no second door.
//!
//! It is also uncontended by construction — one instance is processed by one
//! chain worker, and parallelism is across layers
//! (docs/impl/audio-plugins.md §5).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use lumit_core::fx::{AudioProcessor, EffectDef, EffectSchema, ParamId};
use serde::{Deserialize, Serialize};

use crate::abi::{AnyInstance, AnyModule};
use crate::describe::PluginDescriptor;
use crate::instance::HostError;
use crate::ipc::broker::{Broker, BrokerError};
use crate::ipc::proto::{Bring, InstanceId};
use crate::process::{Block, ParamEvent, INTERLEAVED_LEN};
use crate::schema::{value_routes, ValueRoute};

/// What one live plugin is brought up with.
///
/// The **order** these are applied in is the whole point of the struct, and it
/// is the one in [`crate::HOST_ACTIONS`]: the state first, because it is the
/// plugin's own memory of itself, then the parameters, because they are the
/// project's — **properties win over stale state**
/// (docs/impl/audio-plugins.md §4).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct InstanceSetup {
    /// Which plugin inside the module.
    pub plugin_id: String,
    /// The blob the `.lum` saved, if this instance has been here before.
    pub state: Option<Vec<u8>>,
    /// The values the project holds, by the plugin's own stable parameter id.
    pub params: Vec<(u32, f64)>,
    /// Whether this is an export. Offline means no deadline and the plugin may
    /// take its slower, better path (§3).
    pub offline: bool,
}

impl InstanceSetup {
    /// The same four things, in the shape the pipe carries.
    #[must_use]
    pub fn to_bring(&self) -> Bring {
        Bring {
            plugin_id: self.plugin_id.clone(),
            state: self.state.clone(),
            params: self.params.clone(),
            offline: self.offline,
        }
    }
}

/// Where an audio plugin's blocks actually come from.
///
/// **Never called with any lock held**, and never from a rebuild path: a block
/// may block on somebody else's code.
pub trait AudioHost: Send + Sync {
    /// One block. `input` and `output` are Lumit's **interleaved** stereo,
    /// [`INTERLEAVED_LEN`] samples each; the de-interleaving happens inside.
    ///
    /// `events` need not be sorted — the boundary sorts them, because CLAP
    /// calls an unsorted list undefined and real plugins crash on one.
    /// `steady` is the running frame count since the chain started.
    ///
    /// # Errors
    ///
    /// Whatever the plugin refused. A caller turns that into one dry block and
    /// a strike (§3), never into a stopped mix.
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        events: &[ParamEvent],
        steady: i64,
    ) -> Result<(), HostError>;

    /// The latency the plugin reports, in samples.
    fn latency(&self) -> u32;

    /// The blob to write into the `.lum`. Never parsed, always round-tripped.
    ///
    /// # Errors
    ///
    /// [`HostError::NoExtension`] for a plugin that saves nothing, which is a
    /// perfectly ordinary plugin.
    fn save(&self) -> Result<Vec<u8>, HostError>;
}

/// A plugin loaded into **this** process, of either standard.
///
/// For tests, and for anything Lumit itself ships. The shipping arrangement for
/// third-party code is the broker (AP2): a plugin that dies must cost one block
/// rather than the session, and no amount of care in this file achieves that.
pub struct LocalHost {
    running: Mutex<Running>,
    latency: u32,
    warning: Option<String>,
}

/// The plugin and its scratch, together because a block needs both.
struct Running {
    instance: AnyInstance,
    block: Block,
}

impl LocalHost {
    /// Bring one plugin up, in the order [`crate::HOST_ACTIONS`] pins.
    ///
    /// # Errors
    ///
    /// The plugin refused to be created, to initialise, to activate, or to
    /// start processing. A refused **state blob** or a plugin with no
    /// parameters at all is not an error: it degrades to a
    /// [`LocalHost::warning`] and the plugin still plays, which is CLAP's whole
    /// design and docs/12 §1's rule about losing nothing.
    pub fn open(module: &AnyModule, setup: &InstanceSetup) -> Result<Self, HostError> {
        let mut instance = module.create(&setup.plugin_id)?;
        let mut warning = None;

        if let Some(bytes) = &setup.state {
            if let Err(error) = instance.load_state(bytes) {
                warning = Some(error.to_string());
            }
        }

        let events: Vec<ParamEvent> = setup
            .params
            .iter()
            .map(|(id, value)| ParamEvent {
                time: 0,
                id: *id,
                value: *value,
            })
            .collect();
        if !events.is_empty() {
            if let Err(error) = instance.flush_params(&events) {
                warning = Some(error.to_string());
            }
        }

        if setup.offline {
            instance.set_offline(true);
        }

        instance.activate()?;
        instance.start_processing()?;
        let latency = instance.latency();

        Ok(Self {
            running: Mutex::new(Running {
                instance,
                block: Block::new(),
            }),
            latency,
            warning,
        })
    }

    /// What went wrong bringing this plugin up that did not stop it coming up.
    ///
    /// Read as a **key** by the seam that badges the layer, never shown
    /// verbatim.
    #[must_use]
    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// The three flags the plugin has raised since it was created.
    ///
    /// `restart` is how CLAP says "my latency changed"; AP3's chain worker acts
    /// on it by re-activating and re-placing.
    #[must_use]
    pub fn wants_restart(&self) -> bool {
        self.with(|running| running.instance.wants_restart())
    }

    /// Run `body` with the instance in hand.
    fn with<T>(&self, body: impl FnOnce(&mut Running) -> T) -> T {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        body(&mut running)
    }
}

impl AudioHost for LocalHost {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        events: &[ParamEvent],
        steady: i64,
    ) -> Result<(), HostError> {
        self.with(|running| {
            running.block.load(input);
            running
                .instance
                .process(&mut running.block, events, steady)?;
            running.block.store(output);
            Ok(())
        })
    }

    fn latency(&self) -> u32 {
        self.latency
    }

    fn save(&self) -> Result<Vec<u8>, HostError> {
        // Saving is a main-thread call that CLAP forbids while processing
        // (§9), so the plugin steps out of the stream and back into it. Once
        // per project save, never per block.
        self.with(|running| {
            running.instance.stop_processing();
            let saved = running.instance.save_state();
            let restarted = running.instance.start_processing();
            match (saved, restarted) {
                (Ok(bytes), Ok(())) => Ok(bytes),
                (Ok(bytes), Err(_)) => Ok(bytes),
                (Err(error), _) => Err(error),
            }
        })
    }
}

/// The length of the interleaved buffers [`AudioHost::process`] takes.
pub const BLOCK_SAMPLES: usize = INTERLEAVED_LEN;

// ------------------------------------------------------------ the broker --

/// The lookahead a full ring buys: eight blocks, about eighty-five milliseconds
/// (docs/impl/audio-plugins.md §3).
///
/// The default margin for a block asked for through [`AudioHost::process`],
/// which carries no margin of its own. A chain worker that knows how far ahead
/// it actually is says so per block, through [`BlockJob::margin`].
pub const LOOKAHEAD_MARGIN: Duration = Duration::from_nanos(8 * 10_666_667);

/// One block, as a batch asks for it.
pub struct BlockJob<'a> {
    /// Lumit's interleaved stereo, [`INTERLEAVED_LEN`] samples.
    pub input: &'a [f32],
    /// The parameter values for this block. Need not be sorted.
    pub events: &'a [ParamEvent],
    /// The running frame count since the chain started.
    pub steady: i64,
    /// How much lookahead the caller has left when it asks. This **is** the
    /// deadline, floored at one block period: a plugin gets exactly as long as
    /// the caller can afford to wait and not a second more (§3).
    pub margin: Duration,
}

impl<'a> BlockJob<'a> {
    /// One block with the full ring's margin, which is what a chain worker that
    /// has kept up has.
    #[must_use]
    pub fn new(input: &'a [f32], events: &'a [ParamEvent], steady: i64) -> Self {
        Self {
            input,
            events,
            steady,
            margin: LOOKAHEAD_MARGIN,
        }
    }
}

/// A plugin hosted **in a broker process** — the shipping arrangement
/// (docs/12 §1, docs/impl/audio-plugins.md §5).
///
/// One broker per module — a `.clap` file or a `.vst3` bundle — shared by every
/// plugin in it and by every
/// instance of every plugin in it, behind one lock: a module holding forty
/// effects gets one process, not forty. The lock is short and
/// uncontended by construction — one instance is driven by one chain worker,
/// and parallelism is across layers — and **no FFI happens under it**, because
/// all the FFI is in the other process. What happens under it is a pipe write
/// and a bounded wait, which is what docs/14 §1 permits and what a deadline is
/// for.
pub struct BrokerHost {
    broker: Arc<Mutex<Broker>>,
    /// The plugin's own id, which is what the switched-off list names.
    plugin_id: String,
    instance: InstanceId,
    latency: u32,
    warning: Option<String>,
}

impl BrokerHost {
    /// Bring one plugin up inside an already-spawned, already-described broker.
    ///
    /// # Errors
    ///
    /// [`BrokerError`] — the broker would not make the instance, or the plugin
    /// has already been put away for the session.
    pub fn open(broker: Arc<Mutex<Broker>>, setup: &InstanceSetup) -> Result<Self, BrokerError> {
        let created = {
            let mut held = broker.lock().unwrap_or_else(PoisonError::into_inner);
            held.create_instance(setup.to_bring())?
        };
        Ok(Self {
            broker,
            plugin_id: setup.plugin_id.clone(),
            instance: created.instance,
            latency: created.latency,
            warning: created.warning,
        })
    }

    /// What went wrong bringing this plugin up that did not stop it coming up.
    #[must_use]
    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// The broker this plugin lives in, so a second plugin of the same module
    /// can be opened in the same process.
    #[must_use]
    pub fn broker(&self) -> &Arc<Mutex<Broker>> {
        &self.broker
    }

    /// One batch of blocks, which is how the chain worker fills its lookahead
    /// ring.
    ///
    /// `outputs` holds one whole block per job, back to back. Every job gets an
    /// answer: `Ok` for a block that came back, and `Err` carrying the sentence
    /// for one that did not — which the caller ships **dry**, with a ramp either
    /// side of the splice. A batch never stops early, because the block after a
    /// dead one is very often fine (the broker has already restarted by then).
    ///
    /// The **switched-off list is read once, here, at the top of the batch**: a
    /// plugin the user turns off mid-session stops being asked on the next
    /// batch, and none of its blocks reach a plugin at all.
    pub fn process_batch(
        &self,
        jobs: &[BlockJob<'_>],
        outputs: &mut [f32],
    ) -> Vec<Result<(), HostError>> {
        let mut broker = self.broker.lock().unwrap_or_else(PoisonError::into_inner);
        if broker.is_switched_off(&self.plugin_id) {
            outputs.fill(0.0);
            return jobs
                .iter()
                .map(|_| Err(HostError::Failed("the plugin is switched off".to_owned())))
                .collect();
        }

        let mut answers = Vec::with_capacity(jobs.len());
        for (job, output) in jobs.iter().zip(outputs.chunks_mut(INTERLEAVED_LEN)) {
            let answer = broker.process(
                self.instance,
                job.input,
                output,
                job.events,
                job.steady,
                job.margin,
            );
            answers.push(answer.map_err(HostError::Failed));
        }
        answers
    }
}

impl AudioHost for BrokerHost {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        events: &[ParamEvent],
        steady: i64,
    ) -> Result<(), HostError> {
        let mut broker = self.broker.lock().unwrap_or_else(PoisonError::into_inner);
        broker
            .process(
                self.instance,
                input,
                output,
                events,
                steady,
                LOOKAHEAD_MARGIN,
            )
            .map_err(HostError::Failed)
    }

    fn latency(&self) -> u32 {
        self.latency
    }

    fn save(&self) -> Result<Vec<u8>, HostError> {
        let mut broker = self.broker.lock().unwrap_or_else(PoisonError::into_inner);
        broker
            .save(self.instance)
            .map_err(|error| HostError::Failed(error.to_string()))
    }
}

impl Drop for BrokerHost {
    fn drop(&mut self) {
        let mut broker = self.broker.lock().unwrap_or_else(PoisonError::into_inner);
        broker.destroy(self.instance);
    }
}

/// An audio plugin, as an entry in the effect catalogue.
///
/// One type for both standards, and both really do fill it: nothing downstream
/// of describe knows which standard a plugin speaks.
pub struct AudioEffectDef {
    schema: &'static EffectSchema,
    routes: Vec<ValueRoute>,
    defaults: Vec<(u32, f64)>,
    module: PathBuf,
    plugin_id: String,
}

impl AudioEffectDef {
    /// Build the definition for one described plugin.
    ///
    /// `schema` is the leaked declaration [`crate::schema::schema_of`] made
    /// from the same descriptor; the two are passed separately rather than
    /// derived here because the scan already has the schema in hand and leaking
    /// a second copy of it would be a second answer to the same question.
    #[must_use]
    pub fn new(
        descriptor: &PluginDescriptor,
        schema: &'static EffectSchema,
        module: &Path,
    ) -> Self {
        Self {
            schema,
            routes: value_routes(descriptor),
            defaults: descriptor.defaults(),
            module: module.to_path_buf(),
            plugin_id: descriptor.id.clone(),
        }
    }

    /// Give this definition the `'static` lifetime the catalogue holds.
    ///
    /// The leak is the honest spelling of that lifetime: an effect discovered
    /// at scan time lives as long as the session. Registering it is the
    /// caller's next move and is deliberately not done here — the catalogue
    /// entry and the mix seam have to arrive together (AP3), and a definition
    /// that registered itself would make half of that pair happen out of the
    /// composition root's sight.
    #[must_use]
    pub fn leak(self) -> &'static dyn EffectDef {
        Box::leak(Box::new(self))
    }

    /// Every row's route back to the plugin's own parameter id.
    #[must_use]
    pub fn routes(&self) -> &[ValueRoute] {
        &self.routes
    }

    /// The plugin parameter one schema row addresses. A CLAP parameter id or a
    /// VST3 `ParamID` — the same `u32` either way, which is why one route table
    /// serves both.
    #[must_use]
    pub fn plugin_param(&self, row: ParamId) -> Option<u32> {
        self.routes
            .iter()
            .find(|route| route.id == row)
            .map(|route| route.param)
    }

    /// What a fresh instance of this effect holds, by plugin parameter id.
    #[must_use]
    pub fn defaults(&self) -> &[(u32, f64)] {
        &self.defaults
    }

    /// The `.clap` file or `.vst3` bundle this came out of.
    #[must_use]
    pub fn module(&self) -> &Path {
        &self.module
    }

    /// The plugin's own id inside that file.
    #[must_use]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// The setup that opens this effect with the values given, by row.
    #[must_use]
    pub fn setup(&self, state: Option<Vec<u8>>, values: &[(ParamId, f64)]) -> InstanceSetup {
        let mut params = self.defaults.clone();
        for (row, value) in values {
            if let Some(id) = self.plugin_param(*row) {
                if let Some(slot) = params.iter_mut().find(|(known, _)| *known == id) {
                    slot.1 = *value;
                } else {
                    params.push((id, *value));
                }
            }
        }
        InstanceSetup {
            plugin_id: self.plugin_id.clone(),
            state,
            params,
            offline: false,
        }
    }
}

impl EffectDef for AudioEffectDef {
    fn schema(&self) -> &'static EffectSchema {
        self.schema
    }

    /// An audio effect touches no picture at all, so the render path skips it
    /// and the registry-agreement test excuses it from needing a GPU entry.
    fn is_image_op(&self) -> bool {
        false
    }

    /// One live instance of this plugin, in a broker process.
    ///
    /// This is the whole of the mix seam from the catalogue's side: the mixer
    /// walks a layer's stack, asks every definition in it this question, and
    /// the ones that answer *are* the layer's insert chain, in stack order.
    ///
    /// `None` rather than an error for every way it can fail to come up — the
    /// broker will not start, the module will not describe, the plugin is
    /// switched off — because there is one thing to do about any of them: leave
    /// the link out and let the sound through dry.
    fn open_audio(
        &self,
        state: Option<Vec<u8>>,
        values: &[(ParamId, f64)],
        offline: bool,
    ) -> Option<Arc<dyn AudioProcessor>> {
        // A switched-off plugin is refused before any of its code runs (the
        // switched-off rule, kept for audio): the chain heals around the
        // missing link, the sound goes through dry, and the badge comes from
        // the bridge reading the same list rather than from a block that was
        // never asked for.
        if crate::ipc::broker::session_disabled()
            .lock()
            .is_ok_and(|list| list.contains(&self.plugin_id))
        {
            return None;
        }
        let broker = crate::ipc::broker::module_broker(&self.module).ok()?;
        let mut setup = self.setup(state, values);
        setup.offline = offline;
        let host = BrokerHost::open(broker, &setup).ok()?;
        Some(Arc::new(HostedAudio::new(
            Box::new(host),
            self.routes.clone(),
        )))
    }
}

/// One open plugin, wearing the engine's own words.
///
/// The whole of the translation is the parameter ids: the engine addresses a
/// row by its [`ParamId`] hash and the plugin by its own stable `u32`, and
/// [`ValueRoute`] is the map between them, worked out once at describe. Both
/// standards land here — VST3's front end fills the same routes from its own
/// `ParamID`s, and nothing below this line changed to let it.
pub struct HostedAudio {
    host: Box<dyn AudioHost>,
    routes: Vec<ValueRoute>,
    /// The sentence the most recent refused block carried, for the calm badge
    /// (AP5). Behind a mutex because [`AudioProcessor::process`] takes `&self`;
    /// uncontended by construction — one instance is driven by one bake.
    last_error: Mutex<Option<String>>,
}

impl HostedAudio {
    /// A processor over any host — what a test uses to drive a
    /// [`LocalHost`] through the mixer's own seam without a broker process.
    #[must_use]
    pub fn new(host: Box<dyn AudioHost>, routes: Vec<ValueRoute>) -> Self {
        Self {
            host,
            routes,
            last_error: Mutex::new(None),
        }
    }
}

impl AudioProcessor for HostedAudio {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        steady: i64,
    ) -> bool {
        // One event per row this block carries a number for, at the block's
        // first frame — the ~10 ms control rate the Volume envelope already
        // uses. A row the plugin does not know is simply not routed.
        let events: Vec<ParamEvent> = values
            .iter()
            .filter_map(|(id, value)| {
                let route = self.routes.iter().find(|route| route.id == *id)?;
                Some(ParamEvent {
                    time: 0,
                    id: route.param,
                    value: *value,
                })
            })
            .collect();
        match self.host.process(input, output, &events, steady) {
            Ok(()) => true,
            Err(error) => {
                // Kept, not shown: the bake that ships the block dry reads it
                // off the link and files the badge (AP5).
                if let Ok(mut held) = self.last_error.lock() {
                    *held = Some(error.to_string());
                }
                false
            }
        }
    }

    fn latency(&self) -> u32 {
        self.host.latency()
    }

    fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok()?.clone()
    }
}
