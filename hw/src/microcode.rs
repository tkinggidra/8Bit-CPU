//! The control ROM: what the control unit tells the CPU to do on every tick.
//!
//! The ROM has 8 address lines and 24 data lines (three 8-bit chips):
//!
//! ```text
//! address:  A7-A4 = opcode (IR0 bits 7-4)
//!           A3-A1 = step counter (0-7)
//!           A0    = zero flag
//! data:     the 24 control lines, see `signals`
//! ```
//!
//! Every instruction is the same four fetch ticks (steps 0-3), then up to
//! three ticks of its own (steps 4-6). The table of steps lives in
//! `execute_steps` below, written from the instruction set so the compiler
//! forces an update here whenever an instruction is added.

use crate::signals::{LINES, RegSel, Signals, pin_name};
use isa::{AluOp, Kind};
use std::fmt::Write;

/// Number of entries in the control ROM (8 address lines).
pub const ROM_SIZE: usize = 256;

/// Ticks spent fetching the two instruction bytes, before the instruction runs.
pub const FETCH_STEPS: usize = 4;

/// The ROM address for a given opcode, step and zero flag.
pub fn rom_address(opcode: u8, step: u8, zero: bool) -> u8 {
    ((opcode & 0xF) << 4) | ((step & 0b111) << 1) | u8::from(zero)
}

/// The inverse of `rom_address`: (opcode, step, zero flag).
pub fn split_address(addr: u8) -> (u8, u8, bool) {
    (addr >> 4, (addr >> 1) & 0b111, addr & 1 == 1)
}

/// The four fetch ticks, identical for every instruction.
fn fetch_steps() -> [Signals; FETCH_STEPS] {
    [
        // Address of the first byte goes to the memory address register...
        Signals::PC_OUT | Signals::MAR_LD,
        // ...memory puts that byte on the bus, IR0 grabs it, the PC moves on...
        Signals::RAM_OUT | Signals::IR0_LD | Signals::PC_INC,
        // ...and the same again for the second byte.
        Signals::PC_OUT | Signals::MAR_LD,
        Signals::RAM_OUT | Signals::IR1_LD | Signals::PC_INC,
    ]
}

/// The ticks an instruction needs after the fetch. The last one has `END`,
/// except `hlt`, which stops the clock instead.
///
/// `zero` is the zero flag, which only changes anything for `jz`.
fn execute_steps(kind: Kind, zero: bool) -> Vec<Signals> {
    use Signals as S;
    let x = S::reg_sel(RegSel::X);
    let y = S::reg_sel(RegSel::Y);
    let c = S::reg_sel(RegSel::C);

    // add/sub/and/or/xor: x goes into A, y into B, then the ALU result goes to c.
    let alu = |op: AluOp| {
        vec![
            S::REG_OUT | x | S::A_LD,
            S::REG_OUT | y | S::B_LD,
            S::ALU_OUT | S::alu(op) | S::REG_LD | c | S::END,
        ]
    };

    // Exhaustive: adding an instruction won't compile until it's handled here.
    match kind {
        Kind::Nop => vec![S::END],
        Kind::Add => alu(AluOp::Add),
        Kind::Sub => alu(AluOp::Sub),
        Kind::And => alu(AluOp::And),
        Kind::Or => alu(AluOp::Or),
        Kind::Xor => alu(AluOp::Xor),
        // The value in byte 1 goes straight into the register.
        Kind::Ldi => vec![S::IR1_OUT | S::REG_LD | x | S::END],
        // Byte 1 is an address: aim the memory at it, then read or write.
        Kind::Ld => vec![
            S::IR1_OUT | S::MAR_LD,
            S::RAM_OUT | S::REG_LD | x | S::END,
        ],
        Kind::St => vec![
            S::IR1_OUT | S::MAR_LD,
            S::REG_OUT | x | S::RAM_WE | S::END,
        ],
        Kind::Out => vec![S::REG_OUT | x | S::OUT_LD | S::END],
        Kind::In => vec![S::IN_OUT | S::REG_LD | x | S::END],
        Kind::Jmp => vec![S::IR1_OUT | S::PC_LD | S::END],
        // First look at the register and remember if it's zero, then jump only if it was.
        Kind::Jz => vec![
            S::REG_OUT | x | S::Z_LD,
            if zero { S::IR1_OUT | S::PC_LD | S::END } else { S::END },
        ],
        Kind::Hlt => vec![S::HLT],
    }
}

/// What the ROM outputs for one address.
pub fn control_word(opcode: u8, step: u8, zero: bool) -> Signals {
    let step = usize::from(step & 0b111);
    if step < FETCH_STEPS {
        return fetch_steps()[step];
    }
    match Kind::from_opcode(opcode & 0xF) {
        // An opcode nobody uses: stop, so the lights show something is wrong.
        None => Signals::HLT,
        // Past the end of the instruction (never reached): just reset the step counter.
        Some(kind) => execute_steps(kind, zero)
            .get(step - FETCH_STEPS)
            .copied()
            .unwrap_or(Signals::END),
    }
}

/// The whole ROM, indexed by address.
pub fn build_rom() -> [Signals; ROM_SIZE] {
    let mut rom = [Signals::NONE; ROM_SIZE];
    for addr in 0..=u8::MAX {
        let (opcode, step, zero) = split_address(addr);
        rom[usize::from(addr)] = control_word(opcode, step, zero);
    }
    rom
}

/// The three 256-byte images to burn into the three ROM chips (chip 0 first).
pub fn rom_images(rom: &[Signals; ROM_SIZE]) -> [[u8; ROM_SIZE]; 3] {
    let mut images = [[0u8; ROM_SIZE]; 3];
    for (addr, word) in rom.iter().enumerate() {
        for (chip, byte) in word.to_bytes().into_iter().enumerate() {
            images[chip][addr] = byte;
        }
    }
    images
}

