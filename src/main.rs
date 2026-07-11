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
    let source: &str = r#"
// this is a comment!

//'
This is a multi-line
comment
'//

//' testing single-line multi-line comment '//

extern fn print_str(val: str);

fn pub main(): int {
    var x: str = "https://github.com";
    print_str(x);
    return 0;
};
    "#;

    let mut lexer = Lexer::new(source.to_string());
    lexer.lex();

    let mut parser = myszparser::new(lexer.tokens);
    parser.parse();
    if !parser.parser_errs.is_empty() {
        eprintln!("Parsing inline source failed.");
        return;
    }

    println!("Starting compilation of inline string via compiler driver...");
    match compiler::compile_ast_program(&parser.ast, "output.o") {
        Ok(_) => println!("Compilation successful! Written to output.o"),
        Err(e) => eprintln!("Compilation failed: {}", e),
    }
}
