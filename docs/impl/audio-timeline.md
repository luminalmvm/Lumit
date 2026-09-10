# The Audio timeline: tracks, clips, fades and clip effects

**Status: design, built in the ordered packages below.** [09-AUDIO.md](../09-AUDIO.md)
§1 names the Audio workspace and [07-UI-SPEC.md](../07-UI-SPEC.md) §1.6 its panels; this
note is the binding *how* for the **Audio timeline** panel that stands where the layer
Timeline stood in that workspace: what a track and a clip are in the document, how a fade
and a crossfade are stored and heard, how a clip carries its own effects, and how the panel
draws and edits all of it. Ground truth for the drawing is the approved AudioWorkspace board;
for the behaviour, Vegas Pro's audio tracks, copied where they are right and bettered where
they are not.

**In plain terms:** the layer Timeline is a stack of layers with columns for a picture
edit. Mixing sound wants a different table: rows that are *tracks*, each holding any number
of *clips* laid end to end, a clip you can trim by its edges, fade by its top corners, slide
along and drop on another track, and open up to keyframe the effects on it alone. Nothing
new is invented underneath: a track is a Sequence layer that draws nothing, a clip is a
clip, and the mixer already plays one job per clip through the row's chain. What is new is
four fields on a clip, a second chain on a job, a few bridge calls, and a panel that shows
them the way a mixing desk would.

## 1. Words

In the Audio timeline's own strings a row is a **track** and a thing on it is a **clip**,
and the panel's own files may say track in an identifier or a comment for the same reason.
Everywhere else, and in every op and bridge name, the row is a Sequence layer and the thing
on it is a clip, as [01-GLOSSARY.md](../01-GLOSSARY.md) says. The
glossary carries the one scoped exception for *track* (§8 of this note lands it). *Item* is
not used for a clip: the Project panel already calls its assets items.

## 2. The document (decided)

- **A track is a Sequence layer with `audio_only` set.** The picture path already skips
  such a layer and the mixer already walks its clips (`AudioJobsBuilder::sequence_jobs`),
  so a track sounds today. `add_clip` on a layer places a footage item at a comp frame
  through `commit_clips`; dropping media on empty ground makes a plain Audio layer through
  the existing `add_audio_layer`, which is a track of one clip by the next rule.
- **A plain Audio layer is a track of one clip.** It keeps its own row and the board's own
  bare lane. The first clip gesture on it (a trim, a fade, a move to another track)
  converts it with `convert_to_sequenced` between `begin_undo_group` and `end_undo_group`
  around the edit, so one undo puts the layer back as it was. Looking at the panel changes
  nothing. The conversion must keep the sound: the clip is placed at the layer's own in
  point with `source_in` at the same distance from the layer's start offset, so a trimmed
  or slid layer converts to the same seconds of the same file at the same comp time; the
  conversion is **refused on a retimed layer**, because a retimed clip is silent (09 §7),
  and the panel offers a retimed Audio layer no clip gesture. `convert_to_sequenced` is
  corrected in AT2 to do this for every caller.
