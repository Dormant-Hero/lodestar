//! ROM file names: tags (region, disc, revision …), tidy display names and the comparison key.
//!
//! No-Intro / Redump names look like `Legend of Zelda, The - Twilight Princess (USA) (Rev 2)`;
//! the display name is `The Legend of Zelda: Twilight Princess` (tags dropped, trailing article
//! moved to the front of each " - " segment, the first " - " turned into ": "). Switch dumps glue
//! junk on (`Pokemon Sword[0100ABF008968000][US][v0]`), GoodTools names carry a release number
//! and short codes (`1636 - Pokemon Fire Red (U)(Squirrels)`).

/// File extensions that come in two parts.
const DOUBLE_EXTS: &[&str] = &[".nkit.iso", ".nkit.rvz", ".nkit.gcz", ".tar.gz"];

/// The file name without its extension ("Game (USA).nkit.iso" → "Game (USA)").
pub fn stem(file_name: &str) -> &str {
    let lower = file_name.to_ascii_lowercase();
    for d in DOUBLE_EXTS {
        if lower.ends_with(d) && file_name.len() > d.len() {
            return &file_name[..file_name.len() - d.len()];
        }
    }
    match file_name.rfind('.') {
        Some(i) if i > 0 && file_name.len() - i <= 7 && !file_name[i + 1..].contains(' ') => &file_name[..i],
        _ => file_name,
    }
}

/// Lower-case extension (".nkit.iso" → "iso").
pub fn ext(file_name: &str) -> String {
    let s = stem(file_name);
    let rest = &file_name[s.len()..];
    rest.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

/// The top-level `(…)` / `[…]` groups of a name, and the name with them removed.
pub fn split_tags(name: &str) -> (String, Vec<String>) {
    let mut title = String::new();
    let mut tags = vec![];
    let mut cur = String::new();
    let mut depth = 0i32;
    for c in name.chars() {
        match c {
            '(' | '[' => {
                if depth == 0 {
                    cur.clear();
                    title.push(' ');
                } else {
                    cur.push(c);
                }
                depth += 1;
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    tags.push(cur.trim().to_string());
                } else {
                    cur.push(c);
                }
            }
            _ if depth > 0 => cur.push(c),
            _ => title.push(c),
        }
    }
    if depth > 0 && !cur.is_empty() {
        tags.push(cur.trim().to_string());
    }
    (collapse(&title), tags)
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

const REGIONS: &[&str] = &[
    "USA",
    "Europe",
    "Japan",
    "World",
    "Asia",
    "Australia",
    "Brazil",
    "Canada",
    "China",
    "France",
    "Germany",
    "Hong Kong",
    "Italy",
    "Korea",
    "Netherlands",
    "Spain",
    "Sweden",
    "Taiwan",
    "UK",
    "Russia",
    "Scandinavia",
    "Latin America",
    "Greece",
    "Norway",
    "Denmark",
    "Finland",
    "Poland",
    "Portugal",
    "India",
    "Mexico",
    "New Zealand",
    "Belgium",
    "Austria",
    "Switzerland",
    "Ireland",
    "Israel",
    "South Africa",
    "Argentina",
    "Croatia",
    "Czech",
    "Hungary",
    "Turkey",
    "United Kingdom",
    "Unknown",
];

/// GoodTools short region codes (case-sensitive) and Switch-style bracket codes.
const SHORT_REGIONS: &[(&str, &str)] = &[
    ("U", "USA"),
    ("E", "Europe"),
    ("J", "Japan"),
    ("W", "World"),
    ("UE", "USA, Europe"),
    ("EU", "Europe"),
    ("JU", "Japan, USA"),
    ("JUE", "Japan, USA, Europe"),
    ("UJ", "USA, Japan"),
    ("F", "France"),
    ("G", "Germany"),
    ("S", "Spain"),
    ("I", "Italy"),
    ("K", "Korea"),
    ("A", "Australia"),
    ("B", "Brazil"),
    ("C", "China"),
    ("Ch", "China"),
    ("US", "USA"),
    ("JP", "Japan"),
    ("EUR", "Europe"),
    ("JPN", "Japan"),
    ("KR", "Korea"),
];

/// The region a tag stands for ("USA, Europe", GoodTools "U", Switch "US"), as No-Intro names.
pub fn region_tag(tag: &str) -> Option<String> {
    if let Some((_, r)) = SHORT_REGIONS.iter().find(|(c, _)| *c == tag) {
        return Some(r.to_string());
    }
    // Every comma-separated part must be a region.
    let names: Option<Vec<&str>> =
        tag.split(',').map(|p| REGIONS.iter().find(|r| r.eq_ignore_ascii_case(p.trim())).copied()).collect();
    names.map(|v| v.join(", "))
}

/// The first region tag of a name.
pub fn region(name: &str) -> Option<String> {
    split_tags(name).1.iter().find_map(|t| region_tag(t))
}

/// The region names of a name's first region tag, sorted ("USA, Europe" → ["Europe", "USA"]).
pub fn region_set(region: &str) -> Vec<String> {
    let mut v: Vec<String> = region.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    v.sort();
    v
}

/// The name has a No-Intro / GoodTools / Switch region tag (a dump-set name, not a free-form one).
pub fn has_region_tag(name: &str) -> bool {
    region(name).is_some()
}

/// Disc number of a multi-disc name: `(Disc 2)`, `(Disc 2 of 3)`, `(CD1)`, `(Disk B)`,
/// `(Side B)`, or a trailing ` CD2` / `_Disc2`.
pub fn disc(name: &str) -> Option<u32> {
    let (_, tags) = split_tags(name);
    for t in &tags {
        if let Some(n) = disc_tag(t) {
            return Some(n);
        }
    }
    let lower = name.to_ascii_lowercase();
    for kw in ["disc", "disk", "cd"] {
        if let Some(i) = lower.rfind(kw) {
            let before = lower[..i].chars().last();
            let rest = lower[i + kw.len()..].trim_start();
            if matches!(before, Some(' ' | '_' | '-')) && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                return rest.parse().ok();
            }
        }
    }
    None
}

