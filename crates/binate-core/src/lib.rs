mod binary;
mod diff;
mod disasm;
mod dwarf;
mod error;
mod normalize;
mod symbol;

pub use binary::{BinaryProvider, MmapBinaryProvider, SectionInfo};
pub use diff::{AnonymousDiff, ByteRange, DiffConfig, DiffResult, SemanticDiff, SymbolDiff};
pub use dwarf::SourceLocation;
pub use error::{BinateError, Result};
pub use normalize::{
    AbsolutePathNormalizer, BuildIdNormalizer, LinkerVersionNormalizer, Normalizer,
    NormalizerChain, TimestampNormalizer,
};
pub use symbol::ResolvedSymbol;
