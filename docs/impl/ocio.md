# OCIO colour management — implementation note

**Decision:** K-489 (OCIO is hosted natively; one implementation, baked artefacts),
K-490 (the v1 scope: five surfaces, the working space stays fixed, the config is a
project property).
**Related:** K-026/K-069 (the working space and depth), K-031 (preview equals export
through one colour path), K-034 (perceptual operations in Oklab), K-024 (interpretation
never touches the file), K-173 (paths are relative, absolute never serialised), K-185
(one render walk), K-303/K-005 (engine words), K-304 (three platforms gate a release),
K-479 (`ColourSpace::Ocio(name)` exists and refuses), the export colour family landing
under the concurrent export-colour work in `crates/lumit-render/src/export.rs`. This
note is the *how* for the whole of OCIO support: the hosting decision, the transform
engine, the config parser, the bake, the render paths, the seam, the UI surfaces, the
conformance plan, and the ordered work packages.

## In plain terms

Professional colour work runs on a standard called **OpenColorIO** (OCIO). An OCIO
**config** is a folder: one text file (`config.ocio`) that lists colour spaces by name —
"this footage is ACEScct", "this monitor shows sRGB" — and a set of look-up-table files
the text file points at. Studios publish configs (the ACES ones are the famous
examples), and every serious compositor can load them, so a Lumit project can agree
with a Nuke or Resolve project about what its pixels *mean*.

Lumit already has the sockets for this. Every footage item is supposed to say what
colour space it arrived in; the Viewer's colour picker names the transform the picture
is shown through; the export names the space the file is written in. Today each of
those has exactly one built-in answer (plus the built-in family the export work is
landing). Loading a config fills all three lists with the config's own names. That is
all OCIO support is: the project points at a config file, and the config's vocabulary
appears wherever Lumit already asks a colour question.

The one decision that shapes everything else is **who does the colour maths**. The
official OCIO library is C++, and it deliberately computes differently on the CPU and
the GPU — good enough for film work, but Lumit's foundational promise is that the
preview *is* the export, bit for bit (K-031), and a library with two answers cannot
keep that promise for us. So Lumit implements the config format itself, in Rust, the
same way it hosts OpenFX itself (docs/impl/ofx-host.md) and parses `.cube` files
itself (docs/impl/lut.md): every transform a config describes is resolved once, on the
CPU, into a small **baked table** (a shaper curve and a 3D look-up cube), and that one
table is what both the Viewer and the export sample. One implementation, one answer.
Whether our answer matches the *official* one is not left to hope: a suite of golden
fixtures — inputs and expected outputs generated with the reference library, checked
into the repository — gates every transform we claim to support (§7).

## 1. The hosting decision (K-489)

The fork was: (a) link the official OpenColorIO C++ library, or (b) implement the
config format's core natively in Rust. **Native Rust**, for four reasons, recorded in
the order they weigh:

1. **K-031 is structural, not aspirational.** OCIO v2 ships a CPU path (exact ops) and
   a GPU path (generated shader text, optionally baked to LUTs), and documents that
   they differ. Wiring the library in honestly would mean either running its CPU path
   per pixel per frame in the Viewer (unaffordable) or accepting that preview and
   export diverge (forbidden). Extracting only its resolved transforms and baking them
   ourselves — the workable middle — already means writing the bake, the sampling
   shader, and the CPU oracle natively; at that point the library is being used as a
   config parser with a very large build attached.
2. **Build weight on three platforms (K-304).** OpenColorIO drags yaml-cpp, Imath,
   pystring and minizip-ng, needs CMake and a C++17 toolchain in CI on Windows, macOS
   and Linux, and would be the workspace's second foreign build system after ffmpeg —
   which is already the single most painful seam in the repository. A parser and some
   arithmetic do not justify a second one.
3. **Precedent.** Lumit hosts the OpenFX C ABI from Rust rather than linking a hosting
   library, and parses `.cube` LUTs in `lumit-core::lut` rather than linking one. The
   repository's standing answer to a foreign spec is "implement the spec, gate it with
   tests", and OCIO's config format is a documented spec.
4. **Determinism.** A native bake is pure Rust `f32` arithmetic in a fixed order:
   the same config bakes to the same bytes on every machine, so artefacts can be
   content-hashed into the frame key (§5.5) and CI can assert exact CPU-side values.
   A C++ library's output is stable per version but is a black box we re-validate on
   every upgrade anyway — via exactly the golden fixtures that make the native engine
   safe in the first place.

Licence was checked and is not decisive: OpenColorIO is BSD-3-Clause, compatible with
GPLv3 either way.

**The honest cost** is fidelity risk: a native subset can mis-implement an op, and the
config format has corners (§4.4). The mitigation is not care but proof: no transform
class is claimed until its golden fixtures pass (§7), and a config that uses anything
outside the implemented set is **refused by name** — "this config needs
`FixedFunctionTransform`, which Lumit does not support yet" — never approximated. A
wrong picture that looks plausible is the one failure mode this design refuses to
ship.

## 2. Scope of v1 (K-490)

All five surfaces ship together, because each is small once the core exists and a
colour pipeline with a missing edge is worse than none:

| Surface | v1 | Where |
|---|---|---|
| Config loading | Yes — a project setting: path to `config.ocio`, LUTs resolved via the config's own `search_path` | §3.1 |
| Working space | **Stays fixed**: scene-linear Rec.709/sRGB primaries, fp16, premultiplied. OCIO at the edges | §2.1 |
| Footage input transforms | Yes — per-item colour space assignment from the config's list | §3.2 |
| Viewer display/view | Yes — the colour-pipeline picker lists the config's displays and views | §6.2 |
| Export output transform | Yes — `ColourSpace::Ocio(name)` stops refusing when the name is the loaded config's | §6.3 |

Recorded next, deliberately not v1: a config-defined working space (§2.1), context
variables (`$SHOT`-style per-item LUT substitution), `FixedFunctionTransform` and the
grading-op family, 3D-LUT inversion, and OCIO *looks* as a separate picker (a view
that bakes a look in works today; a standalone look chooser is UI once someone needs
it).

### 2.1 The working space stays fixed — said outright

The tempting alternative is what Nuke does: the working space *becomes* the config's
`scene_linear` role, ACEScg under an ACES config. Lumit does not do this in v1, and
the reason is honest rather than convenient: the engine hard-codes its primaries in
places that are correct today and would silently become wrong. `lumit-gpu::oklab`'s
matrices assume Rec.709 primaries (K-034); the perceptual blend modes and the scopes
take Rec.709 luma of an sRGB encode (docs/06 §3.5); the tone-map curve, the NV12
decode target and the hardware sRGB texture trick (docs/impl/gpu-foundation.md) all
assume the same. A config-defined working space is a real feature — it is the ACES
pipeline native — but it is "make every primaries-dependent constant working-space
aware", a change with its own decision entry, not a side effect of loading a file.