fn disc_tag(tag: &str) -> Option<u32> {
    let lower = tag.to_ascii_lowercase();
    let rest = ["disc", "disk", "cd", "side"].iter().find_map(|kw| lower.strip_prefix(kw))?.trim_start();
    let rest = rest.split(" of ").next().unwrap_or(rest).trim();
    if let Ok(n) = rest.parse::<u32>() {
        return Some(n);
    }
    match rest {
        "a" => Some(1),
        "b" => Some(2),
        "c" => Some(3),
        "d" => Some(4),
        _ => None,
    }
}

/// Is this tag a disc tag?
pub fn is_disc_tag(tag: &str) -> bool {
    disc_tag(tag).is_some()
}

/// What grouping discs compares: the title plus every tag except the disc tag.
pub fn disc_group_key(name: &str) -> String {
    let (title, tags) = split_tags(name);
    let mut title = title;
    // Unbracketed " CD2" / "_Disc2".
    if disc(name).is_some() && !tags.iter().any(|t| is_disc_tag(t)) {
        let lower = title.to_ascii_lowercase();
        if let Some(i) = ["disc", "disk", "cd"].iter().filter_map(|k| lower.rfind(k)).max() {
            title.truncate(i);
        }
    }
    let rest: Vec<&str> = tags.iter().filter(|t| !is_disc_tag(t)).map(String::as_str).collect();
    format!("{}|{}", key(&title), rest.join("|").to_lowercase())
}

/// The key of a disc set across differing free-form tags: the title and only the tags that
/// tell releases apart (region, languages, version, pre-release …). "Gran Turismo 2 (Gran
/// Turismo Mode) (Disc 1)" and "Gran Turismo 2 (Arcade Mode) (Disc 2)" share it; "(USA)" and
/// "(Europe)" discs don't.
pub fn disc_release_key(name: &str) -> String {
    let (title, tags) = split_tags(name);
    let mut title = title;
    if disc(name).is_some() && !tags.iter().any(|t| is_disc_tag(t)) {
        let lower = title.to_ascii_lowercase();
        if let Some(i) = ["disc", "disk", "cd"].iter().filter_map(|k| lower.rfind(k)).max() {
            title.truncate(i);
        }
    }
    let rest: Vec<&str> = tags.iter().filter(|t| !is_disc_tag(t) && release_tag(t)).map(String::as_str).collect();
    format!("{}|{}", key(&title), rest.join("|").to_lowercase())
}

