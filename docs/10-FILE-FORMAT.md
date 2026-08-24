# Lumit project format

**Status: canonical.** Serialisation of the model in [03-DATA-MODEL.md](03-DATA-MODEL.md),
per decision K-040 (hybrid container) and K-024 (non-destructive always).

Design goals, in priority order: **never lose work** → **portable between machines** (K-065)
→ **human-inspectable** → fast. Speed is engineered around the format (caches, lazy thumbs),
never by making the document opaque.

---

## 1. The `.lum` file

A `.lum` file is a ZIP archive (deflate). Contents:

```
myproject.lum
├── manifest.json          # tiny: format + version info, read first
├── project.json           # the entire document model
└── thumbs/                # planned, not yet written (see below)
    ├── comp-<uuid>.webp
    └── item-<uuid>.webp
```

Rules:
- `manifest.json` MUST be the first entry in the archive and MUST parse standalone:
  `{ "format": "lumit-project", "schema_version": "…", "written_by": "lumit x.y.z",
  "min_reader": "…" }`. A reader newer than `schema_version` migrates; older than
  `min_reader` refuses with a clear message; otherwise it loads and preserves unknowns.
  The current schema is **`0.2.0`**. The one migration in the chain, `0.1.0` → `0.2.0`
  (K-249), moves a Footage layer's own retime segment store onto the layer as the Retime
  **property**, and lifts the frame-interpolation policy out beside it; `min_reader` stays
  `0.1.0`, because the fields a `0.1.0` reader does not know about are preserved rather
  than fatal (§1.1).
- `project.json` is pretty-printed with stable key order and stable array order, so two
  saves of the same document are byte-identical and version-control diffs are meaningful.
- Thumbnails are disposable previews for the Project panel and file browsers; a reader MUST
  tolerate their absence. **Not yet written** - v1 saves only `manifest.json` and
  `project.json`; the `thumbs/` folder is planned ([TODO.md](TODO.md)).
- Nothing else goes in the container. Media is never embedded; caches never ride along.

### 1.1 project.json conventions

- Times: rational pairs `[num, den]` — never floats ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)).
- Colours: linear-light float arrays `[r, g, b, a]`.
- Ids: UUIDv7 strings; every cross-reference is an id.
- Enums: serialised by serde's default — a unit variant is its PascalCase name
  (`"channel": "Alpha"`, `"blend": "Screen"`); a data-carrying variant is externally tagged
  (`{ "Footage": { … } }`). Variants are additive, so old readers keep unknown ones via the
  preservation rule below.
- **The interface arrangement rides along, opaquely** (K-245): `ui_state` is the frontend's
  own JSON — the panel arrangement, which comps were open, the playhead, the selection — and
  the engine never reads inside it. Absent by default, so a project nobody has arranged gains
  no line for it. It is a *hint*: a reader that already has its own record of this project
  prefers that, and one that cannot make sense of what is here ignores it. What it may **not**
  contain is anything machine-specific — no pixel window placements, no paths, no usernames
  (§2's rule, which K-245 narrowed rather than lifted: panel names, tab indices and fractional
  shares mean the same thing on any machine, and that is all this field is for).
- **Rendering settings that change the picture travel with the file.** `anti_aliasing`
  (K-274) is the first of them: the project's coverage-sample count, written as its variant
  name (`"anti_aliasing": "x4"`). Absent — as it is in any `.lum` written before the field
  existed — it reads as the default rather than failing, which is the serde-default rule
  every additive field here follows.
- **Unknown-field preservation is mandatory**: a reader keeps any keys it does not
  understand and writes them back out. This is what lets shared projects and newer/older
  Lumit versions coexist (K-065) and lets Placeholder effects round-trip
  ([11-AE-IMPORT.md](11-AE-IMPORT.md)).

## 2. Media references and relinking

Per `MediaRef` in [03-DATA-MODEL.md](03-DATA-MODEL.md) §3, a saved reference carries a
**project-relative path** (rebased against the project's folder on every save; forward
slashes, so a save from any OS resolves on any other) and a **fingerprint**
(size + mtime + head/tail hash, stamped at save time). The file's absolute location is
**session-state only** (K-173): it is held in memory while the app runs and is never
serialized — an absolute path embeds the local username, which this section has always
promised the file never contains. Projects saved before K-173 may still carry one; it is
read and honoured as a fallback, and disappears on their next save. On open:

