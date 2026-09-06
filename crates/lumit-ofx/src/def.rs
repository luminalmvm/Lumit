//! A described plugin, turned into an entry in the effect catalogue.
//!
//! # In plain terms
//!
//! [`schema`](crate::schema) wrote a plugin's *declaration* in Lumit's words;
//! this writes its *behaviour*. Every one of Lumit's own effects is a value
//! implementing one trait — [`EffectDef`] — and the catalogue is a list of
//! those values (docs/impl/effect-registry.md §2.4). An OFX plugin becomes one
//! more value of exactly that trait, registered into exactly that list. From
//! there nothing downstream can tell the difference: the Add-effect menu, the
//! Effect Controls panel, the resolve walk, the frame key and the cache all see
//! an effect.
//!
//! That is the whole point of the refactor §2.6 describes. There is no plugin
//! branch in the render, no second dispatch, no parallel catalogue.
//!
//! # What this definition does with each hook
//!
//! * **`schema`** hands back the leaked declaration the describe made.
//! * **`apply_cpu_temporal`** is the render: the picture is fp32 premultiplied
//!   linear, which is exactly what the plugin boundary wants (docs/12 §2.1), so
//!   it goes straight across to the [`PluginHost`] and comes straight back,
//!   with the layer's decoded neighbours beside it for a plugin that reads the
//!   frames either side. The GPU half is the generic read-back wrapper in
//!   `lumit-render`, which calls this. There is no WGSL to write, because the
//!   maths is the plugin's. The time it is told is the layer's **frame**, the
//!   unit OFX counts in, carried into the bag by `resolve_derived` from the
//!   comp's rate.
//! * **`frames_needed`** asks the plugin what other frames this instance reads,
//!   which is `getFramesNeeded` (docs/12 §2.1) and is what puts a retimer's
//!   sampled frames into the frame key and the prefetch.
//! * **`resolve_derived`** pushes nothing. A built-in derives values from the
//!   things the bag cannot carry — layer time, the marker context; a plugin
//!   derives nothing, because everything it computes it computes itself.
//! * **`last_error`** is how a plugin that died, hung or was disabled tells the
//!   layer to wear a badge instead of stopping the comp (docs/12 §2.3).
//!
//! # Values, and the one that does not cross
//!
//! The resolved bag is keyed by hashed ids, so the plugin's own parameter names
//! are recovered through [`crate::schema::value_routes`] — the reverse of the
//! same enumeration that minted the rows. Numbers, switches, choices and
//! colours all cross. A **path** does not: the bag carries a file-table slot
//! rather than the string, so a plugin's path parameter keeps whatever the
//! plugin defaulted it to until the panel path that edits one lands.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use lumit_core::fx::{
    EffectDef, EffectSchema, ParamId, Params, PressFrame, Pressed, ResolveCx, Value,
};
use lumit_core::model::{EffectInstance, EffectValue};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::bundle::{Bundle, PluginRef};
use crate::describe::{Context, PluginDescriptor};
use crate::ffi::param_types;
use crate::image::Frame16;
use crate::instance::{Instance, ParamSnapshot};
use crate::ipc::broker::Broker;
use crate::ipc::proto::InstanceId;
use crate::props::PropValue;
use crate::render::{RenderRequest, Rendered};
use crate::schema::{value_routes, ValueRoute};

/// One rendered frame, and whether it is the plugin's work or a stand-in for it.
pub struct Rendering {
    /// The picture. On a failure this is the input, unchanged — identity, so
    /// the comp still composites (docs/12 §2.3).
    pub frame: Frame16,
    /// What went wrong, in a sentence, or `None` when nothing did.
    pub error: Option<String>,
}

