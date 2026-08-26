//! Describe: asking a plugin what it is, and writing the answer down.
//!
//! # In plain terms
//!
//! A freshly loaded plugin has told us nothing but its name. **Describe** is the
//! conversation where it says what it is called, what family it belongs to,
//! which shapes of effect it can be, what pictures it wants and what controls
//! it has. It happens once per plugin, before any copy of the effect exists on
//! any layer — which is exactly why it can be done at start-up and the answer
//! kept.
//!
//! The order is fixed and getting it wrong crashes plugins that are otherwise
//! blameless (docs/impl/ofx-host.md §3):
//!
//! 1. `kOfxActionDescribe`, with a fresh, empty descriptor. The plugin fills in
//!    its label, its grouping, and the **contexts** it can work in.
//! 2. `kOfxImageEffectActionDescribeInContext`, once, with a second descriptor
//!    that carries the context we picked. *Now* the plugin defines its clips
//!    and its parameters — they belong to the context, because the same plugin
//!    is a different effect as a filter and as a general effect.
//!
//! **Contexts.** This package drives two, filter and general, which is what
//! ofx-host.md §3 names and what covers every plugin the test bench cares
//! about. docs/12 §2.1 lists four for the finished host — generator and
//! transition arrive with the packages that can render them — and its rule for
//! the gap is written down: *plugins adapt or are rejected at describe time
//! with a report entry*. So a plugin whose only context is one we cannot drive
//! is rejected **with a reason**, the scan carries on, and the reason is a line
//! in the report rather than a silence.
//!
//! Nothing here holds the host lock across a call into a plugin: the plugin
//! re-enters the suites from inside every action, and a held lock would
//! deadlock on the first property it read (docs/14 §7).

use thiserror::Error;

use crate::bundle::{Bundle, PluginRef};
use crate::ffi::{actions, param_types, prop_keys as keys, prop_values as values};
use crate::handles::Handle;
use crate::host::state;
use crate::props::{PropValue, PropertySet};
use crate::schema::schema_of;
use crate::status::Status;
use lumit_core::fx::EffectSchema;

/// One clip a plugin defined, on a descriptor or on an instance.
pub struct ClipRef {
    /// The name the plugin gave it: `Source`, `Output`, `Mask`.
    pub name: String,
    /// Its property set.
    pub props: Handle,
    /// The `OfxImageClipHandle` the plugin holds, on an **instance**. A
    /// descriptor's clip has none: a clip handle names a live image input, and
    /// a description has no images (`clipGetHandle` answers null for it, which
    /// is what the spec says a descriptor's clip is worth).
    pub handle: Option<Handle>,
}

/// One parameter a plugin defined, on a descriptor or on an instance.
pub struct ParamRef {
    /// The name the plugin gave it, which is also its stable id.
    pub name: String,
    /// One of [`crate::ffi::param_types`].
    pub param_type: String,
    /// The handle the plugin holds.
    pub handle: Handle,
    /// Its property set.
    pub props: Handle,
}

/// What an `OfxParamHandle` names in the host's registry.
///
/// The name and the owning effect are kept beside the property set because
/// that is exactly what `paramGetValue` needs and has nothing else to work
/// from: it is handed one parameter handle and must find the **instance's**
/// snapshot value for it (docs/12 §2.2).
pub struct ParamRecord {
    /// The parameter's property set.
    pub props: Handle,
    /// The effect or instance it belongs to.
    pub effect: Handle,
    /// Its name, which is its key in the snapshot.
    pub name: String,
}

/// What an `OfxImageClipHandle` names in the host's registry.
pub struct ClipBinding {
    /// The instance it belongs to.
    pub effect: Handle,
    /// Which clip of that instance it is.
    pub name: String,
    /// The clip's property set.
    pub props: Handle,
}

/// An effect descriptor: what the two describe actions fill in — and, once
/// `instance` is filled in, an **instance** of that effect. They are one kind
/// of object in OFX (both are an `OfxImageEffectHandle` with a property set,
/// clips and parameters), so they are one kind of object here.
pub struct EffectDescriptor {
    /// The effect's own property set.
    pub props: Handle,
    /// The clips it defined, in definition order.
    pub clips: Vec<ClipRef>,
    /// The parameters it defined, in definition order. **Order is a promise**
    /// (docs/impl/effect-registry.md §5): it is the order the panel draws.
    pub params: Vec<ParamRef>,
    /// The live half, present only on an instance ([`crate::instance`]).
    pub instance: Option<crate::instance::InstanceState>,
}

