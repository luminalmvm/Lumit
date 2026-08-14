# Learn: the codebase, and its languages

Onboarding docs for a developer who is new to Rust, Flutter and GPU code but not new
to programming. They teach the codebase **as built**, with real code excerpts. The
numbered specs in `docs/` stay canonical for intent; [GUIDE.md](../GUIDE.md) stays the
no-code-background companion. These docs sit between the two: enough code to change
the code.

Every excerpt cites `path:line`. Line numbers drift as the code moves; the path and
the shown code are the anchor. When a doc and the code disagree, the code has moved —
fix the doc.

## Reading order

First the map, then the language you need, then the area you are changing.

| Doc | Covers |
|---|---|
| [00-MAP.md](00-MAP.md) | The whole system on one page: crates, threads, data flow, diagrams, and "where do I change X" |
| [01-CORE.md](01-CORE.md) | `lumit-core` + `lumit-project`: time, the document, commands/undo, save/load |
| [02-PIXELS.md](02-PIXELS.md) | `lumit-render` + `lumit-eval` + `lumit-cache`: from snapshot to rendered frame |
| [03-GPU.md](03-GPU.md) | `lumit-gpu` + `lumit-flow`: the device, WGSL kernels, colour, optical flow |
| [04-MEDIA-AUDIO.md](04-MEDIA-AUDIO.md) | `lumit-media` + `lumit-audio` + `lumit-text`: decode, the audio clock, text |
| [05-BRIDGE.md](05-BRIDGE.md) | `lumit-bridge` + `lumit-keymap`: how the Flutter frontend drives the engine |
| [06-FRONTEND.md](06-FRONTEND.md) | `flutter_ui/`: shell, panels, state, theme, strings |
| [07-BUILD-SHIP.md](07-BUILD-SHIP.md) | Building, testing, CI gates, packaging, translations |
| [08-WEBSITES.md](08-WEBSITES.md) | `web/` and `web-docs/`: how to update lumitlab.com and the docs site |
| [RUST.md](RUST.md) | Rust, taught from Lumit's own code |
| [FLUTTER.md](FLUTTER.md) | Dart and Flutter, taught from Lumit's own code |
| [WGSL.md](WGSL.md) | GPU thinking and WGSL, taught from Lumit's own shaders |

## Status

Living documents. Sections marked **Landing soon** describe open pull requests;
delete each note when its pull request merges.
