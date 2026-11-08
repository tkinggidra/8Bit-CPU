//! The instruction set of the 8Bit-CPU.
//!
//! This is the one place that says what every instruction is called, which
//! opcode it has and how it is packed into bytes. The assembler and the
//! simulator both build on it, so they can never disagree.
//!
//! Every instruction is exactly two bytes:
//!
//! ```text
//! byte 0:  [ opcode:4 ][ x:2 ][ y:2 ]
//! byte 1:  add/sub/and/or/xor:  [ c:2 ][ 0:6 ]
//!          ldi:                 the value
//!          ld/st/jz/jmp:        the address
//!          everything else:     0
//! ```
//!
//! Any bit not used by an instruction must be zero. `Instr::decode` rejects
//! anything else, so corrupted or hand-mangled programs are caught early
//! instead of doing something strange.

use std::fmt;

/// Memory size in bytes. Addresses are 8 bits wide, so this is fixed.
pub const MEM_SIZE: usize = 256;

/// Every instruction is this many bytes long.
pub const INSTR_SIZE: usize = 2;

// ---------------------------------------------------------------------------
// Registers
// ---------------------------------------------------------------------------

/// One of the four general-purpose registers, `r0` to `r3`.
///
/// The number inside is private, so a `Reg` can only ever hold 0..=3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Reg(u8);

impl Reg {
    /// All four registers, in order.
    pub const ALL: [Reg; 4] = [Reg(0), Reg(1), Reg(2), Reg(3)];

    /// Returns `None` for anything outside 0..=3.
    pub fn new(n: u8) -> Option<Reg> {
        (n < 4).then_some(Reg(n))
    }

    /// The register number, ready to use as an array index.
    pub fn index(self) -> usize {
        usize::from(self.0)
    }
}

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// The instruction table
// ---------------------------------------------------------------------------

/// Every instruction, without its operands.
///
/// `Kind::info` below is the only place mnemonics and opcodes are written
/// down. Adding an instruction means adding a variant here; the compiler then
/// points at every `match` in the project that needs updating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Nop,
    Add,
    Sub,
    And,
    Or,
    Xor,
    Ldi,
    Ld,
    St,
    Out,
    In,
    Jmp,
    Jz,
    Hlt,
}

impl Kind {
    /// Every kind, in opcode order.
    pub const ALL: [Kind; 14] = [
        Kind::Nop,
        Kind::Add,
        Kind::Sub,
        Kind::And,
        Kind::Or,
        Kind::Xor,
        Kind::Ldi,
        Kind::Ld,
        Kind::St,
        Kind::Out,
        Kind::In,
        Kind::Jmp,
        Kind::Jz,
        Kind::Hlt,
    ];

    /// (mnemonic, opcode). Opcodes 0xD and 0xE are free for future use.
    const fn info(self) -> (&'static str, u8) {
        match self {
            Kind::Nop => ("nop", 0x0),
            Kind::Add => ("add", 0x1),
            Kind::Sub => ("sub", 0x2),
            Kind::And => ("and", 0x3),
            Kind::Or => ("or", 0x4),
            Kind::Xor => ("xor", 0x5),
            Kind::Ldi => ("ldi", 0x6),
            Kind::Ld => ("ld", 0x7),
            Kind::St => ("st", 0x8),
            Kind::Out => ("out", 0x9),
            Kind::In => ("in", 0xA),
            Kind::Jmp => ("jmp", 0xB),
            Kind::Jz => ("jz", 0xC),
            Kind::Hlt => ("hlt", 0xF),
        }
    }

    /// The mnemonic, in lower case.
    pub const fn name(self) -> &'static str {
        self.info().0
    }

    /// The 4-bit opcode.
    pub const fn opcode(self) -> u8 {
        self.info().1
    }

    /// Looks up a mnemonic, ignoring case.
    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL
            .into_iter()
            .find(|k| k.name().eq_ignore_ascii_case(name))
    }

    /// Looks up an opcode.
    pub fn from_opcode(opcode: u8) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.opcode() == opcode)
    }
}

