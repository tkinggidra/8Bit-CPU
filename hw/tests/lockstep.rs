//! The hardware model must behave exactly like the simulator.
//!
//! Both run the same program side by side. After every instruction, the
//! registers, program counter, all 256 bytes of memory, the output and the
//! halted flag must match. This is the same check the real boards will get:
//! same program, compare against the simulator.

use hw::Machine;
use isa::{AluOp, Instr, Reg};
use std::path::PathBuf;

/// Runs `program` on both, comparing after every instruction. Returns how many
/// instructions were compared. Stops early where the simulator rejects an
/// instruction, since the hardware deliberately doesn't check reserved bits.
fn lockstep(name: &str, program: &[u8], input: u8, max_instructions: u64) -> u64 {
    let mut cpu = sim::Cpu::new(program).unwrap();
    let mut hw = Machine::new(program).unwrap();
    cpu.input = input;
    hw.input = input;

    let mut compared = 0;
    for _ in 0..max_instructions {
        let before_pc = cpu.pc;
        match cpu.step() {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
        hw.run_instruction()
            .unwrap_or_else(|e| panic!("{name}: hardware error after {compared} instructions: {e}"));
        compared += 1;

        let at = format!("{name}: instruction {compared} (at {before_pc:#04x})");
        assert_eq!(hw.regs, cpu.regs, "{at}: registers differ");
        assert_eq!(hw.pc, cpu.pc, "{at}: pc differs");
        assert_eq!(hw.halted, cpu.halted, "{at}: halted differs");
        assert_eq!(hw.output, cpu.output, "{at}: output differs");
        assert!(hw.mem == cpu.mem, "{at}: memory differs");
        if cpu.halted {
            break;
        }
    }
    compared
}

#[test]
fn every_example_matches_the_simulator() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let mut found = 0;
    for entry in std::fs::read_dir(&dir).expect("examples folder should exist") {
        let path = entry.unwrap().path();
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("asm")) {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let source = std::fs::read_to_string(&path).unwrap();
        let program = asm::assemble(&source).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let compared = lockstep(&name, &program.bytes, 0, 10_000_000);
        assert!(compared > 0, "{name}: nothing ran");
        found += 1;
    }
    assert!(found >= 2);
}

/// A small, fixed-seed random generator (xorshift), so failures are repeatable.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u8 {
        u8::try_from(self.next() % n).unwrap()
    }
    fn reg(&mut self) -> Reg {
        Reg::new(self.below(4)).unwrap()
    }
}

/// A random program: 64 instructions in 0x00-0x7f, random data in 0x80-0xff.
/// Jumps land on instructions; loads and stores mostly hit the data, but
/// sometimes the program itself, so self-modifying code gets tested too.
fn random_program(rng: &mut Rng) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(256);
    for _ in 0..64 {
        let code_addr = |rng: &mut Rng| rng.below(64) * 2;
        let data_addr = |rng: &mut Rng| {
            if rng.below(16) == 0 { rng.below(256) } else { 128 + rng.below(128) }
        };
        let alu = [AluOp::Add, AluOp::Sub, AluOp::And, AluOp::Or, AluOp::Xor];
        let instr = match rng.below(200) {
            0 => Instr::Hlt,
            1..=10 => Instr::Nop,
            11..=70 => Instr::Alu {
                op: alu[usize::from(rng.below(5))],
                x: rng.reg(),
                y: rng.reg(),
                c: rng.reg(),
            },
            71..=95 => Instr::Ldi { x: rng.reg(), value: rng.below(256) },
            96..=115 => Instr::Ld { x: rng.reg(), addr: data_addr(rng) },
            116..=135 => Instr::St { x: rng.reg(), addr: data_addr(rng) },
            136..=150 => Instr::Out { x: rng.reg() },
            151..=160 => Instr::In { x: rng.reg() },
            161..=175 => Instr::Jmp { addr: code_addr(rng) },
            _ => Instr::Jz { x: rng.reg(), addr: code_addr(rng) },
        };
        bytes.extend(instr.encode());
    }
    while bytes.len() < 256 {
        bytes.push(rng.below(256));
    }
    bytes
}

#[test]
fn thousands_of_random_programs_match_the_simulator() {
    let mut rng = Rng(0x8B17_C0DE_2026_1108);
    let mut total = 0;
    for n in 0..3000 {
        let program = random_program(&mut rng);
        let input = rng.below(256);
        total += lockstep(&format!("random program {n}"), &program, input, 2000);
    }
    println!("random programs: {total} instructions compared");
    // Guard against the test quietly checking almost nothing.
    assert!(total > 500_000, "only {total} instructions were compared");
}

#[test]
fn hardware_takes_the_expected_ticks_on_the_examples() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let source = std::fs::read_to_string(dir.join("primes.asm")).unwrap();
    let program = asm::assemble(&source).unwrap();
    let mut hw = Machine::new(&program.bytes).unwrap();
    assert_eq!(hw.run(10_000_000), Ok(hw::Outcome::Halted));
    assert_eq!(hw.instructions + 1, 72_698); // + the hlt, which never "ends"
    println!("primes: {} ticks", hw.ticks);
}
