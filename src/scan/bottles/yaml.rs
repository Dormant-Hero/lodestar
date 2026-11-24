//! A deliberately small YAML subset parser, enough for files PyYAML writes for Bottles
//! (`library.yml`, `bottle.yml`): block mappings and sequences, plain / single / double quoted
//! scalars (including PyYAML's line-folded long scalars), `{}`/`[]` empty flow collections,
//! comments. Anything fancier (anchors, tags, multi-document, flow collections with content,
//! block scalars `|`/`>`) degrades to plain strings or is skipped — never a panic.

#[derive(Clone, Debug, PartialEq)]
pub enum Yaml {
    Null,
    Str(String),
    Map(Vec<(String, Yaml)>),
    Seq(Vec<Yaml>),
}

impl Yaml {
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Yaml::Str(s) => Some(s),
            _ => None,
        }
    }
    /// String value of `key`, trimmed, `None` when missing/empty/not a scalar.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Yaml::str).map(str::trim).filter(|s| !s.is_empty())
    }
    pub fn entries(&self) -> &[(String, Yaml)] {
        match self {
            Yaml::Map(m) => m,
            _ => &[],
        }
    }
}

struct Line<'a> {
    indent: usize,
    text: &'a str, // without indentation, comments kept (stripped lazily)
}

/// Lines indented deeper than this are dropped. Nesting depth (and so parser recursion) is
/// bounded by the indentation, so this keeps a hostile file from overflowing the scan thread's
/// stack. Bottles' own files stay below 20.
const MAX_INDENT: usize = 256;

pub fn parse(src: &str) -> Yaml {
    let lines: Vec<Line> = src
        .lines()
        .filter_map(|l| {
            let l = l.trim_end_matches('\r');
            let trimmed = l.trim_start_matches(' ');
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed == "---"
                || trimmed == "..."
                || l.len() - trimmed.len() > MAX_INDENT
            {
                return None;
            }
            Some(Line { indent: l.len() - trimmed.len(), text: trimmed })
        })
        .collect();
    let mut pos = 0;
    let indent = lines.first().map(|l| l.indent).unwrap_or(0);
    parse_block(&lines, &mut pos, indent)
}

fn parse_block(lines: &[Line], pos: &mut usize, indent: usize) -> Yaml {
    match lines.get(*pos) {
        Some(l) if l.indent == indent && is_seq_item(l.text) => parse_seq(lines, pos, indent),
        Some(l) if l.indent == indent => parse_map(lines, pos, indent),
        _ => Yaml::Null,
    }
}

fn is_seq_item(t: &str) -> bool {
    t == "-" || t.starts_with("- ")
}

fn parse_seq(lines: &[Line], pos: &mut usize, indent: usize) -> Yaml {
    let mut items = vec![];
    while let Some(l) = lines.get(*pos) {
        if l.indent != indent || !is_seq_item(l.text) {
            if l.indent > indent {
                *pos += 1; // malformed: skip
                continue;
            }
            break;
        }
        let rest = l.text[1..].trim_start_matches(' ');
        if rest.is_empty() {
            *pos += 1;
            let child = lines.get(*pos).map(|n| n.indent).filter(|&i| i > indent);
            items.push(child.map_or(Yaml::Null, |ci| parse_block(lines, pos, ci)));
        } else if split_key(rest).is_some() {
            // "- key: value" starts an inline mapping whose further keys are indented to `key`.
            let inner = indent + (l.text.len() - rest.len());
            items.push(parse_map_from(lines, pos, inner, Some(rest)));
        } else {
            let (v, consumed) = scalar_with_continuation(lines, *pos, rest, indent);
            *pos += consumed;
            items.push(v);
        }
    }
    Yaml::Seq(items)
}

fn parse_map(lines: &[Line], pos: &mut usize, indent: usize) -> Yaml {
    parse_map_from(lines, pos, indent, None)
}

