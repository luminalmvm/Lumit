use super::params::Unit;

/// Cost class (docs/08 §1.3) — consumed by degradation ordering and budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostClass {
    Trivial,
    Cheap,
    Moderate,
    Heavy,
}

/// Region-of-interest support (docs/08 §1.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Roi {
    /// Output pixel needs only the same input pixel.
    Exact,
    /// Needs input dilated by a radius in **px@comp** — pixels at composition
    /// size, the unit every distance in Lumit is in. Sized from
    /// the effect's own hard maximum, so a typed radius can never reach past
    /// the tile it was given.
    PaddedPx(f32),
    /// Needs the whole input.
    FullFrame,
}

impl Roi {
    /// The padding in raster pixels at the raster in play. `px_scale` is raster
    /// pixels per comp pixel — exactly the factor a [`Unit::Px`](super::Unit::Px)
    /// parameter is multiplied by in the resolve step, so a padding and the
    /// radius it has to cover move together under preview resolution.
    ///
    /// Rounded up, and never below one pixel: a padding is a whole number of
    /// pixels, and a neighbourhood of one raster pixel is still one raster pixel
    /// at Quarter. `None` is [`Roi::FullFrame`] — no finite padding exists.
    pub fn padding_raster_px(self, px_scale: f32) -> Option<u32> {
        match self {
            Roi::Exact => Some(0),
            // clamp, not `as`, so a nonsense scale cannot wrap: the cast
            // saturates and nothing panics (docs/14).
            Roi::PaddedPx(px) => Some((px * px_scale).ceil().clamp(1.0, u32::MAX as f32) as u32),
            Roi::FullFrame => None,
        }
    }
}

/// Static trait declaration (docs/08 §1.3), read by the scheduler and caches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectTraits {
    pub cost: CostClass,
    pub roi: Roi,
    /// Source-relative frame offsets required; `&[0]` = current frame only.
    pub temporal: &'static [i32],
    /// True = operates on premultiplied alpha (the default working form).
    pub premultiplied: bool,
    pub seeded: bool,
    pub beat_input: bool,
}

/// One declared parameter (docs/08 §1.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSchema {
    /// Stable snake_case identifier (expressions address this).
    pub id: &'static str,
    /// Sentence-case UI label.
    pub label: &'static str,
    pub kind: ParamKind,
    /// What the number *means* (docs/impl/effect-registry.md §2.2): a plain
    /// number, pixels at comp size, a per cent, degrees, seconds, frames.
    ///
    /// Declaring it is what lets the preview-raster rescale be one generic pass
    /// rather than a match that has to know which field of which effect holds a
    /// pixel count — an effect cannot forget to be rescaled. It is also the
    /// single source of truth for the unit rider the panel draws beside the
    /// value, which is why [`Unit::Unset`] is a build failure rather
    /// than a quiet "no unit": a parameter that never decided and a parameter
    /// that is genuinely dimensionless must not look alike.
    ///
    /// Every numeric declaration says it outright. A control that cannot carry
    /// a unit at all — a switch, a dropdown, a colour, a seed, a file, a layer,
    /// a mask, a curve, a button — is [`Unit::Raw`] from the derive, and a
    /// `#[dial]` is [`Unit::Degrees`] from the derive, an angle being degrees by
    /// definition.
    pub unit: Unit,
}

/// One **vector pair**: two adjacent parameters that are the x and y halves of
/// one point, found by [`EffectSchema::pairs`].
///
/// `stem` is the pair's name with the axis taken off — `light` for
/// `light_x` / `light_y` — and it is the **key the link flag is stored under**
/// on an effect instance, so it has to be the pair's identity rather
/// than either half's id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamPair {
    /// The shared name: `light`, `centre`, `focus_point`.
    pub stem: &'static str,
    /// The x half's parameter id.
    pub x: &'static str,
    /// The y half's parameter id.
    pub y: &'static str,
}

/// What a socket on the graph canvas carries
/// ([impl/node-graph.md](../../../docs/impl/node-graph.md) §6.1).
///
/// # In plain terms
///
/// Seven kinds of thing can travel down a wire, and the wire's **colour is its
/// kind** — that is the whole legend, so nothing has to be labelled. Seven
/// types wear five colours, grouped as the approved NodeGraph drawing groups
/// them: image with matte, number, colour, shape with points, audio. The colour
/// itself is a theme token the frontend picks from this enum; **no colour ever
/// crosses the bridge**.
///
/// [`PortType::Points`] lands with the first engine commit even though nothing
/// emits one yet, so the type system is complete before Particulate
/// arrives to consume it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortType {
    /// A picture.
    Image,
    /// Coverage — a picture read for how much of it there is, not what colour.
    Matte,
    /// One number.
    Number,
    /// Scene-linear RGBA.
    Colour,
    /// Vector geometry.
    Shape,
    /// A points stream: one frame's particles or instances. Evaluated
    /// data like an image, never stored in the document.
    Points,
    /// Sound.
    Audio,
}

