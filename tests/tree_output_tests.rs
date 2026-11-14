//! Output of real `tree` implementations, pasted as is.
//!
//! - GNU `tree` (Unicode, with the non-breaking spaces it prints, and
//!   `--charset=ascii`), including its summary line;
//! - Windows `tree /f` (Unicode) and `tree /f /a` (ASCII), including the volume
//!   header and the drive line, whose wording depends on the Windows language.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;
use tree2dir::parser::{parse_tree, tree_node::EntryKind, tree_node::TreeNode};

use EntryKind::{Dir, File};

/// `(depth, name, kind)` of every node, for readable assertions.
fn shape(input: &str) -> Vec<(usize, String, EntryKind)> {
    parse_tree(input)
        .into_iter()
        .map(|n: TreeNode| (n.depth, n.name, n.kind))
        .collect()
}

fn expect(rows: &[(usize, &str, EntryKind)]) -> Vec<(usize, String, EntryKind)> {
    rows.iter()
        .map(|(d, n, k)| (*d, (*n).to_string(), k.clone()))
        .collect()
}

const WINDOWS_ASCII: &str = "\
Folder PATH listing for volume Data
Volume serial number is 1234-5678
D:.
|   .gitignore
|   Cargo.toml
|   
+---src
|   |   main.rs
|   |   
|   \\---util
|           mod.rs
|           
\\---tests
        cli.rs
        
";

/// What the two Windows variants must produce, header and drive line removed.
fn windows_structure() -> Vec<(usize, String, EntryKind)> {
    expect(&[
        (4, ".gitignore", File),
        (4, "Cargo.toml", File),
        (4, "src", Dir),
        (8, "main.rs", File),
        (8, "util", Dir),
        (12, "mod.rs", File),
        (4, "tests", Dir),
        (8, "cli.rs", File),
    ])
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[test]
fn windows_tree_f_a_with_english_header() {
    assert_eq!(shape(WINDOWS_ASCII), windows_structure());
}

#[test]
fn windows_header_in_another_language_is_dropped_too() {
    let input = WINDOWS_ASCII
        .replace(
            "Folder PATH listing for volume Data",
            "Структура папок тома Data",
        )
        .replace(
            "Volume serial number is 1234-5678",
            "Серийный номер тома: 1234-5678",
        );
    assert_eq!(shape(&input), windows_structure());
}

#[test]
fn windows_tree_f_with_unicode_glyphs_and_header() {
    let input = "\
Folder PATH listing for volume Data
Volume serial number is 1234-5678
D:.
│   .gitignore
│   Cargo.toml
│   
├───src
│   │   main.rs
│   │   
│   └───util
│           mod.rs
│           
└───tests
        cli.rs
        
";
    assert_eq!(shape(input), windows_structure());
}

#[test]
fn windows_drive_line_with_a_path_names_the_root_directory() {
    let input = "C:\\Users\\me\\proj\n+---src\n|       a.rs\n\\---docs\n";
    assert_eq!(
        shape(input),
        expect(&[
            (0, "proj", Dir),
            (4, "src", Dir),
            (8, "a.rs", File),
            (4, "docs", File),
        ])
    );
}

#[test]
fn windows_tree_without_a_header_still_parses() {
    // Files of `src` are printed under its own guide: `|` plus seven spaces.
    let input = "+---src\n|       a.rs\n\\---docs\n";
    assert_eq!(
        shape(input),
        expect(&[(4, "src", Dir), (8, "a.rs", File), (4, "docs", File)])
    );
}

// ---------------------------------------------------------------------------
// GNU tree
// ---------------------------------------------------------------------------

const GNU_ASCII: &str = "\
.
|-- Cargo.toml
|-- src
|   |-- lib.rs
|   `-- main.rs
`-- tests
    `-- cli.rs

2 directories, 4 files
";

fn gnu_structure() -> Vec<(usize, String, EntryKind)> {
    expect(&[
        (0, ".", Dir),
        (4, "Cargo.toml", File),
        (4, "src", Dir),
        (8, "lib.rs", File),
        (8, "main.rs", File),
        (4, "tests", Dir),
        (8, "cli.rs", File),
    ])
}

#[test]
fn gnu_tree_charset_ascii_with_summary_line() {
    assert_eq!(shape(GNU_ASCII), gnu_structure());
}

#[test]
fn gnu_tree_unicode_with_non_breaking_spaces() {
    // GNU tree prints `│` followed by two U+00A0 and a space.
    let input = GNU_ASCII
        .replace("|-- ", "├── ")
        .replace("`-- ", "└── ")
        .replace("|   ", "│\u{a0}\u{a0} ");
    assert_eq!(shape(&input), gnu_structure());
}

#[test]
fn gnu_summary_line_in_other_languages_and_singular_forms() {
    for summary in [
        "1 directory, 1 file",
        "2 каталога, 3 файла",
        "10 Verzeichnisse, 20 Dateien",
    ] {
        let input = format!("src\n    a.rs\n\n{summary}\n");
        assert_eq!(
            shape(&input),
            expect(&[(0, "src", Dir), (4, "a.rs", File)]),
            "{summary}"
        );
    }
}

