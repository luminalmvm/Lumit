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

/// Look a schema up by its match name. `None` for a name this build does not
/// know — an unknown effect is preserved as an inert placeholder (K-065).
///
/// Since K-593 the catalogue also carries whatever registered at run time, so
/// this answers for a scanned OFX plugin as readily as for a built-in.
pub fn schema(match_name: &str) -> Option<&'static EffectSchema> {
    def(match_name).map(|d| d.schema())
}

/// The definition named `match_name`, whichever list it lives on — **the one
/// lookup** (K-706).
///
/// The catalogue is asked first, then the layer styles
/// ([`STYLE_DEFS`](super::STYLE_DEFS)). The two are separate lists on purpose:
/// the catalogue is what the Add-effect menu, the command palette and the preset
/// browser walk, and offering "Drop shadow (style)" there beside the Drop shadow
/// effect would be the wrong answer to every search for a shadow. But a style
/// resolves, backfills, greys a row and reads its parameters through exactly the
/// machinery an effect does, and every one of those places wants "the definition
/// answering to this name" — so they ask here rather than picking a list.
///
/// `None` for a name this build does not know, which stays an inert placeholder
/// rendering as identity (K-065, K-258).
pub fn def(match_name: &str) -> Option<&'static dyn super::EffectDef> {
    super::BUILTIN_DEFS
        .get(match_name)
        .or_else(|| super::STYLE_DEFS.get(match_name))
}

/// What every OFX plugin's `match_name` begins with (K-593) — the host mints
/// `ofx:<plugin identifier>`, and this is the one place the prefix is spelled,
/// so the crate that writes it and the crate that reads it cannot drift.
pub const OFX_MATCH_PREFIX: &str = "ofx:";

/// What every **CLAP audio plugin's** `match_name` begins with (K-700) — the
/// host mints `clap:<plugin id>`, and this is the one place the prefix is
/// spelled, so the crate that writes it and the crates that read it cannot
/// drift.
pub const CLAP_MATCH_PREFIX: &str = "clap:";

/// What every **VST3 audio plugin's** `match_name` begins with (K-707) — the
/// host mints `vst3:<class id>`, the class id spelled as its 32 hex digits.
///
/// Beside CLAP's rather than instead of it, and both land in the same
/// [`EffectNamespace::Clap`]: the namespace says "an audio plugin", and which
/// standard it speaks is a fact only the host needs, carried by the prefix
/// (docs/impl/audio-plugins.md §4).
pub const VST3_MATCH_PREFIX: &str = "vst3:";