/// One socket on the graph canvas: its stable id, the English word beside it,
/// and what it carries.
///
/// # In plain terms
///
/// A port is a plug on the side of a box. The **id** is what the document
/// writes down when a wire is joined to it — never seen, never translated. The
/// **label** is what the canvas draws beside the plug, and like every other word
/// the engine sends it crosses the bridge in English and is looked up in the
/// frontend's table — which is why it is declared here, beside the port,
/// rather than worked out from the id at the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Port {
    /// Stable, snake_case, and the only half the document stores.
    pub id: &'static str,
    /// The word drawn beside the socket, in British English.
    pub label: &'static str,
    /// What travels through it, which is also what colours it.
    pub ty: PortType,
    /// **3D awareness**, for a [`PortType::Points`] port only.
    ///
    /// The points wire stays one type: a stream carries three coordinates
    /// whatever reads it. What this says is which of the two readings the
    /// consumer wants — `false`, the default and every consumer today, means
    /// "give me where the camera puts them"
    /// ([`PointsStream::projected`](crate::fx::points::PointsStream::projected));
    /// `true` means "give me the three axes, I will do my own geometry". It is
    /// a **declaration on the port**, not a second type and not a second wire,
    /// so a 3D-aware effect can be dropped into a graph full of 2D ones and
    /// nothing about the connection changes.
    pub three_d: bool,
}

impl Port {
    /// Declare a port. `const` so a signature's output list is a static.
    #[must_use]
    pub const fn new(id: &'static str, label: &'static str, ty: PortType) -> Port {
        Port {
            id,
            label,
            ty,
            three_d: false,
        }
    }

    /// The same port, declared **3D-aware**: its consumer reads the
    /// stream's three axes rather than the projected two.
    #[must_use]
    pub const fn three_d(self) -> Port {
        Port {
            id: self.id,
            label: self.label,
            ty: self.ty,
            three_d: true,
        }
    }
}

