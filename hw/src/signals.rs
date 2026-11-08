//! The control lines: the wires the control unit uses to tell every other
//! part what to do on each clock tick.
//!
//! There are 24 of them, one bit each in a 24-bit control word. That is
//! exactly three 8-bit-wide ROM chips, so the bits are grouped by chip:
//!
//! ```text
//! chip 0  PC_OUT RAM_OUT IR1_OUT REG_OUT ALU_OUT IN_OUT PC_INC  HLT
//! chip 1  MAR_LD RAM_WE  IR0_LD  IR1_LD  A_LD    B_LD   REG_LD  OUT_LD
//! chip 2  PC_LD  Z_LD    END     ALU_OP0 ALU_OP1 ALU_OP2 REG_SEL0 REG_SEL1
//! ```
//!
//! **`*_OUT`**: that part drives its number onto the bus. Only one part may
//! do this per tick, or two parts fight over the wires.
//! **`*_LD`** (load): that part grabs whatever is on the bus when the clock ticks.

use isa::AluOp;
use std::fmt;
use std::ops::BitOr;

/// One control word: which of the 24 control lines are high.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Signals(u32);

/// Which part of the instruction names the register for this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegSel {
    /// The `x` field: IR0 bits 3-2.
    X,
    /// The `y` field: IR0 bits 1-0.
    Y,
    /// The `c` field: IR1 bits 7-6 (the destination of an ALU instruction).
    C,
}

/// Number of control lines.
pub const LINES: u32 = 24;

/// Number of plain on/off lines. The remaining five bits are two small fields.
const FLAG_COUNT: usize = 19;

/// Where the ALU function field starts (3 bits), and the register select (2 bits).
const ALU_SHIFT: u32 = 19;
const SEL_SHIFT: u32 = 22;

impl Signals {
    /// Nothing asserted.
    pub const NONE: Signals = Signals(0);

    // --- ROM chip 0 ---
    pub const PC_OUT: Signals = Signals(1 << 0);
    pub const RAM_OUT: Signals = Signals(1 << 1);
    pub const IR1_OUT: Signals = Signals(1 << 2);
    pub const REG_OUT: Signals = Signals(1 << 3);
    pub const ALU_OUT: Signals = Signals(1 << 4);
    pub const IN_OUT: Signals = Signals(1 << 5);
    /// Count the program counter up by one.
    pub const PC_INC: Signals = Signals(1 << 6);
    /// Stop the clock.
    pub const HLT: Signals = Signals(1 << 7);

    // --- ROM chip 1 ---
    pub const MAR_LD: Signals = Signals(1 << 8);
    /// Write the bus into RAM at the address in MAR.
    pub const RAM_WE: Signals = Signals(1 << 9);
    pub const IR0_LD: Signals = Signals(1 << 10);
    pub const IR1_LD: Signals = Signals(1 << 11);
    pub const A_LD: Signals = Signals(1 << 12);
    pub const B_LD: Signals = Signals(1 << 13);
    pub const REG_LD: Signals = Signals(1 << 14);
    pub const OUT_LD: Signals = Signals(1 << 15);

    // --- ROM chip 2 ---
    pub const PC_LD: Signals = Signals(1 << 16);
    /// Remember whether the bus is zero (for `jz`).
    pub const Z_LD: Signals = Signals(1 << 17);
    /// This was the last tick of the instruction: reset the step counter.
    pub const END: Signals = Signals(1 << 18);

    /// The six parts that can drive the bus.
    pub const BUS_DRIVERS: [Signals; 6] = [
        Signals::PC_OUT,
        Signals::RAM_OUT,
        Signals::IR1_OUT,
        Signals::REG_OUT,
        Signals::ALU_OUT,
        Signals::IN_OUT,
    ];

    /// Every line that grabs a value from the bus.
    pub const BUS_LOADS: Signals = Signals::MAR_LD
        .or(Signals::RAM_WE)
        .or(Signals::IR0_LD)
        .or(Signals::IR1_LD)
        .or(Signals::A_LD)
        .or(Signals::B_LD)
        .or(Signals::REG_LD)
        .or(Signals::OUT_LD)
        .or(Signals::PC_LD)
        .or(Signals::Z_LD);

