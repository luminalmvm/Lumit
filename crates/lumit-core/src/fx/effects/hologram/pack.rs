//! Hologram's uniform. The rows are Holographic foil's, packed by
//! its own code from a foil that carries the film's shared rows: the sheet,
//! the lights, the studio, the surface and the output. The film's line
//! patterns are a grating, so the foil's grating rows hold theirs. The
//! replay's own rows take the place of the flakes' and the dots', which a
//! hologram never reads, mirrored by the constants at the top of
//! `fx_hologram.wgsl`:
//!
//! | Row | Holds |
//! |---|---|
//! | 280 | type (1 rainbow), subject colour, recording (1 stereogram), backdrop |
//! | 281 | reference direction x, y, z on the film, replay wavelength in nm |
//! | 282 | efficiency, the mirror's share, Bragg band width in nm, selectivity in radians |
//! | 283 | slit width in radians, the slit's distance and height in mm, speckle |
//! | 284 | subject depth, depth scale (below 0 inverted), backdrop depth, pattern depth, all mm |
//! | 285 | views, view blend share, view zone in radians |
//! | 286 | pattern line spacing in mm, line width share, pattern strength, pattern radius in mm |

use super::{Hologram, RAINBOW, STEREOGRAM};
use crate::fx::effects::holographic_foil::{motion::Motion, pack, HolographicFoil, BLOCKS};
use crate::fx::params::Params;
use crate::fx::registry::EffectMetadata;

const TYPE: usize = 280;
const REFERENCE: usize = 281;
const REPLAY: usize = 282;
const SLIT: usize = 283;
const DEPTHS: usize = 284;
const VIEWS: usize = 285;
const PATTERN: usize = 286;

/// Line spacings across a pattern's width.
const LINES_ACROSS: f64 = 48.0;

type Blocks = [[f32; 4]; BLOCKS];

fn put(b: &mut Blocks, row: usize, v: [f64; 4]) {
    if let Some(r) = b.get_mut(row) {
        *r = v.map(|x| x as f32);
    }
}

/// The foil whose packing gives the film's shared rows. Its grating is the
/// film's line patterns: shallow grooves at Pitch, a little fan and no dye.
pub(super) fn foil_of(h: &Hologram) -> HolographicFoil {
    HolographicFoil {
        material: 0,
        tilt_x: h.tilt_x,
        tilt_y: h.tilt_y,
        spin: h.spin,
        auto_tilt: h.auto_tilt,
        auto_tilt_speed: h.auto_tilt_speed,
        follow_rotation: h.follow_rotation,
        motion_blur: h.motion_blur,
        sheet_width: h.sheet_width,
        view_distance: h.view_distance,
        pitch: h.pitch,
        // Turning grooves: the wobble traces an ellipse, and the
        // renormalisation follows each light.
        pattern: 1,
        depth: 110.0,
        coverage: 100.0,
        fan: 0.1_f32.to_degrees(),
        softness: 0.0044_f32.to_degrees(),
        orientation_jitter: 0.3,
        pitch_jitter: 0.5,
        lacquer: 1.5,
        tint: 0,
        lighting: h.lighting,
        rig_rotation: h.rig_rotation,
        spot_a_on: h.spot_a_on,
        spot_a_x: h.spot_a_x,
        spot_a_y: h.spot_a_y,
        spot_a_size: h.spot_a_size,
        spot_a_intensity: h.spot_a_intensity,
        spot_a_distance: h.spot_a_distance,
        spot_a_spectrum: h.spot_a_spectrum,
        spot_b_on: h.spot_b_on,
        spot_b_x: h.spot_b_x,
        spot_b_y: h.spot_b_y,
        spot_b_size: h.spot_b_size,
        spot_b_intensity: h.spot_b_intensity,
        spot_b_distance: h.spot_b_distance,
        spot_b_spectrum: h.spot_b_spectrum,
        spot_c_on: h.spot_c_on,
        spot_c_x: h.spot_c_x,
        spot_c_y: h.spot_c_y,
        spot_c_size: h.spot_c_size,
        spot_c_intensity: h.spot_c_intensity,
        spot_c_distance: h.spot_c_distance,
        spot_c_spectrum: h.spot_c_spectrum,
        panel_on: h.panel_on,
        panel_x: h.panel_x,
        panel_y: h.panel_y,
        panel_width: h.panel_width,
        panel_length: h.panel_length,
        panel_intensity: h.panel_intensity,
        panel_distance: h.panel_distance,
        panel_spectrum: h.panel_spectrum,
        rims: h.rims,
        fill: h.fill,
        scrim: h.scrim,
        room: h.room,
        coat_roughness: h.coat_roughness,
        relief_source: h.relief_source,
        relief: h.relief,
        relief_scale: h.relief_scale,
        relief_on: h.relief_on,
        relief_invert: h.relief_invert,
        bevel_width: 1.5,
        bevel_height: 0.0,
        bevel_profile: 0,
        bevel_softness: 0.0,
        waviness: h.waviness,
        orange_peel: h.orange_peel,
        lines: h.lines,
        line_pitch: h.line_pitch,
        line_height: h.line_height,
        bow: h.bow,
        bow_axis: h.bow_axis,
        wear: h.wear,
        height_map_scale: h.height_map_scale,
        height_map_wrap: h.height_map_wrap,
        environment_amount: h.environment_amount,
        exposure: h.exposure,
        highlights: h.highlights,
        quality: h.quality,
        seed: h.seed,
        mix: h.mix,
        ..<HolographicFoil as EffectMetadata>::read(Params::EMPTY)
    }
}

