//! The menu button (Settings → Input → "Lodestar's menu button"): a held button combination
//! (default L3 + R3: a preset or one the user recorded) or the Home button. input/pad.rs turns
//! either into `Nav::Guide`; ui/controller/ingame.rs asks once ("Which button opens Lodestar's
//! menu?").

use super::ControllerConfig;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Older versions: what the Home button did in Steam games (only read, to carry the choice over).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SteamHome {
    Ask,
    SteamOverlay,
    LodestarMenu,
}

/// What opens Lodestar's menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MenuButton {
    /// The button combination (`ControllerConfig::menu_combo`); Home is left to Steam and
    /// emulators.
    #[default]
    #[serde(alias = "home-and-combo", alias = "combo-only")]
    Combo,
    /// The Home (Guide / PS) button.
    #[serde(alias = "home-only")]
    Home,
}

impl MenuButton {
    pub const ALL: [MenuButton; 2] = [MenuButton::Combo, MenuButton::Home];

    pub fn label(self) -> &'static str {
        match self {
            MenuButton::Combo => "Button combination",
            MenuButton::Home => "Home button",
        }
    }

    pub fn next(self) -> MenuButton {
        match self {
            MenuButton::Combo => MenuButton::Home,
            MenuButton::Home => MenuButton::Combo,
        }
    }
}

/// A button that can be part of the menu combination (any but Home). Named as on an Xbox pad;
/// the order here is the order labels list them in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PadButton {
    L3,
    R3,
    Lb,
    Rb,
    Lt,
    Rt,
    Select,
    Start,
    A,
    B,
    X,
    Y,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

impl PadButton {
    pub const ALL: [PadButton; 16] = [
        PadButton::L3,
        PadButton::R3,
        PadButton::Lb,
        PadButton::Rb,
        PadButton::Lt,
        PadButton::Rt,
        PadButton::Select,
        PadButton::Start,
        PadButton::A,
        PadButton::B,
        PadButton::X,
        PadButton::Y,
        PadButton::DpadUp,
        PadButton::DpadDown,
        PadButton::DpadLeft,
        PadButton::DpadRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PadButton::L3 => "L3",
            PadButton::R3 => "R3",
            PadButton::Lb => "LB",
            PadButton::Rb => "RB",
            PadButton::Lt => "LT",
            PadButton::Rt => "RT",
            PadButton::Select => "Select",
            PadButton::Start => "Start",
            PadButton::A => "A",
            PadButton::B => "B",
            PadButton::X => "X",
            PadButton::Y => "Y",
            PadButton::DpadUp => "Up",
            PadButton::DpadDown => "Down",
            PadButton::DpadLeft => "Left",
            PadButton::DpadRight => "Right",
        }
    }

    fn bit(self) -> u16 {
        1 << (self as u16)
    }

    /// A, B, X, Y and the d-pad: what games use all the time.
    fn busy(self) -> bool {
        matches!(
            self,
            PadButton::A
                | PadButton::B
                | PadButton::X
                | PadButton::Y
                | PadButton::DpadUp
                | PadButton::DpadDown
                | PadButton::DpadLeft
                | PadButton::DpadRight
        )
    }

    fn shoulder(self) -> bool {
        matches!(self, PadButton::Lb | PadButton::Rb | PadButton::Lt | PadButton::Rt)
    }
}

/// The buttons held together to open Lodestar's menu (a set; saved as a list of button names).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuCombo(u16);

impl MenuCombo {
    pub const STICKS: MenuCombo = MenuCombo((1 << PadButton::L3 as u16) | (1 << PadButton::R3 as u16));
    pub const SELECT_START: MenuCombo = MenuCombo((1 << PadButton::Select as u16) | (1 << PadButton::Start as u16));
    pub const BUMPERS_START: MenuCombo =
        MenuCombo((1 << PadButton::Lb as u16) | (1 << PadButton::Rb as u16) | (1 << PadButton::Start as u16));
    pub const PRESETS: [MenuCombo; 3] = [MenuCombo::STICKS, MenuCombo::SELECT_START, MenuCombo::BUMPERS_START];
    /// Fewer can't be told apart from a single press.
    pub const MIN_BUTTONS: usize = 2;

    pub const fn empty() -> MenuCombo {
        MenuCombo(0)
    }

    pub fn of(buttons: impl IntoIterator<Item = PadButton>) -> MenuCombo {
        MenuCombo(buttons.into_iter().fold(0, |m, b| m | b.bit()))
    }

    pub fn with(self, b: PadButton) -> MenuCombo {
        MenuCombo(self.0 | b.bit())
    }

    pub fn without(self, b: PadButton) -> MenuCombo {
        MenuCombo(self.0 & !b.bit())
    }

    pub fn contains(self, b: PadButton) -> bool {
        self.0 & b.bit() != 0
    }

