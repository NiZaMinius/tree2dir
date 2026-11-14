//! No write may leave the output root through a symbolic link.
//!
//! The path guard in the builder is lexical (`..`, absolute paths), so it
//! cannot see that `out/link` is really `/somewhere/else`.  Symlinks need extra
//! privileges on Windows, so these tests run on Unix only.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;
use tree2dir::comments::CommentMode;
use tree2dir::error::AppError;
use tree2dir::fs_builder::{build, BuildOptions};
use tree2dir::parser::parse_tree;

/// An output root and, beside it, a directory that must never be written to.
struct Arena {
    _tmp: TempDir,
    out: PathBuf,
    outside: PathBuf,
}

fn arena() -> Arena {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out");
    let outside = tmp.path().join("outside");
    fs::create_dir(&out).unwrap();
    fs::create_dir(&outside).unwrap();
    Arena {
        _tmp: tmp,
        out,
        outside,
    }
}

fn opts(root: &Path, dry_run: bool, force: bool, comments: CommentMode) -> BuildOptions {
    BuildOptions {
        output_root: root.to_path_buf(),
        dry_run,
        force,
        comments,
    }
}

fn is_escape(result: &Result<impl std::fmt::Debug, AppError>) -> bool {
    matches!(result, Err(AppError::SymlinkEscape(_)))
}

fn outside_is_untouched(a: &Arena) -> bool {
    fs::read_dir(&a.outside).unwrap().count() == 0
}

/// Run the build as a dry run and as a real run; both must refuse.
fn assert_refused(a: &Arena, tree: &str, force: bool, comments: CommentMode) {
    for dry_run in [true, false] {
        let result = build(&parse_tree(tree), &opts(&a.out, dry_run, force, comments));
        assert!(
            is_escape(&result),
            "dry_run={dry_run}: expected SymlinkEscape, got {result:?}"
        );
        assert!(outside_is_untouched(a), "dry_run={dry_run}: wrote outside");
    }
}

#[test]
fn file_below_a_directory_symlink_is_refused() {
    let a = arena();
    symlink(&a.outside, a.out.join("link")).unwrap();
    assert_refused(&a, "link/\n└── a.txt\n", false, CommentMode::None);
}

#[test]
fn nested_directory_below_a_directory_symlink_is_refused() {
    let a = arena();
    symlink(&a.outside, a.out.join("link")).unwrap();
    assert_refused(
        &a,
        "link/\n└── deep/\n    └── a.txt\n",
        false,
        CommentMode::None,
    );
}

#[test]
fn directory_comment_through_a_directory_symlink_is_refused() {
    let a = arena();
    symlink(&a.outside, a.out.join("link")).unwrap();
    assert_refused(&a, "link/ # note\n", false, CommentMode::Dirs);
}

#[test]
fn directory_entry_that_is_a_symlink_out_of_the_root_is_refused_even_when_empty() {
    // Conservative on purpose: a later run, or a later entry, would write
    // through it.
    let a = arena();
    symlink(&a.outside, a.out.join("link")).unwrap();
    assert_refused(&a, "link/\n", false, CommentMode::None);
}

#[test]
fn forced_overwrite_of_a_symlinked_file_outside_the_root_is_refused() {
    let a = arena();
    let victim = a.outside.join("victim.txt");
    fs::write(&victim, "precious").unwrap();
    symlink(&victim, a.out.join("a.txt")).unwrap();

    for dry_run in [true, false] {
        let result = build(
            &parse_tree("a.txt\n"),
            &opts(&a.out, dry_run, true, CommentMode::None),
        );
        assert!(is_escape(&result), "dry_run={dry_run}: {result:?}");
        assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
    }
}

#[test]
fn dangling_symlink_would_create_its_target_so_it_is_refused() {
    // `exists()` is false for a dangling link, so a check built on it would
    // let fs::write create the file outside.
    let a = arena();
    symlink(a.outside.join("new.txt"), a.out.join("a.txt")).unwrap();
    assert_refused(&a, "a.txt\n", false, CommentMode::None);
}

#[test]
fn comment_md_that_is_a_symlink_out_of_the_root_is_refused() {
    let a = arena();
    fs::create_dir(a.out.join("proj")).unwrap();
    symlink(a.outside.join("new.md"), a.out.join("proj/COMMENT.md")).unwrap();
    assert_refused(&a, "proj/ # about\n", true, CommentMode::Dirs);
}

#[test]
fn file_comment_through_a_symlinked_directory_is_refused() {
    let a = arena();
    symlink(&a.outside, a.out.join("src")).unwrap();
    assert_refused(&a, "src/\n└── main.rs # entry\n", false, CommentMode::Files);
}

// ---------------------------------------------------------------------------
// Symlinks that stay inside are fine
// ---------------------------------------------------------------------------

