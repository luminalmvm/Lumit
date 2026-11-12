//! Tests for footage too big for a texture.

use super::*;

/// A source wider or taller than the texture limit decodes shrunk to fit,
/// whatever the quality asked for; anything that fits is left alone.
#[test]
fn oversize_footage_decodes_within_the_texture_limit() {
    assert_eq!(fit_texture(None, 7680, 4320), None, "8K fits");
    assert_eq!(fit_texture(Some(960), 7680, 4320), Some(960));
    assert_eq!(fit_texture(None, 9000, 5000), Some(8192), "wide: capped");
    assert_eq!(
        fit_texture(None, 5000, 9000),
        Some(4551),
        "tall: the height caps"
    );
    assert_eq!(
        fit_texture(Some(4000), 9000, 5000),
        Some(4000),
        "smaller ask kept"
    );
    assert_eq!(
        fit_texture(Some(8500), 9000, 5000),
        Some(8192),
        "larger ask capped"
    );
}
