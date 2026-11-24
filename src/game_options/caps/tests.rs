//! Caps detection against fake filesystem trees (spec §4). Nothing here touches the real
//! system: every probe runs on `ProbeRoots` pointing into a tempdir, and the only program run
//! is a fake `gamescope` shell script created by the test.
use super::*;
use crate::test_support::{write_exe, write_file};

/// A fake rootfs + home in a tempdir.
struct Fake {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    env: HashMap<String, String>,
}

impl Fake {
    fn new() -> Fake {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let home = dir.path().join("home/u");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        Fake { _dir: dir, root, home, env: HashMap::new() }
    }

    /// Write `content` at `rel` under the root (`"/sys/…"`) or the home (`"~/…"`).
    fn write(&self, rel: &str, content: impl AsRef<[u8]>) -> PathBuf {
        let p = self.path(rel);
        write_file(&p, content);
        p
    }

    fn path(&self, rel: &str) -> PathBuf {
        match rel.strip_prefix("~/") {
            Some(h) => self.home.join(h),
            None => self.root.join(rel.trim_start_matches('/')),
        }
    }

    /// An executable script.
    fn exe(&self, rel: &str, body: &str) -> PathBuf {
        let p = self.path(rel);
        write_exe(&p, format!("#!/bin/sh\n{body}\n"));
        p
    }

    fn set(&mut self, k: &str, v: impl Into<String>) {
        self.env.insert(k.to_string(), v.into());
    }

    fn roots(&self) -> ProbeRoots {
        ProbeRoots {
            root: self.root.clone(),
            home: self.home.clone(),
            env: self.env.clone(),
            cache_dir: Some(self.root.join("cache")),
            gamescope_timeout: Duration::from_secs(5),
        }
    }

    /// A DRM card with a PCI device.
    #[allow(clippy::too_many_arguments)]
    fn card(&self, n: u32, vendor: &str, device: &str, pci: &str, driver: &str, boot_vga: bool, vram: Option<u64>) {
        let d = format!("/sys/class/drm/card{n}");
        self.write(&format!("{d}/dev"), format!("226:{n}\n"));
        self.write(&format!("{d}/device/vendor"), format!("{vendor}\n"));
        self.write(&format!("{d}/device/device"), format!("{device}\n"));
        self.write(&format!("{d}/device/boot_vga"), if boot_vga { "1\n" } else { "0\n" });
        self.write(&format!("{d}/device/uevent"), format!("DRIVER={driver}\nPCI_SLOT_NAME={pci}\n"));
        if let Some(v) = vram {
            self.write(&format!("{d}/device/mem_info_vram_total"), format!("{v}\n"));
        }
    }

    fn gc(&self, n: u32, major: u8, minor: u8) {
        let d = format!("/sys/class/drm/card{n}/device/ip_discovery/die/0/GC/0");
        self.write(&format!("{d}/major"), format!("{major}\n"));
        self.write(&format!("{d}/minor"), format!("{minor}\n"));
    }

    fn connector(&self, card: u32, name: &str, connected: bool, modes: &str) {
        let d = format!("/sys/class/drm/card{card}-{name}");
        self.write(&format!("{d}/status"), if connected { "connected\n" } else { "disconnected\n" });
        self.write(&format!("{d}/modes"), modes);
    }
}

// ------------------------------------------------------------------------------------- bins

#[test]
fn bins_from_path_ignore_appdir() {
    let mut f = Fake::new();
    let appdir = f.path("/tmp/.mount_lodestar");
    f.exe("/tmp/.mount_lodestar/usr/bin/gamescope", "exit 0");
    f.exe("/usr/bin/mangohud", "exit 0");
    f.exe("/usr/bin/mangoapp", "exit 0");
    f.exe("/opt/bin/systemd-inhibit", "exit 0");
    f.write("/usr/bin/gamemoderun", "not executable");
    let path = format!(
        "{}:{}:{}",
        appdir.join("usr/bin").display(),
        f.path("/usr/bin").display(),
        f.path("/opt/bin").display()
    );
    f.set("PATH", path);
    f.set("APPDIR", appdir.display().to_string());
    let caps = probe_with(&f.roots());
    assert_eq!(caps.bins.gamescope, None, "AppImage-internal hit ignored");
    assert_eq!(caps.bins.mangohud, Some(f.path("/usr/bin/mangohud")));
    assert_eq!(caps.bins.mangoapp, Some(f.path("/usr/bin/mangoapp")));
    assert_eq!(caps.bins.systemd_inhibit, Some(f.path("/opt/bin/systemd-inhibit")));
    assert_eq!(caps.bins.gamemoderun, None, "not executable");
    assert_eq!(caps.gamescope, None);
}

// -------------------------------------------------------------------------------- gamescope

const HELP: &str =
    "\x1b[0;34m[gamescope]\x1b[0m \x1b[1mconsole:\x1b[0m gamescope version \x1b[1m3.16.29\x1b[0m (gcc 15.2.1)
usage: gamescope [options...] -- [command...]

Options:
  --help                         show help message
  -W, --output-width             output width
  -H, --output-height            output height
  -w, --nested-width             game width
  -h, --nested-height            game height
  -r, --nested-refresh           game refresh rate (frames per second)
  -f, --fullscreen               make the window fullscreen
  -b, --borderless               make the window borderless
  -F, --filter                   upscaler filter (linear, nearest, fsr, nis, pixel, sgsr)
                                     fsr => AMD FidelityFX(TM) Super Resolution 1.0
                                     nis => NVIDIA Image Scaling v1.0.3
  -S, --scaler                   upscaler type (auto, integer, fit, fill, stretch)
  --sharpness, --fsr-sharpness   upscaler sharpness from 0 (max) to 20 (min)
  --expose-wayland               support wayland clients using xdg-shell
  --mangoapp                     Launch with the mangoapp (mangohud) performance overlay
  --adaptive-sync                Enable adaptive sync if available (variable rate refresh)
  --hdr-enabled                  enable HDR output (needs Gamescope WSI layer enabled for support from clients)
  --prefer-vk-device             prefer Vulkan device for compositing (ex: 1002:7300)
  --force-grab-cursor            always use relative mouse mode instead of flipping
";

#[test]
fn help_parsing() {
    let g = parse_gamescope_help(HELP);
    assert_eq!(g.version, Some((3, 16, 29)));
    for f in [
        "--help",
        "--output-width",
        "--nested-refresh",
        "--filter",
        "--sharpness",
        "--fsr-sharpness",
        "--expose-wayland",
        "--mangoapp",
        "--adaptive-sync",
        "--hdr-enabled",
        "--prefer-vk-device",
        "--force-grab-cursor",
        "-W",
        "-H",
        "-w",
        "-h",
        "-r",
        "-f",
        "-b",
        "-F",
        "-S",
    ] {
        assert!(g.flags.contains(f), "{f} missing from {:?}", g.flags);
    }
    assert!(!g.flags.contains("--"), "bare -- is not a flag");
    assert!(!g.flags.contains("-1"));
    assert_eq!(g.filters, ["linear", "nearest", "fsr", "nis", "pixel", "sgsr"]);
}

