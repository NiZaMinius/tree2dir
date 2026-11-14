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
//!
//! # Output of `tree` commands
//!
//! Pasted output of real tools is accepted as is:
//!
//! - GNU `tree`, Unicode or `--charset=ascii` (`|-- `, `` `-- ``), including the
//!   non-breaking spaces it prints inside `│` guides.
//! - Windows `tree` and `tree /a` (`+---`, `\---`, `|   `).
//!
//! The ASCII connectors are recognised only in these exact forms and only at
//! the start of a line, so names such as `+layout.svelte`, `-flag` or `|pipe`
//! stay names.  Text that is not part of the tree is dropped: the volume header
//! and drive line of Windows `tree` (whatever the Windows language), and the
//! summary line of GNU `tree` (`2 directories, 4 files`).  A drive line with a
//! path (`C:\Users\me\proj`) becomes the root directory `proj`; `D:.` is dropped,
//! so the tree is built in the output directory itself.
//!
//! # Comments
//!
//! - A line whose content starts with `#` (after the indentation and any
//!   tree-drawing glyphs) is a comment line and produces no entry.  To create
//!   an entry whose name starts with `#`, quote it: `"#recycle"`.
//! - Inside an unquoted entry, a `#` that follows whitespace (space or tab)
//!   starts an inline comment.  A `#` glued to the name (`notes#1.txt`,
//!   `C#/`) is part of the name.
//! - A name wrapped in `"…"` or `'…'` may contain spaces and `#`.  The
//!   closing quote must be followed by nothing, a `/`, or a `#` comment;
//!   otherwise the quote belongs to an ordinary name (`'tis.txt`).

use std::borrow::Cow;

use crate::parser::tree_node::{EntryKind, TreeNode};

/// Characters that constitute tree indentation guide lines rather than entry
/// names.  U+00A0 is there because GNU `tree` prints non-breaking spaces inside
/// its `│` guides.  Any prefix composed exclusively of these scalars is treated as
/// visual decoration and excluded from the entry name.
const INDENT_CHARS: &[char] = &[' ', '\t', '\u{a0}', '│', '├', '└', '─'];

/// Split a raw input line into a `(visual_depth, entry_name)` pair.
///
/// `visual_depth` is the **Unicode scalar count** of the indentation prefix.
/// `entry_name` is the remainder of the line with trailing whitespace removed.
///
/// Returns `None` if the line consists entirely of indentation characters
/// (i.e., the name would be empty).
pub fn split_prefix(line: &str) -> Option<(usize, &str)> {
    let mut boundary = 0;
    let mut depth = 0;

    while boundary < line.len() {
        let rest = &line[boundary..];

        // An ASCII connector ends the prefix: the name follows it directly,
        // and must not be mistaken for more drawing characters.
        if let Some(connector) = ascii_connector(rest) {
            boundary += connector.len();
            depth += connector.chars().count();
            break;
        }

        let Some(ch) = rest.chars().next() else {
            break;
        };
        if INDENT_CHARS.contains(&ch) || is_ascii_guide(rest) {
            boundary += ch.len_utf8();
            depth += 1;
        } else {
            break;
        }
    }

    let name = line[boundary..].trim_end();
    if name.is_empty() {
        return None;
    }
    Some((depth, name))
}

/// The four-character ASCII connectors: `+---` and `\---` (Windows `tree /a`),
/// `|-- ` and `` `-- `` (GNU `tree --charset=ascii`).
fn ascii_connector(text: &str) -> Option<&str> {
    ["+---", "\\---", "|-- ", "`-- "]
        .into_iter()
        .find(|connector| text.starts_with(connector))
        .map(|connector| &text[..connector.len()])
}

/// A `|` that continues a branch (`|   `): it must be followed by whitespace or
/// the end of the line, so that a name such as `|pipe` is left alone.
fn is_ascii_guide(text: &str) -> bool {
    text.strip_prefix('|')
        .is_some_and(|after| after.is_empty() || after.starts_with(char::is_whitespace))
}

