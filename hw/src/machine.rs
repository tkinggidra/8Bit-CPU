//! The machine: every register, the bus and the control ROM, advanced one
//! clock tick at a time.
//!
//! Each `tick` does what the real circuit does between two clock edges:
//!
//! 1. The ROM, addressed by (opcode, step, zero flag), outputs the control word.
//! 2. At most one part drives the bus. Its value is the bus value.
//! 3. On the clock edge every part with a load line high grabs the bus value.
//!    They all grab at the same instant, so each sees the values from *before*
//!    the edge (a register can't be read and rewritten in one tick).
//! 4. The step counter moves on, or resets if `END` was high.
//!
//! Faults the real wires would suffer from are caught as errors instead:
//! two parts driving the bus at once, or a load from a bus nobody drives.

use crate::microcode::{ROM_SIZE, build_rom, rom_address};
use crate::signals::{RegSel, Signals};
use isa::{Kind, MEM_SIZE};
use std::fmt;

/// The CPU, with every register the hardware has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    /// The control ROM. Public so tests can break it on purpose.
    pub rom: [Signals; ROM_SIZE],
    pub mem: [u8; MEM_SIZE],
    /// Registers r0 to r3.
    pub regs: [u8; 4],
    /// Program counter.
    pub pc: u8,
    /// Memory address register: the address RAM is currently looking at.
    pub mar: u8,
    /// Instruction register, byte 0: `[opcode:4][x:2][y:2]`.
    pub ir0: u8,
    /// Instruction register, byte 1: the value, address, or destination register.
    pub ir1: u8,
    /// The ALU's two input holders.
    pub a: u8,
    pub b: u8,
    /// Was the last value checked by `Z_LD` zero?
    pub zero: bool,
    /// Which tick of the instruction we're on (0-7).
    pub step: u8,
    /// The clock has been stopped by `HLT`.
    pub halted: bool,
    /// The output port's register.
    pub outport: u8,
    /// Every value ever latched into the output port, oldest first.
    pub output: Vec<u8>,
    /// What the input port reads.
    pub input: u8,
    /// Clock ticks so far.
    pub ticks: u64,
    /// Instructions finished so far.
    pub instructions: u64,
}

/// What happened on one tick, for tracing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    /// The opcode in IR0 when the tick started (meaningless during fetch).
    pub opcode: u8,
    /// The step counter when the tick started.
    pub step: u8,
    /// The control word the ROM produced.
    pub word: Signals,
    /// The value on the bus, or `None` if nothing drove it.
    pub bus: Option<u8>,
}

/// Something is wrong with the machine or its ROM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HwError {
    /// The program doesn't fit in memory.
    ProgramTooLarge(usize),
    /// The ROM produced a control word the wires can't carry out.
    BadControlWord {
        opcode: u8,
        step: u8,
        word: Signals,
        reason: &'static str,
    },
    /// An instruction ran for 8 ticks without `END`.
    NeverEnded,
}

impl fmt::Display for HwError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HwError::ProgramTooLarge(len) => {
                write!(f, "the program is {len} bytes but memory only holds {MEM_SIZE}")
            }
            HwError::BadControlWord { opcode, step, word, reason } => write!(
                f,
                "bad control word at opcode {opcode:#x}, step {step}: {reason} ({word})"
            ),
            HwError::NeverEnded => write!(f, "an instruction ran for 8 ticks without ending"),
        }
    }
}

impl std::error::Error for HwError {}

/// Why `run` stopped without an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The clock was stopped by `hlt` (or by an unused opcode).
    Halted,
    /// The instruction budget ran out first.
    InstructionLimit,
}

impl Machine {
    /// A powered-on machine with `program` loaded at address 0.
    pub fn new(program: &[u8]) -> Result<Machine, HwError> {
        if program.len() > MEM_SIZE {
            return Err(HwError::ProgramTooLarge(program.len()));
        }
        let mut mem = [0; MEM_SIZE];
        mem[..program.len()].copy_from_slice(program);
        Ok(Machine {
            rom: build_rom(),
            mem,
            regs: [0; 4],
            pc: 0,
            mar: 0,
            ir0: 0,
            ir1: 0,
            a: 0,
            b: 0,
            zero: false,
            step: 0,
            halted: false,
            outport: 0,
            output: Vec::new(),
            input: 0,
            ticks: 0,
            instructions: 0,
        })
    }

