//! Bundled fonts: read each font file's family / style names (the OpenType `name` table) and
//! pick the files that match the family a theme asks for.

/// A font file of the theme.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FontFile {
    pub rel: String,
    pub family: String,
    /// "Regular", "Bold", "Light", "SemiBold Italic" …
    pub style: String,
}

impl FontFile {
    fn weight(&self) -> u16 {
        weight_of(&self.style)
    }
    fn italic(&self) -> bool {
        let s = self.style.to_ascii_lowercase();
        s.contains("italic") || s.contains("oblique")
    }
    /// Icon fonts (icofont, Font Awesome …) are never used for text.
    pub fn is_icons(&self) -> bool {
        let f = format!("{} {}", self.family, self.rel).to_ascii_lowercase();
        ["icofont", "icon", "awesome", "symbol", "glyph", "emoji", "material design", "segoe mdl", "segoe fluent"]
            .iter()
            .any(|w| f.contains(w))
    }
}

/// CSS-like weight of a style name.
pub(crate) fn weight_of(style: &str) -> u16 {
    let s = style.to_ascii_lowercase().replace([' ', '-', '_'], "");
    let table: [(&str, u16); 12] = [
        ("extralight", 200),
        ("ultralight", 200),
        ("semibold", 600),
        ("demibold", 600),
        ("extrabold", 800),
        ("ultrabold", 800),
        ("thin", 100),
        ("light", 300),
        ("medium", 500),
        ("bold", 700),
        ("black", 900),
        ("heavy", 900),
    ];
    table.iter().find(|(w, _)| s.contains(w)).map(|(_, v)| *v).unwrap_or(400)
}

