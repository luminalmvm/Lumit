#!/bin/sh
# Builds the macOS disk image (K-252): a release build, the Homebrew FFmpeg
# dylibs bundled INTO the .app (so the image runs on machines without
# Homebrew), a re-sign, then a DMG laid out by create-dmg: white
# background (dmg-background.png), the app on the left, an Applications
# shortcut on the right, a curved arrow between them. macOS only.
#
#   packaging/macos/make-dmg.sh [version]
#
# Needs: flutter, rust, and `brew install ffmpeg@7 dylibbundler create-dmg`
# (create-dmg optional - without it the image has no drag-to-Applications
# window dressing).
#
# The bundling is the same move the Windows installer makes with the FFmpeg
# DLLs: the bridge links the shared FFmpeg, so the libraries must travel with
# the app. dylibbundler copies every Homebrew-linked dylib (transitive deps
# included) into Contents/Frameworks and rewrites the install names; the
# codesign afterwards is mandatory - macOS kills a process whose binaries
# changed after signing.
#
# Signing and notarisation are opt-in through the environment (K-309), so this
# script does the same thing on a laptop as it does under CI minus the parts a
# certificate pays for:
#
#   MACOS_SIGN_IDENTITY   the Developer ID identity to sign with. Unset means
#                         an ad-hoc signature: enough to run locally, still
#                         "unsigned" as far as Gatekeeper is concerned.
#   APPLE_API_KEY_PATH    App Store Connect .p8 key. Set (with APPLE_API_KEY_ID
#   APPLE_API_KEY_ID      and APPLE_API_ISSUER_ID) to notarise and staple the
#   APPLE_API_ISSUER_ID   .app and the .dmg. Unset skips both.
#
# release.yml sets all four from repository secrets; nothing here needs them.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
root="$here/../.."
version="${1:-$(sed -n 's/^version: *\([0-9.]*\).*/\1/p' "$root/flutter_ui/pubspec.yaml")}"
arch="$(uname -m)"

command -v dylibbundler >/dev/null || {
    echo "dylibbundler not found - brew install dylibbundler" >&2
    exit 1
}
ffprefix="$(brew --prefix ffmpeg@7 2>/dev/null)" || {
    echo "ffmpeg@7 not found - brew install ffmpeg@7" >&2
    exit 1
}

# Flutter forces a universal build on the xcodebuild command line
# (ARCHS="arm64 x86_64", ONLY_ACTIVE_ARCH=NO), which out-ranks every project
# and Podfile setting — cargokit then cross-compiles the bridge for the
# architecture Homebrew has no FFmpeg for, and rusty_ffmpeg's pkg-config
# probe dies. FLUTTER_XCODE_* variables are Flutter's own escape hatch:
# they are appended as build settings AFTER Flutter's, and the last ARCHS
# wins. One architecture per machine until K-033 takes on universal builds.
FLUTTER_XCODE_ARCHS="$arch"
export FLUTTER_XCODE_ARCHS

(cd "$root/flutter_ui" && flutter build macos --release)

app="$root/flutter_ui/build/macos/Build/Products/Release/Lumit.app"
[ -d "$app" ] || { echo "No app at $app" >&2; exit 1; }

# Every Mach-O in the app that still links a Homebrew path gets handed to
# dylibbundler. The bridge dylib is the expected hit; the loop rather than a
# hardcoded path so a renamed framework cannot silently ship keg links.
fixups=""
fixlist=""
for bin in $(find "$app/Contents" -type f ! -name "*.png" ! -name "*.json" ! -name "*.plist"); do
    if otool -L "$bin" 2>/dev/null | tail -n +2 | grep -Eq '/opt/homebrew/|/usr/local/(opt|Cellar)/'; then
        fixups="$fixups -x $bin"
        fixlist="$fixlist $bin"
    fi
