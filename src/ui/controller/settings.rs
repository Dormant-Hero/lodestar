//! The controller settings screen (quick menu → Settings): tabs General, Visuals, Menus,
//! Input and Audio, each a list of switches; LB / RB (or a click) change the tab, ← / → / A change a
//! value. The same settings are in desktop Settings → Controller.

use super::power::{self, Power};
use super::*;
use crate::config::StartMode;
use menu::Setting;

pub(super) const TABS: [&str; 5] = ["General", "Visuals", "Menus", "Input", "Audio"];

/// Settings → Audio → Music folder: the folder picker's answer (it runs on its own thread so
/// the screen never waits for it). `Some(None)`: closed without a choice.
static PICKED_FOLDER: parking_lot::Mutex<Option<Option<std::path::PathBuf>>> = parking_lot::Mutex::new(None);
static PICKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A folder's name for a settings value ("~/Music/Retro" → "Retro").
fn folder_label(f: &str) -> String {
    let p = std::path::Path::new(f);
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.to_string())
}

/// The verified path in Steam's own settings (Nov 2026 Steam client).
pub(crate) const STEAM_GUIDE_PATH: &str = "Steam → Settings → Controller → Show Advanced Settings → External Gamepad Settings → \"Guide button focuses Steam\"";

/// Where Steam's in-game overlay is turned off (Steam desktop client, Nov 2026, confirmed by the
/// user: In Game → section "The Steam Overlay").
pub(crate) const STEAM_OVERLAY_PATH: &str = "Steam → Settings → In Game → \"Enable the Steam Overlay while in-game\"";

/// What choosing Home means: Steam and some emulators react to it too, and where Steam's use of
/// it is turned off ("Which button opens Lodestar's menu?", Settings).
pub(crate) fn home_button_note() -> String {
    format!(
        "Steam also reacts to Home. To stop that, turn off {STEAM_GUIDE_PATH} and {STEAM_OVERLAY_PATH}. \
         Some emulators, such as PCSX2, have Big Picture modes that may react to Home too."
    )
}

/// What choosing the combination means. Only L3 + R3 is known to be left alone; Select + Start
/// in particular is RetroArch's usual menu combination and a reset in many emulated games.
pub(crate) fn combo_note(combo: crate::config::MenuCombo) -> String {
    let others = if combo == crate::config::MenuCombo::STICKS {
        "Steam and emulators leave it alone."
    } else {
        "Some emulators use this combination too."
    };
    format!("{} {others}", combo.how())
}

/// What Home does next to the combination: nothing once a button was chosen; until then it
/// still opens the menu (so the first press can ask "Which button opens Lodestar's menu?").
pub(crate) fn home_note(c: &crate::config::ControllerConfig) -> &'static str {
    if c.home_opens_menu() { "Until you choose a button, Home opens the menu too." } else { "Lodestar ignores Home." }
}

/// The menu button block's names, the same in controller Settings → Input and desktop
/// Settings → Controller.
pub(crate) const MENU_BUTTON_LABEL: &str = "Lodestar's menu button";
pub(crate) const MENU_COMBO_LABEL: &str = "Combination";
pub(crate) const MENU_BUTTON_IN_GAME: &str = "During a game it brings up Lodestar";
pub(crate) const MENU_BUTTON_DESKTOP: &str = "In desktop mode it switches to controller mode";

/// What prompts call the menu button ("L3 + R3", "Home").
pub(crate) fn menu_button_name(st: &AppState) -> String {
    st.config.controller.menu_button_name()
}

fn on(b: bool) -> &'static str {
    if b { "On" } else { "Off" }
}

fn shown(b: bool) -> &'static str {
    if b { "Show" } else { "Hide" }
}

impl ControllerUi {
    pub(super) fn open_settings(&mut self, st: &AppState, now: f64) {
        let m = Menu::new(MenuKind::Settings, "Settings", Self::settings_items(st, 0), now)
            .with_subtitle("Controller mode")
            .with_tabs(TABS.iter().map(|t| t.to_string()).collect(), 0);
        self.menus.push(m);
    }

