pub mod backend;
pub mod compiler;
pub mod ir;
pub mod lex;
pub mod parse;
pub mod semantics;
pub mod utils;
use std::path::{Path};

use crate::utils::ctx::CompilerCtx;

pub fn compile_file<'a, P: AsRef<Path>>(
    ctx: CompilerCtx<'a, P>,
    output_filename: &str,
) -> Result<(), String> {
    compiler::compile_root_file(ctx, output_filename)
}

pub fn check_file<'a, P: AsRef<Path>>(
    ctx: CompilerCtx<'a, P>,
) -> Result<(), String> {
    compiler::check_root_file(ctx)
}
