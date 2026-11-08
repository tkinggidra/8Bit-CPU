//! Command line for the hardware model.
//!
//! ```text
//! hw rom <folder>
//!     Writes control0.bin, control1.bin, control2.bin (one image per ROM
//!     chip) and microcode.txt (a readable description) into <folder>.
//!
//! hw run <program.asm | program.bin> [--trace] [--max-instructions N] [--in N]
//!     Runs a program on the tick-by-tick model. --trace prints every clock tick.
//! ```
//!
//! Exit codes: 0 success/halted, 1 error, 2 wrong arguments, 3 hit the limit.

use hw::{Machine, Outcome};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage:\n  hw rom <folder>\n  \
                     hw run <program.asm | program.bin> [--trace] [--max-instructions N] [--in N]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let result = match args.split_first() {
        Some((cmd, rest)) if cmd == "rom" => match rest {
            [folder] => write_rom(Path::new(folder)).map(|()| ExitCode::SUCCESS),
            _ => return usage_error("'rom' takes one folder"),
        },
        Some((cmd, rest)) if cmd == "run" => match parse_run(rest) {
            Ok(options) => run(&options).map(|outcome| match outcome {
                Outcome::Halted => ExitCode::SUCCESS,
                Outcome::InstructionLimit => ExitCode::from(3),
            }),
            Err(message) => return usage_error(&message),
        },
        Some((cmd, _)) => return usage_error(&format!("unknown command '{cmd}'")),
        None => return usage_error("no command given"),
    };
    result.unwrap_or_else(|message| {
        eprintln!("{message}");
        ExitCode::FAILURE
    })
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("{message}\n{USAGE}");
    ExitCode::from(2)
}

// ---------------------------------------------------------------------------
// hw rom
// ---------------------------------------------------------------------------

fn write_rom(folder: &Path) -> Result<(), String> {
    let fail = |what: &Path, e: std::io::Error| format!("can't write '{}': {e}", what.display());
    std::fs::create_dir_all(folder).map_err(|e| fail(folder, e))?;

    let rom = hw::microcode::build_rom();
    for (chip, image) in hw::microcode::rom_images(&rom).iter().enumerate() {
        let path = folder.join(format!("control{chip}.bin"));
        std::fs::write(&path, image).map_err(|e| fail(&path, e))?;
        println!("wrote {} (ROM chip {chip}, {} bytes)", path.display(), image.len());
    }
    let path = folder.join("microcode.txt");
    std::fs::write(&path, hw::microcode::listing()).map_err(|e| fail(&path, e))?;
    println!("wrote {}", path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// hw run
// ---------------------------------------------------------------------------

struct RunOptions<'a> {
    path: &'a Path,
    trace: bool,
    max_instructions: u64,
    input: u8,
}

fn parse_run(args: &[String]) -> Result<RunOptions<'_>, String> {
    let mut path = None;
    let mut trace = false;
    let mut max_instructions = 10_000_000;
    let mut input = 0;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--trace" => trace = true,
            "--max-instructions" => {
                let value = args.next().ok_or("--max-instructions needs a number")?;
                max_instructions = value
                    .parse()
                    .map_err(|_| format!("--max-instructions: '{value}' isn't a whole number"))?;
            }
            "--in" => {
                let value = args.next().ok_or("--in needs a number")?;
                input = parse_byte(value)
                    .ok_or_else(|| format!("--in: '{value}' isn't a number from 0 to 255"))?;
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option '{flag}'")),
            _ if path.is_some() => return Err(format!("unexpected extra argument '{arg}'")),
            _ => path = Some(Path::new(arg)),
        }
    }
    let path = path.ok_or("no program given")?;
    Ok(RunOptions { path, trace, max_instructions, input })
}

/// 0-255, as decimal or 0x hex.
fn parse_byte(text: &str) -> Option<u8> {
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// Assembles `.asm` files; loads anything else as raw machine code.
fn load(path: &Path) -> Result<Vec<u8>, String> {
    let read_error = |e: std::io::Error| format!("can't read '{}': {e}", path.display());
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("asm")) {
        return std::fs::read(path).map_err(read_error);
    }
    let source = std::fs::read_to_string(path).map_err(read_error)?;
    asm::assemble(&source).map(|p| p.bytes).map_err(|errors| {
        let mut report: Vec<String> = errors
            .iter()
            .map(|e| format!("{}:{}: error: {}", path.display(), e.line, e.message))
            .collect();
        report.push(format!("{} error(s), not running", errors.len()));
        report.join("\n")
    })
}

fn run(options: &RunOptions) -> Result<Outcome, String> {
    let program = load(options.path)?;
    let mut m = Machine::new(&program).map_err(|e| e.to_string())?;
    m.input = options.input;

    // Every write is checked, so a closed pipe (e.g. `| head`) stops printing
    // instead of crashing.
    let mut out = BufWriter::new(std::io::stdout().lock());
    let mut printing = !options.trace
        || writeln!(out, "instr  step  phase  bus  control lines").is_ok();

    let mut result = Ok(());
    for _ in 0..options.max_instructions {
        let number = m.instructions + 1;
        let ran = m.run_instruction_with(|tick, machine| {
            if options.trace && printing {
                let phase = if tick.step < 4 { "fetch" } else { machine.current_name() };
                let bus = tick.bus.map_or("--".to_string(), |b| format!("{b:02x}"));
                printing = writeln!(
                    out,
                    "{number:>5}  {:>4}  {phase:<5}  {bus}   {}",
                    tick.step, tick.word
                )
                .is_ok();
            }
        });
        match ran {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                result = Err(format!("error: {e}"));
                break;
            }
        }
    }

    let [r0, r1, r2, r3] = m.regs;
    let _ = writeln!(out, "output:       {:?}", m.output)
        .and_then(|()| writeln!(out, "registers:    r0={r0} r1={r1} r2={r2} r3={r3}"))
        .and_then(|()| writeln!(out, "pc:           {:#04x}", m.pc))
        .and_then(|()| writeln!(out, "instructions: {}", m.instructions + u64::from(m.halted)))
        .and_then(|()| writeln!(out, "clock ticks:  {}", m.ticks));
    let _ = out.flush();
    result?;

    if m.halted {
        Ok(Outcome::Halted)
    } else {
        let _ = writeln!(
            out,
            "stopped:      hit the limit ({}) before reaching hlt; raise it with --max-instructions",
            options.max_instructions
        );
        let _ = out.flush();
        Ok(Outcome::InstructionLimit)
    }
}
