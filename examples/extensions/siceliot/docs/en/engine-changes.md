# Engine changes

What SICELIOT changes in Lumit (`D:\Siceliot\Lumit`): fourteen commits on the branch
`siceliot/lumit-0.10`, on top of upstream `v0.10.0` (`eb27a6eb`, Lumit 0.10.0), also kept as
`patches/0001…0014` ([README](../../patches/README.md)). Each entry says why the change is there,
what it does today and which files it touches.

*Italiano: [../it/modifiche-engine.md](../it/modifiche-engine.md)*

## How it is checked

- `cargo test -p lumit-core --lib`: 579 passed. `cargo test -p lumit_bridge --lib`: 246 passed.
- `cargo clippy -p lumit-core -p lumit-render -p lumit_bridge --all-targets -- -D warnings`,
  `cargo fmt --check`: clean.
- `flutter test`: all pass but `ae_import_frb_test` ("a real .aep imports through the front
  door"), which fails the same way on an untouched upstream `v0.10.0`.
- `sh extensions/src/build.sh`: the 19 LFX effects pass `tools/lfx-check` under Siceliot FX.
- Renders with `lumit-cli`: `tests/render` (every LFX effect), `tests/copies` (per-copy Clone to
  points, decode labels, the rings sketch), `tests/drivers` and the two example projects.
- All fourteen patches apply on `v0.10.0` with `git am` and give the same tree as the branch.

| # | Commit | Change |
|---|---|---|
| 1 | `313fbc5a` | [A category of Siceliot's own](#1-a-category-of-siceliots-own) |
| 2 | `9863e783` | [Drivers: Stagger, Random, Oscillator, Value noise](#2-drivers-stagger-random-oscillator-value-noise) |
| 3 | `4b5637f0` | [Per-copy Scale, Rotation and Opacity in Clone to points](#3-per-copy-scale-rotation-and-opacity-in-clone-to-points) |
| 4 | `c3133fd9` | [Oscillator and Value noise vary by position](#4-oscillator-and-value-noise-vary-by-position) |
| 5 | `d547d237` | [Greyed-out driver rows and random text in Label points](#5-greyed-out-driver-rows-and-random-text-in-label-points) |
| 6 | `c7405e86` | [Decode: Label points reacts to a wave](#6-decode-label-points-reacts-to-a-wave) |
| 7 | `495e1cf5` | [Rings: Ring wave and smooth, turning labels](#7-rings-ring-wave-and-smooth-turning-labels) |
| 8 | `183f23e5` | [Gradient colours, Away from points, Grow, Vertical anchor](#8-gradient-colours-away-from-points-grow-vertical-anchor) |
| 9 | `4c87457e` | [Copied effects keep their drivers and wires](#9-copied-effects-keep-their-drivers-and-wires) |
| 10 | `4c8c67b0` | [Type fill](#10-type-fill) |
| 11 | `425fa936` | [Two headings: Siceliot FX and Siceliot Nodes](#11-two-headings-siceliot-fx-and-siceliot-nodes) |
| 12 | `48ed40b7` | [Type fill: each word once per column, and Cut words](#12-type-fill-each-word-once-per-column-and-cut-words) |
| 13 | `95cfc997` | [Backdrop: an LFX effect that bends what is behind its layer](#13-backdrop-an-lfx-effect-that-bends-what-is-behind-its-layer) |
| 14 | `f3e3d364` | [Greyed LFX rows: @enabled_if](#14-greyed-lfx-rows-enabled_if) |

Every change that adds a row or a name also updates `flutter_ui/lib/l10n/app_en.arb`,
`engine_labels.dart` and the generated `crates/lumit-core/fx-labels.txt` and `fx-reference.json`
(`cargo test -p lumit-core --lib -- regenerate_fx_label_fixture regenerate_fx_reference
--ignored`). The other language files are not edited by hand, as Lumit's rules require.

---

## 1. A category of Siceliot's own

Lumit files every effect under a built-in category, and an LFX manifest may only name one of the
eight picture categories; anything else is refused with *"`…` is not one of Lumit's effect
categories"*. So the engine has to know Siceliot's category.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/schema.rs` | `FxCategory::Siceliot`, in `FxCategory::ALL` right after `Utility`, so it is the last picture family in every menu. Labelled **Siceliot FX** (change 11) |
| `crates/lumit-core/src/fx/lfx.rs` | `Siceliot` in `PICTURE_CATEGORIES`, so a manifest may name it (case ignored). The manifest test checks it |
| `crates/lumit-core/src/edits.rs` | Machine key `siceliot`, which the Effects browser groups and sorts by |
| `flutter_ui/lib/l10n/app_en.arb`, `engine_labels.dart` | `fxSiceliot`, a proper name kept in every language |

An extension names it like this; it installs only in a Lumit built with these changes:

```json
"effects": [
  { "id": "falloff", "name": "Falloff", "category": "Siceliot FX",
    "version": 1, "api": 1, "shader": "falloff.wgsl" }
]
```

---

## 2. Drivers: Stagger, Random, Oscillator, Value noise

Cavalry's modularity rests on behaviours wired into any attribute and evaluated once per copy. An
LFX effect can only make pictures, so drivers, which make numbers, are built into the engine.
Lumit already gives each copy of Clone to points its number (Clone index); these drivers turn it
into Cavalry's Stagger, Random, Oscillator and Noise. See [effects.md](effects.md#siceliot-nodes).

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/drivers/siceliot.rs` | The drivers (`siceliot_stagger`, `siceliot_random`, `siceliot_oscillator`, `siceliot_noise`), their maths as public functions, and a test of the values the docs promise |
| `crates/lumit-core/src/fx/schema.rs` | `FxCategory::SiceliotDrivers` (shown as **Siceliot Nodes**, change 11) and `FxCategory::is_driver()`, true for `Drivers` and `SiceliotDrivers` |
| `crates/lumit-core/src/comp_graph.rs`, `crates/lumit-eval/src/lib.rs`, `crates/lumit-script/src/commands/effects.rs`, `crates/lumit-bridge/src/api/{composition,effect,layer}.rs` | Every "is this a driver?" test asks `is_driver()`, so Siceliot's drivers go on the node graph, wire into parameters and stay out of the stack exactly as Lumit's own |
| `crates/lumit-core/src/fx/drivers/clone_index.rs` | Clone to points renders a layer once per copy when it finds Clone index **or** a Siceliot driver in it |
| `crates/lumit-core/src/fx/catalogue.rs`, `drivers/mod.rs`, `registry.rs`, `edits.rs`, `tests.rs` | Registration, category key `siceliot_drivers`, the matte test exempting every driver category |

Cavalry's Noise is labelled **Value noise**, because Lumit already has an effect called Noise.

---

## 3. Per-copy Scale, Rotation and Opacity in Clone to points

A Transform effect on the cloned layer crops the copies: a shape layer's picture is only as big
as its art, and an effect cannot draw past it. In Cavalry a behaviour wired into the Duplicator's
Shape Rotation turns each copy whole. Clone to points already turns and sizes each stamp by its
point; it also takes the value per stamp from drivers.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/effects/clone_to_points.rs` | `drive_rows`: when **Scale**, **Rotation** or **Opacity** is wired and the layer's graph holds a driver that reads the copy, the drivers are resolved once per point as copy *i* of *n*, into the columns `siceliot:scale`, `siceliot:rotation`, `siceliot:opacity`. `stamps()` reads them stamp by stamp. At most 4096 points are evaluated one by one; past that the last value repeats. Spread over the cores |
| `crates/lumit-render/src/build.rs` | `points_input_for` runs it on the stream handed to Clone to points (with the graph of node-graph comps) |
| `crates/lumit-core/src/fx/drivers/clone_index.rs` | `reads_copy` public |
| `crates/lumit-core/src/fx/tests.rs`, `drivers/mod.rs` | A Stagger (0 to 90) wired into Rotation turns three points by 0°, 45°, 90° |

---

## 4. Oscillator and Value noise vary by position

With Phase per copy a wave follows the copies' numbers, so on a Grid it runs row by row. In
Cavalry a wave can cross the copies in space. The drivers know where each copy stands.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/registry.rs` | `DriverCx::point`: where the copy stands, in px@comp |
| `crates/lumit-core/src/fx/drivers/mod.rs` | The driver walk carries the point; `resolve_drivers_at_point` |
| `crates/lumit-core/src/fx/drivers/siceliot.rs` | **Oscillator**: **Phase from** (Copy · Position), **Wave angle**, **Wavelength**. **Value noise**: **Vary by** (Nothing · Copy · Position), **Noise scale**. Tests |
| `crates/lumit-core/src/fx/tests.rs` | The new px rows |

Position is known when the driver is wired into Clone to points' own Scale, Rotation or Opacity
(change 3) or into Label points' driven rows (changes 6, 7); anywhere else it reads as the
comp's top-left corner.

---

## 5. Greyed-out driver rows and random text in Label points

Lumit greys out rows a mode does not read; the drivers do too. And Label points, which could only
write numbers, can put random text at every point, like Cavalry's String Generator on a
Duplicator.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/drivers/siceliot.rs` | `OSCILLATOR_ENABLED_WHEN`, `VALUE_NOISE_ENABLED_WHEN`: Phase per copy on Copy, Wave angle and Wavelength on Position, Noise scale on Position |
| `crates/lumit-core/src/fx/effects/label_points.rs` | `random_text`: `{pick:a\|b\|c}`, `{chars:N}` and `{hex:N}` replaced per point, by the point's id and the **Seed** row, so each point keeps its text in any order. A test |
| `web-docs/…/effects/generate/label-points.mdx` | Lumit's manual: the tokens and Seed |

---

## 6. Decode: Label points reacts to a wave

The "decoding text" look: a grid of characters whose text changes and whose colour flashes while
a wave crosses it, resting again once it has passed.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/effects/label_points.rs` | Rows **Change**, **Change above**, **Scramble rate**, **Flash colour** (Off · Random hue · Two colours, and the options of changes 7 and 8), **Colour A**, **Colour B**, **Label scale**. Above Change above the tokens' seed steps Scramble rate times a second and a colour is rolled per point and step. The effect is `seeded`, so the frame cache keys it by time. A test |
| `crates/lumit-render/src/build.rs` | Drivers wired into the rows in `LABEL_DRIVEN_ROWS` (Change, Label scale, and Text step and Label rotation from change 7) are resolved per point with `drive_rows` |
| `web-docs/…/effects/generate/label-points.mdx` | Lumit's manual: the new rows |

---

## 7. Rings: Ring wave and smooth, turning labels

To match a Processing sketch ("lettere su lettere": a giant letter filled with 12 px characters,
crossed by rings that colour, enlarge, turn and change them) with the same smoothness.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/drivers/siceliot.rs` | Driver **Ring wave** (`siceliot_ring_wave`): Minimum, Maximum, Interval, Speed, Speed variance, Thickness min, Thickness max, Origin, Spin, Seed; outputs **Value**, **Rotation**, **Passes** (and **Grow**, change 8). Ring `k` starts at `k` × Interval and lives until it has crossed the frame, so Passes is counted from the time alone. In `MATCH_NAMES`, so it is resolved per point. A test |
| `crates/lumit-core/src/fx/effects/label_points.rs` | Rows **Text step** (each whole number moves the resting seed on), **Hue start**, **Hue range**, **Saturation**, **Darken** (for Flash colour **By strength**), **Grow**, **Label rotation**. A label carries a rotation and a tint (Replace for the flashes, Multiply for By strength). Tests |
| `crates/lumit-core/src/fx/points.rs`, `crates/lumit-render/src/build.rs`, `fxops.rs` | `PointsSchedule::late_drive`: a stream made on the card (Pick points by Image) gets its per-point drivers when it arrives; also for 0.10's Particulate with Trails |
| `crates/lumit-render/src/gpufx.rs` | Each wording is laid out once per half octave of size and every label resampled from it, bilinear, at its own fractional position, size and turn, then tinted. Compositing is spread over the cores |
| `web-docs/…/effects/generate/label-points.mdx` | Lumit's manual: the new rows |

The driver is named Ring wave because Lumit already has a **Ripple** (a distortion). On an RTX 3060,
845 letters at 1920 × 1080 cost Label points about 14 ms a frame.

---

## 8. Gradient colours, Away from points, Grow, Vertical anchor

Closer to the sketch: a gradient of chosen colours, rings that never start on the letter, letters
that grow by each ring's own share, overlapping rings that add up, letters hung from their top.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/effects/label_points.rs` | Flash colour **Gradient**: **Gradient stops** (2 to 4), **Gradient 1** to **Gradient 4**, mixed in linear light by Change, multiplying the Text layer's colour. **Vertical anchor** (Middle · Top · Bottom). By strength turns the hue on past Change 100. Tests |
| `crates/lumit-core/src/fx/drivers/siceliot.rs` | Ring wave: Origin **Away from points** with **Clearance** (up to 64 tries, by the seed, for a place that far from every point); **Add up**; **Grow min**, **Grow max** and the **Grow** output. Tests |
| `crates/lumit-core/src/fx/points.rs`, `registry.rs`, `drivers/mod.rs`, `effects/clone_to_points.rs` | `Cloud`: the stream's points binned; made once by `drive_rows` and carried by the driver walk beside the point |
| `crates/lumit-render/src/gpufx.rs` | Label points passes both anchors |
| `web-docs/…/effects/generate/label-points.mdx` | Lumit's manual: the new rows |

---

## 9. Copied effects keep their drivers and wires

Copying wired effects (Grid, Pick points, Label points) and pasting them on another layer gave a
layer that drew nothing: the wires and the drivers feeding them live in the layer's node graph
and stayed behind.

| File | Change |
|---|---|
| `crates/lumit-core/src/preset.rs` | `EffectPreset::wiring`: `wiring_of` gathers the wires between the copied effects and every driver feeding them, directly or through other drivers; `None` when nothing is wired. `to_json_wired`. A test |
| `crates/lumit-bridge/src/api/layer.rs` | `copy_effects` adds the wiring; `paste_effects` recreates effects, drivers and wires with fresh ids, to the right of what the target's graph holds, in one commit, keyframes landing at the playhead |
| `crates/lumit-bridge/src/api/tests.rs` | A Blur driven by a Wiggle arrives with its own Wiggle wired into Radius |

Saved `.lumfx` presets hold the stack alone, and a Lumit without this change reads a copied
text's effects and ignores the wiring.

---

## 10. Type fill

Cavalry scenes like Tim Siegert's *Submesh Boolean* fill the frame with rows of words that
stretch into whatever room a moving shape leaves. Lumit could not lay text out by a region, and
an LFX shader cannot draw text, so **Type fill** is built in. See
[effects.md](effects.md#type-fill).

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/effects/type_fill.rs` | Effect `siceliot_type_fill`: Text layer, Words, Region layer, Region placement, Region from, Invert region, Threshold, Rows, Row gap, Word gap, Min width, Order, Seed, Mix (Cut words: change 12). The layout (row bands, columns as wide as the words naturally are, free stretches along each row) is plain Rust with a test |
| `crates/lumit-core/src/fx/registry.rs`, `points.rs` | `EffectDef::reads_text_layer()`, `points::wants_carriage_of(def)`: an effect can ask for its Text layer's document without taking points |
| `crates/lumit-render/src/build.rs`, `fxops.rs` | The text document is handed over through `wants_carriage_of` |
| `crates/lumit-render/src/gpufx.rs` | Reads the Region layer back, samples each row along its middle, draws each word trimmed and scaled to its piece (`stamp_box`, shared with Label points) |
| `crates/lumit-core/src/fx/effects/label_points.rs` | `Placed::boxed`, `LabelDrawing::trimmed`, scale per axis |
| `catalogue.rs`, `effects/mod.rs`, `tests.rs`, `web-docs/…/generate/type-fill.mdx` | Registration, px rows, manual page |

---

## 11. Two headings: Siceliot FX and Siceliot Nodes

Picture effects and drivers shared one heading, and nothing told a user that the drivers work only
in the Node graph. There are two headings:

- **Siceliot FX**: effects added to a layer like any other, in Effects & presets and the Add
  effect menu.
- **Siceliot Nodes**: the drivers, listed only in the Node graph.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/schema.rs` | `FxCategory::Siceliot` labelled `Siceliot FX`; `FxCategory::SiceliotDrivers` labelled `Siceliot Nodes`, a heading of its own |
| `crates/lumit-core/src/fx/lfx.rs` | A manifest names `"Siceliot FX"`; `"SICELIOT"` is read as the same category, so 1.2.0 extensions keep working. The test checks both |
| `flutter_ui/lib/l10n/app_en.arb`, `engine_labels.dart` | `fxSiceliot`, `fxSiceliotNodes` |
| `web-docs/…/type-fill.mdx`, `label-points.mdx` | The manual says Siceliot FX |

---

## 12. Type fill: each word once per column, and Cut words

A shape that cut a column in two gave its word to both parts, so a second "the" appeared and
jumped about. Where a shape's edge crossed a column, the inside and outside Type fills each
stretched the word into their own part. And since the region is read only along each row's
middle, a tall row spilled words over a curved edge: white on white, black on black.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/effects/type_fill.rs` | `TypeFill::pieces`: a column's word is stretched from its first free part to its last, if that spans at least **Min width**, and each part gets a `Piece` with its own slice (`clip`). Row **Cut words** (`cut_words`, off): every word whole at its column's width. `TypeFill::coverage`: a soft step at Threshold, so a Type fill and its Invert region twin share each edge pixel exactly. Tests |
| `crates/lumit-core/src/fx/effects/label_points.rs` | `Placed::clipped_x`: only a slice of a placed word is drawn |
| `crates/lumit-render/src/gpufx.rs` | Each piece is drawn clipped to its slice, then the words are masked by the region pixel by pixel (`TypeFill::coverage`), with Cut words on or off |
| `web-docs/…/generate/type-fill.mdx` | The manual: Cut words, Min width, the pixel cut |

---

## 13. Backdrop: an LFX effect that bends what is behind its layer

An LFX effect on a layer with a picture saw only that picture. Glass, a lens or anything else that
bends what is behind a shape had to go on an adjustment layer, with the shape picked in a row. An
effect whose manifest says `"backdrop": true` now works straight on the layer: the layer is drawn
as an adjustment for that frame, over the composite below, and its own picture is the effect's
second input.

- The layer covers the whole frame, unmoved, with its opacity. Its masks and transform shape the
  picture handed to the effect, placed as the layer sits in the comp (as `depth` inputs are).
- A row that picks another layer keeps it. The layer's own pixels are not drawn.
- Every other effect on the layer works on the composite too, as on an adjustment layer.
- The layer stays what it was in the project: its picture is still decoded, and on an adjustment
  layer nothing changes. The official Lumit ignores the line, so the same extension works there on
  an adjustment layer.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/registry.rs` | `EffectDef::backdrop`, false by default |
| `crates/lumit-core/src/fx/lfx.rs` | `LfxEffect::backdrop`, kept in the def's shape key, so a rescan changes it. Test |
| `crates/lumit-extensions/src/lib.rs` | `ManifestEffect::backdrop` (`#[serde(default)]`) |
| `crates/lumit-render/src/build.rs` | `backdrop_fx`: a layer with a live backdrop effect takes the adjustment path, covering the comp with no transform, masks, parent or 3D. `own_shape`: the layer's picture before its effects, placed by `world_placement`, as that effect's input when its row is empty or names the layer |
| `web-docs/…/use/lfx-effects.mdx` | The manual: the `backdrop` field |

---

## 14. Greyed LFX rows: @enabled_if

A shader's rows were all live, even those that did nothing as things stood: Reaizzi Glass on a
PNG kept the built-in shape's Centre, Width, Height, Corner radius and Shape rotation editable,
though the PNG gives the shape. A row annotation now says when a row does anything, and Effect
controls greys it otherwise.

- `@enabled_if(term, …)`: live while any term holds. Several on one row must all hold.
- Terms: `input` / `!input`, whether the second input gives a picture (its row names a layer, or a
  backdrop effect sits on a layer with a picture); `field=N` / `field!=N`, a choice on option N or
  a toggle 0 or 1.
- A term that will not parse drops its rule (the row stays live) with a note. The shader is not
  told, and the official Lumit ignores the annotation.

| File | Change |
|---|---|
| `crates/lumit-core/src/fx/shader/mod.rs` | `RowTerm`; `ShaderField::enabled_if`, read from the field's annotations |
| `crates/lumit-core/src/fx/registry.rs` | `EffectDef::row_state_on(inst, picture)`, `row_state` by default |
| `crates/lumit-core/src/fx/lfx.rs` | `LfxDef::row_state_on`: the rows whose rules fail, greyed. Test |
| `crates/lumit-bridge/src/api/effect.rs`, `layer.rs`, `composition.rs` | `read_instance_info` is told whether the layer has a picture (`!is_adjustment()`) |
| `web-docs/…/use/lfx-effects.mdx` | The manual: `@enabled_if` |