#[test]
fn help_version_variants() {
    assert_eq!(help_version("gamescope version v3.15.9-12-gabcdef"), Some((3, 15, 9)));
    assert_eq!(help_version("gamescope version 3.16 (gcc)"), Some((3, 16, 0)));
    // Fedora's tarball build prints no number.
    assert_eq!(help_version("gamescope version  (gcc 15)"), None);
    assert_eq!(help_version("no versioning here 1.2"), None);
    assert_eq!(strip_ansi("\x1b[1;31mred\x1b[0m plain"), "red plain");
    // Older help without sgsr: only the parenthesised list.
    let old = parse_gamescope_help(
        "  -F, --filter      upscaler filter (linear, nearest, fsr, nis, pixel)\n  -S, --scaler x\n",
    );
    assert_eq!(old.filters, ["linear", "nearest", "fsr", "nis", "pixel"]);
    assert_eq!(old.version, None);
}

#[test]
fn gamescope_help_runs_once_then_cached() {
    let mut f = Fake::new();
    let marker = f.path("/runs");
    let help_file = f.write("/help.txt", HELP);
    f.exe(
        "/usr/bin/gamescope",
        &format!("echo run >> '{}'\ncat '{}' >&2\nexit 1", marker.display(), help_file.display()),
    );
    f.set("PATH", f.path("/usr/bin").display().to_string());
    let roots = f.roots();
    let first = probe_with(&roots).gamescope.expect("help parsed");
    assert_eq!(first.version, Some((3, 16, 29)));
    assert!(first.flags.contains("--mangoapp"));
    assert!(roots.cache_dir.as_ref().unwrap().join(GAMESCOPE_CACHE_FILE).is_file());
    let second = probe_with(&roots).gamescope.expect("cached");
    assert_eq!(first, second);
    let runs = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(runs.lines().count(), 1, "second probe read the cache");
}

#[test]
fn gamescope_help_cache_invalidated_by_size() {
    let mut f = Fake::new();
    let roots_cache = f.path("/cache");
    f.exe("/usr/bin/gamescope", "echo 'gamescope version 3.14.0' >&2\necho '  --mangoapp  x' >&2");
    f.set("PATH", f.path("/usr/bin").display().to_string());
    assert_eq!(probe_with(&f.roots()).gamescope.unwrap().version, Some((3, 14, 0)));
    f.exe("/usr/bin/gamescope", "echo 'gamescope version 3.16.1 (new build)' >&2\necho '  --mangoapp  x' >&2");
    assert_eq!(probe_with(&f.roots()).gamescope.unwrap().version, Some((3, 16, 1)));
    assert!(roots_cache.join(GAMESCOPE_CACHE_FILE).is_file());
}

#[test]
fn gamescope_help_timeout() {
    let mut f = Fake::new();
    f.exe("/usr/bin/gamescope", "sleep 5");
    f.set("PATH", f.path("/usr/bin").display().to_string());
    let mut roots = f.roots();
    roots.gamescope_timeout = Duration::from_millis(300);
    let t = Instant::now();
    let caps = probe_with(&roots);
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert!(caps.bins.gamescope.is_some());
    assert_eq!(caps.gamescope, None);
    assert!(!roots.cache_dir.unwrap().join(GAMESCOPE_CACHE_FILE).exists(), "failures aren't cached");
}

// ----------------------------------------------------------------------------------- layers

fn layer_json(name: &str, enable: Option<(&str, &str)>, disable: &str, lib: &str) -> String {
    let enable = enable.map(|(k, v)| format!(r#","enable_environment":{{"{k}":"{v}"}}"#)).unwrap_or_default();
    format!(
        r#"{{"file_format_version":"1.0.0","layer":{{"name":"{name}","type":"GLOBAL","library_path":"{lib}","description":"{name} layer"{enable},"disable_environment":{{"{disable}":"1"}}}}}}"#
    )
}

#[test]
fn layers_from_all_xdg_dirs_grouped() {
    let mut f = Fake::new();
    // ~/.config (default XDG_CONFIG_HOME)
    f.write(
        "~/.config/vulkan/implicit_layer.d/vkBasalt.json",
        layer_json(
            "VK_LAYER_VKBASALT_post_processing",
            Some(("ENABLE_VKBASALT", "1")),
            "DISABLE_VKBASALT",
            "libvkbasalt.so",
        ),
    );
    // /etc (sysconfdir)
    f.write(
        "/etc/vulkan/implicit_layer.d/afmf.json",
        layer_json("VK_LAYER_AMD_afmf", Some(("ENABLE_AFMF", "yes")), "DISABLE_AFMF", "/usr/lib/libafmf.so"),
    );
    // XDG_DATA_DIRS: 32- and 64-bit MangoHud, grouped by env key; 64-bit kept.
    let share = f.path("/opt/share");
    f.write(
        "/opt/share/vulkan/implicit_layer.d/MangoHud.x86.json",
        layer_json(
            "VK_LAYER_MANGOHUD_overlay_x86",
            Some(("MANGOHUD", "1")),
            "DISABLE_MANGOHUD",
            "/usr/lib32/libMangoHud.so",
        ),
    );
    f.write(
        "/opt/share/vulkan/implicit_layer.d/MangoHud.x86_64.json",
        layer_json(
            "VK_LAYER_MANGOHUD_overlay_x86_64",
            Some(("MANGOHUD", "1")),
            "DISABLE_MANGOHUD",
            "/usr/lib/libMangoHud.so",
        ),
    );
    // "layers" array form, a disable-only layer, and junk.
    f.write(
        "/opt/share/vulkan/implicit_layer.d/multi.json",
        r#"{"file_format_version":"1.2.0","layers":[
            {"name":"VK_LAYER_MAKO_one","description":"Mako","disable_environment":{"DISABLE_MAKO":"1"}},
            {"name":"VK_LAYER_hdr_wsi","enable_environment":{"ENABLE_HDR_WSI":"1"},"disable_environment":{"DISABLE_HDR_WSI":"1"}}]}"#,
    );
    f.write("/opt/share/vulkan/implicit_layer.d/broken.json", "{ not json");
    f.write("/opt/share/vulkan/implicit_layer.d/readme.txt", "ignored");
    // Default /usr/share is NOT searched once XDG_DATA_DIRS is set.
    f.write(
        "/usr/share/vulkan/implicit_layer.d/hidden.json",
        layer_json("VK_LAYER_hidden", None, "DISABLE_HIDDEN", "x.so"),
    );
    f.set("XDG_DATA_DIRS", share.display().to_string());
    let caps = probe_with(&f.roots());
    let names: Vec<&str> = caps.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "VK_LAYER_VKBASALT_post_processing",
            "VK_LAYER_AMD_afmf",
            "VK_LAYER_MANGOHUD_overlay_x86_64",
            "VK_LAYER_MAKO_one",
            "VK_LAYER_hdr_wsi"
        ]
    );
    let afmf = &caps.layers[1];
    assert_eq!(afmf.enable_env, Some(("ENABLE_AFMF".into(), "yes".into())), "exact value");
    assert_eq!(afmf.disable_env.as_deref(), Some("DISABLE_AFMF"));
    assert_eq!(afmf.description, "VK_LAYER_AMD_afmf layer");
    assert_eq!(afmf.file, f.path("/etc/vulkan/implicit_layer.d/afmf.json"));
    assert_eq!(caps.layers[3].enable_env, None, "disable-only layer");
    let generic: Vec<&str> = names.iter().copied().filter(|n| is_generic_layer(n)).collect();
    assert_eq!(generic, ["VK_LAYER_VKBASALT_post_processing", "VK_LAYER_AMD_afmf", "VK_LAYER_MAKO_one"]);
}

