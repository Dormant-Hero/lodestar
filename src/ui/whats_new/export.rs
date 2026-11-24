//! The changelog as a file to keep or share: a self-contained web page, the Markdown itself or
//! plain text (also what "Copy to clipboard" copies). All three are built from the same
//! `parse` blocks the in-app view draws, so they show the same thing.

use super::{Block, Span, parse, spans};

/// What the changelog is exported as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Html,
    Markdown,
    Text,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Html, Format::Markdown, Format::Text];

    /// Menu entry ("Web page (.html)…").
    pub fn label(self) -> &'static str {
        match self {
            Format::Html => "Web page (.html)…",
            Format::Markdown => "Markdown (.md)…",
            Format::Text => "Plain text (.txt)…",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Html => "html",
            Format::Markdown => "md",
            Format::Text => "txt",
        }
    }

    /// Name of the save dialog's file-type filter.
    pub fn filter_name(self) -> &'static str {
        match self {
            Format::Html => "Web page",
            Format::Markdown => "Markdown",
            Format::Text => "Plain text",
        }
    }

    /// "lodestar-changelog.html".
    pub fn default_file_name(self) -> String {
        format!("lodestar-changelog.{}", self.ext())
    }

    /// The format a path's extension asks for (testing hook, `export:<path>`).
    pub fn from_path(p: &std::path::Path) -> Format {
        match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("html" | "htm") => Format::Html,
            Some("md" | "markdown") => Format::Markdown,
            _ => Format::Text,
        }
    }

    /// The changelog `md` in this format.
    pub fn render(self, md: &str, version: &str) -> String {
        match self {
            Format::Html => to_html(md, version),
            Format::Markdown => {
                let mut s = md.to_string();
                if !s.ends_with('\n') {
                    s.push('\n');
                }
                s
            }
            Format::Text => to_text(md),
        }
    }
}

// ------------------------------------------------------------------------------- HTML

/// Escape text for HTML element content and quoted attribute values.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// One line's `**bold**` / `` `code` `` as escaped HTML.
fn inline_html(text: &str) -> String {
    let mut out = String::new();
    for (kind, t) in spans(text) {
        let t = escape_html(t);
        match kind {
            Span::Plain => out.push_str(&t),
            Span::Bold => {
                out.push_str("<strong>");
                out.push_str(&t);
                out.push_str("</strong>");
            }
            Span::Code => {
                out.push_str("<code>");
                out.push_str(&t);
                out.push_str("</code>");
            }
        }
    }
    out
}

const CSS: &str = "\
:root { color-scheme: light dark; --bg: #f7f7f9; --fg: #1d1f24; --dim: #5b6070; --accent: #3b6fd8;
  --rule: #d9dce3; --code-bg: #e9ebf0; }