/// Where a plugin effect's frames actually come from.
///
/// Two implementations, and the difference between them is which side of a
/// process boundary the plugin sits on: [`BrokerHost`] talks to the broker
/// process docs/12 §2.3 requires, and [`LocalHost`] drives a bundle loaded into
/// this one. The definition above does not care, which is what lets the tests
/// prove the catalogue seam without a second process and lets the shipping path
/// keep the plugin at arm's length.
///
/// **Never called with any lock held**, and never from a rebuild path: these
/// are the render and the frames-needed question, both of which may block on
/// somebody else's code.
pub trait PluginHost: Send + Sync {
    /// Render one frame of one instance.
    ///
    /// `instance` is the effect instance's own id, so the host can keep one
    /// live plugin instance per row rather than rebuilding it per frame, and so
    /// a failure is attributable to the row it happened on. `time` is the
    /// layer's frame. `neighbours` are the Source clip at other frames, by
    /// offset from `time`, for a plugin that reads the frames either side. A
    /// host that can't render (disabled, crashed, out of deadline) answers
    /// with `source` and an error rather than a failure: a comp that stops
    /// compositing is worse than a comp with a badge on one layer.
    fn render(
        &self,
        instance: Uuid,
        time: f64,
        params: &ParamSnapshot,
        source: Frame16,
        neighbours: &[(i32, Frame16)],
    ) -> Rendering;

    /// The source-relative frame offsets this instance reads at this time —
    /// `getFramesNeeded`, as offsets rather than as OFX's absolute range.
    ///
    /// `None` when the plugin has nothing more specific to say than its
    /// declaration, which is every plugin that is not a retimer.
    fn frames_needed(&self, instance: Uuid, time: f64, params: &ParamSnapshot) -> Option<Vec<i32>>;

    /// Press one of an instance's buttons, with the picture its own window may
    /// ask for, and hand back every value the plugin holds afterwards.
    ///
    /// Blocks for as long as the plugin does, which for a plugin with an editor
    /// is until the user closes it.
    ///
    /// # Errors
    ///
    /// A sentence for the badge.
    fn press(
        &self,
        instance: Uuid,
        time: f64,
        params: &ParamSnapshot,
        name: &str,
        source: Frame16,
    ) -> Result<ParamSnapshot, String>;
}

thread_local! {
    /// Why the render this thread most recently ran was a placeholder.
    ///
    /// Thread-local rather than a field, because it is read by the dispatch
    /// seam immediately after the call that set it, on the thread that made it:
    /// a shared field would let two frames in flight report each other's
    /// failure. Taken when read, so a stale reason can never badge a later frame
    /// that went perfectly well.
    static LAST_ERROR: std::cell::RefCell<Option<String>> = const {
        std::cell::RefCell::new(None)
    };
}

/// An OFX plugin, as an entry in the effect catalogue.
pub struct OfxEffectDef {
    schema: &'static EffectSchema,
    routes: Vec<ValueRoute>,
    /// The values the plugin declared, which is what a row Lumit cannot carry
    /// (a path, a vendor blob) keeps.
    defaults: ParamSnapshot,
    host: Arc<dyn PluginHost>,
}

impl OfxEffectDef {
    /// Build the definition for one described plugin.
    ///
    /// `schema` is the leaked declaration [`crate::schema::schema_of`] made from
    /// the same descriptor; the two are passed separately rather than derived
    /// here because the scan already has the schema in hand and leaking a second
    /// copy of it would be a second answer to the same question.
    #[must_use]
    pub fn new(
        descriptor: &PluginDescriptor,
        schema: &'static EffectSchema,
        host: Arc<dyn PluginHost>,
    ) -> Self {
        Self {
            schema,
            routes: value_routes(descriptor),
            defaults: ParamSnapshot::from_defaults(descriptor),
            host,
        }
    }

