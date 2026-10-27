//! LFX effects: the WGSL effects an extension carries.
//!
//! **In plain terms.** An extension can bring effects of its own. Each one is a
//! shader written to the Custom shader's contract, plus a few facts from the
//! extension's manifest: its name, its category, how many passes it draws in.
//! `lumit-extensions` reads the package; what the engine keeps of it is an
//! [`LfxEffect`], and this module is the table of those while Lumit runs.
//!
//! The table is replaceable. A rescan installs every effect again, and an
//! effect whose shader changed picks up the new one at once, with no restart.
//! The [`EffectDef`] behind each name is made once and kept, because the
//! catalogue never lets one go; when the extension is removed the def stays
//! and renders its input unchanged, the same as a project naming an effect
//! that was never installed here.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use super::effects::custom_shader::{self as cs, CustomShader};
use super::shader::{self, ShaderProgram, ShaderRefusal};
use super::{
    builtins::LFX_MATCH_PREFIX, params::Value, registry::EffectDef, registry::EffectMetadata,
    registry::ResolveCx, schema::EffectSchema, schema::FxCategory, schema::MatteRole,
    schema::ParamSchema,
};
use crate::model::EffectInstance;

/// The shader contract this build reads.
pub const API: u32 = 1;

/// The most passes one effect may draw in.
pub const MAX_PASSES: usize = 8;

/// The one pass an effect that lists none draws in.
pub const DEFAULT_PASS: &str = "shade";

/// How many lost graphics devices switch an effect off until Lumit restarts.
pub const STRIKES_OFF: u32 = 2;

/// One effect an extension carries, as the engine needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct LfxEffect {
    /// The extension's id, the first half of the match name.
    pub extension: String,
    /// The effect's id within its extension.
    pub id: String,
    pub label: String,
    /// One of the picture categories, by its menu label ("Stylise").
    pub category: String,
    /// Bumped by the author when the pixels change.
    pub version: u32,
    /// The shader contract it was written to.
    pub api: u32,
    /// Empty means one pass called `shade`.
    pub passes: Vec<LfxPass>,
    pub input: Option<LfxInput>,
    /// Drawn over the composite below, as an adjustment layer is, even when
    /// the effect sits on a layer with a picture; that picture is then the
    /// second input wherever its row picks no other layer.
    pub backdrop: bool,
    /// The `.wgsl` file's text.
    pub shader: String,
}

/// One pass: a function in the shader, drawn at `scale` of the raster.
#[derive(Debug, Clone, PartialEq)]
pub struct LfxPass {
    pub name: String,
    /// 1, 0.5 or 0.25.
    pub scale: f32,
}

/// The extra picture an effect may take.
#[derive(Debug, Clone, PartialEq)]
pub struct LfxInput {
    pub label: String,
    pub kind: LfxInputKind,
}

/// What the extra picture is: a picture, or a depth (a Depth socket in the
/// node graph).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfxInputKind {
    Image,
    Depth,
}

/// Why an effect was not installed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LfxError {
    #[error("`{0}` is not a usable effect id")]
    BadId(String),
    #[error("`{0}` is not one of Lumit's effect categories")]
    UnknownCategory(String),
    #[error("it needs a newer Lumit (shader api {0})")]
    NewerApi(u32),
    #[error("it has {0} passes, and an effect may have at most 8")]
    TooManyPasses(usize),
    #[error("`{0}` is not a pass name; use lower-case letters, digits and _")]
    BadPassName(String),
    #[error("two passes are called `{0}`")]
    DuplicatePass(String),
    #[error("the {pass} pass has scale {scale}; a pass is drawn at 1, 0.5 or 0.25")]
    BadScale { pass: String, scale: f32 },
    #[error("the {0} pass draws the picture, so it must be full size")]
    LastPassScaled(String),
}

/// One installed effect: what the extension said, and its shader read and
/// wrapped.
#[derive(Debug)]
pub struct Installed {
    pub match_name: &'static str,
    /// The shader, its passes and its version, hashed: what the frame key
    /// folds, so an edited plugin never serves old frames.
    pub key_hash: u64,
    /// What that shader built into, kept under its key hash.
    pub built: &'static Built,
}