- **A clip gains four fields**, each written only when set so a project written before
  them re-saves byte-identical:
  - `fade_in: Fade` and `fade_out: Fade`, where `Fade { seconds: Rational, shape: FadeShape }`
    and a zero length means no fade; `#[serde(default, skip_serializing_if)]`.
  - `effects: Vec<EffectInstance>`, the clip's own stack, the same shape a layer's and a
    group header's stack has (docs/impl/group-effects.md is the precedent).
  - `fx: bool`, the clip's whole-stack bypass, `#[serde(default = "default_true",
    skip_serializing_if = "is_true")]` as `model.rs` writes the same flag elsewhere.
- **A clip has a gain of its own.** `Clip::gain_db: f64`, `#[serde(default,
  skip_serializing_if = "is_zero")]`, a number and not a `Property`: the line on the clip
  is set, not automated, and automation is the track's Volume row under the twirl. It is
  applied where the fades are, as one multiplier on the clip's placed gain
  (`fade(t) × 10^(gain_db / 20)`, with the same silence knee at −100 dB the fader has), so
  it rides ahead of nothing and after nothing in the chain, exactly as the fades do.
  Written by `set_clip_gain(clip, db)` through `Op::SetSequenceClips`; a cut keeps it on
  both pieces; the signature hashes it.
- **A clip's colour is its track's.** The colour box on a clip shows the layer's label and
  opens the layer's label picker. A colour per clip was considered and not built: nothing
  in the board draws two colours on one track.
- **What a cut does to the fields.** `Clip::cut` and the straddle split in `overwrite_with`
  are the only places a clip becomes two: the left piece keeps `fade_in`, the right keeps
  `fade_out`, both keep `fx`, and each takes a clone of `effects` with **fresh instance
  ids**, because the instance lookup finds an instance by its id alone and two pieces
  sharing one would answer for each other. No fade is added at the cut: the two halves abut
  sample-exactly and play as the one sound did. A trim keeps the fields and the bake clamps
  a fade to the clip's span. A delete takes the clip's stack with it.
- **Every clip edit is `Op::SetSequenceClips`**, the whole-list replace that is already
  exactly invertible; fades, bypasses and effect stacks ride it. A new bridge call reads the
  list, changes one clip, and writes it back through `commit_clips` so the layer's span
  follows. `InstanceHome::Clip(id)` is the fourth arm of the instance lookup, so
  `remove_effect`, `reorder_effect`, `set_effect_enabled` and `set_effects` reach a clip's
  stack with no second write road; `add_clip_effect` is the clip's own add, since
  `add_effect` has no instance to look up, and `set_effects`'s empty-list fallback gains a
  clip arm so removing a clip's last effect does not land on the layer's stack. The op's
  inverse clones every clip's stack, plugin state and all, on every trim; that is the
  ceiling, named in the code, and a reuse of the unchanged list is the upgrade if an edit
  ever breaks the docs/13 budget.
- **A composition remembers that it has been mixed.** `Composition::sound_mix: bool`,
  `#[serde(default, skip_serializing_if = "std::ops::Not::not")]` so a project that has
  never been mixed re-saves unchanged, written by `Op::SetSoundMix { comp, on }`, exactly
  invertible and reading *Sound mix* in the history. The Audio timeline sets it on the
  **first edit made from the panel**: every write road in the panel ends in one
  `_afterWrite`, which marks the comp once and refreshes the model, so looking at a comp
  writes nothing and a comp already marked gets no second write. The mark is its own undo
  step after that first edit, because the row's callbacks fire once the edit's own undo
  group has closed; the ceiling is named in the code. The layer Timeline reads it once per
  revision, beside the master fader. **Convert to precomp** clears it:
  `precompose_sound_mix(name)` on the bridge builds, in one `Op::Batch` inside the undo
  group, **one comp per track** holding one audio-only Sequence layer per clip, and one
  **mix comp** holding one Precomp layer per track, then leaves one Precomp layer for the
  mix comp in the parent where the topmost track stood, audio-only, and soloed where any
  packed track was, so the parent stays as quiet as it was. A track is every layer that reads
  as `BridgeLayerKind::Audio`, which is what the fold takes. A clip goes into its own
  layer whole, `place_start` kept, `start_offset` zero, in and out points the clip's span,
  fades, effects and `fx` untouched, after its crossfade is baked into its stored fade
  seconds, because the mixer finds a join only in a layer's own clip list. A plain Audio
  layer is split as the one clip `convert_to_sequenced` would make; a retimed one moves
  whole. The track's Precomp layer carries the track's name, label, span, start offset,
  volume, pan, switches and markers, so the Audio panel's fades, which are Volume keys, are
  heard at the same moments. The track's effect rack is **copied on to each clip layer**
  with fresh instance ids: the mixer opens a rack on Footage and Sequence layers only, and
  a rack on a Precomp layer is silent; a bus chain on a Precomp layer is the named upgrade
  in TODO. Inner time is outer time less the Precomp layer's start offset and every
  carrier added is unity, so the sound is bit-identical. One undo puts everything back.
  There is no road back from a precomp to a mix.
- **Overlap is a crossfade on a track and nothing else.** Two clips on an audio track may
  overlap, and the overlap is the crossfade. `add_clip`, `slide_clip` and `move_clip` each
  take `overlap: bool`: true keeps what is landed on, false runs `overwrite_with` as today.
  The panel passes true; a Sequence layer that draws a picture still overwrites, because
  `sequence::resolve` shows one clip per frame and cannot dissolve. The no-overlap rule
  therefore survives **per layer**: a picture Sequence never overlaps, an audio track may.
  03 §5.3 and every other statement of the rule (§8) say so.

## 3. Fades (decided)