/// Parameter type + defaults/ranges (docs/08 §1.2: sliders may be exceeded
/// by typing; hard ranges may not).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamKind {
    Float {
        default: f64,
        slider: (f64, f64),
        /// Hard bounds; either side may be None (a threshold clamps
        /// at zero below and runs unbounded above).
        hard: (Option<f64>, Option<f64>),
    },
    /// A **bounded** number: a track and a thumb with the value beside it, for
    /// a parameter whose whole meaning lives inside a closed range.
    ///
    /// The VALUE side is an ordinary [`EffectValue::Float`](crate::model::
    /// EffectValue::Float), exactly as [`ParamKind::Int`] and
    /// [`ParamKind::Angle`] ride — the kind is the *control*, not the storage,
    /// so keyframes, expressions, the graph editor and the resolve step see no
    /// new shape and an adopting parameter changes no stored value.
    ///
    /// The one difference from [`ParamKind::Float`] is that there is no soft
    /// slider and hard bound to keep apart: `range` is both. A Float declares
    /// a slider that typing MAY exceed (docs/08 §1.2); a Slider declares the
    /// only numbers the parameter has — a wipe cannot be less than nought or
    /// more than fully complete, and offering a box to type 150 into would be
    /// offering a picture that does not exist.
    ///
    /// **Not every closed Float wants to be one.** Adoption is for parameters
    /// where the range is the parameter's *nature* rather than a range someone
    /// found sufficient: Temperature's ±150 slider runs to a ±200 hard range
    /// precisely because there is a picture beyond the slider's end, so it
    /// stays a Float (the first candidate for adoption, examined and declined).
    Slider {
        default: f64,
        /// The closed range: the slider's travel, and the hard bounds, which
        /// are the same two numbers.
        range: (f64, f64),
    },
    /// A whole-number parameter (a blade count, a ghost cap). The VALUE side
    /// is still an `EffectValue::Float` — it animates and serialises exactly
    /// like a Float — the kind only tells the UI to step and display it as an
    /// integer and the resolve step to round it, replacing the old "rounded
    /// float row" convention (docs/08 §1.2).
    Int {
        default: i64,
        slider: (i64, i64),
        /// Hard bounds; either side may be None, matching Float.
        hard: (Option<i64>, Option<i64>),
    },
    /// An angle in degrees — the parameter type docs/08 §1.1 has listed since
    /// the beginning and docs/07-UI-SPEC.md §6 names among the widgets, drawn
    /// as a **dial** beneath its number rather than a slider.
    ///
    /// The stored value is an ordinary [`EffectValue::Float`](crate::model::
    /// EffectValue::Float), so keyframes, expressions and the resolve step see
    /// no new shape: an angle *is* a number of degrees, and the only thing this
    /// kind changes is the control drawn for it. Deliberately unbounded — an
    /// angle animates through full turns rather than stopping at 360, which is
    /// why the existing degree parameters (Shake's rotation, the aperture's
    /// blade rotation) declare `hard: (None, None)`. `dial_step` is the
    /// snapping increment while a modifier is held, in degrees.
    ///
    /// There is deliberately **no `Point` kind beside this one**. A 2-D point
    /// is already a pair of adjacent `_x`/`_y` Floats that the panel folds into
    /// one row with a crosshair pick (docs/07 §6.1) — the Lens flare's Light
    /// and Radial blur's Centre both ride it — so a point needs no schema kind
    /// of its own, only the naming convention. An angle has no such fallback:
    /// there is no arrangement of existing rows that draws a dial.
    Angle {
        default: f64,
        dial_step: f64,
    },
    Choice {
        options: &'static [&'static str],
        default: u32,
        /// Option indices after which the dropdown draws a group divider
        /// (T21): e.g. Echo's Mode lists its effect-only compositing orders
        /// (Behind / In front) first, then a divider, then the standard blend
        /// modes. Empty for an ungrouped list. The [`CHOICE_UNGROUPED`] alias
        /// spells "no dividers" for the common case.
        dividers_after: &'static [u32],
    },
    Bool {
        default: bool,
    },
    Colour {
        /// Scene-linear RGBA (docs/08 §1.1's colour type); channels animate
        /// independently in the model.
        default: [f64; 4],
        /// Per-channel edit range — linear values may exceed 1 (HDR tints)
        /// or dip below 0 (a lift), so each colour declares its own.
        range: (f64, f64),
    },
    /// An integer seed (docs/08 §1.1's seed type): selects which
    /// deterministic random-looking sequence a seeded effect follows
    /// (§2.4). No declared default — the default is per-instance (§3.4),
    /// drawn from the fresh instance id at instantiation, so two copies of
    /// a seeded effect never wobble in sync.
    Seed,
    /// A file path chosen from a dialog, e.g. a `.cube` LUT. The
    /// `filter` extensions (lower-case, no dot) and `filter_name` drive the
    /// open dialog. The value carries a [`FileParam`]; it animates only by
    /// stepping (hold keys), since two paths cannot be blended.
    File {
        filter: &'static [&'static str],
        filter_name: &'static str,
    },
    /// A name from the project's OCIO config (docs/impl/ocio.md §6.6): a
    /// colour space, a display, a view or a look, drawn as a dropdown the
    /// frontend fills from the colour summary it already holds. The value is
    /// an [`EffectValue::Text`](crate::model::EffectValue::Text) carrying the
    /// config's own spelling, empty for unset, and it never reaches the
    /// arena: the render resolves the name against the loaded config and
    /// threads the baked table beside the op, as a LUT's cube is threaded.
    /// Static, as a File row is.
    ColourName {
        role: ColourNameRole,
    },
    /// A reference to another layer in the composition (docs/impl/
    /// layer-input.md), sampled as an auxiliary picture — the depth pass a
    /// depth-of-field effect reads, the bright-source matte a Lens flare
    /// detects lights in. The value carries an [`EffectValue::Layer`] (an
    /// optional layer id); the caller renders that layer alone at comp size
    /// and threads its texture beside the resolved op, exactly as a matte
    /// layer is rendered alone. Unset (or a dangling reference) is a
    /// labelled no-op, never a fault.
    ///
    /// **This layer**: a reference to the layer the effect is *on*
    /// is not a re-render of that layer — it is the effect's own input at
    /// its point in the stack. On an ordinary layer that is the picture the
    /// effect is about to process; on an **adjustment layer** it is the
    /// composite of everything below, which is the only thing an adjustment
    /// layer has to look at. `self_default` declares that a fresh instance
    /// should start pointed at its own layer (the Lens flare's matte, whose
    /// natural reading is "the lights in this picture"), rather than unset.
    Layer {
        /// A fresh instance added to a layer starts referencing that layer
        /// (see the type docs). `false` leaves it unset, the historical
        /// no-op default — a depth pass is never the picture itself.
        self_default: bool,
    },
    /// A reference to one of **this layer's masks**, handed to the effect as
    /// *geometry* — where the curve goes — rather than as the coverage the
    /// mask produces.
    ///
    /// # In plain terms
    ///
    /// Some effects work along a shape you have drawn rather than on the
    /// picture: a brush that walks a mask path from one per cent to another,
    /// pencil strokes that fill one, segments that march round one. Coverage
    /// cannot tell them any of that — a coverage buffer is a picture, and a
    /// picture says nothing about which way is *along*. So this kind names a
    /// mask, and the render hands the effect the mask's curve.
    ///
    /// **The value is the choice, never the geometry.** What the document
    /// stores is an [`EffectValue::MaskPath`](crate::model::EffectValue::
    /// MaskPath) — an optional mask id, static exactly as a layer reference
    /// is. The vertices ride beside the op, flattened at the frame's time
    /// ([`crate::mask::mask_path_at`]), the way the aux slots and the matte
    /// do; forcing a path through a parameter would be the wrong shape
    /// permanently.
    ///
    /// **"First mask"** is the self-default, the way [`ParamKind::Layer`] has
    /// `self_default`: an unset row means the layer's first mask rather than
    /// nothing, so an effect dropped on a layer that is about to be masked
    /// already points somewhere sensible. It is *not* written into the
    /// instance at instantiation the way a self-default layer reference is —
    /// an effect is usually added before the mask is drawn, so there is no id
    /// to write. It resolves at render time instead, which is also what keeps
    /// it pointing at the first mask when the masks are reordered.
    ///
    /// A row that names nothing, names a mask that has been deleted, or names
    /// the first mask of a layer that has none resolves to an **empty
    /// polyline**: the effect's documented no-op, degrade and never fault
    /// (14-ENGINEERING-RULES §4).
    MaskPath {
        /// An unset row means the layer's first mask. `false` means it means
        /// nothing — for an effect whose path input is genuinely optional.
        self_default: bool,
    },
    /// A **tone curve**, stored as its own control points.
    ///
    /// # In plain terms
    ///
    /// The shape you drag in a Curves panel: a handful of points in a unit
    /// square, with a smooth line drawn through them. Not a number and not a
    /// list of numbers at fixed positions — the points move sideways as well
    /// as up and down, which is what makes it a curve rather than five
    /// sliders wearing a curve's name.
    ///
    /// The value is an [`EffectValue::Curve`](crate::model::EffectValue::
    /// Curve): an ordered list of **2..=16** points in the unit square,
    /// defaulting to the identity diagonal `[[0, 0], [1, 1]]`. It is read
    /// through [`CurvePoints::sanitised`](crate::fx::CurvePoints::sanitised),
    /// which sorts by x, drops repeated x, clamps into the square and falls
    /// back to the diagonal — quietly, never a panic
    /// (14-ENGINEERING-RULES §4), because a curve arriving out of order is a
    /// document to render, not a fault to report.
    ///
    /// **Static in v1**, joining [`ParamKind::File`], [`ParamKind::Layer`] and
    /// [`ParamKind::MaskPath`] on that side of the seam: a list that grows and
    /// shrinks has no interpolation between two keyframes, which is exactly
    /// why After Effects' own curve blob only ever *steps*.
    Curve {
        /// The shape a fresh instance starts with, in the unit square.
        ///
        /// The grade family's five curves all start on the identity diagonal
        /// ([`CURVE_IDENTITY`](super::params::CURVE_IDENTITY)), which is what
        /// the derive uses when a declaration says nothing. An **over-life**
        /// curve is the reason this is declared rather than assumed:
        /// Particulate's Opacity over life is born solid and dies faded
        /// (`[[0, 1], [1, 0]]`) and its Size over life is flat
        /// (`[[0, 1], [1, 1]]`), and a diagonal would mean particles that
        /// start invisible and grow from nothing (particulate.md §2).
        default: &'static [[f32; 2]],
    },
    /// A **button**, not a value: a row the panel draws as a push
    /// button, which asks the engine to *do* something rather than describing
    /// what a picture should look like.
    ///
    /// # In plain terms
    ///
    /// The Camera track effect's Analyse and Cancel. Pressing one starts or
    /// stops a background job; there is nothing to store, nothing to animate,
    /// and nothing for a kernel to read. Every other kind answers "what should
    /// this frame look like"; this one answers "go".
    ///
    /// It is generic because the tracker will not be the last effect that wants
    /// one — beat detection is already waiting — and because a button written
    /// as a Bool that an effect watches for a rising edge is a control that
    /// keyframes, saves, and fires again on load.
    ///
    /// Three consequences, and they are the whole of the kind:
    ///
    /// - **No value.** [`default_param_value`](crate::fx::default_param_value)
    ///   answers `None`, so `instantiate` writes no `EffectParam` for it and
    ///   the backfill appends none. There is no
    ///   [`EffectValue`](crate::model::EffectValue) variant to add.
    /// - **Never keyframes.** There is no value to interpolate, so the graph
    ///   editor and the expression system never see the row at all.
    /// - **Not in the arena.** The resolve step skips it exactly as it skips a
    ///   File or a Layer row, for a stronger reason: those carry their payload
    ///   beside the op, and this one carries nothing anywhere. It crosses the
    ///   bridge as an *event* (stage 3), never as a parameter value.
    Action,
}

