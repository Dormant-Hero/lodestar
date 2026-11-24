//! Small helpers shared by the test modules (compiled for `cargo test` only).

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// `["a", "b"]` as owned strings.
pub(crate) fn sv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// `[("K", "v")]` as owned pairs.
pub(crate) fn ev(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

/// Write `bytes` at `p`, creating its parent folders.
pub(crate) fn write_file(p: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

/// Make the file at `p` executable (0o755).
pub(crate) fn chmod_exe(p: &Path) {
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Write `body` at `p` (parent folders included) and make it executable (0o755).
pub(crate) fn write_exe(p: &Path, body: impl AsRef<[u8]>) {
    write_file(p, body);
    chmod_exe(p);
}
