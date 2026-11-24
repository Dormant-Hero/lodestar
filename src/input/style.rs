//! Button prompts that match the controller: which labels (Xbox A/B/X/Y, PlayStation ✕○△□,
//! Nintendo B/A/Y/X) the hints draw, and which style a connected pad has.
//!
//! Prompts are positional: `Button::A` is the south face button whatever it is called on the
//! pad (Xbox A, PlayStation ✕, Nintendo B), so the button in the same place always does the
//! same thing and only its label changes. `Prompts::face` also follows `swap_ab` (confirm on
//! the east button), so a hint for "confirm" always shows the button that confirms.
//!
//! Pure (no gilrs, no egui painting) so it is unit-tested without a gamepad.

use egui::Color32;

/// A button as hints name it, by physical position (Xbox names): A south, B east, X west,
/// Y north, Start / Select the two middle buttons, LB / RB the shoulder buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    A,
    B,
    X,
    Y,
    Start,
    Select,
    LB,
    RB,
}

/// Which controller's labels prompts show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonStyle {
    #[default]
    Xbox,
    PlayStation,
    Nintendo,
}

/// What is drawn inside a button glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Letters (A, LB, L1, R …) in the UI font.
    Text(&'static str),
    /// PlayStation face buttons, drawn as shapes (no font needed).
    Cross,
    Circle,
    Triangle,
    Square,
    /// Three bars: Xbox Menu, PlayStation Options.
    Menu,
    /// Two overlapping windows: Xbox View.
    View,
    /// PlayStation Create / Share: three short rays.
    Create,
    /// Nintendo + / −.
    Plus,
    Minus,
}

/// One button in one style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    pub mark: Mark,
    /// The mark's own colour (Xbox / PlayStation face buttons); None = the hint's text colour.
    pub color: Option<Color32>,
    /// Round glyph (face and middle buttons); false = a wider pill (shoulder buttons).
    pub round: bool,
    /// What the button is called in running text ("Press Cross to …").
    pub name: &'static str,
}

const XBOX_GREEN: Color32 = Color32::from_rgb(0x6c, 0xc0, 0x4a);
const XBOX_RED: Color32 = Color32::from_rgb(0xe8, 0x4a, 0x3c);
const XBOX_BLUE: Color32 = Color32::from_rgb(0x3c, 0x8c, 0xe8);
const XBOX_YELLOW: Color32 = Color32::from_rgb(0xf2, 0xc0, 0x2c);
const PS_BLUE: Color32 = Color32::from_rgb(0x7c, 0xa8, 0xe8);
const PS_RED: Color32 = Color32::from_rgb(0xe8, 0x62, 0x6a);
const PS_GREEN: Color32 = Color32::from_rgb(0x3c, 0xc8, 0xa8);
const PS_PINK: Color32 = Color32::from_rgb(0xe0, 0x8c, 0xc8);

