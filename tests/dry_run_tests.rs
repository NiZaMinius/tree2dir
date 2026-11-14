//! `--dry-run` must report what a real run would do on the disk as it is now.
//!
//! The core invariant: on identical disks, a dry run and a real run produce the
//! same [`BuildStats`], whatever already exists, with or without `--force`,
//! for every comment mode.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;
use tree2dir::comments::CommentMode;
use tree2dir::fs_builder::{build, BuildOptions};
use tree2dir::parser::parse_tree;

const TREE: &str = "\
proj/ # the project
├── src/ # sources
│   ├── main.rs # entry
│   └── util.py # helpers
├── data.json
└── README.md # docs
";

/// Duplicate entries: the real run sees the first one on disk when it reaches the second.
const DUPLICATES: &str = "a.txt\na.txt\nd/ # dir\nd/ # again\nd/x.rs # file\nd/x.rs # file\n";

/// The same path listed as a file and as a directory, in both orders.
const TYPE_CONFLICTS: &str = "f\nf/\nd/\nd\nok.txt\n";

fn cmd() -> Command {
    Command::cargo_bin("tree2dir").expect("tree2dir binary not found — run `cargo build` first")
}

fn touch(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

type Setup = fn(&Path);

/// Pre-existing disk states, applied identically to both directories.
fn setups() -> Vec<(&'static str, Setup)> {
    vec![
        ("empty disk", |_| {}),
        ("existing directories", |r| {
            fs::create_dir_all(r.join("proj/src")).unwrap();
        }),
        ("existing files", |r| {
            touch(r, "proj/data.json", "old");
            touch(r, "proj/src/main.rs", "old");
            touch(r, "a.txt", "old");
        }),
        ("existing COMMENT.md", |r| {
            touch(r, "proj/COMMENT.md", "old");
            touch(r, "d/COMMENT.md", "old");
        }),
        ("directories where files are expected", |r| {
            fs::create_dir_all(r.join("proj/data.json")).unwrap();
            fs::create_dir_all(r.join("a.txt")).unwrap();
            fs::create_dir_all(r.join("proj/COMMENT.md")).unwrap();
        }),
        ("everything exists", |r| {
            for rel in [
                "proj/data.json",
                "proj/README.md",
                "proj/src/main.rs",
                "proj/src/util.py",
                "proj/COMMENT.md",
                "proj/src/COMMENT.md",
                "a.txt",
                "d/x.rs",
                "d/COMMENT.md",
            ] {
                touch(r, rel, "old");
            }
        }),
    ]
}

fn opts(root: &Path, dry_run: bool, force: bool, comments: CommentMode) -> BuildOptions {
    BuildOptions {
        output_root: root.to_path_buf(),
        dry_run,
        force,
        comments,
    }
}

#[test]
fn dry_run_and_real_run_agree_in_every_state() {
    let modes = [
        CommentMode::None,
        CommentMode::Files,
        CommentMode::Dirs,
        CommentMode::All,
    ];
    for (state, setup) in setups() {
        for tree in [TREE, DUPLICATES, TYPE_CONFLICTS] {
            for force in [false, true] {
                for mode in modes {
                    let nodes = parse_tree(tree);
                    let (dry_dir, real_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
                    setup(dry_dir.path());
                    setup(real_dir.path());

                    let dry = build(&nodes, &opts(dry_dir.path(), true, force, mode));
                    let real = build(&nodes, &opts(real_dir.path(), false, force, mode));

                    let context =
                        format!("state={state:?} force={force} mode={mode:?} tree={tree:?}");
                    match (dry, real) {
                        (Ok(dry), Ok(real)) => assert_eq!(dry, real, "stats differ: {context}"),
                        // Both fail: the dry run told the truth about a build
                        // that cannot succeed.
                        (Err(_), Err(_)) => {}
                        (dry, real) => panic!(
                            "dry run and real run disagree on success: dry={:?} real={:?}; {context}",
                            dry.map(|_| "ok"),
                            real.map(|_| "ok")
                        ),
                    }
                }
            }
        }
    }
}

#[test]
fn dry_run_never_changes_the_disk_even_when_files_exist() {
    for (state, setup) in setups() {
        let dir = TempDir::new().unwrap();
        setup(dir.path());
        let snapshot = |root: &Path| {
            let mut out = Vec::new();
            fn walk(d: &Path, root: &Path, out: &mut Vec<(String, String)>) {
                let mut es: Vec<_> = fs::read_dir(d).unwrap().map(|e| e.unwrap()).collect();
                es.sort_by_key(|e| e.file_name());
                for e in es {
                    let p = e.path();
                    let rel = p.strip_prefix(root).unwrap().display().to_string();
                    if p.is_dir() {
                        out.push((rel, "<dir>".into()));
                        walk(&p, root, out);
                    } else {
                        out.push((rel, fs::read_to_string(&p).unwrap()));
                    }
                }
            }
            walk(root, root, &mut out);
            out
        };
        let before = snapshot(dir.path());

        // Success is not the point here: with --force some states make the dry run
        // fail (a directory where a file is expected), and it must leave the disk
        // untouched either way.
        let _ = build(
            &parse_tree(TREE),
            &opts(dir.path(), true, true, CommentMode::All),
        );

        assert_eq!(before, snapshot(dir.path()), "state {state:?}");
    }
}

#[test]
fn dry_run_on_a_directory_path_that_is_a_file_fails_like_the_real_run() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "proj", "i am a file");
    let nodes = parse_tree("proj/\n└── a.rs\n");

    let real = build(&nodes, &opts(dir.path(), false, false, CommentMode::None));
    let dry = build(&nodes, &opts(dir.path(), true, false, CommentMode::None));

    assert!(real.is_err(), "the real run cannot create the directory");
    assert!(dry.is_err(), "a dry run must not promise what would fail");
    assert_eq!(
        fs::read_to_string(dir.path().join("proj")).unwrap(),
        "i am a file"
    );
}

