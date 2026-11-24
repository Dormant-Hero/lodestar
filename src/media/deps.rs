//! What to install when a program Lodestar uses for theme media is missing, worded for the
//! user's Linux family (read from `/etc/os-release`): `ffmpeg` for theme videos, a sound player
//! (`pw-cat` / `pacat` / `aplay`) for navigation sounds and music.
//!
//! Package names checked against the distributions' own package lists (Nov 2026): Arch
//! (ffmpeg; pw-cat in pipewire-audio — not in pipewire), Debian / Ubuntu (ffmpeg; pw-cat in
//! pipewire-bin), Fedora (ffmpeg-free; pw-cat in pipewire-utils). openSUSE's lists couldn't be
//! checked, so it gets a plain instruction instead of a command.

use std::sync::OnceLock;

/// Linux families with different install commands, most used first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Arch,
    /// Steam Deck's SteamOS: Arch-based but its system files are read-only.
    SteamOs,
    Debian,
    Fedora,
    /// Fedora Silverblue / Kinoite, Bazzite, Bluefin, Aurora …: image-based, rpm-ostree.
    FedoraAtomic,
    OpenSuse,
    Nix,
    Gentoo,
    Void,
    Unknown,
}

/// What is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// ffmpeg, for theme videos.
    Video,
    /// A sound player, for navigation sounds and music.
    Audio,
}

/// What to tell the user: a sentence, and a command to copy when there is a reliable one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub text: String,
    pub command: Option<String>,
}

/// The family from an os-release file's text.
pub fn family_from(os_release: &str) -> Family {
    let field = |k: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('=')))
            .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_ascii_lowercase())
            .unwrap_or_default()
    };
    let id = field("ID");
    let like = field("ID_LIKE");
    let variant = field("VARIANT_ID");
    let words: Vec<&str> = std::iter::once(id.as_str()).chain(like.split_whitespace()).collect();
    let is = |w: &str| words.contains(&w);
    if id == "steamos" {
        Family::SteamOs
    } else if id == "nixos" {
        Family::Nix
    } else if is("gentoo") {
        Family::Gentoo
    } else if id == "void" {
        Family::Void
    } else if is("arch") {
        Family::Arch
    } else if is("debian") || is("ubuntu") {
        Family::Debian
    } else if is("suse") || is("opensuse") || id.starts_with("opensuse") || like.contains("suse") {
        Family::OpenSuse
    } else if is("fedora") || is("rhel") || is("centos") {
        let atomic =
            ["silverblue", "kinoite", "sericea", "onyx", "atomic", "coreos"].iter().any(|v| variant.contains(v))
                || ["bazzite", "bluefin", "aurora", "ublue"].iter().any(|v| id.contains(v));
        if atomic { Family::FedoraAtomic } else { Family::Fedora }
    } else {
        Family::Unknown
    }
}

/// This machine's family (read once).
pub fn family() -> Family {
    static F: OnceLock<Family> = OnceLock::new();
    *F.get_or_init(|| {
        let text = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .unwrap_or_default();
        family_from(&text)
    })
}

/// What to install for `need` on `f`.
pub fn hint(f: Family, need: Need) -> Hint {
    let cmd = |c: &str| Some(c.to_string());
    let (text, command): (String, Option<String>) = match (need, f) {
        (Need::Video, Family::Arch) => ("Install it with:".into(), cmd("sudo pacman -S ffmpeg")),
        (Need::Audio, Family::Arch) => ("Install PipeWire's player with:".into(), cmd("sudo pacman -S pipewire-audio")),
        (Need::Video, Family::Debian) => ("Install it with:".into(), cmd("sudo apt install ffmpeg")),
        (Need::Audio, Family::Debian) => {
            ("Install PipeWire's player with:".into(), cmd("sudo apt install pipewire-bin"))
        }
        (Need::Video, Family::Fedora) => (
            "Install it with the command below (for every video format, the full ffmpeg from RPM Fusion):".into(),
            cmd("sudo dnf install ffmpeg-free"),
        ),
        (Need::Audio, Family::Fedora) => {
            ("Install PipeWire's player with:".into(), cmd("sudo dnf install pipewire-utils"))
        }
        (Need::Video, Family::FedoraAtomic) => (
            "This system is image-based: add it with the command below, then restart.".into(),
            cmd("rpm-ostree install ffmpeg-free"),
        ),
        (Need::Audio, Family::FedoraAtomic) => (
            "This system is image-based: add PipeWire's player with the command below, then restart.".into(),
            cmd("rpm-ostree install pipewire-utils"),
        ),
        (Need::Video, Family::SteamOs) => (
            "SteamOS's system files are read-only, so it can't be installed the usual way. Put a ready-made \
             ffmpeg (a \"static build\" for Linux x86_64) into ~/.local/bin and Lodestar finds it there."
                .into(),
            None,
        ),
        (Need::Audio, Family::SteamOs) => (
            "SteamOS normally has PipeWire's player (pw-cat). If sound still doesn't work, switch to Desktop Mode \
             once to let SteamOS finish its audio set-up."
                .into(),
            None,
        ),
        (Need::Video, Family::OpenSuse) => (
            "Install ffmpeg with YaST or zypper (for every video format, the ffmpeg from the Packman repository)."
                .into(),
            None,
        ),
        (Need::Audio, Family::OpenSuse) => ("Install PipeWire's tools (pw-cat) with YaST or zypper.".into(), None),
        (Need::Video, Family::Nix) => (
            "Add ffmpeg to environment.systemPackages in /etc/nixos/configuration.nix and rebuild, or for just \
             your user:"
                .into(),
            cmd("nix-env -iA nixos.ffmpeg"),
        ),
        (Need::Audio, Family::Nix) => (
            "Turn on PipeWire in /etc/nixos/configuration.nix (services.pipewire.enable = true;) and rebuild.".into(),
            None,
        ),
        (Need::Video, Family::Gentoo) => ("Install it with:".into(), cmd("sudo emerge --ask media-video/ffmpeg")),
        (Need::Audio, Family::Gentoo) => {
            ("Install PipeWire with:".into(), cmd("sudo emerge --ask media-video/pipewire"))
        }
        (Need::Video, Family::Void) => ("Install it with:".into(), cmd("sudo xbps-install -S ffmpeg")),
        (Need::Audio, Family::Void) => ("Install PipeWire with:".into(), cmd("sudo xbps-install -S pipewire")),
        (Need::Video, Family::Unknown) => {
            ("Install ffmpeg with your system's software centre or package manager.".into(), None)
        }
        (Need::Audio, Family::Unknown) => (
            "Install PipeWire's tools (pw-cat), PulseAudio's (pacat) or ALSA's (aplay) with your system's software \
             centre or package manager."
                .into(),
            None,
        ),
    };
    Hint { text, command }
}

