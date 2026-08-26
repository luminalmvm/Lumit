//! The render driver: the order the actions go in, and what comes back.
//!
//! # In plain terms
//!
//! Asking an OFX plugin for a frame is not one call. It is a short
//! conversation, and **the order is not negotiable** — plugins are written
//! against it, cache things between its steps, and crash when a host improvises
//! (docs/impl/ofx-host.md §3, docs/12 §2.1):
//!
//! 1. **How big are you?** `getRegionOfDefinition` — the region the effect can
//!    produce at all, which for a blur is bigger than its input.
//! 2. **What do you need to see?** `getRegionsOfInterest`. This host renders
//!    whole frames, never tiles, so the answer it *gives* is always the full
//!    region of definition — the plugin is asked, and then handed everything,
//!    because saying "tiles" and meaning "frames" is the classic host bug.
//! 3. **What shape are the pictures?** `getClipPreferences`. The honest answers
//!    are already in the host table: fp32 RGBA, premultiplied, square pixels.
//! 4. **Which frames do you need?** `getFramesNeeded` — the one a retimer lives
//!    on. Nothing is prefetched in this package, but the answer is carried out
//!    in [`Rendered::frames_needed`], because that is what the evaluation
//!    graph's temporal edges are made from.
//! 5. **Are you a no-op right now?** `isIdentity`. A plugin at its default
//!    settings usually is, and the honest short-circuit is to hand its input
//!    straight through without a render at all.
//! 6. **Render**, wrapped in `beginSequenceRender` and `endSequenceRender`.
//!    The wrapping is not decoration: a plugin allocates its scratch in the
//!    begin and frees it in the end, and a render without them leaks or
//!    crashes depending on the vendor.
//!
//! **Cancellation.** The driver checks its epoch token between every one of
//! those steps, and the plugin's own `abort` — which it polls inside its render
//! loop — is answered from the same token (docs/13 §6, docs/14). A scrub that
//! lands mid-frame stops the work rather than waiting it out.
//!
//! **Failure is a value.** A plugin that answers with a failure status yields a
//! typed [`RenderError`], never a panic; and the output buffer it was part way
//! through is dropped rather than handed on, because half a picture that looks
//! like a whole one is worse than no picture.
//!
//! **Concurrency is the plugin's declaration.** Two instances of a
//! `kOfxImageEffectRenderFullySafe` plugin render at once; an unsafe one
//! queues behind one lock (docs/12 §2.3, [`crate::instance::ThreadSafety`]).

use std::cell::Cell;
use std::collections::BTreeMap;

use lumit_eval::epoch::{Cancelled, EpochToken};
use thiserror::Error;

use crate::bundle::PluginRef;
use crate::ffi::{
    actions, frames_needed_key, prop_keys as keys, prop_values as values, region_of_interest_key,
};
use crate::handles::Handle;
use crate::host::state;
use crate::image::{Frame16, Image, RectI, RowOrder};
use crate::instance::{set_images, take_images, Instance};
use crate::props::{PropValue, PropertySet};
use crate::status::Status;

/// The actions the driver dispatches for one frame, in the order it dispatches
/// them. Written down so the order is a thing a test can compare against rather
/// than a thing spread across a function (docs/impl/ofx-host.md §3).
pub const RENDER_ACTIONS: [&str; 8] = [
    actions::GET_REGION_OF_DEFINITION,
    actions::GET_REGIONS_OF_INTEREST,
    actions::GET_CLIP_PREFERENCES,
    actions::GET_FRAMES_NEEDED,
    actions::IS_IDENTITY,
    actions::BEGIN_SEQUENCE_RENDER,
    actions::RENDER,
    actions::END_SEQUENCE_RENDER,
];

/// The name every filter's output clip goes by.
pub const OUTPUT_CLIP: &str = "Output";

/// What went wrong, as a value.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    /// The epoch turned over: this frame is nobody's frame any more.
    #[error("the render was cancelled")]
    Cancelled,
    /// The plugin answered an action with a failure.
    #[error("the plugin answered {status:?} to {action}")]
    Plugin {
        /// Which action.
        action: &'static str,
        /// What it said.
        status: Status,
    },
    /// The host could not set the render up. Not the plugin's fault, and
    /// reported as itself rather than blamed on it.
    #[error("the host could not render ({0:?})")]
    Host(Status),
}

impl From<Status> for RenderError {
    fn from(status: Status) -> Self {
        Self::Host(status)
    }
}