    /// Give this definition the `'static` lifetime the catalogue holds.
    ///
    /// The leak is the honest spelling of that lifetime: an effect discovered at
    /// scan time lives as long as the session. Registering it is the caller's
    /// next move, and it is deliberately not done here — the catalogue entry and
    /// the render pass have to arrive together (`lumit-render`'s
    /// `gpufx::ofx::register`), and a definition that registered itself would
    /// make half of that pair happen out of the composition root's sight.
    #[must_use]
    pub fn leak(self) -> &'static dyn EffectDef {
        Box::leak(Box::new(self))
    }

    /// The bag, as the values the plugin reads.
    fn snapshot(&self, p: Params<'_>) -> ParamSnapshot {
        self.assemble(|route| p.get(route.id))
    }

    /// The document's own rows at `lt`, with the plugin's memory laid over
    /// them. This is what a press starts from, since a press happens outside
    /// a render and so has no bag.
    // ponytail: an expression on a row reads as its plain evaluation here, the
    // resolve walk is the thing that knows the layer's context.
    fn snapshot_of(&self, inst: &EffectInstance, lt: f64) -> ParamSnapshot {
        let mut snapshot = self.assemble(|route| {
            let param = inst.params.iter().find(|param| param.id == route.row)?;
            Some(match &param.value {
                EffectValue::Float(property) => Value::Float(property.value_at(lt) as f32),
                EffectValue::Bool(value) => Value::Bool(*value),
                EffectValue::Choice(value) => Value::Choice(*value),
                EffectValue::Colour(channels) => {
                    let at = |index: usize| channels[index].value_at(lt) as f32;
                    Value::Colour([at(0), at(1), at(2), at(3)])
                }
                _ => return None,
            })
        });
        if let Some(memory) = inst.plugin_state_bytes() {
            if let Ok(memory) = bincode::deserialize::<ParamSnapshot>(&memory) {
                recall(&mut snapshot, &memory);
            }
        }
        snapshot
    }

    /// The rows whose value the plugin changed while it was pressed, as the
    /// document's rows.
    fn rows_written(
        &self,
        before: &ParamSnapshot,
        after: &ParamSnapshot,
    ) -> Vec<(&'static str, Value)> {
        let mut rows = Vec::new();
        for route in &self.routes {
            if route.param_type == param_types::PUSH_BUTTON {
                continue;
            }
            let Some(value) = after.get(&route.name) else {
                continue;
            };
            if Some(value) == before.get(&route.name) {
                continue;
            }
            if let Some(value) = read_component(value, route) {
                rows.push((route.row, value));
            }
        }
        rows
    }

    /// Everything the plugin holds that no row carries, a vendor blob or a
    /// text, packed for `EffectInstance::plugin_state`. `None` when there is
    /// nothing of the kind.
    fn memory_of(&self, after: &ParamSnapshot) -> Option<Vec<u8>> {
        let mut memory = ParamSnapshot::new();
        for (name, value) in after.iter() {
            if self.routes.iter().any(|route| &route.name == name) {
                continue;
            }
            memory.set(name, value.clone());
        }
        if memory.is_empty() {
            return None;
        }
        bincode::serialize(&memory).ok()
    }

    /// The plugin's values, one route at a time, from wherever `get` reads
    /// them. Starts from what the plugin defaulted every control to, so a
    /// control Lumit has no row for keeps the plugin's own value rather than
    /// nothing.
    fn assemble(&self, get: impl Fn(&ValueRoute) -> Option<Value>) -> ParamSnapshot {
        let mut assembled: BTreeMap<&str, PropValue> = BTreeMap::new();
        for route in &self.routes {
            let Some(value) = get(route) else {
                continue;
            };
            let slot = assembled
                .entry(route.name.as_str())
                .or_insert_with(|| blank(&route.param_type, route.dimension));
            write_component(slot, route, value);
        }
        let mut snapshot = self.defaults.clone();
        for (name, value) in assembled {
            snapshot.set(name, value);
        }
        snapshot
    }
}

/// An empty value of the right shape and width for one OFX parameter.
///
/// A colour is the one that is not one row per component: it crosses as three
/// or four doubles under a single id, so its width comes from the type rather
/// than from the row count.
fn blank(param_type: &str, dimension: usize) -> PropValue {
    match param_type {
        param_types::INTEGER
        | param_types::INTEGER_2D
        | param_types::INTEGER_3D
        | param_types::BOOLEAN
        | param_types::CHOICE => PropValue::Int(vec![0; dimension.max(1)]),
        param_types::RGB => PropValue::Double(vec![0.0; 3]),
        param_types::RGBA => PropValue::Double(vec![0.0; 4]),
        _ => PropValue::Double(vec![0.0; dimension.max(1)]),
    }
}

