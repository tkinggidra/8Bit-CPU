//! The assembler: turns assembly text into machine code.
//!
//! # Syntax
//!
//! ```text
//! ; comments start with a semicolon and run to the end of the line
//! start:                  ; a label on its own line...
//! loop: out r0            ; ...or in front of an instruction
//!         add r0, r1, r0  ; one instruction per line, operands split by commas
//!         jz r3, done     ; labels can be used before they are defined
//! data:   .byte 1, 2, 0xff, start   ; raw bytes (numbers or labels)
//! ```
//!
//! - Numbers: decimal `10`, hex `0x0a`, binary `0b1010`. `ldi` also accepts
//!   -128 to -1, stored as two's complement (so `ldi r0, -1` loads 255).
//! - Mnemonics, registers and number prefixes ignore case. Labels don't:
//!   `Loop` and `loop` are different labels.
//! - Every instruction is 2 bytes and must start at an even address, so after
//!   an odd number of `.byte`s, add a padding byte before the next instruction.
//! - Jump targets must be even, because no instruction starts at an odd one.
//!
//! Every error in a file is reported in one go, each with its line number.

use isa::{AluOp, INSTR_SIZE, Instr, Kind, MEM_SIZE, Reg};
use std::collections::HashMap;
use std::fmt;

/// One problem found in the source, tied to the line it was found on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AsmError {
    /// Line number, counting from 1.
    pub line: usize,
    /// What went wrong, in plain words.
    pub message: String,
}

impl fmt::Display for AsmError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for AsmError {}

/// One line of the listing: where some bytes ended up and the source they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListingLine {
    /// Address of the first byte.
    pub addr: u8,
    /// The bytes this line produced.
    pub bytes: Vec<u8>,
    /// The source line, with its comment and surrounding spaces removed.
    pub source: String,
}

/// A successfully assembled program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// The machine code, starting at address 0. Never longer than 256 bytes.
    pub bytes: Vec<u8>,
    /// Which source line produced which bytes, for printing.
    pub listing: Vec<ListingLine>,
}

/// Assembles a whole source file. Returns every error found, sorted by line.
pub fn assemble(source: &str) -> Result<Program, Vec<AsmError>> {
    // Windows Notepad sometimes puts an invisible "byte order mark" at the start.
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);

    let mut errors = Vec::new();
    let (items, labels) = first_pass(source, &mut errors);

    // Pass 2: now that every label's address is known, turn each line into bytes.
    let mut bytes = Vec::new();
    let mut listing = Vec::new();
    for item in &items {
        match encode_item(item, &labels) {
            // Only build output while it's still possible to succeed; after
            // an error, keep going just to find more errors. When there are
            // no errors, every address is below 256, so `try_from` succeeds.
            Ok(item_bytes) => {
                if let (true, Ok(addr)) = (errors.is_empty(), u8::try_from(item.addr)) {
                    listing.push(ListingLine {
                        addr,
                        bytes: item_bytes.clone(),
                        source: item.text.to_string(),
                    });
                    bytes.extend(item_bytes);
                }
            }
            Err(message) => errors.push(AsmError { line: item.line, message }),
        }
    }

    if errors.is_empty() {
        Ok(Program { bytes, listing })
    } else {
        errors.sort_by_key(|e| e.line);
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// Pass 1: split lines up, place everything at an address, collect labels
// ---------------------------------------------------------------------------

/// What a line asks for, with its operands still as text.
enum Body<'a> {
    Instruction { mnemonic: &'a str, args: Vec<&'a str> },
    Bytes(Vec<&'a str>),
}

/// One line that produces bytes.
struct Item<'a> {
    line: usize,
    addr: usize,
    text: &'a str,
    body: Body<'a>,
}

/// A label's address, and the line it was defined on (for duplicate errors).
struct Label {
    addr: usize,
    line: usize,
}