/// The rows for `h` on a `w` by `hh` raster, posed as `m` says, `foil` being
/// `foil_of(h)`.
pub(super) fn blocks(h: &Hologram, foil: &HolographicFoil, w: u32, hh: u32, m: &Motion) -> Blocks {
    let mut b = pack::blocks(foil, w, hh, m);
    let rainbow = h.hologram_type == RAINBOW;
    put(
        &mut b,
        TYPE,
        [
            f64::from(h.hologram_type.min(1)),
            f64::from(h.subject_colour.min(2)),
            f64::from(u32::from(h.recording == STEREOGRAM)),
            f64::from(h.backdrop.min(8)),
        ],
    );
    // The reference as a direction on the film, kept just inside the rim.
    let (rx, ry) = (f64::from(h.reference_x), f64::from(h.reference_y));
    let len = rx.hypot(ry);
    let k = if len > 0.99 { 0.99 / len } else { 1.0 };
    let (rx, ry) = (rx * k, ry * k);
    let rz = (1.0 - rx * rx - ry * ry).max(0.0).sqrt();
    put(
        &mut b,
        REFERENCE,
        [rx, ry, rz, f64::from(h.colour.clamp(380.0, 780.0))],
    );
    let eff = f64::from(h.efficiency / 100.0).clamp(0.0, 1.0);
    let glare = f64::from(h.glare / 100.0).clamp(0.0, 1.0);
    // An embossed rainbow hologram sits on aluminium; a reflection hologram
    // has a black backing that sends back about 1.5 per cent.
    let mirror = if rainbow {
        (1.0 - 0.3 * eff) * glare
    } else {
        0.017 * glare
    };
    put(
        &mut b,
        REPLAY,
        [
            eff,
            mirror,
            f64::from(h.bandwidth.max(0.5)) / 2.3548,
            f64::from(h.selectivity.max(0.5)).to_radians(),
        ],
    );
    // The slit's image sits on the view axis at the eye, so the default view
    // sees the replay colour.
    let view_mm = f64::from(h.view_distance.max(0.05)) * 1000.0;
    put(
        &mut b,
        SLIT,
        [
            f64::from(h.slit.max(0.1)).to_radians() / 2.3548,
            view_mm,
            0.0,
            f64::from(h.speckle / 100.0).clamp(0.0, 1.0),
        ],
    );
    let scale = f64::from(h.depth_scale.max(0.0));
    put(
        &mut b,
        DEPTHS,
        [
            f64::from(h.subject_depth),
            if h.depth_invert { -scale } else { scale },
            f64::from(h.backdrop_depth.min(0.0)),
            f64::from(h.film_depth.min(0.0)),
        ],
    );
    put(
        &mut b,
        VIEWS,
        [
            f64::from(h.views.max(2)),
            f64::from(h.view_blend / 100.0).clamp(0.0, 1.0),
            f64::from(h.view_zone.clamp(1.0, 170.0)).to_radians(),
            0.0,
        ],
    );
    let size = f64::from(h.pattern_size.max(1.0));
    put(
        &mut b,
        PATTERN,
        [
            size / LINES_ACROSS,
            f64::from(h.line_width / 100.0).clamp(0.01, 1.0),
            f64::from(h.pattern_strength.max(0.0)),
            0.5 * size,
        ],
    );
    b
}
