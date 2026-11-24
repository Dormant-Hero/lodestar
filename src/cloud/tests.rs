use super::*;
use crate::procfs::fake;
use crate::test_support::{sv, write_exe, write_file};
use std::io::{Read, Write};

fn cmd_of(spec: &LaunchSpec) -> (String, Vec<String>) {
    match spec {
        LaunchSpec::Command { program, args, .. } => (program.clone(), args.clone()),
        LaunchSpec::Url(u) => ("xdg-open".into(), vec![u.clone()]),
        other => panic!("unexpected {other:?}"),
    }
}

fn gfn_app() -> FoundApp {
    FoundApp {
        service: Service::GeForceNow,
        name: "GeForce NOW app",
        install: Install::Flatpak("com.nvidia.geforcenow".into()),
        fullscreen_arg: None,
        procs: &[],
        icon: None,
    }
}

fn greenlight() -> FoundApp {
    FoundApp {
        service: Service::Xbox,
        name: "Greenlight",
        install: Install::Flatpak("io.github.unknownskl.greenlight".into()),
        fullscreen_arg: Some("--fullscreen"),
        procs: &["greenlight"],
        icon: None,
    }
}

fn browser(key: &str, family: Family, install: Install) -> FoundBrowser {
    let def = BROWSERS.iter().find(|b| key.starts_with(b.key)).unwrap();
    FoundBrowser { key: key.into(), name: def.name.into(), family, install, procs: def.procs }
}

fn chrome() -> FoundBrowser {
    browser("chrome", Family::Chromium, Install::Native("/usr/bin/google-chrome-stable".into()))
}

fn firefox() -> FoundBrowser {
    browser("firefox", Family::Firefox, Install::Native("/usr/bin/firefox".into()))
}

fn firefox_flatpak() -> FoundBrowser {
    browser("firefox@flatpak", Family::Firefox, Install::Flatpak("org.mozilla.firefox".into()))
}

fn entry(service: Service, open: Open, fullscreen: bool) -> Entry {
    Entry { service, open, fullscreen }
}

const GFN: &str = "https://play.geforcenow.com";
const FH5: &str = "https://www.xbox.com/en-US/play/games/forza-horizon-5/9NKX70BBCDRN";

// ------------------------------------------------------------------------------- presets → argv

#[test]
fn geforce_now_app_wins_without_a_link() {
    let found = Found { apps: vec![gfn_app()], browsers: vec![chrome()], default_browser: None };
    let p = plan(&entry(Service::GeForceNow, Open::Auto, true), GFN, false, &found);
    assert_eq!(cmd_of(&p.spec), ("flatpak".into(), sv(&["run", "com.nvidia.geforcenow"])));
    assert!(matches!(p.how, How::App { name: "GeForce NOW app", .. }));
}

#[test]
fn a_game_link_opens_in_a_browser_window_unless_the_app_is_chosen() {
    let found = Found { apps: vec![greenlight()], browsers: vec![chrome()], default_browser: None };
    let p = plan(&entry(Service::Xbox, Open::Auto, true), FH5, true, &found);
    assert_eq!(
        cmd_of(&p.spec),
        ("/usr/bin/google-chrome-stable".into(), vec!["--start-fullscreen".to_string(), format!("--app={FH5}")])
    );
    // Chosen explicitly: the app (with its full-screen switch), even with a link.
    let p = plan(&entry(Service::Xbox, Open::App, true), FH5, true, &found);
    assert_eq!(cmd_of(&p.spec), ("flatpak".into(), sv(&["run", "io.github.unknownskl.greenlight", "--fullscreen"])));
    let p = plan(&entry(Service::Xbox, Open::App, false), FH5, true, &found);
    assert_eq!(cmd_of(&p.spec).1, sv(&["run", "io.github.unknownskl.greenlight"]));
}

