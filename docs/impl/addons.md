# Addons: the model runtime, the model packs, and the analysis they feed

**Decision:** an addon is a large optional download the user installs from Settings and
Lumit never ships or fetches on its own. There are two kinds: the *model runtime* (ONNX
Runtime, loaded at run time through the `ort` crate's dynamic loading) and *model packs*
(one analysis model each, with its licence and a manifest saying what it does). A model
runs only where the user chose it, as a baked analysis with a sidecar or as a named
engine on a control that already exists, never as a default and never inline in a render
path. Nothing generative ever ships as an addon or anything else: no text to image, no
image to video, no prompt-written shaders, no bring-your-own-key provider. The catalogue
of official addons lives in a second repository, `lumit-addons`, and the app reads it
only when the user presses a button. **Related:** plugins and trust (12 §1, §5), the
Settings inventory (07 §15), the flow engine (04 §10, 08 §3.1, [optical-flow.md](optical-flow.md) §0),
the sidecar rules (10 §3), the Roto brush's growth path ([roto.md](roto.md) §9), the
analysis-job shape the camera tracker proved ([tracking.md](tracking.md) §5b), budgets
(13), determinism and refusals (14). This note is the authoritative *how*: the runtime,
the pack format, the install job and its Settings page, where each model runs, what the
frame key and the sidecar record about it, the traps, the test plans, and the ordered
work packages AD1 to AD6.

**Where this note and a spec disagree the spec wins and this note is the bug**, except in
the places named in §7, where the spec is amended by the package that reaches it.

## In plain terms

Some things a compositor wants are best done by a trained model: guessing how far away
every pixel is (a depth map), cutting a person out of their background (a matte), finding
the thing you clicked on and tracing it (segmentation), and inventing the frame that sits
between two real ones (frame synthesis, which Retime's Flow already does with classical
optical flow). These models are large files, from twenty megabytes to a few hundred, and
they need a runtime library to execute. Bundling them would make the installer several
times bigger for something most people never touch, and it would drag other people's
licences into Lumit's own.

So they are addons. Settings gains an Addons page. It lists what is installed and, when
asked, what the official catalogue offers. Each row shows the name, the size, the licence
and a button. Everything runs on the user's own machine on their own graphics card. There
is no account, no server and nothing metered. A project that uses an addon the machine
does not have keeps every value and says calmly what is missing.

The rule that matters most is the one about what a model is *for*. A model here produces
a primitive the compositor consumes: a plane of depth, a plane of coverage, an in-between
frame. It never produces a picture from a description. That is a decision about what
Lumit is, not a gap waiting to be filled.

## 1. Words

- **Addon**: a large optional download installed from Settings, never shipped in the
  application. Two kinds exist, the runtime and the model pack. An addon adds analysis,
  never generation, and Lumit works fully without every one of them.
- **Model runtime**: the one addon every model pack needs: ONNX Runtime plus the
  provider library for the platform (DirectML on Windows). Addon id `runtime`.
- **Model pack**: one addon holding one analysis model: its weights, its manifest naming
  the task and the tensors, its licence and its hash.
- **Task**: what a pack does, one of `synthesis`, `depth`, `matte`, `segmentation`. The
  task is what Lumit's code binds to; the pack's `arch` names the family whose tensor
  contract the code speaks.
- **Provider**: the ONNX Runtime execution provider a session runs on: DirectML, CoreML,
  or CPU. The provider name is part of every key a model result is filed under.
- **Analyse**: the button, already binding (the Camera track uses it). Never Generate,
  never Run.
- **Matte**, not mask, for anything the user reads. `mask` may name a model's raw output
  inside the engine.
- **Frame synthesis**, not frame interpolation, and Retime, never time remap.

"ML" appears in the crate name `lumit-ml` and nowhere the user reads.

## 2. What an addon is, and what it is never (decided)

An addon is a directory under the addons folder holding an `addon.json` manifest and the
files the manifest lists. That is the whole registry: the engine scans the folder, parses
each manifest, and answers. There is no separate preference file recording what is
installed, because the directory is the truth and a second record could only disagree
with it.

An addon is one of:

1. **The runtime** (`kind: "runtime"`, id `runtime`). The ONNX Runtime shared library
   for the platform, plus its provider libraries. Exactly one may be installed. The
   version is pinned by this note (§3) and by the catalogue; a pack never asks for a
   runtime version because there is only one.
2. **A model pack** (`kind: "model"`). One task, one architecture, one or more ONNX
   files, one licence.

An addon is never:

- code. A manifest names files and tensors; nothing in it is executed. ONNX Runtime does
  not run custom operator libraries here (no `register_custom_ops`), so a model file is
  data in the same sense a LUT is data. A later kind `lfx` for plugins from the same
  catalogue is left room for by the `kind` field and is not designed here.
- generative. A pack whose task is anything but the four in §1 is refused by the
  manifest parser, and the catalogue's own check refuses it before it is published.
- a cloud call. The only network traffic in the whole mechanism is the catalogue fetch
  and the file download, both started by a button press, both to URLs the user can read
  on the page.
- automatic. Nothing is fetched at start-up, nothing is probed on a timer, and no model
  runs unless a control on the layer or effect names it (14 §1's no-surprises rule, and
  the standing rule that a smart mode rides an existing control, never a default).

## 3. The runtime: `ort` dynamic loading, the providers, the refusals (decided)

**The crate.** `ort = "2.0.0-rc.13"` with `default-features = false` and features
`["std", "load-dynamic", "api-24"]`, plus `directml` in a Windows-only target table and
`coreml` in a macOS-only one, in one new leaf crate `crates/lumit-ml`. Nothing else in the
workspace names `ort`. The crate needs no `unsafe`: `ort`'s safe API and its own
`libloading` do the opening, so `[lints] workspace = true` stays.

**Why `api-24`.** `ort` rc.13 targets ONNX Runtime 1.28 by default, but the DirectML
build of ONNX Runtime stopped at 1.24.4 (NuGet `Microsoft.ML.OnnxRuntime.DirectML`,
2026-03-17) while the CPU package went on to 1.30. The `api-NN` features lower the API
version the crate asks the library for. `load_dynamic::init` refuses a library whose
minor version is below the feature's number and only logs for a newer one, so 24 loads
the last DirectML build and anything after it. Verified on this machine on 2026-09-13:
the 1.24.4 DLL loads, DirectML initialises, and every pack in §4 opens and runs.

**Files the runtime addon carries, Windows x64.** `onnxruntime.dll` and
`onnxruntime_providers_shared.dll` from `runtimes/win-x64/native/` inside the NuGet
package, and `DirectML.dll` from `bin/x64-win/` inside `Microsoft.AI.DirectML` 1.15.4,
with both licence files. The DirectML licence permits redistribution inside applications
built with machine-learning frameworks; it is shown on the page like every other licence.
macOS and Linux take `libonnxruntime.dylib` and `libonnxruntime.so` from the CPU NuGet
package `Microsoft.ML.OnnxRuntime` 1.24.x, which carries CoreML on macOS. All three are
zip packages, so one unpacker serves them.

**Loading.** `lumit_ml::runtime::load(dir)` runs once per process behind a `OnceLock`:

1. On Windows, prepend `dir` to the process `PATH`. `libloading` opens the library with
   plain `LoadLibraryExW(path, NULL, 0)`, and Windows then resolves that library's own
   dependents (`DirectML.dll`, the providers DLL) by the standard search order, which
   starts at the executable's folder and ends at `PATH`. Without this step the runtime
   loads and DirectML fails to. The alternative flags need `unsafe` or a new `windows`
   feature; the environment edit is one safe line and child processes do not mind.
2. `ort::init_from(dir.join(LIBRARY))?.commit()`. A failure is `MlError::Runtime` with
   the library's own sentence kept for the badge detail, never a panic. The lazy path
   inside `ort` (`setup_api` with no library) panics by design, so nothing in Lumit
   creates a session before `load` has returned `Ok`.
3. Record the loaded version string and the provider that will be tried first.

**Providers.** A session is built with the platform's accelerator first and the CPU
provider as the fallback ONNX Runtime applies on its own: DirectML on Windows, CoreML on
macOS, CPU alone on Linux. Which provider actually took the session is read back and
recorded (§7). A provider that fails to register is not an error; a runtime that fails to
load is.

**Sessions.** `lumit_ml::Session` wraps `ort::Session` and speaks f32 tensors only:
`run(&[(name, shape, &[f32])]) -> Result<Vec<(String, Vec<usize>, Vec<f32>)>>`. The
session is owned by the thread that runs the analysis and is never put behind a shared
lock, because a run is an FFI call that may hold the GPU (14 §1.3). One session per pack
per job; the job drops it when it finishes.

**Refusals**, every one a variant of `MlError`, none a fault: `RuntimeMissing`,
`RuntimeFailed(String)`, `RuntimeInUse`, `PackMissing(task)`, `PackUnreadable`,
`ModelFailed(String)`, `ShapeMismatch`, `Cancelled`. The bridge maps them to text-free enums; the `String`
inside two of them rides the badge's detail slot, which is the one channel where a
library's own words are allowed across the seam.

**Tests without the runtime.** CI has no ONNX Runtime and never fetches one. The crate
mirrors `lumit_gpu::no_adapter`: `REQUIRE_RUNTIME_ENV = "LUMIT_REQUIRE_ML"`, a
`no_runtime()` that asserts the variable is unset and prints `skipping: no model
runtime`, and a pure `runtime_is_required(Option<&str>)` with its own unit test. A test
that needs the runtime reads `LUMIT_ML_RUNTIME_DIR` and `LUMIT_ML_PACKS_DIR` and skips
politely when either is absent; on the reference machine they point at the scratch cache
and the tests run for real. The pure half of the crate (manifests, the scan, unpacking,
tensor packing, pre and post processing arithmetic) has tests that never skip.

## 4. A model pack: layout, manifest, licence, hash (decided)

**Layout on disk.**

```
<addons>/
  runtime/                    addon.json, onnxruntime.dll, onnxruntime_providers_shared.dll,
                              DirectML.dll, LICENSE-onnxruntime.txt, LICENSE-directml.txt
  rife/                       addon.json, rife.onnx
  depth-anything-v2-small/    addon.json, model.onnx
  rvm/                        addon.json, rvm.onnx
  birefnet-lite/              addon.json, model.onnx
  sam2-hiera-tiny/            addon.json, encoder.onnx, decoder.onnx
  .staging/                   an install in progress, renamed into place when complete
```

`<addons>` is `lumit_project::addons_dir()`: `directories::ProjectDirs` `data_local_dir()`
joined with `addons`, which is `%LOCALAPPDATA%\Lumit\Lumit\data\addons` on Windows,
`~/Library/Application Support/dev.Lumit.Lumit/addons` on macOS and
`$XDG_DATA_HOME/lumit/addons` on Linux. It is the first `data_local_dir()` call in the
workspace and its doc comment says why: not `cache_dir()`, because 10 §3 promises the
cache may be deleted at any moment and rebuilt on demand, and a download is not
rebuildable; not `data_dir()`, because that is the roaming profile on Windows and these
are hundreds of megabytes. Nothing here is per project and nothing here goes in a `.lum`.

**The manifest**, `addon.json`, format 1. The same object is the catalogue entry, so an
install copies it into the folder unchanged.

```json
{
  "format": 1,
  "id": "depth-anything-v2-small",
  "kind": "model",
  "name": "Depth Anything V2 Small",
  "version": "1.0",
  "summary": "A depth map from a single frame",
  "licence": "Apache-2.0",
  "licence_url": "https://huggingface.co/onnx-community/depth-anything-v2-small",
  "homepage": "https://github.com/DepthAnything/Depth-Anything-V2",
  "size": 99060839,
  "requires": ["runtime"],
  "platforms": {
    "any": {
      "downloads": [
        { "url": "https://huggingface.co/onnx-community/depth-anything-v2-small/resolve/main/onnx/model.onnx",
          "sha256": "afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c",
          "size": 99060839,
          "unpack": "file",
          "dest": "model.onnx" }
      ]
    }
  },
  "model": {
    "task": "depth",
    "arch": "depth-anything",
    "file": "model.onnx",
    "input": "pixel_values",
    "output": "predicted_depth",
    "size": 518,
    "multiple": 14,
    "mean": [0.485, 0.456, 0.406],
    "std": [0.229, 0.224, 0.225],
    "output_kind": "inverse-relative"
  }
}
```

Rules the parser holds:

- `id` is `[a-z0-9-]+`, at most 64 characters, and is the folder name. `kind` is
  `runtime` or `model`. `format` above 1 is refused as "newer than this build".
- `platforms` keys are `windows-x86_64`, `macos-aarch64`, `macos-x86_64`,
  `linux-x86_64` or `any`. The app picks its own key, then `any`, and refuses a manifest
  with neither. A download is `unpack: "file"` (copied to `dest`) or `unpack: "zip"`
  with `entries: { "path/in/zip": "dest name" }` and only those entries taken. Every
  download has a `sha256` and a `size`, and the file is verified before the engine sees
  it.
- `licence` is an SPDX expression or a short name plus `licence_url`. The page shows
  both. A pack the catalogue knows to be non-commercial is not listed at all; the
  catalogue's own check keeps `licence` out of a small refused set.
- `model.task` is one of the four, `model.arch` names a family the engine has code for,
  and the tensor names and sizes are read from the manifest with the defaults below, so
  a re-export of the same architecture with different tensor names is a catalogue edit
  and not a Lumit release.
- `requires` lists addon ids; today always `["runtime"]`.

**The packs**, with the tensor contracts verified on 2026-09-13 by opening each file:

| id | task, arch | licence | size | tensors |
| --- | --- | --- | --- | --- |
| `rife` (RIFE 4.9, ensemble) | `synthesis`, `rife` | MIT (Practical-RIFE) | 21 MB | `img0`, `img1` [1,3,H,W] RGB 0..1, `timestep` [1]; out `output` [1,3,H,W]; H and W padded to a multiple of 32 |
| `depth-anything-v2-small` | `depth`, `depth-anything` | Apache-2.0 | 99 MB | `pixel_values` [1,3,h,w], ImageNet mean and std, h and w multiples of 14 (518 on the long side); out `predicted_depth` [1,h,w], larger is nearer |
| `rvm` (Robust Video Matting, MobileNetV3) | `matte`, `rvm` | GPL-3.0 | 15 MB | `src` [1,3,H,W] 0..1, `r1i` to `r4i` recurrent state (zeros [1,1,1,1] on the first frame, then the previous `r1o` to `r4o`), `downsample_ratio` [1]; out `fgr`, `pha` [1,1,H,W] |
| `birefnet-lite` | `matte`, `birefnet` | MIT (trained on DIS5K, whose terms are non-commercial; stated on the page) | 224 MB | `input_image` [1,3,1024,1024], ImageNet mean and std; out `output_image` [1,1,1024,1024] logits, sigmoid to coverage |
| `sam2-hiera-tiny` (SAM 2.1) | `segmentation`, `sam2` | Apache-2.0 | 117 MB zip | encoder `image` [1,3,1024,1024] ImageNet mean and std, out `image_embed` [1,256,64,64], `high_res_feats_0` [1,32,256,256], `high_res_feats_1` [1,64,128,128]; decoder takes those plus `point_coords` [1,N,2], `point_labels` [1,N], `mask_input` [1,1,256,256], `has_mask_input` [1], out `masks` [1,3,256,256] logits and `iou_predictions` [1,3] |

The `arch` field is what the engine matches on. Tensor names in the manifest override the
defaults the engine carries for each arch, and a manifest naming a tensor the model does
not have is refused at load with `ShapeMismatch` and the badge says which.

**Hashes.** The catalogue carries SHA-256 because that is what release pages publish and
what Dart already streams. The installed manifest keeps the hash the file was verified
against. The engine does not re-hash a two-hundred-megabyte file on every start; it checks
the file is there and its size matches, and the model's identity in every key (§7) is the
manifest's hash, so a swapped file under the same manifest is a corrupt install and not a
silently different answer.

## 5. Install, update and remove: the Settings page and its job (decided)

**The split.** The download is Dart's; the unpack, the placement and the registry are
the engine's. No Rust crate in the workspace speaks HTTP, and adding one means adding a
TLS stack and a licence decision for `ring` or `aws-lc-rs` to a GPLv3 build on three
platforms. `flutter_ui/lib/state/updates.dart` already streams a release asset to disk
with a cancel check per chunk, notifies once per whole per cent, and verifies size then
SHA-256 with `package:crypto`. Those three functions move to a shared file and serve both
the update and the addon. "Flutter is a thin view" is about decisions over the document;
fetching bytes to a temp file is not one.

**The service.** `flutter_ui/lib/state/addons.dart`, `AddonService extends
ChangeNotifier`, every collaborator injected the way `UpdateService` injects its fetcher,
downloader and extractor, so the widget tests drive the whole flow with no network:

- `check()`: fetch the catalogue `index.json` from the official URL (a constant, the
  `lumit-addons` repository's `main` branch), parse, hold the entries. Only on the
  button. A failure is a sentence on the page, never a dialogue.
- `install(id)`: for each download of the entry's platform block, stream to
  `<addons>/.downloads/<id>/<n>`, verify, then call the engine's `addon_install` with
  the manifest text and the file paths. Stages `downloading`, `verifying`, `installing`,
  `done`, `failed`, with a fraction while downloading. One install at a time; a second
  press while one runs is refused with a sentence.
- `installFromFile(path)`: an `addon.json` the user picked, with the files it names
  sitting beside it; each one is hashed where it lies and handed to the engine as its
  own `unpack: "file"` download, so the same copy and the same checks happen as for a
  catalogue install. A folder rather than a zip, because the manifest text has to come
  back to this side to be handed on and nothing across the seam reads one entry out of
  an archive for it. This is how a pack that is not in the catalogue, or a machine with
  no network, gets one.
- `remove(id)`: the engine deletes the folder; the row goes.
- `cancel()`: stops the download between chunks and deletes the partial file.

**The engine side**, `crates/lumit-bridge/src/api/addons.rs` over `lumit_ml::store`:

- `addons_dir() -> Option<String>`, sync, so Dart and Rust cannot disagree about the
  folder.
- `addon_list() -> Vec<BridgeAddon>`, sync: the scan. `{ id, kind, name, version,
  summary, licence, licence_url, size_bytes, task, broken: bool }`, `broken` when a
  listed file is missing or the wrong size.
- `addon_install(manifest: String, files: Vec<String>) -> Result<(), BridgeError>`, not
  sync: parses the manifest, refuses an unknown format or task, unpacks each download
  into `.staging/<id>-<nonce>/` (the `zip` crate, deflate only, already in the graph),
  writes `addon.json`, moves any existing folder of that id aside under `.staging`,
  renames the staging folder into place and only then throws the old one away. Two or
  three seconds for the largest pack, so it rides the frb worker pool the way
  `rescan_plugins` does rather than owning a thread.
- `addon_remove(id) -> Result<(), BridgeError>`, sync.
- `addon_runtime() -> BridgeRuntimeStatus`, sync: `{ state: Missing | Present | Loaded |
  Failed, provider: String, version: String, detail: String }`. `Present` means the
  folder is there and nothing has asked for it yet; `Loaded` means `load` returned `Ok`
  since Lumit started.

Every refusal is a `BridgeError` variant (`AddonBusy`, `AddonInvalid`, `AddonMissing`)
with a `Display` arm, and every one of them reaches Dart as a typed result.

**The page.** `SettingsPage.addons`, after `export` and before `previewAndCache`, label
`settingsPageAddons`. Three sections built from `_sections`, `_row` and the House
controls so the title-strip search and the metrics test hold:

1. **Runtime.** One row: name, version, provider and state on the description line,
   Licence and Install or Remove on the right. It is the largest download on the page, so
   while it comes down its row carries the bar and the Cancel like any other. Until it is
   installed every model row's button reads "Needs the runtime" and is disabled.
2. **Installed.** One row per pack: name, then licence, size and task on the description
   line, Licence and Remove on the right. An empty section says so in one sentence.
3. **Available.** A "Check for addons" button, then one row per catalogue entry not yet
   installed, installed at another version, which reads Update, or installed with its
   files gone, since installing it again is what mends that: name, licence, size, and any
   terms the catalogue records beyond the licence, in its own words. While one installs,
   its row shows the `HouseProgressBar` and a Cancel, and every other button is disabled.
   Below the list: "Install from file" and "Show folder" (`reveal_in_folder`).

Progress is the service's own `notifyListeners`, so the page needs no poll of the engine;
the one engine read is `addon_list` plus `addon_runtime` on page entry and after each
install or remove, never in `build()`. Colours are theme tokens: `t.accent` for the bar,
`t.textMuted` for the description line, `t.warning` for a broken row. Every string is an
`app_en.arb` key with a description.

**Reaching the page from elsewhere.** `showSettingsWindowFrb` gains an optional
`initialPage`, seeded and shown post-frame so the page's on-entry read runs. The badge
detail on an effect whose pack is missing and the Flow group's engine row both link there.

**Uninstall and updates.** The Windows installer, the macOS bundle and the Flatpak change
nothing for installation: a library under the user's local data folder loads with no
registry, PATH or entitlement edit (macOS already carries `disable-library-validation`
for the plugin hosts). The Flatpak manifest gains `--share=network` in `finish-args`,
because without it no download can succeed on Linux; it is the one packaging edit this
programme makes. An uninstall leaves the addons folder alone, as it leaves every other
user folder alone today.

## 6. Where a model runs: the analysis job, the sidecar, the frame key (decided)

Three of the four tasks run as baked analysis in the shape the tracker and the Roto brush
already share, and the fourth (synthesis) runs at Retime's synthesis seam.

### 6.1 The planes tier: depth and matte

A new module `crates/lumit-render/src/planes.rs`, a sibling of `track.rs` and `roto.rs`,
copied structurally from the Roto brush's half because that is the per-frame raster
case:

- **Settings** (`PlaneSettings`, Copy and Eq): `task`, `arch` index, the model's manifest
  hash as `[u8; 32]`, the provider as a small code, `downsample` for RVM, and the
  effect's own rows that change the answer (nothing display-only). Read off the
  instance with per-field defaults.
- **Key** (`PlaneKey`): `media` prefix plus `run` hash in the Roto brush's two-part
  shape, over a domain literal `b"lumit-planes/"`, `FORMAT_VERSION`, the media
  fingerprint, and the settings. Filed under the effect instance id, so two instances on
  one clip are two runs, and two projects on the same rushes with the same settings find
  the same file.
- **Job**: `instance`, `key`, `settings`, `open: Box<dyn FnOnce() -> Option<Box<dyn
  RotoFrames>> + Send>` (the RGBA-at-source-raster trait roto already has; it moves to a
  small shared module so both tiers name one trait), `analyse: bool`, `stop_after`.
- **Run**: decode a frame, pack the tensor, run the session, unpack, quantise, compress,
  append a record, report, check the cancel flag, next frame. The session is created on
  the worker thread from the pack the settings name, and dropped at the end. A depth
  plane is stored at the model's own output resolution as `u16` under LZ4 (the model
  produced nothing finer; the draw resamples), a matte as bbox-cropped `gray8` under LZ4
  exactly as roto's `FrameRecord`. RVM carries its recurrent state across frames, so a
  run is sequential from the first frame and a cancel keeps the finished prefix (roto's
  stance, since every frame reached is correct and correctly keyed).
- **Sidecar**: `lumit_project::planes_cache_dir()` = `cache/planes`, magic `b"LUMPLN\0"`,
  version 2, `sidecar::frame` framing, the key repeated inside, refusals identical to
  `track/` and `roto/`. Deleting it is safe and costs a re-analysis. Version 1 was the
  record before a matte's box was in it (AD4), and the version is in the key, so the files
  a build of that vintage wrote are simply never asked for.
- **Store**: process-wide `OnceLock<RwLock<HashMap<Uuid, Arc<PlaneRun>>>>`, guard
  dropped inside every accessor, planes cloned out as `Arc`. Its own one-at-a-time slot,
  separate from the tracker's and the Roto brush's: those two exist because a
  disk-bound job halves another on the same drive; this one is GPU-bound and contends
  with the compositor's device instead, and two model runs on one card halve each other
  too, so one slot for every analysis a model reads. The Roto brush's segmentation is
  the named exception (§9).
- **Draw**: one carriage, `PlaneDraw { width, height, kind, data: Arc<Vec<u8>> }`,
  filled in `build.rs` by (instance, source frame) beside `roto_mattes_for`, uploaded in
  `realise.rs` and memoised per (instance, source frame) beside the LUT cache from the
  start, and applied inline at the `run_ops` seam. The roto list and this list fold into
  one `side: &Side` parameter on `run_ops`, which is what the note at `fxops.rs:590`
  asks the second such list to do. That same parameter is threaded into
  `render_layer_inputs`, so a layer read as another effect's depth or matte source
  carries its planes. Today a Roto brush layer used that way renders as a passthrough;
  the fold fixes both.
- **Frame key**: a per-frame stamp fed the way `feed_roto` is, from a
  `lumit_core::planes::frame_stamp(fx, frame)` that hashes the settings (which carry the
  model hash and the provider) and nothing display-only. Constant across the clip for
  these effects, since they have no strokes; still per frame so the shape stays the
  roto one. Emitted for no layer that lacks such an effect.

Two built-in effects ride this tier, both in the Roto brush's declaration shape
(`#[action] analyse`, `#[action] cancel`, `matte = false`, `premultiplied = false`,
`cost = Trivial`, `roi = Exact`, `is_image_op() -> true`, no `GPU_EFFECTS` entry, argued
into the same two test exceptions the Roto brush is):

- **Depth** (`match_name = "depth"`, beside the Camera track in the catalogue order).
  Rows: `model` (Choice over the depth packs the engine has an arch for, today one),
  `view` (Choice: Depth, Source), `invert` (Toggle). Analysed, the layer's picture in
  Depth view is the depth plane drawn as an opaque grey picture, nearer is brighter
  unless inverted; in Source view the effect is a passthrough so the layer can be
  looked at while the plane rides to a consumer. A Depth of field on another layer
  points its Matte row at this layer, and Set matte or a track matte can read it the
  same way. Nothing on the consumer side changes.
- **Remove background** (`match_name = "remove_background"`). Rows: `model` (Choice:
  Robust Video Matting, BiRefNet), `view` (Choice: Composite, Matte), `invert`, and for
  RVM a `detail` Choice mapping to the downsample ratio (Portrait, Full body). Analysed,
  Composite view multiplies the matte into the layer's alpha exactly as the Roto brush's
  pass does with `set_matte`; Matte view draws the plane as grey.

Without a plane for the frame the effect is a passthrough, documented, never a held
neighbour. Without the pack or the runtime the effect wears the calm badge
`addon_missing` from `BADGE_REASONS`, with the detail naming the pack, and its values
stay live and saved.

### 6.2 Segmentation: the Roto brush's seed seam

SAM 2 does not replace anything in the Roto brush; it seeds it. [roto.md](roto.md) §9
pins the seam at stage 3, where seeds for a frame are derived, and that is where it
lands:

- **The prompt** is document state: `prompts: Vec<RotoPrompt>` on `RotoBlock`, each
  `{ id, frame, points: Vec<(f32, f32)>, labels: Vec<u8> }` in source raster pixels,
  `#[serde(default, skip_serializing_if = "Vec::is_empty")]` so older projects round
  trip byte for byte. A prompt is fed into `chain_hash` and `key_hash` through the same
  `between(base, frame, p.frame)` predicate strokes use, with the id left out so a
  redrawn prompt keeps its cache. In this package a prompt is only ever on the base
  frame, which keeps §1's purity sentence intact; prompting a later frame is a decision
  for another note.
- **The model identity** (pack hash and provider) is fed in `RotoSettings::feed` and
  `TIER_VERSION` bumps, so `lendable()` can never lend a frame solved under one seed
  source to a run under another.
- **The seed constructor** is `lumit_roto::mask_seeds(mask, w, h, out)` beside
  `warp_and_seed`: coverage above 0.9 seeds foreground, below 0.1 background, then the
  same two-pixel erosion. `lumit-roto` stays dependency-free; the mask arrives as a
  borrowed slice.
- **The run**: at the base frame, when the block carries a prompt and the `seed` row
  reads Segment, the propagation thread opens the SAM 2 pack, runs the encoder once on
  the frame (resized to 1024 on the long side, padded, ImageNet normalised), runs the
  decoder with the prompt's points and labels, takes the mask with the best
  `iou_predictions`, upsamples the 256-square logits to the frame and thresholds at 0,
  builds seeds from it, then stamps the user's strokes over them. Everything after
  `Seeds` is unchanged. The encoder's embedding is kept for the length of the run so a
  second prompt on the same frame pays only the decoder.
- **The control**: a `seed` Choice on the Roto brush, `Strokes` (default) or `Segment`.
  With Segment chosen, a tap of the Roto brush tool on the base frame adds a prompt
  point, Alt makes it negative, and a drag still lays a stroke; the overlay draws prompt
  points as small rings in `t.success` and `t.error`. Release asks for the base frame's
  own solve through `roto_solve_frame`, unchanged.
- **Refusals**: `RotoFailure::ModelMissing` and `ModelFailed`, mirrored in
  `BridgeRotoFailure` and `rotoFailureSentence`, in the register of
  `rotoFailedFlowUnavailable`.

### 6.3 Synthesis: Retime's Flow seam

RIFE synthesises the in-between frame directly and emits no flow field, so it slots in
at the synthesis level and nowhere else. Motion blur and Datamosh keep DIS vectors
whatever the layer's engine, and the Roto brush never sees it ([roto.md](roto.md) §8).

- **The document**: `FlowEngineChoice { #[default] Dis, Rife }` in
  `lumit_core::retime`, shaped like `FlowResolution` with `OPTIONS`, `code` and
  `from_code`; `#[serde(default)] pub engine: FlowEngineChoice` on `FlowParams`. An older
  project reads Dis. Clips carry it through `Interpolation` with no extra work.
- **The key**: one more byte in `feed_interp`'s fixed-width block, plus the pack hash
  when the engine is Rife, so two engines never share a comp frame name (this is the
  algorithm-version byte 04 §11.7 asks for and nothing wrote). `CompJob::key` serialises
  the whole struct and needs nothing.
- **The seam**: one branch at the top of `combine_pair`'s Flow arm, before
  `flow_settings` is reached: when the engine is Rife, hand the two decoded frames and
  the phase to `DecodePool`'s lazily built `lumit_ml::Synthesis` (held beside
  `flow_engine`), which pads to a multiple of 32, packs NCHW 0..1, runs, unpacks and
  crops. The endpoints keep their bit-exact contract: phi at or below 0 returns A and at
  or above 1 returns B before any backend is asked. LinearF32 sources are refused with a
  named error rather than clamped, since the model is trained on 0..1.
