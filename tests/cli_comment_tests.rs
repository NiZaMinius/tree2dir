//! End-to-end tests for the comment-related CLI flags:
//!
//! | Flag                    | Effect                                                   |
//! |-------------------------|----------------------------------------------------------|
//! | `--with-file-comments`  | write comments into files (using the file's comment syntax) |
//! | `--with-dir-comments`   | write comments into `COMMENT.md` inside each directory   |
//! | `--with-comments`       | shorthand for both                                       |
//!
//! Without any of these flags comments are parsed and stripped from names,
//! but nothing about them is persisted to disk.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cmd() -> Command {
    Command::cargo_bin("tree2dir").expect("tree2dir binary not found — run `cargo build` first")
}

fn write_tree_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, content).expect("failed to write temp tree file");
    path
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref())
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.as_ref().display()))
}

/// A tree that exercises dir comments, file comments, a non-commentable
/// binary extension, an extension-less-comment file and nesting.
const TREE: &str = "\
project/ # the project
├── src/ # source code
│   ├── main.rs # entry point
│   └── data.zip # archive
├── README.md # docs
└── notes.txt
";

/// Run tree2dir in File Mode against [`TREE`] with `extra_args`, returning the
/// output directory guard (so it stays alive for assertions).
fn generate(extra_args: &[&str]) -> TempDir {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let tree_file = write_tree_file(&input, "tree.txt", TREE);

    cmd()
        .args(["--file", tree_file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .args(extra_args)
        .assert()
        .success();

    output
}

// ---------------------------------------------------------------------------
// No flags: comments are stripped, nothing is persisted
// ---------------------------------------------------------------------------

#[test]
fn without_flags_comments_are_stripped_and_not_saved() {
    let out = generate(&[]);
    let root = out.path().join("project");

    // Names are clean (comment text did not leak into the paths).
    assert!(root.join("src").is_dir());
    assert!(root.join("src/main.rs").is_file());
    assert!(root.join("src/data.zip").is_file());
    assert!(root.join("README.md").is_file());
    assert!(root.join("notes.txt").is_file());

    // All files are empty and no COMMENT.md exists anywhere.
    assert_eq!(read(root.join("src/main.rs")), "");
    assert_eq!(read(root.join("README.md")), "");
    assert!(!root.join("COMMENT.md").exists());
    assert!(!root.join("src/COMMENT.md").exists());
}

// ---------------------------------------------------------------------------
// --with-file-comments
// ---------------------------------------------------------------------------

#[test]
fn with_file_comments_writes_comments_into_files_only() {
    let out = generate(&["--with-file-comments"]);
    let root = out.path().join("project");

    assert_eq!(read(root.join("src/main.rs")), "// entry point");
    assert_eq!(read(root.join("README.md")), "<!-- docs -->");

    // File without a comment stays empty.
    assert_eq!(read(root.join("notes.txt")), "");
    // Restricted extension stays empty despite having a comment.
    assert_eq!(fs::metadata(root.join("src/data.zip")).unwrap().len(), 0);

    // Directory comments were not requested.
    assert!(!root.join("COMMENT.md").exists());
    assert!(!root.join("src/COMMENT.md").exists());
}

// ---------------------------------------------------------------------------
// --with-dir-comments
// ---------------------------------------------------------------------------

#[test]
fn with_dir_comments_writes_comment_md_only() {
    let out = generate(&["--with-dir-comments"]);
    let root = out.path().join("project");

    assert_eq!(read(root.join("COMMENT.md")), "the project");
    assert_eq!(read(root.join("src/COMMENT.md")), "source code");

    // File comments were not requested.
    assert_eq!(read(root.join("src/main.rs")), "");
    assert_eq!(read(root.join("README.md")), "");
}

// ---------------------------------------------------------------------------
// --with-comments
// ---------------------------------------------------------------------------

#[test]
fn with_comments_enables_file_and_dir_comments() {
    let out = generate(&["--with-comments"]);
    let root = out.path().join("project");

    assert_eq!(read(root.join("COMMENT.md")), "the project");
    assert_eq!(read(root.join("src/COMMENT.md")), "source code");
    assert_eq!(read(root.join("src/main.rs")), "// entry point");
    assert_eq!(read(root.join("README.md")), "<!-- docs -->");
    assert_eq!(read(root.join("notes.txt")), "");
    assert_eq!(fs::metadata(root.join("src/data.zip")).unwrap().len(), 0);
}

#[test]
fn with_comments_matches_passing_both_specific_flags() {
    let combined = generate(&["--with-comments"]);
    let separate = generate(&["--with-file-comments", "--with-dir-comments"]);

    for rel in [
        "project/COMMENT.md",
        "project/src/COMMENT.md",
        "project/src/main.rs",
        "project/README.md",
        "project/notes.txt",
    ] {
        assert_eq!(
            read(combined.path().join(rel)),
            read(separate.path().join(rel)),
            "{rel} differs between --with-comments and the two specific flags"
        );
    }
}

// ---------------------------------------------------------------------------
// Dry-run
// ---------------------------------------------------------------------------

#[test]
fn dry_run_with_comments_writes_nothing() {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let tree_file = write_tree_file(&input, "tree.txt", TREE);

    cmd()
        .args(["--file", tree_file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .args(["--with-comments", "--dry-run"])
        .assert()
        .success();

    assert_eq!(
        fs::read_dir(output.path()).unwrap().count(),
        0,
        "dry-run must leave the output directory empty"
    );
}

// ---------------------------------------------------------------------------
// --force / existing files
// ---------------------------------------------------------------------------

#[test]
fn force_overwrites_existing_file_with_its_comment() {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let tree_file = write_tree_file(&input, "tree.txt", "README.md # docs\n");

    let target = output.path().join("README.md");
    fs::write(&target, "old content").unwrap();

    cmd()
        .args(["--file", tree_file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .args(["--with-file-comments", "--force"])
        .assert()
        .success();

    assert_eq!(read(&target), "<!-- docs -->");
}

#[test]
fn existing_file_is_preserved_without_force_even_when_comment_given() {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let tree_file = write_tree_file(&input, "tree.txt", "README.md # docs\n");

    let target = output.path().join("README.md");
    fs::write(&target, "precious").unwrap();

    cmd()
        .args(["--file", tree_file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .arg("--with-file-comments")
        .assert()
        .success();

    assert_eq!(read(&target), "precious");
}

// ---------------------------------------------------------------------------
// Input modes & input quirks
// ---------------------------------------------------------------------------

#[test]
fn pipe_mode_supports_comments() {
    let output = TempDir::new().unwrap();

    cmd()
        .args(["--output", output.path().to_str().unwrap()])
        .arg("--with-comments")
        .write_stdin("app/ # the app\n└── main.py # run me\n")
        .assert()
        .success();

    assert_eq!(read(output.path().join("app/COMMENT.md")), "the app");
    assert_eq!(read(output.path().join("app/main.py")), "# run me");
}

#[test]
fn quoted_names_with_spaces_and_comments() {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let tree = "\
root/
├── \"my file.txt\" # spaced name
└── 'other file.md' # single quoted
";
    let tree_file = write_tree_file(&input, "tree.txt", tree);

    cmd()
        .args(["--file", tree_file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .arg("--with-file-comments")
        .assert()
        .success();

    assert_eq!(
        read(output.path().join("root/my file.txt")),
        "// spaced name"
    );
    assert_eq!(
        read(output.path().join("root/other file.md")),
        "<!-- single quoted -->"
    );
}

#[test]
fn full_line_comments_do_not_become_entries() {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let tree = "\
# header comment
root/
# a note between entries
└── a.txt # real comment
";
    let tree_file = write_tree_file(&input, "tree.txt", tree);

    cmd()
        .args(["--file", tree_file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .arg("--with-file-comments")
        .assert()
        .success();

    let top_level: Vec<_> = fs::read_dir(output.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(top_level, vec![std::ffi::OsString::from("root")]);

    let root_entries: Vec<_> = fs::read_dir(output.path().join("root"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(root_entries, vec![std::ffi::OsString::from("a.txt")]);
    assert_eq!(read(output.path().join("root/a.txt")), "// real comment");
}

#[test]
fn unicode_comments_survive_the_round_trip() {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let tree_file = write_tree_file(&input, "tree.txt", "a.py # 日本語 ✓ привет\n");

    cmd()
        .args(["--file", tree_file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .arg("--with-file-comments")
        .assert()
        .success();

    assert_eq!(read(output.path().join("a.py")), "# 日本語 ✓ привет");
}

// ---------------------------------------------------------------------------
// Summary output
// ---------------------------------------------------------------------------

#[test]
fn summary_tree_shows_comments() {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let tree_file = write_tree_file(&input, "tree.txt", TREE);

    let assert = cmd()
        .args(["--file", tree_file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        stdout.contains("main.rs\t# entry point"),
        "stdout was:\n{stdout}"
    );
    assert!(
        stdout.contains("src\t# source code"),
        "stdout was:\n{stdout}"
    );
    assert!(
        stdout.contains("project\t# the project"),
        "stdout was:\n{stdout}"
    );
}