impl From<Cancelled> for RenderError {
    fn from(_: Cancelled) -> Self {
        Self::Cancelled
    }
}

/// One frame's worth of question: when, how big, and what to work from.
pub struct RenderRequest {
    /// The frame being asked for.
    pub time: f64,
    /// The rectangle to render, in pixels.
    pub bounds: RectI,
    /// Which way up the pictures are handed over. Both are legal and a plugin
    /// must cope with either; the tests render the same frame through both.
    pub order: RowOrder,
    /// One picture per input clip, by the name the plugin gave the clip.
    pub inputs: BTreeMap<String, Frame16>,
}

impl RenderRequest {
    /// The common case: one source, one output, top-down (so negative row
    /// bytes), at the source's own size.
    #[must_use]
    pub fn filter(time: f64, source: Frame16) -> Self {
        let bounds = RectI::sized(
            i32::try_from(source.width()).unwrap_or(0),
            i32::try_from(source.height()).unwrap_or(0),
        );
        let mut inputs = BTreeMap::new();
        inputs.insert("Source".to_owned(), source);
        Self {
            time,
            bounds,
            order: RowOrder::TopDown,
            inputs,
        }
    }
}

/// What one render produced, and what the plugin said along the way.
pub struct Rendered {
    /// The picture.
    pub frame: Frame16,
    /// `getRegionOfDefinition`'s answer, in pixels.
    pub region_of_definition: RectI,
    /// `getFramesNeeded`'s answer: per clip, the first and last frame the
    /// plugin wants to see. The evaluation graph's temporal edges are made from
    /// this (docs/05 §4.2); nothing prefetches it yet.
    pub frames_needed: BTreeMap<String, (f64, f64)>,
    /// The clip the plugin said it was a pass-through of, if it said so. When
    /// this is set, no render happened at all.
    pub identity_of: Option<String>,
}

thread_local! {
    /// The token the render on this thread is carrying, as an address.
    ///
    /// A raw address rather than a borrow because the plugin's `abort` is
    /// reached through a C function pointer with no context to hang anything
    /// on. It is set and cleared by [`CancelScope`] around the plugin call on
    /// the same thread that made it, so the token it names is alive at every
    /// moment it can be read.
    ///
    /// A thread `multiThread` spawned sees nought, and so is told to carry on.
    /// That is the ceiling: cancellation is polled by the plugin's main render
    /// thread, which is where every plugin in the test bench polls it, and
    /// carrying the token into the fan-out is work for the package that makes
    /// the fan-out worth having.
    static CANCEL_TOKEN: Cell<usize> = const { Cell::new(0) };
}

/// Sets the thread's cancellation token for as long as it is alive, and puts
/// back whatever was there before.
struct CancelScope {
    previous: usize,
}

impl CancelScope {
    fn new(token: &EpochToken) -> Self {
        let previous = CANCEL_TOKEN.with(|slot| {
            let previous = slot.get();
            slot.set(std::ptr::from_ref(token) as usize);
            previous
        });
        Self { previous }
    }
}

impl Drop for CancelScope {
    fn drop(&mut self) {
        CANCEL_TOKEN.with(|slot| slot.set(self.previous));
    }
}

/// Whether the render on this thread has been cancelled — what the image effect
/// suite's `abort` answers.
#[must_use]
pub fn render_is_cancelled() -> bool {
    let address = CANCEL_TOKEN.with(Cell::get);
    if address == 0 {
        return false;
    }
    // SAFETY: the address was written by a live `CancelScope` on this thread
    // and is cleared by that scope's `Drop`, so the token it names is on this
    // thread's stack, alive, and not moved. Nothing else writes the slot.
    let token = unsafe { &*(address as *const EpochToken) };
    token.cancelled()
}

