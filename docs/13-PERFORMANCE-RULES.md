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

The UI holds 60 fps during any interaction (16.6 ms is the floor) and budgets 8.3 ms a
frame so high-refresh screens are fed. An idle editor schedules no frames.

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
- `lumit-bench` runs B3 to B7, B11 and B12 to B17 and compares each with
  `crates/lumit-bench/baselines/<os>.json`. It fails at 1.6x worse. Under 1 ms isn't
  ratio-gated. Regenerate a baseline by running the harness and committing the file.
- The absolute budgets are only asserted under `LUMIT_REFERENCE_HW=1`.
- B1 and B2 are measured on a real window by
  `flutter_ui/integration_test/ui_budget_test.dart`. B2 is asserted everywhere, B1 only on
  reference hardware. `rebuild_budget_test.dart` pins rebuild and paint counts per
  gesture.
- B8, B9 and B10 are manual checks before a release.