So v1 is **edges-only**, exactly the shape the built-in export family landed in: fixed
linear-Rec.709 working space, config transforms converting in and out at the borders.
Two consequences, both stated rather than hidden:

- **Bridging to the config's reference space.** A config describes transforms to and
  from its own reference space, not ours. When the config declares the OCIO v2
  interchange roles (`aces_interchange` or `cie_xyz_d65_interchange`), the bridge is
  exact: input chain = (space → reference) ∘ (reference → interchange) ∘ (fixed
  Bradford-adapted matrix, interchange → linear Rec.709 D65); the display chain is the
  mirror. When a legacy config has no interchange role, Lumit **composes through**:
  it treats the config's `scene_linear` role as the working space's equal, so any
  input→working→display trip is still end-to-end exact (the middle cancels), and only
  Lumit's own perceptual ops (Oklab, perceptual blends, luma) read the pixels as if
  they were Rec.709 when they may be, say, ACEScg. That is precisely what every OCIO
  v1-era host did; the project settings face states which mode is in force (§6.4).
- **Wide gamut rides as negatives.** Converting a wider-gamut input to linear Rec.709
  produces negative components for colours outside Rec.709. fp16 carries negatives
  natively and the compositing maths is linear, so nothing is lost in the working
  image — the loss point is the baked view LUT's domain, which is the note's biggest
  fidelity bound (§5.4).

## 3. The model

### 3.1 What the document gains

```rust
struct Document {
    // ...
    colour: ColourManagement,   // serde default; absent from the file when default
}

/// The project's colour management (K-490). A project property, not a
/// preference, for the same reason anti_aliasing is: it changes what a comp
/// looks like, so it travels in the `.lum` and matches on another machine.
#[derive(Default)]
struct ColourManagement {
    /// The OCIO config file. None = the built-in family only, exactly today's
    /// behaviour. A MediaRef, deliberately: relative path saved, absolute
    /// never serialised (K-173), fingerprint relink for free.
    config: Option<MediaRef>,
}

struct FootageItem {
    // ...
    /// The colour space this footage arrives in, by the config's name.
    /// None = the built-in interpretation defaults (docs/06 §3.2: video
    /// Rec.709, stills sRGB, container metadata wins). Kept when the config
    /// is missing — names are never dropped on the floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    colour_space: Option<String>,
}
```

`MediaRef` rather than a bare path is the load-bearing choice: the relative-path
serialisation rules, the never-write-the-absolute-path promise and content-fingerprint
relink all exist and are tested; a config that moved with its project keeps working,
and one that moved elsewhere relinks through the machinery footage already uses.

Per-item assignment is a **field on `FootageItem`**, not a map beside the items (the
`item_labels` pattern): unlike a label it changes pixels, it applies to exactly one
item kind, and the glossary already promises the footage item carries its
interpretation settings. Serde default + skip keeps every older `.lum` byte-identical
on round-trip.

**Ops**: `SetColourConfig { config: Option<MediaRef> }` and
`SetFootageColourSpace { item, space: Option<String> }` — ordinary document ops,
undoable, journalled. No new op shapes.

### 3.2 Loaded state is derived, never stored

The parsed config, the resolved transform chains and the baked artefacts are all
derived state, rebuilt from the file at load and cached in RAM by content hash —
exactly as decoded footage is. The document stores *names* only. Consequences that
fall out for free: undo/redo never re-parses (the cache holds), two documents naming
the same config share one parse, and a config edited on disk is picked up by the same
change-detection route footage uses.

### 3.3 The calm degrade story

A missing, moved, unreadable or refused config must never hold the project hostage:

- **The project opens.** Assignments, the export's named space and the picker choice
  all keep their names.
- **Rendering falls back to the built-in family** — the exact pre-OCIO pipeline, so a
  frame is always produced and is honestly labelled (the Viewer picker's face shows
  the missing state, §6.2).
- **Export refuses, by name** — `ColourSpace::Ocio(name)` with no loaded config keeps
  K-479's behaviour: a wrong colour space in a delivered file is worse than an export
  that did not run. Preview degrades, delivery refuses; that asymmetry is deliberate.
- **A refused config says why** (the unsupported op, the missing LUT file, the parse
  error), in one calm sentence on the project settings row (§6.4), and behaves exactly
  like a missing one otherwise.

## 4. The transform core (`lumit-colour`)

A new engine crate, `crates/lumit-colour`: no GPU dependency, no I/O beyond reading
the files it is handed paths to, allocations budgeted (docs/14). `lumit-core::lut`
keeps owning the `.cube` grammar; `lumit-colour` depends on it rather than re-parsing.

### 4.1 The op set

The transforms real configs are made of, each with forward and (where noted) inverse:

| Op | Config forms | Inverse |
|---|---|---|
| Matrix | `MatrixTransform` (3×4: 3×3 + offset) | exact |
| Exponent | `ExponentTransform`, `ExponentWithLinearTransform` (sRGB-style linear toe) | exact |
| Negatives | the `style` key on either exponent form: `mirror`, `pass_thru` | exact |
| ST 2084 | the PQ display encodings' curve, `Op::Pq` | exact |
| Log | `LogTransform` (base only), `LogAffineTransform`, `LogCameraTransform` (lin-side break) | exact |
| CDL | `CDLTransform` (slope/offset/power + saturation, ASC ordering) | exact except clamp |
| Range | `RangeTransform` (scale + clamp) | exact on the non-clamped part |
| 1D LUT | `FileTransform` → `.spi1d`, `.cube` 1D, CLF `LUT1D` | by monotone bisection (§4.3) |
| 3D LUT | `FileTransform` → `.spi3d`, `.cube` 3D, CLF `LUT3D`, **tetrahedral** interpolation | **refused** (§4.3) |
| Group | `GroupTransform` — ordered children | children reversed, each inverted |
| Space-to-space | `ColorSpaceTransform` — resolved through the reference space | by construction |
| Display/view | display + view → the view's transform (+ its look's transform if the view names one) | n/a |
| Builtin | `BuiltinTransform` — a *named* transform implemented in code, not data | per style |

Three notes on that table, each one a real config's doing rather than a design flourish.

