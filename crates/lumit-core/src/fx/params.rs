//! Resolved effect parameters: the key/value form a frame renders from
//! (docs/impl/effect-registry.md §2.3).
//!
//! **In plain terms.** An effect's controls are stored in the project as
//! animatable properties. Once a frame knows what time it is, those properties
//! become plain numbers — and this module is the shape those numbers take on the
//! way to the GPU. It is a small list of `(which control, what number)` pairs
//! rather than a variant of one giant enum listing every effect there will ever
//! be, which is what lets an effect carry controls nobody wrote down when Lumit
//! was compiled: a slider the user added, a uniform read out of a shader they
//! typed.
//!
//! Two properties are load-bearing and are the reason this is not simply a
//! `HashMap<String, f32>`:
//!
//! - **No allocation per effect.** One layer's whole stack resolves into a
//!   single [`ResolvedStack`] with one allocation for its parameters — one fewer
//!   than the `Vec<Resolved>` it replaces. A [`ResolvedFx`] borrows a run of it
//!   and is `Copy`, so it is passed around like the old plain-old-data enum was.
//! - **Determinism.** The resolved stack feeds the frame key (K-143), so it is
//!   hashed field by field through [`ResolvedStack::feed_hash`] — never
//!   byte-wise, because [`Value`] has padding and a padding byte in a cache key
//!   is a wrong picture that reproduces on one machine only.

use std::ops::Range;

use uuid::Uuid;

/// A parameter's identity: the FNV-1a 64 hash of its stable snake_case id.
///
/// Hashing the id rather than storing it keeps [`Value`] lookups to a comparison
/// of two `u64`s, and keeps the resolved form free of borrowed strings — a
/// dynamic parameter's id is owned by the document, not by `'static` schema
/// data, so `&'static str` was never an option for the general case.
///
/// The hash is computed in a `const fn`, so a built-in's ids are compile-time
/// constants: the derive macro emits one `const` per parameter and a lookup
/// never hashes at run time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamId(pub u64);

impl ParamId {
    /// The id of the parameter named `id`.
    ///
    /// FNV-1a 64. Chosen over a cryptographic hash because it is `const`-able in
    /// a few lines and the input space is one effect's ~50 snake_case ASCII ids;
    /// the catalogue test checks every built-in for pairwise-distinct hashes, and
    /// the edit path refuses a dynamic parameter whose id collides on its
    /// instance (docs/impl/effect-registry.md §5).
    pub const fn new(id: &str) -> Self {
        let bytes = id.as_bytes();
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut i = 0;
        while i < bytes.len() {
            hash ^= bytes[i] as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            i += 1;
        }
        Self(hash)
    }
}