    pub(super) fn settings_items(st: &AppState, tab: usize) -> Vec<Item> {
        let c = &st.config.controller;
        let mut v = vec![];
        match tab {
            0 => {
                v.push(Item::value("Show the clock", on(c.show_clock), Act::Setting(Setting::Clock)));
                v.push(Item::value("Show the battery level", on(c.show_battery), Act::Setting(Setting::Battery)));
                v.push(Item::value(
                    "Minimise Lodestar while playing",
                    on(st.config.minimize_on_launch),
                    Act::Setting(Setting::MinimiseOnLaunch),
                ));
                v.push(Item::value("Fullscreen", on(st.wants_fullscreen()), Act::ToggleFullscreen));
                v.push(Item::value(
                    "Start Lodestar in",
                    match st.config.start_mode {
                        StartMode::Controller => "Controller mode",
                        StartMode::Desktop => "Desktop mode",
                    },
                    Act::Setting(Setting::StartMode),
                ));
                v.push(Item::action("Launchers", Act::LaunchersMenu));
                v.push(Item::value("Categories", Self::categories_summary(st), Act::CategoriesMenu));
            }
            1 => {
                v.push(Item::value("Theme", st.controller_theme.file.theme.name.clone(), Act::ThemeMenu));
                if !st.controller_theme.file.variants.is_empty() {
                    v.push(Item::value(
                        "Theme variant",
                        c.variant.clone().unwrap_or_else(|| "Default".into()),
                        Act::VariantMenu,
                    ));
                }
                if !st.controller_theme.file.color_schemes.is_empty() {
                    let f = &st.controller_theme.file;
                    let cur = c
                        .color_scheme
                        .as_ref()
                        .and_then(|k| f.color_schemes.get(k).map(|s| s.label_or(k).to_string()))
                        .unwrap_or_else(|| "Theme's own".into());
                    v.push(Item::value("Theme colours", cur, Act::SchemeMenu));
                }
                if let Some(spec) = crate::ui::theme_options::spec_for(st, &c.theme) {
                    let n = Self::controller_options(&spec).count();
                    if n > 0 {
                        v.push(Item::value("Theme options", format!("{n}"), Act::ThemeOptionsMenu));
                    }
                }
                v.push(Item::value("Blur the background", on(c.blur_background), Act::Setting(Setting::Blur)));
                v.push(Item::value("Darken the background", on(c.darken_background), Act::Setting(Setting::Darken)));
                v.push(Item::value("Show game titles", on(c.show_titles), Act::Setting(Setting::Titles)));
                v.push(Item::value(
                    "Dim games that aren't installed",
                    on(c.darken_not_installed),
                    Act::Setting(Setting::DarkenNotInstalled),
                ));
                v.push(Item::value(
                    "Achievements on game pages",
                    on(st.config.achievements.show),
                    Act::Setting(Setting::Achievements),
                ));
                if st.config.achievements.show {
                    v.push(Item::value(
                        "Achievement counts in the library",
                        on(st.config.achievements.library_counts),
                        Act::Setting(Setting::AchievementCounts),
                    ));
                }
                v.push(Item::value("Text size", c.text_size.label(), Act::Setting(Setting::TextSize)));
                v.push(Item::value("Theme videos", on(c.theme_videos), Act::Setting(Setting::Videos)));
                if c.theme_videos && !crate::media::videos_available() {
                    Self::missing_items(&mut v, crate::media::deps::Need::Video);
                }
                v.push(Item::value("Button prompts", Self::button_prompts_value(st), Act::CycleButtonPrompts));
            }
            2 => {
                v.push(Item::info(
                    "Power options",
                    "Which of these the menu offers. Each one asks before it does anything.",
                ));
                for p in Power::ALL {
                    if power::available(p, power::has_tool) {
                        v.push(Item::value(p.label(), shown(p.shown(&c.power)), Act::Setting(Setting::ShowPower(p))));
                    }
                }
                v.push(Item::value("Quit Lodestar", shown(c.allow_quit), Act::Setting(Setting::AllowQuit)));
            }
            4 => Self::audio_items(st, &mut v),
            _ => {
                v.push(Item::value("Swap confirm / back buttons", on(c.swap_ab), Act::ToggleSwapAb));
                Self::menu_button_items(st, &mut v);
                let name = c.menu_button_name();
                v.push(Item::value("Hide the mouse pointer", on(c.hide_cursor), Act::Setting(Setting::HideCursor)));
                v.push(Item::value(
                    "Only use the controller when Lodestar is the active window",
                    on(!c.background_input),
                    Act::Setting(Setting::ForegroundOnly),
                ));
                v.push(Item::info(
                    "While a game runs",
                    format!(
                        "Lodestar ignores the controller so your presses only reach the game. Only the menu \
                         button ({name}) gets through, to bring up Lodestar's menu."
                    ),
                ));
            }
        }
        v
    }

