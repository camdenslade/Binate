use crate::error::Result;
use object::{Object, ObjectSection, ObjectSymbol, SymbolKind};
use serde::Serialize;
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedSymbol {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub section: Option<String>,
}

pub struct SymbolTable {
    by_address: BTreeMap<u64, ResolvedSymbol>,
}

impl SymbolTable {
    pub fn build(file: &object::File<'_>) -> Result<Self> {
        let mut by_address = BTreeMap::new();
        for sym in file.symbols() {
            if matches!(sym.kind(), SymbolKind::Unknown | SymbolKind::Label) {
                continue;
            }
            let address = sym.address();
            if address == 0 {
                continue;
            }
            let name = sym.name().unwrap_or("<unknown>").to_owned();
            let size = sym.size();
            let section = sym
                .section_index()
                .and_then(|idx| file.section_by_index(idx).ok())
                .and_then(|s| s.name().ok().map(|n: &str| n.to_owned()));
            by_address.insert(address, ResolvedSymbol { name, address, size, section });
        }
        Ok(Self { by_address })
    }

    /// Find the symbol whose range contains `address`.
    pub fn symbol_at(&self, address: u64) -> Option<&ResolvedSymbol> {
        self.by_address
            .range(..=address)
            .next_back()
            .map(|(_, sym)| sym)
            .filter(|sym| sym.size == 0 || address < sym.address + sym.size)
    }

    pub fn symbols_in(&self, range: Range<u64>) -> impl Iterator<Item = &ResolvedSymbol> {
        self.by_address
            .range(range.clone())
            .map(|(_, sym)| sym)
            .filter(move |sym| sym.address < range.end)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ResolvedSymbol> {
        self.by_address.values()
    }
}
