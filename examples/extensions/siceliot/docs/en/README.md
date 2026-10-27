# SICELIOT — documentation (English)

SICELIOT adds Cavalry-style, non-destructive effects to [Lumit](https://github.com/luminalmvm/Lumit).
Its picture effects are listed under **Siceliot FX** in Lumit's Add-effect menu, Effects & presets
panel and node search; its drivers under **Siceliot Nodes**, only in the Node graph.

*Italiano: [../it/README.md](../it/README.md)*

| Page | What it covers |
|---|---|
| [engine-changes.md](engine-changes.md) | Every change made to Lumit's engine and interface, file by file |
| [building.md](building.md) | Setting up a Windows machine and building the modified Lumit |
| [effects.md](effects.md) | Siceliot FX vs Siceliot Nodes; every effect and node: parameters, an example of use, the Cavalry equivalent |
| [examples.md](examples.md) | Two example projects, Flood and Sketch |
| [../SICELIOT.md](../SICELIOT.md) | Everything on one page: every effect, every engine change, how to add SICELIOT to Lumit |

## Folders

| Folder | What it holds |
|---|---|
| `extensions\siceliot` | The SICELIOT LFX extension as Lumit installs it (generated shaders) |
| `extensions\src` | The shader sources and `build.sh`, which writes and checks the extension |
| `dist\siceliot.lfx` | The extension as one package, for *Settings ▸ Addons ▸ Extensions* |
| `tools\lfx-check` | Checks an extension as Lumit reads it and validates its WGSL with naga |
| `examples` | The example projects (Flood, Sketch), the script that builds Flood and one that renders them |
| `tests\render` | Scripts that render every effect with `lumit-cli` |
| `tests\copies`, `tests\drivers` | Scripts that check the drivers and per-copy Clone to points with `lumit-cli` |
| `patches` | The engine changes as `git format-patch` files, to apply on upstream Lumit |
| `docs/research` | Research notes on Cavalry vs Lumit, in English and Italian |
| `Lumit` | The Lumit source with the engine changes (not in this repository) |

## Conventions

- Everything shown inside Lumit is in **English**: effect names, parameter labels, the category,
  manifests and shader comments.
- Lumit's own house style is sentence case. The headings **Siceliot FX** (picture effects)
  and **Siceliot Nodes** (drivers, only in the Node graph) keep the proper name Siceliot in every
  language.
- The Lumit source lives in `D:\Siceliot\Lumit`. The work is on the Git branch
  `siceliot/lumit-0.10`, the fourteen engine changes on top of upstream Lumit 0.10.0; `main`
  stays identical to upstream Lumit.
