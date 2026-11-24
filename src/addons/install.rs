//! Installing an add-on from a folder or a .zip: validate, copy into `<addons>/<id>/`, never run it.

use super::Manifest;
use super::manifest::{self, MANIFEST_FILE};
use std::path::{Path, PathBuf};

const MAX_FILES: usize = 20_000;
const MAX_TOTAL_BYTES: u64 = 1 << 30;
const MAX_DEPTH: usize = 32;

/// Install `src` (an add-on folder, a folder holding exactly one add-on folder, or a .zip of
/// either) into `addons_dir`. Returns the manifest of what was installed.
pub fn install(addons_dir: &Path, src: &Path) -> anyhow::Result<Manifest> {
    std::fs::create_dir_all(addons_dir).map_err(|e| anyhow::anyhow!("{}: {e}", addons_dir.display()))?;
    let meta = std::fs::metadata(src).map_err(|e| anyhow::anyhow!("{}: {e}", src.display()))?;
    if meta.is_dir() {
        return install_dir(addons_dir, src, false);
    }
    if meta.is_file() && is_zip(src) {
        let tmp = TempDir::new(addons_dir, "unzip")?;
        unzip(src, &tmp.0)?;
        return install_dir(addons_dir, &tmp.0, true);
    }
    anyhow::bail!("choose an add-on folder (with {MANIFEST_FILE}) or a .zip of one")
}

fn is_zip(p: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    let ok = std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut magic)).is_ok();
    ok && magic == *b"PK\x03\x04"
}

/// The folder that holds `addon.toml`: `dir` itself or its only sub-folder (zips usually wrap
/// their content in one top folder).
fn find_root(dir: &Path) -> anyhow::Result<PathBuf> {
    if dir.join(MANIFEST_FILE).is_file() {
        return Ok(dir.to_path_buf());
    }
    let subdirs: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.') && e.file_name() != "__MACOSX")
        .map(|e| e.path())
        .collect();
    match subdirs.as_slice() {
        [one] if one.join(MANIFEST_FILE).is_file() => Ok(one.clone()),
        _ => anyhow::bail!("no {MANIFEST_FILE} found in {}", dir.display()),
    }
}

fn install_dir(addons_dir: &Path, src: &Path, unpacked: bool) -> anyhow::Result<Manifest> {
    let root = std::fs::canonicalize(find_root(src)?)?;
    let m = manifest::load(&root)?;
    manifest::resolve_exec(&root, &m.exec)?;
    let target = addons_dir.join(&m.id);
    if let Ok(t) = std::fs::canonicalize(&target)
        && t == root
    {
        anyhow::bail!("\"{}\" is already installed from this folder", m.name);
    }
    if !unpacked && root.starts_with(std::fs::canonicalize(addons_dir)?) {
        anyhow::bail!("choose an add-on folder outside the add-ons folder");
    }
    // Same id already there: only an update of the same add-on (same name and author) replaces it.
    if std::fs::symlink_metadata(&target).is_ok() {
        match manifest::load(&target) {
            Ok(old) if same_addon(&old, &m) => {}
            Ok(old) => anyhow::bail!(
                "another add-on with the id \"{}\" is installed ({}{}); remove it first",
                m.id,
                old.name,
                if old.author.is_empty() { String::new() } else { format!(" by {}", old.author) }
            ),
            Err(_) => {
                anyhow::bail!("the folder for \"{}\" already exists but isn't a valid add-on; remove it first", m.id)
            }
        }
    }

    // Copy into a staging folder, then swap it in (an interrupted install never leaves half an add-on).
    let staging = TempDir::new(addons_dir, &format!("staging-{}", m.id))?;
    let mut budget = Budget { files: 0, bytes: 0 };
    copy_tree(&root, &root, &staging.0, &mut budget, 0)?;
    // Check again on the copy: a symlink that was resolved, a file that changed …
    let copied = manifest::load(&staging.0)?;
    manifest::resolve_exec(&staging.0, &copied.exec)?;
    if copied.id != m.id {
        anyhow::bail!("{MANIFEST_FILE} changed during install");
    }
    if std::fs::symlink_metadata(&target).is_ok() {
        let old = TempDir::new(addons_dir, &format!("old-{}", m.id))?;
        let old_path = old.0.join("addon");
        std::fs::rename(&target, &old_path).map_err(|e| anyhow::anyhow!("replacing the old version: {e}"))?;
        if let Err(e) = std::fs::rename(&staging.0, &target) {
            let _ = std::fs::rename(&old_path, &target);
            anyhow::bail!("installing: {e}");
        }
        // `old` is removed on drop.
    } else {
        std::fs::rename(&staging.0, &target).map_err(|e| anyhow::anyhow!("installing: {e}"))?;
    }
    Ok(copied)
}