#[test]
fn browser_arguments_per_family_and_full_screen() {
    let u = "https://luna.amazon.com";
    assert_eq!(browser_args(Family::Chromium, u, true), sv(&["--start-fullscreen", "--app=https://luna.amazon.com"]));
    assert_eq!(browser_args(Family::Chromium, u, false), sv(&["--app=https://luna.amazon.com"]));
    assert_eq!(browser_args(Family::Firefox, u, true), sv(&["--kiosk", u]));
    assert_eq!(browser_args(Family::Firefox, u, false), sv(&["--new-window", u]));
    // A Flatpak browser runs through flatpak.
    let found = Found { apps: vec![], browsers: vec![firefox_flatpak()], default_browser: None };
    let p = plan(&entry(Service::Boosteroid, Open::Browser("firefox@flatpak".into()), true), u, true, &found);
    assert_eq!(cmd_of(&p.spec), ("flatpak".into(), sv(&["run", "org.mozilla.firefox", "--kiosk", u])));
}

#[test]
fn automatic_prefers_a_browser_the_service_supports() {
    // Firefox is the default, but GeForce NOW doesn't stream there: Chrome is used.
    let found = Found { apps: vec![], browsers: vec![chrome(), firefox()], default_browser: Some("firefox".into()) };
    let p = plan(&entry(Service::GeForceNow, Open::Auto, false), GFN, false, &found);
    assert_eq!(cmd_of(&p.spec), ("/usr/bin/google-chrome-stable".into(), vec![format!("--app={GFN}")]));
    // Boosteroid is fine with Firefox: the default browser is used.
    let b = "https://cloud.boosteroid.com";
    let p = plan(&entry(Service::Boosteroid, Open::Auto, true), b, false, &found);
    assert_eq!(cmd_of(&p.spec), ("/usr/bin/firefox".into(), sv(&["--kiosk", b])));
    // Only Firefox: it is used, with a warning for GeForce NOW.
    let only_ff = Found { apps: vec![], browsers: vec![firefox()], default_browser: None };
    let p = plan(&entry(Service::GeForceNow, Open::Auto, true), GFN, false, &only_ff);
    assert!(matches!(p.how, How::Window { family: Family::Firefox, .. }));
    assert!(warning(Service::GeForceNow, &p, &only_ff).unwrap().contains("Firefox"));
    assert_eq!(warning(Service::Boosteroid, &p, &only_ff), None);
    // Nothing found: a tab in the default browser.
    let none = Found::default();
    let p = plan(&entry(Service::Luna, Open::Auto, true), "https://luna.amazon.com", false, &none);
    assert_eq!(p.spec, LaunchSpec::Url("https://luna.amazon.com".into()));
    assert_eq!(p.how, How::Tab);
}

#[test]
fn a_chosen_way_that_is_gone_falls_back_to_automatic() {
    let found = Found { apps: vec![], browsers: vec![chrome()], default_browser: None };
    let p = plan(&entry(Service::GeForceNow, Open::App, true), GFN, false, &found);
    assert!(matches!(p.how, How::Window { .. }), "no GeForce NOW app any more");
    let p = plan(&entry(Service::GeForceNow, Open::Browser("edge".into()), true), GFN, false, &found);
    assert!(matches!(&p.how, How::Window { name, .. } if name == "Google Chrome"));
    let p = plan(&entry(Service::GeForceNow, Open::Tab, true), GFN, false, &found);
    assert_eq!(p.spec, LaunchSpec::Url(GFN.into()));
}

#[test]
fn choices_list_apps_browsers_and_the_tab() {
    let found = Found {
        apps: vec![gfn_app(), greenlight()],
        browsers: vec![chrome(), firefox(), firefox_flatpak()],
        default_browser: Some("firefox".into()),
    };
    let labels: Vec<String> = choices(Service::Xbox, &found).into_iter().map(|(_, l)| l).collect();
    assert_eq!(
        labels,
        sv(&[
            "Automatic",
            "Greenlight",
            "Google Chrome window",
            "Firefox window",
            "Firefox window",
            "Default browser tab (Firefox)"
        ])
    );
    assert!(!choices(Service::Luna, &found).iter().any(|(o, _)| *o == Open::App));
}