fn first_pass<'a>(
    source: &'a str,
    errors: &mut Vec<AsmError>,
) -> (Vec<Item<'a>>, HashMap<&'a str, Label>) {
    let mut items = Vec::new();
    let mut labels: HashMap<&str, Label> = HashMap::new();
    let mut addr = 0usize;
    let mut reported_overflow = false;

    // `lines()` also strips the '\r' of Windows line endings.
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let mut error = |message: String| errors.push(AsmError { line, message });

        let mut text = strip_comment(raw).trim();

        // A label is anything before a ':'. Nothing else uses ':', so this is safe.
        if let Some((name, rest)) = text.split_once(':') {
            if rest.contains(':') {
                error("only one label per line".to_string());
                continue;
            }
            let name = name.trim();
            match check_label_name(name) {
                Err(message) => error(message),
                Ok(()) => match labels.get(name) {
                    Some(first) => error(format!(
                        "label '{name}' is already defined on line {}",
                        first.line
                    )),
                    None => {
                        labels.insert(name, Label { addr, line });
                    }
                },
            }
            text = rest.trim();
        }

        if text.is_empty() {
            continue;
        }

        let (head, rest) = text
            .split_once(char::is_whitespace)
            .unwrap_or((text, ""));
        let args = match split_operands(rest) {
            Ok(args) => args,
            Err(message) => {
                error(message);
                continue;
            }
        };

        let (body, size) = if head.eq_ignore_ascii_case(".byte") {
            if args.is_empty() {
                error("'.byte' needs at least one value".to_string());
                continue;
            }
            let size = args.len();
            (Body::Bytes(args), size)
        } else if head.starts_with('.') {
            error(format!("unknown directive '{head}' (the only one is '.byte')"));
            continue;
        } else {
            if !addr.is_multiple_of(INSTR_SIZE) {
                error(format!(
                    "this instruction would start at odd address {addr:#04x}; \
                     add a padding byte to the '.byte' line before it"
                ));
            }
            (Body::Instruction { mnemonic: head, args }, INSTR_SIZE)
        };

        items.push(Item { line, addr, text, body });
        addr += size;

        if addr > MEM_SIZE && !reported_overflow {
            error(format!("the program has grown past the {MEM_SIZE} bytes of memory"));
            reported_overflow = true;
        }
    }

    (items, labels)
}

fn strip_comment(line: &str) -> &str {
    line.split_once(';').map_or(line, |(code, _comment)| code)
}

/// Splits "r0, r1, r2" into ["r0", "r1", "r2"], rejecting empty or space-joined operands.
fn split_operands(text: &str) -> Result<Vec<&str>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let args: Vec<&str> = text.split(',').map(str::trim).collect();
    for arg in &args {
        if arg.is_empty() {
            return Err("empty operand (check for a stray or doubled comma)".to_string());
        }
        if arg.contains(char::is_whitespace) {
            return Err(format!("'{arg}' looks like two operands; are you missing a comma?"));
        }
    }
    Ok(args)
}

/// Label names: a letter or '_' first, then letters, digits or '_'.
fn is_label_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn check_label_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        Err("a ':' with no label name in front of it".to_string())
    } else if !is_label_name(name) {
        Err(format!(
            "'{name}' isn't a valid label name (use letters, digits and '_', \
             starting with a letter or '_')"
        ))
    } else if parse_reg(name).is_ok() {
        Err(format!("'{name}' is a register name, so it can't be a label"))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Pass 2: turn each item into bytes
// ---------------------------------------------------------------------------

fn encode_item(item: &Item, labels: &HashMap<&str, Label>) -> Result<Vec<u8>, String> {
    match &item.body {
        Body::Bytes(args) => args
            .iter()
            .map(|arg| parse_value(arg, labels, Sign::Unsigned))
            .collect(),
        Body::Instruction { mnemonic, args } => {
            let instr = build_instr(mnemonic, args, labels)?;
            Ok(instr.encode().to_vec())
        }
    }
}

/// How each instruction is written, for error messages.
fn usage(kind: Kind) -> &'static str {
    match kind {
        Kind::Nop => "nop",
        Kind::Add => "add x, y, c   (c = x + y)",
        Kind::Sub => "sub x, y, c   (c = x - y)",
        Kind::And => "and x, y, c   (c = x & y)",
        Kind::Or => "or x, y, c   (c = x | y)",
        Kind::Xor => "xor x, y, c   (c = x ^ y)",
        Kind::Ldi => "ldi x, value",
        Kind::Ld => "ld x, address",
        Kind::St => "st x, address",
        Kind::Out => "out x",
        Kind::In => "in x",
        Kind::Jmp => "jmp address",
        Kind::Jz => "jz x, address",
        Kind::Hlt => "hlt",
    }
}

