# Compilare il Lumit modificato (Windows)

*English: [../en/building.md](../en/building.md)*

Basato sulla guida di Lumit (`docs/GUIDE.md`). I percorsi sono quelli del PC SICELIOT; su un altro
PC usa i tuoi.

## Strumenti

| Strumento | Versione | Installato con | Dove si trova |
|---|---|---|---|
| Git | 2.56 | `winget install Git.Git` | `D:\Programmi\Git` |
| Rust | 1.97.1 (fissata da `rust-toolchain.toml`) | `winget install Rustlang.Rustup`, poi `rustup toolchain install 1.97.1` | `%USERPROFILE%\.cargo` |
| LLVM | 18.1.8 (le versioni più nuove rompono i binding di FFmpeg) | `winget install LLVM.LLVM --version 18.1.8` | `D:\Programmi\LLVM` |
| Visual Studio 2022 Build Tools | carico di lavoro C++ desktop (`VCTools`) | `winget install Microsoft.VisualStudio.2022.BuildTools` | `D:\Programmi\Microsoft Visual Studio\2022\BuildTools` |
| FFmpeg | 8.1, build BtbN *win64-gpl-shared* | zip da github.com/BtbN/FFmpeg-Builds | `%USERPROFILE%\ffmpeg\ffmpeg-n8.1-latest-win64-gpl-shared-8.1` |
| Flutter | 3.47.1 stable (la versione in `mise.toml`) | zip da flutter.dev | `D:\Programmi\flutter` |

## Variabili d'ambiente (utente)

| Variabile | Valore |
|---|---|
| `FFMPEG_LIBS_DIR` | `…\ffmpeg-n8.1-latest-win64-gpl-shared-8.1\lib` |
| `FFMPEG_INCLUDE_DIR` | `…\ffmpeg-n8.1-latest-win64-gpl-shared-8.1\include` |
| `LIBCLANG_PATH` | `D:\Programmi\LLVM\bin` |
| `Path` += | `…\ffmpeg-n8.1-latest-win64-gpl-shared-8.1\bin`, `D:\Programmi\flutter\bin` |

Sono le stesse che imposta `scripts\win-dev-env.ps1 -Persist`. Lo script però cerca LLVM solo in
`C:\Program Files\LLVM`: con LLVM su `D:` va impostato prima `LIBCLANG_PATH`.

## Due cose che servono su questo PC

- **Modalità sviluppatore** attiva (*Impostazioni ▸ Sistema ▸ Per sviluppatori*). Flutter collega
  i suoi plugin con collegamenti simbolici e senza si ferma con *"Building with plugins requires
  symlink support"*.
- **`ProgramFiles(x86)`** qui punta a `D:\Programmi`, ma `vswhere.exe`, che Flutter usa per
  trovare Visual Studio, resta in `C:\Program Files (x86)\Microsoft Visual Studio\Installer`.
  Flutter allora risponde *"Unable to find suitable Visual Studio toolchain"*. Va impostata solo
  nella finestra in cui si compila:

  ```
  ${env:ProgramFiles(x86)} = 'C:\Program Files (x86)'
  ```

## Comandi

Da `D:\Siceliot\Lumit`:

```
cargo test -p lumit-core --lib        # test del motore (crate core)
cargo build -p lumit_bridge           # la libreria che l'app carica
```

Da `D:\Siceliot\Lumit\flutter_ui`:

```
flutter test test/l10n/engine_labels_test.dart
flutter build windows --release       # l'app, in build\windows\x64\runner\Release
```

La prima compilazione del motore richiede circa 7 minuti su questo PC; le successive ricompilano
solo ciò che è cambiato.
