use super::*;
use crate::art::net::{Download, HttpResponse};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;

const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/update/release-latest.json");
const DL: &str = "https://github.com/Dormant-Hero/lodestar/releases/download/v0.17.1/Lodestar-0.17.1-x86_64.AppImage";
const CDN: &str = "https://release-assets.githubusercontent.com/github-production-release-asset/1/abc?sig=x";

/// GitHub as the tests want it: the API answer, and per URL a download answer.
#[derive(Default)]
struct FakeNet {
    api: Option<Result<HttpResponse, String>>,
    files: HashMap<String, (u16, Option<String>, Vec<u8>)>,
    asked: Mutex<Vec<String>>,
}

impl FakeNet {
    fn file(mut self, url: &str, status: u16, location: Option<&str>, body: &[u8]) -> FakeNet {
        self.files.insert(url.into(), (status, location.map(str::to_string), body.to_vec()));
        self
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().clone()
    }
}

impl Net for FakeNet {
    fn get(&self, url: &str, _: Option<&str>, _: u64) -> Result<HttpResponse, String> {
        Err(format!("unexpected GET {url}"))
    }
    fn head(&self, url: &str) -> Result<u16, String> {
        Err(format!("unexpected HEAD {url}"))
    }
    fn get_with(&self, url: &str, _: &[(&str, String)], _: u64) -> Result<HttpResponse, String> {
        self.asked.lock().push(url.into());
        self.api.clone().unwrap_or_else(|| Err("offline".into()))
    }
    fn download(
        &self,
        url: &str,
        out: &mut dyn std::io::Write,
        _: u64,
        progress: &mut dyn FnMut(u64),
    ) -> Result<Download, String> {
        self.asked.lock().push(url.into());
        let Some((status, location, body)) = self.files.get(url) else { return Err("offline".into()) };
        if *status == 200 {
            let mut n = 0;
            for c in body.chunks(1000) {
                out.write_all(c).map_err(|e| e.to_string())?;
                n += c.len() as u64;
                progress(n);
            }
        }
        Ok(Download { status: *status, location: location.clone() })
    }
}

fn ok(body: &[u8]) -> Result<HttpResponse, String> {
    Ok(HttpResponse { status: 200, body: body.to_vec(), headers: vec![] })
}

fn status(s: u16) -> Result<HttpResponse, String> {
    Ok(HttpResponse { status: s, body: vec![], headers: vec![] })
}

/// A made-up AppImage: the ELF and AppImage type 2 magic, then `fill`.
fn appimage(fill: u8, len: usize) -> Vec<u8> {
    let mut v = vec![fill; len];
    v[..4].copy_from_slice(b"\x7fELF");
    v[8..11].copy_from_slice(b"AI\x02");
    v
}