fn build_instr(mnemonic: &str, args: &[&str], labels: &HashMap<&str, Label>) -> Result<Instr, String> {
    let kind = Kind::from_name(mnemonic)
        .ok_or_else(|| format!("unknown instruction '{mnemonic}'"))?;

    // Every instruction has a fixed number of operands.
    let count = match kind {
        Kind::Nop | Kind::Hlt => 0,
        Kind::Out | Kind::In | Kind::Jmp => 1,
        Kind::Ldi | Kind::Ld | Kind::St | Kind::Jz => 2,
        Kind::Add | Kind::Sub | Kind::And | Kind::Or | Kind::Xor => 3,
    };
    if args.len() != count {
        return Err(format!(
            "'{}' takes {count} operand(s) but got {}; it's written: {}",
            kind.name(),
            args.len(),
            usage(kind)
        ));
    }

    let reg = |i: usize| parse_reg(args[i]);
    let addr = |i: usize| parse_value(args[i], labels, Sign::Unsigned);
    let alu = |op: AluOp| -> Result<Instr, String> {
        Ok(Instr::Alu { op, x: reg(0)?, y: reg(1)?, c: reg(2)? })
    };

    // Exhaustive: adding a new Kind won't compile until it's handled here.
    Ok(match kind {
        Kind::Nop => Instr::Nop,
        Kind::Add => alu(AluOp::Add)?,
        Kind::Sub => alu(AluOp::Sub)?,
        Kind::And => alu(AluOp::And)?,
        Kind::Or => alu(AluOp::Or)?,
        Kind::Xor => alu(AluOp::Xor)?,
        Kind::Ldi => Instr::Ldi { x: reg(0)?, value: parse_value(args[1], labels, Sign::AllowNegative)? },
        Kind::Ld => Instr::Ld { x: reg(0)?, addr: addr(1)? },
        Kind::St => Instr::St { x: reg(0)?, addr: addr(1)? },
        Kind::Out => Instr::Out { x: reg(0)? },
        Kind::In => Instr::In { x: reg(0)? },
        Kind::Jmp => Instr::Jmp { addr: jump_target(addr(0)?)? },
        Kind::Jz => Instr::Jz { x: reg(0)?, addr: jump_target(addr(1)?)? },
        Kind::Hlt => Instr::Hlt,
    })
}

fn jump_target(addr: u8) -> Result<u8, String> {
    if usize::from(addr).is_multiple_of(INSTR_SIZE) {
        Ok(addr)
    } else {
        Err(format!(
            "jump target {addr:#04x} is odd; instructions only start at even addresses"
        ))
    }
}

/// `r0`..`r3`, any case.
fn parse_reg(token: &str) -> Result<Reg, String> {
    let bad = || format!("expected a register r0-r3, got '{token}'");
    let digits = token
        .strip_prefix('r')
        .or_else(|| token.strip_prefix('R'))
        .ok_or_else(bad)?;
    // Exactly one digit, so things like "r+1" or "r01" are rejected.
    if digits.len() != 1 {
        return Err(bad());
    }
    let n: u8 = digits.parse().map_err(|_| bad())?;
    Reg::new(n).ok_or_else(bad)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sign {
    Unsigned,
    AllowNegative,
}

/// A number (0..=255, or -128..=-1 when allowed) or a label.
fn parse_value(token: &str, labels: &HashMap<&str, Label>, sign: Sign) -> Result<u8, String> {
    if token.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+') {
        let n = parse_number(token).ok_or_else(|| format!("'{token}' isn't a valid number"))?;
        if let Ok(byte) = u8::try_from(n) {
            return Ok(byte);
        }
        if sign == Sign::AllowNegative {
            // -1 becomes 255, -128 becomes 128: the two's complement byte.
            return i8::try_from(n)
                .map(i8::cast_unsigned)
                .map_err(|_| format!("{token} doesn't fit in a byte (allowed: -128 to 255)"));
        }
        return Err(format!("{token} doesn't fit in a byte (allowed: 0 to 255)"));
    }

    if !is_label_name(token) {
        return Err(format!("expected a number or a label, got '{token}'"));
    }
    if parse_reg(token).is_ok() {
        return Err(format!("expected a number or a label here, but '{token}' is a register"));
    }
    match labels.get(token) {
        Some(label) => u8::try_from(label.addr).map_err(|_| {
            format!(
                "label '{token}' is at address {}, past the end of memory",
                label.addr
            )
        }),
        None => {
            // Help with the most likely mistake: a label spelled with different case.
            let hint = labels
                .keys()
                .find(|name| name.eq_ignore_ascii_case(token))
                .map(|name| format!(" (did you mean '{name}'? labels are case-sensitive)"))
                .unwrap_or_default();
            Err(format!("undefined label '{token}'{hint}"))
        }
    }
}

