# Making themes for Lodestar

A controller theme decides how Lodestar's fullscreen controller mode looks: the screen where you pick a
collection, the list of games, and (optionally) a game's page. There are two ways to make one:

- **The theme creator** (menu → Theme creator, or Add-ons → Controller themes → Create theme…): start from any
  theme, change it while you watch, and share it as a `.zip`. See [docs/theme-creator.md](docs/theme-creator.md).
- **By hand**: a theme is one text file, `theme.toml`, plus any pictures, fonts and sounds next to it. This guide
  explains that file. Everything the creator does ends up in it, so you can switch between the two at any time.

A ready-to-copy example lives in [examples/themes/starter/theme.toml](examples/themes/starter/theme.toml).

## Where themes live

```
~/.local/share/lodestar/themes/<theme-folder>/
    theme.toml          the theme
    images/…            pictures it uses (any names and folders you like)
    fonts/…             fonts it uses (.ttf / .otf)
    sounds/…, music/…   optional navigation sounds and background music
```

Lodestar finds every folder there with a `theme.toml` in it. Pick a theme in Settings → Visuals (controller
mode) or Settings → Appearance (desktop). Paths inside the file are relative to the theme's folder.

## The shape of a theme

```toml
[theme]            # name, author, version, description … (see "Sharing" below)
[palette]          # the theme's colours, also used by Lodestar's own menus over it
[fonts]            # font roles → files or built-in fonts

[[collections.elements]]   # the collection picker: one block per element
[[games.elements]]         # the game list of one collection
[[details.elements]]       # optional: a game's page (Lodestar draws its own if you leave it out)
[[home.elements]]          # optional: a start page with cover rows and a navigation rail
```

Each **view** is a list of **elements** drawn back to front by `z` (ties: file order). An element has a `type`, a
`name` (used by variants, overrides and the creator), and a place on screen:

