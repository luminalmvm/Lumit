//! Custom shader (docs/08 §3.95, docs/impl/custom-shader.md, K-650): the one
//! effect in the catalogue whose program the user writes.
//!
//! **In plain terms.** Drop it on a layer and it does nothing, because it has no
//! program yet. Type one, or load a file somebody sent you, and the layer starts
//! doing whatever the program says. The clever part is the controls: the program
//! says which numbers it wants — a radius, an angle, a colour — and those turn
//! into ordinary rows in the Effect controls panel, with sliders, keyframes and
//! expressions like any built-in effect's rows.
//!
//! **Four declared rows, and every other row it ever shows is derived.** The
//! source is not a parameter (§1.2): a kilobyte of text is not `Copy`, it is the
//! thing the parameter set is derived *from*, and two shader sources cannot be
//! interpolated. It lives on the instance instead, in `EffectInstance.extra`
//! under a `shader` key, which is `#[serde(flatten)]` and so rides through save,
//! load, undo, copy/paste, the `.lumfx` preset and an older reader with no
//! format work at all.
//!
//! **`roi = FullFrame`, `cost = Heavy`.** Both are statements about a program
//! nobody has read. A shader may sample anywhere in its input, so no padding is
//! honest; and its cost is unknowable, so it declares the class that makes the
//! governor cautious rather than the one that makes it optimistic.
//!
//! There is no CPU reference, for the same reason the LUT has none: the effect
//! *is* a GPU program, and there is no second implementation of somebody else's
//! arithmetic to hold it to. `apply_cpu` keeps its identity default, so the
//! degradation ladder's CPU rung renders a Custom shader as a passthrough —
//! exactly what an unset one renders anyway.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema, ParamId, ParamSchema, ResolveCx, Value};
use crate::model::EffectInstance;
use lumit_fx_macros::Effect;

/// The `extra` key the shader block lives under (§1.2).
pub const EXTRA_KEY: &str = "shader";

/// The top half of this instance's source hash, pushed into the bag at resolve
/// time — how the render side finds the assembled text (see
/// [`crate::fx::shader::program_by_hash`]).
pub const HASH_HI: ParamId = ParamId::new("derived.shader_hash_hi");

/// The bottom half of it.
pub const HASH_LO: ParamId = ParamId::new("derived.shader_hash_lo");

/// Raster pixels per px@comp at this frame — the header's `comp_scale`, which
/// nothing downstream of the resolve otherwise knows.
pub const COMP_SCALE: ParamId = ParamId::new("derived.shader_comp_scale");

/// The Custom shader's declared controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "custom_shader",
    label = "Custom shader",
    version = 1,
    category = Utility,
    // A program nobody has read: the honest class is the cautious one.
    cost = Heavy,
    // A shader may sample anywhere in its input, so the whole frame is the only
    // correct region. An ROI declaration that is wrong is a wrong picture.
    roi = FullFrame,
    // The picture arrives premultiplied scene-linear, which is what `src` is
    // documented as; the helpers are how a shader that wants straight colour
    // gets it.
    premultiplied = true,
)]
pub struct CustomShader {
    /// The one extra picture, on the existing auxiliary-layer carriage (K-429).
    /// One, not many: the carriage is one slot per op walked by a shared
    /// counter. A shader that genuinely needs three inputs wants three shaders
    /// and a stack, which is what the stack is for.
    #[layer(label = "Second input", self_default = false)]
    pub input2: bool,

    /// Open the editor surface. A button, not a value (K-417) — the source is
    /// not a parameter, so neither of these two rows carries one.
    #[action(label = "Edit shader…")]
    pub edit: (),