/// One shader as it was built, kept for good under its key hash, so a frame
/// resolved before a rescan still draws the shader it was resolved with.
#[derive(Debug)]
pub struct Built {
    /// The program, or why the shader was refused. A refused one renders its
    /// input unchanged.
    pub program: Result<&'static ShaderProgram, ShaderRefusal>,
    passes: Vec<(String, f32)>,
    /// Lost devices counted against this shader. An edited one starts clean.
    strikes: AtomicU32,
}

impl Built {
    /// The passes in order as (name, scale), with the default when none
    /// are listed.
    #[must_use]
    pub fn passes(&self) -> Vec<(&str, f32)> {
        self.passes.iter().map(|(n, s)| (n.as_str(), *s)).collect()
    }

    /// Whether two lost devices have switched this shader off.
    #[must_use]
    pub fn is_off(&self) -> bool {
        self.strikes.load(Ordering::Relaxed) >= STRIKES_OFF
    }
}

fn pass_list(effect: &LfxEffect) -> Vec<(&str, f32)> {
    if effect.passes.is_empty() {
        return vec![(DEFAULT_PASS, 1.0)];
    }
    effect
        .passes
        .iter()
        .map(|p| (p.name.as_str(), p.scale))
        .collect()
}

/// The installed effects, by match name.
static TABLE: RwLock<BTreeMap<&'static str, Arc<Installed>>> = RwLock::new(BTreeMap::new());

/// Every shader built, by key hash. Each distinct shader is kept until Lumit
/// restarts, as the programs are leaked, so this grows only on rescans.
static BUILT: RwLock<BTreeMap<u64, &'static Built>> = RwLock::new(BTreeMap::new());

/// Every def made since Lumit started, installed or not. Never shrinks.
static DEFS: Mutex<BTreeMap<&'static str, &'static LfxDef>> = Mutex::new(BTreeMap::new());

/// `lfx:<extension>/<effect>`.
#[must_use]
pub fn match_name(extension: &str, id: &str) -> String {
    format!("{LFX_MATCH_PREFIX}{extension}/{id}")
}

/// The extension an LFX match name belongs to, for the missing badge.
#[must_use]
pub fn extension_of(match_name: &str) -> Option<&str> {
    let rest = match_name.strip_prefix(LFX_MATCH_PREFIX)?;
    rest.split_once('/').map(|(ext, _)| ext)
}

/// Install one effect, or take its new shader on a rescan.
///
/// The first time a name is seen its def is made; after that the same def is
/// handed back and only the table entry changes. A shader that is refused or
/// will not compile still installs: it renders its input unchanged and
/// [`installed`] says why. The caller registers the def in the catalogue and
/// its pass in the render table (`lumit_render::gpufx::lfx::register`).
///
/// # Errors
/// The manifest facts this build cannot take: an unknown category, a newer
/// api, bad or too many passes.
pub fn install(effect: LfxEffect) -> Result<&'static dyn EffectDef, LfxError> {
    let category = check(&effect)?;
    let name = match_name(&effect.extension, &effect.id);
    let key_hash = key_of(&effect);
    let built = match built(key_hash) {
        Some(built) => built,
        None => {
            let passes = pass_list(&effect);
            let names: Vec<&str> = passes.iter().map(|(n, _)| *n).collect();
            let program =
                shader::build_passes(&effect.shader, &names).map(|p| &*Box::leak(Box::new(p)));
            let passes = passes.iter().map(|(n, s)| ((*n).to_owned(), *s)).collect();
            let built: &'static Built = Box::leak(Box::new(Built {
                program,
                passes,
                strikes: AtomicU32::new(0),
            }));
            match BUILT.write() {
                Ok(mut all) => *all.entry(key_hash).or_insert(built),
                Err(_) => built,
            }
        }
    };
    let def = def_for(&name, &effect, category);
    let entry = Arc::new(Installed {
        match_name: def.match_name,
        key_hash,
        built,
    });
    match TABLE.write() {
        Ok(mut table) => {
            table.insert(def.match_name, entry);
        }
        Err(poisoned) => {
            poisoned.into_inner().insert(def.match_name, entry);
        }
    }
    Ok(def)
}

