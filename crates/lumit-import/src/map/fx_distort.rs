//! The Distortion, Stylise, Transition, Utility and Controls half of the After
//! Effects effect table
//! ([docs/11-AE-IMPORT.md](../../../../docs/11-AE-IMPORT.md) §5).
//!
//! # In plain terms
//!
//! After Effects hands over an effect as a match name — `ADBE Twirl` — and a
//! bag of numbered parameters: `ADBE Twirl-0001` is the Angle, `-0002` the
//! Radius. This module is the half of the phrasebook that translates the warps,
//! the wipes and the stylisers. Each entry says which Lumit effect does the same
//! job, which of its controls each After Effects control becomes, and — the part
//! that actually matters — **what has to happen to the number on the way**.
//!
//! Three kinds of thing happen to a number here:
//!
//! - **Nothing.** An angle is an angle and a per cent is a per cent, so it
//!   carries across untouched.
//! - **A change of base.** After Effects measures Twirl's radius as a per cent
//!   of the layer; Lumit measures every distance in px@comp (docs/08 §2.3,
//!   K-419), and scales it to the preview raster itself so that a
//!   half-resolution preview looks like the export. Same length, different
//!   spelling — the import multiplies through, and the report says it did.
//! - **A split, a collapse or a refusal.** After Effects' Spherize has one
//!   signed radius where Lumit has a size and a direction; its Warp has fifteen
//!   styles where Lumit has thirteen; its Card Wipe has a whole camera rig that
//!   Lumit deliberately does not (docs/06 keeps cameras on the composition). A
//!   split is exact, a collapse says so in the report, and a refusal is reported
//!   rather than approximated — **never guessed at**.
//!
//! Whatever happens, the keyframes come too: a conversion is applied to every
//! key's value and to the speed of every bezier handle, so an animated radius
//! arrives animating the same way.

use lumit_core::anim::{Animation, Keyframe, Property as LumProperty, SideInterp};
use lumit_core::model::{EffectInstance, EffectValue};

use crate::capture::Property;
use crate::report::{ItemPath, Outcome, Reason};

use super::props::{self, ae_map, axis_of, display_name, from_node, match_name_of};
use super::{srgb_to_linear, Conv};

/// One conversion: how a Lumit effect's controls are filled from an After
/// Effects instance.
type Build = for<'a, 'b, 'c> fn(&'a mut Fx<'b, 'c>);

/// This half's conversions, keyed by the name `ae-effect-map.toml` calls them.
///
/// Which After Effects match name reaches which of these is the shipped table's
/// business ([`super::table`], docs/11 §5) rather than this file's — the names
/// are Adobe's and we get them wrong, so they live in a file that can be
/// corrected without a rebuild. What stays here is the arithmetic: a change of
/// base, an option list that maps by position, a control After Effects splits
/// where Lumit joins. A row naming a conversion this build does not have simply
/// goes unclaimed, so an edited table can never ask for code that is not here.
fn conversion(key: &str) -> Option<Build> {
    Some(match key {
        // --- Utility ---
        "transform" => transform as Build,
        "set_matte" => set_matte,
        "set_channels" => set_channels,
        // --- Distortion ---
        "motion_tile" => motion_tile,
        "offset" => offset,
        "mirror" => mirror,
        "optics_compensation" => optics_compensation,
        "turbulent_displace" => turbulent_displace,
        "corner_pin" => corner_pin,
        "displacement_map" => displacement_map,
        "polar_coordinates" => polar_coordinates,
        "twirl" => twirl,
        "spherize" => spherize,
        "ripple" => ripple,
        "wave_warp" => wave_warp,
        "bezier_warp" => bezier_warp,
        "warp" => warp,
        // --- Stylise ---
        "drop_shadow" => drop_shadow,
        "roughen_edges" => roughen_edges,
        "median" => median,
        "mosaic" => mosaic,
        "find_edges" => find_edges,
        "emboss" => emboss,
        "texturize" => texturize,
        // --- Blur (the one that sits in this half because it is a wipe's
        // neighbour in docs/11's ordering, not because it blurs) ---
        "channel_blur" => channel_blur,
        // --- Transition ---
        "linear_wipe" => linear_wipe,
        "radial_wipe" => radial_wipe,
        "iris_wipe" => iris_wipe,
        "venetian_blinds" => venetian_blinds,
        "card_wipe" => card_wipe,
        // --- Controls (K-414) ---
        "slider_control" => slider_control,
        "angle_control" => angle_control,
        "checkbox_control" => checkbox_control,
        "colour_control" => colour_control,
        "point_control" => point_control,
        _ => return None,
    })
}

/// The table's claim on one captured effect instance.
///
/// `None` for a match name this half does not know, which is what sends the
/// instance to the other half and then to the placeholder road (docs/11 §5's
/// "never the closest guess").
pub(crate) fn claim(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
) -> Option<EffectInstance> {
    let ae = match_name_of(node);
    let row = super::table::table().row(ae)?;
    let build = conversion(&row.conversion)?;
    let mut fx = Fx::new(conv, path, node, ae, &row.lumit)?;
    build(&mut fx);
    fx.conv.report.imported();
    Some(fx.inst)
}

// ───────────────────────────── the rows ─────────────────────────────

/// AE `ADBE Geometry2` → Transform (docs/11 §5).
fn transform(fx: &mut Fx<'_, '_>) {
    fx.point(1, "anchor_x", "anchor_y", Unit::Px);
    fx.point(2, "position_x", "position_y", Unit::Px);
    // AE's Uniform Scale ties Width to Height and hides Width; Lumit has two
    // axes and no switch, so the tie is *resolved* here rather than carried.
    // Nothing is lost, so nothing is reported.
    let uniform = fx.still(11).unwrap_or(1.0) != 0.0;
    fx.carry(3, "scale_y", Unit::Direct);
    fx.carry(if uniform { 3 } else { 4 }, "scale_x", Unit::Direct);
    fx.carry(7, "rotation", Unit::Direct);
    fx.carry(8, "opacity", Unit::Direct);
    fx.drop_ae(5); // Skew
    fx.drop_ae(6); // Skew Axis
    fx.drop_ae(9); // Use Composition's Shutter Angle
    fx.drop_ae(10); // Shutter Angle
    fx.drop_ae(12); // Sampling
}

/// AE `ADBE Set Matte3` → Set matte (docs/11 §5): the layer row is the
/// universal Matte (docs/08 §2.6), Use For Matte is Channel, Invert Matte is
/// the Matte row's own Invert.
fn set_matte(fx: &mut Fx<'_, '_>) {
    fx.layer(1, "matte");
    fx.channel(2, "channel", None);
    fx.toggle(3, "matte_invert");
    fx.drop_ae(4); // If Layer Sizes Differ — the Matte row is always "stretch to fit"
    fx.toggle(5, "combine");
    fx.drop_ae(6); // Premultiply Matte Layer — Lumit composites premultiplied throughout
}

/// AE `ADBE Set Channels` → Set channels (docs/11 §5, docs/08 §3.94).
///
/// **Four source layers become one.** After Effects gives every output channel
/// its own layer picker; Lumit's carriage carries one auxiliary layer per
/// effect, and every output channel names either that layer or the one the
/// effect is on. So the conversion picks the **first source layer that is
/// neither None nor this layer** as the Source row, and then each output
/// channel maps exactly when it names that layer, this layer, or nothing at
/// all. A second, different source layer is a picture Lumit cannot fetch here,
/// so that channel is **reported and left at its identity default** rather than
/// pointed at the wrong picture (§5's "never the closest guess"); the answer is
/// a second copy of the effect.
///
/// The pairing is `-0001`/`-0002` for red and so on up to `-0007`/`-0008` for
/// alpha. AE's channel list is 1-based — Red, Green, Blue, Alpha, Luminance,
/// Hue, Lightness, Saturation, Full On, Full Off — and the three it has that
/// Lumit does not collapse onto Luminance and say so, exactly as [`Fx::channel`]
/// collapses them.
fn set_channels(fx: &mut Fx<'_, '_>) {
    const OUTPUTS: [(u32, u32, &str, u32); 4] = [
        (1, 2, "red_from", 1),
        (3, 4, "green_from", 2),
        (5, 6, "blue_from", 3),
        (7, 8, "alpha_from", 4),
    ];
    let this = fx.conv.self_index;
    // **An absent picker is not None.** The file stores only what is not at its
    // default, and After Effects' default here is the layer the effect is on —
    // so a slot with no leaf at all means this layer, while a leaf holding zero
    // means the user chose None.
    let sources: Vec<u32> = OUTPUTS
        .iter()
        .map(|(n, ..)| match fx.find(*n) {
            None => this,
            Some(_) => fx.still(*n).map_or(0, |v| v.round().max(0.0) as u32),
        })
        .collect();
    let chosen = sources
        .iter()
        .copied()
        .find(|index| *index != 0 && *index != this);
    if let Some(index) = chosen {
        match fx.conv.layer_ids.get(&index).copied() {
            Some(layer) => fx.set("source", EffectValue::Layer(Some(layer))),
            None => fx.row(Outcome::Adjusted, Reason::MatteTargetMissing { index }),
        }
    }

    for (i, (layer_n, channel_n, id, default_channel)) in OUTPUTS.iter().enumerate() {
        // The file only stores what is not at its default, and the default is
        // the identity assignment — red from red, and so on.
        let picked = match fx.find(*channel_n) {
            None => i64::from(*default_channel),
            Some(_) => fx
                .still(*channel_n)
                .map_or(i64::from(*default_channel), |v| v.round() as i64),
        };
        // AE 1..5 are Red, Green, Blue, Alpha, Luminance, which are Lumit's
        // 0..4 in the same order. 6, 7 and 8 are Hue, Lightness and Saturation,
        // which Lumit does not have; 9 and 10 are Full On and Full Off.
        let base = match picked {
            1..=5 => (picked - 1) as u32,
            9 => {
                fx.set(id, EffectValue::Choice(10));
                continue;
            }
            10 => {
                fx.set(id, EffectValue::Choice(11));
                continue;
            }
            _ => {
                let ae_name = fx.ae_name(*channel_n);
                fx.approximated(&ae_name, "Luminance");
                4
            }
        };
        let source = sources[i];
        if source == this {
            fx.set(id, EffectValue::Choice(base));
        } else if Some(source) == chosen {
            fx.set(id, EffectValue::Choice(base + 5));
        } else {
            // Zero is After Effects' "None" and anything else is a *second*
            // source layer; both are a picture this conversion will not guess
            // at, so the report names the **picker** rather than the channel
            // beside it — that is the control that could not be carried — and
            // the channel keeps its identity default.
            let ae_name = fx.ae_name(*layer_n);
            fx.approximated(&ae_name, "this layer's own channel");
        }
    }
}

/// AE `ADBE Tile` → Tile (docs/11 §5): every control carries, and the four
/// sizes convert — AE keeps them as per cents of the frame, Lumit as px@comp
/// (K-558), so each axis is scaled by the comp's own extent.
fn motion_tile(fx: &mut Fx<'_, '_>) {
    let (comp_w, comp_h) = fx.conv.size;
    fx.point(1, "tile_centre_x", "tile_centre_y", Unit::Px);
    fx.carry(2, "tile_width", Unit::Scale(comp_w / 100.0));
    fx.carry(3, "tile_height", Unit::Scale(comp_h / 100.0));
    fx.carry(4, "output_width", Unit::Scale(comp_w / 100.0));
    fx.carry(5, "output_height", Unit::Scale(comp_h / 100.0));
    fx.toggle(6, "mirror_edges");
    fx.carry(7, "phase", Unit::Direct);
    fx.toggle(8, "horizontal_phase_shift");
}

/// AE `ADBE Offset` → Offset (docs/11 §5): AE's "Shift Center To" is a
/// destination and Lumit stores the shift, so the frame centre comes off.
fn offset(fx: &mut Fx<'_, '_>) {
    let (w, h) = fx.conv.size;
    fx.carry_axis(1, 0, "shift_x", Unit::Shift(-w / 2.0));
    fx.carry_axis(1, 1, "shift_y", Unit::Shift(-h / 2.0));
    fx.carry(2, "mix", Unit::Complement);
}

/// AE `ADBE Mirror` → Mirror (docs/11 §5): one for one.
fn mirror(fx: &mut Fx<'_, '_>) {
    fx.point(1, "centre_x", "centre_y", Unit::Px);
    fx.carry(2, "angle", Unit::Direct);
}

/// AE `ADBE Optics Compensation` → Lens distort (docs/11 §5).
fn optics_compensation(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "fov", Unit::Direct);
    fx.toggle(2, "reverse");
    fx.choice(3, "orientation", &[0, 1, 2], "Horizontal");
    fx.point(4, "centre_x", "centre_y", Unit::Px);
    fx.drop_ae(5); // Optimal Pixels
    fx.drop_ae(6); // Resize — effects render at the frame's raster (docs/08 §2.3)
}