    /// The register number a register-select field currently names.
    fn field(&self, sel: RegSel) -> usize {
        usize::from(match sel {
            RegSel::X => (self.ir0 >> 2) & 0b11,
            RegSel::Y => self.ir0 & 0b11,
            RegSel::C => self.ir1 >> 6,
        })
    }

    /// One clock tick. Returns `None` if the clock is already stopped.
    pub fn tick(&mut self) -> Result<Option<Tick>, HwError> {
        if self.halted {
            return Ok(None);
        }

        // 1. The ROM decides what happens this tick.
        let opcode = self.ir0 >> 4;
        let step = self.step;
        let word = self.rom[usize::from(rom_address(opcode, step, self.zero))];
        let bad = |reason| HwError::BadControlWord { opcode, step, word, reason };

        // Which register do REG_OUT and REG_LD mean? (Worked out before the
        // edge, from the instruction as it is now.)
        let reg_index = word.reg_select().map(|sel| self.field(sel));
        if word.intersects(Signals::REG_OUT | Signals::REG_LD) && reg_index.is_none() {
            return Err(bad("register select is not x, y or c"));
        }
        let alu = word.alu_op();
        if word.contains(Signals::ALU_OUT) && alu.is_none() {
            return Err(bad("ALU function is not valid"));
        }
        if word.contains(Signals::PC_INC) && word.contains(Signals::PC_LD) {
            return Err(bad("PC_INC and PC_LD at the same time"));
        }

        // 2. Whoever is allowed to drive the bus does so.
        let drivers = Signals::BUS_DRIVERS.iter().filter(|d| word.contains(**d)).count();
        if drivers > 1 {
            return Err(bad("two parts drive the bus at once"));
        }
        let bus = if word.contains(Signals::PC_OUT) {
            Some(self.pc)
        } else if word.contains(Signals::RAM_OUT) {
            Some(self.mem[usize::from(self.mar)])
        } else if word.contains(Signals::IR1_OUT) {
            Some(self.ir1)
        } else if word.contains(Signals::REG_OUT) {
            reg_index.map(|i| self.regs[i])
        } else if word.contains(Signals::ALU_OUT) {
            alu.map(|op| op.apply(self.a, self.b))
        } else if word.contains(Signals::IN_OUT) {
            Some(self.input)
        } else {
            None
        };
        if bus.is_none() && word.intersects(Signals::BUS_LOADS) {
            return Err(bad("a part loads from the bus but nothing drives it"));
        }

        // 3. The clock edge: everything that loads grabs the bus at once.
        let old_mar = self.mar;
        if let Some(bus) = bus {
            if word.contains(Signals::MAR_LD) {
                self.mar = bus;
            }
            if word.contains(Signals::RAM_WE) {
                self.mem[usize::from(old_mar)] = bus;
            }
            if word.contains(Signals::IR0_LD) {
                self.ir0 = bus;
            }
            if word.contains(Signals::IR1_LD) {
                self.ir1 = bus;
            }
            if word.contains(Signals::A_LD) {
                self.a = bus;
            }
            if word.contains(Signals::B_LD) {
                self.b = bus;
            }
            if word.contains(Signals::REG_LD)
                && let Some(i) = reg_index
            {
                self.regs[i] = bus;
            }
            if word.contains(Signals::OUT_LD) {
                self.outport = bus;
                self.output.push(bus);
            }
            if word.contains(Signals::PC_LD) {
                self.pc = bus;
            }
            if word.contains(Signals::Z_LD) {
                self.zero = bus == 0;
            }
        }
        if word.contains(Signals::PC_INC) {
            self.pc = self.pc.wrapping_add(1);
        }

        // 4. The step counter, and the clock stop.
        if word.contains(Signals::END) {
            self.step = 0;
            self.instructions += 1;
        } else {
            self.step = (self.step + 1) & 0b111;
        }
        if word.contains(Signals::HLT) {
            self.halted = true;
        }
        self.ticks += 1;

        Ok(Some(Tick { opcode, step, word, bus }))
    }

    /// Runs ticks until one instruction has finished (or the clock stops),
    /// calling `on_tick` after every tick. Returns `false` if it was already halted.
    pub fn run_instruction_with(&mut self, mut on_tick: impl FnMut(&Tick, &Machine)) -> Result<bool, HwError> {
        if self.halted {
            return Ok(false);
        }
        let before = self.instructions;
        for _ in 0..8 {
            if let Some(tick) = self.tick()? {
                on_tick(&tick, self);
            }
            if self.instructions != before || self.halted {
                return Ok(true);
            }
        }
        Err(HwError::NeverEnded)
    }

