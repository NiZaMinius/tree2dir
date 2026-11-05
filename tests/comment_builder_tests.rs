//! Builder-level tests for saving comments to disk.
//!
//! These call [`tree2dir::fs_builder::build`] directly (no binary involved),
//! which makes it easy to assert on [`BuildStats`] and to exercise each
//! option combination in isolation.
//!
//! Behaviour under test:
//!
//! - `CommentMode::Files`: the comment is written *into* the file, formatted with
//!   the comment syntax of its extension. Binary / extension-less files stay
//!   empty.
//! - `CommentMode::Dirs`: the comment is written as plain text to `COMMENT.md`
//!   inside the directory.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use tree2dir::comments::CommentMode;
use tree2dir::fs_builder::{build, BuildOptions, BuildStats};
use tree2dir::parser::parse_tree;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn opts(root: &Path, file_comments: bool, dir_comments: bool) -> BuildOptions {
    BuildOptions {
        output_root: root.to_path_buf(),
        comments: CommentMode::from_switches(file_comments, dir_comments),
        ..Default::default()
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
        ("a.ini", "; note"),
        // Double dash
        ("a.sql", "-- note"),
        ("a.lua", "-- note"),
        ("a.hs", "-- note"),
        // Markup
        ("a.html", "<!-- note -->"),
        ("a.xml", "<!-- note -->"),
        ("a.svg", "<!-- note -->"),
        ("a.md", "<!-- note -->"),
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
            format!("{expected}\n"),
            "wrong comment format for {name}"
        );
    }
}

#[test]
fn file_comment_is_one_line_ending_in_exactly_one_newline() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("main.rs # entry point");
    build(&nodes, &opts(out.path(), true, false)).unwrap();

    assert_eq!(
        fs::read(out.path().join("main.rs")).unwrap(),
        b"// entry point
"
    );
}

#[test]
fn file_comment_keeps_unicode_intact() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("a.py # 日本語 ✓ привет");
    build(&nodes, &opts(out.path(), true, false)).unwrap();

    assert_eq!(read(out.path().join("a.py")), "# 日本語 ✓ привет\n");
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
fn dir_comment_is_written_as_plain_text_to_comment_md() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("src/ # source code");
    build(&nodes, &opts(out.path(), false, true)).unwrap();

    let comment_file = out.path().join("src").join("COMMENT.md");
    assert!(comment_file.is_file(), "src/COMMENT.md should exist");
    // Plain text (not wrapped in <!-- -->) ending in one newline.
    assert_eq!(read(comment_file), "source code\n");
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

    assert_eq!(read(out.path().join("root/COMMENT.md")), "top level\n");
    assert_eq!(
        read(out.path().join("root/src/COMMENT.md")),
        "inner level\n"
    );
    assert_eq!(read(out.path().join("root/src/main.rs")), "// entry\n");

    // COMMENT.md files are side effects, not counted as tree entries.
    assert_eq!(
        stats,
        BuildStats {
            dirs_created: 2,
            files_created: 1,
            files_skipped: 0,
            comments_written: 3,
            comments_skipped: 0,
        }
    );
}

#[test]
fn dir_comment_for_quoted_dir_with_spaces() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("\"my docs/\" # user docs");
    build(&nodes, &opts(out.path(), false, true)).unwrap();

    assert_eq!(read(out.path().join("my docs/COMMENT.md")), "user docs\n");
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

    assert_eq!(read(out.path().join("proj/main.rs")), "// entry\n");
    assert!(!out.path().join("proj/COMMENT.md").exists());
}

#[test]
fn dir_comments_only_does_not_write_file_comments() {
    let out = TempDir::new().unwrap();
    build(&parse_tree(MIXED_TREE), &opts(out.path(), false, true)).unwrap();

    assert_eq!(read(out.path().join("proj/COMMENT.md")), "the project\n");
    assert_eq!(read(out.path().join("proj/main.rs")), "");
}