#[test]
fn layer_32bit_first_is_replaced_by_64bit() {
    let mut f = Fake::new();
    f.write(
        "/d/vulkan/implicit_layer.d/a_obs32.json",
        layer_json(
            "VK_LAYER_OBS_vkcapture_32",
            Some(("OBS_VKCAPTURE", "1")),
            "DISABLE_OBS_VKCAPTURE",
            "/usr/lib32/libobs.so",
        ),
    );
    f.write(
        "/d/vulkan/implicit_layer.d/b_obs64.json",
        layer_json(
            "VK_LAYER_OBS_vkcapture_64",
            Some(("OBS_VKCAPTURE", "1")),
            "DISABLE_OBS_VKCAPTURE",
            "/usr/lib/libobs.so",
        ),
    );
    f.set("XDG_DATA_DIRS", f.path("/d").display().to_string());
    let caps = probe_with(&f.roots());
    assert_eq!(caps.layers.len(), 1);
    assert_eq!(caps.layers[0].name, "VK_LAYER_OBS_vkcapture_64");
}

// ------------------------------------------------------------------------------------- lsfg

const LSFG_CONF: &str = r#"version = 2

[global]
allow_fp16 = true

[[profile]]
active_in = [ "vkcube", "vkcubepp" ]
multiplier = 4
name = "4x FG"

[[profile]]
active_in = "GenshinImpact.exe"
name = "Genshin"

[[profile]]
multiplier = 2
"#;

#[test]
fn lsfg_v2_profiles_and_dll() {
    let mut f = Fake::new();
    f.write(
        "~/.local/share/vulkan/implicit_layer.d/VkLayer_LSFGVK_frame_generation.json",
        layer_json("VK_LAYER_LSFGVK_frame_generation", None, "DISABLE_LSFGVK", "liblsfg-vk.so"),
    );
    f.write("~/.config/lsfg-vk/conf.toml", LSFG_CONF);
    // Steam root with a second library holding Lossless Scaling.
    let lib2 = f.path("/games/SteamLibrary");
    f.write("~/.local/share/Steam/steamapps/appmanifest_10.acf", "\"AppState\" { \"appid\" \"10\" }");
    f.write(
        "~/.local/share/Steam/steamapps/libraryfolders.vdf",
        format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
            f.path("~/.local/share/Steam").display(),
            lib2.display()
        ),
    );
    f.write(
        "/games/SteamLibrary/steamapps/appmanifest_993090.acf",
        "\"AppState\"\n{\n \"appid\" \"993090\"\n \"name\" \"Lossless Scaling\"\n \"installdir\" \"Lossless Scaling\"\n \"StateFlags\" \"4\"\n}\n",
    );
    let dll = f.write("/games/SteamLibrary/steamapps/common/Lossless Scaling/Lossless.dll", "MZ");
    f.set("PATH", "");
    let lsfg = probe_with(&f.roots()).lsfg.expect("lsfg layer present");
    assert_eq!(lsfg.version, 2);
    assert_eq!(
        lsfg.profiles,
        vec![
            ("4x FG".to_string(), vec!["vkcube".to_string(), "vkcubepp".to_string()]),
            ("Genshin".to_string(), vec!["GenshinImpact.exe".to_string()]),
        ],
        "nameless profile skipped"
    );
    assert_eq!(lsfg.dll.map(|d| d.canonicalize().unwrap()), Some(dll.canonicalize().unwrap()));
}

#[test]
fn lsfg_v1_and_config_override() {
    let mut f = Fake::new();
    f.write(
        "/usr/share/vulkan/implicit_layer.d/lsfg.json",
        layer_json("VK_LAYER_LS_frame_generation", None, "DISABLE_LSFG", "liblsfg.so"),
    );
    let dll = f.write("/opt/Lossless.dll", "MZ");
    let conf = f.write(
        "/elsewhere/lsfg.toml",
        format!("version = 1\n[global]\ndll = \"{}\"\n[[game]]\nexe = \"Game.exe\"\nmultiplier = 3\n", dll.display()),
    );
    f.set("LSFGVK_CONFIG", conf.display().to_string());
    let lsfg = probe_with(&f.roots()).lsfg.unwrap();
    assert_eq!(lsfg.version, 1);
    assert_eq!(lsfg.profiles, vec![("Game.exe".to_string(), vec!["Game.exe".to_string()])]);
    assert_eq!(lsfg.dll, Some(dll));
}

#[test]
fn no_lsfg_layer_no_caps() {
    let f = Fake::new();
    f.write("~/.config/lsfg-vk/conf.toml", LSFG_CONF);
    assert_eq!(probe_with(&f.roots()).lsfg, None);
    assert_eq!(parse_lsfg_conf("[[profile"), LsfgConf::default());
}

// ------------------------------------------------------------------------------------- GPUs

#[test]
fn single_amd_rdna4_boot_vga() {
    let f = Fake::new();
    f.card(1, "0x1002", "0x7550", "0000:03:00.0", "amdgpu", true, Some(17_095_983_104));
    f.gc(1, 12, 0);
    f.connector(1, "DP-1", false, "");
    f.connector(1, "DP-2", true, "2560x1440\n1920x1080\n");
    f.write("/sys/class/drm/renderD128/device/vendor", "0x1002\n");
    f.write("/sys/class/drm/version", "drm 1.1.0\n");
    let caps = probe_with(&f.roots());
    assert_eq!(caps.gpus.len(), 1);
    let g = &caps.gpus[0];
    assert_eq!(
        g,
        &Gpu {
            card: "card1".into(),
            pci: "0000:03:00.0".into(),
            vendor: VENDOR_AMD,
            device: 0x7550,
            driver: "amdgpu".into(),
            boot_vga: true,
            vram: Some(17_095_983_104),
            gc_ip: Some((12, 0)),
            discrete_tag: false,
        }
    );
    assert_eq!(caps.preferred_mode, Some((2560, 1440)));
    assert_eq!(best_gpu(&caps.gpus).map(|g| g.card.as_str()), Some("card1"));
}

#[test]
fn hybrid_igpu_dgpu() {
    let f = Fake::new();
    // Intel iGPU drives the panel; AMD dGPU tagged by switcheroo; smaller VRAM wins only on tags.
    f.card(0, "0x8086", "0xa7a0", "0000:00:02.0", "i915", true, None);
    f.card(1, "0x1002", "0x7480", "0000:03:00.0", "amdgpu", false, Some(8 << 30));
    f.gc(1, 11, 0);
    f.write("/run/udev/data/c226:1", "E:ID_PATH=pci-0000:03:00.0\nG:uaccess\nG:switcheroo-discrete-gpu\n");
    f.write("/run/udev/data/c226:0", "G:uaccess\n");
    f.connector(0, "eDP-1", true, "1920x1200\n");
    f.connector(1, "HDMI-A-1", true, "3840x2160\n");
    let caps = probe_with(&f.roots());
    assert_eq!(caps.gpus.len(), 2);
    assert!(caps.gpus[0].boot_vga && !caps.gpus[0].discrete_tag);
    assert!(caps.gpus[1].discrete_tag && !caps.gpus[1].boot_vga);
    assert_eq!(caps.gpus[1].gc_ip, Some((11, 0)));
    assert_eq!(best_gpu(&caps.gpus).map(|g| g.pci.as_str()), Some("0000:03:00.0"));
    assert_eq!(caps.preferred_mode, Some((1920, 1200)), "boot_vga card's output first");
}