fn sha(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn asset_for(body: &[u8], with_digest: bool) -> Asset {
    Asset {
        name: "Lodestar-0.17.1-x86_64.AppImage".into(),
        url: DL.into(),
        size: body.len() as u64,
        sha256: with_digest.then(|| sha(body)),
    }
}

/// A folder with the running AppImage (`Lodestar.AppImage`, old contents, mode 755).
fn installed() -> (tempfile::TempDir, PathBuf, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("Lodestar.AppImage");
    let old = appimage(1, 5000);
    std::fs::write(&target, &old).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, target, old)
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn v(s: &str) -> Version {
    Version::parse(s).unwrap_or_else(|| panic!("{s}"))
}

#[test]
fn versions_compare_by_number_and_a_prerelease_comes_before_its_release() {
    assert!(v("0.17.10") > v("0.17.9"));
    assert!(v("v0.18.0") > v("0.17.1"));
    assert!(v("1.0.0") > v("0.99.99"));
    assert_eq!(v("0.17"), v("0.17.0"));
    assert_eq!(v("V0.17.0+build.5"), v("0.17.0"));
    assert!(v("0.18.0-rc.1") < v("0.18.0"));
    assert!(v("0.18.0-rc.1") > v("0.17.9"));
    assert!(v("0.18.0-rc.10") > v("0.18.0-rc.2"));
    assert!(v("0.18.0-rc.1") > v("0.18.0-1"), "words after numbers");
    assert_eq!(v("0.18.0-rc.1").to_string(), "0.18.0-rc.1");
    for bad in ["", "v", "x.y.z", "1.2.3.4", "1..2", "1.2.3-", "-1.2.3", "1.2.3 beta"] {
        assert!(Version::parse(bad).is_none(), "{bad:?}");
    }
    assert!(current() >= v("0.17.0"), "the running version parses");
}

#[test]
fn release_json_gives_the_version_page_notes_and_the_appimage() {
    let r = parse_release(FIXTURE).unwrap();
    assert_eq!(r.version, "0.17.1");
    assert_eq!(r.tag, "v0.17.1");
    assert_eq!(r.page, "https://github.com/Dormant-Hero/lodestar/releases/tag/v0.17.1");
    assert!(r.notes.starts_with("### Library\n- **Faster start-up**"), "{:?}", r.notes);
    assert!(!r.notes.contains('\r'));
    assert!(r.is_newer_than(&v("0.17.0")));
    assert!(!r.is_newer_than(&v("0.17.1")));
    assert!(!r.is_newer_than(&v("0.18.0")));
    if std::env::consts::ARCH == "x86_64" {
        let a = r.asset.expect("the AppImage, not the .zsync");
        assert_eq!(a.name, "Lodestar-0.17.1-x86_64.AppImage");
        assert_eq!(a.url, DL);
        assert_eq!(a.size, 21734912);
        assert_eq!(a.sha256.as_deref(), Some("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"));
    } else {
        assert!(r.asset.is_none(), "no AppImage for this machine");
    }
}

#[test]
fn drafts_prereleases_and_odd_answers_are_not_releases() {
    let mut j: serde_json::Value = serde_json::from_slice(FIXTURE).unwrap();
    j["prerelease"] = true.into();
    assert!(parse_release(j.to_string().as_bytes()).is_err());
    j["prerelease"] = false.into();
    j["draft"] = true.into();
    assert!(parse_release(j.to_string().as_bytes()).is_err());
    j["draft"] = false.into();
    j["tag_name"] = "nightly".into();
    assert!(parse_release(j.to_string().as_bytes()).is_err());
    assert!(parse_release(b"<html>").is_err());
    // A page link elsewhere is replaced by the releases page; no body is no notes.
    let j = serde_json::json!({"tag_name": "0.18.0", "html_url": "https://evil.example/", "assets": []});
    let r = parse_release(j.to_string().as_bytes()).unwrap();
    assert_eq!(r.page, RELEASES_PAGE);
    assert_eq!(r.notes, "");
    assert!(r.asset.is_none());
}

#[test]
fn the_asset_is_the_exact_appimage_name_fully_uploaded_from_this_repository() {
    let good = || GhAsset {
        name: asset_name("0.17.1"),
        size: 1000,
        browser_download_url: format!(
            "https://github.com/Dormant-Hero/lodestar/releases/download/v0.17.1/{}",
            asset_name("0.17.1")
        ),
        digest: None,
        state: Some("uploaded".into()),
    };
    assert!(pick_asset(&[good()], "0.17.1").is_some());
    assert!(pick_asset(&[good()], "0.17.2").is_none(), "another version's file");
    let zsync = GhAsset { name: format!("{}.zsync", asset_name("0.17.1")), ..good() };
    assert!(pick_asset(&[zsync], "0.17.1").is_none());
    assert!(pick_asset(&[GhAsset { state: Some("new".into()), ..good() }], "0.17.1").is_none(), "still uploading");
    assert!(pick_asset(&[GhAsset { size: 0, ..good() }], "0.17.1").is_none());
    assert!(pick_asset(&[GhAsset { size: 4 << 30, ..good() }], "0.17.1").is_none(), "unbelievable size");
    let elsewhere = GhAsset {
        browser_download_url: format!(
            "https://github.com/someone/lodestar/releases/download/v0.17.1/{}",
            asset_name("0.17.1")
        ),
        ..good()
    };
    assert!(pick_asset(&[elsewhere], "0.17.1").is_none(), "another repository");
}

#[test]
fn downloads_only_come_from_this_repositorys_releases_and_githubs_file_hosts() {
    let n = "Lodestar-0.17.1-x86_64.AppImage";
    assert!(valid_asset_url(DL, n));
    assert!(valid_asset_url(&DL.replace("Dormant-Hero/lodestar", "dormant-hero/Lodestar"), n), "case of the names");
    for bad in [
        DL.replace("https://", "http://"),
        DL.replace("github.com", "github.com:443"),
        DL.replace("github.com", "user@github.com"),
        DL.replace("github.com", "github.com.evil.example"),
        DL.replace("github.com", "evilgithub.com"),
        DL.replace("Dormant-Hero", "Someone-Else"),
        DL.replace("/lodestar/", "/lodestar-fork/"),
        DL.replace("/download/", "/download/x/"),
        DL.replace("v0.17.1", ".."),
        format!("{DL}?x=1"),
        format!("{DL}#x"),
        DL.replace("Lodestar-0.17.1", "Other-0.17.1"),
        "https://github.com/Dormant-Hero/lodestar/archive/refs/tags/v0.17.1.tar.gz".into(),
        String::new(),
    ] {
        assert!(!valid_asset_url(&bad, n), "{bad}");
    }
    assert!(!valid_asset_url(&DL.replace(n, "x/../y"), "x/../y"), "a name that climbs");
    assert!(valid_redirect(CDN));
    assert!(valid_redirect("https://objects.githubusercontent.com/github-production-release-asset-2e65be/1/2?x=y"));
    for bad in [
        CDN.replace("https://", "http://"),
        CDN.replace("release-assets.githubusercontent.com", "release-assets.githubusercontent.com.evil.example"),
        CDN.replace("release-assets.githubusercontent.com", "evil@release-assets.githubusercontent.com"),
        CDN.replace("release-assets.githubusercontent.com", "raw.githubusercontent.com"),
        "https://evil.example/Lodestar-0.17.1-x86_64.AppImage".into(),
        "/relative/path".into(),
    ] {
        assert!(!valid_redirect(&bad), "{bad}");
    }
}

#[test]
fn githubs_digest_is_read_only_as_a_sha256() {
    let hex = "9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08";
    assert_eq!(parse_digest(Some(&format!("sha256:{hex}"))), Some(hex.to_ascii_lowercase()));
    assert_eq!(parse_digest(Some(&format!("sha512:{hex}"))), None);
    assert_eq!(parse_digest(Some("sha256:abc")), None);
    assert_eq!(parse_digest(Some(&format!("sha256:{}", "z".repeat(64)))), None);
    assert_eq!(parse_digest(None), None);
}

#[test]
fn a_check_asks_the_releases_api_once_and_stays_quiet_about_failures() {
    let net = FakeNet { api: Some(ok(FIXTURE)), ..Default::default() };
    assert_eq!(fetch_latest(&net).unwrap().version, "0.17.1");
    assert_eq!(net.asked(), [LATEST_API]);
    for (answer, want) in [
        (status(404), CheckErr::NoRelease),
        (status(403), CheckErr::RateLimited),
        (status(429), CheckErr::RateLimited),
        (status(503), CheckErr::Offline("HTTP 503".into())),
        (Err("dns".to_string()), CheckErr::Offline("dns".into())),
    ] {
        let net = FakeNet { api: Some(answer), ..Default::default() };
        assert_eq!(fetch_latest(&net).unwrap_err(), want);
    }
    let net = FakeNet { api: Some(ok(b"{}")), ..Default::default() };
    assert!(matches!(fetch_latest(&net), Err(CheckErr::Bad(_))));
}

#[test]
fn automatic_checks_run_at_most_once_a_day() {
    let now = 1_800_000_000;
    assert!(check_due(None, now), "never checked");
    assert!(!check_due(Some(now - 60), now));
    assert!(!check_due(Some(now - CHECK_EVERY_SECS + 1), now));
    assert!(check_due(Some(now - CHECK_EVERY_SECS), now));
    assert!(check_due(Some(now + 2 * 3600), now), "the clock went back");
}

#[test]
fn no_automatic_check_when_the_setting_is_off_or_the_run_stays_offline() {
    let now = 1_800_000_000;
    assert!(auto_check_wanted(true, false, None, now));
    assert!(!auto_check_wanted(false, false, None, now), "Check for updates is off");
    assert!(!auto_check_wanted(true, true, None, now), "screenshot runs never go online");
    assert!(!auto_check_wanted(true, false, Some(now - 3600), now), "checked an hour ago");
}

#[test]
fn the_last_check_is_remembered_in_the_cache_folder() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("update-check.json");
    assert_eq!(CheckRecord::load(&p), CheckRecord::default());
    let rec = CheckRecord { checked_at: Some(5), latest: Some(parse_release(FIXTURE).unwrap()) };
    rec.save(&p);
    assert_eq!(CheckRecord::load(&p), rec);
    std::fs::write(&p, "garbage").unwrap();
    assert_eq!(CheckRecord::load(&p), CheckRecord::default());
}

