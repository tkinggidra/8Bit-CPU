//! A model of the 8Bit-CPU hardware, one clock tick at a time.
//!
//! - `signals`: the 24 control lines
//! - `microcode`: the control ROM, generated from the instruction set
//! - `machine`: the registers, the bus and the ROM, ticking
//!
//! The control ROM images this crate writes are what gets burned into the
//! real chips, and the machine is the reference for how the boards should
//! behave. The tests run every example (and thousands of random programs)
//! on both this machine and the instruction-level simulator and require
//! identical results.

pub mod machine;
pub mod microcode;
pub mod signals;

pub use machine::{HwError, Machine, Outcome, Tick};
pub use signals::Signals;
