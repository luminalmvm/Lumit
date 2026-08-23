//! Lens flare (docs/08 §3.27, docs/impl/lens-flare.md, K-256): ghosts traced
//! through a real lens prescription, and the starburst from the aperture's
//! Fourier diffraction.
//!
//! **In plain terms.** The largest declaration in the catalogue — fifty-odd rows,
//! twenty of them one per piece of glass — and, unusually, almost none of it is
//! arithmetic. What the flare *does* is decided by a **bake**: the prescription
//! is parsed, every ghost path is ray-probed and ranked, and the starburst is
//! transformed, all from these numbers, and all of it lives in
//! [`crate::fx::lens_flare`] rather than here. This file is the declaration and
//! the one short step that turns the panel's rows into
//! [`LensFlareParams`](crate::fx::lens_flare::LensFlareParams) — the flat bundle
//! the bake, the CPU reference and the WGSL kernels all read, so the picture is
//! decided in one place (K-031).
//!
//! Three things arrive from outside the bag, and each has its own reason:
//!
//! - **The Matte source and the `.lens` file** are a picture and a file, so only
//!   the render knows whether either turned up, and each comes beside the op on
//!   its own list (K-387). The prescription is this effect's `AuxKind::LensFile`.
//!   The Matte is no longer the flare's private business: since K-395 it is the
//!   Matte row every effect has, arriving on the one matte carriage, and the
//!   flare is simply one of the four that claim it inside their own maths —
//!   here it decides *where the light sources are*, not how strong the flare is.
//! - **Lights mode's sources** (K-360) are the comp's own Light layers at this
//!   frame, which is not a parameter anyone could slide: they are read at resolve
//!   time through the expression context — which already carries the document,
//!   the comp and the time — and pushed into the bag as derived values
//!   ([`LensFlareDef::resolve_derived`], K-385).
//!
//! There is no CPU reference through the single-buffer dispatcher: the flare owns
//! a render pass and its baked tables, and neither reaches a single `&mut [f32]`.
//! So `apply_cpu` keeps its identity default — the passthrough the old
//! `Resolved::LensFlare` arm of `cpu::apply` was (the K-114 LUT precedent). The
//! §1.6 oracle is [`crate::fx::lens_flare::cpu_flare`]/`cpu_combine`, exercised
//! directly from the lumit-gpu tests.

use crate::fx::lens_flare as lf;
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, ResolveCx, Value,
};
use crate::model::EffectValue;
use lumit_fx_macros::Effect;

