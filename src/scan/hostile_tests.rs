//! Hostile / malformed input tests for every scanner: truncated, bit-flipped and spliced launcher
//! files, wrong JSON types, deep nesting, FIFOs and symlink loops. Scanners must return (Ok or
//! Err) without panicking, hanging or allocating without bound.

use super::desktop::test_support::{copy_tree, ctx, fixture};
use super::*;
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

/// Small deterministic PRNG (xorshift64*), so failures are reproducible.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

/// One random mutation of `data`.
fn mutate(rng: &mut Rng, data: &[u8]) -> Vec<u8> {
    let mut d = data.to_vec();
    match rng.below(7) {
        0 => d.truncate(rng.below(d.len() + 1)),
        1 => {
            for _ in 0..1 + rng.below(8) {
                if !d.is_empty() {
                    let i = rng.below(d.len());
                    d[i] = rng.next() as u8;
                }
            }
        }
        2 => {
            // Splice a chunk of the file somewhere else (duplicated keys / unbalanced braces).
            if !d.is_empty() {
                let a = rng.below(d.len());
                let b = (a + rng.below(64)).min(d.len());
                let chunk = d[a..b].to_vec();
                let at = rng.below(d.len());
                d.splice(at..at, chunk);
            }
        }
        3 => {
            // Syntax characters that matter to the formats we parse.
            const SYN: &[u8] = b"{}[]\":,\\\n\t #-'=%\0\xff\xfe";
            for _ in 0..1 + rng.below(6) {
                let at = rng.below(d.len() + 1);
                d.insert(at, SYN[rng.below(SYN.len())]);
            }
        }
        4 => {
            // Deep nesting.
            let at = rng.below(d.len() + 1);
            let unit: &[u8] = match rng.below(4) {
                0 => b"[",
                1 => b"{\"a\":",
                2 => b"\"k\" {",
                _ => b"\n- ",
            };
            d.splice(at..at, unit.repeat(5000));
        }
        5 => {
            // Invalid UTF-8.
            let at = rng.below(d.len() + 1);
            d.splice(at..at, [0xc3, 0x28, 0xa0, 0xa1, 0xe2, 0x28]);
        }
        _ => d.clear(),
    }
    d
}

/// Every non-Steam scanner fixture in one home.
fn hostile_home(home: &Path) {
    let data = home.join(".local/share");
    copy_tree(&fixture("faugus/faugus-launcher"), &data.join("faugus-launcher"));
    copy_tree(&fixture("bottles/bottles"), &data.join("bottles"));
    copy_tree(&fixture("heroic/heroic"), &home.join(".config/heroic"));
    copy_tree(&fixture("desktop/home"), home);
    for (db, schema, rows) in [
        (data.join("lutris/pga.db"), "lutris/schema.sql", "lutris/games.sql"),
        (home.join(".config/itch/db/butler.db"), "itch/schema.sql", "itch/data.sql"),
    ] {
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(&std::fs::read_to_string(fixture(schema)).unwrap()).unwrap();
        conn.execute_batch(&std::fs::read_to_string(fixture(rows)).unwrap()).unwrap();
    }
}

fn files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if e.file_type().unwrap().is_dir() {
            files(&p, out);
        } else if e.file_type().unwrap().is_file() {
            out.push(p);
        }
    }
}

/// Run every scanner directly (not through `run_all`, which would turn a panic into an error)
/// on a thread with the default 2 MiB stack, like the real scan threads.
fn run_scanners(ctx: &ScanContext) {
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            for d in scanners() {
                if d.sources.contains(&Source::Steam) {
                    continue; // covered by the parser fuzz below; needs its own fixture
                }
                let _ = (d.scan)(&ctx);
            }
        })
        .unwrap()
        .join()
        .expect("a scanner panicked");
}

