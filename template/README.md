# The LFX plugin template

Write an effect for [Lumit](https://github.com/luminalmvm/Lumit) in C, C++ or Rust.

LFX is Lumit's native effect ABI: a frozen, typed C interface whose parameter vocabulary is
Lumit's own. An LFX effect appears in **Effects & presets** next to the built-in ones -
same categories, same search, keyframeable, expression-readable - and runs in a process of
its own, so an effect that crashes takes down nothing a person was using.

This repository is the starting point: the canonical header, three sets of bindings, one
working example per language, and the build that lays each one out as a bundle Lumit can
open.

**Licence: MIT**, deliberately more permissive than Lumit's own GPLv3, so a proprietary
vendor may adopt the header and the wrappers without licence anxiety.

---

## What is in here

| | |
|---|---|
| `include/lfx.h` | The canonical header, and the whole agreement. Copied byte for byte from Lumit's own tree, under a test that fails if the two ever differ. |
| `cpp/lfx.hpp` | A header-only C++17 wrapper over it: the describe sink, the strided value array, typed frames at both depths. |
| `rust/lfx-sys` | The same declarations as `#[repr(C)]` Rust, also a verbatim copy of Lumit's own. |
| `rust/lfx` | A safe Rust wrapper: implement one trait, list your effects in one macro. |
| `examples/exposure` | **C**, core only. Every rule the header asks of a plugin, and three lines of maths. |
| `examples/vignette` | **C++**, through `cpp/lfx.hpp`. The same rules, plus the frame geometry a per-pixel effect must get right to tile. |
| `examples/saturation` | **Rust**, through `rust/lfx`. |
| `CMakeLists.txt` | Builds the C and C++ examples and lays each out as a bundle. |
| `scripts/stage-bundle.py` | Lays a Cargo-built payload out as a bundle, which is the one step `cargo build` does not do. |

## Build it

### C and C++

```sh
cmake -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release
# build/bundles/Exposure.lfx.bundle
# build/bundles/Vignette.lfx.bundle
```

### Rust

```sh
cargo build --release
python3 scripts/stage-bundle.py \
    --payload target/release/libsaturation.so \
    --manifest examples/saturation/lfx.toml \
    --name Saturation \
    --out build/bundles
```

(`saturation.dll` on Windows, `libsaturation.dylib` on macOS.)

### Check it

```sh
lfx-validator build/bundles/Exposure.lfx.bundle
```

`lfx-validator` ships with Lumit. It opens the bundle exactly as the editor does - a broker
per bundle, the listing read with the module shut, every frame across a shared-memory ring -
and asks ten questions, printing one table row per question and exiting non-zero if any of
them came back wrong:

| suite | what it proves |
|---|---|
| layout | every struct's size prefix is one this header can read |
| describe | no unstated unit, no duplicate id, a picture family, a version that is not nought |
| lifecycle | every step driven in turn, and nothing answered after destroy |
| depth | the frame at fp16 **and** fp32, compared within tolerance |
| determinism | two runs bit-identical, and a fresh instance the same again |
| ROI | one bright pixel past the declared reach moves nothing; one tile against four |
| temporal | the declared window against what the instance asks for |
| threading | frames out of order from several threads, each the picture its own values make |
| fuzz | parameter edges, one step outside, NaN and both infinities, from one seed |
| baseline | `--baseline`: a stored digest per depth, so a moved pixel with no version bump is a refusal |

"Passes `lfx-validator`" is the bar. Run it before you ship, and run it in your own CI:
`.github/workflows/build.yml` here is a working copy of that job for all three platforms.

## What a bundle is

```
Exposure.lfx.bundle/
  Contents/
    lfx.toml
    linux-x86_64/Exposure.lfx
    win-x86_64/Exposure.lfx
    macos-universal/Exposure.lfx
```

One folder, a listing, and one directory per architecture from a closed list of seven -
`win-x86_64`, `win-arm64`, `macos-universal`, `macos-arm64`, `macos-x86_64`, `linux-x86_64`,
`linux-aarch64`. A scan tries this machine's own first, so one bundle serves every platform
you built for and a machine that gains an architecture needs nothing re-issued.

The payload's extension is `.lfx` on every platform. Underneath it is an ordinary shared
library that exports one symbol, `lfx_entry_point`.

`Contents/lfx.toml` is the listing, and it is read **before any of the bundle's code runs** -
which is what lets Lumit name, label and re-enable a plugin it has never started. It is the
cheap listing and never the authority: everything in it is compared against what the code
answers once the module is open, and a disagreement refuses the plugin. Keeping the two in
step is the whole of what it costs you.

Drop the bundle in the addons folder, or point `LFX_PLUGIN_PATH` at wherever you keep it,
and rescan from Settings ▸ Addons.

## The five rules a first plugin gets wrong

1. **Every struct opens with its own `sizeof`.** That is how a host built against a later
   header reads yours, and how yours reads a later host's.
2. **The value array is walked by `value_stride`, never by `sizeof(lfx_value)`.** The host
   may write a wider element than you compiled against; striding by your own size would read
   correct-looking kind tags over silently wrong values, and no check at run time can see it.
3. **The buffer is the definition and the region is inside it.** A frame carries its own
   top-left corner, so the first requested pixel is at `roi_x0 - origin_x`. Anything that is
   a function of *where* a pixel is belongs in the definition: work it out from the region
   and every tile gets its own geometry.
4. **Both depths are mandatory.** fp16 and fp32 are the project's choice, never yours, and
   the host never converts to accommodate a plugin. The wrappers convert for you; the C
   example writes the two conversions out.
5. **Keep nothing between frames.** The values you are handed are the only truth. Anything
   remembered that can change the picture is a stale frame the host will serve from its
   cache, and be right to - there is no opaque state in this ABI because there is nothing
   the host could hash.

And one that is not about the ABI at all: **link the maths library**. A plugin that calls
`pow` without it loads perfectly well and dies on the first frame, because the symbol is
resolved lazily. It was the first fault these examples hit.

## What version 1 does not have

The header names five extensions - `lfx.temporal`, `lfx.gpu-frames`, `lfx.overlay`,
`lfx.motion-vectors` and `lfx.audio` - and **version 1 offers none of them**. They are
reserved ids with no typed table behind them yet, and a plugin that lists one in its
descriptor's `required_extensions` is refused before it is instantiated, with the extension
named. So there is no example here that uses one: it would be a plugin this repository ships
and Lumit will not load.

Two things that sound like extensions are not, and both work today:

- **The temporal window is a trait, not an extension.** `lfx_traits.temporal_lo/hi` is the
  gate: an effect that declares a window has those frames decoded and hashed into its frame
  key. Reading the neighbouring pictures inside `process` is what waits on `lfx.temporal`.
- **`lfx.thread-unsafe` is not an extension id either.** The opt-out from instance-level
  concurrency is the `LFX_TRAIT_THREAD_UNSAFE` bit in the same trait block. It is the sole,
  discouraged opt-out, and it serialises the whole bundle.

`LFX_PARAM_PATH` and `LFX_PARAM_STRING` are in the frozen enum and refused by name in
version 1: a bezier path with no on-Viewer handles is a control nobody can edit, and the
resolved value bag carries no text at all. `LFX_PARAM_FILE` is admitted, and
`lfx_value.v.file.path` is null until the host's generic file aux lands.

## Where this is developed

The template is staged inside the Lumit repository under `template/`, so Lumit's own CI
builds all three examples and runs `lfx-validator` over each of them before any of this is
published. If you are reading it there, that is why: the copy of the header you are looking
at is held byte for byte against the one the host compiles.