// ------------------------------------------------------------------------------- entries

#[test]
fn entries_round_trip_through_the_source_id() {
    for e in [
        entry(Service::GeForceNow, Open::Auto, true),
        entry(Service::Xbox, Open::App, false),
        entry(Service::Luna, Open::Browser("firefox@flatpak".into()), true),
        entry(Service::Other, Open::Tab, false),
    ] {
        let sid = e.source_id("1700000000-abcd");
        assert!(sid.starts_with("cloud:"), "{sid}");
        assert_eq!(Entry::parse(&sid), Some(e));
    }
    assert_eq!(Entry::source_id(&entry(Service::Boosteroid, Open::Auto, true), "n"), "cloud:boosteroid:auto:fs:n");
    for bad in [
        "manual-1-2",
        "cloud:nope:auto:fs:n",
        "cloud:gfn:what:fs:n",
        "cloud:gfn:auto:x:n",
        "cloud:gfn:b-:fs:n",
        "cloud:gfn",
    ] {
        assert_eq!(Entry::parse(bad), None, "{bad}");
    }
}

fn cloud_game(sid: &str, url: &str) -> Game {
    let mut g = Game::blank("Fortnite");
    g.id = 7;
    g.source_id = sid.into();
    g.launch = Some(LaunchSpec::Url(url.into()));
    g.platform = Some(PLATFORM.into());
    g
}

#[test]
fn games_name_their_service_as_source() {
    let g = cloud_game("cloud:xbox:auto:fs:n", FH5);
    assert!(g.is_cloud());
    assert_eq!(g.source_name(), "Xbox Cloud Gaming");
    assert_eq!(g.source_group_key(), "cloud-xbox");
    let mut plain = Game::blank("Doom");
    assert!(!plain.is_cloud());
    assert_eq!(plain.source_name(), "Manual");
    // Only hand-added games are cloud games.
    plain.source = Source::Steam;
    plain.source_id = "cloud:gfn:auto:fs:n".into();
    assert!(!plain.is_cloud());
    assert_eq!(plain.source_name(), "Steam");
}

#[test]
fn game_url_and_link_detection() {
    let e = entry(Service::GeForceNow, Open::Auto, true);
    let g = cloud_game("cloud:gfn:auto:fs:n", "https://play.geforcenow.com/");
    assert_eq!(game_url(&g, &e).as_deref(), Some("https://play.geforcenow.com/"));
    assert!(!is_game_link(Service::GeForceNow, "https://play.geforcenow.com/"));
    assert!(is_game_link(Service::Xbox, FH5));
    assert!(is_game_link(Service::Other, "https://example.com/play"));
    let mut odd = g.clone();
    odd.launch = None;
    assert_eq!(game_url(&odd, &e).as_deref(), Some(GFN));
    assert!(is_web("https://x.y") && !is_web("ftp://x") && !is_web("https://"));
}

// ------------------------------------------------------------------------------- links → names

#[test]
fn names_from_links() {
    let cases = [
        (FH5, Some("Forza Horizon 5")),
        ("https://www.xbox.com/en-GB/play/launch/the-outer-worlds-2/9NSPRSXXZZLG", Some("The Outer Worlds 2")),
        ("https://www.xbox.com/en-US/games/store/starfield/9NCJSXWZTP88", Some("Starfield")),
        ("https://store.steampowered.com/app/1091500/Cyberpunk_2077/", Some("Cyberpunk 2077")),
        ("https://luna.amazon.com/game/assassins-creed-mirage/B0BXXXXXXX", Some("Assassins Creed Mirage")),
        ("https://example.com/games/final-fantasy-vii-remake", Some("Final Fantasy VII Remake")),
        ("https://play.geforcenow.com/mall/#/deeplink?game-id=8a6d7c5e-1234-4abc-9def-0123456789ab", None),
        ("https://play.geforcenow.com/games?game-id=1&title=Baldur%27s%20Gate%203", Some("Baldur's Gate 3")),
        ("https://play.geforcenow.com", None),
        ("https://www.xbox.com/play", None),
        ("not a link", None),
    ];
    for (url, want) in cases {
        assert_eq!(name_from_url(url).as_deref(), want, "{url}");
    }
    assert_eq!(tidy_slug("lord-of-the-rings-return-to-moria"), "Lord of the Rings Return to Moria");
}