**Matrix coefficients are `f64`, and neighbouring matrices fold** (K-516). A config states
one direction of a space and Lumit inverts it for the other, so a space-to-space chain
routinely carries a matrix immediately followed by its own inverse. Walking that pair in
single precision, rather than composing it away, cost 2 × 10⁻⁵ on a real ACEScc → ACEScg
row. A composition that comes out as the identity is dropped from the chain altogether,
because a kept near-identity still spreads one overflowed channel across the other two.
Evaluation is unchanged: `matrix::apply` rounds to `f32` first, so the processor and the
graphics card multiply the same twelve numbers.

**A curve carries its own reading of negatives** (K-517). `Op::Negatives` wraps a curve in
`mirror` (apply to the magnitude, put the sign back) or `pass_thru` (carry it through).
Every display encoding in the ACES v2 configs mirrors, including the ones whose names do
not say so — and the key a config *file* writes is `style`, not the API's `negativeStyle`,
which is a distinction worth a line here because reading the wrong one finds nothing,
applies the default, and is invisible above zero.

**ST 2084 is an op, not a bake** (K-517). The reference library expresses the PQ display
encodings as a 65536-entry half-domain table, but that table is the standard's published
formula and four published constants, so it lands in tier one with the other curves.

`BuiltinTransform` is the honest hard part: the OCIO v2 ACES configs express their
output transforms as code names ("ACES-OUTPUT — … SDR-VIDEO"), not as data. v1 handles
them in two tiers: styles that are documented matrix/curve compositions are
implemented directly; the ACES output-transform styles are shipped as **vendored
reference bakes** — high-resolution shaper+cube artefacts generated offline with the
reference library, provenance (library version, generation script) recorded in the
fixture header, checked in like any golden data. Exact Rust ports of those styles are
recorded follow-up work, one style at a time, each landing against the same fixtures.
A `BuiltinTransform` style in neither tier refuses the config by name. Note the
happy accident of history: the *legacy* ACES configs (1.0.3/1.2, the most widespread)
are pure config-data — matrices, logs and `.spi1d`/`.spi3d` files — and need no
builtins at all; the file-transform path covers them wholesale.

**Both tiers are populated** as of the ACES CG drop. Tier one holds fourteen styles:
`IDENTITY` and `pass_thru`, the three ACES log/primaries conversions, the AP0→XYZ utility,
and all eight display encodings the CG config names. Tier two holds five artefacts — the
four ACES 2.0 output transforms and the ACES 1.3 Reference Gamut Compression LMT — at 47
MiB in `crates/lumit-colour/vendored/`, read at runtime from a `colour/` data directory
shipped beside the executable (K-527: `data/colour/` on Windows and Linux,
`Contents/Resources/colour/` on macOS, the crate's own `vendored/` in a development
checkout) rather than compiled into every binary; a style whose file is absent refuses by
name, exactly as if it had never been vendored. A vendored
artefact is turned into ordinary chain steps rather than executed specially: the lg2
shaper *is* a log curve with a lin-side offset and the cube *is* a 3D table, so a bake
composes with a display encoding like anything else. **What tier two costs at the gamut
edge is measured and stated in K-518 and §5.4** — 0.117 at the Rec.709 blue primary — and
it is the number the Rust ports exist to reduce.

Everything else a config can name — `FixedFunctionTransform`, `GradingTone`-family,
`ExposureContrastTransform`, `AllocationTransform`-as-op, context variables in file
paths — is **refused by name** in v1 and listed in the refusal taxonomy test (§7).

### 4.2 Resolution

A config's colour space declares `to_reference` and/or `from_reference` (v1 grammar:
`to_scene_reference`/`from_scene_reference` in v2 configs); the missing direction is
the declared one inverted, child by child in reverse. Resolving "space A → space B"
concatenates A's to-reference chain with B's from-reference chain. Roles are one
indirection (name → space) resolved at parse. The result of every resolution is a
flat, ordered `Vec<Op>` — the **resolved chain** — which is what gets baked (§5) and
what the conformance tests evaluate exactly.

Determinism rules: resolution order is the config's declaration order; every map is a
`BTreeMap`; f32 arithmetic throughout with no fused-multiply-add so CPU results are
identical across platforms (docs/14's determinism clause; FMA is the classic source of
last-bit drift).

### 4.3 The maths that must not drift

**Tetrahedral 3D interpolation** (binding; the CPU oracle and the WGSL shader use this
formulation byte for byte). Grid coordinate per channel as in docs/impl/lut.md §2
(`g = clamp((c-lo)/(hi-lo)·(N−1), 0, N−1)`, `i0 = floor(g)`, `f = g − i0`). With the
cell's eight corners `c_xyz` fetched red-fastest, pick one of six tetrahedra by
ordering the fractions:

```
if fr ≥ fg ≥ fb:  out = c000 + fr·(c100−c000) + fg·(c110−c100) + fb·(c111−c110)
if fr ≥ fb ≥ fg:  out = c000 + fr·(c100−c000) + fb·(c101−c100) + fg·(c111−c101)
if fb ≥ fr ≥ fg:  out = c000 + fb·(c001−c000) + fr·(c101−c001) + fg·(c111−c101)
if fg ≥ fr ≥ fb:  out = c000 + fg·(c010−c000) + fr·(c110−c010) + fb·(c111−c110)
if fg ≥ fb ≥ fr:  out = c000 + fg·(c010−c000) + fb·(c011−c010) + fr·(c111−c011)
else:             out = c000 + fb·(c001−c000) + fg·(c011−c001) + fr·(c111−c011)
```

Ties must break identically in both implementations — use `≥` exactly as written, top
branch first. Tetrahedral rather than trilinear because it is the industry's reference
for colour cubes (it preserves the neutral axis exactly when the cube does, which
trilinear does not) and it is what the golden fixtures are generated with. The
existing LUT *effect* keeps its trilinear (that is its own recorded contract);
`lumit-colour` owns the tetrahedral sampler and the LUT effect can adopt it later
under its own entry.

**1D LUT inversion**: an inverse-direction 1D LUT is evaluated by bisection over the
forward curve, with non-monotone curves refused at parse (flat runs take the lower
edge, matching the reference). **3D LUT inversion is refused**: the reference
implementation's is an iterative approximation with its own error bound, and a wrong
"exact" is worse than an honest no. Configs hit this only when a space whose
*to-reference* is a 3D LUT is used in the from-direction — name the space in the
refusal.

**The `.spi1d` grammar** (the one format not already parsed): a small text header —
`Version`, `From <lo> <hi>` (the input domain), `Length`, `Components` — then one
sample per line. Domain from the `From` line, not assumed [0,1]; 1–3 components,
one-component curves applied to all three channels. `.spi3d`: header then
`r g b → R G B` sample lines with **indices in the file**, red-fastest storage as ever.
CLF (`.clf`, and `.ctf` same grammar): XML; v1 reads `LUT1D`, `LUT3D`, `Matrix`,
`Range`, `Log`, `Exponent`, `ASC_CDL` process nodes — the same op set — with
`rawHalfs` and `halfDomain` refused by name; the CLF spec's own test files are part of
the conformance suite (§7).