/// AE `ADBE Turbulent Displace` → Turbulent displace (docs/11 §5).
fn turbulent_displace(fx: &mut Fx<'_, '_>) {
    // Turbulent, Vertical and Horizontal Displacement are the three Lumit
    // ships; Bulge, Twist, the three Smoother variants and Cross Displacement
    // are reported.
    fx.choice(
        1,
        "displacement",
        &[0, -1, -1, -1, -1, -1, 2, 1, -1],
        "Turbulent",
    );
    fx.carry(2, "amount", Unit::Px);
    fx.carry(3, "size", Unit::Px);
    fx.point(4, "offset_x", "offset_y", Unit::Px);
    fx.carry(5, "complexity", Unit::Direct);
    fx.carry(6, "evolution", Unit::Direct);
    fx.toggle(8, "cycle_evolution");
    fx.carry(9, "cycle", Unit::Direct);
    fx.seed(10, "seed");
    // Ten AE combinations, three Lumit ones (docs/08 §3.38 decision 4), and the
    // audit enumerates no option *strings* — only the default index, which is
    // AE's "pin every edge". That one pairing is pinned; every other index is
    // reported rather than mapped to a guess.
    fx.choice(12, "pinning", &[-1, -1, 1], "All edges");
    fx.drop_ae(13); // Resize Layer
    fx.drop_ae(14); // Antialiasing for Best Quality — Lumit resamples bilinearly everywhere
}

/// AE `ADBE Corner Pin` → Corner pin (docs/11 §5): the four points, one for one.
fn corner_pin(fx: &mut Fx<'_, '_>) {
    fx.point(1, "upper_left_x", "upper_left_y", Unit::Px);
    fx.point(2, "upper_right_x", "upper_right_y", Unit::Px);
    fx.point(3, "lower_left_x", "lower_left_y", Unit::Px);
    fx.point(4, "lower_right_x", "lower_right_y", Unit::Px);
}

/// AE `ADBE Displacement Map` → Displacement map (docs/11 §5): the map layer is
/// the universal Matte row, which renders it at this raster — "stretch to fit",
/// which is why AE's three Behaviours are reported rather than approximated.
fn displacement_map(fx: &mut Fx<'_, '_>) {
    fx.layer(1, "matte");
    // The amounts first: AE's "Off" channel means "do not displace on this
    // axis", and `channel` writes the zero that says so (docs/11 §5).
    fx.carry(3, "horizontal_amount", Unit::Px);
    fx.channel(2, "horizontal_channel", Some("horizontal_amount"));
    fx.carry(5, "vertical_amount", Unit::Px);
    fx.channel(4, "vertical_channel", Some("vertical_amount"));
    fx.drop_ae(6); // Displacement Map Behavior
    fx.drop_ae(7); // Edge Behavior — Lumit's Edges default is Repeat, AE's wrap
    fx.drop_ae(8); // Expand Output
                   // AE's "Off" channel means "do not displace on this axis", which is an
                   // Amount of 0 (docs/11 §5). `channel` writes it.
}

/// AE `ADBE Polar Coordinates` → Polar coordinates (docs/11 §5): both
/// conversions one for one, and AE's centre is the layer's as Lumit's is the
/// frame's, so there is nothing to convert.
fn polar_coordinates(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "interpolation", Unit::Direct);
    fx.choice(2, "conversion", &[0, 1], "Rectangular to polar");
}

/// AE `ADBE Twirl` → Twirl (docs/11 §5): the radius changes base.
fn twirl(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "angle", Unit::Direct);
    fx.carry(2, "radius", Unit::LayerPct);
    fx.point(3, "centre_x", "centre_y", Unit::Px);
}

/// AE `ADBE Spherize` → Spherize (docs/11 §5): **AE's one signed Radius becomes
/// two controls** — the magnitude is the size and the sign is the direction
/// (docs/08 §3.52's fourth note). Nothing is lost; a negative AE radius imports
/// as a pinch of the same size.
fn spherize(fx: &mut Fx<'_, '_>) {
    let sign = fx.sign(1);
    fx.carry(1, "radius", Unit::Scale(sign));
    fx.set("bulge", float(if sign < 0.0 { -100.0 } else { 100.0 }));
    fx.point(2, "centre_x", "centre_y", Unit::Px);
}

/// AE `ADBE Ripple` → Ripple (docs/11 §5). Wave Speed reads the clock, which
/// docs/08 §2.4 forbids, so it becomes Evolution keyframes.
fn ripple(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "radius", Unit::LayerPct);
    fx.point(2, "centre_x", "centre_y", Unit::Px);
    // AE's Type of Conversion lists Asymmetric first — its default, and Lumit's.
    fx.choice(3, "wave_type", &[1, 0], "Asymmetric");
    fx.carry(5, "wave_width", Unit::Px);
    fx.carry(6, "wave_height", Unit::Px);
    fx.clock(4, 7, "evolution");
}

/// AE `ADBE Wave Warp` → Wave warp (docs/11 §5). Wave Speed becomes Phase
/// keyframes, for §3.53's reason.
fn wave_warp(fx: &mut Fx<'_, '_>) {
    fx.choice(1, "wave_type", &[0, 1, 2, 3, 4, -1, -1, -1, -1], "Sine");
    fx.carry(2, "wave_height", Unit::Px);
    fx.carry(3, "wave_width", Unit::Px);
    fx.carry(4, "direction", Unit::Direct);
    fx.choice(6, "pinning", &[0, 1, 2, 3, 4, 5, 6, 7], "None");
    fx.clock(5, 7, "phase");
    fx.drop_ae(8); // Antialiasing (Best Quality)
                   // docs/11 §5 also names a Warp Axis here; the 2026-08-20 audit shows
                   // `ADBE Wave Warp` has no such property (that is `ADBE WRPMESH`'s), and the
                   // audit is the ground truth the same paragraph cites. The row is corrected.
}

/// AE `ADBE BEZMESH` → Bezier warp (docs/11 §5): twelve points in AE's own
/// clockwise walk from the upper left.
fn bezier_warp(fx: &mut Fx<'_, '_>) {
    fx.point(1, "upper_left_x", "upper_left_y", Unit::Px);
    fx.point(2, "top_left_tangent_x", "top_left_tangent_y", Unit::Px);
    fx.point(3, "top_right_tangent_x", "top_right_tangent_y", Unit::Px);
    fx.point(4, "upper_right_x", "upper_right_y", Unit::Px);
    fx.point(5, "right_top_tangent_x", "right_top_tangent_y", Unit::Px);
    fx.point(
        6,
        "right_bottom_tangent_x",
        "right_bottom_tangent_y",
        Unit::Px,
    );
    fx.point(7, "lower_right_x", "lower_right_y", Unit::Px);
    fx.point(
        8,
        "bottom_right_tangent_x",
        "bottom_right_tangent_y",
        Unit::Px,
    );
    fx.point(
        9,
        "bottom_left_tangent_x",
        "bottom_left_tangent_y",
        Unit::Px,
    );
    fx.point(10, "lower_left_x", "lower_left_y", Unit::Px);
    fx.point(
        11,
        "left_bottom_tangent_x",
        "left_bottom_tangent_y",
        Unit::Px,
    );
    fx.point(12, "left_top_tangent_x", "left_top_tangent_y", Unit::Px);
    fx.carry(14, "quality", Unit::Direct);
    fx.differs(
        "Quality buys Newton steps where After Effects' bought smaller triangles — the number \
         carries across and means \"more accurate\" on both",
    );
}

/// AE `ADBE WRPMESH` → Warp (docs/11 §5): thirteen of AE's fifteen styles.
fn warp(fx: &mut Fx<'_, '_>) {
    // AE ships Photoshop's list, whose Arc Lower precedes Arc Upper and whose
    // Shell Lower and Shell Upper have no Lumit counterpart.
    fx.choice(
        1,
        "style",
        &[0, 2, 1, 3, 4, -1, -1, 5, 6, 7, 8, 9, 10, 11, 12],
        "Arc",
    );
    fx.drop_ae(2); // Warp Axis
    fx.carry(3, "bend", Unit::Direct);
    fx.carry(4, "horizontal_distortion", Unit::Direct);
    fx.carry(5, "vertical_distortion", Unit::Direct);
    fx.differs(
        "the exact curve of each style is Lumit's own, After Effects' being Photoshop's \
         undocumented mesh — a look-for-look conversion",
    );
}

/// AE `ADBE Drop Shadow` → Drop shadow (docs/11 §5). Direction carries AE's
/// convention unchanged; the opacity does not, AE storing it 0..255.
fn drop_shadow(fx: &mut Fx<'_, '_>) {
    fx.colour(1, "shadow_colour");
    fx.carry(2, "opacity", Unit::Scale(100.0 / 255.0));
    fx.carry(3, "direction", Unit::Direct);
    fx.carry(4, "distance", Unit::Px);
    fx.carry(5, "softness", Unit::Px);
    fx.toggle(6, "shadow_only");
}

/// AE `ADBE Roughen Edges` → Roughen edges (docs/11 §5): **AE's seven edge
/// types become three plus a switch** (docs/08 §3.57 decision 2).
fn roughen_edges(fx: &mut Fx<'_, '_>) {
    let (shape, coloured, approximated) = match fx.still(1).unwrap_or(1.0).round() as i64 {
        1 => (0, false, false),   // Roughen
        2 => (0, true, false),    // Roughen Color
        3 => (1, false, false),   // Cut
        4 => (1, true, false),    // Cut Color
        5 => (2, false, false),   // Spiky
        6 => (2, true, false),    // Spiky Color
        7 | 8 => (1, true, true), // Photocopy, Photocopy Color
        _ => (0, false, true),
    };
    fx.set("edge_type", EffectValue::Choice(shape));
    fx.set("colour_edge", EffectValue::Bool(coloured));
    if approximated {
        fx.approximated("Edge Type", "Cut with the colour edge on");
    }
    fx.colour(10, "edge_colour");
    fx.carry(2, "border", Unit::Px);
    fx.carry(3, "edge_sharpness", Unit::Factor);
    fx.carry(4, "fractal_influence", Unit::Factor);
    fx.carry(5, "scale", Unit::Px);
    fx.drop_ae(6); // Stretch Width or Height
    fx.point(7, "offset_x", "offset_y", Unit::Px);
    fx.carry(8, "complexity", Unit::Direct);
    fx.carry(9, "evolution", Unit::Direct);
    fx.toggle(12, "cycle_evolution");
    fx.carry(13, "cycle", Unit::Direct);
    fx.seed(14, "seed");
}

/// AE `ADBE Median` → Median (docs/11 §5): the one conversion in the table
/// limited by a *budget* — Lumit's radius caps at 3 where AE's runs to 50
/// (docs/08 §3.64 decision 2, the cost being the fourth power of it).
fn median(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "radius", Unit::Px);
    if fx.clamp("radius", 3.0) {
        fx.approximated("Radius", "the largest radius Lumit runs, 3");
    }
    fx.toggle(2, "alpha");
}

/// AE `ADBE Mosaic` → Mosaic (docs/11 §5): direct.
fn mosaic(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "horizontal_blocks", Unit::Direct);
    fx.carry(2, "vertical_blocks", Unit::Direct);
    fx.toggle(3, "sharp_colours");
    fx.differs(
        "a block's colour is sampled on an at-most-8×8 grid rather than read pixel by pixel — the \
         same flat colour on any block worth mosaicking",
    );
}

/// AE `ADBE Find Edges` → Find edges (docs/11 §5).
fn find_edges(fx: &mut Fx<'_, '_>) {
    fx.toggle(1, "invert");
    fx.carry(2, "mix", Unit::Complement);
    fx.differs(
        "the gradient is taken on a perceptual position rather than on the light, so the lines \
         land where a person would draw them",
    );
}

/// AE `ADBE Emboss` → Emboss (docs/11 §5).
fn emboss(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "direction", Unit::Direct);
    fx.carry(2, "relief", Unit::Px);
    fx.carry(3, "contrast", Unit::Direct);
    fx.carry(4, "mix", Unit::Complement);
    fx.differs("the difference is taken on a perceptual position rather than on the light");
}

/// AE `ADBE Texturize` → Texturize (docs/11 §5): the texture layer is Lumit's
/// own Texture row, not the universal Matte (docs/08 §3.68 decision 1).
fn texturize(fx: &mut Fx<'_, '_>) {
    fx.layer(1, "texture");
    fx.carry(2, "light_direction", Unit::Direct);
    fx.carry(3, "texture_contrast", Unit::Factor);
    // AE's list is Tile, Center, Stretch Texture To Fit; Lumit's is Stretch,
    // Tile, Centre.
    let placement = fx.raw(4).unwrap_or(1.0).round() as i64;
    fx.choice(4, "placement", &[1, 2, 0], "Stretch");
    if placement == 1 || placement == 2 {
        // Both are the texture layer's *native* size, which the layer carriage
        // has not preserved (docs/impl/layer-input.md).
        fx.approximated(
            "Texture Placement",
            "the texture at this composition's size",
        );
    }
}

/// AE `ADBE Channel Blur` → Channel blur (docs/11 §5): the four radii are
/// pixels on both sides.
fn channel_blur(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "red", Unit::Px);
    fx.carry(2, "green", Unit::Px);
    fx.carry(3, "blue", Unit::Px);
    fx.carry(4, "alpha", Unit::Px);
    fx.toggle(5, "repeat_edge_pixels");
    fx.drop_ae(6); // Blur Dimensions — Lumit's is always both
}

/// AE `ADBE Linear Wipe` → Linear wipe (docs/11 §5): direct. Lumit's Wipe
/// centre is its own and defaults to the frame centre, which is AE's behaviour.
fn linear_wipe(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "completion", Unit::Direct);
    fx.carry(2, "angle", Unit::Direct);
    fx.carry(3, "feather", Unit::Px);
}

