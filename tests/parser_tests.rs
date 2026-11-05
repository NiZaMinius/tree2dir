//! Parser unit tests.
//!
//! These tests exercise [`tree2dir::parser::parse`] in isolation, covering
//! the edge cases listed in the technical specification:
//!
//! - Tab / space mixed indentation
//! - Trees that lack trailing slashes on directory names
//! - Deeply nested folder structures
//! - Comment and blank-line filtering
//! - Unicode scalar depth calculation correctness
//!
//! `serial_test` is used on tests that share global state (none currently)
//! or that must not run concurrently to avoid flaky failures.

use tree2dir::parser::{parse_tree, EntryKind};

// ---------------------------------------------------------------------------
// Basic parsing
// ---------------------------------------------------------------------------

#[test]
fn parses_simple_tree() {
    let input = "\
my-app/
├── src/
│   └── main.rs
└── README.md
";
    let nodes = parse_tree(input);
    assert_eq!(nodes.len(), 4);
    assert_eq!(nodes[0].kind, EntryKind::Dir); // my-app
    assert_eq!(nodes[1].kind, EntryKind::Dir); // src
    assert_eq!(nodes[2].kind, EntryKind::File); // main.rs
    assert_eq!(nodes[3].kind, EntryKind::File); // README.md
}

// ---------------------------------------------------------------------------
// Comment and blank-line filtering
// ---------------------------------------------------------------------------

#[test]
fn skips_blank_lines() {
    let input = "\
project/

└── main.rs
";
    let nodes = parse_tree(input);
    assert_eq!(nodes.len(), 2);
}

#[test]
fn skips_comment_lines() {
    let input = "\
project/
# This is a comment
└── main.rs
";
    let nodes = parse_tree(input);
    assert_eq!(nodes.len(), 2);
}

// ---------------------------------------------------------------------------
// Directory heuristics
// ---------------------------------------------------------------------------

#[test]
fn trailing_slash_forces_dir() {
    let nodes = parse_tree("standalone_dir/");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].kind, EntryKind::Dir);
    // The trailing slash must be stripped from the stored name.
    assert_eq!(nodes[0].name, "standalone_dir");
}

#[test]
fn lookahead_infers_dir_without_slash() {
    // "src" has no trailing slash, but the next line is deeper, so it must be
    // classified as a directory.
    let input = "\
src
    main.rs
";
    let nodes = parse_tree(input);
    assert_eq!(nodes[0].kind, EntryKind::Dir);
    assert_eq!(nodes[1].kind, EntryKind::File);
}

#[test]
fn leaf_without_slash_is_file() {
    let nodes = parse_tree("Cargo.toml");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].kind, EntryKind::File);
}

// ---------------------------------------------------------------------------
// Deeply nested structures
// ---------------------------------------------------------------------------

#[test]
fn deeply_nested_structure() {
    let input = "\
a/
├── b/
│   └── c/
│       └── d/
│           └── deep.txt
";
    let nodes = parse_tree(input);
    assert_eq!(nodes.len(), 5);
    assert_eq!(nodes[4].kind, EntryKind::File);
}

// ---------------------------------------------------------------------------
// Unicode scalar depth
// ---------------------------------------------------------------------------

#[test]
fn unicode_box_chars_counted_as_scalars() {
    // "│   " (1 box char + 3 spaces) must yield depth 4, not 6 (byte count).
    use tree2dir::parser::parse::split_prefix;
    let (depth, _) = split_prefix("│   file.txt").unwrap();
    assert_eq!(depth, 4);
}

// ---------------------------------------------------------------------------
// Tab indentation
// ---------------------------------------------------------------------------

#[test]
fn tab_indent_treated_as_single_scalar() {
    use tree2dir::parser::parse::split_prefix;
    // A single tab should produce depth 1.
    let (depth, name) = split_prefix("\tfile.txt").unwrap();
    assert_eq!(depth, 1);
    assert_eq!(name, "file.txt");
}

#[test]
fn mixed_tab_space_indentation() {
    // Each tab and space counts as one scalar, so two characters = depth 2.
    let input = "parent\n\t sub.txt";
    let nodes = parse_tree(input);
    // "parent" should be a Dir (lookahead) and "sub.txt" a File.
    assert_eq!(nodes[0].kind, EntryKind::Dir);
    assert_eq!(nodes[1].kind, EntryKind::File);
}

use tree2dir::parser::parse::parse_dir_and_comment;
use tree2dir::parser::tree_node::TreeNode;

/// Shorthand for `Some(String)` in assertions.
fn some(s: &str) -> Option<String> {
    Some(s.to_string())
}

// ---------------------------------------------------------------------------
// Basic extraction
// ---------------------------------------------------------------------------

#[test]
fn comment_after_file_is_captured() {
    let nodes = parse_tree("main.rs # entry point");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].name, "main.rs");
    assert_eq!(nodes[0].kind, EntryKind::File);
    assert_eq!(nodes[0].comment, some("entry point"));
}