/// The context this host drives a plugin in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// One input, one output: the shape of nearly every effect.
    Filter,
    /// As many inputs as the plugin asked for.
    General,
}

impl Context {
    /// The OFX name.
    #[must_use]
    pub const fn ofx_name(self) -> &'static str {
        match self {
            Context::Filter => values::CONTEXT_FILTER,
            Context::General => values::CONTEXT_GENERAL,
        }
    }

    /// The context that OFX name means, or `None` for one this package does
    /// not drive.
    #[must_use]
    pub fn from_ofx_name(name: &str) -> Option<Self> {
        match name {
            values::CONTEXT_FILTER => Some(Context::Filter),
            values::CONTEXT_GENERAL => Some(Context::General),
            _ => None,
        }
    }
}

/// **Preference order.** A filter is the simpler contract — one source, one
/// output — so a plugin that offers both is driven as a filter.
const PREFERRED_CONTEXTS: [Context; 2] = [Context::Filter, Context::General];

/// One parameter, as an owned copy of everything the plugin said about it.
///
/// The properties are kept whole rather than picked apart into typed fields:
/// OFX puts everything in the bag, the bag is already the right shape, and a
/// struct of thirty optional fields would only be the bag with the names
/// spelled twice.
#[derive(Clone, Debug)]
pub struct ParamDescription {
    /// The plugin's name for it, which becomes the schema id.
    pub name: String,
    /// One of [`crate::ffi::param_types`].
    pub param_type: String,
    /// Everything else it said.
    pub props: PropertySet,
}

/// One clip, as an owned copy.
#[derive(Clone, Debug)]
pub struct ClipDescription {
    /// `Source`, `Output`, `Mask`, or whatever the plugin called it.
    pub name: String,
    /// Everything the plugin said about it.
    pub props: PropertySet,
}

/// Everything one plugin said about itself, owned, with no handle in it: the
/// describe conversation is over and the plugin may be unloaded.
#[derive(Clone, Debug)]
pub struct PluginDescriptor {
    /// The reverse-domain identifier, e.g. `net.sf.openfx.invertPlugin`.
    pub identifier: String,
    /// Major and minor, as the plugin declares them.
    pub version: (u32, u32),
    /// The plugin's own menu path, e.g. `Filter/Blur`. Effects & Presets shows
    /// discovered effects under it (docs/12 §2.6).
    pub grouping: String,
    /// The name a person sees.
    pub label: String,
    /// The contexts this host can drive it in, best first. `params` and `clips`
    /// come from the first.
    pub contexts: Vec<Context>,
    /// Its parameters, in definition order.
    pub params: Vec<ParamDescription>,
    /// Its clips, in definition order.
    pub clips: Vec<ClipDescription>,
    /// Whether it declared that it reads frames other than the current one —
    /// `kOfxImageEffectPropTemporalClipAccess`, which is what a retimer must
    /// set to be one.
    pub temporal: bool,
    /// `kOfxImageEffectPluginRenderThreadSafety`, verbatim, or `None` if the
    /// plugin never said. The host reads it into
    /// [`ThreadSafety`](crate::instance::ThreadSafety), which treats anything
    /// it does not recognise — silence included — as the most pessimistic
    /// answer (docs/13 §6).
    pub render_thread_safety: Option<String>,
}

impl PluginDescriptor {
    /// The parameters that have **no row in the schema**, and their OFX types.
    ///
    /// Three kinds land here, and none of them is a mistake: a custom
    /// parameter's vendor blob, which docs/12 §2.2 says is stored and
    /// round-tripped without interpretation and which no control could draw; a
    /// text parameter that is not a path, because Lumit has no text row; and a
    /// parametric curve, which is a function rather than the control points
    /// Lumit's curve is made of (K-412). Groups and pages are excluded — they
    /// have no row because they *are* the layout, and reporting them as missing
    /// controls would be noise.
    ///
    /// It exists so that "this plugin has a control Lumit cannot show" is a
    /// line in the report rather than a silence.
    #[must_use]
    pub fn unrepresented(&self) -> Vec<(&str, &str)> {
        self.params
            .iter()
            .filter(|param| match param.param_type.as_str() {
                param_types::CUSTOM | param_types::PARAMETRIC => true,
                param_types::STRING => !crate::schema::string_is_path(&param.props),
                _ => false,
            })
            .map(|param| (param.name.as_str(), param.param_type.as_str()))
            .collect()
    }
}

