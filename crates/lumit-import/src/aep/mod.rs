//! Reading an After Effects project file directly (K-418,
//! docs/impl/ae-import.md §7, docs/11-AE-IMPORT.md §7).
//!
//! In plain terms: the other way into Lumit asks the user to run a script
//! inside After Effects, which writes a folder of JSON describing the project.
//! This way skips all of that and reads the `.aep` file itself. The catch is
//! that Adobe never documented the format, so what the parser knows it knows by
//! measurement: `tools/ae-bridge/fixtures/` holds one real `.aep` beside After
//! Effects' *own* written account of the same project, and
//! `tests/aep_differential.rs` compares them field by field. Nothing in here is
//! believed because it seemed likely; it is believed because the two agree.
//!
//! The important design decision is that this is a second *front end*, not a
//! second importer: [`parse_capture`] fills the very same [`Capture`] the
//! Bridge's bundle reader produces, so the mapping layer, the effect table, the
//! placeholders and the report are shared and untouched. And the vocabulary is
//! funnelled: where the Bridge writes After Effects' own constant name
//! (`SCREEN`, `ALPHA_INVERTED`), this route translates the file's numeric code
//! into the same name through [`enums`], one table per enum, and a code no
//! table knows arrives as the number written out rather than as a guess.
//!
//! **Phase A** (this module) is the container and the structure: the project
//! block, the item tree, comp settings, and the layer stack with its timing,
//! parentage, switches and blend. **Phase B** ([`props`]) is the property
//! system — the `tdgp`/`tdbs`/`tdb4` trees, static values, keyframes, effects,
//! masks, markers and expressions. **Phase C** is the surface: [`crate::open_ae`]
//! routes a picked path to this reader or to the bundle reader by its magic, and
//! the File menu offers both. Two encodings are still owed after it — a text
//! document (`btds`) and a gradient (`GCst`), which arrive named and marked
//! unreadable rather than dropped.
//!
//! One thing about the file is worth saying here because it shapes every
//! number this route reports: **an `.aep` stores only what is not at its
//! default**. A layer nobody moved has no Position record in it at all. So the
//! parser recovers a few hundred of the several thousand properties the
//! scripting DOM lists, and that is the right answer rather than a shortfall —
//! the rest are absent because After Effects would put them back at their
//! defaults too, which is exactly what the mapping layer does with an absent
//! property.
//!
//! Reimplemented in Rust from `forticheprod/aep_parser` (MIT, licence checked
//! 2026-08-21) and `boltframe/aftereffects-aep-parser` (Go, MIT), both read as
//! documentation. No code is vendored from either.

pub mod enums;
mod props;
pub mod rifx;

use std::collections::HashMap;
use std::path::Path;

use crate::capture::{
    Capture, Comp, Item, Layer, Matte, MotionBlur, Project, Property, Switches, Unreadable,
};
use crate::{Bundle, BundleSource, ImportError, Manifest, Report, FORMAT};
use rifx::{
    bit, f32_at, i32_at, open_egg, rational_at, text_of, u16_at, u32_at, u8_at, Chunk, RifxError,
};

/// What can go wrong before there is anything worth mapping.
///
/// Everything smaller than this — an item whose descriptor is missing, a layer
/// record too short to read — is a *skip*, recorded as an [`Unreadable`] row so
/// the import report can name it, never a failure of the whole file
/// (docs/11 §7: "a parse failure on one chunk skips that chunk and continues").
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AepError {
    /// The bytes are not a RIFX `Egg!` container at all.
    #[error(transparent)]
    Container(#[from] RifxError),
    /// The container is fine but holds no item tree, so there is no project in
    /// it to speak of.
    #[error("this After Effects project has no item tree")]
    NoItemTree,
}

/// Everything one parse produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// The capture, in exactly the shape the Bridge's bundle carries.
    pub capture: Capture,
    /// The After Effects build that wrote the file, e.g. `26.0x67`, read from
    /// the `head` chunk's packed version word.
    pub ae_version: Option<String>,
    /// Everything skipped along the way, ready to be report rows.
    pub skipped: Vec<Unreadable>,
}

/// Open an `.aep` and produce the same [`Bundle`] a Lumit Bridge folder does.
///
/// The manifest is synthesised: the format string and schema version are this
/// reader's own, the After Effects version comes out of the file, and there is
/// no Bridge version or export date because no Bridge was involved.
/// [`Bundle::source`] is what tells the report which route was taken.
pub fn open_aep(path: &Path) -> Result<Bundle, ImportError> {
    let bytes = std::fs::read(path)?;
    let parsed = parse_capture(&bytes)?;
    Ok(Bundle {
        manifest: Manifest {
            format: Some(FORMAT.to_string()),
            version: Some("1.0.0".to_string()),
            ae_version: parsed.ae_version,
            bridge_version: None,
            exported: None,
        },
        capture: parsed.capture,
        report: Report {
            unreadables: parsed.skipped,
        },
        source: BundleSource::Aep,
    })
}

/// Parse a whole project out of the bytes of an `.aep`.
///
/// The caller does the reading, so this stays a pure function over a slice:
/// nothing here touches the filesystem, allocates from a length the file
/// declared, or can panic on a malformed byte.
pub fn parse_capture(bytes: &[u8]) -> Result<Parsed, AepError> {
    let mut skipped = Vec::new();
    let mut root = Vec::new();
    for chunk in open_egg(bytes)? {
        match chunk {
            Ok(chunk) => root.push(chunk),
            Err(error) => {
                skipped.push(skip("the project", "RIFX", &error.to_string()));
                break;
            }
        }
    }

    let ae_version = root
        .iter()
        .find(|chunk| chunk.id == *b"head")
        .and_then(|chunk| version_of(chunk.body));

    let project = read_project(&root);

    let Some(folder) = root.iter().find(|chunk| chunk.is_list(b"Fold")) else {
        return Err(AepError::NoItemTree);
    };

    let mut items = Vec::new();
    let mut comp_chunks = Vec::new();
    read_folder(folder.children().ok(), 0, &mut items, &mut comp_chunks);

    let by_id: HashMap<i64, usize> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.id.map(|id| (id, index)))
        .collect();

    // A precomp layer's "source size" is the size of the composition it points
    // at, and a comp's item row does not carry one — the size lives in the
    // comp's own `cdta`. So every comp's raster is read up front, before any
    // layer is, because a layer can name a comp that has not been walked yet.
    let by_comp_id: HashMap<i64, (f64, f64)> = comp_chunks
        .iter()
        .filter_map(|(id, chunk)| {
            let settings = chunk.children().ok().find(|c| c.id == *b"cdta")?;
            let width = u16_at(settings.body, 140)?;
            let height = u16_at(settings.body, 142)?;
            Some((*id, (f64::from(width), f64::from(height))))
        })
        .collect();

    let comps = comp_chunks
        .into_iter()
        .map(|(id, chunk)| read_comp(id, &chunk, &items, &by_id, &by_comp_id, &mut skipped))
        .collect();

    Ok(Parsed {
        capture: Capture {
            project: Some(project),
            items,
            comps,
        },
        ae_version,
        skipped,
    })
}