impl ButtonStyle {
    pub fn label(self) -> &'static str {
        match self {
            ButtonStyle::Xbox => "Xbox",
            ButtonStyle::PlayStation => "PlayStation",
            ButtonStyle::Nintendo => "Nintendo",
        }
    }

    /// The button at position `b` in this style.
    pub fn face(self, b: Button) -> Face {
        use Button as B;
        let round = |mark, color, name| Face { mark, color, round: true, name };
        let pill = |t: &'static str| Face { mark: Mark::Text(t), color: None, round: false, name: t };
        match (self, b) {
            (ButtonStyle::Xbox, B::A) => round(Mark::Text("A"), Some(XBOX_GREEN), "A"),
            (ButtonStyle::Xbox, B::B) => round(Mark::Text("B"), Some(XBOX_RED), "B"),
            (ButtonStyle::Xbox, B::X) => round(Mark::Text("X"), Some(XBOX_BLUE), "X"),
            (ButtonStyle::Xbox, B::Y) => round(Mark::Text("Y"), Some(XBOX_YELLOW), "Y"),
            (ButtonStyle::Xbox, B::Start) => round(Mark::Menu, None, "Menu"),
            (ButtonStyle::Xbox, B::Select) => round(Mark::View, None, "View"),
            (ButtonStyle::Xbox, B::LB) => pill("LB"),
            (ButtonStyle::Xbox, B::RB) => pill("RB"),
            (ButtonStyle::PlayStation, B::A) => round(Mark::Cross, Some(PS_BLUE), "Cross"),
            (ButtonStyle::PlayStation, B::B) => round(Mark::Circle, Some(PS_RED), "Circle"),
            (ButtonStyle::PlayStation, B::X) => round(Mark::Square, Some(PS_PINK), "Square"),
            (ButtonStyle::PlayStation, B::Y) => round(Mark::Triangle, Some(PS_GREEN), "Triangle"),
            (ButtonStyle::PlayStation, B::Start) => round(Mark::Menu, None, "Options"),
            (ButtonStyle::PlayStation, B::Select) => round(Mark::Create, None, "Create"),
            (ButtonStyle::PlayStation, B::LB) => pill("L1"),
            (ButtonStyle::PlayStation, B::RB) => pill("R1"),
            // Nintendo prints A on the east button and B on the south one (X north, Y west).
            (ButtonStyle::Nintendo, B::A) => round(Mark::Text("B"), None, "B"),
            (ButtonStyle::Nintendo, B::B) => round(Mark::Text("A"), None, "A"),
            (ButtonStyle::Nintendo, B::X) => round(Mark::Text("Y"), None, "Y"),
            (ButtonStyle::Nintendo, B::Y) => round(Mark::Text("X"), None, "X"),
            (ButtonStyle::Nintendo, B::Start) => round(Mark::Plus, None, "+"),
            (ButtonStyle::Nintendo, B::Select) => round(Mark::Minus, None, "−"),
            (ButtonStyle::Nintendo, B::LB) => pill("L"),
            (ButtonStyle::Nintendo, B::RB) => pill("R"),
        }
    }

    /// Style of a pad from its USB vendor id and name (as gilrs reports them).
    pub fn detect(vendor: Option<u16>, name: &str) -> ButtonStyle {
        let n = name.to_lowercase();
        match vendor {
            Some(0x054c) => return ButtonStyle::PlayStation,
            Some(0x057e) => return ButtonStyle::Nintendo,
            Some(0x045e) => return ButtonStyle::Xbox,
            _ => {}
        }
        // "Xbox Wireless Controller" would otherwise match PlayStation's "Wireless Controller".
        if n.contains("xbox") || n.contains("x-box") {
            return ButtonStyle::Xbox;
        }
        let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
        if has(&["dualshock", "dualsense", "playstation", "ps3", "ps4", "ps5", "wireless controller"]) {
            ButtonStyle::PlayStation
        } else if has(&["pro controller", "joy-con", "joycon", "switch", "nintendo"]) {
            ButtonStyle::Nintendo
        } else {
            ButtonStyle::Xbox
        }
    }
}

/// A pad as the input thread knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadInfo {
    pub id: usize,
    pub name: String,
    pub vendor: Option<u16>,
    pub product: Option<u16>,
}

impl PadInfo {
    /// Steam Input's virtual pad (always Xbox-shaped, whatever pad is really in the hand).
    pub fn is_virtual(&self) -> bool {
        (self.vendor == Some(0x28de) && self.product == Some(0x11ff)) || self.name.to_lowercase().contains("virtual")
    }
    pub fn style(&self) -> ButtonStyle {
        ButtonStyle::detect(self.vendor, &self.name)
    }
}

/// Style of the pad in use: the last one pressed (`used`), else the first connected one. A
/// virtual pad (Steam Input mirroring the real one) stands for the first real pad connected.
/// None without any pad.
pub fn pad_style(pads: &[PadInfo], used: Option<usize>) -> Option<ButtonStyle> {
    let real = || pads.iter().find(|p| !p.is_virtual());
    let pick = |p: &PadInfo| {
        if p.is_virtual() { real().unwrap_or(p).style() } else { p.style() }
    };
    used.and_then(|id| pads.iter().find(|p| p.id == id)).or_else(real).or_else(|| pads.first()).map(pick)
}

