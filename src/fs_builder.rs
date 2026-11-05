//! Filesystem builder: translates a parsed [`TreeNode`] list into real paths.
//!
//! # Conflict handling
//!
//! When `--force` is **not** set:
//! - **Directories** that already exist are silently ignored — creating a
//!   directory that is already present is not an error.
//! - **Files** that already exist are skipped and a warning is emitted to
//!   standard error, but the build process continues.
//!
//! When `--force` **is** set, existing files are overwritten unconditionally.
//!
//! # Directory comments
//!
//! A directory comment goes to `COMMENT.md` inside that directory, under the
//! same rules as any other file: an existing `COMMENT.md` is left alone
//! (with a warning) unless `--force` is set.  A `COMMENT.md` that the tree
//! itself lists always wins over a generated one, so the two never fight
//! over the same path.

use std::{
    collections::HashSet,
    fmt::Display,
    fs,
    path::{Component, Path, PathBuf},
};

use crate::{
    comments::{format_dir_comment, format_file_comment, sanitize, CommentMode, DIR_COMMENT_FILE},
    error::AppError,
    parser::tree_node::{EntryKind, TreeNode},
};

/// Statistics collected during a build run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BuildStats {
    /// Number of directories created.
    pub dirs_created: usize,
    /// Number of files created.
    pub files_created: usize,
    /// Number of files skipped due to conflicts (no `--force`).
    pub files_skipped: usize,
    /// Number of comments written to disk (in a dry run: that would be).
    ///
    /// Comments are a side effect of their entry, so the files holding them
    /// (`COMMENT.md`) are not part of [`BuildStats::files_created`].
    pub comments_written: usize,
    /// Number of directory comments left unwritten because their
    /// `COMMENT.md` already exists (no `--force`) or is listed in the tree.
    pub comments_skipped: usize,
}

/// Options that govern how the builder behaves.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// Root directory under which the entire structure is generated.
    pub output_root: PathBuf,
    /// When `true`, print the planned operations without writing to disk.
    pub dry_run: bool,
    /// When `true`, overwrite existing files rather than skipping them.
    pub force: bool,
    /// Which inline comments are written to disk (none by default).
    pub comments: CommentMode,
}

impl Default for BuildOptions {
    /// Build into the current directory for real, skipping existing files,
    /// without writing comments.  Struct-update syntax
    /// (`BuildOptions { force: true, ..Default::default() }`) keeps callers
    /// compiling when options are added.
    fn default() -> Self {
        Self {
            output_root: PathBuf::from("."),
            dry_run: false,
            force: false,
            comments: CommentMode::None,
        }
    }
}

/// Build a filesystem structure from a flat list of [`TreeNode`] values.
///
/// Nodes are consumed in order; a depth-stack tracks the current ancestor
/// chain so that each node's full path can be reconstructed without additional
/// metadata.
///
/// Returns [`BuildStats`] on success. Non-fatal warnings (e.g., skipped
/// files) are printed to `stderr`; fatal I/O errors are surfaced as
/// [`AppError`].
pub fn build(nodes: &[TreeNode], opts: &BuildOptions) -> Result<BuildStats, AppError> {
    if nodes.is_empty() {
        return Err(AppError::EmptyInput);
    }

    // Validate the output root exists (or create it).
    if !opts.dry_run {
        fs::create_dir_all(&opts.output_root).map_err(|e| AppError::io(&opts.output_root, e))?;
    }

    let mut stats = BuildStats::default();
    let resolved = resolve_paths(nodes, &opts.output_root)?;

    // Files the tree lists explicitly: a generated COMMENT.md must not
    // collide with one of them.
    let listed_files: HashSet<&Path> = resolved
        .iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .map(|entry| entry.path.as_path())
        .collect();

    for entry in &resolved {
        match entry.kind {
            EntryKind::Dir => create_directory(entry, opts, &listed_files, &mut stats)?,
            EntryKind::File => create_file(entry, opts, &mut stats)?,
        }
    }

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn ensure_within_root(full_path: &Path, root: &Path) -> Result<(), AppError> {
    let root_c: Vec<_> = root.components().collect();
    let path_c: Vec<_> = full_path.components().collect();
    if path_c.len() < root_c.len() || path_c[..root_c.len()] != root_c[..] {
        return Err(AppError::UnsafePath(full_path.to_path_buf()));
    }
    Ok(())
}

/// A [`TreeNode`] joined onto the output root, ready to be created or printed.
///
/// Borrows from the node list so resolving a large tree does not clone every
/// name and comment.
struct ResolvedEntry<'a> {
    /// Absolute-or-relative path under the output root.
    path: PathBuf,
    kind: EntryKind,
    name: &'a str,
    /// Number of ancestor directories, used only to indent the printed tree.
    indent_level: usize,
    comment: Option<&'a str>,
}