impl ParamKind {
    /// The socket type a driver wire may land on, or `None` for a control no
    /// wire can drive.
    ///
    /// Number accepts number and colour accepts colour; nothing else is
    /// drivable in v1. A switch, a dropdown, a seed, a file, a layer, a mask,
    /// a curve and a button all answer `None` — deliberately, because a wire
    /// into one of them would be a wire whose meaning nobody has decided.
    #[must_use]
    pub const fn port_type(self) -> Option<PortType> {
        match self {
            ParamKind::Float { .. }
            | ParamKind::Slider { .. }
            | ParamKind::Int { .. }
            | ParamKind::Angle { .. } => Some(PortType::Number),
            ParamKind::Colour { .. } => Some(PortType::Colour),
            ParamKind::Bool { .. }
            | ParamKind::Choice { .. }
            | ParamKind::Seed
            | ParamKind::File { .. }
            | ParamKind::Layer { .. }
            | ParamKind::MaskPath { .. }
            | ParamKind::Curve { .. }
            | ParamKind::ColourName { .. }
            | ParamKind::Action => None,
        }
    }
}

/// Which of a config's lists a [`ParamKind::ColourName`] row offers. A view
/// row lists the views of the display its sibling `display` row names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourNameRole {
    Space,
    Display,
    View,
    Look,
    /// The Information row: read-only, showing what the loaded config calls
    /// itself. Its value stays empty and nothing reads it.
    Config,
}

/// How a transform- or displacement-domain effect treats the border pixels
/// its warp reveals (P3): the one reusable Edges control, shared by
/// the blur family (docs/08 §3.8) and Shake (§3.4). The `u32` codes are the
/// wire form the resolved ops and every WGSL kernel read — 0 Transparent,
/// 1 Repeat, 2 Mirror — so the enum only names those numbers, it never
/// changes them. Any effect whose resample can pull in area outside the
/// frame reuses this rather than re-deciding what an edge means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgesMode {
    /// Revealed area is transparent (contributes nothing, full weight kept).
    Transparent,
    /// The border pixel is held outward (clamp-to-edge).
    Repeat,
    /// The image reflects at the border, without repeating the edge sample.
    Mirror,
}

impl EdgesMode {
    /// The Choice option labels, in code order (index 0/1/2). A schema's Edges
    /// parameter declares `options: EdgesMode::OPTIONS` (aliased as the shared
    /// [`EDGE_OPTIONS`](crate::fx::EDGE_OPTIONS) const the blur family already
    /// uses).
    pub const OPTIONS: &'static [&'static str] = &["Transparent", "Repeat", "Mirror"];

    /// The wire code the resolved ops and the WGSL kernels read.
    pub const fn code(self) -> u32 {
        match self {
            EdgesMode::Transparent => 0,
            EdgesMode::Repeat => 1,
            EdgesMode::Mirror => 2,
        }
    }

    /// The mode for a stored Choice index, or `None` for an unknown code (a
    /// caller supplies its own default). 0 Transparent, 1 Repeat, 2 Mirror.
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(EdgesMode::Transparent),
            1 => Some(EdgesMode::Repeat),
            2 => Some(EdgesMode::Mirror),
            _ => None,
        }
    }
}

