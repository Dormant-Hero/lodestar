use super::*;

const SAMPLE: &str = "# Lodestar changelog\n\nIntro with `v0.3.0` tags.\n\n## 0.5.0 · 22 Nov\n\n### New\n\
- **Tray icon**: shows next to\n  the clock.\n  - nested `code` item\n    - deeper\n- back at top\n\n### Notes\n\
Plain paragraph.\n";

#[test]
fn escapes_html_specials() {
    assert_eq!(
        escape_html(r#"<a href="x">Tom & Jerry's</a>"#),
        "&lt;a href=&quot;x&quot;&gt;Tom &amp; Jerry&#39;s&lt;/a&gt;"
    );
    let html = to_html("## 1.0 <beta>\n- **\"Q&A\" <Game>**: `a<b && c>d` and 'Tom's'\n", "1.0");
    assert!(html.contains("<h2>1.0 &lt;beta&gt;</h2>"), "{html}");
    assert!(html.contains("<strong>&quot;Q&amp;A&quot; &lt;Game&gt;</strong>"), "{html}");
    assert!(html.contains("<code>a&lt;b &amp;&amp; c&gt;d</code>"), "{html}");
    assert!(html.contains("&#39;Tom&#39;s&#39;"), "{html}");
    assert!(!html.contains("<Game>") && !html.contains("<beta>"));
}

#[test]
fn html_has_every_block_type() {
    let html = to_html(SAMPLE, "0.6.0");
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<title>Lodestar changelog</title>"));
    assert!(html.contains("prefers-color-scheme: dark"), "light / dark styles inline");
    assert!(html.contains("max-width"));
    assert!(!html.contains("<link") && !html.contains("<script"), "self-contained");
    assert!(html.contains("<h1>Lodestar changelog</h1>"));
    assert!(html.contains("<h2>0.5.0 · 22 Nov</h2>"));
    assert!(html.contains("<h3>New</h3>") && html.contains("<h3>Notes</h3>"));
    assert!(html.contains("<p>Intro with <code>v0.3.0</code> tags.</p>"));
    assert!(html.contains("<li><strong>Tray icon</strong>: shows next to the clock."));
    assert!(html.contains("<p>Plain paragraph.</p>"));
    assert!(html.contains("Lodestar 0.6.0"));
    assert_eq!(html.matches("<ul>").count(), html.matches("</ul>").count());
    assert_eq!(html.matches("<li>").count(), html.matches("</li>").count());
    assert_eq!(html.matches("<li>").count(), 4);
}

#[test]
fn html_nests_bullets() {
    let html = to_html(SAMPLE, "x");
    let compact: String = html.split_whitespace().collect::<Vec<_>>().join("");
    assert!(
        compact.contains(
            "<ul><li><strong>Trayicon</strong>:showsnexttotheclock.<ul><li>nested<code>code</code>item\
             <ul><li>deeper</li></ul></li></ul></li><li>backattop</li></ul><h3>Notes</h3>"
        ),
        "{html}"
    );
    // Skipping a level opens one list, not an empty item.
    let skip = to_html("- a\n    - c\n- d\n", "x");
    let compact: String = skip.split_whitespace().collect::<Vec<_>>().join("");
    assert!(compact.contains("<ul><li>a<ul><li>c</li></ul></li><li>d</li></ul>"), "{skip}");
    // A list that starts nested still opens and closes cleanly.
    let deep = to_html("  - only\n\nAfter\n", "x");
    let compact: String = deep.split_whitespace().collect::<Vec<_>>().join("");
    assert!(compact.contains("<ul><li>only</li></ul><p>After</p>"), "{deep}");
}

#[test]
fn plain_text_conversion() {
    let txt = to_text(SAMPLE);
    let expected = "\
Lodestar changelog
==================

Intro with v0.3.0 tags.

0.5.0 · 22 Nov
--------------

New
~~~

- Tray icon: shows next to the clock.
  - nested code item
    - deeper
- back at top

Notes
~~~~~

Plain paragraph.
";
    assert_eq!(txt, expected);
    assert!(!txt.contains("**") && !txt.contains('`'));
}

#[test]
fn plain_text_wraps_with_hanging_indent() {
    let long = "word ".repeat(60);
    let txt = to_text(&format!("- top\n  - {long}\n\n{long}\n"));
    for l in txt.lines() {
        assert!(l.chars().count() <= TEXT_WIDTH, "{l:?} is too long");
    }
    let lines: Vec<&str> = txt.lines().collect();
    assert_eq!(lines[0], "- top");
    assert!(lines[1].starts_with("  - word word"));
    assert!(lines[2].starts_with("    word"), "continuation lines line up under the text: {:?}", lines[2]);
    assert!(lines.iter().filter(|l| l.starts_with("word")).count() >= 3, "the paragraph wraps too");
    // A word longer than a line gets one of its own instead of being cut.
    let huge = "x".repeat(130);
    let t = to_text(&format!("short {huge} end\n"));
    assert_eq!(t, format!("short\n{huge}\nend\n"));
}

#[test]
fn formats_and_names() {
    assert_eq!(Format::Html.default_file_name(), "lodestar-changelog.html");
    assert_eq!(Format::Markdown.default_file_name(), "lodestar-changelog.md");
    assert_eq!(Format::Text.default_file_name(), "lodestar-changelog.txt");
    assert_eq!(Format::from_path(std::path::Path::new("/tmp/a.HTML")), Format::Html);
    assert_eq!(Format::from_path(std::path::Path::new("/tmp/a.md")), Format::Markdown);
    assert_eq!(Format::from_path(std::path::Path::new("/tmp/a")), Format::Text);
    assert_eq!(Format::Markdown.render("# x", "1"), "# x\n");
    // The real changelog converts without leftovers.
    let txt = Format::Text.render(crate::ui::whats_new::CHANGELOG, "1");
    assert!(!txt.contains("**"));
    assert!(txt.lines().all(|l| l.chars().count() <= TEXT_WIDTH || !l.contains(' ')));
    let html = Format::Html.render(crate::ui::whats_new::CHANGELOG, "1");
    // One heading per release in the changelog.
    let releases = crate::ui::whats_new::CHANGELOG.lines().filter(|l| l.starts_with("## ")).count();
    assert!(releases >= 1);
    assert_eq!(html.matches("<h2>").count(), releases);
    assert_eq!(html.matches("<ul>").count(), html.matches("</ul>").count());
}