/// `LODESTAR_FAKE_PAD=<vendor hex>:<name>` (vendor may be empty, e.g. `:Pro Controller`):
/// tests and screenshot runs act as if this pad were the one in use.
pub fn fake_pad_style(var: &str) -> Option<ButtonStyle> {
    let var = var.trim();
    if var.is_empty() {
        return None;
    }
    let (vendor, name) = var.split_once(':').unwrap_or(("", var));
    let vendor = u16::from_str_radix(vendor.trim().trim_start_matches("0x"), 16).ok();
    Some(ButtonStyle::detect(vendor, name))
}

/// What hints draw this frame: the style and whether confirm / back are swapped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Prompts {
    pub style: ButtonStyle,
    pub swap_ab: bool,
}

impl Prompts {
    pub fn new(style: ButtonStyle, swap_ab: bool) -> Prompts {
        Prompts { style, swap_ab }
    }

    /// The physical button that does what `b` does by default (A = confirm, B = back).
    pub fn position(self, b: Button) -> Button {
        match b {
            Button::A if self.swap_ab => Button::B,
            Button::B if self.swap_ab => Button::A,
            b => b,
        }
    }

    pub fn face(self, b: Button) -> Face {
        self.style.face(self.position(b))
    }

    /// The button's name for running text ("Press {} to scan again").
    pub fn name(self, b: Button) -> &'static str {
        self.face(b).name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(id: usize, name: &str, vendor: Option<u16>, product: Option<u16>) -> PadInfo {
        PadInfo { id, name: name.into(), vendor, product }
    }

    #[test]
    fn detect_by_vendor_then_name() {
        use ButtonStyle::*;
        let cases: &[(Option<u16>, &str, ButtonStyle)] = &[
            (Some(0x054c), "Whatever", PlayStation),
            (Some(0x057e), "Whatever", Nintendo),
            (None, "PS4 Controller", PlayStation),
            (None, "Sony Interactive Entertainment Wireless Controller", PlayStation),
            (None, "DualSense Wireless Controller", PlayStation),
            (None, "Sony PLAYSTATION(R)3 Controller", PlayStation),
            (None, "PS5 Controller", PlayStation),
            (None, "DualShock 4", PlayStation),
            (None, "Nintendo Switch Pro Controller", Nintendo),
            (None, "Pro Controller", Nintendo),
            (None, "Joy-Con (L/R)", Nintendo),
            (None, "Xbox Wireless Controller", Xbox),
            (None, "Xbox One Wireless Controller", Xbox),
            (Some(0x045e), "Wireless Controller", Xbox),
            (None, "Microsoft X-Box 360 pad", Xbox),
            (None, "8BitDo Ultimate", Xbox),
            (None, "Steam Deck", Xbox),
            (None, "", Xbox),
        ];
        for (v, n, want) in cases {
            assert_eq!(ButtonStyle::detect(*v, n), *want, "{v:?} {n}");
        }
    }

    #[test]
    fn last_used_pad_wins_and_virtual_pads_stand_for_the_real_one() {
        let ds4 = pad(0, "PS4 Controller", Some(0x054c), Some(0x09cc));
        let steam = pad(1, "Steam Virtual Gamepad", Some(0x28de), Some(0x11ff));
        let pro = pad(2, "Pro Controller", Some(0x057e), Some(0x2009));
        assert_eq!(pad_style(&[], None), None);
        assert_eq!(pad_style(&[], Some(3)), None);
        // Nothing pressed yet: the first real pad.
        assert_eq!(pad_style(&[steam.clone(), ds4.clone()], None), Some(ButtonStyle::PlayStation));
        // Presses arriving through Steam's virtual pad: still the DS4 in the hand.
        assert_eq!(pad_style(&[ds4.clone(), steam.clone()], Some(1)), Some(ButtonStyle::PlayStation));
        // Only the virtual pad: Xbox.
        assert_eq!(pad_style(std::slice::from_ref(&steam), Some(1)), Some(ButtonStyle::Xbox));
        // Last pressed wins over the first connected.
        assert_eq!(pad_style(&[ds4.clone(), pro.clone()], Some(2)), Some(ButtonStyle::Nintendo));
        // The used pad went away: back to the first connected.
        assert_eq!(pad_style(&[ds4], Some(2)), Some(ButtonStyle::PlayStation));
    }

