//! Small helpers shared by the non-Steam scanners (faugus, heroic, lutris, bottles, itch, desktop).
//!
//! Lives next to `desktop.rs` (declared there as `desktop::common`) because `scan/mod.rs` is owned
//! by someone else; every scanner reaches it as `super::desktop::common`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Days from 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse `YYYY-MM-DD[T| ]HH:MM[:SS[.frac]][Z|±HH[:MM]]` (also a bare date) into unix seconds.
/// Without an explicit offset the time is taken as local time when `assume_local`, else UTC.
pub fn parse_datetime(s: &str, assume_local: bool) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = s.get(from..from + len)?;
        if part.bytes().all(|c| c.is_ascii_digit()) { part.parse().ok() } else { None }
    };
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let (mut h, mut mi, mut sec) = (0, 0, 0);
    let mut i = 10;
    if b.len() > 10 && (b[10] == b'T' || b[10] == b' ') {
        h = num(11, 2)?;
        if b.get(13) != Some(&b':') {
            return None;
        }
        mi = num(14, 2)?;
        i = 16;
        if b.get(16) == Some(&b':') {
            sec = num(17, 2)?;
            i = 19;
        }
        if b.get(i) == Some(&b'.') || b.get(i) == Some(&b',') {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    if h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let rest = s[i..].trim();
    let offset: Option<i64> = if rest.is_empty() {
        None
    } else if rest.eq_ignore_ascii_case("z") || rest.eq_ignore_ascii_case("utc") {
        Some(0)
    } else if let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        let digits: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() < 2 {
            return None;
        }
        let oh: i64 = digits[..2].parse().ok()?;
        let om: i64 = digits.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(0);
        let v = oh * 3600 + om * 60;
        Some(if sign == '-' { -v } else { v })
    } else {
        // Trailing junk such as " m=+0.1" (Go monotonic clock text): ignore it, treat as UTC.
        Some(0)
    };
    let days = days_from_civil(y, mo as u32, d as u32);
    let naive = days * 86_400 + h * 3600 + mi * 60 + sec;
    match offset {
        Some(off) => Some(naive - off),
        None if assume_local => Some(local_to_utc(y, mo, d, h, mi, sec).unwrap_or(naive)),
        None => Some(naive),
    }
}

/// Local wall-clock time -> unix seconds via `mktime` (respects the system time zone / DST).
fn local_to_utc(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<i64> {
    // SAFETY: `tm` is plain data; zeroed is a valid value. mktime only reads/writes the struct.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = (y - 1900) as libc::c_int;
    tm.tm_mon = (mo - 1) as libc::c_int;
    tm.tm_mday = d as libc::c_int;
    tm.tm_hour = h as libc::c_int;
    tm.tm_min = mi as libc::c_int;
    tm.tm_sec = s as libc::c_int;
    tm.tm_isdst = -1;
    // SAFETY: valid pointer to an initialised struct.
    let t = unsafe { libc::mktime(&mut tm) };
    if t == -1 { None } else { Some(t as i64) }
}

/// Open an SQLite database strictly read-only, so a running launcher never sees a write lock
/// from us and we never create -wal/-shm files next to its database.
///
/// Normally `immutable=1` (no locking at all). That flag also makes SQLite ignore the `-wal`
/// file, though, so for a database in WAL mode with changes not yet checkpointed into the main
/// file (the launcher is running, or quit uncleanly) recently added games would be invisible:
/// then it is opened read-only *with* the WAL (readers never block a WAL writer), falling back
/// to immutable if that fails (e.g. no `-shm` and a read-only directory).
pub fn open_sqlite_ro(path: &Path) -> rusqlite::Result<rusqlite::Connection> {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let wal_pending = std::fs::metadata(&wal).is_ok_and(|m| m.len() > 0);
    if wal_pending {
        match open_sqlite_ro_with(path, false) {
            Ok(c) => return Ok(c),
            Err(e) => log::debug!("{}: reading with WAL failed ({e}); immutable", path.display()),
        }
    }
    open_sqlite_ro_with(path, true)
}

fn open_sqlite_ro_with(path: &Path, immutable: bool) -> rusqlite::Result<rusqlite::Connection> {
    use rusqlite::OpenFlags;
    let mut uri = String::from("file:");
    for c in path.to_string_lossy().chars() {
        match c {
            '?' => uri.push_str("%3f"),
            '#' => uri.push_str("%23"),
            '%' => uri.push_str("%25"),
            _ => uri.push(c),
        }
    }
    uri.push_str(if immutable { "?mode=ro&immutable=1" } else { "?mode=ro" });
    let conn = rusqlite::Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(200));
    // Opening is lazy; touch the schema so a corrupt/non-SQLite file fails here.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))?;
    Ok(conn)
}

/// Column names of a table (empty if it doesn't exist).
pub fn table_columns(conn: &rusqlite::Connection, table: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let sql = format!("PRAGMA table_info(\"{}\")", table.replace('"', ""));
    if let Ok(mut st) = conn.prepare(&sql)
        && let Ok(rows) = st.query_map([], |r| r.get::<_, String>(1))
    {
        out.extend(rows.flatten());
    }
    out
}