    /// Settings → Audio.
    fn audio_items(st: &AppState, v: &mut Vec<Item>) {
        let c = &st.config.controller;
        let theme = &st.controller_theme.file;
        let (sounds_on, music_on) = (c.sounds_enabled(theme), c.music_enabled(theme));
        if !sounds_on && !music_on {
            v.push(Item::info(
                "Sounds and music are off",
                "Automatic plays them when the controller theme brings its own; \
                 this one has none. Choose On to hear Lodestar's soft clicks and your music folder. They \
                 play only in controller mode, and never while a game runs.",
            ));
        }
        let themed = !theme.sounds.is_empty();
        v.push(Item::value("Navigation sounds", c.nav_sounds.label(themed), Act::Setting(Setting::NavSounds)));
        v.push(Item::value("Background music", c.music.label(!theme.music.is_empty()), Act::Setting(Setting::Music)));
        v.push(Item::value("Volume", format!("{} %", c.volume.min(100)), Act::Setting(Setting::Volume(1))));
        let folder = c.music_folder.as_deref().map(str::trim).filter(|f| !f.is_empty());
        v.push(Item::value(
            "Music folder",
            folder.map(folder_label).unwrap_or_else(|| "Automatic".into()),
            Act::Setting(Setting::MusicFolder),
        ));
        if folder.is_some() {
            v.push(Item::action("Use the automatic music folder", Act::Setting(Setting::MusicAuto)));
        }
        let status = crate::media::audio::global().status();
        if sounds_on {
            v.push(Item::info(
                "Navigation sounds",
                if themed {
                    "This theme brings its own sounds."
                } else {
                    "This theme has no sounds of its own, so Lodestar plays its soft built-in clicks."
                },
            ));
        }
        if music_on {
            let mut text = match folder {
                Some(f) => format!("Songs (mp3, ogg, flac, wav) from {f} and its folders, in random order."),
                None => "The theme's own music, and songs (mp3, ogg, flac, wav) in ~/ES-DE/music — \
                         ES-DE's music folder — in random order. Choose Music folder to use another one."
                    .to_string(),
            };
            match status.songs {
                Some(0) => text.push_str(" No songs found there yet."),
                Some(n) => text.push_str(&format!(" {n} song{} found.", if n == 1 { "" } else { "s" })),
                None => {}
            }
            v.push(Item::info("Where the music comes from", text));
        }
        if (sounds_on || music_on) && status.no_player {
            Self::missing_items(v, crate::media::deps::Need::Audio);
        }
    }

    /// A missing program: what it's for, how to install it on this Linux, and a Copy item.
    fn missing_items(v: &mut Vec<Item>, need: crate::media::deps::Need) {
        use crate::media::deps;
        let title = match need {
            deps::Need::Video => "Theme videos need ffmpeg",
            deps::Need::Audio => "No sound player found",
        };
        let h = deps::hint(deps::family(), need);
        v.push(Item::info(title, deps::message(need)));
        if let Some(c) = h.command {
            v.push(Item::action("Copy the install command", Act::CopyText(c)));
        }
    }

    /// The music folder picker's answer, once it has one (per frame; cheap).
    pub(super) fn poll_music_folder(&mut self, st: &mut AppState) {
        let Some(picked) = PICKED_FOLDER.lock().take() else { return };
        if let Some(p) = picked {
            st.config.controller.music_folder = Some(p.to_string_lossy().into_owned());
            st.save_config();
        }
        self.refresh_settings(st);
    }

    /// The theme options a controller can change (switches, choices, sliders; colours and
    /// labels are in desktop Settings → Appearance).
    fn controller_options(
        spec: &crate::theme::playnite::options::OptionsSpec,
    ) -> impl Iterator<Item = &crate::theme::playnite::options::ThemeOption> {
        use crate::theme::playnite::options::Kind;
        spec.effective().filter(|o| matches!(o.kind, Kind::Switch { .. } | Kind::Choice { .. } | Kind::Range { .. }))
    }