#[test]
fn an_update_follows_githubs_redirect_checks_it_and_replaces_the_appimage() {
    let (dir, target, old) = installed();
    let new = appimage(2, 12_345);
    let net = FakeNet::default().file(DL, 302, Some(CDN), b"").file(CDN, 200, None, &new);
    let mut seen = vec![];
    download_and_replace(&net, &asset_for(&new, true), &target, &AtomicBool::new(false), &mut |g, t| seen.push((g, t)))
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), new);
    assert_eq!(std::fs::read(old_path(&target)).unwrap(), old, "the old version is kept until the next start");
    let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755);
    assert_eq!(seen.last(), Some(&(12_345, 12_345)));
    assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
    assert_eq!(net.asked(), [DL, CDN]);
    assert_eq!(names(dir.path()), ["Lodestar.AppImage", "Lodestar.AppImage.old"]);
    // The next start: the old version and stray downloads go, the AppImage stays.
    std::fs::write(dir.path().join(".Lodestar.AppImage.update-77.tmp"), b"x").unwrap();
    std::fs::write(dir.path().join(".Other.AppImage.update-77.tmp"), b"x").unwrap();
    assert_eq!(clean_leftovers(&target), 2);
    assert_eq!(names(dir.path()), [".Other.AppImage.update-77.tmp", "Lodestar.AppImage"]);
}

