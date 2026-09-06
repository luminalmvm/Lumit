//! Instances: a live copy of an effect, with values in its controls.
//!
//! # In plain terms
//!
//! **Describe** ([`crate::describe`]) is the plugin saying what it *is*. An
//! instance is one copy of it, sitting on a layer, with a number in every
//! control and a picture to work on. A plugin may have a hundred instances
//! alive at once and they share nothing but the code.
//!
//! Three things make an instance, and the order they arrive in is not
//! negotiable (docs/impl/ofx-host.md §3):
//!
//! 1. Its **parameters exist first, with their defaults in them**. The plugin's
//!    `kOfxActionCreateInstance` handler will read them, and a host that
//!    creates the instance and fills the controls afterwards has plugins that
//!    cache a nonsense value on their first breath.
//! 2. Its **clips** — the image inputs and the output. Each gets a handle of
//!    its own, unlike during describe where a clip is only a property bag.
//! 3. Only then `kOfxActionCreateInstance`, and at the end of its life
//!    `kOfxActionDestroyInstance`.
//!
//! **The values are the host's, not the plugin's.** Lumit owns parameter
//! storage, animation and expressions (docs/12 §1, §2.2), so an instance
//! carries a [`ParamSnapshot`] — what every control reads at the moment this
//! evaluation was scheduled — and every `paramGetValue` is answered out of it.
//! The plugin has no store of its own to consult and is never asked for one;
//! that is what makes a render reproducible from the document alone.
//!
//! **Thread safety is the plugin's to declare and the host's to obey.** A
//! plugin says at describe time whether two of its renders may run at once
//! ([`ThreadSafety`]); the render driver takes the matching lock, or none.
//! Claiming more safety than it has is the plugin's bug, but obeying a claim of
//! less is the host's job, and docs/12 §2.3 spells out all three answers.

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::bundle::PluginRef;
use crate::describe::{
    base_property_set, new_descriptor, release_descriptor, ClipBinding, ClipRef, Context,
    ParamRecord, ParamRef, PluginDescriptor,
};
use crate::ffi::{actions, prop_keys as keys, prop_values as values};
use crate::handles::Handle;
use crate::host::state;
use crate::image::{Frame16, Image, RectI, RowOrder};
use crate::props::{PropValue, PropertySet};
use crate::render::{OUTPUT_CLIP, SOURCE_CLIP};
use crate::status::Status;

/// What a plugin says about running two renders at once
/// (`kOfxImageEffectPluginRenderThreadSafety`, docs/12 §2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadSafety {
    /// `kOfxImageEffectRenderFullySafe` — any number of renders, any number of
    /// instances, all at once.
    FullySafe,
    /// `kOfxImageEffectRenderInstanceSafe` — instances run in parallel, but one
    /// instance renders one frame at a time.
    InstanceSafe,
    /// `kOfxImageEffectRenderUnsafe` — one render at a time across the whole
    /// bundle.
    Unsafe,
}

impl ThreadSafety {
    /// Read the declaration, defaulting to the **most pessimistic** answer for
    /// anything unrecognised. An undeclared effect is treated as the worst case
    /// (docs/13 §6), and a plugin that meant to be fast will have said so.
    #[must_use]
    pub fn from_ofx_name(name: &str) -> Self {
        match name {
            values::RENDER_THREAD_SAFETY_FULLY_SAFE => Self::FullySafe,
            values::RENDER_THREAD_SAFETY_INSTANCE_SAFE => Self::InstanceSafe,
            _ => Self::Unsafe,
        }
    }

    /// The OFX name.
    #[must_use]
    pub const fn ofx_name(self) -> &'static str {
        match self {
            Self::FullySafe => values::RENDER_THREAD_SAFETY_FULLY_SAFE,
            Self::InstanceSafe => values::RENDER_THREAD_SAFETY_INSTANCE_SAFE,
            Self::Unsafe => values::RENDER_THREAD_SAFETY_UNSAFE,
        }
    }
}