done
if [ -n "$fixups" ]; then
    # -cd creates the destination if missing; -of overwrites dylibs a
    # previous run already copied, so reruns are idempotent. NEVER -od here —
    # that flag OVERWRITES the directory, i.e. deletes Contents/Frameworks
    # and every framework already embedded in it.
    # shellcheck disable=SC2086 # word-splitting the -x list is the point
    dylibbundler -cd -of -b $fixups \
        -d "$app/Contents/Frameworks/" \
        -p "@executable_path/../Frameworks/" \
        -s "$ffprefix/lib"
else
    echo "warning: nothing in the app links Homebrew - is the media feature on?" >&2
fi

# Homebrew's libSDL2 is the sdl2-compat shim: its module initializer dlopens
# SDL3, and when that fails the process aborts before main() ever runs — and
# a dlopen is invisible to otool, so no bundler can ship its target. Nothing
# in Lumit calls SDL; it rides in through libavdevice's output devices, and
# libavdevice cannot leave (the bridge imports its version symbols). So the
# bundled libSDL2 is replaced with a generated stub: a dylib exporting no-op
# versions of exactly the symbols the other bundled binaries import from it.
# No initializer, no SDL3, no abort.
sdl="$(find "$app/Contents/Frameworks" -maxdepth 1 -name 'libSDL2*.dylib' | head -1)"
if [ -n "$sdl" ]; then
    sdlbase="$(basename "$sdl")"
    stub="$(mktemp -d)"
    # nm -m names the source library of every undefined symbol under the
    # two-level namespace, so this is the exact import list to satisfy.
    find "$app/Contents/Frameworks" -maxdepth 1 -type f ! -name "$sdlbase" \
        -exec nm -m {} + 2>/dev/null \
        | awk '/from libSDL2/{print $(NF-2)}' | sort -u > "$stub/syms"
    if [ -s "$stub/syms" ]; then
        while read -r s; do
            printf 'void %s(void) {}\n' "${s#_}"
        done < "$stub/syms" > "$stub/stub.c"
        # dyld checks the compatibility version the importers were linked
        # against; carry the real library's numbers over to the stub.
        line="$(otool -L "$app/Contents/Frameworks/"libavdevice.*.dylib 2>/dev/null | grep "$sdlbase" | head -1)"
        compat="$(printf '%s' "$line" | sed -n 's/.*compatibility version \([0-9.]*\).*/\1/p')"
        curr="$(printf '%s' "$line" | sed -n 's/.*current version \([0-9.]*\).*/\1/p')"
        cc -dynamiclib -o "$sdl" "$stub/stub.c" \
            -install_name "@executable_path/../Frameworks/$sdlbase" \
            -compatibility_version "${compat:-1.0}" \
            -current_version "${curr:-1.0}"
        echo "Stubbed $sdlbase ($(wc -l < "$stub/syms" | tr -d ' ') symbols)"
    else
        # nothing imports from it: it has no reason to ship at all
        rm -f "$sdl"
    fi
    rm -rf "$stub"
fi

# dyld (macOS 15+) refuses to launch a binary carrying duplicate LC_RPATH
# entries, and dylibbundler adds its -p path as an rpath on each binary it
# fixes — on top of Xcode's default '@executable_path/../Frameworks'. Every
# binary in the app gets the sweep (a rerun of this script must clean marks
# the previous run left, so the set cannot be limited to freshly-fixed
# files): drop the slashed twin outright, collapse exact duplicates to one.
for bin in "$app/Contents/MacOS/Lumit" $(find "$app/Contents/Frameworks" -type f); do
    while install_name_tool -delete_rpath "@executable_path/../Frameworks/" "$bin" 2>/dev/null; do :; done
    for rp in $(otool -l "$bin" 2>/dev/null | awk '$1=="path"{print $2}' | sort | uniq -d); do
        while install_name_tool -delete_rpath "$rp" "$bin" 2>/dev/null; do :; done
        install_name_tool -add_rpath "$rp" "$bin"
    done
done

