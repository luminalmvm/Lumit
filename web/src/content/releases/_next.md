What the next release has to say before anybody updates to it. A change that alters what a
project or a setting does adds its line here. Each nightly's GitHub release carries the
section below, and when the release note is written the lines move into it.

## Before you update

- Particulate flocks move differently. Separation no longer throws particles apart, and steering turns particles without slowing them down.
- OFX plugins now render on the graphics card where they support it, which can change a frame very slightly. **Render plugins on the graphics card** in Settings turns it off.
- Sprite flares made before this version have slightly different ghosts. The glow and the streak are unchanged.
- Label points labels sit where their points are, between pixels, instead of snapping to the nearest whole pixel, so they can look slightly softer.
- Open shape paths are filled now, as in After Effects. An imported open path with a fill shows it.
- An open mask no longer hides its layer. It cuts nothing, as in After Effects.
