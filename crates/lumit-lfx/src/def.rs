//! A described plugin, turned into an entry in the effect catalogue
//! (docs/impl/lfx.md §4.2).
//!
//! # In plain terms
//!
//! [`schema`](crate::schema) wrote a plugin's *declaration* in Lumit's words;
//! this writes its *behaviour*. Every one of Lumit's own effects is a value
//! implementing one trait - [`EffectDef`] - and the catalogue is a list of
//! those values (docs/impl/effect-registry.md §2.4). An LFX plugin becomes one
//! more value of exactly that trait, registered into exactly that list. From
//! there nothing downstream can tell the difference: the Add-effect menu, the
//! Effect controls panel, the resolve walk, the frame key and the cache all see
//! an effect.
//!
//! # What this definition does with each hook
//!
//! * **`schema`** hands back the leaked declaration the describe made.
//! * **`apply_cpu_temporal`** is the fp32 render: the resolved bag becomes the
//!   dense value array the plugin reads, the picture and its neighbours cross
//!   as [`Picture::F32`], and what comes back is written into `rgba`.
//! * **`apply_f16_temporal`** is the same request at the project's own depth,
//!   and [`LfxDef`] is its only implementor (§4.5). It never declines: a frame
//!   that failed is identity at **this** depth, and answering `false` would
//!   hand the same frame to the plugin a second time down the f32 road.
//! * **`frames_needed`** is what the instance said it reads, as offsets from
//!   the frame that was rendered - read off its last render and clamped to
//!   ±[`MAX_OFFSET`], because the key walk asks this once per live effect per
//!   frame and may not talk to another process to answer it.
//! * **`last_error`** is a thread-local written on **every** road out of a
//!   render - a good frame clears what the frame before it filed - and
//!   **taken** on read besides, so a stale reason cannot badge a later frame
//!   that went perfectly well.
//! * **`hidden_rows`** is the describe-time `hidden` flags, through the same
//!   headings a hidden run hides.
//! * **`press`** is an `ACTION` row. Version 1's entry table has no press hook,
//!   so the broker acknowledges it and nothing runs; the definition still
//!   refuses a name that is not one of this effect's buttons.
//! * **`resolve_derived`** pushes `derived.frame` and nothing else. There is no
//!   `derived.memory` here and there never will be: D8 keeps `plugin_state`
//!   empty for every LFX instance, which is what makes the frame key complete
//!   and a restart an exact replay.
//!
//! # Identity means identity
//!
//! **A failed process returns without writing `rgba` at all.** Not the input
//! written back - that would put the picture through a depth boundary and
//! change it very slightly, and "renders as identity" must not mean that. The
//! same sentence read one level down is the broker's: the buffer a plugin is
//! handed starts as the input, so a plugin that answers `LFX_STATUS_OK` and
//! writes nothing has rendered identity too. What this file owns is the other
//! half - the plugin that writes half the output and *then* fails, whose
//! half-written buffer never reaches the caller because the caller's own is
//! never touched.
//!
//! # The two seams behind it
//!
//! [`LfxHost`] is the frame and the press, and it is the seam the per-render
//! disable gate wraps ([`Gated`](crate::Gated), §5.4). [`LfxInstances`] is the
//! live instance behind them: opening one, telling it what its controls hold,
//! and forgetting it. They are two traits rather than one because the gate has
//! to read the switched-off list inside exactly the two calls that run a
//! plugin's pixels, and because a definition that could open an instance
//! through the gate would have to mint a handle before the gate could refuse.
//! In the shipping path both are one [`BrokerHost`] over one bundle's broker,
//! the first of them behind a `Gated`.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use lumit_core::fx::{
    CurvePoints, EffectDef, EffectSchema, ParamId, Params, PressFrame, Pressed, ResolveCx, Value,
};
use lumit_core::model::{EffectInstance, EffectValue};
use uuid::Uuid;

use crate::describe::{Declaration, Declared, PluginDescriptor};
use crate::discover::{rendering_of, Gated, LfxHost, Rendering};
use crate::ipc::broker::{Broker, BrokerError, Picture, ProcessJob};
use crate::ipc::proto::{InstanceId, ParamValue, RectI};
use crate::pool::{Lease, Policy, Pool, PoolError, Serial};
use crate::schema::{value_routes, ValueRoute};

/// The furthest either side of the frame an instance's answer is believed.
///
/// The neighbour decode is what pays for these, and a plugin asking for a
/// thousand frames either side is asking for more than the ring will ever hold -
/// so the answer is clamped here rather than refused later, which is the
/// older host's arithmetic (`lumit-ofx`'s `offsets_of`) read off a list of
/// offsets instead of a range of absolute frames.
///
/// **The header's own number rather than a second spelling of it.**
/// `LFX_MAX_TEMPORAL_WINDOW` is the whole of what a plugin may declare, and
/// `proto::held_to_the_window` already clamps every `frames_needed` to it as it
/// comes off the pipe - so the clamp here is belt and braces over that one.
/// Writing `64` again would mean that widening the header's window left the
/// frame key computed over a narrower one than the plugin actually reads: a key
/// that does not depend on the frames the effect reads, serving cached frames
/// that are wrong, with nothing in either type system to notice.
pub const MAX_OFFSET: i32 = lumit_lfx_abi::LFX_MAX_TEMPORAL_WINDOW;

/// The prefix the host's own pushes into the resolved bag are spelled under,
/// and the one piece of a row id an LFX plugin may not declare.
///
/// `resolve_derived` writes [`DERIVED_FRAME`] into the same bag every route out
/// of a definition reads, and the built-ins beside it push a dozen more of
/// their own (`derived.noise`, `derived.px_scale`, …). A plugin declaring a
/// control by one of these names would have the person's value overwritten on
/// every render, with nothing said anywhere - so the whole prefix is reserved
/// and a declaration inside it is [`LfxRejection::ReservedParamId`](crate::LfxRejection::ReservedParamId).
/// The vocabulary is Lumit's and it is frozen, which is what makes a rule
/// affordable here that the older host does not have.
pub const DERIVED_PREFIX: &str = "derived.";

/// The bag id the layer's frame rides under: what the plugin is told the time
/// is, in the comp frames LFX counts in rather than the seconds the resolve
/// walk speaks. An instance asks for the frames either side as `time ± 1`, and
/// a time in seconds would make that a second either side.
const DERIVED_FRAME_ID: &str = "derived.frame";

/// The same id, hashed, which is how the bag is keyed.
const DERIVED_FRAME: ParamId = ParamId::new(DERIVED_FRAME_ID);

thread_local! {
    /// Why the render this thread most recently ran was a placeholder.
    ///
    /// Thread-local rather than a field, because it is read by the dispatch
    /// seam immediately after the call that set it, on the thread that made it:
    /// a shared field would let two frames in flight report each other's
    /// failure. **Written on every road out of a render**, so a frame that
    /// worked clears what the frame before it filed; and **taken** when read
    /// besides. Both halves are needed and neither is the other: this slot is
    /// one per thread rather than one per definition, and nothing reads it
    /// after a frame that worked - so without the clearing, a sentence filed
    /// by one plugin's failure would still be here when the next plugin's good
    /// frame is asked what went wrong.
    static LAST_ERROR: std::cell::RefCell<Option<String>> = const {
        std::cell::RefCell::new(None)
    };
}

// ------------------------------------------------------------- the instances --

/// Where an effect instance's **live plugin instance** comes from.
///
/// [`LfxHost`] is the frame; this is what there has to be one of before a frame
/// can be asked for. It is a seam of its own rather than two more methods on
/// that trait because the disable gate wraps [`LfxHost`] and reads the
/// switched-off list inside every call through it - which is exactly right for
/// a frame and a press, and wrong for a handle the gate would have to mint
/// before it could refuse (§5.4).
///
/// **No lock of the pool's rows is held across any of these**, and never from a
/// rebuild path: every one of them may block on somebody else's code in
/// somebody else's process, and the rows are read once per live effect per
/// frame by a frame-key walk that may not wait behind a render. What makes
/// telling an instance its values and asking it for a frame indivisible is the
/// **lease** rather than a lock (§4.4): a leased instance is not visible to any
/// other frame, so there is nobody to interleave with.
///
/// The one lock that *is* held across all four is the bundle's
/// [`Serial`](crate::pool::Serial), and only for a bundle one of whose plugins
/// declared `lfx.thread-unsafe` - which is the whole of what that declaration
/// buys and is why it is discouraged (§2.6).
pub trait LfxInstances: Send + Sync {
    /// Open one live instance, with these values in its controls.
    ///
    /// # Errors
    ///
    /// [`BrokerError`] - the plugin is switched off or put away, the broker
    /// refused, or there is no handle left to name it by.
    fn open(&self, values: Vec<ParamValue>) -> Result<InstanceId, BrokerError>;

    /// Replace an instance's values, in declaration order.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    fn update(&self, instance: InstanceId, values: Vec<ParamValue>) -> Result<(), BrokerError>;

    /// Forget an instance. A driver that has already lost it says nothing.
    fn close(&self, instance: InstanceId);
}

/// One bundle's broker, driving one of the plugins in it.
///
/// One broker per **bundle** behind one lock, because a broker owns a pipe and
/// a shared-memory ring and both are single-conversation things - the same
/// arrangement, and the same recorded ceiling, as the older host's
/// `BrokerHost`: the parallelism is across brokers until one broker can carry
/// two conversations at once (§4.4's *ponytail*).
pub struct BrokerHost {
    /// **Shared**, because a bundle holds many plugins and §3.3 puts one broker
    /// process behind the bundle rather than behind each of them.
    broker: Arc<Mutex<Broker>>,
    /// Which plugin of the bundle, by **the descriptor's own id** rather than
    /// by a position in the last `Described` - the same rule a restart's replay
    /// follows, for the same reason: the disable list can shift a position.
    plugin: String,
}

impl BrokerHost {
    /// Drive one plugin of an already-spawned, already-described broker.
    #[must_use]
    pub fn new(broker: Arc<Mutex<Broker>>, plugin: impl Into<String>) -> Self {
        Self {
            broker,
            plugin: plugin.into(),
        }
    }

    /// The broker, whatever a poisoned lock says - the same reasoning
    /// [`crate::discover`] gives for its own tables: a broker some other thread
    /// panicked while holding is still a broker, and losing the bundle is a
    /// worse answer than talking to it.
    fn broker(&self) -> MutexGuard<'_, Broker> {
        self.broker.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl LfxHost for BrokerHost {
    fn process(&self, instance: InstanceId, job: &ProcessJob<'_>) -> Rendering {
        let answer = self.broker().process(instance, job);
        rendering_of(job, answer)
    }

    fn press(&self, instance: InstanceId, param: &str) -> Result<(), BrokerError> {
        self.broker().press(instance, param)
    }
}

impl LfxInstances for BrokerHost {
    fn open(&self, values: Vec<ParamValue>) -> Result<InstanceId, BrokerError> {
        self.broker().create_instance(&self.plugin, values)
    }

    fn update(&self, instance: InstanceId, values: Vec<ParamValue>) -> Result<(), BrokerError> {
        self.broker().set_values(instance, values)
    }

    fn close(&self, instance: InstanceId) {
        self.broker().destroy(instance);
    }
}

// ------------------------------------------------------------ the definition --

/// An LFX plugin, as an entry in the effect catalogue.
pub struct LfxDef {
    schema: &'static EffectSchema,
    /// The plugin's own reverse-domain identifier - what the switched-off list
    /// names, and what a refusal is filed under.
    identifier: String,
    /// Every schema row's way back to the element of the dense value array it
    /// is part of, read **off** the built schema rather than minted again.
    routes: Vec<ValueRoute>,
    /// What the plugin declared every element to be, which is what a row the
    /// bag has no value for keeps - a `FILE` row's path today, and any row a
    /// stack built by hand never filled in.
    defaults: Vec<ParamValue>,
    /// The rows the plugin declared hidden, through the same headings a hidden
    /// run hides. `&'static` because the hook that reads them answers in the
    /// schema's own ids.
    hidden: Vec<&'static str>,
    /// Which declarations are buttons, by row id.
    actions: BTreeSet<&'static str>,
    host: Arc<dyn LfxHost>,
    instances: Arc<dyn LfxInstances>,
    /// The live plugin instances this definition holds: one per in-flight
    /// frame, pooled per row, and grown under §4.4's provisional policy
    /// ([`Pool`]).
    pool: Pool,
}

impl LfxDef {
    /// Build the definition for one described plugin.
    ///
    /// `schema` is the leaked declaration [`crate::schema::schema_of`] made from
    /// the same descriptor; the two are passed separately rather than derived
    /// here because the scan already has the schema in hand and leaking a second
    /// copy of it would be a second answer to the same question.
    ///
    /// `policy` is §4.4's, and arrives rather than being read off the descriptor
    /// here because two of its three ceilings are the **bundle's** - the ring
    /// the ledger granted that bundle's broker, and the lock a thread-unsafe
    /// declaration pins every plugin of that bundle behind.
    #[must_use]
    pub fn new(
        descriptor: &PluginDescriptor,
        schema: &'static EffectSchema,
        instances: Arc<dyn LfxInstances>,
        host: Arc<dyn LfxHost>,
        policy: Policy,
    ) -> Self {
        Self {
            schema,
            identifier: descriptor.identity.id.clone(),
            routes: value_routes(descriptor, schema),
            defaults: defaults_of(descriptor, schema),
            hidden: crate::schema::hidden_rows(descriptor, schema)
                .into_iter()
                .collect(),
            actions: action_rows(descriptor, schema),
            host,
            instances,
            pool: Pool::new(policy),
        }
    }

