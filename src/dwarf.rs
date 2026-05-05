use crate::binary::BinaryProvider;
use crate::error::Result;
use gimli::{EndianSlice, RunTimeEndian};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SourceLocation {
    pub file: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

type Slice = EndianSlice<'static, RunTimeEndian>;

/// Holds owned DWARF section bytes and a `gimli::Dwarf` view over them.
pub struct DwarfIndex {
    _sections: Arc<DwarfSections>,
    dwarf: gimli::Dwarf<Slice>,
}

struct DwarfSections {
    debug_info:        Vec<u8>,
    debug_abbrev:      Vec<u8>,
    debug_str:         Vec<u8>,
    debug_str_offsets: Vec<u8>,
    debug_line:        Vec<u8>,
    debug_line_str:    Vec<u8>,
    debug_ranges:      Vec<u8>,
    debug_rnglists:    Vec<u8>,
    debug_addr:        Vec<u8>,
    endian:            RunTimeEndian,
}

impl DwarfIndex {
    pub fn load(provider: &dyn BinaryProvider) -> Result<Self> {
        let endian = if is_big_endian(provider.architecture()) {
            RunTimeEndian::Big
        } else {
            RunTimeEndian::Little
        };

        let load = |name: &str| -> Vec<u8> {
            provider
                .section_data(name)
                .map(|(data, _, _)| data.to_vec())
                .unwrap_or_default()
        };

        let sections = Arc::new(DwarfSections {
            debug_info:        load(".debug_info"),
            debug_abbrev:      load(".debug_abbrev"),
            debug_str:         load(".debug_str"),
            debug_str_offsets: load(".debug_str_offsets"),
            debug_line:        load(".debug_line"),
            debug_line_str:    load(".debug_line_str"),
            debug_ranges:      load(".debug_ranges"),
            debug_rnglists:    load(".debug_rnglists"),
            debug_addr:        load(".debug_addr"),
            endian,
        });

        // SAFETY: `_sections` keeps the Vec<u8> buffers alive for the entire
        // lifetime of `DwarfIndex`. We transmute a lifetime-bound slice to
        // `'static` so gimli can borrow it. This is the standard pattern used
        // by gimli's own examples when combining mmap/owned data with Dwarf<>.
        let dwarf = unsafe {
            let s = &*Arc::as_ptr(&sections);
            let mk = |v: &Vec<u8>| -> Slice {
                let raw: &'static [u8] = std::slice::from_raw_parts(v.as_ptr(), v.len());
                EndianSlice::new(raw, endian)
            };

            let loader = |section: gimli::SectionId| -> std::result::Result<Slice, gimli::Error> {
                let slice = match section {
                    gimli::SectionId::DebugInfo       => mk(&s.debug_info),
                    gimli::SectionId::DebugAbbrev     => mk(&s.debug_abbrev),
                    gimli::SectionId::DebugStr        => mk(&s.debug_str),
                    gimli::SectionId::DebugStrOffsets => mk(&s.debug_str_offsets),
                    gimli::SectionId::DebugLine       => mk(&s.debug_line),
                    gimli::SectionId::DebugLineStr    => mk(&s.debug_line_str),
                    gimli::SectionId::DebugRanges     => mk(&s.debug_ranges),
                    gimli::SectionId::DebugRngLists   => mk(&s.debug_rnglists),
                    gimli::SectionId::DebugAddr       => mk(&s.debug_addr),
                    _ => EndianSlice::new(&[], endian),
                };
                Ok(slice)
            };

            gimli::Dwarf::load(loader)?
        };

        Ok(Self { _sections: sections, dwarf })
    }

    /// Map a virtual address to a source location via DWARF line programs.
    pub fn resolve(&self, address: u64) -> Result<Option<SourceLocation>> {
        let mut units = self.dwarf.units();
        while let Some(header) = units.next()? {
            let unit = self.dwarf.unit(header)?;

            if !unit_may_contain(&self.dwarf, &unit, address)? {
                continue;
            }

            if let Some(program) = unit.line_program.clone() {
                let comp_dir = unit.comp_dir.clone();
                let (program, sequences) = program.sequences()?;
                for seq in &sequences {
                    if address < seq.start || address >= seq.end {
                        continue;
                    }
                    let mut rows = program.resume_from(seq);
                    let mut best: Option<SourceLocation> = None;
                    while let Some((header, row)) = rows.next_row()? {
                        if row.address() > address {
                            break;
                        }
                        if let Some(file) = row.file(header) {
                            if let Ok(filename) = file_to_string(&self.dwarf, &unit, file, comp_dir.as_ref()) {
                                best = Some(SourceLocation {
                                    file: filename,
                                    line: row.line().map(|n| n.get() as u32),
                                    column: match row.column() {
                                        gimli::ColumnType::LeftEdge => Some(0),
                                        gimli::ColumnType::Column(c) => Some(c.get() as u32),
                                    },
                                });
                            }
                        }
                    }
                    if let Some(loc) = best {
                        return Ok(Some(loc));
                    }
                }
            }
        }
        Ok(None)
    }
}

fn unit_may_contain(
    dwarf: &gimli::Dwarf<Slice>,
    unit: &gimli::Unit<Slice>,
    address: u64,
) -> Result<bool> {
    let mut cursor = unit.entries();
    if let Some((_, entry)) = cursor.next_dfs()? {
        // DW_AT_ranges
        if let Some(val) = entry.attr_value(gimli::DW_AT_ranges)? {
            let _offset = match val {
                gimli::AttributeValue::RangeListsRef(o) => {
                    let mut ranges = dwarf.ranges(unit, gimli::RangeListsOffset(o.0))?;
                    while let Some(range) = ranges.next()? {
                        if address >= range.begin && address < range.end {
                            return Ok(true);
                        }
                    }
                    return Ok(false);
                }
                gimli::AttributeValue::SecOffset(o) => {
                    let offset = gimli::RangeListsOffset(o as usize);
                    let mut ranges = dwarf.ranges(unit, offset)?;
                    while let Some(range) = ranges.next()? {
                        if address >= range.begin && address < range.end {
                            return Ok(true);
                        }
                    }
                    return Ok(false);
                }
                _ => return Ok(true),
            };
        }

        // DW_AT_low_pc / DW_AT_high_pc fallback
        let low  = entry.attr_value(gimli::DW_AT_low_pc)?;
        let high = entry.attr_value(gimli::DW_AT_high_pc)?;
        if let Some(gimli::AttributeValue::Addr(lo)) = low {
            let hi = match high {
                Some(gimli::AttributeValue::Addr(a)) => a,
                Some(gimli::AttributeValue::Udata(offset)) => lo + offset,
                _ => return Ok(true),
            };
            return Ok(address >= lo && address < hi);
        }
    }
    Ok(true)
}

fn file_to_string(
    dwarf: &gimli::Dwarf<Slice>,
    unit: &gimli::Unit<Slice>,
    file: &gimli::FileEntry<Slice, usize>,
    comp_dir: Option<&Slice>,
) -> std::result::Result<String, gimli::Error> {
    let mut path = String::new();

    if let Some(dir) = comp_dir {
        let s = dir.to_string_lossy();
        if !s.is_empty() {
            path.push_str(&s);
            if !path.ends_with('/') {
                path.push('/');
            }
        }
    }

    if let Some(lp) = unit.line_program.as_ref() {
        if let Some(dir) = file.directory(lp.header()) {
            let dir_str = dwarf.attr_string(unit, dir)?;
            let s = dir_str.to_string_lossy();
            let s = s.trim_end_matches('/');
            if !s.is_empty() && !path.trim_end_matches('/').ends_with(s) {
                path.push_str(s);
                path.push('/');
            }
        }
    }

    let name = dwarf.attr_string(unit, file.path_name())?;
    path.push_str(&name.to_string_lossy());
    Ok(path)
}

fn is_big_endian(arch: object::Architecture) -> bool {
    matches!(
        arch,
        object::Architecture::PowerPc
            | object::Architecture::PowerPc64
            | object::Architecture::Mips
            | object::Architecture::Sparc64
    )
}
