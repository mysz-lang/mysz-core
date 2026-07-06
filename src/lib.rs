pub mod utils;
pub mod lexing;
pub mod parse;
pub mod semantics;
pub mod ir;
pub mod backend;

use std::fs::File;
use std::io::Write;

use cranelift::codegen::Context;
use cranelift_frontend::FunctionBuilderContext;
use lexing::lexer::Lexer;
use parse::parser::Parser as myszparser;
use semantics::analyser::Analyser;
use ir::ir::IRGen;

use crate::backend::clback;
use crate::ir::tac::Instruction;

pub fn compile_source(source: &str, target_str: &str, output_filename: &str) -> Result<(), String> {

    // lexing

    let mut lexer = Lexer::new(source.to_string());
    lexer.lex();
    let tokens = lexer.tokens;

    // parsing

    let mut parser = myszparser::new(tokens);
    parser.parse();

    if !parser.parser_errs.is_empty() {
        let errs: Vec<String> = parser.parser_errs.iter().map(|e| e.to_string()).collect();
        return Err(errs.join("\n"));
    }
    let program = parser.ast;

    // semantics

    let mut analyser = Analyser::new();
    if let Err(e) = analyser.analyse(&program) {
        return Err(e.to_string());
    }

    // IR generation

    let mut irgen = IRGen::new(analyser.types);
    irgen.gen_program(&program);

    let tac_instructions = irgen.code;

    let instruction_refs: Vec<&Instruction> = tac_instructions.iter().collect();

    let mut backend = clback::CraneliftBackend::new();

    let mut ctx = Context::new();
    let mut func_ctx = FunctionBuilderContext::new();

    // entry point will always be main
    backend.compile_function(
        "main", 
        &instruction_refs, 
        &mut ctx, 
        &mut func_ctx
    );

    Ok(())
}