//! Where a folder really is: symlinks resolved, and folders handed to a Flatpak through the
//! document portal (`/run/user/<uid>/doc/<id>/<name>`, what an emulator's "add game folder"
//! dialog stores) mapped back to the folder on disk. Two roots that are the same folder (or one
//! inside the other) are then seen as such, so their games are not listed twice.

use std::path::{Path, PathBuf};

/// The extended attribute the document portal (xdg-desktop-portal ≥ 1.17) puts on its files with
/// the real path behind them.
const PORTAL_XATTR: &str = "user.document-portal.host-path";

/// Ancestors looked at for the portal attribute (a portal path is `/run/user/N/doc/<id>/<name>/…`
/// or `…/doc/by-app/<app>/<id>/<name>/…`).
const MAX_UP: usize = 12;

/// The real path of `p`: canonical, and a document-portal path replaced by the folder it stands
/// for. Falls back to `p` itself when it can't be resolved (gone, no permission).
pub fn host_path(p: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    portal_target(&canon).unwrap_or(canon)
}

/// `p` (canonical) inside the document portal → the real path; None otherwise.
fn portal_target(p: &Path) -> Option<PathBuf> {
    if !looks_like_portal(p) && !cfg!(test) {
        return None;
    }
    let mut rest: Vec<&std::ffi::OsStr> = vec![];
    let mut cur = p;
    for _ in 0..MAX_UP {
        if let Some(host) = read_xattr(cur, PORTAL_XATTR) {
            let host = PathBuf::from(String::from_utf8_lossy(&host).trim_end_matches('\0').to_string());
            if !host.is_absolute() || host == cur {
                return None;
            }
            let mut out = host;
            for c in rest.iter().rev() {
                out.push(c);
            }
            return Some(std::fs::canonicalize(&out).unwrap_or(out));
        }
        rest.push(cur.file_name()?);
        cur = cur.parent()?;
    }
    None
}

/// `/run/user/<uid>/doc/…` (where the document portal is mounted).
pub fn looks_like_portal(p: &Path) -> bool {
    let mut it = p.components().skip(1);
    matches!(
        (it.next(), it.next(), it.next(), it.next()),
        (Some(a), Some(b), Some(_), Some(d)) if a.as_os_str() == "run" && b.as_os_str() == "user" && d.as_os_str() == "doc"
    )
}

fn read_xattr(p: &Path, name: &str) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let name = std::ffi::CString::new(name).ok()?;
    let mut buf = vec![0u8; 4096];
    // SAFETY: both strings are NUL-terminated and `buf` is valid for `buf.len()` bytes.
    let n = unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(buf)
}

/// Mark `dir` as a document-portal entry for `host` (tests only: a tmpfs folder standing in for
/// `/run/user/N/doc/<id>/<name>`). False when the file system has no user attributes.
#[cfg(test)]
pub fn fake_portal(dir: &Path, host: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let (Ok(path), Ok(name)) =
        (std::ffi::CString::new(dir.as_os_str().as_bytes()), std::ffi::CString::new(PORTAL_XATTR))
    else {
        return false;
    };
    let v = host.as_os_str().as_bytes();
    // SAFETY: NUL-terminated strings, `v` valid for its length.
    unsafe { libc::setxattr(path.as_ptr(), name.as_ptr(), v.as_ptr().cast(), v.len(), 0) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_paths_map_to_the_real_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("mnt/games/ps2");
        let portal = tmp.path().join("run/user/1000/doc/abcd/ps2");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(portal.join("sub")).unwrap();
        let real_c = std::fs::canonicalize(&real).unwrap();
        // Not a portal folder: just canonical.
        assert_eq!(host_path(&real), real_c);
        if !fake_portal(&portal, &real_c) {
            eprintln!("no user xattrs on this file system; skipped");
            return;
        }
        assert_eq!(host_path(&portal), real_c);
        // A folder inside it: the attribute is on an ancestor.
        std::fs::create_dir_all(real.join("sub")).unwrap();
        assert_eq!(host_path(&portal.join("sub")), real_c.join("sub"));
        assert!(looks_like_portal(Path::new("/run/user/1000/doc/abR1/ps2")));
        assert!(!looks_like_portal(Path::new("/mnt/space1/games/ps2")));
    }
}