/// A collapsible group of parameters inside one effect's parameter list
/// (P4): the disclosure "twirl" the Effect Controls draws so an effect
/// can tuck advanced controls behind a header (Shake's per-axis wobble). The
/// group is driven entirely from schema metadata, so any effect adopts it by
/// declaring one in its [`EffectSchema::groups`]; the UI renders the named
/// params under `label` and hides them when the twirl is closed. The member
/// ids must be a contiguous run in the schema's `params` (they render in
/// place, where the group's first member sits).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamGroup {
    /// Sentence-case disclosure header. An EMPTY label renders headerless:
    /// the member rows appear in place with no twirl — the shape a
    /// conditional run of parameters wants (the Lens flare's matte rows).
    pub label: &'static str,
    /// The member parameter ids, naming params in the same schema.
    pub params: &'static [&'static str],
    /// Whether the twirl starts closed (the advanced-by-default case).
    pub collapsed: bool,
    /// When set, the whole group is shown only while the named sibling
    /// Choice parameter holds one of the given indices — how an effect's
    /// panel offers different controls per mode (the Lens flare's Source
    /// type: its matte rows answer to Matte alone, its source-colour toggle
    /// to Matte *and* Lights). None, or an empty set, is always visible.
    pub visible_when: Option<(&'static str, &'static [u32])>,
    /// When set, the group is shown only while the lens in play has at least
    /// this many glass elements.
    ///
    /// The Lens flare offers a coating choice per element, and lenses have
    /// between four and eighteen of them, so the row count has to follow the
    /// lens rather than being fixed by the schema. Each element's row is its
    /// own single-member group carrying its own threshold, and the effect
    /// draws exactly as many as the chosen lens has.
    ///
    /// **It never crosses the bridge as itself.** A group's visibility is
    /// already resolved in the panel from a live sibling value, so the bridge
    /// turns this into precisely that: the sibling is the Lens dropdown and
    /// the values are the lens indices whose prescription has at least this
    /// many elements, worked out from the library at the time the panel asks.
    /// One mechanism in the frontend, not two.
    ///
    /// Recorded limit: a user's own `.lens` file overrides the dropdown, and
    /// only the file knows its element count, so the rows offered then follow
    /// the *picked* lens. An element with no row keeps the file's own coating,
    /// which is what an untouched row does anyway.
    pub visible_when_lens_elements: Option<u32>,
}

/// One parameter's availability depending on another's value: the greyed-out
/// row every host draws when a control has been taken over by a switch beside
/// it.
///
/// **In plain terms.** Some controls stop meaning anything once another control
/// is set a certain way. Tick "Use focus point" and the focus *distance* number
/// is no longer what decides focus — the point is — so leaving the number live
/// would invite you to drag something that does nothing. Every editor's answer
/// is the same: draw the row greyed and refuse the edit, so the panel tells you
/// which of the two is in charge.
///
/// It is declared on the [`EffectSchema`], not inside [`ParamSchema`], for the
/// same reason [`ParamGroup`] is: it names parameters rather than living in
/// one, and putting it here leaves the 130-odd existing parameter literals
/// untouched. The rule is a *UI affordance and a piece of documentation* — the
/// resolve step implements the real branch itself and never consults this, so a
/// schema that forgets a rule renders correctly and merely draws a live control
/// that does nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnabledWhen {
    /// The parameter whose row this governs — the one that greys out.
    pub param: &'static str,
    /// The parameter whose value decides it.
    pub on: &'static str,
    /// What `on` must hold for `param` to be editable.
    pub cond: EnabledCond,
}

/// The condition half of an [`EnabledWhen`].
///
/// Deliberately a small closed set rather than an expression language: these
/// are panel affordances, and every case the built-ins have wanted so far is
/// "a switch is on", "a dropdown is on some entry", or "a layer has been
/// picked". Add a variant when an effect genuinely needs one, and give it a
/// test — do not grow this into a scripting surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnabledCond {
    /// Editable while the named [`ParamKind::Bool`] holds this value.
    BoolIs(bool),
    /// Editable while the named [`ParamKind::Choice`] is on this option index.
    ChoiceIs(u32),
    /// Editable while the named [`ParamKind::Choice`] is on anything *but*
    /// this option index — the "…unless it is set to None" shape.
    ChoiceIsNot(u32),
    /// Editable while the named [`ParamKind::Layer`] actually names a layer.
    /// An unset or dangling reference greys the dependent row.
    LayerSet,
}

/// The Add-effect menu's grouping: every schema declares one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxCategory {
    BlurSharpen,
    Colour,
    Distortion,
    /// The effects that **make** pixels rather than change them: Fill,
    /// Gradient, Noise, Fractal noise. Three of the four never read the incoming
    /// picture at all, which is why none of the other six describes them.
    Generate,
    Stylise,
    Temporal,
    /// The effects that **remove** the picture progressively, so a cut can be
    /// made out of one: Linear wipe, Radial wipe. What they do to a
    /// frame is take some of it away by a Completion the timeline animates —
    /// which is neither a stylisation nor a utility, and is the family AE's Iris
    /// wipe, Venetian blinds and Card wipe join when they land.
    Transition,
    Utility,
    /// The effects that hold a **value** rather than change a picture:
    /// Slider control, Angle control, Checkbox control, Colour control, Point
    /// control. Each is one row an expression reads and the timeline keyframes,
    /// which is what After Effects' Expression Controls are and why half the
    /// rigs in the world are wired through them. They render nothing at all, so
    /// no other category describes them.
    Controls,
    /// The **drivers**: the nodes that make a *value* rather than a
    /// picture — Wiggle, Audio level, Colour cycle, Math, Remap, Smooth. Each
    /// declares a [`Signature::Data`](crate::fx::Signature::Data) instead of an
    /// image kernel, and a wire from one into an effect's socket makes that
    /// parameter follow the value instead of its keyframes.
    ///
    /// Kin to [`FxCategory::Controls`] and deliberately not folded into it: a
    /// Slider control *holds* a number someone typed, a driver *computes* one
    /// every frame, and a menu that mixed the two would be a menu that could
    /// not say which.
    Drivers,
}