/// Why a plugin has no schema. Every variant is a line the report prints, so
/// a rejection is never a silence (docs/12 §2.1).
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum Rejection {
    /// The plugin refused to load, so there is nothing to describe.
    #[error("the plugin did not load ({0:?})")]
    NotLoaded(Status),
    /// `kOfxActionDescribe` came back with a failure.
    #[error("describe failed ({0:?})")]
    DescribeFailed(Status),
    /// The plugin works only in contexts this host does not drive yet.
    #[error("it works only in {}, and this host drives filter and general", .declared.join(", "))]
    NoDrivenContext {
        /// What it did declare, verbatim, so the report can name it.
        declared: Vec<String>,
    },
    /// `kOfxImageEffectActionDescribeInContext` came back with a failure.
    #[error("describe in the {context} context failed ({status:?})")]
    DescribeInContextFailed {
        /// The context we asked for.
        context: &'static str,
        /// What the plugin answered.
        status: Status,
    },
    /// Two parameters would land on the same [`ParamId`](lumit_core::fx::ParamId).
    #[error("two parameters share the id {first:?} (the second is {second:?})")]
    DuplicateParamId {
        /// The parameter that got there first.
        first: String,
        /// The one that collided with it.
        second: String,
    },
    /// The host itself ran out of something. Not the plugin's fault, and
    /// reported as itself rather than blamed on it.
    #[error("the host could not describe it ({0:?})")]
    HostFault(Status),
}

impl From<Status> for Rejection {
    fn from(status: Status) -> Self {
        Rejection::HostFault(status)
    }
}

/// One plugin that described itself, and what Lumit made of it.
pub struct DescribedPlugin {
    /// What the plugin said.
    pub descriptor: PluginDescriptor,
    /// The same declaration a built-in effect carries.
    pub schema: EffectSchema,
}

/// One plugin that did not.
pub struct Rejected {
    /// Its identifier, so the report can name it.
    pub identifier: String,
    /// Its version.
    pub version: (u32, u32),
    /// Why.
    pub reason: Rejection,
}

/// What one scan found: the effects, and the reasons for the ones that are not
/// effects here. **A rejection never ends a scan** — one plugin that cannot be
/// driven must not cost the user the other seventy-nine in the bundle.
#[derive(Default)]
pub struct ScanReport {
    /// The effects, in the order the bundle declares them.
    pub effects: Vec<DescribedPlugin>,
    /// The ones that were turned away, and why.
    pub rejected: Vec<Rejected>,
}

/// Describe every plugin in a loaded bundle.
///
/// The bundle must already have been through [`Bundle::load`]; a plugin that
/// did not load is rejected rather than called.
#[must_use]
pub fn describe_bundle(bundle: &Bundle) -> ScanReport {
    let mut report = ScanReport::default();
    for plugin in bundle.plugins() {
        if !plugin.is_supported_image_effect() {
            continue;
        }
        match describe(plugin).and_then(|descriptor| {
            let schema = schema_of(&descriptor)?;
            Ok(DescribedPlugin { descriptor, schema })
        }) {
            Ok(described) => report.effects.push(described),
            Err(reason) => report.rejected.push(Rejected {
                identifier: plugin.identifier.clone(),
                version: plugin.version,
                reason,
            }),
        }
    }
    report
}

