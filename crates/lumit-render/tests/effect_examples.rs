//! The manual's effect example pictures, rendered by the engine itself.
//!
//! # In plain terms
//!
//! Every effect in the manual gets a picture showing what it does. Drawing
//! eighty-five of those by hand would guarantee that half of them were wrong
//! within a month, so the engine renders them: one composition, one layer of
//! footage, one effect on it, one frame out. The same walk the Viewer uses, so
//! a picture on the website is a picture the application would actually make.
//!
//! Output is raw RGBA8 (`<slug>.<w>x<h>.raw`), for the reason `blur_proof.rs`
//! gives: nothing in the workspace encodes PNG or WebP, and a throwaway encoder
//! written for a diagnostic is exactly the code that should not exist. The
//! runner (`web-docs/scripts/gen-effect-shots.mjs`) makes the source clip with
//! ffmpeg, calls this, and encodes the results with sharp.
//!
//! Ignored by default — it wants a GPU, a clip on disk, and it writes files.
//! Run it through the runner rather than by hand:
//!
//! ```text
//! cd web-docs && npm run docs:effect-shots
//! ```
//!
//! Directly, if you must:
//!
//! ```text
//! LUMIT_FX_EXAMPLES_CLIP="C:/tmp/lumit-fx/plate.mp4" \
//! LUMIT_FX_EXAMPLES_OUT="C:/tmp/lumit-fx/out" \
//!   cargo test -p lumit-render --release --test effect_examples -- --ignored --nocapture
//! ```
//!
//! `LUMIT_FX_EXAMPLES_ONLY=<match name>` (for example `accumulation_mb`) renders
//! just that effect, for iterating on one picture.
//!
//! ## The neutral report
//!
//! Many effects do nothing at their declared defaults, which is correct — a
//! fresh Curves is a straight line and a fresh Exposure is 0 stops. A picture
//! of one of those is a picture of the plate. So the run compares every render
//! against the untouched plate and prints the ones that came back identical.
//! Anything on that list needs an entry in [`showcase`], and the run fails if
//! the list is not empty, because a silently neutral example is worse than a
//! missing one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;

use lumit_core::anim::Property;
use lumit_core::mask::{BezierPath, Mask, MaskMode, Vertex};
use lumit_core::model::{
    Composition, Document, EffectInstance, EffectValue, FileParam, FootageItem, Layer, LayerKind,
    LinearColour, MediaRef, MotionBlur, ProjectItem, Switches, TransformGroup,
};
use lumit_core::retime::Interpolation;
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use uuid::Uuid;

/// The example frame's size: the source plate's own, and close enough to 1080p
/// that every effect whose default puts a point at 960, 540 lands somewhere
/// sensible in the picture. The runner encodes the results down to figure width,
/// so the pages do not carry 1920-pixel files.
const W: u32 = 1920;
const H: u32 = 816;

/// Which frame of the clip. One second in, so a temporal effect has frames
/// behind it to look at and the pan has built up some motion.
const FRAME: u64 = 24;

/// Comp length in seconds, and its rate.
const DURATION_S: i64 = 2;
const FPS: u32 = 24;

// ---------------------------------------------------------------------------
// The showcase settings
// ---------------------------------------------------------------------------