/// Which namespace an entry in the catalogue belongs to, from its match name.
///
/// Nothing on [`EffectSchema`] says where an effect came from — a declaration
/// is the same declaration whoever wrote it, which is the point of docs/12 §1 —
/// so the name is what carries the provenance, and it carries it because the
/// host minted the name.
fn namespace_of(match_name: &str) -> EffectNamespace {
    if match_name.starts_with(OFX_MATCH_PREFIX) {
        EffectNamespace::Ofx
    } else if match_name.starts_with(CLAP_MATCH_PREFIX) || match_name.starts_with(VST3_MATCH_PREFIX)
    {
        EffectNamespace::Clap
    } else {
        EffectNamespace::Builtin
    }
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
    // Wave 2's Distort I batch (docs/08 §3.48, §3.51, §3.52) has the same shape
    // of default again: a Corner pin's four points and a Twirl's or a Spherize's
    // centre are all px@comp (K-260), and a schema constant that says "960, 540"
    // lands a fresh instance in the top-left quarter of a 4K comp — the "drop it
    // on and it already looks right" failure §1.2 names. The keystone below is
    // §3.48's declared default expressed as fractions of the actual raster.
    for p in &mut inst.params {
        let v = match (match_name, p.id.as_str()) {
            ("corner_pin", "upper_left_x") => w * 0.05,
            ("corner_pin", "upper_left_y" | "upper_right_y") => h * 0.05,
            ("corner_pin", "upper_right_x") => w * 0.95,
            ("corner_pin", "lower_left_x") => 0.0,
            ("corner_pin", "lower_left_y" | "lower_right_y") => h * 0.95,
            ("corner_pin", "lower_right_x") => w,
            // Planar track's quad (docs/08 §3.87, K-579) is the same default
            // for the same reason, and a rectangle in the middle third rather
            // than the corner pin's near-full-frame keystone: a fresh quad is
            // a thing to drag onto a surface, and one starting at the frame's
            // own edges is harder to grab than one starting inside it.
            ("planar_track", "upper_left_x" | "lower_left_x") => w * 0.34,
            ("planar_track", "upper_right_x" | "lower_right_x") => w * 0.66,
            ("planar_track", "upper_left_y" | "upper_right_y") => h * 0.34,
            ("planar_track", "lower_left_y" | "lower_right_y") => h * 0.66,
            // Wave 2's Transitions batch (docs/08 §3.71) joins them: an Iris
            // wipe's centre is px@comp for the same reason; its two radii
            // are lengths, not positions, so they need no centring.
            ("twirl" | "spherize" | "ripple" | "iris_wipe", "centre_x") => w * 0.5,
            ("twirl" | "spherize" | "ripple" | "iris_wipe", "centre_y") => h * 0.5,
            // The Point control (K-414) is the same default once more: a fresh
            // crosshair belongs in the middle of the frame, which is the only
            // guess that is never absurd, and a schema constant cannot know the
            // raster.
            ("point_control", "point_x") => w * 0.5,
            ("point_control", "point_y") => h * 0.5,
            // Points sample's query point (K-494) is the same default for the
            // same reason: "how far is the nearest particle from here" wants a
            // "here" somebody can see, and the schema cannot know the raster.
            ("points_sample", "position_x") => w * 0.5,
            ("points_sample", "position_y") => h * 0.5,
            // Radial blur's centre became px@comp with K-558, so it joins the
            // list for the reason every other member is on it: a stored 960,
            // 540 is the middle of a 1080p comp and the top-left quarter of a
            // 4K one, and a fresh radial blur must spin about the middle of
            // whatever frame it landed on.
            ("radial_blur", "centre_x") => w * 0.5,
            ("radial_blur", "centre_y") => h * 0.5,
            // Card wipe's Transition width is a distance across the frame
            // (K-558), and its declared half-a-frame default is only *half a
            // frame* on the comp it landed on. The default Flip order runs
            // left to right, so the width is the axis it is measured along.
            ("card_wipe", "transition_width") => w * 0.5,
            // Tile's centre is the same default for a stronger reason (K-542):
            // its whole-frame default tile is only the *identity* if it is cut
            // from the middle of the frame, so a schema constant of 960, 540 on
            // a 4K comp would make a fresh Tile shift the picture — the one
            // thing §1.2 says dropping an effect on must never do.
            ("tile", "tile_centre_x") => w * 0.5,
            ("tile", "tile_centre_y") => h * 0.5,
            // And with K-558 the tile's own size and the output window's join
            // it, for that same reason: they are sizes, so they are px@comp,
            // and the identity is *one whole frame* — which is 1920 × 1080 on
            // exactly one comp. The actual raster is what makes a fresh Tile
            // the identity on all the others.
            ("tile", "tile_width" | "output_width") => w,
            ("tile", "tile_height" | "output_height") => h,
            // Wave 2's Distort II batch (docs/08 §3.55): a Bezier warp's twelve
            // points are the frame's own corners with the handles at the
            // thirds — the patch that is exactly the identity — and every one
            // of them is px@comp, so all twelve need the actual raster.
            ("bezier_warp", "upper_left_x" | "lower_left_x") => 0.0,
            ("bezier_warp", "upper_left_y" | "upper_right_y") => 0.0,
            ("bezier_warp", "upper_right_x" | "lower_right_x") => w,
            ("bezier_warp", "lower_left_y" | "lower_right_y") => h,
            ("bezier_warp", "top_left_tangent_x" | "bottom_left_tangent_x") => w / 3.0,
            ("bezier_warp", "top_right_tangent_x" | "bottom_right_tangent_x") => w * 2.0 / 3.0,
            ("bezier_warp", "top_left_tangent_y" | "top_right_tangent_y") => 0.0,
            ("bezier_warp", "bottom_left_tangent_y" | "bottom_right_tangent_y") => h,
            ("bezier_warp", "left_top_tangent_x" | "left_bottom_tangent_x") => 0.0,
            ("bezier_warp", "right_top_tangent_x" | "right_bottom_tangent_x") => w,
            ("bezier_warp", "left_top_tangent_y" | "right_top_tangent_y") => h / 3.0,
            ("bezier_warp", "left_bottom_tangent_y" | "right_bottom_tangent_y") => h * 2.0 / 3.0,
            _ => continue,
        };
        p.value = EffectValue::Float(Property::fixed(v));
    }
    Some(inst)
}