/// The unit a numeric parameter is declared in (docs/08 §2.3, docs/impl/
/// effect-registry.md §2.2).
///
/// This is what lets the preview-raster rescale be one generic pass instead of a
/// match that has to know which field of which effect holds a pixel count. An
/// effect cannot forget to be rescaled, which was possible when the knowledge
/// lived in `rescale_px`.
/// It is also **what the panel writes beside the number** (K-443's unit rider:
/// "px", "%", "°", "s", "f"). That is the second reason every declaration has to
/// carry one: the alternative was a table in the frontend saying which ids are
/// pixels and which are percentages, which is the engine's knowledge kept in the
/// view — and it was already wrong, because it keyed on the parameter's id alone
/// while `centre_x` means % of comp width on Radial blur and px@comp on four
/// other effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// **Nobody has decided yet.** The derive's default for a numeric
    /// parameter whose declaration says nothing, and a defect rather than a
    /// unit: `every_parameter_declares_a_deliberate_unit` fails the build on
    /// any parameter that ships in it.
    ///
    /// It exists so that "dimensionless" and "unconsidered" are different
    /// answers. They were the same answer while [`Unit::Raw`] was the silent
    /// default, which is how a pixel count could reach the panel with no unit
    /// beside it and nothing to notice.
    Unset,
    /// A plain number: a gamma, a count, a threshold, a stop, a rate in Hz.
    /// The deliberate "none" — the panel draws no unit rider.
    Raw,
    /// A percentage, where 100 is the whole of whatever the parameter is a
    /// share of: the host-uniform Mix, a channel's share of a grain, Radial
    /// blur's centre as a fraction of the comp's width and height.
    ///
    /// **Not a distance.** A per cent of the *frame* is a position that means
    /// the same thing at any raster, so it is not scaled by the preview factor;
    /// a per cent of the *diagonal* is [`Unit::PctDiag`], which K-419 forbids
    /// any parameter from declaring.
    Percent,
    /// A percentage of the composition diagonal. **No parameter may declare
    /// it** (K-419: every distance is px@comp); it stays for the ROI padding
    /// declarations and the reference format, and
    /// `no_parameter_is_a_per_cent_of_the_diagonal` enforces the rule.
    PctDiag,
    /// Pixels at composition size (px@comp), converted to the raster in play by
    /// the resolve step — the resolution-independent form docs/08 §2.3
    /// requires of anything spatial. Never "pixels of whatever buffer I was
    /// handed", which §2.3 forbids.
    Px,
    /// Degrees. Unbounded: an angle animates through full turns.
    Degrees,
    /// Seconds of layer time.
    Seconds,
    /// Comp-rate frames — docs/08 §2.3's other duration unit, for the handful
    /// of controls a frame count is the honest spelling of (Flash's Duration
    /// and Phase, Datamosh's reach along the flow).
    Frames,
}

impl Unit {
    /// Whether a value in this unit follows the raster the frame renders at, and
    /// so is scaled by the preview factor and rescaled when a resolved stack is
    /// reused at another size.
    pub const fn is_spatial(self) -> bool {
        matches!(self, Unit::PctDiag | Unit::Px)
    }
}

/// One parameter, resolved to plain numbers at a frame.
///
/// Deliberately small and `Copy`: no owned allocations, nothing borrowed from the
/// document. A layer reference resolves to *whether* it is bound and a file
/// reference to a slot, because the texture and the path ride beside the op
/// exactly as they do today (docs/impl/layer-input.md).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Float(f32),
    Int(i32),
    Bool(bool),
    /// A Choice option index.
    Choice(u32),
    /// Scene-linear RGBA.
    Colour([f32; 4]),
    /// Four plain floats under one id (K-388): a small fixed vector an effect
    /// would otherwise have to spell as four `derived.*` entries. Shake's
    /// unit-free noise sample `(x, y, rotation, z)` is the first — nine of them
    /// when its own motion blur is on, which as forty flat ids would drown the
    /// bag. Deliberately not a `Colour`: these are not colours, and a kind that
    /// lies about what it holds is a kind the panel and the bridge would have to
    /// second-guess.
    Vec4([f32; 4]),
    /// Whether the layer reference names a layer that was rendered for it.
    Layer(bool),
    /// The op's slot in the stack's file table, or `u32::MAX` for unset.
    File(u32),
    /// Whether the mask-path row names a mask, exactly as [`Value::Layer`]
    /// carries whether the layer row names a layer (K-408).
    ///
    /// The *choice* is what the document stores (a mask id, or the "First
    /// mask" entry); the *geometry* rides beside the op as a flattened
    /// polyline. Neither belongs in the arena: this bag is `Copy`, borrows
    /// nothing from the document, and is hashed field by field into the frame
    /// key — a path of a thousand vertices in it would be all three of those
    /// promises broken. What is left for the bag is the one bit a kernel can
    /// use before it looks at its slot. Whether a path actually arrived is
    /// `AuxSlot`'s answer, not this one: an unset row on a masked layer still
    /// resolves to the first mask, and a named mask can have been deleted.
    MaskPath(bool),
    /// A tone curve's own control points (K-412), inline.
    ///
    /// The one value that carries a *shape* rather than a scalar, and it is
    /// here rather than beside the op — the way a mask path is — for the
    /// opposite reason to the mask path's: a curve is at most sixteen pairs
    /// of numbers the user typed, small enough to stay `Copy`, to be hashed
    /// field by field into the frame key, and to borrow nothing from the
    /// document. What it costs is the width of every arena slot, since an
    /// enum is as wide as its widest variant; what it buys is that both
    /// render paths bake the identical table from the identical points
    /// through [`Curves::packed`](crate::fx::effects::curves::Curves::packed),
    /// with no second list to keep in step. Move it beside the op if the
    /// arena's width ever shows up in a profile — nothing above here reads
    /// the points except that one `packed`.
    Curve(CurvePoints),
}