### 4.4 Parsing the config file

OCIO configs are YAML with custom tags (`!<ColorSpace>`, `!<MatrixTransform>`, …).
`serde_yaml` is archived and handles tags awkwardly; the pinned choice is
**`yaml-rust2`** (pure Rust, maintained, event/tree API that exposes tags directly),
walked by hand into `lumit-colour`'s own structs — the config grammar is small enough
that hand-walking is less code than fighting a derive. Traps, so they are not
re-derived: YAML anchors/merge keys are legal and appear in real configs (the parser
must resolve them, `yaml-rust2` does); `search_path` is colon- or list-separated and
relative entries resolve against the config file's directory, in order, first hit
wins; `ocio_profile_version` gates grammar (accept 1 and 2; refuse higher by name);
`inactive_colorspaces` hides spaces from lists but keeps them resolvable; a
`file_rules`/`default` block is v2-optional and ignored in v1 (assignment is manual);
display names and view names are separate namespaces from colour space names.

Config-supplied names (spaces, displays, views) are **user data, not engine strings**:
they cross the bridge verbatim and get no `app_en.arb` keys, exactly like file names.
The K-303 gate applies to Lumit's own new labels only (§6.5).

## 5. The bake, and where transforms run

### 5.1 One execution form per edge

Everything the pipeline executes is one of two **baked artefact** shapes, produced at
config load on the CPU:

- **Factorised** — for chains containing only channel-independent ops (exponent, log,
  1D LUT, range) and matrices: a per-channel forward curve sampled at 16385 points +
  a 3×4 matrix, in the fixed shape **curve → matrix → curve** (any slot may be
  absent). This is the preferred input-transform form, and camera IDTs — transfer
  curve then primaries matrix — factorise by construction.

  The curve is **not** sampled evenly across [0, 1]. It is sampled through a *signed*
  lg2 shaper fixed at `min_log2 = −8`, `max_log2 = 16`, `offset = 2⁻⁸`, mirrored about
  zero — so one table covers ±65536 (everything the fp16 working format can hold),
  densely near black and sparsely out in the tail, with linear zero on an exact grid
  sample (hence the odd count). Values beyond ±2¹⁶ clamp, as the cube form's do.

  **Why not evaluate the ops analytically outside the sampled range** — the shape WP1
  first landed, and the seam it left open. A tail computed from the live ops is exact
  on the CPU and unreachable on the GPU: the shader would have to re-implement every
  logarithm, power and 1D table in the chain *and* agree with Rust's transcendentals,
  and a 1D table inside the chain has no analytic form at all. Preview would then
  differ from the CPU oracle exactly where wide-gamut negatives and HDR highlights
  live, which is where a colour pipeline is judged. Sampling the tail instead makes
  both sides run the same two lines — `shaper.forward_signed`, then the table lerp —
  so K-031 holds off the end of the range as well as inside it. The cost is stated in
  §5.4 and it is small; the alternative had no cost that could be stated at all.

  A chain that would factorise into more stages than `curve → matrix → curve` takes
  the cube form instead. Real chains never do; the guard exists so the shader has
  three fixed slots and the choice is made **once**, in the bake, where both the CPU
  and the GPU read it.
- **Shaper + cube** — for everything else (views, CDLs, 3D file LUTs): a shaper curve
  mapping working-linear to [0,1], then a 65³ RGB cube sampled tetrahedrally, output
  fp16. The shaper is the lg2 allocation `y = (log2(x + o) − log2(o)) / (hi − log2(o))`
  with `o = 2⁻⁸` and `hi = 5` by default (covering linear 0 → 32 with log-even grid
  spacing; values above clamp to the top grid plane, values below −o clamp to the
  bottom), overridden by the space's own `allocation` vars when the config declares
  them — the config author's statement of the domain beats our default.

Choosing is mechanical: factorise when every op in the resolved chain is factorable,
else bake the cube. Both forms have one CPU sampler and one WGSL sampler sharing the
manual interpolation maths (§4.3, docs/impl/lut.md §3's `textureLoad`-and-lerp rule —
never the hardware filter, which breaks CPU/GPU equality).

### 5.2 Where each edge runs

- **Input transforms** run where linearisation already runs: the decode compute pass
  (docs/06 §3.2) applies the item's artefact instead of the fixed sRGB/Rec.709 curves
  — same pass, new tables, still no CPU round trip. Stills uploaded through
  `upload_srgb8` take a linearise-pass variant with the same artefact. **Every
  source of image content takes it**, not only a layer's own picture: a track
  matte's source and a layer input's plate (a Light wrap background, a Texturize
  texture) carry their own item's space on the draw and linearise through it, so
  log footage gates and wraps as what it is. A mask's coverage raster does not —
  it is a shape's alpha drawn from the mask geometry, not content that arrived in
  anybody's colour space.
- **The display/view transform** runs where the display transform already runs: the
  `ColourEngine` display pass (`colour.wgsl`), as a pipeline variant binding the
  shaper (1D texture) and cube (3D texture) after the existing exposure/tone-map
  block. **Trap — the render target**: the built-in pass writes linear values and
  lets the `Rgba8UnormSrgb` target encode; a baked view's output is *already*
  display-encoded, so the OCIO variant must write through an `Rgba8Unorm` view of the
  same texture (create the texture with both view formats) or the hardware encodes
  twice and everything washes out pale. This is the kind of bug that looks like a
  subtle grading error; the parity test catches it because the CPU oracle does not
  double-encode.
- **Export runs the same pass.** The export path already reads back the
  display-encoded frame from the same `ColourEngine` blit the Viewer presents
  (K-185: there is one walk). `ColourSpace::Ocio(name)` resolves to a baked artefact
  and the export's blit binds it exactly as the Viewer's does — preview equals export
  because they are the same dispatch, not because two implementations agree. The
  prompt-level alternative — a separate CPU transform at the pack stage — would be
  deterministic too, but it would be a *second* implementation of the same transform
  in the delivery path, which is the exact structure K-031 exists to forbid. The CPU
  sampler still exists, as the oracle every effect already has (docs/08 §1.6) and as
  the conformance suite's engine; it is simply not the exporter.
- **Container tagging**: the concurrent export work tags containers for the built-in
  family. An OCIO name has no reliable primaries/transfer metadata in general, so a
  file exported through one is written **untagged** rather than mis-tagged; the known
  ACES display/view names that correspond exactly to a built-in tag may reuse it, one
  explicit table entry at a time.