/// AE `ADBE Radial Wipe` → Radial wipe (docs/11 §5): direct, AE's
/// "Counterclockwise" being Lumit's Anticlockwise (docs/01 §9).
fn radial_wipe(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "completion", Unit::Direct);
    fx.carry(2, "start_angle", Unit::Direct);
    fx.point(3, "centre_x", "centre_y", Unit::Px);
    fx.choice(4, "wipe", &[0, 1, 2], "Clockwise");
    fx.carry(5, "feather", Unit::Px);
}

/// AE `ADBE IRIS_WIPE` → Iris wipe (docs/11 §5): the two radii are pixels on
/// both sides (docs/08 §3.71's fourth note).
fn iris_wipe(fx: &mut Fx<'_, '_>) {
    fx.point(1, "centre_x", "centre_y", Unit::Px);
    fx.carry(2, "points", Unit::Direct);
    fx.carry(3, "outer_radius", Unit::Px);
    fx.toggle(4, "use_inner_radius");
    fx.carry(5, "inner_radius", Unit::Px);
    fx.carry(6, "rotation", Unit::Direct);
    fx.carry(7, "feather", Unit::Px);
}

/// AE `ADBE Venetian Blinds` → Venetian blinds (docs/11 §5): Width converts
/// from raster pixels to px@comp, so a preview and an export show the same rank
/// of slats.
fn venetian_blinds(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "completion", Unit::Direct);
    fx.carry(2, "direction", Unit::Direct);
    fx.carry(3, "width", Unit::Px);
    fx.carry(4, "feather", Unit::Px);
}

/// AE `APC CardWipeCam` → Card wipe (docs/11 §5). Four things are reported
/// rather than approximated: the camera system, the back layer, the card scale
/// and Flip Order's Gradient.
fn card_wipe(fx: &mut Fx<'_, '_>) {
    fx.carry(2, "completion", Unit::Direct);
    // Transition width is a per cent of the frame in After Effects and px@comp
    // in Lumit (K-558), measured along whichever axis the flip order runs — so
    // the order has to be read before the width is converted. AE's 3 and 4 are
    // its two vertical orders.
    let (comp_w, comp_h) = fx.conv.size;
    let ae_order = fx.raw(20).unwrap_or(1.0).round() as i64;
    let basis = if matches!(ae_order, 3 | 4) {
        comp_h
    } else {
        comp_w
    };
    fx.carry(4, "transition_width", Unit::Scale(basis / 100.0));
    fx.drop_ae(6); // Back Layer — a card turns to nothing, which is AE with no back layer
                   // AE's Rows & Columns switch: Independent, or Columns Follows Rows.
    let independent = fx.raw(8).unwrap_or(1.0).round() as i64 == 1;
    fx.carry(10, "rows", Unit::Direct);
    fx.carry(if independent { 12 } else { 10 }, "columns", Unit::Direct);
    fx.drop_ae(14); // Card Scale
    fx.choice(16, "flip_axis", &[0, 1, 2], "Horizontal axis");
    fx.choice(18, "flip_direction", &[0, 1, 2], "Forwards");
    let gradient_order = ae_order == 5;
    fx.choice(20, "flip_order", &[0, 1, 2, 3, -1], "Left to right");
    if gradient_order {
        fx.drop_ae(22); // Gradient Layer — its spread is not in the capture
    }
    fx.carry(24, "randomness", Unit::Direct);
    fx.seed(26, "seed");
    if fx.find(28).is_some() {
        // Camera Position, Corner Pins, Composite Camera, Lighting, Material,
        // Position Jitter, Rotation Jitter — one row for the lot, because they
        // are one absent idea: Lumit keeps cameras on the composition (docs/06).
        fx.not_carried("camera system");
    }
}

// The Expression Controls (K-414, docs/11 §5's five pending rows). Each is one
// property onto one row, in the same units, with nothing to convert and nothing
// left behind — the only rows in this file with no report of any kind. What
// makes them worth writing down at all is what they carry: the keyframes and
// the expressions on that one property, which is the whole of a CC-pack rig.

/// AE `ADBE Slider Control` → Slider control (docs/08 §3.80): one number.
fn slider_control(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "slider", Unit::Direct);
}

/// AE `ADBE Angle Control` → Angle control (docs/08 §3.81): degrees on both
/// sides, and unbounded on both, so a rig that winds past 360 winds past it here.
fn angle_control(fx: &mut Fx<'_, '_>) {
    fx.carry(1, "angle", Unit::Direct);
}

/// AE `ADBE Checkbox Control` → Checkbox control (docs/08 §3.82).
fn checkbox_control(fx: &mut Fx<'_, '_>) {
    fx.toggle(1, "checkbox");
}

/// AE `ADBE Color Control` → Colour control (docs/08 §3.83). The colour crosses
/// from the project's display space into scene-linear light like every other
/// imported colour (docs/11 §3).
fn colour_control(fx: &mut Fx<'_, '_>) {
    fx.colour(1, "colour");
}

/// AE `ADBE Point Control` → Point control (docs/08 §3.84): AE's raster pixels
/// become px@comp, which is the same number read against the composition
/// (docs/08 §2.3).
fn point_control(fx: &mut Fx<'_, '_>) {
    fx.point(1, "point_x", "point_y", Unit::Px);
}

// ─────────────────────── the machinery underneath ───────────────────────

/// What has to happen to a number on the way across.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Unit {
    /// An angle, a per cent, a count: the same thing on both sides.
    Direct,
    /// After Effects' raster pixels become px@comp (docs/08 §2.3). The number
    /// is the same and the meaning is not: Lumit scales it by the preview
    /// factor, so a Half preview frames like the export.
    Px,
    /// After Effects' per cent *of the layer* becomes px@comp. AE's 100 is the
    /// circle that just contains the layer — its half-diagonal — so the
    /// pixel count is that per cent of half the comp's diagonal.
    LayerPct,
    /// A bare factor where Lumit reads a per cent (AE's 1.0 is Lumit's 100).
    Factor,
    /// AE's "Blend With Original" against Lumit's Mix: the same dial, read from
    /// the other end.
    Complement,
    /// A plain multiplier, for the two places that need their own.
    Scale(f64),
    /// A plain offset, for AE's destination points.
    Shift(f64),
}

impl Unit {
    /// The value map, as `value·k + d`. A bezier handle's *speed* is in value
    /// units a second, so it takes the `k` and not the `d`.
    fn affine(self, diagonal: f64) -> (f64, f64) {
        match self {
            Self::Direct | Self::Px => (1.0, 0.0),
            // A per cent of the layer's half-diagonal, in pixels.
            Self::LayerPct => ((diagonal / 2.0) / 100.0, 0.0),
            Self::Factor => (100.0, 0.0),
            Self::Complement => (-1.0, 100.0),
            Self::Scale(k) => (k, 0.0),
            Self::Shift(d) => (1.0, d),
        }
    }

    /// Whether the *number* changed for a documented reason, which is what
    /// earns a report row. A rename and a reinterpretation do not.
    fn rebases(self) -> bool {
        matches!(
            self,
            Self::LayerPct | Self::Factor | Self::Scale(_) | Self::Shift(_)
        )
    }
}

/// One effect being converted: the capture node, the growing Lumit instance,
/// and everything a row needs to say what it did.
struct Fx<'a, 'c> {
    conv: &'a mut Conv<'c>,
    /// comp ▸ layer ▸ this effect, the path every row is filed under.
    path: ItemPath,
    node: &'a Property,
    /// The After Effects match name, and therefore the prefix of every
    /// parameter's own match name.
    ae: &'a str,
    /// What the person saw in After Effects' Effect Controls — the name a
    /// report row should use.
    name: String,
    inst: EffectInstance,
}

impl<'a, 'c> Fx<'a, 'c> {
    fn new(
        conv: &'a mut Conv<'c>,
        path: &ItemPath,
        node: &'a Property,
        ae: &'a str,
        lumit: &str,
    ) -> Option<Self> {
        let (w, h) = conv.size;
        let mut inst = lumit_core::fx::instantiate_for_raster(lumit, w, h)?;
        let name = display_name(node, ae).to_string();
        // An effect switched off in After Effects imports switched off.
        inst.enabled = node.enabled.unwrap_or(true);
        // What After Effects called it, kept where a re-import and the report
        // can find it (docs/11 §2.3's `ae` namespace).
        inst.extra = ae_map(vec![
            ("match_name", serde_json::json!(ae)),
            ("name", serde_json::json!(node.name)),
        ]);
        let path = path.property(&name);
        Some(Self {
            conv,
            path,
            node,
            ae,
            name,
            inst,
        })
    }

