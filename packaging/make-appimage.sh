#!/usr/bin/env bash
# Build dist/Lodestar-<version>-x86_64.AppImage (and its .zsync) from the release binary.
#
# Needs appimagetool (https://github.com/AppImage/appimagetool): on PATH, or pointed to with
# APPIMAGETOOL=/path/to/appimagetool-x86_64.AppImage. No FUSE needed (it is run extracted).
#
# Updates: the AppImage carries update information for AppImage updaters (Gear Lever,
# AppImageUpdate, appimaged …):
#   gh-releases-zsync|Dormant-Hero|lodestar|latest|Lodestar-*x86_64.AppImage.zsync
# and Lodestar itself asks GitHub's "latest release" for a newer version. Both only ever see the
# newest release that is NOT marked as a pre-release (or draft): publish releases as normal ones.
# Attach BOTH files this script prints at the end (the AppImage and its .zsync) to the release.
set -euo pipefail
cd "$(dirname "$0")/.."

TOOL="${APPIMAGETOOL:-$(command -v appimagetool || command -v appimagetool-x86_64.AppImage || true)}"
if [ -z "$TOOL" ] || [ ! -x "$TOOL" ]; then
  echo "appimagetool not found. Download it from https://github.com/AppImage/appimagetool/releases"
  echo "and run:  APPIMAGETOOL=/path/to/appimagetool-x86_64.AppImage $0"
  exit 1
fi

PATH="$HOME/.cargo/bin:$PATH" cargo build --release
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)

APPDIR=$(mktemp -d)/Lodestar.AppDir
trap 'rm -rf "$(dirname "$APPDIR")"' EXIT
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor" \
         "$APPDIR/usr/share/doc/lodestar" "$APPDIR/usr/share/metainfo"

install -m 755 target/release/lodestar "$APPDIR/usr/bin/lodestar"
# Licences of everything Lodestar is built from (also in the app: menu → About Lodestar → Credits).
install -m 644 LICENSE THIRD_PARTY_NOTICES.md assets/fonts/LodestarSymbols-LICENSE.txt "$APPDIR/usr/share/doc/lodestar/"
for f in LICENSE LICENSE.md LICENSE-MIT LICENSE-APACHE COPYING; do
  if [ -f "$f" ]; then install -m 644 "$f" "$APPDIR/usr/share/doc/lodestar/"; fi
done
target/release/lodestar --export-icons "$APPDIR/usr/share/icons/hicolor" >/dev/null
cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/lodestar.png" "$APPDIR/lodestar.png"
cp packaging/lodestar.svg "$APPDIR/lodestar.svg"
ln -s lodestar.png "$APPDIR/.DirIcon"

# Desktop entry: Exec is resolved by the AppImage runtime / integrators (Gear Lever etc.).
sed -e 's|@BIN@|lodestar|g' packaging/lodestar.desktop > "$APPDIR/lodestar.desktop"
cp "$APPDIR/lodestar.desktop" "$APPDIR/usr/share/applications/lodestar.desktop"
# AppStream metadata (name, description, screenshots, releases) for software centres and updaters.
METAINFO=packaging/io.github.Dormant_Hero.Lodestar.metainfo.xml
install -m 644 "$METAINFO" "$APPDIR/usr/share/metainfo/"
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$METAINFO"
elif command -v host-spawn >/dev/null && host-spawn sh -c 'command -v appstreamcli' >/dev/null 2>&1; then
  host-spawn appstreamcli validate --no-net --explain "$PWD/$METAINFO"
else
  echo "Note: appstreamcli not found; $METAINFO not validated."
fi
if ! grep -q "<release version=\"$VERSION\"" "$METAINFO"; then
  echo "Warning: $METAINFO has no <release version=\"$VERSION\">: add one (newest first)."
fi

# AppRun: run the binary directly. Don't touch LD_LIBRARY_PATH etc., so games launched from
# Lodestar get a clean environment (the binary needs only system libraries).
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/lodestar" "$@"
EOF
chmod 755 "$APPDIR/AppRun"

mkdir -p dist
OUT="dist/Lodestar-$VERSION-x86_64.AppImage"
UPDATE_INFO="gh-releases-zsync|Dormant-Hero|lodestar|latest|Lodestar-*x86_64.AppImage.zsync"
rm -f "$OUT" "$OUT.zsync"
# --no-appstream: validated above (appimagetool's own check wants appstreamcli inside this box).
# -u embeds the update information and writes $OUT.zsync next to the AppImage.
ARCH=x86_64 "$TOOL" --appimage-extract-and-run --no-appstream -u "$UPDATE_INFO" "$APPDIR" "$OUT"
# appimagetool writes the .zsync into the current folder on some versions.
[ -f "$OUT.zsync" ] || { [ -f "$(basename "$OUT").zsync" ] && mv "$(basename "$OUT").zsync" "$OUT.zsync"; } || true

# The newest glibc the binary really needs: version requirements marked WEAK (Rust's std looks
# for newer functions such as pidfd_spawnp at run time and does without them) don't count.
GLIBC=$(readelf -V target/release/lodestar 2>/dev/null | grep -E 'Name: GLIBC_[0-9.]+ +Flags: none' \
  | grep -oE 'GLIBC_[0-9.]+' | sort -uV | tail -1 | cut -d_ -f2)
echo
echo "Built $OUT ($(du -h "$OUT" | cut -f1))"
case "$GLIBC" in
  2.3[0-5] | 2.[12][0-9]) DISTROS="Ubuntu 22.04+, Linux Mint 21+, Debian 12+, Fedora 36+, Arch, …" ;;
  2.3[6-9] | 2.4[0-9]) DISTROS="newer than Ubuntu 22.04 / Mint 21 have: see why with readelf -V" ;;
  *) DISTROS="check which distributions have it" ;;
esac
echo "Needs glibc >= ${GLIBC:-?} ($DISTROS)"
echo
echo "Attach both files to the GitHub release (publish it as a normal release, not a pre-release):"
echo "  $OUT"
if [ -f "$OUT.zsync" ]; then
  echo "  $OUT.zsync"
else
  echo "  (no .zsync was written: AppImage updaters will fall back to downloading the whole file)"
fi