/// The most control points a [`ParamKind::Curve`](crate::fx::ParamKind::Curve)
/// carries (K-412). Sixteen is well past what a grade needs and keeps the
/// inline form small.
pub const CURVE_MAX_POINTS: usize = 16;

/// The identity diagonal: the default curve, and what a malformed one falls
/// back to.
pub const CURVE_IDENTITY: [[f32; 2]; 2] = [[0.0, 0.0], [1.0, 1.0]];

/// One tone curve's control points, in the unit square, ordered by x (K-412).
///
/// Fixed-size and `Copy` so it can live in the arena. Only the first `len`
/// entries mean anything; the rest are zero, which is what lets `PartialEq`
/// and the frame key treat two equal curves as equal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoints {
    xy: [[f32; 2]; CURVE_MAX_POINTS],
    len: u32,
}

impl CurvePoints {
    /// The identity diagonal — a fresh curve, and the fallback for one that
    /// arrives unreadable.
    pub const IDENTITY: Self = {
        let mut xy = [[0.0; 2]; CURVE_MAX_POINTS];
        xy[1] = [1.0, 1.0];
        Self { xy, len: 2 }
    };

    /// Read an arbitrary point list into the canonical form: clamped into the
    /// unit square, sorted by x, repeated x dropped, capped at
    /// [`CURVE_MAX_POINTS`], and replaced by [`Self::IDENTITY`] when fewer
    /// than two points survive.
    ///
    /// Quiet on purpose. The list comes off a document that a hand, an older
    /// build or an importer wrote, and a curve out of order is a curve to
    /// straighten, not a panic (14-ENGINEERING-RULES §4). Deterministic: the
    /// sort is stable and the survivor of a repeated x is always the first,
    /// so the same list always reads to the same curve.
    #[must_use]
    pub fn sanitised(points: &[[f32; 2]]) -> Self {
        let mut sorted: Vec<[f32; 2]> = points
            .iter()
            .map(|p| {
                [
                    if p[0].is_nan() {
                        0.0
                    } else {
                        p[0].clamp(0.0, 1.0)
                    },
                    if p[1].is_nan() {
                        0.0
                    } else {
                        p[1].clamp(0.0, 1.0)
                    },
                ]
            })
            .collect();
        sorted.sort_by(|a, b| a[0].total_cmp(&b[0]));

        let mut out = Self {
            xy: [[0.0; 2]; CURVE_MAX_POINTS],
            len: 0,
        };
        for p in sorted {
            if out.len as usize >= CURVE_MAX_POINTS {
                break;
            }
            // Two points at one x have no curve between them, and the spline
            // would divide by that zero gap. The first wins.
            if out.len > 0 && p[0] <= out.xy[out.len as usize - 1][0] {
                continue;
            }
            out.xy[out.len as usize] = p;
            out.len += 1;
        }
        if out.len < 2 {
            return Self::IDENTITY;
        }
        out
    }

    /// The points, in x order.
    #[must_use]
    pub fn points(&self) -> &[[f32; 2]] {
        &self.xy[..self.len as usize]
    }
}