#[test]
fn fuzz_scanner_files() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    hostile_home(home);
    let c = ctx(home);
    run_scanners(&c);
    let mut all = vec![];
    files(home, &mut all);
    let originals: Vec<(std::path::PathBuf, Vec<u8>)> =
        all.into_iter().map(|p| (p.clone(), std::fs::read(&p).unwrap())).collect();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let t0 = Instant::now();
    for _ in 0..300 {
        let (p, orig) = &originals[rng.below(originals.len())];
        std::fs::write(p, mutate(&mut rng, orig)).unwrap();
        let t = Instant::now();
        run_scanners(&c);
        assert!(t.elapsed() < Duration::from_secs(5), "slow scan after mutating {}", p.display());
        std::fs::write(p, orig).unwrap();
    }
    assert!(t0.elapsed() < Duration::from_secs(60));
}

#[test]
fn wrong_json_types_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    for doc in [
        "null",
        "5",
        "\"x\"",
        "{}",
        "{\"games\": 7}",
        "[1, null, \"a\", [], {}]",
        r#"[{"gameid": 5, "title": ["x"], "path": {}, "playtime": -1e308, "last_played": 9}]"#,
        r#"[{"gameid": "g", "title": "T", "playtime": 1e300, "last_played": "9999-99-99T99:99"}]"#,
        r#"[{"gameid": "g", "title": "T", "playtime": "-5", "hidden": {}, "icon": "/dev/null", "path": "/"}]"#,
    ] {
        let _ = faugus::parse_games(doc, dir.path(), false);
    }
}

#[test]
fn steam_parsers_fuzz() {
    use steam::appinfo;
    let manifest = b"\"AppState\"\n{\n\t\"appid\"\t\t\"10\"\n\t\"name\"\t\t\"Game\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"G\"\n\t\"LastPlayed\"\t\t\"1700000000\"\n\t\"SizeOnDisk\"\t\t\"99\"\n}\n";
    let localconfig = b"\"UserLocalConfigStore\" { \"Software\" { \"Valve\" { \"Steam\" { \"apps\" { \"10\" { \"Playtime\" \"5\" \"LastPlayed\" \"-3\" \"LaunchOptions\" \"%command%\" } \"x\" \"y\" \"11\" \"str\" } } } } }";
    let mut shortcuts = Vec::new();
    {
        use vdf::write as bw;
        bw::obj(&mut shortcuts, "shortcuts");
        bw::obj(&mut shortcuts, "0");
        bw::int(&mut shortcuts, "appid", -5);
        bw::str(&mut shortcuts, "AppName", "Emu");
        bw::str(&mut shortcuts, "Exe", "\"/usr/bin/true\"");
        bw::obj(&mut shortcuts, "tags");
        bw::str(&mut shortcuts, "0", "a");
        bw::end(&mut shortcuts);
        bw::end(&mut shortcuts);
        bw::end(&mut shortcuts);
        bw::end(&mut shortcuts);
    }
    let info_file = appinfo::testgen::build(
        vec![
            (10, vec![appinfo::testgen::common("Game", "Game", "linux")]),
            (11, vec![appinfo::testgen::common("Other", "Tool", "")]),
        ],
        true,
    );
    let dir = tempfile::tempdir().unwrap();
    let info_path = dir.path().join("appinfo.vdf");
    let wanted: HashSet<u32> = [10, 11, 12].into_iter().collect();
    let mut rng = Rng(42);
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            for _ in 0..3000 {
                let m = mutate(&mut rng, manifest);
                let _ = steam::parse_manifest(&String::from_utf8_lossy(&m), Path::new("/lib"));
                let l = mutate(&mut rng, localconfig);
                let _ = steam::parse_localconfig(&String::from_utf8_lossy(&l));
                let _ = steam::parse_shortcuts(&mutate(&mut rng, &shortcuts));
            }
            for _ in 0..500 {
                std::fs::write(&info_path, mutate(&mut rng, &info_file)).unwrap();
                let _ = appinfo::read(&info_path, &wanted);
            }
        })
        .unwrap()
        .join()
        .expect("a Steam parser panicked");
}