#[test]
fn comment_after_dir_with_trailing_slash_is_captured() {
    let nodes = parse_tree("src/ # source code");
    assert_eq!(nodes.len(), 1);
    assert_eq!(
        nodes[0].name, "src",
        "trailing slash must still be stripped"
    );
    assert_eq!(nodes[0].kind, EntryKind::Dir);
    assert_eq!(nodes[0].comment, some("source code"));
}

#[test]
fn comment_after_dir_without_slash_still_infers_dir_by_lookahead() {
    let input = "src # source code\n    main.rs # entry point";
    let nodes = parse_tree(input);
    assert_eq!(nodes.len(), 2);

    assert_eq!(nodes[0].name, "src");
    assert_eq!(nodes[0].kind, EntryKind::Dir);
    assert_eq!(nodes[0].comment, some("source code"));

    assert_eq!(nodes[1].name, "main.rs");
    assert_eq!(nodes[1].kind, EntryKind::File);
    assert_eq!(nodes[1].comment, some("entry point"));
}

#[test]
fn comment_on_leaf_without_slash_is_still_a_file() {
    let nodes = parse_tree("README.md # docs");
    assert_eq!(nodes[0].kind, EntryKind::File);
    assert_eq!(nodes[0].name, "README.md");
}

#[test]
fn entries_without_comment_have_none() {
    let nodes = parse_tree("src/\n└── main.rs");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].comment, None);
    assert_eq!(nodes[1].comment, None);
}

#[test]
fn only_some_entries_have_comments() {
    let input = "\
root/ # top
├── a.txt
└── b.txt # last
";
    let nodes = parse_tree(input);
    assert_eq!(nodes[0].comment, some("top"));
    assert_eq!(nodes[1].comment, None);
    assert_eq!(nodes[2].comment, some("last"));
}

// ---------------------------------------------------------------------------
// Whitespace, empty comments and multiple `#`
// ---------------------------------------------------------------------------

#[test]
fn comment_and_name_are_trimmed() {
    let nodes = parse_tree("a.txt      #      spaced out     ");
    assert_eq!(nodes[0].name, "a.txt");
    assert_eq!(nodes[0].comment, some("spaced out"));
}

#[test]
fn empty_comment_becomes_none() {
    let nodes = parse_tree("a.txt #");
    assert_eq!(nodes[0].name, "a.txt");
    assert_eq!(nodes[0].comment, None);
}

#[test]
fn whitespace_only_comment_becomes_none() {
    let nodes = parse_tree("a.txt #    ");
    assert_eq!(nodes[0].name, "a.txt");
    assert_eq!(nodes[0].comment, None);
}

#[test]
fn hash_without_preceding_space_is_part_of_the_name() {
    let nodes = parse_tree("notes#1.txt");
    assert_eq!(nodes[0].name, "notes#1.txt");
    assert_eq!(nodes[0].comment, None);
}

#[test]
fn only_first_space_hash_splits_name_from_comment() {
    let nodes = parse_tree("a.txt # see issue #42");
    assert_eq!(nodes[0].name, "a.txt");
    assert_eq!(nodes[0].comment, some("see issue #42"));
}

