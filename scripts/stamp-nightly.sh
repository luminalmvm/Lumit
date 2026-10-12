#!/usr/bin/env bash
# Turns this checkout into a nightly build, before it is built.
#
# In plain terms: a nightly is the same source as any other build with two
# things changed. Its version, so that it says which nightly it is and the
# updater can tell it from a stable release; and its icon, so that nobody
# mistakes it for one in the Dock. release.yml runs this on its runners; run it
# by hand to build a nightly locally, then `git checkout .` and delete
# flutter_ui/assets/brand/stable-icon.* to undo it.
#
#   scripts/stamp-nightly.sh 0.6.1-nightly.20261012.57
set -euo pipefail

version="${1:?usage: stamp-nightly.sh <version>}"
cd "$(dirname "$0")/.."

# Every crate takes its version from the workspace, and the boot line the
# updater reads is the bridge crate's. Cargo.lock follows on the next build.
sed -i.bak "s/^version = \".*\"/version = \"$version\"/" Cargo.toml
rm Cargo.toml.bak
grep -q "^version = \"$version\"" Cargo.toml

# The regular icon rides along as a picture, for whoever would rather keep it
# (flutter_ui/lib/state/app_icon.dart). Copied before it is replaced below.
cp assets/brand/nightly/stable-icon.png flutter_ui/assets/brand/stable-icon.png
cp flutter_ui/windows/runner/resources/app_icon.ico flutter_ui/assets/brand/stable-icon.ico

# The icon, in each of the three places a platform takes it from: the Icon
# Composer document Xcode compiles, the Windows .ico, and the SVG Linux installs.
rm -rf assets/brand/lumit-icon.icon
cp -R assets/brand/nightly/lumit-icon.icon assets/brand/lumit-icon.icon
cp assets/brand/nightly/app_icon.ico flutter_ui/windows/runner/resources/app_icon.ico
cp assets/brand/nightly/lumit-mark.svg assets/brand/lumit-mark.svg

echo "Stamped nightly $version"