1. Try relative path → 2. a legacy file's absolute path, if present → 3. fingerprint search
   in user-configured search roots and the project's folder tree → 3b. **by file name**
   under the project's own folder tree, for whatever the first three did not find → 4. mark
   **missing** (placeholder slate, never a blocking error), offer the relink dialogue.

Step 3b is the weakest match and so it runs last, and only over what is still lost: it is
what answers a project that arrived beside its media but carrying another machine's paths,
which is every After Effects import on a second computer, where there are no fingerprints
yet because nothing has been saved (K-438). The tree is walked **once** for all the missing
items rather than once each, and where two files share a name the first in walk order
answers for both — the fingerprint search above it is what tells those apart once a project
has been saved.

Steps 1–3b are wired (`resolve_all_media`, run before anything probes); step 4's dialogue is
future work — today missing files are named in a notice and keep their reference untouched,
so a later relink loses nothing.

Relinking one file automatically relinks siblings that moved the same way (K-438). The
mapping is the **longest** rewrite the move supports: whatever the old path and the new one
share at the end did not move, and the prefix in front of it did, so relinking one clip four
folders deep inside a footage tree brings back every other lost item under that same root —
in its own subfolder, not only in the folder the user happened to pick. A sibling is
repointed only when it is currently broken *and* the file the rewrite predicts exists; a
sibling the rewrite does not reach falls back to a file of its name beside the picked one.

**Collect for sharing**: an explicit command copies the project plus all referenced media
into one folder, rewriting references relative — the mechanism behind community project
sharing (K-065). Nothing machine-specific is ever written into `project.json` (no cache
paths, no local usernames, no window placements in monitor pixels); per-machine state lives in
app settings. **Amended by K-245:** the *panel arrangement* is not machine-specific — panel
names, tab indices and fractional shares read the same anywhere — so it travels in `ui_state`
(§1.1) precisely so a shared project opens the way its author left it. The machine-local
workspace store still keeps its own copy per project path, and that copy is preferred on open;
the file's is what answers on a machine that has never seen the project.

## 3. The sidecar cache folder

All derived data lives outside the project. **v1 status:** the rendered-frame cache, the
media index and the camera-solve sidecar are built; `proxies/`, `peaks/`, and `flow/` are
planned ([TODO.md](TODO.md)). What exists today:

```
<global cache root>/
├── frames/<project-uuid>/         # rendered frame cache (06 §5.4), the default location
│   ├── frames/                    #   LZ4 .kfr files, sharded by the first two hex chars
│   ├── index.bin                  #   the index snapshot: hash, size, cost, last use, quality
│   └── index.log                  #   changes since that snapshot, replayed at open
├── media-index/       # frame indexes for exact long-GOP seeking, shared across projects
├── track/             # camera solves (K-417), shared across projects — see below
└── <project-uuid>/journal/ops.jsonl # the crash-recovery journal (§4)

<project>.lum-cache/   # the same frame cache, when the user asks for it beside the project
├── frames/
├── index.bin
└── index.log
```

The intended full per-project layout (`<cache root>/<project-uuid>/` with `disk-cache/`,
`proxies/`, `peaks/`, `flow/`, `index/`) is the design direction; audio peaks are currently
computed on demand rather than stored.

**Where the frame cache sits is the user's choice (K-214, docs/07 §15):** under the global
root keyed by the document's uuid (the default), in a `<project>.lum-cache/` sidecar beside the
project file, or under a folder the user picks. The global root is the platform's own cache
directory, resolved by `directories::ProjectDirs` exactly as the journal and media index resolve
theirs, so one Lumit folder serves all three: `%LOCALAPPDATA%\Lumit\Lumit\cache` on Windows
(**local**, never roaming — a cache this size must not follow a domain profile over the
network), `~/Library/Caches/dev.Lumit.Lumit` on macOS, and `$XDG_CACHE_HOME/lumit` (default
`~/.cache/lumit`) on Linux. The cache directory, not the temp directory: these survive a
reboot, and may be reclaimed by the operating system under disk pressure — which is correct for
a folder deletable at any time. The sidecar cannot be the default because it
needs the project to *have* a file, and a project caches from the moment it is created — the
document uuid is inside the `.lum` and survives every save, so the global-root folder still
finds its frames after a save and a reopen.