/// A tag that makes a different release: region, languages ("En,Fr,De"), version ("Rev 1",
/// "v1.1"), a pre-release / unofficial dump.
fn release_tag(tag: &str) -> bool {
    if region_tag(tag).is_some() {
        return true;
    }
    let l = tag.trim().to_ascii_lowercase();
    let lang = l.split(',').all(|p| {
        let p = p.trim();
        (p.len() == 2 || p.len() == 5 && p.as_bytes()[2] == b'-')
            && p.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
    });
    let version = l.starts_with("rev") || (l.starts_with('v') && l[1..].starts_with(|c: char| c.is_ascii_digit()));
    let status = ["beta", "proto", "demo", "sample", "kiosk", "debug", "promo", "unl", "pirate", "hack"]
        .iter()
        .any(|b| l.starts_with(b))
        || l == "alt"
        || l.starts_with("alt ");
    lang || version || status
}

/// Tags that mark a pre-release / unofficial dump (ranked last).
pub fn status_penalty(name: &str) -> u32 {
    let (_, tags) = split_tags(name);
    let mut p = 0;
    for t in tags {
        let l = t.to_ascii_lowercase();
        let bad = [
            "beta",
            "proto",
            "demo",
            "sample",
            "kiosk",
            "debug",
            "promo",
            "unl",
            "pirate",
            "aftermarket",
            "homebrew",
            "program",
            "hack",
            "t-",
            "t+",
            "bios",
        ];
        if bad.iter().any(|b| l.starts_with(b))
            || (l.len() == 10 && l.as_bytes()[4] == b'-' && l[..4].chars().all(|c| c.is_ascii_digit()))
        {
            p += 20;
        }
        if l.starts_with("rev") || (l.starts_with('v') && l[1..].starts_with(|c: char| c.is_ascii_digit())) {
            p += 1;
        }
    }
    p
}

/// Region preference: USA > World > Europe > other > Japan (on the tags only).
pub fn region_rank(name: &str) -> u32 {
    match region(name) {
        Some(r) if r.contains("USA") => 0,
        Some(r) if r.contains("World") => 1,
        Some(r) if r.contains("Europe") => 2,
        Some(r) if r == "Japan" => 4,
        _ => 3,
    }
}

/// A trailing ", The" style article per " - " segment, moved to the front.
const ARTICLES: &[&str] = &[
    "The", "A", "An", "Die", "Der", "Das", "Le", "La", "Les", "L'", "El", "Los", "Las", "Il", "Lo", "Gli", "Os", "As",
    "O",
];

fn move_article(seg: &str) -> String {
    for a in ARTICLES {
        if let Some(head) = seg.strip_suffix(&format!(", {a}")) {
            return if a.ends_with('\'') { format!("{a}{head}") } else { format!("{a} {head}") };
        }
    }
    seg.to_string()
}

/// The title with tags removed and the release number / underscores cleaned up (no article move).
fn bare_title(name: &str) -> String {
    let mut s = name.trim().to_string();
    if !s.contains(' ') && s.contains('_') {
        s = s.replace('_', " ");
    }
    let (title, tags) = split_tags(&s);
    let mut title = title;
    // GoodTools release number "1636 - Name (U)" (only with a GoodTools region code, so
    // "1943 - The Battle of Midway" keeps its number).
    let goodtools = tags.iter().any(|t| SHORT_REGIONS.iter().any(|(c, _)| c == t && c.len() <= 3));
    if goodtools
        && let Some((num, rest)) = title.split_once(" - ")
        && (3..=4).contains(&num.len())
        && num.chars().all(|c| c.is_ascii_digit())
    {
        title = rest.to_string();
    }
    title
        .trim_end_matches(|c: char| c == '-' || c == '_' || c == '.' || c == '+' || c.is_whitespace())
        .trim()
        .to_string()
}