/// The one lock every `kOfxImageEffectRenderUnsafe` plugin queues behind.
///
/// Process-wide rather than per-bundle because this package hosts in process
/// and there is one bundle in it; docs/12 §2.3 puts the real boundary at the
/// bundle, which is a process of its own once the broker lands, and then this
/// lock *is* the bundle's.
static UNSAFE_PLUGIN_LOCK: Mutex<()> = Mutex::new(());

/// The lock a render of this instance must hold, if any.
pub(crate) fn render_lock(safety: ThreadSafety, instance: &Arc<Mutex<()>>) -> Option<RenderGuard> {
    match safety {
        ThreadSafety::FullySafe => None,
        ThreadSafety::InstanceSafe => Some(RenderGuard::Instance(instance.clone())),
        ThreadSafety::Unsafe => Some(RenderGuard::Bundle),
    }
}

/// A held render lock. Taking it is [`RenderGuard::hold`], because the lock is
/// chosen before the render and taken around it.
pub(crate) enum RenderGuard {
    /// One instance's own lock.
    Instance(Arc<Mutex<()>>),
    /// The whole bundle's.
    Bundle,
}

impl RenderGuard {
    /// Block until the render may proceed, and hand back the guard that holds
    /// it. **No host state lock may be held across this** — the render it
    /// guards calls into a plugin, which re-enters the suites (docs/14 §7).
    pub(crate) fn hold(&self) -> HeldGuard<'_> {
        match self {
            Self::Instance(lock) => HeldGuard::Instance(lock.lock()),
            Self::Bundle => HeldGuard::Bundle(UNSAFE_PLUGIN_LOCK.lock()),
        }
    }
}

/// The live half of [`RenderGuard`]; dropping it lets the next render in.
///
/// Nothing reads the guard inside — holding it *is* the effect, which is what
/// the allow below says out loud rather than papering over.
#[allow(dead_code)]
pub(crate) enum HeldGuard<'a> {
    /// One instance's lock, borrowed from the guard that chose it.
    Instance(parking_lot::MutexGuard<'a, ()>),
    /// The bundle's, which is a static and so borrows nothing.
    Bundle(parking_lot::MutexGuard<'static, ()>),
}

/// Every control's value at the moment this evaluation was scheduled.
///
/// The values are [`PropValue`]s because that is already the shape OFX asks
/// for them in — an array of ints, doubles or one string — and inventing a
/// second spelling of the same four cases would only be the first one with the
/// names changed.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ParamSnapshot {
    values: BTreeMap<String, PropValue>,
}

impl ParamSnapshot {
    /// An empty snapshot.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The snapshot a freshly created instance starts from: every parameter at
    /// the default the plugin declared for it.
    #[must_use]
    pub fn from_defaults(descriptor: &PluginDescriptor) -> Self {
        let mut values = BTreeMap::new();
        for param in &descriptor.params {
            if let Ok(default) = param.props.get(keys::PARAM_DEFAULT) {
                values.insert(param.name.clone(), default.clone());
            }
        }
        Self { values }
    }

    /// Set one control's value, replacing whatever was there.
    pub fn set(&mut self, name: &str, value: PropValue) {
        self.values.insert(name.to_owned(), value);
    }

    /// One control's value.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PropValue> {
        self.values.get(name)
    }

    /// Every control and its value, by name.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &PropValue)> {
        self.values.iter()
    }

    /// Whether there is nothing in it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// The live state hanging off an instance's handle, beside the clips and
/// parameters it shares its shape with a descriptor.
pub struct InstanceState {
    /// Which context it was created in.
    pub context: Context,
    /// What every control reads.
    pub params: ParamSnapshot,
    /// What the plugin said about running two renders at once.
    pub thread_safety: ThreadSafety,
    /// The pictures for the render **currently in flight**, by clip name.
    ///
    /// Empty at every other moment. `clipGetImage` answers out of this map and
    /// nothing else, so a plugin that squirrels a clip handle away and fetches
    /// an image outside a render gets `kOfxStatFailed`, which is the truth:
    /// there is no frame to give it.
    pub images: BTreeMap<String, Image>,
    /// The pictures for **other times** than the one being rendered, keyed by
    /// clip and by [`time_key`].
    ///
    /// This is what a retimer asks for: `getFramesNeeded` says which frames it
    /// wants and `clipGetImage(clip, t)` fetches them one at a time. They are
    /// prefetched in one shipment by the broker (docs/impl/ofx-host.md §4);
    /// in-process there is nothing to prefetch from, so the map is empty and
    /// every time answers with the frame in hand.
    pub images_at: BTreeMap<(String, i64), Image>,
    /// This instance's own render lock, for a `kOfxImageEffectRenderInstanceSafe`
    /// plugin.
    pub lock: Arc<Mutex<()>>,
}