    /// The shipping arrangement: one bundle's broker, driving this plugin,
    /// behind the per-render disable gate.
    ///
    /// The gate is on the **frame and the press** and not on the instance seam,
    /// which is §5.4's shape: a plugin switched off mid-session stops rendering
    /// now, and one switched off before the row was ever drawn is refused at
    /// [`Self::lease`] with the same typed `SwitchedOff` the gate answers a
    /// press with.
    ///
    /// `serial` is the **bundle's** lock and is shared by every definition built
    /// over this broker: `lfx.thread-unsafe` serialises the bundle rather than
    /// the plugin (§2.6), so two plugins of one thread-unsafe bundle may not
    /// render at once either. It is armed from the whole described list -
    /// [`Serial::for_bundle`] over `Broker::described`'s trait flags - and not
    /// from this plugin's own block, since a bundle where one plugin declares
    /// the bit and the next does not is the shape the promise is about.
    #[must_use]
    pub fn hosted(
        descriptor: &PluginDescriptor,
        schema: &'static EffectSchema,
        broker: Arc<Mutex<Broker>>,
        serial: &Serial,
    ) -> Self {
        // The policy is read off the broker the definition will talk to, so the
        // pool's pressure and the ring's slots are the same bundle's numbers
        // rather than a second opinion about them (§4.4).
        let policy = {
            let held = broker.lock().unwrap_or_else(PoisonError::into_inner);
            Policy::declared(
                descriptor.identity.traits.as_ref(),
                &held.granted_slots(),
                held.ledger(),
                serial,
            )
        };
        let identifier = descriptor.identity.id.clone();
        let driver = Arc::new(BrokerHost::new(broker, identifier.clone()));
        let gate: Arc<dyn LfxHost> = Arc::new(Gated::new(
            identifier,
            Arc::clone(&driver) as Arc<dyn LfxHost>,
        ));
        Self::new(descriptor, schema, driver, gate, policy)
    }

    /// Give this definition the `'static` lifetime the catalogue holds.
    ///
    /// The leak is the honest spelling of that lifetime: an effect discovered at
    /// scan time lives as long as the session. Registering it is the caller's
    /// next move, and it is deliberately **not** done here - the catalogue entry
    /// and the render pass have to arrive together (§4.5), and a definition that
    /// registered itself would make half of that pair happen out of the
    /// composition root's sight.
    #[must_use]
    pub fn leak(self) -> &'static dyn EffectDef {
        Box::leak(Box::new(self))
    }

    /// Which plugin this definition is, by the id the switched-off list and the
    /// `REFUSED` table name it under.
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// The live instances this definition holds, and the policy they are grown
    /// under (§4.4).
    ///
    /// Read-only, and read by whoever wants to know what the pool did: the
    /// Addons page's own numbers belong to that page, and the cases that hold §4.4's
    /// clauses are here.
    #[must_use]
    pub const fn pool(&self) -> &Pool {
        &self.pool
    }

    /// The resolved bag as the dense array the plugin reads, in declaration
    /// order and one element per declaration that carries a value.
    ///
    /// Starts from what the plugin declared, so a row the bag has no value for -
    /// a `FILE` row, which `resolve_into_arena` drops outright, or any row a
    /// stack built by hand never filled in - keeps the plugin's own default
    /// rather than nothing.
    fn values_of(&self, p: Params<'_>) -> Vec<ParamValue> {
        let mut values = self.defaults.clone();
        for route in &self.routes {
            let Some(value) = p.get(route.id) else {
                continue;
            };
            if let Some(slot) = values.get_mut(route.element as usize) {
                write_component(slot, route.component, value);
            }
        }
        values
    }

    /// The document's own rows at `lt`, which is what a press starts from since
    /// a press happens outside a render and so has no bag.
    fn values_at(&self, inst: &EffectInstance, lt: f64) -> Vec<ParamValue> {
        let mut values = self.defaults.clone();
        for route in &self.routes {
            let Some(param) = inst.params.iter().find(|param| param.id == route.row) else {
                continue;
            };
            let Some(value) = stored(&param.value, lt) else {
                continue;
            };
            if let Some(slot) = values.get_mut(route.element as usize) {
                write_component(slot, route.component, value);
            }
        }
        values
    }

    /// A live plugin instance for one in-flight frame of this row, leased from
    /// the pool for the length of that frame (§4.4).
    ///
    /// The lease is what makes telling an instance its values and asking it for
    /// a picture one indivisible pair: a leased instance is not visible to any
    /// other frame, so two frames of one row dispatched out of order cannot
    /// interleave as *tell it A, tell it B, ask for A* - which would hand a
    /// frame back painted with the other frame's numbers, as an `Ok`, to be
    /// cached under its own key. Where the pool may grow they are two instances
    /// and where it may not they are one frame after the other; either way
    /// neither frame is ever told anything in the middle of the other's turn.
    ///
    /// `samples` is what the declared scratch is charged against, so a press
    /// passes nought.
    ///
    /// # Errors
    ///
    /// [`PoolError`] - the plugin is switched off or put away, the driver would
    /// not open an instance, or the frame's declared working memory is more
    /// than the ledger will grant and it is not dispatched at all.
    fn lease(
        &self,
        inst: Uuid,
        values: &[ParamValue],
        samples: usize,
    ) -> Result<Lease<'_>, PoolError> {
        // §5.4's list, read at the one place the gate cannot see. Opening an
        // instance runs the plugin's `create`, and a switched-off plugin's code
        // does not run - so this answers the same typed refusal `Gated::press`
        // answers, whose `Display` is the one shared `DISABLED_REASON` the badge
        // seam tests against. It is read before the pool is touched, so a
        // switched-off plugin costs no lease and no reservation either.
        if crate::discover::is_disabled(&self.identifier) {
            return Err(PoolError::Broker(BrokerError::SwitchedOff));
        }
        self.pool.lease(inst, values, samples, &*self.instances)
    }

    /// One frame, at whichever depth it arrived in, or `None` where there is no
    /// frame to hand back and the caller's buffer must be left exactly as it
    /// found it.
    ///
    /// Every road out of here that is not a picture files a sentence, so the
    /// layer wears a calm badge rather than the comp stopping - and **a road
    /// out that is a picture clears the last one**, which is the half that
    /// makes the promise `last_error`'s own doc comment states true. Taking
    /// the sentence on read is not enough on its own: nothing reads it after a
    /// frame that worked, `LAST_ERROR` is one slot per thread rather than per
    /// definition, and a render worker that carried a failure of one plugin
    /// into a good frame of the next would badge the second with the first's
    /// words. So there is one assignment on every road out, as `lumit-ofx`'s
    /// definition has.
    fn render(
        &self,
        inst: Uuid,
        frame: f64,
        input: Picture,
        size: (u32, u32),
        p: Params<'_>,
        neighbours: &[(f64, Picture)],
    ) -> Option<Picture> {
        match self.rendered(inst, frame, input, size, p, neighbours) {
            Ok(pixels) => {
                LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
                Some(pixels)
            }
            Err(why) => {
                LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(why));
                None
            }
        }
    }

    /// The frame itself: the picture the plugin made, or why there is not one.
    ///
    /// Split from [`LfxDef::render`] above so that every road out names itself
    /// and exactly one of them writes the thread's slot. A road added here that
    /// forgot to file would not compile.
    fn rendered(
        &self,
        inst: Uuid,
        frame: f64,
        input: Picture,
        size: (u32, u32),
        p: Params<'_>,
        neighbours: &[(f64, Picture)],
    ) -> Result<Picture, String> {
        let values = self.values_of(p);
        let wanted = input.len();
        // **The lease, held across both calls.** Telling the instance its values
        // and asking it for the picture are one indivisible pair: two frames of
        // one row are dispatched out of order by design, and an instance two of
        // them could both reach would hand this frame back painted with the
        // other frame's numbers, as an `Ok`, to be cached under this frame's own
        // key. The lease is also where the declared scratch is charged, so a
        // frame the ledger will not grant is refused before the plugin is
        // reached at all (§8).
        let mut lease = self
            .lease(inst, &values, wanted)
            .map_err(|error| error.to_string())?;
        lease.tell(&values).map_err(|error| error.to_string())?;
        let instance = lease.instance();
        let bounds = RectI::of(size.0, size.1);
        let job = ProcessJob {
            time: frame,
            bounds,
            // The whole of the buffer. A region of interest narrower than the
            // picture is the render pass's to ask for (§4.6); what this
            // seam promises either way is that a margin the plugin never wrote
            // comes back as the input.
            roi: bounds,
            input: &input,
            neighbours,
        };
        let rendering = self.host.process(instance, &job);
        // The answer is back, so the frame is over and the instance goes back
        // on the pool's idle list for the next frame of this row - before
        // `remember` below, which takes the pool's own table.
        drop(lease);
        if let Some(why) = rendering.error {
            return Err(why);
        }
        if rendering.pixels.depth() != input.depth() || rendering.pixels.len() != wanted {
            return Err(format!(
                "the plugin's frame was {} samples of {:?} where {wanted} of {:?} were asked for",
                rendering.pixels.len(),
                rendering.pixels.depth(),
                input.depth(),
            ));
        }
        // **Only a frame that came back says anything about the frames either
        // side.** A failure answers no offsets at all, and taking that as the
        // instance's new window would narrow the frame key on a bad frame and
        // retire every cached frame the good ones made.
        self.remember(inst, &rendering.frames_needed);
        Ok(rendering.pixels)
    }

    /// Keep what an instance's last render said it reads, for the key walk to
    /// ask about without talking to another process.
    fn remember(&self, inst: Uuid, offsets: &[i32]) {
        let mut wanted: Vec<i32> = offsets
            .iter()
            .map(|offset| offset.clamp(&-MAX_OFFSET, &MAX_OFFSET))
            .copied()
            .collect();
        wanted.push(0);
        wanted.sort_unstable();
        wanted.dedup();
        self.pool.remember(inst, wanted);
    }
}

impl EffectDef for LfxDef {
    fn schema(&self) -> &'static EffectSchema {
        self.schema
    }

    /// A stack built by hand - an oracle, a test - names no instance and no
    /// time. Both are legitimate: the host owns every value, so the bag alone
    /// is enough to render from, and the nil id is one instance like any other.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        self.apply_cpu_at(Uuid::nil(), 0.0, rgba, w, h, p);
    }

    fn apply_cpu_at(&self, inst: Uuid, lt: f64, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        self.apply_cpu_temporal(inst, lt, rgba, w, h, p, &[]);
    }

    fn apply_cpu_temporal(
        &self,
        inst: Uuid,
        lt: f64,
        rgba: &mut [f32],
        w: u32,
        h: u32,
        p: Params<'_>,
        neighbours: &[(i32, &[f32])],
    ) {
        let Some(wanted) = samples(w, h, rgba.len()) else {
            return;
        };
        let Some(input) = rgba.get(..wanted) else {
            return;
        };
        let frame = frame_in_bag(p, lt);
        // *ponytail:* every frame copies the picture and each neighbour into an
        // owned [`Picture`], and a plugin declaring the full window asks for up
        // to a hundred and twenty-eight of them - whole-frame allocations the
        // governor's ledger has never heard of, though §4.4 says a pooled
        // instance's slots are part of the ring's reservation and so refusable
        // by it. The way out is a borrowing `Picture` on [`ProcessJob`], which
        // §5.4's own *ponytail* already proposes for `Rendering::pixels`; until
        // then the ring copies these a second time on the way across.
        let beside: Vec<(f64, Picture)> = neighbours
            .iter()
            .filter(|(_, pixels)| pixels.len() == wanted)
            .map(|(offset, pixels)| (frame + f64::from(*offset), Picture::F32(pixels.to_vec())))
            .collect();
        let answer = self.render(
            inst,
            frame,
            Picture::F32(input.to_vec()),
            (w, h),
            p,
            &beside,
        );
        // **Identity, byte for byte.** A frame that did not come back leaves
        // `rgba` exactly as it was handed over - not the input written back,
        // which would put the picture through the fp16 boundary for nothing.
        let Some(Picture::F32(pixels)) = answer else {
            return;
        };
        if let Some(out) = rgba.get_mut(..wanted) {
            out.copy_from_slice(&pixels);
        }
    }

    /// The fp16 twin, and the one implementation of it in the tree (§4.5).
    ///
    /// It never answers `false`, on any road out. `false` means "use the f32
    /// path", and for a plugin the f32 path is *this same plugin* asked the same
    /// question a second time - an fp16 project would render every hosted frame
    /// twice, and a frame that failed would fail twice. A failure here is
    /// identity at this depth, which is what the halves already hold.
    ///
    /// **Including where there is no frame to send at all.** A zero-area or
    /// short buffer is not a picture anything can be asked about, and the answer
    /// is still `true`: the contract the hook's own doc comment states is that a
    /// definition answering `false` leaves `rgba` exactly as it found it, and
    /// leaving it exactly as it found it is all this case does. Answering
    /// `false` there would send the same halves down the f32 road to be widened
    /// for an effect that is this one.
    fn apply_f16_temporal(
        &self,
        inst: Uuid,
        lt: f64,
        rgba: &mut [half::f16],
        w: u32,
        h: u32,
        p: Params<'_>,
        neighbours: &[(i32, &[half::f16])],
    ) -> bool {
        let Some(wanted) = samples(w, h, rgba.len()) else {
            return true;
        };
        let Some(input) = rgba.get(..wanted) else {
            return true;
        };
        let frame = frame_in_bag(p, lt);
        // *ponytail:* the copy per neighbour, as in the f32 twin above.
        let beside: Vec<(f64, Picture)> = neighbours
            .iter()
            .filter(|(_, pixels)| pixels.len() == wanted)
            .map(|(offset, pixels)| (frame + f64::from(*offset), Picture::F16(pixels.to_vec())))
            .collect();
        let answer = self.render(
            inst,
            frame,
            Picture::F16(input.to_vec()),
            (w, h),
            p,
            &beside,
        );
        if let Some(Picture::F16(pixels)) = answer {
            if let Some(out) = rgba.get_mut(..wanted) {
                out.copy_from_slice(&pixels);
            }
        }
        true
    }

    fn frames_needed(&self, inst: &EffectInstance, _frame: f64) -> Option<Vec<i32>> {
        self.pool.frames_needed(inst.id)
    }

    /// Why the render this thread most recently ran was a placeholder, and
    /// nothing if it was not one.
    ///
    /// Taken on read, so one placeholder badges one frame; and cleared by the
    /// next frame that works, so a sentence nobody happened to read is gone by
    /// the time there is a good frame to mark.
    fn last_error(&self) -> Option<String> {
        LAST_ERROR.with(|slot| slot.borrow_mut().take())
    }

    fn hidden_rows(&self, _inst: &EffectInstance) -> Vec<&'static str> {
        self.hidden.clone()
    }

    /// The layer's frame, and nothing else.
    ///
    /// There is no `derived.memory` here and there never will be: D8 keeps
    /// `plugin_state` empty for every LFX instance, so the frame key is
    /// complete without one and a restart is an exact replay. This is also the
    /// one hook that can see the comp's rate, which is why the frame is minted
    /// here rather than read off the layer's seconds at the render.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let fps = cx
            .context
            .comp
            .and_then(|id| cx.context.document.comp(id))
            .map(|comp| comp.frame_rate.fps());
        if let Some(fps) = fps {
            push(DERIVED_FRAME, Value::Int((cx.lt * fps).round() as i32));
        }
    }

    /// Press one of this effect's `ACTION` rows.
    ///
    /// Version 1's frozen entry table is `init`, `destroy`, `describe`,
    /// `process` and `get_extension` - there is no press hook at all, so the
    /// broker acknowledges the message and nothing runs. What is still worth
    /// doing here is refusing a name that is not one of this effect's buttons,
    /// and handing the plugin the values the document holds first.
    ///
    /// Nothing comes back: an LFX plugin writes no rows and keeps no memory
    /// (D8), so [`Pressed`] is empty by construction rather than by accident.
    fn press(
        &self,
        inst: &EffectInstance,
        lt: f64,
        name: &str,
        _source: &PressFrame<'_>,
    ) -> Result<Pressed, String> {
        let Some(row) = self.actions.get(name) else {
            return Err(format!("the plugin has no button called {name}"));
        };
        let values = self.values_at(inst, lt);
        // The same lease a frame takes, and for the same reason: a press racing
        // a frame of the same row would otherwise press an instance with the
        // frame's values half applied. A press carries no picture, so it is
        // charged no scratch.
        let mut lease = self
            .lease(inst.id, &values, 0)
            .map_err(|error| error.to_string())?;
        lease.tell(&values).map_err(|error| error.to_string())?;
        self.host
            .press(lease.instance(), row)
            .map_err(|error| error.to_string())?;
        Ok(Pressed::default())
    }
}

