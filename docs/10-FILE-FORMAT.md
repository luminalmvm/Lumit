# Luminal project format

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
└── thumbs/                # small embedded preview thumbnails (JPEG/WebP)
    ├── comp-<uuid>.webp
    └── item-<uuid>.webp
```

Rules:
- `manifest.json` MUST be the first entry in the archive and MUST parse standalone:
  `{ "format": "luminal-project", "schema_version": "…", "written_by": "luminal x.y.z",
  "min_reader": "…" }`. A reader newer than `schema_version` migrates; older than
  `min_reader` refuses with a clear message; otherwise it loads and preserves unknowns.
- `project.json` is pretty-printed with stable key order and stable array order, so two
  saves of the same document are byte-identical and version-control diffs are meaningful.
- Thumbnails are disposable previews for the Project panel and file browsers; a reader MUST
  tolerate their absence.
- Nothing else goes in the container. Media is never embedded; caches never ride along.

### 1.1 project.json conventions

- Times: rational pairs `[num, den]` — never floats ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)).
- Colours: linear-light float arrays `[r, g, b, a]`.
- Ids: UUIDv7 strings; every cross-reference is an id.
- Enums: lower-kebab strings (`"blend-mode": "screen"`).
- **Unknown-field preservation is mandatory**: a reader keeps any keys it does not
  understand and writes them back out. This is what lets shared projects and newer/older
  Luminal versions coexist (K-065) and lets Placeholder effects round-trip
  ([11-AE-IMPORT.md](11-AE-IMPORT.md)).

## 2. Media references and relinking

Per `MediaRef` in [03-DATA-MODEL.md](03-DATA-MODEL.md) §3, every reference stores a
project-relative path (preferred), the last absolute path, and a fingerprint
(size + mtime + head/tail hash). On open:

1. Try relative path → 2. absolute path → 3. fingerprint search in user-configured search
   roots and the project's folder tree → 4. mark **missing** (placeholder slate, never a
   blocking error), offer the relink dialogue.

Relinking one file automatically relinks siblings that resolve under the same path mapping.

**Collect for sharing**: an explicit command copies the project plus all referenced media
into one folder, rewriting references relative — the mechanism behind community project
sharing (K-065). Nothing machine-specific is ever written into `project.json` (no cache
paths, no window layout, no local usernames); per-machine state lives in app settings, and
workspaces are app-level with optional project hints.

## 3. The sidecar cache folder

All derived data lives outside the project, in a per-project cache directory:

```
<global cache root>/<project-uuid>/
├── disk-cache/        # rendered frame cache (06-RENDER-PIPELINE.md tier 3)
├── proxies/           # background-generated proxy media
├── peaks/             # audio waveform peak files (09-AUDIO.md)
├── flow/              # cached optical-flow vector fields (04-RETIMING.md, 08-EFFECTS.md)
└── index/             # frame indexes for exact long-GOP seeking (05-ARCHITECTURE.md)
```

Rules, binding:
- The global cache root defaults under the user's local app-data and is configurable with a
  size budget ([13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)).
- Deleting any or all of the sidecar at any time MUST be safe: Luminal rebuilds on demand.
- The project file never references sidecar contents; the sidecar is keyed by project uuid
  and content hashes.

## 4. Save, autosave, crash recovery

- **Atomic saves**: write to a temp file in the destination directory, fsync, rename over
  the target. A crash mid-save can never corrupt the previous save.
- **Autosave**: every N minutes (default 5) and before risky operations (export start,
  plugin install), rotating `<name>.autosave-<k>.lum` copies (default keep 5) in an
  `autosaves/` folder beside the project.
- **Journal recovery**: the operation journal ([03-DATA-MODEL.md](03-DATA-MODEL.md) §10) is
  appended to a sidecar `journal/` log between saves. After a crash, Luminal offers: last
  save + replayed journal (usually everything), or last save, or an autosave. The journal is
  truncated on successful save.
- Recovery is offered calmly on next launch — one dialogue, no error storm
  ([15-DESIGN.md](15-DESIGN.md) voice rules).

## 5. Presets and templates

- **Preset** (`.kfxpreset`): a JSON document containing an effect stack (or single effect,
  or animation) parameter tree — same conventions as project.json, shareable, importable by
  drag onto a layer.
- **Template**: an ordinary `.lum` file opened in "new from template" mode (copy, not
  edit-in-place). Community "CC packs" and project files are just these two forms.

## 6. Interchange (summary)

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
