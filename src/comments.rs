//! Inline comments: which ones are written to disk, and in what syntax.
//!
//! Comments are parsed into [`crate::parser::tree_node::TreeNode::comment`]
//! regardless of settings.  They only reach the disk when the caller opts in
//! with a [`CommentMode`].

use std::path::Path;

/// File that receives a directory's comment.
pub const DIR_COMMENT_FILE: &str = "COMMENT.md";

/// Which inline comments are persisted.  The default writes none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CommentMode {
    /// Comments are parsed and stripped from names but never written.
    #[default]
    None,
    /// Write a file's comment into the file, in the syntax of its extension.
    Files,
    /// Write a directory's comment into `COMMENT.md` inside that directory.
    Dirs,
    /// Both of the above.
    All,
}

impl CommentMode {
    /// Whether file comments are written.
    pub fn files(self) -> bool {
        matches!(self, Self::Files | Self::All)
    }

    /// Whether directory comments are written.
    pub fn dirs(self) -> bool {
        matches!(self, Self::Dirs | Self::All)
    }

    /// Combine two independent switches into one mode.
    pub fn from_switches(files: bool, dirs: bool) -> Self {
        match (files, dirs) {
            (false, false) => Self::None,
            (true, false) => Self::Files,
            (false, true) => Self::Dirs,
            (true, true) => Self::All,
        }
    }
}

pub(crate) fn is_commentable<P: AsRef<Path>>(path: P) -> bool {
    // List of extensions that don't support arbitrary text or comments
    const RESTRICTED_EXTENSIONS: &[&str] = &[
        // Archives and compressed
        "zip", "tar", "gz", "tgz", "7z", "rar", "bz2", // Executables and binaries
        "exe", "dll", "so", "bin", "o", "obj", // Bytecode
        "class", "pyc", "wasm", // Media containers (raw payloads)
        "png", "jpg", "jpeg", "mp3", "mp4", "mkv", // Databases
        "sqlite",
    ];

    path.as_ref()
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext_str| {
            let lower = ext_str.to_lowercase();
            !RESTRICTED_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

pub(crate) fn format_comment<P: AsRef<Path>>(path: P, comment: String) -> String {
    let ext = path
        .as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        // Markup and documentation formats -> <!-- comment -->
        "html" | "xml" | "svg" | "md" => format!("<!-- {comment} -->"),

        // Scripting, config, and shell formats -> # comment
        "py" | "sh" | "bash" | "rb" | "yml" | "yaml" | "toml" | "ini" => format!("# {comment}"),

        // SQL, Lua, Haskell -> -- comment
        "sql" | "lua" | "hs" => format!("-- {comment}"),

        // C-style languages (Rust, JS, TS, C++, Java, Go, etc.) -> // comment
        "rs" | "js" | "ts" | "c" | "cpp" | "h" | "hpp" | "java" | "go" | "cs" | "php" => {
            format!("// {comment}")
        }

        // Default fallback for unknown types
        _ => format!("// {comment}"),
    }
}