/// The display name of a ROM / dump name. `official`: the name comes from the hash database or
/// is a dump-set name (has a region tag), so the "Pokemon" → "Pokémon" fix-up applies (free-form
/// names, e.g. ROM hacks, are kept as their author wrote them).
pub fn tidy(name: &str, official: bool) -> String {
    let mut title = bare_title(name);
    if official {
        for (from, to) in [("Pokemon - ", "Pokémon "), ("Pokemon ", "Pokémon ")] {
            if let Some(rest) = title.strip_prefix(from) {
                title = format!("{to}{rest}");
                break;
            }
        }
    }
    let segs: Vec<String> = title.split(" - ").map(move_article).collect();
    let mut out = String::new();
    for (i, s) in segs.iter().enumerate() {
        match i {
            0 => {}
            1 => out.push_str(": "),
            _ => out.push_str(" - "),
        }
        out.push_str(s);
    }
    let out = collapse(&out);
    if out.is_empty() { collapse(name) } else { out }
}

/// Comparison key: lower-case letters and digits (accents folded), "&" = "and", articles moved.
pub fn key(name: &str) -> String {
    let t = tidy(name, false);
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '&' => out.push_str(" and "),
            '\'' | '’' => {}
            c if c.is_alphanumeric() => out.push(fold(c.to_lowercase().next().unwrap_or(c))),
            _ => out.push(' '),
        }
    }
    collapse(&out)
}

fn fold(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        'ý' | 'ÿ' => 'y',
        c => c,
    }
}

/// The file name says nothing about the game ("rom.gba", "BPRE.gba", "game", "1234").
pub fn uninformative(name: &str) -> bool {
    let k = key(name);
    let squashed: String = k.chars().filter(|c| !c.is_whitespace()).collect();
    let raw = bare_title(name);
    let code_chars = raw.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'));
    // A game code / serial: "BPRE", "GM8E01", "SLUS-20946", "SLUS_209.46", "CTR-P-BNEA".
    let serial_shaped = code_chars
        && ((matches!(raw.len(), 4 | 6) && raw.chars().all(|c| c.is_ascii_alphanumeric()))
            || ((4..=12).contains(&raw.len())
                && raw.chars().any(|c| c.is_ascii_digit())
                && raw.chars().any(|c| c.is_ascii_uppercase())));
    squashed.len() <= 3
        || ["rom", "game", "roms", "untitled", "image", "disc", "disk"].contains(&k.as_str())
        || serial_shaped
}

/// The 16-hex Switch title id in a name (`[0100ABF008968000]`).
pub fn switch_title_id(name: &str) -> Option<String> {
    split_tags(name)
        .1
        .into_iter()
        .find(|t| t.len() == 16 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|t| t.to_ascii_uppercase())
}

