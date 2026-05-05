use crate::error::Result;
use crate::symbol::SymbolTable;
use memmap2::Mmap;
use object::{Architecture, BinaryFormat, Object, ObjectSection, SectionKind};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// An anonymous in-memory path label used when constructing from raw bytes.
const IN_MEMORY_PATH: &str = "<in-memory>";

pub struct SectionInfo {
    pub name: String,
    pub kind: SectionKind,
    pub address: u64,
    pub file_range: Option<(u64, u64)>,
}

pub trait BinaryProvider: Send + Sync {
    fn format(&self) -> BinaryFormat;
    fn architecture(&self) -> Architecture;
    /// Returns (data, base_address, kind) for a named section.
    fn section_data(&self, name: &str) -> Option<(&[u8], u64, SectionKind)>;
    fn symbol_table(&self) -> &SymbolTable;
    fn path(&self) -> &Path;
    fn sections(&self) -> &[SectionInfo];
    /// Iterate over all sections yielding (name, kind, address, data).
    fn visit_sections(&self, visitor: &mut dyn FnMut(&str, SectionKind, u64, &[u8]) -> Result<()>) -> Result<()>;
}

struct ParsedSection {
    name: String,
    kind: SectionKind,
    address: u64,
    offset: usize,
    size: usize,
}

pub struct MmapBinaryProvider {
    path: PathBuf,
    mmap: Arc<Mmap>,
    sections: Vec<ParsedSection>,
    symbols: SymbolTable,
    format: BinaryFormat,
    arch: Architecture,
    section_infos: Vec<SectionInfo>,
}

impl MmapBinaryProvider {
    /// Open a binary from a file path. The file is memory-mapped; no full read occurs.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        Self::from_mmap(mmap, path.to_owned())
    }

    /// Parse a binary from raw bytes already in memory.
    /// The path is recorded as `<in-memory>` in `DiffResult`.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        // Copy into an anonymous mmap so we can hand out `&[u8]` slices safely.
        let mut mmap = memmap2::MmapMut::map_anon(bytes.len().max(1))?;
        mmap[..bytes.len()].copy_from_slice(bytes);
        let mmap = mmap.make_read_only()?;
        Self::from_mmap(mmap, PathBuf::from(IN_MEMORY_PATH))
    }

    fn from_mmap(mmap: Mmap, path: PathBuf) -> Result<Self> {
        let obj = object::File::parse(&*mmap)?;

        let format = obj.format();
        let arch = obj.architecture();
        let symbols = SymbolTable::build(&obj)?;

        let mut sections = Vec::new();
        let mut section_infos = Vec::new();

        for section in obj.sections() {
            let name = section.name().unwrap_or("").to_owned();
            let kind = section.kind();
            let address = section.address();
            let file_range = section.file_range();

            let (offset, size) = match file_range {
                Some((off, sz)) => (off as usize, sz as usize),
                None => {
                    section_infos.push(SectionInfo {
                        name: name.clone(),
                        kind,
                        address,
                        file_range: None,
                    });
                    sections.push(ParsedSection {
                        name,
                        kind,
                        address,
                        offset: 0,
                        size: 0,
                    });
                    continue;
                }
            };

            section_infos.push(SectionInfo {
                name: name.clone(),
                kind,
                address,
                file_range: Some((offset as u64, size as u64)),
            });
            sections.push(ParsedSection {
                name,
                kind,
                address,
                offset,
                size,
            });
        }

        Ok(Self {
            path,
            mmap: Arc::new(mmap),
            sections,
            symbols,
            format,
            arch,
            section_infos,
        })
    }
}

impl BinaryProvider for MmapBinaryProvider {
    fn format(&self) -> BinaryFormat {
        self.format
    }

    fn architecture(&self) -> Architecture {
        self.arch
    }

    fn section_data(&self, name: &str) -> Option<(&[u8], u64, SectionKind)> {
        self.sections.iter().find(|s| s.name == name).and_then(|s| {
            if s.size == 0 {
                return None;
            }
            let end = s.offset + s.size;
            if end > self.mmap.len() {
                return None;
            }
            Some((&self.mmap[s.offset..end], s.address, s.kind))
        })
    }

    fn visit_sections(&self, visitor: &mut dyn FnMut(&str, SectionKind, u64, &[u8]) -> Result<()>) -> Result<()> {
        for s in &self.sections {
            if s.size == 0 {
                continue;
            }
            let end = s.offset + s.size;
            if end > self.mmap.len() {
                continue;
            }
            visitor(&s.name, s.kind, s.address, &self.mmap[s.offset..end])?;
        }
        Ok(())
    }

    fn symbol_table(&self) -> &SymbolTable {
        &self.symbols
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn sections(&self) -> &[SectionInfo] {
        &self.section_infos
    }
}