    /// Load a `.wgsl` from disc, copying the text into the instance. The path is
    /// remembered under `extra.shader.origin` for reload and never read at
    /// render: a project must be one file that opens on another machine.
    #[action(label = "Load from file…")]
    pub load_from_file: (),

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    /// It earns the injected Blend row (K-425), so a custom shader gets a blend
    /// mode for free on the same seam every other effect uses, and none of it
    /// reaches the user's code.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

/// The WGSL source this instance holds, or `None` for a fresh one (§1.2).
///
/// A fresh instance is an identity passthrough with no badge: a thing the user
/// must supply cannot have a tasteful default, and an empty effect is not a
/// failed one (K-111).
#[must_use]
pub fn source_of(inst: &EffectInstance) -> Option<&str> {
    inst.extra.get(EXTRA_KEY)?.get("source")?.as_str()
}

/// The stored inner graph, or `None` for a hand-written (or empty) shader
/// (§4.1, CS4).
#[must_use]
pub fn graph_of(inst: &EffectInstance) -> Option<&serde_json::Value> {
    inst.extra.get(EXTRA_KEY)?.get("graph")
}

/// This instance's read and wrapped program, or `None` when it has no source or
/// the source is refused (§2.2).
///
/// **The graph is master when it is there** (§4.1, CS4): an instance holding a
/// graph renders the graph's compiled text and never the cached `source` beside
/// it — a mismatch between the two is a stale cache to overwrite, never a
/// conflict to resolve. The compile is memoised per distinct graph, so this is
/// a hash and a map lookup on the render path either way.
#[must_use]
pub fn program_of(inst: &EffectInstance) -> Option<&'static crate::fx::shader::ShaderProgram> {
    if let Some(graph) = graph_of(inst) {
        let text = crate::fx::shader::compile::source_for(graph).ok()?;
        return crate::fx::shader::program_for(text).ok();
    }
    crate::fx::shader::program_for(source_of(inst)?).ok()
}

/// The Custom shader's behaviour.
///
/// It takes the **default [`Signature::Image`](crate::fx::Signature)** with
/// neither half filled: a shader is a picture operation with one picture in and
/// one out, and its second input is an ordinary layer row rather than a data
/// port. Declaring the default would be writing it twice.
pub struct CustomShaderDef;

impl EffectDef for CustomShaderDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CustomShader as EffectMetadata>::SCHEMA
    }

    /// The rows this instance's own source declares (§1.5), read by the §1.4
    /// line reader and cached per distinct source — so this is a hash and a map
    /// lookup on the render path, not a parse.
    ///
    /// They are **offered**, not adopted: what the document stores is still the
    /// document's, and the stored values are what render. A source that stops
    /// mentioning a uniform leaves its row and its expression alive.
    fn derived(&self, inst: &EffectInstance) -> &'static [ParamSchema] {
        program_of(inst).map_or(&[], |p| p.params)
    }

    /// Two things the render side cannot work out for itself (§2.4a's hook, put
    /// to the use it was written for).
    ///
    /// **Which shader this is**, as the hash of its source. The bag carries plain
    /// numbers by design — nothing owned, nothing borrowed — so a page of text
    /// cannot ride in it; the hash can, and the render asks
    /// [`crate::fx::shader::program_by_hash`] for the assembled module. The
    /// resolve walk that pushes this is the same walk that read the source, so
    /// the program is always there by the time the picture is drawn. It also
    /// means the source is in the per-effect cache key for free, since that key
    /// is the bag: editing a shader renames its intermediate as well as its
    /// frame.
    ///
    /// **The preview factor**, which the header hands the shader as
    /// `comp_scale` so a distance can be written in px@comp and be right at every
    /// resolution.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let hash = program_of(cx.inst).map_or(0, |p| p.source_hash);
        push(HASH_HI, Value::Int((hash >> 32) as u32 as i32));
        push(HASH_LO, Value::Int(hash as u32 as i32));
        push(COMP_SCALE, Value::Float(cx.px_scale));
    }
}

/// The source hash a resolved bag carries, or `None` for an op with no shader.
#[must_use]
pub fn hash_in(p: crate::fx::Params<'_>) -> Option<u64> {
    let hi = p.int(HASH_HI, 0) as u32;
    let lo = p.int(HASH_LO, 0) as u32;
    let hash = (u64::from(hi) << 32) | u64::from(lo);
    (hash != 0).then_some(hash)
}