/// Write one resolved row into the plugin's value for the parameter it is part
/// of. Anything that does not fit the shape is left alone, which leaves the
/// plugin's own default standing — a wrong kind is never a fault (docs/14 §4).
fn write_component(slot: &mut PropValue, route: &ValueRoute, value: Value) {
    match (slot, value) {
        (PropValue::Double(into), Value::Colour(rgba) | Value::Vec4(rgba)) => {
            for (channel, out) in rgba.iter().zip(into.iter_mut()) {
                *out = f64::from(*channel);
            }
        }
        (PropValue::Double(into), Value::Float(v)) => {
            if let Some(out) = into.get_mut(route.component) {
                *out = f64::from(v);
            }
        }
        (PropValue::Double(into), Value::Int(v)) => {
            if let Some(out) = into.get_mut(route.component) {
                *out = f64::from(v);
            }
        }
        (PropValue::Int(into), Value::Int(v)) => {
            if let Some(out) = into.get_mut(route.component) {
                *out = v;
            }
        }
        (PropValue::Int(into), Value::Float(v)) => {
            if let Some(out) = into.get_mut(route.component) {
                *out = v.round() as i32;
            }
        }
        (PropValue::Int(into), Value::Bool(v)) => {
            if let Some(out) = into.get_mut(route.component) {
                *out = i32::from(v);
            }
        }
        (PropValue::Int(into), Value::Choice(v)) => {
            if let Some(out) = into.get_mut(route.component) {
                *out = i32::try_from(v).unwrap_or(0);
            }
        }
        _ => {}
    }
}

/// One row's value out of the plugin's parameter, the reverse of
/// [`write_component`]. A colour is one row for all its channels, everything
/// else is one row per component.
fn read_component(slot: &PropValue, route: &ValueRoute) -> Option<Value> {
    match (slot, route.param_type.as_str()) {
        (PropValue::Double(channels), param_types::RGB | param_types::RGBA) => {
            // An RGB has no fourth channel, and reads as opaque.
            let at = |index: usize| channels.get(index).copied().unwrap_or(1.0) as f32;
            Some(Value::Colour([at(0), at(1), at(2), at(3)]))
        }
        (PropValue::Double(values), _) => Some(Value::Float(*values.get(route.component)? as f32)),
        (PropValue::Int(values), param_types::BOOLEAN) => Some(Value::Bool(*values.first()? != 0)),
        (PropValue::Int(values), param_types::CHOICE) => {
            Some(Value::Choice(u32::try_from(*values.first()?).unwrap_or(0)))
        }
        (PropValue::Int(values), _) => Some(Value::Int(*values.get(route.component)?)),
        _ => None,
    }
}

/// The bag id the memory's hash rides under, so a look the user changed
/// renames the frames it changes.
const DERIVED_MEMORY: ParamId = ParamId::new("derived.memory");

/// The bag id the layer's frame rides under: what the plugin is told the time
/// is, in the frames OFX counts in rather than the seconds the resolve walk
/// speaks. A plugin asks for the frames either side as `time ± 1`, and a
/// time in seconds would make that a second either side.
const DERIVED_FRAME: ParamId = ParamId::new("derived.frame");

/// The frame the plugin is told, out of the bag, or `lt` as it stands when no
/// comp put one there, which is a stack built by hand.
fn frame_in_bag(p: Params<'_>, lt: f64) -> f64 {
    match p.get(DERIVED_FRAME) {
        Some(Value::Int(frame)) => f64::from(frame),
        _ => lt,
    }
}

/// What each instance's plugin keeps beyond its rows, decoded once per change
/// from `EffectInstance::plugin_state` and laid over the snapshot on every
/// render. Keyed by the effect instance, with the hash it was decoded from.
static MEMORY: Mutex<BTreeMap<Uuid, (u64, Arc<ParamSnapshot>)>> = Mutex::new(BTreeMap::new());

/// Keep `inst`'s memory where the render can find it, and answer its hash.
/// Nought for an instance whose plugin keeps nothing.
fn remember(inst: &EffectInstance) -> i32 {
    let mut table = MEMORY.lock();
    let Some(bytes) = inst.plugin_state_bytes() else {
        table.remove(&inst.id);
        return 0;
    };
    let hash = hash_of(&bytes);
    let known = matches!(table.get(&inst.id), Some((seen, _)) if *seen == hash);
    if !known {
        if let Ok(memory) = bincode::deserialize::<ParamSnapshot>(&bytes) {
            table.insert(inst.id, (hash, Arc::new(memory)));
        }
    }
    // Truncated on purpose: the bag carries an int, and any change in the
    // bytes is still a change in the number.
    hash as i32
}

/// Lay a plugin's memory over a snapshot.
fn recall(snapshot: &mut ParamSnapshot, memory: &ParamSnapshot) {
    for (name, value) in memory.iter() {
        snapshot.set(name, value.clone());
    }
}

