//! What an effect *is*, as a value rather than as a variant
//! (docs/impl/effect-registry.md §2.4, §2.6).
//!
//! **In plain terms.** Lumit used to know its effects the way a switchboard knows
//! its lines: one giant list written into the program, with a matching entry in
//! four other places, and no way for a line to exist that was not soldered in.
//! This module is the replacement — an effect is a **value** implementing
//! [`EffectDef`], and the catalogue is a list of those values. Looking an effect
//! up by name gives you something you can call, so the places that used to ask
//! "which of the thirty-four effects is this?" now simply ask the effect.
//!
//! The catalogue itself is deliberately an explicit list ([`catalogue!`] in
//! `catalogue.rs`) rather than something assembled before `main` runs: the
//! Add-effect menu, the command palette and the preset browser are all driven
//! from it in order (K-137), and an order that depended on link order would be a
//! visible defect. Effects that are *not* known at compile time — OFX plugins
//! (docs/12), and in time the user's own — arrive through the same trait object
//! at run time, which is the seam this arrangement exists for.

use std::sync::Arc;

use super::markers::MarkerContext;
use super::params::{ParamId, Params, Value};
use super::schema::EffectSchema;
use crate::expression::ExpressionContext;
use crate::model::EffectInstance;

/// What resolve-time derivation sees (docs/impl/effect-registry.md §2.4a, K-385).
///
/// **In plain terms.** Most effects are entirely described by their controls: a
/// radius, a colour, a mix. A few are not — a Flash fired from beat markers has
/// to know what time it is, which beats the comp carries, and the shape of a
/// whole keyframed track, none of which is a control anyone could slide. This is
/// the small parcel of "everything that is not a parameter" handed to the one
/// hook that needs it, and it carries exactly what the hand-written resolve arms
/// used to read: no more, so the hook cannot quietly grow into a second engine.
pub struct ResolveCx<'a> {
    /// The instance being resolved — its stored properties in full, which is
    /// what lets a derivation read a whole keyframed track rather than one
    /// evaluated number.
    pub inst: &'a EffectInstance,
    /// Layer time, seconds (the held/sample time for a temporal re-render).
    pub lt: f64,
    /// The raster's diagonal in pixels, for a derivation whose result is spatial.
    pub diag_px: f32,
    /// Raster pixels per comp pixel (the §2.3 preview factor), for a derivation
    /// whose result is built from a px@comp control — Scanlines' roll offset is
    /// a product of the layer time and the *raster* line period, so it cannot be
    /// worked out from the authored number alone.
    pub px_scale: f32,
    /// The layer's §1.4 marker context ([`MarkerContext::NONE`] where no comp is
    /// in play — every marker-driven effect falls back gracefully on it).
    pub markers: &'a MarkerContext,
    /// The context every animated read evaluates through, expressions included.
    pub context: Arc<ExpressionContext>,
}

/// What an entry in the catalogue *produces* (K-471 §1.3).
///
/// **In plain terms.** Everything in the catalogue has always been a picture
/// operation: pixels in, pixels out. A **driver** is the other kind — no WGSL,
/// no CPU pixel path, just a scalar or colour worked out at resolve time and
/// handed to whichever parameter is wired to it. Declaring which one an entry
/// is, rather than inferring it, is what lets the whole K-381 registry
/// machinery carry drivers for free: schema-declared parameters, catalogue
/// generation, `list_parameters`, and the Effect-controls rows all work
/// unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signature {
    /// A picture operation — every effect until now, and the default.
    Image {
        /// The **data** outputs this picture operation declares beside its
        /// picture (K-472, K-492, points-stream.md §4.1).
        ///
        /// Empty for every effect but Particulate, which draws its particles
        /// for the chain *and* hands them out as a points stream. Declaring
        /// them here rather than inventing a third signature kind is what lets
        /// the bridge, the label walk and the graph validator read one method
        /// whichever kind an entry is.
        extra: &'static [super::schema::Port],
    },
    /// A driver: no image kernel, and these named data ports.
    Data {
        /// The **data** input ports (K-492, points-stream.md §4.1): wire-only,
        /// with no stored value, nothing to keyframe and no panel row.
        ///
        /// Empty for every driver that only reads its own parameters, which is
        /// all of them until Points sample. A driver whose input is a *number*
        /// declares it as an ordinary schema parameter instead — that one has a
        /// value to fall back on when nothing is wired. A points stream has no
        /// such value, which is the whole reason this list exists.
        inputs: &'static [super::schema::Port],
        /// The output ports, in the order the node draws them.
        outputs: &'static [super::schema::Port],
    },
}