    /// Settings → Visuals → Theme options.
    pub(super) fn theme_option_items(st: &AppState) -> Vec<Item> {
        let key = &st.config.controller.theme;
        let Some(spec) = crate::ui::theme_options::spec_for(st, key) else { return vec![] };
        let values = crate::ui::theme_options::values(key, &spec);
        let mut v: Vec<Item> = Self::controller_options(&spec)
            .enumerate()
            .map(|(i, o)| Item::value(o.label.clone(), o.value_text(&values), Act::ThemeOption(i, 1)))
            .collect();
        let rest = spec.effective().count() - v.len();
        if rest > 0 {
            v.push(Item::info(
                "More options",
                if rest == 1 {
                    "One more (a colour or a label) is in desktop mode: Settings → Appearance.".to_string()
                } else {
                    format!("{rest} more (colours, labels) are in desktop mode: Settings → Appearance.")
                },
            ));
        }
        v
    }

    /// A theme option a step on / back.
    pub(super) fn step_theme_option(&mut self, st: &mut AppState, i: usize, dir: i8) {
        let key = st.config.controller.theme.clone();
        if let Some(spec) = crate::ui::theme_options::spec_for(st, &key)
            && let Some(o) = Self::controller_options(&spec).nth(i)
        {
            let v = o.stepped(&crate::ui::theme_options::values(&key, &spec), dir);
            crate::ui::theme_options::set_one(st, &key, &spec, &o.key, v);
        }
        let items = Self::theme_option_items(st);
        if let Some(m) = self.menus.last_mut().filter(|m| m.kind == MenuKind::ThemeOptions) {
            m.replace_items(items);
        }
    }

    /// Settings → Input: the menu button block. Which button, the combination, what that means,
    /// then what the button does besides opening the menu (desktop Settings → Controller uses
    /// the same names, `MENU_BUTTON_*`).
    fn menu_button_items(st: &AppState, v: &mut Vec<Item>) {
        let c = &st.config.controller;
        v.push(Item::value(MENU_BUTTON_LABEL, c.menu_button.label(), Act::Setting(Setting::MenuButton)));
        match c.active_menu_combo() {
            Some(combo) => {
                v.push(Item::value(MENU_COMBO_LABEL, combo.label(), Act::PickMenuCombo));
                v.push(Item::info(combo.label(), format!("{} {}", combo_note(combo), home_note(c))));
            }
            None => v.push(Item::info("Home button", home_button_note())),
        }
        v.push(Item::value(MENU_BUTTON_IN_GAME, on(c.guide_opens_lodestar), Act::Setting(Setting::GuideLodestar)));
        v.push(Item::value(
            MENU_BUTTON_DESKTOP,
            on(c.guide_opens_controller_mode),
            Act::Setting(Setting::GuideDesktop),
        ));
    }

    /// Rebuild an open settings menu (after a value changed).
    pub(super) fn refresh_settings(&mut self, st: &AppState) {
        if let Some(m) = self.menus.iter_mut().rev().find(|m| m.kind == MenuKind::Settings) {
            let items = Self::settings_items(st, m.tab);
            m.replace_items(items);
        }
    }

    /// LB / RB or a click on a tab.
    pub(super) fn settings_tab(&mut self, st: &AppState, tab: usize) {
        if let Some(m) = self.menus.last_mut().filter(|m| m.kind == MenuKind::Settings) {
            let tab = tab % TABS.len();
            m.set_tab(tab, Self::settings_items(st, tab));
        }
    }

    pub(super) fn apply_setting(&mut self, st: &mut AppState, s: Setting) {
        if s == Setting::MusicFolder {
            // A folder picker on its own thread (the screen keeps running).
            if !PICKING.swap(true, std::sync::atomic::Ordering::AcqRel) {
                let start = st
                    .config
                    .controller
                    .music_folder
                    .clone()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| st.paths.home.clone());
                let spawned = std::thread::Builder::new().name("lodestar-pick-music".into()).spawn(move || {
                    let picked = rfd::FileDialog::new().set_title("Music folder").set_directory(start).pick_folder();
                    *PICKED_FOLDER.lock() = Some(picked);
                    PICKING.store(false, std::sync::atomic::Ordering::Release);
                });
                if spawned.is_err() {
                    PICKING.store(false, std::sync::atomic::Ordering::Release);
                }
            }
            return;
        }
        apply(&mut st.config, s);
        if s == Setting::Achievements {
            // Answers so far followed the old setting.
            st.achievements.clear();
        }
        st.save_config();
        self.refresh_settings(st);
        self.refresh_quick(st);
    }
}

