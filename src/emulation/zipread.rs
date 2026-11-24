//! A small read-only .zip reader for BIOS / keys / firmware archives (stored + deflate, via
//! miniz_oxide). Only what BIOS handling needs: list the entries from the central directory and
//! read one entry (≤ a size limit). No zip64, no encryption, no writing.
//!
//! Entries of a zip are addressed as "virtual paths": `<zip path>/<entry name>` (e.g.
//! `~/Downloads/ps2-bios-usa.zip/ps2 bios usa/SCPH-39001.bin`). [`split`] tells such a path
//! apart from a real one.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One file inside a zip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Name inside the archive ("dir/file.bin", `/`-separated).
    pub name: String,
    pub size: u64,
    compressed: u64,
    method: u16,
    local_offset: u64,
}

impl Entry {
    /// The file name part ("dir/file.bin" → "file.bin").
    pub fn base_name(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }
}

/// Junk that zips made on macOS carry, and hidden files.
pub fn junk(name: &str) -> bool {
    name.split('/').any(|c| c == "__MACOSX" || c.starts_with('.'))
}

const MAX_ENTRIES: usize = 20_000;

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The files of a zip (folders and macOS / hidden junk left out). None: not a readable zip.
pub fn list_uncached(zip: &Path) -> Option<Vec<Entry>> {
    let mut f = std::fs::File::open(zip).ok()?;
    let len = f.metadata().ok()?.len();
    if len < 22 {
        return None;
    }
    // End of central directory: in the last 64 KiB + 22 bytes.
    let tail_len = len.min(65_557);
    f.seek(SeekFrom::Start(len - tail_len)).ok()?;
    let mut tail = vec![0u8; tail_len as usize];
    f.read_exact(&mut tail).ok()?;
    let eocd = (0..=tail.len() - 22).rev().find(|&i| tail[i..i + 4] == *b"PK\x05\x06")?;
    let count = u16le(&tail, eocd + 10) as usize;
    let cd_size = u32le(&tail, eocd + 12) as u64;
    let cd_off = u32le(&tail, eocd + 16) as u64;
    if cd_off.checked_add(cd_size)? > len || cd_size > 64 * 1024 * 1024 {
        return None;
    }
    f.seek(SeekFrom::Start(cd_off)).ok()?;
    let mut cd = vec![0u8; cd_size as usize];
    f.read_exact(&mut cd).ok()?;
    let mut out = vec![];
    let mut i = 0usize;
    for _ in 0..count.min(MAX_ENTRIES) {
        if i + 46 > cd.len() || cd[i..i + 4] != *b"PK\x01\x02" {
            break;
        }
        let method = u16le(&cd, i + 10);
        let compressed = u32le(&cd, i + 20) as u64;
        let size = u32le(&cd, i + 24) as u64;
        let n = u16le(&cd, i + 28) as usize;
        let m = u16le(&cd, i + 30) as usize;
        let k = u16le(&cd, i + 32) as usize;
        let local_offset = u32le(&cd, i + 42) as u64;
        let name_bytes = cd.get(i + 46..i + 46 + n)?;
        let name = String::from_utf8_lossy(name_bytes).replace('\\', "/");
        i += 46 + n + m + k;
        if name.ends_with('/') || name.is_empty() || junk(&name) {
            continue;
        }
        if name.starts_with('/') || name.split('/').any(|c| c == "..") {
            continue;
        }
        out.push(Entry { name, size, compressed, method, local_offset });
    }
    Some(out)
}

type ListCache = HashMap<PathBuf, (u64, Option<SystemTime>, Option<Vec<Entry>>)>;
static LISTS: Mutex<Option<ListCache>> = Mutex::new(None);

/// [`list_uncached`], cached by path + size + modification time.
pub fn list(zip: &Path) -> Option<Vec<Entry>> {
    let m = std::fs::metadata(zip).ok()?;
    if !m.is_file() {
        return None;
    }
    let mtime = m.modified().ok();
    if let Some((l, t, v)) = LISTS.lock().as_ref().and_then(|c| c.get(zip))
        && *l == m.len()
        && *t == mtime
    {
        return v.clone();
    }
    let v = list_uncached(zip);
    LISTS.lock().get_or_insert_with(HashMap::new).insert(zip.to_path_buf(), (m.len(), mtime, v.clone()));
    v
}