impl Signature {
    /// The type of the named output port, or `None` for a port this signature
    /// does not declare.
    #[must_use]
    pub fn output(self, port: &str) -> Option<super::schema::PortType> {
        self.outputs().iter().find(|p| p.id == port).map(|p| p.ty)
    }

    /// The type of the named **data input** port, or `None` for a port this
    /// signature does not declare.
    #[must_use]
    pub fn input(self, port: &str) -> Option<super::schema::PortType> {
        self.inputs().iter().find(|p| p.id == port).map(|p| p.ty)
    }

    /// This signature's declared **data** input ports — a driver's wire-only
    /// inputs, and empty for a picture operation.
    ///
    /// A picture operation's image and matte inputs are not here: they are
    /// drawn from `INPUT_PORT` and the schema's matte row at the seam, exactly
    /// as its image *output* is. When the points family gives a stack effect a
    /// Points input, this is the method that grows to answer for it, and every
    /// caller already reads it.
    #[must_use]
    pub fn inputs(self) -> &'static [super::schema::Port] {
        match self {
            Signature::Image { .. } => &[],
            Signature::Data { inputs, .. } => inputs,
        }
    }

    /// This signature's **data** output ports: a driver's declared outputs, or
    /// a picture operation's declared extras — empty for all but Particulate.
    ///
    /// The image output itself is not here: every picture operation has one,
    /// it is drawn from `OUTPUT_PORT` at the seam, and a list that repeated it
    /// ninety times would be a second place for it to be wrong.
    #[must_use]
    pub fn outputs(self) -> &'static [super::schema::Port] {
        match self {
            Signature::Image { extra } => extra,
            Signature::Data { outputs, .. } => outputs,
        }
    }
}

/// What a driver is handed when it computes its outputs (K-471 §2.1).
pub struct DriverCx<'a> {
    /// The driver instance's id — Wiggle seeds its noise from it, which is what
    /// makes two Wiggles on one layer wobble differently and the same Wiggle
    /// wobble identically on every machine and in every render.
    pub node: uuid::Uuid,
    /// The driver instance itself — its stored properties in full, which is
    /// what lets Audio level read its layer reference (a binding the resolved
    /// bag deliberately does not carry, K-387).
    pub inst: &'a EffectInstance,
    /// Layer time, seconds.
    pub lt: f64,
    /// This driver's own parameters, already evaluated at `lt` and with any
    /// incoming wires substituted in.
    pub params: super::params::Params<'a>,
    /// Decoded audio, where the host has any to offer. `None` — no host, no
    /// footage, a dangling layer reference — reads as silence, which is the
    /// documented degrade rather than a fault.
    pub audio: Option<&'a dyn AudioTap>,
    /// The value feeding the named **input** port at another layer time, for
    /// the drivers that read their input over a window rather than at a point.
    /// `None` when the port is unwired or the evaluation budget is spent.
    pub sample_input: &'a dyn Fn(&str, f64) -> Option<super::params::Value>,
    /// The **points stream** feeding the named data input this frame, for the
    /// drivers that declare one (points-stream.md §2.2, §3.3). `None` is the
    /// documented empty stream: an unwired socket, a bypassed producer, or a
    /// walk whose budget is spent.
    ///
    /// Shared rather than handed over, because one producer's stream may feed
    /// several wires and it is eight arrays of up to the cap; evaluated once
    /// per producer per frame by the walk's own memo.
    pub points_input: &'a dyn Fn(&str) -> Option<std::rc::Rc<super::points::PointsStream>>,
}