/// Parse a mapping at `indent`. `first` is the text of the first entry when it does not start at
/// the beginning of a line (the `- key: v` sequence form).
fn parse_map_from(lines: &[Line], pos: &mut usize, indent: usize, mut first: Option<&str>) -> Yaml {
    let mut entries: Vec<(String, Yaml)> = vec![];
    loop {
        let text = match first.take() {
            Some(t) => t,
            None => match lines.get(*pos) {
                Some(l) if l.indent == indent && !is_seq_item(l.text) => l.text,
                Some(l) if l.indent > indent => {
                    *pos += 1; // stray deeper line
                    continue;
                }
                _ => break,
            },
        };
        let Some((key, rest)) = split_key(text) else {
            *pos += 1; // not a key: skip the line
            continue;
        };
        let rest = strip_comment(rest).trim();
        if rest.is_empty() {
            *pos += 1;
            let value = match lines.get(*pos) {
                Some(n) if n.indent > indent => parse_block(lines, pos, n.indent),
                // PyYAML writes sequences under a key at the key's own indentation.
                Some(n) if n.indent == indent && is_seq_item(n.text) => parse_seq(lines, pos, indent),
                _ => Yaml::Null,
            };
            entries.push((key, value));
        } else {
            let (v, consumed) = scalar_with_continuation(lines, *pos, rest, indent);
            *pos += consumed;
            entries.push((key, v));
        }
    }
    Yaml::Map(entries)
}

/// `key: rest` → (unquoted key, rest). Handles quoted keys; requires ": " or a trailing ":".
fn split_key(t: &str) -> Option<(String, &str)> {
    if let Some(q @ ('"' | '\'')) = t.chars().next() {
        let end = find_closing_quote(t, q)?;
        let after = &t[end + 1..];
        let rest = after.strip_prefix(':')?;
        if !(rest.is_empty() || rest.starts_with(' ')) {
            return None;
        }
        return Some((unquote(&t[..=end]), rest));
    }
    let bytes = t.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' ') {
            let key = t[..i].trim();
            if key.is_empty() || key.starts_with('#') {
                return None;
            }
            return Some((key.to_string(), &t[i + 1..]));
        }
        if bytes[i] == b'#' && i > 0 && bytes[i - 1] == b' ' {
            return None;
        }
        i += 1;
    }
    None
}

fn find_closing_quote(t: &str, q: char) -> Option<usize> {
    let mut it = t.char_indices().skip(1).peekable();
    while let Some((i, c)) = it.next() {
        if q == '\'' && c == '\'' {
            if it.peek().is_some_and(|&(_, n)| n == '\'') {
                it.next();
                continue;
            }
            return Some(i);
        }
        if q == '"' && c == '\\' {
            it.next();
            continue;
        }
        if q == '"' && c == '"' {
            return Some(i);
        }
    }
    None
}

fn strip_comment(s: &str) -> &str {
    let t = s.trim_start();
    if t.starts_with('"') || t.starts_with('\'') {
        return s;
    }
    match s.find(" #") {
        Some(i) => &s[..i],
        None => s,
    }
}

/// Read a scalar starting with `first`, joining continuation lines (PyYAML folds long scalars
/// onto following lines indented deeper than the owning key). Returns (value, lines consumed).
fn scalar_with_continuation(lines: &[Line], pos: usize, first: &str, owner_indent: usize) -> (Yaml, usize) {
    let first = first.trim();
    if first == "|" || first.starts_with("|-") || first.starts_with('>') || first.starts_with("|+") {
        // Block scalar: take the deeper lines verbatim, joined by newlines (good enough).
        let mut parts = vec![];
        let mut n = 1;
        while let Some(l) = lines.get(pos + n).filter(|l| l.indent > owner_indent) {
            parts.push(l.text);
            n += 1;
        }
        let sep = if first.starts_with('>') { " " } else { "\n" };
        return (Yaml::Str(parts.join(sep)), n);
    }
    let mut text = first.to_string();
    let mut n = 1;
    let quoted = first.starts_with('"') || first.starts_with('\'');
    loop {
        let complete = if quoted {
            let q = first.chars().next().unwrap_or('"');
            find_closing_quote(&text, q).is_some()
        } else {
            false
        };
        if complete {
            break;
        }
        match lines.get(pos + n) {
            Some(l)
                if l.indent > owner_indent
                    && (quoted || split_key(l.text).is_none() || !is_plain_continuation_key(l.text)) =>
            {
                text.push(' ');
                text.push_str(l.text.trim());
                n += 1;
            }
            _ => break,
        }
    }
    (scalar(&text), n)
}