A **shape** is a gain curve g(u) for a fade *in*, u from 0 at silence to 1 at full level:

| Shape | g(u) | Vegas name |
|---|---|---|
| Linear | u | Linear |
| Fast | sin(πu / 2) | Fast |
| Slow | 1 − cos(πu / 2) | Slow |
| Smooth | u² (3 − 2u) | Smooth |
| Sharp | the inverse of Smooth | Sharp |
| Custom | a cubic bezier from (0, 0) to (1, 1) with two handles (x1, y1), (x2, y2), x monotone | none |

A fade **out** plays g(1 − u). So a shape is one curve, named once, and the direction of the
fade decides which way it is read; a Fast fade in and a Fast fade out are one shape at the
two ends of a clip, which is what the presets menu shows.

**How long a fade is.** Where a clip overlaps nothing at that end, its fade is its own
`fade_in.seconds` or `fade_out.seconds`, clamped to the clip's span. Where two clips
overlap, the overlap is the length of both fades across it and the stored seconds are not
read; the stored shapes are what a crossfade takes from each clip. A join is one clip's
end lying inside another: a clip dropped wholly inside a longer one joins it at neither
end, and both are heard for the seconds they store. `ClipFade` gains a head
shape and a tail shape and `gain_at` evaluates them; the head and the tail still multiply,
so a clip shorter than its two fades is heard as their product.

**Equal power by default.** Fast in with Fast out over one overlap is sin with cos, whose
squares sum to one: the crossfade the mixer plays today, and the pair a fresh overlap gets.

**Custom.** The two handles are the Easing panel's own `EasingCurve` numbers, with the same
clamps, and the curve is read at x = u. Across the bridge this is `BridgeClipFadeShape`, a
new enum with a `custom` variant holding the four numbers; the layer fade commands and the
Audio panel's three chips keep `BridgeFadeShape` and are not touched.

**Keep level.** The custom editor shows a crossfade as two curves in one box, the outgoing
clip's from top left to bottom right and the incoming clip's from bottom left to top right.
With *Keep level* on (the default) a drag on one curve's handle sets the other curve to its
**power complement**, g_other(u) = sqrt(1 − g_this(u)²), so the two gains keep their squares
summing to one at every point and the join stays as loud as the default is: the power
complement of Fast is Fast. Off, each curve is its own. For a lone fade the box shows one
curve running from the corner the fade starts in, and no *Keep level* box.

**What changes what is heard.** `jobs_signature` hashes each job's fade shapes and lengths,
the clip's `fx` flag and its effect stack, and the layer's `fx` switch, so a shape edit that
moves no clip still rebuilds the mix.

## 4. Effects on a track and on a clip (decided)

- **A track's rack is the layer's stack**, as it is today: the audio entries of
  `Layer::effects`, edited in the Effect controls panel and under the track's twirl.
- **A clip's rack is `Clip::effects`, baked as a chain of its own.** An `AudioJob` gains a
  second chain beside the layer's: `clip_chain`, built from the clip's stack with its time
  zero at the clip's own start, dropped whole when `clip.fx` is false. The two places that
  bake a job (`build_plan` and `mix_decoded`) run the clip chain first and the layer chain
  on its output, summing the two latencies; clip first, then track, which is the order Vegas
  processes event FX and track FX and the order a rack reads on the board. Nothing else
  about `chain_bake` changes, so the determinism contract of docs/impl/audio-plugins.md §3
  holds: each chain's blocks count from its own first sample.
- **A clip's keyframes are in clip time.** A parameter keyed on a clip's effect is keyed
  from the clip's start, as the clip's Retime is, and rides with the clip when it is slid;
  the clip chain's offset is what makes the bake read it there. The panel's lanes draw such
  keys through the clip's `place_start` and the layer's start offset, the same walk the
  sequence view's envelope makes for Retime keys, and map a dragged key back the same way.
- **The layer's `fx` switch now reaches the mix.** Until this note `Switches::fx` silenced a
  layer's picture effects and left its audio plugins processing. `audio_chain_of` reads it
  from now on, so **a project saved with the switch off on a layer carrying an audio plugin
  plays and exports without that plugin from this version on.** Said in
  [09-AUDIO.md](../09-AUDIO.md) §6 and in the change's own draft.

## 5. The panel (decided)

