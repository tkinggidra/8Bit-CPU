//! The simulator: a software copy of the CPU.
//!
//! It runs one whole instruction per `step`, which is enough to check that
//! programs and the instruction set behave. Later, the real hardware is tested
//! by running the same program on both and comparing.

use isa::{DecodeError, INSTR_SIZE, Instr, MEM_SIZE};
use std::fmt;

/// Everything the CPU knows. All fields are public so tests and tools can
/// inspect or set up any state they like.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cpu {
    /// 256 bytes of memory, holding the program and its data.
    pub mem: [u8; MEM_SIZE],
    /// Registers r0 to r3.
    pub regs: [u8; 4],
    /// Address of the next instruction.
    pub pc: u8,
    /// Set by `hlt`. A halted CPU does nothing when stepped.
    pub halted: bool,
    /// Every value sent to the output port, oldest first.
    pub output: Vec<u8>,
    /// What `in` reads. The input port holds this value until changed.
    pub input: u8,
    /// Instructions executed so far.
    pub steps: u64,
}

/// Something went wrong while running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimError {
    /// The program doesn't fit in memory.
    ProgramTooLarge(usize),
    /// The bytes at `addr` aren't a valid instruction. Nothing was changed.
    BadInstruction { addr: u8, bytes: [u8; 2], reason: DecodeError },
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SimError::ProgramTooLarge(len) => {
                write!(f, "the program is {len} bytes but memory only holds {MEM_SIZE}")
            }
            SimError::BadInstruction { addr, bytes: [b0, b1], reason } => write!(
                f,
                "bad instruction {b0:02x} {b1:02x} at address {addr:#04x}: {reason} \
                 (did the program run off its end, or into its data?)"
            ),
        }
    }
}

impl std::error::Error for SimError {}

/// Why `run` stopped without an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The program reached `hlt`.
    Halted,
    /// The step budget ran out first. Usually means an endless loop.
    StepLimit,
}

impl Cpu {
    /// A fresh CPU with `program` loaded at address 0 and everything else zero.
    pub fn new(program: &[u8]) -> Result<Cpu, SimError> {
        if program.len() > MEM_SIZE {
            return Err(SimError::ProgramTooLarge(program.len()));
        }
        let mut mem = [0; MEM_SIZE];
        mem[..program.len()].copy_from_slice(program);
        Ok(Cpu {
            mem,
            regs: [0; 4],
            pc: 0,
            halted: false,
            output: Vec::new(),
            input: 0,
            steps: 0,
        })
    }

    /// The two bytes at the program counter. Wraps from address 255 to 0,
    /// just like an 8-bit address counter would.
    pub fn fetch(&self) -> [u8; 2] {
        [self.mem[usize::from(self.pc)], self.mem[usize::from(self.pc.wrapping_add(1))]]
    }

    /// Runs one instruction and returns it, or `None` if already halted.
    /// On error the CPU is left exactly as it was.
    pub fn step(&mut self) -> Result<Option<Instr>, SimError> {
        if self.halted {
            return Ok(None);
        }

        // Fetch and decode before changing anything, so an error leaves no trace.
        let addr = self.pc;
        let bytes = self.fetch();
        let instr = Instr::decode(bytes)
            .map_err(|reason| SimError::BadInstruction { addr, bytes, reason })?;

        // The pc moves on first; jumps then overwrite it.
        self.pc = self.pc.wrapping_add(INSTR_SIZE as u8);
        self.steps += 1;

        let r = &mut self.regs;
        match instr {
            Instr::Nop => {}
            Instr::Alu { op, x, y, c } => r[c.index()] = op.apply(r[x.index()], r[y.index()]),
            Instr::Ldi { x, value } => r[x.index()] = value,
            Instr::Ld { x, addr } => r[x.index()] = self.mem[usize::from(addr)],
            Instr::St { x, addr } => self.mem[usize::from(addr)] = r[x.index()],
            Instr::Out { x } => self.output.push(r[x.index()]),
            Instr::In { x } => r[x.index()] = self.input,
            Instr::Jmp { addr } => self.pc = addr,
            Instr::Jz { x, addr } => {
                if r[x.index()] == 0 {
                    self.pc = addr;
                }
            }
            Instr::Hlt => self.halted = true,
        }
        Ok(Some(instr))
    }