- **The control**: an `engine` row inside the Flow group, beside Flow resolution, in
  both places the group is drawn (the Effect controls' `FlowRowsFrb` and the Timeline
  fold-out's `FlowRowKind`), options `Built in` and `RIFE`. Never a fourth entry in the
  Source card's In-between dropdown, for the reason written at `source_rows_frb.dart`.
- **Missing pack or runtime**: no silent downgrade (08 §3.1, 04 §10). In preview the
  layer draws with the built-in engine and the engine row's description line reads
  "RIFE is not installed, using the built-in engine" with a link to the Addons page.
  The row is asked of the layer rather than of the machine, because one of the five
  answers is about the layer's own footage: a scene-linear source is refused by name,
  and the layer beside it on ordinary rushes is painted by the model all the same. An
  export whose document names Rife on any layer or clip without the pack present
  refuses to start with `BridgeError::AddonMissing`, and the export dialogue shows the
  same sentence. Installed is not the same as able to paint, so the pre-flight counts
  a runtime that would not load and a pack the last frame could not open as needs too,
  and a model that gives up part way through a run abandons the file rather than
  letting the built-in engine finish it. Both of the dialogue's footer actions are refused, Add to queue with
  Export: the document is snapshotted as the item is added, so an item that names a
  missing pack is one that will fail whenever the queue reaches it, and the moment the
  button is pressed is the moment somebody is there to be told. The check is one
  function, `lumit_render::addon_needs(doc) -> Vec<Need>`,
  that both the status readout and the export pre-flight call. It is in `lumit-render`
  rather than `lumit-ml` because it reads a document: `lumit-ml` sits under
  `lumit-core`, not above it, and a leaf crate that knew what a layer was would be the
  dependency graph running backwards.

## 7. Determinism, and the record of which backend produced this

14 §1 says the same project and inputs give the same pixels on every machine. A model
under DirectML does not: the same file on two cards, or one card across two driver
versions, differs in the low bits. The stance, already written in
[roto.md](roto.md) §8 and [optical-flow.md](optical-flow.md) §0 and made real here:

- **Invalidation.** Every key a model result is filed under carries the pack's manifest
  hash and the provider that ran it: the planes sidecar key and its per-frame stamp, the
  roto chain hash through `RotoSettings::feed`, and the comp frame key through
  `feed_interp`. A result made under one backend is never served for another.
- **Once cached, the answer is the input.** The planes and roto sidecars are what
  preview and export read, so an export is stable across driver updates until the user
  presses Analyse again. Synthesis has no sidecar; its frames are ordinary frame-cache
  material, and an export re-synthesises, which is the one place two machines can
  legitimately differ. 08 §3.1's engine-quality table gains a row saying so.
- **Provenance proper.** The planes sidecar `Record` carries `made_with: String`, the
  runtime version, provider and pack id and hash that produced it, behind its format
  version from day one. The roto sidecar gains the same field with a `FORMAT_VERSION`
  bump in AD5. Nothing goes into the export container's metadata: `encode::Metadata` is
  ordered so exported bytes stay machine-independent, and a provider string there would
  undo that.
- **The spec edits this note owns**, made by the package that reaches each:
  [optical-flow.md](optical-flow.md) §0's sentence "the project stores which backend
  rendered" becomes "the frame key carries the engine and the pack hash, and the
  sidecars carry provenance" (AD2). 14 §1's determinism bullet gains one sentence
  admitting the model case and pointing here (AD3). 10 §3 gains the `planes/` tier and
  a paragraph for the addons folder, which is the first thing under the user's data that
  is not rebuildable (AD1). 08 §2.6's "two effects carry none" becomes five, the Roto
  brush having been exempt in code and uncounted in the prose (AD4). 08
  §3.96's "classical machinery, not a neural matter" gains "unless the seed row says
  Segment" (AD5).

## 8. Budgets (13 stance: measured, then gated)

Measured on 2026-09-13 on the reference machine, ONNX Runtime 1.24.4, warm session:

| Model | DirectML | CPU |
| --- | --- | --- |
| RIFE 4.9, 1920 by 1088 | 46 ms a frame | 1.7 s |
| RVM, 1920 by 1080, downsample 0.25 | 10 ms | 55 ms |
| BiRefNet Lite, 1024 square | 1.6 s | not measured |
| Depth Anything V2 Small, 518 square | 15 ms | not measured |
| SAM 2.1 tiny encoder, 1024 square | 53 ms a frame | not measured |
| SAM 2.1 tiny decoder, one tap | 24 ms a tap | not measured |

The first run of each session costs 0.8 to 1.5 s (DirectML compiles the graph); a job
pays it once. Rows for 13's table: a planes job holds to 60 ms a frame at 1080p on the
reference GPU for depth and matting, and a synthesis frame at 1080p to 60 ms, both
measured by `--ignored` tests that print and never gate, in the tracker's stance. VRAM: a
session's arena is ONNX Runtime's own and is not a frame-sized allocation the pools know
about; the job registers one frame-sized scratch entry with the governor for its input
and output tensors and the note says the arena is outside the ledger. A job drops its
session when it finishes so nothing is held between analyses.

CPU fallback is allowed and reported, never silent: the runtime row says CPU, and the
planes status card says which provider is running. A synthesis on CPU at 1.7 s a frame is
the user's choice to make. So is BiRefNet, the one model here nowhere near the row's 60 ms
even on the graphics card: it reads a whole 1024 square whatever the frame is, and a shot of
it is a long analysis. The `--ignored` tests measure the whole job rather than the graph
alone, so what they print carries the frame's trip in and the plane's trip out: Robust Video
Matting at 1080p is 28 ms a frame that way against the graph's own 10, and packing the frame
single-threaded is where the rest goes. The two SAM 2 rows are the same reading, taken on
2026-09-14 over a 1920 by 1080 frame: a prompted base frame pays the encoder once and the
decoder once a tap, which is why they are apart. Neither is in 13's table, because a
segmentation is one frame of one run rather than a per-frame budget.

## 9. Refusals and failure (14)

- Every failure is a named variant and reaches the user as a sentence from the arb,
  never a fault, never a dialogue. The detail slot on the badge is the one place a
  library's own words appear, untranslated, marked as such.
- A missing runtime or pack is `addon_missing` on an effect, a description line on the
  Flow group's engine row, and a refusal at export start. The Viewer's missing-file
  slate is not reused: an addon is not a file the project references.
- Cancellation is per frame: one atomic flag checked between frames, and a single model
  run of one frame is not interruptible. The note says so rather than implying finer.
- An install that fails part way leaves nothing: staging is renamed into place only
  when every file is present and verified, the install that was there is moved aside
  rather than deleted so a rename that will not go through can be put back, and a stale
  staging folder is deleted on the next scan. `.downloads`, which is Dart's, is swept
  the same way whenever the page comes forward with nothing in flight.
- The runtime cannot be replaced or removed once it has loaded: the library stays open
  for the life of the process, so `RuntimeInUse` is answered before a single file goes
  rather than half a folder being deleted around a locked DLL.
- Two model **analyses** never run at once; the second is refused Busy. The
  segmentation that seeds a Roto brush is outside that slot on purpose, and this
  is the whole of the reason: it is one encode and one decode at the head of a
  propagation, 77 ms of a job measured in minutes, so refusing the shot over it
  would cost the user minutes to save milliseconds. Both sessions are alive on
  the card for that window and each is a little slower for it.
- No lock is held across a session run. The session is owned by the job's thread.

## 10. The surfaces

- **Settings, Addons page** (§5).
- **Effect controls**: the Depth and Remove background cards on `StatusPoller`, in the
  Camera track's shape, with `Analyse` and `Cancel`, the progress sentence, the provider
  and the refusal sentences; the `addon_missing` badge.
- **The Flow group**: the engine row, in both drawings of the group.
- **The Roto brush card**: the seed row, the prompt count in the status sentence, the
  overlay's prompt markers.
- **The export dialogue**: the pre-flight refusal sentence.
- **The Effects and presets panel**: Depth and Remove background appear beside the
  Camera track with the same provenance tag every built-in has.

Strings, all through `app_en.arb` with descriptions, listed per package in its pull
request draft; labels the engine sends (effect names, row labels, choice options, the
badge key) through `engine_labels.dart` and the two regenerated fixtures.

## 11. Test plans (implement with each package)

Synthetic inputs throughout, as the tracker and the Roto brush do: a manifest written by
the test, a zip built by the test, a plane drawn by the test. The runtime tests skip
politely off the reference machine and run for real on it.

1. **Manifests**: a good manifest round trips; `format` 2 is refused as newer; an
   unknown task, an unknown kind, a bad id, a missing platform block and a download
   without a hash are each refused by name; the platform pick prefers the exact key
   over `any`.
2. **The scan**: an empty folder lists nothing; a folder with two packs and a stale
   staging directory lists two and deletes the staging; a pack whose file is missing or
   the wrong size lists as broken.
3. **Install**: from a `file` download and from a `zip` download with two entries, into
   a temp addons dir (a process-wide override in the tracker's `TEST_CACHE_DIR` shape,
   since the install runs on a worker thread), the folder holds exactly the named files
   plus `addon.json`; a second install of the same id replaces the first; a failure
   after the first file leaves no folder; remove deletes it.
4. **Runtime rules**: `runtime_is_required(None | "" | "0")` skip, `"1"` requires; a
   missing library is `RuntimeMissing`; on the reference machine `load` succeeds, reports
   the provider, and a session on a tiny model the test writes as raw ONNX bytes
   (an `Identity` graph, protobuf assembled by hand in the test) round trips a tensor.
5. **Tensor arithmetic**: NCHW packing and unpacking round trip at 8 bit and f32; the
   padding to a multiple of 32 or 14 and the crop back are exact; ImageNet normalisation
   inverts; RVM's downsample table picks the documented ratio per resolution.
6. **Planes tier**: over a synthetic clip through `RotoFrames`, with a fake session that
   returns a written-down plane: progress readings, the record per frame, cancel keeps
   the prefix, the sidecar round trips and refuses a wrong key, a newer version and
   garbage, a deleted file rebuilds, the store answers `None` outside the span, and the
   frame key renames exactly the frames the settings change touches and none for a
   layer without the effect. Two runs on the fake session are bit-identical; the real
   model is held to a tolerance test only, and the note says why.
7. **The carriage fold**: two ops on one layer with the first slot deliberately empty;
   a Roto brush layer read as another effect's matte source now carries its matte.
8. **Depth and Remove background**: the schema sweeps pass, the button promises hold
   (`a_button_is_a_row_with_no_value`), the badge reads `addon_missing` with no pack and
   nothing with one, Depth view draws the plane, Composite view multiplies the alpha,
   both are passthroughs outside the span.
9. **Synthesis**: `FlowEngineChoice` codes round trip, an old project reads Dis, the
   frame-key knob test gains the engine, the endpoints are bit-exact through the new
   branch, LinearF32 is refused by name, the bridge writes the engine whole with the
   group, the export pre-flight refuses when the pack is missing, and on the reference
   machine `flow_quality` gains a `rife` variant judged on the same clips.
10. **Segmentation**: a prompt round trips and hashes through `between`; a redrawn prompt
    keeps its cache; the model identity renames everything; `mask_seeds` seeds the
    pixels it should from a written-down mask; a tap with Segment chosen lands as a
    prompt in source pixels at view scale 0.5; `ModelMissing` has a sentence.
11. **The page**: the sidebar entry, every named control per `shell_frb_test`'s table,
    a fake fetcher and downloader drive check, install, cancel and remove and the stages
    are asserted; no bridge call in a rebuild; the metrics test needs no change.
12. **Perf, `--ignored`**: the numbers in §8 measured and printed at 1080p.

## 12. Ordered work packages

| # | Package | Lands |
| --- | --- | --- |
| AD1 | Foundation: `lumit-ml` (manifest, scan, install, runtime, session, skip rule), `addons_dir`, the bridge module, the Dart service and the Addons page, the strings, the Flatpak line, GUIDE and glossary rows, 10 §3, 07 §15, 12 §6, 17's section | Built. Tests 1 to 5, 11 |
| AD2 | Synthesis: `FlowEngineChoice`, the key byte, the seam, the engine row, the refusals, the export pre-flight, the `flow_quality` variant, optical-flow.md §0's sentence | Built. Test 9 |
| AD3 | The planes tier and Depth: `planes.rs`, the sidecar, the store, the carriage fold through `run_ops` and `render_layer_inputs`, the Depth effect, its card, its badge, 08's section, the manual page, 14 §1's sentence | Built. Tests 6, 7, 8 |
| AD4 | Remove background on the planes tier: RVM with its recurrent state, BiRefNet for stills, the effect, its card, 08 §2.6 | Built. Test 8 |
| AD5 | Segmentation: the prompt on `RotoBlock`, the hashes, `mask_seeds`, the SAM 2 run at the seed seam, the seed row, the tap path, the overlay, the refusals, roto.md §9's paragraph and 08 §3.96 | Built. Test 10 |
| AD6 | The `lumit-addons` repository: `index.json`, one folder per addon with its manifest and README, the check script and its CI, the repository README | Built. Catalogue check |

AD6 needs only §4 and runs beside AD1. AD2 to AD5 each need AD1 and are independent of
each other.

## 13. Traps, collected

- **The dependents trap on Windows.** `onnxruntime.dll` loads and `DirectML.dll` beside
  it does not, because Windows looks for a loaded library's dependents beside the
  executable, not beside the library. Prepend the folder to `PATH` before `init_from`.
- **The lazy path panics.** `ort` looks for `onnxruntime.dll` on its own if a session is
  built before `init_from`, and `expect`s. `load` runs first, always, and returns a
  refusal.
- **`api-24`, not the default.** The default targets 1.28 and refuses the 1.24.4 DLL
  outright. The pin is in the crate features and in the catalogue entry, and the two
  move together.
- **`download-binaries` off.** `ort`'s default features fetch a prebuilt runtime at build
  time. That is a blob outside cargo-deny's view in every CI job and the opposite of the
  premise. `default-features = false`.
- **The coverage gate is one number.** `lumit-ml`'s inference half never runs in CI, so
  the crate is held out of the `llvm-cov` list with a sentence saying why, as `lumit-gpu`
  is, until a runner can install the runtime.
- **A digest with a leading `#` fails the no-hex job.** The Rust half of that lint is a
  grep for `#` and six hex characters anywhere in a `.rs` file, comments included. Write
  hashes bare.
- **`lendable()` trusts the chain hash alone.** A model identity missing from
  `RotoSettings::feed` lends frames across seed sources. The test in 11.10 is the fence.
- **The frame key is the easiest thing to forget.** A plane drawn through and not in the
  key serves a banked frame after the analysis changes, forever. The per-frame stamp is
  the fix; hashing the whole table is the mirror mistake.
- **Do not copy `GpuSynth`'s silent degrade.** Falling from a user-chosen engine to
  another with no signal is a spec violation. Preview says so on the row; export refuses.
- **A layer input renders with empty carriages today.** Threading the folded side
  parameter into `render_layer_inputs` is what makes a Depth layer readable as a Matte
  source, and it fixes the Roto brush's version of the same fault.
- **`AnalysisSettings` is Copy and Eq.** A model name as a `String` breaks both; the
  identity is a `[u8; 32]` hash and an arch index.
- **The staging rename is the atomicity.** Nothing is written into the final folder
  directly, and a stale staging folder is swept on the next scan.
- **The DirectML package is 193 MB for an 18 MB file.** The catalogue points at the NuGet
  package today; re-hosting a trimmed zip on the `lumit-addons` release page is a
  catalogue edit and the hash changes with it.
- **RVM is a sequence, not a set.** Its state rides frame to frame, so a run starts at
  the first frame, a cancel keeps the prefix, and a resume re-runs from the last kept
  frame's state, which the record does not store: a resume is a fresh run that copies
  nothing. Said here so nobody promises prefix reuse for it.
- **SAM 2's decoder wants the encoder's three outputs, not one.** Keep all three for the
  run; the embedding alone is not enough.