`Panel.audioTimeline`, titled *Audio timeline*, takes the Timeline's place in the Audio
preset; the other presets are untouched and the Window menu offers it anywhere. It reads the
composition off the held read model and crosses the bridge only from a gesture. It keeps
its own zoom, scroll, open set and lane modes, because in the Audio preset it stands where
the Timeline stood; nothing is lifted into the shell's state for it. It claims delete, copy,
paste and the easing apply slot only while it holds `activePanel`, and lets them go when
another panel takes the keys, so the two timelines can stand on screen together. Its keys
are the Timeline's: it maps to `BridgeKeyContext.timeline`, which is where the razor, the
split and the transport keys come from.

**Tracks.** The panel lists, in stack order, every layer that can make a sound: an Audio
layer, a Sequence layer that draws nothing, footage that carries sound, and a Precomp or
Sequence layer with sound in it. The rule is `timelineViewLayers` with its faded set, moved
from the layer Timeline's Sound view: a row that also has a **picture** (footage with both,
a Precomp, a picture Sequence) is drawn faded and takes no pointer, and in place of its
lane-mode chip it wears one button, **Detach audio**; once detached the muted picture row
leaves the list and its sound stands on a track of its own. A muted track stays listed. A
track is **two lane rows tall**; it grows only by the rows its twirls open.

**The outline row** is the board's: mute, solo, an *fx* switch, the twirl, the number, the
name, and at the right edge the lane-mode chip cycling **Wave** and **Spectral**. The
switch cells are the layer Timeline's own cells with the rest of the columns absent, so
pressing one is the same `set_switch_on_layers` call and the same undo step. The lane mode
is the panel's own per-track state, wave to begin with; the layer Timeline's three-mode
store is not shared, so a choice here does not change a lane there. A press on the row
**selects the track**, the shell's selection, so the Effect controls panel follows it and
the name lights as a selected layer's does; **Enter renames** it in the row with the layer
Timeline's own rename field. Above the column header stands the layer Timeline's **chrome
strip**: the timecode, the frame count over the comp length, and the search box, which
filters the tracks by name; LAYERS and GRAPH are not there. The outline draws the lane
half's **row hairlines**, the same pinned seam overlay the layer Timeline uses, so the two
halves rule the same rows and the same empty ground.

**The twirl** opens straight onto the track's **Volume** row, keyed at the path the layer
Timeline keys it so a fold path means the same thing in both, though each panel remembers
its own twirls. An **Effects** heading always follows, carrying an add-effect glyph that
opens the audio catalogue for the **track's own rack**, separate from any clip's; the
rack's parameter rows sit under it as the layer Timeline draws them. There is no Audio
heading and no Waveform twirl: the wave is on the lane.

**An unconverted Audio layer** draws the board's bare row: its wave or spectrogram on the
lane ground through the layer's own peaks, the dB readout at the right, and the track's
Volume rubber band over it. Its two ends take the trim and fade gestures below, and the
first of them converts it. The clip chrome appears only once the row holds clips.

**Clips.** A clip is a box on its track filled with the track's label colour at the clip
fill alpha, a solid leading edge, and a header strip carrying, left to right: the colour
box, the source's name, the *fx* toggle, an **add effect** button that opens the add-effect
menu filtered to audio effects, and a twirl. Below the strip the clip draws its own wave or
spectrogram through `clip_audio_peaks` or `clip_audio_spectrogram`, bucketed in the clip's
placed time so a slid clip carries its picture with it. The wave is drawn on a square-root
scale of its level, so quiet sound reads without loud sound filling the box. Across the box
lies the **gain line** at the clip's gain: 0 dB at the box's top, silence at its foot on
the Volume band's own dB scale, dragged up and down and written on release; the fades'
ramps rise to the gain line rather than to the box's top, so a faded clip reads as loud as
it is. A click on a clip's body selects it, which is panel state; Escape and a click on
empty ground clear it; Delete deletes it. While a drag is on, every part of the box, the
header, the picture, the fades, the gain line and the marks, moves with the box.

**Gestures on a clip**, one commit on release, snapped through the Timeline's shared snap
module with Ctrl suspending the snap (the bar carries no magnet), and abandoned with Escape:

- the body slides the clip along its track; a drag that crosses into another track moves it
  there (`move_clip`), a drop onto another clip keeps both, overlapped, and a drop below the
  last track moves the clip onto a new track of its own, which is how clips are spread out
  again without a button for an empty track;