/// The value a parameter kind starts at (docs/08 §1.2): what `instantiate`
/// fills a fresh instance with, and what [`backfill_builtin_params`] appends
/// for a parameter the saved instance predates.
///
/// `None` for a row that **has** no value — today only [`ParamKind::Action`],
/// the button kind (K-417). A stored value for a button would be a button that
/// saves, animates and fires again on load, which is exactly what the kind
/// exists to avoid; so no `EffectParam` is written for one, and every walk that
/// fills defaults skips it.
pub fn default_param_value(kind: &ParamKind) -> Option<EffectValue> {
    Some(match *kind {
        // A Slider stores as a Float like Int and Angle do (K-414) — the kind
        // is the control drawn, not the value kept.
        ParamKind::Float { default, .. } | ParamKind::Slider { default, .. } => {
            EffectValue::Float(Property::fixed(default))
        }
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
        // A fresh mask-path row is unset, which is the "First mask" entry
        // (K-408) — resolved at render time, not written here: an effect is
        // usually added before the mask is drawn, so there is no id to write,
        // and a row that followed the first mask only until the masks were
        // reordered would be the worse of the two behaviours anyway.
        ParamKind::MaskPath { .. } => EffectValue::MaskPath(None),
        // A fresh curve is the shape its declaration asked for (K-412) — the
        // identity diagonal for the grade family, the grade family's
        // sanctioned exception to the "no no-op default" rule, and its own
        // shape for an over-life curve, which is a *look* rather than a no-op
        // (particulate.md §2).
        ParamKind::Curve { default } => EffectValue::Curve(default.to_vec()),
        // A button holds nothing (K-417) — see the doc comment above.
        ParamKind::Action => return None,
    })
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
            let Some(value) = default_param_value(&p.kind) else {
                continue; // a button has nothing to backfill (K-417)
            };
            if !e.params.iter().any(|have| have.id == p.id) {
                e.params.push(EffectParam {
                    id: p.id.to_owned(),
                    value,
                    extra: serde_json::Map::new(),
                });
            }
        }
    }
}

/// Scale every value a property holds by `k`, curve shape included.
///
/// A span's four control points are `(v, v + speed·influence·Δt)` at each end
/// (see [`crate::anim::CubicSpan::from_ae`]), so a value axis multiplied by `k`
/// is exactly the same curve scaled — provided the speeds go with it. Times and
/// influences are untouched: they live on the other axis.
fn scale_property(p: &mut Property, k: f64) {
    use crate::anim::{Animation, SideInterp};
    let scale_side = |s: &mut SideInterp| {
        if let SideInterp::Bezier { speed, .. } = s {
            *speed *= k;
        }
    };
    match &mut p.animation {
        Animation::Static(v) => *v *= k,
        Animation::Keyframed(keys) => {
            for key in keys {
                key.value *= k;
                scale_side(&mut key.interp_in);
                scale_side(&mut key.interp_out);
            }
        }
        // An expression computes its own number; rewriting someone's script is
        // worse than leaving it, and the row is now read as pixels either way.
        Animation::Expression(_) => {}
    }
}