/// One skipped chunk, said the way a report row says it.
fn skip(where_: &str, what: &str, why: &str) -> Unreadable {
    Unreadable {
        comp: Some(where_.to_string()),
        layer: None,
        path: Some(what.to_string()),
        match_name: None,
        error: Some(why.to_string()),
    }
}

/// The After Effects build, out of `head`'s packed 32-bit version word.
///
/// The word is a bitfield rather than three numbers: the major version is split
/// across two runs (five bits at 26, three at 19, `major = a * 8 + b`), the
/// minor sits at 15, and the build number is the low byte. The fixture's
/// `0x0f100643` reads as 26.0x67, which is what After Effects wrote in the
/// bundle's manifest for the same sitting.
fn version_of(body: &[u8]) -> Option<String> {
    let word = u32_at(body, 4)?;
    let major = ((word >> 26) & 0x1F) * 8 + ((word >> 19) & 0x07);
    let minor = (word >> 15) & 0x0F;
    let build = word & 0xFF;
    Some(format!("{major}.{minor}x{build}"))
}

/// The project-wide settings, which live as loose chunks at the root rather
/// than inside any item.
///
/// Two of them are *presence* flags with no payload at all — `lnrb` for linear
/// blending, `lnrp` for linearising the working space — so the fact being read
/// is whether the chunk is there.
fn read_project(root: &[Chunk<'_>]) -> Project {
    let depth = root
        .iter()
        .find(|chunk| chunk.id == *b"nnhd")
        .or_else(|| root.iter().find(|chunk| chunk.id == *b"nhed"))
        .and_then(|chunk| u8_at(chunk.body, 24))
        .map(enums::bits_per_channel);

    let engine = root
        .iter()
        .find(|chunk| chunk.is_list(b"ExEn"))
        .and_then(|chunk| chunk.children().ok().find(|c| c.id == *b"Utf8"))
        .map(|chunk| chunk.text());

    // The working-space profile is the `Utf8` that follows the `PwCs` marker —
    // identified by the marker rather than by being the first profile envelope,
    // because the display space has an identical envelope right after it. An
    // unset slot holds a literal `{}`, which is the scripting DOM's "None".
    let working_space = root
        .windows(2)
        .find(|pair| {
            pair.first().is_some_and(|chunk| chunk.id == *b"PwCs")
                && pair.get(1).is_some_and(|chunk| chunk.id == *b"Utf8")
        })
        .and_then(|pair| pair.get(1))
        .map(|chunk| String::from_utf8_lossy(chunk.body).into_owned())
        .map(|profile| {
            if profile.trim() == "{}" {
                "None".to_string()
            } else {
                profile
            }
        });

    Project {
        bits_per_channel: depth,
        working_space,
        linear_blending: Some(root.iter().any(|chunk| chunk.id == *b"lnrb")),
        linearize_working_space: Some(root.iter().any(|chunk| chunk.id == *b"lnrp")),
        expression_engine: engine,
    }
}

/// Walk one folder's items, depth first, which is the order the Bridge's walker
/// visits them in — so the two item lists line up entry for entry.
fn read_folder<'a>(
    children: impl Iterator<Item = Chunk<'a>>,
    parent_id: i64,
    items: &mut Vec<Item>,
    comps: &mut Vec<(i64, Chunk<'a>)>,
) {
    for entry in children {
        if !entry.is_list(b"Item") {
            continue;
        }
        let inside: Vec<Chunk<'_>> = entry.children().ok().collect();
        let Some(descriptor) = inside.iter().find(|chunk| chunk.id == *b"idta") else {
            continue;
        };
        let (Some(kind_code), Some(id)) = (
            u16_at(descriptor.body, 0),
            u32_at(descriptor.body, 16).map(i64::from),
        ) else {
            continue;
        };
        let name = inside
            .iter()
            .find(|chunk| chunk.id == *b"Utf8")
            .map(|chunk| chunk.text());

        match kind_code {
            enums::ITEM_FOLDER => {
                items.push(Item {
                    id: Some(id),
                    name,
                    parent_id: Some(parent_id),
                    kind: Some("folder".to_string()),
                    ..Item::default()
                });
                if let Some(sub) = inside.iter().find(|chunk| chunk.is_list(b"Sfdr")) {
                    read_folder(sub.children().ok(), id, items, comps);
                }
            }
            enums::ITEM_COMP => {
                items.push(Item {
                    id: Some(id),
                    name,
                    parent_id: Some(parent_id),
                    kind: Some("comp".to_string()),
                    ..Item::default()
                });
                comps.push((id, entry));
            }
            enums::ITEM_FOOTAGE => {
                items.push(read_footage(id, parent_id, name, &inside));
            }
            // An item type from a newer After Effects: keep the row so the
            // report can name it, and say what the file said.
            other => items.push(Item {
                id: Some(id),
                name,
                parent_id: Some(parent_id),
                kind: Some(other.to_string()),
                ..Item::default()
            }),
        }
    }
}

/// A footage item — which is also how After Effects stores a solid.
///
/// The size lives in the source-settings record `sspc`, shared by every kind of
/// footage. What tells a solid apart is the asset-info record `opti`, whose
/// first four bytes are `Soli`; a solid's colour and its *name* both live in
/// there, which is why a solid's own `Utf8` chunk is empty.
///
/// **The file on disk** is the one interpretation field that is read, and it is
/// read from `LIST Als2` ▸ `alas`, which holds a small JSON object with a
/// `fullpath` key. That is not an offset anybody could get quietly wrong — the
/// field names itself — which is why it is here while the rest of the
/// interpretation (frame rate, alpha handling, fields, pulldown, loop) is not:
/// those are byte offsets, the golden fixture is a solids-and-comps project
/// with no file footage in it, and an unchecked offset is exactly the
/// silently-wrong import this route exists to avoid. They are owed a fixture
/// with real footage (docs/TODO.md).
///
/// **A footage item's name is its file's name.** After Effects writes an empty
/// `Utf8` for an item nobody renamed and displays the file name in the Project
/// panel instead, so an item with no name of its own takes the base name of its
/// path — without which a forty-eight-clip project imports as forty-eight blank
/// rows, and every layer drawing from one arrives blank too (a layer with no
/// name of its own falls back to its source item's).
fn read_footage(id: i64, parent_id: i64, name: Option<String>, inside: &[Chunk<'_>]) -> Item {
    let pin = inside.iter().find(|chunk| chunk.is_list(b"Pin "));
    let within: Vec<Chunk<'_>> = pin.map(|p| p.children().ok().collect()).unwrap_or_default();
    let settings = within.iter().find(|chunk| chunk.id == *b"sspc");
    let asset = within.iter().find(|chunk| chunk.id == *b"opti");
    let alias = within
        .iter()
        .find(|chunk| chunk.is_list(b"Als2"))
        .and_then(|list| list.children().ok().find(|chunk| chunk.id == *b"alas"))
        .and_then(|chunk| serde_json::from_str::<serde_json::Value>(&chunk.text()).ok());
    let path = alias
        .as_ref()
        .and_then(|alias| alias.get("fullpath")?.as_str().map(str::to_string))
        .filter(|path| !path.is_empty());
    let name = name.filter(|name| !name.is_empty());

    // **An image sequence says so in the alias** (K-439): `target_is_folder`,
    // because what the item points at is the folder the run lives in rather
    // than any one file. Like the path beside it, that is a field naming
    // itself and not a byte offset, which is why it can be read here while the
    // rest of the interpretation still waits for a fixture.
    //
    // A sequence also carries its name either side of the frame number, as two
    // extra `Utf8` chunks between the alias list and the asset record — a
    // single file has none. Nothing needs them yet (the run is re-read from
    // the folder), so they are recorded rather than acted on; they corroborate
    // the flag above, and they are the only thing After Effects knows about the
    // run that the folder itself does not say.
    let is_sequence = alias
        .as_ref()
        .and_then(|alias| alias.get("target_is_folder")?.as_bool())
        .unwrap_or(false);

    let run_names: Vec<String> = match (
        within.iter().position(|chunk| chunk.is_list(b"Als2")),
        within.iter().position(|chunk| chunk.id == *b"opti"),
    ) {
        (Some(after), Some(before)) => within
            .get(after + 1..before)
            .unwrap_or_default()
            .iter()
            .filter(|chunk| chunk.id == *b"Utf8")
            .map(Chunk::text)
            .filter(|part| !part.is_empty())
            .collect(),
        _ => Vec::new(),
    };

    let solid = asset.filter(|chunk| chunk.body.get(..4) == Some(b"Soli"));
    let (kind, name, colour) = match solid {
        Some(chunk) => (
            "solid",
            chunk
                .body
                .get(26..)
                .map(text_of)
                .filter(|solid_name| !solid_name.is_empty())
                .or(name),
            Some(vec![
                f64::from(f32_at(chunk.body, 14).unwrap_or_default()),
                f64::from(f32_at(chunk.body, 18).unwrap_or_default()),
                f64::from(f32_at(chunk.body, 22).unwrap_or_default()),
            ]),
        ),
        // The file's own name, taken apart by hand rather than by `Path`: the
        // path in the file is written in the separator of the machine that
        // wrote it, and a Windows path handed to `Path::file_name` on macOS
        // comes back whole.
        None => (
            "footage",
            name.or_else(|| {
                path.as_deref()
                    .and_then(|path| path.rsplit(['/', '\\']).next())
                    .filter(|base| !base.is_empty())
                    .map(str::to_string)
            }),
            None,
        ),
    };

    Item {
        id: Some(id),
        name,
        parent_id: Some(parent_id),
        kind: Some(kind.to_string()),
        path,
        colour,
        is_sequence: is_sequence.then_some(true),
        sequence_prefix: run_names.first().cloned(),
        sequence_suffix: run_names.get(1).cloned(),
        width: settings
            .and_then(|chunk| u16_at(chunk.body, 32))
            .map(u32::from),
        height: settings
            .and_then(|chunk| u16_at(chunk.body, 36))
            .map(u32::from),
        ..Item::default()
    }
}

/// The rasters a layer's property tree is read against: the composition the
/// layer sits in, and every composition's own size — because a precomp layer's
/// source size is the size of the comp it points at, and a comp's item row does
/// not carry one.
#[derive(Clone, Copy)]
struct Rasters<'a> {
    comp: (f64, f64),
    by_comp_id: &'a HashMap<i64, (f64, f64)>,
}

/// A composition's settings and its layer stack.
///
/// Only `LIST:Layr` children are layers. A comp also holds `DLay`, `SLay` and
/// `CLay` — the viewer's own default, side and custom view cameras — and
/// `SecL`, a hidden layer whose only job is to carry the comp's markers. All
/// four are layer-shaped and none of them is a layer, so reading "every layer
/// record in the comp" would import eleven phantom rows per composition.
fn read_comp(
    id: i64,
    entry: &Chunk<'_>,
    items: &[Item],
    by_id: &HashMap<i64, usize>,
    by_comp_id: &HashMap<i64, (f64, f64)>,
    skipped: &mut Vec<Unreadable>,
) -> Comp {
    let inside: Vec<Chunk<'_>> = entry.children().ok().collect();
    let name = items
        .get(by_id.get(&id).copied().unwrap_or(usize::MAX))
        .and_then(|item| item.name.clone())
        .unwrap_or_else(|| format!("item {id}"));

    let renderer = inside
        .iter()
        .find(|chunk| chunk.is_list(b"PRin"))
        .and_then(|chunk| chunk.children().ok().find(|c| c.id == *b"prin"))
        .and_then(|chunk| chunk.body.get(4..52))
        .map(|raw| enums::renderer(&text_of(raw)));

    let mut comp = Comp {
        id: Some(id),
        renderer,
        ..Comp::default()
    };

    match inside.iter().find(|chunk| chunk.id == *b"cdta") {
        Some(chunk) => read_settings(chunk.body, &mut comp),
        None => skipped.push(skip(&name, "cdta", "the comp settings record is missing")),
    }

    let records: Vec<Chunk<'_>> = inside
        .iter()
        .filter(|chunk| chunk.is_list(b"Layr"))
        .copied()
        .collect();

    // A layer's parent and its matte are both stored as another layer's id, so
    // the whole stack's ids have to be known before any one layer is read.
    let indices: HashMap<u32, u32> = records
        .iter()
        .enumerate()
        .filter_map(|(position, record)| {
            let descriptor = record.children().ok().find(|c| c.id == *b"ldta")?;
            let layer_id = u32_at(descriptor.body, 0)?;
            Some((
                layer_id,
                u32::try_from(position).unwrap_or(0).saturating_add(1),
            ))
        })
        .collect();

    // Keyframe times are counted in the comp's own internal timebase, and the
    // property trees below need it before any layer is read.
    let timebase = inside
        .iter()
        .find(|chunk| chunk.id == *b"cdta")
        .and_then(|chunk| u32_at(chunk.body, 8))
        .map_or(1.0, f64::from);
    let size = (
        f64::from(comp.width.unwrap_or_default()),
        f64::from(comp.height.unwrap_or_default()),
    );
    let rasters = Rasters {
        comp: size,
        by_comp_id,
    };

    for (position, record) in records.iter().enumerate() {
        let index = u32::try_from(position).unwrap_or(0).saturating_add(1);
        match read_layer(index, record, items, by_id, &indices, timebase, rasters) {
            Some((layer, mut rows)) => {
                comp.layers.push(layer);
                for row in &mut rows {
                    row.comp = Some(name.clone());
                }
                skipped.append(&mut rows);
            }
            None => skipped.push(skip(&name, "ldta", "a layer record could not be read")),
        }
    }

    // A comp's markers are not in `cdta` at all: After Effects keeps them on a
    // hidden `SecL` layer that exists for exactly that purpose.
    comp.markers = inside
        .iter()
        .find(|chunk| chunk.is_list(b"SecL"))
        .and_then(|secret| secret.children().ok().find(|chunk| chunk.is_list(b"tdgp")))
        .map(|group| {
            props::read_markers(
                &group,
                props::Ctx {
                    params: None,
                    timebase,
                    comp: size,
                    layer: size,
                    has_source: false,
                    start: 0.0,
                    in_effect: false,
                },
            )
        })
        .unwrap_or_default();

    // Being *used* as somebody's matte is the one thing a layer cannot say
    // about itself: it is a fact about whoever points at it, so it is filled in
    // once the whole stack is read. Camera and light layers have no matte block
    // at all, and must not gain one here.
    let used: Vec<u32> = comp
        .layers
        .iter()
        .filter_map(|layer| layer.matte.as_ref().and_then(|matte| matte.layer_index))
        .collect();
    for layer in &mut comp.layers {
        let index = layer.index;
        if let Some(matte) = layer.matte.as_mut() {
            if matte.kind.is_some() {
                matte.is_track_matte = Some(index.is_some_and(|index| used.contains(&index)));
            }
        }
    }

    comp
}

/// The `cdta` record — 204 bytes of fixed layout, every offset below proven
/// against the golden capture (docs/impl/ae-import.md §7 carries the map).
fn read_settings(body: &[u8], comp: &mut Comp) {
    comp.width = u16_at(body, 140).map(u32::from);
    comp.height = u16_at(body, 142).map(u32::from);
    comp.par = match (u32_at(body, 144), u32_at(body, 148)) {
        (Some(top), Some(bottom)) if bottom != 0 => Some(f64::from(top) / f64::from(bottom)),
        _ => None,
    };
    // The frame rate is an integer plus a fraction in 1/65536ths, which is how
    // 23.976 survives as itself rather than as a rounding.
    comp.fps = match (u16_at(body, 156), u16_at(body, 158)) {
        (Some(whole), Some(fraction)) => Some(f64::from(whole) + f64::from(fraction) / 65536.0_f64),
        _ => None,
    };
    comp.duration = rational_at(body, 44, 48);
    comp.start = rational_at(body, 164, 168);
    comp.bg_colour = match (u8_at(body, 52), u8_at(body, 53), u8_at(body, 54)) {
        (Some(r), Some(g), Some(b)) => Some(vec![
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
        ]),
        _ => None,
    };

    let flags = u8_at(body, 139).unwrap_or_default();
    comp.motion_blur = Some(MotionBlur {
        enabled: Some(bit(flags, 3)),
        shutter_angle: u16_at(body, 174).map(f64::from),
        shutter_phase: i32_at(body, 180).map(f64::from),
        samples: i32_at(body, 200).map(|value| value.unsigned_abs()),
        adaptive_limit: i32_at(body, 196).map(|value| value.unsigned_abs()),
    });
    comp.preserve_nested_fps = Some(bit(flags, 5));
    comp.preserve_nested_resolution = Some(bit(flags, 7));
}

/// One layer, out of its `ldta` record plus the name chunk beside it.
///
/// Returns `None` only when the record is missing or too short to read at all,
/// which the caller turns into a skipped-chunk row; otherwise the layer and
/// whatever its property tree could not read.
fn read_layer(
    index: u32,
    record: &Chunk<'_>,
    items: &[Item],
    by_id: &HashMap<i64, usize>,
    indices: &HashMap<u32, u32>,
    timebase: f64,
    rasters: Rasters<'_>,
) -> Option<(Layer, Vec<Unreadable>)> {
    let comp_size = rasters.comp;
    let inside: Vec<Chunk<'_>> = record.children().ok().collect();
    let descriptor = inside.iter().find(|chunk| chunk.id == *b"ldta")?;
    let d = descriptor.body;
    // 132 bytes reaches the layer type and the parent id; the matte reference
    // at 160 is younger than the record (After Effects 23) and is read only
    // when it is there.
    if d.len() < 136 {
        return None;
    }

    let flags_name = u8_at(d, 37).unwrap_or_default();
    let flags_rig = u8_at(d, 38).unwrap_or_default();
    let flags_switch = u8_at(d, 39).unwrap_or_default();
    let transfer = u8_at(d, 103).unwrap_or_default();

    let source_id = u32_at(d, 40)
        .filter(|id| *id != 0 && *id != u32::MAX)
        .map(i64::from);
    let source = source_id
        .and_then(|id| by_id.get(&id))
        .and_then(|index| items.get(*index));

    let type_code = u32::from(u8_at(d, 131).unwrap_or_default());
    let is_null = bit(flags_rig, 7);
    let is_adjustment = bit(flags_rig, 1);
    // A null and an adjustment layer are both backed by a solid item, so the
    // layer's own switch has to win over its source or a rig's null imports as
    // the white card it is made of (docs/impl/ae-import.md §5).
    let kind = if type_code == 0 && is_null {
        "null".to_string()
    } else if type_code == 0 && is_adjustment {
        "adjustment".to_string()
    } else if type_code == 0 {
        match source.and_then(|item| item.kind.as_deref()) {
            Some("comp") => "precomp".to_string(),
            Some("solid") => "solid".to_string(),
            _ => "footage".to_string(),
        }
    } else {
        enums::layer_kind(type_code)
    };
    let is_rig = kind == "camera" || kind == "light";

    // The name is the `Utf8` chunk beside the record when there is one, the
    // 32-byte name inside the record when the file put it there, and otherwise
    // the source item's name — which is what After Effects displays for a layer
    // nobody has renamed.
    let name = inside
        .iter()
        .find(|chunk| chunk.id == *b"Utf8")
        .map(|chunk| chunk.text())
        .filter(|name| !name.is_empty())
        .or_else(|| d.get(64..96).map(text_of).filter(|name| !name.is_empty()))
        .or_else(|| source.and_then(|item| item.name.clone()));

    // Stretch is stored as the ratio it is, not as the percentage scripting
    // reports. The in and out points are stored **on the layer's own clock and
    // unstretched** — the same convention the keyframe times use ([`props`]'s
    // `time_of`) — while scripting reports them on the comp's, so the layer's
    // start is added rather than pivoted about: `start + raw × stretch`. At a
    // negative stretch the two ends come back the other way round, which is a
    // swap, not a repair (docs/impl/ae-import.md §7.1).
    let stretch_top = i32_at(d, 8).unwrap_or(1);
    let stretch_bottom = u32_at(d, 108).unwrap_or(1);
    let stretch = if stretch_bottom == 0 {
        0.0
    } else {
        f64::from(stretch_top) / f64::from(stretch_bottom)
    };
    let start_time = rational_at(d, 12, 16).unwrap_or_default();
    let stretched = |dividend: usize, divisor: usize| {
        rational_at(d, dividend, divisor).map(|raw| start_time + raw * stretch)
    };

    let matte = if is_rig {
        Matte::default()
    } else {
        Matte {
            kind: u8_at(d, 107).map(|code| enums::matte(u32::from(code))),
            layer_index: u32_at(d, 160)
                .filter(|id| *id != 0)
                .and_then(|id| indices.get(&id).copied()),
            is_track_matte: None,
        }
    };

    let switches = if is_rig {
        // A camera and a light have no video, audio, quality, frame blending or
        // effects of their own, and the scripting DOM does not offer those
        // switches on them — so the capture must not either.
        Switches {
            enabled: Some(bit(flags_switch, 0)),
            solo: Some(bit(flags_rig, 3)),
            lock: Some(bit(flags_switch, 5)),
            shy: Some(bit(flags_switch, 6)),
            adjustment: Some(is_adjustment),
            ..Switches::default()
        }
    } else {
        Switches {
            enabled: Some(bit(flags_switch, 0)),
            audio: Some(bit(flags_switch, 1)),
            solo: Some(bit(flags_rig, 3)),
            lock: Some(bit(flags_switch, 5)),
            shy: Some(bit(flags_switch, 6)),
            quality: u16_at(d, 4).map(|code| enums::quality(u32::from(code))),
            motion_blur: Some(bit(flags_switch, 3)),
            adjustment: Some(is_adjustment),
            three_d: Some(bit(flags_rig, 2)),
            collapse: Some(bit(flags_switch, 7)),
            frame_blending: Some(enums::frame_blending(
                bit(flags_switch, 4),
                bit(flags_name, 2),
            )),
            guide: Some(bit(flags_name, 1)),
            effects_active: Some(bit(flags_switch, 2)),
        }
    };

    // The property tree hangs off the layer's own `LIST tdgp`. Everything it
    // needs to put values back into the DOM's units — the comp's size, the
    // layer's source size, whether it has one at all, and where its clock
    // starts — is settled by now.
    let source_size = source
        .map(|item| match item.kind.as_deref() {
            // A comp's item row carries no size; its own settings record does.
            Some("comp") => source_id
                .and_then(|id| rasters.by_comp_id.get(&id))
                .copied()
                .unwrap_or(comp_size),
            _ => (
                f64::from(item.width.unwrap_or_default()),
                f64::from(item.height.unwrap_or_default()),
            ),
        })
        .unwrap_or(comp_size);
    let ctx = props::Ctx {
        params: None,
        timebase,
        comp: comp_size,
        layer: source_size,
        has_source: source.is_some(),
        start: start_time,
        in_effect: false,
    };
    let (mut properties, markers, mut rows) = match inside.iter().find(|c| c.is_list(b"tdgp")) {
        Some(group) => {
            let read = props::read_group(group, ctx);
            (
                read.properties,
                props::read_markers(group, ctx),
                read.skipped,
            )
        }
        None => (Vec::new(), Vec::new(), Vec::new()),
    };
    if !is_rig {
        place_at_defaults(&mut properties, ctx, kind != "null");
    }
    let layer_name = name.clone().unwrap_or_else(|| format!("layer {index}"));
    for row in &mut rows {
        row.layer = Some(layer_name.clone());
    }

    Some((
        Layer {
            index: Some(index),
            name,
            kind: Some(kind.clone()),
            source_id,
            in_point: stretched(20, 24),
            out_point: stretched(28, 32),
            start_time: Some(start_time),
            stretch: Some(stretch * 100.0),
            parent_index: u32_at(d, 132)
                .filter(|id| *id != 0)
                .and_then(|id| indices.get(&id).copied()),
            label: u8_at(d, 61).map(u32::from),
            blend: (!is_rig).then(|| {
                enums::blend(
                    u32::from(u8_at(d, 99).unwrap_or_default()),
                    bit(transfer, 1),
                )
            }),
            preserve_transparency: (!is_rig).then(|| bit(transfer, 0)),
            auto_orient: Some(enums::auto_orient(
                bit(flags_rig, 0),
                (bit(flags_rig, 6) || bit(flags_rig, 5)) && bit(flags_rig, 2),
                bit(flags_name, 4) && bit(flags_name, 3),
            )),
            light_type: (kind == "light")
                .then(|| enums::light_type(u32::from(u8_at(d, 139).unwrap_or_default()))),
            matte: Some(matte),
            switches: Some(switches),
            markers,
            time_remap_enabled: (!is_rig).then(|| time_remap_enabled(&inside)),
            properties,
        },
        rows,
    ))
}

/// The two placing properties After Effects does not default to zero, written
/// in where the file left them out.
///
/// The module's opening rule — an `.aep` stores only what is *not* at its
/// default — is harmless for everything whose default is zero, and quietly
/// wrong for the two that place the layer: **Position starts at the centre of
/// the composition and the Anchor Point at the centre of the layer's own
/// source**. Read as (0, 0) instead, a layer nobody moved pins its top-left
/// corner to the top-left of the frame, and every scale and rotation on it
/// pivots from that corner. So the defaults are put back here, where the comp's
/// size and the source's size are both already in hand, rather than guessed at
/// downstream where neither is.
///
/// Three kinds of layer keep the **zero** anchor, which is After Effects' own
/// default for them: a shape, a text layer, and a null. None of the three draws
/// a source rectangle — a shape and a text layer are drawn around their own
/// origin, and a null is a handle with nothing in it (the 100×100 solid behind
/// it is plumbing, not a picture) — so there is nothing to centre on. A camera
/// and a light are left alone entirely: the scripting DOM does not offer these
/// two properties on a rig in this form.
fn place_at_defaults(properties: &mut [Property], ctx: props::Ctx<'_>, centre_anchor: bool) {
    let Some(group) = properties
        .iter_mut()
        .find(|node| node.match_name.as_deref() == Some("ADBE Transform Group"))
        .and_then(|node| node.group.as_mut())
    else {
        return;
    };
    let mut centre = |match_name: &str, (width, height): (f64, f64)| {
        if group
            .iter()
            .any(|node| node.match_name.as_deref() == Some(match_name))
        {
            return;
        }
        // Scripting reports both as three-dimensional whatever the layer's own
        // 3D switch says, so the depth is present and zero.
        group.push(Property {
            match_name: Some(match_name.to_string()),
            value_type: Some("point3".to_string()),
            value: Some(serde_json::json!([width / 2.0, height / 2.0, 0.0])),
            ..Property::default()
        });
    };
    centre("ADBE Position", ctx.comp);
    if ctx.has_source && centre_anchor {
        centre("ADBE Anchor Point", ctx.layer);
    }
}

/// Whether the layer's Time Remap is switched on.
///
/// Kept below the layer reader on purpose: it is the one place phase A touches
/// the property system at all.
///
/// This one fact does not live in the layer record: scripting reports it as
/// "the `ADBE Time Remapping` property is animated", and that flag sits in the
/// property's own metadata record. So it is read from the one place the
/// property system is unavoidable — the layer's root group, where match names
/// arrive as `tdmn` chunks each followed by the node they name.
fn time_remap_enabled(inside: &[Chunk<'_>]) -> bool {
    let Some(group) = inside.iter().find(|chunk| chunk.is_list(b"tdgp")) else {
        return false;
    };
    let mut named = false;
    for child in group.children().ok() {
        if child.id == *b"tdmn" {
            named = child.text() == "ADBE Time Remapping";
        } else if named && child.is_list(b"tdbs") {
            return child
                .children()
                .ok()
                .find(|chunk| chunk.id == *b"tdb4")
                .and_then(|chunk| u8_at(chunk.body, 68))
                .is_some_and(|animated| animated != 0);
        }
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A chunk: name, big-endian size, body, and a pad byte when the body is
    /// odd.
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn list(list_type: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut inner = list_type.to_vec();
        inner.extend_from_slice(body);
        chunk(b"LIST", &inner)
    }

    fn file(body: &[u8]) -> Vec<u8> {
        let mut out = b"RIFX".to_vec();
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend_from_slice(b"Egg!");
        out.extend_from_slice(body);
        out
    }

    /// An item descriptor: type at 0, id at 16, in a body long enough to hold
    /// both.
    fn idta(kind: u16, id: u32) -> Vec<u8> {
        let mut body = vec![0_u8; 84];
        body.splice(0..2, kind.to_be_bytes());
        body.splice(16..20, id.to_be_bytes());
        chunk(b"idta", &body)
    }

    /// A layer descriptor: stretch 1, and the three rationals the timing is
    /// read from, each given as dividend over `divisor`.
    fn ldta(divisor: u32, start: i32, in_point: i32, out_point: i32) -> Vec<u8> {
        let mut body = vec![0_u8; 164];
        let mut put = |at: usize, value: i32| {
            body.splice(at..at + 4, value.to_be_bytes());
        };
        put(8, 1); // stretch dividend
        put(12, start);
        put(16, divisor as i32);
        put(20, in_point);
        put(24, divisor as i32);
        put(28, out_point);
        put(32, divisor as i32);
        put(108, 1); // stretch divisor
        chunk(b"ldta", &body)
    }

    /// One comp holding the given layer records, with settings enough to read.
    fn comp_with(id: u32, name: &[u8], layers: &[Vec<u8>]) -> Vec<u8> {
        let mut comp = idta(ITEM_COMP_KIND, id);
        comp.extend(chunk(b"Utf8", name));
        comp.extend(chunk(b"cdta", &[0; 204]));
        for layer in layers {
            comp.extend(list(b"Layr", layer));
        }
        file(&list(b"Fold", &list(b"Item", &comp)))
    }

    /// **A layer dragged along the timeline keeps its place.**
    ///
    /// The file counts a layer's in and out points on the layer's *own* clock,
    /// the same way it counts keyframe times, so the layer's start is what
    /// puts them back on the comp's. Reading them as comp times instead puts
    /// every dragged layer's bar at the origin — which is a project that opens
    /// with its assembling comps transparent at almost every frame, because
    /// each clip sits at 0 rather than where it was cut.
    #[test]
    fn a_layers_in_and_out_are_counted_from_its_own_start() {
        // Start 2.4 s, a bar 2.48 s long on the layer's clock.
        let bytes = comp_with(3, b"Clips", &[ldta(1000, 2400, 0, 2480)]);
        let parsed = parse_capture(&bytes).expect("the walk survives");
        let layer = &parsed.capture.comps[0].layers[0];
        assert_eq!(layer.start_time, Some(2.4));
        assert_eq!(layer.in_point, Some(2.4), "the bar begins where it was cut");
        assert_eq!(layer.out_point, Some(4.88));
    }

    /// **And the stretch multiplies the layer-local time, not a pivot about
    /// the start.**
    ///
    /// A layer stretched to 50 % is half as long on the comp's clock; its
    /// start does not move, because the start *is* where its own clock begins.
    #[test]
    fn a_stretched_layer_is_stretched_from_its_start() {
        let mut record = ldta(1000, 4000, 0, 10_000);
        // Stretch 1/2: dividend at 8, divisor at 108, inside the `ldta` body,
        // which the chunk header offsets by eight.
        record.splice(8 + 8..8 + 12, 1_i32.to_be_bytes());
        record.splice(8 + 108..8 + 112, 2_u32.to_be_bytes());
        let bytes = comp_with(3, b"Clips", &[record]);

        let layer = &parse_capture(&bytes)
            .expect("the walk survives")
            .capture
            .comps[0]
            .layers[0];
        assert_eq!(layer.stretch, Some(50.0));
        assert_eq!(layer.in_point, Some(4.0));
        assert_eq!(layer.out_point, Some(9.0));
    }

    /// **A footage item takes its file's path, and its name from that file.**
    ///
    /// After Effects leaves an item's own name chunk empty until somebody
    /// renames it and shows the file name in the Project panel instead. Without
    /// the path there is nothing to show and nothing to relink from: a
    /// forty-eight-clip project imports as forty-eight blank rows pointing
    /// nowhere, and every layer drawing from one arrives blank too.
    #[test]
    fn a_footage_item_is_named_and_pathed_from_its_file() {
        // The JSON escapes its backslashes, exactly as After Effects writes it.
        let alias =
            br#"{"ascendcount_base":1,"fullpath":"C:\\Clips\\Cine1\\Depth.avi","platform":1}"#;
        let mut pin = chunk(b"sspc", &[0; 222]);
        pin.extend(list(b"Als2", &chunk(b"alas", alias)));
        let mut item = idta(ITEM_FOOTAGE_KIND, 11);
        item.extend(chunk(b"Utf8", b""));
        item.extend(list(b"Pin ", &pin));
        let bytes = file(&list(b"Fold", &list(b"Item", &item)));

        let item = &parse_capture(&bytes)
            .expect("the walk survives")
            .capture
            .items[0];
        assert_eq!(item.kind.as_deref(), Some("footage"));
        assert_eq!(item.path.as_deref(), Some(r"C:\Clips\Cine1\Depth.avi"));
        assert_eq!(
            item.name.as_deref(),
            Some("Depth.avi"),
            "a Windows path is taken apart by hand, so this holds on macOS too"
        );
    }

    /// **An image sequence is read from the two things After Effects says
    /// about it, and a single file from neither** (K-439).
    ///
    /// Both signals here were taken off a real project (the golden fixture has
    /// no file footage in it, which is why the rest of the interpretation is
    /// still owed one): the alias targets a folder, and two `Utf8` chunks
    /// carrying the name either side of the frame number sit between the alias
    /// list and the asset record. A single file's `Pin ` has neither, which is
    /// what the second half of this asserts — a plain clip must not come in as
    /// a sequence pointing at its own folder.
    #[test]
    fn a_folder_targeting_alias_and_its_two_names_make_an_image_sequence() {
        let alias = br#"{"fullpath":"C:\\Clips\\Cine3\\Depth","target_is_folder":true}"#;
        let mut pin = chunk(b"sspc", &[0; 222]);
        pin.extend(chunk(b"Utf8", b""));
        pin.extend(list(b"Als2", &chunk(b"alas", alias)));
        pin.extend(chunk(b"Utf8", b"Depth"));
        pin.extend(chunk(b"Utf8", b"_depth.exr"));
        pin.extend(chunk(b"opti", b"oEXR"));
        let mut item = idta(ITEM_FOOTAGE_KIND, 11);
        item.extend(chunk(b"Utf8", b""));
        item.extend(list(b"Pin ", &pin));
        let bytes = file(&list(b"Fold", &list(b"Item", &item)));

        let item = &parse_capture(&bytes)
            .expect("the walk survives")
            .capture
            .items[0];
        assert_eq!(item.is_sequence, Some(true));
        assert_eq!(item.sequence_prefix.as_deref(), Some("Depth"));
        assert_eq!(item.sequence_suffix.as_deref(), Some("_depth.exr"));
        assert_eq!(item.path.as_deref(), Some(r"C:\Clips\Cine3\Depth"));

        // The same shape without either signal: one file, as before.
        let alias = br#"{"fullpath":"C:\\Clips\\Cine3\\World.avi"}"#;
        let mut pin = chunk(b"sspc", &[0; 222]);
        pin.extend(chunk(b"Utf8", b""));
        pin.extend(list(b"Als2", &chunk(b"alas", alias)));
        pin.extend(chunk(b"opti", b"AVIV"));
        let mut item = idta(ITEM_FOOTAGE_KIND, 12);
        item.extend(chunk(b"Utf8", b""));
        item.extend(list(b"Pin ", &pin));
        let bytes = file(&list(b"Fold", &list(b"Item", &item)));

        let item = &parse_capture(&bytes)
            .expect("the walk survives")
            .capture
            .items[0];
        assert_eq!(item.is_sequence, None);
        assert_eq!(item.sequence_prefix, None);
    }

    /// **A footage item somebody *did* rename keeps the name they gave it.**
    #[test]
    fn a_renamed_footage_item_keeps_its_own_name() {
        let alias = br#"{"fullpath":"/media/Depth.avi"}"#;
        let mut pin = chunk(b"sspc", &[0; 222]);
        pin.extend(list(b"Als2", &chunk(b"alas", alias)));
        let mut item = idta(ITEM_FOOTAGE_KIND, 11);
        item.extend(chunk(b"Utf8", b"the deep one"));
        item.extend(list(b"Pin ", &pin));
        let bytes = file(&list(b"Fold", &list(b"Item", &item)));

        let item = &parse_capture(&bytes)
            .expect("the walk survives")
            .capture
            .items[0];
        assert_eq!(item.name.as_deref(), Some("the deep one"));
        assert_eq!(item.path.as_deref(), Some("/media/Depth.avi"));
    }

    /// **A project with no item tree is refused, not half-imported.**
    ///
    /// The one structural failure that is worth failing on: a container that
    /// parses but holds no `LIST:Fold` has no project in it, and returning an
    /// empty capture would look to the user like an After Effects project that
    /// happened to be empty.
    #[test]
    fn a_container_with_no_item_tree_is_refused() {
        let bytes = file(&chunk(b"head", &[0; 20]));
        assert_eq!(parse_capture(&bytes).unwrap_err(), AepError::NoItemTree);
    }

    /// **A comp whose settings record is missing still imports, with a row
    /// saying so.**
    ///
    /// docs/11 §7's policy in one test: a parse failure on one chunk skips that
    /// chunk and keeps going, and the skip becomes a report row rather than
    /// vanishing. The comp arrives with its id and its name — which is worth
    /// far more to the user than a refusal — and the report names exactly what
    /// was lost.
    #[test]
    fn a_comp_missing_its_settings_still_arrives_and_says_what_was_lost() {
        let mut comp = idta(ITEM_COMP_KIND, 7);
        comp.extend(chunk(b"Utf8", b"Broken"));
        let bytes = file(&list(b"Fold", &list(b"Item", &comp)));

        let parsed = parse_capture(&bytes).expect("the walk survives");
        assert_eq!(parsed.capture.comps.len(), 1);
        assert_eq!(parsed.capture.comps[0].id, Some(7));
        assert_eq!(parsed.capture.items[0].name.as_deref(), Some("Broken"));
        assert!(
            parsed.capture.comps[0].width.is_none(),
            "nothing is invented in place of the record that was not there"
        );

        assert_eq!(parsed.skipped.len(), 1);
        let row = &parsed.skipped[0];
        assert_eq!(row.comp.as_deref(), Some("Broken"));
        assert_eq!(row.path.as_deref(), Some("cdta"));
        assert!(row
            .error
            .as_deref()
            .is_some_and(|why| why.contains("missing")));
    }

    /// **A layer record too short to read is skipped, and the stack keeps
    /// going.**
    ///
    /// The `ldta` record grew across After Effects versions, and a damaged one
    /// is shorter still. Reading by offset into whatever is there would import
    /// a layer whose timing, parent and switches are all garbage — a project
    /// that opens and is wrong. The skip is loud instead.
    #[test]
    fn a_layer_record_too_short_to_read_is_skipped_rather_than_guessed() {
        let mut comp = idta(ITEM_COMP_KIND, 3);
        comp.extend(chunk(b"Utf8", b"Comp"));
        comp.extend(chunk(b"cdta", &[0; 204]));
        comp.extend(list(b"Layr", &chunk(b"ldta", &[0; 40])));
        let bytes = file(&list(b"Fold", &list(b"Item", &comp)));

        let parsed = parse_capture(&bytes).expect("the walk survives");
        assert!(parsed.capture.comps[0].layers.is_empty());
        assert_eq!(parsed.skipped.len(), 1);
        assert_eq!(parsed.skipped[0].path.as_deref(), Some("ldta"));
    }

    /// **The same bytes parse to the same capture, byte for byte.**
    ///
    /// The determinism rule, checked on a synthetic file as well as on the
    /// golden one, so it holds for the awkward shapes too.
    #[test]
    fn parsing_is_deterministic() {
        let mut folder = idta(ITEM_FOLDER_KIND, 1);
        folder.extend(chunk(b"Utf8", b"Folder"));
        let mut comp = idta(ITEM_COMP_KIND, 2);
        comp.extend(chunk(b"Utf8", b"Inside"));
        comp.extend(chunk(b"cdta", &[0; 204]));
        folder.extend(list(b"Sfdr", &list(b"Item", &comp)));
        let bytes = file(&list(b"Fold", &list(b"Item", &folder)));

        let once = parse_capture(&bytes).unwrap();
        let twice = parse_capture(&bytes).unwrap();
        assert_eq!(once.capture, twice.capture);
        assert_eq!(once.capture.items.len(), 2);
        assert_eq!(once.capture.items[1].parent_id, Some(1));
    }

    const ITEM_FOLDER_KIND: u16 = enums::ITEM_FOLDER;
    const ITEM_FOOTAGE_KIND: u16 = enums::ITEM_FOOTAGE;
    const ITEM_COMP_KIND: u16 = enums::ITEM_COMP;
}
