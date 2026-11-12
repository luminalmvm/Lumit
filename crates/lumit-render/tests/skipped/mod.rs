//! The effects the example frame leaves without a picture.

/// Effects the example frame cannot honestly illustrate. Posterize time holds
/// one frame for several, which is a change to the clock and shows only in
/// motion. Matte key wants a screen to pull, and this frame has none, so every
/// setting of it either does nothing or keys something arbitrary. Both are left
/// without a picture; the manual's generator omits the figure for any effect
/// with no file on disk, so nothing links to an image that is not there.
pub(super) fn unillustrable(match_name: &str) -> Option<&'static str> {
    match match_name {
        "posterize_time" => Some("holds frames, which only shows in motion"),
        "matte_key" => Some("the example frame has no screen in it to key"),
        // Camera track and Planar track hold a job, and the Controls hold values
        // for expressions to read; none of them draws, so a picture would be the
        // plate twice.
        "camera_track" | "planar_track" | "slider_control" | "angle_control"
        | "checkbox_control" | "colour_control" | "point_control" | "custom_controls"
        | "template" => Some("draws nothing by design"),
        // The drivers. A driver answers with a *number*, a colour or a bag of
        // points for something else to use, and reaches whatever it drives
        // through a wire in the node graph. There is no picture of one.
        "wiggle" | "smooth" | "math" | "remap" | "audio_level" | "colour_cycle"
        | "points_sample" | "layer_points" | "split" | "combine" | "expression" | "clone_index"
        | "measure" | "beat_trigger" => Some("a driver: it answers with a value, not a picture"),
        // The Compositing family. Merge lays one picture over another, Switch
        // picks between several and Time offset shows its input at another
        // moment, and Split and Combine channels take a picture apart and put
        // it back: each is realised by the node graph's own walk rather than by
        // a kernel, and only a node graph can hold one. This harness stages one
        // effect on one layer, so there is no graph here to hold it and nothing
        // to wire a second picture from.
        "merge" | "switch" | "time_offset" | "split_channels" | "combine_channels" => {
            Some("only a node graph holds one, and there is no graph here")
        }
        // And the effect that applies a graph: it shows whatever graph it is
        // bound to, and this harness has no node graph composition to bind.
        "node_graph" => Some("applies a node graph, and there is none in this project"),
        // **Not a nature — a defect.** The physical flare adds nothing to a
        // headless render: the frame comes back bit-identical to the plate at
        // the defaults and still bit-identical with Intensity at 3.5 and an
        // area light placed on the brightest part of the picture. Its showcase
        // entry below is written and correct; it is skipped here so that the
        // run does not stop, and so that the manual does not carry a picture of
        // the plate with "Lens flare" under it. Take this line out the day the
        // flare's additive pass reaches this renderer.
        "lens_flare" => Some("adds nothing to a headless render (defect, not by design)"),
        // The three config-bound OCIO effects list the project's config, and
        // this harness has no config to point at. The file transform draws
        // the example cube, as the LUT does.
        "ocio_colour_space" | "ocio_display" | "ocio_look" => {
            Some("needs the project's OCIO config, and there is none here")
        }
        _ => None,
    }
}
