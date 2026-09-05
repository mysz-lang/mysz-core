use crate::athelp::at_from_directory;

use crate::compiler::compile_at_graph;
use crate::utils::ats::ATEntry;
use crate::utils::ctx::CompilerCtx;

pub mod athelp;
pub mod backend;
pub mod compiler;
pub mod ir;
pub mod lex;
pub mod parse;
pub mod semantics;
pub mod utils;

fn main() {
    let at_info = at_from_directory("main", "./interntest");

    if let Err(e) = at_info {
        eprintln!("Error: {}", e);
        return;
    }

    let at_info = at_info.unwrap();

    let ats = vec![at_info];
    let entry = ATEntry {
        info: 0,
        is_current: true,
    };
    let ctx = CompilerCtx {
        input_path: "./interntest/main.mysz",
        search_paths: &["./interntest".into()],
        output_json: false,
        target: crate::utils::ctx::CompilerTarget::Cranelift,
    };
    let res = compile_at_graph(&ctx, &ats, &entry, "./interntest/main.o");

    if let Err(e) = res {
        eprintln!("Compilation error: {}", e);
    } else {
        println!("Compilation successful!");
    }
}