/// Parameter settings that make an effect visible in one still picture.
///
/// The declared defaults are the right defaults for the application: a fresh
/// grade should change nothing until you move a slider. They are the wrong
/// settings for a figure in a manual. Each entry here says what the picture
/// shows, and only effects whose defaults are neutral (or too subtle to read at
/// figure size) need one.
///
/// Values are deliberately on the strong side. A reader looking at a thumbnail
/// should be able to name the effect from the picture alone.
fn showcase(match_name: &str) -> Vec<(&'static str, EffectValue)> {
    match match_name {
        // --- grades, all of which are identity until a slider moves ---
        "exposure" => vec![("stops", f(1.4))],
        "brightness" => vec![("brightness", f(14.0)), ("contrast", f(45.0))],
        "contrast" => vec![("contrast", f(138.0))],
        "gamma" => vec![("gamma", f(0.52))],
        "saturation" => vec![("saturation", f(340.0))],
        "vibrancy" => vec![("amount", f(200.0))],
        "temperature" => vec![("temperature", f(95.0))],
        "hue_shift" => vec![("angle", f(120.0))],
        "colour_balance" => vec![
            ("lift", colour([0.02, 0.01, 0.07, 1.0])),
            ("gamma", colour([0.94, 1.0, 1.10, 1.0])),
            ("gain", colour([1.18, 1.0, 0.82, 1.0])),
        ],
        // The classic S: shadows down, highlights up, the middle left alone. The
        // master curve is a drawn curve now (K-412), so the S is four points.
        "curves" => vec![(
            "master",
            EffectValue::Curve(vec![[0.0, 0.0], [0.25, 0.17], [0.75, 0.86], [1.0, 1.0]]),
        )],
        "levels" => vec![
            ("master_in_black", f(0.05)),
            ("master_in_white", f(0.72)),
            ("master_gamma", f(0.9)),
        ],
        "hue_saturation" => vec![
            ("master_saturation", f(45.0)),
            ("blues_hue", f(45.0)),
            ("reds_saturation", f(70.0)),
        ],
        // Tint at its defaults maps black to black and white to white, which is
        // a greyscale conversion and looks like a mistake in a manual. Pulling
        // the black end to navy makes it read as what it is: a two-colour map.
        "tint" => vec![("black", colour([0.02, 0.05, 0.18, 1.0]))],

        // --- effects the plate is too polite to show at anything gentle ---
        // Sharpening, grain and a vignette are all real at a couple of per cent
        // and invisible in a figure, so each is set well past where a shot would
        // want it. The caption on every page says as much.
        "sharpen_simple" => vec![("amount", f(4.0)), ("radius", f(3.0))],
        "sharpen" => vec![
            ("amount", f(250.0)),
            ("radius", f(33.0)),
            ("threshold", f(0.0)),
        ],
        // A vignette darkens the corners, and only the corners. Roundness has
        // to come off its default for that: at 1 the falloff is a true circle,
        // and on a 2.35:1 frame a circle that clears the corners has already
        // swallowed both side edges, whatever the radius is set to. At 0 it
        // follows the frame's own shape and the four corners are what is left.
        "vignette" => vec![
            ("amount", f(0.55)),
            ("radius", f(0.9)),
            ("softness", f(0.45)),
            ("roundness", f(0.0)),
        ],
        "add_grain" => vec![
            ("intensity", f(190.0)),
            ("size", f(4.0)),
            ("softness", f(20.0)),
        ],
        "glow" => vec![
            ("threshold", f(0.18)),
            ("radius", f(110.0)),
            ("intensity", f(7.0)),
        ],
        "median" => vec![("radius", f(3.0))],
        // Radial blur's centre is px@comp since K-558, and the schema default
        // is the nominal 1080p middle. This plate is 1920x816, so the picture
        // asks for *its* middle — which is what `instantiate_for_raster` would
        // have written had the effect been dropped on this comp.
        "radial_blur" => vec![
            ("centre_x", f(f64::from(W) * 0.5)),
            ("centre_y", f(f64::from(H) * 0.5)),
        ],
        // A four-pixel period is right on a screen and gone by the time the
        // figure has been scaled to page width, so the example uses a coarse one.
        "scanlines" => vec![("intensity", f(0.9)), ("scanline_period", f(12.0))],
        "chromatic_aberration" => vec![("amount", f(18.0))],
        "photo_filter" => vec![("density", f(90.0))],
        "shadow_highlight" => vec![
            ("shadow_amount", f(80.0)),
            ("highlight_amount", f(70.0)),
            ("radius", f(125.0)),
            ("midtone_contrast", f(25.0)),
        ],
        "texturize" => vec![("relief", f(12.0)), ("texture_contrast", f(180.0))],
        // A frozen frame of a shake is an offset frame, which teaches nothing
        // about shaking, so a little blur goes with it. Only a little: smeared
        // hard enough and the picture reads as a blur rather than a shake, and
        // the reader has to be able to see the frame is knocked off its mark.
        "shake" => vec![
            ("amplitude", f(85.0)),
            ("rotation", f(1.5)),
            ("motion_blur", on(true)),
            ("mb_amount", f(0.3)),
        ],

        // --- geometry ---
        "transform" => vec![
            ("scale_x", f(72.0)),
            ("scale_y", f(72.0)),
            ("rotation", f(11.0)),
            ("position_x", f(90.0)),
        ],
        "offset" => vec![("shift_x", f(420.0)), ("shift_y", f(130.0))],
        // A tile the size of the frame repeated once over the frame is the
        // frame, which is why the declared defaults are neutral. A quarter of
        // the width, mirrored, is a picture of what tiling is: four across,
        // each one flipped against its neighbour so the seams disappear.
        "tile" => vec![
            ("tile_width", f(480.0)),
            ("tile_height", f(204.0)),
            ("mirror_edges", on(true)),
        ],
        "displacement_map" => vec![
            ("horizontal_amount", f(320.0)),
            ("vertical_amount", f(220.0)),
        ],

        // --- the ones that read a second picture, wired in wire_aux ---
        "light_wrap" => vec![("width", f(70.0)), ("intensity", f(2.2))],
        "set_matte" => vec![("channel", choice(0))],
        // Focus is picked by pointing at the running figure rather than by
        // guessing a number, which is the honest way round and the way the
        // control is meant to be used.
        //
        // Invert is deliberately left off. Focus is read through the same
        // invert as every other pixel, so with Focus point on the two cancel
        // and the picture does not move at all. The picture was reading the
        // wrong way round because the depth pass is crushed into its top end:
        // at a wide Range the whole near ground plane sat inside focus and only
        // a mid band softened. A narrow Range with Gamma steepening the falloff
        // is what separates the figure from what is in front of and behind it.
        "dof" => vec![
            ("use_focus_point", on(true)),
            ("focus_point_x", f(580.0)),
            ("focus_point_y", f(330.0)),
            ("gamma", f(6.0)),
            ("range", f(0.02)),
            ("aperture", f(12.0)),
            ("near_aperture", f(12.0)),
            ("far_aperture", f(12.0)),
        ],

        // --- the iris is the transition here, so it has to be opened ---
        // Radial wipe at its half-way default cuts the frame down the middle,
        // which is the Linear wipe picture mirrored. A third of the way round
        // leaves a wedge, so the two pages do not show the same figure twice.
        "radial_wipe" => vec![("completion", f(35.0))],
        "iris_wipe" => vec![
            ("centre_y", f(f64::from(H) / 2.0)),
            ("outer_radius", f(540.0)),
            ("feather", f(30.0)),
        ],

        // --- temporal ---
        // Fast motion blur smears motion the footage already contains, so the
        // example plays the plate faster (see wants_speed_up). The shutter stays
        // where it opens by default: doubling the speed and opening the shutter
        // as well turned the picture into porridge.
        "motion_blur" => vec![("samples", f(24.0))],
        // Thirty-two samples, because the whole claim of this effect is that
        // each one is a real render: too few and the picture is a stack of
        // copies rather than a blur.
        "accumulation_mb" => vec![("force_all", on(true)), ("samples", f(30.0))],

        // The plate is a game render and already broadcast legal, so the
        // default limit finds nothing to pull back. A limit well under the
        // picture is what makes the effect's job visible.
        "broadcast_safe" => vec![("maximum_signal", f(72.0))],

        // --- Flash's Manual mode reads the trigger as the level ---
        "flash" => vec![("trigger", f(0.55)), ("intensity", f(70.0))],
        // Particulate's defaults are a working emitter, not a neutral one, so
        // this is not a picture that would otherwise be the plate — it is one
        // that would otherwise be unreadable. A second of a hundred and fifty
        // four-pixel particles from a four-hundred-pixel emitter is a faint
        // sprinkle at figure size. Bigger particles, more of them, and a wider
        // mouth make a picture a reader can name.
        "particulate" => vec![
            ("emit_rate", f(900.0)),
            ("size", f(14.0)),
            ("width", f(900.0)),
            ("height", f(500.0)),
            ("initial_speed", f(220.0)),
        ],
        // The physical flare, driven hard and pointed at a bright part of the
        // plate: the ghost train only reads at figure size if the light is
        // strong and off to one side, and an area source gives the ghosts a
        // shape rather than a row of dots.
        "lens_flare" => vec![
            ("source_type", choice(0)),
            ("light_x", f(1420.0)),
            ("light_y", f(240.0)),
            ("intensity", f(3.5)),
            ("source_width", f(120.0)),
            ("source_height", f(120.0)),
        ],

        _ => Vec::new(),
    }
}