fn hash_of(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// The pressed effect's picture as the plugin wants it: straight 8-bit rows
/// through the display curve back to linear, premultiplied, half floats.
fn frame_of(source: &PressFrame<'_>) -> Result<Frame16, String> {
    let (width, height) = (source.width as usize, source.height as usize);
    let wanted = width * height * 4;
    let Some(rgba) = source.rgba.get(..wanted) else {
        return Err("the preview frame is short".to_owned());
    };
    let mut pixels = Vec::with_capacity(wanted);
    for pixel in rgba.chunks_exact(4) {
        let alpha = f32::from(pixel[3]) / 255.0;
        for channel in &pixel[..3] {
            pixels.push(lumit_core::pixels::srgb_decode(*channel) * alpha);
        }
        pixels.push(alpha);
    }
    Frame16::from_f32(width, height, &pixels)
        .map_err(|status| format!("the preview frame would not convert ({status:?})"))
}

impl EffectDef for OfxEffectDef {
    fn schema(&self) -> &'static EffectSchema {
        self.schema
    }

    /// A stack built by hand — an oracle, a test — names no instance and no
    /// time. Both are legitimate: the host owns every value, so the bag alone is
    /// enough to render from, and the nil id is one instance like any other.
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
        let expected = (w as usize) * (h as usize) * 4;
        if w == 0 || h == 0 || rgba.len() < expected {
            return;
        }
        let Ok(source) = Frame16::from_f32(w as usize, h as usize, &rgba[..expected]) else {
            return;
        };
        // A neighbour of another size is one the plugin could not compare
        // with the frame in hand, and is left out: for that time the plugin
        // gets the frame in hand, the spec's answer to a frame the host has
        // not got.
        let frames: Vec<(i32, Frame16)> = neighbours
            .iter()
            .filter(|(_, pixels)| pixels.len() == expected)
            .filter_map(|(offset, pixels)| {
                let frame = Frame16::from_f32(w as usize, h as usize, pixels).ok()?;
                Some((*offset, frame))
            })
            .collect();
        let mut snapshot = self.snapshot(p);
        let memory = MEMORY
            .lock()
            .get(&inst)
            .map(|(_, memory)| Arc::clone(memory));
        if let Some(memory) = memory {
            recall(&mut snapshot, &memory);
        }
        let rendered = self
            .host
            .render(inst, frame_in_bag(p, lt), &snapshot, source, &frames);
        let failed = rendered.error.is_some();
        LAST_ERROR.with(|slot| *slot.borrow_mut() = rendered.error);
        if failed {
            // **Identity, byte for byte**. A failed render hands back the
            // input, and the input is already in `rgba` — writing it again
            // would put it through the fp16 boundary for nothing, and a
            // disabled plugin would then change the picture very slightly,
            // which is the one thing "renders as identity" must not mean.
            return;
        }
        for (pixel, out) in rendered
            .frame
            .pixels()
            .iter()
            .zip(rgba[..expected].iter_mut())
        {
            *out = f32::from(*pixel);
        }
    }

    fn frames_needed(&self, inst: &EffectInstance, frame: f64) -> Option<Vec<i32>> {
        self.host.frames_needed(inst.id, frame, &self.defaults)
    }

    fn last_error(&self) -> Option<String> {
        LAST_ERROR.with(|slot| slot.borrow_mut().take())
    }

    /// The plugin's memory reaches the render from here: this is the one hook
    /// that sees the document's instance every frame, so it files the memory
    /// for `apply_cpu_temporal` and puts its hash in the bag for the frame key.
    /// The layer's frame rides in beside it, because this is also the one hook
    /// that can see the comp's rate.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(DERIVED_MEMORY, Value::Int(remember(cx.inst)));
        let fps = cx
            .context
            .comp
            .and_then(|id| cx.context.document.comp(id))
            .map(|comp| comp.frame_rate.fps());
        if let Some(fps) = fps {
            push(DERIVED_FRAME, Value::Int((cx.lt * fps).round() as i32));
        }
    }

    fn press(
        &self,
        inst: &EffectInstance,
        lt: f64,
        name: &str,
        source: &PressFrame<'_>,
    ) -> Result<Pressed, String> {
        let route = self
            .routes
            .iter()
            .find(|route| route.row == name && route.param_type == param_types::PUSH_BUTTON)
            .ok_or_else(|| format!("the plugin has no button called {name}"))?;
        let before = self.snapshot_of(inst, lt);
        let frame = frame_of(source)?;
        let after = self.host.press(inst.id, lt, &before, &route.name, frame)?;
        Ok(Pressed {
            rows: self.rows_written(&before, &after),
            memory: self.memory_of(&after),
        })
    }
}

