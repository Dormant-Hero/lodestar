//! Lodestar's own console artwork: for every console Lodestar knows and every built-in
//! collection (All Games, Favourites, stores …) a card, a wide background, a wordmark and the
//! device alone. Original drawings (`tools/gen-console-art.py` → `assets/consoles/`), no makers'
//! logos; themes use them through `{collection.art.card}` … (see theme/model.rs) and may ship
//! their own per collection (`[collection_art.<key>]`).
//!
//! The SVGs are embedded compressed (`assets/consoles/console-art.bin`: "LSART1\n", an index of
//! "<path> <length>" lines, a blank line, one raw-deflate stream of the files in index order).
//! Pictures are loaded by path everywhere (image cache, theme renderer), so the first time one
//! is asked for, the whole set is written once to `<cache>/console-art/<version>/` (a few
//! hundred small files, a few milliseconds); older versions' folders are removed then.

use crate::library::{Collection, CollectionKind};
use crate::model::Game;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static BLOB: &[u8] = include_bytes!("../../assets/consoles/console-art.bin");
const MAGIC: &[u8] = b"LSART1\n";
const MARKER: &str = ".complete";

/// The pictures every console and collection has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArtKind {
    /// 600 × 800 portrait card: illustration, name, maker and year.
    Card,
    /// 1920 × 1080 background: the illustration on the right, no text.
    Hero,
    /// The name as a white wordmark with the maker and year under it, on transparent.
    Mark,
    /// 800 × 600: the illustration alone, on transparent.
    Device,
}

impl ArtKind {
    pub const ALL: [ArtKind; 4] = [ArtKind::Card, ArtKind::Hero, ArtKind::Mark, ArtKind::Device];

    pub fn name(self) -> &'static str {
        match self {
            ArtKind::Card => "card",
            ArtKind::Hero => "hero",
            ArtKind::Mark => "mark",
            ArtKind::Device => "device",
        }
    }

    pub fn parse(s: &str) -> Option<ArtKind> {
        ArtKind::ALL.into_iter().find(|k| k.name() == s)
    }
}

struct Pack {
    /// "card/sony_playstation2.svg" → (offset, length) in the inflated data.
    files: HashMap<&'static str, (usize, usize)>,
    #[cfg(test)]
    order: Vec<&'static str>,
    data: &'static [u8],
    version: u64,
}

fn pack() -> &'static Pack {
    static PACK: OnceLock<Pack> = OnceLock::new();
    PACK.get_or_init(|| {
        parse(BLOB).unwrap_or_else(|| Pack {
            files: HashMap::new(),
            #[cfg(test)]
            order: vec![],
            data: &[],
            version: 0,
        })
    })
}

fn parse(blob: &'static [u8]) -> Option<Pack> {
    let rest = blob.strip_prefix(MAGIC)?;
    let end = rest.windows(2).position(|w| w == b"\n\n")?;
    let index = std::str::from_utf8(&rest[..end]).ok()?;
    let mut files = HashMap::new();
    #[cfg(test)]
    let mut order = vec![];
    let mut off = 0;
    for line in index.lines() {
        let (name, len) = line.rsplit_once(' ')?;
        let len: usize = len.parse().ok()?;
        files.insert(name, (off, len));
        #[cfg(test)]
        order.push(name);
        off += len;
    }
    Some(Pack {
        files,
        #[cfg(test)]
        order,
        data: &rest[end + 2..],
        version: crate::db::fnv1a(blob),
    })
}

fn file_name(key: &str, kind: ArtKind) -> String {
    format!("{}/{key}.svg", kind.name())
}

/// Lodestar has this picture (`key`: a platform id, a store key or a collection's art key).
pub fn has(key: &str, kind: ArtKind) -> bool {
    pack().files.contains_key(file_name(key, kind).as_str())
}

/// Every art key ("sony_playstation2", "steam", "all" …), in the generator's order.
#[cfg(test)]
pub fn keys() -> Vec<&'static str> {
    pack().order.iter().filter_map(|f| f.strip_prefix("card/")?.strip_suffix(".svg")).collect()
}

/// The art key of a collection: its console, store or kind. None for the user's own
/// collections and tags (themes show their names instead).
pub fn key_for(c: &Collection) -> Option<&str> {
    let k = match &c.kind {
        CollectionKind::All => "all",
        CollectionKind::Installed => "installed",
        CollectionKind::Controller => "controller",
        CollectionKind::Pc => "pc",
        CollectionKind::Favorites => "favorites",
        CollectionKind::Recent => "recent",
        CollectionKind::Source(s) => s.key(),
        CollectionKind::Platform(id) => id.as_str(),
        CollectionKind::Tag(_) | CollectionKind::User(_) => return None,
    };
    has(k, ArtKind::Card).then_some(k)
}

