//! GPUs, and the outputs (connectors) with their modes and HDR.

use super::*;

// -------------------------------------------------------------------------------------- GPUs

/// `card0`, `card1` … (not `card1-DP-2`, `renderD128`).
pub(super) fn is_card_name(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// One [`Gpu`] per `/sys/class/drm/cardN` that has a PCI device.
pub(super) fn find_gpus(r: &ProbeRoots) -> Vec<Gpu> {
    list_dir(&r.at("/sys/class/drm"))
        .into_iter()
        .filter(|(name, _)| is_card_name(name))
        .filter_map(|(name, path)| read_gpu(r, &name, &path))
        .collect()
}

/// Read one card's attributes.
pub(super) fn read_gpu(r: &ProbeRoots, card: &str, path: &Path) -> Option<Gpu> {
    let dev = path.join("device");
    let vendor = parse_hex_u16(&read_attr(&dev.join("vendor"))?)?;
    let device = read_attr(&dev.join("device")).and_then(|d| parse_hex_u16(&d)).unwrap_or(0);
    let uevent = read_text(&dev.join("uevent")).unwrap_or_default();
    let uevent_val =
        |key: &str| uevent.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=').map(str::to_string));
    let basename = |p: PathBuf| p.file_name().and_then(|n| n.to_str()).map(str::to_string);
    let pci = uevent_val("PCI_SLOT_NAME").or_else(|| dev.canonicalize().ok().and_then(basename)).unwrap_or_default();
    let driver = std::fs::read_link(dev.join("driver"))
        .ok()
        .and_then(basename)
        .or_else(|| uevent_val("DRIVER"))
        .unwrap_or_default();
    let gc = dev.join("ip_discovery/die/0/GC/0");
    let gc_part = |f: &str| read_attr(&gc.join(f)).and_then(|v| v.parse::<u8>().ok());
    Some(Gpu {
        card: card.to_string(),
        pci,
        vendor,
        device,
        driver,
        boot_vga: read_attr(&dev.join("boot_vga")).as_deref() == Some("1"),
        vram: read_attr(&dev.join("mem_info_vram_total")).and_then(|v| v.parse().ok()),
        gc_ip: gc_part("major").zip(gc_part("minor")),
        discrete_tag: switcheroo_discrete(r, card, path),
    })
}

/// The udev tag `G:switcheroo-discrete-gpu` in `/run/udev/data/c<major>:<minor>` (the card's
/// `dev` attribute; falls back to 226:N).
pub(super) fn switcheroo_discrete(r: &ProbeRoots, card: &str, path: &Path) -> bool {
    let devnum = read_attr(&path.join("dev")).unwrap_or_else(|| format!("226:{}", &card[4..]));
    read_text(&r.at("/run/udev/data").join(format!("c{devnum}")))
        .is_some_and(|t| t.lines().any(|l| l.trim() == "G:switcheroo-discrete-gpu"))
}

/// PCI vendor ids.
#[cfg(test)]
pub const VENDOR_AMD: u16 = 0x1002;

#[cfg(test)]
pub const VENDOR_NVIDIA: u16 = 0x10de;

/// The GPU games should render on (spec §4.2): NVIDIA proprietary > udev discrete tag >
/// largest VRAM (unknown VRAM counts as 0); ties keep the earlier card. None without GPUs.
#[cfg(test)]
pub fn best_gpu(gpus: &[Gpu]) -> Option<&Gpu> {
    let rank = |g: &Gpu| (g.vendor == VENDOR_NVIDIA && g.driver == "nvidia", g.discrete_tag, g.vram.unwrap_or(0));
    gpus.iter().fold(None, |best: Option<&Gpu>, g| match best {
        Some(b) if rank(b) >= rank(g) => Some(b),
        _ => Some(g),
    })
}

/// The proprietary NVIDIA driver version: `/sys/module/nvidia/version`, else the version in
/// `/proc/driver/nvidia/version` (same driver, other interface). Spec §4.2 also names
/// `/proc/modules`, but that has no version and a loaded module always has `/sys/module/nvidia`,
/// so it adds nothing; "is an NVIDIA GPU on the proprietary driver" is `Gpu::driver == "nvidia"`.
pub(super) fn nvidia_version(r: &ProbeRoots) -> Option<String> {
    read_attr(&r.at("/sys/module/nvidia/version"))
        .or_else(|| read_text(&r.at("/proc/driver/nvidia/version")).and_then(|t| parse_nvrm_version(&t)))
}

/// "NVRM version: NVIDIA UNIX x86_64 Kernel Module  580.95.05  Tue …" → "580.95.05": the first
/// dotted number on the first line.
pub(super) fn parse_nvrm_version(text: &str) -> Option<String> {
    text.lines().next()?.split_whitespace().find_map(|w| {
        let dotted = w.contains('.') && w.bytes().all(|b| b.is_ascii_digit() || b == b'.');
        (dotted && w.as_bytes()[0].is_ascii_digit()).then(|| w.to_string())
    })
}

// ------------------------------------------------------------------------------ outputs/modes

/// One `/sys/class/drm/cardN-<connector>`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Connector {
    card: String,
    /// "DP-2", "HDMI-A-1".
    name: String,
    path: PathBuf,
    /// First line of `modes` (the preferred mode **[U]**).
    first_mode: Option<(u32, u32)>,
}

/// Connected connectors of every card.
pub(super) fn find_connectors(r: &ProbeRoots) -> Vec<Connector> {
    list_dir(&r.at("/sys/class/drm"))
        .into_iter()
        .filter_map(|(name, path)| {
            let (card, conn) = name.split_once('-')?;
            if !is_card_name(card) || read_attr(&path.join("status")).as_deref() != Some("connected") {
                return None;
            }
            let first_mode = read_text(&path.join("modes")).and_then(|m| m.lines().next().and_then(parse_mode));
            Some(Connector { card: card.to_string(), name: conn.to_string(), path, first_mode })
        })
        .collect()
}

