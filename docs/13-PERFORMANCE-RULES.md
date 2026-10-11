# Performance rules

Two rules everything here serves:

1. **The UI never waits for the engine.** Slowness shows as a degraded picture, never a
   frozen app.
2. **Degrade, never crash.** Running out of anything ends in lower quality or a calm
   pause. Never an abort.

## 1. Reference hardware

| | Desktop (mid) | Laptop (floor) |
|---|---|---|
| CPU | 4C/8T, i3-12100 class | 4C/8T, i5-1135G7 class |
| GPU | RTX 3060 12 GB | Iris Xe class |
| RAM | 16 GB | 16 GB |
| Target | 1080p60 project, 60 Hz UI | the same |

The **reference comp**, built in code by `lumit-bench`: 1080p60, 20 s, two H.264 footage
layers (one retimed to 40% with flow), text, a four-clip Sequence layer, an adjustment
layer with a LUT and curves, a glow, motion blur on two layers, a luma matte, audio with
volume keys.

The **long-form comp**, built beside it: 1080p60, 2 hours, 2,000 clips from 300 footage
items on three picture Sequence layers and four audio-only ones, with linked picture and
sound on the main rows, gaps, crossfades and a passage of twelve-frame cuts.

## 2. Budgets

95th percentile unless stated.

| # | Budget | Desktop | Laptop |
|---|---|---|---|
| B1 | UI frame during any interaction | ≤ 8 ms | ≤ 8 ms |
| B2 | Input to first visual response | next frame | next frame |
| B3 | Scrub to first (maybe degraded) frame | ≤ 50 ms | ≤ 100 ms |
| B4 | Idle to full-quality frame | ≤ 500 ms | ≤ 1500 ms |
| B5 | Warm cache playback | 60 fps, 0 drops | 60 fps, 0 drops |
| B6 | Cold playback, degradation allowed | 60 fps | ≥ 30 fps |
| B7 | Cold playback, full resolution | ≥ 24 fps | ≥ 10 fps |
| B8 | Export, 1080p60 hardware encode | ≥ 2x realtime | ≥ 0.5x realtime |
| B9 | Device loss to preview resumed | ≤ 5 s | ≤ 5 s |
| B10 | A/V sync error | ≤ ½ frame | ≤ ½ frame |
| B11 | Idle cache fill of the 20 s work area | ≤ 60 s | ≤ 240 s |
| B12 | Particulate, defaults, above the pass floor | ≲ 0.2 ms | ≲ 0.6 ms |
| B13 | Particulate, 20,000 discs | ≤ 1 ms | ≤ 4 ms |
| B14 | Particulate, 1,000,000 cap, one frame | ≤ 16 ms | |
| B15 | Puppet warp, fully covered 1080p layer | ≤ 120 ms | ≤ 300 ms |
| B16 | Puppet mesh build | ≤ 100 ms | ≤ 250 ms |
| B17 | Puppet solve at 1500 vertices | ≤ 12 ms | ≤ 30 ms |
| B18 | Long cut: comp opened cold to its first frame | ≤ 500 ms | ≤ 1500 ms |
| B19 | Long cut: naming and planning one frame | ≤ 1 ms | ≤ 2 ms |
| B20 | Long cut: playback across edit points, one frame | ≤ 16.7 ms | ≤ 33 ms |
| B21 | Long cut: decoders left open | ≤ 28 | ≤ 28 |
| B22 | Long cut: one trim committed and journalled | ≤ 16 ms | ≤ 16 ms |
| B23 | Long cut: a ripple delete committed and journalled | ≤ 16 ms | ≤ 16 ms |
| B24 | Long cut: the engine's side of the read model | ≤ 8 ms | ≤ 8 ms |
| B25 | Long cut: the mix planned again after an edit | ≤ 100 ms | ≤ 250 ms |
| B26 | Long cut: mixing one second of sound | ≤ 10 ms | ≤ 20 ms |
| B27 | Long cut: decoded sound in memory, comp open | ≤ 64 MB | ≤ 64 MB |
| B28 | Long cut: decoded sound in memory, a minute played | ≤ 512 MB | ≤ 512 MB |
| B29 | Long cut: first peaks of a file never summarised | ≤ 500 ms | ≤ 1500 ms |

