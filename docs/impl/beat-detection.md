# Beat detection: spectral flux onsets and the BPM grid

Feeds automatic beat markers ([09-AUDIO.md](../09-AUDIO.md)). This is well-trodden MIR
ground — implement the boring standard thing well; do not reach for a neural model.

## 1. Onset envelope (spectral flux)

Input: mono mixdown f32 48 kHz (average channels).

1. STFT: window 2048 (≈43 ms), hop 512 (≈10.7 ms → 93.75 fps envelope), Hann window.
   Use `realfft`; precompute the plan.
2. Per frame: magnitude spectrum → log compression `L = ln(1 + 100·|X|)` (tames game-mix
   dynamics), then **positive** flux summed over bins: `SF[n] = Σ_k max(0, L[n,k] − L[n−1,k])`.
3. Optional but cheap and worth it: split into 3 bands (0–200 Hz, 200–2k, 2k–24k) and keep
   per-band envelopes too — kicks live in the low band; montage editors mostly sync to
   kicks/snares, and a "sensitivity: low band" toggle beats any clever fusion.

## 2. Peak picking (adaptive threshold)

Onset at frame n when all hold:

```
SF[n] == max(SF[n−3 ..= n+3])                       // local max, ±32 ms
SF[n] ≥ mean(SF[n−43 ..= n+43]) · δ + λ             // adaptive: ±460 ms window
n − last_onset ≥ 3 frames                            // 32 ms debounce
```

δ (sensitivity, the user slider, default 1.5; lower = more markers), λ small absolute
floor (silence guard, e.g. 90th percentile of SF × 0.05). Onset time = parabolic
interpolation of the peak (sub-frame), converted to a **grid-quantised rational**
([rational-time.md](rational-time.md) §4).

## 3. BPM estimation and the grid assist

- Autocorrelate the (mean-removed, half-wave-rectified) envelope over lags 0.25–2 s
  (240–30 BPM); score lags with a comb of harmonics (lag, 2·lag, 3·lag at weights
  1, 0.5, 0.33); pick the peak, prefer the octave landing in 70–180 BPM (montage music
  norm).
- Phase: maximise comb alignment against detected onsets.
- The **grid assist** UI then offers "snap markers to grid" (replace onset times by
  nearest grid line when within 45 ms) and tap-tempo overrides the estimate. Store markers
  as `MarkerKind::Beat { confidence }` (normalised SF prominence) — regeneration replaces
  only Beat markers ([03-DATA-MODEL.md](../03-DATA-MODEL.md) §11).

## 4. Performance & placement

Runs as a background job on import or on demand per audio item; 3-minute track ≈ tens of
ms of FFTs — do not stream/incrementalise, just compute whole. Cache envelope + markers in
the sidecar `peaks/` alongside waveform data, keyed by media fingerprint + parameters.

**Where it runs (shipped).** `lumit-bridge::beats` is the worker: one dedicated thread,
one analysis at a time, jobs stamped with a generation so closing a project drops the
ones queued for it, and an inline fallback when there is no worker to hand a job to. The
mixdown is the expensive half (it decodes every audible source) and runs there too. The
worker answers **times and confidences**; `CompositionReference::detect_beats` mints the
marker ids afterwards, which is what keeps §5.4's determinism claim about the analysis
rather than about freshly generated uuids. The fingerprint-keyed sidecar cache above is
still owed.

## 5. Test plan

1. Synthetic clicks at 120 BPM ± jitter over noise: recall ≥ 0.98, precision ≥ 0.98,
   timing error ≤ 12 ms; BPM estimate within 0.1.
2. Octave sanity: half/double-tempo material lands in 70–180 preference band.
3. Ten real tracks (EDM, phonk, drum-heavy — genre norm), hand-labelled first 30 beats:
   F-score ≥ 0.85 at default sensitivity; grid assist fixes stragglers.
4. Determinism: identical file + params → byte-identical marker list (fingerprint-keyed
   cache proof).
