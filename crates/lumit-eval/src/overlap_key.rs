//! What two clips overlapping on a picture row add to a frame's name. A dissolve
//! keeps the name it has always had, so every frame cached under it is still
//! found, and a Transition effect adds itself, how far through it is, and each
//! clip's own stack, which it draws on that clip alone.

use lumit_core::sequence::Clip;

/// Feed an overlap's gain and opacity, and the effect the incoming clip names.
pub(super) fn feed_overlap(
    h: &mut blake3::Hasher,
    outgoing: &Clip,
    incoming: &Clip,
    gain: f64,
    opacity: f64,
    lt: f64,
) {
    h.update(b"seq-dissolve/");
    super::feed_f64(h, gain);
    // The clips' own opacity, drawn as the layer's.
    if opacity < 1.0 {
        super::feed_f64(h, opacity);
    }
    if let Some(effect) = incoming.drawn_transition() {
        h.update(b"seq-transition/2/");
        // Written straight into the hasher. Serialising plain data cannot fail.
        let rows = (&effect.effect.match_name, &effect.params);
        let _ = serde_json::to_writer(&mut *h, &rows);
        for clip in [outgoing, incoming] {
            let stack = clip.fx.then_some(&clip.effects);
            let _ = serde_json::to_writer(&mut *h, &stack);
        }
        super::feed_f64(h, lt - incoming.place_start.to_f64());
    }
}
