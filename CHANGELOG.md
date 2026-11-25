# Lodestar changelog

## 0.17.2 — 25 November 2026

### Fixes
- **No more second copies of games** added to Steam or the app menu by another launcher (Faugus, Heroic, Lutris):
  they showed up again after Lodestar finished looking up Steam game names.
- **The right pictures for new games**: a newly found game could show the pictures of a game you had removed. Lodestar
  tidies this up by itself, and games that got someone else's pictures fetch their own again.

### Themes
- **Delete your own themes** in the theme creator: the bin next to Edit in "Your themes".
- A new theme started from one of yours is called "My Northline 2" instead of "My My Northline".
- Settings → Appearance → **Get more themes** opens the theme list straight away.
- Imported Playnite themes with filter tabs show "LB Tabs" in the help bar, and their tabs no longer run under the
  theme's LB / RB pictures.

## 0.17.1 — 25 November 2026

### Updates
- **Lodestar updates itself** when you run it as an AppImage: it tells you when a new version is out, shows what's
  new, and installs it in one click (the download is checked before it replaces anything; your library and
  settings stay as they are). Updater apps such as Gear Lever can update it too.
- **You decide**: switch automatic checks off in Settings → General → Updates, check whenever you like with menu →
  Check for updates…, or skip a version you don't want.

### Steam
- **Works with the new Steam beta**: Steam games keep their details, and games you haven't installed show again.
- Steam games you haven't installed now show even when Steam's own files don't name them; Lodestar looks their names
  up in the background.
- With your Steam Web API key (Settings → Libraries → Steam, or Settings → Achievements), Lodestar lists every Steam
  game you own, including ones you never started on this PC.
- With several Steam accounts on one PC, Lodestar uses the account that owns your installed games.

### Dialogs
- **Every dialog has its buttons in the same place**: the main button at the bottom right next to Cancel, other
  actions on the left, with room under them. In their own windows the buttons are never cut off, at any size.

## 0.17.0 — 24 November 2026

**First public release (alpha).**

Lodestar is a game launcher for your whole library, from desk to couch, built for Linux.

### Your library
- Finds your games by itself: Steam (with non-Steam shortcuts), Heroic (Epic, GOG, Amazon), Lutris, Bottles,
  Faugus, itch, Minecraft launchers, Moonlight, store launchers inside Wine prefixes (Battle.net, EA app, Ubisoft
  Connect …), desktop entries, your ROM folders and your own RomM server.
- Desktop mode in the spirit of Playnite: sidebar, cover grid, list and details views, search, filters, grouping,
  collections, statistics and full game editing, with artwork and descriptions from many sources.
- Achievements: Steam achievements read from the Steam app's own files, and RetroAchievements with your own Web API
  key; counts on covers, in the list and in controller mode.

### Controller mode
- Fullscreen and driven entirely by a gamepad, with an in-game menu while you play.
- Built-in themes with Lodestar's own console artwork: Lodestar, Shelf, Classic, Lodestar Ambient, Fullscreen,
  Top Bar, Single Row, Meridian, Atlas and Northline.
- The theme creator: make your own theme on a live preview of your library, with colour schemes, variants,
  layouts for other screen shapes and per-console looks, and share it as a .zip. Making themes by hand: THEMES.md.
- Playnite themes can be imported.

### Playing
- Emulation: finds your emulators, recognises ROMs, checks BIOS files, and lets you pick the emulator per console
  or per game.
- Launch options and game options (Gamescope, MangoHud, GameMode …), and store launchers kept out of sight while
  their game runs.
