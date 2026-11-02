What the next release has to say before anybody updates to it. A change that alters what a
project or a setting does adds its line here. Each nightly's GitHub release carries the
section below, and when the release note is written the lines move into it.

## Before you update

- Depth of field pro no longer leaves a fine grain over a wide blur, by either method.
- Fast layers with motion blur are drawn with more copies, up to 128, so they smear instead of stepping. A child now blurs with its parent, a 3D layer blurs when the camera moves, and the layers inside a collapsed precomp blur. Exports of those can look smoother and take longer. Set the second number beside **Samples** in **Composition settings** to the first to keep a fixed count.
- Glow, and the other effects that spread light, no longer show a faint box behind each letter
  of a Text layer.
- Corner pin and Bezier warp in a node graph now draw past the picture they're handed, so a graph that pins its picture and then scales it may show what used to be cut off.