    #[test]
    fn fake_pad_hook() {
        assert_eq!(fake_pad_style(""), None);
        assert_eq!(fake_pad_style("054c:x"), Some(ButtonStyle::PlayStation));
        assert_eq!(fake_pad_style("0x057e:x"), Some(ButtonStyle::Nintendo));
        assert_eq!(fake_pad_style(":Pro Controller"), Some(ButtonStyle::Nintendo));
        assert_eq!(fake_pad_style("DualSense"), Some(ButtonStyle::PlayStation));
        assert_eq!(fake_pad_style("045e:Xbox"), Some(ButtonStyle::Xbox));
    }

    #[test]
    fn labels_per_style_and_position() {
        use Button::*;
        let names = |s: ButtonStyle| [A, B, X, Y, Start, Select, LB, RB].map(|b| s.face(b).name);
        assert_eq!(names(ButtonStyle::Xbox), ["A", "B", "X", "Y", "Menu", "View", "LB", "RB"]);
        assert_eq!(
            names(ButtonStyle::PlayStation),
            ["Cross", "Circle", "Square", "Triangle", "Options", "Create", "L1", "R1"]
        );
        // Nintendo: south B, east A, west Y, north X.
        assert_eq!(names(ButtonStyle::Nintendo), ["B", "A", "Y", "X", "+", "−", "L", "R"]);
        let ps = ButtonStyle::PlayStation;
        assert_eq!(ps.face(A).mark, Mark::Cross);
        assert_eq!(ps.face(B).mark, Mark::Circle);
        assert_eq!(ps.face(X).mark, Mark::Square);
        assert_eq!(ps.face(Y).mark, Mark::Triangle);
        assert_eq!(ButtonStyle::Nintendo.face(Start).mark, Mark::Plus);
        assert_eq!(ButtonStyle::Nintendo.face(A).mark, Mark::Text("B"));
        // Coloured letters / shapes on Xbox and PlayStation face buttons; Nintendo is plain.
        for b in [A, B, X, Y] {
            assert!(ButtonStyle::Xbox.face(b).color.is_some());
            assert!(ps.face(b).color.is_some());
            assert!(ButtonStyle::Nintendo.face(b).color.is_none());
        }
        assert_eq!(ButtonStyle::Xbox.face(A).color, Some(XBOX_GREEN));
        assert_eq!(ButtonStyle::Xbox.face(Y).color, Some(XBOX_YELLOW));
        assert!(!ButtonStyle::Xbox.face(LB).round);
        assert!(ButtonStyle::Xbox.face(Start).round);
    }

    #[test]
    fn swap_follows_the_button_that_confirms() {
        let n = Prompts::new(ButtonStyle::Nintendo, true);
        // Confirm sits on the east button: Nintendo calls it A.
        assert_eq!(n.name(Button::A), "A");
        assert_eq!(n.name(Button::B), "B");
        assert_eq!(n.name(Button::X), "Y");
        let x = Prompts::new(ButtonStyle::Xbox, false);
        assert_eq!(x.name(Button::A), "A");
        let xs = Prompts::new(ButtonStyle::Xbox, true);
        assert_eq!(xs.name(Button::A), "B");
        assert_eq!(Prompts::new(ButtonStyle::PlayStation, true).name(Button::B), "Cross");
    }
}