/// The panel's twirls and conditional runs (K-145, K-257, K-371), in the owner's
/// panel order: the lens's own shaping, then one headerless row per glass
/// element (drawn only when the lens in play *has* that element), then the flare
/// detail, then the rows that answer to the Source mode.
pub const LENS_FLARE_GROUPS: &[ParamGroup] = &[
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
        visible_when_lens_elements: None,
    },
    // One coating row per glass element (K-371), each headerless and
    // drawn only when the lens in play has that element: four rows on
    // the Tessar, eighteen on the Canon 70-200.
    ParamGroup {
        label: "",
        params: &["coating_el1"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(1),
    },
    ParamGroup {
        label: "",
        params: &["coating_el2"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(2),
    },
    ParamGroup {
        label: "",
        params: &["coating_el3"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(3),
    },
    ParamGroup {
        label: "",
        params: &["coating_el4"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(4),
    },
    ParamGroup {
        label: "",
        params: &["coating_el5"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(5),
    },
    ParamGroup {
        label: "",
        params: &["coating_el6"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(6),
    },
    ParamGroup {
        label: "",
        params: &["coating_el7"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(7),
    },
    ParamGroup {
        label: "",
        params: &["coating_el8"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(8),
    },
    ParamGroup {
        label: "",
        params: &["coating_el9"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(9),
    },
    ParamGroup {
        label: "",
        params: &["coating_el10"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(10),
    },
    ParamGroup {
        label: "",
        params: &["coating_el11"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(11),
    },
    ParamGroup {
        label: "",
        params: &["coating_el12"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(12),
    },
    ParamGroup {
        label: "",
        params: &["coating_el13"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(13),
    },
    ParamGroup {
        label: "",
        params: &["coating_el14"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(14),
    },
    ParamGroup {
        label: "",
        params: &["coating_el15"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(15),
    },
    ParamGroup {
        label: "",
        params: &["coating_el16"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(16),
    },
    ParamGroup {
        label: "",
        params: &["coating_el17"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(17),
    },
    ParamGroup {
        label: "",
        params: &["coating_el18"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(18),
    },
    ParamGroup {
        label: "",
        params: &["coating_el19"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(19),
    },
    ParamGroup {
        label: "",
        params: &["coating_el20"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: Some(20),
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
        visible_when_lens_elements: None,
    },
    // The source-colour toggle: headerless, and shown for BOTH the
    // source modes that HAVE a source colour to take (Matte, and
    // Lights when it lands) — K-259.
    ParamGroup {
        label: "",
        params: &["use_source_colour"],
        collapsed: false,
        visible_when: Some(("source_type", &[1, 2])),
        visible_when_lens_elements: None,
    },
    // The matte rows: headerless (empty label renders them in place,
    // no twirl), shown only while Source type is Matte.
    ParamGroup {
        label: "",
        params: &["matte", "threshold", "threshold_softness"],
        collapsed: false,
        visible_when: Some(("source_type", &[1])),
        visible_when_lens_elements: None,
    },
];

/// The Lens flare's controls.
///
/// Parameter order is the owner's panel design: the light point pair, the three
/// headline dials, the coating character above the lens it colours, then the
/// collapsed detail groups, the Source mode with its conditional matte rows, and
/// Quality last.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "lens_flare",
    label = "Lens flare",
    // 11 since K-370: the ghost-edge diffraction is the knife-edge
    // asymptotic at each ghost's own (derived, and far higher) Fresnel
    // number, so the rim ringing hugs the rim and the broad interference
    // pattern K-369's mask ladder painted across the frame is gone.
    version = 11,
    category = Stylise,
    // The one effect that owns a render pass.
    cost = Heavy,
    roi = FullFrame,
    // An additive light overlay, the Glow shape.
    premultiplied = true,
    groups = LENS_FLARE_GROUPS,
    // K-395: the flare already declares a parameter called `matte`, and it means
    // something deeper than strength — it is where the flare *detects its
    // sources*. Naming the id claims it inside the flare's own maths and puts it
    // on the one matte carriage; because the row is declared below, none is
    // injected over the top of it.
    matte = (
        "matte",
        "where the flare looks for its light sources, in Matte source mode: the \
         parts of the matte brighter than Detect threshold are the lights, and \
         everything else contributes nothing",
    ),
)]
pub struct LensFlare {
    /// Where the light is, px@comp (K-260: point parameters are PIXELS, the
    /// Transform-anchor convention — never % of frame). Open both sides: an
    /// off-frame light keeps flaring. Pairs with `light_y` into one point row
    /// with a crosshair pick (docs/07 §6.1). The schema default is nominal 1080p;
    /// `instantiate_for_raster` centres a fresh instance on the actual comp's
    /// upper-left third (§1.2).
    #[slider(
        label = "Light x",
        min = 0.0,
        max = 3840.0,
        default = 640.0,
        unit = Px
    )]
    pub light_x: f32,

    /// See [`light_x`](Self::light_x).
    #[slider(
        label = "Light y",
        min = 0.0,
        max = 2160.0,
        default = 360.0,
        unit = Px
    )]
    pub light_y: f32,

    /// The **half**-width of the emitting area, px@comp like the position
    /// (K-260). 0 — the default — is the point source the effect has always had;
    /// anything larger makes it an AREA light whose ghosts take the shape of the
    /// source rather than of a point (K-355), because every ray integrates a
    /// different point of the rectangle (K-367). Pairs with `source_height` into
    /// one row.
    #[slider(
        min = 0.0,
        max = 400.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub source_width: f32,

    /// See [`source_width`](Self::source_width).
    #[slider(
        min = 0.0,
        max = 400.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub source_height: f32,

    /// Master gain on everything the effect adds; 0 is the neutral point (a
    /// bit-exact passthrough, pinned by test).
    #[slider(min = 0.0, max = 4.0, default = 1.0, hard_min = 0.0)]
    pub intensity: f32,

    /// The working f-stop: stops the iris down from the lens's native f-number.
    /// Wider (a smaller number) grows the ghost discs and softens the starburst
    /// ringing. The default is the default lens's native stop (the Master
    /// Prime is a T1.3), so a dropped-on flare starts wide open and at its
    /// brightest - stopping down dims it honestly (K-432).
    #[slider(
        label = "F-stop",
        min = 1.0,
        max = 22.0,
        default = 1.3,
        hard_min = 0.7,
        hard_max = 32.0
    )]
    pub fstop: f32,

    /// The embedded prescription library (K-261, curated to twenty at K-264):
    /// real lenses, transcribed patent data, chosen for maximally different flare
    /// characters. Sorted by name; the default is the Master Prime 50, the
    /// reference cine prime the effect was tuned against. A `.lens` file on
    /// [`lens_file`](Self::lens_file) overrides this pick entirely.
    ///
    /// Not clamped here: an out-of-range index is clamped inside `lens_entry`, so
    /// a pre-K-264 save whose index pointed into the old 1299-lens table simply
    /// lands on a valid curated lens.
    #[choice(
        label = "Lens",
        options = crate::fx::lens_library::LENS_OPTIONS,
        default = 16
    )]
    pub lens_model: u32,

    /// A user's own `.lens` prescription (K-264, the LUT File pattern): set, it
    /// overrides the Lens pick entirely — the twenty bundled lenses are a curated
    /// palette, and this is the door to everything else (the FlareSim /
    /// PhotonsToPhotos Optical Bench format the parser already reads). Unset,
    /// missing on disk or unparsable degrades to the picked lens — a labelled
    /// fallback, never a fault.
    ///
    /// **Always `None` here, by design.** A file slot is decided by the caller —
    /// only the render knows which prescription actually loaded and what its text
    /// hashes to — so `resolve_into_arena` carries no `Value::File`, and the file
    /// arrives at the GPU pass as half of this effect's aux pair (K-387). The row
    /// exists because the panel needs it.
    #[file(filter = ["lens"], filter_name = "Lens prescription")]
    pub lens_file: Option<u32>,

    // ---- The Lens options twirl ----
    /// Focus distance in metres (K-260): shifts the sensor from its calibrated
    /// infinity position by the thin-lens image shift, changing the whole flare's
    /// shape — real flares breathe with focus. Large values are infinity.
    #[slider(
        label = "Focus (m)",
        min = 0.5,
        max = 100.0,
        default = 100.0,
        hard_min = 0.2
    )]
    pub focus: f32,

    /// Horizontal stretch of the whole flare about the frame centre
    /// (1 = spherical, 1.33 / 2 = anamorphic looks).
    #[slider(
        label = "Anamorphic squeeze",
        min = 1.0,
        max = 2.0,
        default = 1.0,
        hard_min = 0.5,
        hard_max = 3.0
    )]
    pub anamorphic: f32,

    /// Iris blade count: the starburst's spike count and the ghost disc shape.
    ///
    /// **Rounded, not floored** — which is what the arena's generic Int
    /// conversion does, so unlike Depth of field's blade count this row needs no
    /// derivation of its own (K-385): the old resolve arm rounded here too, and
    /// moving the step would change what a keyframed sweep renders mid-way.
    #[counter(min = 3, max = 16, default = 8, hard_min = 3, hard_max = 16)]
    pub blades: i32,

    /// Turns the iris. Degrees on a dial (docs/07 §6): turning an iris is the
    /// gesture, not typing a number at it. Unbounded, so it winds through full
    /// turns rather than stopping at 360.
    #[dial(label = "Rotation", default = 0.0, step = 15.0)]
    pub aperture_rotation: f32,

    /// 0 = uncoated (bright neutral ghosts), 1 = full quarter-wave coating
    /// interference (dim, colour-cast ghosts).
    #[slider(min = 0.0, max = 1.0, default = 0.75, hard_min = 0.0, hard_max = 1.0)]
    pub coating: f32,

    /// Blends the blade polygon toward a circle.
    #[slider(min = 0.0, max = 1.0, default = 0.15, hard_min = 0.0, hard_max = 1.0)]
    pub roundness: f32,

    /// Softens the iris edge, and with it every ghost's rim.
    #[slider(
        label = "Softness",
        min = 0.0,
        max = 1.0,
        default = 0.05,
        hard_min = 0.0,
        hard_max = 1.0
    )]
    pub aperture_softness: f32,

    // ---- One row per glass element (K-371) ----
    // Each chooses a real AR design for one piece of glass. Twenty is the
    // schema's ceiling; each row's own group says how many elements a lens must
    // have for it to be drawn, so the panel offers exactly as many as the lens
    // does. Left "As the lens file" — the default — a row changes nothing at all.
    /// See the note above: the coating on glass element 1, the front piece.
    #[choice(
        label = "Element 1",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el1: u32,

    /// The coating on glass element 2 (K-371).
    #[choice(
        label = "Element 2",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el2: u32,

    /// The coating on glass element 3 (K-371).
    #[choice(
        label = "Element 3",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el3: u32,

    /// The coating on glass element 4 (K-371).
    #[choice(
        label = "Element 4",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el4: u32,

    /// The coating on glass element 5 (K-371).
    #[choice(
        label = "Element 5",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el5: u32,

    /// The coating on glass element 6 (K-371).
    #[choice(
        label = "Element 6",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el6: u32,

    /// The coating on glass element 7 (K-371).
    #[choice(
        label = "Element 7",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el7: u32,

    /// The coating on glass element 8 (K-371).
    #[choice(
        label = "Element 8",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el8: u32,

    /// The coating on glass element 9 (K-371).
    #[choice(
        label = "Element 9",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el9: u32,

    /// The coating on glass element 10 (K-371).
    #[choice(
        label = "Element 10",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el10: u32,

    /// The coating on glass element 11 (K-371).
    #[choice(
        label = "Element 11",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el11: u32,

    /// The coating on glass element 12 (K-371).
    #[choice(
        label = "Element 12",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el12: u32,

    /// The coating on glass element 13 (K-371).
    #[choice(
        label = "Element 13",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el13: u32,

    /// The coating on glass element 14 (K-371).
    #[choice(
        label = "Element 14",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el14: u32,

    /// The coating on glass element 15 (K-371).
    #[choice(
        label = "Element 15",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el15: u32,

    /// The coating on glass element 16 (K-371).
    #[choice(
        label = "Element 16",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el16: u32,

    /// The coating on glass element 17 (K-371).
    #[choice(
        label = "Element 17",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el17: u32,

    /// The coating on glass element 18 (K-371).
    #[choice(
        label = "Element 18",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el18: u32,

    /// The coating on glass element 19 (K-371).
    #[choice(
        label = "Element 19",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el19: u32,

    /// The coating on glass element 20 (K-371).
    #[choice(
        label = "Element 20",
        options = *lf::COATING_DESIGN_OPTIONS,
        default = 0,
        dividers_after = &[0, 1]
    )]
    pub coating_el20: u32,

    // ---- The Flare options twirl ----
    /// Gain on the ghost train alone.
    #[slider(min = 0.0, max = 4.0, default = 1.0, hard_min = 0.0)]
    pub ghost_intensity: f32,

    /// Box-blur radius as % of the frame diagonal (K-261, FlareSim's Ghost Blur):
    /// a touch of out-of-focus softness. 0.02 by default (owner-set, K-264) —
    /// with the vertex-smoothed density and the multisampled raster the geometry
    /// no longer needs hiding, so the default is taste, not cover, and 0 stays a
    /// usable, clean setting.
    #[slider(min = 0.0, max = 1.0, default = 0.02, hard_min = 0.0, hard_max = 2.0)]
    pub ghost_softness: f32,

    /// How many of the brightest-ranked ghost pairs render. The cap survives by
    /// rank, so turning it down drops the dimmest ghosts first.
    #[counter(min = 0, max = 150, default = 60, hard_min = 0, hard_max = 200)]
    pub max_ghosts: i32,

    /// Ray-budget multiplier on the Quality tier's pupil grid (K-265,
    /// owner-asked): the tiers pick a sensible base and this dial hands the trade
    /// to the user — a lens whose ghost rims still show their cells buys more rays
    /// without jumping a whole tier, a preview buys fewer. Frame-time, never
    /// rebakes; 1 is the tier as shipped.
    #[slider(min = 0.25, max = 2.0, default = 1.0, hard_min = 0.25, hard_max = 4.0)]
    pub detail: f32,

    /// Scales each traced wavelength's offset from the spectrum midpoint: 0 is a
    /// monochrome trace (no fringing), 1 physical, 2 doubled.
    #[slider(min = 0.0, max = 2.0, default = 1.0, hard_min = 0.0)]
    pub dispersion: f32,

    /// Gain on the starburst alone.
    #[slider(min = 0.0, max = 4.0, default = 1.0, hard_min = 0.0)]
    pub starburst_intensity: f32,

    /// Scales the WHOLE flare about the optical centre — ghost train and
    /// starburst together (owner pass 2).
    #[slider(min = 0.1, max = 4.0, default = 1.0, hard_min = 0.05, hard_max = 20.0)]
    pub scale: f32,

    // ---- Source ----
    /// Where the light comes from: Manual (the light point above), Matte (bright
    /// sources detected in a referenced layer), or **Lights** — the comp's own
    /// Light layers (K-360), which reach the maths through
    /// [`LensFlareDef::resolve_derived`] rather than through a row.
    #[choice(
        label = "Source",
        options = ["Manual light", "Matte", "Lights"],
        default = 0
    )]
    pub source_type: u32,

    /// Multiplies every light's colour, in every source mode (K-259): in Manual
    /// it IS the flare's colour; in Matte it tints whatever the sources
    /// contribute. Scene-linear, and open above 1 so an HDR tint can push a flare
    /// hotter. Alpha unused.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], min = 0.0, max = 4.0)]
    pub light_tint: [f32; 4],

    /// On: a detected source's own colour tints its flare (a warm practical
    /// flares warm). Off: every source flares white through Light tint alone,
    /// which is what a matte used purely as a *position* mask wants (K-259).
    #[toggle(default = true)]
    pub use_source_colour: bool,

    /// The layer whose brightest sources spawn the flares (impl note §6); unset is
    /// a labelled no-flare, never a fault. `self_default` (K-288): a fresh flare
    /// points at its OWN layer, because "flare the lights in this picture" is what
    /// asking for a matte source nearly always means — and on an adjustment layer
    /// that reads the composite of everything below, so the effect works there
    /// without hunting for another layer to point at.
    ///
    /// **Always `false` here, by design.** A Layer binding is decided by the
    /// caller — only the render knows which layer was actually rendered — so
    /// `resolve_into_arena` carries no `Value::Layer`, and the matte arrives at
    /// the GPU pass as half of this effect's aux pair (K-387). The row exists
    /// because the panel needs it; the trace never asks the bag whether it is
    /// bound, because a missing matte simply detects no sources.
    ///
    /// **Labelled "Matte", not "Matte layer"** (K-395): the uniform row's word,
    /// shared with every other effect's matte. The stored id was already `matte`,
    /// so nothing but the label moves.
    #[layer(label = "Matte", self_default = true)]
    pub matte: bool,

    /// The absolute scene-linear luma a pixel must EXCEED to flare (K-363): at 1.0
    /// only over-range highlights, at 0.0 anything brighter than black — black
    /// itself never. The slider is normalised 0–1; typing goes above.
    #[slider(min = 0.0, max = 1.0, default = 1.0, hard_min = 0.0)]
    pub threshold: f32,

    /// Half-width of the soft gate around the threshold.
    #[slider(min = 0.0, max = 1.0, default = 0.25, hard_min = 0.0)]
    pub threshold_softness: f32,

    /// The ray-grid density and traced wavelength count; Draft renders the flare
    /// buffer at half resolution.
    #[choice(options = ["Draft", "Normal", "High", "Ultra"], default = 1)]
    pub quality: u32,

    /// How the flare element combines with the layer under it (K-289, replacing
    /// K-258's Transparent/Black Background pair). The curated light-combine set
    /// Echo offers, for the same reason (T21): the HSL / burn / dodge modes are
    /// ill-defined on a premultiplied light overlay. Normal heads the list, then a
    /// divider, because it is the one mode that REPLACES the layer — the flare on
    /// its own opaque black, which is what Background = Black existed to export.
    /// Default Add: the behaviour every flare had before this menu, so nothing
    /// anyone had built moves.
    #[choice(
        options = *lf::BLEND_OPTIONS,
        default = lf::BLEND_ADD,
        dividers_after = &[0]
    )]
    pub blend: u32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub mix: f32,
}

