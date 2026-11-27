# Lodestar changelog

## 0.18.1 — 27 November 2026

### Game pages
- **Just the background at the top of game pages**: Settings → Appearance → Game page → "Show the logo or name over
  the background" turns off the game's logo, or its name for games without one, on game pages in the Details view.
- **Hide it for one game**: in Arrange mode (the four arrows on a game's page), **Hide logo** leaves that game's logo
  off its page. **Show logo** or **Reset logo** brings it back.

### Fixes
- **The controller works in desktop mode again after Lodestar starts Steam.** When a Steam game launched from
  Lodestar also had to start Steam, Steam kept counting as a running game, so the controller stayed off in desktop
  mode until Steam was restarted.
- Steam Remote Play is now treated as game streaming, so the controller waits while you stream.

## 0.18.0 — 25 November 2026

### Controller
- **Use a controller in desktop mode**: move through the library and the sidebar with the D-pad or left stick, A to
  play or press, B to go back, X for the game's menu, Y to search, Start for the main menu, View to switch between
  Details, Grid and List, LB / RB for All, Installed, Favourites and Recent, and the triggers to jump a page. Menus,
  dialogs and Settings work too, with a clear highlight showing where you are. It only responds while Lodestar is in
  front and no game is running, so presses meant for a game never reach it. Turn it off in Settings → Controller →
  "Use the controller in desktop mode".
- **Choose what controller mode shows**: Settings → General → Categories lists every filter, platform, store and
  collection. Hide the ones you don't want, or add ones your theme doesn't show, such as **PC Games** with PC (Linux)
  and PC (Windows). Highlight a category and open the menu to hide it in one step. Mouse users find the same list in
  desktop mode under Settings → Controller.

### Library
- **Games no longer in your Steam library** (for example ones you played through EA Play after the subscription
  ended) are marked "Not in your Steam library" and offer **Open in Steam** instead of an install that fails. Find
  them under Not installed. This needs your Steam Web API key in Settings.
- The "Desktop" library is now called **Shortcuts**: games from your app menu that belong to no other launcher.
- Leftover second copies of games from before 0.17.2 are folded into the real game, keeping their play time and the
  pictures you picked.
- A game's right-click menu now also has Links, Open install folder, Create desktop shortcut and Calculate install
  size.

### Artwork
- **A welcome step on the very first start** recommends a free SteamGridDB key, so covers, backgrounds and logos show
  up for all your games, including emulated games and shortcuts, from the start. It appears once, never after
  updates, and "Skip for now" says where to add the key later.
- **Far more covers for emulated games without a key**: when GitHub's hourly limit runs out during a big download,
  Lodestar still finds libretro's console covers by the game's exact name.
- New installs ask IGDB before GOG for PC game artwork and details.

### Themes
- **Improved theme creator imports**: imported Playnite themes look much closer to their originals. Desktop themes
  get their selected rows, panels, list columns, facts boxes and darkened art. Fullscreen themes get their own
  buttons, game pages, filter strips and hint rows.
- Imported Playnite fullscreen themes have a **Show game titles** option, off by default as in Playnite.

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
