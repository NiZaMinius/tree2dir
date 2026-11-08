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
//! # Symbolic links
//!
//! The path checks while resolving entries are lexical, so they cannot see that
//! `out/link` is really `/elsewhere`.  Before any write the builder resolves
//! the deepest part of the target that already exists and refuses
//! ([`AppError::SymlinkEscape`]) if it lands outside the output root.  Links
//! that stay inside the root are fine, and so is an output root that is itself
//! a link.  The check and the write are separate steps, so it does not defend
//! against another process swapping a link in between.
//!
//! # Directory comments
//!
//! A directory comment goes to `COMMENT.md` inside that directory, under the
//! same rules as any other file: an existing `COMMENT.md` is left alone
//! (with a warning) unless `--force` is set.  A `COMMENT.md` that the tree
//! itself lists always wins over a generated one, so the two never fight
//! over the same path.

use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    fs, io,
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
    ///
    /// The report follows what a real run would do given what already exists:
    /// existing directories are not counted, existing files are reported as
    /// skipped (or overwritten with `force`), and no comment is promised for a
    /// file that would be skipped.
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

    // Everything the tree lists explicitly, files and directories alike: a
    // generated COMMENT.md must not collide with any of it.  A listed
    // directory named COMMENT.md would otherwise find a file in its place and
    // abort the build.
    let listed_paths: HashSet<&Path> = resolved.iter().map(|entry| entry.path.as_path()).collect();

    // A dry run writes nothing, so a path it "creates" is not on disk when a
    // later entry names it again (duplicate lines).  Remember those paths and
    // what each would be, so the report matches what the real run, which would
    // find them, does.
    let mut planned: HashMap<PathBuf, EntryKind> = HashMap::new();

    // Created after the root so that a root which had to be made is resolved.
    let guard = RootGuard::new(&opts.output_root);

    for entry in &resolved {
        match entry.kind {
            EntryKind::Dir => {
                create_directory(entry, opts, &listed_paths, &mut stats, &mut planned, &guard)?
            }
            EntryKind::File => create_file(entry, opts, &mut stats, &mut planned, &guard)?,
        }
    }

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Refuses writes that would leave the output root through a symbolic link.
struct RootGuard {
    /// The output root with every link resolved; `None` when it does not exist
    /// yet (a dry run into a new directory), in which case nothing below it
    /// can be a link either.
    canonical_root: Option<PathBuf>,
}

impl RootGuard {
    fn new(root: &Path) -> Self {
        Self {
            canonical_root: fs::canonicalize(root).ok(),
        }
    }

