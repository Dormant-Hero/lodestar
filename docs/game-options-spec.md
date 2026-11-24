# Lodestar 4d: Game Options, Implementation Spec

Written before the feature was built; nothing was edited or run for it. It was based on the game model, configuration and launch code of that time.

Tags used below:
- **[V]** verified from source or docs (taken from the research and its fact-checks).
- **[U]** unverified or assumed. Each [U] item needs a real-machine check before it is treated as a settled fact.

Where the research and the fact-checks disagree, the fact-check corrections are applied throughout.

---

## 0. Ground rules

1. **Toggles never go through `LaunchOverride.options` text or `Options`.**
   - They are applied inside `resolve_parts`, after the base command is known and around the user's options.
   - The Steam/URL `bail!` rules (launch.rs:741-753) are unchanged.
   - A global default can therefore never turn a Steam game into an error.
2. **User text wins.**
   - A scalar env key the user typed overrides the toggle's value.
   - A wrapper the user typed replaces Lodestar's copy of it.
   - The user's gamescope flags come after Lodestar's, so they win (gamescope: last option wins **[V]**).
3. **`resolve` stays pure and cheap.**
   - All detection is precomputed into `Caps`.
   - Runner capabilities are cached per directory.
   - Nothing is spawned during resolve.
4. **Which toggles apply to which action.**
   - Env toggles apply to every action. This keeps PROTON_NO_NTSYNC and similar the same for every process that shares a wineserver.
   - Wrapper toggles (gamescope, gamemode, mangohud, obs, lsfg, power profile, keep awake, frame limit) apply only to the Play launch: `action.is_none() || action.play`.
5. **Phases.**
   - **P1** is everything in this spec except the items marked P2.
   - **P2** is the sidecar session holders for Steam, URL, launcher and client launches (§2.9), plus GameMode `RegisterGameByPID`.

---

## 1. Option list

Every field is a per-game override. **Tri = Default / On / Off.**
- Per game, "Default" follows the global setting.
- Globally, "Default" means "Lodestar adds nothing; the tool or runner decides".
- **A** = shown under the "Advanced" fold.

Sub-settings are shown only while their parent option is effectively On.

### Display
| id | Label | Help text | Type |
|---|---|---|---|
| `wayland` | Wayland native | Run Windows games on Wine's Wayland driver instead of XWayland. Needs GE-Proton or proton-cachyos. For native games, affects SDL games only; set it per game. | tri |
| `hdr` | HDR | Let the game output HDR. Needs HDR switched on in System Settings → Display, plus Wayland native or Gamescope. | tri |
| `hdr_force` (A) | Force HDR (GE-Proton) | For KDE setups where SDR brightness equals peak brightness and the game doesn't see HDR. HDR monitors only. | tri |
| `gamescope` | Gamescope | Run the game inside a Gamescope window, for upscaling, frame limiting, HDR and a fixed resolution. | tri |
| `gs_window` | Window mode | Fullscreen / Borderless window / Window. | enum `WindowMode`, default Fullscreen |
| `gs_output` | Output resolution | Auto = the monitor Lodestar is on. | resolution `Res` (Auto / W×H) |
| `gs_internal` | Game resolution | What the game renders at. Native, 77 % (Ultra quality), 67 % (Quality), 58 % (Balanced), 50 % (Performance), or Custom. | `InternalRes` |
| `gs_filter` | Upscaler | Auto (FSR when upscaling, else linear), Linear, Nearest, FSR, NIS, SGSR, Pixel. SGSR is only offered if gamescope lists it. | enum `GsFilter` |
| `gs_sharpness` (A) | Sharpness | 0 = sharpest, 20 = softest. Gamescope's default is 2. Used with FSR, NIS and SGSR. | number 0–20 |
| `gs_scaler` (A) | Scaling | Auto / Fit / Integer / Fill / Stretch. | enum `GsScaler` |
| `gs_adaptive_sync` | Adaptive sync (VRR) | Also set Adaptive sync to Automatic in System Settings → Display. | tri |
| `gs_grab_cursor` (A) | Lock mouse to the game | For games whose camera spins or whose cursor escapes. | tri |
| `gs_expose_wayland` (A) | Expose Wayland | Let Wayland-native games connect to Gamescope. HDR normally works better without this. | tri |
| `gs_disable_wsi` (A) | Disable Gamescope WSI layer | NVIDIA workaround for crashes with upscalers. Turns off HDR. | tri |
| `gs_extra_args` (A) | Extra gamescope arguments | Added after Lodestar's flags, so these win. | text |

### Performance
| id | Label | Help | Type |
|---|---|---|---|
| `gamemode` | GameMode | Feral GameMode: CPU governor, I/O priority and screensaver inhibit while playing. | tri |
| `performance_profile` | Performance power profile | Switches the power profile to Performance while playing (CachyOS `game-performance`, or `powerprofilesctl`). | tri |
| `keep_awake` | Keep screen awake | Stop the screen from dimming, locking or suspending while playing. | tri |
| `frame_limit` | Frame limit | FPS cap. Uses Gamescope, MangoHud or DXVK, whichever applies. 0 = no limit. | number (`Option<u32>`; per game `Some(0)` = off) |
| `prefer_dgpu` | Use discrete GPU | Render on the discrete GPU on hybrid systems. | tri |
| `ntsync` (A) | NTSync | Fast Windows sync primitives. On by default with current Proton when the kernel supports it; Off disables it for this game. | tri |

### Upscaling & frame generation
| id | Label | Help | Type |
|---|---|---|---|
| `wine_fsr` | Wine fullscreen FSR | Set the game to a fullscreen resolution below your monitor's; Wine upscales it with FSR 1. GE-Proton/cachyos, X11 only. | tri |
| `wine_fsr_strength` (A) | FSR sharpness | 0 = sharpest … 5. | number 0–5 |
| `fsr4_upgrade` | FSR 4 upgrade | Replace a game's FSR 3.1 with FSR 4 (RDNA4; RDNA3 is experimental). | tri |
| `dlss_upgrade` | DLSS upgrade | Use the newest DLSS DLLs and presets (NVIDIA RTX). | tri |
| `xess_upgrade` (A) | XeSS upgrade | Use the newest XeSS. | tri |
| `upscaler_indicator` (A) | Upscaler indicator | Show an on-screen FSR4/DLSS indicator, to check whether an upgrade worked. | tri |
| `lsfg` | Frame generation (lsfg-vk) | Lossless Scaling frame generation. Needs lsfg-vk and Lossless Scaling (Steam). | tri |
| `lsfg_profile` | lsfg-vk profile | One of the profiles in `~/.config/lsfg-vk/conf.toml`, or "Lodestar settings". | enum (profile names + `None`) |
| `lsfg_multiplier` | Multiplier | 2×, 3× or 4×. Used with "Lodestar settings". | number 2–4 |
| `lsfg_flow_scale` (A) | Flow scale | 0.25–1.0. | number |
| `lsfg_performance` (A) | Performance mode | Lighter model. | tri |

### Overlay & capture
| id | Label | Help | Type |
|---|---|---|---|
| `mangohud` | MangoHud | Performance overlay. Inside Gamescope it uses mangoapp. | tri |
| `obs_capture` | OBS game capture | Lets OBS's "Game Capture" source (obs-vkcapture) see the game. | tri |

### Compatibility
| id | Label | Help | Type |
|---|---|---|---|
| `nvapi` | NVAPI (DLSS, Reflex) | Auto / Off / Force. Auto = on with NVIDIA drivers. | enum `Nvapi` |
| `sdl_input` (A) | Use SDL for controllers | Fixes some controller problems (GE-Proton/cachyos). | tri |
| `audio_latency` | Audio crackling fix | Larger audio buffer (60 ms) for Wine/PulseAudio games. | tri |

### Advanced
| id | Label | Help | Type |
|---|---|---|---|
| `proton_log` | Proton log | Write a Proton log to Lodestar's log folder for this game. | tri |
| `hdr_wsi_layer` | HDR WSI layer (legacy) | Only for NVIDIA drivers older than 595 without Gamescope. Auto picks it. | tri |
| `layers` | Vulkan layers (detected) | One Default/On/Off row per installed implicit Vulkan layer that Lodestar doesn't handle elsewhere (e.g. vkBasalt, AFMF, MAKO). | `BTreeMap<String, Tri>` |