/// Run the describe sequence at one plugin.
///
/// # Errors
///
/// A [`Rejection`] naming why this plugin is not an effect Lumit can offer.
pub fn describe(plugin: &PluginRef) -> Result<PluginDescriptor, Rejection> {
    match plugin.load_status {
        Some(Status::Ok) => {}
        other => return Err(Rejection::NotLoaded(other.unwrap_or(Status::ErrFatal))),
    }

    // The descriptor is released whatever happens, including on the paths that
    // reject: a rejected plugin must not leave property sets behind for the
    // rest of the session.
    let base = new_descriptor(base_property_set(&plugin.identifier))?;
    let described = describe_base(plugin, base);
    release_descriptor(base);
    let base_props = described?;

    let declared = strings(&base_props, keys::SUPPORTED_CONTEXTS);
    let contexts: Vec<Context> = PREFERRED_CONTEXTS
        .into_iter()
        .filter(|context| declared.iter().any(|name| name == context.ofx_name()))
        .collect();
    let Some(&context) = contexts.first() else {
        return Err(Rejection::NoDrivenContext { declared });
    };

    // The context descriptor starts as a copy of what the plugin said about
    // itself, plus the context: a plugin that reads its own grouping back
    // inside `describeInContext` must find it there.
    let mut context_props = base_props.clone();
    if let Ok(value) = PropValue::string(context.ofx_name()) {
        context_props.seed(keys::CONTEXT, value);
    }
    let handle = new_descriptor(context_props)?;
    let described = describe_in_context(plugin, handle, context);
    let gathered = gather(handle);
    release_descriptor(handle);
    described?;
    let (params, clips) = gathered?;

    Ok(PluginDescriptor {
        identifier: plugin.identifier.clone(),
        version: plugin.version,
        grouping: base_props
            .get_string(keys::GROUPING, 0)
            .map(|text| text.to_string_lossy().into_owned())
            .unwrap_or_default(),
        label: label_of(&base_props, &plugin.identifier),
        contexts,
        params,
        clips,
        temporal: base_props.get_int(keys::TEMPORAL_CLIP_ACCESS, 0) == Ok(1),
        render_thread_safety: base_props
            .get_string(keys::PLUGIN_RENDER_THREAD_SAFETY, 0)
            .ok()
            .map(|text| text.to_string_lossy().into_owned()),
    })
}

/// `kOfxActionDescribe`, and what the plugin left in the descriptor.
fn describe_base(plugin: &PluginRef, handle: Handle) -> Result<PropertySet, Rejection> {
    // No lock is held here; the plugin is about to re-enter the suites.
    let status = plugin.action(actions::DESCRIBE, Some(handle), None, None);
    // `kOfxStatReplyDefault` means "I did not handle that", which for describe
    // leaves no contexts and is caught by the emptiness check rather than by a
    // second rule.
    if !matches!(status, Status::Ok | Status::ReplyDefault) {
        return Err(Rejection::DescribeFailed(status));
    }
    Ok(snapshot_of(handle)?)
}

/// `kOfxImageEffectActionDescribeInContext`.
fn describe_in_context(
    plugin: &PluginRef,
    handle: Handle,
    context: Context,
) -> Result<(), Rejection> {
    // `inArgs` carries the context, and is valid **only for the duration of the
    // action**: it is destroyed the moment the plugin returns, so a plugin that
    // kept the handle finds a dead one rather than a live property set.
    let mut in_args = PropertySet::new();
    if let Ok(value) = PropValue::string(context.ofx_name()) {
        in_args.seed(keys::CONTEXT, value);
    }
    let in_args = state().props.insert(in_args)?;

    let status = plugin.action(
        actions::DESCRIBE_IN_CONTEXT,
        Some(handle),
        Some(in_args),
        None,
    );
    let _ = state().props.remove(in_args);

    if !matches!(status, Status::Ok | Status::ReplyDefault) {
        return Err(Rejection::DescribeInContextFailed {
            context: context.ofx_name(),
            status,
        });
    }
    Ok(())
}

/// Copy a finished descriptor's parameters and clips out of the host state.
fn gather(handle: Handle) -> Result<(Vec<ParamDescription>, Vec<ClipDescription>), Rejection> {
    let state = state();
    let descriptor = state.effects.get(handle)?;
    let mut params = Vec::with_capacity(descriptor.params.len());
    for param in &descriptor.params {
        params.push(ParamDescription {
            name: param.name.clone(),
            param_type: param.param_type.clone(),
            props: state.props.get(param.props)?.clone(),
        });
    }
    let mut clips = Vec::with_capacity(descriptor.clips.len());
    for clip in &descriptor.clips {
        clips.push(ClipDescription {
            name: clip.name.clone(),
            props: state.props.get(clip.props)?.clone(),
        });
    }
    Ok((params, clips))
}

/// The label a person sees: the plugin's own, or its identifier if it never
/// said. An effect with no name in the menu is worse than an ugly one.
fn label_of(props: &PropertySet, identifier: &str) -> String {
    for key in [keys::LABEL, keys::LONG_LABEL, keys::SHORT_LABEL] {
        if let Ok(text) = props.get_string(key, 0) {
            let text = text.to_string_lossy();
            if !text.is_empty() {
                return text.into_owned();
            }
        }
    }
    identifier.to_owned()
}

