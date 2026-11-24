#!/usr/bin/env bash
# Build dist/Lodestar-x86_64.AppImage from the release binary.
#
# Needs appimagetool (https://github.com/AppImage/appimagetool): on PATH, or pointed to with
# APPIMAGETOOL=/path/to/appimagetool-x86_64.AppImage. No FUSE needed (it is run extracted).
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
         "$APPDIR/usr/share/doc/lodestar"

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
ARCH=x86_64 "$TOOL" --appimage-extract-and-run --no-appstream "$APPDIR" "$OUT"
echo
echo "Built $OUT ($(du -h "$OUT" | cut -f1))"
echo "Needs glibc >= $(objdump -T target/release/lodestar 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1 | cut -d_ -f2) (Fedora 40+, Ubuntu 24.04+, Arch, …)"