// ----------------------------------------------------------- the two hosts --

/// The frame offsets an OFX absolute frame range comes to, relative to `time`.
///
/// OFX answers `getFramesNeeded` in absolute frames per clip; the neighbour
/// machinery counts in offsets from the frame being rendered. The range is
/// widened outwards to whole frames — a plugin asking for `t − 2.5` is asking
/// for a frame between two of ours, and fetching both is the answer that never
/// leaves it short. `None` when every clip wants only the current frame, which
/// is the honest "nothing more specific to say than the declaration".
fn offsets_of(frames: &BTreeMap<String, (f64, f64)>, time: f64) -> Option<Vec<i32>> {
    let mut offsets = vec![0i32];
    for (first, last) in frames.values() {
        let (low, high) = ((first - time).floor(), (last - time).ceil());
        if !low.is_finite() || !high.is_finite() {
            continue;
        }
        // A plugin asking for a thousand frames either side is asking for more
        // than the neighbour decode will ever hold; the ring's own byte budget
        // (docs/impl/ofx-host.md §4) is what refuses it, and clamping here keeps
        // this from building the list that would then be refused.
        let (low, high) = (low.max(-64.0) as i32, high.min(64.0) as i32);
        offsets.extend(low..=high);
    }
    offsets.sort_unstable();
    offsets.dedup();
    (offsets.len() > 1).then_some(offsets)
}

/// A plugin hosted **in this process** — the bundle is loaded here and its code
/// runs on this thread.
///
/// Not the shipping arrangement: docs/12 §2.3 puts a third-party plugin in a
/// process of its own, and [`BrokerHost`] is that. This is the arrangement for a
/// bundle Lumit itself ships, and the one the catalogue tests use, because
/// proving that a plugin becomes an effect needs no second process.
pub struct LocalHost {
    bundle: Bundle,
    descriptor: PluginDescriptor,
    context: Context,
    /// One live plugin instance per effect instance, made on first use and kept
    /// for the session. A plugin may hold private state per instance, and
    /// rebuilding it every frame would throw that away once a frame.
    instances: Mutex<BTreeMap<Uuid, Instance>>,
}

impl LocalHost {
    /// Host one described plugin out of an already-loaded bundle.
    ///
    /// The context is the first one the describe found drivable, which is the
    /// order the descriptor's list is in.
    #[must_use]
    pub fn new(bundle: Bundle, descriptor: PluginDescriptor) -> Self {
        let context = descriptor
            .contexts
            .first()
            .copied()
            .unwrap_or(Context::Filter);
        Self {
            bundle,
            descriptor,
            context,
            instances: Mutex::new(BTreeMap::new()),
        }
    }

    /// Run one render sequence, or say why it could not be run.
    fn attempt(
        &self,
        instance: Uuid,
        request: &RenderRequest,
        params: &ParamSnapshot,
    ) -> Result<Rendered, String> {
        self.with_instance(instance, params, |plugin, live| {
            let token = lumit_eval::epoch::Epoch::new().token();
            crate::render::render(plugin, live, request, &token).map_err(|error| error.to_string())
        })
    }