/// Change one setting in the config (pure; saved by the caller).
pub(super) fn apply(cfg: &mut crate::config::Config, s: Setting) {
    let c = &mut cfg.controller;
    match s {
        Setting::Clock => c.show_clock = !c.show_clock,
        Setting::Battery => c.show_battery = !c.show_battery,
        Setting::MinimiseOnLaunch => cfg.minimize_on_launch = !cfg.minimize_on_launch,
        Setting::StartMode => {
            cfg.start_mode = match cfg.start_mode {
                StartMode::Controller => StartMode::Desktop,
                StartMode::Desktop => StartMode::Controller,
            }
        }
        Setting::Blur => c.blur_background = !c.blur_background,
        Setting::Darken => c.darken_background = !c.darken_background,
        Setting::Titles => c.show_titles = !c.show_titles,
        Setting::DarkenNotInstalled => c.darken_not_installed = !c.darken_not_installed,
        Setting::TextSize => c.text_size = c.text_size.next(),
        Setting::ShowPower(p) => p.toggle(&mut c.power),
        Setting::AllowQuit => c.allow_quit = !c.allow_quit,
        Setting::GuideLodestar => c.guide_opens_lodestar = !c.guide_opens_lodestar,
        Setting::GuideDesktop => c.guide_opens_controller_mode = !c.guide_opens_controller_mode,
        Setting::HideCursor => c.hide_cursor = !c.hide_cursor,
        Setting::ForegroundOnly => c.background_input = !c.background_input,
        Setting::Videos => c.theme_videos = !c.theme_videos,
        Setting::NavSounds => c.nav_sounds = c.nav_sounds.next(),
        Setting::Music => c.music = c.music.next(),
        Setting::Volume(d) => {
            c.volume = (c.volume.min(100) as i32 + d.signum() as i32 * 10).clamp(0, 100) as u8;
        }
        // The picker runs in `apply_setting`.
        Setting::MusicFolder => {}
        Setting::MusicAuto => c.music_folder = None,
        // Chosen here: "Which button opens Lodestar's menu?" has nothing left to ask.
        Setting::MenuButton => (c.menu_button, c.menu_button_asked) = (c.menu_button.next(), true),
        Setting::Achievements => cfg.achievements.show = !cfg.achievements.show,
        Setting::AchievementCounts => cfg.achievements.library_counts = !cfg.achievements.library_counts,
    }
}

// ------------------------------------------------------------------ mouse pointer

/// Controller mode hides the mouse pointer (setting) until the mouse moves; it hides again
/// after a few seconds without movement, or at once when the controller is used.
#[derive(Debug)]
pub(super) struct CursorHider {
    last_pos: Option<egui::Pos2>,
    /// Never moved yet: long ago (0.0 would count as a move when egui's clock starts, and
    /// schedule a frame 3 s after start for nothing).
    moved_at: f64,
}

impl Default for CursorHider {
    fn default() -> Self {
        CursorHider { last_pos: None, moved_at: f64::NEG_INFINITY }
    }
}

/// Seconds a still mouse pointer stays visible.
const CURSOR_IDLE_SECS: f64 = 3.0;

impl CursorHider {
    /// Per frame (controller mode). `navs`: the controller / keyboard was used this frame.
    pub fn frame(&mut self, ctx: &egui::Context, enabled: bool, navs: bool, now: f64) {
        let pos = ctx.input(|i| i.pointer.latest_pos());
        if pos.is_some() && pos != self.last_pos {
            if self.last_pos.is_some() {
                self.moved_at = now;
            }
            self.last_pos = pos;
        }
        if navs {
            self.moved_at = f64::NEG_INFINITY;
        }
        if !enabled {
            return;
        }
        let idle = now - self.moved_at;
        if idle >= CURSOR_IDLE_SECS {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(CURSOR_IDLE_SECS - idle + 0.05));
        }
    }
}

// ------------------------------------------------------------------ clock and battery

/// The theme shows a clock (then the status corner leaves the time to it). On a page the theme
/// draws, a clock in any of its views counts: a view without one was laid out without it (its
/// top-right corner may hold something else, like Shelf's "1 of 30"). `None`: a screen the
/// theme doesn't draw (the built-in details page, the empty library), which gets the corner.
pub(super) fn theme_has_clock(t: &crate::theme::model::ThemeFile, view: Option<ViewKind>) -> bool {
    use crate::theme::model::Element;
    view_has(t, view, |e| matches!(e, Element::DateTime { clock: true, .. }))
}