# Re-sign everything; the install-name rewrites above invalidated the
# signatures the build produced.
#
# NEVER --deep here. It walks nested *bundles* and is unreliable for the loose
# dylibs dylibbundler just copied into Contents/Frameworks — notarisation then
# rejects the whole app, naming the one binary it missed, twenty minutes after
# the tag. Sign the contents explicitly, innermost first: a signature covers
# what is inside the bundle, so sealing the app before its frameworks records a
# hash that the next command changes.
if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    identity="$MACOS_SIGN_IDENTITY"
    # --options runtime is the hardened runtime, which notarisation requires;
    # --timestamp has Apple countersign, so signatures outlive the certificate
    # that made them. An ad-hoc signature can carry neither.
    signopts="--options runtime --timestamp"
else
    identity="-"
    signopts=""
fi
for f in "$app/Contents/Frameworks/"*; do
    [ -e "$f" ] || continue
    # shellcheck disable=SC2086 # $signopts is a flag list, not a filename
    codesign --force $signopts --sign "$identity" "$f"
done
# Re-signing drops whatever entitlements the build applied, so they are named
# again here rather than silently lost.
# shellcheck disable=SC2086 # as above
codesign --force $signopts --sign "$identity" \
    --entitlements "$root/flutter_ui/macos/Runner/Release.entitlements" "$app"

# Notarisation: Apple scans the artefact and, if it is happy, issues a ticket;
# `stapler` attaches that ticket to the file so a machine that has never been
# online can still see it. A ticket covers exactly what was submitted, which is
# why this happens twice — once for the .app the updater downloads as a bare
# .zip (K-297), once for the .dmg below.
notarise() {
    xcrun notarytool submit "$1" --wait \
        --key "$APPLE_API_KEY_PATH" \
        --key-id "$APPLE_API_KEY_ID" \
        --issuer "$APPLE_API_ISSUER_ID"
}
if [ -n "${APPLE_API_KEY_PATH:-}" ]; then
    notarydir="$(mktemp -d)"
    # ditto, not zip: an .app carries symlinks, signatures and executable bits,
    # and an archiver that drops those submits a bundle macOS will not open.
    ditto -c -k --keepParent "$app" "$notarydir/Lumit.zip"
    notarise "$notarydir/Lumit.zip"
    xcrun stapler staple "$app"
    rm -rf "$notarydir"
fi

# create-dmg copies the CONTENTS of its source folder, so the app goes into a
# staging folder first. The icon coordinates are centres in a 660x400 window,
# matching the arrow baked into dmg-background.png.
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
# ditto rather than cp -R: the stapled ticket and the signature travel as
# metadata cp is not obliged to carry, and a DMG built from a copy that lost
# them ships an app Gatekeeper treats as unnotarised.
ditto "$app" "$stage/Lumit.app"

# A signature is easy to invalidate and hard to notice, so prove it survived
# everything above rather than assume: --strict verifies the seal, and spctl
# asks Gatekeeper the same question a user's Mac asks on first launch. spctl
# only has an answer once the app is notarised, so it is gated; codesign is
# checked either way, ad-hoc included.
codesign --verify --strict "$stage/Lumit.app"
if [ -n "${APPLE_API_KEY_PATH:-}" ]; then
    spctl --assess --type exec -vv "$stage/Lumit.app"
fi

mkdir -p "$here/dist"
out="$here/dist/lumit-$version-macos-$arch.dmg"
rm -f "$out"
if command -v create-dmg >/dev/null; then
    # The proper drag-into-Applications window (brew install create-dmg).
    create-dmg --volname "Lumit" \
        --background "$here/dmg-background.png" \
        --window-size 660 400 --icon-size 128 \
        --icon "Lumit.app" 165 190 --app-drop-link 495 190 \
        --hide-extension "Lumit.app" \
        "$out" "$stage"
else
    # Plain image: app + Applications shortcut, no window dressing.
    ln -s /Applications "$stage/Applications"
    hdiutil create -volname "Lumit" -srcfolder "$stage" -ov -format UDZO "$out"
fi

# The image needs its own ticket: stapling the app inside it does not vouch for
# the disk image a user actually double-clicks first.
if [ -n "${APPLE_API_KEY_PATH:-}" ]; then
    notarise "$out"
    xcrun stapler staple "$out"
fi
echo "Wrote $out"
