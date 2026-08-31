//! docs/13 §1's reference composition, built in code.
//!
//! # In plain terms
//!
//! Every budget in docs/13 §2 is stated "against the reference comp", so the
//! comp has to be one exact thing rather than a description. §1 describes it in
//! a paragraph; this module is that paragraph turned into a [`Document`], layer
//! by layer, through the same model the application edits. Nothing here is a
//! benchmark fixture in disguise — it is an ordinary project that happens to be
//! written down in Rust instead of saved to a `.lum` file.
//!
//! §1, and where each phrase lands:
//!
//! | §1 says | built by |
//! |---|---|
//! | 1080p60, 20 s | [`reference_comp`]'s `Composition` |
//! | two 1080p60 H.264 footage layers | the `Hero` and `Ramp` layers |
//! | one with a Retime ramp to 40% using flow interpolation | `Ramp`'s `retime` + `interpolation` |
//! | one text layer | `Title` |
//! | one Sequence layer with four clips | `Edit` |
//! | one adjustment layer carrying a grade (3D LUT + curves) | `Grade` |
//! | a glow on one footage layer | `Hero`'s effect stack |
//! | motion blur enabled on two layers | the comp master plus `Hero`/`Ramp`'s switches |
//! | one luma matte | `Hero`'s matte, keyed off `Title` |
//! | audio layer with volume keyframes | `Music` |
//!
//! One substitution, recorded rather than hidden: §1's "curves" half of the
//! grade is built as **Colour balance**, which is lift/gamma/gain — a
//! per-channel tone curve under a different name. Lumit has no effect called
//! Curves. If one lands, the grade should gain it and this note should go.

use std::path::Path;

use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::model::{
    Composition, Document, EffectInstance, EffectValue, FileParam, FootageItem, Layer, LayerKind,
    LinearColour, MatteChannel, MatteRef, MediaRef, MotionBlur, ProjectItem, Switches,
    TextDocument, TransformGroup,
};
use lumit_core::retime::{Ease, FlowParams, Interpolation, Retime};
use lumit_core::sequence::{Clip, ClipSource};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use uuid::Uuid;

use crate::media::{self, RefMedia};

/// The comp's length in seconds — §1's 20 s, and the work area B11 fills.
pub const DURATION_S: i64 = 20;

/// How many layers §1 asks for. Five picture layers plus the audio layer.
pub const LAYER_COUNT: usize = 6;

/// Generate the media into `media_dir` (idempotent — see [`crate::media`]) and
/// build docs/13 §1's reference comp over it. Returns the document and the id
/// of the comp to render.
///
/// `Err` when the media cannot be made (no ffmpeg, unwritable directory) or a
/// built-in effect the grade needs has gone from the catalogue.
pub fn reference_comp(media_dir: &Path) -> Result<(Document, Uuid), String> {
    build(&media::generate(media_dir)?)
}