    /// One numbered After Effects parameter — `ADBE Twirl-0002` — wherever it
    /// sits, since AE tucks some of them inside option groups.
    fn find(&self, n: u32) -> Option<&'a Property> {
        let match_name = format!("{}-{n:04}", self.ae);
        props::find(self.node.children(), &match_name)
    }

    /// Overwrite one declared Lumit parameter, leaving the schema default in
    /// place when After Effects had nothing to say.
    fn set(&mut self, id: &str, value: EffectValue) {
        match self.inst.params.iter_mut().find(|p| p.id == id) {
            Some(param) => param.value = value,
            // A name that is not in the schema is a typo in the table, and the
            // test suite is where it should be caught rather than in a project.
            None => debug_assert!(
                false,
                "{} has no parameter {id}",
                self.inst.effect.match_name
            ),
        }
    }

    fn row(&mut self, outcome: Outcome, reason: Reason) {
        self.conv.report.row(self.path.clone(), outcome, reason);
    }

    /// docs/11 §5's "reported rather than approximated".
    fn not_carried(&mut self, param: &str) {
        let effect = self.name.clone();
        self.row(
            Outcome::Adjusted,
            Reason::EffectParamNotCarried {
                effect,
                param: param.to_string(),
            },
        );
    }

    /// The same, naming the After Effects parameter by the name AE gave it —
    /// and only when the capture actually carried one.
    fn drop_ae(&mut self, n: u32) {
        let Some(leaf) = self.find(n) else { return };
        let param = display_name(leaf, "").trim().to_string();
        if param.is_empty() {
            return;
        }
        self.not_carried(&param);
    }

    /// After Effects' own name for one parameter slot, as the report prints it.
    /// Empty when the file has no leaf there at all — the file stores only what
    /// is not at its default.
    fn ae_name(&self, n: u32) -> String {
        self.find(n)
            .map(|leaf| display_name(leaf, "").trim().to_string())
            .unwrap_or_default()
    }

    fn approximated(&mut self, param: &str, imported_as: &str) {
        let effect = self.name.clone();
        self.row(
            Outcome::Adjusted,
            Reason::EffectParamApproximated {
                effect,
                param: param.to_string(),
                imported_as: imported_as.to_string(),
            },
        );
    }

    fn rebased(&mut self, param: &str) {
        let effect = self.name.clone();
        self.row(
            Outcome::Adjusted,
            Reason::EffectParamRebased {
                effect,
                param: param.to_string(),
            },
        );
    }

    fn differs(&mut self, detail: &str) {
        let effect = self.name.clone();
        self.row(
            Outcome::Adjusted,
            Reason::EffectDiffers {
                effect,
                detail: detail.to_string(),
            },
        );
    }

    /// A whole animatable parameter, keyframes and all.
    fn carry(&mut self, n: u32, id: &str, unit: Unit) {
        self.carry_axis(n, 0, id, unit);
    }

    /// One axis of one — an AE point is one property with two numbers in it.
    fn carry_axis(&mut self, n: u32, axis: usize, id: &str, unit: Unit) {
        let Some(leaf) = self.find(n) else { return };
        let ae_name = display_name(leaf, "").trim().to_string();
        // The Lumit default stands in for anything After Effects could not read,
        // so a broken property leaves a sensible effect rather than a zeroed one.
        let fallback = self.inst.float_at(id, 0.0).unwrap_or(0.0);
        let carried = from_node(self.conv, &self.path, leaf, axis, fallback);

        let expression = matches!(carried.animation, Animation::Expression(_));
        let (k, d) = unit.affine(self.conv.diagonal());
        let converted = affine(carried, k, d);
        if unit.rebases() {
            self.rebased(&ae_name);
        }
        if expression && unit != Unit::Direct && unit != Unit::Px {
            // An expression computes in After Effects' units and nothing here
            // can rewrite it; the number it produces will be in the old base.
            self.approximated(
                &ae_name,
                "an expression still written in After Effects' units",
            );
        }
        self.set(id, EffectValue::Float(converted));
    }

    /// Both axes of an After Effects point.
    fn point(&mut self, n: u32, id_x: &str, id_y: &str, unit: Unit) {
        self.carry_axis(n, 0, id_x, unit);
        self.carry_axis(n, 1, id_y, unit);
    }

    /// The same number without a report row: for the two rows that have to look
    /// at an option *and* then map it, so the reading does not file twice.
    fn raw(&self, n: u32) -> Option<f64> {
        let leaf = self.find(n)?;
        match leaf.keyframes.as_deref() {
            Some(keys) if !keys.is_empty() => keys
                .first()
                .and_then(|k| k.v.as_ref())
                .and_then(|v| axis_of(v, 0)),
            _ => leaf.value.as_ref().and_then(|v| axis_of(v, 0)),
        }
    }

    /// The still number behind a control Lumit does not animate — an option, a
    /// switch, a seed. A keyframed one imports at the value it starts on.
    fn still(&mut self, n: u32) -> Option<f64> {
        let leaf = self.find(n)?;
        let ae_name = display_name(leaf, "").trim().to_string();
        if leaf.unreadable.is_some() {
            let match_name = match_name_of(leaf).to_string();
            self.row(Outcome::Skipped, Reason::PropertyUnreadable { match_name });
            return None;
        }
        if let Some(keys) = leaf.keyframes.as_deref().filter(|k| !k.is_empty()) {
            let first = keys
                .first()
                .and_then(|k| k.v.as_ref())
                .and_then(|v| axis_of(v, 0));
            self.approximated(
                &ae_name,
                "the value it starts on — Lumit's control is not animated",
            );
            return first;
        }
        leaf.value.as_ref().and_then(|v| axis_of(v, 0))
    }

    /// An After Effects dropdown. `table` maps AE's **1-based** index onto a
    /// Lumit option index, `-1` meaning "no counterpart": the Lumit default
    /// stands and the row says so, rather than the closest guess being written.
    fn choice(&mut self, n: u32, id: &str, table: &[i32], default_label: &str) {
        let Some(value) = self.still(n) else { return };
        let ae_name = self
            .find(n)
            .map(|leaf| display_name(leaf, "").trim().to_string())
            .unwrap_or_default();
        let index = value.round() as i64;
        let mapped = usize::try_from(index - 1)
            .ok()
            .and_then(|i| table.get(i).copied())
            .filter(|o| *o >= 0);
        match mapped {
            Some(option) => self.set(id, EffectValue::Choice(option.unsigned_abs())),
            None => self.approximated(&ae_name, default_label),
        }
    }

    /// An After Effects checkbox.
    fn toggle(&mut self, n: u32, id: &str) {
        let Some(value) = self.still(n) else { return };
        self.set(id, EffectValue::Bool(value != 0.0));
    }

    /// An After Effects random seed.
    fn seed(&mut self, n: u32, id: &str) {
        let Some(value) = self.still(n) else { return };
        let seed = value.round().clamp(0.0, f64::from(u32::MAX)) as u32;
        self.set(id, EffectValue::Seed(seed));
    }

    /// A colour, from After Effects' display space into scene-linear light
    /// (docs/11 §3 — values convert on import).
    fn colour(&mut self, n: u32, id: &str) {
        let Some(leaf) = self.find(n) else { return };
        let Some(value) = leaf.value.as_ref() else {
            return;
        };
        let channel = |i: usize| f64::from(srgb_to_linear(axis_of(value, i).unwrap_or(0.0)));
        self.set(
            id,
            EffectValue::Colour([
                LumProperty::fixed(channel(0)),
                LumProperty::fixed(channel(1)),
                LumProperty::fixed(channel(2)),
                LumProperty::fixed(axis_of(value, 3).unwrap_or(1.0)),
            ]),
        );
    }

    /// An After Effects layer reference: its 1-based stacking index becomes the
    /// same Lumit id that layer itself was given. AE's index 0 is "None".
    fn layer(&mut self, n: u32, id: &str) {
        let Some(value) = self.still(n) else { return };
        let index = value.round();
        if index < 1.0 {
            return;
        }
        let index = index as u32;
        match self.conv.layer_ids.get(&index).copied() {
            Some(layer) => self.set(id, EffectValue::Layer(Some(layer))),
            None => self.row(Outcome::Adjusted, Reason::MatteTargetMissing { index }),
        }
    }

    /// AE's ten-entry channel picker onto docs/08 §1.2's shared five. Hue,
    /// Lightness, Saturation and Full collapse onto Luminance and say so; Off
    /// means "do not displace on this axis", which is an Amount of 0.
    fn channel(&mut self, n: u32, id: &str, amount: Option<&str>) {
        let Some(value) = self.still(n) else { return };
        let ae_name = self
            .find(n)
            .map(|leaf| display_name(leaf, "").trim().to_string())
            .unwrap_or_default();
        // AE: Red, Green, Blue, Alpha, Luminance, Hue, Lightness, Saturation,
        // Full, Off. Lumit: Luminance, Alpha, Red, Green, Blue.
        let option = match value.round() as i64 {
            1 => 2,
            2 => 3,
            3 => 4,
            4 => 1,
            5 => 0,
            other => {
                self.approximated(&ae_name, "Luminance");
                if other == 10 {
                    if let Some(amount) = amount {
                        self.set(amount, float(0.0));
                    }
                }
                0
            }
        };
        self.set(id, EffectValue::Choice(option));
    }

    /// The sign of a signed After Effects control, read once so a split can put
    /// it in its own Lumit parameter (Spherize).
    fn sign(&mut self, n: u32) -> f64 {
        let Some(leaf) = self.find(n) else { return 1.0 };
        let values: Vec<f64> = match leaf.keyframes.as_deref() {
            Some(keys) if !keys.is_empty() => keys
                .iter()
                .filter_map(|k| k.v.as_ref().and_then(|v| axis_of(v, 0)))
                .collect(),
            _ => leaf
                .value
                .as_ref()
                .and_then(|v| axis_of(v, 0))
                .into_iter()
                .collect(),
        };
        let negative = values.first().copied().unwrap_or(0.0) < 0.0;
        if values.iter().any(|v| (*v < 0.0) != negative) {
            let ae_name = display_name(leaf, "").trim().to_string();
            self.approximated(
                &ae_name,
                "one direction — an animation that crosses zero changes which control it is",
            );
        }
        if negative {
            -1.0
        } else {
            1.0
        }
    }

    /// An After Effects control that reads the clock (Ripple's Wave Speed, Wave
    /// warp's) becomes two keyframes on Lumit's phase-like dial across the
    /// layer's own span: the same motion, and deterministic (docs/08 §2.4).
    fn clock(&mut self, speed_n: u32, phase_n: u32, id: &str) {
        let speed = self.still(speed_n).unwrap_or(0.0);
        let phase_keyed = self
            .find(phase_n)
            .is_some_and(|p| p.keyframes.as_deref().is_some_and(|k| !k.is_empty()));
        self.carry(phase_n, id, Unit::Direct);
        if !speed.is_finite() || speed == 0.0 {
            return;
        }
        if phase_keyed {
            // The phase's own keys already say where the wave is; two more
            // would fight them.
            self.not_carried("Wave Speed");
            return;
        }
        let (from, to) = self.conv.span;
        let seconds = to.to_f64() - from.to_f64();
        if seconds <= 0.0 {
            return;
        }
        let base = self.inst.float_at(id, 0.0).unwrap_or(0.0);
        // One turn of the dial is one whole wave, so a speed of s is 360·s
        // degrees a second.
        let end = base + 360.0 * speed * seconds;
        self.set(id, EffectValue::Float(props::ramp(from, base, to, end)));
        let effect = self.name.clone();
        self.row(
            Outcome::Adjusted,
            Reason::EffectSpeedAsKeyframes {
                effect,
                param: "Wave Speed".to_string(),
            },
        );
    }

    /// Hold a parameter to a documented cap, and say whether it bit.
    fn clamp(&mut self, id: &str, max: f64) -> bool {
        let Some(EffectValue::Float(property)) = self.inst.param(id).cloned() else {
            return false;
        };
        let over = match &property.animation {
            Animation::Static(v) => *v > max,
            Animation::Keyframed(keys) => keys.iter().any(|k| k.value > max),
            Animation::Expression(_) => false,
        };
        if !over {
            return false;
        }
        let animation = match property.animation {
            Animation::Static(v) => Animation::Static(v.min(max)),
            Animation::Keyframed(keys) => Animation::Keyframed(
                keys.into_iter()
                    .map(|mut k| {
                        k.value = k.value.min(max);
                        k
                    })
                    .collect(),
            ),
            other => other,
        };
        self.set(
            id,
            EffectValue::Float(LumProperty {
                animation,
                extra: property.extra,
            }),
        );
        true
    }
}

/// A static Lumit parameter value.
fn float(v: f64) -> EffectValue {
    EffectValue::Float(LumProperty::fixed(v))
}

