//! LFX effects: the WGSL effects an extension carries.
//!
//! **In plain terms.** An extension can bring effects of its own. Each one is a
//! shader written to the Custom shader's contract, plus a few facts from the
//! extension's manifest: its name, its category, how many passes it draws in.
//! `lumit-extensions` reads the package; what the engine keeps of it is an
//! [`LfxEffect`], and this module is the table of those for the session.
//!
//! The table is replaceable. A rescan installs every effect again, and an
//! effect whose shader changed picks up the new one at once, with no restart.
//! The [`EffectDef`] behind each name is made once and kept, because the
//! catalogue never lets one go; when the extension is removed the def stays
//! and renders its input unchanged, the same as a project naming an effect
//! that was never installed here.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use super::effects::custom_shader::{self as cs, CustomShader};
use super::shader::{self, ShaderProgram, ShaderRefusal};
use super::{
    EffectDef, EffectMetadata, EffectSchema, FxCategory, MatteRole, ParamSchema, ResolveCx, Value,
    LFX_MATCH_PREFIX,
};
use crate::model::EffectInstance;

/// The shader contract this build reads.
pub const API: u32 = 1;

/// The most passes one effect may draw in.
pub const MAX_PASSES: usize = 8;

/// The one pass an effect that lists none draws in.
pub const DEFAULT_PASS: &str = "shade";

/// How many lost graphics devices switch an effect off for the session.
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
    /// The `.wgsl` file's text.
    pub shader: String,
    /// Whether it has an interface page.
    pub interface: bool,
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
    pub effect: LfxEffect,
    pub match_name: &'static str,
    /// The shader, its passes and its version, hashed: what the frame key
    /// folds, so an edited plugin never serves old frames.
    pub key_hash: u64,
    /// The program, or why the shader was refused. A refused one renders its
    /// input unchanged.
    pub program: Result<&'static ShaderProgram, ShaderRefusal>,
}

impl Installed {
    /// The passes in order as (name, scale), with the default when none
    /// are listed.
    #[must_use]
    pub fn passes(&self) -> Vec<(&str, f32)> {
        pass_list(&self.effect)
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

/// The session's installed effects, by match name.
static TABLE: RwLock<BTreeMap<&'static str, Arc<Installed>>> = RwLock::new(BTreeMap::new());

/// Every def made this session, installed or not. Never shrinks.
static DEFS: Mutex<BTreeMap<&'static str, &'static LfxDef>> = Mutex::new(BTreeMap::new());

/// Lost devices counted against each effect.
static STRIKES: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());

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
    let had = installed(&name);
    let program = match &had {
        Some(had) if had.key_hash == key_hash => had.program.clone(),
        _ => {
            let names: Vec<&str> = pass_list(&effect).iter().map(|(n, _)| *n).collect();
            shader::build_passes(&effect.shader, &names).map(|p| &*Box::leak(Box::new(p)))
        }
    };
    let def = def_for(&name, &effect, category);
    let entry = Arc::new(Installed {
        effect,
        match_name: def.match_name,
        key_hash,
        program,
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
    if is_off(match_name) {
        return 1;
    }
    installed(match_name).map_or(0, |e| e.key_hash)
}

/// Count a lost graphics device against an effect. `true` when this strike
/// switched it off.
pub fn strike(match_name: &str) -> bool {
    let Ok(mut strikes) = STRIKES.lock() else {
        return false;
    };
    let n = strikes.entry(match_name.to_owned()).or_insert(0);
    *n += 1;
    *n == STRIKES_OFF
}

/// Whether two lost devices have switched an effect off for the session.
#[must_use]
pub fn is_off(match_name: &str) -> bool {
    STRIKES
        .lock()
        .is_ok_and(|s| s.get(match_name).is_some_and(|n| *n >= STRIKES_OFF))
}

/// The manifest facts, checked. Answers the category.
fn check(effect: &LfxEffect) -> Result<FxCategory, LfxError> {
    for id in [&effect.extension, &effect.id] {
        if id.is_empty() || id.contains('/') || id.chars().any(char::is_whitespace) {
            return Err(LfxError::BadId(id.clone()));
        }
    }
    let category = PICTURE_CATEGORIES
        .iter()
        .copied()
        .find(|c| c.label().eq_ignore_ascii_case(effect.category.trim()))
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
            .and_then(|e| e.program.as_ref().ok().copied())
            .map_or(&[], |p| p.params)
    }

    /// The key hash, so the bag and with it every cache key moves when the
    /// plugin does, and the preview factor the header hands the shader.
    /// Nought while the effect is missing, which the render reads as "draw
    /// nothing".
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(super::ParamId, Value)) {
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
}