/// One live instance of one plugin.
///
/// Destroying it is [`Instance::destroy`] and not a `Drop`: the plugin has to
/// be told, and telling it means calling into it, which a destructor is the
/// wrong place to do — a `Drop` that dispatches an action would run at
/// unpredictable moments, possibly while the host lock is held.
pub struct Instance {
    handle: Handle,
    thread_safety: ThreadSafety,
    lock: Arc<Mutex<()>>,
}

impl Instance {
    /// Build an instance and tell the plugin about it.
    ///
    /// The order here is the order in the module note: property sets, then
    /// parameters at their defaults, then clips, then the action. `values`
    /// overrides the defaults for the controls the host has values for, and is
    /// applied **before** `kOfxActionCreateInstance` so the plugin's first read
    /// is the real number.
    ///
    /// # Errors
    ///
    /// [`Status`] from the host if the instance could not be built, or the
    /// plugin's own failure status from `kOfxActionCreateInstance` — in which
    /// case everything this made is released before the error comes back.
    pub fn create(
        plugin: &PluginRef,
        descriptor: &PluginDescriptor,
        context: Context,
        values: &ParamSnapshot,
    ) -> Result<Self, Status> {
        let thread_safety = ThreadSafety::from_ofx_name(
            descriptor
                .render_thread_safety
                .as_deref()
                .unwrap_or(values::RENDER_THREAD_SAFETY_UNSAFE),
        );

        let mut props = base_property_set(&descriptor.identifier);
        seed_instance_properties(&mut props, descriptor, context, thread_safety);
        let handle = new_descriptor(props)?;

        let mut snapshot = ParamSnapshot::from_defaults(descriptor);
        for param in &descriptor.params {
            if let Some(value) = values.get(&param.name) {
                snapshot.set(&param.name, value.clone());
            }
        }

        // Everything below can fail, and a half-built instance must not be left
        // in the registry, so the one failure path releases it.
        let built = build(handle, descriptor, context, snapshot, thread_safety);
        if built.is_err() {
            release_descriptor(handle);
        }
        let lock = built?;

        let status = plugin.action(actions::CREATE_INSTANCE, Some(handle), None, None);
        if !matches!(status, Status::Ok | Status::ReplyDefault) {
            release_descriptor(handle);
            return Err(status);
        }
        Ok(Self {
            handle,
            thread_safety,
            lock,
        })
    }

    /// The handle the plugin holds.
    #[must_use]
    pub const fn handle(&self) -> Handle {
        self.handle
    }

    /// What the plugin declared about concurrent renders.
    #[must_use]
    pub const fn thread_safety(&self) -> ThreadSafety {
        self.thread_safety
    }

    /// The lock a render of this instance queues behind, if any.
    pub(crate) fn render_lock(&self) -> Option<RenderGuard> {
        render_lock(self.thread_safety, &self.lock)
    }

    /// Replace every control's value **without** telling the plugin.
    ///
    /// This is a scrub, an undo, or a keyframe moving under a playhead: the
    /// values are the host's and the plugin reads them at its next
    /// `paramGetValue` (docs/12 §2.2). `kOfxActionInstanceChanged` is for a
    /// person turning a knob, which is [`Instance::changed`].
    ///
    /// # Errors
    ///
    /// [`Status::ErrBadHandle`] if the instance is gone.
    pub fn set_params(&self, params: ParamSnapshot) -> Result<(), Status> {
        let mut state = state();
        let instance = state
            .effects
            .get_mut(self.handle)?
            .instance
            .as_mut()
            .ok_or(Status::ErrBadHandle)?;
        instance.params = params;
        Ok(())
    }