#[test]
fn best_gpu_ranking() {
    let gpu = |card: &str, vendor, driver: &str, tag, vram| Gpu {
        card: card.into(),
        vendor,
        driver: driver.into(),
        discrete_tag: tag,
        vram,
        ..Default::default()
    };
    let amd_big = gpu("card0", VENDOR_AMD, "amdgpu", false, Some(16 << 30));
    let nv = gpu("card1", VENDOR_NVIDIA, "nvidia", false, None);
    let nouveau = gpu("card2", VENDOR_NVIDIA, "nouveau", true, None);
    assert_eq!(best_gpu(&[amd_big.clone(), nv.clone()]).unwrap().card, "card1", "NVIDIA proprietary first");
    assert_eq!(best_gpu(&[amd_big.clone(), nouveau.clone()]).unwrap().card, "card2", "discrete tag next");
    let amd_small = gpu("card3", VENDOR_AMD, "amdgpu", false, Some(2 << 30));
    assert_eq!(best_gpu(&[amd_small, amd_big]).unwrap().card, "card0", "largest VRAM");
    assert!(best_gpu(&[]).is_none());
}

#[test]
fn gpu_pci_from_device_symlink() {
    let f = Fake::new();
    let dev = f.path("/sys/devices/pci0000:00/0000:01:00.0");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::write(dev.join("vendor"), "0x10de\n").unwrap();
    std::fs::write(dev.join("device"), "0x2684\n").unwrap();
    std::fs::create_dir_all(f.path("/sys/bus/pci/drivers/nvidia")).unwrap();
    std::os::unix::fs::symlink(f.path("/sys/bus/pci/drivers/nvidia"), dev.join("driver")).unwrap();
    std::fs::create_dir_all(f.path("/sys/class/drm/card0")).unwrap();
    std::os::unix::fs::symlink(&dev, f.path("/sys/class/drm/card0/device")).unwrap();
    f.write("/sys/module/nvidia/version", "580.95.05\n");
    let caps = probe_with(&f.roots());
    assert_eq!(caps.gpus[0].pci, "0000:01:00.0");
    assert_eq!(caps.gpus[0].driver, "nvidia");
    assert_eq!(caps.gpus[0].vendor, VENDOR_NVIDIA);
    assert_eq!(caps.nvidia_version.as_deref(), Some("580.95.05"));
}

// ------------------------------------------------------------------------------ system state

#[test]
fn ntsync_states() {
    let f = Fake::new();
    assert_eq!(probe_with(&f.roots()).ntsync, NtsyncState::Missing);
    f.write("/proc/sys/kernel/osrelease", "6.18.1-arch1-1\n");
    f.write("/usr/lib/modules/6.18.1-arch1-1/kernel/drivers/misc/ntsync.ko.zst", "");
    assert_eq!(probe_with(&f.roots()).ntsync, NtsyncState::NotLoaded);
    f.write("/dev/ntsync", "");
    assert_eq!(probe_with(&f.roots()).ntsync, NtsyncState::Usable);
}

#[test]
fn wayland_and_nested() {
    let mut f = Fake::new();
    // The tempdir itself (short path: socket addresses are limited to 108 bytes).
    let run = f._dir.path().to_path_buf();
    f.set("XDG_RUNTIME_DIR", run.display().to_string());
    f.set("WAYLAND_DISPLAY", "wayland-0");
    assert!(!probe_with(&f.roots()).wayland_session, "no socket yet");
    std::fs::write(run.join("wayland-0"), "").unwrap();
    assert!(!probe_with(&f.roots()).wayland_session, "a plain file isn't a socket");
    std::fs::remove_file(run.join("wayland-0")).unwrap();
    let _l = std::os::unix::net::UnixListener::bind(run.join("wayland-0")).unwrap();
    let caps = probe_with(&f.roots());
    assert!(caps.wayland_session);
    assert!(!caps.nested_gamescope);
    f.set("XDG_CURRENT_DESKTOP", "gamescope");
    assert!(probe_with(&f.roots()).nested_gamescope);
    f.set("XDG_CURRENT_DESKTOP", "KDE");
    f.set("GAMESCOPE_WAYLAND_DISPLAY", "gamescope-0");
    assert!(probe_with(&f.roots()).nested_gamescope);
}

/// A minimal EDID base block for KWin's identifier "AOC 45862 219 14 2024".
fn edid(serial: u32) -> Vec<u8> {
    let mut e = vec![0u8; 128];
    e[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
    let m: u16 = (1 << 10) | (15 << 5) | 3; // A O C
    e[8..10].copy_from_slice(&m.to_be_bytes());
    e[10..12].copy_from_slice(&45862u16.to_le_bytes());
    e[12..16].copy_from_slice(&serial.to_le_bytes());
    e[16] = 14;
    e[17] = (2024 - 1990) as u8;
    e
}

#[test]
fn kde_hdr_vrr_notes() {
    let f = Fake::new();
    f.card(0, "0x1002", "0x7550", "0000:03:00.0", "amdgpu", true, None);
    f.connector(0, "DP-1", true, "2560x1440\n");
    f.connector(0, "DP-2", true, "1920x1080\n");
    f.connector(0, "HDMI-A-1", false, "");
    f.write("/sys/class/drm/card0-DP-1/edid", edid(219));
    let kwin = r#"[
      {"name":"outputs","data":[
        {"connectorName":"DP-1","edidIdentifier":"AOC 45862 111 14 2024 0","highDynamicRange":false,"vrrPolicy":"Automatic"},
        {"connectorName":"DP-1","edidIdentifier":"AOC 45862 219 14 2024 0","highDynamicRange":true,"vrrPolicy":"Never"},
        {"connectorName":"DP-2","edidIdentifier":"X 1 2 3 2020 0","highDynamicRange":false,"vrrPolicy":"Never"},
        {"connectorName":"HDMI-A-1","highDynamicRange":true,"vrrPolicy":"Automatic"}]},
      {"name":"setups","data":[{"outputs":[]}]}
    ]"#;
    f.write("~/.config/kwinoutputconfig.json", kwin);
    let caps = probe_with(&f.roots());
    assert_eq!(caps.kde_hdr_enabled, Some(true), "DP-1 matched by EDID; HDMI-A-1 isn't connected");
    assert_eq!(caps.kde_vrr_never, Some(true));
    assert_eq!(edid_identifier(&edid(219)).as_deref(), Some("AOC 45862 219 14 2024"));
    f.write("~/.config/kwinoutputconfig.json", "[]");
    let caps = probe_with(&f.roots());
    assert_eq!((caps.kde_hdr_enabled, caps.kde_vrr_never), (None, None));
}