/// Take an effect out of the table, as removing its extension does. Its def
/// stays and renders its input unchanged. `false` when it was not installed.
pub fn uninstall(match_name: &str) -> bool {
    TABLE
        .write()
        .map(|mut table| table.remove(match_name).is_some())
        .unwrap_or(false)
}

/// The installed effect under `match_name`, or `None` when it is missing.
#[must_use]
pub fn installed(match_name: &str) -> Option<Arc<Installed>> {
    TABLE.read().ok()?.get(match_name).cloned()
}

/// The shader built under `key_hash`, the hash a resolved bag carries, even
/// when a rescan has since installed another.
#[must_use]
pub fn built(key_hash: u64) -> Option<&'static Built> {
    BUILT.read().ok()?.get(&key_hash).copied()
}

/// Whether an LFX effect is installed: the panel's "is this missing?".
#[must_use]
pub fn is_installed(match_name: &str) -> bool {
    TABLE
        .read()
        .is_ok_and(|table| table.contains_key(match_name))
}

/// Every installed effect, in match-name order.
#[must_use]
pub fn all() -> Vec<Arc<Installed>> {
    TABLE
        .read()
        .map(|table| table.values().cloned().collect())
        .unwrap_or_default()
}

/// What the frame key folds for an LFX op: the key hash while it draws, and
/// a different number when it is missing (0) or switched off (1), because
/// then it draws nothing.
#[must_use]
pub fn render_term(match_name: &str) -> u64 {
    match installed(match_name) {
        None => 0,
        Some(e) if e.built.is_off() => 1,
        Some(e) => e.key_hash,
    }
}

/// Count a lost graphics device against the shader under `key_hash`. `true`
/// when this strike switched it off.
pub fn strike(key_hash: u64) -> bool {
    built(key_hash).is_some_and(|b| b.strikes.fetch_add(1, Ordering::Relaxed) + 1 == STRIKES_OFF)
}

/// The manifest facts, checked. Answers the category. Also what
/// `lumit-extensions` asks when it reads a manifest, so a bad one is caught
/// before anything is installed.
///
/// # Errors
/// The facts this build cannot take.
pub fn check(effect: &LfxEffect) -> Result<FxCategory, LfxError> {
    for id in [&effect.extension, &effect.id] {
        if id.is_empty() || id.contains('/') || id.chars().any(char::is_whitespace) {
            return Err(LfxError::BadId(id.clone()));
        }
    }
    // SICELIOT: its heading was once called just "SICELIOT", and manifests
    // written then still name it so.
    let named = match effect.category.trim() {
        old if old.eq_ignore_ascii_case("SICELIOT") => FxCategory::Siceliot.label(),
        other => other,
    };
    let category = PICTURE_CATEGORIES
        .iter()
        .copied()
        .find(|c| c.label().eq_ignore_ascii_case(named))
        .ok_or_else(|| LfxError::UnknownCategory(effect.category.clone()))?;
    if effect.api > API {
        return Err(LfxError::NewerApi(effect.api));
    }
    if effect.passes.len() > MAX_PASSES {
        return Err(LfxError::TooManyPasses(effect.passes.len()));
    }
    for (i, pass) in effect.passes.iter().enumerate() {
        let mut chars = pass.name.chars();
        let good = chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !good {
            return Err(LfxError::BadPassName(pass.name.clone()));
        }
        if effect.passes[..i].iter().any(|p| p.name == pass.name) {
            return Err(LfxError::DuplicatePass(pass.name.clone()));
        }
        if ![1.0, 0.5, 0.25].contains(&pass.scale) {
            return Err(LfxError::BadScale {
                pass: pass.name.clone(),
                scale: pass.scale,
            });
        }
    }
    if let Some(last) = effect.passes.last().filter(|p| p.scale != 1.0) {
        return Err(LfxError::LastPassScaled(last.name.clone()));
    }
    Ok(category)
}

/// The categories a picture effect can be filed under.
const PICTURE_CATEGORIES: &[FxCategory] = &[
    FxCategory::BlurSharpen,
    FxCategory::Colour,
    FxCategory::Distortion,
    FxCategory::Generate,
    FxCategory::Stylise,
    FxCategory::Temporal,
    FxCategory::Transition,
    FxCategory::Utility,
    FxCategory::Siceliot,
];