fn resolve_paths<'a>(
    nodes: &'a [TreeNode],
    root: &Path,
) -> Result<Vec<ResolvedEntry<'a>>, AppError> {
    let mut stack: Vec<(usize, PathBuf)> = vec![(0, root.to_path_buf())];
    let mut paths = Vec::with_capacity(nodes.len());

    for node in nodes {
        // Guard against raw segment path traversal attempt (e.g., ../secret).
        if Path::new(&node.name).components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(AppError::UnsafePath(PathBuf::from(&node.name)));
        }

        // Unwind the stack until the top entry is a strict ancestor.
        while stack.len() > 1 && stack.last().map(|(d, _)| *d).unwrap_or(0) >= node.depth {
            stack.pop();
        }

        let parent = stack.last().map(|(_, p)| p.as_path()).unwrap_or(root);
        let full_path = parent.join(&node.name);

        // Guard against actual path traversal post-join.
        ensure_within_root(&full_path, root)?;

        let indent_level = stack.len().saturating_sub(1);

        if node.kind == EntryKind::Dir {
            stack.push((node.depth, full_path.clone()));
        }

        paths.push(ResolvedEntry {
            path: full_path,
            kind: node.kind.clone(),
            name: &node.name,
            indent_level,
            comment: node.comment.as_deref(),
        });
    }
    Ok(paths)
}

/// Emit a non-fatal warning.  One place, so the output channel can change.
fn warn(message: impl Display) {
    eprintln!("  ⚠  warning: {message}");
}

/// What to do about a directory's comment.
enum DirComment {
    /// No comment, or directory comments are off.
    Nothing,
    Write {
        target: PathBuf,
        content: String,
    },
    /// The tree itself lists `target`; the listed entry wins.
    ListedInTree {
        target: PathBuf,
    },
}

fn plan_dir_comment(
    entry: &ResolvedEntry<'_>,
    opts: &BuildOptions,
    listed_files: &HashSet<&Path>,
) -> DirComment {
    let Some(content) = entry
        .comment
        .filter(|_| opts.comments.dirs())
        .and_then(format_dir_comment)
    else {
        return DirComment::Nothing;
    };

    let target = entry.path.join(DIR_COMMENT_FILE);
    if listed_files.contains(target.as_path()) {
        DirComment::ListedInTree { target }
    } else {
        DirComment::Write { target, content }
    }
}

/// The content a file entry should be created with, if it gets a comment.
fn file_comment_content(entry: &ResolvedEntry<'_>, opts: &BuildOptions) -> Option<String> {
    entry
        .comment
        .filter(|_| opts.comments.files())
        .and_then(|comment| format_file_comment(&entry.path, comment))
}

/// One-line rendering of comment content for the dry-run report.
fn one_line(content: &str) -> String {
    content.trim_end().replace('\n', " ⏎ ")
}

fn skip_listed_comment(target: &Path, stats: &mut BuildStats) {
    warn(format!(
        "{} is listed in the tree, so its directory's comment was not written",
        target.display()
    ));
    stats.comments_skipped += 1;
}

fn create_directory(
    entry: &ResolvedEntry<'_>,
    opts: &BuildOptions,
    listed_files: &HashSet<&Path>,
    stats: &mut BuildStats,
) -> Result<(), AppError> {
    let path = &entry.path;
    let comment = plan_dir_comment(entry, opts, listed_files);

    if opts.dry_run {
        println!("  [dry-run] 📁 {}", path.display());
        stats.dirs_created += 1;
        match comment {
            DirComment::Nothing => {}
            DirComment::Write { content, .. } => {
                println!("              ↳ {DIR_COMMENT_FILE}: {}", one_line(&content));
                stats.comments_written += 1;
            }
            DirComment::ListedInTree { target } => skip_listed_comment(&target, stats),
        }
        return Ok(());
    }

    // An existing directory is not an error and is not counted, but it may
    // still receive a comment below.
    if !path.is_dir() {
        fs::create_dir_all(path).map_err(|e| AppError::io(path, e))?;
        stats.dirs_created += 1;
    }

    match comment {
        DirComment::Nothing => {}
        DirComment::ListedInTree { target } => skip_listed_comment(&target, stats),
        DirComment::Write { target, content } => {
            if target.exists() && !opts.force {
                warn(format!(
                    "{} already exists, directory comment skipped (use --force to overwrite)",
                    target.display()
                ));
                stats.comments_skipped += 1;
            } else {
                fs::write(&target, content).map_err(|e| AppError::io(&target, e))?;
                stats.comments_written += 1;
            }
        }
    }
    Ok(())
}