/// [`reference_comp`] over media already generated — for a harness that makes
/// the clips once and then builds the comp per scenario.
pub fn build(media: &RefMedia) -> Result<(Document, Uuid), String> {
    let mut doc = Document::new();
    let span = secs(DURATION_S);

    let clip_a = add_footage(&mut doc, "ref_a.mp4", &media.clip_a);
    let clip_b = add_footage(&mut doc, "ref_b.mp4", &media.clip_b);
    let tone = add_footage(&mut doc, "ref_tone.wav", &media.audio);

    // §1's text layer. Also the luma matte's source — one layer doing two jobs
    // is how a real title-through-footage shot is built, and it saves inventing
    // a seventh layer §1 does not ask for.
    let title = layer(
        "Title",
        LayerKind::Text {
            document: TextDocument {
                text: "Lumit reference comp".into(),
                expression: None,
                size: 96.0,
                fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                path: None,
                path_offset: lumit_core::anim::Property::zero(),
                animators: Vec::new(),
                extra: serde_json::Map::new(),
            },
        },
        span,
    );

    // §1: "a glow on one footage layer", "one luma matte", and one of the two
    // motion-blurred layers.
    let mut hero = layer("Hero", LayerKind::Footage { item: clip_a }, span);
    hero.effects = vec![effect("glow")?];
    hero.matte = Some(MatteRef {
        layer: title.id,
        channel: MatteChannel::Luma,
        inverted: false,
        source: Default::default(),
    });
    hero.switches.motion_blur = true;

    // §1: "one with a Retime ramp to 40% using flow interpolation". The ramp is
    // built by the engine's own retime store and handed over as the keyframes
    // it evaluates to, so the curve here is the curve the timeline would draw.
    let mut ramp = layer("Ramp", LayerKind::Footage { item: clip_b }, span);
    ramp.retime = Some(Property {
        animation: Animation::Keyframed(
            Retime::single_ramp(span, Rational::ZERO, Rational::ONE, rat(2, 5), Ease::Linear)
                .source_keyframes(),
        ),
        extra: serde_json::Map::new(),
    });
    ramp.interpolation = Interpolation::Flow(FlowParams::default());
    ramp.switches.motion_blur = true;
    // Not fully opaque, so the Sequence layer beneath it is actually composited
    // into the picture rather than covered by it. Full-frame footage over
    // full-frame footage is realistic, but a layer whose pixels can never reach
    // the frame is one a regression could drop without changing anything a test
    // can see.
    ramp.transform.opacity = Property::fixed(70.0);

    // §1: "one Sequence layer with four clips" — four five-second cuts,
    // alternating source and stepping through each clip so no two show the same
    // pictures.
    let quarter = secs(DURATION_S / 4);
    let clips = (0..4)
        .map(|i| {
            let source = if i % 2 == 0 { clip_a } else { clip_b };
            let start = secs(i * DURATION_S / 4);
            Clip::new(
                ClipSource::Footage(source),
                start,
                start.checked_add(quarter).unwrap_or(quarter),
                start,
                quarter,
            )
        })
        .collect();
    let edit = layer("Edit", LayerKind::Sequence { clips }, span);

    // §1: "one adjustment layer carrying a grade (3D LUT + curves)". Topmost,
    // so it grades everything beneath it.
    let mut lut = effect("lut")?;
    set_param(
        &mut lut,
        "file",
        EffectValue::File(FileParam::single(media.lut.to_string_lossy().into_owned())),
    )?;
    let mut grade = layer("Grade", LayerKind::Adjustment, span);
    grade.effects = vec![lut, curves()?];

    // §1: "audio layer with volume keyframes". It draws nothing — the switch
    // says so — and is heard rather than seen.
    let mut music = layer("Music", LayerKind::Footage { item: tone }, span);
    music.switches.visible = false;
    music.volume_db = fade_keys();

    let comp_id = id("Reference");
    doc.items.push(ProjectItem::Composition(Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: comp_id,
        name: "Reference".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).map_err(|e| format!("reference comp rate: {e}"))?,
        duration: Duration(span),
        background: LinearColour::BLACK,
        // B11 fills "the 20 s work area" — so the comp states one.
        work_area: Some((CompTime(Rational::ZERO), CompTime(span))),
        // The comp master §1's "motion blur enabled on two layers" needs: with
        // it off, no per-layer switch does anything.
        motion_blur: MotionBlur {
            enabled: true,
            ..MotionBlur::default()
        },
        // Index 0 is the top of the stack.
        layers: vec![grade, title, hero, ramp, edit, music],
        markers: Vec::new(),
        extra: serde_json::Map::new(),
    }));

    Ok((doc, comp_id))
}

/// The id of the one thing called `name`, hashed from the name itself.
///
/// **Not** `Uuid::now_v7`, which is the clock: every id in a comp feeds the
/// content hash a finished frame is filed under, so ids from the clock would
/// give the same picture a different name on every run — the disk tier would
/// never hit across runs, and two measurements would not be of the same
/// composition. FNV-1a over the name is stable for as long as the names are,
/// and the names are what §1 pins.
fn id(name: &str) -> Uuid {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut h = OFFSET;
    for b in name.as_bytes() {
        h = (h ^ u128::from(*b)).wrapping_mul(PRIME);
    }
    Uuid::from_u128(h)
}