impl Drop for LfxDef {
    /// A leaked definition never reaches this, which is the shipping case; a
    /// definition a test built does, and the instances it opened are the
    /// driver's to forget.
    fn drop(&mut self) {
        for id in self.pool.drain() {
            self.instances.close(id);
        }
    }
}

// ------------------------------------------------------------- the marshalling --

/// How many samples a picture of this size holds, or `None` where the buffer is
/// short or the size is empty - neither of which is a frame to send anywhere.
fn samples(w: u32, h: u32, len: usize) -> Option<usize> {
    let wanted = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    (wanted > 0 && len >= wanted).then_some(wanted)
}

/// The frame the plugin is told, out of the bag, or `lt` as it stands when no
/// comp put one there - which is a stack built by hand.
///
/// The fallback is worth naming, because it mixes two units on purpose.
/// [`LfxDef::resolve_derived`] pushes nothing when there is no comp to read a
/// rate off, so `lt` arrives in **seconds** and the neighbour times built from
/// it are seconds plus whole frame offsets. That state is the oracle and the
/// hand-built stack, where there is no decoded neighbour to be at the wrong time
/// and the offsets are nominal anyway; every comp puts the frame in the bag.
/// `lumit-ofx`'s own `frame_in_bag` falls back the same way for the same reason.
fn frame_in_bag(p: Params<'_>, lt: f64) -> f64 {
    match p.get(DERIVED_FRAME) {
        Some(Value::Int(frame)) => f64::from(frame),
        _ => lt,
    }
}

/// What the plugin declared every element of the dense array to be.
///
/// One entry per declaration that carries a value, in declaration order, which
/// is the order and the length [`ValueRoute::element`] counts in. An `ACTION`
/// carries none and takes no element, exactly as it appears in no bag.
///
/// **The elements are [`crate::schema::value_elements`]'s**, which is the same
/// walk and the same predicate [`value_routes`] numbers its elements by.
/// Answering "which declarations carry a value" a second time here is precisely
/// the two-lists-that-must-agree shape §4.1 item 3 replaced with one predicate:
/// a declaration one side kept and the other dropped would shift every element
/// after it, and the plugin would then read correct-looking kind tags over the
/// neighbouring row's value.
fn defaults_of(plugin: &PluginDescriptor, schema: &EffectSchema) -> Vec<ParamValue> {
    crate::schema::value_elements(plugin, schema)
        .into_iter()
        .filter_map(declared_default)
        .collect()
}

/// What one declaration says a fresh instance's value is, or `None` where the
/// declaration carries no value at all.
///
/// `None` is an `ACTION` and nothing else, which is
/// [`crate::schema::carriage`]'s own answer read from this side - the two are
/// held to it by `every_declaration_that_carries_a_value_has_a_default_to_carry`.
/// A variant added to [`Declared`] fails the `match` below, and its author has
/// to decide here and there at once.
fn declared_default(declaration: &Declaration) -> Option<ParamValue> {
    match &declaration.kind {
        Declared::Float { default, .. } => Some(ParamValue::Float(*default)),
        Declared::Slider { default, .. } => Some(ParamValue::Slider(*default)),
        Declared::Angle { default, .. } => Some(ParamValue::Angle(*default)),
        Declared::Int { default, .. } => Some(ParamValue::Int(*default)),
        // The header gives a Seed no default: the host draws one from the
        // instance's own id, and the bag is where it arrives from.
        Declared::Seed => Some(ParamValue::Seed(0)),
        Declared::Bool { default } => Some(ParamValue::Bool(*default)),
        Declared::Choice { default, .. } => Some(ParamValue::Choice(*default)),
        Declared::Colour { default, .. } => Some(ParamValue::Colour([
            default[0] as f32,
            default[1] as f32,
            default[2] as f32,
            default[3] as f32,
        ])),
        Declared::Point2 { default, .. } => {
            Some(ParamValue::Point2([default.0 as f32, default.1 as f32]))
        }
        Declared::Point3 { default, .. } => Some(ParamValue::Point3([
            default.0 as f32,
            default.1 as f32,
            default.2 as f32,
        ])),
        Declared::Curve { default } => Some(ParamValue::Curve(default.clone())),
        // NULL until the render pass's generic file aux lands, which is the *ponytail*
        // beside the constant in `lfx.h`: the payload rides beside the op
        // and only the render knows which file actually opened.
        Declared::File { .. } => Some(ParamValue::File(None)),
        Declared::Action => None,
    }
}

/// The row ids of this plugin's buttons, in the schema's own `&'static str`s.
fn action_rows(plugin: &PluginDescriptor, schema: &EffectSchema) -> BTreeSet<&'static str> {
    let wanted: BTreeSet<&str> = plugin
        .params
        .iter()
        .filter(|declaration| matches!(declaration.kind, Declared::Action))
        .map(|declaration| declaration.id.as_str())
        .collect();
    schema
        .params
        .iter()
        .filter(|row| wanted.contains(row.id))
        .map(|row| row.id)
        .collect()
}

/// Write one resolved row into the element its declaration owns.
///
/// Anything that does not fit the shape is left alone, which leaves the
/// plugin's own default standing - a wrong kind is never a fault
/// (docs/14 §4). The `component` is which axis of a point this row is, and
/// nought for the scalar kinds, which is most of them.
fn write_component(slot: &mut ParamValue, component: usize, value: Value) {
    let number = match value {
        Value::Float(number) => Some(f64::from(number)),
        Value::Int(number) => Some(f64::from(number)),
        Value::Bool(switch) => Some(f64::from(u8::from(switch))),
        Value::Choice(chosen) => Some(f64::from(chosen)),
        _ => None,
    };
    match (slot, value) {
        (ParamValue::Float(into), _) => write_number(into, number),
        (ParamValue::Slider(into), _) => write_number(into, number),
        (ParamValue::Angle(into), _) => write_number(into, number),
        (ParamValue::Int(into), _) | (ParamValue::Seed(into), _) => {
            if let Some(number) = number {
                *into = number.round() as i64;
            }
        }
        (ParamValue::Bool(into), _) => {
            if let Some(number) = number {
                *into = number != 0.0;
            }
        }
        (ParamValue::Choice(into), _) => {
            if let Some(number) = number {
                *into = number.round().clamp(0.0, f64::from(u32::MAX)) as u32;
            }
        }
        (ParamValue::Colour(into), Value::Colour(rgba) | Value::Vec4(rgba)) => *into = rgba,
        (ParamValue::Point2(into), _) => {
            if let (Some(axis), Some(number)) = (into.get_mut(component), number) {
                *axis = number as f32;
            }
        }
        (ParamValue::Point3(into), _) => {
            if let (Some(axis), Some(number)) = (into.get_mut(component), number) {
                *axis = number as f32;
            }
        }
        (ParamValue::Curve(into), Value::Curve(points)) => *into = points.points().to_vec(),
        // A `FILE` row's path comes from the auxiliary slot beside the op and
        // never from the bag, which carries nothing at all for one.
        _ => {}
    }
}

/// Put a number in a floating-point element, leaving it alone where the row
/// carried something that is not one.
fn write_number(into: &mut f64, number: Option<f64>) {
    if let Some(number) = number {
        *into = number;
    }
}

