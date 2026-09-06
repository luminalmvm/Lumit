# Media I/O: ffmpeg, exact seeking, and hardware decode into wgpu

The media layer's hard parts: linking ffmpeg sanely, seeking long-GOP H.264/HEVC exactly,
and getting decoded frames onto the GPU with one copy. Get these wrong and you get the two
classic NLE bugs: "scrubbing shows the wrong frame" and "4K playback melts the CPU".

## 1. Linking ffmpeg

- Crate: **rsmpeg** (maintained) over ffmpeg-next (maintenance-only). Build against
  **FFmpeg 8.x shared libs**; on Windows fetch gyan.dev/BtbN release builds in CI and ship
  the DLLs (LGPL build, dynamic linking — required for GPLv3-compatibility comfort and to
  swap builds); on the dev Mac, Homebrew ffmpeg. Pin the major version; wrap all direct
  `ffi::` calls in one `lumit-media::av` module so version bumps touch one file.
- Everything below uses libav directly (demux/decode); **never** shell out to the ffmpeg
  CLI for preview paths (process spawn per seek is where naive editors die). CLI use is
  acceptable for one-shot background proxy generation only.

## 2. The frame index (built at import, cached in sidecar)

Goal: exact mapping frame-number ↔ pts ↔ nearest-preceding-keyframe, so seeking is
deterministic ([05-ARCHITECTURE.md](../05-ARCHITECTURE.md)).

- Background job at import: `av_read_frame` loop over packets **without decoding**
  (~seconds for an hour of 4K), recording per video packet: `pts`, `dts`, `is_keyframe`
  (AV_PKT_FLAG_KEY), `pos`. Sort by pts (B-frames reorder), assign frame numbers, store as
  a flat binary table + header (codec, timebase, count, VFR flag) in `index/` keyed by
  media fingerprint.
- **VFR reality**: game captures (ShadowPlay/OBS) are frequently variable-frame-rate.
  Detect: distinct deltas > 1% of packets. Policy per
  [03-DATA-MODEL.md](../03-DATA-MODEL.md) interpretation: default *conform to the median
  rate* (frame n shows at n/rate regardless of wall-clock pts — what editors expect), with
  the true pts table retained for an opt-out. Surface a badge on the footage item.
- Audio gets a coarser index (packet pts every ~250 ms) — sample-accurate positioning comes
  from decode + skip within a packet.
- **An image sequence's index is arithmetic, and is never cached**
  ([03-DATA-MODEL.md](../03-DATA-MODEL.md) §3.1). A run of numbered stills is opened through
  FFmpeg's `image2` demuxer (pattern + `start_number` + `framerate`), so every file is one
  packet and one keyframe, evenly spaced, and how many there are was already counted off the
  directory. Read the first two packets for where the clock starts and how far it steps, then
  write the table out — the packet scan above would read *every file's bytes*, tens of
  gigabytes for a feature-length OpenEXR render, to produce a table this gives exactly. No
  sidecar either: it costs nothing to rebuild, and the sidecar is keyed by one file's
  fingerprint, which cannot tell a 300-frame run from the same run with 400 frames in it.
  Read the timebase and the step rather than assuming them — `image2` sets the stream's
  timebase from the rate it was given, but `avformat_find_stream_info` may refine both, and
  two file reads is a cheap way not to depend on it.

## 3. Seeking exactly

```
seek(frame N):
  k = index.nearest_keyframe_at_or_before(N)
  avformat_seek_file(.., ts = k.dts, flags = AVSEEK_FLAG_BACKWARD)
  avcodec_flush_buffers()
  decode forward, discarding frames with pts < index.pts(N)   // budget: GOP length
  return frame whose pts == index.pts(N)                      // exact match, not nearest
```

- Compare pts **exactly** against the index (both came from the same container); never
  "close enough" float compares — that is the off-by-one-frame scrub bug.
