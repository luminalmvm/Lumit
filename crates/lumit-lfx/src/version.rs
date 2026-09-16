//! The one number an LFX release is called by (docs/impl/lfx.md §4.1).
//!
//! # In plain terms
//!
//! Lumit names every cached frame after everything that went into it, and one
//! of those things is which version of an effect drew it. A plugin declares
//! three numbers - major, minor, patch - and Lumit's `EffectKey` carries one.
//! This module is the squeeze, and the whole of what it has to get right is
//! that no two releases come out the same: if 1.2.0 and 1.1.500 shared a
//! number, a user who updated a plugin would go on being served the pictures
//! the old one drew, with nothing on screen to say so.
//!
//! So the digits are grouped in thousands - `major × 1 000 000` plus
//! `minor × 1 000` plus `patch` - and every way that grouping could stop being
//! one number per release is refused at describe rather than folded into its
//! neighbour: a minor or a patch of a thousand or more, which would borrow the
//! next component's digits, and a major of [`MAJOR_LIMIT`] or more - the first
//! major whose million-block does not fit a `u32` whole, so that the domain
//! stops one release short of the arithmetic rather than at it. What is
//! accepted here is exactly what is injective - there is no admitted corner
//! left over.
//!
//! Ten thousands and hundreds would be tighter and would not be injective:
//! patch 150 would key identically to (minor + 1, patch 50). No ceiling guard
//! catches that one, because it is not a ceiling.

use crate::LfxRejection;

/// The first minor or patch number that is refused. Each component occupies
/// exactly three decimal digits of the minted version, so a thousand is where
/// one release would start borrowing the next one's number.
pub const COMPONENT_LIMIT: u32 = 1_000;

/// What one step of the major number is worth.
const MAJOR_SCALE: u32 = 1_000_000;

/// What one step of the minor number is worth.
const MINOR_SCALE: u32 = 1_000;

/// The first major number whose own block of a million does not fit - 4 294.
///
/// An in-range release of major `n` mints somewhere in
/// `n × 1 000 000 ..= n × 1 000 000 + 999 999`, so the last major that fits a
/// `u32` whole is 4 293 and this is the one after it. Not where the `u32` runs
/// out - that is 4294.967.295, and 4294.0.0 would count perfectly well - but
/// where it stops holding a *whole* major, which is one release earlier and is
/// the bound that keeps the accepted domain the injective one. Clamped
/// instead, 4294.968.0 and 4294.969.0 would both mint `u32::MAX` - two
/// releases sharing a frame key, which is the failure [`COMPONENT_LIMIT`]
/// exists to prevent, arriving by the other road.
pub const MAJOR_LIMIT: u32 = (u32::MAX - (MAJOR_SCALE - 1)) / MAJOR_SCALE + 1;

