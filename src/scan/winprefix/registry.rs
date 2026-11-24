//! Reader for Wine's text registry files (`system.reg` = HKEY_LOCAL_MACHINE, `user.reg` =
//! HKEY_CURRENT_USER). Format:
//!
//! ```text
//! WINE REGISTRY Version 2
//! ;; All keys relative to \\Machine
//! #arch=win64
//!
//! [Software\\Wow6432Node\\Ubisoft\\Launcher] 1700000000
//! #time=1d9a1b2c3d4e5f6
//! @="default value"
//! "InstallDir"="C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher\\"
//! "Count"=dword:0000000a
//! "Path"=str(2):"%ProgramFiles%\\x"
//! "Old"=hex(2):25,00,50,00,00,00
//! "Blob"=hex:01,02,\
//!   03,04
//! ```
//!
//! Key names and string values escape `\` and `"` with a backslash; `\xHHHH` is a UTF-16 code unit,
//! `\n` / `\r` / `\t` / `\0` the usual controls. Long hex values continue on the next line after a
//! trailing `\`. Lookups are case-insensitive (the Windows registry is).

use std::collections::HashMap;
use std::path::Path;

/// Which registry file of a prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hive {
    /// HKEY_LOCAL_MACHINE (`system.reg`).
    System,
    /// HKEY_CURRENT_USER (`user.reg`).
    User,
}

