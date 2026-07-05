pub mod nasm;

use crate::ir::tac::Instruction;

#[derive(Clone, Copy, Debug)]
pub enum Arch {
    X86_64,
    AARCH,
}

// windows not implemented yet
#[derive(Clone, Copy, Debug)]
pub enum OS {
    Linux,
    // Windows,
}

// windows not implemented yet
#[derive(Clone, Copy, Debug)]
pub enum ABI {
    SysV,
    // Win64,
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub arch: Arch,
    pub os: OS,
    pub abi: ABI,
}
impl Target {
    pub const LINUX_X86_64_GENERIC: Target = Target {
        arch: Arch::X86_64,
        os: OS::Linux,
        abi: ABI::SysV,
    };

    pub fn arg_reg(&self, index: usize) -> Option<&'static str> {
        match self.abi {
            ABI::SysV => match index {
                0 => Some("rdi"),
                1 => Some("rsi"),
                2 => Some("rdx"),
                3 => Some("rcx"),
                4 => Some("r8"),
                5 => Some("r9"),
                _ => None,
            },
        }
    }
}

pub trait Backend {
    fn new(target: Target) -> Self
    where
        Self: Sized;

    fn emit_program(&mut self, program: &[Instruction]) -> String;
}