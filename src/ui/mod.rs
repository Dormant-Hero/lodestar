//! UIs. `desktop` = Playnite-style mouse/keyboard UI; `controller` = fullscreen themed UI.
//! Both get `&mut AppState` and act through its methods (see app.rs CONTRACT).

pub(crate) mod achievements;
pub mod art_viewer;
pub mod controller;
pub mod credits;
pub mod desktop;
pub(crate) mod launching;
pub mod platform_icons;
pub mod theme_options;
pub mod whats_new;
pub mod widgets;

/// The game's shown name, or "" when it is gone from the library.
pub(crate) fn game_name(st: &crate::app::AppState, id: crate::model::GameId) -> String {
    st.library.get(id).map(|g| g.display_name().to_string()).unwrap_or_default()
}
