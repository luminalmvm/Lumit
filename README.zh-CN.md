[English](./README.md) | [中文](./README.zh-CN.md) |
<div align="center">

<a href="https://lumitlab.com">
<img src="assets/brand/lumit-mark.svg" alt="lumitlab.com" width="96">
</a>

# Lumit

**一个原生的Motion Graphics与合成软件**
给GMV制作者的免费并开源的剪辑软件，包含After Effects的专业合成功能与Vegas灵活的变速功能。

[![CI](https://github.com/luminalmvm/Lumit/actions/workflows/ci.yml/badge.svg)](https://github.com/luminalmvm/Lumit/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/luminalmvm/Lumit?sort=semver&label=release)](https://github.com/luminalmvm/Lumit/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/luminalmvm/Lumit/total?label=downloads)](https://github.com/luminalmvm/Lumit/releases)
[![Crowdin](https://badges.crowdin.net/lumit/localized.svg)](https://crowdin.com/project/lumit)
[![Licence: GPL v3](https://img.shields.io/badge/licence-GPLv3-blue)](LICENSE)

[官网](https://lumitlab.com) ·
[下载](https://lumitlab.com/download) ·
[文档](https://docs.lumitlab.com) ·
[版本发布](https://lumitlab.com/releases) ·
[路线图](docs/16-ROADMAP.md)

</div>

<!-- A screenshot of the editor goes here. -->

## 什么是Lumit

Lumit 希望整合After Effects与Vegas各自强大的功能，包含合成，线性剪辑以及时间重映射。在未来我们希望加入编辑者们熟悉的音频编辑与节点合成

Lumit 的初衷就是替代 After Effects。目标很单纯——做一个响应飞快的软件，不管你项目里堆了多少关键帧、叠了多少层，它都不会卡死。

我们坚持开源，这样任何人都能来为Lumit做出贡献。Lumit 还处于早期开发阶段，如果你发现了 bug 或者问题，或是你想要的新功能，请提 issue 或者PR。

## 为什么Lumit会存在？

这个软件源于我在 After Effects 里做 fragmovie 时的痛苦——大部分工作时间都花在了等预览上。最初它只面向 GMV 和蒙太奇制作者，但现在范围已经大大扩展，变成了一个功能完整的合成编辑器。

Lumit 还想为创作者提供几个目标，让它足以满足你的所有剪辑需求：
- **多种选项的时间重映射** 无论你喜欢 After Effects 的时间重映射，还是 Vegas 的速度曲线，你都可以按自己的需要更改默认图表视图，而序列图层允许你在单个图层内剪切和拼接片段，同时仍支持每个片段独立变速。
- **内置常用特效** 发光、运动模糊、摄像机抖动、RGB 分离、平滑缩放、带 LUT 加载器的调色、物理建模的镜头光晕等等。全部内置，无需任何外部插件。后续计划支持 OFX，以及我们自己的自定义插件和脚本。

## 安装

安装程序可以在 [lumitlab.com/download](https://lumitlab.com/download) 或 [最新发布](https://github.com/luminalmvm/Lumit/releases/latest) 找到。Lumit 可以自动检查更新并安装，也可以在你想要的时候手动安装。

## 构建

需要 Rust 稳定版（由 `rust-toolchain.toml` 固定）外加两个外部依赖：用于媒体处理的 **FFmpeg 7.x**，以及用于绑定生成器的 **LLVM 18**——较新的 LLVM 会静默生成有问题的绑定，因此所有平台都固定在 18。

<details>
<summary><b>Windows</b></summary>

在 `%USERPROFILE%\ffmpeg\`, 下解压[BtbN FFmpeg 7.1 shared/GPL build](https://github.com/BtbN/FFmpeg-Builds/releases)，然后运行:

```powershell
winget install LLVM.LLVM --version 18.1.8
. .\scripts\win-dev-env.ps1 -Persist
cargo test --workspace
```
</details>

<details>
<summary><b>macOS</b></summary>
运行：

```sh
brew install ffmpeg@7
# The formula is keg-only, so point the build at it (K-204):
export FFMPEG_PKG_CONFIG_PATH="$(brew --prefix ffmpeg@7)/lib/pkgconfig"
cargo test --workspace
```
</details>

<details>
<summary><b>Linux</b> (K-082)</summary>

这里不需要为 FFmpeg 设置环境变量——开发包会把 .pc 文件放在构建系统默认会查找的位置。

```sh
# Debian 13 / Ubuntu 24.10 及其更新版本
sudo apt install pkg-config clang libavcodec-dev libavformat-dev libavutil-dev \
  libswscale-dev libswresample-dev libavfilter-dev libavdevice-dev

# Arch / Artix — 不带版本号的 clang 是 LLVM 19+，会生成有问题的绑定
sudo pacman -S ffmpeg pkgconf clang18 llvm18
```

如果你的默认 clang 版本高于 18，在构建所用的 shell 里把构建指向 18：

```sh
export LIBCLANG_PATH=/usr/lib/llvm18/lib
# Debian/Ubuntu 上是 /usr/lib/llvm-18/lib
cargo test --workspace
```

要求 FFmpeg **7.x**; 仍在使用 FFmpeg 6 的发行版（包括 Ubuntu 24.04 LTS）需要先换到更新的版本，或自行编译 FFmpeg。
</details>


用户界面部分在 [flutter_ui/](flutter_ui/) 并需要 Flutter SDK —
 [flutter_ui/README.md](flutter_ui/README.md). 逐步构建的说明见 [docs/GUIDE.md](docs/GUIDE.md) §8.

## 仓库结构

| | |
|---|---|
| [docs/README.md](docs/README.md) | 索引——从这里开始。十八份带编号的规格说明，从愿景到路线图。 |
| [docs/GUIDE.md](docs/GUIDE.md) | 原本是写给不懂 Rust 的人看的简明英文指南，介绍每个 crate 的作用以及如何安全地修改代码。不过现在已经变成了一个大块头文件，有些部分完全可以忽略。如果你在找信息，我现在更推荐 [docs.lumitlab.com](docs.lumitlab.com) |
| [docs/02-DECISIONS.md](docs/02-DECISIONS.md) | 每个设计决策及其理由，只追加不修改。用来搜索，别从头读。 |
| [docs/impl/](docs/impl/) | 实现笔记。 |
| [docs/TODO.md](docs/TODO.md) | 接下来、稍后以及更远期要做的工作。 |

引擎是在 `crates/`下的一个 Cargo 工作区; 界面在
`flutter_ui/`; 他们在 `crates/lumit-bridge`
([17-BRIDGE-CONTRACT.md](docs/17-BRIDGE-CONTRACT.md)) 桥接； `web/` 和 `web-docs/`
是网站内容 [lumitlab.com](lumitlab.com), 
不依赖仓库中的其他任何内容。

## 参与贡献

欢迎 Issue 和 PR

- [docs/01-GLOSSARY.md](docs/01-GLOSSARY.md) 对代码、界面文案和提交信息具有约束力，请务必使用其中规定的术语。
- 所有改动都需要带上测试，并且 CI 运行必须通过。

特别欢迎翻译者：界面已完全外部化，但目前还没有任何翻译。翻译工作在 [Crowdin](https://crowdin.com/project/lumit) 上进行，不在此仓库内——这里唯一编辑的语言文件是英语。

## 许可证

[GPLv3](LICENSE) 所有衍生项目必须保持开源。