/// The five operations the ALU can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AluOp {
    Add,
    Sub,
    And,
    Or,
    Xor,
}

impl AluOp {
    /// The instruction kind for this operation.
    pub fn kind(self) -> Kind {
        match self {
            AluOp::Add => Kind::Add,
            AluOp::Sub => Kind::Sub,
            AluOp::And => Kind::And,
            AluOp::Or => Kind::Or,
            AluOp::Xor => Kind::Xor,
        }
    }

    /// Does the operation exactly as 8-bit hardware would: results wrap
    /// around, so 255 + 1 = 0 and 0 - 1 = 255.
    pub fn apply(self, a: u8, b: u8) -> u8 {
        match self {
            AluOp::Add => a.wrapping_add(b),
            AluOp::Sub => a.wrapping_sub(b),
            AluOp::And => a & b,
            AluOp::Or => a | b,
            AluOp::Xor => a ^ b,
        }
    }
}

// ---------------------------------------------------------------------------
// Instructions
// ---------------------------------------------------------------------------

/// One complete instruction, with its operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Instr {
    /// Do nothing.
    Nop,
    /// `c = x <op> y`
    Alu { op: AluOp, x: Reg, y: Reg, c: Reg },
    /// `x = value`
    Ldi { x: Reg, value: u8 },
    /// `x = memory[addr]`
    Ld { x: Reg, addr: u8 },
    /// `memory[addr] = x`
    St { x: Reg, addr: u8 },
    /// Send `x` to the output port.
    Out { x: Reg },
    /// `x = input port`
    In { x: Reg },
    /// Jump to `addr`.
    Jmp { addr: u8 },
    /// Jump to `addr` if `x` is zero.
    Jz { x: Reg, addr: u8 },
    /// Stop.
    Hlt,
}