    /// Every control's value as the plugin last left it. After a press this is
    /// where whatever the plugin wrote from its own window ends up.
    ///
    /// # Errors
    ///
    /// [`Status::ErrBadHandle`] if the instance is gone.
    pub fn params(&self) -> Result<ParamSnapshot, Status> {
        let state = state();
        let instance = state
            .effects
            .get(self.handle)?
            .instance
            .as_ref()
            .ok_or(Status::ErrBadHandle)?;
        Ok(instance.params.clone())
    }

    /// Replace one control's value and tell the plugin, wrapped as the spec
    /// requires: `kOfxActionBeginInstanceChanged`, the change, then
    /// `kOfxActionEndInstanceChanged`. Sapphire relies on the wrapping
    /// (docs/impl/ofx-host.md §3), and **it must never happen inside a render**
    /// — which is why this is a method on the instance and not something the
    /// render driver can reach.
    ///
    /// # Errors
    ///
    /// The plugin's own failure status. The value is changed either way: it is
    /// the host's, and a plugin that dislikes it does not get to veto it.
    pub fn changed(
        &self,
        plugin: &PluginRef,
        name: &str,
        value: PropValue,
        reason: &str,
        time: f64,
    ) -> Result<(), Status> {
        {
            let mut state = state();
            let instance = state
                .effects
                .get_mut(self.handle)?
                .instance
                .as_mut()
                .ok_or(Status::ErrBadHandle)?;
            instance.params.set(name, value);
        }

        let mut wrapper = PropertySet::new();
        if let Ok(reason) = PropValue::string(reason) {
            wrapper.seed(keys::CHANGE_REASON, reason);
        }
        let wrapper = state().props.insert(wrapper)?;

        let mut in_args = PropertySet::new();
        if let Ok(kind) = PropValue::string(values::TYPE_PARAMETER) {
            in_args.seed(keys::TYPE, kind);
        }
        if let Ok(name) = PropValue::string(name) {
            in_args.seed(keys::NAME, name);
        }
        if let Ok(reason) = PropValue::string(reason) {
            in_args.seed(keys::CHANGE_REASON, reason);
        }
        in_args.seed(keys::TIME, PropValue::double(time));
        in_args.seed(keys::RENDER_SCALE, PropValue::Double(vec![1.0, 1.0]));
        let in_args = state().props.insert(in_args)?;

        let begin = plugin.action(
            actions::BEGIN_INSTANCE_CHANGED,
            Some(self.handle),
            Some(wrapper),
            None,
        );
        let changed = plugin.action(
            actions::INSTANCE_CHANGED,
            Some(self.handle),
            Some(in_args),
            None,
        );
        let end = plugin.action(
            actions::END_INSTANCE_CHANGED,
            Some(self.handle),
            Some(wrapper),
            None,
        );

        {
            let mut state = state();
            let _ = state.props.remove(in_args);
            let _ = state.props.remove(wrapper);
        }

        for status in [begin, changed, end] {
            if !matches!(status, Status::Ok | Status::ReplyDefault) {
                return Err(status);
            }
        }
        Ok(())
    }

    /// Press one of the plugin's buttons and hand back every value afterwards.
    ///
    /// A button is `kOfxActionInstanceChanged` like any other control, but the
    /// plugin may do anything at all inside it. Magic Bullet Looks opens its
    /// whole editor and stays there until the user closes it. So the frame goes
    /// on first, since that editor asks the Source clip for a preview and will
    /// not open without one, and the values come back after, since a look is
    /// stored by the plugin writing its own parameters.
    ///
    /// # Errors
    ///
    /// The plugin's own failure status, or [`Status::ErrBadHandle`] if the
    /// instance is gone.
    pub fn press(
        &self,
        plugin: &PluginRef,
        name: &str,
        time: f64,
        source: &Frame16,
    ) -> Result<ParamSnapshot, Status> {
        let (width, height) = (source.width(), source.height());
        let bounds = RectI::sized(
            i32::try_from(width).map_err(|_| Status::ErrValue)?,
            i32::try_from(height).map_err(|_| Status::ErrValue)?,
        );
        let mut images = BTreeMap::new();
        images.insert(
            SOURCE_CLIP.to_owned(),
            Image::from_frame(source, RowOrder::BottomUp)?,
        );
        images.insert(
            OUTPUT_CLIP.to_owned(),
            Image::black(bounds, RowOrder::BottomUp)?,
        );
        set_images(self.handle, images, BTreeMap::new())?;
        set_project_size(self.handle, width as f64, height as f64)?;
        let outcome = self.changed(
            plugin,
            name,
            PropValue::Int(vec![0]),
            values::CHANGE_USER_EDITED,
            time,
        );
        // Off again whatever the plugin did, so the next render starts clean.
        let _ = take_images(self.handle);
        outcome?;
        self.params()
    }

