pub mod backend;
pub mod compiler;
pub mod ir;
pub mod lexing;
pub mod parse;
pub mod semantics;
pub mod utils;

use lexing::lexer::Lexer;
use parse::parser::Parser as myszparser;

fn main() {
    let source: &str = r#""#;

    let mut lexer = Lexer::new(source.to_string());
    let res = lexer.lex();

    if res.is_err() {
        eprintln!("{:#}", res.err().unwrap());
    }

    let mut parser = myszparser::new(lexer.tokens);
    parser.parse();
    if !parser.parser_errs.is_empty() {
        for perr in parser.parser_errs {
            eprintln!("{:#}", perr);
        }
        return;
    }

    println!("Starting compilation of inline string via compiler driver...");
    match compiler::compile_ast_program(&parser.ast, "output.o") {
        Ok(_) => println!("Compilation successful! Written to output.o"),
        Err(e) => eprintln!("Compilation failed: {}", e),
    }
}