#[test]
fn dry_run_rejects_a_path_listed_as_a_file_and_then_as_a_directory() {
    let dir = TempDir::new().unwrap();
    let nodes = parse_tree("x\nx/\n");

    let real = build(&nodes, &opts(dir.path(), false, false, CommentMode::None));
    let dry = build(
        &nodes,
        &opts(
            TempDir::new().unwrap().path(),
            true,
            false,
            CommentMode::None,
        ),
    );

    assert!(
        real.is_err(),
        "the real run cannot make a directory over its own file"
    );
    assert!(dry.is_err(), "so the dry run must not report success");
}

#[test]
fn dry_run_with_force_rejects_a_file_over_a_directory_that_exists() {
    let (dry_dir, real_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    for d in [&dry_dir, &real_dir] {
        fs::create_dir(d.path().join("a.txt")).unwrap();
    }
    let nodes = parse_tree("a.txt\n");

    let real = build(
        &nodes,
        &opts(real_dir.path(), false, true, CommentMode::None),
    );
    let dry = build(&nodes, &opts(dry_dir.path(), true, true, CommentMode::None));

    assert!(real.is_err(), "fs::write cannot overwrite a directory");
    assert!(
        dry.is_err(),
        "so --force over a directory is not an overwrite"
    );
}

#[test]
fn dry_run_without_force_skips_a_file_whose_path_is_a_directory_like_the_real_run() {
    let (dry_dir, real_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    for d in [&dry_dir, &real_dir] {
        fs::create_dir(d.path().join("a.txt")).unwrap();
    }
    let nodes = parse_tree("a.txt\n");

    let real = build(
        &nodes,
        &opts(real_dir.path(), false, false, CommentMode::None),
    )
    .unwrap();
    let dry = build(
        &nodes,
        &opts(dry_dir.path(), true, false, CommentMode::None),
    )
    .unwrap();

    assert_eq!(dry, real);
    assert_eq!(dry.files_skipped, 1);
}

#[test]
fn dry_run_with_force_rejects_a_file_listed_after_a_directory_of_the_same_name() {
    let nodes = parse_tree("x/\nx\n");
    let real = build(
        &nodes,
        &opts(
            TempDir::new().unwrap().path(),
            false,
            true,
            CommentMode::None,
        ),
    );
    let dry = build(
        &nodes,
        &opts(
            TempDir::new().unwrap().path(),
            true,
            true,
            CommentMode::None,
        ),
    );
    assert!(real.is_err() && dry.is_err());
}

#[test]
fn dry_run_with_force_rejects_a_comment_file_that_would_replace_a_directory() {
    let (dry_dir, real_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    for d in [&dry_dir, &real_dir] {
        fs::create_dir_all(d.path().join("proj/COMMENT.md")).unwrap();
    }
    let nodes = parse_tree("proj/ # about\n");

    let real = build(
        &nodes,
        &opts(real_dir.path(), false, true, CommentMode::Dirs),
    );
    let dry = build(&nodes, &opts(dry_dir.path(), true, true, CommentMode::Dirs));

    assert!(real.is_err() && dry.is_err());
}

// ---------------------------------------------------------------------------
// The command line: what the user reads
// ---------------------------------------------------------------------------

fn write_tree(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.path().join("tree.txt");
    fs::write(&path, TREE).unwrap();
    path
}

fn run_dry(tree: &Path, out: &Path, extra: &[&str]) -> (String, String) {
    let assert = cmd()
        .args(["--file", tree.to_str().unwrap()])
        .args(["--output", out.to_str().unwrap()])
        .arg("--dry-run")
        .args(extra)
        .assert()
        .success();
    let o = assert.get_output();
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn dry_run_after_a_real_run_says_nothing_would_be_created() {
    let input = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let tree = write_tree(&input);
    cmd()
        .args(["--file", tree.to_str().unwrap()])
        .args(["--output", out.path().to_str().unwrap()])
        .assert()
        .success();

    let (stdout, stderr) = run_dry(&tree, out.path(), &[]);

    assert!(
        stdout.contains("Would create 0 directories and 0 files"),
        "{stdout}"
    );
    assert!(
        stderr.contains("4 existing files would be skipped"),
        "{stderr}"
    );
    assert!(stderr.contains("--force"), "{stderr}");
}

#[test]
fn dry_run_marks_each_existing_path() {
    let input = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let tree = write_tree(&input);
    // Creates `proj` as well, so both directories exist.
    fs::create_dir_all(out.path().join("proj/src")).unwrap();
    touch(out.path(), "proj/data.json", "old");

    let (stdout, _) = run_dry(&tree, out.path(), &[]);
    // Paths are printed with the platform's separator; the assertions below
    // name them with `/`.
    let stdout = stdout.replace('\\', "/");

    let line = |needle: &str| {
        stdout
            .lines()
            .find(|l| l.contains(needle))
            .unwrap_or("")
            .to_string()
    };
    assert!(line("proj/src").contains("(exists)"), "{stdout}");
    assert!(line("data.json").contains("would skip"), "{stdout}");
    assert!(!line("README.md").contains("exists"), "{stdout}");
    assert!(
        stdout.contains("Would create 0 directories and 3 files"),
        "{stdout}"
    );
}

#[test]
fn dry_run_with_force_says_existing_files_would_be_overwritten() {
    let input = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let tree = write_tree(&input);
    touch(out.path(), "proj/data.json", "old");

    let (stdout, stderr) = run_dry(&tree, out.path(), &["--force"]);

    let line = stdout.lines().find(|l| l.contains("data.json")).unwrap();
    assert!(line.contains("would overwrite"), "{stdout}");
    assert!(
        stdout.contains("and 4 files"),
        "overwritten files count as created: {stdout}"
    );
    assert!(!stderr.contains("skipped"), "{stderr}");
}

#[test]
fn dry_run_does_not_promise_a_comment_for_a_file_it_would_skip() {
    let input = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let tree = write_tree(&input);
    touch(out.path(), "proj/README.md", "old");

    let (stdout, _) = run_dry(&tree, out.path(), &["--comments", "files"]);

    assert!(
        stdout.contains("Would write 2 comments."),
        "main.rs and util.py only:\n{stdout}"
    );
    assert!(!stdout.contains("comment: <!-- docs -->"), "{stdout}");
}

#[test]
fn dry_run_reports_an_existing_comment_md_as_skipped() {
    let input = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let tree = write_tree(&input);
    touch(out.path(), "proj/COMMENT.md", "old");

    let (stdout, stderr) = run_dry(&tree, out.path(), &["--comments", "dirs"]);

    assert!(stdout.contains("Would write 1 comment."), "{stdout}");
    assert!(
        stderr.contains("1 directory comment would not be written"),
        "{stderr}"
    );
}

#[test]
fn dry_run_with_force_fails_clearly_when_a_file_would_replace_a_directory() {
    let input = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let tree = write_tree(&input);
    fs::create_dir_all(out.path().join("proj/data.json")).unwrap();

    cmd()
        .args(["--file", tree.to_str().unwrap()])
        .args(["--output", out.path().to_str().unwrap()])
        .args(["--dry-run", "--force"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("data.json"));
}

#[test]
fn dry_run_fails_clearly_when_a_directory_would_collide_with_a_file() {
    let input = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let tree = write_tree(&input);
    touch(out.path(), "proj", "a file");

    cmd()
        .args(["--file", tree.to_str().unwrap()])
        .args(["--output", out.path().to_str().unwrap()])
        .arg("--dry-run")
        .assert()
        .failure()
        .stderr(predicate::str::contains("proj"));
}

// ---------------------------------------------------------------------------
// The listing and the warnings are printed in one go; nothing may be lost
// ---------------------------------------------------------------------------

#[test]
fn dry_run_prints_what_it_listed_before_failing() {
    let input = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let tree = input.path().join("tree.txt");
    fs::write(&tree, "a.txt\nb.txt\nx\nx/\n").unwrap();

    let assert = cmd()
        .args(["--file", tree.to_str().unwrap()])
        .args(["--output", out.path().to_str().unwrap()])
        .arg("--dry-run")
        .assert()
        .failure();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(stdout.contains("a.txt"), "{stdout}");
    assert!(stdout.contains("b.txt"), "{stdout}");
}

#[test]
fn every_skipped_file_is_still_warned_about() {
    let input = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let tree = input.path().join("tree.txt");
    let names: Vec<String> = (0..25).map(|i| format!("f{i}.txt")).collect();
    fs::write(&tree, names.join("\n") + "\n").unwrap();
    let run = || {
        cmd()
            .args(["--file", tree.to_str().unwrap()])
            .args(["--output", out.path().to_str().unwrap()])
            .assert()
            .success()
    };
    run();

    let second = run();

    let stderr = String::from_utf8_lossy(&second.get_output().stderr).into_owned();
    assert_eq!(stderr.matches("already exists").count(), 25, "{stderr}");
}

// ---------------------------------------------------------------------------
// A root entry named `.` is the output directory itself (GNU `tree` prints one)
// ---------------------------------------------------------------------------

#[test]
fn a_root_entry_named_dot_builds_in_the_output_directory() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree(".\n├── a.rs\n└── src\n    └── b.rs\n");

    for dry_run in [true, false] {
        let result = build(&nodes, &opts(out.path(), dry_run, false, CommentMode::All));
        assert!(result.is_ok(), "dry_run={dry_run}: {result:?}");
    }

    assert!(out.path().join("a.rs").is_file());
    assert!(out.path().join("src/b.rs").is_file());
}

#[test]
fn a_dot_entry_below_a_directory_is_that_directory() {
    let out = TempDir::new().unwrap();
    let nodes = parse_tree("proj/\n└── ./\n    └── a.rs\n");

    let result = build(&nodes, &opts(out.path(), false, false, CommentMode::None));

    assert!(result.is_ok(), "{result:?}");
    assert!(out.path().join("proj/a.rs").is_file());
}

#[test]
fn a_root_entry_named_dot_works_with_a_relative_output_directory() {
    let dir = TempDir::new().unwrap();
    let tree = dir.path().join("tree.txt");
    fs::write(&tree, ".\n├── a.rs\n└── b.rs\n").unwrap();

    // `--output .` is the default; the check must not mistake the root's
    // parent for something outside the root.
    cmd()
        .current_dir(dir.path())
        .args(["--file", "tree.txt"])
        .assert()
        .success();

    assert!(dir.path().join("a.rs").is_file());
    assert!(dir.path().join("b.rs").is_file());
}
