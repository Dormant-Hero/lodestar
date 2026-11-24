# Lodestar architecture

A map of the code for developers. It covers how the program is put together. **THEMES.md** explains the theme
format, and **CHANGELOG.md** describes what users see.

Lodestar is one Rust binary using egui/eframe with the glow (OpenGL) backend. Its only system library
dependencies are libc, libudev and libcap. Videos and sounds are handled by other programs running as separate
processes (`ffmpeg`, `pw-cat` / `pacat` / `aplay`), never by linked libraries. Nothing else is required at run
time.

## The big picture

```
                 ┌──────────────────────── UI thread (eframe) ─────────────────────────┐
  gamepad ──▶ input (pad thread) ──▶ Nav events ──▶ app::App::update (one frame)
  keyboard ─────────────────────────────────────▶        │
                                                          ├─ ui::desktop    (Playnite-style window)
                                                          └─ ui::controller (fullscreen, themed)
                                                                 │ reads AppState, calls its methods
                 └────────────────────────────────────────────────────────────────────┘
                                         │ channels (crossbeam), request_repaint()
        ┌──────────────┬─────────────────┼────────────────┬──────────────┬─────────────────┐
     scan::run_all   launch (spawn +   images (decode   art / metadata   theme jobs      media
     (one thread     track threads)    workers + disk   workers (HTTP,   (convert /      (ffmpeg,
     per source)                       thumbnails)      throttled)       import)         audio player)
        │                                                     │
        └────────────────────▶ db (SQLite library.db) ◀───────┘
```

* **One UI thread.** It draws a frame only when something changed: input, a finished background job (every
  worker calls `ctx.request_repaint()` when it posts a result), or a running animation. An idle window draws
  nothing (checked with `LODESTAR_IDLE_PROBE`).
* **Never block the UI thread.** Disk walks, HTTP, process spawning, image decoding and theme imports all run
  on named background threads (`thread::Builder::new().name(…)`). They report back over `crossbeam_channel`,
  and `AppState` polls those channels once per frame.
* **`AppState` (app.rs) is the only way to change anything.** The UIs read `st.library`, `st.config` and
  `st.images` and act only through `AppState` methods, which keep the database, the in-memory library and
  `config.toml` in step. The contract is written at the top of app.rs.

## Source map

The code is organised by area. Larger areas are modules made of several files. For a file split from a parent,
the module path is the parent's name followed by the part (for example `launch::track`).

| Area | Modules | What it does |
|---|---|---|
| Entry point | `main`, `cli` | `main` starts the GUI; `cli` handles every command-line option (`--help`, `--scan`, theme import, headless test runs) |
| App core | `app`, `*_app` | `AppState`, the frame loop, polling background work; `edit_app`, `art_app`, `romm_app` … are the `AppState` methods for one area each |
| Data model | `model`, `library`, `library_collections`, `db`, `search` | `Game`, `GameId`, `Source`, metadata; the in-memory library with its collections and filtered views; the SQLite store; fuzzy search |
| Configuration | `config`, `config_menu_button` | `~/.config/lodestar/config.toml` (serde, lenient: an invalid value drops only itself) and the standard paths (`Paths`, `LODESTAR_HOME`) |
| Finding games | `scan::*` | One scanner per library (Steam, Heroic, Lutris, Bottles, Faugus, itch, Minecraft, Moonlight, Wine-prefix stores, desktop entries, ROM folders, RomM, add-ons) |
| Emulation | `emulation::*`, `scan::emulation*` | Consoles, built-in emulator definitions, detecting emulators, BIOS / firmware, several ROMs per game, the ROM hash database |
| Launching | `launch::*`, `client_launch`, `game_options*` | Resolving a game into a command, spawning it, tracking the session through `/proc`, keeping store clients out of sight (`launch::hygiene`), scripts; game options turn switches into environment variables and wrappers |
| Artwork and metadata | `art*`, `metadata*`, `igdb`, `images*` | Artwork sources and the artwork engine, online metadata (Steam, IGDB, SteamGridDB, Wikipedia, ProtonDB), and the image cache (decode workers, disk thumbnails, GPU texture LRU) |
| Achievements | `achievements::*`, `ui::achievements` | `Hub`: game pages' answers (one worker; Steam from the client's files, RetroAchievements' Web API) and the library's counts (`summary`: a niced background pass over the whole library, page answers, `summary.json`); the UI reads `Hub::counts()` |
| Achievements | `achievements::*` | Steam achievements from the Steam client's files, RetroAchievements from its public Web API (the user's username + Web API key), a worker (`Hub`) with per-game answers and library counts; emulators' RetroAchievements sign-in is only read, never written |
| Themes | `theme::model`, `theme::render::*` | The Lodestar theme format (`theme.toml`) and the renderer that draws a theme's views with data bindings |
| Theme creator | `ui::desktop::theme_creator::*`, `theme::model::{write, schemes}` | The visual theme editor (own window): start / templates, live preview with drag and resize, inspector, theme pages, undo, autosave, .zip export / import; the tidy `theme.toml` writer and colour schemes |
| Console artwork | `theme::console_art`, `tools/gen-console-art.py` | Lodestar's own picture set for every console, store and built-in collection (card, wide background, wordmark, device): SVGs drawn by the generator into `assets/consoles/`, embedded compressed, written once to the cache, reached by themes through `{collection.art.*}` / `{game.consoleart.*}` |
| Theme import | `theme::playnite::*`, `theme::catalog::*` | Playnite theme importer (XAML → resource tree → look → layout / view compiler), Playnite's community theme list (browse, install, updates) |
| Media | `media::*` | Theme videos (one `ffmpeg` process per video), navigation sounds and music (a pure-Rust mixer feeding one player process), install hints for missing programs |
| Input | `input`, `input::pad`, `input::gate`, `input::style` | Gamepad → `Nav` events, the menu-button combination, the focus gate (see "Controller safety rule"), button glyph style |
| Desktop UI | `ui::desktop::*` | Top bar, explorer sidebar, grid / list / details views, dialogs, Settings, Add-ons, editors |
| Controller UI | `ui::controller::*` | Collections → Games → Details through the theme renderer, side menus, on-screen keyboard, the in-game menu |
| Desktop integration | `tray`, `tray_app`, `winctl`, `launch::winhide*`, `instance`, `main_window` | Tray icon (StatusNotifierItem over D-Bus), hiding windows, one instance per library, remembering the window's size and position |
| Other sources | `cloud*`, `romm*`, `addons::*` | Cloud gaming entries, the user's RomM server, external add-ons (one process per call, JSON on stdin / stdout, never loaded in-process) |