- Keep **persistent decoder instances per clip** with their current GOP state; sequential
  playback then never seeks. Pool cap (default 16 decoders) with LRU eviction under the
  governor.
- Backwards scrubbing: decode the whole GOP forward into a small reorder cache (GOPs are
  ≤ ~250 frames in game captures at worst, usually ~120); do not implement reverse-decode
  cleverness in v1.

## 4. Hardware decode → wgpu (the one-copy path)

Priority order per platform, all behind `trait FrameSource { fn frame(&mut self, n) ->
GpuFrame }` with the CPU path as the always-working fallback:

- **v1 baseline (ships first)**: hw decode to system memory — `d3d11va`/`videotoolbox`
  hwaccel with `av_hwframe_transfer_data` → CPU NV12 → `Queue::write_texture` upload.
  Two copies, but simple, correct everywhere, and fast enough for 1080p60 editing. Measure
  before despising it.
  - **Determinism trap (measured, 2026-07-27)**: swscale's `nv12→rgba` and `yuv420p→rgba`
    paths interpolate chroma DIFFERENTLY (≈9 % of bytes off, up to 161/255, on test-pattern
    edges), so a hardware-decoded frame converted straight from NV12 disagrees with the
    software decoder's pixels — breaking preview == export across machines. The fix
    shipped with the baseline: repack the transferred NV12 as planar `yuv420p` (a lossless
    layout change, `SWS_POINT` same-size) and run the identical conversion. Guarded by the
    `hardware_and_software_decode_agree_on_the_pixels` regression test.
  - Also shipped with it: `thread_count = 0` on every codec context — library-default libav
    is single-threaded (unlike the ffmpeg CLI), so the software fallback was grinding one
    core.
- **v1 target (the "one copy")**, Windows: create the decoder with a D3D11 hw device ctx;
  frames arrive as `AV_PIX_FMT_D3D11` (ID3D11Texture2D array slices). Copy the slice into
  a **shared** `ID3D11Texture2D` (created with `D3D11_RESOURCE_MISC_SHARED_NTHANDLE |
  KEYED_MUTEX`), then open that NT handle on wgpu's DX12 device via
  `wgpu::hal::dx12::Device::texture_from_raw` + `OpenSharedHandle` — one GPU→GPU copy,
  zero CPU touches. Synchronise with the keyed mutex (acquire 0/release 1 convention).
  This is the fiddliest code in the media layer: isolate in `lumit-media::interop_dx`,
  feature-gated, always constructed alongside the baseline path so failure = fallback,
  not error. (Precedent: Cap ships exactly this pattern.)
- macOS dev build: VideoToolbox gives `CVPixelBuffer`; `wgpu::hal::metal` can wrap its
  IOSurface-backed `MTLTexture`. Same trait, same fallback discipline.
- **Do not** attempt Vulkan Video or NVDEC-via-CUDA in v1 (CUDA stays per-node and
  optional).

## 5. NV12 → linear RGBA (WGSL, decode end of the colour rule)

One compute shader, two input planes (`texture_2d<f32>` R8 luma + RG8 chroma):

```
Y' = luma − 16/255 scaled by 255/219 (video range; honour the stream's range flag)
C = chroma − 128/255, scaled 255/224
RGB' (BT.709 matrix, the game-capture default; BT.601 for legacy SD flag or override):
  R' = Y' + 1.5748·Cr ; G' = Y' − 0.1873·Cb − 0.4681·Cr ; B' = Y' + 1.8556·Cb
linear = per-channel BT.709/sRGB EOTF decode (use the sRGB curve — captures are tagged
  bt709 but authored on sRGB monitors; this matches AE/Resolve behaviour for this footage)
out = vec4(linear, 1.0)   // premultiplied trivially, alpha 1
```

Honour `AVFrame` colourspace/range metadata when present; the footage interpretation
override ([03-DATA-MODEL.md](../03-DATA-MODEL.md) §3) wins over both. 10-bit (P010) is the
same shader with a scale factor — plumb bit depth from day one, game HDR captures exist.