The UI holds 60 fps during any interaction (16.6 ms is the floor) and budgets 8.3 ms a
frame so high-refresh screens are fed. An idle editor schedules no frames.

B18 to B24 are measured on the long-form comp and take their numbers from the rows above.
B18 is B4's wait. B19 is paid by every frame, warm ones included, so it is a small part
of B5's. B20 is B6's rate as the time one frame may take, edit points included. B21 is a
count: twelve decoders for the render, twelve for read-ahead, and the few files a fast
passage has open ahead of their clips. B22 and B23 are the 16 ms an edit commits in. B24
is built on the UI thread, so it is B1.

B25 to B29 are the same comp's sound. B25 is off the UI thread, and is how long after an
edit the new mix is heard. B26 is work per second of sound, so 10 ms is one per cent of a
core. B27 and B28 are megabytes: decoded sound is kept in two-second blocks near the
playhead under one 512 MB budget, never a whole file. B29 is B4's wait, paid once per
file: its peaks are then read from a file beside its frame index.

**Document scale.** With 200 comps, 5,000 layers, 250,000 keyframes and 2,000 footage
items open, B1 still holds, an edit or undo commits in 16 ms, it opens in 5 s and saves in
2 s without blocking. So the Timeline, Project panel and Graph editor draw only what's on
screen, and nothing in a UI frame walks every layer or keyframe. Known debt: each commit
clones the whole document.

## 3. The resource governor

One component owns memory. `lumit-budget` is the ledger.

- Defaults: VRAM 70% of a dedicated card, 40% of RAM for shared-memory GPUs. RAM 60% for
  caches, queues and buffers. Both can be changed in settings.
- Every frame-sized allocation registers its size, tier and owner. A request is granted or
  denied, and a deny starts the ladder.
- Every queue is bounded.

## 4. The degradation ladder

Steps in order, the cheapest that works, each shown in the status readout:

1. Pause background cache fill.
2. Evict cold cache.
3. Drop preview resolution (interaction and playback only).
4. Tile the frame.
5. Swap flow interpolation to blend while interacting.
6. Fall back to CPU for the offending node.
7. Pause playback with one calm, dismissible banner.

Steps reverse once pressure has been clear for about 2 s. Export ignores 3 and 5. Under
pressure it slows down, it never changes output.

## 5. Device loss

- No dispatch should get near Windows' 2 s timeout. Keep the worst case on minimum spec
  under about 500 ms.
- The renderer owns its device. On loss the worker drops it, builds a new one, refills
  from RAM and disk, republishes the frame and shows one status line.
- Still owed: the shader cache, DRED, CPU fallback after repeated loss, resuming an export
  mid-item.

## 6. Effect authors

- Declare cost class, region growth, temporal window, alpha mode and randomness honestly.
  Undeclared means worst case.
- Check the epoch between passes and tiles. Keep any uninterruptible GPU span under about
  10 ms.
- Declare peak scratch memory and allocate it through the host.
- Ship the CPU twin. It's the oracle and the fallback.
- Be deterministic. The cache depends on it.

## 7. CI

- A frame submits one command buffer however many layers it has. A test holds that.
- `lumit-bench` runs B3 to B7, B11 and B12 to B29 and compares each with
  `crates/lumit-bench/baselines/<os>.json`. It fails at 1.6x worse. Under 1 ms isn't
  ratio-gated. Regenerate a baseline by running the harness and committing the file.
- The absolute budgets are only asserted under `LUMIT_REFERENCE_HW=1`.
- B1 and B2 are measured on a real window by
  `flutter_ui/integration_test/ui_budget_test.dart`. B2 is asserted everywhere, B1 only on
  reference hardware. `rebuild_budget_test.dart` pins rebuild and paint counts per
  gesture.
- B8, B9 and B10 are manual checks before a release.