- `pos` and `size` are fractions of the screen: `pos = [0.05, 0.1]` is 5 % from the left, 10 % from the top. A size
  of `0` means "automatic" (keep the picture's shape, or fit the text).
- `origin` is the point of the element that sits at `pos`: `[0, 0]` top-left, `[0.5, 0.5]` centre, `[1, 1]`
  bottom-right.
- Text sizes (`font_size`) are fractions of the screen height, so a theme fits any resolution.

### Element types

| `type` | What it draws |
| --- | --- |
| `image` | A picture from the theme folder or a value like `{art.cover}`; `fit`, `corner_radius`, `fallbacks`, a `video` |
| `text` | Text with values in braces: `"{game.name} · {game.year}"` |
| `rect` | A filled box or panel (also a progress bar, see below) |
| `carousel` | A row or column of items that scrolls around the selection |
| `grid` | Rows and columns of items (`columns`), the selection grows if you like |
| `textlist` | A plain list of names, made for big libraries and letter jumps |
| `rows` | Cover rows, one per collection (home page) |
| `rail` | The navigation rail (Home, All games, Platforms …) |
| `stack` | Children that flow down or across, measured with the real fonts, so long names never overlap |
| `rating` | Stars from `{game.rating}` |
| `datetime` | A clock or a date |
| `helpbar` | The controller button hints, drawn in the style of the player's pad |

Lists (`carousel`, `grid`, `textlist`, `rows`) take `item_image`, `item_text`, item sizes and selection looks
(`selected_scale`, borders, glow, dimming of the others); `item_elements` lets you design each item yourself.

### Values

Text, pictures and conditions can use values in braces, filled in for the selected collection or game:

| Group | Values |
| --- | --- |
| Game | `game.name`, `game.description`, `game.developer`, `game.publisher`, `game.genre`, `game.year`, `game.platform`, `game.source`, `game.tags`, `game.size`, `game.letter` |
| Play | `game.playtimelabel` ("12h 30m played"), `game.lastplayedlabel`, `game.playcount`, `game.played`, `game.completion` |
| Ratings | `game.rating` (0–1), `game.score`, `game.criticscore`, `game.communityscore`, `game.scorelabel` |
| Badges | `game.favorite`, `game.installed`, `game.controllerlabel`, `game.decklabel`, `game.genrelist` (pills in `chips`) |
| Achievements | `game.achievements` ("12 / 40"), `game.achievementslabel`, `game.achievementspercent` (0–1, for bars) |
| Game art | `art.cover`, `art.hero`, `art.logo`, `art.icon`, `art.banner` |
| Collection | `collection.name`, `collection.key`, `collection.count`, `collection.countlabel` ("12 games") |
| Console art | `collection.art.card`, `collection.art.hero`, `collection.art.mark`, `collection.art.device`, and `game.consoleart.*` for the selected game's console |
| List | `list.index`, `list.count`, `list.countlabel` |
| System | `system.battery`, `system.charging`, `theme.dir` |

A value that isn't known is empty, so `"{game.year}"` simply shows nothing for a game without a year.
Names can change between versions (the "Desktop" library is called "Shortcuts" now), so pictures and
conditions that must keep working are best keyed by `game.sourcekey` or `collection.key`; a theme that
still uses the old name finds `Desktop.png` and matches `{game.source} == Desktop` all the same.

Which categories controller mode lists is partly the player's choice (Settings → Categories): a theme's
`rows` and collection lists leave out what the player hid and include what they added, so a theme should
look right with any number of collections.

### Showing things only sometimes

Any element can have `visible_if` or `hidden_if`:

```toml
visible_if = "{game.installed}"                      # shown when not empty
hidden_if  = "{art.logo}"                            # hide the title text when there is a logo
visible_if = "{game.achievementstotal} > 0 && {game.installed} == true"
```

### Progress bars

A `rect` whose `fill` is a 0–1 value is a bar: `color` covers that share, `track` the rest.

```toml
[[games.elements]]
type = "rect"
name = "achievement_bar"
fill = "{game.achievementspercent}"
color = "#4c8dff"
track = "#ffffff22"
pos = [0.05, 0.9]
size = [0.25, 0.006]
visible_if = "{game.achievements}"
```

### Movement

`transitions` animate an element when the selection moves or when it appears: scale, fade and slide, with easing.
Lodestar turns them off for players who chose reduced motion.

## Console artwork

Lodestar draws its own original artwork for every console, store and built-in collection: a portrait card, a
background, a name mark and the device on its own (`{collection.art.*}`). To use your own picture for a console,
add it per collection key:

```toml
[collection_art.sony_playstation2]
card = "images/ps2-card.png"
hero = "images/ps2-background.jpg"
```

The theme creator's **Console pictures** page fills these for you, and can take a whole folder of pictures named
after the usual short console names (`ps2.png`, `snes.png`, `megadrive.png` …).

## Choices for the player

All optional; the player picks them in Settings without touching the file.

- **Colour schemes**: `[color_schemes.<name>]` change palette colours; elements using those colours follow.
- **Variants**: `[variants.<name>]` change elements by name, for example a "Grid" and a "Wheel" games screen. A
  variant can show elements the main layout keeps hidden (`visible = false`).
- **Screen shapes**: `[aspects."16:10"]` (Steam Deck), `"4:3"`, `"21:9"` adjust the layout for other screens and
  are chosen automatically.
- **Per console**: `[overrides.<collection key>]` change elements for one console or collection, such as a
  different background colour.

## Sharing

Fill in the sharing details in `[theme]` so your theme can be listed in a catalogue later:

```toml
[theme]
name = "Starter"
id = "your-name-starter"        # lower-case letters, digits and dashes
author = "Your name"
version = "1.0.0"
license = "CC-BY-4.0"         # or "All rights reserved" …
summary = "A console carousel, then a grid of covers."
tags = ["dark", "carousel"]
previews = ["previews/collections.png", "previews/games.png"]
```

Then use **Export** in the theme creator to make a `.zip`; anyone with Lodestar adds it with **Add a theme .zip…**.
Only share pictures, fonts and music you made or may share, and credit other people's work in your description.

## Coming from another frontend?

The theme creator guide has a table that maps the ideas you know from other frontends (system views, game lists,
carousels, variants, colour schemes, screen-shape layouts) to Lodestar's:
[docs/theme-creator.md](docs/theme-creator.md#coming-from-another-frontend). Porting a theme you made yourself is
welcome; for anyone else's theme, ask its author first.
