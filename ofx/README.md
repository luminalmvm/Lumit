# Lumit Composition — Resolve link

Windows, Lumit 0.11.2 and Resolve Studio 21.2. Playback requires Lumit open with a saved project.
The effect has Choose Project, Choose Composition, Open in Lumit, and Match
duration to Resolve. Only Choose Project is enabled until a project is selected.
Choosing a project opens it in Lumit; the composition picker reads that open
document, including unsaved compositions. Project and composition values stay
internal for saved links and the Resolve creation script.

Frames use the existing viewer worker, preview scale, view transform and cache.
In CUDA and supported OpenCL modes the connector takes a shared GPU surface from that same renderer
and copies its display pixels straight into Resolve's GPU output. Its small
delivery kernel handles channel order, scaling and orientation; it renders no
layers or effects. The surface is reused and the copy finishes before the next
request can overwrite it. Both apps must use the same GPU.
OpenCL imports that surface through `cl_khr_external_memory_win32` in Resolve's
own context and queue, with no NVIDIA dependency. Drivers without compatible
external-memory handles fall back to shared memory and an OpenCL upload.
CPU hosts use the existing shared-memory delivery buffer.
There is no separate renderer, headless service, frame cache or migration code.
Half-resolution previews are scaled to the Resolve output size.

Build `ofx/build.ps1 -FfmpegDir <shared FFmpeg folder>` using the repository's
Windows prerequisites and Flutter on `PATH`.
The editor stays in Lumit's normal output,
`flutter_ui/build/windows/x64/runner/Release`, with the OFX bundle under its
`Resolve` folder. Run `ofx/install-menu.ps1` to register that plugin folder and
the Resolve menu command, then restart Resolve. The installer EXE is written
to `packaging/windows/dist`; there is no separate root staging directory.
When the CUDA toolkit is available at build time, CMake embeds the delivery
kernel. The installed plugin needs only the NVIDIA driver, not the toolkit or
CUDA runtime DLLs. Without the toolkit the CUDA adapter is omitted.
OpenCL headers enable the OpenCL adapter independently of CUDA; its only runtime
dependency is the system OpenCL driver. AMD-specific validation is still pending.
Installer builds require both development SDKs so a release cannot silently omit
GPU delivery. The release workflow installs these build tools; users need neither SDK.

Run `ofx/install-menu.ps1` once, then restart Resolve. Select video and reference
audio clips and choose **Workspace > Scripts > Create Lumit Composition**.
The selected video becomes a compound with native OFX; its originals stay inside.
Those originals are disabled to avoid decoding them in Resolve. A tiny transparent
PNG sits above them, labeled **Lumit duration canvas - KEEP ENABLED**, with a note
explaining its purpose. Its one-hour span provides extension room while the outer
clip initially keeps the selected range. Its shared Media Pool item is kept in
**Lumit Link - DO NOT DELETE** and named **Lumit duration canvas - KEEP**.
Keep that bin and image in place.
Selected audio is copied into Lumit for reference and left untouched in Resolve.
Cuts, gaps, track order, source trims, enabled state and static audio gain carry
over. Video is fitted to the composition. Resolve effects, grades, inspector
transforms, fades, automation and bus processing are not copied.
Generators, nested clips, unavailable media and retimed sources get labeled,
transparent placeholders with their original timeline spans.

The first link asks in Lumit whether to create a project or choose an existing
one. New files default to the Resolve project's name; you choose the name and
location. The protected canvas item's third-party metadata stores the chosen path
and a private canvas tag inside the Resolve project, never in the PNG file.
Later links on any timeline reuse that tagged item and project. Other images with
the same filename are ignored. A project export/import retains the binding even
when Resolve assigns a new project ID. Canceling the first-link dialogs leaves
the selected clips untouched. Existing unsaved work gets Lumit's normal
save question before switching projects. Composition duration follows the selected
video range; selected audio keeps its offsets even outside that range.
Links also store the Lumit document's persistent ID. Moving or renaming its file
does not change that identity. Open it in Lumit, or use **Open in Lumit** and
locate the file once; all links to that document reconnect. A Lumit dialog explains
the missing project before offering Locate project or Cancel. The existing discovery
folder remembers its latest location for subsequent app restarts. It also keeps
previous paths so existing path-only links follow a file located once. The
saved project path refreshes on the next OFX control action or project save;
frame delivery does not write Inspector parameters. Choosing a
different document is rejected, and two open copies with the same ID are reported
as ambiguous. No filesystem search, timeline rewrite or background helper is used.
Enable **Match duration to Resolve** in the OFX controls to make that clip control
the composition end in Lumit. Resolve supplies the outer clip's trimmed bounds,
independently of the one-hour canvas. Updates use Lumit's normal undoable edits
when Resolve requests a frame; unchanged durations do no editing or disk writes.
Left trims retain the original source-time offset. Enable this on only one clip
per composition. Save the Lumit project normally to persist the changes.
Editing duration in Lumit never resizes the Resolve clip automatically.

The menu command is Lua and runs inside Resolve, without UIManager or external
scripting. Lumit's bundled CLI prepares the native OFX preset and sends a
one-shot import request to the editor, then exits. No Python installation is
needed. Open in Lumit can start the editor and open the saved project and composition.
Free Resolve compatibility is **unverified**: tested here on Studio 21.2, and
Lua/API availability in Free must be checked on a Free installation.

The Windows installer offers **DaVinci Resolve Linking** as an optional component.
It installs the OFX bundle and creation menu command and adds its Resolve folder to
the user's `OFX_PLUGIN_PATH`, preserving other entries. Restart Resolve after
installing. Uninstalling removes this installation's menu entries and plugin path.

Lumit edits do not run scripts in Resolve or toggle effects. When Resolve asks
for a frame, OFX reads the current Lumit preview. Resolve can keep showing its
own cached picture until it decides to request another frame.
When the linked renderer is unreachable, the effect renders a centered
"Lumit is offline" card directing you to **Open in Lumit**. Its surroundings
remain transparent. CPU, CUDA and OpenCL use the same notice image. The next
frame request reconnects when Lumit is available again. The notice also appears
in exports made while offline; closed-Lumit composition rendering is not supported.