The choice is application-wide by default and **may be made per project** (K-215), in which case
it is a field on the document (`cache_location`) and therefore inside `project.json`: it travels
with a copy of the project and survives being opened on another machine, which a setting in one
machine's settings file cannot. Absent when the project follows the application, so a project
that has never been given a place of its own gains no line for it and an older build reads the
file unchanged (§1.1's forward-compatibility rule). Nothing is moved when the choice changes —
the frames in the old folder simply stop being addressed.

**`track/` — the camera-solve sidecar (K-417).** One file per analysis, named
`<32-byte blake3>.ltrk`, where the hash is over the media's fingerprint (size + head/tail
hash), the analysis settings the Camera track effect was carrying, the mask geometry it was
given, and this tier's own format version. The file is a seven-byte magic (`LUMTRK\0`), a
little-endian `u16` version, then a bincode record of that key, the media's frame rate, the
**clip's own frame count**, and the solve: a pose per source frame, the focal per segment,
the point cloud, the keyframes and the solve's notes. The clip's length is stored because the
solve's poses describe only the span that was followed, and a track that stopped part-way
(K-440) has to read back from the cache as the partial thing it is rather than as a whole
one — version 2 of this tier, and version 1 files are simply never asked for. A file whose magic does not match, whose version is **newer than this
build** (the same refuse-newer rule `manifest.json` follows in §1), whose body will not
parse, or whose stored key is not the one being asked for, is ignored and re-analysed —
every refusal costs one analysis and nothing else.

Global rather than per-project, for the reason `media-index/` is: a solve describes the
*file* and the settings it was analysed under, so two projects cutting the same rushes share
one, and a copy of a project finds its solves already there. The solve is deterministic
(K-415), so a rebuild is byte-identical to what was deleted — asserted by a test, not
assumed.

Rules, binding:
- The global cache root defaults under the user's local app-data and is configurable with a
  size budget ([13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)).
- Deleting any or all of the sidecar at any time MUST be safe: Lumit rebuilds on demand.
- The project file never references sidecar contents; the sidecar is keyed by project uuid
  and content hashes.

## 4. Save, autosave, crash recovery

- **Atomic saves**: write to a temp file in the destination directory, fsync, rename over
  the target. A crash mid-save can never corrupt the previous save.
- **Autosave**: every N minutes (default 5) and before risky operations (export start,
  plugin install), rotating `<name>.autosave-<k>.lum` copies (default keep 5) in an
  `autosaves/` folder beside the project.
- **Journal recovery**: the operation journal ([03-DATA-MODEL.md](03-DATA-MODEL.md) §10) is
  appended between saves to `<global cache root>/<project-uuid>/journal/ops.jsonl` (kept out
  of the `.lum` beside the project so shared projects carry no local paths). After a crash,
  Lumit offers: last save + replayed journal (usually everything), or last save, or an
  autosave. The journal is truncated on successful save.
- Recovery is offered calmly on next launch — one dialogue, no error storm
  ([15-DESIGN.md](15-DESIGN.md) voice rules).

## 5. Presets and templates

- **Preset** (`.lumfx`): a JSON document containing an effect stack (or single effect,
  or animation) parameter tree — same conventions as project.json, shareable, importable by
  drag onto a layer.
- **Template**: an ordinary `.lum` file opened in "new from template" mode (copy, not
  edit-in-place). Community "CC packs" and project files are just these two forms.

## 6. Colour themes (`.lumtheme`, K-298)

A custom theme (K-202) written out on its own so it can be shared: a small indented JSON
document carrying `format: "lumit-theme"`, a `version`, and then the theme itself — `name`,
`mode` (`light`/`dark`, the base it is built over), and `colours`, a map of token key to
`#rrggbb`. The colours are exactly what the workspace file stores, so the shared and the
stored form cannot drift.

Read forgivingly: a colour key this build does not know is ignored and one it does not find
falls back to the base, so a theme written by a newer Lumit still opens. A file whose
`format` says something else is refused. Not a document — Lumit does not *open* a
`.lumtheme`; Settings → Appearance imports one, under a free name if that name is taken.
`flutter_ui/lib/theme/theme_file.dart`.

## 7. Interchange (summary)

- AE Bridge JSON bundles import into this model — [11-AE-IMPORT.md](11-AE-IMPORT.md).
- Lottie JSON: import as comps (subset), export is a possible future.
- OpenTimelineIO: possible future for cut interchange; the Sequence layer/clip model maps
  naturally. Not v1.

## Open questions

- Zip member compression level vs stored-for-speed on large projects — measure once real
  projects exist.
- Should the journal be inside the `.lum` on save (perfect portability of undo history)
  or stay sidecar (smaller files)? Currently sidecar; undo history does not travel.
- Embedded fonts: reference-only v1 with a missing-font warning; embedding raises licensing
  questions — revisit with the text animator work.
- Autosave cadence: time-based v1; consider operation-count-based too.