impl FxCategory {
    /// Sentence-case menu label.
    pub const fn label(self) -> &'static str {
        match self {
            FxCategory::BlurSharpen => "Blur & sharpen",
            FxCategory::Colour => "Colour",
            FxCategory::Distortion => "Distortion",
            FxCategory::Generate => "Generate",
            FxCategory::Stylise => "Stylise",
            FxCategory::Temporal => "Temporal",
            FxCategory::Transition => "Transition",
            FxCategory::Utility => "Utility",
            FxCategory::Controls => "Controls",
            FxCategory::Drivers => "Drivers",
        }
    }

    /// The family this entry is **browsed** under — every category but
    /// [`FxCategory::Drivers`], which browses under [`FxCategory::Controls`].
    ///
    /// The variant stays: it is what tells a driver from an effect, and the
    /// manual's own Drivers pages are written against it. What merges is only
    /// the *grouping* the application shows — the console's and the browser's
    /// heading — because a hand looking for Wiggle looks under the family that
    /// already holds Slider control, and two headings for one idea is one
    /// heading too many. A driver still lands on the layer's graph rather than
    /// its stack; where it is filed says nothing about what it is.
    pub const fn grouping(self) -> FxCategory {
        match self {
            FxCategory::Drivers => FxCategory::Controls,
            other => other,
        }
    }

    /// Every category, in menu order.
    pub const ALL: [FxCategory; 10] = [
        FxCategory::BlurSharpen,
        FxCategory::Colour,
        FxCategory::Distortion,
        FxCategory::Generate,
        FxCategory::Stylise,
        FxCategory::Temporal,
        FxCategory::Transition,
        FxCategory::Utility,
        // Last, as it is in the Add-effect menu: the menu groups by first
        // appearance in the catalogue, and the Controls family is appended at
        // the end of it.
        FxCategory::Controls,
        // Last of all: a driver is added from the Graph panel's own search
        // rather than from the Add-effect menu, so it sits after the family it
        // is kin to.
        FxCategory::Drivers,
    ];
}

/// The id of the generic Matte layer parameter every effect gains.
///
/// One definition: `#[derive(Effect)]` emits this very const into the injected
/// [`ParamSchema`], the draw builder looks the layer reference up by it, and the
/// panel finds the row by it. A second spelling of the string would be a matte
/// bound to a layer nobody renders.
pub const MATTE_PARAM: &str = "matte";

/// The id of the Invert switch that rides beside [`MATTE_PARAM`].
pub const MATTE_INVERT_PARAM: &str = "matte_invert";

/// [`MATTE_INVERT_PARAM`]'s resolved id — what the generic post-lerp reads the
/// switch out of the bag by, once per op rather than once per effect.
pub const MATTE_INVERT_ID: super::params::ParamId = super::params::ParamId::new(MATTE_INVERT_PARAM);

/// The id of the Channel choice that rides beside [`MATTE_PARAM`]:
/// which channel of the matte layer drives the effect, by the shared
/// [`CHANNEL_OPTIONS`](super::CHANNEL_OPTIONS) index (Luminance by default).
///
/// Injected on every effect that takes the injected matte row and does not
/// already own a channel choice for it (Depth of field, Displacement map and
/// Set matte pick their channels themselves; the Lens flare detects sources).
/// The seam reads it once, in `cpu::matte_prepare` and its WGSL twin, so no
/// kernel learns about it.
pub const MATTE_CHANNEL_PARAM: &str = "matte_channel";

/// [`MATTE_CHANNEL_PARAM`]'s resolved id.
pub const MATTE_CHANNEL_ID: super::params::ParamId =
    super::params::ParamId::new(MATTE_CHANNEL_PARAM);

/// The id every effect's host-uniform Mix slider is declared under (docs/08
/// §1.5). Named here because the seam has to find it: when the injected
/// [`BLEND_PARAM`] is anything but Normal the kernel runs with this forced to
/// 100 and the seam applies the Mix itself, after the blend.
pub const MIX_PARAM: &str = "mix";

/// [`MIX_PARAM`]'s resolved id.
pub const MIX_ID: super::params::ParamId = super::params::ParamId::new(MIX_PARAM);

/// The id of the Blend choice injected beside every Mix slider: how
/// the effect's result combines with its input, by index into
/// [`BlendMode::ALL`](crate::model::BlendMode::ALL) — the layer modes, verbatim.
/// Normal (index 0, the default) is the effect's output unchanged, byte for
/// byte, and no pass runs. The Lens flare declares its own `blend` and keeps
/// it.
pub const BLEND_PARAM: &str = "blend";

/// [`BLEND_PARAM`]'s resolved id.
pub const BLEND_ID: super::params::ParamId = super::params::ParamId::new(BLEND_PARAM);

