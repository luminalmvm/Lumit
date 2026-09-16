#!/usr/bin/env python3
"""Lay a built payload out as an LFX bundle.

In plain terms
--------------

Cargo builds a shared library and leaves it in `target/`; Lumit looks for a
folder with a listing and one directory per architecture. This script is the
step between the two, and it is the whole of what the Rust example needs that
`cargo build` does not do:

    python3 scripts/stage-bundle.py \\
        --payload target/release/libsaturation.so \\
        --manifest examples/saturation/lfx.toml \\
        --name Saturation \\
        --out build/bundles

produces

    build/bundles/Saturation.lfx.bundle/
      Contents/
        lfx.toml
        linux-x86_64/Saturation.lfx

The architecture directory is worked out from the machine this runs on unless
`--arch` names one, because the answer belongs to the *target* rather than to
the builder: a cross build, or a release job laying out one bundle per
platform, passes it in. The seven names are LFX's own closed vocabulary, which
is what lets a scan tell a build for another CPU from a directory it has never
heard of.

The C and C++ examples need none of this - the CMakeLists lays them out as it
builds them.
"""

from __future__ import annotations

import argparse
import platform
import shutil
import sys
from pathlib import Path

#: Every architecture directory LFX names.
ARCH_DIRS = (
    "win-x86_64",
    "win-arm64",
    "macos-universal",
    "macos-arm64",
    "macos-x86_64",
    "linux-x86_64",
    "linux-aarch64",
)


def arch_of_this_machine() -> str:
    """The directory a payload built here belongs under."""
    machine = platform.machine().lower()
    arm = machine in ("arm64", "aarch64")
    if sys.platform == "win32":
        return "win-arm64" if arm else "win-x86_64"
    if sys.platform == "darwin":
        return "macos-arm64" if arm else "macos-x86_64"
    if sys.platform.startswith("linux"):
        return "linux-aarch64" if arm else "linux-x86_64"
    # LFX has no spelling for this machine, and guessing one would hand the
    # loader a library built for another operating system.
    raise SystemExit(f"no LFX architecture directory for {sys.platform}/{machine}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payload", required=True, type=Path, help="the built shared library")
    parser.add_argument("--manifest", required=True, type=Path, help="the bundle's lfx.toml")
    parser.add_argument("--name", required=True, help="the bundle's name, as a person reads it")
    parser.add_argument("--out", required=True, type=Path, help="where the bundle is laid out")
    parser.add_argument(
        "--arch",
        choices=ARCH_DIRS,
        default=None,
        help="the architecture directory; this machine's own by default",
    )
    arguments = parser.parse_args()

    if not arguments.payload.is_file():
        raise SystemExit(f"{arguments.payload} is not a file - has it been built?")
    if not arguments.manifest.is_file():
        raise SystemExit(f"{arguments.manifest} is not a file")

    arch = arguments.arch or arch_of_this_machine()
    bundle = arguments.out / f"{arguments.name}.lfx.bundle"
    contents = bundle / "Contents"
    payload_dir = contents / arch
    payload_dir.mkdir(parents=True, exist_ok=True)

    shutil.copyfile(arguments.manifest, contents / "lfx.toml")
    # `.lfx` on every platform: the extension is what a scan looks for, and the
    # file under it is an ordinary shared library.
    shutil.copyfile(arguments.payload, payload_dir / f"{arguments.name}.lfx")
    print(f"{bundle}")


if __name__ == "__main__":
    main()
