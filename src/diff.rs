use crate::binary::BinaryProvider;
use crate::disasm;
use crate::dwarf::{DwarfIndex, SourceLocation};
use crate::error::Result;
use crate::normalize::NormalizerChain;
use crate::symbol::ResolvedSymbol;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

const MERGE_THRESHOLD: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct ByteRange {
    pub offset: u64,
    pub len: usize,
    pub left_bytes: Vec<u8>,
    pub right_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolDiff {
    pub symbol: ResolvedSymbol,
    pub source_location: Option<SourceLocation>,
    pub ranges: Vec<ByteRange>,
    pub disasm_left: Option<Vec<String>>,
    pub disasm_right: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnonymousDiff {
    pub section: String,
    pub ranges: Vec<ByteRange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffResult {
    pub left_path: PathBuf,
    pub right_path: PathBuf,
    pub identical: bool,
    pub symbol_diffs: Vec<SymbolDiff>,
    pub anonymous_diffs: Vec<AnonymousDiff>,
    pub sections_only_in_left: Vec<String>,
    pub sections_only_in_right: Vec<String>,
}

pub struct DiffConfig {
    pub normalizer: NormalizerChain,
    pub show_disasm: bool,
    pub parallel: bool,
    pub ignored_sections: Vec<String>,
}

pub struct SemanticDiff;

impl SemanticDiff {
    pub fn compare(
        left: &dyn BinaryProvider,
        right: &dyn BinaryProvider,
        config: &DiffConfig,
    ) -> Result<DiffResult> {
        // 1. Enumerate sections
        let mut left_sections: HashMap<String, (Vec<u8>, u64)> = HashMap::new();
        let mut right_sections: HashMap<String, (Vec<u8>, u64)> = HashMap::new();

        left.visit_sections(&mut |name, _kind, addr, data| {
            if !config.ignored_sections.iter().any(|s| s == name) {
                left_sections.insert(name.to_owned(), (data.to_vec(), addr));
            }
            Ok(())
        })?;

        right.visit_sections(&mut |name, _kind, addr, data| {
            if !config.ignored_sections.iter().any(|s| s == name) {
                right_sections.insert(name.to_owned(), (data.to_vec(), addr));
            }
            Ok(())
        })?;

        let sections_only_in_left: Vec<String> = left_sections
            .keys()
            .filter(|k| !right_sections.contains_key(*k))
            .cloned()
            .collect();

        let sections_only_in_right: Vec<String> = right_sections
            .keys()
            .filter(|k| !left_sections.contains_key(*k))
            .cloned()
            .collect();

        let shared: Vec<String> = left_sections
            .keys()
            .filter(|k| right_sections.contains_key(*k))
            .cloned()
            .collect();

        // 2 & 3. Normalize + byte diff + symbol attribution (per section, parallel)
        let left_syms = left.symbol_table();

        let section_results: Vec<Result<(Vec<SymbolDiff>, Vec<AnonymousDiff>)>> = if config.parallel {
            shared
                .par_iter()
                .map(|name| {
                    diff_section(
                        name,
                        &left_sections[name],
                        &right_sections[name],
                        &config.normalizer,
                        left_syms,
                    )
                })
                .collect()
        } else {
            shared
                .iter()
                .map(|name| {
                    diff_section(
                        name,
                        &left_sections[name],
                        &right_sections[name],
                        &config.normalizer,
                        left_syms,
                    )
                })
                .collect()
        };

        let mut symbol_diffs: Vec<SymbolDiff> = Vec::new();
        let mut anonymous_diffs: Vec<AnonymousDiff> = Vec::new();

        for result in section_results {
            let (syms, anons) = result?;
            symbol_diffs.extend(syms);
            anonymous_diffs.extend(anons);
        }

        // 4. DWARF enrichment (lazy)
        if !symbol_diffs.is_empty() {
            if let Ok(dwarf) = DwarfIndex::load(left) {
                for sd in &mut symbol_diffs {
                    sd.source_location = dwarf.resolve(sd.symbol.address).ok().flatten();
                }
            }
        }

        // 5. Optional disassembly
        if config.show_disasm {
            if let Some(bitness) = disasm::bitness_for(left.architecture()) {
                for sd in &mut symbol_diffs {
                    let addr = sd.symbol.address;
                    let size = sd.symbol.size as usize;

                    let left_bytes = left
                        .section_data(sd.symbol.section.as_deref().unwrap_or(".text"))
                        .and_then(|(data, sec_addr, _)| {
                            let off = addr.saturating_sub(sec_addr) as usize;
                            data.get(off..off + size)
                        });

                    let right_bytes = right
                        .section_data(sd.symbol.section.as_deref().unwrap_or(".text"))
                        .and_then(|(data, sec_addr, _)| {
                            let off = addr.saturating_sub(sec_addr) as usize;
                            data.get(off..off + size)
                        });

                    if let Some(lb) = left_bytes {
                        sd.disasm_left = Some(disasm::disassemble(lb, addr, bitness));
                    }
                    if let Some(rb) = right_bytes {
                        sd.disasm_right = Some(disasm::disassemble(rb, addr, bitness));
                    }
                }
            }
        }

        let identical = symbol_diffs.is_empty() && anonymous_diffs.is_empty();

        Ok(DiffResult {
            left_path: left.path().to_path_buf(),
            right_path: right.path().to_path_buf(),
            identical,
            symbol_diffs,
            anonymous_diffs,
            sections_only_in_left,
            sections_only_in_right,
        })
    }
}

fn diff_section(
    name: &str,
    left: &(Vec<u8>, u64),
    right: &(Vec<u8>, u64),
    normalizer: &NormalizerChain,
    syms: &crate::symbol::SymbolTable,
) -> Result<(Vec<SymbolDiff>, Vec<AnonymousDiff>)> {
    let (left_data, left_addr) = left;
    let (right_data, _right_addr) = right;

    let left_norm = normalizer.apply(name, left_data);
    let right_norm = normalizer.apply(name, right_data);

    if left_norm == right_norm {
        return Ok((Vec::new(), Vec::new()));
    }

    let ranges = find_differing_ranges(&left_norm, &right_norm, *left_addr);

    // Group ranges by symbol
    let mut by_symbol: HashMap<u64, (ResolvedSymbol, Vec<ByteRange>)> = HashMap::new();
    let mut anon_ranges: Vec<ByteRange> = Vec::new();

    for range in ranges {
        let addr = range.offset;
        if let Some(sym) = syms.symbol_at(addr) {
            let entry = by_symbol
                .entry(sym.address)
                .or_insert_with(|| (sym.clone(), Vec::new()));
            entry.1.push(range);
        } else {
            anon_ranges.push(range);
        }
    }

    let sym_diffs: Vec<SymbolDiff> = by_symbol
        .into_values()
        .map(|(symbol, ranges)| SymbolDiff {
            symbol,
            source_location: None,
            ranges,
            disasm_left: None,
            disasm_right: None,
        })
        .collect();

    let anon_diffs = if anon_ranges.is_empty() {
        Vec::new()
    } else {
        vec![AnonymousDiff { section: name.to_owned(), ranges: anon_ranges }]
    };

    Ok((sym_diffs, anon_diffs))
}

/// O(n) linear scan producing `ByteRange`s at virtual addresses.
/// Runs of identical bytes >= MERGE_THRESHOLD end the current differing range.
fn find_differing_ranges(left: &[u8], right: &[u8], base_addr: u64) -> Vec<ByteRange> {
    let len = left.len().min(right.len());
    let mut ranges: Vec<ByteRange> = Vec::new();
    let mut i = 0;

    while i < len {
        if left[i] == right[i] {
            i += 1;
            continue;
        }
        // Start of a differing run
        let start = i;
        i += 1;
        while i < len {
            if left[i] == right[i] {
                // Check if we have MERGE_THRESHOLD identical bytes ahead
                let run_end = (i + MERGE_THRESHOLD).min(len);
                if left[i..run_end] == right[i..run_end] {
                    break;
                }
            }
            i += 1;
        }
        let end = i;
        ranges.push(ByteRange {
            offset: base_addr + start as u64,
            len: end - start,
            left_bytes: left[start..end].to_vec(),
            right_bytes: right[start..end].to_vec(),
        });
    }

    // Handle trailing extra bytes if sizes differ
    if left.len() != right.len() {
        let extra_start = len;
        let left_extra = &left[extra_start..];
        let right_extra = &right[extra_start..];
        if !left_extra.is_empty() || !right_extra.is_empty() {
            ranges.push(ByteRange {
                offset: base_addr + extra_start as u64,
                len: left_extra.len().max(right_extra.len()),
                left_bytes: left_extra.to_vec(),
                right_bytes: right_extra.to_vec(),
            });
        }
    }

    ranges
}