#[test]
fn comment_may_contain_quotes() {
    let nodes = parse_tree(r#"a.txt # it's "fine""#);
    assert_eq!(nodes[0].name, "a.txt");
    assert_eq!(nodes[0].comment, some(r#"it's "fine""#));
}

// ---------------------------------------------------------------------------
// Quoted names
// ---------------------------------------------------------------------------

#[test]
fn double_quoted_name_with_spaces_and_comment() {
    let nodes = parse_tree(r#""my file.txt" # has spaces"#);
    assert_eq!(nodes[0].name, "my file.txt");
    assert_eq!(nodes[0].kind, EntryKind::File);
    assert_eq!(nodes[0].comment, some("has spaces"));
}

#[test]
fn single_quoted_name_with_spaces_and_comment() {
    let nodes = parse_tree("'my file.txt' # has spaces");
    assert_eq!(nodes[0].name, "my file.txt");
    assert_eq!(nodes[0].comment, some("has spaces"));
}

#[test]
fn quoted_name_without_comment() {
    let nodes = parse_tree(r#""my file.txt""#);
    assert_eq!(nodes[0].name, "my file.txt");
    assert_eq!(nodes[0].comment, None);
}

#[test]
fn quoted_name_with_empty_comment_becomes_none() {
    let nodes = parse_tree(r#""my file.txt" #"#);
    assert_eq!(nodes[0].name, "my file.txt");
    assert_eq!(nodes[0].comment, None);
}

#[test]
fn hash_inside_quotes_is_not_a_comment_marker() {
    let nodes = parse_tree(r#""a # b.txt""#);
    assert_eq!(nodes[0].name, "a # b.txt");
    assert_eq!(nodes[0].comment, None);
}

#[test]
fn hash_inside_quotes_and_real_comment_after() {
    let nodes = parse_tree(r#""a # b.txt" # real comment"#);
    assert_eq!(nodes[0].name, "a # b.txt");
    assert_eq!(nodes[0].comment, some("real comment"));
}

#[test]
fn quoted_dir_with_trailing_slash_inside_quotes_is_dir() {
    let nodes = parse_tree(r#""my dir/" # folder"#);
    assert_eq!(nodes[0].name, "my dir");
    assert_eq!(nodes[0].kind, EntryKind::Dir);
    assert_eq!(nodes[0].comment, some("folder"));
}

#[test]
fn quoted_name_after_tree_drawing_prefix() {
    let input = "\
root/
├── \"my file.txt\" # spaced
└── 'other.md' # single
";
    let nodes = parse_tree(input);
    assert_eq!(nodes.len(), 3);
    assert_eq!(nodes[1].name, "my file.txt");
    assert_eq!(nodes[1].comment, some("spaced"));
    assert_eq!(nodes[2].name, "other.md");
    assert_eq!(nodes[2].comment, some("single"));
}

// ---------------------------------------------------------------------------
// `parse_dir_and_comment` directly
// ---------------------------------------------------------------------------

#[test]
fn parse_dir_and_comment_unquoted() {
    assert_eq!(
        parse_dir_and_comment("a.txt # c"),
        ("a.txt".to_string(), some("c"))
    );
}

#[test]
fn parse_dir_and_comment_without_comment() {
    assert_eq!(parse_dir_and_comment("a.txt"), ("a.txt".to_string(), None));
}

#[test]
fn parse_dir_and_comment_quoted() {
    assert_eq!(
        parse_dir_and_comment("\"a b\" # c"),
        ("a b".to_string(), some("c"))
    );
}

// ---------------------------------------------------------------------------
// Interaction with tree structure
// ---------------------------------------------------------------------------

#[test]
fn comments_in_a_realistic_box_drawing_tree() {
    let input = "\
my-app/          # project root
├── src/         # source code
│   ├── main.rs  # entry point
│   └── lib.rs
└── README.md    # docs
";
    let nodes = parse_tree(input);

    let summary: Vec<(&str, EntryKind, Option<&str>)> = nodes
        .iter()
        .map(|n| (n.name.as_str(), n.kind.clone(), n.comment.as_deref()))
        .collect();

    assert_eq!(
        summary,
        vec![
            ("my-app", EntryKind::Dir, Some("project root")),
            ("src", EntryKind::Dir, Some("source code")),
            ("main.rs", EntryKind::File, Some("entry point")),
            ("lib.rs", EntryKind::File, None),
            ("README.md", EntryKind::File, Some("docs")),
        ]
    );
}

#[test]
fn comments_do_not_affect_depth_or_kind() {
    let plain = "\
root/
├── src/
│   └── main.rs
└── README.md
";
    let commented = "\
root/ # a
├── src/ # b
│   └── main.rs # c
└── README.md # d
";
    let a = parse_tree(plain);
    let b = parse_tree(commented);

    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.depth, y.depth);
        assert_eq!(x.name, y.name);
        assert_eq!(x.kind, y.kind);
    }
}

#[test]
fn full_line_comments_are_skipped_and_not_attached_to_neighbours() {
    let input = "\
# header comment
project/ # root
# standalone comment between entries
└── main.rs # entry
    # indented standalone comment
";
    let nodes = parse_tree(input);
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].name, "project");
    assert_eq!(nodes[0].comment, some("root"));
    assert_eq!(nodes[1].name, "main.rs");
    assert_eq!(nodes[1].comment, some("entry"));
}

#[test]
fn comment_on_a_line_with_blank_lines_around_it() {
    let nodes = parse_tree("\nroot/ # r\n\n└── a.txt # a\n\n");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].comment, some("r"));
    assert_eq!(nodes[1].comment, some("a"));
}

// ---------------------------------------------------------------------------
// Unicode / line endings
// ---------------------------------------------------------------------------

#[test]
fn unicode_names_and_comments() {
    let nodes = parse_tree("файл.txt # 注释 ✓");
    assert_eq!(nodes[0].name, "файл.txt");
    assert_eq!(nodes[0].comment, some("注释 ✓"));
}

#[test]
fn unicode_quoted_name_with_comment() {
    let nodes = parse_tree("\"日本 語.txt\" # ✓ ok");
    assert_eq!(nodes[0].name, "日本 語.txt");
    assert_eq!(nodes[0].comment, some("✓ ok"));
}

#[test]
fn crlf_line_endings_do_not_leak_into_comments() {
    let nodes = parse_tree("root/ # r\r\n└── a.txt # c\r\n");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].comment, some("r"));
    assert_eq!(nodes[1].comment, some("c"));
}

// ---------------------------------------------------------------------------
// TreeNode construction
// ---------------------------------------------------------------------------

#[test]
fn tree_node_new_stores_comment() {
    let with = TreeNode::new(2, "a.txt", EntryKind::File, some("hi"));
    assert_eq!(with.depth, 2);
    assert_eq!(with.name, "a.txt");
    assert_eq!(with.kind, EntryKind::File);
    assert_eq!(with.comment, some("hi"));

    let without = TreeNode::new(0, "src", EntryKind::Dir, None);
    assert_eq!(without.comment, None);
}