#[test]
fn a_private_appimage_stays_private_but_executable() {
    let (_dir, target, _) = installed();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    let new = appimage(2, 3000);
    let net = FakeNet::default().file(DL, 200, None, &new);
    download_and_replace(&net, &asset_for(&new, true), &target, &AtomicBool::new(false), &mut |_, _| {}).unwrap();
    assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o700);
}

/// Every refused download leaves the AppImage as it was and no file behind.
fn assert_untouched(dir: &Path, target: &Path, old: &[u8]) {
    assert_eq!(std::fs::read(target).unwrap(), old);
    assert_eq!(names(dir), ["Lodestar.AppImage"]);
}

#[test]
fn a_download_with_the_wrong_checksum_is_thrown_away() {
    let (dir, target, old) = installed();
    let new = appimage(2, 3000);
    let mut asset = asset_for(&new, true);
    asset.sha256 = Some(sha(b"something else"));
    let net = FakeNet::default().file(DL, 200, None, &new);
    let e = download_and_replace(&net, &asset, &target, &AtomicBool::new(false), &mut |_, _| {}).unwrap_err();
    assert!(e.contains("checksum"), "{e}");
    assert_untouched(dir.path(), &target, &old);
}

#[test]
fn without_a_digest_the_size_and_the_appimage_magic_must_match() {
    let (dir, target, old) = installed();
    let new = appimage(2, 3000);
    // Good without a digest.
    let net = FakeNet::default().file(DL, 200, None, &new);
    download_and_replace(&net, &asset_for(&new, false), &target, &AtomicBool::new(false), &mut |_, _| {}).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), new);
    std::fs::write(&target, &old).unwrap();
    clean_leftovers(&target);
    // Not an AppImage.
    let html = vec![b'<'; 3000];
    let net = FakeNet::default().file(DL, 200, None, &html);
    let e = download_and_replace(&net, &asset_for(&html, false), &target, &AtomicBool::new(false), &mut |_, _| {})
        .unwrap_err();
    assert!(e.contains("isn't an AppImage"), "{e}");
    assert_untouched(dir.path(), &target, &old);
    // Shorter than the release says.
    let mut asset = asset_for(&new, false);
    asset.size += 10;
    let net = FakeNet::default().file(DL, 200, None, &new);
    let e = download_and_replace(&net, &asset, &target, &AtomicBool::new(false), &mut |_, _| {}).unwrap_err();
    assert!(e.contains("incomplete"), "{e}");
    assert_untouched(dir.path(), &target, &old);
    // Longer than the release says.
    let mut asset = asset_for(&new, false);
    asset.size -= 10;
    let e = download_and_replace(&net, &asset, &target, &AtomicBool::new(false), &mut |_, _| {}).unwrap_err();
    assert!(e.contains("stopped"), "{e}");
    assert_untouched(dir.path(), &target, &old);
}

