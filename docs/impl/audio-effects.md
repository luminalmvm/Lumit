# Built-in audio effects

The engine's own audio effects: how one is declared, how it runs in the chain, what the
first suite holds and what stays out. Binding for `crates/lumit-core/src/fx/effects/audio/`,
the audio chain in `crates/lumit-core/src/fx/audio_chain.rs`, the bake in
`crates/lumit-render/src/export.rs`, and the rows the Effect controls and the Audio
timeline draw for them. Reverses [09-AUDIO.md](../09-AUDIO.md) §7's "built-in audio effects
stay out": the Audio timeline gave every track and every clip a rack, and a rack with
nothing to put in it unless a plugin is installed is not a rack. Hosted plugins keep their
own note, [audio-plugins.md](audio-plugins.md); everything here rides the seams that note
built.

## 1. Words

- An **audio effect** is a catalogue entry that processes sound and draws nothing. It sits
  on a layer's rack or a clip's rack exactly where a hosted plugin sits, keyframes the same
  way, and undoes the same way.
- The **chain** is the ordered racks a job runs through: the clip's, then the layer's
  ([audio-timeline.md](audio-timeline.md) §4).
- **Latency** is what an effect delays the sound by; the plan places the job earlier by it.
  A **tail** is what an effect keeps making after its input stops: a reverb's decay, an
  echo's repeats. The plan lets the job run on by it.

## 2. The foundation (decided)

Six gaps stand between the chain as it is and a built-in effect being a first-class
citizen. Each is closed once, in the first package, before any effect is written.

- **A category, not a name prefix.** `FxCategory::Audio` joins the enum in `fx/schema.rs`
  with its label and its `ALL` slot, and `fx_category_key` in the bridge answers `audio`
  for it. Every reader that asked `audio_plugin_id` or `is_audio_match_name` to learn
  whether an effect is sound asks the category or a `BridgeEffectInfo`'s category key
  instead, and the Dart `isAudioEffectName` goes. The bridge's namespace for a built-in
  stays `builtin`; the add-effect menu's filter reads the category key, so a built-in and a
  plugin file under one **Audio** heading together.
- **The sample rate reaches the processor.** `open_audio` gains `rate: u32`, passed by
  `chain_bake` from the rate it already holds. The plugin host stops assuming 48 kHz, which
  was wrong for a 96 kHz export.
- **A tail.** `AudioProcessor::tail() -> u32` frames, zero by default. `run_chain` runs
  `frames + latency + tail` frames, feeding silence past the input, and the plan gives the
  job that extra length at its end while still placing it `latency` earlier. Tails sum
  across a chain as latencies do. Two clips overlapped by a tail simply sum, as a crossfade
  does.
- **Denormals.** `run_chain` wraps its block loop in the flush-to-zero guard the plugin
  host already uses, once, so preview and export flush identically.
- **Modes are Int rows.** `bake_values` hands a processor Float values only, so a mode row
  is declared `ParamKind::Int` with its options named in the label's description, and a
  switch is an Int of 0 or 1. The stepped plugin parameter the host declares as Bool and
  never sends is a separate bug, recorded in [TODO.md](../TODO.md)'s audio block.
- **A logarithmic slider.** `ParamKind::Slider` gains `log: bool`; the row's thumb maps
  travel through a power curve so a frequency slider spends half its travel below 1 kHz.
  Frequency rows use it; nothing else does.

