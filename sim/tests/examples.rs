//! End-to-end tests: the real example programs, assembled and run.
//!
//! `every_example_assembles_and_halts` picks up any new `.asm` file in
//! `examples/` automatically, so keep that folder for working programs only.

use sim::{Cpu, Outcome};
use std::path::PathBuf;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples")
}

fn run_example(name: &str) -> Cpu {
    let path = examples_dir().join(name);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("can't read {}: {e}", path.display()));
    let program = asm::assemble(&source)
        .unwrap_or_else(|errors| panic!("{name} failed to assemble: {errors:?}"));
    let mut cpu = Cpu::new(&program.bytes).unwrap();
    let outcome = cpu.run(10_000_000).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(outcome, Outcome::Halted, "{name} never reached hlt");
    cpu
}

#[test]
fn count_outputs_0_to_10() {
    let cpu = run_example("count.asm");
    assert_eq!(cpu.output, (0..=10).collect::<Vec<u8>>());
    assert_eq!(cpu.regs, [10, 1, 10, 0]);
}

#[test]
fn primes_outputs_every_prime_below_128() {
    let cpu = run_example("primes.asm");
    let expected: Vec<u8> = (2..128u8)
        .filter(|&n| (2..n).all(|d| n % d != 0))
        .collect();
    assert_eq!(cpu.output, expected);
    assert_eq!(cpu.steps, 72_698);
}

#[test]
fn every_example_assembles_and_halts() {
    let mut found = 0;
    for entry in std::fs::read_dir(examples_dir()).expect("examples folder should exist") {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("asm")) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            run_example(&name);
            found += 1;
        }
    }
    assert!(found >= 2, "expected at least count.asm and primes.asm, found {found}");
}
