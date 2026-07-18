use clap::builder::OsStr;
use cranelift::codegen::Context;
use cranelift_frontend::FunctionBuilderContext;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use serde_derive::Serialize;
use crate::backend::clback;
use crate::ir::ir::IRGen;
use crate::ir::tac::Instruction;
use crate::lexing::lexer::Lexer;
use crate::parse::parser::Parser as myszparser;
use crate::parse::parsing::{Identifier, Parameter, Program, Stmt};
use crate::semantics::analyser::{Analyser, AnalyserError};

#[derive(Serialize)]
struct JsonError {
    file: String,
    line: usize,
    column: usize,
    message: String,
    severity: String,
}

fn json_error_from_parser_error(err: &crate::parse::parsing::ParserError) -> JsonError {
    JsonError {
        file: err.location.file.to_string(),
        line: err.location.line,
        column: err.location.col,
        message: err.message.clone(),
        severity: "error".to_string(),
    }
}

fn json_error_from_string(file: &str, message: &str) -> JsonError {
    JsonError {
        file: file.to_string(),
        line: 0,
        column: 0,
        message: message.to_string(),
        severity: "error".to_string(),
    }
}

fn json_error_from_analyser_error(err: &AnalyserError) -> JsonError {
    let (location, message) = match err {
        AnalyserError::TypeError { location, message } => (location, message),
        AnalyserError::SemanticError { location, message } => (location, message),
    };
    JsonError {
        file: location.file.to_string(),
        line: location.line,
        column: location.col,
        message: message.clone(),
        severity: "error".to_string(),
    }
}

type SourceMap = HashMap<String, String>;

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

fn format_error_with_location(
    file_path: &str,
    line_num: usize,
    column: usize,
    message: &str,
    source: Option<&str>,
) -> String {
    let source_lines: Vec<&str> = source.map(|s| s.lines().collect()).unwrap_or_default();
    let source_line = if line_num > 0 && line_num <= source_lines.len() {
        source_lines[line_num - 1]
    } else {
        ""
    };

    let column_offset = if column > 0 { column - 1 } else { 0 };

    format!(
        "  --> {}:{}:{}\n      {}\n      {}{}\n      {}",
        file_path,
        line_num,
        column,
        source_line,
        " ".repeat(column_offset),
        "^".repeat(1),
        message
    )
}

fn format_simple_error(file_path: &Path, message: &str) -> String {
    format!("  --> {}\n      {}", file_path.display(), message)
}

fn format_module_error(module_path: &str, message: &str) -> String {
    format!("  --> module '{}'\n      {}", module_path, message)
}

fn format_parser_errors(errors: &[crate::parse::parsing::ParserError], source: &str) -> String {
    let source_lines: Vec<&str> = source.lines().collect();
    let mut error_messages = Vec::new();

    for err in errors {
        let location = &err.location;
        let line_num = location.line;
        let column = location.col;
        let file = location.file.clone();
        let message = &err.message;

        let source_line = if line_num > 0 && line_num <= source_lines.len() {
            source_lines[line_num - 1]
        } else {
            ""
        };

        let column_offset = if column > 0 { column - 1 } else { 0 };

        error_messages.push(format!(
            "  --> {}:{}:{}\n      {}\n      {}{}\n      {}",
            file,
            line_num,
            column,
            source_line,
            " ".repeat(column_offset),
            "^".repeat(1),
            message
        ));
    }

    error_messages.join("\n")
}

