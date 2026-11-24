//! Theme options of imported Playnite themes (thememodifier.yaml), shared by desktop Settings →
//! Appearance and controller Settings → Visuals: the options of a theme, the values the user
//! picked (shown at once, before the theme is rebuilt), and the rebuild itself on a worker
//! thread (`theme::playnite::apply_options`, a few tens of milliseconds). Changes made while a
//! rebuild runs are coalesced: the newest values run next. `poll` (every frame, app.rs) reloads
//! the theme where it is in use once its rebuild is done.

use crate::app::AppState;
use crate::theme::playnite::options::{self, OptionsSpec};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

type Values = BTreeMap<String, String>;

#[derive(Default)]
struct Jobs {
    /// Theme key → the values the user picked last (what the UI shows).
    wanted: BTreeMap<String, Values>,
    /// A rebuild is running for this theme key.
    running: Option<String>,
    /// Finished rebuilds: (key, error).
    done: Vec<(String, Option<String>)>,
    /// Options read from disk: theme dir → (file time, spec).
    specs: BTreeMap<PathBuf, (Option<SystemTime>, Option<Arc<OptionsSpec>>)>,
}

static JOBS: Mutex<Option<Jobs>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Jobs) -> R) -> R {
    let mut g = JOBS.lock();
    f(g.get_or_insert_with(Jobs::default))
}

/// The options of the theme in `dir` (cached per file time; None: none to show).
pub fn spec(dir: &Path) -> Option<Arc<OptionsSpec>> {
    let stamp = std::fs::metadata(dir.join(options::FILE)).and_then(|m| m.modified()).ok();
    if let Some(hit) = with(|j| j.specs.get(dir).filter(|(s, _)| *s == stamp).map(|(_, v)| v.clone())) {
        return hit;
    }
    let v = stamp.and_then(|_| options::read(dir)).map(Arc::new);
    with(|j| {
        if j.specs.len() > 32 {
            j.specs.clear();
        }
        j.specs.insert(dir.to_path_buf(), (stamp, v.clone()));
    });
    v
}

/// A theme's options by key (user themes only).
pub fn spec_for(st: &AppState, key: &str) -> Option<Arc<OptionsSpec>> {
    let dir = st.themes.list().iter().find(|i| i.key == key)?.dir.clone()?;
    spec(&dir)
}

/// The values in effect for a theme: what the user just picked, else what the theme was built
/// with.
pub fn values(key: &str, spec: &OptionsSpec) -> Values {
    with(|j| j.wanted.get(key).cloned()).unwrap_or_else(|| spec.values.clone())
}

/// A rebuild is running or queued for `key`.
pub fn busy(key: &str) -> bool {
    with(|j| j.running.as_deref() == Some(key) || j.wanted.contains_key(key))
}

/// Set option values for a theme (the rebuild starts now, or after the one running).
pub fn set(st: &AppState, key: &str, values: Values) {
    let start = with(|j| {
        j.wanted.insert(key.to_string(), values);
        j.running.is_none()
    });
    if start {
        run_next(st.themes.themes_dir().to_path_buf(), st.ctx().clone());
    }
}

/// One option to one value.
pub fn set_one(st: &AppState, key: &str, spec: &OptionsSpec, option: &str, value: String) {
    let mut v = values(key, spec);
    v.insert(option.to_string(), value);
    set(st, key, v);
}

/// Back to the theme's own values.
pub fn reset(st: &AppState, key: &str) {
    set(st, key, Values::new());
}

fn run_next(themes_dir: PathBuf, ctx: egui::Context) {
    let next = with(|j| {
        if j.running.is_some() {
            return None;
        }
        let (k, v) = j.wanted.iter().next().map(|(k, v)| (k.clone(), v.clone()))?;
        j.running = Some(k.clone());
        Some((k, v))
    });
    let Some((key, vals)) = next else { return };
    let spawned = std::thread::Builder::new().name("theme-options".into()).spawn(move || {
        let r = crate::theme::playnite::apply_options(&themes_dir, &key, &vals);
        let ok = r.is_ok();
        with(|j| {
            j.running = None;
            // Values picked meanwhile run next; otherwise this theme is done.
            if j.wanted.get(&key) == Some(&vals) {
                j.wanted.remove(&key);
            }
            j.done.push((key.clone(), r.err().map(|e| format!("{e:#}"))));
        });
        ctx.request_repaint();
        // The theme shows the new values now; then see whether other options came into play.
        if ok && let Ok(true) = crate::theme::playnite::refresh_option_effects(&themes_dir, &key) {
            ctx.request_repaint();
        }
        run_next(themes_dir, ctx);
    });
    if let Err(e) = spawned {
        log::warn!("theme options: {e}");
        with(|j| j.running = None);
    }
}

/// Every frame: reload themes whose rebuild finished (where they are in use).
pub fn poll(st: &mut AppState) {
    let done = with(|j| std::mem::take(&mut j.done));
    for (key, err) in done {
        if let Some(e) = err {
            st.notify_error(format!("Couldn't apply the theme options: {e}"));
            continue;
        }
        st.themes.refresh();
        if st.config.desktop_theme == key {
            st.set_desktop_theme(&key);
        }
        if st.config.controller.theme == key {
            st.set_controller_theme(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_fall_back_to_the_built_ones() {
        let s = OptionsSpec { values: [("A".to_string(), "1".to_string())].into(), ..Default::default() };
        assert_eq!(values("no-such-theme-xyz", &s).get("A").map(String::as_str), Some("1"));
        assert!(!busy("no-such-theme-xyz"));
        assert!(spec(Path::new("/nonexistent/theme")).is_none());
    }
}