#[test]
fn kde_edid_match_is_whole_word() {
    let f = Fake::new();
    f.card(0, "0x1002", "0x7550", "0000:03:00.0", "amdgpu", true, None);
    f.connector(0, "DP-1", true, "2560x1440\n");
    f.write("/sys/class/drm/card0-DP-1/edid", edid(21));
    // Serial 21 must not match the entry for serial 219 (a plain prefix test would).
    let kwin = r#"[{"name":"outputs","data":[
        {"connectorName":"DP-1","edidIdentifier":"AOC 45862 219 14 2024 0","highDynamicRange":true,"vrrPolicy":"Never"},
        {"connectorName":"DP-1","edidIdentifier":"AOC 45862 21 14 2024 0","highDynamicRange":false,"vrrPolicy":"Automatic"}]}]"#;
    f.write("~/.config/kwinoutputconfig.json", kwin);
    let caps = probe_with(&f.roots());
    assert_eq!((caps.kde_hdr_enabled, caps.kde_vrr_never), (Some(false), Some(false)));
    assert!(edid_id_matches("AOC 45862 21 14 2024", "AOC 45862 21 14 2024"));
    assert!(!edid_id_matches("AOC 45862 219 14 2024 0", "AOC 45862 21"));
}

/// `edid(serial)` plus one CTA-861 extension block; `eotf` = the HDR static metadata EOTF byte
/// (None = no HDR block, only an audio block).
fn edid_with_cta(eotf: Option<u8>) -> Vec<u8> {
    let mut e = edid(1);
    e[126] = 1; // one extension
    let mut cta = vec![0u8; 128];
    cta[0] = 0x02;
    cta[1] = 3;
    let mut blocks = vec![0x23, 0x09, 0x07, 0x07]; // audio data block, 3 bytes
    if let Some(eotf) = eotf {
        blocks.extend_from_slice(&[0xe3, 0x06, eotf, 0x01]); // extended tag 6, 3 bytes
    }
    cta[4..4 + blocks.len()].copy_from_slice(&blocks);
    cta[2] = (4 + blocks.len()) as u8;
    e.extend_from_slice(&cta);
    e
}

#[test]
fn hdr_outputs_from_edid_pq() {
    assert!(edid_has_pq(&edid_with_cta(Some(0b0000_0101))), "SDR + PQ");
    assert!(!edid_has_pq(&edid_with_cta(Some(0b0000_1001))), "SDR + HLG only");
    assert!(!edid_has_pq(&edid_with_cta(None)));
    assert!(!edid_has_pq(&edid(1)), "base block only");
    assert!(!edid_has_pq(&[0u8; 7]), "truncated");
    // A block whose length byte runs past the end doesn't panic.
    let mut bad = edid_with_cta(None);
    bad[128 + 4] = 0xff;
    bad[128 + 2] = 127;
    assert!(!edid_has_pq(&bad));
    let f = Fake::new();
    f.card(1, "0x1002", "0x7550", "0000:03:00.0", "amdgpu", true, None);
    f.connector(1, "DP-2", true, "2560x1440\n");
    f.connector(1, "DP-3", true, "1920x1080\n");
    f.connector(1, "HDMI-A-1", false, "");
    f.write("/sys/class/drm/card1-DP-2/edid", edid_with_cta(Some(0b101)));
    f.write("/sys/class/drm/card1-DP-3/edid", edid_with_cta(None));
    f.write("/sys/class/drm/card1-HDMI-A-1/edid", edid_with_cta(Some(0b101)));
    assert_eq!(pq_outputs(&find_connectors(&f.roots())), ["DP-2"], "connected PQ outputs only");
}

#[test]
fn nvidia_version_from_proc_fallback() {
    let f = Fake::new();
    f.write(
        "/proc/driver/nvidia/version",
        "NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  580.95.05  Release Build  (dvs-builder@U22)\nGCC version:  gcc version 14.2.0\n",
    );
    assert_eq!(probe_with(&f.roots()).nvidia_version.as_deref(), Some("580.95.05"));
    assert_eq!(
        parse_nvrm_version("NVRM version: NVIDIA UNIX x86_64 Kernel Module  595.12  Mon"),
        Some("595.12".into())
    );
    assert_eq!(parse_nvrm_version("garbage x86_64"), None);
}

#[test]
fn flatpak_extensions_and_app_runtime() {
    let f = Fake::new();
    let ext = "org.freedesktop.Platform.VulkanLayer.MangoHud";
    f.write(&format!("~/.local/share/flatpak/runtime/{ext}/x86_64/24.08/active/metadata"), "");
    f.write(&format!("/var/lib/flatpak/runtime/{ext}/x86_64/25.08/active/metadata"), "");
    f.write(&format!("/var/lib/flatpak/runtime/{ext}/x86_64/23.08/stale"), "");
    f.write("/var/lib/flatpak/runtime/org.freedesktop.Platform/x86_64/25.08/active/metadata", "");
    f.write(
        "/var/lib/flatpak/app/com.ex.Game/current/active/metadata",
        "[Application]\nname=com.ex.Game\nruntime=org.freedesktop.Platform/x86_64/25.08\nsdk=x\n",
    );
    let caps = probe_with(&f.roots());
    assert_eq!(caps.flatpak_ext.len(), 1, "only VulkanLayer extensions");
    assert_eq!(caps.flatpak_ext[ext], ["24.08", "25.08"]);
    assert_eq!(
        flatpak_app_runtime_with(&f.roots(), "com.ex.Game"),
        Some(("org.freedesktop.Platform".to_string(), "25.08".to_string()))
    );
    assert_eq!(flatpak_app_runtime_with(&f.roots(), "com.missing"), None);
    assert_eq!(flatpak_app_runtime_with(&f.roots(), "../x"), None);
}

#[test]
fn distro_detection() {
    assert_eq!(parse_os_release("NAME=\"CachyOS Linux\"\nID=cachyos\nID_LIKE=arch\n"), Distro::Arch);
    assert_eq!(parse_os_release("ID=arch\n"), Distro::Arch);
    assert_eq!(parse_os_release("ID=fedora\nVERSION_ID=43\n"), Distro::Fedora);
    assert_eq!(parse_os_release("ID=bazzite\nID_LIKE=\"fedora\"\n"), Distro::Fedora);
    assert_eq!(parse_os_release("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"), Distro::Debian);
    assert_eq!(parse_os_release("ID=ubuntu\nID_LIKE=debian\n"), Distro::Debian);
    assert_eq!(parse_os_release("ID=gentoo\n"), Distro::Other);
    let f = Fake::new();
    assert_eq!(probe_with(&f.roots()).distro, Distro::Other);
    f.write("/usr/lib/os-release", "ID=fedora\n");
    assert_eq!(probe_with(&f.roots()).distro, Distro::Fedora);
    f.write("/etc/os-release", "ID=cachyos\n");
    assert_eq!(probe_with(&f.roots()).distro, Distro::Arch);
}

#[test]
fn preferred_mode_fallbacks() {
    assert_eq!(parse_mode("1920x1080i"), Some((1920, 1080)));
    assert_eq!(parse_mode("garbage"), None);
    let f = Fake::new();
    assert_eq!(probe_with(&f.roots()).preferred_mode, None);
    // No boot_vga card: any connected output.
    f.card(0, "0x1002", "0x7550", "0000:03:00.0", "amdgpu", false, None);
    f.connector(0, "DP-3", true, "3440x1440\n");
    assert_eq!(probe_with(&f.roots()).preferred_mode, Some((3440, 1440)));
}

// ---------------------------------------------------------------------------------- runners