    /// Every button of `other` is in this one.
    pub fn covers(self, other: MenuCombo) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    pub fn buttons(self) -> impl Iterator<Item = PadButton> {
        PadButton::ALL.into_iter().filter(move |b| self.contains(*b))
    }

    /// Enough buttons to be a combination.
    pub fn valid(self) -> bool {
        self.len() >= Self::MIN_BUTTONS
    }

    /// Only buttons games use all the time (face buttons, the d-pad), or one shoulder button with
    /// one of those: likely held together in a game by accident.
    pub fn busy(self) -> bool {
        let busy = self.buttons().filter(|b| b.busy()).count();
        let shoulders = self.buttons().filter(|b| b.shoulder()).count();
        busy == self.len() || (self.len() == 2 && busy == 1 && shoulders == 1)
    }

    /// "L3 + R3".
    pub fn label(self) -> String {
        self.buttons().map(PadButton::label).collect::<Vec<_>>().join(" + ")
    }

    /// How to press it, in plain words.
    pub fn how(self) -> String {
        match self {
            MenuCombo::STICKS => "Click both sticks in and hold for a moment.".into(),
            MenuCombo::SELECT_START => "Hold Select and Start (View and Menu) for a moment.".into(),
            _ => {
                let names: Vec<&str> = self.buttons().map(PadButton::label).collect();
                match names.split_last() {
                    Some((last, rest)) if !rest.is_empty() => {
                        format!("Hold {} and {last} for a moment.", rest.join(", "))
                    }
                    _ => format!("Hold {} for a moment.", self.label()),
                }
            }
        }
    }
}

impl Serialize for MenuCombo {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.buttons().collect::<Vec<_>>().serialize(s)
    }
}

impl<'de> Deserialize<'de> for MenuCombo {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        /// The presets were saved by name before combinations could be recorded.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Saved {
            Buttons(Vec<PadButton>),
            Preset(String),
        }
        match Saved::deserialize(d)? {
            Saved::Buttons(b) => Ok(MenuCombo::of(b)),
            Saved::Preset(name) => match name.as_str() {
                "sticks" => Ok(MenuCombo::STICKS),
                "select-start" => Ok(MenuCombo::SELECT_START),
                "bumpers-start" => Ok(MenuCombo::BUMPERS_START),
                other => Err(serde::de::Error::custom(format!("unknown button combination {other:?}"))),
            },
        }
    }
}

impl ControllerConfig {
    /// Home opens Lodestar's menu: chosen, or not asked yet (so pressing it, the natural first
    /// try, brings up the question).
    pub fn home_opens_menu(&self) -> bool {
        self.menu_button == MenuButton::Home || !self.menu_button_asked
    }

    /// The button combination that opens Lodestar's menu (None: Home was chosen).
    pub fn active_menu_combo(&self) -> Option<MenuCombo> {
        (self.menu_button == MenuButton::Combo).then_some(self.menu_combo)
    }

    /// What prompts call the menu button: "L3 + R3" or "Home".
    pub fn menu_button_name(&self) -> String {
        match self.menu_button {
            MenuButton::Combo => self.menu_combo.label(),
            MenuButton::Home => "Home".into(),
        }
    }