/// A readable description of the whole ROM, for the repo and for wiring it up.
pub fn listing() -> String {
    let mut out = String::new();
    // Writing to a String can't fail, so the results are ignored.
    let _ = writeln!(out, "8Bit-CPU control ROM");
    let _ = writeln!(out, "====================\n");
    let _ = writeln!(out, "Address: A7-A4 = opcode (IR0 bits 7-4), A3-A1 = step counter, A0 = zero flag");
    for chip in 0..3 {
        let pins: Vec<&str> = (0..8).map(|d| pin_name(chip * 8 + d)).collect();
        let _ = writeln!(out, "Chip {chip}: D0-D7 = {}", pins.join(" "));
    }
    let _ = writeln!(out, "{LINES} control lines in total.\n");

    let _ = writeln!(out, "Fetch (every instruction, steps 0-3)");
    for (step, word) in fetch_steps().iter().enumerate() {
        let _ = writeln!(out, "  step {step}: {word}");
    }
    for kind in Kind::ALL {
        let _ = writeln!(out, "\n{} (opcode {:#x})", kind.name(), kind.opcode());
        let off = execute_steps(kind, false);
        let on = execute_steps(kind, true);
        for (i, word) in off.iter().enumerate() {
            let step = FETCH_STEPS + i;
            if *word == on[i] {
                let _ = writeln!(out, "  step {step}: {word}");
            } else {
                let _ = writeln!(out, "  step {step} (zero flag clear): {word}");
                let _ = writeln!(out, "  step {step} (zero flag set):   {}", on[i]);
            }
        }
    }
    let _ = writeln!(out, "\nUnused opcodes (0xd, 0xe)\n  step 4: {}", Signals::HLT);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_layout_round_trips() {
        for addr in 0..=u8::MAX {
            let (opcode, step, zero) = split_address(addr);
            assert_eq!(rom_address(opcode, step, zero), addr);
        }
        assert_eq!(rom_address(0xC, 5, true), 0b1100_1011);
    }

    #[test]
    fn fetch_is_identical_for_every_opcode_and_flag() {
        // The ROM is addressed by the opcode left over from the previous
        // instruction during the fetch, so it must not matter.
        for opcode in 0..16 {
            for step in 0..FETCH_STEPS {
                for zero in [false, true] {
                    assert_eq!(
                        control_word(opcode, u8::try_from(step).unwrap(), zero),
                        fetch_steps()[step]
                    );
                }
            }
        }
    }

    #[test]
    fn no_row_has_a_bus_conflict_or_a_floating_load() {
        for (addr, word) in build_rom().iter().enumerate() {
            let drivers = Signals::BUS_DRIVERS.iter().filter(|d| word.contains(**d)).count();
            assert!(drivers <= 1, "address {addr:#04x}: {drivers} bus drivers ({word})");
            if word.intersects(Signals::BUS_LOADS) {
                assert_eq!(drivers, 1, "address {addr:#04x}: loads from an undriven bus ({word})");
            }
            assert!(
                !(word.contains(Signals::PC_INC) && word.contains(Signals::PC_LD)),
                "address {addr:#04x}: PC_INC with PC_LD"
            );
            if word.intersects(Signals::REG_OUT | Signals::REG_LD) {
                assert!(word.reg_select().is_some(), "address {addr:#04x}: bad register select");
            }
            if word.contains(Signals::ALU_OUT) {
                assert!(word.alu_op().is_some(), "address {addr:#04x}: bad ALU function");
            }
        }
    }

    #[test]
    fn every_instruction_finishes_within_seven_ticks() {
        for kind in Kind::ALL {
            for zero in [false, true] {
                let finish = (0..8u8)
                    .find(|&step| {
                        let word = control_word(kind.opcode(), step, zero);
                        word.contains(Signals::END) || word.contains(Signals::HLT)
                    })
                    .unwrap_or_else(|| panic!("{} never ends", kind.name()));
                assert!(finish <= 6, "{} ends at step {finish}", kind.name());
            }
        }
    }

    #[test]
    fn only_jz_looks_at_the_zero_flag() {
        for addr in (0..=u8::MAX).step_by(2) {
            let (opcode, step, _) = split_address(addr);
            let differs = control_word(opcode, step, false) != control_word(opcode, step, true);
            let is_jz_step_5 = opcode == Kind::Jz.opcode() && step == 5;
            assert_eq!(differs, is_jz_step_5, "opcode {opcode:#x} step {step}");
        }
    }

    #[test]
    fn all_24_control_lines_are_used() {
        let used = build_rom().iter().fold(0, |acc, w| acc | w.bits());
        assert_eq!(used, Signals::ALL_BITS, "a control line is never used: {used:#08x}");
    }

    #[test]
    fn unused_opcodes_halt() {
        for opcode in [0xD, 0xE] {
            assert!(Kind::from_opcode(opcode).is_none());
            assert_eq!(control_word(opcode, 4, false), Signals::HLT);
        }
    }

    #[test]
    fn images_hold_the_same_words_as_the_rom() {
        let rom = build_rom();
        let images = rom_images(&rom);
        for (addr, word) in rom.iter().enumerate() {
            let bytes = [images[0][addr], images[1][addr], images[2][addr]];
            assert_eq!(Signals::from_bytes(bytes), *word);
        }
    }

    #[test]
    fn listing_mentions_every_instruction() {
        let text = listing();
        for kind in Kind::ALL {
            assert!(text.contains(&format!("\n{} (opcode", kind.name())), "{} missing", kind.name());
        }
        assert!(text.contains("zero flag set"));
    }
}
