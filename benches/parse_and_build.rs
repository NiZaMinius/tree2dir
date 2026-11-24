//! Parser and builder benchmarks.
//!
//! Parsing and disk I/O are measured separately: `parse_tree` is pure CPU, so
//! it is benchmarked on its own; `build` into a fresh temporary directory
//! measures the filesystem and is only meaningful per machine and disk.
//!
//! The dry run and the "everything already exists" paths print a listing or a
//! warning per entry, which would drown the report here; the end-to-end script
//! `benches/e2e.sh` measures them with the output discarded.
//!
//! Run with `cargo bench`. See `benches/README.md` for method and results.

use std::{fmt::Write, hint::black_box, time::Duration};

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use tempfile::TempDir;
use tree2dir::{
    comments::CommentMode,
    fs_builder::{build, BuildOptions},
    parser::parse_tree,
};

const SIZES: [usize; 4] = [10, 100, 1_000, 10_000];

/// A balanced tree of roughly `lines` entries: directories of ten children,
/// drawn with box-drawing glyphs like `tree` does.
fn glyph_tree(lines: usize, comments: bool) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut dir = 0;
    while count < lines {
        let note = if comments { " # a directory" } else { "" };
        let _ = writeln!(out, "├── dir{dir}/{note}");
        count += 1;
        for file in 0..10 {
            if count >= lines {
                break;
            }
            let connector = if file == 9 { "└──" } else { "├──" };
            let note = if comments { " # source file" } else { "" };
            let _ = writeln!(out, "│   {connector} file{file}.rs{note}");
            count += 1;
        }
        dir += 1;
    }
    out
}

/// The same shape as Windows `tree /f /a` prints it, header and drive line
/// included.
fn windows_ascii_tree(lines: usize) -> String {
    let mut out = String::from(
        "Folder PATH listing for volume Data\nVolume serial number is 1234-5678\nD:.\n",
    );
    let mut count = 0;
    let mut dir = 0;
    while count < lines {
        let _ = writeln!(out, "+---dir{dir}");
        count += 1;
        for file in 0..10 {
            if count >= lines {
                break;
            }
            let _ = writeln!(out, "|       file{file}.rs");
            count += 1;
        }
        dir += 1;
    }
    out
}

/// Tab indentation only, four levels deep.
fn tab_tree(lines: usize) -> String {
    let mut out = String::new();
    for i in 0..lines {
        let depth = i % 4;
        let kind = if depth < 3 { "d" } else { "f" };
        let _ = writeln!(out, "{}{kind}{i}", "\t".repeat(depth));
    }
    out
}

/// Multi-byte names and glyphs: every `│` is three bytes.
fn unicode_tree(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("│   ├── 日本語ファイル{i}.md\n"))
        .collect()
}

/// A chain of nested directories, as deep as it is long.
fn deep_tree(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("{}d{i}/\n", " ".repeat(i * 2)))
        .collect()
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_tree");
    for size in SIZES {
        let inputs = [
            ("glyphs", glyph_tree(size, false)),
            ("glyphs_with_comments", glyph_tree(size, true)),
            ("windows_ascii", windows_ascii_tree(size)),
            ("tabs", tab_tree(size)),
            ("unicode", unicode_tree(size)),
        ];
        for (name, input) in inputs {
            group.throughput(Throughput::Bytes(input.len() as u64));
            group.bench_with_input(BenchmarkId::new(name, size), &input, |b, input| {
                b.iter(|| parse_tree(black_box(input)));
            });
        }
    }
    // The text of a chain grows quadratically with its length (the indentation
    // of line i is 2i spaces), so keep this one small: the point is the shape.
    for size in [10, 100, 1_000] {
        let input = deep_tree(size);
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("deeply_nested", size),
            &input,
            |b, input| {
                b.iter(|| parse_tree(black_box(input)));
            },
        );
    }
    group.finish();
}

fn bench_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_to_disk");
    // One iteration of the largest size takes a second or more on a slow
    // filesystem; fewer samples keep criterion from warning that it cannot
    // finish within its target time.
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    for size in [10, 100, 1_000] {
        let nodes = parse_tree(&glyph_tree(size, true));
        for (name, comments) in [
            ("no_comments", CommentMode::None),
            ("all_comments", CommentMode::All),
        ] {
            group.bench_with_input(BenchmarkId::new(name, size), &nodes, |b, nodes| {
                b.iter_batched(
                    || TempDir::new().unwrap(),
                    |dir| {
                        let opts = BuildOptions {
                            output_root: dir.path().to_path_buf(),
                            comments,
                            ..Default::default()
                        };
                        build(nodes, &opts).unwrap();
                        // Dropped, and so deleted, outside the timed section.
                        dir
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_parse, bench_build);
criterion_main!(benches);