/// What an effect's Matte row *means*, and therefore who consumes it.
///
/// # In plain terms
///
/// Every effect can be handed a second picture that drives it. For most of them
/// "drives" means strength — the effect runs everywhere and is then dissolved
/// back towards the untouched picture where the matte is dark, which is one
/// dissolve written once for all of them. But for some effects the matte belongs
/// *inside* the maths: a blur that reads a matte should blur softly where the
/// matte is grey, not blur fully and then fade; a glow should only let the lit
/// parts of the matte seed the halo. Those effects claim the matte, and the
/// generic dissolve must then not also run — otherwise the matte would be
/// applied twice, once in the kernel and once beside it.
///
/// This is the whole of that decision, in one place. The draw builder reads it
/// to know which parameter holds the layer reference; `run_ops` reads it to know
/// whether to hand the texture to the kernel or to the dissolve; the derive
/// reads it to know whether to inject the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatteRole {
    /// No matte row, no slot, no dissolve.
    ///
    /// It exists because "every effect" is a claim about the effects that
    /// exist, and an effect that genuinely cannot be driven by a picture should
    /// be able to say so rather than carry a row that does nothing. The
    /// **Controls family** declares it: a Slider control touches no
    /// pixel, so a matte would be a picture gating nothing.
    None,
    /// The generic **strength** semantic: the injected [`MATTE_PARAM`] pair, and
    /// one dissolve beside the registry dispatch
    /// ([`cpu::matte_mix`](super::cpu::matte_mix) and its WGSL twin). The
    /// default, and what all but four effects use.
    Strength,
    /// The effect claims the matte **inside its own maths**. The generic
    /// dissolve does not run.
    Own {
        /// The parameter the layer reference is stored under. [`MATTE_PARAM`]
        /// for an effect that takes the injected row and simply means something
        /// deeper by it (Gaussian blur, Glow), and the effect's own older id
        /// where it owned the idea before the universal matte existed (Depth of
        /// field's `depth`, the Lens flare's `matte`) — a save is a save.
        param: &'static str,
        /// **What this effect's matte does**, in one sentence, sentence case, no
        /// full stop — the schema prose every override carries, and what the
        /// manual prints beside the Matte row (`fx-reference.json`).
        /// The declaration cannot claim the matte without writing it: the two
        /// arrive as one attribute.
        meaning: &'static str,
    },
}

impl MatteRole {
    /// The parameter the matte layer reference is stored under, or `None` when
    /// this effect takes no matte. **The one lookup key**: the draw builder
    /// enumerates slots by it, so a role and a declared parameter that disagree
    /// is a matte bound to a layer nobody renders — which
    /// `every_matte_role_names_a_declared_layer_row` refuses to let happen.
    #[must_use]
    pub const fn param(self) -> Option<&'static str> {
        match self {
            MatteRole::None => None,
            MatteRole::Strength => Some(MATTE_PARAM),
            MatteRole::Own { param, .. } => Some(param),
        }
    }

    /// The one-sentence meaning an override declares, `None` for the generic
    /// strength semantic (whose meaning is the same sentence for every effect
    /// that uses it, and is written once in docs/08 §2.6).
    #[must_use]
    pub const fn meaning(self) -> Option<&'static str> {
        match self {
            MatteRole::Own { meaning, .. } => Some(meaning),
            _ => None,
        }
    }

    /// Whether the generic dissolve runs beside the dispatch. False for an
    /// override, which is the whole point of overriding: the matte is already
    /// spent inside the kernel.
    #[must_use]
    pub const fn generic(self) -> bool {
        matches!(self, MatteRole::Strength)
    }
}

/// One built-in effect's full declaration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectSchema {
    /// Stable match name (participates in the cache key with `version`).
    pub match_name: &'static str,
    pub label: &'static str,
    pub version: u32,
    pub category: FxCategory,
    pub traits: EffectTraits,
    pub params: &'static [ParamSchema],
    /// Collapsible parameter groups (P4): each names a contiguous run
    /// of `params` the Effect Controls tucks behind a twirl. Empty for the
    /// effects that declare none.
    pub groups: &'static [ParamGroup],
    /// Rows that grey out while another parameter says so. Empty for the
    /// effects whose controls are all independent, which is most of them.
    pub enabled_when: &'static [EnabledWhen],
    /// What this effect's Matte row means, and therefore who consumes it — see
    /// [`MatteRole`].
    ///
    /// It is the one fact the generic carriage keys on, and both sides read it:
    /// the draw builder fills one matte slot per op whose role names a
    /// parameter, and `run_ops` consumes one per op whose role names a
    /// parameter — one predicate, one order, the rule every parallel list
    /// follows. Whether the *dissolve* then runs, or the texture goes to
    /// the kernel instead, is the same field's second question
    /// ([`MatteRole::generic`]).
    ///
    /// `#[derive(Effect)]` injects the [`MATTE_PARAM`] pair for any role that
    /// names it and does not already declare it, so no declaration repeats the
    /// row and none can forget it.
    pub matte: MatteRole,
}

impl EffectSchema {
    /// The [`ParamKind::MaskPath`] row this effect declares — its id and
    /// whether an unset row means the layer's first mask.
    ///
    /// **The one predicate.** `build.rs` flattens a slot per op that answers
    /// `Some`, and `fxops::run_ops` consumes one per op that answers `Some`,
    /// in the same order — one predicate and one order, so the two lists
    /// cannot drift apart silently. Anything that needs to know
    /// whether an effect takes a path asks here rather than matching on the
    /// parameter list itself.
    ///
    /// The first declaration wins: an effect takes at most one path, because a
    /// second would need a second carriage and nothing has asked for one.
    /// Whether this effect carries the injected Channel row beside its matte —
    /// and therefore whether the seam prepares the matte (channel pick and
    /// Invert, once) before the kernel or the dissolve sees it. An
    /// effect that owns its channel choice (Depth of field, Displacement map,
    /// Set matte, the Lens flare) carries none and keeps reading the raw RGBA
    /// matte itself, Invert included.
    #[must_use]
    pub fn matte_channel(&self) -> bool {
        self.params.iter().any(|p| p.id == MATTE_CHANNEL_PARAM)
    }