/// Decimal, `0x` hex or `0b` binary, with an optional sign. `None` if malformed.
fn parse_number(token: &str) -> Option<i64> {
    let (negative, unsigned) = match token.as_bytes().first()? {
        b'-' => (true, &token[1..]),
        b'+' => (false, &token[1..]),
        _ => (false, token),
    };
    let lower = unsigned.to_ascii_lowercase();
    let (digits, radix) = if let Some(hex) = lower.strip_prefix("0x") {
        (hex, 16)
    } else if let Some(bin) = lower.strip_prefix("0b") {
        (bin, 2)
    } else {
        (lower.as_str(), 10)
    };
    // from_str_radix would accept a second sign ("--5"), so allow digits only.
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    // Anything too big for i64 fails here, and is reported as out of range above.
    let magnitude = i64::from_str_radix(digits, radix).ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(src: &str) -> Vec<u8> {
        match assemble(src) {
            Ok(program) => program.bytes,
            Err(errors) => panic!("unexpected errors: {errors:?}"),
        }
    }

    /// Assembles `src`, expecting failure, and returns the first error.
    fn first_error(src: &str) -> AsmError {
        match assemble(src) {
            Ok(program) => panic!("expected an error, got bytes {:02x?}", program.bytes),
            Err(errors) => errors[0].clone(),
        }
    }

    fn assert_error(src: &str, line: usize, contains: &str) {
        let e = first_error(src);
        assert_eq!(e.line, line, "wrong line for {src:?}: {e}");
        assert!(e.message.contains(contains), "for {src:?} expected '{contains}' in: {e}");
    }

    const COUNT: &str = "
ldi r0, 0
ldi r1, 1
ldi r2, 10
loop:
out r0
sub r0, r2, r3
jz r3, done
add r0, r1, r0
jmp loop
done:
hlt
";

    #[test]
    fn count_matches_the_hand_worked_bytes() {
        assert_eq!(
            bytes(COUNT),
            vec![
                0x60, 0x00, 0x64, 0x01, 0x68, 0x0a, 0x90, 0x00, 0x22, 0xc0, 0xcc, 0x10, 0x11,
                0x00, 0xb0, 0x06, 0xf0, 0x00,
            ]
        );
    }

    #[test]
    fn windows_line_endings_and_byte_order_mark() {
        let windows = format!("\u{feff}{}", COUNT.replace('\n', "\r\n"));
        assert_eq!(bytes(&windows), bytes(COUNT));
    }

    #[test]
    fn empty_and_comment_only_sources_give_an_empty_program() {
        assert_eq!(bytes(""), Vec::<u8>::new());
        assert_eq!(bytes("; nothing\n\n   \n"), Vec::<u8>::new());
    }

    #[test]
    fn case_whitespace_and_comments() {
        assert_eq!(bytes("  LDI\tR0 ,  0XFF ; hi"), vec![0x60, 0xff]);
        assert_eq!(bytes("ldi r0,0b1010"), vec![0x60, 0x0a]);
    }

    #[test]
    fn labels_forward_backward_and_inline() {
        // jmp end / nop / end: hlt
        assert_eq!(bytes("jmp end\nnop\nend: hlt"), vec![0xb0, 0x04, 0x00, 0x00, 0xf0, 0x00]);
        assert_eq!(bytes("top:\n  jmp top"), vec![0xb0, 0x00]);
        // two labels on one address
        assert_eq!(bytes("a:\nb: jmp a\njmp b"), vec![0xb0, 0x00, 0xb0, 0x00]);
    }

    #[test]
    fn negative_numbers_only_for_ldi() {
        assert_eq!(bytes("ldi r0, -1"), vec![0x60, 0xff]);
        assert_eq!(bytes("ldi r0, -128"), vec![0x60, 0x80]);
        assert_error("ldi r0, -129", 1, "doesn't fit");
        assert_error("st r0, -1", 1, "doesn't fit");
    }

    #[test]
    fn byte_directive_and_alignment() {
        assert_eq!(bytes("ld r0, data\nhlt\ndata: .byte 7, 0x08"), vec![0x70, 0x04, 0xf0, 0x00, 7, 8]);
        assert_eq!(bytes("jmp go\nv: .byte 1, 0\ngo: hlt"), vec![0xb0, 0x04, 1, 0, 0xf0, 0x00]);
        assert_error(".byte 1\nhlt", 2, "odd address");
        assert_error(".byte", 1, "at least one");
        assert_error(".word 5", 1, "unknown directive");
    }

    #[test]
    fn exactly_256_bytes_fits_and_257_does_not() {
        assert_eq!(bytes(&"nop\n".repeat(128)).len(), 256);
        assert_error(&format!("{}.byte 0", "nop\n".repeat(128)), 129, "past the 256 bytes");
        assert_error(&"nop\n".repeat(129), 129, "past the 256 bytes");
    }

    #[test]
    fn label_after_full_memory_cannot_be_used() {
        let src = format!("jmp end\n{}end:", "nop\n".repeat(127));
        assert_error(&src, 1, "past the end of memory");
    }

    #[test]
    fn rejects_bad_instructions() {
        assert_error("foo r0", 1, "unknown instruction 'foo'");
        assert_error("ldi r4, 0", 1, "register r0-r3");
        assert_error("ldi r01, 0", 1, "register r0-r3");
        assert_error("ldi 5, 0", 1, "register r0-r3");
        assert_error("add r0, r1", 1, "takes 3 operand(s)");
        assert_error("hlt r0", 1, "takes 0 operand(s)");
        assert_error("ldi r0, 256", 1, "doesn't fit");
        assert_error("ldi r0, 99999999999999999999999", 1, "isn't a valid number");
        assert_error("ldi r0, 0x", 1, "isn't a valid number");
        assert_error("ldi r0, 12ab", 1, "isn't a valid number");
        assert_error("ldi r0, --5", 1, "isn't a valid number");
        assert_error("ldi r0, r1", 1, "is a register");
        assert_error("ldi r0, $5", 1, "number or a label");
    }

    #[test]
    fn rejects_bad_operand_lists() {
        assert_error("add r0,, r1, r2", 1, "empty operand");
        assert_error("add r0, r1, r2,", 1, "empty operand");
        assert_error("add r0 r1, r2", 1, "missing a comma");
    }

    #[test]
    fn rejects_bad_labels() {
        assert_error("jmp nowhere", 1, "undefined label 'nowhere'");
        assert_error("Loop:\njmp loop", 2, "did you mean 'Loop'");
        assert_error("a:\na:", 2, "already defined on line 1");
        assert_error("2go:", 1, "isn't a valid label name");
        assert_error("r1:", 1, "register name");
        assert_error(": hlt", 1, "no label name");
        assert_error("a: b: hlt", 1, "one label per line");
        assert_error("my label: hlt", 1, "isn't a valid label name");
    }

    #[test]
    fn rejects_odd_jump_targets() {
        assert_error("jmp 3", 1, "is odd");
        assert_error("jz r0, 0x11", 1, "is odd");
        assert_error("jmp 2\n.byte 0\nodd: .byte 0\njmp odd", 4, "is odd");
    }

    #[test]
    fn reports_every_error_sorted_by_line() {
        let errors = assemble("foo\nhlt\nldi r9, 0\njmp nowhere").unwrap_err();
        let lines: Vec<usize> = errors.iter().map(|e| e.line).collect();
        assert_eq!(lines, vec![1, 3, 4]);
    }

    #[test]
    fn listing_lines_up_with_the_bytes() {
        let program = assemble("start: ldi r0, 5 ; five\nv: .byte 1, 2\nhlt").unwrap();
        assert_eq!(program.listing.len(), 3);
        assert_eq!(program.listing[0].addr, 0);
        assert_eq!(program.listing[0].source, "ldi r0, 5");
        assert_eq!(program.listing[1].addr, 2);
        assert_eq!(program.listing[1].bytes, vec![1, 2]);
        assert_eq!(program.listing[2].addr, 4);
        let joined: Vec<u8> = program.listing.iter().flat_map(|l| l.bytes.clone()).collect();
        assert_eq!(joined, program.bytes);
    }

    /// The disassembler (isa's Display) and the assembler must agree:
    /// every valid instruction printed as text must assemble back to itself.
    #[test]
    fn every_instruction_survives_a_round_trip_through_text() {
        for b0 in 0..=255u8 {
            for b1 in 0..=255u8 {
                let Ok(instr) = Instr::decode([b0, b1]) else { continue };
                let is_odd_jump = matches!(instr, Instr::Jmp { addr } | Instr::Jz { addr, .. } if addr % 2 == 1);
                let text = instr.to_string();
                match assemble(&text) {
                    Ok(program) => assert_eq!(program.bytes, vec![b0, b1], "'{text}'"),
                    // The hardware can decode odd jumps, but the assembler refuses to write them.
                    Err(_) if is_odd_jump => {}
                    Err(e) => panic!("'{text}' failed to assemble: {e:?}"),
                }
            }
        }
    }
}