/// Render one frame through one instance of one plugin.
///
/// # Errors
///
/// [`RenderError`] — cancelled, the plugin's own failure, or the host's.
pub fn render(
    plugin: &PluginRef,
    instance: &Instance,
    request: &RenderRequest,
    token: &EpochToken,
) -> Result<Rendered, RenderError> {
    let handle = instance.handle();
    token.check()?;

    let region_of_definition = get_region_of_definition(plugin, handle, request)?;
    token.check()?;

    get_regions_of_interest(plugin, handle, request, region_of_definition)?;
    token.check()?;

    // The clip preferences are asked for and the answers are already true:
    // fp32 RGBA premultiplied with square pixels is the only thing the host
    // table offers, so there is nothing for a plugin to change and nothing to
    // read back. It is dispatched because a plugin that is never asked behaves
    // differently from one that is.
    let (status, _) = dispatch_for_answer(
        plugin,
        handle,
        actions::GET_CLIP_PREFERENCES,
        PropertySet::new(),
        PropertySet::new(),
    )?;
    if !matches!(status, Status::Ok | Status::ReplyDefault) {
        return Err(RenderError::Plugin {
            action: actions::GET_CLIP_PREFERENCES,
            status,
        });
    }
    token.check()?;

    let frames_needed = get_frames_needed(plugin, handle, request)?;
    token.check()?;

    if let Some(clip) = is_identity(plugin, handle, request)? {
        // The honest short-circuit: the plugin says this frame is its input, so
        // the input is the frame. No begin, no render, no end, no buffer.
        let frame = request
            .inputs
            .get(&clip)
            .cloned()
            .ok_or(RenderError::Host(Status::ErrUnknown))?;
        return Ok(Rendered {
            frame,
            region_of_definition,
            frames_needed,
            identity_of: Some(clip),
        });
    }
    token.check()?;

    let frame = render_sequence(plugin, instance, request, token)?;
    Ok(Rendered {
        frame,
        region_of_definition,
        frames_needed,
        identity_of: None,
    })
}

/// Begin, render, end — the part that must not be interrupted halfway and must
/// not be entered by two threads at once unless the plugin said it may be.
fn render_sequence(
    plugin: &PluginRef,
    instance: &Instance,
    request: &RenderRequest,
    token: &EpochToken,
) -> Result<Frame16, RenderError> {
    let handle = instance.handle();

    // The pictures are built before the lock: allocating is the host's own
    // business and nothing else is waiting on it.
    let mut images = BTreeMap::new();
    for (name, frame) in &request.inputs {
        images.insert(name.clone(), Image::from_frame(frame, request.order)?);
    }
    images.insert(
        OUTPUT_CLIP.to_owned(),
        Image::black(request.bounds, request.order)?,
    );

    // Chosen before the lock is taken, and taken with no host lock held: the
    // render behind it calls into a plugin, which re-enters the suites
    // (docs/14 §7).
    let guard = instance.render_lock();
    let _held = guard.as_ref().map(crate::instance::RenderGuard::hold);

    let previous = set_images(handle, images)?;
    drop(previous);

    let outcome = (|| -> Result<(), RenderError> {
        dispatch(
            plugin,
            handle,
            actions::BEGIN_SEQUENCE_RENDER,
            Some(sequence_args(request)),
        )?;
        token.check()?;
        let scope = CancelScope::new(token);
        let render = dispatch(plugin, handle, actions::RENDER, Some(render_args(request)));
        drop(scope);
        // The end action runs whatever the render answered: a plugin that
        // allocated in the begin must be given its chance to free, and a host
        // that skips the end on the failure path is the reason plugins leak.
        let end = dispatch(
            plugin,
            handle,
            actions::END_SEQUENCE_RENDER,
            Some(sequence_args(request)),
        );
        render?;
        end?;
        token.check()?;
        Ok(())
    })();

    // The pictures come back off the instance either way, so a failed render
    // leaves nothing behind for the next one to find.
    let mut images = take_images(handle)?;
    let output = images.remove(OUTPUT_CLIP);
    drop(images);
    // Only now is the failure allowed to escape, and the half-written output
    // goes with it rather than to the caller.
    outcome?;

    let frame = output
        .ok_or(RenderError::Host(Status::ErrFatal))?
        .to_frame()?;
    Ok(frame)
}

/// Dispatch one action with an optional `inArgs`, and turn a failure into a
/// [`RenderError`] naming which action it was.
fn dispatch(
    plugin: &PluginRef,
    handle: Handle,
    action: &'static str,
    in_args: Option<PropertySet>,
) -> Result<(), RenderError> {
    let in_args = match in_args {
        Some(set) => Some(state().props.insert(set)?),
        None => None,
    };
    let status = plugin.action(action, Some(handle), in_args, None);
    if let Some(in_args) = in_args {
        let _ = state().props.remove(in_args);
    }
    if matches!(status, Status::Ok | Status::ReplyDefault) {
        Ok(())
    } else {
        Err(RenderError::Plugin { action, status })
    }
}

