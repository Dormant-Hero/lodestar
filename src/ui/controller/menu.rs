//! Side-panel menus (quick menu, game options, sort, themes, launch options, settings with
//! tabs) and centred panels (confirmations, the in-game menu, "Which button opens Lodestar's
//! menu?"). An item can carry a help text: one panel under the list shows the selected (or
//! hovered) item's.

use super::look::Look;
use super::power::Power;
use crate::config::SortBy;
use crate::model::{CompletionStatus, ControllerSupport, GameId};
use crate::theme::render::draw::{cr, fade, layout_line};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, FontId, Galley, Painter, Pos2, Rect, pos2, vec2};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    SwitchDesktop,
    Rescan,
    ThemeMenu,
    SetTheme(String),
    VariantMenu,
    SetVariant(Option<String>),
    /// The theme's colour schemes.
    SchemeMenu,
    SetScheme(Option<String>),
    /// Settings → Visuals → Theme options (an imported Playnite theme's own options).
    ThemeOptionsMenu,
    /// One theme option a step on (+1) / back (−1): its index among the controller's options.
    ThemeOption(usize, i8),
    ToggleSwapAb,
    /// Button prompts: Automatic → Xbox → PlayStation → Nintendo.
    CycleButtonPrompts,
    ToggleFullscreen,
    /// Settings → Launchers (store clients around games, one game at a time).
    LaunchersMenu,
    ToggleHideLaunchers,
    ToggleCloseLaunchers,
    ToggleHideSteam,
    ToggleCloseSteam,
    ToggleKeepSteam,
    CycleOneGame,
    CycleCloseWhilePlaying,
    ToggleTrayIcon,
    Quit,
    Play(GameId),
    /// Emulated games: Play with the emulator in a window.
    PlayWindowed(GameId),
    /// Emulated games: the game's emulator on its own (its settings).
    OpenEmulator(GameId),
    /// Emulated games: the "Play with" list (another emulator / profile).
    PlayWithMenu(GameId),
    /// Play with (game, emulator, profile) — this once, or from now on when "Always" is on.
    PlayWith(GameId, String, String),
    /// The "Always use this for this game" line of the Play with list.
    ToggleAlwaysUse(GameId),
    /// A line of "Choose what to play" (several Play actions / ROMs / profiles).
    PlayChoice(GameId, crate::model::PlayChoice),
    /// The "Always use this for this game" line of "Choose what to play".
    ToggleChooseAlways(GameId),
    StopGame(GameId),
    ToggleFavorite(GameId),
    Hide(GameId),
    Unhide(GameId),
    ToggleShowHidden,
    LaunchOptions(GameId),
    SortMenu,
    SetSort(Option<SortBy>),
    ToggleDescending,
    EditOptions(GameId),
    EditCommand(GameId),
    UseSourceOptions(GameId),
    ClearOverride(GameId),
    ShowDetails(GameId),
    /// The artwork viewer.
    ViewArt(GameId),
    /// Run `edits.actions[index]` of the game ("Play: Settings" …).
    LaunchAction(GameId, usize),
    CompletionMenu(GameId),
    SetCompletion(GameId, Option<CompletionStatus>),
    ControllerMenu(GameId),
    /// `None` = automatic (what the source says).
    SetController(GameId, Option<ControllerSupport>),
    /// Hand over to the game's store / add-on to install / uninstall it.
    Install(GameId),
    /// A RomM game downloading: stop it (romm_app.rs cleans up).
    CancelDownload(GameId),
    Uninstall(GameId),
    /// Quick menu: a random game of the current list (else of all games).
    RandomGame,
    /// Quick menu → Settings (tabs).
    SettingsMenu,
    /// A settings switch / choice (settings.rs).
    Setting(Setting),
    /// Power item: ask first, then run it.
    PowerAsk(Power),
    PowerDo(Power),
    /// "Cancel" of a confirmation: close it.
    CloseMenu,
    /// Quick menu → About Lodestar (credits.rs).
    Credits,
    /// Quick menu → Update Lodestar (update.rs): its page, Update now, cancel, restart.
    Update,
    /// Quick menu → Check for updates (no newer version known yet).
    CheckUpdates,
    UpdateSkip,
    UpdateNow,
    UpdateCancel,
    UpdateRestart,
    /// A text block of a reading page (About): selectable so ↑ / ↓ scroll, does nothing.
    Read,
    /// Copy this text to the clipboard (an install command).
    CopyText(String),
    /// In-game menu (ingame.rs).
    Resume,
    Screenshot,
    /// ← / → change it, A mutes / unmutes.
    Volume,
    Library,
    QuitGameAsk(GameId),
    QuitGameDo(GameId),
    /// "Which button opens Lodestar's menu?"'s answers (ingame.rs).
    MenuButtonPick(crate::config::MenuButton),
    /// Change the menu button combination: the presets and "Record a combination" (combo.rs).
    PickMenuCombo,
    MenuComboPreset(crate::config::MenuCombo),
    RecordMenuCombo,
    /// The recording screen: use the take, or record again.
    RecordedComboUse(crate::config::MenuCombo),
    RecordComboAgain,
}

