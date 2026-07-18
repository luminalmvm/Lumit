# LUT loading and sampling — impl note (binding for its topic)

Feeds docs/08-EFFECTS.md §3.11 (the LUT effect) and the File parameter in
docs/03-DATA-MODEL.md. The specs say *what*; this note is the authoritative
*how* for the hard parts: the exact trilinear maths the CPU oracle and the GPU
shader must share, the 3D-texture upload, and caching by path.

## In plain terms

A colour LUT (look-up table) is a small cube of sample points: "when the input
colour is *here*, the output is *that*." A `.cube` file stores those samples on
an evenly spaced grid — for a size-33 LUT, 33×33×33 output colours. A real pixel
almost never lands exactly on a grid point, so we **trilinearly interpolate**
between the eight surrounding samples. The effect uploads the cube to the GPU
once as a 3D texture and looks colours up per pixel; a plain-Rust copy of the
same lookup is the oracle that proves the shader is right, exactly as every
other effect has one (docs/08 §1.6). Because the file is chosen by a path (the
File parameter), and a path cannot be blended into another path, an *animated*
LUT can only **step** between files — which is precisely the hold keyframe.

## 1. The `.cube` format (parsed by `lumit-core::lut`)

The parser (`crates/lumit-core/src/lut.rs`) is a separate building block; it
owns the text grammar and the in-memory cube. What this note pins is the layout
the rest of the pipeline relies on, so do not let it drift from the parser:

- `LUT_3D_SIZE N` gives an N×N×N cube; `LUT_1D_SIZE N` a per-channel curve of N
  points. `DOMAIN_MIN`/`DOMAIN_MAX` (default `0 0 0` / `1 1 1`) give the input
  range the grid spans. `#` starts a comment; `TITLE "…"` is ignored.
- 3D data rows are laid out with **red changing fastest**, then green, then
  blue — flat index `i = r + g*N + b*N*N`. Getting this order wrong transposes
  the cube and is the single most common LUT bug; the parser's tests lock it.
- Store samples as `f32` triplets (`[f32; 3]`) from the outset, so the CPU
  oracle and the GPU upload share one numeric type and no conversion step can
  introduce a mismatch.

## 2. Trilinear sampling — the shared maths (binding; CPU and GPU identical)

For an input colour `c` (per channel r,g,b) into a size-`N` cube with domain
`[lo, hi]`:

```
g   = (c - lo) / (hi - lo) * (N - 1)      // grid coordinate, per channel
g   = clamp(g, 0, N - 1)                   // out-of-domain clamps to the edge
i0  = floor(g)          i1 = min(i0 + 1, N - 1)
f   = g - i0                                // interpolation weight in [0,1]
```

Fetch the eight corner samples `S(i0/i1 in each axis)` by the red-fastest index
and lerp along r, then g, then b:

```
c00 = lerp(S(x0,y0,z0), S(x1,y0,z0), fr)   c10 = lerp(S(x0,y1,z0), S(x1,y1,z0), fr)
c01 = lerp(S(x0,y0,z1), S(x1,y0,z1), fr)   c11 = lerp(S(x0,y1,z1), S(x1,y1,z1), fr)
c0  = lerp(c00, c10, fg)                   c1  = lerp(c01, c11, fg)
out = lerp(c0, c1, fb)
```

Trilinear is **continuous** everywhere (the edge clamp is continuous too), so
unlike round/quantize effects it is safe under the fp16 ULP oracle. The 1D LUT
is the same with a single axis: map, clamp, lerp two neighbours per channel.

## 3. GPU path — 3D texture, but interpolate *manually*

This is the first effect to need a **3D texture** (`wgpu::TextureDimension::D3`);
every effect so far is 2D, so the `FxEngine` gains a 3D-texture bind path (an
`rgba16f` or `rgba32f` volume of extent `N×N×N`, RGB padded to RGBA, uploaded
with `write_texture` using the red-fastest row order above).

**Trap — do not rely on the hardware linear sampler.** A `filterable` 3D texture
with a linear sampler *would* give trilinear for free, but the fixed-function
interpolation precision is not guaranteed bit-for-bit across GPUs, so it will
not hold the ≤2 fp16 ULP oracle against the CPU reference. Instead **`textureLoad`
the eight integer corners and do the three lerps in the shader in f32**, byte for
byte the same as §2. The sampler is then only a nearest fetch. This keeps
preview == export (K-031) and CPU == GPU regardless of hardware.

Domain mapping, the `(N-1)` scale and the edge clamp all live in the shader
identically to the CPU. Feed the LUT its input in the effect's working space and
operate on **straight (un-premultiplied) colour** — a LUT is an arbitrary colour
map, so it must not see premultiplied values; unpremult → look up → repremult,
the same discipline the affine grades (saturation, colour balance) use, i.e.
`premultiplied: false` in the traits.

REVIEW (owner): most creative `.cube` LUTs are authored for display- or
log-encoded input, whereas Lumit works scene-linear. For v1 the LUT is applied
in the working space as-is (no implicit input transfer), documented in §3.11; a
colour-managed "LUT input space" control is a recorded follow-up. Flag this when
the effect lands so it can be logged as a K-decision.

Status (shipped, K-114) — **domain gap**: the shipped shader (`fx_lut.wgsl`)
assumes the default `0..1` domain and skips the `(c - lo) / (hi - lo)` remap
above; the CPU reference (`Lut3d::sample`) applies it in full. Almost every
creative `.cube` uses the default domain, where the two are identical — but a
cube with a non-default `DOMAIN_MIN`/`DOMAIN_MAX` currently renders with the
domain **ignored** on the GPU (wrong colours, silently) while the CPU oracle
remaps correctly. Closing this — either passing the six domain floats through
`LutParams` into the shader per §2, or refusing non-default-domain cubes at
load as a labelled no-op — is an open follow-up; until then the oracle test
only exercises default-domain cubes.

