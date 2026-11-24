//! `addon.toml` parsing and validation. Nothing here runs code.

use super::{Manifest, SettingKind};
use std::path::{Component, Path, PathBuf};

pub const MANIFEST_FILE: &str = "addon.toml";
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
pub const MAX_TIMEOUT_SECS: u64 = 600;
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_ID_LEN: usize = 64;

/// Add-on ids: 1–64 of `[a-z0-9-_.]`, not starting with `.` (hidden folders) — the id is also the
/// folder name, so this is what keeps `remove("../x")` and friends harmless.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && !id.starts_with('.')
        && id.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.'))
}

fn valid_setting_key(k: &str) -> bool {
    !k.is_empty() && k.len() <= 64 && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Read `<dir>/addon.toml` (regular file, size-capped) and validate it.
pub fn load(dir: &Path) -> anyhow::Result<Manifest> {
    let path = dir.join(MANIFEST_FILE);
    let meta = std::fs::metadata(&path).map_err(|e| anyhow::anyhow!("{MANIFEST_FILE}: {e}"))?;
    if !meta.is_file() {
        anyhow::bail!("{MANIFEST_FILE} is not a regular file");
    }
    if meta.len() > MAX_MANIFEST_BYTES {
        anyhow::bail!("{MANIFEST_FILE} is too large");
    }
    let text = std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("{MANIFEST_FILE}: {e}"))?;
    parse(&text)
}

/// Parse and validate manifest text (does not look at the file system).
pub fn parse(text: &str) -> anyhow::Result<Manifest> {
    let mut m: Manifest = toml::from_str(text).map_err(|e| anyhow::anyhow!("{MANIFEST_FILE}: {}", e.message()))?;
    m.id = m.id.trim().to_string();
    m.name = m.name.trim().to_string();
    m.exec = m.exec.trim().to_string();
    if m.id.is_empty() {
        anyhow::bail!("{MANIFEST_FILE}: `id` is missing");
    }
    if !valid_id(&m.id) {
        anyhow::bail!("{MANIFEST_FILE}: id \"{}\" may only use a-z, 0-9, '-', '_' and '.' (max {MAX_ID_LEN})", m.id);
    }
    if m.name.is_empty() {
        m.name = m.id.clone();
    }
    if m.exec.is_empty() {
        anyhow::bail!("{MANIFEST_FILE}: `exec` is missing");
    }
    check_exec_path(&m.exec)?;
    if m.timeout_secs == 0 {
        m.timeout_secs = DEFAULT_TIMEOUT_SECS;
    }
    m.timeout_secs = m.timeout_secs.min(MAX_TIMEOUT_SECS);
    let mut seen = std::collections::HashSet::new();
    for f in &mut m.settings {
        f.key = f.key.trim().to_string();
        if !valid_setting_key(&f.key) {
            anyhow::bail!("{MANIFEST_FILE}: setting key \"{}\" may only use letters, digits, '-' and '_'", f.key);
        }
        if !seen.insert(f.key.clone()) {
            anyhow::bail!("{MANIFEST_FILE}: setting \"{}\" is declared twice", f.key);
        }
        if f.label.trim().is_empty() {
            f.label = f.key.clone();
        }
        if f.kind == SettingKind::Toggle && !matches!(f.default.as_str(), "" | "true" | "false") {
            anyhow::bail!("{MANIFEST_FILE}: toggle \"{}\" default must be \"true\" or \"false\"", f.key);
        }
    }
    Ok(m)
}

/// `exec` must be a plain relative path inside the add-on folder.
fn check_exec_path(exec: &str) -> anyhow::Result<()> {
    let p = Path::new(exec);
    if p.is_absolute() {
        anyhow::bail!("{MANIFEST_FILE}: exec must be relative to the add-on folder, not \"{exec}\"");
    }
    for c in p.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                anyhow::bail!("{MANIFEST_FILE}: exec \"{exec}\" must stay inside the add-on folder")
            }
        }
    }
    Ok(())
}

/// The executable as an absolute path, after checking it exists, is a regular executable file and
/// — with symlinks resolved — still lies inside the add-on folder.
pub fn resolve_exec(dir: &Path, exec: &str) -> anyhow::Result<PathBuf> {
    check_exec_path(exec)?;
    let root = std::fs::canonicalize(dir).map_err(|e| anyhow::anyhow!("add-on folder: {e}"))?;
    let path = root.join(exec);
    let real = std::fs::canonicalize(&path).map_err(|_| anyhow::anyhow!("executable \"{exec}\" not found"))?;
    if !real.starts_with(&root) {
        anyhow::bail!("executable \"{exec}\" points outside the add-on folder");
    }
    let meta = std::fs::metadata(&real)?;
    if !meta.is_file() {
        anyhow::bail!("executable \"{exec}\" is not a file");
    }
    use std::os::unix::fs::PermissionsExt;
    if meta.permissions().mode() & 0o111 == 0 {
        anyhow::bail!("\"{exec}\" is not executable (chmod +x {exec})");
    }
    Ok(real)
}
