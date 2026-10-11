//! What a held directory refuses, and what renaming it away cannot redirect.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rivet-beneath-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn text(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

/// **The property the module exists for.** A store held open, then renamed
/// aside with a link planted under its old name: a directory made, a file
/// created and an entry renamed through the held descriptor all land in the
/// real directory, and nothing lands where the link points.
#[cfg(unix)]
#[test]
fn a_held_directory_renamed_aside_and_replaced_by_a_link_is_still_the_one_written() {
    use std::io::Write as _;

    let dir = scratch("swap");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    let store = dir.join("store");

    let held = Dir::anchor(&text(&store)).expect("anchored");
    std::fs::rename(&store, dir.join("aside")).expect("renamed aside");
    std::os::unix::fs::symlink(&outside, &store).expect("the planted link");

    let incoming = held.make("incoming").expect("made through the held descriptor");
    let mut file = incoming.create_new("object.rvt").expect("created");
    file.write_all(b"bytes").expect("written");
    drop(file);
    let objects = held.make("objects").expect("made");
    incoming.rename_to("object.rvt", &objects, "object.rvt").expect("renamed");

    assert_eq!(std::fs::read_dir(&outside).expect("outside").count(), 0, "written through the link");
    assert_eq!(
        std::fs::read(dir.join("aside").join("objects").join("object.rvt")).expect("in the real store"),
        b"bytes"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A link where a directory belongs is refused, by `anchor`, `existing`,
/// `make` and `open` alike, and is told apart from absence.
#[cfg(unix)]
#[test]
fn a_link_where_a_directory_belongs_is_refused_and_not_absent() {
    let dir = scratch("link");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    let planted = dir.join("planted");
    std::os::unix::fs::symlink(&outside, &planted).expect("the planted link");

    let e = Dir::anchor(&text(&planted)).expect_err("anchor through a link");
    assert!(is_not_real(&e), "{e}");
    let e = Dir::existing(&text(&planted)).expect_err("open through a link");
    assert!(is_not_real(&e), "{e}");
    let parent = Dir::existing(&text(&dir)).expect("the scratch directory");
    assert!(is_not_real(&parent.make("planted").expect_err("make over a link")));
    assert!(is_not_real(&parent.open("planted").expect_err("open a link")));
    assert_eq!(parent.kind("planted").expect("a kind"), Kind::Link);

    let absent = parent.open("absent").expect_err("absent");
    assert_eq!(absent.kind(), std::io::ErrorKind::NotFound);
    assert!(!is_not_real(&absent));

    assert_eq!(std::fs::read_dir(&outside).expect("outside").count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A new file is refused over anything already there**, a link included,
/// so nothing is truncated and nothing is followed.
#[cfg(unix)]
#[test]
fn a_new_file_is_refused_over_a_planted_link() {
    let dir = scratch("create");
    let target = dir.join("target");
    std::fs::write(&target, b"not yours").expect("target");
    std::os::unix::fs::symlink(&target, dir.join("name")).expect("the planted link");
    let held = Dir::existing(&text(&dir)).expect("held");
    let e = held.create_new("name").expect_err("refused");
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists, "{e}");
    assert_eq!(std::fs::read(&target).expect("target"), b"not yours");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Removal removes the link, never what it points at**, and empties a real
/// directory first.
#[cfg(unix)]
#[test]
fn removal_takes_a_link_itself_and_a_directorys_contents() {
    let dir = scratch("remove");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join("keep"), b"keep").expect("keep");
    std::os::unix::fs::symlink(&outside, dir.join("link")).expect("link");
    std::fs::create_dir_all(dir.join("real")).expect("real");
    std::fs::write(dir.join("real").join("one"), b"1").expect("one");

    let held = Dir::existing(&text(&dir)).expect("held");
    held.remove("link").expect("the link removed");
    held.remove("real").expect("the directory removed");
    assert!(std::fs::symlink_metadata(dir.join("link")).is_err());
    assert!(!dir.join("real").exists());
    assert!(outside.join("keep").is_file(), "removal followed the link");
    assert_eq!(held.remove("absent").expect_err("absent").kind(), std::io::ErrorKind::NotFound);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A name that is a path is refused before anything is asked of the kernel.
#[test]
fn a_path_is_not_a_name() {
    let dir = scratch("names");
    let held = Dir::existing(&text(&dir)).expect("held");
    for name in ["", ".", "..", "a/b", "../x"] {
        assert_eq!(
            held.open(name).expect_err(name).kind(),
            std::io::ErrorKind::InvalidInput,
            "{name:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