#[test]
fn text_parsers_fuzz() {
    let seeds: [&[u8]; 4] = [
        b"[Desktop Entry]\nType=Application\nName=G\nExec=env A=1 \"/opt/g g/run\" %U --x=\"a\\\\$b\"\nCategories=Game;\n",
        b"2026-10-31T15:26:40.31+02:00",
        b"Programs:\n  - name: \"x\"\n    path: 'C:\\\\a'\n    folded: a\n      b\n  nested:\n    - - k: v\n",
        b"\"a\" { \"b\" \"c\" [$WIN32] // c\n \"d\" { } }",
    ];
    let mut rng = Rng(7);
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            for i in 0..8000 {
                let seed = seeds[i % seeds.len()];
                let m = mutate(&mut rng, seed);
                let s = String::from_utf8_lossy(&m);
                match i % 4 {
                    0 => {
                        let _ = desktop::DesktopEntry::parse(&s);
                        let _ = desktop::parse_exec(&s);
                    }
                    1 => {
                        let _ = desktop::common::parse_datetime(&s, i % 8 == 1);
                    }
                    2 => {
                        let _ = bottles::yaml::parse(&s);
                    }
                    _ => {
                        let _ = vdf::parse(&s);
                    }
                }
            }
        })
        .unwrap()
        .join()
        .expect("a text parser panicked");
}

#[test]
fn deeply_indented_yaml_does_not_overflow_the_stack() {
    // library.yml with thousands of nested levels (each one space deeper).
    let mut doc = String::new();
    for i in 0..3000 {
        doc.push_str(&" ".repeat(i));
        doc.push_str("k:\n");
    }
    let mut seq = String::new();
    for i in 0..3000 {
        seq.push_str(&" ".repeat(i));
        seq.push_str("-\n");
    }
    // Without the indentation cap this needs ~190 bytes of stack per level (11k levels overflow
    // the scan threads' 2 MiB); a 256 KiB stack makes 3k levels enough to catch a regression.
    std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(move || {
            let _ = bottles::yaml::parse(&doc);
            let _ = bottles::yaml::parse(&seq);
        })
        .unwrap()
        .join()
        .expect("yaml parser overflowed its stack");
}

/// A FIFO where a launcher file should be (or a `.desktop` file) must not block the scan, and a
/// symlink loop in an applications dir must not recurse forever.
#[test]
fn fifos_and_symlink_loops_do_not_hang() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    let apps = home.join(".local/share/applications");
    std::fs::create_dir_all(&apps).unwrap();
    std::os::unix::fs::symlink(&apps, apps.join("loop")).unwrap();
    std::os::unix::fs::symlink(&apps, apps.join("loop2")).unwrap();
    let mkfifo = |p: &Path| {
        let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    };
    mkfifo(&apps.join("fifo.desktop"));
    for d in [".local/share/faugus-launcher", ".local/share/bottles", ".config/heroic/gog_store"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    mkfifo(&home.join(".local/share/faugus-launcher/games.json"));
    mkfifo(&home.join(".local/share/bottles/library.yml"));
    mkfifo(&home.join(".config/heroic/gog_store/installed.json"));
    // Steam root whose localconfig.vdf / libraryfolders.vdf are FIFOs.
    let steam = home.join(".local/share/Steam");
    std::fs::create_dir_all(steam.join("steamapps")).unwrap();
    std::fs::create_dir_all(steam.join("userdata/1/config")).unwrap();
    mkfifo(&steam.join("steamapps/libraryfolders.vdf"));
    mkfifo(&steam.join("userdata/1/config/localconfig.vdf"));
    mkfifo(&steam.join("userdata/1/config/shortcuts.vdf"));

    let (tx, rx) = std::sync::mpsc::channel();
    let c = ctx(&home);
    std::thread::spawn(move || {
        let r = run_all(&c);
        let _ = tx.send(r.per_source.len());
    });
    let n = rx.recv_timeout(Duration::from_secs(10)).expect("scan hung on a FIFO or symlink loop");
    assert_eq!(n, scanners().iter().map(|d| d.sources.len()).sum::<usize>());
    // Unblock any reader that is still stuck, so the test process can exit cleanly.
}