/// Dispatch one action that answers into an `outArgs`, and hand the answer
/// back. `seed` is what the host puts in the bag before asking, which is the
/// answer a plugin that does not implement the action leaves there.
fn dispatch_for_answer(
    plugin: &PluginRef,
    handle: Handle,
    action: &'static str,
    in_args: PropertySet,
    seed: PropertySet,
) -> Result<(Status, PropertySet), RenderError> {
    let (in_args, out_args) = {
        let mut state = state();
        let in_args = state.props.insert(in_args)?;
        let out_args = state.props.insert(seed)?;
        (in_args, out_args)
    };
    let status = plugin.action(action, Some(handle), Some(in_args), Some(out_args));
    let answer = {
        let mut state = state();
        let _ = state.props.remove(in_args);
        state.props.remove(out_args)?
    };
    Ok((status, answer))
}

/// `kOfxImageEffectActionGetRegionOfDefinition`.
fn get_region_of_definition(
    plugin: &PluginRef,
    handle: Handle,
    request: &RenderRequest,
) -> Result<RectI, RenderError> {
    let mut in_args = PropertySet::new();
    in_args.seed(keys::TIME, PropValue::double(request.time));
    in_args.seed(keys::RENDER_SCALE, PropValue::Double(vec![1.0, 1.0]));

    let mut seed = PropertySet::new();
    seed.seed(keys::REGION_OF_DEFINITION, rect_as_doubles(request.bounds));

    let (status, answer) = dispatch_for_answer(
        plugin,
        handle,
        actions::GET_REGION_OF_DEFINITION,
        in_args,
        seed,
    )?;
    // `kOfxStatReplyDefault` means "use the default", which is the region we
    // seeded — so the two success codes take the same path.
    if !matches!(status, Status::Ok | Status::ReplyDefault) {
        return Err(RenderError::Plugin {
            action: actions::GET_REGION_OF_DEFINITION,
            status,
        });
    }
    Ok(rect_from_doubles(&answer, keys::REGION_OF_DEFINITION).unwrap_or(request.bounds))
}

/// `kOfxImageEffectActionGetRegionsOfInterest`.
///
/// The plugin is asked and the answer is **not used**: this host does not tile
/// (`kOfxImageEffectPropSupportsTiles` is nought in the host table), so every
/// clip is handed its full region of definition whatever it asked for. Asking
/// anyway is deliberate — plugins do work in this handler, and one that is
/// never called behaves differently from one that is.
fn get_regions_of_interest(
    plugin: &PluginRef,
    handle: Handle,
    request: &RenderRequest,
    region_of_definition: RectI,
) -> Result<(), RenderError> {
    let mut in_args = PropertySet::new();
    in_args.seed(keys::TIME, PropValue::double(request.time));
    in_args.seed(keys::RENDER_SCALE, PropValue::Double(vec![1.0, 1.0]));
    in_args.seed(
        keys::REGION_OF_INTEREST,
        rect_as_doubles(region_of_definition),
    );

    let mut seed = PropertySet::new();
    for name in request.inputs.keys() {
        seed.seed(
            &region_of_interest_key(name),
            rect_as_doubles(region_of_definition),
        );
    }

    let (status, _) = dispatch_for_answer(
        plugin,
        handle,
        actions::GET_REGIONS_OF_INTEREST,
        in_args,
        seed,
    )?;
    if !matches!(status, Status::Ok | Status::ReplyDefault) {
        return Err(RenderError::Plugin {
            action: actions::GET_REGIONS_OF_INTEREST,
            status,
        });
    }
    Ok(())
}

/// `kOfxImageEffectActionGetFramesNeeded`.
fn get_frames_needed(
    plugin: &PluginRef,
    handle: Handle,
    request: &RenderRequest,
) -> Result<BTreeMap<String, (f64, f64)>, RenderError> {
    let mut in_args = PropertySet::new();
    in_args.seed(keys::TIME, PropValue::double(request.time));

    let mut seed = PropertySet::new();
    for name in request.inputs.keys() {
        seed.seed(
            &frames_needed_key(name),
            PropValue::Double(vec![request.time, request.time]),
        );
    }

    let (status, answer) =
        dispatch_for_answer(plugin, handle, actions::GET_FRAMES_NEEDED, in_args, seed)?;
    if !matches!(status, Status::Ok | Status::ReplyDefault) {
        return Err(RenderError::Plugin {
            action: actions::GET_FRAMES_NEEDED,
            status,
        });
    }

    let mut needed = BTreeMap::new();
    for name in request.inputs.keys() {
        let key = frames_needed_key(name);
        let first = answer.get_double(&key, 0).unwrap_or(request.time);
        let last = answer.get_double(&key, 1).unwrap_or(first);
        needed.insert(name.clone(), (first, last));
    }
    Ok(needed)
}

