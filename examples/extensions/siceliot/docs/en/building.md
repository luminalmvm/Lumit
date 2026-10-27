# Building the modified Lumit (Windows)

*Italiano: [../it/compilazione.md](../it/compilazione.md)*

Based on Lumit's own `docs/GUIDE.md`. Paths are the ones used on the SICELIOT machine; on another
machine use your own.

## Tools

| Tool | Version | Installed with | Where it went |
|---|---|---|---|
| Git | 2.56 | `winget install Git.Git` | `D:\Programmi\Git` |
| Rust | 1.97.1 (pinned by `rust-toolchain.toml`) | `winget install Rustlang.Rustup`, then `rustup toolchain install 1.97.1` | `%USERPROFILE%\.cargo` |
| LLVM | 18.1.8 (newer versions break the FFmpeg bindings) | `winget install LLVM.LLVM --version 18.1.8` | `D:\Programmi\LLVM` |
| Visual Studio 2022 Build Tools | C++ desktop workload (`VCTools`) | `winget install Microsoft.VisualStudio.2022.BuildTools` | `D:\Programmi\Microsoft Visual Studio\2022\BuildTools` |
| FFmpeg | 8.1, BtbN *win64-gpl-shared* build | zip from github.com/BtbN/FFmpeg-Builds | `%USERPROFILE%\ffmpeg\ffmpeg-n8.1-latest-win64-gpl-shared-8.1` |
| Flutter | 3.47.1 stable (the version in `mise.toml`) | zip from flutter.dev | `D:\Programmi\flutter` |

## Environment variables (user scope)

| Variable | Value |
|---|---|
| `FFMPEG_LIBS_DIR` | `…\ffmpeg-n8.1-latest-win64-gpl-shared-8.1\lib` |
| `FFMPEG_INCLUDE_DIR` | `…\ffmpeg-n8.1-latest-win64-gpl-shared-8.1\include` |
| `LIBCLANG_PATH` | `D:\Programmi\LLVM\bin` |
| `Path` += | `…\ffmpeg-n8.1-latest-win64-gpl-shared-8.1\bin`, `D:\Programmi\flutter\bin` |

These are what `scripts\win-dev-env.ps1 -Persist` sets; the script looks for LLVM only under
`C:\Program Files\LLVM`, so with LLVM on `D:` set `LIBCLANG_PATH` first.

## Two things this machine needs

- **Developer Mode** on (*Settings ▸ System ▸ For developers*). Flutter links its plugins with
  symlinks and stops with *"Building with plugins requires symlink support"* without it.
- **`ProgramFiles(x86)`** points to `D:\Programmi` here, but `vswhere.exe`, which Flutter uses to
  find Visual Studio, stays in `C:\Program Files (x86)\Microsoft Visual Studio\Installer`. Flutter
  then says *"Unable to find suitable Visual Studio toolchain"*. Set it for the build shell only:

  ```
  ${env:ProgramFiles(x86)} = 'C:\Program Files (x86)'
  ```

## Commands

From `D:\Siceliot\Lumit`:

```
cargo test -p lumit-core --lib        # engine tests for the core crate
cargo build -p lumit_bridge           # the library the app loads
```

From `D:\Siceliot\Lumit\flutter_ui`:

```
flutter test test/l10n/engine_labels_test.dart
flutter build windows --release       # the app, in build\windows\x64\runner\Release
```

The first engine build takes about 7 minutes on this machine; later ones only rebuild what
changed.
