//! Command line for the assembler.
//!
//! ```text
//! asm <input.asm> <output.bin>
//! ```
//!
//! Prints a listing (address, bytes, source) and writes the machine code.
//! Exit codes: 0 success, 1 assembly or file error, 2 wrong arguments.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("usage: asm <input.asm> <output.bin>");
        return ExitCode::from(2);
    };
    match run(Path::new(input), Path::new(output)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(input: &Path, output: &Path) -> Result<(), String> {
    // Guard against swapped arguments, which would overwrite the source file.
    if output
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("asm"))
    {
        return Err(format!(
            "refusing to write machine code over '{}' (did you swap the arguments?)",
            output.display()
        ));
    }
    if same_file(input, output) {
        return Err("the input and output are the same file".to_string());
    }

    let source = std::fs::read_to_string(input)
        .map_err(|e| format!("can't read '{}': {e}", input.display()))?;

    let program = asm::assemble(&source).map_err(|errors| {
        // "file:line: message" is the format editors like VS Code make clickable.
        let mut report: Vec<String> = errors
            .iter()
            .map(|e| format!("{}:{}: error: {}", input.display(), e.line, e.message))
            .collect();
        report.push(format!("{} error(s), nothing written", errors.len()));
        report.join("\n")
    })?;

    for line in &program.listing {
        let hex: Vec<String> = line.bytes.iter().map(|b| format!("{b:02x}")).collect();
        println!("{:02x}: {:<8} {}", line.addr, hex.join(" "), line.source);
    }

    std::fs::write(output, &program.bytes)
        .map_err(|e| format!("can't write '{}': {e}", output.display()))?;
    println!("{} bytes written to {}", program.bytes.len(), output.display());
    Ok(())
}

/// True if both paths point at the same existing file.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false, // the output usually doesn't exist yet
    }
}