/// A still float parameter.
fn f(v: f64) -> EffectValue {
    EffectValue::Float(Property::fixed(v))
}

/// A still point parameter, in pixels at composition size.
fn pt(x: f64, y: f64) -> EffectValue {
    EffectValue::Point(Property::fixed(x), Property::fixed(y))
}

/// A still colour parameter, scene-linear RGBA.
fn colour(rgba: [f64; 4]) -> EffectValue {
    EffectValue::Colour(rgba.map(Property::fixed))
}

/// A dropdown, by option index.
fn choice(i: u32) -> EffectValue {
    EffectValue::Choice(i)
}

/// A checkbox.
fn on(b: bool) -> EffectValue {
    EffectValue::Bool(b)
}

// ---------------------------------------------------------------------------
// The composition
// ---------------------------------------------------------------------------

/// A stable id hashed from a name, so two runs of the same picture are filed
/// under the same frame name and the disk cache can hit across runs. Copied
/// from `lumit_bench::comp` for the same reason it exists there: ids from the
/// clock would rename every frame on every run.
fn id(name: &str) -> Uuid {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut h = OFFSET;
    for b in name.as_bytes() {
        h = (h ^ u128::from(*b)).wrapping_mul(PRIME);
    }
    Uuid::from_u128(h)
}