/// A rational, falling back to zero rather than panicking. Every call site
/// passes a literal non-zero denominator, so the fallback is unreachable — it
/// exists because engine crates do not unwrap (docs/14 §4).
fn rat(num: i64, den: i64) -> Rational {
    Rational::new(num, den).unwrap_or(Rational::ZERO)
}

/// `n` whole seconds.
fn secs(n: i64) -> Rational {
    rat(n, 1)
}

/// Register a footage item and return its id.
fn add_footage(doc: &mut Document, name: &str, path: &Path) -> Uuid {
    let item = id(name);
    doc.items.push(ProjectItem::Footage(FootageItem {
        id: item,
        name: name.into(),
        media: MediaRef {
            relative_path: name.into(),
            absolute_path: path.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        sequence: None,
        extra: serde_json::Map::new(),
        colour_space: None,
    }));
    item
}

/// A layer of `kind` running from the comp's start for `span`, everything else
/// left at its default.
fn layer(name: &str, kind: LayerKind, span: Rational) -> Layer {
    Layer {
        id: id(name),
        name: name.into(),
        kind,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(span),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        markers: Vec::new(),
        volume_db: Property::zero(),
        pan: Property::zero(),
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Interpolation::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
        graph: Default::default(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

/// A built-in effect instance at its declared defaults.
fn effect(match_name: &str) -> Result<EffectInstance, String> {
    lumit_core::fx::instantiate(match_name)
        .ok_or_else(|| format!("reference comp: no built-in effect `{match_name}`"))
}

/// §1's "curves": a per-channel tone curve, which in this catalogue is Colour
/// balance's lift/gamma/gain. Set off neutral, or the grade would be a no-op
/// that costs a pass and changes nothing.
fn curves() -> Result<EffectInstance, String> {
    let mut cb = effect("colour_balance")?;
    set_param(&mut cb, "lift", colour([0.02, 0.01, 0.04, 1.0]))?;
    set_param(&mut cb, "gamma", colour([0.95, 1.0, 1.05, 1.0]))?;
    set_param(&mut cb, "gain", colour([1.08, 1.0, 0.94, 1.0]))?;
    Ok(cb)
}

/// A static colour parameter value.
fn colour(rgba: [f64; 4]) -> EffectValue {
    EffectValue::Colour(rgba.map(Property::fixed))
}

/// Overwrite a parameter the instance declares. An unknown id is an error
/// rather than a new parameter: an effect that renames a control would
/// otherwise leave this comp carrying a setting nothing reads, and the grade
/// would quietly become a no-op that still costs a pass — a benchmark
/// measuring the wrong composition and saying nothing about it.
fn set_param(inst: &mut EffectInstance, id: &str, value: EffectValue) -> Result<(), String> {
    match inst.params.iter_mut().find(|p| p.id == id) {
        Some(p) => {
            p.value = value;
            Ok(())
        }
        None => Err(format!(
            "reference comp: effect `{}` has no parameter `{id}`",
            inst.effect.match_name
        )),
    }
}

/// The audio layer's volume keyframes: silence, up over two seconds, held, and
/// back down over the last two. −60 dB reads as silence without tripping the
/// −100 dB "exact silence" floor, so every key is a real interpolation.
fn fade_keys() -> Property {
    let key = |t: i64, db: f64| Keyframe {
        time: secs(t),
        value: db,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    Property {
        animation: Animation::Keyframed(vec![
            key(0, -60.0),
            key(2, 0.0),
            key(DURATION_S - 2, 0.0),
            key(DURATION_S, -60.0),
        ]),
        extra: serde_json::Map::new(),
    }
}