/// One stored row's value at `lt`, as the bag would have carried it.
///
/// The references - a layer, a clip, a mask path - and a file are not values an
/// out-of-process plugin holds, and LFX mints no row of any of those kinds; a
/// `None` here leaves the plugin's own declared default standing.
///
/// **"As the bag would have carried it" is the whole rule**, so a seed is
/// reinterpreted rather than clamped: `EffectValue::Seed` is a `u32` minted
/// across its whole range by `fx::builtins::fresh_seed`, and
/// `fx::resolved`'s own walk carries it as `Value::Int(*s as i32)`. A clamp
/// here would tell the plugin `i32::MAX` where every frame of the same row
/// tells it the wrapped number, for roughly half of all fresh instances - two
/// seeds written into one live instance, and nothing in the document to say
/// which the picture was made with.
fn stored(value: &EffectValue, lt: f64) -> Option<Value> {
    Some(match value {
        EffectValue::Float(property) => Value::Float(property.value_at(lt) as f32),
        EffectValue::Colour(channels) => {
            let at = |index: usize| channels.get(index).map_or(0.0, |c| c.value_at(lt) as f32);
            Value::Colour([at(0), at(1), at(2), at(3)])
        }
        EffectValue::Bool(switch) => Value::Bool(*switch),
        EffectValue::Choice(chosen) => Value::Choice(*chosen),
        EffectValue::Seed(seed) => Value::Int(*seed as i32),
        EffectValue::Curve(points) => Value::Curve(CurvePoints::sanitised(points)),
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use half::f16;
    use lumit_budget::{Ledger, Tier};
    use lumit_core::anim::Property;
    use lumit_core::fx::Unit;
    use lumit_core::model::{EffectKey, EffectNamespace, EffectParam};
    use lumit_lfx_abi::{LFX_CATEGORY_BLUR_SHARPEN, LFX_TRAIT_THREAD_UNSAFE};

    use crate::describe::{Declaration, Describe, Identity, Traits};
    use crate::discover::gate_lock;
    use crate::ipc::proto::PixelDepth;
    use crate::schema::schema_of;

    use super::*;

    /// What one frame's driver was asked for.
    #[derive(Clone, Debug)]
    struct Seen {
        instance: InstanceId,
        time: f64,
        input: Picture,
        neighbours: Vec<f64>,
        /// What the driver held for that instance at the moment it was asked -
        /// the values the plugin would have read, rather than the values the
        /// caller meant to send.
        values: Vec<ParamValue>,
    }

    /// What the fake driver answers a frame with.
    #[derive(Clone, Debug)]
    enum Answer {
        /// Every sample painted, at the depth the frame arrived in.
        Paint(f32),
        /// A sentence and no picture.
        Fail(String),
        /// A picture of the wrong length, which is what a misbehaving plugin
        /// looks like from here.
        Short,
        /// A picture of exactly the right length at the **other** depth, which
        /// is what a broker reading the wrong slot would serve.
        WrongDepth,
        /// Every sample painted with the **instance's own first value**, which
        /// is the Full personality's trick read into the fake: a frame handed
        /// back painted with another frame's numbers is then visible in the
        /// picture rather than only in what the driver recorded.
        PaintTheValue,
    }

    /// A rendezvous the fixture uses to make an overlap deliberate rather than
    /// hoped for (§11 item 12): a call into the driver announces itself and
    /// waits here until the case says the other thread has arrived.
    #[derive(Default)]
    struct Gate {
        open: Mutex<bool>,
        changed: std::sync::Condvar,
    }

    impl Gate {
        /// Wait until the case opens it. Returns at once once it is open, so a
        /// second caller is never held.
        fn wait(&self) {
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.changed.wait(open).unwrap();
            }
        }

        /// Let everything through, now and afterwards.
        fn open(&self) {
            *self.open.lock().unwrap() = true;
            self.changed.notify_all();
        }
    }

    /// A rendezvous that makes concurrency **recorded** rather than hoped for
    /// (§11 item 12), and does it without a sleep that could go either way.
    ///
    /// Each call into the driver announces itself, notes the high-water mark,
    /// and then waits for a second caller to arrive - for a bounded time. Two
    /// frames the pool let through together meet here at once and both go on
    /// immediately, and [`Overlap::most`] reads two. A pool that let only one
    /// through waits out the deadline and goes on alone, and `most` reads one.
    /// So the same fixture proves the overlap where it is allowed and its
    /// absence where it is not, which is what makes the absence mean something.
    ///
    /// **The deadline is the caller's**, because the two runs want opposite
    /// numbers from it. A run asserted to overlap never spends its wait - the
    /// company is coming - so it can afford to be [`Overlap::PATIENT`], and a
    /// patient one cannot be failed by a loaded machine that took a moment to
    /// schedule the second thread. A run asserted *not* to overlap spends its
    /// wait once per frame and gains nothing by waiting longer, since nothing
    /// is coming: [`Overlap::BRIEF`] is what the case's own running time costs.
    struct Overlap {
        inside: Mutex<(usize, usize)>,
        arrived: std::sync::Condvar,
        patience: std::time::Duration,
    }

    impl Overlap {
        /// What a run that is **asserted to overlap** waits. Long enough that a
        /// second thread which has been spawned and not yet scheduled - on a
        /// machine compiling something else - is still waited for, and never
        /// actually spent, because the second caller arrives.
        const PATIENT: std::time::Duration = std::time::Duration::from_secs(5);

        /// What a run that is **asserted not to overlap** waits, once per
        /// frame. Nothing is coming; the four threads are already spawned and a
        /// pool that let two through would have them inside within
        /// microseconds, so the only thing a longer wait buys is a slower case.
        const BRIEF: std::time::Duration = std::time::Duration::from_millis(100);

        fn within(patience: std::time::Duration) -> Self {
            Self {
                inside: Mutex::new((0, 0)),
                arrived: std::sync::Condvar::new(),
                patience,
            }
        }

        fn enter(&self) {
            let mut inside = self.inside.lock().unwrap();
            inside.0 += 1;
            inside.1 = inside.1.max(inside.0);
            self.arrived.notify_all();
            let deadline = std::time::Instant::now() + self.patience;
            // Only while the overlap this run is about has not been recorded
            // yet. Once two callers have met, the fixture has its evidence and
            // a frame that happens to arrive alone afterwards - the tail of a
            // run of eight - has nothing to wait for and does not pay for it.
            while inside.0 < 2 && inside.1 < 2 {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    break;
                }
                let (held, _timeout) = self.arrived.wait_timeout(inside, left).unwrap();
                inside = held;
            }
        }

        fn leave(&self) {
            let mut inside = self.inside.lock().unwrap();
            inside.0 -= 1;
        }

        /// The most callers that were ever inside at once.
        fn most(&self) -> usize {
            self.inside.lock().unwrap().1
        }
    }

    /// A driver that records what reached it and answers what it is told to.
    struct Fake {
        opened: Mutex<Vec<Vec<ParamValue>>>,
        updated: Mutex<Vec<(InstanceId, Vec<ParamValue>)>>,
        closed: Mutex<Vec<InstanceId>>,
        pressed: Mutex<Vec<(InstanceId, String)>>,
        seen: Mutex<Vec<Seen>>,
        answer: Mutex<Answer>,
        offsets: Mutex<Vec<i32>>,
        next: AtomicUsize,
        /// What the driver currently holds for each instance - the record two
        /// frames of one row would overwrite for each other if the definition
        /// let go between telling an instance its values and asking it for a
        /// picture.
        held: Mutex<BTreeMap<InstanceId, Vec<ParamValue>>>,
        /// Armed by the concurrency case: `update` announces itself here.
        told: Mutex<Option<std::sync::mpsc::Sender<()>>>,
        /// Armed by the concurrency case: `process` announces itself here and
        /// then waits on the gate.
        rendering: Mutex<Option<(std::sync::mpsc::Sender<()>, Arc<Gate>)>>,
        /// Armed by the pool cases: every `process` passes through this, and
        /// what it records is how many were ever inside at once.
        overlap: Mutex<Option<Arc<Overlap>>>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Self {
                opened: Mutex::new(Vec::new()),
                updated: Mutex::new(Vec::new()),
                closed: Mutex::new(Vec::new()),
                pressed: Mutex::new(Vec::new()),
                seen: Mutex::new(Vec::new()),
                answer: Mutex::new(Answer::Paint(1.0)),
                offsets: Mutex::new(Vec::new()),
                next: AtomicUsize::new(1),
                held: Mutex::new(BTreeMap::new()),
                told: Mutex::new(None),
                rendering: Mutex::new(None),
                overlap: Mutex::new(None),
            }
        }
    }

    impl Fake {
        fn answers(&self, answer: Answer) {
            *self.answer.lock().unwrap() = answer;
        }

        fn asks_for(&self, offsets: &[i32]) {
            *self.offsets.lock().unwrap() = offsets.to_vec();
        }

        fn opens(&self) -> Vec<Vec<ParamValue>> {
            self.opened.lock().unwrap().clone()
        }

        fn frames(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }

        /// Record how many frames are ever inside the driver at once, for a
        /// case that expects them to overlap.
        fn records_overlap(&self) -> Arc<Overlap> {
            self.records_overlap_within(Overlap::PATIENT)
        }

        /// The same, where the case chooses how long a lone caller waits for
        /// company before deciding there is none.
        fn records_overlap_within(&self, patience: std::time::Duration) -> Arc<Overlap> {
            let overlap = Arc::new(Overlap::within(patience));
            *self.overlap.lock().unwrap() = Some(Arc::clone(&overlap));
            overlap
        }

        /// Stop recording, so a run of frames the case is not asking about does
        /// not each wait out the recorder's patience alone.
        fn stops_recording(&self) {
            *self.overlap.lock().unwrap() = None;
        }

        /// What the driver holds for this instance right now.
        fn holds(&self, instance: InstanceId) -> Vec<ParamValue> {
            self.held
                .lock()
                .unwrap()
                .get(&instance)
                .cloned()
                .unwrap_or_default()
        }
    }

    impl LfxInstances for Fake {
        fn open(&self, values: Vec<ParamValue>) -> Result<InstanceId, BrokerError> {
            let id = self.next.fetch_add(1, Ordering::SeqCst) as InstanceId;
            self.held.lock().unwrap().insert(id, values.clone());
            self.opened.lock().unwrap().push(values);
            Ok(id)
        }

        fn update(&self, instance: InstanceId, values: Vec<ParamValue>) -> Result<(), BrokerError> {
            let announce = self.told.lock().unwrap().clone();
            if let Some(announce) = announce {
                let _ = announce.send(());
            }
            self.held.lock().unwrap().insert(instance, values.clone());
            self.updated.lock().unwrap().push((instance, values));
            Ok(())
        }

        fn close(&self, instance: InstanceId) {
            self.closed.lock().unwrap().push(instance);
        }
    }

    impl LfxHost for Fake {
        fn process(&self, instance: InstanceId, job: &ProcessJob<'_>) -> Rendering {
            let overlap = self.overlap.lock().unwrap().clone();
            if let Some(overlap) = overlap.as_ref() {
                overlap.enter();
            }
            let answer = self.render(instance, job);
            if let Some(overlap) = overlap.as_ref() {
                overlap.leave();
            }
            answer
        }

        fn press(&self, instance: InstanceId, param: &str) -> Result<(), BrokerError> {
            self.pressed
                .lock()
                .unwrap()
                .push((instance, param.to_owned()));
            Ok(())
        }
    }

    impl Fake {
        /// What the driver answers a frame with, once the overlap recorder has
        /// had its say.
        fn render(&self, instance: InstanceId, job: &ProcessJob<'_>) -> Rendering {
            let waiting = self.rendering.lock().unwrap().clone();
            if let Some((announce, gate)) = waiting {
                let _ = announce.send(());
                gate.wait();
            }
            self.seen.lock().unwrap().push(Seen {
                instance,
                time: job.time,
                input: job.input.clone(),
                neighbours: job.neighbours.iter().map(|(time, _)| *time).collect(),
                values: self
                    .held
                    .lock()
                    .unwrap()
                    .get(&instance)
                    .cloned()
                    .unwrap_or_default(),
            });
            let frames_needed = self.offsets.lock().unwrap().clone();
            let answer = self.answer.lock().unwrap().clone();
            let pixels = match answer {
                Answer::Paint(level) => match job.input {
                    Picture::F16(halves) => Picture::F16(vec![f16::from_f32(level); halves.len()]),
                    Picture::F32(whole) => Picture::F32(vec![level; whole.len()]),
                },
                Answer::Fail(why) => {
                    return Rendering {
                        pixels: job.input.clone(),
                        frames_needed,
                        error: Some(why),
                    }
                }
                Answer::Short => Picture::F32(vec![0.0; 1]),
                Answer::WrongDepth => match job.input {
                    Picture::F16(halves) => Picture::F32(vec![0.0; halves.len()]),
                    Picture::F32(whole) => Picture::F16(vec![f16::from_f32(0.0); whole.len()]),
                },
                Answer::PaintTheValue => {
                    // The instance's own first control, read off what the driver
                    // holds for **this** instance rather than off the job, so a
                    // frame painted with another frame's numbers shows up in the
                    // picture.
                    let level = match self.holds(instance).first() {
                        Some(ParamValue::Float(number)) => *number as f32,
                        _ => 0.0,
                    };
                    match job.input {
                        Picture::F16(halves) => {
                            Picture::F16(vec![f16::from_f32(level); halves.len()])
                        }
                        Picture::F32(whole) => Picture::F32(vec![level; whole.len()]),
                    }
                }
            };
            Rendering {
                pixels,
                frames_needed,
                error: None,
            }
        }
    }

    fn identity(id: &str) -> Identity {
        Identity {
            id: id.to_owned(),
            name: "Example blur".to_owned(),
            vendor: "Example".to_owned(),
            major: 1,
            minor: 2,
            patch: 3,
            categories: vec![LFX_CATEGORY_BLUR_SHARPEN],
            traits: None,
            required_extensions: Vec::new(),
        }
    }

    fn float(id: &str, default: f64) -> Declaration {
        Declaration {
            id: id.to_owned(),
            label: id.to_owned(),
            unit: Unit::Px,
            flags: 0,
            kind: Declared::Float {
                default,
                slider: (0.0, 100.0),
                hard: (None, None),
            },
        }
    }

    fn plain(id: &str, kind: Declared) -> Declaration {
        Declaration {
            id: id.to_owned(),
            label: id.to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind,
        }
    }

    /// The panel every case below is about: a float, a dropdown, a point, a
    /// switch and a button, in that order.
    fn described(id: &str) -> PluginDescriptor {
        let mut sink = Describe::new();
        assert!(sink.declare(float("radius", 3.5)));
        assert!(sink.declare(plain(
            "mode",
            Declared::Choice {
                options: vec!["Box".to_owned(), "Gaussian".to_owned()],
                default: 0,
                dividers_after: Vec::new(),
            }
        )));
        assert!(sink.declare(Declaration {
            id: "centre".to_owned(),
            label: "Centre".to_owned(),
            unit: Unit::Px,
            flags: 0,
            kind: Declared::Point2 {
                default: (0.5, 0.5),
                slider: (0.0, 1.0),
            },
        }));
        assert!(sink.declare(plain("invert", Declared::Bool { default: false })));
        assert!(sink.declare(plain("analyse", Declared::Action)));
        let described = sink.finish().expect("nothing structural happened");
        PluginDescriptor::new(identity(id), described)
    }

    /// A ledger with room for anything and a ring wide enough that neither is
    /// the ceiling under test - what a case that is not about §4.4 wants.
    fn an_open_policy(descriptor: &PluginDescriptor) -> Policy {
        Policy::declared(
            descriptor.identity.traits.as_ref(),
            &a_ring(u32::MAX),
            &Ledger::new(),
            &a_bundle(std::slice::from_ref(descriptor)),
        )
    }

    /// The same ceilings, with the ring narrowed to the floor the ledger
    /// leaves - which pins the pool to one instance per row (`Policy::ceiling`
    /// reads a ring as the frames it carries in flight), so two frames of one
    /// row must take turns rather than opening beside each other.
    ///
    /// The bundle's lock would pin it too, and would pin it for a different
    /// reason: a case that armed `lfx.thread-unsafe` would be about the lock a
    /// thread-unsafe declaration buys rather than about the queue every
    /// ordinary plugin's frames meet.
    fn a_pinned_policy(descriptor: &PluginDescriptor) -> Policy {
        Policy::declared(
            descriptor.identity.traits.as_ref(),
            &a_ring(crate::ipc::ring::RING_MIN_SLOTS),
            &Ledger::new(),
            &a_bundle(std::slice::from_ref(descriptor)),
        )
    }

    /// A ring of this many slots, as the bundle's broker publishes it.
    fn a_ring(slots: u32) -> crate::ipc::ring::RingSlots {
        crate::ipc::ring::RingSlots::of(slots)
    }

    /// The bundle's own lock, armed from every plugin in it - which is how
    /// `LfxDef::hosted` builds it, and the only way a bundle where one plugin
    /// declares `lfx.thread-unsafe` serialises the rest of them (§2.6).
    fn a_bundle(plugins: &[PluginDescriptor]) -> Serial {
        Serial::for_bundle(plugins.iter().map(|plugin| {
            plugin
                .identity
                .traits
                .as_ref()
                .map_or(0, |traits| traits.flags)
        }))
    }

    /// The descriptor, its leaked schema and a fake driving both seams.
    fn definition(descriptor: &PluginDescriptor) -> (LfxDef, Arc<Fake>) {
        let fake = Arc::new(Fake::default());
        let def = over(descriptor, an_open_policy(descriptor), &fake);
        (def, fake)
    }

    /// The same, under a policy the case chose and over a driver it holds - so
    /// two plugins of one bundle can share the one driver their frames are
    /// counted through.
    fn over(descriptor: &PluginDescriptor, policy: Policy, fake: &Arc<Fake>) -> LfxDef {
        let schema = Box::leak(Box::new(schema_of(descriptor).expect("it is an effect")));
        LfxDef::new(
            descriptor,
            schema,
            Arc::clone(fake) as Arc<dyn LfxInstances>,
            Arc::clone(fake) as Arc<dyn LfxHost>,
            policy,
        )
    }

    /// The same panel, with a trait block the case wrote.
    fn declaring(id: &str, traits: Traits) -> PluginDescriptor {
        let mut descriptor = described(id);
        descriptor.identity.traits = Some(traits);
        descriptor
    }

    /// One frame of `row`, whose one control carries the frame's own number, and
    /// the picture that came back read as bits.
    ///
    /// Bits rather than values, so a frame painted with another frame's numbers
    /// cannot pass for one painted with its own.
    fn one_frame(def: &LfxDef, row: Uuid, frame: u32) -> Vec<u32> {
        let bag = [(ParamId::new("radius"), Value::Float(frame as f32))];
        let mut rgba = a_picture();
        def.apply_cpu_at(row, f64::from(frame), &mut rgba, 2, 2, Params::new(&bag));
        rgba.iter().map(|sample| sample.to_bits()).collect()
    }

    /// The same, with the per-render disable gate between the definition and
    /// the driver - the shipping arrangement, minus the second process.
    fn gated(descriptor: &PluginDescriptor) -> (LfxDef, Arc<Fake>) {
        let schema = Box::leak(Box::new(schema_of(descriptor).expect("it is an effect")));
        let fake = Arc::new(Fake::default());
        let gate: Arc<dyn LfxHost> = Arc::new(Gated::new(
            descriptor.identity.id.clone(),
            Arc::clone(&fake) as Arc<dyn LfxHost>,
        ));
        let def = LfxDef::new(
            descriptor,
            schema,
            Arc::clone(&fake) as Arc<dyn LfxInstances>,
            gate,
            an_open_policy(descriptor),
        );
        (def, fake)
    }

    /// A driver that switches the plugin off as it hands the instance back -
    /// the tick that lands in the one window between `LfxDef::lease`'s own read
    /// of the switched-off list and the frame going across.
    struct SwitchesOffOnOpen {
        identifier: String,
        inner: Arc<Fake>,
    }

    impl LfxInstances for SwitchesOffOnOpen {
        fn open(&self, values: Vec<ParamValue>) -> Result<InstanceId, BrokerError> {
            let id = self.inner.open(values)?;
            crate::discover::set_enabled(&self.identifier, false);
            Ok(id)
        }

        fn update(&self, instance: InstanceId, values: Vec<ParamValue>) -> Result<(), BrokerError> {
            self.inner.update(instance, values)
        }

        fn close(&self, instance: InstanceId) {
            self.inner.close(instance);
        }
    }

    /// One declaration of every kind the frozen sink admits, with the unit each
    /// kind is actually in so the sweep meets no report line on its way.
    fn every_kind() -> Vec<(&'static str, Unit, Declared)> {
        vec![
            (
                "f",
                Unit::Px,
                Declared::Float {
                    default: 1.0,
                    slider: (0.0, 2.0),
                    hard: (None, None),
                },
            ),
            (
                "s",
                Unit::Px,
                Declared::Slider {
                    default: 1.0,
                    range: (0.0, 2.0),
                    log: false,
                },
            ),
            (
                "i",
                Unit::Px,
                Declared::Int {
                    default: 1,
                    slider: (0, 2),
                    hard: (None, None),
                },
            ),
            (
                "a",
                Unit::Degrees,
                Declared::Angle {
                    default: 0.0,
                    dial_step: 15.0,
                },
            ),
            ("b", Unit::Raw, Declared::Bool { default: true }),
            (
                "c",
                Unit::Raw,
                Declared::Choice {
                    options: vec!["One".to_owned()],
                    default: 0,
                    dividers_after: Vec::new(),
                },
            ),
            (
                "col",
                Unit::Raw,
                Declared::Colour {
                    default: [0.0; 4],
                    range: (0.0, 1.0),
                },
            ),
            ("seed", Unit::Raw, Declared::Seed),
            (
                "p2",
                Unit::Px,
                Declared::Point2 {
                    default: (0.0, 0.0),
                    slider: (0.0, 1.0),
                },
            ),
            (
                "p3",
                Unit::Px,
                Declared::Point3 {
                    default: (0.0, 0.0, 0.0),
                    slider: (0.0, 1.0),
                },
            ),
            (
                "curve",
                Unit::Raw,
                Declared::Curve {
                    default: vec![[0.0, 0.0], [1.0, 1.0]],
                },
            ),
            (
                "file",
                Unit::Raw,
                Declared::File {
                    filter: vec!["cube".to_owned()],
                    filter_name: "Cube".to_owned(),
                },
            ),
            ("act", Unit::Raw, Declared::Action),
        ]
    }

    /// A four-pixel picture nothing in it repeats, so a frame that came back
    /// changed cannot look unchanged.
    fn a_picture() -> Vec<f32> {
        (0..16).map(|sample| sample as f32 / 16.0).collect()
    }

    fn an_instance(effect: &LfxDef) -> EffectInstance {
        EffectInstance {
            id: Uuid::now_v7(),
            effect: EffectKey {
                namespace: EffectNamespace::Lfx,
                match_name: effect.schema().match_name.to_owned(),
                version: effect.schema().version,
                extra: serde_json::Map::new(),
            },
            enabled: true,
            params: Vec::new(),
            sample_temporally: true,
            custom_name: None,
            linked_pairs: Vec::new(),
            plugin_state: None,
            roto: None,
            extra: serde_json::Map::new(),
        }
    }

    /// The resolved bag becomes the dense array the plugin reads: one element
    /// per declaration that carries a value, in declaration order, a point
    /// folded back into the one element it was spread from, and a button taking
    /// none at all (§2.2).
    #[test]
    fn a_resolved_bag_becomes_the_dense_value_array_the_plugin_reads() {
        let descriptor = described("com.example.dense");
        let (def, fake) = definition(&descriptor);
        let bag = [
            (ParamId::new("radius"), Value::Float(12.5)),
            (ParamId::new("mode"), Value::Choice(1)),
            (ParamId::new("centre_x"), Value::Float(0.25)),
            (ParamId::new("centre_y"), Value::Float(0.75)),
            (ParamId::new("invert"), Value::Bool(true)),
        ];
        let mut rgba = a_picture();
        def.apply_cpu_at(Uuid::nil(), 0.0, &mut rgba, 2, 2, Params::new(&bag));

        assert_eq!(
            fake.opens(),
            vec![vec![
                ParamValue::Float(12.5),
                ParamValue::Choice(1),
                ParamValue::Point2([0.25, 0.75]),
                ParamValue::Bool(true),
            ]],
            "the button is not an element, and the point is one"
        );
    }

    /// A row the bag carries nothing for keeps what the plugin declared, which
    /// is the whole of what a `FILE` row has until the render pass's generic file aux
    /// lands (§2.3).
    #[test]
    fn a_row_the_bag_has_no_value_for_keeps_the_plugins_own_default() {
        let mut sink = Describe::new();
        assert!(sink.declare(float("radius", 3.5)));
        assert!(sink.declare(Declaration {
            id: "lut".to_owned(),
            label: "Lut".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::File {
                filter: vec!["cube".to_owned()],
                filter_name: "Cube".to_owned(),
            },
        }));
        let described = sink.finish().expect("nothing structural happened");
        let descriptor = PluginDescriptor::new(identity("com.example.defaults"), described);
        let (def, fake) = definition(&descriptor);

        let mut rgba = a_picture();
        def.apply_cpu_at(Uuid::nil(), 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(
            fake.opens(),
            vec![vec![ParamValue::Float(3.5), ParamValue::File(None)]],
            "the plugin's own numbers, and a path that is NULL rather than empty"
        );
    }

    /// §14 item 7's other half, the one the in-process host left here: a plugin that writes
    /// half the output and then fails leaves the **caller's** buffer exactly as
    /// it found it. Compared by bits, since the input written back would go
    /// through a depth boundary and change very slightly.
    #[test]
    fn a_failed_frame_leaves_the_picture_exactly_as_it_found_it() {
        let descriptor = described("com.example.failing");
        let (def, fake) = definition(&descriptor);
        fake.answers(Answer::Fail("the plugin fell over".to_owned()));

        let before = a_picture();
        let mut rgba = before.clone();
        def.apply_cpu_at(Uuid::nil(), 0.0, &mut rgba, 2, 2, Params::EMPTY);

        let bits: Vec<u32> = rgba.iter().map(|sample| sample.to_bits()).collect();
        let was: Vec<u32> = before.iter().map(|sample| sample.to_bits()).collect();
        assert_eq!(bits, was, "identity, byte for byte");
        assert_eq!(
            def.last_error().as_deref(),
            Some("the plugin fell over"),
            "and the layer wears the plugin's own sentence"
        );
    }

    /// A frame of the wrong shape is a badge rather than a picture: the host
    /// holds what comes back to the depth **and** the count it asked for, and
    /// anything else leaves the caller's buffer alone.
    ///
    /// Both arms, because they fail differently: a short answer is a plugin
    /// that wrote the wrong amount, and an answer of exactly the right length
    /// at the other depth is what a broker reading the wrong slot would serve.
    /// The sentence is compared whole, so a badge that named one of the two
    /// numbers twice would not pass for one that named both.
    #[test]
    fn a_frame_that_is_not_the_one_asked_for_is_a_badge_rather_than_a_picture() {
        let descriptor = described("com.example.short");
        let (def, fake) = definition(&descriptor);
        fake.answers(Answer::Short);

        let before = a_picture();
        let mut rgba = before.clone();
        def.apply_cpu_at(Uuid::nil(), 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(rgba, before);
        assert_eq!(
            def.last_error().as_deref(),
            Some("the plugin's frame was 1 samples of F32 where 16 of F32 were asked for"),
            "the count that came back and the count that was asked for, both named"
        );

        fake.answers(Answer::WrongDepth);
        def.apply_cpu_at(Uuid::nil(), 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(rgba, before, "the right number of samples is not the frame");
        assert_eq!(
            def.last_error().as_deref(),
            Some("the plugin's frame was 16 samples of F16 where 16 of F32 were asked for"),
            "the depth arm, which no count can reach"
        );

        let mut halves: Vec<f16> = before.iter().map(|sample| f16::from_f32(*sample)).collect();
        let was: Vec<u16> = halves.iter().map(|half| half.to_bits()).collect();
        let took = def.apply_f16_temporal(Uuid::nil(), 0.0, &mut halves, 2, 2, Params::EMPTY, &[]);
        assert!(took, "and the fp16 road still never declines");
        assert_eq!(
            halves.iter().map(|half| half.to_bits()).collect::<Vec<_>>(),
            was,
            "identity at this depth, byte for byte"
        );
        assert_eq!(
            def.last_error().as_deref(),
            Some("the plugin's frame was 16 samples of F32 where 16 of F16 were asked for"),
        );
    }

    /// The badge is **taken** on read, so one placeholder marks one frame
    /// (§4.2).
    ///
    /// This is half the promise and the half a read can keep by itself. The
    /// other half - a frame that works clearing what the frame before it filed,
    /// with nobody reading in between - is
    /// `a_good_frame_clears_the_reason_the_frame_before_it_filed` below, and
    /// deliberately not asserted here: a case that reads twice before it
    /// renders drains the slot with the second read, and would pass over a
    /// definition that never cleared anything at all.
    #[test]
    fn a_badge_is_taken_on_read() {
        let descriptor = described("com.example.taken");
        let (def, _fake) = definition(&descriptor);

        let mut rgba = a_picture();
        def.apply_cpu_at(Uuid::nil(), 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(def.last_error(), None, "a frame that worked says nothing");

        let (def, fake) = definition(&described("com.example.taken.twice"));
        fake.answers(Answer::Fail("once".to_owned()));
        def.apply_cpu_at(Uuid::nil(), 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(def.last_error().as_deref(), Some("once"));
        assert_eq!(def.last_error(), None, "taken, not left behind");
    }

    /// A reason filed against one frame cannot mark a later one that went
    /// perfectly well - **without anybody reading it in between** (§4.2).
    ///
    /// The read is what the dispatch seam does for a frame it already knows is
    /// a placeholder; it is not something a good frame does, and the slot is
    /// one per thread rather than one per definition. So the two definitions
    /// here are the shape the shipping path really has: one render worker,
    /// a failure of one plugin nothing asked about, and the next plugin's good
    /// frame on the same thread. Take the clearing out of `LfxDef::render` and
    /// this case reports the first plugin's sentence under the second plugin's
    /// layer.
    #[test]
    fn a_good_frame_clears_the_reason_the_frame_before_it_filed() {
        let (failing, broken) = definition(&described("com.example.filed"));
        broken.answers(Answer::Fail("the plugin broker did not answer".to_owned()));
        let mut rgba = a_picture();
        failing.apply_cpu_at(Uuid::nil(), 0.0, &mut rgba, 2, 2, Params::EMPTY);

        let (def, fake) = definition(&described("com.example.clean"));
        fake.answers(Answer::Paint(1.0));
        let mut rgba = a_picture();
        def.apply_cpu_at(Uuid::nil(), 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(rgba, vec![1.0; 16], "the second frame is the plugin's work");
        assert_eq!(
            def.last_error(),
            None,
            "a frame that worked wears nobody else's sentence"
        );
        assert_eq!(
            failing.last_error(),
            None,
            "and the definition that filed it has nothing left to hand back either"
        );
    }

    /// Both depths reach the plugin as themselves. The fp16 hook is the one
    /// implementation of `apply_f16_temporal` in the tree and it never declines -
    /// a `false` would send the same frame down the f32 road and render it
    /// twice (§4.5).
    #[test]
    fn both_depths_reach_the_plugin_at_the_depth_they_arrived_in() {
        let descriptor = described("com.example.depths");
        let (def, fake) = definition(&descriptor);

        let mut whole = a_picture();
        def.apply_cpu_at(Uuid::nil(), 0.0, &mut whole, 2, 2, Params::EMPTY);

        let mut halves: Vec<f16> = a_picture().iter().map(|s| f16::from_f32(*s)).collect();
        let took = def.apply_f16_temporal(Uuid::nil(), 0.0, &mut halves, 2, 2, Params::EMPTY, &[]);
        assert!(took, "the fp16 path is this definition's own");
        assert!(halves.iter().all(|half| *half == f16::from_f32(1.0)));

        let depths: Vec<PixelDepth> = fake
            .frames()
            .iter()
            .map(|frame| frame.input.depth())
            .collect();
        assert_eq!(
            depths,
            vec![PixelDepth::F32, PixelDepth::F16],
            "neither frame was converted on the way to the plugin"
        );
    }

    /// And a failed fp16 frame is identity **at this depth**, still answering
    /// `true`: declining would hand the same frame to the same plugin again.
    #[test]
    fn a_failed_fp16_frame_is_identity_rather_than_a_second_attempt() {
        let descriptor = described("com.example.halves");
        let (def, fake) = definition(&descriptor);
        fake.answers(Answer::Fail("no".to_owned()));

        let before: Vec<f16> = a_picture().iter().map(|s| f16::from_f32(*s)).collect();
        let mut halves = before.clone();
        let took = def.apply_f16_temporal(Uuid::nil(), 0.0, &mut halves, 2, 2, Params::EMPTY, &[]);
        assert!(took, "a failure is still an answer");
        let bits: Vec<u16> = halves.iter().map(|half| half.to_bits()).collect();
        let was: Vec<u16> = before.iter().map(|half| half.to_bits()).collect();
        assert_eq!(bits, was);
        assert_eq!(fake.frames().len(), 1, "asked once, not twice");

        // And a buffer that is no picture at all answers the same way. There is
        // nothing to send, `rgba` is left exactly as it was found - which is
        // the whole of what the hook's contract asks of a `false` - and a
        // `false` here would put these same halves down the f32 road to be
        // widened for an effect that is this one.
        let mut none: Vec<f16> = Vec::new();
        assert!(
            def.apply_f16_temporal(Uuid::nil(), 0.0, &mut none, 0, 0, Params::EMPTY, &[]),
            "a frame of no area is still this definition's answer"
        );
        let mut short = before.clone();
        assert!(
            def.apply_f16_temporal(Uuid::nil(), 0.0, &mut short, 4, 4, Params::EMPTY, &[]),
            "and so is a buffer shorter than the size it came with"
        );
        assert_eq!(
            short.iter().map(|half| half.to_bits()).collect::<Vec<_>>(),
            was,
            "neither touched a half"
        );
        assert_eq!(fake.frames().len(), 1, "and neither reached the plugin");
    }

    /// A retimer's sampled frames are what this instance last said it reads,
    /// clamped to what the neighbour decode will ever hold - and `None` before
    /// it has said anything, which is "whatever the schema declares" (§4.2).
    #[test]
    fn a_retimers_sampled_frames_are_what_the_definition_answers() {
        let descriptor = described("com.example.retimer");
        let (def, fake) = definition(&descriptor);
        let inst = an_instance(&def);

        assert_eq!(
            def.frames_needed(&inst, 0.0),
            None,
            "nothing more specific to say than the declaration"
        );

        fake.asks_for(&[-1_000, -2, 0, 2, 1_000]);
        let mut rgba = a_picture();
        def.apply_cpu_at(inst.id, 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(
            def.frames_needed(&inst, 0.0),
            Some(vec![-MAX_OFFSET, -2, 0, 2, MAX_OFFSET]),
            "clamped rather than refused later"
        );

        fake.asks_for(&[0]);
        def.apply_cpu_at(inst.id, 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(
            def.frames_needed(&inst, 0.0),
            None,
            "the frame in hand and nothing else is not an answer"
        );
    }

    /// And a frame that did **not** come back says nothing about the frames
    /// either side: a failure answers no offsets, and taking that for the
    /// instance's new window would narrow the frame key on a bad frame and
    /// retire every cached frame the good ones made.
    #[test]
    fn a_failed_frame_does_not_narrow_the_window_the_good_ones_set() {
        let descriptor = described("com.example.window");
        let (def, fake) = definition(&descriptor);
        let inst = an_instance(&def);
        let mut rgba = a_picture();

        fake.asks_for(&[-1, 0, 1]);
        def.apply_cpu_at(inst.id, 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(def.frames_needed(&inst, 0.0), Some(vec![-1, 0, 1]));

        fake.asks_for(&[]);
        fake.answers(Answer::Fail("not this one".to_owned()));
        def.apply_cpu_at(inst.id, 1.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(
            def.frames_needed(&inst, 1.0),
            Some(vec![-1, 0, 1]),
            "the window the last good frame set still stands"
        );
        assert!(def.last_error().is_some());
    }

    /// The neighbours the stack decoded cross beside the picture, each at its
    /// own comp time rather than at an offset the other side would have to
    /// re-derive.
    #[test]
    fn the_neighbours_cross_at_their_own_comp_times() {
        let descriptor = described("com.example.temporal");
        let (def, fake) = definition(&descriptor);
        let before = a_picture();
        let short = vec![0.0_f32; 3];
        let mut rgba = before.clone();
        def.apply_cpu_temporal(
            Uuid::nil(),
            0.0,
            &mut rgba,
            2,
            2,
            Params::new(&[(DERIVED_FRAME, Value::Int(12))]),
            &[(-1, before.as_slice()), (1, before.as_slice()), (2, &short)],
        );
        let frames = fake.frames();
        let seen = frames.first().expect("one frame went across");
        assert_eq!(seen.time, 12.0, "the comp's frame, not the layer's seconds");
        assert_eq!(
            seen.neighbours,
            vec![11.0, 13.0],
            "a neighbour of another size is left out rather than sent short"
        );
    }

    /// One row is one live instance: a second frame of the same row reuses it,
    /// and a bag that moved is one update rather than one more instance.
    #[test]
    fn one_row_is_one_live_instance_however_many_frames_it_renders() {
        let descriptor = described("com.example.reused");
        let (def, fake) = definition(&descriptor);
        let inst = Uuid::now_v7();
        let mut rgba = a_picture();

        def.apply_cpu_at(inst, 0.0, &mut rgba, 2, 2, Params::EMPTY);
        def.apply_cpu_at(inst, 1.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(fake.opens().len(), 1, "opened once");
        assert!(
            fake.updated.lock().unwrap().is_empty(),
            "and a bag that did not move costs no round trip"
        );

        let bag = [(ParamId::new("radius"), Value::Float(9.0))];
        def.apply_cpu_at(inst, 2.0, &mut rgba, 2, 2, Params::new(&bag));
        assert_eq!(fake.opens().len(), 1);
        assert_eq!(fake.updated.lock().unwrap().len(), 1);

        let second = Uuid::now_v7();
        def.apply_cpu_at(second, 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(fake.opens().len(), 2, "another row is another instance");

        let seen: Vec<InstanceId> = fake.frames().iter().map(|frame| frame.instance).collect();
        assert_eq!(seen, vec![1, 1, 1, 2]);
    }

    /// Two frames of one row are dispatched out of order by design (§4.4), and
    /// each is painted with **its own** values: telling an instance what its
    /// controls hold and asking it for the picture are one indivisible turn. A
    /// definition that let go between the two would hand a frame back painted
    /// with the other frame's numbers, as an `Ok`, to be cached under its own
    /// key - a silently wrong picture that survives in the cache.
    ///
    /// **The pool is pinned to one instance here, and the case says so**
    /// (§11 item 12). Under the open policy the rest of the suite renders with,
    /// the second frame's thread is allowed to open a second instance beside
    /// the first - so it would never tell the first instance anything, the
    /// absence this case asserts would be unreachable on every machine, and a
    /// definition that let go of the lease between the telling and the asking
    /// would pass. So the ring is narrowed to the floor that pins the pool to
    /// one, `fake.opens()` is asserted to be that one instance, and the two
    /// frames really are two frames of one instance that have to take turns.
    ///
    /// What is then driven is the interleaving itself: the second frame is not
    /// started until the first is inside the driver with its lease still held,
    /// and what the case watches for is anything of the second reaching the
    /// instance while it is.
    #[test]
    fn two_frames_of_one_row_are_each_painted_with_their_own_values() {
        let descriptor = described("com.example.turns");
        let fake = Arc::new(Fake::default());
        let def = over(&descriptor, a_pinned_policy(&descriptor), &fake);
        let row = Uuid::now_v7();

        // One frame first, so what follows is a race about values rather than
        // about which thread opens the instance.
        let mut warm = a_picture();
        def.apply_cpu_at(row, 0.0, &mut warm, 2, 2, Params::EMPTY);

        let (announce_told, tellings) = std::sync::mpsc::channel();
        let (announce_rendering, renderings) = std::sync::mpsc::channel();
        let gate = Arc::new(Gate::default());
        *fake.told.lock().unwrap() = Some(announce_told);
        *fake.rendering.lock().unwrap() = Some((announce_rendering, Arc::clone(&gate)));

        let first = [(ParamId::new("radius"), Value::Float(1.0))];
        let second = [(ParamId::new("radius"), Value::Float(2.0))];
        let mut raced = false;
        std::thread::scope(|threads| {
            let one = threads.spawn(|| {
                let mut rgba = a_picture();
                def.apply_cpu_at(row, 1.0, &mut rgba, 2, 2, Params::new(&first));
            });
            tellings.recv().expect("the first frame told the instance");
            renderings.recv().expect("and is inside the driver");

            let two = threads.spawn(|| {
                let mut rgba = a_picture();
                def.apply_cpu_at(row, 2.0, &mut rgba, 2, 2, Params::new(&second));
            });
            // The second frame is in the definition while the first frame's
            // turn is open. Anything it manages to tell the instance now lands
            // in the middle of that turn.
            raced = tellings
                .recv_timeout(std::time::Duration::from_millis(250))
                .is_ok();
            gate.open();
            one.join().expect("the first frame finished");
            two.join().expect("the second frame finished");
        });
        assert!(
            !raced,
            "the second frame told the instance in the middle of the first frame's turn"
        );
        assert_eq!(
            fake.opens().len(),
            1,
            "one instance, so the two frames had to take turns rather than \
             opening beside each other - if this ever grows, the absence above \
             is unreachable and proves nothing"
        );

        let painted: Vec<(f64, Option<ParamValue>)> = fake
            .frames()
            .iter()
            .skip(1)
            .map(|frame| (frame.time, frame.values.first().cloned()))
            .collect();
        assert_eq!(painted.len(), 2, "both frames of the row went across");
        for (time, radius) in painted {
            assert_eq!(
                radius,
                Some(ParamValue::Float(time)),
                "frame {time} was painted with the other frame's numbers"
            );
        }
    }

    /// The narrow window the per-render gate is still the whole of the answer
    /// in, and the reason `LfxDef::hosted` keeps it.
    ///
    /// `LfxDef::lease` reads the switched-off list before it opens anything, so
    /// through the definition the gate's own read can only fire for a tick that
    /// lands **after** that read and before the frame goes across - which is a
    /// person ticking the box while a frame is in flight.
    /// This is the case that reaches it: a driver that switches the plugin off
    /// as it hands the instance back. Take the read out of `Gated::process` and
    /// this is the case that notices; that `LfxDef::hosted` puts a `Gated` there
    /// at all is proved over a real bundle in `lumit-lfx-broker`'s own suite.
    #[test]
    fn a_plugin_switched_off_while_the_frame_is_in_flight_is_caught_by_the_gate() {
        let _guard = gate_lock();
        let descriptor = described("com.example.inflight");
        let schema = Box::leak(Box::new(schema_of(&descriptor).expect("it is an effect")));
        let fake = Arc::new(Fake::default());
        let gate: Arc<dyn LfxHost> = Arc::new(Gated::new(
            descriptor.identity.id.clone(),
            Arc::clone(&fake) as Arc<dyn LfxHost>,
        ));
        let def = LfxDef::new(
            &descriptor,
            schema,
            Arc::new(SwitchesOffOnOpen {
                identifier: descriptor.identity.id.clone(),
                inner: Arc::clone(&fake),
            }),
            gate,
            an_open_policy(&descriptor),
        );

        let before = a_picture();
        let mut rgba = before.clone();
        def.apply_cpu_at(Uuid::now_v7(), 0.0, &mut rgba, 2, 2, Params::EMPTY);

        assert_eq!(
            fake.opens().len(),
            1,
            "the instance was minted before the tick landed"
        );
        assert!(
            fake.frames().is_empty(),
            "and no frame reached the plugin after it"
        );
        let bits: Vec<u32> = rgba.iter().map(|sample| sample.to_bits()).collect();
        let was: Vec<u32> = before.iter().map(|sample| sample.to_bits()).collect();
        assert_eq!(bits, was, "identity, byte for byte");
        assert_eq!(
            def.last_error().as_deref(),
            Some(crate::DISABLED_REASON),
            "and the one shared constant the badge seam tests against"
        );

        crate::discover::set_enabled("com.example.inflight", true);
    }

    /// Which declarations carry a value is answered in one place -
    /// `schema::carriage`, read through `schema::value_elements` - and
    /// `declared_default` has to agree with it for every kind there is.
    ///
    /// The array is dense, so a declaration one side keeps and the other drops
    /// shifts **every element after it**: the plugin then reads correct-looking
    /// kind tags over the neighbouring row's value, which is the fault §2.1's
    /// `value_stride` rule exists to make impossible, arriving from the host's
    /// own side of the boundary.
    ///
    /// The list is written out rather than derived, for the reason
    /// `a_refusal_is_either_a_report_line_or_the_end_of_the_effect` gives:
    /// Rust has no stable way to enumerate an enum's variants. What stops a
    /// variant added later from going unswept is that it fails
    /// `declared_default`'s own exhaustive `match` and `schema::carriage`'s at
    /// once, and each sends its author to the other.
    #[test]
    fn every_declaration_that_carries_a_value_has_a_default_to_carry() {
        let mut sink = Describe::new();
        for (id, unit, kind) in every_kind() {
            assert!(
                sink.declare(Declaration {
                    id: id.to_owned(),
                    label: id.to_owned(),
                    unit,
                    flags: 0,
                    kind,
                }),
                "{id} is a declaration this host admits"
            );
        }
        let described = sink.finish().expect("nothing structural happened");
        let descriptor = PluginDescriptor::new(identity("com.example.sweep"), described);
        let schema = Box::leak(Box::new(schema_of(&descriptor).expect("it is an effect")));

        let crossing: Vec<&str> = crate::schema::value_elements(&descriptor, schema)
            .iter()
            .map(|declaration| declaration.id.as_str())
            .collect();
        let defaulted: Vec<&str> = descriptor
            .params
            .iter()
            .filter(|declaration| declared_default(declaration).is_some())
            .map(|declaration| declaration.id.as_str())
            .collect();
        assert_eq!(
            crossing, defaulted,
            "the schema's answer and this module's are one answer"
        );
        assert_eq!(
            crossing,
            vec!["f", "s", "i", "a", "b", "c", "col", "seed", "p2", "p3", "curve", "file"],
            "a button is the one declaration that carries nothing"
        );

        let defaults = defaults_of(&descriptor, schema);
        assert_eq!(defaults.len(), crossing.len(), "one default per element");
        assert_eq!(
            value_routes(&descriptor, schema)
                .iter()
                .map(|route| route.element)
                .max()
                .map_or(0, |last| last as usize + 1),
            defaults.len(),
            "and the routes number exactly the elements there are defaults to fill"
        );
    }

    /// The rows the plugin declared hidden are the rows the panel skips, and a
    /// hidden heading hides the whole run under it (§2.2).
    #[test]
    fn the_rows_the_plugin_hid_at_describe_are_the_rows_the_panel_skips() {
        let mut sink = Describe::new();
        assert!(sink.declare(float("shown", 0.0)));
        assert!(sink.declare(Declaration {
            flags: lumit_lfx_abi::LFX_PARAM_FLAG_HIDDEN,
            ..float("secret", 0.0)
        }));
        let described = sink.finish().expect("nothing structural happened");
        let descriptor = PluginDescriptor::new(identity("com.example.hidden"), described);
        let (def, _) = definition(&descriptor);
        let inst = an_instance(&def);

        assert_eq!(def.hidden_rows(&inst), vec!["secret"]);
    }

    /// A button reaches the plugin by the row it was declared under, a name
    /// that is not one is refused, and nothing comes back: an LFX plugin writes
    /// no rows and keeps no memory (D8).
    #[test]
    fn a_button_press_reaches_the_plugin_and_writes_no_rows() {
        let descriptor = described("com.example.pressed");
        let (def, fake) = definition(&descriptor);
        let inst = an_instance(&def);
        let rgba = [0_u8; 4];
        let source = PressFrame {
            rgba: &rgba,
            width: 1,
            height: 1,
        };

        assert_eq!(
            def.press(&inst, 0.0, "analyse", &source),
            Ok(Pressed::default()),
            "no rows written and no memory kept"
        );
        assert_eq!(
            fake.pressed.lock().unwrap().clone(),
            vec![(1, "analyse".to_owned())]
        );
        assert!(def
            .press(&inst, 0.0, "radius", &source)
            .is_err_and(|why| why.contains("radius")));
    }

    /// The document's own rows are what a press starts from, since a press
    /// happens outside a render and so has no bag.
    #[test]
    fn a_press_hands_the_plugin_the_rows_the_document_holds() {
        let descriptor = described("com.example.stored");
        let (def, fake) = definition(&descriptor);
        let mut inst = an_instance(&def);
        inst.params = vec![EffectParam {
            id: "radius".to_owned(),
            value: EffectValue::Float(Property::fixed(7.25)),
            extra: serde_json::Map::new(),
        }];
        let rgba = [0_u8; 4];
        let source = PressFrame {
            rgba: &rgba,
            width: 1,
            height: 1,
        };

        def.press(&inst, 0.0, "analyse", &source).expect("pressed");
        assert_eq!(
            fake.opens().first().and_then(|values| values.first()),
            Some(&ParamValue::Float(7.25))
        );
    }

    /// A stored seed past the signed range is **one** number on both roads out
    /// of this module (§4.2).
    ///
    /// `EffectValue::Seed` is a `u32` and `fx::builtins::fresh_seed` mints
    /// across the whole of it, so roughly half of all fresh instances carry a
    /// seed no `i32` holds. The render path takes the resolved bag, which
    /// carries it reinterpreted - `fx::resolved`'s `Value::Int(*s as i32)` -
    /// and the press path reads the stored row itself. Clamp on one road and
    /// the two write different seeds into the same live instance, which is a
    /// plugin's noise field moving under a row for a reason the document does
    /// not record.
    #[test]
    fn a_seed_past_the_signed_range_is_one_number_on_both_roads() {
        const STORED: u32 = 3_000_000_000;

        let mut sink = Describe::new();
        assert!(sink.declare(plain("seed", Declared::Seed)));
        assert!(sink.declare(plain("act", Declared::Action)));
        let described = sink.finish().expect("nothing structural happened");
        let descriptor = PluginDescriptor::new(identity("com.example.seed"), described);
        let (def, fake) = definition(&descriptor);

        let mut inst = an_instance(&def);
        inst.params = vec![EffectParam {
            id: "seed".to_owned(),
            value: EffectValue::Seed(STORED),
            extra: serde_json::Map::new(),
        }];
        let rgba = [0_u8; 4];
        let source = PressFrame {
            rgba: &rgba,
            width: 1,
            height: 1,
        };
        def.press(&inst, 0.0, "act", &source).expect("pressed");
        let pressed = fake
            .opens()
            .first()
            .and_then(|values| values.first())
            .cloned();

        // The bag as `fx::resolved` fills it for that same stored row, and the
        // same row of the document, so both roads reach one live instance.
        let bag = [(ParamId::new("seed"), Value::Int(STORED as i32))];
        let mut picture = a_picture();
        def.apply_cpu_at(inst.id, 0.0, &mut picture, 2, 2, Params::new(&bag));
        let rendered = fake
            .frames()
            .first()
            .and_then(|frame| frame.values.first())
            .cloned();

        assert_eq!(
            pressed, rendered,
            "the press and the frame tell one instance one seed"
        );
        assert_eq!(
            pressed,
            Some(ParamValue::Seed(i64::from(STORED as i32))),
            "and it is the reinterpretation the resolve walk makes, never a clamp"
        );
    }

    /// Failure 5, through the definition rather than through the gate alone: a
    /// plugin switched off mid-session renders identity, files the one shared
    /// reason the badge seam reads, and its code is not reached - not even to
    /// open an instance (§5.4, §4.3).
    ///
    /// The seam that refuses it here is the **instance**: `LfxDef::lease` reads
    /// the switched-off list before anything else happens, so a tick that has
    /// already landed is answered there and the gate behind it is never asked.
    /// The gate's own read is the tick that lands after that one, and
    /// `a_plugin_switched_off_while_the_frame_is_in_flight_is_caught_by_the_gate`
    /// is where it is held.
    #[test]
    fn a_switched_off_plugin_renders_identity_through_the_definition() {
        let _guard = gate_lock();
        let descriptor = described("com.example.switched");
        let (def, fake) = gated(&descriptor);
        let inst = Uuid::now_v7();

        let before = a_picture();
        let mut rgba = before.clone();
        def.apply_cpu_at(inst, 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(rgba, vec![1.0; 16], "on to begin with");

        crate::discover::set_enabled("com.example.switched", false);
        rgba.copy_from_slice(&before);
        def.apply_cpu_at(inst, 1.0, &mut rgba, 2, 2, Params::EMPTY);
        let bits: Vec<u32> = rgba.iter().map(|sample| sample.to_bits()).collect();
        let was: Vec<u32> = before.iter().map(|sample| sample.to_bits()).collect();
        assert_eq!(bits, was, "identity, byte for byte");
        assert_eq!(
            def.last_error().as_deref(),
            Some(crate::DISABLED_REASON),
            "the constant `badge_of` tests against, never a twin of it"
        );
        assert_eq!(fake.frames().len(), 1, "the plugin's code is not reached");

        crate::discover::set_enabled("com.example.switched", true);
    }

    /// And a plugin switched off before its row was ever drawn is refused where
    /// the gate cannot see - at the instance, which is the other place a
    /// plugin's code would run.
    #[test]
    fn a_switched_off_plugin_is_never_opened_at_all() {
        let _guard = gate_lock();
        let descriptor = described("com.example.never");
        let (def, fake) = gated(&descriptor);

        crate::discover::set_enabled("com.example.never", false);
        let mut rgba = a_picture();
        def.apply_cpu_at(Uuid::now_v7(), 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert!(fake.opens().is_empty(), "no instance was ever minted");
        assert_eq!(def.last_error().as_deref(), Some(crate::DISABLED_REASON));

        crate::discover::set_enabled("com.example.never", true);
    }

    /// The definition derives the layer's **frame** and nothing else. There is
    /// no `derived.memory` here and there never will be: D8 keeps
    /// `plugin_state` empty for every LFX instance, which is what makes the
    /// frame key complete without one.
    ///
    /// What is pushed is inside [`DERIVED_PREFIX`], which is the half of that
    /// prefix's refusal (`a_row_inside_the_hosts_own_prefix_refuses_the_effect`)
    /// this side owns: the id the sink turns a plugin away for and the id the
    /// host writes have to be the same string, or the rule guards a name
    /// nothing uses.
    #[test]
    fn an_lfx_effect_derives_the_frame_and_no_state_of_its_own() {
        assert!(
            DERIVED_FRAME_ID.starts_with(DERIVED_PREFIX),
            "the host writes an id the sink would have refused a plugin for"
        );
        let descriptor = described("com.example.derived");
        let (def, _) = definition(&descriptor);
        let inst = an_instance(&def);

        let mut pushed: Vec<(ParamId, Value)> = Vec::new();
        let cx = ResolveCx {
            inst: &inst,
            lt: 0.5,
            diag_px: 1.0,
            px_scale: 1.0,
            markers: &lumit_core::fx::MarkerContext::NONE,
            context: Arc::new(lumit_core::expression::ExpressionContext::detached()),
        };
        def.resolve_derived(&cx, &mut |id, value| pushed.push((id, value)));
        assert!(
            pushed.is_empty(),
            "a stack with no comp behind it has no frame to push, and nothing else is derived"
        );

        let comp = a_comp();
        let mut document = lumit_core::model::Document::new();
        let id = comp.id;
        document
            .items
            .push(lumit_core::model::ProjectItem::Composition(comp));
        let cx = ResolveCx {
            inst: &inst,
            lt: 0.5,
            diag_px: 1.0,
            px_scale: 1.0,
            markers: &lumit_core::fx::MarkerContext::NONE,
            context: Arc::new(lumit_core::expression::ExpressionContext {
                document: Arc::new(document),
                comp: Some(id),
                ..lumit_core::expression::ExpressionContext::detached()
            }),
        };
        pushed.clear();
        def.resolve_derived(&cx, &mut |id, value| pushed.push((id, value)));
        assert_eq!(
            pushed,
            vec![(DERIVED_FRAME, Value::Int(30))],
            "half a second at sixty is frame thirty, and it is the only thing derived"
        );
    }

    /// A definition does not put itself in the catalogue: the pass and the
    /// entry have to arrive together, and a definition that registered itself
    /// would make half of that pair happen out of the composition root's sight
    /// (§4.5, §11 item 2).
    #[test]
    fn a_leaked_definition_does_not_register_itself() {
        let descriptor = described("com.example.unregistered");
        let (def, _) = definition(&descriptor);
        let name = def.schema().match_name;
        let leaked = def.leak();
        assert_eq!(leaked.schema().match_name, name);
        assert!(
            lumit_core::fx::def(name).is_none(),
            "leaking is not registering"
        );
    }

    // -----------------------------------------------------------------------
    // The instance pool and §4.4's provisional concurrency policy
    // (docs/impl/lfx.md §4.4, §14 item 9). The clauses that waited for the
    // pool; the two that did not landed with the in-process host.
    // -----------------------------------------------------------------------

    /// Frames of one row arrive out of order and each is the picture **its own**
    /// numbers call for.
    ///
    /// This is the property the pool exists to keep and the one it could most
    /// easily lose: a plugin holds no opaque state (D8), so its instances are
    /// interchangeable, and the only thing that makes a frame's answer its own
    /// is that nothing told the instance anything else between the telling and
    /// the asking. The case renders the run once in order and once from several
    /// threads in a deliberately jumbled order, and compares the two by bits.
    ///
    /// The overlap is **recorded** rather than hoped for (§11 item 12): the
    /// driver counts how many frames were ever inside it at once, and a run in
    /// which none ever overlapped would prove nothing about a pool.
    #[test]
    fn frames_arrive_out_of_order_and_the_picture_is_unchanged() {
        let descriptor = described("com.example.jumbled");
        let row = Uuid::now_v7();

        let (in_order, one) = definition(&descriptor);
        one.answers(Answer::PaintTheValue);
        let expected: Vec<Vec<u32>> = (0..8)
            .map(|frame| one_frame(&in_order, row, frame))
            .collect();

        let (jumbled, many) = definition(&descriptor);
        many.answers(Answer::PaintTheValue);
        let overlap = many.records_overlap();
        let painted: Mutex<BTreeMap<u32, Vec<u32>>> = Mutex::new(BTreeMap::new());
        let second = Uuid::now_v7();
        std::thread::scope(|threads| {
            // Not the order the timeline would ask in, and not the order the
            // answers are compared in either.
            for frame in [5, 1, 7, 0, 3, 6, 2, 4] {
                let jumbled = &jumbled;
                let painted = &painted;
                threads.spawn(move || {
                    let bits = one_frame(jumbled, second, frame);
                    painted.lock().unwrap().insert(frame, bits);
                });
            }
        });

        assert!(
            overlap.most() > 1,
            "no two frames were ever in flight at once, so this proves nothing about a pool"
        );
        let painted = painted.lock().unwrap().clone();
        assert_eq!(painted.len(), 8, "every frame came back");
        for (frame, bits) in painted {
            assert_eq!(
                bits, expected[frame as usize],
                "frame {frame} came back painted with another frame's numbers"
            );
        }
    }

    /// A thread-unsafe plugin never sees two frames at once - and neither does
    /// any other plugin of its bundle, because `lfx.thread-unsafe` serialises
    /// the **bundle** (§2.6).
    ///
    /// Three runs of the same four frames through the same driver, which is
    /// what makes the absences mean anything (§11 item 12). A bundle where
    /// **both** plugins declare it records one caller inside at a time; a
    /// bundle where **neither** does records two, so the fixture can tell the
    /// two apart; and the **mixed** bundle - one declaring, one not - is the
    /// only shape that tells bundle-wide from per-plugin, and it records one.
    /// The plugin that declared nothing shares a process, a broker and a ring
    /// with the plugin that did.
    #[test]
    fn a_thread_unsafe_plugin_never_sees_two_processes() {
        let declared = Traits {
            flags: LFX_TRAIT_THREAD_UNSAFE,
            ..Traits::default()
        };
        let pinned = [
            declaring("com.example.bundle.one", declared),
            declaring("com.example.bundle.two", declared),
        ];
        let mixed = [
            declaring("com.example.mixed.one", declared),
            described("com.example.mixed.two"),
        ];
        let loose = [
            described("com.example.loose.one"),
            described("com.example.loose.two"),
        ];

        assert_eq!(
            most_at_once(&pinned, Overlap::BRIEF),
            1,
            "two frames of one thread-unsafe bundle were inside the plugin at once"
        );
        assert_eq!(
            most_at_once(&mixed, Overlap::BRIEF),
            1,
            "the plugin that declared nothing rendered beside the plugin that declared it"
        );
        assert!(
            most_at_once(&loose, Overlap::PATIENT) > 1,
            "the fixture never overlapped at all, so the pinned runs prove nothing"
        );
    }

    /// Four frames - two of each of a bundle's two plugins, all dispatched at
    /// once - and the most that were ever inside the driver together.
    ///
    /// `waiting` is how long a lone caller waits for company before deciding
    /// there is none: [`Overlap::PATIENT`] for a run asserted to overlap, where
    /// the wait is never spent because the company arrives, and
    /// [`Overlap::BRIEF`] for a run asserted not to, where nothing is coming
    /// and the number is only the case's own running time.
    fn most_at_once(bundle: &[PluginDescriptor; 2], waiting: std::time::Duration) -> usize {
        let serial = a_bundle(bundle);
        let ledger = Ledger::new();
        let fake = Arc::new(Fake::default());
        let overlap = fake.records_overlap_within(waiting);
        let plugins: Vec<LfxDef> = bundle
            .iter()
            .map(|descriptor| {
                let policy = Policy::declared(
                    descriptor.identity.traits.as_ref(),
                    &a_ring(u32::MAX),
                    &ledger,
                    &serial,
                );
                over(descriptor, policy, &fake)
            })
            .collect();
        std::thread::scope(|threads| {
            for plugin in &plugins {
                for frame in 0..2 {
                    threads.spawn(move || {
                        let mut rgba = a_picture();
                        // A row of its own each time, so the pin is the only
                        // thing that could be serialising them.
                        plugin.apply_cpu_at(
                            Uuid::now_v7(),
                            f64::from(frame),
                            &mut rgba,
                            2,
                            2,
                            Params::EMPTY,
                        );
                    });
                }
            }
        });
        overlap.most()
    }

    /// The pool collapses to one under Severe pressure and grows back when the
    /// tier is quiet again (§4.4).
    ///
    /// The collapse is not merely a lower ceiling: the instances it is no
    /// longer entitled to are **closed**, which is the whole point of trimming
    /// under pressure, and the case asserts that against the driver's own
    /// record rather than against the count.
    #[test]
    fn the_pool_collapses_under_severe_pressure_and_grows_back() {
        // Small enough that one reservation moves the needle, which is what a
        // ledger under test is for.
        let ledger = Ledger::with_budgets(1_024, 1_000);
        let descriptor = described("com.example.pressure");
        let fake = Arc::new(Fake::default());
        let def = over(
            &descriptor,
            Policy::declared(None, &a_ring(u32::MAX), &ledger, &Serial::new()),
            &fake,
        );
        let row = Uuid::now_v7();

        both_at_once(&def, &fake, row);
        assert_eq!(def.pool().live(row), 2, "two frames at once, two instances");

        let squeeze = ledger
            .try_reserve(Tier::Ram, 950)
            .expect("the ledger has room for the squeeze itself");
        assert!(
            ledger.pressure(Tier::Ram).should_trim(),
            "the tier is asking everybody to trim"
        );
        let mut rgba = a_picture();
        def.apply_cpu_at(row, 9.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(def.pool().live(row), 1, "collapsed to one");
        assert_eq!(
            fake.closed.lock().unwrap().len(),
            1,
            "and the instance it gave up was closed rather than merely forgotten"
        );

        drop(squeeze);
        // Evidence for the next growth: a growth that did not raise throughput
        // is the last one, so the pool wants a run of frames before it asks
        // again.
        for frame in 0..10 {
            let mut rgba = a_picture();
            def.apply_cpu_at(row, f64::from(frame), &mut rgba, 2, 2, Params::EMPTY);
        }
        both_at_once(&def, &fake, row);
        assert_eq!(def.pool().live(row), 2, "and grew back");
    }

    /// Two frames of one row, dispatched so that they really are in flight
    /// together - the driver's own recorder says so, and the case fails rather
    /// than passing quietly if they were not.
    fn both_at_once(def: &LfxDef, fake: &Arc<Fake>, row: Uuid) {
        let overlap = fake.records_overlap();
        std::thread::scope(|threads| {
            for frame in 0..2 {
                threads.spawn(move || {
                    let mut rgba = a_picture();
                    def.apply_cpu_at(row, f64::from(frame), &mut rgba, 2, 2, Params::EMPTY);
                });
            }
        });
        fake.stops_recording();
        assert_eq!(overlap.most(), 2, "the two frames never overlapped");
    }

    /// Two exports of one project are bit-identical whatever the pool did
    /// (§14 item 9), which is docs/08 §2.5's determinism read at this seam.
    ///
    /// One export is rendered through a pool the ring pins to a single instance
    /// and the other through a pool free to grow, with the frames dispatched
    /// out of order from several threads. The two runs are compared by bits,
    /// because a pool that mixed two frames' numbers would produce a picture
    /// that is plausible rather than one that is obviously wrong.
    #[test]
    fn two_exports_of_the_same_project_are_bit_identical_whatever_the_pool_did() {
        let descriptor = described("com.example.export");
        let ledger = Ledger::new();

        // A ring the ledger granted only its floor carries one frame in flight,
        // so this export's pool can never be more than one instance wide.
        let narrow = Arc::new(Fake::default());
        narrow.answers(Answer::PaintTheValue);
        let pinned = over(
            &descriptor,
            Policy::declared(
                None,
                &a_ring(crate::ipc::ring::RING_MIN_SLOTS),
                &ledger,
                &Serial::new(),
            ),
            &narrow,
        );
        assert_eq!(pinned.pool().policy().ceiling(), 1, "the ring pins it");
        let row = Uuid::now_v7();
        let first: Vec<Vec<u32>> = (0..8).map(|frame| one_frame(&pinned, row, frame)).collect();

        let wide = Arc::new(Fake::default());
        wide.answers(Answer::PaintTheValue);
        let grown = over(
            &descriptor,
            Policy::declared(None, &a_ring(u32::MAX), &ledger, &Serial::new()),
            &wide,
        );
        let overlap = wide.records_overlap();
        let second: Mutex<BTreeMap<u32, Vec<u32>>> = Mutex::new(BTreeMap::new());
        let other = Uuid::now_v7();
        std::thread::scope(|threads| {
            for frame in [3, 0, 6, 2, 7, 1, 5, 4] {
                let grown = &grown;
                let second = &second;
                threads.spawn(move || {
                    let bits = one_frame(grown, other, frame);
                    second.lock().unwrap().insert(frame, bits);
                });
            }
        });
        assert!(
            overlap.most() > 1,
            "the second export never used more than one instance, so it is the same export twice"
        );

        let second = second.lock().unwrap().clone();
        for (frame, bits) in second {
            assert_eq!(
                bits, first[frame as usize],
                "frame {frame} came out differently when the pool was allowed to grow"
            );
        }
    }

    /// A frame whose declared working memory the ledger will not grant is **not
    /// dispatched** (§4.4, §8).
    ///
    /// This is the ceiling LFX has in place of OFX's `memoryAlloc`: there is no
    /// host allocator to refuse through, so the refusal is here, before the
    /// plugin is reached at all. The alternative is dispatching the frame and
    /// having the plugin discover the refusal with an allocator, in a process
    /// with no way to say so.
    #[test]
    fn a_frame_whose_declared_scratch_the_ledger_will_not_grant_is_not_dispatched() {
        let descriptor = declaring(
            "com.example.hungry",
            Traits {
                scratch_bytes_per_megapixel: 8_192,
                ..Traits::default()
            },
        );
        let starved = Ledger::with_budgets(1_024, 1_024);
        let fake = Arc::new(Fake::default());
        let def = over(
            &descriptor,
            Policy::declared(
                descriptor.identity.traits.as_ref(),
                &a_ring(u32::MAX),
                &starved,
                &Serial::new(),
            ),
            &fake,
        );

        let before = a_picture();
        let mut rgba = before.clone();
        def.apply_cpu_at(Uuid::now_v7(), 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert!(fake.frames().is_empty(), "no frame reached the plugin");
        assert!(
            fake.opens().is_empty(),
            "and no instance was opened to hold one"
        );
        let bits: Vec<u32> = rgba.iter().map(|sample| sample.to_bits()).collect();
        let was: Vec<u32> = before.iter().map(|sample| sample.to_bits()).collect();
        assert_eq!(bits, was, "identity, byte for byte");
        assert_eq!(
            def.last_error().as_deref(),
            Some(
                "the plugin declared 8192 bytes of working memory for this frame, \
                 which the memory budget would not grant"
            ),
            "and the layer wears a sentence naming what was asked for"
        );

        // The same declaration against a ledger that can afford it renders.
        let rich = Ledger::new();
        let fed = Arc::new(Fake::default());
        let def = over(
            &descriptor,
            Policy::declared(
                descriptor.identity.traits.as_ref(),
                &a_ring(u32::MAX),
                &rich,
                &Serial::new(),
            ),
            &fed,
        );
        let mut rgba = a_picture();
        def.apply_cpu_at(Uuid::now_v7(), 0.0, &mut rgba, 2, 2, Params::EMPTY);
        assert_eq!(fed.frames().len(), 1, "a declaration is not a refusal");
        assert_eq!(rich.used(Tier::Ram), 0, "and the frame gave its bytes back");
    }

    /// A row that has left the layer does not hold its instance for ever
    /// ([`crate::pool::MAX_POOL_ROWS`]).
    ///
    /// `LfxDef` left this standing: nothing evicted a row that had gone, the
    /// shipping definition is leaked so `Drop` never ran either, and a session
    /// of add-and-delete churn walked towards the 1,024 live instances a
    /// bundle's broker refuses past. The pool's own table is where the answer
    /// lives, because the pool is the only thing that knows which rows are
    /// still being rendered.
    #[test]
    fn a_row_that_has_left_the_pool_does_not_hold_its_instance_for_ever() {
        let descriptor = described("com.example.churn");
        let (def, fake) = definition(&descriptor);
        let rows: Vec<Uuid> = (0..crate::pool::MAX_POOL_ROWS + 4)
            .map(|_| Uuid::now_v7())
            .collect();
        for row in &rows {
            let mut rgba = a_picture();
            def.apply_cpu_at(*row, 0.0, &mut rgba, 2, 2, Params::EMPTY);
        }

        assert_eq!(
            fake.closed.lock().unwrap().len(),
            4,
            "four rows over the ceiling, four instances closed"
        );
        for row in rows.iter().take(4) {
            assert_eq!(def.pool().live(*row), 0, "the oldest rows went first");
        }
        for row in rows.iter().skip(4) {
            assert_eq!(def.pool().live(*row), 1, "and the rest are still here");
        }
    }

    /// A row whose instances eviction closed keeps what its last render said it
    /// **reads** (§4.4).
    ///
    /// Eviction costs "a round trip and no picture", and that is only true if
    /// the row's remembered `frames_needed` outlives its instances: the frame
    /// key is computed over those offsets, so a row that lost them would come
    /// back keyed, prefetched and rendered against the *declared* window
    /// instead. Which rows are evicted is ordered by when each was last leased,
    /// so the difference would land on whichever rows the threads happened to
    /// reach first - and two exports of one project would not agree.
    #[test]
    fn an_evicted_row_keeps_the_window_its_last_render_asked_for() {
        let descriptor = described("com.example.remembered");
        let (def, fake) = definition(&descriptor);
        let inst = an_instance(&def);

        fake.asks_for(&[-2, -1, 1, 2]);
        let mut rgba = a_picture();
        def.apply_cpu_at(inst.id, 0.0, &mut rgba, 2, 2, Params::EMPTY);
        let window = def.frames_needed(&inst, 0.0);
        assert_eq!(
            window,
            Some(vec![-2, -1, 0, 1, 2]),
            "the retimer's answer, kept for the key walk"
        );

        // Enough other rows that this one - the least recently leased - is
        // evicted.
        fake.asks_for(&[]);
        for _ in 0..crate::pool::MAX_POOL_ROWS + 4 {
            let mut rgba = a_picture();
            def.apply_cpu_at(Uuid::now_v7(), 0.0, &mut rgba, 2, 2, Params::EMPTY);
        }
        assert_eq!(
            def.pool().live(inst.id),
            0,
            "the row's instance was closed, which is what eviction is"
        );

        assert_eq!(
            def.frames_needed(&inst, 0.0),
            window,
            "and the window it reads came back with it rather than falling back to the declaration"
        );
    }

    fn a_comp() -> lumit_core::model::Composition {
        use lumit_core::model::LinearColour;
        use lumit_core::time::{Duration, FrameRate, Rational};
        lumit_core::model::Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "c".to_owned(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }
    }
}