/// Why two bytes are not a valid instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// No instruction has this opcode.
    UnknownOpcode(u8),
    /// The opcode is fine, but bits the instruction doesn't use are not zero.
    ReservedBitsSet,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DecodeError::UnknownOpcode(op) => write!(f, "unknown opcode {op:#x}"),
            DecodeError::ReservedBitsSet => write!(f, "bits the instruction doesn't use are not zero"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl Instr {
    /// Which instruction this is, without its operands.
    pub fn kind(self) -> Kind {
        match self {
            Instr::Nop => Kind::Nop,
            Instr::Alu { op, .. } => op.kind(),
            Instr::Ldi { .. } => Kind::Ldi,
            Instr::Ld { .. } => Kind::Ld,
            Instr::St { .. } => Kind::St,
            Instr::Out { .. } => Kind::Out,
            Instr::In { .. } => Kind::In,
            Instr::Jmp { .. } => Kind::Jmp,
            Instr::Jz { .. } => Kind::Jz,
            Instr::Hlt => Kind::Hlt,
        }
    }

    /// Packs the instruction into its two bytes.
    pub fn encode(self) -> [u8; 2] {
        // byte 0 is always [ opcode:4 ][ x:2 ][ y:2 ]; unused fields are r0 (zero)
        let byte0 = |x: Reg, y: Reg| (self.kind().opcode() << 4) | (x.0 << 2) | y.0;
        let zero = Reg(0);
        match self {
            Instr::Nop | Instr::Hlt => [byte0(zero, zero), 0],
            Instr::Alu { x, y, c, .. } => [byte0(x, y), c.0 << 6],
            Instr::Ldi { x, value } => [byte0(x, zero), value],
            Instr::Ld { x, addr } | Instr::St { x, addr } | Instr::Jz { x, addr } => {
                [byte0(x, zero), addr]
            }
            Instr::Out { x } | Instr::In { x } => [byte0(x, zero), 0],
            Instr::Jmp { addr } => [byte0(zero, zero), addr],
        }
    }

    /// Unpacks two bytes. This is the exact inverse of `encode`: every
    /// instruction it accepts encodes back to the same two bytes.
    pub fn decode([b0, b1]: [u8; 2]) -> Result<Instr, DecodeError> {
        let opcode = b0 >> 4;
        let kind = Kind::from_opcode(opcode).ok_or(DecodeError::UnknownOpcode(opcode))?;
        let x = Reg((b0 >> 2) & 0b11);
        let y = Reg(b0 & 0b11);

        // Fails unless every bit the instruction doesn't use is zero.
        let require = |unused_bits_are_zero: bool| {
            if unused_bits_are_zero {
                Ok(())
            } else {
                Err(DecodeError::ReservedBitsSet)
            }
        };
        let no_x = x.0 == 0;
        let no_y = y.0 == 0;

        let alu = |op: AluOp| {
            require(b1 & 0b0011_1111 == 0)?;
            Ok(Instr::Alu { op, x, y, c: Reg(b1 >> 6) })
        };

        match kind {
            Kind::Nop => require(no_x && no_y && b1 == 0).map(|()| Instr::Nop),
            Kind::Add => alu(AluOp::Add),
            Kind::Sub => alu(AluOp::Sub),
            Kind::And => alu(AluOp::And),
            Kind::Or => alu(AluOp::Or),
            Kind::Xor => alu(AluOp::Xor),
            Kind::Ldi => require(no_y).map(|()| Instr::Ldi { x, value: b1 }),
            Kind::Ld => require(no_y).map(|()| Instr::Ld { x, addr: b1 }),
            Kind::St => require(no_y).map(|()| Instr::St { x, addr: b1 }),
            Kind::Out => require(no_y && b1 == 0).map(|()| Instr::Out { x }),
            Kind::In => require(no_y && b1 == 0).map(|()| Instr::In { x }),
            Kind::Jmp => require(no_x && no_y).map(|()| Instr::Jmp { addr: b1 }),
            Kind::Jz => require(no_y).map(|()| Instr::Jz { x, addr: b1 }),
            Kind::Hlt => require(no_x && no_y && b1 == 0).map(|()| Instr::Hlt),
        }
    }
}

/// Prints the instruction as assembly. The assembler accepts this output,
/// so disassembling and reassembling gives back the same bytes.
impl fmt::Display for Instr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let name = self.kind().name();
        match *self {
            Instr::Nop | Instr::Hlt => write!(f, "{name}"),
            Instr::Alu { x, y, c, .. } => write!(f, "{name} {x}, {y}, {c}"),
            Instr::Ldi { x, value } => write!(f, "{name} {x}, {value}"),
            Instr::Ld { x, addr } | Instr::St { x, addr } | Instr::Jz { x, addr } => {
                write!(f, "{name} {x}, {addr:#04x}")
            }
            Instr::Out { x } | Instr::In { x } => write!(f, "{name} {x}"),
            Instr::Jmp { addr } => write!(f, "{name} {addr:#04x}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: u8) -> Reg {
        Reg::new(n).unwrap()
    }

    #[test]
    fn table_has_unique_names_and_4_bit_opcodes() {
        for (i, a) in Kind::ALL.iter().enumerate() {
            assert!(a.opcode() < 16, "{a:?} opcode does not fit in 4 bits");
            for b in &Kind::ALL[i + 1..] {
                assert_ne!(a.opcode(), b.opcode(), "{a:?} and {b:?} share an opcode");
                assert_ne!(a.name(), b.name(), "{a:?} and {b:?} share a name");
            }
        }
    }

    #[test]
    fn lookups_work_both_ways() {
        for k in Kind::ALL {
            assert_eq!(Kind::from_name(k.name()), Some(k));
            assert_eq!(Kind::from_name(&k.name().to_uppercase()), Some(k));
            assert_eq!(Kind::from_opcode(k.opcode()), Some(k));
        }
        assert_eq!(Kind::from_name("mov"), None);
        assert_eq!(Kind::from_opcode(0xD), None);
    }

    #[test]
    fn reg_only_accepts_0_to_3() {
        assert_eq!(Reg::new(3).map(Reg::index), Some(3));
        assert_eq!(Reg::new(4), None);
        assert_eq!(Reg::new(255), None);
    }

    #[test]
    fn known_encodings() {
        // These match the bytes worked out by hand for count.asm.
        let cases = [
            (Instr::Ldi { x: r(0), value: 0 }, [0x60, 0x00]),
            (Instr::Ldi { x: r(2), value: 10 }, [0x68, 0x0a]),
            (Instr::Out { x: r(0) }, [0x90, 0x00]),
            (Instr::Alu { op: AluOp::Sub, x: r(0), y: r(2), c: r(3) }, [0x22, 0xc0]),
            (Instr::Jz { x: r(3), addr: 0x10 }, [0xcc, 0x10]),
            (Instr::Alu { op: AluOp::Add, x: r(0), y: r(1), c: r(0) }, [0x11, 0x00]),
            (Instr::Jmp { addr: 6 }, [0xb0, 0x06]),
            (Instr::Hlt, [0xf0, 0x00]),
        ];
        for (instr, bytes) in cases {
            assert_eq!(instr.encode(), bytes, "encoding {instr}");
            assert_eq!(Instr::decode(bytes), Ok(instr), "decoding {bytes:02x?}");
        }
    }

    /// Tries all 65,536 possible byte pairs. Whatever decodes must encode back
    /// to exactly the same bytes, so no two byte pairs mean the same thing.
    #[test]
    fn decode_is_the_exact_inverse_of_encode() {
        let mut valid = 0;
        for b0 in 0..=255u8 {
            for b1 in 0..=255u8 {
                if let Ok(instr) = Instr::decode([b0, b1]) {
                    assert_eq!(instr.encode(), [b0, b1], "{instr} re-encodes differently");
                    valid += 1;
                }
            }
        }
        // nop, hlt: 1 each; 5 ALU ops: 4*4*4 each; ldi/ld/st/jz: 4*256 each;
        // out/in: 4 each; jmp: 256.
        assert_eq!(valid, 2 + 5 * 64 + 4 * 1024 + 2 * 4 + 256);
    }

    #[test]
    fn reserved_bits_are_rejected() {
        assert_eq!(Instr::decode([0x01, 0x00]), Err(DecodeError::ReservedBitsSet)); // nop with y set
        assert_eq!(Instr::decode([0xf0, 0x01]), Err(DecodeError::ReservedBitsSet)); // hlt with byte 1 set
        assert_eq!(Instr::decode([0x11, 0x01]), Err(DecodeError::ReservedBitsSet)); // add, low bits of byte 1
        assert_eq!(Instr::decode([0x90, 0x05]), Err(DecodeError::ReservedBitsSet)); // out with byte 1 set
        assert_eq!(Instr::decode([0xb4, 0x00]), Err(DecodeError::ReservedBitsSet)); // jmp with x set
    }

    #[test]
    fn unknown_opcodes_are_rejected() {
        assert_eq!(Instr::decode([0xd0, 0x00]), Err(DecodeError::UnknownOpcode(0xd)));
        assert_eq!(Instr::decode([0xe0, 0x00]), Err(DecodeError::UnknownOpcode(0xe)));
    }

    #[test]
    fn alu_wraps_like_8_bit_hardware() {
        assert_eq!(AluOp::Add.apply(200, 100), 44);
        assert_eq!(AluOp::Sub.apply(1, 2), 255);
        assert_eq!(AluOp::And.apply(0b1100, 0b1010), 0b1000);
        assert_eq!(AluOp::Or.apply(0b1100, 0b1010), 0b1110);
        assert_eq!(AluOp::Xor.apply(0b1100, 0b1010), 0b0110);
    }
}
