#!/usr/bin/env bash
# Install Lodestar for the current user. No root needed, except for the optional
# login-screen session (--session), which asks for sudo.
#
#   ./install.sh               build (if needed) and install to ~/.local
#   ./install.sh --session     also add "Lodestar (Controller)" to the login screen session list
#   ./install.sh --autostart   start Lodestar in controller mode when you log in
#   ./install.sh --no-shortcut don't put a Lodestar icon on the desktop
#   ./install.sh --uninstall   remove everything this script installed (your library DB is kept)
set -euo pipefail
cd "$(dirname "$0")"

BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/scalable/apps"
AUTOSTART="$HOME/.config/autostart/lodestar-controller.desktop"
SESSION_FILE="/usr/share/wayland-sessions/lodestar.desktop"
SESSION_BIN="/usr/local/bin/lodestar-session"
DOC_DIR="$HOME/.local/share/doc/lodestar"

DESKTOP_DIR=$(xdg-user-dir DESKTOP 2>/dev/null || true)
[ -n "$DESKTOP_DIR" ] && [ "$DESKTOP_DIR" != "$HOME" ] || DESKTOP_DIR="$HOME/Desktop"
SHORTCUT="$DESKTOP_DIR/lodestar.desktop"
want_session=0; want_autostart=0; want_shortcut=1
for a in "$@"; do
  case "$a" in
    --session) want_session=1 ;;
    --autostart) want_autostart=1 ;;
    --no-shortcut) want_shortcut=0 ;;
    --uninstall)
      rm -f "$BIN_DIR/lodestar" "$APP_DIR/lodestar.desktop" "$APP_DIR/lodestar-controller.desktop" \
            "$ICON_DIR/lodestar.svg" "$AUTOSTART" "$SHORTCUT"
      rm -rf "$DOC_DIR"
      rm -f "$HOME"/.local/share/icons/hicolor/*/apps/lodestar.png
      if [ -e "$SESSION_FILE" ] || [ -e "$SESSION_BIN" ]; then
        echo "Removing login-screen session (needs sudo)…"
        sudo rm -f "$SESSION_FILE" "$SESSION_BIN"
      fi
      echo "Lodestar removed. Your library and settings are still in ~/.local/share/lodestar and ~/.config/lodestar."
      exit 0 ;;
    -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
    *) echo "unknown option $a"; exit 2 ;;
  esac
done

BIN="target/release/lodestar"
# Sources changed since the binary was built? (files only: a directory's mtime also changes when
# an editor drops a temp file into it)
stale=""
if [ -x "$BIN" ]; then
  stale=$(find src assets Cargo.toml Cargo.lock THIRD_PARTY_NOTICES.md CHANGELOG.md -type f -newer "$BIN" 2>/dev/null | head -1 || true)
fi
if [ ! -x "$BIN" ] || [ -n "$stale" ]; then
  if command -v cargo >/dev/null || [ -x "$HOME/.cargo/bin/cargo" ]; then
    echo "Building release binary…"
    PATH="$HOME/.cargo/bin:$PATH" cargo build --release
  elif [ ! -x "$BIN" ]; then
    echo "No built binary and no Rust toolchain. Build it first (in the dev container): cargo build --release"
    exit 1
  else
    echo "Warning: $stale is newer than $BIN and there is no Rust toolchain here;"
    echo "         installing the existing binary. Rebuild it (cargo build --release) to get the latest changes."
  fi
fi

# Escape a value for the right-hand side of a sed s|…|…| command.
sed_escape() { printf '%s' "$1" | sed -e 's/[\\&|]/\\&/g'; }
# The binary path as a desktop-entry Exec argument (quoted when it has unusual characters).
desktop_exec_arg() {
  case "$1" in
    *[!A-Za-z0-9_./+-]*)
      # Exec quoting rules: inside "…", escape " ` $ \ with a backslash; then the desktop-file
      # string escaping doubles every backslash.
      local q
      q=$(printf '%s' "$1" | sed -e 's/[\\"`$]/\\&/g' -e 's/\\/\\\\/g')
      printf '"%s"' "$q" ;;
    *) printf '%s' "$1" ;;
  esac
}
# The binary path as a POSIX shell word (for the session script).
shell_arg() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }
EXEC_BIN=$(sed_escape "$(desktop_exec_arg "$BIN_DIR/lodestar")")
SHELL_BIN=$(sed_escape "$(shell_arg "$BIN_DIR/lodestar")")

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
install -m 755 "$BIN" "$BIN_DIR/lodestar"
install -m 644 packaging/lodestar.svg "$ICON_DIR/lodestar.svg"
# Licences of everything Lodestar is built from (also in the app: menu → About Lodestar → Credits).
mkdir -p "$DOC_DIR"
install -m 644 LICENSE THIRD_PARTY_NOTICES.md assets/fonts/LodestarSymbols-LICENSE.txt "$DOC_DIR/"
for f in LICENSE LICENSE.md LICENSE-MIT LICENSE-APACHE COPYING; do
  if [ -f "$f" ]; then install -m 644 "$f" "$DOC_DIR/"; fi
done
sed "s|@BIN@|$EXEC_BIN|g" packaging/lodestar.desktop > "$APP_DIR/lodestar.desktop"
sed "s|@BIN@|$EXEC_BIN|g" packaging/lodestar-controller.desktop > "$APP_DIR/lodestar-controller.desktop"
"$BIN_DIR/lodestar" --export-icons "$HOME/.local/share/icons/hicolor" >/dev/null || true
if [ "$want_shortcut" = 1 ] && [ -d "$DESKTOP_DIR" ]; then
  install -m 755 "$APP_DIR/lodestar.desktop" "$SHORTCUT"
  # Plasma only runs desktop-folder launchers that are executable; GNOME also wants them trusted.
  command -v gio >/dev/null && gio set "$SHORTCUT" metadata::trusted true >/dev/null 2>&1 || true
fi
command -v update-desktop-database >/dev/null && update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true
for k in kbuildsycoca6 kbuildsycoca5; do command -v $k >/dev/null && { $k >/dev/null 2>&1 || true; break; }; done
echo "Installed $BIN_DIR/lodestar and menu entries (Lodestar, Lodestar Controller Mode)."

if [ "$want_autostart" = 1 ]; then
  mkdir -p "$(dirname "$AUTOSTART")"
  sed "s|@BIN@|$EXEC_BIN|g" packaging/lodestar-controller.desktop > "$AUTOSTART"
  echo "Autostart enabled: controller mode starts when you log in."
fi

if [ "$want_session" = 1 ]; then
  echo "Adding the 'Lodestar (Controller)' login session — needs sudo."
  tmp=$(mktemp)
  sed "s|@BIN@|$SHELL_BIN|g" packaging/lodestar-session > "$tmp"
  sudo install -m 755 "$tmp" "$SESSION_BIN"
  rm -f "$tmp"
  sudo install -m 644 packaging/lodestar-wayland-session.desktop "$SESSION_FILE"
  echo "Done. Pick 'Lodestar (Controller)' on the login screen to boot straight into controller mode."
  command -v gamescope >/dev/null || echo "Note: gamescope isn't installed; the session will run Lodestar without it (sudo pacman -S gamescope)."
fi
