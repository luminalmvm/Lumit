//! The zero-copy Viewer target: how many handles a session hands out, and at
//! what sizes.
//!
//! The crash this exists for: "Binding D3D surface failed" from Flutter's
//! embedder, then the process gone, shortly after a comp is created from a
//! clip. Everything on this side checked out in isolation — the media probes
//! and decodes, a shared texture can be made at any size, and a comp of that
//! media renders and presents. What was left was the *rate* at which handles
//! changed, because Dart identifies a registered texture by its handle and a new
//! one costs a registration round trip during which the old texture is still on
//! screen.
//!
//! The discriminator in the report was that an empty project was fine. An empty
//! project is the case with no outgoing comp — and therefore no renders in
//! flight for a different size while the new comp starts, which is what makes
//! the sizes *alternate* rather than change once.
//!
//! Runs only where a GPU and the shared-texture feature are both present.

#![cfg(all(windows, feature = "shared-texture"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_render::headless::HeadlessRenderer;

/// **The regression.** Presenting alternately at two sizes — one comp's renders
/// finishing while another's begin, which is what creating a comp in an
/// existing project does — must not hand out a new handle every frame.
///
/// Before the target pool it did exactly that: each size change re-created the
/// one texture, so twenty alternating frames minted twenty handles and asked
/// the frontend for twenty registrations. Now each size keeps its target, so
/// the whole run uses two.
#[test]
fn alternating_sizes_reuse_their_targets_instead_of_minting_handles() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        return; // no adapter
    };
    let mut handles: Vec<u64> = Vec::new();
    for i in 0..20 {
        // The two shapes an outgoing and an incoming comp would present at.
        let (w, h) = if i % 2 == 0 {
            (1280, 720)
        } else {
            (1920, 1080)
        };
        let handle = r
            .present_probe_size(0, w, h)
            .unwrap_or_else(|e| panic!("present {w}x{h} on step {i}: {e}"));
        assert_ne!(handle, 0);
        handles.push(handle);
    }
    let mut distinct = handles.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        2,
        "alternating between two sizes must settle on two handles, not {} — \
         a fresh handle per frame is the registration storm that crashes the \
         compositor",
        distinct.len()
    );
    // And the two alternate rather than one having replaced the other.
    assert_eq!(handles[0], handles[2]);
    assert_eq!(handles[1], handles[3]);
}

/// The pool is bounded: walking through many sizes, as dragging the Viewer
/// does, must not accumulate targets without limit.
#[test]
fn the_target_pool_is_bounded() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        return;
    };
    for i in 0..24u32 {
        let w = 400 + i * 17;
        r.present_probe_size(0, w, 300)
            .unwrap_or_else(|e| panic!("present {w}x300: {e}"));
    }
    assert!(
        r.shared_target_count() <= 4,
        "the pool grew to {} targets",
        r.shared_target_count()
    );
}

/// **The multi-viewer regression.** Two views on comps of the same size must
/// not share a target (docs/impl/multi-viewer.md §2.2).
///
/// Before the pool was keyed by view, a lookup was by width and height alone,
/// so a second view on a 1920x1080 comp found the first view's texture and
/// presented into it. Both Viewers then showed whichever comp had rendered
/// last, flickering between two pictures a frame apart, and every line of code
/// involved read as correct.
#[test]
fn two_views_at_one_size_do_not_share_a_target() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        return; // no adapter
    };
    let left = r.present_probe_size(0, 1920, 1080).expect("left view");
    let right = r.present_probe_size(1, 1920, 1080).expect("right view");
    assert_ne!(
        left, right,
        "two views at one size were handed the same texture, so each \
         overwrites the other's picture every frame"
    );
    // And each keeps its own across the alternation a two-up produces.
    for _ in 0..8 {
        assert_eq!(r.present_probe_size(0, 1920, 1080).expect("left"), left);
        assert_eq!(r.present_probe_size(1, 1920, 1080).expect("right"), right);
    }
}