    /// The plugin and the live instance, made on first use and holding
    /// `params`, handed to `call`.
    fn with_instance<T>(
        &self,
        instance: Uuid,
        params: &ParamSnapshot,
        call: impl FnOnce(&PluginRef, &Instance) -> Result<T, String>,
    ) -> Result<T, String> {
        let plugin = self
            .bundle
            .plugins()
            .iter()
            .find(|plugin| plugin.identifier == self.descriptor.identifier)
            .ok_or_else(|| "the plugin is no longer in the bundle".to_owned())?;

        // The lock is over the **pool**, and the render inside it is what the
        // plugin's own declared thread safety serialises (`render_lock` is
        // where that happens). No host state lock is held across the call into
        // the plugin, which is docs/14 §7's rule.
        let mut pool = self.instances.lock();
        if let std::collections::btree_map::Entry::Vacant(slot) = pool.entry(instance) {
            let made = Instance::create(plugin, &self.descriptor, self.context, params)
                .map_err(|status| format!("the plugin refused an instance ({status:?})"))?;
            slot.insert(made);
        }
        let live = pool
            .get(&instance)
            .ok_or_else(|| "the instance vanished".to_owned())?;
        live.set_params(params.clone())
            .map_err(|status| format!("the values would not go in ({status:?})"))?;
        call(plugin, live)
    }
}

impl PluginHost for LocalHost {
    fn render(
        &self,
        instance: Uuid,
        time: f64,
        params: &ParamSnapshot,
        source: Frame16,
        neighbours: &[(i32, Frame16)],
    ) -> Rendering {
        let mut request = RenderRequest::filter(time, source.clone());
        request.neighbours = neighbours.to_vec();
        match self.attempt(instance, &request, params) {
            Ok(rendered) => Rendering {
                frame: rendered.frame,
                error: None,
            },
            Err(why) => Rendering {
                frame: source,
                error: Some(why),
            },
        }
    }

    fn frames_needed(&self, instance: Uuid, time: f64, params: &ParamSnapshot) -> Option<Vec<i32>> {
        // A one-pixel frame: the question is which *times* the plugin wants, and
        // it is answered before any picture is looked at. The pixels that come
        // back are thrown away.
        let source = Frame16::black(1, 1).ok()?;
        let request = RenderRequest::filter(time, source);
        let rendered = self.attempt(instance, &request, params).ok()?;
        offsets_of(&rendered.frames_needed, time)
    }

    fn press(
        &self,
        instance: Uuid,
        time: f64,
        params: &ParamSnapshot,
        name: &str,
        source: Frame16,
    ) -> Result<ParamSnapshot, String> {
        self.with_instance(instance, params, |plugin, live| {
            live.press(plugin, name, time, &source)
                .map_err(|status| format!("the plugin refused the press ({status:?})"))
        })
    }
}

/// One bundle's broker, shared by the plugins in it, and whether one of them
/// is inside a press right now.
pub struct SharedBroker {
    broker: Mutex<Broker>,
    /// Set for as long as a plugin is inside a press. A render that finds it
    /// set answers identity with a badge instead of queueing behind a window
    /// that may stay open for minutes.
    pressing: AtomicBool,
    /// How long a render waits its turn for the broker before it gives up,
    /// the broker's own render deadline.
    patience: Duration,
}

impl SharedBroker {
    /// Wrap a spawned, described broker.
    #[must_use]
    pub fn new(broker: Broker) -> Self {
        Self {
            patience: broker.render_timeout(),
            broker: Mutex::new(broker),
            pressing: AtomicBool::new(false),
        }
    }

    /// The broker, or `None` while a plugin in this bundle is in a press or
    /// the wait ran past the deadline.
    fn lock(&self) -> Option<parking_lot::MutexGuard<'_, Broker>> {
        if self.pressing.load(Ordering::Acquire) {
            return None;
        }
        self.broker.try_lock_for(self.patience)
    }
}

/// What a render answers while the bundle's plugin is in its own window.
const BUSY: &str = "the plugin is busy in its own window";

/// A plugin hosted **in a broker process** — the shipping arrangement
/// (docs/12 §2.3).
///
/// One broker per bundle behind one lock, because a broker owns a pipe and a
/// shared-memory ring and both are single-conversation things. That lock is the
/// bundle serialisation an unsafe plugin needs anyway; a fully safe plugin pays
/// for it too, which is the recorded ceiling — the parallelism is across
/// *brokers* until one broker can carry two conversations at once.
pub struct BrokerHost {
    /// **Shared**, because a bundle holds many plugins and docs/12 §2.3 puts
    /// one broker process behind the bundle rather than behind each of them:
    /// openfx-misc alone would otherwise be eighty processes. The lock is the
    /// bundle serialisation described above, now also serialising the bundle's
    /// plugins against each other.
    broker: Arc<SharedBroker>,
    /// Which plugin of the bundle, by the index the broker's describe used.
    plugin: u32,
    context: Context,
    /// The broker's own instance ids, by effect instance.
    instances: Mutex<BTreeMap<Uuid, InstanceId>>,
}