/// `kOfxImageEffectActionIsIdentity`, and which clip the plugin named.
fn is_identity(
    plugin: &PluginRef,
    handle: Handle,
    request: &RenderRequest,
) -> Result<Option<String>, RenderError> {
    let mut in_args = PropertySet::new();
    in_args.seed(keys::TIME, PropValue::double(request.time));
    in_args.seed(keys::RENDER_SCALE, PropValue::Double(vec![1.0, 1.0]));
    in_args.seed(
        keys::RENDER_WINDOW,
        PropValue::Int(request.bounds.as_array().to_vec()),
    );
    if let Ok(field) = PropValue::string(values::IMAGE_FIELD_NONE) {
        in_args.seed(keys::FIELD_TO_RENDER, field);
    }

    let mut seed = PropertySet::new();
    if let Ok(empty) = PropValue::string("") {
        seed.seed(keys::NAME, empty);
    }
    seed.seed(keys::TIME, PropValue::double(request.time));

    let (status, answer) =
        dispatch_for_answer(plugin, handle, actions::IS_IDENTITY, in_args, seed)?;
    match status {
        // `kOfxStatOK` from this action means "yes, and here is the clip".
        Status::Ok => {
            let name = answer
                .get_string(keys::NAME, 0)
                .map(|text| text.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() {
                Ok(None)
            } else {
                Ok(Some(name))
            }
        }
        // `kOfxStatReplyDefault` is "no, render me properly".
        Status::ReplyDefault => Ok(None),
        status => Err(RenderError::Plugin {
            action: actions::IS_IDENTITY,
            status,
        }),
    }
}

/// The `inArgs` of `beginSequenceRender` and `endSequenceRender`. One frame is
/// a sequence of one; the host is not a render farm node, so nothing here is
/// sequential or interactive.
fn sequence_args(request: &RenderRequest) -> PropertySet {
    let mut args = PropertySet::new();
    args.seed(
        keys::FRAME_RANGE,
        PropValue::Double(vec![request.time, request.time]),
    );
    args.seed(keys::FRAME_STEP, PropValue::double(1.0));
    args.seed(keys::IS_INTERACTIVE, PropValue::int(0));
    args.seed(keys::RENDER_SCALE, PropValue::Double(vec![1.0, 1.0]));
    args.seed(keys::SEQUENTIAL_RENDER_STATUS, PropValue::int(0));
    args.seed(keys::INTERACTIVE_RENDER_STATUS, PropValue::int(0));
    args
}

/// The `inArgs` of the render itself.
fn render_args(request: &RenderRequest) -> PropertySet {
    let mut args = PropertySet::new();
    args.seed(keys::TIME, PropValue::double(request.time));
    if let Ok(field) = PropValue::string(values::IMAGE_FIELD_NONE) {
        args.seed(keys::FIELD_TO_RENDER, field);
    }
    args.seed(
        keys::RENDER_WINDOW,
        PropValue::Int(request.bounds.as_array().to_vec()),
    );
    args.seed(keys::RENDER_SCALE, PropValue::Double(vec![1.0, 1.0]));
    args.seed(keys::SEQUENTIAL_RENDER_STATUS, PropValue::int(0));
    args.seed(keys::INTERACTIVE_RENDER_STATUS, PropValue::int(0));
    args.seed(keys::RENDER_QUALITY_DRAFT, PropValue::int(0));
    args
}

/// A pixel rectangle as the doubles the region actions speak in.
fn rect_as_doubles(rect: RectI) -> PropValue {
    PropValue::Double(vec![
        f64::from(rect.x1),
        f64::from(rect.y1),
        f64::from(rect.x2),
        f64::from(rect.y2),
    ])
}

/// A pixel rectangle out of a double property, rounded **outwards** — a region
/// rounded inwards loses a row of the picture at its edge.
fn rect_from_doubles(props: &PropertySet, key: &str) -> Option<RectI> {
    let read = |index: usize| props.get_double(key, index).ok();
    let (x1, y1, x2, y2) = (read(0)?, read(1)?, read(2)?, read(3)?);
    let floor = |value: f64| {
        value
            .floor()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    };
    let ceil = |value: f64| value.ceil().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    Some(RectI {
        x1: floor(x1),
        y1: floor(y1),
        x2: ceil(x2),
        y2: ceil(y2),
    })
}