/// The theme shows the battery level itself (then the status corner leaves it to the theme);
/// views as for `theme_has_clock`.
pub(super) fn theme_has_battery(t: &crate::theme::model::ThemeFile, view: Option<ViewKind>) -> bool {
    use crate::theme::model::Element;
    view_has(
        t,
        view,
        |e| matches!(e, Element::Text { text, .. } if text.contains("system.battery") || text.contains("system.charging")),
    )
}

fn view_has(
    t: &crate::theme::model::ThemeFile,
    view: Option<ViewKind>,
    pred: impl Fn(&crate::theme::model::Element) -> bool,
) -> bool {
    view.is_some()
        && [Some(&t.collections), Some(&t.games), t.details.as_ref(), t.home.as_ref()]
            .into_iter()
            .flatten()
            .any(|v| v.elements.iter().any(|e| e.any_nested(&pred)))
}

/// "14:32" / "83 %" for the status corner of `view` (None: nothing to show).
pub(super) fn status_text(st: &AppState, battery: Option<(u8, bool)>, view: Option<ViewKind>) -> Option<String> {
    let c = &st.config.controller;
    let t = &st.controller_theme.file;
    let mut parts = vec![];
    if c.show_battery
        && !theme_has_battery(t, view)
        && let Some((pct, charging)) = battery
    {
        parts.push(if charging { format!("Charging {pct} %") } else { format!("Battery {pct} %") });
    }
    if c.show_clock && !theme_has_clock(t, view) {
        parts.push(crate::theme::render::bind::format_date(now_unix(), "%H:%M"));
    }
    (!parts.is_empty()).then(|| parts.join("   "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_and_battery_inside_a_stack_count() {
        let t = crate::theme::model::ThemeFile::parse(
            r#"
[[games.elements]]
type = "stack"
[[games.elements.children]]
type = "datetime"
clock = true
[[games.elements.children]]
type = "text"
text = "{system.battery}"
"#,
        )
        .unwrap();
        assert!(theme_has_clock(&t, Some(ViewKind::Games)));
        assert!(theme_has_battery(&t, Some(ViewKind::Games)));
        let plain = crate::theme::model::ThemeFile::parse("[[games.elements]]\ntype = \"stack\"").unwrap();
        assert!(!theme_has_clock(&plain, Some(ViewKind::Games)));
    }

    #[test]
    fn every_setting_toggles_and_back() {
        let mut cfg = crate::config::Config::default();
        // Choosing the menu button in Settings also answers "Which button opens Lodestar's menu?".
        apply(&mut cfg, Setting::MenuButton);
        assert!(cfg.controller.menu_button_asked);
        assert_eq!(cfg.controller.menu_button, crate::config::MenuButton::Home);
        apply(&mut cfg, Setting::MenuButton);
        let before = format!("{cfg:?}");
        let all = [
            Setting::Clock,
            Setting::Battery,
            Setting::MinimiseOnLaunch,
            Setting::StartMode,
            Setting::Blur,
            Setting::Darken,
            Setting::Titles,
            Setting::DarkenNotInstalled,
            Setting::ShowPower(Power::Lock),
            Setting::AllowQuit,
            Setting::GuideLodestar,
            Setting::GuideDesktop,
            Setting::HideCursor,
            Setting::ForegroundOnly,
            Setting::Videos,
            Setting::MenuButton,
            Setting::Achievements,
            Setting::AchievementCounts,
        ];
        for s in all {
            let a = format!("{cfg:?}");
            apply(&mut cfg, s);
            assert_ne!(format!("{cfg:?}"), a, "{s:?} changes something");
            apply(&mut cfg, s);
            assert_eq!(format!("{cfg:?}"), a, "{s:?} twice is a no-op");
        }
        // Sounds and music: Automatic → On → Off → Automatic.
        for s in [Setting::NavSounds, Setting::Music] {
            let a = format!("{cfg:?}");
            for _ in 0..2 {
                apply(&mut cfg, s);
                assert_ne!(format!("{cfg:?}"), a, "{s:?} changes something");
            }
            apply(&mut cfg, s);
            assert_eq!(format!("{cfg:?}"), a, "{s:?} three times goes round");
        }
        for _ in crate::config::TextSize::ALL {
            apply(&mut cfg, Setting::TextSize);
        }
        assert_eq!(format!("{cfg:?}"), before, "text size cycles round");
        // "Only use the controller when Lodestar is the active window" is the opposite of
        // background input (on by default).
        assert!(cfg.controller.background_input);
        apply(&mut cfg, Setting::ForegroundOnly);
        assert!(!cfg.controller.background_input);
    }

    #[test]
    fn settings_tabs_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let mut ui = ControllerUi::new();
        ui.open_settings(&st, 0.0);
        let labels =
            |ui: &ControllerUi| ui.menus.last().unwrap().items.iter().map(|i| i.label.clone()).collect::<Vec<_>>();
        assert!(labels(&ui).contains(&"Show the clock".to_string()));
        ui.settings_tab(&st, 3);
        assert!(labels(&ui).contains(&"Only use the controller when Lodestar is the active window".to_string()));
        let value = |ui: &ControllerUi, l: &str| {
            ui.menus.last().unwrap().items.iter().find(|i| i.label == l).and_then(|i| i.value.clone()).unwrap()
        };
        assert_eq!(value(&ui, "Only use the controller when Lodestar is the active window"), "Off");
        ui.menu_act(&mut st, Act::Setting(Setting::ForegroundOnly), 0.0);
        assert_eq!(value(&ui, "Only use the controller when Lodestar is the active window"), "On");
        let saved = crate::config::Config::load(&st.paths.config_file());
        assert!(!saved.controller.background_input);
        // LB / RB wrap around the tabs.
        ui.menu_nav(&mut st, Nav::PageDown, 0.0);
        assert_eq!(ui.menus.last().unwrap().tab, 4);
        ui.menu_nav(&mut st, Nav::PageDown, 0.0);
        assert_eq!(ui.menus.last().unwrap().tab, 0);
        ui.menu_nav(&mut st, Nav::PageUp, 0.0);
        assert_eq!(ui.menus.last().unwrap().tab, 4);
        ui.menu_nav(&mut st, Nav::PageUp, 0.0);
        assert_eq!(ui.menus.last().unwrap().tab, 3);
        // ← / → change values in the settings too.
        assert!(!st.config.controller.swap_ab);
        ui.menu_nav(&mut st, Nav::Right, 0.0);
        assert!(st.config.controller.swap_ab, "the first Input row is the swap switch");
        assert_eq!(value(&ui, "Swap confirm / back buttons"), "On");
    }

    #[test]
    fn audio_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let mut ui = ControllerUi::new();
        ui.open_settings(&st, 0.0);
        ui.settings_tab(&st, 4);
        let labels =
            |ui: &ControllerUi| ui.menus.last().unwrap().items.iter().map(|i| i.label.clone()).collect::<Vec<_>>();
        let l = labels(&ui);
        assert_eq!(l[0], "Sounds and music are off", "says that both are off by default");
        assert!(l.contains(&"Navigation sounds".to_string()) && l.contains(&"Background music".to_string()));
        let value = |ui: &ControllerUi, l: &str| {
            ui.menus.last().unwrap().items.iter().find(|i| i.label == l).and_then(|i| i.value.clone()).unwrap()
        };
        assert_eq!(value(&ui, "Volume"), "70 %");
        assert_eq!(value(&ui, "Music folder"), "Automatic");
        // Move to Volume, → louder, ← ← quieter.
        let row = ui.menus.last().unwrap().items.iter().position(|i| i.label == "Volume").unwrap();
        ui.menus.last_mut().unwrap().sel = row;
        ui.menu_nav(&mut st, Nav::Right, 0.0);
        assert_eq!(st.config.controller.volume, 80);
        ui.menu_nav(&mut st, Nav::Left, 0.0);
        ui.menu_nav(&mut st, Nav::Left, 0.0);
        assert_eq!(value(&ui, "Volume"), "60 %");
        for _ in 0..12 {
            ui.menu_nav(&mut st, Nav::Left, 0.0);
        }
        assert_eq!(st.config.controller.volume, 0, "stops at 0");
        // A chosen folder shows by name and can go back to automatic.
        st.config.controller.music_folder = Some("/home/u/Music/Retro".into());
        ui.refresh_settings(&st);
        assert_eq!(value(&ui, "Music folder"), "Retro");
        assert!(labels(&ui).contains(&"Use the automatic music folder".to_string()));
        apply(&mut st.config, Setting::MusicAuto);
        assert_eq!(st.config.controller.music_folder, None);
        assert_eq!(st.config.controller.music, crate::config::AudioMode::Auto, "unchanged by the folder");
    }
}