/// An update / DLC by its name: `(Update)`, `[UPD]`, `(Patch)` / `(DLC)`, `[DLC]`.
pub fn addon_tag(name: &str) -> Option<super::Role> {
    for t in split_tags(name).1 {
        let l = t.to_ascii_lowercase();
        if ["update", "upd", "patch"].contains(&l.as_str()) || l.starts_with("update ") {
            return Some(super::Role::Update);
        }
        if ["dlc", "add-on", "addon"].contains(&l.as_str()) {
            return Some(super::Role::Dlc);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_and_extensions() {
        assert_eq!(stem("Game (USA).nkit.iso"), "Game (USA)");
        assert_eq!(ext("Game (USA).nkit.iso"), "iso");
        assert_eq!(stem("Pokemon Radical Red v4.1.gba"), "Pokemon Radical Red v4.1");
        assert_eq!(ext("x.Z64"), "z64");
        assert_eq!(ext("Pokemon Glazed.xdelta"), "xdelta", "patch files are recognised");
        assert_eq!(stem("Dr. Mario"), "Dr. Mario");
        assert_eq!(
            stem("Legend of Zelda, The - Breath of the Wild (USA)"),
            "Legend of Zelda, The - Breath of the Wild (USA)"
        );
    }

    #[test]
    fn tidy_names() {
        let t = |s| tidy(s, has_region_tag(s));
        assert_eq!(
            t("Legend of Zelda, The - Twilight Princess (USA) (En,Fr,Es) (Rev 2)"),
            "The Legend of Zelda: Twilight Princess"
        );
        assert_eq!(t("Grand Theft Auto - San Andreas (USA) (v3.00)"), "Grand Theft Auto: San Andreas");
        assert_eq!(t("Pokemon - Red Version (USA, Europe) (SGB Enhanced)"), "Pokémon Red Version");
        assert_eq!(
            t("Pokemon - Yellow Version - Special Pikachu Edition (USA, Europe) (CGB+SGB Enhanced)"),
            "Pokémon Yellow Version: Special Pikachu Edition"
        );
        assert_eq!(t("Pokemon Violet [01008F6008C5E000][v0][US](nsw2u.com)"), "Pokémon Violet");
        assert_eq!(
            t("Pokémon Mystery Dungeon Rescue Team DX[01003D200BAA2000][US][v0]"),
            "Pokémon Mystery Dungeon Rescue Team DX"
        );
        assert_eq!(t("Fire Emblem Three Houses [010055D009F78000][v0]"), "Fire Emblem Three Houses");
        assert_eq!(t("Pokemon Shining Pearl [010018E011D92000][v0](nsw2u.xyz)"), "Pokemon Shining Pearl");
        // ROM hacks keep their names.
        assert_eq!(t("Pokemon Radical Red v4.1"), "Pokemon Radical Red v4.1");
        assert_eq!(t("pokeprism (v0.95.0254)"), "pokeprism");
        assert_eq!(t("Pokémon Renegade Platinum"), "Pokémon Renegade Platinum");
        // GoodTools number only with a GoodTools code.
        assert_eq!(t("1636 - Pokemon Fire Red (U)(Squirrels)"), "Pokémon Fire Red");
        assert_eq!(t("1943 - The Battle of Midway"), "1943: The Battle of Midway");
        assert_eq!(t("Bug's Life, A (USA)"), "A Bug's Life");
        assert_eq!(t("Super_Mario_Land"), "Super Mario Land");
        assert_eq!(t("Crash Bandicoot - The Wrath of Cortex (USA) (v1.01)"), "Crash Bandicoot: The Wrath of Cortex");
    }

    #[test]
    fn tags_regions_discs() {
        assert_eq!(region("Pokemon - Gold Version (USA, Europe) (SGB Enhanced)").as_deref(), Some("USA, Europe"));
        assert_eq!(region("Game (U)(Squirrels)").as_deref(), Some("USA"));
        assert_eq!(region("Pokemon Sword[0100ABF008968000][US][v0]").as_deref(), Some("USA"));
        assert_eq!(region("Game (En,Fr,Es)"), None);
        assert_eq!(disc("Final Fantasy VII (USA) (Disc 2)"), Some(2));
        assert_eq!(disc("Game (Disc 1 of 3) (USA)"), Some(1));
        assert_eq!(disc("Game CD2"), Some(2));
        assert_eq!(disc("Persona 2 - Eternal Punishment (USA)"), None);
        assert_eq!(disc_group_key("FF7 (USA) (Disc 1)"), disc_group_key("FF7 (USA) (Disc 3)"));
        assert_ne!(disc_group_key("FF7 (USA) (Disc 1)"), disc_group_key("FF7 (Europe) (Disc 2)"));
        assert_eq!(switch_title_id("Pokemon Sword[0100abf008968000][US][v0]").as_deref(), Some("0100ABF008968000"));
        assert_eq!(
            addon_tag("Legend of Zelda, The - Breath of the Wild (USA) (Update)"),
            Some(super::super::Role::Update)
        );
        assert_eq!(addon_tag("X (USA) (DLC)"), Some(super::super::Role::Dlc));
        assert!(status_penalty("Game (USA) (Beta)") >= 20);
        assert_eq!(status_penalty("Game (USA)"), 0);
        assert!(region_rank("A (USA)") < region_rank("A (Japan)"));
    }

    #[test]
    fn keys_and_uninformative_names() {
        assert_eq!(key("Pokémon XD: Gale of Darkness (USA)"), key("Pokemon XD - Gale of Darkness (USA)"));
        assert_eq!(key("Legend of Zelda, The - Twilight Princess"), "the legend of zelda twilight princess");
        assert_ne!(key("Pokemon Radical Red v4.1"), key("Pokemon - FireRed Version (USA, Europe)"));
        assert!(uninformative("BPRE"));
        assert!(uninformative("rom"));
        assert!(uninformative("SLUS-20946"));
        assert!(!uninformative("Pokemon Unbound"));
        assert!(!uninformative("Tetris"));
    }
}