    /// All 24 bits.
    pub const ALL_BITS: u32 = (1 << LINES) - 1;

    /// Both signals set (usable in constants; `|` does the same elsewhere).
    pub const fn or(self, other: Signals) -> Signals {
        Signals(self.0 | other.0)
    }

    /// The raw 24 bits.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// True if every line in `other` is also high here.
    pub const fn contains(self, other: Signals) -> bool {
        self.0 & other.0 == other.0
    }

    /// True if at least one line in `other` is high here.
    pub const fn intersects(self, other: Signals) -> bool {
        self.0 & other.0 != 0
    }

    /// The ALU function lines for `op`.
    pub const fn alu(op: AluOp) -> Signals {
        let code = match op {
            AluOp::Add => 0,
            AluOp::Sub => 1,
            AluOp::And => 2,
            AluOp::Or => 3,
            AluOp::Xor => 4,
        };
        Signals(code << ALU_SHIFT)
    }

    /// The ALU function these lines select, or `None` for the unused codes 5-7.
    pub fn alu_op(self) -> Option<AluOp> {
        match (self.0 >> ALU_SHIFT) & 0b111 {
            0 => Some(AluOp::Add),
            1 => Some(AluOp::Sub),
            2 => Some(AluOp::And),
            3 => Some(AluOp::Or),
            4 => Some(AluOp::Xor),
            _ => None,
        }
    }

    /// The register-select lines for `sel`.
    pub const fn reg_sel(sel: RegSel) -> Signals {
        let code = match sel {
            RegSel::X => 0,
            RegSel::Y => 1,
            RegSel::C => 2,
        };
        Signals(code << SEL_SHIFT)
    }

    /// The register select these lines choose, or `None` for the unused code 3.
    pub fn reg_select(self) -> Option<RegSel> {
        match (self.0 >> SEL_SHIFT) & 0b11 {
            0 => Some(RegSel::X),
            1 => Some(RegSel::Y),
            2 => Some(RegSel::C),
            _ => None,
        }
    }

    /// Splits the word into the three bytes burned into the three ROM chips.
    pub fn to_bytes(self) -> [u8; 3] {
        let [a, b, c, _] = self.0.to_le_bytes();
        [a, b, c]
    }

    /// The inverse of `to_bytes`.
    pub fn from_bytes([a, b, c]: [u8; 3]) -> Signals {
        Signals(u32::from_le_bytes([a, b, c, 0]))
    }
}

impl BitOr for Signals {
    type Output = Signals;
    fn bitor(self, rhs: Signals) -> Signals {
        self.or(rhs)
    }
}

/// The 19 on/off lines, in bit order (entry `i` is bit `i`).
const FLAGS: [(Signals, &str); FLAG_COUNT] = [
    (Signals::PC_OUT, "PC_OUT"),
    (Signals::RAM_OUT, "RAM_OUT"),
    (Signals::IR1_OUT, "IR1_OUT"),
    (Signals::REG_OUT, "REG_OUT"),
    (Signals::ALU_OUT, "ALU_OUT"),
    (Signals::IN_OUT, "IN_OUT"),
    (Signals::PC_INC, "PC_INC"),
    (Signals::HLT, "HLT"),
    (Signals::MAR_LD, "MAR_LD"),
    (Signals::RAM_WE, "RAM_WE"),
    (Signals::IR0_LD, "IR0_LD"),
    (Signals::IR1_LD, "IR1_LD"),
    (Signals::A_LD, "A_LD"),
    (Signals::B_LD, "B_LD"),
    (Signals::REG_LD, "REG_LD"),
    (Signals::OUT_LD, "OUT_LD"),
    (Signals::PC_LD, "PC_LD"),
    (Signals::Z_LD, "Z_LD"),
    (Signals::END, "END"),
];

