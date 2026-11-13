//! What an export of a composition needs that a computer may not have, and
//! what this computer has: its effects, fonts, model packs and Lumit, and the
//! card, memory and cores the account's other computers choose by.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use lumit_core::model::{Document, LayerKind, ProjectItem};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::composition::CompositionReference;
use crate::api::remote::BridgeRemoteFit;

/// The match names of effects that come from outside Lumit.
const PLUGINS: [&str; 4] = [
    lumit_core::fx::builtins::OFX_MATCH_PREFIX,
    lumit_core::fx::builtins::CLAP_MATCH_PREFIX,
    lumit_core::fx::builtins::VST3_MATCH_PREFIX,
    lumit_core::fx::builtins::LFX_MATCH_PREFIX,
];

fn plugin(match_name: &str) -> bool {
    PLUGINS.iter().any(|prefix| match_name.starts_with(prefix))
}

/// What an export of one composition needs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Needs {
    /// Lumit's own effects it uses. A Lumit without one is too old for it.
    pub builtins: Vec<String>,
    /// Effects from outside Lumit.
    pub plugins: Vec<String>,
    /// Each font family and the face in it.
    pub fonts: Vec<(String, String)>,
    /// Model packs, by id, or by what they do where any pack doing it will.
    pub models: Vec<String>,
    /// The schema of the document, and the Lumit that wrote it.
    pub schema: String,
    pub version: String,
}

/// What a computer lacks of [`Needs`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Lacks {
    pub plugins: Vec<String>,
    pub fonts: Vec<String>,
    pub models: Vec<String>,
    /// The Lumit that can read the project, when this one can't.
    pub too_old: String,
}