impl Default for CurvePoints {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Value {
    /// This value as a float, whatever kind it is — the accessor the maths
    /// actually want. Bools are 0/1 and Choices are their index, matching the
    /// wire form every WGSL kernel already reads.
    pub fn as_f32(self) -> f32 {
        match self {
            Value::Float(v) => v,
            Value::Int(v) => v as f32,
            Value::Bool(v) => {
                if v {
                    1.0
                } else {
                    0.0
                }
            }
            Value::Choice(v) => v as f32,
            Value::Colour([r, ..]) | Value::Vec4([r, ..]) => r,
            Value::Layer(v) | Value::MaskPath(v) => {
                if v {
                    1.0
                } else {
                    0.0
                }
            }
            Value::File(v) => v as f32,
            // A shape is not a number; its first point's output is the least
            // misleading scalar to give a caller that asked anyway.
            Value::Curve(c) => c.xy[0][1],
        }
    }

    /// The tag byte fed to the frame key, so two kinds holding the same number
    /// cannot hash alike.
    const fn tag(self) -> u8 {
        match self {
            Value::Float(_) => 0,
            Value::Int(_) => 1,
            Value::Bool(_) => 2,
            Value::Choice(_) => 3,
            Value::Colour(_) => 4,
            Value::Layer(_) => 5,
            Value::File(_) => 6,
            Value::Vec4(_) => 7,
            Value::MaskPath(_) => 8,
            Value::Curve(_) => 9,
        }
    }
}

/// A borrowed run of resolved parameters: one effect's worth.
///
/// `Copy`, because it borrows the stack's arena rather than owning anything —
/// which is what lets it be passed by value everywhere the old `Resolved` enum
/// was, without the enum's compile-time closed set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params<'a> {
    entries: &'a [(ParamId, Value)],
}

impl<'a> Params<'a> {
    /// A view over an explicit slice — the shape tests and the CPU oracle build
    /// directly.
    pub const fn new(entries: &'a [(ParamId, Value)]) -> Self {
        Self { entries }
    }

    /// The empty set (an effect with no parameters, such as Invert).
    pub const EMPTY: Params<'static> = Params { entries: &[] };

    /// Every parameter, in schema order. The order is a promise: the panel, the
    /// bridge and the cache key all read it (docs/impl/effect-registry.md §5).
    pub fn iter(&self) -> impl Iterator<Item = (ParamId, Value)> + 'a {
        self.entries.iter().copied()
    }

    /// How many parameters this effect resolved to.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the effect resolved to no parameters at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The value of `id`, or `None` if this instance does not carry it — which
    /// is an ordinary state, not a fault: a project saved before a parameter was
    /// added simply has no entry, and the typed reader supplies the declared
    /// default (K-258).
    ///
    /// A short linear scan over adjacent memory. An effect has at most ~50
    /// parameters and almost always fewer than ten, so this beats hashing into a
    /// map, and it keeps the arena contiguous.
    pub fn get(&self, id: ParamId) -> Option<Value> {
        self.entries.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
    }

    /// `id` as a float, or `default` when absent or of another kind.
    pub fn float(&self, id: ParamId, default: f32) -> f32 {
        match self.get(id) {
            Some(Value::Float(v)) => v,
            Some(Value::Int(v)) => v as f32,
            _ => default,
        }
    }

    /// `id` as a whole number, or `default` when absent or of another kind.
    pub fn int(&self, id: ParamId, default: i32) -> i32 {
        match self.get(id) {
            Some(Value::Int(v)) => v,
            Some(Value::Float(v)) => v.round() as i32,
            _ => default,
        }
    }

    /// `id` as a switch, or `default` when absent or of another kind.
    pub fn bool(&self, id: ParamId, default: bool) -> bool {
        match self.get(id) {
            Some(Value::Bool(v)) => v,
            _ => default,
        }
    }

    /// `id` as a Choice option index, or `default` when absent or of another
    /// kind. Unknown indices are the caller's business to clamp — an effect
    /// reads its own choices through its generated enum, which does exactly that.
    pub fn choice(&self, id: ParamId, default: u32) -> u32 {
        match self.get(id) {
            Some(Value::Choice(v)) => v,
            _ => default,
        }
    }

    /// `id` as scene-linear RGBA, or `default` when absent or of another kind.
    pub fn colour(&self, id: ParamId, default: [f32; 4]) -> [f32; 4] {
        match self.get(id) {
            Some(Value::Colour(v)) => v,
            _ => default,
        }
    }

    /// `id` as four plain floats, or `default` when absent or of another kind
    /// (K-388). A `Colour` is deliberately *not* accepted here: the two kinds
    /// mean different things, and reading one as the other would be the silent
    /// mistake the tag exists to prevent.
    pub fn vec4(&self, id: ParamId, default: [f32; 4]) -> [f32; 4] {
        match self.get(id) {
            Some(Value::Vec4(v)) => v,
            _ => default,
        }
    }

    /// Whether the layer reference `id` is bound to a picture this frame.
    pub fn layer_bound(&self, id: ParamId) -> bool {
        matches!(self.get(id), Some(Value::Layer(true)))
    }

    /// Whether the mask-path row `id` names a mask (K-408). Not whether a
    /// path arrived — that is the op's slot's answer, and the honest one.
    pub fn mask_named(&self, id: ParamId) -> bool {
        matches!(self.get(id), Some(Value::MaskPath(true)))
    }

    /// The tone curve `id`, or the identity diagonal when absent or of
    /// another kind (K-412) — a missing curve is a straight line, never a
    /// fault.
    pub fn curve(&self, id: ParamId) -> CurvePoints {
        match self.get(id) {
            Some(Value::Curve(c)) => c,
            _ => CurvePoints::IDENTITY,
        }
    }

    /// The file-table slot for `id`, or `None` when unset — which resolves to
    /// identity, never a fault (docs/08 §1.2).
    pub fn file_slot(&self, id: ParamId) -> Option<u32> {
        match self.get(id) {
            Some(Value::File(slot)) if slot != u32::MAX => Some(slot),
            _ => None,
        }
    }
}