fn same_addon(a: &Manifest, b: &Manifest) -> bool {
    let norm = |s: &str| s.trim().to_lowercase();
    a.id == b.id && norm(&a.author) == norm(&b.author) && norm(&a.name) == norm(&b.name)
}

struct Budget {
    files: usize,
    bytes: u64,
}

/// Copy `dir` (inside `root`) to `dest`. Regular files keep their permission bits (the executable
/// bit matters). Symlinks are copied as the file they point to, but only when that lies inside
/// `root`; links to directories, outside targets, devices and FIFOs refuse the install.
fn copy_tree(root: &Path, dir: &Path, dest: &Path, budget: &mut Budget, depth: usize) -> anyhow::Result<()> {
    if depth > MAX_DEPTH {
        anyhow::bail!("add-on folder is nested too deeply");
    }
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" || name == "__MACOSX" {
            continue;
        }
        let path = entry.path();
        let to = dest.join(&name);
        let ft = entry.file_type()?;
        let file = if ft.is_symlink() {
            let real = std::fs::canonicalize(&path).map_err(|_| anyhow::anyhow!("broken link {}", rel(root, &path)))?;
            if !real.starts_with(root) {
                anyhow::bail!("{} links outside the add-on folder", rel(root, &path));
            }
            if !std::fs::metadata(&real)?.is_file() {
                anyhow::bail!("{} links to a folder (not supported)", rel(root, &path));
            }
            real
        } else if ft.is_dir() {
            copy_tree(root, &path, &to, budget, depth + 1)?;
            continue;
        } else if ft.is_file() {
            path.clone()
        } else {
            anyhow::bail!("{} is not a regular file", rel(root, &path));
        };
        budget.files += 1;
        budget.bytes += std::fs::metadata(&file)?.len();
        if budget.files > MAX_FILES || budget.bytes > MAX_TOTAL_BYTES {
            anyhow::bail!("add-on is too large");
        }
        // std::fs::copy keeps the permission bits.
        std::fs::copy(&file, &to).map_err(|e| anyhow::anyhow!("copying {}: {e}", rel(root, &path)))?;
    }
    Ok(())
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

/// Unpack a zip with the system `unzip` (no zip crate; Info-ZIP's unzip refuses `../` paths).
fn unzip(zip: &Path, dest: &Path) -> anyhow::Result<()> {
    let Some(unzip) = which("unzip") else {
        anyhow::bail!(
            "installing a .zip needs the `unzip` program, which isn't installed — unpack it and choose the folder instead"
        );
    };
    // Look at the listing first: refuse zip bombs and absolute / parent paths before writing anything.
    let out = std::process::Command::new(&unzip)
        .arg("-Z1")
        .arg(zip)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| anyhow::anyhow!("unzip: {e}"))?;
    if !out.status.success() {
        anyhow::bail!("not a readable .zip file");
    }
    let names = String::from_utf8_lossy(&out.stdout);
    let count = names.lines().count();
    if count > MAX_FILES {
        anyhow::bail!("add-on is too large");
    }
    for n in names.lines() {
        if n.starts_with('/') || n.split(['/', '\\']).any(|c| c == "..") {
            anyhow::bail!("the .zip contains an unsafe path ({n})");
        }
    }
    let list = std::process::Command::new(&unzip)
        .arg("-l")
        .arg(zip)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| anyhow::anyhow!("unzip: {e}"))?;
    let total: u64 = String::from_utf8_lossy(&list.stdout)
        .lines()
        .rev()
        .find_map(|l| l.split_whitespace().next()?.parse().ok())
        .unwrap_or(0);
    if total > MAX_TOTAL_BYTES {
        anyhow::bail!("add-on is too large");
    }
    let st = std::process::Command::new(&unzip)
        .args(["-q", "-o"])
        .arg(zip)
        .arg("-d")
        .arg(dest)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| anyhow::anyhow!("unzip: {e}"))?;
    if !st.status.success() {
        let msg = String::from_utf8_lossy(&st.stderr);
        anyhow::bail!("unpacking failed: {}", super::run::clip(msg.trim(), 200));
    }
    Ok(())
}

pub(crate) fn which(prog: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(prog)).find(|p| p.is_file())
}

/// A hidden work folder inside the add-ons folder (same file system, so `rename` works), removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(parent: &Path, tag: &str) -> anyhow::Result<TempDir> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = parent.join(format!(".{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p)?;
        Ok(TempDir(p))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.0).is_ok() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
