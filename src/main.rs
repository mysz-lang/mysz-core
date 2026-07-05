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

use clap::Parser;
use lexing::lexer::Lexer;
use parse::parser::Parser as myszparser;
use semantics::analyser::Analyser;
use ir::ir::IRGen;

use crate::backend::codegen::{Backend, Target};
use crate::backend::codegen::nasm::NasmBackend;

use crate::tmp::{Cli, Commands};

fn main() {
    let source: String =
        "fn main(): int {if (0) {var y = 0;} else {var y = 1;}; return 0;}"
        
        .to_string();

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

    let mut irgen = IRGen::new(analyser.types);
    irgen.gen_program(&program);

    // irgen.dump();

    let cli = Cli::parse();

    match cli.command {
        Commands::Target { target: t } => {
            match t.as_str() {
                "x86_64_linux" => {
                    let target = Target::LINUX_X86_64_GENERIC;
                    let mut codegen = NasmBackend::new(target);
                    let asm = codegen.emit_program(&irgen.code);

                    let filename = "output.asm";
                    match File::create(filename) {
                        Ok(mut file) => {
                            if let Err(e) = file.write_all(asm.as_bytes()) {
                                eprintln!("Failed to write to {}: {}", filename, e);
                            } else {
                                println!("Successfully wrote assembly to {}", filename);
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to create {}: {}", filename, e);
                        }
                    }
                }

                other => {
                    eprintln!(
                        "Unknown target: '{}'. Supported: x86_64_linux",
                        other
                    );
                    std::process::exit(1);
                }
            }
        }
    }
}