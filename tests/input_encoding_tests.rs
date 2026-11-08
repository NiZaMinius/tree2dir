//! Input encodings at the process boundary, driven by real byte fixtures.
//!
//! Each fixture is the byte sequence a real Windows tool produces, so a
//! regression shows up as the same failure a user would see.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

const TREE: &str = "my-app/\n├── src/\n│   └── main.rs\n└── README.md\n";

fn cmd() -> Command {
    Command::cargo_bin("tree2dir").expect("tree2dir binary not found — run `cargo build` first")
}

fn utf8_bom(text: &str) -> Vec<u8> {
    let mut out = vec![0xEF, 0xBB, 0xBF];
    out.extend(text.as_bytes());
    out
}

/// What PowerShell 5.1 `tree > out.txt` / `Out-File` writes.
fn utf16_le_bom(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    out
}

fn utf16_be_bom(text: &str) -> Vec<u8> {
    let mut out = vec![0xFE, 0xFF];
    out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
    out
}

/// Run against `bytes` as a `--file` and return the output directory.
fn build_from_file(bytes: &[u8]) -> TempDir {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let file = input.path().join("tree.txt");
    fs::write(&file, bytes).unwrap();

    cmd()
        .args(["--file", file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .assert()
        .success();
    output
}

fn assert_expected_structure(root: &Path) {
    assert!(root.join("my-app").is_dir(), "my-app/ missing");
    assert!(root.join("my-app/src/main.rs").is_file());
    assert!(root.join("my-app/README.md").is_file());
}

/// The names that exist directly under `root`, to catch stray BOM-named folders.
fn top_level_names(root: &Path) -> Vec<String> {
    fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// Supported encodings
// ---------------------------------------------------------------------------

#[test]
fn utf8_without_bom() {
    let out = build_from_file(TREE.as_bytes());
    assert_expected_structure(out.path());
}

#[test]
fn utf8_with_bom_does_not_leak_into_the_first_name() {
    let out = build_from_file(&utf8_bom(TREE));
    assert_expected_structure(out.path());
    assert_eq!(top_level_names(out.path()), ["my-app"]);
}

#[test]
fn utf16_le_with_bom() {
    let out = build_from_file(&utf16_le_bom(TREE));
    assert_expected_structure(out.path());
    assert_eq!(top_level_names(out.path()), ["my-app"]);
}

#[test]
fn utf16_be_with_bom() {
    let out = build_from_file(&utf16_be_bom(TREE));
    assert_expected_structure(out.path());
}

#[test]
fn utf16_with_crlf_and_non_ascii_names() {
    let tree = "проект/\r\n├── 日本語.md\r\n└── 📁 notes.txt\r\n";
    let out = build_from_file(&utf16_le_bom(tree));

    assert!(out.path().join("проект/日本語.md").is_file());
    assert!(out.path().join("проект/📁 notes.txt").is_file());
}

#[test]
fn comments_survive_utf16() {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let file = input.path().join("tree.txt");
    fs::write(&file, utf16_le_bom("main.rs # точка входа\r\n")).unwrap();

    cmd()
        .args(["--file", file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .args(["--comments", "files"])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(output.path().join("main.rs")).unwrap(),
        "// точка входа\n"
    );
}

// ---------------------------------------------------------------------------
// Rejected encodings: a clear message, nothing written
// ---------------------------------------------------------------------------

#[test]
fn windows_1251_file_is_rejected_with_instructions() {
    let input = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();
    let file = input.path().join("tree.txt");
    // "проект/" in Windows-1251.
    fs::write(&file, [0xEF, 0xF0, 0xEE, 0xE5, 0xEA, 0xF2, b'/', b'\n']).unwrap();

    cmd()
        .args(["--file", file.to_str().unwrap()])
        .args(["--output", output.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not valid UTF-8"))
        .stderr(predicate::str::contains("Save it as UTF-8"))
        .stderr(predicate::str::contains("tree.txt"));

    assert_eq!(fs::read_dir(output.path()).unwrap().count(), 0);
}

#[test]
fn utf16_without_bom_is_rejected_and_named() {
    let input = TempDir::new().unwrap();
    let file = input.path().join("tree.txt");
    let bytes: Vec<u8> = TREE.encode_utf16().flat_map(u16::to_le_bytes).collect();
    fs::write(&file, bytes).unwrap();

    cmd()
        .args(["--file", file.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("UTF-16 without a byte-order mark"));
}

#[test]
fn truncated_utf16_is_rejected() {
    let input = TempDir::new().unwrap();
    let file = input.path().join("tree.txt");
    let mut bytes = utf16_le_bom(TREE);
    bytes.pop();
    fs::write(&file, bytes).unwrap();

    cmd()
        .args(["--file", file.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("odd number of bytes"));
}

#[test]
fn bom_only_file_is_reported_as_empty_not_as_an_encoding_error() {
    let input = TempDir::new().unwrap();
    let file = input.path().join("tree.txt");
    fs::write(&file, [0xEF, 0xBB, 0xBF]).unwrap();

    cmd()
        .args(["--file", file.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No recognisable entries"));
}

// ---------------------------------------------------------------------------
// Pipe mode goes through the same decoder
// ---------------------------------------------------------------------------

#[test]
fn stdin_accepts_utf16_le_with_bom() {
    let output = TempDir::new().unwrap();

    cmd()
        .args(["--output", output.path().to_str().unwrap()])
        .write_stdin(utf16_le_bom(TREE))
        .assert()
        .success();

    assert_expected_structure(output.path());
}

#[test]
fn stdin_accepts_utf8_with_bom() {
    let output = TempDir::new().unwrap();

    cmd()
        .args(["--output", output.path().to_str().unwrap()])
        .write_stdin(utf8_bom(TREE))
        .assert()
        .success();

    assert_eq!(top_level_names(output.path()), ["my-app"]);
}

#[test]
fn stdin_rejects_invalid_utf8_with_instructions() {
    cmd()
        .write_stdin(vec![0xEF, 0xF0, 0xEE, 0xFF])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Save it as UTF-8"))
        .stderr(predicate::str::contains("stdin"));
}

// ---------------------------------------------------------------------------
// Line endings
// ---------------------------------------------------------------------------

#[test]
fn crlf_input_builds_the_same_structure() {
    let crlf = TREE.replace('\n', "\r\n");
    let out = build_from_file(crlf.as_bytes());
    assert_expected_structure(out.path());
}

#[test]
fn lone_cr_line_endings_build_the_same_structure() {
    // Classic Mac text. Without splitting on `\r` this was a single line.
    let cr = TREE.replace('\n', "\r");
    let out = build_from_file(cr.as_bytes());
    assert_expected_structure(out.path());
}

#[test]
fn mixed_line_endings_in_one_file() {
    let mixed = "my-app/\r\n├── src/\n│   └── main.rs\r└── README.md";
    let out = build_from_file(mixed.as_bytes());
    assert_expected_structure(out.path());
}
