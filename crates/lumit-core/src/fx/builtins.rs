use super::*;
use crate::anim::Property;
use crate::model::{
    EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue, FileParam,
};

/// Edge-policy option labels shared by the blur family (docs/08 §3.8) and
/// Shake (§3.4). Backed by the reusable [`EdgesMode`] enum (P3, K-145), so the
/// labels and the 0/1/2 codes stay in one place.
pub const EDGE_OPTIONS: &[&str] = EdgesMode::OPTIONS;

/// "No group dividers" for a [`ParamKind::Choice`]'s `dividers_after` (T21) —
/// the common case, spelled once so every ungrouped Choice reads the same.
pub const CHOICE_UNGROUPED: &[u32] = &[];

/// Which channel of an auxiliary picture an effect reads as a single number —
/// the depth out of a depth pass, the weight out of a custom aperture image.
///
/// One list, shared, so that every effect naming a channel of an auxiliary
/// picture names it from the same short list rather than declaring its own and
/// letting them drift. The index order is the wire form the resolved ops carry,
/// so entries are appended, never reordered.
///
/// **Every entry has to be able to explain itself.** A depth pass or a dirt
/// plate arrives as a picture, and the question is only which number in it is
/// the one the effect wants:
///
/// - **Luminance** — the default, and right for the overwhelmingly common case:
///   a grey map, whatever combination of channels it was written to. Weighted
///   (Rec.709) rather than a plain mean, so a pass that is only *nearly* grey
///   still reads sensibly.
/// - **Alpha** — some renderers put depth in the alpha of the beauty pass.
/// - **Red / Green / Blue** — a packed pass, where several AOVs were flattened
///   into one image and this one landed in a particular channel. Red is also the
///   historical convention for a depth pass on its own.
///
/// Hue, saturation and lightness are deliberately **not** here. Nothing encodes
/// a depth or a density as a hue, and offering the option only invites someone
/// to find out.
pub const CHANNEL_OPTIONS: &[&str] = &["Luminance", "Alpha", "Red", "Green", "Blue"];

/// Shake's twirls (P4): the per-axis wobble (FX-11, K-146) — the master
/// Amplitude/Frequency drive x and y together while this group biases each axis
/// and adds the z (depth/scale) shake that replaced the old Zoom pump — and the
/// Motion blur group (T18, K-165), the shake's own inter-frame smear (toggle +
/// amount). Each group's ids are a contiguous run of the schema's `params`.
const SHAKE_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Per-axis wobble",
        params: &["x_amp", "x_freq", "y_amp", "y_freq", "z_amp", "z_freq"],
        collapsed: true,
        visible_when: None,
    },
    ParamGroup {
        label: "Motion blur",
        params: &["motion_blur", "mb_amount"],
        collapsed: true,
        visible_when: None,
    },
];

/// The host-uniform Mix parameter every effect ends with (docs/08 §1.5),
/// in per cent, blending processed over unprocessed input.
const MIX_PARAM: ParamSchema = ParamSchema {
    id: "mix",
    label: "Mix",
    kind: ParamKind::Float {
        default: 100.0,
        slider: (0.0, 100.0),
        hard: (Some(0.0), Some(100.0)),
    },
};