/// Every element of a string property, or nothing.
fn strings(props: &PropertySet, key: &str) -> Vec<String> {
    match props.get(key) {
        Ok(PropValue::String(values)) => values
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect(),
        _ => Vec::new(),
    }
}

/// The properties a fresh effect descriptor starts with.
///
/// Seeded rather than left empty because a plugin reads before it writes: the
/// Support library asks a descriptor for its own name and context, and a host
/// that answers "no such property" there has plugins that describe nothing.
pub(crate) fn base_property_set(identifier: &str) -> PropertySet {
    /// A NUL in one of these would be a NUL in the plugin's own identifier;
    /// the property is then simply absent, which reads as "it never said".
    fn seed_string(set: &mut PropertySet, key: &str, value: &str) {
        if let Ok(value) = PropValue::string(value) {
            set.seed(key, value);
        }
    }

    let mut set = PropertySet::new();
    seed_string(&mut set, keys::TYPE, values::TYPE_IMAGE_EFFECT);
    seed_string(&mut set, keys::NAME, identifier);
    seed_string(&mut set, keys::LABEL, "");
    seed_string(&mut set, keys::SHORT_LABEL, "");
    seed_string(&mut set, keys::LONG_LABEL, "");
    seed_string(&mut set, keys::GROUPING, "");
    // The spec's own default, and not the generous one: a plugin that never
    // says gets instance-safe, which is what the OFX header says it means, and
    // a string this host does not recognise is read as fully unsafe
    // (docs/13 §6: an undeclared effect is the pessimistic case).
    seed_string(
        &mut set,
        keys::PLUGIN_RENDER_THREAD_SAFETY,
        values::RENDER_THREAD_SAFETY_INSTANCE_SAFE,
    );
    set.seed(keys::SUPPORTED_CONTEXTS, PropValue::String(Vec::new()));
    set.seed(keys::SUPPORTED_PIXEL_DEPTHS, PropValue::String(Vec::new()));
    set.seed(keys::TEMPORAL_CLIP_ACCESS, PropValue::int(0));
    set.seed(keys::SUPPORTS_TILES, PropValue::int(0));
    set.seed(keys::SUPPORTS_MULTI_RESOLUTION, PropValue::int(0));
    set.seed(keys::SUPPORTS_OVERLAYS, PropValue::int(0));
    set
}

/// Mint a descriptor and the property set that goes with it.
pub(crate) fn new_descriptor(props: PropertySet) -> Result<Handle, Status> {
    let mut state = state();
    let props = state.props.insert(props)?;
    state.effects.insert(EffectDescriptor {
        props,
        clips: Vec::new(),
        params: Vec::new(),
        instance: None,
    })
}

/// A copy of a descriptor's own property set.
fn snapshot_of(handle: Handle) -> Result<PropertySet, Status> {
    let state = state();
    let props = state.effects.get(handle)?.props;
    Ok(state.props.get(props)?.clone())
}

/// Destroy a descriptor — or an instance — and everything hanging off it.
/// Every handle the plugin was given is dead from here on, which is exactly
/// what the OFX lifetime rules say: a descriptor lives only as long as the
/// action, and an instance only until it is destroyed.
///
/// Any pictures the instance still held are moved out and dropped **after** the
/// host lock is released. An image's block belongs to the arena, which has a
/// lock of its own, and taking two locks in an order nobody wrote down is how a
/// deadlock gets built (docs/14 §7); here the images simply outlive the guard
/// by one line.
pub(crate) fn release_descriptor(handle: Handle) {
    let images = {
        let mut state = state();
        let Ok(descriptor) = state.effects.remove(handle) else {
            return;
        };
        let _ = state.props.remove(descriptor.props);
        for clip in &descriptor.clips {
            let _ = state.props.remove(clip.props);
            if let Some(clip_handle) = clip.handle {
                let _ = state.clips.remove(clip_handle);
            }
        }
        for param in &descriptor.params {
            let _ = state.props.remove(param.props);
            let _ = state.params.remove(param.handle);
        }
        descriptor.instance.map(|instance| instance.images)
    };
    drop(images);
}
