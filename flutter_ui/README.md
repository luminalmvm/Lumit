# lumit_flutter — the Flutter frontend

Lumit's interface. The Rust engine crates are untouched; it talks to the engine
through `crates/lumit-bridge`.

**How the frontend and engine communicate is specified in
[`docs/17-BRIDGE-CONTRACT.md`](../docs/17-BRIDGE-CONTRACT.md).**

## Running

Requires the Flutter SDK (stable, `mise.toml` has the version CI uses) and
everything the Rust build needs, as `flutter run` builds the engine too. Set up
FFmpeg and LLVM first, from "Building from source" in the
[top-level README](../README.md). On Windows it's the same VS 2022 C++ tools
the Rust build uses, on macOS it's Xcode 26 or newer, and on Linux it's
`clang cmake ninja-build libgtk-3-dev libgles-dev libwebkit2gtk-4.1-dev` as well.

```
flutter run -d windows                # launch, or -d macos, -d linux
flutter test test/theme_test.dart     # one file at a time, never the whole suite
flutter analyze                       # the lint pass (must stay clean)
```

## House rules

- `lib/theme/theme.dart` is the only file where colour hex values may appear.
- Glossary terms bind (docs/01-GLOSSARY.md): layer not track, speed not
  velocity, Retime not time remap, export not render.
- British English, sentence case, no exclamation marks, no emoji.
- Owned widgets over Material chrome.
- Only add a test when it's essential (docs/GUIDE.md, section 2).
