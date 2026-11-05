//! CLI argument definitions and parsing.
//!
//! All flags and options are defined as a single [`Cli`] struct using
//! [`clap`]'s `derive` feature.  Parsing is performed automatically by
//! [`clap::Parser::parse`] in `main`.

use std::path::PathBuf;

use clap::Parser;

use crate::comments::CommentMode;

/// tree2dir — convert ASCII / Unicode directory trees into real file structures.
///
/// Paste or pipe a `tree`-style listing and let tree2dir recreate the
/// corresponding directories and files on disk.
#[derive(Debug, Parser)]
#[command(
    name = "tree2dir",
    version,
    author,
    about = "Convert ASCII directory trees into physical file structures.",
    long_about = None,
)]
pub struct Cli {
    /// Source file containing an ASCII / Unicode directory tree.
    ///
    /// When supplied, tree2dir operates in *File Mode* and skips the TUI.
    #[arg(short = 'f', long = "file", value_name = "PATH")]
    pub file: Option<PathBuf>,

    /// Target directory in which the structure is generated.
    ///
    /// Defaults to the current working directory (`.`).
    #[arg(short = 'o', long = "output", value_name = "DIR", default_value = ".")]
    pub output: PathBuf,

    /// Simulate output without touching the disk.
    ///
    /// Prints every path that *would* be created, then exits.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Overwrite existing files without prompting.
    ///
    /// Without this flag, existing files are skipped with a warning while
    /// existing directories are silently ignored.
    #[arg(long = "force")]
    pub force: bool,

    /// Skip interactive confirmation prompts.
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,

    /// Disable the TUI even when running inside an interactive terminal.
    ///
    /// Forces plain-text execution mode.
    #[arg(long = "no-tui")]
    pub no_tui: bool,

    /// Which inline `# comments` to save to disk.
    ///
    /// Comments after a name (`main.rs # entry point`) are always removed
    /// from the name. By default they are discarded; this chooses what to
    /// keep. Existing files and COMMENT.md files are never overwritten
    /// without --force.
    #[arg(
        long = "comments",
        value_name = "MODE",
        value_enum,
        conflicts_with_all = ["dir_comments", "file_comments", "with_comments"],
    )]
    pub comments: Option<CommentMode>,

    /// Shorthand for `--comments dirs`: save directory comments to COMMENT.md
    /// inside each directory.
    #[arg(long = "with-dir-comments")]
    pub dir_comments: bool,

    /// Shorthand for `--comments files`: save file comments inside the files,
    /// in the comment syntax of their extension (other files stay empty).
    #[arg(long = "with-file-comments")]
    pub file_comments: bool,

    /// Shorthand for `--comments all`: save file and directory comments.
    #[arg(long = "with-comments")]
    pub with_comments: bool,
}

impl Cli {
    /// Resolve the comment-related flags into a single [`CommentMode`].
    ///
    /// `--comments` and the `--with-*` shorthands cannot be combined (clap
    /// rejects it), so at most one of the two sources is set.
    pub fn comment_mode(&self) -> CommentMode {
        if let Some(mode) = self.comments {
            return mode;
        }
        CommentMode::from_switches(
            self.file_comments || self.with_comments,
            self.dir_comments || self.with_comments,
        )
    }
}