/// The controller settings' switches and choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Clock,
    Battery,
    MinimiseOnLaunch,
    StartMode,
    Blur,
    Darken,
    Titles,
    DarkenNotInstalled,
    TextSize,
    ShowPower(Power),
    AllowQuit,
    GuideLodestar,
    GuideDesktop,
    HideCursor,
    ForegroundOnly,
    /// Settings → Visuals: theme videos.
    Videos,
    /// Settings → Audio.
    NavSounds,
    Music,
    /// Volume up (+1) / down (-1) a step.
    Volume(i8),
    /// Pick the music folder (a folder picker), or go back to the automatic one.
    MusicFolder,
    MusicAuto,
    /// Settings → Input: what opens Lodestar's menu (the button combination ↔ Home).
    MenuButton,
    /// Settings → Visuals: achievements on game pages.
    Achievements,
    /// Settings → Visuals: achievement counts in the library (game lists, covers).
    AchievementCounts,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    pub value: Option<String>,
    pub act: Option<Act>,
    /// Multi-line, non-selectable information block (`label` = heading, `value` = text).
    pub info: bool,
    pub danger: bool,
    /// Shown in the menu's help panel while this item is selected or hovered.
    pub help: Option<String>,
}

impl Item {
    pub fn action(label: impl Into<String>, act: Act) -> Item {
        Item { label: label.into(), value: None, act: Some(act), info: false, danger: false, help: None }
    }
    pub fn value(label: impl Into<String>, value: impl Into<String>, act: Act) -> Item {
        Item { label: label.into(), value: Some(value.into()), act: Some(act), info: false, danger: false, help: None }
    }
    pub fn info(heading: impl Into<String>, text: impl Into<String>) -> Item {
        Item { label: heading.into(), value: Some(text.into()), act: None, info: true, danger: false, help: None }
    }
    pub fn danger(mut self) -> Item {
        self.danger = true;
        self
    }
    pub fn with_help(mut self, text: impl Into<String>) -> Item {
        self.help = Some(text.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MenuKind {
    Quick,
    Themes,
    Variants,
    Schemes,
    ThemeOptions,
    GameOptions(GameId),
    Sort,
    Launchers,
    LaunchOptions(GameId),
    Completion(GameId),
    Controller(GameId),
    PlayWith(GameId),
    /// "Choose what to play".
    ChoosePlay(GameId),
    Settings,
    /// About Lodestar (credits).
    Credits,
    /// "What's new in <version>": Lodestar's own update.
    Update,
    /// A confirmation (power items).
    Confirm,
    InGame,
    /// "Quit <game>?" of the in-game menu.
    InGameConfirm,
    /// "Which button opens Lodestar's menu?": asked once, on the first press.
    MenuButtonAsk,
    /// "Change combination": the presets and "Record a combination".
    MenuComboPick,
    /// "Record a combination": the pad's buttons are recorded, not acted on.
    MenuComboRecord,
}

pub struct Menu {
    pub kind: MenuKind,
    pub title: String,
    pub subtitle: Option<String>,
    pub items: Vec<Item>,
    pub sel: usize,
    pub opened: f64,
    /// A centred panel (confirmations, in-game menu) instead of one sliding in from the right.
    pub centred: bool,
    /// Tabs under the title (LB / RB or a click switch them; the owner swaps the items).
    pub tabs: Vec<String>,
    pub tab: usize,
    /// The footer's button hints instead of the usual Select / Back (an empty list: none, for a
    /// screen where the pad's buttons don't act as usual).
    pub hints: Option<Vec<crate::theme::render::draw::Hint>>,
    /// The row under the pointer last frame: the help panel is sized for the help it shows.
    hovered: Option<usize>,
    scroll: f32,
    scroll_init: bool,
}

#[derive(Default)]
pub struct MenuOut {
    pub clicked: Option<usize>,
    pub animating: bool,
    /// Click outside the panel.
    pub dismissed: bool,
    /// A tab was clicked.
    pub tab: Option<usize>,
}

impl Menu {
    pub fn new(kind: MenuKind, title: impl Into<String>, items: Vec<Item>, now: f64) -> Menu {
        let sel = items.iter().position(|i| i.act.is_some()).unwrap_or(0);
        Menu {
            kind,
            title: title.into(),
            subtitle: None,
            items,
            sel,
            opened: now,
            centred: false,
            tabs: vec![],
            tab: 0,
            hints: None,
            hovered: None,
            scroll: 0.0,
            scroll_init: false,
        }
    }

    pub fn centred(mut self) -> Menu {
        self.centred = true;
        self
    }

    pub fn with_tabs(mut self, tabs: Vec<String>, tab: usize) -> Menu {
        self.tab = tab.min(tabs.len().saturating_sub(1));
        self.tabs = tabs;
        self
    }

    /// New items for another tab: selection on the first selectable one, scrolled to the top.
    pub fn set_tab(&mut self, tab: usize, items: Vec<Item>) {
        self.tab = tab;
        self.sel = items.iter().position(|i| i.act.is_some()).unwrap_or(0);
        self.items = items;
        self.scroll_init = false;
    }

    pub fn with_hints(mut self, hints: Vec<crate::theme::render::draw::Hint>) -> Menu {
        self.hints = Some(hints);
        self
    }

    pub fn with_subtitle(mut self, s: impl Into<String>) -> Menu {
        self.subtitle = Some(s.into());
        self
    }

    /// Select the item with this action if present.
    pub fn select_act(&mut self, act: &Act) {
        if let Some(i) = self.items.iter().position(|i| i.act.as_ref() == Some(act)) {
            self.sel = i;
        }
    }

    /// Replace the items keeping the selection on the same action (or index).
    pub fn replace_items(&mut self, items: Vec<Item>) {
        let act = self.selected_act().cloned();
        self.items = items;
        self.sel = self.sel.min(self.items.len().saturating_sub(1));
        if let Some(a) = act {
            self.select_act(&a);
        }
        if self.items.get(self.sel).is_some_and(|i| i.act.is_none()) {
            self.move_sel(1);
        }
    }

    pub fn move_sel(&mut self, d: i32) {
        let n = self.items.len() as i32;
        if n == 0 {
            return;
        }
        let mut i = self.sel as i32;
        for _ in 0..n {
            i += d.signum();
            if i < 0 || i >= n {
                return; // stop at the ends
            }
            if self.items[i as usize].act.is_some() {
                self.sel = i as usize;
                return;
            }
        }
    }

    pub fn selected_act(&self) -> Option<&Act> {
        self.items.get(self.sel).and_then(|i| i.act.as_ref())
    }

    /// Panel width for a screen.
    fn panel_width(&self, screen: Rect, look: &Look) -> f32 {
        let frac = if self.centred {
            0.42
        } else if self.tabs.is_empty() {
            0.36
        } else {
            0.56
        };
        (screen.width() * frac).clamp(look.h(0.55).min(screen.width()), screen.width())
    }

    pub fn draw(
        &mut self,
        painter: &Painter,
        screen: Rect,
        look: &Look,
        now: f64,
        dt: f32,
        pointer: Option<(Pos2, bool)>,
    ) -> MenuOut {
        let mut out = MenuOut::default();
        let t = (((now - self.opened) / 0.16) as f32).clamp(0.0, 1.0);
        if t < 1.0 {
            out.animating = true;
        }
        painter.rect_filled(screen, 0.0, Color32::from_black_alpha((150.0 * ease(t)) as u8));
        let w = self.panel_width(screen, look);
        let pad = look.h(0.04);
        let m = self.measure(painter, look, w, pad);
        let panel = self.panel_rect(screen, look, w, t, &m);
        let mut p = painter.with_clip_rect(panel);
        p.multiply_opacity(ease(t));
        if self.centred {
            p.rect_filled(panel, cr(look.h(0.02)), look.surface);
        } else {
            p.rect_filled(panel, 0.0, look.surface);
            p.rect_filled(
                Rect::from_min_size(panel.min, vec2(look.h(0.003), panel.height())),
                0.0,
                fade(look.accent, 0.5),
            );
        }
        let inner = panel.shrink2(vec2(pad, pad));
        let list_top = self.draw_header(&p, look, inner, &m, pointer, &mut out);
        let list = Rect::from_min_max(
            pos2(inner.left() - pad * 0.4, list_top),
            pos2(inner.right() + pad * 0.4, inner.bottom() - m.footer_h - m.help_h),
        );
        self.draw_footer(&p, look, inner, m.footer_h);
        self.scroll_to_selection(&m, list.height(), dt, &mut out);
        let hovered = self.draw_rows(&p, look, list, &m, pointer, &mut out);
        if m.total > list.height() && self.scroll < m.total - list.height() - 1.0 {
            // Scroll hint: fade at the bottom edge (not once the end is on screen: it would
            // dim the last lines, e.g. Settings → Input's notes).
            let fade_h = look.h(0.05);
            let b = Rect::from_min_max(pos2(list.left(), list.bottom() - fade_h), list.max);
            crate::theme::render::draw::gradient_rect(&p, b, 0.0, [Color32::TRANSPARENT, look.surface], false);
        }
        if m.help_h > 0.0 {
            let r = Rect::from_min_max(
                pos2(list.left(), list.bottom() + m.gap),
                pos2(list.right(), list.bottom() + m.help_h),
            );
            p.rect_filled(r, cr(look.h(0.01)), look.surface_alt);
            if let Some(Some(g)) = self.help_item(self.hovered).and_then(|i| m.help_galleys.get(i)) {
                p.galley(pos2(r.left() + look.h(0.02), r.top() + look.h(0.0175)), g.clone(), look.text);
            }
        }
        if hovered != self.hovered {
            // The next frame sizes the help panel for the row now under the pointer.
            self.hovered = hovered;
            out.animating = true;
        }
        if let Some((pos, true)) = pointer
            && !panel.contains(pos)
        {
            out.dismissed = true;
        }
        out
    }

    /// Lay out the title, subtitle and rows and work out every height (panel `w` wide).
    fn measure(&self, painter: &Painter, look: &Look, w: f32, pad: f32) -> Measured {
        let list_w = w - 2.0 * pad + pad * 0.8;
        // A long title (a bigger text size on a small screen: "Shut down the computer?")
        // wraps onto a second line instead of being cut off.
        let title = wrapped(painter, self.title.clone(), look.title(0.04), look.text, (w - 2.0 * pad).max(1.0), 2);
        let subtitle =
            self.subtitle.as_ref().map(|s| layout_line(painter, s, look.medium(0.021), look.dim, w - 2.0 * pad));
        let tabs_h = if self.tabs.is_empty() { 0.0 } else { look.h(0.06) };
        let header_h =
            title.size().y + look.h(0.008) + subtitle.as_ref().map_or(0.0, |g| g.size().y) + tabs_h + look.h(0.03);
        let row_h = look.h(0.06);
        let mut heights = Vec::with_capacity(self.items.len());
        let mut info_galleys = Vec::with_capacity(self.items.len());
        // Row labels: a long one (a bigger text size, a narrow screen) wraps onto a second line
        // in a taller row instead of being cut off. Laid out once here, coloured when drawn.
        let mut label_galleys = Vec::with_capacity(self.items.len());
        for it in &self.items {
            if it.info {
                label_galleys.push(None);
                continue;
            }
            let value_w = it.value.as_ref().map_or(0.0, |v| {
                let label_w = painter.layout_no_wrap(it.label.clone(), look.medium(0.025), look.text).size().x;
                let max = (list_w - look.h(0.074) - label_w).max(list_w * 0.4);
                painter.layout_no_wrap(v.clone(), look.medium(0.022), look.dim).size().x.min(max) + look.h(0.03)
            });
            let max_width = (list_w - look.h(0.044) - value_w).max(1.0);
            label_galleys.push(Some(wrapped(painter, it.label.clone(), look.medium(0.025), look.text, max_width, 2)));
        }
        for (k, it) in self.items.iter().enumerate() {
            if it.info {
                // Word boundaries first; a word too long for a row (a path) still breaks.
                let text = it.value.clone().unwrap_or_default();
                let g = wrapped(painter, text, look.font(0.021), look.text, list_w - look.h(0.04), 8);
                let h = g.size().y + look.h(0.021) * 1.3 + look.h(0.035);
                heights.push(h);
                info_galleys.push(Some(g));
            } else {
                let lh = label_galleys[k].as_ref().map_or(0.0, |g| g.size().y);
                heights.push(row_h.max(lh + look.h(0.026)));
                info_galleys.push(None);
            }
        }
        let gap = look.h(0.006);
        let total: f32 = heights.iter().sum::<f32>() + gap * heights.len() as f32;
        // The help panel under the list: as tall as the help it shows (the hovered row's, else
        // the selected one's), so a short help doesn't sit in a big empty box.
        let help_galleys: Vec<_> = self
            .items
            .iter()
            .map(|it| {
                it.help
                    .as_ref()
                    .map(|text| wrapped(painter, text.clone(), look.font(0.021), look.text, list_w - look.h(0.04), 8))
            })
            .collect();
        let mut m = Measured {
            title,
            subtitle,
            tabs_h,
            header_h,
            row_h,
            heights,
            info_galleys,
            label_galleys,
            gap,
            total,
            footer_h: look.h(0.05),
            help_galleys,
            help_h: 0.0,
        };
        m.help_h = self
            .help_item(self.hovered)
            .and_then(|i| m.help_galleys.get(i))
            .and_then(|g| g.as_ref())
            .map_or(0.0, |g| m.help_size(g, look));
        m
    }

    /// Where the panel is: centred, or sliding in from the right (`t`: the opening animation).
    fn panel_rect(&self, screen: Rect, look: &Look, w: f32, t: f32, m: &Measured) -> Rect {
        let pad = look.h(0.04);
        if self.centred {
            let chrome = m.header_h + m.total + m.footer_h + 2.0 * pad + look.h(0.01);
            let h = (chrome + m.help_h).min(screen.height() * 0.9);
            // Placed as if the tallest help showed: the list stays put while the help panel
            // grows and shrinks at the bottom (rows don't move under the pointer).
            let tallest = m.help_galleys.iter().flatten().map(|g| m.help_size(g, look)).fold(0.0, f32::max);
            let top = screen.center().y - (chrome + tallest).min(screen.height() * 0.9) / 2.0;
            let r = Rect::from_min_size(pos2(screen.center().x - w / 2.0, top), vec2(w, h));
            r.translate(vec2(0.0, (1.0 - ease(t)) * look.h(0.03)))
        } else {
            let off = (1.0 - ease(t)) * w * 0.35;
            Rect::from_min_size(pos2(screen.right() - w + off, screen.top()), vec2(w, screen.height()))
        }
    }

    /// Title, subtitle and tabs (a clicked tab goes to `out.tab`). Returns where the list starts.
    fn draw_header(
        &self,
        p: &Painter,
        look: &Look,
        inner: Rect,
        m: &Measured,
        pointer: Option<(Pos2, bool)>,
        out: &mut MenuOut,
    ) -> f32 {
        let mut y = inner.top();
        p.galley(pos2(inner.left(), y), m.title.clone(), look.text);
        y += m.title.size().y + look.h(0.008);
        if let Some(g) = &m.subtitle {
            p.galley(pos2(inner.left(), y), g.clone(), look.dim);
            y += g.size().y;
        }
        if !self.tabs.is_empty() {
            y += look.h(0.015);
            let mut x = inner.left();
            for (i, name) in self.tabs.iter().enumerate() {
                let on = i == self.tab;
                let c = if on { look.text } else { look.dim };
                let g = p.layout_no_wrap(name.clone(), look.medium(0.024), c);
                let size = g.size();
                let r = Rect::from_min_size(pos2(x, y), vec2(size.x, size.y + look.h(0.012)));
                p.galley(pos2(x, y), g, c);
                if on {
                    p.rect_filled(
                        Rect::from_min_size(pos2(x, y + size.y + look.h(0.006)), vec2(size.x, look.h(0.004))),
                        0.0,
                        look.accent,
                    );
                }
                if let Some((pos, true)) = pointer
                    && r.expand(look.h(0.008)).contains(pos)
                {
                    out.tab = Some(i);
                }
                x += size.x + look.h(0.035);
            }
            y += m.tabs_h - look.h(0.015);
        }
        y + look.h(0.03)
    }

    /// The button hints along the bottom (Select / Back … unless the menu brings its own).
    fn draw_footer(&self, p: &Painter, look: &Look, inner: Rect, footer_h: f32) {
        use crate::theme::render::draw::{Button, Hint, paint_hints};
        let hints = self.hints.clone().unwrap_or_else(|| {
            let mut hints = vec![Hint::new(Button::A, "Select"), Hint::new(Button::B, "Back")];
            if !self.tabs.is_empty() {
                hints.push(Hint::new(Button::LB, "Tabs"));
            }
            if !self.centred {
                hints.push(Hint::new(Button::Start, "Close"));
            }
            hints
        });
        let fr = Rect::from_min_max(pos2(inner.left(), inner.bottom() - footer_h * 0.7), inner.max);
        paint_hints(
            p,
            fr,
            &hints,
            look.prompts,
            look.medium.clone(),
            look.h(0.018),
            look.dim,
            fade(look.text, 0.85),
            look.surface,
            crate::theme::Align::Left,
        );
    }

    /// Glide the list towards keeping the selected row in view (`avail`: the list's height).
    fn scroll_to_selection(&mut self, m: &Measured, avail: f32, dt: f32, out: &mut MenuOut) {
        let sel_top: f32 = m.heights.iter().take(self.sel).map(|h| h + m.gap).sum();
        let sel_h = m.heights.get(self.sel).copied().unwrap_or(m.row_h);
        let last_act = self.items.iter().rposition(|i| i.act.is_some());
        let target = if m.total <= avail {
            0.0
        } else if last_act == Some(self.sel) {
            // Information blocks after the last selectable row can't be selected: on that row
            // the list scrolls to its end so they can be read (never past the row itself).
            (m.total - avail).min(sel_top)
        } else {
            (sel_top + sel_h / 2.0 - avail / 2.0).clamp(0.0, m.total - avail)
        };
        if !self.scroll_init {
            self.scroll = target;
            self.scroll_init = true;
        }
        let d = target - self.scroll;
        if d.abs() > 0.5 {
            self.scroll += d * (1.0 - (-dt * 16.0).exp());
            out.animating = true;
        } else {
            self.scroll = target;
        }
    }

    /// The rows inside `list` (a click goes to `out.clicked`). Returns the row under the pointer.
    fn draw_rows(
        &self,
        p: &Painter,
        look: &Look,
        list: Rect,
        m: &Measured,
        pointer: Option<(Pos2, bool)>,
        out: &mut MenuOut,
    ) -> Option<usize> {
        let lp = p.with_clip_rect(list);
        let mut y = list.top() - self.scroll;
        let mut hovered = None;
        for (i, it) in self.items.iter().enumerate() {
            let h = m.heights[i];
            let r = Rect::from_min_size(pos2(list.left(), y), vec2(list.width(), h));
            y += h + m.gap;
            if r.bottom() < list.top() || r.top() > list.bottom() {
                continue;
            }
            if let Some((pos, _)) = pointer
                && r.contains(pos)
                && list.contains(pos)
                && it.act.is_some()
            {
                hovered = Some(i);
            }
            if let Some((pos, clicked)) = pointer
                && clicked
                && r.contains(pos)
                && it.act.is_some()
                && list.contains(pos)
            {
                out.clicked = Some(i);
            }
            match &m.info_galleys[i] {
                Some(g) => self.draw_info_row(&lp, look, r, i, g),
                None => self.draw_row(&lp, look, r, i, m.label_galleys[i].as_ref()),
            }
        }
        hovered
    }

    /// An information block: a small heading over wrapped text.
    fn draw_info_row(&self, lp: &Painter, look: &Look, r: Rect, i: usize, g: &Arc<Galley>) {
        let it = &self.items[i];
        lp.rect_filled(r, cr(look.h(0.01)), look.surface_alt);
        if i == self.sel && it.act.is_some() {
            // A reading page's block (About): the selection is an outline.
            lp.rect_stroke(
                r,
                cr(look.h(0.01)),
                egui::Stroke::new(look.h(0.003), look.accent),
                egui::StrokeKind::Inside,
            );
        }
        let hx = r.left() + look.h(0.02);
        let hg = lp.layout_no_wrap(it.label.to_uppercase(), look.bold(0.0165), look.dim);
        lp.galley(pos2(hx, r.top() + look.h(0.015)), hg.clone(), look.dim);
        lp.galley(pos2(hx, r.top() + look.h(0.015) + hg.size().y + look.h(0.006)), g.clone(), look.text);
    }

    /// A selectable row: its label, and its value on the right.
    fn draw_row(&self, lp: &Painter, look: &Look, r: Rect, i: usize, label: Option<&Arc<Galley>>) {
        let it = &self.items[i];
        let selected = i == self.sel;
        let (fg, sub) = if selected {
            lp.rect_filled(r, cr(look.h(0.012)), look.accent);
            (look.accent_text, fade(look.accent_text, 0.8))
        } else if it.danger {
            (look.danger, look.dim)
        } else {
            (look.text, look.dim)
        };
        let tx = r.left() + look.h(0.022);
        // A long value (a game's name next to "Quit game", a theme name) never runs into
        // the label: it gets what the label leaves, at least 40 % of the row, and ends in "…".
        let label_w = lp.layout_no_wrap(it.label.clone(), look.medium(0.025), fg).size().x;
        let value_max = (r.width() - look.h(0.074) - label_w).max(r.width() * 0.4);
        if let Some(v) = &it.value {
            let g = layout_line(lp, v, look.medium(0.022), sub, value_max);
            let w = g.size().x;
            lp.galley(pos2(r.right() - look.h(0.022) - w, r.center().y - g.size().y / 2.0), g, sub);
        }
        if let Some(g) = label {
            lp.galley_with_override_text_color(pos2(tx, r.center().y - g.size().y / 2.0), g.clone(), fg);
        }
    }

    /// The item whose help the panel shows: the one under the pointer, else the selected one
    /// (None: neither has a help text).
    pub fn help_item(&self, hovered: Option<usize>) -> Option<usize> {
        let has = |i: &usize| self.items.get(*i).is_some_and(|it| it.help.is_some());
        hovered.filter(has).or(Some(self.sel).filter(has))
    }
}

/// A menu's laid-out text and heights for one frame (`Menu::measure`).
struct Measured {
    title: Arc<Galley>,
    subtitle: Option<Arc<Galley>>,
    tabs_h: f32,
    header_h: f32,
    row_h: f32,
    /// Per item: its row's height …
    heights: Vec<f32>,
    /// … an information block's text …
    info_galleys: Vec<Option<Arc<Galley>>>,
    /// … or a selectable row's label …
    label_galleys: Vec<Option<Arc<Galley>>>,
    /// … and its help text.
    help_galleys: Vec<Option<Arc<Galley>>>,
    gap: f32,
    /// The list's full height.
    total: f32,
    footer_h: f32,
    /// The help panel's height this frame (0: no help shown).
    help_h: f32,
}

impl Measured {
    /// The help panel's height for help text `g`.
    fn help_size(&self, g: &Galley, look: &Look) -> f32 {
        g.size().y + look.h(0.035) + self.gap
    }
}

/// `text` in up to `max_rows` rows of `max_width`, broken at words (a word too long for a row
/// still breaks) and ending in "…" when cut.
fn wrapped(
    painter: &Painter,
    text: String,
    font: FontId,
    color: Color32,
    max_width: f32,
    max_rows: usize,
) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text, TextFormat { font_id: font, color, ..Default::default() });
    job.wrap = TextWrapping { max_width, max_rows, break_anywhere: false, overflow_character: Some('…') };
    painter.layout_job(job)
}

pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_skips_info_rows() {
        let mut m = Menu::new(
            MenuKind::Quick,
            "T",
            vec![
                Item::info("a", "b"),
                Item::action("one", Act::Quit),
                Item::info("c", "d"),
                Item::action("two", Act::Rescan),
            ],
            0.0,
        );
        assert_eq!(m.sel, 1);
        m.move_sel(1);
        assert_eq!(m.sel, 3);
        m.move_sel(1);
        assert_eq!(m.sel, 3);
        m.move_sel(-1);
        assert_eq!(m.sel, 1);
        m.move_sel(-1);
        assert_eq!(m.sel, 1);
        m.replace_items(vec![Item::action("x", Act::Rescan), Item::action("y", Act::Quit)]);
        assert_eq!(m.selected_act(), Some(&Act::Quit));
    }

    #[test]
    fn help_follows_the_selection_and_the_pointer() {
        let mut m = Menu::new(
            MenuKind::MenuButtonAsk,
            "T",
            vec![
                Item::action("one", Act::Quit).with_help("first"),
                Item::action("two", Act::Rescan),
                Item::action("three", Act::CloseMenu).with_help("third"),
            ],
            0.0,
        );
        assert_eq!(m.help_item(None), Some(0));
        assert_eq!(m.help_item(Some(2)), Some(2), "hovered wins");
        assert_eq!(m.help_item(Some(1)), Some(0), "hovering one without help: the selected one's");
        m.move_sel(1);
        assert_eq!(m.help_item(None), None);
        m.move_sel(1);
        assert_eq!(m.help_item(None), Some(2));
    }
}