/// A fake Proton dir. `wineserver` / `win32u.so` get filler so the strings sit past a chunk
/// boundary.
fn proton_dir(
    f: &Fake,
    rel: &str,
    version: &str,
    script: &str,
    wineserver: &[&str],
    fsr: bool,
    wayland: bool,
) -> PathBuf {
    let dir = f.path(rel);
    f.write(&format!("{rel}/version"), format!("1793687165 {version}\n"));
    f.write(&format!("{rel}/proton"), script);
    let mut ws = vec![0u8; CHUNK - 5];
    for s in wineserver {
        ws.extend_from_slice(s.as_bytes());
        ws.push(0);
    }
    f.write(&format!("{rel}/files/bin/wineserver"), ws);
    let mut w32 = vec![0x7fu8; CHUNK * 2 - 7];
    if fsr {
        w32.extend_from_slice(b"WINE_FULLSCREEN_FSR_STRENGTH\0");
    }
    w32.extend_from_slice(&[0u8; 1000]);
    f.write(&format!("{rel}/files/lib/wine/x86_64-unix/win32u.so"), w32);
    if wayland {
        f.write(&format!("{rel}/files/lib/wine/x86_64-unix/winewayland.so"), "ELF");
    }
    dir
}

const GE_SCRIPT: &str = r#"#!/usr/bin/env python3
        if "PROTON_ENABLE_WAYLAND" in self.env and nonzero(self.env["PROTON_ENABLE_WAYLAND"]):
            self.env["WINE_DISABLE_FULLSCREEN_HACK"] = "1"
        self.check_environment("PROTON_ENABLE_HDR", "hdr")
        self.check_environment("PROTON_FORCE_HDR", "forcehdr")
        self.check_environment("PROTON_FSR4_UPGRADE", "fsr4")
        self.check_environment("PROTON_DLSS_UPGRADE", "dlss")
        self.check_environment("PROTON_NO_NTSYNC", "nontsync")
        self.env["DXVK_HDR"] = "1"
        name = "PROTON_" + opt.upper()
        lower = "PROTON_lowercase"
"#;

#[test]
fn runner_ge() {
    let f = Fake::new();
    let dir = proton_dir(
        &f,
        "~/.local/share/Steam/compatibilitytools.d/GE-Proton10-25",
        "GE-Proton10-25",
        GE_SCRIPT,
        &["/dev/ntsync", "PROTON_NO_NTSYNC"],
        true,
        true,
    );
    f.write(
        "~/.local/share/Steam/compatibilitytools.d/GE-Proton10-25/protonfixes/upscalers.py",
        "os.environ['PROTON_XESS_UPGRADE']\nos.getenv(\"DXVK_NVAPI_DRS_SETTINGS\")\n",
    );
    f.write(
        "~/.local/share/Steam/compatibilitytools.d/GE-Proton10-25/protonfixes/gamefixes-steam/123.py",
        "PROTON_NOT_TOP_LEVEL",
    );
    let rc = runner_caps(&dir);
    assert_eq!(rc.family, RunnerFamily::Ge);
    assert_eq!(rc.version, "GE-Proton10-25");
    let mut vars: Vec<&str> = rc.vars.iter().map(String::as_str).collect();
    vars.sort();
    assert_eq!(
        vars,
        [
            "DXVK_HDR",
            "DXVK_NVAPI_DRS_SETTINGS",
            "PROTON_DLSS_UPGRADE",
            "PROTON_ENABLE_HDR",
            "PROTON_ENABLE_WAYLAND",
            "PROTON_FORCE_HDR",
            "PROTON_FSR4_UPGRADE",
            "PROTON_NO_NTSYNC",
            "PROTON_XESS_UPGRADE",
        ]
    );
    assert!(rc.ntsync && rc.wineserver_no_ntsync);
    assert!(rc.winewayland && rc.fshack_fsr);
    assert!(!rc.contrib_fsr4);
    assert!(rc.has("PROTON_ENABLE_WAYLAND") && !rc.has("PROTON_MADE_UP"));
}

#[test]
fn runner_valve_umu() {
    let f = Fake::new();
    let dir = proton_dir(
        &f,
        "/r/UMU-Proton-10.0-4",
        "UMU-Proton-10.0-4",
        "self.check_environment(\"PROTON_LOG\", \"log\")\n",
        &["/dev/ntsync"],
        false,
        false,
    );
    f.write("/r/UMU-Proton-10.0-4/files/contrib/amdxcffx64.dll", "MZ");
    let rc = runner_caps(&dir);
    assert_eq!(rc.family, RunnerFamily::Valve);
    assert_eq!(rc.version, "UMU-Proton-10.0-4");
    assert_eq!(rc.vars, HashSet::from(["PROTON_LOG".to_string()]));
    assert!(rc.ntsync && !rc.wineserver_no_ntsync);
    assert!(!rc.winewayland && !rc.fshack_fsr);
    assert!(rc.contrib_fsr4);
    assert!(!rc.has("PROTON_ENABLE_WAYLAND"));
}

#[test]
fn runner_cachyos_and_cache() {
    let f = Fake::new();
    let dir = proton_dir(
        &f,
        "/usr/share/steam/compatibilitytools.d/proton-cachyos",
        "cachyos-11.0-20261015-slr",
        "PROTON_FSR4_UPGRADE PROTON_ENABLE_WAYLAND",
        &["PROTON_NO_NTSYNC", "/dev/ntsync"],
        true,
        true,
    );
    let a = runner_caps(&dir);
    assert_eq!(a.family, RunnerFamily::Cachyos);
    assert!(a.vars.contains("PROTON_FSR4_UPGRADE") && !a.vars.contains("PROTON_ENABLE_HDR"));
    let b = runner_caps(&dir);
    assert!(Arc::ptr_eq(&a, &b), "second call is cached");
    // A new build in place (version rewritten, mtime moved) is rescanned.
    let version = dir.join("version");
    std::fs::write(&version, "1800000000 GE-Proton11-1\n").unwrap();
    let later = SystemTime::now() + Duration::from_secs(10);
    std::fs::File::options().write(true).open(&version).unwrap().set_modified(later).unwrap();
    let c = runner_caps(&dir);
    assert!(!Arc::ptr_eq(&a, &c));
    assert_eq!(c.family, RunnerFamily::Ge);
}

#[test]
fn not_a_proton_dir_is_unknown() {
    let f = Fake::new();
    f.write("/r/wine-ge/bin/wine", "ELF");
    let rc = runner_caps(&f.path("/r/wine-ge"));
    assert_eq!(*rc, RunnerCaps::unknown());
    assert!(rc.has("PROTON_ENABLE_WAYLAND"), "unknown greys nothing out");
    assert_eq!(*runner_caps(&f.path("/does/not/exist")), RunnerCaps::unknown());
}