And two rules for the rows: the wet/dry row is called **Wet** (a row called Mix would trip
the blend-mode test and is the picture's word), and every audio effect declares no matte
and `is_image_op() -> false`, which the matte-row test learns to expect of the family.

## 3. The contract (decided)

The block contract of [audio-plugins.md](audio-plugins.md) §3 holds unchanged: 512 frames a
block, stereo interleaved f32, the first block from the job's first sample, coefficients
recomputed at each block's start from that block's baked values, and one-pole smoothing of
any parameter that would zipper. An LFO carries its phase in the processor's state and steps it a
frame at a time at the block's rate, never from wall time and never from a previous run, so
preview and export agree and a second bake is bit-identical. A phase worked out from the
sample index instead would jump every time an automated Rate row moved, by further the
deeper into the run the block sat. Every effect is a plain struct with its state;
`open_audio` builds it fresh for each bake. No effect allocates in `process`.

## 4. The first suite (decided)

Fifteen effects, chosen to cover what a hand arriving from Vegas Pro or Audacity looks for.
Names are the catalogue's match names; labels are sentence case.

| Effect | What Vegas and Audacity call it | DSP | Rows | Latency, tail |
|---|---|---|---|---|
| `audio_gain` | Volume; Amplify | gain in dB per channel, a polarity invert, the same silence knee the fader has | Gain, Left trim, Right trim, Invert | 0, 0 |
| `audio_stereo_width` | (utility) | mid and side: width scales the side, balance through the pan law, mono below a one-pole crossover | Width, Balance, Mono below | 0, 0 |
| `audio_eq` | Track EQ, Paragraphic EQ; Filter curve EQ | five RBJ biquads in transposed direct form II per channel, band 1 a high-pass and band 5 a low-pass by default, bells between; smoothed frequency, gain and Q | per band: On, Type, Frequency (log), Gain, Q; Output | 0, 0 |
| `audio_graphic_eq` | Graphic EQ; Graphic EQ | ten octave bands, constant-Q peaking biquads | ten gains, Output | 0, 0 |
| `audio_compressor` | Track compressor, Wave hammer; Compressor | feed-forward, log domain, peak or RMS detector on the louder channel, soft knee, branching one-pole ballistics, makeup, parallel wet | Threshold, Ratio, Knee, Attack, Release, Makeup, Detector, Lookahead, Wet | lookahead, 0 |
| `audio_limiter` | Wave hammer; Limiter | brickwall: delay line, sliding maximum over the lookahead, envelope that meets the peak on time, one-pole release; optional 4x true-peak detection | Ceiling, Input, Lookahead, Release, True peak | lookahead, 0 |
| `audio_gate` | Noise gate; Noise gate | the compressor's detector below a threshold with hysteresis, hold, and a range floor; ratio at infinity is a gate, below it an expander | Threshold, Ratio, Hysteresis, Attack, Hold, Release, Range | 0, 0 |
| `audio_distortion` | Distortion; Distortion | 4x oversampled waveshaper through a half-band pair: soft clip, hard clip, foldback, bit crush, rate reduce; a tilt for tone | Drive, Shape, Tone, Output, Wet | the half-band pair's group delay, 0 |
| `audio_delay` | Simple delay, Multi-tap delay; Echo, Delay | per-channel circular buffer with fractional read, damped feedback, ping-pong by cross-feed, tempo sync from the confirmed beat grid | Time, Note, Sync, Feedback, Damping, Ping-pong, Wet | 0, time × repeats to silence |
| `audio_reverb` | Reverb; Reverb | Freeverb: eight damped combs and four all-passes per channel, tunings scaled by rate, pre-delay, high cut | Room size, Damping, Width, Pre-delay, High cut, Dry, Wet | 0, the decay to silence |
| `audio_chorus` | Chorus, Flange; (none) | one modulated delay line, cubic read; chorus is 15 to 35 ms with voices at spread phases, flanger is 0.5 to 5 ms with feedback | Mode, Rate, Depth, Delay, Feedback, Voices, Spread, Wet | 0, 0 |
| `audio_phaser` | Flange/Wah-wah; Phaser | cascaded first-order all-passes swept by an LFO, feedback from last to first | Stages, Rate, Depth, Centre (log), Feedback, Stereo phase, Wet | 0, 0 |
| `audio_tremolo` | Amplitude modulation; Tremolo | gain by a shaped LFO, slewed square, a stereo phase offset for auto-pan | Rate, Depth, Shape, Stereo phase | 0, 0 |
| `audio_vibrato` | Vibrato; (none) | the chorus's delay line wet only, depth in cents | Rate, Depth, Shape | the line's resting read, 0 |
| `audio_wah` | Flange/Wah-wah; Wahwah | a topology-preserving state-variable band-pass swept by an LFO or by an envelope follower | Mode, Rate, Depth, Frequency (log), Resonance, Sensitivity, Wet | 0, 0 |

The graphic EQ is the parametric with ten bells and a different row layout; it is here only
because both products ship one and a hand will look for it.

What stays out, and why, each with its road if wanted later:

- **Normalise** needs the whole run before its first sample. It is a clip action, not an
  insert: a button that measures and writes a gain into the clip, `ParamKind::Action` and
  `EffectDef::press` as Looks does. A note of its own before building.
- **Pitch shift** is buildable in the contract (a phase vocoder, 2048 window, 512 hop,
  latency 1536) but is the first FFT effect in `lumit-core`, which has no FFT dependency;
  the in-house transform in `fx/fft.rs` or `realfft` is a decision to make first.
- **Time stretch** changes the duration, which an insert cannot; its road is Retime
  ([09-AUDIO.md](../09-AUDIO.md) §7).
- **Reverse** is a property of the clip, beside its Retime, not of a signal path.
- **Noise reduction** wants an FFT and a learn gesture; **click removal** wants lookahead
  across blocks; **dither** belongs on the master or in export, where a float path is
  quantised, and on a layer insert it would be wrong.

## 5. Where they show

- The **Effects & presets** browser lists them under one Audio heading beside installed
  plugins. The **add-effect menu** on a clip's header and on a track's Effects heading
  lists the same heading and nothing that draws pixels.
- The **Effect controls** panel draws their rows as it draws a plugin's: Float and Int
  rows, keyframed, driven, undone as any parameter is. No editor window.
- The Audio timeline's twirl rows draw the same rows under the track and under a clip.

## 6. Test plans (implement with each package)

1. **Determinism.** For every effect: the same input baked twice is bit-identical; the
   input processed as one run and as two runs split at a block edge, with the state carried,
   is bit-identical; a bake at 44.1, 48 and 96 kHz produces the same seconds of sound to
   within the resampling of a test tone.
2. **Latency and tail.** A click through an effect with latency lands at the same comp
   time as without it; an echo's repeats and a reverb's decay run past the clip's out point
   and stop at silence; two effects' tails sum.
3. **Sanity per effect.** Gain raises level by the dB asked; the EQ's band boost raises that
   band's energy and leaves a distant band alone; the compressor reduces the peak of a loud
   burst and leaves a quiet one; the limiter never exceeds its ceiling, true-peak on; the
   gate silences below threshold and passes above; distortion adds harmonics; the delay's
   first repeat arrives at the time asked; the chorus, phaser, tremolo, vibrato and wah
   modulate at the rate asked; stereo width at zero is mono.
4. **The chain.** A clip rack of one built-in ahead of a layer rack of one built-in runs
   in that order; `switches.fx` and the clip's `fx` bypass them; the signature changes on a
   parameter edit.
5. **Catalogue and words.** Every audio effect has a label in the engine labels and the
   arb, declares no matte, is filed under Audio by the bridge, and is listed by the
   add-effect menu's audio filter; no picture effect is.
6. **Budgets.** A four-minute track through five effects bakes within the docs/13 audio
   prepare budget; the rebake per knob drag is measured and recorded. **Measured at about
   six seconds** in a release build: four minutes of stereo through a parametric EQ, a
   compressor, a chorus, a distortion and a limiter, which is forty times real time. That
   is what one knob drag costs, because the whole placed span is baked again whenever the
   mix signature changes ([audio-plugins.md](audio-plugins.md) §3), and it is the number
   to beat when the plan streams instead of holding buffers. The reading is
   `four_minutes_of_stereo_through_five_effects` in `crates/lumit-render/src/export.rs`,
   ignored so it does not run on every build.

## 7. Ordered work packages

| # | Package | Lands |
|---|---|---|
| AE1 | The foundation: `FxCategory::Audio` and every reader of the name prefix, `rate` on `open_audio`, `tail` through `run_chain` and the plan, the denormal guard, the log slider, the matte-row and blend-mode test rules, the menu filter by category; codegen for the info field | plans 2, 5 |
| AE2 | Gain, Stereo width, Tremolo, Vibrato, the two cores every later effect shares (the biquad cascade and the modulated delay line) | plans 1, 3 |
| AE3 | Parametric EQ, Graphic EQ, Wah, Phaser, Chorus | plans 1, 3 |
| AE4 | Compressor, Limiter, Gate | plans 1, 3 |
| AE5 | Delay, Reverb, Distortion | plans 1, 2, 3 |
| AE6 | Labels, the browser and menu headings, the Audio timeline's rows, 09 §1 and §7, 12, GUIDE, the README table | plans 4, 5, 6 |

## 8. Traps, collected

- The chain's dry run (`dry_blocks`) is what a plugin's warm-up counts; a built-in warms
  up in its first block and reports none.
- A processor opened for preview and one for export must be the same struct built the
  same way; `offline` changes nothing about the arithmetic.
- The half-band pair in the distortion is linear phase and its latency is exact; report
  it, or the clip lands late.
- Freeverb's tunings are in samples at 44.1 kHz; scale them by the rate or the room shrinks
  at 96 kHz.