    /// Whether this effect carries the injected Blend row: every
    /// effect with a Mix slider that does not declare a `blend` of its own.
    ///
    /// **The options are the test, not the id.** The Lens flare declares a
    /// `blend` under the same id, holding its own curated light-combine set
    /// (`lens_flare::BLEND_OPTIONS`) which its combine kernel applies itself.
    /// Reading the id alone made the seam blend the flare a second time, by an
    /// index into the wrong menu — a fresh flare (its Add, index 1) came back
    /// as the layer modes' Darken against the untouched input, which on any
    /// picture darker than the flare is black. So the row counts only when it
    /// is the injected one, and the injected one is the row offering the layer
    /// modes verbatim.
    #[must_use]
    pub fn blend(&self) -> bool {
        self.params.iter().any(|p| {
            p.id == BLEND_PARAM
                && matches!(
                    p.kind,
                    ParamKind::Choice { options, .. }
                        if options == crate::model::BlendMode::NAMES
                )
        })
    }

    /// This effect's **vector pairs**: the `foo_x` / `foo_y` runs the panel
    /// draws as one row of two wells with a link glyph between them.
    ///
    /// # In plain terms
    ///
    /// A point has never been a parameter *kind* in Lumit — it is two adjacent
    /// number parameters whose ids end `_x` and `_y`, which is why an effect can
    /// have a Centre without the schema growing a Point type (see
    /// [`ParamKind::Angle`]'s note). That convention was read off the ids at the
    /// seam, in the frontend, by whoever happened to need it. It is read here
    /// now: this is the declaration answering "which of my parameters are two
    /// halves of one thing", so the panel, the link flag on the instance
    /// ([`EffectInstance::pair_linked`](crate::model::EffectInstance::
    /// pair_linked)) and anything else that asks all get one answer.
    ///
    /// The rule is exactly the one the panel already folded rows by, written
    /// down once: **adjacent** in schema order, x then y, the same stem, and
    /// both [`ParamKind::Float`] — so a `_x` with no `_y`, or a pair with a
    /// dropdown wedged between them, is not a pair and is not silently made
    /// one. `every_x_parameter_has_its_y_pair` fails the build on the first
    /// half of that, which is the one an effect declaration can get wrong.
    pub fn pairs(&self) -> impl Iterator<Item = ParamPair> + '_ {
        self.params.windows(2).filter_map(|w| {
            let stem = w[0].id.strip_suffix("_x")?;
            let y = w[1].id.strip_suffix("_y")?;
            (y == stem
                && matches!(w[0].kind, ParamKind::Float { .. })
                && matches!(w[1].kind, ParamKind::Float { .. }))
            .then_some(ParamPair {
                stem,
                x: w[0].id,
                y: w[1].id,
            })
        })
    }

    #[must_use]
    pub fn mask_path(&self) -> Option<(&'static str, bool)> {
        self.mask_paths().next()
    }

    /// **Every** [`ParamKind::MaskPath`] row this effect declares, in
    /// declaration order — id and `self_default` apiece.
    ///
    /// [`Self::mask_path`] answers the first, which is the whole story for the
    /// three effects that walk one line. The Matte key takes two (an inside
    /// hold-out and an outside one), so the carriage counts rows rather than
    /// ops: `build.rs` flattens one polyline per row this yields and
    /// `fxops::run_ops` consumes one per row, in this order. One predicate,
    /// one order — the same rule, now over rows instead of effects.
    pub fn mask_paths(&self) -> impl Iterator<Item = (&'static str, bool)> + '_ {
        self.params.iter().filter_map(|p| match p.kind {
            ParamKind::MaskPath { self_default } => Some((p.id, self_default)),
            _ => None,
        })
    }

    /// How many polylines this effect's slot in the carriage holds — zero for
    /// the effects that take no path at all.
    #[must_use]
    pub fn mask_path_count(&self) -> usize {
        self.mask_paths().count()
    }

    /// The **auxiliary layer** this effect samples beside its own input
    /// ([impl/layer-input.md](../../../docs/impl/layer-input.md)):
    /// the first [`ParamKind::Layer`] row that is not the effect's matte.
    ///
    /// **The one predicate**, exactly as [`EffectSchema::mask_path`] is one:
    /// `build.rs` renders a slot per op that answers `Some` and
    /// `fxops::run_ops` consumes one per op that answers `Some`, in the same
    /// order, so the two lists cannot drift apart silently. It replaced a table
    /// of match names in `build.rs` — a table is a second rule, and a
    /// second rule is a thing to forget when an effect gains a layer row.
    ///
    /// It is deliberately *independent* of the matte carriage and of whatever
    /// else the effect consumes: Motion blur reads a whole flow field and
    /// a Motion vectors layer and a matte, and Set matte reads a layer and no
    /// matte at all. An effect takes at most one auxiliary layer, because a
    /// second would need a second carriage and nothing has asked for one.
    #[must_use]
    pub fn layer_input(&self) -> Option<&'static str> {
        let matte = self.matte.param();
        self.params
            .iter()
            .find(|p| matches!(p.kind, ParamKind::Layer { .. }) && Some(p.id) != matte)
            .map(|p| p.id)
    }
}
