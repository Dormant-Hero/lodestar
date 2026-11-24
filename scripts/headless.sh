#!/usr/bin/env bash
# Run a command on an invisible virtual display (Xvfb + software OpenGL), so test windows never
# appear on the real desktop or take focus.
#   scripts/headless.sh ./target/debug/lodestar --desktop --screenshot out.png
set -euo pipefail
unset WAYLAND_DISPLAY WAYLAND_SOCKET
export XDG_SESSION_TYPE=x11 LIBGL_ALWAYS_SOFTWARE=1
exec xvfb-run -a -s "-screen 0 2560x1600x24 -nolisten tcp" "$@"
