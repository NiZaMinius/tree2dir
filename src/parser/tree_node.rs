//! Core abstract syntax types that represent a parsed directory tree.
//!
//! After the raw ASCII input is processed by [`crate::parser::parse`], the
//! result is a flat `Vec<TreeNode>` whose parent–child relationships are
//! encoded by the `depth` field.  [`crate::fs_builder`] consumes this vector
//! and walks the depth-stack to reconstruct the actual filesystem hierarchy.

/// Discriminates between a filesystem directory and a regular file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    /// A directory.  May or may not carry a trailing `/` in the source text.
    Dir,
    /// A regular (empty) file.
    File,
}

/// A single node within the parsed tree.
///
/// `depth` is measured in **Unicode scalar values** (`.chars().count()`) of
/// the indentation prefix, not in raw bytes.  This guarantees consistent
/// behaviour across platforms regardless of box-drawing character encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    /// The visual depth of this node derived from its indentation prefix.
    pub depth: usize,
    /// The name of the entry as it appears in the source text, with any
    /// trailing `/` stripped and leading / trailing whitespace removed.
    pub name: String,
    /// Whether this node represents a directory or a file.
    pub kind: EntryKind,
    /// Optional comment field
    pub comment: Option<String>,
}

impl TreeNode {
    /// Construct a new [`TreeNode`].
    pub fn new(
        depth: usize,
        name: impl Into<String>,
        kind: EntryKind,
        comment: Option<String>,
    ) -> Self {
        Self {
            depth,
            name: name.into(),
            kind,
            comment,
        }
    }
}