fn parse_and_flatten<P: AsRef<Path>>(
    input_path: P,
    custom_search_paths: &[PathBuf],
    json_output: bool,
) -> Result<(Program, SourceMap), String> {
    let input_path = input_path.as_ref().canonicalize().map_err(|e| {
        if json_output {
            let json_err = json_error_from_string(
                &input_path.as_ref().display().to_string(),
                &format!("Failed to canonicalize path: {}", e),
            );
            serde_json::to_string(&json_err).unwrap()
        } else {
            format_simple_error(input_path.as_ref(), &format!("Failed to canonicalize path: {}", e))
        }
    })?;

    let mut search_paths = Vec::new();
    if let Some(parent) = input_path.parent() {
        search_paths.push(parent.to_path_buf());
    }
    search_paths.extend_from_slice(custom_search_paths);

    let (source, tokens) = read_and_lex_file(&input_path, json_output)?;

    let mut parser = myszparser::new(tokens);
    parser.parse();

    if !parser.parser_errs.is_empty() {
        if json_output {
            let json_errors: Vec<JsonError> = parser.parser_errs
                .iter()
                .map(json_error_from_parser_error)
                .collect();
            return Err(serde_json::to_string(&json_errors).unwrap());
        } else {
            let error_report = format_parser_errors(&parser.parser_errs, &source);
            return Err(format!("Parser errors:\n{}", error_report));
        }
    }

    let mut visiting = HashSet::new();
    let mut processed = HashSet::new();
    visiting.insert(input_path.clone());

    let mut sources: SourceMap = HashMap::new();
    sources.insert(input_path.display().to_string(), source.clone());

    let flattened_statements = flatten_program_statements(
        parser.ast.statements,
        &search_paths,
        &mut visiting,
        &mut processed,
        &mut sources,
        json_output,
        &input_path,
    )?;

    let program = Program {
        statements: flattened_statements,
    };

    Ok((program, sources))
}

pub fn check_root_file<P: AsRef<Path>>(
    input_path: P,
    custom_search_paths: &[PathBuf],
    json_output: bool,
) -> Result<(), String> {
    let (program, sources) = parse_and_flatten(&input_path, custom_search_paths, json_output)?;
    let file_path = input_path.as_ref().canonicalize().unwrap();
    let root_source = sources
        .get(&file_path.display().to_string())
        .map(|s| s.as_str());

    let mut analyser = Analyser::new();
    if let Err(err) = analyser.analyse(&program) {
        if json_output {
            let json_err = json_error_from_analyser_error(&err);
            return Err(serde_json::to_string(&json_err).unwrap());
        } else {
            let formatted = format_analyser_error(&err, &sources, root_source);
            return Err(format!("Semantic error:\n{}", formatted));
        }
    }

    Ok(())
}

fn format_analyser_error(
    err: &AnalyserError,
    sources: &SourceMap,
    root_source: Option<&str>,
) -> String {
    let (location, message) = match err {
        AnalyserError::TypeError { location, message } => (location, message),
        AnalyserError::SemanticError { location, message } => (location, message),
    };
    let file_path_str = location.file.as_ref();
    let source = sources
        .get(file_path_str)
        .map(|s| s.as_str())
        .or(root_source);
    format_error_with_location(file_path_str, location.line, location.col, message, source)
}

fn read_and_lex_file(
    file_path: &Path,
    json_output: bool,
) -> Result<(String, Vec<crate::lexing::lexing::Token>), String> {
    let mut file = File::open(file_path)
        .map_err(|e| {
            if json_output {
                let json_err = json_error_from_string(
                    &file_path.display().to_string(),
                    &format!("Failed to open file: {}", e),
                );
                serde_json::to_string(&json_err).unwrap()
            } else {
                format_simple_error(file_path, &format!("Failed to open file: {}", e))
            }
        })?;
    let mut source = String::new();
    file.read_to_string(&mut source)
        .map_err(|e| {
            if json_output {
                let json_err = json_error_from_string(
                    &file_path.display().to_string(),
                    &format!("Failed to read file: {}", e),
                );
                serde_json::to_string(&json_err).unwrap()
            } else {
                format_simple_error(file_path, &format!("Failed to read file: {}", e))
            }
        })?;

    let file_id: Rc<str> = Rc::from(file_path.display().to_string());
    let mut lexer = Lexer::new(source.clone(), file_id);
    let res = lexer.lex();

    if let Err(err) = res {
        if json_output {
            let json_err = json_error_from_string(
                &file_path.display().to_string(),
                &format!("Lexer error: {}", err),
            );
            return Err(serde_json::to_string(&json_err).unwrap());
        } else {
            return Err(format_simple_error(
                file_path,
                &format!("Lexer error: {}", err),
            ));
        }
    }

    Ok((source, lexer.tokens))
}

