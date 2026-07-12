use cranelift::codegen::Context;
use cranelift_frontend::FunctionBuilderContext;
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::backend::clback;
use crate::ir::ir::IRGen;
use crate::ir::tac::Instruction;
use crate::lexing::lexer::Lexer;
use crate::parse::parser::Parser as myszparser;
use crate::parse::parsing::{Program, Stmt};
use crate::semantics::analyser::Analyser;

fn find_module_file(module_path: &[String], search_paths: &[PathBuf]) -> Option<PathBuf> {
    let mut relative_path = PathBuf::new();
    for segment in module_path {
        relative_path.push(segment);
    }
    relative_path.set_extension("mysz");

    for base in search_paths {
        let full_path = base.join(&relative_path);
        if full_path.exists() && full_path.is_file() {
            return Some(full_path);
        }
    }

    let cwd = std::env::current_dir().ok()?;
    let full_path = cwd.join(&relative_path);
    if full_path.exists() && full_path.is_file() {
        return Some(full_path);
    }

    None
}

fn flatten_program_statements(
    statements: Vec<Stmt>,
    search_paths: &[PathBuf],
    visiting: &mut HashSet<PathBuf>,
    processed: &mut HashSet<PathBuf>,
) -> Result<Vec<Stmt>, String> {
    let mut flattened = Vec::new();

    for stmt in statements {
        if let Stmt::Use { path } = stmt {
            let resolved_path = find_module_file(&path, search_paths).ok_or_else(|| {
                format!(
                    "Module Error: Could not find module '{}' in search paths or CWD.",
                    path.join("::")
                )
            })?;

            if visiting.contains(&resolved_path) {
                return Err(format!(
                    "Module Error: Cyclic dependency detected! Module '{}' (path: {:?}) imports itself.",
                    path.join("::"),
                    resolved_path
                ));
            }

            if processed.contains(&resolved_path) {
                continue;
            }

            visiting.insert(resolved_path.clone());

            let mut file = File::open(&resolved_path).map_err(|e| e.to_string())?;
            let mut source = String::new();
            file.read_to_string(&mut source)
                .map_err(|e| e.to_string())?;

            let mut lexer = Lexer::new(source);
            let res = lexer.lex();
            if res.is_err() {
                return Err(res.err().unwrap().to_string());
            }

            let mut parser = myszparser::new(lexer.tokens);
            parser.parse();
            if !parser.parser_errs.is_empty() {
                for perr in parser.parser_errs {
                    eprintln!("{:#}", perr);
                }
                return Err(format!("Parsing module {:?} failed", resolved_path));
            }

            let module_stmts = flatten_program_statements(
                parser.ast.statements,
                search_paths,
                visiting,
                processed,
            )?;

            flattened.extend(module_stmts);
            visiting.remove(&resolved_path);
            processed.insert(resolved_path);
        } else {
            flattened.push(stmt);
        }
    }

    Ok(flattened)
}

pub fn compile_root_file<P: AsRef<Path>>(
    input_path: P,
    output_filename: &str,
    custom_search_paths: &[PathBuf],
) -> Result<(), String> {
    let input_path = input_path
        .as_ref()
        .canonicalize()
        .map_err(|e| e.to_string())?;

    let mut search_paths = Vec::new();
    if let Some(parent) = input_path.parent() {
        search_paths.push(parent.to_path_buf());
    }
    search_paths.extend_from_slice(custom_search_paths);

    let mut file = File::open(&input_path).map_err(|e| e.to_string())?;
    let mut source = String::new();
    file.read_to_string(&mut source)
        .map_err(|e| e.to_string())?;

    let mut lexer = Lexer::new(source);
    let res = lexer.lex();

    if res.is_err() {
        res.err();
    }

    let mut parser = myszparser::new(lexer.tokens);
    parser.parse();
    if !parser.parser_errs.is_empty() {
        return Err("Parsing main file failed".to_string());
    }

    let mut visiting = HashSet::new();
    let mut processed = HashSet::new();
    visiting.insert(input_path.clone());

    let flattened_statements = flatten_program_statements(
        parser.ast.statements,
        &search_paths,
        &mut visiting,
        &mut processed,
    )?;

    let program = Program {
        statements: flattened_statements,
    };

    compile_ast_program(&program, output_filename)
}

pub fn compile_ast_program(program: &Program, output_filename: &str) -> Result<(), String> {
    // println!("{:#?}", program);

    let mut analyser = Analyser::new();
    analyser.analyse(program)?;

    let mut irgen = IRGen::new();
    irgen.gen_program(program);
    irgen.dump();

    println!("irgen.var_types: {:#?}", irgen.var_types);

    let tac_instructions = irgen.code;

    let mut unique_functions = HashSet::new();
    for stmt in &program.statements {
        if let Stmt::Function { name, public, .. } = stmt {
            unique_functions.insert((name.value.clone(), *public));
        }
    }
    for (mangled_name, sig) in &analyser.functions {
        if sig.generic_params.is_empty() {
            unique_functions.insert((mangled_name.clone(), true));
        }
    }

    let mut backend = clback::CraneliftBackend::new(irgen.struct_defs, analyser.functions.clone());
    backend.scan_externs(&tac_instructions);

    let instruction_refs: Vec<&Instruction> = tac_instructions.iter().collect();
    backend.pre_declare_strings(&instruction_refs);

    for (func_name, is_public) in unique_functions {
        let func_instructions: Vec<&Instruction> = tac_instructions
            .iter()
            .skip_while(
                |inst| !matches!(inst, Instruction::FunctionLabel(name) if name == &func_name),
            )
            .skip(1)
            .take_while(|inst| !matches!(inst, Instruction::FunctionLabel(_)))
            .collect();

        if !func_instructions.is_empty() {
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
    }

    let product = backend.finish();
    let emit_result = product.emit().expect("Failed to emit object code");

    let mut file = File::create(output_filename).expect("Failed to create output file");
    file.write_all(&emit_result)
        .expect("Failed to write to output file");

    Ok(())
}