/// What an export of `comp` in `doc` needs.
pub(super) fn of(doc: &Document, comp: Uuid) -> Needs {
    let reached = super::files::reached(doc, comp);
    let comps = doc.items.iter().filter_map(|item| match item {
        ProjectItem::Composition(c) if reached.contains(&c.id) => Some(c),
        _ => None,
    });
    let (mut builtins, mut plugins) = (BTreeSet::new(), BTreeSet::new());
    let mut models = BTreeSet::new();
    let mut fonts = BTreeSet::new();
    for comp in comps {
        for fx in comp.effects() {
            let name = fx.effect.match_name.clone();
            if let Some(task) = lumit_core::planes::task_of(fx) {
                let model = lumit_core::planes::PlaneSettings::of(fx).model;
                models.insert(
                    match task {
                        lumit_core::planes::PlaneTask::Depth => {
                            lumit_core::fx::effects::depth::model_pack(model)
                        }
                        lumit_core::planes::PlaneTask::Matte => {
                            lumit_core::fx::effects::remove_background::model_pack(model)
                        }
                    }
                    .to_owned(),
                );
            }
            if name == lumit_core::roto::ROTO_BRUSH {
                models.insert("segmentation".to_owned());
            }
            if plugin(&name) {
                plugins.insert(name);
            } else {
                builtins.insert(name);
            }
        }
        // Flow with the RIFE model makes the frames between, at export.
        use lumit_core::retime::{FlowEngineChoice, Interpolation};
        let rife = |i: &Interpolation| matches!(i, Interpolation::Flow(p) if p.engine == FlowEngineChoice::Rife);
        for layer in &comp.layers {
            let clips = match &layer.kind {
                LayerKind::Sequence { clips } => clips.iter().any(|c| rife(&c.interpolation)),
                _ => false,
            };
            if rife(&layer.interpolation) || clips {
                models.insert("synthesis".to_owned());
            }
        }
        // ponytail: a text style can sit in a layer, a keyframe or a range of
        // letters, so every object in the comp naming a `family` is taken for
        // one. An effect row of that name would only add a font to check.
        if let Ok(value) = serde_json::to_value(comp) {
            families(&value, &mut fonts);
        }
    }
    Needs {
        builtins: builtins.into_iter().collect(),
        plugins: plugins.into_iter().collect(),
        fonts: fonts.into_iter().collect(),
        models: models.into_iter().collect(),
        schema: lumit_project::SCHEMA_VERSION.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

/// Every font family and face named in `value`. An empty family is Lumit's
/// own Inter, which every Lumit has.
fn families(value: &Value, out: &mut BTreeSet<(String, String)>) {
    match value {
        Value::Object(fields) => {
            if let Some(Value::String(family)) = fields.get("family") {
                let face = fields
                    .get("face")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !family.is_empty() {
                    out.insert((family.clone(), face.to_owned()));
                }
            }
            fields.values().for_each(|v| families(v, out));
        }
        Value::Array(list) => list.iter().for_each(|v| families(v, out)),
        _ => {}
    }
}

/// Whether this Lumit's schema is older than `schema`, which it can't read.
pub(super) fn older_than(schema: &str) -> bool {
    let ours = lumit_project::semver_triple(lumit_project::SCHEMA_VERSION);
    match (ours, lumit_project::semver_triple(schema)) {
        (Some(ours), Some(theirs)) => ours < theirs,
        _ => true,
    }
}

/// What this computer lacks of `needs`.
pub(super) fn lacks(needs: &Needs) -> Lacks {
    let effects = &lumit_core::fx::catalogue::BUILTIN_DEFS;
    let missing_builtin = needs
        .builtins
        .iter()
        .any(|name| effects.get(name).is_none());
    let too_old = if older_than(&needs.schema) || missing_builtin {
        needs.version.clone()
    } else {
        String::new()
    };
    let families = lumit_text::fonts::families();
    let has_family = |family: &str| families.iter().any(|f| f.eq_ignore_ascii_case(family));
    let fonts = needs
        .fonts
        .iter()
        .filter(|(family, face)| {
            !has_family(family)
                || (!face.is_empty()
                    && !lumit_text::fonts::faces(family)
                        .iter()
                        .any(|f| f.eq_ignore_ascii_case(face)))
        })
        .map(|(family, face)| {
            if face.is_empty() {
                family.clone()
            } else {
                format!("{family} {face}")
            }
        })
        .collect();
    let packs = models();
    let models = needs
        .models
        .iter()
        .filter(|need| {
            !packs
                .iter()
                .any(|(id, _, task)| id == *need || task == *need)
        })
        .cloned()
        .collect();
    Lacks {
        plugins: needs
            .plugins
            .iter()
            .filter(|name| effects.get(name).is_none())
            .cloned()
            .collect(),
        fonts,
        models,
        too_old,
    }
}

/// The model packs installed here: id, name and what each does.
fn models() -> Vec<(String, String, String)> {
    crate::api::addons::addon_list()
        .into_iter()
        .filter(|addon| addon.kind == crate::api::addons::BridgeAddonKind::Model)
        .map(|addon| (addon.id, addon.name, addon.task))
        .collect()
}

/// What [`state_fields`] last worked out, and when.
static STATE: Mutex<(Option<web_time::Instant>, u64, String)> =
    Mutex::new((None, 0, String::new()));
static WORKING: AtomicBool = AtomicBool::new(false);

/// How long what this computer has is believed before it is asked again.
const FRESH: web_time::Duration = web_time::Duration::from_secs(60);

/// The fields this computer adds to what it says of itself in the room.
/// Worked out on a thread of its own, at first and when a minute old, an
/// addon or plugin came or went, or the card became known, so asking each
/// second costs nothing.
pub(super) fn state_fields() -> String {
    let card = u64::from(lumit_gpu::card::name().is_some());
    let changed =
        lumit_ml::store::generation() + lumit_core::fx::catalogue::BUILTIN_DEFS.len() as u64 + card;
    let (stale, said) = match STATE.lock() {
        Ok(state) => (
            state.0.is_none_or(|at| at.elapsed() >= FRESH) || state.1 != changed,
            state.2.clone(),
        ),
        Err(_) => (false, String::new()),
    };
    if stale && !WORKING.swap(true, Ordering::Relaxed) {
        let spawned = std::thread::Builder::new()
            .name("lumit-remote-state".into())
            .spawn(move || {
                let fields = work_out();
                if let Ok(mut state) = STATE.lock() {
                    *state = (Some(web_time::Instant::now()), changed, fields);
                }
                WORKING.store(false, Ordering::Relaxed);
            });
        if spawned.is_err() {
            WORKING.store(false, Ordering::Relaxed);
        }
    }
    if said.is_empty() {
        "{}".to_owned()
    } else {
        said
    }
}

fn work_out() -> String {
    let plugins = lumit_core::fx::catalogue::BUILTIN_DEFS
        .iter()
        .filter(|def| plugin(def.schema().match_name))
        .count();
    let models: Vec<Value> = models()
        .into_iter()
        .take(16)
        .map(|(id, name, _)| json!({ "id": id, "name": name }))
        .collect();
    json!({
        "gpu": lumit_gpu::card::name().unwrap_or_default().chars().take(80).collect::<String>(),
        "gpuMemory": crate::api::system::video_memory_bytes(),
        "memory": crate::api::system::system_memory_bytes(),
        "cores": std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
        "plugins": plugins,
        "fonts": lumit_text::fonts::families().len(),
        "models": models,
    })
    .to_string()
}

impl super::Remote {
    pub(super) fn fit(&self, comp: &CompositionReference, computer: &str) -> BridgeRemoteFit {
        let Some(found) = self.computer(computer) else {
            return BridgeRemoteFit::Unknown;
        };
        if !found.online || !found.takes {
            return BridgeRemoteFit::Unknown;
        }
        let Ok(project) = comp.project() else {
            return BridgeRemoteFit::Unknown;
        };
        let Some((doc, revision)) = project.read().ok().map(|s| s.store.snapshot_and_revision())
        else {
            return BridgeRemoteFit::Unknown;
        };
        let key = (computer.to_owned(), comp.id, revision);
        {
            let Ok(mut fits) = self.fits.lock() else {
                return BridgeRemoteFit::Unknown;
            };
            if let Some(fit) = fits.get(&key) {
                return fit.clone();
            }
            if fits.len() >= 64 {
                fits.clear();
            }
            fits.insert(key, BridgeRemoteFit::Checking);
        }
        let _ = self.events.try_send(super::Event::Check {
            computer: computer.to_owned(),
            comp: comp.id,
            revision,
            doc,
        });
        BridgeRemoteFit::Checking
    }

    /// A computer answered whether it can export a composition.
    pub(super) fn checked(
        &self,
        computer: &str,
        comp: Uuid,
        revision: u64,
        body: Option<&Value>,
        error: &str,
    ) {
        let lacks: Option<Lacks> = body
            .filter(|_| error.is_empty())
            .and_then(|b| serde_json::from_value(b.get("lacks")?.clone()).ok());
        let fit = match lacks {
            None => BridgeRemoteFit::Unknown,
            Some(lacks) if !lacks.too_old.is_empty() => BridgeRemoteFit::TooOld {
                needs: lacks.too_old,
            },
            Some(lacks)
                if lacks.plugins.is_empty()
                    && lacks.fonts.is_empty()
                    && lacks.models.is_empty() =>
            {
                BridgeRemoteFit::Ready
            }
            Some(lacks) => BridgeRemoteFit::Lacks {
                plugins: lacks.plugins,
                fonts: lacks.fonts,
                models: lacks.models,
            },
        };
        if let Ok(mut fits) = self.fits.lock() {
            fits.insert((computer.to_owned(), comp, revision), fit);
        }
    }
}
