# lumit_flutter — the Flutter frontend

Lumit's interface (decision K-174). The Rust engine crates are untouched; Talks
to the engine through `crates/lumit-bridge`.

**How the frontend and engine communicate is specified in
[`docs/17-BRIDGE-CONTRACT.md`](../docs/17-BRIDGE-CONTRACT.md).** Read
`docs/GUIDE.md` §9 for the plain-English framing. The historical port notes
(strategy, UI inventory, parity checklist) are archived under
[`docs/archive/flutter-port/`](../docs/archive/flutter-port/README.md).

## Running

Requires the Flutter SDK (stable) and the same VS 2022 C++ tools the Rust
build uses.

```
flutter run -d windows    # launch
flutter test              # the test suite
flutter analyze           # the lint pass (must stay clean)
```

## House rules

- `lib/theme/theme.dart` is the only file where colour hex values may appear.
- Glossary terms bind (docs/01-GLOSSARY.md): layer not track, speed not
  velocity, Retime not time remap, export not render.
- British English, sentence case, no exclamation marks, no emoji.
- Owned widgets over Material chrome - see docs/archive/flutter-port/04-WIDGET-MAP.md.
- Every feature lands with its tests.