impl LensFlare {
    /// How many Lights-mode sources this frame resolved (K-360) — never a panel
    /// row, and 0 in every other mode.
    pub const DERIVED_LIGHT_COUNT: ParamId = ParamId::new("derived.light_count");

    /// One Lights-mode source's two derived entries: its geometry as
    /// `(x, y, half_width, half_height)` in **raster pixels**, and its colour as
    /// `(r, g, b, 0)` with the light's own intensity already folded in.
    ///
    /// Two [`Value::Colour`]s rather than seven floats because a colour is the
    /// arena's widest value and a light is seven numbers: two entries a light
    /// instead of seven, and the fixed sixteen ids below mean the bag never
    /// allocates a name. Only the sources that actually exist are pushed, so a
    /// Manual or Matte flare carries none of this at all.
    ///
    /// Spelled out rather than formatted, exactly as
    /// [`COATING_ELEMENT_IDS`](crate::fx::lens_flare::COATING_ELEMENT_IDS) is, so
    /// a typo is a compile error.
    pub const DERIVED_LIGHTS: [(ParamId, ParamId); lf::MAX_SOURCES] = [
        (
            ParamId::new("derived.light0"),
            ParamId::new("derived.light0_rgb"),
        ),
        (
            ParamId::new("derived.light1"),
            ParamId::new("derived.light1_rgb"),
        ),
        (
            ParamId::new("derived.light2"),
            ParamId::new("derived.light2_rgb"),
        ),
        (
            ParamId::new("derived.light3"),
            ParamId::new("derived.light3_rgb"),
        ),
        (
            ParamId::new("derived.light4"),
            ParamId::new("derived.light4_rgb"),
        ),
        (
            ParamId::new("derived.light5"),
            ParamId::new("derived.light5_rgb"),
        ),
        (
            ParamId::new("derived.light6"),
            ParamId::new("derived.light6_rgb"),
        ),
        (
            ParamId::new("derived.light7"),
            ParamId::new("derived.light7_rgb"),
        ),
        (
            ParamId::new("derived.light8"),
            ParamId::new("derived.light8_rgb"),
        ),
        (
            ParamId::new("derived.light9"),
            ParamId::new("derived.light9_rgb"),
        ),
        (
            ParamId::new("derived.light10"),
            ParamId::new("derived.light10_rgb"),
        ),
        (
            ParamId::new("derived.light11"),
            ParamId::new("derived.light11_rgb"),
        ),
        (
            ParamId::new("derived.light12"),
            ParamId::new("derived.light12_rgb"),
        ),
        (
            ParamId::new("derived.light13"),
            ParamId::new("derived.light13_rgb"),
        ),
        (
            ParamId::new("derived.light14"),
            ParamId::new("derived.light14_rgb"),
        ),
        (
            ParamId::new("derived.light15"),
            ParamId::new("derived.light15_rgb"),
        ),
    ];

