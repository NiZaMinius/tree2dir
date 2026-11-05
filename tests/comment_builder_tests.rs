//! Builder-level tests for saving comments to disk.
//!
//! These call [`tree2dir::fs_builder::build`] directly (no binary involved),
//! which makes it easy to assert on [`BuildStats`] and to exercise each
//! option combination in isolation.
//!
//! Behaviour under test:
//!
//! - `file_comments`: the comment is written *into* the file, formatted with
//!   the comment syntax of its extension. Binary / extension-less files stay
//!   empty.
//! - `dir_comments`: the comment is written, unformatted, to `COMMENT.md`
//!   inside the new directory.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use tree2dir::fs_builder::{build, BuildOptions, BuildStats};
use tree2dir::parser::parse_tree;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn opts(root: &Path, file_comments: bool, dir_comments: bool) -> BuildOptions {
    BuildOptions {
        output_root: root.to_path_buf(),
        dry_run: false,
        force: false,
        file_comments,
        dir_comments,
    }
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref())
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.as_ref().display()))
}

// ---------------------------------------------------------------------------
// File comments: formatting per extension
// ---------------------------------------------------------------------------

#[test]
fn file_comment_uses_comment_syntax_of_the_extension() {
    let out = TempDir::new().unwrap();

    let cases: &[(&str, &str)] = &[
        // C-style
        ("a.rs", "// note"),
        ("a.js", "// note"),
        ("a.ts", "// note"),
        ("a.c", "// note"),
        ("a.go", "// note"),
        ("a.java", "// note"),
        // Hash style
        ("a.py", "# note"),
        ("a.sh", "# note"),
        ("a.rb", "# note"),
        ("a.yml", "# note"),
        ("a.yaml", "# note"),
        ("a.toml", "# note"),
        ("a.ini", "# note"),
        // Double dash
        ("a.sql", "-- note"),
        ("a.lua", "-- note"),
        ("a.hs", "-- note"),
        // Markup
        ("a.html", "<!-- note -->"),
        ("a.xml", "<!-- note -->"),
        ("a.svg", "<!-- note -->"),
        ("a.md", "<!-- note -->"),
        // Unknown extension falls back to `//`
        ("a.txt", "// note"),
        ("a.weird", "// note"),
        // Extension matching is case-insensitive
        ("UPPER.PY", "# note"),
        ("Mixed.Md", "<!-- note -->"),
    ];

    let tree: String = cases
        .iter()
        .map(|(name, _)| format!("{name} # note\n"))
        .collect();
    let nodes = parse_tree(&tree);
    assert_eq!(nodes.len(), cases.len());

    build(&nodes, &opts(out.path(), true, false)).unwrap();

    for (name, expected) in cases {
        assert_eq!(
            read(out.path().join(name)),
            *expected,
            "wrong comment format for {name}"
        );
    }
}

#[test]
fn file_comment_has_no_trailing_newline_and_no_extra_content() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("main.rs # entry point");
    build(&nodes, &opts(out.path(), true, false)).unwrap();

    assert_eq!(
        fs::read(out.path().join("main.rs")).unwrap(),
        b"// entry point"
    );
}

#[test]
fn file_comment_keeps_unicode_intact() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("a.py # 日本語 ✓ привет");
    build(&nodes, &opts(out.path(), true, false)).unwrap();

    assert_eq!(read(out.path().join("a.py")), "# 日本語 ✓ привет");
}

// ---------------------------------------------------------------------------
// File comments: files that must stay empty
// ---------------------------------------------------------------------------

#[test]
fn restricted_extensions_are_created_empty() {
    let out = TempDir::new().unwrap();

    let names = [
        "archive.zip",
        "archive.tar",
        "archive.gz",
        "image.png",
        "image.jpg",
        "song.mp3",
        "program.exe",
        "lib.so",
        "Module.class",
        "cache.pyc",
        "module.wasm",
        "data.sqlite",
        "SHOUTING.ZIP", // case-insensitive
    ];
    let tree: String = names
        .iter()
        .map(|n| format!("{n} # should not be written\n"))
        .collect();
    let nodes = parse_tree(&tree);

    let stats = build(&nodes, &opts(out.path(), true, false)).unwrap();

    assert_eq!(stats.files_created, names.len());
    for name in names {
        let path = out.path().join(name);
        assert!(path.is_file(), "{name} should have been created");
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            0,
            "{name} must stay empty (format cannot hold a text comment)"
        );
    }
}

#[test]
fn extensionless_file_is_created_empty() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("Makefile # build rules");
    build(&nodes, &opts(out.path(), true, false)).unwrap();

    assert!(out.path().join("Makefile").is_file());
    assert_eq!(read(out.path().join("Makefile")), "");
}

#[test]
fn file_without_comment_is_empty_even_when_file_comments_enabled() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("plain.rs\nempty_comment.rs #");
    build(&nodes, &opts(out.path(), true, false)).unwrap();

    assert_eq!(read(out.path().join("plain.rs")), "");
    assert_eq!(read(out.path().join("empty_comment.rs")), "");
}

