#!/bin/sh
# Example Lodestar metadata add-on (POSIX sh, no dependencies). See ../README.md.
set -eu

cmd="${1:-}"
request=$(cat)

json_str() { # json_str <key> <json>
    printf '%s' "$2" | sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p"
}

case "$cmd" in
search)
    name=$(json_str name "$request")
    [ -n "$name" ] || name="Unknown game"
    printf '{"results":[{"id":"example-1","name":"%s","year":2026}]}\n' "$name"
    ;;
fetch)
    printf '{"description":"Details from the example metadata add-on.",'
    printf '"developer":"Example Studio","publisher":"Example Publishing",'
    printf '"genres":["Adventure","Demo"],"release_year":2026}\n'
    ;;
*)
    printf '{"error":"unknown command: %s"}\n' "$cmd"
    exit 2
    ;;
esac