/// Everything that changes the pixels, hashed.
fn key_of(effect: &LfxEffect) -> u64 {
    let mut bytes = Vec::with_capacity(effect.shader.len() + 64);
    bytes.extend_from_slice(&effect.version.to_le_bytes());
    bytes.extend_from_slice(&effect.api.to_le_bytes());
    for (name, scale) in pass_list(effect) {
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&scale.to_le_bytes());
    }
    bytes.extend_from_slice(&(effect.shader.len() as u64).to_le_bytes());
    bytes.extend_from_slice(effect.shader.as_bytes());
    shader::hash64(&bytes)
}

/// What the schema is built from; a rescan that changes none of it keeps the
/// schema it has.
#[derive(Debug, Clone, PartialEq)]
struct ShapeKey {
    label: String,
    category: FxCategory,
    version: u32,
    input: Option<LfxInput>,
    backdrop: bool,
}

struct Shape {
    key: ShapeKey,
    schema: &'static EffectSchema,
}

/// The def under `name`, made the first time and given a new schema when the
/// manifest's label, category, version or input changed.
fn def_for(name: &str, effect: &LfxEffect, category: FxCategory) -> &'static LfxDef {
    let key = ShapeKey {
        label: effect.label.clone(),
        category,
        version: effect.version,
        input: effect.input.clone(),
        backdrop: effect.backdrop,
    };
    let mut defs = match DEFS.lock() {
        Ok(defs) => defs,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(def) = defs.get(name).copied() {
        let mut shape = match def.shape.write() {
            Ok(shape) => shape,
            Err(poisoned) => poisoned.into_inner(),
        };
        if shape.key != key {
            shape.schema = schema_for(def.match_name, &key);
            shape.key = key;
        }
        return def;
    }
    let match_name: &'static str = Box::leak(name.to_owned().into_boxed_str());
    let def: &'static LfxDef = Box::leak(Box::new(LfxDef {
        match_name,
        shape: RwLock::new(Shape {
            schema: schema_for(match_name, &key),
            key,
        }),
    }));
    defs.insert(match_name, def);
    def
}

