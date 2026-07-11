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
struct MyszArray<T> {
    length: int,
    capacity: int,
    elementSize: int,
    data: ptr<T>,
};

extern fn mysz_array_init<T>(arr: ptr<MyszArray<T>>, elementSize: int);
fn pub MyszArray_init<T>(arr: ptr<MyszArray<T>>) {
    mysz_array_init::<T>(arr, sizeof(T)); 
};

extern fn mysz_array_reserve<T>(arr: ptr<MyszArray<T>>, minCapacity: int);
extern fn mysz_array_push<T>(arr: ptr<MyszArray<T>>, element: ptr<T>);
extern fn mysz_array_destroy<T>(arr: ptr<MyszArray<T>>);

extern fn print_char(val: char);

fn pub main(): int {
    var x: MyszArray<char>;
    MyszArray_init::<char>(&x);
    
    mysz_array_push::<char>(&x, &'H');
    mysz_array_push::<char>(&x, &'e');
    mysz_array_push::<char>(&x, &'l');
    mysz_array_push::<char>(&x, &'l');
    mysz_array_push::<char>(&x, &'o');
    mysz_array_push::<char>(&x, &'!');

    for (var i = 0; i < x.length; i = i + 1) {
        print_char(x.data[i]);
    };
    
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
