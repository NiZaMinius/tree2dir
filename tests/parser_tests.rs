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
fn tree_node_with_comment_stores_comment() {
    let with = TreeNode::new(2, "a.txt", EntryKind::File).with_comment(some("hi"));
    assert_eq!(with.depth, 2);
    assert_eq!(with.name, "a.txt");
    assert_eq!(with.kind, EntryKind::File);
    assert_eq!(with.comment, some("hi"));

    let without = TreeNode::new(0, "src", EntryKind::Dir);
    assert_eq!(without.comment, None);

    // `new` takes three arguments; the comment is opt-in.
    let cleared = with.with_comment(None);
    assert_eq!(cleared.comment, None);
}

// ---------------------------------------------------------------------------
// Whitespace before `#`
// ---------------------------------------------------------------------------

#[test]
fn tab_before_hash_starts_a_comment() {
    let nodes = parse_tree("main.rs\t# entry point");
    assert_eq!(nodes[0].name, "main.rs");
    assert_eq!(nodes[0].comment, some("entry point"));
}

#[test]
fn mixed_whitespace_before_hash_starts_a_comment() {
    let nodes = parse_tree("a.txt \t # c");
    assert_eq!(nodes[0].name, "a.txt");
    assert_eq!(nodes[0].comment, some("c"));
}

#[test]
fn hash_after_a_non_space_character_never_starts_a_comment() {
    for name in ["notes#1.txt", "C#/", "a#b#c", "dir#/"] {
        let nodes = parse_tree(name);
        assert_eq!(nodes[0].name, name.trim_end_matches('/'), "{name}");
        assert_eq!(nodes[0].comment, None, "{name}");
    }
}

// ---------------------------------------------------------------------------
// Comment-only lines, including ones behind tree-drawing glyphs
// ---------------------------------------------------------------------------

#[test]
fn comment_line_behind_pipe_glyph_is_not_an_entry() {
    let nodes = parse_tree("root/\n│   # helpers\n├── a.txt");
    let names: Vec<_> = nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["root", "a.txt"]);
}

#[test]
fn comment_line_behind_branch_glyph_is_not_an_entry() {
    let nodes = parse_tree("root/\n├── # note\n└── a.txt");
    let names: Vec<_> = nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["root", "a.txt"]);
}

#[test]
fn comment_line_without_a_space_after_hash_is_still_a_comment() {
    // Same rule as an unindented `#note` line, which has always been skipped.
    let nodes = parse_tree("root/\n├── #note\n└── a.txt");
    assert_eq!(nodes.len(), 2);
}

#[test]
fn comment_line_between_parent_and_child_does_not_hide_the_child() {
    // `src` has no slash; it is a directory only because the next *entry* is
    // deeper, and the comment line in between must not break that lookahead.
    let nodes = parse_tree("src\n    # about main\n    main.rs");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].kind, EntryKind::Dir);
    assert_eq!(nodes[1].kind, EntryKind::File);
}

#[test]
fn quoting_creates_an_entry_whose_name_starts_with_hash() {
    let nodes = parse_tree("\"#recycle\"/ # synology");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].name, "#recycle");
    assert_eq!(nodes[0].kind, EntryKind::Dir);
    assert_eq!(nodes[0].comment, some("synology"));
}

// ---------------------------------------------------------------------------
// Quote characters that are not a quoted name
// ---------------------------------------------------------------------------

#[test]
fn unmatched_leading_quote_is_part_of_the_name() {
    let nodes = parse_tree("'tis.txt");
    assert_eq!(nodes[0].name, "'tis.txt");
    assert_eq!(nodes[0].comment, None);
}

#[test]
fn leading_quote_with_a_later_quote_does_not_swallow_the_tail() {
    // Previously parsed as the name `tis it`, silently dropping `s.txt`.
    let nodes = parse_tree("'tis it's.txt");
    assert_eq!(nodes[0].name, "'tis it's.txt");
    assert_eq!(nodes[0].comment, None);
}

#[test]
fn text_glued_after_a_closing_quote_makes_the_whole_line_a_plain_name() {
    let nodes = parse_tree("\"a\"b.txt");
    assert_eq!(nodes[0].name, "\"a\"b.txt");
}

#[test]
fn quote_inside_an_unquoted_name_is_literal() {
    let nodes = parse_tree("it's.txt # fine");
    assert_eq!(nodes[0].name, "it's.txt");
    assert_eq!(nodes[0].comment, some("fine"));
}

#[test]
fn other_quote_kind_inside_quotes_is_literal() {
    let nodes = parse_tree("\"it's here.txt\" # c");
    assert_eq!(nodes[0].name, "it's here.txt");
    assert_eq!(nodes[0].comment, some("c"));
}

#[test]
fn empty_quotes_are_not_a_quoted_name() {
    // An empty name would resolve to the parent directory itself.
    let nodes = parse_tree("\"\" # c");
    assert_eq!(nodes[0].name, "\"\"");
    assert_eq!(nodes[0].comment, some("c"));
}

// ---------------------------------------------------------------------------
// Trailing slash after the closing quote
// ---------------------------------------------------------------------------

#[test]
fn slash_after_closing_quote_marks_a_directory() {
    let nodes = parse_tree("\"my dir\"/ # c");
    assert_eq!(nodes[0].name, "my dir");
    assert_eq!(nodes[0].kind, EntryKind::Dir);
    assert_eq!(nodes[0].comment, some("c"));
}

#[test]
fn slash_after_closing_quote_without_comment() {
    let nodes = parse_tree("'my dir'/");
    assert_eq!(nodes[0].name, "my dir");
    assert_eq!(nodes[0].kind, EntryKind::Dir);
    assert_eq!(nodes[0].comment, None);
}

#[test]
fn comment_may_follow_the_closing_quote_directly() {
    let nodes = parse_tree("\"a b\"# c");
    assert_eq!(nodes[0].name, "a b");
    assert_eq!(nodes[0].comment, some("c"));
}

// ---------------------------------------------------------------------------
// Comments never influence depth or classification
// ---------------------------------------------------------------------------

#[test]
fn comments_and_comment_lines_leave_depth_and_kind_unchanged() {
    let plain = parse_tree("root/\n├── src/\n│   └── main.rs\n└── README.md\n");
    let noisy = parse_tree(
        "root/ # r\n# banner\n├── src/ # s\n│   # why\n│   └── main.rs # m\n└── README.md\t# d\n",
    );
    let shape = |nodes: &[TreeNode]| {
        nodes
            .iter()
            .map(|n| (n.depth, n.name.clone(), n.kind.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&plain), shape(&noisy));
}

// ---------------------------------------------------------------------------
// Line endings
// ---------------------------------------------------------------------------

#[test]
fn every_line_ending_style_parses_identically() {
    let lf = "root/\n├── a.rs # one\n└── b.rs\n";
    let expected = parse_tree(lf);
    assert_eq!(expected.len(), 3);

    for (label, text) in [
        ("CRLF", lf.replace('\n', "\r\n")),
        ("CR", lf.replace('\n', "\r")),
        ("mixed", "root/\r\n├── a.rs # one\r└── b.rs\n".to_string()),
    ] {
        assert_eq!(parse_tree(&text), expected, "{label}");
    }
}

#[test]
fn carriage_return_never_ends_up_in_names_or_comments() {
    for node in parse_tree("a.rs # note\r\nb.rs\r\n") {
        assert!(!node.name.contains('\r'), "{:?}", node.name);
        assert!(!node.comment.unwrap_or_default().contains('\r'));
    }
}
