pub mod utils;
pub mod lexing;
pub mod parse;

use lexing::lex::Lexer;
use parse::parse::Parser;

fn main() {
    let source: String = "if (1+1) {var x = 20}".to_string();

    let mut lexer = Lexer::new(source);

    lexer.lex();

    let tokens = lexer.tokens;

    // for tok in tokens {
    //     println!("{}", tok);
    // }

    let mut parser = Parser::new(tokens);

    parser.parse();

    for st in parser.ast.statements {
        print!("{:?}", st)
    }
}