// ---------------------------------------------------------------------------
// The same trees without the header and summary lines
// ---------------------------------------------------------------------------

#[test]
fn windows_ascii_body_without_header_or_drive_line() {
    let body: String = WINDOWS_ASCII
        .lines()
        .skip(3)
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(shape(&body), windows_structure());
}

#[test]
fn gnu_tree_charset_ascii_without_summary() {
    let body = GNU_ASCII.split("\n\n").next().unwrap();
    assert_eq!(shape(body), gnu_structure());
}

#[test]
fn gnu_tree_unicode_with_non_breaking_spaces_without_summary() {
    let body = GNU_ASCII
        .split("\n\n")
        .next()
        .unwrap()
        .replace("|-- ", "├── ")
        .replace("`-- ", "└── ")
        .replace("|   ", "│\u{a0}\u{a0} ");
    assert_eq!(shape(&body), gnu_structure());
}

// ---------------------------------------------------------------------------
// Things that must stay names
// ---------------------------------------------------------------------------

#[test]
fn names_that_look_like_drawing_characters_stay_names() {
    let input = "\
+layout.svelte
-flag
|pipe
\\backslash
a---b
|-x
";
    let names: Vec<_> = shape(input).into_iter().map(|(_, n, _)| n).collect();
    assert_eq!(
        names,
        [
            "+layout.svelte",
            "-flag",
            "|pipe",
            "\\backslash",
            "a---b",
            "|-x"
        ]
    );
}

#[test]
fn a_last_line_with_one_count_is_a_real_entry_not_a_summary() {
    let nodes = shape("a.rs\n\n3 notes\n");
    assert_eq!(nodes.last().unwrap().1, "3 notes");
}

#[test]
fn a_summary_shaped_line_without_a_blank_line_before_it_is_kept() {
    let nodes = shape("a.rs\n2 directories, 3 files\n");
    assert_eq!(nodes.len(), 2);
}

#[test]
fn a_drive_like_first_line_far_from_the_top_is_not_a_header() {
    let input = "a.rs\nb.rs\nc.rs\nd.rs\ne.rs\nD:.\n";
    assert_eq!(shape(input).len(), 6);
}

#[test]
fn existing_unicode_and_space_trees_parse_as_before() {
    let input = "my-app/\n├── src/\n│   └── main.rs\n└── README.md\n";
    assert_eq!(
        shape(input),
        expect(&[
            (0, "my-app", Dir),
            (4, "src", Dir),
            (8, "main.rs", File),
            (4, "README.md", File),
        ])
    );
}

// ---------------------------------------------------------------------------
// End to end
// ---------------------------------------------------------------------------

fn build(input: &str) -> TempDir {
    let out = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let tree = src.path().join("tree.txt");
    fs::write(&tree, input).unwrap();
    Command::cargo_bin("tree2dir")
        .unwrap()
        .args(["--file", tree.to_str().unwrap()])
        .args(["--output", out.path().to_str().unwrap()])
        .assert()
        .success();
    out
}

fn listing(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                out.push(format!("{rel}/"));
                walk(&path, root, out);
            } else {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn windows_ascii_tree_builds_the_real_structure() {
    let out = build(WINDOWS_ASCII);
    assert_eq!(
        listing(out.path()),
        [
            ".gitignore",
            "Cargo.toml",
            "src/",
            "src/main.rs",
            "src/util/",
            "src/util/mod.rs",
            "tests/",
            "tests/cli.rs",
        ]
    );
}

#[test]
fn gnu_ascii_tree_builds_the_real_structure_without_the_summary_line() {
    let out = build(GNU_ASCII);
    assert_eq!(
        listing(out.path()),
        [
            "Cargo.toml",
            "src/",
            "src/lib.rs",
            "src/main.rs",
            "tests/",
            "tests/cli.rs",
        ]
    );
}

#[test]
fn windows_ascii_body_builds_the_real_structure() {
    let body: String = WINDOWS_ASCII
        .lines()
        .skip(3)
        .map(|l| format!("{l}\n"))
        .collect();
    let out = build(&body);
    assert_eq!(
        listing(out.path()),
        [
            ".gitignore",
            "Cargo.toml",
            "src/",
            "src/main.rs",
            "src/util/",
            "src/util/mod.rs",
            "tests/",
            "tests/cli.rs",
        ]
    );
}

#[test]
fn gnu_ascii_body_builds_the_real_structure() {
    let body = GNU_ASCII.split("\n\n").next().unwrap();
    let out = build(body);
    assert_eq!(
        listing(out.path()),
        [
            "Cargo.toml",
            "src/",
            "src/lib.rs",
            "src/main.rs",
            "tests/",
            "tests/cli.rs"
        ]
    );
}