/// The art key of a game's console: its emulated platform or the platform set on it
/// ("windows" → PC (Windows)), else its store's.
pub fn key_for_game(g: &Game) -> Option<&'static str> {
    use crate::emulation::platforms;
    let p = g
        .emu
        .as_ref()
        .and_then(|e| platforms::by_id(&e.platform))
        .or_else(|| g.platform.as_deref().and_then(platforms::by_name))
        .map(|p| p.id)
        .filter(|id| has(id, ArtKind::Card));
    p.or_else(|| Some(g.source.key()).filter(|k| has(k, ArtKind::Card)))
}

/// The SVG text of one picture.
#[cfg(test)]
pub fn svg(key: &str, kind: ArtKind) -> Option<String> {
    let all = inflate()?;
    let &(off, len) = pack().files.get(file_name(key, kind).as_str())?;
    String::from_utf8(all.get(off..off + len)?.to_vec()).ok()
}

fn inflate() -> Option<Vec<u8>> {
    let p = pack();
    let total = p.files.values().map(|(o, l)| o + l).max().unwrap_or(0);
    miniz_oxide::inflate::decompress_to_vec_with_limit(p.data, total).ok().filter(|v| v.len() == total)
}

/// `<cache>/console-art` (tests: a folder in the temporary directory).
fn base_dir() -> PathBuf {
    if cfg!(test) {
        std::env::temp_dir().join("lodestar-test-console-art")
    } else {
        crate::config::Paths::detect().cache_dir.join("console-art")
    }
}

/// The folder holding this version's pictures, written the first time it is needed. None when
/// it can't be written (the art is then simply missing, like a theme without it).
fn extracted() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = base_dir();
        let dir = base.join(format!("{:016x}", pack().version));
        if dir.join(MARKER).is_file() {
            return Some(dir);
        }
        match write_all(&dir) {
            Ok(()) => {
                // Older versions' pictures (and their cached thumbnails age out on their own).
                if let Ok(rd) = std::fs::read_dir(&base) {
                    for e in rd.flatten() {
                        let p = e.path();
                        let name = e.file_name();
                        let name = name.to_string_lossy();
                        let old_set = name.len() == 16 && name.chars().all(|c| c.is_ascii_hexdigit());
                        if p != dir && old_set {
                            let _ = std::fs::remove_dir_all(&p);
                        }
                    }
                }
                Some(dir)
            }
            Err(e) => {
                log::warn!("console artwork: can't write {}: {e:#}", dir.display());
                None
            }
        }
    })
    .as_deref()
}

/// Writes the set into a temporary folder, then renames it into place (a cache: no fsync per
/// file, and a half-written set is never seen under the final name).
fn write_all(dir: &Path) -> anyhow::Result<()> {
    use anyhow::Context;
    let data = inflate().context("the embedded console artwork is damaged")?;
    let parent = dir.parent().context("no parent folder")?;
    let tmp = parent.join(format!(".tmp-{}-{:016x}", std::process::id(), pack().version));
    let _ = std::fs::remove_dir_all(&tmp);
    let written = (|| -> anyhow::Result<()> {
        for k in ArtKind::ALL {
            std::fs::create_dir_all(tmp.join(k.name())).with_context(|| format!("creating {}", tmp.display()))?;
        }
        for (name, &(off, len)) in &pack().files {
            std::fs::write(tmp.join(name), &data[off..off + len]).with_context(|| format!("writing {name}"))?;
        }
        std::fs::write(tmp.join(MARKER), b"")?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, dir) {
        let _ = std::fs::remove_dir_all(&tmp);
        // Another Lodestar (or test) got there first.
        if !dir.join(MARKER).is_file() {
            return Err(e).with_context(|| format!("moving the artwork to {}", dir.display()));
        }
    }
    Ok(())
}

/// The file of one of Lodestar's pictures (written on first use); None when there is none.
pub fn path(key: &str, kind: ArtKind) -> Option<PathBuf> {
    if !has(key, kind) {
        return None;
    }
    Some(extracted()?.join(file_name(key, kind)))
}

#[cfg(test)]
mod tests;
