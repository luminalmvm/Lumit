#!/bin/sh
# Installs a built Lumit bundle for the current user (K-252): the app under
# ~/.local/lib/lumit, a launcher on the PATH, the desktop entry, the .lum,
# .lumfx and .lumtheme MIME types, and the icons (the brand SVGs install as scalable icons,
# so the desktop renders them at any size).
#
#   (cd flutter_ui && flutter build linux --release)
#   packaging/linux/install.sh
#
# PREFIX overrides ~/.local for a system install (needs the matching rights).
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
root="$here/../.."
bundle="$root/flutter_ui/build/linux/x64/release/bundle"
prefix="${PREFIX:-$HOME/.local}"

[ -x "$bundle/lumit_flutter" ] || {
    echo "No release bundle at $bundle — run: (cd flutter_ui && flutter build linux --release)" >&2
    exit 1
}

mkdir -p "$prefix/lib" "$prefix/bin" "$prefix/share/applications" \
    "$prefix/share/mime/packages" "$prefix/share/icons/hicolor/scalable/apps" \
    "$prefix/share/icons/hicolor/scalable/mimetypes"

rm -rf "$prefix/lib/lumit"
cp -r "$bundle" "$prefix/lib/lumit"
ln -sf "$prefix/lib/lumit/lumit_flutter" "$prefix/bin/lumit_flutter"

cp "$here/io.github.luminalmvm.Lumit.desktop" "$prefix/share/applications/"
cp "$here/lumit-mime.xml" "$prefix/share/mime/packages/"
cp "$root/assets/brand/lumit-mark.svg" \
   "$prefix/share/icons/hicolor/scalable/apps/lumit.svg"
cp "$root/assets/brand/lumit-project.svg" \
   "$prefix/share/icons/hicolor/scalable/mimetypes/application-x-lumit-project.svg"
cp "$root/assets/brand/lumit-preset.svg" \
   "$prefix/share/icons/hicolor/scalable/mimetypes/application-x-lumit-preset.svg"
cp "$root/assets/brand/lumit-theme.svg" \
   "$prefix/share/icons/hicolor/scalable/mimetypes/application-x-lumit-theme.svg"

command -v update-mime-database >/dev/null && update-mime-database "$prefix/share/mime" || true
command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$prefix/share/icons/hicolor" || true

echo "Installed to $prefix/lib/lumit (launcher: $prefix/bin/lumit_flutter)"