/// The catalogue. Grows one entry per landed effect; the schema is the single
/// source of truth the UI menu, instantiation and resolution all read.
pub const BUILTINS: &[EffectSchema] = &[
    // The blur family, three single-purpose effects (docs/08 §3.8, K-137):
    // Gaussian (separable two-pass), Directional (a line-integral streak
    // along an angle) and Radial (arcs or rays about a centre). This was one
    // mode-driven "Blur" effect until K-137 split it, one job per effect
    // (K-090): each keeps its own maths, kernel and version unchanged — only
    // the schema (and the resolve arms that read it) changed. Gaussian keeps
    // match_name "blur", so a project saved with the old combined effect
    // loads as Gaussian (whatever mode it stored), byte-identically at its
    // Radius. Directional and Radial are new match names, reached from the
    // Add-effect menu.
    //
    // Edges: the old effect carried one shared Transparent/Repeat/Mirror
    // control across every mode; K-137 keeps it only on Radial (the mode
    // whose sweep most often wants Mirror or Transparent). Gaussian and
    // Directional resolve at the old default, Repeat (full-frame game
    // footage never darkens along the border), so their look is unchanged.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "blur",
        label: "Gaussian blur",
        version: 1,
        category: FxCategory::BlurSharpen,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            // The Radius slider's own maximum (its own effect now, no longer
            // sharing the family's largest reach).
            roi: Roi::PaddedPctDiag(25.0),
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "radius",
                label: "Radius",
                // % of the comp diagonal (§2.3), so half-res preview matches.
                // Default per §1.2's "drop it on and it already looks right".
                kind: ParamKind::Float {
                    default: 1.5,
                    slider: (0.0, 25.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Directional blur (docs/08 §3.8, K-137): a line-integral streak along
    // an angle. Full streak Length in % diag and the streak Angle. Length
    // may exceed 100 % of the diagonal now it is its own effect (slider to
    // 200, hard-unbounded above per K-090); the kernel's tap count still
    // clamps (cpu::dir_blur_taps), so a long streak stays bounded in cost.
    // Repeat-edged (see the family note above). ROI is full-frame: an
    // unbounded Length cannot be padded statically.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "directional_blur",
        label: "Directional blur",
        version: 1,
        category: FxCategory::BlurSharpen,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "length",
                label: "Length",
                // The full streak length, % diag (§2.3). Unbounded above
                // (K-090); the slider reaches 200 and typing goes further.
                kind: ParamKind::Float {
                    default: 10.0,
                    slider: (0.0, 200.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "angle",
                label: "Angle",
                // Streak direction, degrees (0° = +x).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-180.0, 180.0),
                    hard: (Some(-3600.0), Some(3600.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Radial blur (docs/08 §3.8, K-137): arcs (Spin) or rays (Zoom) about a
    // centre. Amount is the peak per-pixel tap spread in % diag, reached at
    // the frame's farthest corner from Centre; it may exceed 100 now it is
    // its own effect (slider to 100, hard-unbounded per K-090; the tap count
    // clamps in cpu::radial_blur_taps, so cost stays bounded). Centre is
    // Centre X / Centre Y, two Float params in % of comp width/height (the
    // schema has no Point-shaped ParamKind — Transform's Anchor/Position use
    // the same split). Type is Spin / Zoom; both reduce to one linear scale
    // of the pixel's own (position − centre) vector — Zoom along it (an exact
    // ray sample), Spin along its perpendicular (the tangent approximation to
    // the true arc) — so neither needs a division or a runtime trig call, and
    // every tap collapses to exactly the pixel at Centre with no epsilon
    // guard. This is the one blur to keep the shared Edges control
    // (Transparent/Repeat/Mirror); its taps run through the same
    // bilinear_edge sampler the others use.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "radial_blur",
        label: "Radial blur",
        version: 1,
        category: FxCategory::BlurSharpen,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // Peak tap spread, % diag (§2.3), reached at the farthest
                // corner from Centre. Unbounded above (K-090).
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "centre_x",
                label: "Centre X",
                // % of comp width. resolve_stack only carries diag_px (no
                // separate width/height), so this resolves to a *fraction* of
                // the raster and the CPU/GPU function scales it by its own w —
                // exactly how chromatic aberration derives the frame centre.
                kind: ParamKind::Float {
                    default: 50.0,
                    slider: (0.0, 100.0),
                    hard: (None, None), // off-frame centres are legal
                },
            },
            ParamSchema {
                id: "centre_y",
                label: "Centre Y",
                // % of comp height (see centre_x).
                kind: ParamKind::Float {
                    default: 50.0,
                    slider: (0.0, 100.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "radial_type",
                label: "Type",
                kind: ParamKind::Choice {
                    options: &["Spin", "Zoom"],
                    default: 0,
                    dividers_after: CHOICE_UNGROUPED,
                },
            },
            ParamSchema {
                id: "edge",
                label: "Edges",
                kind: ParamKind::Choice {
                    options: EDGE_OPTIONS,
                    default: 1, // Repeat: full-frame game footage never darkens
                    dividers_after: CHOICE_UNGROUPED,
                },
            },
            MIX_PARAM,
        ],
    },
    // Unsharp mask in linear light (docs/08 §3.9), on unpremultiplied colour
    // (§2.2: sharpening premultiplied values haloes matte edges). The
    // unpremultiply → sharpen → re-premultiply wrap is fused into the kernel.
    // Labelled "Unsharp mask" since K-138 split the plain 3×3 Sharpen out
    // below; the match_name stays "sharpen" so saved projects are unchanged.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "sharpen",
        label: "Unsharp mask",
        version: 1,
        category: FxCategory::BlurSharpen,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::PaddedPctDiag(4.0),
            temporal: &[0],
            premultiplied: false, // §2.2: operates on unpremultiplied colour
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // Per cent of the detail signal added back (§3.9: 0–300%).
                kind: ParamKind::Float {
                    default: 60.0,
                    slider: (0.0, 300.0),
                    hard: (Some(0.0), Some(300.0)),
                },
            },
            ParamSchema {
                id: "radius",
                label: "Radius",
                // % of the comp diagonal (§2.3) — the width of the detail
                // the mask lifts; small values crispen, larger add clarity.
                kind: ParamKind::Float {
                    default: 0.4,
                    slider: (0.05, 2.0),
                    hard: (Some(0.0), Some(4.0)),
                },
            },
            ParamSchema {
                id: "threshold",
                label: "Threshold",
                // Linear-light contrast below which detail is left alone,
                // so compression noise is not amplified (§3.9).
                kind: ParamKind::Float {
                    default: 0.05,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "luminance_only",
                label: "Luminance only",
                // Sharpen the luma signal only — avoids chroma fringing on
                // compressed game capture (§3.9).
                kind: ParamKind::Bool { default: true },
            },
            MIX_PARAM,
        ],
    },
    // Sharpen (docs/08 §3.9, K-138): the plain, radius-free sibling of the
    // Unsharp mask above — a fixed 3×3 high-pass convolution scaled by Amount,
    // `out = u + amount·(4·u − up − down − left − right)` per RGB channel with
    // clamp-addressed neighbours. On unpremultiplied colour (§2.2, the wrap
    // fused into the kernel), alpha untouched; the neighbours read the edge
    // pixel (clamp/Repeat) so a border never invents dark detail. Amount 0 is
    // the bit-exact passthrough (pinned by test). One job, cheap, one pixel of
    // reach — the honest "just sharpen it" control next to the Unsharp mask's
    // radius/threshold/luma knobs.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "sharpen_simple",
        label: "Sharpen",
        version: 1,
        category: FxCategory::BlurSharpen,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            // A fixed 3×3 kernel reads one pixel out; % diag of one raster
            // pixel is tiny, so 1 % over-covers at any sensible resolution.
            roi: Roi::PaddedPctDiag(1.0),
            temporal: &[0],
            premultiplied: false, // §2.2: sharpening premultiplied haloes matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // High-pass strength: 1 is the classic 5/−1 sharpen kernel, 0
                // a no-op. Clamped at zero below (a negative amount would
                // blur, out of scope), unbounded above (K-090).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 5.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "radius",
                label: "Radius",
                // Neighbour distance in raster pixels (T15): 1 = the classic 3×3
                // kernel, larger sharpens over a coarser neighbourhood.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (1.0, 8.0),
                    hard: (Some(1.0), None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Light wrap (docs/08 §3.28, K-358): the oldest trick in compositing and
    // one Lumit had no answer for. A keyed subject reads as pasted on because
    // in a real camera the light behind it spills round its edges; this takes
    // the referenced Background layer, blurs it over Width, and screens that
    // blur back only into the band just inside the foreground's own outline —
    // found from the foreground's alpha, so the effect needs no mask of its
    // own. Screened rather than added, so a bright plate brightens the edge
    // toward itself rather than past white. Width 0 is the bit-exact
    // passthrough, as is an unset Background (the labelled-no-op rule every
    // layer-input effect follows).
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "light_wrap",
        label: "Light wrap",
        version: 1,
        category: FxCategory::Stylise,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            // The wrap reaches Width inside the edge and the blur reads Width
            // out; 10 % of the diagonal covers any sane setting.
            roi: Roi::PaddedPctDiag(10.0),
            temporal: &[0],
            // The band is read off the foreground's own alpha, which only
            // means anything premultiplied.
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "background",
                label: "Background",
                // Unset until the owner picks one — a labelled no-op. No
                // self_default (K-288): a layer is never its own background,
                // so starting pointed at itself would be a wrap of nothing.
                kind: ParamKind::Layer {
                    self_default: false,
                },
            },
            ParamSchema {
                id: "width",
                label: "Width",
                // px@comp (K-260), and the same distance twice: how far the
                // wrap reaches inside the edge, and the radius the background
                // is softened by.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 200.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // Gain on the spill before it is screened on. Open above
                // (K-090) for a deliberately hot wrap.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 3.0),
                    hard: (Some(0.0), None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Chromatic aberration (docs/08 §3.6): R and B sample offset positions,
    // G stays put, alpha follows the green channel so mattes never fringe.
    // Operates premultiplied. Per-channel scales (FX-9) let each channel
    // fringe by its own amount. The Wavelength Bool (K-090 quality tier)
    // swaps the three-channel split for a `samples`-tap spectral dispersion
    // (FX-9/K-144: enough taps that a large offset disperses smoothly
    // rather than showing a few discrete copies) sharing the same offset.
    // The §3.6 Centre/Falloff/channel-blur extras land later; radial mode
    // grows the offset from the frame centre.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "rgb_split",
        label: "RGB split",
        version: 1,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::PaddedPctDiag(25.0),
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // % of the comp diagonal (§2.3); the impact-frame staple is
                // a keyframed spike on this.
                kind: ParamKind::Float {
                    default: 0.4,
                    slider: (0.0, 10.0),
                    hard: (Some(0.0), Some(25.0)),
                },
            },
            ParamSchema {
                id: "angle",
                label: "Angle",
                // Degrees, linear mode: the direction R shifts (B mirrors).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-180.0, 180.0),
                    hard: (Some(-3600.0), Some(3600.0)),
                },
            },
            // Per-tap displacement scales (FX-9): each of the three taps shifts
            // by the overall Amount times its own per-cent scale, so the taps
            // can fringe by different amounts (or the middle tap be nudged off
            // its anchor). Taps 1 and 2 displace along −offset, tap 3 along
            // +offset — so the defaults 100 / 0 / 100 %, paired with the default
            // red / green / blue tints below, reproduce the classic split (R one
            // way, B the other, G unmoved) bit-for-bit. Open both sides (K-135):
            // a negative scale flips a tap's direction, and there is no natural
            // ceiling on how far a tap may fringe. Labelled Red / Green / Blue
            // for the classic case; each really scales its like-numbered tint.
            ParamSchema {
                id: "red_amount",
                label: "Red",
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (-200.0, 200.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "green_amount",
                label: "Green",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-200.0, 200.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "blue_amount",
                label: "Blue",
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (-200.0, 200.0),
                    hard: (None, None),
                },
            },
            // The three tap tints (T17): the same reusable three-colour picker
            // chromatic aberration carries (K-143), tinting the three offset
            // taps. Defaults red / green / blue reproduce the classic
            // channel-separated split bit-for-bit (each primary tint keeps only
            // its own channel of its tap); any other colours cross-tint the
            // fringe. Named `channel_colour_1/2/3` so the picker widget groups
            // them into one swatch row.
            ParamSchema {
                id: "channel_colour_1",
                label: "Colour 1",
                kind: ParamKind::Colour {
                    default: [1.0, 0.0, 0.0, 1.0],
                    range: (0.0, 1.0),
                },
            },
            ParamSchema {
                id: "channel_colour_2",
                label: "Colour 2",
                kind: ParamKind::Colour {
                    default: [0.0, 1.0, 0.0, 1.0],
                    range: (0.0, 1.0),
                },
            },
            ParamSchema {
                id: "channel_colour_3",
                label: "Colour 3",
                kind: ParamKind::Colour {
                    default: [0.0, 0.0, 1.0, 1.0],
                    range: (0.0, 1.0),
                },
            },
            ParamSchema {
                id: "wavelength",
                label: "Wavelength",
                // K-090 quality tier: off = the classic three-tap split
                // (byte-identical to before this Bool existed); on = a smooth
                // dispersion — `samples` spectral taps along the same offset,
                // each tinted by the three-colour picker sampled as a gradient
                // (A1/K-163: Colour 1 → Colour 2 → Colour 3 across the span) and
                // recombined in linear, for the higher-quality smooth fringe. The
                // per-tap scales above apply to the classic mode only; the tints
                // drive both modes.
                kind: ParamKind::Bool { default: false },
            },
            ParamSchema {
                id: "samples",
                label: "Samples",
                // Wavelength mode's tap count (FX-9/K-144): more taps fill the
                // same ±offset span more densely, so a large offset disperses
                // as a smooth rainbow rather than a few discrete stacked
                // copies. The resolver rounds and clamps to 3..=64
                // (SPECTRAL_MAX_SAMPLES); ignored in the classic mode.
                kind: ParamKind::Float {
                    default: 16.0,
                    slider: (3.0, 64.0),
                    hard: (Some(3.0), Some(64.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Chromatic aberration (docs/08 §3.15): the always-radial sibling of
    // RGB split's linear tinted-tap fringe (§3.6, T17) — R pulled outward, B
    // pulled inward, G and alpha unshifted, growing from the frame centre.
    // Where RGB split's Amount is % diag (a currency it shares with its
    // Angle-driven linear offset), this effect has only the radial shape and
    // one purpose, so Amount is authored in raw px@comp (§2.3) instead —
    // scaled by the preview factor exactly like Glitch's Block size — because
    // "how many pixels of fringe" is the honest unit for a single-purpose
    // corner effect with no angle to share a currency with. K-143/K-144 add
    // the reusable three-colour channel picker (the three radial taps' tints,
    // default r/g/b) and the shared Wavelength/Samples spectral machinery.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "chromatic_aberration",
        label: "Chromatic aberration",
        version: 1,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            // Amount is raw px@comp, not % diag, so a tight %-diag padding
            // cannot be declared statically across every comp resolution;
            // full-frame is the safe static bound (mirroring Glitch's own
            // px@comp parameters, which take the same route).
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // px@comp (§2.3): peak channel offset, reached at the
                // corner distance from the frame centre. Open above (K-135):
                // there is no natural ceiling on how much fringe an editor
                // may want.
                kind: ParamKind::Float {
                    default: 4.0,
                    slider: (0.0, 20.0),
                    hard: (Some(0.0), None),
                },
            },
            // The three channel colours (P2/K-143): the reusable three-colour
            // picker tints the three radial taps. Defaults red / green / blue
            // reproduce the classic R-outward / B-inward / G-anchor split
            // bit-for-bit (each primary tint keeps only its own channel).
            // Named `channel_colour_1/2/3` by convention so the picker widget
            // finds the group; any future three-tinted-channel effect reuses it.
            ParamSchema {
                id: "channel_colour_1",
                label: "Colour 1",
                kind: ParamKind::Colour {
                    default: [1.0, 0.0, 0.0, 1.0],
                    range: (0.0, 1.0),
                },
            },
            ParamSchema {
                id: "channel_colour_2",
                label: "Colour 2",
                kind: ParamKind::Colour {
                    default: [0.0, 1.0, 0.0, 1.0],
                    range: (0.0, 1.0),
                },
            },
            ParamSchema {
                id: "channel_colour_3",
                label: "Colour 3",
                kind: ParamKind::Colour {
                    default: [0.0, 0.0, 1.0, 1.0],
                    range: (0.0, 1.0),
                },
            },
            ParamSchema {
                id: "wavelength",
                label: "Wavelength",
                // K-144 quality tier, reusing RGB split's own spectral
                // machinery (K-090): off = the three tinted radial taps above;
                // on = `samples` spectral taps for a smooth rainbow fringe. Off
                // (and absent on projects saved before this Bool) keeps the
                // historical three-channel behaviour.
                kind: ParamKind::Bool { default: false },
            },
            ParamSchema {
                id: "samples",
                label: "Samples",
                // Wavelength mode's tap count (K-144): the same control RGB
                // split's Wavelength mode carries. Rounded and clamped to
                // 3..=64; ignored when Wavelength is off.
                kind: ParamKind::Float {
                    default: 16.0,
                    slider: (3.0, 64.0),
                    hard: (Some(3.0), Some(64.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Beat-aware strobe (docs/08 §3.7). Manual mode is the original manual
    // form: each keyframe on Trigger is a hit (its value = how hard, 0..1)
    // that decays exponentially over Decay; a static Trigger holds a
    // constant flash. Trigger mode fires the §1.4 envelope from the comp's
    // beat markers; Strobe fires every Nth beat only. Instances saved
    // before the marker modes existed carry no "mode" parameter and
    // resolve as Manual, byte-identically. Default is a no-op by design:
    // §1.2 exempts inherently trigger-driven effects.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "flash",
        label: "Flash",
        version: 1,
        category: FxCategory::Stylise,
        traits: EffectTraits {
            cost: CostClass::Trivial,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: true, // binds to comp beat markers per §1.4
        },
        params: &[
            ParamSchema {
                id: "mode",
                label: "Mode",
                // Manual = keyframed hits on Trigger (the original form);
                // Trigger = the §1.4 beat envelope; Strobe = every Nth
                // beat only.
                kind: ParamKind::Choice {
                    options: &["Manual", "Trigger", "Strobe"],
                    default: 0,
                    dividers_after: CHOICE_UNGROUPED,
                },
            },
            ParamSchema {
                id: "trigger",
                label: "Trigger",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "duration",
                label: "Duration",
                // Frames (comp-rate, §2.3) a marker-driven flash lasts.
                // Hard floor 0, unbounded above (the K-090 one-sided
                // clamp); 0 is honestly a flash zero frames long — never
                // shown.
                kind: ParamKind::Float {
                    default: 2.0,
                    slider: (0.0, 12.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "shape",
                label: "Shape",
                // Hard holds full strength for Duration then cuts; Fade
                // decays linearly to zero across it.
                kind: ParamKind::Choice {
                    options: &["Hard", "Fade"],
                    default: 0,
                    dividers_after: CHOICE_UNGROUPED,
                },
            },
            ParamSchema {
                id: "every_nth",
                label: "Every Nth beat",
                // Strobe mode: fire beats 0, N, 2N, … of the comp's beat
                // list. The spec's integer ≥ 1, carried as a Float row —
                // the resolver rounds and clamps at 1.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (1.0, 8.0),
                    hard: (Some(1.0), None),
                },
            },
            ParamSchema {
                id: "phase",
                label: "Phase offset",
                // Frames a marker-driven flash trails (> 0) or leads (< 0)
                // its beat.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-8.0, 8.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // Per cent scale on the trigger envelope.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(400.0)),
                },
            },
            ParamSchema {
                id: "colour",
                label: "Colour",
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.0, 4.0), // linear light: HDR flashes are legal
                },
            },
            ParamSchema {
                id: "decay",
                label: "Decay",
                // Milliseconds for a hit to fall to 1/e.
                kind: ParamKind::Float {
                    default: 120.0,
                    slider: (10.0, 1000.0),
                    hard: (Some(0.0), Some(10000.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Colour balance (docs/08 §3.10 as amended by K-090: the v1 Grade split
    // into single-purpose colour effects): lift / gamma / gain per channel,
    // in linear, on unpremultiplied colour (§2.2). Defaults are neutral —
    // a grade's "tasteful default" is a preset choice, which is what the
    // §3.10 preset browser is for.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "colour_balance",
        label: "Colour balance",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: grading premult shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "lift",
                label: "Lift",
                // Added after gain: raises (or crushes, negative) the blacks.
                kind: ParamKind::Colour {
                    default: [0.0, 0.0, 0.0, 1.0],
                    range: (-1.0, 1.0),
                },
            },
            ParamSchema {
                id: "gamma",
                label: "Gamma",
                // Mid-tone curve per channel; 1 is neutral.
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.1, 4.0),
                },
            },
            ParamSchema {
                id: "gain",
                label: "Gain",
                // Linear multiplier per channel; 1 is neutral.
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.0, 4.0),
                },
            },
            MIX_PARAM,
        ],
    },
    // Saturation (docs/08 §3.10 as amended by K-090): one job — scale
    // colourfulness about Rec. 709 luma, in linear, on unpremultiplied
    // colour (§2.2). Neutral default: like the balance above, its tasteful
    // setting is a preset choice.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "saturation",
        label: "Saturation",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: grading premult shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "saturation",
                label: "Saturation",
                // Per cent about Rec. 709 luma: 0 = greyscale, 100 = neutral,
                // 200 = doubled. The maths (a mix of luma and colour by
                // saturation ÷ 100) simply keeps extrapolating above 200, so
                // the hard ceiling is open (K-135): the slider reaches a
                // heavy 400, and typing higher pushes further.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 400.0),
                    hard: (Some(0.0), None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Vibrancy (docs/08 §3.10, K-152): a saturation boost weighted by each
    // pixel's current colourfulness — low-saturation pixels gain more,
    // already-vivid ones little, so skin tones and near-neutrals lift while
    // saturated areas are protected from clipping (unlike Saturation's uniform
    // scale). Same domain as Saturation: linear light, unpremultiplied (§2.2).
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "vibrancy",
        label: "Vibrancy",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: grading premult shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // Per cent: 0 = neutral (bit-exact identity), higher lifts the
                // less-saturated pixels more. The slider reaches a heavy 200;
                // typing higher pushes further (K-135 open ceiling), floored
                // at 0.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 200.0),
                    hard: (Some(0.0), None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Vignette (docs/08 §3.14, listed as a planned colour effect in §3.10):
    // darkens toward black away from the frame centre, in premultiplied
    // colour (a coverage-like darken, not a lift/gamma/gain grade, so no
    // unpremultiply round trip). Category Colour, alongside Colour balance
    // and Saturation — its closest siblings and where §3.10's own text
    // already lists it.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "vignette",
        label: "Vignette",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // 0..1: the darkening strength; 0 is the neutral point
                // (bit-exact passthrough, pinned by test).
                kind: ParamKind::Float {
                    default: 0.5,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "radius",
                label: "Radius",
                // 0..1: how far from centre the clear area reaches, in the
                // Roundness-blended distance metric below (1.0 = that
                // metric's own reference edge).
                kind: ParamKind::Float {
                    default: 0.75,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "softness",
                label: "Softness",
                // Feather width beyond Radius, in the same normalised metric.
                // The metric is not capped at 1 (a distance reaches ~√2 at a
                // corner under circular roundness), so Softness may exceed 1
                // for a wider feather (K-135): the hard ceiling is open, the
                // slider reaches 2.
                kind: ParamKind::Float {
                    default: 0.5,
                    slider: (0.0, 2.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "roundness",
                label: "Roundness",
                // 1 = circular (both axes read equal pixel distances as
                // equal); 0 = follows the frame's own aspect ratio (an
                // ellipse exactly reaching every edge at Radius 1).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "ramp",
                label: "Ramp",
                // Gamma on the black↔clear falloff (T16): 1 = the plain
                // smoothstep, > 1 rolls the dark in later then faster, < 1
                // earlier and gentler — a curve/levels on the darkening amount.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.2, 4.0),
                    hard: (Some(0.05), None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Exposure (docs/08 §3.16): a single scene-linear gain on RGB (2^stops) —
    // the montage grade's brightness lever. Premultiplied: a scalar scales
    // premultiplied colour consistently, so no unpremultiply round trip and
    // alpha is untouched. 0 stops is the neutral point (bit-exact passthrough,
    // pinned by test). Category Colour, beside its grade siblings.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "exposure",
        label: "Exposure",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "stops",
                label: "Stops",
                // Photographic stops; each +1 doubles the light. 0 is neutral.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-5.0, 5.0),
                    hard: (None, None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Hue shift (docs/08 §3.17, K-136): rotate every colour's hue by an
    // angle. Preserve luminance (default on) keeps perceived brightness
    // fixed as the hue turns — the constant-luminance rotation weighted by
    // Rec.709 luma — while off is a plain geometric spin about the grey
    // axis (equal weights) that lets brightness ride with the hue. Either
    // way it is a linear 3×3 colour matrix, computed host-side (the bool
    // only picks which weights), so the CPU reference and the kernel
    // multiply by identical coefficients and preview equals export (K-031);
    // premultiplied (a linear matrix scales through alpha), alpha untouched.
    // 0° is the bit-exact neutral point in both modes. Category Colour,
    // beside its grade siblings.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "hue_shift",
        label: "Hue shift",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "angle",
                label: "Angle",
                // Degrees on a dial (docs/07 §6): a hue shift is a rotation
                // about the colour wheel, so the control is that wheel. Wraps
                // every 360, and unbounded so an animated hue winds through
                // whole turns rather than stopping.
                kind: ParamKind::Angle {
                    default: 0.0,
                    dial_step: 15.0,
                },
            },
            ParamSchema {
                // On (default): the constant-luminance rotation (Rec.709 luma
                // held). Off: a plain-RGB spin about the grey axis, brightness
                // free to change with the hue. Absent on projects saved before
                // this bool existed → true, the historical behaviour.
                id: "preserve_luminance",
                label: "Preserve luminance",
                kind: ParamKind::Bool { default: true },
            },
            MIX_PARAM,
        ],
    },
    // Contrast (docs/08 §3.18): expand or compress RGB about a fixed mid-grey
    // pivot (0.5) — the montage grade's punch lever. An affine grade
    // (out = (in − pivot) × k + pivot), and because of the − pivot offset it
    // does NOT commute with premultiplied alpha, so premultiplied: false: the
    // host unpremultiplies, grades, and re-premultiplies, exactly like
    // Saturation and Colour balance. 100 % (k = 1) is the neutral point
    // (bit-exact passthrough, pinned by test). Continuous everywhere (no
    // round/clamp/quantize), so the §1.6 oracle holds. Category Colour, beside
    // its grade siblings.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "contrast",
        label: "Contrast",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: an affine grade shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "contrast",
                label: "Contrast",
                // Per cent about mid-grey: 0 = flat grey, 100 = neutral,
                // 200 = doubled. Hard min 0 (no inversion); unbounded above.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 200.0),
                    hard: (Some(0.0), None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Gamma (docs/08 §3.19): a per-channel power curve in the effect's
    // scene-linear working space — out = pow(max(in, 0), 1/gamma) per RGB
    // channel, alpha untouched. The input is clamped to ≥ 0 before the pow
    // (scene-linear can dip slightly negative, and the clamp must be
    // byte-identical on CPU and GPU so the §1.6 oracle holds). pow is
    // non-linear, so — like Contrast and Saturation — it does NOT commute with
    // premultiplied alpha: premultiplied: false, and the host unpremultiplies,
    // curves, and re-premultiplies. Gamma 1.0 is the neutral point (a bit-exact
    // passthrough short-circuit, not a reliance on pow(x, 1) == x). Continuous
    // everywhere for input ≥ 0, so the §1.6 oracle holds. Category Colour,
    // beside its grade siblings.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "gamma",
        label: "Gamma",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: a non-linear curve shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "gamma",
                label: "Gamma",
                // The power curve raises to 1/gamma. 1 is neutral; hard floor
                // 0.01 keeps 1/gamma finite, no hard ceiling above.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.1, 4.0),
                    hard: (Some(0.01), None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Temperature (docs/08 §3.20): a warm/cool white-balance shift as a
    // per-channel gain in scene-linear light — the montage grade's warmth
    // lever. `k = Temperature ÷ 100`; `gain_r = 1 + 0.75·k` boosts red as it
    // warms, `gain_b = 1 − 0.75·k` cuts blue, green untouched — a stronger
    // per-unit gain (K-135) so full deflection reads as a decisive orange or
    // blue rather than a timid tint, the gains floored at 0 so an extreme
    // never drives a channel negative. Premultiplied: a
    // per-channel scalar scales premultiplied colour consistently (straight ×
    // gain, then × the unchanged alpha), so no unpremultiply round trip and
    // alpha is untouched — exactly like Exposure's pure multiply, and unlike
    // the affine Contrast/Saturation grades (their − pivot offset breaks that
    // commutation, §2.2). The two gains are computed host-side (in the resolve
    // step) so the CPU reference and the kernel multiply by byte-identical
    // factors. Temperature 0 is the neutral point (gains 1.0, bit-exact
    // passthrough, pinned by test). Category Colour, beside its grade siblings.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "temperature",
        label: "Temperature",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "temperature",
                label: "Temperature",
                // A plain number: negative cools (blue up, red down), positive
                // warms (red up, blue down). 0 is neutral. The slider reaches
                // ±150 and the hard range ±200 (K-135): with the stronger
                // ±0.75·k gain, ±150 already pushes one channel toward black,
                // so a user rarely runs out of headroom wanting more.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-150.0, 150.0),
                    hard: (Some(-200.0), Some(200.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // LUT (docs/08 §3.11, docs/impl/lut.md, K-114): a 3D colour look-up from a
    // `.cube` file — a colourist's baked grade dropped onto a layer. A File
    // parameter picks the cube (animatable only by stepping between paths, since
    // two files cannot be blended) and Mix blends the graded result over the
    // input. The heavy lifting lives elsewhere: `lumit_core::lut` parses the
    // cube, `lumit_gpu::fx` samples it as a 3D texture. The resolve step carries
    // only Mix — a path is not `Copy`, so (like Echo's neighbour frames and
    // Motion blur's flow field) the loaded cube travels beside the resolved op,
    // supplied by the caller's LUT cache. Unpremultiplied (§2.2: a LUT is an
    // arbitrary colour map, so it must not see premultiplied values); an unset,
    // missing, 1D or unreadable file is a labelled no-op, never a fault (§3.11
    // never-crash rule). Moderate cost (a per-pixel 3D lookup), Exact ROI.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "lut",
        label: "LUT",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: an arbitrary colour map must see straight colour
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "file",
                label: "File",
                // A `.cube` LUT chosen from a dialog (K-111); the value steps
                // between paths with hold keys only.
                kind: ParamKind::File {
                    filter: &["cube"],
                    filter_name: "Cube LUT",
                },
            },
            MIX_PARAM,
        ],
    },
    // Depth of field (docs/08 §3.22, docs/impl/layer-input.md): a lens blur
    // driven by a depth pass. A `Layer` parameter names another layer as the
    // depth input (docs/impl/layer-input.md §3), Focus/Range set the sharp band
    // and Aperture the maximum blur disc.
    //
    // **In plain terms.** This is the whole lens, not just the blur. The first
    // rows are what a depth-of-field always asked for — which layer carries the
    // depth, what is sharp, how soft everything else goes. Behind the twirls is
    // the part that makes it read as a *lens* rather than as a smudge: the
    // shape of the iris the light is smeared into, and the point at which a
    // highlight stops averaging away and starts blooming into a ball.
    //
    // **Every one of those is neutral at its default, and neutral means
    // bit-identical.** The kernel does not multiply by a neutral value, it
    // *branches around* the whole weighted path — because `Σ(c·w)/Σw` is not an
    // identity in IEEE 754 even when every `w` is 1, and neither is splitting a
    // tap at a threshold and putting it back together. So a project saved
    // before any of this existed renders the same pixels it always did, which
    // is what let this fold into the shipped effect instead of arriving as a
    // second one beside it (K-313).
    //
    // The heavy lifting is `lumit_gpu::fx::dof` / `fx_dof.wgsl`; resolution
    // carries only the scalars — the depth layer is not `Copy`, so (like the
    // LUT's cube and Motion blur's flow field) the referenced layer's rendered
    // texture travels beside the resolved op, rendered alone at comp size
    // exactly as a matte layer is. An unset (or dangling) depth reference is a
    // labelled no-op, never a fault (the same sanctioned exception the File
    // parameter takes to the "no no-op default" rule). Premultiplied (the
    // aperture gathers the working premultiplied colour, per `fx_dof.wgsl`),
    // Moderate cost, `{0}` temporal. ROI is a padded gather: the static
    // declaration covers the Aperture slider's 40 px@comp maximum across
    // typical rasters (docs/08 §2.3 % diag ≈ 40 px at ≥ 1080p).
    EffectSchema {
        // The twirls (K-145's P4 groups). Both hold the controls that turn a
        // disc average into a lens, and both arrive collapsed: the rows above
        // them are the effect, and these are how it is shaped.
        groups: &[
            ParamGroup {
                label: "Iris",
                params: &["blades", "roundness", "rotation", "aspect", "rim"],
                collapsed: true,
                visible_when: None,
            },
            ParamGroup {
                label: "Highlights",
                params: &["threshold", "exposure"],
                collapsed: true,
                visible_when: None,
            },
            ParamGroup {
                // How the depth pass is READ — which number in it is depth,
                // which way round it runs, and how hard the blur answers to it.
                // Where focus *is* lives above, beside the rows that set it.
                label: "Depth map",
                params: &[
                    "depth_channel",
                    "depth_invert",
                    "gamma",
                    "remove_edge_leak",
                    "detect_edge_threshold",
                ],
                collapsed: true,
                visible_when: None,
            },
        ],
        // The greyed rows: which of two controls is in charge, said in the
        // panel rather than left for the owner to discover by dragging
        // something inert.
        enabled_when: &[
            // Focus point takes over from Focus distance. While it is ticked,
            // focus is whatever depth sits under the point and the distance
            // number decides nothing — and while it is not, the point does not.
            EnabledWhen {
                param: "focus",
                on: "use_focus_point",
                cond: EnabledCond::BoolIs(false),
            },
            EnabledWhen {
                param: "focus_point_x",
                on: "use_focus_point",
                cond: EnabledCond::BoolIs(true),
            },
            EnabledWhen {
                param: "focus_point_y",
                on: "use_focus_point",
                cond: EnabledCond::BoolIs(true),
            },
            // Everything that reads the depth pass needs one to read. With no
            // layer picked the effect defocuses the frame uniformly, and these
            // rows have nothing to describe.
            EnabledWhen {
                param: "depth_channel",
                on: "depth",
                cond: EnabledCond::LayerSet,
            },
            EnabledWhen {
                param: "use_focus_point",
                on: "depth",
                cond: EnabledCond::LayerSet,
            },
            EnabledWhen {
                param: "remove_edge_leak",
                on: "depth",
                cond: EnabledCond::LayerSet,
            },
            EnabledWhen {
                param: "detect_edge_threshold",
                on: "depth",
                cond: EnabledCond::LayerSet,
            },
        ],
        match_name: "dof",
        label: "Depth of field",
        version: 1,
        category: FxCategory::BlurSharpen,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            // Aperture is px@comp (up to 40); 3 % of the comp diagonal covers
            // that on a 1080p+ raster and over-covers smaller ones — a safe
            // static bound for a runtime-sized gather (docs/impl/layer-input.md).
            // The aperture polygon is INSCRIBED in that circle at every
            // Roundness and Deform (see `aperture_blades`), so shaping it can
            // only ever gather fewer taps — the bound holds unchanged.
            roi: Roi::PaddedPctDiag(3.0),
            temporal: &[0],
            premultiplied: true, // the aperture gathers premultiplied colour (fx_dof.wgsl)
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "depth",
                label: "Depth layer",
                // The layer whose depth channel is the depth pass (0 = near,
                // 1 = far by convention; the effect is symmetric about Focus).
                // Unset until the owner picks one (a labelled no-op): a
                // depth pass is never the picture itself, so no
                // `self_default` here (K-288) — though pointing it at this
                // layer is still allowed, and reads the effect's own input.
                kind: ParamKind::Layer {
                    self_default: false,
                },
            },
            // The depth Layer input's sampling mode (K-142) is not a schema
            // parameter: the inspector renders a source combobox beside the
            // Layer picker (None / Masks / Effects and masks) and stores it as a
            // `depth_source` Choice on the instance, read through
            // `EffectInstance::layer_source("depth")`. A project saved with
            // K-125's `depth_after_effects` bool still loads — `layer_source`
            // falls back to it. Replaces the old "Depth after effects" checkbox.
            ParamSchema {
                id: "focus",
                label: "Focus distance",
                // The in-focus depth, 0..1. Mid-depth by default so a typical
                // near-to-far pass has its middle sharp. Greys out while Use
                // focus point is on, because then the point decides.
                kind: ParamKind::Float {
                    default: 0.5,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                // Focus by clicking the thing you want sharp rather than by
                // hunting for a number. Off by default: it changes what Focus
                // distance means, and a saved project must keep meaning what it
                // meant.
                id: "use_focus_point",
                label: "Use focus point",
                kind: ParamKind::Bool { default: false },
            },
            ParamSchema {
                // Where to read the focus depth, px@comp (K-260: point
                // parameters are PIXELS, never % of frame). Pairs with
                // `focus_point_y` into one point row with a crosshair pick
                // (docs/07 §6.1) — the same row the Lens flare's Light uses,
                // which is why this is a Float pair and not a schema kind of
                // its own. The schema default is nominal 1080p centre;
                // `instantiate_for_raster` centres a fresh instance on the
                // actual comp.
                id: "focus_point_x",
                label: "Focus point x",
                kind: ParamKind::Float {
                    default: 960.0,
                    slider: (0.0, 3840.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "focus_point_y",
                label: "Focus point y",
                kind: ParamKind::Float {
                    default: 540.0,
                    slider: (0.0, 2160.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "range",
                label: "Focus range",
                // Half-width of the sharp band around Focus, 0..1: depths
                // within it stay crisp.
                kind: ParamKind::Float {
                    default: 0.1,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "aperture",
                label: "Aperture",
                // The master maximum circle-of-confusion radius in px@comp
                // (§2.3), reached at the farthest-from-focus depth. Scales both
                // per-side radii about its default 8 (unity: `aperture / 8`), so
                // a project saved before Near/Far existed — which has only this
                // param — renders identically (Near/Far fall back to 8, and
                // 8·aperture/8 = aperture on both sides). Clamped at zero below
                // (a zero master is a passthrough), unbounded typing above the
                // 40 px slider.
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.0, 40.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                // Per-side circle-of-confusion for the near side — depths in
                // front of focus (`d < focus`). px@comp, scaled by the Aperture
                // master. Owner's "adjust close/far blur separately". Absent on
                // pre-feature projects, where it falls back to Aperture.
                id: "near_aperture",
                label: "Near blur",
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.0, 40.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                // Per-side circle-of-confusion for the far side — depths behind
                // focus (`d >= focus`). px@comp, scaled by the Aperture master.
                // Absent on pre-feature projects, where it falls back to
                // Aperture, keeping the old symmetric behaviour.
                id: "far_aperture",
                label: "Far blur",
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.0, 40.0),
                    hard: (Some(0.0), None),
                },
            },
            // ---- The Iris twirl ----
            ParamSchema {
                // The iris's blade count — the shape a defocused highlight is
                // smeared into. Inert at Roundness 1 (a circle has no blades),
                // which is why the schema needs no Circle entry beside it. The
                // ceiling is [`MAX_BLADES`], shared with the kernel's uniform
                // array; an Int rather than a Choice so a keyframe can sweep it,
                // stepping 5 → 6 rather than growing half a blade.
                id: "blades",
                label: "Blades",
                kind: ParamKind::Int {
                    default: 6,
                    slider: (3, MAX_BLADES as i64),
                    hard: (Some(3), Some(MAX_BLADES as i64)),
                },
            },
            ParamSchema {
                // Bows the blades. 0 is a straight-edged polygon, 1 is the
                // circle, and **negative goes concave** — five blades at −1 is a
                // star. 1 by default: that is the plain disc this effect has
                // always gathered, so the shape controls cost an existing
                // project nothing until it asks for them.
                id: "roundness",
                label: "Roundness",
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (-1.0, 1.0),
                    hard: (Some(-1.0), Some(1.0)),
                },
            },
            ParamSchema {
                // Turns the iris. Degrees on a dial (docs/07 §6), unbounded, so
                // it winds through full turns rather than stopping at 360.
                // Inert at Roundness 1, like Blades.
                id: "rotation",
                label: "Rotation",
                kind: ParamKind::Angle {
                    default: 0.0,
                    dial_step: 15.0,
                },
            },
            ParamSchema {
                // The aperture's aspect: 0 is round, positive stretches the
                // highlights wide and negative stretches them tall — the oval
                // an anamorphic scope lens throws. Not a ratio in the
                // 1.33-or-2.0 sense; it is a squeeze either side of round,
                // which is why it runs −1…1 rather than upward from 1.
                id: "aspect",
                label: "Aspect ratio",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1.0, 1.0),
                    hard: (Some(-1.0), Some(1.0)),
                },
            },
            ParamSchema {
                // **Where the light sits inside each ball.** A real lens does
                // not throw a flat disc: an under-corrected one rings the edge
                // bright (the "soap bubble" bokeh), an over-corrected one pools
                // the light in the middle (creamy, smooth). That is spherical
                // aberration, and this is the dial for it — negative a soft
                // centre, 0 the flat disc a plain gather produces, positive a
                // bright rim. **Our reading of the curve, not measured against a
                // reference plugin** — docs/08 §3.22 records that.
                id: "rim",
                label: "Rim brightness",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1.0, 1.0),
                    hard: (Some(-1.0), Some(1.0)),
                },
            },
            // ---- The Highlights twirl ----
            ParamSchema {
                // The linear level each tap is split at before the power mean:
                // everything below it averages flat, everything above expands.
                // 1.0 is scene white, so only genuinely over-range highlights
                // bloom — and with Exposure at 0 this decides nothing at all,
                // because the split never happens.
                id: "threshold",
                label: "Highlight threshold",
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 4.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                // How hard the over-threshold part of each tap blooms, in stops.
                // The gather's mean becomes a *power* mean, so a small bright
                // area survives being averaged with its dark surroundings
                // instead of vanishing into it — which is the whole difference
                // between a blur and a bokeh.
                //
                // **0 by default, and 0 is the plain arithmetic mean**: the
                // kernel branches around the split entirely, so an existing
                // project's blur is untouched to the bit. Turning this up is
                // what lights the balls.
                //
                // The stops-to-power constant lives in `resolve_one`
                // (`EXPOSURE_STOPS_PER_DOUBLING`) and is fitted, not measured;
                // docs/08 §3.22 records it as open.
                id: "exposure",
                label: "Highlight exposure",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 30.0),
                    hard: (Some(-30.0), Some(30.0)),
                },
            },
            // ---- The Depth map twirl ----
            ParamSchema {
                // Which channel of the depth layer carries depth. Red by
                // default — the channel this effect has always read, and the
                // one a depth pass conventionally arrives in — but a pass that
                // comes as luminance or in the alpha is ordinary enough to
                // deserve the pick.
                id: "depth_channel",
                label: "Depth channel",
                kind: ParamKind::Choice {
                    options: CHANNEL_OPTIONS,
                    default: 0, // Red
                    dividers_after: CHOICE_UNGROUPED,
                },
            },
            ParamSchema {
                // Invert the depth pass (d' = 1 - d) before the circle-of-
                // confusion, swapping near and far — the owner's "tick to
                // invert the depth" box (Frischluft / DOF PRO both offer it).
                // Off (default) keeps the historical reading, so old projects
                // are unchanged. Continuous, so the §1.6 ULP oracle still holds.
                id: "depth_invert",
                label: "Depth invert",
                kind: ParamKind::Bool { default: false },
            },
            ParamSchema {
                // **The gamma on the depth axis** — the depth distance rescaled
                // before the ramp, which decides how hard the blur answers to a
                // small change in depth, and is what stops focus being
                // all-or-nothing on a real depth pass.
                //
                // **The range is wide on purpose, and ±1 was not enough.** A
                // real depth pass rarely spreads its content over 0..1: a linear
                // depth channel puts the sky or a distant ceiling at 1.0 and
                // compresses an entire room into the bottom fifth, so the depth
                // *differences* that matter are a tenth of the range or less. At
                // ±1 this control could only compress the falloff fourfold,
                // which left such a pass focusing all-or-nothing however it was
                // set — verified on the owner's own footage through the Focus
                // map view.
                //
                // The scale is **one doubling per unit** (`2^profile`), chosen so
                // the whole slider stays useful: the setting that reads well on a
                // linear depth pass off game footage lands around 6 (a 64×
                // magnification), which is the middle rather than the end, and
                // ±10 reaches 1024× for a pass squeezed harder still. 0 is the
                // neutral multiplier of exactly 1.
                id: "gamma",
                label: "Gamma",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-10.0, 10.0),
                    hard: (Some(-10.0), Some(10.0)),
                },
            },
            ParamSchema {
                // Sharp foreground colour bleeding into the defocused
                // background is the standard artefact of gathering across a
                // depth discontinuity; this pulls back taps that sit across one
                // AND in front of this pixel. 0 is off, and off takes the
                // unweighted gather — the arithmetic this effect has always
                // done. **Our reading**, though the artefact and the family of
                // fixes are well known.
                id: "remove_edge_leak",
                label: "Remove edge leak",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                // How big a depth jump counts as an edge for the row above.
                id: "detect_edge_threshold",
                label: "Detect edge threshold",
                kind: ParamKind::Float {
                    default: 0.10,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            // ---- Back out of the twirls ----
            ParamSchema {
                // On by default, which is what this effect has always done: a
                // gather running off the frame holds the border pixel outward
                // instead of pulling in transparency, so a bright edge does not
                // darken. Off lets the frame edge fall away, which is what a
                // flare element over black wants.
                //
                // Not the shared EdgesMode enum (P3, K-145) on purpose — that is
                // a three-way choice, and this is a two-state switch.
                id: "repeat_edge_pixels",
                label: "Repeat edge pixels",
                kind: ParamKind::Bool { default: true },
            },
            ParamSchema {
                // Diagnostic views (the realistic subset the reference plugins
                // ship). Rendered is the normal blurred output; Depth map shows
                // the post-invert depth as greyscale — after the channel pick,
                // so it is what the effect is actually reading; Focus map is the
                // smooth in-focus mask (white where sharp, darkening out of
                // focus). Every mode is continuous, so the §1.6 ULP oracle holds
                // across them. Absent on pre-feature projects → Rendered
                // (default 0). Forced to Rendered when no depth is bound: with
                // nothing to show, the views would draw whatever texture stands
                // in for the depth binding.
                id: "display",
                label: "Display",
                kind: ParamKind::Choice {
                    options: &["Rendered", "Depth map", "Focus map"],
                    default: 0,
                    dividers_after: CHOICE_UNGROUPED,
                },
            },
            MIX_PARAM,
        ],
    },
    // Transform (docs/08 §3.5, K-090): the layer transform group as a stack
    // entry — same parameter names, units and animatability. Its point is
    // adjustment layers: applied there, it transforms the composite of
    // everything below, which is the montage punch-in/whip gesture without
    // touching per-layer transforms. Identity parameters pass the input
    // through bit-exactly (pinned by test). The §3.5 Skew pair is post-v1.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "transform",
        label: "Transform",
        version: 1,
        category: FxCategory::Utility,
        traits: EffectTraits {
            cost: CostClass::Trivial,
            // §3.5: exact under pure translation, full-frame otherwise —
            // the static declaration carries the general case.
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "anchor_x",
                label: "Anchor x",
                // Pixels at full comp resolution (px@comp, §2.3), exactly
                // like the layer transform's Anchor; unbounded (K-090).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1000.0, 1000.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "anchor_y",
                label: "Anchor y",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1000.0, 1000.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "position_x",
                label: "Position x",
                // px@comp; the anchor point lands here. Defaults equal the
                // anchor's, so a fresh instance is the identity.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1000.0, 1000.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "position_y",
                label: "Position y",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1000.0, 1000.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "scale_x",
                label: "Scale x %",
                // Per cent, 100 = natural size; negative flips (like the
                // layer transform), so both hard sides stay open.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 400.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "scale_y",
                label: "Scale y %",
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 400.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "rotation",
                label: "Rotation",
                // Degrees on a dial (docs/07 §6), unbounded — whip transitions
                // spin whole turns, and a dial that stopped at 360 could not.
                kind: ParamKind::Angle {
                    default: 0.0,
                    dial_step: 15.0,
                },
            },
            ParamSchema {
                id: "opacity",
                label: "Opacity %",
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Glow (docs/08 §3.3): exposure-aware bloom in scene-linear light —
    // bright-pass with a soft knee, a wide gaussian on the leftover light,
    // additive recombine. The v1 core ships Threshold/Softness (id `knee`)/
    // Radius/Intensity/Tint; the §3.3 mip-chain items (Falloff, Chromatic
    // aberration, the Screen recombine) land with the progressive chain later
    // and these
    // parameters stay stable when they do. The bright pass thresholds all
    // four premultiplied channels alike, so the halo carries alpha and glow
    // spreads over transparency like light.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "glow",
        label: "Glow",
        version: 1,
        category: FxCategory::Stylise,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            // Radius is raw px@comp (K-135), unbounded above, so a tight
            // %-diag padding cannot be declared statically across every comp
            // resolution — full-frame is the safe static bound (mirroring
            // Chromatic aberration's own px@comp parameter).
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "threshold",
                label: "Threshold",
                // Linear-light value above which pixels bloom. The K-090
                // one-sided hard range made concrete: clamped at zero below,
                // unbounded above — HDR values beyond the slider are legal
                // and glow harder (§2.1). Default 0.8 so highlights just
                // shy of white already bloom on a fresh instance.
                kind: ParamKind::Float {
                    default: 0.8,
                    slider: (0.0, 4.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                // The id stays "knee" (stable identifier, addressed by
                // expressions and saved projects); only the UI label reads
                // "Softness", the plainer word for the same soft-knee width.
                id: "knee",
                label: "Softness",
                // Soft-knee width: the threshold's onset is eased by a
                // smoothstep over ±knee around it (§3.3 step 1), so the
                // bloom fades in rather than snapping on.
                kind: ParamKind::Float {
                    default: 0.5,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "radius",
                label: "Radius",
                // px@comp (§2.3, K-135): the halo gaussian's half-width in
                // real pixels — scaled by the preview factor like every
                // px@comp parameter — clamped at zero below and unbounded
                // above, so a wide bloom is a matter of typing a larger
                // number, not hitting a cap.
                kind: ParamKind::Float {
                    default: 24.0,
                    slider: (0.0, 200.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // Gain on the added halo; 0 is the effect's neutral point
                // (bit-exact passthrough, pinned by test).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 10.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "tint",
                label: "Tint",
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.0, 4.0), // linear light: HDR tints are legal
                },
            },
            MIX_PARAM,
        ],
    },
    // Shake (docs/08 §3.4, FX-11/K-146): seeded camera-shake — a
    // transform-domain wobble resampled once through the Transform kernel,
    // never pixel noise. The master Amplitude/Frequency/Rotation drive the
    // overall translational sway; a "Per-axis wobble" twirl (P4) biases each
    // of x, y and z (z is the depth/scale shake that replaced the old Zoom
    // pump), and an Edges control (P3) governs the border the wobble reveals
    // (it replaced the old Auto-scale bool). Style presets, Triggered mode
    // (§1.4 markers) and Motion blur shake follow — these parameters stay
    // stable when they do. Seeded (§1.3): its pixels are a function of time
    // under constant parameters, which the frame key reads (lumit-eval).
    EffectSchema {
        groups: SHAKE_GROUPS,
        enabled_when: &[],
        match_name: "shake",
        label: "Shake",
        version: 1,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: true,
            beat_input: false, // Triggered mode arrives with §1.4 plumbing
        },
        params: &[
            ParamSchema {
                id: "amplitude",
                label: "Amplitude",
                // % of the comp diagonal (§2.3): how far the wobble roams.
                kind: ParamKind::Float {
                    default: 1.5,
                    slider: (0.0, 20.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "frequency",
                label: "Frequency",
                // Hz — how fast the wobble wanders; the noise samples at
                // local time × frequency. Unbounded above (K-090): any
                // positive rate is meaningful, sampling handles it.
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.1, 30.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "rotation",
                label: "Rotation amount",
                // Degrees of twist wobble either way.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 45.0),
                    hard: (Some(0.0), Some(360.0)),
                },
            },
            // The "Per-axis wobble" twirl (SHAKE_GROUPS): x/y amount and
            // frequency are dimensionless multipliers on the master Amplitude
            // and Frequency (default 1 reproduces the old uniform x/y shake);
            // z is the depth/scale shake — z amount is a scale-pump per cent
            // (the old Zoom pump, same range) and z frequency a rate multiplier.
            ParamSchema {
                id: "x_amp",
                label: "X amount",
                // × the master Amplitude (0 stills this axis).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 2.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "x_freq",
                label: "X frequency",
                // × the master Frequency.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 4.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "y_amp",
                label: "Y amount",
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 2.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "y_freq",
                label: "Y frequency",
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 4.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "z_amp",
                label: "Z amount",
                // Depth/scale shake, % of scale wobble about natural size —
                // the old Zoom pump's range and meaning (§3.4).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 20.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "z_freq",
                label: "Z frequency",
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 4.0),
                    hard: (Some(0.0), None),
                },
            },
            // The "Motion blur" twirl (SHAKE_GROUPS, T18/K-165): the shake's own
            // motion blur, smeared along the wobble's inter-frame movement and
            // applied to this effect alone. Off by default; the amount is a
            // shutter-like 0..1 fraction (a genuine ratio, so a 0..1 range is
            // the natural unit, P5/K-135).
            ParamSchema {
                id: "motion_blur",
                label: "Motion blur",
                kind: ParamKind::Bool { default: false },
            },
            ParamSchema {
                id: "mb_amount",
                label: "Shutter",
                // 0..1 shutter fraction: how far across the shutter window the
                // wobble is sampled and averaged. 0 is the plain shake (no
                // smear); the resolver also treats motion blur off as no smear,
                // so both are the bit-exact single resample.
                kind: ParamKind::Float {
                    default: 0.5,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "edge",
                label: "Edges",
                // How the resample treats the border the wobble reveals (P3,
                // K-145). Default Mirror (owner, 2026-07-19; was Repeat): the
                // reflected border reads more naturally under the shake's own
                // motion blur than a smeared repeat edge.
                kind: ParamKind::Choice {
                    options: EDGE_OPTIONS,
                    default: 2,
                    dividers_after: CHOICE_UNGROUPED,
                },
            },
            ParamSchema {
                id: "seed",
                label: "Seed",
                kind: ParamKind::Seed,
            },
            MIX_PARAM,
        ],
    },
    // Block glitch (docs/08 §3.12, split out of the old combined Glitch
    // effect by K-107 — one of three now-standalone one-thing effects,
    // alongside Scanlines and Datamosh below). Seeded — category Distortion
    // to match Shake and RGB split, its closest siblings (positional
    // wobble, channel split), not the additive-light Stylise pair (Glow,
    // Flash). Stacking Block glitch → Scanlines, each at Mix 100%,
    // reproduces the old combined Glitch's look bit-for-bit at Intensity 1
    // (each section ran unconditionally there too).
    //
    // Status (shipped): the spec text names most of these without ranges;
    // pinned here, carried over unchanged from the combined effect.
    // Intensity (0–1, the master dial) scales *everything* glitched — grid
    // jitter, displacement, channel offset and slice-repeat odds alike — so
    // it is a genuine single "how glitched" knob and 0 is the bit-exact
    // passthrough. "Rows/columns jitter" is one Block jitter % (of Block
    // size), not separate row/column controls, applied as a per-nominal-
    // block hashed offset to where that block's content is read from — a
    // cheap stand-in for actually moving grid lines (which would need a
    // boundary search a single pointwise pass cannot do), pinned as a
    // deliberate simplification. "Channel-offset toggle or amount" ships as
    // a Float (Channel offset, % diag) — continuous like every other
    // amount-shaped parameter in the catalogue, following RGB split's
    // R/B-offset-from-G shape but with a per-block hashed offset instead of
    // one global vector. Slice repetition ships as a Float 0–100%: the
    // odds (scaled by Intensity) that a given block folds its own content
    // to repeat a short hashed strip instead of a plain positional read.
    // Per-block hashing runs inside the GPU kernel (the block index is a
    // per-pixel quantity, so the hash cannot be a host-precomputed table):
    // WGSL has no 64-bit integer type, so it cannot host Shake's actual
    // splitmix64 lattice. `splitmix32` is a matching-spirit 32-bit sibling
    // added alongside it for exactly this (both CPU and GPU use it, so they
    // agree on the integer hash bit-for-bit; only the fp16 sampling that
    // follows carries the usual small tolerance) — Shake's own
    // splitmix64/value_noise_1d are untouched. "Time-derived tick" (the
    // spec's phrase for per-frame block variation) steps at a fixed,
    // unexposed 8 Hz — chosen so blocks visibly pop rather than blur into
    // continuous noise; no rate parameter is listed in the spec text, so
    // this is pinned as an internal constant.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "block_glitch",
        label: "Block glitch",
        version: 1,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: true,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // The master dial (§1.2): scales every hashed quantity.
                // 0 is the bit-exact passthrough (pinned by test).
                kind: ParamKind::Float {
                    default: 0.35,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "block_size",
                label: "Block size",
                // px@comp (§2.3): a deliberately pixel-scale look.
                kind: ParamKind::Float {
                    default: 24.0,
                    slider: (4.0, 128.0),
                    hard: (Some(2.0), None), // ≥ 2px: never a degenerate grid
                },
            },
            ParamSchema {
                id: "block_jitter",
                label: "Rows/columns jitter",
                // % of Block size (status note above): a hashed offset to
                // where each nominal block's content is read from.
                kind: ParamKind::Float {
                    default: 25.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "block_amount",
                label: "Displacement",
                // % diag (§2.3), the same currency as Blur's Radius/Length.
                kind: ParamKind::Float {
                    default: 3.0,
                    slider: (0.0, 15.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "channel_offset",
                label: "Channel offset",
                // % diag: a per-block hashed RGB split (status note above).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 10.0),
                    hard: (Some(0.0), Some(50.0)),
                },
            },
            ParamSchema {
                id: "slice_repeat",
                label: "Slice repeat",
                // % odds (× Intensity) a block folds to a repeating strip.
                kind: ParamKind::Float {
                    default: 20.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            // Seed sits second-last, immediately before Mix (owner convention
            // for seeded effects: the "which roll of the dice" dial lives at the
            // bottom of the stack of look controls).
            ParamSchema {
                id: "seed",
                label: "Seed",
                kind: ParamKind::Seed,
            },
            MIX_PARAM,
        ],
    },
    // Scanlines (docs/08 §3.12, split out of the old combined Glitch effect
    // by K-107; collapsed to a single Intensity by FX-13/K-147). No hash, no
    // seed — a pointwise periodic darken read straight from the input pixel,
    // never a neighbour, so its ROI is `exact` (tighter than Block glitch's
    // full-frame). Category Distortion, alongside Block glitch and Datamosh.
    // Roll speed's sign is open (either direction); Interlace alternates which
    // half of each scanline period darkens on odd periods, the classic
    // interlaced-field look. Intensity is now the one darken dial (the old
    // separate Darkness param folds into it on load).
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "scanlines",
        label: "Scanlines",
        version: 2,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // The single dial (FX-13, K-147): 0..1 is how dark the dark
                // lines get — 0 is the bit-exact passthrough (pinned by test),
                // 1 takes the dark lines to black. Collapses the old
                // Intensity × Darkness pair into one control; an old project's
                // Darkness folds into this on load (the resolve arm).
                kind: ParamKind::Float {
                    default: 0.35,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "scanline_period",
                label: "Line period",
                // px@comp: the deliberately pixel-scale scanline pitch.
                kind: ParamKind::Float {
                    default: 3.0,
                    slider: (1.0, 20.0),
                    hard: (Some(1.0), None),
                },
            },
            ParamSchema {
                id: "scanline_roll",
                label: "Roll speed",
                // Lines (periods) per second; either direction (K-090).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-30.0, 30.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "scanline_interlace",
                label: "Interlace offset",
                kind: ParamKind::Bool { default: false },
            },
            MIX_PARAM,
        ],
    },
    // Datamosh (docs/08 §3.12, K-104; its own effect since K-107; reworked to
    // a flow-driven melt by K-164/T19). Simulates the compression-glitch look
    // of removing I-frames: the previous picture keeps being dragged along the
    // current frame's motion, so moving regions smear and bloom while static
    // ones stay. Per pixel, a short streamline walk follows the current→
    // previous flow field out of the -1 source neighbour — each step advances
    // by (roughly) one frame of motion, re-sampling the flow so the smear
    // curves with the motion — and the samples along that walk accumulate into
    // a melting prediction that blends over the current frame by Intensity.
    // Displacement sets how many frames of motion the walk reaches; Bloom sets
    // how much of that reach accumulates (a short reset trail near 0, a long
    // melting trail near 1); Reset interval periodically ramps the whole melt
    // back to a clean frame (the simulated I-frame). Reuses Motion blur's flow
    // machinery and its GPU pass/CPU oracle (`FxEngine::datamosh`,
    // `cpu::datamosh`); no new host plumbing. Temporal reach `{0, -1}`,
    // static, exactly the shape Motion blur's own `{0, +1}` has, so
    // `stack_flow_neighbour` reads the match name the same static way.
    // Footage-only: with no -1 neighbour or flow field (a non-footage layer,
    // or a dropped decode) it degrades to a no-op, never a fault. Category
    // Distortion, matching Shake and RGB split (its closest siblings: a
    // seeded positional wobble, a channel split) — but Datamosh itself reads
    // no hash or seed, so `seeded: false`, unlike them.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "datamosh",
        label: "Datamosh",
        version: 3,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            // A streamline of up to 64 bilinear taps (like Motion blur's own
            // streak integral), not the single tap the K-104/K-148 version
            // took — moderate, the same class as Motion blur and Echo.
            cost: CostClass::Moderate,
            // The flow can point anywhere in the frame — the same unbounded-
            // read reasoning Motion blur's own full-frame ROI carries for its
            // flow-directed taps.
            roi: Roi::FullFrame,
            temporal: &[0, -1],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // Blends between the ordinary frame and the moshed one. 0 is
                // the bit-exact passthrough (pinned by test); the hard ceiling
                // is open (K-135/FX-14), so > 1 extrapolates past the moshed
                // frame for a punchier tear. Default 1 (owner, 2026-07-19):
                // below 1 it reads like a lowered Mix, so the full melt is the
                // out-of-the-box look.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "displacement",
                label: "Displacement",
                // Frames of predicted motion the streamline walk reaches
                // (each step advances ~1 frame of flow): 1 is a one-frame
                // prediction, higher reaches further so more smearing
                // accumulates (the P-frame run length between clean reference
                // frames). Open above (K-135); id `displacement` supersedes
                // the K-148 `streak_length`, still read as a fallback.
                kind: ParamKind::Float {
                    default: 4.0,
                    slider: (1.0, 16.0),
                    hard: (Some(1.0), None),
                },
            },
            ParamSchema {
                id: "bloom",
                label: "Bloom",
                // How much of the reach accumulates into the smear: 0 keeps
                // only the nearest step (a short, quickly-resetting trail), 1
                // averages the whole walk evenly (a long melting bloom). A
                // pure 0..1 ratio — the natural unit for a blend weight.
                kind: ParamKind::Float {
                    default: 0.6,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "reset_interval",
                label: "Reset interval",
                // Seconds between simulated I-frames: the melt ramps from a
                // clean frame just after each reset up to full by the next,
                // the accumulating-P-frame look. 0 turns the periodic reset
                // off (a constant melt); the content-driven reset — stills and
                // cuts, where the flow is zero — still fires regardless. In
                // seconds (not frames) because the resolve step is frame-rate-
                // agnostic; a frame-count interval needs the comp frame index
                // threaded through resolve, a deferred broad change (K-148).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 5.0),
                    hard: (Some(0.0), None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Echo / trails (docs/08 §3.13): the montage speed-line staple — the
    // first temporal effect (its window reaches back to previous frames, so
    // the render decodes the layer's source at those offsets). v1 status,
    // pinned here: echoes are spaced one comp frame apart (a Spacing control
    // is a later refinement), so the window reaches back Echoes frames, up to
    // 16 (the static trait cap, raised from 8 by FX-17/K-149). Each echo k is
    // at offset -k with intensity Decay^k, a geometric trail. Mode chooses how
    // each echo combines into the trail — the standard compositing blend modes
    // (default Screen, FX-17/K-149) plus the echo-specific Behind (ghosting)
    // and Max (lighten). Cheap and full-frame (it reads whole neighbour
    // frames). Operates on the layer's *source* frames, not the upstream
    // stack's output at those times (full temporal stacking is later) — so it
    // echoes the footage, placed by the layer's own transform like any effect
    // output.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "echo",
        label: "Echo",
        version: 2,
        category: FxCategory::Temporal,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::FullFrame,
            temporal: &[
                0, -1, -2, -3, -4, -5, -6, -7, -8, -9, -10, -11, -12, -13, -14, -15, -16,
            ],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "echoes",
                label: "Echoes",
                // Count of trailing frames; each is one comp frame further
                // back (v1 fixed spacing). Capped at the 16-frame window
                // (FX-17/K-149, raised from 8).
                kind: ParamKind::Float {
                    default: 4.0,
                    slider: (1.0, 16.0),
                    hard: (Some(1.0), Some(16.0)),
                },
            },
            ParamSchema {
                id: "decay",
                label: "Decay",
                // Per-echo intensity falloff: echo k has intensity decay^k.
                kind: ParamKind::Float {
                    default: 0.6,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "mode",
                label: "Mode",
                // Two effect-only compositing ORDERS first, then a divider (T21),
                // then the order-independent light-combine blend modes. Behind
                // draws each echo behind the trail (ghosting); In front over it
                // (the old "Normal"). Max is gone — it was just Lighten. The
                // HSL / burn / dodge modes a layer offers are omitted here: they
                // are ill-defined on a premultiplied light trail (see §3.13 Open
                // questions). Default is Screen (FX-17/K-149). Pre-release, no
                // migration: old stored indices simply re-map.
                kind: ParamKind::Choice {
                    options: &[
                        "Behind",
                        "In front",
                        "Add",
                        "Screen",
                        "Multiply",
                        "Overlay",
                        "Soft light",
                        "Hard light",
                        "Lighten",
                        "Darken",
                        "Difference",
                        "Exclusion",
                        "Subtract",
                        "Divide",
                    ],
                    default: 3,           // Screen
                    dividers_after: &[1], // divider after In front
                },
            },
            MIX_PARAM,
        ],
    },
    // Posterize time (docs/08 §3.25, docs/impl/temporal-rerender.md): a
    // temporal resample that holds its input on a coarser frame-rate grid, for
    // the choppy stop-motion look. NOT a per-pixel op — it changes *what time*
    // the layers it covers render at, so it is detected and executed at the
    // frame-orchestration layer (the adjustment re-render seam in `draws`/`gpu`
    // walk shared with export), never in `run_ops`; `resolve_stack`
    // deliberately has no arm for it, so it resolves to nothing. Category
    // Temporal, cheap (one render at the held time — often the SAME held time
    // across many frames). Scope chooses adjustment behaviour (Everything below,
    // the owner's global pass) or a per-layer time hold (This layer's effects).
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "posterize_time",
        label: "Posterize time",
        version: 1,
        category: FxCategory::Temporal,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            // It re-renders the composite below at a held time; no per-pixel ROI
            // applies. Full-frame is the safe static declaration.
            roi: Roi::FullFrame,
            // The held frame is the frame the decode already produced (footage
            // is held, docs/impl/temporal-rerender.md §2), so no neighbour
            // window is requested — the decode planner is never re-entered.
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "rate",
                label: "Frame rate",
                // The posterised grid in fps: the animation updates only this
                // many times a second. Default 12 (the classic on-twos look).
                kind: ParamKind::Float {
                    default: 12.0,
                    slider: (1.0, 60.0),
                    hard: (Some(0.01), None),
                },
            },
            ParamSchema {
                id: "phase",
                label: "Phase",
                // Comp seconds: shifts where the steps land, so the hold can be
                // aligned to a beat. 0 snaps to the comp's own zero.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1.0, 1.0),
                    hard: (None, None),
                },
            },
            // The Scope choice was removed (owner, 2026-07-19 / K-166): the
            // reach is implied by the carrier now — a plain layer holds its own
            // source and effect stack, an adjustment layer holds everything
            // below (that IS its effect input). A stored `scope` on an old
            // instance is simply unread.
        ],
    },
    // Accumulation motion blur (docs/08 §3.26, docs/impl/temporal-rerender.md):
    // the expensive, correct motion blur — it renders the WHOLE scene below it
    // several times at in-between moments and averages the finished frames, so
    // footage motion, animated effects, depth passes and everything else are all
    // correct per sample (no blurred-depth artefact). NOT a per-pixel op: like
    // Posterize time it changes *what time the layers below it render at*, so it
    // is detected and executed at the frame-orchestration layer (the adjustment
    // re-render seam shared with export), never in
    // `run_ops`; `resolve_stack` deliberately has no arm for it, so it resolves to
    // nothing. An **adjustment** effect (docs/08 §1.5): it processes everything
    // below, so "apply to all layers" is just the effect on a full-frame
    // adjustment layer. Category Temporal, cost Heavy (≈ N× a full comp render).
    // The sub-frame sample times reuse `MotionBlur::sample_offsets` (the same
    // centred shutter maths per-layer motion blur uses), so `τ_k = t + off_k·dt`;
    // the N finished below-composites are averaged by the hardware
    // additive-at-1/N pass (`Compositor::accumulate`). Mix blends the averaged
    // result against the frame-time composite. Boundaries as Posterize (K-125):
    // temporal effects inside the sampled below-stack hold to stills.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "accumulation_mb",
        // The user-facing motion blur (docs/08 §3.26): the accumulation kind is
        // the correct, whole-scene one, so it takes the plain name. The
        // optical-flow effect (match_name "motion_blur") is "Fast motion blur".
        label: "Motion blur",
        version: 1,
        category: FxCategory::Temporal,
        traits: EffectTraits {
            cost: CostClass::Heavy,
            roi: Roi::FullFrame,
            // The below-stack is re-rendered at each sub-frame time from the SAME
            // held decode (footage is held, docs/impl/temporal-rerender.md §2), so
            // no neighbour window is requested — the decode planner is never
            // re-entered.
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "samples",
                label: "Samples",
                // Sub-frame renders of the scene below across the open shutter
                // (≥ 2 to blur). The schema has no integer kind, so this is a
                // Float row (as Echo's Echoes and flow Motion blur's Samples are);
                // the detector rounds and clamps. Heavy — each sample is a full
                // comp re-render — so a tasteful default of 8.
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (2.0, 32.0),
                    hard: (Some(2.0), Some(64.0)),
                },
            },
            ParamSchema {
                id: "shutter_angle",
                label: "Shutter angle",
                // Degrees: the fraction of the frame interval the shutter is open
                // is shutter ÷ 360, so the samples span that much of the motion.
                // 180° (half a frame) is the film-standard look.
                kind: ParamKind::Float {
                    default: 180.0,
                    slider: (0.0, 720.0),
                    hard: (Some(0.0), Some(720.0)),
                },
            },
            ParamSchema {
                id: "shutter_phase",
                label: "Shutter phase",
                // Degrees: where the open interval sits relative to the frame
                // time. -90 centres the samples on the frame (pairing with a 180
                // angle to open a quarter-frame either side), the AE default.
                kind: ParamKind::Float {
                    default: -90.0,
                    slider: (-360.0, 360.0),
                    hard: (Some(-720.0), Some(720.0)),
                },
            },
            ParamSchema {
                id: "force_all",
                label: "Force on all layers",
                // Force per-layer motion blur (K-120) on every layer during the
                // sub-frame sample renders — the shutter above stands in for the
                // comp master and each layer's own switch, without mutating the
                // comp. So one effect blurs every moving layer without toggling
                // each one; each accumulation sample is itself transform-smeared,
                // smoothing the result at lower sample counts. Off by default.
                kind: ParamKind::Bool { default: false },
            },
            MIX_PARAM,
        ],
    },
    // Motion blur (flow) / RSMB-class (docs/08 §3.2): synthesised motion blur
    // from real optical flow. Game capture has no natural blur; this estimates
    // the per-pixel motion between the current source frame and the next
    // (§3.1's flow engine, run in the decode worker where both frames live),
    // then smears each pixel along its own motion vector so fast-moving areas
    // streak along their actual motion. The second temporal effect: its window
    // is {0, 1} (current + one frame ahead), and the render fetches the +1
    // neighbour through the same machinery Echo added — but where Echo reads
    // the neighbour *pixels*, motion blur reads a *flow field* the decode
    // worker computes from them and hands the kernel as a texture.
    //
    // Status (v1, pinned here): the §3.2 parameter set is trimmed to
    // Shutter angle, Samples and the host Mix. Blur length in pixels =
    // motion vector × (shutter ÷ 360), integrated as a centred box streak of
    // Samples evenly spaced bilinear taps (the same line-integral shape as
    // Directional blur, but per-pixel-directed by the flow). Vector source is
    // Flow only (Auto's transform-derivative path and the engine-motion-blur
    // interaction guard follow); Amount (post-shutter vector scale) and the
    // Quality/adaptive-tap-count control are deferred — Samples is a fixed
    // per-frame tap count so the CPU and GPU integrate identically. Zero
    // motion or a zero shutter is a bit-exact passthrough (pinned by test).
    // Edges clamp (the flow sampler's own rule), so a full-frame smear never
    // darkens the border. Cost heavy, full-frame ROI; footage layers only,
    // exactly like Echo (adjustment-layer temporal effects follow).
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "motion_blur",
        // The optical-flow, footage-internal blur (docs/08 §3.2): "Fast" because
        // it is a single-pass per-pixel smear, distinct from the whole-scene,
        // re-rendering "Motion blur" (accumulation, §3.26).
        label: "Fast motion blur",
        version: 1,
        category: FxCategory::Temporal,
        traits: EffectTraits {
            cost: CostClass::Heavy,
            roi: Roi::FullFrame,
            // Current frame + one ahead: the flow engine brackets the motion
            // between them. The +1 neighbour is fetched by the same decode
            // planner Echo's negative offsets use.
            temporal: &[0, 1],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "shutter_angle",
                label: "Shutter angle",
                // Degrees (§3.2: 0–720, default 180): the fraction of the
                // frame interval the shutter is open, so the streak length is
                // shutter ÷ 360 of the inter-frame motion. 180° = half the
                // motion, the film-standard look.
                kind: ParamKind::Float {
                    default: 180.0,
                    slider: (0.0, 720.0),
                    hard: (Some(0.0), Some(720.0)),
                },
            },
            ParamSchema {
                id: "samples",
                label: "Samples",
                // Taps along the streak (§3.2). The spec's integer, carried
                // as a Float row (the schema has no integer kind — Echo's
                // Echoes does the same); the resolver rounds and clamps. More
                // taps smooth a long streak; fewer are cheaper.
                kind: ParamKind::Float {
                    default: 16.0,
                    slider: (8.0, 32.0),
                    hard: (Some(2.0), Some(64.0)),
                },
            },
            ParamSchema {
                id: "view",
                label: "View",
                // Diagnostic outputs (FX-19): the blurred picture, the flow
                // vectors colour-coded (red +x, green +y), or the confidence as
                // greyscale (white = trusted, black = suspect — where the streak
                // fades out). Rendered by default.
                kind: ParamKind::Choice {
                    options: &["Rendered", "Motion vectors", "Confidence"],
                    default: 0,
                    dividers_after: CHOICE_UNGROUPED,
                },
            },
            MIX_PARAM,
        ],
    },
    // Matte key (docs/08 §3.21, K-121/K-154): a Keylight-style colour-difference
    // keyer — a proper greenscreen keyer, expanded from the K-121 chroma-distance
    // key. It works on straight (unpremultiplied) colour (§2.2, the wrap fused into
    // the kernel like Saturation's). The screen colour's largest channel is the
    // primary screen axis; a pixel's primary-minus-(balance-weighted)-secondary
    // difference, normalised by the screen colour's own, drives the screen matte
    // (Screen gain scales the fall-off, Screen balance weights the two secondaries).
    // Clip black/white/rollback tidy the matte's ends, despill drains screen tint
    // from kept pixels, and the Replace method recolours where spill was removed.
    // Every step is clamp/min/max/lerp — continuous, so the §1.6 ULP oracle holds
    // (cost class `cheap`). Category Utility, beside Transform. The default green +
    // 100 % gain visibly keys a typical green screen ("drop it on and it works",
    // §1.2); there is no neutral no-op default (Mix 0 is the identity). The spatial
    // Keylight controls (screen pre-blur / shrink-grow / softness / despot) and the
    // inside-outside garbage masks, colour correction and source crops are deferred
    // follow-ups (§3.21 status). Migration: a project saved before K-154 keeps its
    // stored Screen colour and Spill; the superseded Tolerance/Softness go unread.
    EffectSchema {
        groups: &[ParamGroup {
            label: "Screen matte",
            params: &[
                "clip_black",
                "clip_white",
                "clip_rollback",
                "replace_method",
                "replace_colour",
            ],
            collapsed: true,
            visible_when: None,
        }],
        enabled_when: &[],
        match_name: "matte_key",
        label: "Matte key",
        version: 2,
        category: FxCategory::Utility,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: keying/despill works on straight colour
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "view",
                label: "View",
                // Final result (the keyed picture), Screen matte (the alpha as
                // greyscale), or Status (a continuous heat of the matte). Default
                // Final so the effect keys the moment it is dropped on.
                kind: ParamKind::Choice {
                    options: &["Final result", "Screen matte", "Status"],
                    default: 0,
                    dividers_after: CHOICE_UNGROUPED,
                },
            },
            ParamSchema {
                id: "key",
                label: "Screen colour",
                // Scene-linear RGBA; alpha ignored. Default a saturated green, the
                // greenscreen the effect exists to remove. Its largest channel
                // picks the primary screen axis (so a blue screen keys too).
                kind: ParamKind::Colour {
                    default: [0.0, 0.6, 0.0, 1.0],
                    range: (0.0, 4.0),
                },
            },
            ParamSchema {
                id: "screen_gain",
                label: "Screen gain",
                // Per cent → a 0.. multiplier on the matte fall-off. 100 % keys
                // the exact screen colour to zero; higher keys more aggressively.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 200.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "screen_balance",
                label: "Screen balance",
                // Per cent → 0..1: how the two non-screen channels are weighted
                // into the reference (0 = their min, 100 = their max, 50 = mean).
                kind: ParamKind::Float {
                    default: 50.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "despill_bias",
                label: "Despill bias",
                // Scene-linear RGBA; shifts the reference the despill clamps the
                // primary down to. A neutral grey is a no-op.
                kind: ParamKind::Colour {
                    default: [0.5, 0.5, 0.5, 1.0],
                    range: (0.0, 4.0),
                },
            },
            ParamSchema {
                id: "alpha_bias",
                label: "Alpha bias",
                // Scene-linear RGBA; shifts what colour counts as neutral for the
                // screen matte. A neutral grey is a no-op.
                kind: ParamKind::Colour {
                    default: [0.5, 0.5, 0.5, 1.0],
                    range: (0.0, 4.0),
                },
            },
            ParamSchema {
                id: "spill",
                label: "Despill amount",
                // Per cent of the primary's screen excess drained from kept pixels
                // (defaults full-on, Keylight-like; an older instance keeps its
                // stored value, an even older one without the param reads 0).
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            // Screen matte twirl (the K-145 collapsible group above).
            ParamSchema {
                id: "clip_black",
                label: "Clip black",
                // Per cent → 0..1: matte at/below this maps to 0 (fully keyed),
                // cleaning residual grey out of the background.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "clip_white",
                label: "Clip white",
                // Per cent → 0..1: matte at/above this maps to 1 (fully kept),
                // filling holes in the foreground.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "clip_rollback",
                label: "Clip rollback",
                // Per cent → 0..1: eases the clips back toward the un-clipped
                // matte, recovering fine edge detail (0 = full clip, the default).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "replace_method",
                label: "Replace method",
                // How despilled areas are recoloured. Default Soft colour, as
                // Keylight (it settles into shading rather than a flat patch).
                kind: ParamKind::Choice {
                    options: &["Source", "Hard colour", "Soft colour", "None"],
                    default: 2,
                    dividers_after: CHOICE_UNGROUPED,
                },
            },
            ParamSchema {
                id: "replace_colour",
                label: "Replace colour",
                // Scene-linear RGBA used by the Hard/Soft replace methods; a
                // neutral grey desaturates spill edges without a colour cast.
                kind: ParamKind::Colour {
                    default: [0.5, 0.5, 0.5, 1.0],
                    range: (0.0, 4.0),
                },
            },
            MIX_PARAM,
        ],
    },
    // Invert (docs/08 §3.23, K-126): a simple colour inverse — out.rgb = 1 − in.rgb
    // per channel, alpha kept. Because 1 − c is affine (not a pure scale) it does
    // NOT commute with premultiplied alpha, so premultiplied: false: the host wraps
    // unpremultiply → invert → re-premultiply (fused into the kernel and the CPU
    // reference), exactly like Contrast and Gamma, so matte edges do not fringe.
    // The inverse is taken in the compositor's scene-linear fp16 working space (the
    // owner's "simple inverse"), so HDR values above 1 invert to honest negatives,
    // never clipped (§2.1). Continuous everywhere, so the §1.6 oracle holds. There
    // is no neutral no-op default — invert always inverts (§1.2) — so only Mix 0 is
    // the identity. Category Colour, beside its grade siblings.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "invert",
        label: "Invert",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: 1 − c is affine, so it shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[MIX_PARAM],
    },
    // Tint (docs/08 §3.24, K-127): a luminance duotone / gradient map. Two colour
    // params — "Map black to" (default black) and "Map white to" (default white) —
    // and out.rgb = black.rgb + (white.rgb − black.rgb) · luma(in.rgb) with Rec.709
    // luma on the unpremultiplied linear colour, alpha kept. A luma-driven colour
    // remap does not commute with premultiplied alpha, so premultiplied: false: the
    // host wraps unpremultiply → map → re-premultiply (fused into the kernel and the
    // CPU reference), exactly like Contrast and Gamma, so matte edges do not fringe.
    // The default black→black / white→white maps every pixel to its own luma — a
    // greyscale, a visible tasteful default (§1.2), not a no-op — so only Mix 0 is
    // the identity. Continuous everywhere, so the §1.6 oracle holds. Category Colour,
    // beside its grade siblings.
    EffectSchema {
        groups: &[],
        enabled_when: &[],
        match_name: "tint",
        label: "Tint",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: a colour remap shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "black",
                label: "Map black to",
                // Scene-linear RGBA (alpha ignored): the colour dark input maps to.
                kind: ParamKind::Colour {
                    default: [0.0, 0.0, 0.0, 1.0],
                    range: (0.0, 4.0),
                },
            },
            ParamSchema {
                id: "white",
                label: "Map white to",
                // Scene-linear RGBA (alpha ignored): the colour bright input maps to.
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.0, 4.0),
                },
            },
            MIX_PARAM,
        ],
    },
    // Lens flare (docs/08 §3.27, docs/impl/lens-flare.md, K-256; panel
    // reshape and source modes K-257): ghosts ray-traced through a real lens
    // prescription, the starburst from the aperture's Fourier diffraction —
    // the one effect that owns a render pass. Heavy cost; full-frame ROI;
    // premultiplied (an additive light overlay, the Glow shape); not seeded.
    // Intensity 0 and Mix 0 are bit-exact passthroughs (pinned by test).
    // GPU-only per K-256: the CPU degradation rung renders it as a labelled
    // no-op, and the §1.6 oracle is staged (trace at mean/quantile bounds,
    // frame at a perceptual bound). Parameter order is the owner's panel
    // design: the light point pair, the three headline dials, the coating
    // character above the lens it colours, then the collapsed detail groups,
    // the Source mode with its conditional matte rows, and Quality last.
    EffectSchema {
        groups: &[
            ParamGroup {
                label: "Lens options",
                params: &[
                    "focus",
                    "anamorphic",
                    "blades",
                    "aperture_rotation",
                    "coating",
                    "roundness",
                    "aperture_softness",
                ],
                collapsed: true,
                visible_when: None,
            },
            ParamGroup {
                label: "Flare options",
                params: &[
                    "ghost_intensity",
                    "ghost_softness",
                    "max_ghosts",
                    "detail",
                    "dispersion",
                    "starburst_intensity",
                    "scale",
                ],
                collapsed: true,
                visible_when: None,
            },
            // The source-colour toggle: headerless, and shown for BOTH the
            // source modes that HAVE a source colour to take (Matte, and
            // Lights when it lands) — K-259.
            ParamGroup {
                label: "",
                params: &["use_source_colour"],
                collapsed: false,
                visible_when: Some(("source_type", &[1, 2])),
            },
            // The matte rows: headerless (empty label renders them in place,
            // no twirl), shown only while Source type is Matte.
            ParamGroup {
                label: "",
                params: &["matte", "threshold", "threshold_softness"],
                collapsed: false,
                visible_when: Some(("source_type", &[1])),
            },
        ],
        enabled_when: &[],
        match_name: "lens_flare",
        label: "Lens flare",
        version: 5,
        category: FxCategory::Stylise,
        traits: EffectTraits {
            cost: CostClass::Heavy,
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "light_x",
                label: "Light x",
                // px@comp (K-260: point parameters are PIXELS, the
                // Transform-anchor convention — never % of frame). Open both
                // sides: an off-frame light keeps flaring. The schema default
                // is nominal 1080p; `instantiate_for_raster` centres a fresh
                // instance on the actual comp's upper-left third (§1.2).
                // Pairs with light_y into one point row (docs/07 §6.1).
                kind: ParamKind::Float {
                    default: 640.0,
                    slider: (0.0, 3840.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "light_y",
                label: "Light y",
                kind: ParamKind::Float {
                    default: 360.0,
                    slider: (0.0, 2160.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "source_width",
                label: "Source width",
                // px@comp like the position (K-260), and the HALF-width of the
                // emitting area: 0 — the default — is the point source the
                // effect has always had, and anything larger makes it an AREA
                // light whose ghosts take the shape of the source rather than
                // of a point (K-355). Pairs with source_height into one row.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 400.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "source_height",
                label: "Source height",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 400.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // Master gain on everything the effect adds; 0 is the
                // neutral point (bit-exact passthrough, pinned by test).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 4.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "fstop",
                label: "F-stop",
                // Wider (smaller number) grows the ghost discs and softens
                // the starburst ringing.
                kind: ParamKind::Float {
                    default: 2.8,
                    slider: (1.0, 22.0),
                    hard: (Some(0.7), Some(32.0)),
                },
            },
            ParamSchema {
                id: "lens_model",
                label: "Lens",
                // The embedded prescription library (K-261, curated to
                // twenty K-264): real lenses, transcribed patent data,
                // chosen for maximally different flare characters. Sorted
                // by name; the default is the Master Prime 50 (the
                // reference cine prime the effect was tuned against). A
                // .lens file on `lens_file` overrides this pick entirely.
                kind: ParamKind::Choice {
                    options: &crate::fx::lens_library::LENS_OPTIONS,
                    default: 16,
                    dividers_after: &[],
                },
            },
            ParamSchema {
                id: "lens_file",
                label: "Lens file",
                // A user's own .lens prescription (K-264, the LUT File
                // pattern): set, it overrides the Lens pick entirely — the
                // twenty bundled lenses are a curated palette, and this is
                // the door to everything else (the FlareSim /
                // PhotonsToPhotos Optical Bench format the parser already
                // reads). Unset, missing on disk or unparsable degrades to
                // the picked lens — a labelled fallback, never a fault.
                kind: ParamKind::File {
                    filter: &["lens"],
                    filter_name: "Lens prescription",
                },
            },
            // --- Lens options group ---
            ParamSchema {
                id: "focus",
                label: "Focus (m)",
                // Focus distance in metres (K-260): shifts the sensor from
                // its calibrated infinity position by the thin-lens image
                // shift, changing the whole flare's shape — real flares
                // breathe with focus. Large values are infinity.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.5, 100.0),
                    hard: (Some(0.2), None),
                },
            },
            ParamSchema {
                id: "anamorphic",
                label: "Anamorphic squeeze",
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (1.0, 2.0),
                    hard: (Some(0.5), Some(3.0)),
                },
            },
            ParamSchema {
                id: "blades",
                label: "Blades",
                // Iris blade count: the starburst's spike count and the
                // ghost disc shape.
                kind: ParamKind::Int {
                    default: 8,
                    slider: (3, 16),
                    hard: (Some(3), Some(16)),
                },
            },
            ParamSchema {
                id: "aperture_rotation",
                label: "Rotation",
                // Degrees on a dial: turning an iris is the gesture, not typing
                // a number at it.
                kind: ParamKind::Angle {
                    default: 0.0,
                    dial_step: 15.0,
                },
            },
            ParamSchema {
                id: "coating",
                label: "Coating",
                // 0 = uncoated (bright neutral ghosts), 1 = full quarter-wave
                // coating interference (dim, colour-cast ghosts).
                kind: ParamKind::Float {
                    default: 0.75,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "roundness",
                label: "Roundness",
                kind: ParamKind::Float {
                    default: 0.15,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "aperture_softness",
                label: "Softness",
                // Softens the iris edge, and with it every ghost's rim.
                kind: ParamKind::Float {
                    default: 0.05,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            // --- Flare options group ---
            ParamSchema {
                id: "ghost_intensity",
                label: "Ghost intensity",
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 4.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "ghost_softness",
                label: "Ghost softness",
                // Box-blur radius as % of the frame diagonal (K-261,
                // FlareSim's Ghost Blur): a touch of out-of-focus softness.
                // 0.02 by default (owner-set, K-264) — with the
                // vertex-smoothed density and the multisampled raster the
                // geometry no longer needs hiding, so the default is taste,
                // not cover, and 0 stays a usable, clean setting.
                kind: ParamKind::Float {
                    default: 0.02,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(2.0)),
                },
            },
            ParamSchema {
                id: "max_ghosts",
                label: "Max ghosts",
                // The brightest-ranked ghosts survive the cap.
                kind: ParamKind::Int {
                    default: 60,
                    slider: (0, 150),
                    hard: (Some(0), Some(200)),
                },
            },
            ParamSchema {
                id: "detail",
                label: "Detail",
                // Ray-budget multiplier on the Quality tier's pupil grid
                // (K-265, owner-asked): the tiers pick a sensible base and
                // this dial hands the trade to the user — a lens whose
                // ghost rims still show their cells buys more rays without
                // jumping a whole tier, a preview buys fewer. Frame-time,
                // never rebakes; 1 is the tier as shipped.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.25, 2.0),
                    hard: (Some(0.25), Some(4.0)),
                },
            },
            ParamSchema {
                id: "dispersion",
                label: "Dispersion",
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 2.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "starburst_intensity",
                label: "Starburst intensity",
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 4.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "scale",
                label: "Scale",
                // Scales the WHOLE flare about the optical centre — ghost
                // train and starburst together (owner pass 2).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.1, 4.0),
                    hard: (Some(0.05), Some(20.0)),
                },
            },
            // --- Source ---
            ParamSchema {
                id: "source_type",
                label: "Source",
                // Where the light comes from. Lights is prepared for light
                // layers (K-257): it resolves as Manual until they land.
                kind: ParamKind::Choice {
                    options: &["Manual light", "Matte", "Lights"],
                    default: 0,
                    dividers_after: &[],
                },
            },
            ParamSchema {
                id: "light_tint",
                label: "Light tint",
                // Multiplies every light's colour, in every source mode
                // (K-259): in Manual it IS the flare's colour; in Matte it
                // tints whatever the sources contribute. Scene-linear, and
                // open above 1 so an HDR tint can push a flare hotter.
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.0, 4.0),
                },
            },
            ParamSchema {
                id: "use_source_colour",
                label: "Use source colour",
                // On: a detected source's own colour tints its flare (a warm
                // practical flares warm). Off: every source flares white
                // through Light tint alone, which is what a matte used purely
                // as a *position* mask wants (K-259).
                kind: ParamKind::Bool { default: true },
            },
            ParamSchema {
                id: "matte",
                label: "Matte layer",
                // The layer whose brightest sources spawn the flares (impl
                // note §6); unset is a labelled no-flare, never a fault —
                // the File/Layer no-op convention (§1.2). `self_default`
                // (K-288): a fresh flare points at its OWN layer, because
                // "flare the lights in this picture" is what asking for a
                // matte source nearly always means — and on an adjustment
                // layer that reads the composite of everything below, so
                // the effect works there without hunting for another layer
                // to point at.
                kind: ParamKind::Layer { self_default: true },
            },
            ParamSchema {
                id: "threshold",
                label: "Threshold",
                // Linear luma at/above which a detected source flares fully.
                // Slider normalised 0–1 (typing goes above; open ceiling).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "threshold_softness",
                label: "Threshold softness",
                // Half-width of the soft gate around the threshold.
                kind: ParamKind::Float {
                    default: 0.25,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "quality",
                label: "Quality",
                // The ray-grid density and traced wavelength count; Draft
                // renders the flare buffer at half resolution.
                kind: ParamKind::Choice {
                    options: &["Draft", "Normal", "High", "Ultra"],
                    default: 1,
                    dividers_after: &[],
                },
            },
            ParamSchema {
                id: "blend",
                label: "Blend",
                // How the flare element combines with the layer under it
                // (K-289, replacing K-258's Transparent/Black Background
                // pair). The curated light-combine set Echo offers, for the
                // same reason (T21): the HSL / burn / dodge modes are
                // ill-defined on a premultiplied light overlay. Normal heads
                // the list, then a divider, because it is the one mode that
                // REPLACES the layer — the flare on its own opaque black,
                // which is what Background = Black existed to export.
                // Default Add: the behaviour every flare had before this
                // menu, so nothing anyone had built moves.
                kind: ParamKind::Choice {
                    options: crate::fx::lens_flare::BLEND_OPTIONS,
                    default: crate::fx::lens_flare::BLEND_ADD,
                    dividers_after: &[0],
                },
            },
            MIX_PARAM,
        ],
    },
];