#[test]
fn both_options_write_both_kinds_of_comment() {
    let out = TempDir::new().unwrap();
    build(&parse_tree(MIXED_TREE), &opts(out.path(), true, true)).unwrap();

    assert_eq!(read(out.path().join("proj/COMMENT.md")), "the project\n");
    assert_eq!(read(out.path().join("proj/main.rs")), "// entry\n");
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

    assert_eq!(read(&target), "// entry\n");
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

// ---------------------------------------------------------------------------
// Allowlist: only known comment syntaxes are written
// ---------------------------------------------------------------------------

#[test]
fn unknown_extensions_are_created_empty() {
    let out = TempDir::new().unwrap();
    let names = [
        "notes.txt",
        "a.weird",
        "data.json",
        "style.css",
        "settings.env",
        "logo.gif",
        "favicon.ico",
        "manual.pdf",
        "font.ttf",
        "app.jar",
        "Makefile",
    ];
    let tree: String = names.iter().map(|n| format!("{n} # a note\n")).collect();

    let stats = build(&parse_tree(&tree), &opts(out.path(), true, true)).unwrap();

    assert_eq!(stats.files_created, names.len());
    for name in names {
        assert_eq!(
            fs::metadata(out.path().join(name)).unwrap().len(),
            0,
            "{name} must stay empty: its comment syntax is not known"
        );
    }
}

#[test]
fn json_is_never_given_a_comment() {
    let out = TempDir::new().unwrap();
    build(
        &parse_tree("data.json # fixtures"),
        &opts(out.path(), true, false),
    )
    .unwrap();

    let content = read(out.path().join("data.json"));
    assert!(
        !content.contains("//") && !content.contains('#'),
        "a comment would make the file invalid JSON: {content:?}"
    );
    assert_eq!(content, "");
}

#[test]
fn ini_uses_semicolon_comments() {
    let out = TempDir::new().unwrap();
    build(
        &parse_tree("settings.ini # defaults"),
        &opts(out.path(), true, false),
    )
    .unwrap();

    assert_eq!(read(out.path().join("settings.ini")), "; defaults\n");
}

#[test]
fn php_comment_is_placed_inside_php_tags() {
    let out = TempDir::new().unwrap();
    build(
        &parse_tree("index.php # front controller"),
        &opts(out.path(), true, false),
    )
    .unwrap();

    assert_eq!(
        read(out.path().join("index.php")),
        "<?php\n// front controller\n"
    );
}

// ---------------------------------------------------------------------------
// Unsafe comment content
// ---------------------------------------------------------------------------

#[test]
fn html_comment_cannot_be_closed_by_its_own_text() {
    let out = TempDir::new().unwrap();
    build(
        &parse_tree("page.html # a --> <script>alert(1)</script>"),
        &opts(out.path(), true, false),
    )
    .unwrap();

    let content = read(out.path().join("page.html"));
    let inner = content
        .strip_prefix("<!-- ")
        .and_then(|s| s.strip_suffix(" -->\n"))
        .expect("still a single well-formed comment");
    assert!(!inner.contains("--"), "{content:?}");
}

#[test]
fn comment_with_a_line_break_stays_on_one_line() {
    // A parsed tree cannot produce this, but library callers can build nodes
    // by hand; a line break in a `.py` comment would be code injection.
    use tree2dir::parser::tree_node::{EntryKind, TreeNode};

    let out = TempDir::new().unwrap();
    let node = TreeNode::new(0, "run.py", EntryKind::File)
        .with_comment(Some("setup\nimport os; os.system('rm -rf /')".to_string()));
    build(&[node], &opts(out.path(), true, false)).unwrap();

    let content = read(out.path().join("run.py"));
    assert_eq!(content, "# setup import os; os.system('rm -rf /')\n");
    assert_eq!(content.lines().count(), 1);
}

#[test]
fn directory_comment_with_a_line_break_stays_on_one_line() {
    use tree2dir::parser::tree_node::{EntryKind, TreeNode};

    let out = TempDir::new().unwrap();
    let node = TreeNode::new(0, "src", EntryKind::Dir)
        .with_comment(Some("line one\nline two".to_string()));
    build(&[node], &opts(out.path(), false, true)).unwrap();

    assert_eq!(
        read(out.path().join("src/COMMENT.md")),
        "line one line two\n"
    );
}

// ---------------------------------------------------------------------------
// COMMENT.md conflicts
// ---------------------------------------------------------------------------

fn forced(root: &Path, file_comments: bool, dir_comments: bool) -> BuildOptions {
    BuildOptions {
        force: true,
        ..opts(root, file_comments, dir_comments)
    }
}

#[test]
fn existing_comment_md_is_not_overwritten_without_force() {
    let out = TempDir::new().unwrap();
    fs::create_dir(out.path().join("src")).unwrap();
    fs::write(out.path().join("src/COMMENT.md"), "hand written\n").unwrap();

    let stats = build(
        &parse_tree("src/ # generated"),
        &opts(out.path(), false, true),
    )
    .unwrap();

    assert_eq!(read(out.path().join("src/COMMENT.md")), "hand written\n");
    assert_eq!(stats.comments_written, 0);
    assert_eq!(stats.comments_skipped, 1);
    assert_eq!(stats.dirs_created, 0, "the directory already existed");
}

#[test]
fn existing_comment_md_is_overwritten_with_force() {
    let out = TempDir::new().unwrap();
    fs::create_dir(out.path().join("src")).unwrap();
    fs::write(out.path().join("src/COMMENT.md"), "hand written\n").unwrap();

    let stats = build(
        &parse_tree("src/ # generated"),
        &forced(out.path(), false, true),
    )
    .unwrap();

    assert_eq!(read(out.path().join("src/COMMENT.md")), "generated\n");
    assert_eq!(stats.comments_written, 1);
    assert_eq!(stats.comments_skipped, 0);
}

#[test]
fn comment_md_listed_in_the_tree_wins_over_the_directory_comment() {
    let out = TempDir::new().unwrap();
    let tree = "src/ # about src\n└── COMMENT.md # mine\n";

    let stats = build(&parse_tree(tree), &opts(out.path(), true, true)).unwrap();

    // The listed entry is created by the normal file rules, with its own
    // comment, and is not reported as skipped.
    assert_eq!(read(out.path().join("src/COMMENT.md")), "<!-- mine -->\n");
    assert_eq!(stats.files_created, 1);
    assert_eq!(stats.files_skipped, 0);
    assert_eq!(stats.comments_skipped, 1);
}

#[test]
fn listed_comment_md_without_a_comment_of_its_own_stays_empty() {
    // The listed entry is still the one that decides the file's content.
    let out = TempDir::new().unwrap();
    let tree = "docs/ # about docs\n├── COMMENT.md\n└── a.md\n";

    let stats = build(&parse_tree(tree), &opts(out.path(), false, true)).unwrap();

    assert_eq!(read(out.path().join("docs/COMMENT.md")), "");
    assert_eq!(stats.comments_skipped, 1);
    assert_eq!(stats.comments_written, 0);
}

#[test]
fn listed_comment_md_wins_even_with_force() {
    let out = TempDir::new().unwrap();
    let tree = "docs/ # about docs\n└── COMMENT.md\n";

    build(&parse_tree(tree), &forced(out.path(), false, true)).unwrap();

    assert_eq!(read(out.path().join("docs/COMMENT.md")), "");
}

#[test]
fn second_run_with_comments_changes_nothing() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree(MIXED_TREE);
    let options = opts(out.path(), true, true);

    let first = build(&nodes, &options).unwrap();
    assert_eq!(first.comments_written, 2);

    let second = build(&nodes, &options).unwrap();

    assert_eq!(
        second,
        BuildStats {
            dirs_created: 0,
            files_created: 0,
            files_skipped: 1,
            comments_written: 0,
            comments_skipped: 1,
        }
    );
    assert_eq!(read(out.path().join("proj/main.rs")), "// entry\n");
    assert_eq!(read(out.path().join("proj/COMMENT.md")), "the project\n");
}