/// One surviving input line before directory/file classification.
struct RawEntry {
    depth: usize,
    /// Name as written, including a trailing `/` when present.
    name: String,
    comment: Option<String>,
}

/// How many non-blank lines from the top may precede a Windows drive line.
const HEADER_SEARCH_LINES: usize = 4;

/// The lines of `input` that belong to the tree: Windows volume header and
/// drive line, and the GNU summary line, removed (see the module docs).
fn tree_lines(input: &str) -> Vec<Cow<'_, str>> {
    let mut lines: Vec<&str> = input.split(['\n', '\r']).collect();

    // GNU `tree` ends with a blank line and "N directories, M files".  The
    // blank line is required so that a real entry such as "3 notes" survives.
    if let Some(last) = lines.iter().rposition(|line| !line.trim().is_empty()) {
        if last > 0 && lines[last - 1].trim().is_empty() && is_summary_line(lines[last]) {
            lines.truncate(last);
        }
    }

    // Windows `tree` starts with a localized header and a drive line.  The
    // header's wording depends on the Windows language, so it is recognised by
    // position: everything before a drive line near the top.
    let mut non_blank = 0;
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        non_blank += 1;
        if non_blank > HEADER_SEARCH_LINES {
            break;
        }
        if let Some(root) = windows_drive_root(line) {
            let mut kept: Vec<Cow<'_, str>> = Vec::with_capacity(lines.len());
            if let Some(name) = root {
                kept.push(Cow::Owned(format!("{name}/")));
            }
            kept.extend(lines[index + 1..].iter().copied().map(Cow::Borrowed));
            return kept;
        }
    }

    lines.into_iter().map(Cow::Borrowed).collect()
}

/// `2 directories, 4 files` and its translations: at least two
/// "<number> <word>" parts separated by commas.
fn is_summary_line(line: &str) -> bool {
    if line.starts_with(char::is_whitespace) {
        return false;
    }
    let parts: Vec<&str> = line.trim_end().split(", ").collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            part.split_once(' ').is_some_and(|(count, word)| {
                !count.is_empty()
                    && count.chars().all(|c| c.is_ascii_digit())
                    && !word.is_empty()
                    && !word.chars().any(|c| c.is_ascii_digit())
            })
        })
}

/// A Windows drive line: `D:.` or `C:\some\path`.
///
/// Returns `None` when `line` is not one.  Otherwise `Some(None)` means "drop
/// it" (`D:.`, `C:\`), and `Some(Some(name))` is the last path component, which
/// becomes the root directory.
fn windows_drive_root(line: &str) -> Option<Option<&str>> {
    let line = line.trim_end();
    let mut chars = line.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    let rest = line[letter.len_utf8()..].strip_prefix(':')?;

    if rest == "." {
        return Some(None);
    }
    let path = rest.strip_prefix(['\\', '/'])?;
    let name = path
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .filter(|name| !name.is_empty());
    Some(name)
}

