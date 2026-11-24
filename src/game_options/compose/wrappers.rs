//! Wrapper options of the composer: GameMode, power profile, keep awake, MangoHud, frame limit,
//! lsfg-vk and OBS capture.

use super::*;

impl Composer<'_> {
    pub(super) fn opt_gamemode(&mut self) {
        use OptionId::Gamemode as I;
        let t = self.o.gamemode;
        let have = self.caps.bins.gamemoderun.is_some();
        let native_like = matches!(self.core, Core::Native | Core::Proton | Core::Wine);
        if t == Tri::Default {
            if native_like && !have {
                self.unavailable(I, "GameMode isn't installed.");
            }
            return;
        }
        if self.wrapper_blocked(I, t.is_on()) {
            return;
        }
        if t.is_off() {
            self.applied(I);
        } else if self.core == Core::Flatpak {
            self.preload_labels.push(label(I).into());
            self.suppressed(I, "Flatpak apps can't take GameMode (Flatpak drops LD_PRELOAD).");
        } else if self.chain.peeled_has(&["gamemoderun"]) {
            self.applied(I);
        } else if !have {
            self.unavailable(I, "GameMode isn't installed.");
        } else {
            self.wrap.gamemode = true;
            self.applied(I);
        }
    }

    /// Lodestar's own power-profile wrapper uses `game-performance`, or the command has it.
    pub(super) fn uses_game_performance(&self) -> bool {
        self.wrap.perf.as_ref().is_some_and(|w| w[0] == "game-performance")
            || self.chain.peeled_has(&["game-performance"])
    }

    pub(super) fn opt_performance(&mut self) {
        use OptionId::PerformanceProfile as I;
        let t = self.o.performance_profile;
        let b = &self.caps.bins;
        let gp = b.game_performance.is_some() && (b.powerprofilesctl.is_some() || b.tlpctl.is_some());
        let ppc = b.powerprofilesctl.is_some();
        if t == Tri::Default {
            if self.core.blocking() && !gp && !ppc {
                self.unavailable(I, "No power profile tool (power-profiles-daemon) is installed.");
            }
            return;
        }
        if self.wrapper_blocked(I, t.is_on()) {
            return;
        }
        if t.is_off() || self.chain.peeled_has(&["game-performance", "powerprofilesctl", "tlpctl"]) {
            self.applied(I);
        } else if gp {
            self.wrap.perf = Some(vec!["game-performance".into()]);
            self.applied(I);
        } else if ppc {
            let reason = format!("Lodestar: {}", self.input.name);
            self.wrap.perf = Some(
                ["powerprofilesctl", "launch", "-p", "performance", "-r", &reason, "--"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
            self.applied(I);
        } else {
            self.unavailable(I, "No power profile tool (power-profiles-daemon) is installed.");
        }
    }

    pub(super) fn opt_keep_awake(&mut self) {
        use OptionId::KeepAwake as I;
        let t = self.o.keep_awake;
        let have = self.caps.bins.systemd_inhibit.is_some();
        if t == Tri::Default {
            if self.core.blocking() && !have {
                self.unavailable(I, "systemd-inhibit isn't installed.");
            }
            return;
        }
        if self.wrapper_blocked(I, t.is_on()) {
            return;
        }
        let gp = self.uses_game_performance();
        if t.is_off() {
            if gp {
                self.set(I, "GAME_PERFORMANCE_SCREENSAVER_ON", "1");
            }
            self.applied(I);
            return;
        }
        if self.chain.peeled_has(&["systemd-inhibit"]) {
            self.applied(I);
        } else if have {
            let why = format!("--why=Playing {}", self.input.name);
            self.wrap.inhibit = Some(
                ["systemd-inhibit", "--what=idle:sleep", "--who=Lodestar", &why]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
            if gp {
                // One inhibitor: game-performance leaves the screensaver alone.
                self.set(I, "GAME_PERFORMANCE_SCREENSAVER_ON", "1");
            }
            self.applied(I);
        } else if gp {
            // game-performance inhibits the screensaver itself.
            self.applied(I);
        } else {
            self.unavailable(I, "systemd-inhibit isn't installed.");
        }
    }

    pub(super) fn opt_mangohud(&mut self) {
        use OptionId::Mangohud as I;
        let t = self.o.mangohud;
        let user_mh = self.chain.peeled_has(&["mangohud"]);
        if self.core.blocking() && self.gs_active && (user_mh || self.user_env_has("MANGOHUD", Some("1"))) {
            self.note(MANGO_IN_GS);
        }
        self.mangohud_applied = user_mh && !self.gs_active;
        self.mango_user_on = user_mh || self.user_typed("MANGOHUD", Some("1"));
        let have = self.caps.bins.mangohud.is_some();
        if t == Tri::Default {
            if matches!(self.core, Core::Native | Core::Proton | Core::Wine) && !have && !self.gs_active {
                self.unavailable(I, "MangoHud isn't installed.");
            }
            return;
        }
        if !self.play {
            if t.is_on() {
                self.suppressed(I, ONLY_PLAY);
            }
            return;
        }
        if t.is_off() {
            if self.mango_user_on {
                // The user's own `mangohud` / MANGOHUD=1 wins over an Off default.
                self.suppressed(I, TYPED_ON);
            } else {
                self.set(I, "DISABLE_MANGOHUD", "1");
                self.applied(I);
            }
            return;
        }
        match self.core {
            _ if self.core.blocking() && self.gs_active => {
                if self.caps.bins.mangoapp.is_some() {
                    self.gsf.mangoapp = true;
                    self.mangohud_applied = true;
                    self.applied(I);
                } else {
                    self.unavailable(I, "mangoapp isn't installed (it comes with MangoHud); needed inside Gamescope.");
                }
            }
            Core::Native | Core::Proton | Core::Wine => {
                if user_mh {
                    self.applied(I);
                } else if have {
                    self.wrap.mangohud = true;
                    self.mangohud_applied = true;
                    self.applied(I);
                } else {
                    self.unavailable(I, "MangoHud isn't installed.");
                }
            }
            Core::Flatpak => {
                if self.flatpak_ext("org.freedesktop.Platform.VulkanLayer.MangoHud") {
                    self.set(I, "MANGOHUD", "1");
                    self.mangohud_applied = true;
                    self.applied(I);
                } else {
                    self.unavailable(
                        I,
                        "Needs the Flatpak extension: flatpak install flathub org.freedesktop.Platform.VulkanLayer.MangoHud",
                    );
                }
            }
            _ => {
                // Launcher / client: env only (Vulkan games).
                self.set(I, "MANGOHUD", "1");
                self.mangohud_applied = true;
                self.applied(I);
            }
        }
    }

    pub(super) fn opt_frame_limit(&mut self) {
        use OptionId::FrameLimit as I;
        let Some(n) = self.o.frame_limit else { return };
        if !self.play {
            self.suppressed(I, ONLY_PLAY);
            return;
        }
        let core = self.core;
        // MangoHud Off sets DISABLE_MANGOHUD=1, which would also switch a MangoHud limiter off
        // (unless the user's own text keeps MangoHud on, then there is no DISABLE_MANGOHUD).
        let mango_ok = !self.o.mangohud.is_off() || self.mango_user_on;
        // A MANGOHUD_CONFIG makes MangoHud skip MangoHud.conf unless it starts with read_cfg; the
        // user's own MANGOHUD_CONFIG already made that choice.
        let read_cfg = !self.user_env_has("MANGOHUD_CONFIG", None);
        // The user's MANGOHUD=1 shows the HUD: the limiter-only form must not hide it.
        let hud_wanted = self.user_env_has("MANGOHUD", Some("1"));
        if core.blocking() && self.gs_active {
            let mut words = self.user_gs_words();
            if self.gs_ours {
                // Only the More Gamescope options gs_words will actually pass (this gamescope
                // documents them); a left-out --framerate-limit limits nothing.
                let known = self.caps.gamescope.as_ref().map(|c| &c.flags).filter(|s| !s.is_empty());
                words.extend(
                    self.o.gs_more.iter().map(|(f, _)| f).filter(|f| known.is_none_or(|k| k.contains(*f))).cloned(),
                );
            }
            if has_gs_flag(&words, &["--framerate-limit"]) {
                // Never both -r and --framerate-limit: they compound.
                self.suppressed(I, "Your Gamescope flags already limit the frame rate (--framerate-limit).");
                return;
            }
            if has_gs_flag(&words, &["-r", "--nested-refresh"]) {
                self.suppressed(I, "Your Gamescope flags set the frame rate (-r).");
                return;
            }
            self.gsf.rate = Some(n);
        } else if self.mangohud_applied {
            if read_cfg {
                self.list(I, "MANGOHUD_CONFIG", ',', "read_cfg");
            }
            self.list(I, "MANGOHUD_CONFIG", ',', format!("fps_limit={n}"));
        } else if matches!(core, Core::Proton | Core::Wine | Core::Client) {
            for k in ["dxvk", "dxgi", "d3d9"] {
                self.list(I, "DXVK_CONFIG", ';', format!("{k}.maxFrameRate={n}"));
            }
            self.set(I, "VKD3D_FRAME_RATE", n.to_string());
        } else if mango_ok
            && ((core == Core::Native && self.caps.bins.mangohud.is_some())
                || (core == Core::Flatpak && self.flatpak_ext("org.freedesktop.Platform.VulkanLayer.MangoHud"))
                || core == Core::Launcher)
        {
            // MangoHud as a limiter only: its HUD hidden, unless the user asked for it.
            if core == Core::Native {
                self.wrap.mangohud = true;
            } else {
                self.set(I, "MANGOHUD", "1");
            }
            if !hud_wanted {
                self.list(I, "MANGOHUD_CONFIG", ',', "no_display");
            } else if read_cfg {
                self.list(I, "MANGOHUD_CONFIG", ',', "read_cfg");
            }
            self.list(I, "MANGOHUD_CONFIG", ',', format!("fps_limit={n}"));
        } else {
            let why = "Frame limit needs Gamescope or MangoHud for this game.";
            self.note(why);
            self.unavailable(I, why);
            return;
        }
        self.applied(I);
    }

    pub(super) fn opt_lsfg(&mut self) {
        use OptionId::Lsfg as I;
        let o = self.o;
        let t = o.lsfg;
        let caps = self.caps;
        let Some(lc) = caps.lsfg.as_ref() else {
            self.unavailable(I, "lsfg-vk isn't installed.");
            return;
        };
        if t == Tri::Default {
            return;
        }
        if !self.play {
            self.suppressed(I, ONLY_PLAY);
            return;
        }
        let v2 = lc.version != 1;
        if t.is_off() {
            self.scoped(I, if v2 { "DISABLE_LSFGVK" } else { "DISABLE_LSFG" }, "1");
            self.applied(I);
            return;
        }
        match o.lsfg_profile.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) if v2 => self.scoped(I, "LSFGVK_PROFILE", p),
            Some(p) => match lc.profiles.iter().find(|(n, _)| n == p).and_then(|(_, exes)| exes.first()) {
                Some(exe) => self.scoped(I, "LSFG_PROCESS", exe.clone()),
                None => {
                    self.suppressed(I, &format!("lsfg-vk has no profile “{p}” with a program."));
                    return;
                }
            },
            None => {
                let (pre, env_key) = if v2 { ("LSFGVK_", "LSFGVK_ENV") } else { ("LSFG_", "LSFG_LEGACY") };
                self.scoped(I, env_key, "1");
                self.scoped(I, &format!("{pre}MULTIPLIER"), o.lsfg_multiplier.to_string());
                // A hand-edited config can hold NaN / inf; those are left out.
                if let Some(f) = o.lsfg_flow_scale.filter(|f| f.is_finite()) {
                    self.scoped(I, &format!("{pre}FLOW_SCALE"), f.clamp(0.25, 1.0).to_string());
                }
                if o.lsfg_performance.is_on() {
                    self.scoped(I, &format!("{pre}PERFORMANCE_MODE"), "1");
                }
                match &lc.dll {
                    Some(d) => self.scoped(I, &format!("{pre}DLL_PATH"), d.to_string_lossy().into_owned()),
                    None => self
                        .note("Lossless Scaling wasn't found in your Steam libraries; lsfg-vk needs its Lossless.dll."),
                }
            }
        }
        if self.core == Core::Flatpak {
            self.note("Frame generation needs the Flatpak extension org.freedesktop.Platform.VulkanLayer.lsfgvk and access to ~/.config/lsfg-vk.");
        }
        if self.core == Core::Client {
            let client = self.chain.launcher.clone().unwrap_or_default();
            self.note(format!("Frame generation also applies to {client}'s window."));
        }
        self.applied(I);
    }

    pub(super) fn opt_obs(&mut self) {
        use OptionId::ObsCapture as I;
        let t = self.o.obs_capture;
        let bin = self.caps.bins.obs_gamecapture.is_some();
        let layer = self.caps.layers.iter().any(|l| l.name.starts_with("VK_LAYER_OBS_vkcapture"));
        if t == Tri::Default {
            if !bin && !layer {
                self.unavailable(I, "obs-vkcapture isn't installed.");
            }
            return;
        }
        if !self.play {
            if t.is_on() {
                self.suppressed(I, ONLY_PLAY);
            }
            return;
        }
        if t.is_off() {
            if self.chain.peeled_has(&["obs-gamecapture"]) || self.user_typed("OBS_VKCAPTURE", Some("1")) {
                self.suppressed(I, TYPED_ON);
            } else {
                self.scoped(I, "DISABLE_OBS_VKCAPTURE", "1");
                self.applied(I);
            }
            return;
        }
        let gl_wrapper = matches!(self.core, Core::Native | Core::Wine) && !self.gs_active && bin;
        if self.core == Core::Flatpak && !self.flatpak_ext("org.freedesktop.Platform.VulkanLayer.OBSVkCapture") {
            self.unavailable(I, "Needs the Flatpak extension: flatpak install flathub org.freedesktop.Platform.VulkanLayer.OBSVkCapture");
            return;
        }
        if gl_wrapper {
            if !self.chain.peeled_has(&["obs-gamecapture"]) {
                self.wrap.obs = true;
            }
        } else if bin || layer || self.core == Core::Flatpak {
            self.scoped(I, "OBS_VKCAPTURE", "1");
        } else {
            self.unavailable(I, "obs-vkcapture isn't installed.");
            return;
        }
        self.applied(I);
    }
}
