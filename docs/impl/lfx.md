# LFX: the native plugin API, its effects, and the Addons page

**Decision:** LFX is a frozen, typed C ABI whose parameter vocabulary is Lumit's own
rather than a standard to be mapped; it runs out of process on the OFX substrate with a
manifest read before any of its code; an LFX effect is an ordinary catalogue entry, and
Settings ▸ Addons is where every hosted plugin - OFX, LFX, audio - is listed, switched on
and off, and installed. **Related:** [12-PLUGINS.md](../12-PLUGINS.md) §3 is the spec and
this note is the binding *how*; [ofx-host.md](ofx-host.md) §4/§4a/§4b is the substrate and
the shape to mirror; [audio-plugins.md](audio-plugins.md) §5 is the precedent for a second
host reusing that architecture without sharing its code;
[effect-registry.md](effect-registry.md) §2.6 is the run-time registration seam LFX
registers through; [resource-governor.md](resource-governor.md) is the ledger the ring now
pays into; [release-signing.md](release-signing.md) is the trust root an installed bundle
does *not* get and the discipline it does.

**Built so far: the shared pipe crate, the ABI header, the namespace wiring, the describe
lowering, the in-process host, the protocol and ring, the broker, discovery, `LfxDef`, the
instance pool, the fp16 seam, the validator, install and trust, and the template.**
§3.1's extraction - `lumit-ipc` holds the
pipe, the spawn helpers and the rules every host answers the same way; both shipping hosts are
migrated and no endpoint name, environment variable or public path changed. §2's ABI -
`lumit-lfx-abi` carries `include/lfx.h` under MIT, its `#[repr(C)]` mirror and the layout
suite that holds the two halves to the same numbers from both sides. §4.1's seam - an `lfx:`
name instantiates in `EffectNamespace::Lfx` and reaches both picture walks through one
`is_catalogued()` predicate. §2.2 to §2.4's describe - the sink a plugin pushes typed
declarations into, and the lowering that turns them into the same `EffectSchema` a built-in
carries, refusals and report lines apart, every ceiling the header declares asked where the
stranger's numbers arrive. §3.2 and §3.4's two planes - `lumit-lfx/src/ipc` carries the
protocol, versioned apart from the ABI and answering every message exactly once, and the ring
whose slots are sized by depth and by the declared window and whose bytes are reserved from
the governor's ledger. And §4.5's two verifiable halves - the defaulted `apply_f16_temporal`
hook and `readback_linear_f16` - which are the whole of the fp16 promise that can be compiled
without a card. §10's fixture and §4.2's in-process half - `lumit-lfx-testplug`'s twelve
personalities on one side of the frozen header and `lumit-lfx/src/local.rs` on the other, so
the ABI edge is driven end to end by a plain `cargo test`: a module opened, a descriptor list
read, the sink filled in, an instance created and handed frames at both depths. And §3.3 and
§3.5's second process - `lumit-lfx-broker` is the program a plugin lives in, and
`lumit-lfx/src/ipc/broker.rs` the supervisor that starts it: the bundle's own listing read
before any of its code, the module opened lazily on the first describe with the switched-off
list travelling with it, the listing re-checked against what the code answered, handles minted
here and only quoted back, three consecutive strikes, and a restart that is a replay by plugin
id rather than by a position the disable list can shift. And §5's discovery - the bundle
layout with its ordered per-target architecture list, the search paths with the addons
folder appended inside every host's own - pinned by name in each of the three - the three
tables, the per-render gate, and the
extension negotiation run from the listing before `create` and before any of that plugin's
own code. And §4.2's
catalogue entry - `lumit-lfx/src/def.rs` is the `EffectDef` a described plugin becomes: the
resolved bag turned into the dense value array the plugin reads, both depths crossing as
themselves, identity byte for byte on every road that is not a picture, a badge taken on
read, a retimer's sampled frames clamped to what the neighbour decode will hold, and the
three `badge_of` edits §4.3 lists, so an LFX instance badges the plugin sentence rather than
"an effect from a newer Lumit". And §4.4's pool - `lumit-lfx/src/pool.rs` is one live
instance per in-flight frame, leased for a frame's length, grown under the first
adaptive-concurrency policy this project has written down and collapsed to one when the
governor asks everybody to trim. And §9's tool - `lumit-lfx-validator` is the shipped
`lfx-validator` CLI and the library under it: ten suites driving a bundle **through the
broker** with a broker of its own per plugin, a markdown table in the OFX conformance bench's
shape, a non-zero exit, and two CI jobs that download nothing, so the gate is on from the
first day. And §6's install - `lumit-lfx/src/install.rs` is what
dropping a `.lfxpack` on Lumit does, with every entry's name swept before a byte is read, the
detached signature checked before the JSON is parsed, the key's fingerprint compared against
`lumit-lfx/src/trust.rs`'s store - where a damaged file is a refusal rather than a first use -
a bounded unpack into a folder no scan looks in, and one rename after a broker has read the
staged bundle's own listing out of process. And §10's template - `template/` is the
repository a vendor starts from, staged in this tree so that this workspace's CI is what
says whether it works: the header and the `#[repr(C)]` mirror copied byte for byte under a
test that reads both, a C++ wrapper and the `lfx-sys`/`lfx` pair beside them, and one
working example per set of bindings, each laid out as a bundle and driven through
`lfx-validator` on three platforms - with docs/12 §3 amended to the ABI as built, docs/05's
crate table carrying the five LFX crates and docs/07's settings inventory saying **Addons**.
The rest is design until built.

## In plain terms

Somebody writes an effect in C or Rust, builds it against a header Lumit publishes, drops
the bundle on Lumit, and it appears in **Effects & presets** next to Gaussian blur - same
category, same rows, keyframeable, expression-readable, drivable from the node graph. If
it crashes it takes down a process nobody was using and the layer wears a calm badge. The
Addons page is where a person finds out which strangers are in the building, and asks one
to leave.

Three things arrive together because they are one thing seen from three sides. Design any
one alone and the other two inherit its holes: an ABI with no manifest gives a settings
page that cannot name a plugin it has not run; a settings page with no *on* switch is a
page that can only make things worse; a catalogue entry that forgets the namespace guards
renders identity with no badge and no clue.

Everything below starts from a failure. Six of them shape the whole design:

1. a plugin that crashes, hangs, lies, or allocates without bound;
2. an ABI break after the first vendor has shipped against the header;
3. a project opened on a machine that has not got the plugin;
4. a Flatpak whose `/usr` is the runtime's own, so the standard search paths are empty;
5. a plugin switched off while a comp that uses it is on screen;
6. a plugin switched off *before* a scan, which has no row of its own in any listing.

## 1. Decisions taken

docs/12 §3 is silent on a great deal. These are the answers this note takes, each with the
cost stated. They are the list to overturn; every one of them is cheap now
and dear later, because §3.1 freezes names and layouts alike.

| # | Question §3 leaves open | Decision | Cost |
|---|---|---|---|
| D1 | `kfx.*` or `lfx.*` | **`lfx.*`**, and the CLI is `lfx-validator` | thirteen occurrences in docs/12 - twelve on eleven lines inside §3, one among the open questions - plus `docs/research/research-plugins-interop.md:77`; impossible once one vendor has compiled. **Paid, in the template**: every one of them now reads `lfx` |
| D2 | Which kinds v1 admits | The mirror of `ParamKind` in §2.3; `LFX_PARAM_PATH` and `LFX_PARAM_STRING` present in the frozen enum, **refused by name** in v1 | a plugin needing text keeps its declared default forever |
| D3 | Two things called "curve" | `LFX_PARAM_CURVE` is the host tone curve; a bezier path is `LFX_PARAM_PATH` and waits for `lfx.overlay` | shape-warping plugins wait; a path with no on-Viewer handles is a control nobody can edit |
| D4 | Whether `unit` is optional | Mandatory. `LFX_UNIT_UNSET = 0` is a describe refusal | one more thing an author must get right, and the one that makes px@comp real |
| D5 | How a plugin declares §3.4's traits | `lfx_traits`, reached by pointer from the descriptor (§2.4) | a struct in the frozen ABI; a `NULL` **or zeroed** trait block is the pessimistic case, which costs every field a `UNSET = 0` |
| D6 | How a *list* of categories becomes one heading | A closed picture-family enum; **first declared is the heading, the rest are search keywords** | an author may not invent a family, which is what lets LFX keep §3.7's promise |
| D7 | 8 bpc projects, which §3.3 does not name | Sent as fp16. A plugin never sees an integer buffer | none: fp16's significand round-trips every 8-bit code value |
| D8 | Whether a plugin may hold opaque state | **No.** `plugin_state` stays empty for every LFX instance | a vendor with a trained LUT declares a `FILE` parameter, whose payload rides beside the op as an aux slot and needs the render pass's generic file aux (§2.3); in exchange the frame key is complete, restart-as-replay is exact, and the staleness hole OFX still has is not inherited |
| D9 | What replaces OFX's `memoryAlloc` ceiling | A declared `scratch_bytes_per_megapixel` checked against the ledger, plus a Windows Job Object memory limit | one Windows-only dependency - `windows-sys`, the workspace's first (§8); on macOS and Linux a liar is contained by the process boundary alone |
| D10 | Discovery, which §3 has none of | §5's bundle layout, search paths, `LFX_PLUGIN_PATH`, and a Lumit-owned addons directory | a new directory convention, recorded in `lumit-project` beside the others |
| D11 | Where an installed bundle lives | `data_local_dir()/addons` - machine-local, never roaming | the first `data_local_dir()` caller in the tree; see the trap in §11 |
| D12 | What "verification" means at install | Trust on first use: signature checked before the JSON is parsed, fingerprint recorded, a later pack under another key refused | stops a silent swap; stops nothing on the first install, and the page says so |
| D13 | Whether a plugin gets a matte row | No. `MatteRole::None`, as OFX and audio declare | a Matte the plugin never heard of would be a control nothing consumes |
| D14 | §3.7 "identically to built-ins" vs docs/07's labelled origin | The row sits under Lumit's own heading; the provenance tag stays in the context menu | none; this is the reconciliation, not a compromise |
| D15 | Whether the substrate is shared or copied | `lumit-ipc` extracted for the three things already duplicated verbatim; proto, ring and handles stay per host | one migration of two shipping crates, provable here (§12) |
| D16 | §3.4's adaptive-concurrency policy, stated nowhere | §4.4's numbers, marked **provisional** | somebody has to write the first one; it will be wrong before it is right |
| D17 | The page's name - docs/07 §15 says *Plugins* | **Addons**, and docs/07 §15 and docs/TODO.md are amended to match | one name in two documents |

---

## 2. The C ABI

The canonical header is `include/lfx.h`, **MIT** - docs/12:373-375's deliberate carve-out
against the workspace's GPLv3, so a proprietary vendor adopts it without licence anxiety.
It lives in `lumit-lfx-abi`, a crate that contains **only** the header, the `#[repr(C)]`
Rust mirror and the layout tests. Nothing else may go in it: a helper that drifts in
becomes MIT by accident, and `cargo deny` checks what comes *in*, never what goes out.

### 2.1 The core, entire

```c
#define LFX_ABI_VERSION 1u          /* the integer that intends to reach 2 never */

extern const lfx_entry lfx_entry_point;      /* the one exported symbol */

typedef struct lfx_entry {
    uint32_t struct_size;  uint32_t abi_version;
    uint32_t (*init)(const char *bundle_path); /* control thread; after the manifest */
    void  (*deinit)(void);                    /* control thread */
    uint32_t (*count)(void);
    const lfx_descriptor *(*descriptor)(uint32_t index);
    lfx_plugin *(*create)(const lfx_host *, const char *id);
} lfx_entry;

typedef struct lfx_descriptor {
    uint32_t struct_size;
    const char *id;            /* reverse-DNS, stable for the plugin's life */
    const char *name;
    const char *vendor;
    uint32_t major, minor, patch;
    const uint32_t *categories; uint32_t category_count;   /* lfx_category */
    const lfx_traits *traits;                  /* §2.4; NULL = the pessimistic case */
    const char *const *required_extensions;    /* §4.3; the code's own answer       */
    uint32_t required_extension_count;
} lfx_descriptor;

struct lfx_plugin {
    uint32_t struct_size;  void *plugin_data;
    uint32_t (*init)(lfx_plugin *);
    void (*destroy)(lfx_plugin *);
    uint32_t (*describe)(lfx_plugin *, lfx_describe_sink *); /* control thread */
    int32_t (*process)(lfx_plugin *, const lfx_process *); /* any worker thread */
    const void *(*get_extension)(lfx_plugin *, const char *id, uint32_t version);
};
```

`lfx_host` carries the mirror-image `get_extension` and a `log`. That is the whole core.

**Every struct opens with `uint32_t struct_size`, with one named exemption.** Fields are
added at the end, never re-ordered, never removed - §3.1's freeze, made a mechanism rather
than a promise - so version 1's fields are always the first bytes of a later version's
struct and `struct_size` is read in the direction growth happens: a side handed a *longer*
struct than it was built against reads the fields it knows and ignores the tail.

**Short is a different question, and the header answers it per struct** rather than in one
sentence, because the first draft's one sentence - "a side reading a struct it was not built
against stops at the bytes it recognises" - is not what any reader in this tree does. A
struct shorter than the reader's own is missing fields that reader's version requires, and
version 1 invents none of them: `lfx_entry` and `lfx_plugin` are **refused**, a function
pointer that is not there being uncallable; a short `lfx_traits` is the pessimistic case
exactly as a `NULL` one is; a short `lfx_descriptor` or declaration record is declined with
a line in the scan report. The cost is stated in the header rather than discovered later: a
field appended to `lfx_entry` or `lfx_plugin` would leave every bundle built against version
1 short, so those two tables do not grow at all - they gain capability through
`get_extension`, which is what extensions are for.

**And no answer the plugin gives crosses as a C `bool`.** `lfx_entry.init`,
`lfx_plugin.init` and `lfx_plugin.describe` answer a `uint32_t` in which non-zero is true.
A Rust `bool` is a validity-invariant type and the host cannot inspect the byte a stranger's
compiler left in the return register before it reads it - and the in-process host's `LocalHost` runs the
plugin in the editor's own process, so the undefined behaviour would not be contained. It is
the choice §2.3 already makes for a switch's `uint32_t default_value`, taken the once more it
was needed. The `bool`s that remain are the host's own, read by the plugin rather than
written by it: the sink's answers and `lfx_process.cancelled`.

`lumit-lfx-abi/tests/layout.rs` asserts the size and offset of every field of every
struct **by number**, and `build.rs` compiles a small C translation unit that asserts the
same with `sizeof`/`offsetof`, so the two halves cannot drift. (`cc` already builds in this
workspace - `lumit-ofx`'s variadic shim proves it, and this is the workspace's second piece
of C rather than a first.)

**The exemption is `lfx_value`** (§2.2), and it is exempt because it is the one struct that
crosses as a dense array addressed by index: what the two sides must agree on is the
element **stride**, not where its fields start. A size prefix catches a truncated read; it
does not catch a plugin built against the older header striding by the older `sizeof` and
landing in the middle of the host's elements - correct-looking kind tags over silently wrong
values, which is not a failure the wrapper's kind check can see. So `lfx_process` carries
`uint32_t value_stride` and `uint32_t value_count`, and **the plugin indexes by that stride,
never by `sizeof(lfx_value)`**. The layout suite pins it as a case of its own: a host writing
an oversized stride is read correctly by a plugin built against the smaller header. Without
that, "a kind mismatch is impossible rather than a runtime status" is true at version 1 and
has no story at version 1.1.

**`lfx_traits` is reached by pointer rather than embedded by value**, which is the same rule
seen from the other end: two size-prefixed structs cannot both grow when one is nested inside
the other. Nesting it would work only while nothing followed it, and §4.3's
`required_extensions` pair already does: a field after the embedded block makes every later
`lfx_traits` field shift a `lfx_descriptor` offset, and an offset that depends on
the embedded `traits.struct_size` the plugin happened to be built with is not a number
`layout.rs` can assert. Nesting would leave the freeze a promise - that nobody ever adds a
field after `traits` - in the one place §2.1 says it is a mechanism. A pointer keeps
`lfx_descriptor`'s offsets literal, lets `lfx_traits` grow freely on its own prefix, and
gives D5's absent trait block a `NULL` that actually means it. (Nothing in the tree's
shipping cross-process layouts nests by value either: `shm.rs`'s `FrameHeader` is flat and
written by explicit offset.)

**Extensions** are typed function-pointer tables fetched by name and version:
`host->get_extension("lfx.temporal", 1)`. A missing one is `NULL`, never a status. That is
§3.1's central refusal - "no stringly-typed property soup" - made mechanical rather than
asserted. A plugin that *requires* an extension the host does not offer is refused before
it is instantiated (§4.3), not left to fail somewhere later.

### 2.2 Describe is a sink, not a property bag

The host hands `describe` an `lfx_describe_sink` and the plugin **pushes** typed records:
one call per kind (`lfx_declare_float`, `_slider`, `_int`, `_angle`, `_bool`, `_choice`,
`_colour`, `_seed`, `_point2`, `_point3`, `_curve`, `_file`, `_action`, plus
`_group_begin` / `_group_end`), each taking a struct with range, default, unit and flags.
There is no key, no string-valued answer, and no question the host can ask that a plugin
can answer in the wrong type.

At `process` the plugin receives a **dense array in declaration order**, addressed by index
and strided by `lfx_process.value_stride` - this is the one struct with no size prefix, and
the stride is why (§2.1):

```c
typedef struct lfx_value {
    uint32_t param;  lfx_param_kind kind;     /* the tag; checked by the wrapper */
    union {
        double  f;   int64_t i;   bool b;   uint32_t choice;
        float   rgba[4];  float xy[2];  float xyz[3];
        struct { const float *pt; uint32_t n; } curve;   /* unit-square points */
        struct { const char *path; } file;
    } v;
} lfx_value;
```

A kind mismatch is impossible rather than a runtime status. This, and nothing else, is
what LFX buys over OFX's untyped get/set - and it stays true across a growth of `lfx_value`
only because both sides walk the array by the stride the host wrote, never by their own
`sizeof`.

### 2.3 The parameter kinds, as a lowering onto Lumit's own vocabulary

**Built**, with §2.2's sink and §2.4's traits. `lumit-lfx/src/describe.rs` is the host's own
side of the sink - a plugin pushes a `Declaration` per control, each push answers `true` or
`false`, and `finish()` hands back the declarations in order, the headings over them, and
the report lines for everything declined. `lumit-lfx/src/schema.rs` is this table: the kinds,
the mandatory units, the eight families, the traits, and `value_routes` reversing exactly
what the rows were minted from - reading the rows **off** the built schema rather than
minting them a second time, because that map is what discovery turns a resolved bag into the dense
value array with, once per frame per instance, and a second minting there would leak one copy
of every row of every LFX instance per rendered frame. Nothing in either touches a raw
pointer: the C sink's thirteen declaration calls and two heading calls are filled in by
whoever holds the plugin - the in-process host in process, the broker out of it - and each turns one
`lfx_*_param` into a `Declaration`, which is what lets the whole lowering be tested with no
plugin, no process and no `unsafe`. The three conversions that are **not** field for field
live in `describe.rs` beside the declaration they make - a Float's and an Int's `bounds` mask
deciding whether `hard_min` and `hard_max` are meant at all, a Slider's `uint32_t log` and a
Bool's `uint32_t default_value` - so that the two edges cannot read one frozen struct two
different ways.

