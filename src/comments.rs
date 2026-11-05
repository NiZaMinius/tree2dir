//! Inline comments: which ones are written to disk, and in what syntax.
//!
//! Comments are parsed into [`crate::parser::tree_node::TreeNode::comment`]
//! regardless of settings.  They only reach the disk when the caller opts in
//! with a [`CommentMode`].
//!
//! # File comments
//!
//! A file receives its comment only when its extension is on the allowlist
//! below, because only then is the comment syntax known.  Every other file
//! (no extension, `json`, `css`, `env`, images, archives, …) is created
//! empty: writing `// …` into a JSON file would corrupt it, and guessing for
//! unfamiliar formats is how files get broken.
//!
//! # What is written
//!
//! A comment is always **one line** ending in `\n`.  Before writing, control
//! characters, line breaks and bidirectional overrides are replaced by spaces:
//! otherwise a comment could escape its own line and inject code into the
//! generated file.

use std::path::Path;

use clap::ValueEnum;

/// File that receives a directory's comment.
pub(crate) const DIR_COMMENT_FILE: &str = "COMMENT.md";

/// Which inline comments are persisted.  The default writes none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum CommentMode {
    /// Strip comments from names but never save them (the default).
    #[default]
    None,
    /// Write each file's comment into the file, in the comment syntax of its
    /// extension. Files of unknown type stay empty.
    Files,
    /// Write each directory's comment to COMMENT.md inside that directory.
    Dirs,
    /// Write both file and directory comments.
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

/// How a language spells a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Syntax {
    /// `<prefix> text`.  `preamble` is emitted first when the language only
    /// treats the prefix as a comment inside code (PHP).
    Line {
        prefix: &'static str,
        preamble: &'static str,
    },
    /// `<!-- text -->`.  XML forbids `--` inside a comment, so every run of
    /// hyphens is broken up, which also covers `-->` and `--!>`.
    Markup,
}

const SLASHES: Syntax = Syntax::Line {
    prefix: "//",
    preamble: "",
};
const PHP: Syntax = Syntax::Line {
    prefix: "//",
    preamble: "<?php\n",
};
const HASH: Syntax = Syntax::Line {
    prefix: "#",
    preamble: "",
};
const DASHES: Syntax = Syntax::Line {
    prefix: "--",
    preamble: "",
};
const SEMICOLON: Syntax = Syntax::Line {
    prefix: ";",
    preamble: "",
};
/// Extensions (lowercase, no dot) whose comment syntax is known.
///
/// Deliberately absent: `json` (no comments), `css` (block comments only,
/// left empty by design), `txt`, `cfg`, `conf`, `env` (dialects disagree), and
/// `ml`/`mli`: OCaml comments nest and its lexer reads string literals inside
/// them, so no arbitrary text is safe to embed.
#[rustfmt::skip]
const ALLOWLIST: &[(Syntax, &[&str])] = &[
    (SLASHES, &[
        "rs", "c", "h", "cc", "cpp", "cxx", "hh", "hpp", "cs", "java", "go",
        "js", "mjs", "cjs", "jsx", "ts", "tsx", "kt", "kts", "swift", "scala",
        "dart", "zig",
    ]),
    (PHP, &["php"]),
    (HASH, &[
        "py", "rb", "pl", "r", "sh", "bash", "zsh", "ps1", "yml", "yaml",
        "toml", "mk",
    ]),
    (DASHES, &["sql", "lua", "hs"]),
    (SEMICOLON, &["ini"]),
    (Syntax::Markup, &["html", "htm", "xml", "svg", "md", "markdown"]),
];

fn syntax_for(path: &Path) -> Option<Syntax> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    ALLOWLIST
        .iter()
        .find(|(_, extensions)| extensions.contains(&ext.as_str()))
        .map(|(syntax, _)| *syntax)
}

/// Make `comment` safe to embed as a single line of text.
///
/// Control characters (including `\n`, `\r`, `\t`, NUL, ESC), Unicode line
/// and paragraph separators, and the bidirectional override/isolate
/// characters are replaced by a space, then the result is trimmed.  The
/// comment of a parsed tree can never contain a line break, but library
/// callers can construct one, and a line break in a `.py` comment is code
/// injection.  Bidi controls are the "Trojan Source" vector.
pub(crate) fn sanitize(comment: &str) -> String {
    let spaced: String = comment
        .chars()
        .map(|c| if is_unsafe(c) { ' ' } else { c })
        .collect();
    spaced.trim().to_string()
}

fn is_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        )
}

/// Insert a space after every hyphen that directly follows another hyphen.
fn break_hyphen_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_was_hyphen = false;
    for c in text.chars() {
        if c == '-' && previous_was_hyphen {
            out.push(' ');
        }
        out.push(c);
        previous_was_hyphen = c == '-';
    }
    out
}

fn render(syntax: Syntax, text: &str) -> String {
    match syntax {
        Syntax::Line { prefix, preamble } => format!("{preamble}{prefix} {text}\n"),
        Syntax::Markup => format!("<!-- {} -->\n", break_hyphen_runs(text)),
    }
}