    /// Runs one whole instruction. Returns `false` if it was already halted.
    pub fn run_instruction(&mut self) -> Result<bool, HwError> {
        self.run_instruction_with(|_, _| {})
    }

    /// Runs until the clock stops, an error, or `max_instructions` have finished.
    pub fn run(&mut self, max_instructions: u64) -> Result<Outcome, HwError> {
        for _ in 0..max_instructions {
            if !self.run_instruction()? {
                break;
            }
        }
        Ok(if self.halted { Outcome::Halted } else { Outcome::InstructionLimit })
    }

    /// Name of the instruction in IR0, for traces.
    pub fn current_name(&self) -> &'static str {
        Kind::from_opcode(self.ir0 >> 4).map_or("???", Kind::name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assemble(src: &str) -> Vec<u8> {
        asm::assemble(src).expect("test program should assemble").bytes
    }

    fn machine(src: &str) -> Machine {
        Machine::new(&assemble(src)).unwrap()
    }

    #[test]
    fn first_instruction_tick_by_tick() {
        // ldi r0, 5
        let mut m = machine("ldi r0, 5");
        let expect = [
            ("PC_OUT MAR_LD", Some(0x00)),
            ("RAM_OUT PC_INC IR0_LD", Some(0x60)),
            ("PC_OUT MAR_LD", Some(0x01)),
            ("RAM_OUT PC_INC IR1_LD", Some(0x05)),
            ("IR1_OUT REG_LD END REG_SEL=x", Some(0x05)),
        ];
        for (i, (words, bus)) in expect.into_iter().enumerate() {
            let tick = m.tick().unwrap().unwrap();
            assert_eq!(tick.step, u8::try_from(i).unwrap());
            assert_eq!(tick.word.to_string(), words, "tick {i}");
            assert_eq!(tick.bus, bus, "tick {i}");
        }
        assert_eq!((m.regs[0], m.pc, m.step, m.instructions), (5, 2, 0, 1));
    }

    #[test]
    fn ticks_per_instruction() {
        // (source, ticks the instruction takes)
        let cases = [
            ("nop", 5),
            ("add r0, r1, r2", 7),
            ("sub r0, r1, r2", 7),
            ("and r0, r1, r2", 7),
            ("or r0, r1, r2", 7),
            ("xor r0, r1, r2", 7),
            ("ldi r0, 1", 5),
            ("ld r0, 0", 6),
            ("st r0, 100", 6),
            ("out r0", 5),
            ("in r0", 5),
            ("jmp 0", 5),
            ("jz r0, 0", 6),
            ("jz r1, 0", 6),
            ("hlt", 5),
        ];
        for (src, ticks) in cases {
            let mut m = machine(&format!("ldi r1, 1\n{src}"));
            m.run_instruction().unwrap(); // the ldi
            let before = m.ticks;
            m.run_instruction().unwrap();
            assert_eq!(m.ticks - before, ticks, "{src}");
        }
    }

    #[test]
    fn count_program_runs() {
        let mut m = machine(
            "ldi r0, 0\nldi r1, 1\nldi r2, 10\nloop:\nout r0\nsub r0, r2, r3\njz r3, done\n\
             add r0, r1, r0\njmp loop\ndone:\nhlt",
        );
        assert_eq!(m.run(1000), Ok(Outcome::Halted));
        assert_eq!(m.output, (0..=10).collect::<Vec<u8>>());
        assert_eq!(m.regs, [10, 1, 10, 0]);
        assert_eq!(m.instructions, 56); // 57 instructions, the last (hlt) never ends
    }

    #[test]
    fn halted_machine_ignores_ticks() {
        let mut m = machine("hlt");
        assert_eq!(m.run(10), Ok(Outcome::Halted));
        let before = m.clone();
        assert_eq!(m.tick(), Ok(None));
        assert_eq!(m.run_instruction(), Ok(false));
        assert_eq!(m, before);
    }

    #[test]
    fn instruction_limit_stops_endless_loops() {
        let mut m = machine("top: jmp top");
        assert_eq!(m.run(100), Ok(Outcome::InstructionLimit));
        assert_eq!(m.instructions, 100);
    }

    #[test]
    fn unused_opcode_halts_the_clock_instead_of_erroring() {
        let mut m = Machine::new(&[0xd0, 0x00]).unwrap();
        assert_eq!(m.run(10), Ok(Outcome::Halted));
    }

    #[test]
    fn hardware_ignores_bits_the_instruction_doesnt_use() {
        // The simulator rejects this `add` (junk in byte 1's low bits); the
        // real wires never look at those bits, so the add just works.
        let mut m = Machine::new(&[0x60, 3, 0x64, 4, 0x11, 0x80 | 0x15, 0xf0, 0x00]).unwrap();
        assert_eq!(m.run(10), Ok(Outcome::Halted));
        assert_eq!(m.regs[2], 7);
    }

    /// Everything loads on the same clock edge, so a tick that both loads
    /// MAR and writes RAM writes at the *old* address, and a tick that loads
    /// IR0 and a register picks the register from the *old* instruction.
    #[test]
    fn all_loads_see_the_values_from_before_the_edge() {
        let mut m = machine("nop");
        m.pc = 0x42;
        m.mar = 10;
        m.rom[usize::from(rom_address(0, 0, false))] = Signals::PC_OUT | Signals::MAR_LD | Signals::RAM_WE;
        m.tick().unwrap();
        assert_eq!((m.mem[10], m.mar, m.mem[0x42]), (0x42, 0x42, 0));

        let mut m = machine("nop");
        m.ir0 = 0b0000_0100; // x field = r1
        m.input = 0b0000_1100; // would make x = r3 if read after the edge
        m.rom[usize::from(rom_address(0, 0, false))] =
            Signals::IN_OUT | Signals::IR0_LD | Signals::REG_LD | Signals::reg_sel(RegSel::X);
        m.tick().unwrap();
        assert_eq!((m.regs[1], m.regs[3], m.ir0), (0b1100, 0, 0b1100));
    }

    #[test]
    fn two_bus_drivers_are_caught() {
        let mut m = machine("nop");
        m.rom[usize::from(rom_address(0, 0, false))] = Signals::PC_OUT | Signals::IR1_OUT | Signals::MAR_LD;
        let err = m.tick().unwrap_err();
        assert!(matches!(err, HwError::BadControlWord { reason, .. } if reason.contains("drive the bus at once")));
    }

    #[test]
    fn loading_from_an_undriven_bus_is_caught() {
        let mut m = machine("nop");
        m.rom[usize::from(rom_address(0, 0, false))] = Signals::MAR_LD;
        let err = m.tick().unwrap_err();
        assert!(matches!(err, HwError::BadControlWord { reason, .. } if reason.contains("nothing drives")));
    }

    #[test]
    fn other_bad_control_words_are_caught() {
        for (word, reason) in [
            (Signals::PC_OUT | Signals::PC_LD | Signals::PC_INC, "PC_INC and PC_LD"),
            (Signals::REG_OUT | Signals::reg_sel(RegSel::X), ""), // valid, as a control
            (Signals::from_bytes([0, 0, 0b1100_0000]) | Signals::REG_OUT, "register select"),
            (Signals::from_bytes([0, 0, 0b0010_1000]) | Signals::ALU_OUT, "ALU function"),
        ] {
            let mut m = machine("nop");
            m.rom[usize::from(rom_address(0, 0, false))] = word;
            let result = m.tick();
            if reason.is_empty() {
                assert!(result.is_ok());
            } else {
                assert!(
                    matches!(result, Err(HwError::BadControlWord { reason: r, .. }) if r.contains(reason)),
                    "{word}: {result:?}"
                );
            }
        }
    }

    #[test]
    fn an_instruction_that_never_ends_is_caught() {
        let mut m = machine("nop");
        m.rom[usize::from(rom_address(0, 4, false))] = Signals::NONE; // nop no longer ends
        for step in 5..8 {
            m.rom[usize::from(rom_address(0, step, false))] = Signals::NONE;
        }
        assert_eq!(m.run_instruction(), Err(HwError::NeverEnded));
    }

    #[test]
    fn program_size_limit() {
        assert!(Machine::new(&[0; MEM_SIZE]).is_ok());
        assert_eq!(Machine::new(&[0; MEM_SIZE + 1]), Err(HwError::ProgramTooLarge(257)));
    }
}
