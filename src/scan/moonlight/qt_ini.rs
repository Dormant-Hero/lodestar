//! Minimal reader for Qt's `QSettings` INI files (Moonlight.conf, Prism / MultiMC `instance.cfg`
//! and `prismlauncher.cfg`). Declared in moonlight.rs as `moonlight::qt_ini`.
//!
//! Format notes (QSettings `IniFormat`):
//! * `[General]` holds top-level keys; other sections prefix their keys (`[hosts]` + `1\hostname`
//!   → `hosts/1/hostname`). `\` in keys is the group separator; keys are %-encoded.
//! * Values: surrounding whitespace dropped; `"…"` quotes (used for text with `,` `;` or edge
//!   spaces); C-style escapes `\n \t \" \\ \xHHHH \ooo`; an unquoted `,` separates a string list
//!   (joined back with ", "). `@ByteArray(…)` / `@Variant(…)` are kept as written; `@@` → `@`.

use std::collections::HashMap;

/// Parse to `group/key` → value (General keys without a prefix).
pub fn parse(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = decode_key(name.trim());
            section = if name.eq_ignore_ascii_case("general") { String::new() } else { name };
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = decode_key(k.trim());
        if key.is_empty() {
            continue;
        }
        let full = if section.is_empty() { key } else { format!("{section}/{key}") };
        out.insert(full, unescape_value(v));
    }
    out
}

fn decode_key(k: &str) -> String {
    crate::scan::desktop::common::percent_decode(&k.replace('\\', "/"), false)
}

/// One INI value as text.
pub fn unescape_value(v: &str) -> String {
    let v = v.trim();
    if let Some(rest) = v.strip_prefix("@@") {
        return format!("@{rest}");
    }
    if v.starts_with("@ByteArray(") || v.starts_with("@Variant(") {
        return v.to_string();
    }
    let mut items: Vec<String> = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = v.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                items.push(cur.trim().to_string());
                cur.clear();
            }
            '\\' => match chars.next() {
                Some('n') => cur.push('\n'),
                Some('t') => cur.push('\t'),
                Some('r') => cur.push('\r'),
                Some('a') => cur.push('\x07'),
                Some('b') => cur.push('\x08'),
                Some('f') => cur.push('\x0c'),
                Some('v') => cur.push('\x0b'),
                Some('x') => {
                    let mut n = 0u32;
                    let mut digits = 0;
                    while digits < 4
                        && let Some(d) = chars.peek().and_then(|c| c.to_digit(16))
                    {
                        n = n * 16 + d;
                        digits += 1;
                        chars.next();
                    }
                    if let Some(ch) = char::from_u32(n).filter(|_| digits > 0) {
                        cur.push(ch);
                    }
                }
                Some(d @ '0'..='7') => {
                    let mut n = d.to_digit(8).unwrap_or(0);
                    for _ in 0..2 {
                        match chars.peek().and_then(|c| c.to_digit(8)) {
                            Some(x) => {
                                n = n * 8 + x;
                                chars.next();
                            }
                            None => break,
                        }
                    }
                    if let Some(ch) = char::from_u32(n) {
                        cur.push(ch);
                    }
                }
                Some(other) => cur.push(other),
                None => {}
            },
            c => cur.push(c),
        }
    }
    if items.is_empty() {
        // Unquoted surrounding spaces are not part of the value; quoted ones are.
        return if v.starts_with('"') { cur } else { cur.trim().to_string() };
    }
    items.push(cur.trim().to_string());
    items.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_keys_and_values() {
        let ini = parse(
            "[General]\nname=\"Better, Faster\"\nplain = hello world \nnotes=a\\nb\\x4e2d\\101\n\n; comment\n\
             [hosts]\n1\\hostname=GAMING-PC\n1\\apps\\2\\name=Steam Big Picture\nsize=1\n1\\srvcert=@ByteArray(abc)\n\
             [my%20group]\nlist=a, b ,c\nquoted=\"  x  \"\n",
        );
        assert_eq!(ini["name"], "Better, Faster");
        assert_eq!(ini["plain"], "hello world");
        assert_eq!(ini["notes"], "a\nb\u{4e2d}A");
        assert_eq!(ini["hosts/1/hostname"], "GAMING-PC");
        assert_eq!(ini["hosts/1/apps/2/name"], "Steam Big Picture");
        assert_eq!(ini["hosts/size"], "1");
        assert_eq!(ini["hosts/1/srvcert"], "@ByteArray(abc)");
        assert_eq!(ini["my group/list"], "a, b, c");
        assert_eq!(ini["my group/quoted"], "  x  ");
        // No section header at all (old MultiMC instance.cfg).
        assert_eq!(parse("name=Old\n")["name"], "Old");
    }
}