    /// The Lights-mode sources out of a resolved bag (K-360) — [`Self::packed`]'s
    /// missing argument, so no caller has to know the ids. Empty in Manual and
    /// Matte modes, which is exactly what the old resolve arm left there.
    pub fn lights_of(p: Params<'_>) -> ([lf::FlareLight; lf::MAX_SOURCES], u32) {
        let mut lights = [lf::DEAD_LIGHT; lf::MAX_SOURCES];
        let count = p
            .int(Self::DERIVED_LIGHT_COUNT, 0)
            .clamp(0, lf::MAX_SOURCES as i32) as u32;
        for (i, slot) in lights.iter_mut().take(count as usize).enumerate() {
            let (geom, rgb) = Self::DERIVED_LIGHTS[i];
            let g = p.colour(geom, [0.0; 4]);
            let c = p.colour(rgb, [0.0; 4]);
            *slot = lf::FlareLight {
                pos: [g[0], g[1]],
                rgb: [c[0], c[1], c[2]],
                extent: [g[2], g[3]],
            };
        }
        (lights, count)
    }

    /// Every element's coating choice as the trace's array (K-371), each clamped
    /// to the palette. A row the panel never showed (this lens has fewer elements)
    /// reads its default and leaves the prescription's own column alone, so an
    /// unset row and a missing row are the same thing.
    fn coating_elements(self) -> [u32; lf::MAX_COATING_ELEMENTS] {
        let chosen = [
            self.coating_el1,
            self.coating_el2,
            self.coating_el3,
            self.coating_el4,
            self.coating_el5,
            self.coating_el6,
            self.coating_el7,
            self.coating_el8,
            self.coating_el9,
            self.coating_el10,
            self.coating_el11,
            self.coating_el12,
            self.coating_el13,
            self.coating_el14,
            self.coating_el15,
            self.coating_el16,
            self.coating_el17,
            self.coating_el18,
            self.coating_el19,
            self.coating_el20,
        ];
        chosen.map(|c| c.min(lf::COATING_DESIGNS - 1))
    }