/// A deeper line after a plain scalar can't legally be a key, so treat it as continuation —
/// unless it plainly looks like `key: value` (malformed input), in which case stop.
fn is_plain_continuation_key(t: &str) -> bool {
    split_key(t).is_some_and(|(k, _)| !k.contains(' '))
}

fn scalar(raw: &str) -> Yaml {
    let t = strip_comment(raw).trim();
    match t {
        "" | "~" | "null" | "Null" | "NULL" => Yaml::Null,
        "{}" => Yaml::Map(vec![]),
        "[]" => Yaml::Seq(vec![]),
        _ if t.starts_with('"') || t.starts_with('\'') => Yaml::Str(unquote(t)),
        _ => Yaml::Str(t.to_string()),
    }
}

fn unquote(t: &str) -> String {
    let Some(q) = t.chars().next() else {
        return String::new();
    };
    let end = find_closing_quote(t, q).unwrap_or(t.len());
    let inner = t.get(1..end).unwrap_or("");
    if q == '\'' {
        return inner.replace("''", "'");
    }
    let mut out = String::with_capacity(inner.len());
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(' ') => out.push(' '),
            Some('/') => out.push('/'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('x') => {
                let h: String = it.by_ref().take(2).collect();
                if let Some(c) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            Some('u') => {
                let h: String = it.by_ref().take(4).collect();
                if let Some(c) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_maps_and_scalars() {
        let y = parse(
            "# comment\na:\n  b: 1\n  c: 'it''s'\n  d: \"x\\ty\"\n  e: ''\n  f: {}\nz: plain value # trailing\nq: ~\n",
        );
        let a = y.get("a").unwrap();
        assert_eq!(a.get_str("b"), Some("1"));
        assert_eq!(a.get_str("c"), Some("it's"));
        assert_eq!(a.get_str("d"), Some("x\ty"));
        assert_eq!(a.get("e"), Some(&Yaml::Str(String::new())));
        assert_eq!(a.get("f"), Some(&Yaml::Map(vec![])));
        assert_eq!(y.get_str("z"), Some("plain value"));
        assert_eq!(y.get("q"), Some(&Yaml::Null));
    }

    #[test]
    fn folded_long_scalars() {
        let y = parse(
            "k:\n  path: /home/u/.local/share/bottles/bottles/Games/drive_c/Program Files\n    (x86)/Some Game/game.exe\n  next: 1\n  q: 'a b\n    c'\n",
        );
        let k = y.get("k").unwrap();
        assert_eq!(
            k.get_str("path"),
            Some("/home/u/.local/share/bottles/bottles/Games/drive_c/Program Files (x86)/Some Game/game.exe")
        );
        assert_eq!(k.get_str("next"), Some("1"));
        assert_eq!(k.get_str("q"), Some("a b c"));
    }

    #[test]
    fn sequences() {
        let y = parse("list:\n- a\n- b\nmaps:\n  - name: x\n    v: 1\n  - name: y\nempty: []\n");
        assert_eq!(y.get("list"), Some(&Yaml::Seq(vec![Yaml::Str("a".into()), Yaml::Str("b".into())])));
        match y.get("maps") {
            Some(Yaml::Seq(items)) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].get_str("v"), Some("1"));
                assert_eq!(items[1].get_str("name"), Some("y"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(y.get("empty"), Some(&Yaml::Seq(vec![])));
    }

    #[test]
    fn garbage_never_panics() {
        for s in [
            "",
            ":",
            "- - -",
            "a: 'unterminated",
            "  x: 1\ny: 2",
            "a:\n    b: 1\n  c: 2",
            "\"k\": v",
            "a: |\n  l1\n  l2\n",
            "é: ü\n",
        ] {
            let _ = parse(s);
        }
        assert_eq!(parse("a: |\n  l1\n  l2\nb: 1").get_str("a"), Some("l1\nl2"));
    }
}
