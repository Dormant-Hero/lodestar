//! Plain text from the HTML in store descriptions.

/// Remove HTML tags / decode a few entities (Steam store descriptions are HTML).
pub(crate) fn strip_html(s: &str) -> String {
    if !s.contains('<') && !s.contains('&') {
        return s.trim().to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '<' => {
                let rest = &s[i..];
                let Some(end) = rest.find('>') else {
                    out.push('<'); // a lone '<' is just text
                    continue;
                };
                let tag = rest[1..end].trim().to_ascii_lowercase();
                let name =
                    tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
                if matches!(name, "br" | "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "ul" | "ol" | "tr")
                    && !out.is_empty()
                    && !out.ends_with('\n')
                {
                    out.push('\n');
                }
                if name == "li" && !tag.starts_with('/') {
                    out.push_str("• ");
                }
                // skip to the end of the tag
                while let Some(&(j, _)) = chars.peek() {
                    if j > i + end {
                        break;
                    }
                    chars.next();
                }
            }
            '&' => {
                let rest = &s[i..];
                let ent = [
                    ("&amp;", '&'),
                    ("&lt;", '<'),
                    ("&gt;", '>'),
                    ("&quot;", '"'),
                    ("&#39;", '\''),
                    ("&nbsp;", ' '),
                    ("&apos;", '\''),
                ]
                .iter()
                .find(|(e, _)| rest.starts_with(e));
                match ent {
                    Some((e, ch)) => {
                        out.push(*ch);
                        for _ in 1..e.len() {
                            chars.next();
                        }
                    }
                    None => out.push('&'),
                }
            }
            _ => out.push(c),
        }
    }
    // collapse 3+ newlines
    let mut res = String::with_capacity(out.len());
    let mut nl = 0;
    for c in out.trim().chars() {
        if c == '\n' {
            nl += 1;
            if nl > 2 {
                continue;
            }
        } else {
            nl = 0;
        }
        res.push(c);
    }
    res
}