/// One resolved effect: which effect it is, which instance it came from, and its
/// parameters.
///
/// `Copy`. The definition is a `'static` trait object from the registry, so
/// dispatch is a virtual call rather than a match over a closed set — the whole
/// point of the exercise (docs/impl/effect-registry.md §2.4).
#[derive(Clone, Copy)]
pub struct ResolvedFx<'a> {
    /// The effect's definition, from the registry.
    pub def: &'static dyn EffectDef,
    /// The instance this resolved from, so a diagnostic can name the row and the
    /// auxiliary inputs can be matched up one-for-one.
    pub instance: Uuid,
    /// The layer time this op's parameters were evaluated at (K-593) — what a
    /// plugin's render is told the frame is. Nought for a hand-built stack.
    pub lt: f64,
    /// The resolved parameters, in schema order.
    pub params: Params<'a>,
}

impl std::fmt::Debug for ResolvedFx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedFx")
            .field("match_name", &self.def.schema().match_name)
            .field("instance", &self.instance)
            .field("params", &self.params)
            .finish()
    }
}

/// One effect's entry in the arena: which effect, and where its parameters sit.
#[derive(Clone)]
struct Op {
    def: &'static dyn EffectDef,
    instance: Uuid,
    /// The layer time this op's parameters were evaluated at (K-593). Nought
    /// for a stack a test built by hand, which is what [`ResolvedStack::begin`]
    /// keeps meaning.
    lt: f64,
    span: Range<u32>,
    /// The rows this **instance** declared beyond its schema's — a Custom
    /// shader's own uniforms (docs/impl/custom-shader.md §1.5), empty for every
    /// other effect. Held so [`ResolvedStack::rescale_spatial`] can find a
    /// derived parameter's unit; without it a shader's `@unit(px)` radius would
    /// be left behind when a stack resolved at one raster is reused at another.
    derived: &'static [ParamSchema],
}

/// Everything one layer's effect stack resolved to at one frame.
///
/// The parameters of every op live contiguously in one `Vec`; an op holds the
/// range of it that is its own. That is one allocation for a whole stack, and
/// borrowing a run of it is what makes [`ResolvedFx`] `Copy`.
#[derive(Clone, Default)]
pub struct ResolvedStack {
    ops: Vec<Op>,
    entries: Vec<(ParamId, Value)>,
}

