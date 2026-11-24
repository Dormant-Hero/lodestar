//! A small YAML reader for the subset Playnite's add-on database and installer manifests use:
//! block mappings, block sequences (also of mappings), flow sequences (`[a, "b"]`), quoted and
//! plain scalars, `|` / `>` block scalars and `#` comments. No anchors, tags or multi-documents
//! (they never appear there). Never panics; unknown shapes come out as plain strings.

/// A parsed YAML value.
#[derive(Clone, Debug, PartialEq)]
pub enum Y {
    Str(String),
    List(Vec<Y>),
    Map(Vec<(String, Y)>),
}

impl Y {
    /// The value under `key` of a mapping (keys compare case-insensitively: manifests mix
    /// `Github` / `GitHub`).
    pub fn get(&self, key: &str) -> Option<&Y> {
        match self {
            Y::Map(m) => m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A non-empty string under `key`.
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Y::Str(s) if !s.trim().is_empty() => Some(s.trim()),
            _ => None,
        }
    }

    pub fn as_list(&self) -> &[Y] {
        match self {
            Y::List(l) => l,
            _ => &[],
        }
    }

    /// Strings of a list (a lone string counts as a one-item list).
    pub fn strings(&self) -> Vec<String> {
        match self {
            Y::Str(s) if !s.trim().is_empty() => vec![s.trim().to_string()],
            Y::List(l) => l
                .iter()
                .filter_map(|y| match y {
                    Y::Str(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        }
    }
}

const MAX_DEPTH: usize = 32;

/// Parse a YAML document (a mapping or a sequence at the top).
pub fn parse(text: &str) -> Y {
    let mut lines: Vec<String> = text.trim_start_matches('\u{feff}').lines().map(|l| l.replace('\t', "    ")).collect();
    // A leading document marker.
    if lines.first().is_some_and(|l| l.trim() == "---") {
        lines.remove(0);
    }
    let mut p = Parser { lines, i: 0 };
    match p.next_indent() {
        Some(ind) => p.block(ind, 0),
        None => Y::Map(vec![]),
    }
}

struct Parser {
    lines: Vec<String>,
    i: usize,
}

fn indent_of(l: &str) -> usize {
    l.len() - l.trim_start_matches(' ').len()
}

fn significant(l: &str) -> bool {
    let t = l.trim();
    !t.is_empty() && !t.starts_with('#') && t != "---" && t != "..."
}

fn is_item(t: &str) -> bool {
    t == "-" || t.starts_with("- ")
}

/// `key: rest` → (key, rest). Keys may be quoted; plain keys hold no `: ` and no `#`.
fn split_key(t: &str) -> Option<(String, &str)> {
    let (key, rest) = if let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let end = t[1..].find(q)? + 1;
        let after = t[end + 1..].trim_start();
        let rest = after.strip_prefix(':')?;
        (t[1..end].to_string(), rest)
    } else {
        let pos = t.find(": ").or_else(|| t.ends_with(':').then(|| t.len() - 1))?;
        let key = &t[..pos];
        if key.is_empty() || key.contains('#') || key.starts_with('[') || key.starts_with('{') {
            return None;
        }
        (key.trim().to_string(), &t[pos + 1..])
    };
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((key, rest.trim()))
}

/// A scalar: quotes removed (with `''` / `\"` unescaped), trailing ` # comment` cut off.
fn scalar(raw: &str) -> String {
    let s = raw.trim();
    if let Some(inner) = s.strip_prefix('\'') {
        let end = inner.rfind('\'').unwrap_or(inner.len());
        return inner[..end].replace("''", "'");
    }
    if let Some(inner) = s.strip_prefix('"') {
        let end = inner.rfind('"').unwrap_or(inner.len());
        let mut out = String::new();
        let mut chars = inner[..end].chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(o) => out.push(o),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    let s = match s.find(" #") {
        Some(p) => &s[..p],
        None => s,
    };
    let s = s.trim();
    if s == "~" || s == "null" { String::new() } else { s.to_string() }
}

/// `[a, "b, c", 'd']` → items.
fn flow_list(raw: &str) -> Y {
    let inner = raw.trim().trim_start_matches('[');
    let inner = inner.strip_suffix(']').unwrap_or(inner);
    let mut items = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in inner.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                cur.push(c);
            }
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                cur.push(c);
            }
            None if c == ',' => {
                items.push(std::mem::take(&mut cur));
            }
            None => cur.push(c),
        }
    }
    items.push(cur);
    Y::List(items.iter().map(|s| scalar(s)).filter(|s| !s.is_empty()).map(Y::Str).collect())
}

impl Parser {
    /// Indentation of the next significant line (skipping blanks / comments).
    fn next_indent(&mut self) -> Option<usize> {
        while self.i < self.lines.len() {
            if significant(&self.lines[self.i]) {
                return Some(indent_of(&self.lines[self.i]));
            }
            self.i += 1;
        }
        None
    }

