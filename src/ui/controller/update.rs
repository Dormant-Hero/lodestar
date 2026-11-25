//! Quick menu → Update Lodestar: "What's new in <version>" (the release notes) with Update now
//! for the AppImage, the download's progress, Restart now; the release page for other installs.

use super::*;
use crate::ui::whats_new::{self, Block};

/// A reading block (selectable so ↑ / ↓ scroll through the notes).
fn read(heading: impl Into<String>, text: impl Into<String>) -> Item {
    let mut it = Item::info(heading, text);
    it.act = Some(Act::Read);
    it
}

/// One line of Markdown without its `**` / `` ` `` markers.
fn plain(text: &str) -> String {
    whats_new::spans(text).into_iter().map(|(_, t)| t).collect()
}

/// Release notes as reading blocks: one per heading, its bullets as "• " lines.
pub(super) fn notes_items(md: &str) -> Vec<Item> {
    let mut out: Vec<(String, Vec<String>)> = vec![];
    for b in whats_new::parse(md) {
        match b {
            Block::Heading(_, h) => out.push((plain(&h), vec![])),
            Block::Bullet(level, t) => {
                if out.is_empty() {
                    out.push(("What's new".into(), vec![]));
                }
                let pad = "   ".repeat(level);
                out.last_mut().unwrap().1.push(format!("{pad}• {}", plain(&t)));
            }
            Block::Para(t) => {
                if out.is_empty() {
                    out.push(("What's new".into(), vec![]));
                }
                out.last_mut().unwrap().1.push(plain(&t));
            }
        }
    }
    out.into_iter().filter(|(_, lines)| !lines.is_empty()).map(|(h, lines)| read(h, lines.join("\n"))).collect()
}

fn update_items(st: &AppState) -> Vec<Item> {
    use crate::update_app::Phase;
    let u = &st.update;
    let Some(r) = &u.latest else { return vec![] };
    let mut v = vec![];
    match (&u.phase, u.why_not_installable()) {
        (_, Some(why)) => v.push(read("Get it from the release page", format!("{why}\n{}", r.page))),
        (Phase::Downloading { .. }, _) => {
            v.push(Item::value("Downloading", format!("{} %", u.percent().unwrap_or(0)), Act::Read));
            v.push(Item::action("Cancel the download", Act::UpdateCancel));
        }
        (Phase::Ready, _) => {
            v.push(Item::action("Restart now", Act::UpdateRestart));
            v.push(read("Installed", "Or it starts the next time you open Lodestar."));
        }
        (Phase::Failed(e), _) => {
            v.push(Item::action("Try again", Act::UpdateNow));
            v.push(read("The update didn't install", e.clone()));
        }
        (Phase::Idle, _) => v.push(
            Item::action("Update now", Act::UpdateNow)
                .with_help("Downloads the new AppImage from GitHub, checks it and puts it in place of this one."),
        ),
    }
    if matches!(u.phase, Phase::Idle | Phase::Failed(_)) {
        if st.update_skipped() {
            v.push(read("Skipped", "You chose to skip this version."));
        } else {
            v.push(
                Item::action("Skip this version", Act::UpdateSkip)
                    .with_help("Lodestar stops telling you about it; a newer version is announced again."),
            );
        }
    }
    let notes = notes_items(&r.notes);
    if notes.is_empty() {
        v.push(read("What's new", "This release came without notes."));
    }
    v.extend(notes);
    v
}

impl ControllerUi {
    /// The quick menu's "Update Lodestar" when there is a newer version, else "Check for updates".
    pub(super) fn update_item(st: &AppState) -> Item {
        let u = &st.update;
        let Some(r) = u.latest.as_ref() else {
            return Item::value("Check for updates", if u.checking { "Checking…" } else { "" }, Act::CheckUpdates);
        };
        let value = if u.ready() {
            "Restart now".to_string()
        } else if let Some(p) = u.percent() {
            format!("Downloading {p} %")
        } else {
            r.version.clone()
        };
        Item::value("Update Lodestar", value, Act::Update)
    }

    pub(super) fn open_update(&mut self, st: &mut AppState, now: f64) {
        if st.update.ready() {
            st.restart_after_update();
            return;
        }
        let Some(r) = &st.update.latest else { return };
        let m = Menu::new(MenuKind::Update, format!("What's new in {}", r.version), update_items(st), now)
            .with_subtitle(format!("You have Lodestar {}", env!("CARGO_PKG_VERSION")));
        self.menus.push(m);
    }

    /// The update's state changed: the quick menu and the update page follow it ("What's new"
    /// opens when a check from the menu found a newer version).
    pub(super) fn refresh_update(&mut self, st: &mut AppState, now: f64) {
        if std::mem::take(&mut st.update.show_notes) {
            self.open_update(st, now);
        }
        if self.update_gen == st.update.generation {
            return;
        }
        self.update_gen = st.update.generation;
        self.refresh_quick(st);
        let items = update_items(st);
        if let Some(m) = self.menus.iter_mut().rev().find(|m| m.kind == MenuKind::Update) {
            m.replace_items(items);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_notes_become_one_reading_block_per_heading() {
        let items =
            notes_items("### Library\n- **Faster** start-up\n- Fixed `covers`\n\n### Controller mode\n- Smoother");
        let got: Vec<(String, String)> = items.iter().map(|i| (i.label.clone(), i.value.clone().unwrap())).collect();
        assert_eq!(
            got,
            [
                ("Library".to_string(), "• Faster start-up\n• Fixed covers".to_string()),
                ("Controller mode".to_string(), "• Smoother".to_string())
            ]
        );
        assert!(items.iter().all(|i| i.act == Some(Act::Read)));
        assert_eq!(notes_items("Just a line.")[0].label, "What's new");
        assert!(notes_items("").is_empty());
    }
}
