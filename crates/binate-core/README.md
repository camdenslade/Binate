# binate-core

The analysis engine behind [Binate](../../README.md), a semantic diff library for validating reproducible Rust builds.

## Usage

```toml
[dependencies]
binate-core = "0.1"
```

```rust
use binate_core::{MmapBinaryProvider, NormalizerChain, DiffConfig, SemanticDiff};

let left  = MmapBinaryProvider::open("left.elf".as_ref())?;
let right = MmapBinaryProvider::open("right.elf".as_ref())?;

let config = DiffConfig {
    normalizer:       NormalizerChain::default(), // masks build-id, timestamps, paths
    show_disasm:      false,
    parallel:         true,
    ignored_sections: vec![],
};

let result = SemanticDiff::compare(&left, &right, &config)?;

if result.identical {
    println!("Reproducible.");
} else {
    for diff in &result.symbol_diffs {
        println!("{}: {} range(s) differ", diff.symbol.name, diff.ranges.len());
        if let Some(loc) = &diff.source_location {
            println!("  {}:{}", loc.file, loc.line.unwrap_or(0));
        }
    }
}
```

## What it does

1. Memory-maps both binaries (`memmap2`), no full read into RAM
2. Normalizes away known non-determinism (build IDs, timestamps, absolute paths)
3. Byte-diffs each section in parallel (`rayon`)
4. Attributes each differing byte range to a symbol via O(log n) address lookup
5. Resolves each changed symbol to a source file and line via DWARF (`gimli`)

## License

MIT
