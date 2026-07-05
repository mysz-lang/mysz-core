pub mod utils;
pub mod lexing;
pub mod parse;
pub mod semantics;
pub mod ir;
pub mod backend;

use std::fs::File;
use std::io::Write;

use lexing::lexer::Lexer;
use parse::parser::Parser as myszparser;
use semantics::analyser::Analyser;
use ir::ir::IRGen;
use backend::codegen::{Backend, Target};
use backend::codegen::nasm::NasmBackend;

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

    // target selection && codegen

    match target_str {
        "x86_64_linux" => {
            let target = Target::LINUX_X86_64_GENERIC;
            let mut codegen = NasmBackend::new(target);
            let asm = codegen.emit_program(&irgen.code);

            // Write output to the current working directory
            let mut file = File::create(output_filename)
                .map_err(|e| format!("Failed to create {}: {}", output_filename, e))?;
            
            file.write_all(asm.as_bytes())
                .map_err(|e| format!("Failed to write to {}: {}", output_filename, e))?;
            
            Ok(())
        }
        other => Err(format!("Unknown target: '{}'. Supported: x86_64_linux", other)),
    }
}