    /// Tell the plugin the instance is going, and take everything down.
    ///
    /// Consuming, because a destroyed instance is not a thing anyone may hold
    /// a second reference to; the handles are dead from here on and every
    /// suite call against them answers `kOfxStatErrBadHandle`.
    ///
    /// # Errors
    ///
    /// The plugin's own failure status. Everything is released regardless — a
    /// plugin that objects to being destroyed is still destroyed.
    pub fn destroy(self, plugin: &PluginRef) -> Result<(), Status> {
        let status = plugin.action(actions::DESTROY_INSTANCE, Some(self.handle), None, None);
        release_descriptor(self.handle);
        if matches!(status, Status::Ok | Status::ReplyDefault) {
            Ok(())
        } else {
            Err(status)
        }
    }
}

/// Fill in the parameters, the clips and the instance state. Split out so the
/// one failure path in [`Instance::create`] has one thing to unwind.
fn build(
    handle: Handle,
    descriptor: &PluginDescriptor,
    context: Context,
    params: ParamSnapshot,
    thread_safety: ThreadSafety,
) -> Result<Arc<Mutex<()>>, Status> {
    let lock = Arc::new(Mutex::new(()));
    let mut state = state();

    for param in &descriptor.params {
        let props = state.props.insert(param.props.clone())?;
        let param_handle = state.params.insert(ParamRecord {
            props,
            effect: handle,
            name: param.name.clone(),
        })?;
        state.effects.get_mut(handle)?.params.push(ParamRef {
            name: param.name.clone(),
            param_type: param.param_type.clone(),
            handle: param_handle,
            props,
        });
    }

    for clip in &descriptor.clips {
        let mut clip_props = clip.props.clone();
        seed_clip_instance_properties(&mut clip_props, &clip.name);
        let props = state.props.insert(clip_props)?;
        let clip_handle = state.clips.insert(ClipBinding {
            effect: handle,
            name: clip.name.clone(),
            props,
        })?;
        state.effects.get_mut(handle)?.clips.push(ClipRef {
            name: clip.name.clone(),
            props,
            handle: Some(clip_handle),
        });
    }

    state.effects.get_mut(handle)?.instance = Some(InstanceState {
        context,
        params,
        thread_safety,
        images: BTreeMap::new(),
        images_at: BTreeMap::new(),
        lock: Arc::clone(&lock),
    });
    Ok(lock)
}