/// `true` if the Flatpak app is installed (system or user installation).
pub fn flatpak_app_installed(home: &Path, app_id: &str) -> bool {
    home.join(".local/share/flatpak/app").join(app_id).exists()
        || Path::new("/var/lib/flatpak/app").join(app_id).exists()
}

/// Lowercase hex SHA-256 (used to find Heroic's image cache files).
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// Decode `%XX` escapes (and `+` as space when `plus_space`).
pub fn percent_decode(s: &str, plus_space: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b'+' if plus_space => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encode everything except RFC 3986 unreserved characters.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~') {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

/// Turn a local art reference (absolute path or `file://` URL) into an existing file.
pub fn local_art(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    let p = if let Some(rest) = s.strip_prefix("file://") {
        PathBuf::from(percent_decode(rest, false))
    } else if s.starts_with('/') {
        PathBuf::from(s)
    } else {
        return None;
    };
    crate::scan::util::file_if_exists(p)
}

/// "62.5 GiB", "700 MB", "1234" -> bytes.
pub fn parse_human_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let n: f64 = s[..split].parse().ok()?;
    let unit = s[split..].trim().to_ascii_lowercase();
    let mul: f64 = match unit.as_str() {
        "" | "b" | "bytes" => 1.0,
        "kb" => 1e3,
        "kib" | "k" => 1024.0,
        "mb" => 1e6,
        "mib" | "m" => 1024.0 * 1024.0,
        "gb" => 1e9,
        "gib" | "g" => 1024.0 * 1024.0 * 1024.0,
        "tb" => 1e12,
        "tib" | "t" => 1024f64.powi(4),
        _ => return None,
    };
    (n >= 0.0).then_some((n * mul) as u64)
}

/// Is this an executable meant for Wine/Proton?
pub fn is_windows_exe(path: &str) -> bool {
    let l = path.trim().to_ascii_lowercase();
    [".exe", ".bat", ".msi", ".lnk", ".com", ".cmd"].iter().any(|e| l.ends_with(e))
}

/// Read width/height from a PNG header (first 24 bytes) without decoding.
pub fn png_dimensions(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 24];
    f.read_exact(&mut buf).ok()?;
    if &buf[..8] != b"\x89PNG\r\n\x1a\n" || &buf[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]);
    let h = u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]);
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_ro_sees_uncheckpointed_wal_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("pga.db");
        // A launcher that is running: WAL mode, rows only in the -wal file.
        let w = rusqlite::Connection::open(&db).unwrap();
        w.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE games (name TEXT);")
            .unwrap();
        w.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        w.execute("INSERT INTO games VALUES ('new game')", []).unwrap();
        let wal = dir.path().join("pga.db-wal");
        assert!(std::fs::metadata(&wal).unwrap().len() > 0);
        let r = open_sqlite_ro(&db).unwrap();
        let n: i64 = r.query_row("SELECT count(*) FROM games", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "row in the WAL must be visible");
        drop(r);
        drop(w); // clean close checkpoints and removes -wal/-shm
        // Plain (non-WAL) database: immutable, no side files created by us.
        let plain = dir.path().join("plain.db");
        let c = rusqlite::Connection::open(&plain).unwrap();
        c.execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);").unwrap();
        drop(c);
        let r = open_sqlite_ro(&plain).unwrap();
        let n: i64 = r.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        drop(r);
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|n| n.starts_with("plain.db-")), "{names:?}");
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(sha256_hex(long), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
    }

    #[test]
    fn datetimes() {
        assert_eq!(parse_datetime("1970-01-01T00:00:00Z", true), Some(0));
        assert_eq!(parse_datetime("2023-05-01T20:00:00.000Z", false), Some(1_682_971_200));
        assert_eq!(parse_datetime("2023-05-01T22:00:00+02:00", false), Some(1_682_971_200));
        assert_eq!(parse_datetime("2023-05-01 20:00:00.409278443+00:00", false), Some(1_682_971_200));
        assert_eq!(parse_datetime("2023-05-01", false), Some(1_682_899_200));
        assert_eq!(parse_datetime("garbage", false), None);
        assert_eq!(parse_datetime("2023-13-01", false), None);
        // Local time: just check it is within a day of the UTC interpretation.
        let local = parse_datetime("2026-10-31T15:26:40.313706", true).unwrap();
        let utc = parse_datetime("2026-10-31T15:26:40Z", true).unwrap();
        assert!((local - utc).abs() <= 14 * 3600);
    }

    #[test]
    fn percent() {
        assert_eq!(percent_decode("a%20b+c", true), "a b c");
        assert_eq!(percent_decode("100%", false), "100%");
        assert_eq!(percent_decode("%zz", false), "%zz");
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_human_size("1.5 GiB"), Some(1_610_612_736));
        assert_eq!(parse_human_size("700 MB"), Some(700_000_000));
        assert_eq!(parse_human_size("1234"), Some(1234));
        assert_eq!(parse_human_size("lots"), None);
    }
}
