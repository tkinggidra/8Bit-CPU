//! Command line for the simulator.
//!
//! ```text
//! sim <program.asm | program.bin> [--trace] [--max-steps N] [--in N]
//! ```
//!
//! - A `.asm` file is assembled first, so one command does everything.
//!   Any other file is loaded as raw machine code.
//! - `--trace` prints every instruction as it runs.
//! - `--max-steps N` stops endless loops (default 10,000,000).
//! - `--in N` sets the value the input port holds (default 0).
//!
//! Exit codes: 0 halted, 1 error, 2 wrong arguments, 3 hit the step limit.

use sim::{Cpu, Outcome};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: sim <program.asm | program.bin> [--trace] [--max-steps N] [--in N]";
const DEFAULT_MAX_STEPS: u64 = 10_000_000;

struct Options {
    path: PathBuf,
    trace: bool,
    max_steps: u64,
    input: u8,
}

fn main() -> ExitCode {
    if std::env::args().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&options) {
        Ok(Outcome::Halted) => ExitCode::SUCCESS,
        Ok(Outcome::StepLimit) => ExitCode::from(3),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut path = None;
    let mut trace = false;
    let mut max_steps = DEFAULT_MAX_STEPS;
    let mut input = 0;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--trace" => trace = true,
            "--max-steps" => {
                let value = args.next().ok_or("--max-steps needs a number")?;
                max_steps = value
                    .parse()
                    .map_err(|_| format!("--max-steps: '{value}' isn't a whole number"))?;
            }
            "--in" => {
                let value = args.next().ok_or("--in needs a number")?;
                input = parse_byte(&value)
                    .ok_or_else(|| format!("--in: '{value}' isn't a number from 0 to 255"))?;
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option '{flag}'")),
            _ if path.is_some() => return Err(format!("unexpected extra argument '{arg}'")),
            _ => path = Some(PathBuf::from(arg)),
        }
    }

    let path = path.ok_or("no program given")?;
    Ok(Options { path, trace, max_steps, input })
}

/// 0-255, as decimal or 0x hex.
fn parse_byte(text: &str) -> Option<u8> {
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// Reads a program: assembles `.asm` files, loads anything else as bytes.
fn load(path: &Path) -> Result<Vec<u8>, String> {
    let is_asm = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("asm"));
    if !is_asm {
        return std::fs::read(path).map_err(|e| format!("can't read '{}': {e}", path.display()));
    }
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("can't read '{}': {e}", path.display()))?;
    asm::assemble(&source)
        .map(|program| program.bytes)
        .map_err(|errors| {
            let mut report: Vec<String> = errors
                .iter()
                .map(|e| format!("{}:{}: error: {}", path.display(), e.line, e.message))
                .collect();
            report.push(format!("{} error(s), not running", errors.len()));
            report.join("\n")
        })
}

fn run(options: &Options) -> Result<Outcome, String> {
    let program = load(&options.path)?;
    let mut cpu = Cpu::new(&program).map_err(|e| e.to_string())?;
    cpu.input = options.input;

    // Buffered, and every write is checked: if whatever reads our output
    // closes early (e.g. `| head`), we stop quietly instead of crashing.
    let mut out = BufWriter::new(std::io::stdout().lock());

    let outcome = if options.trace {
        run_traced(&mut cpu, options.max_steps, &mut out)
    } else {
        cpu.run(options.max_steps)
    };

    // Print what happened even after an error: it's the most useful moment to see it.
    let [r0, r1, r2, r3] = cpu.regs;
    let _ = writeln!(out, "output:    {:?}", cpu.output)
        .and_then(|()| writeln!(out, "registers: r0={r0} r1={r1} r2={r2} r3={r3}"))
        .and_then(|()| writeln!(out, "pc:        {:#04x}", cpu.pc))
        .and_then(|()| writeln!(out, "steps:     {}", cpu.steps));

    let outcome = outcome.map_err(|e| {
        let _ = out.flush(); // keep the summary above the error message
        format!("error: {e}")
    })?;
    if outcome == Outcome::StepLimit {
        let _ = writeln!(
            out,
            "stopped:   hit the step limit ({}) before reaching hlt; \
             endless loop, or raise it with --max-steps",
            options.max_steps
        );
    }
    let _ = out.flush();
    Ok(outcome)
}

/// Same as `Cpu::run`, but prints each instruction as it runs. If the output
/// can no longer be written, the trace stops but the program keeps running.
fn run_traced(cpu: &mut Cpu, max_steps: u64, out: &mut impl Write) -> Result<Outcome, sim::SimError> {
    let mut printing = writeln!(out, "  step  addr  bytes  instruction       registers after").is_ok();
    for _ in 0..max_steps {
        let addr = cpu.pc;
        let [b0, b1] = cpu.fetch();
        let Some(instr) = cpu.step()? else { break };
        if printing {
            printing = writeln!(
                out,
                "{:>6}  {addr:02x}    {b0:02x} {b1:02x}  {:<16}  {:?}",
                cpu.steps,
                instr.to_string(),
                cpu.regs
            )
            .is_ok();
        }
    }
    Ok(if cpu.halted { Outcome::Halted } else { Outcome::StepLimit })
}
