# The graphics card: lumit-gpu and lumit-flow

`lumit-gpu` owns the one wgpu device, every WGSL kernel, the compositor, colour and
scopes. `lumit-flow` measures motion between two frames. Neither knows the document
model: they take plain numbers. `lumit-core` is a **dev**-dependency of `lumit-gpu`
only — that is where the CPU oracles live.

Specs: [06-RENDER-PIPELINE.md](../06-RENDER-PIPELINE.md),
[08-EFFECTS.md](../08-EFFECTS.md). Impl notes: `gpu-foundation.md`,
`anti-aliasing.md`, `lens-flare.md`, `lut.md`, `optical-flow.md`.

To learn the shader language itself, read [WGSL.md](WGSL.md).

## The device

`GpuContext` (`src/lib.rs`) wraps one wgpu `Device` + `Queue`.

- `headless()` pins the backend per OS (DX12 / Vulkan / Metal, K-205) so the
  zero-copy Viewer's low-level reach-through always finds the expected backend.
- `begin_frame`/`end_frame` batch every pass into **one** `CommandEncoder`.
  `submits_so_far()` counts submissions, so "a frame submits once, not once per
  layer" is a test, not a hope.
- `reclaim()` must run each worker-loop turn (`Maintain::Poll`). The driver frees
  dropped textures only on a maintain; skipping it looks exactly like a leak
  (K-277/K-294).
- Multisample support is asked, never assumed (`supported_sample_count`).

## How a kernel is built

No includes, no preprocessor, no string concatenation. Every `.wgsl` file is a
complete standalone module, compiled at engine construction via `include_str!`
(`src/fx/engine.rs`). Shared helpers such as `bilinear` and `unpremult` are
deliberately duplicated per file, each annotated with the CPU function it mirrors.
`tests/wgsl_validates.rs` runs naga over every shader, so a broken shader fails on a
machine with no GPU (K-263).

Four bind-group layouts cover ~37 effects:

| Layout | Bindings |
|---|---|
| shared (most effects) | 0 `src`, 1 `orig`, 2 `dst` storage, 3 uniform |
| adjust | below / processed / coverage |
| mb | src / orig / flow (also datamosh, depth of field) |
| lut | shared + 3D cube at binding 4 |

Every dispatch is `div_ceil(w, 8) × div_ceil(h, 8)`. Parameters travel as
`#[repr(C)] bytemuck::Pod` structs mirrored field-for-field with the WGSL struct,
hand-padded to 16-byte rows.

## Pixel format and colour

`WORKING_FORMAT = Rgba16Float`: scene-linear, premultiplied alpha. The only two sRGB
crossings live in `ColourEngine`, and neither contains gamma arithmetic — hardware
does it. `linearise` samples an `Rgba8UnormSrgb` view into fp16; `display` renders
fp16 into an sRGB target. A golden test proves the round trip within 1 LSB (K-031).

Viewer gain and tone map (`DisplayParams`) are preview-only and short-circuit at
neutral, so exports stay bit-identical (K-314). `oklab.rs` and `oklab.wgsl` are a
CPU/GPU pair with identical constants; OkLCh shortest-arc interpolation is the
gradient primitive (K-034). LUTs upload as `rgba32float` 3D textures and the kernel
does its **own** trilinear filter, because hardware 3D filtering is not
bit-guaranteed across cards (K-271).

## The compositor

Layers draw bottom-up as textured quads through a full 4×4 matrix, premultiplied-over
in linear light. Normal, Add and Multiply are fixed-function blend states. The other
23 modes are "snapshot" blends: the target is copied out (a copy cannot happen inside
a pass, so drawing splits into segments) and the fragment composites itself.

Motion-blur averaging and accumulation sum into `Rgba32Float` ping-pong targets and
round **once** at the resolve, so a still scene averages back bit-for-bit.

## Scopes and readback

