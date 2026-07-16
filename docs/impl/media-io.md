# Media I/O: ffmpeg, exact seeking, and hardware decode into wgpu

The media layer's hard parts: linking ffmpeg sanely, seeking long-GOP H.264/HEVC exactly,
and getting decoded frames onto the GPU with one copy. Get these wrong and you get the two
classic NLE bugs: "scrubbing shows the wrong frame" and "4K playback melts the CPU".

## 1. Linking ffmpeg

- Crate: **rsmpeg** (maintained) over ffmpeg-next (maintenance-only). Build against
  **FFmpeg 7.x shared libs**; on Windows fetch gyan.dev/BtbN release builds in CI and ship
  the DLLs (LGPL build, dynamic linking — required for GPLv3-compatibility comfort and to
  swap builds); on the dev Mac, Homebrew ffmpeg. Pin the major version; wrap all direct
  `ffi::` calls in one `luminal-media::av` module so version bumps touch one file.
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
- **v1 target (the "one copy")**, Windows: create the decoder with a D3D11 hw device ctx;
  frames arrive as `AV_PIX_FMT_D3D11` (ID3D11Texture2D array slices). Copy the slice into
  a **shared** `ID3D11Texture2D` (created with `D3D11_RESOURCE_MISC_SHARED_NTHANDLE |
  KEYED_MUTEX`), then open that NT handle on wgpu's DX12 device via
  `wgpu::hal::dx12::Device::texture_from_raw` + `OpenSharedHandle` — one GPU→GPU copy,
  zero CPU touches. Synchronise with the keyed mutex (acquire 0/release 1 convention).
  This is the fiddliest code in the media layer: isolate in `luminal-media::interop_dx`,
  feature-gated, always constructed alongside the baseline path so failure = fallback,
  not error. (Precedent: Cap ships exactly this pattern.)
- macOS dev build: VideoToolbox gives `CVPixelBuffer`; `wgpu::hal::metal` can wrap its
  IOSurface-backed `MTLTexture`. Same trait, same fallback discipline.
- **Do not** attempt Vulkan Video or NVDEC-via-CUDA in v1 (K-014 keeps CUDA per-node and
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