### 5a. Float sources keep their depth

A source ffmpeg decodes as **float** — OpenEXR, and whatever else carries the descriptor's
`AV_PIX_FMT_FLAG_FLOAT` — never goes through the byte path above. Rounding an EXR into
`AV_PIX_FMT_RGBA` threw away both the range above white and the precision a depth pass is
made of, and it was doing so silently.

- The decode scales to **`gbrapf32le`** and packs the four planes into interleaved
  **32-bit floats**, `PixelFormat::LinearF32`. swscale has no packed float RGBA destination
  (neither `rgbaf32le` nor `rgbaf16le` is among them), so the interleave happens in
  `lumit-media::decode`. On the common case — an EXR with alpha — swscale has no conversion
  to do at all, since that planar format is already what the decoder hands back: the
  samples are copied, never converted.
- **Full float, not half.** A Z channel is the thing this carries, and half runs out of
  integer precision around 2048, which is an ordinary distance in a depth pass.
- The buffer stays one `Vec<u8>` at every stage. `PixelFormat` beside it says how wide the
  samples are, so the frame cache, the temporal neighbours and the byte budgets all work
  unchanged — there are simply four times as many bytes per pixel.
- **The upload is the linearisation.** Float pixels are already scene-linear, so
  `lumit_gpu::ColourEngine::upload_float_frame` hands back a finished working texture and
  the linearise pass does not run. With a colour space assigned the ordinary input
  transform runs on the float values as they stand, writing into the source's own format
  rather than the narrower working one — an input transform is a colour conversion, not a
  decode, and `lumit_gpu::unencoded` leaves a float format alone, which is what makes one
  pass correct for both widths.
- **`Rgba32Float` needs `FLOAT32_FILTERABLE`**, since a layer's pixels are always sampled.
  The device asks for it in the same "if the card has it" mask as the adapter's format
  features. A card without it gets the same frame narrowed to half — every stop above white
  kept, only the precision the hardware will sample — which is the softer-not-absent rule
  the multisample fallback beside it follows.

**Nothing flattens a float layer.** Each CPU stage that touches picture samples comes in a
width to match: `paint::apply_strokes_f32`, `mask::apply_masks_f32`,
`puppet::apply_puppet_f32` with `alpha_at_natural_f32`, and
`lumit_flow::FlowEngine::synthesize_at_f32` for the Flow retiming policy. The two
geometry-heavy ones — the puppet warp and the flow synthesis — are written once over a
`Raster`/`Texel` trait rather than twice, since only reading a pixel in and writing one out
ever depended on a byte.

The one thing that differs by width is *where* flow synthesis runs. The compute kernel
keeps its two frames in storage buffers of packed bytes, four to a pixel, so a float frame
does not fit it, and widening those buffers would quadruple them for the eight-bit case
that is nearly every case. A float plate retimed by Flow therefore costs a CPU pass per
frame where an eight-bit one costs a compute dispatch. The arithmetic is identical — the
synthesis has always worked in `f32` internally, and the card was only doing the same sums
faster.

### 5b. Reading OpenEXR by channel name

The Extract channels effect (docs/08 §3.97) needs two things ffmpeg cannot do: **say which
channels a file holds**, and **hand back one named channel**. Its decoder takes a `layer` and
a `part` but has no way to enumerate either, so without the list the effect's dropdowns would
be a box to type `Z` into blind. Both come from the `exr` crate, in `lumit-media::exr`;
everything else about EXR still goes through ffmpeg, including the ordinary decode.

- The **channel list** is read off the header alone, and passed through exactly as the file
  wrote it, layer prefix and all — `Z`, `N.X`, `diffuse.R`, `crypto_object00.a`. A name the
  user recognises from their render settings is worth more than a name that sorts nicely.
