//! Store clients and game launchers are not games. Lutris, desktop entries, Bottles and Faugus
//! all list them next to the games (a Lutris "Battle.net" entry is the client its installer put
//! in a prefix; the itch / Moonlight Flatpaks have `Categories=Game`). This built-in list drops
//! them whatever the user's `scan.desktop_ignore` holds (a saved config keeps its own copy of
//! that list, so new defaults there would never reach existing users).
//!
//! The games inside those clients are found by their own scanners (Wine-prefix launchers,
//! Minecraft instances, Heroic, itch, Moonlight hosts …). Mojang's own Minecraft launcher is NOT
//! listed: no scanner reads its profiles, so it is the only way such a user's Minecraft shows up.

/// Lutris game slugs (and installer slug stems: `battlenet-standard` → `battlenet`) of store
/// clients and launchers.
pub const LUTRIS_SLUGS: &[&str] = &[
    "battlenet",
    "battle-net",
    "blizzard-app",
    "ea-app",
    "ea-desktop",
    "origin",
    "ubisoft-connect",
    "uplay",
    "epic-games-store",
    "epic-games-launcher",
    "epic-store",
    "gog-galaxy",
    "rockstar-games-launcher",
    "rockstar-launcher",
    "riot-client",
    "riot-games",
    "hoyoplay",
    "amazon-games",
    "amazon-games-app",
    "bethesda-net-launcher",
    "itch",
    "itch-io",
    "itchio",
    "heroic",
    "heroic-games-launcher",
    "legendary",
    "prism-launcher",
    "prismlauncher",
    "polymc",
    "multimc",
    "atlauncher",
    "steam",
    "lutris",
    "bottles",
    "faugus-launcher",
    "moonlight",
    "playnite",
    "wargaming-game-center",
    "xbox-app",
];

/// Desktop-file ids / Flatpak app ids of launchers (compared without `.desktop`, ignoring case).
pub const APP_IDS: &[&str] = &[
    "io.itch.itch",
    "itch",
    "com.moonlight_stream.Moonlight",
    "moonlight",
    "moonlight-qt",
    "org.prismlauncher.PrismLauncher",
    "prismlauncher",
    "org.polymc.PolyMC",
    "polymc",
    "multimc",
    "com.atlauncher.ATLauncher",
    "atlauncher",
    "com.heroicgameslauncher.hgl",
    "heroic",
    "net.lutris.Lutris",
    "lutris",
    "com.usebottles.bottles",
    "com.valvesoftware.Steam",
    "steam",
    "steam-native",
    "steam-runtime",
    "io.github.Faugus.faugus-launcher",
    "faugus-launcher",
    "net.davidotek.pupgui2",
    "com.vysp3r.ProtonPlus",
    "page.kramo.Cartridges",
    "hu.kramo.Cartridges",
    "com.github.Matoking.protontricks",
    "io.github.hedge_dev.hedgemodmanager",
    "com.github.mtkennerly.ludusavi",
    "io.github.limo_app.limo",
    "lodestar",
    "io.github.lodestar.Lodestar",
];

/// Exact names (lowercase, ™/® dropped) of store clients and launchers.
pub const NAMES: &[&str] = &[
    "battle.net",
    "blizzard battle.net",
    "ea app",
    "ea desktop",
    "origin client",
    "ubisoft connect",
    "uplay",
    "epic games store",
    "epic games launcher",
    "gog galaxy",
    "gog galaxy 2.0",
    "rockstar games launcher",
    "riot client",
    "hoyoplay",
    "amazon games",
    "itch.io",
    "heroic games launcher",
    "prism launcher",
    "polymc",
    "multimc",
    "atlauncher",
    "faugus launcher",
    "playnite",
    "wargaming.net game center",
];

