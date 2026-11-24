//! GPU options of the composer: the discrete GPU, NTSync, upscalers (Wine FSR, FSR 4, DLSS,
//! XeSS), their indicators and NVAPI.

use super::*;

impl Composer<'_> {
    /// The GPU "Use discrete GPU" picks, or why it can't.
    pub(super) fn dgpu(&self) -> Result<&Gpu, &'static str> {
        if self.caps.gpus.len() < 2 {
            return Err("Only one GPU.");
        }
        // The one ranking the UI uses too (Caps::best_gpu).
        let best = self.caps.best_gpu().ok_or("Only one GPU.")?;
        if best.boot_vga {
            return Err("This system already renders on the discrete GPU.");
        }
        Ok(best)
    }

    pub(super) fn opt_prefer_dgpu(&mut self) {
        use OptionId::PreferDgpu as I;
        let t = self.o.prefer_dgpu;
        let g = self.dgpu().cloned();
        if t == Tri::Default {
            if let Err(why) = g {
                self.unavailable(I, why);
            }
            return;
        }
        if t.is_off() {
            self.applied(I);
            return;
        }
        let g = match g {
            Ok(g) => g,
            Err(why) => {
                self.unavailable(I, why);
                return;
            }
        };
        if g.driver == "nvidia" {
            self.set(I, "__NV_PRIME_RENDER_OFFLOAD", "1");
            self.set(I, "__GLX_VENDOR_LIBRARY_NAME", "nvidia");
            self.set(I, "__VK_LAYER_NV_optimus", "NVIDIA_only");
        } else {
            self.set(I, "DRI_PRIME", format!("pci-{}", g.pci.replace([':', '.'], "_")));
        }
        if self.core.blocking() && self.gs_active {
            self.gsf.prefer = Some(format!("{:04x}:{:04x}", g.vendor, g.device));
        }
        self.applied(I);
    }

    pub(super) fn opt_ntsync(&mut self) {
        use OptionId::Ntsync as I;
        let t = self.o.ntsync;
        match self.core {
            Core::Native | Core::Flatpak => {}
            Core::Wine if !self.chain.proton_wine => {
                self.unavailable(I, "System Wine uses NTSync automatically and has no switch.");
            }
            _ => match t {
                Tri::Default => {
                    if !self.runner.flag(|r| r.wineserver_no_ntsync) && !self.runner.has("PROTON_USE_NTSYNC") {
                        self.unavailable(I, "This Proton has no NTSync switch.");
                    }
                }
                Tri::Off => {
                    if self.runner.flag(|r| r.wineserver_no_ntsync) {
                        self.set(I, "PROTON_NO_NTSYNC", "1");
                        self.applied(I);
                    } else {
                        self.unavailable(I, "This Proton can't turn NTSync off.");
                    }
                }
                Tri::On => {
                    let old_ge = self.runner.known
                        && self.core != Core::Wine
                        && self.runner.rc.vars.contains("PROTON_USE_NTSYNC")
                        && !self.runner.rc.vars.contains("PROTON_NO_NTSYNC");
                    if self.runner.known && !self.runner.rc.ntsync && !old_ge {
                        self.unavailable(I, "This Proton has no NTSync.");
                        return;
                    }
                    // Only a real probe says the kernel side is missing (default caps don't).
                    if self.caps.probed {
                        match self.caps.ntsync {
                            NtsyncState::Usable => {}
                            NtsyncState::NotLoaded => {
                                self.unavailable(I, "The ntsync kernel module isn't loaded.");
                                return;
                            }
                            NtsyncState::Missing => {
                                self.unavailable(I, "This kernel has no NTSync.");
                                return;
                            }
                        }
                    }
                    if old_ge {
                        self.set(I, "PROTON_USE_NTSYNC", "1");
                    }
                    self.applied(I);
                }
            },
        }
    }

    pub(super) fn opt_wine_fsr(&mut self) {
        use OptionId::WineFsr as I;
        let t = self.o.wine_fsr;
        let avail = match self.core {
            Core::Wine => self.runner.strict(|r| r.fshack_fsr),
            c if c.protonish() => self.runner.flag(|r| r.fshack_fsr),
            _ => {
                if t != Tri::Default {
                    self.rep(I, OptionState::NotApplicable);
                }
                return;
            }
        };
        if !avail {
            let why = if self.core == Core::Wine && !self.runner.known {
                "Couldn't check this Wine for fullscreen FSR (win32u.so)."
            } else {
                "This Proton / Wine has no fullscreen FSR (GE-Proton and proton-cachyos have it)."
            };
            self.unavailable(I, why);
            return;
        }
        if t == Tri::Default {
            return;
        }
        if t.is_on() && self.gs_active {
            let why = "Using Gamescope's upscaler instead of Wine FSR.";
            self.note(why);
            self.suppressed(I, why);
            return;
        }
        if t.is_on() && self.wayland_applied {
            let why = "Wine FSR works only on X11; it's off because Wayland native is on.";
            self.note(why);
            self.suppressed(I, why);
            return;
        }
        self.set(I, "WINE_FULLSCREEN_FSR", on_off(t));
        if t.is_on()
            && let Some(s) = self.o.wine_fsr_strength
        {
            self.set(I, "WINE_FULLSCREEN_FSR_STRENGTH", s.min(5).to_string());
        }
        self.applied(I);
    }

    pub(super) fn opt_fsr4(&mut self) {
        use OptionId::Fsr4Upgrade as I;
        let t = self.o.fsr4_upgrade;
        if !self.core.protonish() {
            if t != Tri::Default {
                self.rep(I, OptionState::NotApplicable);
            }
            return;
        }
        // The GPU the game renders on (not any card in the PC: a hybrid laptop's RDNA3 iGPU
        // doesn't make FSR 4 run on its NVIDIA dGPU). No GPU list yet: the gate passes.
        let rdna3 = |g: &Gpu| g.vendor == 0x1002 && g.gc_ip.is_some_and(|(maj, _)| maj >= 11);
        let render = self.caps.rendering_gpu(self.prefer_dgpu_applies());
        let gpu_ok = self.caps.gpus.is_empty() || render.is_some_and(rdna3);
        let var = self.runner.has("PROTON_FSR4_UPGRADE");
        let contrib = self.runner.known && self.runner.rc.contrib_fsr4;
        let why = if !gpu_ok {
            Some(match render {
                Some(g) if self.caps.gpus.iter().any(rdna3) => {
                    format!(
                        "Games render on the {} GPU here; FSR 4 needs an AMD RDNA3 or newer GPU.",
                        vendor_name(g.vendor)
                    )
                }
                _ => "Needs an AMD RDNA3 or newer GPU.".to_string(),
            })
        } else if !var && !contrib {
            Some(format!("Not supported by {}.", self.runner.name()))
        } else {
            None
        };
        if let Some(why) = why {
            self.unavailable(I, &why);
            return;
        }
        self.fsr4_effective = t.is_on() || (t == Tri::Default && self.runner.family() == RunnerFamily::Cachyos);
        match t {
            Tri::Default => return,
            _ if var => self.set(I, "PROTON_FSR4_UPGRADE", on_off(t)),
            Tri::On => self.set(I, "FSR4_UPGRADE", "1"),
            Tri::Off => {}
        }
        self.applied(I);
    }

    pub(super) fn opt_dlss(&mut self) {
        use OptionId::DlssUpgrade as I;
        let t = self.o.dlss_upgrade;
        if !self.core.protonish() {
            if t != Tri::Default {
                self.rep(I, OptionState::NotApplicable);
            }
            return;
        }
        // The GPU the game renders on must be NVIDIA (no GPU list yet: the gate passes).
        let render = self.caps.rendering_gpu(self.prefer_dgpu_applies());
        if !self.caps.gpus.is_empty() && !render.is_some_and(|g| g.vendor == 0x10de) {
            let why = match render {
                Some(g) if self.caps.gpus.iter().any(|g| g.vendor == 0x10de) => format!(
                    "Games render on the {} GPU here; DLSS needs the NVIDIA GPU (turn on Use discrete GPU).",
                    vendor_name(g.vendor)
                ),
                _ => "Needs an NVIDIA RTX GPU.".to_string(),
            };
            self.unavailable(I, &why);
            return;
        }
        if !self.runner.has("PROTON_DLSS_UPGRADE") {
            let why = format!("Not supported by {} (GE-Proton 10-26+ or proton-cachyos).", self.runner.name());
            self.unavailable(I, &why);
            return;
        }
        if t == Tri::Default {
            return;
        }
        if self.o.nvapi == Nvapi::Off {
            self.suppressed(I, "DLSS needs NVAPI.");
            return;
        }
        self.set(I, "PROTON_DLSS_UPGRADE", on_off(t));
        if self.user_env_has("DXVK_NVAPI_DRS_SETTINGS", None) {
            self.note("Your DXVK_NVAPI_DRS_SETTINGS replaces the DLSS-upgrade presets.");
        }
        self.applied(I);
    }

    pub(super) fn opt_xess(&mut self) {
        use OptionId::XessUpgrade as I;
        let t = self.o.xess_upgrade;
        if !self.core.protonish() {
            if t != Tri::Default {
                self.rep(I, OptionState::NotApplicable);
            }
            return;
        }
        if !self.runner.has("PROTON_XESS_UPGRADE") {
            let why = format!("Not supported by {}.", self.runner.name());
            self.unavailable(I, &why);
            return;
        }
        if t != Tri::Default {
            self.set(I, "PROTON_XESS_UPGRADE", on_off(t));
            self.applied(I);
        }
    }

    pub(super) fn opt_indicator(&mut self) {
        use OptionId::UpscalerIndicator as I;
        let t = self.o.upscaler_indicator;
        if t == Tri::Default {
            return;
        }
        if !matches!(self.core, Core::Proton | Core::Client | Core::Launcher | Core::Wine) {
            self.rep(I, OptionState::NotApplicable);
            return;
        }
        let v = on_off(t);
        let mut any = false;
        if self.core.protonish() {
            if self.runner.has("PROTON_FSR4_INDICATOR") && (t.is_off() || self.fsr4_effective) {
                self.set(I, "PROTON_FSR4_INDICATOR", v);
                any = true;
            }
            if self.runner.has("PROTON_DLSS_INDICATOR") {
                self.set(I, "PROTON_DLSS_INDICATOR", v);
                any = true;
            }
        }
        let valve_or_wine = self.core == Core::Wine || self.runner.family() == RunnerFamily::Valve;
        if !any && self.nvidia() && valve_or_wine {
            let val =
                if t.is_on() { "DLSSIndicator=1024,DLSSGIndicator=2," } else { "DLSSIndicator=0,DLSSGIndicator=0," };
            self.set(I, "DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS", val);
            any = true;
        }
        if any {
            self.applied(I);
        } else {
            let why = format!("No upscaler indicator with {}.", self.runner.name());
            self.unavailable(I, &why);
        }
    }

    pub(super) fn opt_nvapi(&mut self) {
        use OptionId::Nvapi as I;
        let n = self.o.nvapi;
        if n == Nvapi::Default {
            return;
        }
        match self.core {
            c if c.protonish() => {
                let (k, v) = if n == Nvapi::Off { ("PROTON_DISABLE_NVAPI", "1") } else { ("PROTON_FORCE_NVAPI", "1") };
                self.set(I, k, v);
                self.applied(I);
            }
            Core::Wine => self.unavailable(I, "Proton only."),
            _ => self.rep(I, OptionState::NotApplicable),
        }
    }
}
