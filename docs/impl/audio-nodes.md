# Audio in the node graph

How a node reaches sound: the comp's mix, one layer, one clip. Binding for the
`AudioTap` trait in `crates/lumit-core/src/fx/registry.rs`, its host
`lumit_render::audio_tap::DocumentAudio`, the Audio level driver in
`crates/lumit-core/src/fx/drivers/audio_level.rs`, and the pickers the Effect controls
panel draws for them. Extends [node-graph.md](node-graph.md); the words are its words.

## 1. What exists

One node hears anything: **Audio level**, a driver with two Number outputs, Amplitude and
Low, read through the tap. Its Audio row is a layer reference; unset, it reads the comp's
mix, which is the real mixer over a window, master fader and all. Set to a layer, it reads
that layer's file raw: pre-fader, pre-rack, and only a Footage layer, so an Audio timeline
track (a Sequence layer with `audio_only`) reads silence. Nothing names a clip anywhere: no
reference kind, no field on `AudioJob`, no wire the graph validator would accept.

## 2. One reading, three filters (decided)

Every sound the graph could want is the same windowed mixdown over a different set of
jobs. So the tap gains one method and loses a special case:

```rust
/// Mono samples of the comp's mix restricted to `layer` and, within it, to
/// `clip`, each `None` meaning everything, over `half` seconds either side
/// of the frame being drawn. Post-fader, pre-rack: the layers' audio insert
/// chains are not run, so this is the dry sound at the layer's own fader.
fn strip(&self, layer: Option<Uuid>, clip: Option<Uuid>, half: f64, out: &mut Vec<f32>) -> Option<f64>;
```

`mix(half, out)` becomes `strip(None, None, half, out)`. The host builds the comp's job
list as export and playback do (`AudioJobsBuilder`), keeps the jobs whose `layer` matches
and, when asked, whose new `clip` field matches, and mixes those. A job from a nested comp
files under the outer Precomp layer, as it does for the Mixer's strips, so a row that
has become a row precomp still answers by its layer. `samples` stays as the raw
pre-fader read the existing wires use; nothing that works today changes what it reads.

`AudioJob` gains `pub clip: Option<Uuid>`, set by `sequence_jobs` and `None` everywhere
else: one field, one line.

## 3. The node (decided)

Audio level gains a **Source** row, a mode: **This comp**, **Layer**, **Clip**. Layer keeps
the Audio picker it has, now read through `strip`, so it hears the layer post-fader; the
old raw reading is what a wire made before this note reads, and it stays raw, because a
driven parameter must not change value on an update. Clip adds a second picker under the
layer's: the clip, listed by name and start time from the chosen layer's clip list, stored
as a new reference kind, `ParamKind::Clip` and `EffectValue::Clip(Option<Uuid>)`, which
degrades to unset when the clip is gone as a layer reference does. A clip picker lists the
clips of the layer above it and nothing else, so it cannot name a clip on another layer.

Two more outputs, cheap in the same pass: **Peak** and **High** (a one-pole above 4 kHz),
so a node can follow a transient or a hi-hat as well as a bass.

The comp-mix reading joins the frame key: its fingerprint folds in as a layer's does, so a
frame drawn from the mix is not served from cache after the mix has changed.

## 4. Test plans

1. `strip(None, None)` equals `mix`; `strip(Some(layer), None)` on a comp of two layers
   equals the mix of that layer alone, post-fader; `strip(Some(layer), Some(clip))` on a
   row of two clips equals that clip alone, with its fades.
2. A row that has become a row precomp answers `strip` by the Precomp layer's id with
   the same samples it gave before the conversion.
3. Audio level in Clip mode on a comp with two clips follows the chosen one; deleting the
   clip degrades the row to unset and the outputs to zero; undo brings it back.
4. The clip picker lists only the chosen layer's clips, by name and start.
5. A frame drawn with a This comp reading changes its key when a layer's volume changes.
6. Preview and export agree on an audio-driven comp in every mode.

## 5. Ordered work packages

| # | Package | Lands |
|---|---|---|
| AN1 | `strip` on the tap and its host, `AudioJob::clip`, `mix` folded into it, the frame key fingerprint | plans 1, 2, 5 |
| AN2 | `ParamKind::Clip`, `EffectValue::Clip`, the bridge's clip list for a picker, the picker row | plan 4 |
| AN3 | Audio level's Source row, the clip picker under the layer picker, Peak and High, the node-graph note's table row, 09 §5 | plans 3, 6 |

**AN3 landed 2026-09-08.** Audio level gained the Source row (*This comp*, *Layer*,
*Clip*) as a Choice rather than a number, so it draws as the dropdown §3 asks for and
no wire can land on it; the Clip picker sits under the Audio row and lists that
layer's clips alone; and Peak and High (a one-pole above 4 kHz) leave by two new
ports beside Amplitude and Low, computed in the same pass. **How an older instance is
told apart:** by its declared version **and** its Source row. The schema is version 2,
so an instance saved before the row is still version 1, and a version 1 instance whose
Source still says *Layer* goes on giving that layer the raw pre-fader reading a
parameter was driven by; `backfill_builtin_params` writes the Source row to say which
of the two readings the instance was doing and leaves the version alone. Picking *This
comp* or *Clip* on such an instance reads as it does anywhere else, because the pin is
on the reading nobody chose, not on a pair of pickers the panel draws and lets you
drag. The Clip picker greys off *Clip* and the Audio picker greys on *This comp*, so
each says when it is in charge. The frame key folds the mix fingerprint for every
reading now, not only the unset one, because every mode is the mix under a filter and
a fader anywhere in the comp can move it.
