//! Game options for several games at once: only the options the
//! user changed in the dialog are written to each game; every other option keeps that game's own
//! value. A [`GameOptionsPatch`] is that set of changes, field by field (map-valued options per
//! entry: one Vulkan layer, one Gamescope flag).

use crate::game_options::{FlagSetting, GameOptions, OPTIONS, OptionId, Tri};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameOptionsPatch {
    /// Top-level `GameOptions` fields (serde names) and their new value.
    fields: BTreeMap<String, Value>,
    /// `layers` entries (Default = the game's own entry is removed).
    layers: BTreeMap<String, Tri>,
    /// `gs_more` entries (None = removed).
    gs_more: BTreeMap<String, Option<FlagSetting>>,
}

const MAPS: [&str; 2] = ["layers", "gs_more"];

fn object(o: &GameOptions) -> Map<String, Value> {
    match serde_json::to_value(o) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

impl GameOptionsPatch {
    /// What changed from `before` to `after`.
    pub fn between(before: &GameOptions, after: &GameOptions) -> GameOptionsPatch {
        let mut p = GameOptionsPatch::default();
        let (b, a) = (object(before), object(after));
        for (k, v) in &a {
            if !MAPS.contains(&k.as_str()) && b.get(k) != Some(v) {
                p.fields.insert(k.clone(), v.clone());
            }
        }
        for name in before.layers.keys().chain(after.layers.keys()) {
            let (x, y) = (
                before.layers.get(name).copied().unwrap_or_default(),
                after.layers.get(name).copied().unwrap_or_default(),
            );
            if x != y {
                p.layers.insert(name.clone(), y);
            }
        }
        for flag in before.gs_more.keys().chain(after.gs_more.keys()) {
            let y = after.gs_more.get(flag);
            if before.gs_more.get(flag) != y {
                p.gs_more.insert(flag.clone(), y.cloned());
            }
        }
        p
    }

    /// Add later changes (they win over earlier ones of the same option). A change set back to
    /// where it started stays in: the user chose that value for every game.
    pub fn merge(&mut self, later: GameOptionsPatch) {
        self.fields.extend(later.fields);
        self.layers.extend(later.layers);
        self.gs_more.extend(later.gs_more);
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.layers.is_empty() && self.gs_more.is_empty()
    }

    /// `o` with the patched options replaced.
    pub fn apply(&self, o: &GameOptions) -> GameOptions {
        let mut out = if self.fields.is_empty() {
            o.clone()
        } else {
            let mut m = object(o);
            for (k, v) in &self.fields {
                m.insert(k.clone(), v.clone());
            }
            serde_json::from_value(Value::Object(m)).unwrap_or_else(|_| o.clone())
        };
        for (name, v) in &self.layers {
            if *v == Tri::Default {
                out.layers.remove(name);
            } else {
                out.layers.insert(name.clone(), *v);
            }
        }
        for (flag, v) in &self.gs_more {
            match v {
                Some(v) => out.gs_more.insert(flag.clone(), v.clone()),
                None => out.gs_more.remove(flag),
            };
        }
        out
    }

    /// "MangoHud On", "Window mode Borderless" … in the editor's order.
    pub fn summary(&self) -> Vec<String> {
        let mut out = vec![];
        let mut done = std::collections::HashSet::new();
        for m in OPTIONS {
            let key = field_key(m.id);
            if let Some(v) = self.fields.get(&key) {
                out.push(format!("{} {}", m.label, value_text(v)));
                done.insert(key);
            }
        }
        // Fields without a row of their own (none today; kept for future options).
        for (k, v) in &self.fields {
            if !done.contains(k) {
                out.push(format!("{} {}", k.replace('_', " "), value_text(v)));
            }
        }
        for (flag, v) in &self.gs_more {
            out.push(match v {
                Some(FlagSetting::On) => format!("Gamescope {flag} On"),
                Some(FlagSetting::Off) => format!("Gamescope {flag} Off"),
                Some(FlagSetting::Value(s)) => format!("Gamescope {flag} {s}"),
                None => format!("Gamescope {flag} Default"),
            });
        }
        for (name, v) in &self.layers {
            out.push(format!("{} {}", name.trim_start_matches("VK_LAYER_"), tri_text(*v)));
        }
        out
    }
}

/// The serde field name of an option row (`OptionId::FrameLimit` → `frame_limit`).
pub fn field_key(id: OptionId) -> String {
    let dbg = format!("{id:?}");
    let mut s = String::with_capacity(dbg.len() + 4);
    for (i, c) in dbg.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                s.push('_');
            }
            s.push(c.to_ascii_lowercase());
        } else {
            s.push(c);
        }
    }
    s
}

fn tri_text(t: Tri) -> &'static str {
    match t {
        Tri::On => "On",
        Tri::Off => "Off",
        Tri::Default => "Default",
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Null => "Default".into(),
        Value::Bool(b) => if *b { "On" } else { "Off" }.into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => match s.as_str() {
            "default" => "Default".into(),
            "on" => "On".into(),
            "off" => "Off".into(),
            // `lsfg_profile = ""` is "Lodestar settings".
            "" => "Lodestar settings".into(),
            s => {
                let s = s.replace('_', " ");
                let mut c = s.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
            }
        },
        Value::Object(m) => match (m.get("w"), m.get("h")) {
            (Some(w), Some(h)) => format!("{w}×{h}"),
            _ => "set".into(),
        },
        Value::Array(_) => "set".into(),
    }
}