/// The name of control line `bit` (0-23), as it would be labelled on the board.
pub fn pin_name(bit: u32) -> &'static str {
    const FIELDS: [&str; 5] = ["ALU_OP0", "ALU_OP1", "ALU_OP2", "REG_SEL0", "REG_SEL1"];
    match usize::try_from(bit) {
        Ok(i) if i < FLAG_COUNT => FLAGS[i].1,
        Ok(i) if i < FLAG_COUNT + FIELDS.len() => FIELDS[i - FLAG_COUNT],
        _ => "?",
    }
}

/// Prints the high lines by name, e.g. `REG_OUT REG_SEL=x A_LD`.
impl fmt::Display for Signals {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut parts: Vec<String> = FLAGS
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| (*name).to_string())
            .collect();
        if self.intersects(Signals::REG_OUT | Signals::REG_LD) {
            let sel = match self.reg_select() {
                Some(RegSel::X) => "x",
                Some(RegSel::Y) => "y",
                Some(RegSel::C) => "c",
                None => "?",
            };
            parts.push(format!("REG_SEL={sel}"));
        }
        if self.contains(Signals::ALU_OUT) {
            parts.push(format!("ALU={}", self.alu_op().map_or("?", |op| op.kind().name())));
        }
        if parts.is_empty() {
            write!(f, "-")
        } else {
            write!(f, "{}", parts.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_table_is_in_bit_order_with_unique_names() {
        for (i, (flag, name)) in FLAGS.iter().enumerate() {
            assert_eq!(flag.bits(), 1 << i, "{name} is not bit {i}");
            assert_eq!(pin_name(u32::try_from(i).unwrap()), *name);
            assert_eq!(FLAGS.iter().filter(|(_, n)| n == name).count(), 1, "{name} repeated");
        }
    }

    #[test]
    fn fields_do_not_overlap_the_flags() {
        let flags: u32 = FLAGS.iter().fold(0, |acc, (f, _)| acc | f.bits());
        let fields = (0b111 << ALU_SHIFT) | (0b11 << SEL_SHIFT);
        assert_eq!(flags & fields, 0);
        assert_eq!(flags | fields, Signals::ALL_BITS);
        assert_eq!(pin_name(LINES), "?");
    }

    #[test]
    fn every_bus_driver_and_load_is_one_distinct_flag() {
        for driver in Signals::BUS_DRIVERS {
            assert_eq!(driver.bits().count_ones(), 1);
            assert!(!driver.intersects(Signals::BUS_LOADS));
        }
        assert_eq!(Signals::BUS_LOADS.bits().count_ones(), 10);
    }

    #[test]
    fn alu_and_register_fields_round_trip() {
        for op in [AluOp::Add, AluOp::Sub, AluOp::And, AluOp::Or, AluOp::Xor] {
            assert_eq!(Signals::alu(op).alu_op(), Some(op));
        }
        for sel in [RegSel::X, RegSel::Y, RegSel::C] {
            assert_eq!(Signals::reg_sel(sel).reg_select(), Some(sel));
        }
        assert_eq!(Signals(5 << ALU_SHIFT).alu_op(), None);
        assert_eq!(Signals(3 << SEL_SHIFT).reg_select(), None);
    }

    #[test]
    fn bytes_round_trip_and_split_by_chip() {
        let word = Signals::PC_OUT | Signals::MAR_LD | Signals::END;
        assert_eq!(word.to_bytes(), [0b0000_0001, 0b0000_0001, 0b0000_0100]);
        assert_eq!(Signals::from_bytes(word.to_bytes()), word);
        assert_eq!(Signals::from_bytes([0xff; 3]).bits(), Signals::ALL_BITS);
    }

    #[test]
    fn display_names_the_lines() {
        assert_eq!(Signals::NONE.to_string(), "-");
        let word = Signals::REG_OUT | Signals::reg_sel(RegSel::Y) | Signals::B_LD;
        assert_eq!(word.to_string(), "REG_OUT B_LD REG_SEL=y");
        let word = Signals::ALU_OUT | Signals::alu(AluOp::Sub) | Signals::REG_LD | Signals::reg_sel(RegSel::C);
        assert_eq!(word.to_string(), "ALU_OUT REG_LD REG_SEL=c ALU=sub");
    }
}