#[test]
fn file_comments_disabled_creates_empty_file() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("main.rs # entry point");
    build(&nodes, &opts(out.path(), false, false)).unwrap();

    assert_eq!(read(out.path().join("main.rs")), "");
}

// ---------------------------------------------------------------------------
// Directory comments
// ---------------------------------------------------------------------------

#[test]
fn dir_comment_is_written_verbatim_to_comment_md() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("src/ # source code");
    build(&nodes, &opts(out.path(), false, true)).unwrap();

    let comment_file = out.path().join("src").join("COMMENT.md");
    assert!(comment_file.is_file(), "src/COMMENT.md should exist");
    // Not wrapped in <!-- --> and no trailing newline.
    assert_eq!(read(comment_file), "source code");
}

#[test]
fn dir_comments_disabled_writes_no_comment_md() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("src/ # source code");
    build(&nodes, &opts(out.path(), false, false)).unwrap();

    assert!(out.path().join("src").is_dir());
    assert!(!out.path().join("src").join("COMMENT.md").exists());
}

#[test]
fn dir_without_comment_gets_no_comment_md() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("src/\nempty_comment/ #");
    build(&nodes, &opts(out.path(), true, true)).unwrap();

    assert!(!out.path().join("src").join("COMMENT.md").exists());
    assert!(!out.path().join("empty_comment").join("COMMENT.md").exists());
}

#[test]
fn nested_dirs_each_get_their_own_comment_md() {
    let out = TempDir::new().unwrap();
    let tree = "\
root/ # top level
└── src/ # inner level
    └── main.rs # entry
";
    let nodes = parse_tree(tree);
    let stats = build(&nodes, &opts(out.path(), true, true)).unwrap();

    assert_eq!(read(out.path().join("root/COMMENT.md")), "top level");
    assert_eq!(read(out.path().join("root/src/COMMENT.md")), "inner level");
    assert_eq!(read(out.path().join("root/src/main.rs")), "// entry");

    // COMMENT.md files are side effects, not counted as tree entries.
    assert_eq!(
        stats,
        BuildStats {
            dirs_created: 2,
            files_created: 1,
            files_skipped: 0,
        }
    );
}

#[test]
fn dir_comment_for_quoted_dir_with_spaces() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("\"my docs/\" # user docs");
    build(&nodes, &opts(out.path(), false, true)).unwrap();

    assert_eq!(read(out.path().join("my docs/COMMENT.md")), "user docs");
}

// ---------------------------------------------------------------------------
// Option independence
// ---------------------------------------------------------------------------

const MIXED_TREE: &str = "\
proj/ # the project
└── main.rs # entry
";

#[test]
fn file_comments_only_does_not_write_dir_comments() {
    let out = TempDir::new().unwrap();
    build(&parse_tree(MIXED_TREE), &opts(out.path(), true, false)).unwrap();

    assert_eq!(read(out.path().join("proj/main.rs")), "// entry");
    assert!(!out.path().join("proj/COMMENT.md").exists());
}

#[test]
fn dir_comments_only_does_not_write_file_comments() {
    let out = TempDir::new().unwrap();
    build(&parse_tree(MIXED_TREE), &opts(out.path(), false, true)).unwrap();

    assert_eq!(read(out.path().join("proj/COMMENT.md")), "the project");
    assert_eq!(read(out.path().join("proj/main.rs")), "");
}

#[test]
fn both_options_write_both_kinds_of_comment() {
    let out = TempDir::new().unwrap();
    build(&parse_tree(MIXED_TREE), &opts(out.path(), true, true)).unwrap();

    assert_eq!(read(out.path().join("proj/COMMENT.md")), "the project");
    assert_eq!(read(out.path().join("proj/main.rs")), "// entry");
}

// ---------------------------------------------------------------------------
// Interaction with --force / --dry-run
// ---------------------------------------------------------------------------

#[test]
fn existing_file_is_not_touched_without_force_even_with_comment() {
    let out = TempDir::new().unwrap();
    let target = out.path().join("main.rs");
    fs::write(&target, "original").unwrap();

    let stats = build(
        &parse_tree("main.rs # entry"),
        &opts(out.path(), true, false),
    )
    .unwrap();

    assert_eq!(read(&target), "original");
    assert_eq!(stats.files_skipped, 1);
    assert_eq!(stats.files_created, 0);
}

#[test]
fn force_overwrites_existing_file_with_comment() {
    let out = TempDir::new().unwrap();
    let target = out.path().join("main.rs");
    fs::write(&target, "original").unwrap();

    let mut o = opts(out.path(), true, false);
    o.force = true;
    let stats = build(&parse_tree("main.rs # entry"), &o).unwrap();

    assert_eq!(read(&target), "// entry");
    assert_eq!(stats.files_created, 1);
    assert_eq!(stats.files_skipped, 0);
}

#[test]
fn dry_run_with_comments_writes_nothing() {
    let out = TempDir::new().unwrap();

    let mut o = opts(out.path(), true, true);
    o.dry_run = true;
    let stats = build(&parse_tree(MIXED_TREE), &o).unwrap();

    assert!(!out.path().join("proj").exists());
    assert_eq!(stats.dirs_created, 1, "dry-run still reports planned work");
    assert_eq!(stats.files_created, 1);
}
