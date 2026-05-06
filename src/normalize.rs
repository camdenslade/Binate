use std::borrow::Cow;

pub trait Normalizer: Send + Sync {
    fn name(&self) -> &'static str;
    fn normalize<'a>(&self, section: &str, data: &'a [u8]) -> Cow<'a, [u8]>;
}

pub struct NormalizerChain {
    inner: Vec<Box<dyn Normalizer>>,
}

impl NormalizerChain {
    pub fn new() -> Self {
        Self { inner: Vec::new() }
    }

    pub fn push(mut self, n: impl Normalizer + 'static) -> Self {
        self.inner.push(Box::new(n));
        self
    }

    pub fn push_boxed(mut self, n: Box<dyn Normalizer>) -> Self {
        self.inner.push(n);
        self
    }

    pub fn apply<'a>(&self, section: &str, data: &'a [u8]) -> Cow<'a, [u8]> {
        let mut result: Cow<'a, [u8]> = Cow::Borrowed(data);
        for norm in &self.inner {
            match norm.normalize(section, &result) {
                Cow::Borrowed(_) => {}
                Cow::Owned(owned) => {
                    result = Cow::Owned(owned);
                }
            }
        }
        result
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.inner.iter().map(|n| n.name()).collect()
    }
}

impl Default for NormalizerChain {
    fn default() -> Self {
        Self::new()
            .push(BuildIdNormalizer)
            .push(TimestampNormalizer)
            .push(AbsolutePathNormalizer::new())
            .push(LinkerVersionNormalizer)
    }
}

pub struct BuildIdNormalizer;

impl Normalizer for BuildIdNormalizer {
    fn name(&self) -> &'static str {
        "build-id"
    }

    fn normalize<'a>(&self, section: &str, data: &'a [u8]) -> Cow<'a, [u8]> {
        if !section.starts_with(".note") && section != "__DATA,__uuid" {
            return Cow::Borrowed(data);
        }
        Cow::Owned(vec![0u8; data.len()])
    }
}

pub struct TimestampNormalizer;

impl Normalizer for TimestampNormalizer {
    fn name(&self) -> &'static str {
        "timestamp"
    }

    fn normalize<'a>(&self, section: &str, data: &'a [u8]) -> Cow<'a, [u8]> {
        if section != ".comment" {
            return Cow::Borrowed(data);
        }
        // Zero out any 4-byte little-endian values that look like Unix timestamps
        // (between 0x3B9ACA00 / 2001-01-01 and 0x7FFFFFFF / 2038-01-19)
        const TS_MIN: u32 = 0x3B9A_CA00;
        const TS_MAX: u32 = 0x7FFF_FFFF;
        let mut out = data.to_vec();
        for i in 0..out.len().saturating_sub(3) {
            let v = u32::from_le_bytes([out[i], out[i + 1], out[i + 2], out[i + 3]]);
            if v >= TS_MIN && v <= TS_MAX {
                out[i..i + 4].fill(0);
            }
        }
        Cow::Owned(out)
    }
}

pub struct AbsolutePathNormalizer {
    pattern: regex::bytes::Regex,
}

impl AbsolutePathNormalizer {
    pub fn new() -> Self {
        // Matches Unix absolute paths like /Users/... or /home/...
        let pattern = regex::bytes::Regex::new(r"/[A-Za-z0-9_.\-]+(?:/[A-Za-z0-9_.\-]+)+")
            .expect("valid regex");
        Self { pattern }
    }
}

impl Normalizer for AbsolutePathNormalizer {
    fn name(&self) -> &'static str {
        "absolute-path"
    }

    fn normalize<'a>(&self, section: &str, data: &'a [u8]) -> Cow<'a, [u8]> {
        if !matches!(section, ".debug_str" | ".debug_line_str" | ".comment" | "__DWARF,__debug_str") {
            return Cow::Borrowed(data);
        }
        let replaced = self.pattern.replace_all(data, b"<PATH>".as_slice());
        match replaced {
            Cow::Borrowed(_) => Cow::Borrowed(data),
            Cow::Owned(v) => Cow::Owned(v),
        }
    }
}

impl Default for AbsolutePathNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LinkerVersionNormalizer;

impl Normalizer for LinkerVersionNormalizer {
    fn name(&self) -> &'static str {
        "linker-version"
    }

    fn normalize<'a>(&self, section: &str, data: &'a [u8]) -> Cow<'a, [u8]> {
        if section != ".comment" {
            return Cow::Borrowed(data);
        }
        // Zero out null-terminated strings that look like linker version banners
        // e.g. "Linker: LLD 18.0.0", "GCC: (GNU) 14.2.0"
        let mut out = data.to_vec();
        let keywords: &[&[u8]] = &[b"Linker:", b"GCC:", b"clang version", b"LLVM"];
        'outer: for i in 0..out.len() {
            for kw in keywords {
                if out[i..].starts_with(kw) {
                    // Zero from i to the next null byte (end of C string)
                    let end = out[i..].iter().position(|&b| b == 0).map(|p| i + p + 1).unwrap_or(out.len());
                    out[i..end].fill(0);
                    continue 'outer;
                }
            }
        }
        Cow::Owned(out)
    }
}
