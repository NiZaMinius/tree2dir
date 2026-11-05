# tree2dir

[![CI](https://github.com/NiZaMinius/tree2dir/actions/workflows/ci.yml/badge.svg)](https://github.com/NiZaMinius/tree2dir/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/tree2dir.svg)](https://crates.io/crates/tree2dir)
[![Downloads](https://img.shields.io/crates/d/tree2dir.svg)](https://crates.io/crates/tree2dir)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**Paste an ASCII directory tree, get real folders and files in seconds.**

Copy a `tree`-style listing from a README, an AI answer, or your own notes —
`tree2dir` turns it into an actual directory structure on disk.

```
my-app/
├── src/
│   ├── main.rs
│   └── config.rs
├── tests/
│   └── integration_test.rs
├── .gitignore
└── README.md
```
↓
```bash
$ tree2dir --file structure.txt
✔  Successfully generated structure!
  ├── 📁 my-app
    ├── 📁 src
      ├── 📄 main.rs
      ├── 📄 config.rs
    ├── 📁 tests
      ├── 📄 integration_test.rs
    ├── 📄 .gitignore
    ├── 📄 README.md
Done in 2.5ms. Created 3 directories and 4 files.
```

Русская версия: [README.ru.md](README.ru.md)

---

## Features

- **Three input modes** — interactive TUI, Unix pipe, or `--file`.
- **Any indentation style** — box-drawing characters (`│├└─`), plain spaces,
  tabs, or output straight from the `tree` command. Depth is measured in
  Unicode scalar values, not bytes, so mixed indentation styles at the same
  visual depth are handled correctly.
- **Smart directory detection** — a trailing `/`, or simply having nested
  children in the tree, is enough; no need to annotate every folder.
- **Safe by default** — refuses to write outside the target directory
  (path-traversal guard), skips existing files unless `--force` is passed,
  and supports `--dry-run` to preview without touching disk.
- **Fast** — typical trees build in single-digit milliseconds.
- **Inline comments** — `main.rs # entry point` just works: the comment never
  ends up in the file name. Opt in with `--comments` to save it, in the
  file's own comment syntax or in a `COMMENT.md` for directories
  (see [Comments](#comments)).

## Install

```bash
cargo install tree2dir
```

Or download a prebuilt binary from the [Releases page](../../releases) for
Linux, macOS, or Windows.

## Usage

### Interactive TUI (default)

Run with no arguments inside a real terminal:

```bash
tree2dir
```

Paste or type your tree, then press **Enter** to build it in the current
directory.

### Pipe mode

```bash
cat structure.txt | tree2dir
```

### File mode

```bash
tree2dir --file structure.txt --output ./my-project
```

### Flags

| Flag | Description |
|---|---|
| `-f, --file <PATH>` | Read the tree from a file instead of stdin/TUI |
| `-o, --output <DIR>` | Target directory (default: current directory) |
| `--dry-run` | Show what would be created, without touching disk |
| `--force` | Overwrite files that already exist |
| `-y, --yes` | Skip the confirmation prompt |
| `--no-tui` | Never launch the TUI, even in an interactive terminal |
| `--comments <MODE>` | Save inline `# comments`: `none` (default), `files`, `dirs` or `all` — see [Comments](#comments) |
| `--with-file-comments` | Shorthand for `--comments files` |
| `--with-dir-comments` | Shorthand for `--comments dirs` |
| `--with-comments` | Shorthand for `--comments all` |

`--comments` cannot be combined with the three shorthands.

## Comments

Add `# a note` after a name and tree2dir treats it as a comment, never as part
of the name. By default comments are only shown in the summary and not saved;
`--comments` chooses what is written to disk:

| Mode | Writes |
|---|---|
| `none` (default) | nothing |
| `files` | each file's comment, inside the file, in the comment syntax of its extension |
| `dirs` | each directory's comment, in a `COMMENT.md` inside that directory |
| `all` | both |

```
my-lib/ # a small library
├── src/ # sources
│   ├── lib.rs # public API
│   └── util.py # helpers
├── data.json # fixtures
├── "release notes.md" # shown on the releases page
└── README.md # docs
```

`tree2dir --file tree.txt --comments all` produces:

| Path | Content |
|---|---|
| `my-lib/COMMENT.md` | `a small library` |
| `my-lib/src/COMMENT.md` | `sources` |
| `my-lib/src/lib.rs` | `// public API` |
| `my-lib/src/util.py` | `# helpers` |
| `my-lib/README.md` | `<!-- docs -->` |
| `my-lib/release notes.md` | `<!-- shown on the releases page -->` |
| `my-lib/data.json` | *(empty)* |

Details worth knowing:

- **Unknown file types stay empty.** A comment is written only for extensions
  whose comment syntax is known (`rs`, `js`, `ts`, `py`, `sh`, `yml`, `toml`,
  `sql`, `html`, `md`, and a few dozen more). Everything else — `json`,
  `css`, `txt`, `env`, images, archives, files without an extension — is
  created empty rather than risk corrupting it.
- **One line per comment**, ending with a newline. Line breaks, control
  characters and anything that would close a comment early (such as `-->`)
  are neutralised, so a comment cannot inject code into the generated file.
- **Nothing is overwritten without `--force`**: not an existing file, and not
  an existing `COMMENT.md` (a warning explains what was skipped). If the tree
  lists a `COMMENT.md` itself, that entry wins over the generated one.
- **`--dry-run`** shows every comment it would write.
- **Where `#` starts a comment.** After whitespace (space or tab):
  `notes#1.txt` and `C#/` keep their `#`. A line that starts with `#` — even
  behind tree glyphs, like `│   # helpers` — is a comment line and creates
  nothing. To name an entry that starts with `#`, quote it: `"#recycle"/`.
- **Names with spaces or `#`** can be quoted with `"…"` or `'…'`:
  `"my dir"/ # notes`.

## TUI keybindings

| Key | Action |
|---|---|
| `Enter` | Confirm and build the structure |
| `Shift+Enter` / `Alt+Enter` | Insert a newline while editing |
| `Ctrl+V` | Paste from the system clipboard |
| `Ctrl+E` | Load the built-in example tree |
| `Ctrl+R` | Clear the input buffer |
| `Esc` / `Ctrl+C` | Quit without writing anything to disk |
| Arrow keys, `Home`, `End` | Move the cursor |
| `Backspace`, `Delete` | Edit text |

## How the parser works

Indentation is measured by counting **Unicode scalar values**
(`.chars().count()`) in the leading whitespace/box-drawing prefix of each
line — not raw bytes. This matters because `│` is 3 bytes in UTF-8 while a
space is 1 byte; byte-length comparisons would misjudge depth. Counting
scalars instead means a `│`-based prefix and an equivalent-width
space-based prefix are always treated as the same depth, regardless of
locale or terminal font.

A line is classified as a **directory** if either:
1. its name ends with `/`, or
2. the next non-blank line in the input is indented one level deeper
   (i.e. it has children).

Otherwise it's treated as a **file**. This means trailing slashes are
optional — raw `tree` command output (which usually omits them) parses
correctly out of the box.

## Security

Before writing anything to disk, every resolved path is checked against
the output root: entries can't use `..`, an absolute path, or a Windows
drive prefix to escape the target directory. This is checked both on the
raw path segment and on the final joined path, so it holds even if a
segment slips past the first check.

## Building from source

```bash
git clone https://github.com/NiZaMinius/tree2dir.git
cd tree2dir
cargo build --release
cargo test
```

## Contributing

Issues and pull requests are welcome — see [Discussions](../../discussions)
for questions or ideas before opening a PR. Please run `cargo fmt` and
`cargo clippy -- -D warnings` before submitting.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