fn create_file(
    entry: &ResolvedEntry<'_>,
    opts: &BuildOptions,
    stats: &mut BuildStats,
) -> Result<(), AppError> {
    let path = &entry.path;
    let content = file_comment_content(entry, opts);

    if opts.dry_run {
        println!("  [dry-run] 📄 {}", path.display());
        if let Some(content) = &content {
            println!("              ↳ comment: {}", one_line(content));
            stats.comments_written += 1;
        }
        stats.files_created += 1;
        return Ok(());
    }

    if path.exists() {
        if opts.force {
            // Overwrite: fall through to write.
        } else {
            // Skip with a non-fatal warning.
            warn(AppError::FileExists(path.to_path_buf()));
            stats.files_skipped += 1;
            return Ok(());
        }
    }

    // Ensure all ancestor directories exist before writing the file.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    fs::write(path, content.as_deref().unwrap_or("")).map_err(|e| AppError::io(path, e))?;
    stats.files_created += 1;
    if content.is_some() {
        stats.comments_written += 1;
    }
    Ok(())
}

/// Render a post-execution summary to stdout.
pub fn print_summary(
    nodes: &[TreeNode],
    root: &Path,
    stats: &BuildStats,
    elapsed_ms: f64,
    dry_run: bool,
) {
    if dry_run {
        println!("\n✔  Dry-run complete — nothing was written to disk.\n");
    } else {
        println!("\n✔  Successfully generated structure!\n");
    }

    print_tree(nodes, root, 0, "  ");

    let dirs_str = pluralise(stats.dirs_created, "directory", "directories");
    let files_str = pluralise(stats.files_created, "file", "files");

    if dry_run {
        println!(
            "\nWould create {} {dirs_str} and {} {files_str} in {elapsed_ms:.1}ms.",
            stats.dirs_created, stats.files_created
        );
    } else {
        println!(
            "\nDone in {elapsed_ms:.1}ms. Created {} {dirs_str} and {} {files_str}.",
            stats.dirs_created, stats.files_created
        );
    }

    if stats.comments_written > 0 {
        let comments_str = pluralise(stats.comments_written, "comment", "comments");
        let verb = if dry_run { "Would write" } else { "Wrote" };
        println!("{verb} {} {comments_str}.", stats.comments_written);
    }

    if stats.files_skipped > 0 {
        let skipped_str = pluralise(stats.files_skipped, "file", "files");
        eprintln!(
            "  ⚠  {} existing {skipped_str} skipped (use --force to overwrite).",
            stats.files_skipped
        );
    }

    if stats.comments_skipped > 0 {
        let skipped_str = pluralise(
            stats.comments_skipped,
            "directory comment",
            "directory comments",
        );
        eprintln!(
            "  ⚠  {} {skipped_str} not written (see the warnings above).",
            stats.comments_skipped
        );
    }
}

fn print_tree(nodes: &[TreeNode], root: &Path, _depth: usize, indent: &str) {
    if let Ok(resolved) = resolve_paths(nodes, root) {
        for entry in resolved {
            let display_indent = "  ".repeat(entry.indent_level);
            let connector = if entry.kind == EntryKind::Dir {
                "├── 📁"
            } else {
                "├── 📄"
            };
            // Sanitised: comments come from pasted text and must not carry
            // terminal escape sequences into the user's terminal.
            let comment_str = entry
                .comment
                .map(sanitize)
                .filter(|s| !s.is_empty())
                .map(|s| format!("\t# {s}"))
                .unwrap_or_default();
            println!(
                "{indent}{display_indent}{connector} {}{comment_str}",
                entry.name
            );
        }
    }
}

fn pluralise<'a>(n: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if n == 1 {
        singular
    } else {
        plural
    }
}