/// Look a schema up by its match name.
pub fn schema(match_name: &str) -> Option<&'static EffectSchema> {
    BUILTINS.iter().find(|s| s.match_name == match_name)
}

/// A fresh random seed value — the per-instance Seed default (docs/08
/// §3.4) and the Effect Controls "reseed" button (§2.4) both draw from
/// here. Taken from a new UUID's random tail, so it needs no extra
/// dependency; the value becomes stored project data the moment it is
/// chosen, so evaluation determinism (§2.4) is untouched.
pub fn fresh_seed() -> u32 {
    let b = uuid::Uuid::now_v7().into_bytes();
    u32::from_le_bytes([b[12], b[13], b[14], b[15]])
}

/// A new instance of a built-in, carrying the declared defaults.
/// [`instantiate`], then centre any raster-anchored defaults on the target
/// raster (owner T23): the Transform effect's Anchor and Position default to
/// the raster's centre so a fresh instance rotates and scales about the
/// middle, not the 0,0 corner — a schema constant cannot know the raster, so
/// the apply site passes it. Every UI apply path calls this; plain
/// [`instantiate`] keeps the pure schema defaults (tests, presets).
pub fn instantiate_for_raster(match_name: &str, w: f64, h: f64) -> Option<EffectInstance> {
    let mut inst = instantiate(match_name)?;
    if match_name == "transform" {
        for p in &mut inst.params {
            let v = match p.id.as_str() {
                "anchor_x" | "position_x" => w * 0.5,
                "anchor_y" | "position_y" => h * 0.5,
                _ => continue,
            };
            p.value = EffectValue::Float(Property::fixed(v));
        }
    }
    // The flare's light is px@comp (K-260), so its tasteful default — the
    // upper-left third (§1.2) — needs the actual raster.
    if match_name == "lens_flare" {
        for p in &mut inst.params {
            let v = match p.id.as_str() {
                "light_x" => w * 0.33,
                "light_y" => h * 0.30,
                _ => continue,
            };
            p.value = EffectValue::Float(Property::fixed(v));
        }
    }
    // Depth of field's Focus point is the same shape of default: a fresh
    // instance should focus on the middle of the frame, which is where the
    // subject usually is and is the only guess that is never absurd. The schema
    // declares (0, 0) because it cannot know the raster; landing focus in the
    // top-left corner is exactly the "drop it on and it already looks right"
    // failure (§1.2).
    if match_name == "dof" {
        for p in &mut inst.params {
            let v = match p.id.as_str() {
                "focus_point_x" => w * 0.5,
                "focus_point_y" => h * 0.5,
                _ => continue,
            };
            p.value = EffectValue::Float(Property::fixed(v));
        }
    }
    Some(inst)
}