#[test]
fn services_from_links() {
    assert_eq!(Service::for_url("https://play.geforcenow.com/mall/"), Some(Service::GeForceNow));
    assert_eq!(Service::for_url(FH5), Some(Service::Xbox));
    assert_eq!(Service::for_url("https://luna.amazon.co.uk/?g=s"), Some(Service::Luna));
    assert_eq!(Service::for_url("https://cloud.boosteroid.com/"), Some(Service::Boosteroid));
    assert_eq!(Service::for_url("https://store.steampowered.com/app/1"), None);
    assert_eq!(Service::for_url("https://notxbox.com.evil.example/"), None);
    for s in Service::ALL {
        assert_eq!(Service::from_key(s.key()), Some(s));
        if let Some(h) = s.home() {
            assert_eq!(Service::for_url(h), Some(s), "{h}");
        }
    }
}

// ------------------------------------------------------------------------------- detection

#[test]
fn detects_apps_and_browsers_on_a_fixture_machine() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let m = Machine::under(root);
    assert_eq!(detect(&m), Found::default(), "an empty machine");

    std::fs::create_dir_all(root.join("var/lib/flatpak/app/com.nvidia.geforcenow/current/active")).unwrap();
    write_file(
        &root.join("var/lib/flatpak/exports/share/icons/hicolor/128x128/apps/com.nvidia.geforcenow.png"),
        b"png",
    );
    std::fs::create_dir_all(root.join("home/.local/share/flatpak/app/io.github.unknownskl.greenlight/current"))
        .unwrap();
    std::fs::create_dir_all(root.join("home/.local/share/flatpak/app/org.mozilla.firefox/current")).unwrap();
    write_exe(&root.join("usr/bin/firefox"), "#!/bin/sh\n");
    write_exe(&root.join("usr/bin/chromium"), "#!/bin/sh\n");
    write_exe(&root.join("opt/BoosteroidGamesS.R.L./bin/Boosteroid"), "#!/bin/sh\n");
    write_file(&root.join("usr/bin/google-chrome"), "not executable");
    write_file(
        &root.join("home/.config/mimeapps.list"),
        "[Added Associations]\nx-scheme-handler/https=chromium.desktop;\n\n[Default Applications]\nx-scheme-handler/https=org.mozilla.firefox.desktop;firefox.desktop;\n",
    );
    let f = detect(&m);
    let apps: Vec<(Service, &str, &Install)> = f.apps.iter().map(|a| (a.service, a.name, &a.install)).collect();
    assert_eq!(
        apps,
        vec![
            (Service::GeForceNow, "GeForce NOW app", &Install::Flatpak("com.nvidia.geforcenow".into())),
            (Service::Xbox, "Greenlight", &Install::Flatpak("io.github.unknownskl.greenlight".into())),
            (
                Service::Boosteroid,
                "Boosteroid app",
                &Install::Native("/opt/BoosteroidGamesS.R.L./bin/Boosteroid".into())
            ),
        ]
    );
    assert!(f.apps[0].icon.as_ref().is_some_and(|p| p.ends_with("com.nvidia.geforcenow.png")));
    let browsers: Vec<(&str, &str)> = f.browsers.iter().map(|b| (b.key.as_str(), b.name.as_str())).collect();
    assert_eq!(
        browsers,
        vec![("chromium", "Chromium"), ("firefox", "Firefox"), ("firefox@flatpak", "Firefox (Flatpak)")]
    );
    assert_eq!(f.default_browser.as_deref(), Some("firefox@flatpak"));
    assert_eq!(f.default_name(), Some("Firefox (Flatpak)"));
    assert_eq!(f.browser("chromium").map(|b| &b.install), Some(&Install::Native(root.join("usr/bin/chromium"))));
}