/// Where Audio level gets its samples (K-471 §1.3).
///
/// The engine model cannot decode audio — `lumit-core` knows nothing of media —
/// so the host supplies this and the driver does the maths, which is what keeps
/// the windowed RMS testable against a synthesised tone in this crate.
pub trait AudioTap: Sync {
    /// Mono samples of `layer`'s audio covering `[from, to)` in **layer time**,
    /// appended to `out` in order, and the rate they were taken at. `None` for
    /// a layer with no audio, or a reference that names nothing.
    fn samples(&self, layer: uuid::Uuid, from: f64, to: f64, out: &mut Vec<f32>) -> Option<f64>;
}

/// One effect's behaviour: everything the engine needs of it that is not the
/// declaration itself.
///
/// Implemented once per effect, beside its declaration. The derive macro writes
/// [`schema`](EffectDef::schema) from the declared struct; the effect writes the
/// rest.
pub trait EffectDef: Sync + Send + 'static {
    /// The declaration: parameters, traits, category, version. Generated by
    /// `#[derive(Effect)]` from the parameter struct.
    fn schema(&self) -> &'static EffectSchema;

    /// The CPU reference implementation (docs/08 §1.6) — the oracle the WGSL
    /// kernel is tested against, and the degradation ladder's fallback rung
    /// (K-019).
    ///
    /// `rgba` is premultiplied scene-linear, four floats per pixel, row-major.
    /// The default is identity, which is correct for the orchestration-only
    /// effects (Posterize time, Accumulation motion blur) that have no image
    /// operation of their own.
    fn apply_cpu(&self, _rgba: &mut [f32], _w: u32, _h: u32, _p: Params<'_>) {}

    /// [`apply_cpu`](EffectDef::apply_cpu), told **which instance** it is
    /// rendering and at **what layer time** (K-593).
    ///
    /// The dispatch seam calls this one; the default drops the two extra facts
    /// and calls `apply_cpu`, which is what every built-in implements and what
    /// the oracle tests call directly. A built-in's picture is a function of its
    /// bag and nothing else, so for all of them the two are the same call.
    ///
    /// A **plugin** overrides it instead, because neither fact is in the bag and
    /// it needs both: the frame time is what the plugin is told the frame is,
    /// and the instance is which live copy of the plugin renders it — and, when
    /// it fails, which layer wears the badge.
    fn apply_cpu_at(
        &self,
        _inst: uuid::Uuid,
        _lt: f64,
        rgba: &mut [f32],
        w: u32,
        h: u32,
        p: Params<'_>,
    ) {
        self.apply_cpu(rgba, w, h, p);
    }

    /// Values derived at resolve time from things that are not parameters
    /// (docs/impl/effect-registry.md §2.4a, K-385): layer time, the marker
    /// context, a whole keyframed track.
    ///
    /// Pushed into the bag after the declared parameters, under `ParamId`s the
    /// effect declares beside its schema ids and namespaces `derived.`. They are
    /// never panel rows, never keyframed and never serialised — they are the
    /// *result* of parameters and time, recomputed every resolve exactly as the
    /// hand-written arms recomputed them. The hook reads and pushes; it writes
    /// nothing else. The default pushes nothing, which is every effect but a few.
    fn resolve_derived(&self, _cx: &ResolveCx<'_>, _push: &mut dyn FnMut(ParamId, Value)) {}

    /// Whether this effect has an image operation at all. `false` for the
    /// orchestration-only effects, which the render path skips and the
    /// registry-agreement test excuses from needing a GPU entry.
    fn is_image_op(&self) -> bool {
        true
    }

    /// What this entry produces (K-471 §1.3). Every effect is
    /// [`Signature::Image`] with no extras; a driver declares its output
    /// ports, and Particulate declares its Points output beside its picture.
    fn signature(&self) -> Signature {
        Signature::Image { extra: &[] }
    }

    /// Compute a driver's outputs, pushing `(port id, value)` for each one it
    /// declares. Every image effect pushes nothing, which is the default.
    ///
    /// **Deterministic by construction**: it sees the frame's time, its own
    /// parameters and its own node id, and nothing else — no wall clock, no
    /// render order, no shared state. Two renders of the same project agree bit
    /// for bit, and export equals preview (K-031).
    fn eval_driver(
        &self,
        _cx: &DriverCx<'_>,
        _push: &mut dyn FnMut(&'static str, super::params::Value),
    ) {
    }

    /// How far either side of the frame this driver reads its input, in seconds
    /// (K-471 §2.3) — the **temporal declaration**.
    ///
    /// Zero for a pointwise driver, which is all but two of them: Smooth reads
    /// its input over a window, and Audio level reads sound around the frame.
    /// The declared window folds the sampled range into the frame key, so a
    /// cached frame can never outlive the thing it was smoothed from.
    fn driver_window(&self, _p: super::params::Params<'_>) -> f64 {
        0.0
    }

    /// The source-relative frame offsets **this instance** reads at this layer
    /// time — the picture-side twin of [`driver_window`](EffectDef::driver_window).
    ///
    /// `None` means "whatever the schema declares", which is every built-in:
    /// Echo's window is a fact about the effect, not about the copy of it on
    /// this layer. An **OFX plugin's** is not — a retimer answers
    /// `getFramesNeeded` per instance and per frame (docs/12 §2.1), and the
    /// frames it names have to reach the frame key and the neighbour decode or
    /// a cached frame would outlive the frames it was sampled from.
    ///
    /// It must stay cheap and pure: [`super::stack_temporal_window`] calls it
    /// once per live effect per frame, from the key walk.
    fn frames_needed(&self, _inst: &EffectInstance, _lt: f64) -> Option<Vec<i32>> {
        None
    }

    /// Why the last frame this definition rendered was a placeholder rather
    /// than the effect's own work, if it was (docs/12 §2.3).
    ///
    /// `None` for every built-in: a built-in cannot fail — a missing input is a
    /// passthrough, never a fault. A plugin can, because it is somebody else's
    /// code in somebody else's process, and when it does the layer wants a calm
    /// badge rather than a stopped comp. The dispatch seam reads this straight
    /// after the render and files it under the op's instance.
    fn last_error(&self) -> Option<String> {
        None
    }
}

/// The stable name an effect is looked up by, and the schema that answers to it.
///
/// A thin newtype over the catalogue slice so the lookup has one implementation
/// and one place to gain an index if 35 entries ever becomes 350. It is a linear
/// scan today, exactly as `fx::schema` was, and is called at edit time and once
/// per effect per frame — not per pixel.
///
/// **Two lists, one order** (K-593). The built-ins are the compile-time slice
/// and come first, always, so the Add-effect menu, the command palette and the
/// preset browser see exactly the order §2.6 promised. Behind them sit the
/// entries registered at run time — OFX plugins today, the user's own in time —
/// which are the same [`EffectDef`] trait object arriving by another road. A
/// plugin can therefore never reorder a built-in, and a build with no plugins
/// scanned is byte for byte the catalogue it always was.
pub struct Catalogue {
    defs: &'static [&'static dyn EffectDef],
    /// The run-time half. Written once per definition at scan time and read
    /// everywhere else; a `RwLock` rather than a plain `Vec` because "scan
    /// time" is a moment in a running program, not a moment before `main`.
    extra: std::sync::RwLock<Vec<&'static dyn EffectDef>>,
}

impl Catalogue {
    /// Wrap a list of definitions. `catalogue!` calls this; nothing else should
    /// need to.
    pub const fn new(defs: &'static [&'static dyn EffectDef]) -> Self {
        Self {
            defs,
            extra: std::sync::RwLock::new(Vec::new()),
        }
    }

    /// Add a definition discovered at run time (K-593).
    ///
    /// `false` — and nothing added — when the catalogue already answers to that
    /// `match_name`, whether from the built-in list or from an earlier scan. A
    /// second registration of the same plugin is a rescan, not a second effect,
    /// and two entries under one name would make which of them renders depend
    /// on the order of a directory listing.
    ///
    /// **The definition is `&'static`**, which for a plugin means leaked: it is
    /// discovered while the program runs and then lives as long as the session,
    /// and leaking is the honest spelling of that lifetime. Registration is
    /// additive and never removes, so nothing has to reason about a definition
    /// vanishing under a frame that is already in flight.
    pub fn register(&self, def: &'static dyn EffectDef) -> bool {
        let name = def.schema().match_name;
        let Ok(mut extra) = self.extra.write() else {
            // A poisoned lock means a panic in another thread while it held the
            // write half. Engine crates do not panic (docs/14 §4); refusing the
            // registration is the quiet answer, never a second panic here.
            return false;
        };
        if self.defs.iter().any(|d| d.schema().match_name == name)
            || extra.iter().any(|d| d.schema().match_name == name)
        {
            return false;
        }
        extra.push(def);
        true
    }

    /// The definition named `match_name`, or `None` for a name this build does
    /// not know — an unknown effect is preserved as an inert placeholder, never
    /// an error (docs/08 §5, K-065).
    pub fn get(&self, match_name: &str) -> Option<&'static dyn EffectDef> {
        self.defs
            .iter()
            .copied()
            .find(|d| d.schema().match_name == match_name)
            .or_else(|| {
                self.registered()
                    .into_iter()
                    .find(|d| d.schema().match_name == match_name)
            })
    }

    /// Every definition: the built-ins in catalogue order, then whatever
    /// registered at run time, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &'static dyn EffectDef> + '_ {
        self.defs.iter().copied().chain(self.registered())
    }

    /// The **built-ins only**, in catalogue order.
    ///
    /// For the rules that are statements about Lumit's own declarations rather
    /// than about effects in general: that every effect of ours carries a Matte
    /// row (K-395), that a Mix row comes with a Blend, that a per cent is never
    /// a disguised distance (K-558). A plugin's rows are its own (docs/12 §2.2)
    /// and were written by somebody who never read those conventions, so judging
    /// them by ours would fail the build for somebody else's taste.
    pub fn builtins(&self) -> impl Iterator<Item = &'static dyn EffectDef> + '_ {
        self.defs.iter().copied()
    }

    /// How many effects this session knows — built in and registered.
    pub fn len(&self) -> usize {
        self.defs.len() + self.registered().len()
    }

    /// Whether the catalogue is empty (it never is; the method exists so `len`
    /// does not draw a clippy warning).
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty() && self.registered().is_empty()
    }

    /// The run-time half, copied out. A `Vec` of thin pointers rather than a
    /// borrow of the guard, because the callers above want an iterator that
    /// outlives the read and the list is a handful of entries.
    fn registered(&self) -> Vec<&'static dyn EffectDef> {
        self.extra
            .read()
            .map(|list| list.clone())
            .unwrap_or_default()
    }
}

