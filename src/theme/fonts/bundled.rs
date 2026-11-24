//! Fonts compiled into Lodestar, referenced from a theme as `builtin:<name>` (`[fonts]` values or
//! an element's `font`).
//!
//! Titillium Web is Playnite's own typeface (its Fullscreen mode writes in Titillium Web Light,
//! titles in Bold): Playnite ships it, Windows doesn't, so imported Playnite themes and the
//! "Top Bar" layout ask for it by these names. It is under the SIL Open Font License
//! 1.1 (© Accademia di Belle Arti di Urbino): bundled only unmodified (no subsetting), with its
//! OFL.txt next to the files and in THIRD_PARTY_NOTICES; if that OFL.txt declares a Reserved
//! Font Name, the unmodified files may keep the name, a modified copy may not. A build without
//! the files (the list below empty) gives every name a stand-in from the system's fonts, so a
//! theme asking for them never fails.

/// (name, file data). Unmodified OFL files; see the module docs.
static FILES: &[(&str, &[u8])] = &[
    (
        "titillium-web-light",
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/titillium-web/TitilliumWeb-Light.ttf")),
    ),
    (
        "titillium-web",
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/titillium-web/TitilliumWeb-Regular.ttf")),
    ),
    (
        "titillium-web-semibold",
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/titillium-web/TitilliumWeb-SemiBold.ttf")),
    ),
    (
        "titillium-web-bold",
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/titillium-web/TitilliumWeb-Bold.ttf")),
    ),
];

/// The bundled names and the system font alias that stands in for each without the files.
pub const NAMES: [(&str, &str); 4] = [
    ("titillium-web-light", "sans-light"),
    ("titillium-web", "sans"),
    ("titillium-web-semibold", "sans-semibold"),
    ("titillium-web-bold", "sans-bold"),
];

/// The font data of a bundled name (None: not in this build).
pub fn data(name: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim())).map(|(_, d)| *d)
}

/// The system font alias used for a bundled name this build lacks.
pub fn stand_in(name: &str) -> &'static str {
    NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim())).map(|(_, a)| *a).unwrap_or("sans")
}

/// Lodestar includes Titillium Web (Playnite's font).
pub fn has_titillium() -> bool {
    data("titillium-web").is_some()
}

/// `builtin:` name for a Titillium Web weight (CSS-like: 300 light … 700 bold).
pub fn titillium_for_weight(weight: u16) -> &'static str {
    match weight {
        0..=349 => "builtin:titillium-web-light",
        350..=549 => "builtin:titillium-web",
        550..=649 => "builtin:titillium-web-semibold",
        _ => "builtin:titillium-web-bold",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_has_a_stand_in_and_valid_data_when_bundled() {
        for (n, alias) in NAMES {
            assert_eq!(stand_in(n), alias);
            if let Some(d) = data(n) {
                assert!(super::super::sfnt_looks_valid(d), "{n}");
            }
        }
        assert_eq!(stand_in("nope"), "sans");
        assert_eq!(titillium_for_weight(300), "builtin:titillium-web-light");
        assert_eq!(titillium_for_weight(400), "builtin:titillium-web");
        assert_eq!(titillium_for_weight(600), "builtin:titillium-web-semibold");
        assert_eq!(titillium_for_weight(800), "builtin:titillium-web-bold");
        assert_eq!(has_titillium(), !FILES.is_empty());
    }
}
