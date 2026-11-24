use super::*;

#[test]
fn wayland_detection_matches_winit() {
    let env = |vars: &'static [(&'static str, &'static str)]| {
        move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| std::ffi::OsString::from(v))
    };
    assert!(is_wayland(env(&[("WAYLAND_DISPLAY", "wayland-1"), ("DISPLAY", ":0")])));
    assert!(is_wayland(env(&[("WAYLAND_SOCKET", "5")])));
    assert!(!is_wayland(env(&[("WAYLAND_DISPLAY", ""), ("DISPLAY", ":0")])));
    assert!(!is_wayland(env(&[("DISPLAY", ":0")])));
}

#[test]
fn screenshot_waits_for_desktop_keys() {
    assert_eq!(desktop_key_frames(None), 0);
    assert_eq!(desktop_key_frames(Some("")), 0);
    // The desktop UI injects key k at its frame 25 + 3k; the screenshot must not settle
    // (starting at app frame 20 + this) before the last one.
    for n in 1..50u64 {
        let keys = vec!["down"; n as usize].join(",");
        assert!(20 + desktop_key_frames(Some(&keys)) > 25 + 3 * n);
    }
}

#[test]
fn user_art_set_replace_and_reset() {
    use crate::art::ArtSlot;
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::for_test(dir.path());
    use crate::model::ScannedGame;
    let scanned = |st: &mut AppState, games: &[ScannedGame], sources: &[Source]| {
        Db::open(&st.paths.db_file()).unwrap().merge_scan(games, sources).unwrap();
        st.reload_library();
    };
    let mut sg = ScannedGame::new(Source::Steam, "70", "Half-Life");
    let scan_cover = dir.path().join("scan_cover.jpg");
    sg.art.cover = Some(scan_cover.clone());
    sg.art.hero = Some(dir.path().join("scan_hero.jpg"));
    scanned(&mut st, &[sg.clone()], &[Source::Steam]);
    let id = st.library.games.iter().find(|g| g.name == "Half-Life").unwrap().id;
    let png = |w: u32| {
        let mut out = std::io::Cursor::new(vec![]);
        image::RgbImage::new(w, 3).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let src = dir.path().join("mine.png");
    std::fs::write(&src, png(2)).unwrap();
    st.set_art_from_file(id, ArtSlot::Cover, &src);
    assert!(st.art_state.is_applying(id, ArtSlot::Cover));
    let user_cover = st.paths.art_dir().join(format!("user/{id}/cover.png"));
    assert!(st.poll_until(Duration::from_secs(5), |s| s.art_state.applying.is_empty()));
    let g = st.library.get(id).unwrap();
    assert_eq!(g.art.cover.as_deref(), Some(user_cover.as_path()), "user art overrides scanner art");
    assert_eq!(g.art.hero.as_deref(), Some(dir.path().join("scan_hero.jpg").as_path()), "other slots untouched");
    // Survives a rescan.
    scanned(&mut st, &[sg], &[Source::Steam]);
    assert_eq!(st.library.get(id).unwrap().art.cover.as_deref(), Some(user_cover.as_path()));
    // A bad file: error toast, art unchanged.
    let bad = dir.path().join("bad.png");
    std::fs::write(&bad, b"nope").unwrap();
    st.set_art_from_file(id, ArtSlot::Logo, &bad);
    assert!(st.poll_until(Duration::from_secs(5), |s| s.art_state.applying.is_empty()));
    assert!(st.toasts.iter().any(|t| t.error && t.text.contains("logo")));
    assert_eq!(st.library.get(id).unwrap().art.logo, None);
    // Replacing with another format deletes the old file.
    let jpg = dir.path().join("mine.jpg");
    let mut out = std::io::Cursor::new(vec![]);
    image::RgbImage::new(4, 4).write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    std::fs::write(&jpg, out.into_inner()).unwrap();
    st.set_art_from_file(id, ArtSlot::Cover, &jpg);
    assert!(st.poll_until(Duration::from_secs(5), |s| s.art_state.applying.is_empty()));
    let user_jpg = st.paths.art_dir().join(format!("user/{id}/cover.jpg"));
    assert_eq!(st.library.get(id).unwrap().art.cover.as_deref(), Some(user_jpg.as_path()));
    assert!(!user_cover.exists());
    // Reset to default: back to the scanner's art, our file removed.
    st.set_art_slot(id, ArtSlot::Cover, None);
    assert_eq!(st.library.get(id).unwrap().art.cover.as_deref(), Some(scan_cover.as_path()));
    assert!(!user_jpg.exists());
    assert_eq!(st.db.user_art(id).unwrap(), Art::default());
    // A hand-placed background crop survives other slots changing, not a new background.
    let pl = crate::model::BackgroundPlacement { x: 0.2, y: 0.8, zoom: 2.0 };
    let mut e = st.library.get(id).unwrap().edits.clone();
    e.background_placement = Some(pl);
    st.set_edits(id, e);
    st.set_art_slot(id, ArtSlot::Logo, Some(jpg.clone()));
    assert_eq!(st.library.get(id).unwrap().edits.background_placement, Some(pl));
    st.set_art_slot(id, ArtSlot::Hero, Some(jpg.clone()));
    assert_eq!(st.library.get(id).unwrap().edits.background_placement, None);
}
