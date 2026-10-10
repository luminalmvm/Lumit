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
[![Licence: GPL v3](https://img.shields.io/badge/licence-GPLv3-blue)](LICENSE)

[官网](https://lumitlab.com) ·
[下载](https://lumitlab.com/download) ·
[文档](https://docs.lumitlab.com) ·
[版本发布](https://lumitlab.com/releases)

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

需要 Rust 稳定版（由 `rust-toolchain.toml` 固定）外加两个外部依赖：用于媒体处理的 **FFmpeg 8.x**，以及用于绑定生成器的 **LLVM 18**。FFmpeg 必须是 8，用 7 或 9 构建会直接报错停止。较新的 LLVM 会静默生成有问题的绑定，因此 Windows 和 Linux 都固定在 18。

<details>
<summary><b>Windows</b></summary>

在 `%USERPROFILE%\ffmpeg\`, 下解压[BtbN FFmpeg 8.1 shared/GPL build](https://github.com/BtbN/FFmpeg-Builds/releases)，然后运行:

```powershell
winget install LLVM.LLVM --version 18.1.8
. .\scripts\win-dev-env.ps1 -Persist
cargo test --workspace
```
</details>

<details>
<summary><b>macOS</b></summary>
Homebrew 里不带版本号的 `ffmpeg` 已经是 9.x，所以要安装 `ffmpeg@8`。它不会被链接到 `PATH`，因此需要在构建所用的 shell 里把构建指向它：

```sh
brew install ffmpeg@8
export FFMPEG_PKG_CONFIG_PATH="$(brew --prefix ffmpeg@8)/lib/pkgconfig"
export PATH="$(brew --prefix ffmpeg@8)/bin:$PATH"   # 测试用 ffmpeg 命令行生成媒体文件
cargo test --workspace
```

Xcode 自带的 libclang 在这里可以直接使用，不需要另外安装 LLVM。
</details>

<details>
<summary><b>Linux</b></summary>

Ubuntu 26.04 自带 FFmpeg 8，所以只需要安装软件包：

```sh
sudo apt install pkg-config libclang-18-dev libasound2-dev libgl-dev libegl-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libx11-dev libxcursor-dev libxi-dev \
  libxrandr-dev libxcb1-dev libwayland-dev \
  libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev \
  libavfilter-dev libavdevice-dev
```

其他发行版自带的 FFmpeg 版本都不对（Debian 13 和较旧的 Ubuntu 是 7，Arch 是 9），所以不要安装发行版的 FFmpeg 开发包，改为解压一个 [BtbN FFmpeg 8.1 shared/GPL build](https://github.com/BtbN/FFmpeg-Builds/releases)：

```sh
# Arch / Artix: sudo pacman -S pkgconf clang18 llvm18
mkdir -p ~/ffmpeg8
curl -fsSL https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-linux64-gpl-shared-8.1.tar.xz \
  | tar -xJ -C ~/ffmpeg8 --strip-components=1
# .pc 文件里写的是构建它的那台机器上的 prefix
sed -i "s|^prefix=.*|prefix=$HOME/ffmpeg8|" ~/ffmpeg8/lib/pkgconfig/*.pc
export PKG_CONFIG_PATH="$HOME/ffmpeg8/lib/pkgconfig:$PKG_CONFIG_PATH"
export LD_LIBRARY_PATH="$HOME/ffmpeg8/lib:$LD_LIBRARY_PATH"
export PATH="$HOME/ffmpeg8/bin:$PATH"
```

然后在构建所用的 shell 里把构建指向 LLVM 18，因为这些发行版默认的 `clang` 都更新：

```sh
export LIBCLANG_PATH=/usr/lib/llvm-18/lib          # Arch: /usr/lib/llvm18/lib
cargo test --workspace
```
</details>


用户界面部分在 [flutter_ui/](flutter_ui/) 并需要 Flutter SDK —
 [flutter_ui/README.md](flutter_ui/README.md). 逐步构建的说明见 [docs/GUIDE.md](docs/GUIDE.md)。

## 仓库结构

| | |
|---|---|
| [docs/GUIDE.md](docs/GUIDE.md) | 贡献者指南：每个 crate 的作用、代码必须遵守的规则，以及如何构建和运行。 |
| [docs/](docs/) | 术语表、架构、性能与工程规则，以及桥接约定。 |

引擎是在 `crates/`下的一个 Cargo 工作区; 界面在
`flutter_ui/`; 他们在 `crates/lumit-bridge`
([17-BRIDGE-CONTRACT.md](docs/17-BRIDGE-CONTRACT.md)) 桥接； `web/` 和 `web-docs/`
是网站内容 [lumitlab.com](lumitlab.com), 
不依赖仓库中的其他任何内容。

## 参与贡献

欢迎 Issue 和 PR

- [docs/01-GLOSSARY.md](docs/01-GLOSSARY.md) 对代码、界面文案和提交信息具有约束力，请务必使用其中规定的术语。
- 所有改动都需要带上测试，并且 CI 运行必须通过。

特别欢迎翻译者：界面已完全外部化，但目前还没有任何翻译。翻译工作在 [lumitlab.com](https://lumitlab.com) 的翻译页面上进行，不在此仓库内——这里唯一编辑的语言文件是英式英语原文，其余文件由该页面回传的内容写入。

## 许可证

[GPLv3](LICENSE) 所有衍生项目必须保持开源。
