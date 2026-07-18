pub mod backend;
pub mod compiler;
pub mod ir;
pub mod lexing;
pub mod parse;
pub mod semantics;
pub mod utils;

fn main() {
    let res = compiler::compile_root_file("./interntest/main.mysz", "./interntest/main.o", &[], false);

    // let res = compiler::check_root_file("./interntest/main.mysz", &[], true);

    if res.is_err() {
        println!("{:#}", res.err().unwrap());
    }
}