Scopes are three compute passes (K-096): `bin` counts samples with `atomicAdd`,
`peak_reduce` takes an `atomicMax`, `colourise` paints a 256×256 trace. Only the
trace reads back.

`readback8` pads rows to `COPY_BYTES_PER_ROW_ALIGNMENT`, maps, then tightens.
`start_readback8` is the non-blocking sibling polled on later turns — cache demotion
never stalls a render.

## Oracles: how correctness is defined

Every kernel mirrors a `lumit_core::fx::cpu` function operation for operation. The
CPU version is the reference (K-019). `fx/tests.rs` uploads a quantised corpus
(gradient, alpha edge, HDR spike), runs both paths, and asserts a per-class tolerance
plus bit-stability across reruns. Tests skip without an adapter unless
`LUMIT_REQUIRE_GPU` is set; CI sets it.

Parity means **arithmetic-order parity**: explicit left-to-right reductions instead
of `mix()`, host-computed `cos`/`sin` (WGSL trig is not correctly rounded),
`floor(x + 0.5)` instead of `round()`, and `splitmix32` because WGSL has no 64-bit
integers.

## Optical flow: lumit-flow

Given frames A and B, DIS (Dense Inverse Search) computes per-pixel motion:

1. A coarse-to-fine pyramid, halving to ~24 px.
2. At each level, every 8×8 patch refines its vector by inverse-compositional
   Gauss–Newton — the template Hessian is fixed, only B is re-sampled.
3. Densify: covering patches vote, weighted by photometric fit.
4. One bilateral smooth, then variational refinement (K-332) over the whole field.
5. Forward–backward disagreement gives the occlusion mask and a confidence plane.

`src/lib.rs` is the CPU oracle; `src/dis.wgsl` mirrors it line for line and must
agree within 1e-3. `synth.wgsl` synthesises in-between frames and is bit-*tolerant*,
not bit-identical. `FlowEngine` uses the GPU where pipelines build and degrades
permanently to CPU on any GPU error. Consumers: Retime flow interpolation, Fast
motion blur, Datamosh.

## Landing soon (PR #97)

The lens flare is rewritten. After merge the pipeline is: ray-trace compute
(`fx_lens_flare_trace.wgsl`) → splat build → additive hardware raster of one small
quad per ray (`fx_lens_flare_draw.wgsl`) → Matte-mode source detection → combine.
FFTs never run on the GPU; the CPU bake arrives as textures cached by parameter hash
on its own bake thread (K-350). Bit-stability comes from barycentric-derivative
coverage instead of 4× MSAA (K-353), which also deletes a ~66 MB multisample texture.
Pipelines compile on a background thread, cutting worker start 7.7 s → 1.1 s.
Three new shaders arrive: `fx_lighting.wgsl` (the Light-layer shading pass, twin of
`lumit-core/src/lighting.rs`), `fx_light_wrap.wgsl`, `fx_sprite_flare.wgsl`.

## Traps

- **Uniform alignment is load-bearing.** WGSL aligns `array<vec4<f32>, N>` to 16
  bytes. `fx_hue.wgsl` passes a 3×3 as nine scalar fields for exactly this reason.
  A mismatch cost 17 920 fp16 ULP once.
- **Bounds-check every kernel.** Dispatch rounds up to whole 8×8 workgroups, so edge
  threads land outside the image.
- **`workgroupBarrier()` must sit in uniform control flow.** The flare blur makes
  out-of-range threads load zeroes and still reach the barrier before returning.
- **fp16 rounds where f32 does not.** Software rasterisers (lavapipe, WARP) land 1
  LSB from hardware; `GpuContext::software` relaxes bit-exact claims accordingly.
- **Fixed-function surprises.** A pipeline bakes its sample count; copies cannot
  cross sample counts; ANGLE silently refuses non-BGRA legacy share handles — a
  black Viewer with no error anywhere.
- **A long submission trips the OS watchdog** and kills the device. The lens flare
  splits command buffers at 48 M ray-surface steps.
