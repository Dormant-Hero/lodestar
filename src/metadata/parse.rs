//! Pure parsing helpers for the metadata fetcher (no I/O, unit-tested against fixtures).

use crate::model::Metadata;
use serde_json::Value;

/// Max length of a description taken from Steam's long "about the game" text.
pub const ABOUT_MAX_CHARS: usize = 1500;

/// Result of parsing a Steam `appdetails` response.
#[derive(Debug, PartialEq)]
pub enum AppDetails {
    /// `success: true` with data.
    Found(Box<Metadata>),
    /// `success: false` (unknown / region-locked / delisted app): a definitive "nothing".
    NotFound,
    /// `null`, malformed JSON or unexpected shape (Steam returns `null` when rate limiting):
    /// treat as transient.
    Invalid,
}

/// Parse `https://store.steampowered.com/api/appdetails?appids=<appid>`.
pub fn parse_appdetails(body: &[u8], appid: u32) -> AppDetails {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return AppDetails::Invalid;
    };
    let Some(entry) = v.get(appid.to_string()) else {
        return AppDetails::Invalid;
    };
    match entry.get("success").and_then(Value::as_bool) {
        Some(true) => {}
        Some(false) => return AppDetails::NotFound,
        None => return AppDetails::Invalid,
    }
    let Some(d) = entry.get("data").filter(|d| d.is_object()) else {
        return AppDetails::NotFound;
    };
    let text = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("");
    let short = strip_html(text("short_description"));
    let description = if !short.is_empty() {
        Some(short)
    } else {
        let about = strip_html(text("about_the_game"));
        let about = if about.is_empty() { strip_html(text("detailed_description")) } else { about };
        (!about.is_empty()).then(|| truncate_words(&about, ABOUT_MAX_CHARS))
    };
    let join = |k: &str| -> Option<String> {
        let names: Vec<String> = d
            .get(k)?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(|s| decode_entities(s.trim()))
            .filter(|s| !s.is_empty())
            .collect();
        (!names.is_empty()).then(|| names.join(", "))
    };
    let genres = d
        .get("genres")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|g| g.get("description").and_then(Value::as_str))
                .map(|s| decode_entities(s.trim()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let release_year = d.get("release_date").and_then(|r| r.get("date")).and_then(Value::as_str).and_then(parse_year);
    // Store categories: 28 = full controller support, 18 = partial.
    let cats: Vec<u64> = d
        .get("categories")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|c| c.get("id").and_then(Value::as_u64)).collect())
        .unwrap_or_default();
    let controller = if cats.contains(&28) {
        Some(crate::model::ControllerSupport::Full)
    } else if cats.contains(&18) {
        Some(crate::model::ControllerSupport::Partial)
    } else {
        None
    };
    AppDetails::Found(Box::new(Metadata {
        description,
        developer: join("developers"),
        publisher: join("publishers"),
        genres,
        release_year,
        controller,
        players: crate::model::players_from_steam_categories(&cats),
        ..Default::default()
    }))
}

/// SteamGridDB `search/autocomplete/<term>` → id of the first match.
pub fn parse_sgdb_search(body: &[u8]) -> Option<Option<u64>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let data = v.get("data")?.as_array()?;
    Some(data.iter().find_map(|g| g.get("id").and_then(Value::as_u64)))
}

/// SteamGridDB `grids|heroes|logos/game/<id>` → URL of the first (highest-scored) asset.
/// `None` = malformed, `Some(None)` = no assets.
pub fn parse_sgdb_assets(body: &[u8]) -> Option<Option<String>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let data = v.get("data")?.as_array()?;
    Some(
        data.iter()
            .filter_map(|a| a.get("url").and_then(Value::as_str))
            .find(|u| u.starts_with("https://") && !u.ends_with(".webm") && !u.ends_with(".mp4"))
            .map(str::to_string),
    )
}

/// First plausible 4-digit year in a free-form date ("24 Feb, 2022", "Feb 24, 2022", "Q1 2025").
pub fn parse_year(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i - start == 4
                && let Ok(y) = s[start..i].parse::<i32>()
                && (1950..=2100).contains(&y)
            {
                return Some(y);
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Remove HTML tags (block tags and `<br>` become line breaks), decode entities, normalise
/// whitespace. Contents of `<script>`/`<style>` are dropped.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let after = &rest[lt + 1..];
        let Some(gt) = after.find('>') else {
            // Unclosed '<': keep it as text.
            out.push_str(&rest[lt..]);
            rest = "";
            break;
        };
        let tag = after[..gt].trim();
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        rest = &after[gt + 1..];
        match name.as_str() {
            "script" | "style" if !closing => {
                let end = format!("</{name}");
                match rest.to_ascii_lowercase().find(&end) {
                    Some(e) => {
                        rest = &rest[e..];
                        if let Some(g) = rest.find('>') {
                            rest = &rest[g + 1..];
                        }
                    }
                    None => rest = "",
                }
            }
            "br" => out.push('\n'),
            "li" if !closing => out.push_str("\n• "),
            "p" | "div" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "tr" | "blockquote" => {
                out.push('\n')
            }
            "img" | "hr" => out.push(' '),
            _ => {}
        }
    }
    out.push_str(rest);
    normalise_ws(&decode_entities(&out))
}

fn normalise_ws(s: &str) -> String {
    let mut lines: Vec<String> = vec![];
    let mut blank_run = 0;
    for line in s.lines() {
        let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            blank_run += 1;
            if blank_run == 1 && !lines.is_empty() {
                lines.push(String::new());
            }
        } else {
            blank_run = 0;
            lines.push(collapsed);
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Decode named (common subset) and numeric HTML entities. Unknown entities are kept verbatim.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.replace('\u{a0}', " ");
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let semi = after.find(';').filter(|&i| i <= 10);
        let decoded = semi.and_then(|i| entity(&after[..i]).map(|c| (c, i)));
        match decoded {
            Some((c, i)) => {
                out.push(c);
                rest = &after[i + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace('\u{a0}', " ")
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse::<u32>().ok()?,
        };
        return char::from_u32(code).filter(|c| *c != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        "trade" => '™',
        "reg" => '®',
        "copy" => '©',
        "deg" => '°',
        "times" => '×',
        "eacute" => 'é',
        "egrave" => 'è',
        "aacute" => 'á',
        "uuml" => 'ü',
        "ouml" => 'ö',
        "auml" => 'ä',
        "szlig" => 'ß',
        _ => return None,
    })
}

/// Cut to at most `max` chars at a word boundary, appending "…" when shortened.
pub fn truncate_words(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
    let head = &s[..cut];
    let head = match head.rfind(char::is_whitespace) {
        Some(i) if i > cut / 2 => &head[..i],
        _ => head,
    };
    format!("{}…", head.trim_end_matches([',', ';', ':', ' ', '\n', '-']))
}

/// Percent-encode one URL path segment (RFC 3986 unreserved characters stay as they are).
pub fn encode_path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Name as sent to SteamGridDB's search: trademark symbols and bracketed suffixes removed.
pub fn search_name(name: &str) -> String {
    let mut s: String = name.chars().filter(|c| !matches!(c, '™' | '®' | '©')).collect();
    // "Game (2019)" / "Game [GOG]" → "Game"
    for (open, close) in [('(', ')'), ('[', ']')] {
        if let (Some(o), true) = (s.rfind(open), s.trim_end().ends_with(close))
            && o > 0
        {
            s.truncate(o);
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What kind of image a downloaded body is (by magic bytes), as a file extension.
pub fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else {
        None
    }
}