- either edge trims, with the trim zone capped at a third of the clip so a short clip keeps
  a body to hold; the head snaps to the source's start and the tail to the source's end
  whichever way the edge is moving, so a trim taken in comes back out to the sound's own
  edge; the edges of every clip on the track are hit before any clip's body, so the earlier
  clip's tail is still there to trim under the later clip of a crossfade;
- either **top corner** drags a fade in from that end where the clip overlaps nothing there;
  dragging it back to the edge removes the fade, and the fade's length shows while it is
  dragged. Inside an overlap the same corner drags the clip's edge, which is what changes
  the crossfade's length, since the overlap is the fade;
- a right click on a fade, or on an overlap, opens the fade menu: the five shapes by name,
  then *Custom…*, which opens the two-curve editor with *Keep level*;
- a right click on the body opens the clip menu: fade in and fade out with the same
  shapes, split at the playhead, delete;
- the razor cuts a clip at the pointer, as it does in the layer Timeline. The razor and the
  Layer menu's convert and retime gates test for a clip list rather than the Sequence kind,
  because a track answers `BridgeLayerKind::Audio`.

**The drop-down.** A clip's twirl grows the track's block by rows drawn under the track: a
heading with the clip's name, then one heading per effect on the clip and the parameter rows
under each open one, each with its keyframe lane beside it. Several clips may stand open at
once, in track order. The rows come from the same list function both halves of the table
walk, so the outline and the lanes cannot disagree about a track's height.

**Dropping media.** Footage dragged from the Project panel onto a track becomes a clip at
the pointer, overlapping what it lands on; dropped on empty ground it becomes a new Audio
layer, which is a track of one clip. A footage item with a picture dropped here still
becomes a picture layer, and so shows faded until its audio is detached.

**The lane strip** is the layer Timeline's: the ruler with the beat band and the cache bar,
the navigator above it, the work-area wash, the playhead on its own layer, the marquee
ground, the comp tabs above. The bottom bar carries the zoom slider and the scrollbar and
nothing else, which is the board's bottom bar. The track's Volume band is not on the lane:
the board drew one on the Music row, and the gain line on each clip took its place at the
owner's word; the track's Volume keeps its row under the twirl. **A track's height is its
own**: a drag on the outline row's bottom edge sets it between two and eight lane rows,
panel state like the lane mode; the block, the lanes, the clip boxes and the fold rows
follow.
The Wave lane draws the **three-band stack**, the same three stops the Spectral lane
blends, so the two pictures share one range of colour.

**The source's start.** A clip whose head has been dragged before its source's first
sample plays silence until the sound begins (`source_in` below zero). The left-edge drag
snaps to that point, and the box wears a small triangle at its top edge where the sound
starts, so the silence reads as what it is.

**The Sound mix row in the layer Timeline.** The row stands only once the comp has been
mixed (an edit made in this panel, §2), pinned at the foot of both halves, and from then on the Audio
layers are out of the stack and behind it: the row says how many, and its twirl brings them
back for a look, as view state the next mount forgets. A comp that has never been to the
Audio timeline shows no row and keeps its Audio layers in the stack. A double click on the
row, either half, applies the Audio workspace preset. A right click offers **Open Audio
workspace** and **Convert to precomp**; the latter is the one bridge call of §2 and leaves a
Precomp layer named *Sound mix* selected where the top Audio layer stood, and no row,
because the parent is no longer mixed.

**What the panel does not do.** No stack lane mode. No per-clip colour. No ripple. No
per-clip mute or solo. No automated clip gain. No Sound mix row: the
master is the Mixer's job. The layer Timeline's Sound view, its toggle and its faded rows
are gone, and the rule that drew them is this panel's.

## 6. Test plans (implement with each package)

1. **Shapes.** Every shape has g(0) = 0 and g(1) = 1 and is monotone; Fast with Fast sums
   its squares to one across a crossfade; Linear with Linear sums to one; the power
   complement of a custom curve sums its squares with it to one at every sampled point, and
   the complement of Fast is Fast.
2. **Length.** A lone fade is heard for its own seconds, clamped to the clip; an overlap
   is heard for the overlap whatever the seconds say; a clip shorter than its two fades is
   heard as their product; the two halves of a split play as the one clip did.
3. **Preview equals export.** A comp with a shaped crossfade and a clip effect bakes
   byte-identical through `build_plan` and `mix_decoded`.
