# Benchmarks

Numbers a reader can reproduce. Nothing here is a promise about *your* machine:
parsing is CPU-bound, building is filesystem-bound, and the two are measured
separately.

## How to run

```bash
cargo bench                                      # criterion: parse_tree and build
cargo build --release && benches/e2e.sh 1000     # hyperfine, needs `hyperfine`
```

`benches/gen_tree.sh N` prints a tree of about `N` entries (directories of ten
files); `benches/e2e.sh` uses it.

## What is measured

| Benchmark | Input | Measures |
|---|---|---|
| `parse_tree/glyphs/N` | `├──` / `│` tree, N = 10, 100, 1 000, 10 000 lines | parser only, no I/O |
| `parse_tree/glyphs_with_comments/N` | the same, every line has a `# comment` | cost of comment splitting |
| `parse_tree/windows_ascii/N` | Windows `tree /f /a` output with header and drive line | ASCII connectors and header handling |
| `parse_tree/tabs/N` | tab indentation | |
| `parse_tree/unicode/N` | multi-byte names and glyphs | depth counted in scalar values |
| `parse_tree/deeply_nested/N` | a chain of N nested directories (N ≤ 1 000) | the *text* grows as N², so time grows faster than N |
| `build_to_disk/{no,all}_comments/N` | N = 10, 100, 1 000 into a fresh temp directory | real file creation; the directory is deleted outside the timed section |
| `e2e.sh`, 1 | N entries into a new directory | whole process (start-up, read, parse, build) against a shell script doing `mkdir -p` and `: >` for the same structure |
| `e2e.sh`, 2 | `--dry-run` on a tree that is already built | the disk checks and the listing, output discarded |
| `e2e.sh`, 3 | a second run on a built tree | every file exists and is skipped, warnings discarded |

The "Done in X ms" line the tool prints measures only the build step, not
process start-up.

## Machines and method

Two machines, neither of them a reference one.

**Linux:** a single virtual machine: 1 vCPU (Intel Xeon @ 2.30 GHz), 4 GB RAM,
Ubuntu 24.04, ext4 on a virtual disk, Rust 1.91, hyperfine 1.18. Criterion was run with shortened times
(`--warm-up-time 1 --measurement-time 3`), so its intervals are wider than in a
default run.

**Windows:** the maintainer's own machine, a release build, `cargo bench` with
criterion's default times; the hardware was not recorded and the end-to-end
script was not run there.

Re-run on your own hardware before quoting any figure; on Windows
or macOS, and on other disks, the build and end-to-end numbers will differ a lot.

## Results: Linux VM

`parse_tree`, middle value of the criterion interval:

| Input | 10 lines | 100 | 1 000 | 10 000 |
|---|---:|---:|---:|---:|
| glyphs | 1.2 µs | 11.2 µs | 118.6 µs | 1.46 ms |
| glyphs with comments | 1.7 µs | 19.3 µs | 168.1 µs | 2.37 ms |
| windows ascii | 1.3 µs | 11.1 µs | 106.6 µs | 1.12 ms |
| tabs | 798 ns | 8.9 µs | 77.0 µs | 882.0 µs |
| unicode | 1.4 µs | 13.4 µs | 135.9 µs | 1.54 ms |

| Chain depth | 10 | 100 | 1 000 |
|---|---:|---:|---:|
| deeply nested | 1.3 µs | 40.7 µs | 2.28 ms |

Parsing scales linearly with the number of lines (about 11× per 10× lines).
Comments add roughly 40 % to the parse time of a line.

End to end, whole process, output discarded (hyperfine, mean ± σ):

| Scenario | 1 000 entries | 8 500 entries |
|---|---:|---:|
| `--dry-run` on a built tree | 3.8 ± 0.3 ms | 28.0 ± 4.1 ms |
| second run, every file exists | 3.9 ± 0.5 ms | 27.7 ± 2.0 ms |
| build into a new directory, tree2dir | 11.5 ± 6.8 ms | 825 ± 612 ms |
| build into a new directory, shell script | 136.5 ± 8.0 ms | 3.075 ± 0.136 s |

The two build rows are noisy on this machine: the large σ is real, and a repeat
of the same measurement gave means about a factor of two apart. Read them as a
range, not as a point. The dry-run and second-run rows are stable.

## Results: Windows

`parse_tree`, middle value of the criterion interval (the 10 000-line column is
incomplete in the captured output; `glyphs` took 4.54 ms there):

| Input | 10 lines | 100 | 1 000 |
|---|---:|---:|---:|
| glyphs | 5.97 µs | 56.2 µs | 466 µs |
| glyphs with comments | 9.61 µs | 93.7 µs | 880 µs |
| windows ascii | 6.25 µs | 50.6 µs | 441.8 µs |
| tabs | 5.82 µs | 41.6 µs | 362.8 µs |
| unicode | 7.61 µs | 65.5 µs | 520 µs |

| Chain depth | 10 | 100 | 1 000 |
|---|---:|---:|---:|
| deeply nested | 5.51 µs | 102 µs | 6.76 ms |

`build_to_disk`:

| Entries | no comments | all comments |
|---|---:|---:|
| 10 | 8.99 ms | 11.2 ms |
| 100 | 61.5 ms | 108.9 ms |
| 1 000 | 678 ms | 1.138 s |

## Reading the numbers

- **Parsing is about four times slower on the Windows machine** (466 µs against
  119 µs for 1 000 glyph lines). The hardware differs, so this says nothing
  about the operating system by itself; a thousand lines still parse in well
  under a millisecond on both.
- **Creating files is far more expensive on Windows**: about 0.7 ms per entry
  in `build_to_disk` (678 ms for 1 000), against roughly 0.01 ms per entry
  end to end on the Linux VM. There, writing content into each file costs about
  70 % more than creating it empty (1.14 s against 0.68 s for 1 000 entries),
  whereas the end-to-end time on Linux differed by 14 %. The Windows figures
  come from the criterion harness alone, so the harness effect described below
  may inflate them too.
- **Parsing is not where the time goes.** A thousand lines parse in about a tenth
  of a millisecond; creating the files takes far longer. At 8 500 entries about
  95 % of a build's time is system time spent in the filesystem (785.9 ms of
  825.1 ms), against 28.3 ms of user time.
- **A build is not linear in the number of files.** Going from 1 000 to 8 500
  entries multiplied the time by far more than 8.5, for the tool and for the
  shell script alike, because the filesystem itself slows down.
- **Checking what exists is cheap.** A dry run or a second run over 8 500 existing
  entries takes under 30 ms here.
- **`build_to_disk` is only reliable for small sizes.** On the Linux VM at 1 000 entries it
  reported 57 ms without comments and 212 ms with them, but the whole process
  measured with a plain loop (median of 5) takes 8.0 ms and 9.1 ms. The gap
  comes from the harness, most likely from the previous iteration's directory
  being deleted while the next one runs; it was not investigated further. Use
  the end-to-end numbers for anything larger.

Do not quote a speed figure that these benchmarks do not show.