## 4. Caching by path (never re-parse per frame)

Parsing and uploading on every frame would be absurd. Cache on the path plus its
last-modified time: `(PathBuf, mtime) -> Arc<Lut3d>` for the parsed cube and a
parallel `-> GpuLut` for the uploaded texture. Look the resolved path up each
frame; parse+upload only on a miss (path changed, or the file was edited on
disk). Bound the cache (a handful of entries — a comp rarely references many
LUTs at once) and evict LRU. The parse cost is then paid once per distinct file.

Status (shipped, K-114): the shipped caches (`GpuViewer::load_luts`,
`Renderer::layer_luts`) key by **path only** — no mtime, no LRU bound — so a
`.cube` edited on disk keeps showing its old grade until the app restarts, and
distinct paths accumulate uploads for the session. Upgrading both to the
`(path, mtime)` key with an LRU bound as specified here is an open follow-up.

## 5. Animating which file is live (the File parameter)

The effect reads its file path from a `File` parameter, whose value is
`{ paths: Vec<String>, index: Property }` (docs/03-DATA-MODEL.md). The common
case is one path with a static index; an animated LUT keyframes the **index**
with **hold** keys only (§2's hold keyframe), so the resolved path steps at each
key and never tries to blend two files. Resolution picks
`paths[index.value_at(t).round().clamp(0, len-1)]`, then §4 turns that path into
the bound texture. A missing/blank path resolves to identity (the effect is a
no-op), never a panic.

## 6. Traps checklist

- Red-fastest index ordering (§1) — transposes the cube if wrong.
- Edge at `i0 == N-1`: clamp `i1` to `N-1` so the top face samples itself.
- Out-of-domain input **clamps**, it does not wrap.
- Straight-colour only; premultiplied input corrupts the lookup at soft edges.
- Guard non-finite input (NaN/inf) to a defined output; the shader and CPU must
  agree on the guarded value.
- Reject absurd `N` at parse time (memory is `N³` triplets) rather than
  allocating — the parser caps it.

## 7. Test plan (implement with the feature)

- **Parser** (in `lut.rs`): identity 2×2×2 round-trips; a channel-swap/invert
  cube matches hand-computed corners and a trilinear midpoint; 1D parse+sample;
  malformed input (missing size, wrong count, non-numeric, size ≤ 1, huge N)
  returns `Err`, never panics; non-default domain remaps correctly.
- **Oracle** `wgsl_lut_matches_the_cpu_oracle`: over random RGBA inputs
  (including partial alpha) and several LUTs — identity, a per-channel gamma, a
  saturating "film" curve — the WGSL manual-trilinear output matches
  `Lut3d::sample` to the shared fp16 ULP tolerance.
- **Identity is a no-op**: an identity cube leaves every pixel within tolerance
  (a strong end-to-end check that ordering, domain and premult are all right).
- **Cache**: two evaluations at the same path parse once; touching the file's
  mtime forces a re-parse; a blank path yields identity.
- **Determinism**: same inputs → identical bytes across runs (docs/14).

## 8. Wiring the effect into the pipeline (`Resolved` is `Copy`)

The effect stack is resolved to `&[Resolved]` (lumit-core `fx::resolve_stack`),
and `Resolved` is `#[derive(Copy)]` carrying plain scalars — it cannot hold a
`String` path or the LUT data. So the LUT is threaded the same way `flow_field`
and `neighbours` already are (`fxops::run_ops` takes them as separate params for
the effects that need them):

- **`Resolved::Lut { mix: f32 }`** carries only the mix. The resolve arm reads
  Mix; it does not touch the file (there is nowhere Copy to put it).
- **The loaded LUT is threaded alongside `ops`.** `run_ops` gains a parameter
  parallel to `ops` — `luts: &[Option<LoadedLut>]`, one slot per op, `Some` only
  for a `Lut` op whose file loaded. Its `Lut` arm binds that texture and calls
  `FxEngine::lut`; a `None` slot (unset path, or a parse/IO failure) is a
  passthrough — the §3.11 "missing file is a labelled no-op, never a fault" rule
  and the never-crash rule both fall out of this.
- **The caller prepares the LUTs.** `build_comp_draws` (preview) and the export
  renderer both already call `resolve_stack` next to the layer's `EffectInstance`
  list; for each `Lut` effect they read its File param with
  `EffectInstance::path_at("file", lt)`, run it through the cache (§4), and fill
  the parallel `luts` slot. Both paths call the one shared `run_ops`, so preview
  == export (K-031) for free.
- **CPU fallback / the oracle.** Because the LUT data never reaches the
  `Resolved`-based `cpu::apply`, that arm is a **passthrough** (the CPU
  degradation rung renders a LUT as a no-op — acceptable: a LUT is a GPU colour
  map). The CPU *reference* for the §1.6 oracle is `lut::Lut3d::sample` used
  directly in the GPU test (§7), not `cpu::apply` — the one effect whose oracle
  reference lives outside `cpu::apply`, precisely because its parameter is a file,
  not a number.

**v1 shipped subset (to log as K-114 with the effect).** The §3.11 spec lists
File, Input space, Interpolation and Mix; v1 ships **File + Mix** only, **3D
trilinear** only (Tetrahedral deferred), applied in the scene-linear working
space **as-is** (no Input-space transfer — a `.cube` authored for a different
space is applied directly, flagged for the owner). The Input-space control,
Tetrahedral interpolation, the content-hash cache key, and embedding small LUTs
in the project (K-040) are all recorded follow-ups.

## Feeds

08 (LUT effect §3.11), 03 (File parameter), 05/06 (the 3D-texture addition to
the GPU foundation).
