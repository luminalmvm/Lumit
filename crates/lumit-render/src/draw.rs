//! The draw list: what one composited comp frame is made of, as plain data.
//!
//! # In plain terms
//!
//! Between "here is the document" and "here are the finished pixels" sits one
//! honest, inspectable middle step: a **draw list**. Each entry
//! ([`CompLayerDraw`]) says *this picture, at this size, placed here, at this
//! opacity, through this blend mode, with these effects already resolved to
//! plain numbers*. Nothing in this file touches the graphics card, opens a file,
//! or knows which frontend is asking — it is a description, not an action.
//!
//! That separation is the whole point. [`crate::build`] turns a document into a
//! draw list (cheap, CPU-only, no decoding), and [`crate::realise`] turns a draw
//! list into a texture (the expensive GPU part). Because building is cheap and
//! reuses already-decoded pixels, dragging an effect value re-runs only the
//! build and the composite — the video frames underneath are never decoded
//! again. That is what makes a value drag feel instant.

/// One decoded layer ready to composite (evaluator v0).
pub struct MatteDraw {
    pub rgba: Vec<u8>,
    pub tex_w: u32,
    pub tex_h: u32,
    pub natural_size: (f32, f32),
    pub position: (f32, f32),
    pub anchor: (f32, f32),
    pub scale: (f32, f32),
    pub rotation_deg: f32,
    pub opacity: f32,
    pub z: f32,
    pub rotation_x_deg: f32,
    pub rotation_y_deg: f32,
    pub three_d: bool,
    pub luma: bool,
    pub inverted: bool,
    /// The matte source's own effect stack, resolved at the matte's layer time
    /// (docs/impl/layer-input.md; K-142). Non-empty only when the consumer's
    /// `MatteRef::source` is `EffectsAndMasks` — the effects then run on the matte
    /// texture (upload → linearise → `run_ops`) before it is composited alone, so
    /// a keyed or blurred matte gates by its processed pixels. Empty for None /
    /// Masks (the raw pixels carried in `rgba`, with or without masks baked in).
    /// Temporal inputs (neighbours/flow/depth) are not fed through an
    /// effects-and-masks matte in v1, so an echo or flow effect on the matte
    /// source degrades to a still (documented boundary).
    pub fx: lumit_core::fx::ResolvedStack,
    /// The matte source's `lut` file paths, 1:1 and in order with the `lut`
    /// ops in `fx` (as for a layer's own `lut_files`). Empty unless the
    /// source mode is `EffectsAndMasks` and the matte source has a LUT.
    pub lut_files: Vec<Option<String>>,
    /// Set when the matte source is a **Precomp** (K-268): the nested comp's
    /// own draw list, realised recursively exactly as a Precomp layer's
    /// picture is — `rgba` is then empty and `tex_w`/`tex_h` are the nested
    /// comp's size. A comp has no pixels until it is rendered, so `pixels_for`
    /// answers None for one, and a track matte set to a precomp silently
    /// gated nothing at all until this field existed (the layer-input twin of
    /// the same hole was K-266's `DofInputDraw::nested`). The source-mode
    /// masks/effects toggles do not apply to a comp reference — the comp
    /// renders as itself, its layers' own masks and effects included.
    pub nested: Option<Box<NestedInputDraw>>,
}

/// A depth-of-field depth input packaged for the compositor (docs/impl/
/// layer-input.md §2): the referenced layer's **source** pixels, ready for
/// [`crate::fxops::render_layer_input`] to resample into the consuming layer's
/// working raster. The referenced layer is rendered source-only (its own
/// effect stack is not applied), exactly as a matte source is — so a depth
/// reference can never recurse into another effect, and the preview and export
/// threads produce the same depth pass (K-031).
pub struct DofInputDraw {
    pub rgba: Vec<u8>,
    pub tex_w: u32,
    pub tex_h: u32,
    /// The depth layer's own effect stack, resolved at its layer time — run on
    /// the depth texture before it is resampled, when the consuming effect's
    /// depth source is `EffectsAndMasks` (K-142, mirroring the matte). Empty for
    /// None / Masks (the raw pixels are carried in `rgba`). Temporal inputs are
    /// not fed through an effects-and-masks depth input in v1 (matte boundary).
    pub fx: lumit_core::fx::ResolvedStack,
    /// The depth layer's `lut` file paths, 1:1 with the `lut` ops in
    /// `fx`. Empty unless the depth source is `EffectsAndMasks` and the depth
    /// layer has a LUT.
    pub lut_files: Vec<Option<String>>,
    /// Set when the referenced layer is a **Precomp** (K-266): the nested
    /// comp's own draw list, realised recursively exactly as a Precomp
    /// layer's picture is — `rgba` is then empty and `tex_w`/`tex_h` are the
    /// nested comp's size. `pixels_for` has no pixels for a comp (they only
    /// exist on the GPU), which is why "a white circle in a precomp" as a
    /// flare matte silently detected nothing until this field existed. The
    /// source-mode masks/effects toggles do not apply to a comp reference —
    /// the comp renders as itself, its layers' own effects included.
    pub nested: Option<Box<NestedInputDraw>>,
}