impl ResolvedStack {
    /// An empty stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new op. Parameters pushed after this belong to it, until the next
    /// `begin` or the end of the stack.
    pub fn begin(&mut self, def: &'static dyn EffectDef, instance: Uuid) {
        self.begin_at(def, instance, 0.0);
    }

    /// [`begin`](Self::begin), told the layer time the op resolved at (K-593).
    ///
    /// Only one kind of effect reads it: a **plugin**, whose render is a
    /// conversation with somebody else's code and whose `kOfxPropTime` has to be
    /// the frame actually being asked for. Every built-in's parameters are
    /// already evaluated by the time they reach the arena, which is why `begin`
    /// stays the plain spelling and nothing had to change to keep working.
    pub fn begin_at(&mut self, def: &'static dyn EffectDef, instance: Uuid, lt: f64) {
        let start = self.entries.len() as u32;
        self.ops.push(Op {
            def,
            instance,
            lt,
            span: start..start,
            derived: &[],
        });
    }

    /// Tell the op most recently begun which rows its **instance** declared
    /// beyond its schema's (docs/impl/custom-shader.md §1.5). Ignored before the
    /// first `begin`, exactly as [`Self::push`] is.
    pub fn set_derived(&mut self, rows: &'static [ParamSchema]) {
        if let Some(op) = self.ops.last_mut() {
            op.derived = rows;
        }
    }

    /// Add a resolved parameter to the op most recently begun. Silently ignored
    /// before the first `begin` — engine code does not panic on a caller's
    /// ordering mistake (14-ENGINEERING-RULES §4), and the catalogue tests are
    /// what catch it.
    pub fn push(&mut self, id: ParamId, value: Value) {
        if let Some(op) = self.ops.last_mut() {
            self.entries.push((id, value));
            op.span.end = self.entries.len() as u32;
        }
    }

    /// Drop the op most recently begun, and its parameters with it — how an
    /// effect that resolves to nothing this frame (an unset file, a zero mix) is
    /// withdrawn after the fact.
    pub fn drop_last(&mut self) {
        if let Some(op) = self.ops.pop() {
            self.entries.truncate(op.span.start as usize);
        }
    }

    /// Append another resolved stack's ops after this one's, in order (K-706).
    ///
    /// **In plain terms.** A layer's styles resolve in a second walk of their
    /// own, and their ops then run after the effect stack's on the same raster
    /// (docs/impl/layer-styles.md §3). This is the join. Each incoming op's
    /// parameters are copied into this arena and its range moved to where they
    /// landed, so the result is one arena and one order — indistinguishable
    /// downstream from a stack that resolved in a single walk.
    ///
    /// The one thing to know: an op's `span` is an index into the arena it came
    /// from, so it cannot simply be carried across. Shifting by the number of
    /// entries already here is what re-points it.
    pub fn append(&mut self, other: Self) {
        let shift = self.entries.len() as u32;
        self.entries.extend(other.entries);
        self.ops.extend(other.ops.into_iter().map(|mut op| {
            op.span = (op.span.start + shift)..(op.span.end + shift);
            op
        }));
    }