@media (prefers-color-scheme: dark) {
  :root { --bg: #16181d; --fg: #e4e6eb; --dim: #a0a6b4; --accent: #7ea6ff; --rule: #2e323b;
    --code-bg: #262a33; }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg);
  font: 16px/1.6 system-ui, -apple-system, \"Segoe UI\", Roboto, \"Noto Sans\", sans-serif; }
main { max-width: 46rem; margin: 0 auto; padding: 2.5rem 1.25rem 4rem; }
h1 { font-size: 2rem; margin: 0 0 .25rem; }
h2 { font-size: 1.35rem; color: var(--accent); margin: 2.5rem 0 .5rem; padding-bottom: .3rem;
  border-bottom: 1px solid var(--rule); }
h3 { font-size: 1.05rem; margin: 1.5rem 0 .4rem; }
p { color: var(--dim); margin: .5rem 0; }
ul { margin: .3rem 0; padding-left: 1.4rem; }
li { margin: .3rem 0; }
li > ul { margin: .2rem 0; }
strong { font-weight: 650; }
code { font: .9em ui-monospace, \"JetBrains Mono\", \"DejaVu Sans Mono\", monospace;
  background: var(--code-bg); padding: .1em .35em; border-radius: 4px; }
footer { margin-top: 3rem; color: var(--dim); font-size: .85rem; }
";

/// The changelog as one self-contained web page (inline CSS, follows the system's light /
/// dark setting).
pub fn to_html(md: &str, version: &str) -> String {
    let blocks = parse(md);
    let title = blocks
        .iter()
        .find_map(|b| match b {
            Block::Heading(1, h) => Some(h.as_str()),
            _ => None,
        })
        .unwrap_or("Lodestar changelog");
    let mut out = String::new();
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<meta name=\"color-scheme\" content=\"light dark\">\n");
    out.push_str(&format!("<title>{}</title>\n", escape_html(&plain_inline(title))));
    out.push_str("<style>\n");
    out.push_str(CSS);
    out.push_str("</style>\n</head>\n<body>\n<main>\n");
    // Open `<ul>`s; the `<li>` of each is still open too (nested lists go inside it).
    let mut depth = 0usize;
    let close_lists = |out: &mut String, depth: &mut usize, to: usize| {
        while *depth > to {
            out.push_str("</li>\n");
            out.push_str(&"  ".repeat(*depth - 1));
            out.push_str("</ul>\n");
            *depth -= 1;
        }
    };
    for b in &blocks {
        match b {
            Block::Bullet(level, t) => {
                // A jump of several levels opens only one list (no empty items).
                let want = (*level + 1).min(depth + 1);
                close_lists(&mut out, &mut depth, want);
                if depth == want {
                    out.push_str("</li>\n");
                } else {
                    if depth > 0 {
                        out.push('\n');
                    }
                    out.push_str(&"  ".repeat(depth));
                    out.push_str("<ul>\n");
                    depth += 1;
                }
                out.push_str(&"  ".repeat(depth));
                out.push_str("<li>");
                out.push_str(&inline_html(t));
            }
            Block::Heading(level, h) => {
                close_lists(&mut out, &mut depth, 0);
                let l = (*level).clamp(1, 6);
                out.push_str(&format!("<h{l}>{}</h{l}>\n", inline_html(h)));
            }
            Block::Para(t) => {
                close_lists(&mut out, &mut depth, 0);
                out.push_str(&format!("<p>{}</p>\n", inline_html(t)));
            }
        }
    }
    close_lists(&mut out, &mut depth, 0);
    out.push_str(&format!(
        "<footer>Exported from Lodestar {}.</footer>\n</main>\n</body>\n</html>\n",
        escape_html(version)
    ));
    out
}

// ------------------------------------------------------------------------------- plain text

/// Lines are wrapped to this many columns.
pub const TEXT_WIDTH: usize = 100;

/// One line without its `**` / backtick markers.
fn plain_inline(text: &str) -> String {
    spans(text).into_iter().map(|(_, t)| t).collect()
}

/// Word-wrap `text` to `width` columns: the first line starts with `first`, the others with
/// `rest` (hanging indent). A word longer than a line gets a line of its own.
fn wrap(text: &str, width: usize, first: &str, rest: &str) -> String {
    let mut out = String::new();
    let mut line = first.to_string();
    let mut line_len = first.chars().count();
    let mut empty = true;
    for word in text.split_whitespace() {
        let w = word.chars().count();
        if !empty && line_len + 1 + w > width {
            out.push_str(line.trim_end());
            out.push('\n');
            line = rest.to_string();
            line_len = rest.chars().count();
            empty = true;
        }
        if !empty {
            line.push(' ');
            line_len += 1;
        }
        line.push_str(word);
        line_len += w;
        empty = false;
    }
    out.push_str(line.trim_end());
    out.push('\n');
    out
}

/// The changelog as plain text: headings underlined (`=` / `-` / `~`), bullets as `- `
/// (nested ones indented), Markdown markers removed, wrapped to [`TEXT_WIDTH`] columns.
pub fn to_text(md: &str) -> String {
    let mut out = String::new();
    let mut prev_bullet = false;
    for b in parse(md) {
        // A blank line between blocks, except between the items of one list.
        let is_bullet = matches!(b, Block::Bullet(..));
        if !out.is_empty() && !(is_bullet && prev_bullet) {
            out.push('\n');
        }
        prev_bullet = is_bullet;
        match b {
            Block::Heading(level, h) => {
                let h = plain_inline(&h);
                let under = match level {
                    1 => '=',
                    2 => '-',
                    _ => '~',
                };
                let wrapped = wrap(&h, TEXT_WIDTH, "", "");
                let longest = wrapped.lines().map(|l| l.chars().count()).max().unwrap_or(0);
                out.push_str(&wrapped);
                out.extend(std::iter::repeat_n(under, longest.max(3)));
                out.push('\n');
            }
            Block::Bullet(level, t) => {
                let pad = "  ".repeat(level);
                out.push_str(&wrap(&plain_inline(&t), TEXT_WIDTH, &format!("{pad}- "), &format!("{pad}  ")));
            }
            Block::Para(t) => out.push_str(&wrap(&plain_inline(&t), TEXT_WIDTH, "", "")),
        }
    }
    out
}

#[cfg(test)]
mod tests;