    /// Older versions' answer to "Home button in Steam games" becomes the menu button (Steam's
    /// overlay: the combination; Lodestar's menu: Home); "Ask" still asks. A combination too
    /// small to tell from a single press goes back to L3 + R3.
    pub(super) fn sanitize_menu_button(&mut self) {
        if let Some(old) = self.steam_home.take()
            && !self.menu_button_asked
        {
            match old {
                SteamHome::Ask => {}
                SteamHome::SteamOverlay => (self.menu_button, self.menu_button_asked) = (MenuButton::Combo, true),
                SteamHome::LodestarMenu => (self.menu_button, self.menu_button_asked) = (MenuButton::Home, true),
            }
        }
        if !self.menu_combo.valid() {
            self.menu_combo = MenuCombo::STICKS;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn labels_list_the_buttons_in_a_fixed_order() {
        assert_eq!(MenuCombo::STICKS.label(), "L3 + R3");
        assert_eq!(MenuCombo::BUMPERS_START.label(), "LB + RB + Start");
        assert_eq!(MenuCombo::of([PadButton::Start, PadButton::Lb, PadButton::Rb]), MenuCombo::BUMPERS_START);
        assert_eq!(MenuCombo::BUMPERS_START.how(), "Hold LB, RB and Start for a moment.");
        assert_eq!(MenuCombo::of([PadButton::Y, PadButton::Rt]).how(), "Hold RT and Y for a moment.");
    }

    #[test]
    fn combinations_games_use_all_the_time_are_busy() {
        use PadButton::*;
        for busy in [vec![A, B], vec![X, Y, A], vec![DpadDown, A], vec![Lb, A], vec![Rt, X]] {
            assert!(MenuCombo::of(busy.clone()).busy(), "{busy:?}");
        }
        for fine in
            [vec![L3, R3], vec![Select, Start], vec![Lb, Rb, Start], vec![Lb, Rb], vec![Select, A], vec![Lb, Rb, A]]
        {
            assert!(!MenuCombo::of(fine.clone()).busy(), "{fine:?}");
        }
        assert!(!MenuCombo::of([A]).valid() && !MenuCombo::empty().valid() && MenuCombo::of([A, B]).valid());
    }

    #[test]
    fn saved_as_button_names_and_old_preset_names_still_load() {
        let c = ControllerConfig { menu_combo: MenuCombo::BUMPERS_START, ..Default::default() };
        let text = toml::to_string(&c).unwrap();
        assert!(text.contains("menu_combo = [\"lb\", \"rb\", \"start\"]"), "{text}");
        for (saved, combo) in [
            ("\"sticks\"", MenuCombo::STICKS),
            ("\"select-start\"", MenuCombo::SELECT_START),
            ("\"bumpers-start\"", MenuCombo::BUMPERS_START),
            ("[\"y\", \"rt\"]", MenuCombo::of([PadButton::Y, PadButton::Rt])),
            // Too few buttons (edited by hand): the default.
            ("[\"a\"]", MenuCombo::STICKS),
        ] {
            let (cfg, problems) = Config::parse_lenient(&format!("[controller]\nmenu_combo = {saved}\n"));
            assert_eq!((cfg.controller.menu_combo, problems.len()), (combo, 0), "{saved}");
        }
        // Not a button: that value alone is dropped.
        let (cfg, problems) = Config::parse_lenient("[controller]\nmenu_combo = [\"home\", \"a\"]\nswap_ab = true\n");
        assert_eq!((cfg.controller.menu_combo, cfg.controller.swap_ab, problems.len()), (MenuCombo::STICKS, true, 1));
    }

    #[test]
    fn earlier_answers_carry_over() {
        let d = ControllerConfig::default();
        assert_eq!((d.menu_button, d.menu_combo, d.menu_button_asked), (MenuButton::Combo, MenuCombo::STICKS, false));
        for (saved, button, asked) in [
            ("steam_home = \"steam-overlay\"", MenuButton::Combo, true),
            ("steam_home = \"lodestar-menu\"", MenuButton::Home, true),
            ("steam_home = \"ask\"", MenuButton::Combo, false),
            ("menu_button = \"home-and-combo\"\nmenu_button_asked = true", MenuButton::Combo, true),
            ("menu_button = \"combo-only\"\nmenu_button_asked = true", MenuButton::Combo, true),
            ("menu_button = \"home-only\"\nmenu_button_asked = true", MenuButton::Home, true),
            // The old Steam tip dismissed: the question still comes.
            ("steam_guide_tip = false", MenuButton::Combo, false),
            // An answer to the question wins over an old value left in the file.
            (
                "steam_home = \"lodestar-menu\"\nmenu_button = \"combo\"\nmenu_button_asked = true",
                MenuButton::Combo,
                true,
            ),
        ] {
            let (cfg, problems) = Config::parse_lenient(&format!("[controller]\n{saved}\n"));
            let c = cfg.controller;
            assert_eq!(
                (c.menu_button, c.menu_button_asked, c.steam_home, problems.len()),
                (button, asked, None, 0),
                "{saved}"
            );
        }
        let (cfg, _) = Config::parse_lenient("[controller]\nsteam_home = \"steam-overlay\"\n");
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(
            text.contains("menu_button = \"combo\"") && text.contains("menu_combo = [\n    \"l3\",\n    \"r3\",\n]"),
            "{text}"
        );
        assert!(!text.contains("steam_home") && !text.contains("steam_guide_tip"), "{text}");
    }

    #[test]
    fn home_opens_the_menu_when_chosen_or_until_asked() {
        let c =
            |menu_button, menu_button_asked| ControllerConfig { menu_button, menu_button_asked, ..Default::default() };
        assert!(c(MenuButton::Combo, false).home_opens_menu(), "the first press can ask");
        assert!(!c(MenuButton::Combo, true).home_opens_menu());
        assert!(c(MenuButton::Home, true).home_opens_menu());
        assert_eq!(c(MenuButton::Home, true).active_menu_combo(), None);
        assert_eq!(c(MenuButton::Combo, false).active_menu_combo(), Some(MenuCombo::STICKS));
        assert_eq!(c(MenuButton::Combo, true).menu_button_name(), "L3 + R3");
        assert_eq!(c(MenuButton::Home, true).menu_button_name(), "Home");
    }
}