/// The properties an instance's own set carries beyond what the descriptor
/// said: what it is, which context it is in, and the project it sits in.
///
/// The project numbers are the honest ones for this package — a unit pixel
/// aspect, a project the size of the frame — and become the real comp's when
/// the engine hands a render down.
fn seed_instance_properties(
    props: &mut PropertySet,
    descriptor: &PluginDescriptor,
    context: Context,
    thread_safety: ThreadSafety,
) {
    let mut seed_string = |key: &str, value: &str| {
        if let Ok(value) = PropValue::string(value) {
            props.seed(key, value);
        }
    };
    seed_string(keys::TYPE, values::TYPE_IMAGE_EFFECT_INSTANCE);
    seed_string(keys::CONTEXT, context.ofx_name());
    seed_string(keys::LABEL, &descriptor.label);
    seed_string(keys::GROUPING, &descriptor.grouping);
    seed_string(keys::PLUGIN_RENDER_THREAD_SAFETY, thread_safety.ofx_name());

    props.seed(keys::IS_INTERACTIVE, PropValue::int(0));
    // **The project's size, and this instance's own tiles answer.** The OFX
    // support library reads all of these when a plugin is constructed, and a
    // plugin that cannot find one of them throws before it exists — six of the
    // conformance bench's plugins died on `ProjectExtent` alone.
    //
    // The numbers here are a standing default; [`set_project_size`] replaces
    // them with the frame actually being rendered before the plugin is asked
    // anything, because a generator places itself by them.
    props.seed(
        keys::PROJECT_SIZE,
        PropValue::Double(vec![DEFAULT_PROJECT.0, DEFAULT_PROJECT.1]),
    );
    props.seed(
        keys::PROJECT_EXTENT,
        PropValue::Double(vec![DEFAULT_PROJECT.0, DEFAULT_PROJECT.1]),
    );
    // One frame until the first render: Lumit renders a frame at a time and
    // tells a plugin nothing about the layer it sits on (docs/12 §2.1), so the
    // clip is as long as the frames in hand, which [`set_frame_range`] says
    // before every render.
    props.seed(keys::EFFECT_DURATION, PropValue::double(1.0));
    props.seed(keys::SUPPORTS_TILES, PropValue::int(0));
    props.seed(keys::FRAME_RATE, PropValue::double(25.0));
    props.seed(keys::PROJECT_OFFSET, PropValue::Double(vec![0.0, 0.0]));
    props.seed(keys::PROJECT_PIXEL_ASPECT_RATIO, PropValue::double(1.0));
    props.seed(keys::SEQUENTIAL_RENDER, PropValue::int(0));
    props.seed(
        keys::TEMPORAL_CLIP_ACCESS,
        PropValue::int(i32::from(descriptor.temporal)),
    );
}

/// The properties a clip gains when it stops being a description and becomes
/// an input. Every one is a promise: float RGBA, premultiplied, square pixels,
/// no fields — which is exactly what the host table says and what
/// [`crate::image`] hands over. Only the Source and the Output are connected:
/// this host feeds one input, and a plugin's optional second one has nothing
/// on it, which [`set_clip_state`] says again from the request before every
/// render.
fn seed_clip_instance_properties(props: &mut PropertySet, name: &str) {
    let mut seed_string = |key: &str, value: &str| {
        if let Ok(value) = PropValue::string(value) {
            props.seed(key, value);
        }
    };
    seed_string(keys::PIXEL_DEPTH, values::BIT_DEPTH_FLOAT);
    seed_string(keys::COMPONENTS, values::COMPONENT_RGBA);
    seed_string(keys::CLIP_UNMAPPED_COMPONENTS, values::COMPONENT_RGBA);
    seed_string(keys::CLIP_UNMAPPED_PIXEL_DEPTH, values::BIT_DEPTH_FLOAT);
    seed_string(keys::PRE_MULTIPLICATION, values::IMAGE_PRE_MULTIPLIED);
    seed_string(keys::CLIP_FIELD_ORDER, values::IMAGE_FIELD_NONE);

    let connected = name == SOURCE_CLIP || name == OUTPUT_CLIP;
    props.seed(keys::CLIP_CONNECTED, PropValue::int(i32::from(connected)));
    props.seed(keys::CLIP_CONTINUOUS_SAMPLES, PropValue::int(0));
    props.seed(keys::PIXEL_ASPECT_RATIO, PropValue::double(1.0));
    props.seed(keys::FRAME_RATE, PropValue::double(25.0));
    props.seed(keys::FRAME_RANGE, PropValue::Double(vec![0.0, 0.0]));
    props.seed(
        keys::CLIP_UNMAPPED_FRAME_RANGE,
        PropValue::Double(vec![0.0, 0.0]),
    );
}

/// The project size an instance is born with, in pixels: 1080p, replaced by the
/// real frame at the first render ([`set_project_size`]).
const DEFAULT_PROJECT: (f64, f64) = (1920.0, 1080.0);

/// Tell an instance how big the picture it is about to be handed is.
///
/// Called before the first action of every render, because a plugin reads the
/// project size to place things and a stale one puts them in the wrong place.
pub(crate) fn set_project_size(handle: Handle, width: f64, height: f64) -> Result<(), Status> {
    let mut state = state();
    let props = state.effects.get(handle)?.props;
    let set = state.props.get_mut(props)?;
    set.set(keys::PROJECT_SIZE, PropValue::Double(vec![width, height]));
    set.set(keys::PROJECT_EXTENT, PropValue::Double(vec![width, height]));
    Ok(())
}