**Left out on purpose:**
- Hide NVIDIA GPU: it defeats DLSS.
- esync/fsync off: obsolete or patched builds only.
- OptiScaler: needs its own config.
- cachyos DXVK/VKD3D low latency: one runner only. Users can type the variable.
- `WINE_USE_KWIN_HACKS`: not needed on Plasma ≥ 6.4/6.6 **[V]**.
- `--backend sdl`: it loses HDR and VRR **[V]**; users can put it in `gs_extra_args`.
- **"Gamescope refresh"**: nested gamescope has no output-refresh flag. `-r` is a game FPS cap **[V]**, so it is exposed as `frame_limit`. Without `-r`, gamescope follows the real refresh rate **[V]**.

---

## 2. What each option emits, per launch kind

**Kinds.** These come from `classify()` (§3.2), which looks at the core command after peeling off known wrappers.

| Code | Kind |
|---|---|
| N | Native program |
| P | umu-run, or a `<dir>/proton` script |
| W | plain wine |
| F | `flatpak run` |
| L | Launcher CLI: faugus-launcher, lutris, heroic, bottles-cli, legendary, nile, itch, prismlauncher, atlauncher, moonlight, any `is_self_program` other than the client helper, and flatpak apps whose id is a launcher (`net.lutris.Lutris`, `com.usebottles.bottles`, `com.heroicgameslauncher.hgl`, the Prism, ATLauncher and Moonlight ids the scanners emit, Faugus' flatpak id, or anything with `--command=bottles-cli`) |
| C | Store-client helper (`@lodestar --client-launch`) |
| S | Steam / SteamShortcut |
| U | OpenUrl |

**Per-kind rules.**
- **Blocking kinds (N, P, W, F)** get the full chain.
- **L** gets env only, with `NOTE_LAUNCHER`.
  - legendary exits right away, and gamescope's reaper SIGTERMs the descendants **[V]**.
  - Lutris forwards to its running instance without the caller's env **[V]**.
- **C** gets env only, with `NOTE_CLIENT`.
  - Env reaches the client and its games only on a cold start **[U: inheritance via wineserver]**.
  - Wrappers would die with the helper, gamescope would kill the client, and `LODESTAR_HOME` forwarding breaks if the helper is not `program[0]` **[V]**.
- **S and U** get nothing, with `NOTE_STEAM` / `NOTE_URL`. P2 adds sidecars.

**Placement.** "top" = process env. "scoped" = `env K=V` after gamescope's `--` when gamescope is active; otherwise it is the same as top. "F" = for flatpak, also `--env=K=V`, inserted right after the `run` word.

**Runner capabilities.** `rc` is the RunnerCaps from §4.3. `rc.vars` holds the variable names found in `proton` and `protonfixes/*.py`. If the runner is Unknown (umu `PROTONPATH` not resolvable, Steam string), act as if every `PROTON_*` variable is supported and grey nothing out.

### 2.1 Display
**wayland**
- **P**
  - Requires `PROTON_ENABLE_WAYLAND ∈ rc.vars`. Otherwise Unavailable: "Valve Proton has no Wayland switch — use GE-Proton or proton-cachyos."
  - On → `PROTON_ENABLE_WAYLAND=1`. Off → `PROTON_ENABLE_WAYLAND=0`.
  - GE and cachyos act on it only when `WAYLAND_DISPLAY` is in the env **[V]**.
- **W**
  - Requires `winewayland.so` next to that wine (`<wineroot>/lib/wine/x86_64-unix/`, `/usr/lib/wine/…`, `/usr/lib64/wine/…`).
  - On → merge `winex11.drv=d` into `WINEDLLOVERRIDES` (`;`-list). **[U]** inferred from the loader; GE10 and cachyos rely on it.
  - Off → nothing.
- **N**
  - The global On is ignored. Only a per-game value applies, because SDL2 ≤ 2.0.14 fails with a driver list **[V]**.
  - On → `SDL_VIDEODRIVER=wayland,x11`, `SDL_VIDEO_DRIVER=wayland,x11`.
  - Off → both set to `x11`.
- **F:** not applicable. No note unless the per-game value is On ("Flatpak apps choose Wayland through their own permissions").
- **C:** as P for the client's runner, plus a note: "Store clients (Battle.net, EA, Ubisoft) may show a white window on Wine's Wayland driver."
- **L:** as P (env only).
- **S, U:** listed in the kind note.
- **With gamescope active and `gs_expose_wayland` not On:** suppressed. Note: "Wayland native is ignored inside Gamescope (games run on Gamescope's X server)." Gamescope unsets `WAYLAND_DISPLAY` and SDL_* and sets `XDG_SESSION_TYPE=x11` **[V]**.
- **With `gs_expose_wayland` On:** applied. The SDL_* variables go in scoped env, because gamescope strips them before starting the child **[V]**.

**hdr** (absorbs `edits.hdr`)
- **On, P or W or C:**
  - Top: `DXVK_HDR=1`. This alone is the same as GE10's and GE11's `PROTON_ENABLE_HDR=1` mapping, and it avoids GE11's compat flag overwriting a user `DXVK_HDR=0` **[V]**.
  - If gamescope is active: add `--hdr-enabled`.
  - Else, if the runner supports Wayland (P) or `winewayland.so` exists (W): the Wayland vars are applied as if `wayland`=On ("needed for HDR"), unless `wayland` is effectively Off.
  - If neither applies, note:
    - Valve: "Valve Proton has no Wayland driver: turn on Gamescope for HDR."
    - Others: "HDR needs Wayland native or Gamescope; neither is active, so the game will likely show SDR."
- **On, N or F:** gamescope active → `--hdr-enabled`. Else nothing; per-game On gives the note "Native games switch HDR on themselves."
- **Off, P/W/C/L:** `DXVK_HDR=0`, `DXVK_NO_HDR=1`, and `PROTON_ENABLE_HDR=0` if that variable is in `rc.vars`. GE11 maps `=0` to a real off **[V]**. `DXVK_NO_HDR` is a cachyos/GE DXVK patch and is ignored elsewhere **[U]**.
- **ENABLE_HDR_WSI is never emitted by `hdr`.** See `hdr_wsi_layer`.
- **hdr_force (GE only):** requires `PROTON_FORCE_HDR ∈ rc.vars`. On → `PROTON_FORCE_HDR=1` (GE maps it to `DXVK_HDR=1` + `WAYLANDDRV_FORCE_HDR=1`) **[V]**. Only when `hdr` is On and Wayland is applied.

**gamescope** (applies to N, P, W, F only)
- **L / C:** Suppressed. Note: "Gamescope can't wrap a launcher or store client: Gamescope closes when the launcher exits and would stop the game."
- **S / U:** listed in the kind note.
- **Nested:** if `Caps.nested_gamescope` (`GAMESCOPE_WAYLAND_DISPLAY` set or `XDG_CURRENT_DESKTOP=gamescope`), Suppressed. Note: "Lodestar is already running inside Gamescope." A nested copy would land on the SDL backend without HDR or VRR **[V]**.
- **Words**, in this exact order:
  - `gamescope`
  - `-W OW -H OH`, when the output size is known
  - `-w GW -h GH`, always when the output size is known. The internal size is fixed at start-up, otherwise it defaults to 1280×720 **[V]**.
  - `-f` (Fullscreen) | `-b` (Borderless) | nothing (Window)
  - `-r N` (frame_limit)
  - `-F <filter>`. Auto = `fsr` when GW<OW, else omitted. If the filter is not in the `--help` set (e.g. `sgsr` before 3.16.29), fall back to `fsr` with a note.
  - `--sharpness N` (only with fsr/nis/sgsr)
  - `-S <scaler>` (only when not Auto)
  - `--adaptive-sync`
  - `--hdr-enabled`
  - `--mangoapp`
  - `--prefer-vk-device VVVV:DDDD`
  - `--expose-wayland`
  - `--force-grab-cursor`
  - `<gs_extra_args words>`
  - `<user's own gamescope flags>`
  - `--`
- **Output size:**
  - `gs_output` Custom, else `LaunchEnv.monitor`, else the sysfs preferred mode.
  - If none: omit `-W/-H/-w/-h`, force `-f`, and note "Couldn't detect the monitor resolution; the game will start at 1280×720. Set Output resolution."
- **Internal size:** `Percent(p)` → `w=(OW*p+50)/100`, `h=(OH*p+50)/100`.
- **Feature check:** every flag is checked against the cached `--help` text. An unknown option makes gamescope exit 1 **[V]**. A missing flag is dropped with a note.
- **`gs_disable_wsi` On:** top `ENABLE_GAMESCOPE_WSI=0`. Gamescope sets this variable without overwrite **[V]**.
- **F only:** add `--nosocket=wayland` after `run` unless `gs_expose_wayland` is On. Without it, an app could escape gamescope by connecting to `wayland-0` **[V]** libwayland fallback; **[U]** the flatpak result.
- **P using the `<dir>/proton run` verb:** rewrite to `waitforexitandrun` when gamescope wraps it, so gamescope doesn't exit on launcher-exe hand-off.

### 2.2 Performance
**gamemode** (N, P, W)
- Wrapper `gamemoderun` at position [6].
- **F:** Suppressed. `NOTE_FLATPAK_PRELOAD`; Flatpak drops LD_PRELOAD **[V]**.
- **L / C / S / U:** listed in the kind note; P2 sidecar `gamemoded -r`.
- Known limitation: under umu with gamescope, umu clears LD_PRELOAD **[V]**. Only the umu process registers, so global effects work but ioprio and pinning do not reach the game. P2 adds `RegisterGameByPID`.

**performance_profile** (N, P, W, F)
- If `game-performance` is on PATH and (`powerprofilesctl` or `tlpctl` is on PATH): wrapper `game-performance` [2]. The script exits 1 without them **[V]**.
- Else if `powerprofilesctl`: wrapper `powerprofilesctl launch -p performance -r "Lodestar: <name>" --` [2].
- Else Unavailable.
- **L / C / S / U:** kind note; P2 sidecar.

**keep_awake** (N, P, W, F)
- On → wrapper `systemd-inhibit --what=idle:sleep --who=Lodestar "--why=Playing <name>"` [1].
- If `performance_profile` uses game-performance:
  - keep_awake On → also top `GAME_PERFORMANCE_SCREENSAVER_ON=1`, so there is one inhibitor and it does not block shutdown **[V]**.
  - keep_awake Off → top `GAME_PERFORMANCE_SCREENSAVER_ON=1` and no inhibitor.
  - keep_awake Default → game-performance's own inhibitor applies.
- **L / C / S / U:** kind note; P2 sidecar.

**frame_limit = N > 0.** Pick one backend, first match wins:
1. Gamescope active → `-r N`. Never `--framerate-limit`: it is a divisor of the refresh rate **[V]**.
2. MangoHud applied → merge `fps_limit=N` into `MANGOHUD_CONFIG` (`,`-list). Top. mangoapp reads it.
3. P, W or C → `DXVK_CONFIG` merge `dxvk.maxFrameRate=N;dxgi.maxFrameRate=N;d3d9.maxFrameRate=N` (`;`-list), plus `VKD3D_FRAME_RATE=N` **[V]**.
4. N with `mangohud` available → wrapper `mangohud` [7], plus `MANGOHUD_CONFIG=no_display,fps_limit=N`.
5. F with the MangoHud extension, or L → env `MANGOHUD=1` + `MANGOHUD_CONFIG=no_display,fps_limit=N` (Vulkan only).
6. Else note: "Frame limit needs Gamescope or MangoHud for this game."
- **S / U:** kind note.

**prefer_dgpu**
- **Availability:**
  - Unavailable when there are fewer than 2 GPUs.
  - Unavailable when the boot_vga GPU is already the best. Note: "This system already renders on the discrete GPU."
  - On the user's machine card1 (Navi 48) is boot_vga, so the option is disabled. `DRI_PRIME=1` would pick the iGPU **[V]**.
- **On, NVIDIA proprietary target:** `__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia __VK_LAYER_NV_optimus=NVIDIA_only`.
- **On, Mesa target:** `DRI_PRIME=pci-DDDD_BB_SS_F` (e.g. `pci-0000_03_00_0`) **[V]** Mesa docs.
- All kinds except S and U, F included. With gamescope, also `--prefer-vk-device vvvv:dddd`. Never `switcherooctl launch` or `DRI_PRIME=1`.

**ntsync**
- **P / C / L:**
  - Off → `PROTON_NO_NTSYNC=1`, when `rc.wineserver_no_ntsync` (string in `files/bin/wineserver`). It is read with atoi in Valve 11, GE10-10+ and cachyos **[V]**.
  - On → nothing, except old GE (`PROTON_USE_NTSYNC ∈ rc.vars` and not `PROTON_NO_NTSYNC`) → `PROTON_USE_NTSYNC=1`.
- **W:** Unavailable. "System Wine uses NTSync automatically and has no switch." Upstream opens `/dev/ntsync` unconditionally **[V]**.
- **N / F:** not applicable (hidden from the report).
- The status line shows `Caps.ntsync`.
- Also: add `PROTON_NO_NTSYNC` to `client_launch_procs.rs` `SYNC_VARS`, so delivery matches the running client **[V]** fact-check.

### 2.3 Upscaling & frame gen
**wine_fsr** (P, C, L)
- Requires `rc.fshack_fsr` (the string `WINE_FULLSCREEN_FSR` in `files/lib/wine/x86_64-unix/win32u.so`).
- On → `WINE_FULLSCREEN_FSR=1`, plus `WINE_FULLSCREEN_FSR_STRENGTH=n` when set.
- Off → `WINE_FULLSCREEN_FSR=0`.
- Suppressed:
  - when Wayland is applied. The Wayland path sets `WINE_DISABLE_FULLSCREEN_HACK=1` **[V]**. Note: "Wine FSR works only on X11; it's off because Wayland native is on."
  - when gamescope is active. Note: "Using Gamescope's upscaler instead of Wine FSR."
- **W:** Unavailable unless that wine's win32u.so has the string.

**fsr4_upgrade**
- **GPU gate:**
  - AMD GC IP 12.x → available.
  - 11.x → available, labelled "experimental on RDNA3".
  - Otherwise Unavailable: "Needs an AMD RDNA3 or newer GPU."
- **Runner gate:**
  - If `PROTON_FSR4_UPGRADE ∈ rc.vars`: On `=1`, Off `=0`.
    - The UI's Default label for cachyos is "Default (on)", because cachyos adds fsr4 by default **[V]**.
    - For GE11 on RDNA3, On is the "extended GPU support" path **[V]**.
  - Else if `rc.contrib_fsr4` (Valve 11): On → `FSR4_UPGRADE=1` (exact "1" only **[V]**). Off → nothing.
  - Else Unavailable: "Not supported by <runner>."
- `PROTON_FSR4_RDNA3_UPGRADE` is never emitted (obsolete).

**dlss_upgrade**
- Needs an NVIDIA GPU and `PROTON_DLSS_UPGRADE ∈ rc.vars` (GE ≥ 10-26, cachyos).
- On `=1`, Off `=0`.
- Never emit `DXVK_NVAPI_DRS_SETTINGS`. If the user's env has it, note: "Your DXVK_NVAPI_DRS_SETTINGS replaces the DLSS-upgrade presets." **[V]**
- If `nvapi`=Off → Suppressed: "DLSS needs NVAPI."

**xess_upgrade:** needs `PROTON_XESS_UPGRADE ∈ rc.vars`. On `=1`, Off `=0`.

**upscaler_indicator**
- On:
  - `PROTON_FSR4_INDICATOR=1` if that variable is in `rc.vars` and FSR4 is effective.
  - `PROTON_DLSS_INDICATOR=1` if that variable is in `rc.vars`.
  - Else, NVIDIA on Valve Proton or W: `DXVK_NVAPI_SET_NGX_DEBUG_OPTIONS=DLSSIndicator=1024,DLSSGIndicator=2,` **[V]** (dxvk-nvapi ≥ 0.9; Valve 11 ships 0.9.2).
- Off: the same variable names with `=0`. On Valve/W use `DLSSIndicator=0,DLSSGIndicator=0,`, because the setting persists in the registry **[V]**.

**lsfg**
- **Available** if a manifest's `layer.name` starts with `VK_LAYER_LSFGVK_frame_generation` (v2; the 32-bit manifest name is `…_x86`) or is `VK_LAYER_LS_frame_generation` (v1 **[U]**).
- **On, profile chosen:** v2 `LSFGVK_PROFILE=<name>`; v1 `LSFG_PROCESS=<that profile's exe>`.
- **On, "Lodestar settings":**
  - v2: `LSFGVK_ENV=1 LSFGVK_MULTIPLIER=m`, plus `LSFGVK_FLOW_SCALE=f`, `LSFGVK_PERFORMANCE_MODE=1`, `LSFGVK_DLL_PATH=<p>` when known.
  - v1: `LSFG_LEGACY=1 LSFG_MULTIPLIER … LSFG_DLL_PATH`.
  - The DLL is looked for at `<steam lib>/steamapps/common/Lossless Scaling/Lossless.dll` when `appmanifest_993090.acf` exists **[U]** folder name. If it is not found, show a warning note, not a disable.
- **Off:** v2 `DISABLE_LSFGVK=1`; v1 `DISABLE_LSFG=1`.
- Always **scoped** env.
- **F:** apply as `--env`. Note: "Needs the Flatpak extension org.freedesktop.Platform.VulkanLayer.lsfgvk and access to ~/.config/lsfg-vk."
- **C:** note "Frame generation also applies to <client>'s window."
- Lodestar never writes `conf.toml`.

### 2.4 Overlay & capture
**mangohud**
- **Gamescope active:** `--mangoapp`. Requires `mangoapp` on PATH, else Unavailable for this combination with a note. Never add the `mangohud` wrapper or `MANGOHUD=1` **[V]**.
  - If the user's env has `MANGOHUD=1`, or `mangohud` is in the chain, note: "MangoHud inside Gamescope isn't supported — remove it from the launch options; the MangoHud option uses mangoapp."
- **N / P / W without gamescope:** wrapper `mangohud` [7]. It covers OpenGL too, and pressure-vessel passes its absolute `$LIB` preload **[V]**.
- **F:** `MANGOHUD=1` as env/`--env`. Needs `org.freedesktop.Platform.VulkanLayer.MangoHud` on the app's runtime branch; else Unavailable with an install hint.
- **C / L:** `MANGOHUD=1` env (Vulkan only). The store clients are on MangoHud's blacklist **[V]**.
- **Off (every env kind):** `DISABLE_MANGOHUD=1`.

**obs_capture**
- **N or W without gamescope, when `obs-gamecapture` is on PATH:** wrapper `obs-gamecapture` [8] (covers GL).
- **Otherwise (P, F, C, L, or anything under gamescope):** env `OBS_VKCAPTURE=1`, scoped. Under umu inside gamescope only the Vulkan path works.
- **Off:** `DISABLE_OBS_VKCAPTURE=1`.
- **F:** needs `org.freedesktop.Platform.VulkanLayer.OBSVkCapture`.

### 2.5 Compatibility
- **nvapi** (P, C, L; shown only if an NVIDIA GPU is present):
  - Auto → nothing.
  - Off → `PROTON_DISABLE_NVAPI=1`.
  - Force → `PROTON_FORCE_NVAPI=1`.
  - `PROTON_ENABLE_NVAPI` is obsolete **[V]**.
  - **W:** Unavailable ("Proton only").
- **sdl_input** (P/C/L): needs `PROTON_PREFER_SDL ∈ rc.vars`. On `=1`, Off `=0`.
- **audio_latency** (N, P, W, F, C, L):
  - On → `PULSE_LATENCY_MSEC=60` (the 60 is community practice **[U]**).
  - Suppressed when the runner is cachyos (winepipewire is the default **[V]**). Note: "proton-cachyos plays audio through PipeWire directly; this fix has no effect."

### 2.6 Advanced
- **proton_log** (P/C/L):
  - `PROTON_LOG=1`, `PROTON_LOG_DIR=<lodestar data dir>/proton-logs/<game id>`.
  - Under umu, `SteamGameId` is often 0 or "default", so logs would collide **[V]**.
  - `SpawnJob::spawn` runs `create_dir_all` for `PROTON_LOG_DIR` before spawning.
  - **W:** Unavailable.
- **hdr_wsi_layer:**
  - Auto = On only when `hdr` is effective On, the `VK_LAYER_hdr_wsi` manifest is installed, the NVIDIA proprietary version is < 595, and gamescope is not active.
  - On → `ENABLE_HDR_WSI=1`. Off → `DISABLE_HDR_WSI=1`.
  - Never with gamescope; the layer README forbids it **[V]**.
- **layers:**
  - On → the manifest's `enable_environment` K=V, exact value **[V]** loader.
  - Off → its `disable_environment` K=`1`. Disable wins over enable **[V]**.
  - Scoped env. Steam layers and layers handled by other options are excluded (§4.2).
  - Layers that have no enable key offer only Off.

### 2.7 Note texts (constants in `game_options.rs`)
- `NOTE_STEAM`: "Not applied — the Steam client starts this game: {list}. In Steam, set the game's launch options to: `{steam_string}`." `steam_string` is §3.4.
- `NOTE_URL`: "Not applied — {handler} starts this game with its own settings: {list}." The handler is "Heroic" for `heroic://`, else the URL scheme.
- `NOTE_LAUNCHER`: "{launcher} starts this game: {env_list} are passed to it but may not reach the game; {wrapper_list} aren't added. Use {launcher}'s own settings{or_run_directly}." For Lutris, add: " (they never reach it while Lutris is already open)". `or_run_directly` = " or \"Run directly\"" when a direct launch exists.
- `NOTE_CLIENT`: "Game options apply only when Lodestar starts {client} itself — close {client} first. Not added for store-client games: {wrapper_list}."
- `NOTE_FLATPAK_PRELOAD`: "{list} can't reach a Flatpak app (Flatpak drops LD_PRELOAD)."
- Only options with an effective value that differs from built-in Default go into `{list}`. The notes are combined, one per kind.

### 2.8 Report
- Add `Resolved.options: Vec<(OptionId, OptionState)>`.
- `OptionState` is `Applied | AppliedImplicitly(&'static str) | Suppressed(String) | NotApplicable | Unavailable(String)`.
- The per-game page shows this as a chip next to each row. The tests assert it.

### 2.9 P2 sidecars (S, U, L, C, and gamemode for F)
- Add `Resolved.sidecars: Vec<Sidecar { program, args, stop: Signal }>`.
- The Launcher starts them when launch_track reports the game Started. It stops them (process group: the stop signal, then SIGKILL after 3 s) on Exited or when Lodestar quits.
- Commands:
  - `gamemoded -r`, stopped with SIGINT **[V]** (pid-less = request and pause).
  - `<powerprofilesctl|tlpctl> launch -p performance -r "Lodestar: <name>" -- sleep infinity`, stopped with SIGTERM. It forwards the signal and releases the profile; the daemon also releases when the holder leaves the bus **[V]**.
  - `systemd-inhibit --what=idle:sleep --who=Lodestar "--why=Playing <name>" sleep infinity`, stopped with SIGTERM.
- Optional: `busctl --user call com.feralinteractive.GameMode /com/feralinteractive/GameMode com.feralinteractive.GameMode RegisterGameByPID ii <lodestar_pid> <game_pid>` for umu inside gamescope **[V]**. Never `gamemoded -r<PID>`, which toggles **[V]**.

---

## 3. Composition order

### 3.1 Env precedence (process env)
1. `E_base`:
   - with a custom command: only the custom command's `VAR=` / `env` words (cenv);
   - otherwise: the source LaunchSpec env.
   - These two are never mixed **[V]**.
2. `E_toggle_top`, emitted in option-list order (§1 order, layers sorted by name):
   - with a custom command: toggles fill only keys that are absent;
   - otherwise: toggles overwrite source keys (`set_env`).
3. `E_user`: the user's options `VAR=` words, via `set_env`, as at launch.rs:760. **The user wins.**
4. List variables (`WINEDLLOVERRIDES`, `DXVK_CONFIG` joined with `;`, `MANGOHUD_CONFIG` joined with `,`) are merged **after** step 3. Lodestar appends only the items whose key (the text before `=`) is not already in the value.
5. **Gamescope active:**
   - Toggle keys marked scoped go into `E_scoped`, not into the process env.
   - User env keys in `{ENABLE_VKBASALT, SDL_VIDEODRIVER, SDL_VIDEO_DRIVER}` are moved from the process env into `E_scoped`, keeping the user's value. Gamescope strips these **[V]**.
6. **Flatpak:** every key that toggles produced or merged is also emitted as `--env=K=V` (the merged value) at `args[run_idx+1..]`, after `--nosocket=wayland`, so it overrides the app's metadata. Keys the user typed and toggles didn't touch stay plain env; it passes through **[V]**.

### 3.2 Chain and classification
- `chain = user_prefix_words ++ [base_program] ++ base_args`. This is after `fold_env_command`.
- `classify` peels leading words:
  - `env` + `VAR=` words;
  - `gamemoderun`, `mangohud`, `obs-gamecapture`, `prime-run`, `game-performance`;
  - `systemd-inhibit` with `--x=y` options, or two-word options from `--what --who --why --mode`;
  - `powerprofilesctl|tlpctl launch … --`;
  - `gamescope … --`.
- It then classifies the core program (§2 kinds; `umu-run`/`proton` by basename; `wine*` excluding `wineserver`/`winetricks`; `flatpak` + `run`).
- **Wine inside a Proton dir** (`<dir>/files/bin/wine*` with `<dir>/proton` present) → W, with that dir's caps. The script-level `PROTON_*` vars are then NotApplicable; `PROTON_NO_NTSYNC` still applies.
- **The first `gamescope` among the peeled words splits the chain:**
  - `chain_pre` = the words before it;
  - `user_gs_flags` = its flags;
  - `chain_post` = the words after its `--`.
  - With no gamescope: `chain_pre = []` and `chain_post = chain`.
- **"Gamescope active"** = our toggle is effectively On, or the chain contains gamescope. In the second case Lodestar adds only the interaction flags (`--mangoapp`, `--hdr-enabled`, `-r`, `--prefer-vk-device`) in front of the user's flags, and none of the size, window or filter flags.
- **Dedupe:** a toggle wrapper is skipped if its basename is among the peeled words. Our gamescope merges into the user's gamescope, so there is only ever one.

### 3.3 Final argv (outer → inner). Only for N, P, W, F.
```
[1]  systemd-inhibit --what=idle:sleep --who=Lodestar --why=Playing <name>    (keep_awake)
[2]  game-performance | powerprofilesctl launch -p performance -r "Lodestar: <name>" --
[3]  <chain_pre>
[4]  gamescope <Lodestar flags §2.1> <gs_extra_args> <user_gs_flags> --
[5]  env K=V …            (E_scoped; omitted when empty or no gamescope — then E_scoped ⊂ process env)
[6]  gamemoderun
[7]  mangohud             (never with gamescope active)
[8]  obs-gamecapture
[9]  <chain_post>         (= the rest of the user's %command% prefix, then the base program and args;
                           flatpak: run [--nosocket=wayland] [--env=…] <rest>)
[10] <user suffix words>
```
- `program` = the first word of the result; `args` = the rest. Rebuild through `finish()`.
- For L and C: only §3.1 env applies. The argv is built exactly as today (launch.rs:763-774).
- The order of wrappers matters for MangoHud under gamescope.

### 3.4 Steam launch-options string
Run the same composer with kind P, caps Unknown, chain `["%command%"]`, and the session wrappers included. Render it as `K=V … wrappers %command%`. For example:
```
DXVK_HDR=1 game-performance gamescope -W 2560 -H 1440 -w 2560 -h 1440 -f --hdr-enabled --mangoapp -- gamemoderun %command%
```

---

## 4. Availability detection

### 4.1 Caps
- One background pass after the window shows.
- Re-run when Settings → Games or a game's options page opens, and on a "Rescan" button.
- No processes at startup, except a cached `gamescope --help` run with a 2 s timeout, reading stderr **[V]**.

```rust
pub struct Caps {
  pub bins: Bins, // Option<PathBuf> each: gamescope, mangohud, mangoapp, gamemoderun, game_performance,
                  // powerprofilesctl, tlpctl, obs_gamecapture, systemd_inhibit  (launch::find_in_path; ignore hits under $APPDIR)
  pub gamescope: Option<GamescopeCaps>, // flags: HashSet<String>, filters: Vec<String>, version: Option<(u32,u32,u32)>;
                                        // cache key (path, mtime, size) in Lodestar cache dir
  pub layers: Vec<LayerManifest>,       // name, description, enable_env: Option<(String,String)>, disable_env: Option<String>, file
  pub lsfg: Option<LsfgCaps>,           // version 1|2, profiles: Vec<(name, active_in)>, dll: Option<PathBuf>
  pub gpus: Vec<Gpu>,                   // card, pci "0000:03:00.0", vendor, device, driver, boot_vga, vram, gc_ip: Option<(u8,u8)>, discrete_tag: bool
  pub nvidia: Option<NvidiaInfo>,       // version from /sys/module/nvidia/version
  pub ntsync: NtsyncState,              // Usable | NotLoaded | Missing
  pub wayland_session: bool, pub nested_gamescope: bool,
  pub hdr_outputs: Vec<String>, pub kde_hdr_enabled: Option<bool>, pub kde_vrr_never: Option<bool>,
  pub flatpak_ext: HashMap<String, Vec<String>>, // extension id -> branches
  pub distro: Distro,                   // Arch | Fedora | Debian | Other  (/etc/os-release ID, ID_LIKE)
  pub preferred_mode: Option<(u32,u32)>,
}
pub struct RunnerCaps { pub family: RunnerFamily /*Valve|Ge|Cachyos|Unknown*/, pub version: String,
  pub vars: HashSet<String>, pub ntsync: bool, pub wineserver_no_ntsync: bool,
  pub winewayland: bool, pub fshack_fsr: bool, pub contrib_fsr4: bool }
```

### 4.2 Checks
| Item | Cheap check |
|---|---|
| gamescope | PATH, plus the `--help` flag set (`--[a-z-]+` tokens, and the filter list from the `-F` line). Version regex `version\s+v?(\d+(\.\d+)+)` after stripping ANSI. Fedora's tarball build prints no number **[V]**, so detection relies on features. |
| mangoapp | PATH (Arch and Fedora ship it in `mangohud` **[V]**). |
| gamemode | `gamemoderun` on PATH. No daemon check: D-Bus activates it **[V]**. |
| performance | `game-performance` && (`powerprofilesctl` \|\| `tlpctl`); else `powerprofilesctl`. |
| keep awake | `systemd-inhibit` on PATH. |
| obs | `obs-gamecapture` on PATH, or a layer starting `VK_LAYER_OBS_vkcapture`. |
| layers | JSON in `$XDG_CONFIG_HOME`/`~/.config`, `$XDG_CONFIG_DIRS`/`/etc/xdg`, `/etc`, `$XDG_DATA_HOME`/`~/.local/share`, `$XDG_DATA_DIRS`/`/usr/local/share:/usr/share`, each + `/vulkan/implicit_layer.d` **[V]**. Match `layer.name`. Generic-list exclusions: `VK_LAYER_MANGOHUD_*`, `VK_LAYER_OBS_vkcapture*`, `VK_LAYER_LSFGVK*`, `VK_LAYER_LS_frame_generation`, `VK_LAYER_FROG_gamescope_wsi*`, `VK_LAYER_hdr_wsi`, `VK_LAYER_VALVE_*`, `VK_LAYER_MESA_device_select`, `VK_LAYER_NV_optimus`. Group 32- and 64-bit entries by their env key. |
| lsfg | Layer as above; `$LSFGVK_CONFIG` or `~/.config/lsfg-vk/conf.toml` `[[profile]] name/active_in`; `appmanifest_993090.acf` in the Steam libraries. |
| GPUs | `/sys/class/drm/card[0-9]*/device/{vendor,device,boot_vga,mem_info_vram_total}`, the driver link basename, `ip_discovery/die/0/GC/0/{major,minor}` (12 = RDNA4 **[V]**, 11 = RDNA3 **[U]** mapping), and the udev tag `G:switcheroo-discrete-gpu` in `/run/udev/data/c226:N`. "Best" GPU = NVIDIA proprietary > discrete tag > largest VRAM. |
| NVIDIA | vendor 0x10de; `nvidia` in `/proc/modules`. |
| NTSync | `access("/dev/ntsync", R_OK)` → Usable. Else if `/lib/modules/$(uname -r)/kernel/drivers/misc/ntsync.ko*` exists → NotLoaded. Else Missing. |
| Wayland | `WAYLAND_DISPLAY` set and `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` is a socket. |
| nested | `GAMESCOPE_WAYLAND_DISPLAY` set, or `XDG_CURRENT_DESKTOP=gamescope`. |
| HDR | EDID CTA block → HDR static metadata with PQ (connected outputs). `~/.config/kwinoutputconfig.json` `highDynamicRange`/`vrrPolicy`, joined by connectorName/edidIdentifier with connected connectors **[V]**. These produce notes only, never a disable. |
| Flatpak exts | `~/.local/share/flatpak/runtime/<ID>/x86_64/<branch>/active` and `/var/lib/flatpak/runtime/…`. App branch from `app/<APPID>/current/active/metadata` `runtime=`. The GNOME → freedesktop branch mapping is **[U]**; if unknown, treat any branch as OK and show a note. |
| monitor | Set by the app: `viewport().monitor_size × ctx.pixels_per_point()` **[V]** (not `native_pixels_per_point`, because of `ui_scale`). Fallback: the first line of `/sys/class/drm/<boot_vga card>-<connected conn>/modes` **[U]** that preferred = current. Manual: global `gs_output`. |
| RunnerCaps | Per Proton dir, cached by `(dir, mtime(proton), mtime(version))`. `vars` = `PROTON_[A-Z0-9_]+` plus `DXVK_[A-Z_]+` names from `proton` and `protonfixes/*.py`. `ntsync` and `wineserver_no_ntsync` from substring search in `files/bin/wineserver` (`/dev/ntsync`, `PROTON_NO_NTSYNC`). `winewayland` = `files/lib/wine/x86_64-unix/winewayland.so` exists. `fshack_fsr` = `WINE_FULLSCREEN_FSR` substring in `win32u.so`. `contrib_fsr4` = `files/contrib/amdxcffx64.dll` or `contrib/amdxcffx64.dll` **[U]** path. `family` from the `version` file ("GE-Proton…" → Ge, contains "cachyos" → Cachyos, otherwise Valve). |

**Pre-probing runners.**
- Every dir under `~/.local/share/Steam/compatibilitytools.d` and `/usr/share/steam/compatibilitytools.d` is probed in the background pass.
- Other dirs are probed synchronously on first resolve, a few ms once.
- umu `PROTONPATH`:
  - an absolute dir → that dir;
  - `GE-Proton` → the newest `GE-Proton*` in compatibilitytools.d **[U]** umu's storage;
  - unset → the newest `UMU-Proton*` (Valve family);
  - anything else → Unknown.

### 4.3 UI for unavailable options
- The row is greyed out, with the reason and **one** install line for `Caps.distro`.
- A per-game override is kept even while the option is unavailable.

| Option | Arch / CachyOS | Fedora | Ubuntu |
|---|---|---|---|
| Gamescope | `gamescope` | `gamescope` | `gamescope` (26.04+; not in 24.04) |
| MangoHud / mangoapp | `mangohud lib32-mangohud` | `mangohud` | `mangohud` |
| GameMode | `gamemode lib32-gamemode` | `gamemode` | `gamemode` |
| Performance profile | CachyOS `cachyos-settings` + `power-profiles-daemon`; Arch `power-profiles-daemon` | tuned-ppd has no powerprofilesctl **[V]**: "install power-profiles-daemon (replaces tuned-ppd)" | `power-profiles-daemon` |
| Keep awake | systemd | systemd | systemd |
| OBS capture | AUR `obs-vkcapture` | `obs-studio-plugin-vkcapture` | build from source / Flatpak OBS plugin |
| lsfg-vk | AUR `lsfg-vk` / `lsfg-vk-bin` | tarball from lsfg-vk.dev | tarball from lsfg-vk.dev |
| NTSync | kernel ≥ 6.14; `ntsync-autoload` (Arch); built in on CachyOS | Fedora 44 `wine-ntsync`, or `sudo modprobe ntsync` | `sudo modprobe ntsync` |
| Proton-fork options | GE-Proton (ProtonUp-Qt) or `proton-cachyos` **[U]** pkg name | GE-Proton | GE-Proton |
| Flatpak MangoHud / OBS / lsfg | `flatpak install flathub org.freedesktop.Platform.VulkanLayer.{MangoHud,OBSVkCapture,lsfgvk}//<branch>` | same | same |

If only the Flatpak gamescope extension exists, the hint is: "Flatpak Gamescope works only inside Flatpak launchers; install gamescope for Lodestar."

---

## 5. Data model

New module `src/game_options.rs`:
```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tri { #[default] Default, On, Off }
impl Tri { pub fn or(self, g: Tri) -> Tri { if self == Tri::Default { g } else { self } } }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all="snake_case")]
pub enum WindowMode { Fullscreen, Borderless, Windowed }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all="snake_case")]
pub enum Res { Auto, Custom { w: u32, h: u32 } }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all="snake_case")]
pub enum InternalRes { Native, Percent(u8), Custom { w: u32, h: u32 } }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all="snake_case")]
pub enum GsFilter { Auto, Linear, Nearest, Fsr, Nis, Sgsr, Pixel }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all="snake_case")]
pub enum GsScaler { Auto, Fit, Integer, Fill, Stretch }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all="snake_case")]
pub enum Nvapi { #[default] Default, Off, Force }

/// Used both as the global defaults (Config) and the per-game overrides (UserEdits).
/// Every field's Default/None = "inherit" (per game) or "built-in default" (global).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameOptions {
  pub wayland: Tri, pub hdr: Tri, pub hdr_force: Tri,
  pub gamescope: Tri, pub gs_window: Option<WindowMode>, pub gs_output: Option<Res>,
  pub gs_internal: Option<InternalRes>, pub gs_filter: Option<GsFilter>, pub gs_sharpness: Option<u8>,
  pub gs_scaler: Option<GsScaler>, pub gs_adaptive_sync: Tri, pub gs_grab_cursor: Tri,
  pub gs_expose_wayland: Tri, pub gs_disable_wsi: Tri, pub gs_extra_args: Option<String>,
  pub gamemode: Tri, pub performance_profile: Tri, pub keep_awake: Tri,
  pub frame_limit: Option<u32>, pub prefer_dgpu: Tri, pub ntsync: Tri,
  pub wine_fsr: Tri, pub wine_fsr_strength: Option<u8>, pub fsr4_upgrade: Tri, pub dlss_upgrade: Tri,
  pub xess_upgrade: Tri, pub upscaler_indicator: Tri,
  pub lsfg: Tri, pub lsfg_profile: Option<String>, pub lsfg_multiplier: Option<u8>,
  pub lsfg_flow_scale: Option<f32>, pub lsfg_performance: Tri,
  pub mangohud: Tri, pub obs_capture: Tri,
  pub nvapi: Nvapi, pub sdl_input: Tri, pub audio_latency: Tri,
  pub proton_log: Tri, pub hdr_wsi_layer: Tri,
  pub layers: std::collections::BTreeMap<String, Tri>,
}
impl GameOptions { pub fn is_default(&self) -> bool { *self == Self::default() } }

/// Fully merged values (per-game.or(global).or(built-in)), plus `per_game_wayland` for the native rule.
pub struct Effective { /* same fields, Tri/values resolved; gs_window default Fullscreen, gs_output Auto,
  gs_internal Native, gs_filter Auto, gs_scaler Auto, lsfg_multiplier 2 */ }
pub fn effective(global: &GameOptions, game: &GameOptions) -> Effective;
```
- An empty `lsfg_profile` string means "Lodestar settings". `gs_extra_args` from the game replaces the global value (it does not concatenate).

**Config:** `pub game_options: GameOptions` in `Config` (config.rs:278). The struct is `#[serde(default)]`, so old configs load.

**UserEdits** (model.rs:392):
```rust
#[serde(default, skip_serializing_if = "GameOptions::is_default")]
pub game_options: GameOptions,
/// Deprecated: migrated into game_options.hdr on load; kept so old edits_json parses.
#[serde(default, skip_serializing_if = "std::ops::Not::not")]
pub hdr: bool,
```
- Do not change the type of `hdr` in place; old rows contain `"hdr":true`, which would then fail to parse.
- Add `impl UserEdits { pub fn migrate(&mut self) { if self.hdr { if self.game_options.hdr == Tri::Default { self.game_options.hdr = Tri::On; } self.hdr = false; } } }`. Call it wherever `edits_json` is deserialised (db.rs load). It is idempotent, and the next save persists it.
- Remove every reader of `edits.hdr`, the edit-dialog checkbox included.
- Behaviour change: the old `HDR_ENV` always added `ENABLE_HDR_WSI=1` and `PROTON_ENABLE_HDR=1`. Now `hdr_wsi_layer` Auto covers the one case that needs it (NVIDIA < 595 without gamescope). Delete `HDR_ENV` and `apply_hdr`, and move their tests over.

**Plumbing**
- `static LAUNCH_ENV: RwLock<Arc<LaunchEnv>>`.
- `pub struct LaunchEnv { pub defaults: GameOptions, pub caps: Arc<Caps>, pub monitor: Option<(u32,u32)> }`.
- `set_launch_env(...)` is called by the app on config load/save, when the caps probe finishes, and when the window's monitor changes.
- The OnceLock `Host` stays unchanged; it can't hold changing defaults **[V]**.
- `resolve(g)` → `resolve_full(g, host(), &launch_env())`.
- `resolve_with(g, h)` → `resolve_full(g, h, &LaunchEnv::none())`. With no toggles, every existing test is unchanged.
- The same pattern applies to `resolve_action[_with]`, `preview` and `preview_action`.
- `resolve_parts(…, toggles: Option<(&Effective, &LaunchEnv, play: bool, name: &str)>)`, called from `resolve_game` and `resolve_user_action`.
- Callers of `resolve` keep their signature: app.rs:205, launch_play.rs:84, edit_app.rs:113, dialogs.rs:265, edit.rs:583, controller/mod.rs:1852, launchtest.rs:55, launch.rs:1748, launch.rs:2153.

**UI**
- Settings → Games shows the global list. Wrapper-type options appear as a checkbox (On / Default); value-type options appear as a three-state control.
- Each game's Launch options page shows the same list with segmented "Default (On|Off) / On / Off" controls, the §2.8 chips, an Advanced fold, and the existing "Will run:" preview.
- Replace the "gamescope -f --" quick-insert in dialogs.rs:317 with the Gamescope option. Saved text keeps working through the §3.2 merge.
- For Steam games, show the §3.4 string with a Copy button.

---

## 6. Conflicts and interactions

| Situation | Rule |
|---|---|
| gamescope + mangohud | `--mangoapp`. Never the `mangohud` wrapper or `MANGOHUD=1`. The MangoHud config vars stay top-level. |
| gamescope + HDR | `--hdr-enabled` + `DXVK_HDR=1`. No `ENABLE_HDR_WSI`, no Wayland driver. If `gs_disable_wsi` or the user's `ENABLE_GAMESCOPE_WSI=0` is set, note "HDR inside Gamescope needs the Gamescope WSI layer." If the gamescope version is < 3.16.27 and the desktop is KDE, note "Gamescope older than 3.16.27 may not show HDR on Plasma 6.8+" **[V]**. |
| gamescope + Wayland | Suppressed unless `gs_expose_wayland`; when exposed, the SDL vars are scoped. |
| HDR without gamescope | Implies Wayland (P/W/C). Valve Proton → note. Desktop HDR off → note "HDR is off for all monitors in System Settings → Display & Monitor." |
| Wayland + Wine FSR / gamescope + Wine FSR | wine_fsr Suppressed. |
| gamescope filter on NVIDIA proprietary | Note "Gamescope upscaling can crash on NVIDIA (gamescope #1662); try Advanced → Disable Gamescope WSI layer." **[V]** issue open. |
| gs_adaptive_sync + KWin `vrrPolicy` Never | Note "Adaptive sync is off in System Settings → Display; set it to Automatic." |
| frame_limit | Exactly one backend (§2.2). Never both `-r` and `--framerate-limit`; they compound **[V]**. |
| NVAPI Off + DLSS upgrade | DLSS Suppressed. |
| DLSS upgrade + user `DXVK_NVAPI_DRS_SETTINGS` | Note (§2.3). |
| game-performance + keep_awake | One inhibitor (§2.2). |
| umu inside gamescope | LD_PRELOAD is cleared **[V]**. gamemode loses per-process effects; OBS GL capture is lost. Report this as a Suppressed detail on obs (GL path only). |
| User's mangohud/gamemoderun/game-performance in the chain | Our copy is skipped (dedupe). A user's mangohud under gamescope gives the warning note. |
| prefer_dgpu + gamescope | `--prefer-vk-device` with the same GPU. |
| ClientHelper | Env only. NTSync consistency via `SYNC_VARS`. Delivery to a running client ignores toggles (`NOTE_CLIENT`). |
| Controller session | Gamescope Suppressed (nested). |
| lsfg / layers / OBS under gamescope | Scoped after `--`, so mangoapp and gamescope don't load them. |
| proton-cachyos | audio_latency Suppressed. fsr4 Default label is "on". |

---

## 7. Tests (src/game_options_tests.rs + additions to launch_tests.rs)

**Fixture:** `LaunchEnv` with fake `Caps`:
- every bin at `/usr/bin/<name>`;
- gamescope flags = all, filters include `sgsr`, version 3.17.0;
- monitor 2560×1440;
- one AMD GPU, GC 12.0, boot_vga;
- NTSync usable, Wayland session;
- runner `/r/ge` = GE11 caps (vars {`PROTON_ENABLE_WAYLAND`, `PROTON_ENABLE_HDR`, `PROTON_FORCE_HDR`, `PROTON_FSR4_UPGRADE`, `PROTON_DLSS_UPGRADE`, `PROTON_XESS_UPGRADE`, `PROTON_PREFER_SDL`, `PROTON_LOG`, `PROTON_DISABLE_NVAPI`, `PROTON_FORCE_NVAPI`}, winewayland, fshack, ntsync + no_ntsync);
- `/r/valve` = UMU-Proton (no fork vars, no winewayland, no fshack);
- `/r/cachy` = cachyos (family Cachyos, `PROTON_FSR4_UPGRADE`, no `PROTON_ENABLE_HDR`).

**umu base:** `umu-run /g/game.exe`, env `[WINEPREFIX=/p, GAMEID=umu-default, PROTONPATH=/r/ge]`. Name "Test".

1. **native_mangohud_gamemode:** `cmd("/opt/g/run",["-x"])`, mangohud On, gamemode On → program `gamemoderun`, args `["mangohud","/opt/g/run","-x"]`, env `[]`.
2. **umu_full_stack_order:** gamescope, mangohud, hdr, gamemode, performance_profile and keep_awake all On →
   - program `systemd-inhibit`;
   - args `["--what=idle:sleep","--who=Lodestar","--why=Playing Test","game-performance","gamescope","-W","2560","-H","1440","-w","2560","-h","1440","-f","--hdr-enabled","--mangoapp","--","gamemoderun","umu-run","/g/game.exe"]`;
   - env = base + `DXVK_HDR=1`, `GAME_PERFORMANCE_SCREENSAVER_ON=1`;
   - no `PROTON_ENABLE_WAYLAND`, no `MANGOHUD`.
3. **user_env_wins:** hdr On, opts `"DXVK_HDR=0 %command%"` → `DXVK_HDR=0` and no other HDR value.
4. **user_gamescope_merged:** opts `"gamescope -f -W 1920 -H 1080 -- %command%"`, mangohud On, gamescope Default → program `gamescope`, args `["--mangoapp","-f","-W","1920","-H","1080","--","umu-run","/g/game.exe"]`.
5. **toggle_and_user_gamescope_single:** gamescope On + the same opts → exactly one `gamescope`; Lodestar's flags come before the user's.
6. **steam_global_defaults_no_error:** Steam appid game, global mangohud and gamemode On → Ok, kind Steam, args unchanged, one note containing "Steam client" and a launch-options string ending in `%command%`.
7. **url_note:** a `heroic://` game with mangohud On → kind OpenUrl, env empty, note mentions Heroic.
8. **flatpak_env_and_drops:** `flatpak run --branch=stable com.ex.Game` with mangohud On (extension present), gamemode On, frame_limit 60, no gamescope →
   - program `flatpak`;
   - args `["run","--env=MANGOHUD=1","--env=MANGOHUD_CONFIG=fps_limit=60","--branch=stable","com.ex.Game"]`;
   - `NOTE_FLATPAK_PRELOAD` mentions GameMode.
   - Note on `MANGOHUD_CONFIG`: plain `fps_limit` here, because MangoHud is applied rather than limiter-only.
9. **flatpak_gamescope_nosocket:** gamescope On → args start `gamescope … -- flatpak run --nosocket=wayland …`.
10. **client_helper_env_only:** `@lodestar --client-launch battlenet …`, gamescope and mangohud On → program is self, args unchanged, env has `MANGOHUD=1`, `NOTE_CLIENT` present, gamescope Suppressed.
11. **lutris_launcher_note:** `lutris lutris:rungameid/3`, gamemode On → no wrapper, note mentions "already open".
12. **wayland_suppressed_in_gamescope:** wayland + gamescope On → no `PROTON_ENABLE_WAYLAND`, Suppressed. Adding `gs_expose_wayland` → `--expose-wayland` + `PROTON_ENABLE_WAYLAND=1`.
13. **hdr_implies_wayland_ge:** hdr On, no gamescope → env `PROTON_ENABLE_WAYLAND=1`, `DXVK_HDR=1`; report `AppliedImplicitly`.
14. **hdr_valve_needs_gamescope:** PROTONPATH=/r/valve, hdr On → `DXVK_HDR=1` + the Valve note; wayland Unavailable.
15. **hdr_off_ge:** → `DXVK_HDR=0`, `DXVK_NO_HDR=1`, `PROTON_ENABLE_HDR=0`.
16. **cachy_fsr4_off:** PROTONPATH=/r/cachy, fsr4 Off → `PROTON_FSR4_UPGRADE=0`; audio_latency On → Suppressed, no `PULSE_LATENCY_MSEC`.
17. **wine_fsr_vs_wayland:** wine_fsr + wayland On → no `WINE_FULLSCREEN_FSR`, Suppressed.
18. **ntsync_off:** P → `PROTON_NO_NTSYNC=1`; plain `wine x.exe` → Unavailable, no env. GE10-9-style caps (vars has only `PROTON_USE_NTSYNC`) with On → `PROTON_USE_NTSYNC=1`.
19. **frame_limit_backends:**
    - gamescope → `-r 60`;
    - P without mangohud → `DXVK_CONFIG=dxvk.maxFrameRate=60;dxgi.maxFrameRate=60;d3d9.maxFrameRate=60` + `VKD3D_FRAME_RATE=60`;
    - native without mangohud toggle → `mangohud` wrapper + `MANGOHUD_CONFIG=no_display,fps_limit=60`;
    - user `DXVK_CONFIG=dxgi.syncInterval=0` → merged `dxgi.syncInterval=0;dxvk.maxFrameRate=60;…`;
    - per-game `Some(0)` overrides a global 60 → nothing.
20. **dedupe_base_wrapper:** source `gamemoderun /opt/g/run` + gamemode On → exactly one `gamemoderun`.
21. **custom_command_env_wins:** custom `"DXVK_HDR=0 umu-run x.exe"` + hdr On → `DXVK_HDR=0`. Source env `PROTON_ENABLE_WAYLAND=1` + wayland Off without a custom command → `=0`.
22. **stripped_user_env_scoped:** opts `"ENABLE_VKBASALT=1 %command%"` + gamescope → `… -- env ENABLE_VKBASALT=1 umu-run …`, and the key is absent from the process env.
23. **lsfg_scoped:** lsfg On with profile "Genshin" + gamescope → `-- env LSFGVK_PROFILE=Genshin gamemoderun?…`. Without gamescope → top-level env. Off → `DISABLE_LSFGVK=1`.
24. **internal_res_percent:** `gs_internal Percent(67)`, filter Auto → `-w 1715 -h 965 -F fsr`. Filter Sgsr with caps lacking sgsr → `-F fsr` + note.
25. **no_monitor:** monitor None, no `preferred_mode` → no `-W/-H/-w/-h`, `-f` present, 720p note.
26. **nested_session:** `caps.nested_gamescope` → no gamescope, Suppressed.
27. **missing_tool:** gamescope bin None → no gamescope words, Unavailable; the launch still resolves.
28. **non_play_action:** a Settings action with gamescope and hdr On → hdr env present, no wrappers.
29. **prefer_dgpu:** two-GPU caps (iGPU boot_vga, dGPU 1002:7550 at 0000:03:00.0) → `DRI_PRIME=pci-0000_03_00_0`, and with gamescope `--prefer-vk-device 1002:7550`. Fixture where the dGPU is boot_vga → Unavailable.
30. **proton_run_rewrite:** `/r/ge/proton run x.exe` + gamescope → `waitforexitandrun`.
31. **steam_string:** composer output for the test-2 options with `%command%`, as in §3.4.
32. **migration:** `edits_json` `{"hdr":true}` → `migrate()` → `game_options.hdr == On`, `hdr == false`, and the serialised JSON has no `"hdr"` key. `{}` and an old Config TOML/JSON without `game_options` load as Default.
33. **effective_merge:** global mangohud On + game Default → On; game Off → Off; global wayland On with a native game → not applied unless set per game.
34. **display_matches:** `display` for test 2 equals the shell-quoted program+args with env first (via `finish`).
35. **sync_vars:** `SYNC_VARS` contains `PROTON_NO_NTSYNC` (client_launch_procs).

---

## Still unverified — check on a real machine
- Wine store clients' games inheriting the client's environment.
- `winex11.drv=d` forcing Wayland on upstream Wine.
- Flatpak `--nosocket=wayland` behaviour under host gamescope.
- How MangoHud combines `MANGOHUD_CONFIG` with a config file, and whether `fps_limit` alone keeps the default HUD.
- umu's GE-Proton storage path.
- The `contrib/amdxcffx64.dll` location in Valve Proton 11.
- The Lossless Scaling DLL folder name.
- The lsfg-vk v1 layer name.
- Whether `active_in` matches Proton exe names.
- The GC 11.x = RDNA3 mapping.
- The sysfs `modes` first line equals the current mode.
- `PULSE_LATENCY_MSEC=60` as a value.
- `DXVK_NO_HDR` on non-fork DXVK (expected to be ignored).
- KDE honouring logind idle inhibitors on 6.8.
- GNOME HDR in nested gamescope.
- The `proton-cachyos` package name.