#[test]
fn mimeapps_default_reads_only_the_defaults_section() {
    let t = "[Added Associations]\nx-scheme-handler/https=a.desktop;\n[Default Applications]\nx-scheme-handler/http=b.desktop\nx-scheme-handler/https= ;c.desktop;\n";
    assert_eq!(mime_default(t, "x-scheme-handler/https").as_deref(), Some("c.desktop"));
    assert_eq!(mime_default(t, "x-scheme-handler/http").as_deref(), Some("b.desktop"));
    assert_eq!(mime_default(t, "text/html"), None);
}

#[test]
fn already_running_native_and_flatpak() {
    let t = tempfile::tempdir().unwrap();
    let proc_root = t.path().join("proc");
    let run = t.path().join("run/.flatpak");
    fake::boot(&proc_root, 1000);
    fake::add(&proc_root, fake::P::new(100, 5, &["/app/lib/firefox/firefox"]));
    let env = RunEnv { proc_root: proc_root.clone(), flatpak_run: Some(run.clone()) };
    let native_ff = How::Window {
        name: "Firefox".into(),
        family: Family::Firefox,
        install: Install::Native("/usr/bin/firefox".into()),
        procs: &["firefox", "firefox-bin"],
    };
    // A Flatpak Firefox running doesn't make the native one "running".
    assert!(!already_running(&native_ff, &env));
    fake::add(&proc_root, fake::P::new(200, 6, &["/usr/lib/firefox/firefox", "-contentproc"]));
    assert!(already_running(&native_ff, &env));

    let gfn =
        How::App { name: "GeForce NOW app", install: Install::Flatpak("com.nvidia.geforcenow".into()), procs: &[] };
    assert!(!already_running(&gfn, &env));
    write_file(&run.join("1234/info"), "[Application]\nname=com.nvidia.geforcenow\n");
    write_file(&run.join("1234/pid"), "300\n");
    assert!(!already_running(&gfn, &env), "the instance's process is gone");
    fake::add(&proc_root, fake::P::new(300, 7, &["bwrap"]));
    assert!(already_running(&gfn, &env));
    assert!(!already_running(&How::Tab, &env));
}

/// Everything that reads the published detection, in one test (it is process-wide).
#[test]
fn launching_uses_the_published_detection() {
    publish(Found { apps: vec![], browsers: vec![chrome()], default_browser: Some("chrome".into()) });
    let t = tempfile::tempdir().unwrap();
    let proc_root = t.path().join("proc");
    fake::boot(&proc_root, 1000);
    let env = RunEnv { proc_root: proc_root.clone(), flatpak_run: None };

    // resolve: the browser window, not xdg-open of the stored link.
    let g = cloud_game("cloud:xbox:auto:fs:n", FH5);
    let host = crate::launch::Host { steam: crate::launch::SteamCmd::XdgOpen, home: None, vars: Default::default() };
    let r = crate::launch::resolve_with(&g, &host).unwrap();
    assert_eq!(r.program, "/usr/bin/google-chrome-stable");
    assert_eq!(r.args, vec!["--start-fullscreen".to_string(), format!("--app={FH5}")]);
    assert_eq!(r.kind, crate::launch::ResolvedKind::Process);
    assert!(crate::launch::command_template(&g).unwrap().contains("--app="));

    // Chrome not running yet: followed as usual.
    let mut lp = crate::launch::LaunchPlan::default();
    assert_eq!(launch_note_with(&g, None, &mut lp, &env), None);
    assert_eq!(lp.tracking, TrackingMode::Default);
    // Already open: not followed, and the user is told.
    fake::add(&proc_root, fake::P::new(42, 1, &["/opt/google/chrome/chrome"]));
    let note = launch_note_with(&g, None, &mut lp, &env).unwrap();
    assert!(note.contains("Google Chrome, which was already open"), "{note}");
    assert_eq!(lp.tracking, TrackingMode::None);

    // A tab: never followed.
    let tab = cloud_game("cloud:luna:tab:fs:n", "https://luna.amazon.com/game/x/B0");
    let mut lp = crate::launch::LaunchPlan::default();
    assert!(launch_note_with(&tab, None, &mut lp, &env).unwrap().contains("browser tab"));
    assert_eq!(lp.tracking, TrackingMode::None);
    assert_eq!(crate::launch::resolve_with(&tab, &host).unwrap().program, "xdg-open");

    // A custom command or the user's own Play action: Lodestar leaves it alone.
    let mut custom = tab.clone();
    custom.launch_override = Some(LaunchOverride { command: Some("my-player".into()), ..Default::default() });
    let mut lp = crate::launch::LaunchPlan::default();
    assert_eq!(launch_note_with(&custom, None, &mut lp, &env), None);
    assert_eq!(crate::launch::resolve_with(&custom, &host).unwrap().program, "my-player");
    let act = GameAction { name: "Play".into(), path: "/bin/x".into(), play: true, ..Default::default() };
    assert_eq!(launch_note_with(&tab, Some(&act), &mut lp, &env), None);
    assert_eq!(lp.tracking, TrackingMode::Default);
    // Not a cloud game.
    assert_eq!(launch_note_with(&Game::blank("x"), None, &mut lp, &env), None);
}