#[test]
fn directory_comment_is_added_to_a_directory_that_already_exists() {
    let out = TempDir::new().unwrap();
    fs::create_dir(out.path().join("src")).unwrap();

    let stats = build(
        &parse_tree("src/ # late note"),
        &opts(out.path(), false, true),
    )
    .unwrap();

    assert_eq!(read(out.path().join("src/COMMENT.md")), "late note\n");
    assert_eq!(stats.dirs_created, 0);
    assert_eq!(stats.comments_written, 1);
}

// ---------------------------------------------------------------------------
// Dry run reports planned comments
// ---------------------------------------------------------------------------

#[test]
fn dry_run_counts_planned_comments_without_writing_them() {
    let out = TempDir::new().unwrap();
    let options = BuildOptions {
        dry_run: true,
        ..opts(out.path(), true, true)
    };

    let stats = build(&parse_tree(MIXED_TREE), &options).unwrap();

    assert_eq!(stats.comments_written, 2, "would write two comments");
    assert_eq!(fs::read_dir(out.path()).unwrap().count(), 0);
}

#[test]
fn listed_comment_md_directory_wins_over_the_directory_comment() {
    // A directory named COMMENT.md is just as much "listed in the tree" as a
    // file: writing the generated file first made the later create_dir_all fail
    // and aborted the build half way.
    let out = TempDir::new().unwrap();
    let tree = "src/ # about src\n└── COMMENT.md/\n";

    let stats = build(&parse_tree(tree), &opts(out.path(), false, true)).unwrap();

    assert!(out.path().join("src/COMMENT.md").is_dir());
    assert_eq!(stats.dirs_created, 2);
    assert_eq!(stats.comments_written, 0);
    assert_eq!(stats.comments_skipped, 1);
}