fn norm_name(name: &str) -> String {
    name.replace(['™', '®', '©'], "").split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A Lutris entry is a store client / launcher (by slug, installer slug or exact name).
pub fn is_lutris_client(slug: &str, installer_slug: &str, name: &str) -> bool {
    let slug = slug.trim().to_ascii_lowercase();
    let inst = installer_slug.trim().to_ascii_lowercase();
    // The installer slug is `<game slug>-<version>` (`battlenet-standard`, `ea-app-standard`,
    // `prism-launcher-appimage`); it only stands in for a missing game slug.
    let by_installer = || {
        slug.is_empty()
            && LUTRIS_SLUGS.iter().any(|c| inst == *c || inst.strip_prefix(c).is_some_and(|r| r.starts_with('-')))
    };
    LUTRIS_SLUGS.contains(&slug.as_str()) || by_installer() || is_client_name(name)
}

/// An exact store-client / launcher name ("Battle.net", "EA App", "Ubisoft Connect").
pub fn is_client_name(name: &str) -> bool {
    let n = norm_name(name);
    NAMES.contains(&n.as_str())
}

/// Store-client executables, by file name (lowercase). Only names that are unambiguous on their own.
const EXE_NAMES: &[&str] = &[
    "battle.net.exe",
    "battle.net launcher.exe",
    "eadesktop.exe",
    "ealauncher.exe",
    "origin.exe",
    "ubisoftconnect.exe",
    "upc.exe",
    "uplay.exe",
    "epicgameslauncher.exe",
    "galaxyclient.exe",
    "riotclientservices.exe",
    "riot client.exe",
    "amazon games.exe",
    "amazon games ui.exe",
    "hoyoplay.exe",
    "playnite.desktopapp.exe",
    "playnite.fullscreenapp.exe",
    "wgc.exe",
];

/// Client executables whose file name alone is too generic (`launcher.exe`): matched by the end
/// of their path (lowercase, `/`-separated).
const EXE_PATH_ENDS: &[&str] = &["rockstar games/launcher/launcher.exe", "hoyoplay/launcher.exe"];

/// The program a launcher entry runs is a store client (whatever the entry is called):
/// a Faugus / Bottles / hand-made entry for "Blizzard" that runs `Battle.net Launcher.exe`.
/// Accepts Unix or Windows paths.
pub fn is_client_exe(path: &str) -> bool {
    let p = path.trim().trim_matches('"').replace('\\', "/").to_lowercase();
    let file = p.rsplit('/').next().unwrap_or(&p);
    EXE_NAMES.contains(&file) || EXE_PATH_ENDS.iter().any(|e| p.ends_with(e))
}

/// A desktop-file id (`io.itch.itch.desktop`) or Flatpak app id of a launcher.
pub fn is_launcher_app_id(id: &str) -> bool {
    let id = id.trim();
    let id = id.strip_suffix(".desktop").unwrap_or(id);
    APP_IDS.iter().any(|a| a.eq_ignore_ascii_case(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_executables_are_recognised_whatever_the_entry_is_called() {
        assert!(is_client_exe("/home/u/Faugus/bnet/drive_c/Program Files (x86)/Battle.net/Battle.net Launcher.exe"));
        assert!(is_client_exe(r"C:\Program Files\Electronic Arts\EA Desktop\EA Desktop\EADesktop.exe"));
        assert!(is_client_exe("/x/Ubisoft Game Launcher/UbisoftConnect.exe"));
        assert!(is_client_exe("/x/Rockstar Games/Launcher/Launcher.exe"));
        assert!(is_client_exe("\"/x/Epic Games/Launcher/Portal/Binaries/Win64/EpicGamesLauncher.exe\""));
        // Games (and generic launcher names elsewhere) are kept.
        assert!(!is_client_exe("/x/Football Life 2026/FL 2026 start.exe"));
        assert!(!is_client_exe("/x/SomeGame/Launcher.exe"));
        assert!(!is_client_exe("/x/Diablo II Resurrected/D2R.exe"));
    }

    #[test]
    fn clients_are_recognised() {
        // The real Lutris rows of a machine with the store clients installed.
        assert!(is_lutris_client("battlenet", "battlenet-standard", "Battle.net"));
        assert!(is_lutris_client("ea-app", "ea-app-standard", "EA App"));
        assert!(is_lutris_client("ubisoft-connect", "ubisoft-connect-latest", "Ubisoft Connect"));
        assert!(is_lutris_client("prism-launcher", "prism-launcher-appimage", "Prism Launcher"));
        assert!(is_lutris_client("x", "", "GOG GALAXY™"));
        assert!(is_lutris_client("", "ea-app-standard", ""));
        assert!(!is_lutris_client("galaxy-on-fire-2", "galaxy-on-fire-2-standard", "Galaxy on Fire 2"));
        assert!(!is_lutris_client("moonlighter", "moonlighter-gog", "Moonlighter"));
        // Games whose slugs start like a client's are kept.
        assert!(!is_lutris_client("origin-of-species", "origin-of-species-gog", "Origin of Species"));
        assert!(!is_lutris_client("the-witcher-3", "the-witcher-3-gog", "The Witcher 3"));
        assert!(!is_lutris_client("steamworld-dig", "steamworld-dig-gog", "SteamWorld Dig"));
        assert!(!is_lutris_client(
            "diablo-ii-resurrected",
            "diablo-ii-resurrected-battlenet",
            "Diablo II: Resurrected"
        ));

        assert!(is_launcher_app_id("io.itch.itch.desktop"));
        assert!(is_launcher_app_id("com.moonlight_stream.Moonlight.desktop"));
        assert!(is_launcher_app_id("org.prismlauncher.PrismLauncher"));
        assert!(!is_launcher_app_id("org.supertuxproject.SuperTux.desktop"));
        assert!(!is_launcher_app_id("runelite.desktop"));
        // The official Minecraft launcher is the game for its users (no instance scanner).
        assert!(!is_launcher_app_id("com.mojang.Minecraft.desktop"));
        assert!(!is_lutris_client("minecraft-launcher", "minecraft-launcher-standard", "Minecraft Launcher"));
    }
}
