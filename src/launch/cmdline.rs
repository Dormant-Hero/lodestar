//! Command-line text: splitting launch options and commands into words (quotes, `~`, `$VAR`),
//! `VAR=value` words, quoting for display, and PATH lookups.

use super::*;

/// Parsed `LaunchOverride::options`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Options {
    pub(super) env: Vec<(String, String)>,
    /// Words before `%command%` (None when there is no `%command%`).
    pub(super) prefix: Vec<String>,
    /// Words after `%command%`, or every non-env word when there is no `%command%`.
    pub(super) suffix: Vec<String>,
}

/// A `VAR=value` word split at its first `=` (None: no `=`, or not a variable name before it).
pub(crate) fn split_env_word(w: &str) -> Option<(&str, &str)> {
    let (k, v) = w.split_once('=')?;
    let mut chars = k.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((k, v))
}

pub(super) fn is_env_word(w: &str) -> Option<(String, String)> {
    split_env_word(w).map(|(k, v)| (k.to_string(), v.to_string()))
}

pub(super) fn parse_options(s: &str, host: &Host) -> anyhow::Result<Options> {
    let words = split_words(s, host, true)?;
    let mut out = Options::default();
    let mut i = 0;
    while i < words.len() {
        let w = &words[i];
        if w.quoted_eq || w.text == "%command%" {
            break;
        }
        match is_env_word(&w.text) {
            Some((k, v)) => set_env(&mut out.env, k, v),
            None => break,
        }
        i += 1;
    }
    let rest: Vec<Word> = words.into_iter().skip(i).collect();
    if rest.iter().filter(|w| w.text == "%command%").count() > 1 {
        bail!("%command% can only appear once");
    }
    match rest.iter().position(|w| w.text == "%command%") {
        Some(p) => {
            out.prefix = rest[..p].iter().map(|w| w.text.clone()).collect();
            out.suffix = rest[p + 1..].iter().map(|w| w.text.clone()).collect();
        }
        None => out.suffix = rest.into_iter().map(|w| w.text).collect(),
    }
    Ok(out)
}

pub(super) struct Word {
    pub(super) text: String,
    /// The `NAME=` part was quoted (`"A=b"`), so it is not an assignment.
    pub(super) quoted_eq: bool,
}

/// Split a command line like a POSIX shell would (quotes, backslashes), without running a shell.
/// No variable expansion. Errors on unterminated quotes.
pub fn split_command(s: &str) -> anyhow::Result<Vec<String>> {
    let host = Host { steam: SteamCmd::XdgOpen, home: None, vars: HashMap::new() };
    Ok(split_words(s, &host, false)?.into_iter().map(|w| w.text).collect())
}

pub(super) fn split_words(s: &str, host: &Host, expand: bool) -> anyhow::Result<Vec<Word>> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut in_word = false;
    let mut quoted_before_eq = false;
    let mut seen_eq = false;
    let mut chars = s.chars().peekable();
    let push_var = |chars: &mut std::iter::Peekable<std::str::Chars>, cur: &mut String| {
        // Called after '$'.
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                name.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if braced {
            if chars.peek() == Some(&'}') && !name.is_empty() {
                chars.next();
            } else {
                // Not a valid ${NAME}: keep it literal.
                cur.push_str("${");
                cur.push_str(&name);
                return;
            }
        }
        if name.is_empty() {
            cur.push('$');
        } else if let Some(v) = host.vars.get(&name) {
            cur.push_str(v);
        }
    };
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    out.push(Word { text: std::mem::take(&mut cur), quoted_eq: quoted_before_eq });
                    in_word = false;
                    quoted_before_eq = false;
                    seen_eq = false;
                }
            }
            '\'' => {
                in_word = true;
                if !seen_eq {
                    quoted_before_eq = true;
                }
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => bail!("unterminated ' quote"),
                    }
                }
            }
            '"' => {
                in_word = true;
                if !seen_eq {
                    quoted_before_eq = true;
                }
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '\\' | '$' | '`')) => cur.push(e),
                            Some('\n') => {}
                            Some(o) => {
                                cur.push('\\');
                                cur.push(o);
                            }
                            None => bail!("unterminated \" quote"),
                        },
                        Some('$') if expand => push_var(&mut chars, &mut cur),
                        Some(c) => cur.push(c),
                        None => bail!("unterminated \" quote"),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some('\n') => {}
                    Some(e) => cur.push(e),
                    None => cur.push('\\'),
                }
            }
            '$' if expand => {
                in_word = true;
                push_var(&mut chars, &mut cur);
            }
            '~' if expand && tilde_ok(in_word, seen_eq, &cur, chars.peek().copied()) => {
                in_word = true;
                match &host.home {
                    Some(h) => cur.push_str(&h.to_string_lossy()),
                    None => cur.push('~'),
                }
            }
            c => {
                in_word = true;
                if c == '=' {
                    seen_eq = true;
                }
                cur.push(c);
            }
        }
    }
    if in_word {
        out.push(Word { text: cur, quoted_eq: quoted_before_eq });
    }
    Ok(out)
}

/// `~` expands at the start of a word, or right after `NAME=`, when followed by `/` or the end.
pub(super) fn tilde_ok(in_word: bool, seen_eq: bool, cur: &str, next: Option<char>) -> bool {
    let at_start = !in_word || (seen_eq && cur.ends_with('=') && cur.matches('=').count() == 1);
    at_start && next.is_none_or(|c| c == '/' || c.is_whitespace())
}

/// Quote a word for display so that pasting it into a shell gives the same word back.
/// `~` is always quoted: the launch-options parser expands it at a word's start and after `=`.
pub fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_-+=:,./@%^".contains(c));
    if safe { s.to_string() } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

/// Like `shell_quote`, but a program named like `NAME=value` is quoted too, so the command line
/// doesn't read back as an environment assignment.
pub(super) fn quote_program(p: &str) -> String {
    let q = shell_quote(p);
    if q == p && is_env_word(p).is_some() { format!("'{p}'") } else { q }
}

pub(super) fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Look `program` up in PATH (or check it directly when it contains a slash).
pub fn find_in_path(program: &str) -> Option<PathBuf> {
    find_in_path_with(program, std::env::var_os("PATH").as_deref())
}

/// Look `program` up in the PATH a command will run with: its own `PATH=` (std's `Command`
/// resolves the program with the child's PATH when one is set), else Lodestar's.
pub(super) fn find_in_command_path(program: &str, env: &[(String, String)]) -> Option<PathBuf> {
    match env.iter().rev().find(|(k, _)| k == "PATH") {
        Some((_, p)) => find_in_path_with(program, Some(std::ffi::OsStr::new(p))),
        None => find_in_path(program),
    }
}

/// `find_in_path` with an explicit PATH value.
pub fn find_in_path_with(program: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let is_exec =
        |p: &Path| std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false);
    if program.contains('/') {
        let p = PathBuf::from(program);
        return is_exec(&p).then_some(p);
    }
    std::env::split_paths(path?).map(|d| d.join(program)).find(|p| is_exec(p))
}
