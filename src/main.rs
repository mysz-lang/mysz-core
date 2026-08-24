use crate::utils::ctx::{CompilerCtx, CompilerTarget};

pub mod backend;
pub mod compiler;
pub mod ir;
pub mod lex;
pub mod parse;
pub mod semantics;
pub mod utils;

fn main() {
    let ctx = CompilerCtx::new("./interntest/main.mysz", &[], false, CompilerTarget::Cranelift);

    let res =
        compiler::compile_root_file(ctx, "./interntest/main.o");

    if res.is_err() {
        println!("{:#}", res.err().unwrap());
    }
}
