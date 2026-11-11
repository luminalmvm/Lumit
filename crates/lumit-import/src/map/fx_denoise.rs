//! After Effects' Remove Grain onto Denoise.
//!
//! Remove Grain samples the noise at points of its own and Denoise measures it
//! when Learn is pressed, so the measurement does not come across: what does
//! is whether frames either side are used, how far they are trusted, and how
//! much of the original is blended back.

use lumit_core::anim::Property;
use lumit_core::model::{EffectInstance, EffectValue};

use crate::capture::Property as AeProperty;
use crate::report::ItemPath;

use super::fx_colour::Fx;
use super::props::match_name_of;
use super::Conv;

/// This file's claim on a match name: the row whose conversion is
/// `remove_grain`.
pub(super) fn claim(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &AeProperty,
) -> Option<EffectInstance> {
    let row = super::table::table().row(match_name_of(node))?;
    if row.conversion != "remove_grain" {
        return None;
    }
    let mut fx = Fx::new(path, node, row)?;
    // After Effects filters one frame unless Temporal Filtering is on, and the
    // file only keeps what changed from its defaults.
    fx.set("frames", EffectValue::Float(Property::fixed(0.0)));
    if fx
        .still(conv, "VISINF Grain Removal-0022")
        .is_some_and(|on| on > 0.5)
    {
        fx.set("frames", EffectValue::Float(Property::fixed(2.0)));
    }
    // Both are per cents with 100 as the default.
    fx.float(conv, "VISINF Grain Removal-0014", "temporal", 1.0, 0.0);
    // Blend with Original is Mix read from the other end.
    fx.float(conv, "VISINF Grain Removal-0008", "mix", -1.0, 100.0);
    fx.differs(
        conv,
        "the noise is measured again in Lumit: select the effect and press Learn",
    );
    fx.drop_params(
        conv,
        &[
            "Viewing Mode",
            "the Preview Region group",
            "the Noise Reduction Settings group",
            "the Fine Tuning group",
            "Motion Sensitivity",
            "the Unsharp Mask group",
            "the Sampling group",
            "the Blend with Original matte and colour matching",
        ],
    );
    fx.done()
}
