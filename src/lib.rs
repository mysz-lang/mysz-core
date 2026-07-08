pub mod utils;
pub mod lexing;
pub mod parse;
pub mod semantics;
pub mod ir;
pub mod backend;

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use cranelift::codegen::Context;
use cranelift_frontend::FunctionBuilderContext;
use lexing::lexer::Lexer;
use parse::parser::Parser as myszparser;
use semantics::analyser::Analyser;
use ir::ir::IRGen;

use crate::backend::clback;
use crate::ir::tac::Instruction;
use crate::parse::parsing::{Stmt, Program};

fn find_module_file(path: &[String], search_paths: &[PathBuf]) -> Option<PathBuf> {
    let rel_path_str = format!("{}.mysz", path.join("/"));
    let rel_path = Path::new(&rel_path_str);

    for dir in search_paths {
        let full_path = dir.join(rel_path);
        if full_path.exists() {
            return Some(full_path);
        }
    }

    if rel_path.exists() {
        return Some(rel_path.to_path_buf());
    }

    None
}

pub fn compile_source(
    source: &str, 
    output_filename: &str, 
    search_paths: &[PathBuf]
) -> Result<(), String> {
    // Lexing
    let mut lexer = Lexer::new(source.to_string());
    lexer.lex();
    let tokens = lexer.tokens;

    // Parsing
    let mut parser = myszparser::new(tokens);
    parser.parse();

    if !parser.parser_errs.is_empty() {
        let errs: Vec<String> = parser.parser_errs.iter().map(|e| e.to_string()).collect();
        return Err(errs.join("\n"));
    }
    let initial_program = parser.ast;

    let mut flattened_statements = Vec::new();

    for stmt in initial_program.statements {
        if let Stmt::Use { path } = stmt {
            let resolved_path = find_module_file(&path, search_paths).ok_or_else(|| {
                format!("Module Error: Could not find module '{}' in search paths or CWD.", path.join("::"))
            })?;
            
            let mut mod_file = File::open(&resolved_path)
                .map_err(|e| format!("Failed to open module file {:?}: {}", resolved_path, e))?;
            let mut mod_source = String::new();
            mod_file.read_to_string(&mut mod_source)
                .map_err(|e| format!("Failed to read module file {:?}: {}", resolved_path, e))?;

            let mut mod_lexer = Lexer::new(mod_source);
            mod_lexer.lex();
            
            let mut mod_parser = myszparser::new(mod_lexer.tokens);
            mod_parser.parse();
            
            if !mod_parser.parser_errs.is_empty() {
                let errs: Vec<String> = mod_parser.parser_errs.iter().map(|e| e.to_string()).collect();
                return Err(format!("Errors inside parsed module {:?}:\n{}", resolved_path, errs.join("\n")));
            }

            flattened_statements.extend(mod_parser.ast.statements);
        } else {
            flattened_statements.push(stmt);
        }
    }

    let program = Program { statements: flattened_statements };

    // Semantics
    let mut analyser = Analyser::new();
    if let Err(e) = analyser.analyse(&program) {
        return Err(e.to_string());
    }

    // IR generation
    let mut irgen = IRGen::new(analyser.types);
    irgen.gen_program(&program);

    let tac_instructions = irgen.code;
    
    let functions_to_compile: Vec<(String, bool)> = program.statements.iter().filter_map(|stmt| {
        match stmt {
            Stmt::Function { name, public, .. } => Some((name.value.clone(), *public)), 
            _ => None
        }
    }).collect();

    // Codegen
    let mut backend = clback::CraneliftBackend::new();
    
    backend.scan_externs(&tac_instructions);

    for (func_name, is_public) in functions_to_compile {
        let func_instructions: Vec<&Instruction> = tac_instructions.iter()
            .skip_while(|inst| !matches!(inst, Instruction::FunctionLabel(name) if name == &func_name))
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
            &irgen.var_types
        );
    }
    
    let product = backend.finish(); 
    let emit_result = product.emit().expect("Failed to emit object code");

    let mut file = File::create(output_filename).expect("Failed to create output file");
    file.write_all(&emit_result).expect("Failed to write binary payload to disk");

    Ok(())
}