/// What the bulk dialog starts from: the options every game shares; one they disagree on shows
/// as Default (changing it sets it for all of them).
pub fn common<'a>(mut games: impl Iterator<Item = &'a GameOptions>) -> GameOptions {
    let Some(first) = games.next() else { return GameOptions::default() };
    let def = object(&GameOptions::default());
    let mut m = object(first);
    let (mut layers, mut gs_more) = (first.layers.clone(), first.gs_more.clone());
    for g in games {
        let o = object(g);
        for (k, v) in m.iter_mut() {
            if !MAPS.contains(&k.as_str()) && o.get(k) != Some(v) {
                *v = def.get(k).cloned().unwrap_or(Value::Null);
            }
        }
        layers.retain(|k, v| g.layers.get(k) == Some(v));
        gs_more.retain(|k, v| g.gs_more.get(k) == Some(v));
    }
    let mut out: GameOptions = serde_json::from_value(Value::Object(m)).unwrap_or_default();
    out.layers = layers;
    out.gs_more = gs_more;
    out
}

/// "MangoHud On, Gamescope On for 12 games" (long lists end in "and N more").
pub fn summary_line(p: &GameOptionsPatch, games: usize) -> String {
    let parts = p.summary();
    let what = if parts.len() > 4 {
        format!("{} and {} more", parts[..3].join(", "), parts.len() - 3)
    } else {
        parts.join(", ")
    };
    format!("{what} for {games} game{}", if games == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_options::WindowMode;

    #[test]
    fn every_option_row_has_a_field() {
        let keys = object(&GameOptions::default());
        for m in OPTIONS {
            if matches!(m.id, OptionId::GsMore | OptionId::Layer) {
                continue;
            }
            assert!(keys.contains_key(&field_key(m.id)), "{:?} → {}", m.id, field_key(m.id));
        }
    }

    #[test]
    fn patch_writes_only_what_changed() {
        let start = GameOptions::default();
        let mut edited = start.clone();
        edited.mangohud = Tri::On;
        edited.gamescope = Tri::On;
        edited.gs_window = Some(WindowMode::Borderless);
        edited.layers.insert("VK_LAYER_basalt".into(), Tri::On);
        let p = GameOptionsPatch::between(&start, &edited);
        // A game with its own values elsewhere keeps them.
        let mut own = GameOptions { hdr: Tri::On, frame_limit: Some(60), mangohud: Tri::Off, ..Default::default() };
        own.layers.insert("VK_LAYER_other".into(), Tri::Off);
        let got = p.apply(&own);
        assert_eq!((got.mangohud, got.gamescope, got.gs_window), (Tri::On, Tri::On, Some(WindowMode::Borderless)));
        assert_eq!((got.hdr, got.frame_limit), (Tri::On, Some(60)));
        assert_eq!(got.layers.get("VK_LAYER_other"), Some(&Tri::Off));
        assert_eq!(got.layers.get("VK_LAYER_basalt"), Some(&Tri::On));
        assert_eq!(p.summary(), ["Gamescope On", "Window mode Borderless", "MangoHud On", "basalt On"]);
        assert_eq!(summary_line(&p, 12), "Gamescope On, Window mode Borderless, MangoHud On, basalt On for 12 games");
    }

    #[test]
    fn common_values_start_the_dialog() {
        let mut a = GameOptions { mangohud: Tri::On, gamescope: Tri::On, frame_limit: Some(60), ..Default::default() };
        a.layers.insert("L1".into(), Tri::On);
        a.layers.insert("L2".into(), Tri::Off);
        let mut b = GameOptions { mangohud: Tri::On, gamescope: Tri::Off, frame_limit: Some(60), ..Default::default() };
        b.layers.insert("L1".into(), Tri::On);
        let c = common([&a, &b].into_iter());
        assert_eq!((c.mangohud, c.gamescope, c.frame_limit), (Tri::On, Tri::Default, Some(60)));
        assert_eq!(c.layers.len(), 1);
        assert_eq!(common(std::iter::empty()), GameOptions::default());
        assert_eq!(common([&a].into_iter()), a);
    }

    #[test]
    fn touched_then_back_to_default_resets_for_all() {
        // Mixed values show as Default: On then Default again means "Default for every game".
        let start = GameOptions::default();
        let on = GameOptions { mangohud: Tri::On, ..Default::default() };
        let mut p = GameOptionsPatch::between(&start, &on);
        p.merge(GameOptionsPatch::between(&on, &start));
        assert!(!p.is_empty());
        let got = p.apply(&GameOptions { mangohud: Tri::On, wayland: Tri::Off, ..Default::default() });
        assert_eq!((got.mangohud, got.wayland), (Tri::Default, Tri::Off));
        assert_eq!(summary_line(&p, 1), "MangoHud Default for 1 game");
        assert!(GameOptionsPatch::between(&start, &start).is_empty());
    }

    #[test]
    fn gamescope_flags_per_entry_and_long_summaries() {
        let start = GameOptions::default();
        let mut e = start.clone();
        e.gs_more.insert("--rt".into(), FlagSetting::On);
        e.gs_more.insert("--fps-limiter".into(), FlagSetting::Value("60".into()));
        e.ntsync = Tri::On;
        e.gamemode = Tri::Off;
        e.frame_limit = Some(90);
        let p = GameOptionsPatch::between(&start, &e);
        let mut own = GameOptions::default();
        own.gs_more.insert("--keep".into(), FlagSetting::Off);
        own.gs_more.insert("--rt".into(), FlagSetting::Off);
        let got = p.apply(&own);
        assert_eq!(got.gs_more.get("--rt"), Some(&FlagSetting::On));
        assert_eq!(got.gs_more.get("--keep"), Some(&FlagSetting::Off));
        let line = summary_line(&p, 3);
        assert!(line.ends_with("and 2 more for 3 games"), "{line}");
        // Removing a flag in the dialog removes it from each game.
        let p = GameOptionsPatch::between(&e, &start);
        assert!(!p.apply(&e).gs_more.contains_key("--rt"));
    }
}