    fn block(&mut self, ind: usize, depth: usize) -> Y {
        if depth > MAX_DEPTH {
            // Skip the rest of this block.
            while self.next_indent().is_some_and(|n| n >= ind) {
                self.i += 1;
            }
            return Y::Str(String::new());
        }
        let first = self.lines[self.i].trim().to_string();
        if is_item(&first) {
            self.seq(ind, depth)
        } else if split_key(&first).is_some() {
            self.map(ind, depth)
        } else {
            // A plain multi-line scalar.
            let mut parts = vec![];
            while self.next_indent().is_some_and(|n| n >= ind) {
                parts.push(self.lines[self.i].trim().to_string());
                self.i += 1;
            }
            Y::Str(scalar(&parts.join(" ")))
        }
    }

    fn map(&mut self, ind: usize, depth: usize) -> Y {
        let mut out: Vec<(String, Y)> = vec![];
        while let Some(n) = self.next_indent() {
            if n != ind {
                break;
            }
            let line = self.lines[self.i].trim().to_string();
            if is_item(&line) {
                break;
            }
            let Some((key, rest)) = split_key(&line) else {
                // Not a key line: skip it (and anything nested under it).
                self.i += 1;
                while self.next_indent().is_some_and(|m| m > ind) {
                    self.i += 1;
                }
                continue;
            };
            let rest = rest.to_string();
            self.i += 1;
            let value = if rest.is_empty() {
                match self.next_indent() {
                    Some(m) if m > ind => self.block(m, depth + 1),
                    // Sequences may sit at the key's own indentation.
                    Some(m) if m == ind && is_item(self.lines[self.i].trim()) => self.seq(m, depth + 1),
                    _ => Y::Str(String::new()),
                }
            } else if rest.starts_with('|') || rest.starts_with('>') {
                self.block_scalar(ind, &rest)
            } else if rest.starts_with('[') {
                let mut raw = rest.clone();
                while !raw.trim_end().ends_with(']') && self.i < self.lines.len() {
                    raw.push(' ');
                    raw.push_str(self.lines[self.i].trim());
                    self.i += 1;
                }
                flow_list(&raw)
            } else {
                Y::Str(scalar(&rest))
            };
            out.push((key, value));
        }
        Y::Map(out)
    }

    fn seq(&mut self, ind: usize, depth: usize) -> Y {
        let mut out = vec![];
        while let Some(n) = self.next_indent() {
            if n != ind {
                break;
            }
            let line = self.lines[self.i].clone();
            let t = line.trim();
            if !is_item(t) {
                break;
            }
            let content = t[1..].trim_start();
            if content.is_empty() {
                self.i += 1;
                match self.next_indent() {
                    Some(m) if m > ind => out.push(self.block(m, depth + 1)),
                    _ => out.push(Y::Str(String::new())),
                }
            } else if split_key(content).is_some() {
                // `- key: v` + more keys below: a mapping that starts on the item's line.
                let offset = ind + (t.len() - content.len());
                self.lines[self.i] = format!("{}{}", " ".repeat(offset), content);
                out.push(self.map(offset, depth + 1));
            } else if content.starts_with('[') {
                self.i += 1;
                out.push(flow_list(content));
            } else {
                self.i += 1;
                out.push(Y::Str(scalar(content)));
            }
        }
        Y::List(out)
    }

    /// `|`, `|-`, `>`, `>-` … : the more-indented lines after the key.
    fn block_scalar(&mut self, ind: usize, head: &str) -> Y {
        let folded = head.starts_with('>');
        let strip = head.contains('-');
        let mut raw: Vec<String> = vec![];
        while self.i < self.lines.len() {
            let l = &self.lines[self.i];
            if l.trim().is_empty() {
                raw.push(String::new());
                self.i += 1;
                continue;
            }
            if indent_of(l) <= ind {
                break;
            }
            raw.push(l.clone());
            self.i += 1;
        }
        while raw.last().is_some_and(|l| l.is_empty()) {
            raw.pop();
        }
        let cut = raw.iter().filter(|l| !l.is_empty()).map(|l| indent_of(l)).min().unwrap_or(0);
        let body: Vec<String> =
            raw.iter().map(|l| if l.len() >= cut { l[cut..].to_string() } else { String::new() }).collect();
        let mut s = if folded {
            let mut out = String::new();
            for l in &body {
                if l.is_empty() {
                    out.push('\n');
                } else {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push(' ');
                    }
                    out.push_str(l);
                }
            }
            out
        } else {
            body.join("\n")
        };
        if !strip && !s.is_empty() {
            s.push('\n');
        }
        Y::Str(s)
    }
}
