//! Core ASCII / Unicode tree parsing algorithm.
//!
//! # Depth calculation
//!
//! Tree generators produce indentation using a mix of box-drawing characters
//! (`│`, `├`, `└`, `─`) and plain spaces.  Because `│` is **3 bytes** in
//! UTF-8 while a space is **1 byte**, byte-length comparisons produce
//! inconsistent depth values.  This module counts **Unicode scalar values**
//! (`.chars().count()`) instead, which yields a stable result across locales
//! and terminal configurations.
//!
//! # Heuristic for directories vs. files
//!
//! Raw `tree` output often omits trailing slashes on directories.  The parser
//! resolves the ambiguity with a single-line lookahead:
//!
//! 1. Name ends with `/` → **Directory**.
//! 2. The immediately following non-blank, non-comment line has a strictly
//!    greater depth → **Directory**.
//! 3. Otherwise → **File**.

use crate::parser::tree_node::{EntryKind, TreeNode};

/// Characters that constitute tree indentation guide lines rather than entry
/// names.  Any prefix composed exclusively of these scalars is treated as
/// visual decoration and excluded from the entry name.
const INDENT_CHARS: &[char] = &[' ', '\t', '│', '├', '└', '─'];

/// Split a raw input line into a `(visual_depth, entry_name)` pair.
///
/// `visual_depth` is the **Unicode scalar count** of the indentation prefix.
/// `entry_name` is the remainder of the line with trailing whitespace removed.
///
/// Returns `None` if the line consists entirely of indentation characters
/// (i.e., the name would be empty).
pub fn split_prefix(line: &str) -> Option<(usize, &str)> {
    let boundary = line
        .char_indices()
        .find(|(_, c)| !INDENT_CHARS.contains(c))
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(line.len());

    let prefix = &line[..boundary];
    let name = line[boundary..].trim_end();

    if name.is_empty() {
        return None;
    }

    Some((prefix.chars().count(), name))
}

/// Parse an ASCII / Unicode directory tree string into a flat list of
/// [`TreeNode`] values.
///
/// Each returned node carries its indentation depth and kind, but nodes are
/// **not** yet converted into full filesystem paths — that translation is the
/// responsibility of [`crate::fs_builder`].
///
/// Lines that are blank or begin with `#` (after trimming) are silently
/// skipped.
///
/// If user wants to preserve comments (for files, directories, or both), then
/// lines that contain directory/file name AND '#' are splitted in two parts:
/// name and comment. Comment is either written to a file as a comment (if
/// possible; e.g. you can't add comments to .zip) or as a COMMENT.md to a directory.
pub fn parse_tree(input: &str) -> Vec<TreeNode> {
    // Collect only the lines we will actually process, retaining their
    // original text so the lookahead can inspect the *next* line's depth.
    let meaningful: Vec<&str> = input
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            // Reject lines that are entirely empty, comments, or purely decoration
            !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !trimmed.chars().all(|c| INDENT_CHARS.contains(&c))
        })
        .collect();

    let mut nodes: Vec<TreeNode> = Vec::with_capacity(meaningful.len());

    for (i, &line) in meaningful.iter().enumerate() {
        let Some((depth, raw_name)) = split_prefix(line) else {
            continue;
        };

        let (dir_name, comment) = parse_dir_and_comment(raw_name);

        // Determine whether this entry is a directory or a file.
        let (clean_name, kind) = classify_entry(&dir_name, i, &meaningful, depth);

        nodes.push(TreeNode::new(depth, clean_name, kind, comment));
    }

    nodes
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Apply the three-rule heuristic to determine the [`EntryKind`] of a raw
/// entry name.  Returns `(cleaned_name, kind)`.
fn classify_entry<'a>(
    raw_name: &'a str,
    index: usize,
    lines: &[&str],
    current_depth: usize,
) -> (&'a str, EntryKind) {
    // Rule 1 — explicit trailing slash.
    if raw_name.ends_with('/') {
        return (raw_name.trim_end_matches('/'), EntryKind::Dir);
    }

    // Rule 2 — lookahead: the next line sits one level deeper.
    if let Some(&next_line) = lines.get(index + 1) {
        if let Some((next_depth, _)) = split_prefix(next_line) {
            if next_depth > current_depth {
                return (raw_name, EntryKind::Dir);
            }
        }
    }

    // Rule 3 — default: treat as a regular file.
    (raw_name, EntryKind::File)
}

