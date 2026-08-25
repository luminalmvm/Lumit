//! The funnel tables: one per enum, turning the `.aep`'s numeric codes into the
//! **ExtendScript constant names** the capture schema already speaks.
//!
//! In plain terms: the Lumit Bridge writes down what After Effects *called*
//! something — `SCREEN`, `BEZIER`, `ALPHA_INVERTED`. The project file itself
//! stores small numbers instead. So that both routes produce one capture and
//! everything downstream stays unchanged, every number is translated here, in
//! one table per enum, and nowhere else — docs/impl/ae-import.md §7's funnel
//! rule, which is binding. A code no table knows falls through as the number
//! written out as text, which is honest rather than wrong: the reader already
//! copes with an unfamiliar name and the report can say so.
//!
//! Every table below is marked with how its entries were established.
//! **fixture-proven** means the golden `.aep` in `tools/ae-bridge/fixtures/`
//! contains that code and `tests/aep_differential.rs` asserts the name against
//! what After Effects itself said about the same file. **reference** means the
//! entry comes from `forticheprod/aep_parser` (MIT, read as documentation) and
//! is not exercised by the fixture — those are the ones a second fixture is
//! owed for.

/// Look a code up, or write the number out when nothing knows it.
fn name(table: &[(u32, &'static str)], code: u32) -> String {
    table
        .iter()
        .find(|(value, _)| *value == code)
        .map_or_else(|| code.to_string(), |(_, name)| (*name).to_string())
}

/// `idta`'s item type. All three are fixture-proven.
pub const ITEM_FOLDER: u16 = 1;
/// A composition item.
pub const ITEM_COMP: u16 = 4;
/// A footage item — which is also how a solid is stored.
pub const ITEM_FOOTAGE: u16 = 7;

/// `ldta`'s layer type. 0–4 are fixture-proven; 5 and 7 are reference, and have
/// no capture vocabulary of their own, so they fall through as numbers.
const LAYER_KIND: &[(u32, &str)] = &[
    (0, "footage"),
    (1, "light"),
    (2, "camera"),
    (3, "text"),
    (4, "shape"),
];

/// The layer kind, before the switches get their say — a null and an adjustment
/// layer are both plain AV layers here, and are re-labelled by the caller.
pub fn layer_kind(code: u32) -> String {
    name(LAYER_KIND, code)
}

/// `ldta` byte 99, AE's transfer mode. The numbers are the SDK's `PF_Xfer`
/// values, so the run is dense and unambiguous; the fixture proves `NORMAL`
/// (code 2), `DISSOLVE`, `MULTIPLY`, `SCREEN` and `OVERLAY`, and the rest are
/// reference.
///
/// The run does not stop at 24. The modes After Effects added after version 4
/// continue from 25 to 38, and the three the older codes name `CLASSIC_` have
/// their modern counterparts up there — `DIFFERENCE` at 26 beside
/// `CLASSIC_DIFFERENCE` at 12, and the same for the two burns and dodges. A
/// table that ended at 24 answered "unknown" for Difference, Vivid Light,
/// Lighter Color and Subtract, all four of which a real project uses, and every
/// one of them imported as Normal with a report row.
///
/// Code 0 is **reference, not proven**: in the fixture only the camera and the
/// light carry it, and a rig has no blend mode for the capture to report, so
/// nothing sends a 0 through this table. It is mapped to `NORMAL` because that
/// is what the reference implementation reads it as; a project that does put a
/// 0 on an ordinary layer would settle it.
const BLEND: &[(u32, &str)] = &[
    (0, "NORMAL"),
    (2, "NORMAL"),
    (3, "DISSOLVE"),
    (4, "ADD"),
    (5, "MULTIPLY"),
    (6, "SCREEN"),
    (7, "OVERLAY"),
    (8, "SOFT_LIGHT"),
    (9, "HARD_LIGHT"),
    (10, "DARKEN"),
    (11, "LIGHTEN"),
    (12, "CLASSIC_DIFFERENCE"),
    (13, "HUE"),
    (14, "SATURATION"),
    (15, "COLOR"),
    (16, "LUMINOSITY"),
    (17, "STENCIL_ALPHA"),
    (18, "STENCIL_LUMA"),
    (19, "SILHOUETE_ALPHA"),
    (20, "SILHOUETTE_LUMA"),
    (21, "LUMINESCENT_PREMUL"),
    (22, "ALPHA_ADD"),
    (23, "CLASSIC_COLOR_DODGE"),
    (24, "CLASSIC_COLOR_BURN"),
    (25, "EXCLUSION"),
    (26, "DIFFERENCE"),
    (27, "COLOR_DODGE"),
    (28, "COLOR_BURN"),
    (29, "LINEAR_DODGE"),
    (30, "LINEAR_BURN"),
    (31, "LINEAR_LIGHT"),
    (32, "VIVID_LIGHT"),
    (33, "PIN_LIGHT"),
    (34, "HARD_MIX"),
    (35, "LIGHTER_COLOR"),
    (36, "DARKER_COLOR"),
    (37, "SUBTRACT"),
    (38, "DIVIDE"),
];

/// The blend mode. `dancing` is `ldta` byte 103 bit 1: After Effects has no
/// transfer value of its own for Dancing Dissolve, it is Dissolve with a flag
/// (reference; the fixture's Dissolve layer has the flag clear).
pub fn blend(code: u32, dancing: bool) -> String {
    if dancing && code == 3 {
        return "DANCING_DISSOLVE".to_string();
    }
    name(BLEND, code)
}

/// `ldta` bytes 4–5. `DRAFT` and `BEST` are fixture-proven; `WIREFRAME` is
/// reference (the run is contiguous from the ExtendScript constant base).
const QUALITY: &[(u32, &str)] = &[(0, "WIREFRAME"), (1, "DRAFT"), (2, "BEST")];

/// The layer's render quality switch.
pub fn quality(code: u32) -> String {
    name(QUALITY, code)
}

/// `ldta` byte 107. `NO_TRACK_MATTE`, `ALPHA_INVERTED` and `LUMA` are
/// fixture-proven; `ALPHA` and `LUMA_INVERTED` are reference, filling the same
/// contiguous run.
const MATTE: &[(u32, &str)] = &[
    (0, "NO_TRACK_MATTE"),
    (1, "ALPHA"),
    (2, "ALPHA_INVERTED"),
    (3, "LUMA"),
    (4, "LUMA_INVERTED"),
];

/// The matte type.
pub fn matte(code: u32) -> String {
    name(MATTE, code)
}

/// Frame blending, which the file splits over two bits: one says whether it is
/// on at all (`ldta` byte 39 bit 4), the other which kind (byte 37 bit 2). All
/// three outcomes are fixture-proven.
pub fn frame_blending(enabled: bool, pixel_motion: bool) -> String {
    match (enabled, pixel_motion) {
        (false, _) => "NO_FRAME_BLEND",
        (true, false) => "FRAME_MIX",
        (true, true) => "PIXEL_MOTION",
    }
    .to_string()
}

/// Auto-orientation, also assembled from bits rather than a code.
/// `NO_AUTO_ORIENT` and `CAMERA_OR_POINT_OF_INTEREST` are fixture-proven;
/// `ALONG_PATH` and `CHARACTERS_TOWARD_CAMERA` are reference.
pub fn auto_orient(along_path: bool, toward_point: bool, toward_camera: bool) -> String {
    if along_path {
        "ALONG_PATH"
    } else if toward_point {
        "CAMERA_OR_POINT_OF_INTEREST"
    } else if toward_camera {
        "CHARACTERS_TOWARD_CAMERA"
    } else {
        "NO_AUTO_ORIENT"
    }
    .to_string()
}

/// `ldta` byte 139, on light layers. `SPOT` is fixture-proven; the rest are
/// reference, from the same contiguous run.
const LIGHT: &[(u32, &str)] = &[
    (0, "PARALLEL"),
    (1, "SPOT"),
    (2, "POINT"),
    (3, "AMBIENT"),
    (4, "ENVIRONMENT"),
];

/// A light layer's type.
pub fn light_type(code: u32) -> String {
    name(LIGHT, code)
}

/// `mkif` bytes 6–7, the SDK's `PF_MaskMode`. `ADD` and `SUBTRACT` are
/// fixture-proven; the rest are reference, from the same contiguous run.
const MASK_MODE: &[(u32, &str)] = &[
    (0, "NONE"),
    (1, "ADD"),
    (2, "SUBTRACT"),
    (3, "INTERSECT"),
    (4, "LIGHTEN"),
    (5, "DARKEN"),
    (6, "DIFFERENCE"),
];

/// A mask's combine mode.
pub fn mask_mode(code: u32) -> String {
    name(MASK_MODE, code)
}

/// A keyframe's per-side interpolation, out of the byte at `ldat` offsets 4 and
/// 5. All three are fixture-proven — the golden project holds linear keys, an
/// eased pair and a hold.
const INTERPOLATION: &[(u32, &str)] = &[(1, "LINEAR"), (2, "BEZIER"), (3, "HOLD")];

/// One side of one keyframe.
pub fn interpolation(code: u8) -> String {
    name(INTERPOLATION, u32::from(code))
}

/// `nnhd` byte 24 — a colour-depth *exponent*, not a bit count. 16-bit is
/// fixture-proven; 8 and 32 are reference.
pub fn bits_per_channel(code: u8) -> u32 {
    match code {
        0 => 8,
        1 => 16,
        2 => 32,
        // Never seen; say what was there rather than pretending it was 8.
        other => u32::from(other),
    }
}

/// The comp's 3D renderer. The file stores the plug-in's own match name and
/// ExtendScript reports a different one for the same renderer — `ADBE Escher`
/// *is* Classic 3D, which scripting calls `ADBE Advanced 3d`. That swap is
/// fixture-proven; the other three pass through unchanged (reference).
pub fn renderer(match_name: &str) -> String {
    match match_name {
        "ADBE Escher" => "ADBE Advanced 3d".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **An unknown code falls through as its own number.**
    ///
    /// The funnel rule's escape hatch (docs/impl/ae-import.md §7): a blend mode
    /// or matte type from an After Effects newer than these tables must arrive
    /// as something honest rather than as the nearest guess, because the
    /// nearest guess is a silently wrong picture.
    #[test]
    fn a_code_no_table_knows_arrives_as_its_number() {
        assert_eq!(blend(9999, false), "9999");
        assert_eq!(matte(77), "77");
        assert_eq!(quality(5), "5");
        assert_eq!(light_type(60), "60");
        assert_eq!(layer_kind(7), "7");
        assert_eq!(bits_per_channel(9), 9);
        assert_eq!(renderer("ADBE Calder"), "ADBE Calder");
    }

    /// **The modes After Effects added after version 4 are the ones a real
    /// project is full of, and the run reaches them.**
    ///
    /// The three names that appear twice are the point: 12, 23 and 24 are the
    /// 4.x arithmetic, kept apart under `CLASSIC_` because the import flags
    /// them, while 26, 27 and 28 are the modern ones every project since uses.
    /// A table that stopped at 24 sent Difference, Vivid Light, Lighter Color
    /// and Subtract to Normal.
    #[test]
    fn the_modern_blend_modes_are_in_the_table() {
        assert_eq!(blend(25, false), "EXCLUSION");
        assert_eq!(blend(26, false), "DIFFERENCE");
        assert_eq!(blend(27, false), "COLOR_DODGE");
        assert_eq!(blend(28, false), "COLOR_BURN");
        assert_eq!(blend(29, false), "LINEAR_DODGE");
        assert_eq!(blend(30, false), "LINEAR_BURN");
        assert_eq!(blend(31, false), "LINEAR_LIGHT");
        assert_eq!(blend(32, false), "VIVID_LIGHT");
        assert_eq!(blend(33, false), "PIN_LIGHT");
        assert_eq!(blend(34, false), "HARD_MIX");
        assert_eq!(blend(35, false), "LIGHTER_COLOR");
        assert_eq!(blend(36, false), "DARKER_COLOR");
        assert_eq!(blend(37, false), "SUBTRACT");
        assert_eq!(blend(38, false), "DIVIDE");
        // And the 4.x arithmetic keeps its own three codes, which the import
        // flags rather than silently modernising twice over.
        assert_eq!(blend(12, false), "CLASSIC_DIFFERENCE");
        assert_eq!(blend(23, false), "CLASSIC_COLOR_DODGE");
        assert_eq!(blend(24, false), "CLASSIC_COLOR_BURN");
    }

    /// **Dancing Dissolve is Dissolve plus a flag, and only for Dissolve.**
    ///
    /// After Effects has no transfer value of its own for it, so the flag has
    /// to be read beside the code — and must not colour any other mode.
    #[test]
    fn dancing_dissolve_needs_both_the_code_and_the_flag() {
        assert_eq!(blend(3, false), "DISSOLVE");
        assert_eq!(blend(3, true), "DANCING_DISSOLVE");
        assert_eq!(blend(6, true), "SCREEN");
    }
}