/// The value a parameter kind starts at (docs/08 §1.2): what `instantiate`
/// fills a fresh instance with, and what [`backfill_builtin_params`] appends
/// for a parameter the saved instance predates.
pub fn default_param_value(kind: &ParamKind) -> EffectValue {
    match *kind {
        ParamKind::Float { default, .. } => EffectValue::Float(Property::fixed(default)),
        // Int is a display/rounding kind; the value is a Float like any
        // other scalar (see the schema's Int docs).
        ParamKind::Int { default, .. } => EffectValue::Float(Property::fixed(default as f64)),
        // An angle is a number of degrees, so it stores as a plain Float
        // (docs/08 §1.1) — the dial is a control, not a value type, and
        // keyframes and expressions see nothing new.
        ParamKind::Angle { default, .. } => EffectValue::Float(Property::fixed(default)),
        ParamKind::Choice { default, .. } => EffectValue::Choice(default),
        ParamKind::Bool { default } => EffectValue::Bool(default),
        ParamKind::Colour { default, .. } => EffectValue::Colour(default.map(Property::fixed)),
        ParamKind::Seed => EffectValue::Seed(fresh_seed()),
        ParamKind::File { .. } => EffectValue::File(FileParam::empty()),
        // A fresh layer reference is unset (docs/impl/layer-input.md): the
        // effect is a labelled no-op until the owner picks a layer, the same
        // sanctioned exception the File parameter takes to the "no no-op
        // default" rule.
        ParamKind::Layer { .. } => EffectValue::Layer(None),
    }
}