/// The bytes of one entry (None: unreadable, an unknown method, or bigger than `limit`).
pub fn read(zip: &Path, e: &Entry, limit: u64) -> Option<Vec<u8>> {
    if e.size > limit || e.compressed > limit.saturating_mul(2).max(1 << 20) {
        return None;
    }
    let mut f = std::fs::File::open(zip).ok()?;
    f.seek(SeekFrom::Start(e.local_offset)).ok()?;
    let mut head = [0u8; 30];
    f.read_exact(&mut head).ok()?;
    if head[..4] != *b"PK\x03\x04" {
        return None;
    }
    let skip = u16le(&head, 26) as i64 + u16le(&head, 28) as i64;
    f.seek(SeekFrom::Current(skip)).ok()?;
    let mut raw = vec![0u8; e.compressed as usize];
    f.read_exact(&mut raw).ok()?;
    let data = match e.method {
        0 => raw,
        8 => miniz_oxide::inflate::decompress_to_vec_with_limit(&raw, e.size as usize).ok()?,
        _ => return None,
    };
    (data.len() as u64 == e.size).then_some(data)
}

/// Is `p` a file inside a zip (`<zip>/<entry>`)? → (the zip, the entry name).
pub fn split(p: &Path) -> Option<(PathBuf, String)> {
    let s = p.to_string_lossy();
    if !s.to_ascii_lowercase().contains(".zip/") {
        return None;
    }
    let mut zip = p.to_path_buf();
    while zip.pop() {
        if zip.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) && zip.is_file() {
            let entry = p.strip_prefix(&zip).ok()?.to_string_lossy().into_owned();
            return Some((zip, entry));
        }
    }
    None
}

/// [`split`] by the name alone (no disk access, for drawing): the first part ending in ".zip"
/// that has more path after it.
pub fn split_by_name(p: &Path) -> Option<(PathBuf, String)> {
    let mut zip = PathBuf::new();
    let mut comps = p.components();
    for c in comps.by_ref() {
        zip.push(c);
        if zip.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
            let rest = comps.as_path().to_string_lossy().into_owned();
            return (!rest.is_empty()).then_some((zip, rest));
        }
    }
    None
}

/// The entry a virtual path names.
pub fn entry_of(p: &Path) -> Option<(PathBuf, Entry)> {
    let (zip, name) = split(p)?;
    let e = list(&zip)?.into_iter().find(|e| e.name == name)?;
    Some((zip, e))
}

/// The real file behind a path: the zip for a file inside one, else the path itself.
pub fn real_path(p: &Path) -> PathBuf {
    split(p).map(|(z, _)| z).unwrap_or_else(|| p.to_path_buf())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Write a zip: (name, data, deflate?).
    pub(crate) fn write_zip(path: &Path, entries: &[(&str, &[u8], bool)]) {
        let mut out: Vec<u8> = vec![];
        let mut central: Vec<u8> = vec![];
        for (name, data, deflate) in entries {
            let crc = crc32fast::hash(data);
            let body = if *deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
            let method: u16 = if *deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0u8; 4]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&3u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b"xyz"); // an extra field to skip
            out.extend_from_slice(&body);
            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0u8; 4]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0u8; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn lists_and_reads_stored_and_deflated_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let z = tmp.path().join("bios pack.zip");
        let big = vec![7u8; 300_000];
        write_zip(
            &z,
            &[
                ("pack/", b"", false),
                ("pack/.DS_Store", b"junk", true),
                ("__MACOSX/pack/._a.bin", b"junk", true),
                ("pack/a.bin", b"hello stored", false),
                ("pack/sub/b.bin", &big, true),
                ("../evil.bin", b"no", false),
            ],
        );
        let l = list(&z).unwrap();
        let names: Vec<&str> = l.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["pack/a.bin", "pack/sub/b.bin"]);
        assert_eq!(l[1].base_name(), "b.bin");
        assert_eq!(read(&z, &l[0], 1 << 20).unwrap(), b"hello stored");
        assert_eq!(read(&z, &l[1], 1 << 20).unwrap(), big);
        assert_eq!(read(&z, &l[1], 1000), None, "over the limit");
        // Virtual paths.
        let v = z.join("pack/sub/b.bin");
        assert_eq!(split(&v), Some((z.clone(), "pack/sub/b.bin".to_string())));
        assert_eq!(entry_of(&v).unwrap().1.size, 300_000);
        assert_eq!(real_path(&v), z);
        assert_eq!(split_by_name(&v), Some((z.clone(), "pack/sub/b.bin".to_string())));
        assert_eq!(split_by_name(&z), None, "the zip itself");
        assert_eq!(split(&tmp.path().join("plain.bin")), None);
        assert!(list(&tmp.path().join("missing.zip")).is_none());
        std::fs::write(tmp.path().join("bad.zip"), b"PK not really").unwrap();
        assert!(list(&tmp.path().join("bad.zip")).is_none());
    }
}