4. **The chains.** A clip's chain runs before the track's and their latencies sum;
   `clip.fx` off drops the clip's chain alone; `switches.fx` off drops the track's alone;
   both off leave the decoded buffer pointer-equal; a clip keyframe is read from the clip's
   start and follows a slid clip.
5. **The signature** changes on a shape edit, a clip fx flip, an added clip effect and a
   layer fx flip; a project with the layer switch off and an audio plugin on now bakes
   without it.
6. **Byte-identical projects.** A clip written before the fields re-saves unchanged; a clip
   carrying them round-trips; a cut divides the fields as §2 says.
7. **Placement.** `add_clip`, `slide_clip` and `move_clip` keep the neighbour with overlap
   and overwrite it without; `move_clip` is one undo step across two layers, and onto a new
   layer when asked for one.
8. **Conversion.** Converting an Audio layer with a non-zero in point and start offset
   plays the same jobs before and after, and undo puts the Audio layer back; a retimed
   Audio layer refuses.
9. **The list rule** (pure): which layers are tracks, which are faded, in what order; a
   cross-track drag names the right target; the fade geometry maps a corner drag to seconds
   and back.
10. **The panel** (engine-backed): mounts on a comp with a track, a faded picture row and
    its Detach button, a drop that makes a track, a twirl that opens the Volume row, a
    corner drag that writes a fade, a right click that opens the fade menu, the razor
    cutting a clip, a body drag that crosses a track beside a marquee started on empty
    ground, and both timelines mounted at once with Delete reaching only the one that holds
    the keys.
11. **Budgets.** Idle rebuilds nothing and repaints no block; a scrub repaints the playhead
    layer and not the lanes; a clip drag crosses the bridge once on release, or four on the
    one gesture that converts an Audio layer; the panel survives the width sweep.
12. **Strings and glossary.** Every new string has a key with a description; the glossary
    exception for *track* is the only place the word appears outside this panel's strings.
13. **The mix mark.** A comp with an Audio layer shows no Sound mix row until it is mixed;
    showing it in the Audio timeline writes nothing, the first edit made there marks it
    once, and a second edit writes nothing more; from then on the row stands, the Audio
    layers are out of the stack and the twirl brings them back; a double click on the row
    applies the Audio preset; Convert to precomp builds one comp per track with one layer
    per clip and a mix comp with one Precomp layer per track, keeps every clip's timing,
    fades and effects, bakes a crossfade into the two clips' fade seconds, copies the
    track's rack on to each clip layer, selects the mix's Precomp layer, clears the mark,
    and one undo restores the layers and the row; the mixed output of the parent before and
    after is sample-identical; the flag round-trips serde and a comp that was never mixed
    re-saves byte-identical.
14. **Chrome and search.** The outline's chrome strip shows the playhead's timecode and
    frame count and the comp length; typing in the search box hides every track whose name
    does not contain the text and clearing it brings them back; the outline's seam overlay
    rules the same rows as the lane half.
15. **The source's start.** A left-edge drag past the source's start snaps at it and the
    box draws the triangle there; a clip whose `source_in` is zero or positive draws none.
16. **Select, rename, track rack.** A press on a track's row selects its layer; Enter opens
    the rename field and a name typed there is the layer's name; the add-effect glyph on
    the Effects heading adds to the track's stack and not to any clip's.
17. **The drag moves everything.** With a body drag, a head trim and a tail trim held
    mid-way, the header, the fade ramps, the gain line and the source mark sit at the
    shifted box, not the committed one.
18. **Clip gain.** A drag on the gain line writes `gain_db` once on release and the bake
    plays the clip at that gain under its fades; the fade ramps top out at the line; a
    cut keeps the gain on both pieces; the mix signature changes on it.
19. **Edges and source snaps.** A head trimmed in and dragged back out snaps at the source
    start and a tail at the source end, at a zoom where a frame is wider than the magnet;
    with two clips overlapped, a press in the overlap on the earlier clip's tail zone
    trims that clip and not the later one's body.
20. **Track height.** A drag on the outline row's bottom edge changes the track's rows
    within the bounds, both halves follow, and the wave and the gain line scale with it.

## 7. Ordered work packages

