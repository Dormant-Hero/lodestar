#!/bin/sh
# Example Lodestar library add-on (POSIX sh, no dependencies).
#
# Lodestar runs:  main.sh <command>   with a JSON request on stdin and reads one JSON reply from
# stdout. Anything written to stderr goes to Lodestar's log. See ../README.md.
set -eu

cmd="${1:-}"

# Started as the game itself (a game listed without "launch"): Lodestar runs
# `main.sh launch <game id>` with LODESTAR_ADDON_LAUNCH=1 and no request on stdin.
# Start the game with `exec` so Lodestar tracks it and counts playtime.
if [ "${LODESTAR_ADDON_LAUNCH:-}" = "1" ]; then
    echo "example-library: starting ${2:-?}" >&2
    exec sh -c 'sleep 2'
fi

request=$(cat)

# A real add-on would use a JSON parser; for this example a sed pattern is enough.
json_str() { # json_str <key> <json>
    printf '%s' "$2" | sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p"
}

settings="${LODESTAR_ADDON_SETTINGS:-}"
prefix=$(json_str prefix "$settings")
[ -n "$prefix" ] || prefix="Example"
show_second=$(json_str show_second "$settings")

case "$cmd" in
list)
    printf '{"games":['
    # Game one: Lodestar runs the "launch" program itself.
    printf '{"id":"tiny-one","name":"%s Tiny Game","installed":true,' "$prefix"
    printf '"launch":{"program":"sh","args":["-c","sleep 2"]},'
    printf '"cover":"cover-one.png","developer":"Lodestar","publisher":"Lodestar",'
    printf '"genres":["Demo"],"release_year":2026,"platform":"linux",'
    printf '"description":"A demo game from the example add-on. It waits two seconds and exits."}'
    if [ "$show_second" != "false" ]; then
        # Game two: no "launch", so Lodestar starts it through this script (see the top).
        printf ',{"id":"tiny-two","name":"%s Tiny Puzzle","installed":true,' "$prefix"
        printf '"cover":"cover-two.png","developer":"Lodestar","genres":["Demo","Puzzle"],'
        printf '"release_year":2025,"playtime_secs":600,'
        printf '"description":"Started through the add-on itself (main.sh launch tiny-two)."}'
    fi
    printf ']}\n'
    ;;
launch)
    # Asked through the protocol: tell Lodestar what to run.
    printf '{"launch":{"program":"sh","args":["-c","sleep 2"]}}\n'
    ;;
install | uninstall)
    id=$(json_str id "$request")
    echo "example-library: $cmd $id (nothing to do)" >&2
    printf '{"ok":true}\n'
    ;;
*)
    printf '{"error":"unknown command: %s"}\n' "$cmd"
    exit 2
    ;;
esac
