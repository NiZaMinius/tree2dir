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
//! a link.  A directory resolved for one entry is remembered for its siblings,
//! but each use confirms with one `lstat` that it is still not a link.  The
//! check and the write are separate steps, so another process swapping a link
//! in between them, or replacing a directory further up than the one
//! re-checked, is not defended against.
//!
//! # Directory comments
//!
//! A directory comment goes to `COMMENT.md` inside that directory, under the
//! same rules as any other file: an existing `COMMENT.md` is left alone
//! (with a warning) unless `--force` is set.  A `COMMENT.md` that the tree
//! itself lists always wins over a generated one, so the two never fight
//! over the same path.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::{Display, Write as _},
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

    let resolved = resolve_paths(nodes, &opts.output_root)?;

    let mut run = Run {
        opts,
        // Everything the tree lists explicitly, files and directories alike: a
        // generated COMMENT.md must not collide with any of it.  A listed
        // directory named COMMENT.md would otherwise find a file in its place
        // and abort the build.
        listed_paths: resolved.iter().map(|entry| entry.path.as_path()).collect(),
        stats: BuildStats::default(),
        planned: HashMap::new(),
        // Created after the root so that a root which had to be made is resolved.
        guard: RootGuard::new(&opts.output_root),
        log: String::new(),
        warnings: String::new(),
        ensured: HashSet::new(),
    };

    let result = resolved.iter().try_for_each(|entry| match entry.kind {
        EntryKind::Dir => create_directory(entry, &mut run),
        EntryKind::File => create_file(entry, &mut run),
    });

    // Printed even when the build failed: it says how far the dry run got.
    print!("{}", run.log);
    eprint!("{}", run.warnings);
    result.map(|()| run.stats)
}

/// Everything one build carries from entry to entry.
struct Run<'a> {
    opts: &'a BuildOptions,
    listed_paths: HashSet<&'a Path>,
    stats: BuildStats,
    /// A dry run writes nothing, so a path it "creates" is not on disk when a
    /// later entry names it again (duplicate lines).  Remember those paths and
    /// what each would be, so the report matches what the real run, which
    /// would find them, does.
    planned: HashMap<PathBuf, EntryKind>,
    guard: RootGuard,
    /// The dry-run listing.  Collected and printed in one go: a line-buffered
    /// stdout makes a system call per line, which on a console is slow for a
    /// listing of thousands of entries.
    log: String,
    /// Warnings, collected for the same reason and printed to stderr after the
    /// listing.  Unbuffered, `eprintln!` costs several system calls a line, and
    /// a second run over a built tree warns once per existing file.
    warnings: String,
    /// Directories this run made or found, so a file below one needs no
    /// `create_dir_all` of its own.
    ensured: HashSet<PathBuf>,
}

