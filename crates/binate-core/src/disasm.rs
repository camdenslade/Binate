use iced_x86::{Decoder, DecoderOptions, Formatter, IntelFormatter};

/// Disassemble `bytes` starting at virtual address `rip`.
/// Returns one string per decoded instruction in Intel syntax.
pub fn disassemble(bytes: &[u8], rip: u64, bitness: u32) -> Vec<String> {
    let mut decoder = Decoder::with_ip(bitness, bytes, rip, DecoderOptions::NONE);
    let mut formatter = IntelFormatter::new();
    formatter.options_mut().set_digit_separator("");
    formatter.options_mut().set_first_operand_char_index(8);

    let mut output = String::new();
    let mut result = Vec::new();
    for instr in &mut decoder {
        output.clear();
        formatter.format(&instr, &mut output);
        result.push(format!("{:#010x}  {}", instr.ip(), output));
    }
    result
}

/// Map an `object::Architecture` to the bitness required by iced-x86.
/// Returns `None` for non-x86 architectures (disassembly not supported).
pub fn bitness_for(arch: object::Architecture) -> Option<u32> {
    match arch {
        object::Architecture::X86_64 | object::Architecture::X86_64_X32 => Some(64),
        object::Architecture::I386 => Some(32),
        _ => None,
    }
}
