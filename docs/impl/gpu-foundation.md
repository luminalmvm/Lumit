# GPU foundation: wgpu patterns that make or break the engine

How to stand up `kiriko-gpu` so everything above it stays simple. The traps here are
sRGB-vs-linear confusion, texture churn, and treating device-loss as exotic.

## 1. Device and adapter

- Request adapter with `PowerPreference::HighPerformance`; pin the backend explicitly —
  `Backends::DX12` on Windows, `Backends::METAL` on macOS. Do **not** use `Backends::PRIMARY`
  on Windows: it enumerates Vulkan and DX12 together, and on a hybrid-GPU machine wgpu can
  end up on a device that is lost on the first present (the window opens, then vanishes after
  a second). This is set on eframe's `WgpuConfiguration` in `kiriko-app` (`WGPU_BACKEND` still
  overrides for debugging). Store `AdapterInfo` — the degradation ladder and bug workarounds
  key off vendor.
- Required features: `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` not needed; do require
  `TIMESTAMP_QUERY` (profiler) and `FLOAT32_FILTERABLE` optional-with-fallback. fp16
  storage/filtering (`Rgba16Float`) is core — no feature flag needed.
- Limits: request `max_texture_dimension_2d = 16384` (matches the comp cap,
  [03-DATA-MODEL.md](../03-DATA-MODEL.md) §4); if the adapter gives less, record it and let
  the governor macro-tile ([13-PERFORMANCE-RULES.md](../13-PERFORMANCE-RULES.md)).
- One `Device` + one `Queue` for the whole app ([05-ARCHITECTURE.md](../05-ARCHITECTURE.md)).
  All submissions go through the dedicated GPU-submit thread; other threads record command
  buffers freely (wgpu command encoders are Send) and hand them over a bounded channel.

## 2. The working format, and the one sRGB rule

Intermediate frames: `Rgba16Float`, **linear light, premultiplied**, always. The only sRGB
anywhere is at the two ends:

- **Decode end**: the NV12→linear shader applies the transfer function
  ([media-io.md](media-io.md) §5).
- **Display end**: the final Viewer blit shader applies the display transform
  (linear → sRGB encode). Create the swapchain/egui target as **non-sRGB**
  (`Bgra8Unorm`) and encode explicitly in the blit — mixing `*Srgb` view formats with
  egui's own expectations is the classic double-gamma bug; one explicit encode in one
  named shader (`display_transform.wgsl`) is auditable.

Rule for every WGSL effect: sample premultiplied linear, write premultiplied linear. The
host wraps unpremultiply/premultiply around the few effects that declare they need straight
alpha ([08-EFFECTS.md](../08-EFFECTS.md)).

## 3. Texture pool

Frame-sized fp16 allocations at 60 fps will fragment VRAM and stall if you create/destroy
textures per node. Pool them:

```rust
struct PoolKey { w: u32, h: u32, format: TextureFormat, usage: TextureUsages }
struct TexturePool { free: HashMap<PoolKey, Vec<PooledTexture>>, ledger: BudgetLedger }
```

- **Round sizes up** to 64-px buckets in each axis (ROI/DoD-sized requests vary by a few
  pixels between frames; exact-size pooling would never hit). The view crops to actual size.
- Acquire → RAII guard returns to pool on drop; the guard carries the epoch so a stale
  render returning a texture is fine.
- Every acquire goes through the governor's ledger (bytes = w·h·8 for Rgba16Float);
  refusal is an `Err`, and the caller degrades — this is the mechanism behind K-018,
  build it first, not later.
- Idle trim: on governor pressure or 30 s unused, actually destroy.

Bind groups: cache keyed by (pipeline id, texture view ids, sampler ids, uniform buffer
id) in an LRU of ~1024 — egui does this internally; the effect executor needs its own.
Uniforms: one big per-frame arena buffer with dynamic offsets, not one buffer per dispatch.

## 4. Compute dispatch conventions

- Workgroup size 8×8 (portable sweet spot; 16×16 hurts on Intel).
- Every effect kernel takes a common uniform header: `roi_offset: vec2<u32>,
  roi_size: vec2<u32>, comp_scale: f32, time: f32, seed: u32` — matching the effect model's
  resolution-independent parameter rules ([08-EFFECTS.md](../08-EFFECTS.md)).
- Cancellation: GPU work is not cancellable mid-dispatch; keep dispatches < ~4 ms of work
  (macro-tile long kernels) so cancellation granularity comes from *between* dispatches —
  the checkpoint pattern in [playback-scheduler.md](playback-scheduler.md).
- Profiling: `TIMESTAMP_QUERY` write pairs around each node's dispatches, resolved into the
  per-node profiler ([13-PERFORMANCE-RULES.md](../13-PERFORMANCE-RULES.md) instrumentation).
  Gate behind a toggle: timestamps cost ~nothing on DX12, real overhead on some Metal HW.

## 5. Device loss (routine, by decision K-018)

wgpu surfaces loss as: `Device::on_uncaptured_error` callback, submissions failing, or
`SurfaceError::Lost/Outdated`. Treat them identically:

1. GPU-submit thread flips a global `DeviceEpoch` atomic and broadcasts `DeviceLost`.
2. Every in-flight render job's next checkpoint sees the epoch change and aborts quietly.
3. The pool, bind-group cache, and all VRAM cache entries are dropped **by construction**
   (they hold the old `Arc<Device>`; the tiers below them — RAM/disk — are untouched, which
   is why recovery is fast: [06-RENDER-PIPELINE.md](../06-RENDER-PIPELINE.md) cache tiers).
4. Recreate adapter/device/queue, rebuild pipelines from the (CPU-resident) shader module
   cache, re-warm lazily. Pipelines compile in parallel with `create_*_pipeline` on worker
   threads at startup and after loss; keep the WGSL source strings, not just modules.
5. If recreation fails twice, drop to the CPU path with a calm banner.

Test it in CI on Windows with `dxgi` debug tricks or by wrapping device in a test shim that
injects loss; do not ship recovery untested — it is the kind of code that silently rots.

## 6. egui integration and the Viewer

- Use `eframe` with the wgpu backend; Kiriko's renderer registers its output texture with
  `egui_wgpu::Renderer::register_native_texture` → paint as an `egui::Image` inside the
  Viewer panel. Re-register only when the texture object changes (pool swap), not per frame.
- The Viewer's checkerboard, guides, gizmos are egui painting on top; the frame itself and
  the display transform (§2) happen in Kiriko's blit **before** egui sees it, so egui's
  colour handling never touches pixel accuracy.
- Multi-viewport (detached panels) via egui's native viewports; each extra viewport shares
  the one Device.
- The neutral-surround rule ([15-DESIGN.md](../15-DESIGN.md)) is enforced here: the Viewer
  panel background comes from the theme's `viewer_surround` token, drawn by Kiriko, not a
  generic egui frame.

## 7. Test plan

1. Round-trip golden: upload sRGB test PNG → linearise → blit with display transform →
   read back → equals original within 1 LSB (catches every double-gamma).
2. Pool: churn 10⁵ acquire/release of varying ROI sizes — zero `create_texture` calls after
   warm-up (assert via counter); ledger returns to zero.
3. Device-loss drill: inject loss during a 100-node render; assert recovery < 5 s, no
   panic, RAM cache intact, identical pixels after re-render.
4. Timestamp overhead: profiler on vs off < 2% frame-time delta.