/// "2560x1440" (optionally with a suffix like "i") → (2560, 1440).
pub(super) fn parse_mode(line: &str) -> Option<(u32, u32)> {
    let (w, h) = line.trim().split_once('x')?;
    let h: String = h.chars().take_while(|c| c.is_ascii_digit()).collect();
    Some((w.parse().ok()?, h.parse().ok()?)).filter(|(w, h)| *w > 0 && *h > 0)
}

/// Fallback monitor size: the first connected connector with a mode on the boot_vga card, else
/// on any card.
pub(super) fn preferred_mode(gpus: &[Gpu], connectors: &[Connector]) -> Option<(u32, u32)> {
    let boot = gpus.iter().find(|g| g.boot_vga).map(|g| g.card.as_str());
    connectors.iter().filter(|c| Some(c.card.as_str()) == boot).chain(connectors.iter()).find_map(|c| c.first_mode)
}

/// KDE's `kwinoutputconfig.json` HDR / VRR settings for the connected outputs (notes only):
/// (any connected output has HDR on, every connected output has VRR policy "Never"). Entries
/// are joined by `connectorName`, and by `edidIdentifier` when several entries share a
/// connector. (None, None) when the file is missing or nothing matches.
pub(super) fn kde_output_settings(r: &ProbeRoots, connectors: &[Connector]) -> (Option<bool>, Option<bool>) {
    let Some(v) = read_small(&r.config_home().join("kwinoutputconfig.json"))
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    else {
        return (None, None);
    };
    let outputs: Vec<&serde_json::Value> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s.get("name").and_then(|n| n.as_str()) == Some("outputs"))
        .filter_map(|s| s.get("data")?.as_array())
        .flatten()
        .collect();
    let mut matched: Vec<&serde_json::Value> = Vec::new();
    for c in connectors {
        let same: Vec<&serde_json::Value> = outputs
            .iter()
            .copied()
            .filter(|o| o.get("connectorName").and_then(|n| n.as_str()) == Some(c.name.as_str()))
            .collect();
        let pick = match same.len() {
            0 => None,
            1 => Some(same[0]),
            _ => {
                let id = read_small(&c.path.join("edid")).and_then(|e| edid_identifier(&e));
                id.and_then(|id| {
                    same.iter().copied().find(|o| {
                        o.get("edidIdentifier").and_then(|n| n.as_str()).is_some_and(|s| edid_id_matches(s, &id))
                    })
                })
            }
        };
        matched.extend(pick);
    }
    if matched.is_empty() {
        return (None, None);
    }
    let hdr = matched.iter().any(|o| o.get("highDynamicRange").and_then(|h| h.as_bool()) == Some(true));
    let never = matched.iter().all(|o| o.get("vrrPolicy").and_then(|p| p.as_str()) == Some("Never"));
    (Some(hdr), Some(never))
}

/// KWin's `edidIdentifier` ("AOC 45862 219 14 2024 0") starts with our five fields `id`, as
/// whole words (so serial "21" doesn't match "219").
pub(super) fn edid_id_matches(kwin: &str, id: &str) -> bool {
    kwin.strip_prefix(id).is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
}

/// The first five fields of KWin's `edidIdentifier` ("AOC 45862 219 14 2024") from an EDID
/// blob: PNP id, product code, serial, week, year.
pub(super) fn edid_identifier(e: &[u8]) -> Option<String> {
    if e.len() < 18 || e[..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return None;
    }
    let m = u16::from_be_bytes([e[8], e[9]]);
    let letter = |shift: u16| (b'@' + ((m >> shift) & 0x1f) as u8) as char; // 1 = 'A'
    let pnp: String = [letter(10), letter(5), letter(0)].iter().collect();
    let product = u16::from_le_bytes([e[10], e[11]]);
    let serial = u32::from_le_bytes([e[12], e[13], e[14], e[15]]);
    Some(format!("{pnp} {product} {serial} {} {}", e[16], 1990 + e[17] as u32))
}

/// The names of `connectors` whose EDID has HDR with PQ.
pub(super) fn pq_outputs(connectors: &[Connector]) -> Vec<String> {
    connectors
        .iter()
        .filter(|c| read_small(&c.path.join("edid")).is_some_and(|e| edid_has_pq(&e)))
        .map(|c| c.name.clone())
        .collect()
}

/// An EDID with a CTA-861 extension block (tag 0x02) holding an HDR static metadata data block
/// (extended tag 6) whose EOTF byte has SMPTE ST 2084 (PQ, bit 2) set. HDR info carried only
/// in a DisplayID extension isn't looked at.
pub fn edid_has_pq(e: &[u8]) -> bool {
    e.as_chunks::<128>().0.iter().skip(1).any(cta_block_has_pq)
}

/// One 128-byte CTA extension block: walk its data block collection (bytes 4 .. DTD offset).
pub(super) fn cta_block_has_pq(b: &[u8; 128]) -> bool {
    if b[0] != 0x02 {
        return false;
    }
    let end = (b[2] as usize).min(127);
    let mut i = 4;
    while i < end {
        let tag = b[i] >> 5;
        let len = (b[i] & 0x1f) as usize;
        if tag == 7 && len >= 2 && i + 2 < end && b[i + 1] == 6 && b[i + 2] & 0x04 != 0 {
            return true;
        }
        i += 1 + len;
    }
    false
}