- The **named read** takes four channel names, red to alpha, and builds an ordinary
  `LinearF32` frame. An unfilled colour slot is black and an unfilled alpha is opaque; a name
  the file does not hold reads as empty rather than as a fault. It reads the whole file, since
  the crate's typed channel selection is compile-time shaped and these names are chosen at
  run time — an EXR with forty AOVs costs forty AOVs of memory for the moment it takes to pull
  four, which is the same order as opening the file costs anyway.
- Half, float and uint channels all arrive as float, so a colour pass in half and a `Z` in
  float and an object id in uint read alike.
- The selection travels on the decode job and is part of the **cache key**, in both the
  decoded-frame cache and `CompJob::source_key`. The same file at the same frame read as `Z`
  is not the picture it opens as, and without that the cache would hand back whichever was
  asked for first.
- The frame's own file is resolved through `Run::file_at`, so a numbered run extracts frame
  N's channels out of frame N's file. `downsample` then honours the preview width, box
  averaging rather than dropping samples — a depth pass reads as the distance across the pixel
  rather than as whichever corner was sampled.

## 6. Audio decode

Decode to **f32 interleaved at the device rate** (swr_convert to 48 kHz default), cache
decoded PCM per audio item in 1 s blocks under the governor (RAM tier only — PCM is cheap:
~0.4 MB/s stereo). Sample-accurate positioning: block index + offset; never derive audio
position from video frames ([09-AUDIO.md](../09-AUDIO.md): audio clock is master).
Peak files for waveforms: min/max/rms per 256-sample bin, two mip levels (×256, ×65536),
written to `peaks/` in the sidecar.

## 7. Encode (export)

Wrap libav encode with explicit codec selection: try `h264_nvenc` → `h264_amf` →
`h264_qsv` → `libx264` (same family for HEVC), verifying with a 16-frame test encode at
queue start, not at first real frame — hardware encoders fail late and weirdly (driver
sessions exhausted); fail over silently and log. Colour: linear fp16 → BT.709 encode
shader on GPU → readback 8-bit/10-bit NV12 → encoder. Muxing: mp4 with `+faststart`.

## 8. Test plan

1. Seek exactness: for 5 real captures (ShadowPlay VFR, OBS CFR, HEVC, 10-bit, long-GOP
   1-in-250), seek to 1000 random frames — decoded pts equals index pts, every time.
2. Conform: VFR clip, frame n renders at exactly n/rate; toggle interpretation → true-pts
   mode differs where expected.
3. Interop soak: 10⁵ frames through the D3D11→DX12 path under randomised governor pressure
   — no leaks (D3D11 debug layer clean), keyed-mutex never deadlocks (timeout + fallback).
4. Colour golden: synthetic NV12 ramps → known linear values within 1 LSB of 16-bit.
5. Throughput gate: 4K60 H.264 sustained decode ≥ 60 fps on reference hardware via the
   baseline path (hw decode, CPU copy) — proves v1 is viable even if interop slips.
6. Float depth: an EXR carrying a value four times white decodes to that value, not to
   clipped white, and an ordinary eight-bit clip still decodes to four bytes a pixel. The
   plate reaches the draw list still float, masked as well as bare — the draw list is the
   last place anything can say what the samples are before the upload picks a texture
   format. Then each CPU stage in its float form: a stroke leaves the unpainted part of an
   above-white plate untouched, an erase keeps the colour it uncovers, the puppet's sample
   and alpha lift read the right channel at the right width, and flow synthesis at half
   phase averages 2.0 and 6.0 to 4.0 rather than clamping both ends to 1.0 first. Every one
   of them refuses a raster whose length does not match its stated size.
7. Image sequences: frame N of a run decodes file N, out of order as well as
   forward; the run detected from any file of it is the same run; a gap ends it and the
   frames past the gap are unreachable rather than shifted into place; and what Lumit's own
   image-sequence export writes reads back as the run it is. Built with Netpbm fixtures —
   the one still format writable in three lines, so "does file N arrive as frame N" is
   answered without an image encoder in the test.