/// The content to write into `path` for `comment`, or `None` when the file
/// must stay empty: unknown extension, no extension, or nothing left of the
/// comment after [`sanitize`].
///
/// The result is a single line ending in `\n` (PHP gets a leading `<?php`
/// line so the comment is code rather than output).
pub(crate) fn format_file_comment(path: &Path, comment: &str) -> Option<String> {
    let syntax = syntax_for(path)?;
    let text = sanitize(comment);
    (!text.is_empty()).then(|| render(syntax, &text))
}

/// The content of a directory's [`DIR_COMMENT_FILE`]: the sanitised comment
/// and a newline, or `None` when nothing is left of it.
pub(crate) fn format_dir_comment(comment: &str) -> Option<String> {
    let text = sanitize(comment);
    (!text.is_empty()).then(|| format!("{text}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, comment: &str) -> Option<String> {
        format_file_comment(Path::new(name), comment)
    }

    #[test]
    fn mode_switches_round_trip() {
        for (files, dirs) in [(false, false), (true, false), (false, true), (true, true)] {
            let mode = CommentMode::from_switches(files, dirs);
            assert_eq!((mode.files(), mode.dirs()), (files, dirs));
        }
        assert_eq!(CommentMode::default(), CommentMode::None);
    }

    #[test]
    fn known_extensions_use_their_syntax() {
        assert_eq!(file("a.rs", "x").as_deref(), Some("// x\n"));
        assert_eq!(file("a.py", "x").as_deref(), Some("# x\n"));
        assert_eq!(file("a.sql", "x").as_deref(), Some("-- x\n"));
        assert_eq!(file("a.ini", "x").as_deref(), Some("; x\n"));
        assert_eq!(file("a.html", "x").as_deref(), Some("<!-- x -->\n"));
    }

    #[test]
    fn extension_match_ignores_case() {
        assert_eq!(file("UPPER.PY", "x").as_deref(), Some("# x\n"));
        assert_eq!(file("Mixed.Md", "x").as_deref(), Some("<!-- x -->\n"));
    }

    #[test]
    fn unknown_or_missing_extension_stays_empty() {
        for name in [
            "a.txt",
            "a.weird",
            "data.json",
            "style.css",
            "Makefile",
            ".env",
            ".gitignore",
            "photo.gif",
            "app.ico",
            "doc.pdf",
            "font.ttf",
            "lib.jar",
            "a.zip",
            "a.png",
            // OCaml: nesting comments and string literals inside them.
            "a.ml",
            "a.mli",
        ] {
            assert_eq!(file(name, "x"), None, "{name} must stay empty");
        }
    }

    #[test]
    fn php_comment_is_code_not_output() {
        assert_eq!(file("a.php", "x").as_deref(), Some("<?php\n// x\n"));
    }

    #[test]
    fn empty_or_blank_comment_writes_nothing() {
        assert_eq!(file("a.rs", ""), None);
        assert_eq!(file("a.rs", "   \t "), None);
        assert_eq!(format_dir_comment(" \n "), None);
    }

    #[test]
    fn line_breaks_cannot_escape_the_comment() {
        let out = file("a.py", "first\nimport os; os.system('x')").unwrap();
        assert_eq!(out, "# first import os; os.system('x')\n");
        assert_eq!(out.matches('\n').count(), 1);

        for sep in ['\r', '\u{85}', '\u{2028}', '\u{2029}', '\u{b}', '\u{c}'] {
            let out = file("a.js", &format!("a{sep}b")).unwrap();
            assert_eq!(out, "// a b\n", "separator {sep:?}");
        }
    }

    #[test]
    fn control_and_bidi_characters_are_replaced() {
        assert_eq!(file("a.rs", "a\0b\x1b[31mc").unwrap(), "// a b [31mc\n");
        assert_eq!(file("a.rs", "a\u{202e}b\u{2066}c").unwrap(), "// a b c\n");
    }

    #[test]
    fn unicode_text_is_preserved() {
        assert_eq!(
            file("a.py", "日本語 ✓ привет").unwrap(),
            "# 日本語 ✓ привет\n"
        );
    }

    #[test]
    fn markup_comment_cannot_be_closed_early() {
        for tricky in [
            "a --> <script>x</script>",
            "a --!> b",
            "a <!-- b",
            "--",
            "-",
        ] {
            let out = file("a.html", tricky).unwrap();
            let inner = out
                .strip_prefix("<!-- ")
                .and_then(|s| s.strip_suffix(" -->\n"))
                .unwrap();
            assert!(!inner.contains("--"), "{tricky:?} -> {out:?}");
        }
        assert_eq!(file("a.svg", "a-->b").unwrap(), "<!-- a- ->b -->\n");
    }

    #[test]
    fn single_hyphens_are_left_alone() {
        assert_eq!(
            file("a.md", "state-of-the-art").unwrap(),
            "<!-- state-of-the-art -->\n"
        );
    }

    #[test]
    fn dir_comment_is_plain_text_with_one_newline() {
        assert_eq!(
            format_dir_comment("source code").as_deref(),
            Some("source code\n")
        );
        assert_eq!(format_dir_comment("a\nb").as_deref(), Some("a b\n"));
    }
}