Tests sit next to the code: either `#[cfg(test)] mod tests` at the end of a file, or a sibling `*_tests.rs`
file for large suites. Test fixtures live in `tests/fixtures/`.

## Key flows

**Start-up.** `main` (with `cli`) reads the options and the config, then opens `library.db`. It shows the library from the
database immediately, then runs `scan::run_all` in the background. Each scanner returns `ScannedGame`s. `db`
merges them with what the user edited (a user edit always wins over the scanner), and `AppState` swaps in the
new `Library`. Scanner rules are written at the top of `scan/mod.rs`: never panic, never go online, and return
`Err` only when a launcher's data exists but can't be read, so the previous state of that source is kept.

**Launching a game.** `launch::resolve(&Game)` is pure: it applies the user's launch options to the source's
command and returns what would run, which is also what the editor previews. `Launcher::launch` spawns the
command on its own thread and returns at once. `launch::track` follows the game's processes through `/proc`, and
`launch::hygiene` minimises or closes the store client that started it. Events reach the UI through
`Launcher::poll`.

**Drawing a controller theme.** A `Theme` is plain data (`theme::model`): three views, each a list of
positioned elements whose strings carry bindings such as `{game.name}` and `{art.cover}`. `theme::render`
resolves the bindings for the current selection and draws the elements. Images go through `images` and are
loaded in advance for neighbouring items (`render::prefetch`). The Playnite importer never draws anything: it
translates Playnite XAML into this same format, so all themes share one renderer.

**Importing a Playnite theme.** The importer works in stages:

* `source` opens the theme folder or `.pthm` package.
* `xaml` and `tree` lay its files over Playnite's own default theme.
* `extract` produces a `PlayniteLook` (colours, fonts, brushes).
* For fullscreen themes, `compile` translates the Main and GameDetails views, measuring them with `layout` (WPF
  measure / arrange). If that isn't possible, `fill` lays the look onto a Lodestar layout.
* `report` explains in plain words what came across.

The import runs on the `theme-job` thread.

## Controller safety rule

While a game runs and no Lodestar window is focused, only the menu button gets through, and only to open the
menu. When a Lodestar window is focused, only the in-game menu's own buttons get through. A hidden or minimised
window never counts as focused, and an unknown focus state fails safe. The rule lives in `input/gate.rs`, the
combination detection in `input/pad.rs`. Both have regression tests that must keep passing. Change them only
together with those tests.

## Conventions

* **Errors.** `anyhow::Result` at the edges (I/O, processes, network) with `.context(…)` saying what was being
  done. Typed enums where a caller branches on the kind of failure. Every user-visible failure becomes plain
  words in the UI, never a panic. Expected absences, such as a launcher that isn't installed, are `Ok(empty)`,
  not errors.
* **Threads.** Named threads, channels back to the UI thread, `request_repaint()` after posting. Shared state
  uses `parking_lot` locks held briefly. Never hold a lock across I/O on the UI thread.
* **Files.** Files Lodestar keeps (config, caches, shortcuts) are written with `fsutil::write_atomic` (temporary
  file, fsync, rename); the library itself is SQLite in WAL mode.
  Removing a theme or add-on checks that the path is inside Lodestar's own folder first.
* **No new system libraries.** `ldd target/release/lodestar` must list only libudev, libgcc_s, libm, libc,
  libcap and the loader. Pure-Rust crates are fine. After a dependency change, run
  `python3 -I tools/gen-third-party-notices.py`.
* **Wording.** UI text is plain English for players, with no jargon, and offers a way forward wherever something
  is missing.

## Building and testing

```bash
cargo build --release
cargo test --release            # unit and integration tests, about 10 s
cargo clippy --release --all-targets   # kept at zero warnings
```

The GUI can be checked headless without opening anything on the desktop:

```bash
LODESTAR_HOME=/tmp/test-home ./target/release/lodestar --no-scan --screenshot /tmp/shot.png
```

The binary re-executes itself under `xvfb-run`, so no window appears on screen. Useful extras:

* `--controller --inject right*3,accept` drives controller mode.
* `LODESTAR_DESKTOP_OPEN=settings` opens a dialog first, and `LODESTAR_DESKTOP_KEYS=click:x:y,text:abc` sends
  pointer and keyboard events.
* `LODESTAR_FRAME_STATS=1`, `LODESTAR_LATENCY_PROBE=1` and `LODESTAR_IDLE_PROBE=5` measure performance.

`lodestar --help` lists the rest. `LODESTAR_HOME` keeps the config, library and caches in one folder, so a test
never touches a real setup.

Packaging: `./install.sh` installs into `~/.local`, and `./packaging/make-appimage.sh` builds an AppImage.
