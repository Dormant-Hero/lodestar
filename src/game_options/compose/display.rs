//! Display options of the composer: Gamescope (and the flags it gets), Wayland and HDR.

use super::*;

impl Composer<'_> {
    /// Decide whether gamescope runs (ours or the user's) and its size; report the option.
    pub(super) fn setup_gamescope(&mut self) {
        use OptionId::Gamescope as G;
        let o = self.o;
        let blocking = self.core.blocking();
        let gs_user = blocking && self.chain.gs.is_some();
        let have = self.caps.bins.gamescope.is_some();
        let wanted = o.gamescope.is_on();
        if wanted && matches!(self.core, Core::Launcher | Core::Client) {
            self.wrapper_blocked(G, true);
            self.rep(G, OptionState::Suppressed(GS_LAUNCHER.into()));
        } else if blocking && !have && !gs_user {
            self.unavailable(G, "Gamescope isn't installed.");
        } else if wanted && !self.play {
            self.suppressed(G, ONLY_PLAY);
        } else if wanted && self.caps.nested_gamescope {
            self.suppressed(G, GS_NESTED);
            self.note(GS_NESTED);
        } else if wanted && (have || gs_user) {
            // With the user's own gamescope our flags merge into it, so a gamescope the probe
            // didn't find (not on PATH, caps not probed yet) still gets them.
            self.gs_ours = true;
            self.applied(G);
        } else if o.gamescope.is_off() {
            self.applied(G);
        }
        self.gs_active = self.gs_ours || gs_user;
        let user_flags = self.user_gs_flags();
        self.gs_expose = self.gs_active
            && ((self.gs_ours && o.gs_expose_wayland.is_on()) || user_flags.iter().any(|f| f == "--expose-wayland"));
        if !self.gs_ours {
            return;
        }
        // Size, window, filter (ours only; spec §2.1 gamescope words).
        let out = match o.gs_output {
            Res::Custom { w, h } if w > 0 && h > 0 => Some((w, h)),
            _ => self.input.env.monitor.or(self.caps.preferred_mode),
        };
        self.out_size = out;
        let mut window = match o.gs_window {
            WindowMode::Fullscreen => Some("-f"),
            WindowMode::Borderless => Some("-b"),
            WindowMode::Windowed => None,
        };
        match out {
            Some((ow, oh)) => {
                let internal = match o.gs_internal {
                    InternalRes::Native => (ow, oh),
                    // 0 % (a hand-edited config) means Native; u64 so a huge size can't overflow.
                    InternalRes::Percent(p) if p > 0 => (percent(ow, p), percent(oh, p)),
                    InternalRes::Percent(_) => (ow, oh),
                    InternalRes::Custom { w, h } if w > 0 && h > 0 => (w, h),
                    InternalRes::Custom { .. } => (ow, oh),
                };
                self.gsf.size = Some(((ow, oh), internal));
            }
            None => {
                window = Some("-f");
                self.note(
                    "Couldn't detect the monitor resolution; the game will start at 1280×720. Set Output resolution.",
                );
            }
        }
        self.gsf.window = window;
        let upscaling = self.gsf.size.is_some_and(|((ow, oh), (gw, gh))| gw < ow || gh < oh);
        let filter = match o.gs_filter {
            GsFilter::Auto => upscaling.then_some("fsr"),
            GsFilter::Linear => Some("linear"),
            GsFilter::Nearest => Some("nearest"),
            GsFilter::Fsr => Some("fsr"),
            GsFilter::Nis => Some("nis"),
            GsFilter::Sgsr => Some("sgsr"),
            GsFilter::Pixel => Some("pixel"),
        };
        let filter = match (filter, &self.caps.gamescope) {
            (Some(f), Some(gc)) if !gc.filters.is_empty() && !gc.filters.iter().any(|x| x == f) => {
                self.note(format!("Your Gamescope has no {} upscaler; using FSR instead.", f.to_uppercase()));
                Some("fsr")
            }
            (f, _) => f,
        };
        self.gsf.filter = filter;
        if filter.is_some_and(|f| matches!(f, "fsr" | "nis" | "sgsr")) {
            self.gsf.sharpness = o.gs_sharpness.map(|s| s.min(20));
        }
        if filter.is_some() && self.nvidia() {
            self.note("Gamescope upscaling can crash on NVIDIA (gamescope #1662); try Advanced → Disable Gamescope WSI layer.");
        }
        self.gsf.scaler = match o.gs_scaler {
            GsScaler::Auto => None,
            GsScaler::Fit => Some("fit"),
            GsScaler::Integer => Some("integer"),
            GsScaler::Fill => Some("fill"),
            GsScaler::Stretch => Some("stretch"),
        };
        if o.gs_adaptive_sync.is_on() {
            self.gsf.adaptive = true;
            self.applied(OptionId::GsAdaptiveSync);
            if self.caps.kde_vrr_never == Some(true) {
                self.note("Adaptive sync is off in System Settings → Display; set it to Automatic.");
            }
        }
        self.gsf.expose = o.gs_expose_wayland.is_on();
        self.gsf.grab = o.gs_grab_cursor.is_on();
    }

    /// The user's own gamescope flags (between its word and its `--`).
    pub(super) fn user_gs_flags(&self) -> Vec<String> {
        match self.chain.gs {
            Some(g) if self.core.blocking() => self.chain.words[g.flags()].to_vec(),
            _ => vec![],
        }
    }

    /// Wayland's variables for P / C / L / W (`on` = On, else Off).
    pub(super) fn emit_wayland(&mut self, on: bool) {
        use OptionId::Wayland as I;
        match self.core {
            Core::Wine => {
                if on {
                    self.list(I, "WINEDLLOVERRIDES", ';', "winex11.drv=d");
                }
            }
            _ => self.set(I, "PROTON_ENABLE_WAYLAND", if on { "1" } else { "0" }),
        }
        if on {
            self.wayland_applied = true;
            if self.core == Core::Client {
                self.note("Store clients (Battle.net, EA, Ubisoft) may show a white window on Wine's Wayland driver.");
            }
        }
    }

    /// The runner has a Wayland driver Lodestar can switch on.
    pub(super) fn wayland_possible(&self) -> bool {
        match self.core {
            Core::Wine => self.runner.strict(|r| r.winewayland),
            c if c.protonish() => self.runner.has("PROTON_ENABLE_WAYLAND"),
            _ => false,
        }
    }

    pub(super) fn opt_wayland(&mut self) {
        use OptionId::Wayland as I;
        let o = self.o;
        match self.core {
            Core::Native => {
                let w = o.wayland_per_game;
                if w == Tri::Default {
                    if o.wayland.is_on() {
                        self.suppressed(I, "Native games use Wayland only when it is set for that game.");
                    }
                    return;
                }
                if self.gs_active && !self.gs_expose {
                    self.suppressed(I, GS_WAYLAND);
                    if w.is_on() {
                        self.note(GS_WAYLAND);
                    }
                    return;
                }
                // SDL2 reads SDL_VIDEODRIVER, SDL3 SDL_VIDEO_DRIVER: one setting, so the user's
                // value for either name leaves both alone.
                if self.user_typed("SDL_VIDEODRIVER", None) || self.user_typed("SDL_VIDEO_DRIVER", None) {
                    self.suppressed(I, "Your launch options set the SDL video driver.");
                    return;
                }
                if w.is_on() && !self.gs_active && self.no_wayland_session() {
                    self.suppressed(I, X11_SESSION);
                    self.note(X11_SESSION);
                    return;
                }
                let v = if w.is_on() { "wayland,x11" } else { "x11" };
                self.scoped(I, "SDL_VIDEODRIVER", v);
                self.scoped(I, "SDL_VIDEO_DRIVER", v);
                self.wayland_applied = w.is_on();
                self.applied(I);
            }
            Core::Flatpak => {
                if o.wayland_per_game.is_on() {
                    self.note("Flatpak apps choose Wayland through their own permissions.");
                }
                if o.wayland != Tri::Default {
                    self.rep(I, OptionState::NotApplicable);
                }
            }
            _ => {
                if !self.wayland_possible() {
                    let why = if self.core != Core::Wine {
                        VALVE_WAYLAND
                    } else if self.runner.known {
                        "This Wine has no Wayland driver (winewayland.so)."
                    } else {
                        "Couldn't check this Wine for a Wayland driver (winewayland.so)."
                    };
                    self.unavailable(I, why);
                    return;
                }
                if o.wayland == Tri::Default {
                    return;
                }
                if self.gs_active && !self.gs_expose {
                    self.suppressed(I, GS_WAYLAND);
                    if o.wayland.is_on() {
                        self.note(GS_WAYLAND);
                    }
                    return;
                }
                if o.wayland.is_on() && !self.gs_active && self.no_wayland_session() {
                    self.suppressed(I, X11_SESSION);
                    self.note(X11_SESSION);
                    return;
                }
                if self.core == Core::Wine && o.wayland.is_off() {
                    self.applied(I);
                    return;
                }
                self.emit_wayland(o.wayland.is_on());
                self.applied(I);
            }
        }
    }

    pub(super) fn opt_hdr(&mut self) {
        use OptionId::Hdr as I;
        let o = self.o;
        match (o.hdr, self.core) {
            (Tri::Default, _) => {}
            // HDR is off on every monitor in Plasma: nothing can show HDR, so don't switch the
            // game onto Wine's Wayland driver (or Gamescope into HDR) for nothing.
            (Tri::On, _) if self.caps.kde_hdr_enabled == Some(false) => {
                self.note(DESKTOP_HDR_OFF);
                self.suppressed(I, DESKTOP_HDR_OFF);
                return;
            }
            (Tri::On, Core::Proton | Core::Wine | Core::Client) if !self.gs_active && self.no_wayland_session() => {
                // No Wayland session to give Wine's Wayland driver: only Gamescope does HDR here.
                self.note(X11_HDR);
                self.suppressed(I, X11_HDR);
                return;
            }
            (Tri::On, Core::Proton | Core::Wine | Core::Client) => {
                self.set(I, "DXVK_HDR", "1");
                if self.gs_active {
                    self.gsf.hdr = true;
                } else if self.wayland_possible() && !o.wayland.is_off() {
                    if !self.wayland_applied {
                        self.emit_wayland(true);
                        self.rep(OptionId::Wayland, OptionState::AppliedImplicitly("needed for HDR"));
                    }
                } else if self.runner.known && self.runner.family() == RunnerFamily::Valve {
                    self.note("Valve Proton has no Wayland driver: turn on Gamescope for HDR.");
                } else {
                    self.note(
                        "HDR needs Wayland native or Gamescope; neither is active, so the game will likely show SDR.",
                    );
                }
                self.applied(I);
            }
            (Tri::On, Core::Launcher) => {
                self.set(I, "DXVK_HDR", "1");
                self.applied(I);
            }
            (Tri::On, _) => {
                if self.gs_active {
                    self.gsf.hdr = true;
                    self.applied(I);
                } else {
                    let why = "Native games switch HDR on themselves.";
                    if o.hdr_per_game.is_on() {
                        self.note(why);
                    }
                    self.suppressed(I, why);
                }
            }
            (Tri::Off, Core::Proton | Core::Wine | Core::Client | Core::Launcher) => {
                self.set(I, "DXVK_HDR", "0");
                self.set(I, "DXVK_NO_HDR", "1");
                if self.core != Core::Wine && self.runner.has("PROTON_ENABLE_HDR") {
                    self.set(I, "PROTON_ENABLE_HDR", "0");
                }
                self.applied(I);
            }
            (Tri::Off, _) => self.rep(I, OptionState::NotApplicable),
        }
        if !o.hdr.is_on() {
            return;
        }
        if self.gs_active {
            // Same condition as `opt_gamescope_env` (any active gamescope gets the variable).
            let wsi_off = o.gs_disable_wsi.is_on() || self.user_env_has("ENABLE_GAMESCOPE_WSI", Some("0"));
            if wsi_off {
                self.note("HDR inside Gamescope needs the Gamescope WSI layer.");
            }
            let old = self.caps.gamescope.as_ref().and_then(|g| g.version).is_some_and(|v| v < (3, 16, 27));
            if old && self.caps.kde_hdr_enabled.is_some() {
                self.note("Gamescope older than 3.16.27 may not show HDR on Plasma 6.8+.");
            }
        }
    }

    pub(super) fn opt_hdr_force(&mut self) {
        use OptionId::HdrForce as I;
        let o = self.o;
        if o.hdr_force == Tri::Default {
            return;
        }
        if !self.core.protonish() {
            self.rep(I, OptionState::NotApplicable);
        } else if !self.runner.has("PROTON_FORCE_HDR") {
            self.unavailable(I, "Only GE-Proton has this.");
        } else if o.hdr_force.is_off() {
            self.applied(I);
        } else if o.hdr.is_on() && self.wayland_applied {
            self.set(I, "PROTON_FORCE_HDR", "1");
            self.applied(I);
        } else {
            self.suppressed(I, "Needs HDR with Wayland native.");
        }
    }

    /// Gamescope sub-options that are environment (`gs_disable_wsi`).
    pub(super) fn opt_gamescope_env(&mut self) {
        if self.gs_active && self.o.gs_disable_wsi.is_on() {
            self.set(OptionId::GsDisableWsi, "ENABLE_GAMESCOPE_WSI", "0");
            self.applied(OptionId::GsDisableWsi);
        }
    }

    pub(super) fn opt_hdr_wsi(&mut self) {
        use OptionId::HdrWsiLayer as I;
        match self.o.hdr_wsi_layer {
            Tri::Default => {
                let installed = self.caps.layers.iter().any(|l| l.name == "VK_LAYER_hdr_wsi");
                let old_nvidia = self
                    .caps
                    .nvidia_version
                    .as_deref()
                    .and_then(|v| v.split('.').next()?.trim().parse::<u32>().ok())
                    .is_some_and(|major| major < 595);
                if self.o.hdr.is_on() && installed && old_nvidia && !self.gs_active {
                    self.set(I, "ENABLE_HDR_WSI", "1");
                    self.rep(I, OptionState::AppliedImplicitly("needed for HDR with this NVIDIA driver"));
                }
            }
            Tri::On if self.gs_active => self.suppressed(I, "Never used with Gamescope."),
            Tri::On => {
                self.set(I, "ENABLE_HDR_WSI", "1");
                self.applied(I);
            }
            Tri::Off => {
                self.set(I, "DISABLE_HDR_WSI", "1");
                self.applied(I);
            }
        }
    }

    /// Lodestar's gamescope flags in spec order (interaction flags only for the user's own
    /// gamescope), each checked against the `--help` flag set.
    pub(super) fn gs_words(&mut self) -> Vec<String> {
        let full = self.gs_ours;
        let mut g = std::mem::take(&mut self.gsf);
        // Gamescope's booleans can't be switched off again, so a flag the user typed decides:
        // their window mode replaces ours, and ours aren't repeated.
        let user = self.user_gs_words();
        let typed = |names: &[&str]| has_gs_flag(&user, names);
        if typed(&["-f", "--fullscreen", "-b", "--borderless"]) {
            g.window = None;
        }
        g.adaptive &= !typed(&["--adaptive-sync"]);
        g.hdr &= !typed(&["--hdr-enabled"]);
        g.mangoapp &= !typed(&["--mangoapp"]);
        g.expose &= !typed(&["--expose-wayland"]);
        g.grab &= !typed(&["--force-grab-cursor"]);
        let mut f: Vec<Vec<String>> = vec![];
        let one = |s: &str| vec![s.to_string()];
        let two = |s: &str, v: String| vec![s.to_string(), v];
        if full && let Some(((ow, oh), (gw, gh))) = g.size {
            f.push(two("-W", ow.to_string()));
            f.push(two("-H", oh.to_string()));
            f.push(two("-w", gw.to_string()));
            f.push(two("-h", gh.to_string()));
        }
        if full && let Some(w) = g.window {
            f.push(one(w));
        }
        if let Some(r) = g.rate {
            f.push(two("-r", r.to_string()));
        }
        if full {
            if let Some(x) = g.filter {
                f.push(two("-F", x.to_string()));
            }
            if let Some(s) = g.sharpness {
                f.push(two("--sharpness", s.to_string()));
            }
            if let Some(s) = g.scaler {
                f.push(two("-S", s.to_string()));
            }
            if g.adaptive {
                f.push(one("--adaptive-sync"));
            }
        }
        if g.hdr {
            f.push(one("--hdr-enabled"));
        }
        if g.mangoapp {
            f.push(one("--mangoapp"));
        }
        if let Some(p) = g.prefer {
            f.push(two("--prefer-vk-device", p));
        }
        if full && g.expose {
            f.push(one("--expose-wayland"));
        }
        if full && g.grab {
            f.push(one("--force-grab-cursor"));
        }
        let caps = self.caps;
        let known = caps.gamescope.as_ref().map(|c| &c.flags).filter(|s| !s.is_empty());
        let mut out = vec![];
        for flag in f {
            if let Some(k) = known
                && !k.contains(&flag[0])
            {
                self.note(format!("Your Gamescope doesn't support {}; it was left out.", flag[0]));
                continue;
            }
            out.extend(flag);
        }
        // More Gamescope options: after Lodestar's flags (so they can refine them), before the
        // typed extra arguments (which win). Flags Lodestar manages or that would break the
        // launch are skipped even when a hand-edited config has them.
        if full && !self.o.gs_more.is_empty() {
            use crate::game_options::gamescope::{HIDDEN, MANAGED};
            let mut applied = 0;
            let mut missing: Vec<String> = vec![];
            for (flag, value) in &self.o.gs_more {
                if !flag.starts_with("--") || MANAGED.contains(&flag.as_str()) || HIDDEN.contains(&flag.as_str()) {
                    continue;
                }
                if let Some(k) = known
                    && !k.contains(flag)
                {
                    missing.push(flag.clone());
                    continue;
                }
                out.push(flag.clone());
                if let Some(v) = value {
                    out.push(v.clone());
                }
                applied += 1;
            }
            if !missing.is_empty() {
                self.note(format!("Your Gamescope doesn't support {}; left out.", missing.join(", ")));
            }
            if applied > 0 {
                self.applied(OptionId::GsMore);
            } else if !missing.is_empty() {
                self.unavailable(OptionId::GsMore, "None of these are supported by your Gamescope.");
            }
        }
        if full && let Some(extra) = &self.o.gs_extra_args {
            match crate::launch::split_command(extra) {
                Ok(w) => out.extend(w),
                Err(_) => self.note("Extra Gamescope arguments have an unterminated quote; they were left out."),
            }
        }
        out
    }
}
