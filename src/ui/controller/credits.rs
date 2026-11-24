//! Quick menu → About Lodestar: the Credits page for the controller (thanks, services, fonts,
//! built-in data, the libraries by licence). The same content as desktop About → Credits, read
//! from the embedded THIRD_PARTY_NOTICES.md; the full licence texts are pointed at, not shown
//! (thousands of lines are no reading for a gamepad).
//!
//! The blocks are selectable (`Act::Read`, which does nothing) so ↑ / ↓ scrolls through them.

use super::*;
use crate::ui::credits::{self as data, NOTICES};

impl ControllerUi {
    pub(super) fn open_credits(&mut self, now: f64) {
        let m = Menu::new(MenuKind::Credits, "About Lodestar", credits_items(), now)
            .with_subtitle(format!("Version {}", env!("CARGO_PKG_VERSION")));
        self.menus.push(m);
    }
}

fn read(heading: impl Into<String>, text: impl Into<String>) -> Item {
    let mut it = Item::info(heading, text);
    it.act = Some(Act::Read);
    it
}

pub(super) fn credits_items() -> Vec<Item> {
    let mut v = vec![read(
        "Lodestar",
        "A fast game library and controller-mode frontend for Linux, made with the work of many \
         people. Thank you.",
    )];
    for (who, what, url) in data::THANKS {
        let mut w = what.to_string();
        if let Some(c) = w.get(..1) {
            w = c.to_uppercase() + &w[1..];
        }
        v.push(read(format!("Thanks to {who}"), format!("{w}.\n{url}")));
    }
    let services: Vec<String> = data::table(NOTICES, "Online services").into_iter().map(|r| r[0].clone()).collect();
    v.push(read(
        "Game information and artwork",
        format!(
            "{}. Each can be switched off in the settings; the ones that need a key are asked only \
             once you add one. Pictures and texts belong to these services and their contributors.",
            services.join(", ")
        ),
    ));
    let fonts: Vec<String> = data::table(NOTICES, "Fonts built into Lodestar")
        .into_iter()
        .map(|r| {
            let name = data::plain(&r[0]);
            let name = name.split(" (").next().unwrap_or(&name).to_string();
            format!("{name} ({})", data::plain(&data::short(&r[2])))
        })
        .collect();
    v.push(read("Fonts", fonts.join("\n")));
    for r in data::table(NOTICES, "Data built into Lodestar") {
        let what = data::plain(&r[0]);
        let what = what.split(" (").next().unwrap_or(&what).to_string();
        v.push(read(what, format!("{} · {}", data::plain(&r[1]), data::plain(&data::short(&r[2])))));
    }
    let groups = data::built_in();
    let n: usize = groups.iter().map(|g| g.1.len()).sum();
    let by: Vec<String> = groups.iter().map(|(l, libs)| format!("{l} ({})", libs.len())).collect();
    v.push(read(format!("{n} open-source libraries"), by.join(", ")));
    v.push(read(
        "Licence texts",
        "Every library with its licence, and the full licence texts: desktop mode → menu → About \
         Lodestar → Credits, or the THIRD_PARTY_NOTICES.md file that comes with Lodestar.",
    ));
    v.push(read(
        "Trademarks",
        "Game names, console names, store names and logos are trademarks of their owners. \
         Lodestar is not affiliated with or endorsed by any of them.",
    ));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_page_reads_the_notices() {
        let items = credits_items();
        assert!(items.len() > 10);
        assert!(items.iter().all(|i| i.info && i.act == Some(Act::Read)));
        let text: String =
            items.iter().map(|i| format!("{} {}\n", i.label, i.value.clone().unwrap_or_default())).collect();
        for must in
            ["Thanks to Playnite", "SteamGridDB", "LodestarSymbols", "open-source libraries", "MIT OR Apache-2.0 ("]
        {
            assert!(text.contains(must), "{must}\n{text}");
        }
        assert!(!text.contains("**") && !text.contains('`'), "{text}");
    }
}