/// Parse an ASCII / Unicode directory tree string into a flat list of
/// [`TreeNode`] values.
///
/// Each returned node carries its indentation depth, kind and optional inline
/// comment, but nodes are **not** yet converted into full filesystem paths —
/// that translation is the responsibility of [`crate::fs_builder`].
///
/// Blank lines, lines made only of indentation glyphs, and comment lines
/// (content starting with `#`, see the module docs) are skipped.
pub fn parse_tree(input: &str) -> Vec<TreeNode> {
    // Split every line once up front: the directory heuristic needs the depth
    // of the next *surviving* line, and skipped lines must not influence it.
    //
    // Lines end at `\n`, `\r\n` or a lone `\r` (classic Mac text).  Splitting on
    // either character is safe because blank lines are dropped below: the
    // empty piece between the `\r` and `\n` of a CRLF pair simply vanishes.
    let entries: Vec<RawEntry> = tree_lines(input)
        .iter()
        .filter_map(|line| {
            let (depth, text) = split_prefix(line)?;
            // Also catches comment lines hidden behind glyphs (`│   # note`),
            // which a whole-line check on the raw text would miss.
            if text.starts_with('#') {
                return None;
            }
            let (name, comment) = parse_dir_and_comment(text);
            (!name.is_empty()).then_some(RawEntry {
                depth,
                name,
                comment,
            })
        })
        .collect();

    entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let next_depth = entries.get(i + 1).map(|next| next.depth);
            let (clean_name, kind) = classify_entry(&entry.name, entry.depth, next_depth);
            TreeNode::new(entry.depth, clean_name, kind).with_comment(entry.comment.clone())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Apply the three-rule heuristic to determine the [`EntryKind`] of a raw
/// entry name.  Returns `(cleaned_name, kind)`.
fn classify_entry(
    raw_name: &str,
    current_depth: usize,
    next_depth: Option<usize>,
) -> (&str, EntryKind) {
    // Rule 1 — explicit trailing slash.
    if raw_name.ends_with('/') {
        return (raw_name.trim_end_matches('/'), EntryKind::Dir);
    }

    // Rule 2 — lookahead: the next line sits deeper.
    if next_depth.is_some_and(|next| next > current_depth) {
        return (raw_name, EntryKind::Dir);
    }

    // Rule 3 — default: treat as a regular file.
    (raw_name, EntryKind::File)
}

/// Split the text of one entry line into its name and its inline comment.
///
/// `line` is the text after the indentation prefix.  Quoting rules are in the
/// module docs.  A `/` that follows a closing quote stays on the returned
/// name (`"my dir"/` → `my dir/`) so the caller can still recognise a
/// directory.  An empty comment (`name #`) is returned as `None`.
pub fn parse_dir_and_comment(line: &str) -> (String, Option<String>) {
    parse_quoted(line).unwrap_or_else(|| split_unquoted(line))
}

/// Parse `"name"` / `'name'` followed by an optional `/` and `# comment`.
///
/// Returns `None` when `line` is not a well-formed quoted entry, in which
/// case the caller treats the whole text as an ordinary name.
fn parse_quoted(line: &str) -> Option<(String, Option<String>)> {
    let quote = line.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let after_open = &line[quote.len_utf8()..];
    let close = after_open.find(quote)?;

    let name = &after_open[..close];
    if name.is_empty() {
        return None;
    }

    let rest = &after_open[close + quote.len_utf8()..];
    let (slash, rest) = match rest.strip_prefix('/') {
        Some(after_slash) => ("/", after_slash),
        None => ("", rest),
    };

    // Anything other than a comment after the closing quote means the quote
    // was part of a plain name such as `'tis it's.txt`; refusing here avoids
    // silently dropping the tail.
    let rest = rest.trim();
    let comment = if rest.is_empty() {
        None
    } else {
        non_empty(rest.strip_prefix('#')?)
    };

    Some((format!("{name}{slash}"), comment))
}

/// Split an unquoted entry at the first `#` that follows whitespace.
fn split_unquoted(text: &str) -> (String, Option<String>) {
    let mut after_whitespace = false;
    for (idx, ch) in text.char_indices() {
        if ch == '#' && after_whitespace {
            return (text[..idx].trim().to_string(), non_empty(&text[idx + 1..]));
        }
        after_whitespace = ch.is_whitespace();
    }
    (text.to_string(), None)
}

/// Trim `text` and return it, or `None` when nothing is left.
fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
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
        let (name, kind) = classify_entry("src/", 0, None);
        assert_eq!(name, "src");
        assert_eq!(kind, EntryKind::Dir);
    }

    #[test]
    fn classify_deeper_next_line_is_dir() {
        assert_eq!(classify_entry("src", 0, Some(4)).1, EntryKind::Dir);
        assert_eq!(classify_entry("src", 4, Some(4)).1, EntryKind::File);
        assert_eq!(classify_entry("src", 4, Some(0)).1, EntryKind::File);
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