| # | Package | Lands |
|---|---|---|
| AT1 | The clip's fields, the shapes and the chains: `Fade`, `FadeShape`, `Clip::{fade_in, fade_out, effects, fx}`, the cut rule, `ClipFade` shapes, `AudioJob::clip_chain` and its bake in both mixers, `audio_chain_of` reading `fx`, `jobs_signature` | plans 1 to 6 |
| AT2 | The bridge: `BridgeClipFade`, `BridgeClipFadeShape`, `BridgeClip` fields and source name, `set_clip_fade`, `set_clip_fx`, `InstanceHome::Clip` with `get_clip_effects` and `add_clip_effect` and the `set_effects` clip arm, `add_clip`, `move_clip`, `slide_clip` with overlap, `convert_to_sequenced` keeping the sound and refusing a Retime; codegen | plans 7, 8 |
| AT3 | The panel: enum, title, floor, key context, preset, the two halves, tracks, faded rows and Detach (the rule moved from the Timeline with its test), lane modes, the twirl rows, the bottom bar; the layer Timeline's Sound view removed | plans 9, 10, 11 |
| AT4 | Clips: boxes, header strip, wave and `clip_audio_spectrogram`, selection, trims, slides, cross-track moves, drops, the razor and the kind gates, menus | plans 9, 10 |
| AT5 | Fades: corner handles, the drawn ramps, the presets menu, the custom editor and keep level | plans 1, 10 |
| AT6 | Clip effects: the fx toggle, the add-effect button, the drop-down rows with their lanes in clip time, the claims arbitration | plans 4, 10 |
| AT7 | Words and documents: the glossary exception and every no-overlap sentence (01 §clip, 03 §5.3, 05, 06, 07 §4.4, `sequence.rs`), 07 §1.6 and §4.2, 09 §1, §4 and §6, GUIDE's panel row, the README table, TODO | plan 12 |
| AT8 | The mix mark: `Composition::sound_mix`, `Op::SetSoundMix`, `sound_mix`, `set_sound_mix` and `precompose_sound_mix` on the bridge; the layer Timeline's row gated and folded by the mark, its double click and its menu; 03 §4, 07 §1.6 and §4.2, 09 §1 | plan 13 |
| AT9 | The second round: `slide_clip` and `trim_clip` in layer time, the drawn origin on a head trim, the mark on the first edit, the two-level precompose with baked crossfades, the chrome strip and search, the outline seams, the three-band wave, the source-start snap and triangle, track select, rename and the track rack's add glyph | plans 13 to 16 |
| AT10 | The third round: the drag carrying every part of the box, `Clip::gain_db` with `set_clip_gain` and the gain line under the fades, the track's lane band gone, source snaps at both ends and edges hit before bodies, the track height drag, the square-root wave | plans 17 to 20 |

## 8. Traps, collected

- `BridgeLayerKind::Audio` is answered for any `audio_only` layer before the kind is looked
  at, so a track reads as an Audio row to every gate that tests for a Sequence: the razor's
  branch in `timeline_razor.dart`, the Layer menu's convert and retime gates in
  `menu_bar_frb.dart`, and the panel's own track list. Each tests the clip list instead. The
  layer Timeline goes on folding a track as an Audio row, which is right.
- `slide_clip` runs `overwrite_with` unconditionally today, and `add_clip` would have. A
  drop or a drag through it deletes the neighbour a crossfade wants.
- `convert_to_sequenced` copied the layer's Retime onto its clip, which the mixer then
  skipped, and placed the clip at zero whatever the layer's in point was.
- The clip body is a raw `Listener`, not a drag recogniser: a two-axis drag inside the
  doubly scrollable lane loses the arena to the scroll views, and the press must be claimed
  before the marquee sees it. The sequence view's envelope strip says why.
- `FoldVolumeRow` is keyed at `<layer>/audio/volume`. Keep the path when the Audio heading
  is not drawn.
- The fold's rows must come from one list both halves read; a second list is a table that
  drifts.
- A retimed clip is silent (09 §7). The panel offers no speed control and no clip gesture
  on a retimed Audio layer for that reason.
- `Clip.extra` is the forward-compatibility slot, not a home for these fields.
- Never a bridge call in a build: the clip's name, fades and stack ride in on `BridgeClip`,
  and the peaks and spectrogram are fetched under a claimed key.
- `LaneBottomBar` requires a magnet; the panel's bar wants none, so the magnet becomes
  optional there rather than a second bar being written.
- The single-slot claims on the shell state have no arbitration; a panel that keeps them
  while blurred steals another panel's Delete.
- The no-overlap invariant is written in five documents and three comments in
  `sequence.rs`; `active_clip`'s first-match contract is restated for picture layers, not
  deleted.
