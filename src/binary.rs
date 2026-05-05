use crate::error::Result;
use crate::symbol::SymbolTable;
use memmap2::Mmap;
use object::{Architecture, BinaryFormat, Object, ObjectSection, SectionKind};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    file_range: Option<(u64, u64)>,
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
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
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
                        file_range: None,
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
                file_range: Some((offset as u64, size as u64)),
            });
        }

        Ok(Self {
            path: path.to_owned(),
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