/// Forward-migrate a stack loaded from disk (K-258): a built-in instance
/// saved before its schema grew a parameter simply lacks it, which left the
/// panel drawing a dash and `set_value` refusing the id. Append every
/// missing declared parameter at its default. Never touches present values,
/// unknown effects, or plugin namespaces — a project round-trips untouched
/// unless its schema really did grow.
pub fn backfill_builtin_params(effects: &mut [EffectInstance]) {
    for e in effects.iter_mut() {
        if e.effect.namespace != EffectNamespace::Builtin {
            continue;
        }
        let Some(s) = schema(&e.effect.match_name) else {
            continue;
        };
        migrate_lens_flare_background(e);
        for p in s.params {
            if !e.params.iter().any(|have| have.id == p.id) {
                e.params.push(EffectParam {
                    id: p.id.to_owned(),
                    value: default_param_value(&p.kind),
                    extra: serde_json::Map::new(),
                });
            }
        }
    }
}

/// Carry a saved Lens flare's Background choice over to the Blend menu that
/// replaced it (K-289, superseding K-258). The old parameter had two values:
/// Transparent (the flare added to the layer's own alpha) and Black (the
/// output forced opaque — the flare-element-over-black export). Transparent
/// *is* the new Add, bit for bit, and it was the default, so almost every
/// saved flare needs nothing beyond the ordinary backfill. Black becomes
/// Normal: on the empty layer that option was for, "the flare on opaque
/// black" is what both produce.
///
/// The legacy parameter is dropped once read — the schema no longer declares
/// it, so leaving it would be a row `set_value` refuses and the panel cannot
/// draw. Runs before the backfill appends `blend`, so a project saved with
/// Black never briefly reads as Add.
fn migrate_lens_flare_background(e: &mut EffectInstance) {
    if e.effect.match_name != "lens_flare" {
        return;
    }
    let Some(old) = e.params.iter().position(|p| p.id == "background") else {
        return;
    };
    let was_black = matches!(e.params[old].value, EffectValue::Choice(1));
    e.params.remove(old);
    if e.params.iter().any(|p| p.id == "blend") {
        return;
    }
    e.params.push(EffectParam {
        id: "blend".to_owned(),
        value: EffectValue::Choice(if was_black {
            crate::fx::lens_flare::BLEND_NORMAL
        } else {
            crate::fx::lens_flare::BLEND_ADD
        }),
        extra: serde_json::Map::new(),
    });
}

