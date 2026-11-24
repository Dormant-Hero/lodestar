//! The changelog (CHANGELOG.md, built into the binary) shown inside the app: menu → Changelog,
//! the whole history. The renderer understands the small Markdown subset the changelog
//! uses: `#`/`##`/`###` headings, `-` bullets (nested by indentation, wrapped lines continuing
//! the item), plain paragraphs, `**bold**` and `` `code` ``.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Ui};

pub mod export;

/// The changelog as built into this binary.
pub const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

/// One block of the changelog.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(u8, String),
    /// (nesting level from 0, text).
    Bullet(usize, String),
    Para(String),
}

/// Split the Markdown into blocks; indented lines without a marker continue the previous block.
pub fn parse(md: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push(Block::Para(String::new()));
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if let Some(h) = trimmed.strip_prefix("### ") {
            out.push(Block::Heading(3, h.into()));
        } else if let Some(h) = trimmed.strip_prefix("## ") {
            out.push(Block::Heading(2, h.into()));
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            out.push(Block::Heading(1, h.into()));
        } else if let Some(b) = trimmed.strip_prefix("- ") {
            out.push(Block::Bullet(indent / 2, b.into()));
        } else {
            // An indented line continues a bullet; any plain line continues a paragraph.
            match out.last_mut() {
                Some(Block::Bullet(_, t)) if indent > 0 => {
                    t.push(' ');
                    t.push_str(trimmed);
                }
                Some(Block::Para(t)) if !t.is_empty() => {
                    t.push(' ');
                    t.push_str(trimmed);
                }
                _ => out.push(Block::Para(trimmed.into())),
            }
        }
    }
    out.retain(|b| !matches!(b, Block::Para(t) if t.is_empty()));
    out
}

/// How a run of inline text is styled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Plain,
    Bold,
    Code,
}

/// Split one line into `**bold**`, `` `code` `` and plain runs (markers removed). An unclosed
/// marker is plain text, the rest of the line with it.
pub fn spans(text: &str) -> Vec<(Span, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let next_bold = rest.find("**");
        let next_code = rest.find('`');
        let (at, is_bold) = match (next_bold, next_code) {
            (Some(b), Some(c)) if c < b => (c, false),
            (Some(b), _) => (b, true),
            (None, Some(c)) => (c, false),
            (None, None) => {
                out.push((Span::Plain, rest));
                break;
            }
        };
        let marker = if is_bold { "**" } else { "`" };
        let after = &rest[at + marker.len()..];
        match after.find(marker) {
            Some(end) => {
                if at > 0 {
                    out.push((Span::Plain, &rest[..at]));
                }
                out.push((if is_bold { Span::Bold } else { Span::Code }, &after[..end]));
                rest = &after[end + marker.len()..];
            }
            None => {
                out.push((Span::Plain, rest));
                break;
            }
        }
    }
    out
}

/// `**bold**` and `` `code` `` spans of one line as a layout job.
fn inline(text: &str, size: f32, color: Color32, strong: Color32, code_bg: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let plain = TextFormat { font_id: FontId::proportional(size), color, ..Default::default() };
    let bold = TextFormat { font_id: FontId::proportional(size), color: strong, ..Default::default() };
    let code =
        TextFormat { font_id: FontId::monospace(size - 1.0), color: strong, background: code_bg, ..Default::default() };
    for (kind, t) in spans(text) {
        let fmt = match kind {
            Span::Plain => &plain,
            Span::Bold => &bold,
            Span::Code => &code,
        };
        job.append(t, 0.0, fmt.clone());
    }
    job
}

/// Draw the changelog (headings, bullets, paragraphs) into `ui`.
pub fn show(ui: &mut Ui, md: &str, text: Color32, dim: Color32, strong: Color32) {
    let code_bg = ui.visuals().extreme_bg_color;
    // A fixed text width: wrapping to "what's available" lets long lines widen the dialog.
    let width = ui.available_width().clamp(200.0, 700.0);
    ui.set_max_width(width);
    for block in parse(md) {
        match block {
            Block::Heading(1, _) => {}
            Block::Heading(2, h) => {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(h).size(17.0).strong().color(strong));
                ui.separator();
            }
            Block::Heading(_, h) => {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(h).size(14.5).strong().color(text));
            }
            Block::Bullet(level, t) => {
                let indent = 6.0 + level as f32 * 16.0;
                ui.horizontal_top(|ui| {
                    ui.add_space(indent);
                    ui.label(egui::RichText::new(if level == 0 { "•" } else { "◦" }).color(dim));
                    let mut job = inline(&t, 13.0, text, strong, code_bg);
                    job.wrap.max_width = (width - indent - 18.0).max(100.0);
                    ui.add(egui::Label::new(job).wrap());
                });
            }
            Block::Para(t) => {
                let mut job = inline(&t, 13.0, dim, text, code_bg);
                job.wrap.max_width = width;
                ui.add(egui::Label::new(job).wrap());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_changelog_subset() {
        let md = "# Changelog\n\n## 22 Nov\n\n### Game options\n- **Settings → Games**: switches\n  instead of `args`.\n  - nested item\nSome paragraph\ncontinued.\n";
        let b = parse(md);
        assert_eq!(b[0], Block::Heading(1, "Changelog".into()));
        assert_eq!(b[1], Block::Heading(2, "22 Nov".into()));
        assert_eq!(b[2], Block::Heading(3, "Game options".into()));
        assert_eq!(b[3], Block::Bullet(0, "**Settings → Games**: switches instead of `args`.".into()));
        assert_eq!(b[4], Block::Bullet(1, "nested item".into()));
        assert_eq!(b[5], Block::Para("Some paragraph continued.".into()));
        assert_eq!(b.len(), 6);
        assert!(
            parse(CHANGELOG).iter().any(|b| matches!(b, Block::Heading(2, _))),
            "the built-in changelog has entries"
        );
    }

    #[test]
    fn inline_spans() {
        let job = inline("a **b** `c` d **open", 13.0, Color32::GRAY, Color32::WHITE, Color32::BLACK);
        let text: String = job.text.clone();
        assert_eq!(text, "a b c d **open");
    }
}
