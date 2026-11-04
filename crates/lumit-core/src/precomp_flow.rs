//! Flow on a Precomp layer. A comp has no frames on disk to measure between,
//! but it can be built at the frame before and the frame after the moment the
//! layer shows, and the picture between them made from the motion. The draw
//! builder, the decode planner and the frame key all ask [`pair`], so they
//! can never pick different frames.

use crate::model::{Composition, Layer};
use crate::retime::Interpolation;

/// The two frames of `nested` a Precomp `layer` on Flow is made from at layer
/// time `lt`, in a comp running at `comp_fps`: the nested times of the frame
/// before and the frame after, and how far towards the second the picture
/// sits, above 0 and below 1.
///
/// Frames are counted at the Flow group's Input rate when one is set below the
/// nested comp's own, then each lands on the nearest real frame, as a footage
/// layer's conform does. So a comp drawn on twos, conformed to half its rate,
/// moves between drawings rather than between a drawing and its copy.
///
/// `None` off Flow, where flow would not engage (the same gate as footage),
/// and on a moment that is a whole frame already.
#[must_use]
pub fn pair(
    layer: &Layer,
    nested: &Composition,
    lt: f64,
    comp_fps: f64,
) -> Option<(f64, f64, f32)> {
    let Interpolation::Flow(p) = &layer.interpolation else {
        return None;
    };
    let native = nested.frame_rate.fps();
    let rate = p.read_fps_at(lt, native);
    let speed = crate::retime::property_speed_at(layer.retime.as_ref(), lt);
    if !(native > 0.0 && p.engages(rate, comp_fps, speed)) {
        return None;
    }
    let at = crate::model::nested_source_time(layer, nested, lt).max(0.0) * rate;
    let first = at.floor();
    let phase = (at - first) as f32;
    let last = ((nested.duration.0.to_f64() * native).ceil() - 1.0).max(0.0);
    let on_frame = |v: f64| ((v / rate) * native).round().clamp(0.0, last) / native;
    let (a, b) = (on_frame(first), on_frame(first + 1.0));
    // Within a ten-thousandth of a frame is that frame, which also keeps a
    // float's last bit from making a frame on the grid an in-between.
    (phase > 1e-4 && a < b).then_some((a, b, phase))
}