impl Hive {
    pub fn file_name(self) -> &'static str {
        match self {
            Hive::System => "system.reg",
            Hive::User => "user.reg",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegValue {
    /// REG_SZ / REG_EXPAND_SZ (`"…"`, `str(2):"…"`, `hex(2):` decoded from UTF-16).
    Str(String),
    /// REG_DWORD.
    Dword(u32),
    /// REG_MULTI_SZ (`str(7):` / `hex(7):`).
    Multi(Vec<String>),
    /// Anything else (`hex:`, `hex(b):` …) as raw bytes.
    Bytes(Vec<u8>),
}

impl RegValue {
    /// The value as text: strings as-is, DWORDs in decimal, multi-strings joined with `\n`, bytes
    /// as lowercase hex.
    #[cfg(test)]
    pub fn as_text(&self) -> String {
        match self {
            RegValue::Str(s) => s.clone(),
            RegValue::Dword(d) => d.to_string(),
            RegValue::Multi(v) => v.join("\n"),
            RegValue::Bytes(b) => b.iter().map(|x| format!("{x:02x}")).collect(),
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            RegValue::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegKey {
    /// Path as written in the file (unescaped, `\`-separated, original case).
    pub path: String,
    /// Values in file order; the default value is named `""` (written `@`).
    pub values: Vec<(String, RegValue)>,
}

impl RegKey {
    /// Value by name, case-insensitive.
    pub fn get(&self, name: &str) -> Option<&RegValue> {
        self.values.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v)
    }
    /// Non-empty, trimmed string value by name.
    pub fn get_str(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(RegValue::as_str).map(str::trim).filter(|s| !s.is_empty())
    }
    /// Last path component.
    pub fn name(&self) -> &str {
        self.path.rsplit('\\').next().unwrap_or("")
    }
}

/// A parsed registry file (or the keys of it a filter kept).
#[derive(Clone, Debug, Default)]
pub struct Registry {
    /// Lowercased key path → key.
    keys: HashMap<String, RegKey>,
}

/// Normalise a key path for lookup: `HKLM\…` / `HKEY_LOCAL_MACHINE\…` / `HKCU\…` prefixes dropped,
/// `/` accepted as a separator, duplicate / trailing separators removed, lowercased.
pub fn norm_key(path: &str) -> String {
    let mut out = String::new();
    norm_key_into(path, &mut out);
    out
}

/// [`norm_key`] into a reused buffer (the parser calls this for every key header of a hive).
fn norm_key_into(path: &str, out: &mut String) {
    out.clear();
    let mut parts = path.split(['\\', '/']).filter(|p| !p.is_empty()).peekable();
    if parts.peek().is_some_and(|p| {
        ["hklm", "hkey_local_machine", "hkcu", "hkey_current_user"].iter().any(|r| p.eq_ignore_ascii_case(r))
    }) {
        parts.next();
    }
    for (i, p) in parts.enumerate() {
        if i > 0 {
            out.push('\\');
        }
        if p.is_ascii() {
            out.extend(p.chars().map(|c| c.to_ascii_lowercase()));
        } else {
            out.push_str(&p.to_lowercase());
        }
    }
}

impl Registry {
    /// Parse the whole file.
    #[cfg(test)]
    pub fn parse(text: &str) -> Registry {
        Registry::parse_filtered(text, |_| true)
    }

    /// Parse only keys whose normalised (lowercased) path `keep` accepts. Values of other keys are
    /// skipped without decoding, so a 10 MB `system.reg` costs little when only a few keys matter.
    #[cfg(test)]
    pub fn parse_filtered(text: &str, keep: impl Fn(&str) -> bool) -> Registry {
        Registry::parse_prefiltered(text, |_| true, keep)
    }

    /// [`Registry::parse_filtered`] with a cheap first check on each raw header (the text after
    /// `[`, still escaped: `Software\\Wow6432Node\\Ubisoft] 1700000000`). Only headers `raw_keep`
    /// accepts are unescaped and given to `keep`; it must accept every header `keep` could want.
    pub fn parse_prefiltered(text: &str, raw_keep: impl Fn(&str) -> bool, keep: impl Fn(&str) -> bool) -> Registry {
        let mut reg = Registry::default();
        let mut cur: Option<RegKey> = None;
        let mut lines = text.lines();
        // Reused for every header: a Proton prefix's system.reg has tens of thousands of keys,
        // and only a handful are kept.
        let (mut name, mut norm) = (String::new(), String::new());
        while let Some(raw) = lines.next() {
            let line = raw.trim_end_matches('\r');
            if let Some(rest) = line.strip_prefix('[') {
                if let Some(k) = cur.take() {
                    reg.insert(k);
                }
                if !raw_keep(rest) || !key_name_into(rest, &mut name) {
                    continue;
                }
                norm_key_into(&name, &mut norm);
                if keep(&norm) {
                    cur = Some(RegKey { path: name.clone(), values: vec![] });
                }
                continue;
            }
            let Some(key) = cur.as_mut() else { continue };
            if !(line.starts_with('"') || line.starts_with('@')) {
                continue;
            }
            // Hex values continue on following lines after a trailing backslash.
            let mut full = line.to_string();
            while full.ends_with('\\') && is_hex_value(&full) {
                full.pop();
                match lines.next() {
                    Some(next) => full.push_str(next.trim()),
                    None => break,
                }
            }
            if let Some((name, value)) = parse_value_line(&full) {
                key.values.push((name, value));
            }
        }
        if let Some(k) = cur.take() {
            reg.insert(k);
        }
        reg
    }

    /// Read `<prefix>/<hive file>`; a missing / unreadable file is an empty registry.
    #[cfg(test)]
    pub fn load(prefix: &Path, hive: Hive, keep: impl Fn(&str) -> bool) -> Registry {
        Registry::load_prefiltered(prefix, hive, |_| true, keep)
    }

    /// [`Registry::load`] with a raw-header pre-check (see [`Registry::parse_prefiltered`]).
    pub fn load_prefiltered(
        prefix: &Path,
        hive: Hive,
        raw_keep: impl Fn(&str) -> bool,
        keep: impl Fn(&str) -> bool,
    ) -> Registry {
        match crate::scan::util::read_text(prefix.join(hive.file_name())) {
            Ok(text) => Registry::parse_prefiltered(&text, raw_keep, keep),
            Err(_) => Registry::default(),
        }
    }

    fn insert(&mut self, k: RegKey) {
        let norm = norm_key(&k.path);
        match self.keys.get_mut(&norm) {
            // The same key twice (never written by Wine, but cheap to tolerate): merge.
            Some(existing) => existing.values.extend(k.values),
            None => {
                self.keys.insert(norm, k);
            }
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Every key with its values, for comparing two parses in tests.
    #[cfg(test)]
    pub fn debug_keys(&self) -> Vec<String> {
        self.keys.iter().map(|(k, v)| format!("{k} {v:?}")).collect()
    }

    /// Every key kept, in no particular order.
    pub fn keys(&self) -> impl Iterator<Item = &RegKey> {
        self.keys.values()
    }

    /// Key by path (case-insensitive; `HKLM\` etc. allowed).
    pub fn key(&self, path: &str) -> Option<&RegKey> {
        self.keys.get(&norm_key(path))
    }

    /// One value of a key.
    #[cfg(test)]
    pub fn value(&self, path: &str, name: &str) -> Option<&RegValue> {
        self.key(path)?.get(name)
    }

    /// Non-empty string value of a key.
    pub fn get_str(&self, path: &str, name: &str) -> Option<&str> {
        self.key(path)?.get_str(name)
    }

    /// Direct sub-keys of `path` that hold values (Wine writes every key with values as its own
    /// section, so a parent without values may have no section of its own), sorted by path.
    pub fn subkeys(&self, path: &str) -> Vec<&RegKey> {
        let base = norm_key(path);
        let want = format!("{base}\\");
        let mut v: Vec<&RegKey> = self
            .keys
            .iter()
            .filter(|(k, _)| k.strip_prefix(&want).is_some_and(|rest| !rest.is_empty() && !rest.contains('\\')))
            .map(|(_, v)| v)
            .collect();
        v.sort_by_key(|k| k.path.to_lowercase());
        v
    }

    /// Paths of the direct children of `path`, also those that only exist as the parent of a
    /// deeper key (Wine writes no section for a key without values), sorted, original case.
    pub fn child_paths(&self, path: &str) -> Vec<String> {
        let base = norm_key(path);
        let want = format!("{base}\\");
        let n = base.split('\\').count() + 1;
        let mut out: Vec<String> = vec![];
        for (k, v) in &self.keys {
            if !k.starts_with(&want) {
                continue;
            }
            // Components of the file's path line up with the normalised key's.
            let p = v.path.split('\\').filter(|c| !c.is_empty()).take(n).collect::<Vec<_>>().join("\\");
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&p)) {
                out.push(p);
            }
        }
        out.sort_by_key(|p| p.to_lowercase());
        out
    }
}

/// `Software\\X\\Y] 1700000000` → `Software\X\Y`.
/// The key name of a header line (after its `[`) into `out`; false when malformed or empty.
fn key_name_into(rest: &str, out: &mut String) -> bool {
    // The name ends at the first `]` that isn't escaped.
    out.clear();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(n) => out.push(n),
                None => return false,
            },
            ']' => return !out.is_empty(),
            c => out.push(c),
        }
    }
    false
}

fn is_hex_value(line: &str) -> bool {
    // After the name's closing quote: `=hex:` / `=hex(N):`.
    line.contains("=hex:") || line.contains("=hex(")
}

/// Unescape a quoted Wine string body (without the quotes).
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut units: Vec<u16> = vec![];
    let mut it = s.chars().peekable();
    let flush = |units: &mut Vec<u16>, out: &mut String| {
        if !units.is_empty() {
            out.push_str(&String::from_utf16_lossy(units));
            units.clear();
        }
    };
    while let Some(c) = it.next() {
        if c != '\\' {
            flush(&mut units, &mut out);
            out.push(c);
            continue;
        }
        match it.next() {
            Some('x') => {
                let mut hex = String::new();
                while hex.len() < 4 && it.peek().is_some_and(char::is_ascii_hexdigit) {
                    hex.push(it.next().unwrap_or('0'));
                }
                // UTF-16 code units: surrogate pairs come as two `\x` escapes.
                match u16::from_str_radix(&hex, 16) {
                    Ok(u) => units.push(u),
                    Err(_) => {
                        flush(&mut units, &mut out);
                        out.push('x');
                    }
                }
                continue;
            }
            Some(other) => {
                flush(&mut units, &mut out);
                match other {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '0' => out.push('\0'),
                    c => out.push(c),
                }
            }
            None => flush(&mut units, &mut out),
        }
    }
    flush(&mut units, &mut out);
    out
}