// ------------------------------------------------------------------------------- adding

#[test]
fn adding_a_cloud_game() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = crate::app::AppState::for_test(dir.path());
    let e = entry(Service::Xbox, Open::Auto, true);
    let none = st.add_cloud_game(NewCloudGame { name: " ".into(), entry: e.clone(), link: None });
    assert_eq!(none, None);
    let bad = st.add_cloud_game(NewCloudGame {
        name: "Web".into(),
        entry: entry(Service::Other, Open::Auto, true),
        link: None,
    });
    assert_eq!(bad, None, "another service needs a link");

    let id = st
        .add_cloud_game(NewCloudGame { name: "Forza Horizon 5".into(), entry: e.clone(), link: Some(FH5.into()) })
        .unwrap();
    let g = st.library.get(id).unwrap().clone();
    assert_eq!(g.source, Source::Manual);
    assert_eq!(Entry::of(&g), Some(e.clone()));
    assert_eq!(g.platform.as_deref(), Some(PLATFORM));
    assert_eq!(g.launch, Some(LaunchSpec::Url(FH5.into())));
    assert_eq!(g.edits.links, vec![GameLink { name: "Play on Xbox Cloud Gaming".into(), url: FH5.into() }]);
    assert_eq!(st.cloud_duplicates(Service::Xbox, "forza horizon 5"), vec![id]);
    assert!(st.cloud_duplicates(Service::GeForceNow, "Forza Horizon 5").is_empty());

    // Without a link: the service's home page, no extra link.
    let home = st
        .add_cloud_game(NewCloudGame {
            name: "Fortnite".into(),
            entry: entry(Service::GeForceNow, Open::App, false),
            link: None,
        })
        .unwrap();
    let h = st.library.get(home).unwrap();
    assert_eq!(h.launch, Some(LaunchSpec::Url(GFN.into())));
    assert!(h.edits.links.is_empty());

    // Never offered as "the same game" as a scanned copy.
    let mut steam = Game::blank("Forza Horizon 5");
    steam.id = 999;
    steam.source = Source::Steam;
    steam.source_id = "1551360".into();
    let games = vec![g.clone(), steam];
    assert!(crate::addgame::compute_merge_suggestions(&games, &Default::default()).is_empty());

    // Edit → Installation changes how it opens; the service and the game stay.
    let sid = g.source_id.clone();
    st.set_cloud_entry(id, &entry(Service::GeForceNow, Open::Tab, false));
    let changed = st.library.get(id).unwrap();
    assert_eq!(Entry::of(changed), Some(entry(Service::Xbox, Open::Tab, false)));
    assert_eq!(Entry::nonce(&changed.source_id), Entry::nonce(&sid));
    assert_ne!(changed.source_id, sid);
    assert_eq!(changed.edits.links.len(), 1);

    // Removing deletes it (a hand-added game).
    st.remove_game(id);
    assert!(st.library.get(id).is_none());
}

