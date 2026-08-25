#!/usr/bin/env python3
"""Reject the two Icon Composer settings that actool cannot compile (K-312).

In plain terms: the macOS application icon is not a picture but a small stack of
layers, described by `assets/brand/lumit-icon.icon/icon.json`, which Apple's
`actool` turns into the icon the Finder shows. Icon Composer 26 can write two
things into that file which `actool` then chokes on — not with a message saying
the setting is unsupported, but by crashing part-way through with

    Exception while running actool: *** -[__NSPlaceholderArray
    initWithObjects:count:]: attempt to insert nil object from objects[0]

which reads as a corrupt file and says nothing about the cause. Both are
authoring niceties rather than anything the icon needs:

  - a non-empty top-level ``features`` array (Icon Composer records which
    advanced features the document uses; any entry at all is enough to crash);
  - a ``specular`` written as a string, ``"inside"``, naming *where* the
    highlight sits, rather than the plain ``true``/``false`` that says whether
    there is one.

The refraction and glass settings themselves compile fine and are untouched.

This runs on Linux in seconds, which is the point: without it the same mistake
surfaces five minutes into the macOS build job, and only there.
"""

import json
import pathlib
import sys

ICON = pathlib.Path("assets/brand/lumit-icon.icon/icon.json")


def problems(doc: dict) -> list[str]:
    found = []
    if doc.get("features"):
        found.append(
            f'top-level "features" is {doc["features"]!r} — drop the key; the '
            f"features it names still work without being declared"
        )
    for group in doc.get("groups", []):
        specular = group.get("specular")
        if isinstance(specular, str):
            found.append(
                f'group {group.get("name", "?")!r} has "specular": {specular!r} '
                f"— write true or false instead"
            )
    return found


def main() -> int:
    if not ICON.exists():
        print(f"{ICON} is missing")
        return 1
    found = problems(json.loads(ICON.read_text()))
    if not found:
        return 0
    print(f"{ICON} uses Icon Composer settings actool cannot compile (K-312):")
    for line in found:
        print(f"  - {line}")
    print()
    print("Reopening the icon in Icon Composer and saving puts these back, so")
    print("check this file after any edit to the artwork.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
