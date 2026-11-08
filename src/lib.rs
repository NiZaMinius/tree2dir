//! tree2dir library surface.
//!
//! Exposes the internal modules so that integration tests (in `tests/`) and
//! any downstream consumers can import types such as [`parser::parse_tree`]
//! and [`parser::EntryKind`] without going through `main.rs`.

pub mod cli;
pub mod comments;
pub mod error;
pub mod fs_builder;
mod input;
pub mod parser;
pub mod tui;

use std::{
    io::{self, Read},
    path::PathBuf,
    time::Instant,
};

use anyhow::Context;
use clap::Parser;
use is_terminal::IsTerminal;

use crate::{
    cli::Cli,
    fs_builder::{build, print_summary, BuildOptions},
    parser::parse_tree,
};

/// The main application logic.
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Resolve the raw ASCII tree text from the appropriate source.
    let Some(raw_input) = read_input(&cli)? else {
        println!("Cancelled. No changes were made.");
        return Ok(());
    };

    // Parse the raw text into a node list.
    let nodes = parse_tree(&raw_input);

    if nodes.is_empty() {
        anyhow::bail!(
            "No recognisable entries found in the provided input. \
             Ensure the text is a valid ASCII / Unicode directory tree."
        );
    }

    // Build options derived from CLI flags.
    let opts = BuildOptions {
        output_root: cli.output.clone(),
        dry_run: cli.dry_run,
        force: cli.force,
        comments: cli.comment_mode(),
    };

    if cli.dry_run {
        println!("Dry-run mode — nothing will be written to disk.\n");
    }

    let start = Instant::now();
    let stats = build(&nodes, &opts)
        .with_context(|| format!("Failed to generate structure in `{}`", cli.output.display()))?;

    let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
    print_summary(&nodes, &cli.output, &stats, elapsed_ms, cli.dry_run);

    Ok(())
}

// ---------------------------------------------------------------------------
// Input resolution
// ---------------------------------------------------------------------------

/// Determine which execution mode to use and return the raw tree string.
fn read_input(cli: &Cli) -> anyhow::Result<Option<String>> {
    // ── File Mode ─────────────────────────────────────────────────────────────
    if let Some(path) = &cli.file {
        return read_file(path).map(Some);
    }

    // ── Pipe Mode ─────────────────────────────────────────────────────────────
    if !io::stdin().is_terminal() {
        return read_stdin().map(Some);
    }

    // ── Plain-text mode (--no-tui) ────────────────────────────────────────────
    if cli.no_tui {
        // In plain mode without a file or piped data there is nothing to read.
        anyhow::bail!(
            "No input provided. Supply a file with --file <PATH> or pipe data via stdin."
        );
    }

    // ── Interactive TUI Mode ──────────────────────────────────────────────────
    tui::run_tui()
}

/// Read `path` and decode it (see [`input::decode`]).
fn read_file(path: &PathBuf) -> anyhow::Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("Could not read file `{}`", path.display()))?;
    input::decode(&bytes).with_context(|| format!("Could not decode file `{}`", path.display()))
}

/// Read all of stdin and decode it (see [`input::decode`]).
fn read_stdin() -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    io::stdin()
        .lock()
        .read_to_end(&mut bytes)
        .context("Failed to read from stdin")?;
    input::decode(&bytes).context("Could not decode the data read from stdin")
}