#[test]
fn a_redirect_to_another_host_is_never_followed() {
    let (dir, target, old) = installed();
    let new = appimage(2, 3000);
    let evil = "https://evil.example/Lodestar-0.17.1-x86_64.AppImage";
    let net = FakeNet::default().file(DL, 302, Some(evil), b"").file(evil, 200, None, &new);
    let e = download_and_replace(&net, &asset_for(&new, true), &target, &AtomicBool::new(false), &mut |_, _| {})
        .unwrap_err();
    assert!(e.contains("doesn't trust"), "{e}");
    assert_eq!(net.asked(), [DL], "the other host is never asked");
    assert_untouched(dir.path(), &target, &old);
    // An asset address of another repository isn't even asked.
    let mut asset = asset_for(&new, true);
    asset.url = DL.replace("Dormant-Hero", "someone");
    let net = FakeNet::default().file(&asset.url, 200, None, &new);
    assert!(download_and_replace(&net, &asset, &target, &AtomicBool::new(false), &mut |_, _| {}).is_err());
    assert!(net.asked().is_empty());
    assert_untouched(dir.path(), &target, &old);
}

#[test]
fn errors_and_cancelling_leave_the_appimage_alone() {
    let (dir, target, old) = installed();
    let new = appimage(2, 3000);
    let net = FakeNet::default().file(DL, 404, None, b"");
    let e = download_and_replace(&net, &asset_for(&new, true), &target, &AtomicBool::new(false), &mut |_, _| {})
        .unwrap_err();
    assert!(e.contains("HTTP 404"), "{e}");
    assert_untouched(dir.path(), &target, &old);
    let net = FakeNet::default().file(DL, 200, None, &new);
    let cancel = AtomicBool::new(true);
    let e = download_and_replace(&net, &asset_for(&new, true), &target, &cancel, &mut |_, _| {}).unwrap_err();
    assert_eq!(e, "Cancelled.");
    assert_untouched(dir.path(), &target, &old);
}

#[test]
fn only_the_appimage_that_is_running_counts_as_one() {
    let (dir, target, _) = installed();
    let mount = PathBuf::from("/tmp/.mount_LodestXYZ");
    let exe = mount.join("usr/bin/lodestar");
    let env = |appimage: Option<&Path>, appdir: Option<&Path>| {
        let (a, d) = (appimage.map(|p| p.as_os_str().to_owned()), appdir.map(|p| p.as_os_str().to_owned()));
        move |k: &str| match k {
            "APPIMAGE" => a.clone(),
            "APPDIR" => d.clone(),
            _ => None,
        }
    };
    let canon = std::fs::canonicalize(&target).unwrap();
    assert_eq!(
        Install::detect(&env(Some(&target), Some(&mount)), Some(&exe)),
        Install::AppImage { path: canon.clone(), writable: true }
    );
    assert!(Install::detect(&env(Some(&target), Some(&mount)), Some(&exe)).can_update());
    // Not started from an AppImage (install.sh, cargo build).
    assert_eq!(Install::detect(&env(None, None), Some(Path::new("/home/u/.local/bin/lodestar"))), Install::Other);
    // Variables inherited from another AppImage that started this Lodestar.
    assert_eq!(
        Install::detect(&env(Some(&target), Some(&mount)), Some(Path::new("/home/u/.local/bin/lodestar"))),
        Install::Other
    );
    // APPIMAGE pointing at something that isn't an AppImage, or relative.
    let plain = dir.path().join("notes.txt");
    std::fs::write(&plain, "hello there, not an AppImage").unwrap();
    assert_eq!(Install::detect(&env(Some(&plain), Some(&mount)), Some(&exe)), Install::Other);
    assert_eq!(Install::detect(&env(Some(Path::new("Lodestar.AppImage")), Some(&mount)), Some(&exe)), Install::Other);
    assert_eq!(Install::detect(&env(Some(&target), Some(Path::new("/"))), Some(&exe)), Install::Other);
}

#[test]
fn an_appimage_in_a_read_only_folder_is_found_but_not_updated() {
    if unsafe { libc::geteuid() } == 0 {
        return; // root may write anywhere
    }
    let (dir, target, _) = installed();
    let mount = PathBuf::from("/tmp/.mount_LodestXYZ");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let env = |k: &str| match k {
        "APPIMAGE" => Some(target.as_os_str().to_owned()),
        "APPDIR" => Some(mount.as_os_str().to_owned()),
        _ => None,
    };
    let got = Install::detect(&env, Some(&mount.join("usr/bin/lodestar")));
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(got, Install::AppImage { writable: false, .. }), "{got:?}");
    assert!(!got.can_update());
}