fn flatten_program_statements(
    statements: Vec<Stmt>,
    search_paths: &[PathBuf],
    visiting: &mut HashSet<PathBuf>,
    processed: &mut HashSet<PathBuf>,
    sources: &mut SourceMap,
    json_output: bool,
    root_file_path: &Path,
) -> Result<Vec<Stmt>, String> {
    let mut flattened = Vec::new();

    for stmt in statements {
        if let Stmt::Use { path } = stmt {
            let module_path_str = path.join("::");
            let resolved_path = find_module_file(&path, search_paths).ok_or_else(|| {
                let msg = format!(
                    "Could not find module '{}' in search paths or CWD.",
                    module_path_str
                );
                if json_output {
                    let json_err = json_error_from_string(
                        &root_file_path.display().to_string(),
                        &msg,
                    );
                    serde_json::to_string(&json_err).unwrap()
                } else {
                    format_module_error(&module_path_str, &msg)
                }
            })?;

            if visiting.contains(&resolved_path) {
                let msg = format!("Cyclic dependency detected! Module imports itself.");
                if json_output {
                    let json_err = json_error_from_string(
                        &root_file_path.display().to_string(),
                        &msg,
                    );
                    return Err(serde_json::to_string(&json_err).unwrap());
                } else {
                    return Err(format_module_error(&module_path_str, &msg));
                }
            }

            if processed.contains(&resolved_path) {
                continue;
            }

            visiting.insert(resolved_path.clone());

            let (source, tokens) = read_and_lex_file(&resolved_path, json_output)?;
            sources.insert(resolved_path.display().to_string(), source.clone());

            let mut parser = myszparser::new(tokens);
            parser.parse();

            if !parser.parser_errs.is_empty() {
                if json_output {
                    let json_errors: Vec<JsonError> = parser.parser_errs
                        .iter()
                        .map(json_error_from_parser_error)
                        .collect();
                    return Err(serde_json::to_string(&json_errors).unwrap());
                } else {
                    let error_report = format_parser_errors(&parser.parser_errs, &source);
                    return Err(format!(
                        "Parser errors in module '{}':\n{}",
                        module_path_str, error_report
                    ));
                }
            }

            let module_stmts = flatten_program_statements(
                parser.ast.statements,
                search_paths,
                visiting,
                processed,
                sources,
                json_output,
                root_file_path,
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
    json_output: bool,
) -> Result<(), String> {
    let input_path = input_path.as_ref().canonicalize().map_err(|e| {
        format_simple_error(
            input_path.as_ref(),
            &format!("Failed to canonicalize path: {}", e),
        )
    })?;

    let mut search_paths = Vec::new();
    if let Some(parent) = input_path.parent() {
        search_paths.push(parent.to_path_buf());
    }
    search_paths.extend_from_slice(custom_search_paths);

    let (source, tokens) = read_and_lex_file(&input_path, json_output)?;

    let mut parser = myszparser::new(tokens);
    parser.parse();

    if !parser.parser_errs.is_empty() {
        if json_output {
            let json_errors: Vec<JsonError> = parser.parser_errs
                .iter()
                .map(json_error_from_parser_error)
                .collect();
            return Err(serde_json::to_string(&json_errors).unwrap());
        } else {
            let error_report = format_parser_errors(&parser.parser_errs, &source);
            return Err(format!("Parser errors:\n{}", error_report));
        }
    }

    let mut visiting = HashSet::new();
    let mut processed = HashSet::new();
    visiting.insert(input_path.clone());

    let mut sources: SourceMap = HashMap::new();
    sources.insert(input_path.display().to_string(), source.clone());

    let flattened_statements = flatten_program_statements(
        parser.ast.statements,
        &search_paths,
        &mut visiting,
        &mut processed,
        &mut sources,
        json_output,
        input_path.as_path()
    )?;

    let program = Program {
        statements: flattened_statements,
    };

    compile_ast_program(&program, output_filename, &sources, &input_path, json_output)
}

pub fn compile_ast_program(
    program: &Program,
    output_filename: &str,
    sources: &SourceMap,
    file_path: &Path,
    json_output: bool,
) -> Result<(), String> {
    let root_source = sources
        .get(&file_path.display().to_string())
        .map(|s| s.as_str());

    let filename: Rc<str> = Rc::from(
        file_path
            .file_name()
            .unwrap_or(&OsStr::default())
            .to_string_lossy()
            .as_ref(),
    );

    let mut analyser = Analyser::new();
    if let Err(err) = analyser.analyse(program) {
        if json_output {
            let location = match err.clone() {
                AnalyserError::SemanticError { location, .. } | AnalyserError::TypeError { location, .. }=> location
            };
            let message = match err.clone() {
                AnalyserError::SemanticError { message, .. } | AnalyserError::TypeError { message, .. }=> message
            };
            let json_err = JsonError {
                file: location.file.to_string(),
                line: location.line,
                column: location.col,
                message: message.to_string(),
                severity: match err {
                    AnalyserError::TypeError { .. } => "error".to_string(),
                    AnalyserError::SemanticError { .. } => "error".to_string(),
                },
            };
            let json_str = serde_json::to_string(&json_err)
                .map_err(|e| format!("Failed to serialize error: {}", e))?;
            return Err(json_str);
        } else {
            let formatted = format_analyser_error(&err, sources, root_source);
            return Err(format!("Semantic error:\n{}", formatted));
        }
    }
    let mut irgen = IRGen::new();
    irgen.analyser_constants = analyser.constants.clone();
    for (name, sig) in &analyser.structs {
        if !sig.generic_params.is_empty() {
            let fields_vec: Vec<Parameter> = sig
                .fields
                .iter()
                .map(|(fname, ftype)| Parameter {
                    name: Identifier {
                        value: fname.clone(),
                        location: crate::utils::location::Location::new_with_file(
                            0,
                            0,
                            filename.clone(),
                        ),
                    },
                    ptype: Some(ftype.clone()),
                })
                .collect();
            irgen
                .struct_blueprints
                .insert(name.clone(), (sig.generic_params.clone(), fields_vec));
        }
    }
    irgen.gen_program(program);
    // irgen.dump();

    let mut tac_instructions = Vec::new();
    let mut seen_labels = HashSet::new();
    let mut skip_current_duplicate = false;

    for inst in irgen.code {
        match &inst {
            Instruction::FunctionLabel(name) => {
                if seen_labels.contains(name) {
                    skip_current_duplicate = true;
                } else {
                    seen_labels.insert(name.clone());
                    skip_current_duplicate = false;
                    tac_instructions.push(inst);
                }
            }
            _ => {
                if !skip_current_duplicate {
                    tac_instructions.push(inst);
                }
            }
        }
    }

    let mut public_functions = HashSet::new();
    for stmt in &program.statements {
        if let Stmt::Function { name, public, .. } = stmt {
            if *public {
                public_functions.insert(name.value.clone());
            }
        }
    }

    let mut unique_function_names = HashSet::new();
    for inst in &tac_instructions {
        if let Instruction::FunctionLabel(name) = inst {
            unique_function_names.insert(name.clone());
        }
    }

    let mut backend = clback::CraneliftBackend::new(irgen.struct_defs, analyser.functions.clone());
    backend.scan_externs(&tac_instructions);

    let instruction_refs: Vec<&Instruction> = tac_instructions.iter().collect();
    backend.pre_declare_strings(&instruction_refs);

    for func_name in unique_function_names {
        let is_public = public_functions.contains(&func_name);

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
    let emit_result = product.emit().map_err(|e| {
        format_simple_error(file_path, &format!("Failed to emit object code: {}", e))
    })?;

    let mut file = File::create(output_filename).map_err(|e| {
        format_simple_error(
            file_path,
            &format!("Failed to create output file '{}': {}", output_filename, e),
        )
    })?;

    file.write_all(&emit_result).map_err(|e| {
        format_simple_error(
            file_path,
            &format!(
                "Failed to write to output file '{}': {}",
                output_filename, e
            ),
        )
    })?;

    Ok(())
}