/// The body of a quoted string starting right after the opening quote: returns the raw body and
/// the rest after the closing quote.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let mut esc = false;
    for (i, c) in s.char_indices() {
        match c {
            '\\' if !esc => esc = true,
            '"' if !esc => return Some((&s[..i], &s[i + 1..])),
            _ => esc = false,
        }
    }
    None
}

/// `"Name"=<value>` / `@=<value>` → (name, value).
pub fn parse_value_line(line: &str) -> Option<(String, RegValue)> {
    let (name, rest) = if let Some(r) = line.strip_prefix('@') {
        (String::new(), r)
    } else {
        let (n, r) = quoted(line.strip_prefix('"')?)?;
        (unescape(n), r)
    };
    let val = rest.trim_start().strip_prefix('=')?.trim();
    Some((name, parse_data(val)?))
}

fn parse_data(val: &str) -> Option<RegValue> {
    if let Some(v) = val.strip_prefix('"') {
        let s = unescape(quoted(v).map_or(v, |(b, _)| b));
        return Some(RegValue::Str(s.trim_end_matches('\0').to_string()));
    }
    if let Some(rest) = val.strip_prefix("str(") {
        let (ty, rest) = rest.split_once("):")?;
        let body = rest.trim().strip_prefix('"')?;
        let s = unescape(quoted(body).map_or(body, |(b, _)| b));
        return Some(if ty == "7" {
            RegValue::Multi(s.split('\0').filter(|x| !x.is_empty()).map(str::to_string).collect())
        } else {
            RegValue::Str(s.trim_end_matches('\0').to_string())
        });
    }
    if let Some(d) = val.strip_prefix("dword:") {
        return u32::from_str_radix(d.trim(), 16).ok().map(RegValue::Dword);
    }
    let (ty, hex) =
        if let Some(h) = val.strip_prefix("hex:") { ("", h) } else { val.strip_prefix("hex(")?.split_once("):")? };
    let bytes: Vec<u8> = hex
        .split(',')
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .filter_map(|b| u8::from_str_radix(b, 16).ok())
        .collect();
    let utf16 = |b: &[u8]| -> String {
        let u: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u)
    };
    Some(match ty {
        "1" | "2" => RegValue::Str(utf16(&bytes).trim_end_matches('\0').to_string()),
        "7" => RegValue::Multi(utf16(&bytes).split('\0').filter(|x| !x.is_empty()).map(str::to_string).collect()),
        "4" if bytes.len() == 4 => RegValue::Dword(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        _ => RegValue::Bytes(bytes),
    })
}