#[test]
fn wine_caps_system_and_custom_roots() {
    let f = Fake::new();
    // Arch-style system wine: /usr/bin/wine + /usr/lib/wine/x86_64-unix.
    let wine = f.write("/usr/bin/wine", "ELF");
    f.write("/usr/bin/wineserver", b"..\0/dev/ntsync\0..");
    f.write("/usr/lib/wine/x86_64-unix/winewayland.so", "ELF");
    f.write("/usr/lib/wine/x86_64-unix/win32u.so", b"\x7fELF..WINE_FULLSCREEN_FSR\0");
    let rc = wine_caps(&wine);
    assert!(rc.winewayland && rc.fshack_fsr && rc.ntsync && !rc.wineserver_no_ntsync);
    assert_eq!(rc.family, RunnerFamily::Unknown);
    assert!(rc.vars.is_empty());
    assert_eq!(rc.version, "usr");
    assert!(Arc::ptr_eq(&rc, &wine_caps(&wine)), "cached");
    // Debian multiarch layout, reached through a symlinked bin/wine (alternatives).
    let deb = f.write("/deb/usr/lib/wine/wine64", "ELF");
    f.write("/deb/usr/lib/x86_64-linux-gnu/wine/x86_64-unix/win32u.so", "no fsr here");
    std::fs::create_dir_all(f.path("/deb/usr/bin")).unwrap();
    std::os::unix::fs::symlink(&deb, f.path("/deb/usr/bin/wine")).unwrap();
    let rc = wine_caps(&f.path("/deb/usr/bin/wine"));
    assert!(!rc.winewayland && !rc.fshack_fsr && !rc.ntsync);
    assert_eq!(rc.version, "usr");
    // A custom build without a unix folder, and a missing binary.
    let bare = f.write("/opt/wine-x/bin/wine", "ELF");
    assert_eq!(*wine_caps(&bare), RunnerCaps::unknown());
    assert_eq!(*wine_caps(&f.path("/nope/bin/wine")), RunnerCaps::unknown());
}

#[test]
fn wine_inside_proton_dir_uses_runner_caps() {
    let f = Fake::new();
    let dir = proton_dir(
        &f,
        "/r/GE-Proton10-25",
        "GE-Proton10-25",
        GE_SCRIPT,
        &["/dev/ntsync", "PROTON_NO_NTSYNC"],
        true,
        true,
    );
    let wine = f.write("/r/GE-Proton10-25/files/bin/wine64", "ELF");
    let rc = wine_caps(&wine);
    assert!(Arc::ptr_eq(&rc, &runner_caps(&dir)));
    assert_eq!(rc.family, RunnerFamily::Ge);
    assert!(rc.wineserver_no_ntsync && rc.winewayland && rc.fshack_fsr);
}

#[test]
fn version_file_and_family() {
    assert_eq!(parse_version_file("1793687165 GE-Proton11-12\n"), "GE-Proton11-12");
    assert_eq!(parse_version_file("proton-10.0-3\n"), "proton-10.0-3");
    assert_eq!(runner_family("GE-Proton9-27"), RunnerFamily::Ge);
    assert_eq!(runner_family("proton-cachyos-10.0-20250101"), RunnerFamily::Cachyos);
    assert_eq!(runner_family("proton-11.0-1"), RunnerFamily::Valve);
}

#[test]
fn needle_across_chunk_boundary() {
    let f = Fake::new();
    let mut data = vec![b'x'; CHUNK - 3];
    data.extend_from_slice(b"/dev/ntsync");
    let p = f.write("/blob", data);
    assert_eq!(file_contains(&p, &[b"/dev/ntsync", b"nope"]), [true, false]);
    assert_eq!(file_contains(&f.path("/missing"), &[b"x"]), [false]);
}

#[test]
fn runners_pre_probed() {
    let f = Fake::new();
    proton_dir(
        &f,
        "~/.local/share/Steam/compatibilitytools.d/GE-Proton10-25",
        "GE-Proton10-25",
        GE_SCRIPT,
        &[],
        false,
        false,
    );
    proton_dir(
        &f,
        "~/.local/share/Steam/compatibilitytools.d/UMU-Proton-10.0-4",
        "UMU-Proton-10.0-4",
        "",
        &[],
        false,
        false,
    );
    proton_dir(&f, "/usr/share/steam/compatibilitytools.d/proton-cachyos", "cachyos-11.0", "", &[], false, false);
    f.write("~/.local/share/Steam/compatibilitytools.d/not-proton/readme", "x");
    // ~/.steam/root → ~/.local/share/Steam: probed once.
    std::fs::create_dir_all(f.path("~/.steam")).unwrap();
    std::os::unix::fs::symlink(f.path("~/.local/share/Steam"), f.path("~/.steam/root")).unwrap();
    let caps = probe_with(&f.roots());
    let mut keys: Vec<PathBuf> = caps.runners.keys().cloned().collect();
    keys.sort();
    let mut want = vec![
        f.path("~/.local/share/Steam/compatibilitytools.d/GE-Proton10-25"),
        f.path("~/.local/share/Steam/compatibilitytools.d/UMU-Proton-10.0-4"),
        f.path("/usr/share/steam/compatibilitytools.d/proton-cachyos"),
    ];
    want.sort();
    assert_eq!(keys, want);
    let cachy = f.path("/usr/share/steam/compatibilitytools.d/proton-cachyos");
    assert_eq!(caps.runners[&cachy].family, RunnerFamily::Cachyos);
}

// ------------------------------------------------------------------------------------ hints

#[test]
fn install_hints() {
    assert_eq!(install_hint(OptionId::Mangohud, Distro::Arch), Some("sudo pacman -S mangohud lib32-mangohud"));
    assert_eq!(install_hint(OptionId::Mangohud, Distro::Fedora), Some("sudo dnf install mangohud"));
    assert!(install_hint(OptionId::Gamescope, Distro::Debian).unwrap().contains("26.04"));
    assert!(install_hint(OptionId::PerformanceProfile, Distro::Fedora).unwrap().contains("tuned-ppd"));
    assert!(install_hint(OptionId::PerformanceProfile, Distro::Arch).unwrap().contains("cachyos-settings"));
    assert!(install_hint(OptionId::ObsCapture, Distro::Arch).unwrap().contains("AUR"));
    assert!(install_hint(OptionId::Lsfg, Distro::Fedora).unwrap().contains("lsfg-vk.dev"));
    assert!(install_hint(OptionId::Ntsync, Distro::Debian).unwrap().contains("modprobe ntsync"));
    assert!(install_hint(OptionId::Wayland, Distro::Arch).unwrap().contains("proton-cachyos"));
    assert_eq!(install_hint(OptionId::Wayland, Distro::Fedora), Some("Install GE-Proton (ProtonUp-Qt)"));
    assert!(install_hint(OptionId::KeepAwake, Distro::Other).unwrap().contains("systemd"));
    assert_eq!(install_hint(OptionId::PreferDgpu, Distro::Arch), None);
    assert_eq!(install_hint(OptionId::Hdr, Distro::Arch), None);
    // Every distro has a line for every option that has one on Arch.
    for o in OPTIONS {
        if install_hint(o.id, Distro::Arch).is_some() {
            for d in [Distro::Fedora, Distro::Debian, Distro::Other] {
                assert!(install_hint(o.id, d).is_some(), "{:?} {:?}", o.id, d);
            }
        }
    }
    assert_eq!(
        flatpak_ext_hint("org.freedesktop.Platform.VulkanLayer.MangoHud", "25.08"),
        "flatpak install flathub org.freedesktop.Platform.VulkanLayer.MangoHud//25.08"
    );
}

#[test]
fn empty_system_probe_is_fast_and_empty() {
    let f = Fake::new();
    let t = Instant::now();
    let caps = probe_with(&f.roots());
    assert!(t.elapsed() < Duration::from_millis(500));
    assert_eq!(caps.bins, Bins::default());
    assert!(caps.gpus.is_empty() && caps.layers.is_empty() && caps.runners.is_empty());
    assert!(!caps.wayland_session && !caps.nested_gamescope);
}

// --------------------------------------------------------------------- review regressions