### 5.3 Budgets and caching

An artefact is small (cube: 65³ × 4 × 4 B = 4.4 MiB as an `Rgba32Float` texture;
curves: 16385 × 4 × 4 B = 256 KiB per stage, uploaded as rows of 1024) and baking one
is 275k chain evaluations — milliseconds. f32 rather than fp16 on the card, so the CPU
oracle and the shader compare against each other tightly rather than across a
conversion; 4 MiB of texture is not worth a looser gate. Bakes happen off the render thread at config
load or first use, cached in RAM keyed by `(config content hash, transform identity)`,
counted under the existing allocation budgets. GPU uploads ride the existing 3D/1D
texture paths and the texture pool.

### 5.4 The error bound — and the biggest fidelity risk, named

For the factorised form the error is sampling density only: ≤ 1 × 10⁻⁵ **relative**
against exact evaluation for smooth curves at 16385 points, across the whole signed
range (asserted, not assumed — relative because the range now reaches 2¹⁶, so an
absolute bound would be measuring the range rather than the error). For the shaper +
cube form the bound is interpolation error on a 65-point log-spaced grid: for the
smooth view transforms real configs ship, ≤ 2 × 10⁻³ on the display-encoded [0,1]
output — half an 8-bit code value — asserted in-domain by the golden suite (§7).

**Measured, WP6, dense sweeps rather than sample points** (`bake.rs`'s bound tests
carry these numbers and the sweeps that produce them):

| What | Bound | Measured |
|---|---|---|
| Factorised, the curve stage, signed sweep of ±32 | 1 × 10⁻⁵ rel. | 7.3 × 10⁻⁶, worst near x = 20 |
| Factorised, whole curve-then-matrix chain, same sweep | — | 1.9 × 10⁻⁵ rel., worst near x = 25 |
| Shaper + cube, neutrals in domain | 2 × 10⁻³ | 1.6 × 10⁻⁴ |
| Shaper + cube, mild colour in domain | 2 × 10⁻³ | 2.9 × 10⁻⁴ |
| Shaper + cube, deep saturation (x, x/4, x/20) | 5 × 10⁻³ ceiling | 2.2 × 10⁻³ |
| Shaper + cube, harsher saturation (x, x/20, 0) | *none stated* | 5.6 × 10⁻² |

**And four more, measured against the reference library rather than against exact
evaluation** — the reference fixtures found these, and each is a fact about a curve family
the sweeps above never contained rather than a loosening of a promise about the same one:

| What | Bound | Measured |
|---|---|---|
| Factorised, a chain leaving an ACES **log** encoding | 2 × 10⁻⁴ rel. | 1.4 × 10⁻⁴ |
| Factorised, a gamma **encode** below the shaper's first cell | 5 × 10⁻³ | 6 × 10⁻⁴ |
| Cube, past the shaper's reach (input > 32, or output past fp16) | 5 × 10⁻² | 3.9 × 10⁻² |
| Cube, a **vendored ACES output bake** at a gamut primary | 1.5 × 10⁻¹ | 1.17 × 10⁻¹ |

The first is arithmetic, not luck: ACEScct spends 17.52 stops over a 0–1 code range, so at
code 1.0 the decode climbs far faster than the curve table's own log-spaced samples, and
linear interpolation between them costs (ln 2 × 17.52 × h)² / 8 — 7.7 × 10⁻⁵ at the sample
spacing there, times the matrix's gain. The **encode** direction is unaffected, because
compressing 222 into 1.0 is the shallow way round.

The second is the floor rather than the ceiling of the same domain story. The signed curve
shaper's first sample above zero is at linear 7.8 × 10⁻⁶, so everything below that is one
straight line down to black — while a gamma encode has infinite slope at zero. A 10⁻⁷ probe
encoded to gamma 2.2 should be 6.6 × 10⁻⁴ and the table says 6.4 × 10⁻⁵. No table of any
size does better with a curve of infinite slope; the bound is the height of that first cell.

The fourth is **the number this section's out-of-domain warning becomes when a real ACES 2.0
rendering meets a 65-point grid** (K-518). It is not an interpolation wobble: the eight
corners of the cell containing the Rec.709 blue primary span 0.165 in Z and are not even
monotone in blue. Inside the gamut the same bake holds 2 × 10⁻³. The upgrade path is §4.1's
recorded one — port the output styles to tier one, one at a time — and these rows are the
measurement it will be judged by.

Two readings the table settles. First, the **1 × 10⁻⁵ is a statement about the
curve**, and at the curve it holds with room; a whole chain is that error times the
matrix's gain, because the matrix mixes three independently-wrong channels and its
row sums exceed one. Quoting one number for both would be quoting the wrong one, so
the tests hold the curve to 10⁻⁵ and the chain to 3 × 10⁻⁵ and say why. Second, the
deep-saturation **ceiling belongs to its probe family, not to the world**: the last
row is twenty-five times it, and that is exactly the domain-edge risk this section
names as unbounded rather than a regression against it.

**The risk that cannot be bounded away is the domain edge — of the cube form.** A baked
cube clamps what its shaper cannot reach: scene-linear values above the shaper ceiling
(`2^hi`), and — the sharper case — **negative components**, which edges-only Rec.709
working uses to carry wide-gamut colour (§2.1) and which a lg2 shaper cannot represent
below `−o`. The factorised form no longer shares this: its signed shaper covers both
signs out to the working format's own limits, which is most of why it is preferred
wherever a chain allows it.
Through a baked view, out-of-gamut saturation clamps to the gamut edge; the reference
library's exact CPU path would have rolled some of it through the view's own gamut
handling. This is v1's honestly-stated approximation: identical in preview and export
(so never a lie between them), invisible on Rec.709-native material (games, screen
captures — the v1 audience), visible only on wide-gamut footage under a config whose
views do their own gamut mapping. The recorded upgrade path is a mirrored-log shaper
(negative lobe below zero) and a per-config domain audit at bake time that widens
`hi` when the chain's own ops demand it; both slot into the artefact without touching
the samplers' interface. The conformance suite carries out-of-domain fixtures from
day one, tolerance-gated loosely, so the day the shaper improves, the gate tightens
rather than a new test being invented.

### 5.5 Cache keys

Colour state folds into the existing keys; nothing new is invented:

- the **config content hash** (config file bytes + every resolved LUT file's identity,
  i.e. the fingerprint the artefact cache already keys on) folds into the frame key's
  quality field — edit the config *or one of its LUTs* on disk and every frame
  retires. The LUT files enumerate through `LoadedConfig::files_read`, and each
  counts by path, length and last-modified stamp rather than by its bytes — the
  identity the effect LUT cache already uses (K-271) — because this is recomputed at
  the top of every render and re-reading tens of megabytes of cube per frame to be
  told nothing changed is not a cost worth paying. The ceiling is an edit that
  changes neither length nor stamp; reloading the project picks that up;
- a footage item's **space name** folds into its decode-job fields (docs/06 §5.2 keys
  decode by fields), so reassigning one item retires that item's frames only;
- the **display/view choice** folds into the display-encoded RAM-tier key beside the
  preview resolution — switching views re-encodes, switching back finds the old
  frames still banked. The linear tiers upstream are untouched by a view switch,
  which is most of the point of caching linear.

## 6. The seam and the UI

### 6.1 What crosses the bridge

Following docs/17's shapes — commands down, references up, no polling in rebuild
paths:

- **Read**: `ProjectReference::colour_summary() -> BridgeColourSummary` — the state
  (none / loaded / missing / refused + its one-sentence reason), the config's display
  path, the colour space names (active only), the displays each with its view names,
  and the resolved role names v1 uses (`scene_linear`, interchange). Fetched on
  document change, cached Dart-side; the budget test stays at 0 for rebuild paths.
- **Write**: `ProjectReference::set_colour_config(path: Option<String>)`;
  `FootageReference::set_colour_space(space: Option<String>)`. Both lower to the §3.1
  ops; one gesture, one undo step.
- **Viewer**: the existing look/display call gains the selected view —
  `Option<(display, view)>`, `None` = the built-in transform. Session state, stored
  in `ui_state` like the channel isolation it sits beside; never in the document.
- **Export**: `BridgeExportSpec.colour_space` already crosses as a `String` ("" =
  built-in, name = `Ocio(name)`, K-479) — **the seam does not change shape**; the
  engine's refusal simply starts saying yes when the name is the loaded config's.
  Coordinate with the export-colour work's built-in family so the one string field
  namespaces both (built-in names are the enum's own; config names are whatever the
  config says; a collision resolves to the config, which the user loaded on purpose).

### 6.2 The Viewer's colour-pipeline picker