/// Convert a saved instance's share-of-the-frame values into px@comp (K-558),
/// the K-258-style compatibility read for the conversions K-558 ordered.
///
/// **Why this is not part of [`backfill_builtin_params`].** A per cent of the
/// frame only becomes a pixel count once the frame is known, and the backfill is
/// handed a bare stack. The project reader calls this from inside the
/// composition, where `w`/`h` are at hand; the effect's declared `version` is
/// the gate, so a file read twice converts once and a file saved since the
/// conversion is left alone.
pub fn migrate_percent_to_px(effects: &mut [EffectInstance], w: f64, h: f64) {
    for e in effects.iter_mut() {
        if e.effect.namespace != EffectNamespace::Builtin {
            continue;
        }
        // Radial blur v1 → v2: Centre x and Centre y were a per cent of the
        // frame's width and height (K-558).
        if e.effect.match_name == "radial_blur" && e.effect.version < 2 {
            for p in &mut e.params {
                let basis = match p.id.as_str() {
                    "centre_x" => w,
                    "centre_y" => h,
                    _ => continue,
                };
                if let EffectValue::Float(prop) = &mut p.value {
                    scale_property(prop, basis / 100.0);
                }
            }
            e.effect.version = 2;
        }
        // Beam v1 → v2: Length was a per cent of the *run* between Start and
        // End (K-558). The run is the only basis that was ever right for it, so
        // the conversion reads the instance's own four points — at time zero,
        // because a keyframed pair means the old percentage described a
        // distance that moved, and no single pixel number can be all of them.
        // A still beam, which is nearly every beam, converts exactly.
        if e.effect.match_name == "beam" && e.effect.version < 2 {
            let at_zero = |id: &str| match e.param(id) {
                Some(EffectValue::Float(p)) => p.value_at(0.0),
                _ => 0.0,
            };
            let (dx, dy) = (
                at_zero("end_x") - at_zero("start_x"),
                at_zero("end_y") - at_zero("start_y"),
            );
            let run = dx.hypot(dy);
            for p in &mut e.params {
                if p.id != "length" {
                    continue;
                }
                if let EffectValue::Float(prop) = &mut p.value {
                    scale_property(prop, run / 100.0);
                }
            }
            e.effect.version = 2;
        }
        // Card wipe v1 → v2: Transition width was a per cent of the frame
        // measured along whichever axis Flip order runs (K-558), so the basis
        // is the width for the two horizontal orders and the height for the two
        // vertical ones — read off the instance's own choice.
        if e.effect.match_name == "card_wipe" && e.effect.version < 2 {
            let vertical = matches!(e.param("flip_order"), Some(EffectValue::Choice(2 | 3)));
            let basis = if vertical { h } else { w };
            for p in &mut e.params {
                if p.id != "transition_width" {
                    continue;
                }
                if let EffectValue::Float(prop) = &mut p.value {
                    scale_property(prop, basis / 100.0);
                }
            }
            e.effect.version = 2;
        }
        // Tile v1 → v2: the tile's size and the output window's size were per
        // cents of the frame (K-558). Both pairs are sizes, so both convert,
        // each axis against its own extent — and the centre, which was px@comp
        // already, is left alone.
        if e.effect.match_name == "tile" && e.effect.version < 2 {
            for p in &mut e.params {
                let basis = match p.id.as_str() {
                    "tile_width" | "output_width" => w,
                    "tile_height" | "output_height" => h,
                    _ => continue,
                };
                if let EffectValue::Float(prop) = &mut p.value {
                    scale_property(prop, basis / 100.0);
                }
            }
            e.effect.version = 2;
        }
        // Lens flare v11 → v12: Ghost softness was a per cent of the frame's
        // *diagonal* — K-419's one surviving exception, closed by K-558 — so
        // the diagonal is the basis it converts against.
        if e.effect.match_name == "lens_flare" && e.effect.version < 12 {
            let diag = w.hypot(h);
            for p in &mut e.params {
                if p.id != "ghost_softness" {
                    continue;
                }
                if let EffectValue::Float(prop) = &mut p.value {
                    scale_property(prop, diag / 100.0);
                }
            }
            e.effect.version = 12;
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
/// Whether a parameter's row is **on screen at all** for this instance — the
/// visibility half of [`param_enabled`]'s question (K-145, K-257).
///
/// A [`ParamGroup`] may carry `visible_when`, and the rows in it then appear
/// only while a sibling Choice holds one of the listed values: the Lens flare's
/// Matte rows exist only while its Source type is Matte. A parameter in no
/// group, or in a group with no condition, is always visible — nearly all of
/// them.
///
/// **The render reads this, not only the panel.** A matte row nobody can see is
/// a matte nobody can have meant, and rendering the layer it happens to name
/// would cost a whole extra pass per frame for a picture that is thrown away
/// (K-395: `build.rs`'s `mattes_for` skips such a row). That is what the Lens
/// flare's own "only in Matte mode" gate used to say by name, before the matte
/// carriage made it one rule for every effect.
///
/// The lens-element condition (`visible_when_lens_elements`) is deliberately not
/// consulted: it depends on which prescription is loaded, which is a file the
/// engine has not necessarily read here, and no matte row uses it.
#[must_use]
pub fn param_visible(inst: &EffectInstance, id: &str) -> bool {
    let Some(s) = schema(&inst.effect.match_name) else {
        return true;
    };
    s.groups.iter().filter(|g| g.params.contains(&id)).all(|g| {
        let Some((on, values)) = g.visible_when else {
            return true;
        };
        // A group whose deciding parameter the instance does not carry stays
        // visible — the `param_enabled` rule, for the same reason.
        match inst.param(on) {
            Some(EffectValue::Choice(got)) => values.contains(got),
            _ => true,
        }
    })
}

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
            namespace: namespace_of(s.match_name),
            match_name: s.match_name.to_owned(),
            version: s.version,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: s
            .params
            .iter()
            // A button declares no value, so a fresh instance carries no row
            // for it (K-417).
            .filter_map(|p| {
                Some(EffectParam {
                    id: p.id.to_owned(),
                    value: default_param_value(&p.kind)?,
                    extra: serde_json::Map::new(),
                })
            })
            .collect(),
        sample_temporally: true,
        custom_name: None,
        // Unlinked (K-443): a fresh point's two halves move on their own, which
        // is what every effect did before there was a chain to close.
        linked_pairs: Vec::new(),
        // A fresh audio plugin has no memory of itself yet; it is the plugin's
        // own defaults until something saves one (K-700).
        plugin_state: None,
        // A fresh Roto brush has no strokes and no base frame, which is what
        // makes Propagate refuse with `NoBaseFrame` rather than guess (K-710).
        roto: None,
        extra: serde_json::Map::new(),
    })
}