/// A leaked schema: the Custom shader's own Mix row, and its second input
/// under the manifest's label when there is one. Its traits are the Custom
/// shader's too, for the same reasons.
fn schema_for(match_name: &'static str, key: &ShapeKey) -> &'static EffectSchema {
    let base = &<CustomShader as EffectMetadata>::SCHEMA;
    let params: Vec<ParamSchema> = base
        .params
        .iter()
        .filter_map(|row| match (row.id, &key.input) {
            ("input2", Some(input)) => Some(ParamSchema {
                label: Box::leak(input.label.clone().into_boxed_str()),
                ..*row
            }),
            ("mix", _) => Some(*row),
            _ => None,
        })
        .collect();
    Box::leak(Box::new(EffectSchema {
        match_name,
        label: Box::leak(key.label.clone().into_boxed_str()),
        version: key.version,
        category: key.category,
        traits: base.traits,
        params: Box::leak(params.into_boxed_slice()),
        groups: &[],
        enabled_when: &[],
        matte: MatteRole::None,
    }))
}

/// One installed effect's behaviour. Its rows beyond Mix and the input come
/// from the shader's `Params`, read from the current table entry, so an
/// update can change them.
pub struct LfxDef {
    match_name: &'static str,
    shape: RwLock<Shape>,
}

impl EffectDef for LfxDef {
    fn schema(&self) -> &'static EffectSchema {
        match self.shape.read() {
            Ok(shape) => shape.schema,
            Err(poisoned) => poisoned.into_inner().schema,
        }
    }

    fn derived(&self, _inst: &EffectInstance) -> &'static [ParamSchema] {
        installed(self.match_name)
            .and_then(|e| e.built.program.as_ref().ok().copied())
            .map_or(&[], |p| p.params)
    }

    /// The key hash, so the bag and with it every cache key moves when the
    /// plugin does, and the preview factor the header hands the shader.
    /// Nought while the effect is missing, which the render reads as "draw
    /// nothing".
    fn resolve_derived(
        &self,
        cx: &ResolveCx<'_>,
        push: &mut dyn FnMut(super::params::ParamId, Value),
    ) {
        let hash = render_term(self.match_name);
        push(cs::HASH_HI, Value::Int((hash >> 32) as u32 as i32));
        push(cs::HASH_LO, Value::Int(hash as u32 as i32));
        push(cs::COMP_SCALE, Value::Float(cx.px_scale));
    }

    fn depth_inputs(&self) -> &'static [&'static str] {
        let depth = match self.shape.read() {
            Ok(shape) => shape.key.input.as_ref().map(|i| i.kind),
            Err(poisoned) => poisoned.into_inner().key.input.as_ref().map(|i| i.kind),
        };
        if depth == Some(LfxInputKind::Depth) {
            &["input2"]
        } else {
            &[]
        }
    }

    fn backdrop(&self) -> bool {
        match self.shape.read() {
            Ok(shape) => shape.key.backdrop,
            Err(poisoned) => poisoned.into_inner().key.backdrop,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const TWO_PASS: &str = "struct Params {\n    /// Amount\n    amount: f32,\n}\n\
        fn bleed(uv: vec2<f32>) -> vec4<f32> { return lumit_sample(uv) * p.amount; }\n\
        fn shade(uv: vec2<f32>) -> vec4<f32> { return lumit_read(pass_bleed, uv); }\n";

    fn effect(shader: &str) -> LfxEffect {
        let pass = |name: &str, scale: f32| LfxPass {
            name: name.into(),
            scale,
        };
        LfxEffect {
            extension: "core-test".into(),
            id: "two-pass".into(),
            label: "Two pass".into(),
            category: "Stylise".into(),
            version: 1,
            api: API,
            passes: vec![pass("bleed", 0.5), pass("shade", 1.0)],
            input: None,
            backdrop: false,
            shader: shader.into(),
        }
    }

    /// Every pass swap lands, the manifest facts this build can't take are
    /// refused, and a changed shader installed over the old one brings its
    /// own rows and a new key hash. Guards wrong pixels and old frames after
    /// an update.
    #[test]
    fn an_lfx_effect_builds_refuses_and_reinstalls() {
        let program = shader::build_passes(TWO_PASS, &["bleed", "shade"]).expect("swaps land");
        assert_eq!(program.passes.len(), 2);

        let refused = |change: fn(&mut LfxEffect)| {
            let mut e = effect(TWO_PASS);
            change(&mut e);
            check(&e).expect_err("refused")
        };
        assert_eq!(refused(|e| e.api = API + 1), LfxError::NewerApi(API + 1));
        assert!(matches!(
            refused(|e| e.category = "Sparkles".into()),
            LfxError::UnknownCategory(_)
        ));
        let mut siceliot = effect(TWO_PASS);
        siceliot.category = "Siceliot FX".into();
        assert_eq!(check(&siceliot), Ok(FxCategory::Siceliot));
        // The heading's old name still files under it.
        siceliot.category = "SICELIOT".into();
        assert_eq!(check(&siceliot), Ok(FxCategory::Siceliot));
        assert!(matches!(
            refused(|e| e.passes[1].scale = 0.5),
            LfxError::LastPassScaled(_)
        ));
        assert!(matches!(
            refused(|e| e.passes = vec![e.passes[1].clone(); MAX_PASSES + 1]),
            LfxError::TooManyPasses(_)
        ));

        let inst = crate::fx::builtins::instantiate("custom_shader").expect("a built-in");
        let rows = |def: &dyn EffectDef| -> Vec<&str> {
            def.derived(&inst).iter().map(|row| row.id).collect()
        };
        let def = install(effect(TWO_PASS)).expect("it installs");
        let name = def.schema().match_name;
        let (old_rows, old_key) = (rows(def), render_term(name));
        let def = install(effect(&TWO_PASS.replace("amount", "strength"))).expect("again");
        assert_eq!(old_rows, ["amount"]);
        assert_eq!(rows(def), ["strength"]);
        assert_ne!(render_term(name), old_key);
        assert_ne!(old_key, 0);

        // A rescan that turns `backdrop` on changes the def it already has.
        assert!(!def.backdrop());
        let mut glass = effect(&TWO_PASS.replace("amount", "strength"));
        glass.backdrop = true;
        let def = install(glass).expect("with backdrop");
        assert!(def.backdrop());
    }
}