/// What a layer-input parameter resolves to for one effect op (docs/impl/
/// layer-input.md, K-288): nothing, this effect's own input, or another
/// layer's picture. One of these per op that declares a Layer parameter,
/// 1:1 and in order with those ops.
pub enum LayerInputDraw {
    /// Unset, dangling, out of its time span, or not in a mode that reads
    /// one — the effect degrades to its labelled no-op, never a fault.
    Absent,
    /// The reference points at the layer the effect is **on** (K-288), so
    /// the input is that effect's own input at its point in the stack. No
    /// second render happens: `run_ops` binds the texture it is already
    /// carrying. On an adjustment layer that texture is the composite of
    /// everything below, which is the only picture an adjustment layer has —
    /// so a Lens flare added to one finds the lights in the footage beneath
    /// it without being pointed anywhere.
    ThisLayer,
    /// Another layer, rendered alone at this raster.
    Layer(DofInputDraw),
}

/// A layer-input's nested comp render (K-266) — the [`DrawSource::Nested`]
/// shape, boxed onto [`DofInputDraw`].
pub struct NestedInputDraw {
    pub width: u32,
    pub height: u32,
    pub background: [f64; 4],
    pub draws: Vec<CompLayerDraw>,
    pub camera: Option<lumit_core::model::CameraPose>,
}

/// Where a draw's pixels come from: decoded/synthesised bytes, or a nested
/// comp realised recursively on the GPU (Precomp layers).
pub enum DrawSource {
    Pixels {
        rgba: Vec<u8>,
        tex_w: u32,
        tex_h: u32,
    },
    Nested {
        width: u32,
        height: u32,
        background: [f64; 4],
        draws: Vec<CompLayerDraw>,
        /// The nested comp's own active camera at this time.
        camera: Option<lumit_core::model::CameraPose>,
    },
    /// An adjustment layer's staging point (docs/06 §1.5): no pixels of its
    /// own — the draw's `fx` runs on the composite of every draw before it,
    /// and its placement/opacity/`mask_cov` shape the coverage that
    /// attenuates the result. Only emitted with a live, non-empty stack.
    Adjust,
}

/// The below-stack re-rendered at a held/sample time for a temporal adjustment
/// (Posterize Time, docs/08 §3.25; docs/impl/temporal-rerender.md): the draws
/// beneath the effect's layer, built by `build_comp_draws` at the held comp
/// time `tau`, plus the comp's camera at `tau`. Carried on the adjustment's
/// [`DrawSource::Adjust`] draw so `Realiser::realise` composites the held
/// version in place of the plain below-composite — the same `build_comp_draws`
/// + `realise` export drives, so preview equals export (K-031).
pub struct TemporalBelow {
    pub draws: Vec<CompLayerDraw>,
    pub camera: Option<lumit_core::model::CameraPose>,
}

/// The below-stack re-rendered at N sub-frame times for an accumulation motion
/// blur adjustment (docs/08 §3.26; docs/impl/temporal-rerender.md §3): one draw
/// list + camera per shutter sample. `Realiser::realise` renders each, averages
/// the N finished composites with the hardware additive-at-1/N pass
/// ([`lumit_gpu::Compositor::accumulate`]), then blends that average against the
/// plain frame-time below-composite by `mix`, and the result stands in for the
/// below-composite the adjustment's own effects and coverage blend see — the
/// same `render_below_at` (via `below_draws_at`) export drives, so preview equals
/// export (K-031). Carried on the adjustment's [`DrawSource::Adjust`] draw; None
/// on every ordinary draw and every non-accumulation adjustment.
pub struct AccumulationBelow {
    /// One below-stack draw list + camera per sub-frame sample time `τ_k`.
    pub samples: Vec<(Vec<CompLayerDraw>, Option<lumit_core::model::CameraPose>)>,
    /// Averaged-over-original blend, 0..1 (1 = full accumulation blur).
    pub mix: f32,
    /// **The Matte, scaling Shutter angle per pixel** (K-429, docs/08 §2.6).
    /// [`LayerInputDraw::Absent`] is the whole of the old behaviour: equal
    /// weights, one hardware additive pass, byte for byte what it was (K-258).
    /// It is carried here rather than on the effect's op because this effect
    /// resolves to no op at all — it orchestrates a re-render — so the matte
    /// carriage `run_ops` walks skips it on both sides.
    pub matte: LayerInputDraw,
    /// The Matte's Channel and Invert (K-425), applied by the combine itself:
    /// nothing prepares this matte at the dispatch seam, because there is no
    /// dispatch.
    pub matte_channel: u32,
    /// See [`matte_channel`](Self::matte_channel).
    pub matte_invert: bool,
    /// Where the frame's own time falls across the open shutter, 0..1
    /// (`AccumulationMbParams::shutter_anchor`): the point a darker matte
    /// shrinks the shutter toward, so black is the unblurred frame.
    pub anchor: f32,
}