/// Point every `self_default` Layer parameter in `inst` at `layer` — the
/// layer the effect is being added to (K-288, docs/impl/layer-input.md).
///
/// A schema constant cannot know which layer it will land on, so the apply
/// site passes it, exactly as [`instantiate_for_raster`] passes the raster.
/// Today that is the Lens flare's Matte layer: adding the effect and
/// switching Source to Matte should flare the lights in the picture the
/// effect is already looking at, not sit there doing nothing until a layer
/// is picked. Presets and plain [`instantiate`] leave the reference unset,
/// which stays the labelled no-op it always was.
pub fn point_self_layer_params_at(inst: &mut EffectInstance, layer: uuid::Uuid) {
    let Some(s) = schema(&inst.effect.match_name) else {
        return;
    };
    for p in s.params {
        if !matches!(p.kind, ParamKind::Layer { self_default: true }) {
            continue;
        }
        if let Some(slot) = inst.params.iter_mut().find(|have| have.id == p.id) {
            slot.value = EffectValue::Layer(Some(layer));
        }
    }
}

/// Whether the parameter `id` is editable given what `inst` currently holds —
/// the greyed-row rule of [`EnabledWhen`], evaluated.
///
/// **In plain terms.** Ticking "Use focus point" hands focus over to the point,
/// so the focus *distance* number stops being what decides anything; this
/// answers `false` for it while that tick is on, and the panel draws the row
/// greyed. A parameter with no rule against it is always editable, which is
/// nearly all of them.
///
/// This is the single authority on the question. The panel greys from it, and a
/// write to a disabled parameter is still accepted — greying is an affordance
/// telling you which control is in charge, not a lock. The resolve step
/// implements the real branch independently and never calls this, so the two
/// cannot drift into disagreeing about pixels: at worst a missing rule leaves a
/// live control that does nothing, which is a panel bug, not a render bug.
pub fn param_enabled(inst: &EffectInstance, id: &str) -> bool {
    let Some(s) = schema(&inst.effect.match_name) else {
        // No built-in schema (an OFX or placeholder instance) means no rules,
        // so nothing is greyed.
        return true;
    };
    s.enabled_when
        .iter()
        .filter(|rule| rule.param == id)
        .all(|rule| {
            // A rule naming a parameter the instance does not carry cannot be
            // judged, so it does not grey anything: an older instance that
            // predates the deciding parameter stays fully editable rather than
            // locking a row it can never unlock (the `fill_missing_params`
            // trap, from the other side).
            let Some(value) = inst.param(rule.on) else {
                return true;
            };
            match (rule.cond, value) {
                (EnabledCond::BoolIs(want), EffectValue::Bool(got)) => *got == want,
                (EnabledCond::ChoiceIs(want), EffectValue::Choice(got)) => *got == want,
                (EnabledCond::ChoiceIsNot(no), EffectValue::Choice(got)) => *got != no,
                (EnabledCond::LayerSet, EffectValue::Layer(layer)) => layer.is_some(),
                // A rule pointed at the wrong kind of parameter is a schema
                // mistake, not a reason to grey a row the owner can then never
                // reach. `every_enablement_rule_names_a_parameter_of_its_kind`
                // fails the build for it instead.
                _ => true,
            }
        })
}

pub fn instantiate(match_name: &str) -> Option<EffectInstance> {
    let s = schema(match_name)?;
    Some(EffectInstance {
        id: uuid::Uuid::now_v7(),
        effect: EffectKey {
            namespace: EffectNamespace::Builtin,
            match_name: s.match_name.to_owned(),
            version: s.version,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: s
            .params
            .iter()
            .map(|p| EffectParam {
                id: p.id.to_owned(),
                value: default_param_value(&p.kind),
                extra: serde_json::Map::new(),
            })
            .collect(),
        sample_temporally: true,
        custom_name: None,
        extra: serde_json::Map::new(),
    })
}
