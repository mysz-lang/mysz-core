// This is a test file, use library instead of this.

pub mod utils;
pub mod lexing;
pub mod parse;
pub mod semantics;
pub mod ir;

pub mod backend;

pub mod tmp;
use std::fs::File;
use std::io::Write;

use cranelift::codegen::Context;
use cranelift_frontend::FunctionBuilderContext;
use lexing::lexer::Lexer;
use parse::parser::Parser as myszparser;
use semantics::analyser::Analyser;
use ir::ir::IRGen;
use backend::clback;

use crate::ir::tac::Instruction;
use crate::parse::parsing::Stmt;

fn main() {
    let source: String = "".to_string();

    let mut lexer = Lexer::new(source);
    lexer.lex();

    let tokens = lexer.tokens;

    let mut parser = myszparser::new(tokens);
    parser.parse();

    if !parser.parser_errs.is_empty() {
        for perror in parser.parser_errs {
            println!("{}", perror);
        }
        return;
    }

    let program = parser.ast;

    let mut analyser = Analyser::new();

    if let Err(e) = analyser.analyse(&program) {
        println!("{}", e);
        return;
    }

    // println!("{:#?}", program);

    let mut irgen = IRGen::new(analyser.types);
    irgen.gen_program(&program);

    irgen.dump();

    let tac_instructions = irgen.code;
    
    let functions_to_compile: Vec<String> = program.statements.iter().filter_map(|stmt| {
        match stmt {
            Stmt::Function { name, .. } => Some(name.value.clone()), 
            _ => None
        }
    }).collect();

    let mut backend = clback::CraneliftBackend::new();
    
    backend.scan_externs(&tac_instructions);

    for func_name in functions_to_compile {
        let func_instructions: Vec<&Instruction> = tac_instructions.iter()
            .skip_while(|inst| !matches!(inst, Instruction::FunctionLabel(name) if name == &func_name))
            .skip(1) 
            .take_while(|inst| !matches!(inst, Instruction::FunctionLabel(_)))
            .collect();

        let mut ctx = Context::new();
        let mut func_ctx = FunctionBuilderContext::new();

        backend.compile_function(
            &func_name, 
            &func_instructions, 
            &mut ctx, 
            &mut func_ctx
        );
    }
    
    let product = backend.finish(); 
    let emit_result = product.emit().expect("Failed to emit object code");

    let mut file = File::create("output.o").expect("Failed to create output file");
    file.write_all(&emit_result).expect("Failed to write binary payload to disk");

}