pub struct CompLayerDraw {
    /// Which layer of the composition this draw is, so a measured cost can be
    /// put on the right Timeline row (docs/13 §7.1). Nothing about the picture
    /// depends on it — the compositor never reads it — and it is carried
    /// rather than inferred because the draw list is flattened: a collapsed
    /// Precomp splices its children in beside their neighbours, so a draw's
    /// position in the list is not its layer's position in the comp.
    pub layer: uuid::Uuid,
    pub source: DrawSource,
    /// The layer's natural pixel size — transforms act in comp pixels even
    /// when the texture was decoded at a reduced preview resolution.
    pub natural_size: (f32, f32),
    pub position: (f32, f32),
    pub anchor: (f32, f32),
    pub scale: (f32, f32),
    pub rotation_deg: f32,
    pub opacity: f32,
    pub z: f32,
    pub rotation_x_deg: f32,
    pub rotation_y_deg: f32,
    pub three_d: bool,
    pub matte: Option<MatteDraw>,
    pub blend: lumit_gpu::Blend,
    /// Layer-space mask coverage (white RGBA, alpha = coverage) for
    /// GPU-sourced layers — Precomps, whose pixels never exist CPU-side.
    pub mask_cov: Option<(Vec<u8>, u32, u32)>,
    /// Parent placement for layers spliced out of a collapsed Precomp
    /// (docs/06 §1.4): multiplied in front of this draw's own placement so
    /// content is resampled once, never twice. From lumit_gpu::place_matrix.
    pub pre: Option<[[f32; 4]; 4]>,
    /// The layer's live effect stack, resolved to plain numbers at this
    /// frame (docs/08; radius already in texture pixels). Applied to the
    /// linear source texture after masks, before the transform.
    pub fx: lumit_core::fx::ResolvedStack,
    /// The effect *instance* id behind each op in `fx`, 1:1 and in order
    /// (`lumit_core::fx::resolve_stack_temporal_named`). Only the profiler
    /// reads it: a measured millisecond has to land on the row of the stack
    /// that spent it, and this is the list the resolve walk itself reported.
    pub fx_ids: Vec<uuid::Uuid>,
    /// Decoded neighbour source frames for a temporal effect (echo etc.),
    /// keyed by frame offset — same sRGB8 form and decoded size as a Pixels
    /// source. Empty unless the stack is temporal.
    pub neighbours: Vec<(i32, Vec<u8>, u32, u32)>,
    /// The layer's dense forward flow field `(u, v, conf, w, h)` for Fast
    /// motion blur (docs/08 §3.2), carried from its decode job — `w × h` matches
    /// the decoded source. `conf` is the per-pixel confidence in 0..1 (FX-19)
    /// that tapers the streak; Datamosh reads only `(u, v)`. None unless the
    /// stack wants one.
    #[allow(clippy::type_complexity)]
    pub flow_field: Option<(Vec<f32>, Vec<f32>, Vec<f32>, u32, u32)>,
    /// The ordered file paths of the layer's enabled built-in `lut` effects
    /// (docs/08 §3.11; None = unset). Because `resolve_stack` keeps the same
    /// filter and order and a `lut` effect always resolves to exactly one
    /// op, this list is 1:1 and in order with the stack's `lut`
    /// ops — the caller loads each path and passes the parallel `luts` to
    /// `run_ops`. No GPU work happens here; these are just the strings.
    pub lut_files: Vec<Option<String>>,
    /// The layer inputs of the layer's enabled built-in `light_wrap` effects
    /// (docs/08 §3.28, docs/impl/layer-input.md) — a *plate*, not a matte, which
    /// is why it is still its own list. Because `resolve_stack` keeps the same
    /// filter and order and the effect always resolves to exactly one op, this
    /// list is 1:1 and in order with the stack's layer-input-consuming ops — the
    /// caller renders each one alone at comp size and passes the parallel
    /// `layer_inputs` to `run_ops`. A [`LayerInputDraw::Layer`] carries the
    /// referenced layer's source pixels; the GPU render happens in
    /// `realise_segment`.
    pub dof_inputs: Vec<LayerInputDraw>,
    /// **Every op's Matte** (K-395, docs/08 §2.6): one slot per op whose effect
    /// declares a matte parameter — which is every op — 1:1 and in stack order
    /// with them. [`LayerInputDraw::Absent`] when the row is unset or the
    /// reference is dangling, which runs nothing at all and leaves the effect
    /// exactly as it was before K-395 (K-258).
    ///
    /// One list for all four meanings: the generic strength dissolve, Depth of
    /// field's depth pass, the Lens flare's source matte, and the blur and
    /// glow's own readings. What differs between them is which parameter the
    /// reference is stored under and who consumes the texture, and the schema's
    /// `MatteRole` answers both — so nothing here needs to know.
    pub mattes: Vec<LayerInputDraw>,
    /// **Every path op's mask** (K-408, docs/08 §1.2): one flattened polyline
    /// per op whose effect declares a [`ParamKind::MaskPath`](lumit_core::fx::
    /// ParamKind::MaskPath) row, 1:1 and in stack order with them — the same
    /// one-predicate, one-order rule [`Self::mattes`] follows, with its own
    /// counter because its predicate is a different one (most effects take a
    /// matte; almost none takes a path).
    ///
    /// Flattened here rather than on the GPU because it is geometry, not
    /// pixels: the vertices are the layer's, and the tolerance is a constant in
    /// px@comp, so the same document at the same frame produces the same
    /// polyline at any preview raster. An empty polyline is the effect's
    /// documented no-op — an unset row, a mask since deleted, a layer with no
    /// masks — and never a fault.
    pub mask_paths: Vec<lumit_core::mask::MaskPolyline>,
    /// The `lens_file` paths of the layer's enabled built-in `lens_flare`
    /// effects (K-264), 1:1 with the stack's `lens_flare` ops —
    /// None = unset. The caller reads and hashes each file and passes the
    /// parallel `flare_lens` texts to `run_ops`; a missing or unreadable
    /// file degrades to the picked library lens (labelled fallback).
    pub flare_lens_files: Vec<Option<String>>,
    /// The raster width this layer's `fx` were RESOLVED against, when it
    /// can differ from the raster they will RUN on (K-266) — set for
    /// Adjust layers (the comp width; their stack runs on the render
    /// target, which reduced-resolution preview shrinks), `None` when the
    /// resolve factor already matches (footage layers scale by their
    /// decode). The realise walk divides its target width by this and
    /// rescales every px-dimensioned resolved field
    /// (`ResolvedStack::rescale_spatial`) so px@comp parameters land where
    /// the user put them at every preview resolution.
    pub fx_ref_width: Option<f32>,
    /// Per-layer motion-blur sub-frame placements (docs/06 §4, K-120): the
    /// layer's own transform re-evaluated across the open shutter. Empty unless
    /// the comp master and the layer switch are both on (and samples ≥ 2), in
    /// which case the compositor draws the layer's SAME texture at each of
    /// these and averages them into one smeared layer; the single-placement
    /// fields above stay the frame-time (k=0-ish) representative placement.
    pub mb: Vec<lumit_gpu::MbSample>,
    /// The comp's Light layers that shade this one (docs/06, K-361), already
    /// reduced to comp-pixel rectangles. Empty unless the comp holds lights
    /// and this layer's Accepts lights switch is on — and empty means the
    /// lighting pass never runs, which is how a comp without lights renders
    /// byte-for-byte as it did before lighting existed.
    pub lights: Vec<lumit_gpu::fx::LightingLight>,
    /// The below-stack re-rendered at a held time for a temporal adjustment
    /// (Posterize Time, docs/08 §3.25). Some only on an adjustment
    /// [`DrawSource::Adjust`] draw whose stack holds a Posterize Time effect
    /// scoped to *everything below*: `realise` then composites this held
    /// version (with the adjustment's own remaining effects) in place of the
    /// plain below-composite, blended by the adjustment's coverage. None on
    /// every ordinary draw, so nothing changes when no temporal effect is live.
    pub temporal_below: Option<TemporalBelow>,
    /// The below-stack re-rendered at N sub-frame times for accumulation motion
    /// blur (docs/08 §3.26). Some only on an adjustment [`DrawSource::Adjust`]
    /// draw whose stack holds a live accumulation MB effect: `realise` averages
    /// the N finished composites and blends by `mix`, standing in for the plain
    /// below-composite. Takes precedence over `temporal_below` when both are set
    /// (one temporal re-render per adjustment in v1). None on every ordinary draw.
    pub accumulation_below: Option<AccumulationBelow>,
}
