//! Whether each installed emulator that supports RetroAchievements is signed in, read from its own
//! config (only the username and whether a token is present; the token itself is never kept or
//! logged). Lodestar never writes these settings: the emulators sign in themselves.
//!
//! Where each one keeps it (folders per install from `emulation::roots::config_dir`):
//! * RetroArch `retroarch.cfg`: `cheevos_enable`, `cheevos_username`, `cheevos_token`; newer
//!   builds move the last two into `retroarch-keychain.cfg` beside it (its values win).
//! * DuckStation `settings.ini` `[Cheevos]`: `Enabled`, `Username`, `Token`.
//! * PCSX2 `inis/PCSX2.ini` `[Achievements]`: `Enabled`, `Username`, `Token` (2.6+: the token in
//!   `inis/secrets.ini`).
//! * PPSSPP `PSP/SYSTEM/ppsspp.ini` `[Achievements]`: `AchievementsEnable`,
//!   `AchievementsUserName`; the token is the file `ppsspp_retroachievements.dat` (only its size
//!   is looked at).
//! * Dolphin `RetroAchievements.ini` `[Achievements]`: `Enabled`, `Username`, `ApiToken`.
//! * Flycast `emu.cfg` `[achievements]`: `Enabled`, `UserName`, `Token`.

use crate::emulation::detect::Emulator;
use std::path::Path;

/// One emulator install's RetroAchievements sign-in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signin {
    /// Emulator instance id (`emulation::registry`), for "Open <emulator>".
    pub instance: String,
    /// "DuckStation (Flatpak)".
    pub label: String,
    pub status: SigninStatus,
    /// Where to sign in inside that emulator, e.g. "Settings → Achievements".
    pub where_to_sign_in: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SigninStatus {
    SignedIn {
        /// The name the emulator signed in with ("" when it keeps the name sealed: RetroArch's
        /// keychain).
        username: String,
    },
    NotSignedIn,
    /// Achievements switched off in that emulator.
    Disabled,
    /// Its settings couldn't be found or read.
    Unknown,
}

/// Every installed emulator with RetroAchievements support (from the published registry).
/// Cheap file reads; fine on a helper thread, not every frame on the UI thread.
pub fn signins() -> Vec<Signin> {
    match crate::emulation::registry::current() {
        Some(reg) => signins_in(&reg.emulators, &reg.home),
        None => Vec::new(),
    }
}

/// [`signins`] for these emulators and home folder.
pub fn signins_in(emus: &[Emulator], home: &Path) -> Vec<Signin> {
    emus.iter()
        .filter_map(|e| {
            let where_to_sign_in = where_to_sign_in(&e.def)?;
            Some(Signin { instance: e.id.clone(), label: e.label_in_text(), status: status(e, home), where_to_sign_in })
        })
        .collect()
}

/// Where the sign-in is inside each emulator (None: no RetroAchievements support Lodestar reads).
pub fn where_to_sign_in(def: &str) -> Option<&'static str> {
    Some(match def {
        "retroarch" => "Settings → Achievements",
        "duckstation" => "Settings → Achievements",
        "pcsx2" => "Settings → Achievements",
        "ppsspp" => "Settings → Tools → RetroAchievements",
        "dolphin" => "Tools → Achievements → Settings",
        "flycast" => "Settings → General → RetroAchievements",
        _ => return None,
    })
}

/// What an emulator's settings say about one key: a value (usernames, switches) or, for a
/// token, only whether it is there.
#[derive(Default)]
struct Found {
    enabled: Option<bool>,
    username: Option<String>,
    /// Some(non-empty) when the key is there.
    token: Option<bool>,
}

impl Found {
    /// `other`'s keys win (a file read after this one).
    fn overlay(&mut self, other: Found) {
        self.enabled = other.enabled.or(self.enabled);
        self.username = other.username.or(self.username.take());
        self.token = other.token.or(self.token);
    }
}

/// One ini section's `enabled` / `username` / `token` keys (`token`: only non-empty-ness is kept).
fn read_ini(text: &str, section: &str, enabled: &str, username: &str, token: &str, out: &mut Found) {
    let mut in_sec = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_sec = l.trim_start_matches('[').trim_end_matches(']').trim() == section;
            continue;
        }
        if !in_sec || l.starts_with(';') || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let k = k.trim();
        let v = v.trim().trim_matches('"');
        if k == enabled {
            out.enabled = Some(truthy(v));
        } else if k == username {
            out.username = Some(v.to_string());
        } else if k == token {
            out.token = Some(!v.is_empty());
        }
    }
}