    /// Fail if writing `path` would end up outside the root.
    ///
    /// The deepest existing component is found with `symlink_metadata`, which
    /// does not follow links: a dangling link must count as existing, because
    /// `fs::write` through it creates its target.  Resolving that component
    /// with `canonicalize` follows every link on the way, junctions included.
    /// A link that cannot be resolved is refused.
    fn check(&self, path: &Path) -> Result<(), AppError> {
        let Some(root) = &self.canonical_root else {
            return Ok(());
        };

        let mut existing = path;
        while fs::symlink_metadata(existing).is_err() {
            match existing.parent() {
                Some(parent) => existing = parent,
                None => return Ok(()),
            }
        }

        match fs::canonicalize(existing) {
            Ok(real) if real.starts_with(root) => Ok(()),
            _ => Err(AppError::SymlinkEscape(path.to_path_buf())),
        }
    }
}

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
    listed_paths: &HashSet<&Path>,
) -> DirComment {
    let Some(content) = entry
        .comment
        .filter(|_| opts.comments.dirs())
        .and_then(format_dir_comment)
    else {
        return DirComment::Nothing;
    };

    let target = entry.path.join(DIR_COMMENT_FILE);
    if listed_paths.contains(target.as_path()) {
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

/// The error a real run would hit when it tries to write a file at a path that
/// is a directory.
fn is_a_directory(path: &Path) -> AppError {
    AppError::io(
        path,
        io::Error::new(io::ErrorKind::IsADirectory, "is a directory"),
    )
}

/// The error a real run would hit when it tries to create a directory at a
/// path that is a file.
fn not_a_directory(path: &Path) -> AppError {
    AppError::io(
        path,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "exists and is not a directory",
        ),
    )
}

/// Whether `path` is, or in a dry run would by now be, a directory.
fn is_dir_or_planned(path: &Path, planned: &HashMap<PathBuf, EntryKind>) -> bool {
    path.is_dir() || planned.get(path) == Some(&EntryKind::Dir)
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
    listed_paths: &HashSet<&Path>,
    stats: &mut BuildStats,
    planned: &mut HashMap<PathBuf, EntryKind>,
    guard: &RootGuard,
) -> Result<(), AppError> {
    let path = &entry.path;
    guard.check(path)?;
    let comment = plan_dir_comment(entry, opts, listed_paths);

    if opts.dry_run {
        // Same decisions as the real run below, from read-only disk checks.
        // A file on disk, or one planned earlier in this pass, makes the real
        // run's create_dir_all fail; promising success would be a lie.
        if (path.exists() && !path.is_dir()) || planned.get(path) == Some(&EntryKind::File) {
            return Err(not_a_directory(path));
        }
        let exists = is_dir_or_planned(path, planned);
        println!(
            "  [dry-run] 📁 {}{}",
            path.display(),
            if exists { " (exists)" } else { "" }
        );
        if !exists {
            stats.dirs_created += 1;
            planned.insert(path.clone(), EntryKind::Dir);
        }
        match comment {
            DirComment::Nothing => {}
            DirComment::Write { target, content } => {
                guard.check(&target)?;
                let exists = target.exists() || planned.contains_key(&target);
                if exists && opts.force && is_dir_or_planned(&target, planned) {
                    // fs::write cannot replace a directory.
                    return Err(is_a_directory(&target));
                }
                if exists && !opts.force {
                    println!(
                        "              ↳ {DIR_COMMENT_FILE} exists — comment would be skipped"
                    );
                    stats.comments_skipped += 1;
                } else {
                    println!("              ↳ {DIR_COMMENT_FILE}: {}", one_line(&content));
                    stats.comments_written += 1;
                    planned.insert(target, EntryKind::File);
                }
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
            guard.check(&target)?;
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
    planned: &mut HashMap<PathBuf, EntryKind>,
    guard: &RootGuard,
) -> Result<(), AppError> {
    let path = &entry.path;
    guard.check(path)?;
    let content = file_comment_content(entry, opts);

    if opts.dry_run {
        let exists = path.exists() || planned.contains_key(path);
        if exists && opts.force && is_dir_or_planned(path, planned) {
            // The real run's fs::write fails on a directory; an "overwrite"
            // would not happen.
            return Err(is_a_directory(path));
        }
        if exists && !opts.force {
            // The real run skips it, and writes no comment either.
            println!("  [dry-run] 📄 {} (exists — would skip)", path.display());
            stats.files_skipped += 1;
            return Ok(());
        }
        println!(
            "  [dry-run] 📄 {}{}",
            path.display(),
            if exists {
                " (exists — would overwrite)"
            } else {
                ""
            }
        );
        if let Some(content) = &content {
            println!("              ↳ comment: {}", one_line(content));
            stats.comments_written += 1;
        }
        stats.files_created += 1;
        planned.insert(path.clone(), EntryKind::File);
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
        let verb = if dry_run {
            "would be skipped"
        } else {
            "skipped"
        };
        eprintln!(
            "  ⚠  {} existing {skipped_str} {verb} (use --force to overwrite).",
            stats.files_skipped
        );
    }

    if stats.comments_skipped > 0 {
        let skipped_str = pluralise(
            stats.comments_skipped,
            "directory comment",
            "directory comments",
        );
        let (verb, hint) = if dry_run {
            ("would not be written", "the listing above")
        } else {
            ("not written", "the warnings above")
        };
        eprintln!(
            "  ⚠  {} {skipped_str} {verb} (see {hint}).",
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