pub fn parse_dir_and_comment(line: &str) -> (String, Option<String>) {
    // Handle quoted paths (in case directory names contain spaces)
    if line.starts_with('"') || line.starts_with('\'') {
        let quote_char = line.chars().next().unwrap();
        if let Some(end_quote_idx) = line[1..].find(quote_char) {
            let actual_end = end_quote_idx + 1;
            let dir = line[1..actual_end].to_string();
            let remainder = line[actual_end + 1..].trim();

            let comment = if remainder.starts_with('#') {
                let c = remainder[1..].trim();
                if c.is_empty() {
                    None
                } else {
                    Some(c.to_string())
                }
            } else {
                None
            };
            return (dir, comment);
        }
    }

    // Unquoted paths (find ' #' - preceded by whitespace)
    if let Some((dir, comment)) = line.split_once(" #") {
        let dir = dir.trim().to_string();
        let comment = comment.trim();
        let comment = if comment.is_empty() {
            None
        } else {
            Some(comment.to_string())
        };
        (dir, comment)
    } else {
        (line.to_string(), None)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_prefix_plain_spaces() {
        let (depth, name) = split_prefix("    main.rs").unwrap();
        assert_eq!(depth, 4);
        assert_eq!(name, "main.rs");
    }

    #[test]
    fn split_prefix_box_drawing() {
        // "│   " prefix — 4 Unicode scalars despite being 6 bytes.
        let (depth, name) = split_prefix("│   src/").unwrap();
        assert_eq!(depth, 4);
        assert_eq!(name, "src/");
    }

    #[test]
    fn split_prefix_empty_line_returns_none() {
        assert!(split_prefix("   │   ").is_none());
    }

    #[test]
    fn classify_trailing_slash_is_dir() {
        let lines = &["src/"];
        let (name, kind) = classify_entry("src/", 0, lines, 0);
        assert_eq!(name, "src");
        assert_eq!(kind, EntryKind::Dir);
    }

    #[test]
    fn classify_lookahead_dir() {
        let input = "src\n    main.rs";
        let nodes = parse_tree(input);
        assert_eq!(nodes[0].kind, EntryKind::Dir);
        assert_eq!(nodes[1].kind, EntryKind::File);
    }

    #[test]
    fn classify_leaf_is_file() {
        let nodes = parse_tree("README.md");
        assert_eq!(nodes[0].kind, EntryKind::File);
    }

    #[test]
    fn box_drawing_char_and_equivalent_spaces_give_same_depth() {
        // "│   " (│ + 3 spaces = 4 scalars) vs "    " (4 spaces = 4 scalars)
        let input = "root/\n│   ├── a.txt\n    └── b.txt\n";
        let nodes = parse_tree(input);
        assert_eq!(nodes[1].depth, nodes[2].depth);
    }

    #[test]
    fn parse_tree_ignores_pure_decoration_lines() {
        let input = "root/\n   │   \n├── a.txt";
        let nodes = parse_tree(input);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].name, "root");
        assert_eq!(nodes[1].name, "a.txt");
    }

    #[test]
    fn parse_comments_after_path() {
        let input = "root/  # root comment\n   │   \n├── a.txt\n   │   \n├── b.txt  # txt comment";
        let nodes = parse_tree(input);
        assert_eq!(nodes[0].comment, Some("root comment".to_string()));
        assert_eq!(nodes[1].comment, None);
        assert_eq!(nodes[2].comment, Some("txt comment".to_string()));
    }
}