/// One line for the user: what's missing, what it's for, and how to get it.
pub fn message(need: Need) -> String {
    let h = hint(family(), need);
    let lead = match need {
        Need::Video => "Theme videos need ffmpeg, which isn't installed; the theme shows its still pictures meanwhile.",
        Need::Audio => "Sounds and music need a sound player (PipeWire's pw-cat, pacat or aplay); none was found.",
    };
    match &h.command {
        Some(c) => format!("{lead} {} {c}", h.text),
        None => format!("{lead} {}", h.text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(id: &str, like: &str, variant: &str) -> String {
        format!("NAME=\"X\"\nID={id}\nID_LIKE=\"{like}\"\nVARIANT_ID={variant}\n")
    }

    #[test]
    fn families_from_os_release() {
        assert_eq!(family_from(&os("cachyos", "arch", "")), Family::Arch);
        assert_eq!(family_from(&os("endeavouros", "arch", "")), Family::Arch);
        assert_eq!(family_from(&os("manjaro", "arch", "")), Family::Arch);
        assert_eq!(family_from(&os("steamos", "arch", "")), Family::SteamOs);
        assert_eq!(family_from(&os("ubuntu", "debian", "")), Family::Debian);
        assert_eq!(family_from(&os("linuxmint", "ubuntu debian", "")), Family::Debian);
        assert_eq!(family_from(&os("pop", "ubuntu debian", "")), Family::Debian);
        assert_eq!(family_from(&os("debian", "", "")), Family::Debian);
        assert_eq!(family_from(&os("fedora", "", "workstation")), Family::Fedora);
        assert_eq!(family_from(&os("nobara", "fedora", "")), Family::Fedora);
        assert_eq!(family_from(&os("fedora", "", "silverblue")), Family::FedoraAtomic);
        assert_eq!(family_from(&os("bazzite", "fedora", "")), Family::FedoraAtomic);
        assert_eq!(family_from(&os("opensuse-tumbleweed", "opensuse suse", "")), Family::OpenSuse);
        assert_eq!(family_from(&os("nixos", "", "")), Family::Nix);
        assert_eq!(family_from(&os("gentoo", "", "")), Family::Gentoo);
        assert_eq!(family_from(&os("void", "", "")), Family::Void);
        assert_eq!(family_from(""), Family::Unknown);
        // Quotes and single quotes.
        assert_eq!(family_from("ID='arch'\n"), Family::Arch);
    }

    #[test]
    fn every_family_has_a_hint() {
        use Family::*;
        for f in [Arch, SteamOs, Debian, Fedora, FedoraAtomic, OpenSuse, Nix, Gentoo, Void, Unknown] {
            for n in [Need::Video, Need::Audio] {
                let h = hint(f, n);
                assert!(!h.text.is_empty(), "{f:?} {n:?}");
                if let Some(c) = &h.command {
                    assert!(!c.contains('\n') && !c.is_empty());
                }
            }
        }
        assert_eq!(hint(Arch, Need::Audio).command.as_deref(), Some("sudo pacman -S pipewire-audio"));
        assert_eq!(hint(Debian, Need::Audio).command.as_deref(), Some("sudo apt install pipewire-bin"));
        assert_eq!(hint(Fedora, Need::Video).command.as_deref(), Some("sudo dnf install ffmpeg-free"));
        assert!(hint(OpenSuse, Need::Video).command.is_none(), "no guessed commands");
    }
}
