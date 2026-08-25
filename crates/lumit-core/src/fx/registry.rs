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
    /// A driver: no image kernel, and these named output ports.
    Data {
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
            Signature::Data { outputs } => outputs,
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
}

/// The stable name an effect is looked up by, and the schema that answers to it.
///
/// A thin newtype over the catalogue slice so the lookup has one implementation
/// and one place to gain an index if 35 entries ever becomes 350. It is a linear
/// scan today, exactly as `fx::schema` was, and is called at edit time and once
/// per effect per frame — not per pixel.
pub struct Catalogue {
    defs: &'static [&'static dyn EffectDef],
}

impl Catalogue {
    /// Wrap a list of definitions. `catalogue!` calls this; nothing else should
    /// need to.
    pub const fn new(defs: &'static [&'static dyn EffectDef]) -> Self {
        Self { defs }
    }

    /// The definition named `match_name`, or `None` for a name this build does
    /// not know — an unknown effect is preserved as an inert placeholder, never
    /// an error (docs/08 §5, K-065).
    pub fn get(&self, match_name: &str) -> Option<&'static dyn EffectDef> {
        self.defs
            .iter()
            .copied()
            .find(|d| d.schema().match_name == match_name)
    }

    /// Every definition, in catalogue order.
    pub fn iter(&self) -> impl Iterator<Item = &'static dyn EffectDef> + '_ {
        self.defs.iter().copied()
    }

    /// How many effects this build knows.
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Whether the catalogue is empty (it never is; the method exists so `len`
    /// does not draw a clippy warning).
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
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