/// The `EffectKey::version` an LFX release is keyed by, or why it has none.
///
/// Every LFX release re-keys its frames, which is the point: a plugin update
/// is new maths under the same identifier, and the frames the old maths drew
/// must not be served for it. The number reaches the frame key twice - off the
/// stored key in `lumit-eval`, and off the schema in `ResolvedFx::feed_hash` -
/// so it is minted once, here, and both readers see the same arithmetic.
///
/// Inside the accepted range the arithmetic is plain rather than saturating:
/// the refusals in front of it are what makes overflow unreachable, so an edit
/// that breaks one of them fails a debug build instead of quietly clamping two
/// releases onto one number.
///
/// # Errors
///
/// [`LfxRejection::VersionOutOfRange`] when the minor or the patch is
/// [`COMPONENT_LIMIT`] or more, or the major is [`MAJOR_LIMIT`] or more -
/// the three ways the mapping would stop being one number per release.
pub fn mint(major: u32, minor: u32, patch: u32) -> Result<u32, LfxRejection> {
    if major >= MAJOR_LIMIT || minor >= COMPONENT_LIMIT || patch >= COMPONENT_LIMIT {
        return Err(LfxRejection::VersionOutOfRange {
            major,
            minor,
            patch,
        });
    }
    Ok(major * MAJOR_SCALE + minor * MINOR_SCALE + patch)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The three declared numbers become one, and each of them moves it - so
    /// any LFX release re-keys its frames, a patch release included. That last
    /// clause is the whole reason the arithmetic is not the OFX host's, which
    /// keys on the major alone and serves a minor release the frames of the
    /// one before it (docs/impl/lfx.md §13).
    #[test]
    fn every_release_mints_a_version_of_its_own() {
        assert_eq!(mint(1, 2, 3), Ok(1_002_003));
        assert_eq!(mint(0, 0, 0), Ok(0));
        assert_eq!(mint(0, 0, 1), Ok(1));
        assert_eq!(mint(2, 0, 0), Ok(2_000_000));

        // Each component on its own renames the frame.
        let base = mint(1, 2, 3).expect("in range");
        for bumped in [mint(2, 2, 3), mint(1, 3, 3), mint(1, 2, 4)] {
            assert_ne!(Ok(base), bumped, "a release that changed kept its number");
        }
    }

    /// The property the grouping exists for: no two releases the refusals
    /// admit share a number. Swept over the corners a decimal grouping gets
    /// wrong - the pairs that collide under `× 10 000 + × 100`, which is the
    /// tighter arithmetic this one was chosen over - and over the top of the
    /// accepted range, where the last whole million-block ends.
    #[test]
    fn the_minted_version_is_injective_in_range() {
        let mut seen = std::collections::HashSet::new();
        for major in [0_u32, 1, 2, 4_000, MAJOR_LIMIT - 1] {
            for minor in [0_u32, 1, 9, 10, 99, 100, 999] {
                for patch in [0_u32, 1, 9, 10, 50, 99, 100, 150, 999] {
                    let v = mint(major, minor, patch).expect("every component is in range");
                    assert!(
                        seen.insert(v),
                        "{major}.{minor}.{patch} minted {v}, which another release already owns"
                    );
                }
            }
        }
        // The pair the tighter arithmetic would have collided on, spelled out:
        // patch 150 against (minor + 1, patch 50).
        assert_ne!(mint(1, 0, 150), mint(1, 1, 50));

        // And the pair the ceiling would have collided on, which is why the
        // major above the last one that fits is refused rather than clamped:
        // both of these would be `u32::MAX` under saturating arithmetic.
        for (minor, patch) in [(968_u32, 0_u32), (969, 0)] {
            assert!(
                mint(MAJOR_LIMIT, minor, patch).is_err(),
                "{MAJOR_LIMIT}.{minor}.{patch} was minted, and it has a twin"
            );
        }
    }

    /// A minor or patch number of a thousand or more is refused by name, at
    /// describe, rather than quietly borrowing its neighbour's frames - one of
    /// §14 item 3's structural refusals. The major has its own bound, pinned
    /// by `a_major_whose_block_does_not_fit_whole_is_refused` beside this.
    #[test]
    fn a_minor_or_patch_of_a_thousand_or_more_is_refused() {
        assert_eq!(
            mint(1, 1_000, 0),
            Err(LfxRejection::VersionOutOfRange {
                major: 1,
                minor: 1_000,
                patch: 0,
            })
        );
        assert_eq!(
            mint(1, 0, 1_000),
            Err(LfxRejection::VersionOutOfRange {
                major: 1,
                minor: 0,
                patch: 1_000,
            })
        );
        assert!(mint(1, 999, 999).is_ok(), "the last in-range release");

        // The refusal names the numbers it refused, so the report line is a
        // sentence rather than a silence.
        let why = mint(3, 4, 5_000).expect_err("out of range");
        let text = why.to_string();
        assert!(
            text.contains("3.4.5000"),
            "the reason did not say which: {text}"
        );
    }

    /// The ceiling is a refusal too, not a clamp - and it is one release
    /// short of where the arithmetic gives out, which is the part a reader
    /// guesses wrong. `EffectKey::version` is the `u32` the format already
    /// stores; the `u32` itself counts as far as 4294.967.295, so 4294.0.0 is
    /// a number it holds. What 4 294 is, is the first major whose own block of
    /// a million does not fit whole - and clamping rather than refusing there
    /// would hand 4294.968.0 and 4294.969.0 the same number, so the domain
    /// stops at the last major that fits entire.
    #[test]
    fn a_major_whose_block_does_not_fit_whole_is_refused() {
        assert_eq!(
            MAJOR_LIMIT, 4_294,
            "the last major that fits whole is 4 293"
        );
        // And the reason it is 4 294 rather than 4 295: the arithmetic for
        // 4294.0.0 is in range, so this bound is the block that does not fit,
        // not the number that does not exist.
        assert!(u64::from(u32::MAX) > u64::from(MAJOR_LIMIT) * u64::from(MAJOR_SCALE));
        assert_eq!(
            mint(MAJOR_LIMIT - 1, 999, 999),
            Ok(4_293_999_999),
            "the last release in range, exact to the digit"
        );
        for absurd in [MAJOR_LIMIT, 5_000, u32::MAX] {
            assert_eq!(
                mint(absurd, 0, 0),
                Err(LfxRejection::VersionOutOfRange {
                    major: absurd,
                    minor: 0,
                    patch: 0,
                })
            );
        }
        // And a major well inside the ceiling is still exact.
        assert_eq!(mint(4_000, 0, 0), Ok(4_000_000_000));
    }
}
