# Writing a Lodestar add-on

An add-on is a folder with an `addon.toml` and a program in any language. Lodestar never loads
add-on code into itself: it runs the program once per request, sends a JSON request on stdin and
reads one JSON reply from stdout. A broken or slow add-on can't crash or hang Lodestar. It is
killed when its time runs out.

The two add-ons here are complete examples in plain POSIX `sh` with no dependencies:

- `example-library/`: a **library** add-on. It lists two demo games that only wait two seconds.
- `example-metadata/`: a **metadata** add-on. It answers `search` and `fetch` with fixed details.

## Installing

In Lodestar, open Add-ons, choose **Install from folder…** (or a `.zip` of the folder), then
enable the add-on. Lodestar copies the folder into `~/.local/share/lodestar/addons/<id>/`. It
checks the manifest and never runs anything while installing. Library add-ons are asked for their
games on the next library scan.

Installing a `.zip` needs the `unzip` program. Without it, unpack the zip and choose the folder.

## `addon.toml`

```toml
id = "my-addon"            # a-z 0-9 - _ . (max 64). Also the folder name once installed.
name = "My Add-on"
author = "You"
version = "1.0.0"
description = "One sentence for the add-ons list."
kind = "library"           # library | metadata | generic
exec = "main.sh"           # relative to the add-on folder; must be executable (chmod +x)
timeout_secs = 30          # per request; 30 if left out, at most 600
permissions = ["network"]  # what it needs, shown before the add-on is enabled

[[settings]]               # rendered by Lodestar on the add-on's settings page
key = "api_key"
label = "API key"
kind = "secret"            # text | secret | folder | toggle
default = ""
help = "Where to find it."
```

`exec` must stay inside the add-on folder, symlinks included. An installed add-on whose folder name
doesn't match its id is shown with an error and never run. Installing an add-on with an id that is
already installed only works as an update, which needs the same `name` and `author`. Otherwise,
remove the old add-on first.

## Requests

Lodestar runs `<exec> <command>` with the add-on folder as the working directory and these
environment variables:

| Variable | Value |
| --- | --- |
| `LODESTAR_ADDON_ID` | the add-on id |
| `LODESTAR_ADDON_DIR` | the add-on folder |
| `LODESTAR_ADDON_DATA` | a writable folder for your own files: caches, tokens, downloaded art. It is kept when the add-on is updated or removed. |
| `LODESTAR_ADDON_SETTINGS` | the settings as a JSON object of strings (`"true"` / `"false"` for toggles), with your defaults filled in |
| `LODESTAR_ADDON_COMMAND` | the command (same as `$1`) |
| `LODESTAR_VERSION` | Lodestar's version |

Reply with one JSON object on stdout. Write log messages to stderr. Any reply can be
`{"error": "message"}`, which Lodestar shows on the add-on's page. A non-zero exit status also
counts as a failure. Replies are limited to 8 MB.

### Library add-ons

`list` with request `{"include_uninstalled": bool}` returns:

```json
{"games": [{
  "id": "stable-id", "name": "Game", "installed": true,
  "install_dir": "/abs/path",
  "launch": {"program": "/usr/bin/foo", "args": ["--bar"], "cwd": "/abs", "env": {"K": "V"}},
  "cover": "art/cover.png", "hero": "/abs/hero.jpg", "logo": "file:///abs/logo.png", "icon": "...",
  "banner": "...",
  "playtime_secs": 3600, "last_played": 1767225600, "size_bytes": 123456789,
  "description": "...", "developer": "...", "publisher": "...", "genres": ["RPG"],
  "release_year": 2024, "platform": "linux"
}]}
```

- Only `id` and `name` are required. Keep `id` the same across scans, because Lodestar keys the
  game's playtime, tags and edits on it.
- `launch` can be `{"program", "args", "cwd", "env"}` or `{"url": "scheme://…"}`. A program
  with a `/` that isn't absolute (`./run.sh`) is resolved against the add-on folder. A bare
  name is looked up in `PATH`.
- If a game has no `launch`, Play runs your program as the game:
  `<exec> launch <id>`, with `LODESTAR_ADDON_LAUNCH=1` and nothing on stdin. Start the game with
  `exec` so Lodestar can track it and count playtime. Secret settings aren't passed in this
  mode, so keep what you need in `LODESTAR_ADDON_DATA`.
- Art paths can be absolute, `file://`, or relative to the add-on folder, and only files that
  exist are used. A library scan never downloads anything, so http(s) art URLs are ignored. To
  show your art, download it into `LODESTAR_ADDON_DATA` during `list` and return those paths.
  Otherwise Lodestar's own art lookup fills in missing art by name.
- If one library add-on fails, the other add-ons' games still update. If every enabled library
  add-on fails, the add-on games keep their previous state.

The other library commands:

- `launch` `{"id"}` returns `{"launch": {...}}` or `{"handled": true}` when you started it yourself.
- `install` / `uninstall` `{"id"}` return `{"ok": true}` or `{"error": "…"}`. Hand over to the
  store's own client; never delete game files without asking the user.

### Metadata add-ons

- `search` `{"name", "year"}` returns `{"results": [{"id", "name", "year"}]}`.
- `fetch` `{"id", "name"}` returns the same fields as a `list` entry: `description`, `developer`,
  `publisher`, `genres`, `release_year`, plus art (`cover`, `hero`, `logo`, `icon`, `banner`).
  Art can be paths or http(s) URLs.

Lodestar takes the result whose name matches the game's (ignoring case and punctuation; the year
decides between several). Without such a result the game is left alone. Lodestar asks metadata
add-ons for games the built-in sources (Steam store, SteamGridDB) left without a description or art.

## Testing by hand

```sh
cd example-library
printf '{}' | LODESTAR_ADDON_SETTINGS='{"prefix":"Demo"}' ./main.sh list
printf '{"name":"Celeste"}' | ../example-metadata/main.sh search
```
