//! File-system helpers shared by every area that saves files.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Replace `dest` with `bytes` so that no reader (and no crash or full disk) ever sees half a
/// file: write a temporary file next to it, flush it to disk, then rename it over `dest`.
/// Creates the parent folder. On failure the temporary file is removed and `dest` is unchanged.
pub fn write_atomic(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = dest.parent().ok_or_else(|| std::io::Error::other("no parent"))?;
    if !dir.as_os_str().is_empty() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = temp_path(dir, dest);
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, dest)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// `.<name>.<pid>.<n>.tmp` in `dir`: hidden, and unique per process and call, because two threads
/// may write the same file at once (an interactive artwork pick and a bulk run). Cache pruning
/// recognises the `.tmp` ending and removes leftovers.
fn temp_path(dir: &Path, dest: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dir.join(format!(".{name}.{}.{n}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn replaces_the_file_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("sub/config.toml");
        write_atomic(&dest, b"one").unwrap();
        write_atomic(&dest, b"two").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"two");
        assert_eq!(entries(&dir.path().join("sub")), ["config.toml"]);
    }

    #[test]
    fn a_failed_write_keeps_the_old_file_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        // The destination is a non-empty folder: the rename fails.
        let dest = dir.path().join("taken");
        std::fs::create_dir_all(dest.join("inside")).unwrap();
        assert!(write_atomic(&dest, b"x").is_err());
        assert!(dest.join("inside").is_dir());
        assert_eq!(entries(dir.path()), ["taken"]);
    }

    #[test]
    fn concurrent_writers_never_share_a_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("cover.png");
        std::thread::scope(|s| {
            for i in 0..8u8 {
                let dest = &dest;
                s.spawn(move || write_atomic(dest, &[i; 4096]).unwrap());
            }
        });
        let got = std::fs::read(&dest).unwrap();
        assert_eq!(got.len(), 4096);
        assert!(got.iter().all(|&b| b == got[0]), "one writer's bytes, whole");
        assert_eq!(entries(dir.path()), ["cover.png"]);
    }
}
