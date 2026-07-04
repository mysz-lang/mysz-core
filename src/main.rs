pub mod utils;
pub mod lexing;
pub mod parse;
pub mod semantics;

use lexing::lexer::Lexer;
use parse::parser::Parser;
use semantics::analyser::Analyser;
fn main() {
    let source: String = "var x = 0;\n if (x) {}".to_string();

    let mut lexer = Lexer::new(source);

    lexer.lex();

    let tokens = lexer.tokens;

    // for tok in tokens {
    //     println!("{}", tok);
    // }

    let mut parser = Parser::new(tokens);

    parser.parse();

    if parser.parser_errs.len() > 0 {
        for perror in parser.parser_errs {
            println!("{}", perror);
        }
    } else {

        let program = parser.ast;

        // for st in parser.ast.statements {
        //     print!("{:?}", st)
        // }

        let mut analyser = Analyser::new();

        let res = analyser.analyse(&program);
        
        if res.is_err() {
            println!("{}", res.err().unwrap())
        }
    }
}