#[test]
fn ntsync_dkms_and_extra_modules_count_as_not_loaded() {
    // DKMS (updates/dkms/), found through modules.dep.
    let f = Fake::new();
    f.write("/proc/sys/kernel/osrelease", "6.12.9-lts\n");
    f.write(
        "/usr/lib/modules/6.12.9-lts/modules.dep",
        "kernel/fs/fat/fat.ko.zst:\nupdates/dkms/ntsync.ko.zst:\nkernel/drivers/misc/ntsyncx.ko.zst:\n",
    );
    assert_eq!(probe_with(&f.roots()).ntsync, NtsyncState::NotLoaded);
    // No modules.dep: an out-of-tree build under extra/.
    let f = Fake::new();
    f.write("/proc/sys/kernel/osrelease", "6.12.9-lts\n");
    f.write("/lib/modules/6.12.9-lts/extra/ntsync.ko", "");
    assert_eq!(probe_with(&f.roots()).ntsync, NtsyncState::NotLoaded);
    // A modules.dep without it (a similarly named module isn't it): Missing.
    let f = Fake::new();
    f.write("/proc/sys/kernel/osrelease", "6.12.9-lts\n");
    f.write("/usr/lib/modules/6.12.9-lts/modules.dep", "kernel/drivers/misc/ntsyncx.ko.zst:\n");
    assert_eq!(probe_with(&f.roots()).ntsync, NtsyncState::Missing);
    // Built in (but /dev/ntsync not readable here).
    f.write("/usr/lib/modules/6.12.9-lts/modules.builtin", "kernel/drivers/misc/ntsync.ko\n");
    assert_eq!(probe_with(&f.roots()).ntsync, NtsyncState::NotLoaded);
}

#[test]
fn probe_fills_hdr_outputs() {
    let f = Fake::new();
    f.card(1, "0x1002", "0x7550", "0000:03:00.0", "amdgpu", true, None);
    f.connector(1, "DP-2", true, "2560x1440\n");
    f.connector(1, "DP-3", true, "1920x1080\n");
    f.write("/sys/class/drm/card1-DP-2/edid", edid_with_cta(Some(0b101)));
    f.write("/sys/class/drm/card1-DP-3/edid", edid_with_cta(None));
    assert_eq!(probe_with(&f.roots()).hdr_outputs, ["DP-2"]);
    assert!(probe_with(&Fake::new().roots()).hdr_outputs.is_empty());
}

/// Whether `runner_caps`'s process cache holds `dir` (by its canonical path).
fn warmed(dir: &Path) -> bool {
    dir.canonicalize().is_ok_and(|k| runner_cache().lock().unwrap().contains_key(&k))
}

#[test]
fn runners_outside_compat_dirs_are_pre_probed() {
    let f = Fake::new();
    // Steam's own Proton (a Steam root needs a steamapps folder).
    let steam_proton = "~/.local/share/Steam/steamapps/common/Proton 9.0 (Beta)";
    proton_dir(&f, steam_proton, "proton-9.0-4", "", &[], false, false);
    f.write("~/.local/share/Steam/steamapps/common/Proton EasyAntiCheat Runtime/readme", "x");
    f.write("~/.local/share/Steam/steamapps/common/Half-Life/proton", "not a runner (name)");
    // A Lutris (Flatpak) wine runner that is a Proton build, Heroic's Proton, Bottles.
    let lutris = "~/.var/app/net.lutris.Lutris/data/lutris/runners/wine/dw-proton-x86_64";
    proton_dir(&f, lutris, "dw-proton", "", &[], false, false);
    f.write("~/.var/app/net.lutris.Lutris/data/lutris/runners/wine/lutris-7.2/bin/wine", "ELF");
    let heroic = "~/.config/heroic/tools/proton/GE-Proton10-30";
    proton_dir(&f, heroic, "GE-Proton10-30", GE_SCRIPT, &[], false, false);
    let bottles = "~/.local/share/bottles/runners/ge-proton-9";
    proton_dir(&f, bottles, "GE-Proton9-1", "", &[], false, false);
    // umu's own GE-Proton, older than Heroic's.
    let umu_ge = "~/.local/share/Steam/compatibilitytools.d/GE-Proton9-20";
    proton_dir(&f, umu_ge, "GE-Proton9-20", GE_SCRIPT, &[], false, false);
    let caps = probe_with(&f.roots());
    // Only compatibilitytools.d is in Caps::runners: `PROTONPATH=GE-Proton` must pick umu's
    // build, never Heroic's newer one.
    let keys: Vec<PathBuf> = caps.runners.keys().cloned().collect();
    assert_eq!(keys, [f.path(umu_ge)]);
    let pp = [("PROTONPATH".to_string(), "GE-Proton".to_string())];
    assert_eq!(crate::game_options::compose::proton_dir_for(&pp, "umu-run", &caps), Some(f.path(umu_ge)));
    // The others are warm in runner_caps's cache, so their first resolve is a cache hit.
    for d in [steam_proton, lutris, heroic, bottles] {
        assert!(warmed(&f.path(d)), "{d}");
    }
    assert!(!warmed(&f.path("~/.local/share/Steam/steamapps/common/Half-Life")));
    assert_eq!(runner_caps(&f.path(heroic)).family, RunnerFamily::Ge);
}

#[test]
fn extra_runner_pre_probe_is_bounded() {
    let f = Fake::new();
    let dirs: Vec<PathBuf> = (0..MAX_EXTRA_RUNNERS + 5)
        .map(|i| {
            proton_dir(
                &f,
                &format!("~/.config/heroic/tools/proton/GE-Proton{i:03}"),
                "GE-Proton",
                "",
                &[],
                false,
                false,
            )
        })
        .collect();
    assert_eq!(warm_extra_runners(&f.roots()), MAX_EXTRA_RUNNERS);
    assert_eq!(dirs.iter().filter(|d| warmed(d)).count(), MAX_EXTRA_RUNNERS);
    assert!(probe_with(&f.roots()).runners.is_empty());
}

#[test]
fn launch_probe_skips_warming() {
    let f = Fake::new();
    let d = proton_dir(&f, "~/.config/heroic/tools/proton/GE-Proton10-1", "GE-Proton10-1", "", &[], false, false);
    let _ = probe_impl(&f.roots(), false);
    assert!(!warmed(&d));
    let _ = probe_impl(&f.roots(), true);
    assert!(warmed(&d));
}

#[test]
fn runner_cache_hits_by_alias_path() {
    let f = Fake::new();
    let dir = proton_dir(&f, "/r/Proton 9.0", "proton-9.0-4", "PROTON_LOG", &[], false, false);
    let alias = f.path("/r/alias");
    std::os::unix::fs::symlink(&dir, &alias).unwrap();
    let a = runner_caps(&dir);
    let b = runner_caps(&alias);
    assert!(Arc::ptr_eq(&a, &b), "the alias shares the canonical dir's scan");
    assert!(Arc::ptr_eq(&b, &runner_caps(&alias)), "and hits by its own path next time");
    // A rebuilt runner is rescanned through either path.
    let version = dir.join("version");
    std::fs::write(&version, "1800000000 GE-Proton11-1\n").unwrap();
    let later = SystemTime::now() + Duration::from_secs(10);
    std::fs::File::options().write(true).open(&version).unwrap().set_modified(later).unwrap();
    let c = runner_caps(&alias);
    assert!(!Arc::ptr_eq(&a, &c));
    assert_eq!(c.family, RunnerFamily::Ge);
    assert!(Arc::ptr_eq(&c, &runner_caps(&dir)));
}