    /// How many ops the stack resolved to.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether the stack resolved to nothing at all.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// The op at `index`, in stack order.
    pub fn get(&self, index: usize) -> Option<ResolvedFx<'_>> {
        let op = self.ops.get(index)?;
        let entries = self
            .entries
            .get(op.span.start as usize..op.span.end as usize)?;
        Some(ResolvedFx {
            def: op.def,
            instance: op.instance,
            lt: op.lt,
            params: Params::new(entries),
        })
    }

    /// Every op, in stack order.
    pub fn iter(&self) -> impl Iterator<Item = ResolvedFx<'_>> + '_ {
        (0..self.ops.len()).filter_map(|i| self.get(i))
    }

    /// Rescale every spatial value by `factor` — a stack resolved against one
    /// raster and run on another (K-266).
    ///
    /// This is the repair for the Adjust arm of the draw builder, which resolves
    /// with `px_scale` 1 because its stack runs on "the comp-sized
    /// intermediate", which is only true at full preview resolution. Under
    /// reduced-resolution preview every spatial parameter (the flare's light,
    /// DoF apertures, blur radii, Shake's amplitude) landed too far right and
    /// too big by exactly the preview factor; the owner measured the flare's
    /// light hitting the frame edge at 1500 of a 1920 comp. The realise walk
    /// calls this with `render_width / comp_width` before running an adjustment
    /// stack, and a precomp realised at a wider size needs the same.
    ///
    /// Every op's parameters are in the arena, and the arena declares its units,
    /// so this is one generic pass and no effect can be forgotten — which the
    /// per-variant `rescale_px` match it replaced could not promise.
    pub fn rescale_spatial(&mut self, factor: f32) {
        if factor == 1.0 {
            return;
        }
        for op in &self.ops {
            let schema = op.def.schema();
            for slot in self
                .entries
                .get_mut(op.span.start as usize..op.span.end as usize)
                .unwrap_or(&mut [])
            {
                let spatial = schema
                    .params
                    .iter()
                    .chain(op.derived)
                    .find(|p| ParamId::new(p.id) == slot.0)
                    .is_some_and(|p| p.unit.is_spatial());
                if spatial {
                    if let Value::Float(v) = slot.1 {
                        slot.1 = Value::Float(v * factor);
                    }
                }
            }
        }
    }

    /// Feed the whole stack into a frame key, field by field — every op's
    /// [`ResolvedFx::feed_hash`] in stack order.
    ///
    /// Takes the sink rather than a hasher so the engine root does not gain a
    /// hashing dependency for one method; `lumit-eval` passes its blake3 hasher.
    pub fn feed_hash(&self, feed: &mut dyn FnMut(&[u8])) {
        for fx in self.iter() {
            fx.feed_hash(feed);
        }
    }
}

impl ResolvedFx<'_> {
    /// Feed this one op into a key: its effect's name and algorithm version,
    /// then every parameter, field by field.
    ///
    /// Never byte-wise: [`Value`] has padding, and an uninitialised byte in a
    /// cache key is a wrong picture that reproduces on one machine only
    /// (docs/impl/effect-registry.md §5). The instance id is deliberately
    /// absent — a key names content, never which row it came from (docs/06
    /// §5.2) — which is what lets the per-effect cache (K-421) serve a
    /// duplicated layer from its original's intermediates.
    pub fn feed_hash(&self, feed: &mut dyn FnMut(&[u8])) {
        feed(self.def.schema().match_name.as_bytes());
        feed(b"/");
        feed(&self.def.schema().version.to_le_bytes());
        for (id, value) in self.params.iter() {
            feed(&id.0.to_le_bytes());
            feed(&[value.tag()]);
            match value {
                Value::Float(v) => feed(&v.to_le_bytes()),
                Value::Int(v) => feed(&v.to_le_bytes()),
                Value::Bool(v) | Value::Layer(v) | Value::MaskPath(v) => feed(&[u8::from(v)]),
                Value::Choice(v) | Value::File(v) => feed(&v.to_le_bytes()),
                // Four floats, one at a time — the tag above is what keeps a
                // Colour and a Vec4 of the same numbers apart.
                Value::Colour(c) | Value::Vec4(c) => {
                    for ch in c {
                        feed(&ch.to_le_bytes());
                    }
                }
                // The live points only — the tail of the fixed array is
                // padding by another name, and padding never feeds a key.
                // The length goes in first, so two curves sharing a
                // prefix cannot hash alike.
                Value::Curve(c) => {
                    feed(&c.len.to_le_bytes());
                    for p in c.points() {
                        feed(&p[0].to_le_bytes());
                        feed(&p[1].to_le_bytes());
                    }
                }
            }
        }
    }
}

use super::registry::EffectDef;
use super::schema::ParamSchema;

impl std::fmt::Debug for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Op")
            .field("match_name", &self.def.schema().match_name)
            .field("instance", &self.instance)
            .field("span", &self.span)
            .finish()
    }
}

impl std::fmt::Debug for ResolvedStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
