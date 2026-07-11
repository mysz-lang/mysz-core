// This is a test file, use library instead of this.

pub mod backend;
pub mod ir;
pub mod lexing;
pub mod main_helper;
pub mod parse;
pub mod semantics;
pub mod utils;

use std::fs::File;
use std::io::{Read, Write};

use backend::clback;
use cranelift::codegen::Context;
use cranelift_frontend::FunctionBuilderContext;
use ir::ir::IRGen;
use lexing::lexer::Lexer;
use parse::parser::Parser as myszparser;
use semantics::analyser::Analyser;

use crate::ir::tac::Instruction;
use crate::parse::parsing::{Program, Stmt};

fn main() {
    let source: String = "
struct MyszString {
    length: int,
    data: ptr<char>,
};

extern fn print_String(val: ptr<MyszString>);
extern fn mysz_string_new(val: str): MyszString;
extern fn mysz_string_destroy(val: ptr<MyszString>);

fn pub main(): int {
    var string: MyszString = mysz_string_new(\"Hello, world!\");
    print_String(&string);
    
    string.data[12] = '?';
    print_String(&string);

    mysz_string_destroy(&string);
    return 0;
};".to_string();

    let mut lexer = Lexer::new(source);
    lexer.lex();

    // println!("{:#?}", lexer.tokens);

    let mut parser = myszparser::new(lexer.tokens);
    parser.parse();

    if !parser.parser_errs.is_empty() {
        for perror in parser.parser_errs {
            println!("{}", perror);
        }
        return;
    }

    // println!("{:#?}", parser.ast);

    let initial_program = parser.ast;

    let mut flattened_statements = Vec::new();

    for stmt in initial_program.statements {
        if let Stmt::Use { path } = stmt {
            let module_filename = format!("{}.mysz", path.join("/"));

            let mut mod_file = File::open(&module_filename)
                .unwrap_or_else(|_| panic!("Failed to open module file: {}", module_filename));
            let mut mod_source = String::new();
            mod_file
                .read_to_string(&mut mod_source)
                .expect("Failed to read module source");

            let mut mod_lexer = Lexer::new(mod_source);
            mod_lexer.lex();

            let mut mod_parser = myszparser::new(mod_lexer.tokens);
            mod_parser.parse();

            if !mod_parser.parser_errs.is_empty() {
                println!("Errors parsing standard module {}:", module_filename);
                for perror in mod_parser.parser_errs {
                    println!("{}", perror);
                }
                return;
            }

            flattened_statements.extend(mod_parser.ast.statements);
        } else {
            flattened_statements.push(stmt);
        }
    }

    let program = Program {
        statements: flattened_statements,
    };

    let mut analyser = Analyser::new();
    if let Err(e) = analyser.analyse(&program) {
        println!("{}", e);
        return;
    }

    // println!("analyser types:\n{:#?}", analyser.types);

    let mut irgen = IRGen::new(analyser.types);
    irgen.gen_program(&program);
    irgen.dump();

    let tac_instructions = irgen.code;

    let functions_to_compile: Vec<(String, bool)> = program
        .statements
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::Function { name, public, .. } => Some((name.value.clone(), *public)),
            _ => None,
        })
        .collect();

    let mut backend = clback::CraneliftBackend::new(irgen.struct_defs);
    backend.scan_externs(&tac_instructions);

    let instruction_refs: Vec<&Instruction> = tac_instructions.iter().collect();
    backend.pre_declare_strings(&instruction_refs);

    for (func_name, is_public) in functions_to_compile {
        let func_instructions: Vec<&Instruction> = tac_instructions
            .iter()
            .skip_while(
                |inst| !matches!(inst, Instruction::FunctionLabel(name) if name == &func_name),
            )
            .skip(1)
            .take_while(|inst| !matches!(inst, Instruction::FunctionLabel(_)))
            .collect();

        let mut ctx = Context::new();
        let mut func_ctx = FunctionBuilderContext::new();

        backend.compile_function(
            &func_name,
            is_public,
            &func_instructions,
            &mut ctx,
            &mut func_ctx,
            &irgen.var_types,
        );
    }

    let product = backend.finish();
    let emit_result = product.emit().expect("Failed to emit object code");

    let mut file = File::create("output.o").expect("Failed to create output file");
    file.write_all(&emit_result)
        .expect("Failed to write binary payload to disk");
}
