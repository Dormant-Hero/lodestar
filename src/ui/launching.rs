//! Launch-flow state both UIs show, read from the backend in one place:
//! * the status of a launch still on its way ("Starting Diablo IV via Battle.net… sign in there
//!   if asked"), shown in the desktop status bar / details panel and controller mode's overlay;
//! * the "another game is running" prompt (`one_game_at_a_time = Ask`);
//! * the "quit while a game runs?" prompt (`when_closing = Ask`).
//!
//! Screenshot runs can fake these for the selected game (`LODESTAR_DESKTOP_OPEN=starting`
//! / `switch` / `close`, controller mode: `LODESTAR_CONTROLLER_OPEN=…` likewise); the fake never
//! starts or stops anything and only exists in runs that can't launch (`AppState::no_launch`).

use crate::app::AppState;
use crate::model::GameId;
use std::sync::Mutex;

/// The "another game is running" prompt.
pub(crate) struct SwitchAsk {
    /// The game that runs now.
    pub running: String,
    /// The game the user wants to start.
    pub next: String,
}

/// The pending "another game is running" prompt, if any (`AppState::switch_prompt`).
pub(crate) fn switch_ask(st: &AppState) -> Option<SwitchAsk> {
    if let Some(f) = fake(st)
        && f.kind == FakeKind::Switch
    {
        let running = st
            .library
            .games
            .iter()
            .find(|g| g.id != f.id && g.installed)
            .map(|g| g.display_name().to_string())
            .unwrap_or_else(|| "Another game".into());
        return Some(SwitchAsk { running, next: name(st, f.id) });
    }
    st.switch_prompt.map(|(running, next)| SwitchAsk { running: name(st, running), next: name(st, next) })
}