`_ColourDropdown` in `flutter_ui/lib/panels/viewer_panel_frb.dart` — the picker whose
menu was built as "a row rather than a label because the list grows the day a second
one exists". This is that day. The menu becomes: the built-in transform row; then,
when a config is loaded, one section per display (§12A menu conventions: section
label, then rows), each view a tick-row; the tone-map row stays at the bottom and
composes with whichever transform is ticked (it sits inside the display stage, K-314,
and that does not change). The closed face names the view in force ("sRGB — ACES
1.0"), keeps the amber engaged tint for exposure/tone-map, and shows the calm missing
state ("Config missing") when degraded (§3.3). No new panel, no new widget kind.

### 6.3 The export dialog's Colour section

The colour-space dropdown the export work landed gains the config's names under a
section header, exactly as the picker does. A name the capability table refuses
(missing config, refused config) is disabled-not-hidden with the reason as its
tooltip — K-485's standing rule for controls a format cannot honour, applied
unchanged.

### 6.4 The Project settings window (§13.5, K-286)

Colour management is the project's, not the machine's — docs/07 §13.5 already says it
"lands here when built". A **Colour** group: a config row (path well + browse +
clear, the §12A.4 dialog-row conventions), a state line in the calm voice ("Loaded:
42 colour spaces, 3 displays" / the missing or refused sentence), and a read-only
working-space line stating §2.1's mode ("Working space: linear Rec.709" or, legacy,
"Working space: taken as *scene_linear* (ACEScg)"). Settings ▸ Colour (machine
defaults for new projects) is unchanged in v1; a "default config for new projects"
row there is recorded follow-up.

### 6.5 The interpret-footage surface

Per docs/07 §3.2 the colour-space tag lives in *Interpret footage…*; the row lists
the built-in defaults plus the config's spaces. Until that dialogue exists as drawn,
the Project panel's item context menu carries a **Colour space** submenu with the
same list — the smallest honest surface, replaced when the dialogue lands.

**K-303/K-005 gate**: the new Lumit-authored strings — the picker's missing state,
the settings rows, the refusal sentences the engine can send, the submenu label —
land with their `app_en.arb` keys in the same commit, engine-sent ones also in
`engine_labels.dart`; new keys listed in the commit message and PR for translation.
Config-supplied names cross verbatim and are never translated (§4.4).

## 7. Conformance — the golden fixture suite

Fidelity is proven, not asserted, and the proof is data checked into the repository:

1. **Reference fixtures.** For each supported config class — the legacy ACES 1.2
   config, the OCIO v2 ACES CG config, and a synthetic config exercising every op —
   a fixture file of a few hundred scene-linear RGB inputs (neutrals, primaries,
   gamut edges, negatives, HDR values to 100, denormals, exact 0 and 1) with expected
   outputs per (source space → destination space) pair and per (display, view),
   generated **offline with the reference OpenColorIO library**, the library version
   and generation script recorded in the fixture header. CI never builds OCIO; it
   reads the table. **Landed** — both configs, PyOpenColorIO 2.5.2, 912 rows.
2. **Two gates per fixture row**: the resolved chain evaluated exactly on the CPU
   must match within 1 × 10⁻⁵; the baked artefact sampled by the CPU sampler must
   match within the §5.4 bound in-domain (out-of-domain rows carry their own looser
   bound and exist to be tightened). Three details the real runs settled:
   **absolute below one, relative above** (an absolute 10⁻⁵ at an output of 22.76 asks
   for less than one `f32` ULP at that magnitude); a row whose reference answer is
   **not finite** is compared by kind, since `NaN − NaN` is NaN and no tolerance passes
   it; and an edge Lumit answers from a **vendored bake** carries the cube form's bound
   on *both* gates, because "exact on the processor" then means "the table".
3. **CLF suite**: the CLF specification's implementation-test files, vendored, parsed
   and evaluated against their published expectations. **Landed** — see WP6. Note
   what "published expectations" turned out to mean: no answer key is published
   alongside the documents, so each row's expected value is anchored in the
   document's own stated formula, the specification's own worked example, an
   identity that holds by construction, or arithmetic on the document's own
   coefficients. Every one is stated per file in `clf/clf.fixture`.
4. **CPU = GPU**: the WGSL samplers against the CPU samplers, ≤ 2 fp16 ULP on random
   cubes and curves — the lut.md pattern, skip-on-no-GPU as ever.
5. **K-031 parity**: the standing preview-equals-export matrix gains an OCIO row — a
   reference comp under a loaded config, Viewer readback bit-identical to the export
   bytes, in every shipped colour configuration (docs/06 §3.3's gate, now with a
   config in it).
6. **Refusal taxonomy**: one test walking a directory of deliberately-unsupported
   configs, asserting each refuses with the *right name* — the failure mode this
   design promises never to fumble into silence.

## 8. Work packages

Ordered; each sized for one agent; each lands with its tests (K-007). WP1 → WP2 →
WP3 → WP4 → WP5 → WP6, with WP6's fixtures authored alongside WP1–2 (the fixture
*format* is WP1's, so later packages land against real expectations, not hope).

### WP1 — Engine transform core

`crates/lumit-colour`: the op set (§4.1) with forward/inverse evaluation, the
tetrahedral and curve samplers (§4.3), the factorisation analysis, the bake to both
artefact forms (§5.1), the `.spi1d`/`.spi3d` parsers, CLF process-node reading atop a
small XML pull-parse, the shaper maths. Pure CPU, no config file yet — chains are
built in tests by hand.
**Tests**: per-op closed-form checks (matrix identity/inverse round-trip, log/exponent
against `f64` references, CDL against the ASC formula); tetrahedral corner/neutral-axis
exactness; 1D inversion round-trip and non-monotone refusal; factorised-vs-cube
agreement on factorable chains; bake determinism (two bakes, identical bytes);
the synthetic-config fixture rows at both gates (§7.2).

### WP2 — Config parser and resolution

`config.ocio` parsing on `yaml-rust2` (§4.4): spaces, roles, displays/views, looks
(as view-referenced transforms), `search_path` LUT resolution, anchors/merges,
version gating, the refusal taxonomy. Resolution through the reference space and the
interchange bridge (§2.1); `BuiltinTransform`'s two tiers with the vendored
reference bakes.
**Tests**: parse round-trips on the vendored ACES configs (counts of spaces, displays,
views match known values); search-path order; every refusal by name (§7.6); the ACES
fixture rows pass at the exact gate; legacy compose-through equals exact end-to-end on
input→display trips.

### WP3 — Document state, bake wiring, and the render paths

`ColourManagement` on `Document` and `colour_space` on `FootageItem` with their two
ops (§3.1); the derived loaded-state and artefact caches (§3.2); the degrade ladder
(§3.3); frame-key folding (§5.5); the decode-pass input variant, the `ColourEngine`
display-pass variant with the Unorm-view target (§5.2), and export riding the same
blit.
**Files**: `crates/lumit-core/src/model.rs`, `ops.rs`; `crates/lumit-colour` (cache);
`crates/lumit-gpu` (`colour.wgsl` variant, 3D/1D bind path, view formats);
`crates/lumit-render` (decode plan fields, display keys, export resolution).
**Tests**: old-file byte-identical round-trip; op undo symmetry; missing-config
degrade renders the built-in frame; key sensitivity (config edit retires frames,
item reassignment retires that item's, view switch re-encodes only); the
double-encode trap pinned (baked identity view == built-in output exactly); K-031
parity row (§7.5), skip-on-no-GPU.

**Landed.** The seam WP1 left open — a factorised stage's analytic tail — is closed in
the bake rather than in the shader (§5.1); the factorised form is guaranteed to be
`curve → matrix → curve`, so the shader has three fixed slots. Measured on the reference
machine: a factorised bake 0.75 ms and 192 KiB, a shaper+cube bake 5.5 ms and 3.1 MiB;
uploading them 0.5 ms and 1.9 ms; the 1080p display pass 0.12 ms/frame with a table bound
and 0.12 ms/frame without, i.e. the table costs nothing measurable at that raster. The
input transforms are built once per render, one table per distinct space the project's
footage names.

**The seam WP4 exposes** — everything WP3 wired, none of it crossing the bridge yet:
`HeadlessRenderer::{colour, sync_colour, set_colour_view, colour_view, set_colour_output,
can_deliver_colour_space}`, and on the state itself
`ColourState::{loaded, frame_identity}` with `Loaded::{usable, problem, path, vocabulary}`
— which is exactly `BridgeColourSummary`'s content (§6.1). `ExportSpec::check_with_colour`
replaces `check` wherever a project is in hand.

### WP4 — The bridge seam

`colour_summary`, `set_colour_config`, `set_colour_space`, the viewer look call's
view field, in `crates/lumit-bridge/src/api/**` (then codegen; generated files never
edited). Engine-sent refusal sentences into `engine_labels.dart` + `app_en.arb`
(K-005), keys listed for translation.
**Tests**: `engine_labels_test.dart` green; an frb test loading a fixture config and
walking assign/undo through the seam; `bridge_call_budget_test.dart` unchanged at 0.

**Landed.** The seam as WP5 will find it, in `crates/lumit-bridge/src/api/colour.rs`:

- `ProjectReference::colour_summary() -> BridgeColourSummary { path, loaded, problem,
  problem_args, problem_english, spaces, displays }` — `displays` is
  `BridgeColourDisplay { name, views }`. One read of the whole structure, fetched on a
  document change and held in Dart.
- `ProjectReference::set_colour_config(Option<String>)`,
  `FootageReference::{colour_space(), set_colour_space(Option<String>)}` — the two edits
  and the per-item read.
- `CompositionReference::set_viewer_look(..., colour_view: Option<Vec<String>>)` — the
  `[display, view]` list; **the look is set whole**, so a caller that omits it has said
  "no view".
- `ProjectReference::can_deliver_colour_space(name)` for the dropdown's enable, and
  `CompositionReference::export_spec_check(spec)` — which **replaced** the free-standing
  `export_spec_check` — for the pre-queue check.

Two shapes §6.1 did not foresee. The refusal is **not** the engine's sentence: it crosses
as `ColourError::key` plus `::args` (`config_unreadable` is the one id the renderer raises
itself), and `colourProblem` in `engine_labels.dart` writes the words — a sentence with a
config's name in the middle of it can never be looked up whole, and `problem_english` is
the fallback. And the seam keeps its **own** `ColourState` beside the render worker's: the
renderer lives behind a request channel, and a summary read that had to wait for a frame
would be a panel blocked on the Viewer. Both parse the same file by content hash, so the
cost is one parse each and neither can go stale.

### WP5 — Viewer, export, project UI

The picker's display/view sections and faces (§6.2), the export dropdown's config
section with disabled-not-hidden refusals (§6.3), the Project settings Colour group
(§6.4), the item context-menu submenu (§6.5), all arb keys in the same commits.
**Tests**: widget tests per surface (menu contents from a fixture summary, one op per
gesture, the missing-state face); no-hex lint stands; the export dialog refuses an
unavailable name with the reason in the tooltip.

**Landed.** The config is chosen where §6.4 puts it — **File ▸ Project settings ▸
Colour**, a path well with *Choose…* and *Clear*, the state line under it, and the
fixed working space stated as a reading. There is no separate relink: choosing again
*is* the relink, which is what a missing config's state line points at.

Three shapes the note did not settle, decided in the building:

- **The summary is held on `LumitUiState`, refreshed off the app state's own
  notification.** It reads the config file, so no `build` may ask for it (K-183); a
  document change is the only thing that can alter it, and that is exactly when
  `LumitState` notifies. Every surface reads that one field — the picker directly, the
  export dialog and the item menu through the context they are raised from — so there
  is one answer on screen and the bridge-call budget stays where it was. The Project
  settings window keeps its own copy instead, because it is the one surface that
  *changes* the config and must show the result of its own edit immediately.
- **The chosen view rides `_pushedView`.** The look is set whole, so the view joined the
  record `pushViewerLook` compares and sends, encoded as text beside the region for the
  same reason the region is. There is exactly one caller of `set_viewer_look`, which is
  what makes that safe.
- **A dropdown learned per-option refusal.** `BareDropdown` gained `disabledReason`, so
  K-485's disabled-not-hidden rule works inside a list and not only on a whole control;
  the export's colour dropdown asks `can_deliver_colour_space` once per config name as
  the dialog opens, never per rebuild. A file written through a config's transform is
  stated as untagged where the choice is made (§5.2).

Two v1 edges worth naming. The **working-space line is the fixed sentence**: the summary
carries no interchange/legacy flag, so §2.1's second reading ("taken as *scene_linear*")
waits on a field for it. And the **per-item submenu lists a name the loaded config does
not have**, ticked, when one was assigned under a config that has since gone — the name
is the user's statement about the file and a moved path must not silently edit it.

### WP6 — The conformance suite, completed

The full §7 matrix as CI: reference fixtures for both ACES configs regenerated and
frozen, the CLF suite vendored, the out-of-domain rows with their documented looser
bounds, the parity row in every shipped configuration, and a README in the fixture
directory stating provenance and the regeneration recipe. Tightening §5.4's
out-of-domain bound later must only ever mean editing tolerances downward.

**Landed, except for two data drops that need a machine with the reference library
on it.** What is in CI now:

- **The CLF suite (§7.3) is real.** Eight documents from the specification's own
  example and implementation-test set are vendored byte for byte in
  `crates/lumit-colour/tests/fixtures/clf/`, each evaluated against expected values
  that are published rather than measured — a spec example's own worked numbers, a
  generating formula the document prints in its own `Description`, an identity that
  holds by construction, or arithmetic on the document's own coefficients. This is
  the part of §7 that never needed the library, and it found two reader faults on
  the day it landed: vendor elements inside an `Info` block were being read as
  process nodes, and an XML comment inside an `Array` glued the numbers either side
  of it into one token.
- **The §5.4 bounds are re-measured** with dense sweeps rather than sample points,
  and the numbers are in the table there. They hold, with one clarification worth
  more than the reassurance: the 10⁻⁵ figure is the *curve's*, and a chain's error
  is that times the matrix's gain.
- **§7.5's parity row is the colour matrix** in
  `crates/lumit-render/tests/ocio_parity.rs`: no config, every built-in colour
  family named at export, a config's display/view, and a config's space at export,
  each requiring the Viewer's eight-bit present and the export's deep one to be one
  picture — plus a plain-gamma view that must render *differently*, without which
  the other rows pass equally well when nothing is bound at all.

**Both artefact drops have landed** (2026-08-25, PyOpenColorIO 2.5.2, one session):

1. `tests/fixtures/aces-1.2/` and `aces-1.2.fixture` — the legacy config, its five
   reachable LUTs (14 MiB of a 444 MiB set), and 128 rows over the role-space edges and
   the sRGB and Rec.709 views.
2. `tests/fixtures/aces-cg/` and `aces-cg.fixture` — `cg-config-v4.0.0_aces-v2.0_ocio-v2.5`
   whole (33 KiB, no side files: a v2 config's transforms are builtins), 784 rows over
   every role-space edge and **every** one of its 37 display/view pairs, together with the
   five vendored bakes in `crates/lumit-colour/vendored/` from the same session.

Both were expected to be pure data drops. Neither was, and what they found is the
argument for having run them:

- **Five reader faults**, each invisible without a real config. Adjacent matrices were
  walked rather than composed, so a matrix meeting its own inverse left 2 × 10⁻⁵ behind
  (K-516). A shared view's `<USE_DISPLAY_NAME>` placeholder was not resolved, so *every*
  view of *every* OCIO v2 ACES config failed to resolve at all. A conversion touching a
  **data** space still ran the working-space bridge, putting a primaries matrix through a
  matte. A view transform stating only `from_display_reference` had that transform read as
  a scene-referred one and inverted, silently dropping the rendering. And the negative
  style on an exponent was read from `negativeStyle` when a config file writes `style`
  (K-517). Every one is now held by a unit regression as well as by fixture rows.
- **Four new §5.4 measurements**, in the table there: the ACES log decode's own sampling
  error, the gamma encode below the shaper's first cell, the cube past the shaper's reach,
  and — the one that matters — 0.117 at the Rec.709 blue primary through a vendored ACES
  2.0 output bake (K-518).

The recipes that produced them are checked in beside the data: `tests/fixtures/README.md`
and its `generate.py`, `vendored/README.md` and its `bake.py`.

Tier one now answers fourteen builtin styles and tier two five, so **both shipped ACES
configs resolve end to end**. What remains of §4.1's debt is the recorded upgrade, not a
gap: exact Rust ports of the four output transforms and the LMT, one at a time, each
measured against the rows that gate the bake it replaces.

## 9. Test plan — the core invariants

Beyond the per-package tests, the properties a regression would betray:

1. **Old files are untouched**: any pre-K-490 fixture loads, saves, byte-compares.
2. **One implementation**: Viewer readback equals export bytes under every colour
   configuration, config loaded or not (K-031's gate, extended — §7.5).
3. **We match the reference or we refuse**: every claimed transform passes its golden
   fixtures at the stated bound; everything unclaimed refuses by name; there is no
   third state (§7.2, §7.6).
4. **Determinism**: same config bytes → same artefact bytes, on every platform in CI.
5. **The degrade ladder holds**: a project whose config vanished opens, renders the
   built-in pipeline, keeps every name, refuses OCIO exports, and says all of this
   calmly where §3.3 promises.
