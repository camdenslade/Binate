<p align="center">
  <img src="Binate.png" alt="Binate" width="320" />
</p>

<h1 align="center">Binate</h1>

<p align="center">
  A semantic diff tool for validating reproducible Rust builds.
</p>

---

Binate compares two Rust binaries and tells you whether they are truly identical, not just byte-for-byte, but semantically. It masks known sources of non-determinism (build IDs, timestamps, absolute paths) before comparing, so you only see genuine differences. When it finds one, it maps it back to the source symbol and DWARF file/line location.

## Install

```sh
cargo install --path .
```

## Usage

```sh
# Compare two builds (exit 0 if identical after normalization)
binate compare left.elf right.elf

# Show disassembly diff for changed symbols (x86/x86-64)
binate compare left.elf right.elf --disasm

# Machine-readable output for CI
binate compare left.elf right.elf --format json
binate compare left.elf right.elf --format sarif

# Fail with exit code 1 if any differences are found (CI gate)
binate compare left.elf right.elf --strict

# List sections
binate inspect ./target/debug/mybinary

# List symbols
binate symbols ./target/debug/mybinary
```

## How it works

Binate runs a five-stage pipeline:

1. **Load**: memory-maps both binaries via `memmap2`, no full read into RAM
2. **Normalize**: masks build IDs, timestamps, linker version strings, and absolute paths before comparison
3. **Diff**: O(n) linear scan over each section in parallel (`rayon`), identical sections are skipped instantly
4. **Attribute**: maps each differing byte range to a symbol via a `BTreeMap`-indexed symbol table
5. **Enrich**: resolves each changed symbol to a source file and line number using DWARF debug info (`gimli`)

## Normalizers

| Name | What it masks |
|---|---|
| `build-id` | `.note.gnu.build-id`, Mach-O LC_UUID |
| `timestamp` | 4-byte Unix timestamps in `.comment` and PE COFF headers |
| `absolute-path` | Host-absolute paths in `.debug_str` / `.debug_line_str` |
| `linker-version` | Linker version banners in `.comment` |

Skip individual normalizers with `--skip-normalizer <name>`, or disable all with `--no-normalize`.

## Output formats

| Format | Use case |
|---|---|
| `terminal` | Human-readable, colored diff (default) |
| `json` | Structured output for scripting |
| `sarif` | GitHub Actions / CI annotation integration |

## Dependencies

| Crate | Role |
|---|---|
| `object` | Binary parsing (ELF, Mach-O, PE) |
| `gimli` | DWARF debug-info traversal |
| `iced-x86` | x86/x86-64 disassembly |
| `memmap2` | Memory-mapped file access |
| `rayon` | Parallel section analysis |
| `similar` | Disassembly text diff in terminal output |
| `clap` | CLI argument parsing |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Identical (or differences found without `--strict`) |
| `1` | Differences found under `--strict` |
| `2` | Fatal error |