**Every ceiling the header declares, the sink asks**, which is the header's own sentence -
"the host enforces the same numbers where a stranger's bytes arrive, so the two sides agree by
construction rather than by coincidence". `LFX_MAX_OPTIONS`, `LFX_MAX_DIVIDERS`,
`LFX_MAX_FILTERS` and `LFX_MAX_STRING_BYTES` are report lines on the declaration that broke
them, and `LFX_MAX_CATEGORIES` and `LFX_MAX_REQUIRED_EXTENSIONS` on the descriptor - where
each is read **through** the truncation the line beside it claims, `families()` and
`schema::required_extensions()` being the one place each list is read from, as `padding_of`
already is for the ROI. The descriptor's own three strings meet `LFX_MAX_STRING_BYTES` at the
lowering, since that is where they arrive, and `LfxRejection::IdentityStringTooLong` is
structural where a row's is a line: a row this build cannot draw costs that one control, and
an id nobody can carry is the match name a project file would have stored. And two numbers a
dropdown declares are asked against the list beside them, because LFX declares them where OFX
can say neither - a default selecting an option that does not exist
(`ChoiceDefaultOutOfRange`) and a rule drawn after one (`DividerPastTheOptions`), each
declining the whole declaration rather than trimming it, which is the answer the header
already gives that same list. One
ceiling was missing and the describe lowering added it while the header was still free to grow: **`LFX_MAX_PARAMS`
= 512**, and going past it refuses the effect rather than costing the last row its control,
because the rows are leaked for the session and in process (the in-process host's `LocalHost`) there is no
watchdog to stop a describe loop that ran away. **Counted over declarations *pushed*, controls
and headings together, rather than over rows accepted**, which is the whole of what the ceiling
is for: every entry point files an owned string for a declaration it declines, so a loop of
declined declarations - a dropdown with nothing in it, a heading inside a heading, a close with
nothing open - grows the report exactly as a loop of good ones grows the panel, and a gate
reading the accepted count would never trip on the runaway. `lfx.h`'s own sentence beside the
constant was widened to say so. Two numbers a plugin declares are refused for
a reason of Lumit's rather than the ABI's: a range whose ends are not a number or are the
wrong way round (`RangeUnusable` - the resolve's clamp is `max(lo).min(hi)`, so a transposed
pair pins that control to one value for the effect's life), and a whole number outside the
`i32` the resolved bag holds (`WholeNumberOutOfRange` - the header says `int64_t` and the bag
says `i32`, and the backfill's cast truncates).

One structural refusal arrives that §14 item 3 does not list: a **declared temporal window
the host cannot honour is `LfxRejection::TemporalWindowUnusable`**, because `lfx_traits`
already says in the frozen header that a window not containing the frame being rendered is
refused rather than clamped, and a clamp would hand the effect a different window from the
one it says it reads - §2.4's seam by the other road. And the graceful degradations are
`LfxRejection` variants as well, as §2.3's own `PathNeedsOverlay` and `NoTextRow` always
were, filed in the report rather than returned: `refuses_the_effect()` is the exhaustive
match that says which kind each one is, so a variant a later package adds has to decide
rather than inherit an answer.

The single highest-leverage decision in the ABI, and it can only be taken at the header:
**`lfx_param_kind` lowers onto `lumit_core::fx::ParamKind`** rather than being a foreign
vocabulary the host later maps. *Lowers onto*, and not *mirrors*: there is nothing for a
frozen C enum to mirror. `ParamKind` is a data-carrying Rust enum with no `#[repr]` and so no
stable discriminants; its order is arrival order rather than a designed sequence - `Curve`
and `Action` sit at the end because they were appended - and the correspondence is
one-to-one in neither direction. `ColourName`, `Layer`, `Clip` and `MaskPath` have no LFX
counterpart, and `POINT2`, `POINT3` and `GROUP` are not `ParamKind` variants at all. The
table below *is* the lowering, and reads honestly as one. `lumit-ofx/src/schema.rs` is 662
lines largely of impedance mismatch; the *mapping* in `lumit-lfx/src/schema.rs` is a fraction
of that, which was the point. The file is not: it is longer than the OFX one, and the excess is
doc comments and the suite, which is where a first-party lowering puts its reasoning.

Which leaves one seam nothing watches. §14 item 1's "the ABI cannot drift" is delivered by
the layout tests, and those bind the header to the `lumit-lfx-abi` Rust mirror - they cannot
bind it to `ParamKind`, because `ParamKind` is not what the header is a copy of. So the
lowering gets a test of its own: **an exhaustive `match` over `ParamKind` in
`lumit-lfx/src/schema.rs` with no `_` arm**. The next variant added to `ParamKind` then fails
the LFX build and forces a deliberate answer - admit it, or name an `LfxRejection` - instead
of silently becoming an unrepresented row. It is §11 item 1's argument for `is_catalogued()`,
one level down.

| `lfx_param_kind` | becomes | note |
|---|---|---|
| `FLOAT` | `ParamKind::Float` | unbounded, with optional hard bounds |
| `SLIDER` | `ParamKind::Slider { log }` | **LFX can declare logarithmic; OFX cannot say it** |
| `INT` | `ParamKind::Int` | |
| `BOOL` | `ParamKind::Bool` | |
| `CHOICE` | `ParamKind::Choice { dividers_after }` | dividers **declared**, not guessed from the labels |
| `COLOUR` | `ParamKind::Colour` | RGBA, scene-linear |
| `ANGLE` | `ParamKind::Angle` | a first-class kind, not a `double` sub-type |
| `SEED` | `ParamKind::Seed` | how a plugin gets randomness and stays bit-identical between exports (docs/08 §2.5) |
| `POINT2` | two `Float` rows `<id>_x` / `<id>_y` | Lumit has deliberately no Point kind; `EffectSchema::pairs` folds them into one crosshair row |
| `POINT3` | three rows `_x` / `_y` / `_z` | |
| `CURVE` | `ParamKind::Curve` | the **tone** curve: 2..16 points in the unit square, static in v1 |
| `FILE` | `ParamKind::File` | the payload arrives **beside** the op, never in the bag - below |
| `ACTION` | `ParamKind::Action` | no value, no keyframe, no `EffectParam`; reaches `EffectDef::press` |
| `GROUP` | a `ParamGroup` run | no row of its own |
| `PATH` | - | **reserved, refused in v1** - `LfxRejection::PathNeedsOverlay` |
| `STRING` | - | **reserved, refused in v1** - `LfxRejection::NoTextRow` |

The last two rows are the point of a frozen enum. The resolved bag carries no text at all
(`resolve_into_arena` drops `File`, `ColourName`, `Layer`, `Clip`, `MaskPath` and `Action`
outright), so a string row would be a control whose value never reaches `process`; and a
bezier path with no on-Viewer handles is a control nobody can edit. Both discriminants
exist from day one, so admitting them when `lfx.overlay` lands adds no variant and breaks
no compiled plugin. **A named refusal the validator can test is not a gap - it is the
contract saying *not yet*.**

**The refusal is at the tag, not at a sink call.** The frozen sink has an entry point per
kind it admits and there is no `declare_string` and no `declare_path`, so a version 1 plugin
cannot make either declaration through the ABI at all - the door is simply not there. The
host-side seam is `Describe::decline_kind`, whose only possible caller is whoever decodes a
declaration that did not come through that sink: the broker, reading a proto message whose
kind tag this version reserves. A tag that is neither reserved kind - `LFX_PARAM_UNSET`, a
`GROUP` where a row belongs, a number from a newer header - is `LfxRejection::UnknownParamKind`
rather than a path, so the Addons page never calls a control something it is not.

**`FILE` is on that same list, and needs saying out loud rather than assumed.** The resolved
bag carries *nothing* for a File row - not the string and not a slot; `Params::file_slot`
exists and has no production writer anywhere in `crates/`, and
`the_arena_carries_no_file_slot_or_layer_binding` pins it. A `.cube` reaches a built-in as an
**aux slot beside the op**, decided by the render because only the render knows which cube
actually loaded (`fx/effects/lut.rs:38-41`). So `lfx_value.v.file.path` is filled from the
aux side or it is filled from nowhere, and that is a carriage rather than a sentence: today
only `AuxKind::Lut` and `AuxKind::LensFile` carry a file, each hard-wired to one built-in's
parallel list in `build.rs`/`fxops::run_ops`. LFX needs a **generic file aux** and admission
to `a_side_table_effect_declares_the_list_it_consumes`, and because the aux choice belongs to
`lumit-render` (§4.6) that lands in **the render pass**, not here. *The cheaper answer, if v1 does not
want the carriage, is to refuse `FILE` by name beside `STRING` and `PATH` -
`LfxRejection::NoFileSlot` - which costs a paragraph instead of a seam; it also costs D8 its
escape hatch, so it is a call for the maintainer rather than the note. What may not stand is the sentence
`lumit-ofx/src/def.rs:43-45` carries - that the bag carries the file-table slot rather than
the string. It carries neither.*

**The road taken is admission**, settled at the header (§12's paragraph on the ABI header) and carried
through by the describe lowering: `Carriage::Path` is a real arm, a `FILE` declaration mints a
`ParamKind::File` row and `value_routes` gives it an element of its own. What that buys is a
control an author can declare and a dialog a person can use; what it costs until the render pass is that
`lfx_value.v.file.path` is NULL for every one of them, which is the *ponytail* beside the
constant in `lfx.h` and the reason a plugin that cannot work without the path is better off
not declaring the row.

Degradation is graceful, per §3.6: a declared `STRING` is a line in the scan report (the
`unrepresented()` precedent) and **the plugin still loads**, keeping its own declared
default for that parameter forever. Only a structural fault - a duplicate `ParamId`, an
unset unit - refuses the effect. *ponytail:* a `ParamKind::Text` would take one bag
variant and one panel widget, and is the whole of what admitting `STRING` needs.

**Unit is mandatory.** `lfx_unit` mirrors `Unit` one for one and has no `UNSET` value a
plugin can ship in; `LFX_UNIT_UNSET = 0` is a describe refusal, mirroring the build failure
every built-in faces (`every_parameter_declares_a_deliberate_unit`). `LFX_UNIT_PX` means
**pixels at composition size** - never buffer pixels - and the header comment says so
beside the constant. This is the most useful thing LFX takes from being first-party: OFX's
`unit_of` must answer `Unit::Raw` for every normalised spatial type because the standard
does not say.

`LFX_UNIT_PCT_DIAG` exists because the ladder mirrors `Unit` value for value so the two
cannot drift, and for no other reason: version 1 has no consumer for it. It is **not** there
for the ROI padding declaration, which `lfx_traits.roi_padding_px` states in px@comp with no
unit field at all, and `lumit_core::fx::Roi` has no per-cent-of-the-diagonal variant to lower
onto. On a parameter it is a **describe refusal**, exactly as
`no_parameter_is_a_per_cent_of_the_diagonal` refuses it for a built-in. (The same sentence
was stale in `lumit-core/src/fx/params.rs`, where the header copied it from - it said the
member stays "for the ROI padding declarations", and `Roi::PaddedPx` is px@comp with no unit
field at all. The template corrected it there and in the test's own doc: it stays because the
reference format spells every member and because this ladder is what `lfx_unit` mirrors.)

And a unit declared on a kind that carries none is **normalised with a line in the report**,
not silently rewritten. A switch, a dropdown, a colour, a seed, a tone curve, a file and a
button are `Unit::Raw` and an angle is `Unit::Degrees`, which is the default
`#[derive(Effect)]` reaches for when a built-in's author says nothing - but that author may
then say otherwise with `#[dial(unit = Seconds)]` and a plugin's author may not, the header's
own words being that an angle's unit *must* be `LFX_UNIT_DEGREES`. A mandatory field whose
value is thrown away without a word is the "left to fail somewhere later" shape §2.1 exists to
prevent, so `LfxRejection::UnitIgnoredForKind` is the sentence the vendor's own validator run
prints. One predicate answers both ends - `schema::forced_unit` - so the row and the line
beside it cannot disagree.

**The same answer for `LFX_PARAM_FLAG_STATIC`**, which is the one other declared field this
build reads and has nowhere to put. The header says the flag means a row never keyframes,
"as a file choice or a curve is" - and a tone curve, a file choice and a button are exactly
that in Lumit already, so on those three the flag asks for nothing and says nothing. On every
other kind `ParamSchema` carries no non-animatable field, so the row is drawn keyframeable
whatever the plugin declared: `LfxRejection::StaticRowAnimatesAnyway` is the sentence, filed
where the unit's is, and `schema::animates` is the one predicate both ends read. *ponytail:* a
`ParamSchema` flag read by the panel and by the keyframe menu is the whole of what honouring
the declaration needs, and the ponytail beside the constant in `lfx.h` says so.

**Two rows on one `ParamId` refuses the whole effect** (`LfxRejection::DuplicateParamId`).
`ParamId` is a const FNV-1a hash and a collision is one control silently driving another;
a plugin's parameter names are not ours to choose, so the collision is made loud at
describe time rather than shipped as an ambiguity.

### 2.4 Traits, declared at describe

**Built** with §2.3's lowering: `schema::traits_of` is where every zero becomes the
pessimistic answer, and `a_memset_trait_block_schedules_as_heavy_and_full_frame` asserts it
against a `LfxTraits::default()` rather than assuming it.

§3.4 says the host schedules from declared traits and §3.1 never gives a plugin anywhere to
declare them. It declares them in the descriptor:

```c
typedef struct lfx_traits {
    uint32_t struct_size;
    uint32_t cost;      /* UNSET=0 | TRIVIAL | CHEAP | MODERATE | HEAVY  -> CostClass  */
    uint32_t roi_kind;  /* UNSET=0 | EXACT | PADDED | FULL_FRAME         -> Roi        */
    float    roi_padding_px;  /* px@comp; PADDED + this -> Roi::PaddedPx(f32), one variant */
    int32_t  temporal_lo, temporal_hi;   /* comp frames; the declaration gate             */
    uint32_t alpha;     /* UNSET=0 | PREMULTIPLIED | STRAIGHT     -> premultiplied: bool  */
    uint32_t flags;     /* SEEDED | THREAD_UNSAFE | CANCELLABLE                           */
    uint32_t scratch_bytes_per_megapixel;                /* §8's replacement ceiling      */
} lfx_traits;
```

**Every trait field's zero means "unstated", and that is the mechanism, not a convention.**
Written in the obvious order - the order it must be in to mirror the Rust enums it lowers
onto - zero would be `TRIVIAL` and zero would be `EXACT`, so a zeroed block would read as the
most *optimistic* case: trivial cost, no ROI expansion, the window `[0, 0]`, no declared
scratch. That is not a corner. §2.1's whole growth mechanism leaves the tail of a short
struct zeroed; a vendor's `lfx_traits t = {0};` before filling two fields does the same; and
D5's absent block is a `NULL` that has to lower to something. docs/13:297 is explicit about
which way the failure runs - "an undeclared effect is treated as the most pessimistic case.
Claiming less reach than the kernel uses produces tile seams and is a correctness bug." So
`LFX_COST_UNSET = 0`, `LFX_ROI_UNSET = 0` and `LFX_ALPHA_UNSET = 0` sit at the head of each
enumeration, and the host lowers UNSET to `CostClass::Heavy`, `Roi::FullFrame` and the
default premultiplied form - never to discriminant zero. It is the shape D4 already gives
unit safety with `LFX_UNIT_UNSET`; the difference is that an unstated unit is a describe
refusal and an unstated trait is a pessimistic lowering, because a trait can be answered
safely and a unit cannot be guessed at all. `lfx-validator`'s describe suite then asserts
what this section wants - **a memset trait block schedules as `HEAVY` / `FULL_FRAME`** -
rather than asserting the opposite by accident.

**And the `flags` bits are not inert.** `LFX_TRAIT_SEEDED` lowers to
`EffectTraits::seeded`, whose one reader in the tree is `lumit-eval`'s frame-key walk - which
asked `namespace == Builtin` before the describe lowering and asks `is_catalogued()` after it. Without that
widening a seeded plugin's whole span hashed to one key and a hash(seed, time) generator
rendered one frozen frame while the panel said it was animating;
`a_seeded_lfx_effects_key_moves_with_local_time` is where that is pinned, beside the two
the namespace wiring left there. It costs the older host nothing, because an OFX plugin has no way to declare
`seeded` and the bridge writes `false`.

**Alpha is two states, not three.** `EffectTraits::premultiplied` is a `bool`, so IGNORES and
STRAIGHT would both land on `false` and the host would unpremultiply before an effect that
did not want touching - a declaration that reads as scheduling information the host has
nowhere to put. Two enumerators mirror the bool one for one. *ponytail:* a third state that
genuinely bought something - skipping the unpremultiply/repremultiply pair outright - is a
change to the dispatch beside the pass, not a trait `EffectTraits` can carry today, and it is
not in v1.

`roi_kind` and `roi_padding_px` are two fields lowering onto **one** tuple variant,
`Roi::PaddedPx(f32)` - worth spelling, since the comment column spells the arrow for
`CostClass`. A `PADDED` with no padding is `Roi::Exact` and a report line.

The declared temporal window is what `stack_is_temporal` reads - it is the *gate*, and a
plugin that declares none never sees a neighbour however loudly `lfx.temporal` asks at
render time. `DeclaredTraits::temporal_window` (the protocol and ring) *holds* a declaration to
`LFX_MAX_TEMPORAL_WINDOW` and to containing frame nought, because the ring has to be sized
from a number somebody has checked; **holding is not refusing**, and the refusal by name is
the describe lowering's, at describe, where there is somewhere to put the sentence -
`LfxRejection::TemporalWindowUnusable`, which `schema::traits_of` raises - reached from
`schema_of` - for a window reaching past the ceiling or missing the frame being rendered.
(`lumit-lfx/src/schema.rs` has no `lower`; the name an earlier draft of this sentence used was
one.) All three halves are built: the refusal, the narrowing, and - with
the broker - the order between them. `ipc::broker`'s `admit` runs `schema::traits_of` over every
declared window that comes off the wire before anything is sized from it, and
`a_declared_window_the_host_cannot_honour_is_refused_before_it_is_narrowed` asserts both
sides of that: that the narrowing turns `[3, 5]` into `[0, 5]` and says nothing, and that
nothing reaches it. Claiming less reach than the kernel uses produces tile
seams and is a correctness bug; `lfx-validator` checks it by putting one bright pixel
outside the declared padding and asserting the output does not move.

**Categories are a closed picture-family vocabulary of eight**, mapping onto `FxCategory`'s
`grouping()` families a picture effect may claim: Blur & sharpen, Colour, Distortion,
Generate, Stylise, Temporal, Transition, Utility. `Audio`, `Drivers` and `Controls` are **not
in the enum** - an LFX picture plugin declaring "this is a driver" would be claiming a
`Signature::Data` it does not have.

**Nor is `Compositing`**, which is not a family a picture effect may claim at all.
`list_effects()` filters it out by name (`api/effect.rs:90` - "the Compositing family is left
out, as the drivers once were") and routes it to the node-graph console instead, further
filtered by `offered_in_graph`. An author picking a legal value out of the frozen enum would
get a plugin that installs, registers, badges nothing and sits in no menu a layer can reach -
failure 6's shape arriving through the category list. A Merge and a Switch join pictures a
graph's wires bring them and an LFX plugin has one input, so the family was never claimable;
leaving it in a **frozen** enum would mean never being able to take it out. An unrecognised
value lands in `Utility` plus a report line - one line per *distinct* number, since a list
declaring 99 three times is one mistake - so nothing else changes. `LFX_CATEGORY_UNSET` is not
one of those: it is the zero a vendor's `= {0}` left behind rather than a number from a newer
header, so it claims no family, adds no keyword, and a list of nothing else is
`NoCategoryDeclared`.

This is where LFX earns §3.7 and OFX cannot. An OFX grouping is somebody else's taxonomy,
so the bridge gives it an `ofx/<grouping>` heading of its own; an **LFX author is declaring
against Lumit's vocabulary**, which is something they can do and an OFX author cannot.
The first declared category is the schema's `category` and the Add-effect heading; the rest
are search keywords carried on the discovery record. The provenance tag stays in the row's
context menu (docs/07:2592) - the row sits under Lumit's heading, and the menu says where
it came from. That is the reconciliation §3.7 and §2.6 needed.

**No matte row.** `matte: MatteRole::None`, as OFX and the audio hosts declare, so
`role.param()` is `None`, no slot is filled and the generic dissolve never runs. Injecting
a Matte would put a control on the panel the plugin has never heard of and nothing would
consume. *The module doc at `lumit-render/src/gpufx/ofx.rs:23-30` currently says the
opposite in one sentence - "It consumes no matte of its own, so the generic dissolve …
spends it exactly as it does for any other effect on `MatteRole::Strength`" - which
contradicts `lumit-ofx/src/schema.rs:175`. The prose is the bug; the render pass corrects it.*

### 2.5 Depth

`lfx_pixel_format` is `LFX_RGBA_F16` or `LFX_RGBA_F32`, **both mandatory**, per §3.3, and
the host never converts between them to accommodate a plugin. Frames are scene-linear,
premultiplied, tightly packed, top-down.

The project has a third working depth - 8 bpc (docs/06:359-369) - which §3.3 does not name.
**An 8 bpc project sends fp16.** A plugin never sees an integer buffer, and this is not the
conversion the rule forbids: fp16's eleven-bit significand round-trips every 8-bit code
value, so the promotion loses nothing the project had, and the return trip's quantisation
is the project's own, which the next inter-node texture performs anyway.

### 2.6 Threading, and what the annotations are for

Verbatim from §3.4, with the host side named: `process` may be called from any worker
thread and on different instances of one plugin concurrently; **one instance is never
re-entered**; `describe` and instance lifecycle run on one host-designated control thread.
Every callback in every extension carries its allowed calling context in the header
comment, and `lfx-validator`'s stress scheduler calls each from a disallowed thread and
requires a refusal.

The sole, discouraged opt-out is the **`LFX_TRAIT_THREAD_UNSAFE` bit** in §2.4's trait
block, which serialises the bundle. It is a trait flag and **not** an extension id: there is
no `lfx.thread-unsafe` table, and asking `get_extension` for one returns the NULL that means
"not offered" - which would leave the plugin scheduled concurrently, the opposite of what it
asked for. The ABI header settled that and the template amended this sentence and docs/12 §3.4 to
match. Multi-frame rendering is mandatory and is §4.4's instance pool.

---

## 3. The broker

The substrate is OFX's, copied verbatim where it applies, because it is the proven thing.
What is new is named as such.

### 3.1 One extraction, because the third copy is where it stops paying

**Built.** `crates/lumit-ipc` - `pipe.rs` (the transport), `spawn.rs` (`broker_exe`,
`no_console`), `rules.rs` (the shared numbers and `DISABLED_REASON`) and `hosts.rs` (the
reservation the distinctness test reads). Each host's three strings live in its own
`ipc/identity.rs`, and its `ipc::pipe` is the shared module wearing the host's name, so
every existing path - `lumit_ofx::ipc::pipe::send`, `lumit_ofx::DISABLED_REASON`,
`lumit_aplug::STRIKES_BEFORE_DISABLED` - still resolves. Neither host names `interprocess`
any more.

`ipc/pipe.rs` is near-identical between `lumit-ofx` and `lumit-aplug`, and the
comment-stripped diff is not empty: it is exactly one hunk, and that hunk is `pipe_name`
hard-coding the host's own prefix in both branches - `lumit-ofx-{identifier}.pipe` /
`.sock` against `lumit-aplug-{identifier}`. The same is true of the two symbols beside it.
`broker_exe` (ofx `broker.rs:111-120`, aplug `:198-206`) and `no_console` (ofx `:130-142`,
aplug `:217-226`) are genuinely identical; `BROKER_EXE_ENV` (`LUMIT_OFX_BROKER` against
`LUMIT_APLUG_BROKER`) and `broker_exe_name()` (`lumit-ofx-broker` against
`lumit-aplug-broker`) are **per-host identity strings sitting inside the functions being
lifted**.

So this is a **parameterisation, not a move**, and calling it a move understates it by
exactly the API design those three strings need. Lifting them as they stand would put all
three brokers' endpoints in one namespace - `lumit-ofx-<token>.sock` for the lot - and let
`LUMIT_OFX_BROKER` redirect the LFX broker's executable: the collision §14's test 5 exists
to catch, arriving through the extraction meant to be the safe part.

`lumit-ipc` takes them as arguments instead:

```rust
pub fn pipe_name(host_prefix: &str, identifier: &str) -> String;
pub fn broker_exe(exe_name: &str, env_var: &str) -> PathBuf;
```

and each host keeps a three-line module of its own constants - `HOST_PREFIX`,
`BROKER_EXE_ENV`, `broker_exe_name()`. **The existing per-host names stay byte for byte**,
so migrating the two shipping hosts changes no endpoint name and no environment variable
that a test or a packaging step already depends on; `lumit-lfx` adds `lfx` as the third
caller, with `lumit-lfx-broker` and `LUMIT_LFX_BROKER`. `send`/`recv` are already `pub` and
already generic over the message type, and those two really are a move.

Beside them `lumit-ipc` holds the rules both hosts must agree on rather than merely share:
`MAX_MESSAGE_BYTES`, `HANDSHAKE_TIMEOUT`, `STRIKES_BEFORE_DISABLED`, `describe_deadline` -
and `DISABLED_REASON`, which is a shared rule for the same reason the others are and is read
across the crate boundary by string equality today (§4.3).

It does **not** take the handshake driver: `Ready`, `Challenge` and `Hello` are variants of
each host's own protocol enum, and sharing them would mean a trait each per-host enum
implements - an abstraction for its own sake. The *ordering* rules travel as doc comments
instead, and each host writes its sixty lines of handshake. Proto, ring and handle registry
likewise stay per host, for the reason audio-plugins.md §5 already gives: picture frames and
audio blocks share nothing, so a common ring would be an abstraction with one and a half
users. §5's sentence is about the parts that are about sound; the transport underneath them
is this crate, and §5 now says so rather than saying no code is shared at all.

### 3.2 The protocol

**Built.** `lumit-lfx/src/ipc/proto.rs`, `PROTOCOL_VERSION: u32 = 1`, versioned
**independently of `LFX_ABI_VERSION`** - §3.5's promise, made structural by being two
constants in two crates, and pinned by a test that reads the declaring line and refuses to
find the ABI in it. The two rules the strike machinery rests on are predicates on the enums
rather than sentences about them: `HostMessage::expects_reply` and
`BrokerMessage::is_interim`, each an exhaustive match with no `_` arm, so a message added to
the vocabulary has to say which it is. A depth on the wire is `lfx_pixel_format`'s own
number and a value's tag is `lfx_param_kind`'s, read from `lumit-lfx-abi` rather than
repeated here.

Three more things are structural rather than remembered. The two lists the rules are
asserted over are pinned to their enums by `HostMessage::name` and `BrokerMessage::name`,
exhaustive matches with no `_` arm, so a variant added tomorrow fails the match or the
comparison rather than quietly going unasserted - the drift this section's own comment had.
`CreateInstance` names its plugin by the descriptor's **id**, never by a position in
`Described`, because `Describe{disabled}` makes that list's membership depend on the disable
set and §3.5's replay is exact only if the name survives it. And the broker's direction is
held to the header's ceilings as it comes off the pipe - `BrokerMessage::checked`, with
`LfxRejection::PastCeiling` naming which ceiling and how far past it: a manifest or a module
longer than `LFX_MAX_EFFECTS_PER_BUNDLE`, a string past `LFX_MAX_STRING_BYTES`, a category or
extension list past its own number are refusals; a note past `LFX_MAX_LOG_BYTES` is cut on a
character boundary rather than refused, because a plugin that says too much should still be
heard; and `frames_needed` is clamped to `LFX_MAX_TEMPORAL_WINDOW` and deduplicated, which
bounds the list by construction. Before it, 8 MiB of transport cap was the only number
between a stranger and the Addons page.

Those ceilings reach **inside** a declaration as well as around the list of them.
`DescribedPlugin::checked` walks every string and every list a control declared - the id, the
label, each dropdown option, each file filter, the dividers, the curve's points - through the
same `bounded_string`/`bounded_count` the identity goes through, because the sink that already
asked those numbers ran in the broker and the broker is the process holding the stranger's
compiled code. Counting the rows and keeping their contents would leave a label of eight
kilobytes bounded by nothing but the transport cap, and the declarations are what
`schema::lower` leaks for the session: a rescan re-describes, so an unbounded string kept once
is an unbounded string leaked once per scan.

The describe *payload* is where the two planes meet the describe lowering: `Described{plugins}`
carries each plugin's identity and its raw trait block - the two halves §4.3's re-check
compares and the numbers §3.4's ring sizes itself from - and the declaration records the sink
pushed arrive with the lowering that reads them. The broker joined them, and in joining them widened the payload: `DescribedPlugin` carries the
declarations, the headings and the report lines beside the identity and the trait block,
because the sink runs in the **other process** and a record that carried only an identity
would leave the host with a plugin it could name and no rows to put on a panel.
`PluginDescriptor::from` is the one conversion back, so the lowering reads one shape whether
the plugin was opened here or over a pipe. `Described` gained a `report` of its own for the
lines that belong to the *bundle* rather than to any plugin in it - a descriptor with no
readable id, an id two descriptors both declare - which have nowhere else to go. And a
`refused` of its own beside it, for the plugins the describe **turned away**: the describe runs
in the second process, so a plugin dropped there would otherwise reach the host as an absence
and nothing else - not catalogued, not refused, not in the report, and indistinguishable from
one the user switched off, which leaves §5.3's `REFUSED` table with no sentence to carry.

Host → broker: `Challenge{nonce, proof}` · `Open{ring}` · `Manifest{path}` ·
`Describe{disabled}` · `CreateInstance{instance, plugin, values}` · `Values{instance, values}` ·
`Action{instance, param}` · `Process{instance, request}` · `Frames{frames}` ·
`Destroy{instance}` · `Shutdown` - where `CreateInstance`'s `plugin` is the descriptor's own
id, never its place in `Described`.

Broker → host: `Ready{nonce}` · `Hello{version, proof}` · `RingOpened` | `RingRefused` ·
`Manifested{entries}` · `Described{plugins}` · `Created` · `Processed{slot, frames_needed}` ·
**`Done`** · `NeedFrames{frames}` · `Note{kind, text}` · `Failed{action, message}`.

**`Done` is the plain acknowledgement, and it is not optional.** `Values`, `Action` and
`Destroy` have no other answer - `Done` is what the OFX broker replies to `ParamSnapshot` and
`Destroy` - and §3.5's strike machinery is carried over unchanged from `Broker::action`,
which sends one message and blocks in `exchange` until something that is not a `NeedFrames`
or a `Note` arrives, turning a deadline with no reply into a strike. Drop `Done` and each of
those three becomes a *guaranteed* 2 s control-deadline timeout that also restarts the
broker, so a user dragging one slider three times disables the plugin. Either the vocabulary
carries the acknowledgement or the strike machinery is not carried over unchanged; it cannot
be both.

`Frames{frames}` is deliberately the one host message with **no** reply, because it is
consumed inside `exchange`'s own loop rather than through `action`; `Shutdown` is the other,
because by then there is nobody left to answer. §14's test 5 pins the rule in that shape.

No pixels on the control plane: every picture is a `FrameRef { time, slot }` naming a ring
slot. `MAX_MESSAGE_BYTES = 8 MiB`, checked **before a byte is allocated for the body**. And the
frame that comes back is the one that was asked for: the host reads the slot **it** chose
rather than the one the answer names, and holds the slot's header to the job's own bounds. The
ring checks a header against itself, which cannot see a well-formed one-pixel frame where a
whole picture was asked for, and a broker naming the *input* slot would have the input served
as the render with no strike, no sentence and no badge.

### 3.3 Who is on the pipe, and when the code runs

**Built**, with the broker. `crates/lumit-lfx-broker` is the second program - two arguments, the
payload and the pipe name, and its first word a nonce - and `lumit-lfx/src/ipc/broker.rs` is
the supervisor that starts it, holds it to a deadline, strikes it and replays into its
replacement. `lumit-lfx/src/manifest.rs` is the listing, parsed **in the broker** under
`Limits::PLUGIN_MANIFEST`, and `manifest::agrees` is the re-check §4.3 asks for, run at the
first describe with the module finally open.

Unchanged from docs/12 §2.3, and the ordering *is* the security property: the listener is
created before the spawn; the endpoint name is 128 bits of OS randomness via
`lumit_peer::Token`, never a process id, and `listen` claims the name rather than clearing
a stale one; the 256-bit secret goes down the child's stdin, never the command line; the
broker's first word is `Ready{nonce}`; the host checks the **proof before the version**, so
an impostor is not told which build it faces.

Two things are stricter than OFX: the first is what makes failure 6 answerable at all,
the second is what keeps a switched-off plugin's code from running.

**The manifest is read before any of the plugin's code.** `Contents/lfx.toml` declares the
bundle's plugin ids, names, vendors, versions, categories, ABI version and required
extensions. It is read **in the broker** - parsing a stranger's structured text in the
process that holds the project, the media handles and the windows is the one thing the
broker architecture exists to prevent - under `lumit-ingress` with a new
`Limits::PLUGIN_MANIFEST`, and answered as `Manifested{entries}`. The module is not opened
to answer it. That is what lets the Addons page name, label and re-enable a plugin whose
code has never run.

**The module is opened lazily, on the first `Describe`**, which carries the disable list -
`lumit-aplug`'s ordering, not `lumit-ofx`'s, where `Bundle::open` + `load` run immediately
after the handshake and a switched-off plugin's `kOfxActionLoad` fires anyway. A manifest
that disagrees with the entry struct once the module *is* open is
`LfxRejection::ManifestMismatch`: the manifest is the cheap listing, never the authority.

### 3.4 The ring, sized by depth and charged to the ledger

**Built.** `lumit-lfx/src/ipc/ring.rs` is `lumit-ofx::ipc::shm`'s design with **three**
changes - how a slot is sized, who pays for the ring, and the one this note first called two
because it was inherited rather than designed: **a header must describe its own payload.**
The OFX reader ends `read_frame` in `Frame16::from_f32` → `from_pixels`, which refuses a
pixel count that does not match the size it was handed (`lumit-ofx/src/image.rs:136-140`);
the LFX reader hands back a bare `Vec<S>`, so dropping that check would let the same
untrusted header the older host refuses through. Bounds, row bytes and payload length are
therefore read together and refused together - `RingError::WrongPayloadBytes` and
`RingError::WrongRowBytes` on the read, where a stranger's header arrives, and
`RingError::WrongSampleCount` on the write, so the host cannot mint the lie either. The
consumer that would walk a 4K rectangle over sixteen bytes of stale slot is `LfxDef`'s readback
and the render pass, two packages away, which is exactly why the refusal belongs in the header
rather than in whoever gets there first.

**Slots are sized by the frame's depth**, carried in the 64-byte slot header beside magic,
version, bounds, row bytes, premultiplication, payload length and the FNV-1a hash - the one
failure shared memory has, made loud. The reader checks that hash, and checks the header
against **itself**: a hash says only that the bytes are the ones the writer meant, and the
writer at the other end of the mapping is a stranger's compiled code, so a header claiming a
4K rectangle over sixty-four honestly hashed bytes is refused rather than handed back as a
frame whose bounds walk the caller off the end of it. The rule rather than the numbers,
because a number in prose drifts and this one already has:
`slots = floor(RING_BUDGET_BYTES / slot_bytes)`, clamped to `[RING_MIN_SLOTS, RING_MAX_SLOTS]`
= `[3, 64]`, over `slot_bytes = w × h × 4 × depth_bytes + 64`. The protocol and ring's test
generates the table into `crates/lumit-lfx/ring-slots.txt`, regenerated by
`cargo test -p lumit-lfx regenerate_ring_slot_table -- --ignored`; the note prints it only
as an illustration:

| | fp32 slots | fp16 slots |
|---|---|---|
| 1080p | 16 | 32 |
| UHD 4K | 4 | 8 |

*The doc comment at `lumit-ofx/src/ipc/shm.rs:51-52` said fifteen and three. Both cells were
off by one against `Ring::create`'s own arithmetic three lines below them, and the halving
rule proves it from the inside: if an fp16 slot is exactly half an fp32 slot, then 32 and 8
imply 16 and 4. The protocol and ring corrected the comment and left
`the_ring_budget_buys_the_slots_the_comment_says` beside it, because nothing depending on
those numbers is how they drifted.*

**And the 4K prefetch ceiling is declared rather than relieved.** `Broker::process` refuses a
whole shipment, before a slot is written, when the job carries more pictures than the ring has
**slots**, not more bytes than it has - so halving the slot size does not lift it. A `t ± 5`
prefetch is eleven frames; eight fp16 UHD slots is still fewer than eleven, before the output
slot and the frame already in hand are counted, so an fp16 4K project running this very design
refuses the same prefetch the OFX host refuses, for the same reason and with the same
consequence. Two answers are available, and the second is available only to LFX:

- raise `RING_BUDGET_BYTES` - eleven fp16 UHD frames plus an output is ≈ 800 MiB, which the
  ledger reservation below now makes accountable rather than invisible; or
- **size the ring from the declared window.** `lfx_traits.temporal_lo/temporal_hi` is known
  before the first `Process`, so `Ring::create` asks for `hi − lo + 2` slots and `try_reserve`
  walks it down under pressure. A plugin that declares nothing gets the floor and knows why.

**The protocol and ring took the second**, and took it as a *raise* rather than a replacement: `slots_for`
asks for the larger of the budget's own answer and the declared window's, so the table above
still holds for a plugin that declares nothing - at 4K that is four fp32 slots or eight fp16
ones rather than the floor - and a `t ± 5` declaration asks for twelve whatever the depth.
The ledger is then the thing that says no to the *bytes*, which is what makes ≈ 800 MiB of
UHD prefetch accountable rather than either invisible or forbidden. It is not the first
thing that says no, and the order is worth writing down: `slots_for` clamps to
`RING_MAX_SLOTS = 64` before any ledger call, while the header admits
`LFX_MAX_TEMPORAL_WINDOW = 64` each way - so a declared window wider than ±31 is held to
the ring's own ceiling, and only what survives that is put to the ledger. From `slots_for` the two answers read alike, because it returns
a number; telling a budget-shaped no from a ceiling-shaped one is a report line, and the broker
prints it. `ipc::broker`'s `ring_report` holds all three numbers - what the window asked for,
what the ceiling left, what the ledger paid for - and files
`LfxRejection::WindowHeldToTheRing` or `LfxRejection::RingNarrowedByTheLedger`. Neither line
refuses the **bundle** - an effect taken away over a scheduling detail is gone - and both
describe a ceiling that refuses **frames**: nothing stages a wide prefetch across more
journeys, because version 1 has no frames-request seam to stage one through and `Process`
ships every neighbour in the one request, so a shipment wider than the ring's slots comes back
`BrokerError::RingTooSmall` and that frame renders identity with a badge. The line and the
refusal therefore say the same thing to two readers, the Addons page and the layer, which is
the only arrangement in which an operator reading one is not being told the opposite of the
other. *ponytail:* staging a wide prefetch over several journeys is the better answer and it
needs that seam; until it exists, the note says ceiling rather than schedule.

The depth-sized ring is a real win at 1080p and for every smaller window; what it is not is
the removal of a ceiling counted in slots. That ceiling is the honest form of §3's "the fp16
working format without conversion": **declared before the first frame instead of discovered
at one.**

File hygiene is copied exactly: `create_new` (never create-and-truncate, so a planted
symlink cannot be followed), mode `0600` on Unix, `FILE_FLAG_DELETE_ON_CLOSE` on Windows,
and the name unlinked as soon as the broker answers `RingOpened`, so the kernel reclaims it
on a crash or a `kill -9`. A restarted broker therefore always gets a new ring, which is
consistent with a restart being a replay rather than a recovery.

**The ring pays the governor.** docs/13 §3 says an unaccounted frame allocation fails code
review, and neither existing host depends on `lumit-budget` - so the OFX ring is up to
512 MiB of RAM the ledger has never heard of. `lumit-lfx` depends on `lumit-budget`, and the
signature is `Broker::spawn(config: BrokerConfig, ledger: &Arc<Ledger>)` - **an `&Ledger`
cannot reserve.** Every reserving method takes `self: &Arc<Self>`, because a `Reservation`
owns an `Arc<Ledger>` so that `Drop` can give the bytes back, which is the very property
being relied on here; taking a `&Ledger` would not compile, and the obvious patch-over -
reserving through a ledger cloned from nowhere - is what would break the give-back.
`Ledger::new()` already hands back `Arc<Self>`, so the composition root has the `Arc` and
nothing else changes.

The `Reservation` in `Tier::Ram` for `slots × slot_bytes` is held in the same struct as the
mapping - lumit-budget's own rule, "keep it beside the thing it paid for" - which in the
built shape is the `Ring` itself: `Ring::create(path, plan, &Arc<Ledger>)`, and the bytes go
back through the reservation's destructor when the ring does rather than through the broker
remembering. `try_reserve` answering `None` halves the slot count, and **`RING_MIN_SLOTS` is
a floor the ledger may not push through**: three slots are taken whatever the ledger says,
because a ring of two cannot hold one input, one output and one in flight at once. Taking
them stands on the ledger's own denial count rather than passing silently. That is §14.6's
"a ledger with no room buys three slots rather than none"; the bundle is skipped only when
the mapping itself fails, which is a different sentence. Regrowing - a wider ring when a
frame bigger than a slot arrives - re-reserves before it maps, which with the reservation
inside the `Ring` means dropping the ring being replaced first, or the ledger is asked for
both at once. The regrow itself is the caller's and landed with the broker: `Broker::fit` drops the
ring it is replacing before it asks for the new one, and `RingPlan::max_of` is what keeps a
regrow for a bigger frame from quietly giving back the slots a temporal plugin was
promised - a ring is raised and never lowered. **And whether to regrow at all is asked against
the plan the current ring was made from, never against the slots it got.** The two differ
exactly when the ledger narrowed it, which is the case below; measuring the wish against the
answer finds that ring too small for the very plan it *is*, and rebuilds it - file, mapping,
refused reservation and a control round trip - once per frame for as long as the pressure
lasts. A ledger's no is an answer, not a question to put again at every frame. And a ring that
could not be made **at all** - a full disk, no file handles left - leaves this broker holding
none, so the plan is committed only once a ring exists to match it and the failure goes to the
watchdog rather than standing as a `NoRing` for every frame after it: the strike replaces the
broker, which builds a fresh ring at the plan that last worked, and a machine that cannot give
this session a ring at all reaches three strikes and is put away rather than badging every
frame until the session ends.

One thing the paragraph above leaves implicit is worth saying out loud, because it is the
one place the ledger and the ring genuinely disagree. `Ledger` has no method that takes
bytes it has not got, and inventing one would make every other caller's ceiling a
suggestion - so a machine with nothing left gets its three slots **unbilled**, and the
refusal stands in `Ledger::denials(Tier::Ram)` instead. The three slots are visible in the
governor's own count rather than invisible in the allocator, which is the narrower promise
and the honest one, and `Ring::reserved_bytes()` answers nought there so that a caller which
wants to say so can. *ponytail:* `Broker::fit`'s ordering - the old ring dropped before the
replacement is asked for - is argued in its own doc comment and reached by
`a_declared_window_sizes_the_ring_the_second_process_maps`, which does not arrange a ledger
with room for one ring and not two; a case that did would fail on the wrong ordering rather
than merely not exercising it.

This makes LFX correct and leaves the two older hosts visibly wrong. That asymmetry is
itself the defect, not LFX's over-engineering, and retrofitting them is named in §13.

### 3.5 Ceilings, watchdog, restart

**Built**, with the broker, and carried over unchanged: `MAX_LIVE_INSTANCES = 1024` per bundle
(which bounds the crash-replay as well as the memory), `MAX_NOTES = 64`, a 10 s process
deadline and 2 s
control deadline from a `quirks.json` embedded with `include_str!`, `describe_deadline =
HANDSHAKE_TIMEOUT.max(control_timeout)` because the first describe opens a module from
disk, and **three *consecutive* strikes** before a session disable - a success resets the
count; a refusal strikes without restarting, a timeout or a dead pipe strikes and restarts.
**A success is an answer that was accepted, not an answer that arrived**, which is why the
reset is `Broker::accepted` at the seven call sites rather than a line inside `action`: a
`Processed` naming a slot the host did not choose, or carrying a 1×1 rectangle where a whole
picture was asked for, is a reply of the admitted *kind* and the wrong answer, and only the
caller that reads the content knows. Resetting on the kind alone left the two misanswers that
are not out-of-turn replies counted as a success at every frame, which is the same
unreachable "three *consecutive*" the paragraph below is about, arriving one level further
in.
One rule the older hosts leave to their callers is read off the message here instead: a
message the protocol says goes unanswered is never *waited* on, because a wait with no reply
coming is a guaranteed strike, and `HostMessage::expects_reply` is what `action` asks rather
than a list somebody keeps. **And the answer is held to the question** off the same message,
through `HostMessage::answers`: a reply the message does not admit is not a success but two
ends out of step, with the answer this question was owed still on the pipe for the next one to
collect - so it strikes, and it strikes as a suspect process rather than as a refusal. Counting
it as a success is what puts the strike count back to nought, and a broker answering nonsense
for ever would then never reach three *consecutive* strikes at all. The seven messages a
render or a control action sends are held to their lists by `action`; the two the handshake
sends are held at the handshake, `Open` by `ring_is_shared` reading that same list - the one
exchange outside `action`, and the one place an answer out of turn could be taken for "the
ring is mapped" - and `Challenge` by the exhaustive match that takes a `Hello` and refuses
everything else **by name**. `BrokerError::Unexpected` carries a `BrokerMessage::name` at
every one of the nine, never a sentence about one, so a caller can tell which message arrived
rather than reading prose out of a field that sometimes holds a name.

Handles take `lumit-aplug`'s newer shape - `u32` with magic, a four-bit kind and a
twenty-bit index, minted by the **host** and only quoted back by the broker, so a restart
can replay in-flight ids. Validated before every lookup; a forged handle is **answered**,
never followed, and `Failed` is the answer at every entry point rather than "unsupported",
which would tell a plugin the feature is missing when the truth is its handle is rubbish.

The rule has two halves and they are kept in two places. In the **broker**, `Values`, `Action`,
`Process` and `Destroy` each answer `Failed` for a handle it never minted - `Destroy` included,
because a rule kept only by its caller is the shape a handle registry exists to avoid. In the
**host**, every entry point taking a handle answers `NoSuchInstance` without touching the pipe,
because the host holds the record for every live instance and a message about one it has never
heard of is a message with nothing to say. The second half is not tidiness: a `Failed` is a
strike, so a press racing a layer deletion - press, destroy, press, press - would cost three
consecutive strikes and take the whole bundle away for a button the user was entitled to press.
Once it is kept, nothing in the shipping path can reach the broker's half, which is why the
test that proves it goes through a named seam rather than through `press`.

**A restart is a replay, not a recovery**: kill, fresh ring, fresh child, re-manifest,
re-describe, re-create every instance from host-held records - **by plugin id, never by a
position in `Described`**, since the re-describe carries whatever disable list the session
now has and a list that has lost a row renumbers the rest of it. An id the broker has not got
is a `Failed`; an index it has not got is, at best, the wrong effect. That is exactly why
parameter ownership is non-negotiable, and why D8's "no state blob" makes the replay complete
rather than approximate.

---

## 4. Becoming a catalogue effect

### 4.1 The namespace seam, and the bug it exposes

**Built.** `EffectNamespace::is_catalogued()` is on the enum in `lumit-core/src/model.rs`,
written as an exhaustive `match` with no `_` arm so the next namespace added fails that build
rather than becoming a silent identity, and every walk that admits a picture effect asks it.
There are **five, in three files**: `fx/resolved.rs`'s arena filter, the three in
`fx/temporal.rs` (`stack_temporal_window`, `input_times`, `stack_is_temporal`), whose own
private `is_catalogued` is gone, and `comp_frame_key`'s per-effect block in `lumit-eval` -
which the namespace wiring left on `== Builtin` and the describe lowering converted, below, so the enumeration is five rather
than the four this paragraph first claimed. The `== EffectNamespace::Builtin` gates that
remain are deliberate and of two kinds: lookups for **one built-in by name** (Posterize time,
accumulation motion blur, Extract channels, Flow neighbours, Flash, the schema migrations,
and several in `lumit-render/src/build.rs`), and the three walks in
`lumit-render/src/build.rs` that collect a `ParamKind::Layer`, `Clip` or `MaskPath` row -
kinds no hosted schema can declare, here or in the OFX host, so widening them would admit
nothing. `EffectNamespace::is_catalogued`'s own doc carries that enumeration, so the next
namespace reads a checkable list rather than deriving one. The
`LFX_MATCH_PREFIX` constant sits beside the other three in `fx/builtins.rs` with its arm in
`namespace_of`. The version arithmetic is `lumit_lfx::version::mint`, which is the first module
of the host crate, with `LfxRejection::VersionOutOfRange` beside it; the rest of
`crates/lumit-lfx` arrives with the describe lowering and after. §14's item 8 is five tests, four of them here:
`an_lfx_instance_reaches_both_picture_walks` and `only_the_picture_namespaces_are_catalogued`
in `lumit-core`, `an_lfx_retimers_sampled_frames_are_what_its_key_depends_on` and
`an_lfx_effect_keys_without_a_plugin_state` in `lumit-eval`. The describe lowering added the fifth:
`a_seeded_lfx_effects_key_moves_with_local_time`, where the same walk's seeded-effect block
learned to read `is_catalogued()` rather than `Builtin` (§2.4). The first of the four grew
its `input_times` half later still: three of the walks were asked of a registered LFX
instance and the graph's time demand was not, so reverting that one gate left the whole suite
green - the walk a review has to read for is the walk nothing drives.

**And the badge caught up with `LfxDef`.** The three `badge_of` edits §4.3 lists landed with the
catalogue entry, so an `Lfx` instance whose name nothing answers to now badges
`plugin_missing` - "this plugin is not installed on this machine" - rather than
`unknown_effect`, and one the scan turned away badges `plugin_refused` with the refusal's own
words underneath. `an_unknown_effect_is_a_badged_placeholder_and_never_an_error` asks the
`lfx:` name the same question it already asked the `ofx:` and `clap:` ones, and
`a_refused_lfx_plugin_badges_the_refusal_rather_than_missing` is the *order* - the branch
trap 10 is recorded for, which prose alone was holding. A bridge suite has no bundle, no
broker executable and no second process, so it drives the `REFUSED` table through
`lumit_lfx::discover::for_test::file_refusal`, a seam that exists for that case and no other
caller; move the refusal lookup below the namespace fall-through and it is what fails. It is
`#[doc(hidden)]` and under a module that says what it is, because a function that files a
refusal against an installed, working plugin is not something the shipping API should offer
beside `scan` - and "one caller" is a rule a test keeps rather than a doc comment:
`the_refusal_seam_is_named_in_two_files_and_no_others` reads every crate's `src` and fails on
a third.

`EffectNamespace::Lfx` has existed since `model.rs:986` and owns frame-key tag byte 2
(`lumit-eval/src/lib.rs:731`). Nothing else in the tree mentions it. Three edits, and the
third is a bug fix:

1. `pub const LFX_MATCH_PREFIX: &str = "lfx:";` beside the other three in `builtins.rs:74`,
   and an `Lfx` arm in `namespace_of`. Without it an `lfx:` name instantiates as `Builtin`.
   The match name is `"lfx:"` + the descriptor's reverse-DNS id: provenance carried by the
   prefix, spelled in one place, as for every other host.
2. `EffectKey.version` is **minted** as `major × 1_000_000 + minor × 1_000 + patch`, so any LFX
   release re-keys frames. It reaches the frame key twice - off the stored key in `lumit-eval`,
   and off the schema in `ResolvedFx::feed_hash`. This is an **LFX decision, not an existing
   property of the seam**: `EffectSchema::version` is a plain `u32`, and the OFX bridge sets it
   to `plugin.version.0` - the major alone, because `PluginDescriptor::version` is a
   `(u32, u32)` pair that never captures a patch number at all. The hosted path therefore
   re-keys on a major bump only today, which is a live staleness hole recorded in §13 rather
   than a premise this note may lean on. The thousands are deliberate: `× 10_000 + × 100` is
   **not injective** - patch 150 keys identically to (minor + 1, patch 50), two releases
   sharing a frame key and serving each other's cached frames. Both roads to that collision are refusals at describe,
   not clamps: `LfxRejection::VersionOutOfRange` covers `minor >= 1_000 || patch >= 1_000`,
   which would borrow the next component's digits, **and** `major >= 4_294`, the first major
   whose own block of a million does not fit a `u32` whole. That is one release short of where
   the arithmetic gives out, and deliberately so: the `u32` counts to 4 294 967 295, so
   4294.0.0 is a number it holds perfectly well, but 4 293 is the last major it holds *entire*
   and the domain stops there so the accepted set is exactly the injective one - injective by
   refusal rather than by hope, with no admitted corner left over. What the bound rules out is
   clamping: saturate there instead and 4294.968.0 and 4294.969.0 both mint `u32::MAX`, two
   releases sharing a frame key, the same failure by the other road. Inside the bound the
   arithmetic is plain rather than saturating, so an edit that breaks it fails a debug build
   instead of quietly clamping.
3. `resolved.rs:344` and `temporal.rs:342` both spell `matches!(ns, Builtin | Ofx)` as
   literals. **Replace both with one `EffectNamespace::is_catalogued()`** on the enum, and
   add `Lfx`. Two lists that must agree and are written twice is precisely how the next
   namespace gets half-integrated - and the failure is silent: the effect resolves to
   nothing, renders identity, and wears no badge, no line and no clue.

### 4.2 `LfxDef`

**Built**, with `LfxDef`. `crates/lumit-lfx/src/def.rs`, beside `lumit-ofx`'s, over the `LfxHost`
trait discovery left the per-render gate on - with `BrokerHost` (shipping) and whatever a test
hands it behind. The trait is documented as OFX's is, with the one honest difference the instance pool
left: **never called with a lock of the pool's rows held, and never from a rebuild path** -
but a call for a bundle that armed `lfx.thread-unsafe` is made with that bundle's `Serial`
held, since the lease takes it and holds it for the length of the frame (`pool.rs`,
`Pool::lease`). Saying "no lock at all" would tell an implementor it may block freely there,
when blocking stops every plugin of such a bundle. What makes telling an instance its values
and asking it for a frame indivisible is the **lease** rather than a lock of any kind, and
both traits' doc comments say so in the same words.

*Two seams rather than one, which is `LfxDef`'s own answer to a question §4.2 left open.*
`LfxHost` is the frame and the press, because those are the two calls the switched-off list
has to be read inside; opening the live instance is a third place a plugin's code runs, and a
definition that reached it *through* the gate would have to mint a handle before the gate
could refuse. So `LfxInstances` is the instance's own seam - open, update, close - and
`LfxDef::lease` reads the running list itself before it opens anything, answering the
same typed `BrokerError::SwitchedOff` the gate answers a press with and whose `Display` is the
one shared `DISABLED_REASON`. `a_switched_off_plugin_is_never_opened_at_all` is the half that
would otherwise be missing.

*Which leaves the gate a narrower job through `LfxDef` than §5.4's own sentence reads, and
it is worth saying rather than leaving to be discovered.* `LfxDef::lease` runs first on every
render, so a tick that has **already landed** is answered there and `Gated::process` is never
asked. What the gate still owns is the tick that lands *after* that read and before the frame
goes across - a person switching a plugin off while a frame is in flight - and that window is
a case of its own,
`a_plugin_switched_off_while_the_frame_is_in_flight_is_caught_by_the_gate`, driven through a
driver that ticks the box as it hands the instance back. Without it the gate is unreachable
from every case in the suite and `LfxDef::hosted` could drop it with nothing failing.
`a_switched_off_plugin_renders_identity_through_the_definition` proves the instance seam, not
the gate, and its doc comment now says so.

*`LocalHost` is not behind the trait, and that is a ceiling rather than a decision taken.*
The in-process host's `LocalInstance` borrows its host and takes `&mut self`, which is how
`two_instances_render_at_once_and_neither_is_re_entered` proves "one instance is never
re-entered" as a fact about the type; an `&self` trait cannot express it without either an
`Arc`-owning rework of `local.rs` or a self-referential holder, and neither is worth what it
buys. The definition's own cases drive both seams with a fake and the shipping path is
`BrokerHost`. *ponytail:* the day `local.rs` owns its host rather than borrowing it, the two
look the same from `def.rs` as this section wanted.

`LocalHost` itself is **built**, with the in-process host, as `lumit-lfx/src/local.rs` - the module, the
descriptor list, the sink, the instance and the frame handoff, with no trait over it yet
because there is nothing on the other side to abstract from until the broker lands (§10).
What the trait is for is that the two look the same from `def.rs`; what the in-process host proves is that
the in-process one works, which is the half a container can check.

`LocalInstance::process` takes `&mut self`, so "one instance is never re-entered" is a fact
about the type rather than a promise about its callers, and
`two_instances_render_at_once_and_neither_is_re_entered` holds the other half against the
fixture's own barrier.

| hook | what it does |
|---|---|
| `schema()` | the leaked `EffectSchema` from §2.3's lowering |
| `apply_cpu_temporal` | the fp32 process request |
| `apply_f16_temporal` | §4.5's fp16 path; `LfxDef` is the only implementor |
| `frames_needed` | `lfx.temporal`'s per-instance answer, offsets relative to `frame`, clamped to `LFX_MAX_TEMPORAL_WINDOW` |
| `last_error` | a thread-local written on **every** road out of a render and **taken** on read, so a stale reason cannot badge a later frame |
| `hidden_rows` | describe-time `hidden` flags, plus whatever `lfx.overlay` changes |
| `press` | an `ACTION` row |
| `resolve_derived` | `derived.frame` only |

**And `derived.` is the host's, which is a rule LFX can afford and the older host cannot.**
`resolve_derived` writes into the same resolved bag `values_of` reads on every road out, and
the built-ins push a dozen more values of their own under that prefix - so a plugin declaring
a control called `derived.frame` would draw a panel row whose value is replaced by the comp's
frame number before the plugin ever read it: no line, no refusal, and a control that does
nothing. LFX owns its own frozen vocabulary, so the prefix is reserved outright and a
declaration inside it is `LfxRejection::ReservedParamId` - refused in the sink, where the
plugin gets its `false` on the call that made it, and again at the lowering, for a descriptor
that never came through a sink (the argument `DuplicateParamId` already makes, one prefix
along). `DERIVED_PREFIX` is one constant that both the refusal and the push read, and
`an_lfx_effect_derives_the_frame_and_no_state_of_its_own` asserts the id the host writes is
inside it - or the rule would guard a name nothing uses.

`resolve_derived` pushes `derived.frame` and nothing else. The OFX bridge's pushes the same
frame by the same arithmetic - `(cx.lt * fps).round()`, which `LfxDef` copies - and
`derived.memory` beside it, a truncated hash of the instance's stored `plugin_state`
(`lumit-ofx/src/def.rs:620`). *So the contrast is not that LFX declines to smuggle a blob's
hash into a bag; the older host's hash is right there, and the first draft of this paragraph
said it was not. What is true is sharper and it is about staleness.* `plugin_state` is only
ever rewritten when the plugin is **pressed** - `Pressed::memory` through
`EffectInstance::set_plugin_state`, whose one production caller is the bridge's press
(`api/track.rs:584`) - so an OFX plugin whose opaque state
moved while it was *rendering* moves no hash, retires no cached frame, and can be served the
frame from before it moved. The hash makes a press visible to the key and nothing else does.
D8 does not avoid a smuggle; **it removes a hole the OFX host still has**, because a plugin
that keeps nothing has no state that can move unseen. That is the payoff worth naming to a
vendor who asks why, and the hole is recorded in §13 beside the ledger one.

**A failed process returns without writing `rgba` at all** - identity byte for byte. Not
the input written back, which would put the picture through the fp16 boundary and change it
very slightly, and "renders as identity" must not mean that.

The same sentence has a second reading one level down, and the broker has to obey it: **the
buffer a plugin is handed starts as the input, not as nought.** A plugin that answers
`LFX_STATUS_OK` and writes nothing has rendered identity - that is what the ABI means by it -
and an ROI is the output region asked for, full-frame being the degenerate case rather than the
assumption, so a plugin honouring a partial one leaves the margin untouched. A zero-filled
output buffer would send both back as a black frame that the host returns as `Ok`. Seeding the
output from the input is not "the input copied back": the copy is between two buffers of one
depth in one process, and the depth boundary the paragraph above is about is the one the *host*
would cross.

`LfxDef::leak()` is `Box::leak` and deliberately does **not** self-register.

### 4.3 Refusals that are not absences

§3.6 says a plugin requiring a missing extension "fails to instantiate with a clear message
and becomes a placeholder". Concretely: extensions are negotiated from the manifest, before
`create`. A plugin whose required list the host cannot satisfy never reaches instantiation,
never enters the catalogue, and is recorded in the session `REFUSED` table with the
extension named. **Built** with discovery: `lumit-lfx/src/extensions.rs` is the one list and the
one refusal, read by discovery from the listing before `create` - and before the module is
opened, where the bundle has nothing else in it; a bundle's module is still opened for the
plugins beside the refused one, whose own `describe` therefore runs, since the broker's
`Describe` filters on the switched-off list and not on this - and by the
in-process host at the moment it instantiates - two readers, because a host that answered the
same question two ways would catalogue an effect it then cannot make. The `badge_of` edits
below are **built**, with `LfxDef`.

**The sentence has to cross the pipe for that to be true.** The describe runs in the broker, so
every structural fault it meets there - a plugin that declines to describe itself, two rows on
one `ParamId`, a required extension version 1 has not got - is filed against that plugin's own
id and travels in `Described{refused}`. `LfxRejection` grew the three names it needed:
`DescribeRefused`, `RequiresExtension`, and `DescribeFailed` for the faults with no name of
their own, carrying the host's own sentence cut to `LFX_MAX_LOG_BYTES`. Dropping them in the
broker, as the first draft of the broker did, made a plugin that could not be described look exactly
like one the user had switched off.

**And the descriptor declares the same list**, which is why §2.1 adds `required_extensions`
and `required_extension_count` to the end of `lfx_descriptor`. Every other field the manifest
declares has a counterpart in the code the host can check it against; without this one, the
single field that decides whether a plugin is instantiated at all would have none, and trap
6's "the manifest is the cheap listing, never the authority" would be suspended exactly where
it matters most. A bundle declaring `required = []` in `lfx.toml` and in fact calling
`host->get_extension("lfx.temporal", 1)` would pass negotiation, reach `create`, get `NULL`
and fail at process time - the "left to fail somewhere later" outcome §2.1 says the refusal
exists to prevent. So negotiation still runs from the manifest, because keeping the module
shut is what §3.3 buys; the first `Describe` then re-checks against the descriptor, where a
list longer than the manifest declared is `LfxRejection::ManifestMismatch` like every other
disagreement. §9's describe suite and §14's test 5 both carry it. (The precedent is the OFX
describe pass, where the descriptor the host trusts is the one the plugin itself answered.)

`badge_of` then needs **three** edits, not two. It must read the `REFUSED` table **before**
it falls through to the namespace arm - otherwise an instance of such a plugin badges
`plugin_missing`, which is the wrong sentence - and the missing-plugin arm gains `Lfx` beside
`Ofx | Clap`. The third is trap 10 arriving by a door that section does not watch:
`badge_of` decides "switched off" by **string equality against one host's constant**,
`why == lumit_ofx::discover::DISABLED_REASON` (`api/effect.rs:1891`), reading the single
shared `error_of` table that every hosted def files into, audio included. An LFX `Gated`
filing its own `lumit_lfx::…::DISABLED_REASON` would badge `plugin_failed` - the same class
of wrong sentence, arriving by a different door. So `DISABLED_REASON` moves into `lumit-ipc`
beside `STRIKES_BEFORE_DISABLED` and `describe_deadline`, both hosts file the one string, and
`badge_of` tests a shared constant rather than one host's. `BADGE_REASONS` gains
`"plugin_refused"`, and `engine_labels_test.dart` reads the declarations, demands the
sentences, and holds `DISABLED_REASON` the way it already holds `BADGE_REASONS` -
`the_switched_off_word_is_one_shared_constant_with_a_sentence` scrapes the constant out of
`lumit-ipc`, asks the badge table for its sentence, and fails if `badge_of` ever goes back to
reading one host's re-export of it.

### 4.4 The instance pool, and the first written-down concurrency policy

**Built**, with the instance pool: `lumit-lfx/src/pool.rs` is the pool and `Policy` is this list as one
value, which is the thing to overturn. A frame **leases** a live instance for its own
length and hands it back, and that lease is what makes telling an instance its values and
asking it for a picture indivisible - a leased instance is not visible to any other frame, so
the per-instance lock `LfxDef` needed is gone rather than kept beside it. `Throughput` is the
measurement the third growth clause asks for, driven over instants a test chooses rather than
a clock it has to wait on.

§3.4 makes multi-frame rendering mandatory; docs/12 §3.4 and §2.3 both defer to an
adaptive-concurrency policy stated nowhere, and docs/13 §3 to §4 names no instance-pool rule.
This is the first one. **The numbers are provisional** and exist to be measured, not
defended: a limit whose number nobody can justify is a limit somebody will raise the first
time it fires.

- One live instance per in-flight frame, keyed by `(effect instance, frame)`, parameters
  applied per evaluation snapshot. Frames are dispatched **out of order** by design.
- The pool grows by one when it is saturated, `Ledger::pressure(Tier::Ram)` reads `Easy`,
  and measured per-frame throughput rose on the last growth.
- Capped at the render worker count and at `MAX_POOL_INSTANCES = 16`.
- `Pressure::should_trim()` - Severe or Full - collapses it to one.
- `lfx.thread-unsafe` pins it to one, bundle-wide, behind a single lock.
- Each pooled instance's slots are part of the ring's reservation, so growing the pool can
  be **refused by the ledger** rather than discovered by an allocator.
- A ROI whose declared `scratch_bytes_per_megapixel` exceeds what the ledger will grant is
  not dispatched (§8).

**Two of those read the ledger, and they read it differently**, which is worth spelling
because the bullets above read as one rule. The declared scratch is a real `Reservation`,
held for the length of the frame and given back by its own destructor, because it stands for
memory the plugin is about to allocate in another process. Growth is not a reservation at
all: the bytes an extra in-flight frame costs were bought once, by `Ring::create`, and
charging for them a second time would be double counting the same slots. So growth reads the
ring's **granted** slot count - what the ledger was willing to pay for - divided by what one
frame of *that* plugin ships, and a ring the ledger narrowed to `RING_MIN_SLOTS` pins the pool
to one with it. That is "refused by the ledger rather than discovered by an allocator" with
the refusal where the money was actually spent.

**A frame's slots are its window, not two.** `Broker::process` holds a shipment to
`neighbours + 2` slots and `slots_for` sizes the whole ring as `hi − lo + 2` - every frame the
declaration says it reads, plus the one being written - so a plugin declaring ±5 gets a
twelve-slot ring that carries exactly **one** frame. A pool dividing by two there would read
it as room for six and have the sixth shipment refused with the plugin's name on it, wrongly
by the width of the window. And the count is read **as it stands** rather than copied when the
definition is built: `Broker::fit` replaces the ring under the first frame bigger than a slot
and a bigger frame buys fewer slots, so `Broker::granted_slots` publishes the number and
`Policy` holds the handle rather than the integer. A stale copy would always be stale in the
permissive direction.

**The frame's own reservation is taken after the queue rather than before it.** A frame
waiting - behind the bundle's lock, or at its row's ceiling - is a frame allocating nothing,
and one holding its declared scratch while it waited would raise the very `Ledger::pressure`
the growth clause and the collapse both read: the queue would collapse its own ceiling and
then badge a later frame with the plugin's name for what was congestion. What is still
answered *before* the wait is the question waiting cannot change - whether the tier's whole
budget covers this frame at all - so §8's refusal still costs no lease and no instance.

**A row that has left the layer is the pool's to answer too**, and the instance pool answers it:
`MAX_POOL_ROWS = 64` rows of up to sixteen instances each is `MAX_LIVE_INSTANCES` **for a
bundle of one plugin**. Past that, the least recently leased rows - and only rows with
**nothing leased**, so eviction never races a frame - have their instances closed. A frame
that comes back to an evicted row opens one again, which costs a round trip and no picture:
the row itself stays, because the offsets its last render asked for are what the frame key is
computed over, and a row that lost them would come back keyed, prefetched and rendered against
the *declared* window - a different picture, and a different one depending on which rows the
eviction happened to reach. `MAX_POOL_MEMOS = 1_024` is what bounds the memos themselves, on a
larger count and with nothing but rows holding no instance to choose from. Without any of it,
a session of add-and-delete churn walked towards the broker's ceiling and every new row of
that plugin badged rather than rendered until Lumit was restarted, since the shipping
definition is leaked and its `Drop` never runs.

*ponytail:* the 1,024 is the **broker's**, so it belongs to the bundle, while a pool belongs
to one `LfxDef` - that is, to one plugin of it. A bundle of eight much-used effects can
therefore still reach eight times the product and be refused at the broker, and scaling
`MAX_POOL_ROWS` by the described plugin count only moves the arithmetic, since a bundle may
describe `LFX_MAX_EFFECTS_PER_BUNDLE` of them and the first instance of a row can never be
refused. Closing it wants one pool per **broker**, keyed by plugin id and row - which is also
where the bundle's `Serial` belongs. What is true today is that one plugin's churn is bounded
and a bundle's is bounded by the number of plugins in it.

**And the bundle's lock is armed by the bundle.** `lfx.thread-unsafe` "is serialised
bundle-wide" in the header's own words, so `Serial::for_bundle` reads the whole described list
once and every definition built over that broker is handed the same value; `Policy::declared`
asks the lock whether the bundle armed it rather than asking this plugin's trait block. The
mixed bundle - one plugin declaring, the next not - is the only shape that tells bundle-wide
from per-plugin, and it is the shape the promise is about: the plugin that declared nothing
shares one process, one broker and one ring with the plugin that did.

*ponytail:* the OFX broker carries one conversation at a time - "the parallelism is across
brokers until one broker can carry two conversations at once". LFX's proto carries a
request id from message one, so concurrency inside one broker is a thread pool on the child
side rather than a protocol change later. **The same ceiling is `BrokerHost`'s here**, and
the pool sits above it: a bundle's broker is one lock, so two leased instances of one plugin
are two frames taking that lock in turn rather than two conversations. What the pool buys
today is the *correctness* half - an instance is never re-entered, and each frame is painted
with its own numbers - and it is already the shape the parallelism will arrive into.

### 4.5 The pixel handoff, and the fp16 promise made real

Registration goes through the existing composition-root pairing: the `GpuEffect` pass is
leaked into `GPU_EFFECTS` **first**, then the definition into the catalogue, in one call,
so no catalogue entry ever exists whose pass has not arrived. The two tables are joined
only by a `match_name` string and nothing checks the join at compile time, which is why the
ordering is the rule.

The existing `CpuPass` reads the working texture back as linear fp32 and re-uploads to
fp16. For an **fp16 project** that is a conversion at exactly the seam §3.3 promised not to
convert at. The fix is one new defaulted `EffectDef` hook and one new pass. **The hook and
the readback are built (the fp16 seam); the pass is the render pass's**, because it is the only part of the
three that needs a card to prove.

```rust
/// The fp16 twin of `apply_cpu_temporal`. `false` means "I do not do fp16 -
/// use the f32 path", which is every built-in, so nothing moves.
fn apply_f16_temporal(&self, inst: Uuid, lt: f64, rgba: &mut [half::f16],
                      w: u32, h: u32, p: Params<'_>,
                      _neighbours: &[(i32, &[half::f16])]) -> bool { false }
```

`half = "2"` is already a `lumit-core` dependency, so this costs no new crate.
`lumit_gpu::fx::readback_linear_f16` lands in `lumit-gpu/src/fx/common.rs` -
**including the `ctx.flush()`** that fixed hosted plugins being handed empty frames, which
is the one line nobody may leave out. `gpufx::hosted::Fp16Pass` tries the hook and falls
back; a failed readback is a passthrough with a note, never a fault.

The hook is on `EffectDef` with the signature above, and
its doc comment says the part the signature cannot: a definition answering `false` must
leave `rgba` as it found it, because the caller will *widen those same halves* for the f32
path, and a half it wrote on the way past is a pixel the f32 effect never saw.
`every_builtin_declines_the_fp16_path_and_leaves_the_picture_alone` walks the whole built-in
catalogue and holds both halves of that, so "nothing moves" is a swept fact rather than a
sampled one; `a_definition_that_does_fp16_is_handed_the_halves_and_its_neighbours` is the
other side - a definition registered under an `lfx:` name, fetched back out of the catalogue
and driven as the `&dyn EffectDef` the render path will drive, reading a neighbour whose
first half is a **signalling NaN**.

That NaN is the only bit pattern either test can lean on. f32 holds every finite half
exactly, and an fp16 subnormal widens into an ordinary f32 normal, so a zero, a subnormal and
the largest half all come back from a widening unchanged and say nothing about which path was
taken; a widening *quiets* `0x7C01` into `0x7E01`, so a hook implemented as "widen, convert,
narrow" fails and a hook handed the halves passes. Both tests compare bits rather than
values, because `half::f16` compares as a float - it would call two NaNs unequal and two
zeroes of opposite sign the same.

`readback_linear_f16` is in `lumit-gpu/src/fx/common.rs` with `readback_linear_f32`, whose
body it shares, and returns the halves in exactly the layout `upload_linear_f16` takes back.
The one line nobody may leave out is not repeated: the two read-backs **share a body** -
`read_back_rows` flushes, copies into the padded staging buffer, waits, and hands each row
on with its padding dropped - and `readback_linear_f32` is now that body widening each row,
which is what it always was. A shared body cannot omit the flush, which is a stronger answer
to trap 3 than remembering to write it twice;
`halves_read_back_inside_a_batch_wait_for_what_is_still_batched`
is the twin of the OFX-era test that found the empty frames, and fails with every half
nought if the flush goes. `halves_read_back_are_the_bits_that_were_uploaded` is the round
trip at a width of 33, where the row is 264 bytes and the staging buffer pads it, and it
asks the f32 twin the same question of the same texture so the two cannot come to read
different pictures out of one.

The shared body also **refuses what it cannot read**, which the prose above assumed rather
than stated. A working texture is only sometimes an fp16 one:
`GpuContext::working` follows the project's colour depth, so D7's own 8 bpc case makes it
`Rgba8Unorm` and a 32 bpc project makes it `Rgba32Float`, and `new_work_texture` makes every
working texture in whichever it is. Eight bits read as fp16 is a copy wgpu *allows* - the
real row is half as long, so half of what comes back is the staging buffer's own padding
returned as picture, with no error anywhere. Thirty-two bits read as fp16 is a copy wgpu
refuses, and an uncaptured device error ends the process, which is a panic reached from
library code. So `read_back_rows` checks `tex.format()` before it copies and answers
`GpuError::Readback` otherwise - a typed refusal both callers already propagate and
`Fp16Pass` turns into the passthrough with a note that docs/14 §4 asks for.
`a_read_back_refuses_a_texture_that_is_not_the_working_depth` holds it at both depths and for
both read-backs, and a read-back of no area at all is the empty `Vec` its caller already
sized rather than a zero-sized buffer and a zero-long chunk
(`a_read_back_of_no_area_is_empty_rather_than_a_fault`). `SharedGpu::reset` puts the working
depth back to sixteen between tests for the same reason it puts the budgets back: a leased
context that carried a depth would make every read-back after it a refusal.

*ponytail:* what an 8 bpc project then *does* is the render pass's, and the f32 fallback is no help,
because both read-backs refuse the same texture. D7 promises a plugin fp16, so the pass has
to draw the working texture into an fp16 one before it reads and put the answer back the
same way. Until it does, a hosted CPU effect in an 8 or 32 bpc project is a passthrough with
a note rather than a wrong picture or an abort - which is the refusal doing its job: it
makes the gap visible instead of plausible.

The third piece is the pass, and the paragraph above the code block says why it is the render pass's:
it is in `lumit-render`, it needs a card to mean anything, and the aux choice §4.6 argues
over sits beside it.

### 4.6 The aux and temporal seam

`CpuPass` declares `AuxKind::Neighbours` unconditionally, and `op_keys` sets `broken = true`
on `Neighbours | FlowField` - so the per-effect intermediate cache chain stops at that op
and at every op after it in the stack. An LFX plugin that declared a zero temporal window
needs no neighbours and should not pay that. **The pass shape is chosen from the declared
traits** - and the direction of the seam
matters, because the obvious reading of that sentence is backwards. The existing seam
hands `lumit-render` a *definition*, and
`gpufx::ofx::register` constructs `CpuPass(def)` itself: a private struct whose `aux()`
answers `AuxKind::Neighbours` unconditionally. `lumit-lfx` cannot build the pass and hand it
over. `GpuEffect::run` takes `&FxEngine`, `&GpuContext` and an `AuxSlot<'_>` whose every
field is private, and neither existing plugin-host crate depends on `lumit-gpu` or
`lumit-render` - which the module doc at `gpufx/ofx.rs:15-19` says is the whole point. Two
shapes work: `gpufx::hosted::register(def, aux: AuxKind)`, the choice worked out by the
caller and passed as a plain enum; or - cheaper, and no signature changed at all - `aux(&self)`
already holds the definition, so the pass answers
`if self.0.schema().traits.temporal.iter().any(|&o| o != 0) { Neighbours } else { None }` for
itself. Either way `lumit-render` learns nothing about what an LFX plugin is, and either way
**the aux choice is the render pass's**, on the desktop, not `LfxDef`'s.

A plugin *with* a temporal window has its schema `traits.temporal` widened at describe to at
least `[-1, 0, 1]`, so `stack_is_temporal`'s declaration gate opens; the per-instance
refinement comes from `frames_needed` through `stack_temporal_window`, which reaches the
frame key and the neighbour decode.

**The trap:** declaring `AuxKind::None` is a promise. A plugin that later starts sampling
neighbours without redeclaring gets frames that were never decoded. `lfx-validator`'s
temporal check - declared window against what `lfx.temporal` actually asks for - is the only
defence, and it tests the plugin as shipped, not as later modified.

### 4.7 Serialisation and the placeholder round-trip

Free, once `Lfx` is in the guards. `EffectKey { namespace: Lfx, match_name, version }` is
plain serde; unknown fields ride in the `#[serde(flatten)]` `extra` maps on the key, the
param and the instance; a machine without the plugin misses in `fx::def`, resolves to no op,
renders identity with every parameter and keyframe intact, and the panel badges it. Nothing
in the document is lost and nothing is rewritten - `backfill_builtin_params` and
`migrate_percent_to_px` both `continue` on any namespace that is not `Builtin`.

---

## 5. Discovery, the scan's state, and enable/disable

### 5.1 The bundle

**Built.** `lumit-lfx/src/bundle.rs`. `Name.lfx.bundle/Contents/` holding `lfx.toml` and one
architecture directory per build:

| target | tried, in order |
|---|---|
| Windows x86-64 | `win-x86_64` |
| Windows arm64 | `win-arm64`, `win-x86_64` |
| macOS x86-64 | `macos-universal`, `macos-x86_64` |
| macOS arm64 | `macos-universal`, `macos-arm64`, `macos-x86_64` |
| Linux x86-64 | `linux-x86_64` |
| Linux aarch64 | `linux-aarch64` |

An **ordered per-target list**, not OFX's single `BUNDLE_ARCH_DIR` string, which has no
arm64 variant for Windows or Linux at all. A **target** column rather than a platform one,
because the operating system alone does not say which binaries a process can load: a list
chosen by `target_os` would name `macos-arm64` before `macos-x86_64` on an Intel Mac and hand
the broker the wrong dylib out of the two-per-CPU bundle this list exists to serve. Each
target names its own build first and a foreign CPU appears only where the platform emulates
one - `macos-x86_64` last under Rosetta, `win-x86_64` last on Windows arm64. **The target is
the operating system as well as the CPU**: the six rows name Windows, macOS and Linux by name,
and a target outside them - FreeBSD, illumos, Android - gets the empty list and the sorted
fallback, because LFX has no spelling for it yet. A list reached by "not Windows and not
macOS" would hand a FreeBSD box `linux-x86_64` and spawn a broker on an ELF built for another
operating system, which is the same mistake on the other axis.

The payload is the first `.lfx` in `<arch>/`, by name. An installer should write
`<arch>/<Name>.lfx` and the layout above is that; what `bundle::payload` reads is the
extension and not the stem, which is `lumit_aplug::vst3::payload`'s rule and is what makes a
bundle renamed on disk go on loading.

Follow `lumit-aplug/src/vst3.rs:125-139` for the shape and its `ponytail:` marker, with one
deliberate departure: that host knows only its own platform's spellings of somebody else's
standard, so a folder it cannot name might be one it should have taken. **LFX owns all seven
names** (`ALL_ARCH_DIRS`), so the sorted fallback passes over the six that are not this
build's. A `win-x86_64` on a Linux machine is a *known foreign* build, and admitting it would
spawn a broker on a PE file and report "the module would not load" where the truth - and
§7.3's Installed row - is that this bundle carries no build for this machine. The fallback is
left for genuinely unnamed folders, which is the case the `ponytail:` marker is about.

The walk is recursive to four levels, never descends into a bundle looking for another, and
is **sorted** - two runs discover in the same order, which is what makes an effect list
stable between sessions.

### 5.2 Search paths

**Built.** `lumit-lfx/src/bundle.rs`: per platform, plus `LFX_PLUGIN_PATH` split with
`std::env::split_paths` and **appended, never replacing** (the rule a test pins for both
existing hosts), plus `lumit_ipc::addons_dir()`.

The addons-folder append is pinned **once per host, by name**, and not by the environment
variable's own test: LFX's
`the_search_paths_are_the_standard_ones_plus_the_variable_and_the_addons_folder`, and
`the_addons_folder_is_one_of_the_folders_a_scan_looks_in` in each of `lumit-ofx/src/tests.rs`
and `lumit-aplug/src/tests.rs`. The older hosts' existing search-path tests read only the
*first* entry and the *length*, so both of them pass with the line deleted - and deleting it
takes plugin discovery out of every sandboxed Lumit, which is the next paragraph.

*The directory is `lumit-ipc`'s rather than `lumit-project`'s, which is where D10 and §6.1
put it, and discovery moved it for one reason the note did not notice: it has to be appended
**inside each host's own `search_paths()`**, and none of the three hosts depends on the
project format - each reads the switched-off list it is handed rather than reading the
preference file, deliberately. `lumit-ipc` is already the crate all three share for the
handful of answers every host must give the same way, which is exactly what "where a plugin
Lumit installed itself lives" is.*

Failure 4 is the reason the last one matters more than it looks: inside a Flatpak, `/usr` is
the GNOME runtime's own, so a system-installed plugin is invisible - the same trap that
already hides `/usr/OFX/Plugins`. The Lumit-owned directory is not a convenience on Linux;
it is the only route. §7's read-only search-path rows exist so a person can see that.

### 5.3 Three tables, and why

**Built.** `lumit-lfx/src/discover.rs` holds the two session tables and
`lumit-project/src/roster.rs` the third. Three notes on what discovery settled.

*The refusal is the `LfxRejection` itself, not a sentence.* `REFUSED` carries the typed
refusal beside the row, because the page prints it, `lfx-validator` asks for one by name, and
a badge that matched on prose is the coupling §4.3 already records as a trap.

*One row shape, in three states.* `DISCOVERED`, `REFUSED` and the listing all carry an
`AddonRow` - identifier, label, vendor, release and bundle - rather than three structs with
the same five fields, which would be three places for the page to disagree with itself.

*The key is the kind and the identifier.* `PluginRoster` files under `<kind>:<identifier>`
rather than the identifier alone: two standards may legitimately use the same reverse-DNS name
for one vendor's two builds of one effect, and a roster that folded them together would show
one row that switched two plugins off. §7.2's `set_addon_enabled` therefore needs the kind
beside the identifier to find the row.

*The scan hands the roster its rows; it does not write the file.* `ScanOutcome::listed` is
every plugin every bundle's listing declared, whatever became of it - the listing is read
**before** the payload for this machine is looked for, so a bundle whose only build is for
another CPU contributes its rows and then its one skip sentence, and §7.2's `missing` state
has a label and a vendor to draw. Writing that into `addons.json` is the composition root's -
a plugin host that read the preference area would be a plugin host that depends on the
project format, which is the same edge §5.2 keeps.
`PluginKind` lives with the roster and its four spellings are §7.2's `ADDON_KINDS`.

| table | lives | holds |
|---|---|---|
| `DISCOVERED` | session | what registered: identifier, label, vendor, version, categories, bundle |
| `REFUSED` | session | what `offer()` turned away this session, with its own sentence |
| `PluginRoster` | `data_dir()/addons.json` | **everything ever seen**, by kind *and* identifier, with `last_seen` and `last_refusal` |

The roster is the answer to failure 6, and what it buys wants stating precisely, because the
scan already **mentions** a plugin switched off before it: `offer()` pushes a skip line
carrying the identifier and the sentence "switched off in preferences"
(`lumit-ofx/src/discover.rs:403-409`) - and in the LFX scan that line is pushed from the
**listing** instead, because the disable travels with `Describe` and a switched-off plugin
does not normally come back from the second process for `offer` to reach at all, which is the
first of §5.4's three places working. `offer` keeps the same check as a backstop, for the one
tick that lands *after* a plugin has been described and before it is offered; it is the same
sentence, and the listing is where it comes from in every ordinary scan. That line crosses
the bridge in `BridgePluginScan.skipped` - the same report §7.3's Scan section shows,
produced at every launch. What such a plugin has no place in is a **structured row**: label,
vendor, version, kind and state, which is what the Installed section is made of and what a
skip line cannot be parsed into. Nor is there a row for a plugin whose folder has gone, or for
one this session has not scanned at all. So the roster's job is the Installed section's rows,
not the existence of a mention, and "three-quarters of the page would be blank" was overstated -
`DISCOVERED` is refilled by the start-up scan every launch. Every scan writes what it found
*and* what it refused, for all four kinds, and the page opens with rows rather than waiting
on a rescan.

The roster lives beside `plugins.json` in `data_dir()` and is read under an ingress budget:
it is a file of third-party-derived strings. An absent or damaged file reads as **nothing
seen** - a page that shows nothing until the first rescan, never an error.

**The writer is held to the reader's ceiling**, by forgetting the rows seen longest ago until
the file fits. `LFX_MAX_EFFECTS_PER_BUNDLE` is 1024 and `LFX_MAX_STRING_BYTES` is 1024 under
an 8 MiB `Limits::PLUGIN_MANIFEST`, so one bundle's honest listing is several megabytes of
roster all by itself - past `Limits::PLUGIN_ROSTER`'s 2 MiB, which the reader answers by
reading nothing at all. A `save` with no ceiling would let one stranger's manifest put
`addons.json` permanently past its own reader and blank the Installed section for every plugin
of every kind on the machine, silently: failure 6 reopened by the file that exists to close it.
Forgetting the oldest is the right loss - a roster is a record of what was noticed, a plugin
still on the machine gets its row back at the next scan, and one whose folder has gone was
never going to. Roaming to
another machine lists plugins not installed there, which becomes `missing` on the next scan
and is true.

### 5.4 Enable and disable, in three places

The persisted list is unchanged: `lumit_project::PluginPrefs { disabled: BTreeSet<String> }`
at `<data_dir>/plugins.json`, **identifiers only**, one file for four plugin kinds.
`set_plugin_enabled` gains an `lfx:` arm. **`PluginPrefs` gains no `path_override`**, which
is the obvious way to give the Addons-page tests somewhere to write and the wrong one: the
struct derives `Serialize` as well as `Deserialize` and is written back whole with
`to_string_pretty(self)`, so a plain
`#[serde(default)]` field round-trips. Every save would write `"path_override"` into the
user's real `plugins.json` and every load would read it back - a preferences file that
decides where preferences are written, read out of itself, carrying a key that means nothing
on any other machine given the file roams (§6.1). The test seam already exists and is
path-taking rather than field-taking: `PluginPrefs::load(&Path)` and `save(&Path)`, used that
way by the tests today. The bridge's addon calls take the prefs path - or read it from a
`LUMIT_PLUGIN_PREFS` env seam - and pass it in, which is what keeps
`lumit-bridge/src/api/tests.rs:12980` from writing the developer's real file. New fields,
when there are any, are added with `#[serde(default)]`, which the struct already carries, so
an older file reads unchanged.

Disable reaches three places, and the third is new. **Two of them are built** - the list
travels with `Describe`, and `discover::Gated` is the per-render read - and the third is the
bridge's, in the bridge surface:

1. **Before describe.** The list travels with `HostMessage::Describe`, so a switched-off
   plugin is never described and none of its own code runs - and **a bundle with nothing left
   to describe is not asked for a describe at all**, so its module is never opened and its
   `init` never runs. That second half is what makes the property hold for the commonest
   bundle shape, one plugin in one module, which is also the one a person switches off
   precisely because it misbehaves: `scan_bundle` counts the survivors of the listing and
   returns before `broker.describe()` when there are none, the rows and the skip lines having
   already been filed. A module shared with plugins that are on is still opened for them,
   which is the honest ceiling and the reason this is two sentences rather than one.

   The scan hands the broker a **share** of the running table
   rather than a snapshot of it (`discover::running_list` is an `Arc::clone`, and `DISABLED`
   is itself the `DisableList`), which is what makes the read late as well as early: a tick
   landing between the spawn, the handshake and the `Manifest` round trip still reaches the
   describe that has not happened yet, pinned through a real second process by
   `a_plugin_switched_off_after_the_broker_starts_is_still_switched_off_at_describe` in
   `lumit-lfx-broker`'s own suite, with the handle under it pinned by
   `the_disable_list_a_broker_is_spawned_with_is_a_share_of_the_running_table`.

   **There is one list**, and `ScanOptions` carries no second copy of it: a scan reads the
   running table and never writes to it, and the caller seeds that table from `PluginPrefs`
   with `set_disabled`, which replaces it whole. A field on the options merged into the
   running list could never take an identifier back out of it - a scan carrying a narrower
   preference would leave last scan's plugins switched off for the rest of the session - and
   a field that replaced the list would wipe the preference every time start-up asked for the
   standard paths. That is where LFX parts company with the two older hosts'
   `ScanOptions`, and `a_scan_reads_the_running_list_and_never_writes_to_it` holds it.
   (*Note: OFX does not do this today -
   `scan_through_broker` describes the whole bundle before `offer()` consults the list, and
   the broker opens and loads the bundle straight after the handshake. The doc comments
   claim the stronger property; the code does not deliver it. LFX delivers it; correcting
   OFX is §13.*)
2. **Per render.** **Built.** The `Gated` wrapper reads the running list inside every process call and
   files **`lumit-ipc`'s shared `DISABLED_REASON`** - the one string `badge_of` tests
   against, never a per-host twin of it (§4.3) - so a plugin switched off mid-session stops
   rendering **now**
   rather than at the next launch. That is failure 5: inside an open project the instance
   keeps resolving, renders identity, and the panel badges `plugin_disabled` - the
   placeholder that says why.
3. **The catalogue listing.** `catalogue()` in the bridge - the one walk both listings share -
   skips an entry whose identifier is in any host's running disabled list, so a
   switched-off plugin **leaves Effects & presets**. Today its row stays, unmarked, still
   offering only "switch off".

Filtering the listing rather than unregistering is deliberate and load-bearing:
`Catalogue::register` is additive and never removes, which is what makes a rescan
idempotent - and it means a re-enable is **instant and needs no rescan**.

---

## 6. Installation

### 6.1 Where

**Built.** `lumit-ipc/src/addons.rs`. `lumit_ipc::addons_dir()` = `data_local_dir()/addons`
(§5.2 says why it is not
`lumit-project`'s), created on demand as
`presets_dir()` is, and appended **inside each host's own `search_paths()`** - not onto
`ScanOptions.paths`. The audio host's shipping scan does not go through `ScanOptions` at all:
the bridge calls `scan_brokered(&lumit_aplug::search_paths(), …)` with a bare slice, and
`ScanOptions` reaches only `lumit_aplug::scan` and the aplug tests. Appending there would
reach the OFX scan and the audio *tests*, and leave CLAP and VST3 never searching the addons
directory in a running Lumit. Appending in `lumit-ofx/src/bundle.rs`'s `search_paths` and
`lumit-aplug/src/discover.rs:146` instead picks up both routes, and is pinned by the test
that already exists for the `LFX_PLUGIN_PATH` shape - appended, never replacing (§5.2).

**`data_local_dir()`, not `data_dir()`, and this is the trap.** Every other Lumit path uses
`data_dir()`, which on Windows is roaming `%APPDATA%\Lumit\Lumit\data`. Presets are a few
kilobytes of JSON and roam deliberately. A folder of **native plugin binaries** would roam
too - between machines, and across architectures - and the frame-cache comment already
warns that a roaming profile "would try to copy the cache to a network share at logoff".
`data_local_dir()` has no caller in the tree today; this is the first, and the difference is
Windows-only (on macOS and Linux the two resolve to the same place, and inside a Flatpak
both redirect into `~/.var/app/…`, which is writable - §5.2).

Install is LFX-only. Lumit will not write into `C:\Program Files\Common Files\OFX\Plugins`,
and on Linux `/usr/lib/lfx` is the runtime's.

### 6.2 What "install" does

**Built.** `lumit-lfx/src/install.rs` and `lumit-lfx/src/trust.rs`, with the corrections
§12's install and trust paragraph records: the manifest declares a digest per archive entry, so the
signature reaches the payload rather than only the file naming it; the public key rides in the
signature file, because there is no out of band for it to arrive by; the fingerprint is
compared here and written down after the smoke test and before the rename rather than at step
2; a pack carrying **no** signature is compared against the store too; and the trust store
files the bundle directory an install owns beside the key, because the name an install
overwrites is not the identifier a key is filed under.

A `.lfxpack` is a zip carrying `manifest.json`, a detached Ed25519 `manifest.json.sig`, and
the bundle. In order:

1. **Signature before parse - in Rust, and it costs a dependency.** Both files read under a
   byte cap; the signature is checked **before** the JSON is parsed, and a failure is a
   refusal, never a fallback to the weaker check. That is release-signing.md's rule, and its
   reason - "falling back would make the whole mechanism decorative" - applies unchanged.
   What does not carry over is the verifier: there is **no Ed25519 in the Rust tree**.
   `Cargo.lock` has no `ed25519`, no `ring` and no `signature`; the project's only signature
   check is `package:cryptography`'s, in Dart. So this step names its cost as the note names
   `half` and `cc`: `ed25519-dalek` for the verify and `sha2` for the digest - BSD-3-Clause
   and MIT/Apache-2.0 respectively rather than both of the latter, as this paragraph first
   had it, and every crate the two bring with them is one of the three - and so through
   `deny.toml`'s licence gate, with the deliberate-decision
   line in the install and trust pull request that §9 of that file asks for. Verifying in Dart and unpacking
   in Rust cannot satisfy "before the JSON is parsed" across a process boundary - a caller
   could simply skip the Dart step - so the Rust side would have to re-verify anyway, which
   puts the verifier back in Rust. It goes there first.
2. **Trust on first use, in a file of its own.** The key fingerprint is shown and recorded
   as id → fingerprint in `<data_dir>/addon-trust.json` - **not** in `PluginPrefs`.
   `PluginPrefs` is deliberately fail-open: an absent *or damaged* file reads as "nothing has
   been switched off", the parse error swallowed, with
   `a_missing_or_damaged_file_switches_nothing_off` pinning it. That is the right policy for
   a list of switched-off plugins, where losing it costs a re-tick, and the wrong one for a
   trust store, where losing it costs the whole mechanism: a `plugins.json` truncated,
   hand-edited or simply deleted would silently forget every fingerprint, and the very next
   pack, under any key, would install as a first use and never be refused. A defence whose
   value is "stops a silent swap" may not be defeated by deleting one JSON file the
   attacker's own installer can reach. So the trust store distinguishes **three** cases where
   the preferences file distinguishes two: **absent** is a first use, **parsed** is compared,
   and **unreadable or damaged** is a refusal by name - `trust_store_unreadable` - rather
   than a default. A later pack for the same id under a **different** key is refused as
   `signature_changed`, and a later pack for the same id under **no key at all** as
   `signature_missing`: a defence a signed pack cannot get past may not be walked around by
   deleting `manifest.json.sig` out of the zip, which needs no key rather than the wrong one.
   An unsigned pack installs where nothing is known about the addon, with a calm line and no
   elevated capability, per docs/12:653-656; it does not install *over* an addon whose key is
   written down. The key is filed under `lfx:<id>` and not under the bare identifier, for the
   reason §5.3 gives for the roster: `addon-trust.json` is named for addons generally, and a
   second host's packs sharing it must not collide with this one's.

   The store also writes down **which bundle directory each addon's install owns**, because
   "the same addon" is two questions and the key answers only one of them. A pack chooses the
   identifier it is filed under *and* the directory name it lands in, and nothing relates the
   two: without the second row, a pack under a fresh id and a fresh key would be an ordinary
   first use that happens to overwrite a plugin the person installed from somebody else. A
   bundle another addon's install put there is refused as `bundle_claimed`, whatever key the
   pack carries and including none.
3. **Bounded unpack**, through `lumit-ingress` with `Limits::ADDON`: the two-sided check
   from `lumit-project`'s `entry_text` - the archive's own declared uncompressed size *and*
   a read one byte past the ceiling, so a file that lies about its length is refused rather
   than believed. The entry is *streamed* - decompressor to digest to staged file, a block at
   a time - rather than held, because the archive's own budget is a gigabyte and a single
   entry claiming all of it would otherwise cost this process a gigabyte of resident memory
   for about a megabyte of deflated zeros on disk. There is a second and smaller per-entry
   ceiling under it, so one file may not claim the whole archive's budget. The budget's
   `depth` is what bounds an entry's path components and its `work` the bytes of name swept,
   charged entry by entry rather than restated as constants beside it.
4. **Every entry name is a bare relative path.** No separator, no `..`, no drive letter, no
   symlink entries. The rule `isPlainFileName` encodes in Dart, moved into Rust where the
   unpack is.
5. **Layout check**: exactly one `*.lfx.bundle` at the root, a readable `Contents/lfx.toml`,
   and a payload for this machine's architecture.
6. **Staged outside the search path, then renamed.** Unpack into
   `data_local_dir()/addons.staging/<token>` - same volume, so landing is still one rename
   into `addons/` - and **not** into `addons/.staging/`, which is inside the directory §5.2
   and §6.1 push onto every host's search paths. The walk descends into every directory it
   finds, dot-prefixed ones included (`collect_bundles` filters on the bundle suffix and the
   depth, and on nothing else), and the start-up scan fires on every launch. So a scan racing
   an install would walk `addons/.staging/<token>/Name.lfx.bundle` and discover a half-written
   bundle; an install killed between steps 3 and 6, or a crash before step 7's cleanup, would
   leave one discoverable for ever. **The rename alone does not deliver "never looks
   installed"** - keeping the staged tree out of the searched directory does.
   *ponytail:* if `.staging` is kept there instead, the LFX walk must skip any entry whose
   name begins with `.` and a test must pin it ("a bundle under `.staging` is not
   discovered"); `install_site.dart`'s own discipline is marker-plus-rename, not rename
   alone, and a walk that trusted the marker would have to require it.
7. **Manifested in a broker before the install is confirmed.** Out of process, under the
   peer handshake, at the ten-second ceiling. A bundle whose manifest cannot be read is
   refused and the staging folder removed.

Step 7 is a smoke test, not verification, and the note says so where the page will: the
verification is step 1, and step 1 is only as strong as TOFU, which stops a silent swap and
stops nothing on the first install. Nothing here may be described as "verified".

**Copy, never record-a-path.** One place to verify and one place a later swap would have to
happen; TOFU over a path the user controls elsewhere is TOFU over a moving target.

### 6.3 Per platform

| | |
|---|---|
| **Windows** | Per-user, no administrator. `{app}` is the installer's and may be replaced by the next update, so nothing user-installed goes there. Job Object memory limit on the broker (§8). |
| **macOS** | The `.app` is signed and may not be modified, so the addons directory is the only writable route. The broker is codesigned separately with `Broker.entitlements` - library validation off there and **never** on the app - and the existing `lumit-*-broker` glob in the codesign loop covers a third broker for free. A copied bundle carries the quarantine bit; that is a sentence on the page, not a mechanism. |
| **Linux** | Flatpak is the whole release, `--filesystem=home` reaches the addons directory under `~/.var/app/…`, and the standard paths do not reach out of the sandbox at all (§5.2). |

The `cargo build --release -p …` and `cp` lines in `make-dmg.sh`, and the
`add_custom_target` / `install(PROGRAMS)` pairs in both CMakeLists, **name each broker
explicitly** and must gain `lumit-lfx-broker` by hand; the Inno recursive copy and the
Flatpak bundle copy then pick it up for free. Miss this and `broker_exe()` -
`current_exe().parent().join(name)` - finds nothing in any shipped build.

---

## 7. The Addons page

### 7.1 What has to change first

Three things are wrong today and the page cannot be honest without all three fixed:
there is **no way to switch a plugin back on** (`setPluginEnabled` has one Dart call site
and it hard-codes `enabled: false`); a switched-off plugin **still appears** in Effects &
presets, unmarked, offering only "switch off" again; and a plugin switched off before the
scan has **no row of its own** - the scan names it in a skip line, which is a sentence in a
report rather than a row with a label, a vendor, a version and a switch (§5.3). §5.3 and
§5.4 are the engine answers; this section is the page.

### 7.2 Bridge surface

In `crates/lumit-bridge/src/api/effect.rs`, beside `rescan_plugins`:

```rust
#[frb(non_opaque)]
pub struct BridgeAddon {
    pub identifier: String, pub label: String, pub vendor: String,
    pub version: String, pub kind: String,      // ADDON_KINDS
    pub state: String,                          // ADDON_STATES
    pub detail: Option<String>,                 // the scan's own sentence
    pub location: String,
}
pub const ADDON_KINDS:  &[&str] = &["lfx", "ofx", "clap", "vst3"];
pub const ADDON_STATES: &[&str] = &["registered", "disabled", "failed", "refused", "missing"];

#[frb(sync)] pub fn list_addons() -> Vec<BridgeAddon>;    // roster ∪ DISCOVERED ∪ REFUSED
#[frb(sync)] pub fn set_addon_enabled(identifier: String, enabled: bool) -> Result<(), BridgeError>;
#[frb(sync)] pub fn addons_dir_path() -> Option<String>;         // creates on first ask
#[frb(sync)] pub fn addon_search_paths() -> Vec<String>;

// Deliberately NOT sync: it unpacks a zip and spawns a broker.
pub fn install_addon(pack: String) -> Result<BridgeAddon, BridgeError>;
```

**The four `#[frb(sync)]` marks are load-bearing.** flutter_rust_bridge generates a
synchronous Dart signature only for a function that carries one; everything else comes back
as a `Future`. Unmarked, `list_addons()` would be `Future<List<BridgeAddon>>` and could not
be assigned into a field from `_showPage` beside `_perf`, which is filled by sync calls - the
page would have to await all four, and §7.3's "Rescan and Install are awaited off the build
path" would stop distinguishing anything. The precedents are exact:
`#[frb(sync)] presets_dir_path()` is `String? presetsDirPath()` in Dart and
`#[frb(sync)] set_plugin_enabled` is `void`, while `rescan_plugins` is deliberately unmarked
and says so in its own doc comment. `install_addon` stays async for that same reason.
`addons_dir_path` follows `presets_dir_path`'s shape and answers `Option<String>` rather than
a `Result`, because no `BridgeError` variant means "this machine has no home directory" and
inventing one to say so is more surface than the sentence is worth.

`ADDON_STATES` and `ADDON_KINDS` are **closed lists held against Dart** by an addition to
`engine_labels_test.dart`, in `BADGE_REASONS`' shape. `set_addon_enabled` takes the
identifier rather than the match name, because a plugin that never registered has no match
name. `rescan_plugins()` already exists, already returns `skipped`, and the page is its
first reader - `main.dart:91` has fired it unawaited and discarded the report since it was
written.

### 7.3 The page

`SettingsPage.addons` - one enum constant plus a `label` case; the sidebar walks
`SettingsPage.values` so the row and its `settings-page-addons` key come free, and the two
exhaustive switches (`_pageBody`, `_resetPage`) name themselves at compile time.

Three `_sections`:

- **Installed** - one row per addon, keyed `addon-row-<identifier>`: label and version in
  the 190 px column, a `HouseToggle` keyed `addon-toggle-<identifier>`, and a `description`
  line carrying the live state - the bundle path, or why it failed, or which extension it
  wanted. That is the one use §12A.4 permits: something live to report, never help.
  Grouped under kicker titles by kind.
- **Folder** - `addons_dir_path()` on the description line, with **Install…**
  (`addon-install`) and **Reveal** (`addon-reveal`) beside it, and one **read-only row per
  search path** with the path as its description. docs/07 §15 names search paths as this
  page's contents, and it is the only way a Flatpak user learns why the system folder is not
  listed. **Reveal needs a one-line branch, not a reuse:** `reveal_in_folder` is built for
  revealing a *file*, and on everything that is not Windows or macOS it opens
  `path.parent()` - handed the addons directory it opens `~/.local/share/lumit` rather than
  `…/lumit/addons`, on the one platform where §5.2 says the Lumit-owned directory is not a
  convenience but the only route, and the one where the user has just been told to drop a
  bundle in it. Windows (`explorer /select,`) and macOS (`open -R`) select the directory
  inside its parent, which is right. So `reveal_in_folder` gains `xdg-open <dir>` when
  `path.is_dir()`, in the bridge surface.
- **Scan** - **Rescan** (`addon-rescan`), the registered count, and the first few `skipped`
  lines.

Nothing crosses the bridge from `build()`: `list_addons()` is captured in `_showPage` and
after each edit, held in a nullable field exactly as `_perf` and `_exportDefaults` are, and
Rescan and Install are awaited off the build path with `setState` on return. **Reset page**
re-enables everything, and says what it could not reset **on the Scan section's description
line** - the one place §7.3 already permits a live line - under a new `addonsResetIncomplete`
key. There is no precedent to follow here, only a caveat not to inherit: the
Preview-and-cache arm resets the two things it can and says **nothing at all** to the user,
the explanation living in a Dart doc comment, because `_resetPage` is a `void` switch with no
message, toast or description line and the footer carries only the button. The Addons page is
inventing that sentence, so it has to be given somewhere to put it and a key to say it
with.

arb keys, each with a description of at least ten characters in the house forms:
`settingsPageAddons`, `settingsGroupAddonsInstalled`, `settingsGroupAddonsFolder`,
`settingsGroupAddonsScan`, `addonsInstall`, `addonsRescan`, `addonsRevealFolder`,
`addonsNoneFound`, `addonsSearchPath`, `addonsUnsigned`, `addonsSignatureChanged`,
`addonsTrustStoreUnreadable`, `addonsResetIncomplete`, `addonStateRegistered`,
`addonStateDisabled`, `addonStateFailed`, `addonStateRefused`, `addonStateMissing`,
`effectBadgePluginRefused`.

Tests: a metrics test in `settings_metrics_test.dart`'s shape, measuring bands by
`ValueKey` with the drawing's own number in every `reason`; the page's keys added to
`shell_frb_test.dart`'s page-to-keys map; and a new `addons_page_frb_test.dart` injecting
the listing through an `addonsLister` seam, the way `EffectsPresetsPanelFrb.effectsLister`
does, so no CI machine needs an installed plugin.

---

## 8. Containment, and the ceiling LFX cannot express

LFX has **no host allocator**. There is no `memoryAlloc` equivalent, so OFX's
`MAX_PLUGIN_BYTES = 2 GiB` - billed against a running total and refused with a status every
plugin already handles - is simply not expressible. Shipping a plugin host with *weaker*
memory containment than the one it complements would be a quiet regression, so it is
replaced rather than dropped:

- **Declared scratch.** `scratch_bytes_per_megapixel` in `lfx_traits`; the host does not
  dispatch a ROI whose declared scratch exceeds what `lumit-budget` will grant.
- **Windows: a Job Object** with `JOB_OBJECT_LIMIT_PROCESS_MEMORY`, which restores a real
  per-process ceiling - and because a broker hosts one bundle, a per-process ceiling is a
  per-plugin ceiling. **This is the row with an unnamed cost, and it is named here.** The
  workspace has no Win32 bindings at all and avoids them deliberately: `no_console` reaches
  the Win32 surface through `std`'s own `CommandExt` and hand-declares
  `const CREATE_NO_WINDOW: u32 = 0x0800_0000;` "from winbase.h" precisely so that no crate is
  needed, and `grep windows-sys|winapi crates/*/Cargo.toml` finds nothing. A Job Object needs
  real calls - `CreateJobObjectW`, `SetInformationJobObject`, `AssignProcessToJobObject`, and
  `OpenProcessToken`/`CreateRestrictedToken` for the token - so it costs `windows-sys` with
  the `Win32_System_JobObjects` and `Win32_Security` features, Windows-only under
  `[target.'cfg(windows)'.dependencies]`, plus its line in the pull request `deny.toml` asks
  for. The job is created **before** the spawn and the child assigned from its `AsRawHandle`,
  so a plugin cannot outrun the assignment.
- *ponytail:* on macOS and Linux a plugin that lies allocates anyway and is OOM-killed,
  which costs one process and one badged frame. Not graceful; contained.

**Privilege reduction lands here first.** docs/12:645-652 says the plugin server SHOULD run
with reduced privilege and neither existing broker does any of it -
`Broker.entitlements`' own comment says "when it is taken on, this is the process to do it
to first". LFX is the one broker with **no installed base to regress**, which is the whole
argument: Windows gets the Job Object, a restricted token and no network; macOS gets a
`sandbox_init` profile permitting the bundle path and the pipe. *ponytail:* Linux seccomp is
not in v1. Per docs/12:653-656, a blocked capability must surface as a per-plugin permission
the user can grant, never as a silent failure.

---

## 9. The validator

`crates/lumit-lfx-validator` is a **shipped CLI**, its own crate, and it drives the bundle
**through the broker** - never in this process. Two reasons: docs/12:354-356 forbids an
in-process path in v1 ("one fewer code path, and the crash-isolation promise stays
unconditional"), and the one tool whose job is to find a crash must survive the crash it
finds.

Its **program file** joins `no_panicking_prints.rs`'s exemption - `EXEMPT_FILES`, a list of
one, beside the four crate names `lumit-lfx-broker` completed with the broker. **Not** by
putting the binary in `lumit-lfx/src/bin`: `rust_sources` already skips a `bin` directory so
no exemption would be needed, and `EXEMPT_CRATES` is matched against the *crate directory
name*, so exempting `lumit-lfx` would unban `println!` across the whole host library. That
argument is why the validator is exempted by file rather than by crate: it too is a library
with a program in front of it, and the library half keeps the ban its sibling hosts keep.

What it checks, each a named refusal in a sentence:

| suite | what it proves |
|---|---|
| layout | every struct's `struct_size` matches the header |
| describe | no `LFX_UNIT_UNSET`, no duplicate `ParamId`, no unset category, version non-zero; refused kinds reported, not fatal |
| lifecycle | the observed call order against the pinned `LFX_ACTIONS` list |
| **depth** | the whole suite at fp16 **and** fp32, outputs compared within tolerance |
| determinism | two runs bit-identical; wall clock, thread order and pool size change nothing |
| ROI | one bright pixel outside the declared padding must not move the output; one tile against four |
| temporal | the declared window against what `lfx.temporal` actually asks for |
| threading | a stress scheduler, N instances × M frames out of order, with a **deliberate overlap barrier in the fixture** so "no instance re-entered" is proved rather than merely not observed; each annotated callback called from a disallowed thread must refuse |
| fuzz | parameter edges at the declared bounds, one ulp outside, NaN and ±inf, reproducible from `LUMIT_LFX_FUZZ_SEED` |
| `--baseline` | a stored fixture hash: a moved pixel with no version bump is a refusal |

`--baseline` is the only pressure that exists on the one obligation nothing can enforce -
that a vendor bumps the version when the maths changes - and it pairs with §4.1's version
arithmetic, under which any **LFX** release re-keys frames. That pairing is a property LFX
decides and the OFX bridge has not got: the hosted path captures a major number only (§13).

Output is a markdown table in the OFX conformance bench's shape and a non-zero exit. Two CI
jobs mirror the OFX pair: `lfx-conformance` over `lumit-lfx-testplug`'s personalities, and
`lfx-handle-fuzz` under ASan on nightly. Unlike the OFX bench, **both run on a hosted runner
with no FFmpeg and nothing downloaded**, so the gate is on from the first day - there is no
`LUMIT_REQUIRE_LFX_BENCH` to defer.

---

## 10. The template repository

`lumit-lfx-template`, published separately under `lumit-lfx-abi`'s MIT licence, staged
in-tree under `template/` so this workspace's CI builds and validates it before it is
published.

**Built**, with the template:

- `include/lfx.h`, **copied from the in-tree canonical header**, and
  `rust/lfx-sys/src/abi.rs`, copied the same way from the `#[repr(C)]` mirror - both held
  byte for byte by `the_templates_copy_of_the_abi_is_this_workspaces_own`, as is the MIT
  licence file beside them. The mirror is a *module* rather than the crate root because a
  verbatim copy cannot carry a crate doc of its own, and a crate that introduced itself
  with the host's own words would be describing a repository the vendor has not got.
- `rust/lfx-sys` (the declarations, no dependencies) and `rust/lfx` (the safe wrapper: one
  `Effect` trait, one `bundle!` macro, a `Describe` builder with one method per kind, a
  `Values` reader that walks by the host's stride, and a `for_each_pixel` that converts
  both depths so the maths is written once in `f32`). The wrapper catches a panic at every
  callback, because unwinding into C is undefined and a plugin that panics should cost a
  badged layer and nothing more.
- `cpp/lfx.hpp`: the same shape for C++17, header-only, with `lfx::effect` and
  `LFX_BUNDLE` in place of the trait and the macro.
- **One working example per set of bindings**, not per extension: `exposure` in C (core
  only, and every rule the header asks of a plugin), `vignette` in C++ (the frame geometry
  a per-pixel effect must get right to tile, which is the one mistake an example can teach
  out of somebody), and `saturation` in Rust. *The note's `echo`, `vignette-gpu` and `warp`
  are not there, and the reason is this section's own last paragraph: **version 1 offers no
  extension table at all**. A plugin naming `lfx.temporal` in `required_extensions` is
  refused before it is instantiated, so an `echo` example would be a plugin the template
  shipped and the host would not load - and a stub for a table that has no declaration yet
  is a file with nothing in it. The three arrive with the tables they need; the README says
  so where a vendor will look for them, and says which two things that sound like
  extensions are not.*
- A `CMakeLists.txt` that compiles the C and C++ examples straight into
  `Name.lfx.bundle/Contents/<arch>/Name.lfx` and copies the listing beside them, and
  `scripts/stage-bundle.py`, which does the same for a Cargo-built payload - the one step
  `cargo build` does not do. The architecture directory is a cache variable, so the host's
  own `bundle::arch_dirs()` answer is passed in rather than guessed a second time.
- CI, twice over. `template/.github/workflows/build.yml` is the published repository's own
  job - all three languages on Windows, macOS and Linux - and it says plainly when
  `lfx-validator` is not on the machine rather than passing quietly, because the validator
  ships with Lumit and not with the template. The gate that matters is in this tree:
  `lfx-template` runs `cargo test -p lumit-lfx-broker --test template` on the same three
  platforms, which builds all three examples through the template's own CMakeLists and
  staging script and drives each resulting bundle through the ten suites. It lives in that
  package for the flat Cargo reason `validator.rs` does - `CARGO_BIN_EXE_lumit-lfx-broker`
  exists only inside the package that owns the binary - and skips by name, never silently,
  on a machine with no CMake, no compiler or no Python.

**Two things the examples found that no amount of reading would have.** A plugin that calls
`pow` without linking the maths library loads perfectly well and dies on the first frame,
because the symbol is resolved lazily - the validator reported it as "the plugin stopped",
which is exactly right and exactly unhelpful, and the CMakeLists now links `m` where the
platform has one. And MSVC exports a symbol from a DLL only when asked, while the frozen
header declares `lfx_entry_point` with no export attribute and cannot carry one, being the
same declaration the host reads: the ask goes on the link line - `#pragma comment(linker,
"/EXPORT:lfx_entry_point,DATA")` - where it is not the inconsistent-linkage error that
putting it on the definition would be.

The in-tree twin is `lumit-lfx-testplug`. **Built**, with the in-process host: one cdylib, the twelve
personalities in one table (Full, Slim, Temporal, Broken-describe, Duplicate-ids,
Passthrough, ThreadUnsafe, Identity, MissingExtension, Crash, Hang, NoteSpam), the dangerous
three **disarmed unless an environment variable is set in the loading process's
environment** - a scan describes every plugin in a bundle, so a personality that crashed on
sight would take the whole suite with it - and `LumitLfxProbe*` exports recording the call
sequence, the depths it was handed, where each frame said its buffer sat and which region of
it was wanted, how many lines the plugin itself sent, the most process calls ever in flight
and the most ever in flight **on one instance**, which is the number the header's "one
instance is never re-entered" is a claim about. §11 item 12's barrier is there too, as
`LumitLfxProbeRendezvous`, so the overlap a concurrency test is about is deliberate rather
than hoped for; and three more probes make the bundle *misbehave* rather than watch it,
because the faults a host must meet at a count and at a size prefix are ones no honest
personality can be - an entry lying about how many effects it holds, an instance lying about
how long its own table is, and a descriptor pointing at a trait block that really is shorter
than this header's, which is the last of §10's five prefixes and the one a host has the least
excuse to read late. It is a dev-dependency of `lumit-lfx`, so `cargo test -p lumit-lfx` builds
the library its tests load, and it depends on `lumit-lfx-abi` and `half` and nothing else: a
fixture with an edge to `lumit-lfx` would be testing the host against itself, and the halves
are the depth it writes.

**And it stays honest at its own front door**, which is why three more libraries are built
beside it out of `lumit-lfx/examples/` by the same `cargo test`. `not_an_lfx_bundle` loads
perfectly well and exports no `lfx_entry_point` - the second shape of "this file is not a
plugin", where the first is a text file the loader refuses outright. `an_lfx_entry_that_lies`
is an LFX bundle whose entry table lies about itself: its `lfx_entry_point` is mutable, so
one library can be made short at the prefix, made to declare an ABI this host does not speak
and made to hand over no `init` at all, and it counts what the host called so a refusal that
ran somebody else's code first is a failure rather than a pass. (The fourth front-door fault,
an `init` that declines, is an ordinary atomic in the same library and would work with a
`const` entry.) `an_lfx_entry_cut_short` is the short prefix told as the allocation rather
than as the number: a static that really is two words and a hook, because the liar's is the
whole of an `lfx_entry` and a host that read it through a reference before checking the
prefix would be reading its own fixture's bytes with no tool to object. Those faults could
not be personalities: every other test in the suite opens the twelve, and a bundle that was
short at the entry would take all of them with it rather than the one asking.

Its other half is `lumit-lfx/src/local.rs`, the in-process host §4.2 names - **and the one
module in `lumit-lfx` where the workspace's `unsafe_code = "deny"` is given up**, one module
wide rather than a whole crate as the sibling hosts do, so the deny still holds over the
lowering, the protocol and the ring. It opens a module, reads the descriptor list, fills in
the frozen sink so each `declare_*` turns one `*const lfx_*_param` into a `Declaration`,
negotiates the required extensions before `create`, and drives `process` at both depths with
the dense value array written at a stride the caller may widen - widen, and never narrow or
misalign: the stride is raised to the element's own alignment and the array is allocated on
it, so the natural C spelling is an aligned read, which is what the header now says beside
`value_stride` rather than leaving to a fixture's comment. Three rules govern every read of a
stranger's bytes and all three are the header's own. **A count is checked against its ceiling
before its array is read**, never after - which is what `Describe::decline` is public for -
and a count past the ceiling means the array is not read *at all* rather than clamped to the
ceiling and read to it, a clamp being the very read the check exists to prevent, performed
because the check failed. **A string is read to the first NUL inside `LFX_MAX_STRING_BYTES`
or not at all.** And **a size prefix is read before the struct it prefixes**, as a bare
`uint32_t` rather than through a reference to the whole, because forming a `&T` over an
allocation shorter than `T` is already reading somebody else's memory before a field of it is
touched; all five of the ABI's size-prefixed structs are read that way - the entry, the
descriptor, the instance table, the trait block and every declaration - so §2.1's growth
mechanism is a mechanism at each of them rather than a promise at two of them. One refusal
the note did not name arrives with it, because the growth mechanism has an edge on this side
too: `LfxRejection::UnreadableDeclaration` is a declaration - or a descriptor - whose size
prefix is shorter than this header's, or whose identifier has no end inside the ceiling; a
report line, since one row this host cannot read costs that row its control and nothing else.
Two answers are the bundle's rather than a row's: a bundle declaring more effects than
`LFX_MAX_EFFECTS_PER_BUNDLE` is **refused outright**, which is the answer `BrokerMessage::checked`
already gives the same number off the wire and is not a thing to be half of - a truncation
would read as refused to a caller that asks the report and as whole to one that does not -
and an id declared by two descriptors catalogues the first and files
`LfxRejection::DuplicateEffectId` against the second, which is `DuplicateParamId`'s argument
one level up, where the collision decides which effect a saved project resolves to.
Version 1 offers **no extension table at all** and says so: the header declares
`LFX_EXT_TEMPORAL`'s *id* and not the typed table a neighbour would arrive through, and
offering a name with nothing behind it is the "left to fail somewhere later" outcome §4.3
exists to prevent. The temporal *gate* is unaffected, being the trait window rather than an
extension.

---

## 11. Traps

1. **`EffectNamespace::Lfx` resolves to nothing today**, and does so silently - identity,
   no badge, no line. Two literal `matches!` guards must both be widened, which is why §4.1
   replaces them with one predicate rather than editing both.
2. **The catalogue entry and the pass are joined by a string.** Pass first, in one call,
   always. Nothing checks the join at compile time.
3. **`readback_linear_f32`'s `ctx.flush()`.** Inside a batch every pass records into one
   shared encoder; a copy submitted on its own runs ahead of the drawing and reads zeroes -
   the empty frame every OFX plugin was handed until it was found. `readback_linear_f16`
   must carry it too.
4. **`AuxKind::None` is a promise.** It keeps the per-effect cache chain alive, which is a
   real win - and a correctness trap if a plugin lies about not needing neighbours.
5. **`data_dir()` roams on Windows.** Plugin binaries under it would roam between machines
   and across architectures. `data_local_dir()` - §6.1.
6. **The manifest is not the authority.** It is the cheap listing; the code is the truth,
   and a disagreement once the module is open is a refusal.
7. **Parse the stranger's TOML in the broker.** Reading it in the host would put a
   third party's structured text in the process holding the project, the media handles and
   the windows. `lumit-ingress` bounds the damage; it does not move it.
8. **Filtering the listing is not unregistering.** Registration is additive and never
   removes - which is what makes a rescan idempotent and a re-enable instant.
9. **A disabled plugin's code must not run**, so the list travels with `Describe`; and a
   plugin disabled mid-session must stop **now**, so it is read again per render. Two reads,
   two reasons.
10. **`badge_of` is ordered by certainty**, and `plugin_refused` must be read before the
    namespace fall-through or a refused plugin badges `plugin_missing`.
11. **`lumit-lfx-broker` must be added to `make-dmg.sh` and both CMakeLists by hand.** Only
    the macOS codesign loop's glob is automatic.
12. **A stress test that asserts an absence proves nothing** unless the fixture records
    overlap. §9's barrier.
13. **`EXEMPT_CRATES` is matched against the crate directory name.** Exempting a host crate
    to let its `src/bin` print would unban prints across the whole library. A crate that is a
    library with a program in front of it is exempted by **file** instead - `EXEMPT_FILES`,
    which is how `lumit-lfx-validator/src/main.rs` prints its table while everything under
    `src/` beside it is held to the ban (§9).
14. **Nothing here is verified.** TOFU is a real property and a narrow one, and
    `updateSigningKey` is still `''`, so the release trust root is not in force either. A
    fingerprint on screen reads as a guarantee to anyone who does not know what TOFU is.
15. **A dense array is agreed by stride, never by field offsets.** `lfx_value` is the one
    struct with no size prefix, and `lfx_process.value_stride` is what both sides walk by.
    A plugin striding by its own `sizeof` after the struct grows reads correct-looking kind
    tags over wrong values - §2.1. The agreement has a second half the first draft left to
    a comment: **the array's base and the stride are aligned for `lfx_value`**, and the
    header says so beside the field. Without it a host free to widen the stride is a host
    free to put every other element four bytes off, and the natural C spelling -
    `*(const lfx_value *)((const char *)values + i * stride)` - is undefined there and a bus
    fault on a strict-alignment target. A vendor compiling against the published header
    cannot be asked to read unaligned on the strength of a sentence that is not in it.
16. **`PluginPrefs` is fail-open by design.** A damaged file reads as "nothing switched
    off", which is right for a preference and fatal for a trust store. §6.2's fingerprints
    live in their own file, where damaged is a refusal rather than a first use.
17. **The staging folder must not sit in a search path.** The walk descends into
    dot-prefixed directories and the start-up scan fires every launch, so a `.staging` under
    `addons/` is discoverable while it is half-written - §6.2 step 6.
18. **A zero in a trait block is a declaration.** Every trait enumeration starts at
    `UNSET = 0` and lowers to the pessimistic answer; ordering them to mirror the Rust enums
    would make a memset block claim `TRIVIAL` and `EXACT` - §2.4.

---

## 12. Work packages

Ordered so the container-verifiable engine work comes first, each sized for one pull
request, each landing with its tests. **container** = provable in a container with no FFmpeg, no
Flutter and no GPU; **desktop** = needs bridge codegen, Flutter, a card, or a
signing identity.

| | package | verifiable |
|---|---|---|
| **The shared pipe crate** | `lumit-ipc`: the pipe and spawn helpers parameterised by host prefix, the shared constants; migrate both shipping hosts with no name changed ✅ | container |
| **The ABI header** | `lumit-lfx-abi`: the MIT header, the `#[repr(C)]` mirror, layout tests both sides ✅ | container |
| **The namespace wiring** | Namespace wiring: `lfx:` prefix, `namespace_of` arm, `is_catalogued()` predicate ✅ | container |
| **The describe lowering** | Describe → `EffectSchema`: the kind lowering, units, categories, traits, `value_routes` ✅ | container |
| **The in-process host** | `lumit-lfx-testplug` and `LocalHost`: the in-process round trip ✅ | container |
| **The protocol and ring** | Proto, the depth-sized ring, and its ledger reservation ✅ | container |
| **The broker** | `lumit-lfx-broker`: manifest-before-code, handshake, handles, watchdog, replay ✅ | container |
| **Discovery** | Discovery: bundle layout, search paths, the three tables, `Gated`, extension negotiation ✅ | container |
| **`LfxDef`** | `LfxDef`: the `EffectDef`, identity-on-failure, `frames_needed`, the badge seam - the `Lfx` arm the namespace wiring left out of `badge_of`'s missing-plugin match included (§4.1, §4.3) ✅ | container |
| **The instance pool** | The instance pool and §4.4's provisional concurrency policy ✅ | container |
| **The fp16 seam** | The fp16 seam's two verifiable halves: the `EffectDef` hook and `readback_linear_f16` ✅ | container |
| **The validator** | `lfx-validator` and its two CI jobs ✅ | container |
| **Install and trust** | Roster, addons directory, `.lfxpack` install, the Rust Ed25519 verifier, the trust store ✅ | container |
| **The render pass** | The render pass: `gpufx::hosted`, the aux choice, the generic file aux, the matte doc correction | desktop |
| **The bridge surface** | The bridge surface: a fourth namespace, `list_addons`, the disabled filter, codegen | desktop |
| **The Addons page** | Settings ▸ Addons | desktop |
| **Packaging** | Packaging and privilege reduction | desktop |
| **The template** | The template repository and the documentation amendments ✅ | container |

**Built: the ABI header.** `crates/lumit-lfx-abi` is the whole of the C ABI and nothing else -
`include/lfx.h` under MIT, the `#[repr(C)]` mirror in `src/lib.rs`, and both halves of the
layout suite: `tests/layout.rs` asserts every struct's size and every field's offset by
number, and `tests/layout.c` - compiled by `build.rs`, so a moved field is a build failure
rather than a test failure - asserts the same numbers against the header with
`sizeof`/`offsetof`. The offsets reach inside the one union too, since the curve arm is
sixteen bytes and eight-aligned whichever way round its pointer and its count are written and
a swap would hand a plugin the count where the points belong. The two cases the offsets
cannot reach have tests of their own: a host writing an oversized `value_stride` is read
correctly by a plugin built against the smaller `lfx_value`, and a block read to its own
`struct_size` leaves its tail nought, which every trait enumeration lowers pessimistically.

Offsets are only half of the drift, though, because an enumerator renumbered in one half, an
extension id misspelt in one half and a field whose type changed to another of the same width
all move nothing. So the C half also emits the header's own constants and its own spelling of
every string, and *writes* each struct of plain data - every one of them - from the header's
own declarations: `every_constant_is_the_number_the_header_declares`,
`every_string_constant_is_the_spelling_the_header_declares` and
`every_field_carries_the_type_the_header_gives_it` walk them beside the mirror's, so a `FILE`
renumbered to 99, an `lfx.temporalX` and a `float` that became a `u32` fail as loudly as a
moved field.

**And the four structs that are not data are called rather than read.**
`lfx_describe_sink`, `lfx_entry`, `lfx_plugin` and `lfx_host` are tables of function
pointers, and an offset pins where a pointer sits and nothing about its arity, its argument
order, its argument types or what it answers: swapping `lfx_entry.create`'s two arguments, or
pairing `declare_float` with an `lfx_slider_param`, moves no number at all and would hand a
stranger's code an argument of a type it did not expect at the ABI's front door. So the C
half defines one real instance of each table from the header's own declarations, every
callback recording what arrived, and
`every_sink_call_takes_the_record_the_header_pairs_it_with` and
`the_entry_plugin_and_host_tables_are_called_as_the_header_declares_them` call them through
the mirror's own `Option<unsafe extern "C" fn ...>` types. A mismatched signature usually
fails to compile, which is earlier and louder than an assertion; where it does not, the
record's own size prefix is what the call is compared against.

**Three things the header settles that this note left open**, said here rather than buried in
a comment, because they are there to be overturned. *The ceilings are declared* -
`LFX_MAX_OPTIONS`, `LFX_MAX_STRING_BYTES`, `LFX_MAX_TEMPORAL_WINDOW` and the rest - since a
limit each reader invents locally cannot be raised once a vendor has shipped inside it, nor
narrowed once one has shipped up against it; `lumit-ingress` holds the host's side of the same
numbers, the protocol and ring's `BrokerMessage::checked` holds the wire's - the untrusted direction is read
against `LfxRejection::PastCeiling` before anything keeps it - and the validator gets a
case for each. (The describe lowering added one the list was missing - `LFX_MAX_PARAMS`, the most declarations
one effect may push - while the header was still free to grow, and enforces every one of them
in the sink: §2.3.) *`FILE` stays admitted* rather than refused beside `PATH` and `STRING`,
with a *ponytail:* beside the constant saying that
`lfx_value.v.file.path` is NULL until the render pass's generic file aux lands. And *`lfx.thread-unsafe`
is not an extension id*: the opt-out is the `LFX_TRAIT_THREAD_UNSAFE` bit, which is what §2.4's
own struct shows - §2.6 and docs/12 §3.4 were the halves that needed amending, and the template
amended both.

**Three more the review settled, each of them the last moment to.** *`LFX_MAX_DIVIDERS` is
`LFX_MAX_OPTIONS`* rather than a quarter of it: a dropdown draws at most one rule per option,
so the option ceiling is the only divider ceiling that cannot turn out too small, and a
ceiling here declines the whole declaration rather than trimming it - sixty-four would have
admitted a 256-option dropdown and refused the list that groups it into pairs. The
relationship is the assertion, not the number. *The three answers the plugin gives cross as
`uint32_t`*, non-zero being true, for §2.1's reason. And *the growth rule is written down per
struct* in the header, because the sentence it replaced was true of no reader in this tree.

**Built: the in-process host, and the front door it closed on a second pass.** `crates/lumit-lfx-testplug`
is the twelve personalities behind one `lfx_entry_point` and `lumit-lfx/src/local.rs` is the
host that drives them in this process; §10 and §4.2 are what each of them is made of. What a
reading of the finished package found missing was not a behaviour but a name: four of the
answers `LocalHost::open` gives are about the entry table rather than about anything inside
it - a `struct_size` shorter than this header's, an `abi_version` this host does not speak,
an `init` hook that is not there at all, and an `init` that declines - and all of them were
written, reachable and pinned by nothing, so "the entry" was the one of §10's five
size-prefixed structs whose rule no test read. They are told now by a library of their own,
`lumit-lfx/examples/an_lfx_entry_that_lies`, because a personality that was short at the
entry would take the eleven beside it down with it; the fixture counts what the host called,
so each refusal is asserted to have run none of the bundle's code rather than merely to have
been reached. `an_init_that_answers_anything_but_nought_has_loaded` is the same edge read the
other way, and is the only place §2.1's `uint32_t`-rather-than-`bool` rule is a test. One
more guard had been left unnamed beside them -
`a_buffer_that_is_not_the_size_the_request_names_never_reaches_the_plugin`, which is the
host's own caller held to the numbers the frame carries, and which now names the side it
refused so that "in either direction" is the refusal's own word rather than the test's.

**And a third pass, because the second had pinned the refusals and not the rule.** The
entry's own prefix was still read *through* a `&LfxEntry`, and so was the trait block's, so
two of §10's five structs had the sentence and neither had the mechanism; a bundle built
against an earlier, shorter table was refused only after a reference had been formed over
memory it does not own. Both read the prefix bare now, as the descriptor, the instance table
and every declaration always did, and both are pinned:
`a_trait_block_shorter_than_the_header_reads_as_the_null_it_is_indistinguishable_from`
through a `LumitLfxProbeShortTraits` that costs no thirteenth personality, and the entry
through `an_lfx_entry_cut_short`, a second example library whose static really stops where
its prefix says it does - the liar can only *say* it is short, and a host reading it the
wrong way round would find its own fixture's bytes rather than a sanitiser. Two refusals were
carrying meanings that were not theirs: a present-but-null `init` was `NoEntry`, whose
sentence sends a vendor looking for a symbol that is right there, and a bundle directory this
host could not spell as a C string was `InitRefused`, which the suite had just pinned to mean
"the plugin answered nought". They are `NoInit` and `UnspellablePath` now. *ponytail:*
`CreateRefused` and `InstanceRefused` are still constructed and unpinned; what they want is
no longer a thirteenth personality - `an_lfx_entry_that_lies` is a bundle outside §10's
frozen twelve, and one descriptor of its own whose `create` answered null would pin both -
but it is a descriptor list the liar has not got. `UnspellablePath` is unpinned too, and
cannot be reached through `LocalHost::open` at all: the loader refuses a path with a NUL in
it first.

**Built: the broker, and four things it settled that the note left to it.** `crates/lumit-lfx-broker`
is two arguments and a serve loop; `lumit-lfx/src/ipc/broker.rs` is the supervisor;
`src/manifest.rs`, `src/quirks.rs`, `src/ipc/handles.rs`, `src/ipc/identity.rs` and
`src/ipc/pipe.rs` are what the two of them are made of.

*The module path is an argument and the bundle path is a message.* `Manifest{path}` already
names the bundle in a frozen protocol, and nothing in that vocabulary names a module - so
the broker is spawned with the payload and told about the bundle, which leaves §5.1's ordered
per-target arch list whole for discovery to write rather than half-written here.

*`Described` grew.* The sink runs in the second process, so a record carrying only an identity
and a trait block would leave the host with a plugin it could name and no rows to put on a
panel; `DescribedPlugin` carries the declarations, the headings and the report lines, and
`Described` carries a report of its own for the lines that belong to the bundle rather than to
any plugin in it. Nothing anywhere grew a third shape of the same record:
`PluginDescriptor::from` is the one conversion back.

*A refusal crosses the pipe as itself.* `LfxRejection` is what the sink files and what the
page prints, and the sink is in the other process, so the whole enumeration is now
serialisable - typed across the boundary rather than rendered into sentences there, which is
what lets `lfx-validator` ask for one by name on the side that receives it. The two
`&'static str` fields cross through a closed list, `INTERNED`, and a word that is not in it
ends the conversation rather than being printed.

*`ACTION` has nowhere to go.* The frozen entry table is `init`, `destroy`, `describe`,
`process` and `get_extension`, and there is no press hook at all - so an `Action` message is
acknowledged and nothing runs. `Done` rather than `Failed` deliberately: a `Failed` is a
strike, and three presses of a button would disable an effect that has done nothing wrong.
Either the entry table grows a press hook or `ACTION` joins `PATH` and `STRING` as a kind
version 1 reserves; that is a call for the maintainer and it is at the header.

**And five things a second pass over the broker put right**, each a property this note already claimed. The
buffer a plugin is handed **starts as the input** rather than as nought, so a plugin that writes
nothing renders identity through the broker as well as in process and an untouched ROI margin
keeps the picture (§4.2). The header's ceilings reach **inside** a declaration, not only around
the list of them, so a label or an option list off the pipe is bounded by a number rather than
by the transport cap (§3.2). A describe that failed reaches the host as a **refusal with its own
sentence** instead of as an absence, which is what §5.3's `REFUSED` table is made of (§4.3). A
handle the host does not hold is answered by the **host**, so a press racing a layer deletion
costs no strikes (§3.5). And `Broker::process` answers a **typed** `BrokerError` rather than a
sentence, as docs/14 §4 requires and as the older host's `Broker::render` already did - with
the frame that comes back held to the slot and the bounds the job asked for.

**And two more that pass found, each a rule this note states and the code did not keep.**

*An answer out of turn was counted as a success.* The supervisor read a `Failed` as a strike
and took everything else as the reply, so a broker one message behind on the pipe - answering
promptly, inside the deadline, alive - put the strike count back to nought at every frame.
"Three **consecutive** strikes" was therefore unreachable for the one failure that leaves the
two ends unable to agree about anything again: the answer this question was owed is still on
the pipe, and the next message collects it. "Exactly once" is a count, and a count cannot tell
a reply from the reply to the question before last. So the question names what may answer it,
in `HostMessage::answers` beside `expects_reply` - exhaustive, no `_` arm, and pinned to the
other vocabulary by `every_answered_message_names_the_answer_it_admits` - and an answer the
message does not admit is a strike that *replaces* the broker rather than one that resets the
count, because a desynchronised process is suspect where a refusal is merely unwilling.
`an_answer_to_a_question_nobody_asked_is_a_strike_rather_than_a_reset` is that, driven through
a broker told to answer a frame with somebody else's reply.

*And a ledger's no was put again at every frame.* `Ring::create` halves its way down until the
governor says yes and takes `RING_MIN_SLOTS` over its head when it never does, so a ring on a
full machine holds fewer slots than the plan that asked for it - §3.4's own arrangement, three
slots unbilled with the refusal standing on `Ledger::denials`. `Broker::fit` measured the wish
against that answer, found the ring too small for the very plan it *is*, and rebuilt it: a
file, a mapping, a reservation the ledger had already refused and a control round trip, per
frame, for as long as the pressure lasted, arriving back at the same three slots every time.
It asks against the plan the ring was made from instead, and a wider plan is what puts the
question again. The ring's own name is what
`a_ring_the_ledger_narrowed_is_not_made_again_for_every_frame` reads, since a replacement gets
a new one.

**And three ceilings were written, reachable and pinned by nothing** - the same gap the second
pass found at the in-process host's entry table, in the other half of the package. `RingTooSmall` is the whole
of §3.4's prefetch sentence, the ceiling counted in **slots** that halving a slot does not
lift; `TooManyInstances` is §3.5's `MAX_LIVE_INSTANCES`, which bounds the length of a replay as
well as the memory. Both are the host's own refusals, taken before the pipe is touched, so
neither costs the plugin a strike - which is the half worth pinning, and which
`a_shipment_wider_than_the_ring_is_refused_before_a_slot_is_written` and
`a_bundle_may_not_hold_more_instances_than_a_replay_can_carry` now say. *ponytail:* the third,
`NoMoreHandles`, wants a million creates in one session to reach and is pinned one level down
instead, at `Handle::encode`'s own `an_index_past_the_bits_mints_no_handle`; it becomes
reachable from here only if the twenty-bit index ever shrinks.

**And four the third review found, three of them the same rule read one level further in.**

*A wrong answer of the right kind was still counted as a success.* The out-of-turn check held a
reply to the **kinds** its message admits, which is all a message can say about itself, and
`action` put the strike count back to nought the moment it passed - before `process` had read
the slot the answer named or the rectangle it carried. So the two misanswers the fixture ships
beside the out-of-turn one, a `Processed` naming the input slot and a `Processed` carrying a
1×1 frame, were refused frame by frame and counted a success frame by frame: inside the
deadline, alive, never replaced, never put away, badging for the length of the session. A
success is an answer that was **accepted**, so the reset moved to `Broker::accepted` at the
seven call sites and both content refusals strike as a process out of step.
`a_frame_that_is_not_the_one_asked_for_is_refused_rather_than_served` drives three consecutive
frames of each and reads the count.

*A ring that could not be made again was a broker with none, for ever.* A regrow drops the ring
it is replacing before it asks for the new one, so a transient at that moment - a full disk, no
file handles left - left `self.ring` empty and `self.plan` already advanced to the plan nothing
had been made from. Every later frame answered `NoRing` before the pipe was touched, which
counts nothing, so the watchdog could not fire: the bundle was neither mended nor put away. The
plan is now committed only once a ring exists to match it, and the failure goes through the
strike that replaces the broker. `a_ring_that_cannot_be_made_again_reaches_the_watchdog` takes
the ring's directory away underneath a live broker - the one way to reach that failure from
outside, and what `BrokerConfig::ring_dir` is for.

*And the `Open` was the one question whose answer nobody held.* `HostMessage::answers` had a
single reader, `action`, and `ring_is_shared` ended `_ => return`: a broker answering the `Open`
with somebody else's reply was taken for one holding the ring, the name was unlinked on a
mapping that never happened, and the answer really owed to the `Open` stayed on the pipe for
the next question. It reads the list now, and
`an_answer_to_an_open_is_held_to_what_the_open_admits` drives it; `Challenge`'s list is held by
the handshake's own exhaustive match, which the list's doc comment now says out loud rather
than leaving to be discovered. `BrokerError::Unexpected` carries a message name at every one of
its nine sites, so one field means one thing.

*The fourth is the report.* A restart takes the replacement broker's own report, which dropped
the lines the **host** had filed about its ring - and the replacement ring is made from the
same plan on the same pressed machine, so after one crash the Addons page stopped explaining
refusals that carried on happening. `Broker::ring_lines` is filed again by the restart, and
`a_narrowed_rings_line_is_filed_again_by_the_broker_that_replaces_it` reads it. The lines
themselves were saying the wrong thing as well: `WindowHeldToTheRing` and
`RingNarrowedByTheLedger` promised a prefetch "staged in more journeys", which nothing stages -
v1 has no frames-request seam - while the same commit's own test pinned the shipment being
refused. Both sentences now say what the ceiling does, which is what §3.4 above says too.

**Built: discovery, and three things it settled that the note left to it.** `lumit-lfx/src/bundle.rs`
is where a plugin lives on disk - the `.lfx.bundle` layout, §5.1's ordered **per-target**
architecture list with a sorted fallback that passes over the six names that are not this
build's, the sorted four-level walk
that never opens a bundle looking for another, and §5.2's search paths.
`lumit-lfx/src/discover.rs` is the scan: one broker per bundle, spawned, asked for the listing -
always, and **before** the payload for this machine is looked for, so a bundle built for
another CPU still gives the Addons page its rows - and then for the description, and dropped -
`lumit-aplug`'s arrangement, because a scan wants
descriptors and a layer that later wants a live instance gets a broker of its own.
`lumit-lfx/src/extensions.rs` is the negotiation, and `lumit-project/src/roster.rs` the third
table.

*The addons directory is `lumit-ipc`'s.* §5.2 carries the argument: it has to be appended
inside each host's own `search_paths()`, and no host depends on the project format. All three
hosts search it and all three pin it by name - the older two's own search-path tests read the
first entry and the length, which a deletion of the line survives.

*There is no in-process scan, and that is the design rather than a gap.* The OFX scan has one
and the LFX scan cannot: a bundle's listing is a stranger's structured text and §11 item 7
puts parsing it in the broker, so a host-side scan would have to break the rule to read the
one file it reads first. `the_host_never_parses_the_bundles_own_toml` now sweeps discovery as
well as the supervisor, and the folder-of-bundles test therefore lives in the broker's crate
beside the rest of §14 item 5.

*The running switched-off list is one table, shared rather than copied.* `DISABLED` is itself
a `DisableList`, and what a broker is spawned with is an `Arc::clone` of it - so
`Broker::disabled_now`, which exists to read the list late, reads the live one and a tick
landing between the spawn and the describe is not lost. A snapshot would have made §5.4 place
1 true only for a tick that arrived before the scan started.

*The gate is delivered with the seam it gates rather than the instance it will gate.* `Gated`
reads the running list inside every `process` and `press`, returns the input byte for byte and
files `lumit-ipc`'s shared `DISABLED_REASON` - and `BrokerError` grew a `SwitchedOff` beside
its `Disabled`, because one is a person ticking a box and the other is the host putting a
bundle away after three strikes and the badge tells them apart. What it wraps in the shipping
path is `LfxDef`'s, so `LfxDef` supplies the inner `LfxHost` and discovery pins the gate against a
counting stub: a switched-off plugin's code is not merely ignored, it is not reached.

**What a second pass over discovery found.** §5.4 place 1 did not hold for the commonest bundle shape. The
disable travels with `Describe` and the broker filters on it - but a describe is what opens
the module, and `scan_bundle` asked for one whatever the listing had left, so a bundle whose
only plugin was switched off ran the library's initialisers and its `init` at every start-up
scan after the tick. The scan now counts the survivors of the listing and returns before the
describe when there are none; the rows are already in `listed` and the skip lines already
filed, so the page loses nothing. The named test for place 1 had been asserting that
`Arc::clone` of a `Mutex` shares its contents - no broker, no describe - so it is renamed to
what it does, `the_disable_list_a_broker_is_spawned_with_is_a_share_of_the_running_table`, and
the name §14 gives place 1 now belongs to a case that spawns a real broker, ticks the box
between the `Manifest` and the `Describe`, and reads what came back. The related trap was
`ScanOptions::disabled`, which the scan merged into the running table and never subtracted
from: a second scan carrying a narrower preference left the dropped identifiers switched off
for the rest of the session. The field is gone. There is one list, seeded whole by
`set_disabled`, and a scan only reads it.

*Two ceilings and two sweeps.* `PluginRoster::save` had none while `load` refused past 2 MiB,
so one bundle's honest listing - 1024 plugins of 1024-byte strings - could write a file that
read back as "nothing ever seen" for every plugin of every kind on the machine. The writer now
forgets the rows seen longest ago until the file fits. `the_host_never_parses_the_bundles_own_toml`
swept three file names it had been told about; it walks the crate's `src` instead, so the rule
holds against modules nobody has written yet, which is the whole point of §11 item 7.

*And the operating system is an axis too.* `arch_dirs` reached its Linux arms by "not Windows
and not macOS", so FreeBSD, illumos and Android were handed `linux-x86_64` and a broker spawned
on a foreign ELF - the same class of bug the per-target list fixed on the CPU axis. The three
operating systems are named, everything else gets the empty list, and the test's own model of
what this machine can load gained the same axis.

**Built: `LfxDef`, and three things it settled that the note left to it.**
`lumit-lfx/src/def.rs` is what a described plugin becomes: `LfxDef` is the `EffectDef`, and
`BrokerHost` is the bundle's broker behind both of the seams that reach a plugin. The
resolved bag becomes the dense value array through `value_routes` read off the built schema,
one element per declaration that carries one - a point folded back into the one element it
was spread from, a button taking none, and a row the bag has nothing for keeping the
plugin's own declared number. Both depths cross as themselves. Every road out that is not a
picture leaves the caller's buffer untouched and files a sentence taken on read.

*The instance is a seam of its own.* §4.2 named one trait and the gate discovery landed reads the
switched-off list inside exactly two calls; opening a live instance is a third place a
plugin's code runs, and a definition reaching it through the gate would mint a handle before
the gate could refuse. So `LfxInstances` sits beside `LfxHost`, `LfxDef::lease` reads
the running list itself, and a plugin switched off *after it registered* and before its row
was ever drawn is refused with the same typed `SwitchedOff` - never opened, never described,
never asked for a frame. **Failure 6 is a different sentence and `LfxDef` does not close it**: a
plugin switched off before the scan never registers, so `lumit_core::fx::def` misses, there
is no `LfxDef` to refuse, and `badge_of` falls through to `plugin_missing` - "not installed
on this machine" for a plugin that is. That is the older host's behaviour too; the roster
(§5.3) is what gives such a plugin a row on the Addons page, and giving its *layer* the right
badge wants the roster read from the bridge, which is the bridge surface's.

*Telling an instance its values and asking it for a frame are one turn.* Frames of one row
are dispatched out of order by design (§4.4), so a definition that let go of the instance
between the two would hand a frame back painted with the other frame's numbers, as an `Ok`,
to be cached under its own key. `LfxDef` made the pair indivisible with a per-instance `Live::turn` -
per instance rather than per bundle, and deliberately not the instance table's own, since
`frames_needed` reads that table on the key walk and may not wait behind a render. **That lock
is gone**: the instance pool replaced it with the lease, and neither `Live` nor `Turn` is anywhere in the
tree, so read the instance pool's paragraph below rather than looking for them.
`two_frames_of_one_row_are_each_painted_with_their_own_values` drives two threads at one row:
the second frame is not started until the first is inside the driver with its lease still
held, and what the case asserts is that nothing of the second reached the instance while it
was. **It is built under a policy that pins the pool to one** - the ring narrowed to its floor -
and asserts that exactly one instance was opened, which is §11 item 12 taken seriously: under
the open policy the rest of the suite renders with, the second thread opens a second instance
beside the first, and the absence the case asserts is then unreachable on every machine rather
than proved on any.

*`apply_f16_temporal` never declines, on any road out.* The hook's `false` means "use the f32
path", and for a plugin the f32 path is the same plugin asked the same question a second
time: an fp16 project would render every hosted frame twice and a frame that failed would
fail twice. So `LfxDef` answers `true` whatever happened - a failure, a frame of the wrong
shape, and a buffer of no area or short of the size it came with, all of which leave `rgba`
exactly as it was found, which is the whole of what the hook's contract asks of a `false`.
`a_failed_fp16_frame_is_identity_rather_than_a_second_attempt` pins every one of them.

*`frames_needed` is read off the last render rather than asked for.* The key walk calls it
once per live effect per frame and may not talk to another process to answer it, so what an
instance's last `Process` said it reads is kept beside the instance, clamped to `MAX_OFFSET` -
which is `LFX_MAX_TEMPORAL_WINDOW` itself rather than a second spelling of sixty-four, so
widening the header's window cannot leave the frame key computed over a narrower one than the
plugin reads - and an answer of nothing but the frame in hand is `None`, which is "whatever
the schema declares".

*One walk answers which declarations carry a value.* `schema::value_elements` is that walk
and `value_routes` numbers its elements by it, so the dense array's defaults and its routes
cannot come to two opinions about a kind and shift every element after it;
`every_declaration_that_carries_a_value_has_a_default_to_carry` sweeps every kind the frozen
sink admits against `carriage`'s own answer.

And the badge caught up (§4.3): `badge_of` reads the `REFUSED` table before it falls through
to the namespace, the missing-plugin arm gains `Lfx` beside `Ofx | Clap`, the switched-off
comparison is `lumit_ipc::DISABLED_REASON` rather than one host's re-export of it,
`BADGE_REASONS` gains `plugin_refused`, and `engine_labels_test.dart` holds that constant the
way it already holds the reasons.

*And four things a second pass over `LfxDef` found, fixed in the main tree after the instance pool.* The
thread-local a badge is read off is now written on **every** road out of a render rather than
only on the ones that failed, which is the assignment `lumit-ofx`'s definition always had and
`LfxDef` copied the doc comment of rather than the line: the slot is one per thread and nothing
reads it after a frame that worked, so without the clearing a failure of one plugin badged
the next plugin's good frame on the same render worker. The case whose name promised that
became two, the second of which renders a good frame with nobody reading in between and fails
on the old code. `two_frames_of_one_row_are_each_painted_with_their_own_values` is pinned to
one instance, since its absence was unreachable under the open policy it was written with
(§11 item 12). A stored seed above `i32::MAX` is reinterpreted on the press road as the
resolve walk reinterprets it, rather than clamped, so one row tells one instance one seed.
And the `derived.` prefix is reserved: a plugin declaring a control there had its value
overwritten on every render with nothing said, and is now `LfxRejection::ReservedParamId`.

**Built: the instance pool, and four things it settled that the note left to it.** `lumit-lfx/src/pool.rs`
is §4.4's policy as one value and the table it governs: a frame leases a live instance for its
own length and hands it back, `Policy` carries every ceiling the section names, and
`Throughput` is the measurement the growth clause asks for.

*The lease replaces the lock rather than sitting beside it.* `LfxDef` made telling an instance its
values and asking it for a frame indivisible with a per-instance `Turn`, because one row had
one instance and two frames of it had to take turns. A leased instance is not visible to any
other frame, so the pair is indivisible by construction and the lock is gone - two frames of
one row are either two instances or one frame after the other, and neither is ever told
anything in the middle of the other's turn. `two_frames_of_one_row_are_each_painted_with_their_own_values`
still passes, which is the point: the property was the claim and the lock was only one way of
keeping it. *The case itself needed the one edit the second pass over `LfxDef` found and the instance pool did not.* It was
written under the open policy, where two frames of one row are two instances and the
interleaving it asserts the absence of could not have happened either way; it is built under a
policy pinned to one now, and says so in an assertion beside the absence rather than in its
name alone.

*Growth is a number the ledger already answered, not a second reservation.* §4.4 says a pooled
instance's slots are part of the ring's reservation, and the obvious reading - reserve the
slot bytes again as the pool grows - double counts the bytes `Ring::create` already bought. So
the scratch is the only real `Reservation` here, and growth reads the ring's **granted** slot
count over what one frame of that plugin ships: `hi − lo + 2`, which is two for the windowless
common case and twelve for a `t ± 5` declaration, the same arithmetic `slots_for` sizes the
ring by and `Broker::process` holds a shipment to (`neighbours + 2` against the slots there
are). A ring the ledger narrowed to
`RING_MIN_SLOTS` therefore pins the pool to one, which is a refusal by the ledger arriving
where the money was spent. The count is published by `Broker::granted_slots` and held as a
handle rather than copied, because `fit` replaces the ring mid-session and the stale copy is
always stale in the permissive direction.

*What is measured is busy time, not the wall clock.* "Measured per-frame throughput" over wall
seconds is a rate an idle minute dilutes, and a definition is leaked for the session - so a
scrub, a pause and a return would have banked a rate the next epoch could never beat and the
pool would never have grown again. `Throughput` runs its clock only while something is in
flight: `arrived` starts it when the pool goes from idle to busy and `completed` stops it when
the last frame in flight ends, and `an_idle_gap_is_not_a_slow_epoch` is where that is held.

*A zero in the trait block does not pin the pool - and a bundle's declaration pins all of it.*
Every other unstated trait lowers to the pessimistic answer (§2.4), and this one deliberately
does not: `lfx.thread-unsafe` is a bit a plugin sets, and reading its absence as "assume the
worst" would make the serial case the commonest case - a performance answer to a correctness
question. `a_declaration_the_plugin_never_made_does_not_pin_the_pool` is where that decision is
held. What the *bundle* declared is a different question with the opposite answer:
`Serial::for_bundle` arms the lock from the whole described list and `Policy::declared` reads
the lock rather than the descriptor, so a bundle where one plugin declares the bit serialises
the plugins that did not - which is the header's own sentence, and the only reading under which
a shared broker and a shared ring mean anything.

**Built: the validator, and five things it settled that the note left to it.**
`crates/lumit-lfx-validator` is the shipped CLI and the library under it: `src/suites.rs` is
the ten questions, `src/fuzz.rs` the seeded parameter edges, `src/baseline.rs` the stored
digests - one per mandatory depth, so a vendor who rewrites their `apply_f16` maths and leaves
the version standing is caught by the one thing that can catch them - `src/finding.rs` the
closed list of things a row may say, and `src/main.rs` the program, which
`no_panicking_prints.rs` exempts **by file** rather than by crate directory, leaving the
library beside it under the same ban as the hosts' own. The two CI jobs are `lfx-conformance`,
which runs the suites and then the shipped program over the same staged bundle, and
`lfx-handle-fuzz`, which runs the whole pass again under ASan on nightly. Neither downloads
anything, so the gate is on from the first day, as §9 said it would be. A run that asks to
write a record and names nowhere to write it is refused at the command line rather than
measuring everything, storing nothing and exiting 0; the run that does write says what it
wrote, rather than printing "run again with `--write-baseline`" at the vendor who just did. The
table's suite column carries one heading beside the ten - `bundle`, for what the bundle
answered rather than any plugin: the scan's own report lines, the one ceiling a bundle can be
past, and a plugin whose own broker never started, which is a fault in the bundle and not an
answer to any of the ten questions.

*One broker per plugin, not one per bundle.* The editor spawns one per bundle and is right to;
here a fuzz pass that struck one plugin out three times would take the other eleven down with
it, and eleven rows saying "the bundle was put away" say nothing about eleven plugins. The
listing, the describe and the bundle's own report lines are read once, from a broker of their
own, because they are the bundle's answers rather than any plugin's.

*`lfx-handle-fuzz` instruments the broker, because that is where the stranger's bytes are.*
An LFX handle is a `u32` into a map and cannot be undefined behaviour however forged - the
name is kept because §9 chose it and because the OFX pair is the shape being mirrored - so
what the sanitiser is pointed at is the ABI edge: `lumit-lfx`'s `local` module, which runs in
the second process, which `RUSTFLAGS` reaches too. A validator run under ASan is every size
prefix, every string and every stride read with the sanitiser watching.

*Three suites' claims are functions of their own, so that what they find can be asserted
without a fixture that misbehaves.* The twelve personalities are honest about their reach, and
the ROI suite's whole value is what it says when one is not - so `moved_inside`,
`disagreement` and `scrambled` are pinned against pictures and orders in
`a_sample_that_moved_inside_the_region_is_found_and_one_outside_it_is_not`,
`two_depths_disagree_relatively_and_a_nan_always_disagrees` and
`the_stress_order_is_a_permutation_and_is_not_in_order`, and the end-to-end cases pin what an
honest bundle gets. The ROI suite's **geometry** is pinned the same way and is the half worth
spelling: §4.6's bright pixel goes one past the declared *reach* - the region's edge less
`roi_padding_px` - and not one past the region, because a pixel one outside the region is
inside the padding any padded plugin declared and is one it is entitled to read. Putting it
there would refuse the first honest eight-pixel blur a vendor shipped and never reach the
dishonest one, so `the_bright_pixel_is_past_the_declared_reach_rather_than_past_the_region`
holds the arithmetic against a simulated kernel that reads exactly what it declared and one
that reads a pixel more. `--allow-refused` is the same discipline one level up: a refusal the
run expected is a line, a plugin expected to be refused that **passed** is a refusal of its
own, and the downgrade reaches the **describe** refusal the flag is about and nothing else -
a named plugin that starts describing and then moves a pixel with no version bump is refused
as loudly as any other, so the flag CI runs the program with cannot become a way of turning
the program off.

*Several of §9's suites are narrower than the sentence that asked for them, and each says so in
its own doc comment.* The **threading** suite cannot arm the fixture's
`LumitLfxProbeRendezvous`, which is an export of the plugin and therefore reachable only from
the process that loaded it - and through a broker there is nothing to overlap anyway, since
`Broker::process` wants `&mut self` and one bundle has one broker whose lock is held across a
render (§4.4). So it **measures** the overlap and files `NothingOverlapped` as a report line
when there was none, which is §11 item 12's rule kept rather than quietly broken; what that
line measures today is the validator's own `Mutex` as much as the host's, and it becomes a
measurement of the host on the day `Broker` offers a `&self` render path. §2.6's other half -
each annotated callback called from a disallowed thread and required to refuse - needs an
extension to call, and version 1 offers none, so it is not here at all. The **temporal** suite,
with no `lfx.temporal` table behind it in version 1 (§10), is reading the declaration back: it
catches an offset outside the declared window, which is real, and not yet the plugin §4.6's
trap is about. The **lifecycle** suite drives every step of `ACTIONS` in turn and builds its
sentence from the steps that answered, but the order it reports is the order it *drove*: what
the plugin observed is unreadable from another process for the threading suite's reason, and
the half after destroy is a host-side check, since `Broker::set_values` and `Broker::process`
refuse a handle the host's own map no longer holds before anything crosses the pipe. The
**layout** suite reads the ABI version and the size prefixes the broker could not read rather
than comparing each `struct_size` against a number of its own - the broker already refuses a
prefix this header cannot read, and a second opinion written out here would be a third number
to keep. And **determinism** renders one comp frame twice on one instance and once on a fresh
one: the wall clock and the frame time are held still on purpose, because a picture that moved
with either is a correct answer rather than a fault. Each becomes the suite the note asked for
with nothing else here to change - the first when a host renders one bundle's frames
concurrently, the second when the extension has a table, the third when a plugin can report its
own call order across the pipe.

*And "the whole suite at fp16 **and** fp32" is the depth suite and the baseline between them*,
which is the last of the narrowings and the one with a cost behind it: the same frame at both
depths, compared within a relative tolerance and hashed at each, while the suites that drive a
region, an order or an edge value drive fp32. Running all ten twice would double a gate that
runs on every push, to ask at each of them the question the depth suite asks once.

**Built: install and trust, and five things it settled that the note left to it.** Two thirds of this
package's row had already landed with discovery - the roster is `lumit-project/src/roster.rs` and
the addons directory is `lumit_ipc::addons_dir()` - so what is here is the pack:
`lumit-lfx/src/install.rs` is §6.2's seven steps, `lumit-lfx/src/trust.rs` is the store the
second of them reads, and `lumit_ipc::staging_dir()` is the sibling directory the fourth
unpacks into. `ed25519-dalek` and `sha2` are the workspace's first signature check in Rust;
the verify is `verify_strict` rather than `verify`, because the small-order keys and
non-canonical encodings the strict form refuses are exactly the ones that let one signature
check against two different messages, which is the property "the same publisher as last time"
is built on.

*A signature over the manifest alone is a signature over nothing.* The detached signature
covers `manifest.json`, and the bundle is somewhere else in the archive - so as §6.2 first
wrote it, a pack could carry a signed manifest and somebody else's payload and be refused by
nothing. The manifest therefore declares a **SHA-256 per archive entry**; every entry must be
declared and every declaration must be an entry, and each is hashed as it is unpacked. The
signature binds the manifest, the manifest binds the bytes, and trust on first use binds the
key to the addon. That is what `sha2` is for, twice over - the other use is the fingerprint.

*The key rides with the pack, because there is no out of band.* There is no certificate
authority here and no list of publishers Lumit knows; what the signature is for is comparing
this pack's key against the last one's. So `manifest.json.sig` is ninety-six bytes - the
thirty-two of the public key followed by the sixty-four of the signature - rather than a
signature whose key has to arrive by some route nobody has built. A key in a file of its own
would be a second file with the same trust properties and one more way to be half present.

*The fingerprint is compared at step 2 and written down after the smoke test.* §6.2 puts both
at step 2, and recording there means a pack that fails the layout check has claimed that
identifier's key for ever: the vendor's own next pack would be refused as `signature_changed`
by a broken pack an attacker could have sent. So the comparison is where the note puts it and
the write is after everything a pack can fail on its own account - the layout and the
manifest-in-a-broker - with `a_pack_that_did_not_install_records_no_key` holding that. It is
**before** the rename rather than after it, which is the one place the write can be put where
neither failure is silent: `save` is a file write and can fail on a full disk or a read-only
profile, and a failure after the rename would answer `trust_store_unwritable` for an addon
that was by then installed and discoverable and whose key was not remembered - the mechanism
quietly switching itself off, which is what `TrustError::NotWritten`'s own sentence is against.
`an_install_that_cannot_record_its_key_installs_nothing` pins it. *ponytail:* what is left is
the rename failing after the record was written, which remembers a key for an addon that did
not land; the vendor's own next pack is then `Known` rather than `FirstUse`, which is the
harmless direction of the trade.

*Trust on first use has to refuse the pack with no key, not only the pack with the wrong one.*
Consulting the store only for a signed pack leaves the whole mechanism defeated by *deleting*
`manifest.json.sig`: the attacker needs no key, only a zip without one, and the addon installed
under a recorded fingerprint is silently replaced. So the store is asked on every install and
answers `signature_missing` for a pack that carries none where a key is written down -
`a_later_unsigned_pack_for_a_signed_addon_is_refused_by_name`, and end to end in the broker's
suite. An unsigned pack is still the calm line docs/12:653-656 asks for, for an addon nobody
has ever seen.

*A key is filed under an identifier and an install replaces a directory name, and they are not
the same key.* `land` puts the staged bundle at `addons/<its own directory name>`, which comes
out of the archive; the store is keyed on `manifest.id`, which the same pack also chose. With
nothing relating the two, a pack under a fresh id and a fresh key installs as an ordinary first
use and overwrites another vendor's bundle - refused by nothing, because everything the key can
say is about an addon nobody has heard of. So the store writes down which addon's install owns
each bundle directory, for unsigned installs as well as signed ones, and a pack that is not
that addon is refused as `bundle_claimed` before anything is renamed:
`a_pack_may_not_replace_a_bundle_another_addon_installed`, in the store's suite and end to end.

*The rename is after the smoke test rather than before it.* §6.2 numbers the rename 6 and the
manifest-in-a-broker 7, and then says a bundle whose listing cannot be read is "refused and
the staging folder removed" - which is only possible while it is still staged. The order is
therefore stage, manifest, rename, and the rename is what confirms the install.

*An upgrade is two renames, and one platform may refuse it.* A bundle of the same name already
installed is moved aside into the staging folder rather than deleted, and the staging folder
goes once the new one is in. That alone would not leave a failed swap where it was: the second
rename can fail - an indexer holding a handle, a permission change, a volume that filled up in
between - and the staging folder is removed however the install ended, so the answer would be
"the install failed and deleted the plugin you had". So the swap **puts the displaced copy
back** when the second rename fails, and says both halves in its sentence where even that
fails. `a_failed_swap_leaves_the_bundle_that_was_installed_where_it_was` drives it over an
injected rename, because no platform will fail the second one on request. *ponytail:* on
Windows a directory holding a loaded module may refuse to move at all, so upgrading a plugin a
running Lumit has open can answer `InstallError::Io` where every other platform succeeds; the
honest fix is to shut that bundle's broker down first, which wants an instance table this
package has no reach into.

**Built: the template, and three things it settled that the note left to it.** `template/` is §10,
and §10 is now what was built rather than what was first imagined; the amendments are
docs/12 §3 (the thirteen `kfx` spellings and the one in the research note, D2's kind set,
D3's curve and path split, D5's trait block, D7's depth answer, §3.1's per-struct growth
rule and the `lfx_value` stride exemption, and a §3.8 carrying §5's discovery, §6's install
and §5.4's enable/disable, which §3 had none of), docs/05's crate table (five LFX crates in
the table of crates that exist, and `lumit-lfx` out of the table of crates reserved for
later), and docs/07's settings inventory, which says **Addons** now - D17's one name in two
documents, the other being docs/TODO.md. Two lists nobody had updated since the shared pipe crate are
updated with them, because a reader who has just met §3.8 will look in both: the root
README's tour of the tree now names `template/`, and docs/GUIDE.md's crate table carries
the five LFX crates and its CI table the three LFX jobs.

*The first thing it settled is which examples there are.* §10's bullet list wanted one
working example per extension, and §10's own last paragraph says version 1 offers no
extension table at all. The list was the stale half: a plugin naming `lfx.temporal` in
`required_extensions` is refused before it is instantiated, so an `echo` example would be a
plugin the template shipped and the host would not load. They are one per set of bindings
instead - C, C++ and Rust - and the README says where the other three went. *The second is
where the test lives.* The note names no crate for it, and the byte-identity half would have
sat naturally in `lumit-lfx-abi` - except that crate's manifest says nothing but the header,
the mirror and the layout suite may live there, and its MIT licence is the reason. So all of
it is one file in `lumit-lfx-broker`'s suite, where the validator-over-a-real-bundle cases
already are, for the flat Cargo reason they are. *The third is that the template's own build
is the build the test runs.* A test that compiled the examples with flags of its own would
prove that a compiler can compile them and nothing about the CMakeLists a vendor actually
runs; it drives that file and the staging script, so a broken recipe fails here rather than
in somebody's first afternoon.

Four line-number citations moved with the amendments and were repointed rather than left to
rot: `docs/12:283-285` - the sentence forbidding an in-process path, quoted in six files -
is `docs/12:354-356`, the MIT carve-out is `docs/12:373-375`, and docs/12 §5's two security
bullets are `:645-652` and `:653-656`. `docs/05:75` was already stale by five lines before
this package touched it, and is `docs/05:74`.

The shared pipe crate, the ABI header and the namespace wiring have no dependencies and may
land in any order or in parallel. The fp16 seam was
likewise free-standing and landed early, as the only piece of the fp16 promise that can be
*compiled* in a container - see §4.5.

---

## 13. Left undone, deliberately

- **The two older hosts do not pay the ledger.** LFX's ring is reserved; OFX's 512 MiB and
  audio's are not, which by docs/13 §3's own words is a review reject in code that already
  shipped. Retrofitting them is a follow-on, and the asymmetry is the defect rather than
  LFX's rule.
- **OFX loads and describes switched-off plugins.** §5.4's note. The doc comments assert the
  stronger property; the code does not.
- **A broker that starts and never connects is left running, in the older host.** On the
  handshake arm that gives up before `self.link` is set, the child has not been moved anywhere
  `kill()` can reach it and `Child::drop` on Unix neither kills nor reaps - so a bundle whose
  broker hangs before connecting leaves an orphaned process and a thread blocked in `accept`
  per spawn, and a restart retries. `lumit-lfx` ends the child on that arm; `lumit-ofx`'s
  `start`, which this was copied from, still does not, and the two want reading as one shape.
  Three lines in somebody else's package, so it is named here rather than taken here.
- **An `ACTION` row reaches the broker and stops there.** The frozen entry table is `init`,
  `destroy`, `describe`, `process` and `get_extension`; there is no press hook, so
  `HostMessage::Action` is acknowledged with a `Done` and nothing runs. `Done` rather than
  `Failed` is deliberate - a `Failed` is a strike, and three presses of a button would disable
  an effect that has done nothing wrong - but it means an author may declare a button today
  that draws and does nothing. Either the entry table grows a press hook or `ACTION` joins
  `PATH` and `STRING` as a kind version 1 reserves; both are changes at the header, and both
  are calls for the maintainer.
- **`lfx.gpu-frames`, `lfx.overlay`, `lfx.motion-vectors` and `lfx.audio`** are reserved ids
  and v1 headers, nothing more. The GPU path's Linux story (Vulkan external memory and
  dma-buf, per docs/05:213-215) is unwritten in docs/12 §3.5 as well as here.
- **`lfx.overlay` is what unblocks `LFX_PARAM_PATH`**, and until then shape-warping plugins
  wait.
- **Signed addons proper** wait on a key, a rotation story and a directory. TOFU is what the
  project can back today.
- **Command-palette contributions.** docs/07:2806 says plugins MAY contribute commands and
  docs/12 §3 never mentions it. Not in v1.
- **`shm.rs:51-52` was off by one in both cells, and is no longer.** The comment said the
  512 MiB budget buys fifteen slots at 1080p and three at 4K; `Ring::create`'s own
  arithmetic, three lines below it, buys sixteen and four. Nothing depended on the numbers,
  which is how they drifted - and why a note that repeated them instead of running the
  formula inherited the error. The protocol and ring corrected the comment, pinned it with
  `the_ring_budget_buys_the_slots_the_comment_says`, and generates §3.4's table from the
  rule for the same reason. What is still true of the older host is the ceiling itself: four
  slots at 4K is fewer than a `t ± 5` prefetch's twelve, and OFX has no declared window to
  size a ring from.
- **OFX re-keys frames on a major bump only.** `lumit-ofx/src/schema.rs:145` sets
  `version: plugin.version.0`, and `PluginDescriptor::version` is a `(u32, u32)` pair with no
  patch component at all, so a hosted plugin's minor release is served the cached frames of
  the release before it. LFX mints all three numbers (§4.1); the older host's hole is real,
  and closing it is not LFX's to do.
- **An OFX instance's state can move without its frame key moving.** The blob *is* hashed
  into the bag - `derived.memory`, pushed every resolve - but `plugin_state` itself is only
  ever rewritten by a **press** (`Pressed::memory` through
  `EffectInstance::set_plugin_state`, whose one production caller is `api/track.rs:584`), so
  a plugin whose opaque state moved while it was rendering moves no hash and can be served
  the frame from before it moved. D8 is what closes it for LFX (§4.2) by having
  nothing to keep; closing it for OFX means either reading the plugin's state back after a
  render or deciding the staleness is acceptable and writing that down. *(This entry said
  the blob never reaches the key at all, which is one seam further than the truth; §4.2
  carried the same overstatement and `LfxDef` corrected both.)*
- ~~**`docs/12` needs amending**~~ - **done in the template**, along with `docs/05`'s crate table,
  `docs/07` §15's page name, `docs/TODO.md` and
  `docs/research/research-plugins-interop.md:77`. What is still stale and still not LFX's to
  fix: the QuickJS references in `docs/16` Phase 4 and `docs/05:74`, which sit beside LFX and
  will be read with it, and `lumit-ofx`'s row in `docs/05`'s *reserved for later* table,
  which has described a shipping crate for some time. The template moved `lumit-lfx` out of that
  table and left its neighbour where it was, because a package amends what it built.

---

## 14. Test plan

1. **The ABI cannot drift.** Layout tests in Rust and the same assertions in C via
   `build.rs`, by number, for every struct and every field - including the two arms of
   `lfx_value`'s union, which have fields of their own and a size that does not pin them. A
   reordered field fails both. An offset cannot see a type, so the C half writes every struct
   of plain data from the header's own declarations and the Rust half reads it back through
   the mirror; and an offset cannot see a *signature* either, so the C half defines one real
   instance of each of the ABI's four function-pointer tables and the Rust half **calls** it
   through the mirror's own types -
   `every_sink_call_takes_the_record_the_header_pairs_it_with` and
   `the_entry_plugin_and_host_tables_are_called_as_the_header_declares_them`, which is what
   pins arity, argument order, argument type and answer. Two cases neither reaches: a host
   writing an **oversized `value_stride`** is read correctly by a plugin built against the
   smaller `lfx_value`; and the lowering onto `ParamKind` is held by an exhaustive `match`
   with no `_` arm, so the next variant added to `ParamKind` fails the LFX build rather than
   becoming an unrepresented row (§2.3).
2. **A described plugin becomes the rows it declared**, in declaration order, with the
   `_x`/`_y` spread folded back by the panel and `value_routes` reversing exactly what
   `rows_of` minted - never a suffix rule reversed by guesswork, and read off the built
   schema rather than minted again, so the map the render path wants per frame costs no
   leak. Landed with the describe lowering, in
   `lumit-lfx`'s own suite: `a_described_plugin_becomes_the_rows_it_declared_in_order`,
   `a_point_becomes_two_rows_the_panel_folds_back` (which asks `EffectSchema::pairs` itself
   whether it found the crosshair) and
   `value_routes_reverse_exactly_what_the_rows_were_minted_from`, where an Action takes no
   element and a point takes one rather than two.
3. **The structural refusals are refusals**: no unit, a duplicate `ParamId`, a per-cent of the
   diagonal, a minor or patch number of a thousand or more, a major whose block of a million
   does not fit whole the `u32` the frame key is stored in, a descriptor string past the
   header's ceiling, and more declarations **pushed** than `LFX_MAX_PARAMS` - pushed rather
   than accepted, since a loop of declined declarations grows the report exactly as a loop of
   good ones grows the panel. The graceful ones are report
   lines and the plugin still loads: a `STRING` tag, a `PATH` tag, a tag this version does not
   know at all, an unknown category, a count or a string past one of the header's ceilings, a
   range the panel cannot draw, a whole number outside the bag, a unit a kind does not
   carry, a static flag on a kind this build animates, and a dropdown naming an option it has
   not got. And the pessimistic lowering is asserted rather than
   assumed: **a memset trait block schedules as `HEAVY` / `FULL_FRAME` with a full-frame ROI**,
   and a `NULL` trait pointer does the same. Landed with the describe lowering:
   `a_control_with_no_unit_refuses_the_effect`,
   `a_per_cent_of_the_diagonal_refuses_the_effect`,
   `two_controls_on_one_param_id_refuse_the_effect` and its twin at the lowering,
   `a_row_inside_the_hosts_own_prefix_refuses_the_effect` (the `derived.` prefix the host
   pushes its own values under, swept rather than the one name, with a row that merely mentions
   the word admitted - refused in the sink and again at the lowering, as the duplicate is, and
   for the same reason: a descriptor need not have come through a sink),
   `a_temporal_window_the_host_cannot_honour_is_refused`,
   `more_declarations_than_the_abi_carries_refuse_the_effect`,
   `a_describe_loop_of_declined_declarations_meets_the_same_ceiling` (each of the six roads
   into the sink in turn - `decline` among them, the public one a host holding the plugin
   takes for a declaration it could not read - since the ceiling is over pushes rather than
   over rows),
   `a_descriptors_own_strings_meet_the_same_ceiling`,
   `a_static_row_this_build_animates_is_a_report_line`,
   `a_dropdown_that_names_an_option_it_has_not_got_is_a_report_line`,
   `a_heading_inside_one_declined_for_its_label_is_declined_too` (the stack of open headings
   answering what the plugin declared, where a count would have swallowed the wrong close),
   `a_kind_this_version_reserves_is_a_report_line_and_the_plugin_still_loads` (driven from the
   kind **tag**, since no v1 plugin can reach a `STRING` or a `PATH` through the frozen sink -
   §2.3), `a_declaration_past_the_abis_ceilings_is_a_report_line`,
   `a_declaration_past_the_headers_ceilings_is_refused_off_the_pipe` (the same numbers asked a
   second time where a record comes off a pipe, where they are refusals rather than lines
   because the sink that already asked them ran in the broker),
   `every_ceiling_subject_is_a_word_a_refusal_can_carry` (the subjects swept against `INTERNED`
   as the compared fields already were, since a subject that cannot be read back ends the
   conversation and strikes the plugin),
   `a_range_the_panel_cannot_draw_is_a_report_line`,
   `a_whole_number_outside_the_bag_is_a_report_line`,
   `a_declaration_crosses_the_numbers_it_was_written_with`,
   `the_first_declared_family_is_the_heading_and_the_rest_are_keywords` and
   `a_memset_trait_block_schedules_as_heavy_and_full_frame`; the version refusals came with
   the namespace wiring and are met again at the lowering in
   `a_version_outside_the_injective_range_refuses_the_effect`. Which kind of no a refusal is
   is `LfxRejection::refuses_the_effect`'s own exhaustive match, swept by
   `a_refusal_is_either_a_report_line_or_the_end_of_the_effect` so a variant added later
   cannot inherit an answer.
4. **A folder of bundles becomes exactly the effects it should** - and a rescan registers
   nothing a second time. The folder half landed with discovery, and **not** in process: unlike the
   OFX scan there is no in-process route to test it by, because a bundle's listing is read
   before its code and §11 item 7 puts that read in the broker. So
   `a_folder_of_bundles_becomes_exactly_the_effects_it_should` lives in `lumit-lfx-broker`'s
   own suite, one test deliberately, since the session tables are process-wide: a vendor's
   suite folder, reachable through `LFX_PLUGIN_PATH` - which the test asserts of
   `search_paths()` and then takes the variable back out of the environment, scanning the
   fixture folder directly, because a scan through the variable would also sweep whatever the
   developer happens to have installed - exactly the personalities that can be
   catalogued and nothing else, the bundle that will not load and the one with no build for
   this machine each a line rather than a dialogue (and the second of them **still in the
   listing**, with a label and a vendor, because the listing is read before the payload is
   looked for), a fourth bundle whose **only** plugin is switched off and whose payload is not
   a shared library saying one thing and not two - the skip line naming the plugin and no
   "the module did not load", which is the only evidence from outside the process that the
   module was never opened (§5.4 place 1), the switched-off one absent from both
   session tables and present in the listing with a label a page can draw, every refusal typed
   and the extension one naming what it wanted, a rescan registering nothing and handing back
   the same leaked schema rather than a second copy, and the plugin switched back on
   registering at the next scan. Beside it, in `lumit-lfx`'s own suite, the halves that need
   no second process: `the_search_paths_are_the_standard_ones_plus_the_variable_and_the_addons_folder`,
   `a_bundle_is_found_at_any_depth_and_never_inside_another`,
   `the_payload_is_the_first_architecture_this_platform_ships` (over a bundle holding **all
   seven** of `ALL_ARCH_DIRS`, since three of the six targets name one directory - Linux
   x86-64 among them, which is the machine CI runs - and a fixture built from `arch_dirs()`
   would assert an ordering over a list of one),
   `a_bundle_shipping_both_builds_hands_over_this_machines` (both per-CPU directories laid
   down at once, and the one that comes back read off `std::env::consts::ARCH` rather than off
   the list under test),
   `a_bundle_with_only_another_platforms_build_has_no_payload_here` (each of the six foreign
   names in turn, since LFX owns all seven and a known foreign build is not a fallback),
   `an_unnamed_architecture_falls_through_only_to_a_payload`,
   `the_disable_list_a_broker_is_spawned_with_is_a_share_of_the_running_table` (the handle
   under §5.4 place 1, no broker and no describe - the place-1 property itself is driven
   through a real second process by
   `a_plugin_switched_off_after_the_broker_starts_is_still_switched_off_at_describe` in
   `lumit-lfx-broker`'s own suite, in a test binary of its own because the switched-off list
   is one table per process and a case that ticks a box in it cannot share a binary with the
   scan beside it),
   `a_scan_reads_the_running_list_and_never_writes_to_it` (a scan reads that table and never
   adds to it, so a preference that narrows is the whole of it),
   `two_bundles_declaring_one_id_name_the_one_that_won`,
   `a_plugin_refused_from_the_listing_is_never_offered_described`,
   `a_required_extension_the_host_has_not_got_is_refused_by_name` and
   `version_one_offers_no_extension_at_all`; and in `lumit-project`'s, the third table -
   `the_roster_remembers_a_plugin_no_session_table_holds`,
   `a_refusal_is_remembered_and_a_later_scan_clears_it`,
   `a_missing_or_damaged_roster_reads_as_nothing_ever_seen`,
   `a_roster_past_the_ingress_ceiling_reads_as_nothing_ever_seen` (over a roster that would
   plainly load if the ceiling were gone - one real entry padded inside its own label, with
   the same roster under the ceiling beside it, since a fixture of fields the struct ignores
   would pin nothing but `serde`'s manners; written round `save`, which now holds itself to
   that ceiling, because the case is about a file that arrived some other way),
   `a_roster_too_big_for_its_own_ceiling_is_pruned_rather_than_written` (the writer's half:
   four rows of differing age past the ceiling, and what lands reads back and holds the
   newest) and
   `every_plugin_kind_is_one_word_that_round_trips`. The *registering* still waits for
   `LfxDef`. The in-process ABI edge landed with the in-process host,
   in `lumit-lfx`'s `local` suite, where a bundle is opened and each of its twelve
   personalities answered for: `a_module_declares_the_effects_it_holds`,
   `a_described_plugin_becomes_the_schema_a_builtin_carries` (the rows in declaration order,
   the point spread into the axes `EffectSchema::pairs` folds back, the heading over the run,
   and all three version numbers in the key),
   `a_row_this_build_cannot_draw_is_a_report_line_and_the_plugin_still_loads`,
   `a_plugin_that_refuses_to_describe_is_refused_by_name`,
   `two_controls_on_one_id_refuse_the_effect_at_the_sink`,
   `a_null_trait_block_schedules_as_heavy_and_full_frame`,
   `a_declared_window_reaches_the_schema_as_the_neighbours_it_asks_for`,
   `a_thread_unsafe_declaration_survives_into_the_descriptor`,
   `an_effect_requiring_an_extension_the_host_has_not_got_is_refused_before_create`,
   `an_effect_the_bundle_has_not_got_is_refused_by_name`,
   `the_call_order_is_the_one_the_header_pins`,
   `the_bundles_own_directory_is_what_init_is_handed`,
   `a_file_that_is_not_a_bundle_is_refused_rather_than_called` and
   `a_library_with_no_entry_point_is_refused` - the two shapes of "this file is not a
   plugin", which are a different refusal each. Four faults sit one struct further in,
   where what is wrong is the entry table itself and the bundle behind it is whole:
   `an_entry_shorter_than_the_header_is_refused_before_it_is_called`,
   `a_module_declaring_another_abi_is_refused_before_it_is_called`,
   `an_entry_whose_init_is_null_is_refused_by_that_name` and
   `a_bundle_that_declines_to_load_is_not_then_asked_what_it_holds`, each of which asks the
   same counters of the lying bundle that nothing behind the refusal ran - no `init` behind a
   short prefix, an ABI this host does not speak or an entry with no `init` in it, and no
   `count` and no `deinit` behind any of the four. The first of them reads two bundles: the
   liar, whose prefix lies, and `an_lfx_entry_cut_short`, whose static really is that short,
   which is what makes "the prefix before the struct" a rule a sanitiser can see rather than
   a number a test can read. `an_init_that_answers_anything_but_nought_has_loaded` is the
   same edge read the other way, and is what makes §2.1's "no answer the plugin gives crosses
   as a C `bool`" a test rather than a sentence. The stranger's own numbers are met where they
   arrive rather than after - the count before its array, the size prefix before the struct
   it prefixes, and both of them at every one of the five size-prefixed structs rather than
   at three of them - the entry, the descriptor, the instance table, the trait block and
   every declaration: `a_declaration_shorter_than_the_header_is_a_report_line`,
   `a_descriptor_shorter_than_the_header_is_a_line_against_the_bundle`,
   `an_instance_table_shorter_than_the_header_is_refused_before_it_is_read`,
   `a_string_with_no_end_inside_the_ceiling_loses_its_row`,
   `a_count_past_the_headers_ceiling_is_declined_before_its_array_is_read`,
   `a_descriptor_count_past_the_headers_ceiling_is_declined_before_its_array_is_read` (whose
   arrays are deliberately *shorter* than the counts claim, so a clamp to the ceiling is a
   wild read this test sees rather than a read it covers for) and
   `a_curve_past_the_point_ceiling_is_one_line_naming_the_count_declared`, which is the same
   rule read from the report's side: one line per fault, naming the count the plugin
   declared. The fifth prefix is the trait block, and
   `a_trait_block_shorter_than_the_header_reads_as_the_null_it_is_indistinguishable_from`
   is the `NULL` it is indistinguishable from rather than a refusal. Two answers belong to
   the bundle rather than to a row -
   `a_bundle_past_the_headers_effect_ceiling_is_refused_rather_than_truncated` and
   `two_descriptors_declaring_one_id_are_one_effect_and_a_line`. And the
   values cross as the array the header describes:
   `a_plugin_reads_the_values_the_host_wrote`,
   `a_plugin_walks_the_value_array_by_the_stride_the_host_wrote` (§11 item 15, proved against
   a plugin rather than against the mirror),
   `the_value_array_is_aligned_for_its_elements_at_any_stride` (§11 item 15's other half: the
   stride the host writes is aligned for the element, which is what lets a plugin read one
   the way C spells it),
   `a_frames_origin_is_the_buffers_own_corner_rather_than_the_region_asked_for`,
   `a_value_of_the_wrong_kind_never_reaches_the_plugin`,
   `a_buffer_that_is_not_the_size_the_request_names_never_reaches_the_plugin` (the host's own
   caller held to its word, since the frame carries the request's numbers and a plugin doing
   exactly what it was told would write past a buffer that does not match them),
   `the_elements_the_host_writes_are_the_ones_that_cross`,
   `a_cancelled_frame_is_answered_rather_than_rendered`, and
   `the_dangerous_personalities_are_disarmed_unless_their_variable_is_set`, which is the one
   that keeps the rest of the suite alive.
5. **Out of process, in the broker's own crate** because `CARGO_BIN_EXE_…` exists only
   there: a credential-less broker does not start; a wrong protocol is refused *after* the
   proof, not before; a manifest is read without the module being opened; a descriptor whose
   required-extension list is longer than the manifest declared is `ManifestMismatch`; a
   crash on a frame restarts and the session carries on; a hang trips the deadline and the
   third strike disables; **every host message except `Frames` and `Shutdown` is answered
   exactly once, and an unanswered one is a strike**; the ring file is unlinked once both
   ends hold it; two brokers share no name, neither is a process id, and **no two hosts share
   an endpoint prefix or a broker-executable environment variable** - the three strings §3.1
   parameterises. That last one landed with the shared pipe crate rather than here, in `lumit-ipc`'s own suite
   (`no_two_hosts_share_an_endpoint_prefix_or_a_broker_environment_variable`), because that
   is where the reservation is; each host's suite asserts its own three against it. The
   answered-exactly-once clause has its vocabulary half in the protocol and ring already
   (`every_host_message_but_frames_and_shutdown_expects_a_reply`, over
   `HostMessage::expects_reply`, and asserted over the message *names* against
   `HostMessage::name`'s exhaustive match rather than over a count, so a message left out of
   the list fails rather than passing), so what the broker's suite has left to prove is that
   the broker obeys it rather than which messages it is about.

   Landed with the broker, in `lumit-lfx-broker`'s own suite:
   `a_broker_with_no_credential_does_not_start`,
   `a_broker_that_speaks_another_protocol_is_refused_after_the_proof`,
   `a_manifest_is_read_without_the_module_being_opened` (whose payload is a line of text, so
   a broker that opened the module to answer a listing could not have answered one),
   `a_listing_that_disagrees_with_the_code_refuses_that_plugin_by_name`,
   `a_required_extension_list_that_is_not_the_manifests_is_a_manifest_mismatch`,
   `a_crash_on_a_frame_restarts_the_broker_and_the_session_carries_on`,
   `a_hang_trips_the_deadline_and_the_third_strike_disables_the_plugin`,
   `every_host_message_but_frames_and_shutdown_is_answered_exactly_once`,
   `the_ring_file_is_unlinked_once_both_ends_hold_it`,
   `two_brokers_do_not_share_a_name_and_neither_name_is_a_process_id`,
   `a_handle_the_host_never_minted_is_answered_rather_than_followed`,
   `a_plugin_that_will_not_stop_talking_does_not_fill_the_host`,
   `a_switched_off_plugin_is_never_described_in_the_broker`,
   `a_value_the_host_wrote_reaches_the_plugin_through_the_broker`,
   `a_declared_window_sizes_the_ring_the_second_process_maps`,
   `a_bundle_with_no_listing_is_refused_with_a_sentence`,
   `a_bundle_with_nothing_switched_off_describes_everything_that_can_be` - which names each of
   the three that cannot be catalogued with its own reason, rather than asserting the refusal
   list is empty - and `a_frame_that_is_not_the_one_asked_for_is_refused_rather_than_served`,
   which is the slot and the bounds held to what the job asked for. Four more arrived with the
   second pass over the broker, three of them about ceilings the note claims and nothing measured:
   `an_answer_to_a_question_nobody_asked_is_a_strike_rather_than_a_reset`, which is the
   answered-exactly-once clause read as "answered with the answer it admits", since a count
   cannot tell a reply from the reply to the question before last;
   `a_ring_the_ledger_narrowed_is_not_made_again_for_every_frame`, which holds a regrow to the
   plan the ring was made from rather than to the slots the governor granted it;
   `a_shipment_wider_than_the_ring_is_refused_before_a_slot_is_written`, which is §3.4's
   prefetch ceiling counted in slots; and
   `a_bundle_may_not_hold_more_instances_than_a_replay_can_carry`, which is
   `MAX_LIVE_INSTANCES`. The last two are the host's own refusals, so each asserts that nothing
   was struck for them. Three more arrived with the third review, each a rule the code stated
   and did not keep: `a_ring_that_cannot_be_made_again_reaches_the_watchdog`, which takes the
   ring's directory away underneath a live broker and asserts the frames after it are counted
   rather than answered `NoRing` for ever;
   `an_answer_to_an_open_is_held_to_what_the_open_admits`, which is the one exchange outside
   `action` reading the same `HostMessage::answers` the rest of them do; and
   `a_narrowed_rings_line_is_filed_again_by_the_broker_that_replaces_it`, which is the report
   line surviving the restart that replaces the report it was in. The two that were already
   here grew the half they were missing:
   `a_frame_that_is_not_the_one_asked_for_is_refused_rather_than_served` drives three
   consecutive frames of each misanswer and reads the strike count, because refusing a frame
   and counting it are two properties, and
   `a_shipment_wider_than_the_ring_is_refused_before_a_slot_is_written` drives its four-picture
   shipment at both depths, because a ceiling counted in slots that halving a slot does not
   lift is a claim about two depths. Beside them, in `lumit-lfx`'s own suite,
   `every_answered_message_names_the_answer_it_admits` is the vocabulary half of the first:
   every message that is waited on names an answer, every message that is not names none, and
   no name given is `Failed` or one of the two the exchange loop consumes without returning. The forged-handle test
   walks both halves of §3.5: the host answers `NoSuchInstance` for `press`, `set_values` and
   `process` without a strike, and the broker answers `Failed` for `Action`, `Values` and
   `Destroy` through the one seam that can still reach it. Beside them, in
   `lumit-lfx`'s own suite, the three the second process cannot reach:
   `this_hosts_three_strings_are_the_ones_reserved_for_it` and
   `the_endpoint_name_is_this_hosts_own` are this host's half of the shared pipe crate's reservation;
   `a_message_with_no_reply_is_never_waited_on` is the supervisor's own reading of
   `expects_reply`;
   `a_declared_window_the_host_cannot_honour_is_refused_before_it_is_narrowed` and
   `a_plugin_the_listing_never_mentioned_is_a_manifest_mismatch` drive `admit` directly, and
   `a_narrowed_ring_says_which_kind_of_no_it_got` is §3.4's budget-shaped no told from its
   ceiling-shaped twin.

   **One clause cannot be reached through a real bundle in version 1, and the reason is
   itself the design.** A descriptor whose required list is *longer* than the manifest's
   would have to name an extension the host offers, and version 1 offers none - a plugin
   requiring one is refused before `create`, so it never reaches a describe to be compared at
   all. The broker's test therefore puts the difference on the manifest's side; the named
   direction is asked at the lowering, in
   `a_required_extension_the_manifest_did_not_declare_is_a_mismatch`, and becomes reachable
   end to end the day `lfx.temporal` has a table behind it.
6. **Both depths, end to end.** An fp16 frame crosses the ring and comes back unchanged; an
   fp32 frame does; a slot written at one depth and read at the other is refused. The same
   sentence about the *plugin* rather than the ring landed with the in-process host, in
   `both_depths_cross_the_boundary_unconverted`, which drives one instance at fp32 and then
   at fp16, reads the depths back off the plugin's own probe, compares the halves by bits,
   and asks the host for the mismatched pair the depth rule forbids. The ring
   is charged to the ledger and given back when it is dropped, and a ledger with no room
   buys three slots rather than none. Landed with the protocol and ring, in `ipc::ring`'s own suite and under
   those five names - the fifth says *dropped* rather than *when the broker dies*, because
   there is no broker in that package and a name that claims more than its body does is how
   a suite comes to look complete. The second **process** landed with the broker, in
   `both_depths_cross_the_ring_between_two_processes_unchanged`;
   `a_second_mapping_of_the_same_ring_reads_what_the_first_wrote` is the same claim in one
   process, which is exactly what the broker's side does, and pays for nothing. Beside them: a slot whose
   bytes changed under the header is `Corrupt` rather than a picture; a header that lies
   about its own size is refused rather than believed, which is the half the hash cannot
   see; the sixty-four bytes are asserted by number from the side that writes them; a path
   that already exists is refused rather than truncated and the ledger gets its bytes back
   on that road too; a ring goes on working after its name is removed and no second end can
   join it once it has; and the generated table is checked against the rule that generated
   it.
7. **Identity means identity.** A failed, hung, crashed or switched-off plugin returns the
   input **byte for byte** and files a sentence - the half the in-process host pins is a plugin that *succeeds*
   and writes nothing, `a_plugin_that_writes_nothing_leaves_the_output_as_it_found_it`, which
   compares the halves by bits and not by value, since the input copied back would go through
   the depth boundary and change very slightly. The same plugin crossing two processes and a
   ring is `a_plugin_that_writes_nothing_renders_identity_through_the_broker`, where "as it
   found it" is whatever the broker seeded the output buffer with and a zero-filled one would
   be a black frame returned as `Ok`;
   `a_region_the_plugin_never_wrote_comes_back_as_the_input` is the same seam read from the ROI
   side, the fixture having no ROI-honouring personality and `lfx-validator`'s ROI honesty check
   being the validator's. A plugin that writes half the output and then
   answers `LFX_STATUS_FAILED` leaves the caller's buffer half written and `LocalInstance::process`
   does nothing to restore it: **restoring it is the caller's**, and with `LfxDef` the caller is
   `LfxDef`, whose `a_failed_frame_leaves_the_picture_exactly_as_it_found_it` compares the
   halves by bits rather than by value for the same reason the in-process host's does. Two more roads out of a
   render end the same way and are pinned beside it: a frame that is not the one asked for
   (`a_frame_that_is_not_the_one_asked_for_is_a_badge_rather_than_a_picture`, the depth and the
   count held to what was sent - **both**, since a short answer at the asked-for depth cannot
   reach the depth arm, so the case drives a `WrongDepth` personality that answers the right
   count at the other depth, once each way, and compares the sentence whole rather than
   looking for digits in it) and a failed **fp16** frame, which is identity at that depth
   and still answers `true`, because declining would hand the same frame to the same plugin a
   second time (`a_failed_fp16_frame_is_identity_rather_than_a_second_attempt`, which asks the
   same of a frame of no area and of a buffer shorter than its size, the two roads out that
   are not a failure at all). The badge is
   taken on read - `a_badge_is_taken_on_read` - and the next frame that works clears it, which
   is a case of its own and deliberately not an assertion in that one:
   `a_good_frame_clears_the_reason_the_frame_before_it_filed` files a failure, renders a good
   frame **without reading in between**, and asks the definition of another plugin what went
   wrong, which is the shape a render worker really has. A case that reads twice before it
   renders has drained the slot with its second read and would pass over a `render` that
   cleared nothing at all.
   And a *switched-off* one files
   `lumit-ipc`'s shared `DISABLED_REASON` - landed with discovery, in
   `a_switched_off_plugin_renders_identity_and_files_the_shared_reason` and
   `a_plugin_switched_off_mid_session_stops_rendering_now`, which assert against a counting
   stub that the plugin's code is not merely ignored but not reached, and read through the
   whole definition with `LfxDef` in
   `a_switched_off_plugin_renders_identity_through_the_definition` and
   `a_switched_off_plugin_is_never_opened_at_all`. **Those two prove the instance seam, not
   the gate**, because `LfxDef::lease` reads the list first and a tick that has already landed
   never reaches `Gated::process`; the gate's own read owns the tick that lands *after* it,
   which is
   `a_plugin_switched_off_while_the_frame_is_in_flight_is_caught_by_the_gate` - without it
   `LfxDef::hosted` could drop the gate with the whole suite still green. The reason is one
   constant, which `badge_of` tests
   against and
   `engine_labels_test.dart` holds as a constant the way it holds `BADGE_REASONS`, so the
   badge reads "switched off" rather than "failed" (§4.3); the badge is taken on read so a
   stale reason cannot mark a later frame; the next frame that works clears it.
8. **The namespace reaches both walks.** An `Lfx` instance resolves into the arena and
   contributes its temporal window; a retimer's eleven sampled frames are the ones the layer's
   frame key depends on, so a change to any of them retires the cached frame and a change
   outside the window retires nothing. An `Lfx` effect carries no `plugin_state` and its key is
   complete without one - pinned as a fact about the *walk*, an instance carrying a blob keying
   identically to the twin without one, rather than as a fact about a fixture. And a **seeded**
   `Lfx` effect's key moves with the layer's local time as a seeded built-in's does, which is
   the same predicate read at the walk's other gate (§2.4). And the **graph's** demand is the
   layer's: `input_times`, the one table `CompGraph::time_demands` walks, is asked of the same
   registered retimer and answers its own time plus the ten frames it declared, so the
   lowering, the planner and the frame key cannot disagree about which frames a hosted retimer
   is made from. Landed with the namespace wiring,
   across `lumit-core`'s run-time catalogue suite and `lumit-eval`'s key walk - the seeded half
   with the describe lowering, in `a_seeded_lfx_effects_key_moves_with_local_time`, and the graph walk when the
   review found it unpinned; the same suite
   pins that the predicate is a *gate*, since an audio plugin with a temporal declaration
   reaches neither walk and demands no frame but its own.
9. **Concurrency.** Frames arrive out of order and the picture is unchanged; two instances
   render different frames at once; a thread-unsafe plugin never sees two processes; the
   pool collapses under Severe pressure and grows back; two exports of the same project are
   bit-identical whatever the pool did. The two that needed no pool landed with the in-process host in
   `two_instances_render_at_once_and_neither_is_re_entered`, where
   the later frame is dispatched first and the fixture's own barrier makes the overlap
   deliberate - §11 item 12's point, that a stress test asserting an absence proves nothing
   unless the fixture records the overlap it is about. The other four landed with the instance pool, in
   `lumit-lfx`'s own suite and under the clause's own words:
   `frames_arrive_out_of_order_and_the_picture_is_unchanged` (the run rendered once in order
   and once from eight threads in a jumbled order, compared by bits, the driver counting how
   many frames were ever inside it at once so a run that never overlapped fails rather than
   passes), `a_thread_unsafe_plugin_never_sees_two_processes` (**three** runs of the same four
   frames through one driver - a bundle whose two plugins both declare it, one where neither
   does, and the **mixed** bundle, which is the only shape that tells bundle-wide from
   per-plugin; the runs that record a single caller mean something because the loose one
   records two), `the_pool_collapses_under_severe_pressure_and_grows_back` (the surplus
   instance **closed** rather than merely uncounted, asserted against the driver's own record,
   and the growth back wanting a run of frames behind it because a growth that did not raise
   throughput is the last one) and
   `two_exports_of_the_same_project_are_bit_identical_whatever_the_pool_did` (one export
   through a pool a floor-sized ring pins to one instance, the other through a pool free to
   grow and driven out of order from several threads). The recorder's deadline is the case's
   own: a run asserted to overlap waits patiently and never spends the wait, because the
   company is coming, while a run asserted not to overlap waits briefly and gains nothing by
   waiting longer - the same fixture, and neither half failed by a loaded machine. Beside them,
   the clauses §4.4 has that
   this item does not: `a_frame_whose_declared_scratch_the_ledger_will_not_grant_is_not_dispatched`
   (§8's replacement for `memoryAlloc` - no frame reaches the plugin, no instance is opened for
   one, the picture is identity byte for byte, and the same declaration against a ledger that
   can afford it renders and gives its bytes back),
   `a_row_that_has_left_the_pool_does_not_hold_its_instance_for_ever` (the least recently
   leased rows over `MAX_POOL_ROWS`, and the oldest going first) beside
   `an_evicted_row_keeps_the_window_its_last_render_asked_for` (what eviction closes is the
   instances; the offsets the frame key is computed over stay, or the row comes back rendering
   a different picture rather than paying a round trip),
   `the_pool_is_capped_at_the_worker_count_and_at_the_ring_the_ledger_granted` (the rule rather
   than a number, since the worker count is the machine's),
   `a_temporal_plugins_ceiling_falls_as_its_declared_window_widens` (a frame ships `hi − lo + 2`
   slots, so a twelve-slot ring is one frame of a `t ± 5` plugin and not six),
   `the_ceiling_follows_the_ring_the_broker_has_now` (the count is published rather than copied,
   because `fit` replaces the ring mid-session),
   `a_growth_that_did_not_raise_throughput_is_the_last_one`,
   `a_growth_wants_a_run_of_frames_behind_it_rather_than_one` and
   `an_idle_gap_is_not_a_slow_epoch` (all three over instants the case
   feeds, so what is measured is the rule and not the machine it ran on),
   `a_thread_unsafe_declaration_pins_the_pool_to_one_behind_the_bundles_lock`,
   `a_bundle_one_plugin_declared_thread_unsafe_is_pinned_whole` and
   `a_declaration_the_plugin_never_made_does_not_pin_the_pool`.
10. **Install is hostile-input testing.** A `..` entry does not escape the staging folder;
    an absolute name is refused; an archive that lies about its uncompressed size is refused
    both ways; a zip bomb is refused before it is written; an interrupted unpack leaves
    nothing that looks installed **and nothing a scan can find**; a second pack under a new
    key is refused by name, and so is a second pack under **no** key; a pack does not replace
    a bundle another addon installed; a failed swap leaves the old copy where it was; an
    install that cannot write its key down installs nothing; **a damaged trust store refuses
    rather than re-trusting**; an unsigned pack installs and says so. Landed with install and trust, in two
    halves. Everything a temporary directory is enough for is in `lumit-lfx`'s own suite -
    `an_entry_that_climbs_out_of_the_staging_folder_is_refused`,
    `an_absolute_entry_name_is_refused`, `the_names_a_bundle_is_made_of_are_plain` (the
    same rule from the other side, so it refuses the attack rather than the format),
    `an_entry_nested_deeper_than_the_budget_admits_is_refused` (the ceiling being the
    budget's own `depth`, charged per entry, rather than a constant beside it),
    `an_entry_that_lies_about_its_length_is_refused_both_ways`,
    `an_entry_claiming_more_than_one_file_may_be_is_refused_on_its_own_ceiling` and
    `a_run_of_entries_stops_at_the_archives_byte_ceiling` (all three over a reader that
    claims one size and produces another, which is not something a real archive library will
    do on request), `a_zip_bomb_is_refused_before_a_byte_is_written` (which is a claim about
    the *order*, so it is made over a real archive whose declared length is rewritten to two
    gigabytes and asserts that the entries before it were written and this one was not),
    `the_manifests_ceiling_admits_a_declaration_for_every_entry_the_budget_admits` (the two
    ceilings being one ceiling seen twice),
    `the_signature_is_checked_before_the_manifest_is_parsed` (over a pack whose
    manifest is not JSON at all, so the refusal can only be the signature's),
    `a_signature_this_build_cannot_read_is_not_an_unsigned_pack` (an entry that is *there*
    and will not read is a refusal, never an absence, because read as an absence it is a
    downgrade with a calm line on it),
    `a_signature_that_does_not_check_is_a_refusal_rather_than_a_fallback`,
    `a_signature_of_the_wrong_length_is_refused_as_unreadable` and its longer twin
    `a_signature_longer_than_one_is_refused_as_unreadable_too`,
    `an_entry_the_signed_manifest_does_not_declare_is_refused`,
    `an_entry_whose_bytes_are_not_the_declared_ones_is_refused`,
    `a_declared_entry_the_pack_does_not_carry_is_refused`,
    `a_manifest_declaring_a_name_that_is_not_a_plain_path_is_refused` (the manifest's own
    keys being the one string here quoted out of a stranger's structured text rather than
    out of the archive's names),
    `a_pack_that_is_not_one_bundle_is_refused_by_what_it_is_missing` (each of the four
    layout faults in turn), `a_file_that_is_not_a_pack_is_refused_calmly`,
    `a_pack_of_another_format_is_refused_by_the_format_it_declares`,
    `a_second_pack_under_a_new_key_is_refused_by_name`,
    `an_unsigned_pack_for_an_addon_installed_under_a_key_is_refused`,
    `a_damaged_trust_store_refuses_the_install_rather_than_re_trusting`,
    `a_pack_that_did_not_install_records_no_key`,
    `a_failed_swap_leaves_the_bundle_that_was_installed_where_it_was` (over an injected
    rename, because no platform will fail the second one on request) and
    `an_interrupted_unpack_leaves_nothing_installed_and_nothing_a_scan_can_find`; with the
    store's own cases beside them in the `trust` suite -
    `an_absent_trust_store_is_a_first_use`,
    `the_store_files_a_key_under_the_kind_and_the_identifier`,
    `a_later_pack_under_another_key_is_refused_by_name`,
    `a_later_unsigned_pack_for_a_signed_addon_is_refused_by_name`,
    `a_pack_may_not_replace_a_bundle_another_addon_installed`,
    `an_unsigned_install_writes_down_no_key_and_still_owns_its_bundle`,
    `a_damaged_trust_store_refuses_rather_than_re_trusting`,
    `a_trust_store_past_the_ceiling_refuses_rather_than_re_trusting`,
    `a_recorded_key_is_never_replaced_by_a_later_one` and
    `a_fingerprint_is_the_digest_of_the_key_the_pack_carried`. And everything that has to
    reach step 6 is in `lumit-lfx-broker`'s, because the broker executable is
    `CARGO_BIN_EXE_lumit-lfx-broker` and exists only inside the package that owns it:
    `a_signed_pack_installs_and_remembers_the_key_it_arrived_under`,
    `an_unsigned_pack_installs_and_says_so`,
    `a_second_pack_under_a_new_key_is_refused_and_the_installed_one_stays` (which also
    drives the ordinary upgrade under the same key),
    `a_later_unsigned_pack_for_a_signed_addon_is_refused_and_the_installed_one_stays`,
    `a_pack_may_not_replace_a_bundle_another_addon_installed`,
    `an_install_that_cannot_record_its_key_installs_nothing` and
    `a_bundle_whose_listing_the_broker_cannot_read_is_refused_and_the_staging_folder_goes`.
    The staging folder's own property is one line in `lumit-ipc`, where both paths come
    from: `the_staging_folder_is_not_inside_the_folder_every_host_searches`.
11. **The page's own record.** A switched-off addon is listed and switches back on; a plugin
    that never registered is named with its reason; a refused one names the extension it
    wanted; a disabled one is absent from the catalogue listing; the scan report is shown
    rather than discarded; the metrics test measures every band against the drawing's own
    number; every `ADDON_STATE`, `ADDON_KIND` and `BADGE_REASON` has a sentence.
12. **The catalogue entry itself**, which is §4.2 read from the outside. The resolved bag
    becomes the dense value array in declaration order -
    `a_resolved_bag_becomes_the_dense_value_array_the_plugin_reads`, where the point is one
    element and the button is none - and a row the bag carries nothing for keeps the plugin's
    own declared number (`a_row_the_bag_has_no_value_for_keeps_the_plugins_own_default`, which
    is every `FILE` row until the render pass). Both depths reach the plugin as themselves
    (`both_depths_reach_the_plugin_at_the_depth_they_arrived_in`, the depths read back off
    what crossed rather than off what was asked for). The neighbours the stack decoded cross
    beside the picture at **their own comp times**, and one of another size is left out rather
    than sent short (`the_neighbours_cross_at_their_own_comp_times`, which also pins that the
    time the plugin is told is the number `derived.frame` carries and not the layer's seconds -
    the **layer's** local time expressed in the comp's frames, `(cx.lt * fps).round()`,
    which is what `ResolveCx::lt` is and differs from the comp's own frame for any layer with
    a start offset or a time stretch. The test's name says *comp times* because the unit is
    the comp's; the quantity is the layer's). One row is one live instance however many frames
    it renders **one at a time**, a bag that did not move costs no round trip, and another row
    is another instance (`one_row_is_one_live_instance_however_many_frames_it_renders`); frames
    of one row arriving at *once* are the instance pool, and what it opens for them is bounded by
    §4.4 rather than by the arrivals. Telling an instance its values and asking it for a frame
    are **one turn**, so two frames of one row arriving at once are each painted with their own
    numbers rather than one of them with the other's
    (`two_frames_of_one_row_are_each_painted_with_their_own_values`, where the second frame is
    not started until the first is inside the driver, under a policy that pins the pool to one
    so that the two frames are two frames of one instance - and asserting that one open, since
    an absence a second instance makes unreachable is §11 item 12's own trap). That turn was a
    lock of the instance's own until the instance pool and is the **lease** now: a leased instance is not
    visible to any other frame, so it cannot be told anything in the middle of somebody else's
    turn. Which
    declarations carry a value is one walk's answer and not
    two (`every_declaration_that_carries_a_value_has_a_default_to_carry`, sweeping every kind
    the frozen sink admits against `schema::carriage`'s own answer, since a disagreement would
    shift every element after it). A retimer's answer is read
    off its last render and clamped rather than refused later
    (`a_retimers_sampled_frames_are_what_the_definition_answers`, `None` before it has said
    anything and `None` again when it asks for nothing but the frame in hand), and a frame that
    did not come back says nothing about the frames either side
    (`a_failed_frame_does_not_narrow_the_window_the_good_ones_set` - a failure answers no
    offsets, and taking that for the window would narrow the frame key on a bad frame and
    retire every cached frame the good ones made). The rows the
    plugin hid at describe are the rows the panel skips
    (`the_rows_the_plugin_hid_at_describe_are_the_rows_the_panel_skips`). A button reaches the
    plugin by the row it was declared under and a name that is not one is refused
    (`a_button_press_reaches_the_plugin_and_writes_no_rows`), starting from the document's own
    rows rather than from a bag it has not got
    (`a_press_hands_the_plugin_the_rows_the_document_holds`) - and reading them as the bag
    would have carried them rather than as something near enough, which is a rule with exactly
    one number in it: `a_seed_past_the_signed_range_is_one_number_on_both_roads` stores a seed
    above `i32::MAX`, as roughly half of all fresh instances carry, and holds the press's
    `ParamValue::Seed` against the one a frame of the same row sends. A clamp on the press road
    against the resolve walk's own reinterpretation would write two different seeds into one
    live instance, for a reason the document does not record. Nothing but the frame is derived -
    `an_lfx_effect_derives_the_frame_and_no_state_of_its_own`, which is D8 asserted rather
    than assumed. And a definition does not register itself
    (`a_leaked_definition_does_not_register_itself`): the pass and the entry arrive together or
    the join nothing checks at compile time is made out of the composition root's sight
    (§11 item 2). All of those drive the two seams against a fake, which is what lets them be
    about the marshalling; the **wiring** `LfxDef::hosted` does - the driver, the gate over it
    and the broker behind both - is proved over a real bundle in a real second process, and
    so lives in `lumit-lfx-broker`'s own suite for the same flat Cargo reason the rest of item
    5 does: `a_described_plugin_renders_through_the_definition`, where the Full personality
    multiplies by the number the bag carried so the value is visible in the answer rather than
    merely accepted, and `a_frame_that_never_came_back_leaves_the_definitions_picture_alone`,
    where a plugin that hangs costs the caller's buffer nothing at all.
13. **The commercial pass, by hand**, in ofx-host.md's shape and recorded in the pull
    request that ran it: install the template's example bundle by every route each platform
    offers, check it appears under the family it declared rather than a plugin heading,
    switch it off mid-session and check the layer keeps its picture with a calm badge, open
    the project on a machine without it and check every parameter and keyframe survives the
    round trip.
14. **The template is the ABI, and it works.** Landed with the template, in
    `lumit-lfx-broker`'s suite for the reason item 5 lives there:
    `the_templates_copy_of_the_abi_is_this_workspaces_own` holds `template/include/lfx.h`,
    `template/rust/lfx-sys/src/abi.rs` and the MIT licence beside them byte for byte
    against this workspace's own, because a copy is only as honest as the test that reads
    both - and a header that drifted would leave a vendor compiling against an ABI the host
    had stopped speaking, with every size prefix the *older* number, which is exactly the
    case the growth rule makes legal.
    `every_example_declares_the_abi_version_this_workspace_speaks` reads each example's
    listing, since a bundle naming another version is refused with the module still shut
    and a template whose examples are refused on sight is the worst way to be wrong. And
    `every_example_builds_and_passes_every_suite` compiles all three with this machine's own
    compilers - through the template's **own** CMakeLists and staging script, so a recipe a
    vendor runs is the recipe that is tested - lays each out as a bundle and drives it
    through the ten suites, asserting no refusal and that the one effect each declares is
    the one effect that describes. A toolchain that is not on the machine is the only skip,
    and it is named. The `lfx-template` job runs it on all three platforms and downloads
    nothing.