/// `value·k + d` applied to the still value and to every keyframe, with each
/// bezier handle's speed taking the `k` — a speed is value units a second, so a
/// rescaled value graph needs a rescaled speed graph or the curve changes shape.
fn affine(property: LumProperty, k: f64, d: f64) -> LumProperty {
    if (k - 1.0).abs() < f64::EPSILON && d == 0.0 {
        return property;
    }
    let side = |s: SideInterp| match s {
        SideInterp::Bezier { speed, influence } => SideInterp::Bezier {
            speed: speed * k,
            influence,
        },
        other => other,
    };
    let animation = match property.animation {
        Animation::Static(v) => Animation::Static(v * k + d),
        Animation::Keyframed(keys) => Animation::Keyframed(
            keys.into_iter()
                .map(|key| Keyframe {
                    value: key.value * k + d,
                    interp_in: side(key.interp_in),
                    interp_out: side(key.interp_out),
                    ..key
                })
                .collect(),
        ),
        // An expression is source text in After Effects' units; rewriting one
        // is a different piece of work (docs/12 §4).
        other => other,
    };
    LumProperty {
        animation,
        extra: property.extra,
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod tests {
    use std::collections::BTreeMap;

    use lumit_core::time::Rational;
    use uuid::Uuid;

    use super::super::time::TimeBase;
    use super::*;
    use crate::capture::{Ease, Keyframe as AeKey, Property as AeProp};
    use crate::report::ImportReport;

    const W: f64 = 1920.0;
    const H: f64 = 1080.0;

    /// The comp diagonal, the base AE's per-cent-of-layer radii convert
    /// through — computed from the composition, never copied in as a constant.
    fn diag() -> f64 {
        (W * W + H * H).sqrt()
    }

    /// The two layers the test composition has, by After Effects index.
    fn layer_id(index: u32) -> Uuid {
        Uuid::from_u128(u128::from(index))
    }

    // ── building a captured instance ──

    fn fx(ae: &str, name: &str, params: Vec<AeProp>) -> AeProp {
        AeProp {
            match_name: Some(ae.to_string()),
            name: Some(name.to_string()),
            enabled: Some(true),
            group: Some(params),
            ..AeProp::default()
        }
    }

    fn leaf(ae: &str, n: u32, name: &str, kind: &str, value: serde_json::Value) -> AeProp {
        AeProp {
            match_name: Some(format!("{ae}-{n:04}")),
            name: Some(name.to_string()),
            value_type: Some(kind.to_string()),
            value: Some(value),
            ..AeProp::default()
        }
    }

    fn num(ae: &str, n: u32, name: &str, v: f64) -> AeProp {
        leaf(ae, n, name, "float", serde_json::json!(v))
    }

    fn point(ae: &str, n: u32, name: &str, x: f64, y: f64) -> AeProp {
        leaf(ae, n, name, "point", serde_json::json!([x, y]))
    }

    fn colour(ae: &str, n: u32, name: &str, c: [f64; 4]) -> AeProp {
        leaf(ae, n, name, "colour", serde_json::json!(c))
    }

    fn layer_ref(ae: &str, n: u32, name: &str, index: i64) -> AeProp {
        leaf(ae, n, name, "layer", serde_json::json!(index))
    }

    /// A keyframed float: bezier keys, so a conversion has both a value and a
    /// handle speed to get right.
    fn keyed(ae: &str, n: u32, name: &str, keys: &[(f64, f64, f64)]) -> AeProp {
        AeProp {
            match_name: Some(format!("{ae}-{n:04}")),
            name: Some(name.to_string()),
            value_type: Some("float".to_string()),
            keyframes: Some(
                keys.iter()
                    .map(|(t, v, speed)| AeKey {
                        t: Some(*t),
                        v: Some(serde_json::json!(v)),
                        in_interp: Some("BEZIER".to_string()),
                        out_interp: Some("BEZIER".to_string()),
                        in_ease: Some(vec![Ease {
                            speed: Some(*speed),
                            influence: Some(50.0),
                        }]),
                        out_ease: Some(vec![Ease {
                            speed: Some(*speed),
                            influence: Some(50.0),
                        }]),
                        ..AeKey::default()
                    })
                    .collect(),
            ),
            ..AeProp::default()
        }
    }

    // ── running one ──

    struct Run {
        inst: EffectInstance,
        report: ImportReport,
    }

    fn run(node: &AeProp) -> Run {
        let mut report = ImportReport::default();
        let tb = TimeBase::fallback();
        let ids: BTreeMap<u32, Uuid> = (1..=3).map(|i| (i, layer_id(i))).collect();
        let inst = {
            let mut conv = Conv {
                report: &mut report,
                tb,
                offset: Rational::ZERO,
                size: (W, H),
                span: (Rational::ZERO, tb.seconds(2.0)),
                layer_ids: ids,
                masks: Vec::new(),
                // The composition's first layer is the one the effect is on,
                // which is what "this layer" means to a layer-reference row.
                self_index: 1,
            };
            let path = ItemPath::item("Comp 1").layer("Layer 1");
            claim(&mut conv, &path, node).expect("the table claims this match name")
        };
        Run { inst, report }
    }

    // ── reading the answer ──

    fn f(r: &Run, id: &str) -> f64 {
        r.inst.float_at(id, 0.0).expect("a float parameter")
    }

    fn choice(r: &Run, id: &str) -> u32 {
        match r.inst.param(id) {
            Some(EffectValue::Choice(v)) => *v,
            other => panic!("{id} is not a choice: {other:?}"),
        }
    }

    fn boolean(r: &Run, id: &str) -> bool {
        match r.inst.param(id) {
            Some(EffectValue::Bool(v)) => *v,
            other => panic!("{id} is not a switch: {other:?}"),
        }
    }

    fn seed_of(r: &Run, id: &str) -> u32 {
        match r.inst.param(id) {
            Some(EffectValue::Seed(v)) => *v,
            other => panic!("{id} is not a seed: {other:?}"),
        }
    }

    fn layer_of(r: &Run, id: &str) -> Option<Uuid> {
        match r.inst.param(id) {
            Some(EffectValue::Layer(v)) => *v,
            other => panic!("{id} is not a layer reference: {other:?}"),
        }
    }

    /// Every key of an animated parameter as (seconds, value, out-handle speed).
    fn keys_of(r: &Run, id: &str) -> Vec<(f64, f64, f64)> {
        let Some(EffectValue::Float(p)) = r.inst.param(id) else {
            panic!("{id} is not a float");
        };
        let Animation::Keyframed(keys) = &p.animation else {
            panic!("{id} is not animated: {:?}", p.animation);
        };
        keys.iter()
            .map(|k| {
                let speed = match k.interp_out {
                    SideInterp::Bezier { speed, .. } => speed,
                    _ => 0.0,
                };
                (k.time.to_f64(), k.value, speed)
            })
            .collect()
    }

    fn rebased(r: &Run, param: &str) -> bool {
        r.report.rows.iter().any(
            |row| matches!(&row.reason, Reason::EffectParamRebased { param: p, .. } if p == param),
        )
    }

    fn dropped(r: &Run, param: &str) -> bool {
        r.report.rows.iter().any(|row| {
            matches!(&row.reason, Reason::EffectParamNotCarried { param: p, .. } if p == param)
        })
    }

    fn approximated(r: &Run, param: &str) -> bool {
        r.report.rows.iter().any(|row| {
            matches!(&row.reason, Reason::EffectParamApproximated { param: p, .. } if p == param)
        })
    }

    fn differs(r: &Run) -> bool {
        r.report
            .rows
            .iter()
            .any(|row| matches!(&row.reason, Reason::EffectDiffers { .. }))
    }

    // ───────────────────────── the rows ─────────────────────────

    /// A match name neither half knows is not this table's, so the placeholder
    /// road stays open (docs/11 §5's "never the closest guess").
    #[test]
    fn an_unknown_match_name_is_not_claimed() {
        let mut report = ImportReport::default();
        let tb = TimeBase::fallback();
        let mut conv = Conv {
            report: &mut report,
            tb,
            offset: Rational::ZERO,
            size: (W, H),
            span: (Rational::ZERO, tb.seconds(1.0)),
            layer_ids: BTreeMap::new(),
            masks: Vec::new(),
            self_index: 1,
        };
        let node = fx("RE:Vision Twixtor", "Twixtor", vec![]);
        assert!(claim(&mut conv, &ItemPath::default(), &node).is_none());
    }

    /// An effect switched off in After Effects imports switched off, and keeps
    /// what After Effects called it in the `ae` namespace.
    #[test]
    fn a_disabled_instance_imports_disabled_and_keeps_its_ae_name() {
        let ae = "ADBE Twirl";
        let mut node = fx(ae, "Whirlpool", vec![num(ae, 1, "Angle", 90.0)]);
        node.enabled = Some(false);
        let r = run(&node);
        assert!(!r.inst.enabled);
        assert_eq!(r.inst.effect.match_name, "twirl");
        assert_eq!(
            r.inst.extra["ae"]["match_name"],
            serde_json::json!("ADBE Twirl")
        );
        assert_eq!(r.inst.extra["ae"]["name"], serde_json::json!("Whirlpool"));
    }

    #[test]
    fn transform_carries_the_two_points_the_scales_and_reports_the_skew() {
        let ae = "ADBE Geometry2";
        let r = run(&fx(
            ae,
            "Transform",
            vec![
                point(ae, 1, "Anchor Point", 100.0, 200.0),
                point(ae, 2, "Position", 300.0, 400.0),
                num(ae, 11, "Uniform Scale", 0.0),
                num(ae, 3, "Scale Height", 80.0),
                num(ae, 4, "Scale Width", 120.0),
                num(ae, 5, "Skew", 15.0),
                num(ae, 6, "Skew Axis", 45.0),
                keyed(ae, 7, "Rotation", &[(0.0, 0.0, 0.0), (2.0, 90.0, 45.0)]),
                num(ae, 8, "Opacity", 60.0),
                num(ae, 12, "Sampling", 2.0),
            ],
        ));
        assert_eq!((f(&r, "anchor_x"), f(&r, "anchor_y")), (100.0, 200.0));
        assert_eq!((f(&r, "position_x"), f(&r, "position_y")), (300.0, 400.0));
        assert_eq!((f(&r, "scale_x"), f(&r, "scale_y")), (120.0, 80.0));
        assert_eq!(f(&r, "opacity"), 60.0);
        // The rotation's keys carry unchanged: an angle is an angle.
        assert_eq!(
            keys_of(&r, "rotation"),
            vec![(0.0, 0.0, 0.0), (2.0, 90.0, 45.0)]
        );
        assert!(dropped(&r, "Skew") && dropped(&r, "Skew Axis") && dropped(&r, "Sampling"));
    }

    /// AE's Uniform Scale hides Scale Width; the tie is resolved rather than
    /// carried, so both Lumit axes take the one number the person was editing.
    #[test]
    fn transforms_uniform_scale_drives_both_axes() {
        let ae = "ADBE Geometry2";
        let r = run(&fx(
            ae,
            "Transform",
            vec![
                num(ae, 11, "Uniform Scale", 1.0),
                num(ae, 3, "Scale Height", 250.0),
                num(ae, 4, "Scale Width", 100.0),
            ],
        ));
        assert_eq!((f(&r, "scale_x"), f(&r, "scale_y")), (250.0, 250.0));
    }

    #[test]
    fn motion_tile_converts_its_four_sizes_and_carries_the_rest() {
        let ae = "ADBE Tile";
        let r = run(&fx(
            ae,
            "Motion Tile",
            vec![
                point(ae, 1, "Tile Center", 480.0, 270.0),
                keyed(
                    ae,
                    2,
                    "Tile Width",
                    &[(0.0, 100.0, 0.0), (2.0, 25.0, -50.0)],
                ),
                num(ae, 3, "Tile Height", 40.0),
                num(ae, 4, "Output Width", 150.0),
                num(ae, 5, "Output Height", 120.0),
                num(ae, 6, "Mirror Edges", 1.0),
                num(ae, 7, "Phase", 180.0),
                num(ae, 8, "Horizontal Phase Shift", 1.0),
            ],
        ));
        assert_eq!(
            (f(&r, "tile_centre_x"), f(&r, "tile_centre_y")),
            (480.0, 270.0)
        );
        // AE keeps the four sizes as per cents of the frame and Lumit as
        // px@comp (K-558), so each axis converts against the comp's own extent
        // — keyframes and their speeds with it.
        assert_eq!(
            keys_of(&r, "tile_width"),
            vec![(0.0, 1.00 * W, 0.0), (2.0, 0.25 * W, -0.50 * W)]
        );
        assert_eq!(f(&r, "tile_height"), 0.40 * H);
        assert_eq!(
            (f(&r, "output_width"), f(&r, "output_height")),
            (1.50 * W, 1.20 * H)
        );
        assert!(boolean(&r, "mirror_edges") && boolean(&r, "horizontal_phase_shift"));
        assert_eq!(f(&r, "phase"), 180.0);
        // The four conversions are the only rows: a number whose base changed
        // is reported, and everything else here is one for one.
        for name in ["Tile Width", "Tile Height", "Output Width", "Output Height"] {
            assert!(rebased(&r, name), "{name} changed base and must say so");
        }
        assert_eq!(r.report.rows.len(), 4);
    }

    /// AE's "Shift Center To" is a destination and Lumit stores the shift, so
    /// the frame centre comes off.
    #[test]
    fn offset_subtracts_the_frame_centre_and_reads_blend_from_the_other_end() {
        let ae = "ADBE Offset";
        let r = run(&fx(
            ae,
            "Offset",
            vec![
                point(ae, 1, "Shift Center To", 1200.0, 300.0),
                num(ae, 2, "Blend With Original", 25.0),
            ],
        ));
        assert_eq!(f(&r, "shift_x"), 1200.0 - W / 2.0);
        assert_eq!(f(&r, "shift_y"), 300.0 - H / 2.0);
        assert_eq!(f(&r, "mix"), 75.0);
        assert!(rebased(&r, "Shift Center To"));
    }

    #[test]
    fn mirror_converts_one_for_one() {
        let ae = "ADBE Mirror";
        let r = run(&fx(
            ae,
            "Mirror",
            vec![
                point(ae, 1, "Reflection Center", 640.0, 360.0),
                keyed(
                    ae,
                    2,
                    "Reflection Angle",
                    &[(0.0, 0.0, 0.0), (2.0, 45.0, 22.5)],
                ),
            ],
        ));
        assert_eq!((f(&r, "centre_x"), f(&r, "centre_y")), (640.0, 360.0));
        assert_eq!(
            keys_of(&r, "angle"),
            vec![(0.0, 0.0, 0.0), (2.0, 45.0, 22.5)]
        );
    }

    #[test]
    fn optics_compensation_maps_the_look_and_reports_the_resize() {
        let ae = "ADBE Optics Compensation";
        let r = run(&fx(
            ae,
            "Optics Compensation",
            vec![
                keyed(
                    ae,
                    1,
                    "Field Of View (FOV)",
                    &[(0.0, 0.0, 0.0), (2.0, 60.0, 30.0)],
                ),
                num(ae, 2, "Reverse Lens Distortion", 1.0),
                num(ae, 3, "FOV Orientation", 2.0),
                point(ae, 4, "View Center", 100.0, 100.0),
                num(ae, 5, "Optimal Pixels (Invalidates Reversal)", 1.0),
                num(ae, 6, "Resize", 2.0),
            ],
        ));
        assert_eq!(keys_of(&r, "fov"), vec![(0.0, 0.0, 0.0), (2.0, 60.0, 30.0)]);
        assert!(boolean(&r, "reverse"));
        assert_eq!(
            choice(&r, "orientation"),
            1,
            "AE's second entry is Vertical"
        );
        assert_eq!((f(&r, "centre_x"), f(&r, "centre_y")), (100.0, 100.0));
        assert!(dropped(&r, "Resize"));
        assert!(dropped(&r, "Optimal Pixels (Invalidates Reversal)"));
    }

    #[test]
    fn turbulent_displace_maps_three_modes_and_reports_the_rest() {
        let ae = "ADBE Turbulent Displace";
        let r = run(&fx(
            ae,
            "Turbulent Displace",
            vec![
                num(ae, 1, "Displacement", 7.0),
                keyed(ae, 2, "Amount", &[(0.0, 20.0, 0.0), (2.0, 90.0, 35.0)]),
                num(ae, 3, "Size", 250.0),
                point(ae, 4, "Offset (Turbulence)", 200.0, 300.0),
                num(ae, 5, "Complexity", 4.0),
                num(ae, 6, "Evolution", 720.0),
                num(ae, 8, "Cycle Evolution", 1.0),
                num(ae, 9, "Cycle (in Revolutions)", 3.0),
                num(ae, 10, "Random Seed", 17.0),
                num(ae, 12, "Pinning", 3.0),
                num(ae, 13, "Resize Layer", 1.0),
            ],
        ));
        assert_eq!(choice(&r, "displacement"), 2, "AE's seventh is Vertical");
        // Amount is a length on both sides: px@comp is the same number, and the
        // handle speed rides with it.
        assert_eq!(
            keys_of(&r, "amount"),
            vec![(0.0, 20.0, 0.0), (2.0, 90.0, 35.0)]
        );
        assert_eq!(f(&r, "size"), 250.0);
        assert_eq!((f(&r, "offset_x"), f(&r, "offset_y")), (200.0, 300.0));
        assert_eq!(f(&r, "complexity"), 4.0);
        assert_eq!(f(&r, "evolution"), 720.0);
        assert!(boolean(&r, "cycle_evolution"));
        assert_eq!(f(&r, "cycle"), 3.0);
        assert_eq!(seed_of(&r, "seed"), 17);
        assert_eq!(choice(&r, "pinning"), 1, "AE's default is every edge");
        assert!(dropped(&r, "Resize Layer"));
    }

    /// The seven mixed pinning combinations, and every index the audit does not
    /// pin, are reported rather than mapped to a guess.
    #[test]
    fn turbulent_displaces_other_pinnings_are_reported() {
        let ae = "ADBE Turbulent Displace";
        let r = run(&fx(
            ae,
            "Turbulent Displace",
            vec![num(ae, 12, "Pinning", 6.0)],
        ));
        assert!(approximated(&r, "Pinning"));
        assert_eq!(choice(&r, "pinning"), 1, "Lumit's own default stands");
    }

    /// AE stores the shadow's opacity 0..255 where Lumit reads a per cent.
    #[test]
    fn drop_shadow_rebases_the_opacity_and_carries_the_direction() {
        let ae = "ADBE Drop Shadow";
        let r = run(&fx(
            ae,
            "Drop Shadow",
            vec![
                colour(ae, 1, "Shadow Color", [1.0, 0.0, 0.0, 1.0]),
                keyed(ae, 2, "Opacity", &[(0.0, 255.0, 0.0), (2.0, 51.0, -102.0)]),
                num(ae, 3, "Direction", 200.0),
                num(ae, 4, "Distance", 30.0),
                num(ae, 5, "Softness", 12.0),
                num(ae, 6, "Shadow Only", 1.0),
            ],
        ));
        let k = keys_of(&r, "opacity");
        assert_eq!(k[0], (0.0, 100.0, 0.0));
        assert_eq!(k[1].1, 51.0 * 100.0 / 255.0);
        assert_eq!(k[1].2, -102.0 * 100.0 / 255.0, "the handle rides with it");
        assert_eq!(f(&r, "direction"), 200.0);
        assert_eq!((f(&r, "distance"), f(&r, "softness")), (30.0, 12.0));
        assert!(boolean(&r, "shadow_only"));
        assert!(rebased(&r, "Opacity"));
        // Red, from After Effects' display space into scene-linear light.
        match r.inst.param("shadow_colour") {
            Some(EffectValue::Colour(c)) => {
                assert_eq!(c[0].value_at(0.0), 1.0);
                assert_eq!(c[1].value_at(0.0), 0.0);
            }
            other => panic!("not a colour: {other:?}"),
        }
    }

    #[test]
    fn set_matte_takes_the_universal_matte_row_and_reports_the_two_fittings() {
        let ae = "ADBE Set Matte3";
        let r = run(&fx(
            ae,
            "Set Matte",
            vec![
                layer_ref(ae, 1, "Take Matte From Layer", 2),
                num(ae, 2, "Use For Matte", 4.0),
                num(ae, 3, "Invert Matte", 1.0),
                num(ae, 4, "If Layer Sizes Differ", 1.0),
                num(ae, 5, "Composite Matte with Original", 1.0),
                num(ae, 6, "Premultiply Matte Layer", 1.0),
            ],
        ));
        assert_eq!(layer_of(&r, "matte"), Some(layer_id(2)));
        assert_eq!(
            choice(&r, "channel"),
            1,
            "AE's Alpha is Lumit's second entry"
        );
        assert!(boolean(&r, "matte_invert"));
        assert!(boolean(&r, "combine"));
        assert!(dropped(&r, "If Layer Sizes Differ"));
        assert!(dropped(&r, "Premultiply Matte Layer"));
    }

    /// **Set channels: four After Effects source layers onto one Source row.**
    ///
    /// The three cases that actually occur in the reference project, in one
    /// instance each. Every channel that names the chosen source layer, or the
    /// layer the effect is on, converts exactly; a *second* source layer and
    /// After Effects' "None" are both reported and left at the identity, which
    /// is §5's "reported rather than approximated".
    #[test]
    fn set_channels_folds_four_source_layers_onto_one_source_row() {
        let ae = "ADBE Set Channels";
        // The commonest shape by far: all four channels off one other layer,
        // the identity assignment. Six of the project's ten instances.
        let r = run(&fx(
            ae,
            "Set Channels",
            vec![
                layer_ref(ae, 1, "Source Layer 1", 2),
                num(ae, 2, "Set Red To Source 1's", 1.0),
                layer_ref(ae, 3, "Source Layer 2", 2),
                num(ae, 4, "Set Green To Source 2's", 2.0),
                layer_ref(ae, 5, "Source Layer 3", 2),
                num(ae, 6, "Set Blue To Source 3's", 3.0),
                layer_ref(ae, 7, "Source Layer 4", 2),
                num(ae, 8, "Set Alpha To Source 4's", 4.0),
            ],
        ));
        assert_eq!(layer_of(&r, "source"), Some(layer_id(2)));
        assert_eq!(choice(&r, "red_from"), 5, "Source red");
        assert_eq!(choice(&r, "green_from"), 6, "Source green");
        assert_eq!(choice(&r, "blue_from"), 7, "Source blue");
        assert_eq!(choice(&r, "alpha_from"), 8, "Source alpha");

        // Three channels off another layer, the alpha off **this** one, which
        // is After Effects' own default for a picker nobody touched. This
        // layer's channels are the first five options, so it needs no Source
        // row and no report.
        let r = run(&fx(
            ae,
            "Set Channels",
            vec![
                layer_ref(ae, 1, "Source Layer 1", 3),
                num(ae, 2, "Set Red To Source 1's", 1.0),
                layer_ref(ae, 3, "Source Layer 2", 3),
                num(ae, 4, "Set Green To Source 2's", 2.0),
                layer_ref(ae, 5, "Source Layer 3", 3),
                num(ae, 6, "Set Blue To Source 3's", 3.0),
                layer_ref(ae, 7, "Source Layer 4", 1),
                num(ae, 8, "Set Alpha To Source 4's", 5.0),
            ],
        ));
        assert_eq!(layer_of(&r, "source"), Some(layer_id(3)));
        assert_eq!(choice(&r, "red_from"), 5, "Source red");
        assert_eq!(choice(&r, "alpha_from"), 4, "this layer's own luminance");
        assert!(
            !approximated(&r, "Source Layer 4"),
            "naming the layer the effect is on is exact, not an approximation"
        );

        // **An absent picker is the layer the effect is on**, not None: the
        // file stores only what is not at its default, and that is the default.
        let r = run(&fx(
            ae,
            "Set Channels",
            vec![num(ae, 8, "Set Alpha To Source 4's", 5.0)],
        ));
        assert_eq!(layer_of(&r, "source"), None, "nothing named a second layer");
        assert_eq!(choice(&r, "alpha_from"), 4, "this layer's own luminance");
        assert!(!approximated(&r, "Source Layer 4"));

        // A second source layer, After Effects' None, and one of the three
        // channels Lumit does not have. All three are reported; none of them
        // guesses at a picture.
        let r = run(&fx(
            ae,
            "Set Channels",
            vec![
                layer_ref(ae, 1, "Source Layer 1", 2),
                num(ae, 2, "Set Red To Source 1's", 9.0),
                layer_ref(ae, 3, "Source Layer 2", 3),
                num(ae, 4, "Set Green To Source 2's", 2.0),
                layer_ref(ae, 5, "Source Layer 3", 0),
                num(ae, 6, "Set Blue To Source 3's", 3.0),
                layer_ref(ae, 7, "Source Layer 4", 2),
                num(ae, 8, "Set Alpha To Source 4's", 7.0),
            ],
        ));
        assert_eq!(layer_of(&r, "source"), Some(layer_id(2)));
        assert_eq!(choice(&r, "red_from"), 10, "AE's Full On");
        assert_eq!(choice(&r, "green_from"), 1, "the second source is refused");
        assert!(
            approximated(&r, "Source Layer 2"),
            "the picker is what could not be carried, so the picker is what the report names"
        );
        assert_eq!(choice(&r, "blue_from"), 2, "None is refused");
        assert!(approximated(&r, "Source Layer 3"));
        assert_eq!(
            choice(&r, "alpha_from"),
            9,
            "Lightness collapses onto the chosen source's luminance"
        );
        assert!(approximated(&r, "Set Alpha To Source 4's"));
    }

    /// AE's radii are raster pixels and Lumit's are px@comp, so the import
    /// carries them unchanged — keys and handles alike.
    #[test]
    fn channel_blur_carries_its_radii_as_pixels() {
        let ae = "ADBE Channel Blur";
        let r = run(&fx(
            ae,
            "Channel Blur",
            vec![
                keyed(
                    ae,
                    1,
                    "Red Blurriness",
                    &[(0.0, 0.0, 0.0), (2.0, 40.0, 20.0)],
                ),
                num(ae, 2, "Green Blurriness", 10.0),
                num(ae, 3, "Blue Blurriness", 20.0),
                num(ae, 4, "Alpha Blurriness", 5.0),
                num(ae, 5, "Edge Behavior", 0.0),
                num(ae, 6, "Blur Dimensions", 2.0),
            ],
        ));
        assert_eq!(keys_of(&r, "red")[1], (2.0, 40.0, 20.0));
        assert_eq!(f(&r, "green"), 10.0);
        assert_eq!(f(&r, "blue"), 20.0);
        assert_eq!(f(&r, "alpha"), 5.0);
        assert!(!boolean(&r, "repeat_edge_pixels"));
        assert!(!rebased(&r, "Red Blurriness") && !rebased(&r, "Alpha Blurriness"));
        assert!(dropped(&r, "Blur Dimensions"));
    }

    #[test]
    fn linear_wipe_is_direct_and_leaves_lumits_own_centre_alone() {
        let ae = "ADBE Linear Wipe";
        let r = run(&fx(
            ae,
            "Linear Wipe",
            vec![
                keyed(
                    ae,
                    1,
                    "Transition Completion",
                    &[(0.0, 0.0, 0.0), (2.0, 100.0, 50.0)],
                ),
                num(ae, 2, "Wipe Angle", 135.0),
                num(ae, 3, "Feather", 24.0),
            ],
        ));
        assert_eq!(
            keys_of(&r, "completion"),
            vec![(0.0, 0.0, 0.0), (2.0, 100.0, 50.0)]
        );
        assert_eq!(f(&r, "angle"), 135.0);
        assert_eq!(f(&r, "feather"), 24.0);
        // Lumit's own Wipe centre defaults to the frame centre, which is AE's
        // only behaviour, so nothing changes.
        assert_eq!((f(&r, "centre_x"), f(&r, "centre_y")), (W / 2.0, H / 2.0));
    }

    #[test]
    fn radial_wipe_maps_all_three_directions() {
        let ae = "ADBE Radial Wipe";
        let r = run(&fx(
            ae,
            "Radial Wipe",
            vec![
                num(ae, 1, "Transition Completion", 30.0),
                num(ae, 2, "Start Angle", 45.0),
                point(ae, 3, "Wipe Center", 200.0, 100.0),
                num(ae, 4, "Wipe", 2.0),
                keyed(ae, 5, "Feather", &[(0.0, 0.0, 0.0), (2.0, 30.0, 15.0)]),
            ],
        ));
        assert_eq!(f(&r, "completion"), 30.0);
        assert_eq!(f(&r, "start_angle"), 45.0);
        assert_eq!((f(&r, "centre_x"), f(&r, "centre_y")), (200.0, 100.0));
        assert_eq!(
            choice(&r, "wipe"),
            1,
            "AE's Counterclockwise is Anticlockwise"
        );
        assert_eq!(keys_of(&r, "feather")[1], (2.0, 30.0, 15.0));
    }

    #[test]
    fn iris_wipe_rebases_both_radii() {
        let ae = "ADBE IRIS_WIPE";
        let r = run(&fx(
            ae,
            "Iris Wipe",
            vec![
                point(ae, 1, "Iris Center", 300.0, 200.0),
                num(ae, 2, "Iris Points", 12.0),
                keyed(
                    ae,
                    3,
                    "Outer Radius",
                    &[(0.0, 0.0, 0.0), (2.0, 400.0, 200.0)],
                ),
                num(ae, 4, "Use Inner Radius", 1.0),
                num(ae, 5, "Inner Radius", 100.0),
                num(ae, 6, "Rotation", 30.0),
                num(ae, 7, "Feather", 8.0),
            ],
        ));
        assert_eq!((f(&r, "centre_x"), f(&r, "centre_y")), (300.0, 200.0));
        assert_eq!(f(&r, "points"), 12.0);
        assert_eq!(keys_of(&r, "outer_radius")[1], (2.0, 400.0, 200.0));
        assert!(boolean(&r, "use_inner_radius"));
        assert_eq!(f(&r, "inner_radius"), 100.0);
        assert_eq!(f(&r, "rotation"), 30.0);
        assert_eq!(f(&r, "feather"), 8.0, "a feather is px@comp on both sides");
        assert!(!rebased(&r, "Outer Radius") && !rebased(&r, "Inner Radius"));
    }

    #[test]
    fn venetian_blinds_carries_its_width_as_a_length() {
        let ae = "ADBE Venetian Blinds";
        let r = run(&fx(
            ae,
            "Venetian Blinds",
            vec![
                keyed(
                    ae,
                    1,
                    "Transition Completion",
                    &[(0.0, 0.0, 0.0), (2.0, 80.0, 40.0)],
                ),
                num(ae, 2, "Direction", 90.0),
                num(ae, 3, "Width", 35.0),
                num(ae, 4, "Feather", 4.0),
            ],
        ));
        assert_eq!(keys_of(&r, "completion")[1], (2.0, 80.0, 40.0));
        assert_eq!(f(&r, "direction"), 90.0);
        assert_eq!(f(&r, "width"), 35.0);
        assert_eq!(f(&r, "feather"), 4.0);
    }

    #[test]
    fn card_wipe_maps_the_grid_and_reports_the_camera() {
        let ae = "APC CardWipeCam";
        let r = run(&fx(
            ae,
            "Card Wipe",
            vec![
                keyed(
                    ae,
                    2,
                    "Transition Completion",
                    &[(0.0, 0.0, 0.0), (2.0, 100.0, 50.0)],
                ),
                num(ae, 4, "Transition Width", 30.0),
                layer_ref(ae, 6, "Back Layer", 2),
                num(ae, 8, "Rows & Columns", 1.0),
                num(ae, 10, "Rows", 4.0),
                num(ae, 12, "Columns", 9.0),
                num(ae, 14, "Card Scale", 0.8),
                num(ae, 16, "Flip Axis", 2.0),
                num(ae, 18, "Flip Direction", 2.0),
                num(ae, 20, "Flip Order", 3.0),
                num(ae, 24, "Timing Randomness", 40.0),
                num(ae, 26, "Random Seed", 9.0),
                num(ae, 28, "Camera System", 1.0),
            ],
        ));
        assert_eq!(keys_of(&r, "completion")[1], (2.0, 100.0, 50.0));
        // Flip Order 3 is Top to bottom, so AE's 30 % of the frame is 30 % of
        // its *height* in px@comp (K-558).
        assert_eq!(f(&r, "transition_width"), 0.30 * H);
        assert_eq!((f(&r, "rows"), f(&r, "columns")), (4.0, 9.0));
        assert_eq!(choice(&r, "flip_axis"), 1);
        assert_eq!(choice(&r, "flip_direction"), 1);
        assert_eq!(choice(&r, "flip_order"), 2);
        assert_eq!(f(&r, "randomness"), 40.0);
        assert_eq!(seed_of(&r, "seed"), 9);
        assert!(dropped(&r, "Back Layer") && dropped(&r, "Card Scale"));
        assert!(dropped(&r, "camera system"));
    }

    /// AE's "Columns Follows Rows" is a tie, not a second number.
    #[test]
    fn card_wipes_tied_columns_follow_the_rows() {
        let ae = "APC CardWipeCam";
        let r = run(&fx(
            ae,
            "Card Wipe",
            vec![
                num(ae, 8, "Rows & Columns", 2.0),
                num(ae, 10, "Rows", 5.0),
                num(ae, 12, "Columns", 12.0),
            ],
        ));
        assert_eq!((f(&r, "rows"), f(&r, "columns")), (5.0, 5.0));
    }

    /// Flip Order's Gradient has no counterpart: it is reported, and the
    /// gradient layer with it, rather than approximated from a spread the
    /// capture does not carry.
    #[test]
    fn card_wipes_gradient_order_is_reported() {
        let ae = "APC CardWipeCam";
        let r = run(&fx(
            ae,
            "Card Wipe",
            vec![
                num(ae, 20, "Flip Order", 5.0),
                layer_ref(ae, 22, "Gradient Layer", 2),
            ],
        ));
        assert!(approximated(&r, "Flip Order"));
        assert!(dropped(&r, "Gradient Layer"));
        assert_eq!(
            choice(&r, "flip_order"),
            0,
            "Left to right, Lumit's default"
        );
    }

    #[test]
    fn corner_pin_carries_the_four_points() {
        let ae = "ADBE Corner Pin";
        let r = run(&fx(
            ae,
            "Corner Pin",
            vec![
                point(ae, 1, "Upper Left", 10.0, 20.0),
                point(ae, 2, "Upper Right", 1900.0, 40.0),
                point(ae, 3, "Lower Left", 30.0, 1000.0),
                keyed(
                    ae,
                    4,
                    "Lower Right",
                    &[(0.0, 1800.0, 0.0), (2.0, 1500.0, -150.0)],
                ),
            ],
        ));
        assert_eq!((f(&r, "upper_left_x"), f(&r, "upper_left_y")), (10.0, 20.0));
        assert_eq!(
            (f(&r, "upper_right_x"), f(&r, "upper_right_y")),
            (1900.0, 40.0)
        );
        assert_eq!(
            (f(&r, "lower_left_x"), f(&r, "lower_left_y")),
            (30.0, 1000.0)
        );
        assert_eq!(keys_of(&r, "lower_right_x")[1], (2.0, 1500.0, -150.0));
        // Lumit's Edges control is its own and defaults to Transparent, which
        // is AE's only behaviour.
        assert_eq!(choice(&r, "edge"), 0);
    }

    #[test]
    fn displacement_map_takes_the_matte_row_as_the_map() {
        let ae = "ADBE Displacement Map";
        let r = run(&fx(
            ae,
            "Displacement Map",
            vec![
                layer_ref(ae, 1, "Displacement Map Layer", 1),
                num(ae, 2, "Use For Horizontal Displacement", 5.0),
                keyed(
                    ae,
                    3,
                    "Max Horizontal Displacement",
                    &[(0.0, 0.0, 0.0), (2.0, 80.0, 40.0)],
                ),
                num(ae, 4, "Use For Vertical Displacement", 4.0),
                num(ae, 5, "Max Vertical Displacement", -30.0),
                num(ae, 6, "Displacement Map Behavior", 2.0),
                num(ae, 8, "Expand Output", 1.0),
            ],
        ));
        assert_eq!(layer_of(&r, "matte"), Some(layer_id(1)));
        assert_eq!(choice(&r, "horizontal_channel"), 0, "AE's Luminance");
        assert_eq!(choice(&r, "vertical_channel"), 1, "AE's Alpha");
        assert_eq!(keys_of(&r, "horizontal_amount")[1], (2.0, 80.0, 40.0));
        assert_eq!(f(&r, "vertical_amount"), -30.0);
        assert!(dropped(&r, "Displacement Map Behavior"));
        assert!(dropped(&r, "Expand Output"));
    }

    /// AE's "Off" means "do not displace on this axis", which is an Amount of 0.
    #[test]
    fn displacement_maps_off_channel_becomes_an_amount_of_zero() {
        let ae = "ADBE Displacement Map";
        let r = run(&fx(
            ae,
            "Displacement Map",
            vec![
                num(ae, 4, "Use For Vertical Displacement", 10.0),
                num(ae, 5, "Max Vertical Displacement", 60.0),
            ],
        ));
        assert_eq!(f(&r, "vertical_amount"), 0.0);
        assert!(approximated(&r, "Use For Vertical Displacement"));
    }

    #[test]
    fn polar_coordinates_converts_both_types_and_the_interpolation() {
        let ae = "ADBE Polar Coordinates";
        let r = run(&fx(
            ae,
            "Polar Coordinates",
            vec![
                keyed(
                    ae,
                    1,
                    "Interpolation",
                    &[(0.0, 0.0, 0.0), (2.0, 100.0, 50.0)],
                ),
                num(ae, 2, "Type of Conversion", 2.0),
            ],
        ));
        assert_eq!(keys_of(&r, "interpolation")[1], (2.0, 100.0, 50.0));
        assert_eq!(choice(&r, "conversion"), 1, "Polar to rectangular");
    }

    /// AE's Twirl Radius is a per cent of the layer — its 100 is the circle
    /// that just contains it — where Lumit's is px@comp.
    #[test]
    fn twirl_rebases_its_radius_into_pixels() {
        let ae = "ADBE Twirl";
        let r = run(&fx(
            ae,
            "Twirl",
            vec![
                num(ae, 1, "Angle", 180.0),
                keyed(
                    ae,
                    2,
                    "Twirl Radius",
                    &[(0.0, 10.0, 0.0), (2.0, 60.0, 25.0)],
                ),
                point(ae, 3, "Twirl Center", 400.0, 500.0),
            ],
        ));
        let pct = |layer_pct: f64| layer_pct * (diag() / 2.0) / 100.0;
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert_eq!(f(&r, "angle"), 180.0);
        let keys = keys_of(&r, "radius");
        assert!(near(keys[0].1, pct(10.0)) && keys[0].2 == 0.0);
        assert!(keys[1].0 == 2.0 && near(keys[1].1, pct(60.0)) && near(keys[1].2, pct(25.0)));
        assert_eq!((f(&r, "centre_x"), f(&r, "centre_y")), (400.0, 500.0));
        assert!(rebased(&r, "Twirl Radius"));
    }

    /// AE's one signed Radius is two Lumit controls: a size and a direction.
    #[test]
    fn spherize_splits_the_signed_radius_into_a_size_and_a_bulge() {
        let ae = "ADBE Spherize";
        let bulging = run(&fx(
            ae,
            "Spherize",
            vec![
                num(ae, 1, "Radius", 300.0),
                point(ae, 2, "Center of Sphere", 100.0, 200.0),
            ],
        ));
        assert_eq!(f(&bulging, "radius"), 300.0);
        assert_eq!(f(&bulging, "bulge"), 100.0);
        assert_eq!(
            (f(&bulging, "centre_x"), f(&bulging, "centre_y")),
            (100.0, 200.0)
        );

        // A negative AE radius is a pinch of the same size.
        let pinching = run(&fx(
            ae,
            "Spherize",
            vec![keyed(
                ae,
                1,
                "Radius",
                &[(0.0, -100.0, 0.0), (2.0, -400.0, -150.0)],
            )],
        ));
        assert_eq!(keys_of(&pinching, "radius")[1], (2.0, 400.0, 150.0));
        assert_eq!(f(&pinching, "bulge"), -100.0);
        assert!(rebased(&pinching, "Radius"));
    }

    /// Wave Speed reads the clock, which docs/08 §2.4 forbids: it becomes two
    /// Evolution keyframes of 360·speed degrees a second across the layer.
    #[test]
    fn ripple_rebases_its_three_lengths_and_turns_wave_speed_into_keyframes() {
        let ae = "ADBE Ripple";
        let r = run(&fx(
            ae,
            "Ripple",
            vec![
                num(ae, 1, "Radius", 50.0),
                point(ae, 2, "Center of Ripple", 700.0, 400.0),
                num(ae, 3, "Type of Conversion", 2.0),
                num(ae, 4, "Wave Speed", 1.5),
                keyed(ae, 5, "Wave Width", &[(0.0, 20.0, 0.0), (2.0, 60.0, 20.0)]),
                num(ae, 6, "Wave Height", 40.0),
                num(ae, 7, "Ripple Phase", 90.0),
            ],
        ));
        assert_eq!(f(&r, "radius"), 50.0 * (diag() / 2.0) / 100.0);
        assert_eq!((f(&r, "centre_x"), f(&r, "centre_y")), (700.0, 400.0));
        assert_eq!(choice(&r, "wave_type"), 0, "AE's second entry is Symmetric");
        assert_eq!(keys_of(&r, "wave_width")[1], (2.0, 60.0, 20.0));
        assert_eq!(f(&r, "wave_height"), 40.0);
        // Two seconds of layer at one and a half turns a second, from the phase
        // After Effects was sitting at.
        assert_eq!(
            keys_of(&r, "evolution"),
            vec![(0.0, 90.0, 0.0), (2.0, 90.0 + 360.0 * 1.5 * 2.0, 0.0)]
        );
        assert!(r
            .report
            .rows
            .iter()
            .any(|row| matches!(&row.reason, Reason::EffectSpeedAsKeyframes { .. })));
    }

    /// A phase with keyframes of its own already says where the wave is, so the
    /// speed is reported rather than fighting them.
    #[test]
    fn ripples_keyed_phase_wins_over_its_wave_speed() {
        let ae = "ADBE Ripple";
        let r = run(&fx(
            ae,
            "Ripple",
            vec![
                num(ae, 4, "Wave Speed", 2.0),
                keyed(
                    ae,
                    7,
                    "Ripple Phase",
                    &[(0.0, 0.0, 0.0), (2.0, 180.0, 90.0)],
                ),
            ],
        ));
        assert_eq!(
            keys_of(&r, "evolution"),
            vec![(0.0, 0.0, 0.0), (2.0, 180.0, 90.0)]
        );
        assert!(dropped(&r, "Wave Speed"));
    }

    #[test]
    fn wave_warp_carries_all_eight_pinnings_and_reports_the_noise_waves() {
        let ae = "ADBE Wave Warp";
        let r = run(&fx(
            ae,
            "Wave Warp",
            vec![
                num(ae, 1, "Wave Type", 4.0),
                keyed(ae, 2, "Wave Height", &[(0.0, 10.0, 0.0), (2.0, 90.0, 40.0)]),
                num(ae, 3, "Wave Width", 200.0),
                num(ae, 4, "Direction", 45.0),
                num(ae, 5, "Wave Speed", 0.0),
                num(ae, 6, "Pinning", 7.0),
                num(ae, 7, "Phase", 30.0),
                num(ae, 8, "Antialiasing (Best Quality)", 1.0),
            ],
        ));
        assert_eq!(choice(&r, "wave_type"), 3, "Sawtooth");
        assert_eq!(keys_of(&r, "wave_height")[1], (2.0, 90.0, 40.0));
        assert_eq!(f(&r, "wave_width"), 200.0);
        assert_eq!(f(&r, "direction"), 45.0);
        assert_eq!(choice(&r, "pinning"), 6, "Top edge");
        assert_eq!(f(&r, "phase"), 30.0);
        assert!(dropped(&r, "Antialiasing (Best Quality)"));

        let noisy = run(&fx(ae, "Wave Warp", vec![num(ae, 1, "Wave Type", 8.0)]));
        assert!(approximated(&noisy, "Wave Type"));
        assert_eq!(choice(&noisy, "wave_type"), 0, "Sine, Lumit's default");
    }

    #[test]
    fn bezier_warp_walks_ae_s_twelve_points_clockwise_from_the_upper_left() {
        let ae = "ADBE BEZMESH";
        let r = run(&fx(
            ae,
            "Bezier Warp",
            vec![
                point(ae, 1, "Top Left Vertex", 1.0, 2.0),
                point(ae, 2, "Top Left Tangent", 3.0, 4.0),
                point(ae, 3, "Top Right Tangent", 5.0, 6.0),
                point(ae, 4, "Right Top Vertex", 7.0, 8.0),
                point(ae, 5, "Right Top Tangent", 9.0, 10.0),
                point(ae, 6, "Right Bottom Tangent", 11.0, 12.0),
                point(ae, 7, "Bottom Right Vertex", 13.0, 14.0),
                point(ae, 8, "Bottom Right Tangent", 15.0, 16.0),
                point(ae, 9, "Bottom Left Tangent", 17.0, 18.0),
                point(ae, 10, "Left Bottom Vertex", 19.0, 20.0),
                point(ae, 11, "Left Bottom Tangent", 21.0, 22.0),
                point(ae, 12, "Left Top Tangent", 23.0, 24.0),
                keyed(ae, 14, "Quality", &[(0.0, 2.0, 0.0), (2.0, 12.0, 5.0)]),
            ],
        ));
        assert_eq!((f(&r, "upper_left_x"), f(&r, "upper_left_y")), (1.0, 2.0));
        assert_eq!(
            (f(&r, "top_left_tangent_x"), f(&r, "top_left_tangent_y")),
            (3.0, 4.0)
        );
        assert_eq!((f(&r, "upper_right_x"), f(&r, "upper_right_y")), (7.0, 8.0));
        assert_eq!(
            (
                f(&r, "right_bottom_tangent_x"),
                f(&r, "right_bottom_tangent_y")
            ),
            (11.0, 12.0)
        );
        assert_eq!(
            (f(&r, "lower_right_x"), f(&r, "lower_right_y")),
            (13.0, 14.0)
        );
        assert_eq!((f(&r, "lower_left_x"), f(&r, "lower_left_y")), (19.0, 20.0));
        assert_eq!(
            (f(&r, "left_top_tangent_x"), f(&r, "left_top_tangent_y")),
            (23.0, 24.0)
        );
        assert_eq!(keys_of(&r, "quality")[1], (2.0, 12.0, 5.0));
        assert!(differs(&r), "Quality means something else on each side");
    }

    #[test]
    fn warp_maps_thirteen_styles_and_reports_the_two_shells() {
        let ae = "ADBE WRPMESH";
        // AE's second entry is Arc Lower, which is Lumit's third.
        let arc_lower = run(&fx(
            ae,
            "Warp",
            vec![
                num(ae, 1, "Warp Style", 2.0),
                num(ae, 2, "Warp Axis", 2.0),
                keyed(ae, 3, "Bend", &[(0.0, 0.0, 0.0), (2.0, -80.0, -40.0)]),
                num(ae, 4, "Horizontal Distortion", 25.0),
                num(ae, 5, "Vertical Distortion", -15.0),
            ],
        ));
        assert_eq!(choice(&arc_lower, "style"), 2);
        assert_eq!(keys_of(&arc_lower, "bend")[1], (2.0, -80.0, -40.0));
        assert_eq!(f(&arc_lower, "horizontal_distortion"), 25.0);
        assert_eq!(f(&arc_lower, "vertical_distortion"), -15.0);
        assert!(dropped(&arc_lower, "Warp Axis"));
        assert!(differs(&arc_lower));

        let shell = run(&fx(ae, "Warp", vec![num(ae, 1, "Warp Style", 6.0)]));
        assert!(approximated(&shell, "Warp Style"));
        assert_eq!(choice(&shell, "style"), 0, "Arc, Lumit's default");
    }

    /// AE's seven edge types are three shapes and a colour switch.
    #[test]
    fn roughen_edges_splits_the_edge_type_and_rebases_its_two_factors() {
        let ae = "ADBE Roughen Edges";
        let r = run(&fx(
            ae,
            "Roughen Edges",
            vec![
                num(ae, 1, "Edge Type", 6.0),
                colour(ae, 10, "Edge Color", [0.0, 1.0, 0.0, 1.0]),
                keyed(ae, 2, "Border", &[(0.0, 8.0, 0.0), (2.0, 120.0, 56.0)]),
                num(ae, 3, "Edge Sharpness", 0.7),
                num(ae, 4, "Fractal Influence", 1.5),
                num(ae, 5, "Scale", 250.0),
                num(ae, 6, "Stretch Width or Height", 0.5),
                point(ae, 7, "Offset (Turbulence)", 40.0, 50.0),
                num(ae, 8, "Complexity", 5.0),
                num(ae, 9, "Evolution", 360.0),
                num(ae, 12, "Cycle Evolution", 1.0),
                num(ae, 13, "Cycle (in Revolutions)", 2.0),
                num(ae, 14, "Random Seed", 4.0),
            ],
        ));
        assert_eq!(choice(&r, "edge_type"), 2, "Spiky");
        assert!(boolean(&r, "colour_edge"), "AE's Spiky Color");
        assert_eq!(keys_of(&r, "border")[1], (2.0, 120.0, 56.0));
        assert_eq!(f(&r, "edge_sharpness"), 70.0);
        assert_eq!(f(&r, "fractal_influence"), 150.0);
        assert_eq!(f(&r, "scale"), 250.0);
        assert_eq!((f(&r, "offset_x"), f(&r, "offset_y")), (40.0, 50.0));
        assert_eq!(f(&r, "complexity"), 5.0);
        assert_eq!(f(&r, "evolution"), 360.0);
        assert!(boolean(&r, "cycle_evolution"));
        assert_eq!(f(&r, "cycle"), 2.0);
        assert_eq!(seed_of(&r, "seed"), 4);
        assert!(rebased(&r, "Edge Sharpness") && rebased(&r, "Fractal Influence"));
        assert!(dropped(&r, "Stretch Width or Height"));

        // Photocopy is Cut with the colour edge on, and says so.
        let photocopy = run(&fx(ae, "Roughen Edges", vec![num(ae, 1, "Edge Type", 7.0)]));
        assert_eq!(choice(&photocopy, "edge_type"), 1);
        assert!(boolean(&photocopy, "colour_edge"));
        assert!(approximated(&photocopy, "Edge Type"));
    }

    /// The one conversion in the table limited by a budget rather than by a
    /// semantic: Lumit's median radius caps at 3 where AE's runs to 50.
    #[test]
    fn median_caps_the_radius_and_says_so() {
        let ae = "ADBE Median";
        let r = run(&fx(
            ae,
            "Median",
            vec![
                keyed(ae, 1, "Radius", &[(0.0, 1.0, 0.0), (2.0, 12.0, 5.0)]),
                num(ae, 2, "Operate On Alpha Channel", 1.0),
            ],
        ));
        assert_eq!(keys_of(&r, "radius")[0], (0.0, 1.0, 0.0));
        assert_eq!(keys_of(&r, "radius")[1].1, 3.0);
        assert!(boolean(&r, "alpha"));
        assert!(approximated(&r, "Radius"));

        let inside = run(&fx(ae, "Median", vec![num(ae, 1, "Radius", 2.0)]));
        assert_eq!(f(&inside, "radius"), 2.0);
        assert!(!approximated(&inside, "Radius"));
    }

    #[test]
    fn mosaic_carries_both_block_counts() {
        let ae = "ADBE Mosaic";
        let r = run(&fx(
            ae,
            "Mosaic",
            vec![
                keyed(
                    ae,
                    1,
                    "Horizontal Blocks",
                    &[(0.0, 10.0, 0.0), (2.0, 80.0, 35.0)],
                ),
                num(ae, 2, "Vertical Blocks", 45.0),
                num(ae, 3, "Sharp Colors", 1.0),
            ],
        ));
        assert_eq!(keys_of(&r, "horizontal_blocks")[1], (2.0, 80.0, 35.0));
        assert_eq!(f(&r, "vertical_blocks"), 45.0);
        assert!(boolean(&r, "sharp_colours"));
        assert!(differs(&r));
    }

    #[test]
    fn find_edges_reads_blend_with_original_from_the_other_end() {
        let ae = "ADBE Find Edges";
        let r = run(&fx(
            ae,
            "Find Edges",
            vec![
                num(ae, 1, "Invert", 1.0),
                keyed(
                    ae,
                    2,
                    "Blend With Original",
                    &[(0.0, 0.0, 0.0), (2.0, 40.0, 20.0)],
                ),
            ],
        ));
        assert!(boolean(&r, "invert"));
        assert_eq!(
            keys_of(&r, "mix"),
            vec![(0.0, 100.0, 0.0), (2.0, 60.0, -20.0)]
        );
        assert!(differs(&r));
    }

    #[test]
    fn emboss_carries_its_relief_as_a_length() {
        let ae = "ADBE Emboss";
        let r = run(&fx(
            ae,
            "Emboss",
            vec![
                num(ae, 1, "Direction", 225.0),
                keyed(ae, 2, "Relief", &[(0.0, 1.0, 0.0), (2.0, 6.0, 2.5)]),
                num(ae, 3, "Contrast", 150.0),
                num(ae, 4, "Blend With Original", 20.0),
            ],
        ));
        assert_eq!(f(&r, "direction"), 225.0);
        assert_eq!(keys_of(&r, "relief")[1], (2.0, 6.0, 2.5));
        assert_eq!(f(&r, "contrast"), 150.0);
        assert_eq!(f(&r, "mix"), 80.0);
        assert!(differs(&r));
    }

    #[test]
    fn texturize_takes_its_own_texture_row_and_reports_the_native_size() {
        let ae = "ADBE Texturize";
        let r = run(&fx(
            ae,
            "Texturize",
            vec![
                layer_ref(ae, 1, "Texture Layer", 2),
                keyed(
                    ae,
                    2,
                    "Light Direction",
                    &[(0.0, 0.0, 0.0), (2.0, 180.0, 90.0)],
                ),
                num(ae, 3, "Texture Contrast", 1.75),
                num(ae, 4, "Texture Placement", 2.0),
            ],
        ));
        assert_eq!(layer_of(&r, "texture"), Some(layer_id(2)));
        assert_eq!(keys_of(&r, "light_direction")[1], (2.0, 180.0, 90.0));
        assert_eq!(f(&r, "texture_contrast"), 175.0);
        assert_eq!(choice(&r, "placement"), 2, "AE's Center Texture");
        assert!(rebased(&r, "Texture Contrast"));
        assert!(approximated(&r, "Texture Placement"));

        // Stretch Texture to Fit at Scale 100 is exactly Lumit's Stretch, and
        // nothing is approximated.
        let stretched = run(&fx(
            ae,
            "Texturize",
            vec![num(ae, 4, "Texture Placement", 3.0)],
        ));
        assert_eq!(choice(&stretched, "placement"), 0);
        assert_eq!(f(&stretched, "scale"), 100.0);
        assert!(!approximated(&stretched, "Texture Placement"));
    }

    /// The five Expression Controls carry their one property, keyframes and
    /// all, and report nothing — there is nothing to convert (K-414). The
    /// keyframed slider is the case that matters: a CC-pack rig is an animated
    /// Slider Control and a page of expressions reading it.
    #[test]
    fn the_expression_controls_carry_their_one_property() {
        let r = run(&fx(
            "ADBE Slider Control",
            "Slider Control",
            vec![keyed(
                "ADBE Slider Control",
                1,
                "Slider",
                &[(0.0, 0.0, 0.0), (2.0, 240.0, 120.0)],
            )],
        ));
        assert_eq!(keys_of(&r, "slider")[1], (2.0, 240.0, 120.0));
        assert!(r.report.rows.is_empty(), "nothing to report");

        let r = run(&fx(
            "ADBE Angle Control",
            "Angle Control",
            vec![num("ADBE Angle Control", 1, "Angle", 450.0)],
        ));
        assert_eq!(f(&r, "angle"), 450.0, "an angle winds past a full turn");

        let r = run(&fx(
            "ADBE Checkbox Control",
            "Checkbox Control",
            vec![num("ADBE Checkbox Control", 1, "Checkbox", 1.0)],
        ));
        assert!(boolean(&r, "checkbox"));

        let r = run(&fx(
            "ADBE Color Control",
            "Color Control",
            vec![colour(
                "ADBE Color Control",
                1,
                "Color",
                [0.5, 0.5, 0.5, 1.0],
            )],
        ));
        match r.inst.param("colour") {
            Some(EffectValue::Colour(c)) => {
                let red = c[0].value_at(0.0);
                assert!(
                    (red - f64::from(srgb_to_linear(0.5))).abs() < 1e-9,
                    "the swatch crosses into scene-linear light like every other                      imported colour: {red}"
                );
            }
            other => panic!("not a colour: {other:?}"),
        }

        let r = run(&fx(
            "ADBE Point Control",
            "Point Control",
            vec![point("ADBE Point Control", 1, "Point", 640.0, 360.0)],
        ));
        assert_eq!((f(&r, "point_x"), f(&r, "point_y")), (640.0, 360.0));
    }

    /// A conversion cannot rewrite an expression, so a rebased parameter with
    /// one on it is reported rather than quietly wrong.
    #[test]
    fn an_expression_under_a_rebased_parameter_is_reported() {
        let ae = "ADBE Twirl";
        let mut node = num(ae, 2, "Twirl Radius", 0.0);
        node.expression = Some("time * 10".to_string());
        node.expression_enabled = Some(true);
        let r = run(&fx(ae, "Twirl", vec![node]));
        assert!(approximated(&r, "Twirl Radius"));
    }
}