    /// What the bake and the kernels want (docs/impl/effect-registry.md §2.4),
    /// derived exactly as the old resolve arm derived it — every clamp in the same
    /// place, so a saved project renders the picture it always rendered.
    ///
    /// `lights` and `light_count` are [`Self::lights_of`]: Lights mode's sources,
    /// which are not parameters and so cannot be read off `self`.
    pub fn packed(
        self,
        lights: [lf::FlareLight; lf::MAX_SOURCES],
        light_count: u32,
    ) -> lf::LensFlareParams {
        lf::LensFlareParams {
            // Already raster pixels: the declared `Px` unit is what carries the
            // §2.3 preview factor, and what `rescale_spatial` moves again if the
            // stack is reused at another size.
            light: [self.light_x, self.light_y],
            source_size: [self.source_width.max(0.0), self.source_height.max(0.0)],
            lights,
            light_count,
            intensity: self.intensity.max(0.0),
            lens: self.lens_model,
            fstop: self.fstop.clamp(0.7, 32.0),
            focus_m: self.focus.max(0.2),
            blades: self.blades.clamp(3, 16) as u32,
            aperture_rotation_deg: self.aperture_rotation,
            roundness: self.roundness.clamp(0.0, 1.0),
            aperture_softness: self.aperture_softness.clamp(0.0, 1.0),
            coating_elements: self.coating_elements(),
            ghost_intensity: self.ghost_intensity.max(0.0),
            ghost_softness: self.ghost_softness.clamp(0.0, 2.0),
            max_ghosts: self.max_ghosts.clamp(0, 200) as u32,
            dispersion: self.dispersion.max(0.0),
            coating: self.coating.clamp(0.0, 1.0),
            starburst_intensity: self.starburst_intensity.max(0.0),
            scale: self.scale.clamp(0.05, 20.0),
            // Lights (2) is the last mode; an index past the menu clamps rather
            // than faulting.
            source: self.source_type.min(2),
            threshold: self.threshold.max(0.0),
            threshold_softness: self.threshold_softness.max(0.0),
            // Scene-linear RGB, clamped at zero below and open above (an HDR tint
            // pushes the flare hotter). Alpha unused.
            light_tint: [
                self.light_tint[0].max(0.0),
                self.light_tint[1].max(0.0),
                self.light_tint[2].max(0.0),
            ],
            use_source_colour: self.use_source_colour,
            anamorphic: self.anamorphic.clamp(0.5, 3.0),
            quality: self.quality.min(3),
            detail: self.detail.clamp(0.25, 4.0),
            // An index past the menu clamps to the last option; a project saved
            // before the menu existed is migrated by `backfill_builtin_params`.
            blend: self.blend.min(lf::BLEND_OPTIONS.len() as u32 - 1),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// The Lens flare's behaviour: no CPU reference through the single-image
/// dispatcher (the flare is a render pass over baked tables), so `apply_cpu`
/// keeps its identity default — the passthrough the old `Resolved::LensFlare` arm
/// was.
pub struct LensFlareDef;

impl EffectDef for LensFlareDef {
    fn schema(&self) -> &'static EffectSchema {
        &<LensFlare as EffectMetadata>::SCHEMA
    }

    /// **Lights mode's sources** (K-360, K-385): the comp's own Light layers,
    /// resolved here because the expression context already carries the document,
    /// the comp and the time — everything needed, and nothing new to thread.
    ///
    /// Positions and extents go in RASTER pixels, so they ride the same preview
    /// factor the Manual position does and `manual_light` turns both into
    /// fractions in one place. Nothing at all is pushed in Manual or Matte mode,
    /// which is the empty list the old arm left there — and a comp with no lights
    /// flares with nothing rather than falling back to the Manual point, which
    /// would put a flare somewhere nobody asked for.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let source = match cx.inst.param("source_type") {
            Some(EffectValue::Choice(c)) => *c,
            _ => 0,
        };
        if source != 2 {
            return;
        }
        let Some(owner) = cx.context.comp.and_then(|id| cx.context.document.comp(id)) else {
            return;
        };
        let mut count = 0u32;
        for resolved in owner.lights_at(cx.context.comp_time) {
            let Some(&(geom, rgb)) = LensFlare::DERIVED_LIGHTS.get(count as usize) else {
                break;
            };
            push(
                geom,
                Value::Colour([
                    resolved.position.0 as f32 * cx.px_scale,
                    resolved.position.1 as f32 * cx.px_scale,
                    resolved.half_size.0 as f32 * cx.px_scale,
                    resolved.half_size.1 as f32 * cx.px_scale,
                ]),
            );
            push(
                rgb,
                Value::Colour([
                    resolved.colour[0],
                    resolved.colour[1],
                    resolved.colour[2],
                    0.0,
                ]),
            );
            count += 1;
        }
        push(LensFlare::DERIVED_LIGHT_COUNT, Value::Int(count as i32));
    }
}
