//! Catalogue tests: index parsing (hand-made fixtures in the real formats), installs through a
//! local mock HTTP server (127.0.0.1 only), update detection, the YAML subset.

use super::*;
use crate::emulation::zipread::tests::write_zip;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

// ------------------------------------------------------------------ mock server

type Routes = Arc<Mutex<HashMap<String, (u16, Vec<u8>)>>>;

struct Mock {
    base: String,
    routes: Routes,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Mock {
    fn start() -> Mock {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let routes: Routes = Arc::default();
        let hits: Arc<Mutex<Vec<String>>> = Arc::default();
        let (r, h) = (routes.clone(), hits.clone());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let mut reader = BufReader::new(s.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                }
                let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
                h.lock().unwrap().push(path.clone());
                let (status, body) = r.lock().unwrap().get(&path).cloned().unwrap_or((404, b"nope".to_vec()));
                let head =
                    format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        Mock { base, routes, hits }
    }

    fn route(&self, path: &str, body: impl Into<Vec<u8>>) {
        self.routes.lock().unwrap().insert(path.to_string(), (200, body.into()));
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn hit(&self, path: &str) -> bool {
        self.hits.lock().unwrap().iter().any(|h| h == path)
    }
}

const PLAYNITE_DESKTOP_YAML: &str = r#"AddonId: Harbour_Night_Theme
Type: ThemeDesktop
Name: Harbour Night
Author: Tester
ShortDescription: A calm dark theme.
InstallerManifestUrl: BASE/manifests/harbour.yaml
SourceUrl: https://example.invalid/harbour
Tags: ["dark", "calm", 'blue']
Links:
    GitHub: https://example.invalid/harbour
Screenshots:
    - Thumbnail: BASE/shots/harbour_thumb.jpg
      Image: BASE/shots/harbour.jpg
    - Image: BASE/shots/harbour2.jpg
Description: |-
    Line one.

    Line three: with a colon.
"#;

const PLAYNITE_FULLSCREEN_YAML: &str = r#"AddonId: 'Couch_Theme'
Type: ThemeFullscreen
InstallerManifestUrl: BASE/manifests/couch.yaml
ShortDescription: '*Needs nothing* big covers for the TV'
Name: 'Couch'
Author: 'Someone Else'
"#;

const PLAYNITE_GENERIC_YAML: &str =
    "AddonId: Some_Plugin\nType: GenericPlugin\nName: Plugin\nInstallerManifestUrl: BASE/x.yaml\n";

fn manifest(base: &str, versions: &[&str]) -> String {
    let mut s = String::from("AddonId: Harbour_Night_Theme\nPackages:\n");
    for v in versions {
        s.push_str(&format!(
            "  - Version: {v}\n    RequiredApiVersion: 2.6.0\n    ReleaseDate: 2026-01-0{}\n    PackageUrl: {base}/pkg/harbour_{v}.pthm\n    Changelog:\n      - Fixed things\n      - More: things\n",
            v.len() % 9
        ));
    }
    s
}

/// A minimal Playnite desktop theme package.
fn pthm(path: &Path, version: &str) {
    let yaml = format!(
        "Id: Harbour_Night_Theme\nName: Harbour Night\nAuthor: Tester\nVersion: {version}\nMode: Desktop\nThemeApiVersion: 2.6.0\n"
    );
    let constants = r##"<ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml">
<Color x:Key="TextColor">#FFF0F4F8</Color>
<Color x:Key="GlyphColor">#FF2A9D8F</Color>
<SolidColorBrush x:Key="TextBrush" Color="{DynamicResource TextColor}" />
<SolidColorBrush x:Key="GlyphBrush" Color="{DynamicResource GlyphColor}" />
</ResourceDictionary>"##;
    write_zip(path, &[("theme.yaml", yaml.as_bytes(), true), ("Constants.xaml", constants.as_bytes(), true)]);
}

fn playnite_db_zip(path: &Path, base: &str) {
    let d = PLAYNITE_DESKTOP_YAML.replace("BASE", base);
    let f = PLAYNITE_FULLSCREEN_YAML.replace("BASE", base);
    let g = PLAYNITE_GENERIC_YAML.replace("BASE", base);
    write_zip(
        path,
        &[
            ("PlayniteAddonDatabase-master/README.md", b"readme", true),
            ("PlayniteAddonDatabase-master/addons/themes_desktop/Tester_Harbour.yaml", d.as_bytes(), true),
            ("PlayniteAddonDatabase-master/addons/themes_fullscreen/Couch.yaml", f.as_bytes(), true),
            ("PlayniteAddonDatabase-master/addons/generic/Plugin.yaml", g.as_bytes(), true),
        ],
    );
}

// ------------------------------------------------------------------ parsing

#[test]
fn playnite_entries_and_manifests_parse() {
    let e = pnite::parse_entry(&PLAYNITE_DESKTOP_YAML.replace("BASE", "https://x.invalid")).unwrap();
    assert_eq!((e.id.as_str(), e.name.as_str(), e.kind), ("Harbour_Night_Theme", "Harbour Night", Kind::Desktop));
    assert_eq!(e.description, "A calm dark theme.");
    assert_eq!(e.tags, vec!["dark", "calm", "blue"]);
    assert_eq!(e.shots.len(), 2);
    assert_eq!(e.shots[0].thumb, "https://x.invalid/shots/harbour_thumb.jpg");
    assert_eq!(e.shots[1].thumb, "https://x.invalid/shots/harbour2.jpg", "no thumbnail → the image");
    let f = pnite::parse_entry(&PLAYNITE_FULLSCREEN_YAML.replace("BASE", "https://x.invalid")).unwrap();
    assert_eq!((f.id.as_str(), f.name.as_str(), f.kind), ("Couch_Theme", "Couch", Kind::Controller));
    assert_eq!(f.description, "*Needs nothing* big covers for the TV");
    assert!(pnite::parse_entry(PLAYNITE_GENERIC_YAML).is_none(), "not a theme");
    let p = pnite::latest_package(&manifest("https://x.invalid", &["1.9.0", "1.10.0", "1.2"])).unwrap();
    assert_eq!(p.version, "1.10.0");
    assert_eq!(p.url, "https://x.invalid/pkg/harbour_1.10.0.pthm");
    assert!(pnite::latest_package("AddonId: x\n").is_none());
}

#[test]
fn yaml_subset() {
    use yaml::{Y, parse};
    let y = parse(PLAYNITE_DESKTOP_YAML);
    assert_eq!(y.str("description"), Some("Line one.\n\nLine three: with a colon."));
    assert_eq!(y.get("Links").and_then(|l| l.str("github")), Some("https://example.invalid/harbour"));
    let y = parse(
        "a: 'it''s'\nb: \"q\\\"x\"\nc: plain # comment\nlist:\n- one\n- two\nnested:\n  - k: v\n    j: w\n  -\n    k: v2\nfold: >\n  a\n  b\nflow: [x,\n  y]\n",
    );
    assert_eq!(y.str("a"), Some("it's"));
    assert_eq!(y.str("b"), Some("q\"x"));
    assert_eq!(y.str("c"), Some("plain"));
    assert_eq!(y.get("list").unwrap().strings(), vec!["one", "two"]);
    let n = y.get("nested").unwrap().as_list();
    assert_eq!((n[0].str("k"), n[0].str("j"), n[1].str("k")), (Some("v"), Some("w"), Some("v2")));
    assert_eq!(y.str("fold"), Some("a b"));
    assert_eq!(y.get("flow").unwrap().strings(), vec!["x", "y"]);
    // Odd input never panics.
    for t in ["", "- ", ":", "a:\n  - [\n", "a: |\n", "x: [unclosed", "\t- a: b\n  c", "- - - x", &"a:\n ".repeat(80)] {
        let _ = parse(t);
    }
    assert_eq!(parse("- a\n- b"), Y::List(vec![Y::Str("a".into()), Y::Str("b".into())]));
}

#[test]
fn version_comparison() {
    use std::cmp::Ordering::*;
    assert_eq!(compare_versions("1.10.0", "1.9.9"), Greater);
    assert_eq!(compare_versions("1.2", "1.2.0"), Equal);
    assert_eq!(compare_versions("v2.0", "1.99"), Greater);
    assert_eq!(compare_versions("1.0.0", "1.0.1"), Less);
    assert!(is_newer(Index::Playnite, "1.0.0", "1.0.1"));
    assert!(!is_newer(Index::Playnite, "1.0.1", "1.0.1"));
    assert!(!is_newer(Index::Playnite, "2.0", "1.9"), "a newer local copy isn't an update");
}

// ------------------------------------------------------------------ fetching, installing, updating

#[test]
fn fetch_caches_and_falls_back_when_offline() {
    let mock = Mock::start();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("cache");
    let db = tmp.path().join("db.zip");
    playnite_db_zip(&db, &mock.base);
    mock.route("/db.zip", std::fs::read(&db).unwrap());
    let ep = Endpoints { playnite: Some(mock.url("/db.zip")) };
    let cat = fetch(&ep, &dir);
    assert!(cat.problems.is_empty(), "{:?}", cat.problems);
    assert_eq!(cat.entries.len(), 2, "2 Playnite themes (the plug-in is left out)");
    assert!(cat.fetched_playnite > 0);
    assert!(cat.age(now()) < 60);
    assert_eq!(load_cached(&dir).unwrap().entries, cat.entries, "saved");

    // Offline: the saved copy, with a clear note.
    let gone = Endpoints { playnite: Some(mock.url("/missing.zip")) };
    let cat2 = fetch(&gone, &dir);
    assert_eq!(cat2.entries, cat.entries);
    assert_eq!(cat2.problems.len(), 1, "{:?}", cat2.problems);
    assert!(cat2.problems[0].contains("saved earlier") && cat2.problems[0].contains("404"), "{:?}", cat2.problems);

    // Never fetched + no network allowed (screenshot run): empty, with a message.
    let fresh = fetch(&Endpoints { playnite: None }, &tmp.path().join("other"));
    assert!(fresh.entries.is_empty());
    assert_eq!(fresh.problems.len(), 1);
}

#[test]
fn installs_a_playnite_theme_package_and_detects_updates() {
    let mock = Mock::start();
    let tmp = tempfile::tempdir().unwrap();
    let themes = tmp.path().join("themes");
    let work = tmp.path().join("work");
    let entry = pnite::parse_entry(&PLAYNITE_DESKTOP_YAML.replace("BASE", &mock.base)).unwrap();
    mock.route("/manifests/harbour.yaml", manifest(&mock.base, &["1.0.0", "1.1.0"]));
    let pkg = tmp.path().join("p.pthm");
    pthm(&pkg, "1.1.0");
    mock.route("/pkg/harbour_1.1.0.pthm", std::fs::read(&pkg).unwrap());

    let p = install::Progress::default();
    let rep = install::install(&entry, &themes, &work, &p).unwrap();
    assert!(!rep.controller && rep.community);
    assert_eq!((rep.name.as_str(), rep.version.as_str()), ("Harbour Night", "1.1.0"));
    assert!(!rep.carried.is_empty());
    assert!(mock.hit("/pkg/harbour_1.1.0.pthm") && !mock.hit("/pkg/harbour_1.0.0.pthm"), "the newest package only");
    let dir = themes.join(&rep.key);
    assert_eq!(read_record(&dir).unwrap().version, "1.1.0");

    let cat = Catalog { entries: vec![entry.clone()], ..Default::default() };
    let mgr = crate::theme::ThemeManager::new(themes.clone());
    let list = installed::list(mgr.list());
    let c = checks(&list, Some(&cat));
    assert_eq!(c.len(), 1);
    assert!(find_updates(&c).0.is_empty());
    mock.route("/manifests/harbour.yaml", manifest(&mock.base, &["1.0.0", "1.1.0", "1.2.0"]));
    let (ups, _) = find_updates(&c);
    assert_eq!(ups.len(), 1);
    assert_eq!((ups[0].installed.as_str(), ups[0].latest.as_str()), ("1.1.0", "1.2.0"));

    // A theme imported by hand (not from the list) is matched by its Playnite id.
    let hand = tempfile::tempdir().unwrap();
    let rep3 = install::import_playnite(&pkg, hand.path(), &p).unwrap();
    assert!(!rep3.community);
    let mgr = crate::theme::ThemeManager::new(hand.path().to_path_buf());
    let list = installed::list(mgr.list());
    let t = list.iter().find(|t| t.key == rep3.key).unwrap();
    assert!(matches!(&t.from, installed::From::Playnite { playnite_id, .. } if playnite_id == "Harbour_Night_Theme"));
    let c = checks(&list, Some(&cat));
    assert_eq!(c.len(), 1, "matched through the Playnite id");
    assert_eq!(c[0].version, "1.1.0");
    assert!(checks(&list, None).is_empty(), "no list, nothing to match against");

    // Remove.
    installed::remove(&themes, &rep.key).unwrap();
    assert!(!dir.exists());
    assert!(installed::remove(&themes, "..").is_err());
    assert!(installed::remove(&themes, "Lodestar").is_err(), "nothing there / built-in");
}

#[test]
fn downloads_respect_the_size_limit() {
    let mock = Mock::start();
    mock.route("/big", vec![7u8; 5000]);
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("big.bin");
    let (n, c) = (std::sync::atomic::AtomicU64::new(0), std::sync::atomic::AtomicBool::new(false));
    assert!(net::download(&mock.url("/big"), &dest, 1000, &n, &c).is_err());
    assert!(!dest.exists() && !dest.with_extension("part").exists());
    assert_eq!(net::download(&mock.url("/big"), &dest, 10_000, &n, &c).unwrap(), 5000);
    assert!(net::get(&mock.url("/big"), 100).is_err());
    assert!(net::get(&mock.url("/nothing"), 100).unwrap_err().contains("404"));
}
