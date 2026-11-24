//! The theme being edited: its file, where it lives, undo / redo, and what is saved.

use crate::theme::ThemeFile;
use std::path::PathBuf;

/// Undo steps kept.
const MAX_UNDO: usize = 300;

#[derive(Clone)]
struct Step {
    file: ThemeFile,
    label: String,
}

pub(crate) struct Doc {
    /// The theme's folder name (its key in the config).
    pub key: String,
    pub dir: PathBuf,
    pub file: ThemeFile,
    /// The table the theme was read from (keys Lodestar doesn't know, written back on save).
    pub raw: toml::Table,
    /// What is on disk now.
    pub saved: ThemeFile,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Changes with the same tag in a row are one undo step (typing in a field, one drag).
    last_tag: Option<String>,
    /// Bumped on every change (the preview rebuilds its theme).
    pub rev: u64,
}

impl Doc {
    pub fn new(key: String, dir: PathBuf, text: &str) -> anyhow::Result<Doc> {
        let raw: toml::Table = toml::from_str(text)?;
        let file = ThemeFile::deserialize_table(&raw)?;
        Ok(Doc { key, dir, saved: file.clone(), file, raw, undo: vec![], redo: vec![], last_tag: None, rev: 1 })
    }

    pub fn open(key: &str, dir: PathBuf) -> anyhow::Result<Doc> {
        let text = std::fs::read_to_string(dir.join("theme.toml"))
            .map_err(|e| anyhow::anyhow!("couldn't read its theme.toml ({e})"))?;
        Doc::new(key.to_string(), dir, &text).map_err(|e| anyhow::anyhow!("its theme.toml has a mistake: {e:#}"))
    }

    /// Change the theme as one undo step called `label`. False (and no step) when nothing
    /// changed.
    pub fn change(&mut self, label: &str, f: impl FnOnce(&mut ThemeFile)) -> bool {
        self.change_tagged(None, label, f)
    }

    /// Like `change`, but consecutive changes with the same `tag` merge into one undo step.
    pub fn change_merged(&mut self, tag: &str, label: &str, f: impl FnOnce(&mut ThemeFile)) -> bool {
        self.change_tagged(Some(tag), label, f)
    }

    fn change_tagged(&mut self, tag: Option<&str>, label: &str, f: impl FnOnce(&mut ThemeFile)) -> bool {
        let before = self.file.clone();
        f(&mut self.file);
        if self.file == before {
            return false;
        }
        if log::log_enabled!(log::Level::Debug) {
            let a = before.to_theme_toml(None).unwrap_or_default();
            let b = self.file.to_theme_toml(None).unwrap_or_default();
            let diff: Vec<String> =
                b.lines().filter(|l| !a.lines().any(|x| x == *l)).take(5).map(str::to_string).collect();
            log::debug!("theme creator change “{label}” (tag {tag:?}): {diff:?}");
        }
        let merge = tag.is_some() && self.last_tag.as_deref() == tag && !self.undo.is_empty();
        if !merge {
            self.undo.push(Step { file: before, label: label.to_string() });
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.last_tag = tag.map(str::to_string);
        self.redo.clear();
        self.rev += 1;
        true
    }

    /// Replace the whole file (raw TOML edits, a template applied) as one step.
    pub fn replace(&mut self, label: &str, file: ThemeFile) -> bool {
        self.change(label, |f| *f = file)
    }

    /// The next change starts a new undo step even with the same tag (a new drag).
    pub fn break_merge(&mut self) {
        self.last_tag = None;
    }

    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else { return false };
        let now = std::mem::replace(&mut self.file, step.file);
        self.redo.push(Step { file: now, label: step.label });
        self.last_tag = None;
        self.rev += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(step) = self.redo.pop() else { return false };
        let now = std::mem::replace(&mut self.file, step.file);
        self.undo.push(Step { file: now, label: step.label });
        self.last_tag = None;
        self.rev += 1;
        true
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    pub fn dirty(&self) -> bool {
        self.file != self.saved
    }

    /// The text to save.
    pub fn text(&self) -> anyhow::Result<String> {
        self.file.to_theme_toml(Some(&self.raw))
    }

    /// The text was written: that is what's on disk now.
    pub fn mark_saved(&mut self, file: ThemeFile) {
        self.saved = file;
    }
}