/// Build the built-in catalogue from a list of modules and their effect types.
///
/// The whole of registration: one line per effect, in menu order. Expands to the
/// `BUILTIN_DEFS` catalogue and the `BUILTINS` schema slice that the Add-effect
/// menu, the bridge and the preset paths have always read, so nothing downstream
/// changes shape.
#[macro_export]
macro_rules! catalogue {
    ($($def:expr => $decl:ty),* $(,)?) => {
        /// Every built-in effect's definition, in menu order.
        pub static BUILTIN_DEFS: $crate::fx::Catalogue =
            $crate::fx::Catalogue::new(&[$(&$def),*]);

        /// Every built-in effect's declaration, in the same order — the list the
        /// Add-effect menu, the command palette, the preset browser and the
        /// bridge have always read.
        ///
        /// Generated from the same line as `BUILTIN_DEFS`, so the two cannot
        /// disagree about which effects exist or about what order they come in;
        /// until the last effect migrated this was a four-thousand-line literal
        /// held against the generated declarations by a test.
        pub const BUILTINS: &[$crate::fx::EffectSchema] =
            &[$(<$decl as $crate::fx::EffectMetadata>::SCHEMA),*];
    };
}

/// The link between a declared parameter struct and its generated declaration.
///
/// `#[derive(Effect)]` implements this. `SCHEMA` is the declaration itself; the
/// behaviour lives on a small companion type implementing [`EffectDef`], written
/// beside the declaration, because an effect's CPU reference is real code and
/// generating it would only hide it.
pub trait EffectMetadata: Sized {
    /// The generated declaration.
    const SCHEMA: EffectSchema;

    /// Read this effect's parameters out of a resolved bag, filling each field
    /// from its declared default when the bag has no entry for it — which is
    /// what makes a project saved before a parameter existed load and render
    /// (K-258).
    fn read(p: Params<'_>) -> Self;
}