/// RetroArch's `key = "value"` lines (no sections).
fn read_ra_cfg(text: &str, out: &mut Found) {
    read_ini(&format!("[x]\n{text}"), "x", "cheevos_enable", "cheevos_username", "cheevos_token", out);
}

fn truthy(v: &str) -> bool {
    matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
}

/// A username as RetroAchievements allows them (a sealed keychain value isn't one).
fn plain_username(s: &str) -> bool {
    (2..=32).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

/// The sign-in state of one install.
pub fn status(emu: &Emulator, home: &Path) -> SigninStatus {
    let Some(dir) = crate::emulation::roots::config_dir(emu, home).map(|u| u.path) else {
        return SigninStatus::Unknown;
    };
    let mut f = Found::default();
    let mut sealed = false;
    match emu.def.as_str() {
        "retroarch" => {
            let cfg = crate::emulation::detect::retroarch_cfg(&emu.kind, home);
            let Some(t) = read(&cfg) else { return SigninStatus::Unknown };
            read_ra_cfg(&t, &mut f);
            // Newer RetroArch: the sign-in in `retroarch-keychain.cfg` (maybe sealed), over the
            // main file.
            if let Some(k) = cfg.parent().and_then(|d| read(&d.join("retroarch-keychain.cfg"))) {
                let mut kf = Found::default();
                read_ra_cfg(&k, &mut kf);
                if let Some(u) = kf.username.as_mut()
                    && !u.is_empty()
                    && !plain_username(u)
                {
                    sealed = true;
                    u.clear();
                }
                f.overlay(kf);
            }
        }
        "duckstation" => {
            let Some(t) = read(&dir.join("settings.ini")) else { return SigninStatus::Unknown };
            read_ini(&t, "Cheevos", "Enabled", "Username", "Token", &mut f);
        }
        "pcsx2" => {
            let Some(t) = read(&dir.join("PCSX2.ini")) else { return SigninStatus::Unknown };
            read_ini(&t, "Achievements", "Enabled", "Username", "Token", &mut f);
            if let Some(s) = read(&dir.join("secrets.ini")) {
                let mut sf = Found::default();
                read_ini(&s, "Achievements", "-", "-", "Token", &mut sf);
                f.overlay(sf);
            }
        }
        "ppsspp" => {
            let Some(t) = read(&dir.join("ppsspp.ini")) else { return SigninStatus::Unknown };
            read_ini(&t, "Achievements", "AchievementsEnable", "AchievementsUserName", "-", &mut f);
            // The token is this file's content: only its size is looked at.
            f.token = Some(std::fs::metadata(dir.join("ppsspp_retroachievements.dat")).is_ok_and(|m| m.len() > 0));
        }
        "dolphin" => {
            let p = dir.join("RetroAchievements.ini");
            // Dolphin writes the file once achievements were touched: none = never switched on.
            let Some(t) = read(&p) else {
                return if dir.is_dir() { SigninStatus::Disabled } else { SigninStatus::Unknown };
            };
            read_ini(&t, "Achievements", "Enabled", "Username", "ApiToken", &mut f);
        }
        "flycast" => {
            let Some(t) = read(&dir.join("emu.cfg")) else { return SigninStatus::Unknown };
            read_ini(&t, "achievements", "Enabled", "UserName", "Token", &mut f);
        }
        _ => return SigninStatus::Unknown,
    }
    finish(f, sealed)
}

fn finish(f: Found, sealed_name: bool) -> SigninStatus {
    // Every one of them defaults to achievements off.
    if !f.enabled.unwrap_or(false) {
        return SigninStatus::Disabled;
    }
    match f.username {
        Some(u) if f.token == Some(true) && (sealed_name || !u.trim().is_empty()) => {
            SigninStatus::SignedIn { username: u.trim().to_string() }
        }
        _ => SigninStatus::NotSignedIn,
    }
}

#[cfg(test)]
mod tests;