    /// Steps until `hlt`, an error, or `max_steps` instructions have run.
    pub fn run(&mut self, max_steps: u64) -> Result<Outcome, SimError> {
        for _ in 0..max_steps {
            if self.step()?.is_none() {
                break;
            }
        }
        Ok(if self.halted { Outcome::Halted } else { Outcome::StepLimit })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Assembles and runs `src`, which must halt within 10,000 steps.
    fn run(src: &str) -> Cpu {
        let program = asm::assemble(src).expect("test program should assemble");
        let mut cpu = Cpu::new(&program.bytes).unwrap();
        assert_eq!(cpu.run(10_000), Ok(Outcome::Halted), "program did not halt");
        cpu
    }

    #[test]
    fn counts_to_ten() {
        let cpu = run("
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
            hlt");
        assert_eq!(cpu.output, (0..=10).collect::<Vec<u8>>());
        assert_eq!(cpu.regs, [10, 1, 10, 0]);
        assert_eq!(cpu.steps, 3 + 10 * 5 + 3 + 1);
    }

    #[test]
    fn every_alu_op() {
        let cpu = run("
            ldi r0, 0b1100
            ldi r1, 0b1010
            add r0, r1, r2
            out r2
            sub r1, r0, r2
            out r2
            and r0, r1, r2
            out r2
            or r0, r1, r2
            out r2
            xor r0, r1, r2
            out r2
            hlt");
        assert_eq!(cpu.output, vec![22, 254, 0b1000, 0b1110, 0b0110]);
    }

    #[test]
    fn destination_can_be_a_source() {
        let cpu = run("ldi r0, 5\nadd r0, r0, r0\nhlt");
        assert_eq!(cpu.regs[0], 10);
    }

    #[test]
    fn store_load_and_byte_data() {
        let cpu = run("
            ldi r0, 42
            st r0, 200
            ld r1, 200
            ld r2, value
            hlt
        value: .byte 99, 0");
        assert_eq!(cpu.regs[1..3], [42, 99]);
        assert_eq!(cpu.mem[200], 42);
    }

    #[test]
    fn input_port() {
        let program = asm::assemble("in r0\nout r0\nhlt").unwrap();
        let mut cpu = Cpu::new(&program.bytes).unwrap();
        cpu.input = 77;
        cpu.run(10).unwrap();
        assert_eq!(cpu.output, vec![77]);
    }

    #[test]
    fn jz_only_jumps_on_zero() {
        let cpu = run("
            ldi r0, 1
            jz r0, skip
            out r0
        skip:
            ldi r0, 0
            jz r0, end
            out r0
        end:
            hlt");
        assert_eq!(cpu.output, vec![1]);
    }

    #[test]
    fn programs_can_modify_themselves() {
        // Overwrite the byte 1 of `patch` so `ldi r1, 0` becomes `ldi r1, 9`.
        let cpu = run("
            ldi r0, 9
            st r0, 5
        patch:
            ldi r1, 0
            hlt");
        assert_eq!(cpu.regs[1], 9);
    }

    #[test]
    fn halted_cpu_ignores_step() {
        let mut cpu = run("hlt");
        let before = cpu.clone();
        assert_eq!(cpu.step(), Ok(None));
        assert_eq!(cpu.run(100), Ok(Outcome::Halted));
        assert_eq!(cpu, before);
    }

    #[test]
    fn endless_loop_hits_the_step_limit() {
        let program = asm::assemble("top: jmp top").unwrap();
        let mut cpu = Cpu::new(&program.bytes).unwrap();
        assert_eq!(cpu.run(50), Ok(Outcome::StepLimit));
        assert_eq!(cpu.steps, 50);
        assert_eq!(cpu.run(0), Ok(Outcome::StepLimit));
        assert_eq!(cpu.steps, 50);
    }

    #[test]
    fn bad_instruction_is_an_error_and_changes_nothing() {
        let mut cpu = Cpu::new(&[0x60, 0x05, 0xd0, 0x00]).unwrap(); // ldi r0,5 ; opcode 0xD
        cpu.step().unwrap();
        let before = cpu.clone();
        let err = cpu.step().unwrap_err();
        assert_eq!(
            err,
            SimError::BadInstruction {
                addr: 2,
                bytes: [0xd0, 0x00],
                reason: DecodeError::UnknownOpcode(0xd)
            }
        );
        assert_eq!(cpu, before);
        assert_eq!(cpu.run(10), Err(err));
    }

    #[test]
    fn running_off_the_end_into_zeros_is_a_nop_slide() {
        // Empty memory is all `nop`, so the pc wraps around forever.
        let mut cpu = Cpu::new(&[]).unwrap();
        assert_eq!(cpu.run(200), Ok(Outcome::StepLimit));
        assert_eq!(cpu.pc, (200u32 * 2 % 256) as u8);
    }

    #[test]
    fn pc_wraps_from_255_to_0() {
        let mut cpu = Cpu::new(&[]).unwrap();
        cpu.mem[254] = 0x60; // ldi r0, 7 at the very end of memory
        cpu.mem[255] = 7;
        cpu.mem[0] = 0xf0; // hlt at address 0
        cpu.pc = 254;
        assert_eq!(cpu.run(10), Ok(Outcome::Halted));
        assert_eq!(cpu.regs[0], 7);

        // An instruction straddling the end reads its second byte from address 0.
        let mut cpu = Cpu::new(&[]).unwrap();
        cpu.mem[255] = 0x60;
        cpu.mem[0] = 9;
        cpu.pc = 255;
        assert_eq!(cpu.fetch(), [0x60, 9]);
    }

    #[test]
    fn program_size_limits() {
        assert!(Cpu::new(&[0; MEM_SIZE]).is_ok());
        assert_eq!(Cpu::new(&[0; MEM_SIZE + 1]), Err(SimError::ProgramTooLarge(257)));
    }
}