impl Run<'_> {
    /// Record a non-fatal warning; [`build`] prints them at the end.
    fn warn(&mut self, message: impl Display) {
        let _ = writeln!(self.warnings, "  ⚠  warning: {message}");
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Refuses writes that would leave the output root through a symbolic link,
/// and reports what is on disk at the same time.
///
/// Every filesystem call costs a system call (on Windows, usually opening a
/// handle), and a tree can have thousands of entries, so the guard is built to
/// ask the disk about an entry once: [`RootGuard::inspect`] both checks and
/// returns the metadata the builder needs.
struct RootGuard {
    /// The output root without `.` segments, to recognise an entry that is the
    /// root itself.
    root: PathBuf,
    /// The output root with every link resolved; `None` when it does not exist
    /// yet (a dry run into a new directory), in which case nothing below it
    /// can be a link either.
    canonical_root: Option<PathBuf>,
    /// Directories already resolved and found to be inside the root.  The
    /// entries of one directory share it, so it is resolved once, not once
    /// per entry; each use still re-checks that it is not a link now.
    verified: RefCell<HashSet<PathBuf>>,
}

impl RootGuard {
    fn new(root: &Path) -> Self {
        Self {
            root: root.components().collect(),
            canonical_root: fs::canonicalize(root).ok(),
            verified: RefCell::new(HashSet::new()),
        }
    }

    /// Look at `path` on disk: what is there (links followed), or `None`.
    ///
    /// Fails with [`AppError::SymlinkEscape`] if writing `path` would end up
    /// outside the root.  Two things can lead out: a link among the
    /// directories above `path`, and `path` itself being a link.
    ///
    /// - The deepest existing directory above `path` is resolved with
    ///   `canonicalize`, which follows every link on the way (junctions
    ///   included), and remembered.  `symlink_metadata` does not follow links,
    ///   so a dangling link counts as existing: `fs::write` through it creates
    ///   its target.
    /// - `path` itself is only resolved if it is a link; a plain file or
    ///   directory is where its (already verified) parent says it is.
    ///
    /// A link that cannot be resolved is refused.
    fn inspect(&self, path: &Path) -> Result<Option<fs::Metadata>, AppError> {
        let Some(root) = &self.canonical_root else {
            return Ok(fs::metadata(path).ok());
        };

        // `.` segments are not path components, so `out/.` is `out`, and its
        // `parent()` is the directory above the root.  GNU `tree` prints a `.`
        // as the first line, which is an entry for the output directory itself:
        // there is nothing above it to check.
        let components: PathBuf = path.components().collect();
        if components == self.root {
            return Ok(fs::metadata(path).ok());
        }
        if let Some(parent) = components.parent() {
            self.verify_directory(parent, path, root)?;
        }

        match fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => match fs::canonicalize(path) {
                Ok(real) if real.starts_with(root) => Ok(fs::metadata(path).ok()),
                _ => Err(AppError::SymlinkEscape(path.to_path_buf())),
            },
            Ok(meta) => Ok(Some(meta)),
            Err(_) => Ok(None),
        }
    }

    /// Check the deepest existing part of `dir`; `whole` is the path being
    /// written, for the error.
    ///
    /// A remembered directory is not taken on trust: one `symlink_metadata`
    /// confirms it is still not a link.  Otherwise something that replaced it
    /// with a link after its first check would be trusted for the rest of the
    /// build, not just for a single entry.
    fn verify_directory(&self, dir: &Path, whole: &Path, root: &Path) -> Result<(), AppError> {
        let mut existing = dir;
        let meta = loop {
            match fs::symlink_metadata(existing) {
                Ok(meta) => break meta,
                Err(_) => match existing.parent() {
                    Some(parent) => existing = parent,
                    None => return Ok(()),
                },
            }
        };

        if !meta.file_type().is_symlink() && self.verified.borrow().contains(existing) {
            return Ok(());
        }

        match fs::canonicalize(existing) {
            Ok(real) if real.starts_with(root) => {
                self.verified.borrow_mut().insert(existing.to_path_buf());
                Ok(())
            }
            _ => {
                self.verified.borrow_mut().remove(existing);
                Err(AppError::SymlinkEscape(whole.to_path_buf()))
            }
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
fn is_dir_or_planned(
    on_disk: Option<&fs::Metadata>,
    path: &Path,
    planned: &HashMap<PathBuf, EntryKind>,
) -> bool {
    on_disk.is_some_and(fs::Metadata::is_dir) || planned.get(path) == Some(&EntryKind::Dir)
}

/// One-line rendering of comment content for the dry-run report.
fn one_line(content: &str) -> String {
    content.trim_end().replace('\n', " ⏎ ")
}

fn skip_listed_comment(target: &Path, run: &mut Run<'_>) {
    run.warn(format!(
        "{} is listed in the tree, so its directory's comment was not written",
        target.display()
    ));
    run.stats.comments_skipped += 1;
}

fn create_directory(entry: &ResolvedEntry<'_>, run: &mut Run<'_>) -> Result<(), AppError> {
    let path = &entry.path;
    let on_disk = run.guard.inspect(path)?;
    let comment = plan_dir_comment(entry, run.opts, &run.listed_paths);

    if run.opts.dry_run {
        // Same decisions as the real run below, from read-only disk checks.
        // A file on disk, or one planned earlier in this pass, makes the real
        // run's create_dir_all fail; promising success would be a lie.
        let file_on_disk = on_disk.as_ref().is_some_and(|meta| !meta.is_dir());
        if file_on_disk || run.planned.get(path) == Some(&EntryKind::File) {
            return Err(not_a_directory(path));
        }
        let exists = is_dir_or_planned(on_disk.as_ref(), path, &run.planned);
        let _ = writeln!(
            run.log,
            "  [dry-run] 📁 {}{}",
            path.display(),
            if exists { " (exists)" } else { "" }
        );
        if !exists {
            run.stats.dirs_created += 1;
            run.planned.insert(path.clone(), EntryKind::Dir);
        }
        match comment {
            DirComment::Nothing => {}
            DirComment::Write { target, content } => {
                let target_on_disk = run.guard.inspect(&target)?;
                let exists = target_on_disk.is_some() || run.planned.contains_key(&target);
                if exists
                    && run.opts.force
                    && is_dir_or_planned(target_on_disk.as_ref(), &target, &run.planned)
                {
                    // fs::write cannot replace a directory.
                    return Err(is_a_directory(&target));
                }
                if exists && !run.opts.force {
                    let _ = writeln!(
                        run.log,
                        "              ↳ {DIR_COMMENT_FILE} exists — comment would be skipped"
                    );
                    run.stats.comments_skipped += 1;
                } else {
                    let _ = writeln!(
                        run.log,
                        "              ↳ {DIR_COMMENT_FILE}: {}",
                        one_line(&content)
                    );
                    run.stats.comments_written += 1;
                    run.planned.insert(target, EntryKind::File);
                }
            }
            DirComment::ListedInTree { target } => skip_listed_comment(&target, run),
        }
        return Ok(());
    }

    // An existing directory is not an error and is not counted, but it may
    // still receive a comment below.
    if !on_disk.as_ref().is_some_and(fs::Metadata::is_dir) {
        fs::create_dir_all(path).map_err(|e| AppError::io(path, e))?;
        run.stats.dirs_created += 1;
    }
    run.ensured.insert(path.clone());

    match comment {
        DirComment::Nothing => {}
        DirComment::ListedInTree { target } => skip_listed_comment(&target, run),
        DirComment::Write { target, content } => {
            if run.guard.inspect(&target)?.is_some() && !run.opts.force {
                run.warn(format!(
                    "{} already exists, directory comment skipped (use --force to overwrite)",
                    target.display()
                ));
                run.stats.comments_skipped += 1;
            } else {
                fs::write(&target, content).map_err(|e| AppError::io(&target, e))?;
                run.stats.comments_written += 1;
            }
        }
    }
    Ok(())
}

fn create_file(entry: &ResolvedEntry<'_>, run: &mut Run<'_>) -> Result<(), AppError> {
    let path = &entry.path;
    let on_disk = run.guard.inspect(path)?;
    let content = file_comment_content(entry, run.opts);

    if run.opts.dry_run {
        let exists = on_disk.is_some() || run.planned.contains_key(path);
        if exists && run.opts.force && is_dir_or_planned(on_disk.as_ref(), path, &run.planned) {
            // The real run's fs::write fails on a directory; an "overwrite"
            // would not happen.
            return Err(is_a_directory(path));
        }
        if exists && !run.opts.force {
            // The real run skips it, and writes no comment either.
            let _ = writeln!(
                run.log,
                "  [dry-run] 📄 {} (exists — would skip)",
                path.display()
            );
            run.stats.files_skipped += 1;
            return Ok(());
        }
        let _ = writeln!(
            run.log,
            "  [dry-run] 📄 {}{}",
            path.display(),
            if exists {
                " (exists — would overwrite)"
            } else {
                ""
            }
        );
        if let Some(content) = &content {
            let _ = writeln!(run.log, "              ↳ comment: {}", one_line(content));
            run.stats.comments_written += 1;
        }
        run.stats.files_created += 1;
        run.planned.insert(path.clone(), EntryKind::File);
        return Ok(());
    }

    if on_disk.is_some() {
        if run.opts.force {
            // Overwrite: fall through to write.
        } else {
            // Skip with a non-fatal warning.
            run.warn(AppError::FileExists(path.to_path_buf()));
            run.stats.files_skipped += 1;
            return Ok(());
        }
    }

    // Ensure all ancestor directories exist before writing the file.  The
    // parent is usually the directory entry just handled, so this is mostly
    // answered from memory.
    if let Some(parent) = path.parent() {
        if !run.ensured.contains(parent) {
            fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
            run.ensured.insert(parent.to_path_buf());
        }
    }

    fs::write(path, content.as_deref().unwrap_or("")).map_err(|e| AppError::io(path, e))?;
    run.stats.files_created += 1;
    if content.is_some() {
        run.stats.comments_written += 1;
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
    let Ok(resolved) = resolve_paths(nodes, root) else {
        return;
    };

    // One write for the whole tree: see Run::log.
    let mut out = String::new();
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
        let _ = writeln!(
            out,
            "{indent}{display_indent}{connector} {}{comment_str}",
            entry.name
        );
    }
    print!("{out}");
}

fn pluralise<'a>(n: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if n == 1 {
        singular
    } else {
        plural
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    /// A root with a directory `d` inside it, and a directory beside the root.
    fn arena() -> (TempDir, PathBuf, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("out");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(root.join("d")).unwrap();
        fs::create_dir(&outside).unwrap();
        (tmp, root, outside)
    }

    #[test]
    fn a_remembered_directory_replaced_by_a_link_is_noticed() {
        let (_tmp, root, outside) = arena();
        let guard = RootGuard::new(&root);

        // `d` is verified and remembered by the first entry...
        assert!(guard.inspect(&root.join("d/a.txt")).is_ok());

        // ...then something swaps it for a link out of the root.
        fs::remove_dir(root.join("d")).unwrap();
        symlink(&outside, root.join("d")).unwrap();

        // The next sibling must not be trusted on the strength of the old check.
        let result = guard.inspect(&root.join("d/b.txt"));
        assert!(
            matches!(result, Err(AppError::SymlinkEscape(_))),
            "{result:?}"
        );
    }

    #[test]
    fn a_remembered_directory_replaced_by_a_link_inside_the_root_is_still_fine() {
        let (_tmp, root, _outside) = arena();
        fs::create_dir(root.join("real")).unwrap();
        let guard = RootGuard::new(&root);
        assert!(guard.inspect(&root.join("d/a.txt")).is_ok());

        fs::remove_dir(root.join("d")).unwrap();
        symlink(root.join("real"), root.join("d")).unwrap();

        assert!(guard.inspect(&root.join("d/b.txt")).is_ok());
    }

    #[test]
    fn a_remembered_directory_that_disappears_is_not_an_error() {
        let (_tmp, root, _outside) = arena();
        let guard = RootGuard::new(&root);
        assert!(guard.inspect(&root.join("d/a.txt")).is_ok());

        fs::remove_dir(root.join("d")).unwrap();

        // Nothing is there any more, so the entry is simply new.
        assert!(matches!(guard.inspect(&root.join("d/b.txt")), Ok(None)));
    }
}