fn u16be(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

fn u32be(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

/// (family, style) from a TrueType / OpenType / collection file (its first font).
pub(crate) fn names(data: &[u8]) -> Option<(String, String)> {
    let mut base = 0usize;
    if data.get(..4)? == b"ttcf" {
        base = u32be(data, 12)? as usize;
    }
    let tag = data.get(base..base + 4)?;
    if tag != [0, 1, 0, 0] && tag != b"OTTO" && tag != b"true" {
        return None;
    }
    let n = u16be(data, base + 4)? as usize;
    let mut name_off = None;
    for i in 0..n.min(64) {
        let r = base + 12 + i * 16;
        if data.get(r..r + 4)? == b"name" {
            name_off = Some(u32be(data, r + 8)? as usize);
            break;
        }
    }
    let t = name_off?;
    let count = u16be(data, t + 2)? as usize;
    let strings = t + u16be(data, t + 4)? as usize;
    // id → (score, text); Windows English beats other Windows beats Mac.
    let mut best: [Option<(u8, String)>; 4] = Default::default();
    for i in 0..count.min(512) {
        let r = t + 6 + i * 12;
        let (pid, _eid, lang, id) = (u16be(data, r)?, u16be(data, r + 2)?, u16be(data, r + 4)?, u16be(data, r + 6)?);
        let slot = match id {
            1 => 0,
            2 => 1,
            16 => 2,
            17 => 3,
            _ => continue,
        };
        let len = u16be(data, r + 8)? as usize;
        let off = strings + u16be(data, r + 10)? as usize;
        let Some(raw) = data.get(off..off + len) else { continue };
        let (score, text) = match pid {
            3 => {
                let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                (if lang == 0x409 { 3 } else { 2 }, String::from_utf16_lossy(&units))
            }
            1 => (1, raw.iter().map(|&b| b as char).collect()),
            0 => {
                let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                (1, String::from_utf16_lossy(&units))
            }
            _ => continue,
        };
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        if best[slot].as_ref().is_none_or(|(s, _)| score > *s) {
            best[slot] = Some((score, text));
        }
    }
    let take = |i: usize| best[i].as_ref().map(|(_, t)| t.clone());
    // Typographic names (16/17) group all weights under one family.
    let family = take(2).or_else(|| take(0))?;
    let style = if take(2).is_some() { take(3).or_else(|| take(1)) } else { take(1) };
    Some((family, style.unwrap_or_else(|| "Regular".into())))
}

pub(crate) fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// The family a XAML FontFamily value names: "./Fonts/#Titillium Web" → "Titillium Web",
/// "Segoe UI, Arial" → "Segoe UI", "pack://…/Fonts/#Nunito" → "Nunito".
pub(crate) fn family_of_value(v: &str) -> Option<String> {
    let v = v.trim().trim_matches(|c| c == '{' || c == '}');
    let v = v.strip_prefix("ThemeFile").map(|r| r.trim().trim_matches('\'')).unwrap_or(v);
    // "Fonts/#Name, Fallback" / "Name, Fallback"; pack URIs have commas before the '#'.
    let after = v.rsplit_once('#').map(|(_, f)| f).unwrap_or(v);
    let fam = after.split(',').next()?;
    let fam = fam.trim().trim_matches('\'').trim();
    (!fam.is_empty()).then(|| fam.to_string())
}

/// Playnite's own font keys ("FontTitilliumWebLight") → ("Titillium Web", "Light").
pub(crate) fn family_of_key(key: &str) -> Option<(String, String)> {
    let rest = key.strip_prefix("Font")?;
    if rest.is_empty() || rest.eq_ignore_ascii_case("Family") || rest.starts_with("Size") {
        return None;
    }
    let mut words: Vec<String> = vec![];
    for ch in rest.chars() {
        if ch.is_uppercase() || words.is_empty() {
            words.push(String::new());
        }
        words.last_mut()?.push(ch);
    }
    let styles = ["Thin", "Extra", "Light", "Regular", "Medium", "Semi", "Bold", "Black", "Italic"];
    let split = words.iter().position(|w| styles.contains(&w.as_str())).unwrap_or(words.len());
    let family = words[..split].join(" ");
    let style = words[split..].join("");
    (!family.is_empty()).then(|| (family, if style.is_empty() { "Regular".into() } else { style }))
}

/// The theme's fonts by role.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Picked {
    pub family: String,
    pub regular: Option<FontFile>,
    pub semibold: Option<FontFile>,
    pub bold: Option<FontFile>,
}

/// Files of `family` (or, when it's empty or not bundled and `fallback_to_bundled`, of the
/// family most bundled files have) for the regular / semibold / bold roles. `weight` is the
/// weight the theme asks for its body text (Light for "Titillium Web Light").
pub(crate) fn pick(files: &[FontFile], family: Option<&str>, weight: u16) -> Option<Picked> {
    let text: Vec<&FontFile> = files.iter().filter(|f| !f.is_icons()).collect();
    let wanted = family.map(norm).unwrap_or_default();
    let mut weight = weight;
    let mut of_family: Vec<&FontFile> = text.iter().copied().filter(|f| norm(&f.family) == wanted).collect();
    if of_family.is_empty() && !wanted.is_empty() {
        // "Titillium Web Light" = family "Titillium Web" + style "Light".
        of_family = text
            .iter()
            .copied()
            .filter(|f| {
                let fam = norm(&f.family);
                !fam.is_empty() && wanted.starts_with(&fam) && wanted.len() > fam.len()
            })
            .collect();
        if let Some(f) = of_family.first() {
            weight = family.and_then(|w| w.get(f.family.len()..)).map(weight_of).unwrap_or(weight);
        }
    }
    if of_family.is_empty() {
        if !wanted.is_empty() {
            return None;
        }
        // No family named: the family with the most files.
        let mut counts: Vec<(String, usize)> = vec![];
        for f in &text {
            match counts.iter_mut().find(|(n, _)| *n == f.family) {
                Some(c) => c.1 += 1,
                None => counts.push((f.family.clone(), 1)),
            }
        }
        let (fam, _) = counts.into_iter().max_by_key(|(_, n)| *n)?;
        of_family = text.iter().copied().filter(|f| f.family == fam).collect();
    }
    let upright: Vec<&FontFile> = of_family.iter().copied().filter(|f| !f.italic()).collect();
    let pool = if upright.is_empty() { &of_family } else { &upright };
    let nearest = |w: u16| -> Option<FontFile> {
        pool.iter()
            .min_by_key(|f| (f.weight() as i32 - w as i32).abs() * 2 + (f.weight() < w) as i32)
            .map(|f| (*f).clone())
    };
    let regular = nearest(weight);
    let bold = nearest(700).filter(|b| b.weight() >= 600);
    let semibold = nearest(600).filter(|b| b.weight() >= 500);
    Some(Picked { family: of_family[0].family.clone(), regular, semibold, bold })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal sfnt with just a `name` table (family / style, Windows UTF-16).
    pub(crate) fn fake_font(family: &str, style: &str) -> Vec<u8> {
        let recs: Vec<(u16, Vec<u8>)> = [(1u16, family), (2, style)]
            .iter()
            .map(|(id, s)| (*id, s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()))
            .collect();
        let mut name = vec![];
        name.extend_from_slice(&0u16.to_be_bytes());
        name.extend_from_slice(&(recs.len() as u16).to_be_bytes());
        name.extend_from_slice(&((6 + recs.len() * 12) as u16).to_be_bytes());
        let mut strings = vec![];
        for (id, s) in &recs {
            for v in [3u16, 1, 0x409, *id, s.len() as u16, strings.len() as u16] {
                name.extend_from_slice(&v.to_be_bytes());
            }
            strings.extend_from_slice(s);
        }
        name.extend_from_slice(&strings);
        let mut out = vec![0, 1, 0, 0];
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(b"name");
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&28u32.to_be_bytes());
        out.extend_from_slice(&(name.len() as u32).to_be_bytes());
        out.extend_from_slice(&name);
        out
    }

    fn ff(rel: &str, fam: &str, style: &str) -> FontFile {
        FontFile { rel: rel.into(), family: fam.into(), style: style.into() }
    }

    #[test]
    fn reads_family_and_style() {
        assert_eq!(names(&fake_font("Titillium Web", "Light")), Some(("Titillium Web".into(), "Light".into())));
        assert_eq!(names(b"not a font at all"), None);
    }

    #[test]
    fn family_values_and_keys() {
        assert_eq!(family_of_value("./Fonts/#Titillium Web").as_deref(), Some("Titillium Web"));
        assert_eq!(
            family_of_value("pack://siteoforigin:,,,/Themes/Desktop/X/Fonts/#Nunito").as_deref(),
            Some("Nunito")
        );
        assert_eq!(family_of_value("Segoe UI, Arial").as_deref(), Some("Segoe UI"));
        assert_eq!(family_of_value("{ThemeFile 'Fonts/#Lato'}").as_deref(), Some("Lato"));
        assert_eq!(family_of_key("FontTitilliumWebLight"), Some(("Titillium Web".into(), "Light".into())));
        assert_eq!(family_of_key("FontIcoFont"), Some(("Ico Font".into(), "Regular".into())));
        assert_eq!(family_of_key("FontSize"), None);
    }

    #[test]
    fn picks_weights_of_the_named_family() {
        let files = vec![
            ff("Fonts/icofont.ttf", "IcoFont", "Regular"),
            ff("Fonts/TW-Light.ttf", "Titillium Web", "Light"),
            ff("Fonts/TW-Regular.ttf", "Titillium Web", "Regular"),
            ff("Fonts/TW-Italic.ttf", "Titillium Web", "Italic"),
            ff("Fonts/TW-SemiBold.ttf", "Titillium Web", "SemiBold"),
            ff("Fonts/TW-Bold.ttf", "Titillium Web", "Bold"),
            ff("Fonts/Lato.ttf", "Lato", "Regular"),
        ];
        let p = pick(&files, Some("Titillium Web"), 400).unwrap();
        assert_eq!(p.regular.unwrap().rel, "Fonts/TW-Regular.ttf");
        assert_eq!(p.bold.unwrap().rel, "Fonts/TW-Bold.ttf");
        assert_eq!(p.semibold.unwrap().rel, "Fonts/TW-SemiBold.ttf");
        let p = pick(&files, Some("Titillium Web Light"), 400).unwrap();
        assert_eq!(p.regular.unwrap().rel, "Fonts/TW-Light.ttf");
        // Not bundled → nothing (the report says so).
        assert_eq!(pick(&files, Some("Segoe UI"), 400), None);
        // No family named → the main bundled family, never the icon font.
        assert_eq!(pick(&files, None, 400).unwrap().family, "Titillium Web");
        assert_eq!(pick(&files[..1], None, 400), None);
    }
}