/// Tell an instance which of its clips have a picture on them, and how long
/// the clip is: the first and last frame the host has a picture for, which is
/// the frame in hand and the neighbours either side of it.
///
/// Called before the first action of every render, beside [`set_project_size`]
/// and for the same reason. A plugin that reads the frames either side clamps
/// what it asks for to this range. Told the clip is one frame long, it asks for
/// nothing and finds no motion. A clip with no picture is unconnected, so a
/// plugin with an optional second input does not ask for a picture that is
/// not there and fail its render on the answer.
pub(crate) fn set_clip_state(
    handle: Handle,
    connected: impl Fn(&str) -> bool,
    first: f64,
    last: f64,
) -> Result<(), Status> {
    let mut state = state();
    let (props, clips) = {
        let effect = state.effects.get(handle)?;
        let clips: Vec<(String, Handle)> = effect
            .clips
            .iter()
            .map(|clip| (clip.name.clone(), clip.props))
            .collect();
        (effect.props, clips)
    };
    state
        .props
        .get_mut(props)?
        .set(keys::EFFECT_DURATION, PropValue::double(last - first + 1.0));
    for (name, clip) in clips {
        let set = state.props.get_mut(clip)?;
        set.set(
            keys::CLIP_CONNECTED,
            PropValue::int(i32::from(connected(&name))),
        );
        set.set(keys::FRAME_RANGE, PropValue::Double(vec![first, last]));
        set.set(
            keys::CLIP_UNMAPPED_FRAME_RANGE,
            PropValue::Double(vec![first, last]),
        );
    }
    Ok(())
}

/// A time as a key that can be compared exactly.
///
/// Frame times are decimals, and two decimals that ought to be the same frame
/// are not always the same `f64`. A thousandth of a frame is finer than any
/// plugin asks for and coarser than the error, so it is the grain the prefetch
/// map is keyed at. The rounding is one line and it is deterministic, which is
/// the property that matters (docs/14 §10).
#[must_use]
pub fn time_key(time: f64) -> i64 {
    let scaled = (time * 1000.0).round();
    if scaled.is_nan() {
        return 0;
    }
    #[allow(clippy::cast_possible_truncation)]
    {
        scaled.clamp(i64::MIN as f64, i64::MAX as f64) as i64
    }
}

/// Put the pictures for one render in place, and answer with what was there
/// before — which is nothing, unless a render is already in flight.
pub(crate) fn set_images(
    handle: Handle,
    images: BTreeMap<String, Image>,
    images_at: BTreeMap<(String, i64), Image>,
) -> Result<BTreeMap<String, Image>, Status> {
    let mut state = state();
    let instance = state
        .effects
        .get_mut(handle)?
        .instance
        .as_mut()
        .ok_or(Status::ErrBadHandle)?;
    instance.images_at = images_at;
    Ok(std::mem::replace(&mut instance.images, images))
}

/// Add the frames at other times a prefetch just fetched, leaving the frame in
/// hand alone.
///
/// A second call rather than an argument to [`set_images`] because the two
/// happen at different moments: the clips are bound before the plugin is asked
/// anything, and the frames it wants beyond this one are not known until it has
/// answered `getFramesNeeded` (docs/impl/ofx-host.md §4).
pub(crate) fn add_images_at(
    handle: Handle,
    images_at: BTreeMap<(String, i64), Image>,
) -> Result<(), Status> {
    if images_at.is_empty() {
        return Ok(());
    }
    let mut state = state();
    let instance = state
        .effects
        .get_mut(handle)?
        .instance
        .as_mut()
        .ok_or(Status::ErrBadHandle)?;
    instance.images_at.extend(images_at);
    Ok(())
}

/// Take the pictures back off the instance at the end of a render. The caller
/// owns them from here, which is how the output frame gets read out.
pub(crate) fn take_images(handle: Handle) -> Result<BTreeMap<String, Image>, Status> {
    set_images(handle, BTreeMap::new(), BTreeMap::new())
}