#[test]
fn symlink_to_another_directory_inside_the_root_is_allowed() {
    let a = arena();
    fs::create_dir(a.out.join("real")).unwrap();
    symlink(a.out.join("real"), a.out.join("alias")).unwrap();

    let result = build(
        &parse_tree("alias/\n└── a.txt\n"),
        &opts(&a.out, false, false, CommentMode::None),
    );

    assert!(result.is_ok(), "{result:?}");
    assert!(a.out.join("real/a.txt").is_file());
}

#[test]
fn an_output_root_that_is_itself_a_symlink_is_allowed() {
    let a = arena();
    let real = a.out.join("real-root");
    fs::create_dir(&real).unwrap();
    let alias = a.out.join("alias-root");
    symlink(&real, &alias).unwrap();

    let result = build(
        &parse_tree("proj/\n└── a.txt # c\n"),
        &opts(&alias, false, false, CommentMode::All),
    );

    assert!(result.is_ok(), "{result:?}");
    assert!(real.join("proj/a.txt").is_file());
}

#[test]
fn relative_output_root_is_checked_too() {
    // `--output .` style roots must not skip the check.
    let a = arena();
    symlink(&a.outside, a.out.join("link")).unwrap();
    let relative = pathdiff(&a.out);
    let result = build(
        &parse_tree("link/\n└── a.txt\n"),
        &opts(&relative, false, false, CommentMode::None),
    );
    assert!(is_escape(&result), "{result:?}");
    assert!(outside_is_untouched(&a));
}

/// `out` expressed with `..` so the root is not a plain absolute path.
fn pathdiff(out: &Path) -> PathBuf {
    out.join("..").join("out")
}

// ---------------------------------------------------------------------------
// The command line
// ---------------------------------------------------------------------------

#[test]
fn cli_explains_the_refusal_and_exits_non_zero() {
    let a = arena();
    symlink(&a.outside, a.out.join("link")).unwrap();

    Command::cargo_bin("tree2dir")
        .unwrap()
        .args(["--output", a.out.to_str().unwrap()])
        .write_stdin("link/\n└── a.txt\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("symbolic link"))
        .stderr(predicate::str::contains("link"));
    assert!(outside_is_untouched(&a));
}

#[test]
fn cli_dry_run_refuses_too() {
    let a = arena();
    symlink(&a.outside, a.out.join("link")).unwrap();

    Command::cargo_bin("tree2dir")
        .unwrap()
        .args(["--output", a.out.to_str().unwrap(), "--dry-run"])
        .write_stdin("link/\n└── a.txt\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("symbolic link"));
}

// ---------------------------------------------------------------------------
// The guard remembers verified directories; that must not let anything through
// ---------------------------------------------------------------------------

#[test]
fn a_verified_directory_does_not_vouch_for_its_siblings() {
    let a = arena();
    symlink(&a.outside, a.out.join("bad")).unwrap();
    // `ok/` is checked and remembered first; `bad/` must still be refused.
    let tree = "ok/\n├── a.txt\n└── b.txt\nbad/\n└── c.txt\n";

    for dry_run in [true, false] {
        let result = build(
            &parse_tree(tree),
            &opts(&a.out, dry_run, false, CommentMode::None),
        );
        assert!(is_escape(&result), "dry_run={dry_run}: {result:?}");
        assert!(outside_is_untouched(&a));
    }
}

#[test]
fn a_verified_parent_does_not_vouch_for_a_symlinked_file_below_it() {
    let a = arena();
    fs::create_dir(a.out.join("d")).unwrap();
    fs::write(a.outside.join("victim"), "precious").unwrap();
    symlink(a.outside.join("victim"), a.out.join("d/evil.txt")).unwrap();
    // Plenty of plain files first, so `d` is long since verified.
    let tree = "d/\n├── 1.txt\n├── 2.txt\n├── 3.txt\n└── evil.txt\n";

    for dry_run in [true, false] {
        let result = build(
            &parse_tree(tree),
            &opts(&a.out, dry_run, true, CommentMode::None),
        );
        assert!(is_escape(&result), "dry_run={dry_run}: {result:?}");
        assert_eq!(
            fs::read_to_string(a.outside.join("victim")).unwrap(),
            "precious"
        );
    }
}

#[test]
fn a_verified_ancestor_does_not_vouch_for_a_symlinked_subdirectory() {
    let a = arena();
    fs::create_dir(a.out.join("a")).unwrap();
    symlink(&a.outside, a.out.join("a/b")).unwrap();
    let tree = "a/\n├── x.txt\n└── b/\n    └── c.txt\n";

    for dry_run in [true, false] {
        let result = build(
            &parse_tree(tree),
            &opts(&a.out, dry_run, false, CommentMode::None),
        );
        assert!(is_escape(&result), "dry_run={dry_run}: {result:?}");
        assert!(outside_is_untouched(&a));
    }
}
