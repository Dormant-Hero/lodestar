#!/bin/sh
# Test fixture: replies with reply.json for `list`; `env` echoes what Lodestar passed.
cd "$(dirname "$0")/.." || exit 1
case "$1" in
list) cat reply.json ;;
env)
    req=$(cat)
    printf '{"id":"%s","data":"%s","dir":"%s","cmd":"%s","version":"%s","pwd":"%s","settings":%s,"request":%s}\n' \
        "$LODESTAR_ADDON_ID" "$LODESTAR_ADDON_DATA" "$LODESTAR_ADDON_DIR" "$LODESTAR_ADDON_COMMAND" \
        "$LODESTAR_VERSION" "$(pwd)" "$LODESTAR_ADDON_SETTINGS" "$req"
    ;;
*) printf '{"error":"unknown command %s"}\n' "$1"; exit 2 ;;
esac
