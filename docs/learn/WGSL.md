# GPU thinking and WGSL, taught from Lumit's shaders

For a developer with no GPU experience. WGSL is the shader language wgpu compiles.
It looks like Rust and behaves like C with strict rules. The language is the easy
part. The mental model is the part worth reading twice.

## 1. The mental model

A CPU loop processes pixels one after another. A GPU runs **one small program per
pixel, all at once**. Those programs share no state, and they run in no guaranteed
order.

Delphi analogy: imagine `for y := 0 to h-1 do for x := 0 to w-1 do` where the body
runs on ten thousand threads simultaneously. Each thread knows only its own
`(x, y)`. None may touch another's result during the pass.

Consequences that shape every shader in this repo:

- **You cannot read what you are writing.** Input and output are different textures.
  Multi-pass effects (blur, glow, flare) ping-pong between them.
- **There is no ordering.** If two threads write the same pixel, the result is
  undefined. Accumulation uses atomics or separate passes.
- **Branches cost.** Threads run in lockstep groups. When they disagree on a branch,
  both sides execute and results are masked. Uniform branches (same for all threads)
  are free.
- **Work is dispatched in blocks.** Lumit uses 8×8 blocks. A 100-pixel-wide image
  therefore dispatches 13 blocks, and 4 threads per row fall off the edge. Every
  kernel must check.

## 2. Anatomy of a kernel

The complete shape, in eleven lines:

```wgsl
// crates/lumit-gpu/src/fx_adjust.wgsl — `adjust_blend`
@compute @workgroup_size(8, 8)
fn adjust_blend(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(below);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let b = textureLoad(below, p, 0);
    let f = textureLoad(processed, p, 0);
    let c = clamp(textureLoad(coverage, p, 0).a * params.opacity, 0.0, 1.0);
    textureStore(dst, p, mix(b, f, c));
}
```

Line by line:

- `@compute` — a compute kernel (not a vertex or fragment shader).
- `@workgroup_size(8, 8)` — 64 threads per block.
- `global_invocation_id` — this thread's coordinates across the whole dispatch. This
  is your `(x, y)`.
- The bounds check — **mandatory**, because dispatch rounds up to whole blocks.
- `textureLoad(tex, p, 0)` — read texel at integer coordinate, mip level 0. No
  filtering, no sampler.
- `textureStore(dst, p, value)` — write the result.

`let` is an immutable binding, `var` a mutable one. Types are explicit:
`vec4<f32>`, `vec2<i32>`, `u32`.

## 3. Bindings: how data reaches the shader

Every resource is declared with its group and binding number. Those numbers must
match the Rust bind-group layout exactly:

```wgsl
// crates/lumit-gpu/src/fx_blur.wgsl — `Params`
struct Params {
    dir: vec2<f32>,     // (1,0) horizontal pass, (0,1) vertical pass
    radius: f32,        // kernel half-width, px
    sigma: f32,         // radius * 0.5, clamped ≥ 1e-3
    edge: u32,          // 0 transparent, 1 repeat, 2 mirror
    mix_amt: f32,       // 0..1, blended against `orig` (1 on the h-pass)
    _pad: vec2<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
```

That layout — `src`, `orig`, `dst`, uniform — is the shared layout most Lumit effects
use. `orig` is the unprocessed input, so the host can blend the effect at any
strength.

**`_pad` is not decoration.** WGSL aligns struct members by strict rules (a `vec2`
to 8 bytes, a `vec4` to 16). The Rust `#[repr(C)]` struct must agree byte for byte.
When it did not, one effect measured 17 920 fp16 ULP of error.

Two habits avoid it. Pad explicitly to 16-byte rows, and prefer scalar fields over
small arrays. `fx_hue.wgsl` passes a 3×3 matrix as nine separate floats, because a
uniform array strides at 16 bytes while `[f32; 9]` does not.

The same blur file also shows the classic multi-pass structure. `dir` selects the
axis, so one kernel runs twice: horizontal, then vertical. That replaces one shader
doing an O(n²) 2D kernel.

## 4. Colour, premultiplied alpha, and the working format

Lumit's working format is `rgba16float`: scene-linear, **premultiplied** alpha. Half
precision, so about three decimal digits.

Premultiplied means the stored RGB is already multiplied by alpha. Any operation that
must not tint transparent pixels has to undo that, work, then redo it:

```wgsl
// crates/lumit-gpu/src/fx_gamma.wgsl — `gamma`
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.gamma == 1.0) {
        textureStore(dst, xy, o);
        return;
    }
    let inv = 1.0 / p.gamma;
    let u = unpremult(o);
    // Clamp to >= 0 before the pow, byte-identical to the CPU reference.
    let curved = pow(max(u, vec3<f32>(0.0)), vec3<f32>(inv));
    let graded = curved * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + graded * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
```

Note the two comments citing the CPU reference. Every kernel here mirrors a
`lumit_core::fx::cpu` function, and the comments mark where exact agreement matters.

Vector operations apply per component, and `select` replaces a branch when the
condition differs per component:

```wgsl
// crates/lumit-gpu/src/composite.wgsl — `srgb_encode_c`
fn srgb_encode_c(v: vec3<f32>) -> vec3<f32> {
    let lo = v * 12.92;
    let hi = 1.055 * pow(max(v, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, v <= vec3<f32>(0.0031308));
}
```

`select(f, t, cond)` reads as "cond ? t : f". Note the argument order: false value
first. Both branches are computed. That is normal and usually cheaper than
diverging.

Swizzling — `.rgb`, `.xy`, `.zyx` — picks or reorders components. A perceptual
transform reads almost as maths:

```wgsl
// crates/lumit-gpu/src/oklab.wgsl — `oklab_hue_rotate`
fn oklab_hue_rotate(rgb: vec3<f32>, radians: f32) -> vec3<f32> {
    let lab = linear_srgb_to_oklab(rgb);
    let cs = cos(radians);
    let sn = sin(radians);
    return oklab_to_linear_srgb(vec3<f32>(
        lab.x,
        lab.y * cs - lab.z * sn,
        lab.y * sn + lab.z * cs,
    ));
}
```

## 5. Vertex and fragment shaders

Compute kernels do most of the work, but drawing still uses the classic pair. The
fullscreen pass avoids a vertex buffer entirely by generating its own triangle:

```wgsl
// crates/lumit-gpu/src/colour.wgsl — `vs_fullscreen`
@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> VsOut {
    // One triangle covering the screen: (-1,-1) (3,-1) (-1,3).
    var out: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}
```

Draw three vertices with no buffer. The vertex index alone produces one oversized
triangle covering the screen. The fragment shader then runs once per pixel.

## 6. Atomics: when threads must share

Scopes count how many pixels have each value — thousands of threads incrementing the
same bins. That needs atomics:

```wgsl
// crates/lumit-gpu/src/scope.wgsl — `bin`
    let rgb = channel_bytes(x, y);
    let bx = min((x * GRID) / p.width, GRID - 1u);

    switch (p.kind) {
        case 0u: { // luma waveform
            let by = value_row(luma8(rgb));
            atomicAdd(&counts[by * GRID + bx], 1u);
        }
        case 1u: { // rgb waveform: one grid per channel
            for (var c = 0u; c < 3u; c = c + 1u) {
                let by = value_row(f32(rgb[c]) / 255.0);
                atomicAdd(&counts[c * GRID * GRID + by * GRID + bx], 1u);
            }
        }
```

## 7. Workgroup memory: the shared scratchpad

Threads in one workgroup can share fast local memory. The flare blur uses it as a
line cache, cutting 161 texture fetches per pixel to about 3.5:

```wgsl
// crates/lumit-gpu/src/fx_lens_flare_blur.wgsl — `blur`
    let base = i32(wg.x * TILE) - i32(r);
    let needed = TILE + 2u * r;
    // Four strided loads cover the widest cache; a thread whose slot is past
    // what this radius needs simply does not load. Out-of-range rows still
    // take part — the barrier below is uniform control flow.
    for (var i = lid; i < needed; i = i + TILE) {
        var v = vec4<f32>(0.0);
        if (across < across_len) {
            v = sample_at(base + i32(i), across, len);
        }
        line[i] = v;
    }
    workgroupBarrier();
```

Read that comment carefully: `workgroupBarrier()` waits for every thread in the
group. **Every thread must reach it.** An early `return` before a barrier is
undefined behaviour. That is why out-of-range threads load zeroes and continue
rather than exiting.

## 8. Precision: why some passes use fp32

Half-float rounds. Adding many contributions in fp16 drifts, which breaks
bit-stability. Accumulation therefore sums in fp32 and rounds once, at the end:

```wgsl
// crates/lumit-gpu/src/composite.wgsl — `fs_accumulate_f32`
@group(0) @binding(6) var accum_prev: texture_2d<f32>;

@fragment
fn fs_accumulate_f32(in: VsOut) -> @location(0) vec4<f32> {
    let prev = textureLoad(accum_prev, vec2<i32>(in.pos.xy), 0);
    return prev + textureSample(src, samp, in.uv) * layer.params.x;
}

// Resolve the fp32 running sum back into the working (fp16) format — the single,
// final rounding, for downstream compositing and display.
@fragment
fn fs_copy_f32(in: VsOut) -> @location(0) vec4<f32> {
    return textureLoad(accum_prev, vec2<i32>(in.pos.xy), 0);
}
```

## 9. Matching the CPU exactly

Every kernel has a CPU twin that is the reference. Agreement is not
approximate hand-waving. It constrains how you write the shader:

- Use explicit left-to-right reductions where `mix()` or `smoothstep()` would reduce
  in a different order.
- Compute `cos`/`sin` on the host and pass them in. WGSL trig is not correctly
  rounded across vendors.
- Write `floor(x + 0.5)`, not `round()` (which rounds half to even).
- Use `splitmix32` for noise. WGSL has no 64-bit integers.
- Shared helpers such as `bilinear` and `unpremult` are duplicated in each file on
  purpose — there is no include mechanism. Each one is annotated with the CPU
  function it mirrors.

Tests upload a fixed corpus, run both paths, and compare within a per-class
tolerance. They also run the kernel twice and require identical output: bit-stability
is a property, not a hope.

## 10. Editing a shader safely

1. Find the CPU reference in `crates/lumit-core/src/fx/cpu.rs`. Change it first, or
   confirm it already says what you want.
2. Mirror the change in the `.wgsl` file, keeping arithmetic order identical.
3. If you touched a `Params` struct, update the Rust `#[repr(C)]` struct in the same
   commit. Re-check the padding.
4. Run `cargo test -p lumit-gpu -- --test-threads=1`. GPU tests are always serial.
5. `tests/wgsl_validates.rs` catches syntax and validation errors without a GPU. A
   quick `cargo test -p lumit-gpu wgsl_validates` is therefore the fastest first
   check.

## The rules worth memorising

- Bounds-check every kernel. Dispatch rounds up.
- Every thread must reach every `workgroupBarrier()`.
- Uniform structs must match the Rust struct byte for byte. Pad to 16-byte rows.
- Premultiplied alpha: unpremultiply before any per-channel curve, re-premultiply
  after.
- One texture is either read or written in a pass, never both.
- No hex colours in shaders — colours arrive in uniforms.
