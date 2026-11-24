use super::*;

#[test]
fn write_adds_the_missing_extension() {
    let dir = tempfile::tempdir().unwrap();
    let Saved::Done(p) = write_export(Format::Html, dir.path().join("notes")) else {
        panic!("the write should succeed");
    };
    assert_eq!(p, dir.path().join("notes.html"));
    assert!(std::fs::read_to_string(&p).unwrap().starts_with("<!doctype html>"));
    // A name that has an extension keeps it.
    let Saved::Done(p) = write_export(Format::Text, dir.path().join("log.v2")) else {
        panic!("the write should succeed");
    };
    assert_eq!(p, dir.path().join("log.v2"));
}

#[test]
fn write_failure_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let r = write_export(Format::Markdown, dir.path().join("missing/x.md"));
    assert!(matches!(r, Saved::Failed(e) if e.contains("Couldn't save the changelog")));
}

#[test]
fn a_helper_that_dies_is_an_error_not_a_silent_cancel() {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let job = ChangelogSave { rx, _hold: None };
    assert!(job.poll().is_none());
    drop(tx);
    assert!(matches!(job.poll(), Some(Saved::Failed(_))));
    assert!(ChangelogSave::pending_forever().poll().is_none());
}

#[test]
fn browser_copy_is_the_web_page_under_one_name_per_version() {
    let dir = tempfile::tempdir().unwrap();
    // The folder is made when missing.
    let run = dir.path().join("run");
    let p = write_browser_copy(&run).unwrap();
    assert_eq!(p, run.join(format!("lodestar-changelog-{}.html", version())));
    let page = std::fs::read_to_string(&p).unwrap();
    assert!(page.starts_with("<!doctype html>"));
    // Opening it again overwrites the same file.
    std::fs::write(&p, "old").unwrap();
    assert_eq!(write_browser_copy(&run).unwrap(), p);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), page);
    // A folder that can't be made is an error message, not a panic.
    let file = dir.path().join("a-file");
    std::fs::write(&file, "x").unwrap();
    let e = write_browser_copy(&file.join("sub")).unwrap_err();
    assert!(e.contains("Couldn't write the changelog page"), "{e}");
}