impl BrokerHost {
    /// Host one plugin of an already-spawned, already-described broker.
    #[must_use]
    pub fn new(broker: Arc<SharedBroker>, plugin: u32, context: Context) -> Self {
        Self {
            broker,
            plugin,
            context,
            instances: Mutex::new(BTreeMap::new()),
        }
    }

    /// The broker's id for this effect instance, making it if this is the first
    /// frame it has been asked for.
    fn instance_of(
        &self,
        broker: &mut Broker,
        instance: Uuid,
        params: &ParamSnapshot,
    ) -> Option<InstanceId> {
        let mut known = self.instances.lock();
        if let Some(id) = known.get(&instance) {
            return Some(*id);
        }
        let id = broker
            .create_instance(self.plugin, self.context, params.clone())
            .ok()?;
        known.insert(instance, id);
        Some(id)
    }
}

/// Whatever the plugin asked us to *say*, filed after the broker is let go: the
/// message log is another process-wide lock, and taking two of those nested is
/// the deadlock shape (docs/14 §7) even when neither call reaches a plugin.
/// Never modal, the frontend draws these as a calm toast (docs/12 §2.2's open
/// question, answered this way for now).
fn file(notes: Vec<(String, String)>) {
    for (kind, text) in notes {
        crate::host::state().push_message(crate::host::HostMessage {
            message_type: kind,
            message_id: String::new(),
            text,
        });
    }
}

impl PluginHost for BrokerHost {
    fn render(
        &self,
        instance: Uuid,
        time: f64,
        params: &ParamSnapshot,
        source: Frame16,
        neighbours: &[(i32, Frame16)],
    ) -> Rendering {
        let Some(mut broker) = self.broker.lock() else {
            return Rendering {
                frame: source,
                error: Some(BUSY.to_owned()),
            };
        };
        let Some(id) = self.instance_of(&mut broker, instance, params) else {
            return Rendering {
                frame: source,
                error: Some("the plugin would not make an instance".to_owned()),
            };
        };
        let _ = broker.set_params(id, params.clone());
        let mut request = RenderRequest::filter(time, source.clone());
        // The layer's decoded neighbours go across with the frame. A plugin
        // that asks for a frame beyond them gets the frame in hand, which is
        // what the spec says an unfetchable clip gives.
        request.neighbours = neighbours.to_vec();
        let answer = broker.render(id, &request, &|_, _| None);
        let notes = broker.take_notes();
        drop(broker);
        file(notes);
        match answer {
            Ok(answer) => Rendering {
                frame: answer.frame,
                error: answer.error,
            },
            Err(error) => Rendering {
                frame: source,
                error: Some(error.to_string()),
            },
        }
    }

    fn frames_needed(&self, instance: Uuid, time: f64, params: &ParamSnapshot) -> Option<Vec<i32>> {
        let mut broker = self.broker.lock()?;
        if broker.is_disabled() {
            return None;
        }
        let id = self.instance_of(&mut broker, instance, params)?;
        let source = Frame16::black(1, 1).ok()?;
        let request = RenderRequest::filter(time, source);
        let answer = broker.render(id, &request, &|_, _| None).ok()?;
        offsets_of(&answer.frames_needed, time)
    }

    fn press(
        &self,
        instance: Uuid,
        time: f64,
        params: &ParamSnapshot,
        name: &str,
        source: Frame16,
    ) -> Result<ParamSnapshot, String> {
        // A render in flight finishes first, then the flag goes up so no other
        // render queues behind the plugin's window.
        let mut broker = self.broker.broker.lock();
        let id = self
            .instance_of(&mut broker, instance, params)
            .ok_or_else(|| "the plugin would not make an instance".to_owned())?;
        broker
            .set_params(id, params.clone())
            .map_err(|error| error.to_string())?;
        self.broker.pressing.store(true, Ordering::Release);
        let answer = broker.press(id, name, time, &source);
        self.broker.pressing.store(false, Ordering::Release);
        let notes = broker.take_notes();
        drop(broker);
        file(notes);
        answer.map_err(|error| error.to_string())
    }
}
