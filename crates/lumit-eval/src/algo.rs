//! The number that names how frames are drawn, and why it last moved.

/// Bump when any rendering algorithm's output changes — or when the key itself
/// starts covering something it did not: every cached frame keyed under the old
/// version stops being addressed, which is what makes a fix to the key safe.
///
/// * 1 — the original key.
/// * 2 — a layer's inherited parent-chain placement joined the key (see
///   [`crate::feed_layer`]). Under version 1 a hidden parent could be moved without
///   renaming its children's frames, so those frames were served stale; every
///   version-1 entry has to stop being addressed for the fix to mean anything.
/// * 3 — anti-aliasing. Two reasons at once, either sufficient: the key
///   now covers the project's sample count, and turning the setting on by
///   default changes what every comp renders to. Every version-2 frame was made
///   without anti-aliasing, so none of them may be served again.
/// * 5 — the Lens flare's aperture. The key now covers a file
///   parameter's size and modification time, so an edited .lens or .cube
///   renames the frames that read it; and the flare's bake snaps its
///   continuous iris dials to a step, which moves the picture very slightly
///   at every f-stop between two steps.
/// * 6: a 3D layer off z = 0 or tilted in a comp with no camera now
///   draws, where it used to be cut away.
/// * 7: a 2D layer and its children ignore the z and X/Y Rotation it
///   kept from being 3D.
/// * 8: a parented adjustment layer grades where its parent puts it.
/// * 9: a lit plain solid is lit at its own size, not as a stretched tile.
/// * 10: a Rhai expression reads another layer's properties at that layer's
///   own time.
/// * 11: Depth of field pro blurs up to the frame's edge, where it left a strip
///   of the edge pixel.
/// * 12: 3D layers draw farthest first and cut where they pass through each
///   other, so a 3D comp with a camera can look different.
/// * 13: with no camera layer, 3D layers are seen through the default camera.
/// * 14: camera depth of field blurs a 3D layer by each pixel's distance, round
///   on screen, Scene depth lets a higher group of 3D layers cover a lower
///   one, and an adjustment layer's Depth row pointed at itself reads the 3D
///   layers below.
/// * 15: shadows are drawn, where a light and a layer cast them.
/// * 16: camera depth of field blurs a 3D layer's outline too, and blurs as
///   much as After Effects does.
/// * 17: on main, a 3D layer's outline blurs under the camera's depth of
///   field, and a plain solid under one is blurred at its own size.
/// * 18: main's 17 meets 12 to 16.
/// * 19: a shape whose outline crosses itself is filled where it crosses.
/// * 20: main's 18, a phone clip filmed upright drawn upright, meets 19.
/// * 21: an open shape path is filled, as if a straight line closed it, and
///   an open mask no longer hides its layer.
/// * 22: Depth of field pro smooths the grain out of a wide blur.
/// * 23: a blurred layer is drawn with more copies the faster it moves, its
///   parents and the camera move them too, a still one is drawn once, and the
///   inner layers of a collapsed Precomp blur.
/// * 24: Corner pin and Bezier warp on a node graph box draw past their picture.
/// * 25: Model layers cast and take shadows.
/// * 26: an OpenEXR packed with PIZ is read by Lumit's own reader.
/// * 27: a 3D model's glass and clearcoat from its file are drawn.
/// * 28: a project at 8 or 32 bits draws its effects.
/// * 29: Particulate in the scene follows a collapsed precomp.
/// * 30: a mirrored model's normal map turns the right way, what's hidden in
///   Blender is left out, and Shaded is a little darker.
/// * 31: the 3D layers in a collapsed Precomp take the parent's lights.
/// * 32: a layer that only shows shadows takes them with its back to a light.
/// * 33: the camera's depth of field blurs a Model layer.
/// * 34: a row reading a Precomp layer sees that layer's own paint and effects.
/// * 35: a Precomp layer and a motion blurred layer are multiplied by their
///   alpha once, where their soft edges and see-through parts came out dark.
pub const ALGO_VERSION: u32 = 35;