// ------------------------------------------------------------------------------- GeForce NOW list

const LIST: &str = r#"[
 {"id":1,"title":"Cyberpunk 2077®","status":"AVAILABLE"},
 {"id":2,"title":"Fortnite™","status":"AVAILABLE"},
 {"id":3,"title":"Old Game","status":"REMOVED"},
 {"id":4,"title":"Cyber Hook","status":"AVAILABLE"},
 {"id":5,"title":"Fortnite™","status":"AVAILABLE"},
 {"id":6,"title":"The Cybernetic Saga"}
]"#;

#[test]
fn gfn_titles_parse_and_suggest() {
    let v = gfn::parse_titles(LIST.as_bytes()).unwrap();
    assert_eq!(v, sv(&["Cyber Hook", "Cyberpunk 2077", "Fortnite", "The Cybernetic Saga"]));
    assert!(gfn::parse_titles(b"{}").is_err());
    assert_eq!(gfn::suggest("cyber", &v, 5), sv(&["Cyber Hook", "Cyberpunk 2077", "The Cybernetic Saga"]));
    assert_eq!(gfn::suggest("punk", &v, 5), sv(&["Cyberpunk 2077"]));
    assert_eq!(gfn::suggest("c", &v, 5), Vec::<String>::new());
    assert_eq!(gfn::suggest("fortnite", &v, 1), sv(&["Fortnite"]));
    assert!(gfn::contains("cyberpunk 2077", &v));
    assert!(!gfn::contains("cyberpunk", &v));
}

/// A local HTTP server answering `n` requests with `status` and `body`; returns its URL.
fn mock_server(n: usize, status: u16, body: &'static str) -> (String, std::thread::JoinHandle<usize>) {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/list.json", l.local_addr().unwrap());
    let h = std::thread::spawn(move || {
        let mut served = 0;
        for s in l.incoming().take(n) {
            let mut s = s.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let head = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(body.as_bytes()).unwrap();
            served += 1;
        }
        served
    });
    (url, h)
}

#[test]
fn gfn_list_is_fetched_once_and_cached() {
    let t = tempfile::tempdir().unwrap();
    let net = crate::art::net::UreqNet::new();
    // Offline with nothing cached: no names, no request.
    assert_eq!(gfn::load(t.path(), &net, "http://127.0.0.1:9/never", true).unwrap(), Vec::<String>::new());
    let (url, h) = mock_server(1, 200, LIST);
    let v = gfn::load(t.path(), &net, &url, false).unwrap();
    assert_eq!(v.len(), 4);
    assert_eq!(h.join().unwrap(), 1);
    assert!(gfn::cache_file(t.path()).is_file());
    // Fresh cache: no second request (the server is gone; a request would fail).
    assert_eq!(gfn::load(t.path(), &net, &url, false).unwrap().len(), 4);
    assert_eq!(gfn::load(t.path(), &net, &url, true).unwrap().len(), 4);
    // A stale cache is used when fetching fails.
    let old = std::time::SystemTime::now() - gfn::FRESH - std::time::Duration::from_secs(60);
    std::fs::File::options().write(true).open(gfn::cache_file(t.path())).unwrap().set_modified(old).unwrap();
    let (url, h) = mock_server(1, 500, "");
    assert_eq!(gfn::load(t.path(), &net, &url, false).unwrap().len(), 4);
    assert_eq!(h.join().unwrap(), 1);
    // No cache and an error: the error.
    let t2 = tempfile::tempdir().unwrap();
    let (url, h) = mock_server(1, 404, "");
    assert!(gfn::load(t2.path(), &net, &url, false).unwrap_err().contains("404"));
    h.join().unwrap();
}
