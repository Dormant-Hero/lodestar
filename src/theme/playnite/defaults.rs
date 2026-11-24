//! Playnite's own default themes. Playnite themes only ship the files they change and inherit
//! every other file from the default theme of their mode, so the importer lays a theme over
//! these (see `tree.rs`): a theme that only restyles one cover still gets Playnite's top bar,
//! bottom bar and details view, and one that only recolours the text still has every other
//! colour.
//!
//! `assets/playnite/default-theme.bin` holds the default Desktop and Fullscreen themes' resource
//! dictionaries, views and derived styles from Playnite's open-source repository (MIT licence,
//! © Josef Nemec, github.com/JosefNemec/Playnite, `source/Playnite.DesktopApp/Themes/Desktop/
//! Default` and `source/Playnite.FullscreenApp/Themes/Fullscreen/Default`), comments removed,
//! raw-deflate compressed (≈ 21 KB for ≈ 240 KB of XAML). Rebuild it with
//! `python3 -I tools/gen-playnite-defaults.py <Playnite checkout>`.

use super::Mode;
use std::sync::OnceLock;

static BLOB: &[u8] = include_bytes!("../../../assets/playnite/default-theme.bin");
const BLOB_LIMIT: usize = 4 * 1024 * 1024;

/// One default-theme file: its path inside the theme (`Views/Main.xaml`) and text.
pub(crate) struct DefaultFile {
    pub mode: Mode,
    pub rel: &'static str,
    pub text: &'static str,
}

fn all() -> &'static [DefaultFile] {
    static FILES: OnceLock<Vec<DefaultFile>> = OnceLock::new();
    FILES.get_or_init(|| {
        let raw = match miniz_oxide::inflate::decompress_to_vec_with_limit(BLOB, BLOB_LIMIT) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("playnite default theme: {e:?}");
                return vec![];
            }
        };
        let Ok(text) = String::from_utf8(raw) else {
            return vec![];
        };
        // Leaked once: the files live as long as the program (a few hundred KB).
        let text: &'static str = Box::leak(text.into_boxed_str());
        unpack(text)
    })
}

fn unpack(text: &'static str) -> Vec<DefaultFile> {
    let mut out = vec![];
    let Some(mut rest) = text.strip_prefix("PNDT1\n") else {
        return out;
    };
    while let Some((head, tail)) = rest.split_once('\n') {
        let Some((path, len)) = head.split_once('\t') else { break };
        let Ok(len) = len.parse::<usize>() else { break };
        if tail.len() < len || !tail.is_char_boundary(len) {
            break;
        }
        let (body, next) = tail.split_at(len);
        let (mode, rel) = match path.split_once('/') {
            Some(("desktop", r)) => (Mode::Desktop, r),
            Some(("fullscreen", r)) => (Mode::Fullscreen, r),
            _ => break,
        };
        out.push(DefaultFile { mode, rel, text: body });
        rest = next;
    }
    out
}

/// The default theme's files for a mode.
pub(crate) fn files(mode: Mode) -> impl Iterator<Item = &'static DefaultFile> {
    all().iter().filter(move |f| f.mode == mode)
}

#[cfg(test)]
mod tests {
    use super::super::xaml::parse;
    use super::*;

    #[test]
    fn defaults_unpack_and_parse() {
        let d: Vec<&str> = files(Mode::Desktop).map(|f| f.rel).collect();
        let f: Vec<&str> = files(Mode::Fullscreen).map(|f| f.rel).collect();
        for want in ["Constants.xaml", "DerivedStyles/GridViewItemTemplate.xaml", "Views/MainWindow.xaml"] {
            assert!(d.contains(&want), "{want} in {d:?}");
        }
        for want in
            ["Constants.xaml", "DerivedStyles/ListGameItemStyle.xaml", "Views/Main.xaml", "Views/GameDetails.xaml"]
        {
            assert!(f.contains(&want), "{want} in {f:?}");
        }
        for file in files(Mode::Desktop).chain(files(Mode::Fullscreen)) {
            assert!(parse(file.text).is_ok(), "{} parses", file.rel);
        }
    }
}