fn rat(num: i64, den: i64) -> Rational {
    Rational::new(num, den).unwrap_or(Rational::ZERO)
}

fn layer(name: &str, kind: LayerKind, span: Rational) -> Layer {
    Layer {
        graph: Default::default(),
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
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

/// An oval covering most of the frame, for the three effects that draw along a
/// mask path (Vegas, Scribble, Stroke). Four vertices with the usual circular
/// tangent length, which is close enough to an ellipse for a figure.
fn oval_mask(mode: MaskMode) -> Mask {
    let (cx, cy) = (f64::from(W) / 2.0, f64::from(H) / 2.0);
    let (rx, ry) = (f64::from(W) * 0.3, f64::from(H) * 0.34);
    // The magic number that turns four cubic segments into a circle.
    let k = 0.552_284_75;
    let (hx, hy) = (rx * k, ry * k);
    let v = |pos: (f64, f64), tin: (f64, f64), tout: (f64, f64)| Vertex {
        pos,
        tan_in: tin,
        tan_out: tout,
    };
    Mask {
        id: id("Oval"),
        name: "Oval".into(),
        path: BezierPath {
            vertices: vec![
                v((cx, cy - ry), (-hx, 0.0), (hx, 0.0)),
                v((cx + rx, cy), (0.0, -hy), (0.0, hy)),
                v((cx, cy + ry), (hx, 0.0), (-hx, 0.0)),
                v((cx - rx, cy), (0.0, hy), (0.0, -hy)),
            ],
            closed: true,
        },
        path_keys: Vec::new(),
        inverted: false,
        opacity: Property::fixed(100.0),
        mode,
        feather: Property::zero(),
        vertex_feather: Vec::new(),
        expansion: Property::zero(),
        extra: serde_json::Map::new(),
    }
}

/// Effects that need an alpha edge before they show anything. A shadow falls
/// from an outline, a light wrap hugs one, and roughening the edges of a frame
/// that reaches every border roughens nothing. Fill is here for a different
/// reason: it keeps a layer's shape and replaces its colour, so on a full-frame
/// plate it produces one flat rectangle and teaches nobody anything. These get
/// the oval as a real, gating mask; everything else that wants the oval wants
/// its geometry only.
fn wants_alpha_edge(match_name: &str) -> bool {
    matches!(
        match_name,
        "light_wrap" | "drop_shadow" | "roughen_edges" | "fill"
    )
}

/// Accumulation motion blur re-renders **the scene beneath it**, so a copy
/// sitting on the plate's own stack has nothing to sample and renders the plate
/// back unchanged. Its home is an adjustment layer, which is where the example
/// puts it.
fn wants_scene_below(match_name: &str) -> bool {
    match_name == "accumulation_mb"
}

/// A whip across the frame, for the effect that samples the scene beneath it.
///
/// The motion is the layer's own transform rather than the footage's, and that
/// is what accumulation motion blur can actually see: its sub-frame samples
/// re-render the stack beneath with the footage **held** at the frame-time
/// decode (docs/impl/temporal-rerender.md §2), so a retime of the plate gives
/// every sample the same pixels and the average is the plate back, unblurred.
/// Only transforms, effects and the camera move between samples, so the example
/// moves the transform. (An earlier note here blamed a black frame on the
/// retimed path; `a_retimed_layer_under_forced_accumulation_mb_is_not_black` in
/// `headless.rs` pins that it renders, just without any smear.)
///
/// The layer crosses the frame, and the sampled moment is the one where it sits
/// dead centre, so the framing matches every other picture in the manual and the
/// page's before-and-after wipe has nothing to jump across. Scale sits two per
/// cent over so the extreme samples inside the shutter still have picture to show
/// at the frame edge instead of a sliver of background. The span matches the
/// 2.4x the other motion blur page plays at, near enough: 1800 pixels over two
/// seconds is a little over a frame width a second.
fn animate_whip(plate: &mut Layer, span: Rational) {
    let key = |t: Rational, v: f64| lumit_core::anim::Keyframe {
        time: t,
        value: v,
        interp_in: lumit_core::anim::SideInterp::Linear,
        interp_out: lumit_core::anim::SideInterp::Linear,
    };
    plate.transform.scale_x = Property::fixed(102.0);
    plate.transform.scale_y = Property::fixed(102.0);
    plate.transform.position_x = Property {
        animation: lumit_core::anim::Animation::Keyframed(vec![
            key(Rational::ZERO, -900.0),
            key(span, 900.0),
        ]),
        extra: serde_json::Map::new(),
    };
}

/// Fast motion blur reads motion out of the footage itself, so its example plays
/// the plate faster through the frame it is sampled at. (Accumulation motion blur
/// cannot read a retime, see `animate_whip`, so that page moves the transform at
/// a matching rate instead.)
fn wants_speed_up(match_name: &str) -> bool {
    match_name == "motion_blur"
}

/// Play the whole clip through 2 / 2.4 seconds of composition time centred on the
/// sampled frame (keys at 1 s ∓ 5/12 s): 2.4x, with the midpoint exactly on the frame every
/// other picture uses. Written as two plain keyframes because a rate ramp has to
/// be integrated to know where it lands, and landing on a different frame from
/// the rest of the manual is the one thing this example must not do.
fn animate_speed_up(plate: &mut Layer) {
    let key = |t: Rational, v: f64| lumit_core::anim::Keyframe {
        time: t,
        value: v,
        interp_in: lumit_core::anim::SideInterp::Linear,
        interp_out: lumit_core::anim::SideInterp::Linear,
    };
    plate.retime = Some(Property {
        animation: lumit_core::anim::Animation::Keyframed(vec![
            key(rat(7, 12), 0.0),
            key(rat(17, 12), DURATION_S as f64),
        ]),
        extra: serde_json::Map::new(),
    });
}

/// Effects the example frame cannot honestly illustrate. Posterize time holds
/// one frame for several, which is a change to the clock and shows only in
/// motion. Matte key wants a screen to pull, and this frame has none, so every
/// setting of it either does nothing or keys something arbitrary. Both are left
/// without a picture; the manual's generator omits the figure for any effect
/// with no file on disk, so nothing links to an image that is not there.
fn unillustrable(match_name: &str) -> Option<&'static str> {
    match match_name {
        "posterize_time" => Some("holds frames, which only shows in motion"),
        "matte_key" => Some("the example frame has no screen in it to key"),
        // Camera track and Planar track hold a job, and the Controls hold values
        // for expressions to read; none of them draws, so a picture would be the
        // plate twice.
        "camera_track" | "planar_track" | "slider_control" | "angle_control"
        | "checkbox_control" | "colour_control" | "point_control" => {
            Some("draws nothing by design")
        }
        // The drivers. A driver answers with a *number*, a colour or a bag of
        // points for something else to use, and reaches whatever it drives
        // through a wire in the node graph. There is no picture of one.
        "wiggle" | "smooth" | "math" | "remap" | "audio_level" | "colour_cycle"
        | "points_sample" | "layer_points" => {
            Some("a driver: it answers with a value, not a picture")
        }
        // The points effects that *consume* a stream. Their points arrive on a
        // wire-only input (points-stream.md §4.1), which exists in the node
        // graph and nowhere else, and this harness stages one effect on one
        // layer with no graph behind it. With no stream in they draw nothing,
        // and a showcase entry cannot supply one — only a wire can.
        "clone_to_points" | "trail" | "connect_points" => {
            Some("draws what a wired points stream gives it, and there is no graph here")
        }
        // **Not a nature — a defect.** The physical flare adds nothing to a
        // headless render: the frame comes back bit-identical to the plate at
        // the defaults and still bit-identical with Intensity at 3.5 and an
        // area light placed on the brightest part of the picture. Its showcase
        // entry below is written and correct; it is skipped here so that the
        // run does not stop, and so that the manual does not carry a picture of
        // the plate with "Lens flare" under it. Take this line out the day the
        // flare's additive pass reaches this renderer.
        "lens_flare" => Some("adds nothing to a headless render (defect, not by design)"),
        _ => None,
    }
}

/// The auxiliary layer some effects read as a second picture.
///
/// With a depth clip supplied it is the plate's own depth pass, frame for frame,
/// which is what makes the Depth of field picture a real one: the blur follows
/// the scene instead of a ramp somebody invented. Without one it falls back to a
/// soft left-to-right gradient, so the harness still runs on a machine that has
/// only the plate.
///
/// Hidden, because a depth pass has no business appearing in the composition.
fn aux_layer(depth: Option<&std::path::Path>, span: Rational) -> Option<Layer> {
    let mut l = match depth {
        Some(_) => layer(
            "Aux",
            LayerKind::Footage {
                item: id("depth.mp4"),
            },
            span,
        ),
        None => layer(
            "Aux",
            LayerKind::Solid {
                def: id("Aux solid"),
            },
            span,
        ),
    };
    if depth.is_none() {
        let mut grad = lumit_core::fx::instantiate("gradient")?;
        set(&mut grad, "start", pt(0.0, f64::from(H) / 2.0));
        set(&mut grad, "end", pt(f64::from(W), f64::from(H) / 2.0));
        l.effects = vec![grad];
    }
    l.switches.visible = false;
    Some(l)
}

/// Overwrite a parameter if the instance declares it. Unknown ids are ignored
/// rather than fatal: this is documentation tooling, and an effect that renames
/// a control should cost a duller picture, never a red build.
fn set(inst: &mut EffectInstance, id: &str, value: EffectValue) {
    if let Some(p) = inst.params.iter_mut().find(|p| p.id == id) {
        p.value = value;
    }
}

/// The example composition: the plate, optionally one effect on it, and the
/// auxiliary layer for the effects that read a second picture.
fn example_doc(
    clip: &std::path::Path,
    depth: Option<&std::path::Path>,
    fx: Option<EffectInstance>,
) -> (Document, Uuid) {
    let mut doc = Document::new();
    let span = rat(DURATION_S, 1);

    doc.items
        .push(ProjectItem::Solid(lumit_core::model::SolidDef {
            id: id("Aux solid"),
            name: "Aux solid".into(),
            colour: LinearColour([0.0, 0.0, 0.0, 1.0]),
            width: W,
            height: H,
            extra: serde_json::Map::new(),
        }));

    let mut footage = |name: &str, path: &std::path::Path| {
        let item = id(name);
        doc.items.push(ProjectItem::Footage(FootageItem {
            sequence: None,
            id: item,
            name: name.into(),
            media: MediaRef {
                relative_path: name.into(),
                absolute_path: path.to_string_lossy().into_owned(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
            colour_space: None,
        }));
        item
    };
    let item = footage("plate.mp4", clip);
    if let Some(d) = depth {
        footage("depth.mp4", d);
    }

    let mut plate = layer("Plate", LayerKind::Footage { item }, span);
    if let Some(f) = fx.as_ref() {
        let mode = if wants_alpha_edge(&f.effect.match_name) {
            Some(MaskMode::Add)
        } else if f
            .params
            .iter()
            .any(|p| matches!(p.value, EffectValue::MaskPath(_)))
        {
            Some(MaskMode::None)
        } else {
            None
        };
        if let Some(mode) = mode {
            plate.masks = vec![oval_mask(mode)];
        }
        if wants_scene_below(&f.effect.match_name) {
            animate_whip(&mut plate, span);
        }
        if wants_speed_up(&f.effect.match_name) {
            animate_speed_up(&mut plate);
        }
    }

    // The effect goes on the plate, unless it is one that reads what is under
    // it, in which case an adjustment layer carries it and the plate is the
    // scene it reads.
    let mut adjustment = None;
    if let Some(f) = fx {
        if wants_scene_below(&f.effect.match_name) {
            let mut a = layer("Adjust", LayerKind::Adjustment, span);
            a.effects = vec![f];
            adjustment = Some(a);
        } else {
            plate.effects = vec![f];
        }
    }

    let mut layers = Vec::new();
    layers.extend(adjustment);
    layers.push(plate);
    if let Some(aux) = aux_layer(depth, span) {
        layers.push(aux);
    }

    let comp_id = id("Example");
    doc.items.push(ProjectItem::Composition(Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: comp_id,
        name: "Example".into(),
        width: W,
        height: H,
        frame_rate: FrameRate::new(FPS, 1).expect("24 fps"),
        duration: Duration(span),
        background: LinearColour::BLACK,
        work_area: None,
        motion_blur: MotionBlur {
            enabled: true,
            ..MotionBlur::default()
        },
        layers,
        markers: Vec::new(),
        extra: serde_json::Map::new(),
    }));

    (doc, comp_id)
}

/// Point an effect's auxiliary layer reference at the Aux layer, where it has
/// one. Depth of field reads its `depth` row, Light wrap its `background`,
/// Texturize its `texture`; each is unset by default and each renders as a
/// labelled no-op until something is named.
fn wire_aux(inst: &mut EffectInstance) {
    let aux = id("Aux");
    // Displacement map and Set matte take their second picture from the Matte
    // row itself, which is the deeper meaning K-395 gives that row.
    let rows: &[&str] = if matches!(
        inst.effect.match_name.as_str(),
        "displacement_map" | "set_matte"
    ) {
        &["depth", "background", "texture", "matte"]
    } else {
        &["depth", "background", "texture"]
    };
    for row in rows.iter().copied() {
        if let Some(p) = inst.params.iter_mut().find(|p| p.id == row) {
            if matches!(p.value, EffectValue::Layer(_)) {
                p.value = EffectValue::Layer(Some(aux));
            }
        }
    }
}

// ---------------------------------------------------------------------------

/// A small 3D LUT for the LUT effect's picture: cool shadows, warm highlights,
/// which is the grade everyone recognises and the one a `.cube` is usually
/// bought to apply. Written as text, because a `.cube` is text.
fn write_example_cube(path: &std::path::Path) -> std::io::Result<()> {
    const N: usize = 17;
    let mut out = String::from(
        "TITLE \"Lumit manual example\"
LUT_3D_SIZE 17

",
    );
    for b in 0..N {
        for g in 0..N {
            for r in 0..N {
                let (rf, gf, bf) = (
                    r as f32 / (N - 1) as f32,
                    g as f32 / (N - 1) as f32,
                    b as f32 / (N - 1) as f32,
                );
                let luma = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
                // Warmth rises with brightness, coolness falls away with it.
                let warm = luma * luma;
                let cool = (1.0 - luma) * (1.0 - luma);
                let clamp = |v: f32| v.clamp(0.0, 1.0);
                out.push_str(&format!(
                    "{:.6} {:.6} {:.6}
",
                    clamp(rf * (1.0 + 0.55 * warm) - 0.10 * cool),
                    clamp(gf * (1.0 + 0.10 * warm) - 0.04 * cool),
                    clamp(bf * (1.0 + 0.70 * cool) - 0.22 * warm),
                ));
            }
        }
    }
    std::fs::write(path, out)
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from)
}

#[test]
#[ignore = "wants a GPU, a source clip and a writable directory; run through web-docs/scripts/gen-effect-shots.mjs"]
fn render_every_effect_example() {
    let clip = env_path("LUMIT_FX_EXAMPLES_CLIP").expect("set LUMIT_FX_EXAMPLES_CLIP");
    // Optional: the plate's own depth pass, panned identically so the two stay
    // registered. The effects that read a second picture use it when it is here.
    let depth = env_path("LUMIT_FX_EXAMPLES_DEPTH").filter(|p| p.is_file());
    let out = env_path("LUMIT_FX_EXAMPLES_OUT").expect("set LUMIT_FX_EXAMPLES_OUT");
    assert!(clip.is_file(), "no clip at {}", clip.display());
    std::fs::create_dir_all(&out).expect("create the output directory");

    let mut r = match HeadlessRenderer::new() {
        Ok(r) => r,
        Err(e) => {
            lumit_gpu::no_adapter();
            eprintln!("skipping: no GPU adapter ({e})");
            return;
        }
    };

    let write = |dir: &std::path::Path, name: &str, rgba: &[u8], w: u32, h: u32| {
        std::fs::create_dir_all(dir).expect("create the category directory");
        std::fs::write(dir.join(format!("{name}.{w}x{h}.raw")), rgba).expect("write the picture");
    };

    // The untouched plate: the "before" half of a before-and-after, and the
    // yardstick the neutral report measures against.
    let (doc, comp_id) = example_doc(&clip, depth.as_deref(), None);
    let doc = Arc::new(doc);
    let (plate, pw, ph) = r
        .render_rgba(&doc, comp_id, FRAME, 1.0)
        .expect("render the plate");
    assert_eq!((pw, ph), (W, H));
    write(&out, "plate", &plate, pw, ph);

    // A real `.cube` when the runner passes one, and the harness's own plain
    // warm grade when it does not. The pictures differ; both are honest.
    let cube = match env_path("LUMIT_FX_EXAMPLES_LUT").filter(|p| p.is_file()) {
        Some(p) => p,
        None => {
            let p = out.join("example.cube");
            write_example_cube(&p).expect("write the example .cube");
            p
        }
    };

    let reference = lumit_core::fx::reference::reference();
    let mut neutral: Vec<String> = Vec::new();
    let mut flat: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut made = 0usize;

    // LUMIT_FX_EXAMPLES_ONLY=<match name> restricts the run to one effect,
    // for iterating on a single picture without the full several-minute run.
    let only = std::env::var("LUMIT_FX_EXAMPLES_ONLY").ok();
    for e in &reference.effects {
        if only.as_deref().is_some_and(|o| o != e.match_name) {
            continue;
        }
        if let Some(why) = unillustrable(e.match_name) {
            skipped.push(format!("{}: {why}", e.slug));
            continue;
        }
        let Some(mut inst) = lumit_core::fx::instantiate(e.match_name) else {
            failed.push(format!("{}: not in the registry", e.slug));
            continue;
        };
        wire_aux(&mut inst);
        if e.match_name == "lut" {
            set(
                &mut inst,
                "file",
                EffectValue::File(FileParam::single(cube.to_string_lossy().into_owned())),
            );
        }
        for (row, value) in showcase(e.match_name) {
            set(&mut inst, row, value);
        }

        let (doc, comp_id) = example_doc(&clip, depth.as_deref(), Some(inst));
        let doc = Arc::new(doc);
        match r.render_rgba(&doc, comp_id, FRAME, 1.0) {
            Ok((rgba, w, h)) => {
                if rgba == plate {
                    neutral.push(e.slug.clone());
                } else if rgba.chunks_exact(4).all(|px| px == &rgba[..4]) {
                    // One flat colour. The picture is not the plate, so the
                    // check above is happy with it, and it is still useless: an
                    // effect that rendered the frame black has failed in a way
                    // nobody spots until it is on the website.
                    flat.push(e.slug.clone());
                }
                write(&out.join(&e.category_slug), &e.slug, &rgba, w, h);
                made += 1;
            }
            Err(err) => failed.push(format!("{}: {err}", e.slug)),
        }
    }

    println!("wrote {made} example pictures to {}", out.display());
    for s in &skipped {
        println!("skipped: {s}");
    }
    for f in &failed {
        eprintln!("failed: {f}");
    }
    for n in &neutral {
        eprintln!("neutral (needs a showcase entry): {n}");
    }
    for n in &flat {
        eprintln!("flat (rendered one solid colour): {n}");
    }
    assert!(
        failed.is_empty(),
        "{} effect(s) failed to render",
        failed.len()
    );
    assert!(
        neutral.is_empty(),
        "{} effect(s) rendered identically to the plate; give each one a showcase() entry",
        neutral.len()
    );
    assert!(
        flat.is_empty(),
        "{} effect(s) rendered one flat colour, which is a broken picture, not a subtle one",
        flat.len()
    );
}