/// A yes / (other) / cancel question both UIs show over everything: the "another game is
/// running" prompt or the "quit while a game runs?" one.
pub(crate) struct Prompt {
    pub kind: PromptKind,
    pub title: String,
    pub body: String,
    pub hint: String,
    /// The primary answer (A / Enter).
    pub primary: &'static str,
    /// A second answer (X), if any.
    pub alt: Option<&'static str>,
    /// Cancel (B / Esc).
    pub cancel: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PromptKind {
    Switch,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PromptAnswer {
    Primary,
    Alt,
    Cancel,
}

/// The prompt to show, if any (the close one first: it answers the latest request).
pub(crate) fn prompt(st: &AppState) -> Option<Prompt> {
    let fake_close = fake(st).filter(|f| f.kind == FakeKind::Close).map(|f| name(st, f.id));
    if let Some(game) = fake_close.or_else(|| st.close_prompt_game()) {
        let tray = st.tray_shown();
        return Some(Prompt {
            kind: PromptKind::Close,
            title: "Quit Lodestar?".into(),
            body: format!(
                "{game} is still running. Keep Lodestar running {} to record the session, \
                 or quit now: the game keeps running and Lodestar picks it up again when it starts.",
                if tray { "in the system tray" } else { "minimised" }
            ),
            hint: "Choose what closing does in Settings → General.".into(),
            primary: if tray { "Hide to the tray" } else { "Minimise" },
            alt: Some("Quit"),
            cancel: "Cancel",
        });
    }
    switch_ask(st).map(|ask| Prompt {
        kind: PromptKind::Switch,
        title: format!("Close {}?", ask.running),
        body: format!("{} is still running. Close it and start {}?", ask.running, ask.next),
        hint: "Save your game first: Lodestar asks it to quit and closes it if it doesn't. \
               Choose what happens here in Settings → General → Launchers."
            .into(),
        primary: "Close & start",
        alt: None,
        cancel: "Cancel",
    })
}

/// Answer the prompt `kind` (see [`prompt`]).
pub(crate) fn answer_prompt(st: &mut AppState, kind: PromptKind, answer: PromptAnswer) {
    match kind {
        PromptKind::Switch => match answer {
            PromptAnswer::Primary => answer_switch(st, true),
            PromptAnswer::Cancel => answer_switch(st, false),
            PromptAnswer::Alt => {}
        },
        PromptKind::Close if fake(st).is_some_and(|f| f.kind == FakeKind::Close) => {
            log::info!("screenshot run: close prompt answered ({answer:?})");
            set_fake(None);
        }
        PromptKind::Close => st.answer_close_prompt(match answer {
            PromptAnswer::Primary => crate::running_app::CloseAnswer::KeepRunning,
            PromptAnswer::Alt => crate::running_app::CloseAnswer::Quit,
            PromptAnswer::Cancel => crate::running_app::CloseAnswer::Cancel,
        }),
    }
}

/// Answer the prompt: close the running game and start the new one, or cancel.
pub(crate) fn answer_switch(st: &mut AppState, close_and_start: bool) {
    if fake(st).is_some_and(|f| f.kind == FakeKind::Switch) {
        log::info!("screenshot run: switch prompt answered (close & start: {close_and_start})");
        set_fake(None);
        return;
    }
    if close_and_start {
        st.confirm_switch();
    } else {
        st.cancel_switch();
    }
}

/// The game the UIs show as launching / running: one waiting for another game to close first,
/// else the most recent one launched.
pub(crate) fn running(st: &AppState) -> Option<GameId> {
    st.switch_pending()
        .or_else(|| st.running_game())
        .or_else(|| fake(st).filter(|f| f.kind == FakeKind::Starting).map(|f| f.id))
}

/// `id` is being launched, runs, or waits for another game to close (several can, with
/// "Allow several").
pub(crate) fn is_active(st: &AppState, id: GameId) -> bool {
    st.launcher.active_games().contains(&id)
        || st.switch_pending() == Some(id)
        || fake(st).is_some_and(|f| f.kind == FakeKind::Starting && f.id == id)
}

/// `id` is on its way but not seen running yet (a store client may sit on a login, update or
/// "waiting for the client" step for minutes; or another game is closing first).
pub(crate) fn starting(st: &AppState, id: GameId) -> bool {
    status(st, id).is_some()
}

/// What a launch on its way waits for, one line ("Starting Diablo IV via Battle.net… sign in
/// there if asked", "Closing A to start B…"). None when `id` isn't starting.
pub(crate) fn status(st: &AppState, id: GameId) -> Option<String> {
    if fake(st).is_some_and(|f| f.kind == FakeKind::Starting && f.id == id) {
        // The real line's wording, with the game's own store as the client.
        let via = st.library.get(id).map(|g| g.source.display()).unwrap_or("its launcher");
        return Some(format!("Starting {} via {via}… sign in there if asked", name(st, id)));
    }
    st.launch_status(id)
}

/// Stop / cancel `id`: a launch waiting for another game to close is dropped, a launching or
/// running game is asked to quit (asking again later forces it).
pub(crate) fn stop(st: &mut AppState, id: GameId) {
    if fake(st).is_some_and(|f| f.id == id) {
        set_fake(None);
        return;
    }
    if st.switch_pending() == Some(id) {
        st.cancel_switch();
    } else if st.no_launch {
        // Screenshot runs never signal anything (a game they picked up may be a real one).
        log::warn!("screenshot run: not stopping game {id}");
    } else {
        st.launcher.stop_game(id);
    }
}

fn name(st: &AppState, id: GameId) -> String {
    st.library.get(id).map(|g| g.display_name().to_string()).unwrap_or_else(|| "the game".into())
}

// ----- screenshot fakes

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FakeKind {
    Starting,
    Switch,
    /// The "quit while a game runs?" prompt, `id` being the game.
    Close,
}

#[derive(Clone, Copy, Debug)]
struct Fake {
    id: GameId,
    kind: FakeKind,
}

static FAKE: Mutex<Option<Fake>> = Mutex::new(None);

/// Tests that use the fake hold this: it is one for the whole process (tests run in parallel).
#[cfg(test)]
pub(crate) static FAKE_TEST_LOCK: Mutex<()> = Mutex::new(());

fn fake(st: &AppState) -> Option<Fake> {
    if !st.no_launch {
        return None;
    }
    FAKE.lock().ok().and_then(|f| *f)
}

fn set_fake(f: Option<Fake>) {
    if let Ok(mut g) = FAKE.lock() {
        *g = f;
    }
}

/// Screenshot runs only: show `kind` for game `id` (`starting` / `switch` / `close`). Returns false for
/// another `what` or a run that could launch.
pub(crate) fn debug_fake(st: &AppState, id: GameId, what: &str) -> bool {
    let kind = match what {
        "starting" => FakeKind::Starting,
        "switch" => FakeKind::Switch,
        "close" => FakeKind::Close,
        _ => return false,
    };
    if !st.no_launch {
        return false;
    }
    set_fake(Some(Fake { id, kind }));
    true
}
